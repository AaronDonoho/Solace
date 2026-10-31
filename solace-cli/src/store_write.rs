//! Changing the store: submit, delete, check, test, probe and work.

use crate::Ref;
use crate::diagnostics::Diag;
use crate::failure::{Failure, Out};
use crate::{Index, Store, Sym, next_step, operands, path_dependencies};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::cli::PROBE;
use rusqlite::params;
use std::fs;

#[cfg(test)]
#[path = "store_write_tests.rs"]
mod tests;
use crate::lockdown::commands::trim_output;
use crate::board::task_model::task;
use crate::board::reviews::changes_of;
use crate::board::runs::work_run;
use crate::board::runs::context;

const STDLIB: &[&str] = &[
    "bool",
    "bit_array",
    "bytes_tree",
    "dict",
    "dynamic",
    "float",
    "function",
    "int",
    "io",
    "list",
    "option",
    "order",
    "pair",
    "result",
    "set",
    "string",
    "string_tree",
    "uri",
];

/// Each import in a header: (module, unqualified items as written, e.g.
/// `type Option` or `None`, and the line range it spans).
pub(crate) fn import_statements(header: &str) -> Vec<(String, Vec<String>, (usize, usize))> {
    let lines: Vec<&str> = header.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let Some(rest) = lines[i].strip_prefix("import ") else {
            i += 1;
            continue;
        };
        let start = i;
        while !lines[i].contains('}') && lines[start].contains(".{") && i + 1 < lines.len() {
            i += 1;
        }
        let stmt = lines[start..=i].join(" ");
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/').collect();
        let items = match (stmt.find(".{"), stmt.find('}')) {
            (Some(a), Some(b)) => {
                stmt[a + 2..b].split(',').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect()
            }
            _ => vec![],
        };
        out.push((module, items, (start, i)));
        i += 1;
    }
    out
}

/// Adds unqualified `items` to the header's import of `module`, or adds the
/// import. The result is formatted.
pub(crate) fn merge_import(header: &str, module: &str, items: &[String]) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let Some((_, existing, (start, end))) = import_statements(header).into_iter().find(|(m, _, _)| m == module) else {
        let line = if items.is_empty() { format!("import {module}") } else { format!("import {module}.{{{}}}", items.join(", ")) };
        let added = add_import(header, &line);
        return crate::language::gleam::format(&added).map_or(added, |f| f.trim().to_string());
    };
    if items.iter().all(|i| existing.contains(i)) {
        return header.to_string();
    }
    let stmt = lines[start..=end].join(" ");
    let suffix = stmt.rfind('}').map_or_else(|| stmt[format!("import {module}").len()..].to_string(), |b| stmt[b + 1..].to_string());
    let mut all = existing;
    all.extend(items.iter().filter(|i| !all.contains(i)).cloned().collect::<Vec<_>>());
    let line = format!("import {module}.{{{}}}{}", all.join(", "), suffix.trim_end());
    let mut out: Vec<String> = lines[..start].iter().map(|s| s.to_string()).collect();
    out.push(line);
    out.extend(lines[end + 1..].iter().map(|s| s.to_string()));
    let joined = out.join("\n");
    crate::language::gleam::format(&joined).map_or(joined, |f| f.trim().to_string())
}

/// Adds an import line after the existing imports (or after the module doc).
pub(crate) fn add_import(header: &str, line: &str) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let at = lines.iter().rposition(|l| l.starts_with("import ") || l.starts_with("////")).map_or(0, |i| i + 1);
    // a multi-line import ends at its closing brace
    let at = if at > 0 && lines[at - 1].ends_with('{') {
        lines[at..].iter().position(|l| l.starts_with('}')).map_or(at, |p| at + p + 1)
    } else {
        at
    };
    let mut out: Vec<&str> = lines[..at].to_vec();
    if at > 0 && lines[at - 1].starts_with("////") {
        out.push("");
    }
    out.push(line);
    out.extend_from_slice(&lines[at..]);
    out.join("\n").trim().to_string()
}

// --- compiler output ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DependencyModule {
    pub(crate) path: String,
    pub(crate) test: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MissingImport {
    pub(crate) module: String,
    pub(crate) alias: Option<String>,
}

