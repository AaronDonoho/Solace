//! Models a human sets up on the Settings page: their `[[models]]` entries in
//! the system settings, and the API keys pasted for them. A key is kept by
//! model id in `keys.json`, beside the system settings, readable only by its
//! owner (mode 0600); never in `config.toml`, and never sent back to a page.
use super::*;
use std::os::unix::fs::OpenOptionsExt;

use serde_json::Value;
use serde_json::json;
/// The variable a stored key reaches Codex through, on the agent's process only.
pub(super) const KEY_VAR: &str = "SOLACE_ENDPOINT_KEY";

/// Where pasted keys are kept, beside the system settings.
pub(super) fn keys_path() -> PathBuf {
    config_path().with_file_name("keys.json")
}

pub(super) fn read_keys(path: &Path) -> Value {
    jsonx::read_opt(path).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The key stored for model `id`, if any.
pub(super) fn stored(path: &Path, id: &str) -> Option<String> {
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

/// The key model `m`'s endpoint uses: the stored one, else its `key_env` variable's.
pub(super) fn endpoint_key(m: &Value) -> Option<String> {
    m["id"].as_str().and_then(|id| stored(&keys_path(), id)).or_else(|| m["endpoint"]["key_env"].as_str().and_then(|k| env::var(k).ok()))
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
pub(super) fn save(config: &Path, keys: &Path, body: &Value) -> Result<String, Failure> {
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
pub(super) fn remove(config: &Path, keys: &Path, id: &str) -> Result<(), Failure> {
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
pub(super) fn save_language(config: &Path, body: &Value) -> Result<(), Failure> {
    let id = body["id"]
        .as_str()
        .filter(|id| matches!(*id, "gleam" | "rust"))
        .ok_or_else(|| Failure("E7001 language must be gleam or rust".into(), 1))?;
    let enabled = body["enabled"].as_bool().ok_or_else(|| Failure("E7001 enabled must be true or false".into(), 1))?;
    let _lock = settings_lock(config)?;
    let mut doc = settings(config)?;
    let languages = doc.entry("languages").or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let languages =
        languages.as_table_mut().ok_or_else(|| Failure("E6020 invalid system settings: languages must be a table".into(), 1))?;
    languages.insert(id.to_string(), toml::Value::Boolean(enabled));
    let text = toml::to_string(&doc).map_err(|e| Failure(e.to_string(), 1))?;
    parse_system(&text)?;
    replace(config, text.as_bytes())
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
