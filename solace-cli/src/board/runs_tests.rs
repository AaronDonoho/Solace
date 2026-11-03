use super::*;
use crate::testing;
use serde_json::json;
use serde_json::Value;
use std::process::Command;
use std::time::Duration;
use crate::board::runs::Watch;
use std::time::Instant;
use crate::board::task_model::message;
use crate::board::runs::GRACEFUL_EXIT;
use crate::board::runs::supervise;
use crate::board::runs::outline;
use crate::board::task_model::task;
use crate::board::task_model::workflow;
use crate::board::task_model::new_task;
use crate::board::tests::launched;
use crate::lockdown;
use crate::board::runs::failed_step_line;
use crate::board::runs::error_line;
use crate::board::runs::error_code;
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
fn a_missing_program_is_reported_plainly_at_launch() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    let m = json!({"agent":"codex","run":["solace-no-such-program"]});
    let e = spawn_agent(&p.root, &dir, &log, &m, "", &[], None, false, ["t", "r"], 60).unwrap_err();
    assert_eq!(e.0, "solace-no-such-program is not installed. Install it, then start the task again.");
}

#[test]
fn only_the_solace_tool_counts_as_a_solace_call() {
    for title in ["solace", "solace view a", "mcp__solace__solace test", "mcp__solace inspect a"] {
        assert!(solace_title(title), "{title}");
    }
    for title in ["list_mcp_resources ", "read_mcp_resource ", "web search rg", "mcp__solace__list_mcp_resources"] {
        assert!(!solace_title(title), "{title}");
    }
}

/// A task with one run `r1` in `run`'s state, and a hung agent in its own process group.
fn supervised(run: Value) -> (testing::Project, String, std::process::Child) {
    launched(run, Command::new("sleep").arg("45").process_group(0).spawn().unwrap())
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
        !supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
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
        supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_secs(3600), GRACEFUL_EXIT,)
            .unwrap()
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"running"}));
    let log = p.root.join(".solace/r1.log");
    assert!(
        supervise(Watch::Task(&p.root, &tid), "r1", &mut child, &log, &json!({}), false, false, Duration::from_millis(200), GRACEFUL_EXIT,)
            .unwrap()
    );
    assert!(child.try_wait().unwrap().is_some());
    let t = task(&p.root, &tid).unwrap();
    assert_eq!(t["runs"][0]["state"], "ended");
    assert!(t["notes"].as_array().unwrap().iter().any(|n| n["text"] == "task timeout"));
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
/// A strategy run and a run outside a task have no task run: a task run has
/// both ids, and a strategy run's task id is `strategy`.
#[test]
fn a_strategy_run_and_a_run_outside_a_task_have_no_task_run() {
    assert_eq!(task_ids(None, None), None);
    assert_eq!(task_ids(Some(STRATEGY_RUN.into()), Some("r".into())), None);
    assert_eq!(task_ids(Some("t".into()), None), None);
    assert_eq!(task_ids(Some("t".into()), Some("r".into())), Some(("t".into(), "r".into())));
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
    assert_eq!(failed_step_line(2, &batch), "3 submit b: E0108 no b");
    let plain = json!({
        "kind": "tool",
        "title": "solace view b",
        "output": "MCP error: transport closed",
        "status": "failed",
    });
    assert_eq!(failed_step_line(0, &plain), "1 view b: MCP error: transport closed");
    let no_code = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) inspect a\ncheck",
        "output": concat!(
            "==== solace inspect a ====\nfn a() {}\n",
            "==== solace check ====\nerror[E0308]: mismatched types\n1 of 2 commands failed: check"
        ),
        "status": "failed",
    });
    assert_eq!(failed_step_line(0, &no_code), "1 check: error[E0308]: mismatched types");
    let earlier_code = json!({
        "kind": "tool",
        "title": "solace (1 of 2 failed) read a\ncheck",
        "output": concat!(
            "==== solace read a ====\nE1234 not this failure\n",
            "==== solace check ====\ncargo error\n1 of 2 commands failed: check"
        ),
        "status": "failed",
    });
    assert_eq!(failed_step_line(1, &earlier_code), "2 check: cargo error");
    assert_eq!(error_line("==== solace a ====\nE6002 panic\n"), Some("E6002 panic"));
    assert_eq!(error_line("==== solace a ====\nboom\n"), Some("boom"));
    assert!(error_code("E0108 x") && !error_code("E108 x") && !error_code("error"));
}

