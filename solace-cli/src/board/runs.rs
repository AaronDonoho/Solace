use serde_json::Value;
use std::path::Path;
use crate::board::task_model::After;
use std::time::Duration;
use std::time::Instant;
use serde_json::json;
use std::env;
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs;
use crate::Store;
use std::process::Command;
use std::process::Stdio;
use crate::board::task_model::lock;
use crate::board::task_model::open;
use crate::board::err;
use crate::board::setup::system;
use crate::board::setup::endpoint_args;
use crate::board::setup::endpoint_context;
use crate::board::reviews::review_path;
use crate::board::reviews::review_state;
use crate::board::briefs::continuation_brief;
use crate::board::briefs::image_type;
use crate::board::briefs::base64;
use crate::board::briefs::note_fingerprint;
use crate::board::reviews::recoverable_review;
use crate::board::reviews::record_review;
use crate::board::setup::keys_path;
use crate::board::setup::stored;
use crate::board::task_model::task_path;
use crate::board::task_model::task_dir;
use crate::board::task_model::task;
use crate::board::task_model::read_task;
use crate::board::task_model::flow;
use crate::board::task_model::note;
use crate::board::task_model::flag;
use crate::board::task_model::hold;
use crate::board::task_model::on_event;
use crate::board::task_model::run_at;
use crate::board::task_model::end_run;
use crate::board::task_model::run_state;
use crate::failure::Failure;
#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
use crate::fsx;
use crate::drafts;
use crate::tasks;
use crate::scope;
use crate::lockdown;
use crate::jsonx;
use std::os::unix::process::CommandExt;
use std::io::Write;
use crate::board::briefs::task_drafts;
use crate::board::reviews::review_message;
use crate::unique_id;
use crate::failure::Out;
use crate::failure::usage;
use crate::board::task_model::message;
use crate::board::roles::STRATEGY_RUN;
pub(crate) fn after(root: &Path, t: &Value, after: After) {
    match after {
        After::Start(rid, m) => start(root.to_path_buf(), t.clone(), rid, m),
        After::Clean(_) => {
            let root = root.to_path_buf();
            let tid = t["id"].as_str().unwrap().to_string();
            std::thread::spawn(move || {
                if let Ok(_guard) = conversation_lock(&root, &tid) {
                    let _ = clean_conversation(&root, &tid);
                }
            });
        }
        After::Nothing | After::Removed(_) => {}
    }
}

