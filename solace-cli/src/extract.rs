//! `extract <symbols…> --to <module>` (D-103): moves symbols to another module
//! of the project with their identity, brings along the private helpers only
//! they use, rewrites every reference and import, moves the tests that reach
//! only moved code, and refuses a module cycle (`E5002`). One transaction,
//! gated on `check_all`.
use super::*;
use std::collections::BTreeSet;

const MODULE_HINT: &str = "to move a whole module, use move src/x.gleam src/y.gleam";

/// An import a module needs after the move: `import <module> [as <alias>]`,
/// with unqualified `items` as written (`type Task`, `Some`).
#[derive(Clone, Debug)]
struct Need {
    module: String,
    alias: Option<String>,
    items: Vec<String>,
}

/// How a reference reached its symbol.
#[derive(Clone)]
enum Via {
    /// Bare, in the symbol's own module.
    Same,
    /// `alias.member`.
    Qual(String),
    /// Bare through an unqualified import; the import's items for the name, as written.
    Unq(Vec<String>),
}

struct Resolved {
    x: Option<usize>,
    member: String,
    via: Via,
    /// The imported module the reference went through.
    module: Option<String>,
}

fn tail(module: &str) -> String {
    module.rsplit('/').next().unwrap_or(module).to_string()
}

/// `type Task` -> (`Task`, `Task`); `Foo as Bar` -> (`Foo`, `Bar`).
fn item_split(raw: &str) -> (String, String) {
    let t = raw.strip_prefix("type ").unwrap_or(raw).trim();
    match t.split_once(" as ") {
        Some((n, l)) => (n.trim().into(), l.trim().into()),
        None => (t.into(), t.into()),
    }
}

/// The import items that make `member` of `s` available unqualified.
fn item_for(s: &Sym, member: &str) -> Vec<String> {
    match s.kind {
        Kind::Type if member == s.name => {
            let mut v = vec![format!("type {member}")];
            if gleam::constructors(&s.text).iter().any(|c| c == member) {
                v.push(member.to_string());
            }
            v
        }
        _ => vec![member.to_string()],
    }
}

fn is_pub_text(text: &str) -> bool {
    text.lines().find(|l| gleam::declaration(l).is_some()).is_some_and(|l| l.starts_with("pub "))
}

