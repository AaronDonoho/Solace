use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use std::fs;
use crate::now;
use crate::sha256;
use crate::unique_id;
use crate::failure::Failure;
use crate::tasks::server::op;
use crate::tasks::store::records;
use crate::tasks::store::stored_records;
use crate::tasks::store::record;
use crate::tasks::store::mark_start;
use crate::tasks::store::put_record;
use crate::tasks::store::remove_record;
use crate::tasks::store::log;
use crate::tasks::store::write;
use crate::tasks::store::write_tasks_then;
use crate::tasks::store::write_features;
use crate::tasks::store::recover;
use crate::board;
use crate::fsx;
use crate::jsonx;
use crate::board::task_model::Column;
use crate::workspace;
use crate::scope;
use crate::board::task_model::task_root;
use crate::board::task_model::all_tasks;
use crate::board::task_model::conversation;
use crate::board::task_model::note;
use crate::board::task_model::end_run;
use crate::board::task_model::plan_for_feature;
use crate::board::task_model::set_links;
use crate::board::task_model::change;
use crate::board::runs::transcript;
use crate::board::task_model::created;
use crate::board::setup::system;
use crate::board::setup::chosen_model;
use crate::board::roles::STRATEGY_RUN;
/// Moves task `t` from Backlog to Planning when its feature is `active`
/// (D-134), as `board::plan_for_feature` says. Whether it moved.
pub(crate) fn plan(top: &Path, t: &mut Value) -> bool {
    let Some(f) = t["feature"].as_str().and_then(|id| record(top, "features", id)) else { return false };
    plan_for_feature(top, t, &f)
}

/// The states of a milestone or a feature (D-87 point 8; `upcoming` was
/// `planned` before D-128).
const STATES: [&str; 5] = ["proposed", "upcoming", "active", "complete", "canceled"];

pub(crate) fn strategy_dir(top: &Path, kind: &str) -> PathBuf {
    top.join(".solace/strategy").join(kind)
}

/// The event key of one record of `kind` (`milestones` or `features`).
pub(crate) fn singular(kind: &str) -> &'static str {
    if kind == "milestones" { "milestone" } else { "feature" }
}

