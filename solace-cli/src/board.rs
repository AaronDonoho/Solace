use super::*;
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

pub fn lock(root: &Path) -> Result<fs::File, Failure> {
    fs::create_dir_all(root.join(".solace"))?;
    // one project lock; use per-change store isolation if parallel work is introduced.
    lock_file(&root.join(".solace/lock"))
}
/// The project lock when it is free right now; the board's poll skips a busy project.
pub(crate) fn try_lock(root: &Path) -> Option<fs::File> {
    try_lock_file(&root.join(".solace/lock")).ok()?
}
pub fn open(root: &Path) -> Result<Store, Failure> {
    let s = Store::at(root)?;
    fs::create_dir_all(root.join(".solace/board"))?;
    fs::create_dir_all(store_dir(&scope::top(root)))?;
    Ok(s)
}
/// The root's task store (D-77): every task of every project, one file each.
pub(super) fn store_dir(top: &Path) -> PathBuf {
    top.join(".solace/tasks")
}
pub(super) fn err<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "task", message)
}
/// Whether `id` can name a task: it names the task's file and folder, so only
/// letters, digits and `-`.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
fn task_path(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    if !valid_id(id) {
        return err("E7001", "invalid task id");
    }
    Ok(store_dir(&scope::top(root)).join(format!("{id}.json")))
}
/// The task's own folder in a project: run logs, conversations, reviews, files.
fn task_dir(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    let _ = task_path(root, id)?;
    Ok(root.join(".solace/board").join(id))
}
/// A task whose home is the project `root`. A task whose home is no project is
/// re-homed on load (D-160); one with no project at all is opened from the
/// root, which stores it, and says `noProject` (D-129).
pub(super) fn task(root: &Path, id: &str) -> Result<Value, Failure> {
    let path = task_path(root, id)?;
    let top = scope::top(root);
    if !path.exists() {
        return err("E7001", "unknown task");
    }
    let mut t = rehome_task(&top, read_task(&top, &path)?)?;
    let dir = scope::dir(&top, root);
    if t["home"] != dir {
        if dir != "." || !homeless(&top, t["home"].as_str().unwrap_or("")) {
            return err("E7001", "unknown task");
        }
        t["noProject"] = json!(true);
    }
    Ok(t)
}
/// Whether `home` names no project of the workspace (D-129): the task at that
/// home is in no lane, and opening it at its home would create a store there.
/// A project list that can't be read counts every home as a project.
pub(super) fn homeless(top: &Path, home: &str) -> bool {
    workspace::members(top).is_ok_and(|ms| !ms.iter().any(|m| m == home))
}
/// The home and scope a task whose home is no project takes (D-160): the first
/// project of its scope, keeping only the projects of it, else the root with
/// scope `["."]` when the root is a project. None when the root is no project:
/// the task keeps no project. None while a run of the task is live: the agent
/// works from the task's folder, and the load must not move it out from under
/// the run (as `set_scope` says for a home change).
pub(super) fn rehome_to(members: &[String], t: &Value) -> Option<(String, Vec<String>)> {
    let home = t["home"].as_str().unwrap_or(".");
    if members.iter().any(|m| m == home) || active(t) {
        return None;
    }
    let kept: Vec<String> = scope::of(t, home).into_iter().filter(|d| members.iter().any(|m| m == d.as_str())).collect();
    match kept.first() {
        Some(d) => Some((d.clone(), kept)),
        None if members.iter().any(|m| m == ".") => Some((".".to_string(), vec![".".to_string()])),
        None => None,
    }
}
/// A task whose home is no project is re-homed and saved (D-160); any other comes back as it is.
fn rehome_task(top: &Path, t: Value) -> Result<Value, Failure> {
    match workspace::members(top) {
        Ok(ms) if rehome_to(&ms, &t).is_some() => tasks::rehome(top, &t),
        _ => Ok(t),
    }
}
/// The project to open task `t` from: its home, or the root when its home is
/// no project (D-129).
pub(super) fn task_root(top: &Path, t: &Value) -> PathBuf {
    let home = t["home"].as_str().unwrap_or(".");
    if homeless(top, home) { top.to_path_buf() } else { scope::path(top, home) }
}
/// The tasks whose home is the project `root`, in rank order.
pub(super) fn tasks(root: &Path) -> Result<Vec<Value>, Failure> {
    let top = scope::top(root);
    let home = scope::dir(&top, root);
    Ok(all_tasks(&top)?.into_iter().filter(|t| t["home"] == home).collect())
}
/// Every task in the root's store, in rank order.
pub(super) fn all_tasks(top: &Path) -> Result<Vec<Value>, Failure> {
    let mut out = vec![];
    let dir = store_dir(top);
    if !dir.exists() {
        return Ok(out);
    }
    // Each home's workflow and each feature is read at most once per listing.
    let mut flows = HashMap::new();
    let mut features = HashMap::new();
    // A list that can't be read counts every home as a project, as `homeless` does.
    let members = workspace::members(top);
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|x| x == "json") {
            // One unreadable file (empty after a crash) must not hide the rest.
            match read_task_with(top, &p, &mut flows, &mut features) {
                Ok(t) => {
                    // A re-home that fails (a folder that can't move) is logged and
                    // leaves the task as it was, marked: one task must not hide the listing.
                    let mut t = match &members {
                        Ok(ms) if rehome_to(ms, &t).is_some() => match tasks::rehome(top, &t) {
                            Ok(moved) => moved,
                            Err(e) => {
                                eprintln!("re-homing task {}: {}", t["id"].as_str().unwrap_or(""), e.0);
                                t
                            }
                        },
                        _ => t,
                    };
                    if members.as_ref().is_ok_and(|ms| !ms.iter().any(|m| m == t["home"].as_str().unwrap_or(""))) {
                        t["noProject"] = json!(true);
                    }
                    out.push(t);
                }
                Err(e) => eprintln!("skipping task file {}: {}", p.display(), e.0),
            }
        }
    }
    out.sort_by(|a: &Value, b: &Value| a["rank"].as_f64().unwrap_or(0.).total_cmp(&b["rank"].as_f64().unwrap_or(0.)));
    Ok(out)
}
pub(super) fn read_task(top: &Path, path: &Path) -> Result<Value, Failure> {
    read_task_with(top, path, &mut HashMap::new(), &mut HashMap::new())
}
/// `read_task`, taking each home's workflow and each feature from `flows` and
/// `features` once they have been read.
fn read_task_with(
    top: &Path,
    path: &Path,
    flows: &mut HashMap<PathBuf, Value>,
    features: &mut HashMap<String, Option<Value>>,
) -> Result<Value, Failure> {
    let mut t: Value = serde_json::from_slice(&fs::read(path)?).map_err(|e| Failure(e.to_string(), 1))?;
    // The default workflow's `done` column is now `complete`; tasks saved before still say `done`.
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let mut workflow = |home: &PathBuf| -> Result<Value, Failure> {
        if !flows.contains_key(home) {
            flows.insert(home.clone(), flow(home)?);
        }
        Ok(flows[home].clone())
    };
    // Only these two old columns need the workflow, so other tasks read without it.
    let old_column = matches!(t["column"].as_str(), Some("done" | "blocked"));
    let columns = if old_column { workflow(&home)?["columns"].as_array().unwrap().clone() } else { vec![] };
    if t["column"] == "done" && !columns.iter().any(|c| c["id"] == "done") {
        t["column"] = json!("complete");
    }
    // The default workflow has no `blocked` column (D-82): a task saved there stays in
    // `assigned`, flagged as held until its run continues.
    if t["column"] == "blocked" && !columns.iter().any(|c| c["id"] == "blocked") && columns.iter().any(|c| c["id"] == "assigned") {
        t["column"] = json!("assigned");
        if t["attention"].is_null() {
            t["attention"] = json!({"reason":"the agent needed an answer or stopped","time":now()});
        }
        t["attention"]["hold"] = json!(true);
    }
    // Tasks saved before D-87 have no links; one already in a final column takes the
    // time it last finished, or else its file's, as when it closed.
    let modified = || {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    };
    if t.get("closedTime").is_none() {
        for k in ["feature", "milestone", "closedTime", "archivedTime"] {
            t[k] = Value::Null;
        }
        if t.get("dependsOn").is_none() {
            t["dependsOn"] = json!([]);
        }
        if column(&workflow(&home)?, &t)["final"] == true {
            let ended = t["runs"].as_array().into_iter().flatten().filter_map(|r| r["ended"].as_u64());
            let last = ended.chain(t["finished"]["time"].as_u64()).max();
            t["closedTime"] = json!(last.unwrap_or_else(modified));
        }
    }
    // A task with a feature takes the feature's milestone on load (D-188).
    if let Some(fid) = t["feature"].as_str().map(String::from) {
        let feature = features.entry(fid).or_insert_with_key(|fid| tasks::record(top, "features", fid));
        if let Some(f) = feature {
            t["milestone"] = f["milestone"].clone();
        }
    }
    // Tasks saved before D-130 have no createdTime: the earlier of their first note's
    // time and their first run's start, or else their file's.
    if t.get("createdTime").is_none() {
        let first_note = t["notes"][0]["time"].as_u64();
        let first_run = t["runs"][0]["started"].as_u64();
        t["createdTime"] = json!(first_note.into_iter().chain(first_run).min().unwrap_or_else(modified));
    }
    // The mark comes from the home on each load (D-160): one saved by an older build is dropped.
    t.as_object_mut().unwrap().remove("noProject");
    Ok(t)
}
/// Moves a project's tasks from before D-77 into the root's store, once, with
/// the project as their home. Called under the project's serve lock.
pub(super) fn adopt_tasks(top: &Path, root: &Path) -> Result<(), Failure> {
    let old = root.join(".solace/board");
    if !old.exists() {
        return Ok(());
    }
    let home = scope::dir(top, root);
    fs::create_dir_all(store_dir(top))?;
    for entry in fs::read_dir(&old)? {
        let p = entry?.path();
        if !p.is_file() || p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let mut t: Value = serde_json::from_slice(&fs::read(&p)?).map_err(|e| Failure(e.to_string(), 1))?;
        let to = task_path(root, t["id"].as_str().unwrap_or(""))?;
        if to.exists() {
            // A crash between writing the store copy and removing this one: finish the move.
            let stored: Option<Value> = fs::read(&to).ok().and_then(|b| serde_json::from_slice(&b).ok());
            if stored.is_some_and(|s| s["home"] == json!(home)) {
                fs::remove_file(&p)?;
                continue;
            }
            return fail(format!(
                "task {} is in {} and also in the root's task store under another project (or unreadable); the id is used twice, e.g. by a copied project folder. Delete or rename one copy before `solace serve`",
                t["id"].as_str().unwrap(),
                p.display()
            ));
        }
        // The change entries move into the store of the project each was made in
        // (D-77 point 4); the task file keeps none.
        let tid = t["id"].as_str().unwrap_or("");
        for c in t["changes"].as_array().cloned().unwrap_or_default() {
            let (Some(entity), Some(dir)) = (c["entity"].as_str(), c["project"].as_str().or(Some(home.as_str()))) else { continue };
            let st = open(&scope::path(top, dir))?;
            let (run, name) = (c["run"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""));
            // Rows from before D-77 have no run or name: fill the first in, so a
            // repeated move adds nothing.
            let filled = st.db.execute(
                "UPDATE board_touches SET run=?3, name=?4 WHERE seq=(SELECT MIN(seq) FROM board_touches WHERE task=?1 AND entity=?2 AND run IS NULL)",
                params![tid, entity, run, name],
            )?;
            let known: i64 =
                st.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task=?1 AND entity=?2", params![tid, entity], |r| r.get(0))?;
            if filled == 0 && known == 0 {
                st.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![tid, entity, run, name])?;
            }
        }
        t.as_object_mut().unwrap().remove("changes");
        // Older task files may lack lists the server indexes into.
        for k in ["runs", "notes", "questions", "links"] {
            if !t[k].is_array() {
                t[k] = json!([]);
            }
        }
        t["home"] = json!(home);
        write_atomic(&to, serde_json::to_vec_pretty(&t).unwrap())?;
        fs::remove_file(&p)?;
    }
    Ok(())
}
fn event(s: &Store, v: Value) -> Result<(), Failure> {
    s.db.execute("INSERT INTO board_events(data) VALUES (?)", [v.to_string()])?;
    Ok(())
}
/// A project's workflow: its root's, shared by every project on the board (D-61).
pub fn flow(project: &Path) -> Result<Value, Failure> {
    workflow(&lockdown::settings(&scope::top(project))?)
}
pub fn workflow(settings: &toml::Value) -> Result<Value, Failure> {
    let default: toml::Value = include_str!("default-board.toml").parse().unwrap();
    let b = settings.get("board").unwrap_or(&default["board"]);
    let mut w = serde_json::to_value(b).map_err(|e| Failure(e.to_string(), 1))?;
    let invalid = |m: &str| Failure(format!("E6020 invalid board workflow\n  {m}"), 1);
    let cols = w["columns"].as_array().ok_or_else(|| invalid("columns must be an array"))?;
    if cols.is_empty() {
        return Err(invalid("empty columns"));
    }
    let mut ids = HashSet::new();
    let mut allowed = HashSet::new();
    let actions = w["actions"].as_object().ok_or_else(|| invalid("actions must be a table"))?;
    for c in cols {
        if c.get("actions").is_some_and(|v| !v.is_array())
            || c.get("final").is_some_and(|v| !v.is_boolean())
            || c.get("label").is_some_and(|v| !v.is_string())
            || c.get("requires").is_some_and(|v| v.as_array().is_none_or(|a| a.iter().any(|x| !x.is_string())))
        {
            return Err(invalid("invalid column fields"));
        }
        let id = c["id"].as_str().ok_or_else(|| invalid("column id missing"))?;
        if !ids.insert(id.to_string()) {
            return Err(invalid("duplicate column"));
        }
        // A final column may list actions too (D-108), such as cancelled's way back to planning (D-133).
        let a = c.get("actions").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        for name in a {
            let n = name.as_str().ok_or_else(|| invalid("action name"))?;
            if !actions.contains_key(n) {
                return Err(invalid("unknown action"));
            }
            allowed.insert(n.to_string());
        }
    }
    for c in cols {
        if let Some(next) = c.get("next")
            && !ids.contains(next.as_str().unwrap_or(""))
        {
            return Err(invalid("unknown gate next"));
        }
    }
    for (name, a) in actions {
        if !a.is_object()
            || ["stamp", "unstamp", "label"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_string()))
            || ["model", "revert"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_boolean()))
        {
            return Err(invalid("invalid action fields"));
        }
        if !allowed.contains(name) {
            return Err(invalid("action never allowed"));
        }
        if a.get("to").is_some_and(|x| !ids.contains(x.as_str().unwrap_or(""))) {
            return Err(invalid("unknown action column"));
        }
        if a.get("run").is_some_and(|x| !matches!(x.as_str(), Some("investigate" | "work" | "new_work" | "review"))) {
            return Err(invalid("unknown run step"));
        }
    }
    // `finished` needs a target; `asked` and `failed` may have none, and then the task stays put (D-82).
    for e in ["asked", "finished", "failed"] {
        let target = w["events"].get(e).filter(|v| !v.is_null());
        if target.is_none_or(|v| !ids.contains(v.as_str().unwrap_or(""))) && (e == "finished" || target.is_some()) {
            return Err(invalid("event has no target"));
        }
    }
    for c in w["columns"].as_array_mut().unwrap() {
        if c.get("label").is_none() {
            c["label"] = c["id"].clone();
        }
        if c.get("actions").is_none() {
            c["actions"] = json!([]);
        }
        if c.get("final").is_none() {
            c["final"] = json!(false);
        }
    }
    Ok(w)
}
pub(super) fn column<'a>(w: &'a Value, t: &Value) -> &'a Value {
    w["columns"].as_array().unwrap().iter().find(|c| c["id"] == t["column"]).unwrap_or(&Value::Null)
}
fn allowed(w: &Value, t: &Value) -> Vec<String> {
    let c = column(w, t);
    let mut out = c["actions"].as_array().unwrap_or(&vec![]).iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>();
    // A final column offers only the actions it lists: no agent works there (D-108).
    if c["final"] != true {
        out.extend(["pause", "resume", "discuss", "stop", "end_discussion"].map(String::from));
    }
    // A task with a feature goes back to the backlog only through its feature (D-132).
    out.retain(|a| !held_by_feature(w, t, a));
    out
}
/// Whether the task's column offers action `name` but the task's feature
/// holds it back: a task with a feature goes back to the backlog only through
/// its feature (D-132 point 1).
fn held_by_feature(w: &Value, t: &Value, name: &str) -> bool {
    name == "to_backlog" && t["feature"].is_string() && column(w, t)["actions"].as_array().is_some_and(|a| a.iter().any(|x| x == name))
}
/// The task's discussion run that hasn't ended, if one is going.
fn discussion(t: &Value) -> Option<String> {
    t["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "discuss" && r["state"] != "ended")
        .and_then(|r| r["id"].as_str())
        .map(String::from)
}
/// Replaces a discussion's notes with one verbatim transcript, each turn
/// labeled with who said it.
fn compact(t: &mut Value, rid: &str) {
    let model = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).and_then(|r| r["model"].as_str()).unwrap_or("").to_string();
    let notes = t["notes"].as_array_mut().unwrap();
    let (turns, rest): (Vec<Value>, Vec<Value>) = notes.drain(..).partition(|n| n["run"] == rid);
    *notes = rest;
    if turns.is_empty() {
        return;
    }
    note(t, "solace", Some(rid), &conversation(&turns, &model));
}
/// A conversation's turns as one verbatim transcript, each labeled with who
/// said it: a discussion's, or a strategy run's (D-93).
pub(super) fn conversation(turns: &[Value], model: &str) -> String {
    let lines: Vec<String> = turns
        .iter()
        .map(|n| {
            let who = match n["author"].as_str() {
                Some("human") => "Human".to_string(),
                Some("solace") => "Solace".to_string(),
                _ => format!("Agent ({model})"),
            };
            // The label stands on its own line, so a turn that opens with a heading or list still renders.
            format!("**{who}:**\n\n{}", n["text"].as_str().unwrap_or("").trim())
        })
        .collect();
    format!("### Discussion\n\n{}", lines.join("\n\n"))
}
pub(super) fn active(t: &Value) -> bool {
    t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended")
}
pub(super) fn note(t: &mut Value, author: &str, run: Option<&str>, text: &str) {
    t["notes"].as_array_mut().unwrap().push(json!({"author":author,"run":run,"time":now(),"text":text}));
}
/// Marks a task as needing a human. The next action a human takes on it clears the mark.
pub(super) fn flag(t: &mut Value, reason: &str) {
    t["attention"] = json!({"reason":reason,"time":now()});
    t["finished"] = Value::Null;
}
/// Marks a task whose agent is held on a question or stopped before finishing
/// (D-82). The mark stays through other actions until the run continues, is
/// restarted, or the task is stopped or settled; `blocked` is reserved for tasks that depend on other tasks.
pub(super) fn hold(t: &mut Value, reason: &str) {
    flag(t, reason);
    t["attention"]["hold"] = json!(true);
}
/// Moves the task to the column an event names; an event with no target leaves it where it is (D-82).
fn on_event(w: &Value, t: &mut Value, event: &str) -> Result<(), Failure> {
    match w["events"].get(event) {
        Some(to) if !to.is_null() => move_to(w, t, to),
        _ => Ok(()),
    }
}
pub(super) fn move_to(w: &Value, t: &mut Value, to: &Value) -> Result<(), Failure> {
    let c = column(w, t);
    let requires = c["requires"].as_array().cloned().unwrap_or_default();
    if &c["next"] == to {
        if requires.iter().any(|r| t["stamps"].get(r.as_str().unwrap_or("")).is_none()) {
            return err("E7006", "required human stamp missing");
        }
    } else if &t["column"] != to {
        for r in requires {
            t["stamps"].as_object_mut().unwrap().remove(r.as_str().unwrap_or(""));
        }
    }
    t["column"] = to.clone();
    // When the task first reaches a final column; never cleared (D-87).
    if column(w, t)["final"] == true && t["closedTime"].is_null() {
        t["closedTime"] = json!(now());
    }
    // A task that leaves the final columns is back on the board (D-95).
    if column(w, t)["final"] != true {
        t["archivedTime"] = Value::Null;
    }
    Ok(())
}
/// Where run `rid` is in the task's runs.
fn run_at(t: &Value, rid: &str) -> Result<usize, Failure> {
    t["runs"].as_array().and_then(|runs| runs.iter().position(|r| r["id"] == rid)).ok_or_else(|| Failure("E7003 unknown run".into(), 1))
}
pub(super) fn end_run(r: &mut Value, outcome: &str) {
    r["state"] = json!("ended");
    r["ended"] = json!(now());
    r["outcome"] = json!(outcome);
    // The model the agent's start event names, when it names one: an alias
    // shows the model the run used, in the run and in its label.
    let log = PathBuf::from(r["log"].as_str().unwrap_or(""));
    if let Some(used) = used_model(&log) {
        r["used"] = json!(used);
        agents::record_used(r["model"].as_str().unwrap_or(""), &used);
    }
}

/// A run's start time and outcome as the task holds them now: `after_run`
/// clears a usage-limit mark only for a run that succeeded, started before it.
fn run_state(root: &Path, tid: &str, rid: &str) -> (u64, Option<String>) {
    task(root, tid)
        .ok()
        .and_then(|t| {
            t["runs"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .find(|r| r["id"] == rid)
                .map(|r| (r["started"].as_u64().unwrap_or(0), r["outcome"].as_str().map(String::from)))
        })
        .unwrap_or((now(), None))
}

/// The model a run's log start event names, when it names one: Codex in its
/// `thread.started`, Claude Code in its `system`/`init` event.
fn used_model(log: &Path) -> Option<String> {
    use std::io::BufRead;
    // The start event is at the log's head: read line by line and stop after
    // the first 50, so a long log is not read whole on every run's end.
    let Ok(file) = fs::File::open(log) else { return None };
    std::io::BufReader::new(file).lines().take(50).filter_map(|l| serde_json::from_str::<Value>(&l.ok()?).ok()).find_map(|e| {
        if e["type"] == "thread.started" || (e["type"] == "system" && e["subtype"] == "init") {
            e["model"].as_str().map(String::from)
        } else {
            None
        }
    })
}
fn end_runs(t: &mut Value, outcome: &str) {
    for r in t["runs"].as_array_mut().unwrap() {
        if r["state"] != "ended" {
            end_run(r, outcome);
        }
    }
    t["paused_from"] = Value::Null;
}
/// The system settings file: `SOLACE_CONFIG`, else `~/.config/solace/config.toml`.
fn config_path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_CONFIG.with(|p| p.borrow().clone()) {
        return path;
    }
    env::var_os("SOLACE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config/solace/config.toml"))
}
pub fn system() -> Result<Value, Failure> {
    let p = config_path();
    let mut v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    // Each agent's discovered models merge in: presets, then discovery, then
    // the human's config, which wins for the same id.
    v["models"] = json!(agents::merge(&v["models"]));
    Ok(v)
}
fn parse_system(text: &str) -> Result<Value, Failure> {
    let v: toml::Value = text.parse().map_err(|e| Failure(format!("E6020 system settings: {e}"), 1))?;
    let mut v = serde_json::to_value(v).map_err(|e| Failure(e.to_string(), 1))?;
    let invalid = || {
        Failure(
            "E6020 invalid system settings: expected positive task_timeout, a [languages] table of true/false gleam and rust, \
             and [[models]] with id, label, agent and model or run argv; an endpoint needs kind \"openai\" (agent codex) \
             or \"anthropic\" (agent claude) and base_url"
                .into(),
            1,
        )
    };
    // Built-in presets first; configured models with the same id replace them.
    let mut models: Vec<Value> = if v["presets"] == false {
        vec![]
    } else {
        let presets: toml::Value = include_str!("default-models.toml").parse().unwrap();
        serde_json::to_value(presets).unwrap()["models"].as_array().unwrap().clone()
    };
    for mut m in v.get("models").map(|m| m.as_array().cloned().ok_or_else(invalid)).transpose()?.unwrap_or_default() {
        // A configured model keeps its entry when a discovery names its id.
        m["configured"] = json!(true);
        match models.iter().position(|x| x["id"] == m["id"]) {
            Some(i) => models[i] = m,
            None => models.push(m),
        }
    }
    v["models"] = json!(models);
    if v.get("task_timeout").is_none() {
        v["task_timeout"] = json!(3600);
    }
    if v.get("task_timeout").is_some_and(|v| v.as_u64().is_none_or(|n| n == 0)) {
        return Err(invalid());
    }
    if let Some(languages) = v.get("languages") {
        let Some(languages) = languages.as_object() else {
            return Err(invalid());
        };
        if languages.keys().any(|id| !matches!(id.as_str(), "gleam" | "rust")) || languages.values().any(|enabled| !enabled.is_boolean()) {
            return Err(invalid());
        }
    }
    let languages = json!({
        "gleam": v.get("languages").and_then(|l| l.get("gleam")).cloned().unwrap_or(json!(true)),
        "rust": v.get("languages").and_then(|l| l.get("rust")).cloned().unwrap_or(json!(true)),
    });
    v["languages"] = languages;
    if let Some(models) = v.get("models") {
        let mut ids = HashSet::new();
        for m in models.as_array().ok_or_else(invalid)? {
            if m["id"].as_str().is_none_or(|id| id.is_empty() || !ids.insert(id))
                || !m["label"].is_string()
                || !matches!(m["agent"].as_str(), Some("codex" | "claude"))
            {
                return Err(invalid());
            }
            if let Some(argv) = m.get("run") {
                if argv.as_array().is_none_or(|a| a.is_empty() || a.iter().any(|v| !v.is_string())) {
                    return Err(invalid());
                }
            } else if !m["model"].is_string() {
                return Err(invalid());
            }
            if let Some(e) = m.get("endpoint") {
                let kind_matches = matches!(
                    (e["kind"].as_str(), m["agent"].as_str()),
                    (Some("openai"), Some("codex")) | (Some("anthropic"), Some("claude"))
                );
                if !kind_matches
                    || !e["base_url"].is_string()
                    || e.get("key_env").is_some_and(|k| !k.is_string())
                    || e.get("stored_key").is_some_and(|k| !k.is_boolean())
                {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(v)
}

/// Codex settings for a model's OpenAI-compatible endpoint, if it has one. The
/// key stays in the variable the endpoint names, or with a `stored` key in
/// `setup::KEY_VAR`, set on the agent's process; Codex reads it at launch.
/// Current Codex speaks only the Responses API, so the endpoint must serve it.
fn endpoint_args(m: &Value, stored: bool) -> Vec<String> {
    let Some(e) = m.get("endpoint") else {
        return vec![];
    };
    let mut provider = json!({"name": m["label"], "base_url": e["base_url"], "wire_api": "responses"});
    if stored {
        provider["env_key"] = json!(setup::KEY_VAR);
    } else if let Some(k) = e["key_env"].as_str() {
        provider["env_key"] = json!(k);
    }
    let fields: Vec<String> = provider.as_object().unwrap().iter().map(|(k, v)| format!("{k} = {v}")).collect();
    vec![
        "-c".into(),
        "model_provider=\"solace_endpoint\"".into(),
        "-c".into(),
        format!("model_providers.solace_endpoint={{ {} }}", fields.join(", ")),
    ]
}

/// The context window an endpoint reports for its model: `GET /v1/models`,
/// `meta.n_ctx` (llama.cpp, Strata). Claude Code doesn't know a local model's
/// window, so without it, it compacts too late and the server refuses the prompt.
fn endpoint_context(base_url: &str, model: &str) -> Option<u64> {
    // plain http only; an https endpoint goes undetected.
    let rest = base_url.strip_prefix("http://")?;
    let (host, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    let path = [prefix.trim_matches('/'), "v1/models"].iter().filter(|p| !p.is_empty()).fold(String::new(), |a, p| a + "/" + p);
    let addr = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&addr).ok()?.next()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n")?.1).ok()?;
    let models = body["data"].as_array()?;
    // A server with one model often serves it under any name.
    let m = models.iter().find(|x| x["id"] == model).or(if models.len() == 1 { models.first() } else { None })?;
    m["meta"]["n_ctx"].as_u64()
}

fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join(program).is_file()))
}

// Whether a model can start: its agent kind's checks and usage-limit marks.
// An endpoint that is down shows up as a failed run.
mod agents;
mod setup;
pub use agents::unavailable;
fn model(config: &Value, name: &str) -> Result<Value, Failure> {
    config["models"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["id"] == name))
        .cloned()
        .ok_or_else(|| Failure(format!("E7001 unknown model {name}; configure system [[models]]"), 1))
}
/// The model a request names in `model`, with the `effort` it asks for, checked
/// as an action's are: an unknown model or an effort it doesn't offer is
/// `E7001`, and an unavailable model `E7011`.
pub(super) fn chosen_model(config: &Value, body: &Value) -> Result<Value, Failure> {
    let Some(id) = body["model"].as_str() else { return err("E7001", "a model is required") };
    let mut m = model(config, id)?;
    if let Some(e) = body.get("effort").filter(|e| !e.is_null()) {
        let e = e
            .as_str()
            .filter(|e| efforts(&m).iter().any(|x| x == e))
            .ok_or_else(|| Failure(format!("E7001 effort not offered by {id}; offered: {}", efforts(&m).join(", ")), 1))?;
        m["effort"] = json!(e);
    }
    if let Some(reason) = unavailable(&m) {
        return err("E7011", format!("model {id} is unavailable: {reason}"));
    }
    Ok(m)
}
/// Efforts an action may choose for a model. Custom runners take none. A
/// discovered model carries the efforts its agent says it supports; the fixed
/// lists stand behind the presets and the aliases.
fn efforts(m: &Value) -> Vec<String> {
    if let Some(e) = m["efforts"].as_array() {
        return e.iter().filter_map(|e| e.as_str().map(String::from)).collect();
    }
    let fixed: &[&str] = match (m["run"].is_null(), m["agent"].as_str()) {
        (true, Some("codex")) => &["low", "medium", "high", "xhigh"],
        (true, Some("claude")) => &["low", "medium", "high", "xhigh", "max"],
        _ => &[],
    };
    fixed.iter().map(|e| e.to_string()).collect()
}

