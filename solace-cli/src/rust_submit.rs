//! `submit` in a Rust store (D-168): an item is saved by name, an item inside
//! a container is wrapped in that container to be parsed, a whole `impl`,
//! `trait`, inline `mod` or `extern` block with its items is saved by `--new`,
//! a module the store doesn't have is created by `--new`, a whole module is
//! replaced by `submit <module>`, and every submit is checked with cargo before
//! it is saved. `delete` (D-173) retires named items and the module they left
//! empty.
use crate::*;

/// Where a Rust `submit` block lands (D-168): the module, the symbol the text
/// replaces, the container an added item sits in, and the module it creates
/// when the store doesn't have it.
pub(crate) struct RustTarget {
    module: String,
    existing: Option<usize>,
    parent: Option<usize>,
    create: Option<RustNew>,
}

/// A module the store doesn't have yet (D-168): its file from the project root,
/// its `dir`, the `mod <name>;` line its parent's header takes (`None` for a
/// test crate root, which `integration::x` names and no line declares), and the
/// header of the text that made it.
pub(crate) struct RustNew {
    pub(crate) module: String,
    pub(crate) file: String,
    pub(crate) dir: String,
    pub(crate) line: Option<(String, String)>,
    pub(crate) header: String,
}

/// The `// ── <name> ──` blocks of a plain `submit`, each name and text
/// (D-168): `name` and `new_in` name one target, so unless the text holds more
/// than one header it is one block under that name.
fn submit_blocks(name: Option<&str>, new_in: Option<&str>, raw: &str) -> Result<Vec<(Option<String>, String)>, Failure> {
    let is_header = |l: &str| l.starts_with(crate::drafts::BLOCK_HEADER);
    if raw.lines().filter(|l| is_header(l)).count() <= 1 && (name.is_some() || new_in.is_some()) {
        return Ok(vec![(name.map(str::to_string), raw.lines().filter(|l| !is_header(l)).collect::<Vec<_>>().join("\n"))]);
    }
    let mut blocks: Vec<(Option<String>, String)> = vec![];
    for line in raw.lines() {
        if let Some(h) = line.strip_prefix(crate::drafts::BLOCK_HEADER) {
            if h.starts_with("module ") {
                return fail("E0101 a whole module goes through `submit <module>`.");
            }
            blocks.push((Some(block_name(h)), String::new()));
        } else if let Some((_, text)) = blocks.last_mut() {
            text.push_str(line);
            text.push('\n');
        } else if !line.trim().is_empty() {
            return fail(format!(
                "E0101 text for `submit` with no name must be blocks, each headed `{}<module>.<name> ──`.",
                crate::drafts::BLOCK_HEADER
            ));
        }
    }
    Ok(blocks)
}

/// Whether a Rust kind names a container Solace holds the items of (D-168).
pub(crate) fn container_kind(kind: &str) -> bool {
    matches!(kind, "impl" | "trait" | "mod" | "extern")
}

/// An item and every item nested in it, so a block's items clash and count like
/// top-level ones (D-168).
fn nested(item: &store::Submitted) -> Vec<&store::Submitted> {
    std::iter::once(item).chain(item.children.iter().flat_map(nested)).collect()
}

/// The namespace bits a Rust item's name lives in (D-168): a type, a value, or
/// a macro. A type and a value of one name don't clash; two of either do.
pub(crate) fn rust_namespace(kind: &str) -> u8 {
    match kind {
        "struct" | "enum" | "union" | "type" | "trait" | "mod" => TYPE_NAMESPACE,
        "fn" | "const" | "static" => VALUE_NAMESPACE,
        "macro_rules!" | "macro" => MACRO_NAMESPACE,
        _ => 0,
    }
}

/// Whether `text` is one Rust identifier name (D-168): letters, digits and `_`,
/// not starting with a digit, and no keyword — no module can take one.
fn identifier(text: &str) -> bool {
    if reserved(text) {
        return false;
    }
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Whether `text` is a Rust keyword, a reserved word or `_` (D-168): `syn`
/// reads none of them as a module's name, so `mod type;` doesn't parse and no
/// new module may be named one.
fn reserved(text: &str) -> bool {
    const WORDS: &str = concat!(
        "_ abstract as async await become box break const continue crate do dyn else enum extern ",
        "false final fn for gen if impl in let loop macro match mod move mut override priv pub ref ",
        "return self Self static struct super trait true try type typeof unsafe unsized use virtual ",
        "where while yield"
    );
    WORDS.split_whitespace().any(|w| w == text)
}

/// Whether a header line declares a module — `mod a;`, `pub mod a;` or
/// `#[path = "a.rs"] mod a;` (D-168): the new `mod <name>;` goes after the last
/// one.
fn is_mod_line(line: &str) -> bool {
    let line = line.trim();
    !line.starts_with("//") && line.ends_with(';') && line.split_whitespace().any(|w| w == "mod")
}

/// A module header with `mod <name>;` after its last `mod` line, or at its end
/// when it has none (D-168), keeping what followed its last line.
pub(crate) fn add_mod(header: &str, name: &str) -> String {
    let body = header.trim_end();
    let end = &header[body.len()..];
    let end = if end.is_empty() { "\n" } else { end.strip_prefix('\n').unwrap_or(end) };
    let lines: Vec<&str> = body.lines().collect();
    let at = lines.iter().rposition(|l| is_mod_line(l)).map_or(lines.len(), |i| i + 1);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == at {
            out += &format!("mod {name};\n");
        }
        out += line;
        out += "\n";
    }
    if at == lines.len() {
        out += &format!("mod {name};\n");
    }
    out += end;
    out
}

/// The module a `mod` declaration names (D-173): the last word of `mod x;`,
/// `pub mod x;` or `#[path = "x.rs"] mod x;`.
fn mod_name(line: &str) -> Option<&str> {
    line.trim().strip_suffix(';')?.split_whitespace().next_back()
}

/// A header without its declaration of `name` (D-173), the inverse of
/// `add_mod`: the `mod <name>;` line, its attributes and its newline go.
fn remove_mod(header: &str, name: &str) -> String {
    let lines: Vec<&str> = header.split_inclusive('\n').collect();
    let Some(i) = lines.iter().position(|line| is_mod_line(line) && mod_name(line) == Some(name)) else {
        return header.to_string();
    };
    let start = (0..i).rev().take_while(|&j| lines[j].trim_start().starts_with("#[")).last().unwrap_or(i);
    let rest = format!("{}{}", lines[..start].concat(), lines[i + 1..].concat());
    if rest.trim().is_empty() { String::new() } else { rest }
}

/// A test crate root's path: `integration::x` with no part after it (D-169).
fn is_test_crate_root(module: &str) -> bool {
    module.strip_prefix("integration::").is_some_and(|rest| !rest.contains("::"))
}

/// A `path = "..."` value on an attribute line, as `#[path = "h.rs"]` or
/// `#[cfg_attr(unix, path = "os/h.rs")]` (D-173).
fn path_attr(line: &str) -> Option<&str> {
    let attr = line.trim_start().strip_prefix("#[")?;
    let at = attr.find("path")?;
    attr[at + 4..].trim_start().strip_prefix('=')?.trim_start().strip_prefix('"')?.split('"').next()
}

