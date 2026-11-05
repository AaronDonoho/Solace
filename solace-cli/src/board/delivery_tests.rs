#[test]
fn identical_notes_are_delivered_once_each() {
    let note = |id: &str| serde_json::json!({"id":id,"author":"human","run":"r1","time":5,"text":"same"});
    let first = note("n1");
    let second = note("n2");
    let run = serde_json::json!({"notes_seen":[crate::board::briefs::note_fingerprint(&first)]});
    let pending = super::take_pending(&run, &[first, second.clone()], Some("r1")).unwrap();
    assert_eq!(super::pending_note_keys(&pending), vec![crate::board::briefs::note_fingerprint(&second)]);
    assert_eq!(super::pending_text(&pending), "The human replied:\n\nsame\n");
}

#[test]
fn notes_keep_order_and_an_answer_names_its_proposal() {
    let notes = [
        serde_json::json!({"id":"n1","author":"human","run":"r1","time":5,"text":"one"}),
        serde_json::json!({"id":"n2","author":"human","run":"r1","time":6,"text":"two"}),
    ];
    let run = serde_json::json!({"questions":[{"id":"q1","text":"Create task \"T\"\n\nmore","answer":"Approve"}]});
    let pending = super::take_pending(&run, &notes, Some("r1")).unwrap();
    assert_eq!(super::pending_note_keys(&pending).len(), 2);
    assert_eq!(super::pending_answer_keys(&pending), vec!["q1"]);
    assert_eq!(
        super::pending_text(&pending),
        "The human replied:\n\none\n\ntwo\n\nThe human answered the proposal Create task \"T\": Approve\n"
    );
}

#[test]
fn wait_limit_stays_below_the_agent_clients_timeout() {
    for timeout in [60, 7200] {
        let client = std::time::Duration::from_secs(crate::lockdown::tool_timeout(timeout));
        assert!(super::wait_limit_for(timeout) < client, "timeout {timeout}");
    }
}

#[test]
fn two_claims_for_one_note_leave_it_with_one_waiter() {
    let note = serde_json::json!({"id":"n1","author":"human","run":"r1","time":5,"text":"one"});
    let key = crate::board::briefs::note_fingerprint(&note);
    let pending = super::take_pending(&serde_json::json!({}), &[note], Some("r1")).unwrap();
    let mut run = serde_json::json!({});
    let claim = |id: &str| serde_json::json!({"claim":id,"wake":true,"notes":[key.clone()],"answers":[]});
    super::record(&mut run, &claim("first"), false);
    super::record(&mut run, &claim("second"), false);
    assert_eq!(run["notes_seen"][key.as_str()], serde_json::json!("first"));
    assert_eq!(super::pending_note_keys(&super::pending_owned(&pending, &run, "first")), vec![key.clone()]);
    assert!(super::pending_owned(&pending, &run, "second").notes.is_empty());
}

#[test]
fn wait_limit_uses_the_launch_timeout_when_the_setting_changes() {
    let dir = crate::testing::Scratch::new().with(&[("system.toml", "task_timeout=60\n")]);
    let limit = crate::board::setup::with_config(&dir.join("system.toml"), || super::wait_limit_with_launch(Some(7200)));
    assert_eq!(limit, super::wait_limit_for(7200));
}

#[test]
fn pending_replies_see_a_note_or_answer_once() {
    let note = serde_json::json!({"id":"n1","author":"human","run":"r1","time":5,"text":"one"});
    let question = serde_json::json!({"id":"q1","text":"Ask\nmore","answer":"yes"});
    let key = crate::board::briefs::note_fingerprint(&note);
    let mut task = serde_json::json!({"notes":[]});
    assert!(!super::pending_task_reply(&task, &[], "r1") && !super::pending_strategy_reply(&task));
    task["notes"] = serde_json::json!([note]);
    assert!(super::pending_task_reply(&task, task["notes"].as_array().unwrap(), "r1"));
    assert!(!super::pending_task_reply(&task, task["notes"].as_array().unwrap(), "r2"));
    assert!(super::pending_strategy_reply(&task));
    super::record(&mut task, &serde_json::json!({"claim":"c","wake":true,"notes":[key],"answers":[]}), false);
    assert!(!super::pending_strategy_reply(&task));
    let mut answered = serde_json::json!({"notes":[],"questions":[question]});
    assert!(super::pending_strategy_reply(&answered));
    super::record(&mut answered, &serde_json::json!({"claim":"c","wake":true,"notes":[],"answers":["q1"]}), false);
    assert!(!super::pending_strategy_reply(&answered));
}

#[test]
fn record_marks_briefs_but_holds_paused_deliveries() {
    let delivery = |wake: bool| serde_json::json!({"claim":"c","wake":wake,"notes":["n1","n2"],"answers":[]});
    let mut ended = serde_json::json!({"state":"ended"});
    super::record(&mut ended, &delivery(true), false);
    assert_eq!((ended["notes_seen"].clone(), ended["state"].clone()), (serde_json::Value::Null, serde_json::json!("ended")));
    let mut paused = serde_json::json!({"state":"waiting"});
    super::record(&mut paused, &delivery(true), true);
    assert_eq!((paused["notes_seen"].clone(), paused["state"].clone()), (serde_json::Value::Null, serde_json::json!("waiting")));
    super::record(&mut paused, &delivery(false), true);
    assert_eq!((paused["notes_seen"]["n1"].clone(), paused["state"].clone()), (serde_json::json!("c"), serde_json::json!("waiting")));
    let mut running = serde_json::json!({"state":"waiting"});
    super::record(&mut running, &delivery(true), false);
    assert_eq!((running["notes_seen"]["n2"].clone(), running["state"].clone()), (serde_json::json!("c"), serde_json::json!("running")));
}

#[test]
fn wait_limit_reads_the_launch_timeout_from_the_agent_environment() {
    let dir = crate::testing::Scratch::new().with(&[("system.toml", "task_timeout=60\n")]);
    let limit = crate::board::setup::with_config(&dir.join("system.toml"), || {
        super::wait_limit_with_env(|name| (name == "SOLACE_LAUNCH_TIMEOUT_SECS").then(|| "7200".to_string()))
    });
    assert_eq!(limit, super::wait_limit_for(7200));
}
