//! File primitives: writing a file whole, holding a lock on one, and copying one.

use crate::failure::Failure;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

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

pub(crate) fn try_lock_file_within(path: &Path, deadline: Duration) -> Result<Option<fs::File>, Failure> {
    let started = Instant::now();
    loop {
        if let Some(f) = try_lock_file(path)? {
            return Ok(Some(f));
        }
        let left = deadline.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(None);
        }
        thread::sleep(left.min(Duration::from_millis(50)));
    }
}

pub(crate) fn copy_if_changed(from: &Path, to: &Path) -> Result<(), Failure> {
    if let (Ok(a), Ok(b)) = (fs::metadata(from), fs::metadata(to))
        && a.len() == b.len()
        && let (Ok(am), Ok(bm)) = (a.modified(), b.modified())
        && am == bm
    {
        return Ok(());
    }
    if same_contents(from, to) {
        copy_modified_time(from, to);
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    copy_modified_time(from, to);
    Ok(())
}

pub(crate) fn same_contents(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut a), Ok(mut b)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    let (mut one, mut two) = ([0u8; 8192], [0u8; 8192]);
    loop {
        let n = match std::io::Read::read(&mut a, &mut one) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let m = match std::io::Read::read(&mut b, &mut two) {
            Ok(m) => m,
            Err(_) => return false,
        };
        if n != m || one[..n] != two[..m] {
            return false;
        }
        if n == 0 {
            return true;
        }
    }
}

fn copy_modified_time(from: &Path, to: &Path) {
    let Ok(modified) = fs::metadata(from).and_then(|meta| meta.modified()) else { return };
    let Ok(file) = fs::OpenOptions::new().write(true).open(to) else { return };
    let _ = file.set_modified(modified);
}