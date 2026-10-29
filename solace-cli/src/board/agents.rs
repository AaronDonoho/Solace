//! The agents Solace runs, and whether each can start right now. Each agent
//! kind is one entry in `KINDS`: its program, where its sign-in is kept, and
//! how it says a usage limit was reached or its sign-in expired, so another
//! harness is one more entry. The checks are local and make no model call. A
//! hold learned from a run, a usage limit or an expired sign-in, is kept in
//! `agent-holds.json`, beside the system settings, so it outlasts a restart.
use super::*;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use serde_json::json;
/// One agent kind.
pub(super) struct Kind {
    /// The `agent` a model names, which is also its program.
    pub name: &'static str,
    /// What the human calls it.
    pub label: &'static str,
    /// The folder holding this agent's sign-in files.
    pub folder: fn() -> PathBuf,
    /// Sign-in files, from that folder.
    pub files: &'static [&'static str],
    /// The program's arguments that say whether it is signed in, with no
    /// model call; empty when it has none.
    pub status: &'static [&'static str],
    /// Variables that may hold an API key.
    pub keys: &'static [&'static str],
    /// Whether a key in `keys` is a sign-in even when the status command
    /// says signed out.
    pub keys_override_status: bool,
    /// The macOS keychain item that may hold the sign-in instead of a file.
    pub keychain: Option<&'static str>,
    /// Whether a sign-in file says its credentials have expired, at `now` (ms).
    pub expired: fn(&Value, u64) -> bool,
    /// Whether an error is this agent saying a usage limit was reached, and
    /// then the reset time it names, if any.
    pub limit: fn(&str) -> Option<Option<String>>,
    /// Whether an error is this agent saying its sign-in has expired.
    pub signed_out: fn(&str) -> bool,
    /// Whether a message is exactly this agent saying its sign-in expired.
    pub signed_out_whole: fn(&str) -> bool,
    /// How this agent says it needs an update: whether an error says so, and
    /// whether a message is exactly that; none when it says so some other way.
    pub update: Option<(fn(&str) -> bool, fn(&str) -> bool)>,
    /// The models this agent offers now, asked of the program itself; an
    /// empty answer keeps the presets and the configured models.
    pub discover: fn() -> Vec<Value>,
    /// Whether an error says the model a run asked for is off the offer.
    pub gone: fn(&str) -> bool,
}

pub(super) const KINDS: &[Kind] = &[
    Kind {
        name: "codex",
        label: "Codex",
        folder: codex_folder,
        files: &["auth.json"],
        status: &["login", "status"],
        keys: &["OPENAI_API_KEY"],
        keys_override_status: false,
        keychain: None,
        expired: codex_expired,
        limit: codex_limit,
        signed_out: codex_signed_out,
        signed_out_whole: codex_signed_out_whole,
        update: None,
        discover: codex_models,
        gone: model_gone,
    },
    Kind {
        name: "claude",
        label: "Claude Code",
        folder: super::claude_home,
        files: &[".credentials.json"],
        status: &["auth", "status"],
        keys: &["ANTHROPIC_API_KEY"],
        keys_override_status: true,
        keychain: Some("Claude Code-credentials"),
        expired: claude_expired,
        limit: claude_limit,
        signed_out: claude_signed_out,
        signed_out_whole: claude_signed_out_whole,
        update: Some((claude_update, claude_update_whole)),
        discover: claude_models,
        gone: model_gone,
    },
];

pub(super) fn kind(agent: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.name == agent)
}

// Discovery: asking each agent program which models it offers, so a new
// model shows up without a rebuild or a config edit. Each kind's answer, with
// the time it came, is kept in `agent-models.json`, beside the system
// settings: `/models` serves the last list at once and asks a kind again in
// the background when its answer is `STALE` old, at startup, and after a run's
// log says a model is off the offer. A model a discovery dropped stays in the
// list, marked `gone`, so tasks already using it keep working; it is not
// offered for a new run.

/// Where each kind's last answer is kept, beside the system settings.
pub(super) fn models_path() -> PathBuf {
    config_path().with_file_name("agent-models.json")
}

fn saved_models(path: &Path) -> Value {
    cached(&format!("models:{}", path.display()), || jsonx::read_opt(path).filter(Value::is_object).unwrap_or_else(|| json!({})))
}

fn save_models(path: &Path, all: &Value) -> Result<(), Failure> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(all).unwrap())?;
    fs::rename(tmp, path)?;
    CACHE.lock().unwrap().retain(|k, _| !k.starts_with("models:"));
    Ok(())
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

pub(super) fn merge_from(configured: &Value, all: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = configured.as_array().cloned().unwrap_or_default();
    for k in KINDS {
        for m in all[k.name]["models"].as_array().unwrap_or(&vec![]).iter() {
            let m = &plain_label(m);
            match out.iter().position(|x| x["id"] == m["id"]) {
                Some(i) if out[i]["configured"] == true => {}
                Some(i) => out[i] = m.clone(),
                None => out.push(m.clone()),
            }
        }
    }
    for m in all["gone"].as_object().unwrap_or(&Default::default()).values() {
        if !out.iter().any(|x| x["id"] == m["id"]) {
            let mut m = plain_label(m);
            m["gone"] = json!(true);
            out.push(m);
        }
    }
    for m in &mut out {
        m["effort"] = json!(default_effort(m));
        if names_family(m) {
            let used = &all["used"][m["id"].as_str().unwrap_or("")];
            if let Some(label) = used["label"].as_str().or(used.as_str()).or(used["model"].as_str()) {
                m["label"] = json!(label);
            }
        }
    }
    out
}

/// The effort a run takes when none is chosen: the one the agent or the human's
/// config names, else `medium` when offered, else the middle of the offer.
/// Custom runners offer none, so they take none.
pub(super) fn default_effort(m: &Value) -> Option<String> {
    let offered = super::efforts(m);
    if let Some(e) = m["effort"].as_str().filter(|e| offered.iter().any(|x| x == e)) {
        return Some(e.to_string());
    }
    if offered.iter().any(|x| x == "medium") {
        return Some("medium".into());
    }
    offered.get(offered.len() / 2).cloned()
}

