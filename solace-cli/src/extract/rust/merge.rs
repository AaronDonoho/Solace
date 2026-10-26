//! Rust `merge`: folds a duplicate item into its copy in one project (D-207).
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;

use rusqlite::params;

use super::{
    Container, add_use_after, all_use_sites, allows_unused, apply, called_names_where, container_key, container_text, drop_import,
    full_path, helper_kind, is_public, is_reexport, move_tests, moved_tests_reply, post_move_check, remove_member, replace_range,
    rewrite_move, set_container_text, splice, test_file, use_visibility,
};
use crate::language::rust_refs::{Site, Via};
use crate::{Failure, Index, Out, Store, Sym, fail, language, lockdown, revision};
mod tests;

pub(crate) fn merge(s: &Store, survivor: &str, retired: &str, budget: usize) -> Out {
    let ix = s.index()?;
    let keep_i = s.resolve(&ix, survivor)?;
    let gone_i = s.resolve(&ix, retired)?;
    if keep_i == gone_i {
        return crate::usage(format!("{} is one symbol; merge takes two", ix.syms[gone_i].qname()));
    }
    mergeable(&ix.syms[keep_i])?;
    mergeable(&ix.syms[gone_i])?;
    let keep = ix.syms[keep_i].clone();
    let gone = ix.syms[gone_i].clone();
    if ix.uses[keep_i].contains(&gone_i) {
        return fail(format!("E5002 {} uses {}, which would then be itself: change {} first", keep.qname(), gone.qname(), keep.qname()));
    }
    same_signature(&keep, &gone)?;

    let MergeTests { moving, moved_ids, helper_ids } = select_merge_tests(&ix, gone_i, &keep.module)?;
    let rewrite = redirect_merge(&ix, keep_i, gone_i, &moving)?;
    let unused = unused_in_merge(&ix, gone_i, &moving);
    let before = s.broken()?;
    let mut created = vec![];
    if !moved_ids.is_empty()
        && let Some(file) = ix.modules.iter().find(|m| m.path == keep.module).and_then(|m| m.file.as_deref())
    {
        let test_file = s.root.join(test_file(&keep.module, file));
        if !test_file.exists() {
            created.push(test_file);
        }
    }

    let plan = MergePlan {
        gone_i,
        keep_i,
        gone_id: &gone.id,
        keep_id: &keep.id,
        gone_module: &gone.module,
        keep_module: &keep.module,
        moved_ids: &moved_ids,
        helper_ids: &helper_ids,
        unused: &unused,
        before: &before,
    };
    s.transaction(&created, || apply_merge(s, &ix, &rewrite, &plan, budget))
}

fn mergeable(x: &Sym) -> Result<(), Failure> {
    if x.parent.is_some() {
        return fail(format!(
            "E6013 {} is nested; Rust merge folds top-level functions and constants, not impl methods or nested items",
            x.qname()
        ));
    }
    match x.kind_text.as_str() {
        "fn" | "const" | "static" => Ok(()),
        "struct" | "enum" | "union" | "trait" | "type" => {
            fail(format!("E6013 {} is a type; Rust merge folds functions and constants, not types", x.qname()))
        }
        "mod" => fail(format!("E6013 {} is a module; Rust merge folds functions and constants, not modules", x.qname())),
        "impl" => fail(format!("E6013 {} is an impl; Rust merge folds functions and constants, not impls or their methods", x.qname())),
        kind => fail(format!("E6013 {} is a {kind}; Rust merge folds top-level functions and constants", x.qname())),
    }
}

fn kind_word(kind: &str) -> &str {
    match kind {
        "fn" => "function",
        "const" => "constant",
        "static" => "static",
        _ => "item",
    }
}

