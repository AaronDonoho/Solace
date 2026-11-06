use serde_json::Value;
use serde_json::json;
use std::env;
use std::collections::HashMap;
use std::path::PathBuf;
use std::path::Path;
use std::fs;
use rusqlite::params;
use std::collections::HashSet;
use crate::board::err;
use crate::board::setup::system;
use crate::board::setup::model;
use crate::board::setup::efforts;
use crate::board::reviews::changes_of;
use crate::board::reviews::revert;
use crate::board::reviews::tech_debt;
use crate::board::settings::apply_settings;
use crate::board::runs::text;
use crate::board::agents::models_path;
use crate::board::agents::saved_models;
use crate::board::agents::save_models;
use crate::board::agents::MODELS;
use crate::board::agents::unavailable;
use crate::Store;
use crate::now;
use crate::unique_id;
use crate::failure::Failure;
use crate::failure::fail;
#[cfg(test)]
#[path = "task_model_tests.rs"]
pub(super) mod tests;
use crate::scope;
use crate::workspace;
use crate::jsonx;
use crate::lockdown;
use crate::outside;
use crate::fsx;
use crate::board::reviews::scope_stores;
use crate::board::runs::transcript;

/// Whether `id` can name a task: it names the task's file and folder, so only
/// letters, digits and `-`.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

pub(crate) fn task_path(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    if !valid_id(id) {
        return err("E7001", "invalid task id");
    }
    Ok(store_dir(&scope::top(root)).join(format!("{id}.json")))
}

/// The task's own folder in a project: run logs, conversations, reviews, files.
pub(crate) fn task_dir(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    let _ = task_path(root, id)?;
    Ok(root.join(".solace/board").join(id))
}

/// A task whose home is the project `root`. A task whose home is no project is
/// re-homed on load (D-160); one with no project at all is opened from the
/// root, which stores it, and says `noProject` (D-129).
pub(crate) fn task(root: &Path, id: &str) -> Result<Value, Failure> {
    let top = scope::top(root);
    let mut t = stored_task(&top, id)?;
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
    homeless_among(&workspace::members(top), home)
}

/// The home and scope a task whose home is no project takes (D-160): the first
/// project of its scope, keeping only the projects of it, else the root with
/// scope `["."]` when the root is a project. None when the root is no project:
/// the task keeps no project. None while a run of the task is live: the agent
/// works from the task's folder, and the load must not move it out from under
/// the run (as `set_scope` says for a home change).
pub(crate) fn rehome_to(members: &[String], t: &Value) -> Option<(String, Vec<String>)> {
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
        Ok(ms) if rehome_to(&ms, &t).is_some() => crate::tasks::rehome(top, &t),
        _ => Ok(t),
    }
}

/// The project to open task `t` from: its home, or the root when its home is
/// no project (D-129).
pub(crate) fn task_root(top: &Path, t: &Value) -> PathBuf {
    project_home(top, t).unwrap_or_else(|| top.to_path_buf())
}

