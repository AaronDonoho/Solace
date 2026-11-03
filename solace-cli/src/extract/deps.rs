//! Project dependencies: the path dependencies a move reads, the projects that depend on it, and the manifest lines a move adds.

use super::*;
use crate::ModuleRow;
use crate::Store;
use crate::Index;
use crate::extract::ffi::relative;
use crate::extract::plan::Plan;
/// The projects a `gleam.toml` depends on by path.
pub(crate) fn path_deps(dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(dir.join("gleam.toml")) else { return vec![] };
    let Ok(v) = text.parse::<toml::Value>() else { return vec![] };
    ["dependencies", "dev-dependencies", "dev_dependencies"]
        .iter()
        .filter_map(|t| v.get(t)?.as_table())
        .flat_map(|t| t.values())
        .filter_map(|s| s.get("path")?.as_str())
        .filter_map(|p| dir.join(p).canonicalize().ok())
        .collect()
}

/// Whether the project in `from` depends on the one in `to` by path, directly or transitively.
pub(crate) fn depends_on(from: &Path, to: &Path) -> bool {
    let Ok(to) = to.canonicalize() else { return false };
    let (mut seen, mut stack) = (HashSet::new(), vec![from.canonicalize().unwrap_or_else(|_| from.to_path_buf())]);
    while let Some(p) = stack.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        for d in path_deps(&p) {
            if d == to {
                return true;
            }
            stack.push(d);
        }
    }
    false
}

/// Whether the command holds the store of the project in `root`: a dependent
/// the task covers, opened by `run_options` (D-103).
pub(crate) fn covered(root: &Path) -> bool {
    crate::COMMAND_DB.with_borrow(|c| c.iter().any(|(r, _)| crate::project_paths::same_project(r, root)))
}

/// The member folders linked to any of `dirs` through path dependencies,
/// either way and transitively, `dirs` among them.
pub(crate) fn package_graph(top: &Path, members: &[String], dirs: &[&str]) -> Vec<String> {
    let canon: Vec<Option<PathBuf>> = members.iter().map(|d| scope::path(top, d).canonicalize().ok()).collect();
    let deps: Vec<Vec<PathBuf>> = members.iter().map(|d| path_deps(&scope::path(top, d))).collect();
    let on = |a: usize, b: usize| canon[b].as_ref().is_some_and(|p| deps[a].contains(p));
    let mut seen: Vec<usize> = (0..members.len()).filter(|&i| dirs.contains(&members[i].as_str())).collect();
    let mut k = 0;
    while k < seen.len() {
        let a = seen[k];
        for b in 0..members.len() {
            if !seen.contains(&b) && (on(a, b) || on(b, a)) {
                seen.push(b);
            }
        }
        k += 1;
    }
    seen.into_iter().map(|i| members[i].clone()).collect()
}

/// A gleam.toml's `text` with `line` added at the end of `[dependencies]`;
/// the table is made when there is none.
pub(crate) fn add_dependency(text: &str, line: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines.iter().position(|l| l.trim() == "[dependencies]") else {
        return format!("{}\n\n[dependencies]\n{line}\n", text.trim_end());
    };
    let mut end = lines[at + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |k| at + 1 + k);
    while end > at + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut out: Vec<&str> = lines[..end].to_vec();
    out.push(line);
    out.extend(&lines[end..]);
    out.join("\n") + "\n"
}

/// The project `extract --to-project` moves symbols into (D-103).
pub(crate) struct Dest {
    /// Its folder, as the root names it.
    pub(crate) dir: String,
    pub(crate) store: Store,
    pub(crate) ix: Index,
}

impl Dest {
    pub(crate) fn has(&self, module: &str) -> bool {
        self.ix.modules.iter().any(|m| m.path == module)
    }
}

/// A project that depends on the source and that the task covers: its uses of
/// moved symbols are rewritten (D-103).
pub(crate) struct Dependent {
    /// Its folder, as the root names it.
    pub(crate) dir: String,
    pub(crate) store: Store,
    pub(crate) ix: Index,
}

