use super::*;

/// A task with one run `r1` in `run`'s state, and a hung agent in its own process group.
fn supervised(run: Value) -> (testing::Project, String, std::process::Child) {
    launched(run, Command::new("sleep").arg("45").process_group(0).spawn().unwrap())
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

/// D-151: a fresh work or investigate brief holds the session start reply in
/// place of the line asking for it, before the stop line; a resumed brief, a
/// review's and a discussion's don't, and a project of plain files gets its own reply.
#[test]
fn a_fresh_brief_holds_the_session_start_reply() {
    let p = testing::project(&[("src/app.gleam", "/// Totals an order.\npub fn total() -> Int {\n  1\n}\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"Fix the order total","body":"app.total is off by one"})).unwrap();
    let brief = |kind: &str, resumed: bool| continuation_brief(&t, &json!({"id":"r1","kind":kind}), resumed, &p.root, &p.root, false).0;
    let (reply, next) = (
        "\nSession start (the reply to `session start --task`):\n",
        "\nNext: read what the change needs in one call, one command per line: `inspect app.total",
    );
    for kind in ["work", "investigate"] {
        let b = brief(kind, false);
        assert!(b.contains(reply) && b.contains("Symbols most likely relevant to the task") && b.contains("-- app.total"), "{kind}: {b}");
        assert!(!b.contains(START) && b.contains(HELD_START), "{kind}: {b}");
        let (at, stop) = (b.find(next).expect(&b), b.find("After the call that ends the run returns, stop").unwrap());
        assert!(b.find(reply).unwrap() < at && at < stop, "{kind}: the reply's first call comes just before the stop line: {b}");
        assert!(!b.contains("Feature (the feature") && !b.contains("Depends on"), "{kind}: {b}");
    }
    for kind in ["review", "discuss"] {
        let b = brief(kind, false);
        assert!(!b.contains(reply) && !b.contains(HELD_START), "{kind}: {b}");
    }
    let resumed = brief("work", true);
    assert!(
        resumed.contains("Use the continuation context below.") && !resumed.contains(reply) && !resumed.contains(HELD_START),
        "{resumed}"
    );
    fs::remove_file(p.root.join("gleam.toml")).unwrap();
    let plain = brief("work", false);
    assert!(
        plain.contains(reply) && plain.contains("This project has plain files") && plain.contains("`grep <text> --in <path>`"),
        "{plain}"
    );
    // A folder with no store keeps the line asking for the call.
    let bare = testing::Scratch::new();
    let b = continuation_brief(&t, &json!({"id":"r1","kind":"work"}), false, &p.root, &bare, false).0;
    assert!(b.contains(START) && !b.contains(reply), "{b}");
}

/// D-138: a task's feature section names the feature and carries the latest
/// of its investigation report, assignment note and review feedback, cut at
/// a line end within the cap; no feature, no section.
#[test]
fn a_brief_carries_the_features_latest_direction() {
    let top = testing::Scratch::new();
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
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
    let note = concat!(
        "**Assessment: Looks good**\n\n- not debt\n\n### Tech debt\n\nIntro line.\n",
        "- `app.main`: name the constant\n* `app.other`: long\n  second line\n1. `a.rs`: third\n\n",
        "### Verification\n\n- tests pass\n",
    );
    assert_eq!(tech_debt_items(note), vec!["`app.main`: name the constant", "`app.other`: long\n  second line", "`a.rs`: third"]);
    assert_eq!(tech_debt_items("## Tech Debt:\n- one\n"), vec!["one"]);
    assert!(tech_debt_items("### Tech debt\n\n### Next\n- x\n").is_empty());
    assert!(tech_debt_items("**Assessment: Looks good**\n- x\n").is_empty());
    // D-199: the example in the brief reads as one item in the exact shape.
    assert_eq!(
        tech_debt_items(TECH_DEBT_ASK),
        vec!["`board.brief`: <what is wrong>. Change it to <what> because <why>.\n  Check: <test or command and its expected result>."]
    );
}

/// D-112: completing a task in a feature files its last review's tech debt
/// on the feature's open Tech Debt task: made once, appended to after, and
/// made anew once that one is complete. A cancel or a review with no section
/// files nothing.
#[test]
fn completing_a_task_files_its_reviews_tech_debt() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
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
    let texts = |t: &Value| {
        let notes = t["notes"].as_array().unwrap();
        notes.iter().map(|n| n["text"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    let two = concat!(
        "**Assessment: Looks good**\n\n### Tech debt\n\n",
        "- `app.main`: name the constant\n- `app.other`: split it\n\n",
        "### Verification\n\n- tests pass\n",
    );
    let a = reviewed("A", true, two);
    let done = act(&a, "complete");
    let [debt] = debts().try_into().unwrap();
    let did = debt["id"].as_str().unwrap().to_string();
    assert_eq!(
        (debt["title"].as_str(), debt["column"].as_str(), debt["feature"].as_str()),
        (Some("Search: Tech Debt"), Some("planning"), Some(fid.as_str()))
    );
    assert_eq!(debt["scope"], done["scope"]);
    let notes = texts(&debt);
    assert_eq!(notes.len(), 2);
    assert!(
        notes[0].contains(&format!("task:{a}")) && notes[0].contains("rvA") && notes[0].ends_with("`app.main`: name the constant"),
        "{notes:?}"
    );
    assert!(notes[1].ends_with("`app.other`: split it"));
    assert!(texts(&done).last().unwrap().contains(&format!("Filed 2 tech-debt items from the last review on task:{did}")));
    // A second task in the feature appends to the same Tech Debt task.
    let b = reviewed("B", true, "### Tech debt\n- `app.b`: one more\n");
    act(&b, "complete");
    let [debt] = debts().try_into().unwrap();
    assert_eq!(debt["id"], did.as_str());
    assert!(texts(&debt)[2].contains(&format!("task:{b}")));
    // A cancel or no section files nothing.
    act(&reviewed("D", true, two), "cancel");
    act(&reviewed("E", true, "**Assessment: Looks good**\n- fine\n"), "complete");
    let [debt] = debts().try_into().unwrap();
    assert_eq!(texts(&debt).len(), 3);
    // A completed Tech Debt task is not reopened: the next finding makes a new one.
    act(&did, "complete");
    act(&reviewed("F", true, two), "complete");
    let open: Vec<Value> = debts().into_iter().filter(|d| d["id"] != did.as_str() && d["feature"] == fid.as_str()).collect();
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
    let forced = supervise(
        Watch::Task(&p.root, &tid),
        "r1",
        &mut child,
        &log,
        &json!({}),
        false,
        Duration::from_secs(3600),
        Duration::from_millis(300),
    )
    .unwrap();
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
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap()
    );
    assert!(start.elapsed() < Duration::from_secs(10));
}

/// An interrupted run's agent is killed at once; a running one past its timeout ends the run.
#[test]
fn interrupted_and_timed_out_agents_are_killed() {
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"interrupted"}));
    let log = p.root.join(".solace/r1.log");
    let start = Instant::now();
    assert!(
        supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap()
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"running"}));
    let log = p.root.join(".solace/r1.log");
    assert!(
        supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_millis(200), GRACEFUL_EXIT)
            .unwrap()
    );
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
    t["runs"] = json!([
        {"id":"r1","kind":"review","state":"running"},
        {"id":"r2","kind":"work","state":"ended"},
        {"id":"r3","kind":"investigate","state":"running"},
        {"id":"r4","kind":"discuss","state":"running"}
    ]);
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
    // Investigations and discussions run marked commands as reviews do (D-161).
    for rid in ["r3", "r4"] {
        assert!(guard(rid, &["run", "safe"]).is_ok(), "{rid}");
        assert!(guard(rid, &["run", "e2e"]).unwrap_err().contains("allowed_in_reviews"), "{rid}");
        assert!(guard(rid, &["write", "x"]).unwrap_err().contains("this run is read-only"), "{rid}");
    }
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
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("skills/design/SKILL.md", "---\nname: design\ndescription: Layout and color.\n---\n"),
    ]);
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
    assert!(
        listed.contains(&format!("{tid} {} f:{fid} m:{mid} T", t["column"].as_str().unwrap()))
            && listed.contains(&format!("Features:\n{fid} proposed m:{mid} F"))
            && listed.contains(&format!("Milestones:\n{mid} proposed #1 M"))
            && listed.contains("Vision:"),
        "{listed}"
    );
    fs::write(
        store_dir(&top).join("t9.json"),
        json!({"id":"t9","title":"Gone","column":"complete","closedTime":8,"archivedTime":9}).to_string(),
    )
    .unwrap();
    assert!(!strategy_list(&top, false).unwrap().contains("t9") && strategy_list(&top, true).unwrap().contains("t9 complete f:- m:- Gone"));
    fs::write(top.join("DECISIONS.md"), "# Decisions\n## D-1 First thing\nWhy: x\n## Notes\n").unwrap();
    let brief = strategy_brief(&top, &json!({"context":{"kind":"feature","id":fid},"notes":[{"author":"human","text":"Plan F"}]}));
    assert!(brief.contains(&format!("This run is about the feature {fid}")) && brief.contains("\"title\": \"F\""), "{brief}");
    assert!(brief.contains("A `title` names the work in a few words") && brief.contains("go in `body`"), "{brief}");
    assert!(
        brief.contains("Propose the smallest version that meets the need") && brief.contains("Say once if the need or vision"),
        "{brief}"
    );
    assert!(brief.contains("in short Markdown: lead with the answer"), "{brief}");
    assert!(brief.contains(super::strategy::STRATEGY_ROLE), "{brief}");
    // Every strategy run reads the request literally, assumes rather than asks,
    // separates its recommendation, invents no preconditions and stays conversational.
    for guidance in [
        "Read the human's message literally",
        "make the smallest grounded assumption, name it in one line",
        "merely a cursory suggestion, not a prescription",
        "Don't make measurement, investigation, benchmarking or another step a precondition",
        "Keep notes conversational",
    ] {
        assert!(brief.contains(guidance), "{guidance}: {brief}");
    }
    // D-148: records are named as links the human can click.
    assert!(
        brief.contains(concat!(
            "name tasks, features and milestones as `task:<id>`, `feature:<id>` or `milestone:<id>`, ",
            "written bare and never inside backticks",
        )),
        "{brief}"
    );
    assert!(brief.contains("- D-1 First thing") && !brief.contains("- Notes") && brief.contains("- human: Plan F"), "{brief}");
    // D-181: cite code by its printed symbol name.
    assert!(brief.contains("lockdown.dispatch") && !brief.contains("board.rs::within"), "{brief}");
    assert!(brief.contains("`--in <path>`") && brief.contains("`--context n`") && brief.contains("`--failed`"), "{brief}");
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

/// D-158 point 2: a review lists a changed symbol that added `unsafe`, and
/// marks one with no `// SAFETY:` comment; a symbol that gains none is left out.
#[test]
fn a_review_lists_new_unsafe() {
    let (root, s) =
        testing::rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n");
    let mut t = api(&root, "POST", "/tasks", json!({"title": "Unsafe"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"work","kind":"work","state":"ended"}]);
    tasks::put(&s, &t).unwrap();
    let before = snapshot(&s).unwrap();
    let row = |name: &str| -> String { s.db.query_row("SELECT id FROM symbols WHERE name=?1", [name], |r| r.get(0)).unwrap() };
    s.db.execute("UPDATE symbols SET text=?1 WHERE id=?2", params!["pub fn one() -> i32 {\n    unsafe { 1 }\n}\n", row("one")]).unwrap();
    s.db.execute(
        "UPDATE symbols SET text=?1 WHERE id=?2",
        params!["pub fn two() -> i32 {\n    // SAFETY: nothing is dereferenced.\n    unsafe { 2 }\n}\n", row("two")],
    )
    .unwrap();
    s.db.execute("UPDATE symbols SET text=?1 WHERE id=?2", params!["pub fn three() -> &'static str {\n    \"unsafe\"\n}\n", row("three")])
        .unwrap();
    let after = snapshot(&s).unwrap();
    record_listed(&s, &tid, &before, &after);
    let diff = task_diff(&s, &tid, &[], 4000).unwrap();
    assert!(diff.contains("New `unsafe`:\n"), "{diff}");
    assert!(diff.contains("- crate.one (no `// SAFETY:` comment)"), "{diff}");
    assert!(diff.contains("- crate.two\n"), "{diff}");
    assert!(!diff.contains("crate.two (no `// SAFETY:`"), "{diff}");
    assert!(!diff.contains("- crate.three"), "{diff}");
}

/// A review's brief holds the task's changes: the whole task at first, then
/// the delta since the last completed review; it never asks for `task diff`
/// or a session start first.
#[test]
fn a_review_brief_holds_the_task_diff() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Review diff"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"work","kind":"work","state":"ended"}]);
    tasks::put(&s, &t).unwrap();
    fs::write(s.root.join("first.txt"), "first").unwrap();
    record_listed(&s, &tid, &json!({"file:first.txt":null}), &json!({"file:first.txt":b"first".to_vec()}));
    let brief = |t: &Value| continuation_brief(t, &json!({"id":"r1","kind":"review"}), false, &p.root, &p.root, false).0;
    t = task(&s.root, &tid).unwrap();
    let first = brief(&t);
    assert!(first.contains("first.txt (added)") && first.contains("(the whole task)"), "{first}");
    t["runs"].as_array_mut().unwrap().push(json!({"id":"review","kind":"review","state":"ended","outcome":"reviewed"}));
    let mut checkpoint = review_state(&s, &t).unwrap();
    checkpoint["run"] = json!("review");
    checkpoint["changed_during_review"] = json!(false);
    let path = review_path(&s.root, &tid, "review").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    tasks::put(&s, &t).unwrap();
    fs::write(s.root.join("later.txt"), "later").unwrap();
    record_listed(&s, &tid, &json!({"file:later.txt":null}), &json!({"file:later.txt":b"later".to_vec()}));
    let followup = brief(&task(&s.root, &tid).unwrap());
    assert!(followup.contains("Since completed review review") && followup.contains("later.txt (added)"), "{followup}");
    assert!(!followup.contains("first.txt (added)"), "{followup}");
    for b in [&first, &followup] {
        assert!(!b.contains("Start with") && !b.contains(START) && !b.contains(HELD_START), "{b}");
    }
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

/// D-158/D-166: undo carries a module's `file` and a nested symbol's parent, separator and tail,
/// so a Rust module keeps its own file and its nesting.
#[test]
fn undo_keeps_a_rust_modules_file_and_nested_items() {
    fn at<'a>(ix: &'a Index, name: &str) -> &'a Sym {
        ix.syms.iter().find(|s| s.name == name).unwrap()
    }
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let split = language::rust::split("impl Outer {\n    fn inner() {}\n}\n", "x").unwrap();
    store_rust_file(&s.db, "x", "src", "src/other.rs", &split).unwrap();
    let before = s.index().unwrap();
    let snap = snapshot(&s).unwrap();
    let ids: Vec<String> = before.syms.iter().filter(|s| s.module == "x").map(|s| s.id.clone()).collect();
    // The undo puts back the rows the task removed.
    s.db.execute("DELETE FROM modules WHERE path = 'x'", []).unwrap();
    s.db.execute("DELETE FROM symbols WHERE module = 'x'", []).unwrap();
    let mut states = vec![("module:x".to_string(), snap["module:x"].clone())];
    states.extend(ids.iter().map(|id| (format!("symbol:{id}"), snap.get(format!("symbol:{id}")).unwrap().clone())));
    restore(&s, &states).unwrap();
    let ix = s.index().unwrap();
    let m = ix.modules.iter().find(|m| m.path == "x").unwrap();
    assert_eq!(m.file.as_deref(), Some("src/other.rs"));
    assert_eq!(m.rel(), "src/other.rs");
    assert_eq!(at(&ix, "Outer::inner").parent.as_deref(), Some(at(&ix, "Outer").id.as_str()));
    assert_eq!(
        (at(&ix, "Outer").sep.clone(), at(&ix, "Outer").tail.clone()),
        (at(&before, "Outer").sep.clone(), at(&before, "Outer").tail.clone())
    );
    assert_eq!(
        (at(&ix, "Outer::inner").sep.clone(), at(&ix, "Outer::inner").tail.clone()),
        (at(&before, "Outer::inner").sep.clone(), at(&before, "Outer::inner").tail.clone())
    );
}

/// A snapshot carries a Rust module's `owns_dir`, so undo restores the recorded
/// value instead of falling back to the guess (D-168).
#[test]
fn a_snapshot_keeps_a_modules_owns_dir() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut split = language::rust::split("pub fn one() {}\n", "x").unwrap();
    split.owns_dir = true;
    store_rust_file(&s.db, "x", "src", "src/other.rs", &split).unwrap();
    let snap = snapshot(&s).unwrap();
    assert_eq!(snap["module:x"][4], json!(1), "the snapshot keeps the row's owns_dir");
    s.db.execute("DELETE FROM modules WHERE path = 'x'", []).unwrap();
    restore(&s, &[("module:x".to_string(), snap["module:x"].clone())]).unwrap();
    let ix = s.index().unwrap();
    assert_eq!(ix.modules.iter().find(|m| m.path == "x").unwrap().owns_dir, Some(true));
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
    fs::write(
        s.root.join(".solace/pending/notes.json"),
        json!({"task": tid, "run": "r1", "path": "notes.txt", "bytes": b"old\n".to_vec()}).to_string(),
    )
    .unwrap();
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
    assert_eq!(
        (back["column"].as_str(), &back["archivedTime"], &back["closedTime"]),
        (Some("planning"), &Value::Null, &cancelled["closedTime"])
    );
    assert!(t["createdTime"].is_u64());
    assert_eq!(back["createdTime"], t["createdTime"], "D-130: bringing it back keeps when it was created");
    assert_eq!(back["title"], "Shapes");
    let last = back["notes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["author"].as_str(), last["text"].as_str()),
        (Some("solace"), Some("The human brought this task back from Canceled to Planning."))
    );
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
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
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
/// from Review it keeps the task's changes and clears its approval.
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
const DONE_IS_FINAL: &str = concat!(
    "[board]\nevents = { asked = \"todo\", finished = \"done\", failed = \"todo\" }\n",
    "[[board.columns]]\nid = \"todo\"\nactions = [\"cancel\"]\n",
    "[[board.columns]]\nid = \"done\"\nfinal = true\n",
    "[[board.columns]]\nid = \"gone\"\nfinal = true\n",
    "[board.actions]\ncancel = { revert = true, to = \"gone\" }\n",
);

