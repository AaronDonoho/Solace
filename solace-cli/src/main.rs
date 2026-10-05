//! `solace`: the Phase 1a semantic store and agent protocol over Gleam
//! (specs/01-identity.md, 02-projection.md, 03-protocol.md).
//!
//! The store (`.solace/store.db`) is the source of truth. After every change
//! it writes the Gleam files back out so `gleam` can check, test and build
//! them; agents never touch those files.

// Short-lived tuples inside one function read better spelled out than named.
#![allow(clippy::type_complexity)]

mod board;
mod drafts;
mod gleam;
mod lockdown;
mod migrate;
mod outside;
mod reach;
mod scope;
mod view;
mod workspace;

use gleam::{Item, Kind, Ref};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

const REFERENCE: &str = include_str!("reference.txt");

/// The reference as agents get it at session start: the workflow and one line
/// per command (`help` has every option, D-46), and task commands only in a
/// board run.
fn agent_reference() -> String {
    let (workflow, rest) = REFERENCE.split_once("\nCommands in full").unwrap();
    let (_, rest) = rest.split_once("\nTask runs").unwrap();
    let (task, _host) = rest.split_once("\nHost commands").unwrap();
    if board::context().is_some() {
        format!("{workflow}\nTask runs{task}")
    } else {
        // no task to ask or finish outside a board run (run 18)
        format!("{}\n", workflow.replace("ask with `task ask`, or say so", "say so"))
    }
}

/// `help [command]` for agents: every option, without host commands, and
/// task commands only in a board run; with a command, only the paragraphs
/// that list it (D-46).
fn agent_help(topic: Option<&str>) -> String {
    let (_, full) = REFERENCE.split_once("Commands in full\n").unwrap();
    let (full, _host) = full.split_once("\nHost commands").unwrap();
    let full = if board::context().is_some() { full } else { full.split_once("\nTask runs").unwrap().0 };
    let Some(topic) = topic else { return format!("{full}\n") };
    let paragraphs: Vec<&str> = full.split("\n\n").collect();
    let word = |p: &str, f: &dyn Fn(&str) -> bool| {
        p.lines().any(|l| f(l) && l.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == topic))
    };
    let hits: Vec<&str> =
        paragraphs.iter().copied().filter(|p| word(p, &|l| l.starts_with("  ") && l.trim_start().starts_with(topic))).collect();
    let hits = if hits.is_empty() { paragraphs.into_iter().filter(|p| word(p, &|_| true)).collect() } else { hits };
    if hits.is_empty() { format!("No command `{topic}`. Every command:\n{full}\n") } else { hits.join("\n\n") + "\n" }
}
/// The largest reply, in bytes (about 8k tokens). Agents cut longer tool
/// output (Codex at about 10k tokens), so `inspect` and `view` trim by
/// priority to fit rather than being cut mid-document (D-14). `solace mcp
/// --budget <bytes>` sets it per agent.
const BUDGET: usize = 32_000;
const STDLIB: &[&str] = &[
    "bool",
    "bit_array",
    "bytes_tree",
    "dict",
    "dynamic",
    "float",
    "function",
    "int",
    "io",
    "list",
    "option",
    "order",
    "pair",
    "result",
    "set",
    "string",
    "string_tree",
    "uri",
];

/// A command's failure: its message and exit code (1 diagnostics, 2 usage).
#[derive(Debug)]
struct Failure(String, i32);
type Out = Result<String, Failure>;

fn fail<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(msg.into(), 1))
}
fn usage<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(format!("{}\n\nRun `solace help` for usage.", msg.into()), 2))
}
impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Self {
        Failure(format!("store error: {e}"), 1)
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure(format!("io error: {e}"), 1)
    }
}

/// Removes `flag` and its value from `args`; a flag without a value is a usage error.
fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, Failure> {
    let Some(i) = args.iter().position(|a| a == flag) else { return Ok(None) };
    if i + 1 >= args.len() {
        return usage(format!("{flag} needs a value"));
    }
    args.remove(i);
    Ok(Some(args.remove(i)))
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let result = (|| {
        let mut options = Vec::new();
        while let Some(option) = take(&mut args, "--option")? {
            options.push(option);
        }
        let old = take(&mut args, "--old")?;
        if args.first().map(String::as_str) == Some("mcp") {
            let budget = match &args[1..] {
                [] => BUDGET,
                [flag, n] if flag == "--budget" => {
                    n.parse().map_err(|_| Failure(format!("--budget takes a number of bytes, got {n}"), 2))?
                }
                _ => return usage("mcp [--budget N]"),
            };
            mcp(budget);
            return Ok(String::new());
        }
        run_options(&args, &mut stdin, old.as_deref(), BUDGET, &options)
    })();
    let code = match result {
        Ok(out) => {
            print!("{out}");
            0
        }
        Err(Failure(msg, code)) => {
            println!("{msg}");
            code
        }
    };
    std::process::exit(code);
}

/// `input` supplies the new code for submit commands; `old`, the text that
/// `submit <name> --replace` replaces (MCP only); `budget`, the bytes left
/// for this reply.
fn run_options(args: &[String], input: &mut dyn FnMut() -> Out, old: Option<&str>, budget: usize, options: &[String]) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.first() == Some(&"serve") {
        if board::context().is_some() {
            return lockdown::error("E7010", "serve", "host command");
        }
        let port = match a.as_slice() {
            ["serve"] => 3030,
            ["serve", "--port", p] => p.parse().map_err(|_| Failure("invalid port".into(), 2))?,
            _ => return usage("serve [--port N]"),
        };
        return board::serve(workspace::find(&env::current_dir()?)?, port);
    }
    if a.first() == Some(&"migrate") {
        if board::context().is_some() {
            return lockdown::error("E7010", "migrate", "host command");
        }
        return migrate::command(&a[1..]);
    }
    if a.first() == Some(&"task") && board::context().is_none() {
        return lockdown::error("E7010", "task", "task commands work only in a run started from the task board; this session has no task");
    }
    let rest;
    let (args, cross) = match a.iter().position(|x| *x == "--project") {
        Some(i) => {
            let dir =
                a.get(i + 1).copied().ok_or_else(|| Failure("--project needs a project folder; session start lists them".into(), 2))?;
            rest = [&args[..i], &args[i + 2..]].concat();
            match scoped(dir, &rest)? {
                Some(target) => (&rest[..], Some(target)),
                None => return read_project(dir, &rest, budget),
            }
        }
        None => (args, None),
    };
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // The task's home: where the run's agent works, and where the task is stored.
    let home = Store::root().ok();
    let _within = cross.as_deref().map(Within::enter).transpose()?;
    let Ok(root) = Store::root() else {
        board::flow(&env::current_dir()?)?;
        return run_inner(args, input, old, budget);
    };
    let context = board::context();
    let home = home.unwrap_or_else(|| root.clone());
    let guard = if let Some((tid, rid)) = &context {
        board::agent_guard(&home, tid, rid, if a.is_empty() { &["help"] } else { &a }, cross.as_deref())?
    } else {
        vec![board::lock(&root)?]
    };
    let store = board::open(&root)?;
    let home = board::open(&home)?;
    lockdown::recover(&store.root)?;
    board::flow(&store.root)?;
    let noticed = outside::absorb(&store, outside::Mode::Notice)?;
    if let Some((tid, rid)) = &context {
        if a.starts_with(&["task", "diff"]) {
            return board::task_diff(&home, tid, &a[2..], budget);
        }
        if a.first() == Some(&"task") {
            return board::task_command(&home, tid, rid, &a, input, options, guard);
        }
        // A run's settings write is a proposal the human approves (D-38).
        if a.first() == Some(&"write")
            && let Some(file) = a.get(1).map(|p| file_name(Path::new(p))).filter(|p| p == "solace.toml" || p == "AGENTS.md" || lockdown::is_gitignore(p))
        {
            let current = fs::read(store.root.join(&file)).unwrap_or_default();
            let proposed = lockdown::edited(&current, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old)?;
            return board::propose_settings(&home, &store.root, tid, rid, &file, &current, &proposed, guard);
        }
    }
    let before = if a.first().is_some_and(|c| CHANGES.contains(c)) && context.is_some() {
        let mut v = board::snapshot(&store)?;
        v.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        Some(v)
    } else {
        None
    };
    let result = run_inner(args, input, old, budget);
    if let Some((tid, _)) = &context
        && a.first().is_some_and(|c| VERIFIES.contains(c))
    {
        board::record_verification(&home, tid, a[0], &result)?;
    }
    // A process the command ran (`format`, a declared command) may have changed source: that's this command's change.
    let unread =
        if a.first().is_some_and(|c| PROCESSES.contains(c)) { outside::absorb(&store, outside::Mode::Command)? } else { String::new() };
    if let (Some(before), Some((tid, rid))) = (before, context) {
        let mut after = board::snapshot(&store)?;
        after.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        board::record(&store, &home, &tid, &rid, &before, &after)?;
    }
    match result {
        Ok(out) => Ok(format!("{noticed}{out}{unread}")),
        Err(Failure(msg, code)) => Err(Failure(format!("{noticed}{msg}{unread}"), code)),
    }
}

// Commands by what they do. Every check that depends on the command uses these.
/// Plain reads; with `--project`, they read another project.
const READS: &[&str] = &["view", "inspect", "refs", "find", "grep", "files", "read"];
/// May change the project: recorded on the task in a board run, and refused in
/// read-only runs, except `build`, which reviews run to verify.
const CHANGES: &[&str] = &["submit", "delete", "write", "remove", "run", "format", "build"];
/// Run a process that may change source; what it changed is read in after.
const PROCESSES: &[&str] = &["format", "build", "run"];
/// Results recorded as the task's verification.
const VERIFIES: &[&str] = &["check", "test", "build"];
/// Only for the human, never in a board run.
const HOST: &[&str] = &["agent-config", "serve", "create", "migrate"];
/// Need Gleam code: refused in a project of plain files.
const GLEAM: &[&str] = &["submit", "drafts", "view", "inspect", "refs", "impact", "find", "delete", "check", "test", "probe"];

/// A project folder as `--project` takes it.
fn project_dir(dir: &str) -> &str {
    match dir.trim_end_matches('/') {
        "" => ".",
        d => d.strip_prefix("./").unwrap_or(d),
    }
}

/// `--project <dir>` on a command that isn't a plain read, in a board run whose
/// task may change that project (D-61): the folder the command runs in. Plain
/// reads, and commands outside a run, go to `read_project`.
fn scoped(dir: &str, args: &[String]) -> Result<Option<PathBuf>, Failure> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let (Some((tid, _)), Ok(own)) = (board::context(), Store::root()) else { return Ok(None) };
    if READS.contains(&command) && !args.iter().any(|x| x == "--draft") {
        return Ok(None);
    }
    let top = scope::top(&own);
    let dir = project_dir(dir);
    let projects = scope::of(&board::task(&own, &tid)?, &scope::dir(&top, &own));
    if !projects.iter().any(|p| p == dir) {
        return lockdown::error(
            "E6007",
            dir,
            format!(
                "not one of this task's projects ({}); if the task needs it, ask the human to add it with `task ask`",
                projects.join(", ")
            ),
        );
    }
    if command == "task" {
        return lockdown::error("E6007", command, "task commands belong to the task's own project; leave out --project");
    }
    Ok(Some(scope::path(&top, dir)))
}

/// Runs a command in another folder, and comes back after.
// ponytail: the working folder is process-wide; fine while a process serves one call at a time (CLI, MCP).
struct Within(PathBuf);
impl Within {
    fn enter(dir: &Path) -> Result<Within, Failure> {
        let back = env::current_dir()?;
        env::set_current_dir(dir)?;
        Ok(Within(back))
    }
}
impl Drop for Within {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.0);
    }
}

/// `--project <dir>`: a read in another project under the same Solace root
/// (D-57). It takes no lock in that project, so reading never waits on its
/// agents. Changes go through `scoped` (D-61).
fn read_project(dir: &str, args: &[String], budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = a.first().copied().unwrap_or("");
    if !READS.contains(&command) || a.contains(&"--draft") {
        return lockdown::error(
            "E6007",
            command,
            "`--project` only reads (view, inspect, refs, find, grep, files, read) here; in a board run, it also changes the task's other projects",
        );
    }
    // From a project, or from a folder of projects with none at its root.
    let here = env::current_dir()?;
    let own = Store::root().ok();
    let root = match workspace::enclosing(own.as_deref().unwrap_or(&here))? {
        Some(root) => root,
        None => own.clone().ok_or_else(|| Failure("No Solace root here: migrate an existing project (`solace serve`, then Migrate this project) or make a new one with `solace create <name>`.".into(), 1))?,
    };
    let members = workspace::members(&root)?;
    let dir = project_dir(dir);
    if !members.iter().any(|m| m == dir) {
        return lockdown::error("E6007", dir, format!("not a project under this Solace root; its projects: {}", members.join(", ")));
    }
    let target = scope::path(&root, dir);
    // A board run's agent must still be running, as for any command.
    let _guard = match (board::context(), &own) {
        (Some((tid, rid)), Some(own)) => Some(board::agent_guard(own, &tid, &rid, &a, None)?),
        _ => None,
    };
    // A plain connection: a read creates no tables in another project.
    let store = Store { db: Connection::open(target.join(".solace/store.db"))?, root: target };
    let settings = lockdown::settings(&store.root)?;
    if let Some(out) = lockdown::dispatch(&store, &settings, &a, &mut || Ok(String::new()), None, budget) {
        return out;
    }
    if !store.root.join("gleam.toml").exists() {
        return lockdown::plain(command);
    }
    match a.as_slice() {
        ["view", rest @ ..] if !operands(rest).is_empty() => store.view(&operands(rest), rest.contains(&"--bodies"), budget),
        ["inspect", rest @ ..] if !operands(rest).is_empty() => store.inspect(&operands(rest), rest.contains(&"--context"), budget),
        ["refs", name] => store.refs(name),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        _ => usage(format!("{} --project {dir}: give what to read", a.join(" "))),
    }
}

fn run_inner(args: &[String], input: &mut dyn FnMut() -> Result<String, Failure>, old: Option<&str>, budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(a.as_slice(), [] | ["help"] | ["--help"]) {
        return Ok(REFERENCE.to_string());
    }
    if let ["agent-config", agent, dir] = a.as_slice() {
        return lockdown::agent_config(
            agent,
            Path::new(dir),
            &env::current_exe()?,
            board::system()?["task_timeout"].as_u64().unwrap_or(3600),
        );
    }
    if a[0] == "create" {
        return workspace::create(&env::current_dir()?, &a[1..]);
    }

    // run_options has put back any interrupted file write (lockdown::recover).
    let store = Store::open()?;
    let settings = lockdown::settings(&store.root)?;
    if let Some(out) = lockdown::dispatch(&store, &settings, &a, input, old, budget) {
        return out;
    }
    if GLEAM.contains(&a[0]) && !store.root.join("gleam.toml").exists() {
        return lockdown::plain(a[0]);
    }
    match a.as_slice() {
        ["session", "start", "--task"] => store.session_task(&input()?),
        ["session", "start", "--task", task @ ..] => store.session_task(&task.join(" ")),
        ["submit", ..] => drafts::submit(&store, &a, input, old),
        ["drafts"] => drafts::list(&store),
        ["view", "--draft", id] => drafts::view(&store, id),
        ["session", rest @ ..] => store.session(&operands(rest.strip_prefix(&["start"]).unwrap_or(rest))),
        ["view", rest @ ..] if !operands(rest).is_empty() => store.view(&operands(rest), rest.contains(&"--bodies"), budget),
        ["inspect", rest @ ..] if !operands(rest).is_empty() => store.inspect(&operands(rest), rest.contains(&"--context"), budget),
        ["refs", name] => store.refs(name),
        ["impact", names @ ..] => store.impact(names, budget),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        ["delete", name] => store.delete(name),
        ["check"] => command_report(store.check(), budget),
        ["check", "--warnings"] => command_report(store.check_report(true), budget),
        ["test", rest @ ..] => command_report(store.test(rest), budget),
        ["probe", rest @ ..] => {
            let (from, trace) = probe_args(rest)?;
            // with --from the test's body is the probe; `text` only adds to it
            let text = if from.is_some() { input().unwrap_or_default() } else { input()? };
            command_report(store.probe(&text, from, &trace), budget)
        }
        ["work", "add"] => store.work_add(&input()?),
        ["work", "add", text @ ..] => store.work_add(&text.join(" ")),
        ["work", "list"] | ["work"] => store.work_list(),
        ["work", "done", id] => store.work_done(id),
        _ => usage(format!(
            "Unknown command: {}\nCommands: session, inspect, view, refs, impact, find, submit, drafts, delete, check, test, probe, work, files, read, write, remove, build, format, run, task.",
            a.join(" ")
        )),
    }
}

/// Command output without build progress and compiler warnings (a warning
/// runs until the next error or until the program starts).
fn quiet(output: &str) -> String {
    let (mut out, mut warning) = (String::new(), false);
    for l in output.lines() {
        let t = l.trim();
        warning = (warning || l.starts_with("warning")) && !l.starts_with("error") && !t.starts_with("Running ");
        if !warning && !["Compiling ", "Compiled ", "Running ", "Downloading ", "Downloaded "].iter().any(|p| t.starts_with(p)) {
            out += &format!("{l}\n");
        }
    }
    out
}

/// The module name a probe runs as.
const PROBE: &str = "solace_probe";

/// `probe [--from <fn>] [--trace <fn>...]`
fn probe_args<'a>(args: &[&'a str]) -> Result<(Option<&'a str>, Vec<&'a str>), Failure> {
    let (mut from, mut trace, mut tracing) = (None, vec![], false);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--from" => from = Some(*it.next().ok_or_else(|| Failure("--from needs a function name.".into(), 2))?),
            "--trace" => tracing = true,
            _ if tracing && !a.starts_with("--") => trace.push(a),
            _ => return usage("probe [--from <fn>] [--trace <fn>...]; the code is the tool's `text`"),
        }
    }
    if tracing && trace.is_empty() {
        return usage("--trace needs the functions to trace, e.g. `probe --trace pricing.quote`");
    }
    Ok((from, trace))
}

fn command_report(result: Out, budget: usize) -> Out {
    match result {
        Ok(text) => Ok(lockdown::trim_output(&("exit code: 0\n".to_string() + &text), budget)),
        Err(Failure(text, code)) => Err(Failure(lockdown::trim_output(&format!("exit code: {code}\n{text}"), budget), code)),
    }
}

/// Arguments that aren't flags (`--scope` just introduces them).
fn operands<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| !a.starts_with("--")).collect()
}

/// A `command` line that is Gleam code, not a Solace command.
fn looks_like_code(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    matches!(first, "let" | "echo" | "assert" | "use" | "case" | "pub" | "fn" | "import" | "}" | "//") || first.contains('(')
}

fn stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

