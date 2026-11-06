//! Review views: a task's before and after code graphs, and the scratch copy a review's tests run in.

use super::*;
use crate::Sym;
use crate::ModuleRow;
use crate::Store;
use crate::Index;
use crate::unique_id;
use crate::Before;
use crate::failure::Failure;
use crate::view::review;
#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
use crate::lockdown::files::copy_files;

pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<(), Failure> {
    if !from.exists() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    migrate::copy_tree(from, to, &|_| false)
}

/// A task as this project sees it (D-61): only the changes made here.
pub(crate) fn changed_here(s: &Store, mut t: Value) -> Value {
    let here = scope::dir(&scope::top(&s.root), &s.root);
    let mine: Vec<Value> = board::changes_of(s, &t).unwrap_or_default().into_iter().filter(|c| c["project"] == here).collect();
    t["changes"] = json!(mine);
    t
}

/// A task found from any project in its scope, as this project sees it, with its home.
pub(crate) fn task_here(s: &Store, tid: &str) -> Result<(PathBuf, Value), Failure> {
    let home = scope::home(&s.root, tid)?;
    let t = board::task(&home, tid)?;
    Ok((home.clone(), changed_here(s, t)))
}

#[derive(Clone, Copy)]
pub(crate) enum Version<'a> {
    Current,
    Before(&'a str),
}

pub(crate) fn version(s: &Store, v: Version<'_>) -> Result<Index, Failure> {
    let mut rows = board::snapshot(s)?;
    if let Version::Before(tid) = v {
        for (key, data) in board::reviews::before_of(s, tid)? {
            rows[&key] = data;
        }
    }
    let mut syms = vec![];
    let mut modules = vec![];
    for (key, row) in rows.as_object().unwrap() {
        // Absent: `null`, or where undo stops removing folders (D-136).
        if !row.is_array() {
            continue;
        }
        if key.starts_with("symbol:") {
            // A snapshot from before the nested columns has only the first eight (D-158).
            syms.push(Sym {
                id: row[0].as_str().unwrap().into(),
                module: row[1].as_str().unwrap().into(),
                name: row[2].as_str().unwrap().into(),
                kind: Kind::parse(row[3].as_str().unwrap()),
                kind_text: row[3].as_str().unwrap().into(),
                text: row[4].as_str().unwrap().into(),
                parent: row.get(5).and_then(|x| x.as_str()).map(String::from),
                sep: row.get(6).and_then(|x| x.as_str()).map(String::from),
                tail: row.get(7).and_then(|x| x.as_str()).unwrap_or("").into(),
            });
        } else if key.starts_with("module:") {
            modules.push(ModuleRow {
                path: row[0].as_str().unwrap().into(),
                dir: row[1].as_str().unwrap().into(),
                header: row[2].as_str().unwrap().into(),
                file: row.get(3).and_then(|x| x.as_str()).map(String::from),
                // A snapshot row from before the column has only four fields (D-168).
                owns_dir: row.get(4).and_then(|x| x.as_i64()).map(|v| v != 0),
            });
        }
    }
    syms.sort_by(|a, b| a.module.cmp(&b.module).then(a.name.cmp(&b.name)));
    Ok(crate::index::build(s.lang(), syms, modules, language::is_rust(&s.root)))
}

/// What the task's changes reach (D-44): its tests (those it changed, and
/// those that reach a symbol it changed or read a field the change reaches),
/// and the uses no test checks, with declared suites and where data leaves.
pub(crate) fn task_reach(s: &Store, old: &Index, now: &Index, task: &Value) -> Result<Value, Failure> {
    let ids: Vec<String> = task["changes"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c["entity"].as_str().and_then(|x| x.strip_prefix("symbol:")).map(String::from))
        .collect();
    let before: Before = ids.iter().map(|id| (id.clone(), old.syms.iter().find(|s| &s.id == id).map(|s| s.text.clone()))).collect();
    let impact = reach::flow(now, &reach::origins(now, &before));
    let changed: Vec<usize> = (0..now.syms.len()).filter(|&i| ids.contains(&now.syms[i].id)).collect();
    let readers = reach::readers(now, &impact.fields);
    let tests: Vec<&str> = reach::test_symbols(now)
        .into_iter()
        .filter(|&t| {
            changed.contains(&t) || readers.contains(&t) || !reach::closure(now, t).is_disjoint(&changed.iter().copied().collect())
        })
        .map(|t| now.syms[t].id.as_str())
        .collect();
    let untested: Vec<Value> = reach::ranked(now, &impact)
        .into_iter()
        .filter(|(_, t)| t.is_empty())
        .map(|(u, _)| json!({"id":now.syms[u.sym].id,"name":reach::short(now,u.sym),"steps":u.steps,"via":u.via,"certain":u.certain}))
        .collect();
    let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
    let suites = reach::reached(&reach::suites(s, now)?, &syms);
    let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", reach::short(now, *i))).collect();
    Ok(json!({"tests":tests,"untested":untested,"suites":suites,"leaves":leaves}))
}

