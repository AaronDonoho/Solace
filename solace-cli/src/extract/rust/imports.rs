//! Carrying imports: the `use` lines a moved item needs in its new module,
//! resolved against where it came from and the names the target already has.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::glob_reach;
use super::{
    Carry, HiddenThreadLocal, TestGroup, TestModule, enclosing_module, leaf, module_names, names_need_glob, rewrite_targets, splice,
    target_names, test_container, thread_local_paths,
};
use crate::Index;
use crate::failure::Failure;
use crate::language;

/// The parent and crate-root imports a newly created module needs for names
/// the moved code mentions only in a pattern, which the reference graph leaves
/// out of its uses.
pub(super) fn parent_imports(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    references: &language::rust_refs::References<'_>,
    newheaders: &HashMap<String, String>,
) -> Result<Vec<Carry>, Failure> {
    let targets = rewrite_targets(ix, moved);
    let hidden = thread_local_paths(ix, ids, &module_names(ix, ids));
    let mut carried = vec![];
    let mut seen = HashSet::new();
    for i in (0..ix.syms.len()).filter(|&i| moved[i] && newmod[i] == target) {
        let from = ix.syms[i].module.clone();
        let mut sites: Vec<(String, language::rust_refs::ImportSite)> =
            language::rust_refs::import_sites(&ix.syms, &ix.modules, &from, true)?.into_iter().map(|site| (from.clone(), site)).collect();
        sites.extend(glob_reach::super_glob_imports(ix, ids, newheaders, &from));
        let crate_root = glob_reach::reaches_crate_root(ix, ids, newheaders, &from);
        for name in &references.mentioned[i] {
            if !parent_name_needed(ix, &targets, name) {
                continue;
            }
            let found = sites.iter().find(|(_, site)| !site.glob && &site.local == name);
            let (owner, mut site) = match found {
                Some((owner, site)) => (owner.clone(), site.clone()),
                None if crate_root && crate_root_item(ix, name) => {
                    ("crate".to_string(), language::rust_refs::ImportSite::named(vec![name.clone()]))
                }
                None => continue,
            };
            site.path = absolute_import_path(ix, &owner, &site.path);
            if hidden_thread_local(&hidden, &site.path) {
                continue;
            }
            if seen.insert((owner.clone(), site.local.clone(), site.path.clone(), site.glob)) {
                carried.push(Carry { from: owner, site, rename: true });
            }
        }
    }
    Ok(carried)
}

/// Adds the carried imports to `target`, aliasing one whose local name the
/// target already uses, and renames its mentions in the moved code.
pub(super) fn carry_imports(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    carried: &[Carry],
    refs: &language::rust_refs::References<'_>,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<Vec<Vec<String>>, Failure> {
    let mut header = newheaders.get(target).cloned().unwrap_or_default();
    let mut names = super::target_names_after_move(ix, moved, newmod, &header, target)?;
    let mut existing = target_import_sites(ix, &header, target)?;
    let mut aliases: HashMap<(String, Vec<String>, bool), String> = HashMap::new();
    let mut added_globs = vec![];
    for carry in carried {
        let key = import_key(&carry.site);
        let covered = existing.iter().any(|site| import_key(site) == key && covers(site, &carry.site));
        if !covered {
            let ungated = gates(&carry.site.attributes).is_empty();
            let changed = if ungated { drop_gated_imports(ix, target, &key, &mut header, newtext)? } else { false };
            if !changed {
                let (line, alias) = carried_import_line(ix, target, carry, &mut names, &mut added_globs);
                if let Some(alias) = alias {
                    aliases.insert(key.clone(), alias);
                }
                header = add_import_line(&header, &line);
            }
            if changed {
                existing.retain(|site| import_key(site) != key || gates(&site.attributes).is_empty());
            }
            existing.push(carry.site.clone());
        }
        if carry.rename
            && let Some(alias) = aliases.get(&key)
        {
            for i in (0..ix.syms.len()).filter(|&i| moved[i] && ix.syms[i].module == carry.from) {
                newtext[i] = language::rust_refs::rename_name(refs, i, &carry.site.local, alias);
            }
        }
    }
    newheaders.insert(target.to_string(), header);
    Ok(added_globs)
}