/// D-132 point 5: in a workflow with no `planning` column, a new Tech Debt task
/// is made in its first column, whether or not the completed task has a feature.
#[test]
fn tech_debt_goes_to_the_first_column_without_planning() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"A"})).unwrap();
    t["feature"] = f["id"].clone();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"rv","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut t, "agent", Some("rv"), "### Tech debt\n- `app.main`: name the constant\n");
    let (debt, n) = tech_debt(&top, &t).unwrap().unwrap();
    assert_eq!((debt["column"].as_str(), n), (Some("todo"), 1));
    let mut loose = api(&s.root, "POST", "/tasks", json!({"title":"Loose"})).unwrap();
    loose["runs"].as_array_mut().unwrap().push(json!({"id":"rv2","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut loose, "agent", Some("rv2"), "### Tech debt\n- `app.other`: split it\n");
    let (own, n) = tech_debt(&top, &loose).unwrap().unwrap();
    assert_eq!((own["column"].as_str(), n), (Some("todo"), 1));
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
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
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
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
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
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
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

/// A strategy run and a run outside a task have no task run: a task run has
/// both ids, and a strategy run's task id is `strategy`.
#[test]
fn a_strategy_run_and_a_run_outside_a_task_have_no_task_run() {
    assert_eq!(task_ids(None, None), None);
    assert_eq!(task_ids(Some("strategy".into()), Some("r".into())), None);
    assert_eq!(task_ids(Some("t".into()), None), None);
    assert_eq!(task_ids(Some("t".into()), Some("r".into())), Some(("t".into(), "r".into())));
}

/// D-157: a reads message lands on its run; the outline keeps the latest steps that fit.
#[test]
fn a_reads_message_lands_on_its_run_and_the_outline_keeps_the_latest_steps() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    let reads = json!({"symbols":[{"project":".","symbol":"a.f","revision":"sha256:1","recent":true}]});
    message(&w, &mut t, &json!({"kind":"reads","task":"t","run":"r1","reads":reads})).unwrap();
    assert_eq!(t["runs"][0]["reads"], reads);
    assert!(message(&w, &mut t, &json!({"kind":"reads","task":"t","run":"r2","reads":reads})).is_err());
    let steps = [
        json!({"kind":"tool","title":"view a.f","status":"completed"}),
        json!({"kind":"usage"}),
        json!({"kind":"message","text":"Done.\nmore"}),
    ];
    assert_eq!(outline(&steps, 1000), "1 tool: view a.f (completed)\n3 message: Done.\n");
    assert_eq!(outline(&steps, 60), "Earlier steps left out: 1\n3 message: Done.\n");
    assert_eq!(outline(&steps, 10), "");
    assert_eq!(outline(&[], 1000), "");
}

/// `session remember` is for a task run: a discussion's brief holds its
/// context, so it is refused, and a run that isn't there is too.
#[test]
fn session_remember_is_refused_in_a_discussion_and_for_an_unknown_run() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"discuss","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let e = remember(&p.root, tid, "r1", 1000).unwrap_err();
    assert!(e.0.starts_with("E7010"), "{}", e.0);
    let e = remember(&p.root, tid, "r9", 1000).unwrap_err();
    assert!(e.0.starts_with("E7003"), "{}", e.0);
}

/// A resumed brief drops notes the conversation already saw; `session
/// remember` still gives them, so a compacted run can read its task again.
#[test]
fn session_remember_keeps_notes_a_resumed_brief_filters_out() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["notes"] = json!([{"author":"agent","text":"The investigator's plan."}]);
    t["conversation"] = json!({"notes_seen":[note_fingerprint(&t["notes"][0])]});
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let brief = continuation_brief(&t, &run, true, &p.root, &p.root, false).0;
    assert!(!brief.contains("The investigator's plan."), "{brief}");
    let out = remember(&p.root, tid, "r1", 10_000).unwrap();
    assert!(out.contains("The investigator's plan."), "{out}");
}

/// A message for a run that has ended keeps its note and changes nothing else.
#[test]
fn a_late_agent_message_does_not_wake_an_ended_run() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"ended","outcome":"stopped"}]);
    let before = t["column"].clone();
    for command in ["ask", "done", "note"] {
        let m = json!({
            "kind": "agent",
            "task": "t",
            "run": "r1",
            "command": command,
            "text": "late",
            "question": {"id": "q", "text": "?", "options": ["a", "b"]},
        });
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
    fs::write(
        store_dir(&root).join("t1.json"),
        r#"{"id":"t1","column":"backlog","home":".","runs":[],"notes":[],"questions":[],"links":[]}"#,
    )
    .unwrap();
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
        json!({
            "type": "item.completed",
            "item": {
                "id": "i1",
                "type": "mcp_tool_call",
                "tool": "solace",
                "arguments": {"command": "view a"},
                "result": {"content": [{"type": "text", "text": "pub fn a()"}]},
                "status": "completed",
            },
        }),
        json!({"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"All done.\nSecond line."}}),
        json!({
            "type": "item.completed",
            "item": {
                "id": "i3",
                "type": "mcp_tool_call",
                "tool": "solace",
                "arguments": {"command": "inspect a\nsubmit b"},
                "result": {"content": [{"type": "text", "text": concat!(
                    "==== solace inspect a ====\nfn a() {}\n==== solace submit b ====\n",
                    "E0108 unknown symbol `b`\n1 of 2 commands failed: submit b"
                )}]},
                "status": "failed",
            },
        }),
        json!({
            "type": "item.completed",
            "item": {
                "id": "i4",
                "type": "mcp_tool_call",
                "tool": "solace",
                "arguments": {"command": "view b"},
                "result": {"content": [{"type": "text", "text": "MCP error: transport closed"}]},
                "status": "failed",
            },
        }),
        json!({"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}),
    ];
    fs::write(&log, lines.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    let long = format!("{}.\nSecond line.", "x".repeat(120));
    let task = json!({
        "id": "t1",
        "column": "backlog",
        "notes": [
            {"author": "human", "time": 7, "text": "A short note."},
            {"author": "agent", "time": 8, "text": long},
        ],
        "questions": [
            {"id": "q1", "text": "Which one?", "options": ["A"], "answer": "A"},
            {"id": "q2", "text": "Open", "options": ["A"]},
        ],
        "conversation": [{"author": "human", "text": "the thread"}],
        "runs": [{
            "id": "r1", "kind": "work", "state": "ended", "outcome": "finished", "model": "fake",
            "reads": {"symbols": ["a"]}, "log": log.to_str().unwrap()
        }],
    });
    let root = root.with(&[(".solace/board/t1.json", &task.to_string())]);
    adopt_tasks(&root, &root).unwrap();
    let shown = strategy_show(&root, "task", "t1").unwrap();
    assert_eq!(shown["links"]["runs"], json!(["r1"]));
    let rec = shown["record"].clone();
    assert_eq!(
        (rec["notes"].clone(), rec["questions"].clone(), rec["runs"].clone()),
        (
            json!(["1 human 7: A short note.", format!("2 agent 8: {}…", "x".repeat(100))]),
            json!(["1 Which one?: A", "2 Open: unanswered"]),
            json!(["r1 work ended fake"])
        )
    );
    assert!(rec.get("conversation").is_none());
    let one = strategy_note_show(&root, "t1", "2").unwrap();
    assert_eq!(
        (one["task"].as_str(), one["notes"][0]["note"].as_u64(), one["notes"][0]["text"].as_str()),
        (Some("t1"), Some(2), Some(long.as_str()))
    );
    assert_eq!(strategy_note_show(&root, "t1", "all").unwrap()["notes"].as_array().unwrap().len(), 2);
    for bad in ["0", "3", "2..1", "x", "1,,2"] {
        assert!(strategy_note_show(&root, "t1", bad).unwrap_err().0.starts_with("E7001"), "{bad}");
    }
    let outline = strategy_run_show(&root, "t1", "r1", None).unwrap();
    assert_eq!(outline["run"]["id"], "r1");
    assert_eq!(outline["run"]["reads"], json!({"symbols":["a"]}));
    assert!(outline["run"].get("log").is_none());
    assert_eq!(outline["steps"][0], "1 tool: solace view a (ok)");
    assert_eq!(outline["steps"][1], "2 message: All done.");
    assert_eq!(outline["steps"][2], "3 tool: solace (1 of 2 failed) inspect a (failed)");
    assert_eq!(outline["steps"][3], "4 tool: solace view b (failed)");
    // The token-usage entry is not a step.
    assert_eq!(outline["steps"].as_array().unwrap().len(), 4);
    let picked = strategy_run_show(&root, "t1", "r1", Some("2,1")).unwrap();
    assert_eq!(picked["run"], "r1 work fake finished");
    let numbers: Vec<_> = picked["steps"].as_array().unwrap().iter().map(|s| s["step"].clone()).collect();
    assert_eq!(numbers, [json!(2), json!(1)]);
    assert_eq!(picked["steps"][1]["output"], "pub fn a()");
    assert_eq!(picked["steps"][0]["text"], "All done.\nSecond line.");
    let every = strategy_run_show(&root, "t1", "r1", Some("all")).unwrap();
    assert_eq!(every["run"], "r1 work fake finished");
    assert_eq!(every["steps"].as_array().unwrap().len(), 4);
    let failed = strategy::strategy_run_failed(&root, "t1", "r1").unwrap();
    assert_eq!(failed["run"], "r1 work fake finished");
    assert_eq!(failed["steps"], json!(["3 submit b: E0108 unknown symbol `b`", "4 view b: MCP error: transport closed"]));
    // D-105: through the command, a reply of steps stops at a whole step and names the rest.
    let one = format!("{}\n", serde_json::to_string_pretty(&strategy_run_show(&root, "t1", "r1", Some("1")).unwrap()).unwrap());
    let next = "Next: strategy show run t1 r1 2\n";
    let run = |a: &[&str], budget: usize| strategy_command(&root, "s1", a, &mut || Ok(String::new()), budget);
    assert_eq!(run(&["strategy", "show", "run", "t1", "r1", "1,2"], one.len() + next.len()).unwrap(), one.clone() + next);
    assert!(!run(&["strategy", "show", "run", "t1", "r1", "1,2"], 10_000).unwrap().contains("Next:"));
    let failed_reply = format!("{}\n", serde_json::to_string_pretty(&failed).unwrap());
    assert_eq!(run(&["strategy", "show", "run", "t1", "r1", "--failed"], 10_000).unwrap(), failed_reply);
    assert!(run(&["strategy", "show", "run", "t1", "r1", "--failed", "--from", "2"], 10_000).unwrap_err().0.starts_with("E7001"));
    assert!(run(&["strategy", "show", "run", "t1", "r1", "1", "--from", "x"], 10_000).unwrap_err().0.starts_with("E7001"));
    assert!(run(&["strategy", "show", "note", "t1", "2", "--from", "3"], 10_000).unwrap().contains("characters 3.."));
    // An id copied in its link form names the same record.
    assert_eq!(run(&["strategy", "show", "task", "task:t1"], 10_000).unwrap(), run(&["strategy", "show", "task", "t1"], 10_000).unwrap());
    assert_eq!(
        run(&["strategy", "show", "note", "task:t1", "2"], 10_000).unwrap(),
        run(&["strategy", "show", "note", "t1", "2"], 10_000).unwrap()
    );
    assert_eq!(
        run(&["strategy", "show", "run", "task:t1", "r1"], 10_000).unwrap(),
        run(&["strategy", "show", "run", "t1", "r1"], 10_000).unwrap()
    );
    for bad in ["0", "5", "2..1", "x", "1,,2"] {
        assert!(strategy_run_show(&root, "t1", "r1", Some(bad)).unwrap_err().0.starts_with("E7001"), "{bad}");
    }
    // A run whose log holds no steps says so.
    let task =
        json!({"id":"t2","column":"backlog","runs":[{"id":"r2","kind":"work","state":"running","model":"fake","log":"missing.jsonl"}]});
    fs::write(store_dir(&root).join("t2.json"), task.to_string()).unwrap();
    assert_eq!(strategy_run_show(&root, "t2", "r2", None).unwrap()["steps"], json!([]));
    assert!(strategy_run_show(&root, "t2", "r2", Some("1")).unwrap_err().0.contains("no steps"));
    let no_failed = strategy::strategy_run_failed(&root, "t2", "r2").unwrap();
    assert_eq!(no_failed["run"], "r2 work fake running");
    assert_eq!(no_failed["steps"], json!([]));
    assert!(strategy_run_show(&root, "t1", "nope", None).unwrap_err().0.starts_with("E7023"));
    assert!(strategy_run_show(&root, "nope", "r1", None).unwrap_err().0.starts_with("E7023"));
}
/// A proposal's links lose their `task:`/`feature:`/`milestone:` prefix; other fields keep theirs.
#[test]
fn a_proposal_takes_ids_in_their_link_form() {
    assert_eq!(tasks::bare_id("task:t1"), "t1");
    assert_eq!(tasks::bare_id("milestone:m1"), "m1");
    assert_eq!(tasks::bare_id("t1"), "t1");
    let fields = json!({"title":"see task:t9","feature":"feature:f1","milestone":"milestone:m1","dependsOn":["task:t1","t2"]});
    assert_eq!(tasks::bare_links(fields), json!({"title":"see task:t9","feature":"f1","milestone":"m1","dependsOn":["t1","t2"]}));
    assert_eq!(tasks::bare_links(json!({"feature":null})), json!({"feature":null}), "a cleared link stays cleared");
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
    assert_eq!(
        (a["column"].as_str(), a["attention"]["reason"].as_str(), a["attention"]["hold"].as_bool()),
        (Some("assigned"), Some("an agent asked a question"), Some(true))
    );
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
    let root = testing::Scratch::new()
        .with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#), (".solace/board/t1/r1.jsonl", "{}\n")]);
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
        {"id":"codex-gpt-6-astra","label":"GPT-6 Astra","agent":"codex","model":"gpt-6-astra","effort":"medium","efforts":["low","medium"]},
        {"id":"codex-new-1","label":"New 1","agent":"codex","model":"new-1","effort":"high","efforts":["high"]},
    ]}});
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m.iter().find(|x| x["id"] == "codex-gpt-6-astra").unwrap()["label"], "GPT-6 Astra", "discovery replaces a preset");
    assert!(m.iter().any(|x| x["id"] == "codex-new-1"), "a model the agent reports is offered");
    assert_eq!(m.iter().find(|x| x["id"] == "mine").unwrap()["label"], "Mine", "the human's config wins for the same id");
    assert_eq!(agents::merge_from(&configured, &json!({})).len(), 2, "presets stand when discovery fails");
}

/// Every native model gets a default effort: its own, else `medium`, else the
/// middle of its offer; custom runners and an empty offer get none.
#[test]
fn every_native_model_gets_a_default_effort() {
    let configured = json!([
        {"id":"claude-opus-5-5","label":"P","agent":"claude","model":"claude-opus-5-5"},
        {"id":"mine","label":"Mine","agent":"claude","model":"opus","effort":"high","configured":true},
        {"id":"odd","label":"Odd","agent":"claude","model":"x","efforts":["high","max"]},
        {"id":"bare","label":"Bare","agent":"claude","model":"y","efforts":[]},
        {"id":"runner","label":"R","agent":"claude","model":"z","run":["my-agent"]},
        {"id":"local","label":"L","agent":"claude","model":"q","endpoint":{"kind":"anthropic","base_url":"http://h"}},
        {"id":"bogus","label":"B","agent":"claude","model":"opus","effort":"bogus","configured":true},
    ]);
    let all = json!({
        "codex": {
            "time": 1,
            "models": [{
                "id": "codex-a",
                "label": "A",
                "agent": "codex",
                "model": "a",
                "effort": "low",
                "efforts": ["low", "medium"],
            }],
        },
    });
    let m = agents::merge_from(&configured, &all);
    let e = |id: &str| m.iter().find(|x| x["id"] == id).unwrap()["effort"].clone();
    assert_eq!(e("claude-opus-5-5"), "medium", "a preset takes medium");
    assert_eq!(e("mine"), "high", "the config's own effort wins");
    assert_eq!(e("odd"), "max", "no medium: the middle of the offer");
    assert_eq!(e("bare"), Value::Null, "nothing offered, no default");
    assert_eq!(e("runner"), Value::Null, "a custom runner takes none");
    assert_eq!(e("local"), "medium", "an endpoint model is native");
    assert_eq!(e("codex-a"), "low", "the agent's reported default stands");
    assert_eq!(e("bogus"), "medium", "a configured effort the model doesn't offer falls back");
}

/// A recorded `model/list` answer: hidden models are left out, and each model
/// keeps its slug (what `-m` takes), its efforts and its default effort.
#[test]
fn codex_answers_offer_their_models_and_efforts() {
    let r = json!({"data":[
        {
            "id": "gpt-6-sol",
            "model": "gpt-6-sol",
            "displayName": "GPT-6 Sol",
            "hidden": false,
            "supportedReasoningEfforts": [
                {"effort": "low", "description": "Low"},
                {"effort": "medium", "description": "Medium"},
                {"effort": "high", "description": "High"},
            ],
            "defaultReasoningEffort": "medium",
            "isDefault": true,
        },
        {
            "id": "gpt-6-sol-mini",
            "model": "gpt-6-sol-mini",
            "displayName": "GPT-6 Sol Mini",
            "hidden": true,
            "supportedReasoningEfforts": [],
            "defaultReasoningEffort": "medium",
        },
        {
            "id": "gpt-5.6-terra",
            "model": "gpt-5.6-terra",
            "displayName": "GPT-5.6 Terra",
            "supportedReasoningEfforts": [{"effort": "medium"}],
            "defaultReasoningEffort": "medium",
        },
    ]});
    let models = agents::codex_page(&r);
    assert_eq!(models.len(), 2, "a hidden model is left out");
    assert_eq!(models[0]["id"], "codex-gpt-6-sol");
    assert_eq!(models[0]["label"], "GPT-6 Sol", "a label is the model's name alone");
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
    assert_eq!(a[0]["label"], "Opus (latest)");
    assert_eq!(efforts(&a[0]), ["low", "medium", "high", "xhigh", "max"], "an alias takes the fixed Claude efforts");
}

/// With an API key, the provider's models endpoint lists exact ids: active
/// ones, with the efforts its capability flags name and the family its `line` names.
#[test]
fn the_models_endpoint_lists_active_ids() {
    let body = json!({"data":[
        {
            "id": "claude-opus-5-5",
            "display_name": "Opus 5.5",
            "lifecycle": "active",
            "line": "opus",
            "capabilities": {"effort": {"low": true, "medium": true, "high": true, "xhigh": true, "max": true}},
        },
        {
            "id": "claude-old-1",
            "display_name": "Old 1",
            "lifecycle": "deprecated",
            "capabilities": {"effort": {"low": true}},
        },
        {"id": "claude-haiku-5", "display_name": "Haiku 5", "line": "haiku", "capabilities": {}},
    ]});
    let models = agents::api_page(&body);
    assert_eq!(models.len(), 2, "a retired model is left out");
    assert_eq!(models[0]["id"], "claude-opus-5-5");
    assert_eq!(models[0]["label"], "Opus 5.5");
    assert_eq!(models[0]["efforts"], json!(["low", "medium", "high", "xhigh", "max"]));
    assert_eq!(models[1]["efforts"], json!([]), "no effort flags: no efforts to choose");
    assert_eq!(agents::family(&models[0]), Some("opus"), "the endpoint's line names the family");
    assert_eq!(agents::family(&models[1]), Some("haiku"));
}

