//! Unfinished tasks' changes a test run includes: what the tests that ran can
//! reach, so a pass or failure is not mistaken for this task's work (D-110).

use crate::board::reviews::changes_of;
use crate::board::task_model::{all_tasks, column, flow};
use crate::{Index, Store};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
#[path = "unfinished_tests.rs"]
mod tests;

/// A line naming each other unfinished task whose changes the tests that ran
/// (`reached`, positions in `ix`) can reach, one per project of the run.
pub(crate) fn included(s: &Store, ix: &Index, reached: &HashSet<usize>, projects: &HashSet<String>) -> Vec<String> {
    let Some((running, _)) = crate::board::runs::context() else { return vec![] };
    let covered = with_parents(ix, reached);
    let mut out = vec![];
    for t in unfinished(s) {
        if t["id"].as_str() == Some(running.as_str()) {
            continue;
        }
        let Ok(changes) = changes_of(s, &t) else { continue };
        let mut hit: Vec<String> = changes
            .iter()
            .filter(|c| c["project"].as_str().is_some_and(|p| projects.contains(p)))
            .filter(|c| reaches(ix, &covered, c["entity"].as_str().unwrap_or("")))
            .filter_map(|c| c["project"].as_str().map(String::from))
            .collect();
        hit.sort();
        hit.dedup();
        out.extend(hit.iter().map(|project| line(&t, project)));
    }
    out
}

/// A line naming each other unfinished task with changes in this project's
/// code, `Cargo.toml`, `Cargo.lock` or `build.rs`: a compile failure may come
/// from anywhere in the crate.
pub(crate) fn compile_failure(s: &Store) -> Vec<String> {
    let Some((running, _)) = crate::board::runs::context() else { return vec![] };
    let here = project_dir(s);
    let mut out = vec![];
    for t in unfinished(s) {
        if t["id"].as_str() == Some(running.as_str()) {
            continue;
        }
        let Ok(changes) = changes_of(s, &t) else { continue };
        if changes.iter().filter(|c| c["project"] == here).any(|c| crate_change(c["entity"].as_str().unwrap_or(""))) {
            out.push(line(&t, &here));
        }
    }
    out
}

/// The root's unfinished tasks: those not in a final column (D-110).
fn unfinished(s: &Store) -> Vec<Value> {
    let top = crate::scope::top(&s.root);
    let (Ok(tasks), Ok(w)) = (all_tasks(&top), flow(&top)) else { return vec![] };
    tasks.into_iter().filter(|t| column(&w, t)["final"] != true).collect()
}

fn project_dir(s: &Store) -> String {
    crate::scope::dir(&crate::scope::top(&s.root), &s.root)
}

/// `reached` plus the containers its symbols sit in, as `task_untested` sees them.
fn with_parents(ix: &Index, reached: &HashSet<usize>) -> HashSet<usize> {
    let mut out = reached.clone();
    let by_id: HashMap<&str, usize> = ix.syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    for &i in reached {
        let mut parent = ix.syms[i].parent.as_deref();
        while let Some(p) = parent {
            let Some(&k) = by_id.get(p) else { break };
            out.insert(k);
            parent = ix.syms[k].parent.as_deref();
        }
    }
    out
}

fn reaches(ix: &Index, covered: &HashSet<usize>, entity: &str) -> bool {
    if let Some(id) = entity.strip_prefix("symbol:") {
        return ix.syms.iter().position(|s| s.id == id).is_some_and(|i| covered.contains(&i));
    }
    if let Some(module) = entity.strip_prefix("module:") {
        return covered.iter().any(|&i| ix.syms[i].module == module);
    }
    false
}

fn crate_change(entity: &str) -> bool {
    entity.starts_with("symbol:")
        || entity.starts_with("module:")
        || matches!(entity.strip_prefix("file:"), Some("Cargo.toml" | "Cargo.lock" | "build.rs"))
}

fn line(t: &Value, project: &str) -> String {
    let id = t["id"].as_str().unwrap_or("");
    let title = t["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(id);
    format!("included unfinished changes from task \"{title}\" in {project}")
}
