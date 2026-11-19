#[cfg(test)] use crate::cli::routing::unknown_option;
#[cfg(test)] use crate::cli::routing::probe_args;
#[cfg(test)] use crate::cli::routing::next_step;
/// A read command given an option it doesn't take is refused by name.
#[test]
fn an_option_a_command_doesnt_take_is_refused_by_name() {
    let e = unknown_option(&["view", "strategy.update", "--lines", "50..80"]).unwrap().unwrap_err().0;
    assert!(e.starts_with("view has no --lines; --lines belongs to inspect and read. view takes: --bodies, --header"), "{e}");
    let e = unknown_option(&["view", "strategy.update", "--context"]).unwrap().unwrap_err().0;
    assert!(e.contains("`--context` belongs to `inspect`: `inspect <name> --context`"), "{e}");
    assert!(unknown_option(&["read", "a.md", "--lines", "1..2"]).is_none());
    assert!(unknown_option(&["inspect", "a.b", "--context"]).is_none());
    assert!(unknown_option(&["inspect", "a.b", "--lines", "1..2"]).is_none());
    assert!(unknown_option(&["view", "a", "--bodies"]).is_none());
    assert!(unknown_option(&["view", "a", "--header"]).is_none());
    assert!(unknown_option(&["view", "--draft", "x"]).is_none(), "view --draft is in the reference");
    assert!(unknown_option(&["grep", "--x"]).is_none());
}
#[test]
fn probe_args_parse() {
    assert_eq!(probe_args(&["--from", "t", "--trace", "a.b", "c"]).unwrap(), (Some("t"), vec!["a.b", "c"], None));
    assert_eq!(probe_args(&["--in", "parser"]).unwrap(), (None, vec![], Some("parser")));
    assert_eq!(probe_args(&[]).unwrap(), (None, vec![], None));
    assert!(
        probe_args(&["--trace"]).is_err()
            && probe_args(&["--from"]).is_err()
            && probe_args(&["--in"]).is_err()
            && probe_args(&["x"]).is_err()
    );
}

/// D-184: a pass in a work run names the next step, with a test first for a change no test reaches.
#[test]
fn a_pass_names_the_next_step() {
    assert_eq!(next_step(None, true), "Next: if every part of the task is changed, finish with task done.\n");
    assert_eq!(
        next_step(Some("app.two"), true),
        "Next: add a test for app.two; then, if every part of the task is changed, finish with task done.\n"
    );
    assert_eq!(next_step(None, false), "");
    assert_eq!(next_step(Some("app.two"), false), "");
}

/// A check whose task stays missing is refused after the lookup retries.
#[test]
fn a_check_still_reaches_its_project_when_the_task_lookup_misses() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"settings\"]\n"),
        ("settings/gleam.toml", "name = \"settings\"\nversion = \"1.0.0\"\ntarget = \"javascript\"\n"),
    ]);
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("gone", "r1", || crate::cli::projects::scoped("settings", &["test".to_string()]))
    });
    assert_eq!(found.unwrap_err().0, "E7001 unknown task");
}

/// A check reaches its project when the task appears while the lookup retries.
#[test]
fn a_task_that_appears_during_the_lookup_retry_reaches_its_project() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"settings\"]\n"),
        ("settings/gleam.toml", "name = \"settings\"\nversion = \"1.0.0\"\ntarget = \"javascript\"\n"),
    ]);
    let dir = crate::board::store_dir(&root);
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t1.json"), r#"{"id":"t1","column":"backlog","home":".","scope":[".","settings"],"runs":[]}"#).unwrap();
    });
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("t1", "r1", || crate::cli::projects::scoped("settings", &["test".to_string()]))
    });
    writer.join().unwrap();
    assert_eq!(found.unwrap(), Some(root.join("settings")));
}

#[test]
fn a_project_outside_a_tasks_scope_names_it_once() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
    ]);
    let dir = crate::board::store_dir(&root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.json"), r#"{"id":"t1","column":"backlog","home":".","scope":[".","a"],"runs":[]}"#).unwrap();
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("t1", "r1", || crate::cli::projects::scoped("b", &["test".to_string()]))
    });
    let e = found.unwrap_err().0;
    assert!(e.contains("not one of this task's projects (., a). To change a project"), "{e}");
    assert_eq!(e.matches("add it").count(), 1, "{e}");
    assert!(!e.contains("if the task needs it"), "{e}");
}

fn design_test_root() -> (crate::testing::Scratch, String) {
    let root = crate::testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#)]);
    crate::board::task_model::open(&root).unwrap();
    crate::board::task_model::adopt_tasks(&root, &root).unwrap();
    let config = || -> Result<serde_json::Value, crate::failure::Failure> {
        Ok(serde_json::json!({
            "models": [{"id":"fake","label":"Fake","agent":"codex","run":["sh"]}],
            "role_models": {"designer": {"default": "fake"}}
        }))
    };
    let rid = crate::tasks::strategy::design_runs_with(&root, "POST", &[], serde_json::json!({"text":"Design it"}), config).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    (root, rid)
}