/// `dep`'s symbols and modules, then the source's: one index in which the
/// rewrite resolves the dependent's references.
pub(crate) fn merged(dep: &Index, source: &Index) -> Index {
    let syms = dep.syms.iter().chain(&source.syms).cloned().collect();
    let modules = dep
        .modules
        .iter()
        .chain(&source.modules)
        .map(|m| ModuleRow {
            path: m.path.clone(),
            dir: m.dir.clone(),
            header: m.header.clone(),
            file: m.file.clone(),
            owns_dir: m.owns_dir,
        })
        .collect();
    crate::index::build(source.lang, syms, modules, false)
}

pub(crate) fn dependency_lines(
    s: &Store,
    dest: &Option<Dest>,
    deps: &[Dependent],
    dplans: &[Plan<'_>],
    source_needs: bool,
    mine: &str,
    to: &str,
    test_to: &str,
) -> (Option<(String, String)>, Vec<Option<String>>) {
    let dep = match dest {
        Some(d) if source_needs && !path_deps(&s.root).iter().any(|p| d.store.root.canonicalize().ok().as_ref() == Some(p)) => {
            let name = workspace::name(&d.store.root);
            Some((name.clone(), format!("{name} = {{ path = \"{}\" }}", relative(mine, &d.dir))))
        }
        _ => None,
    };
    let dep_lines = deps
        .iter()
        .zip(dplans)
        .map(|(dp, p)| {
            let d = dest.as_ref()?;
            let imports = p.needs.values().flatten().any(|nd| nd.module == to || nd.module == test_to);
            let has = d.store.root.canonicalize().ok().is_some_and(|c| path_deps(&dp.store.root).contains(&c));
            (imports && !has).then(|| format!("{} = {{ path = \"{}\" }}", workspace::name(&d.store.root), relative(&dp.dir, &d.dir)))
        })
        .collect();
    (dep, dep_lines)
}

/// The task's `projects` whose gleam.toml depends on the project in `source`
/// by path, as canonical folders: an extract into another project rewrites them (D-103).
pub(crate) fn dependents(top: &Path, source: &Path, projects: &[String]) -> Vec<PathBuf> {
    let Ok(me) = source.canonicalize() else { return vec![] };
    projects.iter().filter_map(|d| scope::path(top, d).canonicalize().ok()).filter(|p| *p != me && path_deps(p).contains(&me)).collect()
}

/// Every workspace project that depends on the project in `source` by path,
/// directly or through another project.
pub(crate) fn all_dependents(top: &Path, source: &Path, projects: &[String]) -> Vec<PathBuf> {
    let Ok(me) = source.canonicalize() else { return vec![] };
    projects.iter().filter_map(|d| scope::path(top, d).canonicalize().ok()).filter(|p| *p != me && depends_on(p, &me)).collect()
}

/// An index with the project's path dependencies first and its own symbols
/// last, so a test can reach code in every project it depends on.
pub(crate) struct ProjectReach {
    pub(crate) index: Index,
    pub(crate) own_start: usize,
    pub(crate) roots: Vec<PathBuf>,
}

pub(crate) fn project_reach(root: &Path, own: &Index) -> Result<Option<ProjectReach>, Failure> {
    if own.rust {
        return Ok(None);
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let (mut seen, mut stack) = (HashSet::from([root.clone()]), path_deps(&root));
    let mut deps = vec![];
    while let Some(dep) = stack.pop() {
        if seen.insert(dep.clone()) {
            stack.extend(path_deps(&dep));
            deps.push(dep);
        }
    }
    if deps.is_empty() {
        return Ok(None);
    }
    deps.sort();
    let (mut syms, mut modules, mut roots) = (vec![], vec![], vec![]);
    for dep in &deps {
        let Ok(ix) = crate::Store::peek(dep).and_then(|store| store.index()) else { continue };
        syms.extend(ix.syms);
        modules.extend(ix.modules);
        roots.push(dep.clone());
    }
    if roots.is_empty() {
        return Ok(None);
    }
    let own_start = syms.len();
    syms.extend(own.syms.iter().cloned());
    modules.extend(own.modules.iter().map(|m| ModuleRow {
        path: m.path.clone(),
        dir: m.dir.clone(),
        header: m.header.clone(),
        file: m.file.clone(),
        owns_dir: m.owns_dir,
    }));
    let index = crate::index::build(own.lang, syms, modules, false);
    roots.push(root);
    Ok(Some(ProjectReach { index, own_start, roots }))
}