/// Every task in the root's store, in rank order.
pub(crate) fn all_tasks(top: &Path) -> Result<Vec<Value>, Failure> {
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
                        Ok(ms) if rehome_to(ms, &t).is_some() => match crate::tasks::rehome(top, &t) {
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

pub(crate) fn read_task(top: &Path, path: &Path) -> Result<Value, Failure> {
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
    let mut t: Value = jsonx::read(path)?;
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
        let feature = features.entry(fid).or_insert_with_key(|fid| crate::tasks::record(top, "features", fid));
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
pub(crate) fn adopt_tasks(top: &Path, root: &Path) -> Result<(), Failure> {
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
        let mut t: Value = jsonx::read(&p)?;
        let to = task_path(root, t["id"].as_str().unwrap_or(""))?;
        if to.exists() {
            // A crash between writing the store copy and removing this one: finish the move.
            let stored: Option<Value> = jsonx::read_opt(&to);
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
        jsonx::write_pretty(&to, &t)?;
        fs::remove_file(&p)?;
    }
    Ok(())
}

pub(crate) fn event(s: &Store, v: Value) -> Result<(), Failure> {
    s.db.execute("INSERT INTO board_events(data) VALUES (?)", [v.to_string()])?;
    Ok(())
}

/// A project's workflow: its root's, shared by every project on the board (D-61).
pub fn flow(project: &Path) -> Result<Value, Failure> {
    workflow(&lockdown::settings(&scope::top(project))?)
}

pub fn workflow(settings: &toml::Value) -> Result<Value, Failure> {
    let default: toml::Value = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/default-board.toml")).parse().unwrap();
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
    let actions = w["actions"].clone();
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
        let role = main_action(c["id"].as_str().unwrap_or("")).and_then(|name| actions.get(name)).map(run_role).unwrap_or("");
        c["role"] = json!(role);
    }
    Ok(w)
}

pub(crate) fn column<'a>(w: &'a Value, t: &Value) -> &'a Value {
    w["columns"].as_array().unwrap().iter().find(|c| c["id"] == t["column"]).unwrap_or(&Value::Null)
}

