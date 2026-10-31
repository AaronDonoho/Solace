//! Test-file placement: the tests that move with Rust code, the test modules they join, and the `#[path]` file they write.

use super::*;
use crate::Index;
use crate::extract::rust::mark;
use crate::Store;
use crate::extract::rust::leaf;
use crate::extract::rust::bring_helpers;
use crate::extract::rust::bring_helpers_with;
use crate::extract::rust::header_mod_names;
use crate::failure::Failure;
use crate::failure::fail;
use crate::rust_submit::rust_descendants;
use crate::extract::rust::rewrite::drop_import_keys;
use crate::extract::rust::needs_super;
use crate::extract::rust::test_module_path;
use crate::extract::rust::plan::RustMove;
#[cfg(test)]
#[path = "test_files_tests.rs"]
mod tests;

pub(super) fn is_test_container(ix: &Index, i: usize) -> bool {
    ix.syms[i].kind_text == "mod" && is_test(ix, i)
}

pub(crate) struct TestGroup {
    pub(crate) children: Vec<usize>,
}

pub(crate) fn whole_test_containers(ix: &Index, moved: &mut [bool]) -> HashSet<usize> {
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

pub(crate) fn test_groups(ix: &Index, ids: &HashMap<&str, usize>, moved: &[bool], whole: &HashSet<usize>, file_tests: bool) -> Vec<TestGroup> {
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

pub(crate) fn test_container(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> Option<usize> {
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

pub(crate) fn test_covered(ix: &Index, ids: &HashMap<&str, usize>, t: usize) -> (HashSet<usize>, HashSet<usize>) {
    let container = test_container(ix, ids, t);
    let local = |j: usize| {
        belongs_in_test_file(ix, ids, j)
            && !crate::reach::is_test_fn(ix, j)
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
        for u in crate::index::uses_split(ix, j).0 {
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

pub(crate) fn move_tests(ix: &Index, ids: &HashMap<&str, usize>, moved: &mut [bool], helpers: &mut Vec<usize>) {
    let tests: Vec<usize> = crate::reach::test_symbols(ix).into_iter().filter(|&t| !moved[t]).collect();
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

fn target_has(ix: &Index, moved: &[bool], target: &str, name: &str) -> bool {
    ix.syms.iter().enumerate().any(|(i, s)| !moved[i] && s.module == target && s.parent.is_none() && s.name == name && s.kind_text == "mod")
}

pub(crate) fn check_test_containers(
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
    let inline = inline_test_container(ix, target).map(|i| ix.syms[i].id.clone());
    for &c in whole {
        if existing.is_some() || inline.is_some() {
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
    if inline.is_none()
        && (target_has(ix, moved, target, "tests")
            || (existing.is_none()
                && (declared.iter().any(|n| n == "tests") || ix.modules.iter().any(|m| m.path == format!("{target}::tests")))))
    {
        return fail(format!("E1004 {target}.tests already exists in {target}"));
    }
    if inline.is_none() && !names.insert("tests".to_string()) {
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
            if let Some(parent) = &inline
                && ix.syms.iter().any(|s| s.parent.as_deref() == Some(parent.as_str()) && leaf(&s.name) == name)
            {
                return fail(format!("E1004 {target}::tests.{name} already exists in {target}::tests"));
            }
        }
    }
    Ok(())
}

pub(crate) fn move_test_groups(
    s: &Store,
    ix: &Index,
    groups: &[TestGroup],
    test: &TestModule,
    newtext: &mut [String],
) -> Result<(), Failure> {
    let module = test.module.as_str();
    let inline = test.inline.as_deref().and_then(|id| ix.syms.iter().position(|sym| sym.id == id));
    let mut ord: i64 = if let Some(parent) = inline {
        let id = ix.syms[parent].id.as_str();
        s.db.query_row("SELECT COALESCE(MAX(ord), 0) FROM symbols WHERE live = 1 AND parent = ?1", [id], |r| r.get(0))?
    } else {
        s.db.query_row("SELECT COALESCE(MAX(ord), 0) FROM symbols WHERE live = 1 AND module = ?1", [module], |r| r.get(0))?
    };
    let moving: HashSet<usize> = groups.iter().flat_map(|g| g.children.iter().copied()).collect();
    let mut inline_parents: HashSet<usize> = HashSet::new();
    let tests_name = inline.map(|i| ix.syms[i].name.clone());
    for g in groups {
        for &i in &g.children {
            ord += 1;
            let old = ix.syms[i].module.clone();
            if let Some(parent) = ix.syms[i].parent.as_deref()
                && let Some(p) = ix.syms.iter().position(|sym| sym.id == parent && sym.kind_text == "mod")
            {
                inline_parents.insert(p);
                if old != module {
                    newtext[i] = dedent(&newtext[i]);
                }
            }
            let prefix = format!("{}::", ix.syms[i].name);
            let leaf_name = leaf(&ix.syms[i].name).to_string();
            if let (Some(parent), Some(tests_name)) = (inline, &tests_name) {
                let name = format!("{tests_name}::{leaf_name}");
                if old != module {
                    newtext[i] = format!("\n\n{}", crate::store::reindent(&newtext[i], "    "));
                }
                s.db.execute(
                    "UPDATE symbols SET module = ?2, parent = ?3, name = ?4, ord = ?5, sep = '' WHERE id = ?1",
                    params![ix.syms[i].id, module, ix.syms[parent].id.as_str(), name, ord],
                )?;
                for j in rust_descendants(ix, i).into_iter().filter(|&j| j != i) {
                    let name =
                        ix.syms[j].name.strip_prefix(&prefix).map_or_else(|| ix.syms[j].name.clone(), |rest| format!("{name}::{rest}"));
                    s.db.execute("UPDATE symbols SET module = ?2, name = ?3 WHERE id = ?1", params![ix.syms[j].id, module, name])?;
                }
            } else {
                s.db.execute(
                    "UPDATE symbols SET module = ?2, parent = NULL, name = ?3, ord = ?4 WHERE id = ?1",
                    params![ix.syms[i].id, module, leaf_name, ord],
                )?;
                for j in rust_descendants(ix, i).into_iter().filter(|&j| j != i) {
                    let name = ix.syms[j]
                        .name
                        .strip_prefix(&prefix)
                        .map_or_else(|| ix.syms[j].name.clone(), |rest| format!("{leaf_name}::{rest}"));
                    s.db.execute("UPDATE symbols SET module = ?2, name = ?3 WHERE id = ?1", params![ix.syms[j].id, module, name])?;
                }
            }
            s.history(&ix.syms[i].id, "move", &old)?;
        }
    }
    for parent in inline_parents {
        restore_inline_tail(s, ix, parent, &moving)?;
    }
    Ok(())
}

pub(crate) fn moved_tests_reply(ix: &Index, moved: &[bool], target: &str) -> String {
    let tests: Vec<usize> = (0..ix.syms.len()).filter(|&i| moved[i] && crate::reach::is_test_fn(ix, i)).collect();
    if tests.is_empty() {
        return String::new();
    }
    format!("Tests moved: {}\n", tests.iter().map(|&i| format!("{} -> {}", ix.syms[i].qname(), target)).collect::<Vec<_>>().join(", "))
}

fn dedent(text: &str) -> String {
    let Some(first) = text.lines().find(|line| !line.trim().is_empty()) else { return text.to_string() };
    let base = &first[..first.len() - first.trim_start().len()];
    if base.is_empty() {
        return text.to_string();
    }
    let literals = crate::language::rust::string_literals(text);
    let mut out = String::new();
    let mut at = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            at += 1;
        }
        if literals.iter().any(|literal| literal.start < at && at < literal.end) {
            out.push_str(line);
        } else {
            out.push_str(line.strip_prefix(base).unwrap_or(line));
        }
        at += line.len();
    }
    out
}

fn restore_inline_tail(s: &Store, ix: &Index, parent: usize, moving: &HashSet<usize>) -> Result<(), Failure> {
    let siblings: Vec<usize> = (0..ix.syms.len()).filter(|&i| ix.syms[i].parent.as_deref() == Some(ix.syms[parent].id.as_str())).collect();
    let Some(&last) = siblings.last() else { return Ok(()) };
    if !moving.contains(&last) {
        return Ok(());
    }
    let Some(&survivor) = siblings.iter().rev().find(|&&i| !moving.contains(&i)) else { return Ok(()) };
    s.db.execute("UPDATE symbols SET sep = ?2 WHERE id = ?1", params![ix.syms[survivor].id, ix.syms[last].sep.clone()])?;
    Ok(())
}

pub(crate) struct TestModule {
    pub(crate) module: String,
    pub(crate) file: String,
    pub(crate) dir: String,
    pub(crate) header: String,
    pub(crate) created: bool,
    pub(crate) inline: Option<String>,
}

pub(crate) fn test_module<'a>(ix: &'a Index, target: &str) -> Option<&'a crate::ModuleRow> {
    if ix.test_modules.contains(target) {
        return ix.modules.iter().find(|m| m.path == target);
    }
    let path = test_module_path(ix, target);
    let row = ix.modules.iter().find(|m| m.path == path && m.file.is_some())?;
    let parent = ix.modules.iter().find(|m| m.path == target)?;
    (crate::rust_submit::declares_path_test_mod(&parent.header, "tests") && crate::rust_submit::declares_module(parent, "tests", row))
        .then_some(row)
}

fn test_file_header(existing: &str, needs_super: bool) -> String {
    if needs_super { with_super(existing) } else { existing.to_string() }
}

fn with_super(header: &str) -> String {
    let body = header.trim_end();
    if header.lines().any(|line| line.trim() == "use super::*;") {
        return format!("{body}\n\n");
    }
    if body.is_empty() { "use super::*;\n\n".to_string() } else { format!("{body}\nuse super::*;\n\n") }
}

pub(crate) fn retire_test_containers(s: &Store, ix: &Index, whole: &HashSet<usize>) -> Result<(), Failure> {
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

pub(crate) fn plan_test_module(
    s: &Store,
    m: &mut RustMove<'_>,
    ids: &HashMap<&str, usize>,
    existing: Option<&crate::ModuleRow>,
    inline: Option<usize>,
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
    if let Some(container) = inline {
        if !inline_has_super(ix, container) {
            let text = &m.newtext[container];
            let head = text.trim_end();
            if head.ends_with('{') {
                m.newtext[container] = format!("{head}\n    use super::*;\n");
            }
        }
        let dir = target_file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
        return Ok(Some(TestModule {
            module: m.target.clone(),
            file: target_file,
            dir,
            header: String::new(),
            created: false,
            inline: Some(ix.syms[container].id.clone()),
        }));
    }
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
                inline: None,
            }))
        }
        None => {
            let file = crate::language::rust::test_file(&m.target, &target_file);
            if s.root.join(&file).exists() {
                return fail(format!("E1001 {file} already exists outside the store; import it before extracting tests"));
            }
            let module = test_module_path(m.ix, &m.target);
            let existing_header = m.newheaders.get(&module).cloned().unwrap_or_default();
            let header = test_file_header(&existing_header, needs);
            let target_header = m.newheaders.get(&m.target).cloned().unwrap_or_default();
            let path = crate::rust_submit::test_mod_path(&file);
            m.newheaders.insert(m.target.clone(), crate::rust_submit::add_test_mod(&target_header, "tests", path));
            m.newheaders.insert(module.clone(), header.clone());
            let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
            Ok(Some(TestModule { module, file, dir, header, created: true, inline: None }))
        }
    }
}

/// Whether a moved item belongs in the target's test file (D-206): a test
/// container, a `#[test]` function, code inside either, or a test module.
pub(crate) fn belongs_in_test_file(ix: &Index, ids: &HashMap<&str, usize>, i: usize) -> bool {
    if is_test_container(ix, i) || crate::reach::is_test_fn(ix, i) {
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

/// Drops the globs this move added to the test module's header that cargo
/// reports unused, leaving the imports a user wrote.
pub(crate) fn drop_unused_carried_globs(
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

/// The target's own inline `mod tests`, where moved tests join.
pub(crate) fn inline_test_container(ix: &Index, target: &str) -> Option<usize> {
    ix.syms.iter().position(|s| s.module == target && s.parent.is_none() && s.name == "tests" && s.kind_text == "mod")
}

fn inline_has_super(ix: &Index, container: usize) -> bool {
    let id = ix.syms[container].id.as_str();
    ix.syms.iter().any(|s| s.parent.as_deref() == Some(id) && s.kind_text == "use" && s.text.trim() == "use super::*;")
}
