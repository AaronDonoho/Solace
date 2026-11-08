use super::*;

use serde_json::Value;
use serde_json::json;
use std::io::Read;
use crate::cli::{batch, next_step, run_options, with_root};
use crate::tasks::server::SETTLE_FAILED_REASON;
use crate::tasks::server::SETTLE_FAILED_NOTE;
use crate::board::server::Board;
use crate::board::server::Events;
use crate::board::task_model::task;
use crate::board::task_model::all_tasks;
use crate::board::task_model::adopt_tasks;
use crate::board::task_model::flow;
use crate::board::task_model::allowed;
use crate::board::task_model::note;
use crate::board::task_model::end_runs;
use crate::board::task_model::new_task;
use crate::board::task_model::change;
use crate::board::runs::start_project;
use crate::board::runs::run_project;
use crate::board::runs::GRACEFUL_EXIT;
use crate::board::runs::supervise;
use crate::board::runs::Watch;
use crate::board::runs::transcript;
use crate::board::reviews::snapshot;
use crate::board::reviews::scope_stores;
use crate::board::reviews::record;
use crate::board::reviews::existing_folders;
use crate::board::reviews::changes_of;
use crate::board::reviews::review_path;
use crate::board::reviews::moved_to;
use crate::board::reviews::restore;
use crate::board::reviews::revert;
use crate::board::reviews::settle;
use crate::board::reviews::tech_debt;
use crate::board::reviews::settle_final;
use crate::board::reviews::resettle;
use crate::board::reviews::drop_before;
use crate::board::reviews::settle_result;
use crate::board::reviews::record_review;
use crate::board::briefs::continuation_brief;
use crate::board::task_model::needs_lock;
use crate::board::server::api;
use crate::board::runs::work_run;
use crate::board::runs::NO_TOOL;
use crate::board::runs::NO_TOOL_MESSAGE;
use crate::board::task_model::After;
use crate::board::task_model::lock;
use crate::board::task_model::open;
use crate::board::task_model::store_dir;
use crate::board::runs::agent_guard;
use crate::board::runs::with_context;
use crate::board::runs::task_command;
use crate::board::settings::apply_settings;
use crate::board::setup::with_config;
use crate::board::runs::failed_step_line;

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
    assert!(guard("r1", &["view", "app"]).is_ok() && guard("r1", &["build"]).is_ok() && guard("r1", &["probe"]).is_ok());
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
    for rid in ["r1", "r3", "r4"] {
        assert!(guard(rid, &["drafts", "drop", "d1"]).unwrap_err().contains("this run is read-only"), "{rid}");
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
    let brief = strategy_brief(
        &top,
        &json!({"context":{"kind":"feature","id":fid},"notes":[{"author":"human","text":"Plan F"}]}),
        "engineer",
        "standard",
        "balanced",
    );
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
    let brief = strategy_brief(&top, &json!({"context":{"kind":"page"},"notes":[]}), "engineer", "standard", "balanced");
    assert!(brief.contains("- D-5 Thing 5") && brief.contains("- D-14 Thing 14") && !brief.contains("- D-4 Thing"), "{brief}");
    assert!(brief.contains("… 4 earlier decisions: `grep D-` lists them."), "{brief}");
}

