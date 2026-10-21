//! The Rust side of the store (D-158): a file split by `syn` is kept as a module with its items
//! nested, and written back from those rows exactly.

use super::*;

/// Stores a Rust file split by `syn` as module `module` of `file`, its items
/// nested in their containers, to be written back exactly (D-158 point 5).
pub fn store_rust_file(db: &Connection, module: &str, dir: &str, file: &str, split: &language::rust::File) -> Result<(), Failure> {
    db.execute(
        "INSERT INTO modules(path, dir, header, file, owns_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![module, dir, split.header, file, split.owns_dir],
    )?;
    store_pieces(db, module, None, &split.items)
}

fn store_pieces(db: &Connection, module: &str, parent: Option<&str>, pieces: &[language::rust::Piece]) -> Result<(), Failure> {
    for (ord, p) in pieces.iter().enumerate() {
        let (text, tail, sep) = piece_parts(p);
        let rev = revision(&text);
        let id = new_id(language::rust::id_kind(p.kind));
        db.execute(
            "INSERT INTO symbols(id, module, name, kind, text, ord, revision, parent, sep, tail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id, module, p.name, p.kind, text, ord as i64, rev, parent, sep, tail],
        )?;
        store_pieces(db, module, Some(&id), &p.children)?;
    }
    Ok(())
}

/// A piece's stored parts (D-158 point 5): the item's own text, the tail after
/// its items, and the whitespace that follows it. The one cut both storings
/// use, an import's and a `submit`'s whole block (D-168).
pub(crate) fn piece_parts(p: &language::rust::Piece) -> (String, String, String) {
    let container = !p.children.is_empty();
    let last = if container { &p.tail } else { &p.text };
    let body = last.trim_end();
    let sep = &last[body.len()..];
    if container { (p.text.clone(), body.to_string(), sep.to_string()) } else { (body.to_string(), String::new(), sep.to_string()) }
}

/// A Rust item `submit` saves (D-168): the store's name for it, its kind from
/// the file, its text without the whitespace its separator carries and, for a
/// block saved whole, the items it holds and the tail that closes it.
pub struct Submitted {
    pub name: String,
    pub kind: &'static str,
    pub text: String,
    pub children: Vec<Submitted>,
    pub tail: String,
}

/// Whether a saved item is a block whose items Solace holds nested (D-168).
pub(crate) fn is_container(item: &Submitted) -> bool {
    !item.children.is_empty() || matches!(item.kind, "impl" | "trait" | "mod" | "extern")
}

/// Whether a piece is a block `submit --new` saves whole (D-168): a container
/// with items, an empty `impl`, `trait` or `extern` block, or an inline
/// `mod { … }` — `mod x;` declares another file instead.
pub(crate) fn is_block(p: &language::rust::Piece) -> bool {
    !p.children.is_empty() || matches!(p.kind, "impl" | "trait" | "extern") || (p.kind == "mod" && !p.text.trim_end().ends_with(';'))
}

/// A block's own text can't be saved as an item (D-168): a new block goes
/// through `submit --new <module>`, and an existing one changes item by item.
pub(crate) fn block_note(kind: &str) -> String {
    format!("a new `{kind}` block goes through `submit --new <module>`; an existing one changes item by item")
}

/// Why a `mod x;` in submitted text isn't saved (D-168): Solace's modules come
/// from `submit --new`, and a module goes when its items go.
pub(crate) const MODULE_NOTE: &str = "a new module comes from `submit --new <module>`, and removing one comes from deleting its items";

/// A new module's whole text (D-168): formatted on its own — there is no parent
/// container — then split into its header (its docs and `use` lines) and its
/// items, the way `store_rust_file` stores an imported file. A `mod x;` in it
/// names a module this submit doesn't create.
pub fn submit_module(root: &Path, module: &str, text: &str, formatted: bool) -> Result<(String, Vec<Submitted>), Failure> {
    // A `mod x;` names a module this submit doesn't create; it is looked for in
    // the text as written, since rustfmt resolves the file it formats (D-168).
    if let Ok(split) = language::rust::split(text, module)
        && let Some(name) = split.mods.first().map(|(n, _)| n.clone()).or_else(|| file_mod(&split.items).map(|p| p.name.clone()))
    {
        return fail(format!("Not saved: `mod {name};` names another module; {MODULE_NOTE}."));
    }
    let lines = text.lines().count().max(1);
    let source = if formatted {
        text.to_string()
    } else {
        cargo::format_item(root, text, None).map_err(|e| rust_syntax_error(e, module, None, module, 0, lines))?
    };
    let file = language::rust::split(&source, module).map_err(|e| rust_syntax_error(e, module, None, module, 0, lines))?;
    if !file.refusals.is_empty() {
        let lines: Vec<String> = file.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
        return fail(format!("Not saved: the Rust text is outside the subset Solace holds.\n{}", lines.join("\n")));
    }
    Ok((file.header, file.items.iter().map(submitted).collect()))
}

