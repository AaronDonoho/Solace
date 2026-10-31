use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;

use rusqlite::params;

use crate::extract::plan::Args;
use crate::language::rust_refs::{Site, Via};
use crate::rust_submit::rust_descendants;
use crate::Sym;
use crate::{ Index,  Store,  is_test, language, reach};
use crate::failure::fail;
use crate::failure::Out;
use crate::failure::Failure;
#[path = "rust_tests.rs"]
mod tests;
mod visibility;
mod impls;
pub(crate) mod merge;
mod glob_reach;
mod imports;
mod plan;
mod test_files;
pub(super) mod rewrite;
mod repair;
use crate::extract::rust::plan::helper_kind;
use crate::extract::rust::plan::destination;
use crate::extract::rust::plan::roots;
use crate::extract::rust::plan::moving;
use crate::extract::rust::plan::RustMove;
use crate::extract::rust::plan::rewrite_move;
use crate::extract::rust::plan::move_roots;
use crate::extract::rust::test_files::whole_test_containers;
use crate::extract::rust::test_files::test_groups;
use crate::extract::rust::test_files::test_container;
use crate::extract::rust::test_files::test_covered;
use crate::extract::rust::test_files::check_test_containers;
use crate::extract::rust::test_files::move_test_groups;
use crate::extract::rust::test_files::moved_tests_reply;
use crate::extract::rust::test_files::test_module;
use crate::extract::rust::test_files::retire_test_containers;
use crate::extract::rust::test_files::plan_test_module;
use crate::extract::rust::test_files::belongs_in_test_file;
use crate::extract::rust::test_files::drop_unused_carried_globs;
use crate::extract::rust::rewrite::Container;
use crate::extract::rust::rewrite::full_path;
use crate::extract::rust::rewrite::last_root_sep;
use crate::extract::rust::rewrite::is_public;
use crate::extract::rust::rewrite::declaration;
use crate::extract::rust::rewrite::all_use_sites;
use crate::extract::rust::rewrite::new_module;
use crate::extract::rust::rewrite::module_imports_rewritten;
use crate::extract::rust::rewrite::changed_imports;
use crate::lockdown::commands::trim_output;
pub(super) fn extract(s: &Store, p: &Args, budget: usize) -> Out {
    let ix = s.index()?;
    let (target, create) = destination(s, &ix, &plan::module_name(&p.to))?;
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

fn container_key(module: &str, site: &Site) -> Container {
    (site.symbol, module.to_string())
}

pub(crate) fn leaf(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

enum Visibility {
    Private,
    Restricted,
    Public,
}

fn apply(s: &Store, budget: usize, mut created_files: Vec<std::path::PathBuf>, mut m: RustMove<'_>) -> Out {
    let ix = m.ix;
    let n = ix.syms.len();
    let ids = id_indexes(ix);
    let mut moved = m.moved.clone();
    let whole = whole_test_containers(ix, &mut moved);
    let existing_tests = test_module(ix, &m.target);
    let inline_tests = test_files::inline_test_container(ix, &m.target);
    let file_tests = existing_tests.is_some() || inline_tests.is_some();
    let groups = test_groups(ix, &ids, &moved, &whole, file_tests);
    let test = plan_test_module(s, &mut m, &ids, existing_tests, inline_tests, &groups, &moved)?;
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
        imports::carry_test_imports(ix, &moved, &m.newmod, &ids, &m.refs, &groups, t, &mut m.newtext, &mut m.newheaders)?
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
        drop_super_globs(s, ix, &ids, &moved, &m.target, &whole, &mut m.newtext)?;

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
            move_test_groups(s, ix, &groups, t, &mut m.newtext)?;
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
                s.set_symbol_text(&ix.syms[i].id, &ix.syms[i].name, &m.newtext[i])?;
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

        let (warnings, repair) = repair::finish(s, &before, &m)?;
        for (id, text) in &repair.symbols {
            if let Some(&i) = ids.get(id.as_str()) {
                m.newtext[i] = text.clone();
            }
        }
        if let Some(t) = &test {
            drop_unused_carried_globs(s, t, &test_globs, &warnings)?;
        }
        Ok(move_reply(&m, &ids, &moved, &retired, &repair, budget) + &crate::rust_submit::dropped_impls_note(&dropped))
    })?;
    Ok(out)
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
    crate_glob: HashSet<String>,
}

