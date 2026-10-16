use super::*;

/// A task with one run `r1` in `run`'s state, and a hung agent in its own process group.
fn supervised(run: Value) -> (testing::Project, String, std::process::Child) {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Supervise"})).unwrap();
    t["runs"] = json!([run]);
    tasks::put(&s, &t).unwrap();
    let child = Command::new("sleep").arg("45").process_group(0).spawn().unwrap();
    (p, t["id"].as_str().unwrap().to_string(), child)
}

/// D-78: the file is the record, so a log that can't take the change doesn't fail it.
#[test]
fn a_task_is_written_when_the_log_cannot_take_it() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Logged"})).unwrap();
    let log = store_dir(&scope::top(&s.root)).join("events.db");
    fs::remove_file(&log).unwrap();
    fs::create_dir(&log).unwrap();
    t["title"] = json!("Edited");
    tasks::put(&s, &t).unwrap();
    assert_eq!(task(&s.root, t["id"].as_str().unwrap()).unwrap()["title"], "Edited");
}

/// D-129: a home the root lists is a project, one it doesn't is not, and a
/// project list that can't be read counts every home as a project.
#[test]
fn a_home_is_a_project_unless_the_root_lists_otherwise() {
    let root = testing::Scratch::new();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::write(root.join("solace.toml"), "projects = [\"a\"]\n").unwrap();
    assert!(!homeless(&root, "a"));
    assert!(homeless(&root, "b"));
    fs::write(root.join("solace.toml"), "projects = 5\n").unwrap();
    assert!(!homeless(&root, "b"));
}

/// D-91: investigate, work and discuss briefs carry the vision, cut at a line
/// end within the cap; a review's doesn't, and no vision adds nothing.
#[test]
fn a_brief_carries_the_vision_up_to_the_cap() {
    let root = testing::Scratch::new();
    assert_eq!(vision_section(&root, "work"), "");
    let line = format!("{}\n", "v".repeat(99));
    let long = line.repeat(12);
    fs::create_dir_all(root.join(".solace/strategy")).unwrap();
    fs::write(root.join(".solace/strategy/vision.md"), &long).unwrap();
    for kind in ["investigate", "work", "discuss"] {
        let s = vision_section(&root, kind);
        assert!(s.starts_with("\nVision ("), "{kind}: {s}");
        assert_eq!(s.matches(&"v".repeat(99)).count(), 10, "{kind}: ten whole lines fit in {VISION_CAP} characters");
        assert!(s.contains("(cut short;"), "{kind}");
    }
    assert_eq!(vision_section(&root, "review"), "");
    fs::write(root.join(".solace/strategy/vision.md"), "For teams.\n").unwrap();
    assert!(vision_section(&root, "work").ends_with(":\nFor teams.\n"));
}

/// D-27: a run's brief lists the skills of the project it works in — name,
/// description and the path to `read` — for every kind; a project with no
/// skills has no Skills section, and the brief carries no skill contents.
#[test]
fn a_brief_lists_the_skills_of_its_project() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("skills/design/SKILL.md", "---\nname: design\ndescription: Layout and color.\n---\n# Design\n\nPick a palette.\n"),
        ("skills/voice.md", "Speak plainly\n"),
    ]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    for kind in ["work", "investigate", "discuss", "review"] {
        let run = json!({"id":"r1","kind":kind});
        let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false).0;
        assert!(brief.contains("- design: Layout and color. (`read skills/design/SKILL.md`)"), "{kind}: {brief}");
        assert!(brief.contains("- voice: Speak plainly (`read skills/voice.md`)"), "{kind}: {brief}");
        assert!(brief.contains("when the task matches a skill's description"), "{kind}: {brief}");
        assert!(brief.contains("After the call that ends the run returns, stop: no more calls"), "{kind}: {brief}");
        assert!(!brief.contains("finishing tool"), "{kind}: a discussion's first note is not its end: {brief}");
        assert!(!brief.contains("Pick a palette"), "{kind}: the brief carries no skill contents");
    }
    let q = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let u = open(&q.root).unwrap();
    let t2 = api(&u.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let brief = continuation_brief(&t2, &json!({"id":"r1","kind":"work"}), false, &q.root, &q.root, false).0;
    assert!(!brief.contains("Skills ("), "{brief}");
}

/// D-138: a task's feature section names the feature and carries the latest
/// of its investigation report, assignment note and review feedback, cut at
/// a line end within the cap; no feature, no section.
#[test]
fn a_brief_carries_the_features_latest_direction() {
    let top = testing::Scratch::new();
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1 .0).unwrap();
    let runs = json!([{"id":"i1","kind":"investigate"},{"id":"w1","kind":"work"},{"id":"r1","kind":"review"},{"id":"d1","kind":"discuss"}]);
    let mut t = json!({"id":"t","runs":runs,"notes":[]});
    assert_eq!(feature_section(&top, &t), "");
    t["feature"] = json!("gone");
    assert_eq!(feature_section(&top, &t), "");
    t["feature"] = f["id"].clone();
    assert_eq!(feature_section(&top, &t), "\nFeature (the feature this task is part of; its latest direction):\n- Search\n");
    let note = |author: &str, run: Value, text: &str| json!({"author":author,"run":run,"text":text});
    t["notes"] = json!([note("agent", json!("i1"), "plan"), note("solace", Value::Null, "exited")]);
    assert!(feature_section(&top, &t).ends_with("- Search\n- investigation report (run i1): plan\n"));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, "go"));
    t["notes"].as_array_mut().unwrap().push(note("agent", json!("w1"), "done"));
    assert!(feature_section(&top, &t).ends_with("- assignment: go\n"));
    t["notes"].as_array_mut().unwrap().push(note("agent", json!("r1"), "looks good"));
    t["notes"].as_array_mut().unwrap().push(note("human", json!("d1"), "chat"));
    assert!(feature_section(&top, &t).ends_with("- review feedback (run r1): looks good\n"));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, "  "));
    assert!(feature_section(&top, &t).ends_with("- review feedback (run r1): looks good\n"), "an empty note is skipped");
    let line = format!("{}\n", "n".repeat(99));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, &line.repeat(12)));
    let s = feature_section(&top, &t);
    assert_eq!(s.matches(&"n".repeat(99)).count(), 10, "ten whole lines fit in {FEATURE_CAP} characters");
    assert!(s.ends_with("\n(cut short; the whole note is in the task's notes on the board)\n"), "{s}");
}

/// D-112: a `### Tech debt` section gives one finding per top-level bullet,
/// with its indented lines, and ends at the next heading.
#[test]
fn tech_debt_items_are_the_sections_bullets() {
    let note = "**Assessment: Looks good**\n\n- not debt\n\n### Tech debt\n\nIntro line.\n- `app.main`: name the constant\n* `app.other`: long\n  second line\n1. `a.rs`: third\n\n### Verification\n\n- tests pass\n";
    assert_eq!(tech_debt_items(note), vec!["`app.main`: name the constant", "`app.other`: long\n  second line", "`a.rs`: third"]);
    assert_eq!(tech_debt_items("## Tech Debt:\n- one\n"), vec!["one"]);
    assert!(tech_debt_items("### Tech debt\n\n### Next\n- x\n").is_empty());
    assert!(tech_debt_items("**Assessment: Looks good**\n- x\n").is_empty());
}

/// D-112: completing a task in a feature files its last review's tech debt
/// on the feature's open Tech Debt task: made once, appended to after, and
/// made anew once that one is complete. No feature, a cancel, or a review
/// with no section files nothing.
#[test]
fn completing_a_task_files_its_reviews_tech_debt() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1 .0).unwrap();
    let fid = f["id"].as_str().unwrap().to_string();
    let reviewed = |title: &str, feature: bool, text: &str| {
        let id = api(&s.root, "POST", "/tasks", json!({"title":title})).unwrap()["id"].as_str().unwrap().to_string();
        if feature {
            api(&s.root, "PATCH", &format!("/tasks/{id}"), json!({"feature":fid})).unwrap();
        }
        let mut t = task(&s.root, &id).unwrap();
        let rid = format!("rv{title}");
        t["runs"].as_array_mut().unwrap().push(json!({"id":rid,"kind":"review","state":"ended","outcome":"reviewed"}));
        note(&mut t, "agent", Some(&rid), text);
        tasks::put(&s, &t).unwrap();
        api(&s.root, "POST", &format!("/tasks/{id}/actions"), json!({"action":"plan"})).unwrap();
        id
    };
    let act = |id: &str, action: &str| api(&s.root, "POST", &format!("/tasks/{id}/actions"), json!({"action":action})).unwrap();
    let debts = || all_tasks(&top).unwrap().into_iter().filter(|t| t["techDebt"] == true).collect::<Vec<_>>();
    let texts = |t: &Value| t["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let two = "**Assessment: Looks good**\n\n### Tech debt\n\n- `app.main`: name the constant\n- `app.other`: split it\n\n### Verification\n\n- tests pass\n";
    let a = reviewed("A", true, two);
    let done = act(&a, "complete");
    let [debt] = debts().try_into().unwrap();
    let did = debt["id"].as_str().unwrap().to_string();
    assert_eq!((debt["title"].as_str(), debt["column"].as_str(), debt["feature"].as_str()), (Some("Search: Tech Debt"), Some("planning"), Some(fid.as_str())));
    assert_eq!(debt["scope"], done["scope"]);
    let notes = texts(&debt);
    assert_eq!(notes.len(), 2);
    assert!(notes[0].contains(&format!("task:{a}")) && notes[0].contains("rvA") && notes[0].ends_with("`app.main`: name the constant"), "{notes:?}");
    assert!(notes[1].ends_with("`app.other`: split it"));
    assert!(texts(&done).last().unwrap().contains(&format!("Filed 2 tech-debt items from the last review on task:{did}")));
    // A second task in the feature appends to the same Tech Debt task.
    let b = reviewed("B", true, "### Tech debt\n- `app.b`: one more\n");
    act(&b, "complete");
    let [debt] = debts().try_into().unwrap();
    assert_eq!(debt["id"], did.as_str());
    assert!(texts(&debt)[2].contains(&format!("task:{b}")));
    // No feature, a cancel, or no section: nothing is filed.
    act(&reviewed("C", false, two), "complete");
    act(&reviewed("D", true, two), "cancel");
    act(&reviewed("E", true, "**Assessment: Looks good**\n- fine\n"), "complete");
    let [debt] = debts().try_into().unwrap();
    assert_eq!(texts(&debt).len(), 3);
    // A completed Tech Debt task is not reopened: the next finding makes a new one.
    act(&did, "complete");
    act(&reviewed("F", true, two), "complete");
    let open: Vec<Value> = debts().into_iter().filter(|d| d["id"] != did.as_str()).collect();
    assert_eq!(open.len(), 1);
    assert_eq!((open[0]["column"].as_str(), texts(&open[0]).len()), (Some("planning"), 2));
    assert_eq!(task(&s.root, &did).unwrap()["column"], "complete");
}

/// A finished agent that hangs is killed once the grace period passes, not before.
#[test]
fn hung_shutdown_is_killed_after_the_grace_period() {
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"finished"}));
    let log = p.root.join(".solace/r1.log");
    let start = Instant::now();
    let forced = supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, false, Duration::from_secs(3600), Duration::from_millis(300)).unwrap();
    assert!(forced);
    assert!(start.elapsed() >= Duration::from_millis(300) && start.elapsed() < Duration::from_secs(10));
    assert!(child.try_wait().unwrap().is_some());
    assert_eq!(GRACEFUL_EXIT, Duration::from_secs(30));
}

/// A finished agent that exits within the grace period isn't killed.
#[test]
fn graceful_exit_is_not_forced() {
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"finished"}));
    lockdown::kill_group(&mut child);
    let mut child = Command::new("sleep").arg("0.2").process_group(0).spawn().unwrap();
    let log = p.root.join(".solace/r1.log");
    let start = Instant::now();
    assert!(!supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
    assert!(start.elapsed() < Duration::from_secs(10));
}

