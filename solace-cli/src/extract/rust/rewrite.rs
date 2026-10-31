//! Rust import and mention rewriting: the `use` lines and paths a moved item changes.

use super::*;
use crate::Index;
use crate::extract::rust::declaration_visibility;
use crate::failure::Failure;
use crate::language::rust_refs::Site;
use crate::language::rust_refs::use_sites;
use crate::extract::rust::MovedMentions;
use crate::extract::rust::test_module_path;
use crate::extract::rust::test_files::belongs_in_test_file;
#[cfg(test)]
#[path = "rewrite_tests.rs"]
mod tests;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;
use crate::extract::rust::container_key;
use crate::extract::rust::leaf;
use crate::extract::rust::Rewrite;
use crate::extract::rust::names_need_glob;
use crate::extract::rust::is_prelude_name;
use crate::extract::rust::Carry;
use crate::extract::rust::LocalImport;
use crate::extract::rust::is_mod_declaration;
use crate::extract::rust::impl_owner_symbol;
use crate::extract::rust::staying_in;
use crate::extract::rust::promote_hidden_static;
use crate::extract::rust::target_names_after_move;
use crate::extract::rust::in_module_after_move;
use crate::extract::rust::module_use_symbols;
use crate::extract::rust::glob_reach::reaching_modules;
use crate::extract::rust::glob_reach::descendants_need_glob;
use crate::extract::rust::glob_reach::add_descendant_import;
use crate::extract::rust::glob_reach::add_glob_descendant_imports;
use crate::extract::rust::glob_reach::named_imports;
use crate::extract::rust::imports::add_import_line;
use crate::extract::rust::plan::make_visible;
use crate::extract::rust::plan::rewrite_targets;
use crate::extract::rust::plan::can_promote;
use crate::extract::rust::plan::moved_sources;
use crate::failure::fail;
use crate::extract::rust::glob_reach::add_module_import;
use crate::extract::rust::plan::RustMove;

pub(crate) type Container = (Option<usize>, String);

pub(crate) fn container_text(key: &Container, headers: &HashMap<String, String>, text: &[String]) -> String {
    match key.0 {
        Some(i) => text[i].clone(),
        None => headers.get(&key.1).cloned().unwrap_or_default(),
    }
}

pub(crate) fn set_container_text(key: &Container, headers: &mut HashMap<String, String>, text: &mut [String], body: String) {
    match key.0 {
        Some(i) => text[i] = body,
        None => {
            headers.insert(key.1.clone(), body);
        }
    }
}

pub(crate) fn full_path(module: &str, name: &str) -> String {
    if module == "crate" {
        format!("crate::{name}")
    } else if let Some(rest) = module.strip_prefix("integration::") {
        let mut parts = rest.split("::");
        let _root = parts.next();
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() { format!("crate::{name}") } else { format!("crate::{}::{name}", rest.join("::")) }
    } else if module.starts_with("crate::") {
        format!("{module}::{name}")
    } else {
        format!("crate::{module}::{name}")
    }
}

pub(crate) fn last_root_sep(ix: &Index, module: &str) -> Option<String> {
    ix.syms.iter().filter(|s| s.module == module && s.parent.is_none()).last().and_then(|s| s.sep.clone())
}

pub(crate) fn replace_range(text: &str, range: Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &text[..range.start], &text[range.end..])
}

pub(crate) fn splice(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|e| e.0.start);
    let mut out = String::new();
    let mut at = 0;
    for (range, with) in edits {
        if range.start < at {
            continue;
        }
        out.push_str(&text[at..range.start]);
        out.push_str(&with);
        at = range.end;
    }
    out.push_str(&text[at..]);
    out
}

pub(crate) fn is_public(text: &str) -> bool {
    !matches!(declaration_visibility(text), Visibility::Private | Visibility::Restricted)
}

pub(crate) fn declaration(text: &str) -> Option<(usize, &str)> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let mut rest = line.trim_start();
        let mut at = offset + indent;
        loop {
            if rest.starts_with("//") {
                rest = "";
                break;
            }
            if rest.starts_with("#[")
                && let Some(end) = rest.find(']')
            {
                let used = end + 1;
                at += used;
                rest = &rest[used..];
                let ws = rest.len() - rest.trim_start().len();
                at += ws;
                rest = rest.trim_start();
                continue;
            }
            break;
        }
        if !rest.trim().is_empty() {
            return Some((at, rest));
        }
        offset += line.len();
    }
    None
}

pub(crate) fn is_reexport(text: &str, range: &Range<usize>) -> bool {
    !use_visibility(text, range).is_empty()
}

/// The visibility written before the `use` whose tree holds `range`, such as `pub ` or `pub(crate) `.
pub(crate) fn use_visibility(text: &str, range: &Range<usize>) -> String {
    let Ok(ranges) = crate::language::rust::import_ranges(text) else {
        return String::new();
    };
    let Some((item, tree)) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end) else {
        return String::new();
    };
    let head = text[item.start..tree.start].trim();
    let Some(vis) = head.strip_suffix("use") else {
        return String::new();
    };
    let vis = vis.trim();
    let vis = vis.rsplit_once(']').map_or(vis, |(_, after)| after.trim());
    if vis == "pub" || (vis.starts_with("pub(") && vis.ends_with(')')) { format!("{vis} ") } else { String::new() }
}

pub(crate) fn all_use_sites(ix: &Index) -> Result<Vec<(String, Site)>, Failure> {
    let mut out = vec![];
    for m in &ix.modules {
        for site in use_sites(&ix.syms, &ix.modules, &m.path)? {
            out.push((m.path.clone(), site));
        }
    }
    Ok(out)
}

/// Whether the `use` whose tree holds `range` carries an `unused_imports` allowance.
pub(crate) fn allows_unused(text: &str, range: &Range<usize>) -> bool {
    let Ok(ranges) = crate::language::rust::import_ranges(text) else {
        return true;
    };
    ranges
        .into_iter()
        .find(|(_, tree)| tree.start <= range.start && range.end <= tree.end)
        .is_none_or(|(item, tree)| text[item.start..tree.start].contains("unused_imports"))
}

/// The range `remove_member` deletes: one member and one adjacent comma.
pub(crate) fn member_removal(text: &str, range: &Range<usize>, alias: Option<&Range<usize>>) -> Range<usize> {
    let start = range.start;
    let end = alias.map_or(range.end, |a| a.end);
    let after = &text[end..];
    if let Some(pos) = after.find(|c| c == ',' || c == '}')
        && after.as_bytes()[pos] == b','
    {
        return start..end + pos + 1;
    }
    let before = &text[..start];
    if let Some(pos) = before.rfind(',') {
        return pos..end;
    }
    start..end
}