/// D-27: a strategy brief for a folder of projects carries the workspace's
/// skills for its role in full, and leaves another role's out.
#[test]
fn a_strategy_brief_leaves_out_a_folder_of_projects_skills() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\"]\n"),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn main() { 1 }\n"),
        (
            "skills/design/SKILL.md",
            "---\nname: design\ndescription: Layout and color.\nroles: strategist\n---\n# Design\n\nPick a palette.\n",
        ),
        ("skills/review.md", "---\nname: review\ndescription: Review UI.\nroles: reviewer\n---\nReview every screen.\n"),
        ("skills/general.md", "General rules\n"),
    ]);
    let brief = strategy_brief(&root, &json!({"context":{"kind":"page"},"notes":[]}), "engineer", "standard", "balanced");
    assert!(brief.contains("folder of projects"), "{brief}");
    assert!(brief.contains("### design") && brief.contains("Pick a palette."), "{brief}");
    assert!(!brief.contains("### review") && !brief.contains("Review every screen."), "{brief}");
    assert!(!brief.contains("- general:") && !brief.contains("General rules"), "{brief}");
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
    crate::store::store_rust_file(&s.db, "x", "src", "src/other.rs", &split).unwrap();
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
    crate::store::store_rust_file(&s.db, "x", "src", "src/other.rs", &split).unwrap();
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
    let (debt, work, review) = tech_debt(&top, &t).unwrap().unwrap();
    assert_eq!((debt["column"].as_str(), work, review), (Some("todo"), 0, 1));
    let mut loose = api(&s.root, "POST", "/tasks", json!({"title":"Loose"})).unwrap();
    loose["runs"].as_array_mut().unwrap().push(json!({"id":"rv2","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut loose, "agent", Some("rv2"), "### Tech debt\n- `app.other`: split it\n");
    let (own, work, review) = tech_debt(&top, &loose).unwrap().unwrap();
    assert_eq!((own["column"].as_str(), work, review), (Some("todo"), 0, 1));
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
        let e = strategy_note_show(&root, "t1", bad).unwrap_err().0;
        assert!(e.starts_with("E7001") && e.contains("notes") && e.contains("1..2") && !e.contains("steps"), "{bad}: {e}");
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
        let e = strategy_run_show(&root, "t1", "r1", Some(bad)).unwrap_err().0;
        assert!(e.starts_with("E7001") && e.contains("steps") && !e.contains("notes"), "{bad}: {e}");
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
    let _server = fsx::try_lock_file(&store_dir(&s.root).join("server.lock")).unwrap().unwrap();
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
    fs::write(
        root.join("solace.toml"),
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/default-board.toml")).replace("= \"complete\"", "= \"done\""),
    )
    .unwrap();
    assert_eq!(tasks(&root).unwrap()[0]["column"], "done");
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
    let debt = |notes: Value| {
        json!({
            "title": "Tech Debt",
            "body": crate::board::reviews::TECH_DEBT_BODY,
            "techDebt": true,
            "scope": ["a", "b"],
            "notes": notes,
        })
    };
    let found = json!([{"author": "solace", "text": concat!(
        "Tech debt from the work on task:t (work run w):\n\n",
        "`plan.answer` is stale."
    )}]);
    assert_eq!(start_project(&a, &debt(found)), "b", "a Tech Debt task starts where its findings point");
    assert_eq!(
        start_project(&a, &debt(json!([{"author": "agent", "text": "`plan.answer` is fine."}]))),
        "a",
        "a Tech Debt task's other notes are not findings"
    );
    assert_eq!(start_project(&a, &t("Change `plan.answer`.")), "b", "an ordinary task ranks as before");
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
    let out = with_root(&root, || {
        with_context(&tid, "format", || {
            run_options(&["format".to_string(), "crate".to_string()], &mut || Ok(String::new()), None, 1000, &[])
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

/// A checkpoint error that is not a busy log keeps its own message and code.
#[test]
fn settling_keeps_a_non_busy_checkpoint_error() {
    let e = settle_result(vec![], vec![("b".into(), Failure("E1234 disk full".into(), 7))]).unwrap_err();
    assert_eq!(e.0, "b: E1234 disk full");
    assert!(!e.0.contains("E7028"), "{}", e.0);
    assert_eq!(e.1, 7);
    let e = settle_result(vec!["a".into()], vec![("b".into(), Failure("E1234 disk full".into(), 7))]).unwrap_err();
    assert!(e.0.starts_with("b: E1234 disk full"), "{}", e.0);
    assert!(e.0.contains("Also, flushing store.db is pending in a"), "{}", e.0);
    assert!(!e.0.contains("E7028"), "{}", e.0);
    assert_eq!(e.1, 7);
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

/// When entries share the model string a run used, the later current entry
/// supplies the label stored with the run.
#[test]
fn a_later_entry_names_the_shared_model() {
    let models = json!([
        {"id":"first","label":"First","agent":"codex","model":"claude-opus-5-5"},
        {"id":"second","label":"Second","agent":"codex","model":"claude-opus-5-5"}
    ]);
    assert_eq!(agents::used_label(models.as_array().unwrap(), "claude-opus-5-5"), "Second");
    assert_eq!(agents::used_label(models.as_array().unwrap(), "not-listed"), "not-listed", "no entry leaves the model id");
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
    let e = with_config(&bad.join("system.toml"), || revert(&s, &mut t)).unwrap_err();
    assert!(e.0.contains("E6020"), "{}", e.0);
    assert!(pending.join("extract-x.json").exists(), "recovery ran before the settings check");
    assert!(pending.join("undo.json").exists(), "recovery ran before the settings check");
    assert_eq!(fs::read_to_string(s.root.join("priv/note")).unwrap(), "original");
}

/// A model, language and behavior save share one lock, so no update is lost.
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
        let (config, keys, done) = (config.clone(), keys.clone(), done_tx.clone());
        std::thread::spawn(move || {
            let body = json!({"label": "Local", "agent": "codex", "model": "gpt-5", "base_url": "http://localhost:1"});
            done.send(setup::save(&config, &keys, &body).map(|_| ())).unwrap();
        })
    };
    let behavior = {
        let (config, done) = (config.clone(), done_tx);
        std::thread::spawn(move || {
            done.send(setup::save_agent_setting(&config, &json!({"key": "involvement", "value": "engineer"})).map(|_| ())).unwrap();
        })
    };
    let lock = config.with_extension("lock");
    let mut waiting = 0;
    while waiting < 3 {
        waiting += usize::from(attempt_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() == lock);
    }
    assert!(done_rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "a settings write ran while the lock was held");
    *setup::LOCK_ATTEMPT.lock().unwrap() = None;
    drop(held);
    for _ in 0..3 {
        done_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap().unwrap();
    }
    language.join().unwrap();
    model.join().unwrap();
    behavior.join().unwrap();
    let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(v["languages"]["rust"].as_bool(), Some(false));
    assert!(v["models"].as_array().unwrap().iter().any(|m| m["id"].as_str() == Some("local")));
    assert_eq!(v["involvement"].as_str(), Some("engineer"));
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
    let e = settle_result(
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

pub(crate) fn launched(run: Value, child: std::process::Child) -> (testing::Project, String, std::process::Child) {
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
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
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
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
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
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
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
    let (p, tid, mut child, log) =
        exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[agent_tool("i1", "solace", "task diff"), agent_says("Done.")]);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
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
    assert_eq!(with_context(&tid, "r1", || next_step(Some("app.one"), work_run(&root))), "");
    assert_eq!(
        with_context(&tid, "r2", || next_step(None, work_run(&root))),
        "Next: if every part of the task is changed, finish with task done.\n"
    );
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
fn a_sign_in_failure_is_a_plain_message() {
    let said = "Failed to authenticate: OAuth session expired and could not be refreshed";
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 1, &[json!({"type":"error","message":said})]);
    let m = json!({"agent":"claude","model":"m"});
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &m, false, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap()
    );
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
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &m, false, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["attention"]["reason"], "Claude Code needs an update. Update Claude Code, then start the task again.");
    assert_eq!(t["runs"][0]["outcome"], "Claude Code needs an update. Update Claude Code, then start the task again.");
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
    assert_eq!(t["attention"]["reason"], SETTLE_FAILED_REASON);
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
        with_context(&tid, "r2", || next_step(None, work_run(&sub))),
        "Next: if every part of the task is changed, finish with task done.\n"
    );
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
    let (debt, work, review) = tech_debt(&top, &t).unwrap().unwrap();
    assert_eq!((work, review), (0, 2));
    assert_eq!(
        (debt["title"].as_str(), debt["column"].as_str(), debt["feature"].as_str(), debt["milestone"].as_str(), debt["techDebt"].as_bool()),
        (Some("Loose"), Some("planning"), None, None, Some(true))
    );
    assert_eq!((&debt["scope"], &debt["home"]), (&t["scope"], &t["home"]));
    let notes: Vec<&str> = debt["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(notes.len(), 2);
    assert!(notes[0].contains(&format!("task:{}", t["id"].as_str().unwrap())) && notes[0].contains("rv1"));
    assert!(notes[0].ends_with("`app.main`: name the constant"));
    assert!(notes[1].ends_with("`app.other`: split it"));
    // A source task saved before scopes has no list: its home is the filed scope.
    let mut old = t.clone();
    old.as_object_mut().unwrap().remove("scope");
    old["notes"].as_array_mut().unwrap().clear();
    note(&mut old, "agent", Some("rv1"), "### Tech debt\n- `app.main`: name the constant\n");
    let (debt, _, _) = tech_debt(&top, &old).unwrap().unwrap();
    assert_eq!(debt["scope"], json!([old["home"].clone()]));
    // An empty list does the same.
    old["scope"] = json!([]);
    let (debt, _, _) = tech_debt(&top, &old).unwrap().unwrap();
    assert_eq!(debt["scope"], json!([old["home"].clone()]));
    // A review with no section files nothing.
    t["notes"].as_array_mut().unwrap().clear();
    note(&mut t, "agent", Some("rv1"), "**Assessment: Looks good**\n- fine\n");
    assert!(tech_debt(&top, &t).unwrap().is_none());
}

/// A batch that runs a command twice and fails the second copy reports that
/// copy's error, not the first copy's output.
#[test]
fn a_repeated_command_reports_the_copy_that_failed() {
    let mut seen = 0;
    let (output, failed) = batch(&["check", "check"], 10_000, &mut |_, _| {
        seen += 1;
        if seen == 2 { Err(Failure("E0308 mismatched types".into(), 1)) } else { Ok("no problems\n".into()) }
    });
    assert!(failed);
    assert!(output.ends_with("1 of 2 commands failed: check (copy 2)\n"), "{output}");
    let step = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) check\ncheck",
        "output": output,
        "status": "failed",
    });
    assert_eq!(failed_step_line(0, &step), "1 check: E0308 mismatched types");
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
    assert_eq!(failed_step_line(0, &old), "1 check: first copy");
}

fn settle_failures(t: &Value) -> usize {
    t["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["text"].as_str().is_some_and(|s| s.starts_with(SETTLE_FAILED_NOTE)))
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
    assert_eq!(t["attention"]["reason"], SETTLE_FAILED_REASON);
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

/// Two projects on one board, with a work run in the first: a draft can be
/// open in either.
fn draft_board() -> (testing::Scratch, Store, Store, String) {
    let settings = "projects = [\"web\", \"components\"]\n";
    let root = testing::Scratch::new().with(&[
        ("solace.toml", settings),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() { 1 }\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        ("components/src/lib.gleam", "pub fn z() -> Int {\n  0\n}\n"),
    ]);
    for d in ["web", "components"] {
        crate::import(&root.join(d)).unwrap();
    }
    let web = open(&root.join("web")).unwrap();
    let components = open(&root.join("components")).unwrap();
    let mut t = api(&web.root, "POST", "/tasks", json!({"title": "Drafts"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["scope"] = json!(["web", "components"]);
    t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
    tasks::put(&web, &t).unwrap();
    (root, web, components, tid)
}

#[test]
fn a_draft_in_the_home_project_blocks_done() {
    let (_root, web, _components, tid) = draft_board();
    web.db
        .execute(
            "INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)",
            params![tid, json!(["submit", "core.respond"]).to_string(), "pub fn respond() {}", ""],
        )
        .unwrap();
    let done = |s: &Store| task_command(s, &tid, "r1", &["task", "done"], &mut || Ok(String::new()), &[], vec![]);
    let e = done(&web).unwrap_err().0;
    for part in ["task done is refused", "d1", "submit core.respond", "view --draft d1", "submit --draft d1", "drafts drop d1"] {
        assert!(e.contains(part), "{part} missing from:\n{e}");
    }
}

#[test]
fn a_draft_in_another_project_blocks_done_until_dropped() {
    let (_root, web, components, tid) = draft_board();
    let patch = "*** Update File: src/lib.gleam\n@@\n-pub fn z() -> Int {\n-  0\n-}\n+pub fn z() -> Int {\n+  1\n+}\n*** End Patch\n";
    components
        .db
        .execute("INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)", params![tid, json!(["write"]).to_string(), patch, ""])
        .unwrap();
    let done = |s: &Store| task_command(s, &tid, "r1", &["task", "done"], &mut || Ok(String::new()), &[], vec![]);
    let e = done(&web).unwrap_err().0;
    for part in ["d1", "--project components", "write --draft d1", "drafts drop d1"] {
        assert!(e.contains(part), "{part} missing from:\n{e}");
    }
    let dropped = run_draft_command(&web, &tid, &["drafts", "drop", "d1", "--project", "components"], "").unwrap();
    assert!(dropped.contains("Dropped draft d1"), "{dropped}");
    let out = done(&web).unwrap();
    assert!(out.contains(tasks::QUEUED), "{out}");
}

#[test]
fn a_resumed_brief_names_drafts_in_every_project() {
    let (_root, web, components, tid) = draft_board();
    components
        .db
        .execute(
            "INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)",
            params![tid, json!(["submit", "lib.z"]).to_string(), "pub fn z() {}", ""],
        )
        .unwrap();
    let t = task(&web.root, &tid).unwrap();
    let run = json!({"id": "r1", "kind": "work", "state": "running"});
    let brief = continuation_brief(&t, &run, true, &web.root, &web.root, false, "engineer", "standard", "balanced").0;
    assert!(brief.contains("Retained drafts") && brief.contains("view --draft d1 --project components"), "{brief}");
}

#[test]
fn a_refused_submit_in_another_project_names_the_project_of_its_draft() {
    let (_root, web, components, tid) = draft_board();
    let run = |args: &[&str], text: &str| {
        with_root(&web.root, || {
            with_context(&tid, "r1", || {
                run_options(
                    &args.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
                    &mut || Ok(text.to_string()),
                    None,
                    10_000,
                    &[],
                )
            })
        })
    };
    let e = run(&["submit", "lib.z", "--project", "components"], "pub fn z() -> Int {").unwrap_err().0;
    let id = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    let kept = crate::drafts::entries(&components, &tid).unwrap();
    assert!(kept.iter().any(|(draft, ..)| *draft == id), "not kept in components: {kept:?}");
    let home_kept = crate::drafts::entries(&web, &tid).unwrap();
    assert!(!home_kept.iter().any(|(draft, ..)| *draft == id), "kept in the wrong project: {home_kept:?}");
    for part in [
        format!("`view --draft {id} --project components`"),
        format!("`submit --draft {id} --replace --project components`"),
        "`drafts --project components`".to_string(),
        format!("`drafts drop {id} --project components`"),
    ] {
        assert!(e.contains(&part), "{part} missing from:\n{e}");
    }
    let listed = run(&["drafts", "--project", "components"], "").unwrap();
    assert!(listed.contains(&id), "{listed}");
    let viewed = run(&["view", "--draft", id.as_str(), "--project", "components"], "").unwrap();
    assert!(viewed.contains("pub fn z() -> Int {"), "{viewed}");
    let dropped = run(&["drafts", "drop", &id, "--project", "components"], "").unwrap();
    assert!(dropped.contains(&format!("Dropped draft {id}")), "{dropped}");
    let home = run(&["submit", "app.main"], "pub fn main(").unwrap_err().0;
    assert!(!home.contains("--project"), "{home}");
}

fn run_draft_command(web: &Store, tid: &str, args: &[&str], text: &str) -> Out {
    with_root(&web.root, || {
        with_context(tid, "r1", || {
            run_options(
                &args.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
                &mut || Ok(text.to_string()),
                None,
                10_000,
                &[],
            )
        })
    })
}

/// A refusal from another project names that project in the draft command it
/// prints: a stale submit's `view`, and a file patch's `write`.
#[test]
fn a_stale_or_file_patch_draft_in_another_project_names_that_project() {
    let (_root, web, components, tid) = draft_board();
    let e = run_draft_command(&web, &tid, &["submit", "lib.z", "--project", "components"], "pub fn z() -> Int {").unwrap_err().0;
    let stale = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    assert_draft_in_components_only(&components, &web, &tid, &stale);
    run_draft_command(&web, &tid, &["submit", "lib.z", "--project", "components"], "pub fn z() -> Int {\n  5\n}\n").unwrap();
    let e = run_draft_command(&web, &tid, &["submit", "--draft", &stale, "--project", "components"], "").unwrap_err().0;
    assert!(e.contains(&format!("`view --draft {stale} --project components`")), "{e}");
    let patch = "*** Begin Patch\n*** Update File: src/lib.gleam\n-pub fn z\n+pub fn y\n*** End Patch\n";
    let e = run_draft_command(&web, &tid, &["write", "--project", "components"], patch).unwrap_err().0;
    let file = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    assert_draft_in_components_only(&components, &web, &tid, &file);
    let e = run_draft_command(&web, &tid, &["submit", "--draft", &file, "--project", "components"], "").unwrap_err().0;
    assert!(e.contains(&format!("`write --draft {file} --project components`")), "{e}");
}

/// A refused file patch from another project names that project in the calls
/// that retry and drop its draft.
#[test]
fn a_refused_file_patch_in_another_project_names_that_project() {
    let (_root, web, components, tid) = draft_board();
    let patch = "*** Begin Patch\n*** Update File: src/lib.gleam\n-pub fn z\n+pub fn y\n*** End Patch\n";
    let e = run_draft_command(&web, &tid, &["write", "--project", "components"], patch).unwrap_err().0;
    let id = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    assert_draft_in_components_only(&components, &web, &tid, &id);
    for part in [format!("`write --draft {id} --replace --project components`"), format!("`drafts drop {id} --project components`")] {
        assert!(e.contains(&part), "{part} missing from:\n{e}");
    }
    let dropped = run_draft_command(&web, &tid, &["drafts", "drop", &id, "--project", "components"], "").unwrap();
    assert!(dropped.contains(&format!("Dropped draft {id}")), "{dropped}");
}

/// A submit that names both a draft and imports prints both retry commands
/// with the project, so a cross-project retry runs in the right store.
#[test]
fn a_submit_draft_with_imports_names_the_project_in_both_commands() {
    let (_root, web, components, tid) = draft_board();
    let e = run_draft_command(&web, &tid, &["submit", "lib.z", "--project", "components"], "pub fn z() -> Int {").unwrap_err().0;
    let id = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    let e = run_draft_command(&web, &tid, &["submit", "--draft", &id, "--imports", "lib", "--project", "components"], "").unwrap_err().0;
    assert!(
        e.contains(&format!("`submit --imports lib --project components` first, then `submit --draft {id} --project components`")),
        "{e}"
    );
    assert_draft_in_components_only(&components, &web, &tid, &id);
}

/// A submit draft reached through `write --draft` names the project in the
/// `submit --draft` command that retries it.
#[test]
fn a_submit_draft_retried_as_a_file_patch_names_the_project() {
    let (_root, web, components, tid) = draft_board();
    let e = run_draft_command(&web, &tid, &["submit", "lib.z", "--project", "components"], "pub fn z() -> Int {").unwrap_err().0;
    let id = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
    let e = run_draft_command(&web, &tid, &["write", "--draft", &id, "--project", "components"], "").unwrap_err().0;
    assert_draft_in_components_only(&components, &web, &tid, &id);
    assert!(e.contains(&format!("`submit --draft {id} --project components`")), "{e}");
}

/// The draft is in the components store the cross-project command ran in, and not in the home store.
fn assert_draft_in_components_only(components: &Store, web: &Store, tid: &str, id: &str) {
    let kept = crate::drafts::entries(components, tid).unwrap();
    assert!(kept.iter().any(|(draft, ..)| draft == id), "not kept in components: {kept:?}");
    let home = crate::drafts::entries(web, tid).unwrap();
    assert!(!home.iter().any(|(draft, ..)| draft == id), "kept in the wrong project: {home:?}");
}

/// A graph error in another project names that project, so the agent can
/// retry the command there (D-123).
#[test]
fn a_cross_project_graph_hint_names_the_project() {
    let (_root, web, _components, tid) = draft_board();
    let e = run_draft_command(&web, &tid, &["read", "src/lib.gleam", "--project", "components"], "").unwrap_err().0;
    assert!(e.contains("view lib --header --project components") && e.contains("view lib --bodies --project components"), "{e}");
}

/// A move into another project marks its destination before changing it, so a
/// crash before the commit can put the destination's files back (D-84).
#[test]
fn a_move_into_another_project_marks_its_destination() {
    let (_root, web, components, tid) = draft_board();
    std::fs::write(web.root.join("notes.txt"), "one\n").unwrap();
    crate::command_tx::crash_at(Some("command"));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_draft_command(&web, &tid, &["move", "notes.txt", "notes.txt", "--to-project", "components"], "")
    }))
    .is_err();
    crate::command_tx::crash_at(None);
    assert!(panicked, "the move did not reach its crash point");
    let mark = components.root.join(".solace/pending/command-r1.json");
    assert!(mark.exists(), "no destination marker at {}", mark.display());
}

/// A scan that panics leaves the lock poisoned; a migration still takes it.
#[test]
fn a_poisoned_watching_lock_is_still_taken() {
    let board = event_board(std::env::temp_dir());
    let scan = board.clone();
    let _ = std::thread::spawn(move || {
        let _guard = scan.watching_guard();
        panic!("a scan failed");
    })
    .join();
    let _guard = board.watching_guard();
}

/// D-112: a completed task files the tech debt in its last work summary and its
/// last review, with a note naming each source.
#[test]
fn completing_a_task_files_its_work_and_review_tech_debt() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Waste"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"wk","kind":"work","state":"ended","outcome":"finished"}));
    note(
        &mut t,
        "agent",
        Some("wk"),
        "### Tech debt\n- `run cargo-fmt-check`: printed files the task never touched. Check only its files.\n",
    );
    t["runs"].as_array_mut().unwrap().push(json!({"id":"rv","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut t, "agent", Some("rv"), "### Tech debt\n- `app.main`: name the constant\n");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"plan"})).unwrap();
    let done = api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
    let debts: Vec<Value> = all_tasks(&top).unwrap().into_iter().filter(|t| t["techDebt"] == true).collect();
    let [debt] = debts.as_slice() else { panic!("{debts:?}") };
    let notes: Vec<&str> = debt["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].contains(&format!("Tech debt from the review of task:{tid} (review run rv):")), "{}", notes[0]);
    assert!(notes[0].ends_with("`app.main`: name the constant"));
    assert!(notes[1].contains(&format!("Tech debt from the work on task:{tid} (work run wk):")), "{}", notes[1]);
    assert!(notes[1].ends_with("`run cargo-fmt-check`: printed files the task never touched. Check only its files."));
    let last = done["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap();
    assert!(
        last.contains(&format!("Filed 2 tech-debt items from the last work run and review on task:{}", debt["id"].as_str().unwrap())),
        "{last}"
    );
    // A work summary alone files too.
    let mut only = api(&s.root, "POST", "/tasks", json!({"title":"Work only"})).unwrap();
    let only_id = only["id"].as_str().unwrap().to_string();
    only["runs"].as_array_mut().unwrap().push(json!({"id":"wk2","kind":"work","state":"ended","outcome":"finished"}));
    note(&mut only, "agent", Some("wk2"), "### Tech debt\n- `run tests`: the reply named only untouched files.\n");
    tasks::put(&s, &only).unwrap();
    api(&s.root, "POST", &format!("/tasks/{only_id}/actions"), json!({"action":"plan"})).unwrap();
    let work_done = api(&s.root, "POST", &format!("/tasks/{only_id}/actions"), json!({"action":"complete"})).unwrap();
    let fresh: Vec<Value> = all_tasks(&top).unwrap().into_iter().filter(|d| d["techDebt"] == true && d["id"] != debt["id"]).collect();
    let [work_debt] = fresh.as_slice() else { panic!("{fresh:?}") };
    let work_notes: Vec<&str> = work_debt["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(work_notes.len(), 1, "{work_notes:?}");
    assert!(work_notes[0].contains(&format!("Tech debt from the work on task:{only_id} (work run wk2):")), "{}", work_notes[0]);
    let work_last = work_done["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap();
    assert!(
        work_last.contains(&format!("Filed 1 tech-debt item from the last work run on task:{}", work_debt["id"].as_str().unwrap())),
        "{work_last}"
    );
}

/// A Codex log entry for a call to `tool` with `command`.
fn agent_tool(id: &str, tool: &str, command: &str) -> Value {
    json!({
        "type":"item.completed",
        "item":{
            "id":id,
            "type":"mcp_tool_call",
            "tool":tool,
            "arguments":{"command":command},
            "result":{"content":[{"type":"text","text":"ok"}]},
            "status":"completed"
        }
    })
}

/// The failed run's log: resource tools reached the server, but `solace` did not.
fn resource_calls() -> Vec<Value> {
    vec![agent_tool("i1", "list_mcp_resources", ""), agent_tool("i2", "read_mcp_resource", ""), agent_says("Done.")]
}

#[test]
fn a_run_without_a_solace_call_gets_one_fresh_try() {
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 0, &resource_calls());
    let e =
        supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT)
            .unwrap_err();
    assert_eq!(e.1, NO_TOOL);
    assert_eq!(task(&p.root, &tid).unwrap()["runs"][0]["state"], "running");
}

#[test]
fn a_second_run_without_a_solace_call_says_the_tool_was_missing() {
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 0, &resource_calls());
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, true, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["state"], "ended");
    assert_eq!(t["runs"][0]["outcome"], NO_TOOL_MESSAGE);
    assert_eq!(t["runs"][0]["failed"], true);
    assert_eq!(t["attention"]["reason"], NO_TOOL_MESSAGE);
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["author"] == "solace" && n["text"] == NO_TOOL_MESSAGE));
    assert!(!t["runs"][0]["outcome"].as_str().unwrap().contains("without finishing"));
}

#[test]
fn a_run_that_called_solace_keeps_the_generic_exit() {
    let lines = [agent_tool("i1", "solace", "task diff"), agent_says("Done.")];
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 0, &lines);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert!(t["runs"][0]["outcome"].as_str().unwrap().contains("without finishing"));
}

#[test]
fn a_crash_without_a_solace_call_keeps_the_generic_exit() {
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 7, &resource_calls());
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert!(t["runs"][0]["outcome"].as_str().unwrap().contains("without finishing"));
}

#[test]
fn a_custom_run_without_a_solace_call_keeps_the_generic_exit() {
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"work","state":"running"}), 0, &resource_calls());
    let m = json!({"agent": "codex", "run": ["sh"]});
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &m, false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert!(t["runs"][0]["outcome"].as_str().unwrap().contains("without finishing"));
}

