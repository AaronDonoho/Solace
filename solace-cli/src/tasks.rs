//! The task server (D-77): the only writer of the root's task store,
//! `<root>/.solace/tasks/`, and its API, `/tasks/...`. It applies operations
//! one at a time, in order, on one thread, writing each task file by
//! write-then-rename and logging each change for `/events`. Other processes
//! (agents, the CLI) don't write tasks: they leave messages in `inbox/`, which
//! the server applies in order, and again after a restart (D-77 point 3).
//! Clients depend only on the API and the messages, never on how this is built.
#[cfg(test)] use super::*;
#[cfg(test)] use std::sync::mpsc;
#[cfg(test)] use std::time::Duration;

#[cfg(test)] use serde_json::Value;
#[cfg(test)] use serde_json::json;
#[cfg(test)] use crate::tasks::server::Job;
#[cfg(test)] use crate::tasks::server::SERVER;
#[cfg(test)] use crate::tasks::server::tick;
#[cfg(test)] use crate::tasks::server::op;
#[cfg(test)] use crate::tasks::server::inbox;
#[cfg(test)] use crate::tasks::server::server_lock;
#[cfg(test)] use crate::tasks::server::flag_set_aside;
#[cfg(test)] use crate::tasks::server::failed_dir;
#[cfg(test)] use crate::tasks::server::apply;
pub(super) use crate::tasks::server::start;
pub(super) use crate::tasks::server::send;
pub(super) use crate::tasks::server::serving;
pub(super) use crate::tasks::server::deliver;
pub(super) use crate::tasks::server::QUEUED;
pub(super) use crate::tasks::server::applied;
#[cfg(test)] pub(super) use crate::tasks::server::drain;
pub(super) use crate::tasks::server::rejected;
pub(super) use crate::tasks::server::settle_later;
pub(super) mod server;
pub(super) mod store;
pub(super) mod strategy;
pub(super) mod api;
#[cfg(test)] use crate::tasks::store::log;
#[cfg(test)] use crate::tasks::store::finish_write;
#[cfg(test)] use crate::tasks::store::batch;
#[cfg(test)] use crate::tasks::store::batch_keep;
#[cfg(test)] use crate::tasks::store::batch_replace;
#[cfg(test)] use crate::tasks::store::batch_commit;
#[cfg(test)] use crate::tasks::store::batch_rollback;
#[cfg(test)] use crate::tasks::store::with_batch;
#[cfg(test)] use crate::tasks::store::batch_log;
pub(super) use crate::tasks::store::put;
pub(super) use crate::tasks::store::update;
pub(super) use crate::tasks::store::records;
pub(super) use crate::tasks::store::record;
pub(super) use crate::tasks::store::events;
#[cfg(test)] pub(super) use crate::tasks::store::recover;
#[cfg(test)] use crate::tasks::strategy::strategy_dir;
#[cfg(test)] use crate::tasks::strategy::vision_file;
#[cfg(test)] use crate::tasks::strategy::put_run;
#[cfg(test)] use crate::tasks::strategy::runs_api;
pub(super) use crate::tasks::strategy::strategy_api;
pub(super) use crate::tasks::strategy::VISION_LIMIT;
pub(super) use crate::tasks::strategy::vision;
pub(super) use crate::tasks::strategy::vision_api;
pub(super) use crate::tasks::strategy::strategy_runs;
pub(super) use crate::tasks::strategy::strategy_run;
pub(super) use crate::tasks::strategy::update_run;
pub(super) use crate::tasks::strategy::end_strategy_runs;
pub(super) use crate::tasks::strategy::end_strategy;
pub(super) use crate::tasks::strategy::strategy_runs_api;
pub(super) use crate::tasks::strategy::bare_id;
#[cfg(test)] pub(super) use crate::tasks::strategy::bare_links;
pub(super) use crate::tasks::strategy::proposal;
#[cfg(test)] use crate::tasks::api::api_with;
#[cfg(test)] use crate::tasks::api::change;
pub(super) use crate::tasks::api::find_task;
pub(super) use crate::tasks::api::find_record;
pub(super) use crate::tasks::api::api;
pub(super) use crate::tasks::api::with_on_board;
pub(super) use crate::tasks::store::rehome;
pub(super) use crate::tasks::strategy::plan_active;
pub(super) use crate::tasks::api::has_project;
#[cfg(test)] use crate::board::task_model::Column;
#[cfg(test)]
mod tests {
    use super::*;

