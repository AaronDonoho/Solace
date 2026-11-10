use std::time::Duration;
use serde_json::Value;
use serde_json::json;
use std::env;
use std::collections::HashMap;
use std::process::Command;
use std::io::Read;
use rusqlite::Connection;
use std::path::PathBuf;
use std::path::Path;
use std::fs;
use crate::board::task_model::lock;
use crate::board::task_model::open;
use crate::board::task_model::store_dir;
use crate::board::err;
use crate::board::tasks;
use crate::board::task_model::all_tasks;
use crate::board::task_model::adopt_tasks;
use crate::board::task_model::event;
use crate::board::task_model::flow;
use crate::board::task_model::active;
use crate::board::setup::config_path;
use crate::board::setup::models;
use crate::board::runs::stream_task_event;
use crate::board::task_model::homeless_among;
use crate::record_port;
use crate::failure::Failure;
use crate::failure::fail;
use crate::lockdown;
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
use crate::workspace;
use crate::scope;
use crate::fsx;
use crate::view;
use crate::migrate;
use crate::workspace::NewProject;
use std::os::unix::process::CommandExt;
use rusqlite::OptionalExtension;
use crate::board::task_model::task;
use crate::board::task_model::try_lock;
use crate::board::briefs::IMAGE_LIMIT;
use crate::board::briefs::image_type;
use crate::board::briefs::image_path;
use crate::board::briefs::upload;
use crate::board::task_model::valid_id;
use crate::board::task_model::workflow;
use crate::board::task_model::allowed;
use crate::failure::Out;
use crate::extract::ahead;
use crate::outside;
use crate::outside::Mode;
use crate::recipe;
use std::io::Write;
use crate::board::task_model::guard;
use crate::board::task_model::listed;
use crate::board::agents::check;
use crate::board::setup::languages;
use crate::board::agents::refresh_all;
use crate::board::task_model::recover_project;

/// What one `solace serve` shows: the projects of one Solace root (D-57),
/// each held by this server alone. The root's own project (`.`) answers at
/// `/`; the others under `/p/<dir>`.
pub(crate) struct Board {
    pub(crate) top: PathBuf,
    pub(crate) held: std::sync::Mutex<HashMap<PathBuf, fs::File>>,
    /// Serializes changes to the root's project list.
    pub(crate) editing: std::sync::Mutex<()>,
    /// Serializes the watcher's outside-change scan with a migration step (D-58, D-59).
    pub(crate) watching: std::sync::Mutex<()>,
    pub(crate) port: u16,
    /// The migration step running now, and the last one's failure (D-59).
    pub(crate) step: std::sync::Mutex<Option<String>>,
    pub(crate) failed: std::sync::Mutex<Option<String>>,
}