pub(crate) fn review_graphs(before: &Value, after: &Value, task: &Value, reach: &Value, settings: &toml::Value, root: &Path) -> Value {
    let nodes = |g: &Value| -> BTreeMap<String, Value> {
        g["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap().into(), n.clone())).collect()
    };
    let b = nodes(before);
    let a = nodes(after);
    let effects = |g: &Value, id: &str| -> BTreeSet<String> {
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["from"] == id && e["kind"] == "uses")
            .filter_map(|e| e["to"].as_str().map(String::from))
            .collect()
    };
    let mut entries = vec![];
    let mut signatures = vec![];
    for (id, node) in &a {
        if node["kind"] != "entry" {
            continue;
        }
        let old = b.get(id);
        let added = effects(after, id);
        let removed = old.map(|_| effects(before, id)).unwrap_or_default();
        let add: Vec<_> = added.difference(&removed).cloned().collect();
        let del: Vec<_> = removed.difference(&added).cloned().collect();
        let sig_changed = old.is_some_and(|o| o["signature"] != node["signature"]);
        if old.is_none() || !add.is_empty() || !del.is_empty() || sig_changed {
            entries.push(json!({"id":id,"change":if old.is_none(){"added"}else{"changed"},"effects_added":add,"effects_removed":del}));
        }
    }
    for (id, node) in &b {
        if node["kind"] == "entry" && !a.contains_key(id) {
            entries.push(
                json!({"id":id,"change":"removed","effects_added":[],"effects_removed":effects(before,id),"removed":true,"node":node}),
            );
        }
    }
    for (id, node) in &a {
        if let Some(old) = b.get(id)
            && old["signature"] != node["signature"]
            && matches!(node["kind"].as_str(), Some("entry" | "logic"))
        {
            signatures.push(json!({"id":id,"before":old["signature"],"after":node["signature"]}));
        }
    }
    let before_effects: BTreeSet<_> = b.keys().filter(|k| k.starts_with("effect:")).cloned().collect();
    let after_effects: BTreeSet<_> = a.keys().filter(|k| k.starts_with("effect:")).cloned().collect();
    let effects_added: Vec<_> = after_effects.difference(&before_effects).cloned().collect();
    let effects_removed: Vec<_> = before_effects.difference(&after_effects).cloned().collect();
    let mut files = vec![];
    for change in task["changes"].as_array().unwrap_or(&vec![]) {
        if let Some(name) = change["entity"].as_str().filter(|k| board::reviewable(root, k)).and_then(|x| x.strip_prefix("file:")) {
            let path = root.join(name);
            let before_path = board::before_file(root, task["id"].as_str().unwrap(), name);
            let change = if !path.exists() {
                "removed"
            } else if before_path.with_extension("absent").exists() {
                "added"
            } else {
                "changed"
            };
            files.push(json!({"id":format!("file:{name}"),"tier":lockdown::tier(root,settings,name,None),"change":change}));
        }
    }
    let changed: HashSet<_> = task["changes"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c["entity"].as_str().and_then(|x| x.strip_prefix("symbol:")).map(String::from))
        .collect();
    let tested: HashSet<_> = reach["tests"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|id| id.as_str().map(String::from))
        .chain(changed.iter().cloned())
        .collect();
    let stored = &task["review_tests"];
    let test_results = stored["results"].as_array().cloned().unwrap_or_default();
    let tests: Vec<_> = a
        .iter()
        .filter(|(id, n)| n["kind"] == "test" && tested.contains(*id))
        .map(|(id, n)| {
            let mut result = test_results
                .iter()
                .find(|r| r["id"] == *id)
                .cloned()
                .unwrap_or_else(|| json!({"id":id,"name":n["name"],"result":"not run yet","checks":n["checks"]}));
            // run_tests matches failures by module and name.
            result["module"] = n["module"].clone();
            if let Some(old) = b.get(id) {
                let mut old_checks = BTreeMap::new();
                for c in old["checks"].as_array().unwrap_or(&vec![]) {
                    if let Some(call) = c["call"].as_str() {
                        old_checks.insert(call.to_string(), c["expected"].clone());
                    }
                }
                for c in result["checks"].as_array_mut().unwrap() {
                    if let Some(call) = c["call"].as_str()
                        && let Some(was) = old_checks.get(call)
                        && *was != c["expected"]
                    {
                        c["was"] = was.clone();
                    }
                }
            }
            result
        })
        .collect();
    let unfinished = tests.iter().filter(|t| t["result"] == "unfinished").count();
    let added_count: usize = entries.iter().map(|e: &Value| e["effects_added"].as_array().map_or(0, Vec::len)).sum();
    let untested = reach["untested"].as_array().map_or(0, Vec::len);
    let summary = format!(
        "{} effects added, {} signature changed, {} test unfinished, {} untested uses{}",
        added_count,
        signatures.len(),
        unfinished,
        untested,
        if stored["state"].is_null() { ", tests not run yet" } else { "" }
    );
    json!({
        "entries": entries,
        "effects": {"added": effects_added, "removed": effects_removed},
        "signatures": signatures,
        "tests": tests,
        "files": files,
        "untested": reach["untested"],
        "suites": reach["suites"],
        "leaves": reach["leaves"],
        "summary": summary,
    })
}

