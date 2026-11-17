//! Builds (D-246): run a workspace's recipes one at a time, reporting each
//! step's state, a capped log and a failure summary. Records live in the root's
//! store, so a restart can mark a running build interrupted.

use super::*;
use crate::failure::Failure;
use crate::lockdown::commands;
use serde_json::Value;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
#[cfg(test)]
#[path = "builds_tests.rs"]
mod tests;

/// The bytes of a step's log kept: the end, so a failure's last words survive.
const LOG_CAP: usize = 64 * 1024;

/// How many lines a failure summary keeps.
const SUMMARY_LINES: usize = 5;

type ChildHandle = Arc<Mutex<Option<Child>>>;

static RUNS: LazyLock<Mutex<HashMap<String, ChildHandle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

static CHANGE: Mutex<()> = Mutex::new(());

struct Signup(String);

impl Drop for Signup {
    fn drop(&mut self) {
        RUNS.lock().unwrap().remove(&self.0);
    }
}

struct Print {
    project: String,
    outputs: Vec<String>,
    sources: Value,
    last: usize,
}

struct Outcome {
    exit: Option<i32>,
    ms: u64,
    output: String,
    timed_out: bool,
    stopped: bool,
}

#[derive(Default)]
struct Log {
    cap: VecDeque<u8>,
    pending: String,
}

impl Log {
    fn push(&mut self, bytes: &[u8]) {
        let text = crate::diagnostics::strip_ansi(&String::from_utf8_lossy(bytes));
        self.cap.extend(text.bytes());
        while self.cap.len() > LOG_CAP {
            self.cap.pop_front();
        }
        self.pending.push_str(&text);
        if self.pending.len() > LOG_CAP {
            let cut = self.pending.len() - LOG_CAP;
            self.pending.drain(..cut);
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.cap.iter().copied().collect::<Vec<u8>>()).into_owned()
    }