/// A server that ends where the harness won't show its stderr leaves its
/// line in the run log the board reads.
#[test]
fn a_server_end_line_lands_in_the_task_run_log() {
    let root = testing::Scratch::new();
    let log = root.join("run.jsonl");
    std::fs::write(&log, "{}\n").unwrap();
    std::fs::create_dir_all(root.join(".solace/tasks")).unwrap();
    let t = json!({
        "id":"t1","column":"backlog","home":".","createdTime":1,"closedTime":null,
        "runs":[{"id":"r1","kind":"work","state":"running","log":log.to_string_lossy().to_string()}],
    });
    std::fs::write(root.join(".solace/tasks/t1.json"), t.to_string()).unwrap();
    let line = "solace mcp: stdout closed while running submit --imports lockdown::paths";
    crate::cli::with_root(&root, || with_context("t1", "r1", || record_server_end(line)));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "{}\n");
    assert_eq!(std::fs::read_to_string(server_end_log(&log)).unwrap(), format!("{line}\n"));
    assert_eq!(transcript(&log).last().unwrap()["text"], line);
}

/// A strategy run's log takes the same line.
#[test]
fn a_server_end_line_lands_in_the_strategy_run_log() {
    let root = testing::Scratch::new();
    let log = root.join("run.jsonl");
    std::fs::write(&log, "").unwrap();
    std::fs::create_dir_all(root.join(".solace/strategy/runs")).unwrap();
    let run = json!({"id":"s1","log":log.to_string_lossy().to_string()});
    std::fs::write(root.join(".solace/strategy/runs/s1.json"), run.to_string()).unwrap();
    let line = "solace mcp: stdin read failed: stream did not contain valid UTF-8";
    let ids = strategy_ids(Some(STRATEGY_RUN.into()), Some("s1".into()));
    assert_eq!(ids, Some((STRATEGY_RUN.into(), "s1".into())));
    record_server_end_for(&root, ids, line);
    assert_eq!(std::fs::read_to_string(server_end_log(&log)).unwrap(), format!("{line}\n"));
}

/// A server-end line is an error of its own, even after the agent's last plain line.
#[test]
fn a_server_end_line_is_an_error_after_plain_output() {
    let root = testing::Scratch::new();
    let log = root.join("run.jsonl");
    std::fs::write(&log, "boom\n").unwrap();
    let line = "solace mcp: stdin read failed: stream did not contain valid UTF-8";
    std::fs::write(server_end_log(&log), format!("{line}\n")).unwrap();
    let entries = transcript(&log);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], json!({"kind":"output","text":"boom"}));
    assert_eq!(entries[1], json!({"kind":"error","text":line,"source":SERVER}));
}