/// The items `text` defines in `module` (D-168): as written for a top-level
/// item or a whole `impl`, `trait`, inline `mod` or `extern` block with its
/// items, or the children of `parent` (its name and its stored opening text)
/// when the item sits inside a container. Names are the store's: a nested
/// item's own name behind its parent's. `formatted`: `text` is already rustfmt's
/// text (a patch's block), so it is not formatted again. `label` names the
/// block in a syntax error (`<module>.<name>`).
pub fn submit_items(
    root: &Path,
    module: &str,
    label: &str,
    parent: Option<(&str, &str, &str)>,
    text: &str,
    formatted: bool,
) -> Result<Vec<Submitted>, Failure> {
    // A block is saved whole at the top level; a named target holds no room for
    // one, and `mod x;` names a file a `submit` doesn't create (D-168). A
    // nested item's text doesn't parse on its own, so a refusal to parse is not
    // one of these.
    if let Ok(file) = language::rust::split(text, module) {
        let named = file.mods.first().map(|(n, _)| n.clone()).or_else(|| file_mod(&file.items).map(|p| p.name.clone()));
        if let Some(name) = named {
            return fail(format!("Not saved: `mod {name};` names another module; {MODULE_NOTE}."));
        }
        // A whole block nests in an inline `mod` only: every other container
        // takes leaves, so a named target refuses it (D-168).
        if let Some(p) = file.items.iter().find(|p| is_block(p))
            && parent.is_some_and(|(_, _, kind)| kind != "mod")
        {
            return fail(format!("Not saved: {}.", block_note(p.kind)));
        }
    }
    // An empty container is stored whole, so the text before its closing brace
    // is the wrapper its first item is formatted inside (D-168).
    let empty = parent.and_then(|(_, opening, _)| empty_container(opening, module));
    let opening = empty.clone().or_else(|| parent.map(|(_, opening, _)| opening.to_string()));
    let offset = opening.as_ref().map_or(0, |opening| opening.lines().count());
    let lines = text.lines().count().max(1);
    let source = if formatted {
        match &opening {
            Some(opening) => format!("{opening}\n{text}\n}}\n"),
            None => text.to_string(),
        }
    } else {
        cargo::format_item(root, text, opening.as_deref()).map_err(|e| rust_syntax_error(e, label, parent, module, 0, lines))?
    };
    let file = language::rust::split(&source, module).map_err(|e| rust_syntax_error(e, label, parent, module, offset, lines))?;
    if !file.refusals.is_empty() {
        let lines: Vec<String> = file.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
        return fail(format!("Not saved: the Rust text is outside the subset Solace holds.\n{}", lines.join("\n")));
    }
    // A block's `use` items are the module's imports, which `submit --imports`
    // writes; an item block's own are dropped, so saving one loses them.
    if language::rust::imports(text) {
        let imports: Vec<String> =
            language::rust::import_texts(text).into_iter().map(|import| import.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
        let imports: Vec<String> = imports.iter().map(|import| format!("`{import}`")).collect();
        return fail(format!("Not saved: block {label} carries {}; imports go through `submit --imports {module}`.", imports.join(", ")));
    }
    let Some((name, opening, _)) = parent else {
        return Ok(file.items.iter().map(submitted).collect());
    };
    let base = first_indent(opening);
    // The wrapper holds the parent container; its child is the item, named as
    // the file names it: the parent's name, `::`, the child's own.
    let [one] = file.items.as_slice() else {
        return fail(format!("E0101 each block holds one symbol; the text defines {}.", file.items.len()));
    };
    if empty.is_some() {
        // The item is inside the container now: the container's row takes the
        // re-split text and tail, and its items keep the parent's name.
        return Ok(vec![refilled(name, one, base)]);
    }
    // An `extern` block's items are its module's own (D-162), so they take the
    // parent's prefix, not the block's name; every other container's take its.
    let prefix = if one.kind == "extern" { parent_prefix(name) } else { format!("{name}::") };
    Ok(one.children.iter().map(|c| nested_child(&one.name, &prefix, c, base, formatted)).collect())
}

/// One item of a Rust module formatted on its own (D-158 point 5, D-168): the
/// formatting `submit_items` gives a named block without its item-level
/// refusals, for a whole-module `submit`, which has already split the module.
/// `parent` is the container the item sits in (its stored name, its opening
/// text and its kind), so the item is formatted inside that wrapper and
/// reindented to the wrapper's level; `None` at the top level.
pub(crate) fn format_one(root: &Path, module: &str, parent: Option<(&str, &str, &str)>, text: &str) -> Result<Submitted, Failure> {
    let opening = parent.map(|(_, opening, _)| opening);
    let source = cargo::format_item(root, text, opening)?;
    let file = language::rust::split(&source, module)?;
    if !file.refusals.is_empty() {
        let lines: Vec<String> = file.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
        return fail(format!("Not saved: the Rust text is outside the subset Solace holds.\n{}", lines.join("\n")));
    }
    let Some((name, opening, _)) = parent else {
        return match file.items.as_slice() {
            [one] => Ok(submitted(one)),
            _ => fail("E0101 the text defines no one item."),
        };
    };
    let base = first_indent(opening);
    let [one] = file.items.as_slice() else {
        return fail("E0101 the text defines no one item.");
    };
    let prefix = if one.kind == "extern" { parent_prefix(name) } else { format!("{name}::") };
    match one.children.first() {
        Some(c) => Ok(nested_child(&one.name, &prefix, c, base, false)),
        None => fail("E0101 the text defines no one item."),
    }
}

/// A piece of a whole block `submit --new` saves (D-168), its items nested the
/// way `store_pieces` writes them back.
fn submitted(p: &language::rust::Piece) -> Submitted {
    let (text, tail, _) = piece_parts(p);
    Submitted { name: p.name.clone(), kind: p.kind, text, children: p.children.iter().map(submitted).collect(), tail }
}

/// An empty container's stored text, cut before its closing brace so an item is
/// formatted inside it (D-168); `None` for anything else.
fn empty_container(text: &str, module: &str) -> Option<String> {
    let file = language::rust::split(text, module).ok()?;
    let [p] = file.items.as_slice() else { return None };
    if !is_block(p) || !p.children.is_empty() {
        return None;
    }
    let trimmed = text.trim_end();
    Some(trimmed[..trimmed.rfind('}')?].trim_end().to_string())
}

/// An empty container's row once its first item lands (D-168): the re-split
/// container's text and tail, back at the level its file stores, and its items
/// named behind the parent's name — an `extern` block's as its module's own.
fn refilled(name: &str, one: &language::rust::Piece, base: &str) -> Submitted {
    let prefix = if one.kind == "extern" { parent_prefix(name) } else { format!("{name}::") };
    let (text, tail, _) = piece_parts(one);
    Submitted {
        name: name.to_string(),
        kind: one.kind,
        text: reindent(&text, base),
        children: one.children.iter().map(|c| nested_child(&one.name, &prefix, c, base, false)).collect(),
        tail: reindent(&tail, base),
    }
}

/// An item a container takes, nested as the store keeps it (D-168): its name
/// behind the store's `prefix`, its text at the parent's level, and a whole
/// block's items under it.
fn nested_child(wrapper: &str, prefix: &str, p: &language::rust::Piece, base: &str, formatted: bool) -> Submitted {
    let own = p.name.strip_prefix(&format!("{wrapper}::")).unwrap_or(&p.name);
    let (text, tail, _) = piece_parts(p);
    let level = |t: &str| if formatted { t.to_string() } else { reindent(t, base) };
    Submitted {
        name: format!("{prefix}{own}"),
        kind: p.kind,
        text: level(&text),
        children: p.children.iter().map(|c| nested_child(wrapper, prefix, c, base, formatted)).collect(),
        tail: level(&tail),
    }
}

/// The first `mod x;` among `pieces`, at any depth (D-168).
fn file_mod(pieces: &[language::rust::Piece]) -> Option<&language::rust::Piece> {
    pieces.iter().find_map(|p| {
        if p.kind == "mod" && p.children.is_empty() && p.text.trim_end().ends_with(';') { Some(p) } else { file_mod(&p.children) }
    })
}

/// The leading whitespace of `text`'s first line: rustfmt lays a wrapper's item
/// out at the file's first level, so this is what its parent adds to it.
fn first_indent(text: &str) -> &str {
    let line = text.lines().next().unwrap_or("");
    &line[..line.len() - line.trim_start().len()]
}

/// An `extern` parent's prefix (`tests` in `tests::extern "C"`), where its
/// items' names live (D-162).
fn parent_prefix(name: &str) -> String {
    name.rsplit_once("::").map_or(String::new(), |(prefix, _)| format!("{prefix}::"))
}

/// `text` with `base` in front of every line: rustfmt takes a wrapper's own
/// indentation away, so a nested parent's is added back (D-168). A line inside
/// a string literal is left as written, so a multi-line literal doesn't change.
fn reindent(text: &str, base: &str) -> String {
    if base.is_empty() {
        return text.to_string();
    }
    let literals = language::rust::string_literals(text);
    let mut out = String::new();
    let mut at = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            at += 1;
        }
        if !line.is_empty() && !literals.iter().any(|lit| lit.start < at && at < lit.end) {
            out.push_str(base);
        }
        out.push_str(line);
        at += line.len();
    }
    out
}