impl Board {
    /// Whether the root lists projects (or is a folder of them), so the board
    /// shows lanes; otherwise it is one project on its own.
    pub(crate) fn explicit(&self) -> bool {
        workspace::is_root(&self.top) || !self.top.join(".solace/store.db").exists()
    }
    /// `(dir, root)` for each set-up project.
    pub(crate) fn projects(&self) -> Result<Vec<(String, PathBuf)>, Failure> {
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
    /// server would end the first one's runs as a restart. `announce` says
    /// whether recovery logs the workflow; a project joining a served board
    /// already has it in its stream's snapshot.
    pub(crate) fn claim(&self, dir: &str, root: &Path, announce: bool) -> Result<(), Failure> {
        let mut held = self.held.lock().unwrap();
        if held.contains_key(root) {
            return Ok(());
        }
        let Some(f) = fsx::try_lock_file(&root.join(".solace/serve.lock"))? else {
            let name = if dir == "." { root.display().to_string() } else { dir.to_string() };
            return fail(format!("{name} is already served by another `solace serve`; stop that one first."));
        };
        record_port(&f, self.port);
        adopt_tasks(&scope::top(root), root)?;
        recover_project(root, announce)?;
        held.insert(root.to_path_buf(), f);
        Ok(())
    }
    /// The project a request is for, and the path within it; `None` for the
    /// root's own routes.
    pub(crate) fn route(&self, url: &str) -> Result<Option<(PathBuf, String)>, Failure> {
        if root_route(url.split('?').next().unwrap_or(""), self.explicit()) {
            return Ok(None);
        }
        let projects = self.projects()?;
        let Some(after) = url.strip_prefix("/p/") else {
            return Ok(projects.into_iter().find(|(d, _)| d == ".").map(|(_, r)| (r, url.to_string())));
        };
        let end = after.find(['/', '?']).unwrap_or(after.len());
        let dir = view::percent_decode(&after[..end]);
        let Some((dir, root)) = projects.into_iter().find(|(d, _)| *d == dir && d != ".") else {
            return err("E7001", format!("unknown project {dir}"));
        };
        self.claim(&dir, &root, false)?;
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
                let needs = match crate::language::id(&root) {
                    Some(id) => match crate::language::reason(id) {
                        Ok(Some(reason)) => reason,
                        Ok(None) if id == crate::language::RUST => fs::read_to_string(root.join(".solace/fetch-failed"))
                            .ok()
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .unwrap_or_default(),
                        Ok(None) => String::new(),
                        Err(e) => e.0,
                    },
                    None => String::new(),
                };
                json!({"dir":dir,"label":label,"name":workspace::name(&root),"target":workspace::target(&root),"base":base,"needs":needs})
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
            // A scan already running must finish before a step moves a project's files.
            let _watching = me.watching_guard();
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
    pub(crate) fn projects_api(self: &std::sync::Arc<Self>, method: &str, url: &str, body: &Value) -> Result<Value, Failure> {
        let path = url.split('?').next().unwrap_or("");
        match (method, path) {
            ("GET", "/projects") => return self.listing(),
            ("GET", "/about") => return Ok(json!({"version": env!("CARGO_PKG_VERSION")})),
            ("GET", "/languages") => return languages(),
            ("GET", "/roles") => return Ok(crate::board::roles::api()),
            ("GET", "/agent-settings") => return crate::board::setup::agent_settings(),
            ("GET", "/role-models") => return crate::board::setup::role_models(),
            ("GET", "/skills") => return Ok(crate::skills::api(&self.top)),
            ("POST", "/skills") => {
                return crate::skills::save(&self.top, body).and_then(|_| Ok(crate::skills::api(&self.top)));
            }
            ("DELETE", p) if p.starts_with("/skills/") => {
                let name = view::percent_decode(&p["/skills/".len()..]);
                return crate::skills::remove(&self.top, &name).and_then(|_| Ok(crate::skills::api(&self.top)));
            }
            ("GET", "/designs") => return crate::design::list(&self.top),
            ("GET", p) if p.strip_prefix("/designs/").and_then(|rest| rest.strip_suffix("/versions")).is_some() => {
                let id = p.strip_prefix("/designs/").and_then(|rest| rest.strip_suffix("/versions")).unwrap_or("");
                return crate::design::versions(&self.top, &view::percent_decode(id));
            }
            ("GET", p) if p.strip_prefix("/designs/").and_then(|rest| rest.split_once("/versions/")).is_some() => {
                let (id, number) = p.strip_prefix("/designs/").and_then(|rest| rest.split_once("/versions/")).unwrap_or(("", ""));
                if !number.bytes().all(|b| b.is_ascii_digit()) {
                    return err("E7001", "version must be a whole number");
                }
                let number = match number.parse::<u64>() {
                    Ok(number) => number,
                    Err(_) => return err("E7001", "version must be a whole number"),
                };
                return crate::design::version(&self.top, &view::percent_decode(id), number);
            }
            ("POST", p) if p.strip_prefix("/designs/").and_then(|rest| rest.strip_suffix("/restore")).is_some() => {
                let id = p.strip_prefix("/designs/").and_then(|rest| rest.strip_suffix("/restore")).unwrap_or("");
                let number = crate::design::number(&body["version"])?;
                return crate::design::restore(&self.top, &view::percent_decode(id), number);
            }
            ("GET", p) if p.starts_with("/designs/") => {
                let id = view::percent_decode(&p["/designs/".len()..]);
                return crate::design::show(&self.top, &id);
            }
            ("GET", "/setup") => return Ok(check()),
            ("POST", "/languages") => return crate::board::setup::save_language(&config_path(), body).and_then(|_| languages()),
            ("POST", "/agent-settings") => {
                return crate::board::setup::save_agent_setting(&config_path(), body).and_then(|_| crate::board::setup::agent_settings());
            }
            ("POST", "/role-models") => {
                crate::board::setup::save_role_model(&config_path(), body)?;
                let saved = crate::board::setup::role_models()?;
                crate::tasks::store::log(&self.top, "", &json!({"roleModels": saved.clone()}));
                return Ok(saved);
            }
            ("GET", "/models") => return models(),
            ("POST", "/models/try-again") => return crate::board::agents::try_again(body).and_then(|_| models()),
            ("POST", "/models") => {
                return crate::board::setup::save(&config_path(), &crate::board::setup::keys_path(), body).and_then(|_| models());
            }
            ("DELETE", p) if p.starts_with("/models/") => {
                let id = view::percent_decode(&p["/models/".len()..]);
                return crate::board::setup::remove(&config_path(), &crate::board::setup::keys_path(), &id).and_then(|_| models());
            }
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
                let project = match &body["language"] {
                    Value::Null => NewProject::Gleam { javascript: body["target"] == "javascript" },
                    Value::String(language) if language == "gleam" => NewProject::Gleam { javascript: body["target"] == "javascript" },
                    Value::String(language) if language == "rust" => NewProject::Rust,
                    other => return err("E7001", format!("language must be \"gleam\" or \"rust\", not {other}")),
                };
                workspace::new_project(&self.top, dir, name, project)?;
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
                if let Some((home, t)) =
                    covering.iter().find(|(home, t)| *home != root && crate::board::task_model::column(&w, t)["final"] != true)
                {
                    return err(
                        "E7013",
                        format!(
                            "the task \"{}\" in {} also covers {dir}; take {dir} out of its projects first",
                            t["title"].as_str().unwrap_or(""),
                            scope::dir(&self.top, home)
                        ),
                    );
                }
                // Deleting the files (D-74) is checked before anything changes.
                let doomed = if body["delete"] == true {
                    Some(workspace::deletable(&self.top, &dir, body["name"].as_str().unwrap_or("").trim())?)
                } else {
                    None
                };
                // Unlist first, so a failed delete still leaves a consistent list.
                workspace::remove(&self.top, &dir)?;
                self.held.lock().unwrap().remove(&root);
                if let Some(path) = doomed {
                    fs::remove_dir_all(path)?;
                }
                return self.listing();
            }
            _ => return err("E7001", "unknown route"),
        };
        let root = scope::path(&self.top, &dir);
        if let Err(e) = self.claim(&dir, &root, false) {
            if dir != "." {
                workspace::remove(&self.top, &dir)?;
            }
            return Err(e);
        }
        self.listing()
    }

    pub(crate) fn new(top: PathBuf, port: u16) -> Self {
        Board {
            top,
            held: Default::default(),
            editing: Default::default(),
            watching: Default::default(),
            port,
            step: Default::default(),
            failed: Default::default(),
        }
    }

    /// The watcher's scan guard. The lock guards no data, so a panic during a
    /// scan must not keep a migration step from taking it.
    pub(crate) fn watching_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.watching.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|c| if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") })
        .collect()
}

/// Whether a request names this server's own address: `Host` (when sent) and
/// `Origin` (when sent) are `127.0.0.1` or `localhost` with the server's port.
pub(crate) fn own_address(req: &tiny_http::Request, port: u16) -> bool {
    let named = |field: &str, prefix: &str| {
        req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(field)).is_none_or(|h| {
            let v = h.value.as_str().to_ascii_lowercase();
            v.strip_prefix(prefix).is_some_and(|a| a == format!("127.0.0.1:{port}") || a == format!("localhost:{port}"))
        })
    };
    named("Host", "") && named("Origin", "http://")
}

