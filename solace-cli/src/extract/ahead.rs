//! A cross-project extract, written ahead (D-103): one record, a copy in each
//! of its projects' `.solace/pending/`, written before the command changes any
//! file and completed with the computed result (each project's changed rows
//! and files, before and after, and the gleam.toml lines it gains) before the
//! first commit. The destination commits first, then the source and the
//! dependents. A crash anywhere in between is finished or rolled back at the
//! next open, so an extract never stays half-applied.
use super::*;
use std::collections::{BTreeSet, HashSet};

/// The record's file in project `root`.
fn path(root: &Path, rid: &str) -> PathBuf {
    root.join(".solace/pending").join(format!("extract-{rid}.json"))
}

/// Whether a pending record is an extract's.
pub(crate) fn is_record(v: &Value) -> bool {
    v["extract"].is_object()
}

/// Whether project `root` holds an extract's record: its files may be half
/// written, so nothing reads them in as outside changes until it is settled.
pub(crate) fn waiting(root: &Path) -> bool {
    fs::read_dir(root.join(".solace/pending")).is_ok_and(|d| {
        d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("extract-"))
    })
}

fn run(record: &Value) -> &str {
    record["run"].as_str().unwrap_or("")
}

fn roots(record: &Value) -> Vec<PathBuf> {
    record["extract"]["projects"].as_array().into_iter().flatten().filter_map(|p| p["root"].as_str()).map(PathBuf::from).collect()
}

/// Writes `record` into each of its projects.
fn put(record: &Value) -> Result<(), Failure> {
    for root in roots(record) {
        fs::create_dir_all(root.join(".solace/pending"))?;
        write_atomic(&path(&root, run(record)), record.to_string())?;
    }
    Ok(())
}

/// Removes the record from each of its projects.
fn clear(record: &Value) -> Result<(), Failure> {
    for root in roots(record) {
        let p = path(&root, run(record));
        if p.exists() {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

/// Every file under src/ and test/, by path, with its hash: what recovery
/// compares to tell an edit made outside Solace from the extract's own.
fn hashes(root: &Path) -> Value {
    fn walk(root: &Path, dir: &Path, out: &mut serde_json::Map<String, Value>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else if let (Ok(bytes), Ok(rel)) = (fs::read(&p), p.strip_prefix(root)) {
                out.insert(rel.to_string_lossy().into_owned(), json!(sha256(bytes)));
            }
        }
    }
    let mut out = serde_json::Map::new();
    for d in ["src", "test"] {
        walk(root, &root.join(d), &mut out);
    }
    Value::Object(out)
}

/// A snapshot's file bytes as text.
fn text(v: &Value) -> String {
    v.as_array().map(|a| String::from_utf8_lossy(&json_bytes(a)).into_owned()).unwrap_or_default()
}

/// The dependency lines `after` has and `before` hasn't; a table header is
/// `add_dependency`'s to make.
fn gained(before: &str, after: &str) -> Vec<String> {
    let had: HashSet<&str> = before.lines().map(str::trim).collect();
    after.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('[') && !had.contains(l.trim())).map(String::from).collect()
}

/// Adds to `root`'s gleam.toml each line it lacks; the rest of the file, edited
/// outside Solace or not, stays as it is.
fn merge_lines(root: &Path, lines: &Value) -> Result<(), Failure> {
    let p = root.join("gleam.toml");
    let Ok(now) = fs::read_to_string(&p) else { return Ok(()) };
    let mut text = now.clone();
    for line in lines.as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !text.lines().any(|l| l.trim() == line.trim()) {
            text = add_dependency(&text, line);
        }
    }
    if text != now {
        fs::write(p, text)?;
    }
    Ok(())
}

/// Step 1, before the command writes any file: the record, with each
/// project's files as they are. `roots`: the source, the destination, then the dependents.
pub(crate) fn begin(tid: &str, rid: &str, roots: &[&Path]) -> Result<Value, Failure> {
    let projects: Vec<Value> = roots.iter().map(|r| json!({"root": r, "files_before": hashes(r)})).collect();
    let record = json!({"task": tid, "run": rid, "extract": {"projects": projects, "complete": false}});
    put(&record)?;
    Ok(record)
}

