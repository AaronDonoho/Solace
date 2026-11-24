use super::*;

#[test]
fn an_entry_goes_after_what_the_log_holds() {
    let root = crate::testing::Scratch::new();
    fs::write(file(&root), "# System reviews\n\n## Earlier\nKept.").unwrap();
    append(&root, "## Security, 2026-11-22\nTasks: t1\n").unwrap();
    assert_eq!(fs::read_to_string(file(&root)).unwrap(), "# System reviews\n\n## Earlier\nKept.\n\n## Security, 2026-11-22\nTasks: t1\n");
}

#[test]
fn appending_to_a_board_with_no_log_creates_it_first() {
    let root = crate::testing::Scratch::new();
    append(&root, "## Code quality, 2026-11-22").unwrap();
    let text = fs::read_to_string(file(&root)).unwrap();
    assert!(text.starts_with(HEADER) && text.ends_with("\n\n## Code quality, 2026-11-22\n"), "{text}");
}

#[test]
fn ensure_keeps_an_existing_log() {
    let root = crate::testing::Scratch::new();
    fs::write(file(&root), "# System reviews\n\n## Kept\n").unwrap();
    ensure(&root).unwrap();
    assert_eq!(fs::read_to_string(file(&root)).unwrap(), "# System reviews\n\n## Kept\n");
}

/// A project at the root, with nothing reviewed yet.
fn reviewed() -> crate::testing::Project {
    crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")])
}

/// A new system review run of `kinds` on a fake reviewer model, its agent not started.
fn review(top: &Path, kinds: &[&str]) -> String {
    let config = top.join("review-config.toml");
    fs::write(
        &config,
        "[[models]]\nid = \"fake\"\nlabel = \"Fake\"\nagent = \"codex\"\nrun = [\"sh\"]\n\n[role_models.reviewer]\ndefault = \"fake\"\n",
    )
    .unwrap();
    let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    let run = crate::board::setup::with_config(&config, || begin(top, &kinds).unwrap());
    run["id"].as_str().unwrap().to_string()
}

/// The agent's `review findings` in run `rid`.
fn report(top: &Path, rid: &str, findings: Value) -> Out {
    let mut input = move || -> Out { Ok(findings.to_string()) };
    command(top, rid, &["review", "findings"], &mut input)
}

/// The tasks run `rid` filed, in the order it lists them.
fn findings(top: &Path, rid: &str) -> Vec<Value> {
    let run = crate::tasks::strategy::strategy_run(top, rid).unwrap();
    let all = crate::board::task_model::all_tasks(top).unwrap();
    run["findings"].as_array().unwrap().iter().map(|id| all.iter().find(|t| t["id"] == *id).unwrap().clone()).collect()
}

#[test]
fn findings_land_in_planning_under_the_findings_feature_and_are_logged() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security", "code-quality"]);
    let out = report(
        &top,
        &rid,
        json!([
            {"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."},
            {"kind": "code-quality", "title": "Split the server", "body": "`server.api` is too long.", "scope": ["."]}
        ]),
    )
    .unwrap();
    let tasks = findings(&top, &rid);
    let feature = crate::tasks::records(&top, "features").into_iter().find(|f| f["findings"] == true).unwrap();
    assert_eq!(feature["state"], "active");
    let titles: Vec<&str> = tasks.iter().map(|t| t["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Check the token", "Split the server"]);
    for t in &tasks {
        assert_eq!((t["column"].as_str(), &t["feature"]), (Some("planning"), &feature["id"]));
        assert!(out.contains(&format!("task:{}", t["id"].as_str().unwrap())), "{out}");
    }
    assert!(tasks[0]["notes"].to_string().contains(&format!("Found by the security review of system review run {rid}")));
    let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
    assert_eq!((run["state"].as_str(), run["outcome"].as_str()), (Some("ended"), Some("finished")));
    let logged = format!(
        "## {}: Security, Code quality\n\n- Version: 1.0.0\n- Findings: task:{}, task:{}\n",
        crate::decisions::today(),
        tasks[0]["id"].as_str().unwrap(),
        tasks[1]["id"].as_str().unwrap()
    );
    let log = fs::read_to_string(file(&top)).unwrap();
    assert!(log.ends_with(&logged), "{log}");
}

/// D-128 point 3 and D-134: a completed findings feature takes the next
/// review's findings, in Planning, and is active again.
#[test]
fn findings_after_the_feature_completed_go_to_it_and_make_it_active() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let mut done = crate::tasks::strategy_api(&top, "POST", &["features"], json!({"title": "Findings", "state": "complete"})).unwrap();
    done["findings"] = json!(true);
    crate::tasks::store::put_record(&top, "features", &done).unwrap();
    let rid = review(&top, &["user-experience"]);
    report(&top, &rid, json!([{"kind": "user-experience", "title": "Name the button", "body": "The Save button has no label."}])).unwrap();
    let tasks = findings(&top, &rid);
    assert_eq!((tasks[0]["feature"].clone(), tasks[0]["column"].as_str()), (done["id"].clone(), Some("planning")));
    assert_eq!(crate::tasks::record(&top, "features", done["id"].as_str().unwrap()).unwrap()["state"], "active");
}