/// Saved model `m` with its label as D-150 has it: a discovery saved before then
/// named the agent first ("Claude Code · Opus 5.5"), which the next one drops.
fn plain_label(m: &Value) -> Value {
    let mut m = m.clone();
    if let Some(l) = m["label"].as_str().and_then(|l| ["Claude Code · ", "Codex · "].iter().find_map(|p| l.strip_prefix(p))) {
        m["label"] = json!(l);
    }
    m
}

/// How old a kind's answer may be before `/models` asks the program again.
const STALE: u64 = 15 * 60;
/// How long one Codex discovery may take in all, from spawning it to its last page.
const CODEX_DEADLINE: Duration = Duration::from_secs(60);
/// Kinds with a refresh going now, so one request starts one thread per kind.
static REFRESHING: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
/// One writer at a time for `agent-models.json`: each writer rewrites the
/// whole file from a snapshot, so two at once would lose one kind's answer.
static MODELS: LazyLock<Mutex<()>> = LazyLock::new(Default::default);

/// Asks every kind for its models now: the server's startup refresh.
pub(super) fn refresh_all() {
    for k in KINDS {
        // Register the kind first, as `refresh_stale` does: a `/models` poll
        // during the startup refresh must not start a second refresh for it.
        if REFRESHING.lock().unwrap().insert(k.name) {
            refresh(k);
        }
    }
}

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

fn refresh(k: &'static Kind) {
    let found = (k.discover)();
    if !found.is_empty()
        && let Err(e) = note_models(&models_path(), k, found)
    {
        eprintln!("solace: could not record {}'s models: {}", k.label, e.0);
    }
    REFRESHING.lock().unwrap().remove(k.name);
}

/// Records kind `k`'s answer `found`: models its offer dropped become `gone`
/// (kept for the tasks using them), and models back on the offer are not.
fn note_models(path: &Path, k: &'static Kind, found: Vec<Value>) -> Result<(), Failure> {
    let _writer = MODELS.lock().unwrap();
    let mut all = saved_models(path);
    let mut gone = all["gone"].as_object().cloned().unwrap_or_default();
    for m in all[k.name]["models"].as_array().unwrap_or(&vec![]).iter() {
        let id = m["id"].as_str().unwrap_or("");
        if !id.is_empty() && !found.iter().any(|x| x["id"] == id) {
            gone.insert(
                id.to_string(),
                json!({
                    "id": id,
                    "label": m["label"],
                    "agent": m["agent"],
                    "model": m["model"],
                    "reason": format!("no longer offered by {}", k.label),
                }),
            );
        }
    }
    for m in &found {
        gone.remove(m["id"].as_str().unwrap_or(""));
    }
    all[k.name] = json!({"time": now(), "models": found});
    all["gone"] = Value::Object(gone);
    save_models(path, &all)
}

/// Records that model `model` resolved to `used`, with `label` as the plain
/// text the run showed.
pub(super) fn record_used(model: &str, used: &str, label: &str) {
    let _writer = MODELS.lock().unwrap();
    let path = models_path();
    let mut all = saved_models(&path);
    let record = json!({"model": used, "label": label});
    if all["used"][model] != record {
        all["used"][model] = record;
        if let Err(e) = save_models(&path, &all) {
            eprintln!("solace: could not record the model a run used: {}", e.0);
        }
    }
}

/// Marks kind `agent`'s answer stale, so the next `/models` asks the program.
fn mark_stale(agent: &str) {
    let _writer = MODELS.lock().unwrap();
    let path = models_path();
    let mut all = saved_models(&path);
    if all[agent].is_object() {
        all[agent]["time"] = json!(0);
        if let Err(e) = save_models(&path, &all) {
            eprintln!("solace: could not mark {agent}'s models stale: {}", e.0);
        }
    }
}

/// Codex's models, asked of `codex app-server` over its stdio: JSON-RPC
/// `initialize`, then `model/list`, paging by cursor. Hidden models are left
/// out; each model keeps its slug (what `-m` takes), the efforts Codex says it
/// supports and its default effort. No answer keeps the presets.
fn codex_models() -> Vec<Value> {
    if !on_path("codex") {
        return vec![];
    }
    let Ok(mut child) = Command::new("codex").arg("app-server").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
    else {
        return vec![];
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return vec![];
    };
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    // One deadline for the whole discovery, not one per line: a program that
    // answers slowly must not make the pages add up to hours of waiting.
    let deadline = Instant::now() + CODEX_DEADLINE;
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let answer = |stdin: &mut std::process::ChildStdin, id: u64, method: &str, params: Value| -> Option<Value> {
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})).ok()?;
        stdin.flush().ok()?;
        for _ in 0..400 {
            let line = rx.recv_timeout(deadline.checked_duration_since(Instant::now())?).ok()?;
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v["id"] == json!(id) {
                    return Some(v["result"].clone());
                }
            }
        }
        None
    };
    let mut models = vec![];
    let mut asked = false;
    if answer(&mut stdin, 1, "initialize", json!({"clientInfo":{"name":"solace","version":"1"}})).is_some() {
        let _ = writeln!(stdin, "{}", json!({"jsonrpc":"2.0","method":"initialized"}));
        let _ = stdin.flush();
        asked = true;
        let mut cursor: Option<String> = None;
        for page in 0..20 {
            let mut params = json!({"includeHidden": false, "limit": 100});
            if let Some(c) = &cursor {
                params["cursor"] = json!(c);
            }
            let Some(r) = answer(&mut stdin, 2 + page, "model/list", params) else {
                asked = false;
                break;
            };
            models.extend(codex_page(&r));
            cursor = r["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if asked { models } else { vec![] }
}

/// The models one `model/list` answer offers: hidden ones are left out, and
/// each keeps its slug, its efforts and its default effort.
pub(super) fn codex_page(r: &Value) -> Vec<Value> {
    r["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter(|m| m["hidden"] != true)
        .filter_map(|m| {
            let slug = m["model"].as_str()?;
            let efforts: Vec<String> = m["supportedReasoningEfforts"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|e| e["effort"].as_str().map(String::from))
                .collect();
            Some(json!({
                "id": format!("codex-{slug}"),
                "label": m["displayName"].as_str().unwrap_or(slug),
                "agent": "codex",
                "model": slug,
                "effort": m["defaultReasoningEffort"],
                "efforts": efforts,
            }))
        })
        .collect()
}

/// Claude Code's models: its aliases, each naming the latest of one family,
/// plus the exact ids the provider's models endpoint lists when an API key is
/// set. Solace stores no key; it reads the variable that is already set.
fn claude_models() -> Vec<Value> {
    if !on_path("claude") {
        return vec![];
    }
    let mut out = claude_aliases();
    if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            out.extend(api_models(&key));
        }
    }
    out
}

/// Claude's model families, with the label each alias shows: an exact id names one
/// family; an alias names the latest of one.
const FAMILIES: [(&str, &str); 4] = [("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku"), ("fable", "Fable")];

/// The aliases Claude Code resolves to the latest of each family.
pub(super) fn claude_aliases() -> Vec<Value> {
    FAMILIES.iter().map(|(a, l)| json!({"id":format!("claude-{a}"),"label":format!("{l} (latest)"),"agent":"claude","model":a})).collect()
}

/// The family a model entry belongs to: Claude's families only — Codex answers exact ids.
/// A discovered model names its family in `family` (the endpoint's `line`); a preset and an
/// alias name it in their ids, and so does a configured model: `claude-opus-local` is an Opus
/// and can stand behind the family's live model (D-150). A configured id with no family word
/// is its own entry.
pub(super) fn family(m: &Value) -> Option<&'static str> {
    if m["agent"] != "claude" {
        return None;
    }
    if let Some(f) = m["family"].as_str().and_then(|l| FAMILIES.iter().map(|(f, _)| *f).find(|f| *f == l)) {
        return Some(f);
    }
    FAMILIES
        .iter()
        .map(|(f, _)| *f)
        .find(|f| m["id"].as_str().is_some_and(|id| id.starts_with("claude-") && id.split('-').any(|p| p == *f)))
}