    fn take_pending(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

fn dir(top: &Path) -> PathBuf {
    top.join(".solace/builds")
}

fn file(top: &Path, id: &str) -> PathBuf {
    dir(top).join(format!("{id}.json"))
}

fn log_path(top: &Path, id: &str, n: usize) -> PathBuf {
    dir(top).join(id).join(format!("{n}.log"))
}

fn save(top: &Path, build: &Value) -> Result<(), Failure> {
    let id = build["id"].as_str().unwrap_or("");
    crate::jsonx::replace_pretty(&dir(top), &format!("{id}.json"), build)
}

fn announce(top: &Path, build: &Value) {
    crate::tasks::store::log(top, "", &json!({"build": build}));
}

/// Every build, oldest first.
pub fn listed(top: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir(top)) else { return vec![] };
    let mut out: Vec<Value> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| crate::jsonx::read_opt(&p))
        .collect();
    out.sort_by(|a, b| a["startedAt"].as_u64().cmp(&b["startedAt"].as_u64()).then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    out
}

fn read(top: &Path, id: &str) -> Option<Value> {
    super::task_model::valid_id(id).then(|| crate::jsonx::read_opt(&file(top, id))).flatten()
}

/// One build in full.
pub fn get(top: &Path, id: &str) -> Result<Value, Failure> {
    read(top, id).ok_or_else(|| Failure(format!("E7023 unknown build {id}"), 1))
}

/// One step's log, capped to its end. A step not started yet has none.
pub fn step_log(top: &Path, id: &str, n: &str) -> Result<String, Failure> {
    let build = get(top, id)?;
    let i: usize = n.parse().map_err(|_| Failure("E7001 a step number is a whole number".into(), 1))?;
    if i >= build["steps"].as_array().map_or(0, Vec::len) {
        return super::err("E7023", format!("build {id} has no step {i}"));
    }
    Ok(fs::read_to_string(log_path(top, id, i)).unwrap_or_default())
}

/// Starts a build of the named projects and what they depend on, else of every
/// recipe in order. Refused while another build runs (D-246).
pub fn start(top: &Path, body: &Value) -> Result<Value, Failure> {
    let plan = crate::recipe::plan(top)?;
    let names = match body.get("projects") {
        None | Some(Value::Null) => vec![],
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
            .ok_or_else(|| Failure("E7001 projects is a list of project folders".into(), 1))?,
    };
    let chosen = select(plan, &names)?;
    let version = match body.get("version") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) if !v.trim().is_empty() => Some(v.trim().to_string()),
        Some(_) => return super::err("E7001", "version is a semver string like 1.2.3"),
    };
    let commit = crate::releases::git(top, &["rev-parse", "HEAD"]);
    let dirty = crate::releases::dirty(top);
    let _guard = CHANGE.lock().unwrap();
    if let Some(running) = listed(top).into_iter().find(|b| b["state"] == "running") {
        let id = running["id"].as_str().unwrap_or("");
        return super::err("E7003", format!("build {id} is already running"));
    }
    if let Some(id) = RUNS.lock().unwrap().keys().next().cloned() {
        return super::err("E7003", format!("build {id} is still stopping"));
    }
    if let Some(v) = &version {
        crate::versions::check(top, v)?;
    }
    let edits = match &version {
        Some(v) => {
            let edits = crate::versions::edits(top, v)?;
            crate::versions::write(top, &edits)?;
            Some(edits)
        }
        None => None,
    };
    let mut steps = vec![];
    let mut prints = vec![];
    for recipe in &chosen {
        let sources = if recipe.outputs.is_empty() {
            Value::Null
        } else {
            crate::recipe::source_hashes(top, &crate::scope::path(top, &recipe.project))
        };
        let first = steps.len();
        for step in &recipe.steps {
            steps.push(json!({
                "project": recipe.project,
                "name": step.name,
                "command": step.run,
                "timeout": step.timeout,
                "state": "waiting",
                "exitCode": Value::Null,
                "duration": 0,
            }));
        }
        if !recipe.outputs.is_empty() && steps.len() > first {
            prints.push(Print { project: recipe.project.clone(), outputs: recipe.outputs.clone(), sources, last: steps.len() - 1 });
        }
    }
    let id = crate::unique_id();
    let version = version.unwrap_or_else(|| id.clone());
    let mut build = json!({
        "id": id,
        "version": version,
        "state": "running",
        "startedAt": crate::now(),
        "endedAt": Value::Null,
        "commit": commit,
        "dirty": dirty,
        "steps": steps,
    });
    if let Some(edits) = &edits {
        build["versionFiles"] = crate::versions::entries(edits);
    }
    if let Err(e) = save(top, &build) {
        if let Some(edits) = &edits {
            let _ = crate::versions::restore_edits(top, edits);
        }
        return Err(e);
    }
    RUNS.lock().unwrap().insert(id.clone(), Arc::new(Mutex::new(None)));
    announce(top, &build);
    let thread_top = top.to_path_buf();
    let thread_id = id.clone();
    std::thread::spawn(move || run(thread_top, thread_id, prints));
    Ok(build)
}

fn select(plan: Vec<crate::recipe::Recipe>, names: &[String]) -> Result<Vec<crate::recipe::Recipe>, Failure> {
    if names.is_empty() {
        return Ok(plan);
    }
    for name in names {
        if !plan.iter().any(|r| &r.project == name) {
            return super::err("E7023", format!("unknown project {name}"));
        }
    }
    let mut picked = std::collections::HashSet::new();
    let mut stack = names.to_vec();
    while let Some(name) = stack.pop() {
        if !picked.insert(name.clone()) {
            continue;
        }
        if let Some(recipe) = plan.iter().find(|r| r.project == name) {
            stack.extend(recipe.after.iter().cloned());
        }
    }
    Ok(plan.into_iter().filter(|r| picked.contains(&r.project)).collect())
}

