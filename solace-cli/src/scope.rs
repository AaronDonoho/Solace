//! Tasks across projects (D-61). A task's scope is the projects it may
//! change, named by folder as the root names them; the task is stored in the
//! root's task store and its agent works from one of them, its home (D-77).
//! Each project keeps its own store, so a change is recorded where it was
//! made, under the task's id.

use super::*;

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
    for t in board::all_tasks(top)? {
        let home = t["home"].as_str().unwrap_or(".").to_string();
        if of(&t, &home).iter().any(|d| d == dir) {
            out.push((path(top, &home), t));
        }
    }
    Ok(out)
}

/// A task's home, found from any project in its scope.
pub fn home(project: &Path, tid: &str) -> Result<PathBuf, Failure> {
    let top = top(project);
    tasks_in(&top, &dir(&top, project))?
        .into_iter()
        .find(|(_, t)| t["id"] == tid)
        .map(|(home, _)| home)
        .ok_or_else(|| Failure("E7001 unknown task".into(), 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root listing project `sub`: t1 at home in the root and scoped to both,
    /// t2 at home in `sub`, t3 in the root only.
    fn two_projects() -> testing::Scratch {
        let root = testing::Scratch::new().with(&[
            ("solace.toml", "projects = [\"sub\"]\n"),
            (".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#),
            (".solace/board/t3.json", r#"{"id":"t3","column":"backlog"}"#),
            ("sub/.solace/board/t2.json", r#"{"id":"t2","column":"backlog"}"#),
        ]);
        board::adopt_tasks(&root, &root).unwrap();
        board::adopt_tasks(&root, &root.join("sub")).unwrap();
        tasks::update(&root, "t1", |t| {
            t["scope"] = json!([".", "sub"]);
            Ok(())
        })
        .unwrap();
        root
    }

    #[test]
    fn dir_and_path_name_a_project_from_its_root() {
        let top = Path::new("/r");
        assert_eq!(dir(top, Path::new("/r")), ".");
        assert_eq!(dir(top, Path::new("/r/a/b")), "a/b");
        assert_eq!(dir(top, Path::new("/elsewhere")), ".", "a folder outside the root is the root");
        assert_eq!(path(top, "."), PathBuf::from("/r"));
        assert_eq!(path(top, "a/b"), PathBuf::from("/r/a/b"));
        for d in [".", "a", "a/b"] {
            assert_eq!(dir(top, &path(top, d)), d);
        }
    }

    /// A task saved before D-61 (no scope, or an empty one) changes only its home.
    #[test]
    fn of_falls_back_to_the_home() {
        for t in [json!({}), json!({"scope": null}), json!({"scope": []}), json!({"scope": "a"})] {
            assert_eq!(of(&t, "h"), ["h"], "{t}");
        }
        assert_eq!(of(&json!({"scope": ["a", 1, "b"]}), "h"), ["a", "b"]);
    }

    /// Each project is locked once, however often it is named.
    #[test]
    fn lock_all_locks_each_project_once() {
        let root = testing::Scratch::new();
        let (a, b) = (root.join("a"), root.join("b"));
        let held = lock_all(&[b.clone(), a.clone(), b.clone()]).unwrap();
        assert_eq!(held.len(), 2);
        assert!(try_lock_file(&a.join(".solace/lock")).unwrap().is_none());
        drop(held);
        assert!(try_lock_file(&a.join(".solace/lock")).unwrap().is_some());
    }

    #[test]
    fn tasks_in_finds_a_task_from_every_project_of_its_scope() {
        let root = two_projects();
        let ids = |d: &str| {
            let mut ids: Vec<String> = tasks_in(&root, d).unwrap().iter().map(|(_, t)| t["id"].as_str().unwrap().to_string()).collect();
            ids.sort();
            ids
        };
        assert_eq!(ids("."), ["t1", "t3"]);
        assert_eq!(ids("sub"), ["t1", "t2"]);
        assert!(ids("other").is_empty());
        let sub = root.join("sub");
        assert_eq!(home(&sub, "t1").unwrap(), root.to_path_buf(), "found from a project it is not at home in");
        assert_eq!(home(&sub, "t2").unwrap(), sub);
        assert!(home(&sub, "t3").unwrap_err().0.starts_with("E7001"), "out of sub's scope");
        assert!(home(&root, "nope").unwrap_err().0.starts_with("E7001"));
    }
}