/// An interrupted run's agent is killed at once; a running one past its timeout ends the run.
#[test]
fn interrupted_and_timed_out_agents_are_killed() {
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"interrupted"}));
    let log = p.root.join(".solace/r1.log");
    let start = Instant::now();
    assert!(supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
    assert!(start.elapsed() < Duration::from_secs(10));
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"running"}));
    let log = p.root.join(".solace/r1.log");
    assert!(supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, false, Duration::from_millis(200), GRACEFUL_EXIT).unwrap());
    assert!(child.try_wait().unwrap().is_some());
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["state"], "ended");
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["text"] == "task timeout"));
}

/// A review or investigation run reads but doesn't change (it may build to
/// verify); host commands never run in a run; an ended run runs nothing.
#[test]
fn agent_guard_follows_the_run() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Look"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"review","state":"running"},{"id":"r2","kind":"work","state":"ended"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let guard = |rid: &str, a: &[&str]| agent_guard(&s.root, tid, rid, a, &[]).map(drop).map_err(|e| e.0);
    assert!(guard("r1", &["view", "app"]).is_ok() && guard("r1", &["build"]).is_ok());
    std::fs::write(
        s.root.join("solace.toml"),
        "[commands]\nsafe = { run = [\"true\"], allowed_in_reviews = true }\ne2e = { run = [\"true\"] }\n",
    )
    .unwrap();
    assert!(guard("r1", &["run", "safe"]).is_ok());
    assert!(guard("r1", &["run", "e2e"]).unwrap_err().contains("allowed_in_reviews"));
    for a in [["submit", "app.main"], ["write", "x"]] {
        assert!(guard("r1", &a).unwrap_err().contains("this run is read-only"), "{a:?}");
    }
    assert!(guard("r1", &["migrate", "--yes"]).unwrap_err().contains("host command"));
    assert!(guard("r2", &["view", "app"]).unwrap_err().contains("run ended"));
    assert!(guard("r9", &["view", "app"]).unwrap_err().starts_with("E7003 unknown run"));
    // A `task` command takes no project lock (D-77 point 5); any other waits for it.
    let held = lock(&s.root).unwrap();
    let waits = |a: &'static [&'static str]| {
        let (root, tid) = (s.root.clone(), tid.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || drop(tx.send(agent_guard(&root, &tid, "r1", a, &[]).map(|g| g.len()).map_err(|e| e.0))));
        rx
    };
    assert_eq!(waits(&["task", "note", "x"]).recv_timeout(Duration::from_secs(1)).unwrap(), Ok(0));
    let view = waits(&["view", "app"]);
    assert!(view.recv_timeout(Duration::from_millis(300)).is_err());
    drop(held);
    assert_eq!(view.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(1));
}

/// D-93: a strategy run's guard takes no project lock and refuses an ended run;
/// `strategy show` follows links; the brief carries the record, decisions and notes.
#[test]
fn strategy_runs_read_the_graph() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("skills/design/SKILL.md", "---\nname: design\ndescription: Layout and color.\n---\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&p.root);
    let runs = top.join(".solace/strategy/runs");
    fs::create_dir_all(&runs).unwrap();
    for (id, state) in [("r1", "running"), ("r2", "ended")] {
        let run = json!({"id":id,"kind":"strategy","state":state,"context":{"kind":"page"},"notes":[],"questions":[]});
        fs::write(runs.join(format!("{id}.json")), run.to_string()).unwrap();
    }
    let held = lock(&p.root).unwrap();
    assert!(strategy_guard(&top, "r1", &["view", "app"]).is_ok());
    assert!(strategy_guard(&top, "r2", &["view", "app"]).unwrap_err().0.starts_with("E7003"));
    assert!(strategy_guard(&top, "r9", &["view", "app"]).unwrap_err().0.starts_with("E7023"));
    drop(held);
    let m = tasks::strategy_api(&top, "POST", &["milestones"], json!({"title":"M"})).unwrap();
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"F","milestone":m["id"]})).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    (t["feature"], t["milestone"]) = (f["id"].clone(), m["id"].clone());
    tasks::put(&s, &t).unwrap();
    let (tid, fid, mid) = (t["id"].as_str().unwrap(), f["id"].as_str().unwrap(), m["id"].as_str().unwrap());
    assert_eq!(strategy_show(&top, "feature", fid).unwrap()["links"], json!({"milestone":mid,"tasks":[tid]}));
    assert_eq!(strategy_show(&top, "milestone", mid).unwrap()["links"], json!({"features":[fid],"tasks":[tid]}));
    assert_eq!(strategy_show(&top, "task", tid).unwrap()["links"], json!({"feature":fid,"milestone":mid,"dependsOn":[],"runs":[]}));
    assert!(strategy_show(&top, "feature", "nope").unwrap_err().0.starts_with("E7023"));
    assert!(strategy_show(&top, "vision", "x").unwrap_err().0.starts_with("E7001"));
    let listed = strategy_list(&top, false).unwrap();
    assert!(listed.contains(&format!("{tid} {} f:{fid} m:{mid} T", t["column"].as_str().unwrap()))
        && listed.contains(&format!("Features:\n{fid} proposed m:{mid} F"))
        && listed.contains(&format!("Milestones:\n{mid} proposed #1 M"))
        && listed.contains("Vision:"), "{listed}");
    fs::write(store_dir(&top).join("t9.json"), json!({"id":"t9","title":"Gone","column":"complete","closedTime":8,"archivedTime":9}).to_string()).unwrap();
    assert!(!strategy_list(&top, false).unwrap().contains("t9") && strategy_list(&top, true).unwrap().contains("t9 complete f:- m:- Gone"));
    fs::write(top.join("DECISIONS.md"), "# Decisions\n## D-1 First thing\nWhy: x\n## Notes\n").unwrap();
    let brief = strategy_brief(&top, &json!({"context":{"kind":"feature","id":fid},"notes":[{"author":"human","text":"Plan F"}]}));
    assert!(brief.contains(&format!("This run is about the feature {fid}")) && brief.contains("\"title\": \"F\""), "{brief}");
    assert!(brief.contains("A `title` names the work in a few words") && brief.contains("go in `body`"), "{brief}");
    assert!(brief.contains("in short Markdown: lead with the answer"), "{brief}");
    assert!(brief.contains("- D-1 First thing") && !brief.contains("- Notes") && brief.contains("- human: Plan F"), "{brief}");
    // The root is a project itself, so the decisions can be read in full.
    assert!(brief.contains("`grep D-<n>`") && !brief.contains("folder of projects"), "{brief}");
    // D-27: the root project's own skills are listed in the brief.
    assert!(brief.contains("- design: Layout and color. (`read skills/design/SKILL.md`)"), "{brief}");
    // Only commands the run can use.
    for word in ["task ask", "submit", "`test`", "session start", "are refused"] {
        assert!(!brief.contains(word), "{word}: {brief}");
    }
    // Only the latest decisions, and how to find the rest.
    let many: String = (1..=14).map(|n| format!("## D-{n} Thing {n}\n")).collect();
    fs::write(top.join("DECISIONS.md"), many).unwrap();
    let brief = strategy_brief(&top, &json!({"context":{"kind":"page"},"notes":[]}));
    assert!(brief.contains("- D-5 Thing 5") && brief.contains("- D-14 Thing 14") && !brief.contains("- D-4 Thing"), "{brief}");
    assert!(brief.contains("… 4 earlier decisions: `grep D-` lists them."), "{brief}");
}

/// D-27: a strategy brief for a folder of projects leaves the root's skills
/// out, as it leaves the root's own files unread.
#[test]
fn a_strategy_brief_leaves_out_a_folder_of_projects_skills() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\"]\n"),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn main() { 1 }\n"),
        ("skills/design/SKILL.md", "---\nname: design\ndescription: Layout and color.\n---\n"),
    ]);
    let brief = strategy_brief(&root, &json!({"context":{"kind":"page"},"notes":[]}));
    assert!(brief.contains("folder of projects"), "{brief}");
    assert!(!brief.contains("Skills ("), "{brief}");
}

/// A recorded file change stops being reviewable once the file is unwatched, and symbols always are.
#[test]
fn reviewable_follows_the_current_tier() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("a.txt", "x")]);
    let s = open(&p.root).unwrap();
    assert!(reviewable(&s.root, "file:a.txt") && reviewable(&s.root, "symbol:1"));
    assert!(!reviewable(&s.root, "file:node_modules/x/index.js"));
    std::fs::write(s.root.join(".gitignore"), "a.txt\n").unwrap();
    assert!(!reviewable(&s.root, "file:a.txt"));
}

/// Records a work run's change, as a command does. The task lists its changes
/// from the committed rows (D-77 point 4), so nothing is written to the task.
fn record_listed(s: &Store, tid: &str, before: &Value, after: &Value) {
    record(s, tid, "work", before, after, None).unwrap();
}

#[test]
fn review_delta_keeps_module_headers_budget_and_completed_baseline() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Review headers"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"work","kind":"work","state":"ended"}]);
    tasks::put(&s, &t).unwrap();
    let before = snapshot(&s).unwrap();
    s.db.execute("UPDATE modules SET header='//// Reviewed header' WHERE path='app'", []).unwrap();
    record_listed(&s, &tid, &before, &snapshot(&s).unwrap());
    t = task(&s.root, &tid).unwrap();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"review","kind":"review","state":"ended","outcome":"reviewed"}));
    let mut checkpoint = review_state(&s, &t).unwrap();
    checkpoint["run"] = json!("review");
    checkpoint["changed_during_review"] = json!(false);
    let path = review_path(&s.root, &tid, "review").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    // A later interrupted run must not displace the completed review.
    t["runs"].as_array_mut().unwrap().push(json!({"id":"interrupted","kind":"review","state":"ended","outcome":"interrupted"}));
    tasks::put(&s, &t).unwrap();
    assert!(task_diff(&s, &tid, &["--since-review"], 1000).unwrap().contains("No task entity changes"));
    s.db.execute("UPDATE modules SET header='//// Repaired header' WHERE path='app'", []).unwrap();
    fs::write(s.root.join("later.txt"), "later").unwrap();
    record_listed(&s, &tid, &json!({"file:later.txt":null}), &json!({"file:later.txt":b"later".to_vec()}));
    let delta = task_diff(&s, &tid, &["--since-review"], 1).unwrap();
    assert!(delta.contains("Since completed review review"));
    assert!(delta.contains("- //// Reviewed header") && delta.contains("+ //// Repaired header"));
    assert!(delta.contains("Left out to fit the reply") && delta.contains("task diff --since-review <name>"));
    let filtered = task_diff(&s, &tid, &["--since-review", "file:later.txt"], 1000).unwrap();
    assert!(filtered.contains("file:later.txt (added)") && !filtered.contains("module:app"));
}

/// Canceling a task puts back what it changed: a module it created goes,
/// with its file.
#[test]
fn cancel_removes_a_module_the_task_created() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Shapes"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    assert!(s.root.join("src/geo.gleam").exists());
    let mut t = task(&s.root, tid).unwrap();
    end_runs(&mut t, "finished");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert!(!s.root.join("src/geo.gleam").exists());
    assert!(s.resolve_module(&s.index().unwrap(), "geo").is_none());
    assert_eq!(snapshot(&s).unwrap(), before);
}

/// Canceling does not delete a created module's file that was edited
/// outside Solace and not yet read in; the next read-in reports it.
#[test]
fn cancel_keeps_an_edited_file_of_a_module_the_task_created() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Shapes"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let file = s.root.join("src/geo.gleam");
    fs::write(&file, "pub fn side() {\n  22\n}\n").unwrap();
    let mut t = task(&s.root, tid).unwrap();
    end_runs(&mut t, "finished");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "pub fn side() {\n  22\n}\n");
}

