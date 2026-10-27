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
mod extract;
mod language;
mod rust_submit;
mod lockdown;
mod mcp;
mod migrate;
mod orientation;
mod outside;
mod reach;
mod reads;
mod recipe;
mod scope;
mod skills;
mod store;
mod tasks;
mod view;
mod workspace;

use crate::cli::{bare_command, command_words, verb_words};
use language::{Import, Item, Kind, Language, Ref, cargo, gleam};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};
use std::{env, fs};
use store::{projection, store_rust_file};

mod reference;
pub(crate) use reference::*;
mod index;
mod rust_probe;
mod fsx;
mod jsonx;
mod project_paths;
mod rust_move;
mod rust_check;
mod cli;
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
pub(crate) struct Failure(String, i32);
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
            mcp::mcp(budget);
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
    if let Some(rid) = board::strategy_context() {
        return strategy_run(&rid, args, input, budget);
    }
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
    if a.first() == Some(&"strategy") {
        return lockdown::error("E7001", "strategy", "strategy commands work only in a strategy run started from the Strategy page");
    }
    if a.first() == Some(&"task") && board::context().is_none() {
        return lockdown::error("E7010", "task", "task commands work only in a run started from the task board; this session has no task");
    }
    if a.as_slice() == ["session", "remember"] {
        let Some((tid, rid)) = board::context() else {
            return lockdown::error("E7010", "session remember", "session remember works only in a run started from the task board");
        };
        let top = scope::top(&Store::root()?);
        return board::remember(&top, &tid, &rid, budget);
    }
    let rest;
    let (args, cross) = match a.iter().position(|x| *x == "--project") {
        Some(i) => {
            let dir =
                a.get(i + 1).copied().ok_or_else(|| Failure("--project needs a project folder; session start lists them".into(), 2))?;
            rest = [&args[..i], &args[i + 2..]].concat();
            if rest.is_empty() {
                return usage(PROJECT_ONLY.trim_end());
            }
            match scoped(dir, &rest)? {
                Some(target) => (&rest[..], Some(target)),
                None => return read_project(dir, &rest, budget),
            }
        }
        None => (args, None),
    };
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // `move` or `extract` with `--to-project <dir>`: the destination's project,
    // which the task must cover too (D-84, D-103); outside a board run a command
    // changes one project. A `merge`'s is the survivor's, which it only reads (D-117).
    let dest = match a.iter().position(|x| *x == "--to-project") {
        Some(i) if matches!(a.first(), Some(&"move" | &"extract" | &"merge")) => {
            let dir =
                a.get(i + 1).copied().ok_or_else(|| Failure("--to-project needs a project folder; session start lists them".into(), 2))?;
            let Some(p) = scoped(dir, args)? else {
                return lockdown::error(
                    "E6007",
                    dir,
                    format!("`{} --to-project` works only in a board run whose task covers both projects", a[0]),
                );
            };
            let here = match &cross {
                Some(c) => c.clone(),
                None => Store::root()?,
            };
            Some(fs::canonicalize(&p)?).filter(|p| fs::canonicalize(&here).ok().as_ref() != Some(p))
        }
        _ => None,
    };
    // The task's home, where the task is stored. Its agent works in the
    // project its run started in (D-123), which may be another of its projects.
    let home = match (Store::root().ok(), board::context()) {
        (Some(here), Some((tid, _))) => Some(scope::home(&here, &tid).unwrap_or(here)),
        (here, _) => here,
    };
    let _within = cross.as_deref().map(Within::enter).transpose()?;
    let Ok(root) = Store::root() else {
        board::flow(&env::current_dir()?)?;
        return run_inner(args, input, old, budget);
    };
    let context = board::context();
    let home = home.unwrap_or_else(|| root.clone());
    // An extract into another project, and any merge, rewrites the task's
    // projects that depend on the source (D-103, D-117): each is locked,
    // recorded and committed as the destination is.
    let deps: Vec<PathBuf> = match (&context, &dest) {
        (Some((tid, _)), d) if (a[0] == "extract" && d.is_some()) || a[0] == "merge" => {
            let top = scope::top(&root);
            let projects = scope::of(&board::task(&home, tid)?, &scope::dir(&top, &home));
            extract::dependents(&top, &root, &projects).into_iter().filter(|p| Some(p) != d.as_ref()).collect()
        }
        _ => vec![],
    };
    let guard = if let Some((tid, rid)) = &context {
        // Away from home, the project the agent works in is locked too.
        let away = (cross.is_none() && !crate::project_paths::same_project(&root, &home)).then(|| root.clone());
        let also: Vec<&Path> = cross.iter().chain(&away).chain(&dest).chain(&deps).map(PathBuf::as_path).collect();
        board::agent_guard(&home, tid, rid, if a.is_empty() { &["help"] } else { &a }, &also)?
    } else {
        vec![board::lock(&root)?]
    };
    let store = board::open(&root)?;
    let home = board::open(&home)?;
    let dest = dest.map(|d| board::open(&d)).transpose()?;
    let deps = deps.iter().map(|d| board::open(d)).collect::<Result<Vec<_>, _>>()?;
    // A `task` command holds no project lock (D-77 point 5), so it runs before
    // recovery and absorbing: those need the lock, and a live command's records
    // must not be taken for a crashed one's. Its changes are committed rows, so
    // it reads them without either.
    if let Some((tid, rid)) = &context {
        if a.starts_with(&["task", "diff"]) {
            return board::task_diff(&home, tid, &a[2..], budget);
        }
        // In a task run, `impact` with no names is the task's own change list (D-102).
        if a == ["impact"] {
            return board::task_diff(&home, tid, &[], budget);
        }
        if a.starts_with(&["task", "peek"]) {
            return board::task_peek(&home, tid, &a[2..], budget);
        }
        if a.first() == Some(&"task") {
            return board::task_command(&home, tid, rid, &a, input, options, guard);
        }
    }
    // Every guard here holds the lock of `store`'s project.
    // An extract stopped between its projects' commits is undone first, under
    // every lock of its projects (D-135).
    let mut held = vec![store.root.clone(), home.root.clone()];
    held.extend(dest.iter().chain(&deps).map(|d| d.root.clone()));
    extract::ahead::recover(&held)?;
    lockdown::recover(&store.root, &store, &guard)?;
    if let Some(d) = &dest {
        lockdown::recover(&d.root, d, &guard)?;
    }
    for d in &deps {
        lockdown::recover(&d.root, d, &guard)?;
    }
    board::flow(&store.root)?;
    let noticed = outside::absorb(&store, outside::Mode::Notice)?;
    // A run's settings write is a proposal the human approves (D-38); a Rust
    // project's Cargo.toml is one only when it touches a protected key (D-158
    // point 4). Its text is read here, once, and replayed to the command.
    let mut settings_write: Option<String> = None;
    if let Some((tid, rid)) = &context {
        if a.first() == Some(&"write")
            && let Some(file) =
                a.get(1).map(|p| file_name(Path::new(p))).filter(|p| lockdown::settings_file(&store.root, p) || *p == "Cargo.toml")
        {
            let current = fs::read(store.root.join(&file)).unwrap_or_default();
            let proposed = lockdown::edited(&current, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old)?;
            if lockdown::settings_file(&store.root, &file) || lockdown::cargo_settings_changed(&current, &proposed) {
                let reason = REASON.with(|r| r.borrow().clone());
                return board::propose_settings(&home, &store.root, tid, rid, &file, &current, &proposed, reason.as_deref(), guard);
            }
            settings_write = Some(proposed);
        }
    }
    // A file patch (`write` with no path, D-104) names its files in `text`: it
    // is read once, for the snapshots and for the command.
    let patch = if a == ["write"] && context.is_some() { Some(input()?) } else { None };
    // A kept file patch (D-120) names its files in its draft, which a
    // `--replace` edits first; that `text` too is read once.
    let fix = if a.starts_with(&["write", "--draft"]) && a.contains(&"--replace") && context.is_some() { Some(input()?) } else { None };
    // The draft before and after the replacement, each parsed alone: the
    // replacement may name another file, or fix a line that didn't parse.
    let files: Vec<String> = match (a.as_slice(), &patch) {
        (_, Some(p)) => vec![p.clone()],
        (["write", "--draft", id, ..], _) if context.is_some() => match drafts::view(&store, id) {
            Ok(d) => {
                let fixed = fix.as_ref().and_then(|new| drafts::replace(&d, old, new, a.contains(&"--all")).ok()).map(|r| r.0);
                std::iter::once(d).chain(fixed).collect()
            }
            Err(_) => vec![],
        },
        _ => vec![],
    };
    let mut replay = || -> Out { Ok(patch.clone().or_else(|| fix.clone()).or_else(|| settings_write.clone()).unwrap_or_default()) };
    let input: &mut dyn FnMut() -> Out = if patch.is_some() || fix.is_some() || settings_write.is_some() { &mut replay } else { input };
    let before = if a.first().is_some_and(|c| CHANGES.contains(c)) && !reads_only(&a) && context.is_some() {
        let mut v = board::snapshot(&store)?;
        v.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        for f in &files {
            v.as_object_mut().unwrap().extend(lockdown::patch_snapshot(&store.root, Some(f)));
        }
        if a[0] == "extract" && a.contains(&"--to-project") {
            v.as_object_mut().unwrap().extend(dependency_files(&store.root));
            v.as_object_mut().unwrap().extend(script_files(&store.root));
        }
        Some(v)
    } else {
        None
    };
    // A move or extract into another project changes it too, recorded in its
    // store: a move's files there, or an extract's symbols, in a transaction of
    // its own around the command and its undo rows (D-103).
    let dest_before = match (&dest, &before) {
        // a merge's survivor doesn't change
        (Some(_), Some(_)) if a[0] == "merge" => None,
        (Some(d), Some(_)) if a[0] == "extract" => Some((d, None, with_scripts(d)?)),
        (Some(d), Some(_)) => {
            let (_, to, _) = lockdown::move_args(&store.root, &a)?;
            Some((d, Some(to), Value::Object(board::files_under(&d.root, to))))
        }
        _ => None,
    };
    let with_toml = |d: &Store| -> Result<Value, Failure> {
        let mut v = board::snapshot(d)?;
        v.as_object_mut().unwrap().extend(dependency_files(&d.root));
        Ok(v)
    };
    let deps_before: Vec<(&Store, Value)> =
        if dest_before.as_ref().is_some_and(|(_, to, _)| to.is_none()) || (a[0] == "merge" && before.is_some()) {
            deps.iter().map(|d| Ok((d, with_toml(d)?))).collect::<Result<_, Failure>>()?
        } else {
            vec![]
        };
    // The folders each project had, so undo removes only those the task made (D-136).
    let folders = before.as_ref().map(|_| board::existing_folders(&store.root));
    let d_folders = dest_before.as_ref().map(|(d, ..)| board::existing_folders(&d.root));
    let deps_folders: Vec<_> = deps_before.iter().map(|(d, _)| board::existing_folders(&d.root)).collect();
    // A code change and its undo rows commit together (D-77 point 7): one outer
    // transaction around the command and `record`. A process runs outside it, so
    // a long `run` holds the store's write lock only for the rows; the ahead
    // record in `.solace/pending/` covers a crash while the process runs.
    // Rust `format` saves through the store, so it belongs inside the outer
    // transaction; only a process that changes files behind the store stays out.
    let processes = is_process(&a, &store.root);
    COMMAND_DB.set(vec![(store.root.clone(), store.db.clone())]);
    let _outer = OuterTransaction(store.db.clone());
    let dest_tx = dest_before.as_ref().filter(|(_, to, _)| to.is_none()).map(|(d, ..)| *d);
    if let Some(d) = dest_tx {
        COMMAND_DB.with_borrow_mut(|c| c.push((d.root.clone(), d.db.clone())));
    }
    let _dest_outer = dest_tx.map(|d| OuterTransaction(d.db.clone()));
    for (d, _) in &deps_before {
        COMMAND_DB.with_borrow_mut(|c| c.push((d.root.clone(), d.db.clone())));
    }
    let _deps_outer: Vec<OuterTransaction> = deps_before.iter().map(|(d, _)| OuterTransaction(d.db.clone())).collect();
    if let (Some(_), Some((tid, rid))) = (&before, &context) {
        lockdown::mark(&store.root, tid, rid)?;
        if let Some((d, ..)) = &dest_before {
            lockdown::mark(&d.root, tid, rid)?;
        }
        for (d, _) in &deps_before {
            lockdown::mark(&d.root, tid, rid)?;
        }
    }
    // An extract into another project is written ahead: its record is in each of
    // its projects before any file changes (`extract::ahead`).
    let mut ahead = match (&context, dest_tx) {
        (Some((tid, rid)), Some(d)) if a[0] == "extract" && before.is_some() => {
            let mut roots: Vec<&Path> = vec![&store.root, &d.root];
            roots.extend(deps_before.iter().map(|(d, _)| d.root.as_path()));
            Some(extract::ahead::begin(tid, rid, &roots)?)
        }
        _ => None,
    };
    if before.is_some() && !processes {
        store.db.execute_batch("BEGIN IMMEDIATE")?;
        if let Some(d) = dest_tx {
            d.db.execute_batch("BEGIN IMMEDIATE")?;
        }
        for (d, _) in &deps_before {
            d.db.execute_batch("BEGIN IMMEDIATE")?;
        }
    }
    // A settings write's text already applied `old`; replay it whole (D-158 point 4).
    let whole = settings_write.is_some();
    let replay: Vec<String> = args.iter().filter(|x| !whole || (*x != "--replace" && *x != "--all")).cloned().collect();
    let result = run_inner(&replay, input, if whole { None } else { old }, budget);
    if before.is_some() {
        crash_after("command");
    }
    if let Some((tid, _)) = &context
        && a.first().is_some_and(|c| VERIFIES.contains(c))
        && !reads_only(&a)
    {
        board::record_verification(&home, tid, a[0], &result)?;
    }
    if before.is_some() && processes {
        store.db.execute_batch("BEGIN IMMEDIATE")?;
    }
    // A process the command ran (`format`, a declared command) may have changed source: that's this command's change.
    let unread = if processes { outside::absorb(&store, outside::Mode::Command)? } else { String::new() };
    if let (Some(before), Some((tid, rid))) = (before, &context) {
        let mut after = board::snapshot(&store)?;
        after.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        for f in &files {
            after.as_object_mut().unwrap().extend(lockdown::patch_snapshot(&store.root, Some(f)));
        }
        if a[0] == "extract" && a.contains(&"--to-project") {
            after.as_object_mut().unwrap().extend(dependency_files(&store.root));
            after.as_object_mut().unwrap().extend(script_files(&store.root));
        }
        if let (Some(record), Some(d)) = (&mut ahead, dest_tx) {
            // The computed result joins the record before the first commit; then the
            // destination commits, then the source and the dependents, each with its
            // undo rows; then the gleam.toml lines, and the record goes.
            let d_before = &dest_before.as_ref().unwrap().2;
            let d_after = with_scripts(d)?;
            let deps_after: Vec<Value> = deps_before.iter().map(|(d, _)| with_toml(d)).collect::<Result<_, _>>()?;
            let mut stores: Vec<(&Store, &Value, &Value)> = vec![(&store, &before, &after), (d, d_before, &d_after)];
            stores.extend(deps_before.iter().zip(&deps_after).map(|((d, b), a)| (*d, b, a)));
            extract::ahead::complete(record, &stores)?;
            crash_after("complete");
            board::record(d, tid, rid, d_before, &d_after, d_folders.as_ref())?;
            d.db.execute_batch("COMMIT")?;
            crash_after("destination");
            board::record(&store, tid, rid, &before, &after, folders.as_ref())?;
            store.db.execute_batch("COMMIT")?;
            crash_after("source");
            for (((d, d_before), d_after), f) in deps_before.iter().zip(&deps_after).zip(&deps_folders) {
                board::record(d, tid, rid, d_before, d_after, Some(f))?;
                d.db.execute_batch("COMMIT")?;
            }
            extract::ahead::end(record)?;
            crash_after("commit");
        } else {
            // A merge's dependents commit before the source.
            for ((d, d_before), f) in deps_before.iter().zip(&deps_folders) {
                board::record(d, tid, rid, d_before, &with_toml(d)?, Some(f))?;
                d.db.execute_batch("COMMIT")?;
                lockdown::drop_records(&d.root, rid)?;
            }
            if !deps_before.is_empty() {
                crash_after("others");
            }
            board::record(&store, &tid, &rid, &before, &after, folders.as_ref())?;
            store.db.execute_batch("COMMIT")?;
            crash_after("commit");
            // The task lists its changes from the rows that committed (D-77 point 4),
            // computed by changes_of, so there is nothing to write here.
            lockdown::drop_records(&store.root, &rid)?;
        }
        if let Some((d, Some(to), files)) = dest_before {
            d.db.execute_batch("BEGIN IMMEDIATE")?;
            board::record(d, tid, rid, &files, &Value::Object(board::files_under(&d.root, to)), d_folders.as_ref())?;
            d.db.execute_batch("COMMIT")?;
            lockdown::drop_records(&d.root, rid)?;
        }
    }
    match hinted(result, &store.root) {
        Ok(out) => Ok(format!("{noticed}{out}{unread}")),
        Err(Failure(msg, code)) => Err(Failure(format!("{noticed}{msg}{unread}"), code)),
    }
}