/// The line of the import item that holds `range`.
pub(crate) fn use_line_removal(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let ranges = crate::language::rust::import_ranges(text).ok()?;
    let (item, _) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end)?;
    Some(use_item_line_range(text, &item))
}

pub(crate) fn use_item_line_range(text: &str, item: &Range<usize>) -> Range<usize> {
    let line_start = text[..item.start].rfind('\n').map_or(0, |i| i + 1);
    let mut start = if text[line_start..item.start].trim().is_empty() { line_start } else { item.start };
    loop {
        if start == 0 {
            break;
        }
        let prev_end = start - 1;
        let prev_start = text[..prev_end].rfind('\n').map_or(0, |i| i + 1);
        let line = text[prev_start..prev_end].trim();
        if line.starts_with("#[") && line.ends_with(']') {
            start = prev_start;
        } else {
            break;
        }
    }
    let rest = &text[item.end..];
    let end = match rest.find('\n') {
        Some(i) if rest[..i].trim().is_empty() => item.end + i + 1,
        _ => item.end,
    };
    start..end
}

/// The removals for several members of one import container: the whole line
/// when every binding in an item goes, otherwise the members and their commas.
fn member_edits(
    original: &str,
    members: &[(Range<usize>, Option<Range<usize>>)],
    module: &str,
) -> Result<Vec<(Range<usize>, String)>, Failure> {
    if members.is_empty() {
        return Ok(vec![]);
    }
    let merged = merge_ranges(members.iter().map(|(range, alias)| member_removal(original, range, alias.as_ref())).collect());
    let items = crate::language::rust::import_ranges(original).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    let mut edits = vec![];
    for (item, tree) in &items {
        let inside: Vec<Range<usize>> = merged.iter().filter(|range| item.start <= range.start && range.end <= item.end).cloned().collect();
        if inside.is_empty() {
            continue;
        }
        let all = all_bindings_removed(original, item, &inside).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
        if all {
            edits.push((use_item_line_range(original, item), String::new()));
        } else if let Some(collapsed) = collapsed_single_member_item(original, item, tree, &inside) {
            edits.push((item.clone(), collapsed));
        } else {
            edits.extend(inside.into_iter().map(|range| (range, String::new())));
        }
    }
    Ok(edits)
}

pub(crate) fn new_module(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], target: &str, i: usize) -> String {
    if !moved[i] {
        return ix.syms[i].module.clone();
    }
    if belongs_in_test_file(ix, ids, i) && ix.syms[i].kind_text != "mod" { test_module_path(ix, target) } else { target.to_string() }
}

/// `original` with all its import edits applied: each one is worked out from
/// `original` before any shift, and the edits are applied right to left.
pub(crate) fn apply_import_edits(
    original: &str,
    replacements: Vec<(Range<usize>, String)>,
    line_removals: Vec<Range<usize>>,
    members: Vec<(Range<usize>, Option<Range<usize>>)>,
    additions: Vec<(Range<usize>, String)>,
    module: &str,
) -> Result<String, Failure> {
    let mut edits = replacements;
    edits.extend(line_removals.into_iter().map(|range| (range, String::new())));
    edits.extend(member_edits(original, &members, module)?);
    if !additions.is_empty() {
        let items = crate::language::rust::import_ranges(original).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
        let removed: Vec<Range<usize>> = edits.iter().filter(|(_, with)| with.is_empty()).map(|(range, _)| range.clone()).collect();
        // Later additions are applied first so equal-position lines keep collection order.
        for (range, line) in additions.into_iter().rev() {
            edits.push(use_line_edit(original, &items, &removed, &range, &line));
        }
    }
    edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then_with(|| (a.0.start == a.0.end).cmp(&(b.0.start == b.0.end))));
    let mut text = original.to_string();
    for (range, with) in edits {
        text = replace_range(&text, range, &with);
    }
    if !original.starts_with('\n') {
        text = text.trim_start_matches('\n').to_string();
    }
    Ok(text)
}

/// The edit that puts `line` at the import item holding `range`: after the
/// item, or in its place when the whole item is removed.
fn use_line_edit(
    original: &str,
    items: &[(Range<usize>, Range<usize>)],
    removed: &[Range<usize>],
    range: &Range<usize>,
    line: &str,
) -> (Range<usize>, String) {
    let Some((item, _)) = items.iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end) else {
        return (original.len()..original.len(), format!("{line}\n"));
    };
    let item_lines = use_item_line_range(original, item);
    let indent = line_indent(original, item.start);
    if removed.iter().any(|r| r.start <= item_lines.start && item_lines.end <= r.end) {
        (item_lines.start..item_lines.start, format!("{indent}{line}\n"))
    } else {
        (item.end..item.end, format!("\n{indent}{line}"))
    }
}

/// The whitespace before `at`'s line content.
fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at].chars().take_while(|c| c.is_whitespace()).collect()
}

pub(crate) fn attributed_import_line(attributes: &str, line: String) -> String {
    if attributes.is_empty() { line } else { format!("{attributes} {line}") }
}

/// The item holding `tree` with `removals` applied and any group left with one
/// member collapsed, when that collapse changes it.
fn collapsed_single_member_item(original: &str, item: &Range<usize>, tree: &Range<usize>, removals: &[Range<usize>]) -> Option<String> {
    let mut tree_text = original[tree.clone()].to_string();
    for range in removals.iter().rev() {
        tree_text.replace_range((range.start - tree.start)..(range.end - tree.start), "");
    }
    let collapsed = collapse_single_member_groups(&tree_text);
    (collapsed != tree_text).then(|| format!("{}{collapsed}{}", &original[item.start..tree.start], &original[tree.end..item.end]))
}

