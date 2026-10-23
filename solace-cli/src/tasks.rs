//! The task server (D-77): the only writer of the root's task store,
//! `<root>/.solace/tasks/`, and its API, `/tasks/...`. It applies operations
//! one at a time, in order, on one thread, writing each task file by
//! write-then-rename and logging each change for `/events`. Other processes
//! (agents, the CLI) don't write tasks: they leave messages in `inbox/`, which
//! the server applies in order, and again after a restart (D-77 point 3).
//! Clients depend only on the API and the messages, never on how this is built.
use super::*;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

struct Server {
    jobs: mpsc::Sender<Job>,
    thread: std::thread::ThreadId,
    top: PathBuf,
}
/// This process's task server; only `solace serve` starts one.
static SERVER: OnceLock<Server> = OnceLock::new();
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
        std::thread::spawn(move || board::resettle(&again));
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
fn tick(root: &Path, held: &mut Option<fs::File>, queue: &mpsc::Receiver<Job>) {
    if held.is_none() && root.join(".solace").is_dir() {
        *held = fs::create_dir_all(board::store_dir(root)).ok().and_then(|_| try_lock_file(&server_lock(root)).ok().flatten());
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
        let mut t = board::task(&root, &tid)?;
        let out = f(&mut t)?;
        write(&scope::top(&root), &t)?;
        Ok(out)
    })
}