/// Before the first commit: the computed result, so finishing replays it and
/// never computes another plan. `stores` in `begin`'s order, each with its
/// snapshots before and after; its files are written out first.
pub(crate) fn complete(record: &mut Value, stores: &[(&Store, &Value, &Value)]) -> Result<(), Failure> {
    for (i, (s, before, after)) in stores.iter().enumerate() {
        s.materialize(&s.index()?)?;
        let (mut b, mut a) = (serde_json::Map::new(), serde_json::Map::new());
        let keys: BTreeSet<&String> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
        for k in keys {
            if before[k.as_str()] != after[k.as_str()] {
                b.insert(k.clone(), before[k.as_str()].clone());
                a.insert(k.clone(), after[k.as_str()].clone());
            }
        }
        let p = &mut record["extract"]["projects"][i];
        p["before"] = Value::Object(b);
        p["after"] = Value::Object(a);
        p["toml"] = json!(gained(&text(&before["file:gleam.toml"]), &text(&after["file:gleam.toml"])));
        p["files_after"] = hashes(&s.root);
    }
    record["extract"]["complete"] = json!(true);
    put(record)
}

/// The last steps, after every commit: the gleam.toml lines are merged, and the
/// run's undo records and this record go.
pub(crate) fn end(record: &Value) -> Result<(), Failure> {
    for p in record["extract"]["projects"].as_array().into_iter().flatten() {
        let root = PathBuf::from(p["root"].as_str().unwrap_or(""));
        merge_lines(&root, &p["toml"])?;
        lockdown::drop_records(&root, run(record))?;
    }
    clear(record)
}

/// Settles every extract record in the projects `held`, whose locks the caller
/// holds; another project of the record is locked here without waiting, so two
/// holders never wait on each other (`E7028` when one is busy). A record kept
/// for a human, both sides having changed outside Solace, refuses (`E7029`):
/// nothing may change those projects, or read their edits in, until one decides.
pub(crate) fn recover(held: &[PathBuf]) -> Result<(), Failure> {
    let (mut seen, mut kept) = (HashSet::new(), vec![]);
    for root in held {
        let Ok(dir) = fs::read_dir(root.join(".solace/pending")) else { continue };
        let mut found = vec![];
        for e in dir.flatten() {
            let p = e.path();
            if !e.file_name().to_string_lossy().starts_with("extract-") {
                continue;
            }
            // A record is written by rename, so one that can't be read is no record.
            match fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(r) if is_record(&r) => found.push(r),
                _ => fs::remove_file(&p)?,
            }
        }
        for r in found {
            if !seen.insert(run(&r).to_string()) {
                continue;
            }
            let mut _locks = vec![];
            for p in roots(&r).iter().filter(|p| p.exists() && !held.iter().any(|h| lockdown::same_project(h, p))) {
                match board::try_lock(p) {
                    Some(l) => _locks.push(l),
                    None => {
                        return Err(Failure(
                            format!("E7028 an interrupted extract also changed {}, which another command holds now; try again", p.display()),
                            1,
                        ));
                    }
                }
            }
            kept.extend(settle(&r)?);
        }
    }
    if kept.is_empty() { Ok(()) } else { Err(Failure(format!("E7029 {}", kept.join("\n")), 1)) }
}

/// The files of project `p` that match neither its state before the extract
/// nor after it: edited outside Solace since.
fn edited(p: &Value) -> Vec<String> {
    let now = hashes(Path::new(p["root"].as_str().unwrap_or("")));
    let mut paths = BTreeSet::new();
    for v in [&now, &p["files_before"], &p["files_after"]] {
        paths.extend(v.as_object().into_iter().flatten().map(|(k, _)| k.clone()));
    }
    paths.into_iter().filter(|k| now[k.as_str()] != p["files_before"][k.as_str()] && now[k.as_str()] != p["files_after"][k.as_str()]).collect()
}

/// Whether project `p`'s part of the extract committed: its rows are as the
/// record has them after, or, with files only, its undo rows are there.
fn done(s: &Store, p: &Value, tid: &str, rid: &str) -> Result<bool, Failure> {
    let after = p["after"].as_object().cloned().unwrap_or_default();
    let snap = board::snapshot(s)?;
    let rows: Vec<&String> = after.keys().filter(|k| !k.starts_with("file:")).collect();
    if !rows.is_empty() {
        return Ok(rows.iter().all(|k| snap[k.as_str()] == after[k.as_str()]));
    }
    for k in after.keys() {
        let n: bool = s.db.query_row("SELECT EXISTS(SELECT 1 FROM board_touches WHERE task=?1 AND run=?2 AND entity=?3)", params![tid, rid, k], |r| r.get(0))?;
        if !n {
            return Ok(false);
        }
    }
    Ok(true)
}

