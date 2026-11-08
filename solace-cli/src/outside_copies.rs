//! Repair copies under `.solace/outside`: a changed file's bytes before Solace
//! overwrites it, and cleanup of copies from the old id-keyed layout.

use std::fs;
use std::path::Path;

use crate::failure::Failure;
#[cfg(test)]
#[path = "outside_copies_tests.rs"]
mod tests;



/// Keeps a changed file's bytes under `.solace/outside/<path>`, replacing any
/// earlier copy, so a repair that writes over it loses nothing.
pub fn keep(root: &Path, rel: &str, bytes: &[u8]) -> Result<String, Failure> {
    let kept = format!(".solace/outside/{rel}");
    let path = root.join(&kept);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, bytes)?;
    Ok(kept)
}

/// Removes copies kept under the old `.solace/outside/<id>/` layout; a folder
/// that starts a stored source path is a current copy and stays.
pub(crate) fn remove_old_copies(root: &Path, files: &[String]) -> Result<(), Failure> {
    let Ok(entries) = fs::read_dir(root.join(".solace/outside")) else { return Ok(()) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let old_id = name.len() == 16 && name.bytes().all(|b| b.is_ascii_hexdigit());
        if !old_id || !entry.path().is_dir() {
            continue;
        }
        let source_folder = files.iter().any(|f| f.split('/').next() == Some(name.as_str()));
        if !source_folder {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

pub(crate) const OLD_COPIES_REMOVED: &str = "# old-copies-removed";
