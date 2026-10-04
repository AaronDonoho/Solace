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
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS board_events(seq INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS board_before(task TEXT, entity TEXT, data TEXT NOT NULL, PRIMARY KEY(task,entity));
        CREATE TABLE IF NOT EXISTS board_touches(seq INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, entity TEXT);",
    )?;
    fs::create_dir_all(root.join(".solace/board"))?;
    Ok(Store { root: root.into(), db })
}
pub(super) fn err<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "task", message)
}
pub(super) fn id() -> String {
    new_id(Kind::Fn, "board", "").replace("fn:", "")
}
fn task_path(root: &Path, id: &str) -> Result<PathBuf, Failure> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return err("E7001", "invalid task id");
    }
    Ok(root.join(".solace/board").join(format!("{id}.json")))
}
pub(super) fn task(root: &Path, id: &str) -> Result<Value, Failure> {
    let path = task_path(root, id)?;
    if !path.exists() { return err("E7001", "unknown task"); }
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
    if t["column"] == "done" && !workflow(&lockdown::settings(root)?)?["columns"].as_array().unwrap().iter().any(|c| c["id"] == "done") {
        t["column"] = json!("complete");
    }
    Ok(t)
}
fn event(s: &Store, v: Value) -> Result<(), Failure> {
    s.db.execute("INSERT INTO board_events(data) VALUES (?)", [v.to_string()])?;
    Ok(())
}
pub(super) fn save(s: &Store, t: &Value) -> Result<(), Failure> {
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
/// `task diff`: what each entity this task changed looked like before the task,
/// against now. `names` narrows it; past `budget`, the rest are listed by name.
pub fn task_diff(s: &Store, tid: &str, names: &[&str], budget: usize) -> Out {
    let t = task(&s.root, tid)?;
    let current = snapshot(s)?;
    let mut st = s.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
    let before: HashMap<String, Value> = st
        .query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|r| r.map(|(k, v)| (k, serde_json::from_str(&v).unwrap())))
        .collect::<Result<_, _>>()?;
    let text = |key: &str, v: &Value| -> Option<String> {
        if key.starts_with("symbol:") {
            (v[7].as_i64() == Some(1)).then(|| v[4].as_str().unwrap_or("").to_string())
        } else if key.starts_with("module:") {
            v[2].as_str().map(|h| h.to_string())
        } else {
            let bytes: Vec<u8> = v.as_array()?.iter().map(|b| b.as_u64().unwrap() as u8).collect();
            Some(String::from_utf8(bytes).unwrap_or_else(|e| format!("(binary, {} bytes)", e.as_bytes().len())))
        }
    };
    let (mut out, mut left) = (String::new(), vec![]);
    for c in t["changes"].as_array().unwrap() {
        let (key, name) = (c["entity"].as_str().unwrap(), c["name"].as_str().unwrap_or(""));
        let Some(old) = before.get(key) else { continue };
        if !names.is_empty() && !names.iter().any(|n| *n == name || key.split_once(':').is_some_and(|(_, k)| k == *n)) {
            continue;
        }
        let new = match key.strip_prefix("file:") {
            Some(f) => json!(fs::read(lockdown::path(&s.root, f)?).ok()),
            None => current[key].clone(),
        };
        let part = match (text(key, old), text(key, &new)) {
            (a, b) if a == b => format!("## {name} (back to its state before the task)\n"),
            (None, Some(b)) => format!("## {name} (added)\n{}", line_diff("", &b)),
            (Some(a), None) => format!("## {name} (deleted)\n{}", line_diff(&a, "")),
            (a, b) => format!("## {name} (changed)\n{}", line_diff(&a.unwrap_or_default(), &b.unwrap_or_default())),
        };
        if !out.is_empty() && out.len() + part.len() > budget {
            left.push(name.to_string());
        } else {
            out += &part;
            out.push('\n');
        }
    }
    if out.is_empty() {
        return Ok(if names.is_empty() { "This task has no changes.\n".into() } else { "No change of this task matches.\n".into() });
    }
    if !left.is_empty() {
        out += &format!("Left out to fit the reply; read with `task diff <name>`: {}\n", left.join(", "));
    }
    Ok(out)
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
    drafts::clear(s, tid)?;
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
    if let Some(text) = body["text"].as_str() {
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
        let rid = id();
        let kind = if run == "new_work" { "work" } else { run };
        let log = s.root.join(".solace/board").join(t["id"].as_str().unwrap()).join(format!("{rid}.jsonl"));
        t["runs"].as_array_mut().unwrap().push(json!({"id":rid,"kind":kind,"model":m["id"],"effort":m["effort"],"state":"running","started":now(),"log":log,"fresh":run == "new_work" || a["revert"] == true}));
        t["finished"] = Value::Null;
        if kind == "work" {
            t["model"] = m["id"].clone();
            t["effort"] = m["effort"].clone();
        }
        return Ok(Some((rid, m)));
    }
    if column(w, t)["final"] == true {
        end_runs(t, "completed");
        drop_before(s, t["id"].as_str().unwrap())?;
    }
    Ok(None)
}

