//! `view <file>` and `view <file>::<name>` for Rust and Python files (D-121).
//! A line scanner, not a parser: it skips strings and comments, counts
//! brackets, and reads an item's head from the line it starts, which fits the
//! rustfmt-formatted Rust and the Python here without a new dependency.

use super::*;

/// An item of a file. Lines are 1-based; `start..=end` includes the docs,
/// attributes and comments right above it, and `level` is how deeply it is
/// nested in `impl`, `trait`, `mod` or `class` blocks.
#[derive(Debug)]
struct Item {
    kind: &'static str,
    name: String,
    sig: String,
    doc: String,
    start: usize,
    end: usize,
    level: usize,
}

/// `view <file>` (an outline) and `view <file>::<name>` (one item's source),
/// or None when the operands are symbols for the store: in a project with
/// Gleam code, a `.rs` name that matches no watched file is one (`app.rs`).
pub fn view(root: &Path, v: &toml::Value, a: &[&str], budget: usize) -> Option<Out> {
    if a.first() != Some(&"view") {
        return None;
    }
    let (mut ops, mut flags) = (vec![], vec![]);
    let mut i = 1;
    while i < a.len() {
        match a[i] {
            "--project" => i += 1,
            f if f.starts_with("--") => flags.push(f),
            op => ops.push(op),
        }
        i += 1;
    }
    let source = |op: &str| op.ends_with(".rs") || op.ends_with(".py");
    if !ops.iter().any(|&op| op.contains("::") || source(op)) {
        return None;
    }
    let watched = match lockdown::watched_paths(root, v) {
        Ok(w) => w,
        Err(e) => return Some(Err(e)),
    };
    let gleam = root.join("gleam.toml").exists();
    let files: Vec<bool> =
        ops.iter().map(|&op| op.contains("::") || source(op) && (!gleam || !matching(&watched, op).is_empty())).collect();
    if !files.contains(&true) {
        return None;
    }
    Some(view_files(root, &watched, &ops, &files, &flags, budget))
}

fn view_files(root: &Path, watched: &[String], ops: &[&str], files: &[bool], flags: &[&str], budget: usize) -> Out {
    if files.contains(&false) {
        return usage("view takes files (`view <file>`, `view <file>::<name>`) or symbols, not both in one command");
    }
    if !flags.is_empty() {
        return usage("view <file> and view <file>::<name> take no options; `read <file> --lines a..b` reads a range");
    }
    let mut out = String::new();
    for (k, &op) in ops.iter().enumerate() {
        // Several operands share the reply, as `read`'s paths do.
        if k > 0 && budget.saturating_sub(out.len()) < 400 {
            out += &format!("Next: view {}\n", ops[k..].join(" "));
            break;
        }
        let (path, name) = match op.split_once("::") {
            Some((p, n)) => (p, Some(n)),
            None => (op, None),
        };
        let file = match matching(watched, path).as_slice() {
            [one] => one.clone(),
            [] => return lockdown::error("E1001", op, format!("no watched file `{path}`; `files` lists them")),
            many => return lockdown::error("E1002", op, format!("`{path}` could be {}; name one", many.join(", "))),
        };
        let text = fs::read_to_string(root.join(&file))?;
        let (items, sep) = if file.ends_with(".rs") {
            (rust(&text), "::")
        } else if file.ends_with(".py") {
            (python(&text), ".")
        } else {
            return usage(format!("view lists the items of .rs and .py files; `read {file}` reads this one"));
        };
        let room = budget.saturating_sub(out.len());
        let Some(name) = name else {
            out += &outline(&file, &text, &items, room);
            continue;
        };
        let name = if sep == "." { name.replace("::", ".") } else { name.to_string() };
        match find(&items, &name, sep).as_slice() {
            [it] => out += &source_of(&file, &text, it, room),
            [] => {
                return lockdown::error(
                    "E1001",
                    op,
                    format!("no item `{name}` in {file}; `view {file}` lists its items, `grep {name} --in {file} --context 3` finds other text"),
                );
            }
            many => {
                let list: Vec<String> = many.iter().map(|it| format!("    {file}::{} ({}..{})", it.name, it.start, it.end)).collect();
                return lockdown::error("E1002", op, format!("`{name}` could be:\n{}", list.join("\n")));
            }
        }
    }
    Ok(out)
}