/// One record, under every lock of its projects: finished, rolled back, or kept
/// for a human (returned as text).
fn settle(r: &Value) -> Result<Option<String>, Failure> {
    let (tid, rid) = (r["task"].as_str().unwrap_or(""), run(r));
    let ps: Vec<Value> = r["extract"]["projects"].as_array().cloned().unwrap_or_default();
    let stores: Vec<Store> = ps.iter().map(|p| board::open(Path::new(p["root"].as_str().unwrap_or("")))).collect::<Result<_, _>>()?;
    // The destination never committed: the extract never happened.
    if r["extract"]["complete"] != true || stores.len() < 2 || !done(&stores[1], &ps[1], tid, rid)? {
        for (s, p) in stores.iter().zip(&ps) {
            roll_back(s, p, rid, &[])?;
        }
        return clear(r).map(|_| None);
    }
    // The source and the dependents are one side, the destination the other.
    let edits: Vec<Vec<String>> = ps.iter().map(edited).collect();
    let named = |i: usize| edits[i].iter().map(|f| format!("{}/{f}", ps[i]["root"].as_str().unwrap_or(""))).collect::<Vec<_>>();
    let source_side = edits.iter().enumerate().any(|(i, e)| i != 1 && !e.is_empty());
    if source_side && !edits[1].is_empty() {
        let rows: Vec<&String> = ps.iter().flat_map(|p| p["after"].as_object().into_iter().flatten().map(|(k, _)| k)).collect();
        let files: Vec<String> = (0..ps.len()).flat_map(named).collect();
        let what = format!(
            "An extract of run {rid} was stopped between its projects' commits, and both sides were changed outside Solace since: {}. Its rows: {}. Nothing was changed; commands in these projects, revert and cancel refuse until a human decides.",
            files.join(", "),
            rows.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
        );
        if r["extract"]["flagged"] != true {
            let top = scope::top(&stores[0].root);
            tasks::send(&top, json!({"kind":"note","task":tid,"author":"solace","text":what,"flag":"an interrupted extract needs a decision","hold":true}))?;
            let mut flagged = r.clone();
            flagged["extract"]["flagged"] = json!(true);
            put(&flagged)?;
        }
        return Ok(Some(what));
    }
    if source_side {
        // The source or a dependent was edited outside Solace: every part that
        // committed is undone, the extract never happened, and the edits stay.
        let committed: Vec<bool> = stores.iter().zip(&ps).map(|(s, p)| done(s, p, tid, rid)).collect::<Result<_, _>>()?;
        for (((s, p), keep), c) in stores.iter().zip(&ps).zip(&edits).zip(&committed) {
            if *c {
                undo(s, p, tid, rid, keep)?;
            }
        }
        for ((s, p), keep) in stores.iter().zip(&ps).zip(&edits) {
            roll_back(s, p, rid, keep)?;
        }
        return clear(r).map(|_| None);
    }
    finish(&stores, &ps, tid, rid)?;
    end(r).map(|_| None)
}

