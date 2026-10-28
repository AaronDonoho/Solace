use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;

use rusqlite::params;

use self::glob_reach::{
    add_descendant_import, add_glob_descendant_imports, add_module_import, descendants_need_glob, named_imports, reach_map,
    reaching_modules,
};
use super::Args;
use crate::language::rust_refs::{Site, Via, use_sites};
use crate::rust_submit::{rust_descendants, rust_namespace};
use crate::Sym;
use crate::{ Index,  Store,  is_test, language, lockdown, reach, revision};
use crate::failure::fail;
use crate::failure::Out;
use crate::failure::Failure;
#[path = "rust_tests.rs"]
mod tests;
mod visibility;
mod impls;
pub(crate) mod merge;
mod glob_reach;

pub(super) fn extract(s: &Store, p: &Args, budget: usize) -> Out {
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

fn parent_kind<'a>(ix: &'a Index, sym: &Sym) -> Option<&'a str> {
    let parent = sym.parent.as_deref()?;
    ix.syms.iter().find(|x| x.id == parent).map(|x| x.kind_text.as_str())
}

fn helper_kind(kind: &str) -> bool {
    matches!(kind, "fn" | "const" | "static")
}

fn check_clashes(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], target: &str) -> Result<(), Failure> {
    for i in (0..ix.syms.len()).filter(|&i| {
        moved[i]
            && ix.syms[i].parent.is_none()
            && ix.syms[i].module != target
            && ix.syms[i].kind_text != "impl"
            && !(belongs_in_test_file(ix, &ids, i) && ix.syms[i].kind_text != "mod")
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
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    mut newtext: Vec<String>,
    mut newheaders: HashMap<String, String>,
    all_mentioned: &[HashSet<String>],
    external: &HashMap<String, Vec<language::rust_refs::ImportSite>>,
    carried: &[Carry],
    local_sites: &[LocalImport],
    reaching: &HashMap<String, HashSet<String>>,
    references: &language::rust_refs::References<'_>,
) -> Result<(Vec<String>, HashMap<String, String>, BTreeSet<usize>), Failure> {
    let targets = rewrite_targets(ix, moved);
    let sites: HashMap<usize, Vec<Site>> = targets.iter().map(|&t| (t, language::rust_refs::rewrite_sites_refs(references, t))).collect();
    let use_all = all_use_sites(ix)?;
    let use_keys: HashSet<(Option<usize>, usize, usize)> = use_all.iter().map(|(_, site)| use_key(site)).collect();
    let groups = use_groups(&use_all, &targets);
    let sources = moved_sources(ix, moved);
    let mentioned = all_mentioned.to_vec();
    let mut in_sources: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, x) in ix.syms.iter().enumerate().filter(|(_, x)| sources.contains(&x.module) && x.kind_text != "use") {
        in_sources.entry(x.module.clone()).or_default().push(i);
    }
    let cx = Rewrite { ix, moved, newmod, targets, sites, use_keys, sources, mentioned, in_sources, external, reaching };
    let (mut promote, import_ok) = rewrite_imports(&cx, groups, &mut newtext, &mut newheaders)?;
    restore_moved_use_imports(&cx, &mut newheaders)?;
    for module in &cx.sources {
        drop_unused_external_imports(&cx, module, &mut newtext, &mut newheaders)?;
        drop_unused_local_imports(&cx, module, local_sites, &mut newtext, &mut newheaders)?;
    }
    carry_imports(ix, moved, target, carried, references, &mut newtext, &mut newheaders)?;
    check_format_capture_clashes(&cx, &newheaders)?;
    import_format_captures(&cx, &mut newheaders);
    promote.extend(rewrite_mentions(&cx, &import_ok, &mut newtext, &mut newheaders)?);
    promote.retain(|&i| can_promote(ix, ids, i) && ix.syms[i].kind_text != "thread_local!");
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
    } else if module.starts_with("crate::") {
        format!("{module}::{name}")
    } else {
        format!("crate::{module}::{name}")
    }
}

pub(crate) fn leaf(name: &str) -> &str {
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
    let ids = id_indexes(ix);
    let mut moved = vec![false; n];
    for &i in roots {
        mark(ix, &mut moved, i);
    }
    impls::check_direct(ix, &ids, roots, &mut moved)?;
    add_impls(ix, &mut moved)?;
    let mut helpers: Vec<usize> = vec![];
    bring_helpers(ix, &mut moved, &mut helpers);
    move_tests(ix, &ids, &mut moved, &mut helpers);
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
) -> Result<(BTreeSet<usize>, HashSet<(usize, String, String)>), Failure> {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let (sites, use_keys) = (&cx.sites, &cx.use_keys);
    let mut import_ok: HashSet<(usize, String, String)> = HashSet::new();
    let mut promote: BTreeSet<usize> = BTreeSet::new();
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
                    let path = full_path(&newmod[site.target], &ix.syms[site.target].name);
                    if !visibility::visible_from(&newmod[site.target], &use_new) {
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
                if site.outside == 0 {
                    line_removals.extend(use_line_removal(&original, &site.range));
                } else {
                    member_removals.push((site.range.clone(), site.alias.clone()));
                }
            } else {
                let module = match key.0 {
                    Some(i) => &ix.syms[i].module,
                    None => &key.1,
                };
                let unused = alias.as_ref().is_some_and(|a| unused_after_move(cx, module, a));
                if unused && !allows_unused(&original, &site.range) {
                    if let Some(local) = &alias {
                        let target = &ix.syms[site.target];
                        let path: Vec<String> = full_path(&target.module, &target.name).split("::").map(str::to_string).collect();
                        add_descendant_import(cx, module, local, &path, local, newheaders)?;
                    }
                    if site.outside == 0 {
                        line_removals.extend(use_line_removal(&original, &site.range));
                    } else {
                        member_removals.push((site.range.clone(), site.alias.clone()));
                    }
                }
            }
        }
        let text = apply_import_edits(&original, replacements, line_removals, member_removals, additions, &key.1)?;
        set_container_text(&key, &mut *newheaders, &mut *newtext, text);
    }
    Ok((promote, import_ok))
}