// Snapshot live rows only. Before-state rows never enter the semantic index.
pub fn snapshot(s: &Store) -> Result<Value, Failure> {
    let mut out = json!({});
    let mut st = s.db.prepare("SELECT id,module,name,kind,text,ord,revision,live,parent,sep,tail FROM symbols WHERE live=1")?;
    for row in st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, String>(10)?
        ]))
    })? {
        let row = row?;
        let key = format!("symbol:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    // A Rust module's `owns_dir` is part of its before-state: it says where the module's own
    // children go, so undo restores the recorded value instead of guessing (D-168).
    let owns = if s.owns_dir { "owns_dir" } else { "NULL" };
    let mut st = s.db.prepare(&format!("SELECT path,dir,header,file,{owns} FROM modules"))?;
    for row in st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?
        ]))
    })? {
        let row = row?;
        let key = format!("module:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    Ok(out)
}
/// The stores of a task's projects (D-61), with their folders; `s` is its home's.
pub fn scope_stores(s: &Store, t: &Value) -> Result<Vec<(String, Store)>, Failure> {
    let top = scope::top(&s.root);
    let members = workspace::members(&top)?;
    scope::of(t, &scope::dir(&top, &s.root))
        .into_iter()
        .filter(|d| members.contains(d))
        .map(|d| Ok((d.clone(), open(&scope::path(&top, &d))?)))
        .collect()
}
/// A new task in `column`, with nothing done on it yet. Its changes are computed
/// from the scope projects' stores (D-77 point 4), so the task holds none. Its
/// `createdTime` is set here, once (D-130).
pub fn new_task(title: &str, body: &str, column: &Value, rank: f64) -> Value {
    json!({"id":unique_id(),"title":title,"body":body,"links":[],"column":column,"rank":rank,"paused_from":null,"attention":null,
        "stamps":{},"model":null,"notes":[],"questions":[],"runs":[],
        "feature":null,"milestone":null,"dependsOn":[],"createdTime":now(),"closedTime":null,"archivedTime":null})
}
/// Records what a command changed in `s` on the task stored in `home`. The rows
/// commit inside the command's outer transaction (D-77 point 7); the task's
/// change list comes from the committed rows after the release (D-77 point 4).
/// `folders` are the project's folders before the command (`existing_folders`);
/// with none, as for an outside edit, undo removes every folder left empty.
pub fn record(s: &Store, tid: &str, rid: &str, before: &Value, after: &Value, folders: Option<&HashSet<String>>) -> Result<(), Failure> {
    touch(s, tid, rid, before, after, folders)?;
    Ok(())
}
/// The project's folders, relative to `root`, as a command finds them before
/// it runs, skipping what `snapshot` skips: undo removes only the folders a
/// task made (D-136). A folder the project ignores (`target/`) is listed but
/// not walked: undo stops at it, and its contents can be large.
pub fn existing_folders(root: &Path) -> HashSet<String> {
    fn walk(root: &Path, ignores: &mut lockdown::Ignores, dir: &Path, out: &mut HashSet<String>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = file_name(p.strip_prefix(root).unwrap());
            if !e.file_type().is_ok_and(|t| t.is_dir())
                || [".solace", ".git", "build"].contains(&name.as_str())
                || e.file_name() == "node_modules"
            {
                continue;
            }
            if !ignores.ignored(&name) {
                walk(root, ignores, &p, out);
            }
            out.insert(name);
        }
    }
    let mut out = HashSet::new();
    walk(root, &mut lockdown::Ignores::new(root), root, &mut out);
    out
}
/// The before-state of file `name` when it didn't exist (D-136): the nearest
/// folder above it that did (`""`, the project's own), where undo stops
/// removing the folders it leaves empty.
pub fn absent(name: &str, folders: &HashSet<String>) -> Value {
    let mut dir = name;
    while let Some((up, _)) = dir.rsplit_once('/') {
        if folders.contains(up) {
            return json!({ "kept": up });
        }
        dir = up;
    }
    json!({ "kept": "" })
}
/// `absent`, read from the disk just before file `name` is written: the
/// nearest folder above it that exists now. An undo record taken before the
/// write carries it, so recovery after a crash keeps D-136.
pub fn kept_on_disk(root: &Path, name: &str) -> Value {
    let mut dir = name;
    while let Some((up, _)) = dir.rsplit_once('/') {
        if root.join(up).is_dir() {
            return json!({ "kept": up });
        }
        dir = up;
    }
    json!({ "kept": "" })
}
/// The task's changes, computed from each scope project's store (D-77 point 4):
/// `board_touches` rows by task id, in insertion order, one entry per entity and
/// project (the first row that touched it). A WAL read through `open`, no project lock.
pub fn changes_of(s: &Store, t: &Value) -> Result<Vec<Value>, Failure> {
    let tid = t["id"].as_str().unwrap_or("");
    let (mut out, mut seen) = (vec![], HashSet::new());
    for (dir, st) in scope_stores(s, t)? {
        let mut q = st.db.prepare("SELECT entity,run,name FROM board_touches WHERE task=?1 ORDER BY seq")?;
        for row in
            q.query_map(params![tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?
        {
            let (entity, run, name) = row?;
            let (run, name) = (run.unwrap_or_default(), name.unwrap_or_else(|| entity.clone()));
            if !seen.insert((dir.clone(), entity.clone())) {
                continue;
            }
            out.push(json!({"entity": entity, "name": name, "run": run, "project": dir}));
        }
    }
    Ok(out)
}
/// Adds the undo rows for a pending record whose files changed (D-77 point 7):
/// one per path whose bytes differ from the file now, skipping paths whose rows
/// are already committed. A declared run's record lists the watched files as
/// they were before the process; a watched file it doesn't list was created by it.
pub fn adopt_pending(s: &Store, record: &Value) -> Result<(), Failure> {
    let (Some(tid), Some(rid)) = (record["task"].as_str(), record["run"].as_str()) else { return Ok(()) };
    let bad = || Failure("invalid pending record".into(), 1);
    let mut files: Vec<(String, Value)> = vec![];
    if let Some(v) = record.get("files") {
        for f in v.as_array().unwrap() {
            files.push((f["path"].as_str().ok_or_else(bad)?.to_string(), f["bytes"].clone()));
        }
    } else if let Some(path) = record.get("path") {
        files.push((path.as_str().ok_or_else(bad)?.to_string(), record["bytes"].clone()));
    } else {
        // A command's marker: the store already discarded what didn't commit.
        return Ok(());
    }
    let listed: HashSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    // A record taken before the write says where undo stops removing folders
    // (D-136): a file's own `kept`, or a run's folders; an old record has neither.
    let folders: Option<HashSet<String>> =
        record["folders"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect());
    if record.get("files").is_some() {
        let v = lockdown::settings(&s.root)?;
        for name in lockdown::watched_paths(&s.root, &v)? {
            if !listed.contains(&name) {
                files.push((name.clone(), Value::Null));
            }
        }
    }
    for (name, bytes) in files {
        let key = format!("file:{name}");
        let now = fs::read(lockdown::path(&s.root, &name)?).ok();
        let before = bytes.as_array().map(|a| json_bytes(a.as_slice()));
        if now == before {
            continue;
        }
        let committed: bool = s.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_touches WHERE task=?1 AND run=?2 AND entity=?3)",
            params![tid, rid, key],
            |r| r.get(0),
        )?;
        if committed {
            continue;
        }
        let exists: bool =
            s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1 AND entity=?2)", params![tid, key], |r| r.get(0))?;
        if !exists {
            fs::create_dir_all(before_dir(&s.root, tid))?;
            let file = before_file(&s.root, tid, &name);
            match &before {
                Some(b) => fs::write(file, b)?,
                None => fs::write(file.with_extension("absent"), b"")?,
            }
        }
        let stored = match (&bytes, &folders) {
            (Value::Null, _) if record["kept"].is_string() => json!({ "kept": record["kept"] }),
            (Value::Null, Some(f)) => absent(&name, f),
            _ => bytes.clone(),
        };
        s.db.execute("INSERT OR IGNORE INTO board_before VALUES (?1,?2,?3)", params![tid, key, stored.to_string()])?;
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![tid, key, rid, key])?;
    }
    Ok(())
}
/// Records in `s` how to undo what a command changed there for task `tid`,
/// and returns the task's change entries for it.
pub fn touch(
    s: &Store,
    tid: &str,
    rid: &str,
    before: &Value,
    after: &Value,
    folders: Option<&HashSet<String>>,
) -> Result<Vec<Value>, Failure> {
    let mut out = vec![];
    let here = scope::dir(&scope::top(&s.root), &s.root);
    let keys: HashSet<_> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    for key in keys {
        if before[key] == after[key] {
            continue;
        }
        let data = &before[key];
        if let Some(name) = key.strip_prefix("file:") {
            let exists: bool =
                s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1 AND entity=?2)", params![tid, key], |r| r.get(0))?;
            if !exists {
                fs::create_dir_all(before_dir(&s.root, tid))?;
                let file = before_file(&s.root, tid, name);
                if let Some(bytes) = data.as_array() {
                    fs::write(file, json_bytes(bytes))?;
                } else {
                    fs::write(file.with_extension("absent"), b"")?;
                }
            }
        }
        let name = if key.starts_with("symbol:") {
            let row = if after[key].is_null() { data } else { &after[key] };
            format!("{}.{}", row[1].as_str().unwrap_or(""), row[2].as_str().unwrap_or(""))
        } else {
            key.clone()
        };
        let stored = match (key.strip_prefix("file:"), folders) {
            (Some(name), Some(f)) if data.is_null() => absent(name, f),
            // A module the command added: its file's folders count too (D-136).
            (None, Some(f)) if data.is_null() && key.starts_with("module:") && after[key].is_array() => {
                let row = &after[key];
                let file = row[3]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| outside::file_of(row[1].as_str().unwrap_or(""), row[0].as_str().unwrap_or("")));
                absent(&file, f)
            }
            _ => data.clone(),
        };
        s.db.execute("INSERT OR IGNORE INTO board_before VALUES (?1,?2,?3)", params![tid, key, stored.to_string()])?;
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![tid, key, rid, name])?;
        out.push(json!({"entity":key,"name":name,"run":rid,"project":here}));
    }
    Ok(out)
}
/// Whether a recorded change is still worth reviewing: a file that is unwatched now (ignored after
/// the change was recorded, say) is not. Cancel and undo still restore it.
pub fn reviewable(root: &Path, key: &str) -> bool {
    let Some(name) = key.strip_prefix("file:") else { return true };
    lockdown::settings(root).map_or(true, |v| !lockdown::tier(root, &v, name, None).starts_with("unwatched"))
}
/// Review content stays internal, separate from notes (which humans can delete).
fn review_path(root: &Path, tid: &str, rid: &str) -> Result<PathBuf, Failure> {
    Ok(task_dir(root, tid)?.join("reviews").join(format!("{rid}.json")))
}
fn clear_reviews(s: &Store, tid: &str) -> Result<(), Failure> {
    let dir = task_dir(&s.root, tid)?.join("reviews");
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}
fn review_state(s: &Store, t: &Value) -> Result<Value, Failure> {
    let changes = changes_of(s, t)?;
    let mut projects = json!({});
    let mut context = json!({});
    for (dir, st) in scope_stores(s, t)? {
        let current = snapshot(&st)?;
        let mut entities = json!({});
        for c in changes.iter().filter(|c| c["project"] == dir) {
            let key = c["entity"].as_str().unwrap();
            entities[key] = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
        }
        projects[&dir] = entities;
        let mut files = json!({});
        for (name, tier) in lockdown::tree_files(&st.root, &lockdown::settings(&st.root)?)? {
            if tier == "watched" || tier == "protected" {
                files[&name] = json!(sha256(&fs::read(st.root.join(&name))?));
            }
        }
        // Include materialized source too: an outside edit may not yet be absorbed into the store.
        for (key, row) in current.as_object().unwrap() {
            if let Some(module) = key.strip_prefix("module:") {
                let name = format!("{}/{module}.gleam", row[1].as_str().unwrap());
                files[&name] = json!(fs::read(st.root.join(&name)).ok().map(|b| sha256(&b)));
            }
        }
        // Gleam's dependency lock is deliberately unwatched, but affects review context.
        files["manifest.toml"] = json!(fs::read(st.root.join("manifest.toml")).ok().map(|b| sha256(&b)));
        context[dir] = json!([current, files]);
    }
    let instructions = json!([
        t["title"],
        t["body"],
        t["links"],
        t["questions"],
        t["notes"].as_array().unwrap().iter().filter(|n| n["author"] == "human").collect::<Vec<_>>()
    ]);
    Ok(json!({"projects":projects,"context":sha256(context.to_string().as_bytes()),
        "instructions":sha256(instructions.to_string().as_bytes())}))
}
fn previous_review(t: &Value) -> Option<&Value> {
    t["runs"].as_array()?.iter().rev().find(|r| r["kind"] == "review" && r["outcome"] == "reviewed" && r["state"] == "ended")
}
fn review_checkpoint(s: &Store, t: &Value) -> Result<Value, String> {
    let r = previous_review(t).ok_or("no completed review")?;
    let checkpoint = (|| -> Result<Value, Failure> {
        let p = review_path(&s.root, t["id"].as_str().unwrap(), r["id"].as_str().unwrap())?;
        serde_json::from_slice(&fs::read(p)?).map_err(|e| Failure(e.to_string(), 1))
    })()
    .map_err(|_| "missing or legacy review checkpoint")?;
    if checkpoint["run"] != r["id"] || !checkpoint["projects"].is_object() {
        return Err("missing or legacy review checkpoint".into());
    }
    let scope = scope_stores(s, t).map_err(|e| e.0)?.into_iter().map(|(d, _)| d).collect::<HashSet<_>>();
    if scope != checkpoint["projects"].as_object().unwrap().keys().cloned().collect() {
        return Err("project scope changed since review".into());
    }
    Ok(checkpoint)
}
/// `task diff`: what each entity this task changed looked like before the task,
/// against now. `names` narrows it; past `budget`, the rest are listed by name.
pub fn task_diff(s: &Store, tid: &str, names: &[&str], budget: usize) -> Out {
    let t = task(&s.root, tid)?;
    let since = names.contains(&"--since-review");
    let names: Vec<_> = names.iter().copied().filter(|n| *n != "--since-review").collect();
    let checkpoint = since.then(|| review_checkpoint(s, &t));
    let baseline = checkpoint.as_ref().and_then(|c| c.as_ref().ok());
    let heading = match &checkpoint {
        Some(Ok(c)) => format!("Since completed review {} (content at review start):\n\n", c["run"].as_str().unwrap()),
        Some(Err(reason)) => format!("Full task diff: {reason}; falling back to the state before the task.\n\n"),
        None => String::new(),
    };
    let projects = scope_stores(s, &t)?;
    let changes = changes_of(s, &t)?;
    let (mut out, mut left) = (String::new(), vec![]);
    let mut unsafe_lines: Vec<String> = vec![];
    for (dir, st) in &projects {
        let current = snapshot(st)?;
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let before: HashMap<String, Value> = rows
            .query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .map(|r| r.map(|(k, v)| (k, serde_json::from_str(&v).unwrap())))
            .collect::<Result<_, _>>()?;
        let text = |key: &str, v: &Value| -> Option<String> {
            if key.starts_with("symbol:") {
                (v[7].as_i64() == Some(1)).then(|| v[4].as_str().unwrap_or("").to_string())
            } else if key.starts_with("module:") {
                v[2].as_str().map(|h| h.to_string())
            } else {
                let bytes = json_bytes(v.as_array()?);
                Some(String::from_utf8(bytes).unwrap_or_else(|e| format!("(binary, {} bytes)", e.as_bytes().len())))
            }
        };
        for c in changes.iter().filter(|c| c["project"] == *dir) {
            let (key, name) = (c["entity"].as_str().unwrap(), c["name"].as_str().unwrap_or(""));
            let Some(original) = before.get(key) else { continue };
            // Entities first touched after the review use their task before-state.
            let old = baseline.and_then(|b| b["projects"][dir].get(key)).unwrap_or(original);
            if !reviewable(&st.root, key) {
                continue;
            }
            if !names.is_empty()
                && !names.iter().any(|n| *n == format!("{dir}: {name}") || *n == name || key.split_once(':').is_some_and(|(_, k)| k == *n))
            {
                continue;
            }
            let new = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
            if baseline.is_some() && old == &new {
                continue;
            }
            // A task across projects names each change's project (D-61).
            let shown = if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() };
            if crate::language::is_rust(&st.root)
                && let Some(line) = new_unsafe(&shown, old, &new)
            {
                unsafe_lines.push(line);
            }
            let part = match (text(key, old), text(key, &new)) {
                (a, b) if a == b && baseline.is_some() => format!("## {shown} (metadata or binary content changed)\n"),
                (a, b) if a == b => format!("## {shown} (back to its state before the task)\n"),
                // an extract into another project: shown once, with the diff, where it landed
                (Some(_), None) if let Some(there) = moved_to(&projects, dir, key, tid) => format!("## {shown} (moved to {there})\n"),
                (None, Some(b)) => format!("## {shown} (added)\n{}", line_diff("", &b)),
                (Some(a), None) => format!("## {shown} (deleted)\n{}", line_diff(&a, "")),
                (a, b) => format!("## {shown} (changed)\n{}", line_diff(&a.unwrap_or_default(), &b.unwrap_or_default())),
            };
            if !out.is_empty() && out.len() + part.len() > budget {
                left.push(shown);
            } else {
                out += &part;
                out.push('\n');
            }
        }
    }
    if !unsafe_lines.is_empty() {
        // D-158 point 2: a review list, not a refusal.
        out += "New `unsafe`:\n";
        for line in &unsafe_lines {
            out += line;
            out.push('\n');
        }
        out.push('\n');
    }
    if out.is_empty() {
        return Ok(heading
            + if baseline.is_some() && names.is_empty() {
                "No task entity changes since this review.\n"
            } else if names.is_empty() {
                "This task has no changes.\n"
            } else {
                "No change of this task matches.\n"
            });
    }
    if !left.is_empty() {
        out += &format!(
            "Left out to fit the reply; read with `task diff {}<name>`: {}\n",
            if since { "--since-review " } else { "" },
            left.join(", ")
        );
    }
    Ok(heading + &out)
}
/// A changed symbol that added `unsafe`, for the review (D-158 point 2):
/// `name`, marked when the new text has no `// SAFETY:` comment. This lists it
/// for the human; nothing is refused.
fn new_unsafe(name: &str, old: &Value, new: &Value) -> Option<String> {
    let (had, _) = rust_unsafe(old[4].as_str().unwrap_or(""));
    let (now, safety) = rust_unsafe(new[4].as_str().unwrap_or(""));
    (now > had).then(|| if safety { format!("- {name}") } else { format!("- {name} (no `// SAFETY:` comment)") })
}
/// The `unsafe` keywords in Rust text, outside comments, string and char
/// literals, and whether a comment holds `SAFETY:` (D-158 point 2). A name
/// that contains `unsafe` doesn't count.
fn rust_unsafe(text: &str) -> (usize, bool) {
    fn skip_string(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && b[i] != b'"' {
            i += if b[i] == b'\\' { 2 } else { 1 };
        }
        i + 1
    }
    /// A raw string's body start and its `#` count, when one begins at `i`.
    fn raw_start(b: &[u8], i: usize) -> Option<(usize, usize)> {
        let mut j = i;
        if b.get(j) == Some(&b'b') {
            j += 1;
        }
        if b.get(j) != Some(&b'r') {
            return None;
        }
        j += 1;
        let hashes = (j..b.len()).take_while(|&k| b[k] == b'#').count();
        j += hashes;
        (b.get(j) == Some(&b'"')).then_some((j + 1, hashes))
    }
    fn skip_raw(b: &[u8], from: usize, hashes: usize) -> usize {
        let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
        let mut i = from;
        while i + close.len() <= b.len() {
            if &b[i..i + close.len()] == close.as_slice() {
                return i + close.len();
            }
            i += 1;
        }
        b.len()
    }
    let b = text.as_bytes();
    let (mut i, mut count, mut safety) = (0, 0, false);
    while i < b.len() {
        if let Some((from, hashes)) = raw_start(b, i) {
            i = skip_raw(b, from, hashes);
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = text[i..].find('\n').map_or(b.len(), |k| i + k);
            safety |= text[i..end].contains("SAFETY:");
            i = end;
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            safety |= text[i..end].contains("SAFETY:");
            i = end;
        } else if b[i] == b'"' {
            i = skip_string(b, i + 1);
        } else if b[i] == b'\'' {
            if b.get(i + 1) == Some(&b'\\') {
                i += 2;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else if b.get(i + 2) == Some(&b'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if &text[start..i] == "unsafe" {
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    (count, safety)
}
/// The other project of `projects` where the symbol entity `key` is live: an
/// extract into another project moved it there (D-103). One that gained it in
/// task `tid` comes before one that held the same text all along.
fn moved_to(projects: &[(String, Store)], dir: &str, key: &str, tid: &str) -> Option<String> {
    if !key.starts_with("symbol:") {
        return None;
    }
    let live: Vec<&(String, Store)> =
        projects.iter().filter(|(d, s)| d != dir && snapshot(s).is_ok_and(|c| c[key][7].as_i64() == Some(1))).collect();
    let gained = |s: &Store| {
        s.db.query_row("SELECT data FROM board_before WHERE task=?1 AND entity=?2", [tid, key], |r| r.get::<_, String>(0))
            .is_ok_and(|d| d == "null")
    };
    live.iter().find(|(_, s)| gained(s)).or(live.first()).map(|(d, _)| d.clone())
}
/// Line diff with two lines of context; `…` marks skipped lines.
fn line_diff(a: &str, b: &str) -> String {
    let ops = diff_ops(a, b);
    let (mut out, mut gap) = (String::new(), false);
    for k in 0..ops.len() {
        if ops[k.saturating_sub(2)..(k + 3).min(ops.len())].iter().any(|o| o.0 != ' ') {
            if gap {
                out += "  …\n";
            }
            gap = false;
            out += &format!("{} {}\n", ops[k].0, ops[k].1);
        } else {
            gap = true;
        }
    }
    if gap {
        out += "  …\n";
    }
    out
}
/// The lines of `b` (from 0) that a line diff from `a` marks: added lines, and
/// the line where lines were removed.
pub(super) fn changed_lines(a: &str, b: &str) -> Vec<usize> {
    let (mut out, mut line) = (vec![], 0);
    for (op, _) in diff_ops(a, b) {
        match op {
            '+' => {
                out.push(line);
                line += 1;
            }
            '-' => out.push(line.min(b.lines().count().saturating_sub(1))),
            _ => line += 1,
        }
    }
    out.dedup();
    out
}
pub fn diff_ops<'a>(a: &'a str, b: &'a str) -> Vec<(char, &'a str)> {
    let (a, b): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (x, y) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // O(n·m) LCS table over the changed middle; use Myers if whole-file rewrites get large.
    let mut l = vec![vec![0u32; y.len() + 1]; x.len() + 1];
    for i in (0..x.len()).rev() {
        for j in (0..y.len()).rev() {
            l[i][j] = if x[i] == y[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let mut ops: Vec<(char, &str)> = a[..pre].iter().map(|s| (' ', *s)).collect();
    let (mut i, mut j) = (0, 0);
    while i < x.len() || j < y.len() {
        if i < x.len() && j < y.len() && x[i] == y[j] {
            ops.push((' ', x[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < x.len() && (j == y.len() || l[i + 1][j] >= l[i][j + 1]) {
            ops.push(('-', x[i]));
            i += 1;
        } else {
            ops.push(('+', y[j]));
            j += 1;
        }
    }
    ops.extend(a[a.len() - suf..].iter().map(|s| (' ', *s)));
    ops
}
/// Snapshot entries for every regular file at or under `name` in `root`; none
/// for a path an agent can't reach, which the command refuses anyway.
pub fn files_under(root: &Path, name: &str) -> serde_json::Map<String, Value> {
    fn walk(p: &Path, name: &str, out: &mut serde_json::Map<String, Value>) {
        let Ok(m) = fs::symlink_metadata(p) else { return };
        if m.is_file() {
            out.insert(format!("file:{name}"), json!(fs::read(p).ok()));
        } else if m.is_dir() {
            for e in fs::read_dir(p).into_iter().flatten().flatten() {
                walk(&e.path(), &format!("{name}/{}", e.file_name().to_string_lossy()), out);
            }
        }
    }
    let mut out = serde_json::Map::new();
    let name = file_name(Path::new(name));
    if !name.is_empty()
        && !name.split('/').any(|c| c == ".solace")
        && let Ok(p) = lockdown::path(root, &name)
    {
        walk(&p, &name, &mut out);
    }
    out
}
pub fn file_snapshot(root: &Path, a: &[&str]) -> Result<Value, Failure> {
    let mut v = json!({});
    for name in lockdown::paths(a) {
        let p = lockdown::path(root, name)?;
        let name = file_name(p.strip_prefix(root).unwrap());
        v[format!("file:{name}")] = json!(fs::read(p).ok());
    }
    // A move's files on both sides; a side in another project is recorded there (D-84).
    if a[0] == "move"
        && let Ok((from, to, dest)) = lockdown::move_args(root, a)
    {
        let m = v.as_object_mut().unwrap();
        m.extend(files_under(root, from));
        if lockdown::same_project(root, &dest) {
            m.extend(files_under(root, to));
        }
    }
    if a[0] == "run" {
        fn collect(root: &Path, dir: &Path, settings: &toml::Value, out: &mut Value) -> Result<(), Failure> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let p = entry.path();
                let name = p.strip_prefix(root).unwrap().to_string_lossy();
                if entry.file_type()?.is_symlink()
                    || [".solace", ".git", "build"].contains(&name.as_ref())
                    || entry.file_name() == "node_modules"
                {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    collect(root, &p, settings, out)?;
                } else if lockdown::tier(root, settings, &name, None) == "watched" {
                    out[format!("file:{name}")] = json!(fs::read(&p)?);
                }
            }
            Ok(())
        }
        collect(root, root, &lockdown::settings(root)?, &mut v)?;
    }
    Ok(v)
}
pub(crate) fn restore(s: &Store, states: &[(String, Value)]) -> Result<(), Failure> {
    let mut kept = HashMap::new();
    for (key, v) in states {
        if let Some(id) = key.strip_prefix("symbol:") {
            if v.is_null() {
                s.db.execute("DELETE FROM symbols WHERE id=?", [id])?;
            } else {
                // A snapshot from before the nested columns has only the first eight (D-158).
                s.db.execute(
                    "INSERT OR REPLACE INTO symbols(id,module,name,kind,text,ord,revision,live,parent,sep,tail) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        v[0].as_str(),
                        v[1].as_str(),
                        v[2].as_str(),
                        v[3].as_str(),
                        v[4].as_str(),
                        v[5].as_i64(),
                        v[6].as_str(),
                        v[7].as_i64(),
                        v.get(8).and_then(|x| x.as_str()),
                        v.get(9).and_then(|x| x.as_str()),
                        v.get(10).and_then(|x| x.as_str()).unwrap_or("")
                    ],
                )?;
            }
        } else if let Some(name) = key.strip_prefix("module:") {
            if !v.is_array() {
                // Where its file's folders stop going, when recorded (D-136).
                if let Some(k) = v["kept"].as_str() {
                    // A Rust module's row holds its file; a Gleam one its dir (D-168).
                    let row: Option<(String, Option<String>)> =
                        s.db.query_row("SELECT dir, file FROM modules WHERE path=?", [name], |r| Ok((r.get(0)?, r.get(1)?))).ok();
                    if let Some((dir, file)) = row {
                        kept.insert(file.unwrap_or_else(|| outside::file_of(&dir, name)), k.to_string());
                    }
                }
                // Its file goes in the `materialize` below, which keeps one edited outside Solace (D-58).
                s.db.execute("DELETE FROM modules WHERE path=?", [name])?;
            } else {
                // ON CONFLICT keeps a Rust module's `file` and `owns_dir`; a snapshot from
                // before them reads as None.
                let (path, dir, header) = (v[0].as_str(), v[1].as_str(), v[2].as_str());
                let (file, owns) = (v.get(3).and_then(|x| x.as_str()), v.get(4).and_then(|x| x.as_i64()));
                s.db.execute(
                    "INSERT INTO modules(path,dir,header,file,owns_dir) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(path) DO UPDATE SET dir=excluded.dir, header=excluded.header, file=excluded.file, owns_dir=excluded.owns_dir",
                    params![path, dir, header, file, owns],
                )?;
            }
        } else if let Some(name) = key.strip_prefix("file:") {
            let p = lockdown::path(&s.root, name)?;
            if let Some(bytes) = v.as_array() {
                fs::create_dir_all(p.parent().unwrap())?;
                fs::write(p, json_bytes(bytes))?;
            } else if p.exists() {
                fs::remove_file(&p)?;
                // with the folders that leaves empty, such as those a carried file made (D-103),
                // up to the top folder or the one recorded as there before (D-136)
                let top = name.split('/').next().unwrap_or_default();
                let stop = v["kept"].as_str().filter(|k| k.len() > top.len()).unwrap_or(top);
                outside::remove_empty_dirs(&s.root.join(stop), p.parent());
            }
        }
    }
    let ix = s.index()?;
    s.materialize_to(&ix, &s.root, &kept)?;
    Ok(())
}
/// Whether the check after an undo runs in the project: a project with a
/// language, or a Rust project whose toolchain is there (D-158). Without the
/// toolchain the undo is unchecked, not refused.
fn checkable(root: &Path, cargo: bool) -> bool {
    crate::language::detect(root).is_some() || (crate::language::is_rust(root) && cargo)
}
/// Undoes a task's changes in every project of its scope (D-61): each is
/// restored and checked, and a failure puts back the projects already restored.
fn revert(s: &Store, t: &mut Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap().to_string();
    let projects = scope_stores(s, t)?;
    // Malformed settings are reported before recovery, which may undo another run's work.
    let cargo =
        projects.iter().any(|(_, st)| crate::language::is_rust(&st.root)) && crate::language::reason(crate::language::RUST)?.is_none();
    // An extract a stopped run left between its projects' commits is undone first (D-135).
    extract::ahead::recover(&projects.iter().map(|(_, st)| st.root.clone()).collect::<Vec<_>>())?;
    // Under the locks, what a stopped run left half-written becomes the task's own rows first.
    for (_, st) in &projects {
        lockdown::recover(&st.root, st, &[])?;
    }
    let changes = changes_of(s, t)?;
    let named = |dir: &str, key: &str| -> String {
        let name = changes.iter().find(|c| c["entity"] == key && c["project"] == dir);
        let name = name.and_then(|c| c["name"].as_str()).unwrap_or(key);
        if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() }
    };
    let (mut plans, mut conflicts) = (vec![], vec![]);
    for (dir, st) in &projects {
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let mut states = vec![];
        for row in rows.query_map([&tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (k, v) = row?;
            states.push((k, serde_json::from_str::<Value>(&v).unwrap()));
        }
        for (key, _) in &states {
            let later:Option<String>=st.db.query_row("SELECT task FROM board_touches WHERE entity=?1 AND seq > (SELECT MIN(seq) FROM board_touches WHERE task=?2 AND entity=?1) AND task!=?2 ORDER BY seq DESC LIMIT 1",params![key,tid],|r|r.get(0)).optional()?;
            if let Some(other) = later {
                conflicts.push(format!("{}: task {other}", named(dir, key)));
            }
        }
        plans.push((dir, st, states));
    }
    if !conflicts.is_empty() {
        return err("E7007", conflicts.join(", "));
    }
    // Every project is restored before any is checked: one reads another from
    // disk through a path dependency, as an extract's dependents do (D-103).
    let mut done: Vec<(&Store, Vec<(String, Value)>)> = vec![];
    let mut failed: Option<(&str, &Store, Failure)> = None;
    let plans: Vec<_> = plans.into_iter().filter(|p| !p.2.is_empty()).collect();
    for (dir, st, states) in &plans {
        let current = snapshot(st)?;
        let here = existing_folders(&st.root);
        let mut undo = vec![];
        for (key, _) in states {
            let v = if let Some(name) = key.strip_prefix("file:") {
                // a rollback keeps the folders there now (D-136)
                fs::read(lockdown::path(&st.root, name)?).map_or_else(|_| absent(name, &here), |b| json!(b))
            } else {
                current[key].clone()
            };
            undo.push((key.clone(), v));
        }
        st.db.execute_batch("SAVEPOINT revert_task")?;
        done.push((*st, undo));
        if let Err(e) = restore(st, states) {
            failed = Some((dir.as_str(), *st, e));
            break;
        }
    }
    if failed.is_none() {
        for (dir, st, _) in &plans {
            // A project of plain files (D-60) has no code to type-check; the restored files say
            // which it is now.
            if checkable(&st.root, cargo)
                && let Err(e) = st.check()
            {
                failed = Some((dir.as_str(), *st, e));
                break;
            }
        }
    }
    match failed {
        Some((dir, st, e)) => {
            for (earlier, undo) in done.iter().rev() {
                earlier.db.execute_batch("ROLLBACK TO revert_task; RELEASE revert_task")?;
                restore(earlier, undo)?;
            }
            let other = tasks(&st.root)?
                .iter()
                .filter(|x| x["id"] != tid.as_str())
                .map(|x| {
                    let names: Vec<String> =
                        changes_of(st, x).unwrap_or_default().iter().map(|c| c["name"].as_str().unwrap_or("").to_string()).collect();
                    format!("task {}: {}", x["id"], names.join(", "))
                })
                .collect::<Vec<_>>()
                .join("\n");
            let place = if projects.len() > 1 { format!("in {dir}: ") } else { String::new() };
            return err("E7008", format!("{place}{}\n{other}", e.0));
        }
        None => {
            for (st, _) in &done {
                st.db.execute_batch("RELEASE revert_task")?;
            }
        }
    }
    for (_, st) in &projects {
        st.db.execute("DELETE FROM board_touches WHERE task=?", [&tid])?;
        drafts::clear(st, &tid)?;
        drop_before(st, &tid)?;
        outside::checkpoint(st)?;
    }
    clear_reviews(s, &tid)?;
    end_runs(t, "reverted");
    Ok(())
}
/// A completed task's changes become approved in every project it changed.
fn settle(s: &Store, t: &Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    clear_reviews(s, tid)?;
    let (mut pending, mut failed) = (vec![], vec![]);
    for (dir, st) in scope_stores(s, t)? {
        lockdown::recover(&st.root, &st, &[])?;
        outside::approve(&st, tid)?;
        drop_before(&st, tid)?;
        match outside::checkpoint_pending(&st) {
            Ok(false) => {}
            Ok(true) => pending.push(dir),
            Err(e) => failed.push((dir, e)),
        }
    }
    settle_result(pending, failed)
}
/// The body of a feature's Tech Debt task (D-112).
const TECH_DEBT_BODY: &str =
    "Minor, non-blocking findings from reviews of this feature's tasks, one note each, filed when the reviewed task was completed.";
/// The findings of a note's `### Tech debt` section (D-112): one per top-level
/// bullet (`-`, `*`, `+` or `1.`), with the indented lines below it. The
/// section ends at the next heading; no section gives none.
pub(super) fn tech_debt_items(text: &str) -> Vec<String> {
    let (mut items, mut inside): (Vec<String>, bool) = (vec![], false);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            inside = trimmed.trim_start_matches('#').trim().trim_end_matches(':').eq_ignore_ascii_case("tech debt");
            continue;
        }
        if !inside || trimmed.is_empty() {
            continue;
        }
        let numbered = line.split_once(". ").filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(_, rest)| rest);
        match ["- ", "* ", "+ "].iter().find_map(|b| line.strip_prefix(b)).or(numbered) {
            Some(item) => items.push(item.trim().to_string()),
            None => {
                if let Some(last) = items.last_mut() {
                    *last += "\n";
                    *last += line.trim_end();
                }
            }
        }
    }
    items.retain(|i| !i.is_empty());
    items
}
/// The task's last review: the run id and the agent's note of its last review
/// run that ended `reviewed`.
fn last_review(t: &Value) -> Option<(String, String)> {
    let r = t["runs"].as_array()?.iter().rev().find(|r| r["kind"] == "review" && r["outcome"] == "reviewed")?;
    let rid = r["id"].as_str()?;
    let n = t["notes"].as_array()?.iter().rev().find(|n| n["run"] == rid && n["author"] == "agent")?;
    Some((rid.to_string(), n["text"].as_str()?.to_string()))
}
/// What completing task `t` files as tech debt (D-112): its feature's open Tech
/// Debt task (`techDebt: true`, not in a final column) with one `solace` note per
/// finding of the last review's `### Tech debt`, naming the task they came from.
/// With no open one, a new one is made in Planning of `t`'s home (its first
/// column if it has none, D-132), with its scope, titled `<feature title>: Tech
/// Debt`. None when the task has no feature or the review lists no tech debt.
/// Nothing is written here.
pub(super) fn tech_debt(top: &Path, t: &Value) -> Result<Option<(Value, usize)>, Failure> {
    let Some(fid) = t["feature"].as_str() else { return Ok(None) };
    let Some((rid, text)) = last_review(t) else { return Ok(None) };
    let items = tech_debt_items(&text);
    let Some(feature) = tasks::record(top, "features", fid).filter(|_| !items.is_empty()) else { return Ok(None) };
    let tid = t["id"].as_str().unwrap_or("");
    let open = |o: &Value| flow(&scope::path(top, o["home"].as_str().unwrap_or("."))).is_ok_and(|w| column(&w, o)["final"] != true);
    let found = all_tasks(top)?.into_iter().find(|o| o["techDebt"] == true && o["feature"] == fid && o["id"] != tid && open(o));
    let mut debt = match found {
        Some(d) => d,
        None => {
            let home = scope::path(top, t["home"].as_str().unwrap_or("."));
            let rank = tasks(&home)?.iter().filter_map(|x| x["rank"].as_f64()).fold(0., f64::max) + 1.;
            let title = format!("{}: Tech Debt", feature["title"].as_str().unwrap_or("").trim());
            let w = flow(&home)?;
            let first = if has_column(&w, PLANNING) { json!(PLANNING) } else { w["columns"][0]["id"].clone() };
            let mut d = new_task(&title, TECH_DEBT_BODY, &first, rank);
            d["techDebt"] = json!(true);
            d["feature"] = json!(fid);
            d["milestone"] = feature["milestone"].clone();
            d["scope"] = t["scope"].clone();
            d["home"] = t["home"].clone();
            d
        }
    };
    for item in &items {
        note(&mut debt, "solace", None, &format!("Tech debt from the review of task:{tid} (review run {rid}):\n\n{item}"));
    }
    Ok(Some((debt, items.len())))
}
/// Settles task `tid` once the task server has moved it into a final column
/// (D-77 point 5, Option B): a step of its own, never on the server's thread, that
/// takes the scope's locks like a human action does. It reads the task again
/// under them, so a task that has left the final column since is not settled, and
/// it can run again: approving and dropping records twice changes nothing.
pub(super) fn settle_final(top: &Path, tid: &str) -> Result<(), Failure> {
    // A task with no project (D-129) has no scope to settle, and settling it
    // at its home would create a store there.
    let t = task(top, tid)?;
    if t["noProject"] == true {
        return Ok(());
    }
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let _held = guard(&home, vec!["tasks", tid, "settle"], &json!({}))?;
    let t = task(&home, tid)?;
    if column(&flow(&home)?, &t)["final"] == true {
        settle(&open(&home)?, &t)?;
    }
    Ok(())
}
/// Settles every task already in a final column that still has undo records,
/// which a stop or a crash between the move and its settle leaves behind.
pub(super) fn resettle(top: &Path) {
    for t in all_tasks(top).unwrap_or_default() {
        let (Some(tid), Some(home)) = (t["id"].as_str(), t["home"].as_str()) else { continue };
        let home = scope::path(top, home);
        let Ok(w) = flow(&home) else { continue };
        if column(&w, &t)["final"] != true {
            continue;
        }
        let left = open(&home).and_then(|s| scope_stores(&s, &t)).is_ok_and(|stores| {
            stores.iter().any(|(_, st)| {
                st.db
                    .query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0))
                    .unwrap_or(false)
                    || lockdown::pending_for(&st.root, tid)
            })
        });
        if left && let Err(e) = settle_final(top, tid) {
            eprintln!("settling task {tid}: {}", e.0);
        }
    }
}
/// Where a task keeps files as they were before it changed them.
pub fn before_dir(root: &Path, tid: &str) -> PathBuf {
    root.join(".solace/board").join(tid).join("before")
}
/// File `name` as it was before the task changed it; with extension `absent`
/// when it didn't exist.
pub fn before_file(root: &Path, tid: &str, name: &str) -> PathBuf {
    before_dir(root, tid).join(sha256(name.as_bytes()))
}
/// Changes whenever the store's code does.
pub fn fingerprint(s: &Store) -> Out {
    Ok(sha256(snapshot(s)?.to_string().as_bytes()))
}
fn drop_before(s: &Store, tid: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM board_before WHERE task=?", [tid])?;
    let p = before_dir(&s.root, tid);
    if p.exists() {
        fs::remove_dir_all(p)?;
    }
    Ok(())
}

fn continue_answered(t: &mut Value) {
    if !t["paused_from"].is_null() || t["questions"].as_array().unwrap().iter().any(|q| q["answer"].is_null()) {
        return;
    }
    let Some(i) = t["runs"].as_array().unwrap().iter().position(|r| r["state"] == "waiting") else {
        return;
    };
    // The answers go straight to the waiting agent, in any column and any workflow:
    // no workflow action is looked up, and nothing is moved, stamped or replaced.
    t["runs"][i]["state"] = json!("running");
    t["runs"][i]["reply"] = json!({"questions":t["questions"],"text":null});
    t["attention"] = Value::Null;
    t["finished"] = Value::Null;
}

fn action(s: &Store, w: &Value, t: &mut Value, body: &Value) -> Result<Option<(String, Value)>, Failure> {
    let name = body["action"].as_str().unwrap_or("");
    let allowed = allowed(w, t);
    if held_by_feature(w, t, name) {
        return err("E7001", "a task with a feature goes back to the backlog only through its feature");
    }
    if !allowed.iter().any(|x| x == name) {
        return err("E7001", format!("not allowed; allowed: {}", allowed.join(", ")));
    }
    // Back to planning leaves no agent behind: the human stops it first (D-133).
    if name == "to_planning" && active(t) {
        return err("E7001", "a run is open on this task; stop it before bringing the task back to planning");
    }
    if name == "pause" {
        if t["paused_from"].is_null() {
            t["paused_from"] = t["column"].clone();
        }
        return Ok(None);
    }
    if name == "resume" {
        if !t["paused_from"].is_null() {
            t["column"] = t["paused_from"].clone();
            t["paused_from"] = Value::Null;
        }
        continue_answered(t);
        return Ok(None);
    }
    // Stop ends every open run; the killed agent's conversation isn't kept.
    if name == "stop" {
        if !active(t) {
            return err("E7003", "no agent is working on this task");
        }
        let discussing = discussion(t);
        end_runs(t, "stopped");
        if let Some(rid) = discussing {
            compact(t, &rid);
        }
        return Ok(None);
    }
    if name == "end_discussion" {
        let rid = discussion(t).ok_or_else(|| Failure("E7003 no discussion is going".into(), 1))?;
        let i = run_at(t, &rid)?;
        end_run(&mut t["runs"][i], "discussed");
        compact(t, &rid);
        return Ok(None);
    }
    if name == "discuss" {
        if body["text"].as_str().is_none_or(|s| s.trim().is_empty()) {
            return err("E7001", "a discussion starts with a message");
        }
        if active(t) {
            return err("E7003", "task already has a run");
        }
    }
    let discuss = json!({"run":"discuss","model":true});
    let a = if name == "discuss" { &discuss } else { &w["actions"][name] };
    let run = a["run"].as_str().unwrap_or("");
    if matches!(run, "investigate" | "work" | "new_work")
        && body["text"].as_str().is_none_or(|s| s.is_empty())
        && t["questions"].as_array().unwrap().iter().any(|q| q["answer"].is_null())
    {
        return err("E7002", "answer the questions or supply text");
    }
    // `work` and `investigate` continue a waiting agent of their kind.
    let waiting = t["runs"].as_array().unwrap().iter().position(|r| r["state"] == "waiting" && r["kind"] == run);
    let resume = waiting.is_some();
    if !run.is_empty() && active(t) && !resume && run != "new_work" && a["revert"] != true {
        return err("E7003", "task already has a run");
    }
    if matches!(run, "work" | "new_work") {
        let top = scope::top(&s.root);
        claim_projects(&s.root, w, &t["id"], &scope::of(t, &scope::dir(&top, &s.root)))?;
    }
    let config = system()?;
    let chosen = body.get("model").unwrap_or(&t["model"]);
    if a["model"] == true && !resume && body["model"].as_str().is_none() {
        return err("E7001", "this action requires a model");
    }
    let mut selected = if !run.is_empty() && !resume { Some(model(&config, chosen.as_str().unwrap_or(""))?) } else { None };
    if let Some(m) = selected.as_mut() {
        // A chosen effort must be offered; the task's remembered one applies only when still offered.
        if let Some(e) = body.get("effort").filter(|e| !e.is_null()) {
            let e = e.as_str().filter(|e| efforts(m).iter().any(|x| x == e)).ok_or_else(|| {
                Failure(format!("E7001 effort not offered by {}; offered: {}", m["id"].as_str().unwrap_or(""), efforts(m).join(", ")), 1)
            })?;
            m["effort"] = json!(e);
        } else if body.get("model").is_none() && t["effort"].as_str().is_some_and(|e| efforts(m).iter().any(|x| x == e)) {
            m["effort"] = t["effort"].clone();
        }
    }
    if let Some(reason) = selected.as_ref().and_then(unavailable) {
        return err("E7011", format!("model {} is unavailable: {reason}", selected.unwrap()["id"].as_str().unwrap_or("")));
    }
    // Validate the gate before any destructive step.
    let mut proposed = t.clone();
    if let Some(stamp) = a["stamp"].as_str() {
        proposed["stamps"][stamp] = json!({"who":"human","time":now()});
    }
    if let Some(stamp) = a["unstamp"].as_str() {
        proposed["stamps"].as_object_mut().unwrap().remove(stamp);
    }
    if let Some(to) = a.get("to") {
        move_to(w, &mut proposed, to)?;
    }
    if a["revert"] == true {
        revert(s, t)?;
        proposed["runs"] = t["runs"].clone();
    }
    let left = (column(w, t)["final"] == true && column(w, &proposed)["final"] != true)
        .then(|| (column(w, t)["label"].clone(), column(w, &proposed)["label"].clone()));
    *t = proposed;
    // Leaving a final column says so on the task (D-108).
    if let Some((from, to)) = left {
        let text = format!("The human brought this task back from {} to {}.", from.as_str().unwrap_or(""), to.as_str().unwrap_or(""));
        note(t, "solace", None, &text);
    }
    if let Some(text) = body["text"].as_str()
        && run != "discuss"
    {
        note(t, "human", None, text);
        for q in t["questions"].as_array_mut().unwrap() {
            if q["answer"].is_null() {
                q["answer"] = json!(text);
            }
        }
    }
    // Moving the task, or starting or continuing a run, ends a pause; a paused run would hang.
    if a.get("to").is_some() || resume || selected.is_some() {
        t["paused_from"] = Value::Null;
    }
    if resume {
        let i = waiting.unwrap();
        t["runs"][i]["state"] = json!("running");
        t["runs"][i]["reply"] = json!({"questions":t["questions"],"text":body["text"]});
        return Ok(None);
    }
    if run == "new_work" {
        end_runs(t, "replaced");
    }
    if let Some(m) = selected {
        let rid = unique_id();
        let kind = if run == "new_work" { "work" } else { run };
        let log = s.root.join(".solace/board").join(t["id"].as_str().unwrap()).join(format!("{rid}.jsonl"));
        t["runs"].as_array_mut().unwrap().push(json!({"id":rid,"kind":kind,"model":m["id"],"effort":m["effort"],"state":"running","started":now(),"log":log,"fresh":run == "new_work" || a["revert"] == true}));
        if kind == "review" {
            let mut checkpoint = review_state(s, t)?;
            checkpoint["run"] = json!(rid);
            let path = review_path(&s.root, t["id"].as_str().unwrap(), &rid)?;
            fs::create_dir_all(path.parent().unwrap())?;
            write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap())?;
        }
        // The discussion's first message is its first turn.
        if kind == "discuss" {
            note(t, "human", Some(&rid), body["text"].as_str().unwrap_or(""));
        }
        t["finished"] = Value::Null;
        if kind == "work" {
            t["model"] = m["id"].clone();
            t["effort"] = m["effort"].clone();
        }
        return Ok(Some((rid, m)));
    }
    if column(w, t)["final"] == true {
        end_runs(t, "completed");
        settle(s, t)?;
    }
    Ok(None)
}

/// The models this Solace install knows; the same for every project. One entry
/// per Claude family, with why it can't start; `limit`: the reason is a usage
/// limit a human may clear ("Try again", `POST /models/try-again`).
fn models() -> Result<Value, Failure> {
    // Serve the last list; ask a kind whose answer is stale, in the background.
    agents::refresh_stale();
    let all = system()?["models"].as_array().cloned().unwrap_or_default();
    let keys = setup::read_keys(&setup::keys_path());
    Ok(json!(
        served(&all)
            .iter()
            .map(|m| {
                let status = agents::availability(m);
                let limit = status.as_ref().is_some_and(|s| s.1);
                let reason = status.map(|s| s.0);
                // Whether a key is stored, never the key.
                let has_key = m["id"].as_str().is_some_and(|id| keys[id].is_string());
                json!({"id":m["id"],"label":m["label"],"model":m["model"],"agent":m["agent"],"effort":m["effort"],"efforts":efforts(m),"available":reason.is_none(),"reason":reason,"limit":limit,
                    "configured":m["configured"] == true,"base_url":m["endpoint"]["base_url"],"has_key":has_key})
            })
            .collect::<Vec<_>>()
    ))
}

/// One entry per Claude family: the family's live exact model (preset, discovered or
/// configured); its alias only when the family has no live exact one. Codex answers exact
/// ids, so its entries, and a configured model whose id names no family, keep their places.
/// The alias stays in the settings, so a task naming it still starts (`model`).
fn served(models: &[Value]) -> Vec<Value> {
    let mut best: HashMap<&str, usize> = HashMap::new();
    for (i, m) in models.iter().enumerate() {
        let Some(f) = agents::family(m) else { continue };
        if best.get(f).is_none_or(|&j| rank(&models[j], f) < rank(m, f)) {
            best.insert(f, i);
        }
    }
    models
        .iter()
        .enumerate()
        .filter(|(i, m)| match agents::family(m) {
            Some(f) => best[f] == *i,
            None => true,
        })
        .map(|(_, m)| m.clone())
        .collect()
}

/// How well an entry stands for its family: a live exact model, then an alias
/// (live, or dropped with the family's exact model gone), then a dropped exact model.
fn rank(m: &Value, f: &str) -> u8 {
    match (m["gone"] != true, agents::is_alias(m, f)) {
        (true, false) => 2,
        (true, true) | (false, true) => 1,
        _ => 0,
    }
}
fn path_of(url: &str) -> Vec<&str> {
    url.split('?').next().unwrap_or("").trim_matches('/').split('/').collect()
}
/// The locks a change takes: its project's, and for a task, every project in
/// its scope (and in a new scope it is given), in folder order (D-61).
pub(super) fn guard(root: &Path, parts: Vec<&str>, body: &Value) -> Result<Vec<fs::File>, Failure> {
    let ["tasks", tid, rest @ ..] = parts.as_slice() else { return Ok(vec![lock(root)?]) };
    let top = scope::top(root);
    let home = scope::dir(&top, root);
    let members = workspace::members(&top)?;
    loop {
        let Ok(t) = task(root, tid) else { return Ok(vec![lock(root)?]) };
        let seen = scope::of(&t, &home);
        let mut dirs = seen.clone();
        dirs.extend(body["scope"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(String::from)));
        let mut roots: Vec<PathBuf> = dirs.iter().filter(|d| members.contains(d)).map(|d| scope::path(&top, d)).collect();
        roots.push(root.into());
        // Approving a settings proposal writes a file in the project it names.
        if let ["questions", qid] = rest
            && let Some(dir) =
                t["questions"].as_array().into_iter().flatten().find(|q| q["id"] == *qid).and_then(|q| q["settings"]["project"].as_str())
            && members.iter().any(|m| m == dir)
        {
            roots.push(scope::path(&top, dir));
        }
        let held = scope::lock_all(&roots)?;
        // The scope may have changed while this waited for the locks.
        if task(root, tid).is_ok_and(|t| scope::of(&t, &home) == seen) {
            return Ok(held);
        }
    }
}
/// Whether a human change to a task changes code, so it takes its projects'
/// locks (D-77 point 5): undoing the task's changes or settling them in a final
/// column, or approving a settings proposal. Cancelling a task that never had
/// changes, and every other change, is task-only. Reads without a lock.
pub(super) fn needs_lock(root: &Path, parts: &[&str], body: &Value) -> Result<bool, Failure> {
    let ["tasks", tid, rest @ ..] = parts else { return Ok(false) };
    let Ok(t) = task(root, tid) else { return Ok(false) };
    match rest {
        ["actions"] => {
            let name = body["action"].as_str().unwrap_or("");
            if matches!(name, "pause" | "resume" | "stop" | "end_discussion" | "discuss") {
                return Ok(false);
            }
            let w = flow(root)?;
            let a = &w["actions"][name];
            let mut moved = t.clone();
            if let Some(to) = a.get("to")
                && move_to(&w, &mut moved, to).is_err()
            {
                return Ok(false);
            }
            if a["revert"] != true && column(&w, &moved)["final"] != true {
                return Ok(false);
            }
            // A work run's command may be about to record: it holds the lock until it has.
            if t["runs"].as_array().into_iter().flatten().any(|r| r["kind"] == "work" && r["state"] != "ended") {
                return Ok(true);
            }
            let s = open(root)?;
            if !changes_of(&s, &t)?.is_empty() {
                return Ok(true);
            }
            for (_, st) in scope_stores(&s, &t)? {
                if st.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0))? {
                    return Ok(true);
                }
                // A run stopped mid-write left a record that recovery has yet to adopt.
                if lockdown::pending_for(&st.root, tid) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ["questions", qid] => {
            let answer = body.get("text").or_else(|| body.get("option")).and_then(|v| v.as_str());
            Ok(answer == Some("Approve")
                && t["questions"].as_array().into_iter().flatten().any(|q| q["id"] == *qid && !q["settings"].is_null()))
        }
        _ => Ok(false),
    }
}
/// A scope as a request gives it: projects on this board, sorted, at least one.
fn checked_scope(root: &Path, v: &Value) -> Result<Vec<String>, Failure> {
    let top = scope::top(root);
    let members = workspace::members(&top)?;
    let mut out = vec![];
    for d in v.as_array().ok_or_else(|| Failure("E7001 scope must be a list of project folders".into(), 1))? {
        match d.as_str() {
            Some(d) if members.iter().any(|m| m == d) => out.push(d.to_string()),
            _ => return err("E7001", format!("{d} is not a project on this board; its projects: {}", members.join(", "))),
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        return err("E7001", "a task needs at least one project");
    }
    Ok(out)
}
/// Refuses a work run in any of `dirs` while another task holds it (D-61).
fn claim_projects(root: &Path, w: &Value, tid: &Value, dirs: &[String]) -> Result<(), Failure> {
    let top = scope::top(root);
    for dir in dirs {
        let others: Vec<Value> = scope::tasks_in(&top, dir)?.into_iter().map(|(_, t)| t).filter(|t| t["id"] != *tid).collect();
        let place = if dirs.len() > 1 { format!(" in {dir}") } else { String::new() };
        if others.iter().any(|t| t["runs"].as_array().unwrap().iter().any(|r| r["kind"] == "work" && r["state"] != "ended")) {
            return err("E7005", format!("another task has a work run{place}"));
        }
        // Code changed outside Solace waits for a human before agents build on it (D-58).
        if others.iter().any(|t| t["unexpected"] == true && column(w, t)["final"] != true) {
            return err(
                "E7012",
                format!(
                    "complete or cancel the {} task{place} first: it holds code changed outside Solace that no one has reviewed",
                    outside::TITLE
                ),
            );
        }
    }
    Ok(())
}
/// `POST /tasks/{id}/scope` (D-61, D-127): the human adds projects to a task at
/// any time, and removes them while no agent of it is working: between runs,
/// paused, or waiting on a question or a discussion reply.
fn set_scope(s: &Store, w: &Value, mut t: Value, body: &Value) -> Result<(Value, After), Failure> {
    if column(w, &t)["final"] == true {
        return err("E7004", "final task");
    }
    let top = scope::top(&s.root);
    let home = scope::dir(&top, &s.root);
    let old = scope::of(&t, &home);
    let new = checked_scope(&s.root, &body["scope"])?;
    let working = t["runs"].as_array().unwrap().iter().any(|r| r["state"] == "running") && t["paused_from"].is_null();
    if working && old.iter().any(|d| !new.contains(d)) {
        return err(
            "E7003",
            "an agent is working on this task; you can add projects now, and remove them between runs, while it is paused, or while it waits on a question",
        );
    }
    // Takes no lock (D-77 point 5): a paused run could still record in a project
    // after this check and before it leaves the scope. The project's lock goes
    // with it; telling apart which task changed what is later work.
    let changes = changes_of(s, &t)?;
    for d in old.iter().filter(|d| !new.contains(d)) {
        if changes.iter().any(|c| c["project"] == d.as_str()) {
            return err("E7013", format!("this task has changed {d}; undo its changes before taking {d} out of its projects"));
        }
    }
    // An open work run already holds its projects; one added joins them now.
    if t["runs"].as_array().unwrap().iter().any(|r| r["kind"] == "work" && r["state"] != "ended") {
        let added: Vec<String> = new.iter().filter(|d| !old.contains(d)).cloned().collect();
        claim_projects(&s.root, w, &t["id"], &added)?;
    }
    t["scope"] = json!(new);
    if new.contains(&home) {
        return Ok((t, After::Nothing));
    }
    // The agent works from the task's home, so the home changes only between runs.
    if active(&t) {
        return err("E7013", format!("this task's agent works from {home}; take {home} out of its projects once the run has ended"));
    }
    move_task(&s.root, &mut t, &scope::path(&top, &new[0]))?;
    Ok((t, After::Removed(home)))
}
/// Moves a task's folder to another project of its scope when its home leaves it.
/// `from` is the project whose board holds the task's folder now.
pub(super) fn move_task(from: &Path, t: &mut Value, to: &Path) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap().to_string();
    let (from, into) = (from.join(".solace/board").join(&tid), to.join(".solace/board").join(&tid));
    open(to)?;
    // A conversation is tied to the folder it ran in; the next run starts a new one, with a handoff.
    t["conversation"] = Value::Null;
    if from.exists() {
        fs::rename(&from, &into)?;
        for r in t["runs"].as_array_mut().unwrap() {
            if let Some(rest) = r["log"].as_str().and_then(|l| Path::new(l).strip_prefix(&from).ok()) {
                r["log"] = json!(into.join(rest));
            }
        }
    }
    // The task stays in the root's store; only its home changes (D-77).
    t["home"] = json!(scope::dir(&scope::top(to), to));
    Ok(())
}
pub fn api(root: &Path, method: &str, url: &str, body: Value) -> Result<Value, Failure> {
    let (path, query_text) = url.split_once('?').unwrap_or((url, ""));
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    // Tasks are the task server's (D-77).
    if parts.first() == Some(&"tasks") {
        return tasks::api(root, method, &parts, body);
    }
    // Task routes lock only a change to code, in `tasks::api` (D-77 point 5); the rest lock every change here.
    // Reads skip the project lock, so the board stays responsive while an agent command holds it.
    // Task files are replaced by rename and the store waits out commits, so a read sees whole states.
    let _guard = if method == "GET" { None } else { Some(guard(root, path_of(url), &body)?) };
    let s = open(root)?;
    let w = flow(root)?;
    let query: HashMap<String, String> =
        query_text.split('&').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if matches!(parts.first(), Some(&"graph" | &"nodes" | &"tests" | &"files")) {
        return view::api(&s, method, &parts, &query, &body);
    }
    match (method, parts.as_slice()) {
        ("GET", ["workflow"]) => Ok(w),
        ("GET", ["models"]) => models(),
        ("POST", ["models", "try-again"]) => agents::try_again(&body).and_then(|_| models()),
        _ => err("E7001", "unknown route"),
    }
}
/// The project `root`'s tasks as a list shows them, without their notes and changes.
fn listed(root: &Path) -> Result<Vec<Value>, Failure> {
    let mut ts = tasks(root)?;
    for t in &mut ts {
        for k in ["notes", "changes"] {
            t.as_object_mut().unwrap().remove(k);
        }
    }
    Ok(ts)
}
/// `GET /tasks` and `GET /tasks/{id}/...` of the project `root`.
pub(super) fn read_task_api(root: &Path, parts: &[&str]) -> Result<Value, Failure> {
    match parts {
        ["tasks"] => Ok(json!(listed(root)?)),
        ["tasks", tid] => {
            let mut t = task(root, tid)?;
            // The task's changes are computed from its scope projects' stores (D-77 point 4).
            t["changes"] = json!(changes_of(&open(root)?, &t)?);
            Ok(t)
        }
        ["tasks", tid, "runs", rid] => {
            let t = task(root, tid)?;
            let mut run = t["runs"][run_at(&t, rid)?].clone();
            run["transcript"] = json!(transcript(Path::new(run["log"].as_str().unwrap_or(""))));
            Ok(run)
        }
        _ => err("E7001", "unknown route"),
    }
}
/// `POST /tasks`: a new task of the project `root`, not yet written.
pub(super) fn created(root: &Path, body: &Value) -> Result<Value, Failure> {
    let w = flow(root)?;
    let title = body["title"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| Failure("E7001 title required".into(), 1))?;
    if body.get("body").is_some_and(|x| !x.is_string()) || body.get("links").is_some_and(|x| !x.is_array()) {
        return err("E7001", "invalid task fields");
    }
    let top = scope::top(root);
    let home = scope::dir(&top, root);
    let scope = match body.get("scope") {
        Some(v) => checked_scope(root, v)?,
        None => vec![home.clone()],
    };
    if !scope.contains(&home) {
        return err("E7001", format!("a task is created in a project of its scope; post it to one of {}", scope.join(", ")));
    }
    let rank = tasks(root)?.iter().filter_map(|t| t["rank"].as_f64()).fold(0., f64::max) + 1.;
    let mut t = new_task(title, body["body"].as_str().unwrap_or(""), &w["columns"][0]["id"], rank);
    t["scope"] = json!(scope);
    t["links"] = body.get("links").cloned().unwrap_or(json!([]));
    t["home"] = json!(home);
    Ok(t)
}
/// What a change to a task leaves to do once the task server has written it.
pub(super) enum After {
    Nothing,
    /// Launch the run `(rid, model)`.
    Start(String, Value),
    /// The task reached a final column: its conversation goes. A task
    /// completed with its last review's tech debt carries the feature's Tech
    /// Debt task to write with it (D-112).
    Clean(Option<Value>),
    /// The task's home left its scope; the board's lane for this one drops it (D-61).
    Removed(String),
}
pub(super) fn after(root: &Path, t: &Value, after: After) {
    match after {
        After::Start(rid, m) => start(root.to_path_buf(), t.clone(), rid, m),
        After::Clean(_) => {
            let root = root.to_path_buf();
            let tid = t["id"].as_str().unwrap().to_string();
            std::thread::spawn(move || {
                if let Ok(_guard) = conversation_lock(&root, &tid) {
                    let _ = clean_conversation(&root, &tid);
                }
            });
        }
        After::Nothing | After::Removed(_) => {}
    }
}
/// The column a task waits in while it is not active, and the one it is
/// planned into (D-132): found by id, so a workflow without them is left alone.
const BACKLOG: &str = "backlog";
const PLANNING: &str = "planning";
/// Whether workflow `w` has a column with id `id`.
fn has_column(w: &Value, id: &str) -> bool {
    w["columns"].as_array().is_some_and(|cs| cs.iter().any(|c| c["id"] == id))
}
/// Moves task `t` from Backlog to Planning when feature `f` is `active` and
/// the task's workflow has both columns, with a `solace` note naming the
/// feature (D-132). An archived task is left where it is. Whether it moved;
/// nothing is written here.
pub(super) fn plan_for_feature(top: &Path, t: &mut Value, f: &Value) -> bool {
    if f["state"] != "active" || t["column"] != BACKLOG || !t["archivedTime"].is_null() {
        return false;
    }
    let Ok(w) = flow(&scope::path(top, t["home"].as_str().unwrap_or("."))) else { return false };
    if !has_column(&w, BACKLOG) || !has_column(&w, PLANNING) {
        return false;
    }
    t["column"] = json!(PLANNING);
    // The board shows a `feature:<id>` link with the feature's title (D-122).
    let id = f["id"].as_str().unwrap_or("");
    note(t, "solace", None, &format!("Moved to Planning: its feature feature:{id} is active."));
    true
}
/// Sets a task's `feature`, `milestone` and `dependsOn` from a `PATCH` (D-87,
/// D-188). A feature given always brings its milestone, even over one the
/// task had; a task with a feature cannot set its milestone directly, and a
/// task given an `active` feature is planned (D-132).
pub(super) fn set_links(top: &Path, t: &mut Value, body: &Value) -> Result<(), Failure> {
    for (k, kind) in [("feature", "features"), ("milestone", "milestones")] {
        match body.get(k) {
            None | Some(Value::Null) => {}
            Some(Value::String(id)) if tasks::record(top, kind, id).is_some() => {}
            Some(Value::String(id)) => return err("E7023", format!("unknown {k} {id}")),
            Some(_) => return err("E7001", format!("invalid {k}")),
        }
    }
    let feature = match body.get("feature") {
        Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => return err("E7001", "invalid feature"),
        None => t["feature"].as_str().map(String::from),
    };
    if body.get("milestone").is_some() && feature.is_some() {
        return err("E7001", "a task with a feature takes the feature's milestone; change the feature's milestone instead");
    }
    if let Some(f) = body.get("feature") {
        match f {
            Value::String(id) => {
                t["feature"] = f.clone();
                if let Some(r) = tasks::record(top, "features", id) {
                    t["milestone"] = r["milestone"].clone();
                    plan_for_feature(top, t, &r);
                }
            }
            _ => t["feature"] = Value::Null,
        }
    }
    if let Some(m) = body.get("milestone") {
        t["milestone"] = m.clone();
    }
    let Some(deps) = body.get("dependsOn") else { return Ok(()) };
    let Some(deps) = deps.as_array().filter(|d| d.iter().all(Value::is_string)) else { return err("E7001", "invalid dependsOn") };
    let mut ids: Vec<String> = vec![];
    for d in deps.iter().filter_map(Value::as_str) {
        if !ids.iter().any(|x| x == d) {
            ids.push(d.to_string());
        }
    }
    let me = t["id"].as_str().unwrap_or("").to_string();
    let mut graph: HashMap<String, Vec<String>> = all_tasks(top)?
        .iter()
        .map(|o| {
            let deps = o["dependsOn"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
            (o["id"].as_str().unwrap_or("").to_string(), deps)
        })
        .collect();
    for d in &ids {
        if *d == me || !graph.contains_key(d) {
            return err("E7024", format!("a task can depend only on another existing task; {d} is not one"));
        }
    }
    graph.insert(me.clone(), ids.clone());
    // A cycle is a path from one of the new dependencies back to this task.
    let (mut todo, mut seen) = (ids.clone(), HashSet::new());
    while let Some(id) = todo.pop() {
        if id == me {
            return err("E7025", "these dependencies would make a cycle");
        }
        if seen.insert(id.clone()) {
            todo.extend(graph.get(&id).cloned().unwrap_or_default());
        }
    }
    t["dependsOn"] = json!(ids);
    Ok(())
}
/// A change to task `tid` of the project `root` from the API (`method` on
/// `rest`, the path below `/tasks/{tid}`): the task as the server writes it.
pub(super) fn change(root: &Path, tid: &str, method: &str, rest: &[&str], body: &Value) -> Result<(Value, After), Failure> {
    let s = open(root)?;
    let w = flow(root)?;
    let mut t = task(root, tid)?;
    {
        {
            match (method, rest) {
                ("PATCH", []) => {
                    // Links can change during a run and in a final column (D-87 point 4, D-100).
                    let links_only =
                        body.as_object().is_some_and(|o| !o.is_empty() && o.keys().all(|k| k == "feature" || k == "milestone"));
                    if active(&t) && !links_only {
                        return err("E7003", "run going");
                    }
                    if column(&w, &t)["final"] == true && !links_only {
                        return err("E7004", "final task");
                    }
                    set_links(&scope::top(root), &mut t, body)?;
                    for k in ["title", "body", "links", "rank"] {
                        if let Some(v) = body.get(k) {
                            if (matches!(k, "title" | "body") && !v.is_string())
                                || (k == "links" && !v.is_array())
                                || (k == "rank" && !v.is_number())
                            {
                                return err("E7001", "invalid task field");
                            }
                            t[k] = v.clone();
                        }
                    }
                }
                ("POST", ["scope"]) => return set_scope(&s, &w, t, body),
                // Takes a finished task off the board; it stays in the archive (D-95).
                ("POST", ["archive"]) => {
                    if column(&w, &t)["final"] != true {
                        return err("E7001", "only a task in a final column can be archived");
                    }
                    if t["archivedTime"].is_null() {
                        t["archivedTime"] = json!(now());
                    }
                }
                ("POST", ["notes"]) => {
                    let text = body["text"].as_str().ok_or_else(|| Failure("E7001 text required".into(), 1))?;
                    // While a discussion is going, a human note is its next turn.
                    let discussing = discussion(&t);
                    note(&mut t, "human", discussing.as_deref(), text);
                }
                // Notes are only appended, so a note's position names it.
                ("DELETE", ["notes", i]) => {
                    let notes = t["notes"].as_array_mut().unwrap();
                    let i = i.parse().ok().filter(|i| *i < notes.len()).ok_or_else(|| Failure("E7001 unknown note".into(), 1))?;
                    notes.remove(i);
                }
                ("POST", ["questions", qid]) => {
                    let q = t["questions"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|q| q["id"] == *qid)
                        .ok_or_else(|| Failure("E7001 unknown question".into(), 1))?;
                    let answer = body
                        .get("text")
                        .or_else(|| body.get("option"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Failure("E7001 answer required".into(), 1))?;
                    if body.get("text").is_none() && !q["options"].as_array().unwrap().contains(&json!(answer)) {
                        return err("E7001", "option is not offered");
                    }
                    q["answer"] = json!(answer);
                    if !q["settings"].is_null() && answer == "Approve" {
                        apply_settings(root, q)?;
                    }
                    continue_answered(&mut t);
                }
                ("POST", ["actions"]) => {
                    let name = body["action"].as_str().unwrap_or("");
                    // A task with no project (D-129) can't run: no project has
                    // an agent to work on it. Cancel, archive and editing are
                    // the human's own, and need no project.
                    if t["noProject"] == true && !w["actions"][name]["run"].is_null() {
                        return err("E7003", format!("{name} needs a project; this task has none"));
                    }
                    let was_final = column(&w, &t)["final"] == true;
                    let launch = action(&s, &w, &mut t, body)?;
                    // A held question or a stopped work run stays flagged until the run
                    // continues, restarts, stops or the task settles (D-82).
                    let a = &w["actions"][name];
                    let resolves = matches!(a["run"].as_str(), Some("work" | "new_work" | "investigate"))
                        || a["revert"] == true
                        || name == "stop"
                        || column(&w, &t)["final"] == true;
                    if t["attention"]["hold"] != true || resolves {
                        t["attention"] = Value::Null;
                    }
                    t["finished"] = Value::Null;
                    let after = match launch {
                        Some((rid, m)) => After::Start(rid, m),
                        None if column(&w, &t)["final"] == true => {
                            // Completed with its changes kept, not canceled: the last
                            // review's tech debt goes to the feature (D-112).
                            let debt = if !was_final && a["revert"] != true {
                                tech_debt(&scope::top(root), &t).unwrap_or_else(|e| {
                                    eprintln!("filing tech debt of task {tid}: {}", e.0);
                                    None
                                })
                            } else {
                                None
                            };
                            if let Some((d, n)) = &debt {
                                let items = if *n == 1 { "1 tech-debt item".to_string() } else { format!("{n} tech-debt items") };
                                note(
                                    &mut t,
                                    "solace",
                                    None,
                                    &format!("Filed {items} from the last review on task:{}.", d["id"].as_str().unwrap_or("")),
                                );
                            }
                            After::Clean(debt.map(|(d, _)| d))
                        }
                        None => After::Nothing,
                    };
                    return Ok((t, after));
                }
                _ => return err("E7001", "unknown route"),
            }
        }
    }
    Ok((t, After::Nothing))
}
fn start(root: PathBuf, t: Value, rid: String, m: Value) {
    std::thread::spawn(move || {
        let tid = t["id"].as_str().unwrap();
        let result = (|| {
            // A follow-up may be queued while the previous agent is exiting.
            // Never open one provider conversation in two processes.
            let _guard = conversation_lock(&root, tid)?;
            let result = agent_process(&root, &task(&root, tid)?, &rid, &m, true);
            let mut result = match result {
                Err(Failure(_, SESSION_GONE)) => agent_process(&root, &task(&root, tid)?, &rid, &m, false),
                other => other,
            };
            // A discussion agent that exited after replying gets a fresh agent,
            // briefed with every note, at the human's next note.
            while let Err(Failure(_, REPLIED)) = result {
                wait_discussion(&open(&root)?, tid, &rid)?;
                result = agent_process(&root, &task(&root, tid)?, &rid, &m, false);
            }
            let current = task(&root, tid)?;
            if t["runs"].as_array().unwrap().iter().any(|r| r["id"] == rid && matches!(r["kind"].as_str(), Some("review" | "discuss")))
                || m["run"].is_array()
            {
                let dir = conversation_dir(&root, tid, &rid)?;
                if dir.exists() {
                    fs::remove_dir_all(dir)?;
                }
            }
            if column(&flow(&root)?, &current)["final"] == true {
                clean_conversation(&root, tid)?;
            }
            result
        })();
        if let Err(e) = result {
            let _ = finish(&root, tid, &rid, Some(&e.0));
        }
    });
}

fn conversation_lock(root: &Path, tid: &str) -> Result<fs::File, Failure> {
    let dir = task_dir(root, tid)?;
    fs::create_dir_all(&dir)?;
    lock_file(&dir.join("agent.lock"))
}

fn conversation_dir(root: &Path, tid: &str, home: &str) -> Result<PathBuf, Failure> {
    // Reuse the id validator before allowing any deletion or launch path.
    let _ = task_path(root, home)?;
    Ok(task_dir(root, tid)?.join(home))
}

fn clean_conversation(root: &Path, tid: &str) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let t = task(root, tid)?;
    if let Some(home) = t["conversation"]["home"].as_str() {
        let p = conversation_dir(root, tid, home)?;
        if p.exists() {
            fs::remove_dir_all(p)?;
        }
    }
    drafts::clear(&s, tid)?;
    tasks::update(root, tid, |t| {
        t["conversation"] = Value::Null;
        Ok(())
    })
}

fn session_id(log: &Path, agent: &str) -> Option<String> {
    fs::read_to_string(log).ok()?.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find_map(|e| {
        let id = if agent == "codex" && e["type"] == "thread.started" {
            e["thread_id"].as_str()
        } else if agent == "claude" && e["type"] == "system" && e["subtype"] == "init" {
            e["session_id"].as_str()
        } else {
            None
        }?;
        (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| id.to_string())
    })
}

/// `agent_process`'s failure when the agent can't resume its conversation:
/// the run starts again with a fresh one.
const SESSION_GONE: i32 = 3;
/// `agent_process`'s result when a discussion agent replied in plain text and
/// exited: the reply is recorded and the run waits for the human.
const REPLIED: i32 = 4;
fn agent_process(root: &Path, t: &Value, rid: &str, m: &Value, allow_resume: bool) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let run = &t["runs"][run_at(t, rid)?];
    let log = PathBuf::from(run["log"].as_str().unwrap());
    fs::create_dir_all(log.parent().unwrap())?;
    if run["state"] == "ended" {
        return Ok(());
    }
    // A review or discussion has its own conversation, never the author's.
    let author = !matches!(run["kind"].as_str(), Some("review" | "discuss"));
    let persistent = persists(m);
    let previous = &t["conversation"];
    // Compare launch-relevant configuration, not labels. A changed provider, endpoint or runner gets a handoff; model and effort are launch flags and keep the session (D-75).
    let compatibility = json!({"agent":m["agent"],"endpoint":m["endpoint"],"run":m["run"]});
    let resume = if allow_resume && author && persistent && run["fresh"] != true && previous["config"] == compatibility {
        previous["home"]
            .as_str()
            .and_then(|home| conversation_dir(root, tid, home).ok())
            .filter(|p| p.is_dir())
            .and_then(|_| previous["session"].as_str())
            .map(str::to_string)
    } else {
        None
    };
    let home = if resume.is_some() {
        previous["home"].as_str().unwrap().to_string()
    } else if allow_resume {
        rid.to_string()
    } else {
        format!("{rid}-fallback")
    };
    let dir = conversation_dir(root, tid, &home)?;
    if author
        && resume.is_none()
        && let Some(old) = previous["home"].as_str()
    {
        let old = conversation_dir(root, tid, old)?;
        if old != dir && old.exists() {
            fs::remove_dir_all(old)?;
        }
    }
    // A run starts in the project of its task holding the most symbols the
    // task names (D-123); a resumed conversation stays where it was.
    let here = run_project(previous, resume.is_some(), root, t);
    let at = scope::path(&scope::top(root), &here);
    let (prompt, images) = continuation_brief(t, run, resume.is_some(), root, &at, m["run"].is_array());
    let timeout = system()?["task_timeout"].as_u64().unwrap_or(3600);
    if author {
        // Only an exited process has a reusable session; a server crash during
        // a turn falls back instead of resuming a possibly orphaned process.
        let conversation = if persistent { json!({"home":home,"config":compatibility,"session":null,"project":here}) } else { Value::Null };
        tasks::update(root, tid, move |current| {
            current["conversation"] = conversation;
            Ok(())
        })?;
    }
    let mut child = spawn_agent(&at, &dir, &log, m, &prompt, &images, resume.as_deref(), persistent, [tid, rid], timeout)?;
    let supervised =
        supervise(Watch::Task(root, tid), rid, &mut child, &log, resume.is_some(), Duration::from_secs(timeout), GRACEFUL_EXIT);
    let (started, outcome) = run_state(root, tid, rid);
    // A discussion agent that replied in plain text took its turn: its reply
    // answers like a finished run, so it clears a mark the same way.
    let outcome = match &supervised {
        Err(Failure(_, REPLIED)) => Some("discussed".to_string()),
        _ => outcome,
    };
    agents::after_run(m, &log, started, outcome.as_deref());
    if !(author && persistent) && matches!(supervised, Err(Failure(_, REPLIED))) && dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    let forced = supervised?;
    if author && persistent {
        let session = if forced { Value::Null } else { json!(session_id(&log, agent_of(m))) };
        let (briefed, rid) = (t["notes"].clone(), rid.to_string());
        tasks::update(root, tid, move |current| {
            current["conversation"]["session"] = session;
            // Only notes actually briefed, plus the agent's own notes, are known to
            // this conversation. A human may add feedback during graceful exit.
            current["conversation"]["notes_seen"] = json!(
                briefed
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(current["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent"))
                    .map(note_fingerprint)
                    .collect::<Vec<_>>()
            );
            Ok(())
        })?;
    } else {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

fn agent_of(m: &Value) -> &str {
    m["agent"].as_str().unwrap_or("codex")
}
fn claude_home() -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".claude"))
}
/// Whether model `m`'s agent keeps its conversation in its own folder.
fn persists(m: &Value) -> bool {
    // macOS keychain entries are keyed to CLAUDE_CONFIG_DIR. Keep that sign-in
    // working without copying credentials: use a fresh, nonpersistent handoff.
    m["run"].is_null()
        && !(m["agent"] == "claude"
            && cfg!(target_os = "macos")
            && !claude_home().join(".credentials.json").exists()
            && m["endpoint"].is_null()
            && env::var_os("ANTHROPIC_API_KEY").is_none()
            && env::var_os("ANTHROPIC_AUTH_TOKEN").is_none())
}
/// The project task `t`, at home in `home`, starts its runs in (D-123): the
/// one of its scope with the most symbols its title and body name as
/// `module.name`; the home when none has more.
fn start_project(home: &Path, t: &Value) -> String {
    let top = scope::top(home);
    let mine = scope::dir(&top, home);
    let text = format!("{}\n{}", t["title"].as_str().unwrap_or(""), t["body"].as_str().unwrap_or(""));
    let mut candidates = vec![mine.clone()];
    candidates.extend(scope::of(t, &mine).into_iter().filter(|d| *d != mine));
    let mut best = (0, mine);
    for d in candidates {
        let path = scope::path(&top, &d);
        if crate::language::detect(&path).is_none() {
            continue;
        }
        let Ok(s) = Store::peek(&path) else { continue };
        let Ok(ix) = s.index() else { continue };
        let n = s.named(&ix, &text).len();
        if n > best.0 {
            best = (n, d);
        }
    }
    best.1
}

