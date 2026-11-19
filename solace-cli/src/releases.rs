//! Releases (D-247): a passed build's artifacts, the record beside them, and
//! the output folder settings the Build page reads and writes.

use crate::Failure;
use crate::lockdown;
use serde_json::{Value, json};
use std::fs;
use std::path::{Component, Path};
#[cfg(test)]
#[path = "releases_tests.rs"]
mod tests;

pub const DEFAULT_OUTPUT: &str = "build-output";

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub output: String,
    pub read_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { output: DEFAULT_OUTPUT.into(), read_only: true }
    }
}

pub fn check(v: &toml::Value) -> Result<(), Failure> {
    let Some(r) = v.get("releases") else { return Ok(()) };
    let Some(t) = r.as_table() else {
        return Err(lockdown::invalid_settings("releases: expected a table with output and read_only"));
    };
    if let Some(k) = t.keys().find(|k| !matches!(k.as_str(), "output" | "read_only")) {
        return Err(lockdown::invalid_settings(format!("releases.{k}: unknown field; [releases] takes output and read_only")));
    }
    if let Some(o) = t.get("output") {
        match o.as_str() {
            Some(p) if valid_output(p) => {}
            Some(p) => {
                return Err(lockdown::invalid_settings(format!("releases.output: `{p}` is not a folder inside the root, outside .solace")));
            }
            None => return Err(lockdown::invalid_settings("releases.output: expected a folder")),
        }
    }
    if t.get("read_only").is_some_and(|b| b.as_bool().is_none()) {
        return Err(lockdown::invalid_settings("releases.read_only: expected true or false"));
    }
    Ok(())
}

fn valid_output(p: &str) -> bool {
    let path = Path::new(p);
    let parts: Vec<&str> = p.split('/').filter(|c| !c.is_empty() && *c != ".").collect();
    !parts.is_empty()
        && !p.contains('\\')
        && !path.is_absolute()
        && !parts.contains(&"..")
        && parts.first().copied() != Some(".solace")
        && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

pub fn settings(top: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(top.join("solace.toml")) else { return Settings::default() };
    let Ok(v) = text.parse::<toml::Value>() else { return Settings::default() };
    let Some(r) = v.get("releases") else { return Settings::default() };
    let output = r
        .get("output")
        .and_then(|o| o.as_str())
        .filter(|p| valid_output(p))
        .map(|p| p.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_OUTPUT.into());
    let read_only = r.get("read_only").and_then(|b| b.as_bool()).unwrap_or(true);
    Settings { output, read_only }
}

pub fn folder(top: &Path) -> String {
    settings(top).output
}

pub fn json_settings(top: &Path) -> Value {
    let s = settings(top);
    json!({"output": s.output, "read_only": s.read_only})
}

pub fn save_settings(top: &Path, body: &Value) -> Result<Value, Failure> {
    let output = body.get("output").and_then(|o| o.as_str()).ok_or_else(|| Failure("E7001 output is a folder".into(), 1))?;
    if !valid_output(output) {
        return Err(lockdown::invalid_settings(format!("releases.output: `{output}` is not a folder inside the root, outside .solace")));
    }
    let read_only = match body.get("read_only") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(lockdown::invalid_settings("releases.read_only: expected true or false")),
    };
    let path = top.join("solace.toml");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let next = with_releases(&text, output, read_only);
    crate::lockdown::parse_settings(&next)?;
    crate::fsx::write_atomic(&path, next)?;
    Ok(json_settings(top))
}

