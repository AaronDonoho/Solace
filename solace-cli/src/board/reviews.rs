use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::fs;
use rusqlite::params;
use std::path::PathBuf;
use crate::board::task_model::open;
use crate::board::err;
use crate::board::tasks;
use crate::board::task_model::guard;
use crate::board::task_model::message;
use crate::board::runs::Watch;
use crate::board::task_model::task_dir;
use crate::board::task_model::task;
use crate::board::task_model::all_tasks;
use crate::board::task_model::flow;
use crate::board::task_model::note;
use crate::board::task_model::run_at;
use crate::board::task_model::end_runs;
use crate::board::task_model::new_task;
use crate::board::task_model::planning_column;
use crate::board::task_model::task_at_home;
use crate::Store;
use crate::sha256;
use crate::file_name;
use crate::failure::Failure;
use crate::failure::Out;
use crate::lockdown::files::paths;
#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
use crate::scope;
use crate::workspace;
use crate::lockdown;
use crate::lockdown::Ignores;
use crate::jsonx;
use crate::outside;
use crate::extract::ahead;
use crate::drafts;
use rusqlite::OptionalExtension;

// Snapshot live rows only. Before-state rows never enter the semantic index.
pub fn snapshot(s: &Store) -> Result<Value, Failure> {
    let mut out = json!({});
    let mut st = s.db.prepare("SELECT id,module,name,kind,text,ord,revision,live,parent,sep,tail FROM symbols WHERE live=1")?;
    for row in st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, String>(10)?
        ]))
    })? {
        let row = row?;
        let key = format!("symbol:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    // A Rust module's `owns_dir` is part of its before-state: it says where the module's own
    // children go, so undo restores the recorded value instead of guessing (D-168).
    let owns = if s.owns_dir { "owns_dir" } else { "NULL" };
    let mut st = s.db.prepare(&format!("SELECT path,dir,header,file,{owns} FROM modules"))?;
    for row in st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?
        ]))
    })? {
        let row = row?;
        let key = format!("module:{}", row[0].as_str().unwrap());
        out[key] = row;
    }
    Ok(out)
}

/// The stores of a task's projects (D-61), with their folders; `s` is its home's.
pub fn scope_stores(s: &Store, t: &Value) -> Result<Vec<(String, Store)>, Failure> {
    let top = scope::top(&s.root);
    let members = workspace::members(&top)?;
    scope::of(t, &scope::dir(&top, &s.root))
        .into_iter()
        .filter(|d| members.contains(d))
        .map(|d| Ok((d.clone(), open(&scope::path(&top, &d))?)))
        .collect()
}

/// Records what a command changed in `s` on the task stored in `home`. The rows
/// commit inside the command's outer transaction (D-77 point 7); the task's
/// change list comes from the committed rows after the release (D-77 point 4).
/// `folders` are the project's folders before the command (`existing_folders`);
/// with none, as for an outside edit, undo removes every folder left empty.
pub fn record(s: &Store, tid: &str, rid: &str, before: &Value, after: &Value, folders: Option<&HashSet<String>>) -> Result<(), Failure> {
    touch(s, tid, rid, before, after, folders)?;
    Ok(())
}

/// The project's folders, relative to `root`, as a command finds them before
/// it runs, skipping what `snapshot` skips: undo removes only the folders a
/// task made (D-136). A folder the project ignores (`target/`) is listed but
/// not walked: undo stops at it, and its contents can be large.
pub fn existing_folders(root: &Path) -> HashSet<String> {
    fn walk(root: &Path, ignores: &mut lockdown::Ignores, dir: &Path, out: &mut HashSet<String>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = file_name(p.strip_prefix(root).unwrap());
            if !e.file_type().is_ok_and(|t| t.is_dir())
                || [".solace", ".git", "build"].contains(&name.as_str())
                || e.file_name() == "node_modules"
            {
                continue;
            }
            if !ignores.ignored(&name) {
                walk(root, ignores, &p, out);
            }
            out.insert(name);
        }
    }
    let mut out = HashSet::new();
    walk(root, &mut Ignores::new(root), root, &mut out);
    out
}

/// The before-state of file `name` when it didn't exist (D-136): the nearest
/// folder above it that did (`""`, the project's own), where undo stops
/// removing the folders it leaves empty.
pub fn absent(name: &str, folders: &HashSet<String>) -> Value {
    let mut dir = name;
    while let Some((up, _)) = dir.rsplit_once('/') {
        if folders.contains(up) {
            return json!({ "kept": up });
        }
        dir = up;
    }
    json!({ "kept": "" })
}

/// `absent`, read from the disk just before file `name` is written: the
/// nearest folder above it that exists now. An undo record taken before the
/// write carries it, so recovery after a crash keeps D-136.
pub fn kept_on_disk(root: &Path, name: &str) -> Value {
    let mut dir = name;
    while let Some((up, _)) = dir.rsplit_once('/') {
        if root.join(up).is_dir() {
            return json!({ "kept": up });
        }
        dir = up;
    }
    json!({ "kept": "" })
}

