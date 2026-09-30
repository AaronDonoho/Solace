use super::*;
use serde_json::{Value, json};
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

pub fn lock(root: &Path) -> Result<fs::File, Failure> {
    fs::create_dir_all(root.join(".solace"))?;
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join(".solace/lock"))?;
    // ponytail: one project lock; use per-change store isolation if parallel work is introduced.
    f.lock()?;
    Ok(f)
}
pub fn open(root: &Path) -> Result<Store, Failure> {
    let db = Connection::open(root.join(".solace/store.db"))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS board_events(seq INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS board_before(task TEXT, entity TEXT, data TEXT NOT NULL, PRIMARY KEY(task,entity));
        CREATE TABLE IF NOT EXISTS board_touches(seq INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, entity TEXT);",
    )?;
    fs::create_dir_all(root.join(".solace/board"))?;
    Ok(Store { root: root.into(), db })
}
fn err<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "task", message)
}
fn id() -> String {
    new_id(Kind::Fn, "board", "").replace("fn:", "")
}
fn task_path(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return err("E7001", "invalid task id");
    }
    Ok(root.join(".solace/board").join(format!("{id}.json")))
}
fn task(root: &Path, id: &str) -> Result<Value, Failure> {
    serde_json::from_slice(&fs::read(task_path(root, id)?)?).map_err(|e| Failure(e.to_string(), 1))
}
fn tasks(root: &Path) -> Result<Vec<Value>, Failure> {
    let mut out = vec![];
    for entry in fs::read_dir(root.join(".solace/board"))? {
        let p = entry?.path();
        if p.extension().is_some_and(|x| x == "json") {
            out.push(serde_json::from_slice(&fs::read(p)?).map_err(|e| Failure(e.to_string(), 1))?);
        }
    }
    out.sort_by(|a: &Value, b: &Value| a["rank"].as_f64().unwrap_or(0.).total_cmp(&b["rank"].as_f64().unwrap_or(0.)));
    Ok(out)
}
fn event(s: &Store, v: Value) -> Result<(), Failure> {
    s.db.execute("INSERT INTO board_events(data) VALUES (?)", [v.to_string()])?;
    Ok(())
}
fn save(s: &Store, t: &Value) -> Result<(), Failure> {
    let p = task_path(&s.root, t["id"].as_str().unwrap())?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(t).unwrap())?;
    fs::rename(tmp, p)?;
    event(s, json!({"task":t}))
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
        if let Some(next) = c.get("next") {
            if !ids.contains(next.as_str().unwrap_or("")) {
                return Err(invalid("unknown gate next"));
            }
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
fn column<'a>(w: &'a Value, t: &Value) -> &'a Value {
    w["columns"].as_array().unwrap().iter().find(|c| c["id"] == t["column"]).unwrap_or(&Value::Null)
}
fn allowed(w: &Value, t: &Value) -> Vec<String> {
    let c = column(w, t);
    if c["final"] == true {
        return vec![];
    }
    let mut out = c["actions"].as_array().unwrap_or(&vec![]).iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>();
    out.extend(["pause".into(), "resume".into()]);
    out
}
fn active(t: &Value) -> bool {
    t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended")
}
fn note(t: &mut Value, author: &str, run: Option<&str>, text: &str) {
    t["notes"].as_array_mut().unwrap().push(json!({"author":author,"run":run,"time":now(),"text":text}));
}
/// Marks a task as needing a human. The next action a human takes on it clears the mark.
fn flag(t: &mut Value, reason: &str) {
    t["attention"] = json!({"reason":reason,"time":now()});
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
fn end_runs(t: &mut Value, outcome: &str) {
    for r in t["runs"].as_array_mut().unwrap() {
        if r["state"] != "ended" {
            r["state"] = json!("ended");
            r["ended"] = json!(now());
            r["outcome"] = json!(outcome);
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
                let kind_matches = matches!((e["kind"].as_str(), m["agent"].as_str()), (Some("openai"), Some("codex")) | (Some("anthropic"), Some("claude")));
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
    let Some(e) = m.get("endpoint") else { return vec![] };
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
pub fn record(s: &Store, tid: &str, rid: &str, before: &Value, after: &Value) -> Result<(), Failure> {
    let mut t = task(&s.root, tid)?;
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
                let dir = s.root.join(".solace/board").join(tid).join("before");
                fs::create_dir_all(&dir)?;
                let file = dir.join(hex(&Sha256::digest(name.as_bytes())));
                if let Some(bytes) = data.as_array() {
                    fs::write(file, bytes.iter().map(|b| b.as_u64().unwrap() as u8).collect::<Vec<_>>())?;
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
        if !t["changes"].as_array().unwrap().iter().any(|x| x["entity"] == *key) {
            t["changes"].as_array_mut().unwrap().push(json!({"entity":key,"name":name,"run":rid}));
        }
    }
    save(s, &t)
}
pub fn file_snapshot(root: &Path, a: &[&str]) -> Result<Value, Failure> {
    let mut v = json!({});
    if matches!(a[0], "write" | "remove")
        && let Some(name) = a.get(1)
    {
        let p = lockdown::path(root, name)?;
        let name = p
            .strip_prefix(root)
            .unwrap()
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        v[format!("file:{name}")] = json!(fs::read(p).ok());
    }
    if a[0] == "run" {
        fn collect(root: &Path, dir: &Path, settings: &toml::Value, out: &mut Value) -> Result<(), Failure> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let p = entry.path();
                let name = p.strip_prefix(root).unwrap().to_string_lossy();
                if entry.file_type()?.is_symlink() || [".solace", ".git", "build"].contains(&name.as_ref()) {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    collect(root, &p, settings, out)?;
                } else if matches!(lockdown::tier(root, settings, &name, None).chars().next(), Some('2' | '3')) {
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
                fs::write(p, bytes.iter().map(|b| b.as_u64().unwrap() as u8).collect::<Vec<_>>())?;
            } else if p.exists() {
                fs::remove_file(p)?;
            }
        }
    }
    s.materialize(&s.index()?)?;
    Ok(())
}
fn revert(s: &Store, t: &mut Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let mut st = s.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
    let mut states = vec![];
    for row in st.query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, v) = row?;
        states.push((k, serde_json::from_str::<Value>(&v).unwrap()));
    }
    let mut conflicts = vec![];
    for (key, _) in &states {
        let later:Option<String>=s.db.query_row("SELECT task FROM board_touches WHERE entity=?1 AND seq > (SELECT MIN(seq) FROM board_touches WHERE task=?2 AND entity=?1) AND task!=?2 ORDER BY seq DESC LIMIT 1",params![key,tid],|r|r.get(0)).optional()?;
        if let Some(other) = later {
            let name = t["changes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["entity"] == *key)
                .map(|c| c["name"].as_str().unwrap_or(key))
                .unwrap_or(key);
            conflicts.push(format!("{name}: task {other}"));
        }
    }
    if !conflicts.is_empty() {
        return err("E7007", conflicts.join(", "));
    }
    if !states.is_empty() {
        let current = snapshot(s)?;
        let mut undo = vec![];
        for (key, _) in &states {
            let v = if let Some(name) = key.strip_prefix("file:") {
                json!(fs::read(lockdown::path(&s.root, name)?).ok())
            } else {
                current[key].clone()
            };
            undo.push((key.clone(), v));
        }
        s.db.execute_batch("SAVEPOINT revert_task")?;
        let checked = (|| -> Result<(), Failure> {
            restore(s, &states)?;
            s.check()?;
            Ok(())
        })();
        if let Err(e) = checked {
            s.db.execute_batch("ROLLBACK TO revert_task; RELEASE revert_task")?;
            restore(s, &undo)?;
            let other = tasks(&s.root)?
                .iter()
                .filter(|x| x["id"] != tid)
                .map(|x| format!("task {}: {}", x["id"], x["changes"]))
                .collect::<Vec<_>>()
                .join("\n");
            return err("E7008", format!("{}\n{other}", e.0));
        }
        s.db.execute_batch("RELEASE revert_task")?;
    }
    s.db.execute("DELETE FROM board_touches WHERE task=?", [tid])?;
    drop_before(s, tid)?;
    t["changes"] = json!([]);
    end_runs(t, "reverted");
    Ok(())
}
fn drop_before(s: &Store, tid: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM board_before WHERE task=?", [tid])?;
    let p = s.root.join(".solace/board").join(tid).join("before");
    if p.exists() {
        fs::remove_dir_all(p)?;
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
        return Ok(None);
    }
    let a = &w["actions"][name];
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
    if matches!(run, "work" | "new_work")
        && tasks(&s.root)?
            .iter()
            .any(|x| x["id"] != t["id"] && x["runs"].as_array().unwrap().iter().any(|r| r["kind"] == "work" && r["state"] != "ended"))
    {
        return err("E7005", "another task has a work run");
    }
    let config = system()?;
    let chosen = body.get("model").unwrap_or(&t["model"]);
    if a["model"] == true && !resume && body["model"].as_str().is_none() {
        return err("E7001", "this action requires a model");
    }
    let selected = if !run.is_empty() && !resume { Some(model(&config, chosen.as_str().unwrap_or(""))?) } else { None };
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
    if let Some(text) = body["text"].as_str() {
        note(t, "human", None, text);
        for q in t["questions"].as_array_mut().unwrap() {
            if q["answer"].is_null() {
                q["answer"] = json!(text);
            }
        }
    }
    if a.get("to").is_some() {
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
        let rid = id();
        let kind = if run == "new_work" { "work" } else { run };
        let log = s.root.join(".solace/board").join(t["id"].as_str().unwrap()).join(format!("{rid}.jsonl"));
        t["runs"].as_array_mut().unwrap().push(json!({"id":rid,"kind":kind,"model":m["id"],"state":"running","started":now(),"log":log}));
        if kind == "work" {
            t["model"] = m["id"].clone();
        }
        return Ok(Some((rid, m)));
    }
    if column(w, t)["final"] == true {
        drop_before(s, t["id"].as_str().unwrap())?;
    }
    Ok(None)
}

pub fn api(root: &Path, method: &str, url: &str, body: Value) -> Result<Value, Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let w = workflow(&lockdown::settings(root)?)?;
    let parts: Vec<_> = url.trim_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["workflow"]) => Ok(w),
        ("GET", ["models"]) => Ok(json!(
            system()?["models"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|m| {
                    let reason = unavailable(m);
                    json!({"id":m["id"],"label":m["label"],"agent":m["agent"],"available":reason.is_none(),"reason":reason})
                })
                .collect::<Vec<_>>()
        )),
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
            let rank = tasks(root)?.iter().filter_map(|t| t["rank"].as_f64()).fold(0., f64::max) + 1.;
            let t = json!({"id":id(),"title":title,"body":body["body"].as_str().unwrap_or(""),"links":body.get("links").cloned().unwrap_or(json!([])),"column":w["columns"][0]["id"],"rank":rank,"paused_from":null,"attention":null,"stamps":{},"model":null,"notes":[],"questions":[],"runs":[],"changes":[]});
            save(&s, &t)?;
            Ok(t)
        }
        (_, ["tasks", tid, rest @ ..]) => {
            let mut t = task(root, tid)?;
            match (method, rest) {
                ("GET", []) => return Ok(t),
                ("GET", ["runs", rid]) => {
                    let mut run = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == *rid).cloned().ok_or_else(|| Failure("E7001 unknown run".into(), 1))?;
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
                ("POST", ["notes"]) => {
                    let text = body["text"].as_str().ok_or_else(|| Failure("E7001 text required".into(), 1))?;
                    note(&mut t, "human", None, text);
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
                }
                ("POST", ["actions"]) => {
                    let launch = action(&s, &w, &mut t, &body)?;
                    t["attention"] = Value::Null;
                    save(&s, &t)?;
                    if let Some((rid, m)) = launch {
                        start(root.to_path_buf(), t.clone(), rid, m);
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
        if let Err(e) = agent_process(&root, &t, &rid, &m) {
            let _ = finish(&root, t["id"].as_str().unwrap(), &rid, Some(&e.0));
        }
    });
}
fn agent_process(root: &Path, t: &Value, rid: &str, m: &Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let run = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).unwrap();
    let log = PathBuf::from(run["log"].as_str().unwrap());
    fs::create_dir_all(log.parent().unwrap())?;
    let dir = log.parent().unwrap().join(rid);
    let binary = env::current_exe()?;
    let config = system()?;
    let timeout = config["task_timeout"].as_u64().unwrap_or(3600);
    let agent = m["agent"].as_str().unwrap_or("codex");
    let launch: Value = serde_json::from_str(&lockdown::agent_config(agent, &dir, &binary, timeout)?).unwrap();
    let custom = m["run"].as_array();
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
        argv.extend(launch["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()));
        if agent == "codex" {
            if let Some(e) = m["effort"].as_str() {
                argv.extend(["-c".into(), format!("model_reasoning_effort={}", json!(e))]);
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
        if auth.exists() {
            std::os::unix::fs::symlink(auth, dir.join("auth.json"))?;
        }
    }
    let file = fs::File::create(&log)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(root).process_group(0).stdin(Stdio::piped()).stdout(file.try_clone()?).stderr(file);
    for (k, v) in launch["env"].as_object().unwrap() {
        cmd.env(k, v.as_str().unwrap());
    }
    if agent == "claude" && custom.is_none() {
        if let Some(e) = m.get("endpoint") {
            cmd.env("ANTHROPIC_BASE_URL", e["base_url"].as_str().unwrap_or(""));
            // The key is read from the named variable at launch and never stored.
            if let Some(key) = e["key_env"].as_str().and_then(|k| env::var(k).ok()) {
                cmd.env("ANTHROPIC_AUTH_TOKEN", key);
            }
        }
    }
    cmd.env("SOLACE_TASK", tid).env("SOLACE_RUN", rid).env("SOLACE_SESSION", rid).env("SOLACE_BIN", &binary);
    let mut child = cmd.spawn()?;
    writeln!(child.stdin.take().unwrap(), "{}", brief(t, run["kind"].as_str().unwrap_or("")))?;
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let current = task(root, tid)?;
        let r = current["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).unwrap();
        if r["state"] == "running" && current["paused_from"].is_null() {
            elapsed += last.elapsed();
        }
        last = Instant::now();
        let expired = elapsed >= Duration::from_secs(timeout);
        if r["state"] == "ended" || expired {
            let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
            let _ = child.kill();
            let _ = child.wait();
            if expired {
                finish(root, tid, rid, Some("task timeout"))?;
            }
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            let message = match last_words(&transcript(&log)) {
                Some(said) => format!("agent exited without finishing ({status}). Its last message:\n\n{said}"),
                None => format!("agent exited without finishing ({status})"),
            };
            finish(root, tid, rid, Some(&message))?;
            return Ok(());
        }
    }
}
/// An agent's prompt: what its run is for, how to start, ask and finish, then
/// what it needs from the task (Spec 15 §6), as plain text.
fn brief(t: &Value, kind: &str) -> String {
    let start = "Start with `session start --task`, with the task's title and description as `text`.";
    let ask = "If you need a decision from the human, ask with `task ask` (`text` and 2–4 `options`); the call waits for the answer.";
    let read_only = "This run is read-only: reading, `check`, `test` and `build` work; `submit`, `delete`, `write`, `remove`, `format` and `run` are refused.";
    let role = match kind {
        "review" => format!("Review the changes this task made (listed below) and give feedback. {read_only} Finish with `task note`, the feedback in `text`."),
        "work" => format!("Make the change this task describes. {start} {ask} Finish with `task done`, a summary in `text`: what you changed and how you checked it."),
        _ => format!("Investigate this task before any work on it: answer what it asks, or find out what the change involves and plan it. {read_only} {start} {ask} Finish with `task done`, the report in `text`: your findings, or the plan for whoever does the work."),
    };
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let mut out = format!(
        "You are working on a task from this project's task board. You work only through the Solace tool: there is no shell and no file access.\n{role}\n\nTask: {}\n",
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
    section("Notes so far", list("notes").iter().map(|n| format!("- {}: {}", text(&n["author"]), text(&n["text"]))).collect());
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
    if kind == "review" {
        section("Changes to review", list("changes").iter().map(|c| format!("- {}", text(&c["name"]))).collect());
    }
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
                let u = &e["usage"];
                out.push(json!({"kind":"usage","text":format!("{} input tokens ({} cached), {} output tokens", u["input_tokens"], u["cache_read_input_tokens"], u["output_tokens"])}));
                if e["is_error"] == true {
                    out.push(json!({"kind":"error","text":text_of(&e["result"])}));
                }
            }
            _ => {}
        }
    }
    out
}
fn finish(root: &Path, tid: &str, rid: &str, failure: Option<&str>) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let mut t = task(root, tid)?;
    let w = workflow(&lockdown::settings(root)?)?;
    let r = t["runs"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == rid).unwrap();
    if r["state"] == "ended" {
        return Ok(());
    }
    r["state"] = json!("ended");
    r["ended"] = json!(now());
    r["outcome"] = json!(failure.unwrap_or("finished"));
    if let Some(message) = failure {
        note(&mut t, "solace", Some(rid), message);
        flag(&mut t, message);
        move_to(&w, &mut t, &w["events"]["failed"])?;
    }
    save(&s, &t)
}