/// `solace mcp`: the same commands as one MCP tool over stdio, so an agent
/// can pass code without shell quoting and needs no shell at all.
fn mcp(budget: usize) {
    let description = "Read, change and test this project's code. Start with `session start --task`, \
        the task in `text`: it lists the symbols most likely relevant, the workflow and the commands. Several commands \
        go in one call, one per line, and run in order: read everything you need in one call, and put \
        `test` on the line after a `submit`.";
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = msg.get("id").cloned() else {
            continue;
        }; // notifications need no reply
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "solace", "version": env!("CARGO_PKG_VERSION")}
            }),
            "tools/list" => json!({"tools": [{
                "name": "solace",
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "One or more commands, one per line, e.g. `inspect checkout.pay tax.quote --context`, or `submit` then `test`"},
                        "text": {"type": "string", "description": "For submit: a patch (`*** Begin Patch` / `*** Update: <symbol or module>` / `@@` hunks) or new code; the task for `session start --task`; the record, note, summary or question for `work add` and `task note|done|ask`"},
                        "options": {"type": "array", "items": {"type": "string"}, "description": "2–4 options, for `task ask`"},
                        "old": {"type": "string", "description": "For `--replace`: the exact text that `text` replaces"}
                    },
                    "required": ["command"]
                }
            }]}),
            "tools/call" => {
                let args = &msg["params"]["arguments"];
                let text = args["text"].as_str().map(String::from);
                let mut input = || match &text {
                    Some(t) => Ok(t.clone()),
                    _ => usage("This command needs the new code in the `text` argument."),
                };
                // Several commands, one per line, run in order in one call.
                let commands: Vec<&str> =
                    args["command"].as_str().unwrap_or("").lines().map(|c| c.trim()).filter(|c| !c.is_empty()).collect();
                let (mut out, mut is_error) = (String::new(), false);
                SHOWN.with(|s| *s.borrow_mut() = Some(HashSet::new()));
                // Code sent as command lines rather than in `text` (run 11): one
                // error for the call, and nothing runs.
                if let Some(code) = commands.iter().find(|c| looks_like_code(c)) {
                    out = format!(
                        "Not a command: {code}\nThis looks like code. Code (for probe, submit) goes in `text`, \
                        not in `command`; nothing in this call ran.\n"
                    );
                    is_error = true;
                }
                let refused = is_error;
                for command in commands.iter().filter(|_| !refused) {
                    let command = command.strip_prefix("solace").map_or(*command, |c| c.trim_start());
                    let words: Vec<String> = command.split_whitespace().map(String::from).collect();
                    if commands.len() > 1 {
                        out += &format!("==== solace {command} ====\n");
                    }
                    if words.first().is_some_and(|w| w == "help") {
                        out += &agent_help(words.get(1).map(|w| w.as_str()));
                        continue;
                    }
                    // commands in one call share the budget, since the agent cuts the reply as a whole
                    match run_options(
                        &words,
                        &mut input,
                        args["old"].as_str(),
                        budget.saturating_sub(out.len()),
                        &args["options"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>())
                            .unwrap_or_default(),
                    ) {
                        Ok(o) => out += &o,
                        Err(Failure(msg, _)) => {
                            out += &msg;
                            is_error = true;
                        }
                    }
                    if commands.len() > 1 && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                if commands.is_empty() {
                    (out, is_error) = (format!("No command given.\n\n{}", agent_reference()), true);
                }
                json!({"content": [{"type": "text", "text": out}], "isError": is_error})
            }
            "ping" => json!({}),
            _ => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}});
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = stdout.flush();
    }
}

// --- the store --------------------------------------------------------------

#[derive(Clone)]
struct Sym {
    id: String,
    module: String,
    name: String,
    kind: Kind,
    text: String,
}

impl Sym {
    fn qname(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
    fn item(&self) -> Item {
        Item { kind: self.kind, name: self.name.clone(), text: self.text.clone() }
    }
}

struct ModuleRow {
    path: String,
    dir: String,
    header: String,
}

struct Store {
    db: Connection,
    root: PathBuf,
}

/// All live symbols and modules, with the reference graph resolved.
struct Index {
    syms: Vec<Sym>,
    modules: Vec<ModuleRow>,
    uses: Vec<Vec<usize>>,
    /// The reverse of `uses`: the symbols that use each one, in order.
    used_by: Vec<Vec<usize>>,
    /// Every resolved reference: (symbol, line in the user's text).
    at: Vec<Vec<(usize, usize)>>,
}

impl Index {
    fn build(syms: Vec<Sym>, modules: Vec<ModuleRow>) -> Index {
        // (module, name) -> symbol, for names and for type constructors
        let mut names: HashMap<(&str, &str), usize> = HashMap::new();
        for (i, s) in syms.iter().enumerate() {
            names.insert((&s.module, &s.name), i);
        }
        let mut ctors: Vec<(String, String, usize)> = vec![];
        for (i, s) in syms.iter().enumerate() {
            if s.kind == Kind::Type {
                for c in gleam::constructors(&s.text) {
                    ctors.push((s.module.clone(), c, i));
                }
            }
        }
        for (m, c, i) in &ctors {
            names.entry((m, c)).or_insert(*i);
        }
        let imports: HashMap<&str, Vec<gleam::Import>> = modules.iter().map(|m| (m.path.as_str(), gleam::imports(&m.header))).collect();
        let at: Vec<Vec<(usize, usize)>> = syms
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let imps = &imports[s.module.as_str()];
                let mut found = vec![];
                for (r, line) in gleam::references_at(&s.text) {
                    let target = match &r {
                        Ref::Qualified(alias, member) => {
                            imps.iter().find(|imp| &imp.alias == alias).and_then(|imp| names.get(&(imp.module.as_str(), member.as_str())))
                        }
                        Ref::Bare(name) => names.get(&(s.module.as_str(), name.as_str())).or_else(|| {
                            imps.iter().find_map(|imp| {
                                imp.unqualified
                                    .iter()
                                    .find(|(_, local)| local == name)
                                    .and_then(|(orig, _)| names.get(&(imp.module.as_str(), orig.as_str())))
                            })
                        }),
                    };
                    if let Some(&t) = target
                        && t != i
                    {
                        found.push((t, line));
                    }
                }
                found
            })
            .collect();
        let uses: Vec<Vec<usize>> = at
            .iter()
            .map(|found| {
                let mut uses: Vec<usize> = vec![];
                for &(t, _) in found {
                    if !uses.contains(&t) {
                        uses.push(t);
                    }
                }
                uses
            })
            .collect();
        let mut used_by = vec![vec![]; syms.len()];
        for (j, us) in uses.iter().enumerate() {
            for &u in us {
                used_by[u].push(j);
            }
        }
        Index { syms, modules, uses, used_by, at }
    }
}

enum Target<'a> {
    Imports(&'a str),
    /// A whole module's text, as `view <module>` shows it.
    Module(&'a str),
}

const SCHEMA: &str = "
CREATE TABLE modules(path TEXT PRIMARY KEY, dir TEXT NOT NULL, header TEXT NOT NULL);
CREATE TABLE symbols(id TEXT PRIMARY KEY, module TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  text TEXT NOT NULL, ord INTEGER NOT NULL, revision TEXT NOT NULL, live INTEGER NOT NULL DEFAULT 1);
CREATE TABLE history(seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, symbol TEXT NOT NULL, action TEXT NOT NULL, detail TEXT NOT NULL);
CREATE TABLE broken(symbol TEXT PRIMARY KEY, diagnostic TEXT NOT NULL);
CREATE TABLE work(id INTEGER PRIMARY KEY, text TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1);
";

/// Tables added after `SCHEMA`, made when a store opens so older stores have them too.
const LATER_TABLES: &str = "
-- per-test results and the source as of the last full pass (D-44)
CREATE TABLE IF NOT EXISTS test_runs(test TEXT PRIMARY KEY, reach TEXT NOT NULL, result TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS test_base(id TEXT PRIMARY KEY, text TEXT NOT NULL);
-- rejected submissions, kept to retry (D-46)
CREATE TABLE IF NOT EXISTS drafts(id TEXT PRIMARY KEY, owner TEXT NOT NULL, args TEXT NOT NULL, text TEXT NOT NULL, base TEXT NOT NULL);
-- the task board
CREATE TABLE IF NOT EXISTS board_events(seq INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS board_before(task TEXT, entity TEXT, data TEXT NOT NULL, PRIMARY KEY(task,entity));
CREATE TABLE IF NOT EXISTS board_touches(seq INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, entity TEXT);
";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of `bytes`, in hex.
fn sha256(bytes: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(bytes))
}

/// A fresh id: 16 hex digits, unique across processes and calls.
fn unique_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    sha256(format!("{nanos}/{n}/{}", std::process::id()).as_bytes())[..16].to_string()
}

/// A new symbol identity: never derived from content, never reused (Spec 1 §2).
fn new_id(kind: Kind) -> String {
    let prefix = match kind {
        Kind::Fn => "fn",
        Kind::Type => "ty",
        Kind::Const => "cn",
    };
    format!("{prefix}:{}", unique_id())
}

/// `text` without its comment and doc lines.
fn without_comment_lines(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// Phase 1a revision: a hash of the code without its comments or doc text.
fn revision(text: &str) -> String {
    format!("sha256:{}", &sha256(without_comment_lines(text).as_bytes())[..16])
}

const STARTER_SETTINGS: &str = "# Solace project settings. Protected: agents may read this file.
# Text files up to this many bytes are watched; file writes are limited to it.
watch_limit = 262144
# Files Solace doesn't watch or search (agents can still read and write them).
unwatched = []

[validators]
# Optional checks, for example:
# \"docs/**/*.json\" = [\"python3\", \"-m\", \"json.tool\", \"{file}\"]

[commands]
# Optional commands agents may run, for example:
# app = { run = [\"gleam\", \"run\", \"--\"], args = true }
";

/// Sets up a project: `solace.toml` if missing, and a store holding the Gleam
/// code under `src/` and `test/`. Without `gleam.toml` the store stays empty:
/// a project of plain files (D-60).
fn import(root: &Path) -> Out {
    let mut out = String::new();
    match fs::OpenOptions::new().write(true).create_new(true).open(root.join("solace.toml")) {
        Ok(mut file) => {
            file.write_all(STARTER_SETTINGS.as_bytes())?;
            out += "Created solace.toml with default settings.\n";
        }
        // A root's list of projects alone gains the settings its own project needs (D-60).
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let text = fs::read_to_string(root.join("solace.toml"))?;
            if text.parse::<toml::Value>().is_ok_and(|v| v.as_table().is_some_and(|t| t.keys().all(|k| k == "projects"))) {
                fs::write(root.join("solace.toml"), format!("{}\n{STARTER_SETTINGS}", text.trim_end()))?;
                out += "Added default settings to solace.toml.\n";
            }
        }
        Err(e) => return Err(e.into()),
    }
    let dir = root.join(".solace");
    if dir.join("store.db").exists() {
        out += "This project already has a store (.solace/store.db); existing symbols were preserved.\n";
        return Ok(out);
    }
    fs::create_dir_all(&dir)?;
    let db = Connection::open(dir.join("store.db"))?;
    db.execute_batch(SCHEMA)?;
    db.execute_batch(LATER_TABLES)?;
    let (mut modules, mut symbols) = (0, 0);
    let mut written = outside::Written::load(root);
    written.known = true;
    for top in ["src", "test"].into_iter().filter(|_| root.join("gleam.toml").exists()) {
        let mut files = vec![];
        collect_gleam_files(&root.join(top), &mut files)?;
        files.sort();
        for file in files {
            let path = file.strip_prefix(root.join(top)).unwrap().with_extension("");
            let path = path.to_string_lossy().replace('\\', "/");
            let text = fs::read_to_string(&file).map_err(|e| Failure(format!("{}: {e}", file.display()), 1))?;
            written.record(&outside::file_of(top, &path), text.as_bytes());
            let parsed = gleam::split(&text);
            db.execute("INSERT INTO modules VALUES (?1, ?2, ?3)", params![path, top, parsed.header])?;
            for (ord, item) in parsed.items.iter().enumerate() {
                insert_symbol(&db, &path, item, Some(ord as i64))?;
                symbols += 1;
            }
            modules += 1;
        }
    }
    written.save()?;
    let store = Store { db, root: root.to_path_buf() };
    outside::approve_all(&store)?;
    out += &format!("Imported {modules} modules, {symbols} symbols into .solace/store.db\n");
    Ok(out)
}

/// Stores a new symbol at `ord`, or after its module's last; returns its id.
fn insert_symbol(db: &Connection, module: &str, item: &gleam::Item, ord: Option<i64>) -> Result<String, Failure> {
    let ord = match ord {
        Some(o) => o,
        None => db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [module], |r| r.get(0))?,
    };
    let id = new_id(item.kind);
    db.execute(
        "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, module, item.name, item.kind.as_str(), item.text, ord, revision(&item.text)],
    )?;
    Ok(id)
}

