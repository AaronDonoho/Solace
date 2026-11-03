use std::path::Path;
use std::path::PathBuf;
use std::env;
use std::fs;
use std::io::Read;
use crate::failure::{usage, Failure, Out};
use super::projects::{hinted, read_project, scoped};
use super::{HOST, PROJECT_ONLY, READS};
#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
use crate::lockdown::commands::trim_output;
use crate::lockdown::settings::settings;
use crate::lockdown::settings::settings_file;
use crate::lockdown::settings::cargo_settings_changed;
use crate::lockdown::files::edited;
use crate::board::task_model::flow;
use crate::board::reviews::task_diff;
use crate::board::briefs::task_peek;
use crate::board::briefs::remember;
use crate::board::server::serve;
use crate::board::reviews::record_verification;
use crate::board::setup::system;
use crate::board::runs::context;
use crate::board::runs::strategy_context;
use crate::board::runs::task_command;
use crate::board::settings::propose_settings;

/// Removes `flag` and its value from `args`; a flag without a value is a usage error.
pub(crate) fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, Failure> {
    let Some(i) = args.iter().position(|a| a == flag) else { return Ok(None) };
    if i + 1 >= args.len() {
        return usage(format!("{flag} needs a value"));
    }
    args.remove(i);
    Ok(Some(args.remove(i)))
}