/// A family word inside a `claude-` id names the family, a configured one's too (D-150);
/// Codex's ids and a configured model with no family word name none, so they keep their entries.
#[test]
fn a_model_names_its_family_by_id() {
    assert_eq!(agents::family(&json!({"agent":"claude","id":"claude-opus-5-5","model":"claude-opus-5-5"})), Some("opus"));
    assert_eq!(agents::family(&json!({"agent":"claude","id":"claude-haiku-4-5","model":"claude-haiku-4-5-20251001"})), Some("haiku"));
    assert_eq!(agents::family(&json!({"agent":"claude","id":"claude-opus","model":"opus"})), Some("opus"), "an alias names its family");
    assert_eq!(
        agents::family(&json!({"agent":"claude","id":"claude-opus-5-5","family":"fable"})),
        Some("fable"),
        "the endpoint's line outranks the id"
    );
    assert_eq!(agents::family(&json!({"agent":"codex","id":"codex-gpt-6-astra","model":"gpt-6-astra"})), None);
    assert_eq!(
        agents::family(&json!({"agent":"claude","id":"my-opus-local","model":"opus","configured":true})),
        None,
        "a configured model keeps its entry"
    );
    assert_eq!(
        agents::family(&json!({"agent":"claude","id":"claude-opus-local","model":"opus","configured":true})),
        Some("opus"),
        "a configured id with a family word joins the family"
    );
}

/// The pick lists one entry per Claude family: the family's live exact model,
/// its alias only when the family has none; a dropped exact model gives way to
/// the alias, so the family shows one reason. Codex's entries keep their places.
#[test]
fn the_pick_lists_one_entry_per_claude_family() {
    let models = json!([
        {"id":"codex-gpt-6-astra","label":"GPT-6 Astra","agent":"codex","model":"gpt-6-astra"},
        {"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5"},
        {"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"},
        {"id":"claude-opus-local","label":"Local Opus","agent":"claude","model":"opus","configured":true},
        {"id":"claude-fable","label":"Fable (latest)","agent":"claude","model":"fable"},
        {"id":"claude-haiku-4-5","label":"Haiku 4.5","agent":"claude","model":"claude-haiku-4-5-20251001","gone":true},
        {"id":"claude-haiku","label":"Haiku (latest)","agent":"claude","model":"haiku"},
        {"id":"mine","label":"Mine","agent":"claude","model":"local","configured":true},
    ]);
    let picked = served(models.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["codex-gpt-6-astra", "claude-opus-5-5", "claude-fable", "claude-haiku", "mine"]);
    let dropped = json!([
        {"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5","gone":true},
        {"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"},
    ]);
    let picked = served(dropped.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude-opus"], "the alias stands when the family's exact model is gone");
    let exact_only = json!([{"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5","gone":true}]);
    let picked = served(exact_only.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude-opus-5-5"], "a family with only a dropped model shows its reason");
}

/// A model a discovery dropped stays in the list, so tasks already using it
/// keep working, and is not offered for a new run.
#[test]
fn a_model_off_the_offer_stays_for_the_tasks_using_it() {
    let configured = json!([{"id":"claude-opus-5-5","label":"Preset","agent":"claude","model":"claude-opus-5-5"}]);
    let all = json!({
        "claude": {"time": 100, "models": []},
        "gone": {
            "claude-opus": {
                "id": "claude-opus",
                "label": "Claude Code · Opus (latest)",
                "agent": "claude",
                "model": "opus",
                "reason": "no longer offered by Claude Code",
            },
        },
    });
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m.len(), 2, "the model stays in the list");
    let gone = m.iter().find(|x| x["id"] == "claude-opus").unwrap();
    assert_eq!(gone["gone"], true);
    let status: agents::Status = agents::availability(gone).unwrap();
    assert_eq!(status.reason, "no longer offered by Claude Code", "it is not offered for a new run");
    assert!(!status.retry);
    assert!(!status.sign_in);
    assert_eq!(gone["label"], "Opus (latest)", "a label saved before D-150 drops the agent's name");
    let saved = json!({"codex":{"time":1,"models":[{"id":"codex-x","label":"Codex · GPT-6 X","agent":"codex","model":"x"}]}});
    assert_eq!(agents::merge_from(&json!([]), &saved)[0]["label"], "GPT-6 X");
    let mine = json!([{"id":"mine","label":"Codex · mine","agent":"codex","model":"y","configured":true}]);
    assert_eq!(agents::merge_from(&mine, &json!({}))[0]["label"], "Codex · mine", "the human's own label stays as written");
}

#[test]
fn a_model_error_names_the_offer_as_out_of_date() {
    assert!(agents::model_gone("Error: Model 'gpt-9-pro' not found."));
    assert!(agents::model_gone("unknown model claude-opus-9"));
    assert!(agents::model_gone("The model gpt-6-new does not exist"));
    assert!(!agents::model_gone("You've hit your usage limit; try again at 3pm"));
    assert!(!agents::model_gone("stream disconnected before completion"));
    assert!(!agents::model_gone("Model metadata for `commandcode-deepseek-v4.1-flash` not found. Defaulting to fallback metadata."));
}

/// The model a run used comes from its log's start event, and an alias's
/// label becomes that model's once known: its entry's label, else its id.
#[test]
fn a_run_shows_the_model_it_used() {
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    fs::write(
        &log,
        "{\"type\":\"assistant\"}\n{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\",\"model\":\"claude-opus-5-5\"}\n",
    )
    .unwrap();
    assert_eq!(used_model(&log).as_deref(), Some("claude-opus-5-5"));
    fs::write(&log, "not json\n{\"type\":\"thread.started\",\"thread_id\":\"t1\"}\n").unwrap();
    assert_eq!(used_model(&log), None, "a start event naming no model names none");
    let configured = json!([{"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"}]);
    let m = agents::merge_from(&configured, &json!({"used":{"claude-opus":"claude-opus-5-5"}}));
    assert_eq!(m[0]["label"], "claude-opus-5-5", "no entry runs it: its id");
    assert_eq!(m[0]["model"], "opus", "the alias is what runs");
    let listed = json!([configured[0], {"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5"}]);
    let m = agents::merge_from(&listed, &json!({"used":{"claude-opus":"claude-opus-5-5"}}));
    assert_eq!(m[0]["label"], "Opus 5.5", "the entry running it names it");
    let own = agents::merge_from(
        &json!([{"id":"local","label":"Local: Strata Swift","agent":"claude","model":"swift-iq3_xxs","configured":true}]),
        &json!({"used":{"local":"swift-iq2_xs"}}),
    );
    assert_eq!(own[0]["label"], "Local: Strata Swift", "a model with its own name shows it, not what an older run used");
}

#[test]
fn endpoints_are_checked_and_passed_to_codex() {
    // An endpoint's kind must match its agent.
    let wrong = concat!(
        "[[models]]\nid = \"x\"\nlabel = \"X\"\nagent = \"claude\"\nmodel = \"m\"\n",
        "endpoint = { kind = \"openai\", base_url = \"http://h\" }\n",
    );
    assert!(parse_system(wrong).unwrap_err().0.starts_with("E6020"));
    let m = json!({"label":"Local","endpoint":{"kind":"openai","base_url":"http://localhost:11434/v1","key_env":"SOLACE_TEST_UNSET_KEY"}});
    let args = endpoint_args(&m, false);
    assert_eq!(args[1], "model_provider=\"solace_endpoint\"");
    let provider: toml::Value = format!("p = {}", args[3].split_once('=').unwrap().1).parse().unwrap();
    assert_eq!(provider["p"]["base_url"].as_str(), Some("http://localhost:11434/v1"));
    assert_eq!(provider["p"]["env_key"].as_str(), Some("SOLACE_TEST_UNSET_KEY"));
    assert_eq!(provider["p"]["wire_api"].as_str(), Some("responses"));
    assert!(endpoint_args(&json!({"label":"No endpoint"}), false).is_empty());
    // A stored key reaches Codex through Solace's own variable.
    let stored = endpoint_args(&m, true);
    let provider: toml::Value = format!("p = {}", stored[3].split_once('=').unwrap().1).parse().unwrap();
    assert_eq!(provider["p"]["env_key"].as_str(), Some(setup::KEY_VAR));
    let bad = concat!(
        "[[models]]\nid = \"x\"\nlabel = \"X\"\nagent = \"codex\"\nmodel = \"m\"\n",
        "endpoint = { kind = \"openai\", base_url = \"http://h\", stored_key = \"yes\" }\n",
    );
    assert!(parse_system(bad).unwrap_err().0.starts_with("E6020"));
}

#[test]
fn models_and_keys_are_set_up_from_settings() {
    use std::os::unix::fs::PermissionsExt;
    let dir = testing::Scratch::new();
    let (config, keys) = (dir.join("config.toml"), dir.join("keys.json"));
    fs::write(&config, "task_timeout = 60\n").unwrap();
    let body = json!({
        "label": "Command Code",
        "agent": "codex",
        "model": "deepseek/deepseek-v4.1-flash",
        "base_url": "https://api.commandcode.ai/provider/v1",
        "key": "sk-1",
    });
    assert_eq!(setup::save(&config, &keys, &body).unwrap(), "command-code");
    let v = parse_system(&fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(v["task_timeout"], 60, "other settings stay");
    let m = v["models"].as_array().unwrap().iter().find(|m| m["id"] == "command-code").unwrap().clone();
    assert_eq!(m["endpoint"]["kind"], "openai");
    assert_eq!(m["endpoint"]["stored_key"], true);
    assert!(!fs::read_to_string(&config).unwrap().contains("sk-1"), "the key never goes in the settings");
    assert_eq!(setup::stored(&keys, "command-code").as_deref(), Some("sk-1"));
    assert_eq!(fs::metadata(&keys).unwrap().permissions().mode() & 0o777, 0o600);
    // Editing with a blank key keeps the stored one.
    let edit = json!({"id":"command-code","label":"CC","agent":"codex","model":"m2","base_url":"https://h/v1","key":" "});
    setup::save(&config, &keys, &edit).unwrap();
    let v = parse_system(&fs::read_to_string(&config).unwrap()).unwrap();
    let m = v["models"].as_array().unwrap().iter().find(|m| m["id"] == "command-code").unwrap().clone();
    assert_eq!((m["label"].as_str(), m["model"].as_str()), (Some("CC"), Some("m2")));
    assert_eq!(setup::stored(&keys, "command-code").as_deref(), Some("sk-1"));
    // Removing takes the entry and its key.
    setup::remove(&config, &keys, "command-code").unwrap();
    assert_eq!(setup::stored(&keys, "command-code"), None);
    // A model with its own variable keeps it and needs no stored key.
    fs::write(
        &config,
        concat!(
            "[[models]]\nid = \"v\"\nlabel = \"V\"\nagent = \"claude\"\nmodel = \"m\"\n",
            "endpoint = { kind = \"anthropic\", base_url = \"http://h\", key_env = \"V_KEY\" }\n",
        ),
    )
    .unwrap();
    setup::save(&config, &keys, &json!({"id":"v","label":"V","agent":"claude","model":"m","base_url":"http://h2"})).unwrap();
    let v = parse_system(&fs::read_to_string(&config).unwrap()).unwrap();
    let m = v["models"].as_array().unwrap().iter().find(|m| m["id"] == "v").unwrap().clone();
    assert_eq!((m["endpoint"]["key_env"].as_str(), m["endpoint"].get("stored_key")), (Some("V_KEY"), None));
    // Refused: a missing field, an unknown agent, a label with no letters.
    assert!(setup::save(&config, &keys, &json!({"label":"X","agent":"codex","model":"m"})).unwrap_err().0.contains("base_url required"));
    assert!(setup::save(&config, &keys, &json!({"label":"X","agent":"gpt","model":"m","base_url":"h"})).is_err());
    assert!(setup::save(&config, &keys, &json!({"label":"!!","agent":"codex","model":"m","base_url":"h"})).is_err());
    // An unknown id is refused.
    setup::remove(&config, &keys, "v").unwrap();
    assert!(setup::remove(&config, &keys, "v").is_err());
    fs::write(&config, "").unwrap();
    assert!(setup::remove(&config, &keys, "v").is_err());
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
        json!({
            "type": "item.started",
            "item": {
                "id": "i1",
                "type": "mcp_tool_call",
                "tool": "solace",
                "arguments": {"command": "view a"},
                "status": "in_progress",
            },
        }),
        json!({
            "type": "item.completed",
            "item": {
                "id": "i1",
                "type": "mcp_tool_call",
                "tool": "solace",
                "arguments": {"command": "view a", "text": "x"},
                "result": {"content": [{"type": "text", "text": "pub fn a()"}]},
                "status": "completed",
            },
        }),
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
    let batch = json!({
        "type": "item.completed",
        "item": {
            "id": "i3",
            "type": "mcp_tool_call",
            "tool": "solace",
            "arguments": {"command": "read a\ngrep x"},
            "result": {"content": [{"type": "text", "text": "E6002\ndocs/x\n1 of 2 commands failed: read a\n"}]},
            "status": "failed",
        },
    });
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
    assert_eq!(
        partly_failed("failed", "E\n1 of 2 commands failed: write a; skipped: run t\n").as_deref(),
        Some("(1 of 2 failed, 1 skipped)")
    );
    assert_eq!(partly_failed("failed", "E\n1 of 2 commands failed: read a\n").as_deref(), Some("(1 of 2 failed)"));
    // D-119 point 5: a command's text can hold "; ", so the count is capped at the
    // commands the reply says were left after the failure.
    assert_eq!(
        partly_failed("failed", "E\n1 of 2 commands failed: write a; skipped: read x; grep y\n").as_deref(),
        Some("(1 of 2 failed, 1 skipped)")
    );
    let text = codex.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n") + "\nnot json\nstill not\n";
    fs::write(&log, text).unwrap();
    let t = transcript(&log);
    assert_eq!(t[0]["input"], "text:\nx");
    assert_eq!(t[3]["text"], "not json\nstill not");
    assert_eq!(last_words(&t).unwrap(), "All **done**.");
    let claude = [
        json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "Looking."},
                    {"type": "tool_use", "id": "u1", "name": "mcp__solace__solace", "input": {"command": "test"}},
                ],
            },
        }),
        json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "u1",
                    "content": [{"type": "text", "text": "E1 failed"}],
                    "is_error": true,
                }],
            },
        }),
    ];
    fs::write(&log, claude.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    let t = transcript(&log);
    assert_eq!(
        (t[1]["title"].as_str(), t[1]["status"].as_str(), t[1]["output"].as_str()),
        (Some("solace test"), Some("failed"), Some("E1 failed"))
    );
    // D-109: a batch where only some commands failed says how many in its title.
    let batch = [
        json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "u2",
                    "name": "mcp__solace__solace",
                    "input": {"command": "read a\ngrep x"},
                }],
            },
        }),
        json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "u2",
                    "content": [{
                        "type": "text",
                        "text": "==== solace read a ====\nE6002\n==== solace grep x ====\ndocs/x\n1 of 2 commands failed: read a\n",
                    }],
                    "is_error": true,
                }],
            },
        }),
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
    let result = json!({
        "type": "result",
        "usage": {
            "input_tokens": 16,
            "cache_read_input_tokens": 166278,
            "cache_creation_input_tokens": 34062,
            "output_tokens": 4040,
        },
    })
    .to_string();
    fs::write(&log, [lines.join("\n"), result].join("\n")).unwrap();
    let usage: Vec<Value> = transcript(&log).into_iter().filter(|e| e["kind"] == "usage").collect();
    assert_eq!(
        usage[0]["text"],
        "200356 input tokens (166278 read from cache, 34062 written to cache), 4040 output tokens; last context 300 tokens"
    );
    assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
}