/// The watched files a path names: itself, else the files it ends (`board.rs`).
fn matching(watched: &[String], path: &str) -> Vec<String> {
    let path = path.trim_start_matches("./");
    if watched.iter().any(|w| w == path) {
        return vec![path.to_string()];
    }
    watched.iter().filter(|w| w.ends_with(&format!("/{path}"))).cloned().collect()
}

/// The items a name matches: its full name, else a suffix (`Store::view`,
/// `view`); an `impl` only when nothing else does, so a type wins.
fn find<'a>(items: &'a [Item], name: &str, sep: &str) -> Vec<&'a Item> {
    let exact: Vec<&Item> = items.iter().filter(|it| it.name == name).collect();
    let hits = if exact.is_empty() { items.iter().filter(|it| it.name.ends_with(&format!("{sep}{name}"))).collect() } else { exact };
    let named: Vec<&Item> = hits.iter().copied().filter(|it| it.kind != "impl").collect();
    if named.is_empty() { hits } else { named }
}

/// `view <file>`: one line per item, within `budget`: names only when the
/// signatures don't fit, then cut at a whole line (D-14).
fn outline(file: &str, text: &str, items: &[Item], budget: usize) -> String {
    let head = format!("{file}: {} lines, {} items\n", text.lines().count(), items.len());
    if items.is_empty() {
        return head + &format!("No items found; `read {file}` reads it.\n");
    }
    let line = |it: &Item, full: bool| {
        let at = format!("{}{}..{}", "  ".repeat(it.level + 1), it.start, it.end);
        match full {
            true if it.doc.is_empty() => format!("{at} {}\n", it.sig),
            true => format!("{at} {} — {}\n", it.sig, cut(&it.doc, 100)),
            false => format!("{at} {} {}\n", it.kind, it.name),
        }
    };
    let full: String = items.iter().map(|it| line(it, true)).collect();
    if head.len() + full.len() <= budget {
        return head + &full;
    }
    let short: String = items.iter().map(|it| line(it, false)).collect();
    let note = format!("Signatures left out to fit the reply; `view {file}::<name>` shows one item.\n");
    if head.len() + short.len() + note.len() <= budget {
        return head + &short + &note;
    }
    let mut out = head;
    let mut shown = 0;
    for it in items {
        let l = line(it, false);
        if out.len() + l.len() + 200 > budget {
            break;
        }
        out += &l;
        shown += 1;
    }
    out + &format!(
        "{} more items left out to fit the reply; `view {file}::<name>` shows one, `grep <text> --in {file}` finds others.\n",
        items.len() - shown
    )
}

/// `view <file>::<name>`: the item's lines, numbered as `read --lines`
/// numbers them, headed `path:start..end`.
fn source_of(file: &str, text: &str, it: &Item, budget: usize) -> String {
    let mut out = format!("{file}:{}..{}\n", it.start, it.end);
    for (n, l) in text.lines().enumerate().take(it.end).skip(it.start - 1) {
        let l = format!("{}: {l}\n", n + 1);
        if out.len() + l.len() + 100 > budget {
            out += &format!("Next: read {file} --lines {}..{}\n", n + 1, it.end);
            break;
        }
        out += &l;
    }
    out
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() > n { s.chars().take(n - 1).collect::<String>() + "…" } else { s.to_string() }
}