/// `path` with its `.` and `..` parts resolved lexically, so a `#[path]` that
/// climbs compares equal to the file it names. A leading `..`, which leaves the
/// project, stays, so it can't match a file inside it (D-173).
fn normalized_path(path: &str) -> String {
    let mut parts: Vec<&str> = vec![];
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Whether module `m` declares `own` in its header at the file `row` names, as
/// `tests/x.rs` declares `tests/h.rs` with `mod h;` or `#[path = "other.rs"]`
/// `mod h;` (D-173).
fn declares_module(m: &ModuleRow, own: &str, row: &ModuleRow) -> bool {
    let Some(file) = m.file.as_deref() else { return false };
    let Some(target) = row.file.as_deref() else { return false };
    let owns = m.owns_dir.unwrap_or_else(|| language::rust::owns_dir(&m.path, file));
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    let lines: Vec<&str> = m.header.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !(is_mod_line(line) && mod_name(line) == Some(own)) {
            continue;
        }
        let path =
            path_attr(line).or_else(|| lines[..i].iter().rev().take_while(|l| l.trim_start().starts_with("#[")).find_map(|l| path_attr(l)));
        if let Some(p) = path {
            let declared = if dir.is_empty() { p.to_string() } else { format!("{dir}/{p}") };
            return normalized_path(&declared) == normalized_path(target);
        }
        let flat = language::rust::child_file(file, owns, own);
        let nested = format!("{}/mod.rs", flat.trim_end_matches(".rs"));
        return target == flat.as_str() || target == nested.as_str();
    }
    false
}

/// A module's declaration in its parent (D-173): the name its parent's `mod`
/// line holds, and the parent's path (`child` → `child` in `crate`,
/// `crate::a::b` → `b` in `crate::a`).
fn mod_declaration(module: &str) -> (&str, &str) {
    module.rsplit_once("::").map_or((module, "crate"), |(parent, own)| (own, parent))
}

/// A module's parent path (`crate::a::b` → `crate::a`); empty for a top name.
fn parent_of(module: &str) -> &str {
    module.rsplit_once("::").map_or("", |(parent, _)| parent)
}

/// A symbol and every item nested in it, in the store's order (D-173).
pub(crate) fn rust_descendants(ix: &Index, i: usize) -> Vec<usize> {
    let mut out = vec![i];
    let mut ids = vec![ix.syms[i].id.clone()];
    let mut at = 0;
    while at < ids.len() {
        for (j, s) in ix.syms.iter().enumerate() {
            if s.parent.as_deref() == Some(ids[at].as_str()) {
                ids.push(s.id.clone());
                out.push(j);
            }
        }
        at += 1;
    }
    out.sort_unstable();
    unique(&mut out);
    out
}

/// Each level's last row and its separator (D-173): the row a Rust file's or an
/// inline container's ending hangs on, before a delete takes it.
fn rust_endings(ix: &Index) -> Vec<RustEnding> {
    let mut out: Vec<RustEnding> = vec![];
    let mut at: std::collections::HashMap<(String, Option<String>), usize> = std::collections::HashMap::new();
    for s in ix.syms.iter() {
        let key = (s.module.clone(), s.parent.clone());
        match at.get(&key).copied() {
            Some(i) => {
                out[i].id = s.id.clone();
                out[i].sep = s.sep.clone();
            }
            None => {
                at.insert(key, out.len());
                out.push(RustEnding { module: s.module.clone(), parent: s.parent.clone(), id: s.id.clone(), sep: s.sep.clone() });
            }
        }
    }
    out
}

/// The live symbols `input` names, as `resolve` reads a name (D-168): its own
/// name, or its name behind its module, the module exact or a suffix.
fn rust_named(ix: &Index, input: &str) -> Vec<usize> {
    store::matching(ix, input)
}

/// The one symbol `input` names, containers left out (D-168): `None` when
/// nothing matches, E1002 when more than one does.
pub(crate) fn rust_symbol(ix: &Index, input: &str) -> Result<Option<usize>, Failure> {
    let found: Vec<usize> = rust_named(ix, input).into_iter().filter(|&i| !container_kind(&ix.syms[i].kind_text)).collect();
    match found.as_slice() {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        many => {
            let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind_text)).collect();
            fail(format!("E1002 ambiguous name: {input} could be\n{}", list.join("\n")))
        }
    }
}

#[cfg(test)]
#[path = "rust_submit_tests.rs"]
mod tests;

/// The container `input` names (D-168): a live `impl`, `trait` or `mod`, the
/// first `impl` when several share the name. Several other containers with the
/// name are ambiguous, as they are in `rust_symbol`.
pub(crate) fn rust_container(ix: &Index, input: &str) -> Result<Option<usize>, Failure> {
    let found: Vec<usize> = rust_named(ix, input).into_iter().filter(|&i| container_kind(&ix.syms[i].kind_text)).collect();
    rust_one(&found, ix, input)
}

/// An item of a whole-module `submit`, paired with the row the store has for it
/// (D-168): the row's index, the text as submitted and its items when it is a
/// container. `stored` is `None` for an item the store doesn't have yet.
pub(crate) struct RustItem {
    pub(crate) stored: Option<usize>,
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) text: String,
    pub(crate) tail: String,
    pub(crate) sep: String,
    pub(crate) children: Vec<RustItem>,
}

impl RustItem {
    /// The item's whole text: its own text, its items with their separators and
    /// its tail — what `cargo::format_item` formats as one block (D-168).
    fn joined(&self) -> String {
        let mut out = self.text.clone();
        for child in &self.children {
            out += &child.joined();
            out += &child.sep;
        }
        out += &self.tail;
        out
    }
}

/// Pairs a whole module's submitted items with the rows the store has (D-168):
/// an item and a stored row go together when they share a name and a namespace
/// under the same parent, and items of one name (two `impl Store`) pair in
/// order. A stored row the text leaves out is refused (E0102); an unpaired item
/// is new.
pub(crate) fn rust_module_items(ix: &Index, module: &str, pieces: &[language::rust::Piece]) -> Result<Vec<RustItem>, Failure> {
    let roots: Vec<usize> = (0..ix.syms.len()).filter(|&i| ix.syms[i].module.as_str() == module && ix.syms[i].parent.is_none()).collect();
    rust_items_under(ix, pieces, &roots)
}

fn rust_items_under(ix: &Index, pieces: &[language::rust::Piece], stored: &[usize]) -> Result<Vec<RustItem>, Failure> {
    let mut used = vec![false; stored.len()];
    let mut out = vec![];
    for p in pieces {
        let namespace = rust_namespaces(p.kind, &p.text);
        let hit = (0..stored.len()).find(|&k| {
            !used[k]
                && ix.syms[stored[k]].name == p.name
                && rust_namespaces(&ix.syms[stored[k]].kind_text, &ix.syms[stored[k]].text) == namespace
        });
        if let Some(k) = hit {
            used[k] = true;
        }
        let kids: Vec<usize> = match hit {
            Some(k) => {
                let id = ix.syms[stored[k]].id.as_str();
                (0..ix.syms.len()).filter(|&i| ix.syms[i].parent.as_deref() == Some(id)).collect()
            }
            None => vec![],
        };
        let children = rust_items_under(ix, &p.children, &kids)?;
        let (text, tail, sep) = store::piece_parts(p);
        out.push(RustItem { stored: hit.map(|k| stored[k]), name: p.name.clone(), kind: p.kind, text, tail, sep, children });
    }
    let mut missing: Vec<&str> = stored.iter().enumerate().filter(|(k, _)| !used[*k]).map(|(_, &i)| ix.syms[i].name.as_str()).collect();
    if !missing.is_empty() {
        unique(&mut missing);
        return fail(format!(
            "E0102 the module text leaves out: {}. Nothing was saved. Removing a symbol must be explicit (solace delete <name>).",
            missing.join(", ")
        ));
    }
    Ok(out)
}

/// Takes the whole-block text `format_one` produced for a new container
/// (D-168): its items came out formatted with it.
fn rust_adopt(item: &mut RustItem, done: &store::Submitted) {
    item.text = done.text.clone();
    item.tail = done.tail.clone();
    for (child, taken) in item.children.iter_mut().zip(&done.children) {
        rust_adopt(child, taken);
    }
}

impl Store {
    /// The `submit` command in a Rust store (D-168): `--imports` replaces a
    /// module's header; `submit <module>` replaces the module whole.
    pub(crate) fn submit_rust(&self, target: Target, raw: &str) -> Out {
        match target {
            Target::Imports(m) => self.submit_imports_rust(m, raw),
            Target::Module(m) => self.submit_module_rust(m, raw, false),
        }
    }