pub(super) fn carry_test_imports(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    ids: &HashMap<&str, usize>,
    refs: &language::rust_refs::References<'_>,
    groups: &[TestGroup],
    test: &TestModule,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<Vec<Vec<String>>, Failure> {
    let tests: HashSet<usize> = groups.iter().flat_map(|g| g.children.iter().copied()).collect();
    let mut carried: Vec<Carry> = vec![];
    let mut seen = HashSet::new();
    let mut ancestor_named: HashMap<String, HashSet<String>> = HashMap::new();
    for &i in &tests {
        let from = ix.syms[i].module.clone();
        let container = test_container(ix, ids, i);
        let super_from =
            container.map_or_else(|| from.clone(), |container| format!("{}::{}", ix.syms[container].module, ix.syms[container].name));
        let mut sites = vec![];
        if let Some(container) = container {
            sites.extend(test_container_imports(ix, container)?);
        } else if let Some(row) = ix.modules.iter().find(|row| row.path == from) {
            sites.extend(
                language::rust_refs::import_sites_in(&row.header).map_err(|e| Failure(format!("imports in {from} don't parse: {e}"), 1))?,
            );
        }
        let names = super_glob_names(ix, ids, refs, &tests, &from, container);
        let moved_names: HashSet<String> = names.iter().cloned().collect();
        for site in sites {
            if glob_reach::is_super_glob(&site) {
                for name in &names {
                    carry_super_glob(ix, ids, newheaders, &test.module, &super_from, name, &mut carried, &mut seen);
                }
                continue;
            }
            if site.path.first().is_some_and(|part| part == "super") {
                continue;
            }
            let used = tests.iter().any(|&j| ix.syms[j].module == from && refs.mentioned[j].contains(&site.local));
            if !site.glob && !used {
                continue;
            }
            if seen.insert((from.clone(), site.local.clone(), site.path.clone(), site.glob)) {
                carried.push(Carry { from: from.clone(), site, rename: true });
            }
        }
        for (ancestor, site) in glob_reach::super_glob_imports(ix, ids, newheaders, &super_from) {
            if glob_reach::is_super_glob(&site) || site.path.first().is_some_and(|part| part == "super") {
                continue;
            }
            if site.glob {
                if !ancestor_glob_needed(ix, ids, &ancestor, &moved_names, &mut ancestor_named) {
                    continue;
                }
            } else if !moved_names.contains(&site.local) {
                continue;
            }
            if seen.insert((ancestor.clone(), site.local.clone(), site.path.clone(), site.glob)) {
                carried.push(Carry { from: ancestor, site, rename: true });
            }
        }
    }
    if test.created {
        for carry in &mut carried {
            carry.site.path = rooted_path(ix, "crate", &test.module, &carry.site.path);
        }
    }
    if carried.is_empty() {
        return Ok(vec![]);
    }
    carry_imports(ix, moved, newmod, &test.module, &carried, refs, newtext, newheaders)
}

/// Roots the crate-root paths in moved `use` items, so a test container keeps
/// resolving the modules it saw beside its old parent.
pub(super) fn root_moved_use_imports(ix: &Index, moved: &[bool], newmod: &[String], newtext: &mut [String]) {
    for i in
        (0..ix.syms.len()).filter(|&i| moved[i] && matches!(ix.syms[i].kind_text.as_str(), "use" | "mod") && newmod[i] != ix.syms[i].module)
    {
        newtext[i] = root_import_text(ix, &newtext[i]);
    }
}

/// `text`, an import item, with `crate::` in front of each path that names a
/// crate-root module or item.
fn root_import_text(ix: &Index, text: &str) -> String {
    let Ok(sites) = language::rust_refs::import_sites_in(text) else { return text.to_string() };
    let starts: BTreeSet<usize> = sites
        .into_iter()
        .filter_map(|site| {
            let first = site.path.first()?;
            (!matches!(first.as_str(), "crate" | "self" | "super") && crate_root_name(ix, first)).then_some(site.range.start)
        })
        .collect();
    if starts.is_empty() {
        return text.to_string();
    }
    splice(text, starts.into_iter().map(|at| (at..at, "crate::".to_string())).collect())
}

pub(super) fn rooted_path(ix: &Index, from: &str, target: &str, path: &[String]) -> Vec<String> {
    if from != "crate" || target == "crate" {
        return path.to_vec();
    }
    absolute_import_path(ix, from, path)
}

/// The absolute crate path `path` names when read from `from`, or `path`
/// unchanged when it names another crate or is already absolute.
pub(super) fn absolute_import_path(ix: &Index, from: &str, path: &[String]) -> Vec<String> {
    let Some(first) = path.first() else { return path.to_vec() };
    if first == "crate" {
        return path.to_vec();
    }
    if first == "self" || first == "super" {
        let mut base: Vec<String> = if from == "crate" { vec![] } else { from.split("::").map(|part| part.to_string()).collect() };
        let rest = if first == "self" {
            &path[1..]
        } else {
            let up = path.iter().take_while(|part| part.as_str() == "super").count();
            if up > base.len() {
                return path.to_vec();
            }
            base.truncate(base.len() - up);
            &path[up..]
        };
        let mut absolute = Vec::with_capacity(base.len() + rest.len() + 1);
        absolute.push("crate".to_string());
        absolute.extend(base);
        absolute.extend(rest.iter().cloned());
        return absolute;
    }
    if from != "crate" && module_declares(ix, from, first) {
        let mut absolute = Vec::with_capacity(from.split("::").count() + path.len() + 1);
        absolute.push("crate".to_string());
        absolute.extend(from.split("::").map(|part| part.to_string()));
        absolute.extend(path.iter().cloned());
        return absolute;
    }
    if !crate_root_name(ix, first) {
        return path.to_vec();
    }
    let mut absolute = Vec::with_capacity(path.len() + 1);
    absolute.push("crate".to_string());
    absolute.extend(path.iter().cloned());
    absolute
}

/// Whether `module` declares `name` as a child module or an item.
fn module_declares(ix: &Index, module: &str, name: &str) -> bool {
    ix.modules.iter().any(|row| row.path == format!("{module}::{name}")) || module_item(ix, module, name)
}

/// Whether `module` declares an item named `name`.
pub(super) fn module_item(ix: &Index, module: &str, name: &str) -> bool {
    ix.syms
        .iter()
        .any(|sym| sym.module == module && sym.parent.is_none() && sym.kind_text != "use" && (sym.name == name || leaf(&sym.name) == name))
}

/// Whether `name` is an item the crate root declares, not one it imports.
fn crate_root_item(ix: &Index, name: &str) -> bool {
    module_item(ix, "crate", name)
}

fn crate_root_name(ix: &Index, name: &str) -> bool {
    ix.modules.iter().any(|module| module.path == name) || crate_root_item(ix, name)
}

/// Whether a moved mention still needs an import: it is no module qualifier,
/// and no rewritten target already gives it a name.
fn parent_name_needed(ix: &Index, targets: &BTreeSet<usize>, name: &str) -> bool {
    if ix.modules.iter().any(|module| module.path == name) {
        return false;
    }
    !targets.iter().any(|&t| ix.syms[t].name == name || leaf(&ix.syms[t].name) == name)
}

/// Whether an import path names a static declared inside a `thread_local!`
/// block, which `qualify_thread_locals` names directly instead.
fn hidden_thread_local(hidden: &[HiddenThreadLocal], path: &[String]) -> bool {
    let rooted = path.join("::");
    hidden.iter().any(|tl| {
        let full = if tl.module == "crate" { tl.name.clone() } else { format!("{}::{}", tl.module, tl.name) };
        rooted == full || rooted == format!("crate::{full}") || rooted.ends_with(&format!("::{full}"))
    })
}

fn test_container_imports(ix: &Index, container: usize) -> Result<Vec<language::rust_refs::ImportSite>, Failure> {
    let mut sites = language::rust_refs::import_sites_in(&ix.syms[container].text)
        .map_err(|e| Failure(format!("imports in {} don't parse: {e}", ix.syms[container].qname()), 1))?;
    let id = ix.syms[container].id.as_str();
    for symbol in ix.syms.iter().filter(|s| s.parent.as_deref() == Some(id) && s.kind_text == "use") {
        sites.extend(
            language::rust_refs::import_sites_in(&symbol.text)
                .map_err(|e| Failure(format!("imports in {} don't parse: {e}", ix.syms[container].name), 1))?,
        );
    }
    Ok(sites)
}

/// The names the moved tests of `from` mention, for a glob import to carry.
fn super_glob_names(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    refs: &language::rust_refs::References<'_>,
    tests: &HashSet<usize>,
    from: &str,
    container: Option<usize>,
) -> Vec<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for &j in tests {
        let in_group = match container {
            Some(container) => test_container(ix, ids, j) == Some(container),
            None => ix.syms[j].module == from,
        };
        if in_group {
            names.extend(refs.mentioned[j].iter().cloned());
        }
    }
    names.into_iter().collect()
}