#[test]
fn a_canceled_findings_feature_is_not_reused() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let mut gone = crate::tasks::strategy_api(&top, "POST", &["features"], json!({"title": "Findings", "state": "canceled"})).unwrap();
    gone["findings"] = json!(true);
    crate::tasks::store::put_record(&top, "features", &gone).unwrap();
    let rid = review(&top, &["security"]);
    report(&top, &rid, json!([{"kind": "security", "title": "Check the token", "body": "Any token passes."}])).unwrap();
    let feature = crate::tasks::record(&top, "features", findings(&top, &rid)[0]["feature"].as_str().unwrap()).unwrap();
    assert_ne!(feature["id"], gone["id"]);
    assert_eq!((feature["findings"].as_bool(), feature["state"].as_str()), (Some(true), Some("active")));
}

#[test]
fn a_review_with_no_findings_is_logged_with_none() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security"]);
    let out = report(&top, &rid, json!([])).unwrap();
    assert!(out.starts_with("No findings were filed."), "{out}");
    assert!(crate::tasks::records(&top, "features").iter().all(|f| f["findings"] != true), "no feature without findings");
    let log = fs::read_to_string(file(&top)).unwrap();
    assert!(log.ends_with(&format!("## {}: Security\n\n- Version: 1.0.0\n- Findings: none\n", crate::decisions::today())), "{log}");
    assert_eq!(crate::tasks::strategy::strategy_run(&top, &rid).unwrap()["state"], "ended");
}

/// One finding outside the review's kinds, or without a body, files none of
/// them and writes no entry, so the agent can resend.
#[test]
fn a_wrong_finding_files_nothing_and_keeps_the_run_open() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security"]);
    let before = crate::board::task_model::all_tasks(&top).unwrap().len();
    for findings in [
        json!([
            {"kind": "security", "title": "Fine", "body": "Fine."},
            {"kind": "user-experience", "title": "Other", "body": "Other."}
        ]),
        json!([{"kind": "security", "title": "Fine", "body": " "}]),
        json!([{"kind": "security", "title": "Fine", "body": "Fine.", "scope": ["nowhere"]}]),
        json!({"kind": "security"}),
    ] {
        let e = report(&top, &rid, findings).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{}", e.0);
    }
    assert_eq!(crate::board::task_model::all_tasks(&top).unwrap().len(), before);
    assert!(!file(&top).exists() || !fs::read_to_string(file(&top)).unwrap().contains("Findings:"));
    assert_eq!(crate::tasks::strategy::strategy_run(&top, &rid).unwrap()["state"], "running");
}

#[test]
fn a_review_records_its_kinds_once_and_the_version_it_reviews() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    fs::write(top.join("gleam.toml"), "name = \"app\"\nversion = \"1.2.0\"\n").unwrap();
    let kinds = requested_kinds(&json!({"kinds": ["security", "security", "code-quality"]})).unwrap();
    assert_eq!(kinds, ["security", "code-quality"]);
    let rid = review(&top, &["security", "code-quality"]);
    let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
    assert_eq!(run["kind"], crate::board::roles::SYSTEM_REVIEW_RUN);
    assert_eq!((run["context"]["kinds"].clone(), run["context"]["version"].as_str()), (json!(["security", "code-quality"]), Some("1.2.0")));
    assert!(top.join(format!(".solace/system-reviews/{rid}.json")).exists());
    let b = brief(&top, &run, "engineer", "", "");
    assert!(b.contains("for: Security, Code quality") && b.contains("one of security, code-quality"), "{b}");
    assert!(b.contains(crate::board::briefs::FINDINGS_ASK) && b.contains("`review findings`"), "{b}");
}

#[test]
fn an_entry_without_a_version_says_none_is_named() {
    let root = crate::testing::Scratch::new();
    assert_eq!(reviewed_version(&root), None);
    let kinds = vec!["user-experience".to_string()];
    assert_eq!(entry("2026-11-22", &kinds, None, &[]), "## 2026-11-22: User experience\n\n- Version: none named\n- Findings: none\n");
}

#[test]
fn the_kinds_the_page_sends_are_known() {
    // The System Review page's start body, as `sysreview.start_body` writes it for every kind.
    let body = serde_json::json!({"kinds": ["security", "code-quality", "user-experience"]});
    assert_eq!(requested_kinds(&body).unwrap(), ["security", "code-quality", "user-experience"]);
}