/// The modules of the project's direct dependencies, and whether only test
/// code may use them. Indirect packages in `build/packages` don't count.
pub(crate) fn dependency_modules(root: &Path) -> Vec<DependencyModule> {
    let Ok(text) = fs::read_to_string(root.join("gleam.toml")) else { return vec![] };
    let Ok(v) = text.parse::<toml::Value>() else { return vec![] };
    let mut out: Vec<DependencyModule> = vec![];
    for (table, test) in [("dependencies", false), ("dev-dependencies", true), ("dev_dependencies", true)] {
        let Some(deps) = v.get(table).and_then(|d| d.as_table()) else { continue };
        for (name, spec) in deps {
            let dir = match spec.get("path").and_then(|p| p.as_str()) {
                Some(path) => root.join(path),
                None => root.join("build/packages").join(name),
            };
            let mut files = vec![];
            if crate::language::gleam::collect_gleam_files(&dir.join("src"), &mut files).is_err() {
                continue;
            }
            for file in files {
                let Ok(rel) = file.strip_prefix(dir.join("src")) else { continue };
                let path = rel.with_extension("").to_string_lossy().replace('\\', "/");
                match out.iter_mut().find(|m| m.path == path) {
                    Some(one) => one.test &= test,
                    None => out.push(DependencyModule { path, test }),
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn import_available(module: &str, ix: &Index, deps: &[DependencyModule], test: bool) -> bool {
    if let Some(m) = ix.modules.iter().find(|m| m.path == module) {
        return m.dir == "src" || test;
    }
    if let Some(d) = deps.iter().find(|d| d.path == module) {
        return !d.test || test;
    }
    module.strip_prefix("gleam/").is_some_and(|name| STDLIB.contains(&name))
}

fn import_match(ix: &Index, deps: &[DependencyModule], qualifier: &str, test: bool) -> ImportMatch {
    let project = modules_named(ix.modules.iter().filter(|m| m.dir == "src" || test).map(|m| m.path.as_str()), qualifier);
    match project.as_slice() {
        [need] => return ImportMatch::One(need.clone()),
        [] => {}
        _ if STDLIB.contains(&qualifier) => {
            return ImportMatch::One(MissingImport { module: format!("gleam/{qualifier}"), alias: None });
        }
        many => return ImportMatch::Many(many.to_vec()),
    }
    if STDLIB.contains(&qualifier) {
        return ImportMatch::One(MissingImport { module: format!("gleam/{qualifier}"), alias: None });
    }
    let dependency = modules_named(deps.iter().filter(|d| !d.test || test).map(|d| d.path.as_str()), qualifier);
    match dependency.as_slice() {
        [need] => return ImportMatch::One(need.clone()),
        [] => {}
        many => return ImportMatch::Many(many.to_vec()),
    }
    let mut found: Vec<MissingImport> = vec![];
    for m in &ix.modules {
        for import in crate::language::gleam::imports(&m.header) {
            if import.alias != qualifier || !import_available(&import.module, ix, deps, test) {
                continue;
            }
            let need = MissingImport { module: import.module, alias: Some(qualifier.to_string()) };
            if !found.iter().any(|n| n.module == need.module) {
                found.push(need);
            }
        }
    }
    if found.len() == 1 {
        return ImportMatch::One(found.remove(0));
    }
    if found.len() > 1 {
        return ImportMatch::Many(found);
    }
    let mut unavailable: Vec<String> = modules_named(ix.modules.iter().map(|m| m.path.as_str()), qualifier)
        .into_iter()
        .chain(modules_named(deps.iter().map(|d| d.path.as_str()), qualifier))
        .map(|need| need.module)
        .collect();
    unavailable.sort();
    unavailable.dedup();
    if unavailable.is_empty() { ImportMatch::None } else { ImportMatch::Unavailable(unavailable) }
}

impl MissingImport {
    fn line(&self) -> String {
        match &self.alias {
            Some(alias) => format!("import {} as {alias}", self.module),
            None => format!("import {}", self.module),
        }
    }
}

/// When `gleam.toml`, the resolved package list, or a `.gleam` file under a
/// path dependency's `src` last changed.
pub(crate) type DependencyKey =
    (Option<std::time::SystemTime>, Option<std::time::SystemTime>, Vec<(PathBuf, Option<std::time::SystemTime>)>);

/// Every module whose last path segment is `qualifier`.
fn modules_named<'a>(paths: impl Iterator<Item = &'a str>, qualifier: &str) -> Vec<MissingImport> {
    paths.filter(|path| last_segment(path) == qualifier).map(|path| MissingImport { module: path.to_string(), alias: None }).collect()
}

fn quoted_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &text[text.find(marker)? + marker.len()..];
    rest.find('`').map(|end| &rest[..end])
}

fn unknown_module(text: &str) -> Option<&str> {
    if let Some(qualifier) =
        quoted_after(text, "No module has been found with the name `").or_else(|| quoted_after(text, "Unknown module `"))
    {
        return Some(qualifier);
    }
    let title = text.lines().any(|line| line.trim().trim_start_matches("error: ").trim() == "Unknown module");
    if title { quoted_after(text, "The module `") } else { None }
}

impl Store {
    pub(crate) fn submit(&self, target: crate::store::Target, raw: &str) -> Out {
        if crate::language::is_rust(&self.root) {
            return self.submit_rust(target, raw);
        }
        let raw: String = raw.lines().filter(|l| !l.starts_with(crate::drafts::BLOCK_HEADER)).collect::<Vec<_>>().join("\n");
        let formatted = self.lang().format(&raw)?;
        let parsed = self.lang().split(&formatted);
        let ix = self.index()?;
        let deps = self.dependency_modules();
        let (crate::store::Target::Imports(m) | crate::store::Target::Module(m)) = target;
        let module = self
            .resolve_module(&ix, m)
            .ok_or_else(|| {
                Failure(
                    format!(
                        "E1003 unknown module: {m}; create it with a patch `*** Add: {m}`: its module comment, \
                         imports and code, every line `+`."
                    ),
                    1,
                )
            })?
            .path
            .clone();
        let mut notes = vec![];
        let header = &ix.modules.iter().find(|m| m.path == module).unwrap().header;
        let out = self.transaction(&[], || {
            let target_name: Option<String> = match target {
                crate::store::Target::Module(_) => {
                    let existing: Vec<&Sym> = ix.syms.iter().filter(|s| s.module == module).collect();
                    let submitted: HashSet<(String, bool)> = parsed.items.iter().map(|i| name_key(&i.name, i.kind)).collect();
                    if submitted.len() != parsed.items.len() {
                        return crate::failure::fail("E0103 the module text defines the same name twice.");
                    }
                    let missing: Vec<String> =
                        existing.iter().filter(|s| !submitted.contains(&name_key(&s.name, s.kind))).map(|s| s.name.clone()).collect();
                    if !missing.is_empty() {
                        return crate::failure::fail(format!(
                            "E0102 the module text leaves out: {}. Nothing was saved. Removing a symbol must be explicit \
                             (solace delete <name>).",
                            missing.join(", ")
                        ));
                    }
                    let mut header = if parsed.header.is_empty() { header.clone() } else { parsed.header.clone() };
                    let (mut changed, mut added) = (vec![], vec![]);
                    for item in &parsed.items {
                        let test = ix.modules.iter().any(|m| m.path == module && m.dir == "test");
                        for need in missing_imports(&ix, &deps, &header, &item.text, test) {
                            let line = need.line();
                            header = add_import(&header, &line);
                            notes.push(format!("added `{line}` to {module}"));
                        }
                        if test {
                            notes.extend(
                                weak_asserts(&item.text, &aliases(&header)).into_iter().map(|w| format!("{module}.{}: {w}", item.name)),
                            );
                        }
                        match existing.iter().find(|s| name_key(&s.name, s.kind) == name_key(&item.name, item.kind)) {
                            Some(e) if e.text == item.text => {}
                            Some(e) => {
                                self.set_symbol_text(&e.id, &item.name, &item.text)?;
                                changed.push(item.name.clone());
                            }
                            None => {
                                let id = crate::insert_symbol(&self.db, &module, item, None)?;
                                self.history(&id, "create", &item.name)?;
                                added.push(item.name.clone());
                            }
                        }
                    }
                    self.set_header(&module, &header)?;
                    let list = |v: &Vec<String>| {
                        if v.is_empty() { "none".to_string() } else { v.join(", ") }
                    };
                    notes.push(format!("{module}: changed {}; added {}", list(&changed), list(&added)));
                    None
                }
                crate::store::Target::Imports(_) => {
                    if !parsed.items.is_empty() {
                        return crate::failure::fail("E0101 --imports text may only contain imports and the module comment.");
                    }
                    self.set_header(&module, &parsed.header)?;
                    None
                }
            };
            let ix = self.index()?;
            let before_broken = self.broken()?;
            let before_warnings = self.warning_baseline()?;
            let diags = self.check_all(&ix)?;
            let whole_module = matches!(target, crate::store::Target::Module(_));
            let own = |d: &&Diag| match &target_name {
                Some(q) => d.symbol.as_deref() == Some(q.as_str()),
                None if whole_module => d.module.as_deref() == Some(module.as_str()),
                None => d.symbol.is_none() && d.module.as_deref() == Some(module.as_str()),
            };
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error).filter(own).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                let now = self.index()?;
                let module_syms: Vec<String> = now.syms.iter().filter(|s| s.module == module).map(Sym::qname).collect();
                return crate::failure::fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}{}",
                    text.join("\n\n"),
                    self.callee_hint(&own_errors, &module_syms)? + &unknown_module_hint(self, &now, &deps, &own_errors)?
                ));
            }
            if whole_module {
                let after = self.index()?;
                let before: crate::Before = after
                    .syms
                    .iter()
                    .filter(|s| s.module == module)
                    .map(|s| {
                        let old = ix.syms.iter().find(|o| o.module == module && name_key(&o.name, o.kind) == name_key(&s.name, s.kind));
                        (s.id.clone(), old.map(|o| o.text.clone()))
                    })
                    .collect();
                notes.extend(self.impact_note(&after, &before)?);
            }
            let what = if whole_module { module.clone() } else { format!("{module} imports") };
            let mut out = format!("Saved {}.\n", target_name.as_deref().unwrap_or(&what));
            out += &crate::diagnostics::warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error).filter(own),
                diags.iter().filter(|d| !d.error).filter(|d| !own(d)),
            );
            out += &crate::diagnostics::broken_report(&before_broken, &diags);
            Ok(out)
        })?;
        Ok(crate::note_lines(&notes) + &out)
    }
    /// Replaces or adds symbols in one transaction, type-checked together, so a
    /// type change and its callers can land at once (PROPOSAL.md §4.6).
    /// `name`: the one symbol the text replaces. `new_in`: the module the text
    /// adds a symbol to. Neither: the text is blocks headed `// ── <name> ──`,
    /// as `view` and `inspect` print them; a name that doesn't exist yet adds
    /// a symbol to the module its prefix names.
    pub(crate) fn submit_symbols(&self, name: Option<&str>, new_in: Option<&str>, raw: &str) -> Out {
        self.submit_symbols_blocks(name, new_in, raw, false)
    }
    /// `formatted`: the blocks are already rustfmt's text, as a patch's are
    /// (`drafts::patch_blocks`), so a Rust submit formats nothing twice (D-168).
    pub(crate) fn submit_symbols_blocks(&self, name: Option<&str>, new_in: Option<&str>, raw: &str, formatted: bool) -> Out {
        if crate::language::is_rust(&self.root) {
            return self.submit_symbols_rust(name, new_in, raw, formatted);
        }
        let ix = self.index()?;
        let deps = self.dependency_modules();
        let is_header = |l: &str| l.starts_with(crate::drafts::BLOCK_HEADER);
        let blocks: Vec<(Option<String>, String)> =
            if raw.lines().filter(|l| is_header(l)).count() > 1 || (name.is_none() && new_in.is_none()) {
                let mut blocks: Vec<(Option<String>, String)> = vec![];
                for line in raw.lines() {
                    if let Some(h) = line.strip_prefix(crate::drafts::BLOCK_HEADER) {
                        if h.starts_with("module ") {
                            return crate::failure::fail("E0101 a whole module goes through `submit <module>`.");
                        }
                        blocks.push((Some(crate::block_name(h)), String::new()));
                    } else if let Some((_, text)) = blocks.last_mut() {
                        text.push_str(line);
                        text.push('\n');
                    } else if !line.trim().is_empty() {
                        return crate::failure::fail(format!(
                            "E0101 text for `submit` with no name must be blocks, each headed `{}<module>.<name> ──`.",
                            crate::drafts::BLOCK_HEADER
                        ));
                    }
                }
                blocks
            } else {
                vec![(name.map(str::to_string), raw.lines().filter(|l| !is_header(l)).collect::<Vec<_>>().join("\n"))]
            };
        if blocks.is_empty() {
            return crate::failure::fail("E0101 nothing to submit.");
        }
        let mut notes: Vec<String> = vec![];
        if let Some(name) = name.filter(|_| blocks.len() > 1) {
            notes.push(format!("`submit {name}` submitted every block; plain `submit` does the same"));
        }
        // (module, the symbol replaced, the new item, imports in the text)
        let mut plan: Vec<(String, Option<usize>, crate::language::Item, String)> = vec![];
        // Modules that don't exist yet are created, with the module comment and
        // imports of their first block: (path, dir, header).
        let mut created: Vec<(String, &str, String)> = vec![];
        for (block_name, text) in &blocks {
            let parsed = self.lang().split(&self.lang().format(text)?);
            // `--new` adds every symbol in the text; a named block holds one.
            if parsed.items.is_empty() || (new_in.is_none() && parsed.items.len() > 1) {
                return crate::failure::fail(format!(
                    "E0101 each block holds one symbol; {} defines {} ({}). To add several to one module, use `submit --new <module>`.",
                    block_name.as_deref().unwrap_or("the text"),
                    parsed.items.len(),
                    parsed.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            let mut create = |m: &str| {
                if !created.iter().any(|c| c.0 == m) {
                    let dir = if m.rsplit('/').next().unwrap().ends_with("_test") { "test" } else { "src" };
                    created.push((m.to_string(), dir, parsed.header.clone()));
                }
                m.to_string()
            };
            let module_of = |m: &str| {
                self.resolve_module(&ix, m)
                    .or_else(|| m.rsplit_once('.').and_then(|(p, _)| self.resolve_module(&ix, p)))
                    .map(|r| r.path.clone())
                    .ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))
            };
            let (module, existing) = match (new_in, block_name) {
                (Some(m), _) => match module_of(m) {
                    Ok(path) => (path, None),
                    Err(_) if self.lang().valid_module_path(m) => (create(m), None),
                    Err(e) => return Err(e),
                },
                (None, Some(n)) => match self.resolve(&ix, n) {
                    Ok(i) => (ix.syms[i].module.clone(), Some(i)),
                    Err(e) => match n.rsplit_once('.') {
                        Some((m, _)) if self.resolve_module(&ix, m).is_some() => (module_of(m)?, None),
                        Some((m, _)) if self.lang().valid_module_path(m) => (create(m), None),
                        _ => return Err(e),
                    },
                },
                (None, None) => unreachable!(),
            };
            for item in &parsed.items {
                let clash = ix.syms.iter().position(|s| {
                    s.module == module
                        && s.name == item.name
                        && (s.kind == crate::language::Kind::Type) == (item.kind == crate::language::Kind::Type)
                        && existing != Some(ix.syms.iter().position(|x| x.id == s.id).unwrap())
                });
                if let Some(c) = clash {
                    return crate::failure::fail(format!("E1004 {} already exists; submit to it instead.", ix.syms[c].qname()));
                }
                if plan.iter().any(|p| p.0 == module && p.1.is_none() && name_key(&p.2.name, p.2.kind) == name_key(&item.name, item.kind)) {
                    return crate::failure::fail(format!("E0103 the text defines {module}.{} twice.", item.name));
                }
                plan.push((module.clone(), existing, item.clone(), parsed.header.clone()));
            }
        }

        let created_files: Vec<PathBuf> =
            created.iter().map(|(module, dir, _)| self.root.join(dir).join(format!("{module}.gleam"))).collect();
        if created_files.iter().any(|p| p.exists()) {
            return crate::failure::fail(
                "E0101 the new module's file already exists outside the store; import or resolve it before creating this module",
            );
        }
        let out = self.transaction(&created_files, || {
            for (path, dir, header) in &created {
                self.db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, dir, header])?;
                notes.push(format!("created module {path} ({dir}/{path}.gleam)"));
            }
            let (mut saved, mut unchanged): (Vec<String>, Vec<String>) = (vec![], vec![]);
            let mut changed = !created.is_empty();
            for (module, existing, item, imports) in &plan {
                let mut header = self.header(module)?;
                let was = header.clone();
                for (path, items, _) in import_statements(imports) {
                    let merged = merge_import(&header, &path, &items);
                    if merged != header {
                        notes.push(format!("imported {path} in {module}"));
                        header = merged;
                    }
                }
                let test = ix.modules.iter().any(|m| &m.path == module && m.dir == "test")
                    || created.iter().any(|(p, d, _)| p == module && *d == "test");
                for need in missing_imports(&ix, &deps, &header, &item.text, test) {
                    let line = need.line();
                    header = if need.alias.is_some() { add_import(&header, &line) } else { merge_import(&header, &need.module, &[]) };
                    notes.push(format!("added `{line}` to {module}"));
                }
                if test {
                    notes.extend(weak_asserts(&item.text, &aliases(&header)).into_iter().map(|w| format!("{module}.{}: {w}", item.name)));
                }
                self.set_header(module, &header)?;
                changed |= header != was;
                match existing {
                    // the same text as stored: not saved again (D-46)
                    Some(e) if ix.syms[*e].text == item.text && ix.syms[*e].name == item.name && ix.syms[*e].kind == item.kind => {
                        unchanged.push(format!("{module}.{}", item.name));
                        continue;
                    }
                    Some(e) => {
                        let e = &ix.syms[*e];
                        self.db.execute(
                            "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, revision = ?5 WHERE id = ?1",
                            params![e.id, item.name, item.kind.as_str(), item.text, crate::revision(&item.text)],
                        )?;
                        self.history(&e.id, "edit", &item.name)?;
                    }
                    None => {
                        let id = crate::insert_symbol(&self.db, module, item, None)?;
                        self.history(&id, "create", &item.name)?;
                    }
                }
                saved.push(format!("{module}.{}", item.name));
                changed = true;
            }
            if !changed {
                let broken = self.broken()?;
                let still: Vec<&String> = unchanged.iter().filter(|u| broken.contains_key(*u)).collect();
                let still = if still.is_empty() {
                    String::new()
                } else {
                    format!("Still broken: {} (`check` shows why).\n", still.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                };
                return Ok(format!(
                    "Unchanged: {}. The text equals the stored symbols, so nothing was saved or checked.\n{still}",
                    unchanged.join(", ")
                ));
            }
            let modules: HashSet<&str> = plan.iter().map(|p| p.0.as_str()).collect();
            let own = |d: &Diag| match &d.symbol {
                Some(s) => saved.contains(s),
                None => d.module.as_deref().is_some_and(|m| modules.contains(m)),
            };
            let before_broken = self.broken()?;
            let before_warnings = self.warning_baseline()?;
            let mut now = self.index()?;
            let mut diags = self.check_all(&now)?;
            // Unqualified types and constructors (`Option`, `None`, a project
            // type) that aren't imported: import them and check once more.
            // The compiler stops at the first module with errors, so repeat.
            for _ in 0..plan.len() + 2 {
                let mut fixed = false;
                for d in diags.iter().filter(|d| d.error && own(d)) {
                    if let (Some(m), Some((path, item))) = (&d.module, unimported_name(&ix, &d.text)) {
                        let header = self.header(m)?;
                        let merged = merge_import(&header, &path, std::slice::from_ref(&item));
                        if merged != header && path != *m {
                            self.set_header(m, &merged)?;
                            notes.push(format!("imported `{item}` from {path} in {m}"));
                            fixed = true;
                        }
                    }
                }
                if !fixed {
                    break;
                }
                now = self.index()?;
                diags = self.check_all(&now)?;
            }
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                return crate::failure::fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}{}",
                    text.join("\n\n"),
                    self.callee_hint(&own_errors, &saved)?
                        + &self.same_name_hint(&now, &own_errors)
                        + &unknown_module_hint(self, &now, &deps, &own_errors)?
                ));
            }
            let after = now;
            let before: crate::Before = plan
                .iter()
                .filter_map(|(module, existing, item, _)| {
                    let s = after.syms.iter().find(|s| &s.module == module && s.name == item.name && s.kind == item.kind)?;
                    Some((s.id.clone(), existing.map(|e| ix.syms[e].text.clone())))
                })
                .collect();
            notes.extend(self.impact_note(&after, &before)?);
            let mut out = format!("Saved {}.\n", saved.join(", "));
            if !unchanged.is_empty() {
                out += &format!("Unchanged (the same as stored): {}.\n", unchanged.join(", "));
            }
            out += &crate::diagnostics::warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error && own(d)),
                diags.iter().filter(|d| !d.error && !own(d)),
            );
            out += &crate::diagnostics::broken_report(&before_broken, &diags);
            // The compiler stops at the first module with errors, so name the
            // other callers of a changed function, or the sites that construct a
            // changed type, from the reference graph (D-46).
            if diags.iter().any(|d| d.error) {
                let after = self.index()?;
                let broken: HashSet<&str> = diags.iter().filter_map(|d| d.symbol.as_deref()).collect();
                let mut unchecked: Vec<String> = vec![];
                for (_, existing, item, _) in &plan {
                    let Some(e) = existing else { continue };
                    let before = &ix.syms[*e];
                    let Some(a) = after.syms.iter().position(|s| s.id == before.id) else {
                        continue;
                    };
                    if self.lang().head(&before.item()) == self.lang().head(item) {
                        continue;
                    }
                    for &j in &after.used_by[a] {
                        let n = after.syms[j].qname();
                        let site = item.kind != crate::language::Kind::Type || !crate::sites(&after, j, a, false).is_empty();
                        if site && !broken.contains(n.as_str()) && !saved.contains(&n) && !unchecked.contains(&n) {
                            unchecked.push(n);
                        }
                    }
                }
                if !unchecked.is_empty() {
                    out += &format!(
                        "\nNot checked until the errors above are fixed, and may need the same change: {}\n",
                        unchecked.join(", ")
                    );
                }
            }
            Ok(out)
        })?;
        Ok(crate::note_lines(&notes) + &out)
    }
    /// For a rejected submit whose every error is a call, with the wrong arity
    /// or argument types, to a project function the submission leaves out: the
    /// advice to submit that function too, in the same `submit` (D-46).
    fn callee_hint(&self, errors: &[&Diag], submitted: &[String]) -> Result<String, Failure> {
        let ix = self.index()?;
        let mut callees: Vec<String> = vec![];
        for d in errors {
            let title = d.text.lines().next().unwrap_or("");
            let place = crate::diagnostics::place(d);
            let Some((sym, line)) = place.rsplit_once(", line ") else { return Ok(String::new()) };
            let Some(j) = ix.syms.iter().position(|s| s.qname() == sym) else { return Ok(String::new()) };
            let called: Vec<String> = ix.at[j]
                .iter()
                .filter(|(t, l)| line == (l + 1).to_string() && ix.syms[*t].kind == crate::language::Kind::Fn)
                .map(|(t, _)| ix.syms[*t].qname())
                .filter(|q| !submitted.contains(q))
                .collect();
            if !(title.contains("Incorrect arity") || title.contains("Type mismatch")) || called.is_empty() {
                return Ok(String::new());
            }
            callees.extend(called.into_iter().filter(|c| !callees.contains(c)).collect::<Vec<_>>());
        }
        let names: Vec<String> = callees.iter().map(|c| format!("`{c}`")).collect();
        Ok(format!("\n\nIf you are changing {} too, submit both in one `submit`, a block each.", names.join(", ")))
    }
    /// `delete <name>...`: deletes each named symbol in order (D-102). Nothing
    /// is deleted when any named symbol is still used by a symbol outside the
    /// list, so the check runs over the whole list first: uses only shrink as
    /// symbols go, never grow. A Rust store is cargo's to check (D-173).
    pub(crate) fn delete_all(&self, names: &[&str], budget: usize) -> Out {
        let ix = self.index()?;
        // The name-based graph overestimates, so it would refuse a safe delete
        // (D-173): a Rust store's uses are cargo's to find.
        if ix.rust {
            return self.delete_rust(names, budget);
        }
        let listed = names.iter().map(|name| self.resolve(&ix, name)).collect::<Result<Vec<_>, _>>()?;
        let used: Vec<String> = listed
            .iter()
            .map(|&i| {
                let users: Vec<String> = ix.used_by[i].iter().filter(|j| !listed.contains(j)).map(|&j| ix.syms[j].qname()).collect();
                Ok::<String, Failure>(if users.is_empty() {
                    String::new()
                } else {
                    format!("{} is still used by:\n  {}", ix.syms[i].qname(), users.join("\n  "))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let used: Vec<String> = used.into_iter().filter(|s| !s.is_empty()).collect();
        if !used.is_empty() {
            // a used symbol with a same-name copy: the merge that redirects its uses, once
            let hint = listed
                .iter()
                .find_map(|&i| {
                    let users = ix.used_by[i].iter().filter(|j| !listed.contains(j)).count();
                    let h = if users == 0 { String::new() } else { self.merge_hint(&ix, &ix.syms[i], users) };
                    (!h.is_empty()).then_some(h)
                })
                .unwrap_or_default();
            return if used.len() == 1 {
                crate::failure::fail(format!("E5001 {}{hint}", used[0]))
            } else {
                crate::failure::fail(format!("E5001 nothing deleted.\n{}{hint}", used.join("\n")))
            };
        }
        let before = self.broken()?;
        let mut out = String::new();
        for name in names {
            let ix = self.index()?;
            let i = self.resolve(&ix, name)?;
            self.retire(&ix.syms[i].id, "")?;
            // The module goes with its last symbol, unless another module imports it.
            let module = &ix.syms[i].module;
            let last = !ix.syms.iter().enumerate().any(|(j, s)| j != i && s.module == *module);
            let imported = ix.modules.iter().any(|m| ix.lang.imports(&m.header).iter().any(|imp| imp.module == *module));
            let retired = if last && !imported {
                self.db.execute("DELETE FROM modules WHERE path = ?1", [module])?;
                format!("; module {module} retired")
            } else {
                String::new()
            };
            out += &format!("Deleted {}{retired}.\n", ix.syms[i].qname());
        }
        let diags = self.check_all(&self.index()?)?;
        Ok(trim_output(&out, budget) + &crate::diagnostics::broken_report(&before, &diags))
    }
    #[cfg(test)]
    pub(crate) fn delete(&self, name: &str) -> Out {
        self.delete_all(&[name], usize::MAX)
    }
    pub(crate) fn check(&self) -> Out {
        self.check_report(false)
    }
    pub(crate) fn check_report(&self, show_warnings: bool) -> Out {
        let diags = self.check_all(&self.index()?)?;
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        let warnings = diags.len() - errors.len();
        let details = if show_warnings {
            diags.iter().filter(|d| !d.error).map(|d| d.text.as_str()).collect::<Vec<_>>().join("\n\n")
        } else {
            String::new()
        };
        if errors.is_empty() {
            Ok(format!("No errors ({warnings} warnings).\n{details}"))
        } else {
            let more =
                if crate::language::is_rust(&self.root) { format!("\n{}", crate::language::cargo::ROUND_NOTE) } else { String::new() };
            crate::failure::fail(format!("{} errors, {warnings} warnings:\n\n{}\n{details}{more}", errors.len(), errors.join("\n\n")))
        }
    }
    /// `test [--all | <name>...]`: the tests that can see a change since they
    /// last passed, or that failed or were never run; with names, those that
    /// reach them; `--all`, every test (D-44). A Rust project runs the chosen tests
    /// with `cargo test` by libtest name (D-170); Gleam's tests left out are
    /// made private in the projected files for the run, so gleeunit skips them.
    /// When the server's own selection code is newer than the binary running it,
    /// its selection is out of date, so every test runs and the reply says why.
    pub(crate) fn test(&self, args: &[&str]) -> Out {
        let rust = crate::language::is_rust(&self.root);
        let ix = self.index()?;
        let names = operands(args);
        let named: Vec<usize> = names.iter().map(|n| self.resolve(&ix, n)).collect::<Result<_, _>>()?;
        let all_flag = args.contains(&"--all");
        let stale = !all_flag && named.is_empty() && server_needs_every_test(&self.root, &ix);
        let all = all_flag || stale;
        let tests = crate::reach::test_symbols(&ix);
        let revisions: Vec<String> = ix.syms.iter().map(|s| crate::revision(&s.text)).collect();
        let mut rows: HashMap<String, (String, String)> = self
            .db
            .prepare("SELECT test, reach, result FROM test_runs")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()?;
        let (mut run, mut hashes) = (vec![], HashMap::new());
        let mut reached = HashSet::new();
        for &t in &tests {
            let reach = crate::reach::closure(&ix, t);
            let hash = crate::reach::hash(&ix, &reach, &revisions, &self.root);
            let due = if all {
                true
            } else if !named.is_empty() {
                named.iter().any(|n| reach.contains(n))
            } else {
                rows.get(&ix.syms[t].qname()).is_none_or(|(h, r)| *h != hash || r != "pass")
            };
            // a test other code calls can't be made private
            if due || ix.uses.iter().enumerate().any(|(j, u)| j != t && u.contains(&t)) {
                run.push(t);
            }
            hashes.insert(t, hash);
            reached.extend(reach);
        }
        let untested = self.task_untested(&ix, &reached);
        crate::UNTESTED.with(|u| *u.borrow_mut() = untested.clone());
        let next = next_step(untested.as_deref(), work_run(&self.root));
        let untested = untested.map_or(String::new(), |names| format!("No test reaches {names}.\n"));
        // Suites outside `gleam test` that reach a change since tests last passed.
        let changed: Vec<usize> =
            self.since_pass()?.unwrap_or_default().iter().filter_map(|(id, _)| ix.syms.iter().position(|s| &s.id == id)).collect();
        let suites = crate::reach::reached(&crate::reach::suites(self, &ix)?, &changed);
        let head = test_head(run.len(), tests.len(), &names, all_flag, stale);
        let also = if suites.is_empty() { String::new() } else { format!("Also reached: {}.\n", suites.join(", ")) };
        if run.is_empty() {
            // gleeunit fails when it finds no tests; type-check instead
            let errors: Vec<String> = self.check_all(&ix)?.into_iter().filter(|d| d.error).map(|d| d.text).collect();
            if !errors.is_empty() {
                return crate::failure::fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
            }
            return Ok(head + &also + &untested + &next);
        }
        let files = self.materialize(&ix)?;
        let result = if rust {
            // Rust runs the chosen tests by libtest name, and the whole suite
            // when every test is due (D-170).
            let names: Vec<String> = run.iter().map(|&t| crate::reach::libtest_name(&ix, t)).collect();
            let filters = if run.len() == tests.len() { vec![] } else { names };
            let args = crate::language::cargo::test_args(&self.root, &filters);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            self.cargo(&argv, 600)
        } else {
            let skipped: Vec<(usize, String)> = tests
                .iter()
                .filter(|t| !run.contains(t))
                .map(|&t| (t, ix.syms[t].text.replacen(&format!("pub fn {}(", ix.syms[t].name), &format!("fn {}(", ix.syms[t].name), 1)))
                .collect();
            let rewritten = self.rewritten(&ix, &skipped)?;
            let result = (|| {
                for (path, content) in &rewritten {
                    self.rewrite(path, content)?;
                }
                self.tool(&["test"])
            })();
            if !rewritten.is_empty() {
                self.materialize(&ix)?;
            }
            result
        };
        let (ok, output) = result?;
        let diags = if rust {
            crate::language::cargo::diagnostics_for(&self.root, &output, &files)
        } else {
            self.lang().diagnostics(&output, &files)
        };
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        if !errors.is_empty() {
            return crate::failure::fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
        }
        let failed = if rust { crate::language::cargo::failures(&output) } else { self.lang().failures(&output) };
        // Record results only from a run that finished; passes only when every
        // failure it counted was attributed to a test.
        let summary = output.lines().find(|l| l.contains(" passed, "));
        let complete = if rust {
            let (passed, count) = crate::language::cargo::totals(&output);
            if !ok && passed + count == 0 {
                // cargo stopped before the tests ran: its own words say why.
                return crate::failure::fail(format!("Tests did not run.\n\n{}", output.trim()));
            }
            ok || count == failed.len()
        } else {
            summary.is_some_and(|l| {
                let number =
                    |word: &str| l.split(word).next().and_then(|p| p.split_whitespace().last()).and_then(|n| n.parse::<usize>().ok());
                let count = if l.contains("no failures") { Some(0) } else { number(" failure") };
                ok || count == Some(failed.len())
            })
        };
        if rust || summary.is_some() {
            for &t in &run {
                let q = ix.syms[t].qname();
                let key = if rust { crate::reach::libtest_name(&ix, t) } else { q.clone() };
                let result = match failed.get(&key) {
                    Some(_) => "fail",
                    None if complete => "pass",
                    None => continue,
                };
                self.db.execute("INSERT OR REPLACE INTO test_runs VALUES (?1, ?2, ?3)", params![q, hashes[&t], result])?;
                rows.insert(q, (hashes[&t].clone(), result.to_string()));
            }
            if tests.iter().all(|t| rows.get(&ix.syms[*t].qname()).is_some_and(|(h, r)| *h == hashes[t] && r == "pass")) {
                self.db.execute_batch("DELETE FROM test_base; INSERT INTO test_base SELECT id, text FROM symbols WHERE live = 1;")?;
            }
        }
        let mut out = head;
        if rust {
            for (name, (_, text)) in &failed {
                let text = crate::language::cargo::relocate_failure(text, &files, |symbol| {
                    ix.syms.iter().find(|s| s.qname() == symbol).map(|s| s.text.as_str())
                });
                out += &format!("\n{name}:\n{text}\n");
            }
        } else {
            for line in self.lang().quiet(&output).lines() {
                let line = line.trim_start_matches('.');
                if line.trim().is_empty() || line.trim().starts_with("Checking ") {
                    continue;
                }
                out += &self.lang().relocate(line, &files);
                out.push('\n');
            }
        }
        out += &also;
        if ok { Ok(out + &untested + &next) } else { crate::failure::fail(out) }
    }
    /// D-152: this task's changed symbols in this project, outside test modules,
    /// that no test reaches (`reached`: what the tests reach), named for a reply;
    /// `None` when there are none or outside a board run.
    pub(crate) fn task_untested(&self, ix: &Index, reached: &HashSet<usize>) -> Option<String> {
        let (tid, _) = context()?;
        let t = crate::scope::home(&self.root, &tid).and_then(|home| task(&home, &tid)).ok()?;
        let here = crate::scope::dir(&crate::scope::top(&self.root), &self.root);
        let mut covered = reached.clone();
        let by_id: HashMap<&str, usize> = ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        for &i in reached {
            let mut parent = ix.syms[i].parent.as_deref();
            while let Some(p) = parent {
                let Some(&k) = by_id.get(p) else { break };
                covered.insert(k);
                parent = ix.syms[k].parent.as_deref();
            }
        }
        let names: Vec<String> = changes_of(self, &t)
            .unwrap_or_default()
            .iter()
            .filter(|c| c["project"] == here)
            .filter_map(|c| c["entity"].as_str()?.strip_prefix("symbol:").map(String::from))
            .filter_map(|id| ix.syms.iter().position(|s| s.id == id))
            .filter(|&i| !crate::is_test(ix, i) && !covered.contains(&i))
            .map(|i| crate::reach::short(ix, i))
            .collect();
        if names.is_empty() {
            return None;
        }
        let more = if names.len() > 3 { format!(" and {} more", names.len() - 3) } else { String::new() };
        Some(format!("{}{more}", names[..names.len().min(3)].join(", ")))
    }
    /// Symbols changed since `test` last passed in full: (id, text then, `None`
    /// if new). `None` if it never has.
    pub(crate) fn since_pass(&self) -> Result<Option<crate::Before>, Failure> {
        if !self.db.query_row("SELECT EXISTS(SELECT 1 FROM test_base)", [], |r| r.get::<_, bool>(0))? {
            return Ok(None);
        }
        let rows = self
            .db
            .prepare(
                "SELECT s.id, b.text FROM symbols s LEFT JOIN test_base b ON b.id = s.id \
                 WHERE s.live = 1 AND (b.text IS NULL OR b.text != s.text)",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(Some(rows))
    }
    /// Runs throwaway code against the current project and returns its output.
    /// A Gleam project's text is a module with `pub fn main()`, or statements
    /// that become its body; a Rust project's text is statements in a test in a
    /// scratch copy of the crate, placed in `module` when one is named, or after
    /// the body of `from`'s test. Project imports it uses are added. Nothing is saved.
    pub(crate) fn probe(&self, text: &str, from: Option<&str>, trace: &[&str], module: Option<&str>) -> Out {
        if crate::language::is_rust(&self.root) {
            if from.is_some() && module.is_some() {
                return crate::failure::fail("E1006 --from and --in can't be combined: --from places the probe in the test's module.");
            }
            if !trace.is_empty() {
                return crate::failure::fail(
                    "E1006 --trace is Gleam only: Rust has no call tracing at run time; print values with `println!` or `dbg!`.",
                );
            }
            return crate::rust_probe::run(self, text, module, from);
        }
        if module.is_some() {
            return crate::failure::fail("E1006 --in is Rust only: a Gleam probe reaches project modules with `import`.");
        }
        let ix = self.index()?;
        let deps = self.dependency_modules();
        if ix.modules.iter().any(|m| m.path == crate::cli::PROBE) {
            return crate::failure::fail(format!("E0101 a project module is named {PROBE}; probe can't run."));
        }
        let (prelude, setup) = match from {
            Some(name) => self.probe_from(&ix, name)?,
            None => (String::new(), String::new()),
        };
        // A single expression is printed: `probe` with `money.percent(7400, 5)` shows 370.
        let one = text.trim();
        let text = if !one.is_empty()
            && !one.contains('\n')
            && !["echo ", "let ", "assert ", "import ", "use ", "panic", "todo"].iter().any(|k| one.starts_with(k))
        {
            format!("echo {one}")
        } else {
            text.to_string()
        };
        let mut code = if text.contains("fn main(") {
            if from.is_some() {
                return crate::failure::fail("E1005 --from adds statements to a test's body; give statements, not a module.");
            }
            text
        } else {
            let (imports, body): (Vec<&str>, Vec<&str>) = text.lines().partition(|l| l.starts_with("import "));
            format!("{prelude}{}\n\npub fn main() {{\n{setup}{}\n}}\n", imports.join("\n"), body.join("\n"))
        };
        // the probe lives in test/, so test helpers such as `support` are importable
        for need in missing_imports(&ix, &deps, &code, &code, true) {
            code = format!("{}\n{code}", need.line());
        }
        self.materialize(&ix)?;
        let traced = self.trace_sources(&ix, trace)?;
        let file = self.root.join("test").join(format!("{PROBE}.gleam"));
        let result = (|| -> Result<(bool, String), Failure> {
            for (path, content) in &traced {
                self.rewrite(path, content)?;
            }
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, &code)?;
            self.tool(&["run", "-m", crate::cli::PROBE])
        })();
        // Remove the probe and put traced modules back in every case.
        let _ = fs::remove_file(&file);
        if !traced.is_empty() {
            self.materialize(&ix)?;
        }
        let (ok, output) = result?;
        // Compiler warnings (mostly the probe's unused imports) are noise here.
        let mut out = self.lang().quiet(&output);
        if !trace.is_empty() {
            out = trace_report(&out);
        }
        if !ok {
            if out.contains("Unknown module value") {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out += "Hint: print a value with `echo value`.\n";
            }
            out += &format!("\nThe probe ran as test/{PROBE}.gleam:\n{code}");
        }
        if ok { Ok(out) } else { crate::failure::fail(out) }
    }
    /// `probe --from <fn>`: the imports of its module and its body, so a
    /// probe can start from a test's setup and use its local variables.
    fn probe_from(&self, ix: &Index, name: &str) -> Result<(String, String), Failure> {
        let s = &ix.syms[self.resolve(ix, name)?];
        let open = crate::fn_body_start(s).filter(|_| s.kind == crate::language::Kind::Fn);
        let Some(open) = open else {
            return crate::failure::fail(format!(
                "E1005 {} isn't a function; --from takes a test or another function without arguments.",
                s.qname()
            ));
        };
        if !crate::params(&ix.lang.head(&s.item()), &s.name).is_empty() {
            return crate::failure::fail(format!(
                "E1005 {} takes arguments; --from takes a test or another function without arguments.",
                s.qname()
            ));
        }
        let body = &s.text[open + 1..s.text.rfind('}').unwrap_or(s.text.len())];
        let imports: String =
            self.header(&s.module)?.lines().filter(|l| !l.trim_start().starts_with("//")).map(|l| format!("{l}\n")).collect();
        Ok((imports, format!("{}\n", body.trim_matches('\n'))))
    }
    /// The materialized module files with each traced function wrapped so
    /// that its arguments and result are echoed (D-43). Nothing is written here.
    fn trace_sources(&self, ix: &Index, names: &[&str]) -> Result<Vec<(PathBuf, String)>, Failure> {
        let mut edits = vec![];
        for name in names {
            let i = self.resolve(ix, name)?;
            let s = &ix.syms[i];
            if s.kind != crate::language::Kind::Fn || s.text.contains("@external") {
                return crate::failure::fail(format!(
                    "E1006 {} can't be traced: `probe` is Gleam only and traces only functions written in Gleam.",
                    s.qname()
                ));
            }
            edits.push((i, trace_wrap(s)));
        }
        self.rewritten(ix, &edits)
    }
    /// The materialized files of the modules holding `edits` (symbol, new
    /// text), with those symbols' text replaced. Nothing is written here.
    fn rewritten(&self, ix: &Index, edits: &[(usize, String)]) -> Result<Vec<(PathBuf, String)>, Failure> {
        let mut files: Vec<(String, PathBuf, String)> = vec![];
        for (i, text) in edits {
            let s = &ix.syms[*i];
            let Some(m) = ix.modules.iter().find(|m| m.path == s.module) else { continue };
            if !files.iter().any(|f| f.0 == m.path) {
                let path = self.root.join(&m.dir).join(format!("{}.gleam", m.path));
                let content = fs::read_to_string(&path)?;
                files.push((m.path.clone(), path, content));
            }
            let f = files.iter_mut().find(|f| f.0 == m.path).unwrap();
            f.2 = f.2.replacen(&s.text, text, 1);
        }
        Ok(files.into_iter().map(|(_, p, c)| (p, c)).collect())
    }
    pub(crate) fn work_add(&self, text: &str) -> Out {
        self.db.execute("INSERT INTO work(text) VALUES (?1)", [text])?;
        Ok(format!("Work record #{} added.\n", self.db.last_insert_rowid()))
    }
    pub(crate) fn work_done(&self, id: &str) -> Out {
        let id: i64 = id.trim_start_matches('#').parse().map_err(|_| Failure(format!("Not a work record id: {id}"), 2))?;
        match self.db.execute("UPDATE work SET open = 0 WHERE id = ?1", [id])? {
            0 => crate::failure::fail(format!("No work record #{id}.")),
            _ => Ok(format!("Work record #{id} closed.\n")),
        }
    }
    /// The direct dependency modules, read once until `gleam.toml`, the
    /// resolved package list, or a `.gleam` file under a path dependency's
    /// `src` changes.
    pub(crate) fn dependency_modules(&self) -> Rc<Vec<DependencyModule>> {
        let stamp = |path: &str| fs::metadata(self.root.join(path)).and_then(|m| m.modified()).ok();
        let mut paths = path_dependencies(&self.root);
        paths.sort();
        paths.dedup();
        let mut sources: Vec<(PathBuf, Option<std::time::SystemTime>)> = vec![];
        for path in paths {
            let mut files = vec![];
            if crate::language::gleam::collect_gleam_files(&path.join("src"), &mut files).is_err() {
                continue;
            }
            sources.extend(files.into_iter().map(|file| {
                let stamp = fs::metadata(&file).and_then(|m| m.modified()).ok();
                (file, stamp)
            }));
        }
        sources.sort();
        sources.dedup();
        let key = (stamp("gleam.toml"), stamp("build/packages/packages.toml"), sources);
        let cached = self.deps.borrow().as_ref().filter(|(cached, _)| *cached == key).map(|(_, deps)| deps.clone());
        if let Some(deps) = cached {
            return deps;
        }
        let deps = Rc::new(dependency_modules(&self.root));
        *self.deps.borrow_mut() = Some((key, deps.clone()));
        deps
    }
    pub(crate) fn work_list(&self) -> Out {
        self.status()
    }
}

/// A traced function: the original renamed and private, and a wrapper with
/// its name, visibility and labels that echoes the arguments and the result.
fn trace_wrap(s: &Sym) -> String {
    let inner = format!("{}__solace_traced", s.name);
    let decl = format!("fn {}(", s.name);
    let public = s.text.lines().any(|l| l.trim_start().starts_with(&format!("pub {decl}")));
    let renamed = s.text.replacen(&format!("pub {decl}"), &decl, 1).replacen(&decl, &format!("fn {inner}("), 1);
    let ps = crate::params(&crate::language::gleam::head(&s.item()), &s.name);
    let names: Vec<String> = (0..ps.len()).map(|i| format!("p{i}")).collect();
    let decls: Vec<String> =
        ps.iter().zip(&names).map(|((label, _), n)| label.as_ref().map_or(n.clone(), |l| format!("{l} {n}"))).collect();
    let q = s.qname();
    let args: String = names.iter().map(|n| format!(", {n}")).collect();
    format!(
        "{renamed}\n\n{}fn {}({}) {{\n  echo #(\"solace-call\", \"{q}\"{args})\n  \
         let r = {inner}({})\n  echo #(\"solace-return\", \"{q}\", r)\n  r\n}}",
        if public { "pub " } else { "" },
        s.name,
        decls.join(", "),
        names.join(", ")
    )
}

/// The probe's output with the trace's echoes shown as nested calls and
/// results, values cut short, and at most `MAX` calls.
fn trace_report(output: &str) -> String {
    const MAX: usize = 60;
    const WIDTH: usize = 300;
    let cut = |v: &str| if v.chars().count() > WIDTH { v.chars().take(WIDTH).collect::<String>() + "…" } else { v.to_string() };
    let lines: Vec<&str> = output.lines().collect();
    let (mut out, mut depth, mut calls) = (String::new(), 0usize, 0usize);
    for (i, line) in lines.iter().enumerate() {
        // echo prints its location on the line before the value
        if lines.get(i + 1).is_some_and(|n| n.starts_with("#(\"solace-")) && line.contains(".gleam:") {
            continue;
        }
        let event = line
            .strip_prefix("#(\"solace-call\", \"")
            .map(|r| (true, r))
            .or(line.strip_prefix("#(\"solace-return\", \"").map(|r| (false, r)));
        let Some((call, rest)) = event else {
            out += &format!("{line}\n");
            continue;
        };
        let (name, rest) = rest.split_once('"').unwrap_or((rest, ""));
        let value = rest.strip_prefix(", ").unwrap_or(rest).strip_suffix(')').unwrap_or("");
        if call {
            calls += 1;
        }
        if calls <= MAX {
            let indent = "  ".repeat(if call { depth } else { depth.saturating_sub(1) });
            out += &if call { format!("{indent}→ {name}({})\n", cut(value)) } else { format!("{indent}← {name} = {}\n", cut(value)) };
        }
        depth = if call { depth + 1 } else { depth.saturating_sub(1) };
    }
    if calls > MAX {
        out += &format!("… {} more calls not shown\n", calls - MAX);
    }
    if calls == 0 {
        out += "(no traced function was called)\n";
    }
    out
}

/// The names a module header imports modules as.
fn aliases(header: &str) -> HashSet<String> {
    crate::language::gleam::imports(header).into_iter().map(|i| i.alias).collect()
}

/// Single-line `assert a == b` in a test that read the same local variable on
/// both sides: the expected value is taken from the result it checks (D-43).
fn weak_asserts(text: &str, aliases: &HashSet<String>) -> Vec<String> {
    // root identifiers outside string literals: `q` in `q.total`, not `total`
    let roots = |code: &str| -> HashSet<String> {
        let (mut out, mut word, mut in_str, mut prev) = (HashSet::new(), String::new(), false, ' ');
        let mut after_dot = false;
        for c in code.chars().chain([' ']) {
            if in_str {
                in_str = !(c == '"' && prev != '\\');
            } else if c.is_alphanumeric() || c == '_' {
                if word.is_empty() {
                    after_dot = prev == '.';
                }
                word.push(c);
            } else {
                if word.starts_with(|w: char| w.is_lowercase()) && !after_dot {
                    out.insert(std::mem::take(&mut word));
                }
                word.clear();
                in_str = c == '"';
            }
            prev = c;
        }
        out
    };
    let mut locals = HashSet::new();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("let ") {
            locals.extend(roots(rest.split_once('=').map_or(rest, |p| p.0)).into_iter().filter(|w| w != "assert"));
        }
    }
    let mut out = vec![];
    for line in text.lines() {
        let Some((l, r)) = line.trim().strip_prefix("assert ").and_then(|a| a.split_once(" == ")) else { continue };
        let both: Vec<String> =
            roots(l).intersection(&roots(r)).filter(|w| locals.contains(*w) && !aliases.contains(*w)).cloned().collect();
        if let Some(v) = both.first() {
            out.push(format!(
                "`{}` reads `{v}` on both sides: an expected value taken from the result can't catch a wrong result; \
                 work it out from the task's numbers and the rules.",
                line.trim()
            ));
        }
    }
    out
}

/// Imports needed for `alias.name` references the header doesn't import yet:
/// one project module or dependency by last path segment, the stdlib, or the
/// one module another module already imports as that alias. `test`: the code
/// is in a test module, which may also import test modules and dev modules.
fn missing_imports(ix: &Index, deps: &[DependencyModule], header: &str, text: &str, test: bool) -> Vec<MissingImport> {
    let aliases = aliases(header);
    let mut out: Vec<MissingImport> = vec![];
    for r in crate::language::gleam::references(text) {
        let Ref::Qualified(alias, _) = r else {
            continue;
        };
        if aliases.contains(&alias) {
            continue;
        }
        let ImportMatch::One(need) = import_match(ix, deps, &alias, test) else {
            continue;
        };
        if !out.iter().any(|n| n.module == need.module && n.alias == need.alias) {
            out.push(need);
        }
    }
    out
}

/// From a compiler error about an unknown type or constructor, the import
/// that provides it: (module, item), when exactly one module does.
fn unimported_name(ix: &Index, error: &str) -> Option<(String, String)> {
    let between = |pre: &str, post: &str| {
        error.find(pre).and_then(|a| error[a + pre.len()..].find(post).map(|b| error[a + pre.len()..a + pre.len() + b].to_string()))
    };
    let (name, is_type) = match (between("The type `", "` is not defined"), between("constructor `", "` is not in scope")) {
        (Some(t), _) => (t, true),
        (None, Some(c)) => (c, false),
        _ => return None,
    };
    const STD: &[(&str, &str, bool)] = &[
        ("Option", "gleam/option", true),
        ("Some", "gleam/option", false),
        ("None", "gleam/option", false),
        ("Dict", "gleam/dict", true),
        ("Set", "gleam/set", true),
        ("Order", "gleam/order", true),
        ("Lt", "gleam/order", false),
        ("Eq", "gleam/order", false),
        ("Gt", "gleam/order", false),
    ];
    let item = if is_type { format!("type {name}") } else { name.clone() };
    if let Some((_, m, _)) = STD.iter().find(|(n, _, t)| *n == name && *t == is_type) {
        return Some((m.to_string(), item));
    }
    let mut found: Vec<&str> = ix
        .syms
        .iter()
        .filter(|s| s.kind == crate::language::Kind::Type && crate::language::gleam::head(&s.item()).contains("pub type"))
        .filter(|s| if is_type { s.name == name } else { crate::language::gleam::constructors(&s.text).contains(&name) })
        .map(|s| s.module.as_str())
        .collect();
    crate::unique(&mut found);
    match found.as_slice() {
        [m] => Some((m.to_string(), item)),
        _ => None,
    }
}

/// What a change did to the broken set (D-46): `before` is the broken table
/// as it was. Fixed symbols are named; a symbol broken with the same errors as
/// before gets one line; new errors are printed in full, once per message, and
/// the other errors with that message as one line each (Spec 3 §6).
/// What a name is unique by: types and values are separate namespaces in Gleam.
fn name_key(name: &str, kind: crate::language::Kind) -> (String, bool) {
    (name.to_string(), kind == crate::language::Kind::Type)
}

enum ImportMatch {
    One(MissingImport),
    Many(Vec<MissingImport>),
    Unavailable(Vec<String>),
    None,
}

fn unknown_module_hint(s: &Store, ix: &Index, deps: &[DependencyModule], errors: &[&Diag]) -> Result<String, Failure> {
    for d in errors {
        if d.text.contains("Unknown module value") {
            continue;
        }
        let Some(qualifier) = unknown_module(&d.text) else { continue };
        let Some(module) = d.module.as_deref() else { continue };
        let header = s.header(module)?;
        if aliases(&header).contains(qualifier) {
            continue;
        }
        let test = ix.modules.iter().any(|m| m.path == module && m.dir == "test");
        let current = match header.trim() {
            "" => String::new(),
            h => format!("\n\nIts current header is:\n\n{h}"),
        };
        match import_match(ix, deps, qualifier, test) {
            ImportMatch::One(need) => {
                return Ok(format!(
                    "\n\nTo add it, run `submit --imports {module}` with the module's current imports plus the new line \
                     `{}`.{current}",
                    need.line()
                ));
            }
            ImportMatch::None => {
                return Ok(format!(
                    "\n\nTo add it, run `submit --imports {module}` with the module's current imports plus the new line \
                     `import {qualifier}`.{current}"
                ));
            }
            ImportMatch::Unavailable(paths) => {
                let names: Vec<String> = paths.iter().map(|p| format!("`{p}`")).collect();
                return Ok(format!(
                    "\n\n`{qualifier}` is only available to test code ({}), so it can't be imported into {module}.",
                    names.join(", ")
                ));
            }
            ImportMatch::Many(needs) => {
                let lines: Vec<String> = needs.iter().map(|n| format!("`{}`", n.line())).collect();
                return Ok(format!(
                    "\n\n`{qualifier}` matches more than one module: {}; nothing was added. To pick one, run \
                     `submit --imports {module}` with the module's current imports plus one of those lines.{current}",
                    lines.join(", ")
                ));
            }
        }
    }
    Ok(String::new())
}

/// Whether the server's own saved selection code is newer than the binary
/// running it: its compiled test selection is then out of date.
fn server_needs_every_test(root: &Path, ix: &Index) -> bool {
    crate::project_paths::same_project(root, Path::new(env!("CARGO_MANIFEST_DIR")))
        && selection_source_is_newer(root, running_binary_time(), &ix.modules, &selection_modules(ix))
}

/// Whether any of `files`, relative to `root`, is newer than the captured
/// binary time.
fn source_is_newer_than(root: &Path, built: Option<std::time::SystemTime>, files: impl Iterator<Item = String>) -> bool {
    let Some(built) = built else {
        return false;
    };
    files.filter_map(|rel| fs::metadata(root.join(rel)).and_then(|m| m.modified()).ok()).any(|written| written > built)
}

/// The symbols whose code decides which tests run.
const SELECTION_SYMBOLS: &[&str] = &[
    "reach.test_symbols",
    "reach.closure",
    "reach.hash",
    "store_write.Store::test",
    "index.build",
    "language::rust_refs.graph",
    "language::gleam.references_at",
];

/// The modules that hold the test-selection code, with their submodules.
fn selection_modules(ix: &Index) -> HashSet<String> {
    let roots: Vec<&str> =
        SELECTION_SYMBOLS.iter().filter_map(|name| ix.syms.iter().find(|s| s.qname() == *name)).map(|s| s.module.as_str()).collect();
    ix.modules.iter().map(|m| m.path.as_str()).filter(|path| roots.iter().any(|root| in_module(root, path))).map(str::to_string).collect()
}

/// Whether a selected module's file is newer than `built`; with no selected
/// modules, every module counts.
fn selection_source_is_newer(
    root: &Path,
    built: Option<std::time::SystemTime>,
    modules: &[crate::ModuleRow],
    selected: &HashSet<String>,
) -> bool {
    let files: Vec<String> = modules.iter().filter(|m| selected.is_empty() || selected.contains(&m.path)).map(|m| m.rel()).collect();
    source_is_newer_than(root, built, files.into_iter())
}

static BINARY_TIME: std::sync::OnceLock<Option<std::time::SystemTime>> = std::sync::OnceLock::new();

/// The running binary's modification time, taken once so a rebuild that
/// replaces the file can't hide that this process still runs the old code.
fn running_binary_time() -> Option<std::time::SystemTime> {
    *BINARY_TIME.get_or_init(|| {
        let path = Path::new("/proc/self/exe");
        let path = if path.exists() { path.to_path_buf() } else { std::env::current_exe().ok()? };
        fs::metadata(path).and_then(|m| m.modified()).ok()
    })
}

/// Records the running binary's time at startup, before a rebuild can replace the file.
pub(crate) fn capture_running_binary() {
    let _ = running_binary_time();
}

/// The first line of a `test` reply: what ran, and why a stale server forced
/// every test.
fn test_head(run: usize, tests: usize, names: &[&str], all: bool, stale: bool) -> String {
    if run == tests {
        if stale && !all {
            format!("Ran all {tests} tests: Solace's own code changed since this server started; restart it to select tests again.\n")
        } else {
            format!("Ran all {tests} tests.\n")
        }
    } else if !names.is_empty() {
        format!("Ran {run} of {tests} tests: those that reach {}.\n", names.join(", "))
    } else {
        format!(
            "Ran {run} of {tests} tests; the other {} reach nothing changed since they passed (`test --all` runs every test).\n",
            tests - run
        )
    }
}

/// Whether `path` is `root` or a submodule of it.
fn in_module(root: &str, path: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with("::"))
}