/// How a line starts, for the Rust scanner: in code (not inside a string or
/// block comment) or not, at what bracket depth, and the brackets and `;` in
/// code on it, each with the depth it opens at or closes back to.
struct Line {
    code: bool,
    depth: usize,
    marks: Vec<(char, usize)>,
}

fn lex(lines: &[&str]) -> Vec<Line> {
    #[derive(Clone, Copy, PartialEq)]
    enum In {
        Code,
        Str,
        Raw(usize),
        Block(usize),
    }
    let ident = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut state = In::Code;
    let mut depth = 0;
    let mut out = vec![];
    for line in lines {
        let mut l = Line { code: state == In::Code, depth, marks: vec![] };
        let c: Vec<char> = line.chars().collect();
        let before = |i: usize, k: usize| i.checked_sub(k).and_then(|p| c.get(p));
        let mut i = 0;
        while i < c.len() {
            match state {
                In::Str if c[i] == '\\' => i += 1,
                In::Str if c[i] == '"' => state = In::Code,
                In::Raw(n) if c[i] == '"' && c[i + 1..].iter().take_while(|&&h| h == '#').count() >= n => {
                    state = In::Code;
                    i += n;
                }
                In::Block(n) if c[i] == '*' && c.get(i + 1) == Some(&'/') => {
                    state = if n == 1 { In::Code } else { In::Block(n - 1) };
                    i += 1;
                }
                In::Block(n) if c[i] == '/' && c.get(i + 1) == Some(&'*') => {
                    state = In::Block(n + 1);
                    i += 1;
                }
                In::Code => match c[i] {
                    '/' if c.get(i + 1) == Some(&'/') => break,
                    '/' if c.get(i + 1) == Some(&'*') => {
                        state = In::Block(1);
                        i += 1;
                    }
                    '"' => state = In::Str,
                    // `r"…"`, `r#"…"#`, `br"…"`, but not the end of a name such as `for`.
                    'r' if !ident(before(i, 1)) || matches!(before(i, 1), Some(&('b' | 'c'))) && !ident(before(i, 2)) => {
                        let n = c[i + 1..].iter().take_while(|&&h| h == '#').count();
                        if c.get(i + 1 + n) == Some(&'"') {
                            state = In::Raw(n);
                            i += 1 + n;
                        }
                    }
                    // A char literal (`'{'`, `'\''`), not a lifetime (`'a`).
                    '\'' if c.get(i + 1) == Some(&'\\') => i += 3 + c.iter().skip(i + 3).position(|&q| q == '\'').unwrap_or(0),
                    '\'' if c.get(i + 2) == Some(&'\'') => i += 2,
                    '{' | '(' | '[' => {
                        l.marks.push((c[i], depth));
                        depth += 1;
                    }
                    '}' | ')' | ']' => {
                        depth = depth.saturating_sub(1);
                        l.marks.push((c[i], depth));
                    }
                    ';' => l.marks.push((';', depth)),
                    _ => {}
                },
                _ => {}
            }
            i += 1;
        }
        out.push(l);
    }
    out
}

const KINDS: [&str; 11] = ["fn", "struct", "enum", "union", "trait", "type", "const", "static", "mod", "impl", "macro_rules!"];