/// D-110: a brief names the task's unfinished dependencies, from any project,
/// with advice; with none, or only finished ones (canceled counts), it says nothing.
#[test]
fn briefs_name_unfinished_dependencies() {
    // `lib` is listed as a project so a dependency homed there keeps its home:
    // a home that is no project is re-homed on load (D-160).
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"lib\"]\n"),
        ("lib/.solace/keep", ""),
    ]);
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
    for part in
        ["Depends on:\n", "- a Task a (Assigned, in lib) — unfinished", "- b Task b (Complete)\n", "- c Task c (Canceled)\n", WAITING]
    {
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
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "rank": 3,
        "stamps": {},
        "links": [{"name": "spec", "url": "http://x"}],
        "notes": [{"author": "human", "text": "Keep it small", "time": 1, "run": null}],
        "questions": [{"id": "q", "text": "Syntax?", "options": ["a", "b"], "answer": "b"}],
        "runs": [{"id": "r1"}],
        "changes": [{"entity": "symbol:x", "name": "calc.add", "run": "r1"}],
        "test_commands": [{"project": "a", "commands": ["rust-acceptance", "spacing_parity"]}],
    });
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
    // Symbol-held source is named without tying it to Gleam.
    assert!(b.contains("symbol-held source") && !b.contains("with Gleam code"), "{b}");
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind);
        for guidance in [
            "Write human-facing task messages in Markdown",
            "including the text sent to task tools",
            "relevant sections with short headings",
            "blank lines around headings and lists",
            "explain what you need and why it matters",
            "Keep messages short: lead with the answer or outcome",
            "give each option a short example of its outcome",
        ] {
            assert!(prompt.contains(guidance), "{kind}: missing {guidance}");
        }
        assert_eq!(prompt.contains("**Assessment: Looks good**"), kind == "review");
        assert_eq!(prompt.contains("**Assessment: Needs work**"), kind == "review");
    }
    // D-181: source files are projections; only a module, symbol or other file is named.
    for kind in ["work", "investigate", "review", "discuss"] {
        assert!(
            brief(&t, kind).contains(concat!(
                "Code held as symbols is read and cited by name (`lockdown.dispatch`, `checkout.pay`), ",
                "not by source path: source files are projections written by Solace.",
            )),
            "{kind}: missing the projection rule"
        );
    }
    let plan = brief(&t, "investigate");
    assert!(
        plan.contains("Report the required behavior, constraints, acceptance criteria and risks")
            && !plan.contains("names every module or symbol it changes"),
        "{plan}"
    );
    // D-119 point 6: check what is already answered before asking.
    for kind in ["work", "investigate"] {
        assert!(brief(&t, kind).contains("check the Notes so far and Questions below, if any"), "{kind}");
        // D-153: each option shows an example of its outcome.
        assert!(brief(&t, kind).contains("give each option a short example of the outcome the human would see"), "{kind}");
    }
    assert!(brief(&t, "investigate").contains("Investigate this task"));
    // D-147: a report never hands a file change to the human.
    assert!(
        brief(&t, "investigate")
            .contains("The human never edits files or runs commands; any needed change, a settings change included, is the worker's")
    );
    // D-156/D-186: an investigation reports requirements, not a plan for edits.
    assert!(
        brief(&t, "investigate").contains("anything you didn't check")
            && !brief(&t, "investigate").contains("A plan is the smallest complete change")
    );
    assert!(brief(&t, "work").contains("what you skipped and risks the human must know"));
    assert!(brief(&t, "review").contains("Flag code the task didn't need, places it missed"));
    for kind in ["work", "investigate", "discuss"] {
        assert!(!brief(&t, kind).contains("smallest complete change"), "{kind}");
    }
    for kind in ["work", "discuss"] {
        assert!(!brief(&t, kind).contains("Flag code the task didn't need"), "{kind}");
    }
    // Maintainability: each role gets the direction it acts on, and both share
    // the comment and UI rules.
    let work = brief(&t, "work");
    let review = brief(&t, "review");
    for kind in ["work", "review"] {
        assert!(
            brief(&t, kind).contains(
                "Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI."
            ),
            "{kind}: missing the comment and UI rules"
        );
    }
    for rule in [
        "Reuse existing code before adding new code, extracting common patterns",
        "small, stateless, self-documenting functions",
        "small and focused around one domain or component",
        "clear names for new symbols",
        "the module that owns its concern",
        "task:2414798176c466e4",
    ] {
        assert!(work.contains(rule), "work: missing {rule}");
        assert!(!review.contains(rule), "review: unexpected {rule}");
    }
    for rule in [
        "Flag duplication and large modules",
        "minor ones as tech debt",
        "cross-domain concerns in the same module",
        "excessively long functions and dead code",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
        assert!(!work.contains(rule), "work: unexpected {rule}");
    }
    // D-182: only a work run is told to run every test, once, after its last change.
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind);
        assert_eq!(
            prompt.contains("Before `task done`, after your last change, run `test --all` in each project with symbol-held source"),
            kind == "work",
            "{kind}"
        );
        assert_eq!(prompt.contains("and `run <name>` for: rust-acceptance, spacing_parity."), kind == "work", "{kind}");
    }
    assert!(!b.contains("the test commands its `solace.toml` declares"), "{b}");
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
        r.contains(
            "For each concern, inspect every symbol it needs in one call (`inspect a b c --context`); split only when the reply is trimmed"
        ) && r
            .contains("A call is a turn, so batch the symbols one concern needs; start from the changed symbols the review context names")
            && !r.contains("small batches"),
        "{r}"
    );
    assert!(
        r.contains("Only a `task note` call records the feedback")
            && r.contains("don't claim it was recorded unless the reply says `Recorded.`"),
        "{r}"
    );
    for kind in ["work", "investigate", "discuss"] {
        assert!(!brief(&t, kind).contains("Only a `task note` call records the feedback"), "{kind}");
    }
    assert!(
        r.contains("Task changes:\n- calc.add")
            && r.contains("`task note`, the feedback in `text`")
            && r.contains("`format` are refused, and `run <name>` works only for commands marked `allowed_in_reviews`")
            && !r.contains("task done")
    );
    assert!(!b.contains("Projects:"), "one project needs no Projects section");
    assert!(review.contains(FINDINGS_ASK), "a review is asked for detailed findings");
    for rule in [
        "the heading is exactly `### Tech debt`",
        "each item is one `- ` bullet at the start of a line",
        "its details are lines indented under it",
        "no line after the heading starts with `#`",
        "no other heading is inside the section, and nothing follows it except a new heading",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
    }
    assert!(review.contains(TECH_DEBT_ASK), "a review is asked for tech debt");
    for kind in ["work", "investigate", "discuss"] {
        let prompt = brief(&t, kind);
        assert!(!prompt.contains(FINDINGS_ASK), "{kind}: unexpected findings detail");
        assert!(!prompt.contains(TECH_DEBT_ASK), "{kind}: unexpected tech debt ask");
    }
    // D-198: a review of a task with a feature is asked the same.
    let mut featured = t.clone();
    featured["feature"] = json!("f1");
    assert!(brief(&featured, "review").contains(TECH_DEBT_ASK), "a featured review is asked for tech debt");
    // A task across projects (D-61) says where the agent works and how it reaches the rest.
    let mut wide = t.clone();
    wide["scope"] = json!(["a", "web"]);
    wide["home"] = json!("a");
    wide["changes"][0]["project"] = json!("web");
    wide["test_commands"] = json!([
        {"project":"a","commands":["rust-acceptance"]},
        {"project":"web","commands":["spacing_parity"]}
    ]);
    let w = brief(&wide, "work");
    assert!(w.contains("covers the projects a, web. You work in a; for another of them, add `--project <folder>`"), "{w}");
    assert!(w.contains("Task changes:\n- web: calc.add"), "{w}");
    assert!(w.contains("and `run <name>` for: a: rust-acceptance; web: spacing_parity."), "{w}");
}

#[test]
fn unavailable_models_say_why() {
    let missing = json!({"agent":"codex","run":["solace-no-such-program"]});
    assert_eq!(unavailable(&missing).unwrap(), "solace-no-such-program is not installed. Install it.");
    assert_eq!(agents::unavailable(&missing), unavailable(&missing));
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
    let (path, log) = (dir.join("agent-holds.json"), dir.join("log.jsonl"));
    let codex = json!({"agent":"codex","model":"m"});
    let said = fixture["codex"][3]["said"].as_str().unwrap();
    fs::write(&log, json!({"type":"error","message":said}).to_string()).unwrap();
    agents::observe(&path, &codex, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    let reason = "Codex usage limit reached (resets 3:45 PM)";
    assert_eq!(agents::held(&path, "codex", NOON).as_deref(), Some(reason));
    assert!(!agents::sign_in_mark(&path, "codex", NOON));
    assert_eq!(agents::held(&path, "codex", 1_791_992_699).as_deref(), Some(reason));
    // It lifts at the reset time, and holds only Codex's sign-in.
    assert_eq!(agents::held(&path, "codex", 1_791_992_700), None);
    assert_eq!(agents::held(&path, "claude", NOON), None);
    // A model on its own endpoint, or with its own program, isn't on the sign-in.
    let other = dir.join("other.json");
    for m in [json!({"agent":"codex","model":"m","endpoint":{"base_url":"http://x"}}), json!({"agent":"codex","run":["sh"]})] {
        agents::observe(&other, &m, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    }
    assert!(!other.exists());
    // Claude Code's limit, in its stream-json result, with a reset it can't read: held until cleared.
    let claude = json!({"agent":"claude","model":"m"});
    fs::write(&log, json!({"type":"result","is_error":true,"result":"You've hit your limit · resets soon","usage":{}}).to_string())
        .unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON + 365 * 86400).as_deref(), Some("Claude Code usage limit reached (resets soon)"));
    // The mark is on disk, not only in memory.
    agents::forget_marks();
    assert!(agents::held(&path, "claude", NOON).is_some());
    // A run that got no answer leaves it.
    fs::write(&log, "").unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert!(agents::held(&path, "claude", NOON).is_some());
    // A run whose agent answered clears it: one that finished, started after
    // the mark was set.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON), None);
    // A stopped run leaves the mark, even when its agent answered: it says
    // nothing about the sign-in.
    fs::write(&log, json!({"type":"result","is_error":true,"result":"You've hit your limit · resets soon","usage":{}}).to_string())
        .unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("stopped"), &zones).unwrap();
    assert!(agents::held(&path, "claude", NOON).is_some());
    // A finished run started before the mark leaves it too: its answers came
    // before the limit was hit.
    agents::observe(&path, &claude, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    assert!(agents::held(&path, "claude", NOON).is_some());
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON), None);
    // A run that succeeded clears the mark whatever its successful outcome:
    // reviewed, discussed and planned count as `finished` does, the list
    // `supervise` ends a run with. A mark set after the run started stays.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    for o in ["reviewed", "discussed", "planned"] {
        agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some(o), &zones).unwrap();
        assert_eq!(agents::held(&path, "claude", NOON), None, "{o}");
        agents::mark(&path, "claude", "Claude Code usage limit reached", Some(NOON + 3600)).unwrap();
        agents::observe(&path, &claude, &log, NOON, NOON, Some(o), &zones).unwrap();
        assert!(agents::held(&path, "claude", NOON).is_some(), "{o}");
    }
    // Claude Code reports its limit as a message, not an error: a message that
    // is exactly the limit's own text marks the sign-in, and is never counted
    // as "the agent answered".
    fs::write(
        &log,
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your limit · resets 2pm (UTC)"}]}}).to_string(),
    )
    .unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON).as_deref(), Some("Claude Code usage limit reached (resets 2pm (UTC))"));
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert!(agents::held(&path, "claude", NOON).is_some());
    // A passing mention inside a longer message marks nothing, and is an answer.
    fs::write(
        &log,
        json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "text",
                    "text": "I noted: You've hit your limit · resets 2pm (UTC)\nThen I carried on.",
                }],
            },
        })
        .to_string(),
    )
    .unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON), None);
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
    let folder = |k: &agents::Kind| home.join(if k.name == "codex" { ".codex" } else { ".claude" });
    let unknown = |_: &agents::Kind| None;
    let check = |m: &Value, var: &dyn Fn(&str) -> bool| {
        agents::local(m, &agents::Here { folder: &folder, status: &unknown, var, stored: &|_| false, installed: &|_| true, now })
    };
    let (codex, claude) = (json!({"agent":"codex","model":"m"}), json!({"agent":"claude","model":"m"}));
    let missing = agents::Here { folder: &folder, status: &unknown, var: &none, stored: &none, installed: &|_| false, now };
    assert_eq!(agents::local(&codex, &missing).unwrap(), "Codex is not installed. Install it.");
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
    assert_eq!(check(&endpoint, &none).unwrap(), "No key yet; add one in Settings or set the variable LOCAL_KEY");
    assert!(check(&endpoint, &|v| v == "LOCAL_KEY").is_none());
    // A key stored from Settings is enough; an endpoint naming no key needs none.
    let settings = json!({"id":"cc","agent":"codex","model":"m","endpoint":{"base_url":"http://x","stored_key":true}});
    assert_eq!(check(&settings, &none).unwrap(), "No key yet; add one in Settings");
    let stored = agents::Here { folder: &folder, status: &unknown, var: &none, stored: &|id| id == "cc", installed: &|_| true, now };
    assert!(agents::local(&settings, &stored).is_none());
    assert!(
        agents::local(&endpoint.clone(), &agents::Here { stored: &|_| true, ..stored }).is_some(),
        "a key for another id is not this one's"
    );
    assert!(check(&json!({"agent":"codex","model":"m","endpoint":{"base_url":"http://x"}}), &none).is_none());
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
    assert_eq!(
        run_project(&json!({"project": "a"}), true, &a, &t("Change `plan.answer`.")),
        "a",
        "a resumed run keeps its recorded project"
    );
    assert_eq!(
        run_project(&json!({}), true, &a, &t("Change `plan.answer`.")),
        "a",
        "a conversation saved before projects were recorded restarts at the home"
    );
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

/// The check after an undo follows the restored files: a project the task left as plain files
/// (its `gleam.toml` deleted) is Gleam again once restored, so its restored code is checked.
#[test]
fn reverting_checks_a_project_the_restore_makes_gleam_again() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Manifest"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let id: String = s.db.query_row("SELECT id FROM symbols WHERE name='main'", [], |r| r.get(0)).unwrap();
    let mut before = snapshot(&s).unwrap();
    before[format!("symbol:{id}")][4] = json!("pub fn main() -> Int { \"one\" }");
    before["file:gleam.toml"] = json!(fs::read(s.root.join("gleam.toml")).unwrap());
    fs::remove_file(s.root.join("gleam.toml")).unwrap();
    let mut after = snapshot(&s).unwrap();
    after["file:gleam.toml"] = Value::Null;
    record(&s, &tid, "r1", &before, &after, None).unwrap();
    let e = api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap_err().0;
    assert!(e.contains("E7008") && e.contains("errors"), "{e}");
}

/// An undo checks a restored project when its tool is there: Gleam always,
/// Rust only with cargo, and a project of plain files never (D-158).
#[test]
fn an_undo_checks_a_restored_project_only_when_its_tool_is_there() {
    let gleam = testing::Scratch::new().with(&[("gleam.toml", "name=\"a\"\n")]);
    let rust = testing::Scratch::new().with(&[("Cargo.toml", "[package]\nname=\"a\"\nversion=\"0.1.0\"\n")]);
    let plain = testing::Scratch::new().with(&[("notes.md", "x\n")]);
    assert!(checkable(&gleam, false));
    assert!(checkable(&rust, true));
    assert!(!checkable(&rust, false), "no cargo: the undo is unchecked, not refused");
    assert!(!checkable(&plain, true));
}

/// D-167: an undo in a Rust project is checked too. The restored code has a
/// type error, so the cancel fails and the project is put back.
#[test]
fn reverting_checks_a_rust_projects_restored_code() {
    if !crate::language::cargo::installed() {
        return;
    }
    let p = testing::Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    crate::import(&p).unwrap();
    let s = open(&p).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Rust"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let id: String = s.db.query_row("SELECT id FROM symbols WHERE name='one'", [], |r| r.get(0)).unwrap();
    let mut before = snapshot(&s).unwrap();
    before[format!("symbol:{id}")][4] = json!("pub fn one() -> i32 { \"two\" }");
    let after = snapshot(&s).unwrap();
    record(&s, &tid, "r1", &before, &after, None).unwrap();
    let e = api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap_err().0;
    assert!(e.contains("E7008") && e.contains("errors"), "{e}");
}

/// Cancelling a task that made a Rust module removes its file and the `mod x;`
/// line its parent's header took (D-136, D-168).
#[test]
fn reverting_a_rust_module_removes_its_file_and_mod_line() {
    if !crate::language::cargo::installed() {
        return;
    }
    let p = testing::Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    crate::import(&p).unwrap();
    let s = open(&p).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Rust module"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let found = existing_folders(&s.root);
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("x"), "pub fn two() -> i32 { 2 }\n").unwrap();
    let after = snapshot(&s).unwrap();
    record(&s, &tid, "r1", &before, &after, Some(&found)).unwrap();
    assert!(s.root.join("src/x.rs").is_file());
    assert!(fs::read_to_string(s.root.join("src/lib.rs")).unwrap().contains("mod x;"));
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert!(!s.root.join("src/x.rs").exists(), "the file went with the module");
    assert_eq!(fs::read_to_string(s.root.join("src/lib.rs")).unwrap(), "pub fn one() -> i32 { 1 }\n");
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
    s.db.execute("INSERT INTO modules(path,dir,header) SELECT 'ui/deep/new',dir,header FROM modules WHERE path='app'", []).unwrap();
    s.db.execute(
        concat!(
            "INSERT INTO symbols(id,module,name,kind,text,ord,revision,live) ",
            "SELECT 'new1','ui/deep/new',name,kind,text,ord,revision,live FROM symbols WHERE module='app'",
        ),
        [],
    )
    .unwrap();
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

/// A root that is a project listing `sub`, with task t1 at home in `web`, no
/// project: its folder is in the root's board, where its runs wrote.
fn homeless_root() -> testing::Scratch {
    testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"sub\"]\n"),
        (".solace/store.db", ""),
        (".solace/tasks/t1.json", r#"{"id":"t1","column":"backlog","home":"web","scope":["sub","web"],"runs":[],"notes":[]}"#),
        (".solace/board/t1/log", ""),
        ("sub/.solace/keep", ""),
    ])
}

/// D-160: a task whose home is no project is re-homed on load to the first
/// project of its scope, keeping only the projects of it, with a note naming the
/// move and the dropped folder, and its folder moves as a scope change moves it.
#[test]
fn a_homeless_task_is_rehomed_to_the_first_project_of_its_scope() {
    let root = homeless_root();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!((t["home"].as_str(), t["scope"].clone(), t["noProject"].is_null()), (Some("sub"), json!(["sub"]), true), "{t}");
    let saved = fs::read_to_string(store_dir(&root).join("t1.json")).unwrap();
    assert!(
        saved.contains(r#""home": "sub""#)
            && saved.contains("Moved from `web` to `sub`")
            && saved.contains("Dropped `web` from its projects"),
        "{saved}"
    );
    assert!(root.join("sub/.solace/board/t1").is_dir(), "its folder moved to the new home");
    assert!(!root.join(".solace/board/t1").exists(), "and left the root");
    assert_eq!(task(&root.join("sub"), "t1").unwrap()["home"], "sub", "it opens from its new home");
}

/// D-160: a task with a run still live is not re-homed: the agent works from the
/// task's folder, so the load leaves it marked until the run ends.
#[test]
fn a_homeless_task_with_a_live_run_is_not_rehomed() {
    let root = homeless_root();
    fs::write(
        store_dir(&root).join("t1.json"),
        concat!(
            r#"{"id":"t1","column":"assigned","home":"web","scope":["sub","web"],"#,
            r#""runs":[{"id":"r1","kind":"work","state":"running"}],"notes":[]}"#,
        ),
    )
    .unwrap();
    let mut t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!((t["home"].as_str(), t["noProject"].as_bool()), (Some("web"), Some(true)), "{t}");
    assert!(root.join(".solace/board/t1").is_dir(), "its folder stays where the run works");
    t["runs"][0]["state"] = json!("ended");
    fs::write(store_dir(&root).join("t1.json"), t.to_string()).unwrap();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!(t["home"].as_str(), Some("sub"), "and it re-homes once the run ends");
}

/// D-160: a task with no project in its scope goes to the root, and its scope
/// becomes the root alone.
#[test]
fn a_task_with_no_project_in_its_scope_goes_to_the_root() {
    let root = homeless_root();
    fs::write(store_dir(&root).join("t1.json"), r#"{"id":"t1","column":"backlog","home":"web","scope":["web"],"runs":[],"notes":[]}"#)
        .unwrap();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!((t["home"].as_str(), t["scope"].clone()), (Some("."), json!(["."])), "{t}");
    let saved = fs::read_to_string(store_dir(&root).join("t1.json")).unwrap();
    assert!(saved.contains("Moved from `web` to `.`"), "{saved}");
}

/// D-160: a task keeps no project only when the root is no project.
#[test]
fn a_task_stays_without_a_project_when_the_root_is_none() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"sub\"]\n"),
        (".solace/tasks/t1.json", r#"{"id":"t1","column":"backlog","home":"web","scope":["web"],"runs":[],"notes":[]}"#),
    ]);
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!((t["home"].as_str(), t["noProject"].as_bool()), (Some("web"), Some(true)), "{t}");
    assert!(task(&root, "t1").unwrap()["noProject"] == true, "it opens from the root");
}

