//! JSON plumbing: the bytes a JSON file holds, and reading and writing it whole.

use crate::failure::Failure;
use crate::fsx;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[cfg(test)]
#[path = "jsonx_tests.rs"]
mod tests;

/// Bytes stored in JSON as an array of numbers.
pub(crate) fn bytes(a: &[Value]) -> Vec<u8> {
    a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect()
}

/// The JSON at `path`; a read or parse failure is a `Failure` at code 1.
pub(crate) fn read(path: &Path) -> Result<Value, Failure> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| Failure(e.to_string(), 1))
}

/// The JSON at `path`, or `None` when it is missing or invalid.
pub(crate) fn read_opt(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes `v` to `path` whole, without syncing it to disk.
pub(crate) fn write(path: &Path, v: &Value) -> Result<(), Failure> {
    fsx::write_atomic(path, serde_json::to_vec(v).unwrap())
}

/// Writes `v` to `path` as pretty JSON, without syncing it to disk.
pub(crate) fn write_pretty(path: &Path, v: &Value) -> Result<(), Failure> {
    fsx::write_atomic(path, serde_json::to_vec_pretty(v).unwrap())
}

/// Writes `<dir>/<name>` as pretty JSON, synced to disk and renamed into place.
pub(crate) fn replace_pretty(dir: &Path, name: &str, v: &Value) -> Result<(), Failure> {
    fsx::replace_bytes(dir, name, &serde_json::to_vec_pretty(v).unwrap())
}
