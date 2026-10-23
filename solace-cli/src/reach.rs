//! Reach and impact (D-44): which tests can see a change, and where a change's
//! data goes. Built on the reference graph and the formatter's layout, like
//! the rest of the Gleam support: derived, never a guarantee.
use super::*;
use std::collections::VecDeque;

/// Test functions: public `_test` functions in test modules (Gleam), or
/// `#[test]` functions (Rust, D-170).
pub fn test_symbols(ix: &Index) -> Vec<usize> {
    (0..ix.syms.len())
        .filter(|&i| {
            let s = &ix.syms[i];
            is_test(ix, i) && s.kind == Kind::Fn && is_test_fn(ix, i)
        })
        .collect()
}

/// Whether `i` is a test function: a Gleam `pub …_test`, or a Rust `#[test]`.
pub fn is_test_fn(ix: &Index, i: usize) -> bool {
    let s = &ix.syms[i];
    if ix.rust {
        language::rust_refs::has_test_attr(&s.text)
    } else {
        s.name.ends_with("_test") && s.text.contains(&format!("pub fn {}(", s.name))
    }
}

/// A test's libtest name (D-170): its module path and name, `crate` dropped,
/// and the `integration::<file>` prefix of a `tests/*.rs` crate dropped.
pub fn libtest_name(ix: &Index, i: usize) -> String {
    let s = &ix.syms[i];
    let prefix = libtest_prefix(ix, &s.module);
    if prefix.is_empty() { s.name.clone() } else { format!("{prefix}::{}", s.name) }
}

fn libtest_prefix(ix: &Index, module: &str) -> String {
    if module == "crate" {
        return String::new();
    }
    if let Some(rest) = module.strip_prefix("integration::") {
        // A file that is its own `tests/*.rs` binary is that binary's crate
        // root; a helper module (`tests/common/mod.rs`) keeps its own name.
        let root = ix
            .modules
            .iter()
            .find(|m| m.path == module)
            .and_then(|m| m.file.as_deref())
            .and_then(|f| f.strip_prefix("tests/"))
            .and_then(|f| f.strip_suffix(".rs"))
            .is_some_and(|stem| stem == rest && !stem.contains('/'));
        return if root { String::new() } else { rest.to_string() };
    }
    module.to_string()
}

/// Everything a symbol can run or read: its closure over `uses`.
// a DFS per test; switch to SCC condensation if suites reach tens of thousands of tests.
pub fn closure(ix: &Index, i: usize) -> HashSet<usize> {
    let (mut seen, mut stack) = (HashSet::new(), vec![i]);
    while let Some(j) = stack.pop() {
        if seen.insert(j) {
            stack.extend(&ix.uses[j]);
        }
    }
    seen
}

/// What a test checks (D-36.3): what it references, also through helpers in
/// its own test module.
pub fn covered(ix: &Index, t: usize) -> HashSet<usize> {
    let module = &ix.syms[t].module;
    let (mut out, mut seen, mut stack) = (HashSet::new(), HashSet::new(), vec![t]);
    while let Some(j) = stack.pop() {
        if seen.insert(j) {
            for &u in &ix.uses[j] {
                if ix.syms[u].module == *module {
                    stack.push(u)
                } else {
                    out.insert(u);
                }
            }
        }
    }
    out
}

/// Symbols whose effect static reach can't see the end of: other processes,
/// the network, and foreign code. A test that reaches one always runs.
pub fn outside(ix: &Index) -> HashSet<usize> {
    if ix.rust {
        let by_id: HashMap<&str, usize> = ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        let in_extern = |j: usize| {
            let mut p = ix.syms[j].parent.clone();
            while let Some(id) = p {
                let Some(&k) = by_id.get(id.as_str()) else { break };
                if ix.syms[k].kind_text == "extern" {
                    return true;
                }
                p = ix.syms[k].parent.clone();
            }
            false
        };
        return (0..ix.syms.len())
            .filter(|&j| {
                let s = &ix.syms[j];
                s.kind_text == "extern" || in_extern(j) || ["std::process", "std::net", "Command::new"].iter().any(|m| s.text.contains(m))
            })
            .collect();
    }
    (0..ix.syms.len())
        .filter(|&j| {
            ix.syms[j].text.lines().any(|l| l.starts_with("@external"))
                || view::direct_effects(ix, j).keys().any(|c| c == "processes" || c == "network")
        })
        .collect()
}

/// A test's reach, as a hash: the revisions of everything it can run, those
/// modules' headers, and the project and dependency manifests.
pub fn hash(ix: &Index, reach: &HashSet<usize>, revisions: &[String], root: &Path) -> String {
    let mut parts: Vec<String> = reach.iter().map(|&j| format!("{} {}\n", ix.syms[j].id, revisions[j])).collect();
    parts.sort();
    let modules: HashSet<&str> = reach.iter().map(|&j| ix.syms[j].module.as_str()).collect();
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    for m in ix.modules.iter().filter(|m| modules.contains(m.path.as_str())) {
        h.update(&m.header);
    }
    // A Rust project's tests also depend on its manifest, lockfile and build
    // script; a Gleam project's on its manifest and lockfile (D-170).
    let manifests: &[&str] = if ix.rust { &["Cargo.toml", "Cargo.lock", "build.rs"] } else { &["gleam.toml", "manifest.toml"] };
    for f in manifests {
        h.update(fs::read(root.join(f)).unwrap_or_default());
    }
    hex(&h.finalize())
}

/// Changed source symbols as flow origins, each with the lines that changed.
/// `before` holds (id, text before; `None` if new).
pub fn origins(ix: &Index, before: &[(String, Option<String>)]) -> Vec<(usize, Vec<usize>)> {
    let mut out = vec![];
    for (id, old) in before {
        let Some(i) = ix.syms.iter().position(|s| &s.id == id) else { continue };
        let new = &ix.syms[i].text;
        if is_test(ix, i) || old.as_deref().is_some_and(|o| revision(o) == revision(new)) {
            continue;
        }
        let lines: Vec<&str> = new.lines().collect();
        let mut seeds: Vec<usize> = match old {
            Some(o) => board::changed_lines(o, new)
                .into_iter()
                .filter(|&l| !lines.get(l).is_some_and(|t| t.trim_start().starts_with("//")))
                .collect(),
            None => vec![],
        };
        if seeds.is_empty() {
            seeds = (0..lines.len()).collect();
        }
        out.push((i, seeds));
    }
    out
}

pub struct Use {
    pub sym: usize,
    /// 0 for the changed symbols themselves.
    pub steps: usize,
    /// What connects it: `calls x.y` or `reads T.field`.
    pub via: String,
    pub certain: bool,
}