/// Whether an ancestor's glob must move with the names the moved test uses.
fn ancestor_glob_needed(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    ancestor: &str,
    moved_names: &HashSet<String>,
    named: &mut HashMap<String, HashSet<String>>,
) -> bool {
    let named = named
        .entry(ancestor.to_string())
        .or_insert_with(|| module_imports(ix, ids, ancestor).into_iter().filter(|site| !site.glob).map(|site| site.local).collect());
    names_need_glob(ix, ancestor, moved_names, named)
}

/// Carries a module name the source saw through `use super::*`, unless the
/// target test file already sees it the same way.
fn carry_super_glob(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    newheaders: &HashMap<String, String>,
    target: &str,
    from: &str,
    name: &str,
    carried: &mut Vec<Carry>,
    seen: &mut HashSet<(String, String, Vec<String>, bool)>,
) {
    let Some((module, at)) = glob_reach::super_glob_module(ix, ids, newheaders, from, name) else { return };
    if glob_reach::sees_through_super_glob(ix, ids, newheaders, target, &at) {
        return;
    }
    let path: Vec<String> = std::iter::once("crate".to_string()).chain(module.split("::").map(str::to_string)).collect();
    let site = language::rust_refs::ImportSite::named(path);
    if seen.insert((from.to_string(), site.local.clone(), site.path.clone(), site.glob)) {
        carried.push(Carry { from: from.to_string(), site, rename: true });
    }
}

