use super::*;
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

pub fn lock(root: &Path) -> Result<fs::File, Failure> {
    fs::create_dir_all(root.join(".solace"))?;
    // ponytail: one project lock; use per-change store isolation if parallel work is introduced.
    lock_file(&root.join(".solace/lock"))
}
/// The project lock when it is free right now; the board's poll skips a busy project.
fn try_lock(root: &Path) -> Option<fs::File> {
    try_lock_file(&root.join(".solace/lock")).ok()?
}
pub fn open(root: &Path) -> Result<Store, Failure> {
    let s = Store::at(root)?;
    fs::create_dir_all(root.join(".solace/board"))?;
    Ok(s)
}
pub(super) fn err<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "task", message)
}
fn task_path(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return err("E7001", "invalid task id");
    }
    Ok(root.join(".solace/board").join(format!("{id}.json")))
}
pub(super) fn task(root: &Path, id: &str) -> Result<Value, Failure> {
    let path = task_path(root, id)?;
    if !path.exists() {
        return err("E7001", "unknown task");
    }
    read_task(root, &path)
}
pub(super) fn tasks(root: &Path) -> Result<Vec<Value>, Failure> {
    let mut out = vec![];
    for entry in fs::read_dir(root.join(".solace/board"))? {
        let p = entry?.path();
        if p.extension().is_some_and(|x| x == "json") {
            out.push(read_task(root, &p)?);
        }
    }
    out.sort_by(|a: &Value, b: &Value| a["rank"].as_f64().unwrap_or(0.).total_cmp(&b["rank"].as_f64().unwrap_or(0.)));
    Ok(out)
}
fn read_task(root: &Path, path: &Path) -> Result<Value, Failure> {
    let mut t: Value = serde_json::from_slice(&fs::read(path)?).map_err(|e| Failure(e.to_string(), 1))?;
    // The default workflow's `done` column is now `complete`; tasks saved before still say `done`.
    if t["column"] == "done" && !flow(root)?["columns"].as_array().unwrap().iter().any(|c| c["id"] == "done") {
        t["column"] = json!("complete");
    }
    Ok(t)
}
fn event(s: &Store, v: Value) -> Result<(), Failure> {
    s.db.execute("INSERT INTO board_events(data) VALUES (?)", [v.to_string()])?;
    Ok(())
}
pub(super) fn save(s: &Store, t: &Value) -> Result<(), Failure> {
    write_atomic(&task_path(&s.root, t["id"].as_str().unwrap())?, serde_json::to_vec_pretty(t).unwrap())?;
    event(s, json!({"task":t}))
}
/// A project's workflow: its root's, shared by every project on the board (D-61).
pub fn flow(project: &Path) -> Result<Value, Failure> {
    workflow(&lockdown::settings(&scope::top(project))?)
}
pub fn workflow(settings: &toml::Value) -> Result<Value, Failure> {
    let default: toml::Value = include_str!("default-board.toml").parse().unwrap();
    let b = settings.get("board").unwrap_or(&default["board"]);
    let mut w = serde_json::to_value(b).map_err(|e| Failure(e.to_string(), 1))?;
    let invalid = |m: &str| Failure(format!("E6020 invalid board workflow\n  {m}"), 1);
    let cols = w["columns"].as_array().ok_or_else(|| invalid("columns must be an array"))?;
    if cols.is_empty() {
        return Err(invalid("empty columns"));
    }
    let mut ids = HashSet::new();
    let mut allowed = HashSet::new();
    let actions = w["actions"].as_object().ok_or_else(|| invalid("actions must be a table"))?;
    for c in cols {
        if c.get("actions").is_some_and(|v| !v.is_array())
            || c.get("final").is_some_and(|v| !v.is_boolean())
            || c.get("label").is_some_and(|v| !v.is_string())
            || c.get("requires").is_some_and(|v| v.as_array().is_none_or(|a| a.iter().any(|x| !x.is_string())))
        {
            return Err(invalid("invalid column fields"));
        }
        let id = c["id"].as_str().ok_or_else(|| invalid("column id missing"))?;
        if !ids.insert(id.to_string()) {
            return Err(invalid("duplicate column"));
        }
        let a = c.get("actions").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        if c["final"] == true && !a.is_empty() {
            return Err(invalid("final column has actions"));
        }
        for name in a {
            let n = name.as_str().ok_or_else(|| invalid("action name"))?;
            if !actions.contains_key(n) {
                return Err(invalid("unknown action"));
            }
            allowed.insert(n.to_string());
        }
    }
    for c in cols {
        if let Some(next) = c.get("next")
            && !ids.contains(next.as_str().unwrap_or(""))
        {
            return Err(invalid("unknown gate next"));
        }
    }
    for (name, a) in actions {
        if !a.is_object()
            || ["stamp", "unstamp"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_string()))
            || ["model", "revert"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_boolean()))
        {
            return Err(invalid("invalid action fields"));
        }
        if !allowed.contains(name) {
            return Err(invalid("action never allowed"));
        }
        if a.get("to").is_some_and(|x| !ids.contains(x.as_str().unwrap_or(""))) {
            return Err(invalid("unknown action column"));
        }
        if a.get("run").is_some_and(|x| !matches!(x.as_str(), Some("investigate" | "work" | "new_work" | "review"))) {
            return Err(invalid("unknown run step"));
        }
    }
    for e in ["asked", "finished", "failed"] {
        if !ids.contains(w["events"][e].as_str().unwrap_or("")) {
            return Err(invalid("event has no target"));
        }
    }
    for c in w["columns"].as_array_mut().unwrap() {
        if c.get("label").is_none() {
            c["label"] = c["id"].clone();
        }
        if c.get("actions").is_none() {
            c["actions"] = json!([]);
        }
        if c.get("final").is_none() {
            c["final"] = json!(false);
        }
    }
    Ok(w)
}
pub(super) fn column<'a>(w: &'a Value, t: &Value) -> &'a Value {
    w["columns"].as_array().unwrap().iter().find(|c| c["id"] == t["column"]).unwrap_or(&Value::Null)
}
fn allowed(w: &Value, t: &Value) -> Vec<String> {
    let c = column(w, t);
    if c["final"] == true {
        return vec![];
    }
    let mut out = c["actions"].as_array().unwrap_or(&vec![]).iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>();
    out.extend(["pause", "resume", "discuss", "stop", "end_discussion"].map(String::from));
    out
}
/// The task's discussion run that hasn't ended, if one is going.
fn discussion(t: &Value) -> Option<String> {
    t["runs"].as_array().unwrap().iter().find(|r| r["kind"] == "discuss" && r["state"] != "ended").and_then(|r| r["id"].as_str()).map(String::from)
}
/// Replaces a discussion's notes with one verbatim transcript, each turn
/// labeled with who said it.
fn compact(t: &mut Value, rid: &str) {
    let model = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).and_then(|r| r["model"].as_str()).unwrap_or("").to_string();
    let notes = t["notes"].as_array_mut().unwrap();
    let (turns, rest): (Vec<Value>, Vec<Value>) = notes.drain(..).partition(|n| n["run"] == rid);
    *notes = rest;
    if turns.is_empty() {
        return;
    }
    let lines: Vec<String> = turns
        .iter()
        .map(|n| {
            let who = if n["author"] == "human" { "Human".to_string() } else { format!("Agent ({model})") };
            // The label stands on its own line, so a turn that opens with a heading or list still renders.
            format!("**{who}:**\n\n{}", n["text"].as_str().unwrap_or("").trim())
        })
        .collect();
    note(t, "solace", Some(rid), &format!("### Discussion\n\n{}", lines.join("\n\n")));
}
pub(super) fn active(t: &Value) -> bool {
    t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended")
}
pub(super) fn note(t: &mut Value, author: &str, run: Option<&str>, text: &str) {
    t["notes"].as_array_mut().unwrap().push(json!({"author":author,"run":run,"time":now(),"text":text}));
}
/// Marks a task as needing a human. The next action a human takes on it clears the mark.
pub(super) fn flag(t: &mut Value, reason: &str) {
    t["attention"] = json!({"reason":reason,"time":now()});
    t["finished"] = Value::Null;
}
fn move_to(w: &Value, t: &mut Value, to: &Value) -> Result<(), Failure> {
    let c = column(w, t);
    let requires = c["requires"].as_array().cloned().unwrap_or_default();
    if &c["next"] == to {
        if requires.iter().any(|r| t["stamps"].get(r.as_str().unwrap_or("")).is_none()) {
            return err("E7006", "required human stamp missing");
        }
    } else if &t["column"] != to {
        for r in requires {
            t["stamps"].as_object_mut().unwrap().remove(r.as_str().unwrap_or(""));
        }
    }
    t["column"] = to.clone();
    Ok(())
}
/// Where run `rid` is in the task's runs.
fn run_at(t: &Value, rid: &str) -> Result<usize, Failure> {
    t["runs"].as_array().and_then(|runs| runs.iter().position(|r| r["id"] == rid)).ok_or_else(|| Failure("E7003 unknown run".into(), 1))
}
fn end_run(r: &mut Value, outcome: &str) {
    r["state"] = json!("ended");
    r["ended"] = json!(now());
    r["outcome"] = json!(outcome);
}
fn end_runs(t: &mut Value, outcome: &str) {
    for r in t["runs"].as_array_mut().unwrap() {
        if r["state"] != "ended" {
            end_run(r, outcome);
        }
    }
    t["paused_from"] = Value::Null;
}
pub fn system() -> Result<Value, Failure> {
    let p = env::var_os("SOLACE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config/solace/config.toml"));
    parse_system(&if p.exists() { fs::read_to_string(p)? } else { String::new() })
}
fn parse_system(text: &str) -> Result<Value, Failure> {
    let v: toml::Value = text.parse().map_err(|e| Failure(format!("E6020 system settings: {e}"), 1))?;
    let mut v = serde_json::to_value(v).map_err(|e| Failure(e.to_string(), 1))?;
    let invalid = || {
        Failure(
            "E6020 invalid system settings: expected positive task_timeout and [[models]] with id, label, agent and model or run argv; \
             an endpoint needs kind \"openai\" (agent codex) or \"anthropic\" (agent claude) and base_url"
                .into(),
            1,
        )
    };
    // Built-in presets first; configured models with the same id replace them.
    let mut models: Vec<Value> = if v["presets"] == false {
        vec![]
    } else {
        let presets: toml::Value = include_str!("default-models.toml").parse().unwrap();
        serde_json::to_value(presets).unwrap()["models"].as_array().unwrap().clone()
    };
    for m in v.get("models").map(|m| m.as_array().cloned().ok_or_else(invalid)).transpose()?.unwrap_or_default() {
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
                if !kind_matches || !e["base_url"].is_string() || e.get("key_env").is_some_and(|k| !k.is_string()) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(v)
}

/// Codex settings for a model's OpenAI-compatible endpoint, if it has one. The
/// key stays in the variable the endpoint names; Codex reads it at launch.
/// Current Codex speaks only the Responses API, so the endpoint must serve it.
fn endpoint_args(m: &Value) -> Vec<String> {
    let Some(e) = m.get("endpoint") else {
        return vec![];
    };
    let mut provider = json!({"name": m["label"], "base_url": e["base_url"], "wire_api": "responses"});
    if let Some(k) = e["key_env"].as_str() {
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

/// The context window an endpoint reports for its model: `GET /v1/models`,
/// `meta.n_ctx` (llama.cpp, Strata). Claude Code doesn't know a local model's
/// window, so without it, it compacts too late and the server refuses the prompt.
fn endpoint_context(base_url: &str, model: &str) -> Option<u64> {
    // ponytail: plain http only; an https endpoint goes undetected.
    let rest = base_url.strip_prefix("http://")?;
    let (host, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    let path = [prefix.trim_matches('/'), "v1/models"].iter().filter(|p| !p.is_empty()).fold(String::new(), |a, p| a + "/" + p);
    let addr = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&addr).ok()?.next()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    let body: Value = serde_json::from_str(reply.split_once("\r\n\r\n")?.1).ok()?;
    let models = body["data"].as_array()?;
    // A server with one model often serves it under any name.
    let m = models.iter().find(|x| x["id"] == model).or(if models.len() == 1 { models.first() } else { None })?;
    m["meta"]["n_ctx"].as_u64()
}

fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Why a model can't be started right now, if it can't. Checks only what is
/// local (the program, its sign-in, the endpoint's key variable), so it costs
/// nothing; an endpoint that is down shows up as a failed run.
pub fn unavailable(m: &Value) -> Option<String> {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    if let Some(argv) = m["run"].as_array() {
        let program = argv[0].as_str().unwrap_or("");
        return (!on_path(program)).then(|| format!("{program} is not installed"));
    }
    let agent = m["agent"].as_str().unwrap_or("");
    if !on_path(agent) {
        return Some(format!("{agent} is not installed (not on PATH)"));
    }
    if let Some(e) = m.get("endpoint") {
        let key = e["key_env"].as_str()?;
        return env::var(key).is_err().then(|| format!("the endpoint's key variable {key} is not set"));
    }
    let signed_in = match agent {
        "codex" => home.join(".codex/auth.json").exists() || env::var("OPENAI_API_KEY").is_ok(),
        // Claude Code keeps its sign-in in the macOS keychain, which isn't checked.
        _ => cfg!(target_os = "macos") || home.join(".claude/.credentials.json").exists() || env::var("ANTHROPIC_API_KEY").is_ok(),
    };
    (!signed_in).then(|| format!("{agent} is not signed in"))
}
fn model(config: &Value, name: &str) -> Result<Value, Failure> {
    config["models"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["id"] == name))
        .cloned()
        .ok_or_else(|| Failure(format!("E7001 unknown model {name}; configure system [[models]]"), 1))
}
/// Efforts an action may choose for a model. Custom runners take none.
fn efforts(m: &Value) -> &'static [&'static str] {
    match (m["run"].is_null(), m["agent"].as_str()) {
        (true, Some("codex")) => &["low", "medium", "high", "xhigh"],
        (true, Some("claude")) => &["low", "medium", "high", "xhigh", "max"],
        _ => &[],
    }
}

// Snapshot live rows only. Before-state rows never enter the semantic index.
pub fn snapshot(s: &Store) -> Result<Value, Failure> {
    let mut out = json!({});
    let mut st = s.db.prepare("SELECT id,module,name,kind,text,ord,revision,live FROM symbols WHERE live=1")?;
    for row in st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, i64>(7)?
        ]))
    })? {
        let row = row?;
        let key = format!("symbol:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    let mut st = s.db.prepare("SELECT path,dir,header FROM modules")?;
    for row in st.query_map([], |r| Ok(json!([r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?])))? {
        let row = row?;
        let key = format!("module:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    Ok(out)
}
/// The stores of a task's projects (D-61), with their folders; `s` is its home's.
pub fn scope_stores(s: &Store, t: &Value) -> Result<Vec<(String, Store)>, Failure> {
    let top = scope::top(&s.root);
    let members = workspace::members(&top)?;
    scope::of(t, &scope::dir(&top, &s.root))
        .into_iter()
        .filter(|d| members.contains(d))
        .map(|d| Ok((d.clone(), open(&scope::path(&top, &d))?)))
        .collect()
}
/// A change entry's project; entries saved before D-61 are the home's.
/// A new task in `column`, with nothing done on it yet.
pub fn new_task(title: &str, body: &str, column: &Value, rank: f64) -> Value {
    json!({"id":unique_id(),"title":title,"body":body,"links":[],"column":column,"rank":rank,"paused_from":null,"attention":null,
        "stamps":{},"model":null,"notes":[],"questions":[],"runs":[],"changes":[]})
}
/// The project a change was made in; changes saved before D-61 are all in the task's home.
pub fn project_of<'a>(c: &'a Value, home: &'a str) -> &'a str {
    c["project"].as_str().unwrap_or(home)
}
/// Records what a command changed in `s` on the task stored in `home`.
pub fn record(s: &Store, home: &Store, tid: &str, rid: &str, before: &Value, after: &Value) -> Result<(), Failure> {
    let mut t = task(&home.root, tid)?;
    let top = scope::top(&s.root);
    let (here, home_dir) = (scope::dir(&top, &s.root), scope::dir(&top, &home.root));
    let keys: HashSet<_> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    for key in keys {
        if before[key] == after[key] {
            continue;
        }
        let data = &before[key];
        if let Some(name) = key.strip_prefix("file:") {
            let exists: bool =
                s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1 AND entity=?2)", params![tid, key], |r| r.get(0))?;
            if !exists {
                fs::create_dir_all(before_dir(&s.root, tid))?;
                let file = before_file(&s.root, tid, name);
                if let Some(bytes) = data.as_array() {
                    fs::write(file, json_bytes(bytes))?;
                } else {
                    fs::write(file.with_extension("absent"), b"")?;
                }
            }
        }
        s.db.execute("INSERT OR IGNORE INTO board_before VALUES (?1,?2,?3)", params![tid, key, data.to_string()])?;
        s.db.execute("INSERT INTO board_touches(task,entity) VALUES (?1,?2)", params![tid, key])?;
        let name = if key.starts_with("symbol:") {
            let row = if after[key].is_null() { data } else { &after[key] };
            format!("{}.{}", row[1].as_str().unwrap_or(""), row[2].as_str().unwrap_or(""))
        } else {
            key.clone()
        };
        if !t["changes"].as_array().unwrap().iter().any(|x| x["entity"] == *key && project_of(x, &home_dir) == here) {
            t["changes"].as_array_mut().unwrap().push(json!({"entity":key,"name":name,"run":rid,"project":here}));
        }
    }
    save(home, &t)
}
/// Whether a recorded change is still worth reviewing: a file that is unwatched now (ignored after
/// the change was recorded, say) is not. Cancel and undo still restore it.
pub fn reviewable(root: &Path, key: &str) -> bool {
    let Some(name) = key.strip_prefix("file:") else { return true };
    lockdown::settings(root).map_or(true, |v| !lockdown::tier(root, &v, name, None).starts_with("unwatched"))
}
/// Review content stays internal, separate from notes (which humans can delete).
fn review_path(root: &Path, tid: &str, rid: &str) -> Result<PathBuf, Failure> {
    Ok(task_path(root, tid)?.with_extension("").join("reviews").join(format!("{rid}.json")))
}
fn clear_reviews(s: &Store, tid: &str) -> Result<(), Failure> {
    let dir = task_path(&s.root, tid)?.with_extension("").join("reviews");
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}
fn review_state(s: &Store, t: &Value) -> Result<Value, Failure> {
    let home = scope::dir(&scope::top(&s.root), &s.root);
    let mut projects = json!({});
    let mut context = json!({});
    for (dir, st) in scope_stores(s, t)? {
        let current = snapshot(&st)?;
        let mut entities = json!({});
        for c in t["changes"].as_array().unwrap().iter().filter(|c| project_of(c, &home) == dir) {
            let key = c["entity"].as_str().unwrap();
            entities[key] = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
        }
        projects[&dir] = entities;
        let mut files = json!({});
        for (name, tier) in lockdown::tree_files(&st.root, &lockdown::settings(&st.root)?)? {
            if tier == "watched" || tier == "protected" {
                files[&name] = json!(sha256(&fs::read(st.root.join(&name))?));
            }
        }
        // Include materialized source too: an outside edit may not yet be absorbed into the store.
        for (key, row) in current.as_object().unwrap() {
            if let Some(module) = key.strip_prefix("module:") {
                let name = format!("{}/{module}.gleam", row[1].as_str().unwrap());
                files[&name] = json!(fs::read(st.root.join(&name)).ok().map(|b| sha256(&b)));
            }
        }
        // Gleam's dependency lock is deliberately unwatched, but affects review context.
        files["manifest.toml"] = json!(fs::read(st.root.join("manifest.toml")).ok().map(|b| sha256(&b)));
        context[dir] = json!([current, files]);
    }
    let instructions = json!([
        t["title"],
        t["body"],
        t["links"],
        t["questions"],
        t["notes"].as_array().unwrap().iter().filter(|n| n["author"] == "human").collect::<Vec<_>>()
    ]);
    Ok(json!({"projects":projects,"context":sha256(context.to_string().as_bytes()),
        "instructions":sha256(instructions.to_string().as_bytes())}))
}
fn previous_review(t: &Value) -> Option<&Value> {
    t["runs"].as_array()?.iter().rev().find(|r| r["kind"] == "review" && r["outcome"] == "reviewed" && r["state"] == "ended")
}
fn review_checkpoint(s: &Store, t: &Value) -> Result<Value, String> {
    let r = previous_review(t).ok_or("no completed review")?;
    let checkpoint = (|| -> Result<Value, Failure> {
        let p = review_path(&s.root, t["id"].as_str().unwrap(), r["id"].as_str().unwrap())?;
        serde_json::from_slice(&fs::read(p)?).map_err(|e| Failure(e.to_string(), 1))
    })()
    .map_err(|_| "missing or legacy review checkpoint")?;
    if checkpoint["run"] != r["id"] || !checkpoint["projects"].is_object() {
        return Err("missing or legacy review checkpoint".into());
    }
    let scope = scope_stores(s, t).map_err(|e| e.0)?.into_iter().map(|(d, _)| d).collect::<HashSet<_>>();
    if scope != checkpoint["projects"].as_object().unwrap().keys().cloned().collect() {
        return Err("project scope changed since review".into());
    }
    Ok(checkpoint)
}
/// `task diff`: what each entity this task changed looked like before the task,
/// against now. `names` narrows it; past `budget`, the rest are listed by name.
pub fn task_diff(s: &Store, tid: &str, names: &[&str], budget: usize) -> Out {
    let t = task(&s.root, tid)?;
    let since = names.contains(&"--since-review");
    let names: Vec<_> = names.iter().copied().filter(|n| *n != "--since-review").collect();
    let checkpoint = since.then(|| review_checkpoint(s, &t));
    let baseline = checkpoint.as_ref().and_then(|c| c.as_ref().ok());
    let heading = match &checkpoint {
        Some(Ok(c)) => format!("Since completed review {} (content at review start):\n\n", c["run"].as_str().unwrap()),
        Some(Err(reason)) => format!("Full task diff: {reason}; falling back to the state before the task.\n\n"),
        None => String::new(),
    };
    let projects = scope_stores(s, &t)?;
    let home = scope::dir(&scope::top(&s.root), &s.root);
    let (mut out, mut left) = (String::new(), vec![]);
    for (dir, st) in &projects {
        let current = snapshot(st)?;
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let before: HashMap<String, Value> = rows
            .query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .map(|r| r.map(|(k, v)| (k, serde_json::from_str(&v).unwrap())))
            .collect::<Result<_, _>>()?;
        let text = |key: &str, v: &Value| -> Option<String> {
            if key.starts_with("symbol:") {
                (v[7].as_i64() == Some(1)).then(|| v[4].as_str().unwrap_or("").to_string())
            } else if key.starts_with("module:") {
                v[2].as_str().map(|h| h.to_string())
            } else {
                let bytes = json_bytes(v.as_array()?);
                Some(String::from_utf8(bytes).unwrap_or_else(|e| format!("(binary, {} bytes)", e.as_bytes().len())))
            }
        };
        for c in t["changes"].as_array().unwrap().iter().filter(|c| project_of(c, &home) == dir) {
            let (key, name) = (c["entity"].as_str().unwrap(), c["name"].as_str().unwrap_or(""));
            let Some(original) = before.get(key) else { continue };
            // Entities first touched after the review use their task before-state.
            let old = baseline.and_then(|b| b["projects"][dir].get(key)).unwrap_or(original);
            if !reviewable(&st.root, key) {
                continue;
            }
            if !names.is_empty()
                && !names.iter().any(|n| *n == format!("{dir}: {name}") || *n == name || key.split_once(':').is_some_and(|(_, k)| k == *n))
            {
                continue;
            }
            let new = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
            if baseline.is_some() && old == &new {
                continue;
            }
            // A task across projects names each change's project (D-61).
            let shown = if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() };
            let part = match (text(key, old), text(key, &new)) {
                (a, b) if a == b && baseline.is_some() => format!("## {shown} (metadata or binary content changed)\n"),
                (a, b) if a == b => format!("## {shown} (back to its state before the task)\n"),
                (None, Some(b)) => format!("## {shown} (added)\n{}", line_diff("", &b)),
                (Some(a), None) => format!("## {shown} (deleted)\n{}", line_diff(&a, "")),
                (a, b) => format!("## {shown} (changed)\n{}", line_diff(&a.unwrap_or_default(), &b.unwrap_or_default())),
            };
            if !out.is_empty() && out.len() + part.len() > budget {
                left.push(shown);
            } else {
                out += &part;
                out.push('\n');
            }
        }
    }
    if out.is_empty() {
        return Ok(heading
            + if baseline.is_some() && names.is_empty() {
                "No task entity changes since this review.\n"
            } else if names.is_empty() {
                "This task has no changes.\n"
            } else {
                "No change of this task matches.\n"
            });
    }
    if !left.is_empty() {
        out += &format!(
            "Left out to fit the reply; read with `task diff {}<name>`: {}\n",
            if since { "--since-review " } else { "" },
            left.join(", ")
        );
    }
    Ok(heading + &out)
}
/// Line diff with two lines of context; `…` marks skipped lines.
fn line_diff(a: &str, b: &str) -> String {
    let ops = diff_ops(a, b);
    let (mut out, mut gap) = (String::new(), false);
    for k in 0..ops.len() {
        if ops[k.saturating_sub(2)..(k + 3).min(ops.len())].iter().any(|o| o.0 != ' ') {
            if gap {
                out += "  …\n";
            }
            gap = false;
            out += &format!("{} {}\n", ops[k].0, ops[k].1);
        } else {
            gap = true;
        }
    }
    if gap {
        out += "  …\n";
    }
    out
}
/// The lines of `b` (from 0) that a line diff from `a` marks: added lines, and
/// the line where lines were removed.
pub(super) fn changed_lines(a: &str, b: &str) -> Vec<usize> {
    let (mut out, mut line) = (vec![], 0);
    for (op, _) in diff_ops(a, b) {
        match op {
            '+' => {
                out.push(line);
                line += 1;
            }
            '-' => out.push(line.min(b.lines().count().saturating_sub(1))),
            _ => line += 1,
        }
    }
    out.dedup();
    out
}
pub fn diff_ops<'a>(a: &'a str, b: &'a str) -> Vec<(char, &'a str)> {
    let (a, b): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (x, y) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // ponytail: O(n·m) LCS table over the changed middle; use Myers if whole-file rewrites get large.
    let mut l = vec![vec![0u32; y.len() + 1]; x.len() + 1];
    for i in (0..x.len()).rev() {
        for j in (0..y.len()).rev() {
            l[i][j] = if x[i] == y[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let mut ops: Vec<(char, &str)> = a[..pre].iter().map(|s| (' ', *s)).collect();
    let (mut i, mut j) = (0, 0);
    while i < x.len() || j < y.len() {
        if i < x.len() && j < y.len() && x[i] == y[j] {
            ops.push((' ', x[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < x.len() && (j == y.len() || l[i + 1][j] >= l[i][j + 1]) {
            ops.push(('-', x[i]));
            i += 1;
        } else {
            ops.push(('+', y[j]));
            j += 1;
        }
    }
    ops.extend(a[a.len() - suf..].iter().map(|s| (' ', *s)));
    ops
}
pub fn file_snapshot(root: &Path, a: &[&str]) -> Result<Value, Failure> {
    let mut v = json!({});
    if matches!(a[0], "write" | "remove")
        && let Some(name) = a.get(1)
    {
        let p = lockdown::path(root, name)?;
        let name = file_name(p.strip_prefix(root).unwrap());
        v[format!("file:{name}")] = json!(fs::read(p).ok());
    }
    if a[0] == "run" {
        fn collect(root: &Path, dir: &Path, settings: &toml::Value, out: &mut Value) -> Result<(), Failure> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let p = entry.path();
                let name = p.strip_prefix(root).unwrap().to_string_lossy();
                if entry.file_type()?.is_symlink()
                    || [".solace", ".git", "build"].contains(&name.as_ref())
                    || entry.file_name() == "node_modules"
                {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    collect(root, &p, settings, out)?;
                } else if lockdown::tier(root, settings, &name, None) == "watched" {
                    out[format!("file:{name}")] = json!(fs::read(&p)?);
                }
            }
            Ok(())
        }
        collect(root, root, &lockdown::settings(root)?, &mut v)?;
    }
    Ok(v)
}
fn restore(s: &Store, states: &[(String, Value)]) -> Result<(), Failure> {
    for (key, v) in states {
        if let Some(id) = key.strip_prefix("symbol:") {
            if v.is_null() {
                s.db.execute("DELETE FROM symbols WHERE id=?", [id])?;
            } else {
                s.db.execute(
                    "INSERT OR REPLACE INTO symbols(id,module,name,kind,text,ord,revision,live) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        v[0].as_str(),
                        v[1].as_str(),
                        v[2].as_str(),
                        v[3].as_str(),
                        v[4].as_str(),
                        v[5].as_i64(),
                        v[6].as_str(),
                        v[7].as_i64()
                    ],
                )?;
            }
        } else if let Some(name) = key.strip_prefix("module:") {
            if v.is_null() {
                if let Ok(dir) = s.db.query_row("SELECT dir FROM modules WHERE path=?", [name], |r| r.get::<_, String>(0)) {
                    let p = s.root.join(dir).join(format!("{name}.gleam"));
                    if p.exists() {
                        fs::remove_file(p)?;
                    }
                }
                s.db.execute("DELETE FROM modules WHERE path=?", [name])?;
            } else {
                s.db.execute("INSERT OR REPLACE INTO modules VALUES (?1,?2,?3)", params![v[0].as_str(), v[1].as_str(), v[2].as_str()])?;
            }
        } else if let Some(name) = key.strip_prefix("file:") {
            let p = lockdown::path(&s.root, name)?;
            if let Some(bytes) = v.as_array() {
                fs::create_dir_all(p.parent().unwrap())?;
                fs::write(p, json_bytes(bytes))?;
            } else if p.exists() {
                fs::remove_file(p)?;
            }
        }
    }
    s.materialize(&s.index()?)?;
    Ok(())
}
/// Undoes a task's changes in every project of its scope (D-61): each is
/// restored and checked, and a failure puts back the projects already restored.
fn revert(s: &Store, t: &mut Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap().to_string();
    let projects = scope_stores(s, t)?;
    let home = scope::dir(&scope::top(&s.root), &s.root);
    let named = |dir: &str, key: &str| -> String {
        let name = t["changes"].as_array().unwrap().iter().find(|c| c["entity"] == key && project_of(c, &home) == dir);
        let name = name.and_then(|c| c["name"].as_str()).unwrap_or(key);
        if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() }
    };
    let (mut plans, mut conflicts) = (vec![], vec![]);
    for (dir, st) in &projects {
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let mut states = vec![];
        for row in rows.query_map([&tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (k, v) = row?;
            states.push((k, serde_json::from_str::<Value>(&v).unwrap()));
        }
        for (key, _) in &states {
            let later:Option<String>=st.db.query_row("SELECT task FROM board_touches WHERE entity=?1 AND seq > (SELECT MIN(seq) FROM board_touches WHERE task=?2 AND entity=?1) AND task!=?2 ORDER BY seq DESC LIMIT 1",params![key,tid],|r|r.get(0)).optional()?;
            if let Some(other) = later {
                conflicts.push(format!("{}: task {other}", named(dir, key)));
            }
        }
        plans.push((dir, st, states));
    }
    if !conflicts.is_empty() {
        return err("E7007", conflicts.join(", "));
    }
    let mut done: Vec<(&Store, Vec<(String, Value)>)> = vec![];
    for (dir, st, states) in plans.into_iter().filter(|p| !p.2.is_empty()) {
        let current = snapshot(st)?;
        let mut undo = vec![];
        for (key, _) in &states {
            let v = if let Some(name) = key.strip_prefix("file:") {
                json!(fs::read(lockdown::path(&st.root, name)?).ok())
            } else {
                current[key].clone()
            };
            undo.push((key.clone(), v));
        }
        st.db.execute_batch("SAVEPOINT revert_task")?;
        let checked = (|| -> Result<(), Failure> {
            restore(st, &states)?;
            // A project of plain files (D-60) has no code to type-check.
            if st.root.join("gleam.toml").exists() {
                st.check()?;
            }
            Ok(())
        })();
        if let Err(e) = checked {
            st.db.execute_batch("ROLLBACK TO revert_task; RELEASE revert_task")?;
            restore(st, &undo)?;
            for (earlier, undo) in done.iter().rev() {
                restore(earlier, undo)?;
            }
            let other = tasks(&st.root)?
                .iter()
                .filter(|x| x["id"] != tid.as_str())
                .map(|x| format!("task {}: {}", x["id"], x["changes"]))
                .collect::<Vec<_>>()
                .join("\n");
            let place = if projects.len() > 1 { format!("in {dir}: ") } else { String::new() };
            return err("E7008", format!("{place}{}\n{other}", e.0));
        }
        st.db.execute_batch("RELEASE revert_task")?;
        done.push((st, undo));
    }
    for (_, st) in &projects {
        st.db.execute("DELETE FROM board_touches WHERE task=?", [&tid])?;
        drafts::clear(st, &tid)?;
        drop_before(st, &tid)?;
    }
    clear_reviews(s, &tid)?;
    t["changes"] = json!([]);
    end_runs(t, "reverted");
    Ok(())
}
/// A completed task's changes become approved in every project it changed.
fn settle(s: &Store, t: &Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    clear_reviews(s, tid)?;
    for (_, st) in scope_stores(s, t)? {
        outside::approve(&st, tid)?;
        drop_before(&st, tid)?;
    }
    Ok(())
}
/// Where a task keeps files as they were before it changed them.
fn before_dir(root: &Path, tid: &str) -> PathBuf {
    root.join(".solace/board").join(tid).join("before")
}
/// File `name` as it was before the task changed it; with extension `absent`
/// when it didn't exist.
pub fn before_file(root: &Path, tid: &str, name: &str) -> PathBuf {
    before_dir(root, tid).join(sha256(name.as_bytes()))
}
/// Changes whenever the store's code does.
pub fn fingerprint(s: &Store) -> Out {
    Ok(sha256(snapshot(s)?.to_string().as_bytes()))
}
fn drop_before(s: &Store, tid: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM board_before WHERE task=?", [tid])?;
    let p = before_dir(&s.root, tid);
    if p.exists() {
        fs::remove_dir_all(p)?;
    }
    Ok(())
}

fn continue_answered(s: &Store, w: &Value, t: &mut Value) -> Result<(), Failure> {
    if !t["paused_from"].is_null() || t["questions"].as_array().unwrap().iter().any(|q| q["answer"].is_null()) {
        return Ok(());
    }
    let Some(run) = t["runs"].as_array().unwrap().iter().find(|r| r["state"] == "waiting") else {
        return Ok(());
    };
    // Use the configured continuation, never a replacement run or a human stamp.
    let next = allowed(w, t).into_iter().find(|name| {
        let a = &w["actions"][name];
        a["run"] == run["kind"] && a["revert"] != true && a["stamp"].is_null() && a["unstamp"].is_null()
    });
    if let Some(name) = next {
        action(s, w, t, &json!({"action":name}))?;
        t["attention"] = Value::Null;
        t["finished"] = Value::Null;
    } else {
        flag(t, "answers saved; no continuation action is available in this column");
    }
    Ok(())
}

fn action(s: &Store, w: &Value, t: &mut Value, body: &Value) -> Result<Option<(String, Value)>, Failure> {
    let name = body["action"].as_str().unwrap_or("");
    let allowed = allowed(w, t);
    if !allowed.iter().any(|x| x == name) {
        return err("E7001", format!("not allowed; allowed: {}", allowed.join(", ")));
    }
    if name == "pause" {
        if t["paused_from"].is_null() {
            t["paused_from"] = t["column"].clone();
        }
        return Ok(None);
    }
    if name == "resume" {
        if !t["paused_from"].is_null() {
            t["column"] = t["paused_from"].clone();
            t["paused_from"] = Value::Null;
        }
        continue_answered(s, w, t)?;
        return Ok(None);
    }
    // Stop ends every open run; the killed agent's conversation isn't kept.
    if name == "stop" {
        if !active(t) {
            return err("E7003", "no agent is working on this task");
        }
        let discussing = discussion(t);
        end_runs(t, "stopped");
        if let Some(rid) = discussing {
            compact(t, &rid);
        }
        return Ok(None);
    }
    if name == "end_discussion" {
        let rid = discussion(t).ok_or_else(|| Failure("E7003 no discussion is going".into(), 1))?;
        let i = run_at(t, &rid)?;
        end_run(&mut t["runs"][i], "discussed");
        compact(t, &rid);
        return Ok(None);
    }
    if name == "discuss" {
        if body["text"].as_str().is_none_or(|s| s.trim().is_empty()) {
            return err("E7001", "a discussion starts with a message");
        }
        if active(t) {
            return err("E7003", "task already has a run");
        }
    }
    let discuss = json!({"run":"discuss","model":true});
    let a = if name == "discuss" { &discuss } else { &w["actions"][name] };
    let run = a["run"].as_str().unwrap_or("");
    if matches!(run, "investigate" | "work" | "new_work")
        && body["text"].as_str().is_none_or(|s| s.is_empty())
        && t["questions"].as_array().unwrap().iter().any(|q| q["answer"].is_null())
    {
        return err("E7002", "answer the questions or supply text");
    }
    // `work` and `investigate` continue a waiting agent of their kind.
    let waiting = t["runs"].as_array().unwrap().iter().position(|r| r["state"] == "waiting" && r["kind"] == run);
    let resume = waiting.is_some();
    if !run.is_empty() && active(t) && !resume && run != "new_work" && a["revert"] != true {
        return err("E7003", "task already has a run");
    }
    if matches!(run, "work" | "new_work") {
        let top = scope::top(&s.root);
        claim_projects(&s.root, w, &t["id"], &scope::of(t, &scope::dir(&top, &s.root)))?;
    }
    let config = system()?;
    let chosen = body.get("model").unwrap_or(&t["model"]);
    if a["model"] == true && !resume && body["model"].as_str().is_none() {
        return err("E7001", "this action requires a model");
    }
    let mut selected = if !run.is_empty() && !resume { Some(model(&config, chosen.as_str().unwrap_or(""))?) } else { None };
    if let Some(m) = selected.as_mut() {
        // A chosen effort must be offered; the task's remembered one applies only when still offered.
        if let Some(e) = body.get("effort").filter(|e| !e.is_null()) {
            let e = e.as_str().filter(|e| efforts(m).contains(e)).ok_or_else(|| {
                Failure(format!("E7001 effort not offered by {}; offered: {}", m["id"].as_str().unwrap_or(""), efforts(m).join(", ")), 1)
            })?;
            m["effort"] = json!(e);
        } else if body.get("model").is_none() && t["effort"].as_str().is_some_and(|e| efforts(m).contains(&e)) {
            m["effort"] = t["effort"].clone();
        }
    }
    if let Some(reason) = selected.as_ref().and_then(unavailable) {
        return err("E7011", format!("model {} is unavailable: {reason}", selected.unwrap()["id"].as_str().unwrap_or("")));
    }
    // Validate the gate before any destructive step.
    let mut proposed = t.clone();
    if let Some(stamp) = a["stamp"].as_str() {
        proposed["stamps"][stamp] = json!({"who":"human","time":now()});
    }
    if let Some(stamp) = a["unstamp"].as_str() {
        proposed["stamps"].as_object_mut().unwrap().remove(stamp);
    }
    if let Some(to) = a.get("to") {
        move_to(w, &mut proposed, to)?;
    }
    if a["revert"] == true {
        revert(s, t)?;
        proposed["changes"] = t["changes"].clone();
        proposed["runs"] = t["runs"].clone();
    }
    *t = proposed;
    if let Some(text) = body["text"].as_str()
        && run != "discuss"
    {
        note(t, "human", None, text);
        for q in t["questions"].as_array_mut().unwrap() {
            if q["answer"].is_null() {
                q["answer"] = json!(text);
            }
        }
    }
    // Moving the task, or starting or continuing a run, ends a pause; a paused run would hang.
    if a.get("to").is_some() || resume || selected.is_some() {
        t["paused_from"] = Value::Null;
    }
    if resume {
        let i = waiting.unwrap();
        t["runs"][i]["state"] = json!("running");
        t["runs"][i]["reply"] = json!({"questions":t["questions"],"text":body["text"]});
        return Ok(None);
    }
    if run == "new_work" {
        end_runs(t, "replaced");
    }
    if let Some(m) = selected {
        let rid = unique_id();
        let kind = if run == "new_work" { "work" } else { run };
        let log = s.root.join(".solace/board").join(t["id"].as_str().unwrap()).join(format!("{rid}.jsonl"));
        t["runs"].as_array_mut().unwrap().push(json!({"id":rid,"kind":kind,"model":m["id"],"effort":m["effort"],"state":"running","started":now(),"log":log,"fresh":run == "new_work" || a["revert"] == true}));
        if kind == "review" {
            let mut checkpoint = review_state(s, t)?;
            checkpoint["run"] = json!(rid);
            let path = review_path(&s.root, t["id"].as_str().unwrap(), &rid)?;
            fs::create_dir_all(path.parent().unwrap())?;
            write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap())?;
        }
        // The discussion's first message is its first turn.
        if kind == "discuss" {
            note(t, "human", Some(&rid), body["text"].as_str().unwrap_or(""));
        }
        t["finished"] = Value::Null;
        if kind == "work" {
            t["model"] = m["id"].clone();
            t["effort"] = m["effort"].clone();
        }
        return Ok(Some((rid, m)));
    }
    if column(w, t)["final"] == true {
        end_runs(t, "completed");
        settle(s, t)?;
    }
    Ok(None)
}

/// The models this Solace install knows; the same for every project.
fn models() -> Result<Value, Failure> {
    Ok(json!(
        system()?["models"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|m| {
                let reason = unavailable(m);
                json!({"id":m["id"],"label":m["label"],"agent":m["agent"],"effort":m["effort"],"efforts":efforts(m),"available":reason.is_none(),"reason":reason})
            })
            .collect::<Vec<_>>()
    ))
}
fn path_of(url: &str) -> Vec<&str> {
    url.split('?').next().unwrap_or("").trim_matches('/').split('/').collect()
}
/// The locks a change takes: its project's, and for a task, every project in
/// its scope (and in a new scope it is given), in folder order (D-61).
fn guard(root: &Path, parts: Vec<&str>, body: &Value) -> Result<Vec<fs::File>, Failure> {
    let ["tasks", tid, ..] = parts.as_slice() else { return Ok(vec![lock(root)?]) };
    let top = scope::top(root);
    let home = scope::dir(&top, root);
    let members = workspace::members(&top)?;
    loop {
        let Ok(t) = task(root, tid) else { return Ok(vec![lock(root)?]) };
        let seen = scope::of(&t, &home);
        let mut dirs = seen.clone();
        dirs.extend(body["scope"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(String::from)));
        let mut roots: Vec<PathBuf> = dirs.iter().filter(|d| members.contains(d)).map(|d| scope::path(&top, d)).collect();
        roots.push(root.into());
        let held = scope::lock_all(&roots)?;
        // The scope may have changed while this waited for the locks.
        if task(root, tid).is_ok_and(|t| scope::of(&t, &home) == seen) {
            return Ok(held);
        }
    }
}
/// A scope as a request gives it: projects on this board, sorted, at least one.
fn checked_scope(root: &Path, v: &Value) -> Result<Vec<String>, Failure> {
    let top = scope::top(root);
    let members = workspace::members(&top)?;
    let mut out = vec![];
    for d in v.as_array().ok_or_else(|| Failure("E7001 scope must be a list of project folders".into(), 1))? {
        match d.as_str() {
            Some(d) if members.iter().any(|m| m == d) => out.push(d.to_string()),
            _ => return err("E7001", format!("{d} is not a project on this board; its projects: {}", members.join(", "))),
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        return err("E7001", "a task needs at least one project");
    }
    Ok(out)
}
/// Refuses a work run in any of `dirs` while another task holds it (D-61).
fn claim_projects(root: &Path, w: &Value, tid: &Value, dirs: &[String]) -> Result<(), Failure> {
    let top = scope::top(root);
    for dir in dirs {
        let others: Vec<Value> = scope::tasks_in(&top, dir)?.into_iter().map(|(_, t)| t).filter(|t| t["id"] != *tid).collect();
        let place = if dirs.len() > 1 { format!(" in {dir}") } else { String::new() };
        if others.iter().any(|t| t["runs"].as_array().unwrap().iter().any(|r| r["kind"] == "work" && r["state"] != "ended")) {
            return err("E7005", format!("another task has a work run{place}"));
        }
        // Code changed outside Solace waits for a human before agents build on it (D-58).
        if others.iter().any(|t| t["unexpected"] == true && column(w, t)["final"] != true) {
            return err(
                "E7012",
                format!(
                    "complete or cancel the {} task{place} first: it holds code changed outside Solace that no one has reviewed",
                    outside::TITLE
                ),
            );
        }
    }
    Ok(())
}
/// `POST /tasks/{id}/scope` (D-61): the human changes a task's projects while
/// no agent of it is working: between runs, paused, or waiting on a question.
fn set_scope(s: &Store, w: &Value, mut t: Value, body: &Value) -> Result<Value, Failure> {
    if t["runs"].as_array().unwrap().iter().any(|r| r["state"] == "running") && t["paused_from"].is_null() {
        return err(
            "E7003",
            "an agent is working on this task; change its projects between runs, while it is paused, or while it waits on a question",
        );
    }
    if column(w, &t)["final"] == true {
        return err("E7004", "final task");
    }
    let top = scope::top(&s.root);
    let home = scope::dir(&top, &s.root);
    let old = scope::of(&t, &home);
    let new = checked_scope(&s.root, &body["scope"])?;
    for d in old.iter().filter(|d| !new.contains(d)) {
        if t["changes"].as_array().unwrap().iter().any(|c| project_of(c, &home) == d) {
            return err("E7013", format!("this task has changed {d}; undo its changes before taking {d} out of its projects"));
        }
    }
    // A paused or waiting run already holds its projects; one added joins them now.
    if t["runs"].as_array().unwrap().iter().any(|r| r["kind"] == "work" && r["state"] != "ended") {
        let added: Vec<String> = new.iter().filter(|d| !old.contains(d)).cloned().collect();
        claim_projects(&s.root, w, &t["id"], &added)?;
    }
    t["scope"] = json!(new);
    if new.contains(&home) {
        save(s, &t)?;
        return Ok(t);
    }
    // The agent works from the task's home, so the home changes only between runs.
    if active(&t) {
        return err("E7013", format!("this task's agent works from {home}; take {home} out of its projects once the run has ended"));
    }
    move_task(s, &mut t, &scope::path(&top, &new[0]))?;
    Ok(t)
}
/// Moves a task to another project of its scope when its home leaves it.
fn move_task(s: &Store, t: &mut Value, to: &Path) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap().to_string();
    let (from, into) = (s.root.join(".solace/board").join(&tid), to.join(".solace/board").join(&tid));
    let target = open(to)?;
    // A conversation is tied to the folder it ran in; the next run starts a new one, with a handoff.
    t["conversation"] = Value::Null;
    if from.exists() {
        fs::rename(&from, &into)?;
        for r in t["runs"].as_array_mut().unwrap() {
            if let Some(rest) = r["log"].as_str().and_then(|l| Path::new(l).strip_prefix(&from).ok()) {
                r["log"] = json!(into.join(rest));
            }
        }
    }
    save(&target, t)?;
    fs::remove_file(task_path(&s.root, &tid)?)?;
    event(s, json!({"removed":tid}))
}
pub fn api(root: &Path, method: &str, url: &str, body: Value) -> Result<Value, Failure> {
    // Reads skip the project lock, so the board stays responsive while an agent command holds it.
    // Task files are replaced by rename and the store waits out commits, so a read sees whole states.
    let _guard = if method == "GET" { None } else { Some(guard(root, path_of(url), &body)?) };
    let s = open(root)?;
    let w = flow(root)?;
    let (path, query_text) = url.split_once('?').unwrap_or((url, ""));
    let query: HashMap<String, String> =
        query_text.split('&').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if matches!(parts.first(), Some(&"graph" | &"nodes" | &"tests" | &"files")) {
        return view::api(&s, method, &parts, &query, &body);
    }
    match (method, parts.as_slice()) {
        ("GET", ["workflow"]) => Ok(w),
        ("GET", ["models"]) => models(),
        ("GET", ["tasks"]) => {
            let mut ts = tasks(root)?;
            for t in &mut ts {
                for k in ["notes", "changes"] {
                    t.as_object_mut().unwrap().remove(k);
                }
            }
            Ok(json!(ts))
        }
        ("POST", ["tasks"]) => {
            let title = body["title"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| Failure("E7001 title required".into(), 1))?;
            if body.get("body").is_some_and(|x| !x.is_string()) || body.get("links").is_some_and(|x| !x.is_array()) {
                return err("E7001", "invalid task fields");
            }
            let top = scope::top(root);
            let home = scope::dir(&top, root);
            let scope = match body.get("scope") {
                Some(v) => checked_scope(root, v)?,
                None => vec![home.clone()],
            };
            if !scope.contains(&home) {
                return err("E7001", format!("a task is created in a project of its scope; post it to one of {}", scope.join(", ")));
            }
            let rank = tasks(root)?.iter().filter_map(|t| t["rank"].as_f64()).fold(0., f64::max) + 1.;
            let mut t = new_task(title, body["body"].as_str().unwrap_or(""), &w["columns"][0]["id"], rank);
            t["scope"] = json!(scope);
            t["links"] = body.get("links").cloned().unwrap_or(json!([]));
            save(&s, &t)?;
            Ok(t)
        }
        (_, ["tasks", tid, rest @ ..]) => {
            let mut t = task(root, tid)?;
            match (method, rest) {
                ("GET", []) => return Ok(t),
                ("GET", ["runs", rid]) => {
                    let mut run = t["runs"][run_at(&t, rid)?].clone();
                    run["transcript"] = json!(transcript(Path::new(run["log"].as_str().unwrap_or(""))));
                    return Ok(run);
                }
                ("PATCH", []) => {
                    if active(&t) {
                        return err("E7003", "run going");
                    }
                    if column(&w, &t)["final"] == true {
                        return err("E7004", "final task");
                    }
                    for k in ["title", "body", "links", "rank"] {
                        if let Some(v) = body.get(k) {
                            if (matches!(k, "title" | "body") && !v.is_string())
                                || (k == "links" && !v.is_array())
                                || (k == "rank" && !v.is_number())
                            {
                                return err("E7001", "invalid task field");
                            }
                            t[k] = v.clone();
                        }
                    }
                }
                ("POST", ["scope"]) => return set_scope(&s, &w, t, &body),
                ("POST", ["notes"]) => {
                    let text = body["text"].as_str().ok_or_else(|| Failure("E7001 text required".into(), 1))?;
                    // While a discussion is going, a human note is its next turn.
                    let discussing = discussion(&t);
                    note(&mut t, "human", discussing.as_deref(), text);
                }
                // Notes are only appended, so a note's position names it.
                ("DELETE", ["notes", i]) => {
                    let notes = t["notes"].as_array_mut().unwrap();
                    let i = i.parse().ok().filter(|i| *i < notes.len()).ok_or_else(|| Failure("E7001 unknown note".into(), 1))?;
                    notes.remove(i);
                }
                ("POST", ["questions", qid]) => {
                    let q = t["questions"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|q| q["id"] == *qid)
                        .ok_or_else(|| Failure("E7001 unknown question".into(), 1))?;
                    let answer = body
                        .get("text")
                        .or_else(|| body.get("option"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Failure("E7001 answer required".into(), 1))?;
                    if body.get("text").is_none() && !q["options"].as_array().unwrap().contains(&json!(answer)) {
                        return err("E7001", "option is not offered");
                    }
                    q["answer"] = json!(answer);
                    if !q["settings"].is_null() && answer == "Approve" {
                        apply_settings(root, q)?;
                    }
                    continue_answered(&s, &w, &mut t)?;
                }
                ("POST", ["actions"]) => {
                    let launch = action(&s, &w, &mut t, &body)?;
                    t["attention"] = Value::Null;
                    t["finished"] = Value::Null;
                    save(&s, &t)?;
                    if let Some((rid, m)) = launch {
                        start(root.to_path_buf(), t.clone(), rid, m);
                    } else if column(&w, &t)["final"] == true {
                        let root = root.to_path_buf();
                        let tid = t["id"].as_str().unwrap().to_string();
                        std::thread::spawn(move || {
                            if let Ok(_guard) = conversation_lock(&root, &tid) {
                                let _ = clean_conversation(&root, &tid);
                            }
                        });
                    }
                    return Ok(t);
                }
                _ => return err("E7001", "unknown route"),
            }
            save(&s, &t)?;
            Ok(t)
        }
        _ => err("E7001", "unknown route"),
    }
}
fn start(root: PathBuf, t: Value, rid: String, m: Value) {
    std::thread::spawn(move || {
        let tid = t["id"].as_str().unwrap();
        let result = (|| {
            // A follow-up may be queued while the previous agent is exiting.
            // Never open one provider conversation in two processes.
            let _guard = conversation_lock(&root, tid)?;
            let result = agent_process(&root, &task(&root, tid)?, &rid, &m, true);
            let result = match result {
                Err(Failure(_, SESSION_GONE)) => agent_process(&root, &task(&root, tid)?, &rid, &m, false),
                other => other,
            };
            let current = task(&root, tid)?;
            if t["runs"].as_array().unwrap().iter().any(|r| r["id"] == rid && matches!(r["kind"].as_str(), Some("review" | "discuss")))
                || m["run"].is_array()
            {
                let dir = conversation_dir(&root, tid, &rid)?;
                if dir.exists() {
                    fs::remove_dir_all(dir)?;
                }
            }
            if column(&flow(&root)?, &current)["final"] == true {
                clean_conversation(&root, tid)?;
            }
            result
        })();
        if let Err(e) = result {
            let _ = finish(&root, tid, &rid, Some(&e.0));
        }
    });
}

fn conversation_lock(root: &Path, tid: &str) -> Result<fs::File, Failure> {
    let dir = task_path(root, tid)?.with_extension("");
    fs::create_dir_all(&dir)?;
    lock_file(&dir.join("agent.lock"))
}

fn conversation_dir(root: &Path, tid: &str, home: &str) -> Result<PathBuf, Failure> {
    // Reuse the id validator before allowing any deletion or launch path.
    let _ = task_path(root, home)?;
    Ok(task_path(root, tid)?.with_extension("").join(home))
}

fn clean_conversation(root: &Path, tid: &str) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let mut t = task(root, tid)?;
    if let Some(home) = t["conversation"]["home"].as_str() {
        let p = conversation_dir(root, tid, home)?;
        if p.exists() {
            fs::remove_dir_all(p)?;
        }
    }
    t["conversation"] = Value::Null;
    drafts::clear(&s, tid)?;
    save(&s, &t)
}

fn session_id(log: &Path, agent: &str) -> Option<String> {
    fs::read_to_string(log).ok()?.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find_map(|e| {
        let id = if agent == "codex" && e["type"] == "thread.started" {
            e["thread_id"].as_str()
        } else if agent == "claude" && e["type"] == "system" && e["subtype"] == "init" {
            e["session_id"].as_str()
        } else {
            None
        }?;
        (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| id.to_string())
    })
}

/// `agent_process`'s failure when the agent can't resume its conversation:
/// the run starts again with a fresh one.
const SESSION_GONE: i32 = 3;
fn agent_process(root: &Path, t: &Value, rid: &str, m: &Value, allow_resume: bool) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let run = &t["runs"][run_at(t, rid)?];
    let log = PathBuf::from(run["log"].as_str().unwrap());
    fs::create_dir_all(log.parent().unwrap())?;
    if run["state"] == "ended" {
        return Ok(());
    }
    // A review or discussion has its own conversation, never the author's.
    let author = !matches!(run["kind"].as_str(), Some("review" | "discuss"));
    let native = m["run"].is_null();
    let claude_home = env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".claude"));
    // macOS keychain entries are keyed to CLAUDE_CONFIG_DIR. Keep that sign-in
    // working without copying credentials: use a fresh, nonpersistent handoff.
    let persistent = native
        && !(m["agent"] == "claude"
            && cfg!(target_os = "macos")
            && !claude_home.join(".credentials.json").exists()
            && m["endpoint"].is_null()
            && env::var_os("ANTHROPIC_API_KEY").is_none()
            && env::var_os("ANTHROPIC_AUTH_TOKEN").is_none());
    let previous = &t["conversation"];
    // Compare launch-relevant configuration, not labels. A changed model or endpoint gets a handoff.
    let compatibility = json!({"agent":m["agent"],"model":m["model"],"endpoint":m["endpoint"],"effort":m["effort"],"run":m["run"]});
    let resume = if allow_resume && author && persistent && run["fresh"] != true && previous["config"] == compatibility {
        previous["home"]
            .as_str()
            .and_then(|home| conversation_dir(root, tid, home).ok())
            .filter(|p| p.is_dir())
            .and_then(|_| previous["session"].as_str())
            .map(str::to_string)
    } else {
        None
    };
    let home = if resume.is_some() {
        previous["home"].as_str().unwrap().to_string()
    } else if allow_resume {
        rid.to_string()
    } else {
        format!("{rid}-fallback")
    };
    let dir = conversation_dir(root, tid, &home)?;
    if author
        && resume.is_none()
        && let Some(old) = previous["home"].as_str()
    {
        let old = conversation_dir(root, tid, old)?;
        if old != dir && old.exists() {
            fs::remove_dir_all(old)?;
        }
    }
    let custom = m["run"].as_array();
    let (prompt, images) = continuation_brief(t, run, resume.is_some(), root, custom.is_some());
    let binary = env::current_exe()?;
    let config = system()?;
    let timeout = config["task_timeout"].as_u64().unwrap_or(3600);
    let agent = m["agent"].as_str().unwrap_or("codex");
    let launch: Value = serde_json::from_str(&lockdown::agent_config(agent, &dir, &binary, timeout)?).unwrap();
    let mut argv: Vec<String> = if let Some(a) = custom {
        a.iter().filter_map(|x| x.as_str().map(String::from)).collect()
    } else if agent == "codex" {
        vec![
            "codex".into(),
            "exec".into(),
            "--json".into(),
            "--skip-git-repo-check".into(),
            "-m".into(),
            m["model"].as_str().unwrap_or("").into(),
        ]
    } else {
        vec![
            "claude".into(),
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--model".into(),
            m["model"].as_str().unwrap_or("").into(),
        ]
    };
    if custom.is_none() {
        if agent == "claude" && !persistent {
            argv.push("--no-session-persistence".into());
        }
        if let Some(session) = &resume {
            if agent == "codex" {
                argv.insert(2, "resume".into());
                argv.push(session.clone());
            } else {
                argv.extend(["--resume".into(), session.clone()]);
            }
        }
        if agent == "claude" {
            if let Some(e) = m["effort"].as_str() {
                argv.extend(["--effort".into(), e.into()]);
            }
            if !images.is_empty() {
                argv.extend(["--input-format".into(), "stream-json".into()]);
            }
        }
        argv.extend(launch["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()));
        if agent == "codex" {
            if let Some(e) = m["effort"].as_str() {
                argv.extend(["-c".into(), format!("model_reasoning_effort={}", json!(e))]);
            }
            // `--image` takes several values, so each goes before `--json`, never before `-`.
            let at = if resume.is_some() { 3 } else { 2 };
            for image in images.iter().rev() {
                argv.splice(at..at, ["--image".into(), image.to_string_lossy().into_owned()]);
            }
            argv.extend(endpoint_args(m));
            argv.push("-".into());
        }
    }
    if argv.is_empty() {
        return err("E7001", "empty model command");
    }
    // Authentication is shared; settings, plugins and hooks are isolated.
    if agent == "codex" && custom.is_none() {
        let auth = PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".codex/auth.json");
        if auth.exists() && !dir.join("auth.json").exists() {
            std::os::unix::fs::symlink(auth, dir.join("auth.json"))?;
        }
    }
    if agent == "claude" && native {
        let auth = claude_home.join(".credentials.json");
        if auth.exists() && !dir.join(".credentials.json").exists() {
            std::os::unix::fs::symlink(auth, dir.join(".credentials.json"))?;
        }
    }
    if author {
        let _guard = lock(root)?;
        let mut current = task(root, tid)?;
        // Only an exited process has a reusable session; a server crash during
        // a turn falls back instead of resuming a possibly orphaned process.
        current["conversation"] = if persistent { json!({"home":home,"config":compatibility,"session":null}) } else { Value::Null };
        save(&open(root)?, &current)?;
    }
    let file = fs::File::create(&log)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(root).process_group(0).stdin(Stdio::piped()).stdout(file.try_clone()?).stderr(file);
    for (k, v) in launch["env"].as_object().unwrap() {
        cmd.env(k, v.as_str().unwrap());
    }
    if agent == "claude" && custom.is_none() {
        if persistent {
            cmd.env("CLAUDE_CONFIG_DIR", &dir);
        }
        if let Some(e) = m.get("endpoint") {
            cmd.env("ANTHROPIC_BASE_URL", e["base_url"].as_str().unwrap_or(""));
            // The key is read from the named variable at launch and never stored.
            if let Some(key) = e["key_env"].as_str().and_then(|k| env::var(k).ok()) {
                cmd.env("ANTHROPIC_AUTH_TOKEN", key);
            }
            if env::var_os("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none()
                && let Some(n) = endpoint_context(e["base_url"].as_str().unwrap_or(""), m["model"].as_str().unwrap_or(""))
            {
                cmd.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", n.to_string());
            }
        }
    }
    cmd.env("SOLACE_TASK", tid).env("SOLACE_RUN", rid).env("SOLACE_SESSION", rid).env("SOLACE_BIN", &binary);
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    if agent == "claude" && custom.is_none() && !images.is_empty() {
        // Claude Code takes images only inside a stream-json user message.
        let mut content = vec![json!({"type":"text","text":prompt})];
        for image in &images {
            let media = image_type(&image.to_string_lossy()).unwrap_or("image/png");
            content.push(json!({"type":"image","source":{"type":"base64","media_type":media,"data":base64(&fs::read(image)?)}}));
        }
        writeln!(stdin, "{}", json!({"type":"user","message":{"role":"user","content":content}}))?;
    } else {
        writeln!(stdin, "{prompt}")?;
    }
    drop(stdin);
    let forced = supervise(root, tid, rid, &mut child, &log, resume.is_some(), Duration::from_secs(timeout), GRACEFUL_EXIT)?;
    if author && persistent {
        let _guard = lock(root)?;
        let mut current = task(root, tid)?;
        current["conversation"]["session"] = if forced { Value::Null } else { json!(session_id(&log, agent)) };
        // Only notes actually briefed, plus the agent's own notes, are known to
        // this conversation. A human may add feedback during graceful exit.
        current["conversation"]["notes_seen"] = json!(
            t["notes"]
                .as_array()
                .unwrap()
                .iter()
                .chain(current["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent"))
                .map(note_fingerprint)
                .collect::<Vec<_>>()
        );
        save(&open(root)?, &current)?;
    } else {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// How long a run that called a finishing tool may take to exit (spec 15).
const GRACEFUL_EXIT: Duration = Duration::from_secs(30);

/// Watches a launched agent until it exits or is killed, and says whether it
/// was killed. Running time (not paused time) past `timeout` ends the run; a
/// run that finished gets `grace` to exit; any other ended run is killed.
#[allow(clippy::too_many_arguments)]
fn supervise(
    root: &Path,
    tid: &str,
    rid: &str,
    child: &mut std::process::Child,
    log: &Path,
    resumed: bool,
    timeout: Duration,
    grace: Duration,
) -> Result<bool, Failure> {
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    let mut ending = None;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let current = task(root, tid)?;
        let r = &current["runs"][run_at(&current, rid)?];
        if r["state"] == "running" && current["paused_from"].is_null() {
            elapsed += last.elapsed();
        }
        last = Instant::now();
        let expired = r["state"] != "ended" && elapsed >= timeout;
        let ended = r["state"] == "ended";
        if ended && matches!(r["outcome"].as_str(), Some("finished" | "reported" | "reviewed" | "discussed")) {
            ending.get_or_insert_with(Instant::now);
        }
        if let Some(status) = child.try_wait()? {
            if !ended {
                let entries = transcript(log);
                let error = entries.iter().rev().find(|e| e["kind"] == "error").and_then(|e| e["text"].as_str()).unwrap_or("");
                let raw = fs::read_to_string(log).unwrap_or_default().to_lowercase();
                // Retry only a failed session load, before any tool execution. Never retry quota or work failures.
                if resumed
                    && !status.success()
                    && !entries.iter().any(|e| e["kind"] == "tool")
                    && ["session not found", "no session found", "failed to resume", "could not find session", "unable to find session"]
                        .iter()
                        .any(|s| raw.contains(s))
                {
                    return Err(Failure("session unavailable; using a fresh handoff".into(), SESSION_GONE));
                }
                let said = if !error.is_empty() { Some(error.to_string()) } else { last_words(&entries) };
                let message = format!("agent exited without finishing ({status}){}", said.map(|s| format!(":\n\n{s}")).unwrap_or_default());
                finish(root, tid, rid, Some(&message))?;
            }
            return Ok(false);
        }
        if expired || (ended && ending.is_none()) || ending.is_some_and(|at| at.elapsed() >= grace) {
            lockdown::kill_group(child);
            if expired {
                finish(root, tid, rid, Some("task timeout"))?;
            }
            return Ok(true);
        }
    }
}

/// Follow-up review context comes from live notes joined to their runs, never transcripts.
fn review_context(s: &Store, t: &Value) -> Out {
    let mut out = String::from("\nReview context:\n");
    if let Some(previous) = previous_review(t) {
        let rid = previous["id"].as_str().unwrap();
        out += &format!("Previous completed review: {rid}\n");
        let feedback: Vec<_> = t["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent").collect();
        if feedback.is_empty() {
            out += "Previous feedback was deleted or is unavailable; do not infer its assessment.\n";
        }
        for n in feedback {
            out += &format!("Review feedback ({rid}): {}\n", n["text"].as_str().unwrap_or(""));
        }
        let at = run_at(t, rid)?;
        for n in t["notes"].as_array().unwrap() {
            let subsequent = n["run"].as_str().and_then(|id| run_at(t, id).ok()).filter(|i| *i > at);
            if let Some(i) = subsequent {
                out += &format!(
                    "Subsequent {} ({}) — {}: {}\n",
                    t["runs"][i]["kind"].as_str().unwrap_or("run"),
                    t["runs"][i]["id"].as_str().unwrap(),
                    n["author"].as_str().unwrap_or(""),
                    n["text"].as_str().unwrap_or("")
                );
            } else if n["author"] == "human" {
                out += &format!("Human feedback: {}\n", n["text"].as_str().unwrap_or(""));
            }
        }
    }
    match review_checkpoint(s, t) {
        Ok(checkpoint) => {
            out += "Start with `task diff --since-review`. Assess the delta, prior unresolved findings and affected dependencies. Honor human requests for a fresh full review using `task diff`.\n";
            let current = review_state(s, t)?;
            if checkpoint["changed_during_review"] != false {
                out += "Project or instructions changed during the previous review, or stability is unknown. Its checkpoint represents review-start content only; recheck affected code and context before carrying forward any assessment.\n";
            }
            if checkpoint["context"] == current["context"]
                && checkpoint["instructions"] == current["instructions"]
                && checkpoint["changed_during_review"] == false
            {
                out += "Code and relevant context are unchanged since the previous review. A short report carrying forward the previous assessment is sufficient if its feedback is available. Unchanged code must not turn Needs work into Looks good.\n";
            } else {
                out += "Code or relevant context changed since the previous review. Even an empty task delta does not establish unchanged dependencies or instructions; inspect affected context and rerun relevant verification.\n";
            }
            out += &task_diff(s, t["id"].as_str().unwrap(), &["--since-review"], 8000)?;
        }
        Err(reason) => out += &format!("Start with `task diff` and assess the whole task: {reason}. No usable delta baseline.\n"),
    }
    Ok(out)
}

/// The brief, plus the images its notes reference, which are attached to the prompt.
fn continuation_brief(t: &Value, run: &Value, resumed: bool, root: &Path, custom: bool) -> (String, Vec<PathBuf>) {
    let kind = run["kind"].as_str().unwrap_or("");
    let mut packet = t.clone();
    if resumed {
        let seen: HashSet<&str> = t["conversation"]["notes_seen"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        packet["notes"] =
            json!(t["notes"].as_array().unwrap().iter().filter(|n| !seen.contains(note_fingerprint(n).as_str())).collect::<Vec<_>>());
    }
    let tid = t["id"].as_str().unwrap_or("");
    let mut images = vec![];
    for n in packet["notes"].as_array_mut().into_iter().flatten() {
        let text = note_images(n["text"].as_str().unwrap_or(""), &format!("/tasks/{tid}/files/"), &mut images);
        n["text"] = json!(text);
    }
    let images: Vec<PathBuf> = images.iter().filter_map(|name| image_path(root, tid, name).ok()).collect();
    packet["home"] = json!(scope::dir(&scope::top(root), root));
    let mut out = brief(&packet, kind);
    if resumed {
        out = out.replace(START, "Use the continuation context below.");
    }
    if !matches!(kind, "review" | "discuss") && t["runs"].as_array().is_some_and(|r| r.len() > 1) {
        if resumed {
            out += "\nContinue the author's conversation. The role and permissions above replace those of earlier runs. Skip broad orientation; inspect current versions of symbols you need before editing.\n";
        } else {
            out += "\nFresh conversation: use this handoff and the notes above to continue existing work. Reconcile partial edits before proceeding.\n";
        }
        for r in t["runs"].as_array().unwrap().iter().rev().filter(|r| r["id"] != run["id"]).take(3) {
            out += &format!("Previous {}: {}\n", r["kind"].as_str().unwrap_or("run"), r["outcome"].as_str().unwrap_or("unfinished"));
            if let Some(log) = r["log"].as_str()
                && let Some(said) = last_words(&transcript(Path::new(log)))
            {
                out += &format!("Last activity: {said}\n");
            }
        }
    }
    if let Ok(s) = open(root) {
        if kind == "review" {
            match review_context(&s, t) {
                Ok(context) => out += &context,
                Err(e) => out += &format!("\nReview context unavailable: {}. Start with `task diff` and assess the whole task.\n", e.0),
            }
        }
        // A discussion runs no checks, so it gets the decision titles instead of the verification record.
        if kind == "discuss" {
            let decisions = fs::read_to_string(scope::top(root).join("DECISIONS.md")).unwrap_or_default();
            let titles: Vec<String> = decisions.lines().filter_map(|l| l.strip_prefix("## ")).filter(|l| l.starts_with("D-")).map(|l| format!("- {l}")).collect();
            if !titles.is_empty() {
                out += &format!("\nDecisions (DECISIONS.md; `grep D-<n>` then `read` for a record):\n{}\n", titles.join("\n"));
            }
        } else if let Some(checks) = t["verification"].as_object() {
            let current = review_state(&s, t).ok().and_then(|v| v["context"].as_str().map(str::to_string));
            out += "\nLast recorded verification (rerun stale checks before claiming success):\n";
            for (command, v) in checks {
                let stale = current.as_deref() != v["base"].as_str();
                out += &format!(
                    "- {command}: {}{}\n{}\n",
                    if v["passed"] == true { "passed" } else { "failed" },
                    if stale { " (stale: project changed)" } else { "" },
                    v["summary"].as_str().unwrap_or("")
                );
            }
        }
        if let Ok(drafts) = drafts::list_for(&s, t["id"].as_str().unwrap_or(""))
            && !drafts.is_empty()
        {
            out += &format!("\nRetained drafts (view with `view --draft <id>`):\n{drafts}\n");
        }
    }
    if !images.is_empty() {
        out += "\nImages in the notes are attached to this message as [image 1], [image 2], ... in order.\n";
        if custom {
            for (i, p) in images.iter().enumerate() {
                out += &format!("[image {}: {}]\n", i + 1, p.display());
            }
        }
    }
    out += "\nAfter the finishing tool returns, give a short final response and exit normally.\n";
    (out, images)
}

/// Replaces each markdown image of an uploaded file with `[image N]`, numbering
/// files in first-seen order. Other images and text are left as written.
fn note_images(text: &str, prefix: &str, found: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("![") {
        let after = &rest[i..];
        let Some(open) = after.find("](") else { break };
        let Some(close) = after[open..].find(')').map(|c| open + c) else { break };
        out += &rest[..i];
        match after[open + 2..close].strip_prefix(prefix).filter(|n| image_type(n).is_some()) {
            Some(name) => {
                let n = found.iter().position(|x| x == name).unwrap_or_else(|| {
                    found.push(name.to_string());
                    found.len() - 1
                });
                out += &format!("[image {}]", n + 1);
            }
            None => out += &after[..=close],
        }
        rest = &after[close + 1..];
    }
    out + rest
}

/// Note images: content-checked raster formats only (SVG can carry script).
const IMAGE_LIMIT: usize = 5 * 1024 * 1024;
fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}
/// The media type of a stored image's name, which is `<16 hex>.<ext>`.
fn image_type(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match ext {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
fn image_path(root: &Path, tid: &str, name: &str) -> Result<PathBuf, Failure> {
    let p = task_path(root, tid)?.with_extension("").join("files").join(name);
    if image_type(name).is_none() || !p.exists() {
        return err("E7001", "unknown image");
    }
    Ok(p)
}
pub fn upload(root: &Path, tid: &str, bytes: &[u8]) -> Result<Value, Failure> {
    task(root, tid)?;
    if bytes.len() > IMAGE_LIMIT {
        return err("E7001", "image over 5 MB");
    }
    let ext = image_kind(bytes).ok_or_else(|| Failure("E7001 only PNG, JPEG, GIF or WebP images".into(), 1))?;
    let name = format!("{}.{ext}", &sha256(bytes)[..16]);
    let dir = task_path(root, tid)?.with_extension("").join("files");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(&name), bytes)?;
    Ok(json!({"name":name,"url":format!("/tasks/{tid}/files/{name}")}))
}
fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn note_fingerprint(note: &Value) -> String {
    sha256(note.to_string().as_bytes())
}

pub fn record_verification(s: &Store, tid: &str, command: &str, result: &Out) -> Result<(), Failure> {
    let mut t = task(&s.root, tid)?;
    if !t["verification"].is_object() {
        t["verification"] = json!({});
    }
    let summary = match result {
        Ok(out) => out,
        Err(e) => &e.0,
    };
    t["verification"][command] =
        json!({"passed":result.is_ok(),"base":review_state(s, &t)?["context"],"summary":summary.chars().take(1000).collect::<String>()});
    save(s, &t)
}
/// An agent's prompt: what its run is for, how to start, ask and finish, then
/// what it needs from the task (Spec 15 §6), as plain text.
/// A fresh run's first step; a resumed run continues instead.
const START: &str = "Start with `session start --task`, with the task's title and description as `text`.";
fn brief(t: &Value, kind: &str) -> String {
    let start = START;
    let ask = "If you need a decision from the human, ask with `task ask` (`text` and 2–4 `options`); the call waits for the answer.";
    let responses = "Write human-facing task messages in Markdown, including the text sent to task tools. Divide the body into relevant sections with short headings (for example, ### Findings and ### Verification), with blank lines around headings and lists. Omit irrelevant sections. When asking a question, explain what you need and why it matters (for example, ### Question and ### Why this matters); keep the options concise.";
    let read_only = "This run is read-only: reading, `check`, `test`, `build` and `probe` work; `submit`, `delete`, `write`, `remove`, `format` and `run` are refused.";
    let role = match kind {
        "review" => format!(
            "Review the changes this task made (listed below) and give feedback. Follow the review context below: the first review assesses the whole task with `task diff`; follow-ups start with `task diff --since-review` when a completed checkpoint exists. Then read what you need in small batches, one concern at a time. Don't review generated files (build output such as a bundled `priv/app.js`, lockfiles, installed dependencies); test them instead, and rebuild them with the project's commands if a check needs them. {read_only} In a review, `run <name>` is allowed for commands marked `allowed_in_reviews` in `solace.toml`; use those to verify (for example JavaScript tests). Finish with `task note`, the feedback in `text`. Begin the feedback with exactly one assessment label: **Assessment: Looks good** or **Assessment: Needs work**. Follow it with findings and verification that explain the assessment, including any checks not performed."
        ),
        "discuss" => format!(
            "Discuss this task with the human, who asks questions and works out ideas with you. Answer from the high-level context below (the task, its notes and changes); read code only when a question needs it, and briefly. {read_only} The human's message is the last note. Reply with `task note`, the reply in `text`; the call waits and returns the human's next message. Keep replying until a call says the discussion has ended, then stop without further calls."
        ),
        "work" => format!(
            "Make the change this task describes. {start} {ask} Finish with `task done`, a summary in `text`: what you changed and how you checked it."
        ),
        _ => format!(
            "Investigate this task before any work on it: answer what it asks, or find out what the change involves and plan it. {read_only} {start} {ask} Finish with `task done`, the report in `text`: your findings, or the plan for whoever does the work."
        ),
    };
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let mut out = format!(
        "You are working on a task from this project's task board. You work only through the Solace tool: there is no shell and no file access.\n{role}\n{responses}\n\nTask: {}\n",
        text(&t["title"])
    );
    if !text(&t["body"]).is_empty() {
        out += &format!("\n{}\n", text(&t["body"]));
    }
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            out += &format!("\n{title}:\n{}\n", lines.join("\n"));
        }
    };
    let list = |k: &str| t[k].as_array().cloned().unwrap_or_default();
    section("Links", list("links").iter().map(|l| format!("- {}: {}", text(&l["name"]), text(&l["url"]))).collect());
    section(
        "Notes so far",
        list("notes")
            .iter()
            .map(|n| {
                let body = text(&n["text"]);
                let run = list("runs").into_iter().find(|r| r["id"] == n["run"]);
                match run {
                    Some(r) => format!("- {} ({} run {}): {body}", text(&n["author"]), text(&r["kind"]), text(&r["id"])),
                    None => format!("- {}: {body}", text(&n["author"])),
                }
            })
            .collect(),
    );
    section(
        "Questions",
        list("questions")
            .iter()
            .map(|q| match q["answer"].as_str() {
                Some(a) => format!("- {}\n  Answer: {a}", text(&q["text"])),
                None => format!("- {}\n  (not answered yet)", text(&q["text"])),
            })
            .collect(),
    );
    // A task across projects (D-61): where the agent works, and how it reaches the others.
    let home = text(&t["home"]);
    let projects: Vec<String> = list("scope").iter().map(text).collect();
    if projects.len() > 1 {
        section(
            "Projects",
            vec![format!(
                "This task covers the projects {}. You work in {home}; for another of them, add `--project <folder>` to any command (`inspect`, `submit`, `test`, `write`...). A project not listed is out of this task's reach: if you need it, ask the human with `task ask`.",
                projects.join(", ")
            )],
        );
    }
    let shown = |c: &Value| match c["project"].as_str() {
        Some(p) if projects.len() > 1 => format!("- {p}: {}", text(&c["name"])),
        _ => format!("- {}", text(&c["name"])),
    };
    section("Task changes", list("changes").iter().map(shown).collect());
    out
}
/// The agent's last message, else its last error or output, cut to 1000 bytes.
fn last_words(entries: &[Value]) -> Option<String> {
    let last = |kind: &str| entries.iter().rev().find(|e| e["kind"] == kind).and_then(|e| e["text"].as_str());
    let text = last("message").or_else(|| last("error")).or_else(|| last("output"))?.trim();
    if text.is_empty() {
        return None;
    }
    let cut = (text.len().saturating_sub(1000)..=text.len()).find(|&i| text.is_char_boundary(i)).unwrap();
    Some(if cut > 0 { format!("…{}", &text[cut..]) } else { text.to_string() })
}
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
/// A tool call's input for display: the command first, then any other arguments.
fn tool_input(args: &Value) -> (String, String) {
    let command = args["command"].as_str().unwrap_or("").to_string();
    let rest = match args {
        Value::Object(m) => m
            .iter()
            .filter(|(k, _)| *k != "command")
            .map(|(k, v)| format!("{k}:\n{}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    };
    (command, rest)
}
/// Reads an agent's log (Codex `exec --json` or Claude Code `stream-json`) into
/// entries a board can show: message, thinking, tool, usage, error or output.
// ponytail: rereads the whole log per request; keep an offset if logs get large.
pub fn transcript(log: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    let mut tools: HashMap<String, usize> = HashMap::new();
    // Claude Code: the context of its latest API call, and output per call (events repeat per content block).
    let (mut context, mut output, mut ended) = (None, HashMap::new(), false);
    let text = fs::read_to_string(log).unwrap_or_default();
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            if line.trim().is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last["kind"] == "output" => last["text"] = json!(format!("{}\n{line}", last["text"].as_str().unwrap())),
                _ => out.push(json!({"kind":"output","text":line})),
            }
            continue;
        };
        let item = &e["item"];
        match (e["type"].as_str().unwrap_or(""), item["type"].as_str().unwrap_or("")) {
            // Codex
            ("item.started" | "item.updated" | "item.completed", "mcp_tool_call") => {
                let (command, input) = tool_input(&item["arguments"]);
                let status = match item["status"].as_str() {
                    Some("completed") => "ok",
                    Some("failed") => "failed",
                    _ => "running",
                };
                let output = if item["error"].is_null() { text_of(&item["result"]["content"]) } else { text_of(&item["error"]["message"]) };
                let entry = json!({"kind":"tool","title":format!("{} {command}", item["tool"].as_str().unwrap_or("tool")),"input":input,"output":output,"status":status});
                let id = item["id"].as_str().unwrap_or("").to_string();
                match tools.get(&id) {
                    Some(&i) => out[i] = entry,
                    None => {
                        tools.insert(id, out.len());
                        out.push(entry);
                    }
                }
            }
            ("item.completed", "agent_message") => out.push(json!({"kind":"message","text":item["text"]})),
            ("item.completed", "reasoning") => out.push(json!({"kind":"thinking","text":item["text"]})),
            ("item.completed", "web_search") => out.push(json!({"kind":"tool","title":format!("web search {}", item["query"].as_str().unwrap_or("")),"input":"","output":"","status":"ok"})),
            ("item.completed", "error") => out.push(json!({"kind":"error","text":item["message"]})),
            ("turn.completed", _) => {
                let u = &e["usage"];
                out.push(json!({"kind":"usage","text":format!("{} input tokens ({} cached), {} output tokens", u["input_tokens"], u["cached_input_tokens"], u["output_tokens"])}));
            }
            ("error", _) => out.push(json!({"kind":"error","text":e["message"]})),
            ("turn.failed", _) => out.push(json!({"kind":"error","text":e["error"]["message"]})),
            // Claude Code
            ("assistant", _) => {
                let u = &e["message"]["usage"];
                if u.is_object() {
                    context = Some(context_tokens(u));
                    output.insert(e["message"]["id"].to_string(), u["output_tokens"].as_u64().unwrap_or(0));
                }
                for c in e["message"]["content"].as_array().unwrap_or(&vec![]) {
                    match c["type"].as_str() {
                        Some("text") => out.push(json!({"kind":"message","text":c["text"]})),
                        Some("thinking") => out.push(json!({"kind":"thinking","text":c["thinking"]})),
                        Some("tool_use") => {
                            let (command, input) = tool_input(&c["input"]);
                            let name = c["name"].as_str().unwrap_or("tool").trim_start_matches("mcp__solace__");
                            tools.insert(c["id"].as_str().unwrap_or("").to_string(), out.len());
                            out.push(json!({"kind":"tool","title":format!("{name} {command}"),"input":input,"output":"","status":"running"}));
                        }
                        _ => {}
                    }
                }
            }
            ("user", _) => {
                for c in e["message"]["content"].as_array().unwrap_or(&vec![]) {
                    if let Some(&i) = c["tool_use_id"].as_str().and_then(|id| tools.get(id)) {
                        out[i]["output"] = json!(text_of(&c["content"]));
                        out[i]["status"] = json!(if c["is_error"] == true { "failed" } else { "ok" });
                    }
                }
            }
            ("result", _) => {
                ended = true;
                let u = &e["usage"];
                let last = context.map(|c| format!("; last context {c} tokens")).unwrap_or_default();
                out.push(json!({"kind":"usage","text":format!("{} input tokens ({} cached), {} output tokens{last}", u["input_tokens"], u["cache_read_input_tokens"], u["output_tokens"])}));
                if e["is_error"] == true {
                    out.push(json!({"kind":"error","text":text_of(&e["result"])}));
                }
            }
            _ => {}
        }
    }
    if let (Some(c), false) = (context, ended) {
        // Live, until the run's totals arrive. Some endpoints report output only at the end.
        out.push(json!({"kind":"usage","text":format!("Context {c} tokens; {} output tokens so far", output.values().sum::<u64>())}));
    }
    out
}
/// Tokens an API call sent. Anthropic counts cached input separately and always
/// reports cache creation; endpoints that omit it (LM Studio) include cache in `input_tokens`.
fn context_tokens(u: &Value) -> u64 {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    if u.get("cache_creation_input_tokens").is_some() {
        n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens")
    } else {
        n("input_tokens")
    }
}
fn finish(root: &Path, tid: &str, rid: &str, failure: Option<&str>) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let mut t = task(root, tid)?;
    let w = flow(root)?;
    let i = run_at(&t, rid)?;
    let r = &mut t["runs"][i];
    if r["state"] == "ended" {
        return Ok(());
    }
    end_run(r, failure.unwrap_or("finished"));
    // Only a failed work run moves the task; a failed review or investigation stays in its column.
    let work = r["kind"] == "work";
    if let Some(message) = failure {
        note(&mut t, "solace", Some(rid), message);
        flag(&mut t, message);
        if work {
            move_to(&w, &mut t, &w["events"]["failed"])?;
        }
    }
    save(&s, &t)
}

pub fn context() -> Option<(String, String)> {
    Some((env::var("SOLACE_TASK").ok()?, env::var("SOLACE_RUN").ok()?))
}
/// Waits until the run may act, holding its home's lock and, for a command in
/// another project of its scope (`also`), that project's too (D-61).
pub fn agent_guard(root: &Path, tid: &str, rid: &str, a: &[&str], also: Option<&Path>) -> Result<Vec<fs::File>, Failure> {
    loop {
        let guard = if a.first() == Some(&"task") {
            guard(root, vec!["tasks", tid], &json!({}))?
        } else {
            scope::lock_all(&[root.to_path_buf(), also.unwrap_or(root).to_path_buf()])?
        };
        let t = task(root, tid)?;
        let r = &t["runs"][run_at(&t, rid)?];
        if r["state"] == "ended" {
            return err("E7003", "run ended");
        }
        if !t["paused_from"].is_null() {
            drop(guard);
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if HOST.contains(&a[0]) {
            return err("E7010", "host command is unavailable in a run");
        }
        if r["kind"] == "review" && a[0] == "run" && a.len() > 1 {
            let marked = lockdown::settings(also.unwrap_or(root))
                .ok()
                .and_then(|v| v.get("commands")?.get(a[1])?.get("allowed_in_reviews")?.as_bool())
                .unwrap_or(false);
            if !marked {
                return err("E7010", "in a review, run only works for commands marked allowed_in_reviews in solace.toml");
            }
        } else if r["kind"] != "work" && CHANGES.contains(&a[0]) && a[0] != "build" {
            return err("E7010", "this run is read-only");
        }
        // The human may have changed the task's projects while it was paused.
        if let Some(p) = also {
            let top = scope::top(root);
            let dir = scope::dir(&top, p);
            if !scope::of(&t, &scope::dir(&top, root)).contains(&dir) {
                return lockdown::error("E6007", &dir, "no longer one of this task's projects");
            }
        }
        return Ok(guard);
    }
}
pub fn task_command(
    s: &Store,
    tid: &str,
    rid: &str,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    options: &[String],
    guard: Vec<fs::File>,
) -> Out {
    let mut t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let i = run_at(&t, rid)?;
    let text = if a.len() > 2 { a[2..].join(" ") } else { input()? };
    let discussing = t["runs"][i]["kind"] == "discuss";
    match a.get(1).copied() {
        // A discussion's reply waits for the human's next message.
        Some("note") if discussing => {
            note(&mut t, "agent", Some(rid), &text);
            t["runs"][i]["state"] = json!("waiting");
            save(s, &t)?;
            drop(guard);
            return wait_discussion(s, tid, rid);
        }
        Some("done" | "ask") if discussing => return err("E7010", "a discussion goes on until the human ends it; reply with task note"),
        Some("note") => {
            note(&mut t, "agent", Some(rid), &text);
            if t["runs"][i]["kind"] == "review" {
                let path = review_path(&s.root, tid, rid)?;
                if let Ok(bytes) = fs::read(&path) {
                    let mut checkpoint: Value = serde_json::from_slice(&bytes).map_err(|e| Failure(e.to_string(), 1))?;
                    let current = review_state(s, &t)?;
                    checkpoint["changed_during_review"] = json!(
                        checkpoint["context"] != current["context"]
                            || checkpoint["instructions"] != current["instructions"]
                            || checkpoint["projects"] != current["projects"]
                    );
                    if checkpoint["changed_during_review"] == true {
                        note(
                            &mut t,
                            "solace",
                            Some(rid),
                            "Project or instructions changed during review. This feedback applies to the review-start checkpoint, not a verified finish-time state; recheck affected changes.",
                        );
                    }
                    write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap())?;
                }
                end_run(&mut t["runs"][i], "reviewed");
                t["finished"] = json!({"kind":"review","outcome":"reviewed","time":now()});
            }
        }
        Some("done") => {
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "reviews finish with task note");
            }
            note(&mut t, "agent", Some(rid), &text);
            if t["runs"][i]["kind"] == "investigate" {
                // The report is the result; the task stays in planning.
                end_run(&mut t["runs"][i], "reported");
            } else {
                end_run(&mut t["runs"][i], "finished");
                move_to(&w, &mut t, &w["events"]["finished"])?;
            }
            t["finished"] = json!({"kind":t["runs"][i]["kind"],"outcome":t["runs"][i]["outcome"],"time":now()});
        }
        Some("ask") => {
            if !(2..=4).contains(&options.len()) {
                return err("E7001", "task ask needs 2–4 options");
            }
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "review cannot move task with a question");
            }
            let qid = unique_id();
            t["questions"].as_array_mut().unwrap().push(json!({"id":qid,"text":text,"options":options}));
            t["runs"][i]["state"] = json!("waiting");
            flag(&mut t, "an agent asked a question");
            // An investigation waits in planning; `investigate` continues it.
            if t["runs"][i]["kind"] == "work" {
                move_to(&w, &mut t, &w["events"]["asked"])?;
            }
            save(s, &t)?;
            drop(guard);
            return wait_reply(s, tid, i);
        }
        _ => return usage("task note | task ask | task done, with the text in `text`; task diff [--since-review] [name ...]"),
    }
    if column(&w, &t)["final"] == true {
        settle(s, &t)?;
    }
    save(s, &t)?;
    Ok("Recorded.\n".into())
}

