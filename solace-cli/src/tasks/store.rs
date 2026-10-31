use std::time::Duration;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::fs;
use rusqlite::Connection;
use rusqlite::params;
use std::path::PathBuf;
use crate::Store;
use crate::now;
use crate::failure::Failure;
use crate::tasks::strategy::plan;
use crate::tasks::strategy::strategy_dir;
use crate::tasks::strategy::rank_of;
use crate::tasks::server::op;
use crate::scope;
use crate::board;
use crate::jsonx;
use crate::fsx;
use std::io::Write;
use crate::workspace;
use crate::board::task_model::valid_id;
use crate::board::task_model::task;
use crate::board::task_model::rehome_to;
use crate::board::task_model::all_tasks;
use crate::board::task_model::read_task;
use crate::board::task_model::flow;
use crate::board::task_model::note;
use crate::board::task_model::plan_for_feature;
use crate::board::task_model::move_task;
use crate::board::task_model::store_dir;

/// Writes task `t` as it is, with the project saving it as its home if it has
/// none yet.
pub fn put(s: &Store, t: &Value) -> Result<(), Failure> {
    let mut t = t.clone();
    if t["home"].is_null() {
        t["home"] = json!(scope::dir(&scope::top(&s.root), &s.root));
    }
    let top = scope::top(&s.root);
    let served = top.clone();
    op(&served, move || write(&top, &t))
}

/// Changes task `tid` of the project `root` with `f`, read and written as one
/// operation.
pub fn update<R: Send + 'static>(
    root: &Path,
    tid: &str,
    f: impl FnOnce(&mut Value) -> Result<R, Failure> + Send + 'static,
) -> Result<R, Failure> {
    let (top, root, tid) = (scope::top(root), root.to_path_buf(), tid.to_string());
    op(&top, move || {
        let mut t = task(&root, &tid)?;
        let out = f(&mut t)?;
        write(&scope::top(&root), &t)?;
        Ok(out)
    })
}

pub(crate) fn write(top: &Path, t: &Value) -> Result<(), Failure> {
    let batch = batch(top)?;
    match write_task(top, t, None) {
        Ok((home, t, changed)) => match finish_write(top, &home, &t, &changed) {
            Ok(()) => batch_commit(batch),
            Err(e) => {
                batch_rollback(batch);
                Err(e)
            }
        },
        Err(e) => {
            batch_rollback(batch);
            Err(e)
        }
    }
}

/// A feature follows its tasks (D-128): an `active` feature whose tasks are all
/// in a final column, its Tech Debt tasks too, becomes `complete`; a `complete`
/// one with a task out of the final columns is `active` again. No other state
/// changes by itself, and a feature with no tasks stays as it is. Task `saving`
/// counts as it is about to be stored. The feature, when it changed, not yet
/// saved: `write` saves it after the task.
fn follow_tasks(top: &Path, fid: &str, saving: &Value) -> Result<Option<Value>, Failure> {
    let Some(mut f) = record(top, "features", fid) else { return Ok(None) };
    let mut held: Vec<Value> = all_tasks(top)?.into_iter().filter(|t| t["feature"] == fid && t["id"] != saving["id"]).collect();
    if saving["feature"] == fid {
        held.push(saving.clone());
    }
    // A task whose home has no workflow (not a project yet) is in no column: open.
    let closed =
        |t: &Value| flow(&scope::path(top, t["home"].as_str().unwrap_or("."))).is_ok_and(|w| crate::board::task_model::column(&w, t)["final"] == true);
    let open = held.iter().any(|t| !closed(t));
    let state = match f["state"].as_str() {
        Some("active") if !held.is_empty() && !open => "complete",
        Some("complete") if open => "active",
        _ => return Ok(None),
    };
    f["state"] = json!(state);
    mark_start(top, "features", &mut f);
    // The task whose save made it active again is planned by `write` (D-134);
    // a complete feature's other tasks are all in a final column.
    Ok(Some(f))
}

