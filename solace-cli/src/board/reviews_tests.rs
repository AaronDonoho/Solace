#[cfg(test)] use crate::board::reviews::settle_result;
use super::*;
use crate::testing;
use serde_json::json;
use serde_json::Value;
use std::fs;
use rusqlite::params;
use crate::board::task_model::open;
use crate::board::briefs::continuation_brief;
use crate::board::briefs::START;
use crate::board::briefs::HELD_START;
use crate::board::briefs::TECH_DEBT_ASK;
use crate::board::briefs::WORK_TECH_DEBT_ASK;
use crate::board::reviews::snapshot;
use crate::board::reviews::record;
use crate::board::reviews::reviewable;
use crate::board::reviews::review_path;
use crate::board::reviews::review_state;
use crate::board::reviews::task_diff;
use crate::board::reviews::tech_debt_items;
use crate::board::server::api;
use crate::board::task_model::task;
use crate::board::task_model::all_tasks;
use crate::board::task_model::note;
use crate::board::task_model::end_runs;
use crate::Store;
use crate::jsonx;
use crate::scope;
use crate::tasks;

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

/// A busy checkpoint is named once, not once per wrapping layer.
#[test]
fn settling_reports_a_busy_flush_once() {
    assert!(settle_result(vec![], vec![]).is_ok());
    let e = settle_result(vec!["a".into(), "b".into()], vec![]).unwrap_err();
    assert_eq!(e.0.matches("E7028").count(), 1, "{}", e.0);
    assert!(e.0.contains("pending in a, b"), "{}", e.0);
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

#[test]
fn a_single_reply_with_the_batch_marker_is_recorded() {
    assert!(note_recorded("Recorded.\n==== solace read a ====\n"));
    assert!(note_recorded("Recorded; ==== solace task note ==== was in the text\n"));
    assert!(!note_recorded("==== solace inspect a ====\nRecorded.\n"));
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
    assert_eq!(tech_debt_items(WORK_TECH_DEBT_ASK), vec!["`<command or step>`: <what was wasted>. <what would fix it>."]);
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
        (Some("Search"), Some("planning"), Some(fid.as_str()))
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
    jsonx::write(&path, &checkpoint).unwrap();
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
    jsonx::write(&path, &checkpoint).unwrap();
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