/// The project a run starts in: a resumed conversation keeps the project its
/// session began in — the one recorded with it, or the task's home for a
/// conversation saved before projects were recorded. A fresh run starts where
/// its task's symbols are (D-123).
fn run_project(previous: &Value, resume: bool, home: &Path, t: &Value) -> String {
    match previous["project"].as_str() {
        Some(p) if resume => p.to_string(),
        None if resume => scope::dir(&scope::top(home), home),
        _ => start_project(home, t),
    }
}
/// Starts model `m`'s agent with its settings in `dir`, its output in `log`
/// and `prompt` (with `images`) on stdin: the launch every run shares, a
/// task's or a strategy run's. `ids` are its `SOLACE_TASK` and `SOLACE_RUN`.
#[allow(clippy::too_many_arguments)]
fn spawn_agent(
    root: &Path,
    dir: &Path,
    log: &Path,
    m: &Value,
    prompt: &str,
    images: &[PathBuf],
    resume: Option<&str>,
    persistent: bool,
    ids: [&str; 2],
    timeout: u64,
) -> Result<std::process::Child, Failure> {
    let custom = m["run"].as_array();
    let native = m["run"].is_null();
    // `SOLACE_BIN` names the CLI agents call when the server is another program (D-80).
    let binary = match env::var_os("SOLACE_BIN").filter(|b| !b.is_empty()) {
        Some(b) => PathBuf::from(b),
        None => env::current_exe()?,
    };
    let agent = agent_of(m);
    let stored_key = m["id"].as_str().and_then(|id| setup::stored(&setup::keys_path(), id)).filter(|_| m.get("endpoint").is_some());
    let launch: Value = serde_json::from_str(&lockdown::agent_config(agent, dir, &binary, timeout)?).unwrap();
    let mut argv: Vec<String> = if let Some(a) = custom {
        a.iter().filter_map(|x| x.as_str().map(String::from)).collect()
    } else if agent == "codex" {
        vec![
            "codex".into(),
            "exec".into(),
            "--json".into(),
            "--skip-git-repo-check".into(),
            "-m".into(),
            m["model"].as_str().unwrap_or("").into(),
        ]
    } else {
        vec![
            "claude".into(),
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--model".into(),
            m["model"].as_str().unwrap_or("").into(),
        ]
    };
    if custom.is_none() {
        if agent == "claude" && !persistent {
            argv.push("--no-session-persistence".into());
        }
        if let Some(session) = resume {
            if agent == "codex" {
                argv.insert(2, "resume".into());
                argv.push(session.into());
            } else {
                argv.extend(["--resume".into(), session.into()]);
            }
        }
        if agent == "claude" {
            if let Some(e) = m["effort"].as_str() {
                argv.extend(["--effort".into(), e.into()]);
            }
            if !images.is_empty() {
                argv.extend(["--input-format".into(), "stream-json".into()]);
            }
        }
        argv.extend(launch["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()));
        if agent == "codex" {
            if let Some(e) = m["effort"].as_str() {
                argv.extend(["-c".into(), format!("model_reasoning_effort={}", json!(e))]);
            }
            // `--image` takes several values, so each goes before `--json`, never before `-`.
            let at = if resume.is_some() { 3 } else { 2 };
            for image in images.iter().rev() {
                argv.splice(at..at, ["--image".into(), image.to_string_lossy().into_owned()]);
            }
            argv.extend(endpoint_args(m, stored_key.is_some()));
            argv.push("-".into());
        }
    }
    if argv.is_empty() {
        return err("E7001", "empty model command");
    }
    // Authentication is shared; settings, plugins and hooks are isolated.
    if agent == "codex" && custom.is_none() {
        let auth = PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".codex/auth.json");
        if auth.exists() && !dir.join("auth.json").exists() {
            std::os::unix::fs::symlink(auth, dir.join("auth.json"))?;
        }
    }
    if agent == "claude" && native {
        let auth = claude_home().join(".credentials.json");
        // A token refresh replaces the symlink with a private copy whose rotated
        // token goes stale; relink so every launch uses the real sign-in.
        let link = dir.join(".credentials.json");
        if auth.exists() && !fs::read_link(&link).is_ok_and(|t| t == auth) {
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(auth, link)?;
        }
    }
    let file = fs::File::create(log)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(root).process_group(0).stdin(Stdio::piped()).stdout(file.try_clone()?).stderr(file);
    for (k, v) in launch["env"].as_object().unwrap() {
        cmd.env(k, v.as_str().unwrap());
    }
    if agent == "codex"
        && let Some(key) = &stored_key
    {
        cmd.env(setup::KEY_VAR, key);
    }
    if agent == "claude" && custom.is_none() {
        if persistent {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        if let Some(e) = m.get("endpoint") {
            cmd.env("ANTHROPIC_BASE_URL", e["base_url"].as_str().unwrap_or(""));
            if let Some(key) = setup::endpoint_key(m) {
                cmd.env("ANTHROPIC_AUTH_TOKEN", key);
            }
            if env::var_os("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none()
                && let Some(n) = endpoint_context(e["base_url"].as_str().unwrap_or(""), m["model"].as_str().unwrap_or(""))
            {
                cmd.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", n.to_string());
            }
        }
    }
    cmd.env("SOLACE_TASK", ids[0]).env("SOLACE_RUN", ids[1]).env("SOLACE_SESSION", ids[1]).env("SOLACE_BIN", &binary);
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    if agent == "claude" && custom.is_none() && !images.is_empty() {
        // Claude Code takes images only inside a stream-json user message.
        let mut content = vec![json!({"type":"text","text":prompt})];
        for image in images {
            let media = image_type(&image.to_string_lossy()).unwrap_or("image/png");
            content.push(json!({"type":"image","source":{"type":"base64","media_type":media,"data":base64(&fs::read(image)?)}}));
        }
        writeln!(stdin, "{}", json!({"type":"user","message":{"role":"user","content":content}}))?;
    } else {
        writeln!(stdin, "{prompt}")?;
    }
    drop(stdin);
    Ok(child)
}

/// How long a run that called a finishing tool may take to exit (spec 15).
const GRACEFUL_EXIT: Duration = Duration::from_secs(30);

/// Watches a launched agent until it exits or is killed, and says whether it
/// was killed. Running time (not paused time) past `timeout` ends the run; a
/// run that finished gets `grace` to exit; any other ended run is killed.
#[allow(clippy::too_many_arguments)]
fn supervise(
    watch: Watch,
    rid: &str,
    child: &mut std::process::Child,
    log: &Path,
    resumed: bool,
    timeout: Duration,
    grace: Duration,
) -> Result<bool, Failure> {
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    let mut ending = None;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let (run, paused) = watch.run(rid)?;
        let r = &run;
        if r["state"] == "running" && !paused {
            elapsed += last.elapsed();
        }
        last = Instant::now();
        let expired = r["state"] != "ended" && elapsed >= timeout;
        let ended = r["state"] == "ended";
        if ended && matches!(r["outcome"].as_str(), Some("finished" | "reported" | "reviewed" | "discussed" | "planned")) {
            ending.get_or_insert_with(Instant::now);
        }
        if let Some(status) = child.try_wait()? {
            if !ended {
                let entries = transcript(log);
                let error = entries.iter().rev().find(|e| e["kind"] == "error").and_then(|e| e["text"].as_str()).unwrap_or("");
                let raw = fs::read_to_string(log).unwrap_or_default().to_lowercase();
                // Retry only a failed session load, before any tool execution. Never retry quota or work failures.
                if resumed
                    && !status.success()
                    && !entries.iter().any(|e| e["kind"] == "tool")
                    && ["session not found", "no session found", "failed to resume", "could not find session", "unable to find session"]
                        .iter()
                        .any(|s| raw.contains(s))
                {
                    return Err(Failure("session unavailable; using a fresh handoff".into(), SESSION_GONE));
                }
                // A discussion or strategy agent that replied in plain text and exited
                // cleanly has taken its turn: the reply is its note, and the run waits.
                let reply =
                    entries.iter().rev().find(|e| e["kind"] == "message").and_then(|e| e["text"].as_str()).map(str::trim).unwrap_or("");
                if matches!(r["kind"].as_str(), Some("discuss" | "strategy")) && status.success() && error.is_empty() && !reply.is_empty() {
                    watch.reply(rid, reply.to_string())?;
                    return Err(Failure("discussion reply".into(), REPLIED));
                }
                if let Some(assessment) = recoverable_review(r, status.success(), &entries)
                    && record_review(&watch, rid, &assessment)?
                {
                    return Ok(false);
                }
                let said = if !error.is_empty() { Some(error.to_string()) } else { last_words(&entries) };
                let message = format!("agent exited without finishing ({status}){}", said.map(|s| format!(":\n\n{s}")).unwrap_or_default());
                watch.fail(rid, &message)?;
            }
            return Ok(false);
        }
        if expired || (ended && ending.is_none()) || ending.is_some_and(|at| at.elapsed() >= grace) {
            lockdown::kill_group(child);
            if expired {
                watch.fail(rid, "task timeout")?;
            }
            return Ok(true);
        }
    }
}

/// What `supervise` watches: a task's run (the project and task), or a
/// strategy run of the root (D-93).
enum Watch<'a> {
    Task(&'a Path, &'a str),
    Strategy(&'a Path),
}
impl Watch<'_> {
    /// The run as it is now, and whether its task is paused.
    fn run(&self, rid: &str) -> Result<(Value, bool), Failure> {
        match self {
            Watch::Task(root, tid) => {
                let t = task(root, tid)?;
                let i = run_at(&t, rid)?;
                Ok((t["runs"][i].clone(), !t["paused_from"].is_null()))
            }
            Watch::Strategy(top) => match tasks::strategy_run(top, rid) {
                Some(run) => Ok((run, false)),
                None => err("E7023", format!("unknown strategy run {rid}")),
            },
        }
    }
    /// Records a plain-text reply as the agent's note; the run waits for the human.
    fn reply(&self, rid: &str, reply: String) -> Result<(), Failure> {
        let rid = rid.to_string();
        match self {
            Watch::Task(root, tid) => tasks::update(root, tid, move |t| {
                let i = run_at(t, &rid)?;
                if t["runs"][i]["state"] != "ended" {
                    note(t, "agent", Some(&rid), &reply);
                    t["runs"][i]["state"] = json!("waiting");
                }
                Ok(())
            }),
            Watch::Strategy(top) => tasks::update_run(top, &rid.clone(), move |r| {
                if r["state"] != "ended" {
                    note(r, "agent", Some(&rid), &reply);
                    r["state"] = json!("waiting");
                }
                Ok(())
            }),
        }
    }
    /// Ends the run as failed, saying why. A strategy run ends `stopped`, with
    /// its notes as one transcript note.
    fn fail(&self, rid: &str, message: &str) -> Result<(), Failure> {
        match self {
            Watch::Task(root, tid) => finish(root, tid, rid, Some(message)),
            Watch::Strategy(top) => {
                let (rid, message) = (rid.to_string(), message.to_string());
                tasks::update_run(top, &rid.clone(), move |r| {
                    if r["state"] != "ended" {
                        note(r, "solace", Some(&rid), &message);
                        tasks::end_strategy(r, "stopped");
                    }
                    Ok(())
                })
            }
        }
    }
}

/// Follow-up review context comes from live notes joined to their runs, never transcripts.
fn review_context(s: &Store, t: &Value) -> Out {
    let mut out = String::from("\nReview context:\n");
    if let Some(previous) = previous_review(t) {
        let rid = previous["id"].as_str().unwrap();
        out += &format!("Previous completed review: {rid}\n");
        let feedback: Vec<_> = t["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent").collect();
        if feedback.is_empty() {
            out += "Previous feedback was deleted or is unavailable; do not infer its assessment.\n";
        }
        for n in feedback {
            out += &format!("Review feedback ({rid}): {}\n", n["text"].as_str().unwrap_or(""));
        }
        let at = run_at(t, rid)?;
        for n in t["notes"].as_array().unwrap() {
            let subsequent = n["run"].as_str().and_then(|id| run_at(t, id).ok()).filter(|i| *i > at);
            if let Some(i) = subsequent {
                out += &format!(
                    "Subsequent {} ({}) — {}: {}\n",
                    t["runs"][i]["kind"].as_str().unwrap_or("run"),
                    t["runs"][i]["id"].as_str().unwrap(),
                    n["author"].as_str().unwrap_or(""),
                    n["text"].as_str().unwrap_or("")
                );
            } else if n["author"] == "human" {
                out += &format!("Human feedback: {}\n", n["text"].as_str().unwrap_or(""));
            }
        }
    }
    match review_checkpoint(s, t) {
        Ok(checkpoint) => {
            out += "The task's changes are below: the delta since the last review. Assess the delta, prior unresolved findings and affected dependencies. Run `task diff` only when the human asks for a fresh full review, or for parts left out below.\n";
            let current = review_state(s, t)?;
            if checkpoint["changed_during_review"] != false {
                out += "Project or instructions changed during the previous review, or stability is unknown. Its checkpoint represents review-start content only; recheck affected code and context before carrying forward any assessment.\n";
            }
            if checkpoint["context"] == current["context"]
                && checkpoint["instructions"] == current["instructions"]
                && checkpoint["changed_during_review"] == false
            {
                out += "Code and relevant context are unchanged since the previous review. A short report carrying forward the previous assessment is sufficient if its feedback is available. Unchanged code must not turn Needs work into Looks good.\n";
            } else {
                out += "Code or relevant context changed since the previous review. Even an empty task delta does not establish unchanged dependencies or instructions; inspect affected context and rerun relevant verification.\n";
            }
            out += &task_diff(s, t["id"].as_str().unwrap(), &["--since-review"], 8000)?;
        }
        Err(reason) => {
            out += &format!("No usable delta baseline: {reason}. The task's changes are below (the whole task); assess all of them.\n");
            out += &task_diff(s, t["id"].as_str().unwrap(), &[], 8000)?;
        }
    }
    Ok(out)
}

/// The brief, plus the images its notes reference, which are attached to the prompt.
/// `here` is the project the run's agent works in (D-123).
fn continuation_brief(t: &Value, run: &Value, resumed: bool, root: &Path, here: &Path, custom: bool) -> (String, Vec<PathBuf>) {
    let kind = run["kind"].as_str().unwrap_or("");
    let mut packet = t.clone();
    if resumed {
        let seen: HashSet<&str> = t["conversation"]["notes_seen"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        packet["notes"] =
            json!(t["notes"].as_array().unwrap().iter().filter(|n| !seen.contains(note_fingerprint(n).as_str())).collect::<Vec<_>>());
    }
    let tid = t["id"].as_str().unwrap_or("");
    let mut images = vec![];
    for n in packet["notes"].as_array_mut().into_iter().flatten() {
        let text = note_images(n["text"].as_str().unwrap_or(""), &format!("/tasks/{tid}/files/"), &mut images);
        n["text"] = json!(text);
    }
    let images: Vec<PathBuf> = images.iter().filter_map(|name| image_path(root, tid, name).ok()).collect();
    packet["home"] = json!(scope::dir(&scope::top(root), root));
    // The task's changes come from the scope projects' stores (D-77 point 4).
    packet["changes"] = json!(open(root).map(|s| changes_of(&s, &packet).unwrap_or_default()).unwrap_or_default());
    // The brief says where the agent works.
    packet["home"] = json!(scope::dir(&scope::top(root), here));
    // D-182: a work brief names the marked test commands of every project it covers.
    if kind == "work" {
        let (tests, errors) = marked_tests(&scope::top(root), &packet);
        packet["test_commands"] = json!(tests);
        if !errors.is_empty() {
            packet["test_commands_error"] = json!(errors.join("; "));
        }
    }
    let mut out = brief(&packet, kind);
    out += &feature_section(&scope::top(root), t);
    if resumed {
        out = out.replace(START, "Use the continuation context below.");
    }
    // D-151: a fresh run's brief holds its session start reply, so its first
    // call reads; the brief has its feature and dependencies already.
    let reply = if resumed || !out.contains(START) { None } else { session_reply(t, here) };
    if reply.is_some() {
        out = out.replace(START, HELD_START);
    }
    out += &waiting_on(&scope::top(root), tid);
    if !matches!(kind, "review" | "discuss") && t["runs"].as_array().is_some_and(|r| r.len() > 1) {
        if resumed {
            out += "\nContinue the author's conversation. The role and permissions above replace those of earlier runs. Skip broad orientation; inspect current versions of symbols you need before editing.\n";
        } else {
            out += "\nFresh conversation: use this handoff and the notes above to continue existing work. Reconcile partial edits before proceeding.\n";
        }
        for r in t["runs"].as_array().unwrap().iter().rev().filter(|r| r["id"] != run["id"]).take(3) {
            out += &format!("Previous {}: {}\n", r["kind"].as_str().unwrap_or("run"), r["outcome"].as_str().unwrap_or("unfinished"));
            if let Some(log) = r["log"].as_str()
                && let Some(said) = last_words(&transcript(Path::new(log)))
            {
                out += &format!("Last activity: {said}\n");
            }
        }
    }
    if let Ok(s) = open(root) {
        if kind == "review" {
            match review_context(&s, t) {
                Ok(context) => out += &context,
                Err(e) => out += &format!("\nReview context unavailable: {}. Run `task diff` to see the whole task and assess it.\n", e.0),
            }
        }
        // A discussion runs no checks, so it gets the decision titles instead of the verification record.
        if kind == "discuss" {
            let decisions = fs::read_to_string(scope::top(root).join("DECISIONS.md")).unwrap_or_default();
            let titles: Vec<String> =
                decisions.lines().filter_map(|l| l.strip_prefix("## ")).filter(|l| l.starts_with("D-")).map(|l| format!("- {l}")).collect();
            if !titles.is_empty() {
                out += &format!("\nDecisions (DECISIONS.md; `grep D-<n>` then `read` for a record):\n{}\n", titles.join("\n"));
            }
        } else if let Some(checks) = t["verification"].as_object() {
            let current = review_state(&s, t).ok().and_then(|v| v["context"].as_str().map(str::to_string));
            out += "\nLast recorded verification (rerun stale checks before claiming success):\n";
            for (command, v) in checks {
                let stale = current.as_deref() != v["base"].as_str();
                out += &format!(
                    "- {command}: {}{}\n{}\n",
                    if v["passed"] == true { "passed" } else { "failed" },
                    if stale { " (stale: project changed)" } else { "" },
                    v["summary"].as_str().unwrap_or("")
                );
            }
        }
        if let Ok(drafts) = drafts::list_for(&s, t["id"].as_str().unwrap_or(""))
            && !drafts.is_empty()
        {
            out += &format!("\nRetained drafts (view with `view --draft <id>`):\n{drafts}\n");
        }
    }
    out += &vision_section(&scope::top(root), kind);
    out += &skills::section(here);
    if !images.is_empty() {
        out += "\nImages in the notes are attached to this message as [image 1], [image 2], ... in order.\n";
        if custom {
            for (i, p) in images.iter().enumerate() {
                out += &format!("[image {}: {}]\n", i + 1, p.display());
            }
        }
    }
    if let Some(reply) = reply {
        out += &format!("\nSession start (the reply to `session start --task`):\n{reply}");
    }
    // A discussion's `task note` returns the human's next message, so the run
    // ends with the call that says so, not with the first note.
    out +=
        "\nAfter the call that ends the run returns, stop: no more calls, and a final response of one line at most; then exit normally.\n";
    (out, images)
}

/// The reply `session start --task` gives task `t`'s agent, with the task's
/// title and description, in `here`, the project its run works in (D-151);
/// none when that folder has no store, or when building it fails, which is
/// logged so the brief's plain start line can be traced.
fn session_reply(t: &Value, here: &Path) -> Option<String> {
    if !here.join(".solace/store.db").exists() {
        return None;
    }
    let text = |k: &str| t[k].as_str().unwrap_or("").trim().to_string();
    let task = format!("{}\n{}", text("title"), text("body"));
    let tid = t["id"].as_str()?;
    match Store::at(here).and_then(|s| s.task_session(task.trim_end(), Some(tid), "")) {
        Ok(reply) => Some(reply),
        Err(e) => {
            eprintln!("session start for the brief of task {tid}: {}", e.0);
            None
        }
    }
}

/// The most of the vision a brief carries (D-91): all of it, since a save is
/// held to the same limit (D-92). Only a vision saved before the limit is cut.
const VISION_CAP: usize = tasks::VISION_LIMIT;

/// The most of a feature note a brief carries (D-138), as for the vision.
const FEATURE_CAP: usize = VISION_CAP;

/// Task `t`'s feature for its brief and `session start --task` (D-138): the
/// feature's title, then the most recent of the task's investigation report,
/// assignment note (the human's, outside any run) and review feedback, cut at
/// a line end within `FEATURE_CAP` characters. Empty when the task has no
/// feature, or its feature is gone.
pub fn feature_section(top: &Path, t: &Value) -> String {
    let Some(f) = t["feature"].as_str().and_then(|fid| tasks::record(top, "features", fid)) else { return String::new() };
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let runs = t["runs"].as_array().cloned().unwrap_or_default();
    let kind_of = |n: &Value| runs.iter().find(|r| r["id"] == n["run"]).map(|r| text(&r["kind"])).unwrap_or_default();
    let label = |n: &Value| match (n["author"].as_str(), n["run"].as_str(), kind_of(n).as_str()) {
        (Some("agent"), Some(rid), "investigate") => Some(format!("investigation report (run {rid})")),
        (Some("agent"), Some(rid), "review") => Some(format!("review feedback (run {rid})")),
        (Some("human"), None, _) => Some("assignment".to_string()),
        _ => None,
    };
    let mut out = format!("\nFeature (the feature this task is part of; its latest direction):\n- {}\n", text(&f["title"]));
    // An empty note gives no direction, so the next one is used.
    let latest =
        t["notes"].as_array().into_iter().flatten().rev().find_map(|n| Some((label(n)?, text(&n["text"]))).filter(|(_, x)| !x.is_empty()));
    if let Some((label, note)) = latest {
        let mut shown = note.clone();
        if note.chars().count() > FEATURE_CAP {
            let cut: String = note.chars().take(FEATURE_CAP).collect();
            shown = match cut.rfind('\n') {
                Some(i) if i > 0 => cut[..i].trim_end().to_string(),
                _ => cut,
            };
            shown += "\n(cut short; the whole note is in the task's notes on the board)";
        }
        out += &format!("- {label}: {shown}\n");
    }
    out
}

/// `feature_section` for task `tid` of the root `top`.
pub fn feature_of(top: &Path, tid: &str) -> String {
    all_tasks(top).ok().and_then(|all| all.into_iter().find(|o| o["id"] == tid)).map(|t| feature_section(top, &t)).unwrap_or_default()
}

/// What a brief says when a dependency is unfinished (D-110): advice, nothing is refused.
const WAITING: &str = "This task depends on unfinished work. Check whether you can do your part without it. If you can't, ask the human (`task ask`) before building on code that doesn't exist yet.";

/// Task `tid`'s dependencies for its brief and `session start --task` (D-110):
/// each with its id, title and column, from any project of the root, the
/// unfinished ones marked, then advice. Unfinished is as on the board: not in a
/// final column, so complete and canceled are finished. Empty when no
/// dependency is unfinished, so the brief stays short.
pub fn waiting_on(top: &Path, tid: &str) -> String {
    let (Ok(all), Ok(w)) = (all_tasks(top), flow(top)) else { return String::new() };
    let Some(t) = all.iter().find(|o| o["id"] == tid) else { return String::new() };
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let (mut lines, mut unfinished) = (vec![], false);
    for id in t["dependsOn"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        // A deleted task is no work to wait for.
        let Some(d) = all.iter().find(|o| o["id"] == id) else { continue };
        let c = column(&w, d);
        let open = c["final"] != true;
        unfinished |= open;
        let shown = c["label"].as_str().or(c["id"].as_str()).or(d["column"].as_str()).unwrap_or("");
        let project = if d["home"] != t["home"] { format!(", in {}", text(&d["home"])) } else { String::new() };
        lines.push(format!("- {id} {} ({shown}{project}){}", text(&d["title"]), if open { " — unfinished" } else { "" }));
    }
    if !unfinished {
        return String::new();
    }
    format!("\nDepends on:\n{}\n{WAITING}\n", lines.join("\n"))
}

/// The root's vision for an investigate, work or discuss brief (D-91): the
/// whole text, or for an older, longer one its first `VISION_CAP` characters,
/// cut at a line end when one is in reach. None for a review, or when there is
/// no vision.
fn vision_section(top: &Path, kind: &str) -> String {
    let text = tasks::vision(top)["text"].as_str().unwrap_or("").trim().to_string();
    if kind == "review" || text.is_empty() {
        return String::new();
    }
    let mut shown = text.clone();
    if text.chars().count() > VISION_CAP {
        let cut: String = text.chars().take(VISION_CAP).collect();
        shown = match cut.rfind('\n') {
            Some(i) if i > 0 => cut[..i].trim_end().to_string(),
            _ => cut,
        };
        shown += "\n(cut short; the whole vision is on the Strategy page)";
    }
    format!("\nVision (what the product is for, kept on the Strategy page; let it guide your choices):\n{shown}\n")
}

/// Replaces each markdown image of an uploaded file with `[image N]`, numbering
/// files in first-seen order. Other images and text are left as written.
fn note_images(text: &str, prefix: &str, found: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("![") {
        let after = &rest[i..];
        let Some(open) = after.find("](") else { break };
        let Some(close) = after[open..].find(')').map(|c| open + c) else { break };
        out += &rest[..i];
        match after[open + 2..close].strip_prefix(prefix).filter(|n| image_type(n).is_some()) {
            Some(name) => {
                let n = found.iter().position(|x| x == name).unwrap_or_else(|| {
                    found.push(name.to_string());
                    found.len() - 1
                });
                out += &format!("[image {}]", n + 1);
            }
            None => out += &after[..=close],
        }
        rest = &after[close + 1..];
    }
    out + rest
}

/// Note images: content-checked raster formats only (SVG can carry script).
const IMAGE_LIMIT: usize = 5 * 1024 * 1024;
fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}
/// The media type of a stored image's name, which is `<16 hex>.<ext>`.
fn image_type(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match ext {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
fn image_path(root: &Path, tid: &str, name: &str) -> Result<PathBuf, Failure> {
    let p = task_dir(root, tid)?.join("files").join(name);
    if image_type(name).is_none() || !p.exists() {
        return err("E7001", "unknown image");
    }
    Ok(p)
}
pub fn upload(root: &Path, tid: &str, bytes: &[u8]) -> Result<Value, Failure> {
    task(root, tid)?;
    if bytes.len() > IMAGE_LIMIT {
        return err("E7001", "image over 5 MB");
    }
    let ext = image_kind(bytes).ok_or_else(|| Failure("E7001 only PNG, JPEG, GIF or WebP images".into(), 1))?;
    let name = format!("{}.{ext}", &sha256(bytes)[..16]);
    let dir = task_dir(root, tid)?.join("files");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(&name), bytes)?;
    Ok(json!({"name":name,"url":format!("/tasks/{tid}/files/{name}")}))
}
fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn note_fingerprint(note: &Value) -> String {
    sha256(note.to_string().as_bytes())
}

pub fn record_verification(s: &Store, tid: &str, command: &str, result: &Out) -> Result<(), Failure> {
    let t = task(&s.root, tid)?;
    let summary = match result {
        Ok(out) => out,
        Err(e) => &e.0,
    };
    let result =
        json!({"passed":result.is_ok(),"base":review_state(s, &t)?["context"],"summary":summary.chars().take(1000).collect::<String>()});
    tasks::send(&scope::top(&s.root), json!({"kind":"verification","task":tid,"command":command,"result":result}))?;
    Ok(())
}
/// D-157: saves run `rid`'s reads on its run, through the inbox.
pub(super) fn record_reads(top: &Path, tid: &str, rid: &str, reads: Value) -> Result<(), Failure> {
    tasks::send(top, json!({"kind":"reads","task":tid,"run":rid,"reads":reads}))?;
    Ok(())
}
/// D-157: run `rid`'s reads as its MCP process last saved them.
pub(super) fn run_reads(top: &Path, tid: &str, rid: &str) -> Value {
    run_record(top, tid, rid).map_or(Value::Null, |r| r["reads"].clone())
}
/// D-157: run `rid`'s steps so far, one line each (D-99), the latest that fit in `budget`.
pub(super) fn run_outline(top: &Path, tid: &str, rid: &str, budget: usize) -> String {
    let Some(r) = run_record(top, tid, rid) else {
        return String::new();
    };
    outline(&transcript(Path::new(r["log"].as_str().unwrap_or(""))), budget)
}

/// `session remember`: the task's context and the run's own record, for an
/// agent that noticed its conversation was compacted. It reads only and is
/// repeatable; a discussion's brief already holds its context, so it is
/// refused, and so is a run that isn't there (D-157 point 7).
pub(super) fn remember(top: &Path, tid: &str, rid: &str, budget: usize) -> Out {
    let Some(run) = run_record(top, tid, rid) else { return err("E7003", format!("unknown run {rid}")) };
    if run["kind"] == "discuss" {
        return err("E7010", "a discussion's brief holds its context; reply with `task note`");
    }
    let t = read_task(top, &task_path(top, tid)?)?;
    let mut context = task_sections(&t);
    context += &feature_section(top, &t);
    context += &waiting_on(top, tid);
    let reads = reads::window_reads().unwrap_or_else(|| run_reads(top, tid, rid));
    Ok(reads::remember_reply(&context, &reads, top, &|b| run_outline(top, tid, rid, b), budget))
}

fn run_record(top: &Path, tid: &str, rid: &str) -> Option<Value> {
    let t = read_task(top, &task_path(top, tid).ok()?).ok()?;
    let i = run_at(&t, rid).ok()?;
    Some(t["runs"][i].clone())
}
/// The tool calls and messages of `steps`, one line each and numbered as in
/// the run log: the latest that fit in `budget`, after how many were left out.
fn outline(steps: &[Value], budget: usize) -> String {
    let lines: Vec<String> = steps
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e["kind"].as_str(), Some("tool" | "message")))
        .map(|(i, e)| step_line(i, e) + "\n")
        .collect();
    let (mut kept, mut size) = (0, 0);
    for l in lines.iter().rev() {
        // room for the line naming what's left out
        if size + l.len() + 40 > budget {
            break;
        }
        size += l.len();
        kept += 1;
    }
    let left = lines.len() - kept;
    let head = if left > 0 { format!("Earlier steps left out: {left}\n") } else { String::new() };
    if head.len() > budget {
        return String::new();
    }
    head + &lines[left..].concat()
}
/// A line cut to 100 characters, with `…` when more was cut.
fn cut_line(what: &str) -> String {
    let cut: String = what.chars().take(100).collect();
    let more = if cut.len() < what.len() { "…" } else { "" };
    format!("{cut}{more}")
}
/// One line for a transcript entry: its number, kind, and text's first line cut to 100.
fn step_line(i: usize, e: &Value) -> String {
    let kind = e["kind"].as_str().unwrap_or("");
    let what = match kind {
        "tool" => format!("{} ({})", e["title"].as_str().unwrap_or("").lines().next().unwrap_or(""), e["status"].as_str().unwrap_or("")),
        _ => e["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string(),
    };
    format!("{} {kind}: {}", i + 1, cut_line(&what))
}
/// An agent's prompt: what its run is for, how to start, ask and finish, then
/// what it needs from the task (Spec 15 §6), as plain text.
/// A fresh run's first step; a resumed run continues instead.
const START: &str = "Start with `session start --task`, with the task's title and description as `text`.";
/// A fresh run's first step when its brief holds the session start reply (D-151).
const HELD_START: &str = "The reply to `session start --task`, with the task's title and description, is at the end of this brief: start with the call its `Next:` line names. Send `session start --task` again only to see that reply again, for example after the conversation is compacted.";
/// What a review of a task in a feature is asked to add (D-112).
const TECH_DEBT_ASK: &str = "This task is part of a feature: put minor, non-blocking findings that later work in the feature should fix under a `### Tech debt` heading in the feedback, one finding per bullet with its location (`module.name` or a file). When the task is completed, Solace files each on the feature's Tech Debt task; findings that block this task belong in the findings, not there.";
/// The task's context as a brief gives it: title, body, links, notes and
/// questions. Its feature and dependencies are added where a brief adds them.
fn task_sections(t: &Value) -> String {
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let mut out = format!("Task: {}\n", text(&t["title"]));
    if !text(&t["body"]).is_empty() {
        out += &format!("\n{}\n", text(&t["body"]));
    }
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            out += &format!("\n{title}:\n{}\n", lines.join("\n"));
        }
    };
    let list = |k: &str| t[k].as_array().cloned().unwrap_or_default();
    section("Links", list("links").iter().map(|l| format!("- {}: {}", text(&l["name"]), text(&l["url"]))).collect());
    section(
        "Notes so far",
        list("notes")
            .iter()
            .map(|n| {
                let body = text(&n["text"]);
                let run = list("runs").into_iter().find(|r| r["id"] == n["run"]);
                match run {
                    Some(r) => format!("- {} ({} run {}): {body}", text(&n["author"]), text(&r["kind"]), text(&r["id"])),
                    None => format!("- {}: {body}", text(&n["author"])),
                }
            })
            .collect(),
    );
    section(
        "Questions",
        list("questions")
            .iter()
            .map(|q| match q["answer"].as_str() {
                Some(a) => format!("- {}\n  Answer: {a}", text(&q["text"])),
                None => format!("- {}\n  (not answered yet)", text(&q["text"])),
            })
            .collect(),
    );
    out
}

