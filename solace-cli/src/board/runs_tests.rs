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
    assert_eq!(task_ids(Some("strategy".into()), Some("r".into())), None);
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