/// Every milestone (`kind` "milestones") or feature ("features") of the root, in
/// rank order. A missing folder holds none, and an unreadable file is skipped.
pub fn records(top: &Path, kind: &str) -> Vec<Value> {
    let mut out = stored_records(top, kind);
    if kind == "features" {
        out = with_slots(out);
    }
    out.sort_by(|a, b| rank_of(a).total_cmp(&rank_of(b)).then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    out
}

/// Every record file of `kind`, in no order; an unreadable one is skipped.
pub(crate) fn stored_records(top: &Path, kind: &str) -> Vec<Value> {
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(strategy_dir(top, kind)) else { return out };
    for p in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        match fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
            Ok(v) => out.push(stored(kind, v, &p)),
            Err(e) => eprintln!("skipping record file {}: {e}", p.display()),
        }
    }
    out
}

pub(crate) fn record(top: &Path, kind: &str, id: &str) -> Option<Value> {
    if !valid_id(id) {
        return None;
    }
    if kind == "features" {
        return records(top, kind).into_iter().find(|v| v["id"] == id);
    }
    let path = strategy_dir(top, kind).join(format!("{id}.json"));
    Some(stored(kind, jsonx::read_opt(&path)?, &path))
}

/// A record as read from `path`. A feature saved before `startTime` has none:
/// one already active or complete takes its file's time as when it started.
/// A record saved `planned` reads as `upcoming` (D-128).
pub(crate) fn stored(kind: &str, mut v: Value, path: &Path) -> Value {
    if v["state"] == "planned" {
        v["state"] = json!("upcoming");
    }
    if kind == "features" && v.get("startTime").is_none() {
        v["startTime"] = if v["state"] == "active" || v["state"] == "complete" {
            let modified = fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok());
            json!(modified.map_or(0, |d| d.as_secs()))
        } else {
            Value::Null
        };
    }
    v
}

/// Fills in the color slot (D-114) of each started feature saved before slots
/// were kept: one after every kept slot, in `startTime` order (ties by id).
fn with_slots(mut fs: Vec<Value>) -> Vec<Value> {
    let mut next = next_slot(&fs);
    let mut old: Vec<usize> = (0..fs.len()).filter(|&i| fs[i].get("colorSlot").is_none()).collect();
    old.sort_by(|&a, &b| {
        fs[a]["startTime"].as_u64().cmp(&fs[b]["startTime"].as_u64()).then_with(|| fs[a]["id"].as_str().cmp(&fs[b]["id"].as_str()))
    });
    for i in old {
        fs[i]["colorSlot"] = if fs[i]["startTime"].is_null() {
            Value::Null
        } else {
            next += 1;
            json!(next - 1)
        };
    }
    fs
}

/// The slot the next feature to start takes: one past the highest kept.
fn next_slot(fs: &[Value]) -> u64 {
    fs.iter().filter_map(|f| f["colorSlot"].as_u64()).max().map_or(0, |m| m + 1)
}

/// Saves the slots `with_slots` fills in, so a later change can't move them
/// (D-114 point 2). Called before a feature is written or deleted.
fn keep_slots(top: &Path) -> Result<(), Failure> {
    let stored = stored_records(top, "features");
    for (was, f) in stored.iter().zip(with_slots(stored.clone())) {
        let id = f["id"].as_str().unwrap_or("");
        if was.get("colorSlot").is_none() && !f["colorSlot"].is_null() && valid_id(id) {
            batch_replace(&strategy_dir(top, "features").join(format!("{id}.json")), &f)?;
        }
    }
    Ok(())
}

/// A feature's `startTime` is when it first became `active`; it is never
/// cleared. Its `colorSlot` is set with it, one past the highest any other
/// feature holds, and never changes (D-114).
pub(crate) fn mark_start(top: &Path, kind: &str, v: &mut Value) {
    if kind != "features" {
        return;
    }
    if v["state"] == "active" && v["startTime"].is_null() {
        v["startTime"] = json!(now());
    }
    if !v["startTime"].is_null() && v["colorSlot"].is_null() {
        let others: Vec<Value> = records(top, kind).into_iter().filter(|f| f["id"] != v["id"]).collect();
        v["colorSlot"] = json!(next_slot(&others));
    }
}