/// A module's file as Solace writes it, with the line range of each symbol.
pub fn projection(ix: &Index, m: &ModuleRow) -> (String, Vec<(usize, usize, String)>) {
    let mut content = String::new();
    let mut ranges = vec![];
    if m.file.is_some() {
        content.push_str(&m.header);
        let groups = by_parent(ix, &m.path);
        let mut line = content.matches('\n').count() + 1;
        project_exact(&groups, None, &mut content, &mut line, &mut ranges);
        return (content, ranges);
    }
    let mut line = 1;
    if !m.header.is_empty() {
        content.push_str(&m.header);
        content.push_str("\n\n");
        line += m.header.lines().count() + 1;
    }
    for s in ix.syms.iter().filter(|s| s.module == m.path) {
        let n = s.text.lines().count();
        ranges.push((line, line + n - 1, s.qname()));
        content.push_str(&s.text);
        content.push_str("\n\n");
        line += n + 1;
    }
    (content.trim_end().to_string() + "\n", ranges)
}

/// A module's symbols by the parent they sit under, so `project_exact` visits each group once
/// instead of scanning every symbol for each parent.
pub(crate) fn by_parent<'a>(ix: &'a Index, module: &str) -> HashMap<Option<&'a str>, Vec<&'a Sym>> {
    let mut groups: HashMap<Option<&str>, Vec<&Sym>> = HashMap::new();
    for s in ix.syms.iter().filter(|s| s.module == module) {
        groups.entry(s.parent.as_deref()).or_default().push(s);
    }
    groups
}

