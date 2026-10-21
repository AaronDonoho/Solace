//! Tool lookup on `PATH`, with a test's fixed folders in place of the process's.

use std::path::PathBuf;

#[cfg(test)]
thread_local! {
    static PATH_OVERRIDE: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// The path of `program` on `PATH`, with Windows' executable endings; tests can stand in fixed folders.
pub(crate) fn search_path(program: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dirs) = PATH_OVERRIDE.with(|p| p.borrow().clone()) {
        return find_in(&dirs, program);
    }
    let path = std::env::var_os("PATH")?;
    find_in(&std::env::split_paths(&path).collect::<Vec<_>>(), program)
}

fn find_in(dirs: &[PathBuf], program: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![]
    };
    for dir in dirs {
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        for ext in &extensions {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Runs `f` with `PATH` read as `dirs`, so a test can act as a machine with or
/// without a tool.
#[cfg(test)]
pub(crate) fn with_path<T>(dirs: &[PathBuf], f: impl FnOnce() -> T) -> T {
    let old = PATH_OVERRIDE.with(|p| p.replace(Some(dirs.to_vec())));
    let out = f();
    PATH_OVERRIDE.with(|p| *p.borrow_mut() = old);
    out
}