/// The task's changes, computed from each scope project's store (D-77 point 4):
/// `board_touches` rows by task id, in insertion order, one entry per entity and
/// project (the first row that touched it). A WAL read through `open`, no project lock.
pub fn changes_of(s: &Store, t: &Value) -> Result<Vec<Value>, Failure> {
    let tid = t["id"].as_str().unwrap_or("");
    let (mut out, mut seen) = (vec![], HashSet::new());
    for (dir, st) in scope_stores(s, t)? {
        let mut q = st.db.prepare("SELECT entity,run,name FROM board_touches WHERE task=?1 ORDER BY seq")?;
        for row in
            q.query_map(params![tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?
        {
            let (entity, run, name) = row?;
            let (run, name) = (run.unwrap_or_default(), name.unwrap_or_else(|| entity.clone()));
            if !seen.insert((dir.clone(), entity.clone())) {
                continue;
            }
            out.push(json!({"entity": entity, "name": name, "run": run, "project": dir}));
        }
    }
    Ok(out)
}

/// Adds the undo rows for a pending record whose files changed (D-77 point 7):
/// one per path whose bytes differ from the file now, skipping paths whose rows
/// are already committed. A declared run's record lists the watched files as
/// they were before the process; a watched file it doesn't list was created by it.
pub fn adopt_pending(s: &Store, record: &Value) -> Result<(), Failure> {
    let (Some(tid), Some(rid)) = (record["task"].as_str(), record["run"].as_str()) else { return Ok(()) };
    let bad = || Failure("invalid pending record".into(), 1);
    let mut files: Vec<(String, Value)> = vec![];
    if let Some(v) = record.get("files") {
        for f in v.as_array().unwrap() {
            files.push((f["path"].as_str().ok_or_else(bad)?.to_string(), f["bytes"].clone()));
        }
    } else if let Some(path) = record.get("path") {
        files.push((path.as_str().ok_or_else(bad)?.to_string(), record["bytes"].clone()));
    } else {
        // A command's marker: the store already discarded what didn't commit.
        return Ok(());
    }
    let listed: HashSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    // A record taken before the write says where undo stops removing folders
    // (D-136): a file's own `kept`, or a run's folders; an old record has neither.
    let folders: Option<HashSet<String>> =
        record["folders"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect());
    if record.get("files").is_some() {
        let v = lockdown::settings(&s.root)?;
        for name in lockdown::watched_paths(&s.root, &v)? {
            if !listed.contains(&name) {
                files.push((name.clone(), Value::Null));
            }
        }
    }
    for (name, bytes) in files {
        let key = format!("file:{name}");
        let now = fs::read(lockdown::path(&s.root, &name)?).ok();
        let before = bytes.as_array().map(|a| jsonx::bytes(a.as_slice()));
        if now == before {
            continue;
        }
        let committed: bool = s.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_touches WHERE task=?1 AND run=?2 AND entity=?3)",
            params![tid, rid, key],
            |r| r.get(0),
        )?;
        if committed {
            continue;
        }
        let exists: bool =
            s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1 AND entity=?2)", params![tid, key], |r| r.get(0))?;
        if !exists {
            fs::create_dir_all(before_dir(&s.root, tid))?;
            let file = before_file(&s.root, tid, &name);
            match &before {
                Some(b) => fs::write(file, b)?,
                None => fs::write(file.with_extension("absent"), b"")?,
            }
        }
        let stored = match (&bytes, &folders) {
            (Value::Null, _) if record["kept"].is_string() => json!({ "kept": record["kept"] }),
            (Value::Null, Some(f)) => absent(&name, f),
            _ => bytes.clone(),
        };
        s.db.execute("INSERT OR IGNORE INTO board_before VALUES (?1,?2,?3)", params![tid, key, stored.to_string()])?;
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![tid, key, rid, key])?;
    }
    Ok(())
}

/// Records in `s` how to undo what a command changed there for task `tid`,
/// and returns the task's change entries for it.
pub fn touch(
    s: &Store,
    tid: &str,
    rid: &str,
    before: &Value,
    after: &Value,
    folders: Option<&HashSet<String>>,
) -> Result<Vec<Value>, Failure> {
    let mut out = vec![];
    let here = scope::dir(&scope::top(&s.root), &s.root);
    let keys: HashSet<_> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    for key in keys {
        if before[key] == after[key] {
            continue;
        }
        let data = &before[key];
        if let Some(name) = key.strip_prefix("file:") {
            let exists: bool =
                s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1 AND entity=?2)", params![tid, key], |r| r.get(0))?;
            if !exists {
                fs::create_dir_all(before_dir(&s.root, tid))?;
                let file = before_file(&s.root, tid, name);
                if let Some(bytes) = data.as_array() {
                    fs::write(file, jsonx::bytes(bytes))?;
                } else {
                    fs::write(file.with_extension("absent"), b"")?;
                }
            }
        }
        let name = if key.starts_with("symbol:") {
            let row = if after[key].is_null() { data } else { &after[key] };
            format!("{}.{}", row[1].as_str().unwrap_or(""), row[2].as_str().unwrap_or(""))
        } else {
            key.clone()
        };
        let stored = match (key.strip_prefix("file:"), folders) {
            (Some(name), Some(f)) if data.is_null() => absent(name, f),
            // A module the command added: its file's folders count too (D-136).
            (None, Some(f)) if data.is_null() && key.starts_with("module:") && after[key].is_array() => {
                let row = &after[key];
                let file = row[3]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| outside::file_of(row[1].as_str().unwrap_or(""), row[0].as_str().unwrap_or("")));
                absent(&file, f)
            }
            _ => data.clone(),
        };
        s.db.execute("INSERT OR IGNORE INTO board_before VALUES (?1,?2,?3)", params![tid, key, stored.to_string()])?;
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![tid, key, rid, name])?;
        out.push(json!({"entity":key,"name":name,"run":rid,"project":here}));
    }
    Ok(out)
}

/// Whether a recorded change is still worth reviewing: a file that is unwatched now (ignored after
/// the change was recorded, say) is not. Cancel and undo still restore it.
pub fn reviewable(root: &Path, key: &str) -> bool {
    let Some(name) = key.strip_prefix("file:") else { return true };
    lockdown::settings(root).map_or(true, |v| !lockdown::tier(root, &v, name, None).starts_with("unwatched"))
}

/// Review content stays internal, separate from notes (which humans can delete).
pub(crate) fn review_path(root: &Path, tid: &str, rid: &str) -> Result<PathBuf, Failure> {
    Ok(task_dir(root, tid)?.join("reviews").join(format!("{rid}.json")))
}