fn rewrite_mentions(
    cx: &Rewrite,
    import_ok: &HashSet<(usize, String, String)>,
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<BTreeSet<usize>, Failure> {
    let (ix, moved, newmod) = (cx.ix, cx.moved, cx.newmod);
    let (targets, sites, use_keys) = (&cx.targets, &cx.sites, &cx.use_keys);
    let mut edits: HashMap<usize, Vec<(Range<usize>, String)>> = HashMap::new();
    let mut promote: BTreeSet<usize> = BTreeSet::new();
    let mut groups: BTreeMap<(String, usize), MovedMentions> = BTreeMap::new();
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
                if !visibility::visible_from(&newmod[t], &mention_new) {
                    promote.insert(t);
                }
                continue;
            }
            let parent_impl = ix.syms[t].parent.as_deref().and_then(|p| ix.syms.iter().position(|s| s.id == p && s.kind_text == "impl"));
            if let Some(at) = parent_impl {
                if matches!(&site.via, Via::Path) && site.segs.len() > 1 {
                    if let Ok(owner) = impl_owner_symbol(ix, at) {
                        let owner_new = newmod[owner].clone();
                        if mention_new != owner_new {
                            if !visibility::visible_from(&owner_new, &mention_new) {
                                promote.insert(owner);
                            }
                            if let Some(sep) = ix.syms[i].text[site.range.clone()].rfind("::") {
                                edits.entry(i).or_default().push((
                                    site.range.start..site.range.start + sep,
                                    reference_path(ix, moved, owner, &owner_new, i, &site.segs[..site.segs.len() - 1]),
                                ));
                            }
                        }
                    }
                }
                continue;
            }
            let target_new = newmod[t].clone();
            let stays_in_parent = mention_new == test_module_path(&target_new);
            let written = &ix.syms[i].text[site.range.clone()];
            let mut replacement: Option<String> = None;
            let mut needs_visibility = false;
            let mut importable = false;
            match &site.via {
                Via::Import(alias) => {
                    if written.contains("::") {
                        if moved[t] {
                            let (path, promote) = moved_reference(&target_new, &ix.syms[t].name, &mention_new);
                            replacement = Some(path);
                            needs_visibility = promote;
                            importable = !in_moved_inline_module(cx, i);
                        }
                    } else if import_ok.contains(&(t, alias.clone(), mention_new.clone()))
                        || target_new == mention_new
                        || sites[&t].iter().any(|m| {
                            m.symbol == site.symbol && is_import_path(m, &ix.syms[i].text) && matches!(&m.via, Via::Import(a) if a == alias)
                        })
                    {
                    } else {
                        let (path, promote) = moved_reference(&target_new, &ix.syms[t].name, &mention_new);
                        replacement = Some(path);
                        needs_visibility = promote;
                        importable = moved[t] && !in_moved_inline_module(cx, i);
                    }
                }
                Via::Path => {
                    if target_new == mention_new {
                        replacement = Some(leaf(&ix.syms[t].name).to_string());
                    } else if !moved[t] {
                        replacement = Some(reference_path(ix, moved, t, &target_new, i, &site.segs));
                        needs_visibility = !visibility::visible_from(&target_new, &mention_new);
                    } else {
                        let (path, promote) = moved_reference(&target_new, &ix.syms[t].name, &mention_new);
                        replacement = Some(path);
                        needs_visibility = promote;
                        importable = !in_moved_inline_module(cx, i);
                    }
                }
                Via::Own | Via::Glob => {
                    if target_new != mention_new && !stays_in_parent {
                        let (path, promote) = moved_reference(&target_new, &ix.syms[t].name, &mention_new);
                        replacement = Some(path);
                        needs_visibility = promote;
                        importable = moved[t] && !in_moved_inline_module(cx, i);
                    }
                }
            }
            if let Some(text) = replacement {
                if importable {
                    let group = groups.entry((mention_new, t)).or_insert_with(|| MovedMentions { test_only: true, sites: vec![] });
                    group.test_only &= ix.test[i];
                    group.sites.push((i, site.range.clone()));
                } else {
                    edits.entry(i).or_default().push((site.range.clone(), text));
                }
            }
            if needs_visibility {
                promote.insert(t);
            }
        }
    }
    resolve_moved_mentions(cx, groups, newheaders, &mut edits)?;
    for (i, es) in edits {
        newtext[i] = splice(&newtext[i], es);
    }
    Ok(promote)
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
    let groups = test_groups(ix, &ids, &moved, &whole, file_tests);
    let test = plan_test_module(s, &mut m, &ids, existing_tests, &groups, &moved)?;
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
    let test_globs = if let Some(t) = &test {
        carry_test_imports(ix, &moved, &ids, &m.refs, &groups, t, &mut m.newtext, &mut m.newheaders)?
    } else {
        vec![]
    };

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
        drop_super_globs(s, ix, &ids, &moved, &m.target, &whole, &m.newtext)?;

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

        let warnings = crate::rust_check::post_move_check(s, &before)?;
        if let Some(t) = &test {
            drop_unused_carried_globs(s, t, &test_globs, &warnings)?;
        }
        Ok(move_reply(&m, &ids, &moved, &retired, budget) + &crate::rust_submit::dropped_impls_note(&dropped))
    })?;
    Ok(out)
}