pub(super) fn start(root: PathBuf, t: Value, rid: String, m: Value) {
    std::thread::spawn(move || {
        let tid = t["id"].as_str().unwrap();
        let result = (|| {
            // A follow-up may be queued while the previous agent is exiting.
            // Never open one provider conversation in two processes.
            let _guard = conversation_lock(&root, tid)?;
            let t = task(&root, tid)?;
            if t["runs"].as_array().unwrap().iter().any(|r| r["id"] == rid && r["kind"] == "review") {
                write_review_checkpoint(&root, &t, &rid)?;
            }
            let mut result = agent_process(&root, &t, &rid, &m, true, false);
            let mut retried = false;
            let result = loop {
                match result {
                    Err(Failure(_, SESSION_GONE)) => {
                        result = agent_process(&root, &task(&root, tid)?, &rid, &m, false, false);
                    }
                    // A clean run that never called Solace gets one fresh agent, never a resume.
                    Err(Failure(_, NO_TOOL)) if !retried => {
                        retried = true;
                        result = agent_process(&root, &task(&root, tid)?, &rid, &m, false, true);
                    }
                    // A discussion agent that exited after replying gets a fresh agent,
                    // briefed with every note, at the human's next note.
                    Err(Failure(_, REPLIED)) => {
                        wait_discussion(&open(&root)?, tid, &rid, None)?;
                        result = agent_process(&root, &task(&root, tid)?, &rid, &m, false, false);
                    }
                    other => break other,
                }
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
            if crate::board::task_model::column(&flow(&root)?, &current)["final"] == true {
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
    let dir = task_dir(root, tid)?;
    fs::create_dir_all(&dir)?;
    fsx::lock_file(&dir.join("agent.lock"))
}

fn conversation_dir(root: &Path, tid: &str, home: &str) -> Result<PathBuf, Failure> {
    // Reuse the id validator before allowing any deletion or launch path.
    let _ = task_path(root, home)?;
    Ok(task_dir(root, tid)?.join(home))
}

fn clean_conversation(root: &Path, tid: &str) -> Result<(), Failure> {
    let _guard = lock(root)?;
    let s = open(root)?;
    let t = task(root, tid)?;
    if let Some(home) = t["conversation"]["home"].as_str() {
        let p = conversation_dir(root, tid, home)?;
        if p.exists() {
            fs::remove_dir_all(p)?;
        }
    }
    drafts::clear(&s, tid)?;
    tasks::update(root, tid, |t| {
        t["conversation"] = Value::Null;
        Ok(())
    })
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

/// `agent_process`'s result when a discussion agent replied in plain text and
/// exited: the reply is recorded and the run waits for the human.
pub(crate) const REPLIED: i32 = 4;

fn agent_process(root: &Path, t: &Value, rid: &str, m: &Value, allow_resume: bool, retried: bool) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    let run = &t["runs"][run_at(t, rid)?];
    let log = PathBuf::from(run["log"].as_str().unwrap());
    fs::create_dir_all(log.parent().unwrap())?;
    if run["state"] == "ended" {
        return Ok(());
    }
    // A review or discussion has its own conversation, never the author's.
    let author = !matches!(run["kind"].as_str(), Some("review" | "discuss"));
    let persistent = persists(m);
    let previous = &t["conversation"];
    // Compare launch-relevant configuration, not labels. A changed provider, endpoint or runner gets a handoff; model and effort are launch flags and keep the session (D-75).
    let compatibility = json!({"agent":m["agent"],"endpoint":m["endpoint"],"run":m["run"]});
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
    // A run starts in the project of its task holding the most symbols the
    // task names (D-123); a resumed conversation stays where it was.
    let here = run_project(previous, resume.is_some(), root, t);
    let at = scope::path(&scope::top(root), &here);
    let settings = system()?;
    let involvement = crate::board::setup::involvement(&settings);
    let detail = crate::board::setup::detail(&settings);
    let asking = crate::board::setup::asking(&settings);
    let (prompt, images) = continuation_brief(t, run, resume.is_some(), root, &at, m["run"].is_array(), involvement, detail, asking);
    if run["kind"] == "discuss" {
        crate::board::delivery::mark_discussion_brief(root, tid, rid, t)?;
    }
    let timeout = settings["task_timeout"].as_u64().unwrap_or(3600);
    if author {
        // Only an exited process has a reusable session; a server crash during
        // a turn falls back instead of resuming a possibly orphaned process.
        let conversation = if persistent { json!({"home":home,"config":compatibility,"session":null,"project":here}) } else { Value::Null };
        tasks::update(root, tid, move |current| {
            current["conversation"] = conversation;
            Ok(())
        })?;
    }
    let mut child = spawn_agent(&at, &dir, &log, m, &prompt, &images, resume.as_deref(), persistent, [tid, rid], timeout)?;
    let supervised =
        supervise(Watch::Task(root, tid), rid, &mut child, &log, m, resume.is_some(), retried, Duration::from_secs(timeout), GRACEFUL_EXIT);
    let (started, outcome) = run_state(root, tid, rid);
    // A discussion agent that replied in plain text took its turn: its reply
    // answers like a finished run, so it clears a mark the same way.
    let outcome = match &supervised {
        Err(Failure(_, REPLIED)) => Some("discussed".to_string()),
        _ => outcome,
    };
    crate::board::agents::after_run(m, &log, started, outcome.as_deref());
    if !(author && persistent) && matches!(supervised, Err(Failure(_, REPLIED))) && dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    let forced = supervised?;
    if author && persistent {
        let session = if forced { Value::Null } else { json!(session_id(&log, agent_of(m))) };
        let (briefed, rid) = (t["notes"].clone(), rid.to_string());
        tasks::update(root, tid, move |current| {
            current["conversation"]["session"] = session;
            // Only notes actually briefed, plus the agent's own notes, are known to
            // this conversation. A human may add feedback during graceful exit.
            current["conversation"]["notes_seen"] = json!(
                briefed
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(current["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent"))
                    .map(note_fingerprint)
                    .collect::<Vec<_>>()
            );
            Ok(())
        })?;
    } else {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

fn agent_of(m: &Value) -> &str {
    m["agent"].as_str().unwrap_or("codex")
}

pub(crate) fn claude_home() -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".claude"))
}

/// Whether model `m`'s agent keeps its conversation in its own folder.
pub(crate) fn persists(m: &Value) -> bool {
    // macOS keychain entries are keyed to CLAUDE_CONFIG_DIR. Keep that sign-in
    // working without copying credentials: use a fresh, nonpersistent handoff.
    m["run"].is_null()
        && !(m["agent"] == "claude"
            && cfg!(target_os = "macos")
            && !claude_home().join(".credentials.json").exists()
            && m["endpoint"].is_null()
            && env::var_os("ANTHROPIC_API_KEY").is_none()
            && env::var_os("ANTHROPIC_AUTH_TOKEN").is_none())
}

/// The project task `t`, at home in `home`, starts its runs in (D-123): the
/// one of its scope with the most symbols its title and body name as
/// `module.name`; the home when none has more.
pub(crate) fn start_project(home: &Path, t: &Value) -> String {
    let top = scope::top(home);
    let mine = scope::dir(&top, home);
    let text = format!("{}\n{}", t["title"].as_str().unwrap_or(""), t["body"].as_str().unwrap_or(""));
    let mut candidates = vec![mine.clone()];
    candidates.extend(scope::of(t, &mine).into_iter().filter(|d| *d != mine));
    let mut best = (0, mine);
    for d in candidates {
        let path = scope::path(&top, &d);
        if crate::language::detect(&path).is_none() {
            continue;
        }
        let Ok(s) = Store::peek(&path) else { continue };
        let Ok(ix) = s.index() else { continue };
        let n = s.named(&ix, &text).len();
        if n > best.0 {
            best = (n, d);
        }
    }
    best.1
}

/// The project a run starts in: a resumed conversation keeps the project its
/// session began in — the one recorded with it, or the task's home for a
/// conversation saved before projects were recorded. A fresh run starts where
/// its task's symbols are (D-123).
pub(crate) fn run_project(previous: &Value, resume: bool, home: &Path, t: &Value) -> String {
    match previous["project"].as_str() {
        Some(p) if resume => p.to_string(),
        None if resume => scope::dir(&scope::top(home), home),
        _ => start_project(home, t),
    }
}

/// Starts model `m`'s agent with its settings in `dir`, its output in `log`
/// and `prompt` (with `images`) on stdin: the launch every run shares, a
/// task's or a strategy run's. `ids` are its `SOLACE_TASK` and `SOLACE_RUN`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_agent(
    root: &Path,
    dir: &Path,
    log: &Path,
    m: &Value,
    prompt: &str,
    images: &[PathBuf],
    resume: Option<&str>,
    persistent: bool,
    ids: [&str; 2],
    timeout: u64,
) -> Result<std::process::Child, Failure> {
    let custom = m["run"].as_array();
    let native = m["run"].is_null();
    // `SOLACE_BIN` names the CLI agents call when the server is another program (D-80).
    let binary = match env::var_os("SOLACE_BIN").filter(|b| !b.is_empty()) {
        Some(b) => PathBuf::from(b),
        None => env::current_exe()?,
    };
    let agent = agent_of(m);
    let stored_key = m["id"]
        .as_str()
        .and_then(|id| crate::board::setup::stored(&crate::board::setup::keys_path(), id))
        .filter(|_| m.get("endpoint").is_some());
    let launch: Value = serde_json::from_str(&lockdown::agent_config(agent, dir, &binary, timeout)?).unwrap();
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
        if let Some(session) = resume {
            if agent == "codex" {
                argv.insert(2, "resume".into());
                argv.push(session.into());
            } else {
                argv.extend(["--resume".into(), session.into()]);
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
            argv.extend(endpoint_args(m, stored_key.is_some()));
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
        let auth = claude_home().join(".credentials.json");
        // A token refresh replaces the symlink with a private copy whose rotated
        // token goes stale; relink so every launch uses the real sign-in.
        let link = dir.join(".credentials.json");
        if auth.exists() && !fs::read_link(&link).is_ok_and(|t| t == auth) {
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(auth, link)?;
        }
    }
    let file = fs::File::create(log)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(root).process_group(0).stdin(Stdio::piped()).stdout(file.try_clone()?).stderr(file);
    for (k, v) in launch["env"].as_object().unwrap() {
        cmd.env(k, v.as_str().unwrap());
    }
    if agent == "codex"
        && let Some(key) = &stored_key
    {
        cmd.env(crate::board::setup::KEY_VAR, key);
    }
    if agent == "claude" && custom.is_none() {
        if persistent {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        if let Some(e) = m.get("endpoint") {
            cmd.env("ANTHROPIC_BASE_URL", e["base_url"].as_str().unwrap_or(""));
            if let Some(key) = endpoint_key(m) {
                cmd.env("ANTHROPIC_AUTH_TOKEN", key);
            }
            if env::var_os("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none()
                && let Some(n) = endpoint_context(e["base_url"].as_str().unwrap_or(""), m["model"].as_str().unwrap_or(""))
            {
                cmd.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", n.to_string());
            }
        }
    }
    cmd.env("SOLACE_TASK", ids[0])
        .env("SOLACE_RUN", ids[1])
        .env("SOLACE_SESSION", ids[1])
        .env("SOLACE_BIN", &binary)
        .env("SOLACE_LAUNCH_TIMEOUT_SECS", timeout.to_string());
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Failure(crate::board::agents::missing_program_at_launch(m), 1),
        _ => Failure(e.to_string(), 1),
    })?;
    let mut stdin = child.stdin.take().unwrap();
    if agent == "claude" && custom.is_none() && !images.is_empty() {
        // Claude Code takes images only inside a stream-json user message.
        let mut content = vec![json!({"type":"text","text":prompt})];
        for image in images {
            let media = image_type(&image.to_string_lossy()).unwrap_or("image/png");
            content.push(json!({"type":"image","source":{"type":"base64","media_type":media,"data":base64(&fs::read(image)?)}}));
        }
        writeln!(stdin, "{}", json!({"type":"user","message":{"role":"user","content":content}}))?;
    } else {
        writeln!(stdin, "{prompt}")?;
    }
    drop(stdin);
    Ok(child)
}

/// How long a run that called a finishing tool may take to exit (spec 15).
pub(crate) const GRACEFUL_EXIT: Duration = Duration::from_secs(30);

/// Watches a launched agent until it exits or is killed, and says whether it
/// was killed. Running time (not paused time) past `timeout` ends the run; a
/// run that finished gets `grace` to exit; any other ended run is killed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn supervise(
    watch: Watch,
    rid: &str,
    child: &mut std::process::Child,
    log: &Path,
    m: &Value,
    resumed: bool,
    retried: bool,
    timeout: Duration,
    grace: Duration,
) -> Result<bool, Failure> {
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    let mut ending = None;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let (run, paused) = watch.run(rid)?;
        let r = &run;
        if r["state"] == "running" && !paused {
            elapsed += last.elapsed();
        }
        last = Instant::now();
        let expired = r["state"] != "ended" && elapsed >= timeout;
        let ended = r["state"] == "ended";
        if ended && matches!(r["outcome"].as_str(), Some("finished" | "reported" | "reviewed" | "discussed" | "planned")) {
            ending.get_or_insert_with(Instant::now);
        }
        if let Some(status) = child.try_wait()? {
            if !ended {
                handle_exit(&watch, rid, RunContext { run: r, log, model: m, resumed, retried }, status)?;
            }
            return Ok(false);
        }
        if expired || (ended && ending.is_none()) || ending.is_some_and(|at| at.elapsed() >= grace) {
            lockdown::kill_group(child);
            if expired {
                watch.fail(rid, "task timeout")?;
            }
            return Ok(true);
        }
    }
}

/// What `supervise` watches: a task's run (the project and task), or a
/// strategy run of the root (D-93).
pub(crate) enum Watch<'a> {
    Task(&'a Path, &'a str),
    Strategy(&'a Path),
}

impl Watch<'_> {
    /// The run as it is now, and whether its task is paused.
    fn run(&self, rid: &str) -> Result<(Value, bool), Failure> {
        match self {
            Watch::Task(root, tid) => {
                let t = task(root, tid)?;
                let i = run_at(&t, rid)?;
                Ok((t["runs"][i].clone(), !t["paused_from"].is_null()))
            }
            Watch::Strategy(top) => match tasks::strategy_run(top, rid) {
                Some(run) => Ok((run, false)),
                None => err("E7023", format!("unknown strategy run {rid}")),
            },
        }
    }
    /// Records a plain-text reply as the agent's note; the run waits for the human.
    fn reply(&self, rid: &str, reply: String) -> Result<(), Failure> {
        let rid = rid.to_string();
        match self {
            Watch::Task(root, tid) => tasks::update(root, tid, move |t| {
                let i = run_at(t, &rid)?;
                if t["runs"][i]["state"] != "ended" {
                    note(t, "agent", Some(&rid), &reply);
                    t["runs"][i]["state"] = json!("waiting");
                }
                Ok(())
            }),
            Watch::Strategy(top) => tasks::update_run(top, &rid.clone(), move |r| {
                if r["state"] != "ended" {
                    note(r, "agent", Some(&rid), &reply);
                    r["state"] = json!("waiting");
                }
                Ok(())
            }),
        }
    }
    /// Ends the run as failed, saying why. A strategy run ends `stopped`, with
    /// its notes as one transcript note.
    pub(crate) fn fail(&self, rid: &str, message: &str) -> Result<(), Failure> {
        match self {
            Watch::Task(root, tid) => finish(root, tid, rid, Some(message)),
            Watch::Strategy(top) => {
                let (rid, message) = (rid.to_string(), message.to_string());
                tasks::update_run(top, &rid.clone(), move |r| {
                    if r["state"] != "ended" {
                        note(r, "solace", Some(&rid), &message);
                        tasks::end_strategy(r, "stopped");
                    }
                    Ok(())
                })
            }
        }
    }
}

/// D-157: saves run `rid`'s reads on its run, through the inbox.
pub(crate) fn record_reads(top: &Path, tid: &str, rid: &str, reads: Value) -> Result<(), Failure> {
    tasks::send(top, json!({"kind":"reads","task":tid,"run":rid,"reads":reads}))?;
    Ok(())
}

/// D-157: run `rid`'s reads as its MCP process last saved them.
pub(crate) fn run_reads(top: &Path, tid: &str, rid: &str) -> Value {
    run_record(top, tid, rid).map_or(Value::Null, |r| r["reads"].clone())
}

/// D-157: run `rid`'s steps so far, one line each (D-99), the latest that fit in `budget`.
pub(crate) fn run_outline(top: &Path, tid: &str, rid: &str, budget: usize) -> String {
    let Some(r) = run_record(top, tid, rid) else {
        return String::new();
    };
    outline(&transcript(Path::new(r["log"].as_str().unwrap_or(""))), budget)
}

pub(crate) fn run_record(top: &Path, tid: &str, rid: &str) -> Option<Value> {
    let t = read_task(top, &task_path(top, tid).ok()?).ok()?;
    let i = run_at(&t, rid).ok()?;
    Some(t["runs"][i].clone())
}

/// The tool calls and messages of `steps`, one line each and numbered as in
/// the run log: the latest that fit in `budget`, after how many were left out.
pub(crate) fn outline(steps: &[Value], budget: usize) -> String {
    let lines: Vec<String> = steps
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e["kind"].as_str(), Some("tool" | "message")))
        .map(|(i, e)| step_line(i, e) + "\n")
        .collect();
    let (mut kept, mut size) = (0, 0);
    for l in lines.iter().rev() {
        // room for the line naming what's left out
        if size + l.len() + 40 > budget {
            break;
        }
        size += l.len();
        kept += 1;
    }
    let left = lines.len() - kept;
    let head = if left > 0 { format!("Earlier steps left out: {left}\n") } else { String::new() };
    if head.len() > budget {
        return String::new();
    }
    head + &lines[left..].concat()
}