/// The entry naming the latest of family `f`, rather than one exact model: decided by its id.
pub(super) fn is_alias(m: &Value, f: &str) -> bool {
    m["id"].as_str() == Some(&format!("claude-{f}"))
}

/// An entry whose `model` is a family word, whatever its id: the agent resolves it to the latest
/// of that family, so a run's model is known only once its start event names one.
pub(super) fn names_family(m: &Value) -> bool {
    m["model"].as_str().is_some_and(|model| FAMILIES.iter().any(|(f, _)| *f == model))
}

/// The models the provider's models endpoint lists for the key: the active
/// ones, with the efforts its capability flags name.
fn api_models(key: &str) -> Vec<Value> {
    // The key goes to curl in the child's own environment, not in its command
    // line, where every user's `ps` would show it.
    let Ok(out) = Command::new("sh")
        .arg("-c")
        .arg(concat!(
            "curl -s --max-time 15 -H \"x-api-key: $ANTHROPIC_API_KEY\"",
            " -H \"anthropic-version: 2023-06-01\" https://api.anthropic.com/v1/models",
        ))
        .env("ANTHROPIC_API_KEY", key)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return vec![];
    };
    serde_json::from_slice::<Value>(&out.stdout).map_or(vec![], |b| api_page(&b))
}

/// The models one models-endpoint answer lists: active only, each with the
/// efforts its capability flags name and the family its `line` names.
pub(super) fn api_page(body: &Value) -> Vec<Value> {
    body["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter(|m| m["lifecycle"].as_str().unwrap_or("active") == "active")
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let efforts = ["low", "medium", "high", "xhigh", "max"]
                .iter()
                .filter(|e| m["capabilities"]["effort"][*e] == true)
                .map(|e| e.to_string())
                .collect::<Vec<_>>();
            let mut entry = json!({"id":id,"label":m["display_name"].as_str().unwrap_or(id),"agent":"claude","model":id,"efforts":efforts});
            if let Some(line) = m["line"].as_str() {
                entry["family"] = json!(line);
            }
            Some(entry)
        })
        .collect()
}

/// An error saying the model a run asked for is off the agent's offer: the
/// list is out of date, so ask the program again.
pub(super) fn model_gone(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    let metadata_warning = e.contains("metadata") && e.contains("not found");
    !metadata_warning
        && e.contains("model")
        && ["not found", "does not exist", "no such model", "unknown model", "is not available", "not supported"]
            .iter()
            .any(|p| e.contains(p))
}

/// The agent kind whose own sign-in model `m` uses: none for a model with its
/// own endpoint (and key) or its own program.
fn sign_in(m: &Value) -> Option<&'static Kind> {
    if m.get("endpoint").is_some() || !m["run"].is_null() {
        return None;
    }
    kind(m["agent"].as_str()?)
}

/// Codex's sign-in file holds neither an API key nor a token it can renew.
fn codex_expired(f: &Value, _now: u64) -> bool {
    f.is_object()
        && f["OPENAI_API_KEY"].as_str().is_none_or(str::is_empty)
        && f["tokens"]["refresh_token"].as_str().is_none_or(str::is_empty)
}

/// Claude Code's token has expired and there is no refresh token to renew it.
/// A token past `expiresAt` alone is normal: Claude Code renews it.
fn claude_expired(f: &Value, now: u64) -> bool {
    let o = &f["claudeAiOauth"];
    o["expiresAt"].as_u64().is_some_and(|t| t <= now) && o["refreshToken"].as_str().is_none_or(str::is_empty)
}

/// Codex's own messages (`UsageLimitReachedError` in codex-rs). The reset is
/// what follows "try again at", in this computer's local time.
fn codex_limit(error: &str) -> Option<Option<String>> {
    const SAID: [&str; 4] = [
        "You've hit your usage limit",
        "Your workspace is out of credits",
        "You hit your spend cap",
        "Quota exceeded. Check your plan and billing details",
    ];
    let flat = error.replace('’', "'");
    let line = flat.lines().map(str::trim).find(|l| SAID.iter().any(|s| l.starts_with(s)))?;
    Some(after(line, "try again at "))
}