/// A show that fails records no read, so a change without a successful read is refused.
#[test]
fn a_failed_design_show_does_not_count_as_a_read() {
    let (root, rid) = design_test_root();
    let id = "0123456789abcdef";
    let design = serde_json::json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": id,
        "title": "Home",
        "tab": "ui",
        "elements": [{"id": "root", "type": "screen", "properties": {"width": "fill", "height": "fill"}}]
    });
    crate::design::save(&root, &design, crate::design::Source::Human).unwrap();
    let show: Vec<String> = ["design", "show", id, "--from", "nope"].iter().map(|s| s.to_string()).collect();
    let e = super::with_root(&root, || super::design_run_at(&root, &rid, &show, &mut || Ok(String::new()), 4000)).unwrap_err().0;
    assert!(e.contains("has no element `nope`"), "{e}");
    let propose: Vec<String> = ["design", "propose", id].iter().map(|s| s.to_string()).collect();
    let text =
        serde_json::json!({"operations": [{"op":"set","id":"root","field":"properties","property":"width","value":"hug"}]}).to_string();
    let e = super::with_root(&root, || super::design_run_at(&root, &rid, &propose, &mut || Ok(text.clone()), 4000)).unwrap_err().0;
    assert!(e.starts_with("E7026") && e.contains("design show"), "{e}");
}

/// `design show --project <folder> <id>` records the design, not the option.
#[test]
fn a_design_show_with_project_before_its_id_records_the_id() {
    let (root, rid) = design_test_root();
    let id = "0123456789abcdef";
    let design = serde_json::json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": id,
        "title": "Home",
        "tab": "ui",
        "elements": [{"id": "root", "type": "screen", "properties": {"width": "fill", "height": "fill"}}]
    });
    crate::design::save(&root, &design, crate::design::Source::Human).unwrap();
    let show: Vec<String> = ["design", "show", "--project", ".", id].iter().map(|s| s.to_string()).collect();
    let shown = super::with_root(&root, || super::design_run_at(&root, &rid, &show, &mut || Ok(String::new()), 4000)).unwrap();
    assert!(shown.contains(id), "{shown}");
    let propose: Vec<String> = ["design", "propose", id].iter().map(|s| s.to_string()).collect();
    let text =
        serde_json::json!({"operations": [{"op":"set","id":"root","field":"properties","property":"width","value":"hug"}]}).to_string();
    let reply = super::with_root(&root, || super::design_run_at(&root, &rid, &propose, &mut || Ok(text.clone()), 4000)).unwrap();
    assert_eq!(reply.trim(), "Saved version 2");
    let versions = crate::design::versions(&root, id).unwrap();
    assert_eq!(versions[0]["source"]["kind"], "run");
    assert_eq!(versions[0]["source"]["id"].as_str(), Some(rid.as_str()));
}

/// A design run can ask for the whole design format.
#[test]
fn a_design_run_reads_the_design_format() {
    let (root, rid) = design_test_root();
    let args: Vec<String> = ["design", "schema"].iter().map(|s| s.to_string()).collect();
    let shown = super::with_root(&root, || super::design_run_at(&root, &rid, &args, &mut || Ok(String::new()), 32_000)).unwrap();
    assert!(shown.contains("Element types") && shown.contains("Proposal operations"), "{shown}");
}

/// A design run can try a change without saving a version.
#[test]
fn a_design_run_checks_without_saving() {
    let (root, rid) = design_test_root();
    let id = "0123456789abcdef";
    let design = serde_json::json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": id,
        "title": "Home",
        "tab": "ui",
        "elements": [{"id": "root", "type": "screen", "properties": {"width": "fill", "height": "fill"}}]
    });
    crate::design::save(&root, &design, crate::design::Source::Human).unwrap();
    let versions = crate::design::versions(&root, id).unwrap();
    let show: Vec<String> = ["design", "show", id].iter().map(|s| s.to_string()).collect();
    super::with_root(&root, || super::design_run_at(&root, &rid, &show, &mut || Ok(String::new()), 4000)).unwrap();
    let check: Vec<String> = ["design", "check", id].iter().map(|s| s.to_string()).collect();
    let text =
        serde_json::json!({"operations": [{"op":"set","id":"root","field":"properties","property":"width","value":"hug"}]}).to_string();
    let reply = super::with_root(&root, || super::design_run_at(&root, &rid, &check, &mut || Ok(text.clone()), 4000)).unwrap();
    assert_eq!(reply.trim(), "Would set root (screen) properties.width to \"hug\"");
    assert_eq!(crate::design::show(&root, id).unwrap(), design);
    assert_eq!(crate::design::versions(&root, id).unwrap(), versions);
}