fn clear_reviews(s: &Store, tid: &str) -> Result<(), Failure> {
    let dir = task_dir(&s.root, tid)?.join("reviews");
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub(crate) fn review_state(s: &Store, t: &Value) -> Result<Value, Failure> {
    let changes = changes_of(s, t)?;
    let mut projects = json!({});
    let mut context = json!({});
    for (dir, st) in scope_stores(s, t)? {
        let current = snapshot(&st)?;
        let mut entities = json!({});
        for c in changes.iter().filter(|c| c["project"] == dir) {
            let key = c["entity"].as_str().unwrap();
            entities[key] = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
        }
        projects[&dir] = entities;
        let mut files = json!({});
        for (name, tier) in lockdown::tree_files(&st.root, &lockdown::settings(&st.root)?)? {
            if tier == "watched" || tier == "protected" {
                files[&name] = json!(sha256(&fs::read(st.root.join(&name))?));
            }
        }
        // Include materialized source too: an outside edit may not yet be absorbed into the store.
        for (key, row) in current.as_object().unwrap() {
            if let Some(module) = key.strip_prefix("module:") {
                let name = format!("{}/{module}.gleam", row[1].as_str().unwrap());
                files[&name] = json!(fs::read(st.root.join(&name)).ok().map(|b| sha256(&b)));
            }
        }
        // Gleam's dependency lock is deliberately unwatched, but affects review context.
        files["manifest.toml"] = json!(fs::read(st.root.join("manifest.toml")).ok().map(|b| sha256(&b)));
        context[dir] = json!([current, files]);
    }
    let instructions = json!([
        t["title"],
        t["body"],
        t["links"],
        t["questions"],
        t["notes"].as_array().unwrap().iter().filter(|n| n["author"] == "human").collect::<Vec<_>>()
    ]);
    Ok(json!({"projects":projects,"context":sha256(context.to_string().as_bytes()),
        "instructions":sha256(instructions.to_string().as_bytes())}))
}

pub(crate) fn previous_review(t: &Value) -> Option<&Value> {
    t["runs"].as_array()?.iter().rev().find(|r| r["kind"] == "review" && r["outcome"] == "reviewed" && r["state"] == "ended")
}

pub(crate) fn review_checkpoint(s: &Store, t: &Value) -> Result<Value, String> {
    let r = previous_review(t).ok_or("no completed review")?;
    let checkpoint = (|| -> Result<Value, Failure> {
        let p = review_path(&s.root, t["id"].as_str().unwrap(), r["id"].as_str().unwrap())?;
        jsonx::read(&p)
    })()
    .map_err(|_| "missing or legacy review checkpoint")?;
    if checkpoint["run"] != r["id"] || !checkpoint["projects"].is_object() {
        return Err("missing or legacy review checkpoint".into());
    }
    let scope = scope_stores(s, t).map_err(|e| e.0)?.into_iter().map(|(d, _)| d).collect::<HashSet<_>>();
    if scope != checkpoint["projects"].as_object().unwrap().keys().cloned().collect() {
        return Err("project scope changed since review".into());
    }
    Ok(checkpoint)
}

/// `task diff`: what each entity this task changed looked like before the task,
/// against now. `names` narrows it; past `budget`, the rest are listed by name.
pub fn task_diff(s: &Store, tid: &str, names: &[&str], budget: usize) -> Out {
    let t = task(&s.root, tid)?;
    let since = names.contains(&"--since-review");
    let names: Vec<_> = names.iter().copied().filter(|n| *n != "--since-review").collect();
    let checkpoint = since.then(|| review_checkpoint(s, &t));
    let baseline = checkpoint.as_ref().and_then(|c| c.as_ref().ok());
    let heading = match &checkpoint {
        Some(Ok(c)) => format!("Since completed review {} (content at review start):\n\n", c["run"].as_str().unwrap()),
        Some(Err(reason)) => format!("Full task diff: {reason}; falling back to the state before the task.\n\n"),
        None => String::new(),
    };
    let projects = scope_stores(s, &t)?;
    let changes = changes_of(s, &t)?;
    let (mut out, mut left) = (String::new(), vec![]);
    let mut unsafe_lines: Vec<String> = vec![];
    for (dir, st) in &projects {
        let current = snapshot(st)?;
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let before: HashMap<String, Value> = rows
            .query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .map(|r| r.map(|(k, v)| (k, serde_json::from_str(&v).unwrap())))
            .collect::<Result<_, _>>()?;
        let text = |key: &str, v: &Value| -> Option<String> {
            if key.starts_with("symbol:") {
                (v[7].as_i64() == Some(1)).then(|| v[4].as_str().unwrap_or("").to_string())
            } else if key.starts_with("module:") {
                v[2].as_str().map(|h| h.to_string())
            } else {
                let bytes = jsonx::bytes(v.as_array()?);
                Some(String::from_utf8(bytes).unwrap_or_else(|e| format!("(binary, {} bytes)", e.as_bytes().len())))
            }
        };
        for c in changes.iter().filter(|c| c["project"] == *dir) {
            let (key, name) = (c["entity"].as_str().unwrap(), c["name"].as_str().unwrap_or(""));
            let Some(original) = before.get(key) else { continue };
            // Entities first touched after the review use their task before-state.
            let old = baseline.and_then(|b| b["projects"][dir].get(key)).unwrap_or(original);
            if !reviewable(&st.root, key) {
                continue;
            }
            if !names.is_empty()
                && !names.iter().any(|n| *n == format!("{dir}: {name}") || *n == name || key.split_once(':').is_some_and(|(_, k)| k == *n))
            {
                continue;
            }
            let new = match key.strip_prefix("file:") {
                Some(f) => json!(fs::read(lockdown::path(&st.root, f)?).ok()),
                None => current[key].clone(),
            };
            if baseline.is_some() && old == &new {
                continue;
            }
            // A task across projects names each change's project (D-61).
            let shown = if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() };
            if crate::language::is_rust(&st.root)
                && let Some(line) = new_unsafe(&shown, old, &new)
            {
                unsafe_lines.push(line);
            }
            let part = match (text(key, old), text(key, &new)) {
                (a, b) if a == b && baseline.is_some() => format!("## {shown} (metadata or binary content changed)\n"),
                (a, b) if a == b => format!("## {shown} (back to its state before the task)\n"),
                // an extract into another project: shown once, with the diff, where it landed
                (Some(_), None) if let Some(there) = moved_to(&projects, dir, key, tid) => format!("## {shown} (moved to {there})\n"),
                (None, Some(b)) => format!("## {shown} (added)\n{}", line_diff("", &b)),
                (Some(a), None) => format!("## {shown} (deleted)\n{}", line_diff(&a, "")),
                (a, b) => format!("## {shown} (changed)\n{}", line_diff(&a.unwrap_or_default(), &b.unwrap_or_default())),
            };
            if !out.is_empty() && out.len() + part.len() > budget {
                left.push(shown);
            } else {
                out += &part;
                out.push('\n');
            }
        }
    }
    if !unsafe_lines.is_empty() {
        // D-158 point 2: a review list, not a refusal.
        out += "New `unsafe`:\n";
        for line in &unsafe_lines {
            out += line;
            out.push('\n');
        }
        out.push('\n');
    }
    if out.is_empty() {
        return Ok(heading
            + if baseline.is_some() && names.is_empty() {
                "No task entity changes since this review.\n"
            } else if names.is_empty() {
                "This task has no changes.\n"
            } else {
                "No change of this task matches.\n"
            });
    }
    if !left.is_empty() {
        out += &format!(
            "Left out to fit the reply; read with `task diff {}<name>`: {}\n",
            if since { "--since-review " } else { "" },
            left.join(", ")
        );
    }
    Ok(heading + &out)
}

/// A changed symbol that added `unsafe`, for the review (D-158 point 2):
/// `name`, marked when the new text has no `// SAFETY:` comment. This lists it
/// for the human; nothing is refused.
fn new_unsafe(name: &str, old: &Value, new: &Value) -> Option<String> {
    let (had, _) = rust_unsafe(old[4].as_str().unwrap_or(""));
    let (now, safety) = rust_unsafe(new[4].as_str().unwrap_or(""));
    (now > had).then(|| if safety { format!("- {name}") } else { format!("- {name} (no `// SAFETY:` comment)") })
}