/// Whether code moving out of `module` mentioned `name` and nothing staying there does.
fn unused_after_move(cx: &Rewrite, module: &str, name: &str) -> bool {
    let mentions = |i: usize| cx.mentioned[i].contains(name);
    let in_module = cx.in_sources.get(module).map(Vec::as_slice).unwrap_or_default();
    in_module.iter().any(|&i| cx.moved[i] && mentions(i)) && !staying_mentions(cx, module, name)
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
pub(crate) struct Rewrite<'a> {
    ix: &'a Index,
    moved: &'a [bool],
    newmod: &'a [String],
    targets: BTreeSet<usize>,
    sites: HashMap<usize, Vec<Site>>,
    use_keys: HashSet<(Option<usize>, usize, usize)>,
    sources: BTreeSet<String>,
    mentioned: Vec<HashSet<String>>,
    in_sources: HashMap<String, Vec<usize>>,
    external: &'a HashMap<String, Vec<language::rust_refs::ImportSite>>,
    reaching: &'a HashMap<String, HashSet<String>>,
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
    external: &[language::rust_refs::ImportSite],
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
        } else if unused_after_move(cx, module, &site.local) {
            add_descendant_import(cx, module, &site.local, &site.path, &site.local, newheaders)?;
            keys.insert(key);
        }
    }
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
        belongs_in_test_file(ix, ids, j)
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