fn brief(t: &Value, kind: &str) -> String {
    let start = START;
    let ask = "If you need a decision from the human, check the Notes so far and Questions below, if any, for what is already answered (a discussion run can't `task ask`, so its answers are notes); else ask with `task ask` (`text` and 2–4 `options`); the call waits for the answer. In the question's `text`, give each option a short example of the outcome the human would see (what a user sees, what a command prints, what a page shows, what a file or record ends up as), not how the code would change; the `options` stay short labels matching the bold names in the examples. A question carries its own explanation and is the only command in its call: don't add a `task note` beside it.";
    let settings = "To change `solace.toml` or `AGENTS.md`, `write` it alone, with the why in `reason`.";
    let responses = "`text` and `options` are arguments of the Solace tool, beside `command`: in a task message, `command` is just the command (such as `task note`), and the whole message, however long, goes in `text`, never in `command` and never as a `--text` flag. Write human-facing task messages in Markdown, including the text sent to task tools. Divide the body into relevant sections with short headings (for example, ### Findings and ### Verification), with blank lines around headings and lists. Omit irrelevant sections. Keep messages short: lead with the answer or outcome, then only what the human needs to act on; don't restate the task or narrate your steps. When asking a question, explain what you need and why it matters (for example, ### Question and ### Why this matters); in `text`, give each option a short example of its outcome in the human's terms, its name in bold, and keep the `options` labels short, matching the bold names.";
    let read_only = "This run is read-only: reading, `check`, `test`, `build` and `probe` work; `submit`, `delete`, `extract`, `merge`, `write`, `remove` and `format` are refused, and `run <name>` works only for commands marked `allowed_in_reviews` in `solace.toml`.";
    let verify = "Use those to verify (for example test suites).";
    let define = "Probe or run tests only when needed to more thoroughly define the issue.";
    let common = "Code held as symbols is read and cited by name (`lockdown.dispatch`, `checkout.pay`), not by source path: source files are projections written by Solace.";
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let list = |k: &str| t[k].as_array().cloned().unwrap_or_default();
    let home = text(&t["home"]);
    let projects: Vec<String> = list("scope").iter().map(text).collect();
    // D-182: the commands each project marks as tests, so a work brief names
    // them once instead of making the agent guess among every declared command.
    let marked = t["test_commands"].as_array().cloned().unwrap_or_default();
    let tests = if marked.is_empty() {
        String::new()
    } else {
        let suites = marked
            .iter()
            .map(|p| {
                let names: Vec<&str> = p["commands"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let names = names.join(", ");
                if projects.len() > 1 { format!("{}: {names}", text(&p["project"])) } else { names }
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(", and `run <name>` for: {suites}")
    };
    let tests = match t["test_commands_error"].as_str() {
        Some(e) if marked.is_empty() => format!("{tests}; the marked test commands could not be read: {e}"),
        Some(e) => format!("{tests}; the marked test commands could not be read for: {e}"),
        None => tests,
    };
    // A task in a feature files its review's minor findings as the feature's tech debt (D-112).
    let debt = if t["feature"].is_string() { format!(" {TECH_DEBT_ASK}") } else { String::new() };
    let role = match kind {
        "review" => format!(
            "Review the changes this task made and give feedback. The task, its notes and its changes are all in this brief (the delta since the last review, or the whole task, in the review context below), so don't send `session start --task`. Run `task diff` only when the human asks for a fresh full review, or for parts the brief left out. For each concern, inspect every symbol it needs in one call (`inspect a b c --context`); split only when the reply is trimmed. A call is a turn, so batch the symbols one concern needs; start from the changed symbols the review context names. Keep the concern-by-concern focus. Don't review generated files (build output such as a bundled `priv/app.js`, lockfiles, installed dependencies); test them instead, and rebuild them with the project's commands if a check needs them. Flag code the task didn't need, places it missed, and any validation, data-loss handling or security it removed. Flag duplication and large modules, filing minor ones as tech debt; don't allow cross-domain concerns in the same module, and flag excessively long functions and dead code as tech debt. Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI. {read_only} {verify} Only a `task note` call records the feedback; a final message is not recorded. If the call is refused, resend it, and don't claim it was recorded unless the reply says `Recorded.` Finish with `task note`, the feedback in `text`. Begin the feedback with exactly one assessment label: **Assessment: Looks good** or **Assessment: Needs work**. Follow it with findings and verification that explain the assessment, including any checks not performed.{debt}"
        ),
        "discuss" => format!(
            "Discuss this task with the human, who asks questions and works out ideas with you. Answer from the high-level context below (the task, its notes and changes); read code only when a question needs it, and briefly. {read_only} {verify} The human's message is the last note. Reply only with `task note`, the reply in `text`, never as plain text; the call waits and returns the human's next message. Keep replying until a call says the discussion has ended, then stop without further calls."
        ),
        "work" => format!(
            "Make the change this task describes. {start} {ask} {settings} Send every change with its check in the same call: `submit` then `test`, `write` then `run <name>`. Before `task done`, after your last change, run `test --all` in each project with symbol-held source{tests}. Fix the code where a test fails because of your changes; in the summary, report failing tests your changes don't affect and any test you could not run. Reuse existing code before adding new code, extracting common patterns. Prefer small, stateless, self-documenting functions; keep modules and functions small and focused around one domain or component. Use clear names for new symbols. Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI. Follow the module boundaries the refactor task:2414798176c466e4 creates: put new code in the module that owns its concern. Finish with `task done`, a summary in `text`: what you changed, how you checked it, and, if any, what you skipped and risks the human must know."
        ),
        _ => format!(
            "Investigate this task before any work on it: answer what it asks, or find out what the change involves. Report the required behavior, constraints, acceptance criteria and risks, and anything you didn't check. Name the symbols, modules and non-source files that need attention when that helps define the work. Don't prescribe implementation, edits, submission shape or checks. The human never edits files or runs commands; any needed change, a settings change included, is the worker's to make. If a smaller option would change what the human gets, offer it with `task ask`. {read_only} {define} {start} {ask} Finish with `task done`, the report in `text`."
        ),
    };
    let mut out = format!(
        "You are working on a task from this project's task board. You work only through the Solace tool: there is no shell and no file access.\n{role}\n{common}\n{responses}\n\n{}",
        task_sections(t)
    );
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            out += &format!("\n{title}:\n{}\n", lines.join("\n"));
        }
    };
    // A task across projects (D-61): where the agent works, and how it reaches the others.
    if projects.len() > 1 {
        section(
            "Projects",
            vec![format!(
                "This task covers the projects {}. You work in {home}; for another of them, add `--project <folder>` (its folder as listed, from the Solace root) to any command (`inspect`, `submit`, `test`, `write`...). A project not listed is out of this task's reach: if you need it, ask the human with `task ask`.",
                projects.join(", ")
            )],
        );
    }
    let shown = |c: &Value| match c["project"].as_str() {
        Some(p) if projects.len() > 1 => format!("- {p}: {}", text(&c["name"])),
        _ => format!("- {}", text(&c["name"])),
    };
    section("Task changes", list("changes").iter().map(shown).collect());
    out
}
/// The agent's last message, else its last error or output, cut to 1000 bytes.
fn last_words(entries: &[Value]) -> Option<String> {
    let last = |kind: &str| entries.iter().rev().find(|e| e["kind"] == kind).and_then(|e| e["text"].as_str());
    let text = last("message").or_else(|| last("error")).or_else(|| last("output"))?.trim();
    if text.is_empty() {
        return None;
    }
    let cut = (text.len().saturating_sub(1000)..=text.len()).find(|&i| text.is_char_boundary(i)).unwrap();
    Some(if cut > 0 { format!("…{}", &text[cut..]) } else { text.to_string() })
}
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
/// D-109: a failed batch where only some commands failed, from the reply's
/// last line (`1 of 3 commands failed: …`): `(1 of 3 failed)`, for the step's title.
/// D-119: a call that stopped at a failed change (`…; skipped: a; b`) adds `, 2 skipped`;
/// the count is capped at the commands the reply says were left, since a command's
/// text can itself hold `; `.
fn partly_failed(status: &str, output: &str) -> Option<String> {
    let last = output.trim_end().lines().last()?;
    let (last, listed) = match last.rsplit_once("; skipped: ") {
        Some((l, s)) => (l, s.split("; ").count()),
        None => (last, 0),
    };
    let (n, rest) = last.split_once(" of ")?;
    let (m, _) = rest.split_once(" commands failed: ")?;
    let (n, m): (usize, usize) = (n.parse().ok()?, m.parse().ok()?);
    let skipped = listed.min(m.saturating_sub(n));
    let skipped = if skipped > 0 { format!(", {skipped} skipped") } else { String::new() };
    (status == "failed" && n < m).then(|| format!("({n} of {m} failed{skipped})"))
}
/// A tool call's input for display: the command first, then any other arguments.
fn tool_input(args: &Value) -> (String, String) {
    let command = args["command"].as_str().unwrap_or("").to_string();
    let rest = match args {
        Value::Object(m) => m
            .iter()
            .filter(|(k, _)| *k != "command")
            .map(|(k, v)| format!("{k}:\n{}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    };
    (command, rest)
}
/// Reads an agent's log (Codex `exec --json` or Claude Code `stream-json`) into
/// entries a board can show: message, thinking, tool, usage, error or output.
// rereads the whole log per request; keep an offset if logs get large.
pub fn transcript(log: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    let mut tools: HashMap<String, usize> = HashMap::new();
    // Claude Code: the context of its latest API call, and output per call (events repeat per content block).
    let (mut context, mut output, mut ended) = (None, HashMap::new(), false);
    let text = fs::read_to_string(log).unwrap_or_default();
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            if line.trim().is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last["kind"] == "output" => last["text"] = json!(format!("{}\n{line}", last["text"].as_str().unwrap())),
                _ => out.push(json!({"kind":"output","text":line})),
            }
            continue;
        };
        let item = &e["item"];
        match (e["type"].as_str().unwrap_or(""), item["type"].as_str().unwrap_or("")) {
            // Codex
            ("item.started" | "item.updated" | "item.completed", "mcp_tool_call") => {
                let (command, input) = tool_input(&item["arguments"]);
                let status = match item["status"].as_str() {
                    Some("completed") => "ok",
                    Some("failed") => "failed",
                    _ => "running",
                };
                let output = if item["error"].is_null() { text_of(&item["result"]["content"]) } else { text_of(&item["error"]["message"]) };
                let tool = item["tool"].as_str().unwrap_or("tool");
                let title = match partly_failed(status, &output) {
                    Some(p) => format!("{tool} {p} {command}"),
                    None => format!("{tool} {command}"),
                };
                let entry = json!({"kind":"tool","title":title,"input":input,"output":output,"status":status});
                let id = item["id"].as_str().unwrap_or("").to_string();
                match tools.get(&id) {
                    Some(&i) => out[i] = entry,
                    None => {
                        tools.insert(id, out.len());
                        out.push(entry);
                    }
                }
            }
            ("item.completed", "agent_message") => out.push(json!({"kind":"message","text":item["text"]})),
            ("item.completed", "reasoning") => out.push(json!({"kind":"thinking","text":item["text"]})),
            ("item.completed", "web_search") => out.push(json!({"kind":"tool","title":format!("web search {}", item["query"].as_str().unwrap_or("")),"input":"","output":"","status":"ok"})),
            ("item.completed", "error") => out.push(json!({"kind":"error","text":item["message"]})),
            ("turn.completed", _) => {
                let u = &e["usage"];
                out.push(json!({"kind":"usage","text":format!("{} input tokens ({} cached), {} output tokens", u["input_tokens"], u["cached_input_tokens"], u["output_tokens"])}));
            }
            ("error", _) => out.push(json!({"kind":"error","text":e["message"]})),
            ("turn.failed", _) => out.push(json!({"kind":"error","text":e["error"]["message"]})),
            // Claude Code
            ("assistant", _) => {
                let u = &e["message"]["usage"];
                if u.is_object() {
                    context = Some(context_tokens(u));
                    output.insert(e["message"]["id"].to_string(), u["output_tokens"].as_u64().unwrap_or(0));
                }
                for c in e["message"]["content"].as_array().unwrap_or(&vec![]) {
                    match c["type"].as_str() {
                        Some("text") => out.push(json!({"kind":"message","text":c["text"]})),
                        Some("thinking") => out.push(json!({"kind":"thinking","text":c["thinking"]})),
                        Some("tool_use") => {
                            let (command, input) = tool_input(&c["input"]);
                            let name = c["name"].as_str().unwrap_or("tool").trim_start_matches("mcp__solace__");
                            tools.insert(c["id"].as_str().unwrap_or("").to_string(), out.len());
                            out.push(json!({"kind":"tool","title":format!("{name} {command}"),"input":input,"output":"","status":"running"}));
                        }
                        _ => {}
                    }
                }
            }
            ("user", _) => {
                for c in e["message"]["content"].as_array().unwrap_or(&vec![]) {
                    if let Some(&i) = c["tool_use_id"].as_str().and_then(|id| tools.get(id)) {
                        out[i]["output"] = json!(text_of(&c["content"]));
                        out[i]["status"] = json!(if c["is_error"] == true { "failed" } else { "ok" });
                        if let Some(p) = partly_failed(out[i]["status"].as_str().unwrap_or(""), out[i]["output"].as_str().unwrap_or("")) {
                            out[i]["title"] = json!(out[i]["title"].as_str().unwrap_or("").replacen(' ', &format!(" {p} "), 1));
                        }
                    }
                }
            }
            ("result", _) => {
                ended = true;
                let u = &e["usage"];
                let last = context.map(|c| format!("; last context {c} tokens")).unwrap_or_default();
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let written = if u.get("cache_creation_input_tokens").is_some() {
                    format!(", {} written to cache", n("cache_creation_input_tokens"))
                } else {
                    String::new()
                };
                out.push(json!({"kind":"usage","text":format!("{} input tokens ({} read from cache{written}), {} output tokens{last}", context_tokens(u), n("cache_read_input_tokens"), n("output_tokens"))}));
                if e["is_error"] == true {
                    out.push(json!({"kind":"error","text":text_of(&e["result"])}));
                }
            }
            _ => {}
        }
    }
    if let (Some(c), false) = (context, ended) {
        // Live, until the run's totals arrive. Some endpoints report output only at the end.
        out.push(json!({"kind":"usage","text":format!("Context {c} tokens; {} output tokens so far", output.values().sum::<u64>())}));
    }
    out
}
/// Tokens an API call sent. Anthropic counts cached input separately and always
/// reports cache creation; endpoints that omit it (LM Studio) include cache in `input_tokens`.
fn context_tokens(u: &Value) -> u64 {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    if u.get("cache_creation_input_tokens").is_some() {
        n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens")
    } else {
        n("input_tokens")
    }
}
fn finish(root: &Path, tid: &str, rid: &str, failure: Option<&str>) -> Result<(), Failure> {
    let w = flow(root)?;
    let (rid, failure) = (rid.to_string(), failure.map(String::from));
    tasks::update(root, tid, move |t| {
        let i = run_at(t, &rid)?;
        let r = &mut t["runs"][i];
        if r["state"] == "ended" {
            return Ok(());
        }
        end_run(r, failure.as_deref().unwrap_or("finished"));
        // Only a failed work run moves the task; a failed review or investigation stays in its column.
        let work = r["kind"] == "work";
        if let Some(message) = &failure {
            note(t, "solace", Some(&rid), message);
            if work {
                hold(t, message);
                on_event(&w, t, "failed")?;
            } else {
                flag(t, message);
            }
        }
        Ok(())
    })
}

/// A task run's task and run, from its agent's environment; none in a
/// strategy run.
pub fn context() -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(ids) = TEST_CONTEXT.with(|c| c.borrow().clone()) {
        return Some(ids);
    }
    task_ids(env::var("SOLACE_TASK").ok(), env::var("SOLACE_RUN").ok())
}
/// The task run's ids: both of them, and the task is not a strategy run's.
pub(super) fn task_ids(task: Option<String>, run: Option<String>) -> Option<(String, String)> {
    Some((task.filter(|t| t != STRATEGY)?, run?))
}
/// `SOLACE_TASK` in a strategy run's agent (D-93). No task has this id: task
/// ids are hex digits.
const STRATEGY: &str = "strategy";
/// A strategy run's id, from its agent's environment.
pub fn strategy_context() -> Option<String> {
    env::var("SOLACE_TASK").ok().filter(|t| t == STRATEGY)?;
    env::var("SOLACE_RUN").ok()
}
/// Waits until the run may act. A `task` command takes no lock: it sends the
/// task server a message, and reads run state from the task file, which is
/// replaced by rename (D-77 point 5). Any other command holds its home's lock
/// and, for a command in another project of its scope (`also`), that
/// project's too (D-61).
pub fn agent_guard(root: &Path, tid: &str, rid: &str, a: &[&str], also: &[&Path]) -> Result<Vec<fs::File>, Failure> {
    loop {
        let guard = if a.first() == Some(&"task") {
            vec![]
        } else {
            scope::lock_all(&[root].iter().chain(also).map(|p| p.to_path_buf()).collect::<Vec<_>>())?
        };
        let t = task(root, tid)?;
        let r = &t["runs"][run_at(&t, rid)?];
        if r["state"] == "ended" {
            return err("E7003", "run ended");
        }
        if !t["paused_from"].is_null() {
            drop(guard);
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // The run waits for an answer: an agent that went on without it (its ask was
        // queued) holds still until the human replies, or stops if no server is there to hear.
        if r["state"] == "waiting" && a.first() != Some(&"task") {
            drop(guard);
            if !tasks::serving(&scope::top(root)) {
                return err("E7003", "the run waits for an answer and no board server is running; stop now");
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if HOST.contains(&a[0]) {
            return err("E7010", "host command is unavailable in a run");
        }
        // Every read-only run may run the commands a human marked (D-67, D-161).
        if r["kind"] != "work" && a[0] == "run" && a.len() > 1 {
            let marked = lockdown::settings(also.first().copied().unwrap_or(root))
                .ok()
                .and_then(|v| v.get("commands")?.get(a[1])?.get("allowed_in_reviews")?.as_bool())
                .unwrap_or(false);
            if !marked {
                return err("E7010", "this run is read-only: run only works for commands marked allowed_in_reviews in solace.toml");
            }
        } else if r["kind"] != "work" && CHANGES.contains(&a[0]) && a[0] != "build" {
            return err("E7010", "this run is read-only");
        }
        // The human may have changed the task's projects while it was paused.
        for p in also {
            let top = scope::top(root);
            let dir = scope::dir(&top, p);
            if !scope::of(&t, &scope::dir(&top, root)).contains(&dir) {
                return lockdown::error("E6007", &dir, "no longer one of this task's projects");
            }
        }
        return Ok(guard);
    }
}
pub fn task_command(
    s: &Store,
    tid: &str,
    rid: &str,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    options: &[String],
    guard: Vec<fs::File>,
) -> Out {
    let t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let i = run_at(&t, rid)?;
    // A message written into `command` (`task note --text "..."`) arrives cut at
    // its first line; refuse it so the agent resends it whole.
    if matches!(a.get(1).copied(), Some("note" | "done" | "ask")) && a.get(2).is_some_and(|w| w.starts_with("--")) {
        return err(
            "E7015",
            &format!("nothing was sent. Resend with `command` set to just `task {}` and the whole message in the `text` argument", a[1]),
        );
    }
    let text = if a.len() > 2 { a[2..].join(" ") } else { input()? };
    let discussing = t["runs"][i]["kind"] == "discuss";
    let top = scope::top(&s.root);
    // The agent sends the task server a message; the server applies it (D-77).
    let mut m = json!({"kind":"agent","task":tid,"run":rid,"command":a.get(1),"text":text});
    match a.get(1).copied() {
        // A discussion's reply waits for the human's next message.
        Some("note") if discussing => {
            let sent = tasks::send(&top, m)?;
            drop(guard);
            if !tasks::applied(&top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(&top, &sent) {
                return err("E7001", REJECTED);
            }
            return wait_discussion(s, tid, rid);
        }
        Some("done" | "ask") if discussing => return err("E7010", "a discussion goes on until the human ends it; reply with task note"),
        Some("note") => {
            if t["runs"][i]["kind"] == "review" {
                m = review_message(s, tid, &t, rid, &text)?;
            }
        }
        Some("done") => {
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "reviews finish with task note");
            }
        }
        Some("ask") => {
            if !(2..=4).contains(&options.len()) {
                return err("E7001", "task ask needs 2–4 options");
            }
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "review cannot move task with a question");
            }
            m["question"] = json!({"id":unique_id(),"text":text,"options":options});
            message(&w, &mut t.clone(), &m)?;
            let sent = tasks::send(&top, m)?;
            drop(guard);
            if !tasks::applied(&top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(&top, &sent) {
                return err("E7001", REJECTED);
            }
            return wait_reply(s, tid, rid);
        }
        _ => return usage("task note | task ask | task done, with the text in `text`; task diff [--since-review] [name ...]"),
    }
    // The task as the server will have it: checked here, so a refusal reaches the agent.
    let mut next = t.clone();
    message(&w, &mut next, &m)?;
    // Moving into a final column settles on the server, once it has applied the
    // move (`tasks::settle_later`): the task may have changed since this copy.
    tasks::send(&top, m)?;
    Ok(if tasks::serving(&top) { "Recorded.\n".into() } else { format!("{}\n", tasks::QUEUED) })
}

/// Applies a message from an agent or a command to task `t` (D-77 point 3):
/// the same change on the server and, to check it first, in the sender.
pub(super) fn message(w: &Value, t: &mut Value, m: &Value) -> Result<(), Failure> {
    let rid = m["run"].as_str();
    let text = m["text"].as_str().unwrap_or("");
    match m["kind"].as_str().unwrap_or("") {
        "agent" => {
            let rid = rid.unwrap_or("");
            let i = run_at(t, rid)?;
            let kind = t["runs"][i]["kind"].as_str().unwrap_or("").to_string();
            note(t, "agent", Some(rid), text);
            // The run ended (stopped, cancelled, restarted) before this arrived: the
            // note is kept, but it must not wake the run or move the task.
            if t["runs"][i]["state"] == "ended" {
                return Ok(());
            }
            match m["command"].as_str().unwrap_or("") {
                "note" if kind == "discuss" => t["runs"][i]["state"] = json!("waiting"),
                "note" if kind == "review" => {
                    if m["changed_during_review"] == true {
                        note(
                            t,
                            "solace",
                            Some(rid),
                            "Project or instructions changed during review. This feedback applies to the review-start checkpoint, not a verified finish-time state; recheck affected changes.",
                        );
                    }
                    end_run(&mut t["runs"][i], "reviewed");
                    t["finished"] = json!({"kind":"review","outcome":"reviewed","time":now()});
                }
                "note" => {}
                "done" => {
                    if kind == "investigate" {
                        // The report is the result; the task stays in planning.
                        end_run(&mut t["runs"][i], "reported");
                    } else {
                        end_run(&mut t["runs"][i], "finished");
                        move_to(w, t, &w["events"]["finished"])?;
                    }
                    t["finished"] = json!({"kind":t["runs"][i]["kind"],"outcome":t["runs"][i]["outcome"],"time":now()});
                }
                "ask" => {
                    t["questions"].as_array_mut().unwrap().push(m["question"].clone());
                    t["runs"][i]["state"] = json!("waiting");
                    hold(t, "an agent asked a question");
                    // An investigation waits in planning; `investigate` continues it.
                    if kind == "work" {
                        on_event(w, t, "asked")?;
                    }
                }
                _ => return err("E7001", "unknown agent message"),
            }
        }
        // The human replied to a discussion; its agent goes on.
        "resume" => {
            let i = run_at(t, rid.unwrap_or(""))?;
            if t["runs"][i]["state"] == "waiting" {
                t["runs"][i]["state"] = json!("running");
            }
        }
        "settings" => {
            let i = run_at(t, rid.unwrap_or(""))?;
            if t["runs"][i]["state"] == "ended" {
                let late = format!(
                    "A settings proposal arrived after the run ended and was not applied: {}",
                    m["question"]["text"].as_str().unwrap_or("")
                );
                note(t, "solace", rid, &late);
                return Ok(());
            }
            t["questions"].as_array_mut().unwrap().push(m["question"].clone());
            t["runs"][i]["state"] = json!("waiting");
            hold(t, "an agent proposed a settings change");
            on_event(w, t, "asked")?;
        }
        // A changes message from before D-77 point 4: the board_touches rows are the
        // source, so applying one adds nothing. One left in an old inbox is ignored.
        "changes" => {}
        "verification" => {
            if !t["verification"].is_object() {
                t["verification"] = json!({});
            }
            t["verification"][m["command"].as_str().unwrap_or("")] = m["result"].clone();
        }
        "reads" => {
            let i = run_at(t, rid.unwrap_or(""))?;
            t["runs"][i]["reads"] = m["reads"].clone();
        }
        "note" => {
            note(t, m["author"].as_str().unwrap_or("solace"), rid, text);
            if let Some(reason) = m["flag"].as_str() {
                if m["hold"] == true { hold(t, reason) } else { flag(t, reason) }
            }
        }
        _ => return err("E7001", "unknown task message"),
    }
    Ok(())
}

/// Holds the agent's call until its answers arrive and the run continues. The
/// caller has seen its message applied (`tasks::applied`), or the run would still
/// read `running` here and get the previous reply.
fn wait_reply(s: &Store, tid: &str, rid: &str) -> Out {
    let top = scope::top(&s.root);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // No server, so no answer is coming: return rather than wait forever.
        if !tasks::serving(&top) {
            return err("E7003", "the board server stopped while waiting");
        }
        let t = task(&s.root, tid)?;
        let i = run_at(&t, rid)?;
        if t["runs"][i]["state"] == "ended" {
            return err("E7003", "run ended while waiting");
        }
        if t["runs"][i]["state"] == "running" && t["paused_from"].is_null() {
            return Ok(format!("{}\n", t["runs"][i]["reply"]));
        }
    }
}
/// Holds a discussion's reply until the human writes again or ends it. Waiting
/// doesn't count toward the task timeout.
fn wait_discussion(s: &Store, tid: &str, rid: &str) -> Out {
    let top = scope::top(&s.root);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // No server, so the human can't reply: return rather than wait forever.
        if !tasks::serving(&top) {
            return err("E7003", "the board server stopped while waiting");
        }
        let t = task(&s.root, tid)?;
        let i = run_at(&t, rid)?;
        if t["runs"][i]["state"] == "ended" {
            return Ok("The discussion has ended. Stop now, without further calls.\n".into());
        }
        let notes = t["notes"].as_array().unwrap();
        let after = notes.iter().rposition(|n| n["run"] == rid && n["author"] == "agent").map_or(0, |l| l + 1);
        let new: Vec<&str> =
            notes[after..].iter().filter(|n| n["run"] == rid && n["author"] == "human").filter_map(|n| n["text"].as_str()).collect();
        if !new.is_empty() && t["paused_from"].is_null() {
            let reply = format!("The human replied:\n\n{}\n", new.join("\n\n"));
            tasks::send(&scope::top(&s.root), json!({"kind":"resume","task":tid,"run":rid}))?;
            return Ok(reply);
        }
    }
}

