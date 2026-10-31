#[cfg(test)] use crate::board::setup::parse_system;
use super::*;
use serde_json::json;
#[cfg(test)] use crate::board::parse_system as parse_system_2;
use crate::board::setup::language_enabled_at;

#[test]
fn presets_merge_with_configured_models() {
    let v = parse_system_2("").unwrap();
    let ids: Vec<&str> = v["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"codex-gpt-6-astra") && ids.contains(&"claude-opus-5-5"));
    assert_eq!(v["task_timeout"], 3600);
    let v = parse_system_2(
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
    assert!(parse_system_2("presets = false\n").unwrap()["models"].as_array().unwrap().is_empty());
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
fn malformed_language_settings_are_an_error_not_off() {
    let root = testing::Scratch::new().with(&[("system.toml", "[languages]\ngleam = 1\n")]);
    assert!(language_enabled_at(&root.join("system.toml"), "gleam").is_err());
    let root = testing::Scratch::new().with(&[("system.toml", "[languages]\ngleam = false\n")]);
    assert!(!language_enabled_at(&root.join("system.toml"), "gleam").unwrap());
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
    assert_eq!(crate::board::setup::endpoint_context(&base, "configured-name"), Some(131072));
    assert_eq!(crate::board::setup::endpoint_context(&base, "b"), Some(2));
    assert_eq!(crate::board::setup::endpoint_context(&base, "c"), None);
    t.join().unwrap();
    assert_eq!(crate::board::setup::endpoint_context("https://example.com", "m"), None);
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
    let picked = crate::board::setup::served(models.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["codex-gpt-6-astra", "claude-opus-5-5", "claude-fable", "claude-haiku", "mine"]);
    let dropped = json!([
        {"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5","gone":true},
        {"id":"claude-opus","label":"Opus (latest)","agent":"claude","model":"opus"},
    ]);
    let picked = crate::board::setup::served(dropped.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude-opus"], "the alias stands when the family's exact model is gone");
    let exact_only = json!([{"id":"claude-opus-5-5","label":"Opus 5.5","agent":"claude","model":"claude-opus-5-5","gone":true}]);
    let picked = crate::board::setup::served(exact_only.as_array().unwrap());
    let ids: Vec<&str> = picked.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude-opus-5-5"], "a family with only a dropped model shows its reason");
}