/// A line cut to 100 characters, with `…` when more was cut.
pub(crate) fn cut_line(what: &str) -> String {
    let cut: String = what.chars().take(100).collect();
    let more = if cut.len() < what.len() { "…" } else { "" };
    format!("{cut}{more}")
}

/// One line for a transcript entry: its number, kind, and text's first line cut to 100.
pub(crate) fn step_line(i: usize, e: &Value) -> String {
    let kind = e["kind"].as_str().unwrap_or("");
    let what = match kind {
        "tool" => format!("{} ({})", e["title"].as_str().unwrap_or("").lines().next().unwrap_or(""), e["status"].as_str().unwrap_or("")),
        _ => e["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string(),
    };
    format!("{} {kind}: {}", i + 1, cut_line(&what))
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
// rereads the whole log per request; keep an offset if logs get large.
pub fn transcript(log: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    let mut tools: HashMap<String, usize> = HashMap::new();
    // Claude Code: the context of its latest API call, and output per call (events repeat per content block).
    let (mut context, mut output, mut ended) = (None, HashMap::new(), false);
    let text = fs::read_to_string(log).unwrap_or_default();
    let ends = fs::read_to_string(server_end_log(log)).unwrap_or_default();
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
                let tool = item["tool"].as_str().unwrap_or("tool");
                let title = match partly_failed(status, &output) {
                    Some(p) => format!("{tool} {p} {command}"),
                    None => format!("{tool} {command}"),
                };
                let entry = json!({"kind":"tool","title":title,"input":input,"output":output,"status":status});
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
                let input = u["input_tokens"].as_u64().unwrap_or(0);
                let cached = u["cached_input_tokens"].as_u64();
                let cache = cached
                    .map(|n| {
                        let ratio = cache_ratio(Some(n), input).map(|r| format!(", {r}")).unwrap_or_default();
                        format!(" ({n} cached{ratio})")
                    })
                    .unwrap_or_default();
                out.push(json!({"kind":"usage","text":format!("{input} input tokens{cache}, {} output tokens", u["output_tokens"])}));
            }
            ("error", _) => out.push(json!({"kind":"error","text":e["message"]})),
            ("turn.failed", _) => out.push(json!({"kind":"error","text":e["error"]["message"],"fatal":true})),
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
                        if let Some(p) = partly_failed(out[i]["status"].as_str().unwrap_or(""), out[i]["output"].as_str().unwrap_or("")) {
                            out[i]["title"] = json!(out[i]["title"].as_str().unwrap_or("").replacen(' ', &format!(" {p} "), 1));
                        }
                    }
                }
            }
            ("result", _) => {
                ended = true;
                let u = &e["usage"];
                let last = context.map(|c| format!("; last context {c} tokens")).unwrap_or_default();
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let input = context_tokens(u);
                let cached = u["cache_read_input_tokens"].as_u64();
                let written = if u.get("cache_creation_input_tokens").is_some() {
                    format!(", {} written to cache", n("cache_creation_input_tokens"))
                } else {
                    String::new()
                };
                let cache = match cached {
                    Some(c) => {
                        let ratio = cache_ratio(Some(c), input).map(|r| format!(", {r}")).unwrap_or_default();
                        format!(" ({c} read from cache{written}{ratio})")
                    }
                    None if written.is_empty() => String::new(),
                    None => format!(" ({} written to cache)", n("cache_creation_input_tokens")),
                };
                out.push(json!({"kind":"usage","text":format!("{input} input tokens{cache}, {} output tokens{last}", n("output_tokens"))}));
                if e["is_error"] == true {
                    out.push(json!({"kind":"error","text":text_of(&e["result"]),"fatal":true}));
                }
            }
            _ => {}
        }
    }
    for line in ends.lines() {
        if !line.trim().is_empty() {
            out.push(json!({"kind":"error","text":line,"source":SERVER}));
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
    let w = flow(root)?;
    let (rid, failure) = (rid.to_string(), failure.map(String::from));
    tasks::update(root, tid, move |t| {
        let i = run_at(t, &rid)?;
        let r = &mut t["runs"][i];
        if r["state"] == "ended" {
            return Ok(());
        }
        end_run(r, failure.as_deref().unwrap_or("finished"));
        // Only a failed work run moves the task; a failed review or investigation stays in its column.
        let work = r["kind"] == "work";
        if let Some(message) = &failure {
            r["failed"] = json!(true);
            note(t, "solace", Some(&rid), message);
            if work {
                hold(t, message);
                on_event(&w, t, "failed")?;
            } else {
                flag(t, message);
            }
        }
        Ok(())
    })
}

/// Writes task `t`'s checkpoint for review run `rid` before its agent starts,
/// so the reply that queued the review does not wait for the whole store.
fn write_review_checkpoint(root: &Path, t: &Value, rid: &str) -> Result<(), Failure> {
    let mut checkpoint = review_state(&open(root)?, t)?;
    checkpoint["run"] = json!(rid);
    let path = review_path(root, t["id"].as_str().unwrap(), rid)?;
    fs::create_dir_all(path.parent().unwrap())?;
    jsonx::write(&path, &checkpoint)
}

/// Handles an agent that exited before its run ended.
fn handle_exit(watch: &Watch, rid: &str, context: RunContext<'_>, status: std::process::ExitStatus) -> Result<(), Failure> {
    let RunContext { run: r, log, model: m, resumed, retried } = context;
    let entries = transcript(log);
    let blocked = has_blocking_agent_error(&entries);
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
    // A discussion or strategy agent that replied in plain text and exited
    // cleanly has taken its turn: the reply is its note, and the run waits.
    let reply = entries.iter().rev().find(|e| e["kind"] == "message").and_then(|e| e["text"].as_str()).map(str::trim).unwrap_or("");
    if matches!(r["kind"].as_str(), Some("discuss" | "strategy")) && status.success() && !blocked && !reply.is_empty() {
        watch.reply(rid, reply.to_string())?;
        return Err(Failure("discussion reply".into(), REPLIED));
    }
    if let Some(assessment) = recoverable_review(r, status.success(), &entries)
        && record_review(watch, rid, &assessment)?
    {
        return Ok(());
    }
    if let Some(message) = crate::board::agents::stopped(m, &entries) {
        return watch.fail(rid, &message);
    }
    // A custom `run` command writes a log in its own format, so its tool
    // entries can't say whether the Solace tool was offered.
    if m["run"].is_null() && status.success() && !blocked && !called_solace(&entries) {
        if retried || matches!(watch, Watch::Strategy(..)) {
            return watch.fail(rid, NO_TOOL_MESSAGE);
        }
        return Err(Failure("no Solace call; trying a fresh agent".into(), NO_TOOL));
    }
    let said = exit_error(&entries).map(str::to_string).or_else(|| last_words(&entries));
    let message = format!("{} ({status}){}", UNFINISHED_EXIT, said.map(|s| format!(":\n\n{s}")).unwrap_or_default());
    watch.fail(rid, &message)
}

/// The run that exited, its transcript, and how it was launched.
pub(crate) struct RunContext<'a> {
    pub(crate) run: &'a Value,
    pub(crate) log: &'a Path,
    pub(crate) model: &'a Value,
    pub(crate) resumed: bool,
    pub(crate) retried: bool,
}

