use super::*;
use crate::Sym;
use crate::Index;
use crate::fn_body_start;
use crate::failure::Failure;
use crate::failure::fail;
use crate::failure::usage;
use crate::language::Import;
use crate::language::Ref;
use crate::store_write::import_statements;
use crate::Store;
use crate::is_test;
use crate::extract::MODULE_HINT;
use crate::extract::deps::depends_on;
use crate::extract::deps::covered;
use crate::extract::deps::package_graph;
use crate::extract::deps::Dest;
use crate::extract::deps::Dependent;
use crate::extract::ffi::module_dir;
use crate::extract::ffi::ffi_norm;
use crate::extract::ffi::ffi_rel;
use crate::extract::ffi::ffi_spans;
use crate::extract::ffi::Carry;
use crate::store_write::merge_import;
use crate::store_write::add_import;
pub(super) fn tail(module: &str) -> String {
    module.rsplit('/').next().unwrap_or(module).to_string()
}

/// An import a module needs after the move: `import <module> [as <alias>]`,
/// with unqualified `items` as written (`type Task`, `Some`).
#[derive(Clone, Debug)]
pub(crate) struct Need {
    pub(crate) module: String,
    pub(crate) alias: Option<String>,
    pub(crate) items: Vec<String>,
}

/// How a reference reached its symbol.
#[derive(Clone)]
pub(crate) enum Via {
    /// Bare, in the symbol's own module.
    Same,
    /// `alias.member`.
    Qual(String),
    /// Bare through an unqualified import; the import's items for the name, as written.
    Unq(Vec<String>),
}

pub(crate) struct Resolved {
    pub(crate) x: Option<usize>,
    pub(crate) member: String,
    pub(crate) via: Via,
    /// The imported module the reference went through.
    pub(crate) module: Option<String>,
}