fn write(top: &Path, t: &Value) -> Result<(), Failure> {
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
/// Re-homes a task whose home is no project (D-160), as one operation of the task
/// server: it reads the task again, so two loads move it once, moves its folder,
/// notes the move and writes it. The task as it is saved.
pub(super) fn rehome(top: &Path, t: &Value) -> Result<Value, Failure> {
    let (top, tid) = (top.to_path_buf(), t["id"].as_str().unwrap_or("").to_string());
    op(&top.clone(), move || {
        let mut t = board::read_task(&top, &board::store_dir(&top).join(format!("{tid}.json")))?;
        let Some((home, dirs)) = board::rehome_to(&workspace::members(&top)?, &t) else { return Ok(t) };
        let old = t["home"].as_str().unwrap_or(".").to_string();
        // A homeless task's runs wrote in the root, where its folder is.
        let from = scope::path(&top, &old);
        let src = if from.join(".solace/board").join(&tid).is_dir() { from } else { top.clone() };
        board::move_task(&src, &mut t, &scope::path(&top, &home))?;
        // A re-home keeps only the projects of the scope (D-160): the note names what it drops.
        let dropped: Vec<String> = scope::of(&t, &old).into_iter().filter(|d| !dirs.iter().any(|x| x == d)).collect();
        t["scope"] = json!(dirs);
        let dropped = if dropped.is_empty() {
            String::new()
        } else {
            format!(" Dropped {} from its projects.", dropped.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", "))
        };
        board::note(&mut t, "solace", None, &format!("Moved from `{old}` to `{home}`: its home was no project.{dropped}"));
        write(&top, &t)?;
        Ok(t)
    })
}
/// Moves task `t` from Backlog to Planning when its feature is `active`
/// (D-134), as `board::plan_for_feature` says. Whether it moved.
fn plan(top: &Path, t: &mut Value) -> bool {
    let Some(f) = t["feature"].as_str().and_then(|id| record(top, "features", id)) else { return false };
    board::plan_for_feature(top, t, &f)
}
/// Plans every Backlog task whose feature is `active` (D-134), as one
/// operation of the task server: run when the server starts, so tasks stored
/// before the rule move without a click.
pub fn plan_active(top: &Path) -> Result<(), Failure> {
    let root = top.to_path_buf();
    op(top, move || {
        for mut t in board::all_tasks(&root)? {
            if plan(&root, &mut t) {
                write(&root, &t)?;
            }
        }
        Ok(())
    })
}
/// A feature follows its tasks (D-128): an `active` feature whose tasks are all
/// in a final column, its Tech Debt tasks too, becomes `complete`; a `complete`
/// one with a task out of the final columns is `active` again. No other state
/// changes by itself, and a feature with no tasks stays as it is. Task `saving`
/// counts as it is about to be stored. The feature, when it changed, not yet
/// saved: `write` saves it after the task.
fn follow_tasks(top: &Path, fid: &str, saving: &Value) -> Result<Option<Value>, Failure> {
    let Some(mut f) = record(top, "features", fid) else { return Ok(None) };
    let mut held: Vec<Value> = board::all_tasks(top)?.into_iter().filter(|t| t["feature"] == fid && t["id"] != saving["id"]).collect();
    if saving["feature"] == fid {
        held.push(saving.clone());
    }
    // A task whose home has no workflow (not a project yet) is in no column: open.
    let closed =
        |t: &Value| board::flow(&scope::path(top, t["home"].as_str().unwrap_or("."))).is_ok_and(|w| board::column(&w, t)["final"] == true);
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
/// Writes `<dir>/<id>.json` as `v`, synced like an inbox message and renamed
/// into place, so a power loss never leaves an empty file.
fn replace_file(dir: &Path, id: &str, v: &Value) -> Result<(), Failure> {
    replace_bytes(dir, &format!("{id}.json"), &serde_json::to_vec_pretty(v).unwrap())
}
/// Writes `<dir>/<name>` as `bytes`, the same way.
fn replace_bytes(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), Failure> {
    fs::create_dir_all(dir)?;
    let (part, to) = (dir.join(format!("{name}.tmp")), dir.join(name));
    let mut f = fs::File::create(&part)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&part, &to)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// The states of a milestone or a feature (D-87 point 8; `upcoming` was
/// `planned` before D-128).
const STATES: [&str; 5] = ["proposed", "upcoming", "active", "complete", "canceled"];

fn strategy_dir(top: &Path, kind: &str) -> PathBuf {
    top.join(".solace/strategy").join(kind)
}
/// The event key of one record of `kind` (`milestones` or `features`).
fn singular(kind: &str) -> &'static str {
    if kind == "milestones" { "milestone" } else { "feature" }
}
fn rank_of(v: &Value) -> f64 {
    v["rank"].as_f64().unwrap_or(0.)
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
fn stored_records(top: &Path, kind: &str) -> Vec<Value> {
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
pub(super) fn record(top: &Path, kind: &str, id: &str) -> Option<Value> {
    if !board::valid_id(id) {
        return None;
    }
    if kind == "features" {
        return records(top, kind).into_iter().find(|v| v["id"] == id);
    }
    let path = strategy_dir(top, kind).join(format!("{id}.json"));
    Some(stored(kind, serde_json::from_slice(&fs::read(&path).ok()?).ok()?, &path))
}
/// A record as read from `path`. A feature saved before `startTime` has none:
/// one already active or complete takes its file's time as when it started.
/// A record saved `planned` reads as `upcoming` (D-128).
fn stored(kind: &str, mut v: Value, path: &Path) -> Value {
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
        if was.get("colorSlot").is_none() && !f["colorSlot"].is_null() && board::valid_id(id) {
            batch_replace(&strategy_dir(top, "features").join(format!("{id}.json")), &f)?;
        }
    }
    Ok(())
}
/// A feature's `startTime` is when it first became `active`; it is never
/// cleared. Its `colorSlot` is set with it, one past the highest any other
/// feature holds, and never changes (D-114).
fn mark_start(top: &Path, kind: &str, v: &mut Value) {
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
fn put_record(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    recover(top)?;
    let id = v["id"].as_str().unwrap_or("");
    let was = if kind == "features" { record(top, kind, id) } else { None };
    let moved = was.as_ref().is_some_and(|old| old["milestone"] != v["milestone"]);
    let active = kind == "features" && v["state"] == "active";
    if kind == "features" && (active || moved) {
        let mut tasks = vec![];
        for mut t in board::all_tasks(top)?.into_iter().filter(|t| t["feature"] == id) {
            let mut changed = moved;
            if moved {
                t["milestone"] = v["milestone"].clone();
            }
            if active && board::plan_for_feature(top, &mut t, v) {
                changed = true;
            }
            if changed {
                tasks.push(t);
            }
        }
        write_tasks_then(top, &tasks, Some(v), || save_record_in(top, kind, v))?;
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
fn remove_record(top: &Path, kind: &str, id: &str) -> Result<(), Failure> {
    if kind == "features" {
        keep_slots(top)?;
    }
    let dir = strategy_dir(top, kind);
    batch_remove(&dir.join(format!("{id}.json")))?;
    let event = if kind == "milestones" { json!({"removedMilestone": id}) } else { json!({"removedFeature": id}) };
    batch_log("", &event);
    Ok(())
}
/// The rank a new record takes at the end of its group: all milestones, or the
/// features of one milestone (or of none), leaving out `except`.
fn end_rank(top: &Path, kind: &str, milestone: &Value, except: &str) -> f64 {
    let group = |r: &&Value| r["id"] != except && (kind == "milestones" || r["milestone"] == *milestone);
    records(top, kind).iter().filter(group).map(rank_of).fold(0., f64::max) + 1.
}

/// `/milestones...` and `/features...` (D-87), root records: reads take nothing
/// and a change is one operation. An error comes with its HTTP status: 404 for an
/// unknown id in the path, 400 for one in the body, else 409.
pub fn strategy_api(top: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, (u16, Failure)> {
    recover(top).map_err(|e| (409, e))?;
    if method == "GET" && parts.len() == 1 {
        return Ok(json!(records(top, parts[0])));
    }
    let (root, method) = (top.to_path_buf(), method.to_string());
    let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    let done = op(top, move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let mut status = 409;
        Ok(strategy(&root, &method, &parts, &body, &mut status).map_err(|e| (status, e)))
    })
    .map_err(|e| (409, e))?;
    done
}
fn strategy(top: &Path, method: &str, parts: &[&str], req: &Value, status: &mut u16) -> Result<Value, Failure> {
    let kind = parts[0];
    match (method, parts.len()) {
        ("POST", 1) => create_record(top, kind, req, status),
        ("PATCH", 2) => edit_record(top, kind, parts[1], req, status),
        ("DELETE", 2) => delete_record(top, kind, parts[1], status),
        _ => board::err("E7001", "unknown route"),
    }
}
/// `GET /tasks/find/{id}` (D-98): the project that stores a task, whichever it
/// is, and whether it is archived, so a task link opens it on the board or, off
/// the board, on the Strategy page; and its title, which a bare link shows
/// (D-122). 404 for no such task.
pub fn find_task(top: &Path, id: &str) -> Result<Value, (u16, Failure)> {
    recover(&scope::top(top)).map_err(|e| (409, e))?;
    if !board::valid_id(id) {
        return board::err("E7001", "invalid task id").map_err(|e| (400, e));
    }
    let path = board::store_dir(top).join(format!("{id}.json"));
    if !path.exists() {
        return board::err("E7023", format!("unknown task {id}")).map_err(|e| (404, e));
    }
    let t = board::read_task(top, &path).map_err(|e| (409, e))?;
    Ok(json!({
        "id": id,
        "dir": t["home"],
        "archived": !t["archivedTime"].is_null(),
        "title": t["title"],
        "project": !board::homeless(top, t["home"].as_str().unwrap_or(".")),
    }))
}
/// `GET /features/find/{id}` and `GET /milestones/find/{id}` (D-122): a
/// record's title, which a bare link to it shows. 404 for no such record.
pub fn find_record(top: &Path, kind: &str, id: &str) -> Result<Value, (u16, Failure)> {
    recover(top).map_err(|e| (409, e))?;
    if !board::valid_id(id) {
        return board::err("E7001", format!("invalid {} id", singular(kind))).map_err(|e| (400, e));
    }
    match record(top, kind, id) {
        Some(r) => Ok(json!({"id": id, "title": r["title"]})),
        None => board::err("E7023", format!("unknown {} {id}", singular(kind))).map_err(|e| (404, e)),
    }
}
fn unknown_id<T>(kind: &str, id: &str, status: &mut u16, code: u16) -> Result<T, Failure> {
    *status = code;
    board::err("E7023", format!("unknown {} {id}", singular(kind)))
}
/// Checks the type of each field a request names.
fn check_fields(kind: &str, req: &Value) -> Result<(), Failure> {
    let link = if kind == "milestones" { "targetDate" } else { "milestone" };
    for k in ["title", "body", "state", "rank", link] {
        let Some(x) = req.get(k) else { continue };
        let ok = match k {
            "title" => x.as_str().is_some_and(|t| !t.trim().is_empty()),
            "body" => x.is_string(),
            "state" => x.as_str().is_some_and(|s| STATES.contains(&s)),
            "rank" => x.is_number(),
            _ => x.is_string() || x.is_null(),
        };
        if !ok {
            return board::err("E7001", format!("invalid {k}"));
        }
    }
    Ok(())
}
/// The milestone a feature request names, which must exist (any state).
fn linked(top: &Path, req: &Value, status: &mut u16) -> Result<Value, Failure> {
    match req.get("milestone") {
        Some(Value::String(m)) if record(top, "milestones", m).is_none() => unknown_id("milestones", m, status, 400),
        Some(m) => Ok(m.clone()),
        None => Ok(Value::Null),
    }
}
fn create_record(top: &Path, kind: &str, req: &Value, status: &mut u16) -> Result<Value, Failure> {
    let (v, new) = composed(top, kind, req, status)?;
    if new {
        put_record(top, kind, &v)?;
    }
    Ok(v)
}
/// The record a `POST` of `req` creates, checked and not yet written, and
/// whether it is new: a `request` seen before gives its record.
fn composed(top: &Path, kind: &str, req: &Value, status: &mut u16) -> Result<(Value, bool), Failure> {
    check_fields(kind, req)?;
    let Some(title) = req["title"].as_str() else { return board::err("E7001", "title required") };
    let request = match req.get("request") {
        None => None,
        Some(Value::String(r)) if !r.is_empty() => Some(r.clone()),
        Some(_) => return board::err("E7001", "request must be a string"),
    };
    if let Some(r) = &request
        && let Some(v) = records(top, kind).into_iter().find(|v| v["request"] == *r)
    {
        return Ok((v, false));
    }
    let mut v = json!({"id": unique_id(), "title": title, "body": req["body"].as_str().unwrap_or(""),
        "state": req["state"].as_str().unwrap_or("proposed")});
    if kind == "milestones" {
        v["targetDate"] = req.get("targetDate").cloned().unwrap_or(Value::Null);
    } else {
        v["milestone"] = linked(top, req, status)?;
        v["startTime"] = Value::Null;
        v["colorSlot"] = Value::Null;
        mark_start(top, kind, &mut v);
    }
    v["rank"] = json!(end_rank(top, kind, &v["milestone"], ""));
    if let Some(r) = request {
        v["request"] = json!(r);
    }
    Ok((v, true))
}
fn edit_record(top: &Path, kind: &str, id: &str, req: &Value, status: &mut u16) -> Result<Value, Failure> {
    let v = edited(top, kind, id, req, status)?;
    put_record(top, kind, &v)?;
    Ok(v)
}
/// Record `id` as a `PATCH` of `req` leaves it, checked and not yet written.
fn edited(top: &Path, kind: &str, id: &str, req: &Value, status: &mut u16) -> Result<Value, Failure> {
    let Some(mut v) = record(top, kind, id) else { return unknown_id(kind, id, status, 404) };
    check_fields(kind, req)?;
    let link = if kind == "milestones" { "targetDate" } else { "milestone" };
    if kind == "features" {
        linked(top, req, status)?;
    }
    let moved = kind == "features" && req.get("milestone").is_some_and(|m| *m != v["milestone"]);
    for k in ["title", "body", "state", "rank", link] {
        if let Some(x) = req.get(k) {
            v[k] = x.clone();
        }
    }
    // A feature that changes milestone goes to the end of its new group, unless the request ranks it.
    if moved && req.get("rank").is_none() {
        v["rank"] = json!(end_rank(top, kind, &v["milestone"], id));
    }
    mark_start(top, kind, &mut v);
    Ok(v)
}
/// Deletes a record. Its features and tasks lose their link in the same batch,
/// so a failure or crash leaves no link to a missing record (D-87 point 5).
fn delete_record(top: &Path, kind: &str, id: &str, status: &mut u16) -> Result<Value, Failure> {
    recover(top)?;
    let Some(v) = record(top, kind, id) else { return unknown_id(kind, id, status, 404) };
    let mut tasks: Vec<Value>;
    let features: Vec<Value>;
    if kind == "milestones" {
        tasks = board::all_tasks(top)?.into_iter().filter(|t| t["milestone"] == id).collect();
        for t in &mut tasks {
            t["milestone"] = Value::Null;
        }
        features = records(top, "features")
            .into_iter()
            .filter(|f| f["milestone"] == id)
            .map(|mut f| {
                f["milestone"] = Value::Null;
                f
            })
            .collect();
    } else {
        let link = singular(kind);
        tasks = board::all_tasks(top)?.into_iter().filter(|t| t[link] == id).collect();
        for t in &mut tasks {
            t[link] = Value::Null;
        }
        features = Vec::new();
    }
    write_tasks_then(top, &tasks, None, || {
        write_features(top, &features)?;
        remove_record(top, kind, id)
    })?;
    Ok(v)
}

/// The most characters (Unicode scalar values) the vision may hold (D-92).
pub const VISION_LIMIT: usize = 1000;

/// The vision's file (D-91): internal, so agents can't reach it.
fn vision_file(top: &Path) -> PathBuf {
    top.join(".solace/strategy/vision.md")
}
/// The vision as it is, `{text, revision}`; none yet reads as "".
pub fn vision(top: &Path) -> Value {
    let text = fs::read_to_string(vision_file(top)).unwrap_or_default();
    json!({"text": text, "revision": format!("sha256:{}", sha256(text.as_bytes()))})
}
/// `/vision` (D-91), at the root only. `PUT {text, base}` writes only over the
/// revision the editor started from: another is `E7026` (409), and nothing is
/// written. A malformed body is 400, and so is text over `VISION_LIMIT`
/// characters (`E7027`, D-92).
pub fn vision_api(top: &Path, method: &str, body: Value) -> Result<Value, (u16, Failure)> {
    match method {
        "GET" => Ok(vision(top)),
        "PUT" => {
            let (Some(text), Some(base)) = (body["text"].as_str(), body["base"].as_str()) else {
                return board::err("E7001", "text and base must be strings").map_err(|e| (400, e));
            };
            within_limit(text).map_err(|e| (400, e))?;
            let (root, text, base) = (top.to_path_buf(), text.to_string(), base.to_string());
            op(top, move || {
                if vision(&root)["revision"] != base.as_str() {
                    return Ok(Err((409, Failure("E7026 the vision changed since this edit began; reload it".into(), 1))));
                }
                Ok(Ok(write_vision(&root, &text)?))
            })
            .map_err(|e| (409, e))?
        }
        _ => board::err("E7001", "unknown route").map_err(|e| (409, e)),
    }
}
/// Refuses vision text over `VISION_LIMIT` characters (`E7027`, D-92).
fn within_limit(text: &str) -> Result<(), Failure> {
    let count = text.chars().count();
    if count > VISION_LIMIT {
        return board::err("E7027", format!("the vision is {count} characters; the limit is {VISION_LIMIT}"));
    }
    Ok(())
}
/// Writes the vision and logs it as a root event, inside the caller's operation.
fn write_vision(top: &Path, text: &str) -> Result<Value, Failure> {
    replace_bytes(vision_file(top).parent().unwrap(), "vision.md", text.as_bytes())?;
    let v = vision(top);
    log(top, "", &json!({"vision": v}));
    Ok(v)
}

/// Every strategy run (D-93), oldest first, without its notes: the list
/// `GET /strategy/runs` returns and a new connection's replay sends.
pub fn strategy_runs(top: &Path) -> Vec<Value> {
    let mut out = stored_records(top, "runs");
    for r in out.iter_mut().filter_map(Value::as_object_mut) {
        r.remove("notes");
    }
    out.sort_by(|a, b| a["started"].as_u64().cmp(&b["started"].as_u64()).then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    out
}
/// Writes a strategy run and logs it in full as a root event: no project.
fn put_run(top: &Path, run: &Value) -> Result<(), Failure> {
    replace_file(&strategy_dir(top, "runs"), run["id"].as_str().unwrap_or(""), run)?;
    log(top, "", &json!({"strategyRun": run}));
    Ok(())
}
/// One strategy run as its file holds it, if there is one.
pub(super) fn strategy_run(top: &Path, rid: &str) -> Option<Value> {
    record(top, "runs", rid)
}
/// Changes strategy run `rid` with `f`, read and written as one operation.
pub(super) fn update_run<R: Send + 'static>(
    top: &Path,
    rid: &str,
    f: impl FnOnce(&mut Value) -> Result<R, Failure> + Send + 'static,
) -> Result<R, Failure> {
    let (root, rid) = (top.to_path_buf(), rid.to_string());
    op(top, move || {
        let Some(mut run) = record(&root, "runs", &rid) else { return board::err("E7023", format!("unknown strategy run {rid}")) };
        let out = f(&mut run)?;
        put_run(&root, &run)?;
        Ok(out)
    })
}
/// Ends the strategy runs a previous server left going: their agents are gone.
pub fn end_strategy_runs(top: &Path) -> Result<(), Failure> {
    for run in stored_records(top, "runs").into_iter().filter(|r| r["state"] != "ended") {
        let rid = run["id"].as_str().unwrap_or("").to_string();
        update_run(top, &rid.clone(), move |r| {
            if r["state"] != "ended" {
                board::note(r, "solace", Some(&rid), "the Solace server restarted while an agent was working");
                end_strategy(r, "stopped");
            }
            Ok(())
        })?;
    }
    Ok(())
}
/// Ends strategy run `run` with `outcome`: its notes become one transcript
/// note, as a discussion's do.
pub(super) fn end_strategy(run: &mut Value, outcome: &str) {
    let (model, rid) = (run["model"].as_str().unwrap_or("").to_string(), run["id"].as_str().unwrap_or("").to_string());
    board::end_run(run, outcome);
    let turns = run["notes"].as_array_mut().map(std::mem::take).unwrap_or_default();
    if !turns.is_empty() {
        board::note(run, "solace", Some(&rid), &board::conversation(&turns, &model));
    }
}
/// `/strategy/runs...` (D-93), at the root only; `parts` are those after `runs`.
/// Reads take nothing and a change is one operation. Starting a run records it;
/// its agent is launched elsewhere, and stops when it sees the run ended. An
/// error comes with its HTTP status: 404 for an unknown id in the path, 400 for
/// a malformed body, else 409.
pub fn strategy_runs_api(top: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, (u16, Failure)> {
    runs_api(top, method, parts, body, board::system)
}
/// The system settings, with the models a run may choose: `board::system`, or
/// a stand-in in tests.
type System = fn() -> Result<Value, Failure>;
fn runs_api(top: &Path, method: &str, parts: &[&str], body: Value, system: System) -> Result<Value, (u16, Failure)> {
    match (method, parts) {
        ("GET", []) => return Ok(json!(strategy_runs(top))),
        ("GET", [rid]) => {
            let Some(mut run) = record(top, "runs", rid) else {
                return board::err("E7023", format!("unknown strategy run {rid}")).map_err(|e| (404, e));
            };
            run["transcript"] = json!(board::transcript(Path::new(run["log"].as_str().unwrap_or(""))));
            return Ok(run);
        }
        _ => {}
    }
    let (root, method) = (top.to_path_buf(), method.to_string());
    let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    op(top, move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let mut status = 409;
        Ok(change_run(&root, &method, &parts, &body, system, &mut status).map_err(|e| (status, e)))
    })
    .map_err(|e| (409, e))?
}
fn change_run(top: &Path, method: &str, parts: &[&str], req: &Value, system: System, status: &mut u16) -> Result<Value, Failure> {
    if method == "POST" && parts.is_empty() {
        return start_run(top, req, system, status);
    }
    let rid = parts.first().copied().unwrap_or("");
    let Some(mut run) = record(top, "runs", rid) else {
        *status = 404;
        return board::err("E7023", format!("unknown strategy run {rid}"));
    };
    let open = run["state"] != "ended";
    match (method, &parts[1..]) {
        ("POST", ["notes"]) => {
            let Some(text) = req["text"].as_str().filter(|t| !t.trim().is_empty()) else {
                *status = 400;
                return board::err("E7001", "a note needs text");
            };
            if !open {
                return board::err("E7003", "the strategy run has ended");
            }
            board::note(&mut run, "human", Some(rid), text);
        }
        // An approved proposal is applied in this same operation (D-93), and every
        // answer to one is noted on the run.
        ("POST", ["questions", qid]) => {
            let Some(q) = run["questions"].as_array_mut().and_then(|qs| qs.iter_mut().find(|q| q["id"] == *qid)) else {
                *status = 404;
                return board::err("E7023", format!("unknown question {qid}"));
            };
            let Some(answer) = req.get("text").or_else(|| req.get("option")).and_then(Value::as_str) else {
                *status = 400;
                return board::err("E7001", "answer required");
            };
            let chosen = req.get("text").is_none();
            if chosen && !q["options"].as_array().is_some_and(|o| o.contains(&json!(answer))) {
                *status = 400;
                return board::err("E7001", "option is not offered");
            }
            if !q["answer"].is_null() {
                return board::err("E7003", "the question is already answered");
            }
            if !open {
                return board::err("E7003", "the strategy run has ended");
            }
            let mut answer = answer.to_string();
            if !q["proposal"].is_null() {
                if chosen && answer == "Approve" {
                    answer = apply_proposal(top, rid, &q["proposal"]).unwrap_or_else(|e| format!("Not applied: {}", e.0));
                } else if !chosen {
                    answer = format!("Not applied; the human answered: {answer}");
                }
            }
            q["answer"] = json!(answer);
            if !q["proposal"].is_null() {
                let what = q["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
                board::note(&mut run, "solace", Some(rid), &format!("Proposal: {what}\n\nAnswer: {answer}"));
            }
        }
        ("POST", ["end" | "stop"]) => {
            if !open {
                return board::err("E7003", "the strategy run has ended");
            }
            end_strategy(&mut run, if parts[1] == "end" { "planned" } else { "stopped" });
        }
        _ => return board::err("E7001", "unknown route"),
    }
    put_run(top, &run)?;
    Ok(run)
}
/// Starts a strategy run: its first message is its first note. One open run per
/// context; a context names the page, or a task, feature or milestone that exists.
fn start_run(top: &Path, req: &Value, system: System, status: &mut u16) -> Result<Value, Failure> {
    *status = 400;
    let Some(text) = req["text"].as_str().filter(|t| !t.trim().is_empty()) else {
        return board::err("E7001", "a strategy run starts with a message");
    };
    let c = &req["context"];
    let context = match (c["kind"].as_str(), c["id"].as_str()) {
        (Some("page"), _) => json!({"kind": "page"}),
        (Some("task"), Some(id)) if board::all_tasks(top)?.iter().any(|t| t["id"] == id) => json!({"kind": "task", "id": id}),
        (Some("task"), Some(id)) => return board::err("E7001", format!("unknown task {id}")),
        (Some(kind @ ("feature" | "milestone")), Some(id)) => {
            if record(top, &format!("{kind}s"), id).is_none() {
                return board::err("E7023", format!("unknown {kind} {id}"));
            }
            json!({"kind": kind, "id": id})
        }
        _ => return board::err("E7001", "context must be {kind: page} or {kind: task|feature|milestone, id}"),
    };
    let m = board::chosen_model(&system()?, req).map_err(|e| {
        if e.0.starts_with("E7011") {
            *status = 409;
        }
        e
    })?;
    *status = 409;
    if let Some(open) = stored_records(top, "runs").iter().find(|r| r["state"] != "ended" && r["context"] == context) {
        return board::err("E7003", format!("strategy run {} on this context is open", open["id"].as_str().unwrap_or("")));
    }
    let id = unique_id();
    let log = strategy_dir(top, "runs").join(format!("{id}.jsonl"));
    let mut run = json!({"id": id, "kind": "strategy", "model": m["id"], "effort": m["effort"], "state": "running", "started": now(),
        "log": log, "context": context, "notes": [], "questions": []});
    board::note(&mut run, "human", Some(&id), text);
    put_run(top, &run)?;
    Ok(run)
}

/// What an approved strategy proposal (D-93) writes, built as its request
/// would be and checked before anything is written.
enum Built {
    /// A feature or milestone (`kind` "features" or "milestones"), and whether it is new.
    Record(&'static str, Value, bool),
    /// A task, and whether it is new.
    Task(Value, bool),
    Vision(String),
}
/// Builds proposal `p` of strategy run `rid` against the records as they are
/// now, writing nothing: a feature or milestone as `POST`/`PATCH` would, a task
/// as `POST /tasks` or its `PATCH` would (with a `solace` note naming the run),
/// or the vision's new text.
fn build(top: &Path, rid: &str, p: &Value) -> Result<Built, Failure> {
    let (fields, id) = (&p["fields"], p["id"].as_str());
    let mut status = 409;
    match p["kind"].as_str().unwrap_or("") {
        kind @ ("feature" | "milestone") => {
            let plural = if kind == "feature" { "features" } else { "milestones" };
            Ok(match id {
                Some(id) => Built::Record(plural, edited(top, plural, id, fields, &mut status)?, false),
                None => {
                    let (v, new) = composed(top, plural, fields, &mut status)?;
                    Built::Record(plural, v, new)
                }
            })
        }
        "task" => match id {
            Some(id) => {
                let Some(t) = board::all_tasks(top)?.into_iter().find(|t| t["id"] == id) else {
                    return board::err("E7023", format!("unknown task {id}"));
                };
                let home = board::task_root(top, &t);
                let (mut t, _) = board::change(&home, id, "PATCH", &[], fields)?;
                board::note(&mut t, "solace", None, &format!("Changed by strategy run {rid}"));
                Ok(Built::Task(t, false))
            }
            None => Ok(Built::Task(proposed_task(top, rid, fields)?, true)),
        },
        "vision" => {
            let Some(text) = fields["text"].as_str() else { return board::err("E7001", "a vision proposal needs its text") };
            within_limit(text)?;
            Ok(Built::Vision(text.to_string()))
        }
        _ => board::err("E7001", "a proposal is for a task, feature, milestone or the vision"),
    }
}
/// The task a strategy proposal creates: `POST /tasks` in the first project of
/// its `scope` (the root's own without one), with `feature`, `milestone` and
/// `dependsOn` set as `PATCH` sets them. Every folder in `scope` must be a
/// project (D-129): a proposal naming one that isn't is refused here, when the
/// proposal is sent, before the human is asked.
fn proposed_task(top: &Path, rid: &str, fields: &Value) -> Result<Value, Failure> {
    let folders: Vec<String> = match fields.get("scope") {
        None => vec![".".into()],
        Some(v) => {
            let ok = |d: &str| !d.is_empty() && !d.starts_with('/') && !d.split('/').any(|p| p == "..");
            let named: Option<Vec<String>> =
                v.as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|d| ok(d)).map(String::from).collect());
            match named {
                Some(n) if !n.is_empty() && n.len() == v.as_array().unwrap().len() => n,
                _ => return board::err("E7001", "scope must be a list of project folders below the root"),
            }
        }
    };
    let members = workspace::members(top)?;
    if let Some(d) = folders.iter().find(|d| !members.contains(d)) {
        return board::err("E7001", format!("{d} is not a project; scope takes project folders: {}", members.join(", ")));
    }
    let mut body = fields.clone();
    body["scope"] = json!(folders);
    let mut t = board::created(&scope::path(top, &folders[0]), &body)?;
    board::note(&mut t, "solace", None, &format!("Created by strategy run {rid}"));
    board::set_links(top, &mut t, fields)?;
    Ok(t)
}
/// Id `id` without its link prefix: an agent copies `task:<id>` from a reply (D-148)
/// where a plain id is asked for.
pub(super) fn bare_id(id: &str) -> &str {
    ["task:", "feature:", "milestone:"].iter().find_map(|p| id.strip_prefix(p)).unwrap_or(id)
}

/// A proposal's fields with each linked id bare: `feature`, `milestone` and `dependsOn`.
pub(super) fn bare_links(mut fields: Value) -> Value {
    for k in ["feature", "milestone"] {
        if let Some(id) = fields[k].as_str() {
            fields[k] = json!(bare_id(id));
        }
    }
    if let Some(deps) = fields.get_mut("dependsOn").and_then(Value::as_array_mut) {
        for d in deps.iter_mut() {
            if let Some(id) = d.as_str() {
                *d = json!(bare_id(id));
            }
        }
    }
    fields
}

/// A strategy run's proposal (D-93) as the question its agent waits on:
/// checked as its request would be, writing nothing. `fields` is the JSON
/// object of the request, or for the vision its new text. The question names
/// the `complete` or `canceled` feature and milestone a task proposal links to,
/// which Approve also sets `upcoming`.
pub(super) fn proposal(top: &Path, rid: &str, kind: &str, id: Option<&str>, text: &str) -> Result<Value, Failure> {
    let fields = if kind == "vision" {
        json!({"text": text})
    } else {
        match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => v,
            _ => return board::err("E7001", "the proposal's fields go in `text`, as a JSON object"),
        }
    };
    let fields = bare_links(fields);
    let id = id.map(bare_id);
    // D-105: a field the request doesn't take is refused, so a misspelt one isn't lost.
    if kind != "vision" {
        let allowed: &[&str] = match (kind, id.is_some()) {
            ("task", false) => &["title", "body", "scope", "feature", "milestone", "dependsOn"],
            ("task", true) => &["title", "body", "feature", "milestone", "dependsOn", "rank"],
            ("feature", false) => &["title", "body", "milestone", "state"],
            ("feature", true) => &["title", "body", "milestone", "state", "rank"],
            ("milestone", false) => &["title", "body", "targetDate", "state"],
            _ => &["title", "body", "targetDate", "state", "rank"],
        };
        if let Some(k) = fields.as_object().into_iter().flat_map(|o| o.keys()).find(|k| !allowed.contains(&k.as_str())) {
            let edit = if id.is_some() { " that edits" } else { "" };
            return board::err("E7001", format!("a {kind} proposal{edit} takes {}; not `{k}`", allowed.join(", ")));
        }
    }
    let mut p = json!({"kind": kind, "fields": fields});
    // D-115: the record's current value of each proposed field, as the human
    // sees only what changes. The vision's is its text.
    let current = match (kind, id) {
        ("vision", _) => Some(vision(top)),
        (_, Some(id)) => match kind {
            "task" => board::all_tasks(top)?.into_iter().find(|t| t["id"] == id),
            k => record(top, &format!("{k}s"), id),
        },
        (_, None) => None,
    };
    if let Some(id) = id {
        p["id"] = json!(id);
    }
    if let Some(r) = &current {
        let mut before = serde_json::Map::new();
        for k in p["fields"].as_object().into_iter().flat_map(|o| o.keys()) {
            before.insert(k.clone(), r.get(k).cloned().unwrap_or(Value::Null));
        }
        p["before"] = Value::Object(before);
    }
    if kind == "vision" {
        p["base"] = vision(top)["revision"].clone();
    }
    let built = build(top, rid, &p)?;
    let mut reopens = vec![];
    let mut says = String::new();
    if let Built::Task(t, _) = &built {
        let milestone = if fields.get("milestone").is_some() || fields.get("feature").is_some() { &t["milestone"] } else { &Value::Null };
        for (kind, id) in [("feature", &fields["feature"]), ("milestone", milestone)] {
            let Some(r) = id.as_str().and_then(|id| record(top, &format!("{kind}s"), id)) else { continue };
            if matches!(r["state"].as_str(), Some("complete" | "canceled")) {
                says += &format!(
                    "\n\nApproving also sets the {kind} {} (\"{}\", now {}) back to upcoming.",
                    r["id"].as_str().unwrap_or(""),
                    r["title"].as_str().unwrap_or(""),
                    r["state"].as_str().unwrap_or("")
                );
                reopens.push(json!({"kind": kind, "id": r["id"]}));
            }
        }
    }
    if !reopens.is_empty() {
        p["reopens"] = json!(reopens);
    }
    let title = fields["title"].as_str().map(|t| format!(" \"{t}\"")).unwrap_or_default();
    let what = match (kind, id) {
        ("vision", _) => format!("Replace the vision\n\n{text}"),
        (_, Some(id)) => format!("Change {kind} {id}{title}\n\n```json\n{}\n```", serde_json::to_string_pretty(&fields).unwrap()),
        (_, None) => format!("Create {kind}{title}\n\n```json\n{}\n```", serde_json::to_string_pretty(&fields).unwrap()),
    };
    Ok(json!({"id": unique_id(), "text": format!("{what}{says}"), "options": ["Approve", "Reject"], "proposal": p, "answer": null}))
}
/// Applies strategy run `rid`'s approved proposal `p` as its request would be
/// applied (D-93), with the same refusals. Everything is built and checked
/// first, the vision's base among it, so a refused apply writes nothing. The
/// answer to record: what was applied.
fn apply_proposal(top: &Path, rid: &str, p: &Value) -> Result<String, Failure> {
    if p["kind"] == "vision" && vision(top)["revision"] != p["base"] {
        return board::err("E7026", "the vision changed since the proposal; it was not replaced");
    }
    let built = build(top, rid, p)?;
    let mut status = 409;
    let mut reopened = vec![];
    for r in p["reopens"].as_array().into_iter().flatten() {
        let plural = if r["kind"] == "milestone" { "milestones" } else { "features" };
        reopened.push((plural, edited(top, plural, r["id"].as_str().unwrap_or(""), &json!({"state": "upcoming"}), &mut status)?));
    }
    // Set once all are checked, and before the task is written, so the task
    // never sees its feature reopen as `active` and is planned (D-134).
    for (plural, v) in reopened {
        put_record(top, plural, &v)?;
    }
    let answer = match built {
        Built::Record(plural, v, new) => {
            put_record(top, plural, &v)?;
            let created = format!(": created {}:{}", singular(plural), v["id"].as_str().unwrap_or(""));
            format!("Approved and applied{}", if new { created.as_str() } else { "" })
        }
        Built::Task(t, new) => {
            write(top, &t)?;
            format!(
                "Approved and applied{}",
                if new { format!(": created task:{}", t["id"].as_str().unwrap_or("")) } else { String::new() }
            )
        }
        Built::Vision(text) => {
            write_vision(top, &text)?;
            "Approved and applied".into()
        }
    };
    Ok(answer)
}

/// The task log `/events` reads: every change in order, with the project it
/// is shown under.
pub fn events(top: &Path) -> Result<Connection, Failure> {
    fs::create_dir_all(board::store_dir(top))?;
    let db = Connection::open(board::store_dir(top).join("events.db"))?;
    db.busy_timeout(Duration::from_secs(10))?;
    db.execute_batch("PRAGMA journal_mode=WAL;")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY, project TEXT NOT NULL, data TEXT NOT NULL)")?;
    Ok(db)
}
/// Appends `data` to the task log. The file or removal it reports is already
/// done, so a failed append doesn't fail the change (a client would retry it and
/// repeat a note or action): the board catches up from the files when it next
/// connects (D-78).
fn log(top: &Path, project: &str, data: &Value) {
    let appended = events(top).and_then(|db| {
        db.execute("INSERT INTO events(project,data) VALUES (?1,?2)", params![project, data.to_string()])?;
        Ok(())
    });
    if let Err(e) = appended {
        eprintln!("the task log did not take a change: {}", e.0);
    }
}

fn inbox(top: &Path) -> PathBuf {
    board::store_dir(top).join("inbox")
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
fn server_lock(top: &Path) -> PathBuf {
    board::store_dir(top).join("server.lock")
}
/// Whether a server is running for the root, so a message is applied soon.
pub fn serving(top: &Path) -> bool {
    server_for(top).is_some() || try_lock_file(&server_lock(top)).is_ok_and(|f| f.is_none())
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
        match try_lock_file(&server_lock(top)) {
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
fn flag_set_aside(top: &Path, m: &Value, why: &str) {
    let Some(tid) = m["task"].as_str().filter(|t| board::valid_id(t)) else { return };
    let path = board::store_dir(top).join(format!("{tid}.json"));
    let Ok(mut t) = board::read_task(top, &path) else { return };
    let kind = m["kind"].as_str().unwrap_or("");
    let text = format!("A task message ({kind}) could not be applied and was set aside in .solace/tasks/inbox/failed: {why}");
    let Ok(w) = board::flow(&scope::path(top, t["home"].as_str().unwrap_or("."))) else { return };
    let note = json!({"kind":"note","author":"solace","text":text,"flag":"a task message was set aside"});
    if t["notes"].is_array() && board::message(&w, &mut t, &note).is_ok() {
        let _ = write(top, &t);
    }
}
fn failed_dir(top: &Path) -> PathBuf {
    inbox(top).join("failed")
}
/// Whether the server set the message at `path` aside because it could not apply it.
pub fn rejected(top: &Path, path: &Path) -> bool {
    path.file_name().is_some_and(|n| failed_dir(top).join(n).exists())
}
/// Settles task `tid`, which a message just moved into a final column. On the
/// server's thread that is a step of its own (D-77 point 5): settling takes
/// project locks, and a holder of those may be waiting on this thread. Elsewhere
/// (no server, as tests) it runs here.
fn settle_later(top: &Path, tid: String) {
    let on_server = server_for(top).is_some_and(|s| s.thread == std::thread::current().id());
    let top = top.to_path_buf();
    let run = move || {
        if let Err(e) = board::settle_final(&top, &tid) {
            eprintln!("settling task {tid}: {}", e.0);
        }
    };
    if on_server {
        std::thread::spawn(run);
    } else {
        run();
    }
}
/// Applies a message; the task's id when it moved the task into a final column.
fn apply(top: &Path, m: &Value) -> Result<Option<String>, Failure> {
    let id = m["id"].as_str().unwrap_or("");
    if m["kind"] == "create" {
        let t = &m["new"];
        let tid = t["id"].as_str().unwrap_or("");
        if board::store_dir(top).join(format!("{tid}.json")).exists() {
            return Ok(None);
        }
        let mut t = t.clone();
        t["inbox"] = json!(id);
        return write(top, &t).map(|_| None);
    }
    // A strategy run's agent (D-93): its reply or proposal waits for the human,
    // whose next note or answer sets the run going again.
    if let Some(kind @ ("strategy_note" | "strategy_propose" | "strategy_resume")) = m["kind"].as_str() {
        let rid = m["run"].as_str().unwrap_or("");
        let Some(mut run) = record(top, "runs", rid) else { return board::err("E7023", format!("unknown strategy run {rid}")) };
        if run["inbox"].as_str().is_some_and(|last| last == id) {
            return Ok(None);
        }
        let ended = run["state"] == "ended";
        match kind {
            "strategy_note" => board::note(&mut run, "agent", Some(rid), m["text"].as_str().unwrap_or("")),
            // A proposal can no longer be answered once the run has ended.
            "strategy_propose" if ended => {
                let what = m["question"]["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
                board::note(&mut run, "solace", Some(rid), &format!("A proposal arrived after the run ended, so it was not asked: {what}"));
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
            run["state"] = json!(if kind == "strategy_resume" { "running" } else { "waiting" });
        }
        run["inbox"] = json!(id);
        put_run(top, &run)?;
        return Ok(None);
    }
    let tid = m["task"].as_str().unwrap_or("");
    // The id names the task's file, so one that could leave the store is refused.
    if !board::valid_id(tid) {
        return board::err("E7001", "invalid task id");
    }
    let path = board::store_dir(top).join(format!("{tid}.json"));
    let mut t = board::read_task(top, &path)?;
    // Only the message applied just before a crash can come again; a message with
    // an older id may still be new (it was sent before a later one landed).
    if t["inbox"].as_str().is_some_and(|last| last == id) {
        return Ok(None);
    }
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let w = board::flow(&home)?;
    let was_final = board::column(&w, &t)["final"] == true;
    board::message(&w, &mut t, m)?;
    t["inbox"] = json!(id);
    write(top, &t)?;
    Ok((!was_final && board::column(&w, &t)["final"] == true).then(|| tid.to_string()))
}

/// `/tasks/...` for the project `root`. Reads take nothing; a change is one
/// operation, and a create with a `request` seen before returns its task
/// (D-77 point 6).
pub fn api(root: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, Failure> {
    api_with(root, method, parts, body, board::needs_lock)
}
/// Whether a change takes its task's projects' locks: `board::needs_lock`, or
/// a stand-in in tests.
type NeedsLock = fn(&Path, &[&str], &Value) -> Result<bool, Failure>;
/// `api`, with `needs` deciding whether a change takes the locks.
fn api_with(root: &Path, method: &str, parts: &[&str], body: Value, needs: NeedsLock) -> Result<Value, Failure> {
    recover(&scope::top(root))?;
    if method == "GET" {
        return board::read_task_api(root, parts);
    }
    // Only a change to code takes its projects' locks (D-77 point 5), so a
    // task-only change never waits on an agent's command.
    let mut guard = if needs(root, parts, &body)? { Some(board::guard(root, parts.to_vec(), &body)?) } else { None };
    if parts.len() == 1 {
        let (top, root) = (scope::top(root), root.to_path_buf());
        let served = top.clone();
        let request = match body.get("request") {
            None => None,
            Some(Value::String(r)) if !r.is_empty() => Some(r.clone()),
            Some(_) => return board::err("E7001", "request must be a string"),
        };
        return op(&served, move || {
            if let Some(r) = &request
                && let Some(t) = board::all_tasks(&top)?.into_iter().find(|t| t["request"] == *r)
            {
                return Ok(t);
            }
            let mut t = board::created(&root, &body)?;
            if let Some(r) = request {
                t["request"] = json!(r);
            }
            write(&top, &t)?;
            Ok(t)
        });
    }
    let (t, after) = match change(root, method, parts, &body, guard.is_some(), needs)? {
        Some(done) => done,
        // The task gained changes after the check: take the locks and run it
        // again, keeping them whatever it needs now, so it retries once.
        None => {
            guard = Some(board::guard(root, parts.to_vec(), &body)?);
            change(root, method, parts, &body, true, needs)?.ok_or_else(|| Failure("the task's projects were not locked".into(), 1))?
        }
    };
    board::after(&board::task_root(&scope::top(root), &t), &t, after);
    drop(guard);
    Ok(t)
}
/// One change to a task, as an operation. Without its projects' locks
/// (`held` false) it checks again inside the operation, where nothing else
/// writes the task, and returns None, changing nothing, when the change now
/// needs them (`needs` says). The operation never takes a lock itself (see `op`).
fn change(
    root: &Path,
    method: &str,
    parts: &[&str],
    body: &Value,
    held: bool,
    needs: NeedsLock,
) -> Result<Option<(Value, board::After)>, Failure> {
    let (root, method, body, top) = (root.to_path_buf(), method.to_string(), body.clone(), scope::top(root));
    let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    let served = top.clone();
    op(&served, move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        if !held && needs(&root, &parts, &body)? {
            return Ok(None);
        }
        let tid = parts[1];
        let (t, after) = board::change(&root, tid, &method, &parts[2..], &body)?;
        // The feature's Tech Debt task first (D-112): its notes name this task.
        if let board::After::Clean(Some(debt)) = &after {
            write(&top, debt)?;
        }
        write(&top, &t)?;
        if let board::After::Removed(dir) = &after {
            log(&top, dir, &json!({"removed":tid}));
        }
        Ok(Some((t, after)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root whose task store holds t1, in `backlog`.
    fn root() -> testing::Scratch {
        let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#)]);
        board::open(&root).unwrap();
        board::adopt_tasks(&root, &root).unwrap();
        root
    }
    fn stored(root: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(board::store_dir(root)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        names.sort();
        names
    }

    /// The id names the task's file, so one that could leave the store is refused.
    #[test]
    fn write_refuses_an_id_that_is_not_a_plain_name() {
        let root = root();
        let before = stored(&root);
        for bad in [json!(""), json!("../x"), json!("a/b"), json!("a b"), json!("a.b"), json!(1), Value::Null] {
            let e = write(&root, &json!({"id": bad, "home": "."})).unwrap_err();
            assert!(e.0.contains("E7001") && e.0.contains("invalid task id"), "{bad}: {}", e.0);
        }
        assert_eq!(stored(&root), before);
        assert!(!root.join(".solace/x.json").exists());
        write(&root, &json!({"id": "ab-12", "home": "."})).unwrap();
        assert!(stored(&root).contains(&"ab-12.json".to_string()));
        assert!(!stored(&root).iter().any(|n| n.ends_with(".tmp")), "the temporary file is renamed");
    }

    /// A create applied again (a replay after a crash) leaves the first one.
    #[test]
    fn a_create_message_applies_once() {
        let root = root();
        let mut t = board::new_task("New", "", &json!("backlog"), 1.);
        t["home"] = json!(".");
        let tid = t["id"].as_str().unwrap().to_string();
        assert_eq!(apply(&root, &json!({"id": "m1", "kind": "create", "new": t})).unwrap(), None);
        let first = board::task(&root, &tid).unwrap();
        assert_eq!((first["title"].as_str(), first["inbox"].as_str()), (Some("New"), Some("m1")));
        t["title"] = json!("Again");
        assert_eq!(apply(&root, &json!({"id": "m2", "kind": "create", "new": t})).unwrap(), None);
        assert_eq!(board::task(&root, &tid).unwrap(), first);
    }

    /// D-77 point 6: a create with a `request` seen before returns its task.
    #[test]
    fn a_create_with_a_known_request_returns_its_task() {
        let root = root();
        let post = |body: Value| api(&root, "POST", &["tasks"], body);
        let a = post(json!({"title": "A", "request": "req-1"})).unwrap();
        assert_eq!(a["request"], "req-1");
        let again = post(json!({"title": "A, resent", "request": "req-1"})).unwrap();
        assert_eq!(again["id"], a["id"]);
        assert_eq!(board::all_tasks(&root).unwrap().len(), 2);
        // Without a request, each create is a new task.
        let (b, c) = (post(json!({"title": "B"})).unwrap(), post(json!({"title": "B"})).unwrap());
        assert_ne!(b["id"], c["id"]);
        assert!(b.get("request").is_none_or(Value::is_null));
        for bad in [json!(""), json!(5), json!(null)] {
            let e = post(json!({"title": "C", "request": bad})).unwrap_err();
            assert!(e.0.contains("request must be a string"), "{bad}: {}", e.0);
        }
        assert_eq!(board::all_tasks(&root).unwrap().len(), 4);
    }

    /// D-87: milestones and features are created at the end of their group, edited,
    /// reordered and deleted; a missing folder holds none.
    #[test]
    fn milestones_and_features_are_created_edited_reordered_and_deleted() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body);
        let titles = |kind: &str| -> Vec<String> { records(&root, kind).iter().map(|r| r["title"].as_str().unwrap().into()).collect() };
        assert_eq!(call("GET", &["milestones"], json!({})).unwrap(), json!([]));
        assert_eq!(call("GET", &["features"], json!({})).unwrap(), json!([]));
        let a = call("POST", &["milestones"], json!({"title": "1.0"})).unwrap();
        let b = call("POST", &["milestones"], json!({"title": "2.0", "state": "upcoming", "targetDate": "2027-01-01"})).unwrap();
        assert_eq!(
            (a["state"].as_str(), b["state"].as_str(), b["targetDate"].as_str()),
            (Some("proposed"), Some("upcoming"), Some("2027-01-01"))
        );
        assert_eq!((a["rank"].as_f64(), b["rank"].as_f64()), (Some(1.), Some(2.)));
        let (aid, bid) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
        let edited = call("PATCH", &["milestones", aid], json!({"title": "1.0 final", "rank": 3})).unwrap();
        assert_eq!((edited["title"].as_str(), edited["state"].as_str()), (Some("1.0 final"), Some("proposed")));
        assert_eq!(titles("milestones"), ["2.0", "1.0 final"], "a new rank reorders");
        let e = call("PATCH", &["milestones", aid], json!({"state": "done"})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        assert_eq!(record(&root, "milestones", aid).unwrap(), edited, "a refused edit writes nothing");

        // Features rank within their milestone, or among those with none.
        let feature = |title: &str, milestone: Value| call("POST", &["features"], json!({"title": title, "milestone": milestone})).unwrap();
        let (f1, f2, f3) = (feature("F1", json!(aid)), feature("F2", json!(aid)), feature("F3", Value::Null));
        assert_eq!((f1["rank"].as_f64(), f2["rank"].as_f64(), f3["rank"].as_f64()), (Some(1.), Some(2.), Some(1.)));
        assert_eq!((f1["state"].as_str(), f3["milestone"].is_null()), (Some("proposed"), true));
        let (f1id, f2id, f3id) = (f1["id"].as_str().unwrap(), f2["id"].as_str().unwrap(), f3["id"].as_str().unwrap());
        // Moving to another milestone puts a feature at the end of the new group, unless it is ranked.
        let moved = call("PATCH", &["features", f1id], json!({"milestone": bid})).unwrap();
        assert_eq!((moved["milestone"].as_str(), moved["rank"].as_f64()), (Some(bid), Some(1.)));
        let moved = call("PATCH", &["features", f2id], json!({"milestone": bid})).unwrap();
        assert_eq!(moved["rank"].as_f64(), Some(2.));
        let ranked = call("PATCH", &["features", f2id], json!({"milestone": aid, "rank": 7})).unwrap();
        assert_eq!(ranked["rank"].as_f64(), Some(7.));
        let canceled = call("PATCH", &["features", f3id], json!({"state": "canceled", "body": "no"})).unwrap();
        assert_eq!((canceled["state"].as_str(), canceled["body"].as_str()), (Some("canceled"), Some("no")));
        // A feature's startTime is set when it first becomes active, and kept after.
        assert!(f1["startTime"].is_null() && canceled["startTime"].is_null());
        let active = call("PATCH", &["features", f1id], json!({"state": "active"})).unwrap();
        let started = active["startTime"].as_u64().expect("set on becoming active");
        let done = call("PATCH", &["features", f1id], json!({"state": "complete"})).unwrap();
        let again = call("PATCH", &["features", f1id], json!({"state": "active", "startTime": 1, "colorSlot": 9})).unwrap();
        assert_eq!((done["startTime"].as_u64(), again["startTime"].as_u64()), (Some(started), Some(started)));
        let born = call("POST", &["features"], json!({"title": "F4", "state": "active"})).unwrap();
        assert!(born["startTime"].as_u64().is_some(), "created active");
        // D-114: each started feature keeps a color slot, one past the highest; a delete moves no other's.
        assert!(f1["colorSlot"].is_null() && canceled["colorSlot"].is_null());
        let f2on = call("PATCH", &["features", f2id], json!({"state": "active"})).unwrap();
        let slots = (active["colorSlot"].as_u64(), again["colorSlot"].as_u64(), born["colorSlot"].as_u64(), f2on["colorSlot"].as_u64());
        assert_eq!(slots, (Some(0), Some(0), Some(1), Some(2)));
        call("DELETE", &["features", born["id"].as_str().unwrap()], json!({})).unwrap();
        assert_eq!(record(&root, "features", f2id).unwrap()["colorSlot"].as_u64(), Some(2));

        call("DELETE", &["features", f3id], json!({})).unwrap();
        assert_eq!(titles("features"), ["F1", "F2"]);
        call("DELETE", &["milestones", bid], json!({})).unwrap();
        assert_eq!(titles("milestones"), ["1.0 final"]);
        assert!(!strategy_dir(&root, "milestones").join(format!("{bid}.json")).exists());
        assert!(!strategy_dir(&root, "features").read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    /// D-114: a started feature saved before color slots were kept takes one
    /// after every kept slot, in start order, and keeps it from the next write.
    #[test]
    fn an_old_started_feature_takes_a_slot_after_the_kept_ones() {
        let root = root();
        let dir = strategy_dir(&root, "features");
        let put = |v: Value| replace_file(&dir, v["id"].as_str().unwrap(), &v).unwrap();
        put(json!({"id": "old2", "title": "O2", "state": "active", "rank": 1, "startTime": 20}));
        put(json!({"id": "old1", "title": "O1", "state": "complete", "rank": 2, "startTime": 10}));
        put(json!({"id": "kept", "title": "K", "state": "active", "rank": 3, "startTime": 30, "colorSlot": 3}));
        put(json!({"id": "idle", "title": "I", "state": "proposed", "rank": 4}));
        let slot = |id: &str| record(&root, "features", id).unwrap()["colorSlot"].as_u64();
        assert_eq!((slot("old1"), slot("old2"), slot("kept"), slot("idle")), (Some(4), Some(5), Some(3), None));
        let born = strategy_api(&root, "POST", &["features"], json!({"title": "New", "state": "active"})).unwrap();
        assert_eq!(born["colorSlot"].as_u64(), Some(6));
        let saved: Value = serde_json::from_slice(&fs::read(dir.join("old1.json")).unwrap()).unwrap();
        assert_eq!(saved["colorSlot"].as_u64(), Some(4), "saved at the next write");
        strategy_api(&root, "DELETE", &["features", born["id"].as_str().unwrap()], json!({})).unwrap();
        strategy_api(&root, "DELETE", &["features", "kept"], json!({})).unwrap();
        assert_eq!((slot("old1"), slot("old2"), slot("idle")), (Some(4), Some(5), None));
    }

    /// D-87: deleting a milestone clears the link on its features; an unknown
    /// milestone is refused with E7023 (400 in a body, 404 in a path) and writes nothing.
    #[test]
    fn a_deleted_milestone_unlinks_its_features_and_an_unknown_one_is_refused() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body);
        let m = call("POST", &["milestones"], json!({"title": "M"})).unwrap();
        let mid = m["id"].as_str().unwrap();
        let f = call("POST", &["features"], json!({"title": "F", "milestone": mid, "request": "r1"})).unwrap();
        let again = call("POST", &["features"], json!({"title": "F again", "milestone": mid, "request": "r1"})).unwrap();
        assert_eq!(again["id"], f["id"], "a known request returns its record");
        assert_eq!(records(&root, "features").len(), 1);

        let e = call("POST", &["features"], json!({"title": "G", "milestone": "nope"})).unwrap_err();
        assert_eq!(e.0, 400);
        assert!(e.1.0.starts_with("E7023"), "{}", e.1.0);
        let e = call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": "nope"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7023")), (400, true));
        let e = call("PATCH", &["milestones", "nope"], json!({"title": "x"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7023")), (404, true));
        let e = call("DELETE", &["features", "../x"], json!({})).unwrap_err();
        assert_eq!(e.0, 404);
        assert_eq!(records(&root, "features"), [f.clone()], "nothing was written");

        let deleted = call("DELETE", &["milestones", mid], json!({})).unwrap();
        assert_eq!(deleted["id"], m["id"]);
        let kept = records(&root, "features");
        assert_eq!((kept.len(), kept[0]["milestone"].is_null(), kept[0]["title"].as_str()), (1, true, Some("F")));
        // Each change was logged without a project, in order.
        let rows: Vec<String> = events(&root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let keys: Vec<String> =
            rows.iter().map(|d| serde_json::from_str::<Value>(d).unwrap().as_object().unwrap().keys().next().unwrap().clone()).collect();
        assert_eq!(keys, ["milestone", "feature", "feature", "removedMilestone"]);
    }

    /// D-91: the vision starts empty; a put over the current revision writes and
    /// logs it, and one over an older revision is E7026 and writes nothing.
    #[test]
    fn a_vision_put_over_a_stale_revision_is_refused() {
        let root = root();
        let empty = vision_api(&root, "GET", json!({})).unwrap();
        assert_eq!(empty["text"], "");
        let base = empty["revision"].as_str().unwrap().to_string();
        let saved = vision_api(&root, "PUT", json!({"text": "# Why\nFor teams.", "base": base})).unwrap();
        assert_eq!(saved["text"], "# Why\nFor teams.");
        assert_ne!(saved["revision"].as_str(), Some(base.as_str()));
        assert_eq!(fs::read_to_string(vision_file(&root)).unwrap(), "# Why\nFor teams.");

        let e = vision_api(&root, "PUT", json!({"text": "lost", "base": base})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7026")), (409, true), "{}", e.1.0);
        assert_eq!(vision(&root), saved, "nothing was written");
        assert_eq!(vision_api(&root, "PUT", json!({"text": "x"})).unwrap_err().0, 400);
        // D-92: at most VISION_LIMIT characters, counted as chars, not bytes.
        let base = saved["revision"].as_str().unwrap();
        let e = vision_api(&root, "PUT", json!({"text": "é".repeat(VISION_LIMIT + 1), "base": base})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7027")), (400, true), "{}", e.1.0);
        assert_eq!(vision(&root), saved, "nothing was written");

        let rows: Vec<String> = events(&root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.iter().map(|d| serde_json::from_str::<Value>(d).unwrap()).collect::<Vec<_>>(), [json!({"vision": saved})]);
        // Exactly at the limit is accepted, whatever its size in bytes.
        let full = vision_api(&root, "PUT", json!({"text": "é".repeat(VISION_LIMIT), "base": base})).unwrap();
        assert_eq!(full["text"].as_str().unwrap().chars().count(), VISION_LIMIT);
    }

    /// The models a strategy run may choose in these tests: one available, one not.
    fn models() -> Result<Value, Failure> {
        Ok(json!({"models": [
            {"id": "fake", "label": "Fake", "agent": "codex", "run": ["sh"]},
            {"id": "missing", "label": "Missing", "agent": "codex", "run": ["solace-no-such-program"]}
        ]}))
    }
    fn root_events(root: &Path) -> Vec<Value> {
        let rows: Vec<String> = events(root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.iter().map(|d| serde_json::from_str(d).unwrap()).collect()
    }

    /// D-93: a run starts with its first note; a second open run on the same
    /// context is E7003, notes follow, and `end` ends it `planned` with one
    /// transcript note, after which the context is free again.
    #[test]
    fn a_strategy_run_starts_takes_notes_and_ends() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        let on_f = json!({"kind": "feature", "id": f["id"]});
        let run = call("POST", &[], json!({"model": "fake", "text": "Plan F", "context": on_f})).unwrap();
        assert_eq!((run["kind"].as_str(), run["state"].as_str(), &run["context"]), (Some("strategy"), Some("running"), &on_f));
        assert_eq!((run["notes"][0]["author"].as_str(), run["notes"][0]["text"].as_str()), (Some("human"), Some("Plan F")));
        let rid = run["id"].as_str().unwrap();
        let e = call("POST", &[], json!({"model": "fake", "text": "Again", "context": on_f})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
        let page = call("POST", &[], json!({"model": "fake", "text": "Whole page", "context": {"kind": "page"}})).unwrap();
        assert_eq!(page["context"], json!({"kind": "page"}));

        let noted = call("POST", &[rid, "notes"], json!({"text": "And tasks"})).unwrap();
        assert_eq!(noted["notes"].as_array().unwrap().len(), 2);
        let ended = call("POST", &[rid, "end"], json!({})).unwrap();
        assert_eq!((ended["state"].as_str(), ended["outcome"].as_str()), (Some("ended"), Some("planned")));
        let notes = ended["notes"].as_array().unwrap();
        assert_eq!((notes.len(), notes[0]["author"].as_str()), (1, Some("solace")));
        let text = notes[0]["text"].as_str().unwrap();
        assert!(text.contains("**Human:**\n\nPlan F") && text.contains("**Human:**\n\nAnd tasks"), "{text}");
        for (parts, body) in [(vec![rid, "notes"], json!({"text": "late"})), (vec![rid, "end"], json!({})), (vec![rid, "stop"], json!({}))]
        {
            let e = call("POST", &parts, body).unwrap_err();
            assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{parts:?}: {}", e.1.0);
        }
        assert_eq!(record(&root, "runs", rid).unwrap(), ended, "a refused change writes nothing");
        // Once the first run has ended, the feature takes a new one.
        call("POST", &[], json!({"model": "fake", "text": "Later", "context": on_f})).unwrap();
        let stopped = call("POST", &[page["id"].as_str().unwrap(), "stop"], json!({})).unwrap();
        assert_eq!(stopped["outcome"], "stopped");
    }

    /// D-93: a malformed start, an unknown record, model or effort is refused
    /// and writes nothing; an unknown run is E7023 with 404.
    #[test]
    fn a_strategy_run_refuses_bad_requests() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let page = json!({"kind": "page"});
        for (body, status, code) in [
            (json!({"model": "fake", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "text": " ", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "text": "x"}), 400, "E7001"),
            (json!({"model": "fake", "text": "x", "context": {"kind": "feature", "id": "nope"}}), 400, "E7023"),
            (json!({"model": "fake", "text": "x", "context": {"kind": "milestone", "id": "nope"}}), 400, "E7023"),
            (json!({"model": "fake", "text": "x", "context": {"kind": "task", "id": "nope"}}), 400, "E7001"),
            (json!({"model": "other", "text": "x", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "effort": "high", "text": "x", "context": page}), 400, "E7001"),
            (json!({"model": "missing", "text": "x", "context": page}), 409, "E7011"),
        ] {
            let e = call("POST", &[], body.clone()).unwrap_err();
            assert_eq!((e.0, &e.1.0[..5]), (status, code), "{body}: {}", e.1.0);
        }
        assert_eq!(strategy_runs(&root), Vec::<Value>::new());
        let on_task = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "task", "id": "t1"}})).unwrap();
        assert_eq!(on_task["context"], json!({"kind": "task", "id": "t1"}));
        for (m, parts) in [("GET", vec!["nope"]), ("POST", vec!["nope", "notes"]), ("POST", vec!["../x", "end"])] {
            let e = call(m, &parts, json!({"text": "x"})).unwrap_err();
            assert_eq!((e.0, e.1.0.starts_with("E7023")), (404, true), "{parts:?}: {}", e.1.0);
        }
        let rid = on_task["id"].as_str().unwrap();
        assert_eq!(call("POST", &[rid, "notes"], json!({})).unwrap_err().0, 400);
        assert_eq!(call("POST", &[rid, "questions", "q1"], json!({"option": "Approve"})).unwrap_err().0, 404);
    }

    /// D-93: an answer is recorded on its question once; the option must be offered.
    #[test]
    fn a_strategy_question_is_answered_once() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let mut run = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}})).unwrap();
        run["questions"] = json!([{"id": "q1", "text": "Create T?", "options": ["Approve", "Reject"], "answer": null}]);
        put_run(&root, &run).unwrap();
        let rid = run["id"].as_str().unwrap();
        let e = call("POST", &[rid, "questions", "q1"], json!({"option": "Maybe"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7001")), (400, true), "{}", e.1.0);
        let answered = call("POST", &[rid, "questions", "q1"], json!({"option": "Reject"})).unwrap();
        assert_eq!(answered["questions"][0]["answer"], "Reject");
        let e = call("POST", &[rid, "questions", "q1"], json!({"option": "Approve"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
    }

    /// A strategy run on the whole page, for proposals.
    fn page_run(root: &Path) -> String {
        let run = runs_api(root, "POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}}), models).unwrap();
        run["id"].as_str().unwrap().to_string()
    }
    /// Puts a proposal on run `rid` as its agent's message would; its question id.
    fn propose(root: &Path, rid: &str, kind: &str, id: Option<&str>, text: &str) -> String {
        let q = proposal(root, rid, kind, id, text).unwrap();
        let qid = q["id"].as_str().unwrap().to_string();
        let mut run = record(root, "runs", rid).unwrap();
        run["questions"].as_array_mut().unwrap().push(q);
        put_run(root, &run).unwrap();
        qid
    }
    /// The human's answer to question `qid`: the answer stored, and the run's last note.
    fn answer(root: &Path, rid: &str, qid: &str, option: &str) -> (String, String) {
        let run = runs_api(root, "POST", &[rid, "questions", qid], json!({"option": option}), models).unwrap();
        let q = run["questions"].as_array().unwrap().iter().find(|q| q["id"] == qid).unwrap().clone();
        let last = run["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().to_string();
        (q["answer"].as_str().unwrap().to_string(), last)
    }

    /// D-93: Approve creates a feature, a milestone or a task, says its id, and
    /// notes the answer on the run; the task gets a note naming the run.
    #[test]
    fn an_approved_proposal_creates_the_record() {
        let root = root();
        let rid = page_run(&root);
        for kind in ["feature", "milestone", "task"] {
            let qid = propose(&root, &rid, kind, None, r#"{"title": "New one"}"#);
            let (said, note) = answer(&root, &rid, &qid, "Approve");
            let prefix = format!("Approved and applied: created {kind}:");
            assert!(said.starts_with(&prefix), "{said}");
            let id = &said[prefix.len()..];
            assert!(note.contains(&format!("Create {kind} \"New one\"")) && note.contains(&said), "{note}");
            let created = if kind == "task" { board::task(&root, id).unwrap() } else { record(&root, &format!("{kind}s"), id).unwrap() };
            assert_eq!(created["title"], "New one");
            if kind == "task" {
                assert_eq!((created["home"].as_str(), created["column"].as_str()), (Some("."), Some("backlog")));
                let n = created["notes"].as_array().unwrap().last().unwrap().clone();
                assert_eq!(
                    (n["author"].as_str(), n["text"].as_str()),
                    (Some("solace"), Some(format!("Created by strategy run {rid}").as_str()))
                );
            }
        }
        // An edit says nothing more, and the task notes the change.
        let qid = propose(&root, &rid, "task", Some("t1"), r#"{"title": "Renamed"}"#);
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        let t1 = board::task(&root, "t1").unwrap();
        assert_eq!(
            (t1["title"].as_str(), t1["notes"][0]["text"].as_str()),
            (Some("Renamed"), Some(format!("Changed by strategy run {rid}").as_str()))
        );
    }

    /// D-129: a proposed task whose scope names a folder that isn't a project is
    /// refused when the proposal is sent, naming the folder and the projects, and
    /// nothing is written. A folder that is a project is accepted.
    #[test]
    fn a_proposal_scope_must_name_projects() {
        let root = root();
        let rid = page_run(&root);
        for text in [r#"{"title": "Later", "scope": ["web", "api"]}"#, r#"{"title": "Half", "scope": [".", "web"]}"#] {
            let e = proposal(&root, &rid, "task", None, text).unwrap_err();
            assert!(
                e.0.starts_with("E7001") && e.0.contains("is not a project") && e.0.contains("web") && e.0.contains("."),
                "{text}: {}",
                e.0
            );
        }
        assert_eq!(board::all_tasks(&root).unwrap().len(), 1, "the refused proposals wrote nothing");
        assert!(proposal(&root, &rid, "task", None, r#"{"title": "Here", "scope": ["."], "dependsOn": ["t1"]}"#).is_ok());
        let e = proposal(&root, &rid, "task", None, r#"{"title": "Out", "scope": ["../x"]}"#).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{}", e.0);
    }

    /// D-105: a proposal naming a field its request doesn't take is refused,
    /// naming the field and those it takes.
    #[test]
    fn a_proposal_refuses_a_field_it_does_not_take() {
        let root = root();
        let rid = page_run(&root);
        for (kind, id, text, bad) in [
            ("task", None, r#"{"title": "T", "dependson": ["t1"]}"#, "dependson"),
            ("task", Some("t1"), r#"{"scope": ["web"]}"#, "scope"),
            ("feature", None, r#"{"title": "F", "rank": 1}"#, "rank"),
            ("milestone", None, r#"{"title": "M", "milestone": "x"}"#, "milestone"),
        ] {
            let e = proposal(&root, &rid, kind, id, text).unwrap_err();
            assert!(e.0.starts_with("E7001") && e.0.contains(&format!("not `{bad}`")) && e.0.contains("title, body"), "{kind}: {}", e.0);
        }
        assert!(proposal(&root, &rid, "task", None, r#"{"title": "T", "dependsOn": []}"#).is_ok());
    }

    /// D-115: an edit proposal carries the record's current value of each field
    /// it proposes, as `before`, so the human sees only what changes; a create
    /// carries none, and the vision carries its text.
    #[test]
    fn an_edit_proposal_carries_the_current_values() {
        let root = root();
        let rid = page_run(&root);
        let asked = |qid: &str| {
            let run = record(&root, "runs", &rid).unwrap();
            run["questions"].as_array().unwrap().iter().find(|q| q["id"] == qid).unwrap()["proposal"].clone()
        };
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "body": "a"})).unwrap();
        let qid = propose(&root, &rid, "feature", Some(f["id"].as_str().unwrap()), r#"{"title": "F2", "body": "a"}"#);
        assert_eq!(asked(&qid)["before"], json!({"title": "F", "body": "a"}));
        let qid = propose(&root, &rid, "feature", None, r#"{"title": "New"}"#);
        assert!(asked(&qid).get("before").is_none(), "a create carries none");
        let t = board::task(&root, "t1").unwrap();
        let text = json!({"feature": f["id"].clone(), "dependsOn": []}).to_string();
        let qid = propose(&root, &rid, "task", Some("t1"), &text);
        let before = asked(&qid)["before"].clone();
        assert_eq!((&before["feature"], &before["dependsOn"]), (&t["feature"], &t["dependsOn"]), "{before}");
        let qid = propose(&root, &rid, "vision", None, "For teams");
        let p = asked(&qid);
        assert_eq!((p["before"]["text"].as_str(), p["base"].is_string()), (Some(""), true), "{p}");
    }

    /// D-93: Reject writes no record, task or vision; only the run changes, with
    /// the answer on its notes.
    #[test]
    fn a_rejected_proposal_writes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        let qs = [
            propose(&root, &rid, "feature", None, r#"{"title": "No"}"#),
            propose(&root, &rid, "feature", Some(f["id"].as_str().unwrap()), r#"{"title": "No"}"#),
            propose(&root, &rid, "task", None, r#"{"title": "No"}"#),
            propose(&root, &rid, "vision", None, "No"),
        ];
        let (before, tasks, events) = (records(&root, "features"), board::all_tasks(&root).unwrap(), root_events(&root).len());
        for qid in &qs {
            let (said, note) = answer(&root, &rid, qid, "Reject");
            assert_eq!(said, "Reject");
            assert!(note.starts_with("Proposal: ") && note.ends_with("Answer: Reject"), "{note}");
        }
        assert_eq!(
            (records(&root, "features"), board::all_tasks(&root).unwrap(), vision(&root)["text"].as_str()),
            (before, tasks, Some(""))
        );
        assert!(root_events(&root)[events..].iter().all(|e| e.get("strategyRun").is_some()));
    }
    /// D-122: a feature or milestone link asks for its record's title; an
    /// unknown id is 404, a malformed one 400.
    #[test]
    fn a_record_link_finds_its_title() {
        let root = root();
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "1.0"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        for (kind, r, title) in [("milestones", &m, "1.0"), ("features", &f, "F")] {
            let id = r["id"].as_str().unwrap();
            assert_eq!(find_record(&root, kind, id).unwrap(), json!({"id": id, "title": title}));
            assert_eq!(find_record(&root, kind, "0123456789abcdef").unwrap_err().0, 404);
            assert_eq!(find_record(&root, kind, "../x").unwrap_err().0, 400);
        }
    }

    /// D-93: a vision proposal is applied only over the revision it was made
    /// from; after a human edit it is not applied, and the answer says so.
    #[test]
    fn a_vision_proposal_applies_only_over_its_base() {
        let root = root();
        let rid = page_run(&root);
        let qid = propose(&root, &rid, "vision", None, "For teams");
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        assert_eq!(vision(&root)["text"], "For teams");
        let qid = propose(&root, &rid, "vision", None, "For everyone");
        let base = vision(&root)["revision"].as_str().unwrap().to_string();
        vision_api(&root, "PUT", json!({"text": "Human's", "base": base})).unwrap();
        let (said, note) = answer(&root, &rid, &qid, "Approve");
        assert!(said.starts_with("Not applied: E7026"), "{said}");
        assert!(note.contains(&said), "{note}");
        assert_eq!(vision(&root)["text"], "Human's");
        let e = proposal(&root, &rid, "vision", None, &"é".repeat(1001)).unwrap_err();
        assert!(e.0.starts_with("E7027"), "{}", e.0);
    }

    /// D-93: a task proposal linked to a complete feature and milestone says so,
    /// and sets them `upcoming` only on Approve.
    #[test]
    fn a_task_proposal_reopens_its_complete_feature_on_approve() {
        let root = root();
        let rid = page_run(&root);
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "M", "state": "complete"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "canceled", "milestone": m["id"]})).unwrap();
        let fields = json!({"title": "More", "feature": f["id"]}).to_string();
        let rejected = propose(&root, &rid, "task", None, &fields);
        let q = record(&root, "runs", &rid).unwrap()["questions"][0].clone();
        assert_eq!(q["proposal"]["reopens"], json!([{"kind": "feature", "id": f["id"]}, {"kind": "milestone", "id": m["id"]}]));
        assert!(q["text"].as_str().unwrap().contains("back to upcoming"), "{}", q["text"]);
        answer(&root, &rid, &rejected, "Reject");
        let state = |kind: &str, v: &Value| record(&root, kind, v["id"].as_str().unwrap()).unwrap()["state"].clone();
        assert_eq!((state("features", &f), state("milestones", &m)), (json!("canceled"), json!("complete")));
        let approved = propose(&root, &rid, "task", None, &fields);
        let said = answer(&root, &rid, &approved, "Approve").0;
        assert_eq!((state("features", &f), state("milestones", &m)), (json!("upcoming"), json!("upcoming")));
        let t = board::task(&root, said.rsplit(':').next().unwrap()).unwrap();
        assert_eq!((&t["feature"], &t["milestone"]), (&f["id"], &m["id"]));
    }

    /// D-93, D-134: a reopen refused for one record (its milestone is gone)
    /// leaves the other as it was: every reopen is checked before any is saved.
    #[test]
    fn a_refused_reopen_changes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "M", "state": "complete"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "complete", "milestone": m["id"]})).unwrap();
        let qid = propose(&root, &rid, "task", None, &json!({"title": "More", "feature": f["id"]}).to_string());
        fs::remove_file(strategy_dir(&root, "milestones").join(format!("{}.json", m["id"].as_str().unwrap()))).unwrap();
        let said = answer(&root, &rid, &qid, "Approve").0;
        assert!(said.starts_with("Not applied") && said.contains(m["id"].as_str().unwrap()), "the milestone's reopen is refused: {said}");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("complete"));
        assert!(!board::all_tasks(&root).unwrap().iter().any(|t| t["title"] == "More"));
    }

    /// D-93: an apply the request would refuse writes nothing and puts the code
    /// in the answer: a task whose run started since (E7003), a dependsOn cycle
    /// made since (E7025). An ended run takes no answer.
    #[test]
    fn a_refused_apply_changes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let mut t2 = board::created(&root, &json!({"title": "T2"})).unwrap();
        write(&root, &t2).unwrap();
        let t2_id = t2["id"].as_str().unwrap().to_string();
        let cycle = propose(&root, &rid, "task", Some("t1"), &json!({"dependsOn": [t2_id]}).to_string());
        let running = propose(&root, &rid, "task", Some(&t2_id), r#"{"title": "Busy"}"#);
        t2["dependsOn"] = json!(["t1"]);
        t2["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        write(&root, &t2).unwrap();
        let before = (board::task(&root, "t1").unwrap(), board::task(&root, &t2_id).unwrap());
        for (qid, code) in [(cycle, "E7025"), (running, "E7003")] {
            let (said, note) = answer(&root, &rid, &qid, "Approve");
            assert!(said.starts_with(&format!("Not applied: {code}")), "{said}");
            assert!(note.contains(&said), "{note}");
        }
        assert_eq!((board::task(&root, "t1").unwrap(), board::task(&root, &t2_id).unwrap()), before);
        let late = propose(&root, &rid, "feature", None, r#"{"title": "Late"}"#);
        runs_api(&root, "POST", &[&rid, "end"], json!({}), models).unwrap();
        let e = runs_api(&root, "POST", &[&rid, "questions", &late], json!({"option": "Approve"}), models).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
        assert!(records(&root, "features").is_empty());
    }

    /// D-93: a proposal message applied twice asks once and sets the run
    /// waiting; one reaching an ended run is only noted.
    #[test]
    fn a_strategy_proposal_message_applies_once() {
        let root = root();
        let rid = page_run(&root);
        let q = proposal(&root, &rid, "feature", None, r#"{"title": "F"}"#).unwrap();
        let m = json!({"id": "m1", "kind": "strategy_propose", "run": rid, "question": q});
        apply(&root, &m).unwrap();
        apply(&root, &m).unwrap();
        let run = record(&root, "runs", &rid).unwrap();
        assert_eq!((run["questions"].as_array().unwrap().len(), run["state"].as_str()), (1, Some("waiting")));
        assert!(run["questions"][0]["time"].as_u64().is_some_and(|t| t > 0));
        assert_eq!(run["questions"][0]["after"], json!(run["notes"].as_array().unwrap().len()));
        runs_api(&root, "POST", &[&rid, "end"], json!({}), models).unwrap();
        apply(&root, &json!({"id": "m2", "kind": "strategy_propose", "run": rid, "question": q})).unwrap();
        let run = record(&root, "runs", &rid).unwrap();
        assert_eq!((run["questions"].as_array().unwrap().len(), run["state"].as_str()), (1, Some("ended")));
        assert!(run["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().contains("after the run ended"));
    }

    /// D-93: every change is a root event with the run in full; the list leaves
    /// out notes, and one run comes with its transcript.
    #[test]
    fn strategy_run_changes_are_root_events() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let run = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}})).unwrap();
        let rid = run["id"].as_str().unwrap();
        let noted = call("POST", &[rid, "notes"], json!({"text": "y"})).unwrap();
        let ended = call("POST", &[rid, "end"], json!({})).unwrap();
        assert_eq!(root_events(&root), [json!({"strategyRun": run}), json!({"strategyRun": noted}), json!({"strategyRun": ended})]);
        let listed = call("GET", &[], json!({})).unwrap();
        assert_eq!((listed[0]["id"].as_str(), listed[0].get("notes")), (Some(rid), None));
        let one = call("GET", &[rid], json!({})).unwrap();
        assert_eq!((&one["notes"], &one["transcript"]), (&ended["notes"], &json!([])));
    }

    /// D-87: a task saved before the links loads with them empty; one already in a
    /// final column takes its last end time, or its file's, as its closedTime.
    #[test]
    fn an_old_task_loads_with_empty_links_and_a_closed_time() {
        let root = root();
        let t = board::task(&root, "t1").unwrap();
        assert_eq!(
            (&t["feature"], &t["milestone"], &t["dependsOn"], &t["closedTime"], &t["archivedTime"]),
            (&Value::Null, &Value::Null, &json!([]), &Value::Null, &Value::Null)
        );
        let dir = board::store_dir(&root);
        fs::write(dir.join("t2.json"), r#"{"id":"t2","column":"complete","home":".","runs":[{"ended":50}],"finished":{"time":70}}"#)
            .unwrap();
        fs::write(dir.join("t3.json"), r#"{"id":"t3","column":"complete","home":".","runs":[]}"#).unwrap();
        assert_eq!(board::task(&root, "t2").unwrap()["closedTime"], json!(70));
        let modified =
            fs::metadata(dir.join("t3.json")).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(board::task(&root, "t3").unwrap()["closedTime"], json!(modified));
    }

    /// D-130: a task saved before createdTime takes the earlier of its first note's
    /// time and its first run's start, or else its file's.
    #[test]
    fn an_old_task_loads_with_a_created_time() {
        let root = root();
        let dir = board::store_dir(&root);
        let old = [
            ("t2", r#"{"id":"t2","column":"backlog","home":".","notes":[{"time":40},{"time":10}],"runs":[]}"#),
            ("t3", r#"{"id":"t3","column":"backlog","home":".","notes":[],"runs":[{"started":60},{"started":5}]}"#),
            ("t4", r#"{"id":"t4","column":"backlog","home":".","notes":[{"time":40}],"runs":[{"started":30}]}"#),
            ("t5", r#"{"id":"t5","column":"backlog","home":".","notes":[],"runs":[]}"#),
        ];
        for (id, text) in old {
            fs::write(dir.join(format!("{id}.json")), text).unwrap();
        }
        let created = |id: &str| board::task(&root, id).unwrap()["createdTime"].clone();
        assert_eq!((created("t2"), created("t3"), created("t4")), (json!(40), json!(60), json!(30)));
        let modified =
            fs::metadata(dir.join("t5.json")).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(created("t5"), json!(modified));
    }

    /// D-130: a new task records when it was created; cancelling and restoring it keeps the time.
    #[test]
    fn a_new_task_gets_a_created_time_once() {
        let root = root();
        let w = board::flow(&root).unwrap();
        let mut t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let created = t["createdTime"].as_u64().expect("set on create");
        t["createdTime"] = json!(created - 100);
        board::move_to(&w, &mut t, &json!("cancelled")).unwrap();
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["createdTime"], json!(created - 100));
    }

    /// D-87: entering a final column sets closedTime once; later moves keep it.
    #[test]
    fn entering_complete_sets_closed_time_once() {
        let root = root();
        let w = board::flow(&root).unwrap();
        let mut t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        board::move_to(&w, &mut t, &json!("complete")).unwrap();
        let closed = t["closedTime"].as_u64().expect("set on entering complete");
        t["closedTime"] = json!(closed - 100);
        board::move_to(&w, &mut t, &json!("cancelled")).unwrap();
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["closedTime"], json!(closed - 100));
    }

    /// D-95: only a task in a final column is archived, once; leaving the final
    /// columns takes it out of the archive.
    #[test]
    fn archiving_takes_a_final_task_off_the_board() {
        let root = root();
        let archive = || api(&root, "POST", &["tasks", "t1", "archive"], json!({}));
        assert_eq!(&archive().unwrap_err().0[..5], "E7001", "a task in backlog is not archived");
        let w = board::flow(&root).unwrap();
        let mut t = board::task(&root, "t1").unwrap();
        board::move_to(&w, &mut t, &json!("complete")).unwrap();
        write(&scope::top(&root), &t).unwrap();
        let t = archive().unwrap();
        let archived = t["archivedTime"].as_u64().expect("set on archiving");
        let mut t = board::task(&root, "t1").unwrap();
        t["archivedTime"] = json!(archived - 100);
        write(&scope::top(&root), &t).unwrap();
        assert_eq!(archive().unwrap()["archivedTime"], json!(archived - 100), "archiving again keeps the time");
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["archivedTime"], Value::Null);
    }

    /// D-87, D-188: a feature brings its milestone to a task and overrides one
    /// the task had; a featured task's milestone cannot be set. Links are
    /// checked, editable in a final column and during a run, and cleared by a
    /// delete while the milestone stays.
    #[test]
    fn task_links_are_set_checked_and_cleared() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let patch = |tid: &str, body: Value| api(&root, "PATCH", &["tasks", tid], body);
        let code = |r: Result<Value, Failure>| r.unwrap_err().0[..5].to_string();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let t1 = api(&root, "POST", &["tasks"], json!({"title": "T1"})).unwrap();
        let t1 = t1["id"].as_str().unwrap();
        assert_eq!(patch(t1, json!({"milestone": m2["id"]})).unwrap()["milestone"], m2["id"], "a task with no feature owns its milestone");
        let t = patch(t1, json!({"feature": f["id"]})).unwrap();
        assert_eq!((&t["feature"], &t["milestone"]), (&f["id"], &m1["id"]), "a feature brings its milestone and replaces one");
        let refused = patch(t1, json!({"milestone": m2["id"]})).unwrap_err();
        assert!(refused.0.starts_with("E7001") && refused.0.contains("feature's milestone"), "{}", refused.0);
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], m1["id"], "a refused milestone changes nothing");
        let t2 = api(&root, "POST", &["tasks"], json!({"title": "T2"})).unwrap();
        let t2id = t2["id"].as_str().unwrap();
        assert_eq!(code(patch(t2id, json!({"feature": f["id"], "milestone": null}))), "E7001");
        assert!(board::task(&root, t2id).unwrap()["feature"].is_null(), "a refused link changes nothing");
        assert_eq!(patch(t2id, json!({"feature": f["id"]})).unwrap()["milestone"], m1["id"]);
        let bare = call("POST", &["features"], json!({"title": "Bare"}));
        assert!(patch(t2id, json!({"feature": bare["id"]})).unwrap()["milestone"].is_null(), "a feature with no milestone gives none");
        assert_eq!(patch(t2id, json!({"feature": f["id"]})).unwrap()["milestone"], m1["id"]);
        let t = patch(t1, json!({"feature": null})).unwrap();
        assert!(t["feature"].is_null());
        assert_eq!(t["milestone"], m1["id"], "removing a feature keeps the milestone");
        assert_eq!(patch(t1, json!({"milestone": m2["id"]})).unwrap()["milestone"], m2["id"], "the task owns it again");
        assert_eq!(code(patch(t1, json!({"feature": "nope"}))), "E7023");
        assert_eq!(code(patch(t1, json!({"dependsOn": [t1]}))), "E7024");
        assert_eq!(code(patch(t1, json!({"dependsOn": ["nope"]}))), "E7024");
        assert_eq!(patch(t1, json!({"dependsOn": [t2id]})).unwrap()["dependsOn"], json!([t2id]));
        assert_eq!(code(patch(t2id, json!({"dependsOn": [t1]}))), "E7025");
        assert_eq!(board::task(&root, t1).unwrap()["dependsOn"], json!([t2id]));

        // During a run, and in a final column, only the links change; an empty edit is refused too.
        let mut busy = board::task(&root, t2id).unwrap();
        busy["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        write(&root, &busy).unwrap();
        assert_eq!(code(patch(t2id, json!({"milestone": m2["id"]}))), "E7001");
        assert_eq!(code(patch(t2id, json!({"title": "no"}))), "E7003");
        assert_eq!(code(patch(t2id, json!({}))), "E7003");
        let mut done = board::task(&root, t2id).unwrap();
        done["runs"] = json!([]);
        done["column"] = json!("complete");
        write(&root, &done).unwrap();
        assert_eq!(code(patch(t2id, json!({"milestone": null}))), "E7001");
        assert_eq!(code(patch(t2id, json!({"title": "no", "feature": null}))), "E7004");
        assert_eq!(code(patch(t2id, json!({}))), "E7004");

        call("DELETE", &["features", f["id"].as_str().unwrap()], json!({}));
        for id in [t1, t2id] {
            assert_eq!(board::task(&root, id).unwrap()["feature"], Value::Null);
        }
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], m2["id"], "the milestone stays");
        assert_eq!(board::task(&root, t2id).unwrap()["milestone"], m1["id"]);
        call("DELETE", &["milestones", m2["id"].as_str().unwrap()], json!({}));
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], Value::Null);
        assert_eq!(board::task(&root, t2id).unwrap()["milestone"], m1["id"], "another milestone stays");
    }

    /// D-128: an active feature completes when every task it has is in a final
    /// column, a canceled or Tech Debt task too, and is active again when one
    /// leaves; a feature in another state, or with no tasks, never changes.
    #[test]
    fn a_feature_follows_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let state = |f: &Value| record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"].clone();
        let move_to = |tid: &str, column: &str| {
            let mut t = board::task(&root, tid).unwrap();
            t["column"] = json!(column);
            write(&root, &t).unwrap();
        };
        let f = call("POST", &["features"], json!({"title": "F", "state": "active"}));
        let empty = call("POST", &["features"], json!({"title": "Empty", "state": "active"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(state(&f), json!("active"), "an open task keeps it active");
        let mut debt = board::task(&root, "t1").unwrap();
        debt["id"] = json!("t2");
        debt["techDebt"] = json!(true);
        write(&root, &debt).unwrap();
        move_to("t1", "complete");
        assert_eq!(state(&f), json!("active"), "an open Tech Debt task counts");
        move_to("t2", "cancelled");
        assert_eq!(state(&f), json!("complete"));
        assert!(!record(&root, "features", f["id"].as_str().unwrap()).unwrap()["startTime"].is_null());
        move_to("t1", "backlog");
        assert_eq!(state(&f), json!("active"), "a reopened task makes it active again");
        move_to("t1", "complete");
        assert_eq!(state(&f), json!("complete"));
        assert_eq!(state(&empty), json!("active"), "no tasks: no change");

        let upcoming = call("POST", &["features"], json!({"title": "U", "state": "upcoming"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": upcoming["id"]})).unwrap();
        assert_eq!(state(&upcoming), json!("upcoming"), "only active and complete features follow");
        assert_eq!(state(&f), json!("complete"), "t2 is still final");
        api(&root, "PATCH", &["tasks", "t2"], json!({"feature": null})).unwrap();
        assert_eq!(state(&f), json!("complete"), "with no tasks left it stays");
    }

    /// D-132, D-134: a feature that becomes active, by an edit or an approved
    /// proposal, moves its Backlog tasks to Planning, each with a note naming
    /// it; tasks in other columns or of other features stay. A task linked to
    /// or proposed in an active feature is planned; in another state it isn't.
    #[test]
    fn an_active_feature_plans_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let column = |tid: &str| board::task(&root, tid).unwrap()["column"].as_str().unwrap().to_string();
        let notes = |tid: &str| {
            board::task(&root, tid).unwrap()["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["text"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let new_task = |title: &str, feature: &Value| {
            let id = api(&root, "POST", &["tasks"], json!({"title": title})).unwrap()["id"].as_str().unwrap().to_string();
            api(&root, "PATCH", &["tasks", &id], json!({"feature": feature})).unwrap();
            id
        };
        let f = call("POST", &["features"], json!({"title": "Search", "state": "upcoming"}));
        let other = call("POST", &["features"], json!({"title": "Other", "state": "upcoming"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(column("t1"), "backlog", "an upcoming feature's task stays in Backlog");
        let busy = new_task("Busy", &f["id"]);
        let mut t = board::task(&root, &busy).unwrap();
        t["column"] = json!("assigned");
        write(&root, &t).unwrap();
        let waits = new_task("Waits", &other["id"]);

        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"state": "active"}));
        assert_eq!(column("t1"), "planning");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("active"));
        let moved = notes("t1");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0], format!("Moved to Planning: its feature feature:{} is active.", f["id"].as_str().unwrap()));
        assert_eq!((column(&busy), notes(&busy).len()), ("assigned".to_string(), 0), "only Backlog tasks move");
        assert_eq!(column(&waits), "backlog", "another feature's task stays");
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"title": "Search 2"}));
        assert_eq!(notes("t1").len(), 1, "an edit that keeps it active moves nothing");

        let linked = new_task("Linked", &f["id"]);
        assert_eq!(column(&linked), "planning", "a task linked to an active feature is planned");
        assert_eq!(notes(&linked).len(), 1);

        let rid = page_run(&root);
        let qid = propose(&root, &rid, "task", None, &json!({"title": "Proposed", "feature": f["id"]}).to_string());
        let said = answer(&root, &rid, &qid, "Approve").0;
        let id = said.rsplit(':').next().unwrap();
        assert_eq!(column(id), "planning");
        let n = notes(id);
        assert!(n[0].starts_with("Created by strategy run") && n[1].starts_with("Moved to Planning"), "{n:?}");

        let qid = propose(&root, &rid, "feature", other["id"].as_str(), r#"{"state": "active"}"#);
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        assert_eq!(column(&waits), "planning", "an approved proposal plans the tasks too");
    }

    /// Writes task `id` straight into the store, a copy of t1 in Backlog under
    /// `feature`, as a file saved before D-134 would be.
    fn stored_backlog_task(root: &Path, id: &str, feature: &Value) {
        let mut t = board::task(root, "t1").unwrap();
        t["id"] = json!(id);
        t["title"] = json!(id);
        t["column"] = json!("backlog");
        t["feature"] = feature.clone();
        fs::write(board::store_dir(root).join(format!("{id}.json")), t.to_string()).unwrap();
    }
    fn column_of(root: &Path, id: &str) -> String {
        board::task(root, id).unwrap()["column"].as_str().unwrap().to_string()
    }
    fn notes_of(root: &Path, id: &str) -> Vec<String> {
        let t = board::task(root, id).unwrap();
        t["notes"].as_array().map_or(vec![], |ns| ns.iter().map(|n| n["text"].as_str().unwrap().to_string()).collect())
    }

    /// D-134: a task already stored in Backlog under an active feature moves to
    /// Planning when the server starts, once, with a note naming the feature;
    /// saving the feature or the task moves one too.
    #[test]
    fn an_existing_backlog_task_under_an_active_feature_is_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "active"})).unwrap();
        let fid = f["id"].as_str().unwrap();
        stored_backlog_task(&root, "old", &f["id"]);
        assert_eq!(column_of(&root, "old"), "backlog");
        plan_active(&root).unwrap();
        assert_eq!(column_of(&root, "old"), "planning");
        let n = notes_of(&root, "old");
        assert_eq!(n.len(), 1, "{n:?}");
        assert!(n[0].contains(&format!("feature:{fid}")), "{n:?}");
        plan_active(&root).unwrap();
        assert_eq!(notes_of(&root, "old").len(), 1, "a second start adds nothing");

        stored_backlog_task(&root, "second", &f["id"]);
        strategy_api(&root, "PATCH", &["features", fid], json!({"title": "Search 2"})).unwrap();
        assert_eq!(column_of(&root, "second"), "planning", "saving the active feature plans it");
        assert_eq!(notes_of(&root, "old").len(), 1);

        stored_backlog_task(&root, "third", &f["id"]);
        api(&root, "PATCH", &["tasks", "third"], json!({"title": "Renamed"})).unwrap();
        assert_eq!(column_of(&root, "third"), "planning", "saving the task plans it");
        assert_eq!(notes_of(&root, "third").len(), 1);
    }

    /// D-134: a Backlog task linked to a complete feature reopens it as
    /// `active` (D-128), and is planned in the same save.
    #[test]
    fn a_task_that_reopens_its_feature_is_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Done", "state": "complete"})).unwrap();
        let before = task_events(&root, "t1");
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(task_events(&root, "t1"), before + 1, "the save is logged once (D-134)");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("active"));
        assert_eq!(column_of(&root, "t1"), "planning");
        let n = notes_of(&root, "t1");
        assert!(n.len() == 1 && n[0].contains(&format!("feature:{}", f["id"].as_str().unwrap())), "{n:?}");
        let order: Vec<Value> =
            all_events(&root).into_iter().filter(|e| e["task"]["id"] == "t1" || e["feature"]["id"] == f["id"]).collect();
        assert!(order.last().is_some_and(|e| e["feature"]["state"] == "active"), "the feature is saved after its task");
    }

    /// Every task log entry's data, in `seq` order.
    fn all_events(root: &Path) -> Vec<Value> {
        let rows: Vec<String> = events(root)
            .unwrap()
            .prepare("SELECT data FROM events ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.iter().map(|d| serde_json::from_str(d).unwrap()).collect()
    }
    /// How many task log entries record task `id`.
    fn task_events(root: &Path, id: &str) -> usize {
        all_events(root).iter().filter(|e| e["task"]["id"] == id).count()
    }

    /// An archived task stays where it is, in Backlog under a feature that
    /// becomes active, when the server starts and when it is saved.
    #[test]
    fn an_archived_task_is_not_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "upcoming"})).unwrap();
        stored_backlog_task(&root, "old", &f["id"]);
        let path = board::store_dir(&root).join("old.json");
        let mut t: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        t["archivedTime"] = json!(now());
        fs::write(&path, t.to_string()).unwrap();
        strategy_api(&root, "PATCH", &["features", f["id"].as_str().unwrap()], json!({"state": "active"})).unwrap();
        plan_active(&root).unwrap();
        api(&root, "PATCH", &["tasks", "old"], json!({"title": "Renamed"})).unwrap();
        assert_eq!((column_of(&root, "old"), notes_of(&root, "old").len()), ("backlog".to_string(), 0));
    }

    /// D-134: a task with no feature, or whose feature is not active, stays in
    /// Backlog when the server starts and when it is saved.
    #[test]
    fn a_feature_that_is_not_active_leaves_its_tasks_alone() {
        let root = root();
        let mut ids = vec![("none".to_string(), Value::Null)];
        for state in ["proposed", "upcoming", "complete", "canceled"] {
            let f = strategy_api(&root, "POST", &["features"], json!({"title": state, "state": state})).unwrap();
            ids.push((state.to_string(), f["id"].clone()));
        }
        for (id, f) in &ids {
            stored_backlog_task(&root, id, f);
        }
        plan_active(&root).unwrap();
        for (id, _) in &ids {
            api(&root, "PATCH", &["tasks", id], json!({"title": "Renamed"})).unwrap();
            assert_eq!((column_of(&root, id), notes_of(&root, id).len()), ("backlog".to_string(), 0), "{id}");
        }
    }

    /// D-134: columns are found by id; a workflow without both `backlog` and
    /// `planning` leaves an active feature's task where it is.
    #[test]
    fn a_workflow_without_the_columns_is_left_alone() {
        for columns in [["backlog", "doing"], ["waiting", "planning"]] {
            let root = root();
            let toml = format!(
                concat!(
                    "[board]\nevents = {{ finished = \"{1}\" }}\n[[board.columns]]\nid = \"{0}\"\n",
                    "[[board.columns]]\nid = \"{1}\"\n[[board.columns]]\nid = \"backlog_done\"\nfinal = true\n",
                    "[board.actions]\n",
                ),
                columns[0], columns[1]
            );
            fs::write(root.join("solace.toml"), toml).unwrap();
            let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "active"})).unwrap();
            stored_backlog_task(&root, "old", &f["id"]);
            plan_active(&root).unwrap();
            let mut t = board::task(&root, "old").unwrap();
            t["title"] = json!("Renamed");
            write(&root, &t).unwrap();
            assert_eq!((column_of(&root, "old"), notes_of(&root, "old").len()), ("backlog".to_string(), 0), "{columns:?}");
            // A feature that becomes active moves none of its tasks either.
            let g = strategy_api(&root, "POST", &["features"], json!({"title": "Later", "state": "upcoming"})).unwrap();
            stored_backlog_task(&root, "later", &g["id"]);
            strategy_api(&root, "PATCH", &["features", g["id"].as_str().unwrap()], json!({"state": "active"})).unwrap();
            assert_eq!((column_of(&root, "later"), notes_of(&root, "later").len()), ("backlog".to_string(), 0), "{columns:?}");
        }
    }

    /// D-128: a record saved `planned` reads as `upcoming`; `planned` is refused.
    #[test]
    fn planned_reads_as_upcoming() {
        let root = root();
        fs::create_dir_all(strategy_dir(&root, "milestones")).unwrap();
        fs::write(strategy_dir(&root, "milestones").join("m1.json"), r#"{"id":"m1","title":"M","state":"planned"}"#).unwrap();
        assert_eq!(record(&root, "milestones", "m1").unwrap()["state"], json!("upcoming"));
        let refused = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "planned"})).unwrap_err();
        assert!(refused.1.0.starts_with("E7001"), "{}", refused.1.0);
    }

    /// An update whose change fails writes nothing.
    #[test]
    fn a_failed_update_writes_nothing() {
        let root = root();
        let before = board::task(&root, "t1").unwrap();
        let e = update(&root, "t1", |t| {
            t["title"] = json!("Changed");
            Err::<(), _>(Failure("refused".into(), 1))
        })
        .unwrap_err();
        assert_eq!(e.0, "refused");
        assert_eq!(board::task(&root, "t1").unwrap(), before);
        assert_eq!(update(&root, "t1", |t| Ok(t["column"].clone())).unwrap(), "backlog");
        assert!(update(&root, "nope", |_| Ok(())).is_err());
    }

    /// A message lands whole, with its id, and ids sort in the order sent.
    #[test]
    fn send_leaves_whole_messages_in_order() {
        let root = root();
        let first = send(&root, json!({"kind": "note", "task": "t1", "text": "a"})).unwrap();
        let second = send(&root, json!({"kind": "note", "task": "t1", "text": "b"})).unwrap();
        assert!(first < second);
        let m: Value = serde_json::from_slice(&fs::read(&first).unwrap()).unwrap();
        assert_eq!(m["id"].as_str(), first.file_stem().and_then(|s| s.to_str()));
        assert_eq!(m["text"], "a");
        let names: Vec<_> = fs::read_dir(inbox(&root)).unwrap().map(|e| e.unwrap().path()).collect();
        assert!(names.iter().all(|p| p.extension().is_some_and(|x| x == "json")), "{names:?}");
        assert!(!rejected(&root, &first));
    }

    /// The server lock tells whether a server runs for the root.
    #[test]
    fn serving_follows_the_server_lock() {
        let root = root();
        assert!(!serving(&root));
        let held = try_lock_file(&server_lock(&root)).unwrap().unwrap();
        assert!(serving(&root));
        drop(held);
        assert!(!serving(&root));
    }

    /// A message with an older id that lands after a newer one is still applied.
    #[test]
    fn an_older_inbox_message_arriving_late_is_applied() {
        let root = root();
        testing::inbox_file(&root, "0002", json!({"kind":"note","task":"t1","author":"solace","text":"newer"}));
        drain(&root);
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"older"}));
        drain(&root);
        let texts: Vec<String> = board::task(&root, "t1").unwrap()["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(texts, ["newer", "older"]);
        // The same message again (a replay after a crash) is skipped.
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"older"}));
        drain(&root);
        assert_eq!(board::task(&root, "t1").unwrap()["notes"].as_array().unwrap().len(), 2);
    }

    /// A message that can't be applied, or that panics, is set aside and the task is told.
    #[test]
    fn a_message_that_fails_or_panics_is_set_aside() {
        let root = root();
        // No such run: refused.
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":"t1","run":"nope","command":"done","text":"x"}));
        // A task file that lacks its lists (written by hand): applying it panics.
        fs::write(board::store_dir(&root).join("t2.json"), r#"{"id":"t2","column":"backlog","home":"."}"#).unwrap();
        testing::inbox_file(&root, "0002", json!({"kind":"agent","task":"t2","run":"r","command":"note","text":"x"}));
        drain(&root);
        let inbox = inbox(&root);
        assert!(inbox.join("failed/0001.json").exists() && inbox.join("failed/0002.json").exists());
        assert!(!inbox.join("0001.json").exists() && !inbox.join("0002.json").exists());
        assert!(rejected(&root, &inbox.join("0001.json")));
        let t = board::task(&root, "t1").unwrap();
        assert!(t["notes"][0]["text"].as_str().unwrap().contains("set aside"));
        assert_eq!(t["attention"]["reason"], "a task message was set aside");
    }

    /// Telling the task that a message was set aside happens only when the task
    /// can be read and written; every other case leaves the store as it was.
    #[test]
    fn flagging_a_set_aside_message_touches_only_a_readable_task() {
        let root = root();
        let dir = board::store_dir(&root);
        fs::write(dir.join("junk.json"), "not json").unwrap();
        fs::write(dir.join("bare.json"), r#"{"id":"bare","column":"backlog","home":"."}"#).unwrap();
        let before = |names: &[&str]| -> Vec<Vec<u8>> { names.iter().map(|n| fs::read(dir.join(format!("{n}.json"))).unwrap()).collect() };
        let others = ["junk", "bare", "t1"];
        let kept = before(&others);
        let note = |tid: Value| json!({"kind": "note", "task": tid, "text": "x"});
        for m in [note(json!("../x")), note(json!("")), note(json!(null)), note(json!("missing")), note(json!("junk")), note(json!("bare"))]
        {
            flag_set_aside(&root, &m, "why");
        }
        // The flow can't be read while the settings are broken.
        fs::write(root.join("solace.toml"), "[board\n").unwrap();
        flag_set_aside(&root, &note(json!("t1")), "why");
        fs::remove_file(root.join("solace.toml")).unwrap();
        assert_eq!(before(&others), kept, "an invalid id, a missing or unparseable file, no notes list, or no flow: nothing is written");
        flag_set_aside(&root, &json!({"kind": "agent", "task": "t1"}), "the reason");
        let t = board::task(&root, "t1").unwrap();
        assert_eq!(t["notes"].as_array().unwrap().len(), 1);
        let text = t["notes"][0]["text"].as_str().unwrap();
        assert!(text.contains("(agent)") && text.contains("the reason"), "{text}");
        assert_eq!(t["notes"][0]["author"], "solace");
        // A message with no kind is named with an empty one.
        flag_set_aside(&root, &json!({"task": "t1"}), "again");
        assert!(board::task(&root, "t1").unwrap()["notes"][1]["text"].as_str().unwrap().contains("()"));
    }

    /// A task file that doesn't parse sets its message aside, as a panic does.
    #[test]
    fn a_message_for_an_unparseable_task_file_is_set_aside() {
        let root = root();
        fs::write(board::store_dir(&root).join("t9.json"), "{ not json").unwrap();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t9","author":"solace","text":"x"}));
        drain(&root);
        assert!(rejected(&root, &inbox(&root).join("0001.json")));
        assert_eq!(fs::read_to_string(board::store_dir(&root).join("t9.json")).unwrap(), "{ not json");
        // A message for a task that doesn't exist fails the same way.
        testing::inbox_file(&root, "0002", json!({"kind":"note","task":"nope","author":"solace","text":"x"}));
        drain(&root);
        assert!(rejected(&root, &inbox(&root).join("0002.json")));
    }

    /// Only `.json` files in the inbox are messages.
    #[test]
    fn drain_leaves_files_that_are_not_json() {
        let root = root();
        let dir = inbox(&root);
        fs::create_dir_all(&dir).unwrap();
        for name in ["0001.tmp", "notes.txt", "0002"] {
            fs::write(dir.join(name), r#"{"kind":"note","task":"t1","author":"solace","text":"x"}"#).unwrap();
        }
        drain(&root);
        assert!(board::task(&root, "t1").unwrap()["notes"].as_array().is_none_or(Vec::is_empty));
        for name in ["0001.tmp", "notes.txt", "0002"] {
            assert!(dir.join(name).exists(), "{name}");
        }
        assert!(!failed_dir(&root).exists());
    }

    /// The first message in the root's inbox, waited for at most 10 s: a test
    /// that waits for one that never comes fails, it doesn't hang the run.
    fn next_message(top: &Path) -> PathBuf {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(f) = fs::read_dir(inbox(top))
                .ok()
                .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "json")))
            {
                return f;
            }
            assert!(std::time::Instant::now() < end, "no message reached the inbox");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs `f` on a thread and waits at most 10 s for its result: a call that
    /// would wait forever fails the test instead of hanging the run (D-83).
    fn within<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        let (done, result) = mpsc::channel();
        std::thread::spawn(move || drop(done.send(f())));
        result.recv_timeout(Duration::from_secs(10)).expect("the call did not return within 10 s")
    }

    /// A sender where another process serves waits for that server's answer: a
    /// message it took returns Ok, one it set aside is an error, and a server
    /// that stops first leaves the message queued.
    #[test]
    fn deliver_hands_the_message_to_a_server_elsewhere() {
        // Taken: the "server" (another holder of the lock) removes the message.
        let root = root();
        let held = try_lock_file(&server_lock(&root)).unwrap().unwrap();
        let top = root.to_path_buf();
        let taker = std::thread::spawn(move || fs::remove_file(next_message(&top)).unwrap());
        let to = root.to_path_buf();
        within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"x"}))).unwrap();
        taker.join().unwrap();
        // Set aside: the "server" moves it to failed/.
        let top = root.to_path_buf();
        let rejecter = std::thread::spawn(move || {
            let file = next_message(&top);
            fs::create_dir_all(failed_dir(&top)).unwrap();
            fs::rename(&file, failed_dir(&top).join(file.file_name().unwrap())).unwrap();
        });
        let to = root.to_path_buf();
        let e = within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"y"}))).unwrap_err();
        rejecter.join().unwrap();
        assert!(e.0.contains("set aside"), "{}", e.0);
        // Gone: the lock is released and the message stays in the inbox, undelivered.
        let top = root.to_path_buf();
        let stopper = std::thread::spawn(move || {
            next_message(&top);
            drop(held);
        });
        let to = root.to_path_buf();
        within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"z"}))).unwrap();
        stopper.join().unwrap();
        assert_eq!(fs::read_dir(inbox(&root)).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file()).count(), 1);
        assert!(board::task(&root, "t1").unwrap()["notes"].as_array().is_none_or(Vec::is_empty));
    }

    /// A brief failure (`solace.toml` mid-save) is tried again: the message is
    /// applied, not set aside.
    #[test]
    fn drain_retries_a_brief_failure() {
        let root = root();
        fs::write(root.join("solace.toml"), "[board\n").unwrap();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"later"}));
        let settings = root.join("solace.toml");
        let fixer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            fs::remove_file(settings).unwrap();
        });
        drain(&root);
        fixer.join().unwrap();
        assert_eq!(board::task(&root, "t1").unwrap()["notes"][0]["text"], "later");
        assert!(!inbox(&root).join("0001.json").exists());
        assert!(!failed_dir(&root).exists(), "nothing was set aside");
    }

    /// Without a place to set it aside, a failed message is logged and removed,
    /// so the inbox never sticks on it.
    #[test]
    fn a_failed_message_with_nowhere_to_go_is_dropped() {
        let root = root();
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":"t1","run":"nope","command":"done","text":"x"}));
        fs::write(failed_dir(&root), "not a folder").unwrap();
        drain(&root);
        assert!(!inbox(&root).join("0001.json").exists());
        assert!(failed_dir(&root).is_file());
        assert!(!rejected(&root, &inbox(&root).join("0001.json")));
    }

    /// A sender waits for as long as a server lives, not for a fixed time, and
    /// gives up at once when the server stops.
    #[test]
    fn applied_waits_while_a_server_holds_the_lock() {
        let root = root();
        let held = try_lock_file(&server_lock(&root)).unwrap().unwrap();
        let sent = send(&root, json!({"kind":"note","task":"t1","text":"x"})).unwrap();
        let (gone, start) = (sent.clone(), std::time::Instant::now());
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            fs::remove_file(gone).unwrap();
        });
        let (top, file) = (root.to_path_buf(), sent.clone());
        assert!(within(move || applied(&top, &file)));
        assert!(start.elapsed() >= Duration::from_millis(350));
        remover.join().unwrap();
        // The server stops with the message still there: false, and the message stays.
        let sent = send(&root, json!({"kind":"note","task":"t1","text":"y"})).unwrap();
        drop(held);
        let (top, file) = (root.to_path_buf(), sent.clone());
        assert!(!within(move || applied(&top, &file)) && sent.exists());
    }

    /// A message delivered where no server runs lands after the earlier ones in
    /// the inbox: a create a stopped server left must not be sent again.
    #[test]
    fn deliver_applies_the_inbox_before_its_message() {
        let root = root();
        let mut t = board::new_task("Late", "", &json!("backlog"), 1.);
        t["home"] = json!(".");
        let tid = t["id"].as_str().unwrap().to_string();
        testing::inbox_file(&root, "0001", json!({"kind":"create","new":t}));
        deliver(&root, json!({"kind":"note","task":tid,"author":"solace","text":"hi"})).unwrap();
        assert_eq!(board::task(&root, &tid).unwrap()["notes"].as_array().unwrap().len(), 1);
        assert!(!inbox(&root).join("0001.json").exists());
    }

    /// A task id names a file, so it has only letters, digits and dashes.
    #[test]
    fn valid_id_takes_only_letters_digits_and_dashes() {
        for good in ["t1", "ab-12", "A-z-9"] {
            assert!(board::valid_id(good), "{good}");
        }
        for bad in ["", "../x", "a/b", "a b", "a.b", "é"] {
            assert!(!board::valid_id(bad), "{bad}");
        }
    }

    /// A message for a task id that could leave the store is refused, and
    /// nothing outside the store is read into it.
    #[test]
    fn a_message_for_an_invalid_task_id_is_refused() {
        let root = root();
        fs::write(root.join(".solace/x.json"), r#"{"id":"x","column":"backlog","home":".","notes":[]}"#).unwrap();
        let before = stored(&root);
        for bad in ["", "../x", "a/b"] {
            let e = apply(&root, &json!({"id": "m1", "kind": "note", "task": bad, "author": "solace", "text": "x"})).unwrap_err();
            assert!(e.0.contains("E7001") && e.0.contains("invalid task id"), "{bad}: {}", e.0);
        }
        assert_eq!(stored(&root), before);
    }

    fn always(_: &Path, _: &[&str], _: &Value) -> Result<bool, Failure> {
        Ok(true)
    }

    /// Without the locks, a change that now needs them changes nothing; with
    /// them, it applies.
    #[test]
    fn a_change_that_now_needs_the_locks_waits_for_them() {
        let root = root();
        let before = board::task(&root, "t1").unwrap();
        let patch = json!({"title": "Renamed"});
        assert!(change(&root, "PATCH", &["tasks", "t1"], &patch, false, always).unwrap().is_none());
        assert_eq!(board::task(&root, "t1").unwrap(), before);
        let (t, _) = change(&root, "PATCH", &["tasks", "t1"], &patch, true, always).unwrap().unwrap();
        assert_eq!(t["title"], "Renamed");
        assert_eq!(board::task(&root, "t1").unwrap()["title"], "Renamed");
    }

    /// A change that needed no locks at the first check but needs them inside
    /// the operation takes them and runs again, applying once.
    #[test]
    fn api_retries_a_change_that_came_to_need_the_locks() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn later(_: &Path, _: &[&str], _: &Value) -> Result<bool, Failure> {
            Ok(CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0)
        }
        let root = root();
        let t = api_with(&root, "POST", &["tasks", "t1", "notes"], json!({"text": "once"}), later).unwrap();
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 2, "the first check, then the one inside the operation");
        let notes = board::task(&root, "t1").unwrap()["notes"].clone();
        assert_eq!(notes.as_array().unwrap().len(), 1);
        assert_eq!(t["notes"], notes);
    }

    /// The server takes its lock only once the root has a `.solace`.
    #[test]
    fn a_tick_takes_the_server_lock_once_the_root_has_solace() {
        let root = testing::Scratch::new();
        let (_jobs, queue) = mpsc::channel::<Job>();
        let mut held = None;
        tick(&root, &mut held, &queue);
        assert!(held.is_none() && !root.join(".solace").exists());
        fs::create_dir_all(root.join(".solace")).unwrap();
        tick(&root, &mut held, &queue);
        assert!(held.is_some() && serving(&root));
        drop(held);
        let end = std::time::Instant::now() + Duration::from_secs(1);
        while serving(&root) && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!serving(&root));
    }

    /// A tick applies the inbox before the operation it runs, and an operation
    /// that panics leaves the server running.
    #[test]
    fn a_tick_applies_the_inbox_first_and_survives_a_panic() {
        let root = root();
        let (jobs, queue) = mpsc::channel::<Job>();
        let mut held = None;
        let (seen, notes) = mpsc::channel();
        let again = seen.clone();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"first"}));
        let r = root.to_path_buf();
        jobs.send(Box::new(move || seen.send(board::task(&r, "t1").unwrap()["notes"].as_array().unwrap().len()).unwrap())).unwrap();
        tick(&root, &mut held, &queue);
        assert_eq!(notes.try_recv().unwrap(), 1);
        jobs.send(Box::new(|| panic!("a failing operation"))).unwrap();
        tick(&root, &mut held, &queue);
        jobs.send(Box::new(move || again.send(0).unwrap())).unwrap();
        tick(&root, &mut held, &queue);
        assert_eq!(notes.try_recv().unwrap(), 0);
    }

    /// A root listing project `sub`: t1 at home in the root, scoped to both, with
    /// its folder in the root's board.
    fn workspace() -> testing::Scratch {
        let root = testing::Scratch::new().with(&[
            ("solace.toml", "projects = [\"sub\"]\n"),
            (".solace/board/t1.json", r#"{"id":"t1","column":"backlog","home":".","runs":[]}"#),
        ]);
        fs::create_dir_all(root.join("sub/.solace")).unwrap();
        board::adopt_tasks(&root, &root).unwrap();
        fs::create_dir_all(root.join(".solace/board/t1")).unwrap();
        root
    }

    /// D-61: a home leaving the scope moves the task's folder to the first project
    /// left, keeps the task in the root's store, and logs the project it left.
    #[test]
    fn a_task_whose_home_leaves_its_scope_is_logged_as_removed() {
        let root = workspace();
        let t = api(&root, "POST", &["tasks", "t1", "scope"], json!({"scope": ["sub"]})).unwrap();
        assert_eq!(t["home"], "sub");
        assert_eq!(board::task(&root.join("sub"), "t1").unwrap()["home"], "sub", "the task stays in the root's store");
        assert!(board::task(&root, "t1").is_err(), "and is no longer a task of the root's own project");
        assert!(!root.join(".solace/board/t1").exists(), "the folder leaves its old home");
        assert!(root.join("sub/.solace/board/t1").exists(), "the folder moves to the new home");
        let db = events(&root).unwrap();
        let rows: Vec<(String, String)> = db
            .prepare("SELECT project, data FROM events ORDER BY seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(rows.iter().any(|(p, d)| p == "." && d.contains(r#""removed":"t1"#)), "{rows:?}");
    }

    /// D-129 and D-160: while the root is no project a task with no project in its
    /// scope keeps none: it is listed marked `noProject`, opens from the root, is
    /// found with `project` false, and settles without a store at its home. A change
    /// makes the root a project, which re-homes the task to the root with a note,
    /// and its scope saves from there.
    #[test]
    fn a_task_with_no_project_shows_at_the_root() {
        // A root that lists `sub` and has no store of its own: the root is no project.
        let root = testing::Scratch::new().with(&[("solace.toml", "projects = [\"sub\"]\n"), ("sub/.solace/keep", "")]);
        let mut t = board::new_task("Orphan", "", &json!("backlog"), 1.);
        t["home"] = json!("web");
        t["scope"] = json!(["web"]);
        write(&root, &t).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let listed = board::all_tasks(&root).unwrap().into_iter().find(|x| x["id"] == tid).unwrap();
        assert_eq!(listed["noProject"], json!(true), "listed marked");
        assert!(!board::tasks(&root).unwrap().iter().any(|x| x["id"] == tid), "in no lane");
        assert_eq!(board::task(&root, &tid).unwrap()["noProject"], json!(true), "opens from the root");
        let found = find_task(&root, &tid).unwrap();
        assert_eq!((&found["dir"], &found["project"]), (&json!("web"), &json!(false)));
        board::settle_final(&root, &tid).unwrap();
        assert!(!root.join("web").join(".solace").exists(), "settling settles no store at its home");
        // The plan opens the root's store, so the root is a project and the task falls back to it.
        let t = api(&root, "POST", &["tasks", &tid, "actions"], json!({"action": "plan"})).unwrap();
        assert_eq!((t["column"].as_str(), t["home"].as_str()), (Some("planning"), Some(".")), "planned at the root");
        assert_eq!(t["scope"], json!(["."]), "scoped to the root");
        assert!(
            t["notes"].as_array().unwrap().iter().any(|n| n["text"].as_str().is_some_and(|x| x.contains("Moved from `web` to `.`"))),
            "the move is noted: {:?}",
            t["notes"]
        );
        assert_eq!(api(&root, "POST", &["tasks", &tid, "scope"], json!({"scope": ["sub"]})).unwrap()["home"], "sub", "its scope saves");
        let sub = root.join("sub");
        let t = api(&sub, "POST", &["tasks", &tid, "actions"], json!({"action": "cancel"})).unwrap();
        assert_eq!(t["column"], "cancelled");
        let t = api(&sub, "POST", &["tasks", &tid, "archive"], json!({})).unwrap();
        assert!(!t["archivedTime"].is_null(), "archived");
        assert!(!root.join("web").join(".solace").exists(), "no store at its old home");
    }

    /// A workflow whose `finished` event targets a final column, so `task done` settles.
    const DONE_IS_FINAL: &str = concat!(
        "[board]\nevents = { asked = \"todo\", finished = \"done\", failed = \"todo\" }\n",
        "[[board.columns]]\nid = \"todo\"\nactions = [\"cancel\"]\n",
        "[[board.columns]]\nid = \"done\"\nfinal = true\n",
        "[[board.columns]]\nid = \"gone\"\nfinal = true\n",
        "[board.actions]\ncancel = { revert = true, to = \"gone\" }\n",
    );

    /// One server per process, for one root. `start` runs it on its own thread; `op` sends its
    /// operations there; a second `start` says it is already running; and a `done`
    /// the server applies settles on a thread of its own (D-77 point 5).
    #[test]
    fn the_task_server_runs_its_operations_on_its_own_thread() {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
        let root = p.root.clone();
        start(&root).unwrap();
        let ran_on = op(&root, move || Ok(std::thread::current().id())).unwrap();
        assert_eq!(ran_on, SERVER.get().unwrap().thread, "the operation ran on the server's thread");
        assert_ne!(ran_on, std::thread::current().id());
        // Another root has no server here: it isn't served and its operations run on the caller.
        let other = testing::Scratch::new();
        assert!(!serving(&other));
        let here = op(&other, move || Ok(std::thread::current().id())).unwrap();
        assert_eq!(here, std::thread::current().id());
        let e = start(&root).unwrap_err();
        assert!(e.0.contains("the task server is already running"), "{}", e.0);
        // A `done` the server applies moves the task and settles it off the server's thread.
        let t = api(&root, "POST", &["tasks"], json!({"title": "Shapes"})).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let mut t = t;
        t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        write(&root, &t).unwrap();
        let s = board::open(&root).unwrap();
        let before = board::snapshot(&s).unwrap();
        s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
        board::record(&s, &tid, "r1", &before, &board::snapshot(&s).unwrap(), None).unwrap();
        let has_before =
            || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
        assert!(has_before(), "the change is recorded");
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":tid,"run":"r1","command":"done","text":"done"}));
        let mut settled = false;
        for _ in 0..400 {
            if board::task(&root, &tid).unwrap()["column"] == "done" && !has_before() {
                settled = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(settled, "the server applied the done and settled the task off its own thread");
    }

    /// D-188: changing a feature's milestone moves every task linked to it,
    /// including a closed and an archived one; clearing it moves them to none.
    #[test]
    fn a_feature_milestone_moves_all_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let patch = |tid: &str, body: Value| api(&root, "PATCH", &["tasks", tid], body);
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let mut ids = vec![];
        for _ in 0..3 {
            let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
            let id = t["id"].as_str().unwrap().to_string();
            patch(&id, json!({"feature": f["id"]})).unwrap();
            ids.push(id);
        }
        let (open, closed, archived) = (&ids[0], &ids[1], &ids[2]);
        let mut done = board::task(&root, closed).unwrap();
        done["column"] = json!("complete");
        write(&root, &done).unwrap();
        let mut gone = board::task(&root, archived).unwrap();
        gone["column"] = json!("complete");
        gone["archivedTime"] = json!(1);
        write(&root, &gone).unwrap();

        let raw =
            |id: &str| -> Value { serde_json::from_slice(&fs::read(board::store_dir(&root).join(format!("{id}.json"))).unwrap()).unwrap() };
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": m2["id"]}));
        for id in [open, closed, archived] {
            assert_eq!(raw(id)["milestone"], m2["id"], "{id}");
        }
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": null}));
        for id in [open, closed, archived] {
            assert!(raw(id)["milestone"].is_null(), "{id}");
        }
    }

    /// D-188: a task saved with a feature takes that feature's milestone when
    /// loaded, including an archived task and one whose saved milestone differs.
    #[test]
    fn an_old_task_takes_its_features_milestone() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let dir = board::store_dir(&root);
        for (id, saved, archived) in [("t8", Value::Null, Value::Null), ("t9", m2["id"].clone(), json!(1))] {
            let t = json!({"id": id, "column": "planning", "home": ".", "feature": f["id"], "milestone": saved,
                "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": archived, "notes": [], "runs": []});
            fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&t).unwrap()).unwrap();
        }
        let t = json!({"id": "t10", "column": "planning", "home": ".", "feature": null, "milestone": m2["id"],
            "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": null, "notes": [], "runs": []});
        fs::write(dir.join("t10.json"), serde_json::to_vec(&t).unwrap()).unwrap();
        for id in ["t8", "t9"] {
            assert_eq!(board::task(&root, id).unwrap()["milestone"], m1["id"], "{id}");
        }
        assert_eq!(board::task(&root, "t10").unwrap()["milestone"], m2["id"], "a task with no feature keeps its own");
    }

    /// D-188: a cascade that cannot write one task puts every task back and
    /// leaves the feature on its old milestone.
    #[test]
    fn a_failed_milestone_cascade_changes_no_task() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let id = t["id"].as_str().unwrap().to_string();
        api(&root, "PATCH", &["tasks", id.as_str()], json!({"feature": f["id"]})).unwrap();
        let dir = board::store_dir(&root);
        let bad = json!({"id": "bad/id", "column": "backlog", "home": ".", "feature": f["id"], "milestone": m1["id"], "rank": 2.0,
            "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": null, "notes": [], "runs": []});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = task_events(&root, &id);
        let e = strategy_api(&root, "PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": m2["id"]})).unwrap_err();
        assert!(e.1.0.contains("invalid task id"), "{}", e.1.0);
        let raw: Value = serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(raw["milestone"], m1["id"]);
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["milestone"], m1["id"]);
        assert_eq!(task_events(&root, &id), before, "the failed cascade logs no task save");
    }

    /// Deleting a milestone clears its features and writes each linked task once.
    #[test]
    fn deleting_a_milestone_writes_each_task_once() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m["id"]}));
        let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let id = t["id"].as_str().unwrap().to_string();
        api(&root, "PATCH", &["tasks", id.as_str()], json!({"feature": f["id"]})).unwrap();
        let before = task_events(&root, &id);
        call("DELETE", &["milestones", m["id"].as_str().unwrap()], json!({}));
        assert_eq!(task_events(&root, &id), before + 1);
        let raw: Value = serde_json::from_slice(&fs::read(board::store_dir(&root).join(format!("{id}.json"))).unwrap()).unwrap();
        assert!(raw["milestone"].is_null());
    }

    /// D-87: a feature save that fails while deleting a milestone puts back
    /// the features already cleared, and logs none of them.
    #[test]
    fn a_failed_milestone_delete_restores_its_features() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let f1 = call("POST", &["features"], json!({"title": "First", "milestone": m["id"]}));
        let f2 = call("POST", &["features"], json!({"title": "Second", "milestone": m["id"]}));
        let dir = strategy_dir(&root, "features");
        let bad = json!({"id": "bad/id", "title": "Bad", "rank": 3.0, "milestone": m["id"], "state": "upcoming",
            "startTime": null, "colorSlot": null});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = root_events(&root).len();
        let e = strategy_api(&root, "DELETE", &["milestones", m["id"].as_str().unwrap()], json!({})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        for f in [&f1, &f2] {
            assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["milestone"], m["id"]);
        }
        assert_eq!(record(&root, "milestones", m["id"].as_str().unwrap()).unwrap()["title"], json!("M"));
        assert_eq!(root_events(&root).len(), before, "the failed delete logs nothing");
    }

    /// A slot fix belongs to the batch that writes the features: a failed
    /// cascade puts it back instead of leaving it saved.
    #[test]
    fn a_failed_feature_batch_puts_back_a_kept_slot() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let dir = strategy_dir(&root, "features");
        fs::create_dir_all(&dir).unwrap();
        let old = json!({"id": "old", "title": "Old", "state": "active", "rank": 1, "startTime": 10,
            "milestone": m["id"]});
        fs::write(dir.join("old.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        let bad = json!({"id": "bad/id", "title": "Bad", "rank": 2, "milestone": m["id"], "state": "upcoming",
            "startTime": null});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let e = strategy_api(&root, "DELETE", &["milestones", m["id"].as_str().unwrap()], json!({})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        let saved: Value = serde_json::from_slice(&fs::read(dir.join("old.json")).unwrap()).unwrap();
        assert!(saved.get("colorSlot").is_none(), "the kept slot went back with the batch");
        assert_eq!(saved["milestone"], m["id"]);
        assert!(task_journals(&root).is_empty(), "no journal is left: {:?}", task_journals(&root));
    }

    /// A crash partway through a batch leaves its journal, and recovery puts
    /// back the files written so far.
    #[test]
    fn a_crashed_task_batch_is_put_back() {
        let root = root();
        let dir = board::store_dir(&root);
        let before = fs::read(dir.join("t1.json")).unwrap();
        fs::write(dir.join("t1.json"), serde_json::to_vec(&json!({"id": "t1", "column": "planning", "home": "."})).unwrap()).unwrap();
        fs::write(dir.join("t2.json"), serde_json::to_vec(&json!({"id": "t2", "column": "planning", "home": "."})).unwrap()).unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let record = json!({"files": [
            {"path": ".solace/tasks/t1.json", "bytes": before.clone()},
            {"path": ".solace/tasks/t2.json", "bytes": null}
        ]});
        fs::write(pending.join("tasks-crash.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        recover(&root).unwrap();
        assert_eq!(fs::read(dir.join("t1.json")).unwrap(), before);
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-crash.json").exists());
    }

    /// A batch that finishes drops its journal.
    #[test]
    fn a_successful_batch_leaves_no_journal() {
        let root = root();
        let mut t = board::task(&root, "t1").unwrap();
        t["title"] = json!("Changed");
        write(&root, &t).unwrap();
        assert_eq!(board::task(&root, "t1").unwrap()["title"], json!("Changed"));
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    /// Creating a task through the API leaves its file and no journal.
    #[test]
    fn a_created_task_is_on_disk() {
        let root = root();
        strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        assert!(task_journals(&root).is_empty(), "the feature batch is committed: {:?}", task_journals(&root));
        let t = api(&root, "POST", &["tasks"], json!({"title": "Created"})).unwrap();
        let id = t["id"].as_str().unwrap();
        assert!(board::store_dir(&root).join(format!("{id}.json")).exists());
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    fn task_journals(root: &std::path::Path) -> Vec<String> {
        fs::read_dir(root.join(".solace/pending"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("tasks-"))
            .collect()
    }

    /// A journal whose batch holds its file lock belongs to a running process.
    #[test]
    fn a_live_journal_is_not_put_back() {
        let root = root();
        let dir = board::store_dir(&root);
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        fs::write(dir.join("t2.json"), b"{}").unwrap();
        let record = json!({"files": [{"path": ".solace/tasks/t2.json", "bytes": null}]});
        fs::write(pending.join("tasks-live.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        let owner = try_lock_file(&pending.join("tasks-live.lock")).unwrap().unwrap();
        recover(&root).unwrap();
        assert!(dir.join("t2.json").exists());
        assert!(pending.join("tasks-live.json").exists());
        drop(owner);
        recover(&root).unwrap();
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-live.json").exists());
    }

    #[test]
    fn a_journal_path_outside_the_root_is_refused() {
        let root = root();
        let outside = root.parent().unwrap().join("journal-outside.txt");
        fs::write(&outside, b"keep").unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let record = json!({"files": [{"path": "../journal-outside.txt", "bytes": null}]});
        fs::write(pending.join("tasks-escape.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(recover(&root).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        fs::remove_file(&outside).unwrap();
    }

    /// Recovery acts only on a journal; a lock file without one belongs to no
    /// batch it can finish, so it is left alone.
    #[test]
    fn a_lock_without_a_journal_is_left_alone() {
        let root = root();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let lock = pending.join("tasks-orphan.lock");
        fs::write(&lock, b"").unwrap();
        recover(&root).unwrap();
        assert!(lock.exists());
    }

    /// Each newly kept file appends one journal line instead of rewriting the
    /// whole journal.
    #[test]
    fn a_batch_journals_each_file_as_one_line() {
        let root = root();
        let dir = board::store_dir(&root);
        let t2 = dir.join("t2.json");
        fs::write(&t2, b"{}").unwrap();
        let batch = batch(&root).unwrap();
        batch_keep(&dir.join("t1.json")).unwrap();
        batch_keep(&t2).unwrap();
        assert_eq!(fs::read_to_string(journal_path()).unwrap().lines().count(), 2);
        batch_rollback(batch);
    }

    /// Appended journal lines restore in reverse; a last line that was never
    /// finished names no change.
    #[test]
    fn an_appended_journal_puts_back_every_file() {
        let root = root();
        let dir = board::store_dir(&root);
        let before = fs::read(dir.join("t1.json")).unwrap();
        fs::write(dir.join("t1.json"), b"{}").unwrap();
        fs::write(dir.join("t2.json"), b"{}").unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let mut journal =
            [json!({"path": ".solace/tasks/t1.json", "bytes": before.clone()}), json!({"path": ".solace/tasks/t2.json", "bytes": null})]
                .iter()
                .map(|entry| serde_json::to_string(entry).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
        journal.push_str("\n{\"path\":");
        fs::write(pending.join("tasks-lines.json"), journal).unwrap();
        recover(&root).unwrap();
        assert_eq!(fs::read(dir.join("t1.json")).unwrap(), before);
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-lines.json").exists());
    }

    /// Queued events are logged after the files commit, and a rollback drops
    /// them.
    #[test]
    fn a_batch_logs_only_what_it_commits() {
        let root = root();
        let before = root_events(&root).len();
        let dropped = batch(&root).unwrap();
        batch_log("", &json!({"dropped": true}));
        batch_rollback(dropped);
        assert_eq!(root_events(&root).len(), before);
        let kept = batch(&root).unwrap();
        batch_log("", &json!({"kept": true}));
        batch_commit(kept).unwrap();
        assert_eq!(root_events(&root).last().unwrap(), &json!({"kept": true}));
    }

    /// A nested batch shares the outer journal and events, so an outer rollback
    /// puts back its files and logs none of its events.
    #[test]
    fn a_nested_batch_rolls_back_with_the_outer() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let journals = task_journals(&root);
        let inner = batch(&root).unwrap();
        assert_eq!(task_journals(&root), journals);
        batch_replace(&kept, &json!({"id": "t2"})).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_commit(inner).unwrap();
        assert_eq!(root_events(&root).len(), before, "a nested commit logs nothing yet");
        batch_rollback(outer);
        assert!(!kept.exists(), "the outer rollback puts back the nested file");
        assert_eq!(root_events(&root).len(), before, "the outer rollback logs no nested event");
    }

    /// A nested batch queues its events for the outer commit.
    #[test]
    fn a_nested_batch_logs_when_the_outer_commits() {
        let root = root();
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let inner = batch(&root).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_commit(inner).unwrap();
        assert_eq!(root_events(&root).len(), before);
        batch_commit(outer).unwrap();
        assert_eq!(root_events(&root).last(), Some(&json!({"nested": true})));
    }

    /// A nested rollback puts back the outer files and keeps the outer from
    /// logging anything.
    #[test]
    fn a_nested_batch_failure_stops_the_outer_commit() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let inner = batch(&root).unwrap();
        batch_replace(&kept, &json!({"id": "t2"})).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_rollback(inner);
        assert!(batch_commit(outer).is_err());
        assert!(!kept.exists());
        assert_eq!(root_events(&root).len(), before);
    }

    /// A feature follow-up that cannot be saved fails the batch, so the task
    /// and its event are put back too.
    #[test]
    fn a_feature_that_cannot_follow_its_task_rolls_back() {
        let root = root();
        let task = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let batch = batch(&root).unwrap();
        batch_replace(&task, &json!({"id": "t2"})).unwrap();
        let e = finish_write(&root, ".", &json!({"id": "t2"}), &[json!({"id": "bad/id"})]).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{}", e.0);
        batch_rollback(batch);
        assert!(!task.exists());
        assert_eq!(root_events(&root).len(), before, "the failed follow-up logs no task save");
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    fn journal_path() -> PathBuf {
        with_batch(|active| active.as_ref().expect("an open batch").journal.clone())
    }

    /// An outer handle dropped without a finish puts its files back and releases
    /// the thread's batch.
    #[test]
    fn dropping_an_outer_batch_rolls_it_back() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        {
            let _batch = batch(&root).unwrap();
            batch_replace(&kept, &json!({"id": "t2"})).unwrap();
            batch_log("", &json!({"dropped": true}));
        }
        assert!(!kept.exists());
        assert_eq!(root_events(&root).len(), before);
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
        assert!(with_batch(|active| active.is_none()));
    }

    /// An unfinished nested handle keeps the outer batch from committing.
    #[test]
    fn dropping_a_nested_batch_fails_the_outer() {
        let root = root();
        let outer = batch(&root).unwrap();
        drop(batch(&root).unwrap());
        assert!(batch_commit(outer).is_err());
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
        assert!(with_batch(|active| active.is_none()));
    }
}

/// Writes task `t`, returning what to log and the features that follow it.
/// `feature` is the feature being saved, when its tasks are written with it.
fn write_task(top: &Path, t: &Value, feature: Option<&Value>) -> Result<(String, Value, Vec<Value>), Failure> {
    let id = t["id"].as_str().unwrap_or("").to_string();
    if !board::valid_id(&id) {
        return board::err("E7001", "invalid task id");
    }
    // An active feature's tasks are never in Backlog (D-134).
    let mut t = t.clone();
    // The mark comes from the home on each load (D-160): it never goes in the file.
    t.as_object_mut().unwrap().remove("noProject");
    match feature {
        Some(f) => {
            board::plan_for_feature(top, &mut t, f);
        }
        None => {
            plan(top, &mut t);
        }
    }
    let path = board::store_dir(top).join(format!("{id}.json"));
    let was: Option<Value> = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
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
                        board::plan_for_feature(top, &mut t, &f);
                    }
                    changed.push(f);
                }
                Ok(None) => {}
                Err(e) => eprintln!("updating feature {fid}: {}", e.0),
            }
        }
    }
    batch_replace(&board::store_dir(top).join(format!("{id}.json")), &t)?;
    let home = t["home"].as_str().unwrap_or(".").to_string();
    Ok((home, t, changed))
}

fn finish_write(top: &Path, home: &str, t: &Value, changed: &[Value]) -> Result<(), Failure> {
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
fn write_tasks_then<T>(
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
struct WrittenTask {
    home: String,
    task: Value,
    changed: Vec<Value>,
}

/// Puts `<dir>/<name>` back as `before`; a file that did not exist is removed.
/// A failed restore is reported, so an inconsistent store leaves a trace, and
/// returns false.
fn restore_file(dir: &Path, name: &str, before: Option<Vec<u8>>) -> bool {
    let restored = match before {
        Some(bytes) => replace_bytes(dir, name, &bytes),
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
fn write_features(top: &Path, features: &[Value]) -> Result<(), Failure> {
    if features.is_empty() {
        return Ok(());
    }
    keep_slots(top)?;
    let dir = strategy_dir(top, "features");
    for f in features {
        let id = f["id"].as_str().unwrap_or("");
        if !board::valid_id(id) {
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
struct Batch {
    outer: bool,
    finished: bool,
}

/// Starts a batch, first putting back one a crash left unfinished.
fn batch(top: &Path) -> Result<Batch, Failure> {
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
fn batch_keep(p: &Path) -> Result<(), Failure> {
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
fn batch_replace(p: &Path, v: &Value) -> Result<(), Failure> {
    batch_keep(p)?;
    let dir = p.parent().unwrap_or(Path::new("."));
    let name = p.file_name().and_then(|n| n.to_str()).ok_or_else(|| Failure("invalid task store path".into(), 1))?;
    replace_bytes(dir, name, &serde_json::to_vec_pretty(v).unwrap())
}

/// Removes `p`, journaling its before bytes first.
fn batch_remove(p: &Path) -> Result<(), Failure> {
    batch_keep(p)?;
    fs::remove_file(p)?;
    fs::File::open(p.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}

/// Keeps the batch's files and drops its journal.
fn batch_commit(mut batch: Batch) -> Result<(), Failure> {
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

fn batch_rollback(mut batch: Batch) {
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
fn recover(top: &Path) -> Result<(), Failure> {
    if with_batch(|active| active.is_some()) {
        return Ok(());
    }
    let _lock = BATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    recover_locked(top)
}

/// Writes a record and logs it as a root event, in the open batch.
fn save_record_in(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    let id = v["id"].as_str().unwrap_or("");
    if !board::valid_id(id) {
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
fn with_batch<R>(f: impl FnOnce(&mut Option<ActiveBatch>) -> R) -> R {
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
            p.extension().is_some_and(|e| e == "json")
                && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tasks-"))
        })
        .map(|p| p.with_extension(""))
        .collect();
    for stem in stems {
        let lock = stem.with_extension("lock");
        let Some(owner) = try_lock_file(&lock)? else { continue };
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
            replace_bytes(dir, name, &crate::json_bytes(a))?;
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
    let owner = try_lock_file(&journal.with_extension("lock"))?.ok_or_else(|| Failure("the task journal is already in use".into(), 1))?;
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
fn batch_log(project: &str, data: &Value) {
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
struct ActiveBatch {
    top: PathBuf,
    journal: PathBuf,
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