/// D-160: the mark comes from the home on each load, so a `noProject` saved by
/// an older build is dropped: a task at the root lists and opens unmarked.
#[test]
fn a_saved_no_project_mark_is_dropped_on_load() {
    let root = homeless_root();
    fs::write(
        store_dir(&root).join("t1.json"),
        r#"{"id":"t1","column":"backlog","home":".","scope":["."],"noProject":true,"runs":[],"notes":[]}"#,
    )
    .unwrap();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert!(t["noProject"].is_null(), "listed unmarked: {t}");
    assert!(task(&root, "t1").unwrap()["noProject"].is_null(), "opens unmarked");
}

/// A project list that can't be read counts every home as a project: no task is
/// marked and none is re-homed while the list is unreadable.
#[test]
fn an_unreadable_project_list_marks_no_task() {
    let root = homeless_root();
    fs::write(root.join("solace.toml"), "projects = 5\n").unwrap();
    // A task that needs no workflow reads whole, so the listing reaches it.
    fs::write(
        store_dir(&root).join("t1.json"),
        r#"{"id":"t1","column":"backlog","home":"web","scope":["sub","web"],"closedTime":null,"runs":[],"notes":[]}"#,
    )
    .unwrap();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert!(t["noProject"].is_null(), "{t}");
    assert_eq!(t["home"], "web", "and stays where it is");
}

/// The cause of task:59d0b7ae77d161d9: a change on a task with no project saved
/// the mark into its file, where it stayed. The load re-homes the task and the
/// change saves no mark (D-160).
#[test]
fn a_change_on_a_homeless_task_saves_no_mark() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", "projects = [\"sub\"]\n")]);
    let root = p.root.clone();
    fs::create_dir_all(root.join("sub/.solace")).unwrap();
    let mut t = new_task("Orphan", "", &json!("backlog"), 1.);
    t["home"] = json!("web");
    t["scope"] = json!(["web"]);
    tasks::put(&p.store, &t).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let path = store_dir(&root).join(format!("{tid}.json"));
    let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["noProject"] = json!(true);
    fs::write(&path, v.to_string()).unwrap();
    let t = api(&root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "plan"})).unwrap();
    assert_eq!(t["home"], ".", "the load re-homes it to the root");
    let saved = fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("noProject"), "{saved}");
    assert!(saved.contains("Moved from `web` to `.`"), "{saved}");
}

/// D-176: a successful approved fetch clears the reopen failure the lane shows.
#[test]
fn an_approved_fetch_clears_the_reopen_failure() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[("Cargo.toml", testing::CARGO), ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n")]);
    fs::create_dir_all(root.join(".solace")).unwrap();
    fs::write(root.join(".solace/fetch-failed"), "cargo fetch failed: stale").unwrap();
    let base = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let proposed = format!("{base}\n[features]\nextra = []\n");
    let mut q = json!({"settings":{"file":"Cargo.toml","base":base,"proposed":proposed}});
    apply_settings(&root, &mut q).unwrap();
    assert_eq!(q["answer"], "Approved and written", "{q}");
    assert!(!root.join(".solace/fetch-failed").exists(), "the stale failure is cleared");
}

/// D-179: settling checkpoints the undo-row delete too, so a commit's copy of
/// `store.db` sees the finished task.
#[test]
fn settling_puts_the_whole_store_in_store_db() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Settle"})).unwrap();
    let tid = t["id"].as_str().unwrap();
    s.db.query_row("PRAGMA wal_autocheckpoint=0", [], |_| Ok(())).unwrap();
    s.db.execute("INSERT INTO board_before VALUES (?1,'symbol:x','null')", [tid]).unwrap();
    let wal = s.root.join(".solace/store.db-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0, "the change is not in the WAL");
    settle(&s, &t).unwrap();
    let copy = s.root.join(".solace/copied.db");
    std::fs::copy(s.root.join(".solace/store.db"), &copy).unwrap();
    let c = rusqlite::Connection::open(&copy).unwrap();
    let rows: i64 = c.query_row("SELECT COUNT(*) FROM board_before WHERE task=?1", [tid], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0, "the deleted undo row is still only in the WAL");
}

/// A whole-module format is one task change, named by the item it reshaped
/// (D-180).
#[test]
fn a_whole_module_format_is_recorded_on_the_task() {
    if !crate::language::cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n    2\n}\n}\n";
    let (root, s) = testing::rust_project(lib);
    let mut t = api(&root, "POST", "/tasks", json!({"title": "Format"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"format","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let out = crate::with_root(&root, || {
        with_context(&tid, "format", || {
            crate::run_options(&["format".to_string(), "crate".to_string()], &mut || Ok(String::new()), None, 1000, &[])
        })
    })
    .unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    let changes = changes_of(&s, &task(&s.root, &tid).unwrap()).unwrap();
    assert!(changes.iter().any(|c| c["name"] == "crate.Store::open" && c["run"] == "format"), "{changes:?}");
    // The format was recorded with its undo: cancelling puts the bytes back.
    let mut t = task(&s.root, &tid).unwrap();
    end_runs(&mut t, "finished");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action": "cancel"})).unwrap();
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
}

/// D-182: a work brief names the commands each project marks `test = true`,
/// sorted; a project with none adds no `run` clause.
#[test]
fn briefs_name_the_test_commands_each_project_marks() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        (
            "solace.toml",
            concat!(
                "[commands]\nunit = { run = [\"gleam\", \"test\"] }\n",
                "spacing_parity = { run = [\"true\"], test = true }\n",
                "rust-acceptance = { run = [\"true\"], test = true }\n",
                "review = { run = [\"true\"], allowed_in_reviews = true }\n",
            ),
        ),
    ]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let run = json!({"id":"r1","kind":"work"});
    let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false).0;
    assert!(brief.contains("and `run <name>` for: rust-acceptance, spacing_parity."), "{brief}");
    assert!(!brief.contains("for: unit") && !brief.contains("for: rust-acceptance, spacing_parity, review"), "{brief}");
    // A project with no marked command gets only `test --all`.
    let q = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let u = open(&q.root).unwrap();
    let t = api(&u.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let brief = continuation_brief(&t, &run, false, &q.root, &q.root, false).0;
    assert!(brief.contains("run `test --all` in each project with symbol-held source."), "{brief}");
    assert!(!brief.contains("`run <name>` for:"), "{brief}");
    // Two projects in scope, one marked: name the project so `run` lands there.
    let w = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"web\"]\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() { 1 }\n"),
        ("web/solace.toml", "[commands]\nspacing_parity = { run = [\"true\"], test = true }\n"),
    ]);
    let s = open(&w.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["scope"] = json!([".", "web"]);
    t["home"] = json!(".");
    let brief = continuation_brief(&t, &run, false, &w.root, &w.root, false).0;
    assert!(brief.contains("and `run <name>` for: web: spacing_parity."), "{brief}");
    // A malformed settings file is surfaced, not treated as no marked commands.
    let bad = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&bad.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    fs::write(bad.root.join("solace.toml"), "[commands]\nspacing_parity = { run = [\"true\"], test = \"yes\" }\n").unwrap();
    let brief = continuation_brief(&t, &run, false, &bad.root, &bad.root, false).0;
    assert!(brief.contains("the marked test commands could not be read: project .: E6020"), "{brief}");
    // A bad project is named without hiding another project's marks in either order.
    let mixed = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"web\"]\n[commands]\nrust-acceptance = { run = [\"true\"], test = true }\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() { 1 }\n"),
        ("web/solace.toml", "[commands]\nspacing_parity = { run = [\"true\"], test = \"yes\" }\n"),
    ]);
    let s = open(&mixed.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["home"] = json!(".");
    for scope in [json!([".", "web"]), json!(["web", "."])] {
        t["scope"] = scope;
        let brief = continuation_brief(&t, &run, false, &mixed.root, &mixed.root, false).0;
        assert!(brief.contains("and `run <name>` for: .: rust-acceptance;"), "{brief}");
        assert!(brief.contains("the marked test commands could not be read for: project web: E6020"), "{brief}");
    }
}

/// A changed container is covered when a test reaches one of its items.
#[test]
fn a_container_reached_through_its_items_is_tested() {
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, s) = testing::rust_project(lib);
    let mut t = api(&root, "POST", "/tasks", json!({"title": "Coverage"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let before = snapshot(&s).unwrap();
    let container: String = s.index().unwrap().syms.into_iter().find(|x| x.name == "Store" && x.kind_text == "impl").map(|x| x.id).unwrap();
    s.db.execute("UPDATE symbols SET text = ?1 WHERE id = ?2", params!["impl Store {\n    // changed\n", container]).unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let ix = s.index().unwrap();
    let reached: std::collections::HashSet<usize> =
        ix.syms.iter().enumerate().filter(|(_, x)| x.name == "Store::open").map(|(i, _)| i).collect();
    assert_eq!(with_context(&tid, "r1", || s.task_untested(&ix, &reached)), None);
}

/// A busy first project does not leave a later project's writes in its WAL.
#[test]
fn settling_flushes_every_project_after_a_busy_one() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"busy\", \"later\"]\n"),
        ("busy/gleam.toml", "name = \"busy\"\nversion = \"1.0.0\"\n"),
        ("busy/src/app.gleam", "pub fn one() {\n  1\n}\n"),
        ("later/gleam.toml", "name = \"later\"\nversion = \"1.0.0\"\n"),
        ("later/src/app.gleam", "pub fn one() {\n  1\n}\n"),
    ]);
    for d in ["busy", "later"] {
        crate::import(&root.join(d)).unwrap();
    }
    let (busy, later) = (open(&root.join("busy")).unwrap(), open(&root.join("later")).unwrap());
    let mut t = api(&busy.root, "POST", "/tasks", json!({"title": "Both"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["scope"] = json!(["busy", "later"]);
    for st in [&busy, &later] {
        st.db.query_row("PRAGMA wal_autocheckpoint=0", [], |_| Ok(())).unwrap();
        st.db.execute("INSERT INTO board_before VALUES (?1,'symbol:x','null')", [&tid]).unwrap();
        // A row for another task survives settling, so a copy of the main file
        // shows whether this project's WAL was flushed.
        st.db.execute("INSERT INTO board_before VALUES ('other','symbol:y','null')", []).unwrap();
        let wal = st.root.join(".solace/store.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0, "the change is not in the WAL");
    }
    let reader = rusqlite::Connection::open(busy.root.join(".solace/store.db")).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let rows: i64 = reader.query_row("SELECT COUNT(*) FROM board_before", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 2);
    let e = settle(&busy, &t).unwrap_err();
    assert_eq!(e.0.matches("E7028").count(), 1, "{}", e.0);
    assert!(e.0.contains("pending in busy"), "{}", e.0);
    assert!(!e.0.contains("later"), "{}", e.0);
    reader.execute_batch("ROLLBACK").unwrap();
    let copy = later.root.join(".solace/copied.db");
    std::fs::copy(later.root.join(".solace/store.db"), &copy).unwrap();
    let c = rusqlite::Connection::open(&copy).unwrap();
    let kept: i64 = c.query_row("SELECT COUNT(*) FROM board_before WHERE task='other'", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 1, "later's WAL was not flushed");
    let undone: i64 = c.query_row("SELECT COUNT(*) FROM board_before WHERE task=?1", [&tid], |r| r.get(0)).unwrap();
    assert_eq!(undone, 0, "later's undo row was not dropped");
}

/// A busy checkpoint is named once, not once per wrapping layer.
#[test]
fn settling_reports_a_busy_flush_once() {
    assert!(super::settle_result(vec![], vec![]).is_ok());
    let e = super::settle_result(vec!["a".into(), "b".into()], vec![]).unwrap_err();
    assert_eq!(e.0.matches("E7028").count(), 1, "{}", e.0);
    assert!(e.0.contains("pending in a, b"), "{}", e.0);
}

/// A checkpoint error that is not a busy log keeps its own message and code.
#[test]
fn settling_keeps_a_non_busy_checkpoint_error() {
    let e = super::settle_result(vec![], vec![("b".into(), Failure("E1234 disk full".into(), 7))]).unwrap_err();
    assert_eq!(e.0, "b: E1234 disk full");
    assert!(!e.0.contains("E7028"), "{}", e.0);
    assert_eq!(e.1, 7);
    let e = super::settle_result(vec!["a".into()], vec![("b".into(), Failure("E1234 disk full".into(), 7))]).unwrap_err();
    assert!(e.0.starts_with("b: E1234 disk full"), "{}", e.0);
    assert!(e.0.contains("Also, flushing store.db is pending in a"), "{}", e.0);
    assert!(!e.0.contains("E7028"), "{}", e.0);
    assert_eq!(e.1, 7);
}

#[test]
fn languages_are_on_unless_turned_off() {
    let v = parse_system("").unwrap();
    assert_eq!(v["languages"], json!({"gleam": true, "rust": true}));
    let v = parse_system("[languages]\ngleam = false\n").unwrap();
    assert_eq!(v["languages"], json!({"gleam": false, "rust": true}));
    assert!(parse_system("[languages]\ngleam = 1\n").is_err());
    assert!(parse_system("[languages]\npython = false\n").is_err());
}

#[test]
fn turning_a_language_off_is_saved_with_the_rest() {
    let root = testing::Scratch::new().with(&[("system.toml", "task_timeout = 42\n")]);
    let config = root.join("system.toml");
    setup::save_language(&config, &json!({"id": "gleam", "enabled": false})).unwrap();
    setup::save_language(&config, &json!({"id": "rust", "enabled": false})).unwrap();
    let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(v["languages"]["gleam"].as_bool(), Some(false));
    assert_eq!(v["languages"]["rust"].as_bool(), Some(false));
    assert_eq!(v["task_timeout"].as_integer(), Some(42));
    setup::save_language(&config, &json!({"id": "gleam", "enabled": true})).unwrap();
    let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(v["languages"]["gleam"].as_bool(), Some(true));
    assert_eq!(v["languages"]["rust"].as_bool(), Some(false));
    assert_eq!(v["task_timeout"].as_integer(), Some(42));
    assert!(setup::save_language(&config, &json!({"id": "python", "enabled": false})).is_err());
}

#[test]
fn malformed_language_settings_are_an_error_not_off() {
    let root = testing::Scratch::new().with(&[("system.toml", "[languages]\ngleam = 1\n")]);
    assert!(super::language_enabled_at(&root.join("system.toml"), "gleam").is_err());
    let root = testing::Scratch::new().with(&[("system.toml", "[languages]\ngleam = false\n")]);
    assert!(!super::language_enabled_at(&root.join("system.toml"), "gleam").unwrap());
}

/// When entries share the model string a run used, the later current entry
/// names the alias; the alias still runs its own model.
#[test]
fn a_later_entry_names_the_shared_model() {
    let configured = json!([{"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"}]);
    let all = json!({
        "codex":{"models":[
            {"id":"first","label":"First","agent":"codex","model":"claude-opus-5-5"},
            {"id":"second","label":"Second","agent":"codex","model":"claude-opus-5-5"}
        ]},
        "used":{"claude-opus":"claude-opus-5-5"}
    });
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m[0]["label"], "Second");
    assert_eq!(m[0]["model"], "opus");
}

/// Maintainability direction is in the shared reference, not only the briefs.
#[test]
fn the_reference_guides_maintainable_code() {
    let step = REFERENCE
        .split_once("2. Change and test in one call")
        .and_then(|(_, rest)| rest.split_once("\n3. Fix only what the reply reports"))
        .map(|(step, _)| step)
        .expect("workflow step 2");
    let review = REFERENCE
        .split_once("\nReviewing\n")
        .and_then(|(_, rest)| rest.split_once("\n\nCommands in full"))
        .map(|(review, _)| review)
        .expect("review checklist");
    for rule in [
        "Reuse existing code before adding new code, extracting common patterns",
        "small, stateless, self-documenting functions",
        "modules and functions small and focused around one domain or component",
        "clear names for new symbols",
        "the module that owns its concern",
        "task:2414798176c466e4",
    ] {
        assert!(step.contains(rule), "workflow step 2: missing {rule}");
    }
    for rule in [
        "thoughts, decisions and directives out of code comments and the UI",
        "details or decisions of implementation or development out of the UI",
    ] {
        assert!(step.contains(rule), "workflow step 2: missing {rule}");
    }
    for rule in [
        "Flag duplication and large modules",
        "minor ones as tech debt",
        "cross-domain concerns in the same module",
        "excessively long functions and dead code",
        "thoughts, decisions and directives out of code comments and the UI",
        "details or decisions of implementation or development out of the UI",
    ] {
        assert!(review.contains(rule), "review checklist: missing {rule}");
    }
}

/// A malformed settings file is reported, not read as a missing toolchain.
#[test]
fn reverting_reports_invalid_settings() {
    let (_p, s) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let t = api(&s.root, "POST", "/tasks", json!({"title": "Rust"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let id: String = s.db.query_row("SELECT id FROM symbols WHERE name='one'", [], |r| r.get(0)).unwrap();
    let mut before = snapshot(&s).unwrap();
    before[format!("symbol:{id}")][4] = json!("pub fn one() -> i32 { \"two\" }");
    let after = snapshot(&s).unwrap();
    record(&s, &tid, "r1", &before, &after, None).unwrap();
    crate::extract::ahead::begin(&tid, "x", &[&s.root]).unwrap();
    let pending = s.root.join(".solace/pending");
    fs::create_dir_all(&pending).unwrap();
    fs::write(pending.join("undo.json"), json!({"path":"priv/note","bytes":b"changed".to_vec()}).to_string()).unwrap();
    fs::create_dir_all(s.root.join("priv")).unwrap();
    fs::write(s.root.join("priv/note"), "original").unwrap();
    let bad = testing::Scratch::new().with(&[("system.toml", "[languages]\nrust = 1\n")]);
    let mut t = task(&s.root, &tid).unwrap();
    let e = super::with_config(&bad.join("system.toml"), || revert(&s, &mut t)).unwrap_err();
    assert!(e.0.contains("E6020"), "{}", e.0);
    assert!(pending.join("extract-x.json").exists(), "recovery ran before the settings check");
    assert!(pending.join("undo.json").exists(), "recovery ran before the settings check");
    assert_eq!(fs::read_to_string(s.root.join("priv/note")).unwrap(), "original");
}

/// A model save and a language save share one lock, so neither update is lost.
#[test]
fn parallel_model_and_language_writes_keep_both() {
    let root = testing::Scratch::new().with(&[("system.toml", "task_timeout = 42\n")]);
    let (config, keys) = (root.join("system.toml"), root.join("keys.json"));
    let held = setup::settings_lock(&config).unwrap();
    let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
    *setup::LOCK_ATTEMPT.lock().unwrap() = Some(attempt_tx);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let language = {
        let (config, done) = (config.clone(), done_tx.clone());
        std::thread::spawn(move || {
            done.send(setup::save_language(&config, &json!({"id": "rust", "enabled": false})).map(|_| ())).unwrap();
        })
    };
    let model = {
        let (config, keys, done) = (config.clone(), keys.clone(), done_tx);
        std::thread::spawn(move || {
            let body = json!({"label": "Local", "agent": "codex", "model": "gpt-5", "base_url": "http://localhost:1"});
            done.send(setup::save(&config, &keys, &body).map(|_| ())).unwrap();
        })
    };
    let lock = config.with_extension("lock");
    let mut waiting = 0;
    while waiting < 2 {
        waiting += usize::from(attempt_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() == lock);
    }
    assert!(done_rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "a settings write ran while the lock was held");
    *setup::LOCK_ATTEMPT.lock().unwrap() = None;
    drop(held);
    for _ in 0..2 {
        done_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap().unwrap();
    }
    language.join().unwrap();
    model.join().unwrap();
    let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(v["languages"]["rust"].as_bool(), Some(false));
    assert!(v["models"].as_array().unwrap().iter().any(|m| m["id"].as_str() == Some("local")));
    assert_eq!(v["task_timeout"].as_integer(), Some(42));
}

/// A checkpoint failure in each of two projects reaches `settle`'s collection loop.
#[test]
fn settling_keeps_every_non_busy_checkpoint_error() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"b\"\nversion = \"1.0.0\"\n"),
        ("b/src/app.gleam", "pub fn one() {\n  1\n}\n"),
    ]);
    for d in ["a", "b"] {
        crate::import(&root.join(d)).unwrap();
    }
    let (a, b) = (open(&root.join("a")).unwrap(), open(&root.join("b")).unwrap());
    let mut t = api(&a.root, "POST", "/tasks", json!({"title": "Both"})).unwrap();
    t["scope"] = json!(["a", "b"]);
    // An open write transaction makes each checkpoint fail, beyond the busy log
    // that `settle_result` reports as pending.
    crate::COMMAND_DB.set(vec![(a.root.clone(), a.db.clone()), (b.root.clone(), b.db.clone())]);
    for st in [&a, &b] {
        st.db.execute_batch("BEGIN IMMEDIATE").unwrap();
    }
    let e = settle(&a, &t).unwrap_err();
    crate::COMMAND_DB.set(vec![]);
    for st in [&a, &b] {
        st.db.execute_batch("ROLLBACK").unwrap();
    }
    assert_eq!(e.0, "a: store error: database table is locked\nb: store error: database table is locked", "{}", e.0);
    assert_eq!(e.1, 1);
}

/// Every project's checkpoint error is reported, with the first one's code.
#[test]
fn settle_result_keeps_every_non_busy_checkpoint_error() {
    let e = super::settle_result(
        vec!["a".into()],
        vec![("b".into(), Failure("E1234 disk full".into(), 7)), ("c".into(), Failure("E5678 read only".into(), 9))],
    )
    .unwrap_err();
    assert_eq!(e.0, "b: E1234 disk full\nc: E5678 read only\nAlso, flushing store.db is pending in a.", "{}", e.0);
    assert_eq!(e.1, 7);
}

/// A task with run `run`, whose agent exits with `code` after writing `lines` to its log.
fn exited(run: Value, code: i32, lines: &[Value]) -> (testing::Project, String, std::process::Child, std::path::PathBuf) {
    let rid = run["id"].as_str().unwrap().to_string();
    let (p, tid, child) = launched(run, Command::new("sh").arg("-c").arg(format!("exit {code}")).process_group(0).spawn().unwrap());
    let log = p.root.join(".solace").join(format!("{rid}.log"));
    fs::write(&log, lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    (p, tid, child, log)
}

fn launched(run: Value, child: std::process::Child) -> (testing::Project, String, std::process::Child) {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Supervise"})).unwrap();
    t["runs"] = json!([run]);
    tasks::put(&s, &t).unwrap();
    (p, t["id"].as_str().unwrap().to_string(), child)
}

/// A Codex log entry where the agent says `text`.
fn agent_says(text: &str) -> Value {
    json!({"type":"item.completed","item":{"type":"agent_message","text":text}})
}

#[test]
fn a_review_that_exits_with_its_assessment_is_reviewed() {
    let assessment = "**Assessment: Looks good**\n\n### Findings\n\nFine.";
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[agent_says(assessment)]);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    assert_eq!(t["finished"]["outcome"], "reviewed");
    assert!(t["attention"].is_null());
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["author"] == "agent" && n["run"] == "r1" && n["text"] == assessment));
}