/// Writes a record and logs it as a root event, as `save_record` does. A
/// feature saved `active` plans its Backlog tasks (D-134); one whose milestone
/// changed moves its tasks with it (D-188).
pub(crate) fn put_record(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    recover(top)?;
    let id = v["id"].as_str().unwrap_or("");
    let was = if kind == "features" { record(top, kind, id) } else { None };
    let moved = was.as_ref().is_some_and(|old| old["milestone"] != v["milestone"]);
    let active = kind == "features" && v["state"] == "active";
    let announced = kind == "features" && was.as_ref().is_some_and(|old| feature_shows(old)) != feature_shows(v);
    if kind == "features" && (active || moved || announced) {
        let mut writes = vec![];
        let mut unchanged = vec![];
        for mut t in all_tasks(top)?.into_iter().filter(|t| t["feature"] == id) {
            let mut changed = moved;
            if moved {
                t["milestone"] = v["milestone"].clone();
            }
            if active && plan_for_feature(top, &mut t, v) {
                changed = true;
            }
            if changed {
                writes.push(t);
            } else if announced {
                unchanged.push(t);
            }
        }
        write_tasks_then(top, &writes, Some(v), || {
            save_record_in(top, kind, v)?;
            for t in &unchanged {
                batch_log(t["home"].as_str().unwrap_or("."), &json!({"task": t}));
            }
            Ok(())
        })?;
    } else {
        save_record(top, kind, v)?;
    }
    Ok(())
}

/// Writes a record and logs it as a root event: no project (D-87 point 7).
fn save_record(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    let batch = batch(top)?;
    match save_record_in(top, kind, v) {
        Ok(()) => batch_commit(batch),
        Err(e) => {
            batch_rollback(batch);
            Err(e)
        }
    }
}

/// Removes a record and queues its event, in the open batch.
pub(crate) fn remove_record(top: &Path, kind: &str, id: &str) -> Result<(), Failure> {
    if kind == "features" {
        keep_slots(top)?;
    }
    let dir = strategy_dir(top, kind);
    batch_remove(&dir.join(format!("{id}.json")))?;
    let event = if kind == "milestones" { json!({"removedMilestone": id}) } else { json!({"removedFeature": id}) };
    batch_log("", &event);
    Ok(())
}

/// The task log `/events` reads: every change in order, with the project it
/// is shown under.
pub fn events(top: &Path) -> Result<Connection, Failure> {
    fs::create_dir_all(store_dir(top))?;
    let db = Connection::open(store_dir(top).join("events.db"))?;
    db.busy_timeout(Duration::from_secs(10))?;
    db.execute_batch("PRAGMA journal_mode=WAL;")?;
    let auto: i64 = db.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if auto != 1 {
        // Reclaim deleted rows as they commit, so appends never run a blocking vacuum.
        if let Err(e) = db.execute_batch("PRAGMA auto_vacuum=FULL; VACUUM;") {
            eprintln!("solace: could not enable autovacuum on the task log: {e}");
        }
    }
    db.execute_batch("CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY, project TEXT NOT NULL, data TEXT NOT NULL)")?;
    Ok(db)
}

/// Appends `data` to the task log. The file or removal it reports is already
/// done, so a failed append doesn't fail the change (a client would retry it and
/// repeat a note or action): the board catches up from the files when it next
/// connects (D-78).
pub(crate) fn log(top: &Path, project: &str, data: &Value) {
    let db = match events(top) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("the task log did not take a change: {}", e.0);
            return;
        }
    };
    if let Err(e) = db.execute("INSERT INTO events(project,data) VALUES (?1,?2)", params![project, data.to_string()]) {
        eprintln!("the task log did not take a change: {e}");
        return;
    }
    if let Err(e) = prune_event_log(&db) {
        eprintln!("the task log did not prune: {}", e.0);
    }
}

