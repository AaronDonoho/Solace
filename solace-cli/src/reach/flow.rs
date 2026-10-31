//! Data-flow and taint analysis: where a change's data goes through calls, fields and reads.

use super::*;
use crate::Sym;
use crate::Index;
use crate::is_test;
use crate::fn_body_start;
use crate::params;
#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;

/// Record constructors and which types declare each field label.
pub(crate) struct Records {
    pub(crate) ctors: Vec<(usize, String, Vec<Option<String>>)>,
    pub(crate) declared: HashMap<String, Vec<usize>>,
}

pub(crate) fn records(ix: &Index) -> Records {
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
pub(crate) fn read_types(ix: &Index, rec: &Records, r: usize, field: &str) -> (Vec<usize>, bool) {
    let Some(d) = rec.declared.get(field) else { return (vec![], false) };
    if d.len() == 1 {
        return (d.clone(), true);
    }
    let named: Vec<usize> = d.iter().copied().filter(|t| ix.uses[r].contains(t)).collect();
    if named.len() == 1 { (named, true) } else { (d.clone(), false) }
}

/// Rust's flow (D-170): the changed symbols and every caller of them,
/// transitively. A Rust `let` is not the data marker Gleam's is, so no fields
/// are tracked and certainty is not weighed; callers are followed whatever
/// changed, which overestimates.
pub(crate) fn flow_calls(ix: &Index, origins: &[(usize, Vec<usize>)]) -> Impact {
    let mut impact = Impact::default();
    let mut found: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<(usize, usize, String, bool)> = origins.iter().map(|(i, _)| (*i, 0, "changed".to_string(), true)).collect();
    while let Some((i, steps, via, certain)) = queue.pop_front() {
        if is_test(ix, i) || !found.insert(i) {
            continue;
        }
        impact.uses.push(Use { sym: i, steps, via, certain });
        for &c in &ix.used_by[i] {
            queue.push_back((c, steps + 1, format!("calls {}", super::suites::short(ix, i)), certain));
        }
    }
    impact
}

/// How `user`'s code writes symbol `i`: `alias.name`, an unqualified import,
/// or the bare name in its own module.
pub(crate) fn written(ix: &Index, user: usize, i: usize) -> Vec<String> {
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
pub(crate) fn marked_in(text: &str, marks: &HashSet<String>) -> bool {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
    marks.iter().any(|m| {
        let (field, name) = m.strip_prefix('.').map_or((false, m.as_str()), |n| (true, n));
        text.match_indices(name).any(|(p, _)| {
            let before = text[..p].chars().next_back();
            !ident(text[p + name.len()..].chars().next()) && if field { before == Some('.') } else { !ident(before) }
        })
    })
}

/// Effects through which data leaves the program.
pub(crate) fn leaves(ix: &Index, i: usize) -> String {
    const OUT: &[&str] = &["prints", "writes files", "network", "database", "processes", "external code"];
    // effects are per function, not per tainted statement.
    crate::view::graph_effects(ix, i).into_keys().filter(|c| OUT.contains(&c.as_str())).collect::<Vec<_>>().join(", ")
}

#[derive(Default)]
pub(crate) struct Taint {
    pub(crate) any: bool,
    /// (record type, field label) written from tainted data.
    pub(crate) fields: Vec<(usize, String)>,
    /// Functions called with tainted arguments: (function, its tainted parameters).
    pub(crate) calls: Vec<(usize, Vec<String>)>,
}

/// Taint within one symbol, from changed lines (`seeds`), marked calls and
/// reads (`marks`, see `marked_in`) and tainted `params`.
pub(crate) fn taint(ix: &Index, rec: &Records, i: usize, seeds: &HashSet<usize>, marks: &HashSet<String>, params: &[String]) -> Taint {
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
pub fn gleam_flow(ix: &Index, origins: &[(usize, Vec<usize>)]) -> Impact {
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
            queue.push_back((c, at, mark, steps + 1, format!("calls {}", super::suites::short(ix, i)), certain));
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