    /// `submit <module>` in a Rust store (D-168): the text is the module's whole
    /// file, item by item. Every stored item must be in it, matched by name
    /// within its parent so its id stays; an item the text adds is saved as
    /// `--new` saves it; only the items whose text changed are formatted, so
    /// unchanged items keep their bytes (D-158 point 5), unless `format
    /// <module>` has already formatted the whole module (D-180); `ord`, `sep`
    /// and a container's `tail` follow the text. The `mod x;` list can't
    /// change, and a module the store doesn't have is created by `--new`.
    fn submit_module_rust(&self, m: &str, raw: &str, formatted: bool) -> Out {
        let ix = self.index()?;
        self.refuse_drift(&ix)?;
        let Some(row) = self.resolve_module(&ix, m) else {
            return self.submit_symbols_rust(None, Some(m), raw, false);
        };
        let module = row.path.clone();
        let file = language::rust::split(raw, &module)
            .map_err(|e| store::rust_syntax_error(e, &module, None, &module, 0, 0, raw.lines().count().max(1)))?;
        if !file.refusals.is_empty() {
            let lines: Vec<String> = file.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
            return fail(format!("Not saved: the Rust text is outside the subset Solace holds.\n{}", lines.join("\n")));
        }
        let old = self.header(&module)?;
        let before = store::module_text(&ix, row);
        if language::rust::split(&before, &module)?.mods != file.mods {
            return fail(format!("Not saved: the module's `mod` list can't change; {}.", store::MODULE_NOTE));
        }
        let mut items = rust_module_items(&ix, &module, &file.items)?;
        if !formatted {
            self.rust_format_module(&ix, &module, None, &mut items, false, file.header.matches('\n').count() + 1)?;
        }
        let end = &old[old.trim_end().len()..];
        let header = if file.header.trim().is_empty() {
            String::new()
        } else {
            format!("{}{}", file.header.trim_end(), if end.is_empty() { "\n\n" } else { end })
        };
        let max_width = cargo::max_width(&self.root);
        let out = self.transaction(&[], || {
            self.set_header(&module, &header)?;
            let (mut changed, mut added): (Vec<(String, String)>, Vec<(String, String)>) = (vec![], vec![]);
            self.rust_save_items(&module, None, &items, &ix, &mut changed, &mut added)?;
            let dropped = self.rust_prune_impls(&[module.clone()])?;
            let after_ix = self.index()?;
            let live: HashSet<&str> = after_ix.syms.iter().map(|s| s.id.as_str()).collect();
            changed.retain(|(id, _)| live.contains(id.as_str()));
            added.retain(|(id, _)| live.contains(id.as_str()));
            let after = after_ix.modules.iter().find(|r| r.path == module).map(|r| store::module_text(&after_ix, r));
            if after.as_deref() == Some(before.as_str()) && dropped.is_empty() {
                let mut out = format!("Unchanged: {module}. The text equals the stored symbols, so nothing was saved or checked.\n");
                if formatted {
                    out += &note_lines(&long_line_notes(&ix, &module, &items, max_width, true));
                }
                return Ok(out);
            }
            let before_broken = self.broken()?;
            let before_warnings = self.warning_baseline()?;
            let diags = self.check_all(&after_ix)?;
            let own = |d: &Diag| match &d.symbol {
                Some(s) => s.starts_with(&format!("{module}.")),
                None => d.module.as_deref() == Some(module.as_str()),
            };
            let errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !errors.is_empty() {
                return fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}\n{}",
                    crate::group_errors(&errors),
                    cargo::ROUND_NOTE
                ));
            }
            let list = |v: &[(String, String)]| {
                if v.is_empty() { "none".to_string() } else { v.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(", ") }
            };
            let mut out = if after.as_deref() == Some(before.as_str()) {
                String::new()
            } else {
                format!("Saved {module}.\nNote: {module}: changed {}; added {}\n", list(&changed), list(&added))
            };
            out += &dropped_impls_note(&dropped);
            out += &warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error && own(d)),
                diags.iter().filter(|d| !d.error && !own(d)),
            );
            out += &broken_report(&before_broken, &diags);
            out += &note_lines(&long_line_notes(&ix, &module, &items, max_width, formatted));
            Ok(out)
        })?;
        Ok(out)
    }

    /// Formats the items a whole-module `submit` changed (D-158 point 5): an
    /// item whose own text equals its stored row keeps its bytes, so the rest of
    /// the file isn't reformatted; a changed item is rustfmt's, inside its
    /// parent's opening when it is nested. `done`: this item's text came from
    /// formatting its parent whole. `line`: where the first item starts in the
    /// module.
    pub(crate) fn rust_format_module(
        &self,
        ix: &Index,
        module: &str,
        parent: Option<(&str, &str, &str)>,
        items: &mut [RustItem],
        done: bool,
        line: usize,
    ) -> Result<(), Failure> {
        let mut at = line;
        for item in items {
            let same = item.stored.is_some_and(|i| ix.syms[i].text == item.text);
            let mut whole = done;
            let child_line = at + item.text.matches('\n').count();
            let span = item.joined().matches('\n').count() + item.sep.matches('\n').count();
            if !done && !same {
                let text = if item.children.is_empty() { item.text.clone() } else { item.joined() };
                let local = text.lines().count().max(1);
                let module_line = at.saturating_sub(1);
                let wrapper_lines = parent.map_or(0, |(_, opening, _)| opening.lines().count());
                let one = store::format_one(&self.root, module, parent, &text).map_err(|e| {
                    store::rust_syntax_error(
                        e,
                        &format!("{module}.{}", item.name),
                        parent,
                        module,
                        module_line,
                        wrapper_lines,
                        module_line + local,
                    )
                })?;
                if item.stored.is_none() && !item.children.is_empty() && item.children.len() == one.children.len() {
                    // A new container arrived formatted whole, its items too;
                    // they take no second rustfmt run.
                    rust_adopt(item, &one);
                    whole = true;
                } else {
                    item.text = one.text;
                    item.tail = one.tail;
                }
            }
            let (name, opening) = (item.name.clone(), item.text.clone());
            let kind = item.kind;
            self.rust_format_module(ix, module, Some((name.as_str(), opening.as_str(), kind)), &mut item.children, whole, child_line)?;
            at += span;
        }
        Ok(())
    }

    /// Writes the items a whole-module `submit` holds (D-168), level by level: a
    /// stored row keeps its id and takes the text's `ord`, `sep`, `tail` and
    /// container text; an item the store doesn't have is added with a new id. A
    /// row whose text or kind changed records an edit; an added one a create.
    fn rust_save_items(
        &self,
        module: &str,
        parent: Option<&str>,
        items: &[RustItem],
        ix: &Index,
        changed: &mut Vec<(String, String)>,
        added: &mut Vec<(String, String)>,
    ) -> Result<(), Failure> {
        for (ord, item) in items.iter().enumerate() {
            let rev = revision(&item.text);
            match item.stored {
                Some(i) => {
                    let id = ix.syms[i].id.clone();
                    let edited = ix.syms[i].text != item.text || ix.syms[i].kind_text != item.kind;
                    self.db.execute(
                        "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, ord = ?5, revision = ?6, sep = ?7, tail = ?8 WHERE id = ?1",
                        params![id, item.name, item.kind, item.text, ord as i64, rev, item.sep, item.tail],
                    )?;
                    if edited {
                        self.history(&id, "edit", &item.name)?;
                        changed.push((id.clone(), item.name.clone()));
                    }
                    self.rust_save_items(module, Some(id.as_str()), &item.children, ix, changed, added)?;
                }
                None => {
                    let id = new_id(language::rust::id_kind(item.kind));
                    self.db.execute(
                        "INSERT INTO symbols(id, module, name, kind, text, ord, revision, parent, sep, tail) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![id, module, item.name, item.kind, item.text, ord as i64, rev, parent, item.sep, item.tail],
                    )?;
                    self.history(&id, "create", &item.name)?;
                    added.push((id.clone(), item.name.clone()));
                    self.rust_save_items(module, Some(id.as_str()), &item.children, ix, changed, added)?;
                }
            }
        }
        Ok(())
    }

    /// `submit` of items in a Rust store (D-168): each block is one Rust item,
    /// saved by name and checked with cargo; imports are cargo's, so none are
    /// merged and the whole module's own items are never rewritten. `formatted`:
    /// a patch's blocks are already rustfmt's text, so they are not formatted again.
    pub(crate) fn submit_symbols_rust(&self, name: Option<&str>, new_in: Option<&str>, raw: &str, formatted: bool) -> Out {
        let ix = self.index()?;
        self.refuse_drift(&ix)?;
        let blocks = submit_blocks(name, new_in, raw)?;
        if blocks.is_empty() {
            return fail("E0101 nothing to submit.");
        }
        let mut notes: Vec<String> = vec![];
        if let Some(name) = name.filter(|_| blocks.len() > 1) {
            notes.push(format!("`submit {name}` submitted every block; plain `submit` does the same"));
        }
        // (the module, the symbol replaced, its container, the items the text defines)
        let mut plan: Vec<(String, Option<usize>, Option<usize>, Vec<store::Submitted>)> = vec![];
        // The modules the store doesn't have yet, the first text that lands in
        // each giving it its header (D-168).
        let mut created: Vec<RustNew> = vec![];
        for (block_name, text) in &blocks {
            let mut target = match new_in {
                Some(m) => self.rust_new_target(&ix, m)?,
                None => self.rust_target(&ix, block_name.as_deref().unwrap_or_default())?,
            };
            let block = block_name.clone().unwrap_or_else(|| target.module.clone());
            if created.iter().any(|c| c.module == target.module) {
                target.create = None;
            }
            let items = match target.create.take() {
                Some(create) => {
                    let (header, items) = store::submit_module(&self.root, &target.module, text, formatted)?;
                    created.push(RustNew { header, ..create });
                    items
                }
                None => {
                    let container =
                        target.parent.map(|i| (ix.syms[i].name.as_str(), ix.syms[i].text.as_str(), ix.syms[i].kind_text.as_str()));
                    store::submit_items(&self.root, &target.module, &block, container, text, formatted)?
                }
            };
            if items.is_empty() || (new_in.is_none() && items.len() > 1) {
                return fail(format!(
                    "E0101 each block holds one symbol; {} defines {} ({}). To add several to one module, use `submit --new <module>`.",
                    block_name.as_deref().unwrap_or("the text"),
                    items.len(),
                    items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            // A named container takes no whole block: a new one goes through
            // `submit --new <module>` (D-168).
            if new_in.is_none()
                && target.existing.is_none()
                && target.parent.is_some()
                && let [one] = items.as_slice()
                && store::is_container(one)
            {
                return fail(format!("Not saved: {}.", store::block_note(one.kind, None)));
            }
            // A block's own text replaces nothing: a symbol's items change one
            // by one, and only a new block is saved whole (D-168).
            if let [one] = items.as_slice()
                && store::is_container(one)
                && target.existing.is_some()
            {
                return fail(format!("Not saved: {}.", store::block_note(one.kind, None)));
            }
            for item in &items {
                for item in nested(item) {
                    let clash = (0..ix.syms.len()).find(|&i| {
                        let s = &ix.syms[i];
                        Some(i) != target.existing
                            && Some(i) != target.parent
                            && s.module == target.module
                            && s.name == item.name
                            && rust_namespaces_overlap(&s.kind_text, &s.text, item.kind, &item.text)
                    });
                    if let Some(i) = clash {
                        return fail(format!("E1004 {} already exists; submit to it instead.", ix.syms[i].qname()));
                    }
                    if rust_namespaces(item.kind, &item.text) & TYPE_NAMESPACE != 0
                        && ix.modules.iter().any(|m| module_path_named(&target.module, &item.name, &m.path))
                    {
                        return fail(format!("E1004 {}.{} already exists; submit to it instead.", target.module, item.name));
                    }
                    let twice = plan.iter().any(|(m, .., items)| {
                        m == &target.module
                            && items.iter().any(|p| {
                                nested(p)
                                    .iter()
                                    .any(|c| c.name == item.name && rust_namespaces_overlap(c.kind, &c.text, item.kind, &item.text))
                            })
                    });
                    if twice {
                        return fail(format!("E0103 the text defines {}.{} twice.", target.module, item.name));
                    }
                }
            }
            plan.push((target.module.clone(), target.existing, target.parent, items));
        }
        // A file already there outside the store is not this submit's to own (D-168).
        let files: Vec<_> = created.iter().map(|c| self.root.join(&c.file)).collect();
        if files.iter().any(|p| p.exists()) {
            return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
        }
        let max_width = cargo::max_width(&self.root);
        let out = self.transaction(&files, || {
            for c in &created {
                let owns = language::rust::owns_dir(&c.module, &c.file);
                self.db.execute(
                    "INSERT INTO modules(path, dir, header, file, owns_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![c.module, c.dir, c.header, c.file, owns],
                )?;
                match &c.line {
                    Some((parent, own)) => {
                        let header = self.header(parent)?;
                        self.set_header(parent, &add_mod(&header, own))?;
                        notes.push(format!("created module {} ({}; added mod {own}; to {parent})", c.module, c.file));
                    }
                    None => notes.push(format!("created module {} ({})", c.module, c.file)),
                }
            }
            let (mut saved, mut unchanged): (Vec<String>, Vec<String>) = (vec![], vec![]);
            let mut changed = false;
            for (module, existing, parent, items) in &plan {
                for item in items {
                    let qnames: Vec<String> = nested(item).iter().map(|i| format!("{module}.{}", i.name)).collect();
                    match existing {
                        Some(i) if ix.syms[*i].text == item.text && ix.syms[*i].name == item.name && ix.syms[*i].kind_text == item.kind => {
                            unchanged.push(qnames[0].clone());
                            continue;
                        }
                        Some(i) => {
                            let e = &ix.syms[*i];
                            self.db.execute(
                                "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, revision = ?5 WHERE id = ?1",
                                params![e.id, item.name, item.kind, item.text, revision(&item.text)],
                            )?;
                            self.history(&e.id, "edit", &item.name)?;
                        }
                        None => match parent {
                            // An empty container takes the re-split text and tail,
                            // and the item lands inside it (D-168); any other
                            // container takes the item as a new nested block.
                            Some(p) if store::is_container(item) && ix.syms[*p].name == item.name => {
                                let container = &ix.syms[*p];
                                self.rust_fill_container(container, module, item)?;
                                self.history(&container.id, "edit", &item.name)?;
                            }
                            _ => {
                                let pid = parent.as_ref().map(|i| ix.syms[*i].id.as_str());
                                self.rust_insert(module, item, pid)?;
                            }
                        },
                    }
                    saved.extend(qnames);
                    changed = true;
                }
            }
            if !changed {
                let broken = self.broken()?;
                let still: Vec<&str> = unchanged.iter().map(String::as_str).filter(|u| broken.contains_key(*u)).collect();
                let still =
                    if still.is_empty() { String::new() } else { format!("Still broken: {} (`check` shows why).\n", still.join(", ")) };
                let mut out = format!(
                    "Unchanged: {}. The text equals the stored symbols, so nothing was saved or checked.\n{still}",
                    unchanged.join(", ")
                );
                let mut long = vec![];
                for (module, .., items) in &plan {
                    for item in items.iter().flat_map(nested) {
                        let name = format!("{module}.{}", item.name);
                        if let Some(note) = long_line_note(&name, &item.text, max_width) {
                            long.push(note);
                        }
                    }
                }
                out += &note_lines(&long);
                return Ok(out);
            }
            let mut touched: Vec<String> = plan.iter().map(|(module, ..)| module.clone()).collect();
            unique(&mut touched);
            let dropped = self.rust_prune_impls(&touched)?;
            for (qname, _) in &dropped {
                let submitted_impl =
                    plan.iter().any(|(m, .., items)| items.iter().any(|i| i.kind == "impl" && format!("{m}.{}", i.name) == *qname));
                if submitted_impl {
                    for list in [&mut saved, &mut unchanged] {
                        if let Some(p) = list.iter().position(|s| s == qname) {
                            list.remove(p);
                        }
                    }
                }
            }
            let mut modules: HashSet<&str> = plan.iter().map(|p| p.0.as_str()).collect();
            // A new module's `mod` line lands in its parent's header, so an error
            // cargo reports against the parent is this submit's too (D-168).
            modules.extend(created.iter().filter_map(|c| c.line.as_ref().map(|(parent, _)| parent.as_str())));
            let own = |d: &Diag| match &d.symbol {
                Some(s) => saved.contains(s),
                None => d.module.as_deref().is_some_and(|m| modules.contains(m)),
            };
            let before_broken = self.broken()?;
            let before_warnings = self.warning_baseline()?;
            let ix = self.index()?;
            let diags = self.check_all(&ix)?;
            let errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !errors.is_empty() {
                return fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}\n{}",
                    crate::group_errors(&errors),
                    cargo::ROUND_NOTE
                ));
            }
            let mut reported = saved.clone();
            unique(&mut reported);
            let mut out = String::new();
            if !reported.is_empty() {
                out += &format!("Saved {}.\n", reported.join(", "));
            }
            if !unchanged.is_empty() {
                out += &format!("Unchanged (the same as stored): {}.\n", unchanged.join(", "));
            }
            out += &dropped_impls_note(&dropped);
            out += &warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error && own(d)),
                diags.iter().filter(|d| !d.error && !own(d)),
            );
            out += &broken_report(&before_broken, &diags);
            let mut long = vec![];
            for (module, .., items) in &plan {
                for item in items.iter().flat_map(nested) {
                    let name = format!("{module}.{}", item.name);
                    if let Some(note) = long_line_note(&name, &item.text, max_width) {
                        long.push(note);
                    }
                }
            }
            out += &note_lines(&long);
            Ok(out)
        })?;
        Ok(note_lines(&notes) + &out)
    }

    /// The container to add an item to, named in `n` (D-168): the whole name,
    /// or a cut whose prefix names a container itself; a module prefix is not a
    /// container. The first `impl` when several share the name; several other
    /// containers with it are ambiguous.
    pub(crate) fn rust_container_in(&self, ix: &Index, n: &str) -> Result<Option<usize>, Failure> {
        if let Some(i) = rust_container(ix, n)? {
            return Ok(Some(i));
        }
        let mut cuts: Vec<usize> = n.match_indices("::").map(|(i, _)| i).chain(n.match_indices('.').map(|(i, _)| i)).collect();
        cuts.sort_unstable();
        for &at in cuts.iter().rev() {
            let prefix = &n[..at];
            if self.resolve_module(ix, prefix).is_some() {
                continue;
            }
            if let Some(i) = rust_container(ix, prefix)? {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// Where a named block lands (D-168): the symbol it replaces (containers are
    /// left out of the lookup), a container it adds to, or a module it adds
    /// items to. A name that is none of those is a module the store doesn't have.
    pub(crate) fn rust_target(&self, ix: &Index, n: &str) -> Result<RustTarget, Failure> {
        if let Some(i) = rust_symbol(ix, n)? {
            let parent = ix.syms[i].parent.as_ref().and_then(|p| ix.syms.iter().position(|s| &s.id == p));
            return Ok(RustTarget { module: ix.syms[i].module.clone(), existing: Some(i), parent, create: None });
        }
        if let Some(m) = self.resolve_module(ix, n) {
            return Ok(RustTarget { module: m.path.clone(), existing: None, parent: None, create: None });
        }
        if let Some(i) = self.rust_container_in(ix, n)? {
            return Ok(RustTarget { module: ix.syms[i].module.clone(), existing: None, parent: Some(i), create: None });
        }
        if let Some(m) = n.rsplit_once('.').and_then(|(m, _)| self.resolve_module(ix, m)) {
            return Ok(RustTarget { module: m.path.clone(), existing: None, parent: None, create: None });
        }
        if let Some(create) = n.rsplit_once('.').and_then(|(m, _)| self.rust_created(ix, m)) {
            return Ok(RustTarget { module: create.module.clone(), existing: None, parent: None, create: Some(create) });
        }
        let m = n.rsplit_once('.').map_or(n, |(m, _)| m);
        fail(format!("E1003 unknown module: {m}"))
    }

    /// Where `submit --new <m>` adds items (D-168): a module the store has, an
    /// `impl`, `trait`, `mod` or `extern` container it has, or a module the name
    /// creates.
    fn rust_new_target(&self, ix: &Index, m: &str) -> Result<RustTarget, Failure> {
        if let Some(module) = self.resolve_module(ix, m) {
            return Ok(RustTarget { module: module.path.clone(), existing: None, parent: None, create: None });
        }
        if let Some(i) = self.rust_container_in(ix, m)? {
            return Ok(RustTarget { module: ix.syms[i].module.clone(), existing: None, parent: Some(i), create: None });
        }
        match self.rust_created(ix, m) {
            Some(create) => Ok(RustTarget { module: create.module.clone(), existing: None, parent: None, create: Some(create) }),
            None => fail(format!("E1003 unknown module: {m}")),
        }
    }

    /// The module a name the store doesn't have creates (D-168): `cache` beside
    /// the crate root, `a::b::c` beside `a::b`, and `integration::x` a new test
    /// crate root (`tests/x.rs`, D-169). `None` when a part is no identifier or
    /// names a keyword, when the parent module isn't there, or when the name it
    /// would land under is a module the store already has.
    pub(crate) fn rust_created(&self, ix: &Index, name: &str) -> Option<RustNew> {
        let parts: Vec<&str> = name.split("::").collect();
        if !parts.iter().all(|p| identifier(p)) {
            return None;
        }
        let (own, parents) = parts.split_last()?;
        let parent = parents.join("::");
        let created = if parent == "integration" && self.resolve_module(ix, &parent).is_none() {
            RustNew { module: name.to_string(), file: format!("tests/{own}.rs"), dir: "tests".into(), line: None, header: String::new() }
        } else {
            let p = self.resolve_module(ix, if parent.is_empty() { "crate" } else { &parent })?;
            let above = p.file.clone()?;
            let dir = above.split_once('/')?.0.to_string();
            let owns = p.owns_dir.unwrap_or_else(|| language::rust::owns_dir(&p.path, &above));
            let module = language::rust::child_module(&p.path, &above, owns, own);
            let file = language::rust::child_file(&above, owns, own);
            RustNew { module, file, dir, line: Some((p.path.clone(), own.to_string())), header: String::new() }
        };
        // A child of a test crate root takes the `integration::` prefix every
        // `tests/*.rs` file has (D-169), so the name it lands under can differ
        // from the one asked for; a module the store already has under it isn't
        // this submit's to make.
        if self.resolve_module(ix, &created.module).is_some() {
            return None;
        }
        Some(created)
    }

    /// Adds a Rust item (D-168): after every row of its module, `ord`-wise, and
    /// it takes the separator of the last item in its group, which becomes a
    /// blank line, so the file keeps its blank lines and its ending. A block's
    /// items land under it, and its tail holds its closing brace; every row it
    /// adds is recorded as created.
    fn rust_insert(&self, module: &str, item: &store::Submitted, parent: Option<&str>) -> Result<(), Failure> {
        let ord: i64 =
            self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", params![module], |r| r.get(0))?;
        let last: Option<(String, Option<String>)> = self
            .db
            .prepare("SELECT id, sep FROM symbols WHERE live = 1 AND module = ?1 AND parent IS ?2 ORDER BY ord DESC LIMIT 1")?
            .query_map(params![module, parent], |r| Ok((r.get(0)?, r.get(1)?)))?
            .next()
            .transpose()?;
        let sep = match &last {
            Some((id, sep)) => {
                self.db.execute("UPDATE symbols SET sep = ?2 WHERE id = ?1", params![id, "\n\n"])?;
                sep.clone().unwrap_or_else(|| "\n\n".into())
            }
            // A module's first item ends its file; a container's takes its
            // newline from the container's own tail.
            None if parent.is_none() => "\n".to_string(),
            None => String::new(),
        };
        let rev = revision(&item.text);
        let id = new_id(language::rust::id_kind(item.kind));
        self.db.execute(
            "INSERT INTO symbols(id, module, name, kind, text, ord, revision, parent, sep, tail) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id, module, item.name, item.kind, item.text, ord, rev, parent, sep, item.tail],
        )?;
        self.history(&id, "create", &item.name)?;
        for child in &item.children {
            self.rust_insert(module, child, Some(&id))?;
        }
        Ok(())
    }

    /// An empty container's first item (D-168): its row takes the re-split
    /// opening and closing text, and every item of the block lands inside it.
    fn rust_fill_container(&self, container: &Sym, module: &str, item: &store::Submitted) -> Result<(), Failure> {
        let rev = revision(&item.text);
        self.db.execute(
            "UPDATE symbols SET text = ?2, revision = ?3, tail = ?4 WHERE id = ?1",
            params![container.id, item.text, rev, item.tail],
        )?;
        for child in &item.children {
            self.rust_insert(module, child, Some(&container.id))?;
        }
        Ok(())
    }

    /// `submit --imports <module>` in a Rust store (D-168): the header changes
    /// only when the text is imports and the module comment alone — no refusals,
    /// no items — and keeps the same `mod x;` list, since changing it adds or
    /// removes a module. Then cargo checks the module.
    fn submit_imports_rust(&self, m: &str, raw: &str) -> Out {
        let ix = self.index()?;
        self.refuse_drift(&ix)?;
        let Some(module) = self.resolve_module(&ix, m).map(|r| r.path.clone()) else {
            return fail(format!("E1003 unknown module: {m}"));
        };
        let old = self.header(&module)?;
        let new = language::rust::split(raw, &module)?;
        if !new.refusals.is_empty() {
            let lines: Vec<String> = new.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
            return fail(lines.join("\n"));
        }
        if language::rust::split(&old, &module)?.mods != new.mods {
            return fail(format!("Not saved: `--imports` can't add or remove a module; {}.", store::MODULE_NOTE));
        }
        if !new.items.is_empty() {
            return fail("E0101 --imports text may only contain imports and the module comment.");
        }
        let end = &old[old.trim_end().len()..];
        let header = if new.header.trim().is_empty() {
            String::new()
        } else {
            format!("{}{}", new.header.trim_end(), if end.is_empty() { "\n\n" } else { end })
        };
        let saved = format!("{module} imports");
        let out = self.transaction(&[], || {
            self.set_header(&module, &header)?;
            let ix = self.index()?;
            let before_broken = self.broken()?;
            let before_warnings = self.warning_baseline()?;
            let diags = self.check_all(&ix)?;
            // An error on one of the module's items is this submit's too, unless it
            // was already broken: removing a `use` an item needs must not save (D-168).
            let own = |d: &Diag| match &d.symbol {
                Some(s) => s.starts_with(&format!("{module}.")) && !before_broken.contains_key(s),
                None => d.module.as_deref() == Some(module.as_str()),
            };
            let errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !errors.is_empty() {
                let text: Vec<&str> = errors.iter().map(|d| d.text.as_str()).collect();
                return fail(format!("Not saved: the submitted text has errors.\n\n{}\n{}", text.join("\n\n"), cargo::ROUND_NOTE));
            }
            let mut out = format!("Saved {saved}.\n");
            out += &warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error && own(d)),
                diags.iter().filter(|d| !d.error && !own(d)),
            );
            out += &broken_report(&before_broken, &diags);
            Ok(out)
        })?;
        Ok(out)
    }

    /// The stored, formatted text of one added item (D-168) for the block
    /// `module.name`, so `drafts::patch_blocks` pushes a block the submit that
    /// follows doesn't format again. The item as written when its landing can't
    /// be read here; the submit that follows refuses it.
    pub(crate) fn rust_block_text(&self, ix: &Index, module: &str, name: &str, item: &str) -> String {
        let Ok(target) = self.rust_target(ix, &format!("{module}.{name}")) else { return item.to_string() };
        let container = target.parent.map(|i| (ix.syms[i].name.as_str(), ix.syms[i].text.as_str(), ix.syms[i].kind_text.as_str()));
        let block = format!("{module}.{name}");
        match store::submit_items(&self.root, &target.module, &block, container, item, false) {
            Ok(items) => match items.as_slice() {
                [one] if store::is_container(one) => one.children.first().map_or(item.to_string(), |c| c.text.clone()),
                [one] => one.text.clone(),
                _ => item.to_string(),
            },
            Err(_) => item.to_string(),
        }
    }

    /// `delete <name>...` in a Rust store (D-173): each named item goes with the
    /// items it holds, a module this empties loses its row, its file and its
    /// declaration in its parent, and cargo checks the result — a name cargo
    /// still finds a use for rolls the whole list back.
    pub(crate) fn delete_rust(&self, names: &[&str], budget: usize) -> Out {
        let ix = self.index()?;
        let mut targets: Vec<usize> = vec![];
        for name in names {
            let i = match self.resolve(&ix, name) {
                Ok(i) => i,
                // A Rust type and its `impl` share a name and the item wins
                // (D-158), so a container the name also fits is found through it.
                Err(e) if e.1 == 1 => rust_container(&ix, name)?.ok_or(e)?,
                Err(e) => return Err(e),
            };
            targets.push(i);
        }
        let before = self.broken()?;
        let out = self.transaction(&[], || {
            let ix = self.index()?;
            let named: Vec<(String, String)> = targets.iter().map(|&i| (ix.syms[i].qname(), ix.syms[i].module.clone())).collect();
            let gone: Vec<usize> = targets.iter().flat_map(|&i| rust_descendants(&ix, i)).collect();
            // A type and its inherent `impl` share a qname (D-158), so only a
            // name no live symbol still bears shows the error is this delete's:
            // otherwise the leftover `impl`'s error would read as the type's.
            let deleted: Vec<String> = gone
                .iter()
                .map(|&j| ix.syms[j].qname())
                .filter(|q| !ix.syms.iter().enumerate().any(|(k, s)| !gone.contains(&k) && s.qname() == *q))
                .collect();
            let endings = rust_endings(&ix);
            let mut emptied: Vec<String> = targets.iter().map(|&i| ix.syms[i].module.clone()).collect();
            for j in &gone {
                self.retire(&ix.syms[*j].id, "")?;
            }
            let dropped = self.rust_prune_impls(&emptied)?;
            let mut retired: Vec<String> = vec![];
            self.rust_prune_modules(&mut emptied, &mut retired)?;
            // Every level's ending, the file's or an inline container's, hangs
            // on its last row; every row this delete took hands its separator
            // to the new last one, so the level keeps its ending (a `mod x;`
            // item the prune took counts too).
            self.rust_fix_endings(&endings)?;
            let ix = self.index()?;
            let diags = self.check_all(&ix)?;
            let key = |d: &Diag| d.symbol.clone().or_else(|| d.module.clone().map(|m| format!("{m} imports")));
            let mut users: Vec<String> = diags
                .iter()
                .filter(|d| d.error)
                .filter_map(|d| match key(d) {
                    Some(k) if before.contains_key(&k) || deleted.contains(&k) => None,
                    // An error cargo placed on no symbol (a `build.rs` failure, say)
                    // goes by its own first line, so an agent can act on it.
                    k => Some(k.unwrap_or_else(|| d.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("?").to_string())),
                })
                .collect();
            unique(&mut users);
            if !users.is_empty() {
                let listed = users.join("\n  ");
                return if named.len() == 1 {
                    fail(format!("E5001 {} is still used by:\n  {listed}", named[0].0))
                } else {
                    // cargo's errors don't say which named symbol each comes
                    // from, so several names are refused together (D-173).
                    let listed_names: Vec<&str> = named.iter().map(|(q, _)| q.as_str()).collect();
                    fail(format!("E5001 nothing deleted.\n{} are still used by:\n  {listed}", listed_names.join(", ")))
                };
            }
            let mut text = String::new();
            for (qname, module) in &named {
                let retired = if retired.contains(module) { format!("; module {module} retired") } else { String::new() };
                text += &format!("Deleted {qname}{retired}.\n");
            }
            text += &dropped_impls_note(&dropped);
            Ok(lockdown::trim_output(&text, budget) + &broken_report(&before, &diags))
        })?;
        Ok(out)
    }

    /// Deletes the rows of the modules a Rust `delete` emptied (D-173): each
    /// loses its declaration in its parent — the `mod x;` line in the parent's
    /// header, or the parent's `mod x;` item — and `materialize` prunes its file
    /// (D-136). A parent this empties is looked at too; `crate` stays, and a test
    /// crate's file (`tests/x.rs`, D-169) is found by Cargo itself, so it loses
    /// only its row and its file and no parent is touched.
    pub(crate) fn rust_prune_modules(&self, todo: &mut Vec<String>, retired: &mut Vec<String>) -> Result<(), Failure> {
        unique(todo);
        while let Some(module) = todo.pop() {
            if module == "crate" {
                continue;
            }
            let ix = self.index()?;
            if ix.syms.iter().any(|s| s.module == module) {
                continue;
            }
            if ix.modules.iter().any(|m| parent_of(&m.path) == module.as_str()) {
                continue;
            }
            let Some(row) = ix.modules.iter().find(|m| m.path == module) else {
                continue;
            };
            let (own, parent) = mod_declaration(&module);
            let (own, parent) = (own.to_string(), parent.to_string());
            // A test crate root (`integration::x`) has no `mod` line, but a
            // `#[path]` helper of one is named the same way and is declared in
            // its parent's header, so the declaration goes too.
            let declaring: Vec<&ModuleRow> = ix.modules.iter().filter(|m| m.path != module && declares_module(m, &own, row)).collect();
            let root = is_test_crate_root(&module) && declaring.is_empty();
            self.db.execute("DELETE FROM modules WHERE path = ?1", [module.as_str()])?;
            retired.push(module.clone());
            if root {
                continue;
            }
            if !declaring.is_empty() {
                for parent in declaring {
                    let p = parent.path.clone();
                    let header = self.header(&p)?;
                    self.set_header(&p, &remove_mod(&header, &own))?;
                    todo.push(p);
                }
                continue;
            }
            if let Some(p) = self.resolve_module(&ix, &parent).map(|r| r.path.clone()) {
                let header = self.header(&p)?;
                if header.lines().any(|l| is_mod_line(l) && mod_name(l) == Some(own.as_str())) {
                    self.set_header(&p, &remove_mod(&header, &own))?;
                } else if let Some(i) = ix.syms.iter().position(|s| s.module == p && s.kind_text == "mod" && s.name == own) {
                    self.retire(&ix.syms[i].id, "")?;
                }
            }
            todo.push(parent);
        }
        Ok(())
    }

    /// `format <module>` in a Rust store (D-180): rustfmt the module's stored
    /// text whole, then save every item through the whole-module submit path,
    /// so ids stay by name and the result is checked with cargo. A file the
    /// store doesn't hold is not a module it can format. A file that differs
    /// from the store is repaired first, since `format` is the one repair
    /// command.
    pub(crate) fn format_module_rust(&self, m: &str) -> Out {
        let ix = self.index()?;
        let Some(row) = self.resolve_module(&ix, m) else {
            return fail(format!("E1003 unknown module: {m}; `format` takes a module the store holds"));
        };
        let reason = crate::language::reason(crate::language::RUST)?;
        if let Some(message) = format_missing(reason, cargo::rustfmt_installed()) {
            return crate::lockdown::error("E6013", "format", message);
        }
        let module = row.path.clone();
        let note = self.repair_drift(&ix, row)?;
        self.materialize(&ix)?;
        let (source, ranges) = store::projection(&ix, row);
        let text = cargo::format_module(&self.root, &source).map_err(|e| {
            let line =
                e.0.strip_prefix("Not saved: E0100 syntax error at line ")
                    .and_then(|rest| rest.split_once(':'))
                    .and_then(|(line, _)| line.trim().parse::<usize>().ok());
            let label = line
                .and_then(|line| ranges.iter().find(|(first, last, _)| *first <= line && line <= *last).map(|(.., name)| name.clone()))
                .unwrap_or_else(|| module.clone());
            let lines = ranges.iter().map(|(_, last, _)| *last).max().unwrap_or(1);
            add_repair_note(store::rust_syntax_error(e, &label, None, &module, 0, 0, lines), note.as_deref())
        })?;
        let out = self.submit_module_rust(m, &text, true).map_err(|e| add_repair_note(e, note.as_deref()))?;
        Ok(match note {
            Some(note) => format!("{note}\n{out}"),
            None => out,
        })
    }

    /// Saves a module file that differs from the store under `.solace/outside/`
    /// before a repair overwrites it, and returns the note that names it.
    fn repair_drift(&self, ix: &Index, row: &ModuleRow) -> Result<Option<String>, Failure> {
        let module = row.path.clone();
        let rel = row.rel();
        let Ok(disk) = std::fs::read_to_string(self.root.join(&rel)) else { return Ok(None) };
        let names = store::drift(ix, row, &disk);
        if names.is_empty() {
            return Ok(None);
        }
        let names = names.join(", ");
        let differs = format!("E0104 {rel} differs from the store in: {names}.");
        let again = format!("Run `format {module}` again after fixing it.");
        let kept = match crate::outside::keep(&self.root, &rel, disk.as_bytes()) {
            Ok(kept) => kept,
            Err(e) => return fail(format!("{differs} The current file could not be kept: {}. {again}", e.0)),
        };
        if let Err(e) = self.record_module_file(row, disk.as_bytes()) {
            return fail(format!("{differs} The current file is kept at {kept}, but the store could not record it: {}. {again}", e.0));
        }
        Ok(Some(format!("Repaired {rel}: it differed from the store in: {names}. The file it replaced is kept at {kept}.")))
    }

    /// Retires the inherent `impl` rows a write left with no members (D-158,
    /// D-187). A trait impl is named `<Type as Trait>`, so a plain name is
    /// inherent; an impl with any member stays. A dropped last row hands its
    /// separator to the row before it, so the file keeps its ending. Each
    /// returned pair is the dropped impl's qname and its name.
    pub(crate) fn rust_prune_impls(&self, modules: &[String]) -> Result<Vec<(String, String)>, Failure> {
        let ix = self.index()?;
        let endings = rust_endings(&ix);
        let mut dropped: Vec<(String, String)> = vec![];
        for s in &ix.syms {
            if s.kind_text != "impl" || s.name.starts_with('<') || !modules.contains(&s.module) {
                continue;
            }
            if ix.syms.iter().any(|c| c.parent.as_deref() == Some(s.id.as_str())) {
                continue;
            }
            self.retire(&s.id, "")?;
            dropped.push((s.qname(), s.name.clone()));
        }
        if dropped.is_empty() {
            return Ok(dropped);
        }
        self.rust_fix_endings(&endings)?;
        Ok(dropped)
    }

    /// Hands the separator of each level's former last row, if it is gone, to
    /// the new last row, so a file or an inline container keeps its ending.
    fn rust_fix_endings(&self, endings: &[RustEnding]) -> Result<(), Failure> {
        let ix = self.index()?;
        for ending in endings {
            if ix.syms.iter().any(|s| s.id == ending.id) {
                continue;
            }
            let next = ix.syms.iter().filter(|s| s.module == ending.module && s.parent == ending.parent).next_back();
            if let Some(s) = next {
                let sep = ending.sep.clone().unwrap_or_else(|| "\n".into());
                self.db.execute("UPDATE symbols SET sep = ?2 WHERE id = ?1", params![&s.id, sep])?;
            }
        }
        Ok(())
    }
}