/// Writes task `t`, returning what to log and the features that follow it.
/// `feature` is the feature being saved, when its tasks are written with it.
fn write_task(top: &Path, t: &Value, feature: Option<&Value>) -> Result<(String, Value, Vec<Value>), Failure> {
    let id = t["id"].as_str().unwrap_or("").to_string();
    if !valid_id(&id) {
        return board::err("E7001", "invalid task id");
    }
    // An active feature's tasks are never in Backlog (D-134).
    let mut t = t.clone();
    // The mark comes from the home on each load (D-160): it never goes in the file.
    t.as_object_mut().unwrap().remove("noProject");
    match feature {
        Some(f) => {
            plan_for_feature(top, &mut t, f);
        }
        None => {
            plan(top, &mut t);
        }
    }
    let path = store_dir(top).join(format!("{id}.json"));
    let was: Option<Value> = jsonx::read_opt(&path);
    // A feature follows its tasks (D-128): only a new task, a moved one or a
    // changed link can change it. It is worked out with this save in place, so
    // a save that makes its own feature active again is planned before the
    // task is written, once (D-134). The feature is saved after the task, so a
    // failed task save leaves it as it was.
    let mut changed = Vec::new();
    if feature.is_none() && was.as_ref().is_none_or(|w| w["feature"] != t["feature"] || w["column"] != t["column"]) {
        let old = was.as_ref().map_or(Value::Null, |w| w["feature"].clone());
        let mut fids: Vec<String> = [&old, &t["feature"]].into_iter().filter_map(Value::as_str).map(String::from).collect();
        fids.dedup();
        for fid in fids {
            match follow_tasks(top, &fid, &t) {
                Ok(Some(f)) => {
                    if t["feature"] == fid.as_str() {
                        plan_for_feature(top, &mut t, &f);
                    }
                    changed.push(f);
                }
                Ok(None) => {}
                Err(e) => eprintln!("updating feature {fid}: {}", e.0),
            }
        }
    }
    batch_replace(&store_dir(top).join(format!("{id}.json")), &t)?;
    let home = t["home"].as_str().unwrap_or(".").to_string();
    Ok((home, t, changed))
}

pub(crate) fn finish_write(top: &Path, home: &str, t: &Value, changed: &[Value]) -> Result<(), Failure> {
    batch_log(home, &json!({"task":t}));
    for f in changed {
        save_record_in(top, "features", f)?;
    }
    Ok(())
}

