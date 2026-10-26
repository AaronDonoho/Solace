use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;

use rusqlite::params;

use super::Args;
use crate::language::rust_refs::{Site, Via, use_sites};
use crate::rust_submit::{add_mod, rust_descendants, rust_namespace};
use crate::{Failure, Index, Out, Store, fail, is_test, language, lockdown, reach, revision};
mod tests;
mod visibility;
mod impls;
pub(crate) mod merge;

pub(crate) fn extract(s: &Store, p: &Args, budget: usize) -> Out {
    let ix = s.index()?;
    let (target, create) = destination(s, &ix, &module_name(&p.to))?;
    let created_files: Vec<std::path::PathBuf> = create.as_ref().map(|c| s.root.join(&c.file)).into_iter().collect();
    if created_files.iter().any(|f| f.exists()) {
        return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
    }

    let roots = roots(s, &ix, &p.names)?;
    let (moved, helpers) = moving(&ix, &roots)?;
    let n = ix.syms.len();

    let already: Vec<String> = roots.iter().filter(|&&i| ix.syms[i].module == target).map(|&i| ix.syms[i].qname()).collect();
    let going: Vec<usize> = (0..n).filter(|&i| moved[i] && ix.syms[i].module != target && roots.contains(&i)).collect();
    if !(0..n).any(|i| moved[i] && ix.syms[i].module != target) {
        return Ok(format!("{} already in {target}; nothing changed.\n", already.join(", ")));
    }
    let plan = rewrite_move(&ix, target, create, moved, helpers, already, going)?;
    apply(s, budget, created_files, plan)
}

fn module_name(input: &str) -> String {
    let mut name = input.trim().trim_end_matches(".rs").to_string();
    if let Some(rest) = name.strip_prefix("src/") {
        name = rest.to_string();
    }
    name
}

fn movable(ix: &Index, i: usize) -> Result<(), Failure> {
    let sym = &ix.syms[i];
    if sym.kind_text != "impl" && parent_kind(ix, sym) != Some("impl") && sym.parent.is_some() {
        return fail(format!("E6013 {} is nested; select its container to move it", sym.qname()));
    }
    Ok(())
}

fn parent_kind<'a>(ix: &'a Index, sym: &crate::Sym) -> Option<&'a str> {
    let parent = sym.parent.as_deref()?;
    ix.syms.iter().find(|x| x.id == parent).map(|x| x.kind_text.as_str())
}

fn helper_kind(kind: &str) -> bool {
    matches!(kind, "fn" | "const" | "static")
}

fn check_clashes(ix: &Index, moved: &[bool], target: &str) -> Result<(), Failure> {
    for i in (0..ix.syms.len()).filter(|&i| {
        moved[i]
            && ix.syms[i].parent.is_none()
            && ix.syms[i].module != target
            && ix.syms[i].kind_text != "impl"
            && !(is_test(ix, i) && ix.syms[i].kind_text != "mod")
    }) {
        let sym = &ix.syms[i];
        if rust_namespace(&sym.kind_text) & language::rust::TYPE_NAMESPACE != 0
            && ix.modules.iter().any(|m| crate::rust_submit::module_path_named(target, &sym.name, &m.path))
        {
            return fail(format!("E1004 {target}.{} already exists in {target}; {} cannot move there", sym.name, sym.qname()));
        }
        if let Some(j) = (0..ix.syms.len()).find(|&j| {
            !moved[j]
                && ix.syms[j].kind_text != "impl"
                && ix.syms[j].module == target
                && ix.syms[j].name == sym.name
                && crate::rust_submit::rust_namespaces_overlap(&ix.syms[j].kind_text, &ix.syms[j].text, &sym.kind_text, &sym.text)
        }) {
            return fail(format!("E1004 {} already exists in {target}; {} cannot move there", ix.syms[j].qname(), sym.qname()));
        }
        if let Some(j) = (0..ix.syms.len()).find(|&j| {
            j != i
                && moved[j]
                && ix.syms[j].kind_text != "impl"
                && ix.syms[j].parent.is_none()
                && ix.syms[j].module != sym.module
                && ix.syms[j].name == sym.name
                && crate::rust_submit::rust_namespaces_overlap(&ix.syms[j].kind_text, &ix.syms[j].text, &sym.kind_text, &sym.text)
        }) {
            return fail(format!("E1004 both {} and {} move to {target}; give one another name", ix.syms[j].qname(), sym.qname()));
        }
    }
    Ok(())
}

type Container = (Option<usize>, String);

fn rewrite(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    mut newtext: Vec<String>,
    mut newheaders: HashMap<String, String>,
    all_mentioned: &[HashSet<String>],
    external: &HashMap<String, Vec<language::rust_refs::ImportSite>>,
    carried: &[Carry],
    local_sites: &[LocalImport],
    references: &language::rust_refs::References<'_>,
) -> Result<(Vec<String>, HashMap<String, String>, BTreeSet<usize>), Failure> {
    let targets = rewrite_targets(ix, moved);
    let sites: HashMap<usize, Vec<Site>> = targets.iter().map(|&t| (t, language::rust_refs::rewrite_sites_refs(references, t))).collect();
    let use_all = all_use_sites(ix)?;
    let use_keys: HashSet<(Option<usize>, usize, usize)> = use_all.iter().map(|(_, site)| use_key(site)).collect();
    let groups = use_groups(&use_all, &targets);
    let sources: BTreeSet<String> = (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
    let mentioned: Vec<HashSet<String>> = all_mentioned
        .iter()
        .enumerate()
        .map(|(i, names)| if sources.contains(&ix.syms[i].module) { names.clone() } else { HashSet::new() })
        .collect();
    let mut in_sources: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, x) in ix.syms.iter().enumerate().filter(|(_, x)| sources.contains(&x.module) && x.kind_text != "use") {
        in_sources.entry(x.module.clone()).or_default().push(i);
    }
    let cx = Rewrite { ix, moved, newmod, target, targets, sites, use_keys, sources, mentioned, in_sources, external };
    let (mut promote, import_ok) = rewrite_imports(&cx, groups, &mut newtext, &mut newheaders);
    restore_moved_use_imports(&cx, &mut newheaders)?;
    for module in &cx.sources {
        drop_unused_external_imports(&cx, module, &mut newtext, &mut newheaders)?;
        drop_unused_local_imports(&cx, module, local_sites, &mut newtext, &mut newheaders)?;
    }
    carry_imports(ix, moved, target, carried, references, &mut newtext, &mut newheaders)?;
    promote.extend(rewrite_mentions(&cx, &import_ok, &mut newtext));
    let ids = id_indexes(ix);
    promote.retain(|&i| can_promote(ix, &ids, i));
    for &i in &promote {
        if !is_public(&newtext[i]) {
            newtext[i] = make_visible(&newtext[i]);
        }
    }
    Ok((newtext, newheaders, promote))
}

fn container_key(module: &str, site: &Site) -> Container {
    (site.symbol, module.to_string())
}

fn container_text(key: &Container, headers: &HashMap<String, String>, text: &[String]) -> String {
    match key.0 {
        Some(i) => text[i].clone(),
        None => headers.get(&key.1).cloned().unwrap_or_default(),
    }
}