/// A note for the lines of `text` that rustfmt left longer than `max_width`.
fn long_line_note(name: &str, text: &str, max_width: usize) -> Option<String> {
    long_line_note_at(name, text, max_width, 1)
}

/// The long-line notes for a whole module's items: `all` when rustfmt formatted
/// the file whole, otherwise only items whose text changed or are new.
fn long_line_notes(ix: &Index, module: &str, items: &[RustItem], max_width: usize, all: bool) -> Vec<String> {
    fn walk(ix: &Index, module: &str, items: &[RustItem], max_width: usize, all: bool, out: &mut Vec<String>) {
        for item in items {
            let name = format!("{module}.{}", item.name);
            let changed = item.stored.is_none_or(|i| ix.syms[i].text != item.text || ix.syms[i].tail != item.tail);
            if (all || changed)
                && let Some(note) = long_line_note(&name, &item.text, max_width)
            {
                out.push(note);
            }
            walk(ix, module, &item.children, max_width, all, out);
            if (all || changed)
                && let Some(note) = long_line_note_at(&name, &item.tail, max_width, tail_line(item))
            {
                out.push(note);
            }
        }
    }
    let mut out = vec![];
    walk(ix, module, items, max_width, all, &mut out);
    out
}

/// Why `format` can't run: the language's setting or toolchain, then rustfmt.
fn format_missing(reason: Option<String>, has_rustfmt: bool) -> Option<String> {
    reason.or_else(|| (!has_rustfmt).then(|| cargo::RUSTFMT_MISSING.to_string()))
}