/// A strategy agent that exits cleanly after only resource calls is told the tool was missing.
#[test]
fn a_strategy_run_without_a_solace_call_says_the_tool_was_missing() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let runs = top.join(".solace/strategy/runs");
    fs::create_dir_all(&runs).unwrap();
    fs::write(runs.join("r1.json"), json!({"id":"r1","kind":"strategy","state":"running","notes":[]}).to_string()).unwrap();
    let mut child = Command::new("sh").arg("-c").arg("exit 0").process_group(0).spawn().unwrap();
    let log = top.join(".solace/r1.log");
    let lines = &resource_calls()[..2];
    fs::write(&log, lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    assert!(
        !supervise(Watch::Strategy(&top), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let run = tasks::strategy_run(&top, "r1").unwrap();
    assert_eq!(run["state"], "ended");
    let notes = run["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n["text"].as_str().unwrap_or("").contains(NO_TOOL_MESSAGE)), "{run}");
}

/// A strategy run with no model stops at launch.
#[test]
fn a_run_without_a_model_stops_when_it_starts() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let runs = top.join(".solace/strategy/runs");
    fs::create_dir_all(&runs).unwrap();
    fs::write(runs.join("r1.json"), json!({"id":"r1","kind":"strategy","state":"running","notes":[],"questions":[]}).to_string()).unwrap();
    strategy::start_strategy(top.clone(), "r1".into());
    for _ in 0..100 {
        let run = tasks::strategy_run(&top, "r1").unwrap();
        if run["state"] == "ended" {
            assert_eq!(run["outcome"], "stopped");
            assert!(run["notes"].as_array().unwrap().iter().any(|n| n["text"].as_str().unwrap().contains("a model is required")));
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the run did not stop");
}

#[test]
fn setup_keeps_keys_beside_the_system_settings() {
    let config = crate::board::setup::config_path();
    assert_eq!(crate::board::setup::keys_path(), config.with_file_name("keys.json"));
}

/// The agent behavior route serves the defaults for missing or bad values and
/// saves one choice, keeping the other system settings.
#[test]
fn agent_settings_api_serves_defaults_and_saves() {
    let root = testing::Scratch::new();
    let config = root.join("config.toml");
    setup::with_config(&config, || {
        let board = event_board(root.to_path_buf());
        let defaults = json!({"involvement": "collaborator", "detail": "standard", "asking": "balanced"});
        assert_eq!(board.projects_api("GET", "/agent-settings", &json!({})).unwrap(), defaults);
        fs::write(&config, "task_timeout = 42\n").unwrap();
        let saved = board.projects_api("POST", "/agent-settings", &json!({"key": "detail", "value": "thorough"})).unwrap();
        assert_eq!(saved["detail"], "thorough");
        assert_eq!(saved["involvement"], "collaborator");
        let e = board.projects_api("POST", "/agent-settings", &json!({"key": "detail", "value": "verbose"})).unwrap_err();
        assert_eq!(e.0, "E7001 detail must be concise, standard or thorough");
        fs::write(&config, "involvement = \"boss\"\ndetail = 5\nasking = []\ntask_timeout = 42\n").unwrap();
        assert_eq!(board.projects_api("GET", "/agent-settings", &json!({})).unwrap(), defaults);
        let system = setup::system().unwrap();
        assert_eq!(system["detail"], "standard");
        assert!(!system["models"].as_array().unwrap().is_empty());
        assert_eq!(board.projects_api("GET", "/languages", &json!({})).unwrap().as_array().unwrap().len(), 2);
        let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(v["task_timeout"].as_integer(), Some(42));
    });
}

/// The Strategy brief follows the involvement setting as every task brief does.
#[test]
fn strategy_briefs_speak_at_each_involvement_level() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let run = json!({"context":{"kind":"page"},"notes":[]});
    let director = strategy_brief(&top, &run, "director", "standard", "balanced");
    assert!(director.contains("How to talk to the human (Director)"), "{director}");
    assert!(!director.contains("`task note`"), "{director}");
    assert!(director.contains("`strategy note`") && director.contains("`### Calls made` heading"), "{director}");
    assert!(!director.contains("Each question option"), "{director}");
    assert!(!director.contains("(Collaborator)") && !director.contains("(Engineer)"), "{director}");
    let collaborator = strategy_brief(&top, &run, "collaborator", "standard", "balanced");
    assert!(collaborator.contains("How to talk to the human (Collaborator)"), "{collaborator}");
    assert!(collaborator.contains("`strategy note`") && !collaborator.contains("`task note`"), "{collaborator}");
    assert!(!collaborator.contains("(Director)"), "{collaborator}");
    let engineer = strategy_brief(&top, &run, "engineer", "standard", "balanced");
    assert!(!engineer.contains("How to talk to the human"), "{engineer}");
    assert!(engineer.contains(super::strategy::STRATEGY_ROLE), "{engineer}");
    for brief in [&director, &collaborator, &engineer] {
        assert!(brief.contains(crate::board::briefs::UI_DETAILS_ASK), "{brief}");
        assert!(
            brief.contains("Write a task, feature or milestone body so it names only the user-interface behavior the human asked for"),
            "{brief}"
        );
    }
    let detailed = strategy_brief(&top, &run, "engineer", "thorough", "often");
    assert!(detailed.contains("How much to write (Thorough)") && detailed.contains("When to ask (Often)"), "{detailed}");
}

fn review_run() -> (testing::Project, String) {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title": "Review"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id": "r1", "kind": "review", "state": "running"}]);
    tasks::put(&s, &t).unwrap();
    (p, tid)
}