/// The largest request body the API reads, besides image uploads.
pub(crate) const BODY_LIMIT: usize = 1_048_576;

/// A request's body, or `None` when it's over `limit` bytes or can't be read.
pub(crate) fn read_body(req: &mut tiny_http::Request, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let read = req.as_reader().take(limit as u64 + 1).read_to_end(&mut bytes);
    (read.is_ok() && bytes.len() <= limit).then_some(bytes)
}

/// A JSON request body; an empty one is `{}`.
pub(crate) fn body_json(bytes: &[u8]) -> Result<Value, Failure> {
    if bytes.is_empty() { Ok(json!({})) } else { serde_json::from_slice(bytes).or_else(|_| err("E7001", "invalid JSON")) }
}

/// An API error as the board reads it: its code and message.
pub(crate) fn error_json(e: &Failure) -> Value {
    let first = e.0.split_whitespace().next().unwrap_or("");
    let code = match first.strip_prefix('E') {
        Some(digits) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => first,
        _ => "E7001",
    };
    json!({"code":code,"message":e.0})
}

pub(crate) fn respond_json(req: tiny_http::Request, result: Result<Value, Failure>) {
    match result {
        Ok(v) => respond(req, 200, v),
        Err(e) => respond(req, failure_status(&e), error_json(&e)),
    }
}

pub(crate) fn respond(req: tiny_http::Request, status: u16, value: Value) {
    let _ = req.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
}

/// The `/events` stream: one project's events, or in a workspace's merged
/// stream every project's, each naming its `project`, with a `{projects}`
/// event when the list changes. A merged stream's event id holds each
/// project's position as JSON (D-57). A new stream starts from each project's
/// workflow and tasks as they are, then follows the changes (D-78).
pub(crate) struct Events {
    /// Whether the snapshot a new stream starts with is still to be sent.
    pub(crate) fresh: bool,
    pub(crate) board: std::sync::Arc<Board>,
    pub(crate) merged: bool,
    /// The position in the task log, shared by every project (D-77).
    pub(crate) tasks: i64,
    /// `(dir, root, seq)`: each project's position in its workflow events.
    pub(crate) streams: Vec<(String, PathBuf, i64)>,
    /// Whether this connection has sent `ready`, the end of what it missed.
    pub(crate) caught_up: bool,
    pub(crate) bytes: std::io::Cursor<Vec<u8>>,
}