/// A Gleam module path: lowercase names of letters, digits and `_`, joined by `/`.
fn valid_module_path(m: &str) -> bool {
    m.split('/').all(|p| {
        p.starts_with(|c: char| c.is_ascii_lowercase()) && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

/// The Gleam files in a project's `src/` and `test/`, as sorted paths from `dir`.
fn source_files(dir: &Path) -> Result<Vec<String>, Failure> {
    let mut files = vec![];
    for top in ["src", "test"] {
        collect_gleam_files(&dir.join(top), &mut files)?;
    }
    let mut out: Vec<String> = files.iter().map(|f| f.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    out.sort();
    Ok(out)
}

fn collect_gleam_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // A symlinked folder isn't followed: deleting or writing through it could reach outside the project.
        if entry.file_type()?.is_dir() {
            collect_gleam_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "gleam") {
            out.push(path);
        }
    }
    Ok(())
}

/// A module's file as Solace writes it, with the line range of each symbol.
fn projection(ix: &Index, m: &ModuleRow) -> (String, Vec<(usize, usize, String)>) {
    let mut content = String::new();
    let mut line = 1;
    let mut ranges = vec![];
    if !m.header.is_empty() {
        content.push_str(&m.header);
        content.push_str("\n\n");
        line += m.header.lines().count() + 1;
    }
    for s in ix.syms.iter().filter(|s| s.module == m.path) {
        let n = s.text.lines().count();
        ranges.push((line, line + n - 1, s.qname()));
        content.push_str(&s.text);
        content.push_str("\n\n");
        line += n + 1;
    }
    (content.trim_end().to_string() + "\n", ranges)
}

impl Store {
    /// The store of the project at `root`, with every table this version uses.
    fn at(root: &Path) -> Result<Store, Failure> {
        let db = Connection::open(root.join(".solace/store.db"))?;
        db.execute_batch(LATER_TABLES)?;
        Ok(Store { db, root: root.into() })
    }

    fn open() -> Result<Store, Failure> {
        Store::at(&Store::root()?)
    }

    /// The project the working folder is in: the nearest folder up with a store.
    fn root() -> Result<PathBuf, Failure> {
        let mut dir = env::current_dir()?;
        loop {
            if dir.join(".solace/store.db").exists() {
                return Ok(dir);
            }
            if !dir.pop() {
                return fail(
                    "No Solace store found (.solace/store.db). Bring existing code in by migrating it: run `solace serve` here and choose Migrate this project.",
                );
            }
        }
    }

    fn index(&self) -> Result<Index, Failure> {
        let mut stmt = self.db.prepare("SELECT id, module, name, kind, text FROM symbols WHERE live = 1 ORDER BY module, ord")?;
        let syms: Vec<Sym> = stmt
            .query_map([], |r| {
                Ok(Sym { id: r.get(0)?, module: r.get(1)?, name: r.get(2)?, kind: Kind::parse(&r.get::<_, String>(3)?), text: r.get(4)? })
            })?
            .collect::<Result<_, _>>()?;
        let mut stmt = self.db.prepare("SELECT path, dir, header FROM modules ORDER BY dir, path")?;
        let modules: Vec<ModuleRow> =
            stmt.query_map([], |r| Ok(ModuleRow { path: r.get(0)?, dir: r.get(1)?, header: r.get(2)? }))?.collect::<Result<_, _>>()?;

        Ok(Index::build(syms, modules))
    }

    // --- writing files --------------------------------------------------------

    /// Writes every module's file from the store. Returns, per file, the line
    /// range of each symbol, for mapping compiler output back to symbols.
    fn materialize(&self, ix: &Index) -> Result<Vec<FileMap>, Failure> {
        self.materialize_to(ix, &self.root)
    }

    fn materialize_to(&self, ix: &Index, target: &Path) -> Result<Vec<FileMap>, Failure> {
        // A file changed outside Solace is never written over (D-58); the next command reads it in.
        let mut written = (target == self.root).then(|| outside::Written::load(&self.root));
        let mut maps = vec![];
        for m in &ix.modules {
            let (content, ranges) = projection(ix, m);
            let rel = outside::file_of(&m.dir, &m.path);
            let path = target.join(&rel);
            if fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                if written.as_ref().is_some_and(|w| !w.unchanged(&rel)) {
                    maps.push(FileMap { module: m.path.clone(), file: path, ranges });
                    continue;
                }
                fs::create_dir_all(path.parent().unwrap())?;
                fs::write(&path, &content)?;
            }
            if let Some(w) = written.as_mut() {
                w.record(&rel, content.as_bytes());
            }
            maps.push(FileMap { module: m.path.clone(), file: path, ranges });
        }
        if let Some(w) = written.as_mut() {
            w.save()?;
        }
        Ok(maps)
    }

    /// Solace's own short-lived rewrite of a module file (tests made private,
    /// traced functions), put back by `materialize` afterwards.
    fn rewrite(&self, path: &Path, content: &str) -> Result<(), Failure> {
        fs::write(path, content)?;
        let mut w = outside::Written::load(&self.root);
        if w.known {
            w.record(&path.strip_prefix(&self.root).unwrap().to_string_lossy(), content.as_bytes());
            w.save()?;
        }
        Ok(())
    }

    fn gleam(&self, args: &[&str]) -> Result<(bool, String), Failure> {
        let mut argv = vec!["gleam".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let text = lockdown::command(&self.root, &argv, 120, BUDGET)?;
        Ok((text.starts_with("exit code: 0\n"), strip_ansi(text.split_once('\n').map_or("", |(_, s)| s))))
    }

    /// Type-checks the project and records which symbols are broken, with
    /// their errors, so the next reply can report only what changed (D-46).
    fn check_all(&self, ix: &Index) -> Result<Vec<Diag>, Failure> {
        let files = self.materialize(ix)?;
        let (_, output) = self.gleam(&["check"])?;
        let diags = diagnostics(&output, &files);
        self.db.execute("DELETE FROM broken", [])?;
        for d in diags.iter().filter(|d| d.error) {
            if let Some(sym) = &d.symbol {
                self.db.execute(
                    "INSERT INTO broken VALUES (?1, ?2) ON CONFLICT(symbol) DO UPDATE SET diagnostic = diagnostic || char(10, 10) || excluded.diagnostic",
                    params![sym, d.text],
                )?;
            }
        }
        Ok(diags)
    }

    /// The broken symbols as last checked, each with its errors.
    fn broken(&self) -> Result<HashMap<String, String>, Failure> {
        Ok(self
            .db
            .prepare("SELECT symbol, diagnostic FROM broken")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }

    // --- reading ---------------------------------------------------------------

    fn resolve(&self, ix: &Index, input: &str) -> Result<usize, Failure> {
        let (module, name) = match input.rsplit_once('.') {
            Some((m, n)) => (Some(m.replace('.', "/")), n),
            None => (None, input),
        };
        let found: Vec<usize> = (0..ix.syms.len())
            .filter(|&i| {
                let s = &ix.syms[i];
                s.name == name && module.as_ref().is_none_or(|m| s.module == *m || s.module.ends_with(&format!("/{m}")))
            })
            .collect();
        match found.as_slice() {
            [i] => Ok(*i),
            [] => {
                let lower = name.to_lowercase();
                let similar: Vec<String> =
                    ix.syms.iter().filter(|s| s.name.to_lowercase().contains(&lower)).take(10).map(|s| s.qname()).collect();
                let hint = if similar.is_empty() { String::new() } else { format!("\nSimilar names: {}", similar.join(", ")) };
                fail(format!("E1001 unknown name: {input}{hint}"))
            }
            many => {
                let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind.as_str())).collect();
                fail(format!("E1002 ambiguous name: {input} could be\n{}", list.join("\n")))
            }
        }
    }

    /// A dependency's module (`lustre/component`) or public symbol
    /// (`lustre/component.register`), read from `build/packages/` and never
    /// stored (D-15): a module gives its public signatures, a symbol its doc
    /// and declaration. Bodies stay out; agents use the API, not the source.
    fn dependency(&self, input: &str) -> Option<String> {
        let source = |module: &str| -> Option<(String, String)> {
            let module = module.replace('.', "/");
            let packages = fs::read_dir(self.root.join("build/packages")).ok()?;
            packages.flatten().find_map(|p| {
                let text = fs::read_to_string(p.path().join("src").join(format!("{module}.gleam"))).ok()?;
                Some((format!("{} ({}, a dependency; read only)", module, p.file_name().to_string_lossy()), text))
            })
        };
        let declaration = |item: &gleam::Item| {
            let sig = gleam::signature(&gleam::Item { text: without_comment_lines(&item.text), ..item.clone() }).trim_end().to_string();
            // An opaque type's constructors aren't part of its API.
            if sig.contains("pub opaque type ") { sig.split(" {").next().unwrap_or(&sig).to_string() } else { sig }
        };
        let public = |item: &gleam::Item| item.text.lines().any(|l| l.starts_with("pub "));
        if let Some((title, text)) = source(input) {
            let m = gleam::split(&text);
            let mut out = format!("// ── module {title}, public API: `view {input}.<name>` for one with its docs ──\n");
            // The module doc's first paragraph; the rest is guides and links.
            let doc: Vec<&str> = m.header.lines().filter(|l| l.starts_with("////")).take_while(|l| l.trim() != "////").collect();
            if !doc.is_empty() {
                out += &format!("{}\n\n", doc.join("\n"));
            }
            for item in m.items.iter().filter(|i| public(i)) {
                let doc = item.text.lines().find(|l| l.starts_with("///")).map(|l| format!("{l}\n")).unwrap_or_default();
                out += &format!("{doc}{}\n\n", declaration(item));
            }
            return Some(out);
        }
        let (module, name) = input.rsplit_once('.')?;
        let (title, text) = source(module)?;
        let item = gleam::split(&text).items.into_iter().find(|i| i.name == name && public(i))?;
        let doc: String = item.text.lines().filter(|l| l.starts_with("///")).map(|l| format!("{l}\n")).collect();
        Some(format!("// ── {name} in {title} ({}) ──\n{doc}{}\n\n", item.kind.as_str(), declaration(&item)))
    }

    fn resolve_module<'a>(&self, ix: &'a Index, input: &str) -> Option<&'a ModuleRow> {
        let input = input.replace('.', "/");
        let exact = ix.modules.iter().find(|m| m.path == input);
        let suffix: Vec<&ModuleRow> = ix.modules.iter().filter(|m| m.path.ends_with(&format!("/{input}"))).collect();
        exact.or(if suffix.len() == 1 { Some(suffix[0]) } else { None })
    }

    /// The symbol index (one line per source symbol), test modules, and open
    /// work records expanded with the context of the symbols they name.
    fn session(&self, scopes: &[&str]) -> Out {
        const CAP: usize = 80;
        if !self.root.join("gleam.toml").exists() {
            return self.plain_session();
        }
        let ix = self.index()?;
        let in_scope =
            |m: &ModuleRow| scopes.is_empty() || scopes.iter().any(|s| self.resolve_module(&ix, s).is_some_and(|r| r.path == m.path));
        let mut out = self.project_instructions();
        out += &String::from(
            "Symbols per module (types are capitalized). `inspect <name>...` gives signatures and context; \
             `view <module>` an outline.\n",
        );
        let mut tests = vec![];
        for m in ix.modules.iter().filter(|m| in_scope(m)) {
            let syms: Vec<&Sym> = ix.syms.iter().filter(|s| s.module == m.path).collect();
            if m.dir == "test" {
                tests.push(format!("{} ({})", m.path, syms.len()));
                continue;
            }
            // Names only: this is re-sent on every turn, so it must stay small (D-12).
            let names: Vec<&str> = syms.iter().take(CAP).map(|s| s.name.as_str()).collect();
            let more = if syms.len() > CAP { format!(", …{} more", syms.len() - CAP) } else { String::new() };
            out += &format!("{} — {}\n  {}{more}\n", m.path, module_doc(m), names.join(", "));
        }
        out += &format!("\nTest modules (tests): {}\n", tests.join(", "));
        out += &self.session_end(&ix)?;
        Ok(out)
    }

    /// Session start in a project of plain files (D-60): no symbols to list.
    fn plain_session(&self) -> Out {
        let mut out = self.project_instructions();
        out += "This project has plain files and no Gleam code of its own: `files` lists them, `read` and `write` change them, \
                and `run` runs the commands its solace.toml declares.\n";
        out += &self.session_end(&self.index()?)?;
        Ok(out)
    }

    /// The end of every session start: status, open work and its context,
    /// the other projects, and the command reference.
    fn session_end(&self, ix: &Index) -> Out {
        Ok(format!("{}{}{}\n{}", self.status()?, self.work_context(ix)?, self.other_projects(), agent_reference()))
    }

    /// The symbols `text` names as `module.name`, in order, each once.
    fn named(&self, ix: &Index, text: &str) -> Vec<usize> {
        let mut named = vec![];
        for word in text.split(|c: char| !(c.is_alphanumeric() || "_./".contains(c))) {
            let word = word.trim_matches(|c| c == '.' || c == '/');
            if word.contains('.')
                && let Ok(i) = self.resolve(ix, word)
                && !named.contains(&i)
            {
                named.push(i);
            }
        }
        named
    }

    /// The other projects under this project's Solace root, which reads can
    /// reach with `--project` (D-57).
    fn other_projects(&self) -> String {
        let Ok(Some(root)) = workspace::enclosing(&self.root) else { return String::new() };
        let Ok(members) = workspace::members(&root) else { return String::new() };
        let me = self.root.strip_prefix(&root).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let me = if me.is_empty() { ".".to_string() } else { me };
        let others: Vec<String> = members
            .into_iter()
            .filter(|m| *m != me)
            .map(|m| match workspace::name(&scope::path(&root, &m)) {
                name if name.is_empty() => m,
                name => format!("{m} ({name})"),
            })
            .collect();
        if others.is_empty() {
            return String::new();
        }
        format!(
            "\nOther projects under this Solace root (add `--project <folder>` to view, inspect, refs, find, files or read): {}\n\
             This task can change only the projects it covers. If it needs a change in another project (or the root, `.`), \
             ask the human with `task ask` to add that project; never put the change somewhere else instead.\n",
            others.join(", ")
        )
    }

    /// The project's AGENTS.md, which its humans write for agents (D-45), cut
    /// at a whole line past 8,000 bytes; empty when there is none.
    fn project_instructions(&self) -> String {
        const CAP: usize = 8000;
        let text = fs::read_to_string(self.root.join("AGENTS.md")).unwrap_or_default();
        if text.trim().is_empty() {
            return String::new();
        }
        let mut shown = String::new();
        let mut lines = text.lines().enumerate();
        for (n, line) in lines.by_ref() {
            if shown.len() + line.len() + 1 > CAP {
                shown += &format!("… `read AGENTS.md --lines {}..` for the rest\n", n + 1);
                break;
            }
            shown += line;
            shown += "\n";
        }
        format!(
            "Project instructions (AGENTS.md, from the project's maintainers; where they name files or shell commands, \
             use the matching Solace command):\n{shown}\n"
        )
    }

    /// Open work records expanded with the context of the symbols they name.
    fn work_context(&self, ix: &Index) -> Out {
        let mut out = String::new();
        for (_, text) in self.open_work()? {
            let named = self.named(ix, &text);
            if !named.is_empty() {
                out += "\n== Context of the symbols the work record names ==\n";
                let (full, mut shown) = (named.iter().copied().collect(), HashSet::new());
                for &i in &named {
                    out += &self.bundle(ix, i, true, 0, &full, &mut shown)?;
                    out += "\n";
                }
            }
        }
        Ok(out)
    }

    /// `session start --task`: instead of the full index, the symbols most
    /// likely relevant to the task (signature and why each was picked),
    /// within a fixed budget, plus the module list.
    fn session_task(&self, task: &str) -> Out {
        const BUDGET: usize = 6500; // bytes, about 1.6k tokens
        if !self.root.join("gleam.toml").exists() {
            return self.plain_session();
        }
        let ix = self.index()?;
        let n = ix.syms.len();
        let mut modules = String::from("Modules (`view <module>` for an outline):\n");
        let mut tests = vec![];
        for m in &ix.modules {
            if m.dir == "test" {
                tests.push(m.path.as_str());
            } else {
                modules += &format!("  {} — {}\n", m.path, module_doc(m));
            }
        }
        modules += &format!("Test modules: {}\n", tests.join(", "));

        // Direct matches: rare task words weigh more; a word in the name
        // counts 3, in the doc line 2, anywhere else in the code 1.
        let fields: Vec<[HashSet<String>; 3]> = ix
            .syms
            .iter()
            .map(|s| {
                let doc: String = s.text.lines().filter(|l| l.trim_start().starts_with("///")).collect::<Vec<_>>().join(" ");
                [words(&s.name).into_iter().collect(), words(&doc).into_iter().collect(), words(&s.text).into_iter().collect()]
            })
            .collect();
        let mut terms = words(task);
        terms.sort();
        terms.dedup();
        let mut score = vec![0.0f64; n];
        let mut rare: Vec<(f64, String)> = vec![];
        for t in &terms {
            let df = fields.iter().filter(|f| f.iter().any(|w| w.contains(t))).count();
            if df == 0 {
                continue;
            }
            // squared, so one rare word (a code, a name) outweighs several common ones
            let idf = (1.0 + n as f64 / df as f64).ln().powi(2);
            rare.push((idf, t.clone()));
            for (i, f) in fields.iter().enumerate() {
                if let Some(k) = f.iter().position(|w| w.contains(t)) {
                    let w = idf * [3.0, 2.0, 1.0][k];
                    score[i] += w;
                }
            }
        }
        let direct = score.clone();

        // One hop from the best matches: callers, callees and covering tests;
        // then the non-test symbols (fixtures) the top covering tests use.
        let mut seeds: Vec<usize> = (0..n).filter(|&i| direct[i] > 0.0).collect();
        seeds.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]));
        seeds.truncate(10);
        let boost = |j: usize, s: f64, score: &mut Vec<f64>| {
            if direct[j] + s > score[j] {
                score[j] = direct[j] + s;
            }
        };
        for &s in &seeds {
            for &j in &ix.uses[s] {
                boost(j, direct[s] / 4.0, &mut score);
            }
            let mut covering = vec![];
            for &j in &ix.used_by[s] {
                if is_test(&ix, j) {
                    covering.push(j);
                }
                boost(j, direct[s] / 4.0, &mut score);
            }
            covering.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));
            for &t in covering.iter().take(2) {
                for &j in ix.uses[t].iter().filter(|&&j| !is_test(&ix, j)) {
                    boost(j, direct[s] / 8.0, &mut score);
                }
            }
        }

        // Symbols the task names (`module.name`) are listed first.
        let named = self.named(&ix, task);
        for &i in &named {
            score[i] = f64::MAX;
        }
        let mut ranked: Vec<usize> = (0..n).filter(|&i| score[i] > 0.0).collect();
        ranked.sort_by(|&a, &b| score[b].total_cmp(&score[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));

        // The task's own terms in string literals are evidence (D-44): its rarest
        // words, quoted phrases and capitalized names such as "Desk Lamp".
        rare.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut evidence_terms: Vec<String> = rare.into_iter().take(5).map(|r| r.1).collect();
        evidence_terms.extend(task.split('"').skip(1).step_by(2).filter(|p| (3..=40).contains(&p.len())).map(str::to_lowercase));
        evidence_terms.extend(capitalized_runs(task));
        // A test that reaches the top code symbols reproduces the task's case.
        let code: Vec<usize> = ranked
            .iter()
            .copied()
            .filter(|&i| !is_test(&ix, i) && ix.syms[i].kind == Kind::Fn && !ix.syms[i].text.contains("@external"))
            .take(2)
            .collect();
        let name = |i: usize| {
            let short = reach::short(&ix, i);
            if self.resolve(&ix, &short).ok() == Some(i) { short } else { ix.syms[i].qname() }
        };
        let repro = code
            .first()
            .and_then(|_| {
                reach::tests(&ix).into_iter().map(|t| (t, reach::closure(&ix, t))).filter(|(_, r)| r.contains(&code[0])).max_by(|a, b| {
                    let reached = |r: &HashSet<usize>| code.iter().filter(|c| r.contains(c)).count();
                    reached(&a.1)
                        .cmp(&reached(&b.1))
                        .then(score[a.0].total_cmp(&score[b.0]))
                        .then(ix.syms[b.0].text.len().cmp(&ix.syms[a.0].text.len()))
                })
            })
            .map(|(t, r)| {
                let traced: Vec<String> = code.iter().filter(|c| r.contains(c)).map(|&c| name(c)).collect();
                format!(
                    "To reproduce the task's case: `probe --from {} --trace {}`, with statements for the case in `text`.\n",
                    name(t),
                    traced.join(" ")
                )
            })
            .unwrap_or_default();
        let mut packet = String::new();
        let mut room = BUDGET.saturating_sub(modules.len() + repro.len());
        for &i in &ranked {
            // whole entries only: skip one that doesn't fit, try smaller ones
            let entry = format!("-- {}\n{}\n", ix.syms[i].qname(), gleam::signature(&ix.syms[i].item()));
            let mut quoted = String::new();
            for e in evidence(&ix.syms[i].text, &evidence_terms) {
                let line = format!("  > {e}\n");
                if quoted.len() + line.len() <= 300 {
                    quoted += &line;
                }
            }
            for entry in [entry.clone() + &quoted, entry] {
                if entry.len() <= room {
                    room -= entry.len();
                    packet += &entry;
                    break;
                }
            }
        }
        let mut out = self.project_instructions();
        out += &format!(
            "Symbols most likely relevant to the task, best first (signatures; `>` lines quote code that names the task's terms; `inspect` for code):\n{packet}{repro}\n{modules}"
        );
        out += &self.session_end(&ix)?;
        Ok(out)
    }

    fn open_work(&self) -> Result<Vec<(i64, String)>, Failure> {
        let mut stmt = self.db.prepare("SELECT id, text FROM work WHERE open = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Open work records and broken symbols.
    fn status(&self) -> Result<String, Failure> {
        let mut out = String::new();
        let work = self.open_work()?;
        if !work.is_empty() {
            out += "\nOpen work records:\n";
        }
        for (id, text) in work {
            out += &format!("#{id}\n{}\n", text.trim_end());
        }
        let broken: Vec<String> =
            self.db.prepare("SELECT symbol FROM broken ORDER BY symbol")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        if !broken.is_empty() {
            out += &format!("\nBroken symbols: {}\n", broken.join(", "));
        }
        Ok(out)
    }

    /// A module is shown as an outline unless `bodies` is set. Past `budget`,
    /// module bodies fall back to outlines, and what still doesn't fit is
    /// listed to view separately (D-14).
    fn view(&self, names: &[&str], bodies: bool, budget: usize) -> Out {
        let ix = self.index()?;
        let (mut out, mut left) = (String::new(), vec![]);
        let fits = |out: &str, text: &str| out.is_empty() || out.len() + text.len() <= budget;
        for &name in names {
            let text = if let Some(m) = self.resolve_module(&ix, name) {
                let mut body = format!("// ── module {} ({}) ──\n", m.path, m.dir);
                if !m.header.is_empty() {
                    body += &format!("{}\n\n", m.header);
                }
                for s in ix.syms.iter().filter(|s| s.module == m.path) {
                    body += &format!("{}\n\n", s.text);
                }
                if bodies && fits(&out, &body) {
                    body
                } else {
                    let mut text =
                        format!("// ── module {} ({}), outline: `view <name>` for a body, `--bodies` for all ──\n", m.path, m.dir);
                    if bodies {
                        text += &format!("// bodies left out to fit the size limit; `view {} --bodies` alone shows them\n", m.path);
                    }
                    let doc: Vec<&str> = m.header.lines().filter(|l| l.starts_with("////")).collect();
                    if !doc.is_empty() {
                        text += &format!("{}\n\n", doc.join("\n"));
                    }
                    let broken = self.broken()?;
                    for s in ix.syms.iter().filter(|s| s.module == m.path) {
                        text += &format!(
                            "{}{}\n\n",
                            gleam::signature(&s.item()),
                            if broken.contains_key(&s.qname()) { "  // BROKEN" } else { "" }
                        );
                    }
                    text
                }
            } else {
                match self.resolve(&ix, name) {
                    Ok(i) => format!("// ── {} ({}) ──\n{}\n\n", ix.syms[i].qname(), ix.syms[i].kind.as_str(), ix.syms[i].text),
                    Err(e) => self.dependency(name).ok_or(e)?,
                }
            };
            if left.is_empty() && fits(&out, &text) {
                out += &text;
                let printed: Vec<String> = match self.resolve_module(&ix, name) {
                    Some(m) => ix.syms.iter().filter(|s| s.module == m.path).map(Sym::qname).collect(),
                    None => self.resolve(&ix, name).map(|i| vec![ix.syms[i].qname()]).unwrap_or_default(),
                };
                printed.into_iter().for_each(mark_shown);
            } else {
                left.push(name);
            }
        }
        if !left.is_empty() {
            out +=
                &format!("Not shown, to fit the size limit: `solace view {}{}`\n", left.join(" "), if bodies { " --bodies" } else { "" });
        }
        Ok(out)
    }

    /// Several names give one document: each signature is printed once, and
    /// symbols asked for are printed in full only once (D-10, D-14). Over
    /// `budget`, the bundles are trimmed in priority order (callers, then the
    /// test example, then the signatures of what each uses); if that is not
    /// enough, the names that don't fit are listed to inspect separately.
    fn inspect(&self, names: &[&str], context: bool, budget: usize) -> Out {
        let ix = self.index()?;
        let (mut targets, mut unknown): (Vec<(&str, Option<usize>)>, Vec<String>) = (vec![], vec![]);
        for &name in names {
            match self.resolve(&ix, name) {
                Ok(i) => targets.push((name, Some(i))),
                // a module name gets its outline rather than an error
                Err(_) if self.resolve_module(&ix, name).is_some() => targets.push((name, None)),
                // so does a dependency's module or symbol, read from build/packages
                Err(_) if self.dependency(name).is_some() => targets.push((name, None)),
                // one bad name doesn't cost the others (run 11)
                Err(e) => unknown.push(e.0),
            }
        }
        if targets.is_empty() {
            return fail(unknown.join("\n"));
        }
        let unknown = if unknown.is_empty() { String::new() } else { unknown.join("\n") + "\n" };
        let full: HashSet<usize> = targets.iter().filter_map(|t| t.1).collect();
        const LAST: u8 = 3;
        for level in 0..=LAST {
            if level > 0 && !context {
                break;
            }
            // signatures an earlier command in this call printed count as shown
            let earlier: HashSet<usize> = (0..ix.syms.len()).filter(|&j| shown_earlier(&ix.syms[j].qname())).collect();
            let (mut out, mut shown, mut left) = (String::new(), earlier, vec![]);
            for &(name, i) in &targets {
                let before = shown.clone();
                let part = match i {
                    Some(i) => self.bundle(&ix, i, context, level, &full, &mut shown)?,
                    None => self.view(&[name], false, budget)?,
                } + "\n";
                let last = level == LAST || !context;
                if last && (!left.is_empty() || (!out.is_empty() && out.len() + part.len() > budget)) {
                    shown = before;
                    left.push(ix.syms.get(i.unwrap_or(usize::MAX)).map_or(name.to_string(), |s| s.qname()));
                    continue;
                }
                out += &part;
            }
            if out.len() > budget && level < LAST && context {
                continue;
            }
            let trimmed = [
                "",
                "callers as a count only (`solace refs <name>` lists them)",
                "tests by name only",
                "what each uses by name only (`view <name>` for its source)",
            ];
            if level > 0 {
                out += &format!("Trimmed to fit the size limit: {}.\n", trimmed[1..=level as usize].join("; "));
            }
            for &j in shown.iter().chain(full.iter().filter(|&&j| !left.contains(&ix.syms[j].qname()))) {
                mark_shown(ix.syms[j].qname());
            }
            if !left.is_empty() {
                out += &format!(
                    "Not shown, to fit the size limit: `solace inspect {}{}`\n",
                    left.join(" "),
                    if context { " --context" } else { "" }
                );
            }
            return Ok(out + &unknown);
        }
        unreachable!()
    }

    /// `level` is how far the context is trimmed (see `inspect`); `full` holds
    /// the symbols printed in full in this reply, `shown` the signatures
    /// printed so far.
    fn bundle(&self, ix: &Index, i: usize, context: bool, level: u8, full: &HashSet<usize>, shown: &mut HashSet<usize>) -> Out {
        let s = &ix.syms[i];
        let mut out = format!("// ── {} ({}) ──\n{}\n", s.qname(), s.kind.as_str(), s.text);
        if !context {
            return Ok(out);
        }
        let mut entry = |u: usize| {
            let q = ix.syms[u].qname();
            if full.contains(&u) {
                format!("-- {q} (in full in this reply)\n")
            } else if !shown.insert(u) {
                format!("-- {q} (above)\n")
            } else {
                format!("-- {q}\n{}\n", gleam::signature(&ix.syms[u].item()))
            }
        };
        let used_by = ix.used_by[i].clone();
        let (tests, callers): (Vec<usize>, Vec<usize>) = used_by.into_iter().partition(|&j| is_test(ix, j));
        if !ix.uses[i].is_empty() {
            let mut uses = ix.uses[i].clone();
            uses.sort_by_key(|&u| (ix.syms[u].kind != Kind::Type, ix.syms[u].kind != Kind::Const, ix.syms[u].qname()));
            if level >= 3 {
                let names: Vec<String> = uses.iter().map(|&u| ix.syms[u].qname()).collect();
                out += &format!("\n== Uses ==\n  {}\n", names.join(", "));
            } else {
                out += "\n== Uses ==\n";
                for u in uses {
                    out += &entry(u);
                }
            }
        }
        if !callers.is_empty() && level >= 1 {
            out += &format!("\n== Used by ({}): solace refs {} ==\n", callers.len(), s.qname());
        } else if s.kind == Kind::Type {
            // the sites a new field breaks, then the other users by name (D-46)
            out += &type_users(ix, i, &callers, &tests, 20);
        } else if !callers.is_empty() {
            out += &format!("\n== Used by ({}) ==\n", callers.len());
            for &c in callers.iter().take(20) {
                out += &entry(c);
                out += &call_lines(ix, c, i);
            }
            if callers.len() > 20 {
                out += &format!("…and {} more (solace refs {})\n", callers.len() - 20, s.qname());
            }
        }
        if !tests.is_empty() {
            // The smallest test in full, as a worked example (saves a turn of
            // browsing test modules); the rest by name.
            const EXAMPLE_CAP: usize = 1600; // bytes, about 400 tokens
            let example = tests
                .iter()
                .copied()
                .filter(|t| level < 2 && !full.contains(t))
                .min_by_key(|&t| ix.syms[t].text.len())
                .filter(|&t| ix.syms[t].text.len() <= EXAMPLE_CAP);
            out += &format!("\n== Tests that use it ({}) ==\n", tests.len());
            if let Some(t) = example {
                out += &format!("-- {} (example)\n{}\n", ix.syms[t].qname(), ix.syms[t].text);
                // the test helpers it uses, so a new test can call them without guessing
                for &h in ix.uses[t].iter().filter(|&&h| is_test(ix, h) && !ix.syms[h].name.ends_with("_test")) {
                    out += &entry(h);
                }
            }
            let names: Vec<String> = tests.iter().filter(|&&t| Some(t) != example).map(|&t| ix.syms[t].qname()).collect();
            if !names.is_empty() {
                out += &format!("  {}\n", names.join("\n  "));
            }
        }
        if let Some(diag) =
            self.db.query_row("SELECT diagnostic FROM broken WHERE symbol = ?1", [s.qname()], |r| r.get::<_, String>(0)).optional()?
        {
            out += &format!("\n== Broken ==\n{diag}\n");
        }
        Ok(out)
    }

    fn refs(&self, name: &str) -> Out {
        let ix = self.index()?;
        // `refs <module>.<Type>.<field>`: the readers of one field (D-44)
        if let Some((ty, field)) = name.rsplit_once('.')
            && field.starts_with(|c: char| c.is_lowercase())
            && let Ok(t) = self.resolve(&ix, ty)
            && ix.syms[t].kind == Kind::Type
        {
            return Ok(reach::field_refs(&ix, t, field));
        }
        let i = self.resolve(&ix, name)?;
        let users = ix.used_by[i].clone();
        if users.is_empty() {
            return Ok(format!("Nothing uses {}.\n", ix.syms[i].qname()));
        }
        let mut out = format!("{} is used by {}:\n", ix.syms[i].qname(), users.len());
        if ix.syms[i].kind == Kind::Type {
            let (tests, users): (Vec<usize>, Vec<usize>) = users.into_iter().partition(|&j| is_test(&ix, j));
            out += &type_users(&ix, i, &users, &tests, usize::MAX);
            return Ok(out);
        }
        for u in users {
            let q = ix.syms[u].qname();
            out += &if shown_earlier(&q) {
                format!("-- {q} (above)\n")
            } else {
                format!("-- {q}\n{}\n", gleam::signature(&ix.syms[u].item()))
            };
            out += &call_lines(&ix, u, i);
            mark_shown(q);
        }
        Ok(out)
    }

    /// `impact [name...]`: what changed code reaches (D-44). With no names, the
    /// task's changes in a board run, otherwise the changes since `test` last
    /// passed; a name that hasn't changed counts as changed throughout.
    fn impact(&self, names: &[&str], budget: usize) -> Out {
        let ix = self.index()?;
        let before = match board::context() {
            Some((tid, _)) => {
                let mut st = self.db.prepare("SELECT entity, data FROM board_before WHERE task = ?1 AND entity LIKE 'symbol:%'")?;
                let rows: Vec<(String, String)> = st.query_map([&tid], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                let old = |data: &str| {
                    serde_json::from_str::<serde_json::Value>(data).ok().filter(|v| v[7] == 1).and_then(|v| v[4].as_str().map(String::from))
                };
                Some(rows.iter().map(|(k, d)| (k["symbol:".len()..].to_string(), old(d))).collect::<Vec<_>>())
            }
            None => self.since_pass()?,
        };
        let origins = if names.is_empty() {
            let Some(before) = before else {
                return Ok("Tests haven't passed in full yet, so there is no earlier state to compare with. Name the symbols: `impact <name>...`.\n".into());
            };
            reach::origins(&ix, &before)
        } else {
            let mut out = vec![];
            for name in names {
                let i = self.resolve(&ix, name)?;
                let then: Before = before.iter().flatten().filter(|(id, _)| *id == ix.syms[i].id).cloned().collect();
                out.extend(
                    Some(reach::origins(&ix, &then))
                        .filter(|o| !o.is_empty())
                        .unwrap_or_else(|| vec![(i, (0..ix.syms[i].text.lines().count()).collect())]),
                );
            }
            out
        };
        if origins.is_empty() {
            return Ok("No source changed since tests last passed.\n".into());
        }
        let impact = reach::flow(&ix, &origins);
        let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
        Ok(reach::report(&ix, &impact, &reach::reached(&reach::suites(self, &ix)?, &syms), budget))
    }

    /// The submit reply's note on what the saved change reaches (D-44).
    /// `before`: (id, text before the submit; `None` if new).
    fn impact_note(&self, ix: &Index, before: &[(String, Option<String>)]) -> Result<Vec<String>, Failure> {
        let origins = reach::origins(ix, before);
        if origins.is_empty() {
            return Ok(vec![]);
        }
        let impact = reach::flow(ix, &origins);
        let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
        Ok(reach::note(ix, &impact, &reach::reached(&reach::suites(self, ix)?, &syms)))
    }

    fn find(&self, query: &str) -> Out {
        let ix = self.index()?;
        let query = unquoted(query);
        let q = query.to_lowercase();
        let mut hits: Vec<(bool, String)> = vec![];
        for s in &ix.syms {
            let lines: Vec<String> = s
                .text
                .lines()
                .enumerate()
                .filter(|(_, l)| l.to_lowercase().contains(&q))
                .map(|(n, l)| format!("  {:>3}: {}", n + 1, l.trim()))
                .collect();
            let name_hit = s.name.to_lowercase().contains(&q);
            if name_hit || !lines.is_empty() {
                let more = if lines.len() > 4 { format!("\n  … {} more lines", lines.len() - 4) } else { String::new() };
                let shown: Vec<String> = lines.into_iter().take(4).collect();
                hits.push((name_hit, format!("{} ({})\n{}{}", s.qname(), s.kind.as_str(), shown.join("\n"), more)));
            }
        }
        if hits.is_empty() {
            return Ok(format!("No matches for \"{query}\".\n"));
        }
        hits.sort_by_key(|h| !h.0);
        let total = hits.len();
        let mut out: String = hits.iter().take(40).map(|h| h.1.trim_end().to_string() + "\n").collect();
        if total > 40 {
            out += &format!("… {} more symbols match; narrow the search.\n", total - 40);
        }
        Ok(out)
    }

    // --- changing ----------------------------------------------------------------

    fn submit(&self, target: Target, raw: &str) -> Out {
        let raw: String = raw.lines().filter(|l| !l.starts_with("// ── ")).collect::<Vec<_>>().join("\n");
        let formatted = format_gleam(&raw)?;
        let parsed = gleam::split(&formatted);
        let ix = self.index()?;
        let (Target::Imports(m) | Target::Module(m)) = target;
        let module = self.resolve_module(&ix, m).ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))?.path.clone();
        let mut notes = vec![];
        let header = &ix.modules.iter().find(|m| m.path == module).unwrap().header;
        let out = self.transaction(&[], || {
            let target_name: Option<String> = match target {
                Target::Module(_) => {
                    let existing: Vec<&Sym> = ix.syms.iter().filter(|s| s.module == module).collect();
                    let submitted: HashSet<(String, bool)> = parsed.items.iter().map(|i| name_key(&i.name, i.kind)).collect();
                    if submitted.len() != parsed.items.len() {
                        return fail("E0103 the module text defines the same name twice.");
                    }
                    let missing: Vec<String> =
                        existing.iter().filter(|s| !submitted.contains(&name_key(&s.name, s.kind))).map(|s| s.name.clone()).collect();
                    if !missing.is_empty() {
                        return fail(format!(
                            "E0102 the module text leaves out: {}. Nothing was saved. Removing a symbol must be explicit (solace delete <name>).",
                            missing.join(", ")
                        ));
                    }
                    let mut header = if parsed.header.is_empty() { header.clone() } else { parsed.header.clone() };
                    let (mut changed, mut added) = (vec![], vec![]);
                    for item in &parsed.items {
                        let test = ix.modules.iter().any(|m| m.path == module && m.dir == "test");
                        for path in missing_imports(&ix, &header, &item.text, test) {
                            header = add_import(&header, &format!("import {path}"));
                            notes.push(format!("added `import {path}` to {module}"));
                        }
                        if test {
                            notes.extend(weak_asserts(&item.text, &aliases(&header)).into_iter().map(|w| format!("{module}.{}: {w}", item.name)));
                        }
                        match existing.iter().find(|s| name_key(&s.name, s.kind) == name_key(&item.name, item.kind)) {
                            Some(e) if e.text == item.text => {}
                            Some(e) => {
                                self.db.execute(
                                    "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                                    params![e.id, item.text, revision(&item.text)],
                                )?;
                                self.history(&e.id, "edit", &item.name)?;
                                changed.push(item.name.clone());
                            }
                            None => {
                                let id = insert_symbol(&self.db, &module, item, None)?;
                                self.history(&id, "create", &item.name)?;
                                added.push(item.name.clone());
                            }
                        }
                    }
                    self.set_header(&module, &header)?;
                    let list = |v: &Vec<String>| {
                        if v.is_empty() { "none".to_string() } else { v.join(", ") }
                    };
                    notes.push(format!("{module}: changed {}; added {}", list(&changed), list(&added)));
                    None
                }
                Target::Imports(_) => {
                    if !parsed.items.is_empty() {
                        return fail("E0101 --imports text may only contain imports and the module comment.");
                    }
                    self.set_header(&module, &parsed.header)?;
                    None
                }
            };
            let ix = self.index()?;
            let before_broken = self.broken()?;
            let diags = self.check_all(&ix)?;
            let whole_module = matches!(target, Target::Module(_));
            let own = |d: &&Diag| match &target_name {
                Some(q) => d.symbol.as_deref() == Some(q.as_str()),
                None if whole_module => d.module.as_deref() == Some(module.as_str()),
                None => d.symbol.is_none() && d.module.as_deref() == Some(module.as_str()),
            };
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error).filter(own).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                let module_syms: Vec<String> = self.index()?.syms.iter().filter(|s| s.module == module).map(Sym::qname).collect();
                return fail(format!("Not saved: the submitted text has errors.\n\n{}{}", text.join("\n\n"), self.callee_hint(&own_errors, &module_syms)?));
            }
            if whole_module {
                let after = self.index()?;
                let before: Before = after
                    .syms
                    .iter()
                    .filter(|s| s.module == module)
                    .map(|s| {
                        let old = ix.syms.iter().find(|o| o.module == module && name_key(&o.name, o.kind) == name_key(&s.name, s.kind));
                        (s.id.clone(), old.map(|o| o.text.clone()))
                    })
                    .collect();
                notes.extend(self.impact_note(&after, &before)?);
            }
            let what = if whole_module { module.clone() } else { format!("{module} imports") };
            let mut out = format!("Saved {}.\n", target_name.as_deref().unwrap_or(&what));
            out += &warning_summary(diags.iter().filter(|d| !d.error).filter(own));
            out += &broken_report(&before_broken, &diags);
            Ok(out)
        })?;
        Ok(note_lines(&notes) + &out)
    }

    /// Runs `body` in one store transaction: committed, or rolled back with
    /// `created` files removed and the project files written again from the store.
    fn transaction(&self, created: &[PathBuf], body: impl FnOnce() -> Out) -> Out {
        self.db.execute_batch("BEGIN")?;
        match body() {
            Ok(out) => {
                self.db.execute_batch("COMMIT")?;
                Ok(out)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK")?;
                for path in created.iter().filter(|p| p.exists()) {
                    fs::remove_file(path)?;
                }
                self.materialize(&self.index()?)?;
                Err(e)
            }
        }
    }

    /// Replaces or adds symbols in one transaction, type-checked together, so a
    /// type change and its callers can land at once (PROPOSAL.md §4.6).
    /// `name`: the one symbol the text replaces. `new_in`: the module the text
    /// adds a symbol to. Neither: the text is blocks headed `// ── <name> ──`,
    /// as `view` and `inspect` print them; a name that doesn't exist yet adds
    /// a symbol to the module its prefix names.
    fn submit_symbols(&self, name: Option<&str>, new_in: Option<&str>, raw: &str) -> Out {
        let ix = self.index()?;
        let is_header = |l: &str| l.starts_with("// ── ");
        let blocks: Vec<(Option<String>, String)> =
            if raw.lines().filter(|l| is_header(l)).count() > 1 || (name.is_none() && new_in.is_none()) {
                let mut blocks: Vec<(Option<String>, String)> = vec![];
                for line in raw.lines() {
                    if let Some(h) = line.strip_prefix("// ── ") {
                        if h.starts_with("module ") {
                            return fail("E0101 a whole module goes through `submit <module>`.");
                        }
                        let n = h.split([' ', '(']).next().unwrap_or("").to_string();
                        blocks.push((Some(n), String::new()));
                    } else if let Some((_, text)) = blocks.last_mut() {
                        text.push_str(line);
                        text.push('\n');
                    } else if !line.trim().is_empty() {
                        return fail("E0101 text for `submit` with no name must be blocks, each headed `// ── <module>.<name> ──`.");
                    }
                }
                blocks
            } else {
                vec![(name.map(str::to_string), raw.lines().filter(|l| !is_header(l)).collect::<Vec<_>>().join("\n"))]
            };
        if blocks.is_empty() {
            return fail("E0101 nothing to submit.");
        }
        let mut notes: Vec<String> = vec![];
        if let Some(name) = name.filter(|_| blocks.len() > 1) {
            notes.push(format!("`submit {name}` submitted every block; plain `submit` does the same"));
        }
        // (module, the symbol replaced, the new item, imports in the text)
        let mut plan: Vec<(String, Option<usize>, Item, String)> = vec![];
        // Modules that don't exist yet are created, with the module comment and
        // imports of their first block: (path, dir, header).
        let mut created: Vec<(String, &str, String)> = vec![];
        for (block_name, text) in &blocks {
            let parsed = gleam::split(&format_gleam(text)?);
            // `--new` adds every symbol in the text; a named block holds one.
            if parsed.items.is_empty() || (new_in.is_none() && parsed.items.len() > 1) {
                return fail(format!(
                    "E0101 each block holds one symbol; {} defines {} ({}). To add several to one module, use `submit --new <module>`.",
                    block_name.as_deref().unwrap_or("the text"),
                    parsed.items.len(),
                    parsed.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            let mut create = |m: &str| {
                if !created.iter().any(|c| c.0 == m) {
                    let dir = if m.rsplit('/').next().unwrap().ends_with("_test") { "test" } else { "src" };
                    created.push((m.to_string(), dir, parsed.header.clone()));
                }
                m.to_string()
            };
            let module_of = |m: &str| {
                self.resolve_module(&ix, m)
                    .or_else(|| m.rsplit_once('.').and_then(|(p, _)| self.resolve_module(&ix, p)))
                    .map(|r| r.path.clone())
                    .ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))
            };
            let (module, existing) = match (new_in, block_name) {
                (Some(m), _) => match module_of(m) {
                    Ok(path) => (path, None),
                    Err(_) if valid_module_path(m) => (create(m), None),
                    Err(e) => return Err(e),
                },
                (None, Some(n)) => match self.resolve(&ix, n) {
                    Ok(i) => (ix.syms[i].module.clone(), Some(i)),
                    Err(e) => match n.rsplit_once('.') {
                        Some((m, _)) if self.resolve_module(&ix, m).is_some() => (module_of(m)?, None),
                        Some((m, _)) if valid_module_path(m) => (create(m), None),
                        _ => return Err(e),
                    },
                },
                (None, None) => unreachable!(),
            };
            for item in &parsed.items {
                let clash = ix.syms.iter().position(|s| {
                    s.module == module
                        && s.name == item.name
                        && (s.kind == Kind::Type) == (item.kind == Kind::Type)
                        && existing != Some(ix.syms.iter().position(|x| x.id == s.id).unwrap())
                });
                if let Some(c) = clash {
                    return fail(format!("E1004 {} already exists; submit to it instead.", ix.syms[c].qname()));
                }
                if plan.iter().any(|p| p.0 == module && p.1.is_none() && name_key(&p.2.name, p.2.kind) == name_key(&item.name, item.kind)) {
                    return fail(format!("E0103 the text defines {module}.{} twice.", item.name));
                }
                plan.push((module.clone(), existing, item.clone(), parsed.header.clone()));
            }
        }

        let created_files: Vec<PathBuf> =
            created.iter().map(|(module, dir, _)| self.root.join(dir).join(format!("{module}.gleam"))).collect();
        if created_files.iter().any(|p| p.exists()) {
            return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
        }
        let out = self.transaction(&created_files, || {
            for (path, dir, header) in &created {
                self.db.execute("INSERT INTO modules VALUES (?1, ?2, ?3)", params![path, dir, header])?;
                notes.push(format!("created module {path} ({dir}/{path}.gleam)"));
            }
            let (mut saved, mut unchanged): (Vec<String>, Vec<String>) = (vec![], vec![]);
            let mut changed = !created.is_empty();
            for (module, existing, item, imports) in &plan {
                let mut header = self.header(module)?;
                let was = header.clone();
                for (path, items, _) in import_statements(imports) {
                    let merged = merge_import(&header, &path, &items);
                    if merged != header {
                        notes.push(format!("imported {path} in {module}"));
                        header = merged;
                    }
                }
                let test = ix.modules.iter().any(|m| &m.path == module && m.dir == "test")
                    || created.iter().any(|(p, d, _)| p == module && *d == "test");
                for path in missing_imports(&ix, &header, &item.text, test) {
                    header = merge_import(&header, &path, &[]);
                    notes.push(format!("added `import {path}` to {module}"));
                }
                if test {
                    notes.extend(weak_asserts(&item.text, &aliases(&header)).into_iter().map(|w| format!("{module}.{}: {w}", item.name)));
                }
                self.set_header(module, &header)?;
                changed |= header != was;
                match existing {
                    // the same text as stored: not saved again (D-46)
                    Some(e) if ix.syms[*e].text == item.text && ix.syms[*e].name == item.name && ix.syms[*e].kind == item.kind => {
                        unchanged.push(format!("{module}.{}", item.name));
                        continue;
                    }
                    Some(e) => {
                        let e = &ix.syms[*e];
                        self.db.execute(
                            "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, revision = ?5 WHERE id = ?1",
                            params![e.id, item.name, item.kind.as_str(), item.text, revision(&item.text)],
                        )?;
                        self.history(&e.id, "edit", &item.name)?;
                    }
                    None => {
                        let id = insert_symbol(&self.db, module, item, None)?;
                        self.history(&id, "create", &item.name)?;
                    }
                }
                saved.push(format!("{module}.{}", item.name));
                changed = true;
            }
            if !changed {
                let broken = self.broken()?;
                let still: Vec<&String> = unchanged.iter().filter(|u| broken.contains_key(*u)).collect();
                let still = if still.is_empty() {
                    String::new()
                } else {
                    format!("Still broken: {} (`check` shows why).\n", still.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                };
                return Ok(format!(
                    "Unchanged: {}. The text equals the stored symbols, so nothing was saved or checked.\n{still}",
                    unchanged.join(", ")
                ));
            }
            let modules: HashSet<&str> = plan.iter().map(|p| p.0.as_str()).collect();
            let own = |d: &Diag| match &d.symbol {
                Some(s) => saved.contains(s),
                None => d.module.as_deref().is_some_and(|m| modules.contains(m)),
            };
            let before_broken = self.broken()?;
            let mut diags = self.check_all(&self.index()?)?;
            // Unqualified types and constructors (`Option`, `None`, a project
            // type) that aren't imported: import them and check once more.
            // The compiler stops at the first module with errors, so repeat.
            for _ in 0..plan.len() + 2 {
                let mut fixed = false;
                for d in diags.iter().filter(|d| d.error && own(d)) {
                    if let (Some(m), Some((path, item))) = (&d.module, unimported_name(&ix, &d.text)) {
                        let header = self.header(m)?;
                        let merged = merge_import(&header, &path, std::slice::from_ref(&item));
                        if merged != header && path != *m {
                            self.set_header(m, &merged)?;
                            notes.push(format!("imported `{item}` from {path} in {m}"));
                            fixed = true;
                        }
                    }
                }
                if !fixed {
                    break;
                }
                diags = self.check_all(&self.index()?)?;
            }
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                return fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}{}",
                    text.join("\n\n"),
                    self.callee_hint(&own_errors, &saved)?
                ));
            }
            let after = self.index()?;
            let before: Before = plan
                .iter()
                .filter_map(|(module, existing, item, _)| {
                    let s = after.syms.iter().find(|s| &s.module == module && s.name == item.name && s.kind == item.kind)?;
                    Some((s.id.clone(), existing.map(|e| ix.syms[e].text.clone())))
                })
                .collect();
            notes.extend(self.impact_note(&after, &before)?);
            let mut out = format!("Saved {}.\n", saved.join(", "));
            if !unchanged.is_empty() {
                out += &format!("Unchanged (the same as stored): {}.\n", unchanged.join(", "));
            }
            out += &warning_summary(diags.iter().filter(|d| !d.error && own(d)));
            out += &broken_report(&before_broken, &diags);
            // The compiler stops at the first module with errors, so name the
            // other callers of a changed function, or the sites that construct a
            // changed type, from the reference graph (D-46).
            if diags.iter().any(|d| d.error) {
                let after = self.index()?;
                let broken: HashSet<&str> = diags.iter().filter_map(|d| d.symbol.as_deref()).collect();
                let mut unchecked: Vec<String> = vec![];
                for (_, existing, item, _) in &plan {
                    let Some(e) = existing else { continue };
                    let before = &ix.syms[*e];
                    let Some(a) = after.syms.iter().position(|s| s.id == before.id) else {
                        continue;
                    };
                    if gleam::head(&before.item()) == gleam::head(item) {
                        continue;
                    }
                    for &j in &after.used_by[a] {
                        let n = after.syms[j].qname();
                        let site = item.kind != Kind::Type || !sites(&after, j, a, false).is_empty();
                        if site && !broken.contains(n.as_str()) && !saved.contains(&n) && !unchecked.contains(&n) {
                            unchecked.push(n);
                        }
                    }
                }
                if !unchecked.is_empty() {
                    out += &format!(
                        "\nNot checked until the errors above are fixed, and may need the same change: {}\n",
                        unchecked.join(", ")
                    );
                }
            }
            Ok(out)
        })?;
        Ok(note_lines(&notes) + &out)
    }

    /// For a rejected submit whose every error is a call, with the wrong arity
    /// or argument types, to a project function the submission leaves out: the
    /// advice to submit that function too, in the same `submit` (D-46).
    fn callee_hint(&self, errors: &[&Diag], submitted: &[String]) -> Result<String, Failure> {
        let ix = self.index()?;
        let mut callees: Vec<String> = vec![];
        for d in errors {
            let title = d.text.lines().next().unwrap_or("");
            let place = place(d);
            let Some((sym, line)) = place.rsplit_once(", line ") else { return Ok(String::new()) };
            let Some(j) = ix.syms.iter().position(|s| s.qname() == sym) else { return Ok(String::new()) };
            let called: Vec<String> = ix.at[j]
                .iter()
                .filter(|(t, l)| line == (l + 1).to_string() && ix.syms[*t].kind == Kind::Fn)
                .map(|(t, _)| ix.syms[*t].qname())
                .filter(|q| !submitted.contains(q))
                .collect();
            if !(title.contains("Incorrect arity") || title.contains("Type mismatch")) || called.is_empty() {
                return Ok(String::new());
            }
            callees.extend(called.into_iter().filter(|c| !callees.contains(c)).collect::<Vec<_>>());
        }
        let names: Vec<String> = callees.iter().map(|c| format!("`{c}`")).collect();
        Ok(format!("\n\nIf you are changing {} too, submit both in one `submit`, a block each.", names.join(", ")))
    }

    fn header(&self, module: &str) -> Result<String, Failure> {
        Ok(self.db.query_row("SELECT header FROM modules WHERE path = ?1", [module], |r| r.get(0))?)
    }

    fn set_header(&self, module: &str, header: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE modules SET header = ?2 WHERE path = ?1", params![module, header])?;
        Ok(())
    }

    /// Takes a symbol out of the project; its history stays.
    fn retire(&self, id: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [id])?;
        self.history(id, "retire", detail)
    }

    fn history(&self, id: &str, action: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute(
            "INSERT INTO history(at, symbol, action, detail) VALUES (?1, ?2, ?3, ?4)",
            params![now() as i64, id, action, detail],
        )?;
        Ok(())
    }

    fn delete(&self, name: &str) -> Out {
        let ix = self.index()?;
        let i = self.resolve(&ix, name)?;
        let before = self.broken()?;
        let users: Vec<String> = ix.used_by[i].iter().map(|&j| ix.syms[j].qname()).collect();
        if !users.is_empty() {
            return fail(format!("E5001 {} is still used by:\n  {}", ix.syms[i].qname(), users.join("\n  ")));
        }
        self.retire(&ix.syms[i].id, "")?;
        let diags = self.check_all(&self.index()?)?;
        Ok(format!("Deleted {}.\n{}", ix.syms[i].qname(), broken_report(&before, &diags)))
    }

    fn check(&self) -> Out {
        self.check_report(false)
    }

    fn check_report(&self, show_warnings: bool) -> Out {
        let diags = self.check_all(&self.index()?)?;
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        let warnings = diags.len() - errors.len();
        let details = if show_warnings {
            diags.iter().filter(|d| !d.error).map(|d| d.text.as_str()).collect::<Vec<_>>().join("\n\n")
        } else {
            String::new()
        };
        if errors.is_empty() {
            Ok(format!("No errors ({warnings} warnings).\n{details}"))
        } else {
            fail(format!("{} errors, {warnings} warnings:\n\n{}\n{details}", errors.len(), errors.join("\n\n")))
        }
    }

    /// `test [--all | <name>...]`: the tests that can see a change since they
    /// last passed, that failed or were never run, or that reach other
    /// processes, the network or foreign code; with names, those that reach
    /// them; `--all`, every test (D-44). Tests left out are made private in the
    /// projected files for the run, so gleeunit skips them on either target.
    fn test(&self, args: &[&str]) -> Out {
        let ix = self.index()?;
        let named: Vec<usize> = operands(args).iter().map(|n| self.resolve(&ix, n)).collect::<Result<_, _>>()?;
        let all = args.contains(&"--all");
        let tests = reach::tests(&ix);
        let revisions: Vec<String> = ix.syms.iter().map(|s| revision(&s.text)).collect();
        let outside = reach::outside(&ix);
        let mut rows: HashMap<String, (String, String)> = self
            .db
            .prepare("SELECT test, reach, result FROM test_runs")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()?;
        let (mut run, mut hashes) = (vec![], HashMap::new());
        for &t in &tests {
            let reach = reach::closure(&ix, t);
            let hash = reach::hash(&ix, &reach, &revisions, &self.root);
            let due = if all {
                true
            } else if !named.is_empty() {
                named.iter().any(|n| reach.contains(n))
            } else {
                rows.get(&ix.syms[t].qname()).is_none_or(|(h, r)| *h != hash || r != "pass") || reach.iter().any(|j| outside.contains(j))
            };
            // a test other code calls can't be made private
            if due || ix.uses.iter().enumerate().any(|(j, u)| j != t && u.contains(&t)) {
                run.push(t);
            }
            hashes.insert(t, hash);
        }
        // Suites outside `gleam test` that reach a change since tests last passed.
        let changed: Vec<usize> =
            self.since_pass()?.unwrap_or_default().iter().filter_map(|(id, _)| ix.syms.iter().position(|s| &s.id == id)).collect();
        let suites = reach::reached(&reach::suites(self, &ix)?, &changed);
        let head = if run.len() == tests.len() {
            format!("Ran all {} tests.\n", tests.len())
        } else if !named.is_empty() {
            format!("Ran {} of {} tests: those that reach {}.\n", run.len(), tests.len(), operands(args).join(", "))
        } else {
            format!(
                "Ran {} of {} tests; the other {} reach nothing changed since they passed (`test --all` runs every test).\n",
                run.len(),
                tests.len(),
                tests.len() - run.len()
            )
        };
        let also = if suites.is_empty() { String::new() } else { format!("Also reached: {}.\n", suites.join(", ")) };
        if run.is_empty() {
            // gleeunit fails when it finds no tests; type-check instead
            let errors: Vec<String> = self.check_all(&ix)?.into_iter().filter(|d| d.error).map(|d| d.text).collect();
            if !errors.is_empty() {
                return fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
            }
            return Ok(head + &also);
        }
        let files = self.materialize(&ix)?;
        let skipped: Vec<(usize, String)> = tests
            .iter()
            .filter(|t| !run.contains(t))
            .map(|&t| (t, ix.syms[t].text.replacen(&format!("pub fn {}(", ix.syms[t].name), &format!("fn {}(", ix.syms[t].name), 1)))
            .collect();
        let rewritten = self.rewritten(&ix, &skipped)?;
        let result = (|| {
            for (path, content) in &rewritten {
                self.rewrite(path, content)?;
            }
            self.gleam(&["test"])
        })();
        if !rewritten.is_empty() {
            self.materialize(&ix)?;
        }
        let (ok, output) = result?;
        let diags = diagnostics(&output, &files);
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        if !errors.is_empty() {
            return fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
        }
        // Record results only from a run that finished; passes only when every
        // failure it counted was attributed to a test.
        let failed = failures(&output);
        let summary = output.lines().find(|l| l.contains(" passed, ")).map(|l| {
            let number = |word: &str| l.split(word).next().and_then(|p| p.split_whitespace().last()).and_then(|n| n.parse::<usize>().ok());
            if l.contains("no failures") { Some(0) } else { number(" failure") }
        });
        if let Some(count) = summary {
            let complete = ok || count == Some(failed.len());
            for &t in &run {
                let q = ix.syms[t].qname();
                let result = match failed.get(&q) {
                    Some(_) => "fail",
                    None if complete => "pass",
                    None => continue,
                };
                self.db.execute("INSERT OR REPLACE INTO test_runs VALUES (?1, ?2, ?3)", params![q, hashes[&t], result])?;
                rows.insert(q, (hashes[&t].clone(), result.to_string()));
            }
            if tests.iter().all(|t| rows.get(&ix.syms[*t].qname()).is_some_and(|(h, r)| *h == hashes[t] && r == "pass")) {
                self.db.execute_batch("DELETE FROM test_base; INSERT INTO test_base SELECT id, text FROM symbols WHERE live = 1;")?;
            }
        }
        let mut out = head;
        for line in quiet(&output).lines() {
            let line = line.trim_start_matches('.');
            if line.trim().is_empty() || line.trim().starts_with("Checking ") {
                continue;
            }
            out += &relocate(line, &files);
            out.push('\n');
        }
        out += &also;
        if ok { Ok(out) } else { fail(out) }
    }

    /// Symbols changed since `test` last passed in full: (id, text then, `None`
    /// if new). `None` if it never has.
    fn since_pass(&self) -> Result<Option<Before>, Failure> {
        if !self.db.query_row("SELECT EXISTS(SELECT 1 FROM test_base)", [], |r| r.get::<_, bool>(0))? {
            return Ok(None);
        }
        let rows = self
            .db
            .prepare("SELECT s.id, b.text FROM symbols s LEFT JOIN test_base b ON b.id = s.id WHERE s.live = 1 AND (b.text IS NULL OR b.text != s.text)")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(Some(rows))
    }

    /// Runs throwaway code against the current project and returns its output.
    /// The text is a module with `pub fn main()`, or statements that become
    /// its body; project imports it uses are added. Nothing is saved: the file
    /// exists only while it runs.
    fn probe(&self, text: &str, from: Option<&str>, trace: &[&str]) -> Out {
        let ix = self.index()?;
        if ix.modules.iter().any(|m| m.path == PROBE) {
            return fail(format!("E0101 a project module is named {PROBE}; probe can't run."));
        }
        let (prelude, setup) = match from {
            Some(name) => self.probe_from(&ix, name)?,
            None => (String::new(), String::new()),
        };
        // A single expression is printed: `probe` with `money.percent(7400, 5)` shows 370.
        let one = text.trim();
        let text = if !one.is_empty()
            && !one.contains('\n')
            && !["echo ", "let ", "assert ", "import ", "use ", "panic", "todo"].iter().any(|k| one.starts_with(k))
        {
            format!("echo {one}")
        } else {
            text.to_string()
        };
        let mut code = if text.contains("fn main(") {
            if from.is_some() {
                return fail("E1005 --from adds statements to a test's body; give statements, not a module.");
            }
            text
        } else {
            let (imports, body): (Vec<&str>, Vec<&str>) = text.lines().partition(|l| l.starts_with("import "));
            format!("{prelude}{}\n\npub fn main() {{\n{setup}{}\n}}\n", imports.join("\n"), body.join("\n"))
        };
        // the probe lives in test/, so test helpers such as `support` are importable
        for path in missing_imports(&ix, &code, &code, true) {
            code = format!("import {path}\n{code}");
        }
        self.materialize(&ix)?;
        let traced = self.trace_sources(&ix, trace)?;
        let file = self.root.join("test").join(format!("{PROBE}.gleam"));
        let result = (|| -> Result<(bool, String), Failure> {
            for (path, content) in &traced {
                self.rewrite(path, content)?;
            }
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, &code)?;
            self.gleam(&["run", "-m", PROBE])
        })();
        // Remove the probe and put traced modules back in every case.
        let _ = fs::remove_file(&file);
        if !traced.is_empty() {
            self.materialize(&ix)?;
        }
        let (ok, output) = result?;
        // Compiler warnings (mostly the probe's unused imports) are noise here.
        let mut out = quiet(&output);
        if !trace.is_empty() {
            out = trace_report(&out);
        }
        if !ok {
            out += &format!("\nThe probe ran as test/{PROBE}.gleam:\n{code}");
        }
        if ok { Ok(out) } else { fail(out) }
    }

    /// `probe --from <fn>`: the imports of its module and its body, so a
    /// probe can start from a test's setup and use its local variables.
    fn probe_from(&self, ix: &Index, name: &str) -> Result<(String, String), Failure> {
        let s = &ix.syms[self.resolve(ix, name)?];
        let open = fn_body_start(s).filter(|_| s.kind == Kind::Fn);
        let Some(open) = open else {
            return fail(format!("E1005 {} isn't a function; --from takes a test or another function without arguments.", s.qname()));
        };
        if !params(&gleam::head(&s.item()), &s.name).is_empty() {
            return fail(format!("E1005 {} takes arguments; --from takes a test or another function without arguments.", s.qname()));
        }
        let body = &s.text[open + 1..s.text.rfind('}').unwrap_or(s.text.len())];
        let imports: String =
            self.header(&s.module)?.lines().filter(|l| !l.trim_start().starts_with("//")).map(|l| format!("{l}\n")).collect();
        Ok((imports, format!("{}\n", body.trim_matches('\n'))))
    }

    /// The materialized module files with each traced function wrapped so
    /// that its arguments and result are echoed (D-43). Nothing is written here.
    fn trace_sources(&self, ix: &Index, names: &[&str]) -> Result<Vec<(PathBuf, String)>, Failure> {
        let mut edits = vec![];
        for name in names {
            let i = self.resolve(ix, name)?;
            let s = &ix.syms[i];
            if s.kind != Kind::Fn || s.text.contains("@external") {
                return fail(format!("E1006 {} can't be traced: only functions written in Gleam can.", s.qname()));
            }
            edits.push((i, trace_wrap(s)));
        }
        self.rewritten(ix, &edits)
    }

    /// The materialized files of the modules holding `edits` (symbol, new
    /// text), with those symbols' text replaced. Nothing is written here.
    fn rewritten(&self, ix: &Index, edits: &[(usize, String)]) -> Result<Vec<(PathBuf, String)>, Failure> {
        let mut files: Vec<(String, PathBuf, String)> = vec![];
        for (i, text) in edits {
            let s = &ix.syms[*i];
            let Some(m) = ix.modules.iter().find(|m| m.path == s.module) else { continue };
            if !files.iter().any(|f| f.0 == m.path) {
                let path = self.root.join(&m.dir).join(format!("{}.gleam", m.path));
                let content = fs::read_to_string(&path)?;
                files.push((m.path.clone(), path, content));
            }
            let f = files.iter_mut().find(|f| f.0 == m.path).unwrap();
            f.2 = f.2.replacen(&s.text, text, 1);
        }
        Ok(files.into_iter().map(|(_, p, c)| (p, c)).collect())
    }

    fn work_add(&self, text: &str) -> Out {
        self.db.execute("INSERT INTO work(text) VALUES (?1)", [text])?;
        Ok(format!("Work record #{} added.\n", self.db.last_insert_rowid()))
    }

    fn work_list(&self) -> Out {
        self.status()
    }

    fn work_done(&self, id: &str) -> Out {
        let id: i64 = id.trim_start_matches('#').parse().map_err(|_| Failure(format!("Not a work record id: {id}"), 2))?;
        match self.db.execute("UPDATE work SET open = 0 WHERE id = ?1", [id])? {
            0 => fail(format!("No work record #{id}.")),
            _ => Ok(format!("Work record #{id} closed.\n")),
        }
    }
}