/// An item's kind and name from the line its head starts, after visibility
/// and qualifiers: `pub(crate) async fn x`, `impl<T> Display for Y<T>` (`Y`).
fn head(line: &str) -> Option<(&'static str, String)> {
    let mut s = line.trim_start();
    if let Some(r) = s.strip_prefix("pub(") {
        s = r.split_once(')')?.1;
    } else if let Some(r) = s.strip_prefix("pub ") {
        s = r;
    }
    loop {
        s = s.trim_start();
        let next = ["async ", "unsafe ", "default "]
            .iter()
            .find_map(|q| s.strip_prefix(q))
            .or_else(|| s.strip_prefix("const ").filter(|r| ["fn ", "unsafe ", "async ", "extern "].iter().any(|k| r.starts_with(k))))
            .or_else(|| {
                let r = s.strip_prefix("extern ")?.trim_start();
                Some(r.strip_prefix('"').and_then(|q| q.split_once('"')).map_or(r, |(_, rest)| rest))
            });
        match next {
            Some(r) => s = r,
            None => break,
        }
    }
    let (kind, rest) = KINDS.iter().find_map(|&k| {
        let r = s.strip_prefix(k)?;
        (r.starts_with(' ') || k == "impl" && r.starts_with('<')).then_some((k, r.trim_start()))
    })?;
    let name: String = if kind == "impl" {
        let r = if rest.starts_with('<') { &rest[angle_end(rest)..] } else { rest };
        let r = r.split(" where").next()?.split('{').next()?;
        let r = r.rsplit_once(" for ").map_or(r, |(_, t)| t).trim().trim_start_matches(['&', '!']);
        let r = r.strip_prefix("dyn ").unwrap_or(r);
        r.split('<').next()?.rsplit("::").next()?.trim().to_string()
    } else {
        let r = rest.strip_prefix("mut ").unwrap_or(rest);
        r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
    };
    (!name.is_empty()).then_some((kind, name))
}

/// The byte after the `>` that closes the generics a text starts with.
fn angle_end(s: &str) -> usize {
    let mut depth = 0;
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if prev != '-' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        prev = c;
    }
    s.len()
}

/// Where the item whose head is on line `head` (0-based) ends: where its
/// brackets close, or at its `;` (`mod x;`, a `const`, a tuple struct).
fn item_end(lex: &[Line], head: usize, depth: usize, kind: &str) -> usize {
    let semi_only = matches!(kind, "const" | "static" | "type");
    let mut opened = false;
    for (n, l) in lex.iter().enumerate().skip(head) {
        for &(c, d) in &l.marks {
            if d != depth {
                continue;
            }
            match c {
                ';' if semi_only || !opened => return n,
                '{' if !semi_only => opened = true,
                '}' if opened => return n,
                _ => {}
            }
        }
    }
    lex.len() - 1
}

/// The head's text up to its body: wrapped parameters joined on one line.
fn signature(lines: &[&str], kind: &str) -> String {
    let short = matches!(kind, "const" | "static" | "type");
    let mut sig = String::new();
    for line in lines.iter().take(12) {
        sig += line.trim();
        sig.push(' ');
        if short || line.contains('{') || line.trim_end().ends_with(';') {
            break;
        }
    }
    let mut sig = sig.replace("( ", "(").replace(", )", ")").replace(" )", ")");
    if let Some(p) = sig.find(if short { " = " } else { " {" }) {
        sig.truncate(p);
    }
    cut(sig.trim_end_matches([' ', '{', ';', ',']), 140)
}

fn rust(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    let lex = lex(&lines);
    let mut items = vec![];
    rust_block(&lines, &lex, 0..lines.len(), 0, "", &mut items);
    items
}

/// The items whose heads start lines of `range` at `depth`, and the items of
/// the `impl`, `trait` and `mod` blocks among them.
fn rust_block(lines: &[&str], lex: &[Line], range: std::ops::Range<usize>, depth: usize, prefix: &str, items: &mut Vec<Item>) {
    let from = range.start;
    let mut i = from;
    while i < range.end {
        let Some((kind, name)) = (lex[i].code && lex[i].depth == depth).then(|| head(lines[i])).flatten() else {
            i += 1;
            continue;
        };
        let end = item_end(lex, i, depth, kind).min(range.end - 1);
        // Docs, attributes and comments right above the head go with it.
        let mut start = i;
        while start > from && lex[start - 1].code && lex[start - 1].depth == depth && {
            let t = lines[start - 1].trim_start();
            t.starts_with("#[") || t.starts_with("//") && !t.starts_with("//!")
        } {
            start -= 1;
        }
        let above = &lines[start..i];
        let doc = above.iter().find_map(|l| l.trim_start().strip_prefix("///")).unwrap_or("").trim().to_string();
        let mut sig = signature(&lines[i..=end], kind);
        // `#[path = "x.rs"] mod x;`: the module is in that file.
        if let Some((file, _)) = above.iter().find_map(|l| l.trim_start().strip_prefix("#[path = \"")?.split_once('"')) {
            sig += &format!(" → {file}");
        }
        let full = if prefix.is_empty() { name } else { format!("{prefix}::{name}") };
        items.push(Item { kind, name: full.clone(), sig, doc, start: start + 1, end: end + 1, level: depth });
        if matches!(kind, "impl" | "trait" | "mod") && end > i + 1 {
            rust_block(lines, lex, i + 1..end, depth + 1, &full, items);
        }
        i = end + 1;
    }
}