impl Events {
    /// An event id is `{"tasks": n, dir: seq, ...}`; any other starts from the beginning.
    pub(crate) fn new(board: std::sync::Arc<Board>, root: Option<PathBuf>, last: Option<&str>) -> Events {
        let seen: HashMap<String, i64> = last.and_then(|l| serde_json::from_str(l).ok()).unwrap_or_default();
        let at = |d: &str| seen.get(d).copied().unwrap_or(0);
        let tasks = at("tasks");
        let fresh = seen.is_empty() || task_position_stale(&board.top, tasks);
        let (merged, streams) = match root {
            Some(root) => {
                let d = scope::dir(&board.top, &root);
                (false, vec![(d.clone(), root, at(&d))])
            }
            None => (true, board.projects().unwrap_or_default().into_iter().map(|(d, r)| (d.clone(), r, at(&d))).collect()),
        };
        Events { fresh, board, merged, tasks, streams, caught_up: false, bytes: Default::default() }
    }
    /// What a new stream starts from: the log positions first, then the files,
    /// so a change made in between is seen again as an event, never missed.
    fn snapshot(&mut self) -> std::io::Result<String> {
        let top = self.board.top.clone();
        self.tasks = if store_dir(&top).is_dir() {
            let db = crate::tasks::events(&top).map_err(|e| std::io::Error::other(e.0))?;
            db.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0)).map_err(std::io::Error::other)?
        } else {
            0
        };
        for stream in &mut self.streams {
            stream.2 = Connection::open(stream.1.join(".solace/store.db"))
                .and_then(|db| {
                    db.busy_timeout(Duration::from_secs(10))?;
                    db.query_row("SELECT COALESCE(MAX(seq),0) FROM board_events", [], |row| row.get(0))
                })
                .unwrap_or(0);
        }
        Ok(self.state_of(&(0..self.streams.len()).collect::<Vec<_>>()) + &self.strategy_state())
    }
    /// Whether this stream carries the root's own records: the merged stream, or
    /// the root project's (D-87 point 7), never `/p/{dir}/events`.
    fn at_root(&self) -> bool {
        self.merged || self.streams.iter().any(|s| s.0 == ".")
    }
    /// The root's milestones, features, vision, personas, role models and
    /// strategy runs as they are, after the tasks.
    fn strategy_state(&self) -> String {
        if !self.at_root() {
            return String::new();
        }
        let (id, top) = (self.id(), &self.board.top);
        let (milestones, features) =
            (json!({"milestones":crate::tasks::records(top, "milestones")}), json!({"features":crate::tasks::records(top, "features")}));
        let vision = json!({"vision": crate::tasks::vision(top)});
        let personas = json!({"personas": crate::tasks::personas(top)});
        let role_models = match crate::board::setup::role_models() {
            Ok(models) => format!("id: {id}\ndata: {}\n\n", json!({"roleModels": models})),
            Err(_) => String::new(),
        };
        let runs = json!({"strategyRuns": crate::tasks::strategy_runs(top)});
        let design_runs = json!({"designRuns": crate::tasks::strategy::design_runs(top)});
        let designs = match crate::design::listed(top) {
            Ok(designs) => format!("id: {id}\ndata: {}\n\n", json!({"designs": designs})),
            Err(_) => String::new(),
        };
        // Tasks with no project (D-129) ride on the root's streams, marked so
        // the Strategy page can list them whatever their home.
        let homeless = json!({"tasks": all_tasks(top).unwrap_or_default().into_iter().filter(|t| t["noProject"] == true).collect::<Vec<_>>(), "noProject": true});
        format!(
            "id: {id}\ndata: {milestones}\n\nid: {id}\ndata: {features}\n\nid: {id}\ndata: {vision}\n\nid: {id}\ndata: {personas}\n\n{role_models}id: {id}\ndata: {runs}\n\nid: {id}\ndata: {design_runs}\n\n{designs}id: {id}\ndata: {homeless}\n\n"
        )
    }
    /// The workflow and tasks of the streams at `which`, as events.
    fn state_of(&self, which: &[usize]) -> String {
        let (id, merged) = (self.id(), self.merged);
        let mut out = String::new();
        for &i in which {
            let (dir, root, _) = &self.streams[i];
            let mut add = |mut v: Value| {
                if merged {
                    v["project"] = json!(dir);
                }
                out += &format!("id: {id}\ndata: {v}\n\n");
            };
            if let Ok(w) = flow(root) {
                add(json!({"workflow":w}));
            }
            if let Ok(ts) = listed(root) {
                add(json!({"tasks":ts}));
            }
        }
        out
    }
    fn id(&self) -> String {
        let mut id: serde_json::Map<_, _> = self.streams.iter().map(|(d, _, s)| (d.clone(), json!(s))).collect();
        id.insert("tasks".into(), json!(self.tasks));
        json!(id).to_string()
    }
    /// The next task event this stream shows, past those of other projects.
    fn next_task(&mut self) -> std::io::Result<Option<String>> {
        if !store_dir(&self.board.top).is_dir() {
            return Ok(None);
        }
        let db = crate::tasks::events(&self.board.top).map_err(|e| std::io::Error::other(e.0))?;
        if task_log_stale(&db, self.tasks).map_err(|e| std::io::Error::other(e.to_string()))? {
            self.fresh = true;
            return Ok(None);
        }
        let members = workspace::members(&self.board.top);
        loop {
            let row = db
                .query_row("SELECT seq,project,data FROM events WHERE seq>? ORDER BY seq LIMIT 1", [self.tasks], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })
                .optional()
                .map_err(std::io::Error::other)?;
            let Some((seq, project, data)) = row else {
                let last: Option<i64> = db.query_row("SELECT MAX(seq) FROM events", [], |r| r.get(0)).map_err(std::io::Error::other)?;
                if last.is_some_and(|last| self.tasks > last) {
                    self.fresh = true;
                }
                return Ok(None);
            };
            // A prune between polls leaves a gap: rebuilding from the files is
            // the only way to include what the stream no longer holds.
            if seq != self.tasks + 1 {
                self.fresh = true;
                return Ok(None);
            }
            self.tasks = seq;
            // A row with no project is a root record's (D-87); a row whose
            // project is no member is a task with no project (D-129), shown
            // on the root's streams and marked.
            let homeless = !project.is_empty() && homeless_among(&members, &project);
            let shown = if project.is_empty() {
                self.at_root()
            } else {
                self.streams.iter().any(|s| s.0 == project) || (self.at_root() && homeless)
            };
            if !shown {
                continue;
            }
            let mut v: Value = serde_json::from_str(&data).map_err(std::io::Error::other)?;
            if self.merged && !project.is_empty() {
                v["project"] = json!(project);
            }
            if homeless {
                v["noProject"] = json!(true);
            }
            let home = v.get("task").and_then(|t| t.get("home")).and_then(Value::as_str).unwrap_or(".");
            let has_project = !homeless_among(&members, home);
            let v = stream_task_event(&self.board.top, v, has_project);
            return Ok(Some(format!("id: {}\ndata: {v}\n\n", self.id())));
        }
    }
    /// Follows a change to the workspace's list: new projects start at their
    /// latest event, since the board loads them whole.
    fn members_changed(&mut self) -> std::io::Result<Option<String>> {
        let now = self.board.projects().map_err(|e| std::io::Error::other(e.0))?;
        if now.iter().map(|p| &p.0).eq(self.streams.iter().map(|s| &s.0)) {
            return Ok(None);
        }
        let old = std::mem::take(&mut self.streams);
        let mut added = vec![];
        for (d, r) in now {
            let seq = match old.iter().find(|s| s.0 == d) {
                Some(s) => s.2,
                None => {
                    added.push(self.streams.len());
                    Connection::open(r.join(".solace/store.db"))
                        .and_then(|db| {
                            db.busy_timeout(Duration::from_secs(10))?;
                            db.query_row("SELECT COALESCE(MAX(seq),0) FROM board_events", [], |row| row.get(0))
                        })
                        .unwrap_or(0)
                }
            };
            self.streams.push((d, r, seq));
        }
        // The new projects' state follows the list: their positions were read before their files.
        let dirs: Vec<&String> = self.streams.iter().map(|s| &s.0).collect();
        let list = format!("id: {}\ndata: {}\n\n", self.id(), json!({"projects":dirs}));
        Ok(Some(list + &self.strategy_state() + &self.state_of(&added)))
    }

    /// Fills `out` with the next event, or waits for one. An error here ends
    /// the stream: the browser must see the connection close so it reconnects.
    fn next(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.bytes.read(out)?;
            if n > 0 {
                return Ok(n);
            }
            if self.fresh {
                self.fresh = false;
                let text = self.snapshot()?;
                if !text.is_empty() {
                    self.bytes = std::io::Cursor::new(text.into_bytes());
                    return self.bytes.read(out);
                }
            }
            for _ in 0..10 {
                if self.merged
                    && let Some(text) = self.members_changed()?
                {
                    self.bytes = std::io::Cursor::new(text.into_bytes());
                    return self.bytes.read(out);
                }
                for i in 0..self.streams.len() {
                    let (dir, root, seq) = &self.streams[i];
                    // Opened on each poll, not kept: a kept connection would follow a store
                    // that adopting or undoing a migration moved away.
                    let db = Connection::open(root.join(".solace/store.db")).map_err(std::io::Error::other)?;
                    db.busy_timeout(Duration::from_secs(10)).map_err(std::io::Error::other)?;
                    let row = db
                        .query_row("SELECT seq,data FROM board_events WHERE seq>? ORDER BY seq LIMIT 1", [*seq], |r| {
                            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                        })
                        .optional()
                        .map_err(std::io::Error::other)?;
                    if let Some((seq, data)) = row {
                        let dir = dir.clone();
                        self.streams[i].2 = seq;
                        let mut v: Value = serde_json::from_str(&data).map_err(std::io::Error::other)?;
                        if self.merged {
                            v["project"] = json!(dir);
                        }
                        self.bytes = std::io::Cursor::new(format!("id: {}\ndata: {v}\n\n", self.id()).into_bytes());
                        return self.bytes.read(out);
                    }
                }
                // Workflow events first, so a replay draws the columns before their tasks.
                if let Some(text) = self.next_task()? {
                    self.bytes = std::io::Cursor::new(text.into_bytes());
                    return self.bytes.read(out);
                }
                // A prune during this stream left a gap: start over from the files.
                if self.fresh {
                    break;
                }
                if !self.caught_up {
                    self.caught_up = true;
                    self.bytes = std::io::Cursor::new(b"event: ready\ndata: {}\n\n".to_vec());
                    return self.bytes.read(out);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if self.fresh {
                continue;
            }
            self.bytes = std::io::Cursor::new(b": keepalive\n\n".to_vec());
            return self.bytes.read(out);
        }
    }
}

