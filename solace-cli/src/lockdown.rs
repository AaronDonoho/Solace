use super::*;

use serde_json::Value;
use serde_json::json;
use crate::cli::module_elsewhere;
#[cfg(test)] use crate::cli::with_root;
pub(super) use crate::lockdown::commands::trim_output;
pub(super) use crate::lockdown::commands::kill_group;
pub(super) use crate::lockdown::commands::wait_or_kill;
pub(super) use crate::lockdown::commands::command;
pub(super) use crate::lockdown::commands::passed;
use crate::lockdown::settings::in_skills;
pub(super) mod commands;
pub(super) mod settings;
pub(super) mod paths;
pub(super) mod files;
pub(super) mod recovery;
pub(super) use crate::lockdown::settings::settings;
pub(super) use crate::lockdown::settings::parse_settings;
pub(super) use crate::lockdown::settings::cargo_settings_changed;
pub(super) use crate::lockdown::settings::DEFAULT_SKILLS;
pub(super) use crate::lockdown::settings::skills_dir;
use crate::lockdown::paths::tier_and_bytes;
use crate::lockdown::paths::reachable;
use crate::lockdown::paths::elsewhere;
use crate::lockdown::paths::not_found;
use crate::lockdown::paths::not_here;
pub(super) use crate::lockdown::paths::path;
pub(super) use crate::lockdown::paths::Ignores;
pub(super) use crate::lockdown::paths::tier;
pub(super) use crate::lockdown::paths::watched_paths;
pub(super) use crate::lockdown::paths::graph_paths;
pub(super) use crate::lockdown::paths::graph_error;
#[cfg(test)] use crate::lockdown::files::GREP_CUT;
use crate::lockdown::files::read_part;
#[cfg(test)] use crate::lockdown::files::edited;
use crate::lockdown::files::missing;
use crate::lockdown::files::patch_with_draft;
use crate::lockdown::files::tree;
use crate::lockdown::files::listed;
use crate::lockdown::files::measure;
use crate::lockdown::files::move_files;
#[cfg(test)] use crate::lockdown::files::GREP_SYNTAX;
use crate::lockdown::files::grep;
pub(super) use crate::lockdown::files::change_file;
pub(super) use crate::lockdown::files::move_args;
pub(super) use crate::lockdown::files::carry_file;
pub(super) use crate::lockdown::files::tree_files;
use crate::lockdown::recovery::same_run;
pub(super) use crate::lockdown::recovery::recover;
#[cfg(test)] pub(super) use crate::lockdown::recovery::mark;
pub(super) use crate::lockdown::recovery::pending_for;
pub(super) use crate::lockdown::recovery::drop_records;
pub(super) use crate::lockdown::settings::WATCH_LIMIT;
pub fn error<T>(code: &str, at: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    fail(format!("{code} command refused\n  at {at}\n  {message}\n"))
}

/// A symbol command in a project of plain files (D-60).
pub fn plain<T>(store: &Store, tid: Option<&str>, command: &str, name: Option<&str>) -> Result<T, Failure> {
    if command == "format" && language::is_rust(&store.root) {
        return error(
            "E6013",
            command,
            concat!(
                "`format` needs a module in a Rust project: name one, e.g. `format crate`. ",
                "The whole crate isn't rustfmt-clean, so formatting it in one call would bury ",
                "a task's real change (D-158 point 5)."
            ),
        );
    }
    let use_ = if board::strategy_context().is_some() { "files, grep and read" } else { "files, grep, read, write and run" };
    // D-121: Rust and Python files are read by item.
    let files = if matches!(command, "view" | "inspect" | "refs") {
        "; a .rs or .py file: `view <file>` lists its items, `view <file>::<name>` shows one"
    } else {
        ""
    };
    let what = if language::is_rust(&store.root) {
        format!("this Rust project has no code that `{command}` handles")
    } else {
        "this project has no code held as symbols".to_string()
    };
    let mut out = format!("{what}; use {use_}{files}.");
    if let Some(name) = name
        && matches!(command, "view" | "inspect" | "refs")
    {
        let module = name.split_once('.').map_or(name, |(module, _)| module);
        if let Some(project) = module_elsewhere(&store.root, module) {
            out += &format!(" Module `{module}` is in project `{project}`; add `--project {project}`.");
        }
    }
    if let Some(projects) = tid.and_then(|tid| scope::of_task(&store.root, tid)) {
        let top = scope::top(&store.root);
        let here = scope::dir(&top, &store.root);
        let names: Vec<String> = projects.iter().map(|p| if p.as_str() == "." { ". (the root)".to_string() } else { p.clone() }).collect();
        out += &format!(" This task can change: {}.", names.join(", "));
        if let Some(other) = projects.iter().find(|p| p.as_str() != here.as_str() && workspace::has_code(&scope::path(&top, p))) {
            out += &format!(" Run it in {other} with `--project {other}`.");
        }
    }
    error("E6013", command, out)
}

/// E6020: solace.toml can't be used, and why.
pub fn invalid_settings(why: impl std::fmt::Display) -> Failure {
    Failure(format!("E6020 invalid solace.toml\n  {why}"), 1)
}

