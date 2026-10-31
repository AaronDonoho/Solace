use serde_json::Value;
use serde_json::json;
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::fs;
use std::collections::HashSet;
use crate::Store;
use crate::sha256;
use crate::failure::Failure;
use crate::lockdown::paths::path;
use crate::extract::ahead;
use crate::jsonx;
use crate::fsx;
use crate::board::reviews::kept_on_disk;
use crate::board::reviews::adopt_pending;
use crate::board::runs::context;

/// Puts the project back after a crash (D-77 point 7), at the start of each
/// command and at serve start. It runs only under the project lock (`_held`):
/// a live command holds that lock until its records are dropped, so every
/// record found here is a dead command's. Checking that the run has ended
/// would be wrong, since a run whose command crashed keeps running and its
/// next command must adopt the record. A record with a task and a
/// run is an undo record: if its file changed, the write landed and the rows
/// never committed, so recovery adds them; if the file is as the record says,
/// the write never landed and the record is dropped. A record with no task is
/// the in-place validator journal (Spec 14 §64): the file is restored.
pub fn recover(root: &Path, store: &Store, _held: &[fs::File]) -> Result<(), Failure> {
    let pending = root.join(".solace/pending");
    let Ok(dir) = fs::read_dir(&pending) else { return Ok(()) };
    let mut records = vec![];
    for entry in dir {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json") {
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tasks-")) {
                continue;
            }
            // A record that can't be read was never fully written, so its change
            // never started: drop it rather than fail every later command.
            match fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(record) => records.push((p, record)),
                None => fs::remove_file(&p)?,
            }
        }
    }
    // An extract's record, and every record of its run, wait for `extract::ahead`,
    // which settles all of the extract's projects together.
    let waiting: HashSet<String> =
        records.iter().filter(|(_, r)| ahead::is_record(r)).filter_map(|(_, r)| r["run"].as_str().map(String::from)).collect();
    records.retain(|(_, r)| !r["run"].as_str().is_some_and(|x| waiting.contains(x)));
    if records.iter().any(|(_, r)| r["task"].is_string() && r["run"].is_string()) {
        // A command's outer transaction never committed: the code files go back
        // to the store, which is the source of truth.
        store.materialize(&store.index()?)?;
    }
    for (p, record) in records {
        if record["task"].is_string() && record["run"].is_string() {
            adopt_pending(store, &record)?;
        } else {
            let target = path(root, record["path"].as_str().ok_or_else(|| Failure("invalid pending record".into(), 1))?)?;
            if let Some(a) = record["bytes"].as_array() {
                fs::write(target, jsonx::bytes(a))?;
            } else if target.exists() {
                fs::remove_file(target)?;
            }
        }
        fs::remove_file(p)?;
    }
    Ok(())
}

/// Marks that run `rid` of task `tid` is changing the project, before its outer
/// transaction begins, so a crash before the commit sends recovery to put the
/// code files back from the store (D-77 point 7).
pub fn mark(root: &Path, tid: &str, rid: &str) -> Result<(), Failure> {
    let dir = root.join(".solace/pending");
    fs::create_dir_all(&dir)?;
    fsx::write_atomic(&dir.join(format!("command-{rid}.json")), json!({"task":tid,"run":rid}).to_string().into_bytes())?;
    Ok(())
}

/// Whether the project has an undo record or command marker for task `tid`
/// that no recovery has adopted yet: a run was stopped mid-write, so its change
/// is on disk but not yet in the task's rows.
pub fn pending_for(root: &Path, tid: &str) -> bool {
    let Ok(dir) = fs::read_dir(root.join(".solace/pending")) else { return false };
    dir.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .any(|p| jsonx::read_opt(&p).is_some_and(|v| v["task"].as_str() == Some(tid)))
}