fn import_keys(ix: &Index, header: &str, target: &str) -> Result<HashSet<(String, Vec<String>, bool)>, Failure> {
    Ok(target_import_sites(ix, header, target)?.into_iter().map(|site| import_key(&site)).collect())
}

fn fresh_name(local: &str, taken: &HashSet<String>) -> String {
    let mut n = 2;
    loop {
        let name = format!("{local}_{n}");
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

/// Adds the import rustc names for a trait a moved item uses through a type
/// path, which name matching cannot see. Returns whether a header changed.
pub(super) fn add_missing_trait_imports(
    s: &crate::Store,
    ix: &Index,
    before: &HashMap<String, String>,
    diags: &[crate::diagnostics::Diag],
) -> Result<bool, Failure> {
    let mut added = false;
    let mut seen = HashSet::new();
    for diag in crate::rust_check::new_errors(before, diags) {
        let Some(module) = diag.module.as_deref() else { continue };
        for path in suggested_trait_imports(&diag.text) {
            if !seen.insert((module.to_string(), path.clone())) {
                continue;
            }
            let header = s.header(module)?;
            if import_keys(ix, &header, module)?.iter().any(|(_, imported, glob)| !*glob && imported == &path) {
                continue;
            }
            let taken = target_names(ix, &header, module, |_, symbol| symbol.module == module)?;
            let local = path.last().map(String::as_str).unwrap_or("");
            let line = if taken.contains(local) { format!("use {} as _;", path.join("::")) } else { format!("use {};", path.join("::")) };
            s.set_header(module, &add_import_line(&header, &line))?;
            added = true;
        }
    }
    Ok(added)
}

/// The imports rustc offers for a trait missing from scope.
pub(super) fn suggested_trait_imports(text: &str) -> Vec<Vec<String>> {
    if !(text.contains("trait") && text.contains("not in scope")) {
        return vec![];
    }
    let lines: Vec<&str> = text.lines().filter_map(|line| line.find("+ use ").map(|at| &line[at + "+ ".len()..])).collect();
    let [line] = lines.as_slice() else { return vec![] };
    let Some(end) = line.find(';') else { return vec![] };
    let Ok(sites) = language::rust_refs::import_sites_in(&line[..=end]) else { return vec![] };
    sites.into_iter().find(|site| !site.glob).map(|site| vec![site.path]).unwrap_or_default()
}

/// The imports written in `module`, including an inline module's own `use`s.
pub(super) fn module_imports(ix: &Index, ids: &HashMap<&str, usize>, module: &str) -> Vec<language::rust_refs::ImportSite> {
    if ix.modules.iter().any(|m| m.path == module) {
        return language::rust_refs::import_sites(&ix.syms, &ix.modules, module, true).unwrap_or_default();
    }
    if let Some(container) = ix.syms.iter().find(|sym| sym.kind_text == "mod" && format!("{}::{}", sym.module, sym.name) == module) {
        return language::rust_refs::import_sites_in(&container.text).unwrap_or_default();
    }
    (0..ix.syms.len())
        .filter(|&i| ix.syms[i].kind_text == "use" && enclosing_module(ix, ids, i) == module)
        .flat_map(|i| language::rust_refs::import_sites_in(&ix.syms[i].text).unwrap_or_default())
        .collect()
}

pub(super) fn add_import_line(header: &str, line: &str) -> String {
    let body = header.trim_end();
    if body.is_empty() { format!("{line}\n\n") } else { format!("{body}\n{line}\n\n") }
}

/// The target's imports, from its header and its top-level `use` symbols.
fn target_import_sites(ix: &Index, header: &str, target: &str) -> Result<Vec<language::rust_refs::ImportSite>, Failure> {
    let mut sites =
        language::rust_refs::import_sites_in(header).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))?;
    for symbol in ix.syms.iter().filter(|s| s.module == target && s.parent.is_none() && s.kind_text == "use") {
        sites.extend(
            language::rust_refs::import_sites_in(&symbol.text).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))?,
        );
    }
    Ok(sites)
}

