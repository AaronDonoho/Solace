//! Store sharing and ignore handling: the root ignore block that keeps every
//! project's store committed, the generated store ignores it replaces, and the
//! note that an ignore file still hides the store.
use std::path::Path;
use std::fs;
use crate::failure::Failure;
#[cfg(test)]
#[path = "outside_share_tests.rs"]
mod tests;
use crate::scope;
use crate::fsx;
/// Every project's symbol store is committed with the Solace root, but what is
/// local to each checkout is not (D-179): locks, logs, caches, undo records and
/// board data stay ignored, while each `store.db` and `approved` is shared.
pub fn share_store(project: &Path) -> Result<(), Failure> {
    if !project.join(".solace").is_dir() {
        return Ok(());
    }
    share_root_ignore(&scope::top(project))?;
    remove_generated_store_ignore(project)
}

/// What the root's block shares from every project (D-179, D-191).
pub(crate) const STORE_IGNORE: &str = concat!(
    "# Solace: shared store files begin; setup replaces the lines below (D-179).\n",
    "**/.solace/*\n",
    "!**/.solace/store.db\n",
    "!**/.solace/approved\n",
    "# Solace: shared store files end.\n"
);

/// The per-project files earlier setup wrote, and migration copies still do.
pub(crate) const OLD_STORE_IGNORES: [&str; 4] = [
    "# Only the approved list is shared (D-58).\n*\n!.gitignore\n!approved\n",
    "# Solace's store is committed with the project.\n",
    "*\n",
    concat!(
        "# Solace's symbol store is committed; local caches, locks, logs and board data are not (D-179).\n",
        "*\n!.gitignore\n!approved\n!store.db\n"
    ),
];

/// Whether the ignore text still hides `store.db` for `here`, the project's
/// folder from the root: an older variant `share_store` doesn't know, or a
/// human's edit. Such files are left alone (D-179), but the caller says so,
/// since a commit would leave the store out.
fn ignores_store(text: &str, here: &str) -> bool {
    let here = here.replace('\\', "/");
    let store = if here == "." { ".solace".to_string() } else { format!("{here}/.solace") };
    let file = format!("{store}/store.db");
    let mut dirs: Vec<(String, bool)> = store_dirs(&store).into_iter().map(|dir| (dir, false)).collect();
    let mut ignored = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, pattern) = match line.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, line),
        };
        let anchored = pattern.starts_with('/');
        let p = pattern.trim_start_matches('/');
        let dir_pattern = p.trim_end_matches('/');
        for (dir, hidden) in &mut dirs {
            if hides_path(dir_pattern, anchored, dir) {
                *hidden = !negated;
            }
        }
        if !p.ends_with('/') && hides_path(p, anchored, &file) {
            ignored = !negated;
        }
    }
    ignored || dirs.into_iter().any(|(_, hidden)| hidden)
}

/// The note that the ignore files still hide the store, said once for each
/// version of them. Clearing the state when it stops hiding means a later
/// regression is reported again.
pub(crate) fn hidden_note(hidden: Option<&str>, root: &Path) -> HiddenNote {
    let text = ignore_text(root);
    let hiding = ignores_store(&text, &scope::dir(&scope::top(root), root));
    let stamp = (!text.is_empty()).then(|| crate::digest(text.as_bytes()));
    if !hiding {
        return HiddenNote { text: String::new(), hidden: None, dirty: hidden.is_some() };
    }
    if hidden == stamp.as_deref() {
        return HiddenNote { text: String::new(), hidden: stamp, dirty: false };
    }
    HiddenNote {
        text: "Note: an ignore file still ignores store.db; remove that pattern so the symbol store is committed.\n".to_string(),
        hidden: stamp,
        dirty: true,
    }
}

/// The markers around the block Solace keeps current in the root's `.gitignore`.
pub(crate) const BLOCK_START: &str = "# Solace: shared store files begin; setup replaces the lines below (D-179).\n";

pub(crate) const BLOCK_END: &str = "# Solace: shared store files end.\n";

/// Keeps one marked block in the root's `.gitignore`, leaving the rest alone.
fn share_root_ignore(root: &Path) -> Result<(), Failure> {
    let path = root.join(".gitignore");
    let current = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let wanted = current.as_deref().map_or_else(|| STORE_IGNORE.to_string(), with_store_block);
    if current.as_deref() != Some(wanted.as_str()) {
        fsx::write_atomic(&path, wanted)?;
    }
    Ok(())
}

fn with_store_block(text: &str) -> String {
    let Some(start) = text.rfind(BLOCK_START) else {
        return append_store_block(text);
    };
    let Some(end) = text[start..].find(BLOCK_END).map(|i| start + i + BLOCK_END.len()) else {
        return append_store_block(text);
    };
    format!("{}{STORE_IGNORE}{}", &text[..start], &text[end..])
}

fn append_store_block(text: &str) -> String {
    let newline = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    format!("{text}{newline}{STORE_IGNORE}")
}

/// Removes only a per-project file some Solace version generated; a human's
/// file stays where it is and still applies below the root's block.
fn remove_generated_store_ignore(project: &Path) -> Result<(), Failure> {
    let path = project.join(".solace/.gitignore");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(());
    };
    if OLD_STORE_IGNORES.contains(&text) {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// The root's ignore lines, then the store's own, as one heuristic list. They
/// are relative to different folders, so a leading `/` is only approximate.
fn ignore_text(project: &Path) -> String {
    let mut text = fs::read_to_string(scope::top(project).join(".gitignore")).unwrap_or_default();
    let store = fs::read_to_string(project.join(".solace/.gitignore")).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&store);
    text
}

/// Whether a `.gitignore` pattern covers `path`. A pattern with no slash
/// matches at any depth, as in gitignore, unless it is anchored to the root.
fn hides_path(pattern: &str, anchored: bool, path: &str) -> bool {
    let pattern = if anchored || pattern.contains('/') { pattern.to_string() } else { format!("**/{pattern}") };
    globset::GlobBuilder::new(&pattern).literal_separator(true).build().is_ok_and(|g| g.compile_matcher().is_match(path))
}

/// `store` and its ancestor folders, as paths from the root.
fn store_dirs(store: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut dir = String::new();
    for part in store.split('/') {
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(part);
        dirs.push(dir.clone());
    }
    dirs
}

/// What `hidden_note` says, and the state its caller keeps for the next call.
pub(crate) struct HiddenNote {
    pub(crate) text: String,
    pub(crate) hidden: Option<String>,
    pub(crate) dirty: bool,
}