/// A command in strategy run `rid` (D-93): `strategy list | show | note`, and
/// plain reads of any project under the root, taking no project lock. Any
/// other command is refused (`E7010`): host commands, `task` commands and those
/// that change a project.
fn strategy_run(rid: &str, args: &[String], input: &mut dyn FnMut() -> Out, budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = a.first().copied().unwrap_or("");
    if command != "strategy" && !READS.contains(&command) {
        let why = match command {
            "task" => "a strategy run replies with `strategy note`; task commands are refused",
            c if HOST.contains(&c) => "host command is unavailable in a run",
            _ => {
                "a strategy run changes no code and only reads: strategy list | show | note, and view, inspect, \
                 refs, find, grep, files and read (with --project <folder> for another project)"
            }
        };
        return lockdown::error("E7010", command, why);
    }
    let top = workspace::enclosing(&env::current_dir()?)?.ok_or_else(|| Failure("No Solace root here.".into(), 1))?;
    board::strategy_guard(&top, rid, &a)?;
    if command == "strategy" {
        return board::strategy_command(&top, rid, &a, input, budget);
    }
    // A read, in the root's project or with `--project` in another.
    match a.iter().position(|x| *x == "--project") {
        Some(i) => {
            let dir = a.get(i + 1).copied().ok_or_else(|| Failure("--project needs a project folder".into(), 2))?;
            read_project(dir, &[&args[..i], &args[i + 2..]].concat(), budget)
        }
        None => read_project(".", args, budget),
    }
}

/// An extract into another project may add a dependency to gleam.toml, which
/// the build then locks in manifest.toml (D-103): the record of the source and
/// of each dependent covers both, so undo puts both back.
fn dependency_files(root: &Path) -> serde_json::Map<String, Value> {
    ["gleam.toml", "manifest.toml"].iter().map(|n| (format!("file:{n}"), json!(fs::read(root.join(n)).ok()))).collect()
}

/// An extract into another project takes along the JavaScript files its code
/// uses (D-103): the records of both sides cover every `.mjs` under src/ and
/// test/, so undo puts a carried file back and takes away its copy.
fn script_files(root: &Path) -> serde_json::Map<String, Value> {
    extract::js_files(root).into_iter().map(|n| (format!("file:{n}"), json!(fs::read(root.join(&n)).ok()))).collect()
}

/// A store's snapshot with its JavaScript files.
fn with_scripts(d: &Store) -> Result<Value, Failure> {
    let mut v = board::snapshot(d)?;
    v.as_object_mut().unwrap().extend(script_files(&d.root));
    Ok(v)
}

/// Ends a command's outer transaction on every path out of `run_options`: one
/// left open by an early `?` rolls back, and the shared connection is let go,
/// so a longer-lived caller never keeps the store's write lock.
struct OuterTransaction(Rc<Connection>);
impl Drop for OuterTransaction {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        COMMAND_DB.set(vec![]);
    }
}

/// A test-only hook: `SOLACE_FAIL_AFTER=<point>` exits after the named point,
/// and `panic:<point>` panics there instead, so a caught panic is tested too
/// (`command` | `complete` | `destination` | `others` | `commit`;
/// `complete` is after an extract's record gains its result, before any commit;
/// `destination` after its destination commits, before the source does;
/// `source` after the source commits, before the dependents do;
/// `others` after a merge's dependents commit, before the source does).
/// Only in a board run, so a plain CLI call is never interrupted.
fn crash_after(point: &str) {
    if board::context().is_none() {
        return;
    }
    let Ok(asked) = env::var("SOLACE_FAIL_AFTER") else { return };
    if asked == point {
        std::process::exit(1);
    }
    if asked.strip_prefix("panic:") == Some(point) {
        panic!("SOLACE_FAIL_AFTER=panic:{point}");
    }
}

// Commands by what they do. Every check that depends on the command uses these.
/// Plain reads; with `--project`, they read another project.
const READS: &[&str] = &["view", "inspect", "refs", "find", "grep", "files", "read"];
/// May change the project: recorded on the task in a board run, and refused in
/// read-only runs, except `build`, which reviews run to verify.
const CHANGES: &[&str] = &["submit", "delete", "extract", "merge", "write", "remove", "move", "run", "format", "build"];
/// Run a process that may change source; what it changed is read in after.
const PROCESSES: &[&str] = &["format", "build", "run"];
/// Results recorded as the task's verification.
const VERIFIES: &[&str] = &["check", "test", "build"];
/// Only for the human, never in a board run.
const HOST: &[&str] = &["agent-config", "serve", "create", "migrate"];
/// Need symbol-held source (Gleam or Rust): refused in a project of plain files.
const GLEAM: &[&str] =
    &["submit", "drafts", "view", "inspect", "refs", "impact", "find", "delete", "extract", "merge", "check", "test", "probe"];
/// D-119: changes whose failure stops the call, so nothing after them runs on unsaved work.
const STOPS: &[&str] = &["submit", "delete", "extract", "merge", "write", "remove", "move"];
/// D-152: what a reply ends with when it held back a `task done`.
const HELD: &str = "`task done` waits for a test that passes and reaches every change: fix what the test reports, \
then send `task done` again with an updated summary.\n";
/// task:da797576d0cfaaac: a line left with no command once `--project <dir>` is
/// taken off is refused as that line's error; the rest of the call runs.
const PROJECT_ONLY: &str = "`--project` goes at the end of each command line it applies to, e.g. `view x.y --project <dir>`\n";
/// D-131: `build plan` only reads, so it is no change, process or verification.
fn reads_only(a: &[&str]) -> bool {
    a.starts_with(&["build", "plan"])
}

/// Several commands, one per line, run in order; they share `budget`, since the
/// agent cuts the reply as a whole. D-119: after a failed change the rest are
/// skipped. D-152: after a test that failed or left a change no test reaches,
/// a `task done` is skipped. A repeated failed command names its copy. The
/// reply, and whether a command failed.
fn batch(commands: &[&str], budget: usize, run: &mut dyn FnMut(&[String], usize) -> Out) -> (String, bool) {
    let (mut out, mut failed, mut skipped, mut stopped) = (String::new(), vec![], vec![], false);
    let (mut waiting, mut held) = (None::<String>, false);
    for (at, &command) in commands.iter().enumerate() {
        let command = bare_command(command);
        if stopped {
            skipped.push(command.to_string());
            continue;
        }
        let words = command_words(command);
        let verb = verb_words(&words);
        if let Some(why) = &waiting
            && verb.starts_with(&["task", "done"])
        {
            skipped.push(format!("{command} ({why})"));
            held = true;
            continue;
        }
        if commands.len() > 1 {
            out += &format!("==== solace {command} ====\n");
        }
        if words.first().is_some_and(|w| w == "help") {
            out += &agent_help(words.get(1).map(|w| w.as_str()));
            continue;
        }
        // `--project` names where its command runs, not a command of its own.
        if verb.is_empty() && words.iter().any(|w| w == "--project") {
            out += PROJECT_ONLY;
            failed.push(failed_copy(command, commands, at));
            continue;
        }
        let test = verb.first() == Some(&"test");
        if test {
            UNTESTED.with(|u| u.borrow_mut().take());
        }
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&words, budget.saturating_sub(out.len())))) {
            Ok(result) => result,
            Err(p) => {
                // A panic is this line's error, not the tool's: the lines after
                // it are skipped, and the next call starts clean.
                reset_after_panic();
                out += &panic_reply(&*p, Some(command));
                failed.push(failed_copy(command, commands, at));
                stopped = true;
                continue;
            }
        };
        if test && waiting.is_none() {
            waiting = match &result {
                Err(_) => Some("the test failed".into()),
                Ok(_) => UNTESTED.with(|u| u.borrow_mut().take()).map(|names| format!("no test reaches {names}")),
            };
        }
        match result {
            Ok(o) => out += &o,
            Err(Failure(msg, _)) => {
                out += &msg;
                failed.push(failed_copy(command, commands, at));
                stopped = words.first().is_some_and(|w| STOPS.contains(&w.as_str()));
            }
        }
        if commands.len() > 1 && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    // D-109, D-119: a batch names the commands that failed and those it skipped, for the agent and the run log.
    if commands.len() > 1 && !failed.is_empty() {
        out += &format!("{} of {} commands failed: {}", failed.len(), commands.len(), failed.join("; "));
        if !skipped.is_empty() {
            out += &format!("; skipped: {}", skipped.join("; "));
        }
        out.push('\n');
    } else if !skipped.is_empty() {
        out += &format!("Skipped: {}\n", skipped.join("; "));
    }
    if held {
        out += HELD;
    }
    UNTESTED.with(|u| u.borrow_mut().take());
    (out, !failed.is_empty())
}

/// What a panic replies to the agent, whatever its payload carries: the message
/// (`&str` or `String`), and the line it happened in, if it was inside one.
fn panic_reply(p: &(dyn Any + Send), line: Option<&str>) -> String {
    let message = if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    };
    let at = line.map_or(String::new(), |l| format!(" running `{l}`"));
    format!("Solace hit an internal error{at}: {message}; nothing after it ran\n")
}

/// Drops what a caught panic left with this thread — the untested changes a
/// `test` named, and any outer transaction it held open — so the next call
/// starts clean. A transaction's own drop guard rolls back as the panic
/// unwinds; this covers a panic in a place without one.
fn reset_after_panic() {
    UNTESTED.with(|u| u.borrow_mut().take());
    let _ = COMMAND_DB.try_with(|c| {
        let Ok(mut dbs) = c.try_borrow_mut() else { return };
        for (_, db) in std::mem::take(&mut *dbs) {
            if !db.is_autocommit() {
                let _ = db.execute_batch("ROLLBACK");
            }
        }
    });
}

/// The reference's example of a first call, for a session start with no
/// ranked symbols (D-145).
const EXAMPLE_CALL: &str = "for example `inspect checkout.pay payments.capture --context`";
/// D-151: the most bytes of open work records a session start shows.
const WORK_CAP: usize = 4000;
/// D-145: what a reply ends with from the second single read in a row.
const READ_HINT: &str = "Hint: put independent reads in one call, one per line.\n";

/// D-145: the hint a call's reply ends with, from the second call in a row
/// that held one read and succeeded; `streak` counts those calls, and any
/// other call starts it again.
fn read_hint(streak: &mut usize, commands: &[&str], failed: bool) -> &'static str {
    let read = match commands {
        [command] if !failed => {
            let mut words: Vec<&str> = command.split_whitespace().collect();
            if words.first() == Some(&"solace") {
                words.remove(0);
            }
            if let Some(i) = words.iter().position(|w| *w == "--project") {
                words.drain(i..(i + 2).min(words.len()));
            }
            words.first().is_some_and(|w| READS.contains(w))
        }
        _ => false,
    };
    *streak = if read { *streak + 1 } else { 0 };
    if *streak >= 2 { READ_HINT } else { "" }
}

/// Options the reference gives each command: `--x` words in its syntax lines.
fn options_of(command: &str) -> Vec<&'static str> {
    let mut out = vec![];
    for line in REFERENCE.lines() {
        let Some(syntax) = line.strip_prefix("  ").filter(|l| !l.starts_with(' ')) else { continue };
        let syntax = syntax.split("  ").next().unwrap_or("");
        let mut words = syntax.split_whitespace();
        let mut names = vec![words.next().unwrap_or("")];
        let mut rest = vec![];
        while let Some(w) = words.next() {
            if w == "|" && rest.is_empty() {
                names.extend(words.next());
            } else {
                rest.push(w);
            }
        }
        if !names.contains(&command) {
            continue;
        }
        for w in rest {
            let o = w.trim_start_matches(['[', '(', '`']).trim_end_matches([']', ')', '`', ',', '.']);
            let o = o.split(['=', ']']).next().unwrap_or("");
            if o.len() > 2 && o.starts_with("--") && !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}

/// A read command given an option it doesn't take is refused by name, rather
/// than its value read as a name or path. `grep` and `find` search their text
/// as written, so they are not checked.
fn unknown_option(a: &[&str]) -> Option<Out> {
    let command = *a.first()?;
    if !matches!(command, "view" | "inspect" | "refs" | "files" | "read") {
        return None;
    }
    let own = options_of(command);
    let bad = a[1..].iter().find(|x| x.starts_with("--") && !own.contains(x))?;
    let takers: Vec<&str> = READS.iter().copied().filter(|c| *c != command && options_of(c).contains(bad)).collect();
    let elsewhere = if takers.is_empty() {
        String::new()
    } else if *bad == "--context" && takers.contains(&"inspect") {
        "; `--context` belongs to `inspect`: `inspect <name> --context`".to_string()
    } else if takers.len() == 1 {
        format!("; {} takes {bad}", takers[0])
    } else {
        format!("; {bad} belongs to {}", takers.join(" and "))
    };
    let list = if own.is_empty() { format!("{command} takes no options") } else { format!("{command} takes: {}", own.join(", ")) };
    Some(usage(format!("{command} has no {bad}{elsewhere}. {list} (`help {command}`)")))
}

/// `--project <dir>` on a command that isn't a plain read, in a board run whose
/// task may change that project (D-61): the folder the command runs in. Plain
/// reads, and commands outside a run, go to `read_project`.
fn scoped(dir: &str, args: &[String]) -> Result<Option<PathBuf>, Failure> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let (Some((tid, _)), Ok(here)) = (board::context(), Store::root()) else { return Ok(None) };
    if READS.contains(&command) && !args.iter().any(|x| x == "--draft") {
        return Ok(None);
    }
    let own = scope::home(&here, &tid)?;
    let top = scope::top(&own);
    let dir = &crate::project_paths::project_folder(&top, Some(&here), dir, &workspace::members(&top)?);
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
// the working folder is process-wide; fine while a process serves one call at a time (CLI, MCP).
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
    if let Some(out) = unknown_option(&a) {
        return out;
    }
    if !READS.contains(&command) || a.contains(&"--draft") {
        return lockdown::error(
            "E6007",
            command,
            if board::strategy_context().is_some() {
                "`--project` only reads (view, inspect, refs, find, grep, files, read)"
            } else {
                "`--project` only reads (view, inspect, refs, find, grep, files, read) here; in a board run, \
                 it also changes the task's other projects"
            },
        );
    }
    // From a project, or from a folder of projects with none at its root.
    let here = env::current_dir()?;
    let own = Store::root().ok();
    read_project_in(&a, &here, own.as_deref(), dir, budget)
}