// Strategy runs (D-93) are in `strategy.rs`, a module of this one, so they
// share its helpers.
#[path = "strategy.rs"]
mod strategy;
use strategy::start_strategy;
#[cfg(test)]
use strategy::{strategy_brief, strategy_list, strategy_note_show, strategy_run_show, strategy_show};
pub use strategy::{strategy_command, strategy_guard};
/// A work run's `write solace.toml` or `write AGENTS.md`: checked now, written
/// only when a human approves it on the board (D-38, D-45).
/// `project` is the folder whose settings change: the home, or another project in the task (D-61).
#[allow(clippy::too_many_arguments)]
pub fn propose_settings(
    s: &Store,
    project: &Path,
    tid: &str,
    rid: &str,
    file: &str,
    base: &[u8],
    proposed: &str,
    reason: Option<&str>,
    guard: Vec<fs::File>,
) -> Out {
    if file == "solace.toml" {
        let v = lockdown::parse_settings(proposed)?;
        workflow(&v)?;
        // `[build] after` names projects of the workspace (D-131).
        let top = scope::top(project);
        recipe::check_after(&top, &scope::dir(&top, project), &v)?;
        // Which projects share the board is the humans' choice (D-57).
        let projects = |text: &[u8]| std::str::from_utf8(text).ok()?.parse::<toml::Value>().ok()?.get("projects").cloned();
        if projects(base) != projects(proposed.as_bytes()) {
            return Err(Failure(
                "E6020 `projects` lists the projects on this root's board; only humans change it, from the board".into(),
                1,
            ));
        }
    }
    let base = std::str::from_utf8(base).map_err(|_| Failure(format!("E6020 {file} is not UTF-8"), 1))?;
    if base == proposed {
        return Ok(format!("{file} is unchanged.\n"));
    }
    let t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let top = scope::top(&s.root);
    let dir = scope::dir(&top, project);
    // The file's path from the root folder, so a board of several projects says which one.
    let name = top.file_name().map(Path::new).unwrap_or(Path::new("")).join(project.strip_prefix(&top).unwrap_or(Path::new(""))).join(file);
    let name = name.to_string_lossy();
    let why = reason.map(str::trim).filter(|r| !r.is_empty()).map(|r| format!("\n\n{r}")).unwrap_or_default();
    let question = json!({"id":unique_id(),"text":format!("Change {name}{why}"),"options":["Approve","Reject"],
        "settings":{"file":file,"project":dir,"base":base,"proposed":proposed}});
    let m = json!({"kind":"settings","task":tid,"run":rid,"question":question});
    message(&w, &mut t.clone(), &m)?;
    let sent = tasks::send(&top, m)?;
    drop(guard);
    if !tasks::applied(&top, &sent) {
        return Ok(format!("{}\n", tasks::QUEUED));
    }
    if tasks::rejected(&top, &sent) {
        return err("E7001", REJECTED);
    }
    wait_reply(s, tid, rid)
}
/// What an agent is told when the task server set its message aside.
const REJECTED: &str = "the board could not apply this message; it was set aside in .solace/tasks/inbox/failed, so nothing was recorded";
/// Applies an approved settings question, unless the file changed since.
fn apply_settings(root: &Path, q: &mut Value) -> Result<(), Failure> {
    let file = q["settings"]["file"].as_str().unwrap_or("solace.toml").to_string();
    let p = match q["settings"]["project"].as_str() {
        Some(dir) => scope::path(&scope::top(root), dir).join(&file),
        None => root.join(&file),
    };
    let current = fs::read_to_string(&p).unwrap_or_default();
    let proposed = q["settings"]["proposed"].as_str().unwrap();
    if current != q["settings"]["base"] || (file == "solace.toml" && lockdown::parse_settings(proposed).and_then(|v| workflow(&v)).is_err())
    {
        q["answer"] = json!(format!("Not applied: {file} changed since the proposal; read it and propose again"));
        return Ok(());
    }
    // A Cargo.toml dependency change needs a fetch outside the sandbox, and a
    // machine without cargo, or with Rust turned off, can't fill the cache (D-158 point 4).
    let cargo = file.ends_with("Cargo.toml") && lockdown::cargo_settings_changed(current.as_bytes(), proposed);
    if cargo && let Some(why) = crate::language::reason(crate::language::RUST)? {
        q["answer"] = json!(format!("Not applied: E6013 {why}"));
        return Ok(());
    }
    let existed = p.exists();
    fs::create_dir_all(p.parent().unwrap_or(root))?;
    fs::write(&p, proposed)?;
    // A failed fetch undoes the approval, so the project stays as it was.
    if cargo {
        let dir = match q["settings"]["project"].as_str() {
            Some(d) => scope::path(&scope::top(root), d),
            None => root.to_path_buf(),
        };
        if let Err(e) = crate::language::cargo::fetch(&dir) {
            let _ = if existed { fs::write(&p, &current) } else { fs::remove_file(&p) };
            q["answer"] = json!(format!("Not applied: {}", e.0.trim_end()));
            return Ok(());
        }
    }
    q["answer"] = json!("Approved and written");
    Ok(())
}