pub(crate) fn allowed(w: &Value, t: &Value) -> Vec<String> {
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
pub(crate) fn conversation(turns: &[Value], model: &str) -> String {
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

pub(crate) fn active(t: &Value) -> bool {
    t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended")
}

pub(crate) fn note(t: &mut Value, author: &str, run: Option<&str>, text: &str) {
    t["notes"].as_array_mut().unwrap().push(json!({"id":unique_id(),"author":author,"run":run,"time":now(),"text":text}));
}

/// Marks a task as needing a human. The next action a human takes on it clears the mark.
pub(crate) fn flag(t: &mut Value, reason: &str) {
    t["attention"] = json!({"reason":reason,"time":now()});
    t["finished"] = Value::Null;
}

/// Marks a task whose agent is held on a question or stopped before finishing
/// (D-82). The mark stays through other actions until the run continues, is
/// restarted, or the task is stopped or settled; `blocked` is reserved for tasks that depend on other tasks.
pub(crate) fn hold(t: &mut Value, reason: &str) {
    flag(t, reason);
    t["attention"]["hold"] = json!(true);
}

/// Moves the task to the column an event names; an event with no target leaves it where it is (D-82).
pub(crate) fn on_event(w: &Value, t: &mut Value, event: &str) -> Result<(), Failure> {
    match w["events"].get(event) {
        Some(to) if !to.is_null() => move_to(w, t, to),
        _ => Ok(()),
    }
}

pub(crate) fn move_to(w: &Value, t: &mut Value, to: &Value) -> Result<(), Failure> {
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
    set_column(w, t, to);
    Ok(())
}

/// Where run `rid` is in the task's runs.
pub(crate) fn run_at(t: &Value, rid: &str) -> Result<usize, Failure> {
    t["runs"].as_array().and_then(|runs| runs.iter().position(|r| r["id"] == rid)).ok_or_else(|| Failure("E7003 unknown run".into(), 1))
}

pub(crate) fn end_run(r: &mut Value, outcome: &str) {
    r["state"] = json!("ended");
    r["ended"] = json!(now());
    r["outcome"] = json!(outcome);
    // The model the agent's start event names, when it names one: an alias
    // shows the model the run used, in the run and in its label.
    let log = PathBuf::from(r["log"].as_str().unwrap_or(""));
    if let Some(used) = used_model(&log) {
        r["used"] = json!(used);
        let label = system()
            .ok()
            .and_then(|s| s["models"].as_array().map(|models| crate::board::agents::used_label(models, &used)))
            .unwrap_or_else(|| used.clone());
        record_used(r["model"].as_str().unwrap_or(""), &used, &label);
    }
}

/// A run's start time and outcome as the task holds them now: `after_run`
/// clears a usage-limit mark only for a run that succeeded, started before it.
pub(crate) fn run_state(root: &Path, tid: &str, rid: &str) -> (u64, Option<String>) {
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

pub(crate) fn end_runs(t: &mut Value, outcome: &str) {
    for r in t["runs"].as_array_mut().unwrap() {
        if r["state"] != "ended" {
            end_run(r, outcome);
        }
    }
    t["paused_from"] = Value::Null;
}

/// A new task in `column`, with nothing done on it yet. Its changes are computed
/// from the scope projects' stores (D-77 point 4), so the task holds none. Its
/// `createdTime` is set here, once (D-130).
pub fn new_task(title: &str, body: &str, column: &Value, rank: f64) -> Value {
    json!({"id":unique_id(),"title":title,"body":body,"links":[],"column":column,"rank":rank,"paused_from":null,"attention":null,
        "stamps":{},"model":null,"notes":[],"questions":[],"runs":[],
        "feature":null,"milestone":null,"dependsOn":[],"createdTime":now(),"closedTime":null,"archivedTime":null})
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
    if a["revert"] != true
        && column(w, &proposed)["final"] == true
        && let Some(name) = outside::unreadable_change(s, t)?
    {
        return err(
            "E7032",
            format!(
                "{name} was changed outside Solace in a form Solace can't read, so this task can't be completed. \
                 Cancel the task to restore Solace's code, or have an agent redo the change through Solace. \
                 A file whose name isn't a valid module name can only be renamed or canceled."
            ),
        );
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
        let entry = json!({
            "id":rid,
            "kind":kind,
            "model":m["id"],
            "effort":m["effort"],
            "state":"running",
            "started":now(),
            "log":log,
            "fresh":run == "new_work" || a["revert"] == true
        });
        t["runs"].as_array_mut().unwrap().push(entry);
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
    }
    Ok(None)
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
pub(crate) fn plan_for_feature(top: &Path, t: &mut Value, f: &Value) -> bool {
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
pub(crate) fn set_links(top: &Path, t: &mut Value, body: &Value) -> Result<(), Failure> {
    for (k, kind) in [("feature", "features"), ("milestone", "milestones")] {
        match body.get(k) {
            None | Some(Value::Null) => {}
            Some(Value::String(id)) if crate::tasks::record(top, kind, id).is_some() => {}
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
                if let Some(r) = crate::tasks::record(top, "features", id) {
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
pub(crate) fn change(root: &Path, tid: &str, method: &str, rest: &[&str], body: &Value) -> Result<(Value, After), Failure> {
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
                ("POST", ["actions"]) => return act(root, &s, &w, &mut t, body).map(|after| (t, after)),
                _ => return err("E7001", "unknown route"),
            }
        }
    }
    Ok((t, After::Nothing))
}

/// Applies `body`'s action to `t`: launching its run, or moving it.
fn act(root: &Path, s: &Store, w: &Value, t: &mut Value, body: &Value) -> Result<After, Failure> {
    let name = body["action"].as_str().unwrap_or("");
    // A task with no project (D-129) can't run: no project has an agent to
    // work on it. Cancel, archive and editing are the human's own, and need no
    // project.
    if t["noProject"] == true && !w["actions"][name]["run"].is_null() {
        return err("E7003", format!("{name} needs a project; this task has none"));
    }
    let tid = t["id"].as_str().unwrap_or("").to_string();
    let was_final = column(w, t)["final"] == true;
    let launch = action(s, w, t, body)?;
    // A held question or a stopped work run stays flagged until the run
    // continues, restarts, stops or the task settles (D-82).
    let a = &w["actions"][name];
    let resolves = matches!(a["run"].as_str(), Some("work" | "new_work" | "investigate"))
        || a["revert"] == true
        || name == "stop"
        || column(w, t)["final"] == true;
    if t["attention"]["hold"] != true || resolves {
        t["attention"] = Value::Null;
    }
    t["finished"] = Value::Null;
    match launch {
        Some((rid, m)) => Ok(After::Start(rid, m)),
        None if column(w, t)["final"] == true => Ok(completed(root, &tid, t, was_final, a)),
        None => Ok(After::Nothing),
    }
}

/// A completed task's last work summary and review file their tech debt (D-112);
/// its conversation goes.
fn completed(root: &Path, tid: &str, t: &mut Value, was_final: bool, a: &Value) -> After {
    // Completed with its changes kept, not canceled: the last work summary's
    // and review's tech debt is filed.
    let debt = if !was_final && a["revert"] != true {
        tech_debt(&scope::top(root), t).unwrap_or_else(|e| {
            eprintln!("filing tech debt of task {tid}: {}", e.0);
            None
        })
    } else {
        None
    };
    if let Some((d, work, review)) = &debt {
        let n = work + review;
        let items = if n == 1 { "1 tech-debt item".to_string() } else { format!("{n} tech-debt items") };
        let source = match (*work > 0, *review > 0) {
            (true, true) => "the last work run and review",
            (true, false) => "the last work run",
            _ => "the last review",
        };
        note(t, "solace", None, &format!("Filed {items} from {source} on task:{}.", d["id"].as_str().unwrap_or("")));
    }
    After::Clean(debt.map(|(d, _, _)| d))
}

/// The column a task goes in to be planned: `planning` when the workflow has
/// it, else its first column (D-132).
pub(crate) fn planning_column(w: &Value) -> Value {
    if has_column(w, PLANNING) { json!(PLANNING) } else { w["columns"][0]["id"].clone() }
}

/// The task `id` from the root's store, re-homed when its home is no project (D-160).
fn stored_task(top: &Path, id: &str) -> Result<Value, Failure> {
    let path = task_path(top, id)?;
    if !path.exists() {
        return err("E7001", "unknown task");
    }
    rehome_task(top, read_task(top, &path)?)
}

/// Task `id` read at its own home: the root's project or a member project.
/// A task with no project (D-129) opens from the root, marked `noProject`.
pub(crate) fn task_at_home(root: &Path, id: &str) -> Result<Value, Failure> {
    let top = scope::top(root);
    let mut t = stored_task(&top, id)?;
    if project_home(&top, &t).is_none() {
        t["noProject"] = json!(true);
    }
    Ok(t)
}

/// The project that holds task `t`, when its home names one.
pub(crate) fn project_home(top: &Path, t: &Value) -> Option<PathBuf> {
    let home = t["home"].as_str().unwrap_or(".");
    (!homeless(top, home)).then(|| scope::path(top, home))
}

pub(crate) fn homeless_among(members: &Result<Vec<String>, Failure>, home: &str) -> bool {
    members.as_ref().is_ok_and(|ms| !ms.iter().any(|m| m == home))
}

/// One line for a related task (D-110, D-203): `task:<id> <title> (<column>)`,
/// with its project when it is not `home`'s, and `— unfinished` when asked.
pub(crate) fn task_line(t: &Value, home: &Value, w: &Value, unfinished: bool) -> String {
    let project = if t["home"].as_str() != home.as_str() { format!(", in {}", text(&t["home"])) } else { String::new() };
    format!(
        "task:{} {} ({}{}){}",
        text(&t["id"]),
        text(&t["title"]),
        column_name(w, t),
        project,
        if unfinished { " — unfinished" } else { "" }
    )
}

/// A task's column as its label, or its id when the workflow has no label.
pub(crate) fn column_name(w: &Value, t: &Value) -> String {
    let c = column(w, t);
    c["label"].as_str().or(c["id"].as_str()).or(t["column"].as_str()).unwrap_or("").to_string()
}

/// Records that model `model` resolved to `used`, with `label` as the plain
/// text the run showed.
pub(super) fn record_used(model: &str, used: &str, label: &str) {
    let _writer = MODELS.lock().unwrap();
    let path = models_path();
    let mut all = saved_models(&path);
    let record = json!({"model": used, "label": label});
    if all["used"][model] != record {
        all["used"][model] = record;
        if let Err(e) = save_models(&path, &all) {
            eprintln!("solace: could not record the model a run used: {}", e.0);
        }
    }
}

/// What a change to a task leaves to do once the task server has written it.
pub(crate) enum After {
    Nothing,
    /// Launch the run `(rid, model)`.
    Start(String, Value),
    /// The task reached a final column: its conversation goes. A task
    /// completed with its last review's tech debt carries the Tech Debt task to
    /// write with it (D-112).
    Clean(Option<Value>),
    /// The task's home left its scope; the board's lane for this one drops it (D-61).
    Removed(String),
}

/// Where a new task goes: its workflow's first column, or Planning.
#[derive(Clone, Copy)]
pub(crate) enum Column {
    First,
    Planning,
}

pub fn lock(root: &Path) -> Result<fs::File, Failure> {
    fs::create_dir_all(root.join(".solace"))?;
    // one project lock; use per-change store isolation if parallel work is introduced.
    fsx::lock_file(&root.join(".solace/lock"))
}

/// The project lock when it is free right now; the board's poll skips a busy project.
pub(crate) fn try_lock(root: &Path) -> Option<fs::File> {
    fsx::try_lock_file(&root.join(".solace/lock")).ok()?
}

pub fn open(root: &Path) -> Result<Store, Failure> {
    let s = Store::at(root)?;
    fs::create_dir_all(root.join(".solace/board"))?;
    fs::create_dir_all(store_dir(&scope::top(root)))?;
    Ok(s)
}

/// The root's task store (D-77): every task of every project, one file each.
pub(crate) fn store_dir(top: &Path) -> PathBuf {
    top.join(".solace/tasks")
}

/// The tasks whose home is the project `root`, in rank order.
pub(crate) fn tasks(root: &Path) -> Result<Vec<Value>, Failure> {
    let top = scope::top(root);
    let home = scope::dir(&top, root);
    Ok(all_tasks(&top)?.into_iter().filter(|t| t["home"] == home).collect())
}

/// The locks a change takes: its project's, and for a task, every project in
/// its scope (and in a new scope it is given), in folder order (D-61).
pub(crate) fn guard(root: &Path, parts: Vec<&str>, body: &Value) -> Result<Vec<fs::File>, Failure> {
    let ["tasks", tid, rest @ ..] = parts.as_slice() else { return Ok(vec![lock(root)?]) };
    let top = crate::scope::top(root);
    let home = crate::scope::dir(&top, root);
    let members = crate::workspace::members(&top)?;
    loop {
        let Ok(t) = task(root, tid) else { return Ok(vec![lock(root)?]) };
        let seen = crate::scope::of(&t, &home);
        let mut dirs = seen.clone();
        dirs.extend(body["scope"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(String::from)));
        let mut roots: Vec<PathBuf> = dirs.iter().filter(|d| members.contains(d)).map(|d| crate::scope::path(&top, d)).collect();
        roots.push(root.into());
        // Approving a settings proposal writes a file in the project it names.
        if let ["questions", qid] = rest
            && let Some(dir) =
                t["questions"].as_array().into_iter().flatten().find(|q| q["id"] == *qid).and_then(|q| q["settings"]["project"].as_str())
            && members.iter().any(|m| m == dir)
        {
            roots.push(crate::scope::path(&top, dir));
        }
        let held = crate::scope::lock_all(&roots)?;
        // The scope may have changed while this waited for the locks.
        if task(root, tid).is_ok_and(|t| crate::scope::of(&t, &home) == seen) {
            return Ok(held);
        }
    }
}

/// Whether a human change to a task changes code, so it takes its projects'
/// locks (D-77 point 5): undoing the task's changes, a final move that still has
/// changes or a live work run, or approving a settings proposal. Settling that
/// move happens after the reply, under the same locks. Cancelling a task that
/// never had changes, and every other change, is task-only. Reads without a lock.
pub(crate) fn needs_lock(root: &Path, parts: &[&str], body: &Value) -> Result<bool, Failure> {
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
                if crate::lockdown::recovery::pending_for(&st.root, tid) {
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
pub(crate) fn checked_scope(root: &Path, v: &Value) -> Result<Vec<String>, Failure> {
    let top = crate::scope::top(root);
    let members = crate::workspace::members(&top)?;
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

/// Moves a task's folder to another project of its scope when its home leaves it.
/// `from` is the project whose board holds the task's folder now.
pub(crate) fn move_task(from: &Path, t: &mut Value, to: &Path) -> Result<(), Failure> {
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
    t["home"] = json!(crate::scope::dir(&crate::scope::top(to), to));
    Ok(())
}

/// The project `root`'s tasks as a list shows them, without their notes and changes.
pub(crate) fn listed(root: &Path) -> Result<Vec<Value>, Failure> {
    let top = crate::scope::top(root);
    let mut ts = crate::board::tasks(root)?;
    let mut waits = HashMap::new();
    for t in &mut ts {
        for k in ["notes", "changes"] {
            t.as_object_mut().unwrap().remove(k);
        }
        let current = std::mem::take(t);
        let found_project = crate::tasks::has_project(&top, &current);
        *t = crate::tasks::with_on_board(&top, current, found_project, &mut waits);
    }
    Ok(ts)
}

/// `GET /tasks` and `GET /tasks/{id}/...` of the project `root`.
pub(crate) fn read_task_api(root: &Path, parts: &[&str]) -> Result<Value, Failure> {
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

/// `POST /tasks`: a new task of the project `root`, not yet written, in
/// `Column::Planning` (its first column if it has none, D-132), else `Column::First`.
pub(crate) fn created(root: &Path, body: &Value, column: Column) -> Result<Value, Failure> {
    let w = flow(root)?;
    let title = body["title"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| Failure("E7001 title required".into(), 1))?;
    if body.get("body").is_some_and(|x| !x.is_string()) || body.get("links").is_some_and(|x| !x.is_array()) {
        return err("E7001", "invalid task fields");
    }
    let top = crate::scope::top(root);
    let home = crate::scope::dir(&top, root);
    let scope = match body.get("scope") {
        Some(v) => checked_scope(root, v)?,
        None => vec![home.clone()],
    };
    if !scope.contains(&home) {
        return err("E7001", format!("a task is created in a project of its scope; post it to one of {}", scope.join(", ")));
    }
    let rank = crate::board::tasks(root)?.iter().filter_map(|t| t["rank"].as_f64()).fold(0., f64::max) + 1.;
    let column = match column {
        Column::First => w["columns"][0]["id"].clone(),
        Column::Planning => planning_column(&w),
    };
    let mut t = new_task(title, body["body"].as_str().unwrap_or(""), &column, rank);
    t["scope"] = json!(scope);
    t["links"] = body.get("links").cloned().unwrap_or(json!([]));
    t["home"] = json!(home);
    Ok(t)
}

/// Applies a message from an agent or a command to task `t` (D-77 point 3):
/// the same change on the server and, to check it first, in the sender.
pub(crate) fn message(w: &Value, t: &mut Value, m: &Value) -> Result<(), Failure> {
    let rid = m["run"].as_str();
    let text = m["text"].as_str().unwrap_or("");
    match m["kind"].as_str().unwrap_or("") {
        "agent" => {
            let rid = rid.unwrap_or("");
            let i = run_at(t, rid)?;
            let kind = t["runs"][i]["kind"].as_str().unwrap_or("").to_string();
            let command = m["command"].as_str().unwrap_or("");
            // A review's feedback finishes its run, even when it arrives after the
            // run ended; every other note for an ended run is only kept.
            if kind == "review" && command == "note" {
                return crate::board::reviews::record_note(t, rid, text, m["changed_during_review"] == true);
            }
            note(t, "agent", Some(rid), text);
            // The run ended (stopped, canceled, replaced) before this arrived: the
            // note is kept, but it must not wake the run or move the task.
            if t["runs"][i]["state"] == "ended" {
                return Ok(());
            }
            match command {
                "note" if kind == "discuss" => t["runs"][i]["state"] = json!("waiting"),
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
        // A message left by an older version; delivery wakes the run now.
        "resume" => {}
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
        "outside_note" => {
            outside_note(t, text);
            if let Some(reason) = m["flag"].as_str() {
                flag(t, reason);
            }
        }
        "close" => {
            if t["runs"].as_array().is_some_and(|rs| rs.iter().any(|r| r["state"] != "ended")) {
                return Ok(());
            }
            if let Some(to) = closing_column(w) {
                set_column(w, t, &to);
                if t["attention"]["hold"] != true {
                    t["attention"] = Value::Null;
                }
                t["finished"] = Value::Null;
            }
        }
        _ => return err("E7001", "unknown task message"),
    }
    Ok(())
}

/// Ends runs a previous server left going, as after a restart. `announce`
/// logs the workflow; a project joining a served board is in its snapshot.
pub(crate) fn recover_project(root: &Path, announce: bool) -> Result<(), Failure> {
    let guard = lock(root)?;
    let s = open(root)?;
    let w = flow(root)?;
    crate::lockdown::recovery::recover(root, &s, std::slice::from_ref(&guard))?;
    if announce {
        event(&s, json!({"workflow":w}))?;
    }
    for mut t in tasks(root)? {
        if active(&t) {
            let work = t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended" && r["kind"] == "work");
            end_runs(&mut t, "server restarted");
            for r in t["runs"].as_array_mut().unwrap() {
                if r["outcome"] == "server restarted" {
                    r["failed"] = json!(true);
                }
            }
            if work {
                hold(&mut t, "the Solace server restarted while an agent was working");
                on_event(&w, &mut t, "failed")?;
            } else {
                flag(&mut t, "the Solace server restarted while an agent was working");
            }
            crate::tasks::put(&s, &t)?;
        }
    }
    Ok(())
}

/// The action whose agent role gives a column's model pick its default.
fn main_action(column: &str) -> Option<&'static str> {
    match column {
        "planning" => Some("investigate"),
        "assigned" => Some("reassign"),
        "needs_review" => Some("review"),
        _ => None,
    }
}

/// The role an action's run step belongs to, "" for no run.
fn run_role(a: &Value) -> &'static str {
    a["run"].as_str().and_then(crate::board::roles::for_run).map_or("", |role| role.id)
}

/// Replaces the task's outside-changes note in place, so repeated saves do not
/// pile up notes and one that no longer holds is not left behind.
pub(crate) fn outside_note(t: &mut Value, text: &str) {
    let notes = t["notes"].as_array_mut().unwrap();
    if let Some(n) = notes.iter_mut().find(|n| n["outside"] == true) {
        n["text"] = json!(text);
        n["time"] = json!(now());
    } else {
        notes.push(json!({"id":unique_id(),"author":"solace","run":Value::Null,"time":now(),"text":text,"outside":true}));
    }
}

/// The final column an emptied Unexpected Changes task closes into.
fn closing_column(w: &Value) -> Option<Value> {
    let cols = w["columns"].as_array()?;
    let finished = w["events"]["finished"].as_str()?;
    let next = cols.iter().find(|c| c["id"] == finished)?.get("next").and_then(Value::as_str);
    next.filter(|id| cols.iter().any(|c| c["id"] == *id && c["final"] == true))
        .map(|id| json!(id))
        .or_else(|| cols.iter().find(|c| c["final"] == true && c["id"] != "cancelled").map(|c| c["id"].clone()))
        .or_else(|| cols.iter().find(|c| c["final"] == true).map(|c| c["id"].clone()))
}

fn set_column(w: &Value, t: &mut Value, to: &Value) {
    t["column"] = to.clone();
    // When the task first reaches a final column; never cleared (D-87).
    if column(w, t)["final"] == true && t["closedTime"].is_null() {
        t["closedTime"] = json!(now());
    }
    // A task that leaves the final columns is back on the board (D-95).
    if column(w, t)["final"] != true {
        t["archivedTime"] = Value::Null;
    }
}
