//! The change-recording pipeline of a board run: the projects and locks a
//! command holds, its before and after snapshots, the outer transactions, the
//! undo records, and the crash points.

use crate::failure::Failure;
use serde_json::Value;
use serde_json::json;
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// May change the project: recorded on the task in a board run, and refused in
/// read-only runs, except `build`, which reviews run to verify.
pub(crate) const CHANGES: &[&str] = &["submit", "delete", "extract", "merge", "write", "remove", "move", "run", "format", "build"];

/// D-131: `build plan` only reads, so it is no change, process or verification.
pub(crate) fn reads_only(a: &[&str]) -> bool {
    a.starts_with(&["build", "plan"])
}

/// A command's resolved project, stores and locks, held together until the
/// command commits (D-77 point 7).
pub(crate) struct Session {
    pub(crate) args: Vec<String>,
    pub(crate) project: Option<String>,
    pub(crate) here: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) context: Option<(String, String)>,
    pub(crate) deps: Vec<crate::Store>,
    pub(crate) dest: Option<crate::Store>,
    pub(crate) home: crate::Store,
    pub(crate) store: crate::Store,
    pub(crate) guard: Vec<fs::File>,
}

/// What a change records: its own project's before state, a move's or extract's
/// destination, and the projects that depend on it (D-103, D-117).
pub(crate) struct Changes<'a> {
    before: Option<Value>,
    dest_before: Option<(&'a crate::Store, Option<String>, Value)>,
    deps_before: Vec<(&'a crate::Store, Value)>,
    folders: Option<std::collections::HashSet<String>>,
    dest_folders: Option<std::collections::HashSet<String>>,
    deps_folders: Vec<std::collections::HashSet<String>>,
}

impl Changes<'_> {
    /// The project's state before the change, when the command records one.
    pub(crate) fn before(&self) -> Option<&Value> {
        self.before.as_ref()
    }

    /// The destination store of a move or extract, if there is one.
    fn dest(&self) -> Option<&crate::Store> {
        self.dest_before.as_ref().map(|(d, ..)| *d)
    }

    /// The destination of an extract, whose symbols arrive without a named path.
    fn extract_dest(&self) -> Option<&crate::Store> {
        self.dest_before.as_ref().filter(|(_, to, _)| to.is_none()).map(|(d, ..)| *d)
    }
}

/// The outer transactions a change holds until it commits, and an extract's
/// record ahead of its commits (D-77 point 7, D-103).
pub(crate) struct Transactions {
    pub(crate) ahead: Option<Value>,
    _deps: Vec<crate::OuterTransaction>,
    _dest: Option<crate::OuterTransaction>,
    _source: crate::OuterTransaction,
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
    crate::extract::js_files(root).into_iter().map(|n| (format!("file:{n}"), json!(fs::read(root.join(&n)).ok()))).collect()
}

/// A store's snapshot with its JavaScript files.
fn with_scripts(d: &crate::Store) -> Result<Value, Failure> {
    let mut v = crate::board::snapshot(d)?;
    v.as_object_mut().unwrap().extend(script_files(&d.root));
    Ok(v)
}

/// A store's snapshot with its dependency files (D-103).
fn with_toml(d: &crate::Store) -> Result<Value, Failure> {
    let mut v = crate::board::snapshot(d)?;
    v.as_object_mut().unwrap().extend(dependency_files(&d.root));
    Ok(v)
}

/// A project's before or after state for a change: its live rows, the files
/// the command names, and an extract's dependency and script files (D-103).
pub(crate) fn snapshot_change(store: &crate::Store, a: &[&str], files: &[String]) -> Result<Value, Failure> {
    let mut v = crate::board::snapshot(store)?;
    v.as_object_mut().unwrap().extend(crate::board::file_snapshot(&store.root, a)?.as_object().unwrap().clone());
    for f in files {
        v.as_object_mut().unwrap().extend(crate::lockdown::patch_snapshot(&store.root, Some(f)));
    }
    if a[0] == "extract" && a.contains(&"--to-project") {
        v.as_object_mut().unwrap().extend(dependency_files(&store.root));
        v.as_object_mut().unwrap().extend(script_files(&store.root));
    }
    Ok(v)
}