#[test]
fn a_refused_review_note_still_recovers() {
    let assessment = "**Assessment: Needs work**\n\n### Findings\n\nFix it.";
    let refused = json!({
        "type":"item.completed",
        "item":{
            "id":"i1",
            "type":"mcp_tool_call",
            "tool":"solace",
            "arguments":{"command":"task note"},
            "result":{"content":[{"type":"text","text":"E7015\nnothing was sent"}]},
            "status":"failed"
        }
    });
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[refused, agent_says(assessment)]);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["run"] == "r1" && n["text"] == assessment));
}

#[test]
fn a_review_fallback_keeps_existing_attention() {
    let assessment = "**Assessment: Looks good**\n\nFine.";
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[agent_says(assessment)]);
    tasks::update(&p.root, &tid, |t| {
        t["attention"] = json!({"reason":"look here","time":1});
        Ok(())
    })
    .unwrap();
    let before = task(&p.root, &tid).unwrap()["attention"].clone();
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap()
    );
    assert_eq!(task(&p.root, &tid).unwrap()["attention"], before);
}

#[test]
fn a_review_with_recorded_feedback_is_not_noted_twice() {
    let assessment = "**Assessment: Looks good**\n\nFine.";
    let (p, tid, _child, _log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[agent_says(assessment)]);
    tasks::update(&p.root, &tid, |t| {
        note(t, "agent", Some("r1"), assessment);
        Ok(())
    })
    .unwrap();
    let before = task(&p.root, &tid).unwrap()["notes"].as_array().unwrap().len();
    let checkpoint = review_path(&p.root, &tid, "r1").unwrap();
    fs::create_dir_all(checkpoint.parent().unwrap()).unwrap();
    fs::write(&checkpoint, br#"{"run":"r1","changed_during_review":false}"#).unwrap();
    let saved = fs::read(&checkpoint).unwrap();
    assert!(!record_review(&Watch::Task(&p.root, &tid), "r1", assessment).unwrap());
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["notes"].as_array().unwrap().len(), before);
    assert_eq!(t["runs"][0]["state"], "running");
    assert_eq!(fs::read(&checkpoint).unwrap(), saved);
}

#[test]
fn a_review_without_an_assessment_still_fails() {
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[agent_says("Done.")]);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["state"], "ended");
    assert_ne!(t["runs"][0]["outcome"], "reviewed");
    assert!(t["attention"]["reason"].as_str().unwrap().contains("without finishing"));
    assert!(
        t["notes"].as_array().unwrap().iter().any(|n| n["author"] == "solace" && n["text"].as_str().unwrap().contains("without finishing"))
    );
}

#[test]
fn only_a_clean_labeled_review_recovers() {
    let assessment = "**Assessment: Looks good**\n\nFine.";
    let review = json!({"kind":"review"});
    let said = [json!({"kind":"message","text":assessment})];
    assert_eq!(recoverable_review(&review, true, &said).as_deref(), Some(assessment));
    assert!(recoverable_review(&review, false, &said).is_none());
    let error = [json!({"kind":"error","text":"boom"}), json!({"kind":"message","text":assessment})];
    assert!(recoverable_review(&review, true, &error).is_none());
    for kind in ["work", "investigate"] {
        let run = json!({"kind":kind});
        assert!(recoverable_review(&run, true, &said).is_none(), "{kind}");
    }
    for text in ["**Assessment: Looks good** extra", "**Assessment: Looks good", "Assessment: Looks good"] {
        let wrong = [json!({"kind":"message","text":text})];
        assert!(recoverable_review(&review, true, &wrong).is_none(), "{text}");
    }
    assert!(recoverable_review(&review, true, &[]).is_none());
    let recorded = [
        json!({"kind":"tool","title":"solace task note","output":"Recorded.\n","status":"ok"}),
        json!({"kind":"message","text":assessment}),
    ];
    assert!(recoverable_review(&review, true, &recorded).is_none());
    let queued = [
        json!({
            "kind":"tool",
            "title":"solace task note",
            "output":"Recorded; it reaches the board when `solace serve` runs.\n",
            "status":"ok"
        }),
        json!({"kind":"message","text":assessment}),
    ];
    assert!(recoverable_review(&review, true, &queued).is_none());
    let batched = [
        json!({
            "kind":"tool",
            "title":"solace inspect a\ntask note",
            "output":"==== solace inspect a ====\nfn a() {}\n==== solace task note ====\nRecorded.\n",
            "status":"ok"
        }),
        json!({"kind":"message","text":assessment}),
    ];
    assert!(recoverable_review(&review, true, &batched).is_none());
    let batched_queued = [
        json!({
            "kind":"tool",
            "title":"solace task note\ninspect a",
            "output":"==== solace task note ====\nRecorded; it reaches the board when `solace serve` runs.\n==== solace inspect a ====\nfn a() {}\n",
            "status":"ok"
        }),
        json!({"kind":"message","text":assessment}),
    ];
    assert!(recoverable_review(&review, true, &batched_queued).is_none());
    let batched_refused = [
        json!({
            "kind":"tool",
            "title":"solace inspect a\ntask note",
            "output":"==== solace inspect a ====\nRecorded in the file\n==== solace task note ====\nE7015\nnothing was sent\n",
            "status":"failed"
        }),
        json!({"kind":"message","text":assessment}),
    ];
    assert_eq!(recoverable_review(&review, true, &batched_refused).as_deref(), Some(assessment));
    let refused = [
        json!({"kind":"tool","title":"solace task note","output":"E7015\nnothing was sent","status":"failed"}),
        json!({"kind":"message","text":assessment}),
    ];
    assert_eq!(recoverable_review(&review, true, &refused).as_deref(), Some(assessment));
}

/// D-184: only a work run names the next step.
#[test]
fn a_review_run_gets_no_next_step() {
    let (root, s) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let mut t = api(&root, "POST", "/tasks", json!({"title": "Review"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([
        {"id":"r1","kind":"review","state":"running"},
        {"id":"r2","kind":"work","state":"running"}
    ]);
    tasks::put(&s, &t).unwrap();
    assert_eq!(with_context(&tid, "r1", || crate::next_step(Some("app.one"), work_run(&root))), "");
    assert_eq!(
        with_context(&tid, "r2", || crate::next_step(None, work_run(&root))),
        "Next: if every part of the task is changed, finish with task done.\n"
    );
}

/// A current entry names a shared model before a gone entry does.
#[test]
fn a_current_model_names_the_shared_model_before_a_gone_one() {
    let configured = json!([{"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"}]);
    let all = json!({
        "codex":{"models":[
            {"id":"second","label":"Second","agent":"codex","model":"claude-opus-5-5"}
        ]},
        "gone":{
            "old":{"id":"old","label":"Gone","agent":"codex","model":"claude-opus-5-5"}
        },
        "used":{"claude-opus":"claude-opus-5-5"}
    });
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m[0]["label"], "Second", "a current entry names the shared model");

    let gone_only = json!({
        "gone":{"old":{"id":"old","label":"Gone","agent":"codex","model":"claude-opus-5-5"}},
        "used":{"claude-opus":"claude-opus-5-5"}
    });
    let m = agents::merge_from(&configured, &gone_only);
    assert_eq!(m[0]["label"], "Gone", "a gone entry still names it when no current one does");
}

/// D-186: an investigation states outcomes and constraints, not a plan that
/// assigns edits, submission calls or checks.
#[test]
fn investigate_briefs_report_requirements_not_edits() {
    let code = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [{"entity": "symbol:x", "name": "calc.add", "run": "r1"}],
        "test_commands": [{"project": "a", "commands": ["rust-acceptance"]}],
    });
    let settings = json!({
        "id": "t2",
        "title": "Tune the build",
        "body": "Allow the new test command.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [{"entity": "file:solace.toml", "name": "solace.toml", "run": "r2"}],
    });
    for (label, task, named) in [("code change", &code, "calc.add"), ("settings change", &settings, "solace.toml")] {
        let prompt = brief(task, "investigate");
        for required in [
            "Investigate this task",
            "Report the required behavior, constraints, acceptance criteria and risks",
            "Name the symbols, modules and non-source files that need attention",
            "anything you didn't check",
            named,
        ] {
            assert!(prompt.contains(required), "{label}: missing {required}\n{prompt}");
        }
        assert!(prompt.contains(&format!("Task changes:\n- {named}")), "{label}: missing the change it names\n{prompt}");
        for directive in [
            "`submit` then `test`",
            "`write` then `run",
            "`test --all`",
            "settings proposal",
            "To change `solace.toml` or `AGENTS.md`, `write` it alone",
        ] {
            assert!(!prompt.contains(directive), "{label}: unexpected {directive}\n{prompt}");
        }
        assert!(prompt.contains("This run is read-only"), "{label}: missing the read-only notice\n{prompt}");
        assert!(
            prompt.contains("Probe or run tests only when needed to more thoroughly define the issue."),
            "{label}: missing the limit on probes and tests\n{prompt}"
        );
        assert!(!prompt.contains("Use those to verify (for example test suites)"), "{label}\n{prompt}");
    }
    let review = brief(&code, "review");
    assert!(review.contains("Use those to verify (for example test suites)"), "{review}");
    assert!(!review.contains("Probe or run tests only when needed"), "{review}");
    let work = brief(&code, "work");
    for owned in [
        "`submit` then `test`",
        "`write` then `run <name>`",
        "`test --all`",
        "To change `solace.toml` or `AGENTS.md`, `write` it alone, with the why in `reason`.",
    ] {
        assert!(work.contains(owned), "work: missing {owned}\n{work}");
    }
}

fn event_board(top: PathBuf) -> std::sync::Arc<Board> {
    std::sync::Arc::new(Board::new(top, 0))
}

/// A position the pruned log can no longer reach gets the file snapshot.
#[test]
fn an_event_id_before_the_pruned_log_starts_from_the_files() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    api(&s.root, "POST", "/tasks", json!({"title":"During gap"})).unwrap();
    let top = scope::top(&p.root);
    let db = tasks::events(&top).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"older\":true}')", []).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"newest\":true}')", []).unwrap();
    db.execute("DELETE FROM events WHERE seq < (SELECT MAX(seq) FROM events)", []).unwrap();
    let last = json!({"tasks": 1, ".": 0}).to_string();
    let mut reader = Events::new(event_board(top), Some(p.root.clone()), Some(&last));
    let mut text = String::new();
    let mut buf = [0; 8192];
    while !text.contains("event: ready") {
        let n = reader.read(&mut buf).unwrap();
        assert!(n > 0);
        text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
    }
    assert!(text.contains("\"workflow\""), "no workflow snapshot in {text}");
    assert!(text.contains("During gap"), "no task snapshot in {text}");
    assert!(!text.contains("\"newest\":true"), "replayed only part of the gap in {text}");
}

/// A current position still gets just what followed it, without a snapshot.
#[test]
fn an_event_id_at_the_newest_log_row_replays_only_new_events() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let db = tasks::events(&top).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"older\":true}')", []).unwrap();
    let last = json!({"tasks": 1, ".": 0}).to_string();
    let mut reader = Events::new(event_board(top), Some(p.root.clone()), Some(&last));
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"newest\":true}')", []).unwrap();
    let mut buf = [0; 8192];
    let n = reader.read(&mut buf).unwrap();
    let text = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(text.contains("\"newest\":true"), "{text}");
    assert!(!text.contains("\"workflow\""), "{text}");
}

/// A position past the log's end, as after a recreated log, also snaps.
#[test]
fn an_event_id_past_the_log_starts_from_the_files() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let db = tasks::events(&top).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{}')", []).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{}')", []).unwrap();
    db.execute("DELETE FROM events WHERE seq < 2", []).unwrap();
    let fresh = |tasks: i64| {
        let last = json!({"tasks": tasks, ".": 0}).to_string();
        Events::new(event_board(top.clone()), Some(p.root.clone()), Some(&last)).fresh
    };
    assert!(fresh(0));
    assert!(!fresh(1));
    assert!(!fresh(2));
    assert!(fresh(3));
}

/// A connected stream that loses rows between polls rebuilds from the files.
#[test]
fn a_prune_while_a_stream_is_open_starts_it_from_the_files() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let db = tasks::events(&top).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"older\":true}')", []).unwrap();
    let last = json!({"tasks": 1, ".": 0}).to_string();
    let mut reader = Events::new(event_board(top.clone()), Some(p.root.clone()), Some(&last));
    let mut buf = [0; 8192];
    let n = reader.read(&mut buf).unwrap();
    assert!(std::str::from_utf8(&buf[..n]).unwrap().contains("event: ready"));
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"kept\":true}')", []).unwrap();
    db.execute("INSERT INTO events(project,data) VALUES ('', '{\"newest\":true}')", []).unwrap();
    db.execute("DELETE FROM events WHERE seq < (SELECT MAX(seq) FROM events)", []).unwrap();
    let n = reader.read(&mut buf).unwrap();
    let text = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(text.contains("\"workflow\""), "no snapshot after a live prune in {text}");
    assert!(!text.contains("\"kept\":true"), "replayed only part of the gap in {text}");
}

#[test]
fn agents_say_when_their_sign_in_has_expired_or_they_need_an_update() {
    let fixture: Value = serde_json::from_str(USAGE_LIMITS).unwrap();
    for k in agents::KINDS {
        for said in fixture["sign_in"][k.name].as_array().unwrap() {
            assert!((k.signed_out)(said.as_str().unwrap()), "{said}");
        }
        if let Some((update, _)) = k.update {
            for said in fixture["update"][k.name].as_array().unwrap() {
                assert!(update(said.as_str().unwrap()), "{said}");
            }
        }
        for said in fixture["not_limits"].as_array().unwrap() {
            let said = said.as_str().unwrap();
            assert!(!(k.signed_out)(said) && !k.update.is_some_and(|(update, _)| update(said)), "{said}");
        }
    }
}