#[test]
fn past_reviews_are_the_finished_ones_newest_first() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let at = |rid: &str, started: u64, ended: u64| {
        let mut run = crate::tasks::strategy::strategy_run(&top, rid).unwrap();
        run["started"] = json!(started);
        run["ended"] = json!(ended);
        crate::tasks::strategy::put_run(&top, &run).unwrap();
    };
    let older = review(&top, &["security"]);
    report(&top, &older, json!([])).unwrap();
    at(&older, 100, 150);
    let newer = review(&top, &["security", "code-quality"]);
    report(&top, &newer, json!([{"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."}])).unwrap();
    at(&newer, 200, 260);
    review(&top, &["user-experience"]);
    let board = std::sync::Arc::new(crate::board::server::Board::new(top.clone(), 0));
    let listed = board.projects_api("GET", "/system-reviews", &json!(null)).unwrap();
    let task = findings(&top, &newer)[0]["id"].clone();
    assert_eq!(
        listed,
        json!([
            {"id": newer, "started": 200, "ended": 260, "kinds": ["security", "code-quality"], "version": "1.0.0", "findings": [task]},
            {"id": older, "started": 100, "ended": 150, "kinds": ["security"], "version": "1.0.0", "findings": []}
        ])
    );
}

#[test]
fn the_log_reads_through_its_route() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security"]);
    report(&top, &rid, json!([])).unwrap();
    let board = std::sync::Arc::new(crate::board::server::Board::new(top.clone(), 0));
    let log = board.projects_api("GET", "/system-reviews/log", &json!(null)).unwrap();
    let text = log["text"].as_str().unwrap();
    assert_eq!(text, std::fs::read_to_string(file(&top)).unwrap());
    assert!(text.starts_with("# System reviews\n"), "{text}");
    assert!(text.contains(": Security\n\n- Version: 1.0.0\n- Findings: none\n"), "{text}");
}

/// The log can't take the entry: the agent is told so, the run stays open
/// with its findings filed, and a retry logs them once without filing again.
#[test]
fn a_failed_log_write_keeps_the_run_open_and_a_retry_logs_it_once() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security"]);
    let _ = fs::remove_file(file(&top));
    fs::create_dir_all(file(&top)).unwrap();
    let sent = json!([{"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."}]);
    let e = report(&top, &rid, sent.clone()).unwrap_err();
    assert!(!e.0.contains("logged"), "{}", e.0);
    assert_eq!(crate::tasks::strategy::strategy_run(&top, &rid).unwrap()["state"], "running");
    let filed = findings(&top, &rid);
    assert_eq!(filed.len(), 1);
    let before = crate::board::task_model::all_tasks(&top).unwrap().len();
    fs::remove_dir_all(file(&top)).unwrap();
    let out = report(&top, &rid, sent).unwrap();
    let id = filed[0]["id"].as_str().unwrap();
    assert!(out.contains(&format!("task:{id}")) && out.contains("logged in SYSTEMREVIEWS.md"), "{out}");
    assert_eq!(crate::board::task_model::all_tasks(&top).unwrap().len(), before);
    let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
    assert_eq!((run["state"].as_str(), run["outcome"].as_str()), (Some("ended"), Some("finished")));
    let log = fs::read_to_string(file(&top)).unwrap();
    assert_eq!(log.matches("- Findings:").count(), 1, "{log}");
    assert!(log.ends_with(&format!("- Findings: task:{id}\n")), "{log}");
}

#[test]
fn a_review_that_ended_is_not_filed_or_logged_again() {
    let p = reviewed();
    let top = crate::scope::top(&p.root);
    let rid = review(&top, &["security"]);
    let sent = json!([{"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."}]);
    report(&top, &rid, sent.clone()).unwrap();
    let before = crate::board::task_model::all_tasks(&top).unwrap().len();
    let out = report(&top, &rid, sent).unwrap();
    assert!(out.contains("logged in SYSTEMREVIEWS.md"), "{out}");
    assert_eq!(crate::board::task_model::all_tasks(&top).unwrap().len(), before);
    assert_eq!(findings(&top, &rid).len(), 1);
    assert_eq!(fs::read_to_string(file(&top)).unwrap().matches("- Findings:").count(), 1);
}