#[derive(Default)]
pub struct Impact {
    /// The changed symbols, then each use in the order found (nearest first).
    pub uses: Vec<Use>,
    /// (record type, field) the changed data reaches.
    pub fields: Vec<(usize, String)>,
    /// Where it leaves the program: (symbol, effects).
    pub leaves: Vec<(usize, String)>,
}

/// Record constructors and which types declare each field label.
struct Records {
    ctors: Vec<(usize, String, Vec<Option<String>>)>,
    declared: HashMap<String, Vec<usize>>,
}

fn records(ix: &Index) -> Records {
    let mut r = Records { ctors: vec![], declared: HashMap::new() };
    for (t, s) in ix.syms.iter().enumerate().filter(|(_, s)| s.kind == Kind::Type) {
        for (c, labels) in ix.lang.record_fields(&s.text) {
            for l in labels.iter().flatten() {
                let d = r.declared.entry(l.clone()).or_default();
                if !d.contains(&t) {
                    d.push(t);
                }
            }
            r.ctors.push((t, c, labels));
        }
    }
    r
}

/// The record types a read of `field` in `r` can be from, and whether that is
/// certain: one type declares the field, or `r` mentions exactly one of them.
fn read_types(ix: &Index, rec: &Records, r: usize, field: &str) -> (Vec<usize>, bool) {
    let Some(d) = rec.declared.get(field) else { return (vec![], false) };
    if d.len() == 1 {
        return (d.clone(), true);
    }
    let named: Vec<usize> = d.iter().copied().filter(|t| ix.uses[r].contains(t)).collect();
    if named.len() == 1 { (named, true) } else { (d.clone(), false) }
}

/// Every symbol (tests too) that reads one of `fields`.
pub fn readers(ix: &Index, fields: &[(usize, String)]) -> HashSet<usize> {
    let rec = records(ix);
    (0..ix.syms.len())
        .filter(|&r| {
            ix.lang
                .field_reads(&ix.syms[r].text)
                .iter()
                .any(|(f, _)| fields.iter().any(|(t, g)| g == f && read_types(ix, &rec, r, f).0.contains(t)))
        })
        .collect()
}

/// `refs <module>.<Type>.<field>`: the code that reads one field.
pub fn field_refs(ix: &Index, ty: usize, field: &str) -> String {
    let rec = records(ix);
    let q = format!("{}.{field}", ix.syms[ty].qname());
    if !rec.declared.get(field).is_some_and(|d| d.contains(&ty)) {
        return format!("{} has no field {field}.\n", ix.syms[ty].qname());
    }
    let mut out = String::new();
    let mut n = 0;
    for r in 0..ix.syms.len() {
        if !ix.lang.field_reads(&ix.syms[r].text).iter().any(|(f, _)| f == field) {
            continue;
        }
        let (types, certain) = read_types(ix, &rec, r, field);
        if types.contains(&ty) {
            n += 1;
            out += &format!(
                "-- {}{}\n{}\n",
                ix.syms[r].qname(),
                if certain { "" } else { " (possible: another type has this field)" },
                crate::index::signature(ix, &ix.syms[r])
            );
        }
    }
    if n == 0 { format!("Nothing reads {q}.\n") } else { format!("{q} is read by {n}:\n{out}") }
}

/// Where changed data goes (D-44). From each changed symbol's changed lines,
/// taint follows `let` bindings within a function, then: callers of a tainted
/// function, at the call (Gleam is pure, so a call's result reaches the
/// caller's), readers of the record fields it writes, at the read, and the
/// field writes of helpers called with a tainted argument (only those, so a
/// shared helper doesn't pull in all its callers). The analysis stops where
/// the data stops; only displays are cut.
pub fn flow(ix: &Index, origins: &[(usize, Vec<usize>)]) -> Impact {
    if ix.rust {
        return flow_calls(ix, origins);
    }
    let n = ix.syms.len();
    let rec = records(ix);
    let callers = &ix.used_by;
    let reads: Vec<Vec<(String, usize)>> =
        (0..n).map(|j| if is_test(ix, j) { vec![] } else { ix.lang.field_reads(&ix.syms[j].text) }).collect();
    let mut impact = Impact::default();
    // per symbol: tainted lines, and marks (`name` for a call, `.field` for a read)
    let mut seeds: HashMap<usize, (HashSet<usize>, HashSet<String>)> = HashMap::new();
    let mut found: HashMap<usize, usize> = HashMap::new();
    let (mut fields, mut entered): (HashSet<(usize, String)>, HashSet<(usize, Vec<String>)>) = (HashSet::new(), HashSet::new());
    let mut queue: VecDeque<(usize, Vec<usize>, Vec<String>, usize, String, bool)> =
        origins.iter().map(|(i, s)| (*i, s.clone(), vec![], 0, "changed".to_string(), true)).collect();
    while let Some((i, lines, marks, steps, via, certain)) = queue.pop_front() {
        if is_test(ix, i) {
            continue;
        }
        let (known, marked) = seeds.entry(i).or_default();
        let fresh = lines.into_iter().filter(|l| known.insert(*l)).count() + marks.into_iter().filter(|m| marked.insert(m.clone())).count();
        if let std::collections::hash_map::Entry::Vacant(e) = found.entry(i) {
            e.insert(impact.uses.len());
            impact.uses.push(Use { sym: i, steps, via, certain });
        } else if fresh == 0 {
            continue;
        }
        let first = &impact.uses[found[&i]];
        let (steps, certain) = (first.steps, first.certain);
        // a possible use (a field name several types share) is listed but not
        // followed, so ambiguous names can't flood the result; follow it if that misses flows.
        if !certain {
            continue;
        }
        let t = taint(ix, &rec, i, known, marked, &[]);
        if !t.any {
            continue;
        }
        let leaving = leaves(ix, i);
        if !leaving.is_empty() && !impact.leaves.iter().any(|l| l.0 == i) {
            impact.leaves.push((i, leaving));
        }
        for &c in &callers[i] {
            // a type's users are seeded at the lines that name it; others at the call
            let (at, mark) = match ix.syms[i].kind {
                Kind::Type => (ix.at[c].iter().filter(|(g, _)| *g == i).map(|(_, l)| *l).collect(), vec![]),
                _ => (vec![], written(ix, c, i)),
            };
            queue.push_back((c, at, mark, steps + 1, format!("calls {}", short(ix, i)), certain));
        }
        let mut written = t.fields;
        let mut helpers = t.calls;
        while let Some((g, params)) = helpers.pop() {
            if entered.insert((g, params.clone())) {
                // the tainted fields this helper certainly reads
                let reads = fields
                    .iter()
                    .chain(&written)
                    .filter(|(ty, f)| matches!(read_types(ix, &rec, g, f), (types, true) if types.contains(ty)))
                    .map(|(_, f)| format!(".{f}"))
                    .collect();
                let h = taint(ix, &rec, g, &HashSet::new(), &reads, &params);
                written.extend(h.fields);
                helpers.extend(h.calls);
            }
        }
        for (ty, f) in written {
            if !fields.insert((ty, f.clone())) {
                continue;
            }
            impact.fields.push((ty, f.clone()));
            let mark: HashSet<String> = [format!(".{f}")].into();
            for (r, rs) in reads.iter().enumerate() {
                let at: Vec<usize> = rs.iter().filter(|(g, _)| *g == f).map(|(_, l)| *l).collect();
                if at.is_empty() {
                    continue;
                }
                let (types, sure) = read_types(ix, &rec, r, &f);
                if types.contains(&ty) {
                    // `.field` reads are marked; pattern reads seed their lines
                    let code = ix.lang.strip_comments(&ix.syms[r].text);
                    let lines: Vec<&str> = code.split('\n').collect();
                    let at = at.into_iter().filter(|&l| !lines.get(l).is_some_and(|t| marked_in(t, &mark))).collect();
                    queue.push_back((r, at, vec![format!(".{f}")], steps + 1, format!("reads {}.{f}", ix.syms[ty].name), certain && sure));
                }
            }
        }
    }
    impact
}