fn mark(ix: &Index, moved: &mut [bool], i: usize) {
    for j in rust_descendants(ix, i) {
        moved[j] = true;
    }
}

fn bring_helpers(ix: &Index, moved: &mut [bool], helpers: &mut Vec<usize>) {
    let mut moving = HashSet::new();
    bring_helpers_with(ix, moved, helpers, &mut moving);
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

/// Whether `names` has a name that `module`'s own items and named imports do
/// not explain, so a glob in the module may be its source.
pub(crate) fn names_need_glob(ix: &Index, module: &str, names: &HashSet<String>, named: &HashSet<String>) -> bool {
    names.iter().any(|name| !named.contains(name) && !is_known_name(ix, module, name) && !is_builtin_macro(name))
}

/// Whether `name` is in scope in path position without a glob or import: a
/// keyword, primitive, prelude item, crate root, or the module's own item.
pub(crate) fn is_known_name(ix: &Index, module: &str, name: &str) -> bool {
    is_prelude_path_name(name) || imports::module_item(ix, module, name)
}

/// Rust syntax and prelude names, which a glob never has to supply.
fn is_prelude_name(name: &str) -> bool {
    is_prelude_path_name(name) || is_builtin_macro(name)
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

/// A `use` entry the moved code needs in its target module.
pub(crate) struct Carry {
    from: String,
    site: language::rust_refs::ImportSite,
    rename: bool,
}

/// A local `use` entry the move may have to drop from its source module.
pub(crate) struct LocalImport {
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

fn move_reply(
    m: &RustMove<'_>,
    ids: &HashMap<&str, usize>,
    moved: &[bool],
    retired: &[String],
    repair: &repair::Repair,
    budget: usize,
) -> String {
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
    out += &staying_tests_reply(m, ids, moved);
    if let Some(c) = &m.create {
        out += &format!("Created module {} ({})\n", c.module, c.file);
    }
    if !m.already.is_empty() {
        out += &format!("Already in {}: {}\n", m.target, m.already.join(", "));
    }
    let mut promoted: BTreeSet<String> = m
        .promote
        .iter()
        .filter(|&&i| !is_public(&ix.syms[i].text) && is_public(&m.newtext[i]))
        .map(|&i| format!("{}.{}", new_module(ix, ids, moved, &m.target, i), ix.syms[i].name))
        .collect();
    promoted.extend(repair.promoted.iter().cloned());
    if !promoted.is_empty() {
        out += &format!("Now pub(crate): {}\n", promoted.into_iter().collect::<Vec<_>>().join(", "));
    }
    let mut raised: BTreeSet<(String, &'static str)> = m.raised.iter().cloned().collect();
    raised.extend(repair.raised.iter().cloned());
    out += &raised_modules_reply(&raised.into_iter().collect::<Vec<_>>());
    let mut imported: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (module, line) in &repair.imports {
        imported.entry(module).or_default().push(line);
    }
    for (module, lines) in imported {
        out += &format!("Imports added: {module}: {}\n", lines.join(", "));
    }
    let edited =
        (0..ix.syms.len()).filter(|&i| m.newtext[i] != ix.syms[i].text && !repair.visibility_only.contains(&ix.syms[i].id)).count();
    if edited > 0 {
        out += &format!("Rewrote references in {edited} symbol{}\n", if edited == 1 { "" } else { "s" });
    }
    if !retired.is_empty() {
        out += &format!("Retired module {}\n", retired.join(", "));
    }
    trim_output(&out, budget)
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

fn needs_super(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], roots: &[usize], target: &str) -> bool {
    for &root in roots {
        for at in rust_descendants(ix, root) {
            if matches!(ix.syms[at].kind_text.as_str(), "use" | "mod" | "impl" | "trait") {
                continue;
            }
            for used in crate::index::uses_split(ix, at).0 {
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
    newtext: &mut [String],
) -> Result<(), Failure> {
    for &container in whole {
        if ix.syms[container].module == target || needs_super(ix, ids, moved, &[container], target) {
            continue;
        }
        let text = without_super_glob(&newtext[container]);
        newtext[container] = text;
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

pub(crate) fn staying_in<'a>(ix: &'a Index, moved: &'a [bool], newmod: &'a [String], module: &'a str) -> impl Iterator<Item = usize> + 'a {
    ix.syms
        .iter()
        .enumerate()
        .filter(move |(i, symbol)| {
            symbol.module == module && !matches!(symbol.kind_text.as_str(), "use" | "mod") && (!moved[*i] || newmod[*i] == module)
        })
        .map(|(i, _)| i)
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

/// The names the destination module binds after the move.
fn target_names_after_move(ix: &Index, moved: &[bool], newmod: &[String], header: &str, module: &str) -> Result<HashSet<String>, Failure> {
    target_names(ix, header, module, |i, _| in_module_after_move(ix, moved, newmod, i, module))
}

/// The staying tests that reach moved code: a count, and the ones the move rewrote.
fn staying_tests_reply(m: &RustMove<'_>, ids: &HashMap<&str, usize>, moved: &[bool]) -> String {
    let ix = m.ix;
    let test_file = format!("{}::tests", m.target);
    let (mut staying, mut rewritten) = (0, Vec::new());
    let mut rewritten_modules: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut container_imports: HashMap<usize, BTreeSet<String>> = HashMap::new();
    for t in (0..ix.syms.len()).filter(|&t| !moved[t] && reach::is_test_fn(ix, t)) {
        let module = &ix.syms[t].module;
        if module == &m.target || module == &test_file {
            continue;
        }
        let (outside, locals) = test_covered(ix, ids, t);
        if !outside.iter().any(|&u| moved[u]) {
            continue;
        }
        staying += 1;
        let changed = rewritten_modules.entry(module.clone()).or_insert_with_key(|module| module_imports_rewritten(m, module));
        let module_changed = test_uses_changed_import(m, t, &locals, changed);
        let container_changed = test_container(ix, ids, t).is_some_and(|c| {
            let changed = container_imports.entry(c).or_insert_with(|| changed_imports(&ix.syms[c].text, &m.newtext[c]));
            test_uses_changed_import(m, t, &locals, changed)
        });
        if m.newtext[t] != ix.syms[t].text || module_changed || container_changed {
            rewritten.push(ix.syms[t].qname());
        }
    }
    if staying == 0 {
        return String::new();
    }
    let names = if rewritten.is_empty() { String::new() } else { format!(" (rewritten: {})", rewritten_names(&rewritten)) };
    format!("Tests staying: {staying}{names}\n")
}

/// One moved item's mentions in one module, which share a spelling.
pub(crate) struct MovedMentions {
    test_only: bool,
    sites: Vec<(usize, Range<usize>, bool)>,
}

/// Whether symbol `i` is in `module` after the move.
fn in_module_after_move(ix: &Index, moved: &[bool], newmod: &[String], i: usize, module: &str) -> bool {
    if moved[i] { newmod[i] == module } else { ix.syms[i].module == module }
}

/// The test module a target's tests live in: the target itself when it is
/// already a test module, else `<target>::tests`.
fn test_module_path(ix: &Index, target: &str) -> String {
    if ix.test_modules.contains(target) { target.to_string() } else { format!("{target}::tests") }
}

/// The module's `use` symbols whose index `belongs` says are in it.
fn module_use_symbols<'a>(ix: &'a Index, belongs: impl Fn(usize, &Sym) -> bool) -> impl Iterator<Item = &'a Sym> {
    ix.syms
        .iter()
        .enumerate()
        .filter(move |(i, symbol)| symbol.parent.is_none() && symbol.kind_text == "use" && belongs(*i, symbol))
        .map(|(_, symbol)| symbol)
}

/// The surviving modules whose code still names a moved item.
fn caller_modules(ix: &Index, refs: &language::rust_refs::References<'_>, moved: &[bool]) -> Result<BTreeSet<String>, Failure> {
    let staying: HashSet<&str> = ix.syms.iter().enumerate().filter(|(i, _)| !moved[*i]).map(|(_, s)| s.module.as_str()).collect();
    let mut callers = BTreeSet::new();
    for (target, is_moved) in moved.iter().enumerate() {
        if !is_moved {
            continue;
        }
        for &user in &ix.used_by[target] {
            if !moved[user] {
                callers.insert(ix.syms[user].module.clone());
            }
        }
        for site in language::rust_refs::rewrite_sites_refs(refs, target) {
            if let Some(user) = site.symbol
                && !moved[user]
            {
                callers.insert(ix.syms[user].module.clone());
            }
        }
    }
    for (module, site) in all_use_sites(ix)? {
        let use_moved = site.symbol.is_some_and(|i| moved[i]);
        if moved[site.target] && !use_moved && staying.contains(module.as_str()) {
            callers.insert(module);
        }
    }
    Ok(callers)
}

fn target_prefixes(target: &str) -> Vec<String> {
    let mut out = vec![];
    let mut prefix = String::new();
    for part in target.split("::") {
        if !prefix.is_empty() {
            prefix.push_str("::");
        }
        prefix.push_str(part);
        out.push(prefix.clone());
    }
    out
}

/// Raises each `mod` declaration on the way to `target` so callers reach the
/// moved items, or makes every one plain `pub` when a crate-root `pub` item
/// moved, and returns each raised module with its new visibility.
fn raise_target_modules(
    ix: &Index,
    refs: &language::rust_refs::References<'_>,
    target: &str,
    create: Option<&crate::rust_submit::RustNew>,
    moved: &[bool],
    newtext: &mut [String],
    newheaders: &mut HashMap<String, String>,
) -> Result<Vec<(String, &'static str)>, Failure> {
    let callers = caller_modules(ix, refs, moved)?;
    let public_root = moves_public_root_item(ix, moved);
    if target == "crate" || (!public_root && callers.is_empty()) {
        return Ok(vec![]);
    }
    let prefixes = target_prefixes(target);
    let mut raised = vec![];
    for (i, module) in prefixes.iter().enumerate() {
        let parent = if i == 0 { "crate".to_string() } else { prefixes[i - 1].clone() };
        let name = leaf(module);
        let created = create.is_some_and(|c| c.module == *module);
        let file_module = ix.modules.iter().any(|m| m.path == *module && m.file.is_some());
        if created || file_module {
            let Some(header) = newheaders.get(&parent).cloned() else { continue };
            if let Some((header, visibility)) = raise_module_declaration(&header, name, &parent, &callers, public_root) {
                newheaders.insert(parent, header);
                raised.push((module.clone(), visibility));
            }
            continue;
        }
        if let Some(symbol) = ix.syms.iter().position(|s| s.kind_text == "mod" && s.module == parent && s.name == name)
            && let Some((text, visibility)) = raise_module_declaration(&newtext[symbol], name, &parent, &callers, public_root)
        {
            newtext[symbol] = text;
            raised.push((module.clone(), visibility));
        }
    }
    Ok(raised)
}

fn is_prelude_path_name(name: &str) -> bool {
    is_rust_keyword(name) || is_primitive_type(name) || is_std_prelude_item(name) || is_std_crate_root(name)
}

/// A plain `pub` item at the crate root reached outside the crate. Every
/// module on the way to its new home must be public to keep that reach.
fn moves_public_root_item(ix: &Index, moved: &[bool]) -> bool {
    (0..ix.syms.len()).any(|i| moved[i] && ix.syms[i].module == "crate" && visibility::is_plain_public(&ix.syms[i].text))
}

fn raise_module_declaration(
    text: &str,
    name: &str,
    parent: &str,
    callers: &BTreeSet<String>,
    public_root: bool,
) -> Option<(String, &'static str)> {
    if public_root {
        visibility::make_module_public(text, name).map(|text| (text, "pub"))
    } else {
        visibility::raise_module(text, name, parent, callers)
    }
}

fn raised_modules_reply(raised: &[(String, &'static str)]) -> String {
    raised.iter().map(|(module, visibility)| format!("Now {visibility}: mod {module}\n")).collect()
}

/// Whether a test or a helper it reaches names a binding whose import changed.
fn test_uses_changed_import(m: &RustMove<'_>, t: usize, locals: &HashSet<usize>, changed: &BTreeSet<String>) -> bool {
    if changed.contains("*") {
        return true;
    }
    std::iter::once(t).chain(locals.iter().copied()).any(|i| m.refs.mentioned[i].iter().any(|name| changed.contains(name)))
}

/// The reply names at most ten rewritten tests, then counts the rest.
fn rewritten_names(names: &[String]) -> String {
    const SHOWN: usize = 10;
    let mut shown: Vec<String> = names.iter().take(SHOWN).cloned().collect();
    let more = names.len().saturating_sub(SHOWN);
    if more > 0 {
        shown.push(format!("+{more} more"));
    }
    shown.join(", ")
}