    /// A root whose task store holds t1, in `backlog`.
    fn root() -> testing::Scratch {
        let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"backlog"}"#)]);
        board::open(&root).unwrap();
        board::adopt_tasks(&root, &root).unwrap();
        root
    }
    fn stored(root: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(board::store_dir(root)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        names.sort();
        names
    }

    /// The id names the task's file, so one that could leave the store is refused.
    #[test]
    fn write_refuses_an_id_that_is_not_a_plain_name() {
        let root = root();
        let before = stored(&root);
        for bad in [json!(""), json!("../x"), json!("a/b"), json!("a b"), json!("a.b"), json!(1), Value::Null] {
            let e = store::write(&root, &json!({"id": bad, "home": "."})).unwrap_err();
            assert!(e.0.contains("E7001") && e.0.contains("invalid task id"), "{bad}: {}", e.0);
        }
        assert_eq!(stored(&root), before);
        assert!(!root.join(".solace/x.json").exists());
        store::write(&root, &json!({"id": "ab-12", "home": "."})).unwrap();
        assert!(stored(&root).contains(&"ab-12.json".to_string()));
        assert!(!stored(&root).iter().any(|n| n.ends_with(".tmp")), "the temporary file is renamed");
    }

    /// A create applied again (a replay after a crash) leaves the first one.
    #[test]
    fn a_create_message_applies_once() {
        let root = root();
        let mut t = board::new_task("New", "", &json!("backlog"), 1.);
        t["home"] = json!(".");
        let tid = t["id"].as_str().unwrap().to_string();
        assert_eq!(apply(&root, &json!({"id": "m1", "kind": "create", "new": t})).unwrap(), None);
        let first = board::task(&root, &tid).unwrap();
        assert_eq!((first["title"].as_str(), first["inbox"].as_str()), (Some("New"), Some("m1")));
        t["title"] = json!("Again");
        assert_eq!(apply(&root, &json!({"id": "m2", "kind": "create", "new": t})).unwrap(), None);
        assert_eq!(board::task(&root, &tid).unwrap(), first);
    }

    /// D-77 point 6: a create with a `request` seen before returns its task.
    #[test]
    fn a_create_with_a_known_request_returns_its_task() {
        let root = root();
        let post = |body: Value| api(&root, "POST", &["tasks"], body);
        let a = post(json!({"title": "A", "request": "req-1"})).unwrap();
        assert_eq!(a["request"], "req-1");
        let again = post(json!({"title": "A, resent", "request": "req-1"})).unwrap();
        assert_eq!(again["id"], a["id"]);
        assert_eq!(board::all_tasks(&root).unwrap().len(), 2);
        // Without a request, each create is a new task.
        let (b, c) = (post(json!({"title": "B"})).unwrap(), post(json!({"title": "B"})).unwrap());
        assert_ne!(b["id"], c["id"]);
        assert!(b.get("request").is_none_or(Value::is_null));
        for bad in [json!(""), json!(5), json!(null)] {
            let e = post(json!({"title": "C", "request": bad})).unwrap_err();
            assert!(e.0.contains("request must be a string"), "{bad}: {}", e.0);
        }
        assert_eq!(board::all_tasks(&root).unwrap().len(), 4);
    }

    /// D-87: milestones and features are created at the end of their group, edited,
    /// reordered and deleted; a missing folder holds none.
    #[test]
    fn milestones_and_features_are_created_edited_reordered_and_deleted() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body);
        let titles = |kind: &str| -> Vec<String> { records(&root, kind).iter().map(|r| r["title"].as_str().unwrap().into()).collect() };
        assert_eq!(call("GET", &["milestones"], json!({})).unwrap(), json!([]));
        assert_eq!(call("GET", &["features"], json!({})).unwrap(), json!([]));
        let a = call("POST", &["milestones"], json!({"title": "1.0"})).unwrap();
        let b = call("POST", &["milestones"], json!({"title": "2.0", "state": "upcoming", "targetDate": "2027-01-01"})).unwrap();
        assert_eq!(
            (a["state"].as_str(), b["state"].as_str(), b["targetDate"].as_str()),
            (Some("proposed"), Some("upcoming"), Some("2027-01-01"))
        );
        assert_eq!((a["rank"].as_f64(), b["rank"].as_f64()), (Some(1.), Some(2.)));
        let (aid, bid) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
        let edited = call("PATCH", &["milestones", aid], json!({"title": "1.0 final", "rank": 3})).unwrap();
        assert_eq!((edited["title"].as_str(), edited["state"].as_str()), (Some("1.0 final"), Some("proposed")));
        assert_eq!(titles("milestones"), ["2.0", "1.0 final"], "a new rank reorders");
        let e = call("PATCH", &["milestones", aid], json!({"state": "done"})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        assert_eq!(record(&root, "milestones", aid).unwrap(), edited, "a refused edit writes nothing");

        // Features rank within their milestone, or among those with none.
        let feature = |title: &str, milestone: Value| call("POST", &["features"], json!({"title": title, "milestone": milestone})).unwrap();
        let (f1, f2, f3) = (feature("F1", json!(aid)), feature("F2", json!(aid)), feature("F3", Value::Null));
        assert_eq!((f1["rank"].as_f64(), f2["rank"].as_f64(), f3["rank"].as_f64()), (Some(1.), Some(2.), Some(1.)));
        assert_eq!((f1["state"].as_str(), f3["milestone"].is_null()), (Some("proposed"), true));
        let (f1id, f2id, f3id) = (f1["id"].as_str().unwrap(), f2["id"].as_str().unwrap(), f3["id"].as_str().unwrap());
        // Moving to another milestone puts a feature at the end of the new group, unless it is ranked.
        let moved = call("PATCH", &["features", f1id], json!({"milestone": bid})).unwrap();
        assert_eq!((moved["milestone"].as_str(), moved["rank"].as_f64()), (Some(bid), Some(1.)));
        let moved = call("PATCH", &["features", f2id], json!({"milestone": bid})).unwrap();
        assert_eq!(moved["rank"].as_f64(), Some(2.));
        let ranked = call("PATCH", &["features", f2id], json!({"milestone": aid, "rank": 7})).unwrap();
        assert_eq!(ranked["rank"].as_f64(), Some(7.));
        let canceled = call("PATCH", &["features", f3id], json!({"state": "canceled", "body": "no"})).unwrap();
        assert_eq!((canceled["state"].as_str(), canceled["body"].as_str()), (Some("canceled"), Some("no")));
        // A feature's startTime is set when it first becomes active, and kept after.
        assert!(f1["startTime"].is_null() && canceled["startTime"].is_null());
        let active = call("PATCH", &["features", f1id], json!({"state": "active"})).unwrap();
        let started = active["startTime"].as_u64().expect("set on becoming active");
        let done = call("PATCH", &["features", f1id], json!({"state": "complete"})).unwrap();
        let again = call("PATCH", &["features", f1id], json!({"state": "active", "startTime": 1, "colorSlot": 9})).unwrap();
        assert_eq!((done["startTime"].as_u64(), again["startTime"].as_u64()), (Some(started), Some(started)));
        let born = call("POST", &["features"], json!({"title": "F4", "state": "active"})).unwrap();
        assert!(born["startTime"].as_u64().is_some(), "created active");
        // D-114: each started feature keeps a color slot, one past the highest; a delete moves no other's.
        assert!(f1["colorSlot"].is_null() && canceled["colorSlot"].is_null());
        let f2on = call("PATCH", &["features", f2id], json!({"state": "active"})).unwrap();
        let slots = (active["colorSlot"].as_u64(), again["colorSlot"].as_u64(), born["colorSlot"].as_u64(), f2on["colorSlot"].as_u64());
        assert_eq!(slots, (Some(0), Some(0), Some(1), Some(2)));
        call("DELETE", &["features", born["id"].as_str().unwrap()], json!({})).unwrap();
        assert_eq!(record(&root, "features", f2id).unwrap()["colorSlot"].as_u64(), Some(2));

        call("DELETE", &["features", f3id], json!({})).unwrap();
        assert_eq!(titles("features"), ["F1", "F2"]);
        call("DELETE", &["milestones", bid], json!({})).unwrap();
        assert_eq!(titles("milestones"), ["1.0 final"]);
        assert!(!strategy_dir(&root, "milestones").join(format!("{bid}.json")).exists());
        assert!(!strategy_dir(&root, "features").read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    /// D-114: a started feature saved before color slots were kept takes one
    /// after every kept slot, in start order, and keeps it from the next write.
    #[test]
    fn an_old_started_feature_takes_a_slot_after_the_kept_ones() {
        let root = root();
        let dir = strategy_dir(&root, "features");
        let put = |v: Value| jsonx::replace_pretty(&dir, &format!("{}.json", v["id"].as_str().unwrap()), &v).unwrap();
        put(json!({"id": "old2", "title": "O2", "state": "active", "rank": 1, "startTime": 20}));
        put(json!({"id": "old1", "title": "O1", "state": "complete", "rank": 2, "startTime": 10}));
        put(json!({"id": "kept", "title": "K", "state": "active", "rank": 3, "startTime": 30, "colorSlot": 3}));
        put(json!({"id": "idle", "title": "I", "state": "proposed", "rank": 4}));
        let slot = |id: &str| record(&root, "features", id).unwrap()["colorSlot"].as_u64();
        assert_eq!((slot("old1"), slot("old2"), slot("kept"), slot("idle")), (Some(4), Some(5), Some(3), None));
        let born = strategy_api(&root, "POST", &["features"], json!({"title": "New", "state": "active"})).unwrap();
        assert_eq!(born["colorSlot"].as_u64(), Some(6));
        let saved: Value = serde_json::from_slice(&fs::read(dir.join("old1.json")).unwrap()).unwrap();
        assert_eq!(saved["colorSlot"].as_u64(), Some(4), "saved at the next write");
        strategy_api(&root, "DELETE", &["features", born["id"].as_str().unwrap()], json!({})).unwrap();
        strategy_api(&root, "DELETE", &["features", "kept"], json!({})).unwrap();
        assert_eq!((slot("old1"), slot("old2"), slot("idle")), (Some(4), Some(5), None));
    }

    /// D-87: deleting a milestone clears the link on its features; an unknown
    /// milestone is refused with E7023 (400 in a body, 404 in a path) and writes nothing.
    #[test]
    fn a_deleted_milestone_unlinks_its_features_and_an_unknown_one_is_refused() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body);
        let m = call("POST", &["milestones"], json!({"title": "M"})).unwrap();
        let mid = m["id"].as_str().unwrap();
        let f = call("POST", &["features"], json!({"title": "F", "milestone": mid, "request": "r1"})).unwrap();
        let again = call("POST", &["features"], json!({"title": "F again", "milestone": mid, "request": "r1"})).unwrap();
        assert_eq!(again["id"], f["id"], "a known request returns its record");
        assert_eq!(records(&root, "features").len(), 1);

        let e = call("POST", &["features"], json!({"title": "G", "milestone": "nope"})).unwrap_err();
        assert_eq!(e.0, 400);
        assert!(e.1.0.starts_with("E7023"), "{}", e.1.0);
        let e = call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": "nope"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7023")), (400, true));
        let e = call("PATCH", &["milestones", "nope"], json!({"title": "x"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7023")), (404, true));
        let e = call("DELETE", &["features", "../x"], json!({})).unwrap_err();
        assert_eq!(e.0, 404);
        assert_eq!(records(&root, "features"), [f.clone()], "nothing was written");

        let deleted = call("DELETE", &["milestones", mid], json!({})).unwrap();
        assert_eq!(deleted["id"], m["id"]);
        let kept = records(&root, "features");
        assert_eq!((kept.len(), kept[0]["milestone"].is_null(), kept[0]["title"].as_str()), (1, true, Some("F")));
        // Each change was logged without a project, in order.
        let rows: Vec<String> = events(&root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let keys: Vec<String> =
            rows.iter().map(|d| serde_json::from_str::<Value>(d).unwrap().as_object().unwrap().keys().next().unwrap().clone()).collect();
        assert_eq!(keys, ["milestone", "feature", "feature", "removedMilestone"]);
    }

    /// D-91: the vision starts empty; a put over the current revision writes and
    /// logs it, and one over an older revision is E7026 and writes nothing.
    #[test]
    fn a_vision_put_over_a_stale_revision_is_refused() {
        let root = root();
        let empty = vision_api(&root, "GET", json!({})).unwrap();
        assert_eq!(empty["text"], "");
        let base = empty["revision"].as_str().unwrap().to_string();
        let saved = vision_api(&root, "PUT", json!({"text": "# Why\nFor teams.", "base": base})).unwrap();
        assert_eq!(saved["text"], "# Why\nFor teams.");
        assert_ne!(saved["revision"].as_str(), Some(base.as_str()));
        assert_eq!(fs::read_to_string(vision_file(&root)).unwrap(), "# Why\nFor teams.");

        let e = vision_api(&root, "PUT", json!({"text": "lost", "base": base})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7026")), (409, true), "{}", e.1.0);
        assert_eq!(vision(&root), saved, "nothing was written");
        assert_eq!(vision_api(&root, "PUT", json!({"text": "x"})).unwrap_err().0, 400);
        // D-92: at most VISION_LIMIT characters, counted as chars, not bytes.
        let base = saved["revision"].as_str().unwrap();
        let e = vision_api(&root, "PUT", json!({"text": "é".repeat(VISION_LIMIT + 1), "base": base})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7027")), (400, true), "{}", e.1.0);
        assert_eq!(vision(&root), saved, "nothing was written");

        let rows: Vec<String> = events(&root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.iter().map(|d| serde_json::from_str::<Value>(d).unwrap()).collect::<Vec<_>>(), [json!({"vision": saved})]);
        // Exactly at the limit is accepted, whatever its size in bytes.
        let full = vision_api(&root, "PUT", json!({"text": "é".repeat(VISION_LIMIT), "base": base})).unwrap();
        assert_eq!(full["text"].as_str().unwrap().chars().count(), VISION_LIMIT);
    }

    /// The models a strategy run may choose in these tests: one available, one not.
    fn models() -> Result<Value, Failure> {
        Ok(json!({"models": [
            {"id": "fake", "label": "Fake", "agent": "codex", "run": ["sh"]},
            {"id": "missing", "label": "Missing", "agent": "codex", "run": ["solace-no-such-program"]}
        ]}))
    }
    fn root_events(root: &Path) -> Vec<Value> {
        let rows: Vec<String> = events(root)
            .unwrap()
            .prepare("SELECT data FROM events WHERE project='' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.iter().map(|d| serde_json::from_str(d).unwrap()).collect()
    }

    /// D-93: a run starts with its first note; a second open run on the same
    /// context is E7003, notes follow, and `end` ends it `planned` with one
    /// transcript note, after which the context is free again.
    #[test]
    fn a_strategy_run_starts_takes_notes_and_ends() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        let on_f = json!({"kind": "feature", "id": f["id"]});
        let run = call("POST", &[], json!({"model": "fake", "text": "Plan F", "context": on_f})).unwrap();
        assert_eq!((run["kind"].as_str(), run["state"].as_str(), &run["context"]), (Some("strategy"), Some("running"), &on_f));
        assert_eq!((run["notes"][0]["author"].as_str(), run["notes"][0]["text"].as_str()), (Some("human"), Some("Plan F")));
        let rid = run["id"].as_str().unwrap();
        let e = call("POST", &[], json!({"model": "fake", "text": "Again", "context": on_f})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
        let page = call("POST", &[], json!({"model": "fake", "text": "Whole page", "context": {"kind": "page"}})).unwrap();
        assert_eq!(page["context"], json!({"kind": "page"}));

        let noted = call("POST", &[rid, "notes"], json!({"text": "And tasks"})).unwrap();
        assert_eq!(noted["notes"].as_array().unwrap().len(), 2);
        let ended = call("POST", &[rid, "end"], json!({})).unwrap();
        assert_eq!((ended["state"].as_str(), ended["outcome"].as_str()), (Some("ended"), Some("planned")));
        let notes = ended["notes"].as_array().unwrap();
        assert_eq!((notes.len(), notes[0]["author"].as_str()), (1, Some("solace")));
        let text = notes[0]["text"].as_str().unwrap();
        assert!(text.contains("**Human:**\n\nPlan F") && text.contains("**Human:**\n\nAnd tasks"), "{text}");
        for (parts, body) in [(vec![rid, "notes"], json!({"text": "late"})), (vec![rid, "end"], json!({})), (vec![rid, "stop"], json!({}))]
        {
            let e = call("POST", &parts, body).unwrap_err();
            assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{parts:?}: {}", e.1.0);
        }
        assert_eq!(record(&root, "runs", rid).unwrap(), ended, "a refused change writes nothing");
        // Once the first run has ended, the feature takes a new one.
        call("POST", &[], json!({"model": "fake", "text": "Later", "context": on_f})).unwrap();
        let stopped = call("POST", &[page["id"].as_str().unwrap(), "stop"], json!({})).unwrap();
        assert_eq!(stopped["outcome"], "stopped");
    }

    /// D-93, D-202: a malformed start, an unknown record, model or effort is
    /// refused and writes nothing; a task context is refused naming the page;
    /// an unknown run is E7023 with 404.
    #[test]
    fn a_strategy_run_refuses_bad_requests() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let page = json!({"kind": "page"});
        for (body, status, code) in [
            (json!({"model": "fake", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "text": " ", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "text": "x"}), 400, "E7001"),
            (json!({"model": "fake", "text": "x", "context": {"kind": "feature", "id": "nope"}}), 400, "E7023"),
            (json!({"model": "fake", "text": "x", "context": {"kind": "milestone", "id": "nope"}}), 400, "E7023"),
            (json!({"model": "other", "text": "x", "context": page}), 400, "E7001"),
            (json!({"model": "fake", "effort": "high", "text": "x", "context": page}), 400, "E7001"),
            (json!({"model": "missing", "text": "x", "context": page}), 409, "E7011"),
        ] {
            let e = call("POST", &[], body.clone()).unwrap_err();
            assert_eq!((e.0, &e.1.0[..5]), (status, code), "{body}: {}", e.1.0);
        }
        for id in ["t1", "nope"] {
            let e = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "task", "id": id}})).unwrap_err();
            assert_eq!(e.0, 400, "{id}: {}", e.1.0);
            assert!(e.1.0.starts_with("E7001"), "{id}: {}", e.1.0);
            assert!(e.1.0.contains("page"), "{id}: {}", e.1.0);
        }
        assert_eq!(strategy_runs(&root), Vec::<Value>::new());
        let on_page = call("POST", &[], json!({"model": "fake", "text": "x", "context": page})).unwrap();
        assert_eq!(on_page["context"], page);
        for (m, parts) in [("GET", vec!["nope"]), ("POST", vec!["nope", "notes"]), ("POST", vec!["../x", "end"])] {
            let e = call(m, &parts, json!({"text": "x"})).unwrap_err();
            assert_eq!((e.0, e.1.0.starts_with("E7023")), (404, true), "{parts:?}: {}", e.1.0);
        }
        let rid = on_page["id"].as_str().unwrap();
        assert_eq!(call("POST", &[rid, "notes"], json!({})).unwrap_err().0, 400);
        assert_eq!(call("POST", &[rid, "questions", "q1"], json!({"option": "Approve"})).unwrap_err().0, 404);
        // A run stored on a task is a page run: a page start is refused (D-202).
        let legacy = self::root();
        put_run(&legacy, &json!({"id": "old", "kind": "strategy", "state": "running", "context": {"kind": "task", "id": "t1"}})).unwrap();
        let e = runs_api(&legacy, "POST", &[], json!({"model": "fake", "text": "x", "context": page}), models).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
    }

    /// D-93: an answer is recorded on its question once; the option must be offered.
    #[test]
    fn a_strategy_question_is_answered_once() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let mut run = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}})).unwrap();
        run["questions"] = json!([{"id": "q1", "text": "Create T?", "options": ["Approve", "Reject"], "answer": null}]);
        put_run(&root, &run).unwrap();
        let rid = run["id"].as_str().unwrap();
        let e = call("POST", &[rid, "questions", "q1"], json!({"option": "Maybe"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7001")), (400, true), "{}", e.1.0);
        let answered = call("POST", &[rid, "questions", "q1"], json!({"option": "Reject"})).unwrap();
        assert_eq!(answered["questions"][0]["answer"], "Reject");
        let e = call("POST", &[rid, "questions", "q1"], json!({"option": "Approve"})).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
    }

    /// A strategy run on the whole page, for proposals.
    fn page_run(root: &Path) -> String {
        let run = runs_api(root, "POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}}), models).unwrap();
        run["id"].as_str().unwrap().to_string()
    }
    /// Puts a proposal on run `rid` as its agent's message would; its question id.
    fn propose(root: &Path, rid: &str, kind: &str, id: Option<&str>, text: &str) -> String {
        let q = proposal(root, rid, kind, id, text).unwrap();
        let qid = q["id"].as_str().unwrap().to_string();
        let mut run = record(root, "runs", rid).unwrap();
        run["questions"].as_array_mut().unwrap().push(q);
        put_run(root, &run).unwrap();
        qid
    }
    /// The human's answer to question `qid`: the answer stored, and the run's last note.
    fn answer(root: &Path, rid: &str, qid: &str, option: &str) -> (String, String) {
        let run = runs_api(root, "POST", &[rid, "questions", qid], json!({"option": option}), models).unwrap();
        let q = run["questions"].as_array().unwrap().iter().find(|q| q["id"] == qid).unwrap().clone();
        let last = run["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().to_string();
        (q["answer"].as_str().unwrap().to_string(), last)
    }

    /// D-93: Approve creates a feature, a milestone or a task, says its id, and
    /// notes the answer on the run; the task gets a note naming the run.
    #[test]
    fn an_approved_proposal_creates_the_record() {
        let root = root();
        let rid = page_run(&root);
        for kind in ["feature", "milestone", "task"] {
            let qid = propose(&root, &rid, kind, None, r#"{"title": "New one"}"#);
            let (said, note) = answer(&root, &rid, &qid, "Approve");
            let prefix = format!("Approved and applied: created {kind}:");
            assert!(said.starts_with(&prefix), "{said}");
            let id = &said[prefix.len()..];
            assert!(note.contains(&format!("Create {kind} \"New one\"")) && note.contains(&said), "{note}");
            let created = if kind == "task" { board::task(&root, id).unwrap() } else { record(&root, &format!("{kind}s"), id).unwrap() };
            assert_eq!(created["title"], "New one");
            if kind == "task" {
                assert_eq!((created["home"].as_str(), created["column"].as_str()), (Some("."), Some("backlog")));
                let n = created["notes"].as_array().unwrap().last().unwrap().clone();
                assert_eq!(
                    (n["author"].as_str(), n["text"].as_str()),
                    (Some("solace"), Some(format!("Created by strategy run {rid}").as_str()))
                );
            }
        }
        // An edit says nothing more, and the task notes the change.
        let qid = propose(&root, &rid, "task", Some("t1"), r#"{"title": "Renamed"}"#);
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        let t1 = board::task(&root, "t1").unwrap();
        assert_eq!(
            (t1["title"].as_str(), t1["notes"][0]["text"].as_str()),
            (Some("Renamed"), Some(format!("Changed by strategy run {rid}").as_str()))
        );
    }

    /// D-129: a proposed task whose scope names a folder that isn't a project is
    /// refused when the proposal is sent, naming the folder and the projects, and
    /// nothing is written. A folder that is a project is accepted.
    #[test]
    fn a_proposal_scope_must_name_projects() {
        let root = root();
        let rid = page_run(&root);
        for text in [r#"{"title": "Later", "scope": ["web", "api"]}"#, r#"{"title": "Half", "scope": [".", "web"]}"#] {
            let e = proposal(&root, &rid, "task", None, text).unwrap_err();
            assert!(
                e.0.starts_with("E7001") && e.0.contains("is not a project") && e.0.contains("web") && e.0.contains("."),
                "{text}: {}",
                e.0
            );
        }
        assert_eq!(board::all_tasks(&root).unwrap().len(), 1, "the refused proposals wrote nothing");
        assert!(proposal(&root, &rid, "task", None, r#"{"title": "Here", "scope": ["."], "dependsOn": ["t1"]}"#).is_ok());
        let e = proposal(&root, &rid, "task", None, r#"{"title": "Out", "scope": ["../x"]}"#).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{}", e.0);
    }

    /// D-105: a proposal naming a field its request doesn't take is refused,
    /// naming the field and those it takes.
    #[test]
    fn a_proposal_refuses_a_field_it_does_not_take() {
        let root = root();
        let rid = page_run(&root);
        for (kind, id, text, bad) in [
            ("task", None, r#"{"title": "T", "dependson": ["t1"]}"#, "dependson"),
            ("task", Some("t1"), r#"{"scope": ["web"]}"#, "scope"),
            ("feature", None, r#"{"title": "F", "rank": 1}"#, "rank"),
            ("milestone", None, r#"{"title": "M", "milestone": "x"}"#, "milestone"),
        ] {
            let e = proposal(&root, &rid, kind, id, text).unwrap_err();
            assert!(e.0.starts_with("E7001") && e.0.contains(&format!("not `{bad}`")) && e.0.contains("title, body"), "{kind}: {}", e.0);
        }
        assert!(proposal(&root, &rid, "task", None, r#"{"title": "T", "dependsOn": []}"#).is_ok());
    }

    /// D-115: an edit proposal carries the record's current value of each field
    /// it proposes, as `before`, so the human sees only what changes; a create
    /// carries none, and the vision carries its text.
    #[test]
    fn an_edit_proposal_carries_the_current_values() {
        let root = root();
        let rid = page_run(&root);
        let asked = |qid: &str| {
            let run = record(&root, "runs", &rid).unwrap();
            run["questions"].as_array().unwrap().iter().find(|q| q["id"] == qid).unwrap()["proposal"].clone()
        };
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "body": "a"})).unwrap();
        let qid = propose(&root, &rid, "feature", Some(f["id"].as_str().unwrap()), r#"{"title": "F2", "body": "a"}"#);
        assert_eq!(asked(&qid)["before"], json!({"title": "F", "body": "a"}));
        let qid = propose(&root, &rid, "feature", None, r#"{"title": "New"}"#);
        assert!(asked(&qid).get("before").is_none(), "a create carries none");
        let t = board::task(&root, "t1").unwrap();
        let text = json!({"feature": f["id"].clone(), "dependsOn": []}).to_string();
        let qid = propose(&root, &rid, "task", Some("t1"), &text);
        let before = asked(&qid)["before"].clone();
        assert_eq!((&before["feature"], &before["dependsOn"]), (&t["feature"], &t["dependsOn"]), "{before}");
        let qid = propose(&root, &rid, "vision", None, "For teams");
        let p = asked(&qid);
        assert_eq!((p["before"]["text"].as_str(), p["base"].is_string()), (Some(""), true), "{p}");
    }

    /// D-93: Reject writes no record, task or vision; only the run changes, with
    /// the answer on its notes.
    #[test]
    fn a_rejected_proposal_writes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        let qs = [
            propose(&root, &rid, "feature", None, r#"{"title": "No"}"#),
            propose(&root, &rid, "feature", Some(f["id"].as_str().unwrap()), r#"{"title": "No"}"#),
            propose(&root, &rid, "task", None, r#"{"title": "No"}"#),
            propose(&root, &rid, "vision", None, "No"),
        ];
        let (before, tasks, events) = (records(&root, "features"), board::all_tasks(&root).unwrap(), root_events(&root).len());
        for qid in &qs {
            let (said, note) = answer(&root, &rid, qid, "Reject");
            assert_eq!(said, "Reject");
            assert!(note.starts_with("Proposal: ") && note.ends_with("Answer: Reject"), "{note}");
        }
        assert_eq!(
            (records(&root, "features"), board::all_tasks(&root).unwrap(), vision(&root)["text"].as_str()),
            (before, tasks, Some(""))
        );
        assert!(root_events(&root)[events..].iter().all(|e| e.get("strategyRun").is_some()));
    }
    /// D-122: a feature or milestone link asks for its record's title; an
    /// unknown id is 404, a malformed one 400.
    #[test]
    fn a_record_link_finds_its_title() {
        let root = root();
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "1.0"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        for (kind, r, title) in [("milestones", &m, "1.0"), ("features", &f, "F")] {
            let id = r["id"].as_str().unwrap();
            assert_eq!(find_record(&root, kind, id).unwrap(), json!({"id": id, "title": title}));
            assert_eq!(find_record(&root, kind, "0123456789abcdef").unwrap_err().0, 404);
            assert_eq!(find_record(&root, kind, "../x").unwrap_err().0, 400);
        }
    }

    /// D-93: a vision proposal is applied only over the revision it was made
    /// from; after a human edit it is not applied, and the answer says so.
    #[test]
    fn a_vision_proposal_applies_only_over_its_base() {
        let root = root();
        let rid = page_run(&root);
        let qid = propose(&root, &rid, "vision", None, "For teams");
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        assert_eq!(vision(&root)["text"], "For teams");
        let qid = propose(&root, &rid, "vision", None, "For everyone");
        let base = vision(&root)["revision"].as_str().unwrap().to_string();
        vision_api(&root, "PUT", json!({"text": "Human's", "base": base})).unwrap();
        let (said, note) = answer(&root, &rid, &qid, "Approve");
        assert!(said.starts_with("Not applied: E7026"), "{said}");
        assert!(note.contains(&said), "{note}");
        assert_eq!(vision(&root)["text"], "Human's");
        let e = proposal(&root, &rid, "vision", None, &"é".repeat(1001)).unwrap_err();
        assert!(e.0.starts_with("E7027"), "{}", e.0);
    }

    /// D-93: a task proposal linked to a complete feature and milestone says so,
    /// and sets them `upcoming` only on Approve.
    #[test]
    fn a_task_proposal_reopens_its_complete_feature_on_approve() {
        let root = root();
        let rid = page_run(&root);
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "M", "state": "complete"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "canceled", "milestone": m["id"]})).unwrap();
        let fields = json!({"title": "More", "feature": f["id"]}).to_string();
        let rejected = propose(&root, &rid, "task", None, &fields);
        let q = record(&root, "runs", &rid).unwrap()["questions"][0].clone();
        assert_eq!(q["proposal"]["reopens"], json!([{"kind": "feature", "id": f["id"]}, {"kind": "milestone", "id": m["id"]}]));
        assert!(q["text"].as_str().unwrap().contains("back to upcoming"), "{}", q["text"]);
        answer(&root, &rid, &rejected, "Reject");
        let state = |kind: &str, v: &Value| record(&root, kind, v["id"].as_str().unwrap()).unwrap()["state"].clone();
        assert_eq!((state("features", &f), state("milestones", &m)), (json!("canceled"), json!("complete")));
        let approved = propose(&root, &rid, "task", None, &fields);
        let said = answer(&root, &rid, &approved, "Approve").0;
        assert_eq!((state("features", &f), state("milestones", &m)), (json!("upcoming"), json!("upcoming")));
        let t = board::task(&root, said.rsplit(':').next().unwrap()).unwrap();
        assert_eq!((&t["feature"], &t["milestone"]), (&f["id"], &m["id"]));
    }

    /// D-93, D-134: a reopen refused for one record (its milestone is gone)
    /// leaves the other as it was: every reopen is checked before any is saved.
    #[test]
    fn a_refused_reopen_changes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let m = strategy_api(&root, "POST", &["milestones"], json!({"title": "M", "state": "complete"})).unwrap();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "complete", "milestone": m["id"]})).unwrap();
        let qid = propose(&root, &rid, "task", None, &json!({"title": "More", "feature": f["id"]}).to_string());
        fs::remove_file(strategy_dir(&root, "milestones").join(format!("{}.json", m["id"].as_str().unwrap()))).unwrap();
        let said = answer(&root, &rid, &qid, "Approve").0;
        assert!(said.starts_with("Not applied") && said.contains(m["id"].as_str().unwrap()), "the milestone's reopen is refused: {said}");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("complete"));
        assert!(!board::all_tasks(&root).unwrap().iter().any(|t| t["title"] == "More"));
    }

    /// D-93: an apply the request would refuse writes nothing and puts the code
    /// in the answer: a task whose run started since (E7003), a dependsOn cycle
    /// made since (E7025). An ended run takes no answer.
    #[test]
    fn a_refused_apply_changes_nothing() {
        let root = root();
        let rid = page_run(&root);
        let mut t2 = board::created(&root, &json!({"title": "T2"}), Column::First).unwrap();
        store::write(&root, &t2).unwrap();
        let t2_id = t2["id"].as_str().unwrap().to_string();
        let cycle = propose(&root, &rid, "task", Some("t1"), &json!({"dependsOn": [t2_id]}).to_string());
        let running = propose(&root, &rid, "task", Some(&t2_id), r#"{"title": "Busy"}"#);
        t2["dependsOn"] = json!(["t1"]);
        t2["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        store::write(&root, &t2).unwrap();
        let before = (board::task(&root, "t1").unwrap(), board::task(&root, &t2_id).unwrap());
        for (qid, code) in [(cycle, "E7025"), (running, "E7003")] {
            let (said, note) = answer(&root, &rid, &qid, "Approve");
            assert!(said.starts_with(&format!("Not applied: {code}")), "{said}");
            assert!(note.contains(&said), "{note}");
        }
        assert_eq!((board::task(&root, "t1").unwrap(), board::task(&root, &t2_id).unwrap()), before);
        let late = propose(&root, &rid, "feature", None, r#"{"title": "Late"}"#);
        runs_api(&root, "POST", &[&rid, "end"], json!({}), models).unwrap();
        let e = runs_api(&root, "POST", &[&rid, "questions", &late], json!({"option": "Approve"}), models).unwrap_err();
        assert_eq!((e.0, e.1.0.starts_with("E7003")), (409, true), "{}", e.1.0);
        assert!(records(&root, "features").is_empty());
    }

    /// D-93: a proposal message applied twice asks once and sets the run
    /// waiting; one reaching an ended run is only noted.
    #[test]
    fn a_strategy_proposal_message_applies_once() {
        let root = root();
        let rid = page_run(&root);
        let q = proposal(&root, &rid, "feature", None, r#"{"title": "F"}"#).unwrap();
        let m = json!({"id": "m1", "kind": "strategy_propose", "run": rid, "question": q});
        apply(&root, &m).unwrap();
        apply(&root, &m).unwrap();
        let run = record(&root, "runs", &rid).unwrap();
        assert_eq!((run["questions"].as_array().unwrap().len(), run["state"].as_str()), (1, Some("waiting")));
        assert!(run["questions"][0]["time"].as_u64().is_some_and(|t| t > 0));
        assert_eq!(run["questions"][0]["after"], json!(run["notes"].as_array().unwrap().len()));
        runs_api(&root, "POST", &[&rid, "end"], json!({}), models).unwrap();
        apply(&root, &json!({"id": "m2", "kind": "strategy_propose", "run": rid, "question": q})).unwrap();
        let run = record(&root, "runs", &rid).unwrap();
        assert_eq!((run["questions"].as_array().unwrap().len(), run["state"].as_str()), (1, Some("ended")));
        assert!(run["notes"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().contains("after the run ended"));
    }

    /// D-93: every change is a root event with the run in full; the list leaves
    /// out notes, and one run comes with its transcript.
    #[test]
    fn strategy_run_changes_are_root_events() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| runs_api(&root, m, parts, body, models);
        let run = call("POST", &[], json!({"model": "fake", "text": "x", "context": {"kind": "page"}})).unwrap();
        let rid = run["id"].as_str().unwrap();
        let noted = call("POST", &[rid, "notes"], json!({"text": "y"})).unwrap();
        let ended = call("POST", &[rid, "end"], json!({})).unwrap();
        assert_eq!(root_events(&root), [json!({"strategyRun": run}), json!({"strategyRun": noted}), json!({"strategyRun": ended})]);
        let listed = call("GET", &[], json!({})).unwrap();
        assert_eq!((listed[0]["id"].as_str(), listed[0].get("notes")), (Some(rid), None));
        let one = call("GET", &[rid], json!({})).unwrap();
        assert_eq!((&one["notes"], &one["transcript"]), (&ended["notes"], &json!([])));
    }

    /// D-87: a task saved before the links loads with them empty; one already in a
    /// final column takes its last end time, or its file's, as its closedTime.
    #[test]
    fn an_old_task_loads_with_empty_links_and_a_closed_time() {
        let root = root();
        let t = board::task(&root, "t1").unwrap();
        assert_eq!(
            (&t["feature"], &t["milestone"], &t["dependsOn"], &t["closedTime"], &t["archivedTime"]),
            (&Value::Null, &Value::Null, &json!([]), &Value::Null, &Value::Null)
        );
        let dir = board::store_dir(&root);
        fs::write(dir.join("t2.json"), r#"{"id":"t2","column":"complete","home":".","runs":[{"ended":50}],"finished":{"time":70}}"#)
            .unwrap();
        fs::write(dir.join("t3.json"), r#"{"id":"t3","column":"complete","home":".","runs":[]}"#).unwrap();
        assert_eq!(board::task(&root, "t2").unwrap()["closedTime"], json!(70));
        let modified =
            fs::metadata(dir.join("t3.json")).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(board::task(&root, "t3").unwrap()["closedTime"], json!(modified));
    }

    /// D-130: a task saved before createdTime takes the earlier of its first note's
    /// time and its first run's start, or else its file's.
    #[test]
    fn an_old_task_loads_with_a_created_time() {
        let root = root();
        let dir = board::store_dir(&root);
        let old = [
            ("t2", r#"{"id":"t2","column":"backlog","home":".","notes":[{"time":40},{"time":10}],"runs":[]}"#),
            ("t3", r#"{"id":"t3","column":"backlog","home":".","notes":[],"runs":[{"started":60},{"started":5}]}"#),
            ("t4", r#"{"id":"t4","column":"backlog","home":".","notes":[{"time":40}],"runs":[{"started":30}]}"#),
            ("t5", r#"{"id":"t5","column":"backlog","home":".","notes":[],"runs":[]}"#),
        ];
        for (id, text) in old {
            fs::write(dir.join(format!("{id}.json")), text).unwrap();
        }
        let created = |id: &str| board::task(&root, id).unwrap()["createdTime"].clone();
        assert_eq!((created("t2"), created("t3"), created("t4")), (json!(40), json!(60), json!(30)));
        let modified =
            fs::metadata(dir.join("t5.json")).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(created("t5"), json!(modified));
    }

    /// D-130: a new task records when it was created; cancelling and restoring it keeps the time.
    #[test]
    fn a_new_task_gets_a_created_time_once() {
        let root = root();
        let w = board::flow(&root).unwrap();
        let mut t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let created = t["createdTime"].as_u64().expect("set on create");
        t["createdTime"] = json!(created - 100);
        board::move_to(&w, &mut t, &json!("cancelled")).unwrap();
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["createdTime"], json!(created - 100));
    }

    /// D-87: entering a final column sets closedTime once; later moves keep it.
    #[test]
    fn entering_complete_sets_closed_time_once() {
        let root = root();
        let w = board::flow(&root).unwrap();
        let mut t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        board::move_to(&w, &mut t, &json!("complete")).unwrap();
        let closed = t["closedTime"].as_u64().expect("set on entering complete");
        t["closedTime"] = json!(closed - 100);
        board::move_to(&w, &mut t, &json!("cancelled")).unwrap();
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["closedTime"], json!(closed - 100));
    }

    /// D-95: only a task in a final column is archived, once; leaving the final
    /// columns takes it out of the archive.
    #[test]
    fn archiving_takes_a_final_task_off_the_board() {
        let root = root();
        let archive = || api(&root, "POST", &["tasks", "t1", "archive"], json!({}));
        assert_eq!(&archive().unwrap_err().0[..5], "E7001", "a task in backlog is not archived");
        let w = board::flow(&root).unwrap();
        let mut t = board::task(&root, "t1").unwrap();
        board::move_to(&w, &mut t, &json!("complete")).unwrap();
        store::write(&scope::top(&root), &t).unwrap();
        let t = archive().unwrap();
        let archived = t["archivedTime"].as_u64().expect("set on archiving");
        let mut t = board::task(&root, "t1").unwrap();
        t["archivedTime"] = json!(archived - 100);
        store::write(&scope::top(&root), &t).unwrap();
        assert_eq!(archive().unwrap()["archivedTime"], json!(archived - 100), "archiving again keeps the time");
        board::move_to(&w, &mut t, &json!("backlog")).unwrap();
        assert_eq!(t["archivedTime"], Value::Null);
    }

    /// D-87, D-188: a feature brings its milestone to a task and overrides one
    /// the task had; a featured task's milestone cannot be set. Links are
    /// checked, editable in a final column and during a run, and cleared by a
    /// delete while the milestone stays.
    #[test]
    fn task_links_are_set_checked_and_cleared() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let patch = |tid: &str, body: Value| api(&root, "PATCH", &["tasks", tid], body);
        let code = |r: Result<Value, Failure>| r.unwrap_err().0[..5].to_string();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let t1 = api(&root, "POST", &["tasks"], json!({"title": "T1"})).unwrap();
        let t1 = t1["id"].as_str().unwrap();
        assert_eq!(patch(t1, json!({"milestone": m2["id"]})).unwrap()["milestone"], m2["id"], "a task with no feature owns its milestone");
        let t = patch(t1, json!({"feature": f["id"]})).unwrap();
        assert_eq!((&t["feature"], &t["milestone"]), (&f["id"], &m1["id"]), "a feature brings its milestone and replaces one");
        let refused = patch(t1, json!({"milestone": m2["id"]})).unwrap_err();
        assert!(refused.0.starts_with("E7001") && refused.0.contains("feature's milestone"), "{}", refused.0);
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], m1["id"], "a refused milestone changes nothing");
        let t2 = api(&root, "POST", &["tasks"], json!({"title": "T2"})).unwrap();
        let t2id = t2["id"].as_str().unwrap();
        assert_eq!(code(patch(t2id, json!({"feature": f["id"], "milestone": null}))), "E7001");
        assert!(board::task(&root, t2id).unwrap()["feature"].is_null(), "a refused link changes nothing");
        assert_eq!(patch(t2id, json!({"feature": f["id"]})).unwrap()["milestone"], m1["id"]);
        let bare = call("POST", &["features"], json!({"title": "Bare"}));
        assert!(patch(t2id, json!({"feature": bare["id"]})).unwrap()["milestone"].is_null(), "a feature with no milestone gives none");
        assert_eq!(patch(t2id, json!({"feature": f["id"]})).unwrap()["milestone"], m1["id"]);
        let t = patch(t1, json!({"feature": null})).unwrap();
        assert!(t["feature"].is_null());
        assert_eq!(t["milestone"], m1["id"], "removing a feature keeps the milestone");
        assert_eq!(patch(t1, json!({"milestone": m2["id"]})).unwrap()["milestone"], m2["id"], "the task owns it again");
        assert_eq!(code(patch(t1, json!({"feature": "nope"}))), "E7023");
        assert_eq!(code(patch(t1, json!({"dependsOn": [t1]}))), "E7024");
        assert_eq!(code(patch(t1, json!({"dependsOn": ["nope"]}))), "E7024");
        assert_eq!(patch(t1, json!({"dependsOn": [t2id]})).unwrap()["dependsOn"], json!([t2id]));
        assert_eq!(code(patch(t2id, json!({"dependsOn": [t1]}))), "E7025");
        assert_eq!(board::task(&root, t1).unwrap()["dependsOn"], json!([t2id]));

        // During a run, and in a final column, only the links change; an empty edit is refused too.
        let mut busy = board::task(&root, t2id).unwrap();
        busy["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        store::write(&root, &busy).unwrap();
        assert_eq!(code(patch(t2id, json!({"milestone": m2["id"]}))), "E7001");
        assert_eq!(code(patch(t2id, json!({"title": "no"}))), "E7003");
        assert_eq!(code(patch(t2id, json!({}))), "E7003");
        let mut done = board::task(&root, t2id).unwrap();
        done["runs"] = json!([]);
        done["column"] = json!("complete");
        store::write(&root, &done).unwrap();
        assert_eq!(code(patch(t2id, json!({"milestone": null}))), "E7001");
        assert_eq!(code(patch(t2id, json!({"title": "no", "feature": null}))), "E7004");
        assert_eq!(code(patch(t2id, json!({}))), "E7004");

        call("DELETE", &["features", f["id"].as_str().unwrap()], json!({}));
        for id in [t1, t2id] {
            assert_eq!(board::task(&root, id).unwrap()["feature"], Value::Null);
        }
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], m2["id"], "the milestone stays");
        assert_eq!(board::task(&root, t2id).unwrap()["milestone"], m1["id"]);
        call("DELETE", &["milestones", m2["id"].as_str().unwrap()], json!({}));
        assert_eq!(board::task(&root, t1).unwrap()["milestone"], Value::Null);
        assert_eq!(board::task(&root, t2id).unwrap()["milestone"], m1["id"], "another milestone stays");
    }

    /// D-128: an active feature completes when every task it has is in a final
    /// column, a canceled or Tech Debt task too, and is active again when one
    /// leaves; a feature in another state, or with no tasks, never changes.
    #[test]
    fn a_feature_follows_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let state = |f: &Value| record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"].clone();
        let move_to = |tid: &str, column: &str| {
            let mut t = board::task(&root, tid).unwrap();
            t["column"] = json!(column);
            store::write(&root, &t).unwrap();
        };
        let f = call("POST", &["features"], json!({"title": "F", "state": "active"}));
        let empty = call("POST", &["features"], json!({"title": "Empty", "state": "active"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(state(&f), json!("active"), "an open task keeps it active");
        let mut debt = board::task(&root, "t1").unwrap();
        debt["id"] = json!("t2");
        debt["techDebt"] = json!(true);
        store::write(&root, &debt).unwrap();
        move_to("t1", "complete");
        assert_eq!(state(&f), json!("active"), "an open Tech Debt task counts");
        move_to("t2", "cancelled");
        assert_eq!(state(&f), json!("complete"));
        assert!(!record(&root, "features", f["id"].as_str().unwrap()).unwrap()["startTime"].is_null());
        move_to("t1", "backlog");
        assert_eq!(state(&f), json!("active"), "a reopened task makes it active again");
        move_to("t1", "complete");
        assert_eq!(state(&f), json!("complete"));
        assert_eq!(state(&empty), json!("active"), "no tasks: no change");

        let upcoming = call("POST", &["features"], json!({"title": "U", "state": "upcoming"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": upcoming["id"]})).unwrap();
        assert_eq!(state(&upcoming), json!("upcoming"), "only active and complete features follow");
        assert_eq!(state(&f), json!("complete"), "t2 is still final");
        api(&root, "PATCH", &["tasks", "t2"], json!({"feature": null})).unwrap();
        assert_eq!(state(&f), json!("complete"), "with no tasks left it stays");
    }

    /// D-132, D-134: a feature that becomes active, by an edit or an approved
    /// proposal, moves its Backlog tasks to Planning, each with a note naming
    /// it; tasks in other columns or of other features stay. A task linked to
    /// or proposed in an active feature is planned; in another state it isn't.
    #[test]
    fn an_active_feature_plans_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let column = |tid: &str| board::task(&root, tid).unwrap()["column"].as_str().unwrap().to_string();
        let notes = |tid: &str| {
            board::task(&root, tid).unwrap()["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["text"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let new_task = |title: &str, feature: &Value| {
            let id = api(&root, "POST", &["tasks"], json!({"title": title})).unwrap()["id"].as_str().unwrap().to_string();
            api(&root, "PATCH", &["tasks", &id], json!({"feature": feature})).unwrap();
            id
        };
        let f = call("POST", &["features"], json!({"title": "Search", "state": "upcoming"}));
        let other = call("POST", &["features"], json!({"title": "Other", "state": "upcoming"}));
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(column("t1"), "backlog", "an upcoming feature's task stays in Backlog");
        let busy = new_task("Busy", &f["id"]);
        let mut t = board::task(&root, &busy).unwrap();
        t["column"] = json!("assigned");
        store::write(&root, &t).unwrap();
        let waits = new_task("Waits", &other["id"]);

        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"state": "active"}));
        assert_eq!(column("t1"), "planning");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("active"));
        let moved = notes("t1");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0], format!("Moved to Planning: its feature feature:{} is active.", f["id"].as_str().unwrap()));
        assert_eq!((column(&busy), notes(&busy).len()), ("assigned".to_string(), 0), "only Backlog tasks move");
        assert_eq!(column(&waits), "backlog", "another feature's task stays");
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"title": "Search 2"}));
        assert_eq!(notes("t1").len(), 1, "an edit that keeps it active moves nothing");

        let linked = new_task("Linked", &f["id"]);
        assert_eq!(column(&linked), "planning", "a task linked to an active feature is planned");
        assert_eq!(notes(&linked).len(), 1);

        let rid = page_run(&root);
        let qid = propose(&root, &rid, "task", None, &json!({"title": "Proposed", "feature": f["id"]}).to_string());
        let said = answer(&root, &rid, &qid, "Approve").0;
        let id = said.rsplit(':').next().unwrap();
        assert_eq!(column(id), "planning");
        let n = notes(id);
        assert!(n[0].starts_with("Created by strategy run") && n[1].starts_with("Moved to Planning"), "{n:?}");

        let qid = propose(&root, &rid, "feature", other["id"].as_str(), r#"{"state": "active"}"#);
        assert_eq!(answer(&root, &rid, &qid, "Approve").0, "Approved and applied");
        assert_eq!(column(&waits), "planning", "an approved proposal plans the tasks too");
    }

    /// Writes task `id` straight into the store, a copy of t1 in Backlog under
    /// `feature`, as a file saved before D-134 would be.
    fn stored_backlog_task(root: &Path, id: &str, feature: &Value) {
        let mut t = board::task(root, "t1").unwrap();
        t["id"] = json!(id);
        t["title"] = json!(id);
        t["column"] = json!("backlog");
        t["feature"] = feature.clone();
        fs::write(board::store_dir(root).join(format!("{id}.json")), t.to_string()).unwrap();
    }
    fn column_of(root: &Path, id: &str) -> String {
        board::task(root, id).unwrap()["column"].as_str().unwrap().to_string()
    }
    fn notes_of(root: &Path, id: &str) -> Vec<String> {
        let t = board::task(root, id).unwrap();
        t["notes"].as_array().map_or(vec![], |ns| ns.iter().map(|n| n["text"].as_str().unwrap().to_string()).collect())
    }

    /// D-134: a task already stored in Backlog under an active feature moves to
    /// Planning when the server starts, once, with a note naming the feature;
    /// saving the feature or the task moves one too.
    #[test]
    fn an_existing_backlog_task_under_an_active_feature_is_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "active"})).unwrap();
        let fid = f["id"].as_str().unwrap();
        stored_backlog_task(&root, "old", &f["id"]);
        assert_eq!(column_of(&root, "old"), "backlog");
        plan_active(&root).unwrap();
        assert_eq!(column_of(&root, "old"), "planning");
        let n = notes_of(&root, "old");
        assert_eq!(n.len(), 1, "{n:?}");
        assert!(n[0].contains(&format!("feature:{fid}")), "{n:?}");
        plan_active(&root).unwrap();
        assert_eq!(notes_of(&root, "old").len(), 1, "a second start adds nothing");

        stored_backlog_task(&root, "second", &f["id"]);
        strategy_api(&root, "PATCH", &["features", fid], json!({"title": "Search 2"})).unwrap();
        assert_eq!(column_of(&root, "second"), "planning", "saving the active feature plans it");
        assert_eq!(notes_of(&root, "old").len(), 1);

        stored_backlog_task(&root, "third", &f["id"]);
        api(&root, "PATCH", &["tasks", "third"], json!({"title": "Renamed"})).unwrap();
        assert_eq!(column_of(&root, "third"), "planning", "saving the task plans it");
        assert_eq!(notes_of(&root, "third").len(), 1);
    }

    /// D-134: a Backlog task linked to a complete feature reopens it as
    /// `active` (D-128), and is planned in the same save.
    #[test]
    fn a_task_that_reopens_its_feature_is_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Done", "state": "complete"})).unwrap();
        let before = task_events(&root, "t1");
        api(&root, "PATCH", &["tasks", "t1"], json!({"feature": f["id"]})).unwrap();
        assert_eq!(task_events(&root, "t1"), before + 1, "the save is logged once (D-134)");
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["state"], json!("active"));
        assert_eq!(column_of(&root, "t1"), "planning");
        let n = notes_of(&root, "t1");
        assert!(n.len() == 1 && n[0].contains(&format!("feature:{}", f["id"].as_str().unwrap())), "{n:?}");
        let order: Vec<Value> =
            all_events(&root).into_iter().filter(|e| e["task"]["id"] == "t1" || e["feature"]["id"] == f["id"]).collect();
        assert!(order.last().is_some_and(|e| e["feature"]["state"] == "active"), "the feature is saved after its task");
    }

    /// Every task log entry's data, in `seq` order.
    fn all_events(root: &Path) -> Vec<Value> {
        let rows: Vec<String> = events(root)
            .unwrap()
            .prepare("SELECT data FROM events ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.iter().map(|d| serde_json::from_str(d).unwrap()).collect()
    }
    /// How many task log entries record task `id`.
    fn task_events(root: &Path, id: &str) -> usize {
        all_events(root).iter().filter(|e| e["task"]["id"] == id).count()
    }

    /// An archived task stays where it is, in Backlog under a feature that
    /// becomes active, when the server starts and when it is saved.
    #[test]
    fn an_archived_task_is_not_planned() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "upcoming"})).unwrap();
        stored_backlog_task(&root, "old", &f["id"]);
        let path = board::store_dir(&root).join("old.json");
        let mut t: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        t["archivedTime"] = json!(now());
        fs::write(&path, t.to_string()).unwrap();
        strategy_api(&root, "PATCH", &["features", f["id"].as_str().unwrap()], json!({"state": "active"})).unwrap();
        plan_active(&root).unwrap();
        api(&root, "PATCH", &["tasks", "old"], json!({"title": "Renamed"})).unwrap();
        assert_eq!((column_of(&root, "old"), notes_of(&root, "old").len()), ("backlog".to_string(), 0));
    }

    /// D-134: a task with no feature, or whose feature is not active, stays in
    /// Backlog when the server starts and when it is saved.
    #[test]
    fn a_feature_that_is_not_active_leaves_its_tasks_alone() {
        let root = root();
        let mut ids = vec![("none".to_string(), Value::Null)];
        for state in ["proposed", "upcoming", "complete", "canceled"] {
            let f = strategy_api(&root, "POST", &["features"], json!({"title": state, "state": state})).unwrap();
            ids.push((state.to_string(), f["id"].clone()));
        }
        for (id, f) in &ids {
            stored_backlog_task(&root, id, f);
        }
        plan_active(&root).unwrap();
        for (id, _) in &ids {
            api(&root, "PATCH", &["tasks", id], json!({"title": "Renamed"})).unwrap();
            assert_eq!((column_of(&root, id), notes_of(&root, id).len()), ("backlog".to_string(), 0), "{id}");
        }
    }

    /// D-134: columns are found by id; a workflow without both `backlog` and
    /// `planning` leaves an active feature's task where it is.
    #[test]
    fn a_workflow_without_the_columns_is_left_alone() {
        for columns in [["backlog", "doing"], ["waiting", "planning"]] {
            let root = root();
            let toml = format!(
                concat!(
                    "[board]\nevents = {{ finished = \"{1}\" }}\n[[board.columns]]\nid = \"{0}\"\n",
                    "[[board.columns]]\nid = \"{1}\"\n[[board.columns]]\nid = \"backlog_done\"\nfinal = true\n",
                    "[board.actions]\n",
                ),
                columns[0], columns[1]
            );
            fs::write(root.join("solace.toml"), toml).unwrap();
            let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "active"})).unwrap();
            stored_backlog_task(&root, "old", &f["id"]);
            plan_active(&root).unwrap();
            let mut t = board::task(&root, "old").unwrap();
            t["title"] = json!("Renamed");
            store::write(&root, &t).unwrap();
            assert_eq!((column_of(&root, "old"), notes_of(&root, "old").len()), ("backlog".to_string(), 0), "{columns:?}");
            // A feature that becomes active moves none of its tasks either.
            let g = strategy_api(&root, "POST", &["features"], json!({"title": "Later", "state": "upcoming"})).unwrap();
            stored_backlog_task(&root, "later", &g["id"]);
            strategy_api(&root, "PATCH", &["features", g["id"].as_str().unwrap()], json!({"state": "active"})).unwrap();
            assert_eq!((column_of(&root, "later"), notes_of(&root, "later").len()), ("backlog".to_string(), 0), "{columns:?}");
        }
    }

    /// D-128: a record saved `planned` reads as `upcoming`; `planned` is refused.
    #[test]
    fn planned_reads_as_upcoming() {
        let root = root();
        fs::create_dir_all(strategy_dir(&root, "milestones")).unwrap();
        fs::write(strategy_dir(&root, "milestones").join("m1.json"), r#"{"id":"m1","title":"M","state":"planned"}"#).unwrap();
        assert_eq!(record(&root, "milestones", "m1").unwrap()["state"], json!("upcoming"));
        let refused = strategy_api(&root, "POST", &["features"], json!({"title": "F", "state": "planned"})).unwrap_err();
        assert!(refused.1.0.starts_with("E7001"), "{}", refused.1.0);
    }

    /// An update whose change fails writes nothing.
    #[test]
    fn a_failed_update_writes_nothing() {
        let root = root();
        let before = board::task(&root, "t1").unwrap();
        let e = update(&root, "t1", |t| {
            t["title"] = json!("Changed");
            Err::<(), _>(Failure("refused".into(), 1))
        })
        .unwrap_err();
        assert_eq!(e.0, "refused");
        assert_eq!(board::task(&root, "t1").unwrap(), before);
        assert_eq!(update(&root, "t1", |t| Ok(t["column"].clone())).unwrap(), "backlog");
        assert!(update(&root, "nope", |_| Ok(())).is_err());
    }

    /// A message lands whole, with its id, and ids sort in the order sent.
    #[test]
    fn send_leaves_whole_messages_in_order() {
        let root = root();
        let first = send(&root, json!({"kind": "note", "task": "t1", "text": "a"})).unwrap();
        let second = send(&root, json!({"kind": "note", "task": "t1", "text": "b"})).unwrap();
        assert!(first < second);
        let m: Value = serde_json::from_slice(&fs::read(&first).unwrap()).unwrap();
        assert_eq!(m["id"].as_str(), first.file_stem().and_then(|s| s.to_str()));
        assert_eq!(m["text"], "a");
        let names: Vec<_> = fs::read_dir(inbox(&root)).unwrap().map(|e| e.unwrap().path()).collect();
        assert!(names.iter().all(|p| p.extension().is_some_and(|x| x == "json")), "{names:?}");
        assert!(!rejected(&root, &first));
    }

    /// The server lock tells whether a server runs for the root.
    #[test]
    fn serving_follows_the_server_lock() {
        let root = root();
        assert!(!serving(&root));
        let held = fsx::try_lock_file(&server_lock(&root)).unwrap().unwrap();
        assert!(serving(&root));
        drop(held);
        assert!(!serving(&root));
    }

    /// A message with an older id that lands after a newer one is still applied.
    #[test]
    fn an_older_inbox_message_arriving_late_is_applied() {
        let root = root();
        testing::inbox_file(&root, "0002", json!({"kind":"note","task":"t1","author":"solace","text":"newer"}));
        drain(&root);
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"older"}));
        drain(&root);
        let texts: Vec<String> = board::task(&root, "t1").unwrap()["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(texts, ["newer", "older"]);
        // The same message again (a replay after a crash) is skipped.
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"older"}));
        drain(&root);
        assert_eq!(board::task(&root, "t1").unwrap()["notes"].as_array().unwrap().len(), 2);
    }

    /// A message that can't be applied, or that panics, is set aside and the task is told.
    #[test]
    fn a_message_that_fails_or_panics_is_set_aside() {
        let root = root();
        // No such run: refused.
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":"t1","run":"nope","command":"done","text":"x"}));
        // A task file that lacks its lists (written by hand): applying it panics.
        fs::write(board::store_dir(&root).join("t2.json"), r#"{"id":"t2","column":"backlog","home":"."}"#).unwrap();
        testing::inbox_file(&root, "0002", json!({"kind":"agent","task":"t2","run":"r","command":"note","text":"x"}));
        drain(&root);
        let inbox = inbox(&root);
        assert!(inbox.join("failed/0001.json").exists() && inbox.join("failed/0002.json").exists());
        assert!(!inbox.join("0001.json").exists() && !inbox.join("0002.json").exists());
        assert!(rejected(&root, &inbox.join("0001.json")));
        let t = board::task(&root, "t1").unwrap();
        assert!(t["notes"][0]["text"].as_str().unwrap().contains("set aside"));
        assert_eq!(t["attention"]["reason"], "a task message was set aside");
    }

    /// Telling the task that a message was set aside happens only when the task
    /// can be read and written; every other case leaves the store as it was.
    #[test]
    fn flagging_a_set_aside_message_touches_only_a_readable_task() {
        let root = root();
        let dir = board::store_dir(&root);
        fs::write(dir.join("junk.json"), "not json").unwrap();
        fs::write(dir.join("bare.json"), r#"{"id":"bare","column":"backlog","home":"."}"#).unwrap();
        let before = |names: &[&str]| -> Vec<Vec<u8>> { names.iter().map(|n| fs::read(dir.join(format!("{n}.json"))).unwrap()).collect() };
        let others = ["junk", "bare", "t1"];
        let kept = before(&others);
        let note = |tid: Value| json!({"kind": "note", "task": tid, "text": "x"});
        for m in [note(json!("../x")), note(json!("")), note(json!(null)), note(json!("missing")), note(json!("junk")), note(json!("bare"))]
        {
            flag_set_aside(&root, &m, "why");
        }
        // The flow can't be read while the settings are broken.
        fs::write(root.join("solace.toml"), "[board\n").unwrap();
        flag_set_aside(&root, &note(json!("t1")), "why");
        fs::remove_file(root.join("solace.toml")).unwrap();
        assert_eq!(before(&others), kept, "an invalid id, a missing or unparseable file, no notes list, or no flow: nothing is written");
        flag_set_aside(&root, &json!({"kind": "agent", "task": "t1"}), "the reason");
        let t = board::task(&root, "t1").unwrap();
        assert_eq!(t["notes"].as_array().unwrap().len(), 1);
        let text = t["notes"][0]["text"].as_str().unwrap();
        assert!(text.contains("(agent)") && text.contains("the reason"), "{text}");
        assert_eq!(t["notes"][0]["author"], "solace");
        // A message with no kind is named with an empty one.
        flag_set_aside(&root, &json!({"task": "t1"}), "again");
        assert!(board::task(&root, "t1").unwrap()["notes"][1]["text"].as_str().unwrap().contains("()"));
    }

    /// A task file that doesn't parse sets its message aside, as a panic does.
    #[test]
    fn a_message_for_an_unparseable_task_file_is_set_aside() {
        let root = root();
        fs::write(board::store_dir(&root).join("t9.json"), "{ not json").unwrap();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t9","author":"solace","text":"x"}));
        drain(&root);
        assert!(rejected(&root, &inbox(&root).join("0001.json")));
        assert_eq!(fs::read_to_string(board::store_dir(&root).join("t9.json")).unwrap(), "{ not json");
        // A message for a task that doesn't exist fails the same way.
        testing::inbox_file(&root, "0002", json!({"kind":"note","task":"nope","author":"solace","text":"x"}));
        drain(&root);
        assert!(rejected(&root, &inbox(&root).join("0002.json")));
    }

    /// Only `.json` files in the inbox are messages.
    #[test]
    fn drain_leaves_files_that_are_not_json() {
        let root = root();
        let dir = inbox(&root);
        fs::create_dir_all(&dir).unwrap();
        for name in ["0001.tmp", "notes.txt", "0002"] {
            fs::write(dir.join(name), r#"{"kind":"note","task":"t1","author":"solace","text":"x"}"#).unwrap();
        }
        drain(&root);
        assert!(board::task(&root, "t1").unwrap()["notes"].as_array().is_none_or(Vec::is_empty));
        for name in ["0001.tmp", "notes.txt", "0002"] {
            assert!(dir.join(name).exists(), "{name}");
        }
        assert!(!failed_dir(&root).exists());
    }

    /// The first message in the root's inbox, waited for at most 10 s: a test
    /// that waits for one that never comes fails, it doesn't hang the run.
    fn next_message(top: &Path) -> PathBuf {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(f) = fs::read_dir(inbox(top))
                .ok()
                .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "json")))
            {
                return f;
            }
            assert!(std::time::Instant::now() < end, "no message reached the inbox");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs `f` on a thread and waits at most 10 s for its result: a call that
    /// would wait forever fails the test instead of hanging the run (D-83).
    fn within<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        let (done, result) = mpsc::channel();
        std::thread::spawn(move || drop(done.send(f())));
        result.recv_timeout(Duration::from_secs(10)).expect("the call did not return within 10 s")
    }

    /// A sender where another process serves waits for that server's answer: a
    /// message it took returns Ok, one it set aside is an error, and a server
    /// that stops first leaves the message queued.
    #[test]
    fn deliver_hands_the_message_to_a_server_elsewhere() {
        // Taken: the "server" (another holder of the lock) removes the message.
        let root = root();
        let held = fsx::try_lock_file(&server_lock(&root)).unwrap().unwrap();
        let top = root.to_path_buf();
        let taker = std::thread::spawn(move || fs::remove_file(next_message(&top)).unwrap());
        let to = root.to_path_buf();
        within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"x"}))).unwrap();
        taker.join().unwrap();
        // Set aside: the "server" moves it to failed/.
        let top = root.to_path_buf();
        let rejecter = std::thread::spawn(move || {
            let file = next_message(&top);
            fs::create_dir_all(failed_dir(&top)).unwrap();
            fs::rename(&file, failed_dir(&top).join(file.file_name().unwrap())).unwrap();
        });
        let to = root.to_path_buf();
        let e = within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"y"}))).unwrap_err();
        rejecter.join().unwrap();
        assert!(e.0.contains("set aside"), "{}", e.0);
        // Gone: the lock is released and the message stays in the inbox, undelivered.
        let top = root.to_path_buf();
        let stopper = std::thread::spawn(move || {
            next_message(&top);
            drop(held);
        });
        let to = root.to_path_buf();
        within(move || deliver(&to, json!({"kind":"note","task":"t1","author":"solace","text":"z"}))).unwrap();
        stopper.join().unwrap();
        assert_eq!(fs::read_dir(inbox(&root)).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file()).count(), 1);
        assert!(board::task(&root, "t1").unwrap()["notes"].as_array().is_none_or(Vec::is_empty));
    }

    /// A brief failure (`solace.toml` mid-save) is tried again: the message is
    /// applied, not set aside.
    #[test]
    fn drain_retries_a_brief_failure() {
        let root = root();
        fs::write(root.join("solace.toml"), "[board\n").unwrap();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"later"}));
        let settings = root.join("solace.toml");
        let fixer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            fs::remove_file(settings).unwrap();
        });
        drain(&root);
        fixer.join().unwrap();
        assert_eq!(board::task(&root, "t1").unwrap()["notes"][0]["text"], "later");
        assert!(!inbox(&root).join("0001.json").exists());
        assert!(!failed_dir(&root).exists(), "nothing was set aside");
    }

    /// Without a place to set it aside, a failed message is logged and removed,
    /// so the inbox never sticks on it.
    #[test]
    fn a_failed_message_with_nowhere_to_go_is_dropped() {
        let root = root();
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":"t1","run":"nope","command":"done","text":"x"}));
        fs::write(failed_dir(&root), "not a folder").unwrap();
        drain(&root);
        assert!(!inbox(&root).join("0001.json").exists());
        assert!(failed_dir(&root).is_file());
        assert!(!rejected(&root, &inbox(&root).join("0001.json")));
    }

    /// A sender waits for as long as a server lives, not for a fixed time, and
    /// gives up at once when the server stops.
    #[test]
    fn applied_waits_while_a_server_holds_the_lock() {
        let root = root();
        let held = fsx::try_lock_file(&server_lock(&root)).unwrap().unwrap();
        let sent = send(&root, json!({"kind":"note","task":"t1","text":"x"})).unwrap();
        let (gone, start) = (sent.clone(), std::time::Instant::now());
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            fs::remove_file(gone).unwrap();
        });
        let (top, file) = (root.to_path_buf(), sent.clone());
        assert!(within(move || applied(&top, &file)));
        assert!(start.elapsed() >= Duration::from_millis(350));
        remover.join().unwrap();
        // The server stops with the message still there: false, and the message stays.
        let sent = send(&root, json!({"kind":"note","task":"t1","text":"y"})).unwrap();
        drop(held);
        let (top, file) = (root.to_path_buf(), sent.clone());
        assert!(!within(move || applied(&top, &file)) && sent.exists());
    }

    /// A message delivered where no server runs lands after the earlier ones in
    /// the inbox: a create a stopped server left must not be sent again.
    #[test]
    fn deliver_applies_the_inbox_before_its_message() {
        let root = root();
        let mut t = board::new_task("Late", "", &json!("backlog"), 1.);
        t["home"] = json!(".");
        let tid = t["id"].as_str().unwrap().to_string();
        testing::inbox_file(&root, "0001", json!({"kind":"create","new":t}));
        deliver(&root, json!({"kind":"note","task":tid,"author":"solace","text":"hi"})).unwrap();
        assert_eq!(board::task(&root, &tid).unwrap()["notes"].as_array().unwrap().len(), 1);
        assert!(!inbox(&root).join("0001.json").exists());
    }

    /// A task id names a file, so it has only letters, digits and dashes.
    #[test]
    fn valid_id_takes_only_letters_digits_and_dashes() {
        for good in ["t1", "ab-12", "A-z-9"] {
            assert!(board::valid_id(good), "{good}");
        }
        for bad in ["", "../x", "a/b", "a b", "a.b", "é"] {
            assert!(!board::valid_id(bad), "{bad}");
        }
    }

    /// A message for a task id that could leave the store is refused, and
    /// nothing outside the store is read into it.
    #[test]
    fn a_message_for_an_invalid_task_id_is_refused() {
        let root = root();
        fs::write(root.join(".solace/x.json"), r#"{"id":"x","column":"backlog","home":".","notes":[]}"#).unwrap();
        let before = stored(&root);
        for bad in ["", "../x", "a/b"] {
            let e = apply(&root, &json!({"id": "m1", "kind": "note", "task": bad, "author": "solace", "text": "x"})).unwrap_err();
            assert!(e.0.contains("E7001") && e.0.contains("invalid task id"), "{bad}: {}", e.0);
        }
        assert_eq!(stored(&root), before);
    }

    fn always(_: &Path, _: &[&str], _: &Value) -> Result<bool, Failure> {
        Ok(true)
    }

    /// Without the locks, a change that now needs them changes nothing; with
    /// them, it applies.
    #[test]
    fn a_change_that_now_needs_the_locks_waits_for_them() {
        let root = root();
        let before = board::task(&root, "t1").unwrap();
        let patch = json!({"title": "Renamed"});
        assert!(change(&root, "PATCH", &["tasks", "t1"], &patch, false, always).unwrap().is_none());
        assert_eq!(board::task(&root, "t1").unwrap(), before);
        let (t, _) = change(&root, "PATCH", &["tasks", "t1"], &patch, true, always).unwrap().unwrap();
        assert_eq!(t["title"], "Renamed");
        assert_eq!(board::task(&root, "t1").unwrap()["title"], "Renamed");
    }

    /// A change that needed no locks at the first check but needs them inside
    /// the operation takes them and runs again, applying once.
    #[test]
    fn api_retries_a_change_that_came_to_need_the_locks() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn later(_: &Path, _: &[&str], _: &Value) -> Result<bool, Failure> {
            Ok(CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0)
        }
        let root = root();
        let t = api_with(&root, "POST", &["tasks", "t1", "notes"], json!({"text": "once"}), later).unwrap();
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 2, "the first check, then the one inside the operation");
        let notes = board::task(&root, "t1").unwrap()["notes"].clone();
        assert_eq!(notes.as_array().unwrap().len(), 1);
        assert_eq!(t["notes"], notes);
    }

    /// A tick applies the inbox before the operation it runs, and an operation
    /// that panics leaves the server running.
    #[test]
    fn a_tick_applies_the_inbox_first_and_survives_a_panic() {
        let root = root();
        let (jobs, queue) = mpsc::channel::<Job>();
        let mut held = None;
        let (seen, notes) = mpsc::channel();
        let again = seen.clone();
        testing::inbox_file(&root, "0001", json!({"kind":"note","task":"t1","author":"solace","text":"first"}));
        let r = root.to_path_buf();
        jobs.send(Box::new(move || seen.send(board::task(&r, "t1").unwrap()["notes"].as_array().unwrap().len()).unwrap())).unwrap();
        tick(&root, &mut held, &queue);
        assert_eq!(notes.try_recv().unwrap(), 1);
        jobs.send(Box::new(|| panic!("a failing operation"))).unwrap();
        tick(&root, &mut held, &queue);
        jobs.send(Box::new(move || again.send(0).unwrap())).unwrap();
        tick(&root, &mut held, &queue);
        assert_eq!(notes.try_recv().unwrap(), 0);
    }

    /// A root listing project `sub`: t1 at home in the root, scoped to both, with
    /// its folder in the root's board.
    fn workspace() -> testing::Scratch {
        let root = testing::Scratch::new().with(&[
            ("solace.toml", "projects = [\"sub\"]\n"),
            (".solace/board/t1.json", r#"{"id":"t1","column":"backlog","home":".","runs":[]}"#),
        ]);
        fs::create_dir_all(root.join("sub/.solace")).unwrap();
        board::adopt_tasks(&root, &root).unwrap();
        fs::create_dir_all(root.join(".solace/board/t1")).unwrap();
        root
    }

    /// D-61: a home leaving the scope moves the task's folder to the first project
    /// left, keeps the task in the root's store, and logs the project it left.
    #[test]
    fn a_task_whose_home_leaves_its_scope_is_logged_as_removed() {
        let root = workspace();
        let t = api(&root, "POST", &["tasks", "t1", "scope"], json!({"scope": ["sub"]})).unwrap();
        assert_eq!(t["home"], "sub");
        assert_eq!(board::task(&root.join("sub"), "t1").unwrap()["home"], "sub", "the task stays in the root's store");
        assert!(board::task(&root, "t1").is_err(), "and is no longer a task of the root's own project");
        assert!(!root.join(".solace/board/t1").exists(), "the folder leaves its old home");
        assert!(root.join("sub/.solace/board/t1").exists(), "the folder moves to the new home");
        let db = events(&root).unwrap();
        let rows: Vec<(String, String)> = db
            .prepare("SELECT project, data FROM events ORDER BY seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(rows.iter().any(|(p, d)| p == "." && d.contains(r#""removed":"t1"#)), "{rows:?}");
    }

    /// D-129 and D-160: while the root is no project a task with no project in its
    /// scope keeps none: it is listed marked `noProject`, opens from the root, is
    /// found with `project` false, and settles without a store at its home. A change
    /// makes the root a project, which re-homes the task to the root with a note,
    /// and its scope saves from there.
    #[test]
    fn a_task_with_no_project_shows_at_the_root() {
        // A root that lists `sub` and has no store of its own: the root is no project.
        let root = testing::Scratch::new().with(&[("solace.toml", "projects = [\"sub\"]\n"), ("sub/.solace/keep", "")]);
        let mut t = board::new_task("Orphan", "", &json!("backlog"), 1.);
        t["home"] = json!("web");
        t["scope"] = json!(["web"]);
        store::write(&root, &t).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let listed = board::all_tasks(&root).unwrap().into_iter().find(|x| x["id"] == tid).unwrap();
        assert_eq!(listed["noProject"], json!(true), "listed marked");
        assert!(!board::tasks(&root).unwrap().iter().any(|x| x["id"] == tid), "in no lane");
        assert_eq!(board::task(&root, &tid).unwrap()["noProject"], json!(true), "opens from the root");
        let found = find_task(&root, &tid).unwrap();
        assert_eq!((&found["dir"], &found["project"], &found["onBoard"]), (&json!("web"), &json!(false), &json!(false)));
        board::settle_final(&root, &tid).unwrap();
        assert!(!root.join("web").join(".solace").exists(), "settling settles no store at its home");
        // The plan opens the root's store, so the root is a project and the task falls back to it.
        let t = api(&root, "POST", &["tasks", &tid, "actions"], json!({"action": "plan"})).unwrap();
        assert_eq!((t["column"].as_str(), t["home"].as_str()), (Some("planning"), Some(".")), "planned at the root");
        assert_eq!(t["scope"], json!(["."]), "scoped to the root");
        assert!(
            t["notes"].as_array().unwrap().iter().any(|n| n["text"].as_str().is_some_and(|x| x.contains("Moved from `web` to `.`"))),
            "the move is noted: {:?}",
            t["notes"]
        );
        assert_eq!(api(&root, "POST", &["tasks", &tid, "scope"], json!({"scope": ["sub"]})).unwrap()["home"], "sub", "its scope saves");
        let sub = root.join("sub");
        let t = api(&sub, "POST", &["tasks", &tid, "actions"], json!({"action": "cancel"})).unwrap();
        assert_eq!(t["column"], "cancelled");
        let t = api(&sub, "POST", &["tasks", &tid, "archive"], json!({})).unwrap();
        assert!(!t["archivedTime"].is_null(), "archived");
        assert!(!root.join("web").join(".solace").exists(), "no store at its old home");
    }

    /// A workflow whose `finished` event targets a final column, so `task done` settles.
    const DONE_IS_FINAL: &str = concat!(
        "[board]\nevents = { asked = \"todo\", finished = \"done\", failed = \"todo\" }\n",
        "[[board.columns]]\nid = \"todo\"\nactions = [\"cancel\"]\n",
        "[[board.columns]]\nid = \"done\"\nfinal = true\n",
        "[[board.columns]]\nid = \"gone\"\nfinal = true\n",
        "[board.actions]\ncancel = { revert = true, to = \"gone\" }\n",
    );

    /// One server per process, for one root. `start` runs it on its own thread; `op` sends its
    /// operations there; a second `start` says it is already running; and a `done`
    /// the server applies settles on a thread of its own (D-77 point 5).
    #[test]
    fn the_task_server_runs_its_operations_on_its_own_thread() {
        let (_serial, root) = served_root();
        fs::write(root.join("solace.toml"), DONE_IS_FINAL).unwrap();
        let ran_on = op(&root, move || Ok(std::thread::current().id())).unwrap();
        assert_eq!(ran_on, SERVER.get().unwrap().thread, "the operation ran on the server's thread");
        assert_ne!(ran_on, std::thread::current().id());
        // Another root has no server here: it isn't served and its operations run on the caller.
        let other = testing::Scratch::new();
        assert!(!serving(&other));
        let here = op(&other, move || Ok(std::thread::current().id())).unwrap();
        assert_eq!(here, std::thread::current().id());
        let e = start(&root).unwrap_err();
        assert!(e.0.contains("the task server is already running"), "{}", e.0);
        // A `done` the server applies moves the task and settles it off the server's thread.
        let t = api(&root, "POST", &["tasks"], json!({"title": "Shapes"})).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let mut t = t;
        t["runs"] = json!([{"id": "r1", "kind": "work", "state": "running"}]);
        store::write(&root, &t).unwrap();
        let s = board::open(&root).unwrap();
        let before = board::snapshot(&s).unwrap();
        s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
        board::record(&s, &tid, "r1", &before, &board::snapshot(&s).unwrap(), None).unwrap();
        let has_before =
            || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
        assert!(has_before(), "the change is recorded");
        testing::inbox_file(&root, "0001", json!({"kind":"agent","task":tid,"run":"r1","command":"done","text":"done"}));
        let settled = wait_until(|| board::task(&root, &tid).unwrap()["column"] == "done" && !has_before());
        assert!(settled, "the server applied the done and settled the task off its own thread");
    }

    /// D-188: changing a feature's milestone moves every task linked to it,
    /// including a closed and an archived one; clearing it moves them to none.
    #[test]
    fn a_feature_milestone_moves_all_its_tasks() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let patch = |tid: &str, body: Value| api(&root, "PATCH", &["tasks", tid], body);
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let mut ids = vec![];
        for _ in 0..3 {
            let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
            let id = t["id"].as_str().unwrap().to_string();
            patch(&id, json!({"feature": f["id"]})).unwrap();
            ids.push(id);
        }
        let (open, closed, archived) = (&ids[0], &ids[1], &ids[2]);
        let mut done = board::task(&root, closed).unwrap();
        done["column"] = json!("complete");
        store::write(&root, &done).unwrap();
        let mut gone = board::task(&root, archived).unwrap();
        gone["column"] = json!("complete");
        gone["archivedTime"] = json!(1);
        store::write(&root, &gone).unwrap();

        let raw =
            |id: &str| -> Value { serde_json::from_slice(&fs::read(board::store_dir(&root).join(format!("{id}.json"))).unwrap()).unwrap() };
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": m2["id"]}));
        for id in [open, closed, archived] {
            assert_eq!(raw(id)["milestone"], m2["id"], "{id}");
        }
        call("PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": null}));
        for id in [open, closed, archived] {
            assert!(raw(id)["milestone"].is_null(), "{id}");
        }
    }

    /// D-188: a task saved with a feature takes that feature's milestone when
    /// loaded, including an archived task and one whose saved milestone differs.
    #[test]
    fn an_old_task_takes_its_features_milestone() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let dir = board::store_dir(&root);
        for (id, saved, archived) in [("t8", Value::Null, Value::Null), ("t9", m2["id"].clone(), json!(1))] {
            let t = json!({"id": id, "column": "planning", "home": ".", "feature": f["id"], "milestone": saved,
                "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": archived, "notes": [], "runs": []});
            fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&t).unwrap()).unwrap();
        }
        let t = json!({"id": "t10", "column": "planning", "home": ".", "feature": null, "milestone": m2["id"],
            "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": null, "notes": [], "runs": []});
        fs::write(dir.join("t10.json"), serde_json::to_vec(&t).unwrap()).unwrap();
        for id in ["t8", "t9"] {
            assert_eq!(board::task(&root, id).unwrap()["milestone"], m1["id"], "{id}");
        }
        assert_eq!(board::task(&root, "t10").unwrap()["milestone"], m2["id"], "a task with no feature keeps its own");
    }

    /// D-188: a cascade that cannot write one task puts every task back and
    /// leaves the feature on its old milestone.
    #[test]
    fn a_failed_milestone_cascade_changes_no_task() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let (m1, m2) = (call("POST", &["milestones"], json!({"title": "1"})), call("POST", &["milestones"], json!({"title": "2"})));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m1["id"]}));
        let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let id = t["id"].as_str().unwrap().to_string();
        api(&root, "PATCH", &["tasks", id.as_str()], json!({"feature": f["id"]})).unwrap();
        let dir = board::store_dir(&root);
        let bad = json!({"id": "bad/id", "column": "backlog", "home": ".", "feature": f["id"], "milestone": m1["id"], "rank": 2.0,
            "dependsOn": [], "createdTime": 1, "closedTime": null, "archivedTime": null, "notes": [], "runs": []});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = task_events(&root, &id);
        let e = strategy_api(&root, "PATCH", &["features", f["id"].as_str().unwrap()], json!({"milestone": m2["id"]})).unwrap_err();
        assert!(e.1.0.contains("invalid task id"), "{}", e.1.0);
        let raw: Value = serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(raw["milestone"], m1["id"]);
        assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["milestone"], m1["id"]);
        assert_eq!(task_events(&root, &id), before, "the failed cascade logs no task save");
    }

    /// Deleting a milestone clears its features and writes each linked task once.
    #[test]
    fn deleting_a_milestone_writes_each_task_once() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let f = call("POST", &["features"], json!({"title": "F", "milestone": m["id"]}));
        let t = api(&root, "POST", &["tasks"], json!({"title": "T"})).unwrap();
        let id = t["id"].as_str().unwrap().to_string();
        api(&root, "PATCH", &["tasks", id.as_str()], json!({"feature": f["id"]})).unwrap();
        let before = task_events(&root, &id);
        call("DELETE", &["milestones", m["id"].as_str().unwrap()], json!({}));
        assert_eq!(task_events(&root, &id), before + 1);
        let raw: Value = serde_json::from_slice(&fs::read(board::store_dir(&root).join(format!("{id}.json"))).unwrap()).unwrap();
        assert!(raw["milestone"].is_null());
    }

    /// D-87: a feature save that fails while deleting a milestone puts back
    /// the features already cleared, and logs none of them.
    #[test]
    fn a_failed_milestone_delete_restores_its_features() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let f1 = call("POST", &["features"], json!({"title": "First", "milestone": m["id"]}));
        let f2 = call("POST", &["features"], json!({"title": "Second", "milestone": m["id"]}));
        let dir = strategy_dir(&root, "features");
        let bad = json!({"id": "bad/id", "title": "Bad", "rank": 3.0, "milestone": m["id"], "state": "upcoming",
            "startTime": null, "colorSlot": null});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = root_events(&root).len();
        let e = strategy_api(&root, "DELETE", &["milestones", m["id"].as_str().unwrap()], json!({})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        for f in [&f1, &f2] {
            assert_eq!(record(&root, "features", f["id"].as_str().unwrap()).unwrap()["milestone"], m["id"]);
        }
        assert_eq!(record(&root, "milestones", m["id"].as_str().unwrap()).unwrap()["title"], json!("M"));
        assert_eq!(root_events(&root).len(), before, "the failed delete logs nothing");
    }

    /// A slot fix belongs to the batch that writes the features: a failed
    /// cascade puts it back instead of leaving it saved.
    #[test]
    fn a_failed_feature_batch_puts_back_a_kept_slot() {
        let root = root();
        let call = |m: &str, parts: &[&str], body: Value| strategy_api(&root, m, parts, body).unwrap();
        let m = call("POST", &["milestones"], json!({"title": "M"}));
        let dir = strategy_dir(&root, "features");
        fs::create_dir_all(&dir).unwrap();
        let old = json!({"id": "old", "title": "Old", "state": "active", "rank": 1, "startTime": 10,
            "milestone": m["id"]});
        fs::write(dir.join("old.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        let bad = json!({"id": "bad/id", "title": "Bad", "rank": 2, "milestone": m["id"], "state": "upcoming",
            "startTime": null});
        fs::write(dir.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        let e = strategy_api(&root, "DELETE", &["milestones", m["id"].as_str().unwrap()], json!({})).unwrap_err();
        assert!(e.1.0.starts_with("E7001"), "{}", e.1.0);
        let saved: Value = serde_json::from_slice(&fs::read(dir.join("old.json")).unwrap()).unwrap();
        assert!(saved.get("colorSlot").is_none(), "the kept slot went back with the batch");
        assert_eq!(saved["milestone"], m["id"]);
        assert!(task_journals(&root).is_empty(), "no journal is left: {:?}", task_journals(&root));
    }

    /// A crash partway through a batch leaves its journal, and recovery puts
    /// back the files written so far.
    #[test]
    fn a_crashed_task_batch_is_put_back() {
        let root = root();
        let dir = board::store_dir(&root);
        let before = fs::read(dir.join("t1.json")).unwrap();
        fs::write(dir.join("t1.json"), serde_json::to_vec(&json!({"id": "t1", "column": "planning", "home": "."})).unwrap()).unwrap();
        fs::write(dir.join("t2.json"), serde_json::to_vec(&json!({"id": "t2", "column": "planning", "home": "."})).unwrap()).unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let record = json!({"files": [
            {"path": ".solace/tasks/t1.json", "bytes": before.clone()},
            {"path": ".solace/tasks/t2.json", "bytes": null}
        ]});
        fs::write(pending.join("tasks-crash.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        recover(&root).unwrap();
        assert_eq!(fs::read(dir.join("t1.json")).unwrap(), before);
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-crash.json").exists());
    }

    /// A batch that finishes drops its journal.
    #[test]
    fn a_successful_batch_leaves_no_journal() {
        let root = root();
        let mut t = board::task(&root, "t1").unwrap();
        t["title"] = json!("Changed");
        store::write(&root, &t).unwrap();
        assert_eq!(board::task(&root, "t1").unwrap()["title"], json!("Changed"));
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    /// Creating a task through the API leaves its file and no journal.
    #[test]
    fn a_created_task_is_on_disk() {
        let root = root();
        strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        assert!(task_journals(&root).is_empty(), "the feature batch is committed: {:?}", task_journals(&root));
        let t = api(&root, "POST", &["tasks"], json!({"title": "Created"})).unwrap();
        let id = t["id"].as_str().unwrap();
        assert!(board::store_dir(&root).join(format!("{id}.json")).exists());
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    fn task_journals(root: &std::path::Path) -> Vec<String> {
        fs::read_dir(root.join(".solace/pending"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("tasks-"))
            .collect()
    }

    /// A journal whose batch holds its file lock belongs to a running process.
    #[test]
    fn a_live_journal_is_not_put_back() {
        let root = root();
        let dir = board::store_dir(&root);
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        fs::write(dir.join("t2.json"), b"{}").unwrap();
        let record = json!({"files": [{"path": ".solace/tasks/t2.json", "bytes": null}]});
        fs::write(pending.join("tasks-live.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        let owner = fsx::try_lock_file(&pending.join("tasks-live.lock")).unwrap().unwrap();
        recover(&root).unwrap();
        assert!(dir.join("t2.json").exists());
        assert!(pending.join("tasks-live.json").exists());
        drop(owner);
        // A process spawned by a parallel test may hold the lock until it execs.
        for _ in 0..100 {
            recover(&root).unwrap();
            if !dir.join("t2.json").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-live.json").exists());
    }

    #[test]
    fn a_journal_path_outside_the_root_is_refused() {
        let root = root();
        let outside = root.parent().unwrap().join("journal-outside.txt");
        fs::write(&outside, b"keep").unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let record = json!({"files": [{"path": "../journal-outside.txt", "bytes": null}]});
        fs::write(pending.join("tasks-escape.json"), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(recover(&root).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        fs::remove_file(&outside).unwrap();
    }

    /// Recovery acts only on a journal; a lock file without one belongs to no
    /// batch it can finish, so it is left alone.
    #[test]
    fn a_lock_without_a_journal_is_left_alone() {
        let root = root();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let lock = pending.join("tasks-orphan.lock");
        fs::write(&lock, b"").unwrap();
        recover(&root).unwrap();
        assert!(lock.exists());
    }

    /// Each newly kept file appends one journal line instead of rewriting the
    /// whole journal.
    #[test]
    fn a_batch_journals_each_file_as_one_line() {
        let root = root();
        let dir = board::store_dir(&root);
        let t2 = dir.join("t2.json");
        fs::write(&t2, b"{}").unwrap();
        let batch = batch(&root).unwrap();
        batch_keep(&dir.join("t1.json")).unwrap();
        batch_keep(&t2).unwrap();
        assert_eq!(fs::read_to_string(journal_path()).unwrap().lines().count(), 2);
        batch_rollback(batch);
    }

    /// Appended journal lines restore in reverse; a last line that was never
    /// finished names no change.
    #[test]
    fn an_appended_journal_puts_back_every_file() {
        let root = root();
        let dir = board::store_dir(&root);
        let before = fs::read(dir.join("t1.json")).unwrap();
        fs::write(dir.join("t1.json"), b"{}").unwrap();
        fs::write(dir.join("t2.json"), b"{}").unwrap();
        let pending = root.join(".solace/pending");
        fs::create_dir_all(&pending).unwrap();
        let mut journal =
            [json!({"path": ".solace/tasks/t1.json", "bytes": before.clone()}), json!({"path": ".solace/tasks/t2.json", "bytes": null})]
                .iter()
                .map(|entry| serde_json::to_string(entry).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
        journal.push_str("\n{\"path\":");
        fs::write(pending.join("tasks-lines.json"), journal).unwrap();
        recover(&root).unwrap();
        assert_eq!(fs::read(dir.join("t1.json")).unwrap(), before);
        assert!(!dir.join("t2.json").exists());
        assert!(!pending.join("tasks-lines.json").exists());
    }

    /// Queued events are logged after the files commit, and a rollback drops
    /// them.
    #[test]
    fn a_batch_logs_only_what_it_commits() {
        let root = root();
        let before = root_events(&root).len();
        let dropped = batch(&root).unwrap();
        batch_log("", &json!({"dropped": true}));
        batch_rollback(dropped);
        assert_eq!(root_events(&root).len(), before);
        let kept = batch(&root).unwrap();
        batch_log("", &json!({"kept": true}));
        batch_commit(kept).unwrap();
        assert_eq!(root_events(&root).last().unwrap(), &json!({"kept": true}));
    }

    /// A nested batch shares the outer journal and events, so an outer rollback
    /// puts back its files and logs none of its events.
    #[test]
    fn a_nested_batch_rolls_back_with_the_outer() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let journals = task_journals(&root);
        let inner = batch(&root).unwrap();
        assert_eq!(task_journals(&root), journals);
        batch_replace(&kept, &json!({"id": "t2"})).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_commit(inner).unwrap();
        assert_eq!(root_events(&root).len(), before, "a nested commit logs nothing yet");
        batch_rollback(outer);
        assert!(!kept.exists(), "the outer rollback puts back the nested file");
        assert_eq!(root_events(&root).len(), before, "the outer rollback logs no nested event");
    }

    /// A nested batch queues its events for the outer commit.
    #[test]
    fn a_nested_batch_logs_when_the_outer_commits() {
        let root = root();
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let inner = batch(&root).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_commit(inner).unwrap();
        assert_eq!(root_events(&root).len(), before);
        batch_commit(outer).unwrap();
        assert_eq!(root_events(&root).last(), Some(&json!({"nested": true})));
    }

    /// A nested rollback puts back the outer files and keeps the outer from
    /// logging anything.
    #[test]
    fn a_nested_batch_failure_stops_the_outer_commit() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let outer = batch(&root).unwrap();
        let inner = batch(&root).unwrap();
        batch_replace(&kept, &json!({"id": "t2"})).unwrap();
        batch_log("", &json!({"nested": true}));
        batch_rollback(inner);
        assert!(batch_commit(outer).is_err());
        assert!(!kept.exists());
        assert_eq!(root_events(&root).len(), before);
    }

    /// A feature follow-up that cannot be saved fails the batch, so the task
    /// and its event are put back too.
    #[test]
    fn a_feature_that_cannot_follow_its_task_rolls_back() {
        let root = root();
        let task = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        let batch = batch(&root).unwrap();
        batch_replace(&task, &json!({"id": "t2"})).unwrap();
        let e = finish_write(&root, ".", &json!({"id": "t2"}), &[json!({"id": "bad/id"})]).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{}", e.0);
        batch_rollback(batch);
        assert!(!task.exists());
        assert_eq!(root_events(&root).len(), before, "the failed follow-up logs no task save");
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
    }

    fn journal_path() -> PathBuf {
        with_batch(|active| active.as_ref().expect("an open batch").journal.clone())
    }

    /// An outer handle dropped without a finish puts its files back and releases
    /// the thread's batch.
    #[test]
    fn dropping_an_outer_batch_rolls_it_back() {
        let root = root();
        let kept = board::store_dir(&root).join("t2.json");
        let before = root_events(&root).len();
        {
            let _batch = batch(&root).unwrap();
            batch_replace(&kept, &json!({"id": "t2"})).unwrap();
            batch_log("", &json!({"dropped": true}));
        }
        assert!(!kept.exists());
        assert_eq!(root_events(&root).len(), before);
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
        assert!(with_batch(|active| active.is_none()));
    }

    /// An unfinished nested handle keeps the outer batch from committing.
    #[test]
    fn dropping_a_nested_batch_fails_the_outer() {
        let root = root();
        let outer = batch(&root).unwrap();
        drop(batch(&root).unwrap());
        assert!(batch_commit(outer).is_err());
        assert!(task_journals(&root).is_empty(), "{:?}", task_journals(&root));
        assert!(with_batch(|active| active.is_none()));
    }

    /// Past the cap only the newest row stays, and numbering still moves forward.
    #[test]
    fn the_task_log_keeps_its_newest_row_after_a_hundred_events() {
        let root = root();
        for n in 0..100 {
            log(&root, "", &json!({"n": n}));
        }
        let shape = || {
            events(&root)
                .unwrap()
                .query_row("SELECT COUNT(*), MAX(seq) FROM events", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .unwrap()
        };
        assert_eq!(shape(), (100, 100));
        log(&root, "", &json!({"n": 100}));
        assert_eq!(all_events(&root), vec![json!({"n": 100})]);
        assert_eq!(shape(), (1, 101));
        log(&root, "", &json!({"n": 101}));
        assert_eq!(shape(), (2, 102));
        for n in 102..=200 {
            log(&root, "", &json!({"n": n}));
        }
        assert_eq!(all_events(&root), vec![json!({"n": 200})]);
        assert_eq!(shape(), (1, 201));
    }

    /// Pruning gives the deleted rows' pages back to the file system.
    #[test]
    fn pruning_the_task_log_shrinks_its_file() {
        let root = root();
        let mode: i64 = events(&root).unwrap().query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 1, "the event log uses autovacuum");
        let pad = "x".repeat(4096);
        for n in 0..100 {
            log(&root, "", &json!({"n": n, "pad": pad.as_str()}));
        }
        let pages = || -> i64 { events(&root).unwrap().query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap() };
        let before = pages();
        log(&root, "", &json!({"n": 100, "pad": pad.as_str()}));
        let after = pages();
        assert!(after < before, "{after} pages should be fewer than {before}");
    }

    /// A delete outside a prune still returns its pages.
    #[test]
    fn pages_freed_outside_a_prune_are_reclaimed() {
        let root = root();
        let pad = "x".repeat(4096);
        for n in 0..100 {
            log(&root, "", &json!({"n": n, "pad": pad.as_str()}));
        }
        let pages = || -> i64 { events(&root).unwrap().query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap() };
        let before = pages();
        events(&root).unwrap().execute("DELETE FROM events WHERE seq < 100", []).unwrap();
        let after = pages();
        assert!(after < before, "{after} pages should be fewer than {before}");
    }

    /// An event log made before autovacuum gains it once.
    #[test]
    fn an_old_event_log_switches_to_full_autovacuum() {
        let root = root();
        let path = board::store_dir(&root).join("events.db");
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE events(seq INTEGER PRIMARY KEY, project TEXT NOT NULL, data TEXT NOT NULL);",
        )
        .unwrap();
        old.execute("INSERT INTO events(project,data) VALUES ('', '{}')", []).unwrap();
        drop(old);
        let db = events(&root).unwrap();
        let mode: i64 = db.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 1);
    }

    /// The one root this process's task server serves, and a lock that keeps
    /// the tests using it apart. A process has one server, so both tests share
    /// the root it serves; each starts from an empty task store.
    fn served_root() -> (std::sync::MutexGuard<'static, ()>, PathBuf) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        static ROOT: std::sync::OnceLock<testing::Scratch> = std::sync::OnceLock::new();
        let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = ROOT.get_or_init(|| {
            let root = testing::Scratch::new().with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", DONE_IS_FINAL)]);
            crate::import(root.0.as_path()).unwrap();
            start(root.0.as_path()).unwrap();
            root
        });
        let root = root.0.clone();
        clear_tasks(&root);
        (lock, root)
    }

    /// A change's reply comes back before the settle it starts: while the
    /// settle waits for the task's project lock, the reply has already moved
    /// the task and its undo records are still there.
    #[test]
    fn a_reply_comes_before_the_settle_it_starts() {
        let (_serial, root) = served_root();
        fs::write(root.join("solace.toml"), include_str!("default-board.toml")).unwrap();
        let t = api(&root, "POST", &["tasks"], json!({"title": "Reply"})).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let mut t = t;
        t["runs"] = json!([{"id": "r2", "kind": "work", "state": "ended", "outcome": "stopped"}]);
        store::write(&root, &t).unwrap();
        api(&root, "POST", &["tasks", &tid, "actions"], json!({"action": "plan"})).unwrap();
        let s = board::open(&root).unwrap();
        let before = board::snapshot(&s).unwrap();
        s.submit_symbols(None, Some("geo2"), "pub fn side2() { 3 }").unwrap();
        board::record(&s, &tid, "r2", &before, &board::snapshot(&s).unwrap(), None).unwrap();
        let has_before =
            || s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0)).unwrap();
        let held = board::lock(&root).unwrap();
        let reply = api_with(&root, "POST", &["tasks", &tid, "actions"], json!({"action": "complete"}), |_, _, _| Ok(false)).unwrap();
        assert_eq!(reply["column"], "complete");
        assert!(has_before(), "the reply came back before the settle dropped the undo records");
        drop(held);
        let settled = wait_until(|| !has_before());
        assert!(settled, "the deferred settle ran after the reply");
    }

    /// Removes the tasks and event log a test before left in the shared root,
    /// leaving the server's lock and inbox in place.
    fn clear_tasks(root: &Path) {
        let top = root.to_path_buf();
        op(root, move || {
            let dir = board::store_dir(&top);
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries {
                    let p = entry?.path();
                    if p.is_file() && p.extension().is_some_and(|x| x == "json") {
                        fs::remove_file(p)?;
                    }
                }
            }
            for name in ["events.db", "events.db-wal", "events.db-shm"] {
                let path = dir.join(name);
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            Ok(())
        })
        .unwrap();
    }

    /// Whether `done` holds within the ten seconds a deferred settle may take.
    fn wait_until(done: impl Fn() -> bool) -> bool {
        for _ in 0..400 {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// A test that runs after one that logged events starts with an empty log.
    #[test]
    fn clearing_a_served_root_clears_its_event_log() {
        let (lock, root) = served_root();
        log(&root, "", &json!({"earlier": true}));
        assert!(!root_events(&root).is_empty());
        drop(lock);
        let (_lock, root) = served_root();
        let logged = root_events(&root);
        assert!(logged.is_empty(), "{logged:?}");
    }

    /// D-195: a task link opens on the board only while the board shows the task:
    /// not in Backlog, not under a waiting feature, and not archived.
    #[test]
    fn a_task_link_is_on_the_board_only_when_the_board_shows_it() {
        let root = root();
        let feature =
            |state: &str| strategy_api(&root, "POST", &["features"], json!({"title": state, "state": state})).unwrap()["id"].clone();
        let proposed = feature("proposed");
        let upcoming = feature("upcoming");
        let canceled = feature("canceled");
        let active = feature("active");
        let complete = feature("complete");
        let store = |id: &str, column: &str, feature: Value, archived: bool| {
            let mut t = board::task(&root, "t1").unwrap();
            t["id"] = json!(id);
            t["title"] = json!(id);
            t["column"] = json!(column);
            t["feature"] = feature;
            t["archivedTime"] = if archived { json!(1) } else { Value::Null };
            fs::write(board::store_dir(&root).join(format!("{id}.json")), t.to_string()).unwrap();
        };
        store("backlog", "backlog", Value::Null, false);
        store("proposed", "planning", proposed, false);
        store("upcoming", "planning", upcoming, false);
        store("canceled", "planning", canceled, false);
        store("active", "planning", active, false);
        store("complete", "planning", complete, false);
        store("missing", "planning", json!("ffffffffffffffff"), false);
        store("archived", "planning", Value::Null, true);
        let on_board = |id: &str| find_task(&root, id).unwrap()["onBoard"].as_bool().unwrap();
        assert!(!on_board("backlog"), "Backlog is off the board");
        assert!(!on_board("proposed"), "a proposed feature's task is off the board");
        assert!(!on_board("upcoming"), "an upcoming feature's task is off the board");
        assert!(!on_board("canceled"), "a canceled feature's task is off the board");
        assert!(on_board("active"), "an active feature's task is on the board");
        assert!(on_board("complete"), "a complete feature's task is on the board");
        assert!(on_board("missing"), "a task whose feature is missing is on the board");
        assert!(!on_board("archived"), "an archived task is off the board");
        let listed = board::read_task_api(&root, &["tasks"]).unwrap();
        let listed = |id: &str| listed.as_array().unwrap().iter().find(|t| t["id"] == id).unwrap()["onBoard"].clone();
        assert_eq!((listed("backlog"), listed("active")), (json!(false), json!(true)));
    }

    /// D-196: a feature state change that hides or shows its tasks logs those
    /// tasks, so the board receives their new `onBoard`.
    #[test]
    fn a_feature_visibility_change_announces_its_tasks() {
        let root = root();
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Waiting", "state": "proposed"})).unwrap();
        let fid = f["id"].as_str().unwrap();
        let mut t = board::task(&root, "t1").unwrap();
        t["id"] = json!("shown");
        t["title"] = json!("Shown");
        t["column"] = json!("planning");
        t["feature"] = f["id"].clone();
        fs::write(board::store_dir(&root).join("shown.json"), t.to_string()).unwrap();
        let before = all_events(&root).len();
        strategy_api(&root, "PATCH", &["features", fid], json!({"state": "active"})).unwrap();
        let shown = all_events(&root)[before..]
            .iter()
            .find(|e| e["task"]["id"] == "shown")
            .map(|e| board::stream_task_event(&root, e.clone(), true)["task"]["onBoard"].clone())
            .expect("the task is announced");
        assert_eq!(shown, json!(true));
        let before = all_events(&root).len();
        strategy_api(&root, "PATCH", &["features", fid], json!({"state": "proposed"})).unwrap();
        let hidden = all_events(&root)[before..]
            .iter()
            .find(|e| e["task"]["id"] == "shown")
            .map(|e| board::stream_task_event(&root, e.clone(), true)["task"]["onBoard"].clone())
            .expect("the task is announced again");
        assert_eq!(hidden, json!(false));
    }

    /// A new task with no feature offers Approve to Planning; every other
    /// proposal and an edit offers only Approve and Reject.
    #[test]
    fn only_a_new_task_without_a_feature_offers_approve_to_planning() {
        let root = root();
        let rid = page_run(&root);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "F"})).unwrap();
        let fid = f["id"].as_str().unwrap();
        let options = |kind: &str, id: Option<&str>, text: &str| proposal(&root, &rid, kind, id, text).unwrap()["options"].clone();
        let three = json!(["Approve", "Approve to Planning", "Reject"]);
        let two = json!(["Approve", "Reject"]);
        assert_eq!(options("task", None, r#"{"title": "New"}"#), three);
        assert_eq!(options("task", None, r#"{"title": "No feature", "feature": null}"#), three);
        assert_eq!(options("task", None, &json!({"title": "With feature", "feature": fid}).to_string()), two);
        assert_eq!(options("task", Some("t1"), r#"{"title": "Edit"}"#), two);
        assert_eq!(options("feature", None, r#"{"title": "F2"}"#), two);
        assert_eq!(options("feature", Some(fid), r#"{"title": "F"}"#), two);
        assert_eq!(options("milestone", None, r#"{"title": "M"}"#), two);
        assert_eq!(options("vision", None, "New vision"), two);
    }

    /// Approve to Planning creates a featureless task in Planning and says so;
    /// Approve still creates it in Backlog.
    #[test]
    fn approving_to_planning_creates_the_task_in_planning() {
        let root = root();
        let rid = page_run(&root);
        let prefix = "Approved and applied: created task:";
        let qid = propose(&root, &rid, "task", None, r#"{"title": "Now"}"#);
        let (said, note) = answer(&root, &rid, &qid, "Approve to Planning");
        assert!(said.starts_with(prefix) && said.ends_with(" (Planning)"), "{said}");
        assert!(note.contains(&said), "{note}");
        let id = &said[prefix.len()..said.len() - " (Planning)".len()];
        let t = board::task(&root, id).unwrap();
        assert_eq!((t["title"].as_str(), t["home"].as_str(), t["column"].as_str()), (Some("Now"), Some("."), Some("planning")));
        let qid = propose(&root, &rid, "task", None, r#"{"title": "Later"}"#);
        let said = answer(&root, &rid, &qid, "Approve").0;
        let id = &said[prefix.len()..];
        assert_eq!(board::task(&root, id).unwrap()["column"], "backlog");
    }

    /// D-132: without a Planning column, Approve to Planning takes the
    /// workflow's first column and still says (Planning).
    #[test]
    fn approving_to_planning_takes_the_first_column_without_planning() {
        let root = root();
        fs::write(
            root.join("solace.toml"),
            concat!(
                "[board]\nevents = { finished = \"done\" }\n",
                "[[board.columns]]\nid = \"waiting\"\n",
                "[[board.columns]]\nid = \"backlog\"\n",
                "[[board.columns]]\nid = \"done\"\nfinal = true\n",
                "[board.actions]\n",
            ),
        )
        .unwrap();
        let rid = page_run(&root);
        let prefix = "Approved and applied: created task:";
        let qid = propose(&root, &rid, "task", None, r#"{"title": "Planned"}"#);
        let said = answer(&root, &rid, &qid, "Approve to Planning").0;
        assert!(said.starts_with(prefix) && said.ends_with(" (Planning)"), "{said}");
        let id = &said[prefix.len()..said.len() - " (Planning)".len()];
        assert_eq!(board::task(&root, id).unwrap()["column"], "waiting");
    }

    /// D-134, D-196: a task a proposal creates in Backlog under an upcoming
    /// feature is listed in Planning with `onBoard` true once the feature starts.
    #[test]
    fn a_started_feature_lists_its_planned_task_on_the_board() {
        let root = root();
        let rid = page_run(&root);
        let f = strategy_api(&root, "POST", &["features"], json!({"title": "Search", "state": "upcoming"})).unwrap();
        let fid = f["id"].as_str().unwrap();
        let qid = propose(&root, &rid, "task", None, &json!({"title": "Planned", "feature": fid}).to_string());
        let said = answer(&root, &rid, &qid, "Approve").0;
        let id = said.rsplit(':').next().unwrap();
        assert_eq!(column_of(&root, id), "backlog");
        let listed = |root: &Path| board::read_task_api(root, &["tasks"]).unwrap();
        let task =
            |root: &Path, id: &str| listed(root).as_array().unwrap().iter().find(|t| t["id"] == id).cloned().expect("the task is listed");
        assert_eq!(task(&root, id)["onBoard"], json!(false), "a Backlog task is off the board");
        strategy_api(&root, "PATCH", &["features", fid], json!({"state": "active"})).unwrap();
        let t = task(&root, id);
        assert_eq!((t["column"].as_str(), t["home"].as_str()), (Some("planning"), Some(".")));
        assert_eq!(t["onBoard"], json!(true));
    }
}