fn set_container_text(key: &Container, headers: &mut HashMap<String, String>, text: &mut [String], body: String) {
    match key.0 {
        Some(i) => text[i] = body,
        None => {
            headers.insert(key.1.clone(), body);
        }
    }
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

fn full_path(module: &str, name: &str) -> String {
    if module == "crate" {
        format!("crate::{name}")
    } else if let Some(rest) = module.strip_prefix("integration::") {
        let mut parts = rest.split("::");
        let _root = parts.next();
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() { format!("crate::{name}") } else { format!("crate::{}::{name}", rest.join("::")) }
    } else {
        format!("crate::{module}::{name}")
    }
}

fn leaf(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn last_root_sep(ix: &Index, module: &str) -> Option<String> {
    ix.syms.iter().filter(|s| s.module == module && s.parent.is_none()).last().and_then(|s| s.sep.clone())
}

fn replace_range(text: &str, range: Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &text[..range.start], &text[range.end..])
}

fn splice(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
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

fn remove_member(text: &str, range: &Range<usize>, alias: Option<&Range<usize>>) -> String {
    let start = range.start;
    let end = alias.map_or(range.end, |a| a.end);
    let after = &text[end..];
    if let Some(pos) = after.find(|c| c == ',' || c == '}')
        && after.as_bytes()[pos] == b','
    {
        return format!("{}{}", &text[..start], &text[end + pos + 1..]);
    }
    let before = &text[..start];
    if let Some(pos) = before.rfind(',') {
        return format!("{}{}", &text[..pos], &text[end..]);
    }
    format!("{}{}", &text[..start], &text[end..])
}

fn remove_use_line(text: &str, range: &Range<usize>) -> String {
    let Ok(ranges) = language::rust::import_ranges(text) else {
        return text.to_string();
    };
    let Some((item, _)) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end) else {
        return text.to_string();
    };
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
    format!("{}{}", &text[..start], &text[end..])
}

enum Visibility {
    Private,
    Restricted,
    Public,
}

fn is_public(text: &str) -> bool {
    !matches!(declaration_visibility(text), Visibility::Private | Visibility::Restricted)
}

fn declaration(text: &str) -> Option<(usize, &str)> {
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

fn make_visible(text: &str) -> String {
    match declaration_visibility(text) {
        Visibility::Public => text.to_string(),
        Visibility::Restricted => {
            let Some((at, code)) = declaration(text) else { return text.to_string() };
            let end = code.find(')').map_or(at, |i| at + i + 1);
            format!("{}pub(crate){}", &text[..at], &text[end..])
        }
        Visibility::Private => {
            let Some((at, _)) = declaration(text) else { return text.to_string() };
            format!("{}pub(crate) {}", &text[..at], &text[at..])
        }
    }
}

fn is_reexport(text: &str, range: &Range<usize>) -> bool {
    !use_visibility(text, range).is_empty()
}

/// The visibility written before the `use` whose tree holds `range`, such as `pub ` or `pub(crate) `.
fn use_visibility(text: &str, range: &Range<usize>) -> String {
    let Ok(ranges) = language::rust::import_ranges(text) else {
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

/// Inserts a new `use` line after the import item that holds `range`, keeping
/// an inline module's braces and indentation.
fn add_use_line(text: &str, range: &Range<usize>, line: &str) -> String {
    let Ok(ranges) = language::rust::import_ranges(text) else {
        return format!("{text}{line}\n");
    };
    let Some((item, _)) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end) else {
        return format!("{text}{line}\n");
    };
    add_use_after(text, &item, line)
}

/// Imports from another crate that the moved code uses: a named import it
/// mentions, a macro it calls, or a glob that may bind a name it mentions.
fn external_imports(
    ix: &Index,
    moved: &[bool],
    refs: &language::rust_refs::References<'_>,
) -> Result<(Vec<Carry>, HashMap<String, Vec<language::rust_refs::ImportSite>>), Failure> {
    let mut by_module: HashMap<String, Vec<language::rust_refs::ImportSite>> = HashMap::new();
    let mut carried = vec![];
    let modules: BTreeSet<String> = (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
    for module in modules {
        let sites = language::rust_refs::references_external_imports(refs, &module)?;
        let module_moved: Vec<usize> = (0..ix.syms.len()).filter(|&i| moved[i] && ix.syms[i].module == module).collect();
        let named: HashSet<String> = language::rust_refs::import_sites(&ix.syms, &ix.modules, &module, true)?
            .into_iter()
            .filter(|site| !site.glob)
            .map(|site| site.local)
            .collect();
        let needs_glob = module_moved.iter().any(|&i| names_need_glob(ix, &module, &refs.mentioned[i], &named));
        for site in &sites {
            let needed = if site.glob { needs_glob } else { module_moved.iter().any(|&i| refs.mentioned[i].contains(&site.local)) };
            if needed {
                carried.push(Carry { from: module.clone(), site: site.clone(), rename: true });
            }
        }
        by_module.insert(module, sites);
    }
    Ok((carried, by_module))
}

fn destination(s: &Store, ix: &Index, to: &str) -> Result<(String, Option<crate::rust_submit::RustNew>), Failure> {
    match s.resolve_module(ix, to) {
        Some(row) => Ok((row.path.clone(), None)),
        None => match s.rust_created(ix, to) {
            Some(new) => Ok((new.module.clone(), Some(new))),
            None => fail(format!("E1003 {to} is not a module name; use --to <module>, e.g. --to helpers")),
        },
    }
}

fn roots(s: &Store, ix: &Index, names: &[String]) -> Result<Vec<usize>, Failure> {
    let mut roots: Vec<usize> = vec![];
    for name in names {
        let i = match impls::named_impl(ix, name)? {
            Some(i) => i,
            None => match s.resolve(ix, name) {
                Ok(i) => i,
                Err(e) => {
                    if s.resolve_module(ix, name).is_some() {
                        return fail(super::MODULE_HINT);
                    }
                    return Err(e);
                }
            },
        };
        if !roots.contains(&i) {
            roots.push(i);
        }
    }
    for &i in &roots {
        movable(ix, i)?;
    }
    Ok(roots)
}

fn moving(ix: &Index, roots: &[usize]) -> Result<(Vec<bool>, Vec<usize>), Failure> {
    let n = ix.syms.len();
    let mut moved = vec![false; n];
    for &i in roots {
        mark(ix, &mut moved, i);
    }
    impls::check_direct(ix, roots, &mut moved)?;
    add_impls(ix, &mut moved)?;
    let mut helpers: Vec<usize> = vec![];
    bring_helpers(ix, &mut moved, &mut helpers);
    move_tests(ix, &mut moved, &mut helpers)?;
    Ok((moved, helpers))
}

fn rewrite_targets(ix: &Index, moved: &[bool]) -> BTreeSet<usize> {
    let n = ix.syms.len();
    let mut targets: BTreeSet<usize> = (0..n).filter(|&i| moved[i]).collect();
    for i in (0..n).filter(|&i| moved[i]) {
        for &u in &ix.uses[i] {
            if !moved[u] {
                targets.insert(u);
            }
        }
    }
    targets
}

fn all_use_sites(ix: &Index) -> Result<Vec<(String, Site)>, Failure> {
    let mut out = vec![];
    for m in &ix.modules {
        for site in use_sites(&ix.syms, &ix.modules, &m.path)? {
            out.push((m.path.clone(), site));
        }
    }
    Ok(out)
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
) -> (BTreeSet<usize>, HashSet<(usize, String, String)>) {
    let (ix, moved, newmod, target) = (cx.ix, cx.moved, cx.newmod, cx.target);
    let (sites, use_keys) = (&cx.sites, &cx.use_keys);
    let mut import_ok: HashSet<(usize, String, String)> = HashSet::new();
    let mut promote: BTreeSet<usize> = BTreeSet::new();
    for (key, mut group) in groups {
        group.sort_by(|a, b| b.range.start.cmp(&a.range.start));
        let original = container_text(&key, &*newheaders, &*newtext);
        let mut text = original.clone();
        let mut additions: Vec<(Range<usize>, String)> = vec![];
        for site in &group {
            let alias = alias_of(site);
            let use_new = match key.0 {
                Some(i) => newmod[i].clone(),
                None => key.1.clone(),
            };
            let needed = is_reexport(&original, &site.range)
                || sites[&site.target].iter().any(|m| {
                    let Some(mi) = m.symbol else { return false };
                    !is_import_path(m, &ix.syms[mi].text)
                        && !use_keys.contains(&use_key(m))
                        && matches!(&m.via, Via::Import(a) if Some(a) == alias.as_ref())
                        && newmod[mi] == use_new
                });
            if needed {
                if let Some(a) = &alias {
                    import_ok.insert((site.target, a.clone(), use_new.clone()));
                }
                if moved[site.target] {
                    let path = full_path(target, &ix.syms[site.target].name);
                    if use_new != target {
                        promote.insert(site.target);
                    }
                    if site.outside == 0 {
                        text = replace_range(&text, site.range.clone(), &path);
                    } else {
                        text = remove_member(&text, &site.range, site.alias.as_ref());
                        let alias_text = site.alias.as_ref().map(|r| original[r.clone()].to_string()).unwrap_or_default();
                        let vis = use_visibility(&original, &site.range);
                        additions.push((site.range.clone(), format!("{vis}use {path}{alias_text};")));
                    }
                }
            } else if moved[site.target] {
                text = drop_import(&text, &site.range, site.alias.as_ref(), site.outside);
            } else {
                let module = match key.0 {
                    Some(i) => &ix.syms[i].module,
                    None => &key.1,
                };
                let unused = alias.as_ref().is_some_and(|a| unused_after_move(cx, module, a));
                if unused && !allows_unused(&original, &site.range) {
                    text = drop_import(&text, &site.range, site.alias.as_ref(), site.outside);
                }
            }
        }
        for (range, line) in additions {
            text = add_use_line(&text, &range, &line);
        }
        set_container_text(&key, &mut *newheaders, &mut *newtext, text);
    }
    (promote, import_ok)
}

fn rewrite_mentions(cx: &Rewrite, import_ok: &HashSet<(usize, String, String)>, newtext: &mut [String]) -> BTreeSet<usize> {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let (targets, sites, use_keys) = (&cx.targets, &cx.sites, &cx.use_keys);
    let mut edits: HashMap<usize, Vec<(Range<usize>, String)>> = HashMap::new();
    let mut promote: BTreeSet<usize> = BTreeSet::new();
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
            let parent_impl = ix.syms[t].parent.as_deref().and_then(|p| ix.syms.iter().position(|s| s.id == p && s.kind_text == "impl"));
            if let Some(at) = parent_impl {
                if matches!(&site.via, Via::Path) && site.segs.len() > 1 {
                    if let Ok(owner) = impl_owner_symbol(ix, at) {
                        let owner_module = &ix.syms[owner].module;
                        if &mention_new != owner_module {
                            if let Some(sep) = ix.syms[i].text[site.range.clone()].rfind("::") {
                                edits
                                    .entry(i)
                                    .or_default()
                                    .push((site.range.start..site.range.start + sep, full_path(owner_module, &ix.syms[owner].name)));
                            }
                        }
                    }
                }
                continue;
            }
            let target_new = newmod[t].clone();
            let written = &ix.syms[i].text[site.range.clone()];
            let mut replacement: Option<String> = None;
            let mut needs_visibility = false;
            match &site.via {
                Via::Import(alias) => {
                    if written.contains("::") {
                        if moved[t] {
                            replacement = Some(full_path(&target_new, &ix.syms[t].name));
                            needs_visibility = target_new != mention_new;
                        }
                    } else if import_ok.contains(&(t, alias.clone(), mention_new.clone()))
                        || target_new == mention_new
                        || sites[&t].iter().any(|m| {
                            m.symbol == site.symbol && is_import_path(m, &ix.syms[i].text) && matches!(&m.via, Via::Import(a) if a == alias)
                        })
                    {
                    } else {
                        replacement = Some(full_path(&target_new, &ix.syms[t].name));
                        needs_visibility = true;
                    }
                }
                Via::Path => {
                    if target_new == mention_new {
                        replacement = Some(leaf(&ix.syms[t].name).to_string());
                    } else {
                        replacement = Some(full_path(&target_new, &ix.syms[t].name));
                        needs_visibility = true;
                    }
                }
                Via::Own | Via::Glob => {
                    if target_new != mention_new {
                        replacement = Some(full_path(&target_new, &ix.syms[t].name));
                        needs_visibility = true;
                    }
                }
            }
            if let Some(text) = replacement {
                edits.entry(i).or_default().push((site.range.clone(), text));
            }
            if needs_visibility {
                promote.insert(t);
            }
        }
    }
    for (i, es) in edits {
        newtext[i] = splice(&newtext[i], es);
    }
    promote
}

struct RustMove<'a> {
    ix: &'a Index,
    refs: language::rust_refs::References<'a>,
    target: String,
    create: Option<crate::rust_submit::RustNew>,
    moved: Vec<bool>,
    helpers: Vec<usize>,
    already: Vec<String>,
    going: Vec<usize>,
    newtext: Vec<String>,
    newheaders: HashMap<String, String>,
    promote: BTreeSet<usize>,
    placements: Vec<impls::Placement>,
}

fn apply(s: &Store, budget: usize, mut created_files: Vec<std::path::PathBuf>, mut m: RustMove<'_>) -> Out {
    let ix = m.ix;
    let n = ix.syms.len();
    let ids = id_indexes(ix);
    let mut moved = m.moved.clone();
    let whole = whole_test_containers(ix, &mut moved);
    let existing_tests = test_module(ix, &m.target);
    let file_tests = existing_tests.is_some();
    let groups = test_groups(ix, &moved, &whole, file_tests);
    let test = plan_test_module(s, &mut m, existing_tests, &groups, &moved)?;
    if let Some(t) = &test
        && t.created
    {
        created_files.push(s.root.join(&t.file));
    }
    let old_modules: Vec<String> = {
        let mut v: Vec<String> = (0..n).filter(|&i| moved[i] && ix.syms[i].module != m.target).map(|i| ix.syms[i].module.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    let source_ends: HashMap<String, Option<String>> =
        old_modules.iter().map(|module| (module.clone(), last_root_sep(ix, module))).collect();
    let target_end = last_root_sep(ix, &m.target);
    let moving_roots: Vec<usize> = (0..n)
        .filter(|&i| {
            moved[i]
                && ix.syms[i].module != m.target
                && !groups.iter().any(|g| g.children.contains(&i))
                && !(file_tests && whole.contains(&i))
                && ix.syms[i].parent.as_deref().is_none_or(|p| ids.get(p).is_none_or(|&p| !moved[p]))
        })
        .collect();
    let before = s.broken()?;
    let mut retired: Vec<String> = vec![];

    check_test_containers(ix, &moved, &m.target, &whole, &groups, &ids)?;
    if let Some(t) = &test {
        carry_test_imports(ix, &moved, &ids, &m.refs, &groups, t, &mut m.newtext, &mut m.newheaders)?;
    }

    let out = s.transaction(&created_files, || {
        if let Some(c) = &m.create {
            let owns = language::rust::owns_dir(&c.module, &c.file);
            s.db.execute(
                "INSERT INTO modules(path, dir, header, file, owns_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![c.module, c.dir, c.header, c.file, owns],
            )?;
        }
        if let Some(t) = &test {
            if t.created {
                let owns = language::rust::owns_dir(&t.module, &t.file);
                s.db.execute(
                    "INSERT INTO modules(path, dir, header, file, owns_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![t.module, t.dir, t.header, t.file, owns],
                )?;
            }
        }
        drop_super_globs(s, ix, &moved, &m.target, &whole, &m.newtext)?;

        let existing_target =
            ix.syms.iter().enumerate().filter(|(j, x)| x.module == m.target && x.parent.is_none() && !moved[*j]).map(|(j, _)| j).last();
        if let Some(j) = existing_target {
            s.db.execute("UPDATE symbols SET sep = ?2 WHERE id = ?1", params![ix.syms[j].id, "\n\n"])?;
        }
        let mut ord: i64 =
            s.db.query_row("SELECT COALESCE(MAX(ord), 0) FROM symbols WHERE live = 1 AND module = ?1", [&m.target], |r| r.get(0))?;
        let parents = impls::place_target_impls(s, ix, &m.target, &m.placements, &mut ord)?;
        move_roots(s, ix, &moving_roots, &m.target, target_end.as_deref(), &parents, &mut ord)?;
        if let Some(t) = &test {
            move_test_groups(s, ix, &groups, &t.module)?;
        }
        if file_tests {
            retire_test_containers(s, ix, &whole)?;
        }

        let mut touched = old_modules.clone();
        touched.push(m.target.clone());

        let now = s.index()?;
        for (module, end) in &source_ends {
            if let Some(x) = now.syms.iter().filter(|x| x.module == *module && x.parent.is_none()).last() {
                s.db.execute("UPDATE symbols SET sep = ?2 WHERE id = ?1", params![x.id, end.clone().unwrap_or_else(|| "\n".to_string())])?;
            }
        }

        for i in 0..n {
            if m.newtext[i] != ix.syms[i].text {
                s.db.execute(
                    "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                    params![ix.syms[i].id, m.newtext[i], revision(&m.newtext[i])],
                )?;
                s.history(&ix.syms[i].id, "edit", &ix.syms[i].name)?;
            }
        }
        for (module, header) in &m.newheaders {
            if ix.modules.iter().find(|m| &m.path == module).map(|m| &m.header) != Some(header) {
                s.set_header(module, header)?;
            }
        }

        let dropped = s.rust_prune_impls(&touched)?;
        let mut emptied = old_modules.clone();
        s.rust_prune_modules(&mut emptied, &mut retired)?;

        post_move_check(s, &before)?;
        Ok(move_reply(&m, &moved, &retired, budget) + &crate::rust_submit::dropped_impls_note(&dropped))
    })?;
    Ok(out)
}

/// `text` without the import at `site`: its whole `use` line, or its member of a group.
fn drop_import(text: &str, range: &Range<usize>, alias: Option<&Range<usize>>, outside: usize) -> String {
    if outside == 0 { remove_use_line(text, range) } else { remove_member(text, range, alias) }
}

/// Whether code moving out of `module` mentioned `name` and nothing staying there does.
fn unused_after_move(cx: &Rewrite, module: &str, name: &str) -> bool {
    let mentions = |i: usize| cx.mentioned[i].contains(name);
    let in_module = cx.in_sources.get(module).map(Vec::as_slice).unwrap_or_default();
    let stays = |i: usize| !cx.moved[i] || cx.newmod[i] == module;
    in_module.iter().any(|&i| cx.moved[i] && mentions(i)) && !in_module.iter().any(|&i| stays(i) && mentions(i))
}

/// Whether the `use` whose tree holds `range` carries an `unused_imports` allowance.
fn allows_unused(text: &str, range: &Range<usize>) -> bool {
    let Ok(ranges) = language::rust::import_ranges(text) else {
        return true;
    };
    ranges
        .into_iter()
        .find(|(_, tree)| tree.start <= range.start && range.end <= tree.end)
        .is_none_or(|(item, tree)| text[item.start..tree.start].contains("unused_imports"))
}

/// What a move's rewrite reads: the index, where each symbol ends up, and the sites it rewrites.
struct Rewrite<'a> {
    ix: &'a Index,
    moved: &'a [bool],
    newmod: &'a [String],
    target: &'a str,
    targets: BTreeSet<usize>,
    sites: HashMap<usize, Vec<Site>>,
    use_keys: HashSet<(Option<usize>, usize, usize)>,
    sources: BTreeSet<String>,
    mentioned: Vec<HashSet<String>>,
    in_sources: HashMap<String, Vec<usize>>,
    external: &'a HashMap<String, Vec<language::rust_refs::ImportSite>>,
}

/// The source module without the imports from another crate that only the code
/// moving out of it used.
fn drop_unused_external_imports(
    cx: &Rewrite,
    module: &str,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let external: Vec<language::rust_refs::ImportSite> = cx.external.get(module).cloned().unwrap_or_default();
    if let Some(header) = newheaders.get(module).cloned() {
        let kept = drop_unused_imports(cx, module, &header, &external)?;
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
        let kept = drop_unused_imports(cx, module, &newtext[i], &external)?;
        newtext[i] = kept;
    }
    Ok(())
}

/// `text` without the external imports that only code moving out of `module`
/// mentions, keeping re-exports and imports allowed to be unused.
fn drop_unused_imports(cx: &Rewrite, module: &str, text: &str, external: &[language::rust_refs::ImportSite]) -> Result<String, Failure> {
    let glob_unused = external.iter().any(|site| site.glob) && !staying_needs_glob(cx, module)?;
    let keys: HashSet<(String, Vec<String>, bool)> = external
        .iter()
        .filter(|site| if site.glob { glob_unused } else { unused_after_move(cx, module, &site.local) })
        .map(|site| (site.local.clone(), site.path.clone(), site.glob))
        .collect();
    drop_import_keys(text, module, &keys)
}

fn can_promote(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> bool {
    match ix.syms[i].parent.as_deref() {
        None => true,
        Some(parent) => ids.get(parent).is_some_and(|&p| ix.syms[p].kind_text == "impl" && !ix.syms[p].name.starts_with('<')),
    }
}

fn mark(ix: &Index, moved: &mut [bool], i: usize) {
    for j in rust_descendants(ix, i) {
        moved[j] = true;
    }
}

fn add_impls(ix: &Index, moved: &mut [bool]) -> Result<(), Failure> {
    let impls: Vec<usize> = (0..ix.syms.len()).filter(|&i| !moved[i] && ix.syms[i].kind_text == "impl").collect();
    let owners = crate::language::rust_refs::impl_owner_batch(&ix.syms, &ix.modules, &impls);
    for (i, owner) in impls.into_iter().zip(owners) {
        match owner {
            crate::language::rust_refs::ImplOwner::One(t) if moved[t] => mark(ix, moved, i),
            crate::language::rust_refs::ImplOwner::Ambiguous(ts) if ts.iter().any(|&t| moved[t]) => {
                let list: Vec<String> = ts.iter().map(|&t| ix.syms[t].qname()).collect();
                return fail(format!(
                    "E6013 {} is an impl whose type is ambiguous: {}; qualify its self type",
                    ix.syms[i].qname(),
                    list.join(", ")
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn bring_helpers(ix: &Index, moved: &mut [bool], helpers: &mut Vec<usize>) {
    let mut moving = HashSet::new();
    bring_helpers_with(ix, moved, helpers, &mut moving);
}

fn test_container(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> Option<usize> {
    let mut parent = ix.syms[i].parent.as_deref();
    while let Some(id) = parent {
        let k = *ids.get(id)?;
        if ix.syms[k].kind_text == "mod" {
            return Some(k);
        }
        parent = ix.syms[k].parent.as_deref();
    }
    None
}

fn test_covered(ix: &Index, ids: &HashMap<&str, usize>, t: usize) -> (HashSet<usize>, HashSet<usize>) {
    let container = test_container(ix, ids, t);
    let local = |j: usize| {
        is_test(ix, j)
            && !reach::is_test_fn(ix, j)
            && match container {
                Some(c) => ix.syms[j].parent.as_deref() == Some(ix.syms[c].id.as_str()),
                None => ix.syms[j].module == ix.syms[t].module,
            }
    };
    let (mut out, mut locals, mut seen, mut stack) = (HashSet::new(), HashSet::new(), HashSet::new(), vec![t]);
    while let Some(j) = stack.pop() {
        if !seen.insert(j) {
            continue;
        }
        for &u in &ix.uses[j] {
            if u == t || matches!(ix.syms[u].kind_text.as_str(), "mod" | "use") {
                continue;
            }
            if local(u) {
                locals.insert(u);
                stack.push(u);
            } else {
                out.insert(u);
            }
        }
    }
    (out, locals)
}

fn move_tests(ix: &Index, moved: &mut [bool], helpers: &mut Vec<usize>) -> Result<(), Failure> {
    let tests: Vec<usize> = reach::test_symbols(ix).into_iter().filter(|&t| !moved[t]).collect();
    if tests.is_empty() {
        return Ok(());
    }
    let ids = id_indexes(ix);
    let covered: HashMap<usize, (HashSet<usize>, HashSet<usize>)> = tests.iter().map(|&t| (t, test_covered(ix, &ids, t))).collect();
    let candidates: Vec<usize> = tests.iter().copied().filter(|t| covered[t].0.iter().any(|&u| moved[u])).collect();
    if candidates.is_empty() {
        return Ok(());
    }
    let candidate_set: HashSet<usize> = candidates.iter().copied().collect();
    let locals: HashSet<usize> = candidates.iter().flat_map(|t| covered[t].1.iter().copied()).collect();
    let mut movable_locals: HashSet<usize> = HashSet::new();
    loop {
        let mut added = false;
        for &l in &locals {
            if movable_locals.contains(&l) {
                continue;
            }
            if ix.used_by[l].iter().all(|&u| moved[u] || candidate_set.contains(&u) || movable_locals.contains(&u)) {
                movable_locals.insert(l);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut moving: HashSet<usize> = candidate_set.iter().chain(movable_locals.iter()).copied().collect();
    bring_helpers_with(ix, moved, helpers, &mut moving);
    for &t in &candidates {
        let (out, locs) = &covered[&t];
        let staying: Vec<usize> = out.iter().copied().filter(|&u| !moved[u]).collect();
        let staying_locals: Vec<usize> = locs.iter().copied().filter(|u| !movable_locals.contains(u)).collect();
        if !staying.is_empty() || !staying_locals.is_empty() {
            let moving: Vec<String> = out.iter().filter(|&&u| moved[u]).map(|&u| ix.syms[u].qname()).collect();
            let stays: Vec<String> = staying.iter().chain(staying_locals.iter()).map(|&u| ix.syms[u].qname()).collect();
            return fail(format!(
                "E5002 {} reaches {}, which moves, and {}, which stays; move the staying code too or move less",
                reach::libtest_name(ix, t),
                if moving.is_empty() { "moved code".to_string() } else { moving.join(", ") },
                stays.join(", ")
            ));
        }
    }
    for &t in &candidates {
        mark(ix, moved, t);
        for &l in &covered[&t].1 {
            mark(ix, moved, l);
        }
    }
    bring_helpers(ix, moved, helpers);
    Ok(())
}

struct TestGroup {
    children: Vec<usize>,
}

fn is_test_container(ix: &Index, i: usize) -> bool {
    ix.syms[i].kind_text == "mod" && is_test(ix, i)
}

fn whole_test_containers(ix: &Index, moved: &mut [bool]) -> HashSet<usize> {
    let mut whole = HashSet::new();
    for c in 0..ix.syms.len() {
        if !is_test_container(ix, c) {
            continue;
        }
        if moved[c] {
            whole.insert(c);
            continue;
        }
        let mut any = false;
        let mut all = true;
        for (j, sym) in ix.syms.iter().enumerate().filter(|(_, s)| s.parent.as_deref() == Some(ix.syms[c].id.as_str())) {
            if moved[j] {
                any = true;
            } else if sym.kind_text != "use" {
                all = false;
            }
        }
        if any && all {
            mark(ix, moved, c);
            whole.insert(c);
        }
    }
    whole
}

fn test_groups(ix: &Index, moved: &[bool], whole: &HashSet<usize>, file_tests: bool) -> Vec<TestGroup> {
    let mut groups: std::collections::BTreeMap<String, TestGroup> = std::collections::BTreeMap::new();
    let ids = id_indexes(ix);
    for i in 0..ix.syms.len() {
        if !moved[i] || matches!(ix.syms[i].kind_text.as_str(), "mod" | "use") || !is_test(ix, i) {
            continue;
        }
        let parent = ix.syms[i].parent.as_deref().and_then(|id| ids.get(id).copied());
        match parent {
            Some(p) if whole.contains(&p) && !file_tests => {}
            Some(p) if moved[p] && !whole.contains(&p) => {}
            Some(p) if ix.syms[p].kind_text == "mod" => {
                groups.entry(format!("inline:{p}")).or_insert_with(|| TestGroup { children: vec![] }).children.push(i);
            }
            _ => {
                groups.entry(format!("module:{}", ix.syms[i].module)).or_insert_with(|| TestGroup { children: vec![] }).children.push(i);
            }
        }
    }
    groups.into_values().collect()
}

fn target_has(ix: &Index, moved: &[bool], target: &str, name: &str) -> bool {
    ix.syms.iter().enumerate().any(|(i, s)| !moved[i] && s.module == target && s.parent.is_none() && s.name == name && s.kind_text == "mod")
}

fn rename_subtree(s: &Store, ix: &Index, root: usize, new_name: &str, target: &str) -> Result<(), Failure> {
    let old = ix.syms[root].name.clone();
    let prefix = format!("{old}::");
    for j in rust_descendants(ix, root) {
        let name = if j == root {
            new_name.to_string()
        } else {
            ix.syms[j].name.strip_prefix(&prefix).map_or_else(|| ix.syms[j].name.clone(), |rest| format!("{new_name}::{rest}"))
        };
        s.db.execute("UPDATE symbols SET module = ?2, name = ?3 WHERE id = ?1", params![ix.syms[j].id, target, name])?;
    }
    Ok(())
}

/// A moved trait impl names a staying trait by its path from the new module.
fn qualify_trait_impls(ix: &Index, moved: &[bool], newmod: &[String], newtext: &mut [String]) {
    for i in 0..ix.syms.len() {
        if !moved[i] || ix.syms[i].kind_text != "impl" {
            continue;
        }
        let Some((t, _)) = crate::language::rust_refs::impl_trait(&ix.syms, &ix.modules, i) else { continue };
        if newmod[t] == newmod[i] {
            continue;
        }
        if let Some(range) = impl_trait_range(&newtext[i]) {
            newtext[i].replace_range(range, &full_path(&newmod[t], &ix.syms[t].name));
        }
    }
}

/// The trait path's name range in an impl header, before its generic arguments.
fn impl_trait_range(text: &str) -> Option<Range<usize>> {
    let item = match syn::parse_str::<syn::ItemImpl>(text) {
        Ok(item) => item,
        Err(_) if text.trim_end().ends_with('{') => syn::parse_str::<syn::ItemImpl>(&format!("{text}\n}}")).ok()?,
        Err(_) => return None,
    };
    let (path, _) = item.trait_?;
    let first = path.segments.first()?.ident.span().byte_range();
    let last = path.segments.last()?.ident.span().byte_range();
    Some(first.start..last.end)
}

fn bring_helpers_with(ix: &Index, moved: &mut [bool], helpers: &mut Vec<usize>, moving: &mut HashSet<usize>) {
    loop {
        let mut added = false;
        for i in 0..ix.syms.len() {
            let sym = &ix.syms[i];
            if moved[i] || sym.parent.is_some() || is_test(ix, i) || !helper_kind(&sym.kind_text) || is_public(&sym.text) {
                continue;
            }
            if ix.used_by[i].is_empty() || !ix.used_by[i].iter().any(|&u| moved[u] || moving.contains(&u)) {
                continue;
            }
            if ix.used_by[i].iter().all(|&u| moved[u] || moving.contains(&u) || u == i) {
                mark(ix, moved, i);
                helpers.push(i);
                moving.insert(i);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
}

fn id_indexes(ix: &Index) -> HashMap<&str, usize> {
    ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect()
}

fn check_test_containers(
    ix: &Index,
    moved: &[bool],
    target: &str,
    whole: &HashSet<usize>,
    groups: &[TestGroup],
    ids: &HashMap<&str, usize>,
) -> Result<(), Failure> {
    let mut names = HashSet::new();
    let declared = ix.modules.iter().find(|m| m.path == target).and_then(|m| header_mod_names(&m.header).ok()).unwrap_or_default();
    let existing = test_module(ix, target).map(|m| m.path.clone());
    for &c in whole {
        if existing.is_some() {
            continue;
        }
        if ix.syms[c].module == target {
            continue;
        }
        let parent_moved = ix.syms[c].parent.as_deref().is_some_and(|p| ids.get(p).is_some_and(|&p| moved[p]));
        if parent_moved {
            continue;
        }
        let name = if ix.syms[c].parent.is_some() { leaf(&ix.syms[c].name).to_string() } else { ix.syms[c].name.clone() };
        if !names.insert(name.clone()) {
            return fail(format!("E1004 two test containers would be named {name} in {target}"));
        }
        if target_has(ix, moved, target, &name) || declared.contains(&name) {
            return fail(format!("E1004 {target}.{name} already exists in {target}"));
        }
    }
    if groups.is_empty() {
        return Ok(());
    }
    if target_has(ix, moved, target, "tests")
        || (existing.is_none()
            && (declared.iter().any(|n| n == "tests") || ix.modules.iter().any(|m| m.path == format!("{target}::tests"))))
    {
        return fail(format!("E1004 {target}.tests already exists in {target}"));
    }
    if !names.insert("tests".to_string()) {
        return fail(format!("E1004 two test containers would be named tests in {target}"));
    }
    let mut tests = HashSet::new();
    for g in groups {
        for &i in &g.children {
            let name = leaf(&ix.syms[i].name).to_string();
            if !tests.insert(name.clone()) {
                return fail(format!("E1004 two tests would be named {name} in {target}::tests"));
            }
            if let Some(module) = &existing
                && ix.syms.iter().any(|s| &s.module == module && s.parent.is_none() && s.name == name)
            {
                return fail(format!("E1004 {target}::tests.{name} already exists in {target}::tests"));
            }
        }
    }
    Ok(())
}

fn move_roots(
    s: &Store,
    ix: &Index,
    roots: &[usize],
    target: &str,
    target_end: Option<&str>,
    parents: &HashMap<usize, String>,
    ord: &mut i64,
) -> Result<(), Failure> {
    let mut child_ord: HashMap<String, i64> = HashMap::new();
    for (k, &i) in roots.iter().enumerate() {
        let sym = &ix.syms[i];
        if let Some(parent) = parents.get(&i) {
            let next = match child_ord.get(parent) {
                Some(next) => *next,
                None => {
                    let max: i64 = s.db.query_row(
                        "SELECT COALESCE(MAX(ord), 0) FROM symbols WHERE live = 1 AND parent = ?1",
                        [parent.as_str()],
                        |r| r.get(0),
                    )?;
                    max + 1
                }
            };
            child_ord.insert(parent.clone(), next + 1);
            s.db.execute(
                "UPDATE symbols SET module = ?2, parent = ?3, ord = ?4, sep = ?5 WHERE id = ?1",
                params![sym.id, target, parent, next, "\n"],
            )?;
            for j in rust_descendants(ix, i).into_iter().filter(|&j| j != i) {
                s.db.execute("UPDATE symbols SET module = ?2 WHERE id = ?1", params![ix.syms[j].id, target])?;
            }
            s.history(&sym.id, "move", &sym.module)?;
            continue;
        }
        *ord += 1;
        let sep = if k + 1 == roots.len() { target_end.unwrap_or("\n").to_string() } else { "\n\n".to_string() };
        let detach = sym.parent.is_some();
        let rename = is_test_container(ix, i) && detach;
        let name = if rename { leaf(&sym.name).to_string() } else { sym.name.clone() };
        s.db.execute(
            "UPDATE symbols SET module = ?2, ord = ?3, sep = ?4, parent = NULL, name = ?5 WHERE id = ?1",
            params![sym.id, target, *ord, sep, name],
        )?;
        if rename {
            rename_subtree(s, ix, i, &name, target)?;
        } else {
            for j in rust_descendants(ix, i).into_iter().filter(|&j| j != i) {
                s.db.execute("UPDATE symbols SET module = ?2 WHERE id = ?1", params![ix.syms[j].id, target])?;
            }
        }
        s.history(&sym.id, "move", &sym.module)?;
    }
    Ok(())
}

fn move_test_groups(s: &Store, ix: &Index, groups: &[TestGroup], module: &str) -> Result<(), Failure> {
    let mut ord: i64 =
        s.db.query_row("SELECT COALESCE(MAX(ord), 0) FROM symbols WHERE live = 1 AND module = ?1", [module], |r| r.get(0))?;
    for g in groups {
        for &i in &g.children {
            ord += 1;
            let old = ix.syms[i].module.clone();
            let prefix = format!("{}::", ix.syms[i].name);
            let name = leaf(&ix.syms[i].name).to_string();
            s.db.execute(
                "UPDATE symbols SET module = ?2, parent = NULL, name = ?3, ord = ?4 WHERE id = ?1",
                params![ix.syms[i].id, module, name, ord],
            )?;
            for j in rust_descendants(ix, i).into_iter().filter(|&j| j != i) {
                let name = ix.syms[j].name.strip_prefix(&prefix).map_or_else(|| ix.syms[j].name.clone(), |rest| format!("{name}::{rest}"));
                s.db.execute("UPDATE symbols SET module = ?2, name = ?3 WHERE id = ?1", params![ix.syms[j].id, module, name])?;
            }
            s.history(&ix.syms[i].id, "move", &old)?;
        }
    }
    Ok(())
}

fn moved_tests_reply(ix: &Index, moved: &[bool], target: &str) -> String {
    let tests: Vec<usize> = (0..ix.syms.len()).filter(|&i| moved[i] && reach::is_test_fn(ix, i)).collect();
    if tests.is_empty() {
        return String::new();
    }
    format!("Tests moved: {}\n", tests.iter().map(|&i| format!("{} -> {}", ix.syms[i].qname(), target)).collect::<Vec<_>>().join(", "))
}

/// `text` without the imports whose binding is in `keys`.
fn drop_import_keys(text: &str, module: &str, keys: &HashSet<(String, Vec<String>, bool)>) -> Result<String, Failure> {
    let mut sites: Vec<language::rust_refs::ImportSite> = language::rust_refs::import_sites_in(text)
        .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
        .into_iter()
        .filter(|site| keys.contains(&(site.local.clone(), site.path.clone(), site.glob)))
        .collect();
    sites.sort_by(|a, b| b.range.start.cmp(&a.range.start));
    let mut out = text.to_string();
    for site in sites {
        if !allows_unused(&out, &site.range) && !is_reexport(&out, &site.range) {
            out = drop_import(&out, &site.range, site.alias.as_ref(), site.outside);
        }
    }
    Ok(out)
}

/// `text` without the local imports that only the moved code uses.
fn drop_unused_local_imports(
    cx: &Rewrite,
    module: &str,
    local: &[LocalImport],
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let keys: HashSet<(String, Vec<String>, bool)> = local
        .iter()
        .filter(|import| import.module == module && !import.used_by_staying)
        .map(|import| (import.site.local.clone(), import.site.path.clone(), import.site.glob))
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    if let Some(header) = newheaders.get(module).cloned() {
        let kept = drop_import_keys(&header, module, &keys)?;
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
        let kept = drop_import_keys(&newtext[i], module, &keys)?;
        newtext[i] = kept;
    }
    Ok(())
}

/// The local imports (traits for method calls, macros) the moved code needs,
/// and every such import with whether code staying in its module needs it.
fn local_import_uses(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    refs: &language::rust_refs::References<'_>,
) -> Result<(Vec<Carry>, Vec<LocalImport>), Failure> {
    let mut carried = vec![];
    let mut imports = vec![];
    let modules: BTreeSet<String> = (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
    for module in modules {
        let stays = |i: usize| !moved[i] || newmod[i] == module;
        let moved_methods = called_names_where(ix, language::rust_refs::CallKind::Method, |i| moved[i] && ix.syms[i].module == module);
        let staying_methods = called_names_where(ix, language::rust_refs::CallKind::Method, |i| ix.syms[i].module == module && stays(i));
        let moved_macros = called_names_where(ix, language::rust_refs::CallKind::Macro, |i| moved[i] && ix.syms[i].module == module);
        let staying_macros = called_names_where(ix, language::rust_refs::CallKind::Macro, |i| ix.syms[i].module == module && stays(i));
        let external: HashSet<(String, Vec<String>, bool)> = language::rust_refs::references_external_imports(refs, &module)?
            .into_iter()
            .map(|site| (site.local, site.path, site.glob))
            .collect();
        for (trait_i, site) in language::rust_refs::imported_traits(refs, &module)? {
            let moved_needs = language::rust_refs::trait_has_method(refs, trait_i, &moved_methods);
            let staying_needs = language::rust_refs::trait_has_method(refs, trait_i, &staying_methods);
            if moved_needs {
                carried.push(Carry { from: module.clone(), site: site.clone(), rename: false });
            }
            if moved_needs || staying_needs {
                imports.push(LocalImport { module: module.clone(), site, used_by_staying: staying_needs });
            }
        }
        for site in language::rust_refs::import_sites(&ix.syms, &ix.modules, &module, true)? {
            if site.glob || external.contains(&(site.local.clone(), site.path.clone(), site.glob)) {
                continue;
            }
            let moved_needs = moved_macros.contains(&site.local);
            let staying_needs = staying_macros.contains(&site.local);
            if moved_needs {
                carried.push(Carry { from: module.clone(), site: site.clone(), rename: true });
            }
            if moved_needs || staying_needs {
                imports.push(LocalImport { module: module.clone(), site, used_by_staying: staying_needs });
            }
        }
    }
    Ok((carried, imports))
}

/// Whether code staying in `module` may name something the module's external
/// glob supplies.
fn staying_needs_glob(cx: &Rewrite, module: &str) -> Result<bool, Failure> {
    let named: HashSet<String> = language::rust_refs::import_sites(&cx.ix.syms, &cx.ix.modules, module, true)?
        .into_iter()
        .filter(|site| !site.glob)
        .map(|site| site.local)
        .collect();
    Ok(cx
        .in_sources
        .get(module)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .any(|&i| (!cx.moved[i] || cx.newmod[i] == module) && names_need_glob(cx.ix, module, &cx.mentioned[i], &named)))
}

/// Whether `names` has a name that `module`'s own items and named imports do
/// not explain, so a glob in the module may be its source.
fn names_need_glob(ix: &Index, module: &str, names: &HashSet<String>, named: &HashSet<String>) -> bool {
    names.iter().any(|name| !named.contains(name) && !is_known_name(ix, module, name))
}

fn is_known_name(ix: &Index, module: &str, name: &str) -> bool {
    is_prelude_name(name)
        || ix
            .syms
            .iter()
            .any(|s| s.module == module && s.parent.is_none() && s.kind_text != "use" && (s.name == name || leaf(&s.name) == name))
}

/// Rust syntax and prelude names, which a glob never has to supply.
fn is_prelude_name(name: &str) -> bool {
    is_rust_keyword(name) || is_primitive_type(name) || is_std_prelude_item(name) || is_std_crate_root(name) || is_builtin_macro(name)
}

/// Adds the carried imports to `target`, aliasing one whose local name the
/// target already uses, and renames its mentions in the moved code.
fn carry_imports(
    ix: &Index,
    moved: &[bool],
    target: &str,
    carried: &[Carry],
    refs: &language::rust_refs::References<'_>,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let mut header = newheaders.get(target).cloned().unwrap_or_default();
    let mut names = target_names(ix, &header, target)?;
    let mut handled = import_keys(ix, &header, target)?;
    let mut aliases: HashMap<(String, Vec<String>, bool), String> = HashMap::new();
    for carry in carried {
        let key = (carry.site.local.clone(), carry.site.path.clone(), carry.site.glob);
        if handled.insert(key.clone()) {
            let path = carry.site.path.join("::");
            let line = if carry.site.glob {
                format!("use {path}::*;")
            } else {
                let local = if names.contains(&carry.site.local) {
                    let fresh = fresh_name(&carry.site.local, &names);
                    aliases.insert(key.clone(), fresh.clone());
                    names.insert(fresh.clone());
                    fresh
                } else {
                    carry.site.local.clone()
                };
                names.insert(local.clone());
                if local == carry.site.local { format!("use {path};") } else { format!("use {path} as {local};") }
            };
            header = add_import_line(&header, &line);
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
    Ok(())
}

fn target_names(ix: &Index, header: &str, target: &str) -> Result<HashSet<String>, Failure> {
    let mut names: HashSet<String> = ix
        .syms
        .iter()
        .filter(|s| s.module == target && s.parent.is_none() && !matches!(s.kind_text.as_str(), "impl" | "use"))
        .map(|s| s.name.clone())
        .collect();
    for site in language::rust_refs::import_sites_in(header).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))? {
        names.insert(site.local);
    }
    names.extend(header_mod_names(header).map_err(|e| Failure(format!("modules in {target} don't parse: {e}"), 1))?);
    for symbol in ix.syms.iter().filter(|s| s.module == target && s.parent.is_none() && s.kind_text == "use") {
        for site in
            language::rust_refs::import_sites_in(&symbol.text).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))?
        {
            names.insert(site.local);
        }
    }
    Ok(names)
}

fn import_keys(ix: &Index, header: &str, target: &str) -> Result<HashSet<(String, Vec<String>, bool)>, Failure> {
    let mut keys = HashSet::new();
    for site in language::rust_refs::import_sites_in(header).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))? {
        keys.insert((site.local, site.path, site.glob));
    }
    for symbol in ix.syms.iter().filter(|s| s.module == target && s.parent.is_none() && s.kind_text == "use") {
        for site in
            language::rust_refs::import_sites_in(&symbol.text).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))?
        {
            keys.insert((site.local, site.path, site.glob));
        }
    }
    Ok(keys)
}

fn add_import_line(header: &str, line: &str) -> String {
    let mut out = header.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
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

/// A `use` entry the moved code needs in its target module.
struct Carry {
    from: String,
    site: language::rust_refs::ImportSite,
    rename: bool,
}

/// A local `use` entry the move may have to drop from its source module.
struct LocalImport {
    module: String,
    site: language::rust_refs::ImportSite,
    used_by_staying: bool,
}

/// Whether `range` is the name in a `mod name` declaration.
fn is_mod_declaration(text: &str, range: &Range<usize>) -> bool {
    let Some(before) = text[..range.start].trim_end().strip_suffix("mod") else { return false };
    before.chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// The names a header's `mod name;` lines declare.
fn header_mod_names(header: &str) -> Result<Vec<String>, syn::Error> {
    let file = syn::parse_file(header).or_else(|_| syn::parse_file(&format!("{header}\n}}")))?;
    Ok(file
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Mod(m) if m.content.is_none() => Some(m.ident.to_string()),
            _ => None,
        })
        .collect())
}

fn post_move_check(s: &Store, before: &HashMap<String, String>) -> Result<(), Failure> {
    let ix = s.index()?;
    let diags = s.check_all(&ix)?;
    let errors: Vec<&crate::Diag> = diags
        .iter()
        .filter(|d| d.error)
        .filter(|d| {
            let key = d.symbol.clone().or_else(|| d.module.clone().map(|m| format!("{m} imports")));
            key.as_ref().is_none_or(|k| !before.contains_key(k))
        })
        .collect();
    if !errors.is_empty() {
        return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", crate::group_errors(&errors)));
    }
    Ok(())
}

fn move_reply(m: &RustMove<'_>, moved: &[bool], retired: &[String], budget: usize) -> String {
    let ix = m.ix;
    let mut out = format!("Moved {}", m.going.iter().map(|&i| ix.syms[i].qname()).collect::<Vec<_>>().join(", "));
    if !m.helpers.is_empty() {
        let names: Vec<&str> = m.helpers.iter().map(|&i| ix.syms[i].name.as_str()).collect();
        out += &format!(" (+{} helper{}: {})", m.helpers.len(), if m.helpers.len() == 1 { "" } else { "s" }, names.join(", "));
    }
    out += &format!(" to {}\n", m.target);
    let impls: Vec<usize> = (0..ix.syms.len()).filter(|&i| moved[i] && ix.syms[i].kind_text == "impl").collect();
    if !impls.is_empty() {
        out += &format!("Impls moved: {}\n", impls.iter().map(|&i| ix.syms[i].qname()).collect::<Vec<_>>().join(", "));
    }
    out += &moved_tests_reply(ix, moved, &m.target);
    if let Some(c) = &m.create {
        out += &format!("Created module {} ({})\n", c.module, c.file);
    }
    if !m.already.is_empty() {
        out += &format!("Already in {}: {}\n", m.target, m.already.join(", "));
    }
    if !m.promote.is_empty() {
        out += &format!("Now pub(crate): {}\n", m.promote.iter().map(|&i| ix.syms[i].qname()).collect::<Vec<_>>().join(", "));
    }
    let edited = (0..ix.syms.len()).filter(|&i| m.newtext[i] != ix.syms[i].text).count();
    if edited > 0 {
        out += &format!("Rewrote references in {edited} symbol{}\n", if edited == 1 { "" } else { "s" });
    }
    if !retired.is_empty() {
        out += &format!("Retired module {}\n", retired.join(", "));
    }
    lockdown::trim_output(&out, budget)
}

fn rewrite_move<'a>(
    ix: &'a Index,
    target: String,
    create: Option<crate::rust_submit::RustNew>,
    moved: Vec<bool>,
    helpers: Vec<usize>,
    already: Vec<String>,
    going: Vec<usize>,
) -> Result<RustMove<'a>, Failure> {
    let n = ix.syms.len();
    let references = language::rust_refs::references(&ix.syms, &ix.modules);
    let (mut carried, external_by_module) = external_imports(&ix, &moved, &references)?;
    check_clashes(&ix, &moved, &target)?;
    let placements = impls::placements(&ix, &target, &moved, &going)?;

    let newmod: Vec<String> = (0..n).map(|i| if moved[i] { target.clone() } else { ix.syms[i].module.clone() }).collect();
    let (local_carried, local_sites) = local_import_uses(&ix, &moved, &newmod, &references)?;
    carried.extend(local_carried);
    let newtext: Vec<String> = ix.syms.iter().map(|x| x.text.clone()).collect();
    let mut newheaders: HashMap<String, String> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
    if let Some(c) = &create {
        newheaders.insert(c.module.clone(), c.header.clone());
    }

    let (mut newtext, mut newheaders, mut promote) = rewrite(
        &ix,
        &moved,
        &newmod,
        &target,
        newtext,
        newheaders,
        &references.mentioned,
        &external_by_module,
        &carried,
        &local_sites,
        &references,
    )?;
    visibility::promote_access(&ix, &moved, &target, &mut newtext, &mut promote);
    for &i in &promote {
        if !is_public(&newtext[i]) {
            newtext[i] = make_visible(&newtext[i]);
        }
    }
    qualify_trait_impls(&ix, &moved, &newmod, &mut newtext);
    qualify_impl_selfs(&ix, &moved, &target, &mut newtext);
    if let Some((parent, own)) = create.as_ref().and_then(|c| c.line.as_ref()) {
        let header = add_mod(&newheaders[parent], own);
        newheaders.insert(parent.clone(), header);
    }

    Ok(RustMove { ix, refs: references, target, create, moved, helpers, already, going, newtext, newheaders, promote, placements })
}

fn called_names_where(ix: &Index, kind: language::rust_refs::CallKind, wanted: impl Fn(usize) -> bool) -> HashSet<String> {
    (0..ix.syms.len()).filter(|&i| wanted(i)).flat_map(|i| language::rust_refs::called_names(&ix.syms[i].text, kind)).collect()
}

fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "self"
            | "super"
            | "Self"
    )
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

fn is_std_prelude_item(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Vec"
            | "Box"
            | "Option"
            | "Some"
            | "None"
            | "Result"
            | "Ok"
            | "Err"
            | "Clone"
            | "Copy"
            | "Default"
            | "Drop"
            | "Eq"
            | "PartialEq"
            | "Ord"
            | "PartialOrd"
            | "Iterator"
            | "IntoIterator"
            | "From"
            | "Into"
            | "TryFrom"
            | "TryInto"
            | "AsRef"
            | "AsMut"
            | "ToOwned"
            | "ToString"
            | "Send"
            | "Sync"
            | "Sized"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "Debug"
            | "Hash"
    )
}

fn is_std_crate_root(name: &str) -> bool {
    matches!(name, "std" | "core" | "alloc")
}

fn is_builtin_macro(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "assert_eq"
            | "assert_ne"
            | "cfg"
            | "column"
            | "compile_error"
            | "concat"
            | "dbg"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "env"
            | "eprint"
            | "eprintln"
            | "file"
            | "format"
            | "include"
            | "include_bytes"
            | "include_str"
            | "line"
            | "matches"
            | "module_path"
            | "option_env"
            | "panic"
            | "print"
            | "println"
            | "stringify"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "vec"
            | "write"
            | "writeln"
    )
}

/// Puts a moved top-level `use` back in its source header when staying code
/// still needs it, privately so the item is re-exported only from its new module.
fn restore_moved_use_imports(cx: &Rewrite<'_>, newheaders: &mut HashMap<String, String>) -> Result<(), Failure> {
    for (_, symbol) in cx.ix.syms.iter().enumerate().filter(|(i, s)| cx.moved[*i] && s.parent.is_none() && s.kind_text == "use") {
        let mut header = newheaders.get(&symbol.module).cloned().unwrap_or_default();
        let mut existing: HashSet<(String, Vec<String>, bool)> = language::rust_refs::import_sites_in(&header)
            .map_err(|e| Failure(format!("imports in {} don't parse: {e}", symbol.module), 1))?
            .into_iter()
            .map(|site| (site.local, site.path, site.glob))
            .collect();
        let sites = language::rust_refs::import_sites_in(&symbol.text)
            .map_err(|e| Failure(format!("imports in {} don't parse: {e}", symbol.module), 1))?;
        let stays = |j: usize| !cx.moved[j] || cx.newmod[j] == symbol.module;
        let mentioned = |local: &str| {
            cx.in_sources.get(&symbol.module).is_some_and(|users| users.iter().any(|&j| stays(j) && cx.mentioned[j].contains(local)))
        };
        let needed_glob = sites.iter().any(|site| site.glob) && staying_needs_glob(cx, &symbol.module)?;
        for site in sites {
            let needed = if site.glob { needed_glob } else { mentioned(&site.local) };
            if !needed {
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
            let line = if attributes.is_empty() { line } else { format!("{attributes} {line}") };
            header = add_import_line(&header, &line);
        }
        newheaders.insert(symbol.module.clone(), header);
    }
    Ok(())
}

/// The attributes written before the `use` whose tree holds `range`.
fn use_attributes(text: &str, range: &Range<usize>) -> String {
    let Ok(ranges) = language::rust::import_ranges(text) else {
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

fn module_named_like(module: &str, prefix: &str) -> bool {
    module == prefix || module.ends_with(&format!("::{prefix}")) || prefix.ends_with(&format!("::{module}"))
}

fn impl_owner_symbol(ix: &Index, i: usize) -> Result<usize, Failure> {
    let owners = crate::language::rust_refs::impl_owner_batch(&ix.syms, &ix.modules, &[i]);
    match owners.into_iter().next() {
        Some(crate::language::rust_refs::ImplOwner::One(owner)) => Ok(owner),
        Some(crate::language::rust_refs::ImplOwner::Ambiguous(owners)) => {
            let list: Vec<String> = owners.iter().map(|&t| ix.syms[t].qname()).collect();
            fail(format!("E6013 {} is an impl whose type is ambiguous: {}; qualify its self type", ix.syms[i].qname(), list.join(", ")))
        }
        _ => fail(format!(
            "E6013 {} has no type in this crate its self type names: {}; make its self type a path to a type in this crate",
            ix.syms[i].qname(),
            ix.syms[i].name
        )),
    }
}

fn qualify_impl_selfs(ix: &Index, moved: &[bool], target: &str, newtext: &mut [String]) {
    for i in 0..ix.syms.len() {
        if !moved[i] || ix.syms[i].kind_text != "impl" {
            continue;
        }
        let Ok(owner) = impl_owner_symbol(ix, i) else { continue };
        if moved[owner] || ix.syms[owner].module == target {
            continue;
        }
        if let Some(range) = impl_self_range(&newtext[i]) {
            let current = newtext[i][range.clone()].to_string();
            let generics = current.find('<').map_or(String::new(), |at| current[at..].to_string());
            let ty = format!("{}{generics}", full_path(&ix.syms[owner].module, &ix.syms[owner].name));
            if current != ty {
                newtext[i].replace_range(range, &ty);
            }
        }
    }
}

fn impl_self_range(text: &str) -> Option<Range<usize>> {
    let item = match syn::parse_str::<syn::ItemImpl>(text) {
        Ok(item) => item,
        Err(_) if text.trim_end().ends_with('{') => syn::parse_str::<syn::ItemImpl>(&format!("{text}\n}}")).ok()?,
        Err(_) => return None,
    };
    Some(syn::spanned::Spanned::span(&item.self_ty).byte_range())
}

fn needs_super(ix: &Index, moved: &[bool], roots: &[usize], target: &str) -> bool {
    for &root in roots {
        for at in rust_descendants(ix, root) {
            if matches!(ix.syms[at].kind_text.as_str(), "use" | "mod" | "impl" | "trait") {
                continue;
            }
            for &used in &ix.uses[at] {
                if is_test(ix, used) || matches!(ix.syms[used].kind_text.as_str(), "use" | "mod") {
                    continue;
                }
                let method = ix.syms[used]
                    .parent
                    .as_deref()
                    .is_some_and(|parent| ix.syms.iter().any(|sym| sym.id == parent && sym.kind_text == "impl"));
                if !method && (moved[used] || ix.syms[used].module == target) {
                    return true;
                }
            }
        }
    }
    false
}

fn without_super_glob(text: &str) -> String {
    text.split_inclusive('\n').filter(|line| line.trim() != "use super::*;").collect()
}

fn declaration_visibility(text: &str) -> Visibility {
    let Some((_, code)) = declaration(text) else { return Visibility::Private };
    if code.starts_with("pub(crate)") || code.starts_with("pub ") || code.starts_with("pub\n") {
        Visibility::Public
    } else if code.starts_with("pub(") {
        Visibility::Restricted
    } else {
        Visibility::Private
    }
}

fn drop_super_globs(
    s: &Store,
    ix: &Index,
    moved: &[bool],
    target: &str,
    whole: &HashSet<usize>,
    newtext: &[String],
) -> Result<(), Failure> {
    for &container in whole {
        if ix.syms[container].module == target || needs_super(ix, moved, &[container], target) {
            continue;
        }
        let text = without_super_glob(&newtext[container]);
        if text != ix.syms[container].text {
            s.db.execute(
                "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                params![ix.syms[container].id, text, revision(&text)],
            )?;
            s.history(&ix.syms[container].id, "edit", &ix.syms[container].name)?;
        }
        let drops: Vec<String> = ix
            .syms
            .iter()
            .filter(|sym| {
                sym.parent.as_deref() == Some(ix.syms[container].id.as_str())
                    && sym.kind_text == "use"
                    && sym.text.trim() == "use super::*;"
            })
            .map(|sym| sym.id.clone())
            .collect();
        for id in drops {
            s.retire(&id, "")?;
        }
    }
    Ok(())
}

struct TestModule {
    module: String,
    file: String,
    dir: String,
    header: String,
    created: bool,
}

fn test_module<'a>(ix: &'a Index, target: &str) -> Option<&'a crate::ModuleRow> {
    let path = format!("{target}::tests");
    let row = ix.modules.iter().find(|m| m.path == path && m.file.is_some())?;
    let parent = ix.modules.iter().find(|m| m.path == target)?;
    (crate::rust_submit::declares_path_test_mod(&parent.header, "tests") && crate::rust_submit::declares_module(parent, "tests", row))
        .then_some(row)
}

fn test_file(target: &str, target_file: &str) -> String {
    let (dir, name) = target_file.rsplit_once('/').map_or(("", target_file), |(dir, name)| (dir, name));
    let stem = if target == "crate" { name.strip_suffix(".rs").unwrap_or(name) } else { leaf(target) };
    if dir.is_empty() { format!("{stem}_tests.rs") } else { format!("{dir}/{stem}_tests.rs") }
}

fn test_file_header(needs_super: bool) -> String {
    if needs_super { "use super::*;\n".to_string() } else { String::new() }
}

fn with_super(header: &str) -> String {
    if header.lines().any(|line| line.trim() == "use super::*;") {
        return header.to_string();
    }
    if header.trim().is_empty() { "use super::*;\n".to_string() } else { format!("{}\nuse super::*;\n", header.trim_end()) }
}

fn retire_test_containers(s: &Store, ix: &Index, whole: &HashSet<usize>) -> Result<(), Failure> {
    let ids: Vec<String> = whole.iter().map(|&c| ix.syms[c].id.clone()).collect();
    let now = s.index()?;
    for id in ids {
        let children: Vec<String> =
            now.syms.iter().filter(|sym| sym.parent.as_deref() == Some(id.as_str())).map(|sym| sym.id.clone()).collect();
        for child in children {
            s.retire(&child, "")?;
        }
        s.retire(&id, "")?;
    }
    Ok(())
}

fn plan_test_module(
    s: &Store,
    m: &mut RustMove<'_>,
    existing: Option<&crate::ModuleRow>,
    groups: &[TestGroup],
    moved: &[bool],
) -> Result<Option<TestModule>, Failure> {
    if groups.is_empty() {
        return Ok(None);
    }
    let ix = m.ix;
    let target_file = m
        .create
        .as_ref()
        .map(|c| c.file.clone())
        .or_else(|| ix.modules.iter().find(|row| row.path == m.target).and_then(|row| row.file.clone()))
        .ok_or_else(|| Failure(format!("E1001 {} has no file for its tests", m.target), 1))?;
    let needs = groups.iter().any(|g| needs_super(ix, moved, &g.children, &m.target));
    match existing {
        Some(row) => {
            if needs {
                let header = m.newheaders.get(&row.path).cloned().unwrap_or_else(|| row.header.clone());
                m.newheaders.insert(row.path.clone(), with_super(&header));
            }
            Ok(Some(TestModule {
                module: row.path.clone(),
                file: row.file.clone().unwrap(),
                dir: row.dir.clone(),
                header: row.header.clone(),
                created: false,
            }))
        }
        None => {
            let file = test_file(&m.target, &target_file);
            if s.root.join(&file).exists() {
                return fail(format!("E1001 {file} already exists outside the store; import it before extracting tests"));
            }
            let path = file.rsplit_once('/').map_or(file.as_str(), |(_, name)| name).to_string();
            let module = format!("{}::tests", m.target);
            let header = test_file_header(needs);
            let target_header = m.newheaders.get(&m.target).cloned().unwrap_or_default();
            m.newheaders.insert(m.target.clone(), crate::rust_submit::add_test_mod(&target_header, "tests", &path));
            m.newheaders.insert(module.clone(), header.clone());
            let dir = file.split_once('/').map_or("src", |(dir, _)| dir).to_string();
            Ok(Some(TestModule { module, file, dir, header, created: true }))
        }
    }
}

fn carry_test_imports(
    ix: &Index,
    moved: &[bool],
    ids: &HashMap<&str, usize>,
    refs: &language::rust_refs::References<'_>,
    groups: &[TestGroup],
    test: &TestModule,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let tests: HashSet<usize> = groups.iter().flat_map(|g| g.children.iter().copied()).collect();
    let mut carried: Vec<Carry> = vec![];
    let mut seen = HashSet::new();
    for &i in &tests {
        let from = ix.syms[i].module.clone();
        let mut sites = vec![];
        if let Some(container) = test_container(ix, ids, i) {
            sites.extend(test_container_imports(ix, container)?);
        } else if let Some(row) = ix.modules.iter().find(|row| row.path == from) {
            sites.extend(
                language::rust_refs::import_sites_in(&row.header).map_err(|e| Failure(format!("imports in {from} don't parse: {e}"), 1))?,
            );
        }
        for site in sites {
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
    }
    if carried.is_empty() {
        return Ok(());
    }
    carry_imports(ix, moved, &test.module, &carried, refs, newtext, newheaders)
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

/// Inserts `line` after the import item; keeps an inline module's braces and indentation.
fn add_use_after(text: &str, item: &Range<usize>, line: &str) -> String {
    let start = text[..item.start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[start..item.start].chars().take_while(|c| c.is_whitespace()).collect();
    let mut out = String::with_capacity(text.len() + line.len() + indent.len() + 2);
    out.push_str(&text[..item.end]);
    out.push('\n');
    out.push_str(&indent);
    out.push_str(line);
    out.push_str(&text[item.end..]);
    out
}