/// Failed tests in `gleam test` output by `module.name` (Erlang prints
/// modules as `pilot@x_test`): whether wrong or unfinished, and the failure
/// block (`assert …`, `test:`, `code:`, `left:`, `right:`, `info:`).
fn failures(output: &str) -> BTreeMap<String, (String, String)> {
    let clean = strip_ansi(output);
    let mut out = BTreeMap::new();
    let mut block: Vec<&str> = vec![];
    for line in clean.lines().map(str::trim).chain([""]) {
        let start = ["todo ", "assert ", "let assert ", "panic "].iter().any(|p| line.starts_with(p));
        if (start || line.is_empty()) && !block.is_empty() {
            if let Some(name) = block.iter().find_map(|l| l.strip_prefix("test: ")) {
                let kind = if block[0].starts_with("todo ") { "unfinished" } else { "wrong" };
                out.insert(name.replace('@', "/"), (kind.into(), block.join("\n")));
            }
            block.clear();
        }
        if start || (!block.is_empty() && !line.is_empty()) {
            block.push(line);
        }
    }
    out
}

/// Search terms: identifiers split at `_` and case changes, lowercased, a
/// plural `s` dropped; short words, numbers and common English words skipped.
fn words(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "when", "should", "must", "into", "are", "was", "were", "been", "has", "have",
        "not", "but", "any", "all", "its", "also", "each", "only", "than", "then", "there", "their", "they", "which", "will", "would",
        "can", "could", "does", "did", "you", "your", "new", "add", "make", "use", "using", "test", "tests", "existing", "keep", "one",
        "other", "what", "how", "may", "per", "such",
    ];
    let mut out = vec![];
    for tok in text.split(|c: char| !c.is_alphanumeric()) {
        let mut cur = String::new();
        let mut prev_lower = false;
        for c in tok.chars() {
            if c.is_uppercase() && prev_lower {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.extend(c.to_lowercase());
        }
        out.push(cur);
    }
    out.into_iter()
        .map(|w| if w.len() > 4 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_string() } else { w })
        .filter(|w| w.len() >= 3 && !w.chars().all(|c| c.is_ascii_digit()) && !STOP.contains(&w.as_str()))
        .collect()
}