/// `m`'s items under `parent` as stored, each with its children, tail and separator; `line` is
/// the 1-based line the next write lands on.
fn project_exact<'a>(
    groups: &HashMap<Option<&'a str>, Vec<&'a Sym>>,
    parent: Option<&'a str>,
    content: &mut String,
    line: &mut usize,
    ranges: &mut Vec<(usize, usize, String)>,
) {
    for s in groups.get(&parent).map(Vec::as_slice).unwrap_or(&[]) {
        let start = *line;
        content.push_str(&s.text);
        *line += s.text.matches('\n').count();
        project_exact(groups, Some(&s.id), content, line, ranges);
        content.push_str(&s.tail);
        *line += s.tail.matches('\n').count();
        ranges.push((start, *line, s.qname()));
        let sep = s.sep.as_deref().unwrap_or("");
        content.push_str(sep);
        *line += sep.matches('\n').count();
    }
}

/// Whether `input` names `s`: its own name, or its name behind its module, the
/// module exact or a suffix; `.`, `::` and `/` all separate the parts (D-158).
pub(crate) fn names(s: &Sym, input: &str) -> bool {
    if s.name == input {
        return true;
    }
    let Some(rest) = input.strip_suffix(&s.name) else { return false };
    let Some(prefix) = rest.strip_suffix("::").or_else(|| rest.strip_suffix('.')) else { return false };
    [prefix.replace('.', "/"), prefix.replace('.', "::")]
        .iter()
        .any(|m| s.module == *m || s.module.ends_with(&format!("/{m}")) || s.module.ends_with(&format!("::{m}")))
}