/// How wide `line` prints: a tab reaches the next 4-column stop, a wide
/// character or a regional-indicator pair takes two columns, and a combining
/// mark none.
fn display_width(line: &str) -> usize {
    let mut width = 0;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        width += match c {
            '\t' => 4 - width % 4,
            '\u{1f1e6}'..='\u{1f1ff}' => {
                if chars.peek().is_some_and(|next| matches!(*next, '\u{1f1e6}'..='\u{1f1ff}')) {
                    chars.next();
                }
                2
            }
            '\u{0300}'..='\u{036f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{200b}'..='\u{200f}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{fe20}'..='\u{fe2f}'
            | '\u{e0100}'..='\u{e01ef}' => 0,
            '\u{1100}'..='\u{115f}'
            | '\u{2e80}'..='\u{a4cf}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7a3}'
            | '\u{f900}'..='\u{faff}'
            | '\u{fe10}'..='\u{fe19}'
            | '\u{fe30}'..='\u{fe6f}'
            | '\u{ff00}'..='\u{ff60}'
            | '\u{ffe0}'..='\u{ffe6}'
            | '\u{1f300}'..='\u{1f64f}'
            | '\u{1f680}'..='\u{1f6ff}'
            | '\u{1f900}'..='\u{1f9ff}'
            | '\u{1fa70}'..='\u{1faff}'
            | '\u{20000}'..='\u{3fffd}' => 2,
            _ => 1,
        };
    }
    width
}