/// A copy of the project as it is now, for running review tests away from the real one.
fn scratch(s: &Store, dir: &Path) -> Result<(), Failure> {
    s.materialize_to(&version(s, Version::Current)?, dir, &HashMap::new())?;
    // Tests may read data and fixtures, text or binary.
    copy_files(s, dir, |_, tier| tier == "watched" || tier == "unwatched (binary)", &["gleam.toml", "manifest.toml"])?;
    copy_dir(&s.root.join("build/packages"), &dir.join("build/packages"))?;
    copy_dir(&s.root.join("build/dev"), &dir.join("build/dev"))?;
    Ok(())
}

pub fn run_tests(root: PathBuf, tid: String) {
    std::thread::spawn(move || {
        let result = (|| -> Result<(), Failure> {
            let dir = env::temp_dir().join(format!("solace-view-{}", unique_id()));
            // The task may be stored in another project of its scope (D-61).
            // one review_tests per task; tests run from another project replace them.
            let home = scope::home(&root, &tid)?;
            let (base, tests) = {
                let _guard = scope::lock_all(&[root.clone(), home.clone()])?;
                let s = board::open(&root)?;
                let (_, here) = task_here(&s, &tid)?;
                let base = board::fingerprint(&s)?;
                scratch(&s, &dir)?;
                let r = review(&s, &here)?;
                let running = json!({"base":base,"state":"running","results":[]});
                tasks::update(&home, &tid, move |t| {
                    t["review_tests"] = running;
                    Ok(())
                })?;
                (base, r["tests"].as_array().cloned().unwrap_or_default())
            };
            let lang = language::of(&dir);
            let output = lockdown::command(&dir, &[lang.tool().into(), "test".into()], 300, 100_000, false, false);
            let _ = fs::remove_dir_all(&dir);
            let (state, results) = match output {
                Ok(log) => {
                    let failed = lang.failures(&log);
                    let compile = log.contains("error:") && failed.is_empty();
                    let results = tests
                        .iter()
                        .map(|t| {
                            let id = t["id"].as_str().unwrap();
                            let name = t["name"].as_str().unwrap();
                            let module = t["module"].as_str().unwrap_or("");
                            let key = format!("{}.{}", module, name);
                            let (result, message) = failed.get(&key).cloned().unwrap_or((
                                if compile { "failed to compile" } else { "passing" }.into(),
                                if compile { log.clone() } else { String::new() },
                            ));
                            json!({"id":id,"name":name,"result":result,"message":message,"checks":t["checks"]})
                        })
                        .collect::<Vec<_>>();
                    (if compile { "failed to compile" } else { "done" }, results)
                }
                Err(e) => (
                    "failed to compile",
                    tests
                        .iter()
                        .map(|t| json!({"id":t["id"],"name":t["name"],"result":"failed to compile","message":e.0,"checks":t["checks"]}))
                        .collect(),
                ),
            };
            let done = json!({"base":base,"state":state,"results":results});
            tasks::update(&home, &tid, move |t| {
                t["review_tests"] = done;
                Ok(())
            })
        })();
        if let Err(e) = result {
            eprintln!("review tests: {}", e.0);
        }
    });
}