pub fn context() -> Option<(String, String)> {
    Some((env::var("SOLACE_TASK").ok()?, env::var("SOLACE_RUN").ok()?))
}
pub fn agent_guard(root: &Path, tid: &str, rid: &str, a: &[&str]) -> Result<fs::File, Failure> {
    loop {
        let guard = lock(root)?;
        let t = task(root, tid)?;
        let r = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).ok_or_else(|| Failure("E7003 unknown run".into(), 1))?;
        if r["state"] == "ended" {
            return err("E7003", "run ended");
        }
        if !t["paused_from"].is_null() {
            drop(guard);
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if r["kind"] != "work" && matches!(a[0], "write" | "submit" | "remove" | "run" | "delete" | "import" | "format" | "agent-config") {
            return err("E7010", "this run is read-only");
        }
        if matches!(a[0], "agent-config" | "serve" | "import") {
            return err("E7010", "host command is unavailable in a run");
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
    guard: fs::File,
) -> Out {
    let mut t = task(&s.root, tid)?;
    let w = workflow(&lockdown::settings(&s.root)?)?;
    let i = t["runs"].as_array().unwrap().iter().position(|r| r["id"] == rid).unwrap();
    let text = if a.len() > 2 { a[2..].join(" ") } else { input()? };
    match a.get(1).copied() {
        Some("note") => {
            note(&mut t, "agent", Some(rid), &text);
            if t["runs"][i]["kind"] == "review" {
                t["runs"][i]["state"] = json!("ended");
                t["runs"][i]["ended"] = json!(now());
                t["runs"][i]["outcome"] = json!("reviewed");
            }
        }
        Some("done") => {
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "reviews finish with task note");
            }
            note(&mut t, "agent", Some(rid), &text);
            t["runs"][i]["state"] = json!("ended");
            t["runs"][i]["ended"] = json!(now());
            if t["runs"][i]["kind"] == "investigate" {
                // The report is the result; the task stays in planning.
                t["runs"][i]["outcome"] = json!("reported");
            } else {
                t["runs"][i]["outcome"] = json!("finished");
                move_to(&w, &mut t, &w["events"]["finished"])?;
            }
        }
        Some("ask") => {
            if !(2..=4).contains(&options.len()) {
                return err("E7001", "task ask needs 2–4 options");
            }
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "review cannot move task with a question");
            }
            let qid = id();
            t["questions"].as_array_mut().unwrap().push(json!({"id":qid,"text":text,"options":options}));
            t["runs"][i]["state"] = json!("waiting");
            flag(&mut t, "an agent asked a question");
            // An investigation waits in planning; `investigate` continues it.
            if t["runs"][i]["kind"] == "work" {
                move_to(&w, &mut t, &w["events"]["asked"])?;
            }
            save(s, &t)?;
            drop(guard);
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
        _ => return usage("task note | task ask | task done, with the text in `text`"),
    }
    if column(&w, &t)["final"] == true {
        drop_before(s, tid)?;
    }
    save(s, &t)?;
    Ok("Recorded.\n".into())
}