/// Rust's flow (D-170): the changed symbols and every caller of them,
/// transitively. A Rust `let` is not the data marker Gleam's is, so no fields
/// are tracked and certainty is not weighed; callers are followed whatever
/// changed, which overestimates.
fn flow_calls(ix: &Index, origins: &[(usize, Vec<usize>)]) -> Impact {
    let mut impact = Impact::default();
    let mut found: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<(usize, usize, String, bool)> = origins.iter().map(|(i, _)| (*i, 0, "changed".to_string(), true)).collect();
    while let Some((i, steps, via, certain)) = queue.pop_front() {
        if is_test(ix, i) || !found.insert(i) {
            continue;
        }
        impact.uses.push(Use { sym: i, steps, via, certain });
        for &c in &ix.used_by[i] {
            queue.push_back((c, steps + 1, format!("calls {}", short(ix, i)), certain));
        }
    }
    impact
}

/// How `user`'s code writes symbol `i`: `alias.name`, an unqualified import,
/// or the bare name in its own module.
fn written(ix: &Index, user: usize, i: usize) -> Vec<String> {
    let (s, u) = (&ix.syms[i], &ix.syms[user]);
    if s.module == u.module {
        return vec![s.name.clone()];
    }
    let header = ix.modules.iter().find(|m| m.path == u.module).map_or("", |m| m.header.as_str());
    let mut out = vec![];
    for imp in ix.lang.imports(header).into_iter().filter(|imp| imp.module == s.module) {
        out.push(format!("{}.{}", imp.alias, s.name));
        out.extend(imp.unqualified.into_iter().filter(|(o, _)| *o == s.name).map(|(_, local)| local));
    }
    out
}

/// Whether code mentions a marked symbol (as `written` gives it) or field
/// read (`.field`).
fn marked_in(text: &str, marks: &HashSet<String>) -> bool {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
    marks.iter().any(|m| {
        let (field, name) = m.strip_prefix('.').map_or((false, m.as_str()), |n| (true, n));
        text.match_indices(name).any(|(p, _)| {
            let before = text[..p].chars().next_back();
            !ident(text[p + name.len()..].chars().next()) && if field { before == Some('.') } else { !ident(before) }
        })
    })
}

/// `pricing.quote` for `pilot/pricing.quote`.
pub fn short(ix: &Index, i: usize) -> String {
    let s = &ix.syms[i];
    format!("{}.{}", s.module.rsplit('/').next().unwrap_or(&s.module), s.name)
}

/// Effects through which data leaves the program.
fn leaves(ix: &Index, i: usize) -> String {
    const OUT: &[&str] = &["prints", "writes files", "network", "database", "processes", "external code"];
    // effects are per function, not per tainted statement.
    view::direct_effects(ix, i).into_keys().filter(|c| OUT.contains(&c.as_str())).collect::<Vec<_>>().join(", ")
}

#[derive(Default)]
struct Taint {
    any: bool,
    /// (record type, field label) written from tainted data.
    fields: Vec<(usize, String)>,
    /// Functions called with tainted arguments: (function, its tainted parameters).
    calls: Vec<(usize, Vec<String>)>,
}

