use super::*;
use crate::Sym;
use crate::Index;
use crate::failure::Failure;
use crate::failure::fail;
use crate::Store;
use crate::extract::rust::test_files::belongs_in_test_file;
use crate::rust_submit::rust_namespace;
use std::collections::BTreeSet;
use crate::extract::rust::mark;
use crate::extract::rust::bring_helpers;
use crate::extract::rust::test_files::move_tests;
use crate::extract::rust::id_indexes;
use crate::extract::rust::impl_owner_symbol;
use crate::extract::rust::impls::children;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;
use crate::extract::rust::rewrite::full_path;
use crate::extract::rust::leaf;
use crate::extract::rust::rewrite::splice;
use crate::extract::rust::rewrite::is_public;
use crate::extract::rust::rewrite::declaration;
use crate::extract::rust::qualify_trait_impls;
use crate::extract::rust::names_need_glob;
use crate::extract::rust::Carry;
use crate::extract::rust::LocalImport;
use crate::extract::rust::called_names_where;
use crate::extract::rust::declaration_visibility;
use crate::extract::rust::rewrite::new_module;
use crate::extract::rust::HiddenThreadLocal;
use crate::extract::rust::enclosing_module;
use crate::extract::rust::promote_hidden_static;
use crate::extract::rust::raise_target_modules;
use crate::extract::rust::glob_reach::reaching_modules;
use crate::extract::rust::glob_reach::reach_map;
use crate::extract::rust::impls::ImplTarget;
use crate::extract::rust::impls::Placement;
use crate::extract::rust::imports::absolute_import_path;
use crate::extract::rust::imports::crate_root_item;
use crate::extract::rust::imports::crate_root_name;
use crate::extract::rust::imports::module_imports;
use crate::extract::rust::visibility::visible_from;
use crate::extract::rust::visibility::method_fields;
use crate::rust_submit::rust_descendants;
use crate::extract::rust::rewrite::rewrite;

pub(super) fn module_name(input: &str) -> String {
    let mut name = input.trim().trim_end_matches(".rs").to_string();
    if let Some(rest) = name.strip_prefix("src/") {
        name = rest.to_string();
    }
    name
}

