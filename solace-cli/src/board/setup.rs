//! Models a human sets up on the Settings page: their `[[models]]` entries in
//! the system settings, and the API keys pasted for them. A key is kept by
//! model id in `keys.json`, beside the system settings, readable only by its
//! owner (mode 0600); never in `config.toml`, and never sent back to a page.
//! Also reads the system settings for the app: the selected model, endpoints,
//! and whether each language is on.
use super::*;
use std::os::unix::fs::OpenOptionsExt;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use crate::board::err;
use crate::board::agents::KINDS;
use crate::board::agents::models_path;
use crate::board::agents::saved_models;
use crate::board::agents::merge_from;
use crate::board::agents::REFRESHING;
use crate::board::agents::refresh;
use crate::board::unavailable;
use crate::now;
use crate::failure::Failure;
#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
use std::fs;
/// The variable a stored key reaches Codex through, on the agent's process only.
pub(crate) const KEY_VAR: &str = "SOLACE_ENDPOINT_KEY";

/// Where pasted keys are kept, beside the system settings.
pub(crate) fn keys_path() -> PathBuf {
    config_path().with_file_name("keys.json")
}

pub(super) fn read_keys(path: &Path) -> Value {
    jsonx::read_opt(path).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The key stored for model `id`, if any.
pub(crate) fn stored(path: &Path, id: &str) -> Option<String> {
    read_keys(path)[id].as_str().filter(|k| !k.is_empty()).map(String::from)
}

/// Stores model `id`'s key, or with None forgets it.
pub(super) fn set_key(path: &Path, id: &str, key: Option<&str>) -> Result<(), Failure> {
    let mut all = read_keys(path);
    match key {
        Some(k) => all[id] = json!(k),
        None => {
            all.as_object_mut().unwrap().remove(id);
        }
    }
    replace(path, &serde_json::to_vec_pretty(&all).unwrap())
}

/// Writes `path` whole, through a file only its owner can read, so a crash
/// leaves the old file and no reader sees half of one.
fn replace(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?.write_all(bytes)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn settings(config: &Path) -> Result<toml::Table, Failure> {
    let text = if config.exists() { fs::read_to_string(config)? } else { String::new() };
    text.parse().map_err(|e| Failure(format!("E6020 system settings: {e}"), 1))
}

/// An id made from a label: "Command Code" is `command-code`.
fn slug(label: &str) -> String {
    let words: Vec<String> =
        label.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect();
    words.join("-")
}

/// Adds or changes the model `body` describes (`id` when editing, `label`,
/// `agent`, `model`, `base_url`, and `key`, which a blank leaves as it is).
/// The new settings are checked before they are written. Returns its id.
pub(crate) fn save(config: &Path, keys: &Path, body: &Value) -> Result<String, Failure> {
    let field = |k: &str| body[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    let required = |k: &str| field(k).ok_or_else(|| Failure(format!("E7001 {k} required"), 1));
    let (label, agent, model, base_url) = (required("label")?, required("agent")?, required("model")?, required("base_url")?);
    let kind = match agent {
        "codex" => "openai",
        "claude" => "anthropic",
        _ => return err("E7001", "agent must be codex or claude"),
    };
    let id = field("id").map(String::from).unwrap_or_else(|| slug(label));
    if id.is_empty() {
        return err("E7001", "label needs a letter or digit");
    }
    let _lock = settings_lock(config)?;
    let mut doc = settings(config)?;
    if !doc.contains_key("models") {
        doc.insert("models".into(), toml::Value::Array(vec![]));
    }
    let invalid = || Failure("E6020 invalid system settings: models must be a list".into(), 1);
    let models = doc.get_mut("models").and_then(|v| v.as_array_mut()).ok_or_else(invalid)?;
    let at = models.iter().position(|m| m.get("id").and_then(|v| v.as_str()) == Some(id.as_str()));
    let mut entry = at.and_then(|i| models[i].as_table().cloned()).unwrap_or_default();
    let mut endpoint = entry.get("endpoint").and_then(|e| e.as_table().cloned()).unwrap_or_default();
    endpoint.insert("kind".into(), kind.into());
    endpoint.insert("base_url".into(), base_url.into());
    // Without a variable of its own, the endpoint's key is Solace's to keep.
    if !endpoint.contains_key("key_env") {
        endpoint.insert("stored_key".into(), true.into());
    }
    entry.remove("run");
    for (k, v) in [("id", id.as_str()), ("label", label), ("agent", agent), ("model", model)] {
        entry.insert(k.into(), v.into());
    }
    entry.insert("endpoint".into(), toml::Value::Table(endpoint));
    match at {
        Some(i) => models[i] = toml::Value::Table(entry),
        None => models.push(toml::Value::Table(entry)),
    }
    let text = toml::to_string(&doc).map_err(|e| Failure(e.to_string(), 1))?;
    parse_system(&text)?;
    replace(config, text.as_bytes())?;
    if let Some(key) = field("key") {
        set_key(keys, &id, Some(key))?;
    }
    agents::forget_local();
    Ok(id)
}

/// Removes configured model `id` and its stored key.
pub(crate) fn remove(config: &Path, keys: &Path, id: &str) -> Result<(), Failure> {
    let _lock = settings_lock(config)?;
    let mut doc = settings(config)?;
    let Some(models) = doc.get_mut("models").and_then(|v| v.as_array_mut()) else {
        return err("E7001", format!("no configured model {id}"));
    };
    let before = models.len();
    models.retain(|m| m.get("id").and_then(|v| v.as_str()) != Some(id));
    if models.len() == before {
        return err("E7001", format!("no configured model {id}"));
    }
    replace(config, toml::to_string(&doc).map_err(|e| Failure(e.to_string(), 1))?.as_bytes())?;
    set_key(keys, id, None)?;
    agents::forget_local();
    Ok(())
}

/// Turns system language `id` on or off, preserving the rest of the settings.
pub(crate) fn save_language(config: &Path, body: &Value) -> Result<(), Failure> {
    let id = body["id"]
        .as_str()
        .filter(|id| matches!(*id, "gleam" | "rust"))
        .ok_or_else(|| Failure("E7001 language must be gleam or rust".into(), 1))?;
    let enabled = body["enabled"].as_bool().ok_or_else(|| Failure("E7001 enabled must be true or false".into(), 1))?;
    edit_settings(config, |doc| {
        let languages = doc.entry("languages").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let languages =
            languages.as_table_mut().ok_or_else(|| Failure("E6020 invalid system settings: languages must be a table".into(), 1))?;
        languages.insert(id.to_string(), toml::Value::Boolean(enabled));
        Ok(())
    })
}

/// The lock on `config`'s changes, so a read-modify-write can't lose another
/// writer's update.
pub(super) fn settings_lock(config: &Path) -> Result<fs::File, Failure> {
    let path = config.with_extension("lock");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    #[cfg(test)]
    if let Some(attempt) = LOCK_ATTEMPT.lock().unwrap().as_ref() {
        if fsx::try_lock_file(&path)?.is_none() {
            let _ = attempt.send(path.clone());
        }
    }
    fsx::lock_file(&path)
}

#[cfg(test)]
/// Test hook: receives a lock path when a writer finds it already held.
pub(super) static LOCK_ATTEMPT: std::sync::Mutex<Option<std::sync::mpsc::Sender<PathBuf>>> = std::sync::Mutex::new(None);

/// The context window an endpoint reports for its model: `GET /v1/models`,
/// `meta.n_ctx` (llama.cpp, Strata). Claude Code doesn't know a local model's
/// window, so without it, it compacts too late and the server refuses the prompt.
pub(crate) fn endpoint_context(base_url: &str, model: &str) -> Option<u64> {
    // plain http only; an https endpoint goes undetected.
    let rest = base_url.strip_prefix("http://")?;
    let (host, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    let path = [prefix.trim_matches('/'), "v1/models"].iter().filter(|p| !p.is_empty()).fold(String::new(), |a, p| a + "/" + p);
    let addr = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&addr).ok()?.next()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n")?.1).ok()?;
    let models = body["data"].as_array()?;
    // A server with one model often serves it under any name.
    let m = models.iter().find(|x| x["id"] == model).or(if models.len() == 1 { models.first() } else { None })?;
    m["meta"]["n_ctx"].as_u64()
}

/// One entry per Claude family: the family's configured entry, then an exact model the
/// provider discovered, then its alias, then a shipped exact preset. Among discovered
/// models the provider's newest-first order decides. A gone entry stays below every live
/// one. Codex answers exact ids, so its entries, and a configured model whose id names no
/// family, keep their places. The alias stays in the settings, so a task naming it still
/// starts (`model`).
pub(crate) fn served(models: &[Value]) -> Vec<Value> {
    let mut best: HashMap<&str, usize> = HashMap::new();
    for (i, m) in models.iter().enumerate() {
        let Some(f) = agents::family(m) else { continue };
        if best.get(f).is_none_or(|&j| outranks(m, &models[j], f)) {
            best.insert(f, i);
        }
    }
    models
        .iter()
        .enumerate()
        .filter(|(i, m)| match agents::family(m) {
            Some(f) => best[f] == *i,
            None => true,
        })
        .map(|(_, m)| m.clone())
        .collect()
}

/// How well a live entry stands for its family: the human's configured entry, then an
/// exact model the provider discovered, then the alias, then a shipped exact preset.
/// A gone entry sits below every live one; a gone alias above a gone exact model.
pub(crate) fn rank(m: &Value, f: &str) -> Rank {
    if m["gone"] == true {
        return if agents::is_alias(m, f) { Rank::GoneAlias } else { Rank::GoneExact };
    }
    if m["configured"] == true {
        Rank::LiveConfigured
    } else if m["discovered"] == true {
        Rank::LiveDiscovered
    } else if agents::is_alias(m, f) {
        Rank::LiveAlias
    } else {
        Rank::LivePreset
    }
}

/// The system settings file: `SOLACE_CONFIG`, else `~/.config/solace/config.toml`.
pub(crate) fn config_path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_CONFIG.with(|p| p.borrow().clone()) {
        return path;
    }
    env::var_os("SOLACE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config/solace/config.toml"))
}

pub fn system() -> Result<Value, Failure> {
    let p = config_path();
    let mut v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    // Each agent's discovered models merge in: presets, then discovery, then
    // the human's config, which wins for the same id.
    v["models"] = json!(merge(&v["models"]));
    Ok(v)
}

pub(crate) fn parse_system(text: &str) -> Result<Value, Failure> {
    let v: toml::Value = text.parse().map_err(|e| Failure(format!("E6020 system settings: {e}"), 1))?;
    let mut v = serde_json::to_value(v).map_err(|e| Failure(e.to_string(), 1))?;
    let invalid = || {
        Failure(
            "E6020 invalid system settings: expected positive task_timeout, a [languages] table of true/false gleam and rust, \
             and [[models]] with id, label, agent and model or run argv; an endpoint needs kind \"openai\" (agent codex) \
             or \"anthropic\" (agent claude) and base_url"
                .into(),
            1,
        )
    };
    // Built-in presets first; configured models with the same id replace them.
    let mut models: Vec<Value> = if v["presets"] == false {
        vec![]
    } else {
        let presets: toml::Value = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/default-models.toml")).parse().unwrap();
        serde_json::to_value(presets).unwrap()["models"].as_array().unwrap().clone()
    };
    for mut m in v.get("models").map(|m| m.as_array().cloned().ok_or_else(invalid)).transpose()?.unwrap_or_default() {
        // A configured model keeps its entry when a discovery names its id.
        m["configured"] = json!(true);
        match models.iter().position(|x| x["id"] == m["id"]) {
            Some(i) => models[i] = m,
            None => models.push(m),
        }
    }
    v["models"] = json!(models);
    if v.get("task_timeout").is_none() {
        v["task_timeout"] = json!(3600);
    }
    if v.get("task_timeout").is_some_and(|v| v.as_u64().is_none_or(|n| n == 0)) {
        return Err(invalid());
    }
    if let Some(languages) = v.get("languages") {
        let Some(languages) = languages.as_object() else {
            return Err(invalid());
        };
        if languages.keys().any(|id| !matches!(id.as_str(), "gleam" | "rust")) || languages.values().any(|enabled| !enabled.is_boolean()) {
            return Err(invalid());
        }
    }
    let languages = json!({
        "gleam": v.get("languages").and_then(|l| l.get("gleam")).cloned().unwrap_or(json!(true)),
        "rust": v.get("languages").and_then(|l| l.get("rust")).cloned().unwrap_or(json!(true)),
    });
    v["languages"] = languages;
    let involvement = agent_choice(&v, "involvement", &INVOLVEMENT_VALUES, DEFAULT_INVOLVEMENT);
    let detail = agent_choice(&v, "detail", &DETAIL_VALUES, "standard");
    let asking = agent_choice(&v, "asking", &ASKING_VALUES, "balanced");
    v["involvement"] = involvement;
    v["detail"] = detail;
    v["asking"] = asking;
    if let Some(models) = v.get("models") {
        let mut ids = HashSet::new();
        for m in models.as_array().ok_or_else(invalid)? {
            if m["id"].as_str().is_none_or(|id| id.is_empty() || !ids.insert(id))
                || !m["label"].is_string()
                || !matches!(m["agent"].as_str(), Some("codex" | "claude"))
            {
                return Err(invalid());
            }
            if let Some(argv) = m.get("run") {
                if argv.as_array().is_none_or(|a| a.is_empty() || a.iter().any(|v| !v.is_string())) {
                    return Err(invalid());
                }
            } else if !m["model"].is_string() {
                return Err(invalid());
            }
            if let Some(e) = m.get("endpoint") {
                let kind_matches = matches!(
                    (e["kind"].as_str(), m["agent"].as_str()),
                    (Some("openai"), Some("codex")) | (Some("anthropic"), Some("claude"))
                );
                if !kind_matches
                    || !e["base_url"].is_string()
                    || e.get("key_env").is_some_and(|k| !k.is_string())
                    || e.get("stored_key").is_some_and(|k| !k.is_boolean())
                {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(v)
}

/// Codex settings for a model's OpenAI-compatible endpoint, if it has one. The
/// key stays in the variable the endpoint names, or with a `stored` key in
/// `setup::KEY_VAR`, set on the agent's process; Codex reads it at launch.
/// Current Codex speaks only the Responses API, so the endpoint must serve it.
pub(crate) fn endpoint_args(m: &Value, stored: bool) -> Vec<String> {
    let Some(e) = m.get("endpoint") else {
        return vec![];
    };
    let mut provider = json!({"name": m["label"], "base_url": e["base_url"], "wire_api": "responses"});
    if stored {
        provider["env_key"] = json!(KEY_VAR);
    } else if let Some(k) = e["key_env"].as_str() {
        provider["env_key"] = json!(k);
    }
    let fields: Vec<String> = provider.as_object().unwrap().iter().map(|(k, v)| format!("{k} = {v}")).collect();
    vec![
        "-c".into(),
        "model_provider=\"solace_endpoint\"".into(),
        "-c".into(),
        format!("model_providers.solace_endpoint={{ {} }}", fields.join(", ")),
    ]
}

pub(crate) fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join(program).is_file()))
}

pub(crate) fn model(config: &Value, name: &str) -> Result<Value, Failure> {
    config["models"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["id"] == name))
        .cloned()
        .ok_or_else(|| Failure(format!("E7001 unknown model {name}; configure system [[models]]"), 1))
}

/// The model a request names in `model`, with the `effort` it asks for, checked
/// as an action's are: an unknown model or an effort it doesn't offer is
/// `E7001`, and an unavailable model `E7011`.
pub(crate) fn chosen_model(config: &Value, body: &Value) -> Result<Value, Failure> {
    let Some(id) = body["model"].as_str() else { return err("E7001", "a model is required") };
    let mut m = model(config, id)?;
    if let Some(e) = body.get("effort").filter(|e| !e.is_null()) {
        let e = e
            .as_str()
            .filter(|e| efforts(&m).iter().any(|x| x == e))
            .ok_or_else(|| Failure(format!("E7001 effort not offered by {id}; offered: {}", efforts(&m).join(", ")), 1))?;
        m["effort"] = json!(e);
    }
    if let Some(reason) = unavailable(&m) {
        return err("E7011", format!("model {id} is unavailable: {reason}"));
    }
    Ok(m)
}

/// Efforts an action may choose for a model. Custom runners take none. A
/// discovered model carries the efforts its agent says it supports; the fixed
/// lists stand behind the presets and the aliases.
pub(crate) fn efforts(m: &Value) -> Vec<String> {
    if let Some(e) = m["efforts"].as_array() {
        return e.iter().filter_map(|e| e.as_str().map(String::from)).collect();
    }
    let fixed: &[&str] = match (m["run"].is_null(), m["agent"].as_str()) {
        (true, Some("codex")) => &["low", "medium", "high", "xhigh"],
        (true, Some("claude")) => &["low", "medium", "high", "xhigh", "max"],
        _ => &[],
    };
    fixed.iter().map(|e| e.to_string()).collect()
}

/// The models this Solace install knows; the same for every project. One entry
/// per Claude family, with why it can't start; `retry`: the reason is a mark a
/// human may clear ("Try again", `POST /models/try-again`); `sign_in`: that
/// mark is an expired sign-in.
pub(crate) fn models() -> Result<Value, Failure> {
    // Serve the last list; ask a kind whose answer is stale, in the background.
    refresh_stale();
    let all = system()?["models"].as_array().cloned().unwrap_or_default();
    let keys = read_keys(&keys_path());
    Ok(json!(
        served(&all)
            .iter()
            .map(|m| {
                let status = crate::board::agents::availability(m);
                let retry = status.as_ref().is_some_and(|s| s.retry);
                let sign_in = status.as_ref().is_some_and(|s| s.sign_in);
                let reason = status.map(|s| s.reason);
                // Whether a key is stored, never the key.
                let has_key = m["id"].as_str().is_some_and(|id| keys[id].is_string());
                json!({"id":m["id"],"label":m["label"],"model":m["model"],"agent":m["agent"],"effort":m["effort"],"efforts":efforts(m),
                    "available":reason.is_none(),"reason":reason,"retry":retry,"sign_in":sign_in,
                    "configured":m["configured"] == true,"base_url":m["endpoint"]["base_url"],"has_key":has_key})
            })
            .collect::<Vec<_>>()
    ))
}

/// The configured models (presets first, then `[[models]]`) with each kind's
/// discovered models merged in: a discovered model replaces a preset of the
/// same id, and a configured model keeps its entry — the human's config wins.
/// Models a discovery dropped stay, marked `gone`, so tasks already using them
/// keep working; they are not offered for a new run. Once a run has named the
/// model it used, an alias's label is that run's stored label.
pub(super) fn merge(configured: &Value) -> Vec<Value> {
    merge_from(configured, &saved_models(&models_path()))
}

/// How old a kind's answer may be before `/models` asks the program again.
const STALE: u64 = 15 * 60;

/// Asks each kind whose answer is `STALE` old, or that has none, in the
/// background; the caller serves the last list. A model error marks its kind
/// stale, so the next `/models` asks again.
pub(super) fn refresh_stale() {
    let all = saved_models(&models_path());
    for k in KINDS {
        if all[k.name]["time"].as_u64().is_none_or(|t| now().saturating_sub(t) >= STALE) && REFRESHING.lock().unwrap().insert(k.name) {
            std::thread::spawn(move || refresh(k));
        }
    }
}

/// Whether the system settings leave language `id` on; an absent setting is on.
pub(crate) fn language_enabled(id: &str) -> Result<bool, Failure> {
    language_enabled_at(&config_path(), id)
}

/// Why language `id` cannot use its tools, if it cannot: turned off first, then
/// a toolchain that is not installed.
pub(crate) fn language_reason(id: &str) -> Result<Option<String>, Failure> {
    if !crate::language::enabled(id)? {
        return Ok(Some(crate::language::off(id).to_string()));
    }
    if !crate::language::installed(id) {
        return Ok(Some(crate::language::missing(id).to_string()));
    }
    Ok(None)
}

/// Whether `config` leaves language `id` on; an absent setting is on.
fn language_enabled_at(config: &Path, id: &str) -> Result<bool, Failure> {
    let v = parse_system(&if config.exists() { fs::read_to_string(config)? } else { String::new() })?;
    Ok(v["languages"][id].as_bool().unwrap_or(true))
}

#[cfg(test)]
thread_local! {
    /// A test's system settings file, in place of `SOLACE_CONFIG` and the home folder's.
    static TEST_CONFIG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `config_path` reading `path`, so a test drives settings without
/// changing the process's environment.
#[cfg(test)]
pub(crate) fn with_config<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let old = TEST_CONFIG.with(|p| p.replace(Some(path.to_path_buf())));
    let out = f();
    TEST_CONFIG.with(|p| *p.borrow_mut() = old);
    out
}

/// The system languages and whether each is on and found on this computer.
pub(crate) fn languages() -> Result<Value, Failure> {
    let p = config_path();
    let v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    let mut out = vec![];
    for id in [crate::language::GLEAM, crate::language::RUST] {
        out.push(json!({
            "id": id,
            "label": crate::language::label(id),
            "enabled": v["languages"][id].as_bool().unwrap_or(true),
            "installed": crate::language::installed(id),
        }));
    }
    Ok(json!(out))
}

const INVOLVEMENT_VALUES: [&str; 3] = ["director", DEFAULT_INVOLVEMENT, "engineer"];

const DETAIL_VALUES: [&str; 3] = ["concise", "standard", "thorough"];

const ASKING_VALUES: [&str; 3] = ["often", "balanced", "rarely"];

/// Saves one agent behavior choice, preserving the rest of the settings.
pub(crate) fn save_agent_setting(config: &Path, body: &Value) -> Result<(), Failure> {
    let key = body["key"].as_str().unwrap_or("");
    let (allowed, message) = match key {
        "involvement" => (&INVOLVEMENT_VALUES[..], "involvement must be director, collaborator or engineer"),
        "detail" => (&DETAIL_VALUES[..], "detail must be concise, standard or thorough"),
        "asking" => (&ASKING_VALUES[..], "asking must be often, balanced or rarely"),
        _ => return err("E7001", "key must be involvement, detail or asking"),
    };
    let value = body["value"].as_str().filter(|value| allowed.contains(value)).ok_or_else(|| Failure(format!("E7001 {message}"), 1))?;
    edit_settings(config, |doc| {
        doc.insert(key.to_string(), toml::Value::String(value.to_string()));
        Ok(())
    })
}

/// The machine-wide agent behavior settings: how agents address the human,
/// how much they write, and how readily they ask. A value that is missing or
/// unknown is its default.
pub(crate) fn agent_settings() -> Result<Value, Failure> {
    let p = config_path();
    let v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    Ok(json!({
        "involvement": v["involvement"],
        "detail": v["detail"],
        "asking": v["asking"],
    }))
}

fn agent_choice(v: &Value, key: &str, allowed: &[&str], default: &str) -> Value {
    json!(v.get(key).and_then(Value::as_str).filter(|value| allowed.contains(value)).unwrap_or(default))
}

fn edit_settings(config: &Path, edit: impl FnOnce(&mut toml::Table) -> Result<(), Failure>) -> Result<(), Failure> {
    let _lock = settings_lock(config)?;
    let mut doc = settings(config)?;
    edit(&mut doc)?;
    let text = toml::to_string(&doc).map_err(|e| Failure(e.to_string(), 1))?;
    parse_system(&text)?;
    replace(config, text.as_bytes())
}

/// The involvement level a brief uses; `parse_system` has already supplied its default.
pub(crate) fn involvement(settings: &Value) -> &str {
    settings["involvement"].as_str().unwrap_or(DEFAULT_INVOLVEMENT)
}

/// The detail level a brief uses; `parse_system` has already supplied its default.
pub(crate) fn detail(settings: &Value) -> &str {
    settings["detail"].as_str().unwrap_or("standard")
}

/// How readily a brief asks; `parse_system` has already supplied its default.
pub(crate) fn asking(settings: &Value) -> &str {
    settings["asking"].as_str().unwrap_or("balanced")
}

/// The involvement level agents use when the settings name none.
const DEFAULT_INVOLVEMENT: &str = "collaborator";

/// Whether `m` outranks `current` as family `f`'s pick: a higher rank, or the earlier
/// discovery when both are exact models the provider listed.
fn outranks(m: &Value, current: &Value, f: &str) -> bool {
    let (new, old) = (rank(m, f), rank(current, f));
    new > old
        || (new == old
            && new == Rank::LiveDiscovered
            && m["found_order"].as_u64().unwrap_or(u64::MAX) < current["found_order"].as_u64().unwrap_or(u64::MAX))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Rank {
    GoneExact,
    GoneAlias,
    LivePreset,
    LiveAlias,
    LiveDiscovered,
    LiveConfigured,
}

/// The role models a system settings document holds: every role, with an
/// empty choice when it has none or its stored value is not text.
fn role_models_json(v: &Value) -> Value {
    let roles = crate::board::roles::ROLES.iter().map(|role| {
        let default =
            v.get("role_models").and_then(|r| r.get(role.id)).and_then(|s| s.get("default")).and_then(Value::as_str).unwrap_or("");
        json!({"role": role.id, "name": role.name, "default": default})
    });
    Value::Array(roles.collect())
}

/// Every role's model, as Settings reads it. A missing role or a value that
/// isn't text is empty.
pub(crate) fn role_models() -> Result<Value, Failure> {
    let p = config_path();
    let v = parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })?;
    Ok(role_models_json(&v))
}

/// Saves one role's model, preserving the rest of the settings. Unknown model
/// ids are kept: a model may return. An old backup is removed.
pub(crate) fn save_role_model(config: &Path, body: &Value) -> Result<(), Failure> {
    let role = body["role"]
        .as_str()
        .filter(|id| crate::board::roles::from_id(id).is_some())
        .ok_or_else(|| Failure("E7001 role must be a known role".into(), 1))?;
    let default = body["default"].as_str().ok_or_else(|| Failure("E7001 default must be a model id".into(), 1))?;
    edit_settings(config, |doc| {
        let roles = doc.entry("role_models").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let roles = roles.as_table_mut().ok_or_else(|| Failure("E6020 invalid system settings: role_models must be a table".into(), 1))?;
        let slots = roles.entry(role).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let slots =
            slots.as_table_mut().ok_or_else(|| Failure(format!("E6020 invalid system settings: role_models.{role} must be a table"), 1))?;
        slots.remove("backup");
        slots.insert("default".into(), toml::Value::String(default.into()));
        Ok(())
    })
}

