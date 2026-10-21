//! The agents Solace runs, and whether each can start right now. Each agent
//! kind is one entry in `KINDS`: its program, where its sign-in is kept, and
//! how it says a usage limit was reached, so another harness is one more entry.
//! The checks are local and make no model call. A usage limit is learned from a
//! run that hit it and kept in `agent-limits.json`, beside the system settings,
//! so it outlasts a restart.
use super::*;
use std::sync::{LazyLock, Mutex};

/// One agent kind.
pub(super) struct Kind {
    /// The `agent` a model names, which is also its program.
    pub name: &'static str,
    /// What the human calls it.
    pub label: &'static str,
    /// Sign-in files, from the home folder.
    pub files: &'static [&'static str],
    /// Variables holding an API key; any one set is a sign-in.
    pub keys: &'static [&'static str],
    /// The macOS keychain item that may hold the sign-in instead of a file.
    pub keychain: Option<&'static str>,
    /// Whether a sign-in file says its credentials have expired, at `now` (ms).
    pub expired: fn(&Value, u64) -> bool,
    /// Whether an error is this agent saying a usage limit was reached, and
    /// then the reset time it names, if any.
    pub limit: fn(&str) -> Option<Option<String>>,
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
        files: &[".codex/auth.json"],
        keys: &["OPENAI_API_KEY"],
        keychain: None,
        expired: codex_expired,
        limit: codex_limit,
        discover: codex_models,
        gone: model_gone,
    },
    Kind {
        name: "claude",
        label: "Claude Code",
        files: &[".claude/.credentials.json"],
        keys: &["ANTHROPIC_API_KEY"],
        keychain: Some("Claude Code-credentials"),
        expired: claude_expired,
        limit: claude_limit,
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
    cached(&format!("models:{}", path.display()), || {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
    })
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
/// model it used, an alias's label is that model's: the label of the entry
/// running it, else its id.
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
    let labels: Vec<(Value, Value)> = out.iter().map(|m| (m["model"].clone(), m["label"].clone())).collect();
    for m in &mut out {
        m["effort"] = json!(default_effort(m));
        if let Some(used) = all["used"][m["id"].as_str().unwrap_or("")].as_str() {
            if names_family(m) {
                m["label"] = labels.iter().rev().find(|(model, _)| model == used).map_or(json!(used), |(_, l)| l.clone());
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

/// Says model `model` resolved to `used`: an alias's label names the model a run used, and a
/// model with its own `model` keeps showing it.
pub(super) fn record_used(model: &str, used: &str) {
    let _writer = MODELS.lock().unwrap();
    let path = models_path();
    let mut all = saved_models(&path);
    if all["used"][model].as_str() != Some(used) {
        all["used"][model] = json!(used);
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
    e.contains("model")
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
    pub home: &'a Path,
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
/// endpoint's key, its sign-in (a key variable, a sign-in file that hasn't
/// expired, or on macOS the keychain).
pub(super) fn local(m: &Value, here: &Here) -> Option<String> {
    if let Some(argv) = m["run"].as_array() {
        let program = argv[0].as_str().unwrap_or("");
        return (!(here.installed)(program)).then(|| format!("{program} is not installed"));
    }
    let agent = m["agent"].as_str().unwrap_or("");
    if !(here.installed)(agent) {
        return Some(format!("{agent} is not installed (not on PATH)"));
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
    if k.keys.iter().any(|v| (here.var)(v)) {
        return None;
    }
    let files: Vec<Value> = k
        .files
        .iter()
        .map(|f| here.home.join(f))
        .filter(|p| p.exists())
        .map(|p| fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null))
        .collect();
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

/// Where usage-limit marks are kept: beside the system settings, the same for every project.
pub(super) fn limits_path() -> PathBuf {
    config_path().with_file_name("agent-limits.json")
}

fn marks(path: &Path) -> Value {
    cached(&format!("marks:{}", path.display()), || {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
    })
}

fn save_marks(path: &Path, marks: &Value) -> Result<(), Failure> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(marks).unwrap())?;
    fs::rename(tmp, path)?;
    forget_marks();
    Ok(())
}

/// Marks sign-in `agent` as held by a usage limit, until `until` (seconds
/// since the epoch) or, without one, until cleared.
pub(super) fn mark(path: &Path, agent: &str, reason: &str, until: Option<u64>) -> Result<(), Failure> {
    let mut all = marks(path);
    all[agent] = json!({"reason":reason,"until":until,"time":now()});
    save_marks(path, &all)
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

/// Why sign-in `agent` is held by a usage limit at `now` (seconds), if it is.
pub(super) fn limited(path: &Path, agent: &str, now: u64) -> Option<String> {
    let m = &marks(path)[agent];
    (m.is_object() && m["until"].as_u64().is_none_or(|u| now < u))
        .then(|| m["reason"].as_str().unwrap_or("usage limit reached").to_string())
}

/// Learns from model `m`'s run log `log`, started at `started` and ended with
/// `outcome`, whether its sign-in hit a usage limit. The agent's own error
/// saying so marks the sign-in until the reset it names; so does a message
/// that is exactly such a saying, as Claude Code reports its limit with
/// `is_error: false`. A run that ended with a successful outcome clears a
/// mark, the same list `supervise` ends a run with: a stopped, timed-out or
/// failed run leaves it, and so does a mark set after the run started. A
/// model on its own endpoint or program is never marked.
pub(super) fn observe(
    path: &Path,
    m: &Value,
    log: &Path,
    now: u64,
    started: u64,
    outcome: Option<&str>,
    offset: &dyn Fn(Option<&str>, i64) -> Option<i64>,
) -> Result<(), Failure> {
    let Some(k) = sign_in(m) else { return Ok(()) };
    let entries = transcript(log);
    if entries.iter().filter(|e| e["kind"] == "error").any(|e| (k.gone)(e["text"].as_str().unwrap_or(""))) {
        mark_stale(k.name);
    }
    // A limit said as the whole of an agent's message, not a passing mention
    // inside one: Claude Code writes its limit as a message, not an error.
    let in_text = |e: &Value| {
        e["kind"] == "message"
            && e["text"].as_str().is_some_and(|t| {
                let t = t.trim();
                !t.contains('\n') && (k.limit)(t).is_some()
            })
    };
    let hit = entries
        .iter()
        .filter(|e| e["kind"] == "error")
        .find_map(|e| (k.limit)(e["text"].as_str().unwrap_or("")))
        .or_else(|| entries.iter().filter(|e| in_text(e)).find_map(|e| (k.limit)(e["text"].as_str().unwrap_or(""))));
    match hit {
        Some(said) => {
            let until = said.as_deref().and_then(|s| reset_time(s, now, offset));
            let reason = match &said {
                Some(s) => format!("{} usage limit reached (resets {s})", k.label),
                None => format!("{} usage limit reached", k.label),
            };
            mark(path, k.name, &reason, until)
        }
        // The agent answered: its sign-in works again. Only a run that ended
        // with a successful outcome says that, and only a mark set before it
        // started is this run's.
        None if outcome.is_some_and(|o| matches!(o, "finished" | "reported" | "reviewed" | "discussed" | "planned"))
            && entries.iter().any(|e| matches!(e["kind"].as_str(), Some("message" | "thinking" | "tool")) && !in_text(e)) =>
        {
            let m = &marks(path)[k.name];
            if m.is_object() && m["time"].as_u64().is_none_or(|t| t <= started) { clear(path, k.name).map(drop) } else { Ok(()) }
        }
        None => Ok(()),
    }
}

/// After a run of model `m`, started at `started` and ended with `outcome`:
/// what its log says about its sign-in's limit.
pub(super) fn after_run(m: &Value, log: &Path, started: u64, outcome: Option<&str>) {
    if let Err(e) = observe(&limits_path(), m, log, now(), started, outcome, &zone_offset) {
        eprintln!("solace: could not record the agent's usage limit: {}", e.0);
    }
}

/// Why model `m` can't start right now, and whether that is a usage limit a
/// human may clear. Checks only what is on this computer, cached for `FRESH`.
pub(super) fn availability(m: &Value) -> Option<(String, bool)> {
    if m["gone"] == true {
        return Some((m["reason"].as_str().unwrap_or("no longer offered by its agent").to_string(), false));
    }
    let reason = cached(&format!("local:{m}"), || {
        let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
        let keys = setup::read_keys(&setup::keys_path());
        let here =
            Here { home: &home, var: &|v| env::var(v).is_ok(), stored: &|id| keys[id].is_string(), installed: &on_path, now: now_ms() };
        json!(local(m, &here))
    });
    if let Some(r) = reason.as_str() {
        return Some((r.to_string(), false));
    }
    limited(&limits_path(), sign_in(m)?.name, now()).map(|r| (r, true))
}

/// Why a model can't be started right now, if it can't.
pub fn unavailable(m: &Value) -> Option<String> {
    availability(m).map(|(r, _)| r)
}

/// "Try again": clears the usage-limit mark of the sign-in `body.agent` names.
pub(super) fn try_again(body: &Value) -> Result<(), Failure> {
    let Some(agent) = body["agent"].as_str().filter(|a| kind(a).is_some()) else { return err("E7001", "an agent kind is required") };
    clear(&limits_path(), agent)?;
    Ok(())
}