/// Taint within one symbol, from changed lines (`seeds`), marked calls and
/// reads (`marks`, see `marked_in`) and tainted `params`.
fn taint(ix: &Index, rec: &Records, i: usize, seeds: &HashSet<usize>, marks: &HashSet<String>, params: &[String]) -> Taint {
    let s = &ix.syms[i];
    let mut t = Taint::default();
    if s.kind == Kind::Type {
        t.any = !seeds.is_empty() || !marks.is_empty();
        return t;
    }
    let code = ix.lang.strip_comments(&s.text);
    let lines: Vec<&str> = code.split('\n').collect();
    let open = fn_body_start(s).filter(|_| s.kind == Kind::Fn).map(|o| s.text[..o].matches('\n').count());
    let close = lines.iter().rposition(|l| l.starts_with('}'));
    let segments = match (open, close) {
        (Some(o), Some(c)) if c > o + 1 => statements(&lines, o + 1, c),
        _ => None,
    };
    // A change in the signature, or a body that can't be segmented: all of it.
    let whole = !seeds.is_empty() && (segments.is_none() || seeds.iter().any(|&l| l <= open.unwrap_or(0)));
    let segments = segments.unwrap_or_else(|| vec![(0, lines.len())]);
    let mut names: HashSet<String> = params.iter().cloned().collect();
    for (start, end) in segments {
        let text = lines[start..end].join("\n");
        let hit = whole || seeds.iter().any(|l| (start..end).contains(l)) || !roots(&text).is_disjoint(&names) || marked_in(&text, marks);
        let head = text.trim_start();
        if let Some(rest) = head.strip_prefix("let ").or_else(|| head.strip_prefix("use ")) {
            let (pattern, value) = rest.split_once(if head.starts_with("use ") { "<-" } else { "=" }).unwrap_or((rest, ""));
            // `let db = store.put(db, x)` updates a value: its changed parts are
            // tracked as fields, so the name keeps its state
            let updated = roots(value);
            for w in ix.lang.pattern_names(pattern).into_iter().filter(|w| !updated.contains(w)) {
                if hit {
                    names.insert(w)
                } else {
                    names.remove(&w)
                };
            }
        }
        if !hit {
            continue;
        }
        t.any = true;
        let touched = |value: &str, (a, b): (usize, usize)| {
            whole || !roots(value).is_disjoint(&names) || marked_in(value, marks) || seeds.iter().any(|l| a <= *l && *l < b)
        };
        let span = |at: usize, part: &str| {
            let first = start + text[..at].matches('\n').count();
            (first, first + part.matches('\n').count() + 1)
        };
        // record constructors: fields whose argument is tainted
        for (p, c) in text.char_indices() {
            if !c.is_uppercase() || text[..p].ends_with(|x: char| x.is_alphanumeric() || x == '_') {
                continue;
            }
            let word: String = text[p..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let open = p + word.len();
            if !text[open..].starts_with('(') {
                continue;
            }
            for (ty, _, labels) in rec.ctors.iter().filter(|(ty, n, _)| *n == word && ix.uses[i].contains(ty)) {
                // `..base` adds no field: the fields a value carries are tracked one by one
                let mut position = 0;
                for (off, arg) in language::split_args(&text[open + 1..]) {
                    let sp = span(open + 1 + off, &arg);
                    let arg = arg.trim();
                    if arg.starts_with("..") {
                        continue;
                    }
                    let (label, value) = split_label(arg);
                    let k = match label {
                        Some(l) => labels.iter().position(|x| x.as_deref() == Some(l.as_str())),
                        None => {
                            position += 1;
                            Some(position - 1)
                        }
                    };
                    let Some(k) = k else { continue };
                    if touched(&value, sp)
                        && let Some(Some(f)) = labels.get(k)
                    {
                        t.fields.push((*ty, f.clone()));
                    }
                }
                if whole {
                    t.fields.extend(labels.iter().flatten().map(|f| (*ty, f.clone())));
                }
            }
        }
        // calls with a tainted argument: the parameters it reaches
        for &(g, line) in &ix.at[i] {
            if !(start..end).contains(&line) || g == i || ix.syms[g].kind != Kind::Fn || is_test(ix, g) {
                continue;
            }
            let ps = params_of(ix, &ix.syms[g]);
            let name = format!("{}(", ix.syms[g].name);
            let call = text.match_indices(&name).map(|(p, _)| p).find(|&p| !text[..p].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
            let piped = call.is_some_and(|p| {
                text[..p].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.').trim_end().ends_with("|>")
            });
            let tainted: Vec<String> = match call.filter(|_| !whole) {
                Some(p) => {
                    let mut free: Vec<usize> = (0..ps.len()).collect();
                    let (mut out, mut positional) = (vec![], vec![]);
                    // a piped call's first argument is the pipeline's value: tainted if its head is
                    if piped && !ps.is_empty() {
                        free.remove(0);
                        let head = &text[..text.find("|>").unwrap_or(p)];
                        if touched(head, span(0, head)) {
                            out.push(ps[0].1.clone());
                        }
                    }
                    for (off, arg) in language::split_args(&text[p + name.len()..]) {
                        let sp = span(p + name.len() + off, &arg);
                        match split_label(arg.trim()) {
                            (Some(l), v) => {
                                if let Some(k) = ps.iter().position(|(pl, _)| pl.as_deref() == Some(l.as_str())) {
                                    free.retain(|&x| x != k);
                                    if touched(&v, sp) {
                                        out.push(ps[k].1.clone());
                                    }
                                }
                            }
                            (None, v) => positional.push((v, sp)),
                        }
                    }
                    for ((v, sp), k) in positional.into_iter().zip(free) {
                        if touched(&v, sp) {
                            out.push(ps[k].1.clone());
                        }
                    }
                    out
                }
                // passed as a value, or unsure: every parameter
                None => ps.iter().map(|p| p.1.clone()).collect(),
            };
            if !tainted.is_empty() && !t.calls.contains(&(g, tainted.clone())) {
                t.calls.push((g, tainted));
            }
        }
    }
    t
}

/// A function's parameters: (label, name).
fn params_of(ix: &Index, s: &Sym) -> Vec<(Option<String>, String)> {
    params(&ix.lang.head(&s.item()), &s.name)
}

/// `label: value` (or the shorthand `label:`) into its parts.
fn split_label(arg: &str) -> (Option<String>, String) {
    let label: String = arg.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    match arg[label.len()..].trim_start().strip_prefix(':') {
        Some(v) if label.starts_with(|c: char| c.is_lowercase()) && !v.starts_with(':') => {
            let v = v.trim();
            (Some(label.clone()), if v.is_empty() { label } else { v.to_string() })
        }
        _ => (None, arg.to_string()),
    }
}

/// A function body's top-level statements as line ranges, split at the
/// formatter's indentation: a line at the body's indent starts a statement
/// unless it closes a bracket or continues an expression (`|>`, an operator).
/// `None` when the layout isn't the formatter's.
fn statements(lines: &[&str], from: usize, to: usize) -> Option<Vec<(usize, usize)>> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let base = indent(lines[from..to].iter().find(|l| !l.trim().is_empty())?);
    let mut out: Vec<(usize, usize)> = vec![];
    for (k, l) in lines.iter().enumerate().take(to).skip(from) {
        let t = l.trim_start();
        if t.is_empty() {
            continue;
        }
        if indent(l) < base {
            return None;
        }
        let continues = indent(l) > base || t.starts_with([')', ']', '}', '|', '+', '-', '*', '/', '<', '>', '=', '!', '&', '.']);
        match out.last_mut() {
            Some(last) if continues => last.1 = k + 1,
            _ => out.push((k, k + 1)),
        }
    }
    if let Some(last) = out.last_mut() {
        last.1 = to + 1; // the closing brace
    }
    Some(out)
}

/// Names a piece of code reads as whole values: identifiers that aren't a
/// field (`.x`), a label (`x:`, except the shorthand `x:`, which reads `x`) or
/// the record a field is read from (`x` in `x.total`: that read counts only
/// if the field is tainted, through its `.total` mark).
fn roots(text: &str) -> HashSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let (mut out, mut i) = (HashSet::new(), 0);
    while i < chars.len() {
        if !(chars[i].is_alphabetic() || chars[i] == '_') || (i > 0 && is_ident(chars[i - 1])) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_ident(chars[j]) {
            j += 1;
        }
        let field = i > 0 && chars[i - 1] == '.' && !(i > 1 && chars[i - 2] == '.');
        let label = chars.get(j) == Some(&':') && chars.get(j + 1) != Some(&':');
        let shorthand = label && chars[j + 1..].iter().find(|c| **c != ' ').is_none_or(|c| matches!(c, ',' | ')' | '\n'));
        let record = chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|c| c.is_alphabetic());
        if !field && !record && (!label || shorthand) {
            out.insert(chars[i..j].iter().collect());
        }
        i = j;
    }
    out
}

/// Declared commands that are test suites outside `gleam test` (`checks` in
/// solace.toml, D-44): (name, reach; `None` when it checks everything). For
/// `gleam run -m <module>`, the module's `main` is an entry point.
/// A declared command that tests: its name, and what it reaches (`None`: everything).
pub type Suite = (String, Option<HashSet<usize>>);

