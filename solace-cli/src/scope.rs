//! Tasks across projects (D-61). A task's scope is the projects it may
//! change, named by folder as the root names them; the task is stored in one
//! of them, its home. Each project keeps its own store, so a change is
//! recorded where it was made, under the task's id.

use super::*;
use serde_json::Value;

/// The Solace root that holds a project: the project itself when none does.
pub fn top(project: &Path) -> PathBuf {
    workspace::enclosing(project).ok().flatten().unwrap_or_else(|| project.to_path_buf())
}

/// A project's folder as its root names it.
pub fn dir(top: &Path, project: &Path) -> String {
    match project.strip_prefix(top) {
        Ok(p) if !p.as_os_str().is_empty() => p.to_string_lossy().into(),
        _ => ".".into(),
    }
}

pub fn path(top: &Path, dir: &str) -> PathBuf {
    if dir == "." { top.into() } else { top.join(dir) }
}

/// The projects a task may change; a task saved before D-61 changes only its home.
pub fn of(t: &Value, home: &str) -> Vec<String> {
    match t["scope"].as_array() {
        Some(a) if !a.is_empty() => a.iter().filter_map(|d| d.as_str().map(String::from)).collect(),
        _ => vec![home.to_string()],
    }
}

/// Locks several projects in folder order, so two holders never wait on each other.
pub fn lock_all(roots: &[PathBuf]) -> Result<Vec<fs::File>, Failure> {
    let mut roots = roots.to_vec();
    roots.sort();
    roots.dedup();
    roots.iter().map(|r| board::lock(r)).collect()
}

/// Every task on the board whose scope includes the project `dir`, with its home.
pub fn tasks_in(top: &Path, dir: &str) -> Result<Vec<(PathBuf, Value)>, Failure> {
    let mut out = vec![];
    for home in workspace::members(top)? {
        let root = path(top, &home);
        if !root.join(".solace/board").exists() {
            continue;
        }
        for t in board::tasks(&root)? {
            if of(&t, &home).iter().any(|d| d == dir) {
                out.push((root.clone(), t));
            }
        }
    }
    Ok(out)
}

/// A task's home, found from any project in its scope.
pub fn home(project: &Path, tid: &str) -> Result<PathBuf, Failure> {
    if board::task(project, tid).is_ok() {
        return Ok(project.into());
    }
    let top = top(project);
    tasks_in(&top, &dir(&top, project))?
        .into_iter()
        .find(|(_, t)| t["id"] == tid)
        .map(|(home, _)| home)
        .ok_or_else(|| Failure("E7001 unknown task".into(), 1))
}
