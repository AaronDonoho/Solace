//! Releases (D-247): a passed build's artifacts, the record beside them, and
//! the output folder settings the Build page reads and writes.

use crate::Failure;
use crate::lockdown;
use serde_json::{Value, json};
use std::fs;
use std::path::{Component, Path};

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
    Ok(json!({"releases": records}))
}

pub fn get(top: &Path, version: &str) -> Result<Value, Failure> {
    if version.is_empty() || version.contains('/') || version.contains("..") {
        return Err(Failure(format!("E7023 unknown release {version}"), 1));
    }
    crate::jsonx::read_opt(&top.join(folder(top)).join(version).join("release.json"))
        .ok_or_else(|| Failure(format!("E7023 unknown release {version}"), 1))
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
