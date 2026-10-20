//! `extract <symbols…> --to <module> [--to-project <folder>]` (D-103): moves
//! symbols to another module of the project, or of another project of the
//! task, with their identity, brings along the private helpers only they use,
//! rewrites every reference and import, moves the tests that reach only moved
//! code, and refuses a module or package cycle (`E5002`). One transaction,
//! gated on `check_all`. JavaScript FFI paths follow the module; into another
//! project the `.mjs` files go along (`E5007` names what keeps one), and the
//! reply names the suites that reach moved code, since FFI paths aren't type-checked.
use super::*;
use std::collections::BTreeSet;

pub(crate) mod ahead;
mod merge;

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

/// The folder of module `m` in the compiled JavaScript, as parts: `a/b/c` ->
/// `[a, b]`. src/ and test/ compile into one tree, so only the module path counts.
fn module_dir(m: &str) -> Vec<String> {
    let mut parts: Vec<String> = m.split('/').map(String::from).collect();
    parts.pop();
    parts
}

/// `rel` taken from folder `dir`, as parts from the package root of the
/// compiled output; leading `..` parts leave the package.
fn ffi_norm(dir: &[String], rel: &str) -> Vec<String> {
    let mut out: Vec<String> = dir.to_vec();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." if out.last().is_some_and(|l| l != "..") => {
                out.pop();
            }
            p => out.push(p.into()),
        }
    }
    out
}

/// The relative path from folder `dir` to `target` (parts from the package
/// root), starting with `./` or `../` as JavaScript imports need.
fn ffi_rel(dir: &[String], target: &[String]) -> String {
    let common = dir.iter().zip(target).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".into(); dir.len() - common];
    out.extend(target[common..].iter().cloned());
    let s = out.join("/");
    if s.starts_with("../") { s } else { format!("./{s}") }
}

/// The relative paths of a symbol's `@external(javascript, "<path>", …)`
/// attributes, each with its byte span inside the quotes.
fn ffi_spans(text: &str) -> Vec<(usize, usize, String)> {
    let (mut out, mut at) = (vec![], 0);
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("@external(")
            && rest.trim_start().starts_with("javascript")
            && let Some(q) = rest.find('"')
            && let Some(len) = rest[q + 1..].find('"')
        {
            let path = &rest[q + 1..q + 1 + len];
            let start = at + (line.len() - t.len()) + "@external(".len() + q + 1;
            if path.starts_with("./") || path.starts_with("../") {
                out.push((start, start + len, path.to_string()));
            }
        }
        at += line.len();
    }
    out
}

/// A relative module specifier in a JavaScript file.
struct JsImport {
    /// Its byte span inside the quotes, and its line.
    start: usize,
    end: usize,
    line: usize,
    spec: String,
    /// The names it brings in by brace: `{ a, b as c }` -> `a`, `b`.
    names: Vec<String>,
}

/// The relative specifiers of a JavaScript file's `import`/`export … from "…"`,
/// `import "…"` and `import("…")`, in file order.
fn js_imports(text: &str) -> Vec<JsImport> {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    // comments and strings (but for a module specifier) are blanked, same length
    let masked = mask_js(text);
    let text = masked.as_str();
    // Gleam escapes a reserved name with a trailing `$`
    let plain = |s: &str| s.trim_end_matches('$').to_string();
    let mut out = vec![];
    for kw in ["from", "import"] {
        for (at, _) in text.match_indices(kw) {
            if text[..at].chars().last().is_some_and(word) || text[at + kw.len()..].chars().next().is_some_and(word) {
                continue;
            }
            if text[..at].trim_end().ends_with('.') {
                continue;
            }
            let after = at + kw.len();
            let mut r = text[after..].trim_start();
            if kw == "import"
                && let Some(x) = r.strip_prefix('(')
            {
                r = x.trim_start();
            }
            let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
            let Some(len) = r[1..].find(q) else { continue };
            let spec = &r[1..1 + len];
            if !(spec.starts_with("./") || spec.starts_with("../")) {
                continue;
            }
            let start = text.len() - r.len() + 1;
            let names = if kw == "from" {
                let head = &text[..at];
                let k = head.rfind("import").into_iter().chain(head.rfind("export")).max().unwrap_or(0);
                let stmt = &head[k..];
                let mut names: Vec<String> = match (stmt.find('{'), stmt.rfind('}')) {
                    (Some(a), Some(b)) if a < b => {
                        stmt[a + 1..b].split(',').filter_map(|x| x.split_whitespace().next()).map(plain).collect()
                    }
                    _ => vec![],
                };
                // `import * as m from "…"`: the names used as `m.name`
                if let Some((_, rest)) = stmt.split_once("* as ")
                    && let Some(alias) = rest.split_whitespace().next()
                {
                    for (i, _) in text.match_indices(&format!("{alias}.")) {
                        // `obj.m.name` and `obj?.m.name` are another object's property
                        if text[..i].chars().last().is_some_and(|c| word(c) || c == '.') {
                            continue;
                        }
                        let member: String = text[i + alias.len() + 1..].chars().take_while(|c| word(*c)).collect();
                        if !member.is_empty() {
                            names.push(plain(&member));
                        }
                    }
                }
                names
            } else {
                vec![]
            };
            out.push(JsImport { start, end: start + len, line: text[..start].matches('\n').count() + 1, spec: spec.into(), names });
        }
    }
    out.sort_by_key(|i| i.start);
    out
}

