use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;
use serde_json::Value;
use serde_json::json;
use std::path::PathBuf;
use std::path::Path;
use std::fs;
use crate::now;
use crate::unique_id;
use crate::failure::Failure;
use crate::tasks::store::write;
use crate::tasks::strategy::put_run;
use crate::tasks::store::recover;
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
use crate::board;
use crate::scope;
use crate::fsx;
use std::io::Write;
use crate::board::task_model::valid_id;
use crate::board::task_model::read_task;
use crate::board::task_model::flow;
use crate::board::task_model::note;
use crate::board::task_model::flag;
use crate::board::task_model::task_at_home;
use crate::board::reviews::settle_final;
use crate::board::reviews::resettle;
use crate::board::task_model::store_dir;
use crate::board::task_model::message;
pub(crate) type Job = Box<dyn FnOnce() + Send>;

pub(crate) struct Server {
    pub(crate) jobs: mpsc::Sender<Job>,
    pub(crate) thread: std::thread::ThreadId,
    pub(crate) top: PathBuf,
}

/// This process's task server; only `solace serve` starts one.
pub(super) static SERVER: OnceLock<Server> = OnceLock::new();

/// This process's task server, when it serves the root `top`: another root
/// (a test's, as the server test runs in the same process) has no server here.
fn server_for(top: &Path) -> Option<&'static Server> {
    SERVER.get().filter(|s| s.top == top)
}

/// Starts the task server for the root `top`: first the messages left in the
/// inbox, then each operation and message in the order they arrive.
pub fn start(top: &Path) -> Result<(), Failure> {
    recover(top)?;
    let (jobs, queue) = mpsc::channel::<Job>();
    let root = top.to_path_buf();
    let thread = std::thread::spawn(move || {
        // Held while this process serves the root, so others see it is served;
        // taken once the root has a `.solace`, which an empty folder doesn't yet.
        let mut held = None;
        // Tasks a stop or crash left in a final column unsettled, settled off this thread.
        let again = root.clone();
        std::thread::spawn(move || resettle(&again));
        loop {
            tick(&root, &mut held, &queue);
        }
    });
    SERVER
        .set(Server { jobs, thread: thread.thread().id(), top: scope::top(top) })
        .map_err(|_| Failure("the task server is already running".into(), 1))
}

/// One turn of the task server: takes the server lock (`held`) once the root
/// has a `.solace`, applies the inbox, then runs at most one queued operation.
pub(super) fn tick(root: &Path, held: &mut Option<fs::File>, queue: &mpsc::Receiver<Job>) {
    if held.is_none() && root.join(".solace").is_dir() {
        *held = fs::create_dir_all(store_dir(root)).ok().and_then(|_| fsx::try_lock_file(&server_lock(root)).ok().flatten());
    }
    drain(root);
    if let Ok(job) = queue.recv_timeout(Duration::from_millis(50)) {
        // Messages sent before this job was queued (an agent's `done` just
        // before it exits) are applied first, so they keep their order.
        drain(root);
        // A failed operation fails its request, not the server.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
    }
}

/// Runs `f` as one operation of the task server for the root `top`: on its
/// thread when this process serves that root, else here (a process without a
/// server for it, as tests).
/// An operation never takes a project lock, so a caller holding one can't
/// block the server. An operation can be slow (an undo type-checks each
/// project inside it): other writes and senders wait their turn, however long it
/// takes (`applied` has no time limit while a server runs).
pub fn op<R: Send + 'static>(top: &Path, f: impl FnOnce() -> Result<R, Failure> + Send + 'static) -> Result<R, Failure> {
    match server_for(top) {
        Some(server) if server.thread != std::thread::current().id() => {
            let (reply, answer) = mpsc::channel();
            let stopped = || Failure("the task server stopped".into(), 1);
            server.jobs.send(Box::new(move || drop(reply.send(f())))).map_err(|_| stopped())?;
            answer.recv().map_err(|_| stopped())?
        }
        _ => f(),
    }
}

