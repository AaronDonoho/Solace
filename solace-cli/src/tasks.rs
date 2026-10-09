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
    let id = t["id"].as_str().unwrap_or("");
    if !board::valid_id(id) {
        return board::err("E7001", "invalid task id");
    }
    replace_file(&board::store_dir(top), id, t)?;
    log(top, t["home"].as_str().unwrap_or("."), &json!({"task":t}));
    Ok(())
}
/// Writes `<dir>/<id>.json` as `v`, synced like an inbox message and renamed
/// into place, so a power loss never leaves an empty file.
fn replace_file(dir: &Path, id: &str, v: &Value) -> Result<(), Failure> {
    fs::create_dir_all(dir)?;
    let (part, to) = (dir.join(format!("{id}.json.tmp")), dir.join(format!("{id}.json")));
    let mut f = fs::File::create(&part)?;
    f.write_all(&serde_json::to_vec_pretty(v).unwrap())?;
    f.sync_all()?;
    fs::rename(&part, &to)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// The states of a milestone or a feature (D-87 point 8).
const STATES: [&str; 5] = ["proposed", "planned", "active", "complete", "canceled"];

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
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(strategy_dir(top, kind)) else { return out };
    for p in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        match fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
            Ok(v) => out.push(v),
            Err(e) => eprintln!("skipping record file {}: {e}", p.display()),
        }
    }
    out.sort_by(|a, b| rank_of(a).total_cmp(&rank_of(b)).then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    out
}
pub(super) fn record(top: &Path, kind: &str, id: &str) -> Option<Value> {
    if !board::valid_id(id) {
        return None;
    }
    serde_json::from_slice(&fs::read(strategy_dir(top, kind).join(format!("{id}.json"))).ok()?).ok()
}
/// Writes a record and logs it as a root event: no project (D-87 point 7).
fn put_record(top: &Path, kind: &str, v: &Value) -> Result<(), Failure> {
    let id = v["id"].as_str().unwrap_or("");
    if !board::valid_id(id) {
        return board::err("E7001", "invalid id");
    }
    replace_file(&strategy_dir(top, kind), id, v)?;
    let event = if kind == "milestones" { json!({"milestone": v}) } else { json!({"feature": v}) };
    log(top, "", &event);
    Ok(())
}
fn remove_record(top: &Path, kind: &str, id: &str) -> Result<(), Failure> {
    let dir = strategy_dir(top, kind);
    fs::remove_file(dir.join(format!("{id}.json")))?;
    fs::File::open(&dir)?.sync_all()?;
    let event = if kind == "milestones" { json!({"removedMilestone": id}) } else { json!({"removedFeature": id}) };
    log(top, "", &event);
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
        return Ok(v);
    }
    let mut v = json!({"id": unique_id(), "title": title, "body": req["body"].as_str().unwrap_or(""),
        "state": req["state"].as_str().unwrap_or("proposed")});
    if kind == "milestones" {
        v["targetDate"] = req.get("targetDate").cloned().unwrap_or(Value::Null);
    } else {
        v["milestone"] = linked(top, req, status)?;
    }
    v["rank"] = json!(end_rank(top, kind, &v["milestone"], ""));
    if let Some(r) = request {
        v["request"] = json!(r);
    }
    put_record(top, kind, &v)?;
    Ok(v)
}
fn edit_record(top: &Path, kind: &str, id: &str, req: &Value, status: &mut u16) -> Result<Value, Failure> {
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
    put_record(top, kind, &v)?;
    Ok(v)
}
/// Deletes a record. Its features and tasks lose their link first, so a crash
/// between the steps leaves no link to a missing record (D-87 point 5).
fn delete_record(top: &Path, kind: &str, id: &str, status: &mut u16) -> Result<Value, Failure> {
    let Some(v) = record(top, kind, id) else { return unknown_id(kind, id, status, 404) };
    if kind == "milestones" {
        for mut f in records(top, "features").into_iter().filter(|f| f["milestone"] == id) {
            f["milestone"] = Value::Null;
            put_record(top, "features", &f)?;
        }
    }
    let link = singular(kind);
    for mut t in board::all_tasks(top)?.into_iter().filter(|t| t[link] == id) {
        t[link] = Value::Null;
        write(top, &t)?;
    }
    remove_record(top, kind, id)?;
    Ok(v)
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
                    return Err(Failure("the task server could not apply the message; it was set aside in .solace/tasks/inbox/failed".into(), 1));
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
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
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
    board::after(&scope::path(&scope::top(root), t["home"].as_str().unwrap_or(".")), &t, after);
    drop(guard);
    Ok(t)
}
/// One change to a task, as an operation. Without its projects' locks
/// (`held` false) it checks again inside the operation, where nothing else
/// writes the task, and returns None, changing nothing, when the change now
/// needs them (`needs` says). The operation never takes a lock itself (see `op`).
fn change(root: &Path, method: &str, parts: &[&str], body: &Value, held: bool, needs: NeedsLock) -> Result<Option<(Value, board::After)>, Failure> {
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
        board::adopt_tasks(&root, &root).unwrap();
        root
    }
    fn stored(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(board::store_dir(root)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
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
        let b = call("POST", &["milestones"], json!({"title": "2.0", "state": "planned", "targetDate": "2027-01-01"})).unwrap();
        assert_eq!((a["state"].as_str(), b["state"].as_str(), b["targetDate"].as_str()), (Some("proposed"), Some("planned"), Some("2027-01-01")));
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

        call("DELETE", &["features", f3id], json!({})).unwrap();
        assert_eq!(titles("features"), ["F1", "F2"]);
        call("DELETE", &["milestones", bid], json!({})).unwrap();
        assert_eq!(titles("milestones"), ["1.0 final"]);
        assert!(!strategy_dir(&root, "milestones").join(format!("{bid}.json")).exists());
        assert!(!strategy_dir(&root, "features").read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
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
        let keys: Vec<String> = rows.iter().map(|d| serde_json::from_str::<Value>(d).unwrap().as_object().unwrap().keys().next().unwrap().clone()).collect();
        assert_eq!(keys, ["milestone", "feature", "feature", "removedMilestone"]);
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
        fs::write(dir.join("t2.json"), r#"{"id":"t2","column":"complete","home":".","runs":[{"ended":50}],"finished":{"time":70}}"#).unwrap();
        fs::write(dir.join("t3.json"), r#"{"id":"t3","column":"complete","home":".","runs":[]}"#).unwrap();
        assert_eq!(board::task(&root, "t2").unwrap()["closedTime"], json!(70));
        let modified = fs::metadata(dir.join("t3.json")).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(board::task(&root, "t3").unwrap()["closedTime"], json!(modified));
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

    /// D-87: a feature brings its milestone to a task with none; links are checked,
    /// editable in a final column and during a run, and cleared by a delete.
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
        let t = patch(t1, json!({"feature": f["id"]})).unwrap();
        assert_eq!((&t["feature"], &t["milestone"]), (&f["id"], &m1["id"]), "an empty milestone takes the feature's");
        let t = patch(t1, json!({"milestone": m2["id"]})).unwrap();
        let t2 = api(&root, "POST", &["tasks"], json!({"title": "T2"})).unwrap();
        let t2id = t2["id"].as_str().unwrap();
        assert_eq!(patch(t1, json!({"feature": f["id"]})).unwrap()["milestone"], m2["id"], "a set milestone stays");
        assert_eq!(patch(t2id, json!({"feature": f["id"], "milestone": null})).unwrap()["milestone"], Value::Null, "the request's wins");
        assert_eq!(code(patch(t1, json!({"feature": "nope"}))), "E7023");
        assert_eq!(code(patch(t1, json!({"dependsOn": [t1]}))), "E7024");
        assert_eq!(code(patch(t1, json!({"dependsOn": ["nope"]}))), "E7024");
        assert_eq!(patch(t1, json!({"dependsOn": [t2id]})).unwrap()["dependsOn"], json!([t2id]));
        assert_eq!(code(patch(t2id, json!({"dependsOn": [t1]}))), "E7025");
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], t["milestone"]);

        // During a run, and in a final column, only the links change; an empty edit is refused too.
        let mut busy = board::task(&root, t2id).unwrap();
        busy["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        write(&root, &busy).unwrap();
        assert_eq!(patch(t2id, json!({"milestone": m1["id"]})).unwrap()["milestone"], m1["id"]);
        assert_eq!(code(patch(t2id, json!({"title": "no"}))), "E7003");
        assert_eq!(code(patch(t2id, json!({}))), "E7003");
        let mut done = board::task(&root, t2id).unwrap();
        done["runs"] = json!([]);
        done["column"] = json!("complete");
        write(&root, &done).unwrap();
        assert_eq!(patch(t2id, json!({"milestone": null})).unwrap()["milestone"], Value::Null);
        assert_eq!(code(patch(t2id, json!({"title": "no", "feature": null}))), "E7004");
        assert_eq!(code(patch(t2id, json!({}))), "E7004");

        call("DELETE", &["features", f["id"].as_str().unwrap()], json!({}));
        for id in [t1, t2id] {
            assert_eq!(board::task(&root, id).unwrap()["feature"], Value::Null);
        }
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], m2["id"], "the milestone stays");
        call("DELETE", &["milestones", m2["id"].as_str().unwrap()], json!({}));
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], Value::Null);
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
        let texts: Vec<String> = board::task(&root, "t1").unwrap()["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap().to_string()).collect();
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
        for m in [note(json!("../x")), note(json!("")), note(json!(null)), note(json!("missing")), note(json!("junk")), note(json!("bare"))] {
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
            if let Some(f) = fs::read_dir(inbox(top)).ok().and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "json"))) {
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

    /// A workflow whose `finished` event targets a final column, so `task done` settles.
    const DONE_IS_FINAL: &str = "[board]\nevents = { asked = \"todo\", finished = \"done\", failed = \"todo\" }\n[[board.columns]]\nid = \"todo\"\nactions = [\"cancel\"]\n[[board.columns]]\nid = \"done\"\nfinal = true\n[[board.columns]]\nid = \"gone\"\nfinal = true\n[board.actions]\ncancel = { revert = true, to = \"gone\" }\n";

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
        board::record(&s, &tid, "r1", &before, &board::snapshot(&s).unwrap()).unwrap();
        let has_before = || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
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
}