/// `text` with comments and string contents blanked (same byte length, lines
/// kept), except a string that is a module specifier (after `from`, `import`
/// or `import(`).
fn mask_js(text: &str) -> String {
    let blank = |out: &mut String, c: char| {
        out.extend(std::iter::repeat_n(if c == '\n' { '\n' } else { ' ' }, if c == '\n' { 1 } else { c.len_utf8() }))
    };
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let specifier = |head: &str| {
        let t = head.trim_end().trim_end_matches('(').trim_end();
        let keyword = |k: &str| t.strip_suffix(k).is_some_and(|b| !b.chars().last().is_some_and(word) && !b.trim_end().ends_with('.'));
        keyword("from") || keyword("import")
    };
    let mut out = String::with_capacity(text.len());
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let next = it.peek().map(|x| x.1);
        match c {
            '/' if next == Some('/') => {
                blank(&mut out, c);
                while let Some(&(_, d)) = it.peek() {
                    if d == '\n' {
                        break;
                    }
                    blank(&mut out, d);
                    it.next();
                }
            }
            '/' if next == Some('*') => {
                blank(&mut out, c);
                while let Some((_, d)) = it.next() {
                    blank(&mut out, d);
                    if d == '*' && it.peek().map(|x| x.1) == Some('/') {
                        blank(&mut out, '/');
                        it.next();
                        break;
                    }
                }
            }
            '"' | '\'' | '`' => {
                let keep = c != '`' && specifier(&text[..i]);
                out.push(c);
                while let Some((_, d)) = it.next() {
                    if d == '\\' {
                        let e = it.next().map(|x| x.1);
                        for ch in std::iter::once(d).chain(e) {
                            if keep { out.push(ch) } else { blank(&mut out, ch) }
                        }
                    } else if d == c {
                        out.push(c);
                        break;
                    } else if keep {
                        out.push(d);
                    } else {
                        blank(&mut out, d);
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The project's JavaScript files under src/ and test/, as `src/…`.
pub(crate) fn js_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(root.join(rel)) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = format!("{rel}/{}", e.file_name().to_string_lossy());
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(root, &name, out),
                Ok(t) if t.is_file() && name.ends_with(".mjs") => out.push(name),
                _ => {}
            }
        }
    }
    let mut out = vec![];
    for top in ["src", "test"] {
        walk(root, top, &mut out);
    }
    out
}

/// A file's folder in the compiled output: `src/a/x.mjs` -> `[a]`.
fn out_dir(file: &str) -> Vec<String> {
    let mut parts: Vec<String> = file.split('/').skip(1).map(String::from).collect();
    parts.pop();
    parts
}

/// Ends every file change, kept or put back, even after one fails; the first error is returned.
fn end_all<F: FnOnce(bool) -> Result<(), Failure>>(ends: Vec<F>, kept: bool) -> Result<(), Failure> {
    let mut first = None;
    for end in ends {
        if let Err(e) = end(kept)
            && first.is_none()
        {
            first = Some(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// The failure `e`, with the error of putting back what came before it, if any.
fn with_put_back(e: Failure, ended: Result<(), Failure>) -> Failure {
    match ended {
        Ok(()) => e,
        Err(end) => Failure(format!("{}\n\nPutting back also failed: {}", e.0, end.0), e.1),
    }
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
fn spliced(text: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut t = text.to_string();
    for (a, b, r) in edits {
        t.replace_range(a..b, &r);
    }
    t
}

/// A JavaScript file an extract into another project takes along (D-103, D-84).
struct Carry {
    /// Its place in the source and in the destination, `src/…` or `test/…`.
    from: String,
    to: String,
    /// Its place in the compiled output, from the package root, before and after.
    old: Vec<String>,
    new: Vec<String>,
    /// Its text with its relative imports rewritten, when one changed.
    text: Option<String>,
}

/// The JavaScript files the moved code uses, which go along to `dest`, beside
/// the module that now uses them: a relative path can't reach another package.
/// Refused, naming them, when staying code or another file still needs one, a
/// carried file imports what stays, or the destination has a file there.
fn ffi_carries(root: &Path, ix: &Index, moved: &[bool], newmod: &[String], dest: &Dest) -> Result<Vec<Carry>, Failure> {
    let place = |root: &Path, out: &[String]| {
        ["src", "test"].into_iter().map(|t| format!("{t}/{}", out.join("/"))).find(|p| root.join(p).is_file())
    };
    let mut carries: Vec<Carry> = vec![];
    for i in (0..ix.syms.len()).filter(|&i| moved[i]) {
        let s = &ix.syms[i];
        for (_, _, path) in ffi_spans(&s.text) {
            let old = ffi_norm(&module_dir(&s.module), &path);
            if carries.iter().any(|c| c.old == old) {
                continue;
            }
            if old.first().is_some_and(|p| p == "..") {
                return fail(format!("E5007 {} uses {path}, outside its package; extract can't take that file along", s.qname()));
            }
            let Some(from) = place(root, &old) else {
                return fail(format!("E5007 {} uses {path}, which is in neither src/ nor test/; fix the path first", s.qname()));
            };
            let mut new = module_dir(&newmod[i]);
            new.push(old.last().unwrap().clone());
            let to = format!("{}/{}", from.split('/').next().unwrap_or("src"), new.join("/"));
            if let Some(c) = carries.iter().find(|c| c.new == new) {
                return fail(format!("E5007 {from} and {} would both go to {to} in {}: rename one of them first", c.from, dest.dir));
            }
            if let Some(there) = place(&dest.store.root, &new) {
                return fail(format!(
                    "E5007 {}: {there} already exists, and {} needs its own {path} there: rename one of them first",
                    dest.dir,
                    s.qname()
                ));
            }
            carries.push(Carry { from, to, old, new, text: None });
        }
    }
    if carries.is_empty() {
        return Ok(carries);
    }
    let carried = |target: &[String]| carries.iter().find(|c| c.old == target);
    let staying: Vec<String> = (0..ix.syms.len())
        .filter(|&j| !moved[j])
        .flat_map(|j| ffi_spans(&ix.syms[j].text).into_iter().map(move |(_, _, p)| (j, p)))
        .filter(|(j, p)| carried(&ffi_norm(&module_dir(&ix.syms[*j].module), p)).is_some())
        .map(|(j, p)| format!("{} ({p})", ix.syms[j].qname()))
        .collect();
    if !staying.is_empty() {
        return fail(format!(
            concat!(
                "E5007 staying code uses a JavaScript file the moved code takes to {}: {}; ",
                "extract it too, or give the moved code a file of its own",
            ),
            dest.dir,
            staying.join(", ")
        ));
    }
    let mut bad: Vec<String> = vec![];
    for f in js_files(root).iter().filter(|f| !carries.iter().any(|c| c.from == **f)) {
        let Ok(text) = fs::read_to_string(root.join(f)) else { continue };
        for imp in js_imports(&text) {
            if carried(&ffi_norm(&out_dir(f), &imp.spec)).is_some() {
                bad.push(format!("{f}:{} imports {}", imp.line, imp.spec));
            }
        }
    }
    if !bad.is_empty() {
        return fail(format!(
            concat!(
                "E5007 a JavaScript file imports one the moved code takes to {}, ",
                "and a relative import can't reach another package: {}; change it first, or move less",
            ),
            dest.dir,
            bad.join(", ")
        ));
    }
    let mut texts = vec![];
    for c in &carries {
        let text = fs::read_to_string(root.join(&c.from))?;
        let (was, now) = (&c.old[..c.old.len() - 1], &c.new[..c.new.len() - 1]);
        let mut edits = vec![];
        for imp in js_imports(&text) {
            let target = ffi_norm(was, &imp.spec);
            // the prelude is in every package; a path that leaves the package keeps its target
            let goes = if target == ["gleam.mjs"] || target.first().is_some_and(|p| p == "..") {
                target
            } else if let Some(o) = carried(&target) {
                o.new.clone()
            } else {
                bad.push(format!("{}:{} imports {}", c.from, imp.line, imp.spec));
                continue;
            };
            let spec = ffi_rel(now, &goes);
            if spec != imp.spec {
                edits.push((imp.start, imp.end, spec));
            }
        }
        texts.push((!edits.is_empty()).then(|| spliced(&text, edits)));
    }
    if !bad.is_empty() {
        return fail(format!(
            concat!(
                "E5007 a JavaScript file the moved code takes to {} imports what stays, ",
                "and a relative import can't reach another package: {}; move that too, or change the import first",
            ),
            dest.dir,
            bad.join(", ")
        ));
    }
    for (c, t) in carries.iter_mut().zip(texts) {
        c.text = t;
    }
    Ok(carries)
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

/// Whether the project in `from` depends on the one in `to` by path, directly or transitively.
fn depends_on(from: &Path, to: &Path) -> bool {
    let Ok(to) = to.canonicalize() else { return false };
    let (mut seen, mut stack) = (HashSet::new(), vec![from.canonicalize().unwrap_or_else(|_| from.to_path_buf())]);
    while let Some(p) = stack.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        for d in path_deps(&p) {
            if d == to {
                return true;
            }
            stack.push(d);
        }
    }
    false
}

/// The task's `projects` whose gleam.toml depends on the project in `source`
/// by path, as canonical folders: an extract into another project rewrites them (D-103).
pub(crate) fn dependents(top: &Path, source: &Path, projects: &[String]) -> Vec<PathBuf> {
    let Ok(me) = source.canonicalize() else { return vec![] };
    projects.iter().filter_map(|d| scope::path(top, d).canonicalize().ok()).filter(|p| *p != me && path_deps(p).contains(&me)).collect()
}

/// Whether the command holds the store of the project in `root`: a dependent
/// the task covers, opened by `run_options` (D-103).
fn covered(root: &Path) -> bool {
    COMMAND_DB.with_borrow(|c| c.iter().any(|(r, _)| lockdown::same_project(r, root)))
}

/// The member folders linked to any of `dirs` through path dependencies,
/// either way and transitively, `dirs` among them.
fn package_graph(top: &Path, members: &[String], dirs: &[&str]) -> Vec<String> {
    let canon: Vec<Option<PathBuf>> = members.iter().map(|d| scope::path(top, d).canonicalize().ok()).collect();
    let deps: Vec<Vec<PathBuf>> = members.iter().map(|d| path_deps(&scope::path(top, d))).collect();
    let on = |a: usize, b: usize| canon[b].as_ref().is_some_and(|p| deps[a].contains(p));
    let mut seen: Vec<usize> = (0..members.len()).filter(|&i| dirs.contains(&members[i].as_str())).collect();
    let mut k = 0;
    while k < seen.len() {
        let a = seen[k];
        for b in 0..members.len() {
            if !seen.contains(&b) && (on(a, b) || on(b, a)) {
                seen.push(b);
            }
        }
        k += 1;
    }
    seen.into_iter().map(|i| members[i].clone()).collect()
}

/// A gleam.toml's `text` with `line` added at the end of `[dependencies]`;
/// the table is made when there is none.
fn add_dependency(text: &str, line: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines.iter().position(|l| l.trim() == "[dependencies]") else {
        return format!("{}\n\n[dependencies]\n{line}\n", text.trim_end());
    };
    let mut end = lines[at + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |k| at + 1 + k);
    while end > at + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut out: Vec<&str> = lines[..end].to_vec();
    out.push(line);
    out.extend(&lines[end..]);
    out.join("\n") + "\n"
}

/// The project `extract --to-project` moves symbols into (D-103).
struct Dest {
    /// Its folder, as the root names it.
    dir: String,
    store: Store,
    ix: Index,
}

impl Dest {
    fn has(&self, module: &str) -> bool {
        self.ix.modules.iter().any(|m| m.path == module)
    }
}

/// A project that depends on the source and that the task covers: its uses of
/// moved symbols are rewritten (D-103).
struct Dependent {
    /// Its folder, as the root names it.
    dir: String,
    store: Store,
    ix: Index,
}

/// `dep`'s symbols and modules, then the source's: one index in which the
/// rewrite resolves the dependent's references.
fn merged(dep: &Index, source: &Index) -> Index {
    let syms = dep.syms.iter().chain(&source.syms).cloned().collect();
    let modules = dep
        .modules
        .iter()
        .chain(&source.modules)
        .map(|m| ModuleRow {
            path: m.path.clone(),
            dir: m.dir.clone(),
            header: m.header.clone(),
            file: m.file.clone(),
            owns_dir: m.owns_dir,
        })
        .collect();
    Index::build(source.lang, syms, modules, false)
}

/// `body` in a transaction of each store, the first outermost; the last one
/// takes away the files it created when the body fails.
fn nested(stores: &[&Store], created: &[PathBuf], body: &mut dyn FnMut() -> Out) -> Out {
    match stores {
        [] => body(),
        [last] => last.transaction(created, || body()),
        [first, rest @ ..] => first.transaction(&[], || nested(rest, created, body)),
    }
}

fn name_map(ix: &Index) -> HashMap<(String, String), usize> {
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
struct Plan<'a> {
    ix: &'a Index,
    names: HashMap<(String, String), usize>,
    /// Each module's imports before the move, the destination's target modules too.
    headers: HashMap<String, String>,
    /// (module, name) the destination's target modules already define.
    foreign: HashSet<(String, String)>,
    moved: Vec<bool>,
    newmod: Vec<String>,
    newtext: Vec<String>,
    needs: BTreeMap<String, Vec<Need>>,
    /// The symbols the rewrite edits: those before it; a dependent's plan
    /// resolves into the source's symbols after them.
    upto: usize,
    edits: usize,
    edited: BTreeSet<String>,
    /// A merge (D-117): every reference to the first goes to the second, which
    /// stays where it is; the first is retired and its text not rewritten.
    redirect: Option<(usize, usize)>,
    /// The references a merge redirected, and the modules they are in.
    hits: usize,
    hit_in: BTreeSet<String>,
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
    fn rewrite(&mut self) -> Result<(), Failure> {
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

/// Writes to its store the imports each of `modules` needs after the move,
/// minus those the move made unused; returns each module's header after.
fn write_headers(plan: &Plan, modules: &[(String, String, &Store)]) -> Result<Vec<(String, String)>, Failure> {
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
        let ix = self.index()?;
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let members = workspace::members(&top).unwrap_or_default();
        let exists = |m: &str| ix.modules.iter().any(|r| r.path == m);

        // the destination project, with --to-project
        let dest = match &p.to_project {
            None => None,
            Some(d) => {
                // `../x` and `./x` resolve against this project, as `--project` does (D-126 point 3).
                let d = project_folder(&top, Some(&self.root), d, &members);
                if !members.iter().any(|m| *m == d) {
                    return fail(format!("E6007 {d} is not a project under this Solace root; its projects: {}", members.join(", ")));
                }
                let dir = scope::path(&top, &d);
                if lockdown::same_project(&dir, &self.root) {
                    None
                } else if crate::language::is_rust(&dir) {
                    return fail(format!(
                        "E6013 {d} is a Rust project (Cargo.toml): `extract` is Gleam only; Rust extract is deferred (D-158 point 6)"
                    ));
                } else if !dir.join("gleam.toml").exists() {
                    return fail(format!("E6013 {d} has no code held as symbols to extract into"));
                } else {
                    let store = Store::at(&dir)?;
                    let ix = store.index()?;
                    Some(Dest { dir: d.to_string(), store, ix })
                }
            }
        };
        let in_target = |m: &str, name: &str| match &dest {
            Some(d) => d.ix.syms.iter().any(|s| s.module == m && s.name == name),
            None => ix.syms.iter().any(|s| s.module == m && s.name == name),
        };

        // the target, in its forgiving forms
        let mut to = p.to.trim().trim_end_matches(".gleam").to_string();
        for pre in ["src/", "test/"] {
            if let Some(r) = to.strip_prefix(pre) {
                to = r.to_string();
                break;
            }
        }
        if let Some(d) = &dest {
            if !d.has(&to) && !d.store.lang().valid_module_path(&to) {
                return fail(format!("{to} is not a module name; use --to <module>, e.g. --to solace_task"));
            }
        } else if !exists(&to) {
            if let Some((prefix, module)) = to.rsplit_once('/')
                && let Some(d) = members.iter().find(|d| **d != mine && (d.as_str() == prefix || d.ends_with(&format!("/{prefix}"))))
            {
                return fail(format!("use --to {module} --to-project {d}"));
            }
            if !self.lang().valid_module_path(&to) {
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
                    if self.resolve_module(&ix, name).is_some() {
                        return fail(MODULE_HINT);
                    }
                    let last = name.rsplit('.').next().unwrap_or(name);
                    if in_target(&to, last) {
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
        if dest.is_none() && !exists(&to) {
            for d in members.iter().filter(|d| **d != mine) {
                let dir = scope::path(&top, d);
                if !["src", "test"].iter().any(|t| dir.join(t).join(format!("{to}.gleam")).exists()) {
                    continue;
                }
                if self.connected(&dir) {
                    return fail(format!("{to} is in {d}; add --to-project {d}"));
                }
                notes.push(format!("Created module {to}; a module of that name exists in the unrelated project {d}."));
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
            for t in reach::test_symbols(&ix) {
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
        let has = |m: &str| match &dest {
            Some(d) => d.has(m),
            None => exists(m),
        };

        // a module the destination gains is new to every package connected to it or to
        // the source: Gleam module names are unique within a dependency graph
        if let Some(d) = &dest {
            let linked = package_graph(&top, &members, &[mine.as_str(), d.dir.as_str()]);
            for m in [&to, &test_to] {
                if !(0..n).any(|i| moved[i] && newmod[i] == *m) {
                    continue;
                }
                if has(m) {
                    // the destination's module is not new, but a module of that name the source keeps clashes with it
                    if (0..n).any(|i| !moved[i] && ix.syms[i].module == *m) {
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
                if exists(m) {
                    return fail(format!(
                        concat!(
                            "E1004 module {} is in this project too; ",
                            "Gleam module names are unique across packages that depend on each other: pick another module name",
                        ),
                        m
                    ));
                }
                for other in linked.iter().filter(|o| **o != mine && **o != d.dir) {
                    let dir = scope::path(&top, other);
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
        }

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
            if let Some(d) = &dest
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

        // a module cycle, or with --to-project a package cycle: the source comes to
        // depend on the destination, which gains no dependency (D-103)
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
        let moved_uses_staying = |why: &str| -> Out {
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
            let list: Vec<String> = stay.iter().map(|(u, by)| format!("{} (by {})", ix.syms[*u].name, by.join(", "))).collect();
            fail(format!(
                "E5002 moved code uses {} symbol{} that stay{why}: {} (moving them would bring {} in total)",
                stay.len(),
                if stay.len() == 1 { "" } else { "s" },
                list.join(", "),
                seen.len()
            ))
        };
        let staying_uses_moved = |why: &str| -> Out {
            let mut back: Vec<String> = vec![];
            for h in (0..n).filter(|&h| !moved[h]) {
                for &u in ix.uses[h].iter().filter(|&&u| moved[u]) {
                    back.push(format!("{} (by {})", ix.syms[u].name, ix.syms[h].name));
                }
            }
            fail(format!("E5002 staying code uses {} moved symbols and {why}: {}", back.len(), back.join(", ")))
        };
        let source_needs = (0..n).any(|h| !moved[h] && ix.uses[h].iter().any(|&u| moved[u]));
        if let Some(d) = &dest {
            let dest_needs = (0..n).any(|m| moved[m] && ix.uses[m].iter().any(|&u| !moved[u]));
            let reaches = depends_on(&d.store.root, &self.root);
            if source_needs && reaches {
                return staying_uses_moved(&format!("{} already depends on {mine}, so this would close a package loop", d.dir));
            }
            if dest_needs && !reaches {
                return moved_uses_staying(&format!(" in {mine}, and {} gains no dependency on it", d.dir));
            }
        } else if cyclic(&graph(&|i| newmod[i].clone())) && !cyclic(&graph(&|i| ix.syms[i].module.clone())) {
            let any_stay = (0..n).any(|m| moved[m] && ix.uses[m].iter().any(|&u| !moved[u]));
            return if any_stay { moved_uses_staying("") } else { staying_uses_moved("closes an import loop") };
        }

        // projects that depend on this one and use a moved public symbol; with
        // --to-project, those the task covers are rewritten (D-103)
        let names = name_map(&ix);
        let outside: HashMap<&(String, String), usize> =
            names.iter().filter(|&(_, &x)| moved[x] && is_pub_text(&ix.syms[x].text)).map(|(k, &x)| (k, x)).collect();
        let mut users: Vec<String> = vec![];
        let mut deps: Vec<Dependent> = vec![];
        for d in members.iter().filter(|d| **d != mine) {
            let dir = scope::path(&top, d);
            let Some(me) = self.root.canonicalize().ok() else { break };
            if !path_deps(&dir).contains(&me) {
                continue;
            }
            if let Some(dt) = &dest
                && *d != dt.dir
                && covered(&dir)
            {
                let store = Store::at(&dir)?;
                let ix = store.index()?;
                deps.push(Dependent { dir: d.clone(), store, ix });
                continue;
            }
            let Ok(other) = board::open(&dir) else { continue };
            let Ok(dix) = other.index() else { continue };
            for s in &dix.syms {
                let header = dix.modules.iter().find(|m| m.path == s.module).map_or("", |m| m.header.as_str());
                let imps = dix.lang.imports(header);
                for r in dix.lang.references(&s.text) {
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
                        let line = format!("{d}: {} uses {hit}", s.qname());
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

        // JavaScript FFI files: with --to-project, those the moved code uses go along
        let carries = match &dest {
            Some(d) => ffi_carries(&self.root, &ix, &moved, &newmod, d)?,
            None => vec![],
        };

        // the rewrite
        let mut headers: HashMap<String, String> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
        let mut foreign = HashSet::new();
        if let Some(d) = &dest {
            for m in d.ix.modules.iter().filter(|m| m.path == to || m.path == test_to) {
                headers.insert(m.path.clone(), m.header.clone());
            }
            for s in d.ix.syms.iter().filter(|s| s.module == to || s.module == test_to) {
                foreign.insert((s.module.clone(), s.name.clone()));
                if s.kind == Kind::Type {
                    foreign.extend(d.ix.lang.constructors(&s.text).into_iter().map(|c| (s.module.clone(), c)));
                }
            }
        }
        let mut plan = Plan {
            ix: &ix,
            names: names.clone(),
            headers,
            foreign,
            moved: moved.clone(),
            newmod: newmod.clone(),
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
        let mut now_pub: Vec<usize> = vec![];
        for x in 0..n {
            if !is_pub_text(&plan.newtext[x]) && ix.used_by[x].iter().any(|&j| newmod[j] != newmod[x]) {
                plan.newtext[x] = make_pub(&plan.newtext[x]);
                now_pub.push(x);
            }
        }
        // JavaScript FFI paths are relative to the compiled module: a move to
        // another folder, or a carried file's new place, rewrites them
        let mut ffi: Vec<String> = vec![];
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

        // the dependents' references, resolved across their store and the source's
        let joint: Vec<Index> = deps.iter().map(|dp| merged(&dp.ix, &ix)).collect();
        let mut dplans: Vec<Plan> = vec![];
        for (dp, jx) in deps.iter().zip(&joint) {
            let k = dp.ix.syms.len();
            let mut dplan = Plan {
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
            dplan.rewrite()?;
            dplans.push(dplan);
        }

        // modules to create
        let mut created: Vec<(String, &str)> = vec![];
        if !has(&to) {
            created.push((to.clone(), if target_is_test { "test" } else { "src" }));
        }
        if !moved_tests.is_empty() && !target_is_test && !has(&test_to) && !created.iter().any(|c| c.0 == test_to) {
            created.push((test_to.clone(), "test"));
        }
        let tgt: &Store = dest.as_ref().map_or(self, |d| &d.store);
        let created_files: Vec<PathBuf> = created.iter().map(|(m, dir)| tgt.root.join(dir).join(format!("{m}.gleam"))).collect();
        if created_files.iter().any(|f| f.exists()) {
            return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
        }

        // the dependency the source gains on the destination
        let dep = match &dest {
            Some(d) if source_needs && !path_deps(&self.root).iter().any(|p| d.store.root.canonicalize().ok().as_ref() == Some(p)) => {
                let name = workspace::name(&d.store.root);
                Some((name.clone(), format!("{name} = {{ path = \"{}\" }}", relative(&mine, &d.dir))))
            }
            _ => None,
        };
        // and the dependency each dependent gains, when it now imports the destination
        let dep_lines: Vec<Option<String>> = deps
            .iter()
            .zip(&dplans)
            .map(|(dp, p)| {
                let d = dest.as_ref()?;
                let imports = p.needs.values().flatten().any(|nd| nd.module == to || nd.module == test_to);
                let has = d.store.root.canonicalize().ok().is_some_and(|c| path_deps(&dp.store.root).contains(&c));
                (imports && !has).then(|| format!("{} = {{ path = \"{}\" }}", workspace::name(&d.store.root), relative(&dp.dir, &d.dir)))
            })
            .collect();

        // what the suites name now, to list those that stop resolving
        let suites_before: Vec<(String, String)> =
            reach::entries(self)?.into_iter().filter(|(_, e)| self.resolve(&ix, e).is_ok()).collect();
        let before = self.broken()?;
        let dest_broken = match &dest {
            Some(d) => d.store.broken()?,
            None => HashMap::new(),
        };
        let deps_broken = deps.iter().map(|dp| dp.store.broken()).collect::<Result<Vec<_>, _>>()?;

        let old_modules: BTreeSet<String> = (0..n).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
        let body = || {
            for (path, dir) in &created {
                tgt.db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, dir, ""])?;
            }
            for i in 0..n {
                let s = &ix.syms[i];
                if moved[i]
                    && let Some(d) = &dest
                {
                    // the same identity in the destination; the source keeps the retired row and its history
                    let (kind, rev): (String, String) =
                        self.db.query_row("SELECT kind, revision FROM symbols WHERE id = ?1", [&s.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                    let rev = if plan.newtext[i] != s.text { revision(&plan.newtext[i]) } else { rev };
                    let ord: i64 =
                        tgt.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&newmod[i]], |r| r.get(0))?;
                    tgt.db.execute(
                        "INSERT OR REPLACE INTO symbols(id,module,name,kind,text,ord,revision,live) VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                        params![s.id, newmod[i], s.name, kind, plan.newtext[i], ord, rev],
                    )?;
                    tgt.history(&s.id, "move", &format!("{mine}: {}", s.module))?;
                    self.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [&s.id])?;
                    self.history(&s.id, "move", &format!("to {}: {}", d.dir, newmod[i]))?;
                    continue;
                }
                if moved[i] {
                    let ord: i64 =
                        self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&newmod[i]], |r| r.get(0))?;
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
            let mut headers: Vec<(String, String, &Store)> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), self)).collect();
            headers.extend(created.iter().map(|(m, _)| (m.clone(), String::new(), tgt)));
            if let Some(d) = &dest {
                headers.extend(
                    d.ix.modules.iter().filter(|m| m.path == to || m.path == test_to).map(|m| (m.path.clone(), m.header.clone(), tgt)),
                );
            }
            let finals = write_headers(&plan, &headers)?;
            // the dependents: their rewritten symbols and imports
            for (dp, p) in deps.iter().zip(&dplans) {
                for (i, s) in dp.ix.syms.iter().enumerate() {
                    if p.newtext[i] != s.text {
                        dp.store.db.execute(
                            "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                            params![s.id, p.newtext[i], revision(&p.newtext[i])],
                        )?;
                        dp.store.history(&s.id, "edit", &s.name)?;
                    }
                }
                let modules: Vec<(String, String, &Store)> =
                    dp.ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), &dp.store)).collect();
                write_headers(p, &modules)?;
            }
            // a source module the move emptied goes, unless another module imports it
            let mut retired = vec![];
            for m in &old_modules {
                let empty = !(0..n).any(|i| newmod[i] == *m);
                let imported = finals.iter().any(|(o, h)| o != m && ix.lang.imports(h).iter().any(|i| i.module == *m));
                if empty && !imported {
                    self.db.execute("DELETE FROM modules WHERE path = ?1", [m])?;
                    retired.push(m.clone());
                }
            }
            // the JavaScript files the moved code takes along; one that fails puts back those before it
            let mut carried = vec![];
            for c in &carries {
                match lockdown::carry_file(&self.root, &c.from, &tgt.root, &c.to, c.text.clone()) {
                    Ok(end) => carried.push(end),
                    Err(e) => {
                        return Err(with_put_back(e, end_all(carried, false)));
                    }
                }
            }
            // the dependencies the source and the dependents gain; one that fails puts back those before it
            let mut adds: Vec<(&Path, &str)> = vec![];
            if let Some((_, line)) = &dep {
                adds.push((self.root.as_path(), line.as_str()));
            }
            for (dp, line) in deps.iter().zip(&dep_lines) {
                if let Some(line) = line {
                    adds.push((dp.store.root.as_path(), line.as_str()));
                }
            }
            let mut tomls = vec![];
            for (root, line) in adds {
                let changed = fs::read_to_string(root.join("gleam.toml"))
                    .map_err(Failure::from)
                    .and_then(|text| lockdown::change_file(root, "gleam.toml", &add_dependency(&text, line)));
                match changed {
                    Ok(end) => tomls.push(end),
                    Err(e) => {
                        let (a, b) = (end_all(tomls, false), end_all(carried, false));
                        return Err(with_put_back(e, a.and(b)));
                    }
                }
            }
            let checked = (|| -> Out {
                let mut bad: Vec<String> = vec![];
                // the destination's files first: Gleam reads a path dependency from disk
                if let Some(d) = &dest {
                    let diags = tgt.check_all(&tgt.index()?)?;
                    bad.extend(
                        diags
                            .iter()
                            .filter(|g| g.error && g.symbol.as_ref().is_none_or(|s| !dest_broken.contains_key(s)))
                            .map(|g| format!("{}: {}", d.dir, g.text)),
                    );
                }
                let ix2 = self.index()?;
                let diags = self.check_all(&ix2)?;
                bad.extend(
                    diags.iter().filter(|g| g.error && g.symbol.as_ref().is_none_or(|s| !before.contains_key(s))).map(|g| g.text.clone()),
                );
                // the dependents last: they read both from disk
                for (dp, broken) in deps.iter().zip(&deps_broken) {
                    let diags = dp.store.check_all(&dp.store.index()?)?;
                    bad.extend(
                        diags
                            .iter()
                            .filter(|g| g.error && g.symbol.as_ref().is_none_or(|s| !broken.contains_key(s)))
                            .map(|g| format!("{}: {}", dp.dir, g.text)),
                    );
                }
                if !bad.is_empty() {
                    return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", bad.join("\n\n")));
                }
                let gone: Vec<String> = suites_before
                    .iter()
                    .filter(|(_, e)| self.resolve(&ix2, e).is_err())
                    .map(|(s, e)| format!("suite {s} entry {e}"))
                    .collect();
                Ok(format!("{}\u{1}{}", gone.join("\u{2}"), retired.join("\u{2}")))
            })();
            // every end runs, so a failing one never leaves the rest undecided
            let (a, b) = (end_all(tomls, checked.is_ok()), end_all(carried, checked.is_ok()));
            a?;
            b?;
            checked
        };
        // a transaction in each store, the destination's innermost: a failure
        // anywhere rolls them all back
        let mut stores: Vec<&Store> = vec![self];
        stores.extend(deps.iter().map(|dp| &dp.store));
        stores.extend(dest.as_ref().map(|d| &d.store));
        let mut body = body;
        let suites_after = nested(&stores, &created_files, &mut body)?;
        let (gone, retired) = suites_after.split_once('\u{1}').unwrap_or(("", ""));

        // the reply: names and counts
        let q = |i: usize| ix.syms[i].qname();
        let mut out = format!("Moved {}", listed.iter().map(|&i| q(i)).collect::<Vec<_>>().join(", "));
        if !helpers.is_empty() {
            let names: Vec<&str> = helpers.iter().map(|&i| ix.syms[i].name.as_str()).collect();
            out += &format!(" (+{} helper{}: {})", helpers.len(), if helpers.len() == 1 { "" } else { "s" }, names.join(", "));
        }
        out += &format!(" to {to}{}\n", dest.as_ref().map_or(String::new(), |d| format!(" in {}", d.dir)));
        let mut gained: Vec<String> = dep.iter().map(|_| workspace::name(&self.root)).collect();
        gained.extend(deps.iter().zip(&dep_lines).filter(|(_, l)| l.is_some()).map(|(dp, _)| workspace::name(&dp.store.root)));
        if let Some(d) = dest.as_ref().filter(|_| !gained.is_empty()) {
            out += &format!("Dependency added: {} -> {}\n", gained.join(", "), workspace::name(&d.store.root));
        }
        if !already.is_empty() {
            out += &format!("Already in {to}: {}\n", already.join(", "));
        }
        if !now_pub.is_empty() {
            out +=
                &format!("Now pub: {} (new revision)\n", now_pub.iter().map(|&i| ix.syms[i].name.as_str()).collect::<Vec<_>>().join(", "));
        }
        if plan.edits > 0 {
            out += &format!(
                "Rewrote {} reference{} in {} module{}\n",
                plan.edits,
                if plan.edits == 1 { "" } else { "s" },
                plan.edited.len(),
                if plan.edited.len() == 1 { "" } else { "s" }
            );
        }
        for (dp, p) in deps.iter().zip(&dplans).filter(|(_, p)| p.edits > 0) {
            let plural = |k: usize| if k == 1 { "" } else { "s" };
            out += &format!(
                "Rewrote {} reference{} in {} module{} in {}\n",
                p.edits,
                plural(p.edits),
                p.edited.len(),
                plural(p.edited.len()),
                dp.dir
            );
        }
        if !moved_tests.is_empty() {
            out += &format!(
                "Tests moved: {}\n",
                moved_tests.iter().map(|&t| format!("{} -> {}", q(t), newmod[t])).collect::<Vec<_>>().join(", ")
            );
        }
        for c in &carries {
            let imports = if c.text.is_some() { " (its imports rewritten)" } else { "" };
            out += &format!("Moved file {} to {}: {}{imports}\n", c.from, dest.as_ref().map_or("", |d| d.dir.as_str()), c.to);
        }
        if !ffi.is_empty() {
            out += &format!("Rewrote FFI paths: {}\n", ffi.join(", "));
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
        // the declared suites that reach moved code (D-44): JavaScript FFI paths
        // aren't type-checked, so a suite is what catches a wrong one
        let ids: HashSet<&str> = (0..n).filter(|&i| moved[i]).map(|i| ix.syms[i].id.as_str()).collect();
        let at = |x: &Index| -> Vec<usize> { (0..x.syms.len()).filter(|&i| ids.contains(x.syms[i].id.as_str())).collect() };
        let mut suites = vec![];
        match &dest {
            None => {
                let x = self.index()?;
                suites.extend(reach::reached(&reach::suites(self, &x)?, &at(&x)));
            }
            Some(d) => {
                let x = d.store.index()?;
                let hits = at(&x);
                for (name, r) in reach::suites(&d.store, &x)? {
                    if !hits.is_empty() && r.as_ref().is_none_or(|r| hits.iter().any(|j| r.contains(j))) {
                        suites.push(format!("{name} in {0} (`run {name} --project {0}`)", d.dir));
                    }
                }
                // and this project's suites that reach code now calling across the packages
                let callers: HashSet<&str> =
                    (0..n).filter(|&h| !moved[h] && ix.uses[h].iter().any(|&u| moved[u])).map(|h| ix.syms[h].id.as_str()).collect();
                let sx = self.index()?;
                let there: Vec<usize> = (0..sx.syms.len()).filter(|&i| callers.contains(sx.syms[i].id.as_str())).collect();
                if !there.is_empty() {
                    suites.extend(reach::reached(&reach::suites(self, &sx)?, &there));
                }
                // and each covered dependent's suites that reach the code rewritten there
                for (dp, p) in deps.iter().zip(&dplans) {
                    let rewritten: HashSet<&str> =
                        dp.ix.syms.iter().enumerate().filter(|(i, s)| p.newtext[*i] != s.text).map(|(_, s)| s.id.as_str()).collect();
                    if rewritten.is_empty() {
                        continue;
                    }
                    let x = dp.store.index()?;
                    let there: Vec<usize> = (0..x.syms.len()).filter(|&i| rewritten.contains(x.syms[i].id.as_str())).collect();
                    for (name, r) in reach::suites(&dp.store, &x)? {
                        if !there.is_empty() && r.as_ref().is_none_or(|r| there.iter().any(|j| r.contains(j))) {
                            suites.push(format!("{name} in {0} (`run {name} --project {0}`)", dp.dir));
                        }
                    }
                }
            }
        }
        if !suites.is_empty() {
            out += &format!("Also reached: {}\n", suites.join(", "));
        } else if !ffi.is_empty() || !carries.is_empty() {
            out += concat!(
                "JavaScript FFI paths are not type-checked, and no declared suite reaches the moved code: ",
                "run the JavaScript tests, or declare them in solace.toml with `checks` (D-44)\n",
            );
        }
        // JavaScript that imports moved code from its old compiled module
        let mut moved_names: HashSet<String> = HashSet::new();
        for i in (0..n).filter(|&i| moved[i]) {
            moved_names.insert(ix.syms[i].name.clone());
            if ix.syms[i].kind == Kind::Type {
                moved_names.extend(ix.lang.constructors(&ix.syms[i].text));
            }
        }
        let olds: Vec<Vec<String>> = old_modules
            .iter()
            .map(|m| {
                let mut p = module_dir(m);
                p.push(format!("{}.mjs", tail(m)));
                p
            })
            .collect();
        let mut js_hits = vec![];
        for f in js_files(&self.root) {
            let Ok(text) = fs::read_to_string(self.root.join(&f)) else { continue };
            for imp in js_imports(&text) {
                if olds.contains(&ffi_norm(&out_dir(&f), &imp.spec)) && imp.names.iter().any(|x| moved_names.contains(x)) {
                    js_hits.push(format!("{f}:{}", imp.line));
                }
            }
        }
        if !js_hits.is_empty() {
            let (s, it) = if js_hits.len() == 1 { ("", "it") } else { ("s", "them") };
            out += &format!(
                "Not rewritten: JavaScript import{s} of moved code from its old module ({}): change {it} with `write`\n",
                js_hits.join(", ")
            );
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
            out += &format!(
                "Not rewritten: {} place{} in {} file{} ({}{more})\n",
                hits.len(),
                if hits.len() == 1 { "" } else { "s" },
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                shown.join(", ")
            );
        }
        Ok(lockdown::trim_output(&out, budget))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    const BOARD: &str = concat!(
        "pub type Task {\n  Task(id: Int, lane: Int)\n}\n\n",
        "pub fn new_task(id: Int) -> Task {\n  Task(id, lane_of(id))\n}\n\n",
        "fn lane_of(id: Int) -> Int {\n  id % 3\n}\n\n",
        "pub fn bump(t: Task) -> Task {\n  let _note = \"a Task here\" // Task\n  Task(..t, lane: t.lane + 1)\n}\n\n",
        "pub fn lane(t: Task) -> Int {\n  case t {\n    Task(lane: l, ..) -> l\n  }\n}\n",
    );

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
        assert!(
            text(&store, "board.new_task").contains("-> task.Task {\n  task.Task(id, lane_of(id))"),
            "{}",
            text(&store, "board.new_task")
        );
        let bump = text(&store, "board.bump");
        assert!(
            bump.contains("(t: task.Task) -> task.Task")
                && bump.contains("\"a Task here\" // Task")
                && bump.contains("task.Task(..t, lane: t.lane + 1)"),
            "{bump}"
        );
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
        assert!(
            header(&store, "app").contains("import task.{type Task, Task}") && !header(&store, "app").contains("board"),
            "{}",
            header(&store, "app")
        );
        assert!(
            header(&store, "other").contains("import task.{Task}") && !header(&store, "other").contains("board"),
            "{}",
            header(&store, "other")
        );
    }

    /// Moved code names staying code through its module, and `target.x` is bare;
    /// a private symbol that moved code uses becomes pub (new revision).
    #[test]
    fn moved_code_refers_back_and_target_references_become_bare() {
        let board = concat!(
            "pub type Task {\n  Task(id: Int)\n}\n\n",
            "fn lane_order(t: Task) -> Int {\n  t.id\n}\n\n",
            "pub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\n",
            "pub fn first(t: Task) -> Int {\n  lane_order(t)\n}\n\n",
            "pub fn twice(n: Int) -> Int {\n  util.double(n)\n}\n",
        );
        let store = project(&[
            ("src/board.gleam", &format!("import util\n\n{board}")),
            ("src/util.gleam", "pub fn double(n: Int) -> Int {\n  n * 2\n}\n"),
        ]);
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
        let store =
            project(&[("src/board.gleam", "pub fn show() -> Int {\n  1\n}\n"), ("src/task.gleam", "pub fn show() -> Int {\n  2\n}\n")]);
        let err = extract(&store, &["board.show", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E1004 task.show already exists in task"), "{err}");
        assert!(err.ends_with("; to fold board.show into it, use merge task.show board.show"), "{err}");
    }

    /// Moving code that staying code calls, and that calls staying code, would
    /// make a module loop: refused with every boundary use, then the retry works.
    #[test]
    fn a_module_cycle_is_refused_with_every_boundary_use() {
        let board = concat!(
            "pub type Task {\n  Task(id: Int)\n}\n\n",
            "pub fn lane_order(t: Task) -> Int {\n  t.id\n}\n\n",
            "pub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\n",
            "pub fn top(a: Task, b: Task) -> Bool {\n  sort(a, b)\n}\n",
        );
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
        let board = concat!(
            "pub opaque type Box {\n  Box(n: Int)\n}\n\n",
            "pub fn new_box(n: Int) -> Box {\n  Box(n)\n}\n\n",
            "pub fn open(b: Box) -> Box {\n  b\n}\n",
        );
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
        let tests = concat!(
            "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n\n",
            "pub fn mixed_test() {\n  let t = board.Task(1, 2)\n  assert board.keep() == t.id\n}\n",
        );
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
        let store = project(&[
            ("src/board.gleam", "pub fn one() -> Int {\n  1\n}\n"),
            ("src/other.gleam", "import board\n\npub fn two() -> Int {\n  board.one() + 1\n}\n"),
        ]);
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
        assert!(extract(&store, &["board.lane", "--to", "task", "--to-project", "../x"]).unwrap_err().0.starts_with("E6007"));
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
        assert_eq!(err, "solace_task is in components; add --to-project components");
        let err = extract(&store, &["board.lane", "--to", "components/solace_task"]).unwrap_err().0;
        assert_eq!(err, "use --to solace_task --to-project components");
        let err = extract(&store, &["board.Task", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E5003") && err.contains("front: front.view uses board.Task"), "{err}");
    }

    /// D-103: a dependent the command holds (one the task covers) is rewritten
    /// and gains the destination dependency; one it doesn't hold is refused.
    #[test]
    fn a_covered_dependent_is_rewritten() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import board\n\npub fn view(t: board.Task) -> Int {\n  board.lane(t)\n}\n"),
            ("web/solace.toml", "[commands]\nboard = { run = [\"true\"], checks = [\"board.bump\"] }\n"),
            ("front/solace.toml", "[commands]\nui = { run = [\"true\"], checks = [\"front.view\"] }\n"),
        ]);
        for d in ["web", "components", "front"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let args = ["board.Task", "board.new_task", "board.lane", "--to", "solace_task", "--to-project", "components"];
        assert!(extract(&store, &args).unwrap_err().0.starts_with("E5003"));
        let front = Store::at(&root.join("front")).unwrap();
        COMMAND_DB.with_borrow_mut(|c| c.push((front.root.clone(), front.db.clone())));
        let out = extract(&store, &args);
        COMMAND_DB.set(vec![]);
        let out = out.unwrap();
        assert!(out.contains("Dependency added: web, front -> components\n"), "{out}");
        assert!(out.contains("Rewrote 2 references in 1 module in front\n"), "{out}");
        // the suites that reach code now calling across packages, here and in the dependent
        assert!(out.contains("board (`run board`)") && out.contains("ui in front (`run ui --project front`)"), "{out}");
        assert_eq!(text(&front, "front.view"), "pub fn view(t: solace_task.Task) -> Int {\n  solace_task.lane(t)\n}");
        assert_eq!(header(&front, "front"), "import solace_task");
        assert_eq!(
            fs::read_to_string(root.join("front/gleam.toml")).unwrap(),
            concat!(
                "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "web = { path = \"../web\" }\ncomponents = { path = \"../components\" }\n",
            )
        );
    }

    /// Two projects under a root, `web` and `components`, each with its store.
    fn two_projects(components_toml: &str) -> Scratch {
        let tests = "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n";
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\n\n[dev-dependencies]\n"),
            ("web/src/board.gleam", BOARD),
            ("web/test/board_test.gleam", tests),
            ("components/gleam.toml", components_toml),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        root
    }

    /// D-103: symbols move into another project with their identity; the source
    /// gains the dependency, the destination doesn't.
    #[test]
    fn a_move_into_another_project_adds_the_dependency_to_the_source() {
        let components = "name = \"components\"\nversion = \"1.0.0\"\n";
        let root = two_projects(components);
        let store = Store::at(&root.join("web")).unwrap();
        let ix = store.index().unwrap();
        let id = ix.syms[store.resolve(&ix, "board.Task").unwrap()].id.clone();
        let out = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert!(out.starts_with("Moved board.Task to solace_task in components\nDependency added: web -> components\n"), "{out}");
        assert!(out.contains("Tests moved: board_test.round_trip_test -> solace_task_test"), "{out}");
        assert_eq!(
            fs::read_to_string(root.join("web/gleam.toml")).unwrap(),
            "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n\n[dev-dependencies]\n"
        );
        assert_eq!(fs::read_to_string(root.join("components/gleam.toml")).unwrap(), components);
        let dest = Store::at(&root.join("components")).unwrap();
        let dix = dest.index().unwrap();
        assert_eq!(dix.syms[dest.resolve(&dix, "solace_task.Task").unwrap()].id, id);
        assert!(dest.resolve(&dix, "solace_task_test.round_trip_test").is_ok());
        assert!(store.resolve(&store.index().unwrap(), "board.Task").is_err());
        assert!(text(&store, "board.new_task").contains("-> solace_task.Task {"), "{}", text(&store, "board.new_task"));
        assert!(fs::read_to_string(root.join("components/src/solace_task.gleam")).unwrap().contains("pub type Task"));
        // run again: nothing to do
        let again = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert_eq!(again, "solace_task.Task already in solace_task; nothing changed.\n");
    }

    /// D-103: a destination that already depends on the source can't take code
    /// the source still uses: refused with every use, and nothing changes.
    #[test]
    fn a_package_cycle_is_refused_with_the_staying_uses() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n");
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E5002 staying code uses 3 moved symbols and components already depends on web"), "{err}");
        assert!(err.contains("Task (by new_task)") && err.contains("Task (by bump)") && err.contains("Task (by lane)"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.Task").is_ok());
        assert!(!fs::read_to_string(root.join("web/gleam.toml")).unwrap().contains("components"));
        // a module name the source has is refused too
        let err = extract(&store, &["board.lane", "--to", "board", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E1004 module board is in this project too"), "{err}");
    }

    /// A module the destination already has, whose name the source keeps too,
    /// is refused before anything is written (Gleam module names are unique
    /// across connected packages).
    #[test]
    fn a_destination_module_named_like_one_the_source_keeps_is_refused() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/board.gleam", "pub fn y() -> Int {\n  1\n}\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.lane", "--to", "board", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E1004 module board is in this project too and stays"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.lane").is_ok());
    }

    /// A destination that already depends on the source may take code that uses
    /// what stays, as long as nothing staying uses the moved code.
    #[test]
    fn a_destination_that_reaches_the_source_takes_code_that_uses_staying_symbols() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n");
        let store = Store::at(&root.join("web")).unwrap();
        let out = extract(&store, &["board.lane", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert!(out.starts_with("Moved board.lane to solace_task in components\n"), "{out}");
        assert!(!out.contains("Dependency added"), "{out}");
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(text(&dest, "solace_task.lane").contains("board.Task"), "{}", text(&dest, "solace_task.lane"));
        assert!(!fs::read_to_string(root.join("web/gleam.toml")).unwrap().contains("components"));
    }

    /// Emptying a source module across projects retires it, and the source
    /// gains the dependency on the destination.
    #[test]
    fn emptying_a_module_into_another_project_retires_it() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n");
        let store = Store::at(&root.join("web")).unwrap();
        let out = extract(
            &store,
            &["board.Task", "board.new_task", "board.bump", "board.lane", "--to", "solace_task", "--to-project", "components"],
        )
        .unwrap();
        assert!(out.contains("Retired module board"), "{out}");
        // the test goes too, so nothing left in web uses components
        assert!(!out.contains("Dependency added"), "{out}");
        assert_eq!(
            fs::read_to_string(root.join("web/gleam.toml")).unwrap(),
            "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\n\n[dev-dependencies]\n"
        );
        let ix = store.index().unwrap();
        assert!(ix.modules.iter().all(|m| m.path != "board"), "board should be gone");
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(dest.resolve(&dest.index().unwrap(), "solace_task.bump").is_ok());
    }

    #[test]
    fn helpers_for_paths() {
        assert_eq!(relative("solace-cli/board", "solace-cli/components"), "../components");
        assert_eq!(relative(".", "a/b"), "a/b");
        assert_eq!(add_dependency("name = \"a\"\n", "b = 1"), "name = \"a\"\n\n[dependencies]\nb = 1\n");
        assert_eq!(
            add_dependency("[dependencies]\nx = 1\n\n[dev-dependencies]\ny = 2\n", "b = 1"),
            "[dependencies]\nx = 1\nb = 1\n\n[dev-dependencies]\ny = 2\n"
        );
        assert_eq!(item_split("type Task"), ("Task".to_string(), "Task".to_string()));
        assert_eq!(item_split("Foo as Bar"), ("Foo".to_string(), "Bar".to_string()));
        assert_eq!(make_pub("// doc\nfn f() {\n  1\n}"), "// doc\npub fn f() {\n  1\n}");
    }

    /// FFI paths follow the compiled module: nested, back up, between siblings,
    /// and out of the package.
    #[test]
    fn ffi_paths_follow_the_module() {
        let moved = |from: &str, to: &str, path: &str| ffi_rel(&module_dir(to), &ffi_norm(&module_dir(from), path));
        assert_eq!(moved("board", "ui/task", "./x.mjs"), "../x.mjs");
        assert_eq!(moved("ui/task", "board", "../x.mjs"), "./x.mjs");
        assert_eq!(moved("ui/task", "board", "./x.mjs"), "./ui/x.mjs");
        assert_eq!(moved("ui/a/m", "ui/b/n", "./x.mjs"), "../a/x.mjs");
        assert_eq!(moved("a/m", "b/n", "./ffi/x.mjs"), "../a/ffi/x.mjs");
        assert_eq!(moved("a/m", "a/n", "./x.mjs"), "./x.mjs");
        assert_eq!(moved("board", "ui/task", "../gleam_stdlib/x.mjs"), "../../gleam_stdlib/x.mjs");
        let text = "@external(erlang, \"erlang\", \"abs\")\n@external(javascript, \"./x.mjs\", \"abs\")\npub fn abs(x: Int) -> Int";
        let spans = ffi_spans(text);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].0..spans[0].1], "./x.mjs");
        assert!(ffi_spans("@external(javascript, \"lib\", \"f\")\npub fn f() -> Int").is_empty());
    }

    /// A failure keeps its message and code; a failed put-back is added to it.
    #[test]
    fn a_failed_put_back_is_named() {
        let e = || Failure("E6002 x: no file there".into(), 1);
        assert_eq!(with_put_back(e(), Ok(())).0, "E6002 x: no file there");
        let both = with_put_back(e(), Err(Failure("disk full".into(), 1)));
        assert_eq!((both.0.as_str(), both.1), ("E6002 x: no file there\n\nPutting back also failed: disk full", 1));
    }

    /// Relative specifiers of every import form, with their lines and brace names.
    #[test]
    fn js_imports_are_found() {
        let js = concat!(
            "import { a, b as c } from \"./x.mjs\";\nimport \"./side.mjs\";\n",
            "const m = await import('../y.mjs');\n",
            "export {\n  d,\n} from \"../gleam_stdlib/gleam.mjs\";\nimport lib from \"lib\";\nconst from_here = 1;\n",
        );
        let found: Vec<(String, usize, Vec<String>)> =
            js_imports(js).into_iter().map(|i| (js[i.start..i.end].to_string(), i.line, i.names)).collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            found,
            vec![
                ("./x.mjs".into(), 1, s(&["a", "b"])),
                ("./side.mjs".into(), 2, vec![]),
                ("../y.mjs".into(), 3, vec![]),
                ("../gleam_stdlib/gleam.mjs".into(), 6, s(&["d"])),
            ]
        );
    }

    /// Comments, strings and property calls are not imports; a namespace import
    /// names what it uses as `m.name`, and a `$`-escaped name matches its plain one.
    #[test]
    fn js_imports_skip_noise_and_read_namespaces() {
        let js = concat!(
            "// import { a } from \"./c.mjs\";\n/* from \"./d.mjs\" */\n",
            "const s = \"import x from './e.mjs'\";\n",
            "const t = Array.from(\"./f.mjs\");\n",
            "import * as m from \"./board.mjs\";\n",
            "import { type$ } from \"./g.mjs\";\nm.abs(1);\nobj.m.name;\nobj?.m.other;\n",
        );
        let found: Vec<(String, Vec<String>)> = js_imports(js).into_iter().map(|i| (js[i.start..i.end].to_string(), i.names)).collect();
        assert_eq!(found, vec![("./board.mjs".to_string(), vec!["abs".to_string()]), ("./g.mjs".to_string(), vec!["type".to_string()])]);
    }

    const ABS: &str =
        "@external(erlang, \"erlang\", \"abs\")\n@external(javascript, \"./board_ffi.mjs\", \"abs\")\npub fn abs(x: Int) -> Int\n";

    /// In one project the file stays and a move to another folder rewrites the
    /// path; a JavaScript import of moved code is named, and so is the suite
    /// that reaches it, or the lack of one.
    #[test]
    fn a_move_to_another_folder_rewrites_ffi_paths() {
        let files = [
            ("src/board.gleam", ABS),
            ("src/board_ffi.mjs", "export function abs(x) { return Math.abs(x) }\n"),
            ("src/app.mjs", "import { abs } from \"./board.mjs\";\n"),
        ];
        let store = project(&files);
        let out = extract(&store, &["board.abs", "--to", "ui/task"]).unwrap();
        assert!(out.contains("Rewrote FFI paths: board.abs ./board_ffi.mjs -> ../board_ffi.mjs\n"), "{out}");
        assert!(out.contains("no declared suite reaches the moved code"), "{out}");
        assert!(out.contains("Not rewritten: JavaScript import of moved code from its old module (src/app.mjs:1)"), "{out}");
        let abs = text(&store, "ui/task.abs");
        assert!(
            abs.contains("@external(javascript, \"../board_ffi.mjs\", \"abs\")") && abs.contains("@external(erlang, \"erlang\", \"abs\")"),
            "{abs}"
        );
        assert!(store.root.join("src/board_ffi.mjs").exists());
        // a suite that checks everything is named instead
        let toml = "[commands]\njs = { run = [\"true\"], checks = [\"*\"] }\n";
        let store = project(&[files[0], files[1], ("solace.toml", toml)]);
        let out = extract(&store, &["board.abs", "--to", "ui/task"]).unwrap();
        assert!(out.contains("Also reached: js (`run js`)\n") && !out.contains("no declared suite"), "{out}");
    }

    /// Into another project the file goes along, beside the module that uses
    /// it, with its imports rewritten; refused, naming them, when the
    /// destination has the file, staying code uses it, another file imports it,
    /// or it imports what stays.
    #[test]
    fn a_move_into_another_project_carries_ffi_files() {
        let neg =
            "\n@external(erlang, \"erlang\", \"-\")\n@external(javascript, \"./board_ffi.mjs\", \"neg\")\npub fn neg(x: Int) -> Int\n";
        let ffi = "import { Ok } from \"./gleam.mjs\";\nimport { h } from \"./util.mjs\";\nexport function abs(x) { return Math.abs(x) }\n";
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", &format!("{ABS}{neg}")),
            ("web/src/board_ffi.mjs", ffi),
            ("web/src/util.mjs", "export const h = 1;\n"),
            ("web/src/other.mjs", "import { abs } from \"./board_ffi.mjs\";\n"),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("components/src/ui/board_ffi.mjs", "taken\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let go = |names: &[&str]| {
            let mut args = names.to_vec();
            args.extend(["--to", "ui/task", "--to-project", "components"]);
            extract(&store, &args)
        };
        let err = go(&["board.abs"]).unwrap_err().0;
        assert!(err.starts_with("E5007 components: src/ui/board_ffi.mjs already exists"), "{err}");
        fs::remove_file(root.join("components/src/ui/board_ffi.mjs")).unwrap();
        let err = go(&["board.abs"]).unwrap_err().0;
        assert!(err.starts_with("E5007 staying code uses a JavaScript file") && err.contains("board.neg (./board_ffi.mjs)"), "{err}");
        let err = go(&["board.abs", "board.neg"]).unwrap_err().0;
        assert!(err.contains("src/other.mjs:1 imports ./board_ffi.mjs"), "{err}");
        fs::remove_file(root.join("web/src/other.mjs")).unwrap();
        let err = go(&["board.abs", "board.neg"]).unwrap_err().0;
        assert!(err.contains("imports what stays") && err.contains("src/board_ffi.mjs:2 imports ./util.mjs"), "{err}");
        assert!(root.join("web/src/board_ffi.mjs").exists() && !root.join("components/src/ui/board_ffi.mjs").exists());
        let ffi = ffi.replace("import { h } from \"./util.mjs\";\n", "");
        fs::write(root.join("web/src/board_ffi.mjs"), &ffi).unwrap();
        let out = go(&["board.abs", "board.neg"]).unwrap();
        assert!(out.contains("Moved file src/board_ffi.mjs to components: src/ui/board_ffi.mjs (its imports rewritten)\n"), "{out}");
        assert!(!out.contains("Rewrote FFI paths"), "{out}");
        assert!(!root.join("web/src/board_ffi.mjs").exists());
        let carried = fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap();
        assert_eq!(carried, ffi.replace("\"./gleam.mjs\"", "\"../gleam.mjs\""));
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(text(&dest, "ui/task.abs").contains("\"./board_ffi.mjs\""));
        // a carry that isn't kept puts the file back as it was
        let end = lockdown::carry_file(&dest.root, "src/ui/board_ffi.mjs", &store.root, "src/x.mjs", Some("changed".into())).unwrap();
        assert_eq!(fs::read_to_string(root.join("web/src/x.mjs")).unwrap(), "changed");
        end(false).unwrap();
        assert!(!root.join("web/src/x.mjs").exists());
        assert_eq!(fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap(), carried);
        // and takes away the folders it made for it, not those that were there
        let end = lockdown::carry_file(&dest.root, "src/ui/board_ffi.mjs", &store.root, "src/a/b/x.mjs", None).unwrap();
        assert!(root.join("web/src/a/b/x.mjs").exists());
        end(false).unwrap();
        assert!(!root.join("web/src/a").exists() && root.join("web/src").is_dir());
        assert_eq!(fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap(), carried);
    }
}