/// `event` with its task's board visibility worked out, where the stream's
/// reader expects it (D-196). `has_project` is what the stream already knows.
pub(crate) fn stream_task_event(top: &Path, mut event: Value, has_project: bool) -> Value {
    if event.get("task").is_some_and(Value::is_object) {
        let task = event["task"].take();
        event["task"] = tasks::with_on_board(top, task, has_project, &mut HashMap::new());
    }
    event
}

/// Whether a transcript entry calls the `solace` MCP tool, not one of the
/// harness's resource tools.
fn called_solace(entries: &[Value]) -> bool {
    entries.iter().any(|e| e["kind"] == "tool" && e["title"].as_str().is_some_and(solace_title))
}

/// The key model `m`'s endpoint uses: the stored one, else its `key_env` variable's.
pub(super) fn endpoint_key(m: &Value) -> Option<String> {
    m["id"].as_str().and_then(|id| stored(&keys_path(), id)).or_else(|| m["endpoint"]["key_env"].as_str().and_then(|k| env::var(k).ok()))
}

/// The agent's last message, else its last error or output, cut to 1000 bytes.
pub(crate) fn last_words(entries: &[Value]) -> Option<String> {
    let last = |kind: &str| entries.iter().rev().find(|e| e["kind"] == kind).and_then(|e| e["text"].as_str());
    let text = last("message").or_else(|| last("error")).or_else(|| last("output"))?.trim();
    if text.is_empty() {
        return None;
    }
    let cut = (text.len().saturating_sub(1000)..=text.len()).find(|&i| text.is_char_boundary(i)).unwrap();
    Some(if cut > 0 { format!("…{}", &text[cut..]) } else { text.to_string() })
}