/// An unknown-name hint for `resolve`: the symbols whose name contains `input`'s
/// leaf, the part after its last `.` or `::` (`app.mian` suggests by `mian`);
/// empty when none are close.
pub(crate) fn similar_hint(ix: &Index, input: &str) -> String {
    let leaf = input.rsplit(['.', ':']).next().unwrap_or(input).to_lowercase();
    if leaf.is_empty() {
        return String::new();
    }
    let similar: Vec<String> = ix.syms.iter().filter(|s| s.name.to_lowercase().contains(&leaf)).take(10).map(|s| s.qname()).collect();
    if similar.is_empty() { String::new() } else { format!("\nSimilar names: {}", similar.join(", ")) }
}

/// Whether `m` holds tests: Gleam's `test/` or a Rust crate's `tests/` (D-169).
pub(crate) fn is_test(m: &ModuleRow) -> bool {
    m.dir == "test" || m.dir == "tests"
}

/// A module's own doc line: Gleam's `////` or Rust's `//!` (D-158).
pub(crate) fn module_doc(m: &ModuleRow) -> &str {
    if let Some(l) = m.header.lines().find(|l| l.starts_with("////")) {
        return l.trim_start_matches('/').trim();
    }
    m.header.lines().find_map(|l| l.trim_start().strip_prefix("//!")).map_or("", |l| l.trim())
}

/// A module's symbol names, each once: in a Rust store an `impl Store` shares
/// its type's name, and one name is listed once (D-158).
pub(crate) fn module_names<'a>(ix: &'a Index, module: &str) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    ix.syms.iter().filter(|s| s.module == module).map(|s| s.name.as_str()).filter(|n| seen.insert(*n)).collect()
}

/// A module's text for `view <module> --bodies`: a Rust module is its file
/// exactly (D-158), a Gleam one its header and its items.
pub(crate) fn module_text(ix: &Index, m: &ModuleRow) -> String {
    if m.file.is_some() {
        return projection(ix, m).0;
    }
    let mut body = format!("// ── module {} ({}) ──\n", m.path, m.dir);
    if !m.header.is_empty() {
        body += &format!("{}\n\n", m.header);
    }
    for s in ix.syms.iter().filter(|s| s.module == m.path) {
        body += &format!("{}\n\n", s.text);
    }
    body
}

/// A module's outline: its heading and items, a Rust module's nested under its
/// parents with each item's signature and its `// BROKEN` mark (D-158).
pub(crate) fn module_outline(ix: &Index, m: &ModuleRow, broken: &HashMap<String, String>, bodies: bool) -> String {
    let mut text = format!("// ── module {} ({}), outline: `view <name>` for a body, `--bodies` for all ──\n", m.path, m.dir);
    if bodies {
        text += &format!("// bodies left out to fit the size limit; `view {} --bodies` alone shows them\n", m.path);
    }
    if m.file.is_some() {
        let doc: Vec<&str> = m.header.lines().filter(|l| l.trim_start().starts_with("//!")).collect();
        if !doc.is_empty() {
            text += &doc.join("\n");
            text.push('\n');
            text.push('\n');
        }
        walk_by_parent(ix, &by_parent(ix, &m.path), None, 0, broken, &mut text);
        return text;
    }
    let doc: Vec<&str> = m.header.lines().filter(|l| l.starts_with("////")).collect();
    if !doc.is_empty() {
        text += &format!("{}\n\n", doc.join("\n"));
    }
    for s in ix.syms.iter().filter(|s| s.module == m.path) {
        text += &format!("{}{}\n\n", ix.signature(s), if broken.contains_key(&s.qname()) { "  // BROKEN" } else { "" });
    }
    text
}

/// Each item under its parent, indented by its depth.
fn walk_by_parent<'a>(
    ix: &Index,
    groups: &HashMap<Option<&'a str>, Vec<&'a Sym>>,
    parent: Option<&'a str>,
    depth: usize,
    broken: &HashMap<String, String>,
    out: &mut String,
) {
    for s in groups.get(&parent).map(Vec::as_slice).unwrap_or(&[]) {
        out.push_str(&"  ".repeat(depth));
        out.push_str(&ix.signature(s));
        if broken.contains_key(&s.qname()) {
            out.push_str("  // BROKEN");
        }
        out.push_str("\n\n");
        walk_by_parent(ix, groups, Some(&s.id), depth + 1, broken, out);
    }
}