/// Holds the agent's call until its answers arrive and the run continues.
fn wait_reply(s: &Store, tid: &str, i: usize) -> Out {
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let _guard = lock(&s.root)?;
        let t = task(&s.root, tid)?;
        if t["runs"][i]["state"] == "ended" {
            return err("E7003", "run ended while waiting");
        }
        if t["runs"][i]["state"] == "running" && t["paused_from"].is_null() {
            return Ok(format!("{}\n", t["runs"][i]["reply"]));
        }
    }
}
/// Holds a discussion's reply until the human writes again or ends it. Waiting
/// doesn't count toward the task timeout.
fn wait_discussion(s: &Store, tid: &str, rid: &str) -> Out {
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let _guard = lock(&s.root)?;
        let mut t = task(&s.root, tid)?;
        let i = run_at(&t, rid)?;
        if t["runs"][i]["state"] == "ended" {
            return Ok("The discussion has ended. Stop now, without further calls.\n".into());
        }
        let notes = t["notes"].as_array().unwrap();
        let after = notes.iter().rposition(|n| n["run"] == rid && n["author"] == "agent").map_or(0, |l| l + 1);
        let new: Vec<&str> = notes[after..].iter().filter(|n| n["run"] == rid && n["author"] == "human").filter_map(|n| n["text"].as_str()).collect();
        if !new.is_empty() && t["paused_from"].is_null() {
            let reply = format!("The human replied:\n\n{}\n", new.join("\n\n"));
            t["runs"][i]["state"] = json!("running");
            save(s, &t)?;
            return Ok(reply);
        }
    }
}
/// A work run's `write solace.toml` or `write AGENTS.md`: checked now, written
/// only when a human approves it on the board (D-38, D-45).
/// `project` is the folder whose settings change: the home, or another project in the task (D-61).
#[allow(clippy::too_many_arguments)]
pub fn propose_settings(
    s: &Store,
    project: &Path,
    tid: &str,
    rid: &str,
    file: &str,
    base: &[u8],
    proposed: &str,
    guard: Vec<fs::File>,
) -> Out {
    if file == "solace.toml" {
        workflow(&lockdown::parse_settings(proposed)?)?;
        // Which projects share the board is the humans' choice (D-57).
        let projects = |text: &[u8]| std::str::from_utf8(text).ok()?.parse::<toml::Value>().ok()?.get("projects").cloned();
        if projects(base) != projects(proposed.as_bytes()) {
            return Err(Failure(
                "E6020 `projects` lists the projects on this root's board; only humans change it, from the board".into(),
                1,
            ));
        }
    }
    let base = std::str::from_utf8(base).map_err(|_| Failure(format!("E6020 {file} is not UTF-8"), 1))?;
    if base == proposed {
        return Ok(format!("{file} is unchanged.\n"));
    }
    let mut t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let i = run_at(&t, rid)?;
    let top = scope::top(&s.root);
    let dir = scope::dir(&top, project);
    // The file's path from the root folder, so a board of several projects says which one.
    let name = top.file_name().map(Path::new).unwrap_or(Path::new("")).join(project.strip_prefix(&top).unwrap_or(Path::new(""))).join(file);
    let name = name.to_string_lossy();
    t["questions"].as_array_mut().unwrap().push(json!({"id":unique_id(),"text":format!("Change {name}"),"options":["Approve","Reject"],
        "settings":{"file":file,"project":dir,"base":base,"proposed":proposed}}));
    t["runs"][i]["state"] = json!("waiting");
    flag(&mut t, "an agent proposed a settings change");
    move_to(&w, &mut t, &w["events"]["asked"])?;
    save(s, &t)?;
    drop(guard);
    wait_reply(s, tid, i)
}
/// Applies an approved settings question, unless the file changed since.
fn apply_settings(root: &Path, q: &mut Value) -> Result<(), Failure> {
    let file = q["settings"]["file"].as_str().unwrap_or("solace.toml").to_string();
    let p = match q["settings"]["project"].as_str() {
        Some(dir) => scope::path(&scope::top(root), dir).join(&file),
        None => root.join(&file),
    };
    let current = fs::read_to_string(&p).unwrap_or_default();
    let proposed = q["settings"]["proposed"].as_str().unwrap();
    if current != q["settings"]["base"] || (file == "solace.toml" && lockdown::parse_settings(proposed).and_then(|v| workflow(&v)).is_err())
    {
        q["answer"] = json!(format!("Not applied: {file} changed since the proposal; read it and propose again"));
        return Ok(());
    }
    fs::write(p, proposed)?;
    q["answer"] = json!("Approved and written");
    Ok(())
}