/// The agent's own failure is the reason a run stopped, not the server line after it.
#[test]
fn an_agent_error_is_shown_before_a_server_end_line() {
    let said = "You've hit your limit; resets at 2pm";
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let (p, tid, mut child) = launched(run.clone(), Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap());
    let log = p.root.join(".solace/r1.log");
    std::fs::write(&log, json!({"type":"turn.failed","error":{"message":said}}).to_string()).unwrap();
    let server = "solace mcp: stdin read failed: stream did not contain valid UTF-8";
    std::fs::write(server_end_log(&log), format!("{server}\n")).unwrap();
    let status = child.wait().unwrap();
    handle_exit(
        &Watch::Task(&p.root, &tid),
        "r1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap();
    let outcome = task(&p.root, &tid).unwrap()["runs"][0]["outcome"].as_str().unwrap().to_string();
    assert!(outcome.contains(said), "{outcome}");
    assert!(!outcome.contains(server), "{outcome}");
}

/// A server line is not the agent's failure, so a clean run with no Solace call still retries.
#[test]
fn a_server_end_line_does_not_block_the_fresh_agent_retry() {
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let (p, tid, mut child) = launched(run.clone(), Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap());
    let log = p.root.join(".solace/r1.log");
    std::fs::write(&log, "").unwrap();
    std::fs::write(server_end_log(&log), "solace mcp: stdin closed\n").unwrap();
    let status = child.wait().unwrap();
    let e = handle_exit(
        &Watch::Task(&p.root, &tid),
        "r1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap_err();
    assert_eq!(e.1, NO_TOOL);
}

/// A strategy agent that replied and exited cleanly still replies when the server left a line.
#[test]
fn a_strategy_reply_is_kept_when_a_server_end_line_follows() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let runs = top.join(".solace/strategy/runs");
    std::fs::create_dir_all(&runs).unwrap();
    let reply = "I suggest starting with the smallest version.";
    let run = json!({"id":"s1","kind":"strategy","state":"running","notes":[],"questions":[]});
    std::fs::write(runs.join("s1.json"), run.to_string()).unwrap();
    let log = top.join(".solace/s1.log");
    std::fs::write(&log, json!({"type":"item.completed","item":{"type":"agent_message","text":reply}}).to_string()).unwrap();
    std::fs::write(server_end_log(&log), "solace mcp: stdin read failed\n").unwrap();
    let status = Command::new("sh").arg("-c").arg("exit 0").status().unwrap();
    let e = handle_exit(
        &Watch::Strategy(&top),
        "s1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap_err();
    assert_eq!(e.1, REPLIED);
    let saved = tasks::strategy_run(&top, "s1").unwrap();
    assert_eq!(saved["state"], "waiting");
    assert_eq!(saved["notes"][0]["text"], reply);
}

/// A cache ratio needs both a cached count and input tokens to compare it with.
#[test]
fn a_cache_ratio_is_shown_only_with_a_cached_count_and_input() {
    assert_eq!(cache_ratio(Some(4), 10).as_deref(), Some("40% cache hit"));
    assert_eq!(cache_ratio(Some(4), 0), None);
    assert_eq!(cache_ratio(Some(11), 10), None);
    assert_eq!(cache_ratio(None, 10), None);
}

#[test]
fn a_late_note_finishes_a_failed_review() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] =
        json!([{"id":"r1","kind":"review","state":"ended","outcome":"agent exited without finishing (exit status: 1)","failed":true}]);
    t["attention"] = json!({"reason":"agent exited without finishing (exit status: 1)","time":1});
    let m = json!({
        "kind":"agent",
        "task":"t",
        "run":"r1",
        "command":"note",
        "text":"**Assessment: Needs work**",
        "changed_during_review":true,
    });
    message(&w, &mut t, &m).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    assert_eq!(t["finished"]["outcome"], "reviewed");
    assert!(t["attention"].is_null());
    let notes = t["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n["author"] == "agent" && n["run"] == "r1" && n["text"] == "**Assessment: Needs work**"));
    assert!(notes.iter().any(|n| n["author"] == "solace" && n["run"] == "r1"));
}

#[test]
fn a_late_note_does_not_duplicate_an_ended_reviews_feedback() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"review","state":"ended","outcome":"reviewed"}]);
    t["notes"] = json!([{"author":"agent","run":"r1","time":1,"text":"first"}]);
    let m = json!({
        "kind":"agent",
        "task":"t",
        "run":"r1",
        "command":"note",
        "text":"second",
        "changed_during_review":true,
    });
    message(&w, &mut t, &m).unwrap();
    assert_eq!(t["notes"].as_array().unwrap().len(), 1);
    assert_eq!(t["notes"][0]["text"], "first");
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
}

#[test]
fn a_late_note_leaves_a_stopped_review_alone() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"review","state":"ended","outcome":"stopped"}]);
    t["finished"] = Value::Null;
    let m = json!({
        "kind":"agent",
        "task":"t",
        "run":"r1",
        "command":"note",
        "text":"late",
        "changed_during_review":true,
    });
    message(&w, &mut t, &m).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "stopped");
    assert!(t["finished"].is_null());
    assert_eq!(t["notes"].as_array().unwrap().len(), 1);
    assert_eq!(t["notes"][0]["text"], "late");
}

#[test]
fn a_late_note_keeps_attention_that_is_not_an_unfinished_exit() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] =
        json!([{"id":"r1","kind":"review","state":"ended","outcome":"agent exited without finishing (exit status: 1)","failed":true}]);
    t["attention"] = json!({"reason":"the model is not signed in","time":1});
    let m = json!({
        "kind":"agent",
        "task":"t",
        "run":"r1",
        "command":"note",
        "text":"late",
        "changed_during_review":false,
    });
    message(&w, &mut t, &m).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "reviewed");
    assert_eq!(t["attention"]["reason"], "the model is not signed in");
    assert_eq!(t["notes"].as_array().unwrap().len(), 1);
}