fn import_key(site: &language::rust_refs::ImportSite) -> (String, Vec<String>, bool) {
    (site.local.clone(), site.path.clone(), site.glob)
}

/// The `cfg` attributes that decide when an import is available.
fn gates(attributes: &str) -> Vec<String> {
    language::rust_refs::attrs_and_rest(attributes).0.into_iter().filter(|attr| is_config_gate(attr)).map(str::to_string).collect()
}

/// Whether `existing` provides the binding `carry` needs in every
/// configuration where `carry` is needed.
pub(super) fn covers(existing: &language::rust_refs::ImportSite, carry: &language::rust_refs::ImportSite) -> bool {
    let existing_gates = gates(&existing.attributes);
    existing_gates.is_empty() || existing_gates == gates(&carry.attributes)
}

/// Drops the target's `cfg` gates on `key`: the first copy becomes ungated and
/// the rest go, so no configuration binds the name twice.
fn drop_gated_imports(
    ix: &Index,
    target: &str,
    key: &(String, Vec<String>, bool),
    header: &mut String,
    newtext: &mut [String],
) -> Result<bool, Failure> {
    let (kept, mut changed, mut ungated) = ungate_import(header, target, key, true)?;
    *header = kept;
    for (i, _) in ix.syms.iter().enumerate().filter(|(_, s)| s.module == target && s.parent.is_none() && s.kind_text == "use") {
        let (kept, did, did_ungate) = ungate_import(&newtext[i], target, key, !ungated)?;
        newtext[i] = kept;
        changed |= did;
        ungated |= did_ungate;
    }
    Ok(changed)
}

fn is_config_gate(attr: &str) -> bool {
    attr.trim_start().strip_prefix("#[cfg").is_some_and(|rest| rest.trim_start().starts_with('('))
}

/// `text` with its gated copies of `key`: one ungated when `ungate`, and the
/// rest dropped. The bools say whether the text changed and whether an ungated
/// copy was made.
pub(super) fn ungate_import(
    text: &str,
    module: &str,
    key: &(String, Vec<String>, bool),
    ungate: bool,
) -> Result<(String, bool, bool), Failure> {
    let sites = language::rust_refs::import_sites_in(text).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    let gated: Vec<language::rust_refs::ImportSite> =
        sites.into_iter().filter(|site| import_key(site) == *key && !gates(&site.attributes).is_empty()).collect();
    if gated.is_empty() {
        return Ok((text.to_string(), false, false));
    }
    if !ungate {
        return Ok((remove_gated_copies(text, module, key, None)?, true, false));
    }
    let chosen = &gated[0];
    if chosen.outside == 0
        && let Some(ungated) = without_cfg(text, &chosen.range)
    {
        return Ok((remove_gated_copies(&ungated, module, key, None)?, true, true));
    }
    let line = ungated_import_line(text, chosen);
    Ok((remove_gated_copies(text, module, key, Some((chosen.range.clone(), line)))?, true, true))
}

