use super::*;
use crate::board::agents;
use crate::scope;
use crate::testing;
use serde_json::json;
use std::env;
use std::fs;
use crate::board::task_model::store_dir;
use crate::board::task_model::task;
use crate::board::task_model::all_tasks;
use crate::board::task_model::tests::homeless_root as homeless_root_2;
use serde_json::Value;
use crate::board::server::api;
use crate::board::task_model::new_task;
use crate::tasks;
use crate::board::task_model::tests::homeless_root as homeless_root_3;

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
    let default = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/default-board.toml"));
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
/// The model a run used comes from its log's start event, and an alias's
/// label is the plain text stored with that run.
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
    assert_eq!(m[0]["label"], "claude-opus-5-5", "a record from before labels shows its model id");
    assert_eq!(m[0]["model"], "opus", "the alias is what runs");
    let listed = json!([configured[0], {"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5"}]);
    let m = agents::merge_from(&listed, &json!({"used":{"claude-opus":{"model":"claude-opus-5-5","label":"Opus 5.5"}}}));
    assert_eq!(m[0]["label"], "Opus 5.5", "the stored label names it");
    let own = agents::merge_from(
        &json!([{"id":"local","label":"Local: Strata Swift","agent":"claude","model":"swift-iq3_xxs","configured":true}]),
        &json!({"used":{"local":{"model":"swift-iq2_xs","label":"Swift IQ2"}}}),
    );
    assert_eq!(own[0]["label"], "Local: Strata Swift", "a model with its own name shows it, not what an older run used");
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

/// A root that is a project listing `sub`, with task t1 at home in `web`, no
/// project: its folder is in the root's board, where its runs wrote.
pub(crate) fn homeless_root() -> testing::Scratch {
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
    let root = homeless_root_2();
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
    let root = homeless_root_3();
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
    let root = homeless_root_3();
    fs::write(store_dir(&root).join("t1.json"), r#"{"id":"t1","column":"backlog","home":"web","scope":["web"],"runs":[],"notes":[]}"#)
        .unwrap();
    let t = all_tasks(&root).unwrap().into_iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!((t["home"].as_str(), t["scope"].clone()), (Some("."), json!(["."])), "{t}");
    let saved = fs::read_to_string(store_dir(&root).join("t1.json")).unwrap();
    assert!(saved.contains("Moved from `web` to `.`"), "{saved}");
}

/// D-160: the mark comes from the home on each load, so a `noProject` saved by
/// an older build is dropped: a task at the root lists and opens unmarked.
#[test]
fn a_saved_no_project_mark_is_dropped_on_load() {
    let root = homeless_root_3();
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
    let root = homeless_root_3();
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

