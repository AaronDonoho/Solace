use std::path::Path;
use std::path::PathBuf;
use std::env;
use std::fs;
use crate::failure::{usage, Failure, Out};
use super::routing::{operands, unknown_option};
use super::READS;
use crate::lockdown::settings::settings;
use crate::board::runs::context;
use crate::board::runs::strategy_context;
use crate::board::runs::agent_guard;
/// `--project <dir>` on a command that isn't a plain read, in a board run whose
/// task may change that project (D-61): the folder the command runs in. Plain
/// reads, and commands outside a run, go to `read_project`.
pub(crate) fn scoped(dir: &str, args: &[String]) -> Result<Option<PathBuf>, Failure> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let (Some((tid, _)), Ok(here)) = (context(), crate::Store::root()) else { return Ok(None) };
    if READS.contains(&command) && !args.iter().any(|x| x == "--draft") {
        return Ok(None);
    }
    let top = crate::scope::top(&here);
    let members = crate::workspace::members(&top)?;
    let dir = &crate::project_paths::project_folder(&top, Some(&here), dir, &members);
    if !members.iter().any(|m| m == dir) {
        return crate::lockdown::error("E6007", dir, format!("not a project on this board; its projects: {}", members.join(", ")));
    }
    match crate::scope::retry_task(|| crate::scope::task_in(&here, &tid)) {
        Ok((own, t)) => {
            let projects = crate::scope::of(&t, &crate::scope::dir(&top, &own));
            if !projects.iter().any(|p| p == dir) {
                return crate::lockdown::error(
                    "E6007",
                    dir,
                    format!("not one of this task's projects ({}). {}", projects.join(", "), crate::scope::ASK_FOR_PROJECT),
                );
            }
        }
        Err(e) => return Err(e),
    }
    if command == "task" {
        return crate::lockdown::error("E6007", command, "task commands belong to the task's own project; leave out --project");
    }
    Ok(Some(crate::scope::path(&top, dir)))
}

/// `--project <dir>`: a read in another project under the same Solace root
/// (D-57). It takes no lock in that project, so reading never waits on its
/// agents. Changes go through `scoped` (D-61).
pub(crate) fn read_project(dir: &str, args: &[String], budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = a.first().copied().unwrap_or("");
    if let Some(out) = unknown_option(&a) {
        return out;
    }
    if !READS.contains(&command) || a.contains(&"--draft") {
        return crate::lockdown::error(
            "E6007",
            command,
            if strategy_context().is_some() {
                "`--project` only reads (view, inspect, refs, find, grep, files, read)"
            } else {
                "`--project` only reads (view, inspect, refs, find, grep, files, read) here; in a board run, \
                 it also changes the task's other projects"
            },
        );
    }
    // From a project, or from a folder of projects with none at its root.
    let here = env::current_dir()?;
    let own = crate::Store::root().ok();
    read_project_in(&a, &here, own.as_deref(), dir, budget)
}

/// Each `E1001 unknown name` line of a reply, followed by the project that
/// has the name when another one does (D-123).
pub(crate) fn hinted(out: Out, here: &Path) -> Out {
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
        let Some(s) = crate::Store::peek(&crate::scope::path(&top, d)).ok() else { return false };
        let Ok(ix) = s.index() else { return false };
        s.resolve(&ix, name).is_ok()
    })
}

/// `read_project` from an explicit working folder and owning project, so a test
/// never changes the process-wide working folder.
pub(crate) fn read_project_in(a: &[&str], here: &Path, own: Option<&Path>, dir: &str, budget: usize) -> Out {
    let command = a.first().copied().unwrap_or("");
    // From a project, or from a folder of projects with none at its root.
    let root = match crate::workspace::enclosing(own.unwrap_or(here))? {
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
    let members = crate::workspace::members(&root)?;
    let dir = &crate::project_paths::project_folder(&root, own, dir, &members);
    if !members.iter().any(|m| m == dir) {
        return crate::lockdown::error(
            "E6007",
            dir,
            format!(
                "not a project under this Solace root: `--project` takes a folder from the Solace root; its projects: {}",
                members.join(", ")
            ),
        );
    }
    let target = crate::scope::path(&root, dir);
    // A board run's agent must still be running, as for any command.
    let _guard = match (context(), own) {
        (Some((tid, rid)), Some(own)) => Some(agent_guard(&crate::scope::home(own, &tid)?, &tid, &rid, &a, &[])?),
        _ => None,
    };
    let store = crate::Store::peek(&target)?;
    let settings = settings(&store.root)?;
    if let Some(out) = crate::lockdown::dispatch(&store, &settings, &a, &mut || Ok(String::new()), None, budget) {
        return out;
    }
    // A Rust project has no `Language`; its store's symbols are read as any
    // other's (D-158), so only a project of plain files is refused.
    if store.lang.is_none() && !crate::language::is_rust(&store.root) {
        let task = context().map(|(tid, _)| tid);
        return crate::lockdown::plain(&store, task.as_deref(), command, a.get(1).copied());
    }
    let out = match a {
        ["view", rest @ ..] if !operands(rest).is_empty() => {
            store.view_with_header(&operands(rest), rest.contains(&"--bodies"), rest.contains(&"--header"), budget)
        }
        ["inspect", rest @ ..] if !operands(rest).is_empty() => {
            let (names, lines) = crate::cli::routing::inspect_options(rest)?;
            store.inspect(&names, rest.contains(&"--context"), lines, budget)
        }
        ["refs", names @ ..] if !names.is_empty() => store.refs_all(names, budget),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        _ => usage(format!("{} --project {dir}: give what to read", a.join(" "))),
    };
    hinted(out, &store.root)
}

/// The Solace root and the folders of the other projects under it, the run's
/// task's projects first, then the rest: `None` when `here` is in no Solace root.
fn other_projects(here: &Path) -> Option<(PathBuf, Vec<String>)> {
    let top = crate::workspace::enclosing(here).ok().flatten()?;
    let mine = crate::scope::dir(&top, here);
    let mut order = crate::scope::of_run(here).unwrap_or_default();
    let rest: Vec<String> = crate::workspace::members(&top).ok()?.into_iter().filter(|m| !order.contains(m)).collect();
    order.extend(rest);
    let others = order.into_iter().filter(|d| *d != mine).collect();
    Some((top, others))
}

/// The first project other than `here` with a module named `name` (D-123):
/// the run's task's projects first, then the others under the Solace root.
pub(crate) fn module_elsewhere(here: &Path, name: &str) -> Option<String> {
    let (top, dirs) = other_projects(here)?;
    dirs.into_iter().find(|d| {
        let Some(s) = crate::Store::peek(&crate::scope::path(&top, d)).ok() else { return false };
        let Ok(ix) = s.index() else { return false };
        ix.modules.iter().any(|m| m.path == name)
    })
}

/// The directories the manifest names as path dependencies.
pub(crate) fn path_dependencies(root: &Path) -> Vec<PathBuf> {
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
