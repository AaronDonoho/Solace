//! `view <file>` and `view <file>::<name>` for Rust and Python files (D-121).
//! Rust items come from the `syn` splitter (`language::rust`); Python's from
//! a line scanner that follows indentation.

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

/// `view <file>` (an outline) and `view <file>::<name>` (one item's source) for
/// Python and plain-file Rust, or None when the operands are symbols for the
/// store: in a project with Gleam code, a `.rs` name that matches no watched
/// file is one (`app.rs`). A graph path is refused (D-181).
pub fn view(store: &Store, v: &toml::Value, a: &[&str], budget: usize) -> Option<Out> {
    let root = &store.root;
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
    // An operand is a file only when its path part, before `::`, is a `.rs` or
    // `.py` file; any other name in a Rust project is the store's (D-158).
    let rust = crate::language::is_rust(root);
    fn path_of(op: &str) -> &str {
        op.split_once("::").map_or(op, |(p, _)| p)
    }
    fn graph_path(name: &str) -> Option<String> {
        let path = std::path::Path::new(name);
        if path.is_absolute() {
            return None;
        }
        let path = crate::file_name(path);
        (!path.split('/').any(|part| part == "..")).then_some(path)
    }
    // Only an operand that could name a source file needs the tree: a symbol
    // view stays cheap.
    let graph = if ops.iter().any(|op| {
        let path = path_of(op);
        path.ends_with(".rs") || path.ends_with(".gleam")
    }) {
        match lockdown::graph_paths(root, v) {
            Ok(paths) => paths,
            Err(e) => return Some(Err(e)),
        }
    } else {
        vec![]
    };
    // A graph path is refused before the file forms: an exact graph path, or a
    // new path under a graph root. A bare `app.rs` stays a symbol name (D-181).
    for &op in &ops {
        let raw = path_of(op);
        if !(raw.ends_with(".rs") || raw.ends_with(".gleam")) {
            continue;
        }
        let Some(path) = graph_path(raw) else {
            return Some(lockdown::error("E6001", raw, "this path goes outside the project"));
        };
        if graph.iter().any(|p| p == &path) || lockdown::tier(root, v, &path, None) == "graph" {
            return Some(lockdown::graph_error(store, &path));
        }
    }
    let file = |op: &str| match rust {
        false => op.contains("::") || source(op),
        true => {
            let path = op.split_once("::").map_or(op, |(p, _)| p);
            path.ends_with(".rs") || path.ends_with(".py")
        }
    };
    if !ops.iter().any(|&op| file(op)) {
        return None;
    }
    let watched = match lockdown::watched_paths(root, v) {
        Ok(w) => w,
        Err(e) => return Some(Err(e)),
    };
    let gleam = crate::language::detect(root).is_some();
    let files: Vec<bool> = ops
        .iter()
        .map(|&op| match rust {
            true => file(op),
            false => op.contains("::") || source(op) && (!gleam || !matching(&watched, op).is_empty()),
        })
        .collect();
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
        // A refusal of one operand is that operand's part of the reply, as `read`'s
        // is, so the other operands are still answered (D-119 point 5).
        let file = match matching(watched, path).as_slice() {
            [one] => one.clone(),
            [] => {
                out += &lockdown::error::<()>("E1001", op, format!("no watched file `{path}`; `files` lists them")).unwrap_err().0;
                continue;
            }
            many => {
                out += &lockdown::error::<()>("E1002", op, format!("`{path}` could be {}; name one", many.join(", "))).unwrap_err().0;
                continue;
            }
        };
        let text = fs::read_to_string(root.join(&file))?;
        let (items, sep) = if file.ends_with(".rs") {
            match rust(&text) {
                Ok(items) => (items, "::"),
                Err(e) => {
                    out += &format!("{file}: {}; `read {file}` reads it.\n", e.0);
                    continue;
                }
            }
        } else if file.ends_with(".py") {
            (python(&text), ".")
        } else {
            out += &usage::<()>(format!("view lists the items of .rs and .py files; `read {file}` reads this one")).unwrap_err().0;
            continue;
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
                out += &lockdown::error::<()>(
                    "E1001",
                    op,
                    format!(
                        "no item `{name}` in {file}; `view {file}` lists its items, `grep {name} --in {file} --context 3` finds other text"
                    ),
                )
                .unwrap_err()
                .0;
                continue;
            }
            many => {
                let list: Vec<String> = many.iter().map(|it| format!("    {file}::{} ({}..{})", it.name, it.start, it.end)).collect();
                out += &lockdown::error::<()>("E1002", op, format!("`{name}` could be:\n{}", list.join("\n"))).unwrap_err().0;
                continue;
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

/// The signature of a Rust item as the store keeps its text: its leading
/// comments and attributes are dropped, then `signature` reads the head (D-158).
pub fn rust_signature(text: &str, kind: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut at = 0;
    while at < lines.len() {
        let t = lines[at].trim_start();
        if t.starts_with("//") {
            at += 1;
        } else if t.starts_with("#[") || t.starts_with("#!") {
            // A wrapped attribute runs to the line that closes it.
            while at < lines.len() && !lines[at].contains(']') {
                at += 1;
            }
            at += 1;
        } else {
            break;
        }
    }
    signature(&lines[at.min(lines.len())..], kind)
}

fn rust(text: &str) -> Result<Vec<Item>, Failure> {
    let file = crate::language::rust::split(text, "crate")?;
    let mut items = vec![];
    rust_items(text, &file.items, 0, &mut items);
    Ok(items)
}

/// The outline's items of `pieces` and of their children, imports left out.
/// An item starts at its first doc or attribute: a plain `//` comment above it
/// is not shown with it (D-119 point 4).
fn rust_items(text: &str, pieces: &[crate::language::rust::Piece], level: usize, items: &mut Vec<Item>) {
    let line = |b: usize| text[..b].matches('\n').count() + 1;
    for p in pieces.iter().filter(|p| p.kind != "use") {
        let lines: Vec<&str> = text[p.head..p.item.end].lines().collect();
        let above: Vec<&str> = text[p.item.start..p.head].lines().collect();
        let doc = above.iter().find_map(|l| l.trim_start().strip_prefix("///")).unwrap_or("").trim().to_string();
        let mut sig = signature(&lines, p.kind);
        // `#[path = "x.rs"] mod x;`: the module is in that file.
        if let Some(file) = &p.file {
            sig += &format!(" → {file}");
        }
        items.push(Item { kind: p.kind, name: p.name.clone(), sig, doc, start: line(p.item.start), end: line(p.item.end), level });
        rust_items(text, &p.children, level + 1, items);
    }
}

/// Python by indentation: `def` and `class` at the top level and in classes
/// (`Class.method`), each up to the next line no deeper than its head.
fn python(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    // Whether each line starts outside a triple-quoted string, scanned by byte: a
    // `#` comment can't open a string, and a lone quote in one is not a string's
    // start (D-119 point 4).
    let mut code = vec![];
    let mut open: Option<u8> = None;
    for line in &lines {
        code.push(open.is_none());
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if let Some(q) = open {
                if c == q && b[i..].starts_with(&[q; 3]) {
                    open = None;
                    i += 3;
                } else {
                    i += 1;
                }
            } else if c == b'#' {
                break;
            } else if c == b'"' || c == b'\'' {
                if b[i..].starts_with(&[c; 3]) {
                    open = Some(c);
                    i += 3;
                } else {
                    // A one-line string: it ends at the next same quote that
                    // is not escaped; `\"` inside a string is not its end.
                    let mut j = i + 1;
                    while j < b.len() {
                        if b[j] == b'\\' {
                            j += 2;
                        } else if b[j] == c {
                            break;
                        } else {
                            j += 1;
                        }
                    }
                    i = if j < b.len() { j + 1 } else { i + 1 };
                }
            } else {
                i += 1;
            }
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
#[path = "orientation_tests.rs"]
mod tests;