#[test]
fn a_sign_in_failure_marks_the_sign_in_until_try_again_or_a_successful_run() {
    let fixture: Value = serde_json::from_str(USAGE_LIMITS).unwrap();
    let dir = testing::Scratch::new();
    let (path, log) = (dir.join("agent-holds.json"), dir.join("log.jsonl"));
    let claude = json!({"agent":"claude","model":"m"});
    let said = fixture["sign_in"]["claude"][0].as_str().unwrap();
    fs::write(&log, json!({"type":"error","message":said}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    let reason = "Claude Code's sign-in has expired. Open Claude Code once and sign in, then press Try again.";
    // It has no reset: it holds until cleared, even far in the future.
    assert_eq!(agents::held(&path, "claude", NOON + 365 * 86400).as_deref(), Some(reason));
    assert!(agents::sign_in_mark(&path, "claude", NOON));
    assert!(agents::clear(&path, "claude").unwrap());
    agents::mark_sign_in(&path, "claude", reason).unwrap();
    assert!(agents::sign_in_mark(&path, "claude", NOON));
    // A run that answers clears it.
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &claude, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON), None);
    // An update needed is not a sign-in mark.
    let none = dir.join("none.json");
    let update = fixture["update"]["claude"][0].as_str().unwrap();
    fs::write(&log, json!({"type":"error","message":update}).to_string()).unwrap();
    agents::observe(&none, &claude, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    assert!(!none.exists());
    // A passing mention inside a longer message marks nothing.
    fs::write(
        &log,
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"I saw: OAuth token has expired\nThen I carried on."}]}})
            .to_string(),
    )
    .unwrap();
    agents::observe(&none, &claude, &log, NOON, NOON, Some("finished"), &zones).unwrap();
    assert!(!none.exists());
}

#[test]
fn a_sign_in_failure_is_a_plain_message() {
    let said = "Failed to authenticate: OAuth session expired and could not be refreshed";
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 1, &[json!({"type":"error","message":said})]);
    let m = json!({"agent":"claude","model":"m"});
    assert!(!supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &m, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
    let t = task(&p.root, &tid).unwrap();
    let plain = "Claude Code's sign-in has expired. Open Claude Code once and sign in, then press Try again.";
    assert_eq!(t["attention"]["reason"], plain);
    assert_eq!(t["runs"][0]["outcome"], plain);
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["author"] == "solace" && n["text"] == plain));
    assert!(!t["notes"].as_array().unwrap().iter().any(|n| n["text"].as_str().unwrap_or("").contains("agent exited")));
    // The raw words are still in the run's transcript.
    assert!(transcript(&log).iter().any(|e| e["text"] == said));
}

#[test]
fn an_update_needed_is_a_plain_message() {
    let said = "Claude Code needs an update. A newer version (2.0.0 or higher) is required to continue.";
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 1, &[json!({"type":"error","message":said})]);
    let m = json!({"agent":"claude","model":"m"});
    assert!(!supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &m, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["attention"]["reason"], "Claude Code needs an update. Update Claude Code, then start the task again.");
    assert_eq!(t["runs"][0]["outcome"], "Claude Code needs an update. Update Claude Code, then start the task again.");
}

#[test]
fn a_missing_program_is_reported_plainly_at_launch() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    let m = json!({"agent":"codex","run":["solace-no-such-program"]});
    let e = spawn_agent(&p.root, &dir, &log, &m, "", &[], None, false, ["t", "r"], 60).unwrap_err();
    assert_eq!(e.0, "solace-no-such-program is not installed. Install it, then start the task again.");
}

#[test]
fn sign_in_helpers_say_what_happened_and_what_to_do() {
    let codex = agents::kind("codex").unwrap();
    let claude = agents::kind("claude").unwrap();
    let _: &agents::Kind = claude;
    assert!(agents::codex_signed_out("Your access token could not be refreshed"));
    assert!(agents::claude_signed_out("OAuth token has expired"));
    assert!(agents::claude_update("A newer version (2.0 or higher) is required to continue."));
    assert_eq!(agents::sign_in_message(codex), "Codex's sign-in has expired. Open Codex once and sign in, then press Try again.");
    assert_eq!(agents::update_message(claude), "Claude Code needs an update. Update Claude Code, then start the task again.");
    assert_eq!(agents::missing_program(&json!({"agent":"claude"})), "Claude Code is not installed. Install it.");
    assert_eq!(agents::program_name(&json!({"agent":"claude"})), "Claude Code");
    assert_eq!(agents::missing_program(&json!({"agent":"claude","run":[]})), "Claude Code is not installed. Install it.");
    assert_eq!(
        agents::missing_program_at_launch(&json!({"agent":"claude","run":[]})),
        "Claude Code is not installed. Install it, then start the task again."
    );
    assert!(agents::codex_signed_out_whole("Please log out and sign in again"));
    assert!(!agents::codex_signed_out_whole("I saw: Please log out and sign in again"));
    assert!(agents::claude_signed_out_whole("OAuth token has expired"));
    assert!(!agents::claude_signed_out_whole("I saw: OAuth token has expired"));
    assert!(agents::claude_update_whole("is required to continue"));
    assert!(!agents::claude_update_whole("I saw: is required to continue"));
    assert!(claude.update.is_some());
    assert!(codex.update.is_none());
    assert!(agents::whole_in("Please run /login", &agents::CLAUDE_SIGNED_OUT));
    assert!(agents::CLAUDE_SIGNED_OUT.contains(&"Please run /login"));
    assert!(agents::CLAUDE_UPDATE.contains(&"needs an update"));
    assert!(agents::CODEX_SIGNED_OUT.contains(&"Please log out and sign in again"));
    let entries = [json!({"kind":"error","text":"OAuth token has expired"})];
    assert!(agents::said(&entries, agents::claude_signed_out, agents::claude_signed_out_whole));
    let login = [json!({"kind":"error","text":"Invalid API key · Please run /login"})];
    assert!(agents::said(&login, agents::claude_signed_out, agents::claude_signed_out_whole));
    assert_eq!(agents::stopped(&json!({"agent":"claude","model":"m"}), &login).unwrap(), agents::sign_in_message(claude));
    assert!(agents::said(
        &[json!({"kind":"message","text":"Please run /login"})],
        agents::claude_signed_out,
        agents::claude_signed_out_whole
    ));
    assert!(!agents::said(
        &[json!({"kind":"message","text":"I saw: Please run /login"})],
        agents::claude_signed_out,
        agents::claude_signed_out_whole
    ));
    assert_eq!(agents::stopped(&json!({"agent":"claude","model":"m"}), &entries).unwrap(), agents::sign_in_message(claude));
    assert!(agents::stopped(&json!({"agent":"claude","model":"m","run":["sh"]}), &entries).is_none());
    assert!(agents::stopped(&json!({"agent":"claude","model":"m","endpoint":{"base_url":"http://x"}}), &entries).is_none());
    let limit = [json!({"kind":"error","text":"You've hit your limit · resets 2pm (UTC)"})];
    assert!(agents::stopped(&json!({"agent":"claude","model":"m"}), &limit).is_none());
    let update = [json!({"kind":"error","text":"A newer version (2.0 or higher) is required to continue."})];
    assert_eq!(agents::stopped(&json!({"agent":"claude","model":"m"}), &update).unwrap(), agents::update_message(claude));
    assert!(agents::stopped(&json!({"agent":"codex","model":"m"}), &update).is_none());
}

#[test]
fn an_ended_strategy_run_starts_nothing() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let runs = top.join(".solace/strategy/runs");
    fs::create_dir_all(&runs).unwrap();
    fs::write(runs.join("r1.json"), json!({"id":"r1","kind":"strategy","state":"ended"}).to_string()).unwrap();
    assert!(super::strategy::strategy_process(&top, "r1").is_ok());
    assert!(super::strategy::strategy_process(&top, "r9").unwrap_err().0.starts_with("E7023"));
}

#[test]
fn agent_hold_helpers_cover_each_case() {
    let claude = agents::kind("claude").unwrap();
    let codex = agents::kind("codex").unwrap();
    assert!(agents::holds_path().ends_with("agent-holds.json"));
    let limit = json!({"kind":"error","text":"You've hit your limit · resets 2pm (UTC)"});
    let said = [limit.clone()];
    assert!(agents::limit_hit(claude, &said).is_some());
    let whole = json!({"kind":"message","text":"You've hit your limit · resets 2pm (UTC)"});
    assert!(agents::limit_hit(claude, &[whole.clone()]).is_some());
    let mention = json!({"kind":"message","text":"I noted: You've hit your limit · resets 2pm (UTC)\nThen I carried on."});
    assert!(agents::limit_hit(claude, &[mention]).is_none());
    assert!(!agents::limit_message(claude, &limit));
    assert!(agents::limit_message(claude, &whole));
    assert!(agents::answered(claude, &[json!({"kind":"message","text":"Done."})]));
    assert!(!agents::answered(claude, &said));
    assert!(agents::succeeded(Some("finished")));
    assert!(agents::succeeded(Some("reviewed")));
    assert!(!agents::succeeded(Some("stopped")));
    let gone = [json!({"kind":"error","text":"model not found"})];
    assert!(agents::model_is_gone(codex, &gone));
    assert!(!agents::model_is_gone(claude, &said));
    assert_eq!(agents::held_reason(&json!({"reason":"held"})), "held");
    assert_eq!(agents::held_reason(&json!({})), "usage limit reached");
    let dir = testing::Scratch::new();
    let path = dir.join("agent-holds.json");
    let old = dir.join("agent-limits.json");
    fs::write(&old, "{}").unwrap();
    agents::save_marks(&path, &json!({})).unwrap();
    assert!(!old.exists());
    agents::write_mark(&path, "claude", "held", Some(NOON + 60), Some("sign_in")).unwrap();
    assert!(agents::held_mark(&path, "claude", NOON).is_some());
    assert!(agents::held_mark(&path, "claude", NOON + 60).is_none());
    agents::mark_limit(&path, claude, Some("2pm (UTC)".into()), NOON, &zones).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON).as_deref(), Some("Claude Code usage limit reached (resets 2pm (UTC))"));
    agents::mark_sign_in(&path, "claude", "sign-in").unwrap();
    agents::clear_before(&path, "claude", u64::MAX).unwrap();
    assert_eq!(agents::held(&path, "claude", NOON), None);
}

/// A task in Planning with one recorded change, moved to its final column
/// without settling: its undo records are still in the store.
fn unsettled_final_task() -> (testing::Project, String) {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Fast move"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"ended","outcome":"stopped"}]);
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"plan"})).unwrap();
    let before = snapshot(&s).unwrap();
    s.submit_symbols(None, Some("geo"), "pub fn side() {\n  2\n}\n").unwrap();
    record(&s, &tid, "r1", &before, &snapshot(&s).unwrap(), None).unwrap();
    let (t, after) = change(&s.root, &tid, "POST", &["actions"], &json!({"action":"complete"})).unwrap();
    assert!(matches!(after, After::Clean(_)));
    tasks::put(&s, &t).unwrap();
    (p, tid)
}

#[test]
fn a_final_move_replies_before_its_undo_records_are_dropped() {
    let (p, tid) = unsettled_final_task();
    let s = open(&p.root).unwrap();
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    assert_eq!(task(&s.root, &tid).unwrap()["column"], "complete");
    assert!(has_before(), "the move was written before its undo records were dropped");
    settle_final(&s.root, &tid).unwrap();
    assert!(!has_before(), "the deferred settle dropped them");
}

#[test]
fn a_failed_deferred_settle_is_flagged_and_resettle_clears_it() {
    let (p, tid) = unsettled_final_task();
    let s = open(&p.root).unwrap();
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    fs::write(s.root.join("solace.toml"), "not = [").unwrap();
    tasks::settle_later(&s.root, tid.clone());
    let t = task(&s.root, &tid).unwrap();
    assert_eq!(t["settleFailed"], true);
    assert_eq!(t["attention"]["reason"], crate::tasks::SETTLE_FAILED_REASON);
    assert_eq!(settle_failures(&t), 1);
    assert!(has_before());
    tasks::settle_later(&s.root, tid.clone());
    assert_eq!(settle_failures(&task(&s.root, &tid).unwrap()), 1, "a repeated failure adds no note");
    fs::remove_file(s.root.join("solace.toml")).unwrap();
    resettle(&s.root);
    let t = task(&s.root, &tid).unwrap();
    assert!(t["settleFailed"].is_null());
    assert!(t["attention"].is_null());
    assert_eq!(settle_failures(&t), 0);
    assert!(!has_before());
    let approved = s.root.join(".solace/approved");
    assert!(approved.exists());
    fs::remove_file(&approved).unwrap();
    tasks::settle_later(&s.root, tid);
    assert!(!approved.exists(), "a retry approves nothing twice");
}

#[test]
fn resettle_retries_a_marked_task_without_undo_records() {
    let (p, tid) = unsettled_final_task();
    let s = open(&p.root).unwrap();
    drop_before(&s, &tid).unwrap();
    let mut t = task(&s.root, &tid).unwrap();
    t["settleFailed"] = json!(true);
    tasks::put(&s, &t).unwrap();
    resettle(&s.root);
    assert!(task(&s.root, &tid).unwrap()["settleFailed"].is_null());
}

/// A new project's language is `gleam` when absent and otherwise must name
/// one of the two languages, whatever JSON type it came as.
#[test]
fn a_new_project_language_is_gleam_or_rust() {
    let root = testing::Scratch::new();
    let board = event_board(root.to_path_buf());
    for language in [json!("python"), json!(5)] {
        let body = json!({"name": "ok", "language": language});
        let e = board.projects_api("POST", "/projects/new", &body).unwrap_err().0;
        assert!(e.contains("language must be \"gleam\" or \"rust\""), "{e}");
    }
    assert!(!root.join("ok").exists());
}

#[test]
fn a_missing_agent_program_is_checked_now() {
    let reason = agents::local_now(&json!({"run": ["solace-agent-that-does-not-exist"]}));
    assert!(reason.as_ref().is_some_and(|reason| reason.contains("is not installed")), "{reason:?}");
}

#[test]
fn an_agents_status_answer_is_the_sign_in() {
    assert_eq!(agents::status_answer(true, r#"{"loggedIn":true,"authMethod":"claude.ai"}"#), Some(true));
    assert_eq!(agents::status_answer(false, r#"{"loggedIn":false}"#), Some(false));
    assert_eq!(agents::status_answer(true, "Logged in using ChatGPT"), Some(true));
    assert_eq!(agents::status_answer(false, "Not logged in"), Some(false));
    assert_eq!(agents::status_answer(false, "error: unrecognized subcommand"), None);
    assert_eq!(agents::status_answer(true, ""), None);
    assert!(!agents::nonempty_key(Some("  ".into())) && !agents::nonempty_key(None));
    assert!(agents::nonempty_key(Some("sk-x".into())));
    let home = testing::Scratch::new();
    let (codex, claude) = (json!({"agent":"codex","model":"m"}), json!({"agent":"claude","model":"m"}));
    let elsewhere = home.join("elsewhere");
    let folder = |k: &agents::Kind| if k.name == "claude" { elsewhere.clone() } else { home.join(".codex") };
    let no_key = |_: &str| false;
    let (signed_in, signed_out, unknown) = (Some(true), Some(false), None);
    let local = |m: &Value, answer: Option<bool>| {
        agents::local(
            m,
            &agents::Here {
                folder: &folder,
                status: &move |_| answer,
                var: &no_key,
                stored: &no_key,
                installed: &|_| true,
                now: NOON * 1000,
            },
        )
    };
    // Signed in by the program's word, with no file and no key.
    assert!(local(&claude, signed_in).is_none());
    assert!(local(&codex, signed_in).is_none());
    // A signed-out answer holds when no key is set; a token file doesn't override it.
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/auth.json"), json!({"tokens":{"refresh_token":"r"}}).to_string()).unwrap();
    assert!(local(&codex, unknown).is_none());
    assert_eq!(local(&codex, signed_out).unwrap(), "codex is not signed in");
    // Claude Code's key is a sign-in even when it answers signed out; Codex's
    // key only stands in when it answers unclearly.
    let with_key = |m: &Value, key: &'static str, answer: Option<bool>| {
        let var = move |v: &str| v == key;
        let status = move |_: &agents::Kind| answer;
        agents::local(
            m,
            &agents::Here { folder: &folder, status: &status, var: &var, stored: &no_key, installed: &|_| true, now: NOON * 1000 },
        )
    };
    assert!(with_key(&claude, "ANTHROPIC_API_KEY", signed_out).is_none());
    assert_eq!(with_key(&codex, "OPENAI_API_KEY", signed_out).unwrap(), "codex is not signed in");
    assert!(with_key(&codex, "OPENAI_API_KEY", unknown).is_none());
    // Claude's file is read from its configured folder, not the home folder.
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/.credentials.json"), json!({"claudeAiOauth":{"refreshToken":"r"}}).to_string()).unwrap();
    if !cfg!(target_os = "macos") {
        assert_eq!(local(&claude, unknown).unwrap(), "claude is not signed in");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join(".credentials.json"), json!({"claudeAiOauth":{"refreshToken":"r"}}).to_string()).unwrap();
        assert!(local(&claude, unknown).is_none());
    }
}

#[test]
fn an_agents_refusal_of_an_endpoint_model_holds_that_model_alone() {
    let dir = testing::Scratch::new();
    let (path, log) = (dir.join("agent-holds.json"), dir.join("log.jsonl"));
    let endpoint = json!({"id":"gw","agent":"claude","model":"x","endpoint":{"base_url":"http://x"}});
    let native = json!({"agent":"claude","model":"m"});
    assert_eq!(agents::hold_key(&endpoint).as_deref(), Some("model:gw"));
    assert_eq!(agents::hold_key(&native).as_deref(), Some("claude"));
    assert_eq!(
        agents::hold_key(&json!({"agent":"claude","model":"x","endpoint":{"base_url":"http://x"}})).as_deref(),
        Some("model:claude:x")
    );
    fs::write(&log, json!({"type":"error","message":"There is an issue with the selected model (x): model not found"}).to_string())
        .unwrap();
    agents::observe(&path, &endpoint, &log, NOON, NOON, Some("stopped"), &zones).unwrap();
    assert_eq!(agents::held(&path, "model:gw", NOON).as_deref(), Some("There is an issue with the selected model (x): model not found"));
    assert!(!agents::sign_in_mark(&path, "model:gw", NOON));
    assert_eq!(agents::held(&path, "claude", NOON), None);
    fs::write(&log, json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}).to_string()).unwrap();
    agents::observe(&path, &endpoint, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "model:gw", NOON), None);
}