/// The `unsafe` keywords in Rust text, outside comments, string and char
/// literals, and whether a comment holds `SAFETY:` (D-158 point 2). A name
/// that contains `unsafe` doesn't count.
fn rust_unsafe(text: &str) -> (usize, bool) {
    fn skip_string(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && b[i] != b'"' {
            i += if b[i] == b'\\' { 2 } else { 1 };
        }
        i + 1
    }
    /// A raw string's body start and its `#` count, when one begins at `i`.
    fn raw_start(b: &[u8], i: usize) -> Option<(usize, usize)> {
        let mut j = i;
        if b.get(j) == Some(&b'b') {
            j += 1;
        }
        if b.get(j) != Some(&b'r') {
            return None;
        }
        j += 1;
        let hashes = (j..b.len()).take_while(|&k| b[k] == b'#').count();
        j += hashes;
        (b.get(j) == Some(&b'"')).then_some((j + 1, hashes))
    }
    fn skip_raw(b: &[u8], from: usize, hashes: usize) -> usize {
        let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
        let mut i = from;
        while i + close.len() <= b.len() {
            if &b[i..i + close.len()] == close.as_slice() {
                return i + close.len();
            }
            i += 1;
        }
        b.len()
    }
    let b = text.as_bytes();
    let (mut i, mut count, mut safety) = (0, 0, false);
    while i < b.len() {
        if let Some((from, hashes)) = raw_start(b, i) {
            i = skip_raw(b, from, hashes);
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = text[i..].find('\n').map_or(b.len(), |k| i + k);
            safety |= text[i..end].contains("SAFETY:");
            i = end;
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            safety |= text[i..end].contains("SAFETY:");
            i = end;
        } else if b[i] == b'"' {
            i = skip_string(b, i + 1);
        } else if b[i] == b'\'' {
            if b.get(i + 1) == Some(&b'\\') {
                i += 2;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else if b.get(i + 2) == Some(&b'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if &text[start..i] == "unsafe" {
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    (count, safety)
}

/// The other project of `projects` where the symbol entity `key` is live: an
/// extract into another project moved it there (D-103). One that gained it in
/// task `tid` comes before one that held the same text all along.
pub(crate) fn moved_to(projects: &[(String, Store)], dir: &str, key: &str, tid: &str) -> Option<String> {
    if !key.starts_with("symbol:") {
        return None;
    }
    let live: Vec<&(String, Store)> =
        projects.iter().filter(|(d, s)| d != dir && snapshot(s).is_ok_and(|c| c[key][7].as_i64() == Some(1))).collect();
    let gained = |s: &Store| {
        s.db.query_row("SELECT data FROM board_before WHERE task=?1 AND entity=?2", [tid, key], |r| r.get::<_, String>(0))
            .is_ok_and(|d| d == "null")
    };
    live.iter().find(|(_, s)| gained(s)).or(live.first()).map(|(d, _)| d.clone())
}

/// Line diff with two lines of context; `…` marks skipped lines.
fn line_diff(a: &str, b: &str) -> String {
    let ops = diff_ops(a, b);
    let (mut out, mut gap) = (String::new(), false);
    for k in 0..ops.len() {
        if ops[k.saturating_sub(2)..(k + 3).min(ops.len())].iter().any(|o| o.0 != ' ') {
            if gap {
                out += "  …\n";
            }
            gap = false;
            out += &format!("{} {}\n", ops[k].0, ops[k].1);
        } else {
            gap = true;
        }
    }
    if gap {
        out += "  …\n";
    }
    out
}

/// The lines of `b` (from 0) that a line diff from `a` marks: added lines, and
/// the line where lines were removed.
pub(crate) fn changed_lines(a: &str, b: &str) -> Vec<usize> {
    let (mut out, mut line) = (vec![], 0);
    for (op, _) in diff_ops(a, b) {
        match op {
            '+' => {
                out.push(line);
                line += 1;
            }
            '-' => out.push(line.min(b.lines().count().saturating_sub(1))),
            _ => line += 1,
        }
    }
    out.dedup();
    out
}

pub fn diff_ops<'a>(a: &'a str, b: &'a str) -> Vec<(char, &'a str)> {
    let (a, b): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (x, y) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // O(n·m) LCS table over the changed middle; use Myers if whole-file rewrites get large.
    let mut l = vec![vec![0u32; y.len() + 1]; x.len() + 1];
    for i in (0..x.len()).rev() {
        for j in (0..y.len()).rev() {
            l[i][j] = if x[i] == y[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let mut ops: Vec<(char, &str)> = a[..pre].iter().map(|s| (' ', *s)).collect();
    let (mut i, mut j) = (0, 0);
    while i < x.len() || j < y.len() {
        if i < x.len() && j < y.len() && x[i] == y[j] {
            ops.push((' ', x[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < x.len() && (j == y.len() || l[i + 1][j] >= l[i][j + 1]) {
            ops.push(('-', x[i]));
            i += 1;
        } else {
            ops.push(('+', y[j]));
            j += 1;
        }
    }
    ops.extend(a[a.len() - suf..].iter().map(|s| (' ', *s)));
    ops
}

/// Snapshot entries for every regular file at or under `name` in `root`; none
/// for a path an agent can't reach, which the command refuses anyway.
pub fn files_under(root: &Path, name: &str) -> serde_json::Map<String, Value> {
    fn walk(p: &Path, name: &str, out: &mut serde_json::Map<String, Value>) {
        let Ok(m) = fs::symlink_metadata(p) else { return };
        if m.is_file() {
            out.insert(format!("file:{name}"), json!(fs::read(p).ok()));
        } else if m.is_dir() {
            for e in fs::read_dir(p).into_iter().flatten().flatten() {
                walk(&e.path(), &format!("{name}/{}", e.file_name().to_string_lossy()), out);
            }
        }
    }
    let mut out = serde_json::Map::new();
    let name = file_name(Path::new(name));
    if !name.is_empty()
        && !name.split('/').any(|c| c == ".solace")
        && let Ok(p) = lockdown::path(root, &name)
    {
        walk(&p, &name, &mut out);
    }
    out
}

pub fn file_snapshot(root: &Path, a: &[&str]) -> Result<Value, Failure> {
    let mut v = json!({});
    for name in paths(a) {
        let p = lockdown::path(root, name)?;
        let name = file_name(p.strip_prefix(root).unwrap());
        v[format!("file:{name}")] = json!(fs::read(p).ok());
    }
    // A move's files on both sides; a side in another project is recorded there (D-84).
    if a[0] == "move"
        && let Ok((from, to, dest)) = lockdown::move_args(root, a)
    {
        let m = v.as_object_mut().unwrap();
        m.extend(files_under(root, from));
        if crate::project_paths::same_project(root, &dest) {
            m.extend(files_under(root, to));
        }
    }
    if a[0] == "run" {
        fn collect(root: &Path, dir: &Path, settings: &toml::Value, out: &mut Value) -> Result<(), Failure> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let p = entry.path();
                let name = p.strip_prefix(root).unwrap().to_string_lossy();
                if entry.file_type()?.is_symlink()
                    || [".solace", ".git", "build"].contains(&name.as_ref())
                    || entry.file_name() == "node_modules"
                {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    collect(root, &p, settings, out)?;
                } else if lockdown::tier(root, settings, &name, None) == "watched" {
                    out[format!("file:{name}")] = json!(fs::read(&p)?);
                }
            }
            Ok(())
        }
        collect(root, root, &lockdown::settings(root)?, &mut v)?;
    }
    Ok(v)
}

pub(crate) fn restore(s: &Store, states: &[(String, Value)]) -> Result<(), Failure> {
    let mut kept = HashMap::new();
    for (key, v) in states {
        if let Some(id) = key.strip_prefix("symbol:") {
            if v.is_null() {
                s.db.execute("DELETE FROM symbols WHERE id=?", [id])?;
            } else {
                // A snapshot from before the nested columns has only the first eight (D-158).
                s.db.execute(
                    "INSERT OR REPLACE INTO symbols(id,module,name,kind,text,ord,revision,live,parent,sep,tail) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        v[0].as_str(),
                        v[1].as_str(),
                        v[2].as_str(),
                        v[3].as_str(),
                        v[4].as_str(),
                        v[5].as_i64(),
                        v[6].as_str(),
                        v[7].as_i64(),
                        v.get(8).and_then(|x| x.as_str()),
                        v.get(9).and_then(|x| x.as_str()),
                        v.get(10).and_then(|x| x.as_str()).unwrap_or("")
                    ],
                )?;
            }
        } else if let Some(name) = key.strip_prefix("module:") {
            if !v.is_array() {
                // Where its file's folders stop going, when recorded (D-136).
                if let Some(k) = v["kept"].as_str() {
                    // A Rust module's row holds its file; a Gleam one its dir (D-168).
                    let row: Option<(String, Option<String>)> =
                        s.db.query_row("SELECT dir, file FROM modules WHERE path=?", [name], |r| Ok((r.get(0)?, r.get(1)?))).ok();
                    if let Some((dir, file)) = row {
                        kept.insert(file.unwrap_or_else(|| outside::file_of(&dir, name)), k.to_string());
                    }
                }
                // Its file goes in the `materialize` below, which keeps one edited outside Solace (D-58).
                s.db.execute("DELETE FROM modules WHERE path=?", [name])?;
            } else {
                // ON CONFLICT keeps a Rust module's `file` and `owns_dir`; a snapshot from
                // before them reads as None.
                let (path, dir, header) = (v[0].as_str(), v[1].as_str(), v[2].as_str());
                let (file, owns) = (v.get(3).and_then(|x| x.as_str()), v.get(4).and_then(|x| x.as_i64()));
                s.db.execute(
                    "INSERT INTO modules(path,dir,header,file,owns_dir) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(path) DO UPDATE SET dir=excluded.dir, header=excluded.header, file=excluded.file, owns_dir=excluded.owns_dir",
                    params![path, dir, header, file, owns],
                )?;
            }
        } else if let Some(name) = key.strip_prefix("file:") {
            let p = lockdown::path(&s.root, name)?;
            if let Some(bytes) = v.as_array() {
                fs::create_dir_all(p.parent().unwrap())?;
                fs::write(p, jsonx::bytes(bytes))?;
            } else if p.exists() {
                fs::remove_file(&p)?;
                // with the folders that leaves empty, such as those a carried file made (D-103),
                // up to the top folder or the one recorded as there before (D-136)
                let top = name.split('/').next().unwrap_or_default();
                let stop = v["kept"].as_str().filter(|k| k.len() > top.len()).unwrap_or(top);
                outside::remove_empty_dirs(&s.root.join(stop), p.parent());
            }
        }
    }
    let ix = s.index()?;
    s.materialize_to(&ix, &s.root, &kept)?;
    Ok(())
}

/// Whether the check after an undo runs in the project: a project with a
/// language, or a Rust project whose toolchain is there (D-158). Without the
/// toolchain the undo is unchecked, not refused.
fn checkable(root: &Path, cargo: bool) -> bool {
    crate::language::detect(root).is_some() || (crate::language::is_rust(root) && cargo)
}

/// Undoes a task's changes in every project of its scope (D-61): each is
/// restored and checked, and a failure puts back the projects already restored.
pub(crate) fn revert(s: &Store, t: &mut Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap().to_string();
    let projects = scope_stores(s, t)?;
    // Malformed settings are reported before recovery, which may undo another run's work.
    let cargo =
        projects.iter().any(|(_, st)| crate::language::is_rust(&st.root)) && crate::language::reason(crate::language::RUST)?.is_none();
    // An extract a stopped run left between its projects' commits is undone first (D-135).
    ahead::recover(&projects.iter().map(|(_, st)| st.root.clone()).collect::<Vec<_>>())?;
    // Under the locks, what a stopped run left half-written becomes the task's own rows first.
    for (_, st) in &projects {
        lockdown::recover(&st.root, st, &[])?;
    }
    let changes = changes_of(s, t)?;
    let named = |dir: &str, key: &str| -> String {
        let name = changes.iter().find(|c| c["entity"] == key && c["project"] == dir);
        let name = name.and_then(|c| c["name"].as_str()).unwrap_or(key);
        if projects.len() > 1 { format!("{dir}: {name}") } else { name.to_string() }
    };
    let (mut plans, mut conflicts) = (vec![], vec![]);
    for (dir, st) in &projects {
        let mut rows = st.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
        let mut states = vec![];
        for row in rows.query_map([&tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (k, v) = row?;
            states.push((k, serde_json::from_str::<Value>(&v).unwrap()));
        }
        for (key, _) in &states {
            let later:Option<String>=st.db.query_row("SELECT task FROM board_touches WHERE entity=?1 AND seq > (SELECT MIN(seq) FROM board_touches WHERE task=?2 AND entity=?1) AND task!=?2 ORDER BY seq DESC LIMIT 1",params![key,tid],|r|r.get(0)).optional()?;
            if let Some(other) = later {
                conflicts.push(format!("{}: task {other}", named(dir, key)));
            }
        }
        plans.push((dir, st, states));
    }
    if !conflicts.is_empty() {
        return err("E7007", conflicts.join(", "));
    }
    // Every project is restored before any is checked: one reads another from
    // disk through a path dependency, as an extract's dependents do (D-103).
    let mut done: Vec<(&Store, Vec<(String, Value)>)> = vec![];
    let mut failed: Option<(&str, &Store, Failure)> = None;
    let plans: Vec<_> = plans.into_iter().filter(|p| !p.2.is_empty()).collect();
    for (dir, st, states) in &plans {
        let current = snapshot(st)?;
        let here = existing_folders(&st.root);
        let mut undo = vec![];
        for (key, _) in states {
            let v = if let Some(name) = key.strip_prefix("file:") {
                // a rollback keeps the folders there now (D-136)
                fs::read(lockdown::path(&st.root, name)?).map_or_else(|_| absent(name, &here), |b| json!(b))
            } else {
                current[key].clone()
            };
            undo.push((key.clone(), v));
        }
        st.db.execute_batch("SAVEPOINT revert_task")?;
        done.push((*st, undo));
        if let Err(e) = restore(st, states) {
            failed = Some((dir.as_str(), *st, e));
            break;
        }
    }
    if failed.is_none() {
        for (dir, st, _) in &plans {
            // A project of plain files (D-60) has no code to type-check; the restored files say
            // which it is now.
            if checkable(&st.root, cargo)
                && let Err(e) = st.check()
            {
                failed = Some((dir.as_str(), *st, e));
                break;
            }
        }
    }
    match failed {
        Some((dir, st, e)) => {
            for (earlier, undo) in done.iter().rev() {
                earlier.db.execute_batch("ROLLBACK TO revert_task; RELEASE revert_task")?;
                restore(earlier, undo)?;
            }
            let other = tasks(&st.root)?
                .iter()
                .filter(|x| x["id"] != tid.as_str())
                .map(|x| {
                    let names: Vec<String> =
                        changes_of(st, x).unwrap_or_default().iter().map(|c| c["name"].as_str().unwrap_or("").to_string()).collect();
                    format!("task {}: {}", x["id"], names.join(", "))
                })
                .collect::<Vec<_>>()
                .join("\n");
            let place = if projects.len() > 1 { format!("in {dir}: ") } else { String::new() };
            return err("E7008", format!("{place}{}\n{other}", e.0));
        }
        None => {
            for (st, _) in &done {
                st.db.execute_batch("RELEASE revert_task")?;
            }
        }
    }
    for (_, st) in &projects {
        st.db.execute("DELETE FROM board_touches WHERE task=?", [&tid])?;
        drafts::clear(st, &tid)?;
        drop_before(st, &tid)?;
        outside::checkpoint(st)?;
    }
    clear_reviews(s, &tid)?;
    end_runs(t, "reverted");
    Ok(())
}

/// A completed task's changes become approved in every project it changed.
pub(crate) fn settle(s: &Store, t: &Value) -> Result<(), Failure> {
    let tid = t["id"].as_str().unwrap();
    clear_reviews(s, tid)?;
    let (mut pending, mut failed) = (vec![], vec![]);
    for (dir, st) in scope_stores(s, t)? {
        lockdown::recover(&st.root, &st, &[])?;
        outside::approve(&st, tid)?;
        drop_before(&st, tid)?;
        match outside::checkpoint_pending(&st) {
            Ok(false) => {}
            Ok(true) => pending.push(dir),
            Err(e) => failed.push((dir, e)),
        }
    }
    settle_result(pending, failed)
}

/// The body of a Tech Debt task (D-112).
const TECH_DEBT_BODY: &str = "Minor, non-blocking findings from reviews, one note each, filed when the reviewed task was completed.";

/// The findings of a note's `### Tech debt` section (D-112): one per top-level
/// bullet (`-`, `*`, `+` or `1.`), with the indented lines below it. The
/// section ends at the next heading; no section gives none.
pub(crate) fn tech_debt_items(text: &str) -> Vec<String> {
    let (mut items, mut inside): (Vec<String>, bool) = (vec![], false);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            inside = trimmed.trim_start_matches('#').trim().trim_end_matches(':').eq_ignore_ascii_case("tech debt");
            continue;
        }
        if !inside || trimmed.is_empty() {
            continue;
        }
        let numbered = line.split_once(". ").filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(_, rest)| rest);
        match ["- ", "* ", "+ "].iter().find_map(|b| line.strip_prefix(b)).or(numbered) {
            Some(item) => items.push(item.trim().to_string()),
            None => {
                if let Some(last) = items.last_mut() {
                    *last += "\n";
                    *last += line.trim_end();
                }
            }
        }
    }
    items.retain(|i| !i.is_empty());
    items
}

/// The task's last review: the run id and the agent's note of its last review
/// run that ended `reviewed`.
fn last_review(t: &Value) -> Option<(String, String)> {
    last_run_note(t, "review", "reviewed")
}

/// What completing task `t` files as tech debt (D-112, D-199): its feature's
/// open Tech Debt task (`techDebt: true`, not in a final column) with one
/// `solace` note per finding of the last work run's summary and the last
/// review's `### Tech debt`, naming the task and run they came from. With no
/// open one, a new one is made in Planning of `t`'s home (its first column if
/// it has none, D-132), with its scope, titled with just the feature's title.
/// A task with no feature always gets a new one titled with just the completed
/// task's title, with no feature or milestone. Returns the task with the
/// counts of work-run and review items; None when neither lists tech debt.
/// Nothing is written here.
pub(crate) fn tech_debt(top: &Path, t: &Value) -> Result<Option<(Value, usize, usize)>, Failure> {
    let tid = t["id"].as_str().unwrap_or("");
    let mut items: Vec<String> = Vec::new();
    let (mut work, mut review) = (0, 0);
    if let Some((rid, text)) = last_review(t) {
        let found = tech_debt_items(&text);
        review = found.len();
        items.extend(found.into_iter().map(|item| format!("Tech debt from the review of task:{tid} (review run {rid}):\n\n{item}")));
    }
    if let Some((rid, text)) = last_work(t) {
        let found = tech_debt_items(&text);
        work = found.len();
        items.extend(found.into_iter().map(|item| format!("Tech debt from the work on task:{tid} (work run {rid}):\n\n{item}")));
    }
    if items.is_empty() {
        return Ok(None);
    }
    let fid = t["feature"].as_str();
    let feature = fid.and_then(|f| tasks::record(top, "features", f));
    let mut debt = match (fid, feature) {
        (Some(fid), Some(feature)) => {
            let open = |o: &Value| flow(&scope::path(top, o["home"].as_str().unwrap_or("."))).is_ok_and(|w| crate::board::task_model::column(&w, o)["final"] != true);
            let found = all_tasks(top)?.into_iter().find(|o| o["techDebt"] == true && o["feature"] == fid && o["id"] != tid && open(o));
            match found {
                Some(d) => d,
                None => new_debt(top, t, Some(&feature))?,
            }
        }
        (Some(_), None) => return Ok(None),
        (None, _) => new_debt(top, t, None)?,
    };
    for item in &items {
        note(&mut debt, "solace", None, item);
    }
    Ok(Some((debt, work, review)))
}

/// Settles task `tid` once the task server has moved it into a final column
/// (D-77 point 5, Option B): a step of its own, after the reply, that takes the
/// scope's locks the human action took. It reads the task again
/// under them, so a task that has left the final column since is not settled, and
/// it can run again: approving and dropping records twice changes nothing.
pub(crate) fn settle_final(top: &Path, tid: &str) -> Result<(), Failure> {
    // A task with no project (D-129) has no scope to settle, and settling it
    // at its home would create a store there.
    let t = task_at_home(top, tid)?;
    if t["noProject"] == true {
        return Ok(());
    }
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let _held = guard(&home, vec!["tasks", tid, "settle"], &json!({}))?;
    let t = task(&home, tid)?;
    if crate::board::task_model::column(&flow(&home)?, &t)["final"] == true {
        settle(&open(&home)?, &t)?;
    }
    Ok(())
}

/// Settles every task already in a final column that still has undo records or
/// whose deferred settle failed, which a stop, a crash, or a failed flush leaves
/// behind.
pub(crate) fn resettle(top: &Path) {
    for t in all_tasks(top).unwrap_or_default() {
        let (Some(tid), Some(home)) = (t["id"].as_str(), t["home"].as_str()) else { continue };
        let home = scope::path(top, home);
        let marked = t["settleFailed"] == true;
        if !marked {
            let Ok(w) = flow(&home) else { continue };
            if crate::board::task_model::column(&w, &t)["final"] != true {
                continue;
            }
        }
        let left = open(&home).and_then(|s| scope_stores(&s, &t)).is_ok_and(|stores| {
            stores.iter().any(|(_, st)| {
                st.db
                    .query_row("SELECT EXISTS(SELECT 1 FROM board_before WHERE task=?1)", params![tid], |r| r.get::<_, bool>(0))
                    .unwrap_or(false)
                    || lockdown::pending_for(&st.root, tid)
            })
        });
        if left || marked {
            tasks::settle_later(top, tid.to_string());
        }
    }
}

/// Where a task keeps files as they were before it changed them.
pub fn before_dir(root: &Path, tid: &str) -> PathBuf {
    root.join(".solace/board").join(tid).join("before")
}

/// File `name` as it was before the task changed it; with extension `absent`
/// when it didn't exist.
pub fn before_file(root: &Path, tid: &str, name: &str) -> PathBuf {
    before_dir(root, tid).join(sha256(name.as_bytes()))
}

/// Changes whenever the store's code does.
pub fn fingerprint(s: &Store) -> Out {
    Ok(sha256(snapshot(s)?.to_string().as_bytes()))
}

pub(crate) fn drop_before(s: &Store, tid: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM board_before WHERE task=?", [tid])?;
    let p = before_dir(&s.root, tid);
    if p.exists() {
        fs::remove_dir_all(p)?;
    }
    Ok(())
}

/// The task is settled once every project's writes are approved; a busy flush
/// leaves only `store.db` behind, while any other failure keeps its own error.
pub(crate) fn settle_result(pending: Vec<String>, failed: Vec<(String, Failure)>) -> Result<(), Failure> {
    if let Some((_, e)) = failed.first() {
        let reasons: Vec<String> = failed.iter().map(|(dir, e)| format!("{dir}: {}", e.0)).collect();
        let also =
            if pending.is_empty() { String::new() } else { format!("\nAlso, flushing store.db is pending in {}.", pending.join(", ")) };
        return Err(Failure(format!("{}{also}", reasons.join("\n")), e.1));
    }
    if pending.is_empty() {
        return Ok(());
    }
    err("E7028", format!("the task is settled; only flushing store.db is pending in {}", pending.join(", ")))
}

/// Marks whether the project or instructions changed since the run's review checkpoint.
fn review_changed(s: &Store, t: &Value, rid: &str) -> Result<bool, Failure> {
    let path = review_path(&s.root, t["id"].as_str().unwrap(), rid)?;
    let Ok(mut checkpoint) = jsonx::read(&path) else {
        return Ok(false);
    };
    let current = review_state(s, t)?;
    let changed = checkpoint["context"] != current["context"]
        || checkpoint["instructions"] != current["instructions"]
        || checkpoint["projects"] != current["projects"];
    checkpoint["changed_during_review"] = json!(changed);
    jsonx::write(&path, &checkpoint)?;
    Ok(changed)
}

/// The agent message a review's feedback travels in, after its checkpoint is marked.
pub(crate) fn review_message(s: &Store, tid: &str, t: &Value, rid: &str, text: &str) -> Result<Value, Failure> {
    let mut m = json!({"kind":"agent","task":tid,"run":rid,"command":"note","text":text});
    m["changed_during_review"] = json!(review_changed(s, t, rid)?);
    Ok(m)
}

/// Whether an agent note for this review run is already on the task.
fn review_feedback_recorded(t: &Value, rid: &str) -> bool {
    t["notes"].as_array().is_some_and(|notes| notes.iter().any(|n| n["author"] == "agent" && n["run"] == rid))
}

/// A review's final reply counts when it holds one assessment label, with no other label disagreeing.
fn review_assessment(reply: &str) -> bool {
    let mut labels = reply.lines().filter_map(assessment_label);
    let Some(first) = labels.next() else {
        return false;
    };
    labels.all(|label| label == first)
}

/// A clean review's final reply, when nothing already recorded its feedback.
pub(crate) fn recoverable_review(r: &Value, success: bool, entries: &[Value]) -> Option<String> {
    if r["kind"] != "review" || !success {
        return None;
    }
    if crate::board::runs::has_blocking_agent_error(entries) {
        return None;
    }
    let recorded = entries.iter().any(|e| {
        e["kind"] == "tool"
            && e["title"].as_str().is_some_and(|title| title.contains("task note"))
            && e["output"].as_str().is_some_and(note_recorded)
    });
    if recorded {
        return None;
    }
    let after_tool = entries.iter().rposition(|e| e["kind"] == "tool").map_or(0, |i| i + 1);
    let reply =
        entries[after_tool..].iter().filter(|e| e["kind"] == "message").filter_map(|e| e["text"].as_str()).collect::<Vec<_>>().join("\n\n");
    review_assessment(&reply).then(|| reply.trim().to_string())
}

/// Records a review's recovered assessment through the same path as `task note`.
pub(crate) fn record_review(watch: &Watch, rid: &str, text: &str) -> Result<bool, Failure> {
    let Watch::Task(root, tid) = watch else {
        return Ok(false);
    };
    let root = root.to_path_buf();
    let tid = tid.to_string();
    let rid = rid.to_string();
    let current = task(&root, &tid)?;
    if review_recorded(&current, &rid)? {
        return Ok(false);
    }
    let m = review_message(&open(&root)?, &tid, &current, &rid, text)?;
    let w = flow(&root)?;
    let update_tid = tid.clone();
    tasks::update(&root, &update_tid, move |t| {
        if review_recorded(t, &rid)? {
            return Ok(false);
        }
        message(&w, t, &m)?;
        Ok(true)
    })
}

/// Whether review `rid`'s feedback is already on the task or its run has ended.
fn review_recorded(t: &Value, rid: &str) -> Result<bool, Failure> {
    let i = run_at(t, rid)?;
    Ok(t["runs"][i]["state"] == "ended" || review_feedback_recorded(t, rid))
}

/// A new Tech Debt task for completed task `t`: in Planning of its home (its
/// first column if it has none, D-132), with its scope. A `feature` names the
/// task it serves and supplies its milestone and title; without one, it is
/// titled after `t` and belongs to no feature or milestone.
fn new_debt(top: &Path, t: &Value, feature: Option<&Value>) -> Result<Value, Failure> {
    let named = feature.unwrap_or(t);
    let title = named["title"].as_str().unwrap_or("").trim();
    let home = scope::path(top, t["home"].as_str().unwrap_or("."));
    let rank = tasks(&home)?.iter().filter_map(|x| x["rank"].as_f64()).fold(0., f64::max) + 1.;
    let w = flow(&home)?;
    let first = planning_column(&w);
    let mut d = new_task(title, TECH_DEBT_BODY, &first, rank);
    d["techDebt"] = json!(true);
    d["feature"] = t["feature"].clone();
    d["milestone"] = feature.map(|f| f["milestone"].clone()).unwrap_or(Value::Null);
    d["scope"] = t["scope"].clone();
    d["home"] = t["home"].clone();
    Ok(d)
}

/// The task's last `kind` run that ended `outcome`: the run id and the agent's
/// note of it.
fn last_run_note(t: &Value, kind: &str, outcome: &str) -> Option<(String, String)> {
    let r = t["runs"].as_array()?.iter().rev().find(|r| r["kind"] == kind && r["outcome"] == outcome)?;
    let rid = r["id"].as_str()?;
    let n = t["notes"].as_array()?.iter().rev().find(|n| n["run"] == rid && n["author"] == "agent")?;
    Some((rid.to_string(), n["text"].as_str()?.to_string()))
}

/// The task's last work run that ended `finished`: the run id and the agent's
/// note of it.
fn last_work(t: &Value) -> Option<(String, String)> {
    last_run_note(t, "work", "finished")
}

pub fn record_verification(s: &Store, tid: &str, command: &str, result: &Out) -> Result<(), Failure> {
    let t = task(&s.root, tid)?;
    let summary = match result {
        Ok(out) => out,
        Err(e) => &e.0,
    };
    let result =
        json!({"passed":result.is_ok(),"base":review_state(s, &t)?["context"],"summary":summary.chars().take(1000).collect::<String>()});
    tasks::send(&scope::top(&s.root), json!({"kind":"verification","task":tid,"command":command,"result":result}))?;
    Ok(())
}

/// Whether a tool reply says the task note was recorded, including in a batch.
fn note_recorded(output: &str) -> bool {
    fn header(line: &str) -> Option<&str> {
        line.strip_prefix("==== solace ")?.strip_suffix(" ====")
    }
    let batch = output.lines().next().and_then(|line| header(line)).is_some();
    if !batch {
        return output.lines().next().is_some_and(|line| line.trim() == "Recorded.");
    }
    let mut in_note = false;
    for line in output.lines() {
        if let Some(command) = header(line) {
            in_note = command.trim() == "task note";
        } else if in_note && line.trim() == "Recorded." {
            return true;
        }
    }
    false
}

const CHANGED_DURING_REVIEW: &str = concat!(
    "Project or instructions changed during review. This feedback applies to the review-start checkpoint, ",
    "not a verified finish-time state; recheck affected changes.",
);

/// Whether a review ended for a reason its late feedback can still finish.
/// A stopped, canceled or replaced run was ended on purpose; an agent's exit or
/// a server restart was not.
fn ended_failed(r: &Value) -> bool {
    r["failed"] == true
}

/// Records review feedback, including a note that arrives after its run ended.
pub(crate) fn record_note(t: &mut Value, rid: &str, text: &str, changed_during_review: bool) -> Result<(), Failure> {
    let i = run_at(t, rid)?;
    if t["runs"][i]["state"] == "ended" {
        if review_feedback_recorded(t, rid) {
            return Ok(());
        }
        if !ended_failed(&t["runs"][i]) {
            note(t, "agent", Some(rid), text);
            return Ok(());
        }
        if t["attention"]["reason"].as_str().is_some_and(|reason| reason.starts_with(crate::board::runs::UNFINISHED_EXIT)) {
            t["attention"] = Value::Null;
        }
    }
    note(t, "agent", Some(rid), text);
    if changed_during_review {
        note(t, "solace", Some(rid), CHANGED_DURING_REVIEW);
    }
    crate::board::task_model::end_run(&mut t["runs"][i], "reviewed");
    t["finished"] = json!({"kind":"review","outcome":"reviewed","time":crate::now()});
    Ok(())
}

/// The assessment a line names, once surrounding Markdown heading and emphasis markers are removed.
fn assessment_label(line: &str) -> Option<&'static str> {
    let line = line.trim().trim_start_matches('#').trim();
    let marker = line.chars().next().filter(|c| matches!(c, '*' | '_'));
    let plain = marker
        .and_then(|marker| {
            let markers: String = line.chars().take_while(|c| *c == marker).collect();
            let rest = line.strip_prefix(&markers)?;
            if let Some(inner) = rest.strip_suffix(&markers) {
                return Some(inner.trim().to_string());
            }
            let after = rest.strip_prefix("Assessment:")?.strip_prefix(&markers)?;
            Some(format!("Assessment:{}", after).trim().to_string())
        })
        .unwrap_or_else(|| line.to_string());
    match plain.as_str() {
        "Assessment: Looks good" => Some("Assessment: Looks good"),
        "Assessment: Needs work" => Some("Assessment: Needs work"),
        _ => None,
    }
}
