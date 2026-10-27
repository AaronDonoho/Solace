//! Project paths: how `--project` names a folder, and whether two folders are
//! the same project.

use std::fs;
use std::path::Path;

#[cfg(test)]
#[path = "project_paths_tests.rs"]
mod tests;

/// A project folder as `--project` takes it.
fn project_dir(dir: &str) -> &str {
    match dir.trim_end_matches('/') {
        "" => ".",
        d => d.strip_prefix("./").unwrap_or(d),
    }
}

/// The folder `--project` names: `../x` or `./x` from the current project
/// `here` when that is one of `members`, else the folder from the root.
pub(crate) fn project_folder(top: &Path, here: Option<&Path>, dir: &str, members: &[String]) -> String {
    if let Some(here) = here
        && (dir.starts_with("./") || dir.starts_with("../"))
    {
        let mut parts: Vec<String> = match crate::scope::dir(top, here).as_str() {
            "." => vec![],
            d => d.split('/').map(String::from).collect(),
        };
        let mut inside = true;
        for c in dir.split('/') {
            match c {
                "" | "." => {}
                ".." => inside &= parts.pop().is_some(),
                c => parts.push(c.to_string()),
            }
        }
        let found = if parts.is_empty() { ".".to_string() } else { parts.join("/") };
        if inside && members.contains(&found) {
            return found;
        }
    }
    project_dir(dir).to_string()
}

/// Whether two folders are the same project.
pub fn same_project(a: &Path, b: &Path) -> bool {
    a == b || fs::canonicalize(a).ok().is_some_and(|x| fs::canonicalize(b).ok() == Some(x))
}