#[test]
fn program_status_reads_its_command() {
    let kind = |name: &'static str, status: &'static [&'static str]| agents::Kind {
        name,
        label: "Test",
        folder: || std::path::PathBuf::new(),
        files: &[],
        status,
        keys: &[],
        keys_override_status: false,
        keychain: None,
        expired: |_, _| false,
        limit: |_| None,
        signed_out: |_| false,
        signed_out_whole: |_| false,
        update: None,
        discover: || Vec::new(),
        gone: |_| false,
    };
    assert_eq!(agents::program_status(&kind("sh", &[])), None);
    assert_eq!(agents::program_status(&kind("solace-no-such-status-command", &["status"])), None);
    assert_eq!(agents::program_status(&kind("sh", &["-c", "printf 'Logged in'"])), Some(true));
    assert_eq!(agents::program_status(&kind("sh", &["-c", "printf 'Not logged in'; exit 1"])), Some(false));
    assert_eq!(agents::program_status(&kind("sh", &["-c", "printf 'maybe'"])), None);
    assert_eq!(agents::program_status_within(&kind("sh", &["-c", "sleep 1"]), std::time::Duration::from_millis(50)), None);
}

#[test]
fn codex_folder_prefers_codex_home() {
    let path = |home: Option<&str>| home.map(std::ffi::OsString::from);
    assert_eq!(agents::codex_folder_from(path(Some("/tmp/codex")), path(Some("/home/me"))), std::path::PathBuf::from("/tmp/codex"));
    assert_eq!(agents::codex_folder_from(None, path(Some("/home/me"))), std::path::PathBuf::from("/home/me").join(".codex"));
    assert_eq!(agents::codex_folder_from(None, None), std::path::PathBuf::from(".codex"));
}

#[test]
fn refusal_finds_the_first_matching_error_line() {
    let codex = agents::kind("codex").unwrap();
    assert!(agents::refusal(codex, &[json!({"kind":"message","text":"model not found"})]).is_none());
    assert_eq!(
        agents::refusal(codex, &[json!({"kind":"error","text":"unrelated"}), json!({"kind":"error","text":"  model not found  \nmore"})],)
            .as_deref(),
        Some("model not found")
    );
    assert_eq!(
        agents::refusal(codex, &[json!({"kind":"error","text":"Please log out and sign in again"})]).as_deref(),
        Some("Please log out and sign in again")
    );
    assert_eq!(
        agents::refusal(
            codex,
            &[
                json!({"kind":"error","text":"Model metadata for `x` not found. Defaulting to fallback metadata."}),
                json!({"kind":"error","text":"model not found"}),
            ],
        )
        .as_deref(),
        Some("model not found")
    );
    let long = format!("model not found {}", "x".repeat(300));
    assert_eq!(agents::refusal(codex, &[json!({"kind":"error","text":long})]).unwrap().chars().count(), 240);
}

#[test]
fn try_again_keeps_a_usage_limit_that_has_not_reset() {
    let dir = testing::Scratch::new();
    let config = dir.join("config.toml");
    let holds = dir.join("agent-holds.json");
    let now = now();
    agents::save_marks(
        &holds,
        &json!({
            "codex": {"reason":"Codex usage limit reached","until":now + 3600,"time":now},
            "claude": {"reason":"Claude Code usage limit reached","until":now - 1,"time":now},
            "signed": {"reason":"sign-in expired","until":null,"time":now,"kind":"sign_in"},
            "model:x": {"reason":"model not found","until":null,"time":now,"kind":"refusal"},
        }),
    )
    .unwrap();
    with_config(&config, || agents::try_again(&json!({})).unwrap());
    assert!(agents::held_mark(&holds, "codex", now).is_some());
    assert!(agents::held_mark(&holds, "claude", now - 2).is_none());
    assert!(agents::held_mark(&holds, "signed", now).is_none());
    assert!(agents::held_mark(&holds, "model:x", now).is_none());
    with_config(&config, || agents::try_again(&json!({"agent":"codex"})).unwrap());
    assert!(agents::held_mark(&holds, "codex", now).is_none());
}

#[test]
fn read_pipe_reads_the_whole_output() {
    let text = agents::read_pipe(std::io::Cursor::new(b"logged in".to_vec())).join().unwrap();
    assert_eq!(text, "logged in");
}

#[test]
fn codex_folder_uses_the_process_environment() {
    let folder = agents::codex_folder();
    match std::env::var_os("CODEX_HOME") {
        Some(home) => assert_eq!(folder, std::path::PathBuf::from(home)),
        None => assert!(folder.ends_with(".codex")),
    }
}

#[test]
fn clear_retryable_keeps_a_usage_limit_that_has_not_reset() {
    let dir = testing::Scratch::new();
    let holds = dir.join("agent-holds.json");
    let now = now();
    agents::save_marks(
        &holds,
        &json!({
            "codex": {"reason":"Codex usage limit reached","until":now + 3600,"time":now},
            "claude": {"reason":"Claude Code usage limit reached","until":now - 1,"time":now},
            "signed": {"reason":"sign-in expired","until":null,"time":now,"kind":"sign_in"},
        }),
    )
    .unwrap();
    agents::clear_retryable(&holds).unwrap();
    assert!(agents::held_mark(&holds, "codex", now).is_some());
    assert!(agents::held_mark(&holds, "claude", now - 2).is_none());
    assert!(agents::held_mark(&holds, "signed", now).is_none());
}

#[test]
fn a_successful_command_code_run_keeps_its_model_available() {
    let dir = testing::Scratch::new();
    let (path, log) = (dir.join("agent-holds.json"), dir.join("log.jsonl"));
    let cmd = json!({
        "id": "commandcode-deepseek-v4.1-flash",
        "agent": "codex",
        "model": "commandcode-deepseek-v4.1-flash",
        "endpoint": {"base_url": "http://commandcode"}
    });
    let warning = json!({
        "type": "item.completed",
        "item": {
            "type": "error",
            "message": "Model metadata for `commandcode-deepseek-v4.1-flash` not found. Defaulting to fallback metadata."
        }
    });
    fs::write(&log, warning.to_string()).unwrap();
    agents::observe_refusal(&path, &cmd, &log, NOON, Some("finished")).unwrap();
    assert_eq!(agents::held(&path, "model:commandcode-deepseek-v4.1-flash", NOON), None);
    agents::write_mark(&path, "model:commandcode-deepseek-v4.1-flash", "model not found", None, Some("refusal")).unwrap();
    assert_eq!(agents::held(&path, "model:commandcode-deepseek-v4.1-flash", NOON).as_deref(), Some("model not found"));
    agents::observe_refusal(&path, &cmd, &log, NOON + 3650 * 86400, Some("finished")).unwrap();
    assert_eq!(agents::held(&path, "model:commandcode-deepseek-v4.1-flash", NOON), None);
    agents::write_mark(&path, "model:commandcode-deepseek-v4.1-flash", "model not found", None, Some("refusal")).unwrap();
    let answered = json!({"type": "item.completed", "item": {"type": "agent_message", "text": "Done."}});
    fs::write(&log, format!("{warning}\n{answered}")).unwrap();
    agents::observe(&path, &cmd, &log, NOON, NOON + 3650 * 86400, Some("finished"), &zones).unwrap();
    assert_eq!(agents::held(&path, "model:commandcode-deepseek-v4.1-flash", NOON), None);
    fs::write(&log, json!({"type":"error","message":"model not found"}).to_string()).unwrap();
    agents::observe(&path, &cmd, &log, NOON, NOON + 3650 * 86400, Some("stopped"), &zones).unwrap();
    assert_eq!(agents::held(&path, "model:commandcode-deepseek-v4.1-flash", NOON).as_deref(), Some("model not found"));
}

#[test]
fn a_review_that_ended_before_recovery_keeps_its_checkpoint() {
    let assessment = "**Assessment: Looks good**\n\nFine.";
    let (p, tid, _child, _log) = exited(json!({"id":"r1","kind":"review","state":"ended"}), 0, &[agent_says(assessment)]);
    let checkpoint = review_path(&p.root, &tid, "r1").unwrap();
    fs::create_dir_all(checkpoint.parent().unwrap()).unwrap();
    fs::write(&checkpoint, br#"{"run":"r1","changed_during_review":false}"#).unwrap();
    let saved = fs::read(&checkpoint).unwrap();
    let before = task(&p.root, &tid).unwrap()["notes"].as_array().unwrap().len();
    assert!(!record_review(&Watch::Task(&p.root, &tid), "r1", assessment).unwrap());
    assert_eq!(fs::read(&checkpoint).unwrap(), saved);
    assert_eq!(task(&p.root, &tid).unwrap()["notes"].as_array().unwrap().len(), before);
}

#[test]
fn a_work_run_is_found_from_a_secondary_project() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"sub\"]\n"),
        (".solace/store.db", ""),
        ("sub/src/app.gleam", "pub fn main() { 1 }\n"),
    ]);
    let s = open(&root).unwrap();
    let mut t = api(&root, "POST", "/tasks", json!({"title":"Work"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["home"] = json!(".");
    t["scope"] = json!([".", "sub"]);
    t["runs"] = json!([
        {"id":"r1","kind":"review","state":"running"},
        {"id":"r2","kind":"work","state":"running"}
    ]);
    tasks::put(&s, &t).unwrap();
    let sub = root.join("sub");
    assert!(!with_context(&tid, "r1", || work_run(&sub)));
    assert_eq!(
        with_context(&tid, "r2", || crate::next_step(None, work_run(&sub))),
        "Next: if every part of the task is changed, finish with task done.\n"
    );
}

#[test]
fn a_single_reply_with_the_batch_marker_is_recorded() {
    assert!(note_recorded("Recorded.\n==== solace read a ====\n"));
    assert!(note_recorded("Recorded; ==== solace task note ==== was in the text\n"));
    assert!(!note_recorded("==== solace inspect a ====\nRecorded.\n"));
}

/// D-196: the stream reads the feature state after a change commits, so a
/// Backlog task planned by its feature becoming active arrives shown.
#[test]
fn a_streamed_task_carries_on_board() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let root = p.root.clone();
    open(&root).unwrap();
    let feature = tasks::strategy_api(&root, "POST", &["features"], json!({"title": "Waiting", "state": "proposed"})).unwrap();
    let fid = feature["id"].as_str().unwrap().to_string();
    let mut t = new_task("Shown", "", &json!("backlog"), 1.);
    t["home"] = json!(".");
    t["scope"] = json!(["."]);
    let tid = t["id"].as_str().unwrap().to_string();
    fs::write(store_dir(&root).join(format!("{tid}.json")), t.to_string()).unwrap();
    let before: Option<i64> = tasks::events(&root).unwrap().query_row("SELECT MAX(seq) FROM events", [], |r| r.get(0)).unwrap();
    api(&root, "PATCH", &format!("/tasks/{tid}"), json!({"feature": fid})).unwrap();
    let top = scope::top(&root);
    let last = json!({"tasks": before.unwrap_or(0), ".": 0}).to_string();
    let mut reader = Events::new(event_board(top), Some(root.clone()), Some(&last));
    let next_task = |reader: &mut Events| {
        let mut buf = [0; 8192];
        loop {
            let n = reader.read(&mut buf).unwrap();
            assert!(n > 0);
            let text = std::str::from_utf8(&buf[..n]).unwrap();
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    let v: Value = serde_json::from_str(data).unwrap();
                    if v.get("task").is_some() {
                        return v;
                    }
                }
            }
        }
    };
    assert_eq!(next_task(&mut reader)["task"]["onBoard"], json!(false));
    tasks::strategy_api(&root, "PATCH", &["features", &fid], json!({"state": "active"})).unwrap();
    let planned = next_task(&mut reader);
    assert_eq!(planned["task"]["column"], json!("planning"));
    assert_eq!(planned["task"]["onBoard"], json!(true));
    tasks::strategy_api(&root, "PATCH", &["features", &fid], json!({"state": "proposed"})).unwrap();
    assert_eq!(next_task(&mut reader)["task"]["onBoard"], json!(false));
}

/// D-112: a task with no feature files its last review's tech debt on a new
/// Tech Debt task of its own, and a review with no section files nothing.
#[test]
fn a_task_with_no_feature_files_its_own_tech_debt() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Loose"})).unwrap();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"rv1","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut t, "agent", Some("rv1"), "### Tech debt\n- `app.main`: name the constant\n- `app.other`: split it\n");
    let (debt, n) = tech_debt(&top, &t).unwrap().unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        (debt["title"].as_str(), debt["column"].as_str(), debt["feature"].as_str(), debt["milestone"].as_str(), debt["techDebt"].as_bool()),
        (Some("Loose: Tech Debt"), Some("planning"), None, None, Some(true))
    );
    assert_eq!((&debt["scope"], &debt["home"]), (&t["scope"], &t["home"]));
    let notes: Vec<&str> = debt["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(notes.len(), 2);
    assert!(notes[0].contains(&format!("task:{}", t["id"].as_str().unwrap())) && notes[0].contains("rv1"));
    assert!(notes[0].ends_with("`app.main`: name the constant"));
    assert!(notes[1].ends_with("`app.other`: split it"));
    // A review with no section files nothing.
    t["notes"].as_array_mut().unwrap().clear();
    note(&mut t, "agent", Some("rv1"), "**Assessment: Looks good**\n- fine\n");
    assert!(tech_debt(&top, &t).unwrap().is_none());
}

/// A failed batch names the command that failed, and a failure without an
/// error code falls back to its first message line.
#[test]
fn failed_tool_lines_name_the_code_and_failed_command() {
    let batch = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) inspect a\nsubmit b",
        "output": "==== solace inspect a ====\nfn a() {}\n==== solace submit b ====\nE0108 no b\n1 of 2 commands failed: submit b",
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(2, &batch), "3 submit b: E0108 no b");
    let plain = json!({
        "kind": "tool",
        "title": "solace view b",
        "output": "MCP error: transport closed",
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(0, &plain), "1 view b: MCP error: transport closed");
    let no_code = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) inspect a\ncheck",
        "output": concat!(
            "==== solace inspect a ====\nfn a() {}\n",
            "==== solace check ====\nerror[E0308]: mismatched types\n1 of 2 commands failed: check"
        ),
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(0, &no_code), "1 check: error[E0308]: mismatched types");
    let earlier_code = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) read a\ncheck",
        "output": concat!(
            "==== solace read a ====\nE1234 not this failure\n",
            "==== solace check ====\ncargo error\n1 of 2 commands failed: check"
        ),
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(1, &earlier_code), "2 check: cargo error");
    assert_eq!(super::error_line("==== solace a ====\nE6002 panic\n"), Some("E6002 panic"));
    assert_eq!(super::error_line("==== solace a ====\nboom\n"), Some("boom"));
    assert!(super::error_code("E0108 x") && !super::error_code("E108 x") && !super::error_code("error"));
}

/// A batch that runs a command twice and fails the second copy reports that
/// copy's error, not the first copy's output.
#[test]
fn a_repeated_command_reports_the_copy_that_failed() {
    let mut seen = 0;
    let (output, failed) = crate::batch(&["check", "check"], 10_000, &mut |_, _| {
        seen += 1;
        if seen == 2 { Err(crate::Failure("E0308 mismatched types".into(), 1)) } else { Ok("no problems\n".into()) }
    });
    assert!(failed);
    assert!(output.ends_with("1 of 2 commands failed: check (copy 2)\n"), "{output}");
    let step = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) check\ncheck",
        "output": output,
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(0, &step), "1 check: E0308 mismatched types");
    let old = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) check\ncheck",
        "output": concat!(
            "==== solace check ====\nfirst copy\n",
            "==== solace check ====\nsecond copy\n",
            "1 of 2 commands failed: check"
        ),
        "status": "failed",
    });
    assert_eq!(super::failed_step_line(0, &old), "1 check: first copy");
}

fn settle_failures(t: &Value) -> usize {
    t["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["text"].as_str().is_some_and(|s| s.starts_with(crate::tasks::SETTLE_FAILED_NOTE)))
        .count()
}

/// A root whose only project is `sub`, with a task homed and scoped there and
/// one undo record in its store.
fn unsettled_member_final_task() -> (testing::Scratch, Store, String) {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"sub\"]\n"),
        ("sub/gleam.toml", "name = \"sub\"\nversion = \"1.0.0\"\n"),
        ("sub/src/app.gleam", "pub fn main() { 1 }\n"),
    ]);
    crate::import(&root.join("sub")).unwrap();
    let s = open(&root.join("sub")).unwrap();
    let mut t = new_task("Member", "", &json!("complete"), 1.);
    t["home"] = json!("sub");
    t["scope"] = json!(["sub"]);
    let tid = t["id"].as_str().unwrap().to_string();
    tasks::put(&s, &t).unwrap();
    s.db.execute("INSERT INTO board_before VALUES (?1,'symbol:x','null')", [&tid]).unwrap();
    (root, s, tid)
}

#[test]
fn a_final_task_of_a_member_project_settles() {
    let (root, s, tid) = unsettled_member_final_task();
    let has_before =
        || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    assert!(has_before());
    tasks::settle_later(&root, tid.clone());
    let t = task(&root.join("sub"), &tid).unwrap();
    assert!(t["settleFailed"].is_null(), "{t}");
    assert!(t["attention"].is_null(), "{t}");
    assert_eq!(settle_failures(&t), 0);
    assert!(!has_before(), "the member project's undo record is gone");
}

#[test]
fn a_member_projects_settle_failure_is_flagged_and_cleared() {
    let (root, s, tid) = unsettled_member_final_task();
    let before = root.join("sub/.solace/board").join(&tid).join("before");
    fs::create_dir_all(before.parent().unwrap()).unwrap();
    fs::write(&before, "").unwrap();
    tasks::settle_later(&root, tid.clone());
    let t = task(&root.join("sub"), &tid).unwrap();
    assert_eq!(t["settleFailed"], true);
    assert_eq!(t["attention"]["reason"], crate::tasks::SETTLE_FAILED_REASON);
    assert_eq!(settle_failures(&t), 1);
    tasks::settle_later(&root, tid.clone());
    assert_eq!(settle_failures(&task(&root.join("sub"), &tid).unwrap()), 1, "a repeated failure adds no note");
    fs::remove_file(&before).unwrap();
    resettle(&root);
    let t = task(&root.join("sub"), &tid).unwrap();
    assert!(t["settleFailed"].is_null(), "{t}");
    assert!(t["attention"].is_null(), "{t}");
    assert_eq!(settle_failures(&t), 0);
    let left: bool =
        s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
    assert!(!left);
}