/// Ends runs a previous server left going, as after a restart.
fn recover_project(root: &Path) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let w = flow(root)?;
    lockdown::recover(root)?;
    event(&s, json!({"workflow":w}))?;
    for mut t in tasks(root)? {
        if active(&t) {
            let work = t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended" && r["kind"] == "work");
            end_runs(&mut t, "server restarted");
            flag(&mut t, "the Solace server restarted while an agent was working");
            if work {
                move_to(&w, &mut t, &w["events"]["failed"])?;
            }
            save(&s, &t)?;
        }
    }
    Ok(())
}

/// What one `solace serve` shows: the projects of one Solace root (D-57),
/// each held by this server alone. The root's own project (`.`) answers at
/// `/`; the others under `/p/<dir>`.
struct Board {
    top: PathBuf,
    held: std::sync::Mutex<HashMap<PathBuf, fs::File>>,
    /// Serializes changes to the root's project list.
    editing: std::sync::Mutex<()>,
    port: u16,
    /// The migration step running now, and the last one's failure (D-59).
    step: std::sync::Mutex<Option<String>>,
    failed: std::sync::Mutex<Option<String>>,
}
impl Board {
    /// Whether the root lists projects (or is a folder of them), so the board
    /// shows lanes; otherwise it is one project on its own.
    fn explicit(&self) -> bool {
        workspace::is_root(&self.top) || !self.top.join(".solace/store.db").exists()
    }
    /// `(dir, root)` for each set-up project.
    fn projects(&self) -> Result<Vec<(String, PathBuf)>, Failure> {
        Ok(workspace::members(&self.top)?
            .into_iter()
            .map(|d| {
                let r = scope::path(&self.top, &d);
                (d, r)
            })
            .filter(|(_, r)| r.join(".solace/store.db").exists())
            .collect())
    }
    /// Holds the project's serve lock for this server's life: a second
    /// server would end the first one's runs as a restart.
    fn claim(&self, dir: &str, root: &Path) -> Result<(), Failure> {
        let mut held = self.held.lock().unwrap();
        if held.contains_key(root) {
            return Ok(());
        }
        let Some(f) = try_lock_file(&root.join(".solace/serve.lock"))? else {
            let name = if dir == "." { root.display().to_string() } else { dir.to_string() };
            return fail(format!("{name} is already served by another `solace serve`; stop that one first."));
        };
        recover_project(root)?;
        held.insert(root.to_path_buf(), f);
        Ok(())
    }
    /// The project a request is for, and the path within it; `None` for the
    /// root's own routes.
    fn route(&self, url: &str) -> Result<Option<(PathBuf, String)>, Failure> {
        let projects = self.projects()?;
        let Some(after) = url.strip_prefix("/p/") else {
            return Ok(projects.into_iter().find(|(d, _)| d == ".").map(|(_, r)| (r, url.to_string())));
        };
        let end = after.find(['/', '?']).unwrap_or(after.len());
        let dir = view::percent_decode(&after[..end]);
        let Some((dir, root)) = projects.into_iter().find(|(d, _)| *d == dir && d != ".") else {
            return err("E7001", format!("unknown project {dir}"));
        };
        self.claim(&dir, &root)?;
        let rest = &after[end..];
        Ok(Some((root, if rest.starts_with('/') { rest.to_string() } else { format!("/{rest}") })))
    }
    fn listing(&self) -> Result<Value, Failure> {
        let projects: Vec<Value> = self
            .projects()?
            .into_iter()
            .map(|(dir, root)| {
                let (base, label) = if dir == "." {
                    (String::new(), self.top.file_name().map_or(".".into(), |n| n.to_string_lossy().to_string()))
                } else {
                    (format!("/p/{}", percent_encode(&dir)), dir.clone())
                };
                json!({"dir":dir,"label":label,"name":workspace::name(&root),"target":workspace::target(&root),"base":base})
            })
            .collect();
        Ok(json!({"workspace":self.explicit(),"projects":projects,"found":workspace::found(&self.top)?}))
    }
    /// `/migration` (D-59, Spec 17): the long steps run in the background,
    /// and those that move the served folder restart the server there.
    fn migration(self: &std::sync::Arc<Self>, method: &str, path: &str, body: &Value) -> Result<Value, Failure> {
        let action = match (method, path) {
            ("GET", "/migration") => {
                // While a step runs, the folders may be mid-swap: report only the step.
                let mut v = match self.step.lock().unwrap().clone() {
                    Some(step) => json!({"state":"running","top":self.top,"step":step,"candidates":[],"blockers":[],"undo":false}),
                    None => migrate::status(&self.top)?,
                };
                v["error"] = json!(self.failed.lock().unwrap().clone());
                return Ok(v);
            }
            ("POST", p) => p.trim_start_matches("/migration/").to_string(),
            _ => return err("E7001", "unknown route"),
        };
        let mut step = self.step.lock().unwrap();
        if step.is_some() {
            return err("E7003", "a migration step is already running");
        }
        let projects: Vec<String> =
            body["projects"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
        let smoke: Vec<String> = body["smoke"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::trim))
            .filter(|c| !c.is_empty())
            .map(String::from)
            .collect();
        match action.as_str() {
            "start" if projects.is_empty() => return err("E7001", "projects required"),
            "start" | "confirm" | "recheck" | "adopt" | "abandon" | "undo" => {}
            _ => return err("E7001", "unknown route"),
        }
        *step = Some("Starting".into());
        *self.failed.lock().unwrap() = None;
        drop(step);
        let me = self.clone();
        std::thread::spawn(move || {
            let report = |s: &str| *me.step.lock().unwrap() = Some(s.to_string());
            let copy = migrate::locate(&me.top);
            let moved = match action.as_str() {
                // A migration that stops partway keeps its copy: serve it, to show why.
                "start" => migrate::start(&me.top, &projects, &smoke, false, &report)
                    .map(Some)
                    .or_else(|e| if migrate::load(&copy).is_some() && me.top != copy { Ok(Some(copy.clone())) } else { Err(e) }),
                "confirm" => migrate::confirm_paths(&copy, &report).map(Some),
                "recheck" => migrate::recheck(&copy, &report).map(|_| None),
                "adopt" => migrate::adopt(&copy, &report).map(Some),
                "abandon" => migrate::abandon(&copy).map(|(original, _)| Some(original)),
                _ => migrate::undo(&me.top).map(Some),
            };
            match moved {
                Ok(Some(at)) => {
                    // /proc/self/exe still runs this binary after a reinstall has replaced its file.
                    let exe = Path::new("/proc/self/exe");
                    let exe = if exe.exists() { exe.to_path_buf() } else { env::current_exe().unwrap_or_else(|_| "solace".into()) };
                    let e = Command::new(exe).args(["serve", "--port", &me.port.to_string()]).current_dir(&at).exec();
                    *me.failed.lock().unwrap() = Some(format!("could not restart the server in {}: {e}", at.display()));
                }
                Ok(None) => {}
                Err(Failure(e, _)) => *me.failed.lock().unwrap() = Some(e),
            }
            *me.step.lock().unwrap() = None;
        });
        Ok(json!({"state":"running"}))
    }
    /// `/projects`: list, add, create and remove a workspace's projects.
    fn projects_api(self: &std::sync::Arc<Self>, method: &str, url: &str, body: &Value) -> Result<Value, Failure> {
        let path = url.split('?').next().unwrap_or("");
        match (method, path) {
            ("GET", "/projects") => return self.listing(),
            ("GET", "/models") => return models(),
            (_, p) if p == "/migration" || p.starts_with("/migration/") => return self.migration(method, p, body),
            _ => {}
        }
        let _edit = self.editing.lock().unwrap();
        let field =
            |k: &str| body[k].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| Failure(format!("E7001 {k} required"), 1));
        let dir = match (method, path) {
            ("POST", "/projects") => {
                let dir = field("dir")?;
                workspace::add(&self.top, dir)?;
                dir.to_string()
            }
            ("POST", "/projects/new") => {
                let name = field("name")?;
                let dir = body["dir"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(name);
                workspace::new_project(&self.top, dir, name, body["target"] == "javascript")?;
                dir.to_string()
            }
            ("DELETE", p) if p.starts_with("/projects/") => {
                let dir = view::percent_decode(&p["/projects/".len()..]);
                let root = scope::path(&self.top, &dir);
                // Tasks stored elsewhere may cover this project too (D-61).
                let covering = scope::tasks_in(&self.top, &dir)?;
                if covering.iter().any(|(_, t)| active(t)) {
                    return err("E7003", format!("an agent is working in {dir}; let it finish or stop it first"));
                }
                let w = flow(&self.top)?;
                if let Some((home, t)) = covering.iter().find(|(home, t)| *home != root && column(&w, t)["final"] != true) {
                    return err(
                        "E7013",
                        format!(
                            "the task \"{}\" in {} also covers {dir}; take {dir} out of its projects first",
                            t["title"].as_str().unwrap_or(""),
                            scope::dir(&self.top, home)
                        ),
                    );
                }
                workspace::remove(&self.top, &dir)?;
                self.held.lock().unwrap().remove(&root);
                return self.listing();
            }
            _ => return err("E7001", "unknown route"),
        };
        let root = scope::path(&self.top, &dir);
        if let Err(e) = self.claim(&dir, &root) {
            if dir != "." {
                workspace::remove(&self.top, &dir)?;
            }
            return Err(e);
        }
        self.listing()
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|c| if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") })
        .collect()
}

/// The largest request body the API reads, besides image uploads.
const BODY_LIMIT: usize = 1_048_576;
/// A request's body, or `None` when it's over `limit` bytes or can't be read.
fn read_body(req: &mut tiny_http::Request, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let read = req.as_reader().take(limit as u64 + 1).read_to_end(&mut bytes);
    (read.is_ok() && bytes.len() <= limit).then_some(bytes)
}
/// A JSON request body; an empty one is `{}`.
fn body_json(bytes: &[u8]) -> Result<Value, Failure> {
    if bytes.is_empty() { Ok(json!({})) } else { serde_json::from_slice(bytes).or_else(|_| err("E7001", "invalid JSON")) }
}
/// An API error as the board reads it: its code and message.
fn error_json(e: &Failure) -> Value {
    json!({"code":e.0.split_whitespace().next().unwrap_or("E7001"),"message":e.0})
}
fn respond_json(req: tiny_http::Request, result: Result<Value, Failure>) {
    match result {
        Ok(v) => respond(req, 200, v),
        Err(e) => respond(req, 409, error_json(&e)),
    }
}
fn respond(req: tiny_http::Request, status: u16, value: Value) {
    let _ = req.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
}

pub fn serve(top: PathBuf, port: u16) -> Out {
    if let Some(done) = migrate::recover_here(&top)? {
        return Ok(format!("{done}Run `solace serve` again from there.\n"));
    }
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| Failure(e.to_string(), 1))?;
    let port = server.server_addr().to_ip().map_or(port, |a| a.port());
    let board = std::sync::Arc::new(Board {
        top,
        held: Default::default(),
        editing: Default::default(),
        port,
        step: Default::default(),
        failed: Default::default(),
    });
    for (dir, root) in board.projects()? {
        board.claim(&dir, &root)?;
    }
    // A migration's folder is held too, so a shell can't move it from under the board.
    let _migration = match migrate::load(&board.top) {
        Some(_) if !board.held.lock().unwrap().contains_key(&board.top) => {
            let Some(f) = try_lock_file(&board.top.join(".solace/serve.lock"))? else {
                return fail(format!("{} is already served by another `solace serve`; stop that one first.", board.top.display()));
            };
            Some(f)
        }
        _ => None,
    };
    println!("Solace listening on http://{}", server.server_addr());
    if board.explicit() {
        println!("Solace root {}: {} projects", board.top.display(), board.projects()?.len());
    }
    std::io::stdout().flush()?;
    let watcher = board.clone();
    std::thread::spawn(move || {
        let mut previous: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
        let mut tick = 0u64;
        loop {
            tick += 1;
            std::thread::sleep(Duration::from_millis(100));
            if watcher.step.lock().unwrap().is_some() {
                continue;
            }
            for (_, root) in watcher.projects().unwrap_or_default() {
                // About once a second, read in code changed outside Solace (D-58).
                if tick % 10 == 1
                    && let Some(_guard) = try_lock(&root)
                    && let Ok(s) = open(&root)
                {
                    let _ = outside::absorb(&s, outside::Mode::Notice);
                }
                let current = fs::read(root.join("solace.toml")).ok();
                match previous.insert(root.clone(), current.clone()) {
                    Some(old) if old != current => {}
                    _ => continue,
                }
                if let Ok(_guard) = lock(&root)
                    && let Ok(s) = open(&root)
                    && let Ok(w) = lockdown::settings(&root).and_then(|v| workflow(&v))
                {
                    let _ = event(&s, json!({"workflow":w}));
                }
            }
        }
    });
    for mut req in server.incoming_requests() {
        let board = board.clone();
        std::thread::spawn(move || {
            let full = req.url().to_string();
            let path = full.split('?').next().unwrap_or("");
            // The root's own routes; `/events` merges every project's when it has several.
            let own = path == "/projects"
                || path == "/models"
                || path.starts_with("/projects/")
                || path == "/migration"
                || path.starts_with("/migration/")
                || (path == "/events" && board.explicit());
            if req.method().as_str() != "GET" && !path.starts_with("/migration") && board.step.lock().unwrap().is_some() {
                return respond_json(req, err("E7003", "a migration step is running"));
            }
            let routed = if own { Ok(None) } else { board.route(&full) };
            let (root, url) = match routed {
                Err(e) => return respond_json(req, Err(e)),
                Ok(Some((root, url))) => (Some(root), url),
                Ok(None) => (None, full.clone()),
            };
            if url.split('?').next() == Some("/events") {
                let last = req.headers().iter().find(|h| h.field.equiv("Last-Event-ID")).map(|h| h.value.as_str().to_string());
                let mut reader = Events::new(board.clone(), root.clone(), last.as_deref());
                let mut writer = req.into_writer();
                if writer
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                    )
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    return;
                }
                let mut buf = [0; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || writer.write_all(&buf[..n]).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
                return;
            }
            if req.method().as_str() == "GET"
                && matches!(url.split('?').next().unwrap_or(""), "/" | "/view" | "/app.js" | "/guide" | "/fonts/sans-400.woff2" | "/fonts/sans-700.woff2" | "/fonts/mono-400.woff2")
            {
                let (data, kind) = match url.split('?').next().unwrap_or("") {
                    "/" | "/view" => (include_bytes!("../board/index.html").as_slice(), "text/html"),
                    "/guide" => (include_bytes!("../board/guide.html").as_slice(), "text/html"),
                    "/fonts/sans-400.woff2" => (include_bytes!("../board/priv/fonts/ibm-plex-sans-latin-400-normal.woff2").as_slice(), "font/woff2"),
                    "/fonts/sans-700.woff2" => (include_bytes!("../board/priv/fonts/ibm-plex-sans-latin-700-normal.woff2").as_slice(), "font/woff2"),
                    "/fonts/mono-400.woff2" => (include_bytes!("../board/priv/fonts/ibm-plex-mono-latin-400-normal.woff2").as_slice(), "font/woff2"),
                    _ => (include_bytes!("../board/priv/app.js").as_slice(), "text/javascript"),
                };
                let _ = req.respond(
                    tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                );
                return;
            }
            let Some(root) = root else {
                let result = match read_body(&mut req, BODY_LIMIT) {
                    Some(bytes) => body_json(&bytes).and_then(|v| board.projects_api(req.method().as_str(), &url, &v)),
                    None => err("E7001", "request body too large"),
                };
                return respond_json(req, result);
            };
            let parts: Vec<String> = url.split('?').next().unwrap_or("").trim_matches('/').split('/').map(String::from).collect();
            let file = match parts.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["tasks", tid, "files", rest @ ..] => Some((tid.to_string(), rest.join("/"))),
                _ => None,
            };
            if let (Some((tid, name)), "GET") = (&file, req.method().as_str()) {
                if let Ok(data) = image_path(&root, tid, name).and_then(|p| Ok(fs::read(p)?)) {
                    let kind = image_type(name).unwrap();
                    let _ = req.respond(
                        tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                    );
                } else {
                    let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                }
                return;
            }
            let upload_to = file.filter(|(_, name)| name.is_empty() && req.method().as_str() == "POST").map(|(tid, _)| tid);
            let limit = if upload_to.is_some() { IMAGE_LIMIT } else { BODY_LIMIT };
            let result = match (read_body(&mut req, limit), upload_to) {
                (None, upload_to) => err("E7001", if upload_to.is_some() { "image over 5 MB" } else { "request body too large" }),
                (Some(bytes), Some(tid)) => upload(&root, &tid, &bytes),
                (Some(bytes), None) => body_json(&bytes).and_then(|v| api(&root, req.method().as_str(), &url, v)),
            };
            match result {
                Ok(v) => respond(req, 200, v),
                Err(e) => {
                    let mut v = error_json(&e);
                    // A refused action says which actions the task allows now.
                    if v["code"] == "E7001" && url.ends_with("/actions") {
                        let id = url.split('/').nth(2).unwrap_or("");
                        if let (Ok(t), Ok(w)) = (task(&root, id), flow(&root)) {
                            v["allowed"] = json!(allowed(&w, &t));
                        }
                    }
                    respond(req, 409, v)
                }
            }
        });
    }
    Ok(String::new())
}