pub fn serve(root: &Path, port: u16) -> Out {
    {
        let _guard = lock(root)?;
        let s = open(root)?;
        let w = workflow(&lockdown::settings(root)?)?;
        lockdown::recover(root)?;
        event(&s, json!({"workflow":w}))?;
        for mut t in tasks(root)? {
            if active(&t) {
                end_runs(&mut t, "server restarted");
                flag(&mut t, "the Solace server restarted while an agent was working");
                move_to(&w, &mut t, &w["events"]["failed"])?;
                save(&s, &t)?;
            }
        }
    }
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| Failure(e.to_string(), 1))?;
    println!("Solace listening on http://{}", server.server_addr());
    std::io::stdout().flush()?;
    let watcher_root = root.to_path_buf();
    std::thread::spawn(move || {
        let mut previous = fs::read(watcher_root.join("solace.toml")).ok();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let current = fs::read(watcher_root.join("solace.toml")).ok();
            if current != previous {
                previous = current;
                if let Ok(_guard) = lock(&watcher_root) {
                    if let Ok(s) = open(&watcher_root) {
                        if let Ok(w) = lockdown::settings(&watcher_root).and_then(|v| workflow(&v)) {
                            let _ = event(&s, json!({"workflow":w}));
                        }
                    }
                }
            }
        }
    });
    for mut req in server.incoming_requests() {
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let url = req.url().to_string();
            if url == "/events" {
                let seq =
                    req.headers().iter().find(|h| h.field.equiv("Last-Event-ID")).and_then(|h| h.value.as_str().parse().ok()).unwrap_or(0);
                let mut reader = Events { root, seq, bytes: std::io::Cursor::new(vec![]) };
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
            if req.method().as_str() == "GET" && matches!(url.as_str(), "/" | "/app.js" | "/guide") {
                let (data, kind) = match url.as_str() {
                    "/" => (include_bytes!("../board/index.html").as_slice(), "text/html"),
                    "/guide" => (include_bytes!("../board/guide.html").as_slice(), "text/html"),
                    _ => (include_bytes!("../board/priv/app.js").as_slice(), "text/javascript"),
                };
                let _ = req.respond(
                    tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                );
                return;
            }
            let mut bytes = Vec::new();
            let parsed = req.as_reader().take(1_048_577).read_to_end(&mut bytes);
            let result = if parsed.is_err() || bytes.len() > 1_048_576 {
                err("E7001", "request body too large")
            } else {
                let body = if bytes.is_empty() { Ok(json!({})) } else { serde_json::from_slice(&bytes) };
                match body {
                    Ok(v) => api(&root, req.method().as_str(), &url, v),
                    Err(_) => err("E7001", "invalid JSON"),
                }
            };
            let (status, value) = match result {
                Ok(v) => (200, v),
                Err(e) => {
                    let code = e.0.split_whitespace().next().unwrap_or("E7001");
                    let mut v = json!({"code":code,"message":e.0});
                    if code == "E7001" && url.ends_with("/actions") {
                        let id = url.split('/').nth(2).unwrap_or("");
                        if let (Ok(t), Ok(w)) = (task(&root, id), lockdown::settings(&root).and_then(|s| workflow(&s))) {
                            v["allowed"] = json!(allowed(&w, &t));
                        }
                    }
                    (409, v)
                }
            };
            let _ = req.respond(
                tiny_http::Response::from_string(value.to_string())
                    .with_status_code(status)
                    .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
            );
        });
    }
    Ok(String::new())
}
struct Events {
    root: PathBuf,
    seq: i64,
    bytes: std::io::Cursor<Vec<u8>>,
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
            let db = Connection::open(self.root.join(".solace/store.db")).map_err(std::io::Error::other)?;
            let row = db
                .query_row("SELECT seq,data FROM board_events WHERE seq>? ORDER BY seq LIMIT 1", [self.seq], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .map_err(std::io::Error::other)?;
            if let Some((seq, data)) = row {
                self.seq = seq;
                self.bytes = std::io::Cursor::new(format!("id: {seq}\ndata: {data}\n\n").into_bytes());
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
        let m = json!({"label":"Local","endpoint":{"kind":"openai","base_url":"http://localhost:11434/v1","key_env":"SOLACE_TEST_UNSET_KEY"}});
        let args = endpoint_args(&m);
        assert_eq!(args[1], "model_provider=\"solace_endpoint\"");
        let provider: toml::Value = format!("p = {}", args[3].split_once('=').unwrap().1).parse().unwrap();
        assert_eq!(provider["p"]["base_url"].as_str(), Some("http://localhost:11434/v1"));
        assert_eq!(provider["p"]["env_key"].as_str(), Some("SOLACE_TEST_UNSET_KEY"));
        assert_eq!(provider["p"]["wire_api"].as_str(), Some("responses"));
        assert!(endpoint_args(&json!({"label":"No endpoint"})).is_empty());
    }

    #[test]
    fn transcripts_read_codex_and_claude_logs() {
        let log = env::temp_dir().join(format!("{}.jsonl", id()));
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
        assert_eq!((t[0]["title"].as_str(), t[0]["status"].as_str(), t[0]["output"].as_str()), (Some("solace view a"), Some("ok"), Some("pub fn a()")));
        assert_eq!(t[0]["input"], "text:\nx");
        assert_eq!(t[3]["text"], "not json\nstill not");
        assert_eq!(last_words(&t).unwrap(), "All **done**.");
        let claude = [
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"u1","name":"mcp__solace__solace","input":{"command":"test"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u1","content":[{"type":"text","text":"E1 failed"}],"is_error":true}]}}),
        ];
        fs::write(&log, claude.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        let t = transcript(&log);
        assert_eq!((t[1]["title"].as_str(), t[1]["status"].as_str(), t[1]["output"].as_str()), (Some("solace test"), Some("failed"), Some("E1 failed")));
        assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
        fs::remove_file(log).unwrap();
    }

    #[test]
    fn briefs_hold_what_the_agent_needs_as_text() {
        let t = json!({"id":"t1","title":"Add coolcalc","body":"A CLI calculator.","rank":3,"stamps":{},
            "links":[{"name":"spec","url":"http://x"}],"notes":[{"author":"human","text":"Keep it small","time":1,"run":null}],
            "questions":[{"id":"q","text":"Syntax?","options":["a","b"],"answer":"b"}],"runs":[{"id":"r1"}],"changes":[{"entity":"symbol:x","name":"calc.add","run":"r1"}]});
        let b = brief(&t, "work");
        for part in ["Make the change", "`task done`, a summary in `text`", "Task: Add coolcalc\n\nA CLI calculator.\n", "- spec: http://x", "- human: Keep it small", "- Syntax?\n  Answer: b"] {
            assert!(b.contains(part), "{part} missing from:\n{b}");
        }
        for noise in ["{", "r1", "rank", "calc.add"] {
            assert!(!b.contains(noise), "{noise} in:\n{b}");
        }
        assert!(brief(&t, "investigate").contains("Investigate this task"));
        let r = brief(&t, "review");
        assert!(r.contains("Changes to review:\n- calc.add") && r.contains("`task note`, the feedback in `text`") && r.contains("`run` are refused") && !r.contains("task done"));
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