/// Runs of capitalized words in the task, lowercased ("Desk Lamp", "TENOFF"),
/// leaving out a single word that starts a sentence.
fn capitalized_runs(task: &str) -> Vec<String> {
    let (mut out, mut run, mut start) = (vec![], vec![], true);
    for token in task.split_whitespace() {
        let word = token.trim_matches(|c: char| !c.is_alphanumeric());
        let capital = word.starts_with(|c: char| c.is_uppercase());
        if capital {
            run.push((word, start));
        }
        if !capital || token.ends_with(|c: char| !c.is_alphanumeric()) {
            if run.len() > 1 || run.first().is_some_and(|(w, s)| !s && w.len() >= 3) {
                out.push(run.iter().map(|r| r.0).collect::<Vec<_>>().join(" ").to_lowercase());
            }
            run.clear();
        }
        start = token.trim_end_matches(['"', '\'', ')']).ends_with(['.', '!', '?']);
    }
    out
}

/// String literals in `text` holding one of `terms` (lowercase) at a word's
/// start, each shown as
/// the smallest bracketed expression around it, on one line:
/// `coupon("TENOFF", FixedOff(1000))`. A block or a long expression gives the
/// literal's line instead.
fn evidence(text: &str, terms: &[String]) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    // chars inside string literals and comments, and the literals' spans
    let (mut inside, mut literals, mut k) = (vec![false; chars.len()], vec![], 0);
    while k < chars.len() {
        let start = k;
        if chars[k] == '"' {
            k += 1;
            while k < chars.len() && chars[k] != '"' {
                k += if chars[k] == '\\' { 2 } else { 1 };
            }
            literals.push((start, k.min(chars.len() - 1)));
        } else if chars[k] == '/' && chars.get(k + 1) == Some(&'/') {
            while k < chars.len() && chars[k] != '\n' {
                k += 1;
            }
        } else {
            k += 1;
            continue;
        }
        for c in inside.iter_mut().take((k + 1).min(chars.len())).skip(start) {
            *c = true;
        }
        k += 1;
    }
    let one_line = |cs: &[char]| -> String {
        let s: String = cs.iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        s.replace(", )", ")").replace(", ]", "]").replace("( ", "(").replace(" )", ")").replace("[ ", "[").replace(" ]", "]")
    };
    let mut out: Vec<String> = vec![];
    for (s, e) in literals {
        let literal = chars[s..=e].iter().collect::<String>().to_lowercase();
        // at a word's start: "lamp" in "LAMP-OAK", not "come" in "WELCOME15"
        let starts_word = |t: &String| literal.match_indices(t.as_str()).any(|(p, _)| !literal[..p].ends_with(char::is_alphanumeric));
        if !terms.iter().any(starts_word) {
            continue;
        }
        let mut depth = 0;
        let open = (0..s).rev().filter(|&j| !inside[j]).find(|&j| match chars[j] {
            ')' | ']' | '}' => {
                depth += 1;
                false
            }
            '(' | '[' | '{' if depth == 0 => true,
            '(' | '[' | '{' => {
                depth -= 1;
                false
            }
            _ => false,
        });
        let expression = open.filter(|&o| chars[o] != '{').and_then(|o| {
            let mut depth = 0;
            let close = (o..chars.len()).filter(|&j| !inside[j]).find(|&j| match chars[j] {
                '(' | '[' | '{' => {
                    depth += 1;
                    false
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    depth == 0
                }
                _ => false,
            })?;
            let mut from = o;
            while from > 0 && (chars[from - 1].is_alphanumeric() || "_.".contains(chars[from - 1])) {
                from -= 1;
            }
            Some(one_line(&chars[from..=close])).filter(|x| x.len() <= 200)
        });
        let line = || {
            let a = chars[..s].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
            let b = chars[s..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| s + p);
            one_line(&chars[a..b]).chars().take(200).collect()
        };
        let shown = expression.unwrap_or_else(line);
        if !out.contains(&shown) {
            out.push(shown);
        }
    }
    out
}