fn same_signature(keep: &Sym, gone: &Sym) -> Result<(), Failure> {
    if keep.kind_text != gone.kind_text {
        return fail(format!(
            "E5008 {} is a {} and {} is a {}; merge folds one item into another of the same kind",
            gone.qname(),
            kind_word(&gone.kind_text),
            keep.qname(),
            kind_word(&keep.kind_text)
        ));
    }
    if merge_signature(keep) == merge_signature(gone) {
        return Ok(());
    }
    let verb = if keep.kind_text == "fn" { "takes" } else { "has" };
    fail(format!(
        "E5008 {} {verb} `{}` and {} {verb} `{}`; \
         merge needs the same parameters, return type and generics, so change one to match the other",
        keep.qname(),
        crate::orientation::rust_signature(&keep.text, &keep.kind_text),
        gone.qname(),
        crate::orientation::rust_signature(&gone.text, &gone.kind_text)
    ))
}

fn merge_signature(s: &Sym) -> String {
    let signature = crate::orientation::rust_signature(&s.text, &s.kind_text);
    let head = without_visibility(&signature);
    match s.kind_text.as_str() {
        "fn" => {
            let Some(at) = head.find("fn ") else { return head.to_string() };
            let name_at = at + 3;
            let end = head[name_at..].find(|c: char| c == '(' || c == '<').map_or(head.len(), |i| name_at + i);
            merge_params(&format!("{}ITEM{}", &head[..name_at], &head[end..]))
        }
        "const" => merge_named_item(&head, "const ", false),
        "static" => merge_named_item(&head, "static ", true),
        _ => head.to_string(),
    }
}

struct MergeRewrite {
    text: Vec<String>,
    headers: HashMap<String, String>,
    hits: usize,
    modules: BTreeSet<String>,
}

fn redirect_merge(ix: &Index, keep_i: usize, gone_i: usize, moving: &[bool]) -> Result<MergeRewrite, Failure> {
    let keep = &ix.syms[keep_i];
    let references = language::rust_refs::references(&ix.syms, &ix.modules);
    let mentions = language::rust_refs::rewrite_sites_refs(&references, gone_i);
    let use_gone: Vec<(String, Site)> = all_use_sites(ix)?.into_iter().filter(|(_, s)| s.target == gone_i).collect();
    let mut text: Vec<String> = ix.syms.iter().map(|s| s.text.clone()).collect();
    let mut headers: HashMap<String, String> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
    check_merge_visibility(ix, keep, moving, &mentions, &use_gone, &text, &headers)?;
    let MergeImports { rewritten, alias_kept, path_only, hits: import_hits, mut modules } =
        rewrite_merge_imports(ix, keep, moving, &mentions, use_gone, &mut text, &mut headers)?;
    let (mention_hits, mention_modules) = rewrite_merge_mentions(ix, keep, gone_i, moving, &alias_kept, &path_only, mentions, &mut text);
    modules.extend(mention_modules);
    drop_merge_imports(ix, gone_i, &rewritten, &mut text, &mut headers)?;
    Ok(MergeRewrite { text, headers, hits: import_hits + mention_hits, modules })
}

fn drop_merge_imports(
    ix: &Index,
    gone_i: usize,
    rewritten: &BTreeSet<Container>,
    text: &mut [String],
    headers: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let gone = &ix.syms[gone_i];
    let references = language::rust_refs::references(&ix.syms, &ix.modules);
    if references.mentioned[gone_i].is_empty() {
        return Ok(());
    }
    let mut containers: BTreeSet<Container> = rewritten.clone();
    containers.insert((None, gone.module.clone()));
    for (i, symbol) in ix.syms.iter().enumerate() {
        if symbol.module == gone.module && matches!(symbol.kind_text.as_str(), "use" | "mod") {
            containers.insert((Some(i), gone.module.clone()));
        }
    }
    let mut unused: HashMap<String, HashSet<String>> = HashMap::new();
    for key in containers {
        let module = key.1.clone();
        let names = unused.entry(module.clone()).or_insert_with(|| merge_unused_imports(ix, &references, gone_i, &module));
        if names.is_empty() {
            continue;
        }
        let body = container_text(&key, headers, text);
        let used: HashSet<String> = language::rust_refs::import_sites_in(&body)
            .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
            .into_iter()
            .map(|site| site.local)
            .filter(|name| !names.contains(name))
            .collect();
        let kept = drop_imports_not_mentioned(&module, &body, &used)?;
        set_container_text(&key, headers, text, kept);
    }
    Ok(())
}