/// A fake task server: holds the serve lock, waits for one inbox message, then
/// acts on it. It lets go of the lock after five seconds, so a broken test
/// returns instead of waiting forever.
fn fake_server(top: &Path, act: impl FnOnce(&Path, &Path) + Send + 'static) -> std::sync::mpsc::Receiver<()> {
    let top = top.to_path_buf();
    let (ready, waiting) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _lock = fsx::try_lock_file(&store_dir(&top).join("server.lock")).unwrap().unwrap();
        ready.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let sent = std::fs::read_dir(store_dir(&top).join("inbox"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|x| x == "json"));
            if let Some(sent) = sent {
                act(&top, &sent);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    waiting
}

#[test]
fn a_review_note_is_recorded_only_after_the_board_applies_it() {
    let (p, tid) = review_run();
    let s = open(&p.root).unwrap();
    let server = fake_server(&s.root, |top, _| tasks::drain(top));
    server.recv_timeout(Duration::from_secs(5)).unwrap();
    let out =
        task_command(&s, &tid, "r1", &["task", "note", "**Assessment: Looks good**"], &mut || Ok(String::new()), &[], vec![]).unwrap();
    assert_eq!(out, "Recorded.\n");
    let t = task(&s.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    let notes = t["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["author"], "agent");
}

#[test]
fn a_review_note_the_board_sets_aside_is_refused() {
    let (p, tid) = review_run();
    let s = open(&p.root).unwrap();
    let server = fake_server(&s.root, |top, sent| {
        let failed = store_dir(top).join("inbox/failed");
        std::fs::create_dir_all(&failed).unwrap();
        std::fs::rename(sent, failed.join(sent.file_name().unwrap())).unwrap();
    });
    server.recv_timeout(Duration::from_secs(5)).unwrap();
    let e = task_command(&s, &tid, "r1", &["task", "note", "feedback"], &mut || Ok(String::new()), &[], vec![]).unwrap_err().0;
    let failed = store_dir(&s.root).join("inbox/failed");
    let file = std::fs::read_dir(&failed).unwrap().next().unwrap().unwrap().path();
    assert!(e.contains("set this note aside"), "{e}");
    assert!(e.contains(file.file_name().unwrap().to_str().unwrap()), "{e}");
    assert!(e.contains(".solace/tasks/inbox/failed"), "{e}");
    assert!(!e.lines().any(|line| line.starts_with("Recorded")), "{e}");
}

#[test]
fn a_review_note_without_a_server_is_queued() {
    let (p, tid) = review_run();
    let s = open(&p.root).unwrap();
    let out = task_command(&s, &tid, "r1", &["task", "note", "feedback"], &mut || Ok(String::new()), &[], vec![]).unwrap();
    assert_eq!(out, format!("{}\n", tasks::QUEUED));
    assert_eq!(std::fs::read_dir(store_dir(&s.root).join("inbox")).unwrap().count(), 1);
}

#[test]
fn a_batched_review_note_set_aside_is_not_recorded() {
    let (p, tid) = review_run();
    let s = open(&p.root).unwrap();
    let server = fake_server(&s.root, |top, sent| {
        let failed = store_dir(top).join("inbox/failed");
        std::fs::create_dir_all(&failed).unwrap();
        std::fs::rename(sent, failed.join(sent.file_name().unwrap())).unwrap();
    });
    server.recv_timeout(Duration::from_secs(5)).unwrap();
    let (out, failed) = batch(&["task diff", "task note"], 10_000, &mut |words: &[String], _: usize| -> Out {
        let args: Vec<String> = words.to_vec();
        with_root(&s.root, || {
            with_context(&tid, "r1", || run_options(&args, &mut || Ok("**Assessment: Needs work**".to_string()), None, 10_000, &[]))
        })
    });
    assert!(failed, "{out}");
    assert!(out.contains("==== solace task note ===="), "{out}");
    assert!(!out.lines().any(|line| line.starts_with("Recorded")), "{out}");
    assert!(out.contains(".solace/tasks/inbox/failed"), "{out}");
}

/// D-218: a strategy brief carries every persona whole, as task briefs do.
#[test]
fn strategy_briefs_carry_personas() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let run = json!({"context":{"kind":"page"},"notes":[]});
    assert!(!strategy_brief(&top, &run, "engineer", "standard", "balanced").contains("Personas ("));
    let strategy = top.join(".solace/strategy");
    std::fs::create_dir_all(&strategy).unwrap();
    std::fs::write(
        strategy.join("personas.json"),
        concat!(
            r#"{"personas":[{"name":"Ada","objectives":"Ship it.","tools":"A debugger.","#,
            r#""concerns":"","other":""}],"retired":[]}"#
        ),
    )
    .unwrap();
    let brief = strategy_brief(&top, &run, "engineer", "standard", "balanced");
    let section = concat!(
        "\nPersonas (who the software is for, or who works on it, kept on the Strategy page; ",
        "let them guide your choices):\nAda\n  Objectives: Ship it.\n  Tools: A debugger.\n"
    );
    assert!(brief.contains(section), "{brief}");
}

/// D-220: the roles route serves the one role list, with the run kinds each covers.
#[test]
fn roles_api_serves_the_role_set_and_run_kinds() {
    let root = testing::Scratch::new();
    let board = event_board(root.to_path_buf());
    let roles = board.projects_api("GET", "/roles", &json!({})).unwrap();
    let listed = roles.as_array().unwrap();
    let ids: Vec<&str> = listed.iter().map(|role| role["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["strategist", "investigator", "worker", "reviewer", "coordinator"]);
    for (kind, id) in [
        ("strategy", "strategist"),
        ("investigate", "investigator"),
        ("discuss", "investigator"),
        ("work", "worker"),
        ("new_work", "worker"),
        ("review", "reviewer"),
    ] {
        let found = listed.iter().find(|role| role["runs"].as_array().unwrap().iter().any(|run| run.as_str() == Some(kind)));
        assert_eq!(found.map(|role| role["id"].as_str().unwrap()), Some(id), "{kind}");
    }
    let coordinator = listed.iter().find(|role| role["id"] == "coordinator").unwrap();
    assert!(coordinator["runs"].as_array().unwrap().is_empty());
    assert!(listed.iter().all(|role| role.get("model").is_none()));
}

/// D-27: Settings saves a skill file in the workspace's skills folder, lists
/// the skills it manages, edits their roles, and refuses a bad skill without
/// writing anything.
#[test]
fn skills_api_saves_lists_and_removes_role_skills() {
    let root = testing::Scratch::new();
    let board = event_board(root.to_path_buf());
    assert_eq!(board.projects_api("GET", "/skills", &json!({})).unwrap(), json!([]));
    let saved = board
        .projects_api(
            "POST",
            "/skills",
            &json!({"name":"house-review","description":"Review UI changes.","body":"Check contrast and focus.","roles":["reviewer"]}),
        )
        .unwrap();
    assert_eq!(
        saved,
        json!([{
            "name":"house-review","description":"Review UI changes.",
            "body":"Check contrast and focus.","roles":["reviewer"],
            "managed":true,"all_roles":false
        }])
    );
    let file = root.join("skills/house-review.md");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("roles: reviewer") && text.contains("Check contrast and focus."), "{text}");
    let edited = board
        .projects_api(
            "POST",
            "/skills",
            &json!({
                "original":"house-review","name":"house-review","description":"Review UI changes.",
                "body":"Check contrast and focus.","roles":["reviewer","worker"]
            }),
        )
        .unwrap();
    assert_eq!(edited[0]["roles"], json!(["reviewer", "worker"]));
    // A hand-written folder skill is not edited, and a saved name may not take its place.
    fs::create_dir_all(root.join("skills/folder")).unwrap();
    fs::write(root.join("skills/folder/SKILL.md"), "Folder skill\n").unwrap();
    let general = root.join("skills/general.md");
    fs::write(&general, "General rules\n").unwrap();
    let listed = board.projects_api("GET", "/skills", &json!({})).unwrap();
    let found = |name: &str| listed.as_array().unwrap().iter().find(|skill| skill["name"] == name).unwrap().clone();
    assert_eq!(found("general")["all_roles"], json!(true), "a hand-written skill reaches every role: {listed}");
    assert_eq!(found("general")["managed"], json!(false), "a hand-written skill is view only: {listed}");
    assert_eq!(found("folder")["all_roles"], json!(true), "a folder skill with no roles key reaches every role: {listed}");
    assert_eq!(found("folder")["managed"], json!(false), "a folder skill is view only: {listed}");
    let before = fs::read_dir(root.join("skills")).unwrap().count();
    let bad = [
        json!({"name":"","description":"x","body":"x","roles":["reviewer"]}),
        json!({"name":"empty","description":"x","body":"  ","roles":["reviewer"]}),
        json!({"name":"long","description":"x","body":"x".repeat(1001),"roles":["reviewer"]}),
        json!({"name":"../escape","description":"x","body":"x","roles":["reviewer"]}),
        json!({"name":"README","description":"x","body":"x","roles":["reviewer"]}),
        json!({"name":"two lines","description":"one\ntwo","body":"x","roles":["reviewer"]}),
        json!({"name":"unknown","description":"x","body":"x","roles":["nobody"]}),
        json!({"name":"folder","description":"x","body":"x","roles":["reviewer"]}),
        json!({"name":"house-review","description":"x","body":"x","roles":["reviewer"]}),
        json!({"original":"gone","name":"gone","description":"x","body":"x","roles":["reviewer"]}),
    ];
    for body in bad {
        assert!(board.projects_api("POST", "/skills", &body).is_err(), "{body}");
        assert_eq!(fs::read_dir(root.join("skills")).unwrap().count(), before, "{body}");
    }
    // A skill may be saved with no roles: it reaches no run, and Settings still manages it.
    let idle = board
        .projects_api("POST", "/skills", &json!({"name":"idle","description":"Nobody's rules.","body":"Nothing.","roles":[]}))
        .unwrap();
    let idle = idle.as_array().unwrap().iter().find(|skill| skill["name"] == "idle").unwrap();
    assert_eq!(idle["roles"], json!([]));
    assert_eq!(idle["managed"], json!(true));
    assert_eq!(idle["all_roles"], json!(false));
    // A skill whose roles were emptied still edits and deletes.
    let edited = board
        .projects_api(
            "POST",
            "/skills",
            &json!({"original":"idle","name":"idle","description":"Still nobody's.","body":"Nothing again.","roles":[]}),
        )
        .unwrap();
    let edited = edited.as_array().unwrap().iter().find(|skill| skill["name"] == "idle").unwrap();
    assert_eq!(edited["description"], json!("Still nobody's."));
    assert_eq!(edited["roles"], json!([]));
    assert!(
        board
            .projects_api(
                "POST",
                "/skills",
                &json!({"original":"general","name":"general","description":"x","body":"x","roles":["reviewer"]}),
            )
            .is_err(),
        "a hand-written skill is not overwritten"
    );
    assert_eq!(fs::read_to_string(&general).unwrap(), "General rules\n");
    assert!(board.projects_api("DELETE", "/skills/general", &json!({})).is_err(), "a hand-written skill is not deleted");
    assert_eq!(fs::read_to_string(&general).unwrap(), "General rules\n");
    let left = board.projects_api("DELETE", "/skills/house-review", &json!({})).unwrap();
    assert!(left.as_array().unwrap().iter().all(|skill| skill["name"] != "house-review"), "{left}");
    assert_eq!(left.as_array().unwrap().iter().find(|skill| skill["name"] == "idle").unwrap()["roles"], json!([]));
    assert!(!file.exists());
    let left = board.projects_api("DELETE", "/skills/idle", &json!({})).unwrap();
    assert!(left.as_array().unwrap().iter().all(|skill| skill["name"] != "idle"), "{left}");
    assert!(!root.join("skills/idle.md").exists());
    assert!(board.projects_api("DELETE", "/skills/folder", &json!({})).is_err(), "a folder skill is not deleted");
    assert!(root.join("skills/folder/SKILL.md").is_file());
}

/// Role models are served for every role, save to the machine settings, come
/// back after a restart, and a broken or unknown value is empty or kept
/// without blocking the page.
#[test]
fn role_models_api_round_trips_and_falls_back() {
    let root = testing::Scratch::new();
    let config = root.join("config.toml");
    setup::with_config(&config, || {
        let board = event_board(root.to_path_buf());
        let empty = board.projects_api("GET", "/role-models", &json!({})).unwrap();
        let listed = empty.as_array().unwrap();
        assert_eq!(listed.len(), 5);
        assert!(listed.iter().all(|role| role["default"] == "" && role.get("backup").is_none()));
        assert_eq!(listed.iter().find(|role| role["role"] == "worker").unwrap()["name"], "Worker");
        let saved = board.projects_api("POST", "/role-models", &json!({"role": "worker", "default": "cc"})).unwrap();
        let worker = saved.as_array().unwrap().iter().find(|role| role["role"] == "worker").unwrap();
        assert_eq!(worker["default"].as_str(), Some("cc"));
        assert!(worker.get("backup").is_none());
        assert_eq!(board.projects_api("GET", "/role-models", &json!({})).unwrap(), saved);
        let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(v["role_models"]["worker"]["default"].as_str(), Some("cc"));
        assert!(v["role_models"]["worker"].get("backup").is_none());
        let e = board.projects_api("POST", "/role-models", &json!({"role": "nobody", "default": "cc"})).unwrap_err();
        assert_eq!(e.0, "E7001 role must be a known role");
        let stale = board.projects_api("POST", "/role-models", &json!({"role": "reviewer", "default": "gone"})).unwrap();
        let reviewer = stale.as_array().unwrap().iter().find(|role| role["role"] == "reviewer").unwrap();
        assert_eq!(reviewer["default"], "gone");
        fs::write(
            &config,
            concat!(
                "task_timeout = 42\n",
                "role_models = { worker = { default = 5, backup = \"\" }, ",
                "reviewer = { default = \"kept\", backup = \"spare\" }, ",
                "ghost = { default = \"x\" } }\n",
            ),
        )
        .unwrap();
        let broken = board.projects_api("GET", "/role-models", &json!({})).unwrap();
        let listed = broken.as_array().unwrap();
        assert_eq!(listed.len(), 5);
        let worker = listed.iter().find(|role| role["role"] == "worker").unwrap();
        assert_eq!(worker["default"].as_str(), Some(""));
        assert!(worker.get("backup").is_none());
        let reviewer = listed.iter().find(|role| role["role"] == "reviewer").unwrap();
        assert_eq!(reviewer["default"].as_str(), Some("kept"));
        assert!(reviewer.get("backup").is_none());
        assert!(listed.iter().all(|role| role["role"] != "ghost"));
        assert_eq!(setup::system().unwrap()["task_timeout"], 42, "other settings stay");
        board.projects_api("POST", "/role-models", &json!({"role": "worker", "default": "new"})).unwrap();
        let v: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(v["role_models"]["worker"]["default"].as_str(), Some("new"));
        assert!(v["role_models"]["worker"].get("backup").is_none(), "an old backup is removed on save");
    });
}

/// Changing a role's assignment changes later picks, never an open run's model.
#[test]
fn role_model_changes_leave_a_run_model_alone() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Keep"})).unwrap();
    t["model"] = json!("first");
    t["runs"] = json!([{"id":"r1","kind":"work","model":"first","effort":"","state":"running","started":1,"log":""}]);
    tasks::put(&s, &t).unwrap();
    let config_dir = testing::Scratch::new();
    let config = config_dir.join("config.toml");
    fs::write(&config, "role_models = { worker = { default = \"second\", backup = \"old\" } }\n").unwrap();
    setup::with_config(&config, || {
        let board = event_board(p.root.clone());
        board.projects_api("POST", "/role-models", &json!({"role":"worker","default":"second"})).unwrap();
        let got = api(&p.root, "GET", &format!("/tasks/{}", t["id"].as_str().unwrap()), json!({})).unwrap();
        assert_eq!(got["model"], "first");
        assert_eq!(got["runs"][0]["model"], "first");
    });
}

#[test]
fn a_review_that_recovered_from_a_transient_error_is_reviewed() {
    let assessment = "**Assessment: Looks good**\n\n### Findings\n\nFine.";
    let notice = json!({"type":"error","message":"Reconnecting... 1/5"});
    let (p, tid, mut child, log) = exited(json!({"id":"r1","kind":"review","state":"running"}), 0, &[notice, agent_says(assessment)]);
    assert!(
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["author"] == "agent" && n["run"] == "r1" && n["text"] == assessment));
    assert!(t["attention"].is_null());
}

/// Changing a role's model announces the whole list to open pickers.
#[test]
fn role_model_changes_are_logged_for_open_pickers() {
    let root = testing::Scratch::new();
    let config = root.join("config.toml");
    setup::with_config(&config, || {
        let board = event_board(root.to_path_buf());
        let saved = board.projects_api("POST", "/role-models", &json!({"role": "worker", "default": "cc"})).unwrap();
        let data: String =
            tasks::events(&root).unwrap().query_row("SELECT data FROM events ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
        let event: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(event["roleModels"], saved);
    });
}

/// A design added to the workspace reaches an open root stream, even when the
/// root is not a project.
#[test]
fn a_design_change_reaches_the_root_stream() {
    let root = testing::Scratch::new();
    let last = json!({"tasks": 0}).to_string();
    let mut reader = Events::new(event_board(root.to_path_buf()), None, Some(&last));
    let mut buf = [0; 8192];
    let mut next_designs = |reader: &mut Events| {
        loop {
            let n = reader.read(&mut buf).unwrap();
            assert!(n > 0);
            let text = std::str::from_utf8(&buf[..n]).unwrap();
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    let v: Value = serde_json::from_str(data).unwrap();
                    if v.get("designs").is_some() {
                        return v;
                    }
                }
            }
        }
    };
    let mut seen = None;
    crate::design::announce_change(&root, &mut seen);
    assert_eq!(next_designs(&mut reader), json!({"designs": []}));
    let design = json!({
        "schemaVersion": 1,
        "id": "0123456789abcdef",
        "title": "Home",
        "tab": "architecture",
        "elements": []
    });
    crate::design::save(&root, &design, crate::design::Source::Human).unwrap();
    crate::design::announce_change(&root, &mut seen);
    assert_eq!(next_designs(&mut reader), json!({"designs": [{"id": "0123456789abcdef", "title": "Home", "tab": "architecture"}]}));
}