fn module_doc(m: &ModuleRow) -> &str {
    m.header.lines().find(|l| l.starts_with("////")).map_or("", |l| l.trim_start_matches('/').trim())
}

fn is_test(ix: &Index, i: usize) -> bool {
    ix.modules.iter().any(|m| m.path == ix.syms[i].module && m.dir == "test")
}

/// A type's users for `inspect --context` and `refs`: the sites that
/// construct or match it with every field, one line each, then the other
/// non-test users by name (at most `cap`).
fn type_users(ix: &Index, ty: usize, users: &[usize], tests: &[usize], cap: usize) -> String {
    let mut out = String::new();
    let (mut built, mut others, mut updates) = (vec![], vec![], 0);
    for &j in users.iter().chain(tests) {
        updates += sites(ix, j, ty, true).len();
        let at = sites(ix, j, ty, false);
        if at.is_empty() && !tests.contains(&j) {
            others.push(ix.syms[j].qname());
        }
        built.extend(at.into_iter().map(|(n, line)| format!("-- {}, line {n}: {line}\n", ix.syms[j].qname())));
    }
    let q = ix.syms[ty].qname();
    if !built.is_empty() {
        out += &format!("\n== Constructed by ({}): every field listed, so a new field breaks these ==\n{}", built.len(), built.concat());
        if updates > 0 {
            out += &format!("Record updates and patterns with `..` need no change; {updates} not listed.\n");
        }
    }
    if !others.is_empty() {
        let more = if others.len() > cap { format!(", …and {} more (solace refs {q})", others.len() - cap) } else { String::new() };
        out += &format!(
            "\n== Other users ({}); `refs {q}.<field>` lists a field's readers ==\n  {}{more}\n",
            others.len(),
            others.iter().take(cap).cloned().collect::<Vec<_>>().join(", ")
        );
    }
    out
}

/// The lines where `caller` calls function `f`, quoted under its entry.
fn call_lines(ix: &Index, caller: usize, f: usize) -> String {
    if ix.syms[f].kind != Kind::Fn {
        return String::new();
    }
    ref_lines(ix, caller, f).iter().map(|(n, line)| format!("  > line {n}: {line}\n")).collect()
}

/// The lines where symbol `j` refers to `i`: (line from 1, that source line).
fn ref_lines(ix: &Index, j: usize, i: usize) -> Vec<(usize, String)> {
    let lines: Vec<&str> = ix.syms[j].text.lines().collect();
    let mut at: Vec<usize> = ix.at[j].iter().filter(|r| r.0 == i).map(|r| r.1).collect();
    at.dedup();
    at.into_iter().filter_map(|n| Some((n + 1, lines.get(n)?.trim().to_string()))).collect()
}

/// Where `j` constructs or matches type `ty` listing every field (no `..`):
/// the places that break when a field is added (D-46). With `updates`, the
/// record updates and patterns with `..` instead, which don't.
// ponytail: a generic type's annotation such as `Box(Int)` also matches; check the position if that misleads.
fn sites(ix: &Index, j: usize, ty: usize, updates: bool) -> Vec<(usize, String)> {
    let ctors = gleam::constructors(&ix.syms[ty].text);
    let text = &ix.syms[j].text;
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(k, _)| k + 1)).collect();
    ref_lines(ix, j, ty)
        .into_iter()
        .filter(|(n, line)| {
            let at = starts[n - 1] + text[starts[n - 1]..].len() - text[starts[n - 1]..].trim_start().len();
            ctors.iter().any(|c| {
                line.match_indices(&format!("{c}(")).any(|(k, _)| {
                    let whole = !line[..k].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
                    whole && updates == gleam::split_args(&text[at + k + c.len() + 1..]).iter().any(|(_, a)| a.trim().starts_with(".."))
                })
            })
        })
        .collect()
}

thread_local! {
    /// The symbols whose signature or text an earlier command in this MCP call
    /// printed, so `inspect` and `refs` can say `(above)` (D-46); `None`
    /// outside a call.
    static SHOWN: std::cell::RefCell<Option<HashSet<String>>> = const { std::cell::RefCell::new(None) };
}

fn shown_earlier(q: &str) -> bool {
    SHOWN.with(|s| s.borrow().as_ref().is_some_and(|s| s.contains(q)))
}

fn mark_shown(q: String) {
    SHOWN.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.insert(q);
        }
    })
}

/// Formats Gleam source, which doubles as the syntax check.
fn format_gleam(source: &str) -> Result<String, Failure> {
    let mut child =
        Command::new("gleam").args(["format", "--stdin"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(source.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let msg = strip_ansi(&String::from_utf8_lossy(&out.stderr)).replace("<stdin>", "submitted text");
        fail(format!("Not saved: E0100 syntax error.\n\n{}", msg.trim()))
    }
}

/// Where a function's body opens: the first `{` outside parentheses after
/// its `fn name(`.
fn fn_body_start(s: &Sym) -> Option<usize> {
    let at = s.text.find(&format!("fn {}(", s.name))?;
    Some(at + gleam::body_start(&s.text[at..])?)
}

/// A function's parameters from its signature: (label if any, name).
fn params(signature: &str, name: &str) -> Vec<(Option<String>, String)> {
    let Some(at) = signature.find(&format!("fn {name}(")) else { return vec![] };
    gleam::split_args(&signature[at + name.len() + 4..])
        .iter()
        .map(|(_, p)| p.split(':').next().unwrap_or("").split_whitespace().collect::<Vec<_>>())
        .filter(|words| !words.is_empty())
        .map(|words| (if words.len() > 1 { Some(words[0].to_string()) } else { None }, words[words.len() - 1].to_string()))
        .collect()
}

/// A traced function: the original renamed and private, and a wrapper with
/// its name, visibility and labels that echoes the arguments and the result.
fn trace_wrap(s: &Sym) -> String {
    let inner = format!("{}__solace_traced", s.name);
    let decl = format!("fn {}(", s.name);
    let public = s.text.lines().any(|l| l.trim_start().starts_with(&format!("pub {decl}")));
    let renamed = s.text.replacen(&format!("pub {decl}"), &decl, 1).replacen(&decl, &format!("fn {inner}("), 1);
    let ps = params(&gleam::head(&s.item()), &s.name);
    let names: Vec<String> = (0..ps.len()).map(|i| format!("p{i}")).collect();
    let decls: Vec<String> =
        ps.iter().zip(&names).map(|((label, _), n)| label.as_ref().map_or(n.clone(), |l| format!("{l} {n}"))).collect();
    let q = s.qname();
    let args: String = names.iter().map(|n| format!(", {n}")).collect();
    format!(
        "{renamed}\n\n{}fn {}({}) {{\n  echo #(\"solace-call\", \"{q}\"{args})\n  let r = {inner}({})\n  echo #(\"solace-return\", \"{q}\", r)\n  r\n}}",
        if public { "pub " } else { "" },
        s.name,
        decls.join(", "),
        names.join(", ")
    )
}

/// The probe's output with the trace's echoes shown as nested calls and
/// results, values cut short, and at most `MAX` calls.
fn trace_report(output: &str) -> String {
    const MAX: usize = 60;
    const WIDTH: usize = 300;
    let cut = |v: &str| if v.chars().count() > WIDTH { v.chars().take(WIDTH).collect::<String>() + "…" } else { v.to_string() };
    let lines: Vec<&str> = output.lines().collect();
    let (mut out, mut depth, mut calls) = (String::new(), 0usize, 0usize);
    for (i, line) in lines.iter().enumerate() {
        // echo prints its location on the line before the value
        if lines.get(i + 1).is_some_and(|n| n.starts_with("#(\"solace-")) && line.contains(".gleam:") {
            continue;
        }
        let event = line
            .strip_prefix("#(\"solace-call\", \"")
            .map(|r| (true, r))
            .or(line.strip_prefix("#(\"solace-return\", \"").map(|r| (false, r)));
        let Some((call, rest)) = event else {
            out += &format!("{line}\n");
            continue;
        };
        let (name, rest) = rest.split_once('"').unwrap_or((rest, ""));
        let value = rest.strip_prefix(", ").unwrap_or(rest).strip_suffix(')').unwrap_or("");
        if call {
            calls += 1;
        }
        if calls <= MAX {
            let indent = "  ".repeat(if call { depth } else { depth.saturating_sub(1) });
            out += &if call { format!("{indent}→ {name}({})\n", cut(value)) } else { format!("{indent}← {name} = {}\n", cut(value)) };
        }
        depth = if call { depth + 1 } else { depth.saturating_sub(1) };
    }
    if calls > MAX {
        out += &format!("… {} more calls not shown\n", calls - MAX);
    }
    if calls == 0 {
        out += "(no traced function was called)\n";
    }
    out
}

/// The names a module header imports modules as.
fn aliases(header: &str) -> HashSet<String> {
    gleam::imports(header).into_iter().map(|i| i.alias).collect()
}

/// Single-line `assert a == b` in a test that read the same local variable on
/// both sides: the expected value is taken from the result it checks (D-43).
fn weak_asserts(text: &str, aliases: &HashSet<String>) -> Vec<String> {
    // root identifiers outside string literals: `q` in `q.total`, not `total`
    let roots = |code: &str| -> HashSet<String> {
        let (mut out, mut word, mut in_str, mut prev) = (HashSet::new(), String::new(), false, ' ');
        let mut after_dot = false;
        for c in code.chars().chain([' ']) {
            if in_str {
                in_str = !(c == '"' && prev != '\\');
            } else if c.is_alphanumeric() || c == '_' {
                if word.is_empty() {
                    after_dot = prev == '.';
                }
                word.push(c);
            } else {
                if word.starts_with(|w: char| w.is_lowercase()) && !after_dot {
                    out.insert(std::mem::take(&mut word));
                }
                word.clear();
                in_str = c == '"';
            }
            prev = c;
        }
        out
    };
    let mut locals = HashSet::new();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("let ") {
            locals.extend(roots(rest.split_once('=').map_or(rest, |p| p.0)).into_iter().filter(|w| w != "assert"));
        }
    }
    let mut out = vec![];
    for line in text.lines() {
        let Some((l, r)) = line.trim().strip_prefix("assert ").and_then(|a| a.split_once(" == ")) else { continue };
        let both: Vec<String> =
            roots(l).intersection(&roots(r)).filter(|w| locals.contains(*w) && !aliases.contains(*w)).cloned().collect();
        if let Some(v) = both.first() {
            out.push(format!(
                "`{}` reads `{v}` on both sides: an expected value taken from the result can't catch a wrong result; \
                 work it out from the task's numbers and the rules.",
                line.trim()
            ));
        }
    }
    out
}

/// `import` paths needed for `alias.name` references to modules the header
/// doesn't import yet: project modules by last path segment, or the stdlib.
/// `test`: the code is in a test module, which may also import test modules.
fn missing_imports(ix: &Index, header: &str, text: &str, test: bool) -> Vec<String> {
    let aliases = aliases(header);
    let mut out = vec![];
    for r in gleam::references(text) {
        let Ref::Qualified(alias, _) = r else {
            continue;
        };
        if aliases.contains(&alias) {
            continue;
        }
        let project: Vec<&ModuleRow> =
            ix.modules.iter().filter(|m| (m.dir == "src" || test) && m.path.rsplit('/').next() == Some(alias.as_str())).collect();
        let path = match project.as_slice() {
            [m] => Some(m.path.clone()),
            _ if STDLIB.contains(&alias.as_str()) => Some(format!("gleam/{alias}")),
            _ => None,
        };
        if let Some(p) = path
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// Each import in a header: (module, unqualified items as written, e.g.
/// `type Option` or `None`, and the line range it spans).
fn import_statements(header: &str) -> Vec<(String, Vec<String>, (usize, usize))> {
    let lines: Vec<&str> = header.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let Some(rest) = lines[i].strip_prefix("import ") else {
            i += 1;
            continue;
        };
        let start = i;
        while !lines[i].contains('}') && lines[start].contains(".{") && i + 1 < lines.len() {
            i += 1;
        }
        let stmt = lines[start..=i].join(" ");
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/').collect();
        let items = match (stmt.find(".{"), stmt.find('}')) {
            (Some(a), Some(b)) => {
                stmt[a + 2..b].split(',').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect()
            }
            _ => vec![],
        };
        out.push((module, items, (start, i)));
        i += 1;
    }
    out
}

/// Adds unqualified `items` to the header's import of `module`, or adds the
/// import. The result is formatted.
fn merge_import(header: &str, module: &str, items: &[String]) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let Some((_, existing, (start, end))) = import_statements(header).into_iter().find(|(m, _, _)| m == module) else {
        let line = if items.is_empty() { format!("import {module}") } else { format!("import {module}.{{{}}}", items.join(", ")) };
        let added = add_import(header, &line);
        return format_gleam(&added).map_or(added, |f| f.trim().to_string());
    };
    if items.iter().all(|i| existing.contains(i)) {
        return header.to_string();
    }
    let stmt = lines[start..=end].join(" ");
    let suffix = stmt.rfind('}').map_or_else(|| stmt[format!("import {module}").len()..].to_string(), |b| stmt[b + 1..].to_string());
    let mut all = existing;
    all.extend(items.iter().filter(|i| !all.contains(i)).cloned().collect::<Vec<_>>());
    let line = format!("import {module}.{{{}}}{}", all.join(", "), suffix.trim_end());
    let mut out: Vec<String> = lines[..start].iter().map(|s| s.to_string()).collect();
    out.push(line);
    out.extend(lines[end + 1..].iter().map(|s| s.to_string()));
    let joined = out.join("\n");
    format_gleam(&joined).map_or(joined, |f| f.trim().to_string())
}

/// From a compiler error about an unknown type or constructor, the import
/// that provides it: (module, item), when exactly one module does.
fn unimported_name(ix: &Index, error: &str) -> Option<(String, String)> {
    let between = |pre: &str, post: &str| {
        error.find(pre).and_then(|a| error[a + pre.len()..].find(post).map(|b| error[a + pre.len()..a + pre.len() + b].to_string()))
    };
    let (name, is_type) = match (between("The type `", "` is not defined"), between("constructor `", "` is not in scope")) {
        (Some(t), _) => (t, true),
        (None, Some(c)) => (c, false),
        _ => return None,
    };
    const STD: &[(&str, &str, bool)] = &[
        ("Option", "gleam/option", true),
        ("Some", "gleam/option", false),
        ("None", "gleam/option", false),
        ("Dict", "gleam/dict", true),
        ("Set", "gleam/set", true),
        ("Order", "gleam/order", true),
        ("Lt", "gleam/order", false),
        ("Eq", "gleam/order", false),
        ("Gt", "gleam/order", false),
    ];
    let item = if is_type { format!("type {name}") } else { name.clone() };
    if let Some((_, m, _)) = STD.iter().find(|(n, _, t)| *n == name && *t == is_type) {
        return Some((m.to_string(), item));
    }
    let mut found: Vec<&str> = ix
        .syms
        .iter()
        .filter(|s| s.kind == Kind::Type && gleam::head(&s.item()).contains("pub type"))
        .filter(|s| if is_type { s.name == name } else { gleam::constructors(&s.text).contains(&name) })
        .map(|s| s.module.as_str())
        .collect();
    unique(&mut found);
    match found.as_slice() {
        [m] => Some((m.to_string(), item)),
        _ => None,
    }
}

/// Adds an import line after the existing imports (or after the module doc).
fn add_import(header: &str, line: &str) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let at = lines.iter().rposition(|l| l.starts_with("import ") || l.starts_with("////")).map_or(0, |i| i + 1);
    // a multi-line import ends at its closing brace
    let at = if at > 0 && lines[at - 1].ends_with('{') {
        lines[at..].iter().position(|l| l.starts_with('}')).map_or(at, |p| at + p + 1)
    } else {
        at
    };
    let mut out: Vec<&str> = lines[..at].to_vec();
    if at > 0 && lines[at - 1].starts_with("////") {
        out.push("");
    }
    out.push(line);
    out.extend_from_slice(&lines[at..]);
    out.join("\n").trim().to_string()
}