pub(crate) fn rank_of(v: &Value) -> f64 {
    v["rank"].as_f64().unwrap_or(0.)
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
        tasks = all_tasks(top)?.into_iter().filter(|t| t["milestone"] == id).collect();
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
        tasks = all_tasks(top)?.into_iter().filter(|t| t[link] == id).collect();
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
pub(crate) fn vision_file(top: &Path) -> PathBuf {
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
    fsx::replace_bytes(vision_file(top).parent().unwrap(), "vision.md", text.as_bytes())?;
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
pub(crate) fn put_run(top: &Path, run: &Value) -> Result<(), Failure> {
    jsonx::replace_pretty(&strategy_dir(top, "runs"), &format!("{}.json", run["id"].as_str().unwrap_or("")), run)?;
    log(top, "", &json!({"strategyRun": run}));
    Ok(())
}

/// One strategy run as its file holds it, if there is one.
pub(crate) fn strategy_run(top: &Path, rid: &str) -> Option<Value> {
    record(top, "runs", rid)
}

/// Changes strategy run `rid` with `f`, read and written as one operation.
pub(crate) fn update_run<R: Send + 'static>(
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
                note(r, "solace", Some(&rid), "the Solace server restarted while an agent was working");
                end_strategy(r, "stopped");
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// Ends strategy run `run` with `outcome`: its notes become one transcript
/// note, as a discussion's do.
pub(crate) fn end_strategy(run: &mut Value, outcome: &str) {
    let (model, rid) = (run["model"].as_str().unwrap_or("").to_string(), run["id"].as_str().unwrap_or("").to_string());
    end_run(run, outcome);
    let turns = run["notes"].as_array_mut().map(std::mem::take).unwrap_or_default();
    if !turns.is_empty() {
        note(run, "solace", Some(&rid), &conversation(&turns, &model));
    }
}

/// `/strategy/runs...` (D-93), at the root only; `parts` are those after `runs`.
/// Reads take nothing and a change is one operation. Starting a run records it;
/// its agent is launched elsewhere, and stops when it sees the run ended. An
/// error comes with its HTTP status: 404 for an unknown id in the path, 400 for
/// a malformed body, else 409.
pub fn strategy_runs_api(top: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, (u16, Failure)> {
    runs_api(top, method, parts, body, system)
}

/// The system settings, with the models a run may choose: `board::system`, or
/// a stand-in in tests.
type System = fn() -> Result<Value, Failure>;

pub(crate) fn runs_api(top: &Path, method: &str, parts: &[&str], body: Value, system: System) -> Result<Value, (u16, Failure)> {
    match (method, parts) {
        ("GET", []) => return Ok(json!(strategy_runs(top))),
        ("GET", [rid]) => {
            let Some(mut run) = record(top, "runs", rid) else {
                return board::err("E7023", format!("unknown strategy run {rid}")).map_err(|e| (404, e));
            };
            run["transcript"] = json!(transcript(Path::new(run["log"].as_str().unwrap_or(""))));
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
            note(&mut run, "human", Some(rid), text);
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
                if chosen && matches!(answer.as_str(), "Approve" | "Approve to Planning") {
                    let column = if answer == "Approve to Planning" { Column::Planning } else { Column::First };
                    answer = apply_proposal(top, rid, &q["proposal"], column).unwrap_or_else(|e| format!("Not applied: {}", e.0));
                } else if !chosen {
                    answer = format!("Not applied; the human answered: {answer}");
                }
            }
            q["answer"] = json!(answer);
            if !q["proposal"].is_null() {
                let what = q["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
                note(&mut run, "solace", Some(rid), &format!("Proposal: {what}\n\nAnswer: {answer}"));
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
/// context; a context names the page, or a feature or milestone that exists. A
/// task context is refused: a strategy run is never started on a task (D-202).
fn start_run(top: &Path, req: &Value, system: System, status: &mut u16) -> Result<Value, Failure> {
    *status = 400;
    let Some(text) = req["text"].as_str().filter(|t| !t.trim().is_empty()) else {
        return board::err("E7001", "a strategy run starts with a message");
    };
    let c = &req["context"];
    let context = match (c["kind"].as_str(), c["id"].as_str()) {
        (Some("page"), _) => json!({"kind": "page"}),
        (Some("task"), _) => return board::err("E7001", "a strategy run starts on the page, not a task"),
        (Some(kind @ ("feature" | "milestone")), Some(id)) => {
            if record(top, &format!("{kind}s"), id).is_none() {
                return board::err("E7023", format!("unknown {kind} {id}"));
            }
            json!({"kind": kind, "id": id})
        }
        _ => return board::err("E7001", "context must be {kind: page} or {kind: task|feature|milestone, id}"),
    };
    let m = chosen_model(&system()?, req).map_err(|e| {
        if e.0.starts_with("E7011") {
            *status = 409;
        }
        e
    })?;
    *status = 409;
    if let Some(open) = stored_records(top, "runs").iter().find(|r| r["state"] != "ended" && on_page(&r["context"]) == context) {
        return board::err("E7003", format!("strategy run {} on this context is open", open["id"].as_str().unwrap_or("")));
    }
    let id = unique_id();
    let log = strategy_dir(top, "runs").join(format!("{id}.jsonl"));
    let mut run = json!({"id": id, "kind": STRATEGY_RUN, "model": m["id"],
        "effort": m["effort"], "state": "running", "started": now(),
        "log": log, "context": context, "notes": [], "questions": []});
    note(&mut run, "human", Some(&id), text);
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
/// or the vision's new text. A new task goes in `column`.
fn build(top: &Path, rid: &str, p: &Value, column: Column) -> Result<Built, Failure> {
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
                let Some(t) = all_tasks(top)?.into_iter().find(|t| t["id"] == id) else {
                    return board::err("E7023", format!("unknown task {id}"));
                };
                let home = task_root(top, &t);
                let (mut t, _) = change(&home, id, "PATCH", &[], fields)?;
                note(&mut t, "solace", None, &format!("Changed by strategy run {rid}"));
                Ok(Built::Task(t, false))
            }
            None => Ok(Built::Task(proposed_task(top, rid, fields, column)?, true)),
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
/// its `scope` (the root's own without one), in `column` (Planning is its
/// first column if it has none, D-132), with `feature`, `milestone` and
/// `dependsOn` set as `PATCH` sets them. Every folder in `scope` must be a
/// project (D-129): a proposal naming one that isn't is refused here, when the
/// proposal is sent, before the human is asked.
fn proposed_task(top: &Path, rid: &str, fields: &Value, column: Column) -> Result<Value, Failure> {
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
    let mut t = created(&scope::path(top, &folders[0]), &body, column)?;
    note(&mut t, "solace", None, &format!("Created by strategy run {rid}"));
    set_links(top, &mut t, fields)?;
    Ok(t)
}

/// Id `id` without its link prefix: an agent copies `task:<id>` from a reply (D-148)
/// where a plain id is asked for.
pub(crate) fn bare_id(id: &str) -> &str {
    ["task:", "feature:", "milestone:"].iter().find_map(|p| id.strip_prefix(p)).unwrap_or(id)
}

/// A proposal's fields with each linked id bare: `feature`, `milestone` and `dependsOn`.
pub(crate) fn bare_links(mut fields: Value) -> Value {
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
pub(crate) fn proposal(top: &Path, rid: &str, kind: &str, id: Option<&str>, text: &str) -> Result<Value, Failure> {
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
    let offers_planning = kind == "task" && id.is_none() && matches!(fields.get("feature"), None | Some(Value::Null));
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
            "task" => all_tasks(top)?.into_iter().find(|t| t["id"] == id),
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
    let built = build(top, rid, &p, Column::First)?;
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
    let options = if offers_planning { json!(["Approve", "Approve to Planning", "Reject"]) } else { json!(["Approve", "Reject"]) };
    Ok(json!({"id": unique_id(), "text": format!("{what}{says}"), "options": options, "proposal": p, "answer": null}))
}

/// Applies strategy run `rid`'s approved proposal `p` as its request would be
/// applied (D-93), with the same refusals. Everything is built and checked
/// first, the vision's base among it, so a refused apply writes nothing. The
/// answer to record: what was applied. A new task goes in `column`.
fn apply_proposal(top: &Path, rid: &str, p: &Value, column: Column) -> Result<String, Failure> {
    if p["kind"] == "vision" && vision(top)["revision"] != p["base"] {
        return board::err("E7026", "the vision changed since the proposal; it was not replaced");
    }
    let built = build(top, rid, p, column)?;
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
            let created = if new {
                format!(
                    ": created task:{}{}",
                    t["id"].as_str().unwrap_or(""),
                    if matches!(column, Column::Planning) { " (Planning)" } else { "" }
                )
            } else {
                String::new()
            };
            format!("Approved and applied{created}")
        }
        Built::Vision(text) => {
            write_vision(top, &text)?;
            "Approved and applied".into()
        }
    };
    Ok(answer)
}

/// `context` with a task read as the page (D-202).
fn on_page(context: &Value) -> Value {
    if context["kind"] == "task" { json!({"kind": "page"}) } else { context.clone() }
}

/// Plans every Backlog task whose feature is `active` (D-134), as one
/// operation of the task server: run when the server starts, so tasks stored
/// before the rule move without a click.
pub fn plan_active(top: &Path) -> Result<(), Failure> {
    let root = top.to_path_buf();
    op(top, move || {
        for mut t in all_tasks(&root)? {
            if plan(&root, &mut t) {
                write(&root, &t)?;
            }
        }
        Ok(())
    })
}

pub const PERSONA_LIMIT: usize = VISION_LIMIT;

pub(crate) const PERSONA_FIELDS: [(&str, &str); 4] =
    [("objectives", "Objectives"), ("tools", "Tools"), ("concerns", "Concerns"), ("other", "Other")];

/// The personas' file (D-216): internal, next to the vision.
pub(crate) fn personas_file(top: &Path) -> PathBuf {
    top.join(".solace/strategy/personas.json")
}

/// Every persona, in creation order.
pub fn personas(top: &Path) -> Vec<Value> {
    persona_store(top).map(|(list, _)| list).unwrap_or_default()
}

/// The personas and retired ids. A missing file is empty; a damaged one is a
/// failure, so a write never replaces it with an empty list (D-216).
fn persona_store(top: &Path) -> Result<(Vec<Value>, Vec<String>), Failure> {
    let path = personas_file(top);
    if !path.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let v = jsonx::read(&path).map_err(|_| persona_damaged())?;
    let Some(personas) = v["personas"].as_array() else { return Err(persona_damaged()) };
    let retired = match v.get("retired") {
        None => Vec::new(),
        Some(Value::Array(ids)) => {
            ids.iter().map(|id| id.as_str().map(str::to_string).ok_or_else(persona_damaged)).collect::<Result<_, _>>()?
        }
        Some(_) => return Err(persona_damaged()),
    };
    Ok((personas.clone(), retired))
}

fn save_personas(top: &Path, list: &[Value], retired: &[String]) -> Result<(), Failure> {
    let doc = json!({"personas": list, "retired": retired});
    fsx::replace_bytes(personas_file(top).parent().unwrap(), "personas.json", &serde_json::to_vec_pretty(&doc).unwrap())?;
    log(top, "", &json!({"personas": list}));
    Ok(())
}

fn fresh_persona_id(list: &[Value], retired: &[String]) -> String {
    loop {
        let id = unique_id();
        if !retired.contains(&id) && !list.iter().any(|p| p["id"] == id.as_str()) {
            return id;
        }
    }
}

fn persona_fields(body: &Value) -> Result<Value, Failure> {
    let Some(name) = body["name"].as_str() else { return board::err("E7001", "name must be a string") };
    let name = name.trim();
    if name.is_empty() {
        return board::err("E7001", "a persona needs a name");
    }
    let mut p = json!({"name": name});
    for (key, _) in PERSONA_FIELDS {
        let text = match body.get(key) {
            None => "",
            Some(Value::String(text)) => text.as_str(),
            Some(_) => return board::err("E7001", format!("{key} must be a string")),
        };
        p[key] = json!(text);
    }
    persona_within_limit(&p)?;
    Ok(p)
}

fn persona_revision(p: &Value) -> String {
    let text: Vec<&str> =
        std::iter::once("name").chain(PERSONA_FIELDS.iter().map(|(key, _)| *key)).map(|key| p[key].as_str().unwrap_or("")).collect();
    format!("sha256:{}", sha256(text.join("\u{0}")))
}

fn persona_within_limit(p: &Value) -> Result<(), Failure> {
    let count: usize = std::iter::once("name")
        .chain(PERSONA_FIELDS.iter().map(|(key, _)| *key))
        .map(|key| p[key].as_str().unwrap_or("").chars().count())
        .sum();
    if count > PERSONA_LIMIT {
        return board::err("E7029", format!("the persona is {count} characters; the limit is {PERSONA_LIMIT}"));
    }
    Ok(())
}

fn create_persona(top: &Path, body: &Value, status: &mut u16) -> Result<Value, Failure> {
    let mut p = persona_fields(body).map_err(|e| {
        *status = 400;
        e
    })?;
    let (mut list, retired) = persona_store(top).map_err(|e| {
        *status = 409;
        e
    })?;
    p["id"] = json!(fresh_persona_id(&list, &retired));
    p["revision"] = json!(persona_revision(&p));
    list.push(p.clone());
    save_personas(top, &list, &retired)?;
    Ok(p)
}

fn replace_persona(top: &Path, id: &str, body: &Value, status: &mut u16) -> Result<Value, Failure> {
    let (mut list, retired) = persona_store(top).map_err(|e| {
        *status = 409;
        e
    })?;
    let Some(i) = list.iter().position(|p| p["id"] == id) else {
        *status = 404;
        return board::err("E7023", format!("unknown persona {id}"));
    };
    let Some(base) = body["base"].as_str() else {
        *status = 400;
        return board::err("E7001", "base must be a string");
    };
    let mut p = persona_fields(body).map_err(|e| {
        *status = 400;
        e
    })?;
    if list[i]["revision"] != base {
        *status = 409;
        return board::err("E7028", "the persona changed since this edit began; reload it");
    }
    p["id"] = json!(id);
    p["revision"] = json!(persona_revision(&p));
    list[i] = p.clone();
    save_personas(top, &list, &retired)?;
    Ok(p)
}

fn delete_persona(top: &Path, id: &str, status: &mut u16) -> Result<Value, Failure> {
    let (mut list, mut retired) = persona_store(top).map_err(|e| {
        *status = 409;
        e
    })?;
    let Some(i) = list.iter().position(|p| p["id"] == id) else {
        *status = 404;
        return board::err("E7023", format!("unknown persona {id}"));
    };
    let removed = list.remove(i);
    retired.push(id.to_string());
    save_personas(top, &list, &retired)?;
    Ok(removed)
}

fn persona_change(top: &Path, method: &str, parts: &[&str], body: &Value, status: &mut u16) -> Result<Value, Failure> {
    match (method, parts.len()) {
        ("POST", 0) => create_persona(top, body, status),
        ("PUT", 1) => replace_persona(top, parts[0], body, status),
        ("DELETE", 1) => delete_persona(top, parts[0], status),
        _ => {
            *status = 400;
            board::err("E7001", "unknown route")
        }
    }
}

/// `/personas` (D-216), at the root only: the list, create, replace over one
/// persona's revision, and delete. A refusal writes nothing.
pub fn personas_api(top: &Path, method: &str, parts: &[&str], body: Value) -> Result<Value, (u16, Failure)> {
    if method == "GET" && parts.is_empty() {
        return Ok(json!(personas(top)));
    }
    let (root, method) = (top.to_path_buf(), method.to_string());
    let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    let done = op(top, move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let mut status = 409;
        Ok(persona_change(&root, &method, &parts, &body, &mut status).map_err(|e| (status, e)))
    })
    .map_err(|e| (409, e))?;
    done
}

fn persona_damaged() -> Failure {
    Failure("E7030 the personas file is unreadable; it was left as it is".into(), 1)
}
