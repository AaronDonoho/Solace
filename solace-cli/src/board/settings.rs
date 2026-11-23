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
    let top = scope::top(&s.root);
    let dir = scope::dir(&top, project);
    // The file's path from the root folder, so a board of several projects says which one.
    let name = top.file_name().map(Path::new).unwrap_or(Path::new("")).join(project.strip_prefix(&top).unwrap_or(Path::new(""))).join(file);
    let name = name.to_string_lossy();
    let why = reason.map(str::trim).filter(|r| !r.is_empty()).map(|r| format!("\n\n{r}")).unwrap_or_default();
    let question = json!({"id":unique_id(),"text":format!("Change {name}{why}"),"options":["Approve","Reject"],
        "settings":{"file":file,"project":dir,"base":base,"proposed":proposed}});
    ask_approval(s, tid, rid, question, guard)
}

/// Applies an approved settings question from task `title`, unless the file changed since.
pub(crate) fn apply_settings(root: &Path, title: &str, q: &mut Value) -> Result<(), Failure> {
    if let Some(d) = q["settings"].get("decision") {
        let entry = crate::decisions::Entry {
            title: d["title"].as_str().unwrap_or("").to_string(),
            body: d["body"].as_str().unwrap_or("").to_string(),
            edit: d["edit"].as_u64().map(|n| n as u32),
        };
        let origin = format!("board task \"{title}\"");
        match crate::decisions::append(&scope::top(root), &entry, "human", &origin, d["base"].as_str()) {
            Ok(id) => {
                let what = if entry.edit.is_some() { "changed" } else { "added" };
                q["answer"] = json!(format!("Approved and {what} {id}"));
            }
            Err(e) => q["answer"] = json!(format!("Not applied: {}", e.0)),
        }
        return Ok(());
    }
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

/// A run's `write DECISIONS.md`: the entry it proposes is added only when a
/// human approves it, with its number chosen then.
pub fn propose_decision(s: &Store, tid: &str, rid: &str, text: &str, reason: Option<&str>, guard: Vec<fs::File>) -> Out {
    let entry = crate::decisions::entry(text)?;
    let top = scope::top(&s.root);
    let base = entry.edit.map(|n| crate::decisions::current(&top, n)).transpose()?;
    let why = reason.map(str::trim).filter(|r| !r.is_empty()).map(|r| format!("\n\n{r}")).unwrap_or_default();
    let what = match (entry.edit, &base) {
        (Some(n), Some(current)) => {
            format!("Change decision D-{n}{why}\n\nCurrent:\n{current}\n\nProposed:\n## D-{n} {}\n{}", entry.title, entry.body)
        }
        _ => format!("Record decision \"{}\"{why}\n\n{}", entry.title, entry.body),
    };
    let question = json!({
        "id": unique_id(),
        "text": what,
        "options": ["Approve", "Reject"],
        "settings": {"decision": {"title": entry.title, "body": entry.body, "edit": entry.edit, "base": base}},
    });
    ask_approval(s, tid, rid, question, guard)
}

/// Puts `question`, a proposal with Approve and Reject, to the human for run
/// `rid` of task `tid`: the human's answer, or that it waits in the queue.
fn ask_approval(s: &Store, tid: &str, rid: &str, question: Value, guard: Vec<fs::File>) -> Out {
    let t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let top = scope::top(&s.root);
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
    wait_reply(s, tid, rid, crate::board::delivery::wait_limit())
}

/// A run's `task propose`: the task `text` (its fields as a JSON object) would
/// create, checked now and created in Planning only when a human approves it
/// (D-93 point 4). Without a `scope` it goes in the project of task `tid`. A
/// `blocking` one also makes task `tid` depend on it.
pub fn propose_task(s: &Store, tid: &str, rid: &str, text: &str, blocking: bool, guard: Vec<fs::File>) -> Out {
    let top = scope::top(&s.root);
    let mut fields = crate::tasks::strategy::proposal_fields("task", false, text)?;
    if fields.get("scope").is_none() {
        fields["scope"] = json!([scope::dir(&top, &s.root)]);
    }
    let proposed = task_for(&top, tid, rid, &fields, blocking)?;
    let blocks = if blocking { "\n\nApproving also makes this task depend on it." } else { "" };
    let question = json!({
        "id": unique_id(),
        "text": format!("Create task \"{}\"{blocks}", proposed["title"].as_str().unwrap_or("")),
        "options": ["Approve", "Reject"],
        "settings": {"task": {"fields": fields, "blocking": blocking, "run": rid}},
    });
    ask_approval(s, tid, rid, question, guard)
}

/// The task run `rid` of task `tid` proposes, built and checked against the
/// tasks as they are now, writing nothing. One that blocks task `tid` must not
/// depend on it.
fn task_for(top: &Path, tid: &str, rid: &str, fields: &Value, blocking: bool) -> Result<Value, Failure> {
    let origin = format!("Created by run {rid} of task:{tid}");
    let t = crate::tasks::strategy::proposed_task(top, &origin, fields, crate::board::task_model::Column::Planning)?;
    let deps: Vec<String> = t["dependsOn"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(String::from)).collect();
    if blocking && crate::board::task_model::reaches(&crate::board::task_model::dependency_graph(top)?, deps, tid) {
        return err("E7025", "these dependencies would make a cycle");
    }
    Ok(t)
}

/// Creates the task an approved `task propose` names, in Planning, unless it
/// is refused now, and records the answer on `q`. The new task's id when it
/// blocks task `tid`, for `tid` to depend on.
pub(crate) fn approve_task(root: &Path, tid: &str, q: &mut Value) -> Option<String> {
    let p = q["settings"]["task"].clone();
    let blocking = p["blocking"] == true;
    let top = scope::top(root);
    let created = task_for(&top, tid, p["run"].as_str().unwrap_or(""), &p["fields"], blocking)
        .and_then(|t| crate::tasks::store::write(&top, &t).map(|()| t));
    match created {
        Ok(t) => {
            let id = t["id"].as_str().unwrap_or("").to_string();
            q["answer"] = json!(format!("Approved and created task:{id} (Planning)"));
            blocking.then_some(id)
        }
        Err(e) => {
            q["answer"] = json!(format!("Not applied: {}", e.0));
            None
        }
    }
}