/// D-61/D-112: completing a task saved without a project list files its tech
/// debt with its home as the new task's scope, which reads back whole.
#[test]
fn a_task_with_no_scope_files_tech_debt_with_its_home() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&s.root);
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Old"})).unwrap();
    let home = t["home"].clone();
    let tid = t["id"].as_str().unwrap().to_string();
    t["runs"].as_array_mut().unwrap().push(json!({"id":"rv","kind":"review","state":"ended","outcome":"reviewed"}));
    note(&mut t, "agent", Some("rv"), "### Tech debt\n- `app.main`: name the constant\n");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"plan"})).unwrap();
    let mut t = task(&s.root, &tid).unwrap();
    t.as_object_mut().unwrap().remove("scope");
    tasks::put(&s, &t).unwrap();
    api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
    let debt = all_tasks(&top).unwrap().into_iter().find(|d| d["techDebt"] == true).unwrap();
    assert_eq!(debt["scope"], json!([home]));
    assert_eq!(debt["notes"].as_array().unwrap().len(), 1);
}

#[test]
fn design_reads_work_in_every_run_kind() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Designs"})).unwrap();
    t["runs"] = json!([
        {"id":"w","kind":"work","state":"running"},
        {"id":"i","kind":"investigate","state":"running"},
        {"id":"r","kind":"review","state":"running"},
        {"id":"d","kind":"discuss","state":"running"}
    ]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let id = "0123456789abcdef";
    std::fs::create_dir_all(p.root.join("designs")).unwrap();
    std::fs::write(
        p.root.join("designs").join(format!("{id}.json")),
        json!({"schemaVersion": crate::design::SCHEMA_VERSION, "id": id, "title": "Home", "tab": "architecture", "elements": []})
            .to_string(),
    )
    .unwrap();
    let commands = vec!["design list".to_string(), format!("design show {id}"), format!("design show design:{id}")];
    for rid in ["w", "i", "r", "d"] {
        for command in &commands {
            let args: Vec<String> = command.split(' ').map(String::from).collect();
            let out = crate::cli::routing::with_root(&p.root, || {
                crate::board::runs::with_context(tid, rid, || {
                    crate::cli::routing::run_options(&args, &mut || Ok(String::new()), None, 4000, &[])
                })
            })
            .unwrap();
            assert!(out.contains(id) && out.contains("Home"), "{rid} {command}: {out}");
        }
    }
    for command in &commands[..2] {
        let args: Vec<String> = command.split(' ').map(String::from).collect();
        let out = crate::cli::routing::with_root(&p.root, || crate::cli::projects::read_project(".", &args, 4000)).unwrap();
        assert!(out.contains(id), "{command}: {out}");
    }
}