/// D-77 point 4: a `changes` message left in an old inbox adds nothing. The
/// board_touches rows are the task's changes, so a revert followed by the
/// pending message leaves the task with none.
#[test]
fn a_pending_changes_message_adds_nothing_after_a_revert() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    assert!(!changes_of(&s, &task(&s.root, &tid).unwrap()).unwrap().is_empty());
    // A changes message from before D-77 point 4 waits in the inbox.
    let top = scope::top(&s.root);
    let inbox = store_dir(&top).join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    let id = "0000000000000000000000000001";
    let msg = json!({"id": id, "kind": "changes", "task": tid, "run": "r1",
        "changes": [{"entity": "symbol:1", "name": "geo.side", "run": "r1"}]});
    fs::write(inbox.join(format!("{id}.json")), msg.to_string().as_bytes()).unwrap();
    // Cancel reverts the task: its rows go, so it has no changes. It also clears
    // the task's conversation on a thread of its own; with no task server to
    // order the two, that write and the drain's would race, so the drain waits
    // for it (the conversation gone).
    end_runs(&mut t, "finished");
    t["conversation"] = json!({"home": tid.as_str()});
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert!(changes_of(&s, &task(&s.root, &tid).unwrap()).unwrap().is_empty());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !task(&s.root, &tid).unwrap()["conversation"].is_null() {
        assert!(std::time::Instant::now() < deadline, "the conversation was never cleared");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Applying the pending message adds nothing.
    tasks::drain(&top);
    let t = task(&s.root, &tid).unwrap();
    assert_eq!(t["inbox"], json!(id), "the pending message was applied");
    assert!(!inbox.join(format!("{id}.json")).exists(), "the inbox is drained");
    assert!(changes_of(&s, &t).unwrap().is_empty());
    assert!(t["changes"].is_null(), "{}", t);
}

/// D-77 point 5: only a change to code takes the project locks.
#[test]
fn needs_lock_only_for_changes_to_code() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let needs = |rest: &[&str], body: Value| {
        let parts: Vec<&str> = ["tasks", tid.as_str()].into_iter().chain(rest.iter().copied()).collect();
        needs_lock(&s.root, &parts, &body).unwrap()
    };
    assert!(!needs_lock(&s.root, &["tasks"], &json!({"title": "x"})).unwrap());
    assert!(!needs(&["notes"], json!({"text": "x"})));
    assert!(!needs(&["actions"], json!({"action": "stop"})));
    assert!(!needs(&["actions"], json!({"action": "cancel"})), "a task that never had changes");
    t["questions"] = json!([{"id": "q1", "options": ["Approve", "Reject"], "answer": null,
        "settings": {"file": "solace.toml", "base": "", "proposed": ""}}]);
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&s, &t).unwrap();
    assert!(needs(&["questions", "q1"], json!({"option": "Approve"})));
    assert!(!needs(&["questions", "q1"], json!({"option": "Reject"})));
    assert!(needs(&["actions"], json!({"action": "cancel"})), "a work run may be mid-command");
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    assert!(needs(&["actions"], json!({"action": "cancel"})));
    assert!(!needs(&["actions"], json!({"action": "stop"})));
}

/// A run stopped mid-write leaves a record and no rows; cancelling must still lock and undo.
#[test]
fn a_pending_record_alone_needs_the_locks() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap();
    let cancel = |tid: &str| needs_lock(&s.root, &["tasks", tid, "actions"], &json!({"action": "cancel"})).unwrap();
    assert!(!cancel(tid));
    lockdown::mark(&s.root, "other", "r9").unwrap();
    assert!(!cancel(tid), "another task's record is not this task's");
    lockdown::mark(&s.root, tid, "r1").unwrap();
    assert!(cancel(tid));
}

/// Stop then Cancel while a write was under way: the record is adopted under the
/// locks first, so cancelling restores the file instead of leaving it to recovery.
#[test]
fn cancel_undoes_a_write_a_stopped_run_left_pending() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("notes.txt", "old\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    lockdown::mark(&s.root, &tid, "r1").unwrap();
    fs::write(s.root.join(".solace/pending/notes.json"), json!({"task": tid, "run": "r1", "path": "notes.txt", "bytes": b"old\n".to_vec()}).to_string()).unwrap();
    fs::write(s.root.join("notes.txt"), "new\n").unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert_eq!(fs::read_to_string(s.root.join("notes.txt")).unwrap(), "old\n");
    assert!(!lockdown::pending_for(&s.root, &tid));
}

/// D-108, D-133: a canceled task goes back to planning, out of the archive,
/// with its closedTime and a note; complete offers nothing.
#[test]
fn a_canceled_task_goes_back_to_planning() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let act = |name: &str| api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": name}));
    let refused = |r: Result<Value, Failure>| r.unwrap_err().0[..5].to_string();
    assert_eq!(refused(act("to_planning")), "E7001", "not offered in backlog");
    let cancelled = act("cancel").unwrap();
    assert_eq!(cancelled["column"], "cancelled");
    assert_eq!(allowed(&flow(&s.root).unwrap(), &cancelled), ["to_planning"]);
    let archived = api(&s.root, "POST", &format!("/tasks/{tid}/archive"), json!({})).unwrap();
    assert!(archived["archivedTime"].is_u64());
    let back = act("to_planning").unwrap();
    assert_eq!((back["column"].as_str(), &back["archivedTime"], &back["closedTime"]), (Some("planning"), &Value::Null, &cancelled["closedTime"]));
    assert!(t["createdTime"].is_u64());
    assert_eq!(back["createdTime"], t["createdTime"], "D-130: bringing it back keeps when it was created");
    assert_eq!(back["title"], "Shapes");
    let last = back["notes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["author"].as_str(), last["text"].as_str()), (Some("solace"), Some("The human brought this task back from Canceled to Planning.")));
    let mut done = back.clone();
    done["column"] = json!("complete");
    assert!(allowed(&flow(&s.root).unwrap(), &done).is_empty(), "complete offers nothing");
}

/// D-132: a task with a feature can't go back to the backlog from Planning;
/// without its feature it can again. From Canceled it goes back to planning,
/// feature or not (D-133).
#[test]
fn a_task_with_a_feature_is_refused_back_to_the_backlog() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1 .0).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    api(&s.root, "PATCH", &format!("/tasks/{tid}"), json!({"feature": f["id"]})).unwrap();
    let act = |name: &str| api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": name}));
    let refused = |r: Result<Value, Failure>| r.unwrap_err().0;
    let planned = act("plan").unwrap();
    assert_eq!(planned["column"], "planning");
    assert!(!allowed(&flow(&s.root).unwrap(), &planned).contains(&"to_backlog".to_string()));
    let e = refused(act("to_backlog"));
    assert!(e.starts_with("E7001") && e.contains("feature"), "{e}");
    let cancelled = act("cancel").unwrap();
    assert_eq!(allowed(&flow(&s.root).unwrap(), &cancelled), ["to_planning"]);
    assert!(refused(act("to_backlog")).starts_with("E7001"));
    let back = act("to_planning").unwrap();
    assert_eq!((back["column"].as_str(), &back["feature"]), (Some("planning"), &f["id"]));
    api(&s.root, "PATCH", &format!("/tasks/{tid}"), json!({"feature": null})).unwrap();
    assert_eq!(act("to_backlog").unwrap()["column"], "backlog");
}

/// D-133: back to planning from Assigned is refused while a run is open, and
/// from Needs review it keeps the task's changes and clears its approval.
#[test]
fn back_to_planning_waits_for_the_run_and_keeps_changes() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let put = |column: &str, state: &str, stamped: bool| {
        let mut v = task(&s.root, &tid).unwrap();
        v["column"] = json!(column);
        v["runs"] = json!([{"id": "r1", "kind": "work", "state": state, "model": "m", "log": "/l"}]);
        v["stamps"] = if stamped { json!({"approved": {"who": "human", "time": 1}}) } else { json!({}) };
        tasks::put(&s, &v).unwrap();
    };
    let act = || api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "to_planning"}));
    for state in ["running", "waiting"] {
        put("assigned", state, false);
        let e = act().unwrap_err().0;
        assert!(e.starts_with("E7001") && e.contains("stop"), "{e}");
    }
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    put("assigned", "ended", false);
    assert_eq!(act().unwrap()["column"], "planning");
    put("needs_review", "ended", true);
    let back = act().unwrap();
    assert_eq!(back["column"], "planning");
    assert!(back["stamps"].get("approved").is_none(), "approval cleared");
    assert!(!changes_of(&s, &task(&s.root, &tid).unwrap()).unwrap().is_empty(), "changes kept");
}

/// A workflow whose `finished` event targets a final column, so `task done` settles.
const DONE_IS_FINAL: &str = "[board]\nevents = { asked = \"todo\", finished = \"done\", failed = \"todo\" }\n[[board.columns]]\nid = \"todo\"\nactions = [\"cancel\"]\n[[board.columns]]\nid = \"done\"\nfinal = true\n[[board.columns]]\nid = \"gone\"\nfinal = true\n[board.actions]\ncancel = { revert = true, to = \"gone\" }\n";

/// D-132 point 5: in a workflow with no `planning` column, a new Tech Debt task
/// is made in its first column.
#[test]
fn tech_debt_goes_to_the_first_column_without_planning() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1 .0).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"A"})).unwrap();
    t["feature"] = f["id"].clone();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"rv","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut t, "agent", Some("rv"), "### Tech debt\n- `app.main`: name the constant\n");
    let (debt, n) = tech_debt(&top, &t).unwrap().unwrap();
    assert_eq!((debt["column"].as_str(), n), (Some("todo"), 1));
}

/// A `done` from a live run, applied by the server, settles: the move and the settle both happen.
#[test]
fn a_done_applied_by_the_server_settles() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&s, &t).unwrap();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let has_before = || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    assert!(has_before());
    testing::inbox_file(&s.root, "0001", json!({"kind":"agent","task":tid,"run":"r1","command":"done","text":"done"}));
    tasks::drain(&s.root);
    assert_eq!(task(&s.root, &tid).unwrap()["column"], "done");
    assert!(!has_before(), "applying the move settled the task");
}

/// `serve` settles a task left final with undo records (a crash between move and settle).
#[test]
fn resettle_settles_a_final_task_left_unsettled() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let has_before = || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    resettle(&s.root);
    assert!(has_before(), "a task not in a final column is left alone");
    t["column"] = json!("done");
    tasks::put(&s, &t).unwrap();
    resettle(&s.root);
    assert!(!has_before());
}

/// `task done` from a run that ended before the server applied it neither moves
/// the task nor settles it: its undo records stay.
#[test]
fn a_late_done_does_not_settle_and_a_real_one_does() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Shapes"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "ended", "outcome": "stopped"}]);
    tasks::put(&s, &t).unwrap();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let has_before = || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    assert!(has_before());
    testing::inbox_file(&s.root, "0001", json!({"kind":"agent","task":tid,"run":"r1","command":"done","text":"late"}));
    tasks::drain(&s.root);
    assert!(has_before(), "the late done must not drop the undo records");
    // The server itself moving the task into a final column settles it.
    let w = flow(&s.root).unwrap();
    let last = w["columns"].as_array().unwrap().iter().find(|c| c["final"] == true).unwrap()["id"].clone();
    tasks::update(&s.root, &tid, move |t| {
        t["column"] = last;
        Ok(())
    })
    .unwrap();
    settle_final(&s.root, &tid).unwrap();
    assert!(!has_before());
}

#[test]
fn note_images_are_checked_numbered_and_encoded() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    assert_eq!(image_kind(b"<svg onload=alert(1)>"), None);
    assert_eq!(image_type("0123456789abcdef.png"), Some("image/png"));
    for bad in ["../x.png", "0123456789abcdef.svg", "x.png"] {
        assert_eq!(image_type(bad), None);
    }
    let a = "/tasks/t1/files/0123456789abcdef.png";
    let mut found = vec![];
    let text = note_images(&format!("see ![one]({a}) and ![x](http://e/x.png) ![again]({a}) ![broken"), "/tasks/t1/files/", &mut found);
    assert_eq!(text, "see [image 1] and ![x](http://e/x.png) [image 1] ![broken");
    assert_eq!(found, ["0123456789abcdef.png"]);
}