pub(crate) fn changes<'a>(
    store: &crate::Store,
    dest: &'a Option<crate::Store>,
    deps: &'a [crate::Store],
    a: &[&str],
    context: Option<&(String, String)>,
    files: &[String],
) -> Result<Changes<'a>, Failure> {
    let before = if a.first().is_some_and(|c| CHANGES.contains(c)) && !reads_only(a) && context.is_some() {
        Some(snapshot_change(store, a, files)?)
    } else {
        None
    };
    // A move or extract into another project changes it too, recorded in its
    // store: a move's files there, or an extract's symbols, in a transaction of
    // its own around the command and its undo rows (D-103).
    let dest_before = match (dest, &before) {
        // a merge's survivor doesn't change
        (Some(_), Some(_)) if a[0] == "merge" => None,
        (Some(d), Some(_)) if a[0] == "extract" => Some((d, None, with_scripts(d)?)),
        (Some(d), Some(_)) => {
            let (_, to, _) = crate::lockdown::move_args(&store.root, a)?;
            Some((d, Some(to.to_string()), Value::Object(crate::board::files_under(&d.root, to))))
        }
        _ => None,
    };
    let deps_before: Vec<(&crate::Store, Value)> =
        if dest_before.as_ref().is_some_and(|(_, to, _)| to.is_none()) || (a[0] == "merge" && before.is_some()) {
            deps.iter().map(|d| Ok((d, with_toml(d)?))).collect::<Result<_, Failure>>()?
        } else {
            vec![]
        };
    // The folders each project had, so undo removes only those the task made (D-136).
    let folders = before.as_ref().map(|_| crate::board::existing_folders(&store.root));
    let dest_folders = dest_before.as_ref().map(|(d, ..)| crate::board::existing_folders(&d.root));
    let deps_folders: Vec<_> = deps_before.iter().map(|(d, _)| crate::board::existing_folders(&d.root)).collect();
    Ok(Changes { before, dest_before, deps_before, folders, dest_folders, deps_folders })
}

pub(crate) fn session(
    args: Vec<String>,
    cross: Option<PathBuf>,
    dest: Option<PathBuf>,
    project: Option<String>,
    here: PathBuf,
    own: PathBuf,
) -> Result<Session, Failure> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let context = crate::board::context();
    let home_path = match &context {
        Some((tid, _)) => crate::scope::home(&own, tid).unwrap_or_else(|_| own.clone()),
        None => own.clone(),
    };
    let root = cross.clone().unwrap_or(own);
    // An extract into another project, and any merge, rewrites the task's
    // projects that depend on the source (D-103, D-117): each is locked,
    // recorded and committed as the destination is.
    let deps: Vec<PathBuf> = match (&context, &dest) {
        (Some((tid, _)), d) if (a[0] == "extract" && d.is_some()) || a[0] == "merge" => {
            let top = crate::scope::top(&root);
            let projects = crate::scope::of(&crate::board::task(&home_path, tid)?, &crate::scope::dir(&top, &home_path));
            crate::extract::dependents(&top, &root, &projects).into_iter().filter(|p| Some(p) != d.as_ref()).collect()
        }
        _ => vec![],
    };
    let guard = if let Some((tid, rid)) = &context {
        // Away from home, the project the agent works in is locked too.
        let away = (cross.is_none() && !crate::project_paths::same_project(&root, &home_path)).then(|| root.clone());
        let also: Vec<&Path> = cross.iter().chain(&away).chain(&dest).chain(&deps).map(PathBuf::as_path).collect();
        crate::board::agent_guard(&home_path, tid, rid, if a.is_empty() { &["help"] } else { &a }, &also)?
    } else {
        vec![crate::board::lock(&root)?]
    };
    let store = crate::board::open(&root)?;
    let home = crate::board::open(&home_path)?;
    let dest = dest.map(|d| crate::board::open(&d)).transpose()?;
    let deps = deps.iter().map(|d| crate::board::open(d)).collect::<Result<Vec<_>, _>>()?;
    Ok(Session { args, project, here, root, context, deps, dest, home, store, guard })
}