/// Claude Code's messages: "You've hit your limit · resets 2pm (UTC)", and
/// older ones, "Claude usage limit reached. Your limit will reset at 3pm" and
/// "Claude AI usage limit reached|<seconds since the epoch>".
fn claude_limit(error: &str) -> Option<Option<String>> {
    let flat = error.replace('’', "'");
    for line in flat.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Claude AI usage limit reached|") {
            return Some(rest.trim().parse::<u64>().ok().map(utc_text));
        }
        if (line.starts_with("You've hit your") && line.contains("limit")) || line.starts_with("Claude usage limit reached") {
            return Some(after(line, "resets ").or_else(|| after(line, "reset at ")));
        }
    }
    None
}

/// What follows `marker` (ASCII, any case) in `line`, without a final period.
fn after(line: &str, marker: &str) -> Option<String> {
    let at = line.to_ascii_lowercase().find(marker)? + marker.len();
    Some(line[at..].trim().trim_end_matches('.').trim().to_string()).filter(|s| !s.is_empty())
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A time in seconds since the epoch, as a reset reads: "Oct 9, 2025 8:53 AM (UTC)".
fn utc_text(t: u64) -> String {
    let (y, m, d) = civil((t / 86400) as i64);
    let s = t % 86400;
    let (h, min) = (s / 3600, s % 3600 / 60);
    let (h12, half) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{} {d}, {y} {h12}:{min:02} {half} (UTC)", MONTHS[m as usize - 1])
}

/// Days since the epoch of a date (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let (m, d) = (m as i64, d as i64);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// The date of a day since the epoch: year, month, day.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Seconds into the day of a clock time: "2pm", "3:45pm", "17:00".
fn clock(c: &str) -> Option<i64> {
    let (digits, pm) = match (c.strip_suffix("pm"), c.strip_suffix("am")) {
        (Some(x), _) => (x, Some(true)),
        (_, Some(x)) => (x, Some(false)),
        _ => (c, None),
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None => (digits.parse::<i64>().ok()?, 0),
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    (h < 24 && m < 60).then_some(h * 3600 + m * 60)
}

/// When a reset an agent named comes, in seconds since the epoch. It is a
/// time ("2pm", "3:45 PM"), maybe after a date ("Oct 15th, 2026", "Oct 15"),
/// maybe followed by its time zone in parentheses ("(Europe/Paris)"); without
/// a zone it is this computer's. A time with no date is its next coming.
/// A date with no year whose day has passed this year is next year's.
/// `offset(zone, at)` gives a zone's offset from UTC in seconds at `at`
/// (`None`: the local zone); a reset is placed by the offset at the reset.
pub(super) fn reset_time(said: &str, now: u64, offset: &dyn Fn(Option<&str>, i64) -> Option<i64>) -> Option<u64> {
    let said = said.trim();
    let (text, zone) = match said.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
        Some((t, z)) => (t.trim(), Some(z.trim())),
        None => (said, None),
    };
    let off = offset(zone, now as i64)?;
    let words: Vec<String> = text.replace(',', " ").split_whitespace().map(str::to_ascii_lowercase).filter(|w| w != "at").collect();
    let (time, date) = match words.as_slice() {
        [date @ .., t, half] if half == "am" || half == "pm" => (format!("{t}{half}"), date),
        [date @ .., t] => (t.clone(), date),
        [] => return None,
    };
    let secs = clock(&time)?;
    let today = (now as i64 + off).div_euclid(86400);
    let month = |w: &str| MONTHS.iter().position(|m| w.starts_with(&m.to_ascii_lowercase())).map(|i| i as u32 + 1);
    let day = |w: &str| w.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<u32>().ok().filter(|d| (1..=31).contains(d));
    let at = |d: i64, off: i64| d * 86400 + secs - off;
    let t0 = match date {
        [] if at(today, off) > now as i64 => at(today, off),
        [] => at(today + 1, off),
        [m, d] => {
            let (month, day) = (month(m)?, day(d)?);
            let t = at(days_from_civil(civil(today).0, month, day), off);
            // A date with no year whose day has passed this year is next year's.
            if t <= now as i64 { at(days_from_civil(civil(today).0 + 1, month, day), off) } else { t }
        }
        [m, d, y] => at(days_from_civil(y.parse().ok()?, month(m)?, day(d)?), off),
        _ => return None,
    };
    // The offset at the reset, not now's: a daylight-saving change between
    // now and the reset moves the time by an hour.
    let t = match offset(zone, t0) {
        Some(off2) if off2 != off => at((t0 + off).div_euclid(86400), off2),
        _ => t0,
    };
    u64::try_from(t).ok()
}

/// Where this computer keeps its time-zone files; macOS keeps them under
/// `/var/db/timezone/zoneinfo`.
const ZONE_DIRS: [&str; 2] = ["/usr/share/zoneinfo", "/var/db/timezone/zoneinfo"];

/// A time zone's offset from UTC at `at` (seconds since the epoch), in
/// seconds: `date` with `TZ` set, or this computer's own zone without one.
/// `UTC` and `GMT` need no process. A zone this computer does not know has no
/// offset: `date +%z` answers `+0000` for one, as if it were UTC. A zone
/// name must be a zone file: a folder such as `America` is not one.
pub(super) fn zone_offset(zone: Option<&str>, at: i64) -> Option<i64> {
    if matches!(zone, Some("UTC" | "GMT")) {
        return Some(0);
    }
    if let Some(z) = zone {
        // A zone name is a file under the zone folders, never a path into one.
        if z.starts_with('.') || z.starts_with('/') || z.contains("..") {
            return None;
        }
        if !ZONE_DIRS.iter().any(|d| Path::new(d).join(z).is_file()) {
            return None;
        }
    }
    let at = at.max(0).to_string();
    let at_time = |flag: String| {
        let mut cmd = Command::new("date");
        cmd.arg(flag).arg("+%z");
        if let Some(z) = zone {
            cmd.env("TZ", z);
        }
        let out = String::from_utf8(cmd.output().ok()?.stdout).ok()?;
        let out = out.trim();
        // A signed `HHMM` only: GNU `date` answers one, BSD `date -d` answers
        // its format, and an answer that is neither is no offset.
        let (sign, rest) = match (out.chars().next(), out.get(1..)) {
            (Some('+'), Some(r)) => (1, r),
            (Some('-'), Some(r)) => (-1, r),
            _ => return None,
        };
        let (h, m) = (rest.get(..2)?.parse::<i64>().ok()?, rest.get(2..)?.parse::<i64>().ok()?);
        (h < 24 && m < 60).then(|| sign * (h * 3600 + m * 60))
    };
    // GNU `date -d @<t>` first, then BSD `date -r <t>`: the offset at `at`, not now.
    at_time(format!("-d@{at}")).or_else(|| at_time(format!("-r{at}")))
}

/// What a detection reads from this computer, so tests can give their own.
pub(super) struct Here<'a> {
    /// The folder holding an agent's sign-in files.
    pub folder: &'a dyn Fn(&Kind) -> PathBuf,
    /// What an agent's own status command says: signed in or not, none when it can't say.
    pub status: &'a dyn Fn(&Kind) -> Option<bool>,
    /// Whether an environment variable is set.
    pub var: &'a dyn Fn(&str) -> bool,
    /// Whether a key is stored for a model id (Settings).
    pub stored: &'a dyn Fn(&str) -> bool,
    /// Whether a program is installed.
    pub installed: &'a dyn Fn(&str) -> bool,
    /// Milliseconds since the epoch.
    pub now: u64,
}

