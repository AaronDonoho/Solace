use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use crate::failure::Failure;
use crate::tasks::store::feature_shows;
use crate::tasks::store::write;
use crate::tasks::server::op;
use crate::tasks::server::settle_later;
use crate::tasks::store::record;
use crate::tasks::store::log;
use crate::tasks::store::recover;
use crate::tasks::strategy::singular;
use crate::scope;
use crate::board;
use crate::board::task_model::Column;
use crate::board::task_model::After;
use crate::board::task_model::valid_id;
use crate::board::task_model::task_root;
use crate::board::task_model::all_tasks;
use crate::board::task_model::read_task;
use crate::board::task_model::project_home;
use crate::board::task_model::guard;
use crate::board::task_model::needs_lock;
use crate::board::task_model::read_task_api;
use crate::board::task_model::created;
use crate::board::task_model::store_dir;
/// `GET /tasks/find/{id}` (D-98): the project that stores a task, whichever it
/// is, whether it is archived, and whether the board shows it, so a task link
/// opens it on the board or, off the board, on the Strategy page; and its title,
/// which a bare link shows (D-122). 404 for no such task.
pub fn find_task(top: &Path, id: &str) -> Result<Value, (u16, Failure)> {
    recover(&scope::top(top)).map_err(|e| (409, e))?;
    if !valid_id(id) {
        return board::err("E7001", "invalid task id").map_err(|e| (400, e));
    }
    let path = store_dir(top).join(format!("{id}.json"));
    if !path.exists() {
        return board::err("E7023", format!("unknown task {id}")).map_err(|e| (404, e));
    }
    let t = read_task(top, &path).map_err(|e| (409, e))?;
    let found_project = has_project(top, &t);
    let mut waits = HashMap::new();
    let t = with_on_board(top, t, found_project, &mut waits);
    Ok(json!({
        "id": id,
        "dir": t["home"],
        "archived": !t["archivedTime"].is_null(),
        "title": t["title"],
        "project": found_project,
        "onBoard": t["onBoard"].clone(),
    }))
}

/// `GET /features/find/{id}` and `GET /milestones/find/{id}` (D-122): a
/// record's title, which a bare link to it shows. 404 for no such record.
pub fn find_record(top: &Path, kind: &str, id: &str) -> Result<Value, (u16, Failure)> {
    recover(top).map_err(|e| (409, e))?;
    if !valid_id(id) {
        return board::err("E7001", format!("invalid {} id", singular(kind))).map_err(|e| (400, e));
    }
    match record(top, kind, id) {
        Some(r) => Ok(json!({"id": id, "title": r["title"]})),
        None => board::err("E7023", format!("unknown {} {id}", singular(kind))).map_err(|e| (404, e)),
    }
}

/// `/tasks/...` for the project `root`. Reads take nothing; a change is one
/// operation, and a create with a `request` seen before returns its task
/// (D-77 point 6).
pub fn api(root: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, Failure> {
    api_with(root, method, parts, body, needs_lock)
}

/// Whether a change takes its task's projects' locks: `board::needs_lock`, or
/// a stand-in in tests.
type NeedsLock = fn(&Path, &[&str], &Value) -> Result<bool, Failure>;

/// `api`, with `needs` deciding whether a change takes the locks.
pub(crate) fn api_with(root: &Path, method: &str, parts: &[&str], body: Value, needs: NeedsLock) -> Result<Value, Failure> {
    recover(&scope::top(root))?;
    if method == "GET" {
        return read_task_api(root, parts);
    }
    // Only a change to code takes its projects' locks (D-77 point 5), so a
    // task-only change never waits on an agent's command.
    let mut guard = if needs(root, parts, &body)? { Some(guard(root, parts.to_vec(), &body)?) } else { None };
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
                && let Some(t) = all_tasks(&top)?.into_iter().find(|t| t["request"] == *r)
            {
                return Ok(t);
            }
            let mut t = created(&root, &body, Column::First)?;
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
            guard = Some(crate::board::task_model::guard(root, parts.to_vec(), &body)?);
            change(root, method, parts, &body, true, needs)?.ok_or_else(|| Failure("the task's projects were not locked".into(), 1))?
        }
    };
    let settle = matches!(&after, After::Clean(_));
    board::after(&task_root(&scope::top(root), &t), &t, after);
    drop(guard);
    if settle {
        settle_later(&scope::top(root), t["id"].as_str().unwrap().to_string());
    }
    Ok(t)
}

/// One change to a task, as an operation. Without its projects' locks
/// (`held` false) it checks again inside the operation, where nothing else
/// writes the task, and returns None, changing nothing, when the change now
/// needs them (`needs` says). The operation never takes a lock itself (see `op`).
pub(crate) fn change(
    root: &Path,
    method: &str,
    parts: &[&str],
    body: &Value,
    held: bool,
    needs: NeedsLock,
) -> Result<Option<(Value, After)>, Failure> {
    let (root, method, body, top) = (root.to_path_buf(), method.to_string(), body.clone(), scope::top(root));
    let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    let served = top.clone();
    op(&served, move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        if !held && needs(&root, &parts, &body)? {
            return Ok(None);
        }
        let tid = parts[1];
        let (t, after) = crate::board::task_model::change(&root, tid, &method, &parts[2..], &body)?;
        // The feature's Tech Debt task first (D-112): its notes name this task.
        if let After::Clean(Some(debt)) = &after {
            write(&top, debt)?;
        }
        write(&top, &t)?;
        if let After::Removed(dir) = &after {
            log(&top, dir, &json!({"removed":tid}));
        }
        Ok(Some((t, after)))
    })
}

/// Whether the board shows task `t`, which has a project when `has_project` is
/// true and waits on a feature when `waiting` is true (D-95, D-128, D-129, D-133).
fn shows_on_board(t: &Value, has_project: bool, waiting: bool) -> bool {
    t["archivedTime"].is_null() && t["column"] != "backlog" && has_project && !waiting
}

/// Adds the board's computed `onBoard` to task `t`, which has a project when
/// `has_project` is true (D-196). `waits` remembers feature states.
pub(crate) fn with_on_board(top: &Path, mut t: Value, has_project: bool, waits: &mut HashMap<String, bool>) -> Value {
    let waiting = t["feature"].as_str().is_some_and(|id| feature_waits(top, id, waits));
    t["onBoard"] = json!(shows_on_board(&t, has_project, waiting));
    t
}

/// Whether feature `id` keeps its tasks off the board until it starts; a
/// missing feature does not (D-128). `waits` remembers answers for a list.
pub(super) fn feature_waits(top: &Path, id: &str, waits: &mut HashMap<String, bool>) -> bool {
    *waits.entry(id.to_string()).or_insert_with(|| record(top, "features", id).is_some_and(|f| !feature_shows(&f)))
}

/// Whether task `t`'s home names a project of the workspace (D-129).
pub(crate) fn has_project(top: &Path, t: &Value) -> bool {
    project_home(top, t).is_some()
}