impl Read for Events {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.next(out)
    }
}

/// Whether a stream's task position is outside what the log still holds: its
/// older rows were pruned, or the log restarted its numbering. Either way the
/// stream starts from the files again (D-78).
fn task_log_stale(db: &Connection, tasks: i64) -> rusqlite::Result<bool> {
    let (first, last): (Option<i64>, Option<i64>) =
        db.query_row("SELECT MIN(seq), MAX(seq) FROM events", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(first.zip(last).is_some_and(|(first, last)| tasks < first - 1 || tasks > last))
}

/// `task_log_stale` for a starting stream; a log that can't be read leaves the
/// position usable, as before pruning existed.
fn task_position_stale(top: &Path, tasks: i64) -> bool {
    store_dir(top).is_dir() && crate::tasks::events(top).is_ok_and(|db| task_log_stale(&db, tasks).unwrap_or(false))
}

fn path_of(url: &str) -> Vec<&str> {
    url.split('?').next().unwrap_or("").trim_matches('/').split('/').collect()
}

pub fn api(root: &Path, method: &str, url: &str, body: Value) -> Result<Value, Failure> {
    let (path, query_text) = url.split_once('?').unwrap_or((url, ""));
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    // Tasks are the task server's (D-77).
    if parts.first() == Some(&"tasks") {
        return tasks::api(root, method, &parts, body);
    }
    // Task routes lock only a change to code, in `tasks::api` (D-77 point 5); the rest lock every change here.
    // Reads skip the project lock, so the board stays responsive while an agent command holds it.
    // Task files are replaced by rename and the store waits out commits, so a read sees whole states.
    let _guard = if method == "GET" { None } else { Some(guard(root, path_of(url), &body)?) };
    let s = open(root)?;
    let w = flow(root)?;
    let query: HashMap<String, String> =
        query_text.split('&').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if matches!(parts.first(), Some(&"graph" | &"nodes" | &"tests" | &"files")) {
        return view::api(&s, method, &parts, &query, &body);
    }
    match (method, parts.as_slice()) {
        ("GET", ["workflow"]) => Ok(w),
        ("GET", ["models"]) => models(),
        ("POST", ["models", "try-again"]) => crate::board::agents::try_again(&body).and_then(|_| models()),
        _ => err("E7001", "unknown route"),
    }
}

pub(super) const INDEX_HTML: &[u8] = include_bytes!("../../frontend/index.html");

pub(super) const SANS_400: &[u8] = include_bytes!("../../board/priv/fonts/ibm-plex-sans-latin-400-normal.woff2");

pub(super) const SANS_700: &[u8] = include_bytes!("../../board/priv/fonts/ibm-plex-sans-latin-700-normal.woff2");

pub(super) const MONO_400: &[u8] = include_bytes!("../../board/priv/fonts/ibm-plex-mono-latin-400-normal.woff2");

pub(super) const APP_JS: &[u8] = include_bytes!("../../frontend/priv/app.js");
pub fn serve(top: PathBuf, port: u16) -> Out {
    if let Some(done) = migrate::recover_here(&top)? {
        return Ok(format!("{done}Run `solace serve` again from there.\n"));
    }
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| Failure(e.to_string(), 1))?;
    let port = server.server_addr().to_ip().map_or(port, |a| a.port());
    let board = std::sync::Arc::new(crate::board::server::Board::new(top, port));
    for (dir, root) in board.projects()? {
        board.claim(&dir, &root, true)?;
    }
    // Once every project's tasks are in the root's store, the task server
    // applies what its inbox holds and takes over (D-77).
    tasks::start(&board.top)?;
    tasks::end_strategy_runs(&board.top)?;
    // Tasks stored before D-134 in Backlog under an active feature move now.
    tasks::plan_active(&board.top)?;
    // Ask each agent which models it offers, so the list is ready when asked.
    std::thread::spawn(refresh_all);
    // A migration's folder is held too, so a shell can't move it from under the board.
    let _migration = match migrate::load(&board.top) {
        Some(_) if !board.held.lock().unwrap().contains_key(&board.top) => {
            let Some(f) = fsx::try_lock_file(&board.top.join(".solace/serve.lock"))? else {
                return fail(format!("{} is already served by another `solace serve`; stop that one first.", board.top.display()));
            };
            record_port(&f, board.port);
            Some(f)
        }
        _ => None,
    };
    println!("Solace listening on http://{}", server.server_addr());
    if board.explicit() {
        println!("Solace root {}: {} projects", board.top.display(), board.projects()?.len());
    }
    std::io::stdout().flush()?;
    let mut designs = crate::design::listed(&board.top).ok();
    let watcher = board.clone();
    std::thread::spawn(move || {
        let mut previous: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
        let mut flows: HashMap<PathBuf, Option<Value>> = HashMap::new();
        let mut tick = 0u64;
        loop {
            tick += 1;
            std::thread::sleep(Duration::from_millis(100));
            if watcher.step.lock().unwrap().is_some() {
                continue;
            }
            crate::design::announce_change(&watcher.top, &mut designs);
            for (_, root) in watcher.projects().unwrap_or_default() {
                // About once a second, read in code changed outside Solace (D-58).
                if tick % 10 == 1 {
                    let _watching = watcher.watching_guard();
                    if let Some(_guard) = try_lock(&root)
                        && watcher.step.lock().unwrap().is_none()
                        // an extract's half-written files are not outside changes
                        && !ahead::waiting(&root)
                        && let Ok(s) = open(&root)
                    {
                        let _ = outside::absorb(&s, Mode::Notice);
                    }
                }
                let current = fs::read(root.join("solace.toml")).ok();
                let changed = match previous.insert(root.clone(), current.clone()) {
                    None => {
                        flows.insert(root.clone(), lockdown::settings(&root).and_then(|v| workflow(&v)).ok());
                        continue;
                    }
                    Some(old) => old != current,
                };
                if !changed {
                    continue;
                }
                let w = lockdown::settings(&root).and_then(|v| workflow(&v)).ok();
                if flows.insert(root.clone(), w.clone()).is_some_and(|old| old == w) {
                    continue;
                }
                if let Some(w) = w
                    && let Ok(_guard) = lock(&root)
                    && let Ok(s) = open(&root)
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
            // Only the server's own localhost address may ask (D-80): a page on
            // another site, or a rebound name, is refused before any route.
            if !own_address(&req, board.port) {
                return respond(req, 403, error_json(&Failure("E7016 request is not from this server's own address".into(), 1)));
            }
            // The app's watchdog asks this: a task server busy with a slow
            // operation answers `503`, one that has stopped answers `500`.
            if req.method().as_str() == "GET" && path == "/health" {
                use crate::tasks::server::Answering;
                let answer = crate::tasks::server::responding(&board.top, Duration::from_millis(800));
                return match answer {
                    Answering::Yes => respond(req, 200, json!({"ok": true})),
                    Answering::Busy => respond(req, 503, json!({"ok": false, "busy": true})),
                    Answering::No => respond(req, 500, json!({"ok": false})),
                };
            }
            if req.method().as_str() != "GET" && !path.starts_with("/migration") && board.step.lock().unwrap().is_some() {
                return respond_json(req, err("E7003", "a migration step is running"));
            }
            // Milestones, features (D-87), the vision (D-91), personas (D-216) and
            // strategy runs (D-93) are the root's: never under `/p/{dir}`.
            let strategy: Vec<&str> = path.trim_matches('/').split('/').collect();
            // A task by id, whichever project stores it (D-98): a task link asks where to open it.
            if let ("GET", ["tasks", "find", tid]) = (req.method().as_str(), strategy.as_slice()) {
                return match tasks::find_task(&board.top, tid) {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            // A feature or milestone by id (D-122): a link to it asks for its title.
            if let ("GET", [kind @ ("features" | "milestones"), "find", id]) = (req.method().as_str(), strategy.as_slice()) {
                return match tasks::find_record(&board.top, kind, id) {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            // A design by id (D-234): a link to it asks for its title and tab.
            if let ("GET", ["designs", "find", id]) = (req.method().as_str(), strategy.as_slice()) {
                return match crate::design::find(&board.top, id) {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            // Every project's build recipe, in build order (D-131).
            if let ("GET", ["build", "plan"]) = (req.method().as_str(), strategy.as_slice()) {
                return match recipe::plan_json(&board.top) {
                    Ok(v) => respond(req, 200, v),
                    Err(e) => respond(req, 409, error_json(&e)),
                };
            }
            let runs = strategy.len() >= 2 && strategy[..2] == ["strategy", "runs"];
            let design_runs = strategy.len() >= 2 && strategy[..2] == ["design", "runs"];
            if matches!(strategy[0], "milestones" | "features" | "personas") || strategy == ["vision"] || runs || design_runs {
                let result = match read_body(&mut req, BODY_LIMIT) {
                    Some(bytes) => body_json(&bytes).map_err(|e| (409, e)).and_then(|v| match strategy[0] {
                        "vision" => tasks::vision_api(&board.top, req.method().as_str(), v),
                        "personas" => tasks::personas_api(&board.top, req.method().as_str(), &strategy[1..], v),
                        "design" => {
                            let started = req.method().as_str() == "POST" && strategy.len() == 2;
                            tasks::strategy::design_runs_api(&board.top, req.method().as_str(), &strategy[2..], v).inspect(|run| {
                                // A new run's agent starts now.
                                if started {
                                    crate::board::strategy::start_design(board.top.clone(), run["id"].as_str().unwrap_or("").to_string());
                                }
                            })
                        }
                        "strategy" => {
                            let started = req.method().as_str() == "POST" && strategy.len() == 2;
                            tasks::strategy_runs_api(&board.top, req.method().as_str(), &strategy[2..], v).inspect(|run| {
                                // A new run's agent starts now.
                                if started {
                                    crate::board::strategy::start_strategy(board.top.clone(), run["id"].as_str().unwrap_or("").to_string());
                                }
                            })
                        }
                        _ => tasks::strategy_api(&board.top, req.method().as_str(), &strategy, v),
                    }),
                    None => err("E7001", "request body too large").map_err(|e| (409, e)),
                };
                return match result {
                    Ok(v) => respond(req, 200, v),
                    Err((status, e)) => respond(req, status, error_json(&e)),
                };
            }
            let routed = board.route(&full);
            let (root, url) = match routed {
                Err(e) => return respond_json(req, Err(e)),
                Ok(Some((root, url))) => (Some(root), url),
                Ok(None) => (None, full.clone()),
            };
            if url.split('?').next() == Some("/events") {
                let header = req.headers().iter().find(|h| h.field.equiv("Last-Event-ID")).map(|h| h.value.as_str().to_string());
                let last = events_position(&full, header.as_deref());
                let reader = crate::board::server::Events::new(board.clone(), root.clone(), last.as_deref());
                crate::board::server::respond_events(req, reader);
                return;
            }
            // A link's address, `/task/{id}` (D-98), `/feature/{id}`, `/milestone/{id}`
            // (D-122) or `/design/{id}` (D-234), is the shell's to resolve.
            let page = url.split('?').next().unwrap_or("");
            let record_link = ["/task/", "/feature/", "/milestone/", "/design/"].iter().any(|p| page.strip_prefix(p).is_some_and(valid_id));
            // The guide is the shell's Knowledgebase page (D-140): its old address sends the browser there.
            if req.method().as_str() == "GET" && page == "/guide" {
                let _ = req.respond(
                    tiny_http::Response::empty(301).with_header(tiny_http::Header::from_bytes("Location", "/knowledgebase").unwrap()),
                );
                return;
            }
            if req.method().as_str() == "GET"
                && (record_link
                    || matches!(
                        page,
                        "/" | "/workspace"
                            | "/migrate"
                            | "/contracts"
                            | "/strategy"
                            | "/design"
                            | "/settings"
                            | "/knowledgebase"
                            | "/app.js"
                            | "/fonts/sans-400.woff2"
                            | "/fonts/sans-700.woff2"
                            | "/fonts/mono-400.woff2"
                    ))
            {
                let (data, kind) = match page {
                    _ if record_link => (INDEX_HTML, "text/html"),
                    "/" | "/workspace" | "/migrate" | "/contracts" | "/strategy" | "/design" | "/settings" | "/knowledgebase" => {
                        (INDEX_HTML, "text/html")
                    }
                    "/fonts/sans-400.woff2" => (SANS_400, "font/woff2"),
                    "/fonts/sans-700.woff2" => (SANS_700, "font/woff2"),
                    "/fonts/mono-400.woff2" => (MONO_400, "font/woff2"),
                    _ => (APP_JS, "text/javascript"),
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

/// Whether the root answers `path` itself rather than sending it to the root
/// project when the root is one. Every route `projects_api` serves belongs
/// here, and `/events` when the board has several projects.
fn root_route(path: &str, explicit: bool) -> bool {
    path == "/projects"
        || path == "/about"
        || path == "/languages"
        || path == "/roles"
        || path == "/agent-settings"
        || path == "/role-models"
        || path == "/skills"
        || path.starts_with("/skills/")
        || path == "/designs"
        || path.starts_with("/designs/")
        || path == "/setup"
        || path == "/models"
        || path.starts_with("/models/")
        || path.starts_with("/projects/")
        || path == "/migration"
        || path.starts_with("/migration/")
        || (path == "/events" && explicit)
}

/// Answers `/events` with the stream as the response body: when the stream
/// ends, the response ends too, so the browser reconnects instead of waiting
/// on a connection that stays open but silent.
pub(crate) fn respond_events(req: tiny_http::Request, mut reader: impl std::io::Read) {
    let mut writer = req.into_writer();
    let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Cache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\
                 Connection: close\r\n\r\n";
    if writer.write_all(head).and_then(|_| writer.flush()).is_err() {
        return;
    }
    let mut buf = [0; 8192];
    loop {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(&mut buf)));
        match read {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                let chunk = &buf[..n];
                let framed = format!("{:X}\r\n", chunk.len());
                if writer
                    .write_all(framed.as_bytes())
                    .and_then(|_| writer.write_all(chunk))
                    .and_then(|_| writer.write_all(b"\r\n"))
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    return;
                }
            }
            Ok(Err(e)) => {
                log_event_error(&e.to_string());
                break;
            }
            Err(_) => {
                eprintln!("events stream panicked");
                break;
            }
        }
    }
    // The terminal chunk ends the response, so the browser closes the stream
    // and reconnects even though tiny_http keeps the socket for another request.
    let _ = writer.write_all(b"0\r\n\r\n").and_then(|_| writer.flush());
}

/// The position a reconnect asks for: the `Last-Event-ID` header a native
/// reconnect sends, or the `?last=` query a page names when it opens a new
/// stream. The header wins when both are present.
fn events_position(full: &str, header: Option<&str>) -> Option<String> {
    header.map(str::to_string).or_else(|| {
        full.split_once('?')
            .map(|(_, query)| query)
            .unwrap_or("")
            .split('&')
            .find_map(|pair| pair.strip_prefix("last="))
            .map(view::percent_decode)
    })
}

/// Whether to print `reason` now: each reason is news once and then worth
/// repeating only after `quiet` has passed, whatever other reasons came between.
fn note_event_error(last: &mut Vec<(String, std::time::Instant)>, reason: &str, now: std::time::Instant, quiet: Duration) -> bool {
    const KEPT: usize = 8;
    if let Some((_, at)) = last.iter_mut().find(|(old, _)| old == reason) {
        if now.duration_since(*at) < quiet {
            return false;
        }
        *at = now;
        return true;
    }
    if last.len() >= KEPT {
        if let Some((i, _)) = last.iter().enumerate().min_by_key(|(_, (_, at))| *at) {
            last.remove(i);
        }
    }
    last.push((reason.to_string(), now));
    true
}

/// Prints an event-stream failure, keeping a browser's reconnect loop from
/// printing the same line every few seconds.
fn log_event_error(reason: &str) {
    static LAST: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());
    let mut last = LAST.lock().unwrap();
    if note_event_error(&mut last, reason, std::time::Instant::now(), Duration::from_secs(60)) {
        eprintln!("events stream ended: {reason}");
    }
}

fn failure_status(e: &Failure) -> u16 {
    if e.0.starts_with("E7023") { 404 } else { 409 }
}
