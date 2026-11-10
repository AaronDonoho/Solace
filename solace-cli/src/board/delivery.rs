//! Handing human notes to a run's agent exactly once: the run keeps the notes
//! and answers it has delivered, so a wait never depends on the agent's last note.

use crate::board::task;
use crate::board::task_model::run_at;
use crate::failure::Failure;
use crate::scope;
use crate::tasks;
use serde_json::Value;
use std::path::Path;
#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;



fn seen(run: &Value, field: &str) -> std::collections::HashSet<String> {
    match &run[field] {
        Value::Array(keys) => keys.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Value::Object(keys) => keys.keys().cloned().collect(),
        _ => std::collections::HashSet::new(),
    }
}

fn pending_notes(notes: &[Value], run: &Value, rid: Option<&str>) -> Vec<(String, String)> {
    let delivered = seen(run, "notes_seen");
    notes
        .iter()
        .filter(|n| n["author"] == "human" && rid.is_none_or(|rid| n["run"] == rid))
        .filter_map(|n| {
            let key = crate::board::briefs::note_fingerprint(n);
            (!delivered.contains(&key)).then(|| (key, n["text"].as_str().unwrap_or("").to_string()))
        })
        .collect()
}

fn pending_answers(run: &Value) -> Vec<(String, String, String)> {
    let delivered = seen(run, "answers_seen");
    run["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| {
            let id = q["id"].as_str()?;
            let answer = q["answer"].as_str()?;
            let first = crate::board::runs::cut_line(q["text"].as_str().unwrap_or("").lines().next().unwrap_or(""));
            (!delivered.contains(id)).then(|| (id.to_string(), first, answer.to_string()))
        })
        .collect()
}

fn take_pending(run: &Value, notes: &[Value], rid: Option<&str>) -> Option<Pending> {
    let pending = Pending { notes: pending_notes(notes, run, rid), answers: pending_answers(run) };
    (!pending_is_empty(&pending)).then_some(pending)
}

/// Hands over every human note and answered proposal the run has not seen, once,
/// and wakes a waiting run. Every open proposal's answer is handed over.
pub(crate) fn claim_strategy_reply(top: &Path, rid: &str) -> Result<Option<String>, Failure> {
    let Some(run) = tasks::strategy_run(top, rid) else { return crate::board::err("E7023", format!("unknown run {rid}")) };
    if run["state"] == "ended" {
        return Ok(None);
    }
    let notes = run["notes"].as_array().cloned().unwrap_or_default();
    let Some(pending) = take_pending(&run, &notes, None) else { return Ok(None) };
    let claim = crate::unique_id();
    let m = serde_json::json!({
        "kind": "delivery", "target": "strategy", "run": rid, "wake": true, "claim": claim,
        "notes": pending_note_keys(&pending), "answers": pending_answer_keys(&pending),
    });
    if !send_delivery(top, m)? {
        return crate::board::err("E7003", "the board server stopped while waiting");
    }
    let Some(run) = tasks::strategy_run(top, rid) else { return Ok(None) };
    let owned = pending_owned(&pending, &run, &claim);
    Ok((!pending_is_empty(&owned)).then(|| pending_text(&owned)))
}

/// A discussion's copy of `claim_strategy_reply`: a paused task holds its notes.
pub(crate) fn claim_task_reply(root: &Path, tid: &str, rid: &str) -> Result<Option<String>, Failure> {
    let top = scope::top(root);
    let t = task(root, tid)?;
    if !t["paused_from"].is_null() {
        return Ok(None);
    }
    let i = run_at(&t, rid)?;
    if t["runs"][i]["state"] == "ended" {
        return Ok(None);
    }
    let notes = t["notes"].as_array().cloned().unwrap_or_default();
    let Some(pending) = take_pending(&t["runs"][i], &notes, Some(rid)) else { return Ok(None) };
    let claim = crate::unique_id();
    let m = serde_json::json!({
        "kind": "delivery", "target": "task", "task": tid, "run": rid, "wake": true, "claim": claim,
        "notes": pending_note_keys(&pending), "answers": pending_answer_keys(&pending),
    });
    if !send_delivery(&top, m)? {
        return crate::board::err("E7003", "the board server stopped while waiting");
    }
    let t = task(root, tid)?;
    let i = run_at(&t, rid)?;
    let owned = pending_owned(&pending, &t["runs"][i], &claim);
    Ok((!pending_is_empty(&owned)).then(|| pending_text(&owned)))
}