fn drop_imports_not_mentioned(module: &str, text: &str, used: &HashSet<String>) -> Result<String, Failure> {
    let mut sites = language::rust_refs::import_sites_in(text).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    sites.sort_by(|a, b| b.range.start.cmp(&a.range.start));
    let mut out = text.to_string();
    for site in sites {
        if site.glob || used.contains(&site.local) || is_reexport(&out, &site.range) || allows_unused(&out, &site.range) {
            continue;
        }
        out = drop_import(&out, &site.range, site.alias.as_ref(), site.outside);
    }
    Ok(out)
}

/// A function signature without its parameter names, so names that differ do not block a merge.
fn merge_params(head: &str) -> String {
    let (mut open, mut angle) = (None, 0usize);
    for (i, c) in head.char_indices() {
        match c {
            '<' => angle += 1,
            '>' if angle > 0 && !head[..i].ends_with("->") => angle -= 1,
            '(' if angle == 0 => {
                open = Some(i);
                break;
            }
            _ => {}
        }
    }
    let Some(open) = open else { return head.to_string() };
    let mut depth = 0usize;
    let mut close = None;
    for (i, c) in head[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else { return head.to_string() };
    let params = &head[open + 1..close];
    let (mut parts, mut start, mut depth) = (vec![], 0usize, 0usize);
    for (i, c) in params.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(merge_param(&params[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(merge_param(&params[start..]));
    format!("{}({}){}", &head[..open], parts.join(", "), &head[close + 1..])
}

fn merge_param(param: &str) -> String {
    let param = param.trim();
    let (mut depth, mut colon) = (0i32, None);
    for (i, c) in param.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ':' if depth == 0 && !param[i + 1..].starts_with(':') => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    match colon {
        Some(at) => {
            let ty = param[at + 1..].trim();
            if ty.is_empty() { param.to_string() } else { format!("_: {ty}") }
        }
        None => param.to_string(),
    }
}

/// A signature without its visibility, which doesn't change an item's type.
fn without_visibility(head: &str) -> &str {
    let head = head.trim_start();
    let Some(rest) = head.strip_prefix("pub") else { return head };
    let rest = rest.trim_start();
    if let Some(inside) = rest.strip_prefix('(') {
        if let Some(close) = inside.find(')') {
            return inside[close + 1..].trim_start();
        }
        return head;
    }
    rest
}

/// The imports in `module` that only the retired item still mentions.
fn merge_unused_imports(ix: &Index, references: &language::rust_refs::References<'_>, gone_i: usize, module: &str) -> HashSet<String> {
    let mut names = references.mentioned[gone_i].clone();
    names.retain(|name| {
        !(0..ix.syms.len()).any(|i| {
            i != gone_i
                && ix.syms[i].module == module
                && !matches!(ix.syms[i].kind_text.as_str(), "use" | "mod")
                && references.mentioned[i].contains(name)
        })
    });
    if !names.is_empty() {
        let methods = called_names_where(ix, language::rust_refs::CallKind::Method, |i| i != gone_i && ix.syms[i].module == module);
        if let Ok(traits) = language::rust_refs::imported_traits(references, module) {
            for (trait_i, site) in traits {
                if language::rust_refs::trait_has_method(references, trait_i, &methods) {
                    names.remove(&site.local);
                }
            }
        }
    }
    names
}

/// An item's signature with its name replaced, so two names compare equal.
fn merge_named_item(head: &str, keyword: &str, maybe_mut: bool) -> String {
    let Some(at) = head.find(keyword) else { return head.to_string() };
    let mut name_at = at + keyword.len();
    if maybe_mut && head[name_at..].starts_with("mut ") {
        name_at += 4;
    }
    let end = head[name_at..].find(':').map_or(head.len(), |i| name_at + i);
    format!("{}ITEM{}", &head[..name_at], &head[end..])
}

struct MergeTests {
    moving: Vec<bool>,
    moved_ids: Vec<String>,
    helper_ids: Vec<String>,
}

/// The tests that reach only the retired item, with their helpers, and their ids.
fn select_merge_tests(ix: &Index, gone_i: usize, keep_module: &str) -> Result<MergeTests, Failure> {
    let n = ix.syms.len();
    let mut moving = vec![false; n];
    moving[gone_i] = true;
    let mut helpers = vec![];
    move_tests(ix, &mut moving, &mut helpers)?;
    moving[gone_i] = false;
    let target_tests = format!("{keep_module}::tests");
    for i in 0..n {
        let module = ix.syms[i].module.as_str();
        if moving[i] && (module == keep_module || module == target_tests || module.starts_with(&format!("{target_tests}::"))) {
            moving[i] = false;
        }
    }
    loop {
        let mut changed = false;
        for &helper in &helpers {
            if moving[helper] && !ix.used_by[helper].iter().any(|&user| moving[user]) {
                moving[helper] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    helpers.retain(|&i| moving[i]);
    Ok(MergeTests {
        moved_ids: (0..n).filter(|&i| moving[i]).map(|i| ix.syms[i].id.clone()).collect(),
        helper_ids: helpers.iter().map(|&i| ix.syms[i].id.clone()).collect(),
        moving,
    })
}

/// Private helpers that only the retired item used.
fn unused_in_merge(ix: &Index, gone_i: usize, moving: &[bool]) -> Vec<String> {
    (0..ix.syms.len())
        .filter(|&i| {
            i != gone_i
                && !moving[i]
                && helper_kind(&ix.syms[i].kind_text)
                && !is_public(&ix.syms[i].text)
                && !ix.used_by[i].is_empty()
                && ix.used_by[i].iter().all(|&user| user == gone_i)
        })
        .map(|i| ix.syms[i].qname())
        .collect()
}

/// A merge refused because a private survivor is used outside its module.
fn check_merge_visibility(
    ix: &Index,
    keep: &Sym,
    moving: &[bool],
    mentions: &[Site],
    use_gone: &[(String, Site)],
    text: &[String],
    headers: &HashMap<String, String>,
) -> Result<(), Failure> {
    if is_public(&keep.text) {
        return Ok(());
    }
    let mut users: Vec<String> = mentions
        .iter()
        .filter_map(|s| s.symbol)
        .filter(|&i| !moving[i] && ix.syms[i].module != keep.module)
        .map(|i| ix.syms[i].qname())
        .collect();
    for (module, site) in use_gone {
        let original = container_text(&(site.symbol, module.clone()), headers, text);
        if module != &keep.module && is_reexport(&original, &site.range) {
            users.push(format!("the import in {module}"));
        }
    }
    users.sort();
    users.dedup();
    if users.is_empty() {
        Ok(())
    } else {
        fail(format!("E5009 {} is private and {} would use it: make it pub first, then merge", keep.qname(), users.join(", ")))
    }
}

struct MergeImports {
    rewritten: BTreeSet<Container>,
    alias_kept: HashSet<(String, String)>,
    path_only: HashSet<String>,
    hits: usize,
    modules: BTreeSet<String>,
}

/// Every import of the retired item now imports the survivor, or is dropped.
fn rewrite_merge_imports(
    ix: &Index,
    keep: &Sym,
    moving: &[bool],
    mentions: &[Site],
    use_gone: Vec<(String, Site)>,
    text: &mut [String],
    headers: &mut HashMap<String, String>,
) -> Result<MergeImports, Failure> {
    let rewritten: BTreeSet<Container> = use_gone.iter().map(|(module, site)| container_key(module, site)).collect();
    let stays = |module: &str| mentions.iter().any(|s| s.symbol.is_some_and(|i| ix.syms[i].module == module && !moving[i]));
    let mut alias_kept: HashSet<(String, String)> = HashSet::new();
    let mut path_only: HashSet<String> = HashSet::new();
    let mut actions: std::collections::BTreeMap<Container, Vec<(Site, bool)>> = std::collections::BTreeMap::new();
    for (module, site) in use_gone {
        let key = (site.symbol, module.clone());
        let original = container_text(&key, &headers, &text);
        let redirect = is_reexport(&original, &site.range) || stays(&module);
        actions.entry(key).or_default().push((site, redirect));
    }
    let (mut hits, mut modules) = (0usize, BTreeSet::new());
    for (key, mut group) in actions {
        let module = key.1.clone();
        let original = container_text(&key, &headers, &text);
        let mut body = original.clone();
        let item_ranges = language::rust::import_ranges(&original).unwrap_or_default();
        let mut additions: std::collections::BTreeMap<usize, Vec<String>> = std::collections::BTreeMap::new();
        group.sort_by_key(|(s, _)| std::cmp::Reverse(s.range.start));
        for (site, redirect) in group {
            let path = full_path(&keep.module, &keep.name);
            if !redirect {
                body = drop_import(&body, &site.range, site.alias.as_ref(), site.outside);
                continue;
            }
            if is_reexport(&original, &site.range) {
                hits += 1;
                modules.insert(module.clone());
            }
            if site.outside == 0 {
                body = replace_range(&body, site.range.clone(), &path);
            } else {
                let item =
                    item_ranges.iter().position(|(_, tree)| tree.start <= site.range.start && site.range.end <= tree.end).unwrap_or(0);
                body = remove_member(&body, &site.range, site.alias.as_ref());
                let vis = use_visibility(&original, &site.range);
                let alias = site.alias.as_ref().map(|r| original[r.clone()].to_string()).unwrap_or_default();
                let line = if alias.is_empty() { format!("{vis}use {path};") } else { format!("{vis}use {path} {alias};") };
                additions.entry(item).or_default().push(line);
            }
            if let Via::Import(local) = &site.via {
                if site.alias.is_some() {
                    alias_kept.insert((module.clone(), local.clone()));
                } else if ix.syms.iter().any(|s| s.module == module && s.parent.is_none() && s.name == keep.name && s.id != keep.id) {
                    path_only.insert(module.clone());
                }
            }
        }
        for (item, lines) in additions {
            for line in lines {
                let Ok(ranges) = language::rust::import_ranges(&body) else { break };
                let Some((range, _)) = ranges.get(item) else { break };
                body = add_use_after(&body, range, line.as_str());
            }
        }
        set_container_text(&key, &mut *headers, &mut *text, body);
    }
    Ok(MergeImports { rewritten, alias_kept, path_only, hits, modules })
}

/// Every mention of the retired item now names the survivor.
fn rewrite_merge_mentions(
    ix: &Index,
    keep: &Sym,
    gone_i: usize,
    moving: &[bool],
    alias_kept: &HashSet<(String, String)>,
    path_only: &HashSet<String>,
    mentions: Vec<Site>,
    text: &mut [String],
) -> (usize, BTreeSet<String>) {
    let (mut hits, mut modules) = (0usize, BTreeSet::new());
    let mut edits: HashMap<usize, Vec<(Range<usize>, String)>> = HashMap::new();
    for site in mentions {
        let Some(i) = site.symbol else { continue };
        if i == gone_i {
            continue;
        }
        let module = ix.syms[i].module.clone();
        let path = full_path(&keep.module, &keep.name);
        let replacement = if module == keep.module {
            if ix.lang.bound_names(&ix.syms[i].text).contains(&keep.name) { Some(path) } else { Some(keep.name.clone()) }
        } else if moving[i] || path_only.contains(&module) {
            Some(path)
        } else if let Via::Import(local) = &site.via {
            let written = &ix.syms[i].text[site.range.clone()];
            if written.contains("::") || ix.lang.bound_names(&ix.syms[i].text).contains(&keep.name) {
                Some(path)
            } else if alias_kept.contains(&(module.clone(), local.clone())) {
                None
            } else {
                Some(keep.name.clone())
            }
        } else {
            Some(path)
        };
        hits += 1;
        modules.insert(module);
        if let Some(replacement) = replacement {
            edits.entry(i).or_default().push((site.range, replacement));
        }
    }
    for (i, edits) in edits {
        text[i] = splice(&text[i], edits);
    }
    (hits, modules)
}

struct MergePlan<'a> {
    gone_i: usize,
    keep_i: usize,
    gone_id: &'a str,
    keep_id: &'a str,
    gone_module: &'a str,
    keep_module: &'a str,
    moved_ids: &'a [String],
    helper_ids: &'a [String],
    unused: &'a [String],
    before: &'a HashMap<String, String>,
}

fn apply_merge(s: &Store, ix: &Index, rewrite: &MergeRewrite, plan: &MergePlan<'_>, budget: usize) -> Out {
    for i in (0..ix.syms.len()).filter(|&i| i != plan.gone_i) {
        if rewrite.text[i] != ix.syms[i].text {
            s.db.execute(
                "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                params![ix.syms[i].id, rewrite.text[i], revision(&rewrite.text[i])],
            )?;
            s.history(&ix.syms[i].id, "edit", &ix.syms[i].name)?;
        }
    }
    for (module, header) in &rewrite.headers {
        if ix.modules.iter().find(|m| &m.path == module).map(|m| &m.header) != Some(header) {
            s.set_header(module, header)?;
        }
    }
    s.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [plan.gone_id])?;
    s.history(plan.gone_id, "merged-into", plan.keep_id)?;

    let after = s.index()?;
    let pos: HashMap<&str, usize> = after.syms.iter().enumerate().map(|(i, x)| (x.id.as_str(), i)).collect();
    let mut moved = vec![false; after.syms.len()];
    for id in plan.moved_ids {
        if let Some(&i) = pos.get(id.as_str()) {
            moved[i] = true;
        }
    }
    let helpers: Vec<usize> = plan.helper_ids.iter().filter_map(|id| pos.get(id.as_str()).copied()).collect();
    let moved_any = moved.iter().any(|&b| b);
    let tests = if moved_any { moved_tests_reply(&after, &moved, plan.keep_module) } else { String::new() };
    if moved_any {
        let move_plan = rewrite_move(&after, plan.keep_module.to_string(), None, moved, helpers, vec![], vec![])?;
        apply(s, budget, vec![], move_plan)?;
    }
    let mut todo = vec![plan.gone_module.to_string()];
    if !moved_any {
        todo.push(plan.keep_module.to_string());
    }
    let mut retired = vec![];
    s.rust_prune_modules(&mut todo, &mut retired)?;
    if !retired.is_empty() || !moved_any {
        post_move_check(s, plan.before)?;
    }
    Ok(merge_reply(ix, plan, rewrite, &tests, &retired, budget))
}

fn merge_reply(ix: &Index, plan: &MergePlan<'_>, rewrite: &MergeRewrite, tests: &str, retired: &[String], budget: usize) -> String {
    let plural = |k: usize| if k == 1 { "" } else { "s" };
    let mut out = format!("Merged {} into {}\n", ix.syms[plan.gone_i].qname(), ix.syms[plan.keep_i].qname());
    if rewrite.hits > 0 {
        out += &format!(
            "Redirected {} reference{} in {} module{}\n",
            rewrite.hits,
            plural(rewrite.hits),
            rewrite.modules.len(),
            plural(rewrite.modules.len())
        );
    }
    out += tests;
    if !plan.unused.is_empty() {
        out += &format!("Now unused: {}\n", plan.unused.join(", "));
    }
    if !retired.is_empty() {
        out += &format!("Retired module {}\n", retired.join(", "));
    }
    lockdown::trim_output(&out, budget)
}