/// Writes tasks as one change and lets their features follow, then `after`, as
/// one journaled batch. If writing or `after` fails, every file is put back, so
/// a cascade leaves no task on the old milestone and a power loss leaves a
/// record to put them back.
pub(crate) fn write_tasks_then<T>(
    top: &Path,
    tasks: &[Value],
    feature: Option<&Value>,
    after: impl FnOnce() -> Result<T, Failure>,
) -> Result<T, Failure> {
    let batch = batch(top)?;
    let mut written: Vec<WrittenTask> = Vec::new();
    let mut failed = None;
    for t in tasks {
        match write_task(top, t, feature) {
            Ok((home, task, changed)) => written.push(WrittenTask { home, task, changed }),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    let out = match failed {
        Some(e) => Err(e),
        None => (|| {
            for w in &written {
                finish_write(top, &w.home, &w.task, &w.changed)?;
            }
            after()
        })(),
    };
    match out {
        Ok(out) => batch_commit(batch).map(|()| out),
        Err(e) => {
            batch_rollback(batch);
            Err(e)
        }
    }
}

/// A task written by `write_tasks_then`, with what to log once every write has
/// succeeded.
pub(crate) struct WrittenTask {
    pub(crate) home: String,
    pub(crate) task: Value,
    pub(crate) changed: Vec<Value>,
}

/// Puts `<dir>/<name>` back as `before`; a file that did not exist is removed.
/// A failed restore is reported, so an inconsistent store leaves a trace, and
/// returns false.
fn restore_file(dir: &Path, name: &str, before: Option<Vec<u8>>) -> bool {
    let restored = match before {
        Some(bytes) => fsx::replace_bytes(dir, name, &bytes),
        None => fs::remove_file(dir.join(name)).map_err(Failure::from),
    };
    if let Err(e) = &restored {
        eprintln!("restoring {name}: {}", e.0);
    }
    restored.is_ok()
}

/// Logs a saved milestone or feature as a root event.
fn log_record(kind: &str, v: &Value) {
    let event = if kind == "milestones" { json!({"milestone": v}) } else { json!({"feature": v}) };
    batch_log("", &event);
}

/// Writes every feature in the open batch, then logs them. The slot fixes belong to
/// the same batch, so a failed cascade puts them back too.
pub(crate) fn write_features(top: &Path, features: &[Value]) -> Result<(), Failure> {
    if features.is_empty() {
        return Ok(());
    }
    keep_slots(top)?;
    let dir = strategy_dir(top, "features");
    for f in features {
        let id = f["id"].as_str().unwrap_or("");
        if !valid_id(id) {
            return board::err("E7001", "invalid id");
        }
        batch_replace(&dir.join(format!("{id}.json")), f)?;
    }
    for f in features {
        log_record("features", f);
    }
    Ok(())
}

/// A handle to the thread's task batch: whether it began the batch and whether
/// it has finished. A nested batch joins the outer one.
pub(crate) struct Batch {
    outer: bool,
    finished: bool,
}

/// Starts a batch, first putting back one a crash left unfinished.
pub(crate) fn batch(top: &Path) -> Result<Batch, Failure> {
    if let Some(nested) = with_batch(|active| match active {
        Some(active) if active.top.as_path() == top => Ok(Some(Batch { outer: false, finished: false })),
        Some(_) => Err(Failure("another task batch is open on this thread".into(), 1)),
        None => Ok(None),
    })? {
        return Ok(nested);
    }
    let lock = BATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    recover_locked(top)?;
    let (journal, live_lock) = claim_journal(top)?;
    with_batch(|active| {
        *active = Some(ActiveBatch {
            top: top.to_path_buf(),
            journal,
            files: Vec::new(),
            events: Vec::new(),
            failed: false,
            _thread_lock: Some(lock),
            _live_lock: live_lock,
        });
    });
    Ok(Batch { outer: true, finished: false })
}

/// Records `p`'s bytes before its first change in the open batch.
pub(crate) fn batch_keep(p: &Path) -> Result<(), Failure> {
    let Some((rel, journal, before)) = with_batch(|active| -> Result<Option<(String, PathBuf, Option<Vec<u8>>)>, Failure> {
        let active = active.as_mut().ok_or_else(|| Failure("the task batch is no longer open".into(), 1))?;
        let rel = p.strip_prefix(&active.top).map_err(|_| Failure("task store path outside its root".into(), 1))?;
        let rel = rel.to_string_lossy().into_owned();
        if active.files.iter().any(|(name, _)| *name == rel) {
            return Ok(None);
        }
        Ok(Some((rel, active.journal.clone(), fs::read(p).ok())))
    })?
    else {
        return Ok(());
    };
    batch_save(&journal, &rel, &before)?;
    with_batch(|active| {
        if let Some(active) = active.as_mut() {
            active.files.push((rel, before));
        }
    });
    Ok(())
}

/// Appends one file's before bytes to the journal before it changes.
fn batch_save(journal: &Path, path: &str, before: &Option<Vec<u8>>) -> Result<(), Failure> {
    let entry = json!({"path": path, "bytes": before});
    let dir = journal.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut file = fs::OpenOptions::new().create(true).append(true).open(journal)?;
    file.write_all(&serde_json::to_vec(&entry).unwrap())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Rewrites `p` as `v`, journaling its before bytes first.
pub(crate) fn batch_replace(p: &Path, v: &Value) -> Result<(), Failure> {
    batch_keep(p)?;
    let dir = p.parent().unwrap_or(Path::new("."));
    let name = p.file_name().and_then(|n| n.to_str()).ok_or_else(|| Failure("invalid task store path".into(), 1))?;
    jsonx::replace_pretty(dir, name, v)
}

/// Removes `p`, journaling its before bytes first.
fn batch_remove(p: &Path) -> Result<(), Failure> {
    batch_keep(p)?;
    fs::remove_file(p)?;
    fs::File::open(p.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}

/// Keeps the batch's files and drops its journal.
pub(crate) fn batch_commit(mut batch: Batch) -> Result<(), Failure> {
    if !batch.outer {
        batch.finished = true;
        return Ok(());
    }
    let Some(mut active) = with_batch(|active| active.take()) else {
        batch.finished = true;
        return Err(Failure("the task batch is no longer open".into(), 1));
    };
    batch.finished = true;
    if active.failed {
        restore_batch(active);
        return Err(Failure("a nested task batch failed".into(), 1));
    }
    if !active.journal.exists() {
        log_events(&active.top, &mut active.events);
        batch_finish(active);
        return Ok(());
    }
    let dir = active.journal.parent().unwrap_or(Path::new("."));
    let dropped = fs::remove_file(&active.journal).and_then(|_| fs::File::open(dir).and_then(|f| f.sync_all()));
    if let Err(e) = dropped {
        restore_batch(active);
        return Err(e.into());
    }
    log_events(&active.top, &mut active.events);
    batch_finish(active);
    Ok(())
}

pub(crate) fn batch_rollback(mut batch: Batch) {
    batch.finished = true;
    if !batch.outer {
        with_batch(|active| {
            if let Some(active) = active.as_mut() {
                active.failed = true;
            }
        });
        return;
    }
    if let Some(active) = with_batch(|active| active.take()) {
        restore_batch(active);
    }
}

/// Puts back the files of a batch a crash left unfinished, if its journal is
/// still there. A batch on this thread skips recovery: its own journal is live.
pub(crate) fn recover(top: &Path) -> Result<(), Failure> {
    if with_batch(|active| active.is_some()) {
        return Ok(());
    }
    let _lock = BATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    recover_locked(top)
}

/// Writes a record and logs it as a root event, in the open batch.
fn save_record_in(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    let id = v["id"].as_str().unwrap_or("");
    if !valid_id(id) {
        return board::err("E7001", "invalid id");
    }
    if kind == "features" {
        keep_slots(top)?;
    }
    batch_replace(&strategy_dir(top, kind).join(format!("{id}.json")), v)?;
    log_record(kind, v);
    Ok(())
}

/// Serializes batches and recovery across threads of one process.
static BATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` with this thread's active batch, if any.
pub(crate) fn with_batch<R>(f: impl FnOnce(&mut Option<ActiveBatch>) -> R) -> R {
    thread_local! {
        static ACTIVE: std::cell::RefCell<Option<ActiveBatch>> = const { std::cell::RefCell::new(None) };
    }
    ACTIVE.with(|active| f(&mut active.borrow_mut()))
}

/// Drops the batch's live mark, so the next one takes the lock.
fn batch_finish(batch: ActiveBatch) {
    let lock = batch.journal.with_extension("lock");
    drop(batch);
    let _ = fs::remove_file(lock);
}

/// Puts back every task batch journal a crash left unfinished. A batch another
/// process is still running holds its journal's file lock, so it is left alone.
fn recover_locked(top: &Path) -> Result<(), Failure> {
    let pending = top.join(".solace/pending");
    let Ok(entries) = fs::read_dir(&pending) else { return Ok(()) };
    let stems: std::collections::BTreeSet<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let json = p.extension().is_some_and(|e| e == "json");
            let tasks = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tasks-"));
            json && tasks
        })
        .map(|p| p.with_extension(""))
        .collect();
    for stem in stems {
        let lock = stem.with_extension("lock");
        let Some(owner) = fsx::try_lock_file(&lock)? else { continue };
        let recovered = recover_journal(top, &stem.with_extension("json"));
        drop(owner);
        let _ = fs::remove_file(&lock);
        recovered?;
    }
    Ok(())
}

/// Puts back the files named by a task batch journal.
fn restore_record_locked(top: &Path, record: &Value) -> Result<(), Failure> {
    let Some(files) = record["files"].as_array() else { return Ok(()) };
    for file in files.iter().rev() {
        let p = journal_target(top, file["path"].as_str().unwrap_or(""))?;
        if let Some(a) = file["bytes"].as_array() {
            let dir = p.parent().unwrap_or(top);
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            fsx::replace_bytes(dir, name, &jsonx::bytes(a))?;
        } else if p.exists() {
            fs::remove_file(&p)?;
            fs::File::open(p.parent().unwrap_or(top))?.sync_all()?;
        }
    }
    Ok(())
}

/// A fresh journal path and the file lock that tells other processes its batch is live.
fn claim_journal(top: &Path) -> Result<(PathBuf, fs::File), Failure> {
    let pending = top.join(".solace/pending");
    fs::create_dir_all(&pending)?;
    let journal = pending.join(format!("tasks-{}.json", crate::unique_id()));
    let owner =
        fsx::try_lock_file(&journal.with_extension("lock"))?.ok_or_else(|| Failure("the task journal is already in use".into(), 1))?;
    let prepared = (|| -> Result<(), Failure> {
        fs::File::create(&journal)?.sync_all()?;
        fs::File::open(&pending)?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = prepared {
        drop(owner);
        let _ = fs::remove_file(journal.with_extension("lock"));
        return Err(e);
    }
    Ok((journal, owner))
}

/// Puts back the files one dead batch's journal names, then drops it.
fn recover_journal(top: &Path, journal: &Path) -> Result<(), Failure> {
    let Ok(bytes) = fs::read(journal) else { return Ok(()) };
    let record = journal_record(&bytes)?;
    restore_record_locked(top, &record)?;
    fs::remove_file(journal)?;
    fs::File::open(journal.parent().unwrap_or(top))?.sync_all()?;
    Ok(())
}

/// The file a journal entry names, which must lie inside the root.
fn journal_target(top: &Path, name: &str) -> Result<PathBuf, Failure> {
    let rel = Path::new(name);
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return board::err("E7001", "invalid task journal");
    }
    Ok(top.join(rel))
}

/// Queues a root event for the batch, after its files commit.
pub(crate) fn batch_log(project: &str, data: &Value) {
    with_batch(|active| {
        if let Some(active) = active.as_mut() {
            active.events.push((project.to_string(), data.clone()));
        }
    });
}

/// Logs what the committed batch changed, after every file is in place.
fn log_events(top: &Path, events: &mut Vec<(String, Value)>) {
    for (project, data) in std::mem::take(events) {
        log(top, &project, &data);
    }
}

/// A journal's files in change order, from either a whole record or appended
/// lines; an unfinished last line names no change.
fn journal_record(bytes: &[u8]) -> Result<Value, Failure> {
    let Ok(text) = std::str::from_utf8(bytes) else { return board::err("E7001", "invalid task journal") };
    let text = text.trim();
    if text.is_empty() {
        return Ok(json!({"files": []}));
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(files) = v["files"].as_array() {
            return Ok(json!({"files": files}));
        }
        if v["path"].is_string() {
            return Ok(json!({"files": [v]}));
        }
        return board::err("E7001", "invalid task journal");
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut files = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v["path"].is_string() => files.push(v),
            Ok(_) => return board::err("E7001", "invalid task journal"),
            Err(_) if lines[i + 1..].iter().all(|l| l.trim().is_empty()) => break,
            Err(_) => return board::err("E7001", "invalid task journal"),
        }
    }
    Ok(json!({"files": files}))
}

/// The task-store files written whole or nothing: each changed path's bytes
/// before its first change, the events to log once committed, the process-wide
/// lock (outermost batch only) and the file lock that marks its journal live.
pub(crate) struct ActiveBatch {
    top: PathBuf,
    pub(super) journal: PathBuf,
    files: Vec<(String, Option<Vec<u8>>)>,
    events: Vec<(String, Value)>,
    /// A nested batch failed, so the outer one must not commit.
    failed: bool,
    _thread_lock: Option<std::sync::MutexGuard<'static, ()>>,
    _live_lock: fs::File,
}

/// Puts every file back and drops the journal; a file that cannot be put back
/// keeps the journal, so the next batch tries again.
fn restore_batch(batch: ActiveBatch) {
    let restored = batch.files.iter().rev().fold(true, |ok, (path, before)| {
        let p = batch.top.join(path);
        let dir = p.parent().unwrap_or(Path::new("."));
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        restore_file(dir, name, before.clone()) && ok
    });
    if restored {
        let _ = fs::remove_file(&batch.journal);
    }
    batch_finish(batch);
}

impl Drop for Batch {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.outer {
            if let Some(active) = with_batch(|active| active.take()) {
                restore_batch(active);
            }
        } else {
            with_batch(|active| {
                if let Some(active) = active.as_mut() {
                    active.failed = true;
                }
            });
        }
    }
}

/// The log keeps its newest row once it passes this many events: older rows
/// only spare a reconnecting board a snapshot.
const EVENT_LOG_LIMIT: i64 = 100;

/// Keeps the task log bounded without restarting its numbering: the newest row
/// stays, so the next event still gets a higher `seq`. A board whose position
/// is now outside the log rebuilds from the files (D-78).
fn prune_event_log(db: &Connection) -> Result<(), Failure> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
    if count > EVENT_LOG_LIMIT {
        db.execute("DELETE FROM events WHERE seq < (SELECT MAX(seq) FROM events)", [])?;
    }
    Ok(())
}

/// Re-homes a task whose home is no project (D-160), as one operation of the task
/// server: it reads the task again, so two loads move it once, moves its folder,
/// notes the move and writes it. The task as it is saved.
pub(crate) fn rehome(top: &Path, t: &Value) -> Result<Value, Failure> {
    let (top, tid) = (top.to_path_buf(), t["id"].as_str().unwrap_or("").to_string());
    op(&top.clone(), move || {
        let mut t = read_task(&top, &store_dir(&top).join(format!("{tid}.json")))?;
        let Some((home, dirs)) = rehome_to(&workspace::members(&top)?, &t) else { return Ok(t) };
        let old = t["home"].as_str().unwrap_or(".").to_string();
        // A homeless task's runs wrote in the root, where its folder is.
        let from = scope::path(&top, &old);
        let src = if from.join(".solace/board").join(&tid).is_dir() { from } else { top.clone() };
        move_task(&src, &mut t, &scope::path(&top, &home))?;
        // A re-home keeps only the projects of the scope (D-160): the note names what it drops.
        let dropped: Vec<String> = scope::of(&t, &old).into_iter().filter(|d| !dirs.iter().any(|x| x == d)).collect();
        t["scope"] = json!(dirs);
        let dropped = if dropped.is_empty() {
            String::new()
        } else {
            format!(" Dropped {} from its projects.", dropped.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", "))
        };
        note(&mut t, "solace", None, &format!("Moved from `{old}` to `{home}`: its home was no project.{dropped}"));
        write(&top, &t)?;
        Ok(t)
    })
}

/// Whether a found feature is one whose tasks the board shows (D-128).
pub(crate) fn feature_shows(f: &Value) -> bool {
    matches!(f["state"].as_str(), Some("active" | "complete"))
}