fn make_pub(text: &str) -> String {
    let mut done = false;
    text.split('\n')
        .map(|l| {
            if !done && gleam::declaration(l).is_some() {
                done = true;
                format!("pub {l}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The path from project folder `from` to `to` (both as the root names them).
fn relative(from: &str, to: &str) -> String {
    let parts = |d: &str| d.split('/').filter(|s| !s.is_empty() && *s != ".").map(String::from).collect::<Vec<_>>();
    let (f, t) = (parts(from), parts(to));
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".into(); f.len() - common];
    out.extend(t[common..].iter().cloned());
    out.join("/")
}

/// The projects a `gleam.toml` depends on by path.
fn path_deps(dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(dir.join("gleam.toml")) else { return vec![] };
    let Ok(v) = text.parse::<toml::Value>() else { return vec![] };
    ["dependencies", "dev-dependencies", "dev_dependencies"]
        .iter()
        .filter_map(|t| v.get(t)?.as_table())
        .flat_map(|t| t.values())
        .filter_map(|s| s.get("path")?.as_str())
        .filter_map(|p| dir.join(p).canonicalize().ok())
        .collect()
}

fn name_map(ix: &Index) -> HashMap<(String, String), usize> {
    let mut names = HashMap::new();
    for (i, s) in ix.syms.iter().enumerate() {
        names.insert((s.module.clone(), s.name.clone()), i);
    }
    for (i, s) in ix.syms.iter().enumerate() {
        if s.kind == Kind::Type {
            for c in gleam::constructors(&s.text) {
                names.entry((s.module.clone(), c)).or_insert(i);
            }
        }
    }
    names
}

/// The items of `module`'s import in `header` that bring in `orig` as `local`.
fn raw_items(header: &str, module: &str, orig: &str, local: &str) -> Vec<String> {
    let found: Vec<String> = import_statements(header)
        .into_iter()
        .filter(|(m, _, _)| m == module)
        .flat_map(|(_, items, _)| items)
        .filter(|raw| item_split(raw) == (orig.to_string(), local.to_string()))
        .collect();
    if found.is_empty() { vec![orig.to_string()] } else { found }
}

fn resolve_ref(r: &Ref, oh: &str, imps: &[gleam::Import], header: &str, names: &HashMap<(String, String), usize>) -> Option<Resolved> {
    match r {
        Ref::Qualified(alias, member) => {
            let imp = imps.iter().find(|i| &i.alias == alias)?;
            Some(Resolved {
                x: names.get(&(imp.module.clone(), member.clone())).copied(),
                member: member.clone(),
                via: Via::Qual(alias.clone()),
                module: Some(imp.module.clone()),
            })
        }
        Ref::Bare(name) => {
            if let Some(&x) = names.get(&(oh.to_string(), name.clone())) {
                return Some(Resolved { x: Some(x), member: name.clone(), via: Via::Same, module: None });
            }
            let imp = imps.iter().find(|i| i.unqualified.iter().any(|(_, l)| l == name))?;
            let (orig, local) = imp.unqualified.iter().find(|(_, l)| l == name)?.clone();
            Some(Resolved {
                x: names.get(&(imp.module.clone(), orig.clone())).copied(),
                member: orig.clone(),
                via: Via::Unq(raw_items(header, &imp.module, &orig, &local)),
                module: Some(imp.module.clone()),
            })
        }
    }
}

/// The names a piece of code uses from its imports: bare names and module aliases.
fn locals_used(text: &str) -> HashSet<String> {
    gleam::references(text)
        .into_iter()
        .map(|r| match r {
            Ref::Bare(n) => n,
            Ref::Qualified(a, _) => a,
        })
        .collect()
}

/// Rewrites the import lines of `header`: items for which `keep_item(module,
/// raw)` is false are dropped, and so is an import for which `keep_import(
/// module, alias, items_were_dropped)` is false once it has no items left.
fn edit_imports(header: &str, keep_item: &dyn Fn(&str, &str) -> bool, keep_import: &dyn Fn(&str, &str, bool) -> bool) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let (mut out, mut at): (Vec<String>, usize) = (vec![], 0);
    for (module, items, (start, end)) in import_statements(header) {
        out.extend(lines[at..start].iter().map(|s| s.to_string()));
        at = end + 1;
        let stmt = lines[start..=end].join(" ");
        let suffix = stmt.rfind('}').map_or_else(|| stmt[format!("import {module}").len()..].to_string(), |b| stmt[b + 1..].to_string());
        let alias = suffix.trim().strip_prefix("as ").map_or_else(|| tail(&module), |a| a.trim().to_string());
        let kept: Vec<String> = items.iter().filter(|i| keep_item(&module, i)).cloned().collect();
        let dropped = kept.len() < items.len();
        if kept.is_empty() && !keep_import(&module, &alias, dropped) {
            continue;
        }
        if !dropped {
            out.extend(lines[start..=end].iter().map(|s| s.to_string()));
        } else if kept.is_empty() {
            out.push(format!("import {module}{}", suffix.trim_end()));
        } else {
            out.push(format!("import {module}.{{{}}}{}", kept.join(", "), suffix.trim_end()));
        }
    }
    out.extend(lines[at..].iter().map(|s| s.to_string()));
    out.join("\n")
}

/// Whether a module-level graph (module -> modules it uses) has a cycle.
fn cyclic(edges: &BTreeMap<String, BTreeSet<String>>) -> bool {
    fn visit(m: &str, edges: &BTreeMap<String, BTreeSet<String>>, state: &mut HashMap<String, u8>) -> bool {
        match state.get(m) {
            Some(1) => return true,
            Some(_) => return false,
            None => {}
        }
        state.insert(m.to_string(), 1);
        for n in edges.get(m).into_iter().flatten() {
            if visit(n, edges, state) {
                return true;
            }
        }
        state.insert(m.to_string(), 2);
        false
    }
    let mut state = HashMap::new();
    edges.keys().any(|m| visit(m, edges, &mut state))
}

/// `Task(` style uses of an opaque type's constructors by `j`: constructing or
/// matching, not the type in an annotation.
fn constructs(ix: &Index, j: usize, ty: usize) -> bool {
    let ctors = gleam::constructors(&ix.syms[ty].text);
    let s = &ix.syms[j];
    let body = fn_body_start(s).filter(|_| s.kind == Kind::Fn);
    let code = gleam::strip_comments(&s.text);
    gleam::references_spans(&s.text).into_iter().any(|(r, a, _)| {
        let name = match &r {
            Ref::Bare(n) | Ref::Qualified(_, n) => n,
        };
        if !ctors.contains(name) {
            return false;
        }
        if *name != ix.syms[ty].name {
            return true;
        }
        // the type's own name: a constructor call or pattern, unless in a signature or after `:`
        let prev = code[..a].trim_end().chars().last();
        !(body.is_some_and(|b| a < b) || prev == Some(':'))
    })
}

/// Everything the move changes, before the store is touched.
struct Plan<'a> {
    ix: &'a Index,
    names: HashMap<(String, String), usize>,
    moved: Vec<bool>,
    newmod: Vec<String>,
    newtext: Vec<String>,
    needs: BTreeMap<String, Vec<Need>>,
    edits: usize,
    edited: BTreeSet<String>,
}

impl Plan<'_> {
    fn old_imports(&self, m: &str) -> Vec<gleam::Import> {
        self.ix.modules.iter().find(|r| r.path == m).map_or(vec![], |r| gleam::imports(&r.header))
    }

    /// (alias, module) of the imports `m` has after the move.
    fn aliases(&self, m: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.old_imports(m).into_iter().map(|i| (i.alias, i.module)).collect();
        for n in self.needs.get(m).into_iter().flatten() {
            out.push((n.alias.clone().unwrap_or_else(|| tail(&n.module)), n.module.clone()));
        }
        out
    }

    /// (local name, module, name there) of the unqualified imports `m` has after the move.
    fn unqualified(&self, m: &str) -> Vec<(String, String, String)> {
        let mut out = vec![];
        for imp in self.old_imports(m) {
            for (orig, local) in imp.unqualified {
                if self.names.get(&(imp.module.clone(), orig.clone())).is_none_or(|&x| !self.moved[x]) {
                    out.push((local, imp.module.clone(), orig));
                }
            }
        }
        for n in self.needs.get(m).into_iter().flatten() {
            for raw in &n.items {
                let (orig, local) = item_split(raw);
                out.push((local, n.module.clone(), orig));
            }
        }
        out
    }

    fn defines(&self, m: &str, name: &str) -> bool {
        self.ix.syms.iter().enumerate().any(|(i, s)| {
            self.newmod[i] == m && (s.name == name || (s.kind == Kind::Type && gleam::constructors(&s.text).iter().any(|c| c == name)))
        })
    }

    fn need(&mut self, m: &str, n: Need) -> Result<(), Failure> {
        let alias = n.alias.clone().unwrap_or_else(|| tail(&n.module));
        if n.items.is_empty()
            && let Some((_, other)) = self.aliases(m).into_iter().find(|(a, o)| *a == alias && *o != n.module)
        {
            return fail(format!("E5004 {m} already imports {other} as {alias}; {} would need that name too", n.module));
        }
        let list = self.needs.entry(m.to_string()).or_default();
        if !list.iter().any(|x| x.module == n.module && x.alias == n.alias && x.items == n.items) {
            list.push(n);
        }
        Ok(())
    }

    /// How every module refers to every symbol after the move, written into
    /// `newtext`, with the imports each module then needs in `needs`.
    fn rewrite(&mut self) -> Result<(), Failure> {
        let ix = self.ix;
        for h in 0..ix.syms.len() {
            let (oh, nh) = (ix.syms[h].module.clone(), self.newmod[h].clone());
            let imps = self.old_imports(&oh);
            let header = ix.modules.iter().find(|m| m.path == oh).map_or("", |m| m.header.as_str());
            let text = ix.syms[h].text.clone();
            let mut bound: Option<HashSet<String>> = None;
            let mut edits: Vec<(usize, usize, String)> = vec![];
            for (r, a, b) in gleam::references_spans(&text) {
                let Some(res) = resolve_ref(&r, &oh, &imps, header, &self.names) else { continue };
                let Some(x) = res.x else {
                    // a dependency's or unknown name: moved code carries the import
                    if nh != oh
                        && let Some(module) = &res.module
                    {
                        match &res.via {
                            Via::Qual(alias) => {
                                let alias = (*alias != tail(module)).then(|| alias.clone());
                                self.need(&nh, Need { module: module.clone(), alias, items: vec![] })?
                            }
                            Via::Unq(raw) => self.need(&nh, Need { module: module.clone(), alias: None, items: raw.clone() })?,
                            Via::Same => {}
                        }
                    }
                    continue;
                };
                if x == h {
                    continue;
                }
                let (ox, mx) = (ix.syms[x].module.clone(), self.newmod[x].clone());
                if nh == oh && mx == ox {
                    continue;
                }
                let member = res.member.clone();
                let bound = bound.get_or_insert_with(|| gleam::bound_names(&text));
                let slice = text[a..b].to_string();
                if nh == mx {
                    if matches!(res.via, Via::Qual(_)) && bound.contains(&member) {
                        return fail(format!("E5004 {}.{} uses {member} next to a local name {member}; rename the local first", ix.syms[h].module, ix.syms[h].name));
                    }
                    if slice != member {
                        edits.push((a, b, member));
                    }
                    continue;
                }
                if let Via::Unq(raw) = &res.via {
                    self.need(&nh, Need { module: mx.clone(), alias: None, items: raw.clone() })?;
                    continue;
                }
                let existing = self.aliases(&nh).into_iter().find(|(_, m)| *m == mx).map(|(al, _)| al);
                let alias = existing.clone().unwrap_or_else(|| tail(&mx));
                let clash = bound.contains(&alias)
                    || self.defines(&nh, &alias)
                    || (existing.is_none() && self.aliases(&nh).iter().any(|(al, m)| *al == alias && *m != mx));
                if !clash {
                    if existing.is_none() {
                        self.need(&nh, Need { module: mx.clone(), alias: None, items: vec![] })?;
                    }
                    let repl = format!("{alias}.{member}");
                    if slice != repl {
                        edits.push((a, b, repl));
                    }
                } else {
                    let taken = bound.contains(&member)
                        || self.defines(&nh, &member)
                        || self.unqualified(&nh).iter().any(|(l, m, o)| *l == member && !(*m == mx && *o == member));
                    if taken {
                        return fail(format!(
                            "E5004 {nh} cannot refer to {mx}.{member}: the qualifier {alias} clashes with an import, a definition or a local name, and {member} is taken too"
                        ));
                    }
                    self.need(&nh, Need { module: mx.clone(), alias: None, items: item_for(&ix.syms[x], &member) })?;
                    if slice != member {
                        edits.push((a, b, member));
                    }
                }
            }
            if !edits.is_empty() {
                edits.sort_by_key(|e| std::cmp::Reverse(e.0));
                let mut t = text.clone();
                for (a, b, repl) in &edits {
                    t.replace_range(*a..*b, repl);
                }
                self.newtext[h] = t;
                self.edits += edits.len();
                self.edited.insert(nh);
            }
        }
        Ok(())
    }
}

/// Private symbols whose users all move come along (a fixpoint over `used_by`).
fn bring_helpers(ix: &Index, moved: &mut [bool], out: &mut Vec<usize>) {
    loop {
        let mut grew = false;
        for j in 0..ix.syms.len() {
            if moved[j] || is_pub_text(&ix.syms[j].text) || ix.used_by[j].is_empty() {
                continue;
            }
            if ix.used_by[j].iter().all(|&u| moved[u]) {
                moved[j] = true;
                out.push(j);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

struct Args {
    names: Vec<String>,
    to: String,
    to_project: Option<String>,
}

fn parse(args: &[&str]) -> Result<Args, Failure> {
    const FORM: &str = "extract <symbol>... --to <module>";
    let (mut names, mut to, mut to_project) = (vec![], None, None);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--to" => to = Some(it.next().ok_or_else(|| Failure(format!("--to needs a module: {FORM}"), 2))?.to_string()),
            "--to-project" => {
                to_project = Some(it.next().ok_or_else(|| Failure(format!("--to-project needs a folder: {FORM} --to-project <folder>"), 2))?.to_string())
            }
            _ if a.starts_with("--") => return usage(format!("unknown option {a}: {FORM}")),
            _ => names.push(a.to_string()),
        }
    }
    let Some(to) = to else { return usage(format!("extract needs a target module: {FORM}")) };
    if names.is_empty() {
        return usage(format!("extract needs the symbols to move: {FORM}"));
    }
    Ok(Args { names, to, to_project })
}

impl Store {
    /// Whether the project in `dir` and this one depend on each other by path.
    fn connected(&self, dir: &Path) -> bool {
        let (mine, theirs) = (self.root.canonicalize().ok(), dir.canonicalize().ok());
        match (mine, theirs) {
            (Some(m), Some(t)) => path_deps(&self.root).contains(&t) || path_deps(dir).contains(&m),
            _ => false,
        }
    }

    /// `extract <symbols…> --to <module>` (D-103).
    pub(crate) fn extract(&self, args: &[&str], budget: usize) -> Out {
        let p = parse(args)?;
        if p.to_project.is_some() {
            return fail("E5005 extract --to-project is not supported yet; extract moves symbols to a module of this project");
        }
        let ix = self.index()?;
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let members = workspace::members(&top).unwrap_or_default();
        let exists = |m: &str| ix.modules.iter().any(|r| r.path == m);

        // the target, in its forgiving forms
        let mut to = p.to.trim().trim_end_matches(".gleam").to_string();
        for pre in ["src/", "test/"] {
            if let Some(r) = to.strip_prefix(pre) {
                to = r.to_string();
                break;
            }
        }
        if !exists(&to) {
            if let Some((prefix, module)) = to.rsplit_once('/')
                && let Some(d) = members.iter().find(|d| **d != mine && (d.as_str() == prefix || d.ends_with(&format!("/{prefix}"))))
            {
                return fail(format!("use --to {module} --to-project {}", relative(&mine, d)));
            }
            if !valid_module_path(&to) {
                return fail(format!("{to} is not a module name; use --to <module>, e.g. --to solace_task"));
            }
        }

        // the symbols
        let mut listed: Vec<usize> = vec![];
        let mut already: Vec<String> = vec![];
        for name in &p.names {
            if name.ends_with(".gleam") {
                return fail(MODULE_HINT);
            }
            match self.resolve(&ix, name) {
                Ok(i) if ix.syms[i].module == to => already.push(ix.syms[i].qname()),
                Ok(i) => {
                    if !listed.contains(&i) {
                        listed.push(i)
                    }
                }
                Err(e) if e.0.starts_with("E1002") => {
                    let (m, n) = match name.rsplit_once('.') {
                        Some((m, n)) => (Some(m.replace('.', "/")), n),
                        None => (None, name.as_str()),
                    };
                    let found: Vec<usize> = (0..ix.syms.len())
                        .filter(|&i| {
                            let s = &ix.syms[i];
                            s.name == n && m.as_ref().is_none_or(|m| s.module == *m || s.module.ends_with(&format!("/{m}")))
                        })
                        .collect();
                    if found.iter().any(|&i| ix.syms[i].module == to) && found.iter().any(|&i| ix.syms[i].module != to) {
                        let list: Vec<String> = found.iter().map(|&i| ix.syms[i].qname()).collect();
                        return fail(format!("E1004 {name} resolves in both the source and {to}: {}; name the module, e.g. {}", list.join(", "), list[0]));
                    }
                    return Err(e);
                }
                Err(e) => {
                    if self.resolve_module(&ix, name).is_some() {
                        return fail(MODULE_HINT);
                    }
                    let last = name.rsplit('.').next().unwrap_or(name);
                    if ix.syms.iter().any(|s| s.module == to && s.name == last) {
                        already.push(format!("{to}.{last}"));
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        if listed.is_empty() {
            return Ok(format!("{} already in {to}; nothing changed.\n", already.join(", ")));
        }
        let target_is_test = tail(&to).ends_with("_test");
        if listed.iter().any(|&i| is_test(&ix, i)) && !target_is_test {
            return fail("test symbols go only to a test module (*_test): use --to <module>_test");
        }
        if listed.iter().any(|&i| !is_test(&ix, i)) && target_is_test {
            return fail("source symbols do not go to a test module (*_test): use --to <module>");
        }

        // a module that exists in a project connected through gleam.toml
        let mut notes: Vec<String> = vec![];
        if !exists(&to) {
            for d in members.iter().filter(|d| **d != mine) {
                let dir = scope::path(&top, d);
                if !["src", "test"].iter().any(|t| dir.join(t).join(format!("{to}.gleam")).exists()) {
                    continue;
                }
                let rel = relative(&mine, d);
                if self.connected(&dir) {
                    return fail(format!("{to} is in {rel}; add --to-project {rel}"));
                }
                notes.push(format!("Created module {to}; a module of that name exists in the unrelated project {rel}."));
            }
        }

        // what moves: the symbols, their helpers, the tests that reach only moved code
        let n = ix.syms.len();
        let mut moved = vec![false; n];
        for &i in &listed {
            moved[i] = true;
        }
        let mut helpers: Vec<usize> = vec![];
        bring_helpers(&ix, &mut moved, &mut helpers);
        let mut moved_tests: Vec<usize> = vec![];
        if !target_is_test {
            for t in reach::tests(&ix) {
                let c = reach::covered(&ix, t);
                if !moved[t] && !c.is_empty() && c.iter().all(|&u| moved[u]) {
                    moved_tests.push(t);
                }
            }
            for &t in &moved_tests {
                moved[t] = true;
            }
            let mut test_helpers = vec![];
            bring_helpers(&ix, &mut moved, &mut test_helpers);
            helpers.extend(test_helpers);
        }
        let test_to = if target_is_test { to.clone() } else { format!("{to}_test") };
        let newmod: Vec<String> = (0..n)
            .map(|i| match (moved[i], is_test(&ix, i)) {
                (false, _) => ix.syms[i].module.clone(),
                (true, true) => test_to.clone(),
                (true, false) => to.clone(),
            })
            .collect();

        // names taken in the target
        for i in (0..n).filter(|&i| moved[i]) {
            let s = &ix.syms[i];
            let clash = (0..n).find(|&j| {
                j != i
                    && newmod[j] == newmod[i]
                    && ix.syms[j].name == s.name
                    && (ix.syms[j].kind == Kind::Type) == (s.kind == Kind::Type)
                    && (!moved[j] || ix.syms[j].module != s.module)
            });
            if let Some(j) = clash {
                return fail(format!("E1004 {} already exists in {}; {} cannot move there", ix.syms[j].qname(), newmod[i], s.qname()));
            }
        }

        // an opaque type stays with the code that builds or matches its constructors
        for x in 0..n {
            let s = &ix.syms[x];
            if s.kind != Kind::Type || !s.text.lines().any(|l| l.starts_with("pub opaque type ")) {
                continue;
            }
            let users: Vec<usize> = ix.used_by[x].iter().copied().filter(|&j| constructs(&ix, j, x)).collect();
            let apart: Vec<String> = users.iter().filter(|&&j| moved[j] != moved[x]).map(|&j| ix.syms[j].qname()).collect();
            if !apart.is_empty() {
                let verb = if moved[x] { "extract them too" } else { "extract it too" };
                return fail(format!("E5006 {} is opaque; its constructors are used by {}; {verb}", s.name, apart.join(", ")));
            }
        }

        // a module cycle
        let graph = |at: &dyn Fn(usize) -> String| {
            let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for h in 0..n {
                for &u in &ix.uses[h] {
                    if at(h) != at(u) {
                        edges.entry(at(h)).or_default().insert(at(u));
                    }
                }
            }
            edges
        };
        if cyclic(&graph(&|i| newmod[i].clone())) && !cyclic(&graph(&|i| ix.syms[i].module.clone())) {
            let mut stay: Vec<(usize, Vec<String>)> = vec![];
            for m in (0..n).filter(|&m| moved[m]) {
                for &u in ix.uses[m].iter().filter(|&&u| !moved[u]) {
                    match stay.iter_mut().find(|(s, _)| *s == u) {
                        Some((_, by)) => by.push(ix.syms[m].name.clone()),
                        None => stay.push((u, vec![ix.syms[m].name.clone()])),
                    }
                }
            }
            let (mut seen, mut stack) = (HashSet::new(), stay.iter().map(|(u, _)| *u).collect::<Vec<_>>());
            while let Some(u) = stack.pop() {
                if seen.insert(u) {
                    stack.extend(ix.uses[u].iter().filter(|&&v| !moved[v]));
                }
            }
            if stay.is_empty() {
                let mut back: Vec<String> = vec![];
                for h in (0..n).filter(|&h| !moved[h]) {
                    for &u in ix.uses[h].iter().filter(|&&u| moved[u]) {
                        back.push(format!("{} (by {})", ix.syms[u].name, ix.syms[h].name));
                    }
                }
                return fail(format!("E5002 staying code uses {} moved symbols and closes an import loop: {}", back.len(), back.join(", ")));
            }
            let list: Vec<String> = stay.iter().map(|(u, by)| format!("{} (by {})", ix.syms[*u].name, by.join(", "))).collect();
            return fail(format!(
                "E5002 moved code uses {} symbol{} that stay: {} (moving them would bring {} in total)",
                stay.len(),
                if stay.len() == 1 { "" } else { "s" },
                list.join(", "),
                seen.len()
            ));
        }

        // projects that depend on this one and use a moved public symbol
        let names = name_map(&ix);
        let outside: HashMap<&(String, String), usize> =
            names.iter().filter(|&(_, &x)| moved[x] && is_pub_text(&ix.syms[x].text)).map(|(k, &x)| (k, x)).collect();
        let mut users: Vec<String> = vec![];
        for d in members.iter().filter(|d| **d != mine) {
            let dir = scope::path(&top, d);
            let Some(me) = self.root.canonicalize().ok() else { break };
            if !path_deps(&dir).contains(&me) {
                continue;
            }
            let Ok(other) = board::open(&dir) else { continue };
            let Ok(dix) = other.index() else { continue };
            for s in &dix.syms {
                let header = dix.modules.iter().find(|m| m.path == s.module).map_or("", |m| m.header.as_str());
                let imps = gleam::imports(header);
                for r in gleam::references(&s.text) {
                    let hit = match &r {
                        Ref::Qualified(alias, member) => imps
                            .iter()
                            .find(|i| &i.alias == alias)
                            .filter(|i| outside.contains_key(&(i.module.clone(), member.clone())))
                            .map(|i| format!("{}.{member}", i.module)),
                        Ref::Bare(b) => imps.iter().find_map(|i| {
                            i.unqualified
                                .iter()
                                .find(|(o, l)| l == b && outside.contains_key(&(i.module.clone(), o.clone())))
                                .map(|(o, _)| format!("{}.{o}", i.module))
                        }),
                    };
                    if let Some(hit) = hit {
                        let line = format!("{} uses {hit}; extracting across projects comes with --to-project support", s.qname());
                        if !users.contains(&line) {
                            users.push(line);
                        }
                    }
                }
            }
        }
        if !users.is_empty() {
            return fail(format!("E5003 a project that depends on this one uses moved symbols:\n{}", users.join("\n")));
        }

        // the rewrite
        let mut plan = Plan {
            ix: &ix,
            names: names.clone(),
            moved: moved.clone(),
            newmod: newmod.clone(),
            newtext: ix.syms.iter().map(|s| s.text.clone()).collect(),
            needs: BTreeMap::new(),
            edits: 0,
            edited: BTreeSet::new(),
        };
        plan.rewrite()?;
        let mut now_pub: Vec<usize> = vec![];
        for x in 0..n {
            if !is_pub_text(&plan.newtext[x]) && ix.used_by[x].iter().any(|&j| newmod[j] != newmod[x]) {
                plan.newtext[x] = make_pub(&plan.newtext[x]);
                now_pub.push(x);
            }
        }

        // modules to create
        let mut created: Vec<(String, &str)> = vec![];
        if !exists(&to) {
            created.push((to.clone(), if target_is_test { "test" } else { "src" }));
        }
        if !moved_tests.is_empty() && !target_is_test && !exists(&test_to) && !created.iter().any(|c| c.0 == test_to) {
            created.push((test_to.clone(), "test"));
        }
        let created_files: Vec<PathBuf> = created.iter().map(|(m, dir)| self.root.join(dir).join(format!("{m}.gleam"))).collect();
        if created_files.iter().any(|f| f.exists()) {
            return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
        }

        // what the suites name now, to list those that stop resolving
        let suites_before: Vec<(String, String)> =
            reach::entries(self)?.into_iter().filter(|(_, e)| self.resolve(&ix, e).is_ok()).collect();
        let before = self.broken()?;

        let old_modules: BTreeSet<String> = (0..n).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
        let suites_after = self.transaction(&created_files, || {
            for (path, dir) in &created {
                self.db.execute("INSERT INTO modules VALUES (?1, ?2, ?3)", params![path, dir, ""])?;
            }
            for i in 0..n {
                let s = &ix.syms[i];
                if moved[i] {
                    let ord: i64 = self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&newmod[i]], |r| r.get(0))?;
                    self.db.execute("UPDATE symbols SET module = ?2, ord = ?3 WHERE id = ?1", params![s.id, newmod[i], ord])?;
                    self.history(&s.id, "move", &s.module)?;
                }
                if plan.newtext[i] != s.text {
                    self.db.execute(
                        "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                        params![s.id, plan.newtext[i], revision(&plan.newtext[i])],
                    )?;
                    if !moved[i] {
                        self.history(&s.id, "edit", &s.name)?;
                    }
                }
            }
            // headers: the imports each module now needs, minus those the move made unused
            let (mut before_used, mut after_used): (HashMap<String, HashSet<String>>, HashMap<String, HashSet<String>>) = Default::default();
            for i in 0..n {
                before_used.entry(ix.syms[i].module.clone()).or_default().extend(locals_used(&ix.syms[i].text));
                after_used.entry(newmod[i].clone()).or_default().extend(locals_used(&plan.newtext[i]));
            }
            let mut headers: Vec<(String, String)> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
            headers.extend(created.iter().map(|(m, _)| (m.clone(), String::new())));
            let mut finals: Vec<(String, String)> = vec![];
            for (m, header) in &headers {
                let empty = HashSet::new();
                let (was, now) = (before_used.get(m).unwrap_or(&empty), after_used.get(m).unwrap_or(&empty));
                let keep_item = |module: &str, raw: &str| {
                    let (orig, local) = item_split(raw);
                    if plan.names.get(&(module.to_string(), orig)).is_some_and(|&x| moved[x]) {
                        return false;
                    }
                    !(was.contains(&local) && !now.contains(&local))
                };
                let keep_import = |_: &str, alias: &str, dropped: bool| {
                    if dropped || was.contains(alias) { now.contains(alias) } else { true }
                };
                let mut h = header.clone();
                for need in plan.needs.get(m).into_iter().flatten() {
                    match &need.alias {
                        Some(a) => {
                            if !gleam::imports(&h).iter().any(|i| i.module == need.module && &i.alias == a) {
                                h = add_import(&h, &format!("import {} as {a}", need.module));
                            }
                        }
                        None => h = merge_import(&h, &need.module, &need.items),
                    }
                }
                let edited = edit_imports(&h, &keep_item, &keep_import);
                if edited != *header {
                    let formatted = format_gleam(&edited).map_or(edited.clone(), |f| f.trim().to_string());
                    self.set_header(m, &formatted)?;
                    finals.push((m.clone(), formatted));
                } else {
                    finals.push((m.clone(), header.clone()));
                }
            }
            // a source module the move emptied goes, unless another module imports it
            let mut retired = vec![];
            for m in &old_modules {
                let empty = !(0..n).any(|i| newmod[i] == *m);
                let imported = finals.iter().any(|(o, h)| o != m && gleam::imports(h).iter().any(|i| i.module == *m));
                if empty && !imported {
                    self.db.execute("DELETE FROM modules WHERE path = ?1", [m])?;
                    retired.push(m.clone());
                }
            }
            let ix2 = self.index()?;
            let diags = self.check_all(&ix2)?;
            let bad: Vec<&str> = diags
                .iter()
                .filter(|d| d.error && d.symbol.as_ref().is_none_or(|s| !before.contains_key(s)))
                .map(|d| d.text.as_str())
                .collect();
            if !bad.is_empty() {
                return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", bad.join("\n\n")));
            }
            let gone: Vec<String> = suites_before.iter().filter(|(_, e)| self.resolve(&ix2, e).is_err()).map(|(s, e)| format!("suite {s} entry {e}")).collect();
            Ok(format!("{}\u{1}{}", gone.join("\u{2}"), retired.join("\u{2}")))
        })?;
        let (gone, retired) = suites_after.split_once('\u{1}').unwrap_or(("", ""));

        // the reply: names and counts
        let q = |i: usize| ix.syms[i].qname();
        let mut out = format!("Moved {}", listed.iter().map(|&i| q(i)).collect::<Vec<_>>().join(", "));
        if !helpers.is_empty() {
            let names: Vec<&str> = helpers.iter().map(|&i| ix.syms[i].name.as_str()).collect();
            out += &format!(" (+{} helper{}: {})", helpers.len(), if helpers.len() == 1 { "" } else { "s" }, names.join(", "));
        }
        out += &format!(" to {to}\n");
        if !already.is_empty() {
            out += &format!("Already in {to}: {}\n", already.join(", "));
        }
        if !now_pub.is_empty() {
            out += &format!("Now pub: {} (new revision)\n", now_pub.iter().map(|&i| ix.syms[i].name.as_str()).collect::<Vec<_>>().join(", "));
        }
        if plan.edits > 0 {
            out += &format!("Rewrote {} reference{} in {} module{}\n", plan.edits, if plan.edits == 1 { "" } else { "s" }, plan.edited.len(), if plan.edited.len() == 1 { "" } else { "s" });
        }
        if !moved_tests.is_empty() {
            out += &format!("Tests moved: {}\n", moved_tests.iter().map(|&t| format!("{} -> {}", q(t), newmod[t])).collect::<Vec<_>>().join(", "));
        }
        for m in created.iter().map(|c| &c.0) {
            out += &format!("Created module {m}\n");
        }
        if !retired.is_empty() {
            out += &format!("Retired module {}\n", retired.replace('\u{2}', ", "));
        }
        for note in notes {
            out += &format!("{note}\n");
        }
        for g in gone.split('\u{2}').filter(|g| !g.is_empty()) {
            out += &format!("Not rewritten: {g} no longer resolves\n");
        }
        // places outside the code that name a moved symbol, in qualified form only
        let mut needles: Vec<String> = (0..n).filter(|&i| moved[i]).map(|i| format!("{}.{}", ix.syms[i].module, ix.syms[i].name)).collect();
        needles.extend(old_modules.iter().map(|m| format!("-m {m}")));
        let (mut hits, mut files): (Vec<String>, BTreeSet<String>) = (vec![], BTreeSet::new());
        if let Ok(settings) = lockdown::settings(&self.root)
            && let Ok(paths) = lockdown::watched_paths(&self.root, &settings)
        {
            for path in paths {
                let Ok(text) = fs::read_to_string(self.root.join(&path)) else { continue };
                for (k, line) in text.lines().enumerate() {
                    let found = needles.iter().any(|nd| {
                        line.match_indices(nd.as_str()).any(|(at, _)| {
                            let word = |c: char| c.is_alphanumeric() || c == '_' || c == '/';
                            !line[..at].ends_with(word) && !line[at + nd.len()..].starts_with(word)
                        })
                    });
                    if found {
                        hits.push(format!("{path}:{}", k + 1));
                        files.insert(path.clone());
                    }
                }
            }
        }
        if !hits.is_empty() {
            let shown: Vec<&str> = hits.iter().take(5).map(String::as_str).collect();
            let more = if hits.len() > 5 { ", …" } else { "" };
            out += &format!("Not rewritten: {} place{} in {} file{} ({}{more})\n", hits.len(), if hits.len() == 1 { "" } else { "s" }, files.len(), if files.len() == 1 { "" } else { "s" }, shown.join(", "));
        }
        Ok(lockdown::trim_output(&out, budget))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    const BOARD: &str = "pub type Task {\n  Task(id: Int, lane: Int)\n}\n\npub fn new_task(id: Int) -> Task {\n  Task(id, lane_of(id))\n}\n\nfn lane_of(id: Int) -> Int {\n  id % 3\n}\n\npub fn bump(t: Task) -> Task {\n  let _note = \"a Task here\" // Task\n  Task(..t, lane: t.lane + 1)\n}\n\npub fn lane(t: Task) -> Int {\n  case t {\n    Task(lane: l, ..) -> l\n  }\n}\n";

    fn text(s: &Store, name: &str) -> String {
        let ix = s.index().unwrap();
        ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
    }
    fn header(s: &Store, module: &str) -> String {
        s.index().unwrap().modules.iter().find(|m| m.path == module).map(|m| m.header.clone()).unwrap_or_default()
    }
    fn extract(s: &Store, args: &[&str]) -> Out {
        s.extract(args, usize::MAX)
    }

    /// A type moves; annotations, constructor calls, patterns, record updates
    /// and a reference after a string literal or comment are all rewritten,
    /// and the symbol keeps its identity.
    #[test]
    fn rewrites_every_position_and_keeps_identity() {
        let store = project(&[("src/board.gleam", BOARD)]);
        let ix = store.index().unwrap();
        let id = ix.syms[store.resolve(&ix, "board.Task").unwrap()].id.clone();
        let out = extract(&store, &["board.Task", "--to", "src/task.gleam"]).unwrap();
        assert!(out.starts_with("Moved board.Task to task\n"), "{out}");
        assert!(out.contains("Created module task") && out.contains("Rewrote 7 references in 1 module"), "{out}");
        let ix = store.index().unwrap();
        let moved = &ix.syms[store.resolve(&ix, "task.Task").unwrap()];
        assert_eq!(moved.id, id);
        assert_eq!(moved.module, "task");
        assert!(text(&store, "board.new_task").contains("-> task.Task {\n  task.Task(id, lane_of(id))"), "{}", text(&store, "board.new_task"));
        let bump = text(&store, "board.bump");
        assert!(bump.contains("(t: task.Task) -> task.Task") && bump.contains("\"a Task here\" // Task") && bump.contains("task.Task(..t, lane: t.lane + 1)"), "{bump}");
        assert!(text(&store, "board.lane").contains("task.Task(lane: l, ..) -> l"));
        assert!(header(&store, "board").contains("import task"));
        // run again: nothing to do
        let again = extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(again, "task.Task already in task; nothing changed.\n");
    }

    /// An unqualified import follows the symbol, constructor-only items too,
    /// and the old import goes when nothing else uses it.
    #[test]
    fn unqualified_imports_move_with_the_symbol() {
        let app = "import board.{type Task, Task}\n\npub fn mk() -> Task {\n  Task(1, 2)\n}\n";
        let only_ctor = "import board.{Task}\n\npub fn mk2(id: Int) {\n  Task(id, 1)\n}\n";
        let store = project(&[("src/board.gleam", BOARD), ("src/app.gleam", app), ("src/other.gleam", only_ctor)]);
        extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(text(&store, "app.mk"), "pub fn mk() -> Task {\n  Task(1, 2)\n}");
        assert!(header(&store, "app").contains("import task.{type Task, Task}") && !header(&store, "app").contains("board"), "{}", header(&store, "app"));
        assert!(header(&store, "other").contains("import task.{Task}") && !header(&store, "other").contains("board"), "{}", header(&store, "other"));
    }

    /// Moved code names staying code through its module, and `target.x` is bare;
    /// a private symbol that moved code uses becomes pub (new revision).
    #[test]
    fn moved_code_refers_back_and_target_references_become_bare() {
        let board = "pub type Task {\n  Task(id: Int)\n}\n\nfn lane_order(t: Task) -> Int {\n  t.id\n}\n\npub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\npub fn first(t: Task) -> Int {\n  lane_order(t)\n}\n\npub fn twice(n: Int) -> Int {\n  util.double(n)\n}\n";
        let store = project(&[("src/board.gleam", &format!("import util\n\n{board}")), ("src/util.gleam", "pub fn double(n: Int) -> Int {\n  n * 2\n}\n")]);
        let out = extract(&store, &["board.sort", "--to", "task"]).unwrap();
        assert!(out.contains("Now pub: lane_order (new revision)") && !out.contains("helper"), "{out}");
        let sort = text(&store, "task.sort");
        assert!(sort.contains("board.lane_order(a) <= board.lane_order(b)") && sort.contains("a: board.Task"), "{sort}");
        assert!(header(&store, "task").contains("import board"));
        assert!(text(&store, "board.lane_order").starts_with("pub fn lane_order"));
        // `util.double` is bare once `twice` moves into util
        let out = extract(&store, &["board.twice", "--to", "util"]).unwrap();
        assert!(out.contains("Rewrote 1 reference in 1 module"), "{out}");
        assert!(text(&store, "util.twice").contains("  double(n)"), "{}", text(&store, "util.twice"));
        assert!(!header(&store, "board").contains("util"), "{}", header(&store, "board"));
    }

    /// Private helpers only the moved symbol uses come along; a helper used
    /// from both sides stays and becomes pub.
    #[test]
    fn private_helpers_come_along_or_become_pub() {
        let store = project(&[("src/board.gleam", BOARD)]);
        let out = extract(&store, &["board.new_task", "--to", "task"]).unwrap();
        assert!(out.contains("Moved board.new_task (+1 helper: lane_of) to task"), "{out}");
        assert!(store.resolve(&store.index().unwrap(), "task.lane_of").is_ok());
        let shared = format!("{BOARD}\npub fn reset(id: Int) -> Int {{\n  lane_of(id) + 0\n}}\n");
        let store = project(&[("src/board.gleam", &shared)]);
        let out = extract(&store, &["board.new_task", "--to", "task"]).unwrap();
        assert!(!out.contains("helper") && out.contains("Now pub: lane_of (new revision)"), "{out}");
        assert!(text(&store, "task.new_task").contains("board.lane_of(id)"));
    }

    /// A qualifier that is a local name is never aliased around: the
    /// unqualified import is used instead.
    #[test]
    fn a_qualifier_that_is_a_local_name_gets_the_unqualified_form() {
        let board = format!("{BOARD}\npub fn make(task: Int) -> Task {{\n  Task(task, 1)\n}}\n");
        let store = project(&[("src/board.gleam", &board)]);
        extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(text(&store, "board.make"), "pub fn make(task: Int) -> Task {\n  Task(task, 1)\n}");
        assert!(header(&store, "board").contains("import task.{type Task, Task}"), "{}", header(&store, "board"));
    }

    /// A name the target already defines is refused, naming it.
    #[test]
    fn a_name_taken_in_the_target_is_refused() {
        let store = project(&[("src/board.gleam", "pub fn show() -> Int {\n  1\n}\n"), ("src/task.gleam", "pub fn show() -> Int {\n  2\n}\n")]);
        let err = extract(&store, &["board.show", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E1004 task.show already exists in task"), "{err}");
    }

    /// Moving code that staying code calls, and that calls staying code, would
    /// make a module loop: refused with every boundary use, then the retry works.
    #[test]
    fn a_module_cycle_is_refused_with_every_boundary_use() {
        let board = "pub type Task {\n  Task(id: Int)\n}\n\npub fn lane_order(t: Task) -> Int {\n  t.id\n}\n\npub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\npub fn top(a: Task, b: Task) -> Bool {\n  sort(a, b)\n}\n";
        let store = project(&[("src/board.gleam", board)]);
        let err = extract(&store, &["board.sort", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E5002 moved code uses 2 symbols that stay: "), "{err}");
        assert!(err.contains("lane_order (by sort)") && err.contains("Task (by sort)") && err.contains("would bring 2 in total"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.sort").is_ok(), "nothing changed");
        extract(&store, &["board.sort", "board.lane_order", "board.Task", "--to", "task"]).unwrap();
        assert!(text(&store, "board.top").contains("task.sort(a, b)"));
    }

    /// An opaque type stays with the code that builds it.
    #[test]
    fn an_opaque_type_is_refused_apart_from_its_constructors() {
        let board = "pub opaque type Box {\n  Box(n: Int)\n}\n\npub fn new_box(n: Int) -> Box {\n  Box(n)\n}\n\npub fn open(b: Box) -> Box {\n  b\n}\n";
        let store = project(&[("src/board.gleam", board)]);
        let err = extract(&store, &["board.Box", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("Box is opaque; its constructors are used by board.new_box; extract them too"), "{err}");
        extract(&store, &["board.Box", "board.new_box", "--to", "task"]).unwrap();
        assert!(text(&store, "board.open").contains("task.Box"));
    }

    /// A test that reaches only moved code moves to `<target>_test`; one that
    /// also reaches staying code is rewritten in place.
    #[test]
    fn tests_split_by_what_they_reach() {
        let board = format!("{BOARD}\npub fn keep() -> Int {{\n  1\n}}\n");
        let tests = "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n\npub fn mixed_test() {\n  let t = board.Task(1, 2)\n  assert board.keep() == t.id\n}\n";
        let store = project(&[("src/board.gleam", &board), ("test/board_test.gleam", tests)]);
        let out = extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert!(out.contains("Tests moved: board_test.round_trip_test -> task_test"), "{out}");
        assert!(store.resolve(&store.index().unwrap(), "task_test.round_trip_test").is_ok());
        assert!(text(&store, "board_test.mixed_test").contains("task.Task(1, 2)"));
        assert!(header(&store, "board_test").contains("import task"), "{}", header(&store, "board_test"));
        assert!(store.index().unwrap().modules.iter().any(|m| m.path == "task_test" && m.dir == "test"));
    }

    /// An emptied source module is retired, with its file; importers follow.
    #[test]
    fn an_emptied_source_is_retired() {
        let store = project(&[("src/board.gleam", "pub fn one() -> Int {\n  1\n}\n"), ("src/other.gleam", "import board\n\npub fn two() -> Int {\n  board.one() + 1\n}\n")]);
        let out = extract(&store, &["board.one", "--to", "task"]).unwrap();
        assert!(out.contains("Retired module board"), "{out}");
        assert!(!store.root.join("src/board.gleam").exists() && store.root.join("src/task.gleam").exists());
        assert!(text(&store, "other.two").contains("task.one() + 1") && !header(&store, "other").contains("board"));
        assert_eq!(extract(&store, &["board.one", "--to", "task"]).unwrap(), "task.one already in task; nothing changed.\n");
    }

    /// A result that does not compile changes nothing.
    #[test]
    fn a_failed_type_check_leaves_the_store_unchanged() {
        // the target imports the source without using it, so the move closes a real import cycle
        let store = project(&[
            ("src/board.gleam", "pub fn a() -> Int {\n  1\n}\n\npub fn b() -> Int {\n  a()\n}\n"),
            ("src/task.gleam", "import board\n\npub fn z() -> Int {\n  0\n}\n"),
        ]);
        let err = extract(&store, &["board.a", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("Not moved: the result does not type-check, so nothing changed."), "{err}");
        let ix = store.index().unwrap();
        assert!(store.resolve(&ix, "board.a").is_ok() && store.resolve(&ix, "task.a").is_err());
        assert_eq!(text(&store, "board.b"), "pub fn b() -> Int {\n  a()\n}");
        assert!(store.root.join("src/board.gleam").exists());
        assert!(fs::read_to_string(store.root.join("src/board.gleam")).unwrap().contains("pub fn a()"));
    }

    /// Operand forms: a module in the list, tests and sources apart, a missing name.
    #[test]
    fn operands_are_checked() {
        let tests = "pub fn only_test() {\n  assert 1 == 1\n}\n";
        let store = project(&[("src/board.gleam", BOARD), ("test/board_test.gleam", tests)]);
        let err = extract(&store, &["board.gleam", "--to", "task"]).unwrap_err().0;
        assert_eq!(err, MODULE_HINT);
        let err = extract(&store, &["board", "--to", "task"]).unwrap_err().0;
        assert_eq!(err, MODULE_HINT);
        let err = extract(&store, &["board_test.only_test", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("test symbols go only to a test module"), "{err}");
        let err = extract(&store, &["board.lane", "--to", "task_test"]).unwrap_err().0;
        assert!(err.contains("source symbols do not go to a test module"), "{err}");
        let err = extract(&store, &["board.nothing", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E1001"), "{err}");
        assert!(extract(&store, &["board.lane"]).unwrap_err().0.contains("extract needs a target module"));
        assert!(extract(&store, &["board.lane", "--to", "task", "--to-project", "../x"]).unwrap_err().0.starts_with("E5005"));
        // a test symbol goes to a test module
        extract(&store, &["board_test.only_test", "--to", "test/task_test.gleam"]).unwrap();
        assert!(store.resolve(&store.index().unwrap(), "task_test.only_test").is_ok());
    }

    /// A suite entry that stops resolving, and a watched file that names the
    /// moved symbol, are listed.
    #[test]
    fn suites_and_watched_files_are_named() {
        let toml = "[commands]\nghost = { run = [\"make\"], checks = [\"board.main\"] }\n";
        let script = "gleam run -m board\nboard.main()\nunrelated\n";
        let store = project(&[("src/board.gleam", "pub fn main() {\n  Nil\n}\n"), ("solace.toml", toml), ("script.py", script)]);
        let out = extract(&store, &["board.main", "--to", "task"]).unwrap();
        assert!(out.contains("Not rewritten: suite ghost entry board.main no longer resolves"), "{out}");
        assert!(out.contains("Not rewritten: 2 places in 1 file (script.py:1, script.py:2)"), "{out}");
    }

    /// A target that lives in a connected project needs `--to-project`; a project
    /// that depends on this one and uses a moved symbol is named.
    #[test]
    fn other_projects_are_named() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import board\n\npub fn view(t: board.Task) -> Int {\n  t.id\n}\n"),
        ]);
        for d in ["web", "components", "front"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.lane", "--to", "solace_task"]).unwrap_err().0;
        assert_eq!(err, "solace_task is in ../components; add --to-project ../components");
        let err = extract(&store, &["board.lane", "--to", "components/solace_task"]).unwrap_err().0;
        assert_eq!(err, "use --to solace_task --to-project ../components");
        let err = extract(&store, &["board.Task", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("front.view uses board.Task; extracting across projects comes with --to-project support"), "{err}");
    }

    #[test]
    fn helpers_for_paths() {
        assert_eq!(relative("solace-cli/board", "solace-cli/components"), "../components");
        assert_eq!(relative(".", "a/b"), "a/b");
        assert_eq!(item_split("type Task"), ("Task".to_string(), "Task".to_string()));
        assert_eq!(item_split("Foo as Bar"), ("Foo".to_string(), "Bar".to_string()));
        assert_eq!(make_pub("// doc\nfn f() {\n  1\n}"), "// doc\npub fn f() {\n  1\n}");
    }
}