fn run(top: PathBuf, id: String, prints: Vec<Print>) {
    let _signup = Signup(id.clone());
    let total = read(&top, &id).and_then(|b| b["steps"].as_array().map(Vec::len)).unwrap_or(0);
    for n in 0..total {
        {
            let _guard = CHANGE.lock().unwrap();
            let Some(mut build) = read(&top, &id) else { return };
            if build["state"] != "running" {
                return;
            }
            build["steps"][n]["state"] = json!("running");
            if save(&top, &build).is_err() {
                return;
            }
            announce(&top, &build);
        }
        let outcome = execute(&top, &id, n);
        {
            let _guard = CHANGE.lock().unwrap();
            let Some(mut build) = read(&top, &id) else { return };
            if build["state"] != "running" {
                return;
            }
            match outcome {
                Ok(out) if !out.timed_out && !out.stopped && out.exit == Some(0) => {
                    build["steps"][n]["state"] = json!("passed");
                    build["steps"][n]["exitCode"] = json!(out.exit);
                    build["steps"][n]["duration"] = json!(out.ms);
                }
                Ok(out) => {
                    let summary = summarize(&build["steps"][n], &out.output, out.exit, out.timed_out);
                    build["steps"][n]["state"] = json!("failed");
                    build["steps"][n]["exitCode"] = json!(out.exit);
                    build["steps"][n]["duration"] = json!(out.ms);
                    build["steps"][n]["summary"] = summary;
                    fail_rest(&mut build, n + 1);
                    let _ = crate::versions::restore(&top, &build);
                    build["versionsRestored"] = json!(true);
                    let _ = save(&top, &build);
                    announce(&top, &build);
                    return;
                }
                Err(e) => {
                    let summary = summarize(&build["steps"][n], &e.0, None, false);
                    build["steps"][n]["state"] = json!("failed");
                    build["steps"][n]["exitCode"] = Value::Null;
                    build["steps"][n]["summary"] = summary;
                    fail_rest(&mut build, n + 1);
                    let _ = crate::versions::restore(&top, &build);
                    build["versionsRestored"] = json!(true);
                    let _ = save(&top, &build);
                    announce(&top, &build);
                    return;
                }
            }
            if save(&top, &build).is_err() {
                return;
            }
            announce(&top, &build);
        }
        if let Some(print) = prints.iter().find(|p| p.last == n) {
            let root = crate::scope::path(&top, &print.project);
            let _ = crate::recipe::write_fingerprints(&root, &print.outputs, &print.sources);
        }
    }
    let _guard = CHANGE.lock().unwrap();
    let Some(mut build) = read(&top, &id) else { return };
    if build["state"] == "running" {
        if let Err(e) = crate::releases::record(&top, &build) {
            build["state"] = json!("failed");
            build["endedAt"] = json!(crate::now());
            let _ = crate::versions::restore(&top, &build);
            build["versionsRestored"] = json!(true);
            build["error"] = json!(e.0);
            let _ = save(&top, &build);
            announce(&top, &build);
            return;
        }
        build["state"] = json!("passed");
        build["endedAt"] = json!(crate::now());
        let _ = save(&top, &build);
        announce(&top, &build);
    }
}

fn fail_rest(build: &mut Value, from: usize) {
    build["state"] = json!("failed");
    build["endedAt"] = json!(crate::now());
    if let Some(steps) = build["steps"].as_array_mut() {
        for step in steps.iter_mut().skip(from) {
            if step["state"] == "waiting" {
                step["state"] = json!("skipped");
            }
        }
    }
}