/// `text` without the `cfg` attributes on the `use` whose tree holds `range`.
fn without_cfg(text: &str, range: &std::ops::Range<usize>) -> Option<String> {
    let (item, tree) = sole_tree_item(text, range)?;
    let head = &text[item.start..tree.start];
    let (attrs, _) = language::rust_refs::attrs_and_rest(head);
    if !attrs.iter().any(|attr| is_config_gate(attr)) {
        return None;
    }
    let mut kept = String::new();
    let mut at = 0;
    for attr in attrs {
        let start = head[at..].find(attr).map_or(at, |i| at + i);
        kept.push_str(&head[at..start]);
        if !is_config_gate(attr) {
            kept.push_str(attr);
        }
        at = start + attr.len();
    }
    kept.push_str(&head[at..]);
    Some(format!("{}{}{}", &text[..item.start], kept.trim_start(), &text[tree.start..]))
}

fn carried_import_line(
    ix: &Index,
    target: &str,
    carry: &Carry,
    names: &mut HashSet<String>,
    added_globs: &mut Vec<Vec<String>>,
) -> (String, Option<String>) {
    let rooted = rooted_path(ix, &carry.from, target, &carry.site.path);
    let path = rooted.join("::");
    let (line, alias) = if carry.site.glob {
        added_globs.push(rooted);
        (format!("use {path}::*;"), None)
    } else {
        let (local, alias) = if names.contains(&carry.site.local) {
            let fresh = fresh_name(&carry.site.local, names);
            (fresh.clone(), Some(fresh))
        } else {
            (carry.site.local.clone(), None)
        };
        names.insert(local.clone());
        let line = if local == leaf(&path) { format!("use {path};") } else { format!("use {path} as {local};") };
        (line, alias)
    };
    (super::attributed_import_line(&carry.site.attributes, line), alias)
}

/// `text` without the gated copies of `key`, adding `line` for the copy at
/// `add` when one is given.
fn remove_gated_copies(
    text: &str,
    module: &str,
    key: &(String, Vec<String>, bool),
    add: Option<(std::ops::Range<usize>, String)>,
) -> Result<String, Failure> {
    let sites = language::rust_refs::import_sites_in(text).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    let mut line_removals = vec![];
    let mut members = vec![];
    for site in sites.into_iter().filter(|site| import_key(site) == *key && !gates(&site.attributes).is_empty()) {
        if site.outside == 0 && sole_tree_item(text, &site.range).is_some() {
            if let Some(range) = super::use_line_removal(text, &site.range) {
                line_removals.push(range);
            }
        } else {
            members.push((site.range, site.alias));
        }
    }
    super::apply_import_edits(text, vec![], line_removals, members, add.into_iter().collect(), module)
}

/// The ungated `use` line for one grouped import `site`.
fn ungated_import_line(text: &str, site: &language::rust_refs::ImportSite) -> String {
    let path = site.path.join("::");
    let line = if site.glob {
        format!("use {path}::*;")
    } else if site.local == leaf(&path) {
        format!("use {path};")
    } else {
        format!("use {path} as {};", site.local)
    };
    let line = format!("{}{line}", super::use_visibility(text, &site.range));
    super::attributed_import_line(&without_cfg_attributes(&site.attributes), line)
}

/// `attributes` without its `cfg` gates.
fn without_cfg_attributes(attributes: &str) -> String {
    language::rust_refs::attrs_and_rest(attributes).0.into_iter().filter(|attr| !is_config_gate(attr)).collect::<Vec<_>>().join("\n")
}

/// The import item holding `range`, when it binds exactly one tree.
fn sole_tree_item(text: &str, range: &std::ops::Range<usize>) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let items = language::rust::import_ranges(text).ok()?;
    let (item, tree) = items.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end)?;
    let bindings = language::rust_refs::import_sites_in(&text[item.clone()]).ok()?;
    (bindings.len() == 1).then_some((item, tree))
}