/// `long_line_note` for text that starts at `first_line` in its item.
fn long_line_note_at(name: &str, text: &str, max_width: usize, first_line: usize) -> Option<String> {
    let lines: Vec<String> =
        text.lines().enumerate().filter(|(_, line)| display_width(line) > max_width).map(|(i, _)| (first_line + i).to_string()).collect();
    if lines.is_empty() {
        return None;
    }
    let (what, verb, them) = if lines.len() == 1 { ("line", "is", "it") } else { ("lines", "are", "them") };
    Some(format!(
        "{name}: {what} {} {verb} longer than rustfmt's max_width ({max_width}); rustfmt left {them} \
         as written, so check the indentation or split the long literal",
        lines.join(", ")
    ))
}

/// The line of an item's tail where it starts in that item's text.
fn tail_line(item: &RustItem) -> usize {
    let mut prefix = item.text.clone();
    for child in &item.children {
        prefix += &child.joined();
        prefix += &child.sep;
    }
    prefix.lines().count() + usize::from(prefix.ends_with('\n'))
}

/// The one container among `found` (D-168): the first `impl` when several
/// share the name; several other containers with the name are ambiguous, as
/// they are in `rust_symbol`.
pub(crate) fn rust_one(found: &[usize], ix: &Index, input: &str) -> Result<Option<usize>, Failure> {
    if let Some(i) = found.iter().copied().find(|&i| ix.syms[i].kind_text == "impl") {
        return Ok(Some(i));
    }
    match found {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        many => {
            let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind_text)).collect();
            fail(format!("E1002 ambiguous name: {input} could be\n{}", list.join("\n")))
        }
    }
}