/// The findings feature can't be saved as the first finding reactivates it:
/// the task already written is rolled back, and nothing is logged.
#[test]
fn a_failed_task_write_files_none_and_logs_nothing() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let mut done = crate::tasks::strategy_api(&top, "POST", &["features"], json!({"title": "Findings", "state": "complete"})).unwrap();
        done["findings"] = json!(true);
        crate::tasks::store::put_record(&top, "features", &done).unwrap();
        let rid = review(&top, &["security"]);
        let before = crate::board::task_model::all_tasks(&top).unwrap().len();
        let features = crate::tasks::strategy::strategy_dir(&top, "features");
        fs::set_permissions(&features, fs::Permissions::from_mode(0o555)).unwrap();
        let sent = json!([
            {"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."},
            {"kind": "security", "title": "Check the key", "body": "`auth.key` is logged."}
        ]);
        let failed = report(&top, &rid, sent);
        fs::set_permissions(&features, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(failed.is_err());
        assert_eq!(crate::board::task_model::all_tasks(&top).unwrap().len(), before);
        let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
        assert_eq!((run["state"].as_str(), &run["findings"]), (Some("running"), &Value::Null));
        assert!(!file(&top).exists() || !fs::read_to_string(file(&top)).unwrap().contains("Findings:"));
    }
}

#[test]
fn the_review_log_is_readable_and_only_solace_writes_it() {
    let s = crate::testing::project(&[("src/main.gleam", "pub fn main() { 1 }\n")]);
    let v: toml::Value = "watch_limit = 100\n".parse().unwrap();
    fs::write(s.root.join("SYSTEMREVIEWS.md"), "# System reviews\n").unwrap();
    assert_eq!(crate::lockdown::paths::tier(&s.root, &v, "SYSTEMREVIEWS.md", None), "protected");
    assert!(!crate::lockdown::settings::settings_file(&s.root, "SYSTEMREVIEWS.md"), "a write is refused, not proposed");
    assert!(crate::testing::dispatch(&s, &v, &["read", "SYSTEMREVIEWS.md"], "").unwrap().contains("# System reviews"));
    let patch = "*** Begin Patch\n*** Update File: SYSTEMREVIEWS.md\n@@\n-# System reviews\n+# Forged\n*** End Patch\n";
    for (args, text) in [
        (vec!["write", "SYSTEMREVIEWS.md"], "# Forged\n"),
        (vec!["write"], patch),
        (vec!["remove", "SYSTEMREVIEWS.md"], ""),
        (vec!["move", "SYSTEMREVIEWS.md", "priv/reviews.md"], ""),
    ] {
        let e = crate::testing::dispatch(&s, &v, &args, text).unwrap_err();
        assert!(e.0.contains("only Solace writes this file"), "{args:?}: {}", e.0);
    }
    assert_eq!(fs::read_to_string(s.root.join("SYSTEMREVIEWS.md")).unwrap(), "# System reviews\n");
    assert!(!s.root.join("priv/reviews.md").exists());
}

/// A Gleam project's files in folder `d` of `root`, with no store yet.
fn gleam_folder(root: &Path, d: &str) {
    fs::create_dir_all(root.join(d).join("src")).unwrap();
    fs::write(root.join(d).join("gleam.toml"), format!("name = \"{}\"\n", d.replace(['/', ' ', '.'], "_"))).unwrap();
}

#[test]
fn setting_up_creates_the_review_log_and_keeps_it() {
    let root = crate::testing::Scratch::new();
    gleam_folder(&root, ".");
    crate::workspace::setup(&root, ".").unwrap();
    let log = root.join("SYSTEMREVIEWS.md");
    assert!(fs::read_to_string(&log).unwrap().starts_with("# System reviews\n"), "a new root gets the log");
    fs::write(&log, "# System reviews\n\n## Kept\n").unwrap();
    gleam_folder(&root, "frontend");
    crate::workspace::setup(&root, "frontend").unwrap();
    assert_eq!(fs::read_to_string(&log).unwrap(), "# System reviews\n\n## Kept\n", "adding a project keeps the log");
}

#[test]
fn adding_a_project_to_a_root_without_the_review_log_creates_it() {
    let root = crate::testing::Scratch::new();
    gleam_folder(&root, "frontend");
    crate::workspace::setup(&root, "frontend").unwrap();
    assert!(fs::read_to_string(root.join("SYSTEMREVIEWS.md")).unwrap().starts_with("# System reviews\n"));
    assert!(!root.join("frontend/SYSTEMREVIEWS.md").exists(), "one log, at the top folder");
}

#[test]
fn reading_the_review_log_from_a_member_reaches_the_top() {
    let root = crate::testing::Scratch::new();
    gleam_folder(&root, ".");
    gleam_folder(&root, "frontend");
    fs::write(root.join("frontend/src/app.gleam"), "pub fn main() { 1 }\n").unwrap();
    crate::workspace::setup(&root, "frontend").unwrap();
    fs::write(root.join("SYSTEMREVIEWS.md"), "# System reviews\n\n## Security\n").unwrap();
    let s = crate::Store::peek(&root.join("frontend")).unwrap();
    let v = crate::lockdown::settings(&root).unwrap();
    let out = crate::testing::dispatch(&s, &v, &["read", "SYSTEMREVIEWS.md"], "").unwrap();
    assert!(out.contains("## Security"), "{out}");
}