#[cfg(test)]
fn read(root: &Path, name: &str, args: &[&str], budget: usize) -> Out {
    Ok(read_part(root, name, args, budget)?.0)
}
pub fn dispatch(
    store: &Store,
    v: &toml::Value,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    old: Option<&str>,
    budget: usize,
) -> Option<Out> {
    let root = &store.root;
    // D-181: file-form `view` remains for Python and plain-file Rust; graph
    // source is refused by `orientation::view`.
    if let Some(out) = orientation::view(store, v, a, budget) {
        return Some(out);
    }
    if a[0] == "grep" {
        return Some(grep(root, v, &a[1..].join(" "), budget));
    }
    if a[0] == "move" {
        return Some((|| {
            let (from, to, dest) = move_args(root, a)?;
            if crate::project_paths::same_project(root, &dest)
                && let Some(out) = crate::rust_move::move_module_file(store, from, to)
            {
                return out;
            }
            let dv = if crate::project_paths::same_project(root, &dest) { v.clone() } else { settings(&dest)? };
            move_files(root, v, from, &dest, &dv, to)
        })());
    }
    if matches!(a[0], "files" | "read" | "write" | "remove") {
        // `write --draft <id> [--replace [--all]]` retries a kept file patch (D-120).
        if a[0] == "write" && a.get(1) == Some(&"--draft") {
            return Some((|| {
                let rest: Vec<&str> = a[2..].iter().copied().filter(|x| *x != "--all").collect();
                let mut replaced = false;
                let (id, raw) = match rest.as_slice() {
                    [id] => (*id, drafts::file_patch(store, id)?),
                    [id, "--replace"] => {
                        let (raw, _) = drafts::replace(&drafts::file_patch(store, id)?, old, &input()?, a.contains(&"--all"))?;
                        replaced = true;
                        (*id, raw)
                    }
                    _ => return usage("write --draft <id> [--replace [--all]], with `old` and `text` for --replace"),
                };
                patch_with_draft(store, v, Some(id), &raw, budget, replaced)
            })());
        }
        // D-102: `files` and `read` take a list of paths, answered in order;
        // `write` and `remove` change one file and refuse a list.
        // A `--lines` or `--tail` range belongs to a single path.
        let mut paths = vec![];
        let mut flags = vec![];
        let mut i = 1;
        while i < a.len() {
            if a[i] == "--lines" || a[i] == "--tail" {
                let next = a.get(i + 1).copied().unwrap_or("");
                flags.extend([a[i], next]);
                i += 2;
            } else if a[i].starts_with("--") {
                flags.push(a[i]);
                i += 1;
            } else {
                paths.push(a[i]);
                i += 1;
            }
        }
        // `write` with no path takes a patch that changes several files (D-104).
        if a[0] == "write" && paths.is_empty() {
            return Some((|| {
                let raw = input()?;
                if !flags.is_empty() || !drafts::is_patch(&raw) {
                    return usage(concat!(
                        "write with no path takes a file patch in `text`: `*** Begin Patch`, then ",
                        "`*** Add File: <path>`, `*** Update File: <path>` or `*** Delete File: <path>` ",
                        "sections; `write <path>` changes one file"
                    ));
                }
                patch_with_draft(store, v, None, &raw, budget, false)
            })());
        }
        if paths.is_empty() {
            paths.push(".");
        }
        // A path from the Solace root into another project is read there, as
        // with `--project`; each reply is headed with the project.
        if matches!(a[0], "read" | "files") && paths.iter().any(|p| elsewhere(root, &file_name(Path::new(p)), true).is_some()) {
            return Some((|| {
                let mut out = String::new();
                for p in &paths {
                    let name = file_name(Path::new(p));
                    let left = budget.saturating_sub(out.len());
                    match elsewhere(root, &name, true) {
                        Some((dir, there, rest)) => {
                            let (s, sv) = (Store::peek(&there)?, settings(&there)?);
                            let mut line = vec![a[0], rest.as_str()];
                            line.extend(&flags);
                            out += &format!("==== {rest} ({dir}) ====\n");
                            out += &dispatch(&s, &sv, &line, input, old, left).unwrap()?;
                        }
                        None => {
                            let mut line = vec![a[0], name.as_str()];
                            line.extend(&flags);
                            out += &format!("==== {name} ====\n");
                            out += &dispatch(store, v, &line, input, old, left).unwrap()?;
                        }
                    }
                }
                Ok(out)
            })());
        }
        return Some((|| {
            if paths.len() > 1 && a[0] == "write" {
                return usage(concat!(
                    "write <path> changes one file; for several, `write` with no path takes ",
                    "a file patch (`*** Update File: <path>` sections)"
                ));
            }
            if paths.len() > 1 && !flags.is_empty() {
                return usage("a --lines or --tail range takes one path: read <path> --lines 3,7,10..12");
            }
            // D-109: a path that doesn't exist is named before anything is read.
            if a[0] == "read" {
                for name in &paths {
                    let name = file_name(Path::new(name));
                    if !reachable(root, &name)?.exists() && tier(root, v, &name, None) != "graph" {
                        return missing(root, &name, &paths);
                    }
                }
            }
            // Several files share the reply: each is headed with its path, and
            // the paths that don't fit are named for the next read.
            let many = paths.len() > 1;
            let mut out = String::new();
            // `remove` checks every path before it removes any: whole or nothing.
            let (mut doomed, mut refused) = (vec![], vec![]);
            for (k, name) in paths.iter().enumerate() {
                if a[0] == "read" && many && budget.saturating_sub(out.len()) < 400 {
                    out += &format!("Next: read {}\n", paths[k..].join(" "));
                    break;
                }
                let name = file_name(Path::new(name));
                let p = reachable(root, &name)?;
                let (t, bytes) = tier_and_bytes(root, v, &mut crate::lockdown::paths::Ignores::new(root), &name, None);
                if a[0] != "files" && t == "graph" {
                    let e = graph_error(store, &name);
                    if a[0] == "remove" {
                        refused.extend(e.err());
                        continue;
                    }
                    return e;
                }
                match a[0] {
                    "files" => {
                        if p.is_dir() {
                            tree(root, v, &p, &mut out)?;
                        } else {
                            out += &listed(&name, &t, measure(root, v, &name, &t, bytes.as_deref()).as_deref());
                        }
                    }
                    "read" => {
                        if many {
                            out += &format!("==== {name} ====\n");
                        }
                        let (text, cut) =
                            read_part(root, &name, &flags, if many { budget.saturating_sub(out.len() + 200) } else { budget })?;
                        out += &text;
                        if cut && k + 1 < paths.len() {
                            out += &format!("Next: read {}\n", paths[k + 1..].join(" "));
                            break;
                        }
                    }
                    "write" => {
                        if t == "protected" {
                            if in_skills(v, &name) {
                                return error("E6002", &name, "skills are the humans' house rules: `read` them; agents can't change them");
                            }
                            return error(
                                "E6002",
                                &name,
                                concat!(
                                    "protected files change only through a human-approved proposal: ",
                                    "in a work run, `write` it with the exact new text"
                                ),
                            );
                        }
                        not_here(root, &name)?;
                        return crate::lockdown::files::write(root, v, &name, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old);
                    }
                    "remove" => {
                        if name == "gleam.toml" || (t != "watched" && !t.starts_with("unwatched")) {
                            refused.extend(
                                error::<()>(
                                    "E6002",
                                    &name,
                                    format!("only watched and unwatched files can be removed, and never gleam.toml; {t}"),
                                )
                                .err(),
                            );
                        } else if !p.is_file() {
                            refused.extend(error::<()>("E6002", &name, not_found(root, &name, "no such file")).err());
                        } else {
                            doomed.push((name, p));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            if let [one] = &refused[..]
                && paths.len() == 1
            {
                return Err(Failure(one.0.clone(), one.1));
            }
            if !refused.is_empty() {
                let all: Vec<String> = refused.iter().map(|f| f.0.clone()).collect();
                return Err(Failure(format!("nothing removed:\n{}", all.join("\n")), refused[0].1));
            }
            for (name, p) in doomed {
                if let Some((tid, rid)) = board::context() {
                    fs::create_dir_all(root.join(".solace/pending"))?;
                    let record = root.join(".solace/pending").join(format!("{}.json", sha256(name.as_bytes())));
                    // An earlier change to this file in the command keeps its record: it holds the original bytes.
                    if !same_run(&record, &json!(rid)) {
                        fsx::write_atomic(&record, json!({"task":tid,"run":rid,"path":name,"bytes":fs::read(&p).ok()}).to_string())?;
                    }
                }
                fs::remove_file(p)?;
                out += &format!("Removed {name}\n");
            }
            Ok(trim_output(&out, budget))
        })());
    }
    // D-131: `build plan` reads every project's build recipe; it runs nothing.
    if matches!(a, ["build", "plan"]) {
        return Some(recipe::plan_text(&scope::top(root)));
    }
    if a[0] == "build" && a.len() > 1 {
        return Some(usage("build takes no arguments; `build plan` lists every project's build recipe, in order"));
    }
    if matches!(a[0], "build" | "format")
        && let Some(id) = language::id(root)
    {
        match language::reason(id) {
            Ok(Some(why)) => return Some(error("E6013", a[0], why)),
            Err(e) => return Some(Err(e)),
            Ok(None) => {}
        }
    }
    if a[0] == "format" && language::is_rust(root) {
        return Some(match a {
            ["format"] => {
                let task = board::context().map(|(tid, _)| tid);
                plain(store, task.as_deref(), "format", None)
            }
            ["format", module] => store.format_module_rust(module),
            _ => usage("format <module>: name one module, e.g. `format crate`"),
        });
    }
    if matches!(a[0], "build" | "format" | "run") {
        return Some((|| {
            let (argv, timeout) = if a[0] == "run" {
                let name = a.get(1).copied().unwrap_or("");
                let commands = v.get("commands").and_then(|x| x.as_table());
                let Some(c) = commands.and_then(|c| c.get(name)) else {
                    return error(
                        "E6010",
                        name,
                        format!(
                            concat!("declared commands: {}; to add one, `write solace.toml` in a work run ", "proposes it for approval"),
                            commands.map(|c| c.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()
                        ),
                    );
                };
                if a.len() > 2 && c.get("args").and_then(|x| x.as_bool()) != Some(true) {
                    return error("E6011", name, "this command does not allow extra arguments");
                }
                let mut argv: Vec<String> = c["run"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()).collect();
                argv.extend(a[2..].iter().map(|s| s.to_string()));
                (argv, c.get("timeout").and_then(|x| x.as_integer()).unwrap_or(120) as u64)
            } else if a[0] == "build" && language::is_rust(root) {
                if let Some(why) = language::reason(language::RUST)? {
                    return error("E6013", "build", why);
                }
                let mut argv = vec!["cargo".to_string()];
                argv.extend(cargo::build_args(root));
                (argv, 120)
            } else if !root.join("gleam.toml").exists() {
                let task = board::context().map(|(tid, _)| tid);
                return plain(store, task.as_deref(), a[0], a.get(1).copied());
            } else {
                (vec!["gleam".into(), a[0].into()], 120)
            };
            // A declared run writes one undo record with its whole snapshot of
            // watched files before the process runs (D-77 point 7).
            if let Some((tid, rid)) = board::context() {
                fs::create_dir_all(root.join(".solace/pending"))?;
                let files: Vec<Value> = watched_paths(root, v)?
                    .into_iter()
                    .map(|name| json!({"path": name, "bytes": fs::read(root.join(&name)).ok()}))
                    .collect();
                let folders: Vec<String> = board::existing_folders(root).into_iter().collect();
                fsx::write_atomic(
                    &root.join(".solace/pending").join(format!("run-{rid}.json")),
                    json!({"task":tid,"run":rid,"files":files,"folders":folders}).to_string(),
                )?;
            }
            store.materialize(&store.index()?)?;
            // D-166: the sources are hashed before the command runs, so a source the build itself
            // changes isn't recorded as built. A recipe that can't be read doesn't refuse the run:
            // recording is a note, never a gate, like a record write that fails.
            let (record, unread) = if a[0] == "run" {
                let top = scope::top(root);
                let dir = scope::dir(&top, root);
                match recipe::built_sources(&top, &dir, a[1]) {
                    Ok(sources) => (sources.map(|sources| (top, dir, sources)), None),
                    Err(e) => (None, Some(e.0)),
                }
            } else {
                (None, None)
            };
            let mut out = command(root, &argv, timeout, budget, a[0] != "format", a[0] == "build" && language::is_rust(root))?;
            if !passed(&out) {
                return fail(out);
            }
            if let Some(why) = unread {
                out.push_str(&format!("\nnote: could not record what it built ({}); `build plan` uses file times\n", why.trim()));
            }
            // D-166: a passing declared command that is a recipe's last step records what it built.
            if let Some((top, dir, sources)) = record {
                if let Err(e) = recipe::record_built(&top, &dir, a[1], sources) {
                    // The build passed: a record that can't be written leaves the plan on file times.
                    out.push_str(&format!("\nnote: could not record what it built ({}); `build plan` uses file times\n", e.0.trim()));
                }
            }
            Ok(out)
        })());
    }
    None
}

pub fn agent_config(agent: &str, dir: &Path, binary: &Path, timeout: u64) -> Out {
    fs::create_dir_all(dir)?;
    let binary = binary.to_string_lossy();
    let launch = match agent {
        "codex" => {
            // Codex runs some models with `exec` and others with the MCP tool directly.
            // An `exec` script can batch several solace replies, so its output limit is
            // raised past Codex's default 10k tokens. Codex takes its tool list once at
            // startup, so it must wait for the server instead of dropping a slow one.
            let hint = "The tool is the MCP tool `solace`. If you have `exec`, call it from there as \
                `await tools.mcp__solace__solace({command: \"inspect app.main --context\"})`; one `exec` \
                script can make several solace calls, so read everything you need in one step. Otherwise \
                call `mcp__solace__solace` directly, putting new code in `text` for submit commands.";
            let mut config = format!(
                concat!(
                    "sandbox_mode = \"read-only\"\napproval_policy = \"never\"\n",
                    "web_search = \"live\"\ntool_output_token_limit = 30000\n",
                    "mcp_optional_startup_grace_ms = 0\n",
                    "developer_instructions = {}\n[features]\n"
                ),
                json!(hint)
            );
            for f in [
                "shell_tool",
                "unified_exec",
                "apply_patch_freeform",
                "multi_agent",
                "apps",
                "plugins",
                "goals",
                "image_generation",
                "view_image",
                "browser_use",
                "computer_use",
                "in_app_browser",
                "hooks",
                "memories",
                "tool_suggest",
            ] {
                config += &format!("{f} = false\n");
            }
            config += &format!(
                concat!(
                    "[mcp_servers.solace]\ncommand = {}\nargs = [\"mcp\", \"--budget\", \"32000\"]\n",
                    "env_vars = [\"SOLACE_TASK\", \"SOLACE_RUN\", \"SOLACE_SESSION\", \"SOLACE_CONFIG\"]\n",
                    "default_tools_approval_mode = \"approve\"\ntool_timeout_sec = {}\n",
                    "required = true\nstartup_timeout_sec = 60\n"
                ),
                json!(binary),
                timeout.max(3600)
            );
            fs::write(dir.join("config.toml"), config)?;
            json!({"args":[],"env":{"CODEX_HOME":dir},"config":"config.toml"})
        }
        "claude" => {
            fs::write(
                dir.join("mcp.json"),
                json!({"mcpServers":{"solace":{"command":binary,"args":["mcp","--budget","40000"]}}}).to_string(),
            )?;
            fs::write(
                dir.join("settings.json"),
                json!({
                    "disableAllHooks": true,
                    "permissions": {
                        "allow": ["mcp__solace__solace", "WebFetch", "WebSearch"],
                        "deny": ["Bash", "Read", "Write", "Edit", "Glob", "Grep", "Agent", "Task"]
                    },
                    "enabledPlugins": {}
                })
                .to_string(),
            )?;
            // Claude Code takes its MCP tool list once at startup too.
            json!({
                "args": [
                    "--strict-mcp-config",
                    "--mcp-config",
                    dir.join("mcp.json"),
                    "--settings",
                    dir.join("settings.json"),
                    "--setting-sources",
                    "",
                    "--disable-slash-commands",
                    "--tools",
                    "WebFetch,WebSearch",
                    "--allowedTools",
                    "mcp__solace__solace",
                    "WebFetch",
                    "WebSearch"
                ],
                "env": {
                    "MCP_TIMEOUT": "60000",
                    "MCP_TOOL_TIMEOUT": (timeout.max(3600) * 1000).to_string()
                },
                "config": "settings.json"
            })
        }
        _ => return usage("agent-config codex|claude <dir>"),
    };
    fs::write(dir.join("launch.json"), launch.to_string())?;
    Ok(format!("{launch}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;
    fn fixture() -> (Project, toml::Value) {
        let dir = Scratch::new()
            .with(&[("gleam.toml", "name = \"fixture\"\nversion = \"1.0.0\"\n"), ("src/main.gleam", "pub fn main() { 1 }\n")]);
        fs::create_dir_all(dir.join("priv")).unwrap();
        let v = "watch_limit = 100\nunwatched = [\"priv/generated/\"]\n".parse().unwrap();
        (dir.import(), v)
    }
    fn call(s: &Store, v: &toml::Value, args: &[&str], text: &str, old: Option<&str>) -> Out {
        dispatch(s, v, args, &mut || Ok(text.into()), old, 1000).unwrap()
    }
    /// A folder's `.gitignore` is matched from that folder: `/gen` in
    /// `src/.gitignore` ignores `src/gen`, not a `gen` at the root.
    #[test]
    fn a_nested_gitignore_is_anchored_at_its_folder() {
        let (p, v) = fixture();
        fs::create_dir_all(p.root.join("src/gen")).unwrap();
        fs::create_dir_all(p.root.join("gen")).unwrap();
        fs::write(p.root.join("src/.gitignore"), "/gen\n").unwrap();
        fs::write(p.root.join("src/gen/x.txt"), "x\n").unwrap();
        fs::write(p.root.join("gen/x.txt"), "x\n").unwrap();
        assert_eq!(tier(&p.root, &v, "src/gen/x.txt", None), "unwatched");
        assert_eq!(tier(&p.root, &v, "gen/x.txt", None), "watched");
    }
    /// A `!pattern` in an ignored folder's `.gitignore` doesn't re-include its
    /// files: git never reads it.
    #[test]
    fn an_ignored_folder_stays_ignored_whatever_its_gitignore_says() {
        let (p, v) = fixture();
        fs::create_dir_all(p.root.join("out")).unwrap();
        fs::write(p.root.join(".gitignore"), "out/\n").unwrap();
        fs::write(p.root.join("out/.gitignore"), "!keep.txt\n").unwrap();
        fs::write(p.root.join("out/keep.txt"), "x\n").unwrap();
        assert_eq!(tier(&p.root, &v, "out/keep.txt", None), "unwatched");
    }
    /// A path from the Solace root reads another project's file; a write to it
    /// is refused with the `--project` form, and E6002 names the base.
    #[test]
    fn a_root_path_reads_through_another_project() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"a\", \"b\"]\n"),
            ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
            ("a/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
            ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
            ("b/src/plan.gleam", "pub fn answer() -> Int {\n  42\n}\n"),
            ("b/docs/notes.md", "needle here\n"),
        ]);
        for d in ["a", "b"] {
            import(&root.join(d)).unwrap();
        }
        let s = Store::at(&root.join("a")).unwrap();
        let v = settings(&s.root).unwrap();
        let r = call(&s, &v, &["read", "b/docs/notes.md"], "", None).unwrap();
        assert!(r.starts_with("==== docs/notes.md (b) ====\n") && r.contains("needle here"), "{r}");
        let g = call(&s, &v, &["grep", "needle", "--in", "b/docs"], "", None).unwrap();
        assert!(g.starts_with("==== docs (b) ====\n") && g.contains("docs/notes.md"), "{g}");
        let f = call(&s, &v, &["files", "b/docs"], "", None).unwrap();
        assert!(f.starts_with("==== docs (b) ====\n") && f.contains("notes.md"), "{f}");
        // Writes stay in their project: the refusal names the form that works.
        let w = call(&s, &v, &["write", "b/docs/new.md"], "x\n", None).unwrap_err().0;
        assert!(w.contains("E6002") && w.contains("(paths are relative to a); it is docs/new.md in b: add --project b"), "{w}");
        assert!(!s.root.join("b").exists());
        // A move to or from a root path is refused the same way, and moves nothing.
        fs::write(s.root.join("x.md"), "x\n").unwrap();
        let m = call(&s, &v, &["move", "x.md", "b/docs/x.md"], "", None).unwrap_err().0;
        assert!(m.contains("E6002") && m.contains("it is docs/x.md in b: add --project b"), "{m}");
        let m = call(&s, &v, &["move", "b/docs/notes.md", "y.md"], "", None).unwrap_err().0;
        assert!(m.contains("(paths are relative to a); it is docs/notes.md in b: add --project b"), "{m}");
        assert!(s.root.join("x.md").exists() && !s.root.join("b").exists());
        // So is a file patch's `*** Add File:`.
        let patch = "*** Begin Patch\n*** Add File: b/docs/new.md\n+x\n*** End Patch\n";
        let w = call(&s, &v, &["write"], patch, None).unwrap_err().0;
        assert!(w.contains("it is docs/new.md in b: add --project b"), "{w}");
        assert!(!s.root.join("b").exists());
        let e = call(&s, &v, &["remove", "b/docs/notes.md"], "", None).unwrap_err().0;
        assert!(e.contains("no such file (paths are relative to a); it is docs/notes.md in b: add --project b"), "{e}");
        let e = call(&s, &v, &["read", "docs/notes.md"], "", None).unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("no such file (paths are relative to a)") && !e.contains("it is"), "{e}");
    }
    #[test]
    fn a1_tiers_golden() {
        let (s, v) = fixture();
        for (p, b) in [
            ("priv/a.html", b"ok".as_slice()),
            ("priv/b.bin", b"\0"),
            ("priv/large", &[b'x'; 101]),
            ("priv/latin", b"\xe9t\xe9\n"),
            ("docs.md", b"a\n\nb"),
            ("priv/ignored", b"x"),
            ("manifest.toml", b"x"),
            ("solace.toml", b""),
            ("AGENTS.md", b"x"),
            ("misc", b"x"),
            (".gitignore", b"priv/ignored\n"),
            ("priv/generated/out", b"x"),
            (".git/config", b"x"),
            ("node_modules/pkg/index.js", b"x"),
            ("sub/node_modules/pkg/index.js", b"x"),
            ("sub/.gitignore", b"x"),
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let got = call(&s, &v, &["files"], "", None).unwrap();
        assert_eq!(
            got,
            // D-119: lines for UTF-8 text, bytes for any other file or one over
            // watch_limit; collapsed folders keep their file count.
            concat!(
                ".git/ [unwatched; 1 files]\n.gitignore [protected; 1 lines]\n.solace/ [internal]\n",
                "AGENTS.md [protected; 1 lines]\ndocs.md [watched; 3 lines]\ngleam.toml [watched; 2 lines]\n",
                "manifest.toml [unwatched; 1 lines]\nmisc [watched; 1 lines]\nnode_modules/ [unwatched; 1 files]\n",
                "priv/\npriv/a.html [watched; 1 lines]\npriv/b.bin [unwatched (binary); 1 bytes]\n",
                "priv/generated/ [unwatched; 1 files]\npriv/ignored [unwatched; 1 lines]\n",
                "priv/large [unwatched (over limit); 101 bytes]\npriv/latin [unwatched (binary); 4 bytes]\n",
                "solace.toml [protected; 0 lines]\nsrc/\nsrc/main.gleam [graph]\nsub/\n",
                "sub/.gitignore [protected; 1 lines]\nsub/node_modules/ [unwatched; 1 files]\n"
            )
        );
    }
    #[test]
    fn grep_searches_watched_files_only() {
        let (s, _) = fixture();
        let v = "watch_limit = 2000\nunwatched = [\"priv/generated/\"]\n".parse().unwrap();
        let long = format!("{}needle é{}", "word ".repeat(60), " tail".repeat(60));
        for (p, b) in [
            ("docs/a.md", format!("intro\nThe Needle here\n{long}\n")),
            ("solace.toml", "# needle in settings\n".into()),
            ("priv/generated/out", "needle\n".into()),
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        fs::write(s.root.join("src/main.gleam"), "pub fn main() { \"needle\" }\n").unwrap();
        let got = call(&s, &v, &["grep", "\"needle\""], "", None).unwrap();
        // Watched and protected files, with line numbers; never source or unwatched files.
        assert!(got.starts_with("docs/a.md\n     2: The Needle here\n     3: …"), "{got}");
        assert!(got.contains("needle é") && got.contains("solace.toml\n     1: # needle in settings"), "{got}");
        assert!(!got.contains("generated") && !got.contains("main.gleam"), "{got}");
        assert!(got.lines().nth(3).unwrap().len() < 260, "{got}");
        // A miss in a project with source says the source is `find`'s.
        assert_eq!(
            call(&s, &v, &["grep", "absent"], "", None).unwrap(),
            "No matches for \"absent\" in watched files.\nSource is held as symbols (use `find`).\n"
        );
        assert!(call(&s, &v, &["grep", "\"The", "Needle\""], "", None).unwrap().starts_with("docs/a.md"));
    }
    /// D-109: grep searches code text as written, quotes included unless they
    /// wrap the whole query, and a miss that looks like a shell habit gets a hint.
    #[test]
    fn grep_searches_code_text_and_hints_after_a_misuse() {
        let (s, _) = fixture();
        let v = "watch_limit = 2000\n".parse().unwrap();
        fs::create_dir_all(s.root.join("docs")).unwrap();
        fs::write(s.root.join("docs/code.rs"), "\"write\" => go(),\nlet x = \"write\" | y;\nfn f() -> Result<(), E>\n| a \\| b |\n")
            .unwrap();
        for (q, line) in [
            (&["grep", "\"write\"", "=>"][..], 1),
            (&["grep", "\"write\"", "|"], 2),
            (&["grep", "->", "Result"], 3),
            (&["grep", "a", "\\|", "b"], 4),
        ] {
            let got = call(&s, &v, q, "", None).unwrap();
            assert!(got.starts_with("docs/code.rs\n") && got.contains(&format!("     {line}: ")), "{q:?}: {got}");
        }
        // One quoted term is searched without its quotes.
        let got = call(&s, &v, &["grep", "\"write\""], "", None).unwrap();
        assert!(got.contains("     1: ") && got.contains("     2: "), "{got}");
        for (q, filter) in
            [(&["grep", "-i", "needle"][..], false), (&["grep", "\"x\"", "src/a.rs"], true), (&["grep", "foo", "--path", "src"], true)]
        {
            let got = call(&s, &v, q, "", None).unwrap();
            assert!(
                got.starts_with("No matches") && got.contains("Searched for: ") && got.contains("literal text, already case-insensitive"),
                "{q:?}: {got}"
            );
            assert_eq!(got.contains("end the query with `--in <path>`"), filter, "{q:?}: {got}");
            assert!(!got.contains("no path filter"), "{q:?}: {got}");
        }
        assert!(call(&s, &v, &["grep", "-i", "needle"], "", None).unwrap().contains("Searched for: -i needle."));
        // Code at the end of a query is not taken for a path: a plain miss.
        for q in [&["grep", "let", "n", "=", "a.len"][..], &["grep", "x", "=", "a", "/"], &["grep", "self.name"]] {
            assert_eq!(
                call(&s, &v, q, "", None).unwrap(),
                format!("No matches for \"{}\" in watched files.\nSource is held as symbols (use `find`).\n", q[1..].join(" ")),
                "{q:?}"
            );
        }
        // A folder or a file is: the hint says to use `--in <path>`.
        for q in [&["grep", "foo", "src/app"][..], &["grep", "foo", "notes.md"]] {
            assert!(call(&s, &v, q, "", None).unwrap().contains("end the query with `--in <path>`"), "{q:?}");
        }
        // `--in` with no path after it is text, not an option.
        assert_eq!(
            call(&s, &v, &["grep", "foo", "--in"], "", None).unwrap(),
            "No matches for \"foo --in\" in watched files.\nSource is held as symbols (use `find`).\n"
        );
    }
    /// D-119: `--context` merges touching windows, shows gaps, marks context
    /// lines `-`, cuts long ones, and takes only 1 to 5.
    #[test]
    fn grep_context_merges_windows_and_takes_one_to_five() {
        let (s, _) = fixture();
        let v = "watch_limit = 20000\n".parse().unwrap();
        fs::create_dir_all(s.root.join("docs")).unwrap();
        let mut text: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        text[2] = "hit three".into();
        text[4] = "hit five".into();
        text[14] = "hit fifteen".into();
        text[9] = format!("{}{}", "x ".repeat(300), "tail");
        fs::write(s.root.join("docs/a.txt"), text.join("\n") + "\n").unwrap();
        let got = call(&s, &v, &["grep", "hit", "--context", "1"], "", None).unwrap();
        // 3 and 5 are two lines apart: windows 2..4 and 4..6 touch and merge; 15 is apart.
        assert_eq!(
            got,
            concat!(
                "docs/a.txt\n     2- line 2\n     3: hit three\n     4- line 4\n",
                "     5: hit five\n     6- line 6\n  …\n    14- line 14\n",
                "    15: hit fifteen\n    16- line 16\n"
            )
        );
        // Without context, no gap marker and no context lines.
        assert_eq!(
            call(&s, &v, &["grep", "hit"], "", None).unwrap(),
            "docs/a.txt\n     3: hit three\n     5: hit five\n    15: hit fifteen\n"
        );
        // A long context line is cut around its start; the reply stays short.
        let got = call(&s, &v, &["grep", "hit", "five", "--context", "5"], "", None).unwrap();
        assert!(got.contains("    10- x x") && got.lines().all(|l| l.len() < 260), "{got}");
        // A match past the 10 shown, inside a window, is context, not a match line.
        fs::write(s.root.join("docs/b.txt"), (1..=12).map(|n| format!("hit {n}")).collect::<Vec<_>>().join("\n") + "\n").unwrap();
        let got = call(&s, &v, &["grep", "hit", "--in", "docs/b.txt", "--context", "1"], "", None).unwrap();
        assert!(
            got.starts_with("docs/b.txt (12 matches, showing 10)\n")
                && got.contains("    10: hit 10\n")
                && got.contains("    11- hit 11\n"),
            "{got}"
        );
        // A context line keeps its indentation, as the line is written.
        fs::write(s.root.join("docs/c.txt"), "    ind hit\n        deep\n").unwrap();
        assert_eq!(
            call(&s, &v, &["grep", "hit", "--in", "docs/c.txt", "--context", "1"], "", None).unwrap(),
            "docs/c.txt\n     1:     ind hit\n     2-         deep\n"
        );
        for bad in ["0", "6", "x", "-1"] {
            let e = call(&s, &v, &["grep", "hit", "--context", bad], "", None).unwrap_err();
            assert!(e.0.contains("--context takes a number from 1 to 5") && e.1 == 2, "{bad}: {e:?}");
        }
        assert!(call(&s, &v, &["grep", "hit", "--context", "5"], "", None).is_ok());
        // The options count only at the end, in either order.
        let a = call(&s, &v, &["grep", "hit", "--in", "docs", "--context", "1"], "", None).unwrap();
        assert_eq!(a, call(&s, &v, &["grep", "hit", "--context", "1", "--in", "docs"], "", None).unwrap());
        assert!(a.starts_with("docs/a.txt\n     2- line 2"), "{a}");
        assert_eq!(
            call(&s, &v, &["grep", "--context", "3", "hit"], "", None).unwrap(),
            "No matches for \"--context 3 hit\" in watched files.\nSearched for: --context 3 hit. ".to_string()
                + GREP_SYNTAX
                + "\nSource is held as symbols (use `find`).\n"
        );
    }
    /// D-119: `--in` takes a file or a folder, checked as `read` checks a path;
    /// a missing path is refused, an unwatched or internal one named.
    #[test]
    fn grep_in_takes_a_file_or_a_folder() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 20000\nunwatched = [\"priv/generated/\"]\n".parse().unwrap();
        for (p, b) in [
            ("docs/a.md", "needle a\n"),
            ("docs/deep/b.md", "needle b\n"),
            ("docs2/c.md", "needle c\n"),
            ("priv/generated/out", "needle\n"),
            ("priv/blob.bin", "needle\0"),
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let file = call(&s, &v, &["grep", "needle", "--in", "docs/a.md"], "", None).unwrap();
        assert_eq!(file, "docs/a.md\n     1: needle a\n");
        // A folder takes its files at any depth, and not a folder that shares its prefix.
        let dir = call(&s, &v, &["grep", "needle", "--in", "docs"], "", None).unwrap();
        assert_eq!(dir, "docs/a.md\n     1: needle a\ndocs/deep/b.md\n     1: needle b\n");
        assert_eq!(call(&s, &v, &["grep", "needle", "--in", "./docs/"], "", None).unwrap(), dir);
        assert_eq!(call(&s, &v, &["grep", "absent", "--in", "docs"], "", None).unwrap(), "No matches for \"absent\" in docs.\n");
        // A path that is not there is refused and named.
        let e = call(&s, &v, &["grep", "needle", "--in", "docs/gone.md"], "", None).unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("docs/gone.md") && e.contains("no such file or folder"), "{e}");
        let e = call(&s, &v, &["grep", "needle", "--in", "../x"], "", None).unwrap_err().0;
        assert!(e.contains("E6006") || e.contains("E6002"), "{e}");
        // Unwatched, binary, source and internal paths are named as not searched.
        for (p, why) in [
            ("priv/generated/out", "unwatched"),
            ("priv/generated", "holds no watched or protected files"),
            ("priv/blob.bin", "unwatched (binary)"),
            ("src/main.gleam", "source held as symbols"),
            (".solace", "Solace's own files"),
        ] {
            let got = call(&s, &v, &["grep", "needle", "--in", p], "", None).unwrap();
            assert!(got.starts_with(&format!("{p}")) && got.contains(why) && got.contains("not searched"), "{p}: {got}");
        }
    }
    /// D-119: a file with more than 10 matches shows 10 and its count; at most
    /// 40 files; the reply stops at a whole file within the budget and says
    /// how many files it left out.
    #[test]
    fn grep_counts_matches_and_stops_at_a_whole_file_within_the_budget() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 100000\n".parse().unwrap();
        fs::create_dir_all(s.root.join("docs")).unwrap();
        let many: String = (1..=14).map(|n| format!("needle {n}\n")).collect();
        fs::write(s.root.join("docs/many.md"), &many).unwrap();
        let got = grep(&s.root, &v, "needle --in docs/many.md", 10_000).unwrap();
        assert!(got.starts_with("docs/many.md (14 matches, showing 10)\n"), "{got}");
        assert_eq!(got.matches("needle").count(), 10, "{got}");
        assert!(got.contains("    10: needle 10\n") && !got.contains("needle 11"), "{got}");
        fs::remove_file(s.root.join("docs/many.md")).unwrap();
        // 45 files: 40 shown, 5 left out.
        for n in 0..45 {
            fs::write(s.root.join(format!("docs/f{n:02}.md")), "needle\n").unwrap();
        }
        let got = grep(&s.root, &v, "needle", 100_000).unwrap();
        assert_eq!(got.matches(".md\n").count(), 40, "{got}");
        assert!(got.ends_with("… 5 more files match, not shown; narrow with `--in <path>`.\n"), "{got}");
        // Files of about 1 KB each against a budget of 3,000: whole files only.
        for n in 0..45 {
            fs::write(s.root.join(format!("docs/f{n:02}.md")), format!("needle {}\n", "w".repeat(150)).repeat(6)).unwrap();
        }
        let got = grep(&s.root, &v, "needle", 3_000).unwrap();
        let shown = got.matches(".md\n").count();
        assert!(got.len() <= 3_000 && (1..40).contains(&shown), "{} {shown}", got.len());
        assert!(got.ends_with(&format!("… {} more files match, not shown; narrow with `--in <path>`.\n", 45 - shown)), "{got}");
        assert_eq!(got.matches("needle").count(), shown * 6, "every shown file is whole");
        // One file left out: the tail says "file matches".
        for n in 0..45 {
            fs::remove_file(s.root.join(format!("docs/f{n:02}.md"))).unwrap();
        }
        for n in 0..3 {
            fs::write(s.root.join(format!("docs/g{n}.md")), (1..=30).map(|k| format!("needle {k}\n")).collect::<String>()).unwrap();
        }
        // Two of the three fit the budget, so the tail names one file.
        let got = grep(&s.root, &v, "needle", 600).unwrap();
        assert_eq!(got.matches("docs/g").count(), 2, "{got}");
        assert!(got.ends_with("… 1 more file matches, not shown; narrow with `--in <path>`.\n"), "{got}");
        // A budget below one file shows none and says so.
        let got = grep(&s.root, &v, "needle", 100).unwrap();
        assert_eq!(got, "… the budget fits no file; 3 files match, narrow with `--in <path>`.\n");
        // `--in` already names the only place searched, so the tail can't
        // repeat it: it points at the rest of the call and `--context`.
        let got = grep(&s.root, &v, "needle --in docs --context 1", 100).unwrap();
        assert_eq!(got, "… the budget fits no file; 3 files match, run grep alone in the call, or drop `--context`.\n");
        let got = grep(&s.root, &v, "needle --in docs --context 1", 500).unwrap();
        assert_eq!(got.matches("docs/g").count(), 1, "{got}");
        assert!(got.ends_with("… 2 more files match, not shown; run grep alone in the call, or drop `--context`.\n"), "{got}");
        // `--in` without `--context`: the tail names only the rest of the call.
        let got = grep(&s.root, &v, "needle --in docs", 100).unwrap();
        assert_eq!(got, "… the budget fits no file; 3 files match, run grep alone in the call.\n");
        let got = grep(&s.root, &v, "needle --in docs", 500).unwrap();
        assert!(got.ends_with("… 2 more files match, not shown; run grep alone in the call.\n"), "{got}");
    }
    /// A reply that shows a line cut at either end ends with one footer.
    #[test]
    fn grep_says_when_a_shown_line_is_cut() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 100000\n".parse().unwrap();
        let long = |before: usize, after: usize| format!("{}needle{}\n", "a".repeat(before), "b".repeat(after));
        let run = |files: &[(&str, String)], query: &str| {
            fs::create_dir_all(s.root.join("cuts")).unwrap();
            for (n, text) in files {
                fs::write(s.root.join("cuts").join(n), text).unwrap();
            }
            let got = grep(&s.root, &v, query, 100_000).unwrap();
            fs::remove_dir_all(s.root.join("cuts")).unwrap();
            got
        };
        let once = |got: &str| got.matches("are cut from longer lines").count() == 1 && got.ends_with(&format!("{GREP_CUT}\n"));
        // Cut at the start only.
        let got = run(&[("a.md", long(1000, 10))], "needle --in cuts");
        assert!(once(&got), "{got}");
        // Cut at the end only.
        let got = run(&[("a.md", long(0, 1000))], "needle --in cuts");
        assert!(once(&got), "{got}");
        // A cut context line.
        let got = run(&[("a.md", format!("needle\n{}\n", "x".repeat(1000)))], "needle --in cuts --context 1");
        assert!(once(&got), "{got}");
        // Many cut lines in several files: still one footer.
        let two = long(1000, 1000).repeat(3);
        let got = run(&[("a.md", two.clone()), ("b.md", two)], "needle --in cuts");
        assert!(once(&got), "{got}");
        // Whole lines only, one starting with `…`, and a `…` window separator.
        let mut text = "…needle whole\n".to_string();
        text += &"filler\n".repeat(8);
        text += "needle again\n";
        let got = run(&[("a.md", text)], "needle --in cuts --context 1");
        assert!(got.contains("  …\n") && !got.contains("are cut from longer lines"), "{got}");
    }
    /// The footer's room is kept only for a file the reply shows: a cut file
    /// the budget leaves out pushes out no whole-line file that fits.
    #[test]
    fn grep_a_cut_file_left_out_by_the_budget_pushes_out_nothing() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 100000\n".parse().unwrap();
        fs::create_dir_all(s.root.join("cuts")).unwrap();
        // a.md: three whole lines; its block is 64 bytes.
        fs::write(s.root.join("cuts/a.md"), "needle one\nneedle two\nneedle three\n").unwrap();
        // b.md: one 1016-character line, shown cut to about 100 bytes.
        fs::write(s.root.join("cuts/b.md"), format!("{}needle{}\n", "a".repeat(1000), "b".repeat(10))).unwrap();
        // 250 fits a.md's block with the "more files" line (204), not the old
        // footer room for b.md's cut (388); b.md with its own footer room
        // (361) is left out, so the reply shows a.md and carries no footer.
        let got = grep(&s.root, &v, "needle --in cuts", 250).unwrap();
        fs::remove_dir_all(s.root.join("cuts")).unwrap();
        assert!(got.contains("needle one\n") && got.contains("needle three\n"), "{got}");
        assert!(got.contains("1 more file matches"), "{got}");
        assert!(!got.contains("b.md") && !got.contains("are cut from longer lines"), "{got}");
        assert!(got.len() <= 250, "{got}");
    }
    /// D-109: `write <path>` refuses patch text and a drastic shrink, and its
    /// reply gives the file's lines before and after.
    #[test]
    fn write_refuses_a_patch_and_a_drastic_shrink() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 100000\n".parse().unwrap();
        fs::write(s.root.join("a.txt"), "one\ntwo\n").unwrap();
        call(&s, &v, &["read", "a.txt"], "", None).unwrap();
        let e = call(&s, &v, &["write", "a.txt"], "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+uno\n*** End Patch\n", None)
            .unwrap_err();
        assert!(e.0.contains("this text is a patch; send it with `write` and no path") && e.1 == 2, "{e:?}");
        assert_eq!(fs::read_to_string(s.root.join("a.txt")).unwrap(), "one\ntwo\n");
        assert_eq!(call(&s, &v, &["write", "a.txt"], "one\ntwo\nthree\n", None).unwrap(), "Wrote a.txt (2 → 3 lines)\n");
        assert_eq!(call(&s, &v, &["write", "b.txt"], "x", None).unwrap(), "Wrote b.txt (1 line)\n");
        // A partial read allows a whole write, so a shrink to under half of a long file is refused.
        let long: String = (1..=300).map(|n| format!("line {n}\n")).collect();
        fs::write(s.root.join("long.rs"), &long).unwrap();
        call(&s, &v, &["read", "long.rs", "--lines", "1..5"], "", None).unwrap();
        let e = call(&s, &v, &["write", "long.rs"], "fn main() {}\n", None).unwrap_err().0;
        assert!(e.contains("would cut long.rs from 300 lines to 1") && e.contains("`remove long.rs` first"), "{e}");
        assert_eq!(fs::read_to_string(s.root.join("long.rs")).unwrap(), long);
        let half: String = long.lines().take(150).map(|l| format!("{l}\n")).collect();
        assert_eq!(call(&s, &v, &["write", "long.rs"], &half, None).unwrap(), "Wrote long.rs (300 → 150 lines)\n");
        // To replace it whole: remove it, then write it new.
        call(&s, &v, &["remove", "long.rs"], "", None).unwrap();
        assert_eq!(call(&s, &v, &["write", "long.rs"], "fn main() {}\n", None).unwrap(), "Wrote long.rs (1 line)\n");
    }
    /// D-109: a `read` path that doesn't exist is named with what it likely
    /// meant, and nothing is read.
    #[test]
    fn read_names_a_missing_path() {
        let (s, v) = fixture();
        fs::write(s.root.join("a.rs"), "fn a() {}\n").unwrap();
        for range in ["1340-1400", "1340..1400", "1340:1400"] {
            let e = call(&s, &v, &["read", "a.rs", range], "", None).unwrap_err();
            assert!(
                e.0.contains(&format!("`{range}` is not a file")) && e.0.contains("`read a.rs --lines 1340..1400`") && e.1 == 2,
                "{e:?}"
            );
        }
        let e = call(&s, &v, &["read", "a.rs:3-4"], "", None).unwrap_err().0;
        assert!(e.contains("`read a.rs --lines 3..4`"), "{e}");
        let e = call(&s, &v, &["read", "some_fn"], "", None).unwrap_err().0;
        assert!(e.contains("some_fn") && e.contains("not symbols") && e.contains("`grep some_fn`"), "{e}");
        let e = call(&s, &v, &["read", "docs/gone.md"], "", None).unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("docs/gone.md") && e.contains("no such file") && !e.contains("symbols"), "{e}");
    }
    #[test]
    fn a2_source_internal_and_path_aliases() {
        let (s, v) = fixture();
        for cmd in ["read", "write", "remove"] {
            let e = call(&s, &v, &[cmd, "src/main.gleam"], "x", None).unwrap_err();
            assert!(e.0.contains("E6001") && e.0.contains("view main") && e.0.contains("inspect main."));
        }
        for cmd in ["files", "read", "write", "remove"] {
            assert!(call(&s, &v, &[cmd, ".solace/board/x"], "x", None).unwrap_err().0.contains("E6006"));
            // The root's task store (D-77), from the root and from a project inside it.
            assert!(call(&s, &v, &[cmd, ".solace/tasks/x.json"], "x", None).unwrap_err().0.contains("E6006"));
            assert!(call(&s, &v, &[cmd, "../.solace/tasks/x.json"], "x", None).unwrap_err().0.contains("E6006"));
            // The vision (D-91).
            assert!(call(&s, &v, &[cmd, ".solace/strategy/vision.md"], "x", None).unwrap_err().0.contains("E6006"));
        }
        std::os::unix::fs::symlink(s.root.join("src/main.gleam"), s.root.join("priv/link")).unwrap();
        assert!(call(&s, &v, &["write", "priv/link"], "x", None).is_err());
        assert!(call(&s, &v, &["read", "../secret"], "", None).is_err());
    }
    #[test]
    fn a3_large_log_budget_and_tail() {
        let (s, _) = fixture();
        let mut f = fs::File::create(s.root.join("log")).unwrap();
        let block = "123456789\n".repeat(10000);
        for _ in 0..1000 {
            f.write_all(block.as_bytes()).unwrap();
        }
        let tail = read(&s.root, "log", &["--tail", "10"], 1000).unwrap();
        assert_eq!(tail.lines().count(), 10);
        let head = read(&s.root, "log", &[], 1000).unwrap();
        assert!(head.len() <= 1000 && head.contains("Next: read log --lines"));
    }
    #[test]
    fn read_lines_takes_a_range() {
        let (s, _) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\nthree\nfour\n").unwrap();
        let out = read(&s.root, "notes", &["--lines", "2..3"], 1000).unwrap();
        assert!(out.contains("two") && out.contains("three") && !out.contains("one") && !out.contains("four"), "{out}");
        for bad in ["3..2", "0..1", "x", "1,,2", "2,"] {
            assert_eq!(read(&s.root, "notes", &["--lines", bad], 1000).unwrap_err().1, 2, "{bad}");
        }
        // D-105: lines and ranges, in file order, merged, a gap shown as `…`.
        assert_eq!(read(&s.root, "notes", &["--lines", "4,1..2"], 1000).unwrap(), "1: one\n2: two\n…\n4: four\n");
        assert_eq!(read(&s.root, "notes", &["--lines", "2..3,3"], 1000).unwrap(), "2: two\n3: three\n");
        // Cut at the limit, the rest is named in the same form.
        assert_eq!(read(&s.root, "notes", &["--lines", "1,3..4"], 110).unwrap(), "1: one\nNext: read notes --lines 3..4\n");
    }
    /// D-102: `read` and `files` answer a list of paths in order; a `--lines`
    /// or `--tail` range takes one path.
    #[test]
    fn read_and_files_take_several_paths() {
        let (s, v) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\n").unwrap();
        fs::write(s.root.join("log"), "a\nb\n").unwrap();
        let out = call(&s, &v, &["read", "notes", "log"], "", None).unwrap();
        assert!(out.contains("==== notes ====\none\ntwo\n==== log ====\na\nb\n"), "{out}");
        // Files past the budget are named, not cut silently, and stay unread.
        fs::write(s.root.join("big"), "x\n".repeat(1000)).unwrap();
        let out = call(&s, &v, &["read", "big", "notes"], "", None).unwrap();
        assert!(out.len() <= 1000 && out.contains("==== big ====") && out.ends_with("Next: read notes\n"), "{out}");
        assert!(call(&s, &v, &["write", "big"], "y", None).unwrap_err().0.contains("E6005"));
        // `write` changes one file and refuses a list.
        let err = call(&s, &v, &["write", "notes", "log"], "x", None).unwrap_err();
        assert!(err.0.contains("changes one file") && err.1 == 2, "{err:?}");
        // `remove` takes a list, whole or nothing: a refused path removes none.
        let err = call(&s, &v, &["remove", "notes", "gleam.toml", "src/main.gleam", "gone"], "", None).unwrap_err();
        assert!(err.0.starts_with("nothing removed:") && err.0.contains("gleam.toml") && err.0.contains("E6001"), "{err:?}");
        assert!(err.0.contains("gone") && s.root.join("notes").exists(), "{err:?}");
        let out = call(&s, &v, &["remove", "notes", "log"], "", None).unwrap();
        assert_eq!(out, "Removed notes\nRemoved log\n");
        assert!(!s.root.join("notes").exists() && !s.root.join("log").exists());
        fs::write(s.root.join("notes"), "one\ntwo\n").unwrap();
        fs::write(s.root.join("log"), "a\nb\n").unwrap();
        let lines = call(&s, &v, &["read", "notes", "--lines", "2..2"], "", None).unwrap();
        assert!(lines.contains("two") && !lines.contains("one"), "{lines}");
        fs::create_dir_all(s.root.join("priv")).unwrap();
        fs::write(s.root.join("priv/notes.txt"), "x\n").unwrap();
        let many = call(&s, &v, &["files", "src", "priv"], "", None).unwrap();
        assert!(many.contains("src/main.gleam") && many.contains("priv"), "{many}");
        let err = call(&s, &v, &["read", "notes", "log", "--lines", "1..1"], "", None).unwrap_err();
        assert!(err.0.contains("takes one path") && err.1 == 2, "{err:?}");
    }
    /// `run` replies with the exit code, the elapsed time and the condensed output.
    #[test]
    fn run_reply_is_condensed() {
        let (s, _) = fixture();
        let v: toml::Value = concat!(
            "[commands]\nsuite = { run = [\"sh\", \"-c\", ",
            "\"printf 'running 2 tests\\\\ntest a ... ok\\\\ntest b ... FAILED\\\\n'; exit 1\"] }\n"
        )
        .parse()
        .unwrap();
        let err = call(&s, &v, &["run", "suite"], "", None).unwrap_err();
        let (prefix, rest) = err.0.split_once('\n').unwrap();
        assert!(prefix.starts_with("exit code: 1 (") && prefix.ends_with("s)"), "{prefix}");
        assert_eq!(rest, "test b ... FAILED\n");
        assert!(!passed(&err.0) && passed("exit code: 0 (0.0s)\n"));
    }

    /// D-166: a passing `run` of a recipe's last step records what it built; a failing one
    /// records nothing.
    #[test]
    fn a_passing_run_records_what_the_recipe_built() {
        let recipe = |run: &str| {
            format!(
                concat!(
                    "[commands]\nbuild-release = {{ run = [\"sh\", \"-c\", \"{}\"] }}\n",
                    "[build]\noutputs = [\"priv/app.js\"]\n",
                    "steps = [{{ name = \"release\", command = \"build-release\" }}]\n"
                ),
                run
            )
        };
        let dir = Scratch::new()
            .with(&[("gleam.toml", "name = \"fixture\"\nversion = \"1.0.0\"\n"), ("src/main.gleam", "pub fn main() { 1 }\n")]);
        let p = dir.import();
        let record = p.root.join(".solace/built.json");
        let failing = recipe("echo no >&2; exit 1");
        fs::write(p.root.join("solace.toml"), &failing).unwrap();
        let v: toml::Value = failing.parse().unwrap();
        assert!(call(&p, &v, &["run", "build-release"], "", None).unwrap_err().0.starts_with("exit code: 1"));
        assert!(!record.exists(), "a failing run records nothing");
        let passing = recipe("mkdir -p priv && echo built > priv/app.js");
        fs::write(p.root.join("solace.toml"), &passing).unwrap();
        let v: toml::Value = passing.parse().unwrap();
        call(&p, &v, &["run", "build-release"], "", None).unwrap();
        let text = fs::read_to_string(&record).unwrap();
        assert!(text.contains("priv/app.js") && text.contains("src"), "{text}");
    }

    /// D-166: a passing `run` stays passed when the record can't be written; `build plan` then
    /// falls back to file times.
    #[test]
    fn a_run_that_cannot_record_what_it_built_still_passes() {
        let recipe = concat!(
            "[commands]\nbuild-release = { run = [\"sh\", \"-c\", ",
            "\"mkdir -p priv && echo built > priv/app.js\"] }\n",
            "[build]\noutputs = [\"priv/app.js\"]\n",
            "steps = [{ name = \"release\", command = \"build-release\" }]\n"
        );
        let dir = Scratch::new()
            .with(&[("gleam.toml", "name = \"fixture\"\nversion = \"1.0.0\"\n"), ("src/main.gleam", "pub fn main() { 1 }\n")]);
        let p = dir.import();
        fs::write(p.root.join("solace.toml"), recipe).unwrap();
        // A folder where the record goes makes the write fail, not the build.
        fs::create_dir_all(p.root.join(".solace/built.json")).unwrap();
        let v: toml::Value = recipe.parse().unwrap();
        let out = call(&p, &v, &["run", "build-release"], "", None).unwrap();
        assert!(out.starts_with("exit code: 0 (") && out.contains("could not record"), "{out}");
        assert!(p.root.join("priv/app.js").is_file());
    }

    #[test]
    fn a4_new_watched_file_and_size() {
        let (s, v) = fixture();
        call(&s, &v, &["write", "README.md"], "usage", None).unwrap();
        call(&s, &v, &["write", "bin/coolcalc"], "#!/bin/sh\ngleam run -- \"$@\"\n", None).unwrap();
        assert_eq!(call(&s, &v, &["files", "README.md"], "", None).unwrap(), "README.md [watched; 1 lines]\n");
        assert!(s.find("usage").unwrap().contains("No matches"));
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        fs::write(s.root.join("README.md"), "usage changed").unwrap();
        assert!(call(&s, &v, &["write", "README.md"], "new", None).unwrap_err().0.contains("E6005"));
        call(&s, &v, &["write", "README.md", "--replace"], "new", Some("usage")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("README.md")).unwrap(), "new changed");
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        call(&s, &v, &["write", "README.md"], "replacement", None).unwrap();
        // Files follow the same replacement rules as symbols (E0105, `--all`).
        call(&s, &v, &["write", "README.md", "--replace"], "two two", Some("replacement")).unwrap();
        assert!(call(&s, &v, &["write", "README.md", "--replace"], "x", Some("two")).unwrap_err().0.contains("E0105 `old` occurs 2 times"));
        call(&s, &v, &["write", "README.md", "--replace", "--all"], "2", Some("two")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("README.md")).unwrap(), "2 2");
        call(&s, &v, &["remove", "bin/coolcalc"], "", None).unwrap();
        assert!(call(&s, &v, &["remove", "gleam.toml"], "", None).unwrap_err().0.contains("E6002"));
        assert!(!s.root.join("bin/coolcalc").exists());
        fs::write(s.root.join("binary"), b"x\xff").unwrap();
        assert!(call(&s, &v, &["write", "binary", "--replace"], "y", Some("x")).unwrap_err().0.contains("UTF-8"));
        assert_eq!(fs::read(s.root.join("binary")).unwrap(), b"x\xff");
        assert!(call(&s, &v, &["write", "README.md"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
        call(&s, &v, &["write", "priv/new"], "x", None).unwrap();
        assert!(call(&s, &v, &["write", "priv/large"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
    }
    /// D-84: a file or folder moves whole, never onto anything, and a repeat changes nothing.
    #[test]
    fn move_takes_files_and_folders_and_never_replaces() {
        let (s, v) = fixture();
        for (p, b) in [("priv/a", "a"), ("priv/b", "b"), ("docs/x/one", "1"), ("docs/x/deep/two", "2")] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let mv = |from: &str, to: &str| call(&s, &v, &["move", from, to], "", None);
        assert_eq!(mv("priv/a", "new/dir/a").unwrap(), "Moved priv/a to new/dir/a\n");
        assert_eq!(fs::read_to_string(s.root.join("new/dir/a")).unwrap(), "a");
        assert!(!s.root.join("priv/a").exists());
        assert!(mv("priv/a", "new/dir/a").unwrap().starts_with("Already moved priv/a to new/dir/a"));
        // Onto an existing file or folder: refused, and both stay as they were.
        let e = mv("priv/b", "new/dir/a").unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("already exists"), "{e}");
        assert!(mv("priv/b", "docs").unwrap_err().0.contains("already exists"));
        assert_eq!(fs::read_to_string(s.root.join("priv/b")).unwrap(), "b");
        assert_eq!(fs::read_to_string(s.root.join("new/dir/a")).unwrap(), "a");
        assert_eq!(mv("docs/x", "notes/x").unwrap(), "Moved docs/x to notes/x (2 files)\n");
        assert_eq!(fs::read_to_string(s.root.join("notes/x/deep/two")).unwrap(), "2");
        assert!(!s.root.join("docs/x").exists());
        assert!(mv("notes", "notes/inner").unwrap_err().0.contains("into itself"));
        assert!(mv("nothing", "else").unwrap_err().0.contains("nothing to move"));
        assert_eq!(call(&s, &v, &["move", "priv/b"], "", None).unwrap_err().1, 2);
    }
    /// D-84: graph, protected and internal files, gleam.toml and symlinks stay
    /// put, named directly or inside a folder, and a refused move changes nothing.
    #[test]
    fn move_refuses_what_isnt_the_agents() {
        let (s, v) = fixture();
        for (p, b) in [("AGENTS.md", "x"), ("priv/a", "a"), ("box/.gitignore", "x"), ("box/keep", "k"), ("inner/.solace/store.db", "")] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        fs::create_dir_all(s.root.join("links")).unwrap();
        std::os::unix::fs::symlink(s.root.join("priv/a"), s.root.join("links/l")).unwrap();
        for (from, to, code) in [
            ("src/main.gleam", "priv/main.gleam", "E6001"),
            ("priv/a", "src/a.gleam", "E6001"),
            ("src", "lib", "E6001"),
            ("AGENTS.md", "priv/agents", "E6002"),
            ("priv/a", "AGENTS2/.gitignore", "E6002"),
            ("box", "crate", "E6002"),
            ("gleam.toml", "priv/g", "E6002"),
            ("links", "elsewhere", "E6002"),
            ("priv/a", ".solace/a", "E6006"),
            ("priv/a", "../a", "E6006"),
            ("inner", "outer", "E6006"),
            ("priv/a", "inner/a", "E6006"),
        ] {
            let e = call(&s, &v, &["move", from, to], "", None).unwrap_err().0;
            assert!(e.contains(code), "{from} -> {to}: {e}");
        }
        let e = call(&s, &v, &["move", "src/main.gleam", "priv/main.gleam"], "", None).unwrap_err().0;
        assert!(e.contains("view main") && e.contains("inspect main."), "{e}");
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "a");
        assert!(s.root.join("src/main.gleam").exists() && s.root.join("box/keep").exists());
        for gone in ["lib", "crate", "elsewhere", "AGENTS2", "outer"] {
            assert!(!s.root.join(gone).exists(), "{gone}");
        }
    }
    /// D-84: a move between projects checks the destination by its own project's rules.
    #[test]
    fn move_between_projects() {
        let ((a, v), (b, w)) = (fixture(), fixture());
        fs::write(a.root.join("priv/a"), "a").unwrap();
        call(&a, &v, &["read", "priv/a"], "", None).unwrap();
        let out = move_files(&a.root, &v, "priv/a", &b.root, &w, "docs/a").unwrap();
        assert!(out.starts_with("Moved ") && out.contains("docs/a"), "{out}");
        assert_eq!(fs::read_to_string(b.root.join("docs/a")).unwrap(), "a");
        assert!(!a.root.join("priv/a").exists());
        // What was read counts as read under its new name.
        call(&b, &w, &["write", "docs/a"], "b", None).unwrap();
        fs::write(a.root.join("priv/c"), "c").unwrap();
        assert!(move_files(&a.root, &v, "priv/c", &b.root, &w, "src/c.gleam").unwrap_err().0.contains("E6001"));
        assert!(move_files(&a.root, &v, "priv/c", &b.root, &w, "docs/a").unwrap_err().0.contains("already exists"));
        assert!(a.root.join("priv/c").exists());
    }
    #[test]
    fn a5_validation_in_place_and_recovery() {
        let (s, _) = fixture();
        let v: toml::Value = concat!(
            r#"[validators]
"priv/**" = ["python3", "-c", "import pathlib,sys; p=pathlib.Path(sys.argv[1]); "#,
            r#"print('validator output'); assert p.read_text() == 'good' and p.with_name('sibling').read_text() == 'yes'", "{file}"]
"#
        )
        .parse()
        .unwrap();
        fs::write(s.root.join("priv/sibling"), "yes").unwrap();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        call(&s, &v, &["read", "priv/a"], "", None).unwrap();
        let e = call(&s, &v, &["write", "priv/a"], "bad", None).unwrap_err();
        assert!(e.0.contains("E6003") && e.0.contains("validator output"));
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
        assert!(call(&s, &v, &["write", "priv/new"], "bad", None).is_err());
        assert!(!s.root.join("priv/new").exists());
        call(&s, &v, &["write", "priv/a"], "good", None).unwrap();
        fs::write(s.root.join(".solace/pending/test.json"), json!({"path":"priv/a","bytes":b"original".to_vec()}).to_string()).unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
    }
    /// D-104: `write` with no path takes a file patch that adds, updates and
    /// deletes several files, with no earlier read, whole or nothing.
    #[test]
    fn a_file_patch_writes_several_files_whole_or_nothing() {
        let (s, v) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\nthree\n").unwrap();
        fs::write(s.root.join("old"), "gone\n").unwrap();
        let patch = concat!(
            "*** Begin Patch\n*** Update File: notes\n@@ one\n-two\n+2\n",
            "*** Add File: docs/new.md\n+# New\n+\n+text\n",
            "*** Delete File: old\n*** End Patch\n"
        );
        assert_eq!(call(&s, &v, &["write"], patch, None).unwrap(), "Wrote notes\nAdded docs/new.md\nRemoved old\n");
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\n");
        assert_eq!(fs::read_to_string(s.root.join("docs/new.md")).unwrap(), "# New\n\ntext\n");
        assert!(!s.root.join("old").exists());
        // Lines added with no `@@` line go at the end of the file.
        call(&s, &v, &["write"], "*** Begin Patch\n*** Update File: notes\n+four\n*** End Patch\n", None).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\nfour\n");
        // Any refused section writes nothing, and each refusal is named.
        let update = "*** Update File: notes\n-one\n+1\n";
        for (section, why) in [
            ("*** Update File: docs/new.md\n-nope\n+x\n", "E0108 in docs/new.md (hunk at line 6), these lines weren't found: `nope`"),
            ("*** Update File: docs/new.md\n-ext\n+x\n", "`ext` is only part of line 3: `text`"),
            ("*** Update File: src/main.gleam\n-x\n+y\n", "E6001"),
            ("*** Add File: solace.toml\n+x\n", "human-approved proposal"),
            ("*** Add File: .solace/x\n+x\n", "E6006"),
            ("*** Add File: docs/new.md\n+x\n", "already exists; change it with `*** Update File`"),
            ("*** Delete File: missing\n", "no such file; create it with `*** Add File`"),
            ("*** Delete File: gleam.toml\n", "can't be removed"),
            ("*** Add File: priv/large\n+", "E6004"),
            ("*** Update File: notes\n-two\n+3\n", "names notes twice"),
        ] {
            let section = if why == "E6004" { format!("{section}{}\n", "x".repeat(101)) } else { section.to_string() };
            let e = call(&s, &v, &["write"], &format!("*** Begin Patch\n{update}{section}*** End Patch\n"), None).unwrap_err().0;
            assert!(e.starts_with("nothing written:\n") && e.contains(why), "{why}: {e}");
            if why == "E6001" {
                assert!(e.contains("view main") && e.contains("inspect main."), "{e}");
            }
            assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\nfour\n", "{why}");
        }
        for (patch, why) in [
            ("*** Begin Patch\n*** Update File: notes\n-one\n+one\n*** End Patch\n", "E0108 the patch changes nothing"),
            ("*** Begin Patch\n*** Update File: notes\n*** Move to: other\n*** End Patch\n", "use `move <from> <to>`"),
            ("*** Begin Patch\n*** Delete File: notes\nx\n*** End Patch\n", "takes no lines"),
            ("*** Begin Patch\n*** Add File: priv/x\nx\n*** End Patch\n", "`*** Add File: priv/x` (line 2) has no `+` lines"),
            ("*** Begin Patch\n*** End Patch\n", "the patch has no `*** Add File: <path>`"),
        ] {
            let e = call(&s, &v, &["write"], patch, None).unwrap_err().0;
            assert!(e.contains(why), "{why}: {e}");
            // D-141: a fault in the whole patch is resent, not fixed with `--replace`.
            if why.contains("no `+` lines") || why.contains("has no `*** Add File") {
                assert!(e.contains("Nothing was written.") && e.contains(drafts::RESEND) && !e.contains("--replace"), "{e}");
            }
        }
        // With no path, `text` must be a file patch; with a list, `write` points to one.
        let e = call(&s, &v, &["write"], "plain text", None).unwrap_err();
        assert!(e.0.contains("takes a file patch") && e.1 == 2, "{e:?}");
        let e = call(&s, &v, &["write", "notes", "docs/new.md"], "x", None).unwrap_err();
        assert!(e.0.contains("changes one file") && e.0.contains("*** Update File"), "{e:?}");
        // A `submit` patch that names a file other than code says to use `write`.
        let e = drafts::patch_blocks(&s, "*** Begin Patch\n*** Update File: notes\n-one\n+1\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("`notes` (line 2) is no symbol or module; a file other than code changes with `write`"), "{e}");
        let e = drafts::patch_blocks(&s, "*** Begin Patch\n*** Add File: docs/x.md\n+# X\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("`docs/x.md` (line 2) is no symbol or module"), "{e}");
    }
    /// D-120: a refused file patch is kept as a draft; `write --draft <id>
    /// --replace` fixes its bad line and retries the whole patch, matched
    /// against the files as they are then.
    #[test]
    fn a_refused_file_patch_is_fixed_in_its_draft() {
        let (s, v) = fixture();
        fs::write(s.root.join("notes"), "one\n").unwrap();
        fs::write(s.root.join("spec"), "A failed read does not stop the call (D-109).\n").unwrap();
        let patch = concat!(
            "*** Begin Patch\n*** Update File: notes\n-one\n+1\n",
            "*** Update File: spec\n-does not stop the call (D-109).\n",
            "+does not stop the call (D-119).\n*** End Patch\n"
        );
        let e = call(&s, &v, &["write"], patch, None).unwrap_err().0;
        assert!(
            e.starts_with("nothing written:\n")
                && e.contains("is only part of line 1: `A failed read")
                && !e.contains("Nothing was written.")
                && e.contains("Fix only the failing line")
                && e.contains("other lines")
                && e.contains("don't resend the whole patch")
                && e.contains("lines of the patch, prefixes included")
                && e.contains("write --draft"),
            "{e}"
        );
        let bad = "*** Begin Patch\n*** Update File: notes\none\n+1\n*** End Patch\n";
        let bad_e = call(&s, &v, &["write"], bad, None).unwrap_err().0;
        assert!(bad_e.contains("`old` and `text` are lines of the patch, prefixes included"), "{bad_e}");
        assert!(bad_e.contains("with `old` and `text`, which are lines of the patch, prefixes included"), "{bad_e}");
        let id = e.split("Draft ").nth(1).and_then(|r| r.split(' ').next()).unwrap().to_string();
        assert_eq!(drafts::view(&s, &id).unwrap(), patch);
        assert!(drafts::list(&s).unwrap().contains(&format!("{id}: [\"write\"] (current)")));
        // `submit --draft` names the command that retries it.
        let e = drafts::submit(&s, &["submit", "--draft", &id], &mut || Ok(String::new()), None).unwrap_err().0;
        assert!(e.contains("is a file patch; retry it with `write --draft"), "{e}");
        // A retry matches the files as they are: a changed one refuses it, and the fixed draft stays.
        let (old, fix) = ("-does not stop the call (D-109).\n+does", "-A failed read does not stop the call (D-109).\n+A failed read does");
        fs::write(s.root.join("notes"), "uno\n").unwrap();
        let e = call(&s, &v, &["write", "--draft", &id, "--replace"], fix, Some(old)).unwrap_err().0;
        assert!(
            e.contains("these lines weren't found: `one`")
                && e.contains(&format!("Draft {id} retained"))
                && e.contains("The replacement was applied: the draft now holds it"),
            "{e}"
        );
        assert_eq!(fs::read_to_string(s.root.join("spec")).unwrap(), "A failed read does not stop the call (D-109).\n");
        fs::write(s.root.join("notes"), "one\n").unwrap();
        assert_eq!(call(&s, &v, &["write", "--draft", &id], "", None).unwrap(), "Wrote notes\nWrote spec\n");
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "1\n");
        assert_eq!(fs::read_to_string(s.root.join("spec")).unwrap(), "A failed read does not stop the call (D-119).\n");
        // A retry that applies removes the draft.
        assert!(call(&s, &v, &["write", "--draft", &id], "", None).unwrap_err().0.contains("E0106"));
        // A one-file patch has no other lines to keep.
        let single = "*** Begin Patch\n*** Update File: notes\n-nope\n+one\n*** End Patch\n";
        let e = call(&s, &v, &["write"], single, None).unwrap_err().0;
        assert!(
            e.starts_with("nothing written:\n")
                && !e.contains("Nothing was written.")
                && e.contains("Fix the failing line")
                && e.contains("--replace")
                && !e.contains("other lines"),
            "{e}"
        );
    }
    /// A file patch writes every file before it validates any; one that fails
    /// puts every file back and leaves no journal.
    #[test]
    fn a_file_patch_that_fails_validation_puts_every_file_back() {
        let (s, _) = fixture();
        let v: toml::Value = concat!(
            r#"[validators]
"priv/**" = ["python3", "-c", "import pathlib,sys; p=pathlib.Path(sys.argv[1]); "#,
            r#"print('validator output'); assert p.read_text() == 'good\\n' and pathlib.Path('notes').read_text() == 'new\\n'", "{file}"]
"#
        )
        .parse()
        .unwrap();
        fs::write(s.root.join("notes"), "old\n").unwrap();
        let patch =
            |text: &str| format!("*** Begin Patch\n*** Update File: notes\n-old\n+new\n*** Add File: priv/a\n+{text}\n*** End Patch\n");
        let e = call(&s, &v, &["write"], &patch("bad"), None).unwrap_err().0;
        assert!(e.starts_with("nothing written:\nE6003") && e.contains("validator output"), "{e}");
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "old\n");
        assert!(!s.root.join("priv/a").exists());
        assert!(fs::read_dir(s.root.join(".solace/pending")).unwrap().next().is_none());
        // The validator sees the other file of the patch already written.
        call(&s, &v, &["write"], &patch("good"), None).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "good\n");
    }
    /// A pending undo record whose file changed: recovery adds the rows (D-77 point 7).
    #[test]
    fn undo_record_recovery_adds_rows_when_the_file_changed() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(
            s.root.join(".solace/pending/x.json"),
            json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string(),
        )
        .unwrap();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let rows: Vec<(String, String, String)> =
            s.db.prepare("SELECT task,entity,name FROM board_touches WHERE task='t1'")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
        assert_eq!(rows, [("t1".to_string(), "file:priv/a".to_string(), "file:priv/a".to_string())]);
        assert_eq!(fs::read(board::before_file(&s.root, "t1", "priv/a")).unwrap(), b"original");
        assert!(!s.root.join(".solace/pending/x.json").exists());
    }
    /// A pending undo record whose file never changed: recovery drops it, adds nothing.
    #[test]
    fn undo_record_recovery_drops_an_unchanged_file() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(
            s.root.join(".solace/pending/x.json"),
            json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string(),
        )
        .unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let n: i64 = s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task='t1'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!s.root.join(".solace/pending/x.json").exists());
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
    }
    /// A declared run's record: rows for changed files, and for files the process created.
    #[test]
    fn declared_run_record_adds_rows_for_changed_and_created_files() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        // The snapshot lists every watched file, as `run` takes it; one it doesn't list was created.
        let manifest = fs::read(s.root.join("gleam.toml")).unwrap();
        let files = json!([{"path":"gleam.toml","bytes":manifest},{"path":"priv/a","bytes":b"original".to_vec()}]);
        fs::write(s.root.join(".solace/pending/run-r1.json"), json!({"task":"t1","run":"r1","files":files}).to_string()).unwrap();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        fs::write(s.root.join("priv/created"), "new").unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let entities: Vec<String> =
            s.db.prepare("SELECT entity FROM board_touches WHERE task='t1' ORDER BY entity")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
        assert_eq!(entities, ["file:priv/a", "file:priv/created"]);
        assert_eq!(fs::read(board::before_file(&s.root, "t1", "priv/created").with_extension("absent")).unwrap(), b"");
    }
    /// D-136 after a crash: a recovered record that names the folders before
    /// stores where undo stops, not `null`; an old record still stores `null`.
    #[test]
    fn recovered_records_keep_the_folders_that_were_there() {
        let (s, _) = fixture();
        fs::create_dir_all(s.root.join("priv/empty")).unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        let manifest = fs::read(s.root.join("gleam.toml")).unwrap();
        let files = json!([{"path":"gleam.toml","bytes":manifest}]);
        fs::write(
            s.root.join(".solace/pending/run-r1.json"),
            json!({"task":"t1","run":"r1","files":files,"folders":["priv","priv/empty"]}).to_string(),
        )
        .unwrap();
        fs::write(
            s.root.join(".solace/pending/x.json"),
            json!({"task":"t2","run":"r2","path":"priv/one/b","bytes":null,"kept":"priv"}).to_string(),
        )
        .unwrap();
        fs::write(s.root.join(".solace/pending/y.json"), json!({"task":"t3","run":"r3","path":"priv/old/c","bytes":null}).to_string())
            .unwrap();
        fs::create_dir_all(s.root.join("priv/empty/new")).unwrap();
        fs::write(s.root.join("priv/empty/new/a"), "new").unwrap();
        fs::create_dir_all(s.root.join("priv/one")).unwrap();
        fs::write(s.root.join("priv/one/b"), "new").unwrap();
        fs::create_dir_all(s.root.join("priv/old")).unwrap();
        fs::write(s.root.join("priv/old/c"), "new").unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let before = |t: &str, e: &str| -> String {
            s.db.query_row("SELECT data FROM board_before WHERE task=?1 AND entity=?2", params![t, e], |r| r.get(0)).unwrap()
        };
        assert_eq!(before("t1", "file:priv/empty/new/a"), json!({"kept":"priv/empty"}).to_string());
        assert_eq!(before("t2", "file:priv/one/b"), json!({"kept":"priv"}).to_string());
        assert_eq!(before("t3", "file:priv/old/c"), "null");
    }
    /// An undo record for a file that doesn't exist yet names the nearest folder that does.
    #[test]
    fn kept_on_disk_names_the_nearest_existing_folder() {
        let (s, _) = fixture();
        fs::create_dir_all(s.root.join("priv/empty")).unwrap();
        assert_eq!(board::kept_on_disk(&s.root, "priv/empty/a/b/c"), json!({"kept":"priv/empty"}));
        assert_eq!(board::kept_on_disk(&s.root, "none/x"), json!({"kept":""}));
    }
    /// A second change to a file keeps only its own run's record, so a refused
    /// write never deletes the earlier change's record.
    #[test]
    fn same_run_matches_only_its_own_record() {
        let (s, _) = fixture();
        let journal = s.root.join(".solace/pending/x.json");
        assert!(!same_run(&journal, &json!("r1")));
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(&journal, json!({"task":"t1","run":"r1","path":"priv/a","bytes":null}).to_string()).unwrap();
        assert!(same_run(&journal, &json!("r1")));
        assert!(!same_run(&journal, &json!("r2")));
        fs::write(&journal, "{torn").unwrap();
        assert!(!same_run(&journal, &json!("r1")));
    }
    /// A crash after the commit: the rows are already there, recovery adds no duplicate.
    #[test]
    fn recovery_skips_rows_already_committed() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES ('t1','file:priv/a','r1','file:priv/a')", []).unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(
            s.root.join(".solace/pending/x.json"),
            json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string(),
        )
        .unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let n: i64 = s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task='t1'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        assert!(!s.root.join(".solace/pending/x.json").exists());
    }
    #[test]
    fn a6_manifest_validation() {
        let (s, mut v) = fixture();
        v["watch_limit"] = toml::Value::Integer(1000);
        let before = fs::read(s.root.join("gleam.toml")).unwrap();
        call(&s, &v, &["read", "gleam.toml"], "", None).unwrap();
        assert!(call(&s, &v, &["write", "gleam.toml"], "=broken", None).unwrap_err().0.contains("E6003"));
        assert_eq!(fs::read(s.root.join("gleam.toml")).unwrap(), before);
        // The package name follows the rule `new_project` uses.
        assert!(call(&s, &v, &["write", "gleam.toml"], "name=\"1abc\"\nversion=\"1.0.0\"\n", None).unwrap_err().0.contains("E6003"));
        assert_eq!(fs::read(s.root.join("gleam.toml")).unwrap(), before);
        // Dependencies are Gleam's to check, not Solace's.
        call(
            &s,
            &v,
            &["write", "gleam.toml"],
            "name=\"fixture\"\nversion=\"1.0.0\"\n[dependencies]\nmissing={path=\"./does_not_exist\"}",
            None,
        )
        .unwrap();
    }
    #[test]
    fn a7_stale_and_exact_replace() {
        let (s, v) = fixture();
        call(&s, &v, &["write", "priv/a"], "first", None).unwrap();
        call(&s, &v, &["read", "priv/a"], "", None).unwrap();
        fs::write(s.root.join("priv/a"), "first changed").unwrap();
        assert!(call(&s, &v, &["write", "priv/a"], "new", None).unwrap_err().0.contains("E6005"));
        call(&s, &v, &["write", "priv/a", "--replace"], "second", Some("first")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "second changed");
        // `old` without --replace still replaces only that text (an agent wiped a decision log this way).
        call(&s, &v, &["write", "priv/a"], "third", Some("second")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "third changed");
        // The same for a settings proposal, which is built by `edited` alone.
        assert_eq!(
            edited(b"projects = []\n[commands]\n", "[commands]\nx = 1", false, false, Some("[commands]")).unwrap(),
            "projects = []\n[commands]\nx = 1\n"
        );
        // A tail read doesn't allow a whole-file write.
        fs::write(s.root.join("priv/b"), "one\ntwo\n").unwrap();
        call(&s, &v, &["read", "priv/b", "--tail", "1"], "", None).unwrap();
        assert!(call(&s, &v, &["write", "priv/b"], "new", None).unwrap_err().0.contains("E6005"));
    }
    #[test]
    fn a8_protected_human_only_and_reload() {
        let (s, v) = fixture();
        assert!(call(&s, &v, &["write", "solace.toml"], "", None).is_err());
        fs::write(s.root.join("solace.toml"), "unwatched=[\"docs/**\"]").unwrap();
        call(&s, &settings(&s.root).unwrap(), &["write", "docs/new"], "x", None).unwrap();
        fs::write(s.root.join("solace.toml"), "unwatched=1").unwrap();
        assert!(settings(&s.root).unwrap_err().0.contains("E6020"));
        for (old, fix) in [("resources=[]", "delete it"), ("ignore=[]", "now `unwatched`"), ("resource_limit=1", "now `watch_limit`")] {
            assert!(parse_settings(old).unwrap_err().0.contains(fix));
        }
    }
    #[test]
    fn a9_declared_args_unknown_timeout() {
        let (s, _) = fixture();
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],checks=\"*\"}").unwrap_err().0.contains("E6020"));
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],checks=[\"*\"]}").is_ok());
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],test=\"yes\"}").unwrap_err().0.contains("E6020"));
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],test=true}").is_ok());
        let v: toml::Value = "[commands]\nsmoke={run=[\"sleep\",\"5\"],timeout=1}".parse().unwrap();
        assert!(call(&s, &v, &["run", "smoke", "extra"], "", None).unwrap_err().0.contains("E6011"));
        let e = call(&s, &v, &["run", "nope"], "", None).unwrap_err();
        assert!(e.0.contains("E6010") && e.0.contains("smoke"));
        assert!(call(&s, &v, &["run", "smoke"], "", None).unwrap_err().0.contains("E6012"));
    }
    /// A `run` whose recipe can't be read still runs: the record is skipped with
    /// a note, not a refusal (D-166).
    #[test]
    fn a_run_with_an_unreadable_recipe_still_runs() {
        let (s, _) = fixture();
        fs::write(s.root.join("solace.toml"), "unwatched=1\n").unwrap();
        let v: toml::Value = "[commands]\nsmoke={run=[\"true\"]}".parse().unwrap();
        let out = call(&s, &v, &["run", "smoke"], "", None).unwrap();
        assert!(out.contains("could not record what it built") && out.contains("E6020"), "{out}");
    }
    #[test]
    fn a10_agent_config_golden() {
        let (s, _) = fixture();
        for agent in ["codex", "claude"] {
            let dir = s.root.join(agent);
            agent_config(agent, &dir, Path::new("/bin/solace"), 3600).unwrap();
            let files =
                if agent == "codex" { vec!["config.toml", "launch.json"] } else { vec!["mcp.json", "settings.json", "launch.json"] };
            for file in files {
                let actual = fs::read_to_string(dir.join(file)).unwrap().replace(&dir.to_string_lossy().to_string(), "/agent");
                let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{agent}-{file}"));
                assert_eq!(actual, fs::read_to_string(golden).unwrap());
            }
        }
    }
    /// D-27: the skills folder is the humans' house rules: its files are
    /// protected, a write into it is refused naming skills, a patch that adds
    /// one is refused whole, a move out is refused, and `read` works.
    #[test]
    fn the_skills_folder_is_read_only_for_agents() {
        let (p, v) = fixture();
        fs::create_dir_all(p.root.join("skills/design")).unwrap();
        fs::write(p.root.join("skills/design/SKILL.md"), "---\ndescription: Layout.\n---\n# Layout rules\n").unwrap();
        fs::write(p.root.join("skills/voice.md"), "Speak plainly\n").unwrap();
        assert_eq!(tier(&p.root, &v, "skills", None), "protected");
        assert_eq!(tier(&p.root, &v, "skills/voice.md", None), "protected");
        assert_eq!(tier(&p.root, &v, "skills/design/SKILL.md", None), "protected");
        assert_eq!(tier(&p.root, &v, "voice.md", None), "watched");
        let r = call(&p, &v, &["read", "skills/voice.md"], "", None).unwrap();
        assert!(r.contains("Speak plainly"), "{r}");
        // A direct write is refused, naming skills.
        let w = call(&p, &v, &["write", "skills/voice.md"], "new\n", None).unwrap_err().0;
        assert!(w.contains("E6002") && w.contains("skills are the humans' house rules"), "{w}");
        // So is a patch that adds a file there; the whole patch writes none.
        let patch = "*** Begin Patch\n*** Add File: skills/new/SKILL.md\n+x\n*** Update File: README.md\n+hi\n*** End Patch\n";
        let w = call(&p, &v, &["write"], patch, None).unwrap_err().0;
        assert!(w.contains("skills are the humans' house rules"), "{w}");
        assert!(!p.root.join("README.md").exists() && !p.root.join("skills/new").exists());
        // A move out of the folder is refused, and moves nothing.
        let m = call(&p, &v, &["move", "skills/voice.md", "voice.md"], "", None).unwrap_err().0;
        assert!(m.contains("E6002") && m.contains("can't be moved"), "{m}");
        assert!(p.root.join("skills/voice.md").exists() && !p.root.join("voice.md").exists());
    }
    /// D-27: a skills folder that is a nested project (D-57) collides with its
    /// store's internal tier, so `settings` refuses it.
    #[test]
    fn a_skills_folder_that_is_a_nested_project_is_refused() {
        let root = Scratch::new().with(&[
            ("gleam.toml", "name = \"outer\"\nversion = \"1.0.0\"\n"),
            ("solace.toml", "skills = \"inner\"\n"),
            ("inner/gleam.toml", "name = \"inner\"\nversion = \"1.0.0\"\n"),
            ("inner/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
        ]);
        import(&root.join("inner")).unwrap();
        let e = settings(&root).unwrap_err().0;
        assert!(e.contains("E6020") && e.contains("nested project"), "{e}");
        // A folder inside a nested project is refused too: its store's
        // internal tier covers the whole project, skills folder included.
        fs::write(root.join("solace.toml"), "skills = \"inner/skills\"\n").unwrap();
        let e = settings(&root).unwrap_err().0;
        assert!(e.contains("E6020") && e.contains("nested project"), "{e}");
        // The skills folder itself is fine, and so is the default.
        fs::write(root.join("solace.toml"), "skills = \"house\"\n").unwrap();
        assert!(settings(&root).is_ok());
    }

    /// D-158 point 1, D-174: a Rust project's `.rs` files are symbols, its
    /// Cargo and rustup settings are protected, and `target/` is unwatched.
    #[test]
    fn a_rust_projects_files_have_their_tiers() {
        let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        assert_eq!(tier(&root, &v, "src/lib.rs", None), "graph");
        assert_eq!(tier(&root, &v, "src/board/agents.rs", None), "graph");
        assert_eq!(tier(&root, &v, "tests/acceptance.rs", None), "graph");
        assert_eq!(tier(&root, &v, "build.rs", None), "protected");
        assert_eq!(tier(&root, &v, ".cargo/config.toml", None), "protected");
        assert_eq!(tier(&root, &v, ".cargo/config", None), "protected");
        assert_eq!(tier(&root, &v, "rust-toolchain.toml", None), "protected");
        assert_eq!(tier(&root, &v, "rust-toolchain", None), "protected");
        assert_eq!(tier(&root, &v, "member/build.rs", None), "protected");
        assert_eq!(tier(&root, &v, "member/Cargo.toml", None), "protected");
        assert_eq!(tier(&root, &v, "target/debug/app", None), "unwatched");
        assert_eq!(tier(&root, &v, "target/debug/build/x/build.rs", None), "unwatched");
        assert_eq!(tier(&root, &v, "Cargo.toml", None), "watched");
        let got = call(&store, &v, &["files"], "", None).unwrap();
        assert!(got.contains("src/lib.rs [graph]"), "{got}");
    }

    /// D-158 point 4: a file patch that changes Cargo.toml's dependency keys
    /// is refused, like `write <path>`, and nothing is written.
    #[test]
    fn a_cargo_toml_patch_that_changes_a_dependency_is_refused() {
        let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        let patch =
            "*** Begin Patch\n*** Update File: Cargo.toml\n@@\n edition = \"2021\"\n+[dependencies]\n+serde = \"1\"\n*** End Patch\n";
        let e = call(&store, &v, &["write"], patch, None).unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("human-approved proposal"), "{e}");
        assert!(e.contains("`write Cargo.toml` alone") && e.contains(drafts::WHOLE) && !e.contains("--replace"), "{e}");
        assert!(!fs::read_to_string(root.join("Cargo.toml")).unwrap().contains("serde"));
    }

    /// A Rust project's miss, and an absent symbol-held `.rs` path, point at `find`.
    #[test]
    fn grep_points_at_find_for_rust_source() {
        let (root, s) = rust_project("pub fn helper() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        let got = call(&s, &v, &["grep", "absent"], "", None).unwrap();
        assert!(got.starts_with("No matches") && got.contains("use `find`"), "{got}");
        fs::remove_file(root.join("src/lib.rs")).unwrap();
        let got = call(&s, &v, &["grep", "absent", "--in", "src/lib.rs"], "", None).unwrap();
        assert!(got.contains("src/lib.rs is source held as symbols") && got.contains("use `find`"), "{got}");
        assert!(!got.contains("no such file"), "{got}");
    }

    #[test]
    fn view_refuses_rust_graph_source() {
        let (root, s) = rust_project("pub fn helper() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        for args in [&["view", "src/lib.rs"][..], &["view", "src/lib.rs::helper"][..], &["view", "./src/lib.rs"][..]] {
            let e = call(&s, &v, args, "", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("view crate") && e.contains("inspect crate."), "{e}");
        }
        let e = call(&s, &v, &["view", "foo/../src/lib.rs"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("view crate"), "{e}");
        let e = call(&s, &v, &["view", "../src/lib.rs"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("outside the project"), "{e}");
        for cmd in ["view", "read"] {
            let e = call(&s, &v, &[cmd, "src/new.rs"], "", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("submit --new new"), "{cmd}: {e}");
        }
        for cmd in ["write", "remove"] {
            let e = call(&s, &v, &[cmd, "src/lib.rs"], "x", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("view crate"), "{cmd}: {e}");
        }
        let e = call(&s, &v, &["move", "src/lib.rs", "src/other.rs"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("view crate"), "{e}");
        let e = call(&s, &v, &["view", "src/bin/x.rs"], "", None).unwrap_err().0;
        assert!(e.contains("can't tell which module"), "{e}");
    }

    #[test]
    fn graph_paths_share_one_hint() {
        let (s, v) = fixture();
        let viewed = call(&s, &v, &["view", "src/main.gleam"], "", None).unwrap_err().0;
        let read = call(&s, &v, &["read", "src/main.gleam"], "", None).unwrap_err().0;
        assert_eq!(viewed, read);
        assert!(
            viewed.contains("E6001")
                && viewed.contains("view main --header")
                && viewed.contains("view main --bodies")
                && viewed.contains("inspect main."),
            "{viewed}"
        );
        for cmd in ["view", "read"] {
            let e = call(&s, &v, &[cmd, "src/new.gleam"], "", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("submit --new new"), "{cmd}: {e}");
            let e = call(&s, &v, &[cmd, "test/new.gleam"], "", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("submit --new new_test"), "{cmd}: {e}");
        }
    }

    #[test]
    fn view_reads_python_beside_rust_graph() {
        let (root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
        fs::write(root.join("tool.py"), "def main():\n    pass\n").unwrap();
        let v = settings(&root).unwrap();
        let out = call(&s, &v, &["view", "tool.py"], "", None).unwrap();
        assert!(out.contains("def main"), "{out}");
        let out = call(&s, &v, &["view", "tool.py::main"], "", None).unwrap();
        assert!(out.starts_with("tool.py:1..2"), "{out}");
        let e = call(&s, &v, &["view", "src/lib.rs"], "", None).unwrap_err().0;
        assert!(e.contains("E6001"), "{e}");
    }

    /// A `test/foo.gleam` path resolves to the held `foo_test` module.
    #[test]
    fn a_held_test_module_is_named_for_its_path() {
        let s = project(&[("test/foo_test.gleam", "pub fn foo_test() { 1 }\n")]);
        let v = toml::Value::Table(Default::default());
        let e = call(&s, &v, &["view", "test/foo.gleam"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("view foo_test") && !e.contains("submit --new"), "{e}");
    }

    /// `read`, `write`, and `move` resolve an internal parent part before the graph
    /// check, so they give the same hint as `view` (D-181).
    #[test]
    fn parent_paths_share_the_graph_hint() {
        let (root, s) = rust_project("pub fn helper() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        for args in
            [&["read", "foo/../src/lib.rs"][..], &["write", "foo/../src/lib.rs"][..], &["move", "foo/../src/lib.rs", "src/other.rs"][..]]
        {
            let e = call(&s, &v, args, "x", None).unwrap_err().0;
            assert!(e.contains("E6001") && e.contains("view crate"), "{args:?}: {e}");
        }
    }

    /// When both `foo` and `foo_test` are held, `test/foo.gleam` names `foo_test`.
    #[test]
    fn a_both_held_test_module_prefers_the_test_name() {
        let s = project(&[("src/foo.gleam", "pub fn foo() { 1 }\n"), ("test/foo_test.gleam", "pub fn foo_test() { 1 }\n")]);
        let v = toml::Value::Table(Default::default());
        let e = call(&s, &v, &["view", "test/foo.gleam"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("view foo_test"), "{e}");
    }

    /// `remove` shares the parent-path hint, and every command rejects `/../...`.
    #[test]
    fn parent_and_absolute_paths_are_checked_for_every_command() {
        let (root, s) = rust_project("pub fn helper() -> i32 {\n    1\n}\n");
        let v = settings(&root).unwrap();
        let e = call(&s, &v, &["remove", "foo/../src/lib.rs"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("view crate"), "{e}");
        for args in [
            &["read", "/../src/lib.rs"][..],
            &["write", "/../src/lib.rs"][..],
            &["remove", "/../src/lib.rs"][..],
            &["move", "/../src/lib.rs", "src/other.rs"][..],
        ] {
            let e = call(&s, &v, args, "x", None).unwrap_err().0;
            assert!(
                e.contains("E6002") && e.contains("expected a path inside the project") && e.contains(args[1]) && !e.contains("//"),
                "{args:?}: {e}"
            );
        }
        let e = call(&s, &v, &["write"], "*** Add File: /../docs/x\n+x\n", None).unwrap_err().0;
        assert!(
            e.contains("E6002") && e.contains("expected a path inside the project") && e.contains("/../docs/x") && !e.contains("//"),
            "{e}"
        );
    }

    /// A `test/` path names the `_test` module even when only the source module is held.
    #[test]
    fn a_test_path_without_the_test_module_names_the_test_module() {
        let s = project(&[("src/foo.gleam", "pub fn foo() { 1 }\n")]);
        let v = toml::Value::Table(Default::default());
        let e = call(&s, &v, &["view", "test/foo.gleam"], "", None).unwrap_err().0;
        assert!(e.contains("E6001") && e.contains("submit --new foo_test") && !e.contains("view foo"), "{e}");
    }

    /// A task batch belongs to the task server: recovery leaves its journal
    /// and files alone.
    #[test]
    fn a_task_batch_journal_is_left_for_the_task_server() {
        let (s, _) = fixture();
        let before = br#"{"id":"t1","column":"backlog"}"#.to_vec();
        let p = s.root.join(".solace/tasks/t1.json");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, br#"{"id":"t1","column":"planning"}"#).unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        let record = json!({"files": [{"path": ".solace/tasks/t1.json", "bytes": before}]});
        fs::write(s.root.join(".solace/pending/tasks-crash.json"), record.to_string()).unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        assert_eq!(fs::read(&p).unwrap(), br#"{"id":"t1","column":"planning"}"#);
        assert!(s.root.join(".solace/pending/tasks-crash.json").exists());
    }

    #[test]
    fn a_protected_file_patch_says_to_send_the_whole_text() {
        let (s, v) = fixture();
        let patch = "*** Begin Patch\n*** Update File: solace.toml\n+x = 1\n*** End Patch\n";
        let e = call(&s, &v, &["write"], patch, None).unwrap_err().0;
        assert!(e.contains("`write solace.toml` alone"), "{e}");
        assert!(e.contains("exact new text"), "{e}");
        assert!(e.contains("`reason`"), "{e}");
        assert!(e.contains(drafts::WHOLE), "{e}");
        assert!(!e.contains("--replace"), "{e}");
    }

    #[test]
    fn a_path_that_repeats_its_project_is_named_without_it() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"a\", \"b\"]\n"),
            ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
            ("a/src/app.gleam", "pub fn one() { 1 }\n"),
            ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ]);
        for d in ["a", "b"] {
            import(&root.join(d)).unwrap();
        }
        let s = Store::at(&root.join("a")).unwrap();
        let v = settings(&s.root).unwrap();
        let e = call(&s, &v, &["read", "a/src/app.gleam"], "", None).unwrap_err().0;
        assert!(e.contains("view app --header") && e.contains("view app --bodies"), "{e}");
        assert!(!e.contains("did you mean"), "{e}");
        fs::write(s.root.join("notes"), "notes\n").unwrap();
        let e = call(&s, &v, &["read", "a/notes"], "", None).unwrap_err().0;
        assert!(e.contains("; did you mean `notes`?"), "{e}");
    }

    #[test]
    fn grep_in_from_the_root_names_the_project_with_the_file() {
        let root = Scratch::new()
            .with(&[("solace.toml", "projects = [\"core\"]\n"), ("core/gleam.toml", "name = \"core\"\nversion = \"1.0.0\"\n")]);
        let v = settings(&root).unwrap();
        let e = grep(&root, &v, "needle --in gleam.toml", 10_000).unwrap_err().0;
        assert!(e.contains("it is in project core: add --project core"), "{e}");
    }

    #[test]
    fn a_read_from_another_project_names_its_project() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"a\", \"b\"]\n"),
            ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
            ("a/src/app.gleam", "pub fn one() { 1 }\n"),
            ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
            ("b/src/plan.gleam", "pub fn answer() { 42 }\n"),
        ]);
        for d in ["a", "b"] {
            import(&root.join(d)).unwrap();
        }
        let s = Store::at(&root.join("a")).unwrap();
        let v = settings(&s.root).unwrap();
        let e = with_root(&s.root, || call(&s, &v, &["read", "b/src/plan.gleam"], "", None)).unwrap_err().0;
        assert!(e.contains("view plan --header --project b") && e.contains("view plan --bodies --project b"), "{e}");
    }

    #[test]
    fn a_module_in_another_project_is_named_with_its_project() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"protocol\"]\n"),
            ("protocol/gleam.toml", "name = \"protocol\"\nversion = \"1.0.0\"\n"),
            ("protocol/src/protocol.gleam", "pub fn one() { 1 }\n"),
        ]);
        import(&root).unwrap();
        import(&root.join("protocol")).unwrap();
        let s = Store::at(&root).unwrap();
        for (command, name) in [("view", "protocol"), ("inspect", "protocol.one")] {
            let e = super::plain::<()>(&s, None, command, Some(name)).unwrap_err().0;
            assert!(e.contains("Module `protocol` is in project `protocol`; add `--project protocol`"), "{command} {name}: {e}");
        }
    }

    /// A declared command's color codes never reach the reply.
    #[test]
    fn run_reply_strips_color() {
        let (s, _) = fixture();
        let v: toml::Value =
            concat!("[commands]\nsuite = { run = [\"sh\", \"-c\", ", "\"printf '\\\\033[31mred\\\\033(B\\\\033[m\\\\n'\"] }\n")
                .parse()
                .unwrap();
        let out = call(&s, &v, &["run", "suite"], "", None).unwrap();
        let (prefix, rest) = out.split_once('\n').unwrap();
        assert!(prefix.starts_with("exit code: 0 ("), "{prefix}");
        assert_eq!(rest, "red\n");
        assert!(!out.contains('\u{1b}'));
    }
}