#[test]
fn a_late_note_leaves_an_unknown_outcome_alone() {
    let w = workflow(&toml::Value::Table(Default::default())).unwrap();
    let mut t = new_task("T", "", &w["columns"][0]["id"], 1.);
    t["runs"] = json!([{"id":"r1","kind":"review","state":"ended","outcome":"archived"}]);
    t["finished"] = Value::Null;
    let m = json!({
        "kind":"agent",
        "task":"t",
        "run":"r1",
        "command":"note",
        "text":"late",
        "changed_during_review":true,
    });
    message(&w, &mut t, &m).unwrap();
    assert_eq!(t["runs"][0]["outcome"], "archived");
    assert!(t["finished"].is_null());
    assert_eq!(t["notes"].as_array().unwrap().len(), 1);
    assert_eq!(t["notes"][0]["text"], "late");
}

#[test]
fn a_saved_baseline_matches_only_the_same_kind() {
    let error = crate::store_core::DependentError { symbol: "app.legacy".into(), kind: "Type mismatch".into(), text: String::new() };
    assert!(baseline_has(&[json!("app.legacy")], &error), "a legacy entry names the symbol only");
    assert!(baseline_has(&[json!(["app.legacy", "Type mismatch"])], &error));
    assert!(!baseline_has(&[json!(["app.legacy", "Incorrect arity"])], &error));
    assert!(!baseline_has(&[json!(7)], &error));
}

#[test]
fn a_failed_turn_and_a_failed_result_are_fatal_errors() {
    let dir = testing::Scratch::new();
    let log = dir.join("log.jsonl");
    fs::write(&log, json!({"type":"turn.failed","error":{"message":"boom"}}).to_string()).unwrap();
    assert_eq!(transcript(&log), [json!({"kind":"error","text":"boom","fatal":true})]);
    fs::write(&log, json!({"type":"result","is_error":true,"result":"boom","usage":{}}).to_string()).unwrap();
    assert_eq!(transcript(&log).last().unwrap(), &json!({"kind":"error","text":"boom","fatal":true}));
}

#[test]
fn a_transient_error_does_not_block_a_discussion_reply() {
    let run = json!({"id":"r1","kind":"discuss","state":"running"});
    let (p, tid, mut child) = launched(run.clone(), Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap());
    let log = p.root.join(".solace/r1.log");
    let notice = json!({"type":"error","message":"Reconnecting... 1/5"});
    let reply = json!({"type":"item.completed","item":{"type":"agent_message","text":"Hello."}});
    std::fs::write(&log, format!("{notice}\n{reply}")).unwrap();
    let status = child.wait().unwrap();
    let e = handle_exit(
        &Watch::Task(&p.root, &tid),
        "r1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap_err();
    assert_eq!(e.1, REPLIED);
    let saved = task(&p.root, &tid).unwrap();
    assert_eq!(saved["runs"][0]["state"], "waiting");
    assert_eq!(saved["notes"][0]["text"], "Hello.");
}

#[test]
fn a_transient_error_does_not_block_the_fresh_agent_retry() {
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let (p, tid, mut child) = launched(run.clone(), Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap());
    let log = p.root.join(".solace/r1.log");
    let notice = json!({"type":"error","message":"Reconnecting... 1/5"});
    let reply = json!({"type":"item.completed","item":{"type":"agent_message","text":"Nothing to run."}});
    std::fs::write(&log, format!("{notice}\n{reply}")).unwrap();
    let status = child.wait().unwrap();
    let e = handle_exit(
        &Watch::Task(&p.root, &tid),
        "r1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap_err();
    assert_eq!(e.1, NO_TOOL);
}

/// With no reply at all, an agent error still comes after the last message and blocks the run.
#[test]
fn an_agent_error_without_a_reply_is_shown() {
    let said = "Reconnecting... 5/5";
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let (p, tid, mut child) = launched(run.clone(), Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap());
    let log = p.root.join(".solace/r1.log");
    std::fs::write(&log, json!({"type":"error","message":said}).to_string()).unwrap();
    let status = child.wait().unwrap();
    handle_exit(
        &Watch::Task(&p.root, &tid),
        "r1",
        RunContext { run: &run, log: &log, model: &json!({}), resumed: false, retried: false },
        status,
    )
    .unwrap();
    let outcome = task(&p.root, &tid).unwrap()["runs"][0]["outcome"].as_str().unwrap().to_string();
    assert!(outcome.contains(said), "{outcome}");
}