fn with_releases(text: &str, output: &str, read_only: bool) -> String {
    let block = format!("[releases]\noutput = {}\nread_only = {read_only}\n", toml::Value::String(output.into()));
    let mut out = String::new();
    let mut skipping = false;
    let mut replaced = false;
    for line in text.lines() {
        if line.trim_start().starts_with('[') {
            skipping = false;
            if line.trim() == "[releases]" {
                skipping = true;
                if !replaced {
                    out.push_str(&block);
                    replaced = true;
                }
                continue;
            }
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        if !out.is_empty() && !out.ends_with("\n\n") {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str(&block);
    }
    out
}

pub(crate) fn record(top: &Path, build: &Value) -> Result<(), Failure> {
    let id = build["id"].as_str().unwrap_or("");
    let version = build["version"].as_str().unwrap_or(id);
    let mut files = vec![];
    let mut artifacts = vec![];
    for project in projects(build) {
        let Some(recipe) = crate::recipe::of(top, &project)? else { continue };
        for output in &crate::recipe::resolved(top, &recipe) {
            let rel = relative(&project, output);
            let bytes = fs::read(crate::scope::path(top, &project).join(output))
                .map_err(|_| Failure(format!("E7001 release output {rel} is missing"), 1))?;
            artifacts.push(json!({"path": rel.clone(), "size": bytes.len(), "sha256": crate::sha256(&bytes)}));
            files.push((rel, bytes));
        }
    }
    if artifacts.is_empty() {
        return Ok(());
    }
    let output = folder(top);
    let dir = top.join(&output).join(version);
    for (rel, bytes) in &files {
        let dest = dir.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dest, bytes)?;
    }
    crate::outside_share::ignore_output(top, &output)?;
    let release = json!({
        "version": version,
        "build": id,
        "time": crate::now(),
        "artifacts": artifacts,
        "versions": build.get("versionFiles").cloned().unwrap_or(json!([])),
        "commit": build.get("commit").cloned().unwrap_or(Value::Null),
        "dirty": build.get("dirty").cloned().unwrap_or(json!(false)),
        "tools": tools(top, build),
    });
    crate::jsonx::replace_pretty(&dir, "release.json", &release)
}

fn projects(build: &Value) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for step in build["steps"].as_array().into_iter().flatten() {
        if let Some(p) = step["project"].as_str()
            && !out.iter().any(|x| x == p)
        {
            out.push(p.to_string());
        }
    }
    out
}

fn relative(project: &str, output: &str) -> String {
    let output = output.trim_start_matches("./");
    if project == "." { output.to_string() } else { format!("{project}/{output}") }
}

fn tools(top: &Path, build: &Value) -> Value {
    let mut names: Vec<String> = vec![];
    for step in build["steps"].as_array().into_iter().flatten() {
        if let Some(name) = step["command"].as_array().and_then(|a| a.first()).and_then(|x| x.as_str())
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_string());
        }
    }
    let mut out = serde_json::Map::new();
    for name in names {
        let version = match crate::lockdown::commands::command(top, &[name.clone(), "--version".into()], 5, 4000, false, false) {
            Ok(text) if succeeded(&text) => {
                text.lines().find(|l| !l.starts_with("exit code:") && !l.trim().is_empty()).map(|l| json!(l.trim()))
            }
            Ok(_) => None,
            Err(_) => None,
        };
        out.insert(name, version.unwrap_or(Value::Null));
    }
    Value::Object(out)
}

pub(crate) fn git(top: &Path, args: &[&str]) -> Option<String> {
    let argv: Vec<String> = std::iter::once("git".to_string()).chain(args.iter().map(|a| a.to_string())).collect();
    let text = crate::lockdown::commands::command(top, &argv, 5, 4000, false, false).ok()?;
    if !succeeded(&text) {
        return None;
    }
    text.lines().find(|l| !l.starts_with("exit code:") && !l.trim().is_empty()).map(|l| l.trim().to_string())
}

pub(crate) fn dirty(top: &Path) -> Value {
    match crate::lockdown::commands::command(top, &["git".into(), "status".into(), "--porcelain".into()], 5, 4000, false, false) {
        Ok(text) if succeeded(&text) => json!(text.lines().any(|l| !l.starts_with("exit code:") && !l.trim().is_empty())),
        Err(_) => json!(false),
        Ok(_) => json!(false),
    }
}

