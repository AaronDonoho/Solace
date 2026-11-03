use serde_json::Value;
use serde_json::json;
use std::env;
use std::fs;
use crate::board::agents;
use crate::testing;
use std::time::Duration;
use crate::board::setup::parse_system;
use crate::board::setup::endpoint_args;
use crate::board::setup::efforts;
use crate::board::setup::served;
use crate::board::setup::with_config;
use crate::board::runs::transcript;
use crate::board::runs::last_words;
use crate::board::runs::partly_failed;
use crate::board::task_model::end_run;
use crate::now;
use crate::unique_id;
use crate::jsonx;
use crate::board::setup;
use agents::unavailable;

/// What agents say when a usage limit is reached, as they say it (see its `about`).
const USAGE_LIMITS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/board/usage-limits.json"));
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
    assert_eq!(t[2]["text"], "10 input tokens (4 cached, 40% cache hit), 2 output tokens");
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
        "200356 input tokens (166278 read from cache, 34062 written to cache, 83% cache hit), 4040 output tokens; last context 300 tokens"
    );
    // A missing cache count leaves the ratio, and the cached count, out.
    let result = json!({"type":"result","usage":{"input_tokens":40,"output_tokens":1}}).to_string();
    fs::write(&log, result).unwrap();
    let usage = transcript(&log).into_iter().find(|e| e["kind"] == "usage").unwrap();
    assert_eq!(usage["text"], "40 input tokens, 1 output tokens");
    let turn = json!({"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2}}).to_string();
    fs::write(&log, turn).unwrap();
    let usage = transcript(&log).into_iter().find(|e| e["kind"] == "usage").unwrap();
    assert_eq!(usage["text"], "10 input tokens, 2 output tokens");
    assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
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

/// A current entry names a shared model before a gone entry does.
#[test]
fn a_current_model_names_the_shared_model_before_a_gone_one() {
    let models = json!([
        {"id":"old","label":"Gone","agent":"codex","model":"claude-opus-5-5","gone":true},
        {"id":"second","label":"Second","agent":"codex","model":"claude-opus-5-5"}
    ]);
    assert_eq!(agents::used_label(models.as_array().unwrap(), "claude-opus-5-5"), "Second", "a current entry names the shared model");

    let gone_only = json!([{"id":"old","label":"Gone","agent":"codex","model":"claude-opus-5-5","gone":true}]);
    assert_eq!(
        agents::used_label(gone_only.as_array().unwrap(), "claude-opus-5-5"),
        "Gone",
        "a gone entry still names it when no current one does"
    );
}

#[test]
fn a_missing_agent_program_is_checked_now() {
    let reason = agents::local_now(&json!({"run": ["solace-agent-that-does-not-exist"]}));
    assert!(reason.as_ref().is_some_and(|reason| reason.contains("is not installed")), "{reason:?}");
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

/// The shipped presets give way to Claude's aliases: one entry per family, and
/// the Haiku entry is the latest model rather than the exact one a preset once named.
#[test]
fn the_shipped_presets_show_the_latest_claude_models() {
    let mut models = parse_system("").unwrap()["models"].as_array().unwrap().clone();
    models.extend(agents::claude_aliases());
    let picked = served(&models);
    let haiku: Vec<&Value> = picked.iter().filter(|m| agents::family(m) == Some("haiku")).collect();
    assert_eq!(haiku.len(), 1, "one Haiku entry");
    assert_eq!(haiku[0]["id"], "claude-haiku");
    assert_eq!(haiku[0]["label"], "Haiku (latest)");
    assert_eq!(haiku[0]["model"], "haiku");
    for family in ["opus", "sonnet", "haiku", "fable"] {
        assert_eq!(picked.iter().filter(|m| agents::family(m) == Some(family)).count(), 1, "one {family} entry");
    }
}

/// A run's stored label names its model after every entry that ran it is gone.
#[test]
fn a_used_model_keeps_the_label_a_run_stored() {
    let configured = json!([{"id":"claude-haiku","label":"Haiku (latest)","agent":"claude","model":"haiku"}]);
    let all = json!({
        "used":{"claude-haiku":{"model":"claude-haiku-4-5-20251001","label":"Haiku 4.5"}}
    });
    let m = agents::merge_from(&configured, &all);
    assert_eq!(m[0]["label"], "Haiku 4.5", "the label stored with the run is shown");
    assert_eq!(m[0]["model"], "haiku", "the alias is still what runs");
}

/// Ending a run stores the label the model list gave its used model.
#[test]
fn ending_a_run_stores_its_model_label() {
    let dir = testing::Scratch::new();
    let config = dir.join("config.toml");
    fs::write(&config, "[[models]]\nid = \"claude-haiku-9-9\"\nlabel = \"Haiku 9.9\"\nagent = \"claude\"\nmodel = \"claude-haiku-9-9\"\n")
        .unwrap();
    let log = dir.join("log.jsonl");
    fs::write(&log, "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-haiku-9-9\"}\n").unwrap();
    let mut run = json!({"model":"claude-haiku","log":log});
    with_config(&config, || end_run(&mut run, "finished"));
    let saved: Value = jsonx::read(&dir.join("agent-models.json")).unwrap();
    assert_eq!(saved["used"]["claude-haiku"], json!({"model":"claude-haiku-9-9","label":"Haiku 9.9"}));
}