pub(crate) fn movable(ix: &Index, i: usize) -> Result<(), Failure> {
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

pub(crate) fn helper_kind(kind: &str) -> bool {
    matches!(kind, "fn" | "const" | "static")
}

pub(crate) fn check_clashes(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], target: &str) -> Result<(), Failure> {
    for i in (0..ix.syms.len()).filter(|&i| {
        moved[i]
            && ix.syms[i].parent.is_none()
            && ix.syms[i].module != target
            && ix.syms[i].kind_text != "impl"
            && !(belongs_in_test_file(ix, &ids, i) && ix.syms[i].kind_text != "mod")
    }) {
        let sym = &ix.syms[i];
        if rust_namespace(&sym.kind_text) & crate::language::rust::TYPE_NAMESPACE != 0
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

pub(crate) fn destination(s: &Store, ix: &Index, to: &str) -> Result<(String, Option<crate::rust_submit::RustNew>), Failure> {
    match s.resolve_module(ix, to) {
        Some(row) => Ok((row.path.clone(), None)),
        None => match s.rust_created(ix, to) {
            Ok(new) => Ok((new.module.clone(), Some(new))),
            Err(_) => fail(format!("E1003 {to} is not a module name; use --to <module>, e.g. --to helpers")),
        },
    }
}

pub(crate) fn roots(s: &Store, ix: &Index, names: &[String]) -> Result<Vec<usize>, Failure> {
    let mut roots: Vec<usize> = vec![];
    for name in names {
        let i = match named_impl(ix, name)? {
            Some(i) => i,
            None => match s.resolve(ix, name) {
                Ok(i) => i,
                Err(e) => {
                    if s.resolve_module(ix, name).is_some() {
                        return fail(crate::extract::MODULE_HINT);
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

pub(crate) fn moving(ix: &Index, roots: &[usize]) -> Result<(Vec<bool>, Vec<usize>), Failure> {
    let n = ix.syms.len();
    let ids = id_indexes(ix);
    let mut moved = vec![false; n];
    for &i in roots {
        mark(ix, &mut moved, i);
    }
    check_direct(ix, &ids, roots, &mut moved)?;
    add_impls(ix, &mut moved)?;
    let mut helpers: Vec<usize> = vec![];
    bring_helpers(ix, &mut moved, &mut helpers);
    move_tests(ix, &ids, &mut moved, &mut helpers);
    Ok((moved, helpers))
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

fn module_named_like(module: &str, prefix: &str) -> bool {
    module == prefix || module.ends_with(&format!("::{prefix}")) || prefix.ends_with(&format!("::{module}"))
}

pub(super) fn check_direct(
    ix: &Index,
    ids: &std::collections::HashMap<&str, usize>,
    roots: &[usize],
    moved: &mut [bool],
) -> Result<(), Failure> {
    let children = children(ix);
    let mut chosen: BTreeSet<usize> = BTreeSet::new();
    for &root in roots {
        let sym = &ix.syms[root];
        if sym.kind_text == "impl" {
            impl_owner_symbol(ix, root)?;
            if sym.name.starts_with('<') && crate::language::rust_refs::impl_trait(&ix.syms, &ix.modules, root).is_none() {
                return fail(format!(
                    "E6013 {} is a trait impl whose trait path does not resolve exactly; make the path name one trait in this crate",
                    sym.qname()
                ));
            }
            chosen.insert(root);
            continue;
        }
        let Some(parent) = sym.parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" {
            continue;
        }
        impl_owner_symbol(ix, at)?;
        if ix.syms[at].name.starts_with('<') {
            if !moved[at] {
                if crate::language::rust_refs::impl_trait(&ix.syms, &ix.modules, at).is_none() {
                    return fail(format!(
                        concat!(
                            "E6013 {} is a method of the trait impl {}, whose trait path does not resolve exactly; ",
                            "make the path name one trait in this crate"
                        ),
                        sym.qname(),
                        ix.syms[at].qname()
                    ));
                }
                let siblings = children.get(parent).map(Vec::as_slice).unwrap_or_default();
                if siblings.iter().any(|&j| ix.syms[j].kind_text != "use" && !moved[j]) {
                    return fail(format!(
                        "E6013 {} is a method of the trait impl {}; a trait impl can move only whole: select {} or every method in it",
                        sym.qname(),
                        ix.syms[at].qname(),
                        ix.syms[at].qname()
                    ));
                }
                mark(ix, moved, at);
            }
            chosen.insert(at);
        } else if !moved[at] {
            chosen.insert(at);
        }
    }
    if chosen.len() > 1 {
        let list: Vec<String> = chosen.iter().map(|&i| ix.syms[i].qname()).collect();
        return fail(format!("E6013 methods from more than one impl move at once: {}; select methods from one impl", list.join(", ")));
    }
    Ok(())
}

pub(super) fn named_impl(ix: &Index, input: &str) -> Result<Option<usize>, Failure> {
    let Some(open) = input.find('<') else { return Ok(None) };
    let Some(inner) = input[open + 1..].strip_suffix('>') else { return Ok(None) };
    if inner.is_empty() || inner.contains(" as ") {
        return Ok(None);
    }
    let prefix = input[..open].trim_end_matches(['.', ':']);
    let found: Vec<usize> = (0..ix.syms.len())
        .filter(|&i| {
            let sym = &ix.syms[i];
            sym.kind_text == "impl"
                && sym.parent.is_none()
                && (sym.name == inner || crate::store::without_generics(&sym.name) == inner)
                && (prefix.is_empty() || module_named_like(&sym.module, prefix))
        })
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|&i| {
                    let methods: Vec<&str> =
                        ix.syms.iter().filter(|c| c.parent.as_deref() == Some(ix.syms[i].id.as_str())).map(|c| c.name.as_str()).collect();
                    format!(
                        "  {} (impl; methods: {})",
                        ix.syms[i].qname(),
                        if methods.is_empty() { "none".to_string() } else { methods.join(", ") }
                    )
                })
                .collect();
            fail(format!("E1002 ambiguous name: {input} could be\n{}; select methods from the block you want", list.join("\n")))
        }
    }
}

pub(crate) struct RustMove<'a> {
    pub(crate) ix: &'a Index,
    pub(crate) refs: crate::language::rust_refs::References<'a>,
    pub(crate) target: String,
    pub(crate) create: Option<crate::rust_submit::RustNew>,
    pub(crate) moved: Vec<bool>,
    pub(crate) newmod: Vec<String>,
    pub(crate) helpers: Vec<usize>,
    pub(crate) already: Vec<String>,
    pub(crate) going: Vec<usize>,
    pub(crate) newtext: Vec<String>,
    pub(crate) newheaders: HashMap<String, String>,
    pub(crate) promote: BTreeSet<usize>,
    pub(crate) raised: Vec<(String, &'static str)>,
    pub(crate) placements: Vec<crate::extract::rust::impls::Placement>,
}

pub(crate) fn make_visible(text: &str) -> String {
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

/// Imports from another crate that the moved code uses: a named import it
/// mentions, a macro it calls, or a glob that may bind a name it mentions.
fn external_imports(
    ix: &Index,
    moved: &[bool],
    refs: &crate::language::rust_refs::References<'_>,
) -> Result<(Vec<Carry>, HashMap<String, Vec<crate::language::rust_refs::ImportSite>>), Failure> {
    let mut by_module: HashMap<String, Vec<crate::language::rust_refs::ImportSite>> = HashMap::new();
    let mut carried = vec![];
    let modules: BTreeSet<String> = (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
    for module in modules {
        let sites = crate::language::rust_refs::references_external_imports(refs, &module)?;
        let module_moved: Vec<usize> = (0..ix.syms.len()).filter(|&i| moved[i] && ix.syms[i].module == module).collect();
        let named: HashSet<String> = crate::language::rust_refs::import_sites(&ix.syms, &ix.modules, &module, true)?
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

pub(crate) fn rewrite_targets(ix: &Index, moved: &[bool]) -> BTreeSet<usize> {
    let n = ix.syms.len();
    let mut targets: BTreeSet<usize> = (0..n).filter(|&i| moved[i]).collect();
    for i in (0..n).filter(|&i| moved[i]) {
        for u in crate::index::uses_split(ix, i).0 {
            if !moved[u] {
                targets.insert(u);
            }
        }
    }
    targets
}

pub(crate) fn can_promote(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> bool {
    match ix.syms[i].parent.as_deref() {
        None => true,
        Some(parent) => ids.get(parent).is_some_and(|&p| ix.syms[p].kind_text == "impl" && !ix.syms[p].name.starts_with('<')),
    }
}

/// The local imports (traits for method calls, macros) the moved code needs,
/// and every such import with whether code staying in its module needs it.
fn local_import_uses(
    ix: &Index,
    moved: &[bool],
    newmod: &[String],
    refs: &crate::language::rust_refs::References<'_>,
    reach: &HashMap<String, HashSet<String>>,
) -> Result<(Vec<Carry>, Vec<LocalImport>), Failure> {
    let mut carried = vec![];
    let mut imports = vec![];
    let modules = moved_sources(ix, moved);
    for module in modules {
        let stays = |i: usize| !moved[i] || newmod[i] == module;
        let moved_methods = called_names_where(ix, crate::language::rust_refs::CallKind::Method, |i| moved[i] && ix.syms[i].module == module);
        let moved_macros = called_names_where(ix, crate::language::rust_refs::CallKind::Macro, |i| moved[i] && ix.syms[i].module == module);
        let staying_methods: HashMap<String, HashSet<String>> = reaching_modules(reach, &module)
            .map(|m| {
                let names = called_names_where(ix, crate::language::rust_refs::CallKind::Method, |i| ix.syms[i].module == m && stays(i));
                (m.to_string(), names)
            })
            .collect();
        let staying_macros: HashMap<String, HashSet<String>> = reaching_modules(reach, &module)
            .map(|m| {
                let names = called_names_where(ix, crate::language::rust_refs::CallKind::Macro, |i| ix.syms[i].module == m && stays(i));
                (m.to_string(), names)
            })
            .collect();
        let external: HashSet<(String, Vec<String>, bool)> = crate::language::rust_refs::references_external_imports(refs, &module)?
            .into_iter()
            .map(|site| (site.local, site.path, site.glob))
            .collect();
        for (trait_i, site) in crate::language::rust_refs::imported_traits(refs, &module)? {
            let moved_needs = crate::language::rust_refs::trait_has_method(refs, trait_i, &moved_methods);
            let staying_needs = crate::language::rust_refs::trait_has_method(refs, trait_i, staying_methods.get(&module).unwrap());
            let needed_by = import_needed_by(&staying_methods, &module, staying_needs, |names| {
                crate::language::rust_refs::trait_has_method(refs, trait_i, names)
            });
            record_local_import(&module, &site, false, moved_needs, staying_needs, needed_by, &mut carried, &mut imports);
        }
        for site in crate::language::rust_refs::import_sites(&ix.syms, &ix.modules, &module, true)? {
            let is_external = external.contains(&(site.local.clone(), site.path.clone(), site.glob));
            if !site.glob
                && is_external
                && let Some(methods) = crate::language::rust_refs::std_trait_methods(&site.path)
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

pub(crate) fn rewrite_move<'a>(
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
    let references = crate::language::rust_refs::references(&ix.syms, &ix.modules);
    let (mut carried, external_by_module) = external_imports(ix, &moved, &references)?;
    check_clashes(ix, &ids, &moved, &target)?;
    let placements = placements(ix, &ids, &target, &moved, &going)?;

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
    carried.extend(parent_imports(ix, &ids, &moved, &newmod, &target, &references, &newheaders)?);
    if create.is_some() {
        for carry in &mut carried {
            carry.site.path = crate::extract::rust::imports::absolute_import_path(ix, &carry.from, &carry.site.path);
        }
    }

    let (mut newtext, mut newheaders, mut promote, public) = rewrite(
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
    if create.is_some() {
        root_moved_use_imports(ix, &moved, &newmod, &mut newtext);
    }
    qualify_thread_locals(ix, &ids, &moved, &newmod, &references, &mut newtext);
    promote_access(ix, &ids, &moved, &target, &mut newtext, &mut promote);
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
    let raised = raise_target_modules(ix, &references, &target, create.as_ref(), &moved, &mut newtext, &mut newheaders)?;

    for &i in &public {
        if can_promote(ix, &ids, i) {
            newtext[i] = make_public(&newtext[i]);
        }
    }

    Ok(RustMove {
        ix,
        refs: references,
        target,
        create,
        moved,
        newmod,
        helpers,
        already,
        going,
        newtext,
        newheaders,
        promote,
        placements,
        raised,
    })
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

pub(crate) fn moved_sources(ix: &Index, moved: &[bool]) -> BTreeSet<String> {
    (0..ix.syms.len()).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect()
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
    site: &crate::language::rust_refs::ImportSite,
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

/// The statics declared inside `thread_local!` blocks that have no symbol of
/// their own, with the module that declares them and their full path.
fn thread_local_paths(ix: &Index, ids: &HashMap<&str, usize>, names: &HashSet<(String, String)>) -> Vec<HiddenThreadLocal> {
    let mut paths = vec![];
    for (block, tl) in ix.syms.iter().enumerate().filter(|(_, s)| s.kind_text == "thread_local!") {
        let module = enclosing_module(ix, ids, block);
        for (name, ..) in crate::language::rust_refs::thread_local_names(&tl.text) {
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
    references: &crate::language::rust_refs::References<'_>,
    newtext: &mut [String],
) {
    let names = module_names(ix, ids);
    let paths = thread_local_paths(ix, ids, &names);
    if paths.is_empty() {
        return;
    }
    let mut imports: HashMap<String, Vec<crate::language::rust_refs::ImportSite>> = HashMap::new();
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
        newtext[i] = crate::language::rust_refs::rename_names(references, i, &newtext[i], &renames);
        for tl in used {
            let target = if moved[tl.block] { &newmod[tl.block] } else { &tl.module };
            if !crate::extract::rust::visibility::visible_from(target, &newmod[i]) {
                newtext[tl.block] = promote_hidden_static(&newtext[tl.block], &tl.name);
            }
        }
    }
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

/// Whether `from` imports `name` from `module`.
fn imports_from(
    ix: &Index,
    references: &crate::language::rust_refs::References<'_>,
    ids: &HashMap<&str, usize>,
    from: &str,
    module: &str,
    name: &str,
    imports: &mut HashMap<String, Vec<crate::language::rust_refs::ImportSite>>,
) -> Vec<String> {
    let sites = imports.entry(from.to_string()).or_insert_with(|| module_imports(ix, ids, from));
    sites
        .iter()
        .filter_map(|site| {
            if site.glob {
                return (crate::extract::rust::visibility::visible_from(module, from)
                    && crate::language::rust_refs::import_module(references, from, &site.path) == Some(module.to_string()))
                .then(|| name.to_string());
            }
            if site.path.last().is_none_or(|leaf| leaf != name)
                || crate::language::rust_refs::import_module(references, from, &site.path[..site.path.len() - 1]) != Some(module.to_string())
            {
                return None;
            }
            Some(site.local.clone())
        })
        .collect()
}

pub(super) fn placements(
    ix: &Index,
    ids: &std::collections::HashMap<&str, usize>,
    target: &str,
    moved: &[bool],
    roots: &[usize],
) -> Result<Vec<Placement>, Failure> {
    let children = children(ix);
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for &root in roots {
        let sym = &ix.syms[root];
        if sym.kind_text == "impl" {
            continue;
        }
        let Some(parent) = sym.parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" || moved[at] {
            continue;
        }
        groups.entry(at).or_default().push(root);
    }
    let mut out = vec![];
    for (source, members) in groups {
        let owner = impl_owner_symbol(ix, source)?;
        let existing = target_impl(ix, target, owner, moved)?;
        if let Some(at) = existing {
            for &root in &members {
                let name = leaf(&ix.syms[root].name);
                let duplicate = children
                    .get(ix.syms[at].id.as_str())
                    .and_then(|members| members.iter().copied().find(|&j| !moved[j] && leaf(&ix.syms[j].name) == name));
                if let Some(that) = duplicate {
                    return fail(format!("E1004 {} already exists; {} cannot move there", ix.syms[that].qname(), ix.syms[root].qname()));
                }
            }
        }
        let (text, tail) =
            if existing.is_some() { (String::new(), String::new()) } else { new_impl_parts(&ix.syms[source], &ix.syms[owner]) };
        out.push(Placement { roots: members, target: ImplTarget { name: ix.syms[source].name.clone(), existing, text, tail } });
    }
    Ok(out)
}

fn target_impl(ix: &Index, target: &str, owner: usize, moved: &[bool]) -> Result<Option<usize>, Failure> {
    let impls: Vec<usize> = (0..ix.syms.len())
        .filter(|&i| {
            let sym = &ix.syms[i];
            sym.kind_text == "impl" && sym.parent.is_none() && sym.module == target && !moved[i] && !sym.name.starts_with('<')
        })
        .collect();
    let owners = crate::language::rust_refs::impl_owner_batch(&ix.syms, &ix.modules, &impls);
    Ok(impls.into_iter().zip(owners).find_map(|(i, owner_here)| match owner_here {
        crate::language::rust_refs::ImplOwner::One(t) if t == owner => Some(i),
        _ => None,
    }))
}

fn new_impl_parts(source: &crate::Sym, owner: &crate::Sym) -> (String, String) {
    let Some(range) = impl_self_range(&source.text) else {
        return (format!("impl {} {{\n    ", full_path(&owner.module, &owner.name)), "\n}".to_string());
    };
    let current = &source.text[range.clone()];
    let generics = current.find('<').map_or(String::new(), |at| current[at..].to_string());
    let ty = format!("{}{generics}", full_path(&owner.module, &owner.name));
    let mut text = source.text.clone();
    text.replace_range(range, &ty);
    (text, "\n}".to_string())
}

/// The source and crate-root imports moved code needs for names it mentions
/// only in a pattern or as a path head, which the reference graph leaves out
/// of its uses.
pub(super) fn parent_imports(
    ix: &Index,
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    newmod: &[String],
    target: &str,
    references: &crate::language::rust_refs::References<'_>,
    newheaders: &HashMap<String, String>,
) -> Result<Vec<Carry>, Failure> {
    let targets = rewrite_targets(ix, moved);
    let hidden = thread_local_paths(ix, ids, &module_names(ix, ids));
    let mut carried = vec![];
    let mut seen = HashSet::new();
    for i in (0..ix.syms.len()).filter(|&i| moved[i] && newmod[i] == target) {
        let from = ix.syms[i].module.clone();
        let mut sites: Vec<(String, crate::language::rust_refs::ImportSite)> =
            crate::language::rust_refs::import_sites(&ix.syms, &ix.modules, &from, true)?.into_iter().map(|site| (from.clone(), site)).collect();
        sites.extend(crate::extract::rust::glob_reach::super_glob_imports(ix, ids, newheaders, &from));
        let crate_root = crate::extract::rust::glob_reach::reaches_crate_root(ix, ids, newheaders, &from);
        for name in &references.mentioned[i] {
            if !parent_name_needed(ix, &targets, name) {
                continue;
            }
            let found = sites.iter().find(|(_, site)| !site.glob && &site.local == name);
            let (owner, mut site) = match found {
                Some((owner, site)) => (owner.clone(), site.clone()),
                None if crate_root && crate_root_item(ix, name) => {
                    ("crate".to_string(), crate::language::rust_refs::ImportSite::named(vec![name.clone()]))
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
    let Ok(sites) = crate::language::rust_refs::import_sites_in(text) else { return text.to_string() };
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

pub(super) fn promote_access(
    ix: &Index,
    ids: &std::collections::HashMap<&str, usize>,
    moved: &[bool],
    target: &str,
    newtext: &mut [String],
    promote: &mut BTreeSet<usize>,
) {
    let mut owners: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for method in 0..ix.syms.len() {
        if !moved[method] || ix.syms[method].kind_text != "fn" {
            continue;
        }
        let Some(parent) = ix.syms[method].parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" {
            continue;
        }
        let Ok(owner) = impl_owner_symbol(ix, at) else { continue };
        if !moved[owner] && !visible_from(&ix.syms[owner].module, target) {
            owners.entry(owner).or_default().push(method);
            if !is_public(&newtext[owner]) {
                promote.insert(owner);
            }
        }
        let users = crate::index::users_split(ix, method).0;
        if users.iter().any(|&u| !moved[u] && !visible_from(target, &ix.syms[u].module))
            && can_promote(ix, ids, method)
            && !is_public(&newtext[method])
        {
            promote.insert(method);
        }
        for &used in &crate::index::uses_split(ix, method).0 {
            if !moved[used]
                && can_promote(ix, ids, used)
                && helper_kind(&ix.syms[used].kind_text)
                && !is_public(&newtext[used])
                && !visible_from(&ix.syms[used].module, target)
            {
                promote.insert(used);
            }
        }
    }
    for (owner, methods) in owners {
        let owner_name = &ix.syms[owner].name;
        let used: HashSet<String> = methods.iter().flat_map(|&m| method_fields(&newtext[m], owner_name)).collect();
        newtext[owner] = promote_fields(&newtext[owner], &used);
    }
    let mut used_fields: std::collections::BTreeMap<usize, HashSet<String>> = std::collections::BTreeMap::new();
    for moved_fn in 0..ix.syms.len() {
        if !moved[moved_fn] || ix.syms[moved_fn].kind_text != "fn" {
            continue;
        }
        for &used in &ix.uses[moved_fn] {
            if moved[used] || ix.syms[used].kind_text != "struct" {
                continue;
            }
            let fields = method_fields(&newtext[moved_fn], &ix.syms[used].name);
            if fields.is_empty() || visible_from(&ix.syms[used].module, target) {
                continue;
            }
            if !is_public(&newtext[used]) {
                promote.insert(used);
            }
            used_fields.entry(used).or_default().extend(fields);
        }
    }
    for (owner, fields) in used_fields {
        newtext[owner] = promote_fields(&newtext[owner], &fields);
    }
    for owner in 0..ix.syms.len() {
        if !moved[owner] || ix.syms[owner].kind_text != "struct" {
            continue;
        }
        newtext[owner] = promote_fields(&newtext[owner], &all_fields(&newtext[owner]));
        promote.insert(owner);
    }
}

pub(super) fn promote_fields(text: &str, used: &HashSet<String>) -> String {
    let Ok(item) = syn::parse_str::<syn::ItemStruct>(text) else { return text.to_string() };
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    for (index, field) in item.fields.iter().enumerate() {
        let name = field.ident.as_ref().map_or_else(|| index.to_string(), |name| name.to_string());
        if !used.contains(&name) {
            continue;
        }
        match &field.vis {
            syn::Visibility::Public(_) => {}
            syn::Visibility::Inherited => {
                let at = match &field.ident {
                    Some(name) => syn::spanned::Spanned::span(name).byte_range().start,
                    None => syn::spanned::Spanned::span(&field.ty).byte_range().start,
                };
                edits.push((at..at, "pub(crate) ".to_string()));
            }
            syn::Visibility::Restricted(vis) => {
                if vis.path.segments.len() == 1 && vis.path.segments[0].ident == "crate" {
                    continue;
                }
                edits.push((syn::spanned::Spanned::span(&field.vis).byte_range(), "pub(crate) ".to_string()));
            }
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = text.to_string();
    for (range, with) in edits.into_iter().rev() {
        out.replace_range(range, &with);
    }
    out
}

fn all_fields(text: &str) -> HashSet<String> {
    let Ok(item) = syn::parse_str::<syn::ItemStruct>(text) else { return HashSet::new() };
    item.fields
        .iter()
        .enumerate()
        .map(|(index, field)| field.ident.as_ref().map_or_else(|| index.to_string(), |name| name.to_string()))
        .collect()
}

/// The declaration with plain `pub` visibility, so a dropped re-export keeps
/// the reach it had.
pub(super) fn make_public(text: &str) -> String {
    let Some((at, code)) = declaration(text) else { return text.to_string() };
    if code.starts_with("pub ") || code.starts_with("pub\n") {
        return text.to_string();
    }
    if code.starts_with("pub(") {
        let Some(end) = code.find(')') else { return text.to_string() };
        return format!("{}pub{}", &text[..at], &text[at + end + 1..]);
    }
    format!("{}pub {}", &text[..at], &text[at..])
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

pub(crate) fn move_roots(
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
        let rename = crate::extract::rust::test_files::is_test_container(ix, i) && detach;
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