/// `type Task` -> (`Task`, `Task`); `Foo as Bar` -> (`Foo`, `Bar`).
pub(crate) fn item_split(raw: &str) -> (String, String) {
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

pub(crate) fn is_pub_text(text: &str) -> bool {
    text.lines().find(|l| gleam::declaration(l).is_some()).is_some_and(|l| l.starts_with("pub "))
}

pub(crate) fn make_pub(text: &str) -> String {
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

/// Whether the reference at `a..b` in `text` names a type rather than a value:
/// anywhere before `head` (a signature, a type's body, a constant's
/// annotation), and in a body after `:`, as a `-> T {` return type, or as an
/// argument of a type that is itself in a type position.
fn type_position(text: &str, a: usize, b: usize, head: usize) -> bool {
    // strings and comments blanked once, so their brackets and words count for nothing; offsets stay
    a < head || in_type_position(&gleam::strip_comments(text), a, b, head)
}

/// `type_position` on `text` with its strings and comments already blanked.
fn in_type_position(text: &str, a: usize, b: usize, head: usize) -> bool {
    if a < head {
        return true;
    }
    let before = text[..a].trim_end();
    if before.ends_with("::") {
        return false;
    }
    if before.ends_with(':') {
        return !is_label(&before[..before.len() - 1]);
    }
    if before.ends_with("->") {
        // an anonymous function's return type, its arguments skipped, comes before its body
        let mut rest = text[b..].trim_start();
        if rest.starts_with('(') {
            let mut depth = 0;
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            rest = &rest[i + 1..];
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        return rest.trim_start().starts_with('{');
    }
    if !(before.ends_with('(') || before.ends_with(',')) {
        return false;
    }
    // the bracket the reference is an argument of
    let mut depth = 0;
    let mut open = None;
    for (i, c) in text[..a].char_indices().rev() {
        match c {
            ')' | ']' => depth += 1,
            '(' | '[' if depth > 0 => depth -= 1,
            '(' => {
                open = Some(i);
                break;
            }
            '[' | '{' => return false,
            _ => {}
        }
    }
    let Some(p) = open else { return false };
    let start = text[..p].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.').len();
    let name = &text[start..p];
    if name == "fn" {
        // a function type's arguments; an anonymous function's are lowercase names
        return true;
    }
    if name.is_empty() && text[..p].ends_with('#') {
        return in_type_position(text, p - 1, p, head);
    }
    name.rsplit('.').next().is_some_and(|n| n.starts_with(|c: char| c.is_uppercase())) && in_type_position(text, start, p, head)
}

/// Whether the `:` that follows `before` ends a label of a call, a record update
/// or a pattern (`f(state: x)`) rather than a parameter or `let` annotation
/// (`fn f(state: T)`, `fn(state: T)`, `let state: T`).
fn is_label(before: &str) -> bool {
    let mut depth = 0;
    for (i, c) in before.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' if depth > 0 => depth -= 1,
            '(' => {
                let head = before[..i].trim_end();
                if head.ends_with('#') {
                    return false;
                }
                let word_start = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
                let word = &head[word_start..];
                if word.is_empty() || word == "fn" {
                    return false;
                }
                // after the keyword `fn`, not a word that ends in it
                let kw = head[..word_start].trim_end();
                let is_fn = kw.strip_suffix("fn").is_some_and(|r| !r.ends_with(|c: char| c.is_alphanumeric() || c == '_'));
                return !is_fn;
            }
            '[' | '{' => return false,
            _ => {}
        }
    }
    false
}

/// Text with `edits` (span, replacement) applied.
pub(crate) fn spliced(text: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut t = text.to_string();
    for (a, b, r) in edits {
        t.replace_range(a..b, &r);
    }
    t
}

pub(crate) fn name_map(ix: &Index) -> HashMap<(String, String), usize> {
    let mut names = HashMap::new();
    for (i, s) in ix.syms.iter().enumerate() {
        names.insert((s.module.clone(), s.name.clone()), i);
    }
    for (i, s) in ix.syms.iter().enumerate() {
        if s.kind == Kind::Type {
            for c in ix.lang.constructors(&s.text) {
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

fn resolve_ref(r: &Ref, oh: &str, imps: &[Import], header: &str, names: &HashMap<(String, String), usize>) -> Option<Resolved> {
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
pub(crate) fn locals_used(text: &str) -> HashSet<String> {
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
pub(crate) fn edit_imports(header: &str, keep_item: &dyn Fn(&str, &str) -> bool, keep_import: &dyn Fn(&str, &str, bool) -> bool) -> String {
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
pub(crate) fn cyclic(edges: &BTreeMap<String, BTreeSet<String>>) -> bool {
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
pub(crate) fn constructs(ix: &Index, j: usize, ty: usize) -> bool {
    let ctors = ix.lang.constructors(&ix.syms[ty].text);
    let s = &ix.syms[j];
    let body = fn_body_start(s).filter(|_| s.kind == Kind::Fn);
    let code = ix.lang.strip_comments(&s.text);
    ix.lang.references_spans(&s.text).into_iter().any(|(r, a, _)| {
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
pub(crate) struct Plan<'a> {
    pub(crate) ix: &'a Index,
    pub(crate) names: HashMap<(String, String), usize>,
    /// Each module's imports before the move, the destination's target modules too.
    pub(crate) headers: HashMap<String, String>,
    /// (module, name) the destination's target modules already define.
    pub(crate) foreign: HashSet<(String, String)>,
    pub(crate) moved: Vec<bool>,
    pub(crate) newmod: Vec<String>,
    pub(crate) newtext: Vec<String>,
    pub(crate) needs: BTreeMap<String, Vec<Need>>,
    /// The symbols the rewrite edits: those before it; a dependent's plan
    /// resolves into the source's symbols after them.
    pub(crate) upto: usize,
    pub(crate) edits: usize,
    pub(crate) edited: BTreeSet<String>,
    /// A merge (D-117): every reference to the first goes to the second, which
    /// stays where it is; the first is retired and its text not rewritten.
    pub(crate) redirect: Option<(usize, usize)>,
    /// The references a merge redirected, and the modules they are in.
    pub(crate) hits: usize,
    pub(crate) hit_in: BTreeSet<String>,
}

impl Plan<'_> {
    fn old_imports(&self, m: &str) -> Vec<Import> {
        self.headers.get(m).map_or(vec![], |h| self.ix.lang.imports(h))
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
        self.foreign.contains(&(m.to_string(), name.to_string()))
            || self.ix.syms.iter().enumerate().any(|(i, s)| {
                // a merge's retired symbol is gone from its module
                self.redirect.is_none_or(|(r, _)| r != i)
                    && self.newmod[i] == m
                    && (s.name == name || (s.kind == Kind::Type && self.ix.lang.constructors(&s.text).iter().any(|c| c == name)))
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
    pub(crate) fn rewrite(&mut self) -> Result<(), Failure> {
        let ix = self.ix;
        for h in 0..self.upto {
            if self.redirect.is_some_and(|(r, _)| r == h) {
                continue;
            }
            let (oh, nh) = (ix.syms[h].module.clone(), self.newmod[h].clone());
            let imps = self.old_imports(&oh);
            let header = ix.modules.iter().find(|m| m.path == oh).map_or("", |m| m.header.as_str());
            let text = ix.syms[h].text.clone();
            // where the declaration's types end and its values begin
            let head = match ix.syms[h].kind {
                Kind::Fn => fn_body_start(&ix.syms[h]).unwrap_or(text.len()),
                Kind::Type => text.len(),
                Kind::Const => text.find('=').unwrap_or(text.len()),
            };
            let mut bound: Option<HashSet<String>> = None;
            let mut edits: Vec<(usize, usize, String)> = vec![];
            for (r, a, b) in ix.lang.references_spans(&text) {
                let Some(mut res) = resolve_ref(&r, &oh, &imps, header, &self.names) else { continue };
                // a merge's redirect: the survivor, under its own name when that differs
                let (mut hit, mut renamed) = (false, false);
                if let Some((gone, keep)) = self.redirect
                    && res.x == Some(gone)
                {
                    res.x = Some(keep);
                    // a constructor named like the retired type keeps its name (the
                    // survivor's constructors match): one in a value or pattern, not a type
                    let ctor_alike = ix.syms[gone].kind == Kind::Type
                        && ix.lang.constructors(&ix.syms[gone].text).contains(&ix.syms[gone].name)
                        && !type_position(&text, a, b, head);
                    if res.member == ix.syms[gone].name && ix.syms[keep].name != ix.syms[gone].name && !ctor_alike {
                        res.member = ix.syms[keep].name.clone();
                        renamed = true;
                    }
                    hit = true;
                    self.hits += 1;
                    self.hit_in.insert(oh.clone());
                }
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
                if nh == oh && mx == ox && !hit {
                    continue;
                }
                let member = res.member.clone();
                let bound = bound.get_or_insert_with(|| ix.lang.bound_names(&text));
                let slice = text[a..b].to_string();
                if nh == mx {
                    if (matches!(res.via, Via::Qual(_)) || renamed) && bound.contains(&member) {
                        return fail(format!(
                            "E5004 {}.{} uses {member} next to a local name {member}; rename the local first",
                            ix.syms[h].module, ix.syms[h].name
                        ));
                    }
                    if slice != member {
                        edits.push((a, b, member));
                    }
                    continue;
                }
                // an unqualified import keeps its items, unless a merge changes the name
                if let Via::Unq(raw) = &res.via
                    && !renamed
                {
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
                            concat!(
                                "E5004 {} cannot refer to {}.{}: the qualifier {} clashes with an import, ",
                                "a definition or a local name, and {} is taken too",
                            ),
                            nh, mx, member, alias, member
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

pub(crate) struct Args {
    pub(crate) names: Vec<String>,
    pub(crate) to: String,
    pub(crate) to_project: Option<String>,
}

pub(crate) fn parse(args: &[&str]) -> Result<Args, Failure> {
    const FORM: &str = "extract <symbol>... --to <module>";
    let (mut names, mut to, mut to_project) = (vec![], None, None);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--to" => to = Some(it.next().ok_or_else(|| Failure(format!("--to needs a module: {FORM}"), 2))?.to_string()),
            "--to-project" => {
                to_project = Some(
                    it.next().ok_or_else(|| Failure(format!("--to-project needs a folder: {FORM} --to-project <folder>"), 2))?.to_string(),
                )
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

/// Writes to its store the imports each of `modules` needs after the move,
/// minus those the move made unused; returns each module's header after.
pub(crate) fn write_headers(plan: &Plan, modules: &[(String, String, &Store)]) -> Result<Vec<(String, String)>, Failure> {
    let ix = plan.ix;
    let (mut before_used, mut after_used): (HashMap<String, HashSet<String>>, HashMap<String, HashSet<String>>) = Default::default();
    for i in 0..ix.syms.len() {
        before_used.entry(ix.syms[i].module.clone()).or_default().extend(locals_used(&ix.syms[i].text));
        after_used.entry(plan.newmod[i].clone()).or_default().extend(locals_used(&plan.newtext[i]));
    }
    let mut finals: Vec<(String, String)> = vec![];
    for (m, header, st) in modules {
        let empty = HashSet::new();
        let (was, now) = (before_used.get(m).unwrap_or(&empty), after_used.get(m).unwrap_or(&empty));
        let keep_item = |module: &str, raw: &str| {
            let (orig, local) = item_split(raw);
            if plan.names.get(&(module.to_string(), orig)).is_some_and(|&x| plan.moved[x]) {
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
                    if !ix.lang.imports(&h).iter().any(|i| i.module == need.module && &i.alias == a) {
                        h = add_import(&h, &format!("import {} as {a}", need.module));
                    }
                }
                None => h = merge_import(&h, &need.module, &need.items),
            }
        }
        let edited = edit_imports(&h, &keep_item, &keep_import);
        if edited != *header {
            let formatted = ix.lang.format(&edited).map_or(edited.clone(), |f| f.trim().to_string());
            st.set_header(m, &formatted)?;
            finals.push((m.clone(), formatted));
        } else {
            finals.push((m.clone(), header.clone()));
        }
    }
    Ok(finals)
}

/// Private symbols whose users all move come along (a fixpoint over `used_by`).
pub(crate) fn bring_helpers(ix: &Index, moved: &mut [bool], out: &mut Vec<usize>) {
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

pub(crate) fn resolve_listed(s: &Store, ix: &Index, names: &[String], dest: &Option<Dest>, to: &str) -> Result<(Vec<usize>, Vec<String>), Failure> {
    let in_target = |m: &str, name: &str| match dest {
        Some(d) => d.ix.syms.iter().any(|s| s.module == m && s.name == name),
        None => ix.syms.iter().any(|s| s.module == m && s.name == name),
    };
    let mut listed: Vec<usize> = vec![];
    let mut already: Vec<String> = vec![];
    for name in names {
        if name.ends_with(".gleam") {
            return fail(MODULE_HINT);
        }
        match s.resolve(ix, name) {
            Ok(i) if dest.is_none() && ix.syms[i].module == to => already.push(ix.syms[i].qname()),
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
                    return fail(format!(
                        "E1004 {name} resolves in both the source and {to}: {}; name the module, e.g. {}",
                        list.join(", "),
                        list[0]
                    ));
                }
                return Err(e);
            }
            Err(e) => {
                if s.resolve_module(ix, name).is_some() {
                    return fail(MODULE_HINT);
                }
                let last = name.rsplit('.').next().unwrap_or(name);
                if in_target(to, last) {
                    already.push(format!("{to}.{last}"));
                } else {
                    return Err(e);
                }
            }
        }
    }
    Ok((listed, already))
}

/// What an extract decided before its transaction: what moves, where to, the
/// rewrites, and the files and modules it creates.
pub(crate) struct MovePlan<'a> {
    pub(crate) s: &'a Store,
    pub(crate) ix: &'a Index,
    pub(crate) listed: Vec<usize>,
    pub(crate) helpers: Vec<usize>,
    pub(crate) to: String,
    pub(crate) test_to: String,
    pub(crate) mine: &'a str,
    pub(crate) dest: Option<Dest>,
    pub(crate) dep: Option<(String, String)>,
    pub(crate) deps: Vec<Dependent>,
    pub(crate) dep_lines: Vec<Option<String>>,
    pub(crate) already: Vec<String>,
    pub(crate) now_pub: Vec<usize>,
    pub(crate) plan: Plan<'a>,
    pub(crate) dplans: Vec<Plan<'a>>,
    pub(crate) moved_tests: Vec<usize>,
    pub(crate) newmod: Vec<String>,
    pub(crate) carries: Vec<Carry>,
    pub(crate) ffi: Vec<String>,
    pub(crate) created: Vec<(String, &'a str)>,
    pub(crate) notes: Vec<String>,
    pub(crate) moved: Vec<bool>,
    pub(crate) old_modules: BTreeSet<String>,
}

pub(crate) fn destination(s: &Store, p: &Args, top: &Path, members: &[String]) -> Result<Option<Dest>, Failure> {
    let Some(d) = &p.to_project else { return Ok(None) };
    // `../x` and `./x` resolve against this project, as `--project` does (D-126 point 3).
    let d = crate::project_paths::project_folder(top, Some(&s.root), d, members);
    if !members.iter().any(|m| *m == d) {
        return fail(format!("E6007 {d} is not a project under this Solace root; its projects: {}", members.join(", ")));
    }
    let dir = scope::path(top, &d);
    if crate::project_paths::same_project(&dir, &s.root) {
        Ok(None)
    } else if crate::language::is_rust(&dir) {
        fail(format!("E6013 {d} is a Rust project: `--to-project` is not available for Rust (D-185)"))
    } else if !dir.join("gleam.toml").exists() {
        fail(format!("E6013 {d} has no code held as symbols to extract into"))
    } else {
        let store = Store::at(&dir)?;
        let ix = store.index()?;
        Ok(Some(Dest { dir: d.to_string(), store, ix }))
    }
}

pub(crate) fn target_module(s: &Store, ix: &Index, p: &Args, dest: &Option<Dest>, mine: &str, members: &[String]) -> Result<String, Failure> {
    let mut to = p.to.trim().trim_end_matches(".gleam").to_string();
    for pre in ["src/", "test/"] {
        if let Some(r) = to.strip_prefix(pre) {
            to = r.to_string();
            break;
        }
    }
    let exists = |m: &str| ix.modules.iter().any(|r| r.path == m);
    if let Some(d) = dest {
        if !d.has(&to) && !d.store.lang().valid_module_path(&to) {
            return fail(format!("{to} is not a module name; use --to <module>, e.g. --to solace_task"));
        }
    } else if !exists(&to) {
        if let Some((prefix, module)) = to.rsplit_once('/')
            && let Some(d) = members.iter().find(|d| d.as_str() != mine && (d.as_str() == prefix || d.ends_with(&format!("/{prefix}"))))
        {
            return fail(format!("use --to {module} --to-project {d}"));
        }
        if !s.lang().valid_module_path(&to) {
            return fail(format!("{to} is not a module name; use --to <module>, e.g. --to solace_task"));
        }
    }
    Ok(to)
}

pub(crate) fn module_notes(
    s: &Store,
    dest: &Option<Dest>,
    ix: &Index,
    to: &str,
    top: &Path,
    mine: &str,
    members: &[String],
) -> Result<Vec<String>, Failure> {
    let mut notes = vec![];
    if dest.is_some() || ix.modules.iter().any(|r| r.path == to) {
        return Ok(notes);
    }
    for d in members.iter().filter(|d| d.as_str() != mine) {
        let dir = scope::path(top, d);
        if !["src", "test"].iter().any(|t| dir.join(t).join(format!("{to}.gleam")).exists()) {
            continue;
        }
        if s.connected(&dir) {
            return fail(format!("{to} is in {d}; add --to-project {d}"));
        }
        notes.push(format!("Created module {to}; a module of that name exists in the unrelated project {d}."));
    }
    Ok(notes)
}

pub(crate) struct SymbolMoves {
    pub(crate) moved: Vec<bool>,
    pub(crate) helpers: Vec<usize>,
    pub(crate) moved_tests: Vec<usize>,
    pub(crate) test_to: String,
    pub(crate) newmod: Vec<String>,
}

/// The symbols, helpers and tests that move, and the module each symbol lands in.
pub(crate) fn moves(ix: &Index, listed: &[usize], to: &str, target_is_test: bool) -> SymbolMoves {
    let n = ix.syms.len();
    let mut moved = vec![false; n];
    for &i in listed {
        moved[i] = true;
    }
    let mut helpers = vec![];
    bring_helpers(ix, &mut moved, &mut helpers);
    let mut moved_tests = vec![];
    if !target_is_test {
        for t in reach::test_symbols(ix) {
            let c = reach::covered(ix, t);
            if !moved[t] && !c.is_empty() && c.iter().all(|&u| moved[u]) {
                moved_tests.push(t);
            }
        }
        for &t in &moved_tests {
            moved[t] = true;
        }
        let mut test_helpers = vec![];
        bring_helpers(ix, &mut moved, &mut test_helpers);
        helpers.extend(test_helpers);
    }
    let test_to = if target_is_test { to.to_string() } else { format!("{to}_test") };
    let newmod = (0..n)
        .map(|i| match (moved[i], is_test(ix, i)) {
            (false, _) => ix.syms[i].module.clone(),
            (true, true) => test_to.clone(),
            (true, false) => to.to_string(),
        })
        .collect();
    SymbolMoves { moved, helpers, moved_tests, test_to, newmod }
}

pub(crate) fn check_module_clashes(
    ix: &Index,
    dest: &Option<Dest>,
    top: &Path,
    mine: &str,
    members: &[String],
    to: &str,
    test_to: &str,
    moved: &[bool],
    newmod: &[String],
) -> Result<(), Failure> {
    let Some(d) = dest else { return Ok(()) };
    let linked = package_graph(top, members, &[mine, d.dir.as_str()]);
    let has = |m: &str| d.has(m);
    for m in [to, test_to] {
        if !(0..ix.syms.len()).any(|i| moved[i] && newmod[i] == m) {
            continue;
        }
        if has(m) {
            // the destination's module is not new, but a module of that name the source keeps clashes with it
            if (0..ix.syms.len()).any(|i| !moved[i] && ix.syms[i].module == m) {
                return fail(format!(
                    concat!(
                        "E1004 module {} is in this project too and stays; ",
                        "Gleam module names are unique across packages that depend on each other: pick another module name",
                    ),
                    m
                ));
            }
            continue;
        }
        if ix.modules.iter().any(|r| r.path == m) {
            return fail(format!(
                concat!(
                    "E1004 module {} is in this project too; ",
                    "Gleam module names are unique across packages that depend on each other: pick another module name",
                ),
                m
            ));
        }
        for other in linked.iter().filter(|o| o.as_str() != mine && o.as_str() != d.dir) {
            let dir = scope::path(top, other);
            if ["src", "test"].iter().any(|t| dir.join(t).join(format!("{m}.gleam")).exists()) {
                return fail(format!(
                    concat!(
                        "E1004 module {} is in {}, a package connected to {} or {}; ",
                        "Gleam module names are unique across them: pick another module name",
                    ),
                    m, other, mine, d.dir
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn check_symbol_clashes(ix: &Index, dest: &Option<Dest>, moved: &[bool], newmod: &[String]) -> Result<(), Failure> {
    let n = ix.syms.len();
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
            return fail(format!(
                "E1004 {} already exists in {}; {} cannot move there; to fold {} into it, use merge {} {}",
                ix.syms[j].qname(),
                newmod[i],
                s.qname(),
                s.qname(),
                ix.syms[j].qname(),
                s.qname()
            ));
        }
        if let Some(d) = dest
            && let Some(t) =
                d.ix.syms.iter().find(|t| t.module == newmod[i] && t.name == s.name && (t.kind == Kind::Type) == (s.kind == Kind::Type))
        {
            return fail(format!(
                "E1004 {} already exists in {} ({}); {} cannot move there; to fold {} into it, use merge {} {} --to-project {}",
                t.qname(),
                newmod[i],
                d.dir,
                s.qname(),
                s.qname(),
                t.qname(),
                s.qname(),
                d.dir
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_opaque(ix: &Index, moved: &[bool]) -> Result<(), Failure> {
    for x in 0..ix.syms.len() {
        let s = &ix.syms[x];
        if s.kind != Kind::Type || !s.text.lines().any(|l| l.starts_with("pub opaque type ")) {
            continue;
        }
        let users: Vec<usize> = ix.used_by[x].iter().copied().filter(|&j| constructs(ix, j, x)).collect();
        let apart: Vec<String> = users.iter().filter(|&&j| moved[j] != moved[x]).map(|&j| ix.syms[j].qname()).collect();
        if !apart.is_empty() {
            let verb = if moved[x] { "extract them too" } else { "extract it too" };
            return fail(format!("E5006 {} is opaque; its constructors are used by {}; {verb}", s.name, apart.join(", ")));
        }
    }
    Ok(())
}

pub(crate) fn check_cycles(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    dest: &Option<Dest>,
    source_root: &Path,
    mine: &str,
) -> Result<bool, Failure> {
    let n = ix.syms.len();
    let exact: Vec<Vec<usize>> = (0..n).map(|i| crate::index::uses_split(ix, i).0).collect();
    let graph = |at: &dyn Fn(usize) -> String| {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for h in 0..n {
            for u in &exact[h] {
                if at(h) != at(*u) {
                    edges.entry(at(h)).or_default().insert(at(*u));
                }
            }
        }
        edges
    };
    let moved_uses_staying = |why: &str| -> Result<bool, Failure> {
        let mut stay: Vec<(usize, Vec<String>)> = vec![];
        for m in (0..n).filter(|&m| moved[m]) {
            for u in exact[m].clone().into_iter().filter(|&u| !moved[u]) {
                match stay.iter_mut().find(|(s, _)| *s == u) {
                    Some((_, by)) => by.push(ix.syms[m].name.clone()),
                    None => stay.push((u, vec![ix.syms[m].name.clone()])),
                }
            }
        }
        let (mut seen, mut stack) = (HashSet::new(), stay.iter().map(|(u, _)| *u).collect::<Vec<_>>());
        while let Some(u) = stack.pop() {
            if seen.insert(u) {
                stack.extend(exact[u].clone().into_iter().filter(|&v| !moved[v]));
            }
        }
        let list: Vec<String> = stay.iter().map(|(u, by)| format!("{} (by {})", ix.syms[*u].name, by.join(", "))).collect();
        fail(format!(
            "E5002 moved code uses {} symbol{} that stay{why}: {} (moving them would bring {} in total)",
            stay.len(),
            if stay.len() == 1 { "" } else { "s" },
            list.join(", "),
            seen.len()
        ))
    };
    let staying_uses_moved = |why: &str| -> Result<bool, Failure> {
        let mut back: Vec<String> = vec![];
        for h in (0..n).filter(|&h| !moved[h]) {
            for u in exact[h].clone().into_iter().filter(|&u| moved[u]) {
                back.push(format!("{} (by {})", ix.syms[u].name, ix.syms[h].name));
            }
        }
        fail(format!("E5002 staying code uses {} moved symbols and {why}: {}", back.len(), back.join(", ")))
    };
    let source_needs = (0..n).any(|h| !moved[h] && exact[h].iter().any(|&u| moved[u]));
    if let Some(d) = dest {
        let dest_needs = (0..n).any(|m| moved[m] && exact[m].iter().any(|&u| !moved[u]));
        let reaches = depends_on(&d.store.root, source_root);
        if source_needs && reaches {
            return staying_uses_moved(&format!("{} already depends on {mine}, so this would close a package loop", d.dir));
        }
        if dest_needs && !reaches {
            return moved_uses_staying(&format!(" in {mine}, and {} gains no dependency on it", d.dir));
        }
    } else if cyclic(&graph(&|i| newmod[i].clone())) && !cyclic(&graph(&|i| ix.syms[i].module.clone())) {
        let any_stay = (0..n).any(|m| moved[m] && exact[m].iter().any(|&u| !moved[u]));
        return if any_stay { moved_uses_staying("") } else { staying_uses_moved("closes an import loop") };
    }
    Ok(source_needs)
}

pub(crate) fn scan_dependents(
    s: &Store,
    ix: &Index,
    top: &Path,
    mine: &str,
    members: &[String],
    dest: &Option<Dest>,
    moved: &[bool],
) -> Result<(HashMap<(String, String), usize>, Vec<Dependent>), Failure> {
    let names = name_map(ix);
    let outside: HashMap<&(String, String), usize> =
        names.iter().filter(|&(_, &x)| moved[x] && is_pub_text(&ix.syms[x].text)).map(|(k, &x)| (k, x)).collect();
    let mut users: Vec<String> = vec![];
    let mut deps: Vec<Dependent> = vec![];
    for d in members.iter().filter(|d| d.as_str() != mine) {
        let dir = scope::path(top, d);
        let Some(me) = s.root.canonicalize().ok() else { break };
        if !crate::extract::deps::path_deps(&dir).contains(&me) {
            continue;
        }
        if let Some(dt) = dest
            && d.as_str() != dt.dir
            && covered(&dir)
        {
            let store = Store::at(&dir)?;
            let ix = store.index()?;
            deps.push(Dependent { dir: d.clone(), store, ix });
            continue;
        }
        let other = board::open(&dir).map_err(|e| Failure(format!("{d}: {}", e.0), e.1))?;
        let dix = other.index().map_err(|e| Failure(format!("{d}: {}", e.0), e.1))?;
        for sym in &dix.syms {
            let header = dix.modules.iter().find(|m| m.path == sym.module).map_or("", |m| m.header.as_str());
            let imps = dix.lang.imports(header);
            for r in dix.lang.references(&sym.text) {
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
                    let line = format!("{d}: {} uses {hit}", sym.qname());
                    if !users.contains(&line) {
                        users.push(line);
                    }
                }
            }
        }
    }
    if !users.is_empty() {
        return fail(format!(
            concat!(
                "E5003 a project that depends on this one uses moved symbols, ",
                "and extract rewrites another project only with --to-project when the task covers it: ",
                "change those uses first, add the project to the task, or move less\n{}",
            ),
            users.join("\n")
        ));
    }
    Ok((names, deps))
}

pub(crate) fn build_rewrite<'a>(
    ix: &'a Index,
    dest: &Option<Dest>,
    to: &str,
    test_to: &str,
    moved: &[bool],
    newmod: &[String],
    names: &HashMap<(String, String), usize>,
    carries: &[Carry],
) -> Result<(Plan<'a>, Vec<usize>, Vec<String>), Failure> {
    let n = ix.syms.len();
    let mut headers: HashMap<String, String> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
    let mut foreign = HashSet::new();
    if let Some(d) = dest {
        for m in d.ix.modules.iter().filter(|m| m.path == to || m.path == test_to) {
            headers.insert(m.path.clone(), m.header.clone());
        }
        for sym in d.ix.syms.iter().filter(|s| s.module == to || s.module == test_to) {
            foreign.insert((sym.module.clone(), sym.name.clone()));
            if sym.kind == Kind::Type {
                foreign.extend(d.ix.lang.constructors(&sym.text).into_iter().map(|c| (sym.module.clone(), c)));
            }
        }
    }
    let mut plan = Plan {
        ix,
        names: names.clone(),
        headers,
        foreign,
        moved: moved.to_vec(),
        newmod: newmod.to_vec(),
        newtext: ix.syms.iter().map(|s| s.text.clone()).collect(),
        needs: BTreeMap::new(),
        upto: n,
        edits: 0,
        edited: BTreeSet::new(),
        redirect: None,
        hits: 0,
        hit_in: BTreeSet::new(),
    };
    plan.rewrite()?;
    let mut now_pub = vec![];
    for x in 0..n {
        if !is_pub_text(&plan.newtext[x]) && ix.used_by[x].iter().any(|&j| newmod[j] != newmod[x]) {
            plan.newtext[x] = make_pub(&plan.newtext[x]);
            now_pub.push(x);
        }
    }
    // JavaScript FFI paths are relative to the compiled module: a move to
    // another folder, or a carried file's new place, rewrites them
    let mut ffi = vec![];
    for h in (0..n).filter(|&h| moved[h]) {
        let (was, now) = (module_dir(&ix.syms[h].module), module_dir(&newmod[h]));
        let mut edits = vec![];
        for (a, b, path) in ffi_spans(&plan.newtext[h]) {
            let target = ffi_norm(&was, &path);
            let goes = carries.iter().find(|c| c.old == target).map_or(target, |c| c.new.clone());
            let new = ffi_rel(&now, &goes);
            if new != path {
                ffi.push(format!("{} {path} -> {new}", ix.syms[h].qname()));
                edits.push((a, b, new));
            }
        }
        if !edits.is_empty() {
            plan.newtext[h] = spliced(&plan.newtext[h], edits);
        }
    }
    Ok((plan, now_pub, ffi))
}

pub(crate) fn dependent_plan<'a>(dp: &Dependent, jx: &'a Index, moved: &[bool], newmod: &[String]) -> Result<Plan<'a>, Failure> {
    let k = dp.ix.syms.len();
    let mut plan = Plan {
        ix: jx,
        names: name_map(jx),
        headers: dp.ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect(),
        foreign: HashSet::new(),
        moved: std::iter::repeat_n(false, k).chain(moved.iter().copied()).collect(),
        newmod: dp.ix.syms.iter().map(|s| s.module.clone()).chain(newmod.iter().cloned()).collect(),
        newtext: jx.syms.iter().map(|s| s.text.clone()).collect(),
        needs: BTreeMap::new(),
        upto: k,
        edits: 0,
        edited: BTreeSet::new(),
        redirect: None,
        hits: 0,
        hit_in: BTreeSet::new(),
    };
    plan.rewrite()?;
    Ok(plan)
}