/// Each `E1001 unknown name` line of a reply, followed by the project that
/// has the name when another one does (D-123).
fn hinted(out: Out, here: &Path) -> Out {
    const MARK: &str = "E1001 unknown name: ";
    let add = |text: String| -> String {
        if !text.contains(MARK) {
            return text;
        }
        let lines: Vec<String> = text
            .split('\n')
            .map(|line| {
                match line.split_once(MARK).and_then(|(_, rest)| rest.split_whitespace().next()).and_then(|name| elsewhere(here, name)) {
                    Some(d) => format!("{line}\nfound in {d}: add --project {d}"),
                    None => line.to_string(),
                }
            })
            .collect();
        lines.join("\n")
    };
    match out {
        Ok(text) => Ok(add(text)),
        Err(Failure(msg, code)) => Err(Failure(add(msg), code)),
    }
}

fn elsewhere(here: &Path, name: &str) -> Option<String> {
    let (top, dirs) = other_projects(here)?;
    dirs.into_iter().find(|d| {
        let Some(s) = Store::peek(&scope::path(&top, d)).ok() else { return false };
        let Ok(ix) = s.index() else { return false };
        s.resolve(&ix, name).is_ok()
    })
}

fn run_inner(args: &[String], input: &mut dyn FnMut() -> Result<String, Failure>, old: Option<&str>, budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(a.as_slice(), [] | ["help"] | ["--help"]) {
        return Ok(REFERENCE.to_string());
    }
    if let Some(out) = unknown_option(&a) {
        return out;
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
    let task = board::context().map(|(tid, _)| tid);
    if CARGO_TOOLS.contains(&a[0])
        && let Some(id) = language::id(&store.root)
        && let Some(why) = language::reason(id)?
    {
        return lockdown::error("E6013", a[0], why);
    }
    if let Some(out) = lockdown::dispatch(&store, &settings, &a, input, old, budget) {
        return out;
    }
    // drafts include kept file patches (D-120), so a plain-files project lists, reads and drops them
    if GLEAM.contains(&a[0]) && a[0] != "drafts" && !matches!(a.as_slice(), ["view", "--draft", _]) && store.lang.is_none() {
        if !language::is_rust(&store.root) {
            return lockdown::plain(&store, task.as_deref(), a[0], a.get(1).copied());
        }
        // A Rust project's tools are cargo's (D-158, D-168, D-173, D-185, D-207):
        // `check` and `test` run, `submit` saves items by name and `delete`
        // removes them, `extract` moves items and `merge` folds duplicates, each
        // checked with cargo; `drafts` is language-neutral.
        match a[0] {
            tool if CARGO_TOOLS.contains(&tool) && cargo::installed() => {}
            tool if CARGO_TOOLS.contains(&tool) => {
                return lockdown::error("E6013", a[0], cargo::MISSING);
            }
            // Reads reach a Rust store's symbols by name, as they do Gleam's
            // (D-158); `refs` and `impact` answer from its reference graph (D-170).
            "view" | "inspect" | "find" | "refs" | "impact" => {}
            _ => return lockdown::plain(&store, task.as_deref(), a[0], a.get(1).copied()),
        }
    }
    match a.as_slice() {
        ["session", "start", "--task"] => store.session_task(&input()?),
        ["session", "start", "--task", task @ ..] => store.session_task(&task.join(" ")),
        ["submit", ..] => drafts::submit(&store, &a, input, old),
        ["drafts", "drop", id] => drafts::drop_one(&store, id),
        ["drafts"] => drafts::list(&store),
        ["view", "--draft", id] => drafts::view(&store, id),
        ["session", rest @ ..] => store.session(&operands(rest.strip_prefix(&["start"]).unwrap_or(rest))),
        ["view", rest @ ..] if !operands(rest).is_empty() => store.view(&operands(rest), rest.contains(&"--bodies"), budget),
        ["inspect", rest @ ..] if !operands(rest).is_empty() => store.inspect(&operands(rest), rest.contains(&"--context"), budget),
        ["refs", names @ ..] if !names.is_empty() => store.refs_all(names, budget),
        ["impact", names @ ..] => store.impact(names, budget),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        ["delete", names @ ..] if !names.is_empty() => store.delete_all(names, budget),
        ["extract", rest @ ..] => store.extract(rest, budget),
        ["merge", rest @ ..] => store.merge(rest, budget),
        ["check"] => command_report(store.check(), budget),
        ["check", "--warnings"] => command_report(store.check_report(true), budget),
        ["test", rest @ ..] => command_report(store.test(rest), budget),
        ["probe", rest @ ..] => {
            let (from, trace, in_) = probe_args(rest)?;
            // with --from the test's body is the probe; `text` only adds to it
            let text = if from.is_some() { input().unwrap_or_default() } else { input()? };
            command_report(store.probe(&text, from, &trace, in_), budget)
        }
        ["work", "add"] => store.work_add(&input()?),
        ["work", "add", text @ ..] => store.work_add(&text.join(" ")),
        ["work", "list"] | ["work"] => store.work_list(),
        ["work", "done", id] => store.work_done(id),
        _ => usage(format!(
            "Unknown command: {}\nCommands: session, inspect, view, refs, impact, find, submit, drafts, delete, \
             extract, merge, check, test, probe, work, files, read, write, remove, move, build, format, run, task.",
            a.join(" ")
        )),
    }
}

/// The module name a probe runs as.
const PROBE: &str = "solace_probe";

/// `probe [--from <fn>] [--in <module>] [--trace <fn>...]`
fn probe_args<'a>(args: &[&'a str]) -> Result<(Option<&'a str>, Vec<&'a str>, Option<&'a str>), Failure> {
    let (mut from, mut trace, mut module, mut tracing) = (None, vec![], None, false);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--from" => from = Some(*it.next().ok_or_else(|| Failure("--from needs a function name.".into(), 2))?),
            "--in" => module = Some(*it.next().ok_or_else(|| Failure("--in needs a module name.".into(), 2))?),
            "--trace" => tracing = true,
            _ if tracing && !a.starts_with("--") => trace.push(a),
            _ => return usage("probe [--from <fn>] [--in <module>] [--trace <fn>...]; the code is the tool's `text`"),
        }
    }
    if tracing && trace.is_empty() {
        return usage("--trace needs the functions to trace, e.g. `probe --trace pricing.quote`");
    }
    Ok((from, trace, module))
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