/// Why model `m` can't start, from what is on this computer: its program, its
/// endpoint's key, its sign-in (a sign-in file that hasn't expired, on macOS
/// the keychain, or a key variable, which only some CLIs let override a
/// signed-out status).
pub(super) fn local(m: &Value, here: &Here) -> Option<String> {
    if let Some(argv) = m["run"].as_array() {
        let program = argv.first().and_then(|p| p.as_str()).unwrap_or("");
        return (!(here.installed)(program)).then(|| missing_program(m));
    }
    let agent = m["agent"].as_str().unwrap_or("");
    if !(here.installed)(agent) {
        return Some(missing_program(m));
    }
    if let Some(e) = m.get("endpoint") {
        if m["id"].as_str().is_some_and(|id| (here.stored)(id)) {
            return None;
        }
        return match e["key_env"].as_str() {
            Some(key) if (here.var)(key) => None,
            Some(key) => Some(format!("No key yet; add one in Settings or set the variable {key}")),
            // An endpoint that names no key, such as a local server, needs none.
            None if e["stored_key"] != true => None,
            None => Some("No key yet; add one in Settings".into()),
        };
    }
    let k = kind(agent)?;
    let key = k.keys.iter().any(|v| (here.var)(v));
    if key && k.keys_override_status {
        return None;
    }
    let answered = (here.status)(k);
    if key && answered.is_none() {
        return None;
    }
    match answered {
        Some(true) => return None,
        Some(false) => return Some(format!("{agent} is not signed in")),
        None => {}
    }
    let folder = (here.folder)(k);
    let files: Vec<Value> =
        k.files.iter().map(|f| folder.join(f)).filter(|p| p.exists()).map(|p| jsonx::read_opt(&p).unwrap_or(Value::Null)).collect();
    if files.iter().any(|f| !(k.expired)(f, here.now)) || k.keychain.is_some_and(in_keychain) {
        return None;
    }
    if !files.is_empty() {
        return Some(format!("{}'s sign-in has expired; sign in again", k.label));
    }
    Some(format!("{agent} is not signed in"))
}

/// Whether the macOS keychain holds item `service`. Without `-w` the secret
/// isn't read, so macOS asks the human nothing.
fn in_keychain(service: &str) -> bool {
    cfg!(target_os = "macos")
        && Command::new("security")
            .args(["find-generic-password", "-s", service])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// How long a detection or the limit file is trusted before it is read again.
const FRESH: Duration = Duration::from_secs(10);
static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Value)>>> = LazyLock::new(Default::default);

/// `work`'s value for `key`, worked out at most once per `FRESH`.
fn cached(key: &str, work: impl FnOnce() -> Value) -> Value {
    cached_for(key, FRESH, work)
}

/// `work`'s value for `key`, worked out again once it is `fresh` old.
pub(super) fn cached_for(key: &str, fresh: Duration, work: impl FnOnce() -> Value) -> Value {
    if let Some((at, v)) = CACHE.lock().unwrap().get(key)
        && at.elapsed() < fresh
    {
        return v.clone();
    }
    let v = work();
    CACHE.lock().unwrap().insert(key.to_string(), (Instant::now(), v.clone()));
    v
}

/// Drops the cached limit marks, after one changes.
pub(super) fn forget_marks() {
    CACHE.lock().unwrap().retain(|k, _| !k.starts_with("marks:"));
}

/// Drops the cached availability, after a model or its key changes.
pub(super) fn forget_local() {
    CACHE.lock().unwrap().retain(|k, _| !k.starts_with("local:"));
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn marks(path: &Path) -> Value {
    cached(&format!("marks:{}", path.display()), || jsonx::read_opt(path).filter(Value::is_object).unwrap_or_else(|| json!({})))
}

pub(super) fn save_marks(path: &Path, marks: &Value) -> Result<(), Failure> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(marks).unwrap())?;
    fs::rename(tmp, path)?;
    if path.ends_with("agent-holds.json") {
        // The old filename is no longer read.
        let _ = fs::remove_file(path.with_file_name("agent-limits.json"));
    }
    forget_marks();
    Ok(())
}

/// Marks sign-in `agent` as held by a usage limit, until `until` (seconds
/// since the epoch) or, without one, until cleared.
pub(super) fn mark(path: &Path, agent: &str, reason: &str, until: Option<u64>) -> Result<(), Failure> {
    write_mark(path, agent, reason, until, None)
}

/// Clears sign-in `agent`'s mark; says whether there was one.
pub(super) fn clear(path: &Path, agent: &str) -> Result<bool, Failure> {
    let mut all = marks(path);
    let had = all.as_object_mut().unwrap().remove(agent).is_some();
    if had {
        save_marks(path, &all)?;
    }
    Ok(had)
}

