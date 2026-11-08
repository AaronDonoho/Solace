//! Settings proposals (D-38, D-45): a work run's `write solace.toml` or
//! `write AGENTS.md` is checked now and written only when a human approves it
//! on the board. An approved Cargo.toml dependency change is fetched; a failed
//! fetch puts the file back as it was.
use serde_json::Value;
use serde_json::json;
use std::fs;
use std::path::Path;
use crate::board::err;
use crate::board::runs::wait_reply;
use crate::board::runs::REJECTED;
use crate::board::task_model::task;
use crate::board::task_model::flow;
use crate::board::task_model::workflow;
use crate::board::task_model::message;
use crate::Store;
use crate::unique_id;
use crate::failure::Failure;
use crate::failure::Out;
use crate::lockdown;
use crate::scope;
use crate::recipe;
use crate::tasks;
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

/// Applies an approved settings question, unless the file changed since.
pub(crate) fn apply_settings(root: &Path, q: &mut Value) -> Result<(), Failure> {
    let file = q["settings"]["file"].as_str().unwrap_or("solace.toml").to_string();
    let dir = match q["settings"]["project"].as_str() {
        Some(project) => scope::path(&scope::top(root), project),
        None => root.to_path_buf(),
    };
    if let Err(e) = crate::lockdown::paths::reachable(&dir, &file) {
        q["answer"] = json!(format!("Not applied: {}", e.0));
        return Ok(());
    }
    if crate::lockdown::paths::design_path(&dir, &file) {
        let e = crate::lockdown::paths::design_error::<()>(&file).unwrap_err();
        q["answer"] = json!(format!("Not applied: {}", e.0));
        return Ok(());
    }
    let p = dir.join(&file);
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
        if let Err(e) = crate::language::cargo::fetch(&dir) {
            let _ = if existed { fs::write(&p, &current) } else { fs::remove_file(&p) };
            q["answer"] = json!(format!("Not applied: {}", e.0.trim_end()));
            return Ok(());
        }
    }
    q["answer"] = json!("Approved and written");
    Ok(())
}