/// A message for a run that has ended keeps its note and changes nothing else.
#[test]
fn a_late_agent_message_does_not_wake_an_ended_run() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"ended","outcome":"stopped"}]);
    let before = t["column"].clone();
    for command in ["ask", "done", "note"] {
        let m = json!({"kind":"agent","task":"t","run":"r1","command":command,"text":"late","question":{"id":"q","text":"?","options":["a","b"]}});
        message(&w, &mut t, &m).unwrap();
    }
    assert_eq!(t["runs"][0]["state"], "ended");
    assert_eq!(t["column"], before);
    assert!(t["questions"].as_array().unwrap().is_empty());
    assert_eq!(t["notes"].as_array().unwrap().len(), 3);
}
/// A crash between the store copy and removing the old file leaves both; the move finishes.
#[test]
fn adopt_finishes_an_interrupted_move_and_refuses_a_reused_id() {
    let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#)]);
    fs::create_dir_all(store_dir(&root)).unwrap();
    fs::write(store_dir(&root).join("t1.json"), r#"{"id":"t1","column":"backlog","home":".","runs":[],"notes":[],"questions":[],"links":[]}"#).unwrap();
    adopt_tasks(&root, &root).unwrap();
    assert!(!root.join(".solace/board/t1.json").exists());
    fs::create_dir_all(root.join(".solace/board")).unwrap();
    fs::write(root.join(".solace/board/t1.json"), r#"{"id":"t1","column":"backlog"}"#).unwrap();
    fs::write(store_dir(&root).join("t1.json"), r#"{"id":"t1","column":"backlog","home":"other"}"#).unwrap();
    let e = adopt_tasks(&root, &root).unwrap_err();
    assert!(e.0.contains("used twice") && e.0.contains("t1"));
    assert!(root.join(".solace/board/t1.json").exists());
}
/// `strategy show run` outlines a run's steps, then gives chosen ones in full.
#[test]
fn strategy_show_run_outlines_and_picks_steps() {
    let root = testing::Scratch::new();
    let log = root.join("run.jsonl");
    let lines = [
        json!({"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view a"},"result":{"content":[{"type":"text","text":"pub fn a()"}]},"status":"completed"}}),
        json!({"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"All done.\nSecond line."}}),
        json!({"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}),
    ];
    fs::write(&log, lines.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    let long = format!("{}.\nSecond line.", "x".repeat(120));
    let task = json!({"id":"t1","column":"backlog","notes":[{"author":"human","time":7,"text":"A short note."},{"author":"agent","time":8,"text":long}],
        "questions":[{"id":"q1","text":"Which one?","options":["A"],"answer":"A"},{"id":"q2","text":"Open","options":["A"]}],"conversation":[{"author":"human","text":"the thread"}],
        "runs":[{"id":"r1","kind":"work","state":"ended","model":"fake","log":log.to_str().unwrap()}]});
    let root = root.with(&[(".solace/board/t1.json", &task.to_string())]);
    adopt_tasks(&root, &root).unwrap();
    let shown = strategy_show(&root, "task", "t1").unwrap();
    assert_eq!(shown["links"]["runs"], json!(["r1"]));
    let rec = shown["record"].clone();
    assert_eq!((rec["notes"].clone(), rec["questions"].clone(), rec["runs"].clone()), (json!(["1 human 7: A short note.", format!("2 agent 8: {}…", "x".repeat(100))]), json!(["1 Which one?: A", "2 Open: unanswered"]), json!(["r1 work ended fake"])));
    assert!(rec.get("conversation").is_none());
    let one = strategy_note_show(&root, "t1", "2").unwrap();
    assert_eq!((one["task"].as_str(), one["notes"][0]["note"].as_u64(), one["notes"][0]["text"].as_str()), (Some("t1"), Some(2), Some(long.as_str())));
    assert_eq!(strategy_note_show(&root, "t1", "all").unwrap()["notes"].as_array().unwrap().len(), 2);
    for bad in ["0", "3", "2..1", "x", "1,,2"] { assert!(strategy_note_show(&root, "t1", bad).unwrap_err().0.starts_with("E7001"), "{bad}"); }
    let outline = strategy_run_show(&root, "t1", "r1", None).unwrap();
    assert_eq!(outline["run"]["id"], "r1");
    assert!(outline["run"].get("log").is_none());
    assert_eq!(outline["steps"][0], "1 tool: solace view a (ok)");
    assert_eq!(outline["steps"][1], "2 message: All done.");
    // The token-usage entry is not a step.
    assert_eq!(outline["steps"].as_array().unwrap().len(), 2);
    let picked = strategy_run_show(&root, "t1", "r1", Some("2,1")).unwrap();
    let numbers: Vec<_> = picked["steps"].as_array().unwrap().iter().map(|s| s["step"].clone()).collect();
    assert_eq!(numbers, [json!(2), json!(1)]);
    assert_eq!(picked["steps"][1]["output"], "pub fn a()");
    assert_eq!(picked["steps"][0]["text"], "All done.\nSecond line.");
    assert_eq!(strategy_run_show(&root, "t1", "r1", Some("all")).unwrap()["steps"].as_array().unwrap().len(), 2);
    // D-105: through the command, a reply of steps stops at a whole step and names the rest.
    let one = format!("{}\n", serde_json::to_string_pretty(&strategy_run_show(&root, "t1", "r1", Some("1")).unwrap()).unwrap());
    let next = "Next: strategy show run t1 r1 2\n";
    let run = |a: &[&str], budget: usize| strategy_command(&root, "s1", a, &mut || Ok(String::new()), budget);
    assert_eq!(run(&["strategy", "show", "run", "t1", "r1", "1,2"], one.len() + next.len()).unwrap(), one.clone() + next);
    assert!(!run(&["strategy", "show", "run", "t1", "r1", "1,2"], 10_000).unwrap().contains("Next:"));
    assert!(run(&["strategy", "show", "run", "t1", "r1", "1", "--from", "x"], 10_000).unwrap_err().0.starts_with("E7001"));
    assert!(run(&["strategy", "show", "note", "t1", "2", "--from", "3"], 10_000).unwrap().contains("characters 3.."));
    for bad in ["0", "3", "2..1", "x", "1,,2"] {
        assert!(strategy_run_show(&root, "t1", "r1", Some(bad)).unwrap_err().0.starts_with("E7001"), "{bad}");
    }
    // A run whose log holds no steps says so.
    let task = json!({"id":"t2","column":"backlog","runs":[{"id":"r2","kind":"work","state":"ended","log":"missing.jsonl"}]});
    fs::write(store_dir(&root).join("t2.json"), task.to_string()).unwrap();
    assert_eq!(strategy_run_show(&root, "t2", "r2", None).unwrap()["steps"], json!([]));
    assert!(strategy_run_show(&root, "t2", "r2", Some("1")).unwrap_err().0.contains("no steps"));
    assert!(strategy_run_show(&root, "t1", "nope", None).unwrap_err().0.starts_with("E7023"));
    assert!(strategy_run_show(&root, "nope", "r1", None).unwrap_err().0.starts_with("E7023"));
}
/// One unreadable task file doesn't hide the others.
#[test]
fn all_tasks_skips_an_unreadable_file() {
    let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#)]);
    adopt_tasks(&root, &root).unwrap();
    fs::write(store_dir(&root).join("empty.json"), "").unwrap();
    let all = all_tasks(&root).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["id"], "t1");
}
/// A run that waits for an answer lets no command through: it holds still
/// while a server runs and stops when none does.
#[test]
fn agent_guard_holds_a_run_that_waits() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Ask"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"waiting"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let guard = |a: &'static [&'static str]| {
        let (root, tid) = (s.root.clone(), tid.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || drop(tx.send(agent_guard(&root, &tid, "r1", a, &[]).map(drop).map_err(|e| e.0))));
        rx
    };
    // No server: stop.
    assert!(guard(&["view", "app"]).recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().contains("waits for an answer"));
    // A `task` command is not held.
    assert!(guard(&["task", "note", "x"]).recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    // A server runs: the command waits until the run is answered.
    let _server = try_lock_file(&store_dir(&s.root).join("server.lock")).unwrap().unwrap();
    let view = guard(&["view", "app"]);
    assert!(view.recv_timeout(Duration::from_millis(300)).is_err());
    t["runs"][0]["state"] = json!("running");
    tasks::put(&s, &t).unwrap();
    assert!(view.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
}
#[test]
fn tasks_saved_in_done_load_as_complete() {
    let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"done"}"#)]);
    adopt_tasks(&root, &root).unwrap();
    assert_eq!(task(&root, "t1").unwrap()["column"], "complete");
    // A custom workflow that still has a `done` column keeps it.
    fs::write(root.join("solace.toml"), include_str!("default-board.toml").replace("= \"complete\"", "= \"done\"")).unwrap();
    assert_eq!(tasks(&root).unwrap()[0]["column"], "done");
}
/// D-82: the default workflow has no `blocked` column.
#[test]
fn tasks_saved_in_blocked_load_as_assigned_and_held() {
    let root = testing::Scratch::new().with(&[
        (".solace/board/t1.json", r#"{"id":"t1","column":"blocked","attention":{"reason":"an agent asked a question","time":1}}"#),
        (".solace/board/t2.json", r#"{"id":"t2","column":"blocked"}"#),
    ]);
    adopt_tasks(&root, &root).unwrap();
    let (a, b) = (task(&root, "t1").unwrap(), task(&root, "t2").unwrap());
    assert_eq!((a["column"].as_str(), a["attention"]["reason"].as_str(), a["attention"]["hold"].as_bool()), (Some("assigned"), Some("an agent asked a question"), Some(true)));
    assert_eq!((b["column"].as_str(), b["attention"]["hold"].as_bool()), (Some("assigned"), Some(true)));
    // `asked` and `failed` need no target; `finished` does, and a target must exist.
    let default = include_str!("default-board.toml");
    let w = |text: String| workflow(&text.parse::<toml::Value>().unwrap());
    assert!(w(default.to_string()).is_ok());
    assert!(w(default.replace("finished = \"needs_review\"", "")).is_err());
    assert!(w(default.replace("events = {", "events = { asked = \"missing\",")).is_err());
}
#[test]
fn tasks_move_once_into_the_root_store() {
    let root = testing::Scratch::new().with(&[
        (".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#),
        (".solace/board/t1/r1.jsonl", "{}\n"),
    ]);
    adopt_tasks(&root, &root).unwrap();
    assert!(!root.join(".solace/board/t1.json").exists());
    assert!(root.join(".solace/board/t1/r1.jsonl").exists(), "run logs stay in the project");
    let t = task(&root, "t1").unwrap();
    assert_eq!(t["home"], ".");
    assert_eq!(scope::home(&root, "t1").unwrap(), root.to_path_buf());
    // A leftover copy of a task already moved from this project is dropped, not merged.
    fs::write(root.join(".solace/board/t1.json"), r#"{"id":"t1"}"#).unwrap();
    adopt_tasks(&root, &root).unwrap();
    assert!(!root.join(".solace/board/t1.json").exists());
    assert_eq!(task(&root, "t1").unwrap()["home"], ".");
}
#[test]
fn presets_merge_with_configured_models() {
    let v = parse_system("").unwrap();
    let ids: Vec<&str> = v["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"codex-gpt-6-astra") && ids.contains(&"claude-opus-5-5"));
    assert_eq!(v["task_timeout"], 3600);
    let v = parse_system(
        "[[models]]\nid = \"claude-opus-5-5\"\nlabel = \"Mine\"\nagent = \"claude\"\nmodel = \"x\"\n\
         [[models]]\nid = \"local\"\nlabel = \"Local\"\nagent = \"codex\"\nmodel = \"qwen\"\n\
         endpoint = { kind = \"openai\", base_url = \"http://localhost:11434/v1\", key_env = \"LOCAL_KEY\" }\n",
    )
    .unwrap();
    let models = v["models"].as_array().unwrap();
    assert_eq!(models.iter().find(|m| m["id"] == "claude-opus-5-5").unwrap()["label"], "Mine");
    assert_eq!(models.last().unwrap()["id"], "local");
    assert_eq!(models.last().unwrap()["configured"], true, "a configured model keeps its entry over a discovery");
    assert!(models.iter().find(|m| m["id"] == "codex-gpt-6-astra").unwrap().get("configured").is_none(), "a preset is not configured");
    assert!(parse_system("presets = false\n").unwrap()["models"].as_array().unwrap().is_empty());
}

/// Presets, then discovery, then the human's config: a discovered model
/// replaces a preset of the same id and adds new ones; a configured model
/// keeps its entry; no discovery leaves the presets standing.
#[test]
fn discovered_models_merge_after_presets() {
    let configured = json!([
        {"id":"codex-gpt-6-astra","label":"Preset","agent":"codex","model":"gpt-6-astra"},
        {"id":"mine","label":"Mine","agent":"claude","model":"opus","configured":true},
    ]);
    let all = json!({"codex":{"time":100,"models":[
        {"id":"codex-gpt-6-astra","label":"Codex · GPT-6 Astra","agent":"codex","model":"gpt-6-astra","effort":"medium","efforts":["low","medium"]},
        {"id":"codex-new-1","label":"Codex · New 1","agent":"codex","model":"new-1","effort":"high","efforts":["high"]},
    ]}});
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m.iter().find(|x| x["id"] == "codex-gpt-6-astra").unwrap()["label"], "Codex · GPT-6 Astra", "discovery replaces a preset");
    assert!(m.iter().any(|x| x["id"] == "codex-new-1"), "a model the agent reports is offered");
    assert_eq!(m.iter().find(|x| x["id"] == "mine").unwrap()["label"], "Mine", "the human's config wins for the same id");
    assert_eq!(agents::merge_from(&configured, &json!({})).len(), 2, "presets stand when discovery fails");
}

/// A recorded `model/list` answer: hidden models are left out, and each model
/// keeps its slug (what `-m` takes), its efforts and its default effort.
#[test]
fn codex_answers_offer_their_models_and_efforts() {
    let r = json!({"data":[
        {"id":"gpt-6-sol","model":"gpt-6-sol","displayName":"GPT-6 Sol","hidden":false,
         "supportedReasoningEfforts":[{"effort":"low","description":"Low"},{"effort":"medium","description":"Medium"},{"effort":"high","description":"High"}],
         "defaultReasoningEffort":"medium","isDefault":true},
        {"id":"gpt-6-sol-mini","model":"gpt-6-sol-mini","displayName":"GPT-6 Sol Mini","hidden":true,"supportedReasoningEfforts":[],"defaultReasoningEffort":"medium"},
        {"id":"gpt-5.6-terra","model":"gpt-5.6-terra","displayName":"GPT-5.6 Terra","supportedReasoningEfforts":[{"effort":"medium"}],"defaultReasoningEffort":"medium"}
    ]});
    let models = agents::codex_page(&r);
    assert_eq!(models.len(), 2, "a hidden model is left out");
    assert_eq!(models[0]["id"], "codex-gpt-6-sol");
    assert_eq!(models[0]["label"], "Codex · GPT-6 Sol");
    assert_eq!(models[0]["model"], "gpt-6-sol", "the slug is what the run passes to -m");
    assert_eq!(models[0]["efforts"], json!(["low", "medium", "high"]));
    assert_eq!(models[0]["effort"], "medium");
    assert_eq!(efforts(&models[1]), ["medium"], "efforts come from the answer, not the fixed list");
}

/// Claude Code has no list command: its aliases name the latest of each family.
#[test]
fn claude_falls_back_to_its_aliases() {
    let a = agents::claude_aliases();
    let ids: Vec<&str> = a.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude-opus", "claude-sonnet", "claude-haiku", "claude-fable"]);
    assert_eq!(a[0]["model"], "opus", "the alias is what the run passes to --model");
    assert_eq!(a[0]["label"], "Claude Code · Opus (latest)");
    assert_eq!(efforts(&a[0]), ["low", "medium", "high", "xhigh", "max"], "an alias takes the fixed Claude efforts");
}

/// With an API key, the provider's models endpoint lists exact ids: active
/// ones, with the efforts its capability flags name.
#[test]
fn the_models_endpoint_lists_active_ids() {
    let body = json!({"data":[
        {"id":"claude-opus-5-5","display_name":"Opus 5.5","lifecycle":"active","capabilities":{"effort":{"low":true,"medium":true,"high":true,"xhigh":true,"max":true}}},
        {"id":"claude-old-1","display_name":"Old 1","lifecycle":"deprecated","capabilities":{"effort":{"low":true}}},
        {"id":"claude-haiku-5","display_name":"Haiku 5","capabilities":{}}
    ]});
    let models = agents::api_page(&body);
    assert_eq!(models.len(), 2, "a retired model is left out");
    assert_eq!(models[0]["id"], "claude-opus-5-5");
    assert_eq!(models[0]["label"], "Claude Code · Opus 5.5");
    assert_eq!(models[0]["efforts"], json!(["low", "medium", "high", "xhigh", "max"]));
    assert_eq!(models[1]["efforts"], json!([]), "no effort flags: no efforts to choose");
}

/// A model a discovery dropped stays in the list, so tasks already using it
/// keep working, and is not offered for a new run.
#[test]
fn a_model_off_the_offer_stays_for_the_tasks_using_it() {
    let configured = json!([{"id":"claude-opus-5-5","label":"Preset","agent":"claude","model":"claude-opus-5-5"}]);
    let all = json!({"claude":{"time":100,"models":[]},"gone":{"claude-opus":{"id":"claude-opus","label":"Claude Code · Opus (latest)","agent":"claude","model":"opus","reason":"no longer offered by Claude Code"}}});
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m.len(), 2, "the model stays in the list");
    let gone = m.iter().find(|x| x["id"] == "claude-opus").unwrap();
    assert_eq!(gone["gone"], true);
    assert_eq!(agents::availability(gone), Some(("no longer offered by Claude Code".to_string(), false)), "it is not offered for a new run");
}

#[test]
fn a_model_error_names_the_offer_as_out_of_date() {
    assert!(agents::model_gone("Error: Model 'gpt-9-pro' not found."));
    assert!(agents::model_gone("unknown model claude-opus-9"));
    assert!(agents::model_gone("The model gpt-6-new does not exist"));
    assert!(!agents::model_gone("You've hit your usage limit; try again at 3pm"));
    assert!(!agents::model_gone("stream disconnected before completion"));
}

/// The model a run used comes from its log's start event, and an alias's
/// label names it once known.
#[test]
fn a_run_shows_the_model_it_used() {
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    fs::write(&log, "{\"type\":\"assistant\"}\n{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\",\"model\":\"claude-opus-5-5\"}\n").unwrap();
    assert_eq!(used_model(&log).as_deref(), Some("claude-opus-5-5"));
    fs::write(&log, "not json\n{\"type\":\"thread.started\",\"thread_id\":\"t1\"}\n").unwrap();
    assert_eq!(used_model(&log), None, "a start event naming no model names none");
    let configured = json!([{"id":"claude-opus","label":"Claude Code · Opus (latest)","agent":"claude","model":"opus"}]);
    let m = agents::merge_from(&configured, &json!({"used":{"claude-opus":"claude-opus-5-5"}}));
    assert_eq!(m[0]["label"], "Claude Code · Opus (latest) · claude-opus-5-5");
    assert_eq!(m[0]["model"], "opus", "the alias is what runs");
}

#[test]
fn endpoints_are_checked_and_passed_to_codex() {
    // An endpoint's kind must match its agent.
    let wrong = "[[models]]\nid = \"x\"\nlabel = \"X\"\nagent = \"claude\"\nmodel = \"m\"\nendpoint = { kind = \"openai\", base_url = \"http://h\" }\n";
    assert!(parse_system(wrong).unwrap_err().0.starts_with("E6020"));
    let m =
        json!({"label":"Local","endpoint":{"kind":"openai","base_url":"http://localhost:11434/v1","key_env":"SOLACE_TEST_UNSET_KEY"}});
    let args = endpoint_args(&m);
    assert_eq!(args[1], "model_provider=\"solace_endpoint\"");
    let provider: toml::Value = format!("p = {}", args[3].split_once('=').unwrap().1).parse().unwrap();
    assert_eq!(provider["p"]["base_url"].as_str(), Some("http://localhost:11434/v1"));
    assert_eq!(provider["p"]["env_key"].as_str(), Some("SOLACE_TEST_UNSET_KEY"));
    assert_eq!(provider["p"]["wire_api"].as_str(), Some("responses"));
    assert!(endpoint_args(&json!({"label":"No endpoint"})).is_empty());
}

/// A line diff keeps two lines around each change and marks the lines it
/// skips, between changes and after the last one.
#[test]
fn line_diff_marks_the_lines_it_skips() {
    let lines = |f: &dyn Fn(usize) -> String| (1..=12).map(|n| format!("{}\n", f(n))).collect::<String>();
    let a = lines(&|n| format!("l{n}"));
    let b = lines(&|n| if n == 2 || n == 10 { format!("L{n}") } else { format!("l{n}") });
    assert_eq!(line_diff(&a, &b), "  l1\n- l2\n+ L2\n  l3\n  l4\n  …\n  l8\n  l9\n- l10\n+ L10\n  l11\n  l12\n");
    let first = lines(&|n| if n == 1 { "L1".into() } else { format!("l{n}") });
    assert_eq!(line_diff(&a, &first), "- l1\n+ L1\n  l2\n  l3\n  …\n");
    assert_eq!(line_diff("", ""), "");
    assert_eq!(line_diff("x\ny\n", "x\ny\n"), "  …\n");
    assert_eq!(diff_ops("a\nb", "a\nb"), [(' ', "a"), (' ', "b")]);
}

/// The marked lines are numbered in the new text; a removal marks the line it left.
#[test]
fn changed_lines_number_the_new_text() {
    let lines = |f: &dyn Fn(usize) -> String| (1..=12).map(|n| format!("{}\n", f(n))).collect::<String>();
    let a = lines(&|n| format!("l{n}"));
    let b = lines(&|n| if n == 2 || n == 10 { format!("L{n}") } else { format!("l{n}") });
    assert_eq!(changed_lines(&a, &b), [1, 9]);
    assert_eq!(changed_lines("x\ny\nz", "x\ny"), [1], "a removal at the end marks the last line there is");
    assert_eq!(changed_lines("b", "a\nb"), [0]);
    assert!(changed_lines("a\nb", "a\nb").is_empty());
}

/// Anthropic reports cache creation, so cached input is added; an endpoint
/// that omits it counts the cache in `input_tokens` already.
#[test]
fn context_tokens_adds_the_cache_only_when_creation_is_reported() {
    assert_eq!(context_tokens(&json!({"input_tokens": 1, "cache_read_input_tokens": 10, "cache_creation_input_tokens": 100})), 111);
    assert_eq!(context_tokens(&json!({"input_tokens": 3, "cache_read_input_tokens": 4, "cache_creation_input_tokens": null})), 7);
    assert_eq!(context_tokens(&json!({"input_tokens": 5, "cache_read_input_tokens": 10})), 5);
    assert_eq!(context_tokens(&json!({})), 0);
}

#[test]
fn percent_encode_keeps_unreserved_and_escapes_the_rest() {
    assert_eq!(percent_encode("a b/é~-_.9"), "a%20b%2F%C3%A9~-_.9");
    assert_eq!(percent_encode(""), "");
}

#[test]
fn endpoint_context_is_read_from_the_server() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let replies = [
        r#"{"data":[{"id":"served-name","meta":{"n_ctx":131072}}]}"#,
        r#"{"data":[{"id":"a","meta":{"n_ctx":1}},{"id":"b","meta":{"n_ctx":2}}]}"#,
        r#"{"data":[{"id":"a","meta":{"n_ctx":1}},{"id":"b","meta":{"n_ctx":2}}]}"#,
    ];
    let t = std::thread::spawn(move || {
        for body in replies {
            let rq = server.recv().unwrap();
            assert_eq!(rq.url(), "/v1/models");
            rq.respond(tiny_http::Response::from_string(body)).unwrap();
        }
    });
    // One model: used whatever it's called. Several: matched by name, or unknown.
    assert_eq!(endpoint_context(&base, "configured-name"), Some(131072));
    assert_eq!(endpoint_context(&base, "b"), Some(2));
    assert_eq!(endpoint_context(&base, "c"), None);
    t.join().unwrap();
    assert_eq!(endpoint_context("https://example.com", "m"), None);
}

#[test]
fn transcripts_read_codex_and_claude_logs() {
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    let codex = [
        json!({"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view a"},"status":"in_progress"}}),
        json!({"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view a","text":"x"},"result":{"content":[{"type":"text","text":"pub fn a()"}]},"status":"completed"}}),
        json!({"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"All **done**."}}),
        json!({"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}),
    ];
    let text = codex.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n") + "\nnot json\nstill not\n";
    fs::write(&log, text).unwrap();
    let t = transcript(&log);
    let kinds: Vec<&str> = t.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["tool", "message", "usage", "output"]);
    assert_eq!(
        (t[0]["title"].as_str(), t[0]["status"].as_str(), t[0]["output"].as_str()),
        (Some("solace view a"), Some("ok"), Some("pub fn a()"))
    );
    // D-109: Codex's failed call where only some commands failed says how many in its title.
    let batch = json!({"type":"item.completed","item":{"id":"i3","type":"mcp_tool_call","tool":"solace","arguments":{"command":"read a\ngrep x"},"result":{"content":[{"type":"text","text":"E6002\ndocs/x\n1 of 2 commands failed: read a\n"}]},"status":"failed"}});
    fs::write(&log, batch.to_string()).unwrap();
    let t = transcript(&log);
    assert_eq!((t[0]["title"].as_str(), t[0]["status"].as_str()), (Some("solace (1 of 2 failed) read a\ngrep x"), Some("failed")));
    // Every command failed: the title stays as it was.
    let all = batch.to_string().replace("1 of 2 commands", "2 of 2 commands");
    fs::write(&log, all).unwrap();
    assert_eq!(transcript(&log)[0]["title"], "solace read a\ngrep x");
    // D-119: a call that stopped at a failed change says how many it skipped.
    let stopped = batch.to_string().replace("1 of 2 commands failed: read a", "1 of 3 commands failed: write a; skipped: run t; read b");
    fs::write(&log, stopped).unwrap();
    assert_eq!(transcript(&log)[0]["title"], "solace (1 of 3 failed, 2 skipped) read a\ngrep x");
    assert_eq!(partly_failed("failed", "E\n1 of 2 commands failed: write a; skipped: run t\n").as_deref(), Some("(1 of 2 failed, 1 skipped)"));
    assert_eq!(partly_failed("failed", "E\n1 of 2 commands failed: read a\n").as_deref(), Some("(1 of 2 failed)"));
    // D-119 point 5: a command's text can hold "; ", so the count is capped at the
    // commands the reply says were left after the failure.
    assert_eq!(partly_failed("failed", "E\n1 of 2 commands failed: write a; skipped: read x; grep y\n").as_deref(), Some("(1 of 2 failed, 1 skipped)"));
    let text = codex.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n") + "\nnot json\nstill not\n";
    fs::write(&log, text).unwrap();
    let t = transcript(&log);
    assert_eq!(t[0]["input"], "text:\nx");
    assert_eq!(t[3]["text"], "not json\nstill not");
    assert_eq!(last_words(&t).unwrap(), "All **done**.");
    let claude = [
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"u1","name":"mcp__solace__solace","input":{"command":"test"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u1","content":[{"type":"text","text":"E1 failed"}],"is_error":true}]}}),
    ];
    fs::write(&log, claude.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    let t = transcript(&log);
    assert_eq!(
        (t[1]["title"].as_str(), t[1]["status"].as_str(), t[1]["output"].as_str()),
        (Some("solace test"), Some("failed"), Some("E1 failed"))
    );
    // D-109: a batch where only some commands failed says how many in its title.
    let batch = [
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"u2","name":"mcp__solace__solace","input":{"command":"read a\ngrep x"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u2","content":[{"type":"text","text":"==== solace read a ====\nE6002\n==== solace grep x ====\ndocs/x\n1 of 2 commands failed: read a\n"}],"is_error":true}]}}),
    ];
    fs::write(&log, batch.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    let t = transcript(&log);
    assert_eq!((t[0]["title"].as_str(), t[0]["status"].as_str()), (Some("solace (1 of 2 failed) read a\ngrep x"), Some("failed")));
    // Live usage: the latest call's context and output per call; Anthropic adds cache, LM Studio includes it.
    let call = |id: &str, u: Value| json!({"type":"assistant","message":{"id":id,"usage":u,"content":[]}}).to_string();
    let anthropic = json!({"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":5,"output_tokens":7});
    let lines = [
        call("m1", anthropic.clone()),
        call("m1", anthropic),
        call("m2", json!({"input_tokens":300,"cache_read_input_tokens":250,"output_tokens":2})),
    ];
    fs::write(&log, lines.join("\n")).unwrap();
    assert_eq!(transcript(&log).last().unwrap()["text"], "Context 300 tokens; 9 output tokens so far");
    let result = json!({"type":"result","usage":{"input_tokens":310,"cache_read_input_tokens":350,"output_tokens":9}}).to_string();
    fs::write(&log, [lines.join("\n"), result].join("\n")).unwrap();
    let usage: Vec<Value> = transcript(&log).into_iter().filter(|e| e["kind"] == "usage").collect();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0]["text"], "310 input tokens (350 read from cache), 9 output tokens; last context 300 tokens");
    // Anthropic reports input outside the cache: the total adds reads and writes.
    let result = json!({"type":"result","usage":{"input_tokens":16,"cache_read_input_tokens":166278,"cache_creation_input_tokens":34062,"output_tokens":4040}}).to_string();
    fs::write(&log, [lines.join("\n"), result].join("\n")).unwrap();
    let usage: Vec<Value> = transcript(&log).into_iter().filter(|e| e["kind"] == "usage").collect();
    assert_eq!(usage[0]["text"], "200356 input tokens (166278 read from cache, 34062 written to cache), 4040 output tokens; last context 300 tokens");
    assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
}

/// D-110: a brief names the task's unfinished dependencies, from any project,
/// with advice; with none, or only finished ones (canceled counts), it says nothing.
#[test]
fn briefs_name_unfinished_dependencies() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let home = scope::dir(&top, &p.root);
    let dir = store_dir(&top);
    fs::create_dir_all(&dir).unwrap();
    let put = |id: &str, home: &str, column: &str, deps: &[&str]| {
        let t = json!({"id":id,"title":format!("Task {id}"),"body":"","home":home,"column":column,"rank":1,"dependsOn":deps,
            "runs":[],"notes":[],"questions":[],"links":[],"closedTime":null,"archivedTime":null});
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("a", "lib", "assigned", &[]);
    put("b", &home, "complete", &[]);
    put("c", &home, "cancelled", &[]);
    put("t", &home, "planning", &["a", "b", "c"]);
    put("f", &home, "planning", &["b", "c"]);
    put("n", &home, "backlog", &[]);
    let w = waiting_on(&top, "t");
    for part in ["Depends on:\n", "- a Task a (Assigned, in lib) — unfinished", "- b Task b (Complete)\n", "- c Task c (Canceled)\n", WAITING] {
        assert!(w.contains(part), "{part} missing from:\n{w}");
    }
    assert_eq!((waiting_on(&top, "f"), waiting_on(&top, "n")), (String::new(), String::new()));
    // The run prompt carries it; one with only finished dependencies doesn't.
    let task = |id: &str| all_tasks(&top).unwrap().into_iter().find(|t| t["id"] == id).unwrap();
    let run = json!({"id":"r1","kind":"work"});
    assert!(continuation_brief(&task("t"), &run, false, &p.root, &p.root, false).0.contains(&w));
    let finished = continuation_brief(&task("f"), &run, false, &p.root, &p.root, false).0;
    assert!(!finished.contains("Depends on:") && !finished.contains(WAITING), "{finished}");
    // A task brought back to the backlog counts as unfinished again.
    put("b", &home, "backlog", &[]);
    assert!(waiting_on(&top, "f").contains("- b Task b (Backlog) — unfinished"));
}

#[test]
fn briefs_hold_what_the_agent_needs_as_text() {
    let t = json!({"id":"t1","title":"Add coolcalc","body":"A CLI calculator.","rank":3,"stamps":{},
        "links":[{"name":"spec","url":"http://x"}],"notes":[{"author":"human","text":"Keep it small","time":1,"run":null}],
        "questions":[{"id":"q","text":"Syntax?","options":["a","b"],"answer":"b"}],"runs":[{"id":"r1"}],"changes":[{"entity":"symbol:x","name":"calc.add","run":"r1"}]});
    let b = brief(&t, "work");
    for part in [
        "Make the change",
        "`task done`, a summary in `text`",
        "Task: Add coolcalc\n\nA CLI calculator.\n",
        "- spec: http://x",
        "- human: Keep it small",
        "- Syntax?\n  Answer: b",
    ] {
        assert!(b.contains(part), "{part} missing from:\n{b}");
    }
    for noise in ["{", "r1", "rank"] {
        assert!(!b.contains(noise), "{noise} in:\n{b}");
    }
    for kind in ["work", "investigate", "review"] {
        let prompt = brief(&t, kind);
        for guidance in [
            "Write human-facing task messages in Markdown",
            "including the text sent to task tools",
            "relevant sections with short headings",
            "blank lines around headings and lists",
            "explain what you need and why it matters",
            "Keep messages short: lead with the answer or outcome",
        ] {
            assert!(prompt.contains(guidance), "{kind}: missing {guidance}");
        }
        assert_eq!(prompt.contains("**Assessment: Looks good**"), kind == "review");
        assert_eq!(prompt.contains("**Assessment: Needs work**"), kind == "review");
    }
    // D-119 point 6: check what is already answered before asking.
    for kind in ["work", "investigate"] {
        assert!(brief(&t, kind).contains("check the Notes so far and Questions below, if any"), "{kind}");
    }
    assert!(brief(&t, "investigate").contains("Investigate this task"));
    // Only a work run changes code, so only it is told to run every test before finishing.
    for kind in ["work", "investigate", "review", "discuss"] {
        assert_eq!(brief(&t, kind).contains("Before `task done`, run all tests of the projects this task covers"), kind == "work", "{kind}");
    }
    // A discussion keeps every note whole: no command reads a cut note's full text.
    let mut talk = t.clone();
    talk["notes"] = json!([
        {"author":"agent","text":"### Summary\n\nPlan A.\n\n### Details\n\nLong A.","run":null},
        {"author":"human","text":"### Mine\n\nKeep.\n\n### All\n\nOf it.","run":null},
        {"author":"agent","text":"### Summary\n\nPlan B.\n\n### Details\n\nLong B.","run":null}
    ]);
    let d = brief(&talk, "discuss");
    assert!(d.contains("Long A.") && d.contains("Keep.\n\n### All\n\nOf it.") && d.contains("Long B."), "{d}");
    let r = brief(&t, "review");
    assert!(
        r.contains("Task changes:\n- calc.add")
            && r.contains("`task note`, the feedback in `text`")
            && r.contains("`run` are refused")
            && !r.contains("task done")
    );
    assert!(!b.contains("Projects:"), "one project needs no Projects section");
    // A task across projects (D-61) says where the agent works and how it reaches the rest.
    let mut wide = t.clone();
    wide["scope"] = json!(["a", "web"]);
    wide["home"] = json!("a");
    wide["changes"][0]["project"] = json!("web");
    let w = brief(&wide, "work");
    assert!(w.contains("covers the projects a, web. You work in a; for another of them, add `--project <folder>`"), "{w}");
    assert!(w.contains("Task changes:\n- web: calc.add"), "{w}");
}

#[test]
fn unavailable_models_say_why() {
    let missing = json!({"agent":"codex","run":["solace-no-such-program"]});
    assert_eq!(unavailable(&missing).unwrap(), "solace-no-such-program is not installed");
    assert!(unavailable(&json!({"agent":"codex","run":["sh"]})).is_none());
    let agent = json!({"agent":"solace-no-such-agent","model":"m"});
    assert!(unavailable(&agent).unwrap().contains("not installed"));
}

/// What agents say when a usage limit is reached, as they say it (see its `about`).
const USAGE_LIMITS: &str = include_str!("board/usage-limits.json");
/// 2026-10-14 12:00 UTC.
const NOON: u64 = 1_791_979_200;
/// Time zones for the tests, so no process is spawned: offsets at the time
/// asked for. `Europe/London` steps from `+0100` to `+0000` a day after
/// `NOON`, as a daylight-saving change does, so a reset named for after it is
/// placed by the offset at the reset, not now's.
fn zones(z: Option<&str>, at: i64) -> Option<i64> {
    match z {
        Some("Europe/London") => Some(if at < (NOON + 86400) as i64 { 3600 } else { 0 }),
        Some("America/Toronto") => Some(-4 * 3600),
        Some("UTC") | None => Some(0),
        _ => None,
    }
}

#[test]
fn agents_say_when_a_usage_limit_is_reached_and_when_it_resets() {
    let fixture: Value = serde_json::from_str(USAGE_LIMITS).unwrap();
    for k in agents::KINDS {
        for case in fixture[k.name].as_array().unwrap() {
            let said = case["said"].as_str().unwrap();
            assert_eq!((k.limit)(said), Some(case["resets"].as_str().map(String::from)), "{said}");
        }
        for said in fixture["not_limits"].as_array().unwrap() {
            assert_eq!((k.limit)(said.as_str().unwrap()), None, "{said}");
        }
    }
    let at = |said: &str, now: u64| agents::reset_time(said, now, &zones);
    // A time alone is its next coming: later today, else tomorrow.
    assert_eq!(at("3:45 PM", NOON), Some(1_791_992_700));
    assert_eq!(at("3:45 PM", NOON + 5 * 3600), Some(1_792_079_100));
    assert_eq!(at("3pm", NOON), Some(1_791_990_000));
    assert_eq!(at("Oct 15th, 2026 9:05 AM", NOON), Some(1_792_055_100));
    // A zone in parentheses: its own clock, not this computer's.
    assert_eq!(at("2pm (UTC)", NOON), Some(1_791_986_400));
    assert_eq!(at("1am (Europe/London)", NOON), Some(1_792_022_400));
    assert_eq!(at("2:50pm (America/Toronto)", NOON), Some(1_792_003_800));
    assert_eq!(at("Oct 9, 2025 8:53 AM (UTC)", NOON), Some(1_759_999_980));
    // A date with no year whose day has passed this year is next year's; one
    // still to come this year stays this year.
    assert_eq!(at("Oct 14 2pm", NOON + 86400), Some(1_823_522_400));
    assert_eq!(at("Oct 16 9 am", NOON + 86400), Some(1_792_141_200));
    // The offset at the reset, not now's: London's change a day after NOON
    // moves a reset named for Oct 16 by an hour.
    assert_eq!(at("Oct 16 1 am (Europe/London)", NOON), Some(1_792_112_400));
    // A reset that can't be read, or in an unknown zone, has no time.
    assert_eq!(at("soon", NOON), None);
    assert_eq!(at("2pm (Mars/Olympus)", NOON), None);
    assert_eq!(agents::zone_offset(Some("UTC"), NOON as i64), Some(0));
    // A zone this computer does not know has no offset: `date +%z` would
    // answer `+0000` for it. A name that is a path into the zones is refused.
    assert_eq!(agents::zone_offset(Some("Mars/Olympus"), NOON as i64), None);
    assert_eq!(agents::zone_offset(Some("../etc"), NOON as i64), None);
    // A zone folder (such as `America`) is not a zone file: `date` answers
    // `+0000` for it, so it has no offset.
    assert_eq!(agents::zone_offset(Some("America"), NOON as i64), None);
}

#[test]
fn a_usage_limit_marks_the_sign_in_until_it_resets_or_a_run_succeeds() {
    let fixture: Value = serde_json::from_str(USAGE_LIMITS).unwrap();
    let dir = testing::Scratch::new();
    let (path, log) = (dir.join("agent-limits.json"), dir.join("log.jsonl"));
    let codex = json!({"agent":"codex","model":"m"});
    let said = fixture["codex"][3]["said"].as_str().unwrap();
    fs::write(&log, json!({"type":"error","message":said}).to_string()).unwrap();
    agents::observe(&path, &codex, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    let reason = "Codex usage limit reached (resets 3:45 PM)";
    assert_eq!(agents::limited(&path, "codex", NOON).as_deref(), Some(reason));
    assert_eq!(agents::limited(&path, "codex", 1_791_992_699).as_deref(), Some(reason));
    // It lifts at the reset time, and holds only Codex's sign-in.
    assert_eq!(agents::limited(&path, "codex", 1_791_992_700), None);
    assert_eq!(agents::limited(&path, "claude", NOON), None);
    // A model on its own endpoint, or with its own program, isn't on the sign-in.
    let other = dir.join("other.json");
    for m in [json!({"agent":"codex","model":"m","endpoint":{"base_url":"http://x"}}), json!({"agent":"codex","run":["sh"]})] {
        agents::observe(&other, &m, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    }
    assert!(!other.exists());
    // Claude Code's limit, in its stream-json result, with a reset it can't read: held until cleared.
    let claude = json!({"agent":"claude","model":"m"});
    fs::write(&log, json!({"type":"result","is_error":true,"result":"You've hit your limit · resets soon","usage":{}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    assert_eq!(agents::limited(&path, "claude", NOON + 365 * 86400).as_deref(), Some("Claude Code usage limit reached (resets soon)"));
    // The mark is on disk, not only in memory.
    agents::forget_marks();
    assert!(agents::limited(&path, "claude", NOON).is_some());
    // A run that got no answer leaves it.
    fs::write(&log, "").unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("finished"), &zones).unwrap();
    assert!(agents::limited(&path, "claude", NOON).is_some());
    // A run whose agent answered clears it: one that finished, started after
    // the mark was set.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::limited(&path, "claude", NOON), None);
    // A stopped run leaves the mark, even when its agent answered: it says
    // nothing about the sign-in.
    fs::write(&log, json!({"type":"result","is_error":true,"result":"You've hit your limit · resets soon","usage":{}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("stopped"), &zones).unwrap();
    assert!(agents::limited(&path, "claude", NOON).is_some());
    // A finished run started before the mark leaves it too: its answers came
    // before the limit was hit.
    agents::observe(&path, &claude, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    assert!(agents::limited(&path, "claude", NOON).is_some());
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::limited(&path, "claude", NOON), None);
    // A run that succeeded clears the mark whatever its successful outcome:
    // reviewed, discussed and planned count as `finished` does, the list
    // `supervise` ends a run with. A mark set after the run started stays.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    for o in ["reviewed", "discussed", "planned"] {
        agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some(o), &zones).unwrap();
        assert_eq!(agents::limited(&path, "claude", NOON), None, "{o}");
        agents::mark(&path, "claude", "Claude Code usage limit reached", Some(NOON + 3600)).unwrap();
        agents::observe(&path, &claude, &log, NOON, NOON, Some(o), &zones).unwrap();
        assert!(agents::limited(&path, "claude", NOON).is_some(), "{o}");
    }
    // Claude Code reports its limit as a message, not an error: a message that
    // is exactly the limit's own text marks the sign-in, and is never counted
    // as "the agent answered".
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your limit · resets 2pm (UTC)"}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    assert_eq!(agents::limited(&path, "claude", NOON).as_deref(), Some("Claude Code usage limit reached (resets 2pm (UTC))"));
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("finished"), &zones).unwrap();
    assert!(agents::limited(&path, "claude", NOON).is_some());
    // A passing mention inside a longer message marks nothing, and is an answer.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"I noted: You've hit your limit · resets 2pm (UTC)\nThen I carried on."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 10 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::limited(&path, "claude", NOON), None);
    // Try again clears a mark.
    assert!(agents::clear(&path, "codex").unwrap());
    assert!(!agents::clear(&path, "codex").unwrap());
    // Errors that aren't usage limits mark nothing.
    let none = dir.join("none.json");
    for e in fixture["not_limits"].as_array().unwrap() {
        fs::write(&log, json!({"type":"error","message":e}).to_string()).unwrap();
        agents::observe(&none, &codex, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    }
    assert!(!none.exists());
}

#[test]
fn sign_ins_are_detected_without_a_model_call() {
    let home = testing::Scratch::new();
    let none = |_: &str| false;
    let now = NOON * 1000;
    let check = |m: &Value, var: &dyn Fn(&str) -> bool| agents::local(m, &agents::Here { home: &home, var, installed: &|_| true, now });
    let (codex, claude) = (json!({"agent":"codex","model":"m"}), json!({"agent":"claude","model":"m"}));
    let missing = agents::Here { home: &home, var: &none, installed: &|_| false, now };
    assert_eq!(agents::local(&codex, &missing).unwrap(), "codex is not installed (not on PATH)");
    assert_eq!(check(&codex, &none).unwrap(), "codex is not signed in");
    // The keychain is asked on macOS only; elsewhere no file and no key is no sign-in.
    if !cfg!(target_os = "macos") {
        assert_eq!(check(&claude, &none).unwrap(), "claude is not signed in");
    }
    // A key variable is a sign-in.
    assert!(check(&codex, &|v| v == "OPENAI_API_KEY").is_none());
    assert!(check(&claude, &|v| v == "ANTHROPIC_API_KEY").is_none());
    // An endpoint needs only its own key.
    let endpoint = json!({"agent":"codex","model":"m","endpoint":{"base_url":"http://x","key_env":"LOCAL_KEY"}});
    assert_eq!(check(&endpoint, &none).unwrap(), "the endpoint's key variable LOCAL_KEY is not set");
    assert!(check(&endpoint, &|v| v == "LOCAL_KEY").is_none());
    // Codex's file: a token it can renew is a sign-in; neither key nor token has expired.
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/auth.json"), json!({"OPENAI_API_KEY":null,"tokens":{"refresh_token":"r"}}).to_string()).unwrap();
    assert!(check(&codex, &none).is_none());
    fs::write(home.join(".codex/auth.json"), json!({"OPENAI_API_KEY":null,"tokens":{}}).to_string()).unwrap();
    assert_eq!(check(&codex, &none).unwrap(), "Codex's sign-in has expired; sign in again");
    // Claude Code's file: a token past its time is renewed while there is a refresh token.
    fs::create_dir_all(home.join(".claude")).unwrap();
    let credentials = home.join(".claude/.credentials.json");
    fs::write(&credentials, json!({"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":now - 1}}).to_string()).unwrap();
    assert!(check(&claude, &none).is_none());
    fs::write(&credentials, json!({"claudeAiOauth":{"accessToken":"a","expiresAt":now + 1}}).to_string()).unwrap();
    assert!(check(&claude, &none).is_none());
    if !cfg!(target_os = "macos") {
        fs::write(&credentials, json!({"claudeAiOauth":{"accessToken":"a","expiresAt":now - 1}}).to_string()).unwrap();
        assert_eq!(check(&claude, &none).unwrap(), "Claude Code's sign-in has expired; sign in again");
    }
    // A file that says nothing about expiry counts as a sign-in.
    fs::write(&credentials, "not json").unwrap();
    assert!(check(&claude, &none).is_none());
}

#[test]
fn detections_are_cached_for_a_while() {
    let key = format!("test:{}", unique_id());
    let n = std::cell::Cell::new(0);
    let work = || {
        n.set(n.get() + 1);
        json!(n.get())
    };
    let minute = Duration::from_secs(60);
    assert_eq!(agents::cached_for(&key, minute, work), json!(1));
    assert_eq!(agents::cached_for(&key, minute, work), json!(1));
    assert_eq!(agents::cached_for(&key, Duration::ZERO, work), json!(2));
}

/// D-103: a revert restores every project before it checks any. Here `web`'s
/// restored code calls `components`' restored `lib.z`, and `web` is checked
/// first: checked before `components` was restored, it would not type-check.
#[test]
fn revert_restores_every_project_before_checking_any() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"web\", \"components\"]\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
        ("web/src/app.gleam", "import lib\n\npub fn f() -> Int {\n  lib.z()\n}\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        ("components/src/lib.gleam", "pub fn z() -> Int {\n  0\n}\n"),
    ]);
    for d in ["web", "components"] {
        crate::import(&root.join(d)).unwrap();
    }
    let (web, comp) = (open(&root.join("web")).unwrap(), open(&root.join("components")).unwrap());
    let text = |s: &Store, name: &str| {
        let ix = s.index().unwrap();
        ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
    };
    let mut t = api(&web.root, "POST", "/tasks", json!({"title": "Both"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["scope"] = json!(["web", "components"]);
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&web, &t).unwrap();
    assert_eq!(scope_stores(&web, &t).unwrap().first().map(|p| p.0.as_str()), Some("web"), "web is restored and checked first");
    // The task stops calling lib.z in web, then changes its type in components.
    let (bw, bc) = (snapshot(&web).unwrap(), snapshot(&comp).unwrap());
    web.submit_symbols(Some("app.f"), None, "pub fn f() -> Int {\n  1\n}").unwrap();
    comp.submit_symbols(Some("lib.z"), None, "pub fn z() -> String {\n  \"0\"\n}").unwrap();
    record(&web, &tid, "r1", &bw, &snapshot(&web).unwrap(), None).unwrap();
    record(&comp, &tid, "r1", &bc, &snapshot(&comp).unwrap(), None).unwrap();
    end_runs(&mut t, "finished");
    tasks::put(&web, &t).unwrap();
    api(&web.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert_eq!(text(&web, "app.f"), "pub fn f() -> Int {\n  lib.z()\n}");
    assert_eq!(text(&comp, "lib.z"), "pub fn z() -> Int {\n  0\n}");
}

/// D-123: a run starts in the project of its task that holds the most
/// symbols the task names; on a tie, or with none, at home. A resumed
/// conversation keeps its recorded project; one saved before projects were
/// recorded restarts at the home, where its session began.
#[test]
fn a_run_starts_where_the_named_symbols_are() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ("b/src/plan.gleam", "pub fn answer() -> Int {\n  42\n}\n"),
    ]);
    for d in ["a", "b"] {
        crate::import(&root.join(d)).unwrap();
    }
    let t = |body: &str| json!({"title": "T", "body": body, "scope": ["a", "b"]});
    let a = root.join("a");
    assert_eq!(start_project(&a, &t("Change `plan.answer`.")), "b");
    assert_eq!(start_project(&a, &t("app.one and plan.answer")), "a", "a tie stays home");
    assert_eq!(start_project(&a, &t("No names here")), "a");
    assert_eq!(start_project(&a, &json!({"title": "plan.answer"})), "a", "outside its scope");
    assert_eq!(run_project(&json!({"project": "a"}), true, &a, &t("Change `plan.answer`.")), "a", "a resumed run keeps its recorded project");
    assert_eq!(run_project(&json!({}), true, &a, &t("Change `plan.answer`.")), "a", "a conversation saved before projects were recorded restarts at the home");
    assert_eq!(run_project(&json!({}), false, &a, &t("Change `plan.answer`.")), "b", "a fresh run starts where its symbols are");
}

/// D-103: a moved symbol's heading names the project that gained it, not
/// another one that held a symbol of the same id all along.
#[test]
fn moved_to_names_the_project_that_gained_the_symbol() {
    let lib = "pub fn z() -> Int {\n  0\n}\n";
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\", \"c\"]\n"),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"b\"\nversion = \"1.0.0\"\n"),
        ("b/src/lib.gleam", lib),
        ("c/gleam.toml", "name = \"c\"\nversion = \"1.0.0\"\n"),
        ("c/src/lib.gleam", lib),
    ]);
    let projects: Vec<(String, Store)> = ["a", "b", "c"]
        .into_iter()
        .map(|d| {
            crate::import(&root.join(d)).unwrap();
            (d.to_string(), open(&root.join(d)).unwrap())
        })
        .collect();
    let key = |s: &Store| {
        let ix = s.index().unwrap();
        format!("symbol:{}", ix.syms[s.resolve(&ix, "lib.z").unwrap()].id)
    };
    let k = key(&projects[1].1);
    let id = |k: &str| k.strip_prefix("symbol:").unwrap().to_string();
    projects[2].1.db.execute("UPDATE symbols SET id=?1 WHERE id=?2", [id(&k), id(&key(&projects[2].1))]).unwrap();
    assert_eq!(moved_to(&projects, "a", &k, "t1").as_deref(), Some("b"));
    projects[2].1.db.execute("INSERT INTO board_before VALUES ('t1',?1,'null')", [&k]).unwrap();
    assert_eq!(moved_to(&projects, "a", &k, "t1").as_deref(), Some("c"));
}

/// Putting back a file the task added removes the folders that leaves empty,
/// as those a file `extract` carried made (D-103), and keeps `src/`.
#[test]
fn restoring_an_added_file_removes_the_folders_it_leaves_empty() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::create_dir_all(s.root.join("src/ui/ffi")).unwrap();
    fs::write(s.root.join("src/ui/ffi/x.mjs"), "export const x = 1\n").unwrap();
    restore(&s, &[("file:src/ui/ffi/x.mjs".to_string(), json!(null))]).unwrap();
    assert!(!s.root.join("src/ui").exists());
    assert!(s.root.join("src/app.gleam").exists());
}

/// A task adds `file`, recorded with the folders found before it or, as an old
/// record, without; then it is canceled.
fn cancel_an_added_file(s: &Store, file: &str, folders: bool) {
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Folders"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let found = folders.then(|| existing_folders(&s.root));
    let key = format!("file:{file}");
    let (mut before, mut after) = (json!({}), json!({}));
    before[&key] = Value::Null;
    fs::create_dir_all(s.root.join(file).parent().unwrap()).unwrap();
    fs::write(s.root.join(file), "export const x = 1\n").unwrap();
    after[&key] = json!(b"export const x = 1\n".to_vec());
    record(s, &tid, "r1", &before, &after, found.as_ref()).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert!(!s.root.join(file).exists());
}

/// D-136: undo keeps a folder that was there before the task, even empty.
#[test]
fn reverting_keeps_an_empty_folder_that_was_there_before() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::create_dir_all(s.root.join("src/ui")).unwrap();
    cancel_an_added_file(&s, "src/ui/x.mjs", true);
    assert!(s.root.join("src/ui").is_dir());
}

/// D-136: a folder the task made still goes; the one above it, there before, stays.
#[test]
fn reverting_removes_a_folder_the_task_made() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::create_dir_all(s.root.join("src/ui")).unwrap();
    cancel_an_added_file(&s, "src/ui/ffi/x.mjs", true);
    assert!(!s.root.join("src/ui/ffi").exists());
    assert!(s.root.join("src/ui").is_dir());
}

/// D-136: an old record, with no folders, removes every folder left empty.
#[test]
fn reverting_an_old_record_removes_every_empty_folder() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::create_dir_all(s.root.join("src/ui")).unwrap();
    cancel_an_added_file(&s, "src/ui/x.mjs", false);
    assert!(!s.root.join("src/ui").exists());
    assert!(s.root.join("src/app.gleam").exists());
}