pub fn suites(s: &Store, ix: &Index) -> Result<Vec<Suite>, Failure> {
    let settings = lockdown::settings(&s.root)?;
    let mut out = vec![];
    for (name, c) in settings.get("commands").and_then(|c| c.as_table()).into_iter().flatten() {
        let Some(checks) = c.get("checks").and_then(|x| x.as_array()) else { continue };
        let mut entries: Vec<String> = checks.iter().filter_map(|x| x.as_str().map(String::from)).collect();
        let run: Vec<&str> = c["run"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
        if let ["gleam", "run", "-m", m, ..] = run.as_slice() {
            entries.push(format!("{m}.main"));
        }
        if entries.iter().any(|e| e == "*") {
            out.push((name.clone(), None));
            continue;
        }
        // entry points that don't resolve are skipped silently.
        let reach = entries.iter().filter_map(|e| s.resolve(ix, e).ok()).flat_map(|i| closure(ix, i)).collect();
        out.push((name.clone(), Some(reach)));
    }
    Ok(out)
}

/// Every entry point the declared suites name, resolved or not, as (suite,
/// entry): `extract` lists those that stop resolving (`suites` skips them).
pub fn entries(s: &Store) -> Result<Vec<(String, String)>, Failure> {
    let settings = lockdown::settings(&s.root)?;
    let mut out = vec![];
    for (name, c) in settings.get("commands").and_then(|c| c.as_table()).into_iter().flatten() {
        let Some(checks) = c.get("checks").and_then(|x| x.as_array()) else { continue };
        let run: Vec<&str> = c["run"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
        let mut named: Vec<String> = checks.iter().filter_map(|x| x.as_str().map(String::from)).filter(|e| e != "*").collect();
        if let ["gleam", "run", "-m", m, ..] = run.as_slice() {
            named.push(format!("{m}.main"));
        }
        out.extend(named.into_iter().map(|e| (name.clone(), e)));
    }
    Ok(out)
}

/// The suites whose reach holds one of `syms`, as `e2e (`run e2e`)`.
pub fn reached(suites: &[Suite], syms: &[usize]) -> Vec<String> {
    suites
        .iter()
        .filter(|(_, r)| !syms.is_empty() && r.as_ref().is_none_or(|r| syms.iter().any(|j| r.contains(j))))
        .map(|(n, _)| format!("{n} (`run {n}`)"))
        .collect()
}

/// Each source symbol's tests, by D-36.3 coverage.
fn own_tests(ix: &Index) -> HashMap<usize, Vec<usize>> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for t in test_symbols(ix) {
        for u in covered(ix, t) {
            out.entry(u).or_default().push(t);
        }
    }
    out
}

/// The uses (not the changed symbols): certain ones nearest first, those
/// without a test of their own first within a step, then the possible ones
/// (run 20: these crowded out the callers whose tests reach the change):
/// (use, its tests).
pub fn ranked<'a>(ix: &Index, impact: &'a Impact) -> Vec<(&'a Use, Vec<usize>)> {
    let own = own_tests(ix);
    let mut out: Vec<(&Use, Vec<usize>)> =
        impact.uses.iter().filter(|u| u.steps > 0).map(|u| (u, own.get(&u.sym).cloned().unwrap_or_default())).collect();
    out.sort_by_key(|(u, t)| (!u.certain, u.steps, !t.is_empty()));
    out
}

/// "N possible uses: readers of fields another type also declares (…)", or nothing.
fn possible_text(uses: &[(&Use, Vec<usize>)]) -> Option<String> {
    let mut fields: Vec<&str> = vec![];
    for (u, _) in uses.iter().filter(|(u, _)| !u.certain) {
        let f = u.via.strip_prefix("reads ").unwrap_or(&u.via);
        if !fields.contains(&f) {
            fields.push(f);
        }
    }
    let n = uses.iter().filter(|(u, _)| !u.certain).count();
    let more = if fields.len() > 4 { format!(" and {} more", fields.len() - 4) } else { String::new() };
    (n > 0).then(|| {
        format!(
            "{}: readers of a field another type also declares ({}{more})",
            count(n, "possible use"),
            fields[..fields.len().min(4)].join(", ")
        )
    })
}

fn fields_text(ix: &Index, impact: &Impact) -> String {
    let (mut out, mut last) = (vec![], usize::MAX);
    for (ty, f) in impact.fields.iter().take(8) {
        out.push(if *ty == last { format!(".{f}") } else { format!("{}.{f}", ix.syms[*ty].name) });
        last = *ty;
    }
    if impact.fields.len() > 8 {
        out.push(format!("and {} more", impact.fields.len() - 8));
    }
    out.join(", ")
}

/// The submit reply's note: at most three lines (D-44).
pub fn note(ix: &Index, impact: &Impact, suites: &[String]) -> Vec<String> {
    let uses = ranked(ix, impact);
    if uses.is_empty() {
        return vec![];
    }
    let origin: Vec<String> = impact.uses.iter().filter(|u| u.steps == 0).map(|u| short(ix, u.sym)).collect();
    let callers = uses.iter().any(|(u, _)| u.via.starts_with("calls"));
    let kinds = match (callers, impact.fields.is_empty()) {
        (true, true) => "callers".to_string(),
        (true, false) => format!("callers, and readers of {}", fields_text(ix, impact)),
        (false, _) => format!("readers of {}", fields_text(ix, impact)),
    };
    let sure = uses.iter().filter(|(u, _)| u.certain).count();
    let maybe = if sure < uses.len() { format!(" (and {} possible)", uses.len() - sure) } else { String::new() };
    let mut out = vec![format!("Reaches {}{maybe}: {kinds}.", count(sure, "use"))];
    let untested: Vec<String> = uses
        .iter()
        .filter(|(u, t)| u.certain && t.is_empty())
        .map(|(u, _)| {
            let via = u.via.strip_prefix("reads ").map_or(String::new(), |f| format!(", reads {}", f.split_once('.').map_or(f, |p| p.1)));
            format!("{} ({}{via})", short(ix, u.sym), count(u.steps, "step"))
        })
        .collect();
    let all = format!("`impact {}` lists all.", origin.join(" "));
    if untested.is_empty() {
        out[0] += &format!(" Each has a test of its own; {all}");
    } else {
        let more = if untested.len() > 3 { format!(" and {} more", untested.len() - 3) } else { String::new() };
        out.push(format!("No test of their own: {}{more} — {all}", untested[..untested.len().min(3)].join(", ")));
    }
    let mut last = vec![];
    if !impact.leaves.is_empty() {
        let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", short(ix, *i))).collect();
        last.push(format!("Leaves the program: {}.", leaves.join(", ")));
    }
    if !suites.is_empty() {
        last.push(format!("Also reached: {}.", suites.join(", ")));
    }
    if !last.is_empty() {
        out.push(last.join(" "));
    }
    out
}