/// The `/events` stream: one project's events, or in a workspace's merged
/// stream every project's, each naming its `project`, with a `{projects}`
/// event when the list changes. A merged stream's event id holds each
/// project's position as JSON (D-57).
struct Events {
    board: std::sync::Arc<Board>,
    merged: bool,
    /// `(dir, root, seq)`: each project's position.
    streams: Vec<(String, PathBuf, i64)>,
    /// Whether this connection has sent `ready`, the end of what it missed.
    caught_up: bool,
    bytes: std::io::Cursor<Vec<u8>>,
}
impl Events {
    fn new(board: std::sync::Arc<Board>, root: Option<PathBuf>, last: Option<&str>) -> Events {
        let (merged, streams) = match root {
            Some(root) => (false, vec![(String::new(), root, last.and_then(|l| l.parse().ok()).unwrap_or(0))]),
            None => {
                let seen: HashMap<String, i64> = last.and_then(|l| serde_json::from_str(l).ok()).unwrap_or_default();
                let streams = board.projects().unwrap_or_default().into_iter().map(|(d, r)| {
                    let seq = seen.get(&d).copied().unwrap_or(0);
                    (d, r, seq)
                });
                (true, streams.collect())
            }
        };
        Events { board, merged, streams, caught_up: false, bytes: Default::default() }
    }
    fn id(&self) -> String {
        json!(self.streams.iter().map(|(d, _, s)| (d.clone(), json!(s))).collect::<serde_json::Map<_, _>>()).to_string()
    }
    /// Follows a change to the workspace's list: new projects start at their
    /// latest event, since the board loads them whole.
    fn members_changed(&mut self) -> std::io::Result<bool> {
        let now = self.board.projects().map_err(|e| std::io::Error::other(e.0))?;
        if now.iter().map(|p| &p.0).eq(self.streams.iter().map(|s| &s.0)) {
            return Ok(false);
        }
        let old = std::mem::take(&mut self.streams);
        for (d, r) in now {
            let seq = match old.iter().find(|s| s.0 == d) {
                Some(s) => s.2,
                None => Connection::open(r.join(".solace/store.db"))
                    .and_then(|db| db.query_row("SELECT COALESCE(MAX(seq),0) FROM board_events", [], |row| row.get(0)))
                    .unwrap_or(0),
            };
            self.streams.push((d, r, seq));
        }
        Ok(true)
    }
}
impl Read for Events {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let n = self.bytes.read(out)?;
        if n > 0 {
            return Ok(n);
        }
        for _ in 0..10 {
            if self.merged && self.members_changed()? {
                let dirs: Vec<&String> = self.streams.iter().map(|s| &s.0).collect();
                self.bytes = std::io::Cursor::new(format!("id: {}\ndata: {}\n\n", self.id(), json!({"projects":dirs})).into_bytes());
                return self.bytes.read(out);
            }
            for i in 0..self.streams.len() {
                let (dir, root, seq) = &self.streams[i];
                // Opened on each poll, not kept: a kept connection would follow a store
                // that adopting or undoing a migration moved away.
                let db = Connection::open(root.join(".solace/store.db")).map_err(std::io::Error::other)?;
                let row = db
                    .query_row("SELECT seq,data FROM board_events WHERE seq>? ORDER BY seq LIMIT 1", [*seq], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .optional()
                    .map_err(std::io::Error::other)?;
                if let Some((seq, data)) = row {
                    let dir = dir.clone();
                    self.streams[i].2 = seq;
                    let text = if self.merged {
                        let mut v: Value = serde_json::from_str(&data).map_err(std::io::Error::other)?;
                        v["project"] = json!(dir);
                        format!("id: {}\ndata: {v}\n\n", self.id())
                    } else {
                        format!("id: {seq}\ndata: {data}\n\n")
                    };
                    self.bytes = std::io::Cursor::new(text.into_bytes());
                    return self.bytes.read(out);
                }
            }
            if !self.caught_up {
                self.caught_up = true;
                self.bytes = std::io::Cursor::new(b"event: ready\ndata: {}\n\n".to_vec());
                return self.bytes.read(out);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.bytes = std::io::Cursor::new(b": keepalive\n\n".to_vec());
        self.bytes.read(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task with one run `r1` in `run`'s state, and a hung agent in its own process group.
    fn supervised(run: Value) -> (testing::Project, String, std::process::Child) {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let s = open(&p.root).unwrap();
        let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Supervise"})).unwrap();
        t["runs"] = json!([run]);
        save(&s, &t).unwrap();
        let child = Command::new("sleep").arg("45").process_group(0).spawn().unwrap();
        (p, t["id"].as_str().unwrap().to_string(), child)
    }

    /// A finished agent that hangs is killed once the grace period passes, not before.
    #[test]
    fn hung_shutdown_is_killed_after_the_grace_period() {
        let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"finished"}));
        let log = p.root.join(".solace/r1.log");
        let start = Instant::now();
        let forced = supervise(&p.root, &tid, "r1", &mut child, &log, false, Duration::from_secs(3600), Duration::from_millis(300)).unwrap();
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
        assert!(!supervise(&p.root, &tid, "r1", &mut child, &log, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    /// An interrupted run's agent is killed at once; a running one past its timeout ends the run.
    #[test]
    fn interrupted_and_timed_out_agents_are_killed() {
        let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"ended","outcome":"interrupted"}));
        let log = p.root.join(".solace/r1.log");
        let start = Instant::now();
        assert!(supervise(&p.root, &tid, "r1", &mut child, &log, false, Duration::from_secs(3600), GRACEFUL_EXIT).unwrap());
        assert!(start.elapsed() < Duration::from_secs(10));
        let (p, tid, mut child) = supervised(json!({"id":"r1","kind":"work","state":"running"}));
        let log = p.root.join(".solace/r1.log");
        assert!(supervise(&p.root, &tid, "r1", &mut child, &log, false, Duration::from_millis(200), GRACEFUL_EXIT).unwrap());
        assert!(child.try_wait().unwrap().is_some());
        let t = task(&p.root, &tid).unwrap();
        assert_eq!(t["runs"][0]["state"], "ended");
        assert!(t["notes"].as_array().unwrap().iter().any(|n| n["text"] == "task timeout"));
    }

    /// A review or investigation run reads but doesn't change (it may build to
    /// verify); host commands never run in a run; an ended run runs nothing.
    #[test]
    fn agent_guard_follows_the_run() {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let s = open(&p.root).unwrap();
        let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Look"})).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"review","state":"running"},{"id":"r2","kind":"work","state":"ended"}]);
        save(&s, &t).unwrap();
        let tid = t["id"].as_str().unwrap();
        let guard = |rid: &str, a: &[&str]| agent_guard(&s.root, tid, rid, a, None).map(drop).map_err(|e| e.0);
        assert!(guard("r1", &["view", "app"]).is_ok() && guard("r1", &["build"]).is_ok());
        std::fs::write(
            s.root.join("solace.toml"),
            "[commands]\nsafe = { run = [\"true\"], allowed_in_reviews = true }\ne2e = { run = [\"true\"] }\n",
        )
        .unwrap();
        assert!(guard("r1", &["run", "safe"]).is_ok());
        assert!(guard("r1", &["run", "e2e"]).unwrap_err().contains("allowed_in_reviews"));
        for a in [["submit", "app.main"], ["write", "x"]] {
            assert!(guard("r1", &a).unwrap_err().contains("this run is read-only"), "{a:?}");
        }
        assert!(guard("r1", &["migrate", "--yes"]).unwrap_err().contains("host command"));
        assert!(guard("r2", &["view", "app"]).unwrap_err().contains("run ended"));
        assert!(guard("r9", &["view", "app"]).unwrap_err().starts_with("E7003 unknown run"));
    }

    /// A recorded file change stops being reviewable once the file is unwatched, and symbols always are.
    #[test]
    fn reviewable_follows_the_current_tier() {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("a.txt", "x")]);
        let s = open(&p.root).unwrap();
        assert!(reviewable(&s.root, "file:a.txt") && reviewable(&s.root, "symbol:1"));
        assert!(!reviewable(&s.root, "file:node_modules/x/index.js"));
        std::fs::write(s.root.join(".gitignore"), "a.txt\n").unwrap();
        assert!(!reviewable(&s.root, "file:a.txt"));
    }

    #[test]
    fn review_delta_keeps_module_headers_budget_and_completed_baseline() {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let s = open(&p.root).unwrap();
        let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Review headers"})).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        t["runs"] = json!([{"id":"work","kind":"work","state":"ended"}]);
        save(&s, &t).unwrap();
        let before = snapshot(&s).unwrap();
        s.db.execute("UPDATE modules SET header='//// Reviewed header' WHERE path='app'", []).unwrap();
        record(&s, &s, &tid, "work", &before, &snapshot(&s).unwrap()).unwrap();
        t = task(&s.root, &tid).unwrap();
        t["runs"].as_array_mut().unwrap().push(json!({"id":"review","kind":"review","state":"ended","outcome":"reviewed"}));
        let mut checkpoint = review_state(&s, &t).unwrap();
        checkpoint["run"] = json!("review");
        checkpoint["changed_during_review"] = json!(false);
        let path = review_path(&s.root, &tid, "review").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(&path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
        // A later interrupted run must not displace the completed review.
        t["runs"].as_array_mut().unwrap().push(json!({"id":"interrupted","kind":"review","state":"ended","outcome":"interrupted"}));
        save(&s, &t).unwrap();
        assert!(task_diff(&s, &tid, &["--since-review"], 1000).unwrap().contains("No task entity changes"));
        s.db.execute("UPDATE modules SET header='//// Repaired header' WHERE path='app'", []).unwrap();
        fs::write(s.root.join("later.txt"), "later").unwrap();
        record(&s, &s, &tid, "work", &json!({"file:later.txt":null}), &json!({"file:later.txt":b"later".to_vec()})).unwrap();
        let delta = task_diff(&s, &tid, &["--since-review"], 1).unwrap();
        assert!(delta.contains("Since completed review review"));
        assert!(delta.contains("- //// Reviewed header") && delta.contains("+ //// Repaired header"));
        assert!(delta.contains("Left out to fit the reply") && delta.contains("task diff --since-review <name>"));
        let filtered = task_diff(&s, &tid, &["--since-review", "file:later.txt"], 1000).unwrap();
        assert!(filtered.contains("file:later.txt (added)") && !filtered.contains("module:app"));
    }

    /// Canceling a task puts back what it changed: a module it created goes,
    /// with its file.
    #[test]
    fn cancel_removes_a_module_the_task_created() {
        let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let s = open(&p.root).unwrap();
        let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Shapes"})).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
        save(&s, &t).unwrap();
        let tid = t["id"].as_str().unwrap();
        let before = snapshot(&s).unwrap();
        s.submit_symbols(None, Some("geo"), "pub fn side() { 2 }").unwrap();
        record(&s, &s, tid, "r1", &before, &snapshot(&s).unwrap()).unwrap();
        assert!(s.root.join("src/geo.gleam").exists());
        let mut t = task(&s.root, tid).unwrap();
        end_runs(&mut t, "finished");
        save(&s, &t).unwrap();
        api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert!(!s.root.join("src/geo.gleam").exists());
        assert!(s.resolve_module(&s.index().unwrap(), "geo").is_none());
        assert_eq!(snapshot(&s).unwrap(), before);
    }

    #[test]
    fn note_images_are_checked_numbered_and_encoded() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(image_kind(b"<svg onload=alert(1)>"), None);
        assert_eq!(image_type("0123456789abcdef.png"), Some("image/png"));
        for bad in ["../x.png", "0123456789abcdef.svg", "x.png"] {
            assert_eq!(image_type(bad), None);
        }
        let a = "/tasks/t1/files/0123456789abcdef.png";
        let mut found = vec![];
        let text = note_images(&format!("see ![one]({a}) and ![x](http://e/x.png) ![again]({a}) ![broken"), "/tasks/t1/files/", &mut found);
        assert_eq!(text, "see [image 1] and ![x](http://e/x.png) [image 1] ![broken");
        assert_eq!(found, ["0123456789abcdef.png"]);
    }

    #[test]
    fn tasks_saved_in_done_load_as_complete() {
        let root = testing::Scratch::new().with(&[(".solace/board/t1.json", r#"{"id":"t1","column":"done"}"#)]);
        assert_eq!(task(&root, "t1").unwrap()["column"], "complete");
        // A custom workflow that still has a `done` column keeps it.
        fs::write(root.join("solace.toml"), include_str!("default-board.toml").replace("= \"complete\"", "= \"done\"")).unwrap();
        assert_eq!(tasks(&root).unwrap()[0]["column"], "done");
    }
    #[test]
    fn presets_merge_with_configured_models() {
        let v = parse_system("").unwrap();
        let ids: Vec<&str> = v["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"codex-gpt-6-astra") && ids.contains(&"claude-opus-5-5"));
        assert_eq!(v["task_timeout"], 3600);
        let v = parse_system(
            "[[models]]\nid = \"claude-opus-5-5\"\nlabel = \"Mine\"\nagent = \"claude\"\nmodel = \"x\"\n\
             [[models]]\nid = \"local\"\nlabel = \"Local\"\nagent = \"codex\"\nmodel = \"qwen\"\n\
             endpoint = { kind = \"openai\", base_url = \"http://localhost:11434/v1\", key_env = \"LOCAL_KEY\" }\n",
        )
        .unwrap();
        let models = v["models"].as_array().unwrap();
        assert_eq!(models.iter().find(|m| m["id"] == "claude-opus-5-5").unwrap()["label"], "Mine");
        assert_eq!(models.last().unwrap()["id"], "local");
        assert!(parse_system("presets = false\n").unwrap()["models"].as_array().unwrap().is_empty());
    }

    #[test]
    fn endpoints_are_checked_and_passed_to_codex() {
        // An endpoint's kind must match its agent.
        let wrong = "[[models]]\nid = \"x\"\nlabel = \"X\"\nagent = \"claude\"\nmodel = \"m\"\nendpoint = { kind = \"openai\", base_url = \"http://h\" }\n";
        assert!(parse_system(wrong).unwrap_err().0.starts_with("E6020"));
        let m =
            json!({"label":"Local","endpoint":{"kind":"openai","base_url":"http://localhost:11434/v1","key_env":"SOLACE_TEST_UNSET_KEY"}});
        let args = endpoint_args(&m);
        assert_eq!(args[1], "model_provider=\"solace_endpoint\"");
        let provider: toml::Value = format!("p = {}", args[3].split_once('=').unwrap().1).parse().unwrap();
        assert_eq!(provider["p"]["base_url"].as_str(), Some("http://localhost:11434/v1"));
        assert_eq!(provider["p"]["env_key"].as_str(), Some("SOLACE_TEST_UNSET_KEY"));
        assert_eq!(provider["p"]["wire_api"].as_str(), Some("responses"));
        assert!(endpoint_args(&json!({"label":"No endpoint"})).is_empty());
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
        assert_eq!(endpoint_context(&base, "configured-name"), Some(131072));
        assert_eq!(endpoint_context(&base, "b"), Some(2));
        assert_eq!(endpoint_context(&base, "c"), None);
        t.join().unwrap();
        assert_eq!(endpoint_context("https://example.com", "m"), None);
    }

    #[test]
    fn transcripts_read_codex_and_claude_logs() {
        let dir = testing::Scratch::new();
        let log = dir.join("log.jsonl");
        let codex = [
            json!({"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view a"},"status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view a","text":"x"},"result":{"content":[{"type":"text","text":"pub fn a()"}]},"status":"completed"}}),
            json!({"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"All **done**."}}),
            json!({"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}),
        ];
        let text = codex.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n") + "\nnot json\nstill not\n";
        fs::write(&log, text).unwrap();
        let t = transcript(&log);
        let kinds: Vec<&str> = t.iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["tool", "message", "usage", "output"]);
        assert_eq!(
            (t[0]["title"].as_str(), t[0]["status"].as_str(), t[0]["output"].as_str()),
            (Some("solace view a"), Some("ok"), Some("pub fn a()"))
        );
        assert_eq!(t[0]["input"], "text:\nx");
        assert_eq!(t[3]["text"], "not json\nstill not");
        assert_eq!(last_words(&t).unwrap(), "All **done**.");
        let claude = [
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"u1","name":"mcp__solace__solace","input":{"command":"test"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u1","content":[{"type":"text","text":"E1 failed"}],"is_error":true}]}}),
        ];
        fs::write(&log, claude.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        let t = transcript(&log);
        assert_eq!(
            (t[1]["title"].as_str(), t[1]["status"].as_str(), t[1]["output"].as_str()),
            (Some("solace test"), Some("failed"), Some("E1 failed"))
        );
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
        assert!(usage[0]["text"].as_str().unwrap().ends_with("; last context 300 tokens"));
        assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
    }

    #[test]
    fn briefs_hold_what_the_agent_needs_as_text() {
        let t = json!({"id":"t1","title":"Add coolcalc","body":"A CLI calculator.","rank":3,"stamps":{},
            "links":[{"name":"spec","url":"http://x"}],"notes":[{"author":"human","text":"Keep it small","time":1,"run":null}],
            "questions":[{"id":"q","text":"Syntax?","options":["a","b"],"answer":"b"}],"runs":[{"id":"r1"}],"changes":[{"entity":"symbol:x","name":"calc.add","run":"r1"}]});
        let b = brief(&t, "work");
        for part in [
            "Make the change",
            "`task done`, a summary in `text`",
            "Task: Add coolcalc\n\nA CLI calculator.\n",
            "- spec: http://x",
            "- human: Keep it small",
            "- Syntax?\n  Answer: b",
        ] {
            assert!(b.contains(part), "{part} missing from:\n{b}");
        }
        for noise in ["{", "r1", "rank"] {
            assert!(!b.contains(noise), "{noise} in:\n{b}");
        }
        for kind in ["work", "investigate", "review"] {
            let prompt = brief(&t, kind);
            for guidance in [
                "Write human-facing task messages in Markdown",
                "including the text sent to task tools",
                "relevant sections with short headings",
                "blank lines around headings and lists",
                "explain what you need and why it matters",
            ] {
                assert!(prompt.contains(guidance), "{kind}: missing {guidance}");
            }
            assert_eq!(prompt.contains("**Assessment: Looks good**"), kind == "review");
            assert_eq!(prompt.contains("**Assessment: Needs work**"), kind == "review");
        }
        assert!(brief(&t, "investigate").contains("Investigate this task"));
        // A discussion keeps every note whole: no command reads a cut note's full text.
        let mut talk = t.clone();
        talk["notes"] = json!([
            {"author":"agent","text":"### Summary\n\nPlan A.\n\n### Details\n\nLong A.","run":null},
            {"author":"human","text":"### Mine\n\nKeep.\n\n### All\n\nOf it.","run":null},
            {"author":"agent","text":"### Summary\n\nPlan B.\n\n### Details\n\nLong B.","run":null}
        ]);
        let d = brief(&talk, "discuss");
        assert!(d.contains("Long A.") && d.contains("Keep.\n\n### All\n\nOf it.") && d.contains("Long B."), "{d}");
        let r = brief(&t, "review");
        assert!(
            r.contains("Task changes:\n- calc.add")
                && r.contains("`task note`, the feedback in `text`")
                && r.contains("`run` are refused")
                && !r.contains("task done")
        );
        assert!(!b.contains("Projects:"), "one project needs no Projects section");
        // A task across projects (D-61) says where the agent works and how it reaches the rest.
        let mut wide = t.clone();
        wide["scope"] = json!(["a", "web"]);
        wide["home"] = json!("a");
        wide["changes"][0]["project"] = json!("web");
        let w = brief(&wide, "work");
        assert!(w.contains("covers the projects a, web. You work in a; for another of them, add `--project <folder>`"), "{w}");
        assert!(w.contains("Task changes:\n- web: calc.add"), "{w}");
    }

    #[test]
    fn unavailable_models_say_why() {
        let missing = json!({"agent":"codex","run":["solace-no-such-program"]});
        assert_eq!(unavailable(&missing).unwrap(), "solace-no-such-program is not installed");
        assert!(unavailable(&json!({"agent":"codex","run":["sh"]})).is_none());
        let agent = json!({"agent":"solace-no-such-agent","model":"m"});
        assert!(unavailable(&agent).unwrap().contains("not installed"));
    }
}
