//! File primitives: writing a file whole, and holding a lock on one.

use crate::Failure;
use std::fs;
use std::io::Write;
use std::path::Path;

#[cfg(test)]
#[path = "fsx_tests.rs"]
mod tests;

/// Writes a file whole or not at all: a reader never sees half of it.
pub(crate) fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), Failure> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Holds the lock on the file at `path`, created if missing; waits for it.
pub(crate) fn lock_file(path: &Path) -> Result<fs::File, Failure> {
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    f.lock()?;
    Ok(f)
}

/// The lock on the file at `path`, or `None` while another holds it.
/// `flock` is interruptible: a signal ending a child process interrupts the
/// attempt, which is not a holder, so it is tried again. Reading it as one
/// makes `serving` report a server that isn't there.
pub(crate) fn try_lock_file(path: &Path) -> Result<Option<fs::File>, Failure> {
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(Some(f)),
            Err(fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Ok(None),
        }
    }
}

/// Writes `<dir>/<name>` as `bytes`, synced to disk and renamed into place, so
/// a power loss never leaves an empty file.
pub(crate) fn replace_bytes(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), Failure> {
    fs::create_dir_all(dir)?;
    let (part, to) = (dir.join(format!("{name}.tmp")), dir.join(name));
    let mut f = fs::File::create(&part)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&part, &to)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}