pub fn listed(top: &Path) -> Result<Value, Failure> {
    let mut records: Vec<Value> = vec![];
    if let Ok(entries) = fs::read_dir(top.join(folder(top))) {
        for entry in entries.flatten() {
            if let Some(record) = crate::jsonx::read_opt(&entry.path().join("release.json")) {
                records.push(record);
            }
        }
    }
    records.sort_by(|a, b| b["time"].as_u64().cmp(&a["time"].as_u64()).then_with(|| b["version"].as_str().cmp(&a["version"].as_str())));
    for record in &mut records {
        let version = record["version"].as_str().unwrap_or("");
        record["notes"] = saved(top, version).unwrap_or(Value::Null);
    }
    Ok(json!({"releases": records}))
}

pub fn get(top: &Path, version: &str) -> Result<Value, Failure> {
    if version.is_empty() || version.contains('/') || version.contains("..") {
        return Err(Failure(format!("E7023 unknown release {version}"), 1));
    }
    let mut record = crate::jsonx::read_opt(&top.join(folder(top)).join(version).join("release.json"))
        .ok_or_else(|| Failure(format!("E7023 unknown release {version}"), 1))?;
    record["notes"] = saved(top, version).unwrap_or(Value::Null);
    Ok(record)
}

fn succeeded(text: &str) -> bool {
    text.starts_with("exit code: 0")
}

pub(crate) fn artifact(top: &Path, version: &str, path: &str) -> Result<std::path::PathBuf, Failure> {
    let record = get(top, version)?;
    let listed = !path.is_empty() && record["artifacts"].as_array().into_iter().flatten().any(|a| a["path"].as_str() == Some(path));
    let file = top.join(folder(top)).join(version).join(path);
    if !listed || !file.is_file() {
        return Err(Failure(format!("E7023 unknown release artifact {path}"), 1));
    }
    Ok(file)
}

/// Starts release `version`'s notes run, with a draft built from the work
/// completed since the release before it.
pub(crate) fn ask(top: &Path, version: &str) -> Result<Value, Failure> {
    get(top, version)?;
    if saved(top, version).is_some_and(|notes| notes["state"] == "approved") {
        return Err(Failure(format!("E7003 release {version}'s notes were already approved"), 1));
    }
    if let Some(open) = crate::tasks::strategy::notes_runs(top)
        .into_iter()
        .find(|r| r["state"] != "ended" && r["context"]["kind"] == "release" && r["context"]["id"] == version)
    {
        let rid = open["id"].as_str().unwrap_or("");
        return Err(Failure(format!("E7003 notes run {rid} for release {version} is open"), 1));
    }
    let mut notes = draft(top, version)?;
    let text = notes["text"].as_str().unwrap_or("").to_string();
    let message = format!(
        "Draft the release notes for release {version}. The draft below is already saved on the Build tab; \
         review it and save your version with `notes draft`, its Markdown in `text`. Keep one entry per task, \
         written `[title](task:<id>)`, under its feature's title; add no work that is not listed.\n\n{text}"
    );
    let run = crate::tasks::strategy::start_notes_run(top, version, &message)?;
    let rid = run["id"].as_str().unwrap_or("").to_string();
    notes["run"] = json!(rid);
    notes["state"] = json!("draft");
    notes["time"] = json!(crate::now());
    save(top, version, &notes)?;
    crate::board::runs::start_notes(top.to_path_buf(), rid);
    Ok(notes)
}

/// Saves the draft an agent wrote for release `version`.
pub(crate) fn save_draft(top: &Path, version: &str, text: &str) -> Result<Value, Failure> {
    let _lock = crate::board::task_model::lock(top)?;
    let mut notes = saved(top, version).ok_or_else(|| Failure(format!("E7023 release {version} has no notes draft"), 1))?;
    if notes["state"] != "draft" {
        return Err(Failure(format!("E7023 release {version}'s notes were already settled"), 1));
    }
    notes["state"] = json!("draft");
    notes["text"] = json!(text.trim_end());
    notes["time"] = json!(crate::now());
    save(top, version, &notes)?;
    Ok(notes)
}