/// A task adds module `ui/deep/new` (its file `src/ui/deep/new.gleam`), recorded
/// with the folders found before it; then it is canceled.
fn cancel_an_added_module(s: &Store) {
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Module folders"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let found = existing_folders(&s.root);
    let before = snapshot(s).unwrap();
    s.db.execute("INSERT INTO modules SELECT 'ui/deep/new',dir,header FROM modules WHERE path='app'", []).unwrap();
    s.db.execute("INSERT INTO symbols(id,module,name,kind,text,ord,revision,live) SELECT 'new1','ui/deep/new',name,kind,text,ord,revision,live FROM symbols WHERE module='app'", []).unwrap();
    s.materialize(&s.index().unwrap()).unwrap();
    assert!(s.root.join("src/ui/deep/new.gleam").exists());
    let after = snapshot(s).unwrap();
    record(s, &tid, "r1", &before, &after, Some(&found)).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert!(!s.root.join("src/ui/deep/new.gleam").exists());
}

/// D-136 for module files: undo keeps an empty folder that was there before
/// the task, and removes the one the task made below it.
#[test]
fn reverting_an_added_module_keeps_the_folder_that_was_there() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::create_dir_all(s.root.join("src/ui")).unwrap();
    cancel_an_added_module(&s);
    assert!(!s.root.join("src/ui/deep").exists());
    assert!(s.root.join("src/ui").is_dir());
}