/// Ends runs a previous server left going, as after a restart.
fn recover_project(root: &Path) -> Result<(), Failure> {
    let guard = lock(root)?;
    let s = open(root)?;
    let w = flow(root)?;
    lockdown::recover(root, &s, std::slice::from_ref(&guard))?;
    event(&s, json!({"workflow":w}))?;
    for mut t in tasks(root)? {
        if active(&t) {
            let work = t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended" && r["kind"] == "work");
            end_runs(&mut t, "server restarted");
            if work {
                hold(&mut t, "the Solace server restarted while an agent was working");
                on_event(&w, &mut t, "failed")?;
            } else {
                flag(&mut t, "the Solace server restarted while an agent was working");
            }
            tasks::put(&s, &t)?;
        }
    }
    Ok(())
}

/// What one `solace serve` shows: the projects of one Solace root (D-57),
/// each held by this server alone. The root's own project (`.`) answers at
/// `/`; the others under `/p/<dir>`.
struct Board {
    top: PathBuf,
    held: std::sync::Mutex<HashMap<PathBuf, fs::File>>,
    /// Serializes changes to the root's project list.
    editing: std::sync::Mutex<()>,
    port: u16,
    /// The migration step running now, and the last one's failure (D-59).
    step: std::sync::Mutex<Option<String>>,
    failed: std::sync::Mutex<Option<String>>,
}
impl Board {
    /// Whether the root lists projects (or is a folder of them), so the board
    /// shows lanes; otherwise it is one project on its own.
    fn explicit(&self) -> bool {
        workspace::is_root(&self.top) || !self.top.join(".solace/store.db").exists()
    }
    /// `(dir, root)` for each set-up project.
    fn projects(&self) -> Result<Vec<(String, PathBuf)>, Failure> {
        Ok(workspace::members(&self.top)?
            .into_iter()
            .map(|d| {
                let r = scope::path(&self.top, &d);
                (d, r)
            })
            .filter(|(_, r)| r.join(".solace/store.db").exists())
            .collect())
    }
    /// Holds the project's serve lock for this server's life: a second
    /// server would end the first one's runs as a restart.
    fn claim(&self, dir: &str, root: &Path) -> Result<(), Failure> {
        let mut held = self.held.lock().unwrap();
        if held.contains_key(root) {
            return Ok(());
        }
        let Some(f) = try_lock_file(&root.join(".solace/serve.lock"))? else {
            let name = if dir == "." { root.display().to_string() } else { dir.to_string() };
            return fail(format!("{name} is already served by another `solace serve`; stop that one first."));
        };
        record_port(&f, self.port);
        adopt_tasks(&scope::top(root), root)?;
        recover_project(root)?;
        held.insert(root.to_path_buf(), f);
        Ok(())
    }
    /// The project a request is for, and the path within it; `None` for the
    /// root's own routes.
    fn route(&self, url: &str) -> Result<Option<(PathBuf, String)>, Failure> {
        let projects = self.projects()?;
        let Some(after) = url.strip_prefix("/p/") else {
            return Ok(projects.into_iter().find(|(d, _)| d == ".").map(|(_, r)| (r, url.to_string())));
        };
        let end = after.find(['/', '?']).unwrap_or(after.len());
        let dir = view::percent_decode(&after[..end]);
        let Some((dir, root)) = projects.into_iter().find(|(d, _)| *d == dir && d != ".") else {
            return err("E7001", format!("unknown project {dir}"));
        };
        self.claim(&dir, &root)?;
        let rest = &after[end..];
        Ok(Some((root, if rest.starts_with('/') { rest.to_string() } else { format!("/{rest}") })))
    }
    fn listing(&self) -> Result<Value, Failure> {
        let projects: Vec<Value> = self
            .projects()?
            .into_iter()
            .map(|(dir, root)| {
                let (base, label) = if dir == "." {
                    (String::new(), self.top.file_name().map_or(".".into(), |n| n.to_string_lossy().to_string()))
                } else {
                    (format!("/p/{}", percent_encode(&dir)), dir.clone())
                };
                let needs = match crate::language::id(&root) {
                    Some(id) => match crate::language::reason(id) {
                        Ok(Some(reason)) => reason,
                        Ok(None) if id == crate::language::RUST => fs::read_to_string(root.join(".solace/fetch-failed"))
                            .ok()
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .unwrap_or_default(),
                        Ok(None) => String::new(),
                        Err(e) => e.0,
                    },
                    None => String::new(),
                };
                json!({"dir":dir,"label":label,"name":workspace::name(&root),"target":workspace::target(&root),"base":base,"needs":needs})
            })
            .collect();
        Ok(json!({"workspace":self.explicit(),"projects":projects,"found":workspace::found(&self.top)?}))
    }
    /// `/migration` (D-59, Spec 17): the long steps run in the background,
    /// and those that move the served folder restart the server there.
    fn migration(self: &std::sync::Arc<Self>, method: &str, path: &str, body: &Value) -> Result<Value, Failure> {
        let action = match (method, path) {
            ("GET", "/migration") => {
                // While a step runs, the folders may be mid-swap: report only the step.
                let mut v = match self.step.lock().unwrap().clone() {
                    Some(step) => json!({"state":"running","top":self.top,"step":step,"candidates":[],"blockers":[],"undo":false}),
                    None => migrate::status(&self.top)?,
                };
                v["error"] = json!(self.failed.lock().unwrap().clone());
                return Ok(v);
            }
            ("POST", p) => p.trim_start_matches("/migration/").to_string(),
            _ => return err("E7001", "unknown route"),
        };
        let mut step = self.step.lock().unwrap();
        if step.is_some() {
            return err("E7003", "a migration step is already running");
        }
        let projects: Vec<String> =
            body["projects"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
        let smoke: Vec<String> = body["smoke"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::trim))
            .filter(|c| !c.is_empty())
            .map(String::from)
            .collect();
        match action.as_str() {
            "start" if projects.is_empty() => return err("E7001", "projects required"),
            "start" | "confirm" | "recheck" | "adopt" | "abandon" | "undo" => {}
            _ => return err("E7001", "unknown route"),
        }
        *step = Some("Starting".into());
        *self.failed.lock().unwrap() = None;
        drop(step);
        let me = self.clone();
        std::thread::spawn(move || {
            let report = |s: &str| *me.step.lock().unwrap() = Some(s.to_string());
            let copy = migrate::locate(&me.top);
            let moved = match action.as_str() {
                // A migration that stops partway keeps its copy: serve it, to show why.
                "start" => migrate::start(&me.top, &projects, &smoke, false, &report)
                    .map(Some)
                    .or_else(|e| if migrate::load(&copy).is_some() && me.top != copy { Ok(Some(copy.clone())) } else { Err(e) }),
                "confirm" => migrate::confirm_paths(&copy, &report).map(Some),
                "recheck" => migrate::recheck(&copy, &report).map(|_| None),
                "adopt" => migrate::adopt(&copy, &report).map(Some),
                "abandon" => migrate::abandon(&copy).map(|(original, _)| Some(original)),
                _ => migrate::undo(&me.top).map(Some),
            };
            match moved {
                Ok(Some(at)) => {
                    // /proc/self/exe still runs this binary after a reinstall has replaced its file.
                    let exe = Path::new("/proc/self/exe");
                    let exe = if exe.exists() { exe.to_path_buf() } else { env::current_exe().unwrap_or_else(|_| "solace".into()) };
                    let e = Command::new(exe).args(["serve", "--port", &me.port.to_string()]).current_dir(&at).exec();
                    *me.failed.lock().unwrap() = Some(format!("could not restart the server in {}: {e}", at.display()));
                }
                Ok(None) => {}
                Err(Failure(e, _)) => *me.failed.lock().unwrap() = Some(e),
            }
            *me.step.lock().unwrap() = None;
        });
        Ok(json!({"state":"running"}))
    }
    /// `/projects`: list, add, create and remove a workspace's projects.
    fn projects_api(self: &std::sync::Arc<Self>, method: &str, url: &str, body: &Value) -> Result<Value, Failure> {
        let path = url.split('?').next().unwrap_or("");
        match (method, path) {
            ("GET", "/projects") => return self.listing(),
            ("GET", "/languages") => return languages(),
            ("POST", "/languages") => return setup::save_language(&config_path(), body).and_then(|_| languages()),
            ("GET", "/models") => return models(),
            ("POST", "/models/try-again") => return agents::try_again(body).and_then(|_| models()),
            ("POST", "/models") => return setup::save(&config_path(), &setup::keys_path(), body).and_then(|_| models()),
            ("DELETE", p) if p.starts_with("/models/") => {
                let id = view::percent_decode(&p["/models/".len()..]);
                return setup::remove(&config_path(), &setup::keys_path(), &id).and_then(|_| models());
            }
            (_, p) if p == "/migration" || p.starts_with("/migration/") => return self.migration(method, p, body),
            _ => {}
        }
        let _edit = self.editing.lock().unwrap();
        let field =
            |k: &str| body[k].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| Failure(format!("E7001 {k} required"), 1));
        let dir = match (method, path) {
            ("POST", "/projects") => {
                let dir = field("dir")?;
                workspace::add(&self.top, dir)?;
                dir.to_string()
            }
            ("POST", "/projects/new") => {
                let name = field("name")?;
                let dir = body["dir"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(name);
                workspace::new_project(&self.top, dir, name, body["target"] == "javascript")?;
                dir.to_string()
            }
            ("DELETE", p) if p.starts_with("/projects/") => {
                let dir = view::percent_decode(&p["/projects/".len()..]);
                let root = scope::path(&self.top, &dir);
                // Tasks stored elsewhere may cover this project too (D-61).
                let covering = scope::tasks_in(&self.top, &dir)?;
                if covering.iter().any(|(_, t)| active(t)) {
                    return err("E7003", format!("an agent is working in {dir}; let it finish or stop it first"));
                }
                let w = flow(&self.top)?;
                if let Some((home, t)) = covering.iter().find(|(home, t)| *home != root && column(&w, t)["final"] != true) {
                    return err(
                        "E7013",
                        format!(
                            "the task \"{}\" in {} also covers {dir}; take {dir} out of its projects first",
                            t["title"].as_str().unwrap_or(""),
                            scope::dir(&self.top, home)
                        ),
                    );
                }
                // Deleting the files (D-74) is checked before anything changes.
                let doomed = if body["delete"] == true {
                    Some(workspace::deletable(&self.top, &dir, body["name"].as_str().unwrap_or("").trim())?)
                } else {
                    None
                };
                // Unlist first, so a failed delete still leaves a consistent list.
                workspace::remove(&self.top, &dir)?;
                self.held.lock().unwrap().remove(&root);
                if let Some(path) = doomed {
                    fs::remove_dir_all(path)?;
                }
                return self.listing();
            }
            _ => return err("E7001", "unknown route"),
        };
        let root = scope::path(&self.top, &dir);
        if let Err(e) = self.claim(&dir, &root) {
            if dir != "." {
                workspace::remove(&self.top, &dir)?;
            }
            return Err(e);
        }
        self.listing()
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|c| if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") })
        .collect()
}

/// Whether a request names this server's own address: `Host` (when sent) and
/// `Origin` (when sent) are `127.0.0.1` or `localhost` with the server's port.
fn own_address(req: &tiny_http::Request, port: u16) -> bool {
    let named = |field: &str, prefix: &str| {
        req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(field)).is_none_or(|h| {
            let v = h.value.as_str().to_ascii_lowercase();
            v.strip_prefix(prefix).is_some_and(|a| a == format!("127.0.0.1:{port}") || a == format!("localhost:{port}"))
        })
    };
    named("Host", "") && named("Origin", "http://")
}