/// `tree` with one-member brace groups collapsed, innermost first.
fn collapse_single_member_groups(tree: &str) -> String {
    fn next_group(text: &str) -> Option<(usize, usize, String)> {
        let bytes = text.as_bytes();
        let mut opens = vec![];
        let mut found = None;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    i = text[i..].find('\n').map_or(bytes.len(), |line| i + line + 1);
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    let mut depth = 1;
                    i += 2;
                    while i < bytes.len() && depth > 0 {
                        if bytes[i..].starts_with(b"/*") {
                            depth += 1;
                            i += 2;
                        } else if bytes[i..].starts_with(b"*/") {
                            depth -= 1;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                }
                b'{' => {
                    opens.push(i);
                    i += 1;
                }
                b'}' => {
                    if let Some(open) = opens.pop() {
                        let inner = text[open + 1..i].trim();
                        let member = inner.strip_suffix(',').map_or(inner, |member| member.trim_end());
                        if !member.is_empty() && !member.chars().any(|c| matches!(c, '{' | '}' | ',')) {
                            let self_import =
                                member.strip_prefix("self").filter(|rest| rest.is_empty() || rest.trim_start().starts_with("as "));
                            let (start, with) = if let Some(rest) = self_import {
                                let before = text[..open].trim_end();
                                let start = before.strip_suffix("::").map_or(open, |prefix| prefix.len());
                                let with = if rest.trim().is_empty() { String::new() } else { format!(" {}", rest.trim()) };
                                (start, with)
                            } else {
                                (open, member.to_string())
                            };
                            found = Some((start, i + 1, with));
                        }
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        found
    }
    let mut out = tree.to_string();
    while let Some((start, end, with)) = next_group(&out) {
        out.replace_range(start..end, &with);
    }
    out
}

/// The `use` item inside a fn body that holds `range`.
pub(crate) fn body_use_item(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    crate::language::rust::body_import_ranges(text)
        .ok()?
        .into_iter()
        .find(|(item, _)| item.start <= range.start && range.end <= item.end)
        .map(|(item, _)| item)
}

/// The edit that drops the moved members of one body `use`.
pub(crate) fn body_use_edit(text: &str, item: Range<usize>, removals: &[Range<usize>], added: &[String]) -> Option<(Range<usize>, String)> {
    let merged = merge_ranges(removals.to_vec());
    if all_bindings_removed(text, &item, &merged).ok()? {
        if added.is_empty() {
            return Some((use_item_line_range(text, &item), String::new()));
        }
        let indent = line_indent(text, item.start);
        return Some((item, added.join(&format!("\n{indent}"))));
    }
    let tree_start = item.start + text[item.clone()].find("use ")? + "use ".len();
    let tree_end = item.end.checked_sub(1)?;
    let mut tree = text[tree_start..tree_end].to_string();
    for removal in merged.iter().rev() {
        if removal.start < tree_start || removal.end > tree_end {
            return None;
        }
        tree.replace_range((removal.start - tree_start)..(removal.end - tree_start), "");
    }
    let collapsed = collapse_single_member_groups(&tree);
    let mut lines = vec![];
    if !collapsed.trim().is_empty() {
        lines.push(format!("use {collapsed};"));
    }
    lines.extend(added.iter().cloned());
    if lines.is_empty() {
        return Some((use_item_line_range(text, &item), String::new()));
    }
    let indent = line_indent(text, item.start);
    Some((item, lines.join(&format!("\n{indent}"))))
}

/// The ranges that remain after overlapping or touching removals are merged.
fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = vec![];
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

/// The removals and added imports for one `use` item inside a fn body.
#[derive(Default)]
pub(crate) struct BodyUseGroup {
    pub(crate) removals: Vec<Range<usize>>,
    pub(crate) added: Vec<String>,
    pub(crate) sites: Vec<Site>,
}

/// The edits a mention rewrite gathers for one move.
#[derive(Default)]
pub(crate) struct MentionEdits {
    pub(crate) groups: BTreeMap<(String, usize), MovedMentions>,
    pub(crate) edits: HashMap<usize, Vec<(Range<usize>, String)>>,
    pub(crate) promote: BTreeSet<usize>,
}

/// Whether the removals cover every binding of the import item.
fn all_bindings_removed(text: &str, item: &Range<usize>, removals: &[Range<usize>]) -> Result<bool, syn::Error> {
    let bindings = crate::language::rust_refs::import_sites_in(&text[item.clone()])?;
    Ok(!bindings.is_empty()
        && bindings.iter().all(|binding| {
            let range = (item.start + binding.range.start)..(item.start + binding.range.end);
            removals.iter().any(|removal| removal.start <= range.start && range.end <= removal.end)
        }))
}

#[derive(Default)]
pub(crate) struct MentionPlan {
    pub(crate) replacement: Option<String>,
    pub(crate) needs_visibility: bool,
    pub(crate) importable: bool,
}

pub(crate) fn rewrite(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    mut newtext: Vec<String>,
    mut newheaders: HashMap<String, String>,
    all_mentioned: &[HashSet<String>],
    external: &HashMap<String, Vec<crate::language::rust_refs::ImportSite>>,
    carried: &[Carry],
    local_sites: &[LocalImport],
    reaching: &HashMap<String, HashSet<String>>,
    references: &crate::language::rust_refs::References<'_>,
) -> Result<(Vec<String>, HashMap<String, String>, BTreeSet<usize>, BTreeSet<usize>), Failure> {
    let targets = rewrite_targets(ix, moved);
    let sites: HashMap<usize, Vec<Site>> =
        targets.iter().map(|&t| (t, crate::language::rust_refs::rewrite_sites_refs(references, t))).collect();
    let use_all = all_use_sites(ix)?;
    let use_keys: HashSet<(Option<usize>, usize, usize)> = use_all.iter().map(|(_, site)| use_key(site)).collect();
    let groups = use_groups(&use_all, &targets);
    let sources = moved_sources(ix, moved);
    let mentioned = all_mentioned.to_vec();
    let mut in_sources: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, x) in ix.syms.iter().enumerate().filter(|(_, x)| sources.contains(&x.module) && x.kind_text != "use") {
        in_sources.entry(x.module.clone()).or_default().push(i);
    }
    let crate_glob = if sources.contains("crate") { crate::extract::rust::glob_reach::crate_glob_reach(ix)? } else { HashSet::new() };
    let cx = Rewrite { ix, moved, newmod, targets, sites, use_keys, sources, mentioned, in_sources, external, reaching, crate_glob };
    let (mut promote, public, import_ok) = rewrite_imports(&cx, groups, &mut newtext, &mut newheaders)?;
    restore_moved_use_imports(&cx, &mut newheaders)?;
    for module in &cx.sources {
        drop_unused_external_imports(&cx, module, &mut newtext, &mut newheaders)?;
        drop_unused_local_imports(ix, module, local_sites, &mut newtext, &mut newheaders)?;
    }
    crate::extract::rust::imports::carry_imports(ix, ids, moved, newmod, target, carried, references, &mut newtext, &mut newheaders, None)?;
    check_format_capture_clashes(&cx, &newheaders)?;
    import_format_captures(&cx, &mut newheaders);
    promote.extend(rewrite_mentions(&cx, &import_ok, &mut newtext, &mut newheaders)?);
    reexport_moved_mentions(&cx, &mut newheaders, &mut newtext);
    for &i in promote.iter().filter(|&&i| ix.syms[i].kind_text == "thread_local!") {
        newtext[i] = promote_hidden_static(&newtext[i], &ix.syms[i].name);
    }
    promote.retain(|&i| can_promote(ix, ids, i) && ix.syms[i].kind_text != "thread_local!");
    for &i in &promote {
        if !is_public(&newtext[i]) {
            newtext[i] = make_visible(&newtext[i]);
        }
    }
    Ok((newtext, newheaders, promote, public))
}

fn use_key(site: &Site) -> (Option<usize>, usize, usize) {
    (site.symbol, site.range.start, site.range.end)
}

fn alias_of(site: &Site) -> Option<String> {
    match &site.via {
        Via::Import(alias) => Some(alias.clone()),
        _ => None,
    }
}

fn is_import_path(site: &Site, text: &str) -> bool {
    matches!(site.via, Via::Import(_)) && text[site.range.clone()].contains("::")
}

fn use_groups(use_all: &[(String, Site)], targets: &BTreeSet<usize>) -> HashMap<Container, Vec<Site>> {
    let mut groups: HashMap<Container, Vec<Site>> = HashMap::new();
    for (module, site) in use_all {
        if targets.contains(&site.target) {
            groups.entry(container_key(module, site)).or_default().push(site.clone());
        }
    }
    groups
}

fn rewrite_imports(
    cx: &Rewrite,
    groups: HashMap<Container, Vec<Site>>,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(BTreeSet<usize>, BTreeSet<usize>, HashSet<(usize, String, String)>), Failure> {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let (sites, use_keys) = (&cx.sites, &cx.use_keys);
    let mut import_ok: HashSet<(usize, String, String)> = HashSet::new();
    let mut promote: BTreeSet<usize> = BTreeSet::new();
    let mut public: BTreeSet<usize> = BTreeSet::new();
    for (key, mut group) in groups {
        group.sort_by(|a, b| b.range.start.cmp(&a.range.start));
        let original = container_text(&key, &*newheaders, &*newtext);
        let mut replacements: Vec<(Range<usize>, String)> = vec![];
        let mut line_removals: Vec<Range<usize>> = vec![];
        let mut member_removals: Vec<(Range<usize>, Option<Range<usize>>)> = vec![];
        let mut additions: Vec<(Range<usize>, String)> = vec![];
        for site in &group {
            let alias = alias_of(site);
            let use_new = match key.0 {
                Some(i) => newmod[i].clone(),
                None => key.1.clone(),
            };
            let use_vis = use_visibility(&original, &site.range);
            let needed = !use_vis.is_empty()
                || sites[&site.target].iter().any(|m| {
                    let Some(mi) = m.symbol else { return false };
                    !is_import_path(m, &ix.syms[mi].text)
                        && !use_keys.contains(&use_key(m))
                        && matches!(&m.via, Via::Import(a) if Some(a) == alias.as_ref())
                        && newmod[mi] == use_new
                });
            let self_import =
                moved[site.target] && use_new == newmod[site.target] && alias.as_deref() == Some(ix.syms[site.target].name.as_str());
            if self_import {
                if use_vis == "pub " {
                    public.insert(site.target);
                } else if !use_vis.is_empty() {
                    promote.insert(site.target);
                }
                remove_import_site(&original, site, &mut line_removals, &mut member_removals);
            } else if needed {
                if let Some(a) = &alias {
                    import_ok.insert((site.target, a.clone(), use_new.clone()));
                }
                if moved[site.target] {
                    let path = full_path(&newmod[site.target], &ix.syms[site.target].name);
                    if !crate::extract::rust::visibility::visible_from(&newmod[site.target], &use_new) {
                        promote.insert(site.target);
                    }
                    if site.outside == 0 {
                        replacements.push((site.range.clone(), path));
                    } else {
                        member_removals.push((site.range.clone(), site.alias.clone()));
                        let alias_text = site.alias.as_ref().map(|r| original[r.clone()].to_string()).unwrap_or_default();
                        let vis = use_visibility(&original, &site.range);
                        additions.push((site.range.clone(), format!("{vis}use {path}{alias_text};")));
                    }
                }
            } else if moved[site.target] {
                remove_import_site(&original, site, &mut line_removals, &mut member_removals);
            } else {
                let module = match key.0 {
                    Some(i) => &ix.syms[i].module,
                    None => &key.1,
                };
                let unused = match alias.as_ref() {
                    Some(alias) => unused_after_move(cx, module, alias)?,
                    None => false,
                };
                if unused && !allows_unused(&original, &site.range) {
                    if let Some(local) = &alias {
                        let target = &ix.syms[site.target];
                        let path: Vec<String> = full_path(&target.module, &target.name).split("::").map(str::to_string).collect();
                        add_descendant_import(cx, module, local, &path, local, newheaders)?;
                    }
                    remove_import_site(&original, site, &mut line_removals, &mut member_removals);
                }
            }
        }
        let text = apply_import_edits(&original, replacements, line_removals, member_removals, additions, &key.1)?;
        set_container_text(&key, &mut *newheaders, &mut *newtext, text);
    }
    Ok((promote, public, import_ok))
}

fn rewrite_mentions(
    cx: &Rewrite,
    import_ok: &HashSet<(usize, String, String)>,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<BTreeSet<usize>, Failure> {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let (targets, sites, use_keys) = (&cx.targets, &cx.sites, &cx.use_keys);
    let mut out = MentionEdits::default();
    let mut body_groups: BTreeMap<(usize, usize, usize), BodyUseGroup> = BTreeMap::new();
    for &t in targets {
        for site in &sites[&t] {
            let Some(i) = site.symbol else { continue };
            if ix.syms[i].kind_text == "mod" && matches!(site.via, Via::Import(_)) {
                continue;
            }
            if ix.syms[i].kind_text == "mod" && is_mod_declaration(&ix.syms[i].text, &site.range) {
                continue;
            }
            if use_keys.contains(&use_key(site)) || (!moved[t] && !moved[i]) {
                continue;
            }
            let mention_new = newmod[i].clone();
            if site.format {
                if !crate::extract::rust::visibility::visible_from(&newmod[t], &mention_new) {
                    out.promote.insert(t);
                }
                continue;
            }
            if matches!(site.via, Via::Import(_)) && in_fn_at_destination(cx, i, t) {
                if site.outside == 0 {
                    if let Some(range) = body_use_line_removal(&newtext[i], &site.range) {
                        out.edits.entry(i).or_default().push((range, String::new()));
                        continue;
                    }
                } else if let Some(item) = body_use_item(&newtext[i], &site.range) {
                    let group = body_groups.entry((i, item.start, item.end)).or_default();
                    group.removals.push(member_removal(&newtext[i], &site.range, site.alias.as_ref()));
                    if let Some(alias) = &site.alias {
                        let path = full_path(&newmod[t], &ix.syms[t].name);
                        group.added.push(format!("use {path} {};", &newtext[i][alias.clone()]));
                    }
                    group.sites.push(site.clone());
                    continue;
                }
            }
            rewrite_mention(cx, import_ok, t, i, site, &mention_new, &mut out);
        }
    }
    for ((i, start, end), group) in body_groups {
        match body_use_edit(&newtext[i], start..end, &group.removals, &group.added) {
            Some(edit) => out.edits.entry(i).or_default().push(edit),
            None => {
                let mention_new = newmod[i].clone();
                for site in group.sites {
                    rewrite_mention(cx, import_ok, site.target, i, &site, &mention_new, &mut out);
                }
            }
        }
    }
    let MentionEdits { groups, mut edits, promote } = out;
    resolve_moved_mentions(cx, groups, newheaders, &mut edits)?;
    for (i, es) in edits {
        newtext[i] = splice(&newtext[i], es);
    }
    Ok(promote)
}

/// Whether code moving out of `module` mentioned `name` and nothing staying
/// there, or seeing it through a glob, does.
fn unused_after_move(cx: &Rewrite, module: &str, name: &str) -> Result<bool, Failure> {
    let mentions = |i: usize| cx.mentioned[i].contains(name);
    let in_module = cx.in_sources.get(module).map(Vec::as_slice).unwrap_or_default();
    if !in_module.iter().any(|&i| cx.moved[i] && mentions(i)) || staying_mentions(cx, module, name) {
        return Ok(false);
    }
    for at in reaching_modules(cx.reaching, module) {
        if crate::extract::rust::glob_reach::module_mentions(cx, module, at, name)? {
            return Ok(false);
        }
    }
    if module == "crate" {
        for at in &cx.crate_glob {
            if crate::extract::rust::glob_reach::module_mentions(cx, module, at, name)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// The source module without the imports from another crate that only the code
/// moving out of it used.
fn drop_unused_external_imports(
    cx: &Rewrite,
    module: &str,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let external: Vec<crate::language::rust_refs::ImportSite> = cx.external.get(module).cloned().unwrap_or_default();
    if let Some(header) = newheaders.get(module).cloned() {
        let kept = drop_unused_imports(cx, module, &header, &external, newheaders)?;
        newheaders.insert(module.to_string(), kept);
    }
    let symbols: Vec<usize> = cx
        .ix
        .syms
        .iter()
        .enumerate()
        .filter(|(_, s)| s.module == module && matches!(s.kind_text.as_str(), "use" | "mod"))
        .map(|(i, _)| i)
        .collect();
    for i in symbols {
        let kept = drop_unused_imports(cx, module, &newtext[i], &external, newheaders)?;
        newtext[i] = kept;
    }
    Ok(())
}

/// `text` without the external imports that only code moving out of `module`
/// mentions, keeping re-exports and imports allowed to be unused.
fn drop_unused_imports(
    cx: &Rewrite,
    module: &str,
    text: &str,
    external: &[crate::language::rust_refs::ImportSite],
    newheaders: &mut HashMap<String, String>,
) -> Result<String, Failure> {
    let glob_unused = external.iter().any(|site| site.glob) && !module_needs_glob(cx, module)?;
    let descendant_glob = descendants_need_glob(cx, module)?;
    let mut keys = HashSet::new();
    for site in external {
        let key = (site.local.clone(), site.path.clone(), site.glob);
        if site.glob {
            if glob_unused {
                if descendant_glob {
                    add_glob_descendant_imports(cx, module, site, newheaders)?;
                }
                keys.insert(key);
            }
        } else if unused_after_move(cx, module, &site.local)? {
            add_descendant_import(cx, module, &site.local, &site.path, &site.local, newheaders)?;
            keys.insert(key);
        }
    }
    drop_import_keys(text, module, &keys)
}

/// Puts a moved top-level `use` back in its source header when staying code
/// still needs it, privately so the item is re-exported only from its new module.
fn restore_moved_use_imports(cx: &Rewrite<'_>, newheaders: &mut HashMap<String, String>) -> Result<(), Failure> {
    for (_, symbol) in cx.ix.syms.iter().enumerate().filter(|(i, s)| cx.moved[*i] && s.parent.is_none() && s.kind_text == "use") {
        let mut header = newheaders.get(&symbol.module).cloned().unwrap_or_default();
        let mut existing: HashSet<(String, Vec<String>, bool)> = crate::language::rust_refs::import_sites_in(&header)
            .map_err(|e| Failure(format!("imports in {} don't parse: {e}", symbol.module), 1))?
            .into_iter()
            .map(|site| (site.local, site.path, site.glob))
            .collect();
        let sites = crate::language::rust_refs::import_sites_in(&symbol.text)
            .map_err(|e| Failure(format!("imports in {} don't parse: {e}", symbol.module), 1))?;
        let source_glob = sites.iter().any(|site| site.glob) && module_needs_glob(cx, &symbol.module)?;
        for site in sites {
            let source_needed = if site.glob { source_glob } else { staying_mentions(cx, &symbol.module, &site.local) };
            if !source_needed {
                if site.glob {
                    add_glob_descendant_imports(cx, &symbol.module, &site, newheaders)?;
                } else {
                    add_descendant_import(cx, &symbol.module, &site.local, &site.path, &site.local, newheaders)?;
                }
                continue;
            }
            if !existing.insert((site.local.clone(), site.path.clone(), site.glob)) {
                continue;
            }
            let attributes = use_attributes(&symbol.text, &site.range);
            let path = site.path.join("::");
            let line = if site.glob {
                format!("use {path}::*;")
            } else if site.local == leaf(&path) {
                format!("use {path};")
            } else {
                format!("use {path} as {};", site.local)
            };
            let line = attributed_import_line(&attributes, line);
            header = add_import_line(&header, &line);
        }
        newheaders.insert(symbol.module.clone(), header);
    }
    Ok(())
}

/// The attributes written before the `use` whose tree holds `range`.
fn use_attributes(text: &str, range: &Range<usize>) -> String {
    let Ok(ranges) = crate::language::rust::import_ranges(text) else {
        return String::new();
    };
    let Some((item, tree)) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end) else {
        return String::new();
    };
    let Some(head) = text[item.start..tree.start].trim().strip_suffix("use") else {
        return String::new();
    };
    let head = head.trim();
    let visibility = use_visibility(text, range);
    head.strip_suffix(visibility.trim()).unwrap_or(head).trim().to_string()
}

fn staying_mentions(cx: &Rewrite, source: &str, name: &str) -> bool {
    staying_in(cx.ix, cx.moved, cx.newmod, source).any(|i| cx.mentioned[i].contains(name))
}

fn module_needs_glob(cx: &Rewrite, module: &str) -> Result<bool, Failure> {
    let named = named_imports(cx, module)?;
    Ok(staying_in(cx.ix, cx.moved, cx.newmod, module).any(|i| names_need_glob(cx.ix, module, &cx.mentioned[i], &named)))
}

/// The path a moved item's mention in another module needs, and whether the
/// target must be made reachable.
fn moved_reference(target: &str, name: &str, mention: &str) -> (String, bool) {
    (full_path(target, name), !crate::extract::rust::visibility::visible_from(target, mention))
}

/// The path to write for `target` in `mention`'s module: a path written from
/// the crate root stays root-relative, so a re-export keeps the path that
/// reaches it; otherwise it is the target's own module path.
fn reference_path(ix: &Index, moved: &[bool], target: usize, target_new: &str, mention: usize, segs: &[String]) -> String {
    if !moved[target] {
        let path = crate::extract::rust::imports::rooted_path(ix, &ix.syms[mention].module, target_new, segs);
        if path.first().is_some_and(|s| s == "crate") {
            return path.join("::");
        }
    }
    full_path(target_new, &ix.syms[target].name)
}

/// Imports a format capture the moved code can no longer name in scope. A
/// capture can only be an identifier, so a path cannot stand in for it.
fn import_format_captures(cx: &Rewrite, newheaders: &mut HashMap<String, String>) {
    for &t in &cx.targets {
        for site in &cx.sites[&t] {
            let Some((module, _, line)) = format_capture_import(cx, t, site) else { continue };
            let header = newheaders.get(&module).cloned().unwrap_or_default();
            if !header.lines().any(|existing| existing.trim() == line) {
                newheaders.insert(module, add_import_line(&header, &line));
            }
        }
    }
}

/// The import a format capture needs in its new module, and the name it binds.
fn format_capture_import(cx: &Rewrite, t: usize, site: &Site) -> Option<(String, String, String)> {
    if !site.format {
        return None;
    }
    let i = site.symbol?;
    if !cx.moved[t] && !cx.moved[i] {
        return None;
    }
    let mention_new = cx.newmod[i].clone();
    let target_new = cx.newmod[t].clone();
    if target_new == mention_new {
        return None;
    }
    let local = site.segs.first().cloned().unwrap_or_else(|| cx.ix.syms[t].name.clone());
    let path = reference_path(cx.ix, cx.moved, t, &target_new, i, &site.segs);
    let line = if leaf(&path) == local.as_str() { format!("use {path};") } else { format!("use {path} as {local};") };
    let line = if cx.ix.test[i] { format!("#[cfg(test)] {line}") } else { line };
    Some((mention_new, local, line))
}

/// Refuses a format capture whose name the destination already binds.
fn check_format_capture_clashes(cx: &Rewrite, newheaders: &HashMap<String, String>) -> Result<(), Failure> {
    for &t in &cx.targets {
        for site in &cx.sites[&t] {
            let Some((module, local, line)) = format_capture_import(cx, t, site) else { continue };
            let header = newheaders.get(&module).cloned().unwrap_or_default();
            if header.lines().any(|existing| existing.trim() == line) {
                continue;
            }
            if target_names_after_move(cx.ix, cx.moved, cx.newmod, &header, &module)?.contains(&local) {
                return fail(format!("E1004 {module}.{local} already exists in {module}; the format capture cannot be imported there"));
            }
        }
    }
    Ok(())
}

/// Whether a header imports `path` under `name` for the code that needs it.
fn header_binds(header: &str, name: &str, path: &str, test_only: bool) -> bool {
    let bound = crate::language::rust_refs::import_sites_in(header)
        .map(|sites| sites.into_iter().any(|site| !site.glob && site.local == name && site.path.join("::") == path))
        .unwrap_or(false);
    if !bound {
        return false;
    }
    let plain = format!("use {path};");
    let test = format!("#[cfg(test)] {plain}");
    let test_gated = header.lines().any(|line| line.trim() == test);
    !test_gated || test_only || header.lines().any(|line| line.trim() == plain)
}

/// Whether a glob in `module` supplies `name` to code beyond the moved item's
/// own mentions, which a new explicit import would shadow.
fn glob_binds(cx: &Rewrite, header: &str, module: &str, name: &str, target: usize) -> Result<bool, Failure> {
    let has_glob = std::iter::once(header)
        .chain(module_use_symbols(cx.ix, |i, _| in_module_after_move(cx.ix, cx.moved, cx.newmod, i, module)).map(|s| s.text.as_str()))
        .map(|text| imports_have_glob(text, module))
        .find(|r| !matches!(r, Ok(false)))
        .transpose()?
        .unwrap_or(false);
    if !has_glob {
        return Ok(false);
    }
    let own: HashSet<usize> = cx.sites[&target].iter().filter_map(|site| site.symbol).collect();
    Ok(cx
        .ix
        .syms
        .iter()
        .enumerate()
        .any(|(i, _)| cx.newmod[i] == module && i != target && !own.contains(&i) && cx.mentioned[i].contains(name)))
}

/// Drops a test-only import that non-test code now needs.
fn remove_test_import(header: &str, path: &str) -> String {
    let test = format!("#[cfg(test)] use {path};");
    header.split_inclusive('\n').filter(|line| line.trim() != test).collect()
}

/// Whether an inline module on the way to `i` leaves it in a different module
/// from its header, so no import can be added there.
fn in_moved_inline_module(cx: &Rewrite, i: usize) -> bool {
    let mut parent = cx.ix.syms[i].parent.as_deref();
    while let Some(id) = parent {
        let Some(p) = cx.ix.syms.iter().position(|sym| sym.id == id) else { break };
        if cx.ix.syms[p].kind_text == "mod" && cx.newmod[p] != cx.newmod[i] {
            return true;
        }
        parent = cx.ix.syms[p].parent.as_deref();
    }
    false
}

/// Chooses one spelling for each moved item's mentions in a module.
fn resolve_moved_mentions(
    cx: &Rewrite,
    groups: BTreeMap<(String, usize), MovedMentions>,
    newheaders: &mut HashMap<String, String>,
    edits: &mut HashMap<usize, Vec<(Range<usize>, String)>>,
) -> Result<(), Failure> {
    let (ix, newmod) = (cx.ix, cx.newmod);
    for ((module, t), group) in groups {
        let path = full_path(&newmod[t], &ix.syms[t].name);
        let name = leaf(&ix.syms[t].name).to_string();
        let header = newheaders.get(&module).cloned().unwrap_or_default();
        let imported = header_binds(&header, &name, &path, group.test_only);
        let base = if !imported && !group.test_only { remove_test_import(&header, &path) } else { header };
        let taken = !imported
            && (target_names_after_move(cx.ix, cx.moved, cx.newmod, &base, &module)?.contains(&name)
                || is_prelude_name(&name)
                || glob_binds(cx, &base, &module, &name, t)?);
        let mut bare = false;
        for (i, range, hidden) in group.sites {
            let text = if taken || hidden {
                path.clone()
            } else {
                bare = true;
                name.clone()
            };
            edits.entry(i).or_default().push((range, text));
        }
        if bare && !imported {
            let gated = group.test_only && !ix.test_modules.contains(&module);
            let line = if gated { format!("#[cfg(test)] use {path};") } else { format!("use {path};") };
            newheaders.insert(module, add_import_line(&base, &line));
        }
    }
    Ok(())
}

/// Whether `text`'s imports include a glob (`use path::*;`).
fn imports_have_glob(text: &str, module: &str) -> Result<bool, Failure> {
    Ok(crate::language::rust_refs::import_sites_in(text)
        .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
        .iter()
        .any(|site| site.glob))
}

/// Removes a standalone import's line, or only its member in a group.
fn remove_import_site(
    original: &str,
    site: &Site,
    line_removals: &mut Vec<Range<usize>>,
    member_removals: &mut Vec<(Range<usize>, Option<Range<usize>>)>,
) {
    if site.outside == 0 {
        line_removals.extend(use_line_removal(original, &site.range));
    } else {
        member_removals.push((site.range.clone(), site.alias.clone()));
    }
}

/// Whether a mention sits in a fn that shares the moved item's destination
/// module, where the item is already in scope.
fn in_fn_at_destination(cx: &Rewrite, i: usize, t: usize) -> bool {
    cx.ix.syms[i].kind_text == "fn" && cx.newmod[i] == cx.newmod[t]
}

/// The line of a plain `use` statement inside a fn body that holds `range`.
fn body_use_line_removal(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let item = body_use_item(text, range)?;
    let code = &text[item.clone()];
    if code.contains('{') || code.contains(" as ") {
        return None;
    }
    Some(use_item_line_range(text, &item))
}

/// Rewrites one mention that no body import edit covers.
fn rewrite_mention(
    cx: &Rewrite,
    import_ok: &HashSet<(usize, String, String)>,
    t: usize,
    i: usize,
    site: &Site,
    mention_new: &str,
    out: &mut MentionEdits,
) {
    if rewrite_impl_mention(cx, t, i, site, mention_new, out) {
        return;
    }
    let target_new = cx.newmod[t].clone();
    let stays_in_parent = test_module_path(cx.ix, &target_new).as_str() == mention_new;
    let plan = match &site.via {
        Via::Import(alias) => import_mention(cx, import_ok, t, i, site, mention_new, &target_new, alias),
        Via::Path => path_mention(cx, t, i, site, mention_new, &target_new),
        Via::Own | Via::Glob => own_mention(cx, t, i, mention_new, &target_new, stays_in_parent),
    };
    if let Some(text) = plan.replacement {
        if plan.importable {
            let group = out.groups.entry((mention_new.to_string(), t)).or_insert_with(|| MovedMentions { test_only: true, sites: vec![] });
            group.test_only &= cx.ix.test[i];
            group.sites.push((i, site.range.clone(), site.shadowed));
        } else {
            out.edits.entry(i).or_default().push((site.range.clone(), text));
        }
    }
    if plan.needs_visibility {
        out.promote.insert(t);
    }
}

fn rewrite_impl_mention(cx: &Rewrite, t: usize, i: usize, site: &Site, mention_new: &str, out: &mut MentionEdits) -> bool {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let parent_impl = ix.syms[t].parent.as_deref().and_then(|p| ix.syms.iter().position(|s| s.id == p && s.kind_text == "impl"));
    let Some(at) = parent_impl else { return false };
    if matches!(&site.via, Via::Path) && site.segs.len() > 1 {
        if let Ok(owner) = impl_owner_symbol(ix, at) {
            let owner_new = newmod[owner].clone();
            if mention_new != owner_new.as_str() {
                if !crate::extract::rust::visibility::visible_from(&owner_new, mention_new) {
                    out.promote.insert(owner);
                }
                if let Some(sep) = ix.syms[i].text[site.range.clone()].rfind("::") {
                    out.edits.entry(i).or_default().push((
                        site.range.start..site.range.start + sep,
                        reference_path(ix, moved, owner, &owner_new, i, &site.segs[..site.segs.len() - 1]),
                    ));
                }
            }
        }
    }
    true
}

fn import_mention(
    cx: &Rewrite,
    import_ok: &HashSet<(usize, String, String)>,
    t: usize,
    i: usize,
    site: &Site,
    mention_new: &str,
    target_new: &str,
    alias: &str,
) -> MentionPlan {
    let ix = cx.ix;
    let written = &ix.syms[i].text[site.range.clone()];
    let mut plan = MentionPlan::default();
    if written.contains("::") {
        if cx.moved[t] {
            let (path, promote) = moved_reference(target_new, &ix.syms[t].name, mention_new);
            plan.replacement = Some(path);
            plan.needs_visibility = promote;
            plan.importable = !in_moved_inline_module(cx, i);
        }
    } else if import_ok.contains(&(t, alias.to_string(), mention_new.to_string()))
        || target_new == mention_new
        || cx.sites[&t]
            .iter()
            .any(|m| m.symbol == site.symbol && is_import_path(m, &ix.syms[i].text) && matches!(&m.via, Via::Import(a) if a == alias))
    {
    } else {
        let (path, promote) = moved_reference(target_new, &ix.syms[t].name, mention_new);
        plan.replacement = Some(path);
        plan.needs_visibility = promote;
        plan.importable = !path_only(cx, t, i);
    }
    plan
}

fn path_mention(cx: &Rewrite, t: usize, i: usize, site: &Site, mention_new: &str, target_new: &str) -> MentionPlan {
    let ix = cx.ix;
    let mut plan = MentionPlan::default();
    if target_new == mention_new {
        let name = leaf(&ix.syms[t].name);
        plan.replacement = Some(if site.shadowed { full_path(target_new, &ix.syms[t].name) } else { name.to_string() });
    } else if !cx.moved[t] {
        plan.replacement = Some(reference_path(ix, cx.moved, t, target_new, i, &site.segs));
        plan.needs_visibility = !crate::extract::rust::visibility::visible_from(target_new, mention_new);
    } else {
        let (path, promote) = moved_reference(target_new, &ix.syms[t].name, mention_new);
        plan.replacement = Some(path);
        plan.needs_visibility = promote;
        plan.importable = !path_only(cx, t, i);
    }
    plan
}

fn own_mention(cx: &Rewrite, t: usize, i: usize, mention_new: &str, target_new: &str, stays_in_parent: bool) -> MentionPlan {
    let ix = cx.ix;
    let mut plan = MentionPlan::default();
    if target_new != mention_new && !stays_in_parent {
        let (path, promote) = moved_reference(target_new, &ix.syms[t].name, mention_new);
        plan.replacement = Some(path);
        plan.needs_visibility = promote;
        plan.importable = !path_only(cx, t, i);
    }
    plan
}

fn path_only(cx: &Rewrite, t: usize, i: usize) -> bool {
    in_moved_inline_module(cx, i) || (!cx.moved[t] && cx.ix.syms[t].kind_text == "thread_local!")
}

/// `text` without the imports whose binding is in `keys`.
pub(crate) fn drop_import_keys(text: &str, module: &str, keys: &HashSet<(String, Vec<String>, bool)>) -> Result<String, Failure> {
    let sites: Vec<crate::language::rust_refs::ImportSite> = crate::language::rust_refs::import_sites_in(text)
        .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
        .into_iter()
        .filter(|site| keys.contains(&(site.local.clone(), site.path.clone(), site.glob)))
        .collect();
    let mut line_removals: Vec<Range<usize>> = vec![];
    let mut members: Vec<(Range<usize>, Option<Range<usize>>)> = vec![];
    for site in sites {
        if allows_unused(text, &site.range) || is_reexport(text, &site.range) {
            continue;
        }
        if site.outside == 0 {
            line_removals.extend(use_line_removal(text, &site.range));
        } else {
            members.push((site.range, site.alias));
        }
    }
    apply_import_edits(text, vec![], line_removals, members, vec![], module)
}

/// `text` without the local imports that only the moved code uses.
pub(crate) fn drop_unused_local_imports(
    ix: &Index,
    module: &str,
    local: &[LocalImport],
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let mut keys = HashSet::new();
    for import in local.iter().filter(|import| import.module == module && !import.used_by_staying) {
        let key = (import.site.local.clone(), import.site.path.clone(), import.site.glob);
        for child in &import.needed_by {
            add_module_import(child, &import.site.path, &import.site.local, newheaders)?;
        }
        keys.insert(key);
    }
    if keys.is_empty() {
        return Ok(());
    }
    if let Some(header) = newheaders.get(module).cloned() {
        let kept = drop_import_keys(&header, module, &keys)?;
        newheaders.insert(module.to_string(), kept);
    }
    let symbols: Vec<usize> = ix
        .syms
        .iter()
        .enumerate()
        .filter(|(_, s)| s.module == module && matches!(s.kind_text.as_str(), "use" | "mod"))
        .map(|(i, _)| i)
        .collect();
    for i in symbols {
        let kept = drop_import_keys(&newtext[i], module, &keys)?;
        newtext[i] = kept;
    }
    Ok(())
}

/// The imports whose binding the move changed in the module's header.
pub(crate) fn module_imports_rewritten(m: &RustMove<'_>, module: &str) -> BTreeSet<String> {
    let before = m.ix.modules.iter().find(|row| row.path.as_str() == module).map(|row| row.header.as_str()).unwrap_or("");
    let after = m.newheaders.get(module).map(String::as_str).unwrap_or("");
    changed_imports(before, after)
}

/// The import bindings a header or module body holds.
fn import_keys(text: &str) -> BTreeSet<(String, Vec<String>, bool, String)> {
    crate::language::rust_refs::import_sites_in(text)
        .unwrap_or_default()
        .into_iter()
        .map(|site| (site.local, site.path, site.glob, site.attributes))
        .collect()
}

/// The import bindings that differ between two texts.
pub(crate) fn changed_imports(before: &str, after: &str) -> BTreeSet<String> {
    import_keys(before).symmetric_difference(&import_keys(after)).map(|key| key.0.clone()).collect()
}

/// Gives each moved item still named by its old path a parent re-export at
/// the narrowest visibility its remaining callers need.
fn reexport_moved_mentions(cx: &Rewrite, newheaders: &mut HashMap<String, String>, newtext: &mut [String]) {
    for &t in &cx.targets {
        let sym = &cx.ix.syms[t];
        if !cx.moved[t] || sym.parent.is_some() || sym.kind_text == "impl" {
            continue;
        }
        let parent = sym.module.as_str();
        let destination = cx.newmod[t].as_str();
        if destination == parent || !destination.starts_with(&format!("{parent}::")) {
            continue;
        }
        let (callers, outside_test_only) = possible_callers(cx, t);
        if callers.is_empty() {
            continue;
        }
        let test_only = outside_test_only && parent_mentions_are_tests(cx, t, parent);
        let visibility = crate::extract::rust::visibility::visibility_for(parent, &callers);
        let path = full_path(destination, &sym.name);
        let header = newheaders.get(parent).cloned().unwrap_or_default();
        newheaders.insert(parent.to_string(), ensure_reexport(header, &path, &sym.name, visibility, test_only));
        if let Some((raised, _)) = crate::extract::rust::visibility::raise_declaration(&newtext[t], destination, &callers) {
            newtext[t] = raised;
        }
    }
}

/// The modules outside a moved item's destination that may still name its old
/// path, and whether all of those callers are test code.
fn possible_callers(cx: &Rewrite, target: usize) -> (BTreeSet<String>, bool) {
    let destination = cx.newmod[target].as_str();
    let mut callers = BTreeSet::new();
    let mut test_only = true;
    for i in 0..cx.ix.syms.len() {
        if i == target {
            continue;
        }
        let module = cx.newmod[i].as_str();
        if crate::extract::rust::visibility::visible_from(destination, module) {
            continue;
        }
        if cx.ix.possible[i].iter().any(|&(t, _)| t == target) {
            callers.insert(module.to_string());
            test_only &= cx.ix.test[i];
        }
    }
    (callers, test_only)
}

/// Whether the parent's own staying mentions of a moved item are all test code.
fn parent_mentions_are_tests(cx: &Rewrite, target: usize, parent: &str) -> bool {
    cx.sites
        .get(&target)
        .into_iter()
        .flatten()
        .filter_map(|site| site.symbol)
        .filter(|&i| !cx.moved[i] && cx.ix.syms[i].module == parent)
        .all(|i| cx.ix.test[i])
}

/// A parent header with `path` re-exported at `visibility`, gated only when
/// every user of it is test code.
fn ensure_reexport(header: String, path: &str, name: &str, visibility: Option<&str>, test_only: bool) -> String {
    let plain = format!("use {path};");
    let gated = format!("#[cfg(test)] {plain}");
    let desired =
        format!("{}{}{plain}", if test_only { "#[cfg(test)] " } else { "" }, visibility.map_or(String::new(), |v| format!("{v} ")));
    if header.lines().any(|line| line.trim() == desired) {
        return header;
    }
    let existing = crate::language::rust_refs::import_sites_in(&header).unwrap_or_default();
    let bound = |site: &crate::language::rust_refs::ImportSite| !site.glob && site.local == name && site.path.join("::") == path;
    if existing.iter().filter(|site| bound(site)).any(|site| !use_visibility(&header, &site.range).is_empty()) {
        return header;
    }
    let simple = header.lines().any(|line| line.trim() == plain || line.trim() == gated);
    if existing.iter().any(|site| bound(site)) && !simple {
        return header;
    }
    let kept: String = header.split_inclusive('\n').filter(|line| line.trim() != plain && line.trim() != gated).collect();
    add_import_line(&kept, &desired)
}