/// Marks the notes and answers in a fresh strategy prompt as delivered.
pub(crate) fn mark_strategy_brief(top: &Path, run: &Value) -> Result<(), Failure> {
    let Some(rid) = run["id"].as_str() else { return Ok(()) };
    let notes = run["notes"].as_array().cloned().unwrap_or_default();
    let answers: Vec<String> = run["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| !q["answer"].is_null())
        .filter_map(|q| q["id"].as_str().map(str::to_string))
        .collect();
    let keys: Vec<String> = notes.iter().filter(|n| n["author"] == "human").map(crate::board::briefs::note_fingerprint).collect();
    let m = serde_json::json!({
        "kind": "delivery", "target": "strategy", "run": rid, "wake": false, "claim": crate::unique_id(),
        "notes": keys, "answers": answers,
    });
    send_delivery(top, m)?;
    Ok(())
}

/// Marks the notes in a fresh discussion prompt as delivered.
pub(crate) fn mark_discussion_brief(root: &Path, tid: &str, rid: &str, t: &Value) -> Result<(), Failure> {
    let notes = t["notes"].as_array().cloned().unwrap_or_default();
    let keys: Vec<String> =
        notes.iter().filter(|n| n["author"] == "human" && n["run"] == rid).map(crate::board::briefs::note_fingerprint).collect();
    let m = serde_json::json!({
        "kind": "delivery", "target": "task", "task": tid, "run": rid, "wake": false, "claim": crate::unique_id(),
        "notes": keys, "answers": [],
    });
    send_delivery(&scope::top(root), m)?;
    Ok(())
}

pub(crate) fn wait_limit() -> std::time::Duration {
    wait_limit_with_env(|name| std::env::var(name).ok())
}

/// The reply of a wait that reached `wait_limit`: nothing is claimed, so the
/// note stays pending for the wait-only call that follows.
pub(crate) const WAIT_AGAIN: &str =
    "Still waiting for the human. Call `strategy wait` (a strategy run) or `task wait` (a discussion) to keep waiting; it sends nothing.\n";

/// Leaves a delivery message for the task server and waits for it. A message
/// no server took is withdrawn, so it can't be delivered without its agent.
fn send_delivery(top: &Path, m: Value) -> Result<bool, Failure> {
    let sent = tasks::send(top, m)?;
    if tasks::applied(top, &sent) {
        return Ok(true);
    }
    let _ = std::fs::remove_file(&sent);
    Ok(false)
}

/// `wait_limit` for a task timeout, kept a minute below the client's cap so a
/// reply reaches the agent before its tool call is cut off.
pub(crate) fn wait_limit_for(task_timeout: u64) -> std::time::Duration {
    std::time::Duration::from_secs(crate::lockdown::tool_timeout(task_timeout).saturating_sub(60).max(1))
}

fn owner<'a>(run: &'a Value, field: &str, key: &str) -> Option<&'a str> {
    run[field].get(key).and_then(Value::as_str)
}

fn claims(run: &Value, field: &str) -> serde_json::Map<String, Value> {
    match &run[field] {
        Value::Object(keys) => keys.clone(),
        Value::Array(keys) => keys.iter().filter_map(Value::as_str).map(|key| (key.to_string(), Value::String(String::new()))).collect(),
        _ => serde_json::Map::new(),
    }
}