fn join_command(step: &Value) -> String {
    step["command"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn summarize(step: &Value, output: &str, exit: Option<i32>, timed_out: bool) -> Value {
    let mut lines: Vec<String> = output
        .lines()
        .filter(|line| line.to_lowercase().contains("error"))
        .take(SUMMARY_LINES)
        .map(|line| line.trim_end().to_string())
        .collect();
    if lines.is_empty() {
        lines = output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .rev()
            .take(SUMMARY_LINES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| line.trim_end().to_string())
            .collect();
    }
    if timed_out {
        lines.push(format!("timed out after {}s", step["timeout"].as_u64().unwrap_or(0)));
    }
    json!({"step": step["name"], "command": join_command(step), "exitCode": exit, "lines": lines})
}

fn execute(top: &Path, id: &str, n: usize) -> Result<Outcome, Failure> {
    let build = get(top, id)?;
    let step = build["steps"].get(n).cloned().ok_or_else(|| Failure(format!("E7023 build {id} has no step {n}"), 1))?;
    let root = crate::scope::path(top, step["project"].as_str().unwrap_or("."));
    let run: Vec<String> =
        step["command"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let timeout = step["timeout"].as_u64().unwrap_or(120);
    let path = log_path(top, id, n);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut cmd = commands::prepared(&root, &run, false)?;
    let mut child = cmd
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure(format!("could not start {}: {e}", run.first().cloned().unwrap_or_default()), 1))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let log = Arc::new(Mutex::new(Log::default()));
    let out_thread = {
        let log = log.clone();
        std::thread::spawn(move || drain(stdout, log))
    };
    let err_thread = {
        let log = log.clone();
        std::thread::spawn(move || drain(stderr, log))
    };
    let handle = RUNS.lock().unwrap().get(id).cloned();
    let mut held = Some(child);
    if let Some(handle) = &handle {
        *handle.lock().unwrap() = held.take();
    }
    let canceled = handle.is_none() || {
        let _guard = CHANGE.lock().unwrap();
        read(top, id).map_or(true, |build| build["state"] != "running")
    };
    if canceled && let Some(mut child) = handle.as_ref().and_then(|h| h.lock().unwrap().take()).or_else(|| held.take()) {
        commands::kill_group(&mut child);
    }
    let started = Instant::now();
    let mut status = None;
    let mut timed_out = false;
    let mut stopped = canceled;
    loop {
        {
            let mut slot = handle.as_ref().map(|h| h.lock().unwrap());
            match slot.as_deref_mut().and_then(Option::as_mut) {
                None => stopped = true,
                Some(child) => match child.try_wait() {
                    Ok(Some(done)) => status = Some(done),
                    Ok(None) => {}
                    Err(_) => stopped = true,
                },
            }
        }
        flush(top, id, n, &path, &log);
        if stopped || status.is_some() || timed_out {
            break;
        }
        if started.elapsed() >= Duration::from_secs(timeout) {
            if let Some(handle) = &handle
                && let Some(mut child) = handle.lock().unwrap().take()
            {
                commands::kill_group(&mut child);
            }
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(handle) = &handle
        && let Some(mut child) = handle.lock().unwrap().take()
    {
        let _ = child.wait();
    }
    let _ = out_thread.join();
    let _ = err_thread.join();
    let text = flush(top, id, n, &path, &log);
    Ok(Outcome { exit: status.and_then(|s| s.code()), ms: started.elapsed().as_millis() as u64, output: text, timed_out, stopped })
}

fn drain(mut pipe: impl Read, log: Arc<Mutex<Log>>) {
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => log.lock().unwrap().push(&buffer[..n]),
        }
    }
}

/// Writes the capped log and sends what is new since the last flush.
fn flush(top: &Path, id: &str, n: usize, path: &Path, log: &Arc<Mutex<Log>>) -> String {
    let (text, pending) = {
        let mut log = log.lock().unwrap();
        let pending = log.take_pending();
        (log.text(), pending)
    };
    let _ = crate::fsx::write_atomic(path, &text);
    if !pending.is_empty() {
        crate::tasks::store::log(top, "", &json!({"buildLog": {"id": id, "step": n, "text": pending}}));
    }
    text
}

/// Stops the running step and its children, and skips the rest (D-246).
pub fn cancel(top: &Path, id: &str) -> Result<Value, Failure> {
    let build = {
        let _guard = CHANGE.lock().unwrap();
        let mut build = get(top, id)?;
        if build["state"] != "running" {
            return super::err("E7003", format!("build {id} is not running"));
        }
        build["state"] = json!("canceled");
        build["endedAt"] = json!(crate::now());
        if let Some(steps) = build["steps"].as_array_mut() {
            for step in steps {
                if step["state"] == "running" || step["state"] == "waiting" {
                    step["state"] = json!("skipped");
                }
            }
        }
        save(top, &build)?;
        announce(top, &build);
        build
    };
    let handle = RUNS.lock().unwrap().get(id).cloned();
    if let Some(handle) = handle
        && let Some(mut child) = handle.lock().unwrap().take()
    {
        commands::kill_group(&mut child);
    }
    wait_for_stop(id);
    {
        let _guard = CHANGE.lock().unwrap();
        if let Some(mut build) = read(top, id)
            && build["state"] == "canceled"
        {
            let _ = crate::versions::restore(top, &build);
            build["versionsRestored"] = json!(true);
            let _ = save(top, &build);
        }
    }
    Ok(build)
}

/// Marks a build a stopped server left running as failed (D-246).
pub fn recover(top: &Path) -> Result<(), Failure> {
    let _guard = CHANGE.lock().unwrap();
    for mut build in listed(top) {
        if build["state"] != "running" {
            continue;
        }
        build["state"] = json!("failed");
        build["endedAt"] = json!(crate::now());
        build["reason"] = json!("interrupted");
        if let Some(steps) = build["steps"].as_array_mut() {
            for step in steps {
                match step["state"].as_str() {
                    Some("running") => {
                        let summary = summarize(step, "", None, false);
                        step["state"] = json!("failed");
                        step["exitCode"] = Value::Null;
                        step["summary"] = summary;
                    }
                    Some("waiting") => step["state"] = json!("skipped"),
                    _ => {}
                }
            }
        }
        let _ = crate::versions::restore(top, &build);
        build["versionsRestored"] = json!(true);
        save(top, &build)?;
        announce(top, &build);
    }
    Ok(())
}

/// How long a cancel waits for the run's commands to stop.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Waits until the run loop has released `id`, at most `STOP_WAIT`.
fn wait_for_stop(id: &str) {
    let started = Instant::now();
    while started.elapsed() < STOP_WAIT && RUNS.lock().unwrap().contains_key(id) {
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Files a failed step as a task in its project (D-246): the step's summary,
/// command, a link to the build and the full log. A step that already has a
/// task returns it, so a second press opens that task.
pub fn file_task(top: &Path, id: &str, n: &str, body: &Value) -> Result<Value, Failure> {
    let i: usize = n.parse().map_err(|_| Failure("E7001 a step number is a whole number".into(), 1))?;
    let title = match body.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(_) => return super::err("E7001", "title must be text"),
    };
    let text = match body.get("body") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return super::err("E7001", "body must be text"),
    };
    let _guard = CHANGE.lock().unwrap();
    let mut build = get(top, id)?;
    let step = build["steps"].get(i).cloned().ok_or_else(|| Failure(format!("E7023 build {id} has no step {i}"), 1))?;
    if step["state"] != "failed" {
        return super::err("E7003", format!("step {i} of build {id} has not failed"));
    }
    if let Some(tid) = step["task"].as_str()
        && let Ok(path) = super::task_model::task_path(top, tid)
        && let Ok(existing) = super::task_model::read_task(top, &path)
    {
        return Ok(existing);
    }
    let project = step["project"].as_str().unwrap_or(".").to_string();
    let root = crate::scope::path(top, &project);
    let log = step_log(top, id, n)?;
    let build_id = id.to_string();
    let request = format!("build:{build_id}:{i}");
    let top_path = top.to_path_buf();
    let task = crate::tasks::server::op(top, move || {
        if let Some(existing) = super::task_model::all_tasks(&top_path)?.into_iter().find(|t| t["request"] == json!(request)) {
            return Ok(existing);
        }
        let title = title.unwrap_or_else(|| format!("build: {project} {} fails", step["name"].as_str().unwrap_or("")));
        let text = text.ok_or_else(|| Failure("E7001 body must be text".into(), 1))?;
        let mut t = super::task_model::created(&root, &json!({"title": title, "body": text}), super::task_model::Column::First)?;
        t["request"] = json!(request);
        t["links"] = json!([{"name":"Build","url":build_url(&build_id)}]);
        let note = if log.is_empty() { "The step wrote no output.".to_string() } else { log };
        super::task_model::note(&mut t, "solace", None, &note);
        crate::tasks::store::write(&top_path, &t)?;
        Ok(t)
    })?;
    build["steps"][i]["task"] = task["id"].clone();
    save(top, &build)?;
    announce(top, &build);
    Ok(task)
}

/// The address of a build: the Delivery page's Build tab, keeping the build's id.
fn build_url(id: &str) -> String {
    format!("/delivery?build={id}")
}