/// D-109: a failed batch where only some commands failed, from the reply's
/// last line (`1 of 3 commands failed: …`): `(1 of 3 failed)`, for the step's title.
/// D-119: a call that stopped at a failed change (`…; skipped: a; b`) adds `, 2 skipped`;
/// the count is capped at the commands the reply says were left, since a command's
/// text can itself hold `; `.
pub(crate) fn partly_failed(status: &str, output: &str) -> Option<String> {
    let last = output.trim_end().lines().last()?;
    let (last, listed) = match last.rsplit_once("; skipped: ") {
        Some((l, s)) => (l, s.split("; ").count()),
        None => (last, 0),
    };
    let (n, rest) = last.split_once(" of ")?;
    let (m, _) = rest.split_once(" commands failed: ")?;
    let (n, m): (usize, usize) = (n.parse().ok()?, m.parse().ok()?);
    let skipped = listed.min(m.saturating_sub(n));
    let skipped = if skipped > 0 { format!(", {skipped} skipped") } else { String::new() };
    (status == "failed" && n < m).then(|| format!("({n} of {m} failed{skipped})"))
}

/// Whether the current board run is a work run, which may finish with `task done`.
pub fn work_run(root: &Path) -> bool {
    let Some((tid, rid)) = context() else { return false };
    let Ok(t) = scope::home(root, &tid).and_then(|home| task(&home, &tid)) else { return false };
    run_at(&t, &rid).is_ok_and(|i| t["runs"][i]["kind"].as_str() == Some("work"))
}

/// `agent_process`'s failure when a clean run made no Solace call: the run
/// starts once more with a fresh agent.
pub(crate) const NO_TOOL: i32 = 5;

/// What a run that never called Solace says, after its one fresh try.
pub(crate) const NO_TOOL_MESSAGE: &str = concat!(
    "The agent made no Solace call: it didn't get the Solace tool, or never used it, so it couldn't work. ",
    "Solace tried again once. Start the run again; if it keeps happening, try another model."
);

/// The tool is `solace` in a Codex log and `mcp__solace__solace` in a Claude
/// log; resource tools such as `list_mcp_resources` share the server but are
/// not the tool.
fn solace_title(title: &str) -> bool {
    ["solace", "mcp__solace", "mcp__solace__solace"]
        .iter()
        .any(|name| title == *name || title.strip_prefix(name).is_some_and(|rest| rest.starts_with(' ')))
}

/// A task run's task and run, from its agent's environment; none in a
/// strategy run.
pub fn context() -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(ids) = TEST_CONTEXT.with(|c| c.borrow().clone()) {
        return Some(ids);
    }
    task_ids(env::var("SOLACE_TASK").ok(), env::var("SOLACE_RUN").ok())
}

/// The task run's ids: both of them, and the task is not a strategy run's.
pub(super) fn task_ids(task: Option<String>, run: Option<String>) -> Option<(String, String)> {
    Some((task.filter(|t| t != STRATEGY_RUN)?, run?))
}