thread_local! {
    /// The `reason` argument of the current tool call, for a settings write's question.
    static REASON: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// D-152: the task's changes the last passing `test` found no test reaching,
    /// named for the reply, so the call's `task done` waits.
    static UNTESTED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// A command that asks the human something: `task ask`, a strategy run's
/// proposal (D-93), or a run's write of a protected file (a settings proposal).
/// It must be the only command in its call.
fn is_question(line: &str) -> bool {
    let w: Vec<&str> = line.strip_prefix("solace").unwrap_or(line).split_whitespace().collect();
    match w.as_slice() {
        ["task", "ask", ..] | ["strategy", "task" | "feature" | "milestone" | "vision", ..] => true,
        ["write", rest @ ..] if board::context().is_some() => {
            let root = Store::root().ok();
            rest.iter().any(|p| root.as_deref().is_some_and(|r| lockdown::settings_file(r, &file_name(Path::new(p)))))
        }
        _ => false,
    }
}

/// A `command` line that is Gleam or Rust code, not a Solace command.
fn looks_like_code(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    matches!(first, "let" | "echo" | "assert" | "use" | "case" | "pub" | "fn" | "import" | "}" | "//") || first.contains('(')
}

fn stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

// --- the store --------------------------------------------------------------

pub(crate) type Sym = index::Sym;

pub(crate) type ModuleRow = index::ModuleRow;

pub(crate) struct Store {
    /// Shared with the command's other stores of the same project, so they
    /// run inside its outer transaction (D-77 point 7).
    db: Rc<Connection>,
    root: PathBuf,
    /// The project's language, chosen once from its files; `None` for a
    /// project of plain files (D-60).
    lang: Option<&'static dyn Language>,
    /// Whether the store has the nested Rust item columns (`symbols.parent`, `modules.file`);
    /// `false` only for an older store opened by `peek` (D-158).
    nested: bool,
    /// Whether the store has `modules.owns_dir`; `false` only for an older store
    /// opened by `peek` (D-168).
    owns_dir: bool,
    /// Direct dependency modules, scanned again only when the manifest or the
    /// resolved package list changes.
    deps: std::cell::RefCell<Option<(DependencyKey, Rc<Vec<DependencyModule>>)>>,
}

thread_local! {
    /// The connections a command's outer transactions hold, by project root:
    /// the project's, and an extract's destination (D-103).
    static COMMAND_DB: RefCell<Vec<(PathBuf, Rc<Connection>)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) type Index = index::Index;

enum Target<'a> {
    Imports(&'a str),
    /// A whole module's text, as `view <module>` shows it.
    Module(&'a str),
}

const SCHEMA: &str = "
CREATE TABLE modules(path TEXT PRIMARY KEY, dir TEXT NOT NULL, header TEXT NOT NULL, file TEXT, owns_dir INTEGER);
CREATE TABLE symbols(id TEXT PRIMARY KEY, module TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  text TEXT NOT NULL, ord INTEGER NOT NULL, revision TEXT NOT NULL, live INTEGER NOT NULL DEFAULT 1,
  parent TEXT, sep TEXT, tail TEXT NOT NULL DEFAULT '');
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
CREATE TABLE IF NOT EXISTS board_touches(seq INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, entity TEXT, run TEXT, name TEXT);
-- the warnings as of the last check, so a submit can name the new ones
CREATE TABLE IF NOT EXISTS warnings(key TEXT PRIMARY KEY, module TEXT NOT NULL DEFAULT 'project');
CREATE TABLE IF NOT EXISTS warning_checks(one INTEGER PRIMARY KEY CHECK (one = 1));
";

/// Whether `table` has the column `col`.
fn has_column(db: &Connection, table: &str, col: &str) -> Result<bool, Failure> {
    Ok(db.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"), params![col], |r| r.get(0))?)
}

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

/// The name in a `// ── <name> (<kind>) ──` header (D-168): up to the closing
/// dashes, without the kind. A name may hold spaces (`<T as Tr>::f`).
fn block_name(header: &str) -> String {
    let name = header.split(" ──").next().unwrap_or("").trim();
    name.strip_suffix(')').and_then(|n| n.rsplit_once(" (")).map_or(name, |(name, _)| name).to_string()
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
    import_with(root, cargo::installed())
}

/// Imports `root`, fetching a Rust project's dependencies with `cargo` when
/// `has_cargo` (D-158 point 4); without it, the import goes ahead and says so.
fn import_with(root: &Path, has_cargo: bool) -> Out {
    let mut out = String::new();
    let dir = root.join(".solace");
    let stored = dir.join("store.db").exists();
    // A new Cargo package is read whole first, so a refused one leaves no settings or store behind.
    let rust = if stored {
        None
    } else {
        match language::detect(root) {
            None if root.join("Cargo.toml").exists() => Some(language::rust::package(root)?),
            _ => None,
        }
    };
    // Fetch before any write, so a failed fetch leaves the project as it was.
    let dependencies = dependencies(root, has_cargo)?;
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
    if stored {
        out += "This project already has a store (.solace/store.db); existing symbols were preserved.\n";
        out += &dependencies;
        return Ok(out);
    }
    fs::create_dir_all(&dir)?;
    let db = Connection::open(dir.join("store.db"))?;
    db.busy_timeout(Duration::from_secs(10))?;
    db.execute_batch(SCHEMA)?;
    db.execute_batch(LATER_TABLES)?;
    let (mut modules, mut symbols) = (0, 0);
    let mut written = outside::Written::load(root);
    written.known = true;
    if let Some(lang) = language::detect(root) {
        for rel in lang.source_files(root)? {
            let (top, file) = rel.split_once('/').unwrap();
            let path = Path::new(file).with_extension("").to_string_lossy().replace('\\', "/");
            let file = root.join(&rel);
            let text = fs::read_to_string(&file).map_err(|e| Failure(format!("{}: {e}", file.display()), 1))?;
            written.record(&outside::file_of(top, &path), text.as_bytes());
            let parsed = lang.split(&text);
            db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, top, parsed.header])?;
            for (ord, item) in parsed.items.iter().enumerate() {
                insert_symbol(&db, &path, item, Some(ord as i64))?;
                symbols += 1;
            }
            modules += 1;
        }
    }
    let rust_imported = rust.is_some();
    for (rel, module, file) in rust.into_iter().flatten() {
        written.record(&rel, file.join().as_bytes());
        store_rust_file(&db, &module, rel.split_once('/').unwrap().0, &rel, &file)?;
        modules += 1;
        symbols = db.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    }
    written.save()?;
    // The first check is warm, without making the import wait (D-158 point 3).
    if rust_imported && has_cargo {
        cargo::warm(root);
    }
    let store = Store::new(Rc::new(db), root, true, true);
    outside::approve_all(&store)?;
    out += &format!("Imported {modules} modules, {symbols} symbols into .solace/store.db\n");
    out += &dependencies;
    Ok(out)
}

/// Fetches a Rust project's dependencies, or says why it couldn't (D-158 point 4).
fn dependencies(root: &Path, has_cargo: bool) -> Out {
    if !language::is_rust(root) {
        return Ok(String::new());
    }
    if let Some(why) = language::reason(language::RUST)? {
        return Ok(format!("Dependencies not fetched: {why}\n"));
    }
    if has_cargo {
        cargo::fetch(root)?;
        Ok(String::new())
    } else {
        Ok(format!("Dependencies not fetched: {}\n", language::missing(language::RUST)))
    }
}

/// Stores a new symbol at `ord`, or after its module's last; returns its id.
fn insert_symbol(db: &Connection, module: &str, item: &Item, ord: Option<i64>) -> Result<String, Failure> {
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

impl Store {
    /// The store of the project at `root`, with every table this version uses.
    fn at(root: &Path) -> Result<Store, Failure> {
        let shared =
            COMMAND_DB.with_borrow(|c| c.iter().find(|(r, _)| crate::project_paths::same_project(r, root)).map(|(_, db)| db.clone()));
        if let Some(db) = shared {
            return Ok(Store::new(db, root, true, true));
        }
        let db = Connection::open(root.join(".solace/store.db"))?;
        // WAL and a busy timeout: readers see the last committed state while a
        // command holds the store's write lock (D-77 point 8).
        db.busy_timeout(Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.execute_batch(LATER_TABLES)?;
        Self::add_columns(&db)?;
        Ok(Store::new(Rc::new(db), root, true, true))
    }

    /// The language that reads this store's code.
    fn lang(&self) -> &'static dyn Language {
        language::reading(self.lang)
    }

    /// Columns added after their tables: `board_touches`' `run` and `name`
    /// (D-77 point 4), nested Rust items (D-158). Older stores get them.
    fn add_columns(db: &Connection) -> Result<(), Failure> {
        for (table, col, decl) in [
            ("board_touches", "run", "TEXT"),
            ("board_touches", "name", "TEXT"),
            ("symbols", "parent", "TEXT"),
            ("symbols", "sep", "TEXT"),
            ("symbols", "tail", "TEXT NOT NULL DEFAULT ''"),
            ("modules", "file", "TEXT"),
            ("modules", "owns_dir", "INTEGER"),
            ("warnings", "module", "TEXT NOT NULL DEFAULT 'project'"),
        ] {
            // A store of the root's board alone has no `symbols` or `modules`.
            let exists: bool =
                db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| r.get(0))?;
            if exists && !has_column(db, table, col)? {
                db.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), [])?;
            }
        }
        Ok(())
    }

    fn open() -> Result<Store, Failure> {
        Store::at(&Store::root()?)
    }

    /// Another project's store, only to read: a plain connection, which
    /// creates no tables and takes no lock (D-57).
    fn peek(root: &Path) -> Result<Store, Failure> {
        let file = root.join(".solace/store.db");
        if !file.exists() {
            return fail(format!("no Solace store in {}", root.display()));
        }
        let db = Connection::open(file)?;
        db.busy_timeout(Duration::from_secs(10))?;
        // An older store may lack the nested-item columns; the checks happen here, once
        // (D-158), so `index` reads its rows without touching `pragma_table_info`.
        let nested = has_column(&db, "symbols", "parent")? && has_column(&db, "modules", "file")?;
        let owns_dir = has_column(&db, "modules", "owns_dir")?;
        Ok(Store::new(Rc::new(db), root, nested, owns_dir))
    }

    /// The project the working folder is in: the nearest folder up with a store.
    fn root() -> Result<PathBuf, Failure> {
        #[cfg(test)]
        if let Some(root) = TEST_ROOT.with(|r| r.borrow().clone()) {
            return Ok(root);
        }
        let mut dir = env::current_dir()?;
        loop {
            if dir.join(".solace/store.db").exists() {
                return Ok(dir);
            }
            if !dir.pop() {
                return fail(
                    "No Solace store found (.solace/store.db). Bring existing code in by migrating it: \
                     run `solace serve` here and choose Migrate this project.",
                );
            }
        }
    }

    fn index(&self) -> Result<Index, Failure> {
        // A store only peeked at may be older than its nested-item columns (checked once in `peek`).
        let nested = if self.nested { "parent, sep, tail" } else { "NULL, NULL, ''" };
        let mut stmt =
            self.db.prepare(&format!("SELECT id, module, name, kind, text, {nested} FROM symbols WHERE live = 1 ORDER BY module, ord"))?;
        let syms: Vec<Sym> = stmt
            .query_map([], |r| {
                let kind: String = r.get(3)?;
                Ok(Sym {
                    id: r.get(0)?,
                    module: r.get(1)?,
                    name: r.get(2)?,
                    kind: Kind::parse(&kind),
                    kind_text: kind,
                    text: r.get(4)?,
                    parent: r.get(5)?,
                    sep: r.get(6)?,
                    tail: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let file = if self.nested { "file" } else { "NULL" };
        let owns = if self.owns_dir { "owns_dir" } else { "NULL" };
        let mut stmt = self.db.prepare(&format!("SELECT path, dir, header, {file}, {owns} FROM modules ORDER BY dir, path"))?;
        let modules: Vec<ModuleRow> = stmt
            .query_map([], |r| Ok(ModuleRow { path: r.get(0)?, dir: r.get(1)?, header: r.get(2)?, file: r.get(3)?, owns_dir: r.get(4)? }))?
            .collect::<Result<_, _>>()?;

        Ok(index::build(self.lang(), syms, modules, language::is_rust(&self.root)))
    }

    // --- writing files ------------------------------------------------------------
    /// Writes every module's file from the store. Returns, per file, the line
    /// range of each symbol, for mapping compiler output back to symbols.
    fn materialize(&self, ix: &Index) -> Result<Vec<FileMap>, Failure> {
        self.materialize_to(ix, &self.root, &HashMap::new())
    }

    /// `stops`: where an undo stops removing the folders a deleted module file
    /// leaves empty (D-136; `Written::prune`).
    fn materialize_to(&self, ix: &Index, target: &Path, stops: &HashMap<String, String>) -> Result<Vec<FileMap>, Failure> {
        // A file changed outside Solace is never written over (D-58); the next command reads it in.
        let mut written = (target == self.root).then(|| outside::Written::load(&self.root));
        let mut maps = vec![];
        for m in &ix.modules {
            let (content, ranges) = projection(ix, m);
            let rel = m.rel();
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
            // A file whose module is gone goes with it, unless edited since.
            w.prune(&ix.modules.iter().map(|m| m.rel()).collect(), stops)?;
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

    /// Runs the language's tool (`gleam check`, `gleam test`) in the project.
    fn tool(&self, args: &[&str]) -> Result<(bool, String), Failure> {
        if let Some(id) = language::id(&self.root)
            && let Some(why) = language::reason(id)?
        {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let mut argv = vec![self.lang().tool().to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        // whole, so every failure is parsed; the reply is trimmed later
        let text = lockdown::command(&self.root, &argv, 120, usize::MAX, false, false)?;
        Ok((lockdown::passed(&text), strip_ansi(text.split_once('\n').map_or("", |(_, s)| s))))
    }

    /// Runs `cargo` in the project (D-158): the exit code and its output.
    fn cargo(&self, args: &[&str], timeout: u64) -> Result<(bool, String), Failure> {
        if let Some(why) = language::reason(language::RUST)? {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let mut argv = vec!["cargo".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let text = lockdown::command(&self.root, &argv, timeout, usize::MAX, false, true)?;
        Ok((lockdown::passed(&text), strip_ansi(text.split_once('\n').map_or("", |(_, s)| s))))
    }

    /// Type-checks the project and records which symbols are broken, with
    /// their errors, so the next reply can report only what changed (D-46).
    fn check_all(&self, ix: &Index) -> Result<Vec<Diag>, Failure> {
        if let Some(id) = language::id(&self.root)
            && let Some(why) = language::reason(id)?
        {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let files = self.materialize(ix)?;
        self.refuse_drift(ix)?;
        let diags = if language::is_rust(&self.root) {
            let args = cargo::check_args(&self.root);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let (passed, output) = self.cargo(&argv, 120)?;
            let mut diags = cargo::diagnostics(&output, &files);
            // A build script that fails this way (no network, a refused
            // write) is cargo's exit with no located diagnostic, so the
            // check must add it or it would report a pass (D-158 point 4).
            if !passed && !diags.iter().any(|d| d.error) {
                diags.push(Diag { error: true, module: None, symbol: None, text: cargo::failure_text(&output) });
            }
            diags
        } else {
            let (_, output) = self.tool(&["check"])?;
            self.lang().diagnostics(&output, &files)
        };
        self.db.execute("DELETE FROM broken", [])?;
        for d in diags.iter().filter(|d| d.error) {
            if let Some(sym) = &d.symbol {
                self.db.execute(
                    "INSERT INTO broken VALUES (?1, ?2) ON CONFLICT(symbol) DO UPDATE SET diagnostic = \
                     diagnostic || char(10, 10) || excluded.diagnostic",
                    params![sym, d.text],
                )?;
            }
        }
        self.record_warnings(&diags)?;
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

    fn resolve(&self, ix: &Index, input: &str) -> Result<usize, Failure> {
        let mut found: Vec<usize> = store::matching(ix, input);
        // In a Rust store a type and its `impl` share a name; the item wins (D-158).
        if ix.rust {
            let named: Vec<usize> = found.iter().copied().filter(|&i| ix.syms[i].kind_text != "impl").collect();
            if !named.is_empty() {
                found = named;
            }
        }
        match found.as_slice() {
            [i] => Ok(*i),
            [] => {
                let hint = match input.strip_prefix("module:") {
                    Some(name) if !name.is_empty() => {
                        format!("; `module:` names a module, not a symbol: use `view {name}`")
                    }
                    _ => store::similar_hint(ix, input),
                };
                fail(format!("E1001 unknown name: {input}{hint}"))
            }
            many => {
                let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind_text)).collect();
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
        let declaration = |item: &Item| {
            let sig = self.lang().signature(&Item { text: without_comment_lines(&item.text), ..item.clone() }).trim_end().to_string();
            // An opaque type's constructors aren't part of its API.
            if sig.contains("pub opaque type ") { sig.split(" {").next().unwrap_or(&sig).to_string() } else { sig }
        };
        let public = |item: &Item| item.text.lines().any(|l| l.starts_with("pub "));
        if let Some((title, text)) = source(input) {
            let m = self.lang().split(&text);
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
        let item = self.lang().split(&text).items.into_iter().find(|i| i.name == name && public(i))?;
        let doc: String = item.text.lines().filter(|l| l.starts_with("///")).map(|l| format!("{l}\n")).collect();
        Some(format!("// ── {name} in {title} ({}) ──\n{doc}{}\n\n", item.kind.as_str(), declaration(&item)))
    }

    fn resolve_module<'a>(&self, ix: &'a Index, input: &str) -> Option<&'a ModuleRow> {
        store::module_named(ix, input)
    }

    /// The symbol index (one line per source symbol), test modules, and open
    /// work records expanded with the context of the symbols they name.
    fn session(&self, scopes: &[&str]) -> Out {
        const CAP: usize = 80;
        let tid = board::context().map(|(t, _)| t);
        let rust = language::is_rust(&self.root);
        if self.lang.is_none() && !rust {
            return self.plain_session("", tid.as_deref());
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
            if store::is_test(m) {
                tests.push(format!("{} ({})", m.path, syms.len()));
                continue;
            }
            // Names only: this is re-sent on every turn, so it must stay small (D-12).
            let names = store::module_names(&ix, &m.path);
            let more = if names.len() > CAP { format!(", …{} more", names.len() - CAP) } else { String::new() };
            out += &format!("{} — {}\n  {}{more}\n", m.path, store::module_doc(m), names[..names.len().min(CAP)].join(", "));
        }
        out += &format!("\nTest modules (tests): {}\n", tests.join(", "));
        if rust {
            out += &format!("\n{}", store::rust_brief(&ix));
        }
        out += &self.session_end(&ix, EXAMPLE_CALL, tid.as_deref())?;
        Ok(out)
    }

    /// Session start in a project of plain files (D-60): no symbols to list.
    /// `deps` is the task's unfinished dependencies, for `--task` in a board
    /// run of task `tid`.
    fn plain_session(&self, deps: &str, tid: Option<&str>) -> Out {
        let mut out = self.project_instructions();
        out += deps;
        out += "This project has plain files and no code held as symbols of its own: `files` lists them, `read` and `write` change them, \
                and `run` runs the commands its solace.toml declares.\n";
        out += &self.session_end(&self.index()?, "for example `grep <text> --in <path>` and `read <path> --lines a..b`", tid)?;
        Ok(out)
    }

    /// The end of every session start: status, open work and its context,
    /// the other projects, the command reference, and last `call`, a first
    /// call to send, where the agent plans its reads (D-145). `tid` is the
    /// board run's task; the open work records are capped (D-151).
    fn session_end(&self, ix: &Index, call: &str, tid: Option<&str>) -> Out {
        let next = next_call(call, board::strategy_context().is_some());
        let (status, work) = self.status_within(WORK_CAP)?;
        Ok(format!("{status}{}{}\n{}{next}", self.work_context(ix, &work)?, self.other_projects(tid), agent_reference(tid.is_some())))
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

    /// Where the agent is, and the other projects under this project's Solace
    /// root, which reads reach with `--project` (D-57): in a task run, those
    /// its task may change apart from those it only reads, and the rules for
    /// `--project` and paths (D-123); `tid` is the board run's task.
    fn other_projects(&self, tid: Option<&str>) -> String {
        let Ok(Some(root)) = workspace::enclosing(&self.root) else { return String::new() };
        let Ok(members) = workspace::members(&root) else { return String::new() };
        let me = scope::dir(&root, &self.root);
        let label = |m: &str| match workspace::name(&scope::path(&root, m)) {
            name if name.is_empty() && m == "." => ". (the root)".to_string(),
            name if name.is_empty() => m.to_string(),
            name => format!("{m} ({name})"),
        };
        // The root itself last: it is rarely where code lives.
        let mut others: Vec<&String> = members.iter().filter(|m| **m != me).collect();
        others.sort_by_key(|m| *m == ".");
        if others.is_empty() {
            return String::new();
        }
        let labels = |ms: Vec<&String>| ms.into_iter().map(|m| label(m)).collect::<Vec<_>>().join(", ");
        let task = tid.and_then(|t| scope::of_task(&self.root, t));
        let mut out = format!("\nYou are in {}.\n", label(&me));
        match &task {
            Some(scoped) => {
                out += &format!("This task can change: {}.\n", labels(scoped.iter().collect()));
                let read: Vec<&String> = others.iter().copied().filter(|m| !scoped.contains(m)).collect();
                if !read.is_empty() {
                    out += &format!("Read only: {}.\n", labels(read));
                }
            }
            None => out += &format!("Other projects under this Solace root (read only): {}.\n", labels(others.clone())),
        }
        let example = task.as_ref().and_then(|s| others.iter().copied().find(|m| s.contains(m))).unwrap_or(others[0]);
        out += &format!(
            "`--project <folder>` runs one line in another project: its folder as listed, from the Solace root \
             (`--project {example}`), for that line only. Reads (view, inspect, refs, find, grep, files, read) \
             reach every project listed{}.\n\
             Paths in `read`, `grep --in`, `files`, `write` and `move` are relative to the project the line runs in; \
             `read`, `grep --in` and `files` also take a path from the Solace root into another project.\n",
            if task.is_some() { "; other commands, only this task's" } else { "" }
        );
        if task.is_some() {
            out += "If the task needs a change in a read-only project (or the root, `.`), ask the human with `task ask` \
                    to add that project, then make the change yourself; never put the change somewhere else instead.\n";
        }
        out
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

    /// The open work records `work` expanded with the context of the symbols they name.
    fn work_context(&self, ix: &Index, work: &[String]) -> Out {
        let mut out = String::new();
        for text in work {
            let named = self.named(ix, text);
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
    /// within a fixed budget, plus the module list. In a board run it also
    /// names the task's feature (D-138) and unfinished dependencies (D-110).
    fn session_task(&self, task: &str) -> Out {
        let tid = board::context().map(|(tid, _)| tid);
        let plan = tid.as_deref().map(|tid| board::plan_reading(&scope::top(&self.root), tid)).unwrap_or_default();
        self.task_session(task, tid.as_deref(), &plan)
    }

    /// `session start --task` for board run task `tid`, with `deps` before the
    /// symbols; the board builds a fresh run's brief with it (D-151), outside
    /// the run's environment.
    fn task_session(&self, task: &str, tid: Option<&str>, deps: &str) -> Out {
        const BUDGET: usize = 6500; // bytes, about 1.6k tokens
        let rust = language::is_rust(&self.root);
        // The task's other projects with code, by folder: the list of symbols
        // draws on them too (D-123).
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let others: Vec<(String, Store, Index)> = tid
            .and_then(|t| scope::of_task(&self.root, t))
            .unwrap_or_default()
            .into_iter()
            .filter(|d| {
                let path = scope::path(&top, d);
                *d != mine && (language::detect(&path).is_some() || language::is_rust(&path))
            })
            .filter_map(|d| {
                let s = Store::peek(&scope::path(&top, &d)).ok()?;
                let ix = s.index().ok()?;
                Some((d, s, ix))
            })
            .collect();
        if self.lang.is_none() && !rust && others.is_empty() {
            return self.plain_session(deps, tid);
        }
        let ix = self.index()?;
        let projects: Vec<(Option<&str>, &Store, &Index)> =
            std::iter::once((None, self, &ix)).chain(others.iter().map(|(d, s, x)| (Some(d.as_str()), s, x))).collect();
        let mut modules = String::from("Modules (`view <module>` for an outline):\n");
        let grouped = !others.is_empty();
        for (p, (folder, _, pix)) in projects.iter().enumerate() {
            if grouped && pix.modules.is_empty() {
                continue;
            }
            if grouped {
                modules += &format!("{}:\n", if p == 0 { mine.as_str() } else { folder.unwrap_or(mine.as_str()) });
            }
            let mut tests = vec![];
            for m in &pix.modules {
                if store::is_test(m) {
                    tests.push(m.path.as_str());
                } else {
                    modules += &format!("  {} — {}\n", m.path, store::module_doc(m));
                }
            }
            modules += &format!("Test modules: {}\n", tests.join(", "));
        }
        let rust_ix = if rust { Some(&ix) } else { others.iter().find(|(_, s, _)| language::is_rust(&s.root)).map(|(_, _, x)| x) };
        if let Some(rust_ix) = rust_ix {
            modules += &format!("\n{}", store::rust_brief(rust_ix));
        }

        // Direct matches: rare task words weigh more; a word in the name
        // counts 3, in the doc line 2, anywhere else in the code 1. Words are
        // counted over all the task's projects.
        let fields: Vec<Vec<[HashSet<String>; 3]>> = projects
            .iter()
            .map(|(_, _, ix)| {
                ix.syms
                    .iter()
                    .map(|s| {
                        let doc: String = s.text.lines().filter(|l| l.trim_start().starts_with("///")).collect::<Vec<_>>().join(" ");
                        [words(&s.name).into_iter().collect(), words(&doc).into_iter().collect(), words(&s.text).into_iter().collect()]
                    })
                    .collect()
            })
            .collect();
        let n: usize = fields.iter().map(Vec::len).sum();
        let mut terms = words(task);
        terms.sort();
        terms.dedup();
        let mut scores: Vec<Vec<f64>> = fields.iter().map(|f| vec![0.0f64; f.len()]).collect();
        let mut rare: Vec<(f64, String)> = vec![];
        for t in &terms {
            let df = fields.iter().flatten().filter(|f| f.iter().any(|w| w.contains(t))).count();
            if df == 0 {
                continue;
            }
            // squared, so one rare word (a code, a name) outweighs several common ones
            let idf = (1.0 + n as f64 / df as f64).ln().powi(2);
            rare.push((idf, t.clone()));
            for (score, fields) in scores.iter_mut().zip(&fields) {
                for (i, f) in fields.iter().enumerate() {
                    if let Some(k) = f.iter().position(|w| w.contains(t)) {
                        score[i] += idf * [3.0, 2.0, 1.0][k];
                    }
                }
            }
        }

        // One hop from each project's best matches: callers, callees and
        // covering tests; then the non-test symbols (fixtures) the top
        // covering tests use. Symbols the task names (`module.name`) come first.
        for (&(_, store, ix), score) in projects.iter().zip(scores.iter_mut()) {
            let direct = score.clone();
            let mut seeds: Vec<usize> = (0..direct.len()).filter(|&i| direct[i] > 0.0).collect();
            seeds.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]));
            seeds.truncate(10);
            let boost = |j: usize, s: f64, score: &mut Vec<f64>| {
                if direct[j] + s > score[j] {
                    score[j] = direct[j] + s;
                }
            };
            for &s in &seeds {
                for &j in &ix.uses[s] {
                    boost(j, direct[s] / 4.0, &mut *score);
                }
                let mut covering = vec![];
                for &j in &ix.used_by[s] {
                    if is_test(ix, j) {
                        covering.push(j);
                    }
                    boost(j, direct[s] / 4.0, &mut *score);
                }
                covering.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));
                for &t in covering.iter().take(2) {
                    for &j in ix.uses[t].iter().filter(|&&j| !is_test(ix, j)) {
                        boost(j, direct[s] / 8.0, &mut *score);
                    }
                }
            }
            for i in store.named(ix, task) {
                score[i] = f64::MAX;
            }
        }
        // Each entry is (project, symbol), the current project's first.
        let mut ranked: Vec<(usize, usize)> =
            scores.iter().enumerate().flat_map(|(p, s)| (0..s.len()).filter(move |&i| s[i] > 0.0).map(move |i| (p, i))).collect();
        let size = |(p, i): (usize, usize)| projects[p].2.syms[i].text.len();
        ranked.sort_by(|&a, &b| scores[b.0][b.1].total_cmp(&scores[a.0][a.1]).then(size(a).cmp(&size(b))));
        let score = &scores[0];

        // The task's own terms in string literals are evidence (D-44): its rarest
        // words, quoted phrases and capitalized names such as "Desk Lamp".
        rare.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut evidence_terms: Vec<String> = rare.into_iter().take(5).map(|r| r.1).collect();
        evidence_terms.extend(task.split('"').skip(1).step_by(2).filter(|p| (3..=40).contains(&p.len())).map(str::to_lowercase));
        evidence_terms.extend(capitalized_runs(task));
        // A test that reaches the top code symbols reproduces the task's case.
        let code: Vec<usize> = ranked
            .iter()
            .filter(|&&(p, _)| p == 0)
            .map(|&(_, i)| i)
            .filter(|&i| !is_test(&ix, i) && ix.syms[i].kind == Kind::Fn && !ix.syms[i].text.contains("@external"))
            .take(2)
            .collect();
        let name = |p: usize, i: usize| {
            let (_, store, pix) = projects[p];
            let short = reach::short(pix, i);
            if store.resolve(pix, &short).ok() == Some(i) { short } else { pix.syms[i].qname() }
        };
        let repro = if rust {
            String::new()
        } else {
            code.first()
                .and_then(|_| {
                    reach::test_symbols(&ix).into_iter().map(|t| (t, reach::closure(&ix, t))).filter(|(_, r)| r.contains(&code[0])).max_by(
                        |a, b| {
                            let reached = |r: &HashSet<usize>| code.iter().filter(|c| r.contains(c)).count();
                            reached(&a.1)
                                .cmp(&reached(&b.1))
                                .then(score[a.0].total_cmp(&score[b.0]))
                                .then(ix.syms[b.0].text.len().cmp(&ix.syms[a.0].text.len()))
                        },
                    )
                })
                .map(|(t, r)| {
                    let traced: Vec<String> = code.iter().filter(|c| r.contains(c)).map(|&c| name(0, c)).collect();
                    format!(
                        "To reproduce the task's case: `probe --from {} --trace {}`, with statements for the case in `text`.\n",
                        name(0, t),
                        traced.join(" ")
                    )
                })
                .unwrap_or_default()
        };
        let mut packet = String::new();
        let mut room = BUDGET.saturating_sub(modules.len() + repro.len());
        for &(p, i) in &ranked {
            // whole entries only: skip one that doesn't fit, try smaller ones
            let (folder, _, pix) = projects[p];
            let s = &pix.syms[i];
            let at = folder.map(|d| format!(" ({d})")).unwrap_or_default();
            let entry = format!("-- {}{at}\n{}\n", s.qname(), index::signature(pix, s));
            let mut quoted = String::new();
            for e in evidence(&s.text, &evidence_terms) {
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
        out += deps;
        let marked = if others.is_empty() { "" } else { "; one marked `(<folder>)` is in that project: add `--project <folder>`" };
        out += &format!(
            "Symbols most likely relevant to the task, best first (signatures; `>` lines quote code that names the task's terms; \
             `inspect` for code{marked}):\n{packet}{repro}\n{modules}"
        );
        let start: Vec<usize> = ranked.iter().filter(|&&(p, _)| p == 0).map(|&(_, i)| i).take(3).collect();
        let call = if !start.is_empty() {
            let names: Vec<String> = start.iter().map(|&i| name(0, i)).collect();
            format!("`inspect {} --context`", names.join(" "))
        } else if let Some(&(p, _)) = ranked.first() {
            let names: Vec<String> = ranked.iter().filter(|&&(q, _)| q == p).map(|&(_, i)| name(p, i)).take(3).collect();
            format!("`inspect {} --project {} --context`", names.join(" "), projects[p].0.unwrap_or(mine.as_str()))
        } else {
            EXAMPLE_CALL.to_string()
        };
        out += &self.session_end(&ix, &call, tid)?;
        Ok(out)
    }

    fn open_work(&self) -> Result<Vec<(i64, String)>, Failure> {
        let mut stmt = self.db.prepare("SELECT id, text FROM work WHERE open = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Open work records and broken symbols.
    fn status(&self) -> Result<String, Failure> {
        Ok(self.status_within(usize::MAX)?.0)
    }

    /// `status` with the open work records cut at a whole record within `cap`
    /// bytes, saying how many were left out (D-14), and the texts of those shown.
    fn status_within(&self, cap: usize) -> Result<(String, Vec<String>), Failure> {
        let mut out = String::new();
        let work = self.open_work()?;
        if !work.is_empty() {
            out += "\nOpen work records:\n";
        }
        let mut shown = vec![];
        for (id, text) in &work {
            let entry = format!("#{id}\n{}\n", text.trim_end());
            if out.len() + entry.len() > cap {
                break;
            }
            out += &entry;
            shown.push(text.clone());
        }
        if shown.len() < work.len() {
            out += &format!(
                "{} of {} open work records left out to fit the reply's size limit; `work list` shows them all.\n",
                work.len() - shown.len(),
                work.len()
            );
        }
        let broken: Vec<String> =
            self.db.prepare("SELECT symbol FROM broken ORDER BY symbol")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        if !broken.is_empty() {
            out += &format!("\nBroken symbols: {}\n", broken.join(", "));
        }
        Ok((out, shown))
    }

    /// A module is shown as an outline unless `bodies` is set. Past `budget`,
    /// module bodies fall back to outlines, and what still doesn't fit is
    /// listed to view separately (D-14).
    fn view(&self, names: &[&str], bodies: bool, budget: usize) -> Out {
        let ix = self.index()?;
        let (mut out, mut left) = (String::new(), vec![]);
        let fits = |out: &str, text: &str| out.is_empty() || out.len() + text.len() <= budget;
        for &name in names {
            let (text, served): (String, Vec<&Sym>) = if let Some(m) = self.resolve_module(&ix, name) {
                let body = store::module_text(&ix, m);
                if bodies && fits(&out, &body) {
                    (body, ix.syms.iter().filter(|s| s.module == m.path).collect())
                } else {
                    (store::module_outline(&ix, m, &self.broken()?, bodies), vec![])
                }
            } else {
                match self.resolve(&ix, name) {
                    Ok(i) => {
                        (format!("// ── {} ({}) ──\n{}\n\n", ix.syms[i].qname(), ix.syms[i].kind_text, ix.syms[i].text), vec![&ix.syms[i]])
                    }
                    Err(e) => (self.dependency(name).ok_or(e)?, vec![]),
                }
            };
            if left.is_empty() && fits(&out, &text) {
                out += &text;
                let printed: Vec<String> = match self.resolve_module(&ix, name) {
                    Some(m) => ix.syms.iter().filter(|s| s.module == m.path).map(Sym::qname).collect(),
                    None => self.resolve(&ix, name).map(|i| vec![ix.syms[i].qname()]).unwrap_or_default(),
                };
                printed.into_iter().for_each(mark_shown);
                served.into_iter().for_each(|s| reads::mark_served(&self.root, s));
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
            for &j in full.iter().filter(|&&j| !left.contains(&ix.syms[j].qname())) {
                reads::mark_served(&self.root, &ix.syms[j]);
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
        let mut out = format!("// ── {} ({}) ──\n{}\n", s.qname(), s.kind_text, s.text);
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
                format!("-- {q}\n{}\n", index::signature(ix, &ix.syms[u]))
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
                for &h in ix.uses[t].iter().filter(|&&h| is_test(ix, h) && !reach::is_test_fn(ix, h)) {
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

    /// `refs <name>...`: each symbol's users, in the order named (D-102).
    fn refs_all(&self, names: &[&str], budget: usize) -> Out {
        let mut out = String::new();
        for name in names {
            out += &self.refs(name)?;
        }
        Ok(lockdown::trim_output(&out, budget))
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
        let (exact, possible) = index::users_split(&ix, i);
        if ix.syms[i].kind == Kind::Type {
            let (tests, users): (Vec<usize>, Vec<usize>) = exact.into_iter().partition(|&j| is_test(&ix, j));
            out += &type_users(&ix, i, &users, &tests, usize::MAX);
            out += &possible_users(&ix, i, &possible);
            return Ok(out);
        }
        for u in exact {
            out += &ref_entry(&ix, u, i);
        }
        out += &possible_users(&ix, i, &possible);
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
                return Ok("Tests haven't passed in full yet, so there is no earlier state to compare with. Name the symbols: \
                     `impact <name>...`.\n"
                    .into());
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
                hits.push((name_hit, format!("{} ({})\n{}{}", s.qname(), s.kind_text, shown.join("\n"), more)));
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
        if language::is_rust(&self.root) {
            return self.submit_rust(target, raw);
        }
        let raw: String = raw.lines().filter(|l| !l.starts_with(drafts::BLOCK_HEADER)).collect::<Vec<_>>().join("\n");
        let formatted = self.lang().format(&raw)?;
        let parsed = self.lang().split(&formatted);
        let ix = self.index()?;
        let deps = self.dependency_modules();
        let (Target::Imports(m) | Target::Module(m)) = target;
        let module = self
            .resolve_module(&ix, m)
            .ok_or_else(|| {
                Failure(
                    format!(
                        "E1003 unknown module: {m}; create it with a patch `*** Add: {m}`: its module comment, \
                         imports and code, every line `+`."
                    ),
                    1,
                )
            })?
            .path
            .clone();
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
                            "E0102 the module text leaves out: {}. Nothing was saved. Removing a symbol must be explicit \
                             (solace delete <name>).",
                            missing.join(", ")
                        ));
                    }
                    let mut header = if parsed.header.is_empty() { header.clone() } else { parsed.header.clone() };
                    let (mut changed, mut added) = (vec![], vec![]);
                    for item in &parsed.items {
                        let test = ix.modules.iter().any(|m| m.path == module && m.dir == "test");
                        for need in missing_imports(&ix, &deps, &header, &item.text, test) {
                            let line = need.line();
                            header = add_import(&header, &line);
                            notes.push(format!("added `{line}` to {module}"));
                        }
                        if test {
                            notes.extend(
                                weak_asserts(&item.text, &aliases(&header)).into_iter().map(|w| format!("{module}.{}: {w}", item.name)),
                            );
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
            let before_warnings = self.warning_baseline()?;
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
                let now = self.index()?;
                let module_syms: Vec<String> = now.syms.iter().filter(|s| s.module == module).map(Sym::qname).collect();
                return fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}{}",
                    text.join("\n\n"),
                    self.callee_hint(&own_errors, &module_syms)? + &unknown_module_hint(self, &now, &deps, &own_errors)?
                ));
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
            out += &warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error).filter(own),
                diags.iter().filter(|d| !d.error).filter(|d| !own(d)),
            );
            out += &broken_report(&before_broken, &diags);
            Ok(out)
        })?;
        Ok(note_lines(&notes) + &out)
    }

    /// Runs `body` in one store transaction: committed, or rolled back with
    /// `created` files removed and the project files written again from the store.
    fn transaction<T>(&self, created: &[PathBuf], body: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
        // A savepoint, so it nests inside a command's outer transaction (D-77 point 7).
        self.db.execute_batch("SAVEPOINT solace_tx")?;
        match body() {
            Ok(out) => {
                self.db.execute_batch("RELEASE solace_tx")?;
                Ok(out)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK TO solace_tx; RELEASE solace_tx")?;
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
        self.submit_symbols_blocks(name, new_in, raw, false)
    }

    /// `formatted`: the blocks are already rustfmt's text, as a patch's are
    /// (`drafts::patch_blocks`), so a Rust submit formats nothing twice (D-168).
    fn submit_symbols_blocks(&self, name: Option<&str>, new_in: Option<&str>, raw: &str, formatted: bool) -> Out {
        if language::is_rust(&self.root) {
            return self.submit_symbols_rust(name, new_in, raw, formatted);
        }
        let ix = self.index()?;
        let deps = self.dependency_modules();
        let is_header = |l: &str| l.starts_with(drafts::BLOCK_HEADER);
        let blocks: Vec<(Option<String>, String)> =
            if raw.lines().filter(|l| is_header(l)).count() > 1 || (name.is_none() && new_in.is_none()) {
                let mut blocks: Vec<(Option<String>, String)> = vec![];
                for line in raw.lines() {
                    if let Some(h) = line.strip_prefix(drafts::BLOCK_HEADER) {
                        if h.starts_with("module ") {
                            return fail("E0101 a whole module goes through `submit <module>`.");
                        }
                        blocks.push((Some(block_name(h)), String::new()));
                    } else if let Some((_, text)) = blocks.last_mut() {
                        text.push_str(line);
                        text.push('\n');
                    } else if !line.trim().is_empty() {
                        return fail(format!(
                            "E0101 text for `submit` with no name must be blocks, each headed `{}<module>.<name> ──`.",
                            drafts::BLOCK_HEADER
                        ));
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
            let parsed = self.lang().split(&self.lang().format(text)?);
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
                    Err(_) if self.lang().valid_module_path(m) => (create(m), None),
                    Err(e) => return Err(e),
                },
                (None, Some(n)) => match self.resolve(&ix, n) {
                    Ok(i) => (ix.syms[i].module.clone(), Some(i)),
                    Err(e) => match n.rsplit_once('.') {
                        Some((m, _)) if self.resolve_module(&ix, m).is_some() => (module_of(m)?, None),
                        Some((m, _)) if self.lang().valid_module_path(m) => (create(m), None),
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
                self.db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, dir, header])?;
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
                for need in missing_imports(&ix, &deps, &header, &item.text, test) {
                    let line = need.line();
                    header = if need.alias.is_some() { add_import(&header, &line) } else { merge_import(&header, &need.module, &[]) };
                    notes.push(format!("added `{line}` to {module}"));
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
            let before_warnings = self.warning_baseline()?;
            let mut now = self.index()?;
            let mut diags = self.check_all(&now)?;
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
                now = self.index()?;
                diags = self.check_all(&now)?;
            }
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error && own(d)).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                return fail(format!(
                    "Not saved: the submitted text has errors.\n\n{}{}",
                    text.join("\n\n"),
                    self.callee_hint(&own_errors, &saved)?
                        + &self.same_name_hint(&now, &own_errors)
                        + &unknown_module_hint(self, &now, &deps, &own_errors)?
                ));
            }
            let after = now;
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
            out += &warnings_report(
                before_warnings.as_ref(),
                diags.iter().filter(|d| !d.error && own(d)),
                diags.iter().filter(|d| !d.error && !own(d)),
            );
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
                    if self.lang().head(&before.item()) == self.lang().head(item) {
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

    /// `delete <name>...`: deletes each named symbol in order (D-102). Nothing
    /// is deleted when any named symbol is still used by a symbol outside the
    /// list, so the check runs over the whole list first: uses only shrink as
    /// symbols go, never grow. A Rust store is cargo's to check (D-173).
    fn delete_all(&self, names: &[&str], budget: usize) -> Out {
        let ix = self.index()?;
        // The name-based graph overestimates, so it would refuse a safe delete
        // (D-173): a Rust store's uses are cargo's to find.
        if ix.rust {
            return self.delete_rust(names, budget);
        }
        let listed = names.iter().map(|name| self.resolve(&ix, name)).collect::<Result<Vec<_>, _>>()?;
        let used: Vec<String> = listed
            .iter()
            .map(|&i| {
                let users: Vec<String> = ix.used_by[i].iter().filter(|j| !listed.contains(j)).map(|&j| ix.syms[j].qname()).collect();
                Ok::<String, Failure>(if users.is_empty() {
                    String::new()
                } else {
                    format!("{} is still used by:\n  {}", ix.syms[i].qname(), users.join("\n  "))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let used: Vec<String> = used.into_iter().filter(|s| !s.is_empty()).collect();
        if !used.is_empty() {
            // a used symbol with a same-name copy: the merge that redirects its uses, once
            let hint = listed
                .iter()
                .find_map(|&i| {
                    let users = ix.used_by[i].iter().filter(|j| !listed.contains(j)).count();
                    let h = if users == 0 { String::new() } else { self.merge_hint(&ix, &ix.syms[i], users) };
                    (!h.is_empty()).then_some(h)
                })
                .unwrap_or_default();
            return if used.len() == 1 {
                fail(format!("E5001 {}{hint}", used[0]))
            } else {
                fail(format!("E5001 nothing deleted.\n{}{hint}", used.join("\n")))
            };
        }
        let before = self.broken()?;
        let mut out = String::new();
        for name in names {
            let ix = self.index()?;
            let i = self.resolve(&ix, name)?;
            self.retire(&ix.syms[i].id, "")?;
            // The module goes with its last symbol, unless another module imports it.
            let module = &ix.syms[i].module;
            let last = !ix.syms.iter().enumerate().any(|(j, s)| j != i && s.module == *module);
            let imported = ix.modules.iter().any(|m| ix.lang.imports(&m.header).iter().any(|imp| imp.module == *module));
            let retired = if last && !imported {
                self.db.execute("DELETE FROM modules WHERE path = ?1", [module])?;
                format!("; module {module} retired")
            } else {
                String::new()
            };
            out += &format!("Deleted {}{retired}.\n", ix.syms[i].qname());
        }
        let diags = self.check_all(&self.index()?)?;
        Ok(lockdown::trim_output(&out, budget) + &broken_report(&before, &diags))
    }

    #[cfg(test)]
    fn delete(&self, name: &str) -> Out {
        self.delete_all(&[name], usize::MAX)
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
            let more = if language::is_rust(&self.root) { format!("\n{}", cargo::ROUND_NOTE) } else { String::new() };
            fail(format!("{} errors, {warnings} warnings:\n\n{}\n{details}{more}", errors.len(), errors.join("\n\n")))
        }
    }

    /// `test [--all | <name>...]`: the tests that can see a change since they
    /// last passed, that failed or were never run, or that reach other
    /// processes, the network or foreign code; with names, those that reach
    /// them; `--all`, every test (D-44). A Rust project runs the chosen tests
    /// with `cargo test` by libtest name (D-170); Gleam's tests left out are
    /// made private in the projected files for the run, so gleeunit skips them.
    fn test(&self, args: &[&str]) -> Out {
        let rust = language::is_rust(&self.root);
        let ix = self.index()?;
        let named: Vec<usize> = operands(args).iter().map(|n| self.resolve(&ix, n)).collect::<Result<_, _>>()?;
        let all = args.contains(&"--all");
        let tests = reach::test_symbols(&ix);
        let revisions: Vec<String> = ix.syms.iter().map(|s| revision(&s.text)).collect();
        let outside = reach::outside(&ix);
        let mut rows: HashMap<String, (String, String)> = self
            .db
            .prepare("SELECT test, reach, result FROM test_runs")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()?;
        let (mut run, mut hashes) = (vec![], HashMap::new());
        let mut reached = HashSet::new();
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
            reached.extend(reach);
        }
        let untested = self.task_untested(&ix, &reached);
        UNTESTED.with(|u| *u.borrow_mut() = untested.clone());
        let next = next_step(untested.as_deref(), board::work_run(&self.root));
        let untested = untested.map_or(String::new(), |names| format!("No test reaches {names}.\n"));
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
            return Ok(head + &also + &untested + &next);
        }
        let files = self.materialize(&ix)?;
        let result = if rust {
            // Rust runs the chosen tests by libtest name, and the whole suite
            // when every test is due (D-170).
            let names: Vec<String> = run.iter().map(|&t| reach::libtest_name(&ix, t)).collect();
            let filters = if run.len() == tests.len() { vec![] } else { names };
            let args = cargo::test_args(&self.root, &filters);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            self.cargo(&argv, 600)
        } else {
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
                self.tool(&["test"])
            })();
            if !rewritten.is_empty() {
                self.materialize(&ix)?;
            }
            result
        };
        let (ok, output) = result?;
        let diags = if rust { cargo::diagnostics(&output, &files) } else { self.lang().diagnostics(&output, &files) };
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        if !errors.is_empty() {
            return fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
        }
        let failed = if rust { cargo::failures(&output) } else { self.lang().failures(&output) };
        // Record results only from a run that finished; passes only when every
        // failure it counted was attributed to a test.
        let summary = output.lines().find(|l| l.contains(" passed, "));
        let complete = if rust {
            let (passed, count) = cargo::totals(&output);
            if !ok && passed + count == 0 {
                // cargo stopped before the tests ran: its own words say why.
                return fail(format!("Tests did not run.\n\n{}", output.trim()));
            }
            ok || count == failed.len()
        } else {
            summary.is_some_and(|l| {
                let number =
                    |word: &str| l.split(word).next().and_then(|p| p.split_whitespace().last()).and_then(|n| n.parse::<usize>().ok());
                let count = if l.contains("no failures") { Some(0) } else { number(" failure") };
                ok || count == Some(failed.len())
            })
        };
        if rust || summary.is_some() {
            for &t in &run {
                let q = ix.syms[t].qname();
                let key = if rust { reach::libtest_name(&ix, t) } else { q.clone() };
                let result = match failed.get(&key) {
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
        if rust {
            for (name, (_, text)) in &failed {
                out += &format!("\n{name}:\n{text}\n");
            }
        } else {
            for line in self.lang().quiet(&output).lines() {
                let line = line.trim_start_matches('.');
                if line.trim().is_empty() || line.trim().starts_with("Checking ") {
                    continue;
                }
                out += &self.lang().relocate(line, &files);
                out.push('\n');
            }
        }
        out += &also;
        if ok { Ok(out + &untested + &next) } else { fail(out) }
    }

    /// D-152: this task's changed symbols in this project, outside test modules,
    /// that no test reaches (`reached`: what the tests reach), named for a reply;
    /// `None` when there are none or outside a board run.
    fn task_untested(&self, ix: &Index, reached: &HashSet<usize>) -> Option<String> {
        let (tid, _) = board::context()?;
        let t = scope::home(&self.root, &tid).and_then(|home| board::task(&home, &tid)).ok()?;
        let here = scope::dir(&scope::top(&self.root), &self.root);
        let mut covered = reached.clone();
        let by_id: HashMap<&str, usize> = ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        for &i in reached {
            let mut parent = ix.syms[i].parent.as_deref();
            while let Some(p) = parent {
                let Some(&k) = by_id.get(p) else { break };
                covered.insert(k);
                parent = ix.syms[k].parent.as_deref();
            }
        }
        let names: Vec<String> = board::changes_of(self, &t)
            .unwrap_or_default()
            .iter()
            .filter(|c| c["project"] == here)
            .filter_map(|c| c["entity"].as_str()?.strip_prefix("symbol:").map(String::from))
            .filter_map(|id| ix.syms.iter().position(|s| s.id == id))
            .filter(|&i| !is_test(ix, i) && !covered.contains(&i))
            .map(|i| reach::short(ix, i))
            .collect();
        if names.is_empty() {
            return None;
        }
        let more = if names.len() > 3 { format!(" and {} more", names.len() - 3) } else { String::new() };
        Some(format!("{}{more}", names[..names.len().min(3)].join(", ")))
    }

    /// Symbols changed since `test` last passed in full: (id, text then, `None`
    /// if new). `None` if it never has.
    fn since_pass(&self) -> Result<Option<Before>, Failure> {
        if !self.db.query_row("SELECT EXISTS(SELECT 1 FROM test_base)", [], |r| r.get::<_, bool>(0))? {
            return Ok(None);
        }
        let rows = self
            .db
            .prepare(
                "SELECT s.id, b.text FROM symbols s LEFT JOIN test_base b ON b.id = s.id \
                 WHERE s.live = 1 AND (b.text IS NULL OR b.text != s.text)",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(Some(rows))
    }

    /// Runs throwaway code against the current project and returns its output.
    /// A Gleam project's text is a module with `pub fn main()`, or statements
    /// that become its body; a Rust project's text is statements in a test in a
    /// scratch copy of the crate, placed in `module` when one is named. Project
    /// imports it uses are added. Nothing is saved.
    fn probe(&self, text: &str, from: Option<&str>, trace: &[&str], module: Option<&str>) -> Out {
        if language::is_rust(&self.root) {
            if let Some(name) = from {
                return fail(format!(
                    "E1006 --from is Gleam only: Rust probe runs the statements in `text`; {name} can be called from them."
                ));
            }
            if !trace.is_empty() {
                return fail("E1006 --trace is Gleam only: Rust has no call tracing at run time; print values with `println!` or `dbg!`.");
            }
            return rust_probe::run(self, text, module);
        }
        if module.is_some() {
            return fail("E1006 --in is Rust only: a Gleam probe reaches project modules with `import`.");
        }
        let ix = self.index()?;
        let deps = self.dependency_modules();
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
        for need in missing_imports(&ix, &deps, &code, &code, true) {
            code = format!("{}\n{code}", need.line());
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
            self.tool(&["run", "-m", PROBE])
        })();
        // Remove the probe and put traced modules back in every case.
        let _ = fs::remove_file(&file);
        if !traced.is_empty() {
            self.materialize(&ix)?;
        }
        let (ok, output) = result?;
        // Compiler warnings (mostly the probe's unused imports) are noise here.
        let mut out = self.lang().quiet(&output);
        if !trace.is_empty() {
            out = trace_report(&out);
        }
        if !ok {
            if out.contains("Unknown module value") {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out += "Hint: print a value with `echo value`.\n";
            }
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
        if !params(&ix.lang.head(&s.item()), &s.name).is_empty() {
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
                return fail(format!(
                    "E1006 {} can't be traced: `probe` is Gleam only and traces only functions written in Gleam.",
                    s.qname()
                ));
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

    /// The first Rust module file that no longer matches the store: its
    /// differing symbols, and the one command that repairs it.
    fn drift_error(&self, ix: &Index) -> Result<Option<Failure>, Failure> {
        if !language::is_rust(&self.root) {
            return Ok(None);
        }
        for m in ix.modules.iter().filter(|m| m.file.is_some()) {
            let rel = m.rel();
            let Ok(disk) = fs::read_to_string(self.root.join(&rel)) else { continue };
            let names = store::drift(ix, m, &disk);
            if !names.is_empty() {
                return Ok(Some(Failure(
                    format!(
                        "E0104 {rel} differs from the store in: {}. Nothing was saved. Run `format {}` to replace it \
                         with the store's text; the replaced file is kept under `.solace/outside/`.",
                        names.join(", "),
                        m.path
                    ),
                    1,
                )));
            }
        }
        Ok(None)
    }

    /// Accepts a module file's bytes as what Solace last wrote, so the next
    /// projection writes over a file left behind by an interrupted change.
    fn record_module_file(&self, m: &ModuleRow, bytes: &[u8]) -> Result<(), Failure> {
        let rel = m.rel();
        let mut written = outside::Written::load(&self.root);
        written.record(&rel, bytes);
        written.save()
    }

    /// Refuses while a module file differs from the store.
    fn refuse_drift(&self, ix: &Index) -> Result<(), Failure> {
        match self.drift_error(ix)? {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// The warnings of the last check; `None` before a check has ever run.
    fn warning_baseline(&self) -> Result<Option<HashSet<String>>, Failure> {
        let checked: i64 = self.db.query_row("SELECT COUNT(*) FROM warning_checks", [], |r| r.get(0))?;
        if checked == 0 {
            return Ok(None);
        }
        Ok(Some(self.db.prepare("SELECT key FROM warnings")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?))
    }

    /// Keeps this check's warnings, so a submit can name only the ones it adds.
    fn record_warnings(&self, diags: &[Diag]) -> Result<(), Failure> {
        let warnings: Vec<&Diag> = diags.iter().filter(|d| !d.error).collect();
        let modules: HashSet<&str> = warnings.iter().map(|d| warning_module(d)).collect();
        for module in modules {
            self.db.execute("DELETE FROM warnings WHERE module = ?1", params![module])?;
        }
        for d in warnings {
            self.db.execute("INSERT OR IGNORE INTO warnings(key, module) VALUES (?1, ?2)", params![warning_key(d), warning_module(d)])?;
        }
        self.db.execute("INSERT OR IGNORE INTO warning_checks(one) VALUES (1)", [])?;
        Ok(())
    }

    /// The direct dependency modules, read once until `gleam.toml`, the
    /// resolved package list, or a `.gleam` file under a path dependency's
    /// `src` changes.
    fn dependency_modules(&self) -> Rc<Vec<DependencyModule>> {
        let stamp = |path: &str| fs::metadata(self.root.join(path)).and_then(|m| m.modified()).ok();
        let mut paths = path_dependencies(&self.root);
        paths.sort();
        paths.dedup();
        let mut sources: Vec<(PathBuf, Option<std::time::SystemTime>)> = vec![];
        for path in paths {
            let mut files = vec![];
            if gleam::collect_gleam_files(&path.join("src"), &mut files).is_err() {
                continue;
            }
            sources.extend(files.into_iter().map(|file| {
                let stamp = fs::metadata(&file).and_then(|m| m.modified()).ok();
                (file, stamp)
            }));
        }
        sources.sort();
        sources.dedup();
        let key = (stamp("gleam.toml"), stamp("build/packages/packages.toml"), sources);
        let cached = self.deps.borrow().as_ref().filter(|(cached, _)| *cached == key).map(|(_, deps)| deps.clone());
        if let Some(deps) = cached {
            return deps;
        }
        let deps = Rc::new(dependency_modules(&self.root));
        *self.deps.borrow_mut() = Some((key, deps.clone()));
        deps
    }

    /// A store whose shared fields are set the same way by every constructor.
    fn new(db: Rc<Connection>, root: &Path, nested: bool, owns_dir: bool) -> Store {
        Store { db, root: root.into(), lang: language::detect(root), nested, owns_dir, deps: std::cell::RefCell::new(None) }
    }
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

fn is_test(ix: &Index, i: usize) -> bool {
    ix.test[i]
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
// a generic type's annotation such as `Box(Int)` also matches; check the position if that misleads.
fn sites(ix: &Index, j: usize, ty: usize, updates: bool) -> Vec<(usize, String)> {
    let ctors = ix.lang.constructors(&ix.syms[ty].text);
    let text = &ix.syms[j].text;
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(k, _)| k + 1)).collect();
    ref_lines(ix, j, ty)
        .into_iter()
        .filter(|(n, line)| {
            let at = starts[n - 1] + text[starts[n - 1]..].len() - text[starts[n - 1]..].trim_start().len();
            ctors.iter().any(|c| {
                line.match_indices(&format!("{c}(")).any(|(k, _)| {
                    let whole = !line[..k].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
                    whole && updates == language::split_args(&text[at + k + c.len() + 1..]).iter().any(|(_, a)| a.trim().starts_with(".."))
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

/// Where a function's body opens: the first `{` outside parentheses after
/// its `fn name(`.
fn fn_body_start(s: &Sym) -> Option<usize> {
    let at = s.text.find(&format!("fn {}(", s.name))?;
    Some(at + language::body_start(&s.text[at..])?)
}

/// A function's parameters from its signature: (label if any, name).
fn params(signature: &str, name: &str) -> Vec<(Option<String>, String)> {
    let Some(at) = signature.find(&format!("fn {name}(")) else { return vec![] };
    language::split_args(&signature[at + name.len() + 4..])
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
        "{renamed}\n\n{}fn {}({}) {{\n  echo #(\"solace-call\", \"{q}\"{args})\n  \
         let r = {inner}({})\n  echo #(\"solace-return\", \"{q}\", r)\n  r\n}}",
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

/// Imports needed for `alias.name` references the header doesn't import yet:
/// one project module or dependency by last path segment, the stdlib, or the
/// one module another module already imports as that alias. `test`: the code
/// is in a test module, which may also import test modules and dev modules.
fn missing_imports(ix: &Index, deps: &[DependencyModule], header: &str, text: &str, test: bool) -> Vec<MissingImport> {
    let aliases = aliases(header);
    let mut out: Vec<MissingImport> = vec![];
    for r in gleam::references(text) {
        let Ref::Qualified(alias, _) = r else {
            continue;
        };
        if aliases.contains(&alias) {
            continue;
        }
        let ImportMatch::One(need) = import_match(ix, deps, &alias, test) else {
            continue;
        };
        if !out.iter().any(|n| n.module == need.module && n.alias == need.alias) {
            out.push(need);
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
        return gleam::format(&added).map_or(added, |f| f.trim().to_string());
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
    gleam::format(&joined).map_or(joined, |f| f.trim().to_string())
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

/// Where a file position falls: (module, symbol and its first line). Cargo in
/// a workspace reports a path relative to the workspace root, so more than one
/// file can end with it; an exact match is the shallowest, and the shallowest
/// is the project's own file.
fn locate<'a>(files: &'a [FileMap], path: &str, line: usize) -> Option<(&'a str, Option<(&'a str, usize)>)> {
    let f =
        files.iter().filter(|f| f.file.to_string_lossy() == path || f.file.ends_with(path)).min_by_key(|f| f.file.components().count())?;
    let sym = f.ranges.iter().find(|r| r.0 <= line && line <= r.1).map(|r| (r.2.as_str(), r.0));
    Some((&f.module, sym))
}

fn warning_summary<'a>(diags: impl Iterator<Item = &'a Diag>) -> String {
    let lines: Vec<String> = diags.map(warning_text).collect();
    if lines.is_empty() { String::new() } else { format!("\nWarnings:\n{}\n", lines.join("\n")) }
}

/// What a change did to the broken set (D-46): `before` is the broken table
/// as it was. Fixed symbols are named; a symbol broken with the same errors as
/// before gets one line; new errors are printed in full, once per message, and
/// the other errors with that message as one line each (Spec 3 §6).
/// What a name is unique by: types and values are separate namespaces in Gleam.
fn name_key(name: &str, kind: Kind) -> (String, bool) {
    (name.to_string(), kind == Kind::Type)
}

/// Writes the serving port into a lock this server holds (D-80). A reader
/// trusts it only while the lock is held.
fn record_port(f: &fs::File, port: u16) {
    let _ = f.set_len(0);
    let _ = std::io::Write::write_all(&mut &*f, format!("{{\"port\":{port},\"pid\":{}}}", std::process::id()).as_bytes());
}

/// A search query without the quotes, backticks or apostrophes around it
/// (`find "Customer(.."`: the quotes delimit the query, run 18).
fn unquoted(query: &str) -> &str {
    ['"', '\'', '`'].iter().find_map(|&c| query.strip_prefix(c)?.strip_suffix(c)).filter(|q| !q.is_empty()).unwrap_or(query)
}

/// `n` and the noun, plural unless `n` is 1: "1 file", "2 files".
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// A project file's name as Solace lists it: `/`-separated, with `.` removed
/// and parent parts resolved for relative paths. A rooted path stays as
/// written, so an error echoes what the agent sent.
fn file_name(path: &Path) -> String {
    if path.components().any(|part| matches!(part, std::path::Component::RootDir | std::path::Component::Prefix(_))) {
        return path.to_string_lossy().into_owned();
    }
    let mut parts: Vec<String> = vec![];
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else {
                    parts.push("..".to_string());
                }
            }
            part => parts.push(part.as_os_str().to_string_lossy().into_owned()),
        }
    }
    parts.join("/")
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
    // new errors: one summary per shared cause, then the rest grouped by message
    let new: Vec<&Diag> = errors.iter().copied().filter(|d| !d.symbol.as_ref().is_some_and(|s| same.contains(&s))).collect();
    let (caused, rest): (Vec<&Diag>, Vec<&Diag>) = new.into_iter().partition(|d| error_cause(d).is_some());
    if !caused.is_empty() {
        out += &format!("\n{}\n", group_errors(&caused));
    }
    let mut groups: Vec<(String, &Diag, Vec<&Diag>)> = vec![];
    for d in rest {
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

/// Fixtures for every module's tests.
#[cfg(test)]
#[path = "testing.rs"]
mod testing;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

/// Whether the command is a process that changes files behind the store, so it
/// runs outside the command's outer transaction. Rust's `format` saves through
/// the store instead (D-180).
fn is_process(a: &[&str], root: &Path) -> bool {
    a.first().is_some_and(|c| PROCESSES.contains(c)) && !reads_only(a) && !(a[0] == "format" && language::is_rust(root))
}

#[cfg(test)]
thread_local! {
    /// A test's project root, in place of the working folder's.
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `Store::root` reading `root`, so a test drives a command
/// without changing the process's working folder.
#[cfg(test)]
pub(crate) fn with_root<T>(root: &Path, f: impl FnOnce() -> T) -> T {
    let old = TEST_ROOT.with(|r| r.replace(Some(root.to_path_buf())));
    let out = f();
    TEST_ROOT.with(|r| *r.borrow_mut() = old);
    out
}

/// `read_project` from an explicit working folder and owning project, so a test
/// never changes the process-wide working folder.
fn read_project_in(a: &[&str], here: &Path, own: Option<&Path>, dir: &str, budget: usize) -> Out {
    let command = a.first().copied().unwrap_or("");
    // From a project, or from a folder of projects with none at its root.
    let root = match workspace::enclosing(own.unwrap_or(here))? {
        Some(root) => root,
        None => own.map(Path::to_path_buf).ok_or_else(|| {
            Failure(
                "No Solace root here: migrate an existing project (`solace serve`, then Migrate this project) or \
                 make a new one with `solace create <name>`."
                    .into(),
                1,
            )
        })?,
    };
    let members = workspace::members(&root)?;
    let dir = &crate::project_paths::project_folder(&root, own, dir, &members);
    if !members.iter().any(|m| m == dir) {
        return lockdown::error(
            "E6007",
            dir,
            format!(
                "not a project under this Solace root: `--project` takes a folder from the Solace root; its projects: {}",
                members.join(", ")
            ),
        );
    }
    let target = scope::path(&root, dir);
    // A board run's agent must still be running, as for any command.
    let _guard = match (board::context(), own) {
        (Some((tid, rid)), Some(own)) => Some(board::agent_guard(&scope::home(own, &tid)?, &tid, &rid, &a, &[])?),
        _ => None,
    };
    let store = Store::peek(&target)?;
    let settings = lockdown::settings(&store.root)?;
    if let Some(out) = lockdown::dispatch(&store, &settings, &a, &mut || Ok(String::new()), None, budget) {
        return out;
    }
    // A Rust project has no `Language`; its store's symbols are read as any
    // other's (D-158), so only a project of plain files is refused.
    if store.lang.is_none() && !language::is_rust(&store.root) {
        let task = board::context().map(|(tid, _)| tid);
        return lockdown::plain(&store, task.as_deref(), command, a.get(1).copied());
    }
    let out = match a {
        ["view", rest @ ..] if !operands(rest).is_empty() => store.view(&operands(rest), rest.contains(&"--bodies"), budget),
        ["inspect", rest @ ..] if !operands(rest).is_empty() => store.inspect(&operands(rest), rest.contains(&"--context"), budget),
        ["refs", names @ ..] if !names.is_empty() => store.refs_all(names, budget),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        _ => usage(format!("{} --project {dir}: give what to read", a.join(" "))),
    };
    hinted(out, &store.root)
}

/// D-184: the next step after a passing test in a work run.
fn next_step(untested: Option<&str>, in_work_run: bool) -> String {
    if !in_work_run {
        return String::new();
    }
    untested.map_or_else(
        || "Next: if every part of the task is changed, finish with task done.\n".to_string(),
        |names| format!("Next: add a test for {names}; then, if every part of the task is changed, finish with task done.\n"),
    )
}

/// The shared cause of a Rust error that repeats at many sites: a key to group
/// by and the cause written for the summary, or `None` to print it in full.
fn error_cause(d: &Diag) -> Option<(String, String)> {
    let title = d.text.lines().next()?.trim();
    let code = title.strip_prefix("error[")?.split_once(']')?.0;
    if !matches!(code, "E0603" | "E0624" | "E0616" | "E0599") {
        return None;
    }
    let items: Vec<&str> = title.split('`').skip(1).step_by(2).collect();
    if items.is_empty() {
        return None;
    }
    let item = items[0];
    let cause = match code {
        "E0603" | "E0624" => format!("`{item}` is private"),
        "E0616" if items.len() > 1 => format!("`{item}` is private on `{}`", items[1]),
        "E0599" if items.len() > 1 => format!("`{item}` is missing on `{}`", items[1]),
        _ => return None,
    };
    let site = d.module.as_deref().unwrap_or("");
    Some((format!("{code} {site} {}", items.join(" ")), cause))
}

/// The fix for a shared cause, named once instead of at every site.
fn cause_fix(key: &str) -> &'static str {
    if key.starts_with("E0599") { "add the method or fix the call" } else { "make it `pub(crate)`" }
}

/// Errors of one cause print as one line naming the item and the fix, with the
/// first few sites and a count of the rest; every other error prints in full.
fn group_errors(errors: &[&Diag]) -> String {
    const SITE_CAP: usize = 6;
    let mut groups: Vec<(String, String, Vec<&Diag>)> = vec![];
    let mut plain: Vec<&str> = vec![];
    for d in errors {
        match error_cause(d) {
            Some((key, cause)) => match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.2.push(d),
                None => groups.push((key, cause, vec![d])),
            },
            None => plain.push(&d.text),
        }
    }
    let mut parts: Vec<String> = vec![];
    for (key, cause, ds) in groups {
        if ds.len() == 1 {
            parts.push(ds[0].text.clone());
            continue;
        }
        let mut part = format!("{cause} ({} places): {}", ds.len(), cause_fix(&key));
        for &d in ds.iter().take(SITE_CAP) {
            part += &format!("\n  at {}", place(d));
        }
        if ds.len() > SITE_CAP {
            let mut modules: Vec<&str> = vec![];
            for &d in ds.iter().skip(SITE_CAP) {
                if let Some(module) = site_module(d) {
                    modules.push(module);
                }
            }
            unique(&mut modules);
            part += &format!("\n… and {} more", ds.len() - SITE_CAP);
            if !modules.is_empty() {
                part += &format!(" in {}", modules.join(", "));
            }
        }
        parts.push(part);
    }
    parts.extend(plain.into_iter().map(String::from));
    parts.join("\n\n")
}

/// Names a repeated command's copy (`check (copy 2)`), so its section can be
/// found; a command that appears once is named as written.
fn failed_copy(command: &str, commands: &[&str], at: usize) -> String {
    if commands.iter().filter(|c| bare_command(c) == command).count() < 2 {
        return command.to_string();
    }
    let copy = commands[..=at].iter().filter(|c| bare_command(c) == command).count();
    format!("{command} (copy {copy})")
}

/// One warning as a submit prints it.
fn warning_text(d: &Diag) -> String {
    format!("  {}: {}", warning_place(d), warning_line(d))
}

/// A warning's place and its `warning:` line. The rendered `at` line is left
/// out: it moves when unrelated code above changes.
fn warning_key(d: &Diag) -> String {
    format!("{}\n{}", warning_place(d), warning_line(d))
}

fn warning_place(d: &Diag) -> &str {
    d.symbol.as_deref().or(d.module.as_deref()).unwrap_or("project")
}

fn warning_line(d: &Diag) -> &str {
    d.text.lines().find(|l| l.trim_start().starts_with("warning:")).unwrap_or("compiler warning").trim()
}

/// Warnings outside the saved symbols that the last check did not have.
fn new_warnings_summary<'a>(before: Option<&HashSet<String>>, diags: impl Iterator<Item = &'a Diag>) -> String {
    let Some(before) = before else { return String::new() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut lines: Vec<String> = vec![];
    for d in diags.filter(|d| !d.error) {
        let key = warning_key(d);
        if !before.contains(&key) && seen.insert(key) {
            lines.push(warning_text(d));
        }
    }
    if lines.is_empty() { String::new() } else { format!("\nNew warnings elsewhere:\n{}\n", lines.join("\n")) }
}

/// The warning sections of a submit reply, with their footer once.
fn warnings_report<'a>(
    before: Option<&HashSet<String>>,
    own: impl Iterator<Item = &'a Diag>,
    elsewhere: impl Iterator<Item = &'a Diag>,
) -> String {
    let own = warning_summary(own);
    let elsewhere = new_warnings_summary(before, elsewhere);
    if own.is_empty() && elsewhere.is_empty() {
        String::new()
    } else {
        format!("{own}{elsewhere}Use `check --warnings` for full diagnostics.\n")
    }
}

fn warning_module(d: &Diag) -> &str {
    d.module.as_deref().unwrap_or("project")
}

#[derive(Clone, Debug, PartialEq)]
struct DependencyModule {
    path: String,
    test: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct MissingImport {
    module: String,
    alias: Option<String>,
}

enum ImportMatch {
    One(MissingImport),
    Many(Vec<MissingImport>),
    Unavailable(Vec<String>),
    None,
}

/// The modules of the project's direct dependencies, and whether only test
/// code may use them. Indirect packages in `build/packages` don't count.
fn dependency_modules(root: &Path) -> Vec<DependencyModule> {
    let Ok(text) = fs::read_to_string(root.join("gleam.toml")) else { return vec![] };
    let Ok(v) = text.parse::<toml::Value>() else { return vec![] };
    let mut out: Vec<DependencyModule> = vec![];
    for (table, test) in [("dependencies", false), ("dev-dependencies", true), ("dev_dependencies", true)] {
        let Some(deps) = v.get(table).and_then(|d| d.as_table()) else { continue };
        for (name, spec) in deps {
            let dir = match spec.get("path").and_then(|p| p.as_str()) {
                Some(path) => root.join(path),
                None => root.join("build/packages").join(name),
            };
            let mut files = vec![];
            if gleam::collect_gleam_files(&dir.join("src"), &mut files).is_err() {
                continue;
            }
            for file in files {
                let Ok(rel) = file.strip_prefix(dir.join("src")) else { continue };
                let path = rel.with_extension("").to_string_lossy().replace('\\', "/");
                match out.iter_mut().find(|m| m.path == path) {
                    Some(one) => one.test &= test,
                    None => out.push(DependencyModule { path, test }),
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn import_available(module: &str, ix: &Index, deps: &[DependencyModule], test: bool) -> bool {
    if let Some(m) = ix.modules.iter().find(|m| m.path == module) {
        return m.dir == "src" || test;
    }
    if let Some(d) = deps.iter().find(|d| d.path == module) {
        return !d.test || test;
    }
    module.strip_prefix("gleam/").is_some_and(|name| STDLIB.contains(&name))
}

fn import_match(ix: &Index, deps: &[DependencyModule], qualifier: &str, test: bool) -> ImportMatch {
    let project = modules_named(ix.modules.iter().filter(|m| m.dir == "src" || test).map(|m| m.path.as_str()), qualifier);
    match project.as_slice() {
        [need] => return ImportMatch::One(need.clone()),
        [] => {}
        _ if STDLIB.contains(&qualifier) => {
            return ImportMatch::One(MissingImport { module: format!("gleam/{qualifier}"), alias: None });
        }
        many => return ImportMatch::Many(many.to_vec()),
    }
    if STDLIB.contains(&qualifier) {
        return ImportMatch::One(MissingImport { module: format!("gleam/{qualifier}"), alias: None });
    }
    let dependency = modules_named(deps.iter().filter(|d| !d.test || test).map(|d| d.path.as_str()), qualifier);
    match dependency.as_slice() {
        [need] => return ImportMatch::One(need.clone()),
        [] => {}
        many => return ImportMatch::Many(many.to_vec()),
    }
    let mut found: Vec<MissingImport> = vec![];
    for m in &ix.modules {
        for import in gleam::imports(&m.header) {
            if import.alias != qualifier || !import_available(&import.module, ix, deps, test) {
                continue;
            }
            let need = MissingImport { module: import.module, alias: Some(qualifier.to_string()) };
            if !found.iter().any(|n| n.module == need.module) {
                found.push(need);
            }
        }
    }
    if found.len() == 1 {
        return ImportMatch::One(found.remove(0));
    }
    if found.len() > 1 {
        return ImportMatch::Many(found);
    }
    let mut unavailable: Vec<String> = modules_named(ix.modules.iter().map(|m| m.path.as_str()), qualifier)
        .into_iter()
        .chain(modules_named(deps.iter().map(|d| d.path.as_str()), qualifier))
        .map(|need| need.module)
        .collect();
    unavailable.sort();
    unavailable.dedup();
    if unavailable.is_empty() { ImportMatch::None } else { ImportMatch::Unavailable(unavailable) }
}

fn unknown_module(text: &str) -> Option<&str> {
    if let Some(qualifier) =
        quoted_after(text, "No module has been found with the name `").or_else(|| quoted_after(text, "Unknown module `"))
    {
        return Some(qualifier);
    }
    let title = text.lines().any(|line| line.trim().trim_start_matches("error: ").trim() == "Unknown module");
    if title { quoted_after(text, "The module `") } else { None }
}

fn unknown_module_hint(s: &Store, ix: &Index, deps: &[DependencyModule], errors: &[&Diag]) -> Result<String, Failure> {
    for d in errors {
        if d.text.contains("Unknown module value") {
            continue;
        }
        let Some(qualifier) = unknown_module(&d.text) else { continue };
        let Some(module) = d.module.as_deref() else { continue };
        let header = s.header(module)?;
        if aliases(&header).contains(qualifier) {
            continue;
        }
        let test = ix.modules.iter().any(|m| m.path == module && m.dir == "test");
        let current = match header.trim() {
            "" => String::new(),
            h => format!("\n\nIts current header is:\n\n{h}"),
        };
        match import_match(ix, deps, qualifier, test) {
            ImportMatch::One(need) => {
                return Ok(format!(
                    "\n\nTo add it, run `submit --imports {module}` with the module's current imports plus the new line \
                     `{}`.{current}",
                    need.line()
                ));
            }
            ImportMatch::None => {
                return Ok(format!(
                    "\n\nTo add it, run `submit --imports {module}` with the module's current imports plus the new line \
                     `import {qualifier}`.{current}"
                ));
            }
            ImportMatch::Unavailable(paths) => {
                let names: Vec<String> = paths.iter().map(|p| format!("`{p}`")).collect();
                return Ok(format!(
                    "\n\n`{qualifier}` is only available to test code ({}), so it can't be imported into {module}.",
                    names.join(", ")
                ));
            }
            ImportMatch::Many(needs) => {
                let lines: Vec<String> = needs.iter().map(|n| format!("`{}`", n.line())).collect();
                return Ok(format!(
                    "\n\n`{qualifier}` matches more than one module: {}; nothing was added. To pick one, run \
                     `submit --imports {module}` with the module's current imports plus one of those lines.{current}",
                    lines.join(", ")
                ));
            }
        }
    }
    Ok(String::new())
}

impl MissingImport {
    fn line(&self) -> String {
        match &self.alias {
            Some(alias) => format!("import {} as {alias}", self.module),
            None => format!("import {}", self.module),
        }
    }
}

/// When `gleam.toml`, the resolved package list, or a `.gleam` file under a
/// path dependency's `src` last changed.
type DependencyKey = (Option<std::time::SystemTime>, Option<std::time::SystemTime>, Vec<(PathBuf, Option<std::time::SystemTime>)>);

/// Every module whose last path segment is `qualifier`.
fn modules_named<'a>(paths: impl Iterator<Item = &'a str>, qualifier: &str) -> Vec<MissingImport> {
    paths.filter(|path| last_segment(path) == qualifier).map(|path| MissingImport { module: path.to_string(), alias: None }).collect()
}

fn quoted_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &text[text.find(marker)? + marker.len()..];
    rest.find('`').map(|end| &rest[..end])
}

/// The Solace root and the folders of the other projects under it, the run's
/// task's projects first, then the rest: `None` when `here` is in no Solace root.
fn other_projects(here: &Path) -> Option<(PathBuf, Vec<String>)> {
    let top = workspace::enclosing(here).ok().flatten()?;
    let mine = scope::dir(&top, here);
    let mut order = scope::of_run(here).unwrap_or_default();
    let rest: Vec<String> = workspace::members(&top).ok()?.into_iter().filter(|m| !order.contains(m)).collect();
    order.extend(rest);
    let others = order.into_iter().filter(|d| *d != mine).collect();
    Some((top, others))
}

/// The first project other than `here` with a module named `name` (D-123):
/// the run's task's projects first, then the others under the Solace root.
pub(crate) fn module_elsewhere(here: &Path, name: &str) -> Option<String> {
    let (top, dirs) = other_projects(here)?;
    dirs.into_iter().find(|d| {
        let Some(s) = Store::peek(&scope::path(&top, d)).ok() else { return false };
        let Ok(ix) = s.index() else { return false };
        ix.modules.iter().any(|m| m.path == name)
    })
}

/// The directories the manifest names as path dependencies.
fn path_dependencies(root: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(root.join("gleam.toml")) else { return vec![] };
    let Ok(v) = text.parse::<toml::Value>() else { return vec![] };
    ["dependencies", "dev-dependencies", "dev_dependencies"]
        .into_iter()
        .filter_map(|table| v.get(table).and_then(|deps| deps.as_table()))
        .flat_map(|deps| deps.values())
        .filter_map(|spec| spec.get("path").and_then(|p| p.as_str()))
        .map(|path| root.join(path))
        .collect()
}

/// The name of the probe text's line range in its generated module.
pub(crate) const PROBE_RANGE: &str = "probe";

/// The name of the generated module's lines before the probe text.
pub(crate) const PROBE_START_RANGE: &str = "probe start";

/// The name of the generated module's lines after the probe text.
pub(crate) const PROBE_END_RANGE: &str = "probe end";

/// The indentation the probe's wrapper adds to each of its lines.
pub(crate) const PROBE_INDENT: &str = "        ";

/// The Rust project's commands that run through cargo, and that a language's
/// unavailable toolchain refuses with E6013.
const CARGO_TOOLS: &[&str] = &["check", "test", "submit", "delete", "probe", "extract", "merge"];

/// One user's entry under `refs`, printed once per reply.
fn ref_entry(ix: &Index, user: usize, target: usize) -> String {
    let q = ix.syms[user].qname();
    let mut out =
        if shown_earlier(&q) { format!("-- {q} (above)\n") } else { format!("-- {q}\n{}\n", index::signature(ix, &ix.syms[user])) };
    out += &call_lines(ix, user, target);
    mark_shown(q);
    out
}

/// The `refs` section for uses only type inference could settle.
fn possible_users(ix: &Index, target: usize, users: &[usize]) -> String {
    if users.is_empty() {
        return String::new();
    }
    let mut out = format!("\n== Possible uses ({}): only type inference could settle these ==\n", users.len());
    for &u in users {
        out += &ref_entry(ix, u, target);
    }
    out
}

/// A site's module, falling back to the part of its name before the last dot.
fn site_module(d: &Diag) -> Option<&str> {
    d.module.as_deref().filter(|m| !m.is_empty()).or_else(|| d.symbol.as_deref()?.rsplit_once('.').map(|(m, _)| m))
}