/// Python by indentation: `def` and `class` at the top level and in classes
/// (`Class.method`), each up to the next line no deeper than its head.
fn python(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    // Whether each line starts outside a triple-quoted string.
    let mut code = vec![];
    let mut open: Option<&str> = None;
    for line in &lines {
        code.push(open.is_none());
        let mut rest = *line;
        loop {
            let next = match open {
                Some(q) => rest.find(q).map(|p| (p, None)),
                None => ["\"\"\"", "'''"].iter().filter_map(|&q| Some((rest.find(q)?, Some(q)))).min(),
            };
            let Some((p, q)) = next else { break };
            rest = &rest[p + 3..];
            open = q;
        }
    }
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut items = vec![];
    // The `def`s and `class`es around the line: indent, name, kind.
    let mut scopes: Vec<(usize, String, &'static str)> = vec![];
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if !code[i] || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let at = indent(line);
        scopes.retain(|s| s.0 < at);
        let (kind, rest) = match t.strip_prefix("def ").or_else(|| t.strip_prefix("async def ")) {
            Some(r) => ("def", r),
            None => match t.strip_prefix("class ") {
                Some(r) => ("class", r),
                None => continue,
            },
        };
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let listed = !name.is_empty() && scopes.iter().all(|s| s.2 == "class");
        let full = scopes.iter().map(|s| s.1.as_str()).chain([name.as_str()]).collect::<Vec<_>>().join(".");
        let level = scopes.len();
        scopes.push((at, name, kind));
        if !listed {
            continue;
        }
        let mut start = i;
        while start > 0 && code[start - 1] && indent(lines[start - 1]) == at && lines[start - 1].trim_start().starts_with(['@', '#']) {
            start -= 1;
        }
        // The head runs to the line ending in `:`; the body, to a line no deeper than the head.
        let head_end = (i..lines.len().min(i + 12)).find(|&k| lines[k].trim_end().ends_with(':')).unwrap_or(i);
        let mut end = head_end;
        for k in head_end + 1..lines.len() {
            let t = lines[k].trim_start();
            if t.is_empty() {
                continue;
            }
            let shallow = code[k] && indent(lines[k]) <= at;
            if shallow && !t.starts_with('#') {
                break;
            }
            if !shallow {
                end = k;
            }
        }
        let sig = lines[i..=head_end].iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ").replace("( ", "(").replace(", )", ")");
        let doc = lines
            .get(head_end + 1)
            .map(|l| l.trim())
            .and_then(|l| l.strip_prefix("\"\"\"").or_else(|| l.strip_prefix("'''")))
            .map(|d| d.trim_end_matches(['"', '\'']).trim().to_string())
            .unwrap_or_default();
        items.push(Item { kind, name: full, sig: cut(sig.trim_end_matches(':'), 140), doc, start: start + 1, end: end + 1, level });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    const SAMPLE: &str = r####"//! A module.
use std::fs;

/// The store.
#[derive(Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// Opens it.
    pub fn open() -> Self {
        let brace = '{';
        let s = "}";
        Store { root: PathBuf::new() }
    }

    pub(crate) fn view(&self, names: &[&str]) -> Out {
        Ok(String::new())
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

pub fn view() {}

const GLEAM: &[&str] = &[
    "submit",
];

pub fn outer() {
    fn nested() {}
    let src = r#"pub fn main() {
}
fn fake() {
}"#;
}

#[cfg(test)]
// The tests are here.
#[path = "sample_tests.rs"]
mod more;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opens() {
        assert!(Store::open().root.as_os_str().is_empty());
    }
}
"####;

    const PY: &str = "import os\n\n\nclass View(unittest.TestCase):\n    \"\"\"Views.\"\"\"\n\n    def setUp(self):\n        self.x = '''\ndef fake():\n'''\n\n    @skip\n    def test_one(self):\n        def inner():\n            pass\n        inner()\n\n\ndef helper(a,\n           b):\n    \"\"\"Helps.\"\"\"\n    return a\n";

    #[test]
    fn rust_items_take_their_docs_and_skip_strings_and_nested_fns() {
        let items = rust(SAMPLE);
        let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
        assert_eq!(
            got,
            [
                ("Store", 4, 8, 0),
                ("Store", 10, 21, 0),
                ("Store::open", 11, 16, 1),
                ("Store::view", 18, 20, 1),
                ("Failure", 23, 25, 0),
                ("Failure::fmt", 24, 24, 1),
                ("view", 27, 27, 0),
                ("GLEAM", 29, 31, 0),
                ("outer", 33, 39, 0),
                ("more", 41, 44, 0),
                ("tests", 46, 53, 0),
                ("tests::opens", 49, 52, 1),
            ]
        );
        let sig = |n: &str, k: &str| items.iter().find(|i| i.name == n && i.kind == k).unwrap().sig.clone();
        assert_eq!(sig("Store", "struct"), "pub struct Store");
        assert_eq!(sig("Failure", "impl"), "impl fmt::Display for Failure");
        assert_eq!(sig("Store::view", "fn"), "pub(crate) fn view(&self, names: &[&str]) -> Out");
        assert_eq!(sig("GLEAM", "const"), "const GLEAM: &[&str]");
        assert_eq!(sig("more", "mod"), "mod more → sample_tests.rs");
        assert_eq!(items[0].doc, "The store.");
    }

    #[test]
    fn names_match_in_full_or_by_suffix() {
        let items = rust(SAMPLE);
        let one = |n: &str| find(&items, n, "::").iter().map(|i| (i.name.clone(), i.kind)).collect::<Vec<_>>();
        assert_eq!(one("Store"), [("Store".to_string(), "struct")], "the type, not its impl");
        assert_eq!(one("view"), [("view".to_string(), "fn")], "a full name wins over a suffix");
        assert_eq!(one("Store::view"), [("Store::view".to_string(), "fn")]);
        assert_eq!(one("opens"), [("tests::opens".to_string(), "fn")]);
        assert!(one("nested").is_empty() && one("fake").is_empty() && one("main").is_empty());
        let two = rust("impl A {\n    fn new() {}\n}\nimpl B {\n    fn new() {}\n}\n");
        assert_eq!(find(&two, "new", "::").len(), 2);
    }

    #[test]
    fn python_items_follow_indentation() {
        let items = python(PY);
        let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
        assert_eq!(got, [("View", 4, 16, 0), ("View.setUp", 7, 10, 1), ("View.test_one", 12, 16, 1), ("helper", 19, 22, 0)]);
        assert_eq!((items[0].doc.as_str(), items[3].doc.as_str(), items[3].sig.as_str()), ("Views.", "Helps.", "def helper(a, b)"));
    }

    #[test]
    fn view_gives_an_outline_or_an_item_and_names_a_miss() {
        let dir = Scratch::new().with(&[("src/lib.rs", SAMPLE), ("src/other/lib.rs", "fn a() {}\n"), ("tests/check.py", PY)]);
        let v = toml::Value::Table(Default::default());
        let run = |a: &[&str], budget: usize| view(&dir, &v, a, budget).expect("a file form");
        let outline = run(&["view", "src/lib.rs"], 32_000).unwrap();
        assert!(outline.starts_with("src/lib.rs: 53 lines, 12 items\n"), "{outline}");
        assert!(outline.contains("  4..8 pub struct Store — The store.\n"), "{outline}");
        assert!(outline.contains("    11..16 pub fn open() -> Self — Opens it.\n"), "{outline}");
        assert!(outline.contains("  41..44 mod more → sample_tests.rs\n"), "{outline}");
        let item = run(&["view", "src/lib.rs::Store::open"], 32_000).unwrap();
        assert_eq!(
            item,
            "src/lib.rs:11..16\n11:     /// Opens it.\n12:     pub fn open() -> Self {\n13:         let brace = '{';\n14:         let s = \"}\";\n15:         Store { root: PathBuf::new() }\n16:     }\n"
        );
        let e = run(&["view", "lib.rs"], 32_000).unwrap_err().0;
        assert!(e.contains("E1002") && e.contains("src/other/lib.rs"), "{e}");
        let e = run(&["view", "src/lib.rs::nested"], 32_000).unwrap_err().0;
        assert!(e.contains("E1001") && e.contains("grep nested --in src/lib.rs --context 3"), "{e}");
        assert!(run(&["view", "check.py::test_one"], 32_000).unwrap().starts_with("tests/check.py:12..16\n12:     @skip\n"));
        // Over the budget: names only, then a cut at a whole line.
        let short = run(&["view", "src/lib.rs"], outline.len() - 1).unwrap();
        assert!(short.contains("  4..8 struct Store\n") && short.contains("    11..16 fn Store::open\n"), "{short}");
        assert!(short.contains("Signatures left out to fit the reply") && !short.contains("The store."), "{short}");
        let cut = run(&["view", "src/lib.rs"], 280).unwrap();
        assert!(cut.contains("more items left out to fit the reply") && cut.len() < 400, "{cut}");
        // Symbols are the store's, and the two don't mix.
        assert!(view(&dir, &v, &["view", "app.main"], 32_000).is_none());
        assert!(run(&["view", "src/lib.rs", "app.main"], 32_000).unwrap_err().0.contains("not both"));
        let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n")]);
        assert!(view(&gleam, &v, &["view", "app.rs"], 32_000).is_none(), "a Gleam symbol `app.rs`");
    }

    #[test]
    fn the_scanner_reads_this_crate() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in fs::read_dir(&src).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = fs::read_to_string(&p).unwrap();
            let lines = text.lines().count();
            let mut last = 0;
            for it in rust(&text).iter().filter(|i| i.level == 0) {
                assert!(last < it.start && it.start <= it.end && it.end <= lines, "{}: {it:?}", p.display());
                last = it.end;
            }
        }
        let read = |f: &str| rust(&fs::read_to_string(src.join(f)).unwrap());
        let board = read("board.rs");
        assert!(board.iter().any(|i| i.name == "tests" && i.sig.ends_with("→ board_tests.rs")));
        assert!(board.iter().any(|i| i.name == "brief" && i.kind == "fn"));
        let view = read("view.rs");
        assert!(!view.iter().any(|i| i.sig.contains("fn write(path: String")), "Gleam in a raw string is not an item");
        assert!(view.iter().any(|i| i.name == "tests::c1_graph_has_entries_effects_and_tests"));
        let lockdown = read("lockdown.rs");
        assert!(lockdown.iter().any(|i| i.name == "dispatch"));
        // The top-level `drop_records`, not the one nested in `carry_file`.
        assert_eq!(lockdown.iter().filter(|i| i.name.ends_with("drop_records")).count(), 1);
    }
}