/// Learns from model `m`'s run log `log`, started at `started` and ended with
/// `outcome`, whether its sign-in expired or hit a usage limit. The agent's
/// own error saying so marks the sign-in; a usage limit holds until the reset
/// it names, and an expired sign-in until cleared. So does a message that is
/// exactly such a saying, as Claude Code reports its limit with
/// `is_error: false`. A run that ended with a successful outcome clears a
/// mark, the same list `supervise` ends a run with: a stopped, timed-out or
/// failed run leaves it, and so does a mark set after the run started. A
/// model on its own program is never marked. A model on its own endpoint is
/// marked alone, when the agent refuses it.
pub(super) fn observe(
    path: &Path,
    m: &Value,
    log: &Path,
    now: u64,
    started: u64,
    outcome: Option<&str>,
    offset: &dyn Fn(Option<&str>, i64) -> Option<i64>,
) -> Result<(), Failure> {
    let Some(k) = sign_in(m) else { return observe_refusal(path, m, log, started, outcome) };
    let entries = transcript(log);
    if model_is_gone(k, &entries) {
        mark_stale(k.name);
    }
    if said(&entries, k.signed_out, k.signed_out_whole) {
        return mark_sign_in(path, k.name, &sign_in_message(k));
    }
    match limit_hit(k, &entries) {
        Some(said) => mark_limit(path, k, said, now, offset),
        None if succeeded(outcome) && answered(k, &entries) => clear_before(path, k.name, started),
        None => Ok(()),
    }
}

/// After a run of model `m`, started at `started` and ended with `outcome`:
/// what its log says about its sign-in's usage limit or expiry.
pub(super) fn after_run(m: &Value, log: &Path, started: u64, outcome: Option<&str>) {
    if let Err(e) = observe(&holds_path(), m, log, now(), started, outcome, &zone_offset) {
        eprintln!("solace: could not record the agent's sign-in: {}", e.0);
    }
    forget_local();
}

/// Why model `m` can't start right now, and whether that is a mark a human
/// may clear. Checks only what is on this computer, cached for `FRESH`.
pub(super) fn availability(m: &Value) -> Option<Status> {
    if m["gone"] == true {
        return Some(Status {
            reason: m["reason"].as_str().unwrap_or("no longer offered by its agent").to_string(),
            retry: false,
            sign_in: false,
        });
    }
    let reason = cached(&format!("local:{m}"), || json!(local_now(m)));
    if let Some(r) = reason.as_str() {
        return Some(Status { reason: r.to_string(), retry: false, sign_in: false });
    }
    let key = hold_key(m)?;
    let reason = held(&holds_path(), &key, now())?;
    Some(Status { reason, retry: true, sign_in: sign_in_mark(&holds_path(), &key, now()) })
}

/// Why a model can't be started right now, if it can't.
pub fn unavailable(m: &Value) -> Option<String> {
    availability(m).map(|s| s.reason)
}

/// "Try again": clears the mark of the sign-in `body.agent` names, or every
/// hold that may be retried without one, and the cached availability. A
/// usage limit that has not reset remains in force.
pub(super) fn try_again(body: &Value) -> Result<(), Failure> {
    match body["agent"].as_str() {
        None => clear_retryable(&holds_path())?,
        Some(agent) if kind(agent).is_some() => {
            clear(&holds_path(), agent)?;
        }
        Some(_) => return err("E7001", "the agent must be a known kind"),
    }
    forget_local();
    Ok(())
}

/// Codex's sign-in errors: a token it can no longer refresh or use.
pub(super) fn codex_signed_out(error: &str) -> bool {
    CODEX_SIGNED_OUT.iter().any(|s| error.contains(s))
}

/// Claude Code's sign-in errors: an OAuth session it let expire.
pub(super) fn claude_signed_out(error: &str) -> bool {
    CLAUDE_SIGNED_OUT.iter().any(|s| error.contains(s))
}

/// Claude Code's update errors: it can't continue on this version.
pub(super) fn claude_update(error: &str) -> bool {
    CLAUDE_UPDATE.iter().any(|s| error.contains(s))
}

/// Whether `entries` show the agent saying `says`: in an error, or as the
/// whole of a single-line message, never as a passing mention.
pub(super) fn said(entries: &[Value], says: fn(&str) -> bool, whole: fn(&str) -> bool) -> bool {
    entries.iter().any(|e| match e["kind"].as_str() {
        Some("error") => e["text"].as_str().is_some_and(says),
        Some("message") => e["text"].as_str().is_some_and(|t| {
            let t = t.trim();
            !t.contains('\n') && whole(t)
        }),
        _ => false,
    })
}

/// What to do when `k`'s sign-in has expired.
pub(super) fn sign_in_message(k: &Kind) -> String {
    format!("{}'s sign-in has expired. Open {} once and sign in, then press Try again.", k.label, k.label)
}

/// What to do when `k` needs an update.
pub(super) fn update_message(k: &Kind) -> String {
    format!("{} needs an update. Update {}, then start the task again.", k.label, k.label)
}

/// What to do when model `m`'s program is not on this computer.
pub(super) fn missing_program(m: &Value) -> String {
    format!("{} is not installed. Install it.", program_name(m))
}

/// Why model `m`'s run stopped, in words a human can act on, when its output
/// says its sign-in expired or its program needs an update.
pub(super) fn stopped(m: &Value, entries: &[Value]) -> Option<String> {
    if !m["run"].is_null() {
        return None;
    }
    let k = kind(m["agent"].as_str()?)?;
    if sign_in(m).is_some() && said(entries, k.signed_out, k.signed_out_whole) {
        return Some(sign_in_message(k));
    }
    match k.update {
        Some((update, whole)) if said(entries, update, whole) => Some(update_message(k)),
        _ => None,
    }
}

/// Why a model can't start now: its reason, whether Try again can clear it,
/// and whether that hold is an expired sign-in.
pub(super) struct Status {
    pub(super) reason: String,
    pub(super) retry: bool,
    pub(super) sign_in: bool,
}

/// The mark holding sign-in `agent` at `now` (seconds), if any.
pub(super) fn held_mark(path: &Path, agent: &str, now: u64) -> Option<Value> {
    let m = &marks(path)[agent];
    (m.is_object() && m["until"].as_u64().is_none_or(|u| now < u)).then(|| m.clone())
}