pub(crate) fn inbox(top: &Path) -> PathBuf {
    store_dir(top).join("inbox")
}

/// Leaves message `m` for the task server, durably: written, synced and
/// renamed into the inbox before this returns. The path is gone once the
/// server has applied it.
pub fn send(top: &Path, mut m: Value) -> Result<PathBuf, Failure> {
    let id = message_id();
    m["id"] = json!(id);
    let dir = inbox(top);
    fs::create_dir_all(&dir)?;
    let (part, to) = (dir.join(format!("{id}.part")), dir.join(format!("{id}.json")));
    let mut f = fs::File::create(&part)?;
    f.write_all(m.to_string().as_bytes())?;
    f.sync_all()?;
    fs::rename(&part, &to)?;
    fs::File::open(&dir)?.sync_all()?;
    Ok(to)
}

/// Ids sort in the order messages were sent.
fn message_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{nanos:024}-{}", unique_id())
}

pub(crate) fn server_lock(top: &Path) -> PathBuf {
    store_dir(top).join("server.lock")
}

/// Whether a server is running for the root, so a message is applied soon.
pub fn serving(top: &Path) -> bool {
    server_for(top).is_some() || fsx::try_lock_file(&server_lock(top)).is_ok_and(|f| f.is_none())
}

/// Applies message `m` before returning, for a sender that reads the task
/// next: as an operation where this process serves the root, through the
/// inbox when another one does, else here, where no server can write.
pub fn deliver(top: &Path, mut m: Value) -> Result<(), Failure> {
    // Without a server here, the server lock tells whether another process has one.
    // Where none does, the lock is held while this applies the message, after the
    // inbox's earlier ones (a create a stopped server left must land before a note
    // to it), so no server starts in between. A caller may hold project locks, so
    // nothing is settled here: `serve` settles what is left.
    let mut _held = None;
    if server_for(top).is_none() {
        match fsx::try_lock_file(&server_lock(top)) {
            Ok(None) => {
                let sent = send(top, m)?;
                // Waits for as long as the server lives; if it stops, the message stays for the next `serve`.
                if applied(top, &sent) && rejected(top, &sent) {
                    return Err(Failure(
                        "the task server could not apply the message; it was set aside in .solace/tasks/inbox/failed".into(),
                        1,
                    ));
                }
                return Ok(());
            }
            Ok(Some(f)) => {
                _held = Some(f);
                drain_with(top, false);
            }
            Err(_) => {}
        }
    }
    m["id"] = json!(message_id());
    let top = top.to_path_buf();
    if let Some(tid) = op(&top, {
        let top = top.clone();
        move || apply(&top, &m)
    })? {
        settle_later(&top, tid);
    }
    Ok(())
}

/// What a sender says when no server is there to take its message: it stays in
/// the inbox, for the next `serve`.
pub const QUEUED: &str = "Recorded; it reaches the board when `solace serve` runs.";