/// D-136 for module files: folders the task made all go, up to `src/`.
#[test]
fn reverting_an_added_module_removes_the_folders_it_made() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    cancel_an_added_module(&s);
    assert!(!s.root.join("src/ui").exists());
    assert!(s.root.join("src/app.gleam").exists());
}

/// An ignored folder is listed but not walked: undo stops at it, and its
/// contents, which can be large (`target/`), aren't read.
#[test]
fn existing_folders_lists_an_ignored_folder_without_its_contents() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::write(s.root.join(".gitignore"), "target/\n").unwrap();
    fs::create_dir_all(s.root.join("target/a/b")).unwrap();
    fs::create_dir_all(s.root.join("src/x")).unwrap();
    let found = existing_folders(&s.root);
    assert!(found.contains("target") && found.contains("src/x") && found.contains("src"));
    assert!(!found.contains("target/a") && !found.contains("target/a/b"));
}

/// A folder's `.gitignore` is matched from that folder: `/gen` in
/// `src/.gitignore` ignores `src/gen`, which is listed but not walked.
#[test]
fn existing_folders_reads_a_nested_gitignore_from_its_folder() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    fs::write(s.root.join("src/.gitignore"), "/gen\n").unwrap();
    fs::create_dir_all(s.root.join("src/gen/a")).unwrap();
    let found = existing_folders(&s.root);
    assert!(found.contains("src/gen"));
    assert!(!found.contains("src/gen/a"));
}