/// D-158 point 2's brief lines: a Rust project's session start carries them,
/// and no other project's does.
pub(crate) const RUST_BRIEF: &str = "Rust in Solace: every item has its own name, once per module; put platform differences inside the body (`#[cfg(unix)] { … }` or `if cfg!(unix)`). At module level a macro only defines (`macro_rules!`) or is `thread_local!`: no `lazy_static!`, `bitflags!` or other item-making calls (use `LazyLock`, `OnceLock`, consts). No `include!`; use `include_str!` or `include_bytes!` for data. `#[path]` names a file beside its parent. Every `unsafe` has a `// SAFETY:` comment. `build.rs`, `.cargo/config[.toml]`, `rust-toolchain[.toml]`, Cargo.toml's dependency, `[workspace]` and build keys, and every crate below the root's `Cargo.toml` and `build.rs` are settings: `write` them with `reason`.\n";

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "store_submit_tests.rs"]
mod submit_tests;

#[cfg(test)]
#[path = "store_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "store_delete_tests.rs"]
mod delete_tests;

/// A Rust syntax failure `e` located in the block `label`: its line in the
/// submitted text (the wrapper's `offset` lines are removed) and the container
/// the item was to be added to, so a wrong placement is visible. A line from
/// rustfmt's output is approximate once rustfmt has reflowed the text.
pub(crate) fn rust_syntax_error(
    e: Failure,
    label: &str,
    parent: Option<(&str, &str, &str)>,
    module: &str,
    offset: usize,
    lines: usize,
) -> Failure {
    let (line, what) = if let Some(rest) = e.0.strip_prefix("Not saved: E0100 syntax error at line ") {
        let (line, what) = rest.split_once(':').unwrap_or((rest, ""));
        (line.trim().parse::<usize>().ok(), what.trim().to_string())
    } else if let Some(rest) = e.0.strip_prefix("line ") {
        let (line, what) = rest.split_once(':').unwrap_or((rest, ""));
        let line = line.trim().parse::<usize>().ok().map(|line| line.saturating_sub(offset).max(1));
        (line, what.trim().to_string())
    } else if e.0.starts_with("Not saved:") {
        return e;
    } else {
        return Failure(format!("Not saved: {}", e.0), e.1);
    };
    let mut at = format!("Not saved: E0100 syntax error in `{label}`");
    if let Some(line) = line {
        at += &format!(" at line {}", line.min(lines.max(1)));
    }
    if let Some((name, _, kind)) = parent {
        at += &format!(", added inside `{module}.{name}` ({kind})");
    }
    let what = if what.is_empty() { "the text doesn't parse".to_string() } else { what };
    Failure(format!("{at}: {what}"), e.1)
}

/// D-158 point 2's brief lines, with real names from this project's store
/// (task:5bc1e9311b667f05): a method and an inline test when the store has them.
pub(crate) fn rust_brief(ix: &Index) -> String {
    let mut out = RUST_BRIEF.to_string();
    let by_id: HashMap<&str, usize> = ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    let in_impl = |i: usize| ix.syms[i].parent.as_deref().and_then(|id| by_id.get(id)).is_some_and(|&p| ix.syms[p].kind_text == "impl");
    let method = (0..ix.syms.len()).find(|&i| ix.syms[i].kind_text == "fn" && !ix.test[i] && in_impl(i));
    let method = method.or_else(|| (0..ix.syms.len()).find(|&i| ix.syms[i].kind_text == "fn" && in_impl(i)));
    if let Some(i) = method {
        out += &format!("A method is `<module>.<Type>::<fn>`, e.g. `{}`; the module is the one holding the `impl`.\n", ix.syms[i].qname());
    }
    let inline_tests = |i: usize| -> Option<usize> {
        let mut parent = ix.syms[i].parent.as_deref();
        while let Some(id) = parent {
            let &p = by_id.get(id)?;
            let s = &ix.syms[p];
            if s.kind_text == "mod" && s.text.contains('{') && s.name.rsplit("::").next() == Some("tests") {
                return Some(p);
            }
            parent = s.parent.as_deref();
        }
        None
    };
    let test = (0..ix.syms.len()).find(|&i| ix.syms[i].kind_text == "fn" && crate::reach::is_test_fn(ix, i) && inline_tests(i).is_some());
    if let Some(i) = test {
        let t = &ix.syms[i];
        let m = &ix.syms[inline_tests(i).unwrap()];
        out += &format!(
            concat!(
                "A test in an inline `mod tests` is `<module>.tests::<name>`, or ",
                "`<module>.outer::tests::<name>` when the `tests` module is nested, e.g. `{}`; ",
                "add one with `*** Add: {}::{}.<test>`.\n",
            ),
            t.qname(),
            m.module,
            m.name
        );
    }
    out += "`find <text>` searches Rust source; `grep` searches other files only.\n";
    out
}