/// Waits until the server has applied the message at `path`, however long that
/// takes while a server runs: its thread may be busy with a slow operation, and
/// a sender that gave up would act as if the message had not been taken. False
/// only when the server stops first: the message stays in the inbox for the next
/// `serve`, so the sender says that and returns instead of waiting forever for a
/// server that will never apply it.
pub fn applied(top: &Path, path: &Path) -> bool {
    while path.exists() {
        if !serving(top) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    true
}

/// Applies the inbox's messages in order, each once: a task remembers the
/// last message applied to it, so one applied just before a crash is skipped.
pub fn drain(top: &Path) {
    drain_with(top, true)
}

/// `drain`, settling the tasks it moves into a final column only if `settle`.
fn drain_with(top: &Path, settle: bool) {
    let Ok(entries) = fs::read_dir(inbox(top)) else { return };
    let mut paths: Vec<PathBuf> =
        entries.filter_map(|e| Some(e.ok()?.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    for p in paths {
        let bytes = fs::read(&p);
        let parsed = bytes.map_err(Failure::from).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| Failure(e.to_string(), 1)));
        // A failure may be brief (`solace.toml` mid-save), so it is tried again a
        // few times; nothing is written until an application succeeds.
        let result = parsed.and_then(|m| {
            let mut last = Ok(());
            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(200));
                }
                let applied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(top, &m)))
                    .unwrap_or_else(|_| Err(Failure("the message made the task server panic".into(), 1)));
                last = applied.map(|entered| {
                    if let Some(tid) = entered
                        && settle
                    {
                        settle_later(top, tid);
                    }
                });
                if last.is_ok() {
                    break;
                }
            }
            if let Err(e) = &last {
                flag_set_aside(top, &m, &e.0);
            }
            last
        });
        match result {
            Ok(()) => drop(fs::remove_file(&p)),
            // A message that can't be applied is set aside, never lost, and never tried again.
            Err(e) => {
                eprintln!("task message {}: {}", p.display(), e.0);
                let dir = failed_dir(top);
                if fs::create_dir_all(&dir).is_err() || fs::rename(&p, dir.join(p.file_name().unwrap())).is_err() {
                    // Without a place to keep it, its text goes to the log before it goes.
                    eprintln!("task message dropped: {}", fs::read_to_string(&p).unwrap_or_default());
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }
}

/// Tells the task, when it can be read, that a message for it was set aside, so
/// a human learns that something an agent said never took effect.
pub(crate) fn flag_set_aside(top: &Path, m: &Value, why: &str) {
    let Some(tid) = m["task"].as_str().filter(|t| valid_id(t)) else { return };
    let path = store_dir(top).join(format!("{tid}.json"));
    let Ok(mut t) = read_task(top, &path) else { return };
    let kind = m["kind"].as_str().unwrap_or("");
    let text = format!("A task message ({kind}) could not be applied and was set aside in .solace/tasks/inbox/failed: {why}");
    let Ok(w) = flow(&scope::path(top, t["home"].as_str().unwrap_or("."))) else { return };
    let note = json!({"kind":"note","author":"solace","text":text,"flag":"a task message was set aside"});
    if t["notes"].is_array() && message(&w, &mut t, &note).is_ok() {
        let _ = write(top, &t);
    }
}

pub(crate) fn failed_dir(top: &Path) -> PathBuf {
    inbox(top).join("failed")
}

/// Whether the server set the message at `path` aside because it could not apply it.
pub fn rejected(top: &Path, path: &Path) -> bool {
    path.file_name().is_some() && failed_path(top, path).exists()
}

/// Settles task `tid`, which a change just moved into a final column, after the
/// reply. Where this process serves the root it runs on a thread of its own
/// (D-77 point 5): settling takes project locks, and a holder of those may be
/// waiting on this thread. Elsewhere (no server, as tests) it runs here.
pub(crate) fn settle_later(top: &Path, tid: String) {
    let top = top.to_path_buf();
    let on_server = server_for(&top).is_some();
    let run = move || match settle_final(&top, &tid) {
        Ok(()) if settle_failed(&top, &tid) => clear_settle_failed(&top, &tid),
        Ok(()) => {}
        Err(e) => {
            eprintln!("settling task {tid}: {}", e.0);
            flag_settle_failed(&top, &tid, &e.0);
        }
    };
    if on_server {
        std::thread::spawn(run);
    } else {
        run();
    }
}

/// Applies a message; the task's id when it moved the task into a final column.
pub(crate) fn apply(top: &Path, m: &Value) -> Result<Option<String>, Failure> {
    let id = m["id"].as_str().unwrap_or("");
    if m["kind"] == "create" {
        let t = &m["new"];
        let tid = t["id"].as_str().unwrap_or("");
        if store_dir(top).join(format!("{tid}.json")).exists() {
            return Ok(None);
        }
        let mut t = t.clone();
        t["inbox"] = json!(id);
        return write(top, &t).map(|_| None);
    }
    // A wait hands over the notes and answers it read: the server alone records
    // them, and only while the run is still open.
    if m["kind"] == "delivery" {
        if m["target"] == "strategy" {
            let rid = m["run"].as_str().unwrap_or("");
            let Some(mut run) = crate::tasks::strategy::strategy_run(top, rid) else {
                return board::err("E7023", format!("unknown run {rid}"));
            };
            if run["inbox"].as_str().is_some_and(|last| last == id) {
                return Ok(None);
            }
            crate::board::delivery::record(&mut run, m, false);
            run["inbox"] = json!(id);
            put_run(top, &run)?;
            return Ok(None);
        }
        let tid = m["task"].as_str().unwrap_or("");
        if !valid_id(tid) {
            return board::err("E7001", "invalid task id");
        }
        let path = store_dir(top).join(format!("{tid}.json"));
        let mut t = read_task(top, &path)?;
        if t["inbox"].as_str().is_some_and(|last| last == id) {
            return Ok(None);
        }
        let i = crate::board::task_model::run_at(&t, m["run"].as_str().unwrap_or(""))?;
        let paused = !t["paused_from"].is_null();
        crate::board::delivery::record(&mut t["runs"][i], m, paused);
        t["inbox"] = json!(id);
        write(top, &t)?;
        return Ok(None);
    }
    // A message left by an older version; delivery wakes the run now.
    if m["kind"] == "strategy_resume" {
        return Ok(None);
    }
    // A strategy run's agent (D-93): its reply or proposal waits for the human,
    // whose next note or answer sets the run going again.
    if let Some(kind @ ("strategy_note" | "strategy_propose")) = m["kind"].as_str() {
        let rid = m["run"].as_str().unwrap_or("");
        let Some(mut run) = crate::tasks::strategy::strategy_run(top, rid) else {
            return board::err("E7023", format!("unknown run {rid}"));
        };
        if run["inbox"].as_str().is_some_and(|last| last == id) {
            return Ok(None);
        }
        let ended = run["state"] == "ended";
        match kind {
            "strategy_note" => note(&mut run, "agent", Some(rid), m["text"].as_str().unwrap_or("")),
            // A proposal can no longer be answered once the run has ended.
            "strategy_propose" if ended => {
                let what = m["question"]["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
                note(&mut run, "solace", Some(rid), &format!("A proposal arrived after the run ended, so it was not asked: {what}"));
            }
            // A question's time is when it is asked, so it orders among the notes;
            // `after`, the notes before it, orders it among those of the same second.
            // Run notes are never removed or reordered: `plan.conversation` matches
            // `after` against note indexes, so pruning run notes would need `after`
            // rewritten, or a note id instead.
            "strategy_propose" => {
                let mut q = m["question"].clone();
                q["time"] = json!(now());
                q["after"] = json!(run["notes"].as_array().map_or(0, Vec::len));
                run["questions"].as_array_mut().unwrap().push(q);
            }
            _ => {}
        }
        // A run that ended keeps the note, but nothing wakes it.
        if !ended {
            run["state"] = json!("waiting");
        }
        run["inbox"] = json!(id);
        put_run(top, &run)?;
        return Ok(None);
    }
    let tid = m["task"].as_str().unwrap_or("");
    // The id names the task's file, so one that could leave the store is refused.
    if !valid_id(tid) {
        return board::err("E7001", "invalid task id");
    }
    let path = store_dir(top).join(format!("{tid}.json"));
    let mut t = read_task(top, &path)?;
    // Only the message applied just before a crash can come again; a message with
    // an older id may still be new (it was sent before a later one landed).
    if t["inbox"].as_str().is_some_and(|last| last == id) {
        return Ok(None);
    }
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let w = flow(&home)?;
    let was_final = crate::board::task_model::column(&w, &t)["final"] == true;
    message(&w, &mut t, m)?;
    t["inbox"] = json!(id);
    write(top, &t)?;
    Ok((!was_final && crate::board::task_model::column(&w, &t)["final"] == true).then(|| tid.to_string()))
}

/// Leaves a `solace` note and an attention flag on a task whose deferred settle
/// failed, so a human learns that its undo records may still be present. The
/// marker makes the next `resettle` try again even when no rows remain.
fn flag_settle_failed(top: &Path, tid: &str, why: &str) {
    let root = top.to_path_buf();
    let call = root.clone();
    let id = tid.to_string();
    let reason = why.to_string();
    let result = op(&root, move || {
        let mut t = task_at_home(&call, &id)?;
        if t["settleFailed"] == true {
            return Ok(());
        }
        t["settleFailed"] = json!(true);
        note(&mut t, "solace", None, &format!("{SETTLE_FAILED_NOTE} {reason}\n\nSolace will try again when this board is served."));
        flag(&mut t, SETTLE_FAILED_REASON);
        write(&call, &t)
    });
    if let Err(e) = result {
        eprintln!("recording the settling failure of task {tid}: {}", e.0);
    }
}

/// Clears the failure left by `flag_settle_failed`: the marker, the note, and
/// the attention flag it set, never another reason the task may carry.
fn clear_settle_failed(top: &Path, tid: &str) {
    let root = top.to_path_buf();
    let call = root.clone();
    let id = tid.to_string();
    let result = op(&root, move || {
        let mut t = task_at_home(&call, &id)?;
        if t["settleFailed"] != true {
            return Ok(());
        }
        t.as_object_mut().unwrap().remove("settleFailed");
        if t["attention"]["reason"] == SETTLE_FAILED_REASON {
            t["attention"] = Value::Null;
        }
        if let Some(notes) = t["notes"].as_array_mut() {
            notes.retain(|n| {
                !(n["author"] == "solace" && n["run"].is_null() && n["text"].as_str().is_some_and(|s| s.starts_with(SETTLE_FAILED_NOTE)))
            });
        }
        write(&call, &t)
    });
    if let Err(e) = result {
        eprintln!("clearing the settling failure of task {tid}: {}", e.0);
    }
}

pub(crate) const SETTLE_FAILED_REASON: &str = "settling this task failed";

pub(crate) const SETTLE_FAILED_NOTE: &str = "Settling this task failed:";

/// Whether task `tid` still carries the marker of a failed deferred settle.
fn settle_failed(top: &Path, tid: &str) -> bool {
    task_at_home(top, tid).is_ok_and(|t| t["settleFailed"] == true)
}

/// The file a message the server could not apply is kept in.
pub(crate) fn failed_path(top: &Path, path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => failed_dir(top).join(name),
        None => failed_dir(top),
    }
}

/// Asks this process's task server for `top` how it is doing: `Yes` when it
/// takes a no-op within `wait`, `Busy` while an operation still holds its
/// thread, and `No` when there is no server or its thread has ended.
pub fn responding(top: &Path, wait: Duration) -> Answering {
    answering(server_for(top), std::thread::current().id(), wait)
}

fn answering(server: Option<&Server>, current: std::thread::ThreadId, wait: Duration) -> Answering {
    let Some(server) = server else {
        return Answering::No;
    };
    if server.thread == current {
        return Answering::Yes;
    }
    let (reply, answer) = mpsc::channel();
    if server
        .jobs
        .send(Box::new(move || {
            let _ = reply.send(());
        }))
        .is_err()
    {
        return Answering::No;
    }
    match answer.recv_timeout(wait) {
        Ok(()) => Answering::Yes,
        Err(mpsc::RecvTimeoutError::Timeout) => Answering::Busy,
        Err(mpsc::RecvTimeoutError::Disconnected) => Answering::No,
    }
}

/// How this process's task server answers a health check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answering {
    /// It took a no-op within the wait.
    Yes,
    /// Its queue is live but an operation still holds its thread.
    Busy,
    /// No task server runs for the root, or its thread has gone.
    No,
}