/// The draft saved beside release `version`, if there is one.
pub(crate) fn saved(top: &Path, version: &str) -> Option<Value> {
    crate::jsonx::read_opt(&file(top, version))
}

/// The draft release `version` starts from: the work completed after the
/// release before it, up to and including it.
pub(crate) fn draft(top: &Path, version: &str) -> Result<Value, Failure> {
    let releases = listed(top)?["releases"].as_array().cloned().unwrap_or_default();
    let at = releases.iter().position(|r| r["version"] == version).ok_or_else(|| Failure(format!("E7023 unknown release {version}"), 1))?;
    let upper = releases[at]["time"].as_u64().unwrap_or(0);
    let lower = releases.get(at + 1).and_then(|r| r["time"].as_u64());
    let tasks = window_tasks(top, lower, upper)?;
    let features = crate::tasks::records(top, "features");
    let groups = groups(&tasks, &features);
    let decisions = decisions(top, &tasks, lower, upper);
    let text = markdown(version, &groups, &decisions);
    Ok(json!({"version": version, "groups": groups, "decisions": decisions, "text": text}))
}

fn file(top: &Path, version: &str) -> std::path::PathBuf {
    top.join(folder(top)).join(version).join("notes.json")
}

fn save(top: &Path, version: &str, notes: &Value) -> Result<(), Failure> {
    let dir = file(top, version).parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    crate::jsonx::replace_pretty(&dir, "notes.json", notes)?;
    crate::tasks::store::log(top, "", &json!({"releaseNotes": {"version": version}}));
    Ok(())
}

fn window_tasks(top: &Path, lower: Option<u64>, upper: u64) -> Result<Vec<Value>, Failure> {
    let mut flows: std::collections::HashMap<std::path::PathBuf, Option<Value>> = std::collections::HashMap::new();
    let mut out = vec![];
    for t in crate::board::task_model::all_tasks(top)? {
        let Some(closed) = t["closedTime"].as_u64() else { continue };
        if closed > upper || lower.is_some_and(|low| closed <= low) {
            continue;
        }
        let root = crate::scope::path(top, t["home"].as_str().unwrap_or("."));
        let w = flows.entry(root.clone()).or_insert_with(|| crate::board::task_model::flow(&root).ok());
        let Some(w) = w else { continue };
        if crate::board::task_model::column(w, &t)["final"] != true {
            continue;
        }
        out.push(t);
    }
    Ok(out)
}

fn groups(tasks: &[Value], features: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    for t in tasks {
        let id = t["id"].as_str().unwrap_or("");
        let title = t["title"].as_str().unwrap_or(id);
        let feature = t["feature"].as_str().and_then(|fid| features.iter().find(|f| f["id"] == fid));
        let (key, label, complete) = match feature {
            Some(f) => {
                (f["id"].as_str().unwrap_or("").to_string(), f["title"].as_str().unwrap_or("").to_string(), f["state"] == "complete")
            }
            None => (String::new(), "Other work".to_string(), false),
        };
        let entry = json!({"task": id, "title": title});
        match out.iter().position(|g| g["key"] == key) {
            Some(i) => out[i]["tasks"].as_array_mut().unwrap().push(entry),
            None => out.push(json!({"key": key, "title": label, "complete": complete, "tasks": [entry]})),
        }
    }
    out
}