/// Keeps a repair's note ahead of a later error, since the file was already
/// replaced and its copy named.
fn add_repair_note(e: Failure, note: Option<&str>) -> Failure {
    match note {
        Some(note) => Failure(format!("{note}\n{}", e.0), e.1),
        None => e,
    }
}

pub(crate) const TYPE_NAMESPACE: u8 = 1;

pub(crate) const VALUE_NAMESPACE: u8 = 2;

pub(crate) const MACRO_NAMESPACE: u8 = 4;

/// Every namespace an item's name lives in: a unit or tuple struct also names
/// a value constructor, so it clashes with a `fn` of the same name.
pub(crate) fn rust_namespaces(kind: &str, text: &str) -> u8 {
    let mut namespaces = rust_namespace(kind);
    let braced = syn::parse_str::<syn::ItemStruct>(text).is_ok_and(|item| matches!(item.fields, syn::Fields::Named(_)));
    if kind == "struct" && !braced {
        namespaces |= VALUE_NAMESPACE;
    }
    namespaces
}

/// Whether two items of one module would clash by sharing a namespace.
pub(crate) fn rust_namespaces_overlap(a_kind: &str, a_text: &str, b_kind: &str, b_text: &str) -> bool {
    rust_namespaces(a_kind, a_text) & rust_namespaces(b_kind, b_text) != 0
}

/// Whether `path` names a child module `name` of `module`.
pub(crate) fn module_path_named(module: &str, name: &str, path: &str) -> bool {
    let child = if module == "crate" { name.to_string() } else { format!("{module}::{name}") };
    path == child || path == format!("crate::{child}")
}

/// The reply line naming the empty impls a write dropped; empty when none.
pub(crate) fn dropped_impls_note(dropped: &[(String, String)]) -> String {
    if dropped.is_empty() {
        return String::new();
    }
    let names = dropped.iter().map(|(q, _)| q.as_str()).collect::<Vec<_>>().join(", ");
    format!("Dropped empty impl {names}.\n")
}

/// The last row under one parent and the separator that followed it (D-173):
/// the row an ending hangs on, before a delete takes it.
struct RustEnding {
    module: String,
    parent: Option<String>,
    id: String,
    sep: Option<String>,
}