// --- compiler output ---------------------------------------------------------------

struct FileMap {
    module: String,
    file: PathBuf,
    /// (first line, last line, qualified name)
    ranges: Vec<(usize, usize, String)>,
}

struct Diag {
    error: bool,
    module: Option<String>,
    symbol: Option<String>,
    text: String,
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Where a file position falls: (module, symbol and its first line).
fn locate<'a>(files: &'a [FileMap], path: &str, line: usize) -> Option<(&'a str, Option<(&'a str, usize)>)> {
    let f = files.iter().find(|f| f.file.to_string_lossy() == path || f.file.ends_with(path))?;
    let sym = f.ranges.iter().find(|r| r.0 <= line && line <= r.1).map(|r| (r.2.as_str(), r.0));
    Some((&f.module, sym))
}

/// Rewrites `path/to/file.gleam:LINE` references in a line to symbol positions.
fn relocate(line: &str, files: &[FileMap]) -> String {
    let Some(end) = line.find(".gleam:") else {
        return line.to_string();
    };
    let start = line[..end].rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
    let path = &line[start..end + 6];
    let rest = &line[end + 7..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let (Ok(n), Some((module, sym))) = (digits.parse::<usize>(), locate(files, path, digits.parse().unwrap_or(0))) else {
        return line.to_string();
    };
    let tail = &rest[digits.len()..];
    let tail = tail.strip_prefix(':').map_or(tail, |t| t.trim_start_matches(|c: char| c.is_ascii_digit()));
    let place = match sym {
        Some((q, first)) => format!("{q}, line {}", n - first + 1),
        None => format!("{module} imports"),
    };
    format!("{}{place}{tail}", &line[..start])
}

/// Splits compiler output into diagnostics located by symbol.
fn diagnostics(output: &str, files: &[FileMap]) -> Vec<Diag> {
    let mut blocks: Vec<Vec<&str>> = vec![];
    for line in output.lines() {
        let t = line.trim_start();
        if line.starts_with("error") || line.starts_with("warning") {
            blocks.push(vec![line]);
        } else if ["Compiling ", "Compiled ", "Checking ", "Running "].iter().any(|p| t.starts_with(p)) {
            continue;
        } else if let Some(b) = blocks.last_mut() {
            b.push(line);
        }
    }
    blocks
        .into_iter()
        .map(|b| {
            let error = b[0].starts_with("error");
            let mut module = None;
            let mut symbol = None;
            let mut offset = 0;
            let mut text = vec![];
            for line in &b {
                if let Some(loc) = line.trim_start().strip_prefix("┌─ ") {
                    let mut parts = loc.rsplitn(3, ':');
                    let (_col, row, path) = (parts.next(), parts.next(), parts.next());
                    if let (Some(path), Some(Ok(row))) = (path, row.map(|r| r.parse::<usize>()))
                        && let Some((m, sym)) = locate(files, path, row)
                    {
                        module = Some(m.to_string());
                        match sym {
                            Some((q, first)) => {
                                symbol = Some(q.to_string());
                                offset = first - 1;
                                text.push(format!("  at {q}, line {}", row - offset));
                            }
                            None => text.push(format!("  at {m} imports")),
                        }
                        continue;
                    }
                }
                // renumber the code excerpt to lines within the symbol
                let t = line.trim_start();
                let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                if offset > 0
                    && !digits.is_empty()
                    && t[digits.len()..].starts_with(" │")
                    && let Ok(n) = digits.parse::<usize>()
                    && n > offset
                {
                    text.push(format!("{:>4}{}", n - offset, &t[digits.len()..]));
                    continue;
                }
                text.push(line.to_string());
            }
            Diag { error, module, symbol, text: text.join("\n").trim_end().to_string() }
        })
        .collect()
}

fn warning_summary<'a>(diags: impl Iterator<Item = &'a Diag>) -> String {
    let lines: Vec<String> = diags
        .map(|d| {
            format!(
                "  {}: {}",
                d.symbol.as_deref().or(d.module.as_deref()).unwrap_or("project"),
                d.text.lines().find(|l| l.trim_start().starts_with("warning:")).unwrap_or("compiler warning").trim()
            )
        })
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("\nWarnings:\n{}\nUse `check --warnings` for full diagnostics.\n", lines.join("\n"))
    }
}

/// What a change did to the broken set (D-46): `before` is the broken table
/// as it was. Fixed symbols are named; a symbol broken with the same errors as
/// before gets one line; new errors are printed in full, once per message, and
/// the other errors with that message as one line each (Spec 3 §6).
/// What a name is unique by: types and values are separate namespaces in Gleam.
fn name_key(name: &str, kind: Kind) -> (String, bool) {
    (name.to_string(), kind == Kind::Type)
}

/// Holds the lock on the file at `path`, created if missing; waits for it.
fn lock_file(path: &Path) -> Result<fs::File, Failure> {
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    f.lock()?;
    Ok(f)
}

/// The lock on the file at `path`, or `None` while another holds it.
fn try_lock_file(path: &Path) -> Result<Option<fs::File>, Failure> {
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    Ok(f.try_lock().is_ok().then_some(f))
}

/// Bytes stored in JSON as an array of numbers.
fn json_bytes(a: &[Value]) -> Vec<u8> {
    a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect()
}

/// A search query without the quotes, backticks or apostrophes around it
/// (`find "Customer(.."`: the quotes delimit the query, run 18).
fn unquoted(query: &str) -> &str {
    ['"', '\'', '`'].iter().find_map(|&c| query.strip_prefix(c)?.strip_suffix(c)).filter(|q| !q.is_empty()).unwrap_or(query)
}