pub fn api(root: &Path, method: &str, url: &str, body: Value) -> Result<Value, Failure> {
    // Reads skip the project lock, so the board stays responsive while an agent command holds it.
    // Task files are replaced by rename and the store waits out commits, so a read sees whole states.
    let _guard = if method == "GET" { None } else { Some(lock(root)?) };
    let s = open(root)?;
    let w = workflow(&lockdown::settings(root)?)?;
    let (path, query_text) = url.split_once('?').unwrap_or((url, ""));
    let query: HashMap<String,String> = query_text.split('&').filter_map(|pair| pair.split_once('=')).map(|(k,v)|(k.to_string(),v.to_string())).collect();
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if matches!(parts.first(), Some(&"graph" | &"nodes" | &"tests" | &"files")) {
        return view::api(&s, method, &parts, &query, &body);
    }
    match (method, parts.as_slice()) {
        ("GET", ["workflow"]) => Ok(w),
        ("GET", ["models"]) => Ok(json!(
            system()?["models"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|m| {
                    let reason = unavailable(m);
                    json!({"id":m["id"],"label":m["label"],"agent":m["agent"],"effort":m["effort"],"efforts":efforts(m),"available":reason.is_none(),"reason":reason})
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
                    let mut run = t["runs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["id"] == *rid)
                        .cloned()
                        .ok_or_else(|| Failure("E7001 unknown run".into(), 1))?;
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
                Err(Failure(_, 3)) => agent_process(&root, &task(&root, tid)?, &rid, &m, false),
                other => other,
            };
            let current = task(&root, tid)?;
            if t["runs"].as_array().unwrap().iter().any(|r| r["id"] == rid && r["kind"] == "review") || m["run"].is_array() {
                let dir = conversation_dir(&root, tid, &rid)?;
                if dir.exists() { fs::remove_dir_all(dir)?; }
            }
            if column(&workflow(&lockdown::settings(&root)?)?, &current)["final"] == true {
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
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("agent.lock"))?;
    f.lock()?;
    Ok(f)
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

fn agent_process(root: &Path, t: &Value, rid: &str, m: &Value, allow_resume: bool) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let run = t["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).unwrap();
    let log = PathBuf::from(run["log"].as_str().unwrap());
    fs::create_dir_all(log.parent().unwrap())?;
    if run["state"] == "ended" {
        return Ok(());
    }
    let author = run["kind"] != "review";
    let native = m["run"].is_null();
    let claude_home = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".claude"));
    // macOS keychain entries are keyed to CLAUDE_CONFIG_DIR. Keep that sign-in
    // working without copying credentials: use a fresh, nonpersistent handoff.
    let persistent = native && !(m["agent"] == "claude" && cfg!(target_os = "macos")
        && !claude_home.join(".credentials.json").exists() && m["endpoint"].is_null()
        && env::var_os("ANTHROPIC_API_KEY").is_none() && env::var_os("ANTHROPIC_AUTH_TOKEN").is_none());
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
    if author && resume.is_none() {
        if let Some(old) = previous["home"].as_str() {
            let old = conversation_dir(root, tid, old)?;
            if old != dir && old.exists() {
                fs::remove_dir_all(old)?;
            }
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
        if agent == "claude" && !persistent { argv.push("--no-session-persistence".into()); }
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
        if persistent { cmd.env("CLAUDE_CONFIG_DIR", &dir); }
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
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    let mut ending = None;
    let mut forced = false;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let current = task(root, tid)?;
        let r = current["runs"].as_array().unwrap().iter().find(|r| r["id"] == rid).unwrap();
        if r["state"] == "running" && current["paused_from"].is_null() {
            elapsed += last.elapsed();
        }
        last = Instant::now();
        let expired = r["state"] != "ended" && elapsed >= Duration::from_secs(timeout);
        let ended = r["state"] == "ended";
        if ended && matches!(r["outcome"].as_str(), Some("finished" | "reported" | "reviewed")) {
            ending.get_or_insert_with(Instant::now);
        }
        if let Some(status) = child.try_wait()? {
            if !ended {
                let entries = transcript(&log);
                let error = entries.iter().rev().find(|e| e["kind"] == "error").and_then(|e| e["text"].as_str()).unwrap_or("");
                let raw = fs::read_to_string(&log).unwrap_or_default().to_lowercase();
                // Retry only a failed session load, before any tool execution. Never retry quota or work failures.
                if resume.is_some()
                    && !status.success()
                    && !entries.iter().any(|e| e["kind"] == "tool")
                    && ["session not found", "no session found", "failed to resume", "could not find session", "unable to find session"]
                        .iter()
                        .any(|s| raw.contains(s))
                {
                    return Err(Failure("session unavailable; using a fresh handoff".into(), 3));
                }
                let said = if !error.is_empty() { Some(error.to_string()) } else { last_words(&entries) };
                let message = format!("agent exited without finishing ({status}){}", said.map(|s| format!(":\n\n{s}")).unwrap_or_default());
                finish(root, tid, rid, Some(&message))?;
            }
            break;
        }
        if expired || (ended && ending.is_none()) || ending.is_some_and(|at| at.elapsed() >= Duration::from_secs(30)) {
            let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
            let _ = child.kill();
            let _ = child.wait();
            if expired {
                finish(root, tid, rid, Some("task timeout"))?;
            }
            forced = true;
            break;
        }
    }
    if author && persistent {
        let _guard = lock(root)?;
        let mut current = task(root, tid)?;
        current["conversation"]["session"] = if forced { Value::Null } else { json!(session_id(&log, agent)) };
        // Only notes actually briefed, plus the agent's own notes, are known to
        // this conversation. A human may add feedback during graceful exit.
        current["conversation"]["notes_seen"] = json!(t["notes"].as_array().unwrap().iter()
            .chain(current["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent"))
            .map(note_fingerprint).collect::<Vec<_>>());
        save(&open(root)?, &current)?;
    } else {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// The brief, plus the images its notes reference, which are attached to the prompt.
fn continuation_brief(t: &Value, run: &Value, resumed: bool, root: &Path, custom: bool) -> (String, Vec<PathBuf>) {
    let kind = run["kind"].as_str().unwrap_or("");
    let mut packet = t.clone();
    if resumed {
        let seen: HashSet<&str> = t["conversation"]["notes_seen"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        packet["notes"] = json!(t["notes"].as_array().unwrap().iter().filter(|n| !seen.contains(note_fingerprint(n).as_str())).collect::<Vec<_>>());
    }
    let tid = t["id"].as_str().unwrap_or("");
    let mut images = vec![];
    for n in packet["notes"].as_array_mut().into_iter().flatten() {
        let text = note_images(n["text"].as_str().unwrap_or(""), &format!("/tasks/{tid}/files/"), &mut images);
        n["text"] = json!(text);
    }
    let images: Vec<PathBuf> = images.iter().filter_map(|name| image_path(root, tid, name).ok()).collect();
    let mut out = brief(&packet, kind);
    if resumed {
        out = out.replace(
            "Start with `session start --task`, with the task's title and description as `text`.",
            "Use the continuation context below.",
        );
    }
    if kind != "review" && t["runs"].as_array().is_some_and(|r| r.len() > 1) {
        if resumed {
            out += "\nContinue the author's conversation. The role and permissions above replace those of earlier runs. Skip broad orientation; inspect current versions of symbols you need before editing.\n";
        } else {
            out += "\nFresh conversation: use this handoff and the notes above to continue existing work. Reconcile partial edits before proceeding.\n";
        }
        for r in t["runs"].as_array().unwrap().iter().rev().filter(|r| r["id"] != run["id"]).take(3) {
            out += &format!("Previous {}: {}\n", r["kind"].as_str().unwrap_or("run"), r["outcome"].as_str().unwrap_or("unfinished"));
            if let Some(log) = r["log"].as_str() {
                if let Some(said) = last_words(&transcript(Path::new(log))) {
                    out += &format!("Last activity: {said}\n");
                }
            }
        }
    }
    if let Ok(s) = open(root) {
        if let Some(checks) = t["verification"].as_object() {
            let current = verification_base(&s).ok();
            out += "\nLast recorded verification (rerun stale checks before claiming success):\n";
            for (command, v) in checks {
                let stale = current.as_deref() != v["base"].as_str();
                out += &format!("- {command}: {}{}\n{}\n", if v["passed"] == true { "passed" } else { "failed" }, if stale { " (stale: project changed)" } else { "" }, v["summary"].as_str().unwrap_or(""));
            }
        }
        if let Ok(drafts) = drafts::list_for(&s, t["id"].as_str().unwrap_or("")) {
            if !drafts.is_empty() {
                out += &format!("\nRetained drafts (view with `view --draft <id>`):\n{drafts}\n");
            }
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
    let name = format!("{}.{ext}", &hex(&Sha256::digest(bytes))[..16]);
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
    hex(&Sha256::digest(note.to_string().as_bytes()))
}

fn verification_base(s: &Store) -> Out {
    let state = json!([snapshot(s)?, file_snapshot(&s.root, &["run"])?]);
    Ok(hex(&Sha256::digest(state.to_string().as_bytes())))
}

pub fn record_verification(s: &Store, tid: &str, command: &str, result: &Out) -> Result<(), Failure> {
    let mut t = task(&s.root, tid)?;
    if !t["verification"].is_object() { t["verification"] = json!({}); }
    let summary = match result { Ok(out) => out, Err(e) => &e.0 };
    t["verification"][command] = json!({"passed":result.is_ok(),"base":verification_base(s)?,"summary":summary.chars().take(1000).collect::<String>()});
    save(s, &t)
}
/// An agent's prompt: what its run is for, how to start, ask and finish, then
/// what it needs from the task (Spec 15 §6), as plain text.
fn brief(t: &Value, kind: &str) -> String {
    let start = "Start with `session start --task`, with the task's title and description as `text`.";
    let ask = "If you need a decision from the human, ask with `task ask` (`text` and 2–4 `options`); the call waits for the answer.";
    let read_only = "This run is read-only: reading, `check`, `test`, `build` and `probe` work; `submit`, `delete`, `write`, `remove`, `format` and `run` are refused.";
    let role = match kind {
        "review" => format!(
            "Review the changes this task made (listed below) and give feedback. Start with `task diff`: each change against its state before the task. Then read what you need in small batches, one concern at a time. {read_only} Finish with `task note`, the feedback in `text`."
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
    section("Task changes", list("changes").iter().map(|c| format!("- {}", text(&c["name"]))).collect());
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
    let w = workflow(&lockdown::settings(root)?)?;
    let r = t["runs"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == rid).unwrap();
    if r["state"] == "ended" {
        return Ok(());
    }
    r["state"] = json!("ended");
    r["ended"] = json!(now());
    r["outcome"] = json!(failure.unwrap_or("finished"));
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
                t["finished"] = json!({"kind":"review","outcome":"reviewed","time":now()});
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
            t["finished"] = json!({"kind":t["runs"][i]["kind"],"outcome":t["runs"][i]["outcome"],"time":now()});
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
            return wait_reply(s, tid, i);
        }
        _ => return usage("task note | task ask | task done, with the text in `text`; task diff [name ...]"),
    }
    if column(&w, &t)["final"] == true {
        drop_before(s, tid)?;
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
/// A work run's `write solace.toml` or `write AGENTS.md`: checked now, written
/// only when a human approves it on the board (D-38, D-45).
pub fn propose_settings(s: &Store, tid: &str, rid: &str, file: &str, base: &[u8], proposed: &str, guard: fs::File) -> Out {
    if file == "solace.toml" {
        workflow(&lockdown::parse_settings(proposed)?)?;
    }
    let base = std::str::from_utf8(base).map_err(|_| Failure(format!("E6020 {file} is not UTF-8"), 1))?;
    if base == proposed {
        return Ok(format!("{file} is unchanged.\n"));
    }
    let mut t = task(&s.root, tid)?;
    let w = workflow(&lockdown::settings(&s.root)?)?;
    let i = t["runs"].as_array().unwrap().iter().position(|r| r["id"] == rid).unwrap();
    t["questions"].as_array_mut().unwrap().push(json!({"id":id(),"text":format!("Change {file}"),"options":["Approve","Reject"],
        "settings":{"file":file,"base":base,"proposed":proposed}}));
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
    let p = root.join(&file);
    let current = fs::read_to_string(&p).unwrap_or_default();
    let proposed = q["settings"]["proposed"].as_str().unwrap();
    if current != q["settings"]["base"] || (file == "solace.toml" && lockdown::parse_settings(proposed).and_then(|v| workflow(&v)).is_err()) {
        q["answer"] = json!(format!("Not applied: {file} changed since the proposal; read it and propose again"));
        return Ok(());
    }
    fs::write(p, proposed)?;
    q["answer"] = json!("Approved and written");
    Ok(())
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
                let work = t["runs"].as_array().unwrap().iter().any(|r| r["state"] != "ended" && r["kind"] == "work");
                end_runs(&mut t, "server restarted");
                flag(&mut t, "the Solace server restarted while an agent was working");
                if work {
                    move_to(&w, &mut t, &w["events"]["failed"])?;
                }
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
                let mut reader = Events { root, seq, caught_up: false, bytes: std::io::Cursor::new(vec![]) };
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
            if req.method().as_str() == "GET" && matches!(url.split('?').next().unwrap_or(""), "/" | "/view" | "/app.js" | "/guide" | "/drawing.woff") {
                let (data, kind) = match url.split('?').next().unwrap_or("") {
                    "/" | "/view" => (include_bytes!("../board/index.html").as_slice(), "text/html"),
                    "/guide" => (include_bytes!("../board/guide.html").as_slice(), "text/html"),
                    "/drawing.woff" => (include_bytes!("../board/priv/drawing.woff").as_slice(), "font/woff"),
                    _ => (include_bytes!("../board/priv/app.js").as_slice(), "text/javascript"),
                };
                let _ = req.respond(
                    tiny_http::Response::from_data(data).with_header(tiny_http::Header::from_bytes("Content-Type", kind).unwrap()),
                );
                return;
            }
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
            let limit = if upload_to.is_some() { IMAGE_LIMIT } else { 1_048_576 };
            let mut bytes = Vec::new();
            let parsed = req.as_reader().take(limit as u64 + 1).read_to_end(&mut bytes);
            let result = if parsed.is_err() || bytes.len() > limit {
                err("E7001", if upload_to.is_some() { "image over 5 MB" } else { "request body too large" })
            } else if let Some(tid) = upload_to {
                upload(&root, &tid, &bytes)
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
    /// Whether this connection has sent `ready`, the end of what it missed.
    caught_up: bool,
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
        let root = env::temp_dir().join(id());
        fs::create_dir_all(root.join(".solace/board")).unwrap();
        fs::write(root.join(".solace/board/t1.json"), r#"{"id":"t1","column":"done"}"#).unwrap();
        assert_eq!(task(&root, "t1").unwrap()["column"], "complete");
        // A custom workflow that still has a `done` column keeps it.
        fs::write(root.join("solace.toml"), include_str!("default-board.toml").replace("= \"complete\"", "= \"done\"")).unwrap();
        assert_eq!(tasks(&root).unwrap()[0]["column"], "done");
        fs::remove_dir_all(root).unwrap();
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
        let lines = [call("m1", anthropic.clone()), call("m1", anthropic), call("m2", json!({"input_tokens":300,"cache_read_input_tokens":250,"output_tokens":2}))];
        fs::write(&log, lines.join("\n")).unwrap();
        assert_eq!(transcript(&log).last().unwrap()["text"], "Context 300 tokens; 9 output tokens so far");
        let result = json!({"type":"result","usage":{"input_tokens":310,"cache_read_input_tokens":350,"output_tokens":9}}).to_string();
        fs::write(&log, [lines.join("\n"), result].join("\n")).unwrap();
        let usage: Vec<Value> = transcript(&log).into_iter().filter(|e| e["kind"] == "usage").collect();
        assert_eq!(usage.len(), 1);
        assert!(usage[0]["text"].as_str().unwrap().ends_with("; last context 300 tokens"));
        assert_eq!(last_words(&[json!({"kind":"message","text":"é".repeat(600)})]).unwrap().chars().count(), 501);
        fs::remove_file(log).unwrap();
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
        assert!(brief(&t, "investigate").contains("Investigate this task"));
        let r = brief(&t, "review");
        assert!(
            r.contains("Task changes:\n- calc.add")
                && r.contains("`task note`, the feedback in `text`")
                && r.contains("`run` are refused")
                && !r.contains("task done")
        );
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