pub(crate) fn recover_all(
    store: &crate::Store,
    home: &crate::Store,
    dest: Option<&crate::Store>,
    deps: &[crate::Store],
    guard: &[fs::File],
) -> Result<String, Failure> {
    let mut held = vec![store.root.clone(), home.root.clone()];
    held.extend(dest.iter().copied().chain(deps).map(|d| d.root.clone()));
    crate::extract::ahead::recover(&held)?;
    crate::lockdown::recover(&store.root, store, guard)?;
    if let Some(d) = dest {
        crate::lockdown::recover(&d.root, d, guard)?;
    }
    for d in deps {
        crate::lockdown::recover(&d.root, d, guard)?;
    }
    crate::board::flow(&store.root)?;
    crate::outside::absorb(store, crate::outside::Mode::Notice)
}

pub(crate) fn begin_transactions(session: &Session, changes: &Changes<'_>, a: &[&str], processes: bool) -> Result<Transactions, Failure> {
    let store = &session.store;
    let deps_before = &changes.deps_before;
    let before = changes.before.as_ref();
    let context = session.context.as_ref();
    let dest_tx = changes.extract_dest();
    let mark_dest = changes.dest();
    crate::COMMAND_DB.set(vec![(store.root.clone(), store.db.clone())]);
    let source = crate::OuterTransaction(store.db.clone());
    if let Some(d) = dest_tx {
        crate::COMMAND_DB.with_borrow_mut(|c| c.push((d.root.clone(), d.db.clone())));
    }
    let dest = dest_tx.map(|d| crate::OuterTransaction(d.db.clone()));
    for (d, _) in deps_before {
        crate::COMMAND_DB.with_borrow_mut(|c| c.push((d.root.clone(), d.db.clone())));
    }
    let deps: Vec<crate::OuterTransaction> = deps_before.iter().map(|(d, _)| crate::OuterTransaction(d.db.clone())).collect();
    if let (Some(_), Some((tid, rid))) = (before, context) {
        crate::lockdown::mark(&store.root, tid, rid)?;
        if let Some(d) = mark_dest {
            crate::lockdown::mark(&d.root, tid, rid)?;
        }
        for (d, _) in deps_before {
            crate::lockdown::mark(&d.root, tid, rid)?;
        }
    }
    // An extract into another project is written ahead: its record is in each of
    // its projects before any file changes (`extract::ahead`).
    let ahead = match (context, dest_tx) {
        (Some((tid, rid)), Some(d)) if a[0] == "extract" && before.is_some() => {
            let mut roots: Vec<&Path> = vec![&store.root, &d.root];
            roots.extend(deps_before.iter().map(|(d, _)| d.root.as_path()));
            Some(crate::extract::ahead::begin(tid, rid, &roots)?)
        }
        _ => None,
    };
    if before.is_some() && !processes {
        store.db.execute_batch("BEGIN IMMEDIATE")?;
        if let Some(d) = dest_tx {
            d.db.execute_batch("BEGIN IMMEDIATE")?;
        }
        for (d, _) in deps_before {
            d.db.execute_batch("BEGIN IMMEDIATE")?;
        }
    }
    Ok(Transactions { ahead, _deps: deps, _dest: dest, _source: source })
}