/// The largest request body the API reads, besides image uploads.
const BODY_LIMIT: usize = 1_048_576;
/// A request's body, or `None` when it's over `limit` bytes or can't be read.
fn read_body(req: &mut tiny_http::Request, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let read = req.as_reader().take(limit as u64 + 1).read_to_end(&mut bytes);
    (read.is_ok() && bytes.len() <= limit).then_some(bytes)
}
/// A JSON request body; an empty one is `{}`.
fn body_json(bytes: &[u8]) -> Result<Value, Failure> {
    if bytes.is_empty() { Ok(json!({})) } else { serde_json::from_slice(bytes).or_else(|_| err("E7001", "invalid JSON")) }
}
/// An API error as the board reads it: its code and message.
fn error_json(e: &Failure) -> Value {
    json!({"code":e.0.split_whitespace().next().unwrap_or("E7001"),"message":e.0})
}
fn respond_json(req: tiny_http::Request, result: Result<Value, Failure>) {
    match result {
        Ok(v) => respond(req, 200, v),
        Err(e) => respond(req, 409, error_json(&e)),
    }
}
fn respond(req: tiny_http::Request, status: u16, value: Value) {
    let _ = req.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
}

pub fn serve(top: PathBuf, port: u16) -> Out {
    if let Some(done) = migrate::recover_here(&top)? {
        return Ok(format!("{done}Run `solace serve` again from there.\n"));
    }
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| Failure(e.to_string(), 1))?;
    let port = server.server_addr().to_ip().map_or(port, |a| a.port());
    let board = std::sync::Arc::new(Board {
        top,
        held: Default::default(),
        editing: Default::default(),
        port,
        step: Default::default(),
        failed: Default::default(),
    });
    for (dir, root) in board.projects()? {
        board.claim(&dir, &root)?;
    }
    // Once every project's tasks are in the root's store, the task server
    // applies what its inbox holds and takes over (D-77).
    tasks::start(&board.top)?;
    tasks::end_strategy_runs(&board.top)?;
    // Tasks stored before D-134 in Backlog under an active feature move now.
    tasks::plan_active(&board.top)?;
    // Ask each agent which models it offers, so the list is ready when asked.
    std::thread::spawn(agents::refresh_all);
    // A migration's folder is held too, so a shell can't move it from under the board.
    let _migration = match migrate::load(&board.top) {
        Some(_) if !board.held.lock().unwrap().contains_key(&board.top) => {
            let Some(f) = try_lock_file(&board.top.join(".solace/serve.lock"))? else {
                return fail(format!("{} is already served by another `solace serve`; stop that one first.", board.top.display()));
            };
            record_port(&f, board.port);
            Some(f)
        }
        _ => None,
    };
    println!("Solace listening on http://{}", server.server_addr());
    if board.explicit() {
        println!("Solace root {}: {} projects", board.top.display(), board.projects()?.len());
    }
    std::io::stdout().flush()?;
    let watcher = board.clone();
    std::thread::spawn(move || {
        let mut previous: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
        let mut tick = 0u64;
        loop {
            tick += 1;
            std::thread::sleep(Duration::from_millis(100));
            if watcher.step.lock().unwrap().is_some() {
                continue;
            }
            for (_, root) in watcher.projects().unwrap_or_default() {
                // About once a second, read in code changed outside Solace (D-58).
                if tick % 10 == 1
                    && let Some(_guard) = try_lock(&root)
                    // an extract's half-written files are not outside changes
                    && !extract::ahead::waiting(&root)
                    && let Ok(s) = open(&root)
                {
                    let _ = outside::absorb(&s, outside::Mode::Notice);
                }
                let current = fs::read(root.join("solace.toml")).ok();
                match previous.insert(root.clone(), current.clone()) {
                    Some(old) if old != current => {}
                    _ => continue,
                }
                if let Ok(_guard) = lock(&root)
                    && let Ok(s) = open(&root)
                    && let Ok(w) = lockdown::settings(&root).and_then(|v| workflow(&v))
                {
                    let _ = event(&s, json!({"workflow":w}));
                }
            }
        }
    });
    for mut req in server.incoming_requests() {
        let board = board.clone();
        std::thread::spawn(move || {
            let full = req.url().to_string();
            let path = full.split('?').next().unwrap_or("");
            // Only the server's own localhost address may ask (D-80): a page on
            // another site, or a rebound name, is refused before any route.
            if !own_address(&req, board.port) {
                return respond(req, 403, error_json(&Failure("E7016 request is not from this server's own address".into(), 1)));
            }
            // The root's own routes; `/events` merges every project's when it has several.
            let own = path == "/projects"
                || path == "/languages"
                || path == "/models"
                || path.starts_with("/projects/")
                || path == "/migration"
                || path.starts_with("/migration/")
                || (path == "/events" && board.explicit());
            if req.method().as_str() != "GET" && !path.starts_with("/migration") && board.step.lock().unwrap().is_some() {
                return respond_json(req, err("E7003", "a migration step is running"));
            }
            // Milestones, features (D-87), the vision (D-91) and strategy runs (D-93)
            // are the root's: never under `/p/{dir}`.
            let strategy: Vec<&str> = path.trim_matches('/').split('/').collect();
            // A task by id, whichever project stores it (D-98): a task link asks where to open it.
            if let ("GET", ["tasks", "find", tid]) = (req.method().as_str(), strategy.as_slice()) {
                return match tasks::find_task(&board.top, tid) {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            // A feature or milestone by id (D-122): a link to it asks for its title.
            if let ("GET", [kind @ ("features" | "milestones"), "find", id]) = (req.method().as_str(), strategy.as_slice()) {
                return match tasks::find_record(&board.top, kind, id) {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            // Every project's build recipe, in build order (D-131).
            if let ("GET", ["build", "plan"]) = (req.method().as_str(), strategy.as_slice()) {
                return match recipe::plan_json(&board.top) {
                    Ok(v) => respond(req, 200, v),
                    Err(e) => respond(req, 409, error_json(&e)),
                };
            }
            let runs = strategy.len() >= 2 && strategy[..2] == ["strategy", "runs"];
            if matches!(strategy[0], "milestones" | "features") || strategy == ["vision"] || runs {
                let result = match read_body(&mut req, BODY_LIMIT) {
                    Some(bytes) => body_json(&bytes).map_err(|e| (409, e)).and_then(|v| match strategy[0] {
                        "vision" => tasks::vision_api(&board.top, req.method().as_str(), v),
                        "strategy" => {
                            let started = req.method().as_str() == "POST" && strategy.len() == 2;
                            tasks::strategy_runs_api(&board.top, req.method().as_str(), &strategy[2..], v).inspect(|run| {
                                // A new run's agent starts now.
                                if started {
                                    start_strategy(board.top.clone(), run["id"].as_str().unwrap_or("").to_string());
                                }
                            })
                        }
                        _ => tasks::strategy_api(&board.top, req.method().as_str(), &strategy, v),
                    }),
                    None => err("E7001", "request body too large").map_err(|e| (409, e)),
                };
                return match result {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            let routed = if own { Ok(None) } else { board.route(&full) };
            let (root, url) = match routed {
                Err(e) => return respond_json(req, Err(e)),
                Ok(Some((root, url))) => (Some(root), url),
                Ok(None) => (None, full.clone()),
            };
            if url.split('?').next() == Some("/events") {
                let last = req.headers().iter().find(|h| h.field.equiv("Last-Event-ID")).map(|h| h.value.as_str().to_string());
                let mut reader = Events::new(board.clone(), root.clone(), last.as_deref());
                let mut writer = req.into_writer();
                if writer
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                    )
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    return;
                }
                let mut buf = [0; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || writer.write_all(&buf[..n]).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
                return;
            }
            // A link's address, `/task/{id}` (D-98), `/feature/{id}` or `/milestone/{id}`
            // (D-122), is the shell's to resolve.
            let page = url.split('?').next().unwrap_or("");
            let record_link = ["/task/", "/feature/", "/milestone/"].iter().any(|p| page.strip_prefix(p).is_some_and(valid_id));
            // The guide is the shell's Knowledgebase page (D-140): its old address sends the browser there.
            if req.method().as_str() == "GET" && page == "/guide" {
                let _ = req.respond(
                    tiny_http::Response::empty(301).with_header(tiny_http::Header::from_bytes("Location", "/knowledgebase").unwrap()),
                );
                return;
            }
            if req.method().as_str() == "GET"
                && (record_link
                    || matches!(
                        page,
                        "/" | "/workspace"
                            | "/migrate"
                            | "/contracts"
                            | "/strategy"
                            | "/settings"
                            | "/knowledgebase"
                            | "/app.js"
                            | "/fonts/sans-400.woff2"
                            | "/fonts/sans-700.woff2"
                            | "/fonts/mono-400.woff2"
                    ))
            {
                let (data, kind) = match page {
                    _ if record_link => (include_bytes!("../frontend/index.html").as_slice(), "text/html"),
                    "/" | "/workspace" | "/migrate" | "/contracts" | "/strategy" | "/settings" | "/knowledgebase" => {
                        (include_bytes!("../frontend/index.html").as_slice(), "text/html")
                    }
                    "/fonts/sans-400.woff2" => {
                        (include_bytes!("../board/priv/fonts/ibm-plex-sans-latin-400-normal.woff2").as_slice(), "font/woff2")
                    }
                    "/fonts/sans-700.woff2" => {
                        (include_bytes!("../board/priv/fonts/ibm-plex-sans-latin-700-normal.woff2").as_slice(), "font/woff2")
                    }
                    "/fonts/mono-400.woff2" => {
                        (include_bytes!("../board/priv/fonts/ibm-plex-mono-latin-400-normal.woff2").as_slice(), "font/woff2")
                    }
                    _ => (include_bytes!("../frontend/priv/app.js").as_slice(), "text/javascript"),
                };
                let _ = req.respond(
                    tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                );
                return;
            }
            let Some(root) = root else {
                let result = match read_body(&mut req, BODY_LIMIT) {
                    Some(bytes) => body_json(&bytes).and_then(|v| board.projects_api(req.method().as_str(), &url, &v)),
                    None => err("E7001", "request body too large"),
                };
                return respond_json(req, result);
            };
            let parts: Vec<String> = url.split('?').next().unwrap_or("").trim_matches('/').split('/').map(String::from).collect();
            let file = match parts.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["tasks", tid, "files", rest @ ..] => Some((tid.to_string(), rest.join("/"))),
                _ => None,
            };
            if let (Some((tid, name)), "GET") = (&file, req.method().as_str()) {
                if let Ok(data) = image_path(&root, tid, name).and_then(|p| Ok(fs::read(p)?)) {
                    let kind = image_type(name).unwrap();
                    let _ = req.respond(
                        tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                    );
                } else {
                    let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                }
                return;
            }
            let upload_to = file.filter(|(_, name)| name.is_empty() && req.method().as_str() == "POST").map(|(tid, _)| tid);
            let limit = if upload_to.is_some() { IMAGE_LIMIT } else { BODY_LIMIT };
            let result = match (read_body(&mut req, limit), upload_to) {
                (None, upload_to) => err("E7001", if upload_to.is_some() { "image over 5 MB" } else { "request body too large" }),
                (Some(bytes), Some(tid)) => upload(&root, &tid, &bytes),
                (Some(bytes), None) => body_json(&bytes).and_then(|v| api(&root, req.method().as_str(), &url, v)),
            };
            match result {
                Ok(v) => respond(req, 200, v),
                Err(e) => {
                    let mut v = error_json(&e);
                    // A refused action says which actions the task allows now.
                    if v["code"] == "E7001" && url.ends_with("/actions") {
                        let id = url.split('/').nth(2).unwrap_or("");
                        if let (Ok(t), Ok(w)) = (task(&root, id), flow(&root)) {
                            v["allowed"] = json!(allowed(&w, &t));
                        }
                    }
                    respond(req, 409, v)
                }
            }
        });
    }
    Ok(String::new())
}

/// The `/events` stream: one project's events, or in a workspace's merged
/// stream every project's, each naming its `project`, with a `{projects}`
/// event when the list changes. A merged stream's event id holds each
/// project's position as JSON (D-57). A new stream starts from each project's
/// workflow and tasks as they are, then follows the changes (D-78).
struct Events {
    /// Whether the snapshot a new stream starts with is still to be sent.
    fresh: bool,
    board: std::sync::Arc<Board>,
    merged: bool,
    /// The position in the task log, shared by every project (D-77).
    tasks: i64,
    /// `(dir, root, seq)`: each project's position in its workflow events.
    streams: Vec<(String, PathBuf, i64)>,
    /// Whether this connection has sent `ready`, the end of what it missed.
    caught_up: bool,
    bytes: std::io::Cursor<Vec<u8>>,
}
impl Events {
    /// An event id is `{"tasks": n, dir: seq, ...}`; any other starts from the beginning.
    fn new(board: std::sync::Arc<Board>, root: Option<PathBuf>, last: Option<&str>) -> Events {
        let seen: HashMap<String, i64> = last.and_then(|l| serde_json::from_str(l).ok()).unwrap_or_default();
        let at = |d: &str| seen.get(d).copied().unwrap_or(0);
        let (merged, streams) = match root {
            Some(root) => {
                let d = scope::dir(&board.top, &root);
                (false, vec![(d.clone(), root, at(&d))])
            }
            None => (true, board.projects().unwrap_or_default().into_iter().map(|(d, r)| (d.clone(), r, at(&d))).collect()),
        };
        Events { fresh: seen.is_empty(), board, merged, tasks: at("tasks"), streams, caught_up: false, bytes: Default::default() }
    }
    /// What a new stream starts from: the log positions first, then the files,
    /// so a change made in between is seen again as an event, never missed.
    fn snapshot(&mut self) -> std::io::Result<String> {
        let top = self.board.top.clone();
        self.tasks = if store_dir(&top).is_dir() {
            let db = tasks::events(&top).map_err(|e| std::io::Error::other(e.0))?;
            db.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0)).map_err(std::io::Error::other)?
        } else {
            0
        };
        for stream in &mut self.streams {
            stream.2 = Connection::open(stream.1.join(".solace/store.db"))
                .and_then(|db| {
                    db.busy_timeout(Duration::from_secs(10))?;
                    db.query_row("SELECT COALESCE(MAX(seq),0) FROM board_events", [], |row| row.get(0))
                })
                .unwrap_or(0);
        }
        Ok(self.state_of(&(0..self.streams.len()).collect::<Vec<_>>()) + &self.strategy_state())
    }
    /// Whether this stream carries the root's own records: the merged stream, or
    /// the root project's (D-87 point 7), never `/p/{dir}/events`.
    fn at_root(&self) -> bool {
        self.merged || self.streams.iter().any(|s| s.0 == ".")
    }
    /// The root's milestones, features, vision and strategy runs as they are,
    /// after the tasks.
    fn strategy_state(&self) -> String {
        if !self.at_root() {
            return String::new();
        }
        let (id, top) = (self.id(), &self.board.top);
        let (milestones, features) =
            (json!({"milestones":tasks::records(top, "milestones")}), json!({"features":tasks::records(top, "features")}));
        let vision = json!({"vision": tasks::vision(top)});
        let runs = json!({"strategyRuns": tasks::strategy_runs(top)});
        // Tasks with no project (D-129) ride on the root's streams, marked so
        // the Strategy page can list them whatever their home.
        let homeless = json!({"tasks": all_tasks(top).unwrap_or_default().into_iter().filter(|t| t["noProject"] == true).collect::<Vec<_>>(), "noProject": true});
        format!(
            "id: {id}\ndata: {milestones}\n\nid: {id}\ndata: {features}\n\nid: {id}\ndata: {vision}\n\nid: {id}\ndata: {runs}\n\nid: {id}\ndata: {homeless}\n\n"
        )
    }
    /// The workflow and tasks of the streams at `which`, as events.
    fn state_of(&self, which: &[usize]) -> String {
        let (id, merged) = (self.id(), self.merged);
        let mut out = String::new();
        for &i in which {
            let (dir, root, _) = &self.streams[i];
            let mut add = |mut v: Value| {
                if merged {
                    v["project"] = json!(dir);
                }
                out += &format!("id: {id}\ndata: {v}\n\n");
            };
            if let Ok(w) = flow(root) {
                add(json!({"workflow":w}));
            }
            if let Ok(ts) = listed(root) {
                add(json!({"tasks":ts}));
            }
        }
        out
    }
    fn id(&self) -> String {
        let mut id: serde_json::Map<_, _> = self.streams.iter().map(|(d, _, s)| (d.clone(), json!(s))).collect();
        id.insert("tasks".into(), json!(self.tasks));
        json!(id).to_string()
    }
    /// The next task event this stream shows, past those of other projects.
    fn next_task(&mut self) -> std::io::Result<Option<String>> {
        if !store_dir(&self.board.top).is_dir() {
            return Ok(None);
        }
        let db = tasks::events(&self.board.top).map_err(|e| std::io::Error::other(e.0))?;
        let members = workspace::members(&self.board.top);
        loop {
            let row = db
                .query_row("SELECT seq,project,data FROM events WHERE seq>? ORDER BY seq LIMIT 1", [self.tasks], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })
                .optional()
                .map_err(std::io::Error::other)?;
            let Some((seq, project, data)) = row else { return Ok(None) };
            self.tasks = seq;
            // A row with no project is a root record's (D-87); a row whose
            // project is no member is a task with no project (D-129), shown
            // on the root's streams and marked.
            let homeless = !project.is_empty() && members.as_ref().is_ok_and(|ms| !ms.iter().any(|m| m == &project));
            let shown = if project.is_empty() {
                self.at_root()
            } else {
                self.streams.iter().any(|s| s.0 == project) || (self.at_root() && homeless)
            };
            if !shown {
                continue;
            }
            let mut v: Value = serde_json::from_str(&data).map_err(std::io::Error::other)?;
            if self.merged && !project.is_empty() {
                v["project"] = json!(project);
            }
            if homeless {
                v["noProject"] = json!(true);
            }
            return Ok(Some(format!("id: {}\ndata: {v}\n\n", self.id())));
        }
    }
    /// Follows a change to the workspace's list: new projects start at their
    /// latest event, since the board loads them whole.
    fn members_changed(&mut self) -> std::io::Result<Option<String>> {
        let now = self.board.projects().map_err(|e| std::io::Error::other(e.0))?;
        if now.iter().map(|p| &p.0).eq(self.streams.iter().map(|s| &s.0)) {
            return Ok(None);
        }
        let old = std::mem::take(&mut self.streams);
        let mut added = vec![];
        for (d, r) in now {
            let seq = match old.iter().find(|s| s.0 == d) {
                Some(s) => s.2,
                None => {
                    added.push(self.streams.len());
                    Connection::open(r.join(".solace/store.db"))
                        .and_then(|db| {
                            db.busy_timeout(Duration::from_secs(10))?;
                            db.query_row("SELECT COALESCE(MAX(seq),0) FROM board_events", [], |row| row.get(0))
                        })
                        .unwrap_or(0)
                }
            };
            self.streams.push((d, r, seq));
        }
        // The new projects' state follows the list: their positions were read before their files.
        let dirs: Vec<&String> = self.streams.iter().map(|s| &s.0).collect();
        let list = format!("id: {}\ndata: {}\n\n", self.id(), json!({"projects":dirs}));
        Ok(Some(list + &self.strategy_state() + &self.state_of(&added)))
    }
}
impl Read for Events {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let n = self.bytes.read(out)?;
        if n > 0 {
            return Ok(n);
        }
        if self.fresh {
            self.fresh = false;
            let text = self.snapshot()?;
            if !text.is_empty() {
                self.bytes = std::io::Cursor::new(text.into_bytes());
                return self.bytes.read(out);
            }
        }
        for _ in 0..10 {
            if self.merged
                && let Some(text) = self.members_changed()?
            {
                self.bytes = std::io::Cursor::new(text.into_bytes());
                return self.bytes.read(out);
            }
            for i in 0..self.streams.len() {
                let (dir, root, seq) = &self.streams[i];
                // Opened on each poll, not kept: a kept connection would follow a store
                // that adopting or undoing a migration moved away.
                let db = Connection::open(root.join(".solace/store.db")).map_err(std::io::Error::other)?;
                db.busy_timeout(Duration::from_secs(10)).map_err(std::io::Error::other)?;
                let row = db
                    .query_row("SELECT seq,data FROM board_events WHERE seq>? ORDER BY seq LIMIT 1", [*seq], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .optional()
                    .map_err(std::io::Error::other)?;
                if let Some((seq, data)) = row {
                    let dir = dir.clone();
                    self.streams[i].2 = seq;
                    let mut v: Value = serde_json::from_str(&data).map_err(std::io::Error::other)?;
                    if self.merged {
                        v["project"] = json!(dir);
                    }
                    self.bytes = std::io::Cursor::new(format!("id: {}\ndata: {v}\n\n", self.id()).into_bytes());
                    return self.bytes.read(out);
                }
            }
            // Workflow events first, so a replay draws the columns before their tasks.
            if let Some(text) = self.next_task()? {
                self.bytes = std::io::Cursor::new(text.into_bytes());
                return self.bytes.read(out);
            }
            if !self.caught_up {
                self.caught_up = true;
                self.bytes = std::io::Cursor::new(b"event: ready\ndata: {}\n\n".to_vec());
                return self.bytes.read(out);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.bytes = std::io::Cursor::new(b": keepalive\n\n".to_vec());
        self.bytes.read(out)
    }
}

#[cfg(test)]
// The unit tests are in their own file, so this one stays within watch_limit.
#[path = "board_tests.rs"]
mod tests;

/// The `[commands]` each project of the task marks with `test = true`, in
/// project order and sorted by name (D-182). A project with none is left out;
/// a settings file that cannot be read is named, and the other projects' marks
/// are kept.
fn marked_tests(top: &Path, t: &Value) -> (Vec<Value>, Vec<String>) {
    let home = t["home"].as_str().unwrap_or("");
    let mut found = vec![];
    let mut errors = vec![];
    for dir in scope::of(t, home) {
        let settings = match lockdown::settings(&scope::path(top, &dir)) {
            Ok(settings) => settings,
            Err(e) => {
                errors.push(format!("project {dir}: {}", e.0));
                continue;
            }
        };
        let Some(commands) = settings.get("commands").and_then(|c| c.as_table()) else { continue };
        let mut names: Vec<&str> =
            commands.iter().filter(|(_, c)| c.get("test").and_then(|x| x.as_bool()) == Some(true)).map(|(name, _)| name.as_str()).collect();
        names.sort();
        if !names.is_empty() {
            found.push(json!({"project": dir, "commands": names}));
        }
    }
    (found, errors)
}

#[cfg(test)]
thread_local! {
    /// A test's task and run, in place of the agent's environment.
    static TEST_CONTEXT: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `context` reading `tid` and `rid`, for a test that drives a
/// task command without an agent's environment.
#[cfg(test)]
pub(super) fn with_context<T>(tid: &str, rid: &str, f: impl FnOnce() -> T) -> T {
    let old = TEST_CONTEXT.with(|c| c.replace(Some((tid.to_string(), rid.to_string()))));
    let out = f();
    TEST_CONTEXT.with(|c| *c.borrow_mut() = old);
    out
}

/// The task is settled once every project's writes are approved; a busy flush
/// leaves only `store.db` behind, while any other failure keeps its own error.
fn settle_result(pending: Vec<String>, failed: Vec<(String, Failure)>) -> Result<(), Failure> {
    if let Some((_, e)) = failed.first() {
        let reasons: Vec<String> = failed.iter().map(|(dir, e)| format!("{dir}: {}", e.0)).collect();
        let also =
            if pending.is_empty() { String::new() } else { format!("\nAlso, flushing store.db is pending in {}.", pending.join(", ")) };
        return Err(Failure(format!("{}{also}", reasons.join("\n")), e.1));
    }
    if pending.is_empty() {
        return Ok(());
    }
    err("E7028", format!("the task is settled; only flushing store.db is pending in {}", pending.join(", ")))
}

/// Whether the system settings leave language `id` on; an absent setting is on.
pub(crate) fn language_enabled(id: &str) -> Result<bool, Failure> {
    language_enabled_at(&config_path(), id)
}

/// Why language `id` cannot use its tools, if it cannot: turned off first, then
/// a toolchain that is not installed.
pub(crate) fn language_reason(id: &str) -> Result<Option<String>, Failure> {
    if !crate::language::enabled(id)? {
        return Ok(Some(crate::language::off(id).to_string()));
    }
    if !crate::language::installed(id) {
        return Ok(Some(crate::language::missing(id).to_string()));
    }
    Ok(None)
}

/// The system languages and whether each is on and found on this computer.
fn languages() -> Result<Value, Failure> {
    let p = config_path();
    let v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    let mut out = vec![];
    for id in [crate::language::GLEAM, crate::language::RUST] {
        out.push(json!({
            "id": id,
            "label": crate::language::label(id),
            "enabled": v["languages"][id].as_bool().unwrap_or(true),
            "installed": crate::language::installed(id),
        }));
    }
    Ok(json!(out))
}

/// Whether `config` leaves language `id` on; an absent setting is on.
fn language_enabled_at(config: &Path, id: &str) -> Result<bool, Failure> {
    let v = parse_system(&if config.exists() { fs::read_to_string(config)? } else { String::new() })?;
    Ok(v["languages"][id].as_bool().unwrap_or(true))
}

#[cfg(test)]
thread_local! {
    /// A test's system settings file, in place of `SOLACE_CONFIG` and the home folder's.
    static TEST_CONFIG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `config_path` reading `path`, so a test drives settings without
/// changing the process's environment.
#[cfg(test)]
pub(crate) fn with_config<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let old = TEST_CONFIG.with(|p| p.replace(Some(path.to_path_buf())));
    let out = f();
    TEST_CONFIG.with(|p| *p.borrow_mut() = old);
    out
}

/// Marks whether the project or instructions changed since the run's review checkpoint.
fn review_changed(s: &Store, t: &Value, rid: &str) -> Result<bool, Failure> {
    let path = review_path(&s.root, t["id"].as_str().unwrap(), rid)?;
    let Ok(bytes) = fs::read(&path) else {
        return Ok(false);
    };
    let mut checkpoint: Value = serde_json::from_slice(&bytes).map_err(|e| Failure(e.to_string(), 1))?;
    let current = review_state(s, t)?;
    let changed = checkpoint["context"] != current["context"]
        || checkpoint["instructions"] != current["instructions"]
        || checkpoint["projects"] != current["projects"];
    checkpoint["changed_during_review"] = json!(changed);
    write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap())?;
    Ok(changed)
}

/// The agent message a review's feedback travels in, after its checkpoint is marked.
fn review_message(s: &Store, tid: &str, t: &Value, rid: &str, text: &str) -> Result<Value, Failure> {
    let mut m = json!({"kind":"agent","task":tid,"run":rid,"command":"note","text":text});
    m["changed_during_review"] = json!(review_changed(s, t, rid)?);
    Ok(m)
}

/// Whether an agent note for this review run is already on the task.
fn review_feedback_recorded(t: &Value, rid: &str) -> bool {
    t["notes"].as_array().is_some_and(|notes| notes.iter().any(|n| n["author"] == "agent" && n["run"] == rid))
}

/// A review's final message counts only when its first line is exactly an assessment label.
fn review_assessment(reply: &str) -> bool {
    matches!(reply.trim().lines().next().map(str::trim_end), Some("**Assessment: Looks good**" | "**Assessment: Needs work**"))
}

/// A clean review's last message, when nothing already recorded its feedback.
fn recoverable_review(r: &Value, success: bool, entries: &[Value]) -> Option<String> {
    if r["kind"] != "review" || !success || entries.iter().any(|e| e["kind"] == "error") {
        return None;
    }
    let recorded = entries.iter().any(|e| {
        e["kind"] == "tool"
            && e["title"].as_str().is_some_and(|title| title.contains("task note"))
            && e["output"].as_str().is_some_and(|output| output.starts_with("Recorded"))
    });
    if recorded {
        return None;
    }
    let reply = entries.iter().rev().find(|e| e["kind"] == "message").and_then(|e| e["text"].as_str())?;
    review_assessment(reply).then(|| reply.trim().to_string())
}

/// Records a review's recovered assessment through the same path as `task note`.
fn record_review(watch: &Watch, rid: &str, text: &str) -> Result<bool, Failure> {
    let Watch::Task(root, tid) = watch else {
        return Ok(false);
    };
    let s = open(root)?;
    let t = task(root, tid)?;
    let i = run_at(&t, rid)?;
    if t["runs"][i]["kind"] != "review" {
        return Ok(false);
    }
    let m = review_message(&s, tid, &t, rid, text)?;
    let w = flow(root)?;
    let rid = rid.to_string();
    tasks::update(root, tid, move |t| {
        let i = run_at(t, &rid)?;
        if t["runs"][i]["state"] == "ended" || review_feedback_recorded(t, &rid) {
            return Ok(false);
        }
        message(&w, t, &m)?;
        Ok(true)
    })
}

/// Whether the current board run is a work run, which may finish with `task done`.
pub fn work_run(root: &Path) -> bool {
    let Some((tid, rid)) = context() else { return false };
    let Ok(t) = scope::home(root, &tid).and_then(|home| task(&home, &tid)) else { return false };
    run_at(&t, &rid).is_ok_and(|i| t["runs"][i]["kind"].as_str() == Some("work"))
}