fn record_keys(run: &mut Value, field: &str, keys: &[Value], claim: &str) {
    let mut claimed = claims(run, field);
    for key in keys.iter().filter_map(Value::as_str) {
        claimed.entry(key.to_string()).or_insert_with(|| serde_json::json!(claim));
    }
    run[field] = Value::Object(claimed);
}

struct Pending {
    notes: Vec<(String, String)>,
    answers: Vec<(String, String, String)>,
}

fn pending_note_keys(pending: &Pending) -> Vec<String> {
    pending.notes.iter().map(|(key, _)| key.clone()).collect()
}

fn pending_answer_keys(pending: &Pending) -> Vec<String> {
    pending.answers.iter().map(|(key, _, _)| key.clone()).collect()
}

fn pending_is_empty(pending: &Pending) -> bool {
    pending.notes.is_empty() && pending.answers.is_empty()
}

fn pending_owned(pending: &Pending, run: &Value, claim: &str) -> Pending {
    Pending {
        notes: pending.notes.iter().filter(|(key, _)| owner(run, "notes_seen", key) == Some(claim)).cloned().collect(),
        answers: pending.answers.iter().filter(|(key, _, _)| owner(run, "answers_seen", key) == Some(claim)).cloned().collect(),
    }
}

fn pending_text(pending: &Pending) -> String {
    let mut parts = vec![];
    if !pending.notes.is_empty() {
        let text: Vec<&str> = pending.notes.iter().map(|(_, text)| text.as_str()).collect();
        parts.push(format!("The human replied:\n\n{}", text.join("\n\n")));
    }
    if !pending.answers.is_empty() {
        let text: Vec<String> =
            pending.answers.iter().map(|(_, first, answer)| format!("The human answered the proposal {first}: {answer}")).collect();
        parts.push(text.join("\n\n"));
    }
    format!("{}\n", parts.join("\n\n"))
}

/// Records a delivered handover on `run`: the notes and answers this claim
/// took, and whether it wakes the run. A paused run takes a brief mark but
/// holds a waiting claim.
pub(crate) fn record(run: &mut Value, m: &Value, paused: bool) {
    if run["state"] == "ended" {
        return;
    }
    let wake = m["wake"] == true;
    if paused && wake {
        return;
    }
    let claim = m["claim"].as_str().unwrap_or("");
    record_keys(run, "notes_seen", m["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]), claim);
    record_keys(run, "answers_seen", m["answers"].as_array().map(Vec::as_slice).unwrap_or(&[]), claim);
    if wake && !paused {
        run["state"] = serde_json::json!("running");
    }
}

pub(crate) fn pending_task_reply(run: &Value, notes: &[Value], rid: &str) -> bool {
    !pending_notes(notes, run, Some(rid)).is_empty() || !pending_answers(run).is_empty()
}

pub(crate) fn pending_strategy_reply(run: &Value) -> bool {
    let notes = run["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    !pending_notes(notes, run, None).is_empty() || !pending_answers(run).is_empty()
}

/// How long the agent's own wait may run before it returns and asks the agent
/// to call again: below the timeout the agent client allows a tool call.
fn wait_limit_with_launch(launch: Option<u64>) -> std::time::Duration {
    let secs = launch.or_else(|| crate::board::setup::system().ok().and_then(|s| s["task_timeout"].as_u64())).unwrap_or(3600);
    wait_limit_for(secs)
}

/// The wait chosen by the agent's environment: an explicit wait limit, else
/// the timeout the agent was launched with, else the current setting.
fn wait_limit_with_env(get: impl Fn(&str) -> Option<String>) -> std::time::Duration {
    if let Some(secs) = get("SOLACE_WAIT_LIMIT_SECS").and_then(|s| s.parse::<u64>().ok()).filter(|n| *n > 0) {
        return std::time::Duration::from_secs(secs);
    }
    let launch = get("SOLACE_LAUNCH_TIMEOUT_SECS").and_then(|s| s.parse::<u64>().ok()).filter(|n| *n > 0);
    wait_limit_with_launch(launch)
}
