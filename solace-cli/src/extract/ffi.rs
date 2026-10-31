//! JavaScript FFI paths: where an extracted module's `.mjs` files go, and which ones travel with it.

use super::*;
use crate::Index;
use crate::extract::plan::spliced;
use crate::extract::deps::Dest;
use crate::failure::Failure;
use crate::failure::fail;
#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;

/// The path from project folder `from` to `to` (both as the root names them).
pub(super) fn relative(from: &str, to: &str) -> String {
    let parts = |d: &str| d.split('/').filter(|s| !s.is_empty() && *s != ".").map(String::from).collect::<Vec<_>>();
    let (f, t) = (parts(from), parts(to));
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".into(); f.len() - common];
    out.extend(t[common..].iter().cloned());
    out.join("/")
}

/// The folder of module `m` in the compiled JavaScript, as parts: `a/b/c` ->
/// `[a, b]`. src/ and test/ compile into one tree, so only the module path counts.
pub(crate) fn module_dir(m: &str) -> Vec<String> {
    let mut parts: Vec<String> = m.split('/').map(String::from).collect();
    parts.pop();
    parts
}

/// `rel` taken from folder `dir`, as parts from the package root of the
/// compiled output; leading `..` parts leave the package.
pub(crate) fn ffi_norm(dir: &[String], rel: &str) -> Vec<String> {
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
pub(crate) fn ffi_rel(dir: &[String], target: &[String]) -> String {
    let common = dir.iter().zip(target).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".into(); dir.len() - common];
    out.extend(target[common..].iter().cloned());
    let s = out.join("/");
    if s.starts_with("../") { s } else { format!("./{s}") }
}

/// The relative paths of a symbol's `@external(javascript, "<path>", …)`
/// attributes, each with its byte span inside the quotes.
pub(crate) fn ffi_spans(text: &str) -> Vec<(usize, usize, String)> {
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
pub(crate) struct JsImport {
    /// Its byte span inside the quotes, and its line.
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) line: usize,
    pub(crate) spec: String,
    /// The names it brings in by brace: `{ a, b as c }` -> `a`, `b`.
    pub(crate) names: Vec<String>,
}

/// The relative specifiers of a JavaScript file's `import`/`export … from "…"`,
/// `import "…"` and `import("…")`, in file order.
pub(crate) fn js_imports(text: &str) -> Vec<JsImport> {
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
pub(crate) fn out_dir(file: &str) -> Vec<String> {
    let mut parts: Vec<String> = file.split('/').skip(1).map(String::from).collect();
    parts.pop();
    parts
}

/// A JavaScript file an extract into another project takes along (D-103, D-84).
pub(crate) struct Carry {
    /// Its place in the source and in the destination, `src/…` or `test/…`.
    pub(crate) from: String,
    pub(crate) to: String,
    /// Its place in the compiled output, from the package root, before and after.
    pub(crate) old: Vec<String>,
    pub(crate) new: Vec<String>,
    /// Its text with its relative imports rewritten, when one changed.
    pub(crate) text: Option<String>,
}

/// The JavaScript files the moved code uses, which go along to `dest`, beside
/// the module that now uses them: a relative path can't reach another package.
/// Refused, naming them, when staying code or another file still needs one, a
/// carried file imports what stays, or the destination has a file there.
pub(crate) fn ffi_carries(root: &Path, ix: &Index, moved: &[bool], newmod: &[String], dest: &Dest) -> Result<Vec<Carry>, Failure> {
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