fn move_tests(ix: &Index, ids: &HashMap<&str, usize>, moved: &mut [bool], helpers: &mut Vec<usize>) {
    let tests: Vec<usize> = reach::test_symbols(ix).into_iter().filter(|&t| !moved[t]).collect();
    if tests.is_empty() {
        return;
    }
    let covered: HashMap<usize, (HashSet<usize>, HashSet<usize>)> = tests.iter().map(|&t| (t, test_covered(ix, ids, t))).collect();
    let candidates: Vec<usize> = tests.iter().copied().filter(|t| covered[t].0.iter().any(|&u| moved[u])).collect();
    if candidates.is_empty() {
        return;
    }
    let locals: HashSet<usize> = candidates.iter().flat_map(|t| covered[t].1.iter().copied()).collect();
    let mut moving_tests: HashSet<usize> = candidates.iter().copied().collect();
    let mut moving_locals: HashSet<usize> = locals.clone();
    loop {
        let mut changed = false;
        for &t in &candidates {
            if !moving_tests.contains(&t) {
                continue;
            }
            let (out, locs) = &covered[&t];
            if out.iter().any(|&u| !moved[u]) || locs.iter().any(|u| !moving_locals.contains(u)) {
                moving_tests.remove(&t);
                changed = true;
            }
        }
        for &l in &locals {
            if !moving_locals.contains(&l) {
                continue;
            }
            if !ix.used_by[l].iter().all(|&u| u == l || moved[u] || moving_tests.contains(&u) || moving_locals.contains(&u)) {
                moving_locals.remove(&l);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut moving: HashSet<usize> = moving_tests.iter().chain(moving_locals.iter()).copied().collect();
    bring_helpers_with(ix, moved, helpers, &mut moving);
    for &t in &moving_tests {
        mark(ix, moved, t);
        for &l in &covered[&t].1 {
            mark(ix, moved, l);
        }
    }
    bring_helpers(ix, moved, helpers);
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

fn test_groups(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], whole: &HashSet<usize>, file_tests: bool) -> Vec<TestGroup> {
    let mut groups: std::collections::BTreeMap<String, TestGroup> = std::collections::BTreeMap::new();
    for i in 0..ix.syms.len() {
        if !moved[i] || matches!(ix.syms[i].kind_text.as_str(), "mod" | "use") || !belongs_in_test_file(ix, ids, i) {
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
            if moved[i] || sym.parent.is_some() || !helper_kind(&sym.kind_text) || is_public(&sym.text) {
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
    let sites: Vec<language::rust_refs::ImportSite> = language::rust_refs::import_sites_in(text)
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
fn drop_unused_local_imports(
    cx: &Rewrite,
    module: &str,
    local: &[LocalImport],
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let mut keys = HashSet::new();
    let mut allowed = HashSet::new();
    for import in local.iter().filter(|import| import.module == module && !import.used_by_staying) {
        let key = (import.site.local.clone(), import.site.path.clone(), import.site.glob);
        if import.site.glob && !import.needed_by.is_empty() {
            allowed.insert(key);
        } else {
            if !import.site.glob {
                for child in &import.needed_by {
                    add_module_import(child, &import.site.path, &import.site.local, newheaders)?;
                }
            }
            keys.insert(key);
        }
    }
    if keys.is_empty() && allowed.is_empty() {
        return Ok(());
    }
    if let Some(header) = newheaders.get(module).cloned() {
        let kept = drop_import_keys(&header, module, &keys)?;
        newheaders.insert(module.to_string(), allow_import_keys(&kept, module, &allowed)?);
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
        newtext[i] = allow_import_keys(&kept, module, &allowed)?;
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
    reach: &HashMap<String, HashSet<String>>,
) -> Result<(Vec<Carry>, Vec<LocalImport>), Failure> {
    let mut carried = vec![];
    let mut imports = vec![];
    let modules = moved_sources(ix, moved);
    for module in modules {
        let stays = |i: usize| !moved[i] || newmod[i] == module;
        let moved_methods = called_names_where(ix, language::rust_refs::CallKind::Method, |i| moved[i] && ix.syms[i].module == module);
        let moved_macros = called_names_where(ix, language::rust_refs::CallKind::Macro, |i| moved[i] && ix.syms[i].module == module);
        let staying_methods: HashMap<String, HashSet<String>> = reaching_modules(reach, &module)
            .map(|m| {
                let names = called_names_where(ix, language::rust_refs::CallKind::Method, |i| ix.syms[i].module == m && stays(i));
                (m.to_string(), names)
            })
            .collect();
        let staying_macros: HashMap<String, HashSet<String>> = reaching_modules(reach, &module)
            .map(|m| {
                let names = called_names_where(ix, language::rust_refs::CallKind::Macro, |i| ix.syms[i].module == m && stays(i));
                (m.to_string(), names)
            })
            .collect();
        let external: HashSet<(String, Vec<String>, bool)> = language::rust_refs::references_external_imports(refs, &module)?
            .into_iter()
            .map(|site| (site.local, site.path, site.glob))
            .collect();
        for (trait_i, site) in language::rust_refs::imported_traits(refs, &module)? {
            let moved_needs = language::rust_refs::trait_has_method(refs, trait_i, &moved_methods);
            let staying_needs = language::rust_refs::trait_has_method(refs, trait_i, staying_methods.get(&module).unwrap());
            let needed_by = import_needed_by(&staying_methods, &module, staying_needs, |names| {
                language::rust_refs::trait_has_method(refs, trait_i, names)
            });
            record_local_import(&module, &site, false, moved_needs, staying_needs, needed_by, &mut carried, &mut imports);
        }
        for site in language::rust_refs::import_sites(&ix.syms, &ix.modules, &module, true)? {
            let is_external = external.contains(&(site.local.clone(), site.path.clone(), site.glob));
            if !site.glob
                && is_external
                && let Some(methods) = language::rust_refs::std_trait_methods(&site.path)
            {
                let uses_method = |names: &HashSet<String>| methods.iter().any(|method| names.contains(*method));
                let moved_needs = uses_method(&moved_methods);
                let staying_needs = staying_methods.get(&module).is_some_and(|names| uses_method(names));
                let needed_by = import_needed_by(&staying_methods, &module, staying_needs, uses_method);
                record_local_import(&module, &site, false, moved_needs, staying_needs, needed_by, &mut carried, &mut imports);
                continue;
            }
            if site.glob || is_external {
                continue;
            }
            let moved_needs = moved_macros.contains(&site.local);
            let staying_needs = staying_macros.get(&module).is_some_and(|names| names.contains(&site.local));
            let needed_by = import_needed_by(&staying_macros, &module, staying_needs, |names| names.contains(&site.local));
            record_local_import(&module, &site, true, moved_needs, staying_needs, needed_by, &mut carried, &mut imports);
        }
    }
    Ok((carried, imports))
}

/// Whether `names` has a name that `module`'s own items and named imports do
/// not explain, so a glob in the module may be its source.
pub(crate) fn names_need_glob(ix: &Index, module: &str, names: &HashSet<String>, named: &HashSet<String>) -> bool {
    names.iter().any(|name| !named.contains(name) && !is_known_name(ix, module, name))
}

pub(crate) fn is_known_name(ix: &Index, module: &str, name: &str) -> bool {
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
) -> Result<Vec<Vec<String>>, Failure> {
    let mut header = newheaders.get(target).cloned().unwrap_or_default();
    let mut names = target_names(ix, &header, target, |_, s| s.module == target)?;
    let mut handled = import_keys(ix, &header, target)?;
    let mut aliases: HashMap<(String, Vec<String>, bool), String> = HashMap::new();
    let mut added_globs = vec![];
    for carry in carried {
        let key = (carry.site.local.clone(), carry.site.path.clone(), carry.site.glob);
        if handled.insert(key.clone()) {
            let rooted = rooted_path(ix, &carry.from, target, &carry.site.path);
            let path = rooted.join("::");
            let line = if carry.site.glob {
                added_globs.push(rooted);
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
    Ok(added_globs)
}

fn target_names(ix: &Index, header: &str, target: &str, belongs: impl Fn(usize, &Sym) -> bool) -> Result<HashSet<String>, Failure> {
    let mut names: HashSet<String> = ix
        .syms
        .iter()
        .enumerate()
        .filter(|(i, s)| belongs(*i, s) && s.parent.is_none() && !matches!(s.kind_text.as_str(), "impl" | "use"))
        .map(|(_, s)| s.name.clone())
        .collect();
    for site in language::rust_refs::import_sites_in(header).map_err(|e| Failure(format!("imports in {target} don't parse: {e}"), 1))? {
        names.insert(site.local);
    }
    names.extend(header_mod_names(header).map_err(|e| Failure(format!("modules in {target} don't parse: {e}"), 1))?);
    for symbol in module_use_symbols(ix, belongs) {
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

pub(crate) fn add_import_line(header: &str, line: &str) -> String {
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
    needed_by: Vec<String>,
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

fn move_reply(m: &RustMove<'_>, ids: &HashMap<&str, usize>, moved: &[bool], retired: &[String], budget: usize) -> String {
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
    out += &staying_tests_reply(ix, ids, moved, &m.target);
    if let Some(c) = &m.create {
        out += &format!("Created module {} ({})\n", c.module, c.file);
    }
    if !m.already.is_empty() {
        out += &format!("Already in {}: {}\n", m.target, m.already.join(", "));
    }
    if !m.promote.is_empty() {
        let names: Vec<String> =
            m.promote.iter().map(|&i| format!("{}.{}", new_module(ix, ids, moved, &m.target, i), ix.syms[i].name)).collect();
        out += &format!("Now pub(crate): {}\n", names.join(", "));
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
    let ids = id_indexes(ix);
    let references = language::rust_refs::references(&ix.syms, &ix.modules);
    let (mut carried, external_by_module) = external_imports(ix, &moved, &references)?;
    check_clashes(ix, &ids, &moved, &target)?;
    let placements = impls::placements(ix, &ids, &target, &moved, &going)?;

    let newmod: Vec<String> = (0..n).map(|i| new_module(ix, &ids, &moved, &target, i)).collect();
    let sources = moved_sources(ix, &moved);
    let reaching = reach_map(ix, &sources)?;
    let (local_carried, local_sites) = local_import_uses(ix, &moved, &newmod, &references, &reaching)?;
    carried.extend(local_carried);
    let newtext: Vec<String> = ix.syms.iter().map(|x| x.text.clone()).collect();
    let mut newheaders: HashMap<String, String> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect();
    if let Some(c) = &create {
        newheaders.insert(c.module.clone(), c.header.clone());
    }

    let (mut newtext, mut newheaders, mut promote) = rewrite(
        ix,
        &ids,
        &moved,
        &newmod,
        &target,
        newtext,
        newheaders,
        &references.mentioned,
        &external_by_module,
        &carried,
        &local_sites,
        &reaching,
        &references,
    )?;
    qualify_thread_locals(ix, &ids, &moved, &newmod, &references, &mut newtext);
    visibility::promote_access(ix, &ids, &moved, &target, &mut newtext, &mut promote);
    for &i in &promote {
        if !is_public(&newtext[i]) {
            newtext[i] = make_visible(&newtext[i]);
        }
    }
    qualify_trait_impls(ix, &moved, &newmod, &mut newtext);
    qualify_impl_selfs(ix, &moved, &target, &mut newtext);
    if let Some(created) = &create
        && let Some((parent, _)) = &created.line
    {
        let header = crate::rust_submit::add_created_mod(&newheaders[parent], created);
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

fn needs_super(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], roots: &[usize], target: &str) -> bool {
    for &root in roots {
        for at in rust_descendants(ix, root) {
            if matches!(ix.syms[at].kind_text.as_str(), "use" | "mod" | "impl" | "trait") {
                continue;
            }
            for &used in &ix.uses[at] {
                if belongs_in_test_file(ix, ids, used) || matches!(ix.syms[used].kind_text.as_str(), "use" | "mod") {
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
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    target: &str,
    whole: &HashSet<usize>,
    newtext: &[String],
) -> Result<(), Failure> {
    for &container in whole {
        if ix.syms[container].module == target || needs_super(ix, ids, moved, &[container], target) {
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
    let path = test_module_path(target);
    let row = ix.modules.iter().find(|m| m.path == path && m.file.is_some())?;
    let parent = ix.modules.iter().find(|m| m.path == target)?;
    (crate::rust_submit::declares_path_test_mod(&parent.header, "tests") && crate::rust_submit::declares_module(parent, "tests", row))
        .then_some(row)
}

fn test_file_header(existing: &str, needs_super: bool) -> String {
    if needs_super { with_super(existing) } else { existing.to_string() }
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
    ids: &HashMap<&str, usize>,
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
    let needs = groups.iter().any(|g| needs_super(ix, ids, moved, &g.children, &m.target));
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
            let file = language::rust::test_file(&m.target, &target_file);
            if s.root.join(&file).exists() {
                return fail(format!("E1001 {file} already exists outside the store; import it before extracting tests"));
            }
            let module = test_module_path(&m.target);
            let existing_header = m.newheaders.get(&module).cloned().unwrap_or_default();
            let header = test_file_header(&existing_header, needs);
            let target_header = m.newheaders.get(&m.target).cloned().unwrap_or_default();
            let path = crate::rust_submit::test_mod_path(&file);
            m.newheaders.insert(m.target.clone(), crate::rust_submit::add_test_mod(&target_header, "tests", path));
            m.newheaders.insert(module.clone(), header.clone());
            let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
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
    if carried.is_empty() {
        return Ok(vec![]);
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

fn staying_mentions(cx: &Rewrite, source: &str, name: &str) -> bool {
    staying_in(cx.ix, cx.moved, cx.newmod, source).any(|i| cx.mentioned[i].contains(name))
}

fn module_needs_glob(cx: &Rewrite, module: &str) -> Result<bool, Failure> {
    let named = named_imports(cx, module)?;
    Ok(staying_in(cx.ix, cx.moved, cx.newmod, module).any(|i| names_need_glob(cx.ix, module, &cx.mentioned[i], &named)))
}

fn allow_import_keys(text: &str, module: &str, keys: &HashSet<(String, Vec<String>, bool)>) -> Result<String, Failure> {
    if keys.is_empty() {
        return Ok(text.to_string());
    }
    let sites: Vec<language::rust_refs::ImportSite> = language::rust_refs::import_sites_in(text)
        .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
        .into_iter()
        .filter(|site| keys.contains(&(site.local.clone(), site.path.clone(), site.glob)))
        .collect();
    let ranges = language::rust::import_ranges(text).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    let mut starts: Vec<usize> = sites
        .iter()
        .filter(|site| !allows_unused(text, &site.range))
        .filter_map(|site| {
            ranges
                .iter()
                .find(|(item, _)| item.start <= site.range.start && site.range.end <= item.end)
                .map(|(item, _)| text[..item.start].rfind('\n').map_or(0, |i| i + 1))
        })
        .collect();
    starts.sort_unstable();
    starts.dedup();
    let mut out = text.to_string();
    for start in starts.into_iter().rev() {
        let indent_end = out[start..].find(|c: char| !c.is_whitespace()).map_or(start, |n| start + n);
        let indent = out[start..indent_end].to_string();
        out.insert_str(start, &format!("{indent}#[allow(unused_imports)]\n"));
    }
    Ok(out)
}

fn moved_sources(ix: &Index, moved: &[bool]) -> BTreeSet<String> {
    (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect()
}

pub(crate) fn staying_in<'a>(ix: &'a Index, moved: &'a [bool], newmod: &'a [String], module: &'a str) -> impl Iterator<Item = usize> + 'a {
    ix.syms
        .iter()
        .enumerate()
        .filter(move |(i, symbol)| {
            symbol.module == module && !matches!(symbol.kind_text.as_str(), "use" | "mod") && (!moved[*i] || newmod[*i] == module)
        })
        .map(|(i, _)| i)
}

/// The range `remove_member` deletes: one member and one adjacent comma.
fn member_removal(text: &str, range: &Range<usize>, alias: Option<&Range<usize>>) -> Range<usize> {
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
fn use_line_removal(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let ranges = language::rust::import_ranges(text).ok()?;
    let (item, _) = ranges.into_iter().find(|(_, tree)| tree.start <= range.start && range.end <= tree.end)?;
    Some(use_item_line_range(text, &item))
}

fn use_item_line_range(text: &str, item: &Range<usize>) -> Range<usize> {
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
    let mut ranges: Vec<Range<usize>> = members.iter().map(|(range, alias)| member_removal(original, range, alias.as_ref())).collect();
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
    let items = language::rust::import_ranges(original).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
    let mut edits = vec![];
    for (item, _) in &items {
        let inside: Vec<Range<usize>> = merged.iter().filter(|range| item.start <= range.start && range.end <= item.end).cloned().collect();
        if inside.is_empty() {
            continue;
        }
        let bindings = language::rust_refs::import_sites_in(&original[item.clone()])
            .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
        let all = !bindings.is_empty()
            && bindings.iter().all(|binding| {
                let range = (item.start + binding.range.start)..(item.start + binding.range.end);
                inside.iter().any(|removal| removal.start <= range.start && range.end <= removal.end)
            });
        if all {
            edits.push((use_item_line_range(original, item), String::new()));
        } else {
            edits.extend(inside.into_iter().map(|range| (range, String::new())));
        }
    }
    Ok(edits)
}

fn rooted_path(ix: &Index, from: &str, target: &str, path: &[String]) -> Vec<String> {
    let Some(first) = path.first() else { return path.to_vec() };
    if from != "crate" || target == "crate" || matches!(first.as_str(), "crate" | "self" | "super") || !crate_root_name(ix, first) {
        return path.to_vec();
    }
    let mut rooted = Vec::with_capacity(path.len() + 1);
    rooted.push("crate".to_string());
    rooted.extend(path.iter().cloned());
    rooted
}

fn crate_root_name(ix: &Index, name: &str) -> bool {
    ix.modules.iter().any(|module| module.path == name)
        || ix.syms.iter().any(|sym| {
            sym.module == "crate" && sym.parent.is_none() && sym.kind_text != "use" && (sym.name == name || leaf(&sym.name) == name)
        })
}

fn import_needed_by(
    staying: &HashMap<String, HashSet<String>>,
    module: &str,
    staying_needs: bool,
    uses: impl Fn(&HashSet<String>) -> bool,
) -> Vec<String> {
    if staying_needs {
        return vec![];
    }
    staying.iter().filter(|(m, _)| m.as_str() != module).filter(|(_, names)| uses(names)).map(|(m, _)| m.clone()).collect()
}

fn record_local_import(
    module: &str,
    site: &language::rust_refs::ImportSite,
    rename: bool,
    moved_needs: bool,
    staying_needs: bool,
    needed_by: Vec<String>,
    carried: &mut Vec<Carry>,
    imports: &mut Vec<LocalImport>,
) {
    if moved_needs {
        carried.push(Carry { from: module.to_string(), site: site.clone(), rename });
    }
    if moved_needs || staying_needs || !needed_by.is_empty() {
        imports.push(LocalImport { module: module.to_string(), site: site.clone(), used_by_staying: staying_needs, needed_by });
    }
}

fn new_module(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], target: &str, i: usize) -> String {
    if !moved[i] {
        return ix.syms[i].module.clone();
    }
    if belongs_in_test_file(ix, ids, i) && ix.syms[i].kind_text != "mod" { test_module_path(target) } else { target.to_string() }
}

/// The path a moved item's mention in another module needs, and whether the
/// target must be made reachable.
fn moved_reference(target: &str, name: &str, mention: &str) -> (String, bool) {
    (full_path(target, name), !visibility::visible_from(target, mention))
}

/// `original` with all its import edits applied: each one is worked out from
/// `original` before any shift, and the edits are applied right to left.
fn apply_import_edits(
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
        let items = language::rust::import_ranges(original).map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
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

/// Whether a moved item belongs in the target's test file (D-206): a test
/// container, a `#[test]` function, code inside either, or a test module.
fn belongs_in_test_file(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> bool {
    if is_test_container(ix, i) || reach::is_test_fn(ix, i) {
        return true;
    }
    let mut parent = ix.syms[i].parent.as_deref();
    while let Some(id) = parent {
        let Some(&k) = ids.get(id) else { break };
        if is_test_container(ix, k) {
            return true;
        }
        parent = ix.syms[k].parent.as_deref();
    }
    ix.test_modules.contains(&ix.syms[i].module)
}

/// The statics declared inside `thread_local!` blocks that have no symbol of
/// their own, with the module that declares them and their full path.
fn thread_local_paths(ix: &Index, ids: &HashMap<&str, usize>, names: &HashSet<(String, String)>) -> Vec<HiddenThreadLocal> {
    let mut paths = vec![];
    for (block, tl) in ix.syms.iter().enumerate().filter(|(_, s)| s.kind_text == "thread_local!") {
        let module = enclosing_module(ix, ids, block);
        for (name, ..) in language::rust_refs::thread_local_names(&tl.text) {
            if names.contains(&(module.clone(), name.clone())) {
                continue;
            }
            paths.push(HiddenThreadLocal { block, path: full_path(&module, &name), module: module.clone(), name });
        }
    }
    paths
}

/// Names the hidden `thread_local!` statics a moved item uses by their full
/// path, since the store has no symbol to carry an import for.
fn qualify_thread_locals(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    newmod: &[String],
    references: &language::rust_refs::References<'_>,
    newtext: &mut [String],
) {
    let names = module_names(ix, ids);
    let paths = thread_local_paths(ix, ids, &names);
    if paths.is_empty() {
        return;
    }
    let mut imports: HashMap<String, Vec<language::rust_refs::ImportSite>> = HashMap::new();
    for i in (0..ix.syms.len()).filter(|&i| moved[i] && ix.syms[i].kind_text != "thread_local!") {
        let from = enclosing_module(ix, ids, i);
        let mut renames = HashMap::new();
        let mut used: Vec<&HiddenThreadLocal> = vec![];
        for tl in &paths {
            if names.contains(&(from.clone(), tl.name.clone())) {
                continue;
            }
            let locals = if from == tl.module {
                vec![tl.name.clone()]
            } else {
                imports_from(ix, references, ids, &from, &tl.module, &tl.name, &mut imports)
            };
            let target = if moved[tl.block] { &newmod[tl.block] } else { &tl.module };
            let path = if moved[tl.block] { full_path(target, &tl.name) } else { tl.path.clone() };
            for local in locals {
                if !references.mentioned[i].contains(&local) {
                    continue;
                }
                renames.insert(local, path.clone());
                if !used.iter().any(|u| u.block == tl.block) {
                    used.push(tl);
                }
            }
        }
        if renames.is_empty() {
            continue;
        }
        newtext[i] = language::rust_refs::rename_names(references, i, &newtext[i], &renames);
        for tl in used {
            let target = if moved[tl.block] { &newmod[tl.block] } else { &tl.module };
            if !visibility::visible_from(target, &newmod[i]) {
                newtext[tl.block] = promote_hidden_static(&newtext[tl.block], &tl.name);
            }
        }
    }
}

/// A static declared inside a `thread_local!` block that has no symbol of its
/// own, and the module that declares it.
struct HiddenThreadLocal {
    block: usize,
    module: String,
    name: String,
    path: String,
}

/// The module a symbol is written in, including the inline modules around it.
fn enclosing_module(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> String {
    let mut parent = ix.syms[i].parent.as_deref();
    while let Some(id) = parent {
        let Some(&k) = ids.get(id) else { break };
        if ix.syms[k].kind_text == "mod" {
            return format!("{}::{}", ix.syms[k].module, ix.syms[k].name);
        }
        parent = ix.syms[k].parent.as_deref();
    }
    ix.syms[i].module.clone()
}

/// Each module's own names, for telling a hidden static from a local one.
fn module_names(ix: &Index, ids: &HashMap<&str, usize>) -> HashSet<(String, String)> {
    (0..ix.syms.len())
        .filter(|&i| {
            let sym = &ix.syms[i];
            !matches!(sym.kind_text.as_str(), "use" | "thread_local!")
                && (sym.parent.is_none()
                    || sym.parent.as_deref().and_then(|id| ids.get(id)).is_some_and(|&k| ix.syms[k].kind_text == "mod"))
        })
        .map(|i| (enclosing_module(ix, ids, i), leaf(&ix.syms[i].name).to_string()))
        .collect()
}

/// The imports written in `module`, including an inline module's own `use`s.
fn module_imports(ix: &Index, ids: &HashMap<&str, usize>, module: &str) -> Vec<language::rust_refs::ImportSite> {
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

/// Whether `from` imports `name` from `module`.
fn imports_from(
    ix: &Index,
    references: &language::rust_refs::References<'_>,
    ids: &HashMap<&str, usize>,
    from: &str,
    module: &str,
    name: &str,
    imports: &mut HashMap<String, Vec<language::rust_refs::ImportSite>>,
) -> Vec<String> {
    let sites = imports.entry(from.to_string()).or_insert_with(|| module_imports(ix, ids, from));
    sites
        .iter()
        .filter_map(|site| {
            if site.glob {
                return (visibility::visible_from(module, from)
                    && language::rust_refs::import_module(references, from, &site.path) == Some(module.to_string()))
                .then(|| name.to_string());
            }
            if site.path.last().is_none_or(|leaf| leaf != name)
                || language::rust_refs::import_module(references, from, &site.path[..site.path.len() - 1]) != Some(module.to_string())
            {
                return None;
            }
            Some(site.local.clone())
        })
        .collect()
}

/// Makes a static inside a `thread_local!` block visible to a module outside
/// the one that declares it.
fn promote_hidden_static(text: &str, name: &str) -> String {
    let Some((_, at, public)) = language::rust_refs::thread_local_names(text).into_iter().find(|(n, ..)| n.as_str() == name) else {
        return text.to_string();
    };
    if public {
        return text.to_string();
    }
    let mut out = text.to_string();
    out.insert_str(at.start, "pub(crate) ");
    out
}

/// The path to write for `target` in `mention`'s module: a path written from
/// the crate root stays root-relative, so a re-export keeps the path that
/// reaches it; otherwise it is the target's own module path.
fn reference_path(ix: &Index, moved: &[bool], target: usize, target_new: &str, mention: usize, segs: &[String]) -> String {
    if !moved[target] {
        let path = rooted_path(ix, &ix.syms[mention].module, target_new, segs);
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
            if target_names_after_move(cx, &header, &module)?.contains(&local) {
                return fail(format!("E1004 {module}.{local} already exists in {module}; the format capture cannot be imported there"));
            }
        }
    }
    Ok(())
}

/// The names the destination module binds after the move.
fn target_names_after_move(cx: &Rewrite, header: &str, module: &str) -> Result<HashSet<String>, Failure> {
    target_names(cx.ix, header, module, |i, _| in_module_after_move(cx, i, module))
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
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
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
    let site = language::rust_refs::ImportSite { local: name.to_string(), path, range: 0..0, alias: None, outside: 0, glob: false };
    if seen.insert((from.to_string(), site.local.clone(), site.path.clone(), site.glob)) {
        carried.push(Carry { from: from.to_string(), site, rename: true });
    }
}

/// The tests that reach moved code but stay, with one staying symbol each.
fn staying_tests_reply(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], target: &str) -> String {
    let test_file = format!("{target}::tests");
    let mut out = String::new();
    for t in (0..ix.syms.len()).filter(|&t| !moved[t] && reach::is_test_fn(ix, t)) {
        let module = &ix.syms[t].module;
        if module == target || module == &test_file {
            continue;
        }
        let (outside, locals) = test_covered(ix, ids, t);
        if !outside.iter().any(|&u| moved[u]) {
            continue;
        }
        let stay = outside.iter().chain(locals.iter()).copied().filter(|&u| !moved[u] && !reach::is_test_fn(ix, u)).min();
        if let Some(stay) = stay {
            out += &format!("stays: {} (reaches {}, which stays)\n", ix.syms[t].qname(), ix.syms[stay].qname());
        }
    }
    out
}

/// One moved item's mentions in one module, which share a spelling.
struct MovedMentions {
    test_only: bool,
    sites: Vec<(usize, Range<usize>)>,
}

/// Whether a header imports `path` under `name` for the code that needs it.
fn header_binds(header: &str, name: &str, path: &str, test_only: bool) -> bool {
    let bound = language::rust_refs::import_sites_in(header)
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
        .chain(module_use_symbols(cx.ix, |i, _| in_module_after_move(cx, i, module)).map(|s| s.text.as_str()))
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

/// Whether symbol `i` is in `module` after the move.
fn in_module_after_move(cx: &Rewrite, i: usize, module: &str) -> bool {
    if cx.moved[i] { cx.newmod[i] == module } else { cx.ix.syms[i].module == module }
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
            && (target_names_after_move(cx, &base, &module)?.contains(&name)
                || is_prelude_name(&name)
                || glob_binds(cx, &base, &module, &name, t)?);
        let text = if taken { path.clone() } else { name };
        for (i, range) in group.sites {
            edits.entry(i).or_default().push((range, text.clone()));
        }
        if !taken && !imported {
            let line = if group.test_only { format!("#[cfg(test)] use {path};") } else { format!("use {path};") };
            newheaders.insert(module, add_import_line(&base, &line));
        }
    }
    Ok(())
}

/// The test module a target's tests live in: `<target>::tests`.
fn test_module_path(target: &str) -> String {
    format!("{target}::tests")
}

/// The module's `use` symbols whose index `belongs` says are in it.
fn module_use_symbols<'a>(ix: &'a Index, belongs: impl Fn(usize, &Sym) -> bool) -> impl Iterator<Item = &'a Sym> {
    ix.syms
        .iter()
        .enumerate()
        .filter(move |(i, symbol)| symbol.parent.is_none() && symbol.kind_text == "use" && belongs(*i, symbol))
        .map(|(_, symbol)| symbol)
}

/// Whether `text`'s imports include a glob (`use path::*;`).
fn imports_have_glob(text: &str, module: &str) -> Result<bool, Failure> {
    Ok(language::rust_refs::import_sites_in(text)
        .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?
        .iter()
        .any(|site| site.glob))
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

/// Drops the globs this move added to the test module's header that cargo
/// reports unused, leaving the imports a user wrote.
fn drop_unused_carried_globs(
    s: &Store,
    test: &TestModule,
    added: &[Vec<String>],
    warnings: &[crate::diagnostics::Diag],
) -> Result<(), Failure> {
    let mut unused: HashSet<(String, Vec<String>, bool)> = HashSet::new();
    let mut stale: Vec<&crate::diagnostics::Diag> = vec![];
    for path in added {
        let glob = format!("{}::*", path.join("::"));
        let found: Vec<&crate::diagnostics::Diag> = warnings
            .iter()
            .filter(|d| {
                !d.error && d.module.as_deref() == Some(test.module.as_str()) && d.text.contains(&format!("unused import: `{glob}`"))
            })
            .collect();
        if !found.is_empty() {
            unused.insert(("*".to_string(), path.clone(), true));
            stale.extend(found);
        }
    }
    if unused.is_empty() {
        return Ok(());
    }
    let ix = s.index()?;
    let Some(row) = ix.modules.iter().find(|row| row.path == test.module) else { return Ok(()) };
    let header = drop_import_keys(&row.header, &test.module, &unused)?;
    if header == row.header {
        return Ok(());
    }
    s.set_header(&test.module, &header)?;
    s.forget_warnings(&stale)?;
    s.materialize(&s.index()?)?;
    Ok(())
}