/// Drops the undo records a committed command wrote for run `rid`.
pub fn drop_records(root: &Path, rid: &str) -> Result<(), Failure> {
    let dir = root.join(".solace/pending");
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json")
            && fs::read(&p).is_ok_and(|b| {
                serde_json::from_slice::<Value>(&b).is_ok_and(|v| v["run"].as_str() == Some(rid) && !ahead::is_record(&v))
            })
        {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

/// Whether `journal` already holds an undo record from run `rid`.
pub(crate) fn same_run(journal: &Path, rid: &serde_json::Value) -> bool {
    jsonx::read_opt(journal).is_some_and(|r| &r["run"] == rid)
}

pub(crate) fn session_file(root: &Path) -> PathBuf {
    let session = env::var("SOLACE_SESSION")
        .unwrap_or_else(|_| if env::args().any(|a| a == "mcp") { std::process::id().to_string() } else { "cli".into() });
    root.join(".solace").join(format!("reads-{}.json", sha256(session.as_bytes())))
}

pub(crate) fn reads(root: &Path) -> Value {
    crate::jsonx::read_opt(&session_file(root)).unwrap_or(json!({}))
}

pub(crate) fn remember(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Failure> {
    let mut r = reads(root);
    r[name] = json!(sha256(bytes));
    fs::write(session_file(root), r.to_string())?;
    Ok(())
}

/// Saves `name`'s bytes before a change in `.solace/pending/`. In a board run
/// the record is an undo record: it stays until the command's outer
/// transaction commits, and recovery adds the rows from it (D-77 point 7);
/// otherwise it is the validator journal. Returns the record's path and
/// whether this change wrote it: an earlier change to this file in the
/// command keeps its record, which holds the original bytes.
pub(crate) fn journal(root: &Path, name: &str, before: &Option<Vec<u8>>) -> Result<(PathBuf, bool), Failure> {
    fs::create_dir_all(root.join(".solace/pending"))?;
    let journal = root.join(".solace/pending").join(format!("{}.json", sha256(name.as_bytes())));
    let record = match context() {
        Some((tid, rid)) => {
            let mut r = json!({"task":tid,"run":rid,"path":name,"bytes":before});
            if before.is_none() {
                r["kept"] = kept_on_disk(root, name)["kept"].clone();
            }
            r
        }
        None => json!({"path":name,"bytes":before}),
    };
    let wrote = record["task"].is_null() || !same_run(&journal, &record["run"]);
    if wrote {
        crate::fsx::write_atomic(&journal, record.to_string())?;
    }
    Ok((journal, wrote))
}

/// Ends a file change. A refused one puts the file back, with no undo to
/// record, and drops the record if this change wrote it (an earlier change's
/// record stays: that change still stands). A kept one drops the validator
/// journal (an undo record stays) and remembers the new bytes as read.
pub(crate) fn settle(
    root: &Path,
    name: &str,
    p: &Path,
    before: &Option<Vec<u8>>,
    after: Option<&[u8]>,
    (journal, wrote): &(PathBuf, bool),
    kept: bool,
) -> Result<(), Failure> {
    if !kept {
        match before {
            Some(b) => fs::write(p, b)?,
            None if p.exists() => fs::remove_file(p)?,
            None => {}
        }
        if *wrote {
            fs::remove_file(journal)?;
        }
        return Ok(());
    }
    if context().is_none() {
        fs::remove_file(journal)?;
    }
    if let Some(after) = after {
        remember(root, name, after)?;
    }
    Ok(())
}

/// In a board run, the undo record for `name` before a move changes it, as
/// `remove` writes one. Returns a record this call wrote, so a failed move can drop it.
pub(crate) fn undo_record(root: &Path, name: &str, bytes: Option<Vec<u8>>) -> Result<Option<PathBuf>, Failure> {
    let Some((tid, rid)) = context() else { return Ok(None) };
    let dir = root.join(".solace/pending");
    fs::create_dir_all(&dir)?;
    let record = dir.join(format!("{}.json", sha256(name.as_bytes())));
    // An earlier change to this file in the command keeps its record: it holds the original bytes.
    if same_run(&record, &json!(rid)) {
        return Ok(None);
    }
    let mut r = json!({"task":tid,"run":rid,"path":name,"bytes":bytes});
    if bytes.is_none() {
        r["kept"] = kept_on_disk(root, name)["kept"].clone();
    }
    crate::fsx::write_atomic(&record, r.to_string())?;
    Ok(Some(record))
}

/// This session's moves, by source, so a repeated move finds its own.
pub(crate) fn moves_file(root: &Path) -> PathBuf {
    let reads = session_file(root);
    reads.with_file_name(reads.file_name().unwrap().to_string_lossy().replacen("reads-", "moves-", 1))
}