/// `impact`: every use, nearest first, within `budget`.
pub fn report(ix: &Index, impact: &Impact, suites: &[String], budget: usize) -> String {
    let origin: Vec<String> = impact.uses.iter().filter(|u| u.steps == 0).map(|u| short(ix, u.sym)).collect();
    let uses = ranked(ix, impact);
    let sure = uses.iter().filter(|(u, _)| u.certain).count();
    let mut out =
        format!("{} reach{} {}, nearest first:\n", origin.join(", "), if origin.len() == 1 { "es" } else { "" }, count(sure, "use"));
    let mut tail = possible_text(&uses).map_or(String::new(), |p| format!("And {p}; `refs <Type>.<field>` lists them\n"));
    if !impact.fields.is_empty() {
        let fields: Vec<String> = impact.fields.iter().map(|(t, f)| format!("{}.{f}", ix.syms[*t].name)).collect();
        tail += &format!("Fields: {}\n", fields.join(", "));
    }
    if !impact.leaves.is_empty() {
        let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", short(ix, *i))).collect();
        tail += &format!("Leaves the program: {}\n", leaves.join(", "));
    }
    if !suites.is_empty() {
        tail += &format!("Also reached: {}\n", suites.join(", "));
    }
    for (k, (u, t)) in uses.iter().take(sure).enumerate() {
        let tests = match t.as_slice() {
            [] => "no test of its own".to_string(),
            [one] => format!("tested by {}", short(ix, *one)),
            [one, rest @ ..] => format!("tested by {} and {} more", short(ix, *one), rest.len()),
        };
        let line = format!("  {} — {}, {}; {tests}\n", short(ix, u.sym), count(u.steps, "step"), u.via);
        if out.len() + line.len() + tail.len() + 60 > budget {
            out += &format!("  … {} more not shown, to fit the size limit\n", sure - k);
            break;
        }
        out += &line;
    }
    out + &tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    /// `refs Type.field`: who reads one field, sure or possible (D-44).
    #[test]
    fn field_refs_list_the_readers_of_a_field() {
        let ix = crate::index::test_index(&[
            ("src", "price", "pub type Quote {\n  Quote(discount: Int, total: Int)\n}\n\npub type Rate {\n  Rate(total: Int)\n}"),
            (
                "src",
                "report",
                "import price\n\npub fn discount(q: price.Quote) -> Int {\n  q.discount\n}\n\npub fn total(x) -> Int {\n  x.total\n}",
            ),
        ]);
        let quote = ix.syms.iter().position(|s| s.name == "Quote").unwrap();
        let out = field_refs(&ix, quote, "discount");
        assert!(out.starts_with("price.Quote.discount is read by 1:\n-- report.discount\n"), "{out}");
        let out = field_refs(&ix, quote, "total");
        assert!(out.contains("-- report.total (possible: another type has this field)"), "{out}");
        assert_eq!(field_refs(&ix, quote, "nope"), "price.Quote has no field nope.\n");
    }

    /// Declared commands with `checks` are suites: they reach what their
    /// entries reach, a `gleam run -m` module's `main` included.
    #[test]
    fn declared_suites_reach_their_entries() {
        let store = project(&[
            ("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n"),
            ("src/e2e.gleam", "import calc\n\npub fn main() {\n  calc.sub(2, 1)\n}\n"),
        ]);
        fs::write(
            store.root.join("solace.toml"),
            concat!(
                "[commands]\ne2e = { run = [\"gleam\", \"run\", \"-m\", \"e2e\"], ",
                "checks = [\"calc.add\"] }\nall = { run = [\"make\"], checks = [\"*\"] }\n"
            ),
        )
        .unwrap();
        let ix = store.index().unwrap();
        let suites = suites(&store, &ix).unwrap();
        let id = |name: &str| ix.syms.iter().position(|s| s.name == name).unwrap();
        assert_eq!(reached(&suites, &[id("add")]), ["e2e (`run e2e`)", "all (`run all`)"]);
        assert_eq!(reached(&suites, &[id("sub")]), ["e2e (`run e2e`)", "all (`run all`)"]);
        assert!(reached(&suites, &[]).is_empty());
    }

    fn names(ix: &Index, impact: &Impact) -> Vec<String> {
        impact.uses.iter().filter(|u| u.steps > 0).map(|u| short(ix, u.sym)).collect()
    }

    #[test]
    fn flow_follows_lets_fields_and_helpers() {
        let ix = crate::index::test_index(&[
            ("src", "money", "pub fn percent(x: Int, p: Int) -> Int {\n  x * p / 100\n}"),
            (
                "src",
                "price",
                concat!(
                    "import money\n\npub type Quote {\n  Quote(discount: Int, total: Int, note: String)\n}\n\n",
                    "pub type Rate {\n  Rate(total: Int)\n}\n\n",
                    "pub fn quote(cents: Int, extra: Int) -> Quote {\n  let discount = money.percent(cents, 5)\n",
                    "  let total = cents + extra\n  let note = \"fixed\"\n",
                    "  Quote(discount: discount, total: total, note:)\n}\n\n",
                    "pub fn wrap(q: Quote) -> Quote {\n  Quote(..q, total: q.total + 1, note: \"x\")\n}"
                ),
            ),
            (
                "src",
                "report",
                concat!(
                    "import price\n\npub fn discounts(qs: List(price.Quote)) -> Int {\n  case qs {\n",
                    "    [q, ..] -> q.discount\n    [] -> 0\n  }\n}\n\n",
                    "pub fn totals(q) -> Int {\n  q.total\n}\n\n",
                    "pub fn rate(r: price.Rate) -> Int {\n  r.total\n}"
                ),
            ),
            ("src", "other", "import money\n\npub fn tip(x: Int) -> Int {\n  money.percent(x, 10)\n}"),
            ("test", "price_test", "import price\n\npub fn quote_test() {\n  assert price.quote(100, 0).total == 100\n}"),
        ]);
        let quote = ix.syms.iter().position(|s| s.name == "quote").unwrap();
        // the changed `let discount` (line 1) feeds only `discount`
        let impact = flow(&ix, &[(quote, vec![1])]);
        let fields: Vec<&str> = impact.fields.iter().map(|(_, f)| f.as_str()).collect();
        assert_eq!(fields, ["discount"]);
        let found = names(&ix, &impact);
        // a reader in another module, reading the field from stored data
        assert!(found.contains(&"report.discounts".to_string()), "{found:?}");
        assert!(!found.iter().any(|n| n == "report.totals" || n == "report.rate"), "{found:?}");
        // the helper called with tainted data doesn't pull in its other callers
        assert!(!found.contains(&"other.tip".to_string()), "{found:?}");
        // a field two types declare: certain where the reader names one, possible where it names none
        let impact = flow(&ix, &[(quote, vec![2])]);
        let total = |n: &str| impact.uses.iter().find(|u| short(&ix, u.sym) == n).map(|u| u.certain);
        assert_eq!(total("report.totals"), Some(false));
        assert_eq!(total("report.rate"), None);
        assert!(impact.fields.iter().any(|(_, f)| f == "total") && !impact.fields.iter().any(|(_, f)| f == "discount"));
        // run 20: possible uses come after the certain ones, as a count
        let r = report(&ix, &impact, &[], 10_000);
        assert!(r.contains("And 1 possible use: readers of a field another type also declares (Quote.total)"), "{r}");
        assert!(!r.contains("report.totals"), "{r}");
        let n = note(&ix, &impact, &[]);
        assert!(n[0].contains("(and 1 possible)") && !n.iter().any(|l| l.contains("report.totals")), "{n:?}");
        // a record update: a field read from a tainted record counts only if that field is tainted
        let wrap = ix.syms.iter().position(|s| s.name == "wrap").unwrap();
        let fields = |marks: &[&str]| -> Vec<String> {
            let marks = marks.iter().map(|m| m.to_string()).collect();
            taint(&ix, &records(&ix), wrap, &HashSet::new(), &marks, &["q".into()]).fields.into_iter().map(|f| f.1).collect()
        };
        assert!(fields(&[]).is_empty());
        assert_eq!(fields(&[".total"]), ["total"]);
        assert_eq!(flow(&ix, &[(wrap, vec![0])]).fields.len(), 3, "a signature change taints every field");
        let lines = note(&ix, &flow(&ix, &[(quote, vec![1])]), &[]);
        assert!(lines[0].starts_with("Reaches 1 use: readers of Quote.discount."), "{lines:?}");
        assert!(lines[1].starts_with("No test of their own: report.discounts (1 step, reads discount)"), "{lines:?}");
    }

    #[test]
    fn coverage_and_always_run() {
        let ix = crate::index::test_index(&[
            (
                "src",
                "app",
                concat!(
                    "import shellout\n\npub fn main() {\n  step()\n}\n\n",
                    "fn step() {\n  helper()\n}\n\nfn helper() {\n  1\n}\n\n",
                    "pub fn run() {\n  shellout.command(\"ls\", [], \".\", [])\n}\n\n",
                    "@external(erlang, \"erlang\", \"abs\")\npub fn abs(x: Int) -> Int"
                ),
            ),
            (
                "test",
                "app_test",
                concat!(
                    "import app\n\npub fn main_test() {\n  assert app.main() == 1\n}\n\n",
                    "pub fn run_test() {\n  check(app.run())\n}\n\n",
                    "fn check(x) {\n  app.abs(x)\n}"
                ),
            ),
        ]);
        let id = |n: &str| ix.syms.iter().position(|s| s.name == n).unwrap();
        let main_test = id("main_test");
        assert!(closure(&ix, main_test).contains(&id("helper")));
        // calling `main` doesn't make a helper two steps away tested
        assert!(covered(&ix, main_test).contains(&id("main")) && !covered(&ix, main_test).contains(&id("helper")));
        // helpers in the test's own module count
        assert!(covered(&ix, id("run_test")).contains(&id("abs")));
        let out = outside(&ix);
        assert!(out.contains(&id("run")) && out.contains(&id("abs")) && !out.contains(&id("main")));
        assert_eq!(test_symbols(&ix), [main_test, id("run_test")]);
    }

    /// A new symbol seeds every line; a comment-only change is no change at
    /// all; changed lines that are comments don't seed.
    #[test]
    fn origins_seed_the_changed_lines_and_skip_what_is_not_code() {
        let ix = crate::index::test_index(&[
            ("src", "main", "pub fn a() -> Int {\n  1\n}\n\npub fn b() -> Int {\n  2\n  // c\n  3\n}"),
            ("test", "main_test", "import main\n\npub fn a_test() {\n  assert main.a() == 1\n}"),
        ]);
        let b = ix.syms.iter().position(|s| s.name == "b").unwrap();
        assert!(
            origins(&ix, &[("fn:main.a".to_string(), Some("pub fn a() -> Int {\n  // note\n  1\n}".to_string()))]).is_empty(),
            "a comment-only change is not code"
        );
        assert!(
            origins(
                &ix,
                &[("fn:main_test.a_test".to_string(), Some("import main\n\npub fn a_test() {\n  assert main.a() == 2\n}".to_string()))]
            )
            .is_empty(),
            "a test's own change is not an origin"
        );
        assert_eq!(origins(&ix, &[("fn:main.b".to_string(), None)]), [(b, vec![0, 1, 2, 3, 4])], "a new symbol seeds every line");
        assert_eq!(
            origins(&ix, &[("fn:main.b".to_string(), Some("pub fn b() -> Int {\n  2\n  // c\n}".to_string()))]),
            [(b, vec![3])],
            "the added code line seeds, its closing brace doesn't"
        );
        assert_eq!(
            origins(&ix, &[("fn:main.b".to_string(), Some("pub fn b() -> Int {\n  2\n}".to_string()))]),
            [(b, vec![3])],
            "the added code line seeds; the added comment line doesn't"
        );
    }

    /// A changed type seeds its users at the lines that name it, not at a call.
    #[test]
    fn a_type_change_seeds_the_lines_that_name_it() {
        let ix = crate::index::test_index(&[
            ("src", "shape", "pub type Shape {\n  Shape(a: Int)\n}"),
            ("src", "use", "import shape\n\npub fn f(s: shape.Shape) -> Int {\n  1\n}"),
        ]);
        let shape = ix.syms.iter().position(|s| s.name == "Shape").unwrap();
        let impact = flow(&ix, &[(shape, vec![0])]);
        assert_eq!(names(&ix, &impact), ["use.f"], "seeded at the line naming the type");
        assert_eq!(impact.uses.iter().find(|u| u.sym == shape).map(|u| u.steps), Some(0));
    }

    /// A piped call taints its first parameter when the pipeline's head is
    /// tainted; a positional argument reaches the parameter at its position;
    /// an argument that isn't tainted reaches none.
    #[test]
    fn piped_and_positional_arguments_taint_the_parameters_they_carry() {
        let ix = crate::index::test_index(&[
            ("src", "lib", "pub fn double(n: Int) -> Int {\n  n * 2\n}"),
            ("src", "main", "import lib\n\npub fn f(x: Int, y: Int) -> Int {\n  x |> lib.double()\n  lib.double(x)\n  lib.double(y)\n}"),
        ]);
        let double = ix.syms.iter().position(|s| s.name == "double").unwrap();
        let f = ix.syms.iter().position(|s| s.name == "f").unwrap();
        let marks: HashSet<String> = ["lib.double".to_string()].into();
        let t = taint(&ix, &records(&ix), f, &HashSet::new(), &marks, &["x".into()]);
        assert_eq!(t.calls, [(double, vec!["n".to_string()])], "the piped head and `double(x)` carry x, `double(y)` nothing");
        let t = taint(&ix, &records(&ix), f, &HashSet::new(), &marks, &[]);
        assert!(t.any && t.calls.is_empty(), "nothing is tainted, so no parameter is: {:?}", t.calls);
    }

    /// A body whose layout isn't the formatter's can't be segmented: the taint
    /// takes the whole symbol, so every field of a record it writes counts.
    #[test]
    fn an_unformatted_body_is_taken_as_a_whole() {
        let ix = crate::index::test_index(&[
            ("src", "quote", "pub type Quote {\n  Quote(a: Int, b: Int)\n}"),
            ("src", "odd", "import quote\n\npub fn f(n: Int) -> quote.Quote {\n    let q = quote.Quote(a: n)\n  quote.Quote(..q, b: 1)\n}"),
        ]);
        let f = ix.syms.iter().position(|s| s.name == "f").unwrap();
        let mut fields: Vec<String> =
            taint(&ix, &records(&ix), f, &[1].into(), &HashSet::new(), &["n".into()]).fields.into_iter().map(|(_, g)| g).collect();
        fields.sort();
        fields.dedup();
        assert_eq!(fields, ["a", "b"], "the whole symbol is tainted, so every field counts");
    }

    /// A caller using an unqualified import — its own name or its `as` name —
    /// is seeded by the symbol it writes, as an alias is.
    #[test]
    fn unqualified_imports_count_as_writing_the_symbol() {
        let ix = crate::index::test_index(&[
            ("src", "lib", "pub fn make() -> Int {\n  1\n}"),
            ("src", "plain", "import lib.{make}\n\npub fn a() {\n  make()\n}"),
            ("src", "renamed", "import lib.{make as m}\n\npub fn b() {\n  m()\n}"),
            ("src", "aliased", "import lib\n\npub fn c() {\n  lib.make()\n}"),
        ]);
        let make = ix.syms.iter().position(|s| s.name == "make").unwrap();
        assert_eq!(names(&ix, &flow(&ix, &[(make, vec![0, 1])])), ["plain.a", "renamed.b", "aliased.c"]);
    }

    /// The note and the report cut long lists: fields past eight, untested uses
    /// past three, possible fields past four, and uses past the size limit.
    #[test]
    fn the_note_and_the_report_cut_their_lists() {
        let ix = crate::index::test_index(&[
            (
                "src",
                "main",
                concat!(
                    "pub fn a() {\n  1\n}\n\npub fn b() {\n  1\n}\n\n",
                    "pub fn c() {\n  1\n}\n\npub fn d() {\n  1\n}\n\n",
                    "pub fn e() {\n  1\n}\n\npub fn f() {\n  1\n}\n\n",
                    "pub fn g() {\n  1\n}\n\npub fn h() {\n  1\n}\n\n",
                    "pub fn i() {\n  1\n}\n\npub fn j() {\n  1\n}\n\n",
                    "pub fn k() {\n  1\n}\n\npub fn l() {\n  1\n}"
                ),
            ),
            ("src", "shape", "pub type T {\n  T(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int, i: Int)\n}"),
        ]);
        let t = ix.syms.iter().position(|s| s.name == "T").unwrap();
        let mut impact = Impact::default();
        impact.uses.push(Use { sym: 0, steps: 0, via: "changed".to_string(), certain: true });
        for i in 1..5 {
            impact.uses.push(Use { sym: i, steps: 1, via: "calls main.a".to_string(), certain: true });
        }
        for k in 0..6 {
            impact.uses.push(Use { sym: 5 + k, steps: 1, via: format!("reads T.{}", (b'a' + k as u8) as char), certain: false });
        }
        impact.fields = (0..9).map(|k| (t, format!("{}", (b'a' + k as u8) as char))).collect();
        impact.leaves = vec![(0, "prints".to_string())];
        let lines = note(&ix, &impact, &["e2e (`run e2e`)".to_string()]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].starts_with("Reaches 4 uses (and 6 possible): callers, and readers of T.a, .b, .c, .d, .e, .f, .g, .h, and 1 more."),
            "{lines:?}"
        );
        assert!(lines[1].starts_with("No test of their own: main.b (1 step), main.c (1 step), main.d (1 step) and 1 more"), "{lines:?}");
        assert_eq!(lines[2], "Leaves the program: main.a (prints). Also reached: e2e (`run e2e`).");
        let r = report(&ix, &impact, &["e2e (`run e2e`)".to_string()], 10_000);
        assert!(r.starts_with("main.a reaches 4 uses, nearest first:\n  main.b — 1 step, calls main.a; no test of its own\n"), "{r}");
        assert!(r.contains("And 6 possible uses: readers of a field another type also declares (T.a, T.b, T.c, T.d and 2 more)"), "{r}");
        let r = report(&ix, &impact, &[], 10);
        assert!(r.contains("  … 4 more not shown, to fit the size limit"), "{r}");
    }

    /// A declared command whose entries don't resolve is a suite that reaches
    /// nothing, and it is skipped silently.
    #[test]
    fn suites_skip_entries_that_do_not_resolve() {
        let store = project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
        fs::write(store.root.join("solace.toml"), "[commands]\nghost = { run = [\"make\"], checks = [\"calc.gone\"] }\n").unwrap();
        let ix = store.index().unwrap();
        let suites = suites(&store, &ix).unwrap();
        assert_eq!(suites.len(), 1);
        assert!(suites[0].1.as_ref().is_some_and(|r| r.is_empty()), "{suites:?}");
        assert!(reached(&suites, &[0]).is_empty(), "nothing is reached, so no suite is listed");
    }

    /// A test's reach hash covers the revisions of everything it reaches, those
    /// modules' headers, and the manifests: any of them changes it.
    #[test]
    fn the_reach_hash_follows_revisions_headers_and_manifests() {
        let store = project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
        let ix = store.index().unwrap();
        let reach: HashSet<usize> = [0].into();
        let base = hash(&ix, &reach, &["r1".to_string()], &store.root);
        assert_eq!(base, hash(&ix, &reach, &["r1".to_string()], &store.root), "the same inputs give the same hash");
        assert_ne!(base, hash(&ix, &reach, &["r2".to_string()], &store.root), "a revision is part of it");
        fs::write(store.root.join("gleam.toml"), "name = \"other\"\n").unwrap();
        assert_ne!(base, hash(&ix, &reach, &["r1".to_string()], &store.root), "the manifest is part of it");
    }
}