fn decisions(top: &Path, tasks: &[Value], lower: Option<u64>, upper: u64) -> Vec<Value> {
    let text = fs::read_to_string(crate::decisions::file(top)).unwrap_or_default();
    let titles: Vec<String> = tasks.iter().filter_map(|t| t["title"].as_str().map(unquote)).collect();
    let lower_date = lower.map(crate::decisions::date_of);
    let upper_date = crate::decisions::date_of(upper);
    crate::decisions::records(&text)
        .into_iter()
        .filter(|r| match origin(&r.text) {
            Some((origin, date)) => match origin.strip_prefix("board task ") {
                Some(title) => titles.iter().any(|t| *t == unquote(title)),
                None => {
                    origin.starts_with("strategy run")
                        && lower_date.as_deref().is_none_or(|low| date.as_str() >= low)
                        && date.as_str() <= upper_date.as_str()
                }
            },
            None => false,
        })
        .map(|r| json!({"id": r.id, "title": r.title}))
        .collect()
}

fn origin(record: &str) -> Option<(String, String)> {
    let line = record
        .lines()
        .find(|l| l.starts_with('(') && l.ends_with(')') && (l.contains(", board task ") || l.contains(", strategy run ")))?;
    let parts: Vec<&str> = line[1..line.len() - 1].split(", ").collect();
    let date = parts.last()?.to_string();
    let origin = parts.get(1..parts.len() - 1)?.join(", ");
    Some((origin, date))
}

fn unquote(text: &str) -> String {
    text.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(text).to_string()
}

fn markdown(version: &str, groups: &[Value], decisions: &[Value]) -> String {
    let mut out = format!("# Release {version}\n");
    for group in groups {
        out += &format!("\n## {}\n", clean(group["title"].as_str().unwrap_or("")));
        for entry in group["tasks"].as_array().into_iter().flatten() {
            let title = clean(entry["title"].as_str().unwrap_or(""));
            out += &format!("- [{}](task:{})\n", title, entry["task"].as_str().unwrap_or(""));
        }
    }
    if !decisions.is_empty() {
        out += "\n## Decisions\n";
        for d in decisions {
            out += &format!("- {}: {}\n", d["id"].as_str().unwrap_or(""), clean(d["title"].as_str().unwrap_or("")));
        }
    }
    out
}

fn clean(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace(['[', ']'], "")
}

/// Saves the human's edited draft as release `version`'s notes.
pub(crate) fn approve(top: &Path, version: &str, text: &str) -> Result<Value, Failure> {
    let _lock = crate::board::task_model::lock(top)?;
    get(top, version)?;
    let mut notes = saved(top, version)
        .filter(|n| n["state"] == "draft")
        .ok_or_else(|| Failure(format!("E7023 release {version} has no notes draft"), 1))?;
    end_open_notes_run(top, version, "finished")?;
    notes["state"] = json!("approved");
    notes["text"] = json!(text.trim_end());
    notes["time"] = json!(crate::now());
    save(top, version, &notes)?;
    get(top, version)
}

/// Drops release `version`'s notes and any draft run still writing them.
pub(crate) fn reject(top: &Path, version: &str) -> Result<Value, Failure> {
    let _lock = crate::board::task_model::lock(top)?;
    get(top, version)?;
    if saved(top, version).filter(|notes| notes["state"] == "draft").is_none() {
        return Err(Failure(format!("E7023 release {version} has no notes draft"), 1));
    }
    end_open_notes_run(top, version, "stopped")?;
    let path = file(top, version);
    if path.exists() {
        fs::remove_file(path)?;
    }
    crate::tasks::store::log(top, "", &json!({"releaseNotes": {"version": version}}));
    get(top, version)
}

/// Ends the notes run still writing `version`'s draft, if there is one.
fn end_open_notes_run(top: &Path, version: &str, outcome: &str) -> Result<(), Failure> {
    for run in crate::tasks::strategy::notes_runs(top) {
        if run["state"] == "ended" || run["context"]["kind"] != "release" || run["context"]["id"] != version {
            continue;
        }
        let rid = run["id"].as_str().unwrap_or("").to_string();
        let outcome = outcome.to_string();
        crate::tasks::strategy::update_run(top, &rid, move |run| {
            crate::tasks::strategy::end_strategy(run, &outcome);
            Ok(())
        })?;
    }
    Ok(())
}