pub(super) fn held_reason(m: &Value) -> String {
    m["reason"].as_str().unwrap_or("usage limit reached").to_string()
}

/// Whether the mark holding sign-in `agent` at `now` is an expired sign-in.
pub(super) fn sign_in_mark(path: &Path, agent: &str, now: u64) -> bool {
    held_mark(path, agent, now).is_some_and(|m| m["kind"] == "sign_in")
}

pub(super) fn write_mark(path: &Path, agent: &str, reason: &str, until: Option<u64>, kind: Option<&str>) -> Result<(), Failure> {
    let mut all = marks(path);
    let mut m = json!({"reason":reason,"until":until,"time":now()});
    if let Some(kind) = kind {
        m["kind"] = json!(kind);
    }
    all[agent] = m;
    save_marks(path, &all)
}

/// Marks sign-in `agent` as held by an expired sign-in, until cleared.
pub(super) fn mark_sign_in(path: &Path, agent: &str, reason: &str) -> Result<(), Failure> {
    write_mark(path, agent, reason, None, Some("sign_in"))
}

pub(super) fn model_is_gone(k: &Kind, entries: &[Value]) -> bool {
    entries.iter().filter(|e| e["kind"] == "error").any(|e| (k.gone)(e["text"].as_str().unwrap_or("")))
}

/// A limit said as the whole of an agent's message, not a passing mention
/// inside one: Claude Code writes its limit as a message, not an error.
pub(super) fn limit_message(k: &Kind, e: &Value) -> bool {
    e["kind"] == "message"
        && e["text"].as_str().is_some_and(|t| {
            let t = t.trim();
            !t.contains('\n') && (k.limit)(t).is_some()
        })
}

pub(super) fn limit_hit(k: &Kind, entries: &[Value]) -> Option<Option<String>> {
    entries
        .iter()
        .filter(|e| e["kind"] == "error")
        .find_map(|e| (k.limit)(e["text"].as_str().unwrap_or("")))
        .or_else(|| entries.iter().filter(|e| limit_message(k, e)).find_map(|e| (k.limit)(e["text"].as_str().unwrap_or(""))))
}

pub(super) fn answered(k: &Kind, entries: &[Value]) -> bool {
    entries.iter().any(|e| matches!(e["kind"].as_str(), Some("message" | "thinking" | "tool")) && !limit_message(k, e))
}

pub(super) fn succeeded(outcome: Option<&str>) -> bool {
    outcome.is_some_and(|o| matches!(o, "finished" | "reported" | "reviewed" | "discussed" | "planned"))
}

pub(super) fn mark_limit(
    path: &Path,
    k: &Kind,
    said: Option<String>,
    now: u64,
    offset: &dyn Fn(Option<&str>, i64) -> Option<i64>,
) -> Result<(), Failure> {
    let until = said.as_deref().and_then(|s| reset_time(s, now, offset));
    let reason = match &said {
        Some(s) => format!("{} usage limit reached (resets {s})", k.label),
        None => format!("{} usage limit reached", k.label),
    };
    mark(path, k.name, &reason, until)
}

/// The run succeeded: its agent's sign-in works and its endpoint model ran, so
/// a hold set before it started is stale.
pub(super) fn clear_before(path: &Path, agent: &str, started: u64) -> Result<(), Failure> {
    let m = &marks(path)[agent];
    if m.is_object() && m["time"].as_u64().is_none_or(|t| t <= started) { clear(path, agent).map(drop) } else { Ok(()) }
}

/// Where agents' marks are kept, a usage limit or an expired sign-in: beside
/// the system settings, the same for every project.
pub(super) fn holds_path() -> PathBuf {
    config_path().with_file_name("agent-holds.json")
}

/// Why sign-in `agent` is held at `now` (seconds), if it is.
pub(super) fn held(path: &Path, agent: &str, now: u64) -> Option<String> {
    held_mark(path, agent, now).map(|m| held_reason(&m))
}

/// Whether a message is exactly Codex saying its sign-in expired.
pub(super) fn codex_signed_out_whole(error: &str) -> bool {
    whole_in(error, &CODEX_SIGNED_OUT)
}

/// Whether a message is exactly Claude Code saying its sign-in expired.
pub(super) fn claude_signed_out_whole(error: &str) -> bool {
    whole_in(error, &CLAUDE_SIGNED_OUT)
}

/// Whether a message is exactly Claude Code saying it needs an update.
pub(super) fn claude_update_whole(error: &str) -> bool {
    whole_in(error, &CLAUDE_UPDATE)
}

pub(super) const CODEX_SIGNED_OUT: [&str; 2] = ["Your access token could not be refreshed", "Please log out and sign in again"];

pub(super) const CLAUDE_SIGNED_OUT: [&str; 3] =
    ["Failed to authenticate: OAuth session expired and could not be refreshed", "OAuth token has expired", "Please run /login"];

pub(super) const CLAUDE_UPDATE: [&str; 2] = ["needs an update", "is required to continue"];

/// Whether `text`, trimmed, is one of `markers`.
pub(super) fn whole_in(text: &str, markers: &[&str]) -> bool {
    markers.contains(&text.trim())
}

/// Model `m`'s program, as its command or kind names it.
pub(super) fn program_name(m: &Value) -> &str {
    m["run"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|p| p.as_str())
        .or_else(|| kind(m["agent"].as_str().unwrap_or("")).map(|k| k.label))
        .or_else(|| m["agent"].as_str())
        .unwrap_or("The program")
}

/// What to do when model `m`'s program is not on this computer and a task is starting.
pub(super) fn missing_program_at_launch(m: &Value) -> String {
    format!("{} is not installed. Install it, then start the task again.", program_name(m))
}

/// Whether an agent can start now: one is signed in, none is installed, or one
/// is installed but needs its sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentCheck {
    Ready,
    Missing,
    SignedOut { name: &'static str, label: &'static str },
}