/// Commits each project's part that didn't, from the record's rows, with its undo rows.
fn finish(stores: &[Store], ps: &[Value], tid: &str, rid: &str) -> Result<(), Failure> {
    for (i, (s, p)) in stores.iter().zip(ps).enumerate() {
        if i == 1 || done(s, p, tid, rid)? {
            continue;
        }
        s.db.execute_batch("BEGIN IMMEDIATE")?;
        let applied = (|| -> Result<(), Failure> {
            let mut states = vec![];
            for (k, v) in p["after"].as_object().into_iter().flatten() {
                match k.strip_prefix("symbol:") {
                    // the source keeps a moved symbol's row, retired
                    Some(id) if v.is_null() => {
                        s.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [id])?;
                    }
                    // files are on disk already
                    _ if k.starts_with("file:") => {}
                    _ => states.push((k.clone(), v.clone())),
                }
            }
            board::restore(s, &states)?;
            board::touch(s, tid, rid, &p["before"], &p["after"])?;
            Ok(())
        })();
        match applied {
            Ok(()) => s.db.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = s.db.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Undoes a project's committed part: its rows as before, and the extract's undo
/// rows gone. gleam.toml is `roll_back`'s (it loses only its lines), and a file
/// in `keep`, edited outside Solace, stays.
fn undo(s: &Store, p: &Value, tid: &str, rid: &str, keep: &[String]) -> Result<(), Failure> {
    let states: Vec<(String, Value)> = p["before"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.as_str() != "file:gleam.toml" && !k.strip_prefix("file:").is_some_and(|f| keep.iter().any(|x| x == f)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let saved: Vec<(PathBuf, Vec<u8>)> = keep.iter().filter_map(|k| fs::read(s.root.join(k)).ok().map(|b| (s.root.join(k), b))).collect();
    s.db.execute_batch("BEGIN IMMEDIATE")?;
    let undone = (|| -> Result<(), Failure> {
        board::restore(s, &states)?;
        for k in p["before"].as_object().into_iter().flatten().map(|(k, _)| k) {
            s.db.execute("DELETE FROM board_touches WHERE task=?1 AND run=?2 AND entity=?3", params![tid, rid, k])?;
        }
        s.db.execute("DELETE FROM board_before WHERE task=?1 AND entity NOT IN (SELECT entity FROM board_touches WHERE task=?1)", params![tid])?;
        Ok(())
    })();
    match undone {
        Ok(()) => s.db.execute_batch("COMMIT")?,
        Err(e) => {
            let _ = s.db.execute_batch("ROLLBACK");
            return Err(e);
        }
    }
    for (file, bytes) in saved {
        fs::write(file, bytes)?;
    }
    Ok(())
}

/// Puts project `p` back as its store has it, the store's own uncommitted rows
/// being gone: the run's file records undone (gleam.toml only loses the lines
/// it gained, unless nothing else changed it), files the command created and
/// nothing changed since removed, and the code files written out again, except
/// those in `keep`, edited outside Solace. A new file the record can't show the
/// command made (it stopped before `complete`) stays, as an outside edit.
fn roll_back(s: &Store, p: &Value, rid: &str, keep: &[String]) -> Result<(), Failure> {
    let mut states = vec![];
    for e in fs::read_dir(s.root.join(".solace/pending")).into_iter().flatten().flatten() {
        let Some(v) = fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        let Some(name) = v["path"].as_str().filter(|_| v["run"] == rid && !is_record(&v)) else { continue };
        if name == "gleam.toml" {
            let file = s.root.join(name);
            let (saved, now) = (text(&v["bytes"]), fs::read_to_string(&file).unwrap_or_default());
            let lost = gained(&saved, &now);
            if lost.iter().fold(saved.clone(), |t, l| add_dependency(&t, l)) == now {
                fs::write(&file, saved)?;
            } else if !lost.is_empty() {
                let left: Vec<&str> = now.lines().filter(|l| !lost.iter().any(|x| x.trim() == l.trim())).collect();
                fs::write(&file, left.join("\n") + "\n")?;
            }
        } else if !keep.iter().any(|k| k == name) {
            states.push((format!("file:{name}"), v["bytes"].clone()));
        }
    }
    lockdown::drop_records(&s.root, rid)?;
    for (name, hash) in hashes(&s.root).as_object().into_iter().flatten() {
        if p["files_before"][name.as_str()].is_null() && p["files_after"][name.as_str()] == *hash && !keep.contains(name) {
            fs::remove_file(s.root.join(name))?;
        }
    }
    let saved: Vec<(PathBuf, Vec<u8>)> = keep.iter().filter_map(|k| fs::read(s.root.join(k)).ok().map(|b| (s.root.join(k), b))).collect();
    board::restore(s, &states)?;
    for (file, bytes) in saved {
        fs::write(file, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gained_lines_skip_headers_and_lines_already_there() {
        let before = "name=\"web\"\n";
        let after = "name=\"web\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n";
        assert_eq!(gained(before, after), ["components = { path = \"../components\" }"]);
        assert!(gained(after, after).is_empty());
    }

    #[test]
    fn merging_lines_keeps_an_outside_edit_and_adds_a_line_once() {
        let dir = tempfile_dir();
        fs::write(dir.join("gleam.toml"), "name=\"web\"\n# edited\n").unwrap();
        let lines = json!(["components = { path = \"../components\" }"]);
        merge_lines(&dir, &lines).unwrap();
        merge_lines(&dir, &lines).unwrap();
        let text = fs::read_to_string(dir.join("gleam.toml")).unwrap();
        assert!(text.contains("# edited"));
        assert_eq!(text.matches("components = ").count(), 1);
    }

    #[test]
    fn a_file_matching_neither_side_is_edited() {
        let dir = tempfile_dir();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.gleam"), "before").unwrap();
        let before = hashes(&dir);
        fs::write(dir.join("src/a.gleam"), "after").unwrap();
        let p = json!({"root": dir, "files_before": before, "files_after": hashes(&dir)});
        assert!(edited(&p).is_empty());
        fs::write(dir.join("src/a.gleam"), "outside").unwrap();
        assert_eq!(edited(&p), ["src/a.gleam"]);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = env::temp_dir().join(format!("solace-ahead-{}", unique_id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