/// D-127: while its agent works, a task's scope widens, and the run claims
/// the added project, but doesn't narrow.
#[test]
fn a_working_tasks_scope_widens_but_does_not_narrow() {
    let dirs = ["web", "components", "busy", "free"];
    let mut files = vec![("solace.toml".to_string(), "projects = [\"web\", \"components\", \"busy\", \"free\"]\n".to_string())];
    for d in dirs {
        files.push((format!("{d}/gleam.toml"), format!("name = \"{d}\"\nversion = \"1.0.0\"\n")));
        files.push((format!("{d}/src/{d}.gleam"), "pub fn f() -> Int {\n  1\n}\n".to_string()));
    }
    let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let root = testing::Scratch::new().with(&files);
    for d in dirs {
        crate::import(&root.join(d)).unwrap();
    }
    let (web, busy) = (open(&root.join("web")).unwrap(), open(&root.join("busy")).unwrap());
    let mut other = api(&busy.root, "POST", "/tasks", json!({"title": "Busy"})).unwrap();
    other["runs"] = json!([{"id": "r0", "kind": "work", "state": "running"}]);
    tasks::put(&busy, &other).unwrap();
    let mut t = api(&web.root, "POST", "/tasks", json!({"title": "Wide"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let created = t["createdTime"].clone();
    t["scope"] = json!(["components", "web"]);
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&web, &t).unwrap();
    let url = format!("/tasks/{tid}/scope");
    let narrow = api(&web.root, "POST", &url, json!({"scope": ["web"]})).unwrap_err().0;
    assert!(narrow.contains("E7003") && narrow.contains("you can add projects now"), "{narrow}");
    let held = api(&web.root, "POST", &url, json!({"scope": ["busy", "components", "web"]})).unwrap_err().0;
    assert!(held.contains("E7005"), "{held}");
    let wide = api(&web.root, "POST", &url, json!({"scope": ["components", "free", "web"]})).unwrap();
    assert_eq!(wide["scope"], json!(["components", "free", "web"]));
    // Waiting on a question, it narrows again.
    let mut t = task(&web.root, &tid).unwrap();
    t["runs"][0]["state"] = json!("waiting");
    tasks::put(&web, &t).unwrap();
    assert_eq!(api(&web.root, "POST", &url, json!({"scope": ["web"]})).unwrap()["scope"], json!(["web"]));
    // D-130: once the run ends, a move to another project keeps when the task was created.
    let mut t = task(&web.root, &tid).unwrap();
    t["runs"][0]["state"] = json!("ended");
    tasks::put(&web, &t).unwrap();
    let moved = api(&web.root, "POST", &url, json!({"scope": ["components"]})).unwrap();
    assert_eq!((&moved["home"], &moved["createdTime"]), (&json!("components"), &created));
}