/// Writes a file whole or not at all: a reader never sees half of it.
fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), Failure> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `n` and the noun, plural unless `n` is 1: "1 file", "2 files".
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// A project file's name as Solace lists it: `/`-separated, without `.` parts.
fn file_name(path: &Path) -> String {
    path.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Symbols as they were before a change: (id, text then; `None` if new).
type Before = Vec<(String, Option<String>)>;

/// Notes for a reply, a line each.
fn note_lines(notes: &[String]) -> String {
    notes.iter().map(|n| format!("Note: {n}\n")).collect()
}

/// Drops repeats anywhere in `v`, keeping the first of each, in order.
fn unique<T: PartialEq>(v: &mut Vec<T>) {
    let mut out = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    *v = out;
}

fn broken_report(before: &HashMap<String, String>, diags: &[Diag]) -> String {
    let errors: Vec<&Diag> = diags.iter().filter(|d| d.error).collect();
    let mut names: Vec<String> =
        errors.iter().map(|d| d.symbol.clone().or(d.module.clone().map(|m| m + " imports")).unwrap_or("?".into())).collect();
    unique(&mut names);
    let mut fixed: Vec<&str> = before.keys().map(String::as_str).filter(|k| !names.iter().any(|n| n == k)).collect();
    fixed.sort();
    let fixed = if fixed.is_empty() { String::new() } else { format!("Fixed: {}.\n", fixed.join(", ")) };
    if errors.is_empty() {
        return format!("{fixed}The project type-checks.\n");
    }
    let mut out = format!("\nBroken symbols ({}): {}\n{fixed}", names.len(), names.join(", "));
    let own =
        |sym: &str| errors.iter().filter(|d| d.symbol.as_deref() == Some(sym)).map(|d| d.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let same: Vec<&String> = names.iter().filter(|n| before.get(*n).is_some_and(|t| *t == own(n))).collect();
    if !same.is_empty() {
        out += "Still broken, with the errors shown before (`check` shows them again):\n";
        for n in &same {
            let all: Vec<&&Diag> = errors.iter().filter(|d| d.symbol.as_ref() == Some(n)).collect();
            let more = if all.len() > 1 { format!(" (and {} more errors in it)", all.len() - 1) } else { String::new() };
            out += &format!("  {}: {}{more}\n", place(all[0]), error_message(all[0]));
        }
    }
    // new errors, grouped by message: (message, first in full, the rest)
    let mut groups: Vec<(String, &Diag, Vec<&Diag>)> = vec![];
    for d in errors.iter().filter(|d| !d.symbol.as_ref().is_some_and(|s| same.contains(&s))) {
        let m = error_message(d);
        match groups.iter_mut().find(|g| g.0 == m) {
            Some(g) => g.2.push(d),
            None => groups.push((m, d, vec![])),
        }
    }
    for (_, first, rest) in groups.iter().take(5) {
        out += &format!("\n{}\n", first.text);
        if !rest.is_empty() {
            out += &format!("same error: {} more\n", rest.len());
            for d in rest {
                out += &format!("  at {}: {}\n", place(d), error_line(d));
            }
        }
    }
    let more: usize = groups.iter().skip(5).map(|g| 1 + g.2.len()).sum();
    if more > 0 {
        out += &format!("\n… {more} more errors (`check`)\n");
    }
    out
}

/// `<symbol>, line N` from a diagnostic's `at` line.
fn place(d: &Diag) -> String {
    d.text.lines().find_map(|l| l.trim_start().strip_prefix("at ")).unwrap_or("?").to_string()
}

/// An error's title and the compiler's note at its caret, e.g. "Incorrect
/// arity: Expected 6 arguments, got 5".
fn error_message(d: &Diag) -> String {
    let title = d.text.lines().next().unwrap_or("").trim_start_matches("error:").trim();
    let note = d.text.lines().rev().find_map(|l| l.rsplit_once('^')).map_or("", |(_, n)| n.trim());
    if note.is_empty() { title.to_string() } else { format!("{title}: {note}") }
}

/// The source line an error is at, from its excerpt.
fn error_line(d: &Diag) -> String {
    let n = place(d).rsplit_once("line ").map_or(String::new(), |(_, n)| n.to_string());
    d.text
        .lines()
        .find_map(|l| l.trim_start().strip_prefix(n.as_str()).and_then(|r| r.strip_prefix(" │")))
        .map_or(String::new(), |r| r.trim_start_matches(|c: char| " │╭╰─·".contains(c)).trim_end().to_string())
}

/// An index built from (dir, module, source) triples, for tests.
#[cfg(test)]
fn test_index(sources: &[(&str, &str, &str)]) -> Index {
    let (mut syms, mut modules) = (vec![], vec![]);
    for (dir, path, text) in sources {
        let m = gleam::split(text);
        modules.push(ModuleRow { path: (*path).into(), dir: (*dir).into(), header: m.header });
        for item in m.items {
            syms.push(Sym {
                id: format!("fn:{path}.{}", item.name),
                module: (*path).into(),
                name: item.name,
                kind: item.kind,
                text: item.text,
            });
        }
    }
    Index::build(syms, modules)
}

/// Fixtures for every module's tests.
#[cfg(test)]
mod testing {
    use super::*;

    /// A folder for one test, deleted when the test ends, passed or failed.
    pub(crate) struct Scratch(pub PathBuf);
    impl Scratch {
        pub(crate) fn new() -> Scratch {
            let dir = env::temp_dir().join(format!("solace-test-{}", unique_id()));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        /// Writes `files` (path, text) into the folder.
        pub(crate) fn with(self, files: &[(&str, &str)]) -> Scratch {
            for (path, text) in files {
                let p = self.0.join(path);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, text).unwrap();
            }
            self
        }
        /// Gleam's packages and compiled dependencies, so `gleam test` runs
        /// without downloading anything: the bench pilot's.
        pub(crate) fn with_packages(self) -> Scratch {
            let pilot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
            for f in ["manifest.toml", "test/pilot_test.gleam"] {
                fs::create_dir_all(self.0.join(f).parent().unwrap()).unwrap();
                fs::copy(pilot.join(f), self.0.join(f)).unwrap();
            }
            let name = fs::read_to_string(self.0.join("gleam.toml")).map(|_| workspace::name(&self.0)).unwrap_or_else(|_| "app".into());
            fs::write(self.0.join("gleam.toml"), fs::read_to_string(pilot.join("gleam.toml")).unwrap().replace("pilot_large", &name))
                .unwrap();
            migrate::copy_tree(&pilot.join("build"), &self.0.join("build"), &|_| false).unwrap();
            self
        }
        /// The folder's Gleam project, read into a new store.
        pub(crate) fn import(self) -> Project {
            if !self.0.join("gleam.toml").exists() {
                fs::write(self.0.join("gleam.toml"), "name = \"app\"\nversion = \"1.0.0\"\n").unwrap();
            }
            import(&self.0).unwrap();
            Project { store: Store::at(&self.0).unwrap(), _dir: self }
        }
    }
    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A store and its folder, deleted together.
    pub(crate) struct Project {
        pub(crate) store: Store,
        _dir: Scratch,
    }
    impl std::ops::Deref for Project {
        type Target = Store;
        fn deref(&self) -> &Store {
            &self.store
        }
    }

    /// A Gleam project of `files` (path, text), named `app`, in a store.
    pub(crate) fn project(files: &[(&str, &str)]) -> Project {
        Scratch::new().with(files).import()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    #[test]
    fn broken_symbols_counted_once() {
        let d = |sym: &str| Diag { error: true, module: None, symbol: Some(sym.into()), text: format!("error: {sym}") };
        let out = broken_report(&HashMap::new(), &[d("a.f"), d("a.g"), d("a.f")]);
        assert!(out.contains("Broken symbols (2): a.f, a.g"), "{out}");
    }
    #[test]
    fn agent_reference_leaves_out_host_and_task_commands() {
        // Tests run outside a board run; `help` keeps everything.
        let r = agent_reference();
        assert!(r.contains("Several commands go in one call") && r.contains("work records"));
        assert!(!r.contains("task done") && !r.contains("Host commands"));
        assert!(REFERENCE.contains("task done") && REFERENCE.contains("agent-config"));
        // D-46: session start gets the workflow, with the signature-change and
        // test examples (D-48: a test at the level reported, one per shifted
        // behavior), in under 3k bytes; `help` keeps every option.
        assert!(r.len() < 3000, "{}", r.len());
        assert!(
            r.contains("Changing a signature or adding a field") && r.contains("one at the level the task reports") && r.contains("`help`")
        );
        assert!(!r.contains("probe --from <fn>") && REFERENCE.contains("probe --from <fn>") && REFERENCE.contains("--replace [--all]"));
        assert!(r.contains("Imports are added") && !r.contains("task ask"));
        // `help` for agents: every option, but no host or task commands here.
        let h = agent_help(None);
        assert!(h.contains("probe --from <fn>") && !h.contains("task diff") && !h.contains("agent-config"));
        let s = agent_help(Some("submit"));
        assert!(s.contains("--replace [--all]") && s.contains("draft") && !s.contains("probe --from"), "{s}");
        assert!(agent_help(Some("work")).contains("work done") && agent_help(Some("nope")).starts_with("No command"));
    }

    #[test]
    fn probe_args_parse() {
        assert_eq!(probe_args(&["--from", "t", "--trace", "a.b", "c"]).unwrap(), (Some("t"), vec!["a.b", "c"]));
        assert_eq!(probe_args(&[]).unwrap(), (None, vec![]));
        assert!(probe_args(&["--trace"]).is_err() && probe_args(&["--from"]).is_err() && probe_args(&["x"]).is_err());
    }

    #[test]
    fn dependencies_show_their_public_api() {
        let store = project(&[
            ("src/app.gleam", "pub fn main() { 1 }\n"),
            (
                "build/packages/dep/src/dep/api.gleam",
                "//// Talks to things.\n////\n//// A long guide.\n\nimport gleam/list\n\n\
             /// Makes a widget.\n/// Second doc line.\npub fn make(size: Int) -> Widget {\n  secret(size)\n}\n\n\
             // SECTION ---\n\nfn secret(n: Int) -> Widget {\n  Widget(n)\n}\n\n\
             /// A widget.\npub opaque type Widget {\n  Widget(size: Int)\n}\n",
            ),
        ]);
        // A module: its first doc paragraph and public declarations, no bodies.
        let out = store.view(&["dep/api"], false, usize::MAX).ok().unwrap();
        assert!(
            out.starts_with("// ── module dep/api (dep, a dependency; read only)") && out.contains("//// Talks to things.\n\n"),
            "{out}"
        );
        assert!(
            out.contains("/// Makes a widget.\npub fn make(size: Int) -> Widget\n") && out.contains("pub opaque type Widget\n"),
            "{out}"
        );
        for hidden in ["secret", "A long guide", "SECTION", "Widget(size", "Second doc line"] {
            assert!(!out.contains(hidden), "{hidden}: {out}");
        }
        // A symbol: all of its doc; `inspect` falls back to the same view; store names still win.
        let one = store.inspect(&["dep/api.make", "app.main"], true, usize::MAX).ok().unwrap();
        assert!(one.contains("/// Second doc line.\npub fn make(size: Int) -> Widget") && !one.contains("secret(size)"), "{one}");
        assert!(one.contains("app.main"), "{one}");
        assert!(store.view(&["dep/api.secret"], false, usize::MAX).unwrap_err().0.contains("E1001"));
    }

    /// D-14: a multi-name inspect prints each signature once, and trims, then
    /// defers names, to stay within the budget.
    #[test]
    fn inspect_dedups_and_fits_budget() {
        // A real project of some size: the bench pilot's code.
        let dir = Scratch::new();
        let pilot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
        for top in ["src", "test"] {
            migrate::copy_tree(&pilot.join(top), &dir.join(top), &|_| false).unwrap();
        }
        fs::copy(pilot.join("gleam.toml"), dir.join("gleam.toml")).unwrap();
        let store = dir.import();
        let names = ["store.put_shipment", "shipments.dispatch", "shipments.deliver", "fulfillment.ship"];
        let ok = |r: Out| r.ok().unwrap();

        let out = ok(store.inspect(&names, true, usize::MAX));
        assert_eq!(out.matches("-- pilot/shipments.Shipment\n").count(), 1);
        assert!(out.contains("-- pilot/shipments.Shipment (above)"));
        assert!(!out.contains("Trimmed") && !out.contains("Not shown"));

        let small = ok(store.inspect(&names, true, 3_000));
        assert!(small.contains("Trimmed to fit the size limit"));
        assert!(small.contains("Not shown, to fit the size limit: `solace inspect "));
        assert!(small.len() < out.len());
        for n in names {
            assert!(
                small.contains(&format!("// ── pilot/{n} ")) || small.lines().last().unwrap().contains(&format!("pilot/{n}")),
                "{n} lost"
            );
        }

        // an unknown name is reported after the others, not instead of them
        let mixed = ok(store.inspect(&["shipments.deliver", "shipping.standard_cost_cents"], false, usize::MAX));
        assert!(mixed.contains("// ── pilot/shipments.deliver ") && mixed.contains("E1001 unknown name: shipping.standard_cost_cents"));
        assert!(store.inspect(&["shipping.nope"], false, usize::MAX).is_err());
        // D-46: a type's users split into the sites that list every field and
        // the rest by name; a function's callers show the call line.
        let ty = ok(store.inspect(&["customer.Customer"], true, usize::MAX));
        assert!(ty.contains("== Constructed by (") && ty.contains("-- pilot/seed.customers, line 3: Customer("), "{ty}");
        assert!(ty.contains("-- pilot/accounts.register, line 11: customer.Customer(\n"), "{ty}");
        assert!(ty.contains("== Other users (") && !ty.contains("== Used by"), "{ty}");
        // commands in one MCP call share what was printed: `refs` says (above)
        SHOWN.with(|s| *s.borrow_mut() = Some(HashSet::new()));
        let f = ok(store.inspect(&["pricing.quote"], true, usize::MAX));
        assert!(f.contains("-- pilot/checkout.quote\n") && f.contains("  > line "), "{f}");
        let r = ok(store.refs("pricing.quote"));
        assert!(r.contains("-- pilot/checkout.quote (above)\n  > line "), "{r}");
        SHOWN.with(|s| *s.borrow_mut() = None);
        assert!(ok(store.refs("pricing.quote")).contains("-- pilot/checkout.quote\n"));
    }

    #[test]
    fn code_is_told_from_commands() {
        assert!(looks_like_code("let assert Ok(q) = checkout.quote(db, o.id, Standard)") && looks_like_code("echo q"));
        assert!(!looks_like_code("probe --from checkout_test.quote_test --trace pricing.quote"));
    }

    #[test]
    fn import_creates_settings_without_overwriting() {
        for existing in [None, Some("# custom\nresources = [\"docs/**\"]\n")] {
            let root =
                Scratch::new().with(&[("gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"), ("src/app.gleam", "pub fn main() { 1 }\n")]);
            if let Some(text) = existing {
                fs::write(root.join("solace.toml"), text).unwrap();
            }
            let out = import(&root).unwrap();
            let text = fs::read_to_string(root.join("solace.toml")).unwrap();
            if let Some(existing) = existing {
                assert_eq!(text, existing);
                assert!(!out.contains("Created solace.toml"));
            } else {
                assert!(out.contains("Created solace.toml"));
                assert_eq!(lockdown::settings(&root).unwrap()["watch_limit"].as_integer(), Some(262144));
                assert!(lockdown::settings(&root).unwrap()["commands"].as_table().unwrap().is_empty());
            }
            let store = fs::read(root.join(".solace/store.db")).unwrap();
            assert!(import(&root).unwrap().contains("existing symbols were preserved"));
            assert_eq!(fs::read_to_string(root.join("solace.toml")).unwrap(), text);
            fs::remove_file(root.join("solace.toml")).unwrap();
            fs::write(root.join("src/app.gleam"), "pub fn changed_on_disk() { 2 }\n").unwrap();
            assert!(import(&root).unwrap().contains("Created solace.toml"));
            assert!(lockdown::settings(&root).is_ok());
            assert_eq!(fs::read(root.join(".solace/store.db")).unwrap(), store);
        }
    }

    #[test]
    fn submit_new_creates_a_module() {
        let store = project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        // Code with errors creates nothing.
        assert!(store.submit_symbols(None, Some("calc"), "pub fn add(a: Int) -> Int { a <> 1 }").is_err());
        assert!(store.resolve_module(&store.index().unwrap(), "calc").is_none());
        let out = store.submit_symbols(None, Some("calc"), "pub fn add(a: Int, b: Int) -> Int { a + b }").unwrap();
        assert!(out.contains("created module calc (src/calc.gleam)"), "{out}");
        assert!(store.resolve_module(&store.index().unwrap(), "calc").is_some());
        let out = store.submit_symbols(None, Some("calc_test"), "pub fn add_test() { assert calc.add(1, 2) == 3 }").unwrap();
        assert!(out.contains("(test/calc_test.gleam)"), "{out}");
        assert!(store.submit_symbols(None, Some("Bad"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E1003"));
        // A whole new module in one submit: module comment, imports and several symbols.
        let module = "//// Geometry helpers.\nimport calc\n\npub const unit = 1\n\npub type Shape {\n  Square(Int)\n}\n\npub fn area(s: Shape) -> Int {\n  let Square(n) = s\n  calc.add(n * n, unit)\n}\n";
        let out = store.submit_symbols(None, Some("geo"), module).unwrap();
        assert!(out.contains("created module geo") && out.contains("geo.unit, geo.Shape, geo.area"), "{out}");
        let file = fs::read_to_string(store.root.join("src/geo.gleam")).unwrap();
        assert!(file.starts_with("//// Geometry helpers.\n") && file.contains("import calc\n"), "{file}");
        assert!(store.submit_symbols(None, Some("geo2"), "pub fn a() { 1 }\npub fn a() { 2 }").unwrap_err().0.starts_with("E0103"));
        // Blocks may name a module that doesn't exist yet; a named block still holds one symbol.
        let out = store
            .submit_symbols(None, None, "// ── shapes.side ──\npub fn side() { 2 }\n// ── shapes.twice ──\npub fn twice() { side() * 2 }\n")
            .unwrap();
        assert!(out.contains("created module shapes"), "{out}");
        assert!(
            store
                .submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int { a + b }\npub fn sub() { 1 }")
                .unwrap_err()
                .0
                .starts_with("E0101")
        );
    }

    /// `calc.add` and `scale`, a test helper, and a test with a local.
    fn probe_project() -> Project {
        project(&[
            (
                "src/calc.gleam",
                "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn scale(x: Int, by factor: Int) -> Int {\n  factor * x\n}\n",
            ),
            ("test/helper.gleam", "pub fn two() {\n  2\n}\n"),
            ("test/calc_test.gleam", "import calc\n\npub fn sum_test() {\n  let x = calc.add(1, 2)\n  assert x == 3\n}\n"),
        ])
    }

    #[test]
    fn probe_runs_and_leaves_nothing() {
        let store = probe_project();
        let probe_file = store.root.join("test/solace_probe.gleam");
        let out = store.probe("echo calc.add(2, 3)", None, &[]).unwrap();
        assert!(out.contains("5"), "{out}");
        assert!(!probe_file.exists());
        let err = store.probe("echo calc.nope()", None, &[]).unwrap_err().0;
        assert!(err.contains("The probe ran as") && err.contains("import calc"), "{err}");
        assert!(!probe_file.exists());
        // A probe is test-side code: it may use test helpers.
        assert!(store.probe("echo helper.two()", None, &[]).unwrap().contains("2"));
        // `--from` starts from a function's body; a single expression is printed (D-43).
        let out = store.probe("echo x + 10", Some("calc_test.sum_test"), &[]).unwrap();
        assert!(out.contains("13"), "{out}");
        assert!(store.probe("", Some("calc.add"), &[]).unwrap_err().0.starts_with("E1005"));
        let out = store.probe("calc.add(20, 3)", None, &[]).unwrap();
        assert!(out.contains("23") && !out.contains("warning"), "{out}");
    }

    #[test]
    fn probe_trace_shows_calls_and_puts_the_module_back() {
        let store = probe_project();
        let before = fs::read_to_string(store.root.join("src/calc.gleam")).unwrap();
        let out = store.probe("calc.add(2, 3)", None, &["calc.add"]).unwrap();
        assert!(out.contains("→ calc.add(2, 3)") && out.contains("← calc.add = 5"), "{out}");
        assert_eq!(fs::read_to_string(store.root.join("src/calc.gleam")).unwrap(), before);
        let out = store.probe("calc.scale(3, by: 2)", None, &["calc.scale"]).unwrap();
        assert!(out.contains("← calc.scale = 6"), "{out}");
        assert!(store.probe("1", None, &["calc_test"]).is_err());
    }

    #[test]
    fn test_helpers_are_imported_only_by_test_code() {
        let store = probe_project();
        let out = store.submit_symbols(None, Some("calc_test"), "pub fn two_test() { assert helper.two() == 2 }").unwrap();
        assert!(out.contains("import helper"), "{out}");
        assert!(store.submit_symbols(None, Some("calc"), "pub fn four() { helper.two() * 2 }").is_err());
    }

    /// D-43: an assert whose expected side reads the value it checks gets a note.
    #[test]
    fn weak_asserts_get_a_note() {
        let store = probe_project();
        let out = store
            .submit_symbols(
                None,
                Some("calc_test"),
                "pub fn weak_test() {\n  let q = #(1, 2)\n  assert q.0 == q.1 - 1\n  assert q.1 == 2\n}",
            )
            .unwrap();
        assert!(out.contains("`assert q.0 == q.1 - 1` reads `q` on both sides") && !out.contains("assert q.1 == 2`"), "{out}");
    }

    /// A small project: `calc.add` and three callers.
    fn calc_project() -> Project {
        let callers =
            "pub fn one() -> Int {\n  add(0, 1)\n}\n\npub fn two() -> Int {\n  add(1, 1)\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n";
        project(&[("src/calc.gleam", &format!("pub fn add(a: Int, b: Int) -> Int {{\n  a + b\n}}\n\n{callers}"))])
    }

    /// D-46: a submit reports what changed in the broken set; an error seen
    /// before is one line, and errors with one message are printed in full once.
    #[test]
    fn signature_change_reports_changes_once() {
        let store = calc_project();
        let out = store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int, c: Int) -> Int { a + b + c }").unwrap();
        assert!(out.contains("Broken symbols (3)") && out.contains("same error: 2 more"), "{out}");
        assert_eq!(out.matches("Expected 3 arguments, got 2").count(), 1, "{out}");
        assert!(out.contains("  at calc.three, line 2: add(1, 2)"), "{out}");
        let out = store.submit_symbols(Some("calc.one"), None, "pub fn one() -> Int { add(0, 1, 0) }").unwrap();
        assert!(out.contains("Fixed: calc.one.") && out.contains("Still broken, with the errors shown before"), "{out}");
        assert!(out.contains("  calc.two, line 2: Incorrect arity: Expected 3 arguments, got 2\n"), "{out}");
        assert!(!out.contains("│"), "{out}");
        // `check` stays complete.
        let full = store.check().unwrap_err().0;
        assert_eq!(full.matches("Expected 3 arguments, got 2").count(), 2, "{full}");
        let out = store
            .submit_symbols(
                None,
                None,
                "// ── calc.two ──\npub fn two() -> Int { add(1, 1, 0) }\n// ── calc.three ──\npub fn three() -> Int { add(1, 2, 0) }\n",
            )
            .unwrap();
        assert!(out.contains("Fixed: calc.three, calc.two.\nThe project type-checks."), "{out}");
    }

    /// D-46: text equal to what is stored is reported unchanged, and when
    /// nothing changed the type check is skipped.
    #[test]
    fn unchanged_submit_skips_check() {
        let store = calc_project();
        let blocks = "// ── calc.one ──\npub fn one() -> Int { add(0, 2) }\n// ── calc.two ──\npub fn two() -> Int { add(1, 1) }\n";
        let out = store.submit_symbols(Some("calc.one"), None, blocks).unwrap();
        assert!(
            out.contains("plain `submit` does the same") && out.contains("Saved calc.one.\nUnchanged (the same as stored): calc.two."),
            "{out}"
        );
        // A source change on disk would show if the project were checked again.
        fs::write(store.root.join("src/calc.gleam"), "not gleam").unwrap();
        let out = store.submit_symbols(Some("calc.two"), None, blocks).unwrap();
        assert!(out.contains("Unchanged: calc.one, calc.two.") && out.contains("nothing was saved or checked"), "{out}");
        assert_eq!(fs::read_to_string(store.root.join("src/calc.gleam")).unwrap(), "not gleam");
    }

    /// D-44: `test` runs the tests whose reach changed since they passed, those
    /// that failed, and those reaching foreign code; the rest are skipped.
    #[test]
    fn test_runs_what_reaches_a_change() {
        let store = Scratch::new()
            .with(&[
                (
                    "src/calc.gleam",
                    "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn double(a: Int) -> Int {\n  add(a, a)\n}\n\npub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n\n@external(erlang, \"erlang\", \"abs\")\npub fn abs(x: Int) -> Int\n",
                ),
                (
                    "test/calc_test.gleam",
                    "import calc\n\npub fn add_test() {\n  assert calc.add(1, 2) == 3\n}\n\npub fn double_test() {\n  assert calc.double(2) == 4\n}\n\npub fn sub_test() {\n  assert calc.sub(3, 1) == 3\n}\n\npub fn abs_test() {\n  assert calc.abs(-2) == 2\n}\n",
                ),
            ])
            .with_packages()
            .import();
        let first = |out: Out| out.unwrap_or_else(|e| e.0).lines().next().unwrap_or("").to_string();
        // First run: everything; sub_test fails.
        let err = store.test(&[]).unwrap_err().0;
        assert!(err.starts_with("Ran all 4 tests.") && err.contains("sub_test"), "{err}");
        // The failing test runs again, as does the one reaching foreign code.
        store.submit_symbols(Some("calc_test.sub_test"), None, "pub fn sub_test() {\n  assert calc.sub(3, 1) == 2\n}").unwrap();
        assert_eq!(
            first(store.test(&[])),
            "Ran 2 of 4 tests; the other 2 reach nothing changed since they passed (`test --all` runs every test)."
        );
        // A change reaches only the tests that can see it, and the files match the store afterwards.
        store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int {\n  b + a\n}").unwrap();
        let out = store.test(&[]).unwrap();
        assert!(out.starts_with("Ran 3 of 4 tests;") && out.contains("3 passed"), "{out}");
        let file = fs::read_to_string(store.root.join("test/calc_test.gleam")).unwrap();
        assert_eq!(file.matches("pub fn ").count(), 4, "{file}");
        assert_eq!(first(store.test(&["--all"])), "Ran all 4 tests.");
        assert_eq!(first(store.test(&["calc.double"])), "Ran 1 of 4 tests: those that reach calc.double.");
        assert!(store.test(&["calc.nope"]).unwrap_err().0.starts_with("E1001"));
        // With nothing changed, only the test reaching foreign code runs.
        assert!(first(store.test(&[])).starts_with("Ran 1 of 4 tests;"));
    }

    /// D-44: the packet quotes code that names the task's terms, and names a
    /// test that reaches the top symbols to reproduce the case with.
    #[test]
    fn packet_quotes_evidence_and_reproduction() {
        let store = project(&[
            (
                "src/shop.gleam",
                "pub type Customer {\n  Customer(name: String, tier: Int)\n}\n\npub const customers = [Customer(\"Grace\", 2), Customer(\"Ada\", 1)]\n\n/// The price after the tier discount.\npub fn price(c: Customer, cents: Int) -> Int {\n  cents - c.tier * 100\n}\n",
            ),
            (
                "test/shop_test.gleam",
                "import shop\n\npub fn price_test() {\n  let assert [grace, ..] = shop.customers\n  assert shop.price(grace, 1000) == 800\n}\n",
            ),
        ]);
        let root = &store.root;
        let out = store.session_task("Grace (tier 2) was charged the wrong price.").unwrap();
        assert!(
            out.contains(
                "-- shop.customers\npub const customers = [Customer(\"Grace\", 2), Customer(\"Ada\", 1)]\n  > Customer(\"Grace\", 2)\n"
            ),
            "{out}"
        );
        assert!(out.contains("To reproduce the task's case: `probe --from shop_test.price_test --trace shop.price`"), "{out}");
        assert!(!out.contains("Project instructions"));
        // The project's AGENTS.md comes first in both orientations, cut at a whole line (D-45).
        fs::write(root.join("AGENTS.md"), "Prices are in cents.\n").unwrap();
        for out in [store.session_task("Grace").unwrap(), store.session(&[]).unwrap()] {
            assert!(out.starts_with("Project instructions (AGENTS.md") && out.contains("):\nPrices are in cents.\n\n"), "{out}");
        }
        fs::write(root.join("AGENTS.md"), "a line of forty bytes, give or take ok.\n".repeat(250)).unwrap();
        let out = store.session(&[]).unwrap();
        assert!(out.contains("ok.\n… `read AGENTS.md --lines 201..` for the rest\n"), "{out}");
    }

    /// A whole module replaces every symbol it holds, and may not drop one;
    /// `--imports` replaces only the header.
    #[test]
    fn whole_module_and_imports_submit() {
        let store = calc_project();
        let file = store.root.join("src/calc.gleam");
        let module = fs::read_to_string(&file).unwrap().replace("a + b", "b + a");
        let out = store.submit(Target::Module("calc"), &module).unwrap();
        assert_eq!(out, "Note: calc: changed add; added none\nSaved calc.\nThe project type-checks.\n");
        assert!(fs::read_to_string(&file).unwrap().contains("b + a"));
        let without_three = module.split("pub fn three").next().unwrap();
        assert!(store.submit(Target::Module("calc"), without_three).unwrap_err().0.contains("E0102 the module text leaves out: three"));
        assert!(store.submit(Target::Module("calc"), &format!("{module}\npub fn one() {{ 1 }}")).unwrap_err().0.starts_with("E0103"));
        // A rejected module leaves the stored code, and its file, as they were.
        let before = fs::read_to_string(&file).unwrap();
        let err = store.submit(Target::Module("calc"), &module.replace("b + a", "b +. a")).unwrap_err().0;
        assert!(err.starts_with("Not saved: the submitted text has errors."), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
        store.submit_symbols(None, Some("other"), "pub fn x() { 1 }").unwrap();
        let out = store.submit(Target::Imports("calc"), "//// Arithmetic.\nimport other").unwrap();
        assert!(out.starts_with("Saved calc imports."), "{out}");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("//// Arithmetic.\n\nimport other\n"), "{text}");
        assert!(store.submit(Target::Imports("calc"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E0101"));
        assert!(store.submit(Target::Imports("nope"), "").unwrap_err().0.starts_with("E1003"));
    }

    /// D-44: `impact` follows a symbol's data to what uses it.
    #[test]
    fn impact_names_what_a_symbol_reaches() {
        let store = calc_project();
        let out = store.impact(&["calc.add"], BUDGET).unwrap();
        assert!(out.starts_with("calc.add reaches 3 uses, nearest first:"), "{out}");
        assert!(out.contains("calc.one") && out.contains("calc.two") && out.contains("calc.three"), "{out}");
        assert!(store.impact(&[], BUDGET).unwrap().starts_with("Tests haven't passed in full yet"));
        assert!(store.impact(&["calc.nope"], BUDGET).unwrap_err().0.starts_with("E1001"));
    }

    /// D-12: work records last between sessions; a session start shows the
    /// open ones with the symbols they name.
    #[test]
    fn work_records_open_and_close() {
        let store = calc_project();
        assert_eq!(store.work_add("Make calc.add handle overflow").unwrap(), "Work record #1 added.\n");
        let list = store.work_list().unwrap();
        assert!(list.contains("Open work records:\n#1\nMake calc.add handle overflow\n"), "{list}");
        let session = store.session(&[]).unwrap();
        assert!(session.contains("== Context of the symbols the work record names ==") && session.contains("pub fn add("), "{session}");
        assert_eq!(store.work_done("#1").unwrap(), "Work record #1 closed.\n");
        assert!(!store.work_list().unwrap().contains("#1"));
        assert_eq!(store.work_done("9").unwrap_err().0, "No work record #9.");
        assert_eq!(store.work_done("x").unwrap_err().1, 2);
    }

    #[test]
    fn delete_refuses_a_used_symbol_and_names_are_unambiguous() {
        let store = calc_project();
        let err = store.delete("calc.add").unwrap_err().0;
        assert!(err.starts_with("E5001 calc.add is still used by:") && err.contains("calc.three"), "{err}");
        assert!(store.delete("calc.one").unwrap().starts_with("Deleted calc.one."));
        assert!(store.resolve(&store.index().unwrap(), "calc.one").is_err());
        store.submit_symbols(None, Some("other"), "pub fn two() -> Int { 2 }").unwrap();
        let err = store.resolve(&store.index().unwrap(), "two").unwrap_err().0;
        assert!(err.starts_with("E1002 ambiguous name: two") && err.contains("calc.two") && err.contains("other.two"), "{err}");
    }

    /// An unqualified type or constructor the module doesn't import is imported
    /// for it: from the standard library, or the one project module that has it.
    #[test]
    fn missing_type_imports_are_added() {
        let store = Scratch::new()
            .with(&[
                ("src/shape.gleam", "pub type Shape {\n  Square(Int)\n}\n"),
                ("src/app.gleam", "import shape\n\npub fn main() {\n  1\n}\n"),
            ])
            .with_packages()
            .import();
        let out = store.submit_symbols(None, Some("app"), "pub fn unit() -> Shape { Square(1) }").unwrap();
        assert!(
            out.contains("Note: imported `type Shape` from shape in app") && out.contains("Note: imported `Square` from shape in app"),
            "{out}"
        );
        let file = fs::read_to_string(store.root.join("src/app.gleam")).unwrap();
        assert!(file.starts_with("import shape.{type Shape, Square}\n"), "{file}");
        let out = store.submit_symbols(None, Some("app"), "pub fn none() -> Option(Int) { None }").unwrap();
        assert!(out.contains("from gleam/option in app") && out.contains("type-checks"), "{out}");
    }

    #[test]
    fn task_words() {
        assert_eq!(
            words("Add the TENOFF coupon to refundsByMonth in order_totals, 10 tests"),
            ["tenoff", "coupon", "refund", "month", "order", "total"]
        );
    }
}