pub(crate) fn commit(
    session: &Session,
    changes: &Changes<'_>,
    before: &Value,
    after: &Value,
    ahead: &mut Option<Value>,
) -> Result<(), Failure> {
    let Some((tid, rid)) = session.context.as_ref() else { return Ok(()) };
    let store = &session.store;
    let dest_tx = changes.extract_dest();
    if let (Some(record), Some(d)) = (&mut *ahead, dest_tx) {
        // The computed result joins the record before the first commit; then the
        // destination commits, then the source and the dependents, each with its
        // undo rows; then the gleam.toml lines, and the record goes.
        let d_before = &changes.dest_before.as_ref().unwrap().2;
        let d_after = with_scripts(d)?;
        let deps_after: Vec<Value> = changes.deps_before.iter().map(|(d, _)| with_toml(d)).collect::<Result<_, _>>()?;
        let mut stores: Vec<(&crate::Store, &Value, &Value)> = vec![(store, before, after), (d, d_before, &d_after)];
        stores.extend(changes.deps_before.iter().zip(&deps_after).map(|((d, b), a)| (*d, b, a)));
        crate::extract::ahead::complete(record, &stores)?;
        crash_after("complete");
        crate::board::record(d, tid, rid, d_before, &d_after, changes.dest_folders.as_ref())?;
        d.db.execute_batch("COMMIT")?;
        crash_after("destination");
        crate::board::record(store, tid, rid, before, after, changes.folders.as_ref())?;
        store.db.execute_batch("COMMIT")?;
        crash_after("source");
        for (((d, d_before), d_after), f) in changes.deps_before.iter().zip(&deps_after).zip(&changes.deps_folders) {
            crate::board::record(d, tid, rid, d_before, d_after, Some(f))?;
            d.db.execute_batch("COMMIT")?;
        }
        crate::extract::ahead::end(record)?;
        crash_after("commit");
    } else {
        // A merge's dependents commit before the source.
        for ((d, d_before), f) in changes.deps_before.iter().zip(&changes.deps_folders) {
            crate::board::record(d, tid, rid, d_before, &with_toml(d)?, Some(f))?;
            d.db.execute_batch("COMMIT")?;
            crate::lockdown::drop_records(&d.root, rid)?;
        }
        if !changes.deps_before.is_empty() {
            crash_after("others");
        }
        crate::board::record(store, tid, rid, before, after, changes.folders.as_ref())?;
        store.db.execute_batch("COMMIT")?;
        crash_after("commit");
        // The task lists its changes from the rows that committed (D-77 point 4),
        // computed by changes_of, so there is nothing to write here.
        crate::lockdown::drop_records(&store.root, rid)?;
    }
    if let Some((d, Some(to), files)) = &changes.dest_before {
        d.db.execute_batch("BEGIN IMMEDIATE")?;
        crate::board::record(d, tid, rid, files, &Value::Object(crate::board::files_under(&d.root, to)), changes.dest_folders.as_ref())?;
        d.db.execute_batch("COMMIT")?;
        crate::lockdown::drop_records(&d.root, rid)?;
    }
    Ok(())
}

/// A test-only hook: `SOLACE_FAIL_AFTER=<point>` exits after the named point,
/// and `panic:<point>` panics there instead, so a caught panic is tested too
/// (`command` | `complete` | `destination` | `others` | `commit`;
/// `complete` is after an extract's record gains its result, before any commit;
/// `destination` after its destination commits, before the source does;
/// `source` after the source commits, before the dependents do;
/// `others` after a merge's dependents commit, before the source does).
/// Only in a board run, so a plain CLI call is never interrupted.
pub(crate) fn crash_after(point: &str) {
    if crate::board::context().is_none() {
        return;
    }
    #[cfg(test)]
    if CRASH_AT.with(|p| p.borrow().as_deref() == Some(point)) {
        panic!("crash after {point}");
    }
    let Ok(asked) = env::var("SOLACE_FAIL_AFTER") else { return };
    if asked == point {
        std::process::exit(1);
    }
    if asked.strip_prefix("panic:") == Some(point) {
        panic!("SOLACE_FAIL_AFTER=panic:{point}");
    }
}

#[cfg(test)]
thread_local! {
    static CRASH_AT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn crash_at(point: Option<&str>) {
    CRASH_AT.with(|p| *p.borrow_mut() = point.map(str::to_string));
}