/// A strategy run's id, from its agent's environment.
pub fn strategy_context() -> Option<String> {
    strategy_ids(env::var("SOLACE_TASK").ok(), env::var("SOLACE_RUN").ok()).map(|(_, rid)| rid)
}

/// Waits until the run may act. A `task` command takes no lock: it sends the
/// task server a message, and reads run state from the task file, which is
/// replaced by rename (D-77 point 5). Any other command holds its home's lock
/// and, for a command in another project of its scope (`also`), that
/// project's too (D-61).
pub fn agent_guard(root: &Path, tid: &str, rid: &str, a: &[&str], also: &[&Path]) -> Result<Vec<fs::File>, Failure> {
    loop {
        // A task file is replaced while the server saves it; a check that arrives
        // in that moment waits for it rather than reporting it missing.
        let (guard, t) = scope::retry_task(|| {
            let guard = if a.first() == Some(&"task") {
                vec![]
            } else {
                scope::lock_all(&[root].iter().chain(also).map(|p| p.to_path_buf()).collect::<Vec<_>>())?
            };
            Ok((guard, task(root, tid)?))
        })?;
        let r = &t["runs"][run_at(&t, rid)?];
        if r["state"] == "ended" {
            return err("E7003", "run ended");
        }
        if !t["paused_from"].is_null() {
            drop(guard);
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // The run waits for an answer: an agent that went on without it (its ask was
        // queued) holds still until the human replies, or stops if no server is there to hear.
        if r["state"] == "waiting" && a.first() != Some(&"task") {
            let notes = t["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let replied = r["kind"] == "discuss" && crate::board::delivery::pending_task_reply(r, notes, rid);
            drop(guard);
            if !tasks::serving(&scope::top(root)) {
                return err("E7003", "the run waits for an answer and no board server is running; stop now");
            }
            if replied {
                return err("E7003", "the human replied; call `task wait` to receive it");
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if crate::cli::HOST.contains(&a[0]) {
            return err("E7010", "host command is unavailable in a run");
        }
        if r["kind"] != "work" && a.starts_with(&["drafts", "drop"]) {
            return err("E7010", "this run is read-only");
        }
        // Every read-only run may run the commands a human marked (D-67, D-161).
        if r["kind"] != "work" && a[0] == "run" && a.len() > 1 {
            let marked = lockdown::settings(also.first().copied().unwrap_or(root))
                .ok()
                .and_then(|v| v.get("commands")?.get(a[1])?.get("allowed_in_reviews")?.as_bool())
                .unwrap_or(false);
            if !marked {
                return err("E7010", "this run is read-only: run only works for commands marked allowed_in_reviews in solace.toml");
            }
        } else if r["kind"] != "work" && crate::command_tx::CHANGES.contains(&a[0]) && a[0] != "build" {
            return err("E7010", "this run is read-only");
        }
        // The human may have changed the task's projects while it was paused.
        for p in also {
            let top = scope::top(root);
            let dir = scope::dir(&top, p);
            if !scope::of(&t, &scope::dir(&top, root)).contains(&dir) {
                return lockdown::error("E6007", &dir, "no longer one of this task's projects");
            }
        }
        return Ok(guard);
    }
}

#[cfg(test)]
thread_local! {
    /// A test's task and run, in place of the agent's environment.
    static TEST_CONTEXT: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `context` reading `tid` and `rid`, for a test that drives a
/// task command without an agent's environment.
#[cfg(test)]
pub(crate) fn with_context<T>(tid: &str, rid: &str, f: impl FnOnce() -> T) -> T {
    let old = TEST_CONTEXT.with(|c| c.replace(Some((tid.to_string(), rid.to_string()))));
    let out = f();
    TEST_CONTEXT.with(|c| *c.borrow_mut() = old);
    out
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
    let t = task(&s.root, tid)?;
    let w = flow(&s.root)?;
    let i = run_at(&t, rid)?;
    // A message written into `command` (`task note --text "..."`) arrives cut at
    // its first line; refuse it so the agent resends it whole.
    if matches!(a.get(1).copied(), Some("note" | "done" | "ask")) && a.get(2).is_some_and(|w| w.starts_with("--")) {
        return err(
            "E7015",
            &format!("nothing was sent. Resend with `command` set to just `task {}` and the whole message in the `text` argument", a[1]),
        );
    }
    let discussing = t["runs"][i]["kind"] == "discuss";
    // A wait-only call keeps waiting without sending the same note again.
    if a.get(1) == Some(&"wait") {
        if !discussing {
            return err("E7001", "task wait is only for a discussion");
        }
        drop(guard);
        return wait_discussion(s, tid, rid, Some(crate::board::delivery::wait_limit()));
    }
    let text = if a.len() > 2 { a[2..].join(" ") } else { input()? };
    let top = scope::top(&s.root);
    // The agent sends the task server a message; the server applies it (D-77).
    let mut m = json!({"kind":"agent","task":tid,"run":rid,"command":a.get(1),"text":text});
    match a.get(1).copied() {
        // A discussion's reply waits for the human's next message.
        Some("note") if discussing => {
            let sent = tasks::send(&top, m)?;
            drop(guard);
            if !tasks::applied(&top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(&top, &sent) {
                return err("E7001", REJECTED);
            }
            return wait_discussion(s, tid, rid, Some(crate::board::delivery::wait_limit()));
        }
        Some("done" | "ask") if discussing => return err("E7010", "a discussion goes on until the human ends it; reply with task note"),
        Some("note") => {
            if t["runs"][i]["kind"] == "review" {
                m = review_message(s, tid, &t, rid, &text)?;
                let mut next = t.clone();
                message(&w, &mut next, &m)?;
                let sent = tasks::send(&top, m)?;
                drop(guard);
                return review_reply(&top, &sent);
            }
        }
        Some("done") => {
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "reviews finish with task note");
            }
            if t["runs"][i]["kind"] == "work"
                && let Some(refusal) = draft_refusal(s, &t)?
            {
                return err("E7001", refusal);
            }
            if t["runs"][i]["kind"] == "work"
                && let Some(refusal) = dependent_refusal(s, &t)?
            {
                return err("E7001", refusal);
            }
        }
        Some("ask") => {
            if !(2..=4).contains(&options.len()) {
                return err("E7001", "task ask needs 2–4 options");
            }
            if t["runs"][i]["kind"] == "review" {
                return err("E7010", "review cannot move task with a question");
            }
            m["question"] = json!({"id":unique_id(),"text":text,"options":options});
            message(&w, &mut t.clone(), &m)?;
            let sent = tasks::send(&top, m)?;
            drop(guard);
            if !tasks::applied(&top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(&top, &sent) {
                return err("E7001", REJECTED);
            }
            return wait_reply(s, tid, rid);
        }
        _ => {
            return usage(concat!(
                "task peek <task> [--notes [3,7,10..12]] [--from <n>] | task peek feature | ",
                "task note | task wait | task ask | task done, with the text in `text`; ",
                "task diff [--since-review] [name ...]",
            ));
        }
    }
    // The task as the server will have it: checked here, so a refusal reaches the agent.
    let mut next = t.clone();
    message(&w, &mut next, &m)?;
    // Moving into a final column settles on the server, once it has applied the
    // move (`tasks::settle_later`): the task may have changed since this copy.
    tasks::send(&top, m)?;
    Ok(if tasks::serving(&top) { "Recorded.\n".into() } else { format!("{}\n", tasks::QUEUED) })
}

/// Holds the agent's call until its answers arrive and the run continues. The
/// caller has seen its message applied (`tasks::applied`), or the run would still
/// read `running` here and get the previous reply.
pub(crate) fn wait_reply(s: &Store, tid: &str, rid: &str) -> Out {
    let top = scope::top(&s.root);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // No server, so no answer is coming: return rather than wait forever.
        if !tasks::serving(&top) {
            return err("E7003", "the board server stopped while waiting");
        }
        let t = task(&s.root, tid)?;
        let i = run_at(&t, rid)?;
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
fn wait_discussion(s: &Store, tid: &str, rid: &str, limit: Option<Duration>) -> Out {
    let top = scope::top(&s.root);
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // No server, so the human can't reply: return rather than wait forever.
        if !tasks::serving(&top) {
            return err("E7003", "the board server stopped while waiting");
        }
        if limit.is_some_and(|limit| started.elapsed() > limit) {
            return Ok(crate::board::delivery::WAIT_AGAIN.into());
        }
        let t = task(&s.root, tid)?;
        let i = run_at(&t, rid)?;
        if t["runs"][i]["state"] == "ended" {
            return Ok("The discussion has ended. Stop now, without further calls.\n".into());
        }
        if let Some(reply) = crate::board::delivery::claim_task_reply(&s.root, tid, rid)? {
            return Ok(reply);
        }
    }
}

/// What an agent is told when the task server set its message aside.
pub(crate) const REJECTED: &str = "the board could not apply this message; it was set aside in .solace/tasks/inbox/failed, so nothing was recorded";

pub(crate) fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

/// Why `task done` waits, if a draft of this task is open in any of its projects.
fn draft_refusal(s: &Store, t: &Value) -> Result<Option<String>, Failure> {
    let mut lines = vec![];
    for (dir, id, args, current) in task_drafts(s, t)? {
        let flag = format!(" --project {dir}");
        let retry = if args.first().is_some_and(|a| a == "write") { "write" } else { "submit" };
        let what = if args.is_empty() { "draft".to_string() } else { args.join(" ") };
        let stale = if current { "" } else { ", stale" };
        lines.push(format!(
            "- {id} in {dir} ({what}{stale}): view `view --draft {id}{flag}`; \
             retry `{retry} --draft {id}{flag}`; drop `drafts drop {id}{flag}`"
        ));
    }
    if lines.is_empty() {
        return Ok(None);
    }
    let (noun, verb) = if lines.len() == 1 { ("draft", "holds") } else { ("drafts", "hold") };
    Ok(Some(format!("task done is refused: {} {noun} {verb} unsaved work in the task's projects.\n{}", lines.len(), lines.join("\n"))))
}

/// One line for a failed tool step: its number, command head and first error line.
pub(crate) fn failed_step_line(i: usize, e: &Value) -> String {
    let title = e["title"].as_str().unwrap_or("");
    let output = e["output"].as_str().unwrap_or("");
    let (command, copy) = failed_command(title, output);
    let section = failed_section(output, &command, copy);
    let error = error_line(&section).unwrap_or("(no error text)");
    format!("{} {}: {}", i + 1, cut_line(&command), cut_line(error))
}

/// The command a failed batch names first, with which copy of it failed, or the
/// title's first line for one command, without the tool name or failure marker.
fn failed_command(title: &str, output: &str) -> (String, usize) {
    let listed = output.lines().rev().find_map(|line| line.split_once(" commands failed: ").map(|(_, rest)| rest.trim()));
    let (command, copy) = match listed {
        Some(listed) => command_copy(listed.split("; ").next().unwrap_or("").trim()),
        None => (title.lines().next().unwrap_or(""), 1),
    };
    let command = command.strip_prefix("solace ").unwrap_or(&command);
    match command.strip_prefix('(').and_then(|rest| rest.split_once(") ")) {
        Some((marker, head)) if marker.ends_with(" failed") && marker.contains(" of ") => (head.to_string(), copy),
        _ => (command.to_string(), copy),
    }
}

/// The first error code line, else the first line that is not a batch header or summary.
fn error_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| error_code(line)).or_else(|| {
        output.lines().map(str::trim).find(|line| {
            !line.is_empty() && !line.starts_with("==== solace ") && !line.contains(" commands failed: ") && !line.starts_with("skipped: ")
        })
    })
}

/// Whether a line starts with an error code such as `E0108` or `E6002`.
fn error_code(line: &str) -> bool {
    line.strip_prefix('E').is_some_and(|rest| {
        let mut chars = rest.chars();
        (0..4).all(|_| chars.next().is_some_and(|c| c.is_ascii_digit()))
    })
}

/// A failed command's copy in a batch, up to the next header or summary; all of
/// the output when the failure is not a batch.
fn failed_section(output: &str, command: &str, copy: usize) -> String {
    let header = format!("==== solace {command} ====");
    let start = output
        .match_indices(&header)
        .nth(copy.saturating_sub(1))
        .or_else(|| output.match_indices(&header).next())
        .map(|(at, _)| at + header.len());
    let Some(start) = start else { return output.to_string() };
    output[start..]
        .lines()
        .take_while(|line| {
            let line = line.trim_start();
            !line.starts_with("==== solace ") && !line.contains(" commands failed: ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A batch labels a repeated failed command's copy: `check (copy 2)`.
fn command_copy(command: &str) -> (&str, usize) {
    match command.rsplit_once(" (copy ") {
        Some((command, copy)) => match copy.strip_suffix(')').and_then(|copy| copy.parse().ok()) {
            Some(copy) => (command, copy),
            None => (command, 1),
        },
        None => (command, 1),
    }
}

/// Appends `line` to the log of the run this process serves, if it serves one:
/// where the harness keeps the server's stderr to itself, the log is the only
/// place the board sees why the server ended. Best effort.
pub(crate) fn record_server_end(line: &str) {
    let root = Store::root().ok().or_else(|| env::current_dir().ok()).unwrap_or_default();
    let ids = context().or_else(|| strategy_context().map(|rid| (STRATEGY_RUN.into(), rid)));
    record_server_end_for(&root, ids, line);
}

/// Appends `line` to task or strategy run `rid`'s log, when the run has one.
fn append_to_run_log(root: &Path, tid: &str, rid: &str, line: &str) {
    let run = if tid == STRATEGY_RUN { tasks::strategy::strategy_run(&scope::top(root), rid) } else { run_record(root, tid, rid) };
    let Some(path) = run.as_ref().and_then(|r| r["log"].as_str()) else { return };
    if let Ok(mut log) = fs::OpenOptions::new().append(true).create(true).open(server_end_log(Path::new(path))) {
        let _ = writeln!(log, "{line}");
    }
}

/// A run's server-end lines: beside its log, so an append cannot be overwritten
/// by the agent still writing the log.
fn server_end_log(log: &Path) -> PathBuf {
    log.with_extension("server-end")
}

/// Records a server-end line for the run `ids` names, if any.
fn record_server_end_for(root: &Path, ids: Option<(String, String)>, line: &str) {
    let Some((tid, rid)) = ids else { return };
    append_to_run_log(root, &tid, &rid, line);
}

/// A strategy run's task and run ids from an agent's environment values.
fn strategy_ids(task: Option<String>, run: Option<String>) -> Option<(String, String)> {
    Some((task.filter(|t| t == STRATEGY_RUN)?, run?))
}

fn agent_error(entries: &[Value]) -> Option<&str> {
    entries.iter().rev().find(|e| is_agent_error(e)).and_then(error_message)
}

fn exit_error(entries: &[Value]) -> Option<&str> {
    agent_error(entries).or_else(|| entries.iter().rev().find(|e| e["kind"] == "error").and_then(error_message))
}

fn error_message(entry: &Value) -> Option<&str> {
    entry["text"].as_str().filter(|text| !text.is_empty())
}

/// The transcript tag for a line Solace wrote about itself, not the agent.
pub(crate) const SERVER: &str = "server";

/// Whether a transcript entry is the agent's own failure, rather than a server-end line.
pub(crate) fn is_agent_error(e: &Value) -> bool {
    e["kind"] == "error" && e["source"] != SERVER
}

/// The share of input tokens that came from cache, when the agent reports them.
fn cache_ratio(cached: Option<u64>, input: u64) -> Option<String> {
    let cached = cached?;
    if input == 0 || cached > input {
        return None;
    }
    let percent = (u128::from(cached) * 100 + u128::from(input) / 2) / u128::from(input);
    Some(format!("{percent}% cache hit"))
}

/// A review's note counts as recorded only once the server has applied it.
fn review_reply(top: &Path, sent: &Path) -> Out {
    if !tasks::applied(top, sent) {
        return Ok(format!("{}\n", tasks::QUEUED));
    }
    if tasks::rejected(top, sent) {
        return err(
            "E7001",
            format!(
                "the board set this note aside at {}; the note was not recorded, so resend it",
                tasks::server::failed_path(top, sent).display()
            ),
        );
    }
    Ok("Recorded.\n".into())
}

/// The start of an agent's failure message when its process exited early.
pub(crate) const UNFINISHED_EXIT: &str = "agent exited without finishing";

/// Why `task done` waits, if a covered project depends on this task's projects
/// and still has errors it did not have before. An earlier error is old only
/// when its symbol and kind match, so a new error on an already broken symbol
/// still waits.
fn dependent_refusal(s: &Store, t: &Value) -> Result<Option<String>, Failure> {
    let top = scope::top(&s.root);
    let allowed = scope::of(t, &scope::dir(&top, &s.root));
    let changed: Vec<String> =
        crate::board::reviews::changes_of(s, t)?.iter().filter_map(|c| c["project"].as_str().map(String::from)).collect();
    let mut seen: Vec<String> = vec![];
    let mut lines: Vec<String> = vec![];
    for dir in &allowed {
        if !changed.contains(dir) {
            continue;
        }
        let store = open(&scope::path(&top, dir))?;
        for (dependent, new, all) in store.dependent_errors_since()? {
            if !scope::covers(&top, &allowed, &dependent) {
                continue;
            }
            if seen.contains(&dependent) {
                continue;
            }
            let errors: Vec<String> = match t["dependent_errors"].get(&dependent).and_then(Value::as_array) {
                Some(base) => all.into_iter().filter(|e| !baseline_has(base, e)).map(|e| e.text).collect(),
                None => new.into_iter().map(|e| e.text).collect(),
            };
            if errors.is_empty() {
                continue;
            }
            seen.push(dependent.clone());
            let first = errors.first().cloned().unwrap_or_default();
            let more = if errors.len() > 1 { format!("\n  … {} more", errors.len() - 1) } else { String::new() };
            lines.push(format!("- {dependent}: {first}{more}"));
        }
    }
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "task done is refused: a project this task changed still has errors it did not have before.\n{}\n\nFix them before finishing.",
        lines.join("\n")
    )))
}

/// Whether the task's saved errors for a dependent already hold this error. An
/// entry written before kinds were stored names only a symbol.
fn baseline_has(base: &[Value], e: &crate::store_core::DependentError) -> bool {
    base.iter().any(|b| match b {
        Value::String(symbol) => symbol == &e.symbol,
        Value::Array(pair) => {
            pair.first().and_then(Value::as_str) == Some(e.symbol.as_str()) && pair.get(1).and_then(Value::as_str) == Some(e.kind.as_str())
        }
        _ => false,
    })
}

/// Whether an agent error blocks a clean exit: a fatal one, or one after the reply's last message.
pub(crate) fn has_blocking_agent_error(entries: &[Value]) -> bool {
    let last_message = entries.iter().rposition(|e| e["kind"] == "message");
    entries.iter().enumerate().any(|(i, e)| is_agent_error(e) && (e["fatal"] == true || last_message.is_none_or(|last| i > last)))
}