/// `input` supplies the new code for submit commands; `old`, the text that
/// `submit <name> --replace` replaces (MCP only); `budget`, the bytes left
/// for this reply.
pub(crate) fn run_options(args: &[String], input: &mut dyn FnMut() -> Out, old: Option<&str>, budget: usize, options: &[String]) -> Out {
    if let Some(out) = early_options(args, input, budget)? {
        return out;
    }
    let (args, cross, dest) = match resolve_scope(args, budget)? {
        Scoped::Reply(out) => return out,
        Scoped::Run { args, cross, dest } => (args, cross, dest),
    };
    let project = cross.as_deref().map(|p| crate::scope::dir(&crate::scope::top(p), p));
    let here = match &cross {
        Some(dir) => dir.clone(),
        None => env::current_dir()?,
    };
    let Ok(own) = crate::Store::root() else {
        flow(&here)?;
        return crate::drafts::with_project(project.as_deref(), || run_inner(&here, &here, &args, input, old, budget));
    };
    let mut session = crate::command_tx::session(args, cross, dest, project, here, own)?;
    let a: Vec<&str> = session.args.iter().map(String::as_str).collect();
    // A `task` command holds no project lock (D-77 point 5), so it runs before
    // recovery and absorbing: those need the lock, and a live command's records
    // must not be taken for a crashed one's. Its changes are committed rows, so
    // it reads them without either.
    if let Some(out) = board_command(&session.home, &session.context, &a, &mut *input, budget, options, &mut session.guard) {
        return out;
    }
    // Every guard here holds the lock of `store`'s project.
    // An extract stopped between its projects' commits is undone first, under
    // every lock of its projects (D-135).
    let noticed = crate::command_tx::recover_all(&session.store, &session.home, session.dest.as_ref(), &session.deps, &session.guard)?;
    // A run's settings write is a proposal the human approves (D-38); a Rust
    // project's Cargo.toml is one only when it touches a protected key (D-158
    // point 4). Its text is read here, once, and replayed to the command.
    let settings = settings_proposal(&session.store, &session.home, &a, session.context.as_ref(), &mut *input, old, &mut session.guard)?;
    let settings_write = match settings {
        SettingsWrite::None => None,
        SettingsWrite::Text(text) => Some(text),
        SettingsWrite::Proposed(out) => return out,
    };
    let whole = settings_write.is_some();
    // A file patch (`write` with no path, D-104), or a kept file patch (D-120)
    // a `--replace` edits first, names its files in `text`: it is read once, for
    // the snapshots and for the command.
    let (read_once, files) = replay_input(&session.store, &a, session.context.is_some(), settings_write, &mut *input, old)?;
    let mut replay = || -> Out { Ok(read_once.clone().unwrap_or_default()) };
    let input: &mut dyn FnMut() -> Out = if read_once.is_some() { &mut replay } else { input };
    let changes = crate::command_tx::changes(&session.store, &session.dest, &session.deps, &a, session.context.as_ref(), &files)?;
    // A code change and its undo rows commit together (D-77 point 7): one outer
    // transaction around the command and `record`. A process runs outside it, so
    // a long `run` holds the store's write lock only for the rows; the ahead
    // record in `.solace/pending/` covers a crash while the process runs.
    // Rust `format` saves through the store, so it belongs inside the outer
    // transaction; only a process that changes files behind the store stays out.
    let processes = is_process(&a, &session.store.root);
    let mut transactions = crate::command_tx::begin_transactions(&session, &changes, &a, processes)?;
    // A settings write's text already applied `old`; replay it whole (D-158 point 4).
    let result = run_command(&session, &a, input, old, budget, changes.before().is_some(), whole)?;
    if changes.before().is_some() && processes {
        session.store.db.execute_batch("BEGIN IMMEDIATE")?;
    }
    // A process the command ran (`format`, a declared command) may have changed source: that's this command's change.
    let unread = if processes { crate::outside::absorb(&session.store, crate::outside::Mode::Command)? } else { String::new() };
    if let Some(before) = changes.before() {
        let after = crate::command_tx::snapshot_change(&session.store, &a, &files)?;
        crate::command_tx::commit(&session, &changes, before, &after, &mut transactions.ahead)?;
    }
    match hinted(result, &session.store.root) {
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
        return crate::lockdown::error("E7010", command, why);
    }
    let top = crate::workspace::enclosing(&env::current_dir()?)?.ok_or_else(|| Failure("No Solace root here.".into(), 1))?;
    crate::board::strategy_guard(&top, rid, &a)?;
    if command == "strategy" {
        return crate::board::strategy_command(&top, rid, &a, input, budget);
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

/// Run a process that may change source; what it changed is read in after.
const PROCESSES: &[&str] = &["format", "build", "run"];

/// Results recorded as the task's verification.
const VERIFIES: &[&str] = &["check", "test", "build"];

/// Need symbol-held source (Gleam or Rust): refused in a project of plain files.
const GLEAM: &[&str] =
    &["submit", "drafts", "view", "inspect", "refs", "impact", "find", "delete", "extract", "merge", "check", "test", "probe"];

/// Options the reference gives each command: `--x` words in its syntax lines.
fn options_of(command: &str) -> Vec<&'static str> {
    let mut out = vec![];
    for line in crate::reference::REFERENCE.lines() {
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
pub(crate) fn unknown_option(a: &[&str]) -> Option<Out> {
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

pub(crate) fn run_inner(
    root: &Path,
    here: &Path,
    args: &[String],
    input: &mut dyn FnMut() -> Result<String, Failure>,
    old: Option<&str>,
    budget: usize,
) -> Out {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(a.as_slice(), [] | ["help"] | ["--help"]) {
        return Ok(crate::reference::REFERENCE.to_string());
    }
    if let Some(out) = unknown_option(&a) {
        return out;
    }
    if let ["agent-config", agent, dir] = a.as_slice() {
        return crate::lockdown::agent_config(
            agent,
            Path::new(dir),
            &env::current_exe()?,
            system()?["task_timeout"].as_u64().unwrap_or(3600),
        );
    }
    if a[0] == "create" {
        return crate::workspace::create(here, &a[1..]);
    }

    // run_options has put back any interrupted file write (lockdown::recover).
    let store = crate::Store::at(root)?;
    let settings = settings(&store.root)?;
    let task = context().map(|(tid, _)| tid);
    if CARGO_TOOLS.contains(&a[0])
        && let Some(id) = crate::language::id(&store.root)
        && let Some(why) = crate::language::reason(id)?
    {
        return crate::lockdown::error("E6013", a[0], why);
    }
    if let Some(out) = crate::lockdown::dispatch(&store, &settings, &a, input, old, budget) {
        return out;
    }
    // drafts include kept file patches (D-120), so a plain-files project lists, reads and drops them
    if GLEAM.contains(&a[0]) && a[0] != "drafts" && !matches!(a.as_slice(), ["view", "--draft", _]) && store.lang.is_none() {
        if !crate::language::is_rust(&store.root) {
            return crate::lockdown::plain(&store, task.as_deref(), a[0], a.get(1).copied());
        }
        // A Rust project's tools are cargo's (D-158, D-168, D-173, D-185, D-207):
        // `check` and `test` run, `submit` saves items by name and `delete`
        // removes them, `extract` moves items and `merge` folds duplicates, each
        // checked with cargo; `drafts` is language-neutral.
        match a[0] {
            tool if CARGO_TOOLS.contains(&tool) && crate::language::cargo::installed() => {}
            tool if CARGO_TOOLS.contains(&tool) => {
                return crate::lockdown::error("E6013", a[0], crate::language::cargo::MISSING);
            }
            // Reads reach a Rust store's symbols by name, as they do Gleam's
            // (D-158); `refs` and `impact` answer from its reference graph (D-170).
            "view" | "inspect" | "find" | "refs" | "impact" => {}
            _ => return crate::lockdown::plain(&store, task.as_deref(), a[0], a.get(1).copied()),
        }
    }
    match a.as_slice() {
        ["session", "start", "--task"] => {
            let task = match input() {
                Ok(text) if !text.trim().is_empty() => text,
                result => match context() {
                    Some((tid, _)) => crate::board::briefs::task_text(&crate::scope::task_in(&store.root, &tid)?.1),
                    None => result?,
                },
            };
            store.session_task(task.trim_end())
        }
        ["session", "start", "--task", task @ ..] => store.session_task(&task.join(" ")),
        ["submit", ..] => crate::drafts::submit(&store, &a, input, old),
        ["drafts", "drop", id] => crate::drafts::drop_one(&store, id),
        ["drafts"] => crate::drafts::list(&store),
        ["view", "--draft", id] => crate::drafts::view(&store, id),
        ["session", rest @ ..] => store.session(&operands(rest.strip_prefix(&["start"]).unwrap_or(rest))),
        ["view", rest @ ..] if !operands(rest).is_empty() => {
            store.view_with_header(&operands(rest), rest.contains(&"--bodies"), rest.contains(&"--header"), budget)
        }
        ["inspect", rest @ ..] if !operands(rest).is_empty() => {
            let (names, lines) = inspect_options(rest)?;
            store.inspect(&names, rest.contains(&"--context"), lines, budget)
        }
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
        Ok(text) => Ok(trim_output(&("exit code: 0\n".to_string() + &text), budget)),
        Err(Failure(text, code)) => Err(Failure(trim_output(&format!("exit code: {code}\n{text}"), budget), code)),
    }
}

/// Arguments that aren't flags (`--scope` just introduces them).
pub(crate) fn operands<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| !a.starts_with("--")).collect()
}

pub(crate) fn stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

// --- the store --------------------------------------------------------------

/// Whether the command is a process that changes files behind the store, so it
/// runs outside the command's outer transaction. Rust's `format` saves through
/// the store instead (D-180).
pub(crate) fn is_process(a: &[&str], root: &Path) -> bool {
    a.first().is_some_and(|c| PROCESSES.contains(c))
        && !crate::command_tx::reads_only(a)
        && !(a[0] == "format" && crate::language::is_rust(root))
}

/// Runs `f` with `Store::root` reading `root`, so a test drives a command
/// without changing the process's working folder.
#[cfg(test)]
pub(crate) fn with_root<T>(root: &Path, f: impl FnOnce() -> T) -> T {
    let old = crate::TEST_ROOT.with(|r| r.replace(Some(root.to_path_buf())));
    let out = f();
    crate::TEST_ROOT.with(|r| *r.borrow_mut() = old);
    out
}

/// D-184: the next step after a passing test in a work run.
pub(crate) fn next_step(untested: Option<&str>, in_work_run: bool) -> String {
    if !in_work_run {
        return String::new();
    }
    untested.map_or_else(
        || "Next: if every part of the task is changed, finish with task done.\n".to_string(),
        |names| format!("Next: add a test for {names}; then, if every part of the task is changed, finish with task done.\n"),
    )
}

/// The Rust project's commands that run through cargo, and that a language's
/// unavailable toolchain refuses with E6013.
const CARGO_TOOLS: &[&str] = &["check", "test", "submit", "delete", "probe", "extract", "merge"];

/// The command's arguments after `--project`, and the project it names, or a
/// reply already made while reading another project.
enum Scoped {
    Run { args: Vec<String>, cross: Option<PathBuf>, dest: Option<PathBuf> },
    Reply(Out),
}

/// A settings write that `write` found: none, text to replay, or a proposal
/// the human approves (D-38).
enum SettingsWrite {
    None,
    Text(String),
    Proposed(Out),
}

fn early_options(args: &[String], input: &mut dyn FnMut() -> Out, budget: usize) -> Result<Option<Out>, Failure> {
    if let Some(rid) = strategy_context() {
        return Ok(Some(strategy_run(&rid, args, input, budget)));
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.first() == Some(&"serve") {
        if context().is_some() {
            return Ok(Some(crate::lockdown::error("E7010", "serve", "host command")));
        }
        let port = match a.as_slice() {
            ["serve"] => 3030,
            ["serve", "--port", p] => p.parse().map_err(|_| Failure("invalid port".into(), 2))?,
            _ => return Ok(Some(usage("serve [--port N]"))),
        };
        return Ok(Some(serve(crate::workspace::find(&env::current_dir()?)?, port)));
    }
    if a.first() == Some(&"migrate") {
        if context().is_some() {
            return Ok(Some(crate::lockdown::error("E7010", "migrate", "host command")));
        }
        return Ok(Some(crate::migrate::command(&a[1..])));
    }
    if a.first() == Some(&"strategy") {
        return Ok(Some(crate::lockdown::error(
            "E7001",
            "strategy",
            "strategy commands work only in a strategy run started from the Strategy page",
        )));
    }
    if a.first() == Some(&"task") && context().is_none() {
        return Ok(Some(crate::lockdown::error(
            "E7010",
            "task",
            "task commands work only in a run started from the task board; this session has no task",
        )));
    }
    if a.as_slice() == ["session", "remember"] {
        let Some((tid, rid)) = context() else {
            return Ok(Some(crate::lockdown::error(
                "E7010",
                "session remember",
                "session remember works only in a run started from the task board",
            )));
        };
        let top = crate::scope::top(&crate::Store::root()?);
        return Ok(Some(remember(&top, &tid, &rid, budget)));
    }
    Ok(None)
}

fn resolve_scope(args: &[String], budget: usize) -> Result<Scoped, Failure> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let (args, cross) = match a.iter().position(|x| *x == "--project") {
        Some(i) => {
            let dir =
                a.get(i + 1).copied().ok_or_else(|| Failure("--project needs a project folder; session start lists them".into(), 2))?;
            let rest = [&args[..i], &args[i + 2..]].concat();
            if rest.is_empty() {
                return Ok(Scoped::Reply(usage(PROJECT_ONLY.trim_end())));
            }
            match scoped(dir, &rest)? {
                Some(target) => (rest, Some(target)),
                None => return Ok(Scoped::Reply(read_project(dir, &rest, budget))),
            }
        }
        None => (args.to_vec(), None),
    };
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // `move` or `extract` with `--to-project <dir>`: the destination's project,
    // which the task must cover too (D-84, D-103); outside a board run a command
    // changes one project. A `merge`'s is the survivor's, which it only reads (D-117).
    let dest = match a.iter().position(|x| *x == "--to-project") {
        Some(i) if matches!(a.first(), Some(&"move" | &"extract" | &"merge")) => {
            let dir =
                a.get(i + 1).copied().ok_or_else(|| Failure("--to-project needs a project folder; session start lists them".into(), 2))?;
            let Some(p) = scoped(dir, &args)? else {
                return Ok(Scoped::Reply(crate::lockdown::error(
                    "E6007",
                    dir,
                    format!("`{} --to-project` works only in a board run whose task covers both projects", a[0]),
                )));
            };
            let here = match &cross {
                Some(c) => c.clone(),
                None => crate::Store::root()?,
            };
            Some(fs::canonicalize(&p)?).filter(|p| fs::canonicalize(&here).ok().as_ref() != Some(p))
        }
        _ => None,
    };
    Ok(Scoped::Run { args, cross, dest })
}

fn board_command(
    home: &crate::Store,
    context: &Option<(String, String)>,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    budget: usize,
    options: &[String],
    guard: &mut Vec<fs::File>,
) -> Option<Out> {
    let (tid, rid) = context.as_ref()?;
    if a.starts_with(&["task", "diff"]) {
        return Some(task_diff(home, tid, &a[2..], budget));
    }
    // In a task run, `impact` with no names is the task's own change list (D-102).
    if a.len() == 1 && a[0] == "impact" {
        return Some(task_diff(home, tid, &[], budget));
    }
    if a.starts_with(&["task", "peek"]) {
        return Some(task_peek(home, tid, &a[2..], budget));
    }
    if a.first() == Some(&"task") {
        return Some(task_command(home, tid, rid, a, input, options, std::mem::take(guard)));
    }
    None
}

fn settings_proposal(
    store: &crate::Store,
    home: &crate::Store,
    a: &[&str],
    context: Option<&(String, String)>,
    input: &mut dyn FnMut() -> Out,
    old: Option<&str>,
    guard: &mut Vec<fs::File>,
) -> Result<SettingsWrite, Failure> {
    let Some((tid, rid)) = context else {
        return Ok(SettingsWrite::None);
    };
    if a.first() != Some(&"write") {
        return Ok(SettingsWrite::None);
    }
    let Some(file) =
        a.get(1).map(|p| crate::file_name(Path::new(p))).filter(|p| settings_file(&store.root, p) || *p == "Cargo.toml")
    else {
        return Ok(SettingsWrite::None);
    };
    let current = fs::read(store.root.join(&file)).unwrap_or_default();
    let proposed = edited(&current, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old)?;
    if settings_file(&store.root, &file) || cargo_settings_changed(&current, &proposed) {
        let reason = crate::REASON.with(|r| r.borrow().clone());
        return Ok(SettingsWrite::Proposed(propose_settings(
            home,
            &store.root,
            tid,
            rid,
            &file,
            &current,
            &proposed,
            reason.as_deref(),
            std::mem::take(guard),
        )));
    }
    Ok(SettingsWrite::Text(proposed))
}

/// The text a `write` command reads once, for the snapshots and the command,
/// and the files it names (D-104, D-120).
fn replay_input(
    store: &crate::Store,
    a: &[&str],
    context: bool,
    settings: Option<String>,
    input: &mut dyn FnMut() -> Out,
    old: Option<&str>,
) -> Result<(Option<String>, Vec<String>), Failure> {
    let patch = if a.len() == 1 && a[0] == "write" && context { Some(input()?) } else { None };
    let fix = if a.starts_with(&["write", "--draft"]) && a.contains(&"--replace") && context { Some(input()?) } else { None };
    let files = if let Some(p) = &patch {
        vec![p.clone()]
    } else if context && a.len() >= 3 && a[0] == "write" && a[1] == "--draft" {
        match crate::drafts::view(store, a[2]) {
            Ok(d) => {
                let fixed = fix.as_ref().and_then(|new| crate::drafts::replace(&d, old, new, a.contains(&"--all")).ok()).map(|r| r.0);
                std::iter::once(d).chain(fixed).collect()
            }
            Err(_) => vec![],
        }
    } else {
        vec![]
    };
    Ok((patch.or(fix).or(settings), files))
}

fn run_command(
    session: &crate::command_tx::Session,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    old: Option<&str>,
    budget: usize,
    before: bool,
    whole: bool,
) -> Result<Out, Failure> {
    let replay: Vec<String> = session.args.iter().filter(|x| !whole || (*x != "--replace" && *x != "--all")).cloned().collect();
    let result = crate::drafts::with_project(session.project.as_deref(), || {
        run_inner(&session.root, &session.here, &replay, input, if whole { None } else { old }, budget)
    });
    if before {
        crate::command_tx::crash_after("command");
    }
    let mut result = result;
    if let Some((tid, _)) = &session.context
        && a.first().is_some_and(|c| VERIFIES.contains(c))
        && !crate::command_tx::reads_only(a)
    {
        if let Err(e) = record_verification(&session.home, tid, a[0], &result) {
            let note = format!("\nnote: could not record the verification ({})\n", e.0.trim());
            result = match result {
                Ok(mut out) => {
                    out.push_str(&note);
                    Ok(out)
                }
                Err(Failure(mut msg, code)) => {
                    msg.push_str(&note);
                    Err(Failure(msg, code))
                }
            };
        }
    }
    Ok(result)
}

/// `inspect`'s names and its `--lines` range: an option's value is not a name.
pub(crate) fn inspect_options<'a>(args: &[&'a str]) -> Result<(Vec<&'a str>, Option<&'a str>), Failure> {
    let mut names = vec![];
    let mut lines = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--lines" {
            let Some(spec) = args.get(i + 1).copied() else {
                return usage("inspect --lines takes a range, such as 3,7,10..12");
            };
            lines = Some(spec);
            i += 2;
        } else {
            if !args[i].starts_with("--") {
                names.push(args[i]);
            }
            i += 1;
        }
    }
    Ok((names, lines))
}