/// Why model `m` can't start from what is on this computer, checked now.
pub(super) fn local_now(m: &Value) -> Option<String> {
    let keys = setup::read_keys(&setup::keys_path());
    let here = Here {
        folder: &|k| (k.folder)(),
        status: &|k| cached(&format!("local:status:{}", k.name), || json!(program_status(k))).as_bool(),
        var: &|v| nonempty_key(env::var(v).ok()),
        stored: &|id| keys[id].is_string(),
        installed: &on_path,
        now: now_ms(),
    };
    local(m, &here)
}

/// Whether an agent is installed and signed in, checked now instead of from the cache.
pub(super) fn check_now() -> AgentCheck {
    let installed: Vec<&Kind> = KINDS.iter().filter(|k| on_path(k.name)).collect();
    if installed.is_empty() {
        return AgentCheck::Missing;
    }
    if installed.iter().any(|k| local_now(&json!({"agent": k.name})).is_none()) {
        return AgentCheck::Ready;
    }
    let k = installed[0];
    AgentCheck::SignedOut { name: k.name, label: k.label }
}

/// Codex's folder: `CODEX_HOME`, else `.codex` in the home folder.
pub(super) fn codex_folder() -> PathBuf {
    codex_folder_from(env::var_os("CODEX_HOME"), env::var_os("HOME"))
}

/// Whether an environment variable's value is a key: set and not blank.
pub(super) fn nonempty_key(value: Option<String>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// How long an agent's status command may take.
const STATUS_DEADLINE: Duration = Duration::from_secs(5);

/// Whether agent `k` says it is signed in, asked of its status command. None
/// when it has none, can't run it, takes longer than `STATUS_DEADLINE`, or
/// answers unclearly.
pub(super) fn program_status(k: &Kind) -> Option<bool> {
    program_status_within(k, STATUS_DEADLINE)
}

/// What a status command's output says: JSON with `loggedIn`, or words
/// saying the agent is or is not logged in. None when it says neither.
pub(super) fn status_answer(success: bool, said: &str) -> Option<bool> {
    if let Some(logged_in) = serde_json::from_str::<Value>(said.trim()).ok().and_then(|v| v["loggedIn"].as_bool()) {
        return Some(logged_in);
    }
    let said = said.to_ascii_lowercase();
    if ["not logged in", "not signed in"].iter().any(|w| said.contains(w)) {
        return Some(false);
    }
    (success && ["logged in", "signed in"].iter().any(|w| said.contains(w))).then_some(true)
}

/// Learns from a run of a model on its own endpoint: the agent refusing the
/// model holds that model alone; a run that succeeded clears it.
pub(super) fn observe_refusal(path: &Path, m: &Value, log: &Path, started: u64, outcome: Option<&str>) -> Result<(), Failure> {
    let (Some(key), Some(k)) = (hold_key(m), kind(m["agent"].as_str().unwrap_or(""))) else { return Ok(()) };
    if succeeded(outcome) {
        return clear_before(path, &key, started);
    }
    match refusal(k, &transcript(log)) {
        Some(reason) => write_mark(path, &key, &reason, None, Some("refusal")),
        None => Ok(()),
    }
}

/// The first line of the error where agent `k` refuses the model, or says its sign-in expired.
pub(super) fn refusal(k: &Kind, entries: &[Value]) -> Option<String> {
    entries
        .iter()
        .filter(|e| e["kind"] == "error")
        .filter_map(|e| e["text"].as_str())
        .find(|t| (k.gone)(t) || (k.signed_out)(t))
        .map(|t| t.lines().next().unwrap_or(t).trim().chars().take(240).collect())
}

/// The key of the mark that holds model `m` back: its agent's sign-in, or,
/// for a model on its own endpoint, that model alone.
pub(super) fn hold_key(m: &Value) -> Option<String> {
    if let Some(k) = sign_in(m) {
        return Some(k.name.to_string());
    }
    kind(m["agent"].as_str()?)?;
    (m.get("endpoint").is_some() && m["run"].is_null()).then(|| match m["id"].as_str() {
        Some(id) if !id.is_empty() => format!("model:{id}"),
        _ => format!("model:{}:{}", m["agent"].as_str().unwrap_or(""), m["model"].as_str().unwrap_or("")),
    })
}

/// Reads `pipe` while its program runs, so an answer larger than the pipe
/// buffer can't stall the program.
pub(super) fn read_pipe(mut pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut said = String::new();
        let _ = pipe.read_to_string(&mut said);
        said
    })
}

/// `program_status` with a deadline a test can shorten.
pub(super) fn program_status_within(k: &Kind, deadline: Duration) -> Option<bool> {
    if k.status.is_empty() {
        return None;
    }
    let mut child = Command::new(k.name).args(k.status).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let out = child.stdout.take().map(read_pipe);
    let err = child.stderr.take().map(read_pipe);
    let started = Instant::now();
    let exit = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break exit,
            Ok(None) if started.elapsed() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut said = String::new();
    if let Some(part) = out.and_then(|handle| handle.join().ok()) {
        said.push_str(&part);
    }
    if let Some(part) = err.and_then(|handle| handle.join().ok()) {
        said.push_str(&part);
    }
    status_answer(exit.success(), &said)
}

pub(super) fn codex_folder_from(codex_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> PathBuf {
    codex_home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(home.unwrap_or_default()).join(".codex"))
}

/// Clears every hold Try again may clear, keeping a usage limit until its reset.
pub(super) fn clear_retryable(path: &Path) -> Result<(), Failure> {
    let now = now();
    let mut all = marks(path);
    all.as_object_mut()
        .unwrap()
        .retain(|_, m| !matches!(m["kind"].as_str(), Some("sign_in" | "refusal")) && m["until"].as_u64().is_some_and(|until| now < until));
    save_marks(path, &all)
}

/// The label a run's `used` model had when the run ended: a current entry
/// before a gone one, the later entry among equals, else the model id.
pub(super) fn used_label(models: &[Value], used: &str) -> String {
    models
        .iter()
        .filter(|m| m["gone"] == true)
        .chain(models.iter().filter(|m| m["gone"] != true))
        .filter(|m| m["model"] == used)
        .filter_map(|m| m["label"].as_str())
        .last()
        .unwrap_or(used)
        .to_string()
}
