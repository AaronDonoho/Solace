//! Changes made outside Solace (D-58). Solace remembers what it last wrote to
//! each Gleam file (`.solace/written`) and never writes over a file that
//! differs from that; such a file is read into the store instead, and the
//! change goes to the running task or to Unexpected Changes for review.
//! `.solace/approved` holds each symbol's and header's hash as last approved,
//! so code that came from Solace (a pull, a branch switch) is read in quietly.
use super::*;
use std::collections::BTreeMap;

use serde_json::Value;
use serde_json::json;
use crate::store::projection;
use std::collections::HashMap;
pub const TITLE: &str = "Unexpected Changes";

fn digest(bytes: &[u8]) -> String {
    sha256(bytes)[..32].to_string()
}

fn stat(path: &Path) -> Option<(u64, u128)> {
    let m = fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some((m.len(), t))
}

/// What Solace last wrote to each graph file: `hash size mtime path` lines,
/// `! hash path` for a version it couldn't read in and already reported,
/// `? digest` for the ignore file that last hid the store, and
/// `# old-copies-removed` once the old id-keyed copies are gone.
pub struct Written {
    root: PathBuf,
    files: BTreeMap<String, (String, u64, u128)>,
    bad: BTreeMap<String, String>,
    /// Digest of the ignore files that last hid the store, so the note is said once per change.
    hidden: Option<String>,
    /// False for a store from before D-58 that has no record yet.
    pub known: bool,
    /// True once copies under the old id-keyed layout have been removed.
    old_copies_removed: bool,
    dirty: bool,
}

impl Written {
    pub fn load(root: &Path) -> Written {
        let text = fs::read_to_string(root.join(".solace/written"));
        let (mut files, mut bad, mut hidden) = (BTreeMap::new(), BTreeMap::new(), None);
        let mut old_copies_removed = false;
        for line in text.as_deref().unwrap_or("").lines() {
            if line == OLD_COPIES_REMOVED {
                old_copies_removed = true;
                continue;
            }
            if let Some(h) = line.strip_prefix("? ") {
                hidden = Some(h.to_string());
                continue;
            }
            if let Some((h, path)) = line.strip_prefix("! ").and_then(|l| l.split_once(' ')) {
                bad.insert(path.to_string(), h.to_string());
                continue;
            }
            let mut p = line.splitn(4, ' ');
            if let (Some(h), Some(size), Some(time), Some(path)) = (p.next(), p.next(), p.next(), p.next()) {
                files.insert(path.to_string(), (h.to_string(), size.parse().unwrap_or(0), time.parse().unwrap_or(0)));
            }
        }
        Written { root: root.into(), files, bad, hidden, known: text.is_ok(), old_copies_removed, dirty: false }
    }

    pub fn save(&mut self) -> Result<(), Failure> {
        if self.dirty {
            let mut text: String = self.hidden.iter().map(|h| format!("? {h}\n")).collect();
            if self.old_copies_removed {
                text.push_str(OLD_COPIES_REMOVED);
                text.push('\n');
            }
            text.extend(self.files.iter().map(|(p, (h, size, time))| format!("{h} {size} {time} {p}\n")));
            text.extend(self.bad.iter().map(|(p, h)| format!("! {h} {p}\n")));
            fsx::write_atomic(&self.root.join(".solace/written"), text)?;
            self.dirty = false;
        }
        Ok(())
    }

    /// Notes that `rel` now holds `bytes`, as Solace wrote or read it.
    pub fn record(&mut self, rel: &str, bytes: &[u8]) {
        let (size, time) = stat(&self.root.join(rel)).unwrap_or((0, 0));
        let entry = (digest(bytes), size, time);
        if self.files.get(rel) != Some(&entry) {
            self.files.insert(rel.to_string(), entry);
            self.dirty = true;
        }
        self.dirty |= self.bad.remove(rel).is_some();
    }

    pub fn forget(&mut self, rel: &str) {
        self.dirty |= self.files.remove(rel).is_some();
    }

    /// Whether `rel` still holds what Solace last wrote (or doesn't exist),
    /// so writing over it loses nothing. Without a record, anything goes.
    pub fn unchanged(&self, rel: &str) -> bool {
        let path = self.root.join(rel);
        if !self.known || !path.exists() {
            return true;
        }
        let Some((h, size, time)) = self.files.get(rel) else { return false };
        stat(&path) == Some((*size, *time)) || fs::read(&path).is_ok_and(|b| &digest(&b) == h)
    }

    /// Removes each file Solace wrote that no module in `keep` holds any more,
    /// with the folders that leaves empty. A file edited since stays, for
    /// `absorb` to read in and report; files Solace never wrote are not its own,
    /// and a file an interrupted extract created is its rollback's (D-135).
    /// `stops` holds, for module files an undo is deleting, the folder recorded
    /// as there before the task (D-136): removing empty folders stops there.
    pub fn prune(&mut self, keep: &HashSet<String>, stops: &HashMap<String, String>) -> Result<(), Failure> {
        if !self.known {
            return Ok(());
        }
        let extract = crate::extract::ahead::created(&self.root);
        let gone: Vec<String> = self
            .files
            .keys()
            .filter(|rel| !keep.contains(*rel) && !self.bad.contains_key(*rel) && !extract.contains(*rel))
            .cloned()
            .collect();
        for rel in gone {
            if !self.unchanged(&rel) {
                continue;
            }
            let path = self.root.join(&rel);
            if path.exists() {
                fs::remove_file(&path)?;
            }
            drop_build_output(&self.root, &rel)?;
            // Up to, not including, `src/` or `test/`, or the folder an undo
            // recorded as there before the task (D-136).
            let top = rel.split('/').next().unwrap_or_default().to_string();
            let stop = stops.get(&rel).cloned().filter(|k| k.len() > top.len()).unwrap_or(top);
            remove_empty_dirs(&self.root.join(stop), path.parent());
            self.forget(&rel);
        }
        Ok(())
    }
}

/// Removes `dir` and each parent it leaves empty, up to, not including, `top`.
pub fn remove_empty_dirs(top: &Path, mut dir: Option<&Path>) {
    while let Some(d) = dir.filter(|d| d.starts_with(top) && *d != top) {
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Removes what Gleam compiled from module file `rel`, which `gleam build`
/// leaves behind once the source is gone: `ebin/` and `_gleam_artefacts/`
/// files named `a@b.*`, and JavaScript's `a/b.mjs` and `a/b.d.mts`.
fn drop_build_output(root: &Path, rel: &str) -> Result<(), Failure> {
    let module = rel.split_once('/').map_or(rel, |p| p.1).trim_end_matches(".gleam");
    let erl = format!("{}.", module.replace('/', "@"));
    let pkg = workspace::name(root);
    if pkg.is_empty() {
        return Ok(());
    }
    for profile in ["dev", "prod", "lsp"] {
        for target in ["erlang", "javascript"] {
            let base = root.join("build").join(profile).join(target).join(&pkg);
            for sub in ["ebin", "_gleam_artefacts"] {
                for e in fs::read_dir(base.join(sub)).into_iter().flatten().flatten() {
                    if e.file_name().to_string_lossy().starts_with(&erl) && e.path().is_file() {
                        fs::remove_file(e.path())?;
                    }
                }
            }
            for ext in ["mjs", "d.mts"] {
                let p = base.join(format!("{module}.{ext}"));
                if p.is_file() {
                    fs::remove_file(p)?;
                }
            }
        }
    }
    Ok(())
}

/// A module's file path relative to the root, as `src/app.gleam`.
pub fn file_of(dir: &str, module: &str) -> String {
    format!("{dir}/{module}.gleam")
}

/// Every file the store holds: a Gleam project's `src/` and `test/` files
/// except a probe left behind, or a Rust project's module files (D-158). A
/// project of plain files holds no symbols (D-60).
fn graph_files(s: &Store) -> Result<Vec<String>, Failure> {
    if let Some(lang) = language::detect(&s.root) {
        let mut out = lang.source_files(&s.root)?;
        out.retain(|rel| *rel != file_of("test", crate::cli::PROBE));
        return Ok(out);
    }
    if !language::is_rust(&s.root) {
        return Ok(vec![]);
    }
    let mut stmt = s.db.prepare("SELECT file FROM modules WHERE file IS NOT NULL ORDER BY file")?;
    let files: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
    Ok(files.into_iter().filter(|rel| s.root.join(rel).is_file()).collect())
}

// --- approved hashes ------------------------------------------------------------

fn approved(root: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(root.join(".solace/approved")).unwrap_or_default();
    // A line git left half-merged simply doesn't match, so its symbol counts as unexpected.
    text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once(' ')).map(|(h, k)| (k.to_string(), h.to_string())).collect()
}

fn save_approved(root: &Path, map: &BTreeMap<String, String>) -> Result<(), Failure> {
    let mut text = String::from("# Solace: the code as last approved (D-58). Commit this file; Solace keeps it current.\n");
    for (k, h) in map {
        text += &format!("{h} {k}\n");
    }
    fsx::write_atomic(&root.join(".solace/approved"), text)?;
    share_store(root)
}

fn header_key(path: &str) -> String {
    format!("{path} (imports)")
}

/// The approved key and hash of a snapshot entry (`symbol:…`, `module:…`).
fn approval(key: &str, row: &Value) -> Option<(String, String)> {
    if key.starts_with("symbol:") {
        Some((format!("{}.{}", row[1].as_str()?, row[2].as_str()?), digest(row[4].as_str()?.as_bytes())))
    } else if key.starts_with("module:") {
        Some((header_key(row[0].as_str()?), digest(row[2].as_str()?.as_bytes())))
    } else {
        None
    }
}

/// The whole store as approved: at `create`, and for a store that has no list yet.
pub fn approve_all(s: &Store) -> Result<(), Failure> {
    // A commit copies `store.db` alone: checkpoint the WAL into it first (D-179).
    checkpoint(s)?;
    let snap = board::snapshot(s)?;
    let map = snap.as_object().unwrap().iter().filter_map(|(k, v)| approval(k, v)).collect();
    save_approved(&s.root, &map)
}

/// A completed task's changes become approved: each entity's name before the
/// task leaves the list, and its current state joins it. The caller flushes the
/// store after its last write (D-179).
// takes each entity as it is now, even if a later open task also changed it.
pub fn approve(s: &Store, tid: &str) -> Result<(), Failure> {
    let mut st = s.db.prepare("SELECT entity,data FROM board_before WHERE task=?")?;
    let before: Vec<(String, Value)> = st
        .query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|r| r.map(|(k, v)| (k, serde_json::from_str(&v).unwrap_or(Value::Null))))
        .collect::<Result<_, _>>()?;
    if before.is_empty() {
        return Ok(());
    }
    let now = board::snapshot(s)?;
    let mut map = approved(&s.root);
    for (key, old) in &before {
        if let Some((k, _)) = approval(key, old) {
            map.remove(&k);
        }
        if let Some((k, h)) = approval(key, &now[key]) {
            map.insert(k, h);
        }
    }
    save_approved(&s.root, &map)
}

// --- reading changed files in -----------------------------------------------------

/// Reads one changed file into the store, or says why it can't.
fn read_in(s: &Store, rel: &str, text: Option<&str>) -> Result<Result<(), String>, Failure> {
    // Reading a changed Rust file back into the store is the Rust store
    // task's; until then the change is reported, not lost (D-158).
    if language::is_rust(&s.root) {
        return Ok(Err(format!("{rel}: changes to Rust files outside Solace aren't read in yet")));
    }
    let (dir, file) = rel.split_once('/').unwrap();
    let module = file.strip_suffix(".gleam").unwrap();
    if !s.lang().valid_module_path(module) {
        return Ok(Err(format!("{rel}: `{module}` isn't a Gleam module name")));
    }
    let existing: Vec<(String, String, String, i64, String)> =
        s.db.prepare("SELECT id, name, text, ord, kind FROM symbols WHERE module = ?1 AND live = 1")?
            .query_map([module], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
    let Some(text) = text else {
        for (id, ..) in &existing {
            s.retire(id, "outside Solace")?;
        }
        s.db.execute("DELETE FROM modules WHERE path = ?1", [module])?;
        return Ok(Ok(()));
    };
    let parsed = s.lang().split(text);
    if let Some(twice) = language::repeated(&parsed.items).first() {
        return Ok(Err(format!(
            "{} defines `{}` twice; Solace left it as it is and reads it again \
             once fixed",
            rel, twice
        )));
    }
    // ON CONFLICT keeps a Rust module's `file` (D-158).
    s.db.execute(
        "INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3) ON CONFLICT(path) \
         DO UPDATE SET dir=excluded.dir, header=excluded.header",
        params![module, dir, parsed.header],
    )?;
    for (ord, item) in parsed.items.iter().enumerate() {
        match existing.iter().find(|e| e.1 == item.name) {
            Some((id, _, old, old_ord, kind)) => {
                if *old != item.text || *old_ord != ord as i64 || kind != item.kind.as_str() {
                    s.db.execute(
                        "UPDATE symbols SET text = ?2, revision = ?3, ord = ?4, kind = ?5 WHERE id = ?1",
                        params![id, item.text, revision(&item.text), ord as i64, item.kind.as_str()],
                    )?;
                }
                if *old != item.text {
                    s.history(id, "edit", "outside Solace")?;
                }
            }
            None => {
                let id = insert_symbol(&s.db, module, item, Some(ord as i64))?;
                s.history(&id, "add", "outside Solace")?;
            }
        }
    }
    for (id, name, ..) in &existing {
        if !parsed.items.iter().any(|i| &i.name == name) {
            s.retire(id, "outside Solace")?;
        }
    }
    Ok(Ok(()))
}

pub enum Mode {
    /// At the start of a command, or the board's poll: changes go to the
    /// running task or to Unexpected Changes.
    Notice,
    /// Right after a command that ran a process (`format`, `run`): the
    /// caller records the changes as that command's.
    Command,
}

/// Reads in every graph file changed outside Solace. Returns a note for the
/// command's reply, or "" when nothing changed. The caller holds the lock.
pub fn absorb(s: &Store, mode: Mode) -> Result<String, Failure> {
    share_store(&s.root)?;
    let mut w = Written::load(&s.root);
    let hidden = hidden_note(&mut w, &s.root);
    if !w.known {
        // A store from before D-58: what it holds is what Solace last wrote.
        let ix = s.index()?;
        for m in &ix.modules {
            let (content, _) = projection(&ix, m);
            w.files.insert(m.rel(), (digest(content.as_bytes()), 0, 0));
        }
        w.known = true;
        w.dirty = true;
    }
    if !s.root.join(".solace/approved").exists() {
        approve_all(s)?;
    }
    let disk = graph_files(s)?;
    if !w.old_copies_removed {
        remove_old_copies(&s.root, &disk)?;
        w.old_copies_removed = true;
        w.dirty = true;
    }
    let mut changed: Vec<(String, Option<Vec<u8>>)> = vec![];
    for rel in &disk {
        let path = s.root.join(rel);
        let now = stat(&path);
        match w.files.get(rel) {
            Some((_, size, time)) if now == Some((*size, *time)) => continue,
            _ => {}
        }
        let bytes = fs::read(&path)?;
        if w.files.get(rel).is_some_and(|(h, ..)| *h == digest(&bytes)) {
            w.record(rel, &bytes);
        } else if w.bad.get(rel) != Some(&digest(&bytes)) {
            changed.push((rel.clone(), Some(bytes)));
        }
    }
    for rel in w.files.keys() {
        if !disk.contains(rel) && !w.bad.contains_key(rel) {
            changed.push((rel.clone(), None));
        }
    }
    if changed.is_empty() {
        w.save()?;
        return Ok(hidden);
    }
    let before = board::snapshot(s)?;
    let mut unreadable = vec![];
    for (rel, bytes) in &changed {
        let text = match bytes.as_deref().map(std::str::from_utf8) {
            Some(Err(_)) => {
                unreadable.push(format!("{rel} isn't UTF-8 text; Solace left it as it is"));
                w.bad.insert(rel.clone(), digest(bytes.as_deref().unwrap()));
                w.dirty = true;
                continue;
            }
            Some(Ok(t)) => Some(t),
            None => None,
        };
        match read_in(s, rel, text)? {
            Ok(()) => match bytes {
                Some(b) => w.record(rel, b),
                None => w.forget(rel),
            },
            Err(why) => {
                unreadable.push(why);
                w.bad.insert(rel.clone(), digest(bytes.as_deref().unwrap_or_default()));
                w.dirty = true;
            }
        }
    }
    w.save()?;
    let after = board::snapshot(s)?;
    let unreadable_note = unreadable.iter().map(|u| format!("Not read in: {u}.\n")).collect::<String>();
    if matches!(mode, Mode::Command) {
        return Ok(format!("{hidden}{unreadable_note}"));
    }
    // What changed, and of that, what wasn't approved elsewhere.
    // Tasks from other projects whose scope includes this one count too (D-61).
    let flow = board::flow(&s.root)?;
    let top = scope::top(&s.root);
    let here = scope::dir(&top, &s.root);
    let all = scope::tasks_in(&top, &here)?;
    let open: Vec<&(PathBuf, Value)> = all.iter().filter(|(_, t)| board::column(&flow, t)["final"] != true).collect();
    let mut touched: HashSet<String> = HashSet::new();
    for (_, t) in &open {
        let changes = board::changes_of(s, t)?;
        touched.extend(changes.iter().filter(|c| c["project"] == here).filter_map(|c| c["entity"].as_str().map(String::from)));
    }
    let list = approved(&s.root);
    let keys: HashSet<&String> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    let mut unexpected = vec![];
    for key in keys {
        if before[key] == after[key] {
            continue;
        }
        let quiet = !touched.contains(key.as_str())
            && match approval(key, &after[key]) {
                Some((k, h)) => list.get(&k) == Some(&h),
                None => approval(key, &before[key]).is_some_and(|(k, _)| !list.contains_key(&k)),
            };
        if !quiet {
            unexpected.push(key.clone());
        }
    }
    if unexpected.is_empty() && unreadable.is_empty() {
        return Ok(hidden);
    }
    let pick = |v: &Value| json!(unexpected.iter().map(|k| (k.clone(), v[k].clone())).collect::<serde_json::Map<_, _>>());
    let (bf, af) = (pick(&before), pick(&after));
    let names: Vec<String> = {
        let mut n: Vec<String> = unexpected
            .iter()
            .map(|k| match approval(k, if af[k].is_null() { &bf[k] } else { &af[k] }) {
                Some((name, _)) => name,
                None => k.clone(),
            })
            .collect();
        n.sort();
        n
    };
    let running = open.iter().find_map(|(home, t)| {
        let r = t["runs"].as_array().unwrap().iter().find(|r| r["kind"] == "work" && r["state"] == "running")?;
        Some((home.clone(), t["id"].as_str()?.to_string(), r["id"].as_str()?.to_string()))
    });
    // Tasks change through the task server (D-77).
    if let Some((_, tid, rid)) = running {
        if !unexpected.is_empty() {
            board::record(s, &tid, &rid, &bf, &af, None)?;
        }
        let text = format!(
            "{}{unreadable_note}",
            if names.is_empty() {
                String::new()
            } else {
                format!("Changed outside Solace during this run and added to this task: {}.\n", names.join(", "))
            }
        );
        tasks::deliver(&top, json!({"kind":"note","task":tid,"run":rid,"author":"solace","text":text}))?;
        return Ok(format!("{hidden}Note: {text}"));
    }
    let tid =
        match open.iter().map(|(_, t)| t).find(|t| t["unexpected"] == true && board::task(&s.root, t["id"].as_str().unwrap_or("")).is_ok())
        {
            Some(t) => t["id"].as_str().unwrap().to_string(),
            None => {
                let rank = all.iter().filter_map(|(_, t)| t["rank"].as_f64()).fold(0., f64::min) - 1.;
                let mut t = board::new_task(
                    TITLE,
                    concat!(
                        "Code changed outside Solace while no agent was working. Complete this task to keep ",
                        "the changes, or cancel it to restore the code Solace had. No agent starts work until ",
                        "it is completed or canceled."
                    ),
                    &flow["events"]["finished"],
                    rank,
                );
                t["unexpected"] = json!(true);
                t["home"] = json!(here);
                tasks::deliver(&top, json!({"kind":"create","new":t}))?;
                t["id"].as_str().unwrap().to_string()
            }
        };
    if !unexpected.is_empty() {
        board::record(s, &tid, "outside", &bf, &af, None)?;
    }
    let compiled = match s.check() {
        Ok(out) => out,
        Err(Failure(out, _)) => out,
    };
    let compiled: String = compiled.chars().take(4000).collect();
    let changed_note = if names.is_empty() { String::new() } else { format!("Changed outside Solace: {}.\n", names.join(", ")) };
    tasks::deliver(
        &top,
        json!({"kind":"note","task":tid,"author":"solace","text":format!("{changed_note}{unreadable_note}\nCheck: {compiled}"),
            "flag":"code changed outside Solace"}),
    )?;
    Ok(format!(
        "{hidden}Note: code changed outside Solace with no agent working; review it on the {TITLE} task. {changed_note}{unreadable_note}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    const APP: &str = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n";

    /// The `app` module, `APP`, on a board.
    fn app() -> Project {
        let p = project(&[("src/app.gleam", APP)]);
        board::open(&p.root).unwrap();
        p
    }

    fn file(s: &Store) -> String {
        fs::read_to_string(s.root.join("src/app.gleam")).unwrap()
    }

    fn text(s: &Store, name: &str) -> Option<String> {
        s.db.query_row("SELECT text FROM symbols WHERE module = 'app' AND name = ?1 AND live = 1", [name], |r| r.get(0)).optional().unwrap()
    }

    /// D-58.1/4/5: an edit made outside Solace is read in, never written
    /// over, and waits on Unexpected Changes, which blocks work until a
    /// human cancels it, restoring Solace's code.
    #[test]
    fn outside_edits_are_read_in_and_reviewed() {
        let s = app();
        assert!(s.root.join(".solace/approved").exists() && s.root.join(".solace/written").exists());
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let edited = APP.replace("  2", "  22") + "\npub fn three() -> Int {\n  3\n}\n";
        fs::write(s.root.join("src/app.gleam"), &edited).unwrap();
        // A write in the middle of a command leaves the changed file alone.
        s.materialize(&s.index().unwrap()).unwrap();
        assert_eq!(file(&s), edited);
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains(TITLE) && note.contains("app.three, app.two"), "{note}");
        assert_eq!(text(&s, "two").as_deref(), Some("pub fn two() -> Int {\n  22\n}"));
        assert!(text(&s, "three").is_some());
        s.check().unwrap();
        assert_eq!(file(&s), edited);
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap();
        assert_eq!(t["column"], "needs_review");
        assert!(t["notes"][0]["text"].as_str().unwrap().contains("Check: No errors"), "{}", t["notes"]);
        // Nothing new: no second note.
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let other = board::api(&s.root, "POST", "/tasks", json!({"title":"Other"})).unwrap();
        let oid = other["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"plan"})).unwrap();
        let refused = board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"assign","model":"x"})).unwrap_err();
        assert!(refused.0.starts_with("E7012"), "{}", refused.0);
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(file(&s), APP);
        assert!(text(&s, "three").is_none());
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
    }

    /// D-58.6: code whose hashes are on the approved list (a pull of work
    /// done through Solace) is read in without a task; completing a task
    /// puts its changes on the list.
    #[test]
    fn approved_code_is_read_in_quietly() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let pulled = APP.replace("  1", "  11");
        let mut list = approved(&s.root);
        list.insert("app.one".into(), digest(b"pub fn one() -> Int {\n  11\n}"));
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/app.gleam"), &pulled).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  11\n}"));
        assert!(board::tasks(&s.root).unwrap().is_empty());
        // A hand edit isn't approved; completing its task approves it.
        fs::write(s.root.join("src/app.gleam"), pulled.replace("  2", "  5")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("app.two"));
        let t = board::tasks(&s.root).unwrap().remove(0);
        let tid = t["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(approved(&s.root).get("app.two"), Some(&digest(b"pub fn two() -> Int {\n  5\n}")));
    }

    /// D-58.5: a file Solace can't read into symbols is left as it is and
    /// reported once; once fixed it is read in.
    #[test]
    fn unreadable_files_are_left_alone() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("defines `two` twice"));
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        s.materialize(&s.index().unwrap()).unwrap();
        assert_eq!(file(&s), twice);
        fs::write(s.root.join("src/app.gleam"), APP.replace("  2", "  3")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("app.two"));
        assert_eq!(text(&s, "two").as_deref(), Some("pub fn two() -> Int {\n  3\n}"));
    }

    /// D-58.2: during an agent's work run, a change from outside is the run's:
    /// recorded on its task, with a note, and no Unexpected Changes task.
    #[test]
    fn outside_edits_during_a_run_join_its_task() {
        let s = app();
        let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":"Work"})).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
        tasks::put(&s, &t).unwrap();
        fs::write(s.root.join("src/app.gleam"), APP.replace("  2", "  9")).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert_eq!(note, "Note: Changed outside Solace during this run and added to this task: app.two.\n");
        let t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{}", json!(changes));
        assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
    }

    /// D-58.2: what a command's own process changed (`format`, a declared command) is
    /// read in as that command's change, with no note.
    #[test]
    fn a_commands_own_changes_are_read_in_quietly() {
        let s = app();
        fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  4")).unwrap();
        assert_eq!(absorb(&s, Mode::Command).unwrap(), "");
        assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  4\n}"));
        assert!(board::tasks(&s.root).unwrap().is_empty());
    }

    /// A file read in with a bad module name is refused with its reason; one
    /// deleted outside retires its symbols and drops its module.
    #[test]
    fn read_in_refuses_a_bad_name_and_retires_a_deleted_file() {
        let s = app();
        let msg = read_in(&s, "src/Bad-Name.gleam", Some(APP)).unwrap().unwrap_err();
        assert!(msg.contains("isn't a Gleam module name"), "{msg}");
        let modules = |s: &Store| s.db.query_row("SELECT count(*) FROM modules WHERE path = 'app'", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(modules(&s), 1);
        read_in(&s, "src/app.gleam", None).unwrap().unwrap();
        assert!(text(&s, "one").is_none() && text(&s, "two").is_none());
        assert_eq!(modules(&s), 0);
    }

    /// D-158: reading a module in keeps the file a Rust module is written to.
    #[test]
    fn read_in_keeps_a_modules_file() {
        let s = app();
        s.db.execute("UPDATE modules SET file = 'src/app.rs' WHERE path = 'app'", []).unwrap();
        let edited = APP.replace("  1", "  11");
        read_in(&s, "src/app.gleam", Some(&edited)).unwrap().unwrap();
        let file: Option<String> = s.db.query_row("SELECT file FROM modules WHERE path = 'app'", [], |r| r.get(0)).unwrap();
        assert_eq!(file.as_deref(), Some("src/app.rs"));
    }

    /// Moving a symbol without editing it changes its order only; the text stays.
    #[test]
    fn a_reordered_symbol_keeps_its_text_and_takes_its_new_place() {
        let s = app();
        let ord = |name: &str| {
            s.db.query_row("SELECT ord FROM symbols WHERE module = 'app' AND name = ?1 AND live = 1", [name], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        let before = (ord("one"), ord("two"));
        assert!(before.0 < before.1);
        let swapped = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  1\n}\n";
        read_in(&s, "src/app.gleam", Some(swapped)).unwrap().unwrap();
        assert!(ord("two") < ord("one"));
        assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  1\n}"));
    }

    /// `unchanged` is true where nothing is lost by writing: no record, or no file.
    #[test]
    fn unchanged_follows_the_record() {
        let s = app();
        let mut w = Written::load(&s.root);
        assert!(w.known && w.unchanged("src/app.gleam") && w.unchanged("src/absent.gleam"));
        fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  9")).unwrap();
        assert!(!w.unchanged("src/app.gleam"), "edited since Solace wrote it");
        fs::write(s.root.join("src/app.gleam"), APP).unwrap();
        assert!(w.unchanged("src/app.gleam"), "the same bytes again, by hash");
        w.forget("src/app.gleam");
        assert!(!w.unchanged("src/app.gleam"), "a file with no entry is not Solace's");
        let fresh = Written::load(&Scratch::new());
        assert!(!fresh.known && fresh.unchanged("src/app.gleam"));
    }

    /// A project of plain files holds no symbols, and an approval needs the
    /// fields of the entry it describes.
    #[test]
    fn plain_files_have_no_graph_and_odd_entries_no_approval() {
        let plain = Scratch::new().with(&[("notes/readme.txt", "hi")]);
        crate::import(&plain).unwrap();
        assert!(graph_files(&crate::Store::at(&plain).unwrap()).unwrap().is_empty());
        assert_eq!(approval("task:1", &json!(["a"])), None);
        assert_eq!(approval("symbol:1", &json!([0, "app", 5, 0, "text"])), None);
        assert_eq!(approval("module:1", &json!(["app.gleam", 0, null])), None);
        assert!(approval("symbol:1", &json!([0, "app", "one", 0, "text"])).is_some_and(|(k, _)| k == "app.one"));
    }

    /// A store from before D-58 has no record: what it holds counts as
    /// written, so a file edited since is read in rather than overwritten.
    #[test]
    fn old_stores_start_from_what_they_hold() {
        let s = app();
        fs::remove_file(s.root.join(".solace/written")).unwrap();
        fs::remove_file(s.root.join(".solace/approved")).unwrap();
        fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  7")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("app.one"));
        assert_eq!(approved(&s.root).get("app.one"), Some(&digest(b"pub fn one() -> Int {\n  1\n}")));
        assert_eq!(file(&s), APP.replace("  1", "  7"));
    }

    /// A repair copy is the only one for its path, git-ignored, and never scanned.
    #[test]
    fn a_kept_copy_replaces_the_last_one_and_is_never_project_source() {
        let p = project(&[("src/app.gleam", APP)]);
        let lost = "pub fn lost() -> Int {\n  9\n}\n";
        let kept = keep(&p.root, "src/app.gleam", lost.as_bytes()).unwrap();
        assert_eq!(kept, ".solace/outside/src/app.gleam");
        let mut ignores = crate::lockdown::Ignores::new(&p.root);
        assert!(ignores.ignored(&kept));
        let v = crate::lockdown::settings(&p.root).unwrap();
        let files = crate::lockdown::dispatch(&p, &v, &["files"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap();
        assert!(files.contains(".solace/ [internal]") && !files.contains("outside/src/app.gleam"), "{files}");
        let grep = crate::lockdown::dispatch(&p, &v, &["grep", "lost"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap();
        assert!(grep.starts_with("No matches for \"lost\""), "{grep}");
        keep(&p.root, "src/app.gleam", b"newer").unwrap();
        assert_eq!(fs::read_to_string(p.root.join(&kept)).unwrap(), "newer");
        assert_eq!(fs::read_dir(p.root.join(".solace/outside")).unwrap().count(), 1);
    }

    /// Copies from the old id-keyed layout go; path-keyed copies stay.
    #[test]
    fn old_id_copies_are_removed() {
        let s = app();
        let old = s.root.join(".solace/outside").join(crate::unique_id());
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("app.gleam"), "lost").unwrap();
        keep(&s.root, "src/app.gleam", b"kept").unwrap();
        absorb(&s, Mode::Command).unwrap();
        assert!(!old.exists());
        assert!(s.root.join(".solace/outside/src/app.gleam").exists());
    }

    /// Cleanup runs once; an old-id folder made afterward is left alone.
    #[test]
    fn old_copies_are_removed_once() {
        let s = app();
        let first = s.root.join(".solace/outside").join(crate::unique_id());
        fs::create_dir_all(&first).unwrap();
        absorb(&s, Mode::Command).unwrap();
        assert!(!first.exists());
        let later = s.root.join(".solace/outside").join(crate::unique_id());
        fs::create_dir_all(&later).unwrap();
        absorb(&s, Mode::Command).unwrap();
        assert!(later.exists());
    }

    /// A 16-hex source folder is a current path key, not an old id.
    #[test]
    fn a_16_hex_source_folder_keeps_its_copy() {
        let s = app();
        let name = "0123456789abcdef";
        let copy = s.root.join(".solace/outside").join(name);
        fs::create_dir_all(&copy).unwrap();
        remove_old_copies(&s.root, &[format!("{name}/app.gleam")]).unwrap();
        assert!(copy.exists());
    }

    #[cfg(test)]
    #[test]
    fn a_root_ignore_that_still_hides_the_store_is_flagged() {
        let s = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
        crate::board::open(&s.root).unwrap();
        let root = s.root.join(".gitignore");
        fs::write(&root, format!("{STORE_IGNORE}*.db\n")).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("still ignores store.db"), "{note}");
        fs::write(&root, format!("{STORE_IGNORE}*.db\n!**/.solace/store.db\n")).unwrap();
        let fixed = absorb(&s, Mode::Notice).unwrap();
        assert!(!fixed.contains("still ignores store.db"), "{fixed}");
    }

    #[cfg(test)]
    #[test]
    fn ignores_store_sees_an_excluded_store_directory() {
        assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "."));
        assert!(ignores_store("**/.solace\n!**/.solace/store.db\n", "project"));
        assert!(ignores_store("project/.solace/\n!project/.solace/store.db\n", "project"));
        assert!(!ignores_store("!.solace/\n!**/.solace/store.db\n", "."));
        assert!(!ignores_store("other/.solace/\n!other/.solace/store.db\n", "project"));
        assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "project"));
        assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "two/nested"));
        assert!(ignores_store("*/.solace/\n", "project"));
        assert!(!ignores_store("*/.solace/\n", "two/nested"));
        assert!(ignores_store("/.solace/\n", "."));
        assert!(!ignores_store("/.solace/\n", "project"));
    }

    #[cfg(test)]
    #[test]
    fn ignores_store_sees_store_file_patterns_at_their_paths() {
        assert!(ignores_store("*\n", "project"));
        assert!(ignores_store("**\n", "project"));
        assert!(ignores_store("**/*\n", "project"));
        assert!(ignores_store("store.db\n", "project"));
        assert!(ignores_store("*.db\n", "project"));
        assert!(ignores_store("store.*\n", "project"));
        assert!(ignores_store("**/.solace/store.db\n", "project"));
        assert!(!ignores_store("/store.db\n", "."));
        assert!(!ignores_store("/store.db\n", "project"));
        assert!(!ignores_store("/*.db\n", "project"));
        assert!(!ignores_store("/store.*\n", "project"));
        assert!(ignores_store("/*\n", "."));
        assert!(ignores_store("/*\n", "project"));
        assert!(ignores_store("/*\n!/.solace/store.db\n", "."));
        assert!(!ignores_store("/*\n!/.solace\n", "."));
        assert!(ignores_store("/project\n", "project"));
        assert!(!ignores_store("/project\n", "one"));
        assert!(ignores_store("project/\n", "project"));
        assert!(ignores_store("/.solace/store.db\n", "."));
        assert!(!ignores_store("/.solace/store.db\n", "project"));
        assert!(ignores_store("/project/.solace/store.db\n", "project"));
    }
}

/// Every project's symbol store is committed with the Solace root, but what is
/// local to each checkout is not (D-179): locks, logs, caches, undo records and
/// board data stay ignored, while each `store.db` and `approved` is shared.
pub fn share_store(project: &Path) -> Result<(), Failure> {
    if !project.join(".solace").is_dir() {
        return Ok(());
    }
    share_root_ignore(&scope::top(project))?;
    remove_generated_store_ignore(project)
}

/// D-179: one root block shares every store; setting up again leaves it alone.
#[cfg(test)]
#[test]
fn the_store_is_committed_with_the_project() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() { 1 }\n")]);
    assert!(!p.root.join(".solace/.gitignore").exists());

    let root = root_with_two_projects();
    share_store(&root.join("one")).unwrap();
    let once = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(once, STORE_IGNORE);
    share_store(&root.join("two/nested")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), STORE_IGNORE);
    assert_eq!(once.matches(BLOCK_START).count(), 1);
    assert!(!root.join("one/.solace/.gitignore").exists());
    assert!(!root.join("two/nested/.solace/.gitignore").exists());
}

/// What the root's block shares from every project (D-179, D-191).
const STORE_IGNORE: &str = concat!(
    "# Solace: shared store files begin; setup replaces the lines below (D-179).\n",
    "**/.solace/*\n",
    "!**/.solace/store.db\n",
    "!**/.solace/approved\n",
    "# Solace: shared store files end.\n"
);

/// The per-project files earlier setup wrote, and migration copies still do.
const OLD_STORE_IGNORES: [&str; 4] = [
    "# Only the approved list is shared (D-58).\n*\n!.gitignore\n!approved\n",
    "# Solace's store is committed with the project.\n",
    "*\n",
    concat!(
        "# Solace's symbol store is committed; local caches, locks, logs and board data are not (D-179).\n",
        "*\n!.gitignore\n!approved\n!store.db\n"
    ),
];

/// Whether the ignore text still hides `store.db` for `here`, the project's
/// folder from the root: an older variant `share_store` doesn't know, or a
/// human's edit. Such files are left alone (D-179), but the caller says so,
/// since a commit would leave the store out.
fn ignores_store(text: &str, here: &str) -> bool {
    let here = here.replace('\\', "/");
    let store = if here == "." { ".solace".to_string() } else { format!("{here}/.solace") };
    let file = format!("{store}/store.db");
    let mut dirs: Vec<(String, bool)> = store_dirs(&store).into_iter().map(|dir| (dir, false)).collect();
    let mut ignored = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, pattern) = match line.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, line),
        };
        let anchored = pattern.starts_with('/');
        let p = pattern.trim_start_matches('/');
        let dir_pattern = p.trim_end_matches('/');
        for (dir, hidden) in &mut dirs {
            if hides_path(dir_pattern, anchored, dir) {
                *hidden = !negated;
            }
        }
        if !p.ends_with('/') && hides_path(p, anchored, &file) {
            ignored = !negated;
        }
    }
    ignored || dirs.into_iter().any(|(_, hidden)| hidden)
}

/// Approving marks the changes; checkpointing puts them in the file a commit copies.
#[cfg(test)]
#[test]
fn approving_puts_the_whole_store_in_store_db() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    let s = board::open(&p.root).unwrap();
    s.db.query_row("PRAGMA wal_autocheckpoint=0", [], |_| Ok(())).unwrap();
    s.db.execute("INSERT INTO board_before VALUES ('t','symbol:x','null')", []).unwrap();
    let wal = s.root.join(".solace/store.db-wal");
    assert!(fs::metadata(&wal).unwrap().len() > 0, "the change is not in the WAL");
    approve(&s, "t").unwrap();
    checkpoint(&s).unwrap();
    // A commit copies `store.db` alone, so the change must be in that file.
    let copy = s.root.join(".solace/copied.db");
    fs::copy(s.root.join(".solace/store.db"), &copy).unwrap();
    let c = rusqlite::Connection::open(&copy).unwrap();
    let rows: i64 = c.query_row("SELECT COUNT(*) FROM board_before", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[cfg(test)]
#[test]
fn an_ignore_file_that_still_hides_the_store_is_flagged() {
    let s = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    crate::board::open(&s.root).unwrap();
    fs::write(s.root.join(".solace/.gitignore"), "# mine\n*\n!/custom\n").unwrap();
    let note = absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("still ignores store.db"), "{note}");
    let again = absorb(&s, Mode::Notice).unwrap();
    assert!(!again.contains("still ignores store.db"), "{again}");
    fs::write(s.root.join(".solace/.gitignore"), "# mine again\n*\n!/custom\n").unwrap();
    let changed = absorb(&s, Mode::Notice).unwrap();
    assert!(changed.contains("still ignores store.db"), "{changed}");
    fs::write(s.root.join(".solace/.gitignore"), "# mine\n!store.db\n").unwrap();
    let fixed = absorb(&s, Mode::Notice).unwrap();
    assert!(!fixed.contains("still ignores store.db"), "{fixed}");
    fs::write(s.root.join(".solace/.gitignore"), "# mine\n*\n!/custom\n").unwrap();
    let regressed = absorb(&s, Mode::Notice).unwrap();
    assert!(regressed.contains("still ignores store.db"), "{regressed}");
}

/// The note that the ignore files still hide the store, said once for each
/// version of them. Clearing the state when it stops hiding means a later
/// regression is reported again.
fn hidden_note(w: &mut Written, root: &Path) -> String {
    let text = ignore_text(root);
    let hiding = ignores_store(&text, &scope::dir(&scope::top(root), root));
    let stamp = (!text.is_empty()).then(|| digest(text.as_bytes()));
    if !hiding {
        if w.hidden.take().is_some() {
            w.dirty = true;
        }
        return String::new();
    }
    if w.hidden == stamp {
        return String::new();
    }
    w.hidden = stamp;
    w.dirty = true;
    "Note: an ignore file still ignores store.db; remove that pattern so the symbol store is committed.\n".to_string()
}

/// Flushes the write-ahead log into `store.db`, the file a commit copies (D-179).
pub(crate) fn checkpoint(s: &Store) -> Result<(), Failure> {
    checkpointed(if checkpoint_pending(s)? { 1 } else { 0 })
}

/// Whether a checkpoint finished, or stopped partway because the database was busy.
fn checkpointed(busy: i64) -> Result<(), Failure> {
    if busy == 0 {
        return Ok(());
    }
    crate::lockdown::error("E7028", "store", "the write-ahead log is busy; store.db does not hold every change")
}

#[cfg(test)]
#[test]
fn a_busy_checkpoint_is_reported() {
    assert!(checkpointed(0).is_ok());
    let e = checkpointed(1).unwrap_err();
    assert!(e.0.starts_with("E7028"), "{}", e.0);
}

/// Whether a full checkpoint left frames unflushed because the log was busy.
pub(crate) fn checkpoint_pending(s: &Store) -> Result<bool, Failure> {
    let (busy, _, _): (i64, i64, i64) = s.db.query_row("PRAGMA wal_checkpoint(FULL)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(busy != 0)
}

/// Keeps a changed file's bytes under `.solace/outside/<path>`, replacing any
/// earlier copy, so a repair that writes over it loses nothing.
pub fn keep(root: &Path, rel: &str, bytes: &[u8]) -> Result<String, Failure> {
    let kept = format!(".solace/outside/{rel}");
    let path = root.join(&kept);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, bytes)?;
    Ok(kept)
}

/// Removes copies kept under the old `.solace/outside/<id>/` layout; a folder
/// that starts a stored source path is a current copy and stays.
fn remove_old_copies(root: &Path, files: &[String]) -> Result<(), Failure> {
    let Ok(entries) = fs::read_dir(root.join(".solace/outside")) else { return Ok(()) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let old_id = name.len() == 16 && name.bytes().all(|b| b.is_ascii_hexdigit());
        if !old_id || !entry.path().is_dir() {
            continue;
        }
        let source_folder = files.iter().any(|f| f.split('/').next() == Some(name.as_str()));
        if !source_folder {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

const OLD_COPIES_REMOVED: &str = "# old-copies-removed";

/// The markers around the block Solace keeps current in the root's `.gitignore`.
const BLOCK_START: &str = "# Solace: shared store files begin; setup replaces the lines below (D-179).\n";

const BLOCK_END: &str = "# Solace: shared store files end.\n";

/// Keeps one marked block in the root's `.gitignore`, leaving the rest alone.
fn share_root_ignore(root: &Path) -> Result<(), Failure> {
    let path = root.join(".gitignore");
    let current = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let wanted = current.as_deref().map_or_else(|| STORE_IGNORE.to_string(), with_store_block);
    if current.as_deref() != Some(wanted.as_str()) {
        fsx::write_atomic(&path, wanted)?;
    }
    Ok(())
}

fn with_store_block(text: &str) -> String {
    let Some(start) = text.rfind(BLOCK_START) else {
        return append_store_block(text);
    };
    let Some(end) = text[start..].find(BLOCK_END).map(|i| start + i + BLOCK_END.len()) else {
        return append_store_block(text);
    };
    format!("{}{STORE_IGNORE}{}", &text[..start], &text[end..])
}

fn append_store_block(text: &str) -> String {
    let newline = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    format!("{text}{newline}{STORE_IGNORE}")
}

/// Removes only a per-project file some Solace version generated; a human's
/// file stays where it is and still applies below the root's block.
fn remove_generated_store_ignore(project: &Path) -> Result<(), Failure> {
    let path = project.join(".solace/.gitignore");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(());
    };
    if OLD_STORE_IGNORES.contains(&text) {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
fn root_with_two_projects() -> crate::testing::Scratch {
    crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"one\", \"two/nested\"]\n"),
        (".solace/store.db", ""),
        ("one/.solace/store.db", ""),
        ("two/nested/.solace/store.db", ""),
    ])
}

#[cfg(test)]
#[test]
fn only_the_store_block_is_changed() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), format!("# keep before\n{BLOCK_START}# old contents\n{BLOCK_END}# keep after\n")).unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), format!("# keep before\n{STORE_IGNORE}# keep after\n"));

    fs::write(root.join(".gitignore"), "# no final newline").unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), format!("# no final newline\n{STORE_IGNORE}"));

    let partial = format!("# keep\n{BLOCK_START}# no end marker\n");
    fs::write(root.join(".gitignore"), &partial).unwrap();
    share_store(&root.join("one")).unwrap();
    let appended = format!("{partial}{STORE_IGNORE}");
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), appended);
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), appended);
}

#[cfg(test)]
#[test]
fn generated_store_ignores_are_removed_and_an_edited_one_stays() {
    let root = root_with_two_projects();
    let path = root.join("one/.solace/.gitignore");
    for old in OLD_STORE_IGNORES {
        fs::write(&path, old).unwrap();
        share_store(&root.join("one")).unwrap();
        assert!(!path.exists(), "{old:?}");
    }
    fs::write(&path, "# mine\n*\n!/custom\n").unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# mine\n*\n!/custom\n");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe]);
}

#[cfg(test)]
#[test]
fn a_folder_without_a_store_gets_no_ignore_file() {
    let bare = crate::testing::Scratch::new();
    share_store(&bare).unwrap();
    assert!(!bare.join(".solace").exists());
    assert!(!bare.join(".gitignore").exists());
}

/// D-191: the generated block keeps the store committed, and its markers match.
#[cfg(test)]
#[test]
fn the_generated_store_ignore_is_pinned() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() { 1 }\n")]);
    assert!(STORE_IGNORE.starts_with(BLOCK_START));
    assert!(STORE_IGNORE.ends_with(BLOCK_END));
    assert_eq!(
        fs::read_to_string(p.root.join(".gitignore")).unwrap(),
        concat!(
            "# Solace: shared store files begin; setup replaces the lines below (D-179).\n",
            "**/.solace/*\n",
            "!**/.solace/store.db\n",
            "!**/.solace/approved\n",
            "# Solace: shared store files end.\n",
        )
    );
}

/// The root's ignore lines, then the store's own, as one heuristic list. They
/// are relative to different folders, so a leading `/` is only approximate.
fn ignore_text(project: &Path) -> String {
    let mut text = fs::read_to_string(scope::top(project).join(".gitignore")).unwrap_or_default();
    let store = fs::read_to_string(project.join(".solace/.gitignore")).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&store);
    text
}

#[cfg(test)]
#[test]
fn another_projects_store_directory_is_not_this_projects_warning() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), "two/nested/.solace/\n").unwrap();
    let one = root.join("one");
    let mut w = Written::load(&one);
    assert_eq!(hidden_note(&mut w, &one), "");
    let nested = root.join("two/nested");
    let mut w = Written::load(&nested);
    assert!(hidden_note(&mut w, &nested).contains("still ignores store.db"));
}

#[cfg(test)]
#[test]
fn a_bare_store_directory_ignore_still_warns_for_a_nested_project() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), ".solace/\n").unwrap();
    let nested = root.join("two/nested");
    let mut w = Written::load(&nested);
    assert!(hidden_note(&mut w, &nested).contains("still ignores store.db"));
}

/// Whether a `.gitignore` pattern covers `path`. A pattern with no slash
/// matches at any depth, as in gitignore, unless it is anchored to the root.
fn hides_path(pattern: &str, anchored: bool, path: &str) -> bool {
    let pattern = if anchored || pattern.contains('/') { pattern.to_string() } else { format!("**/{pattern}") };
    globset::GlobBuilder::new(&pattern).literal_separator(true).build().is_ok_and(|g| g.compile_matcher().is_match(path))
}

/// `store` and its ancestor folders, as paths from the root.
fn store_dirs(store: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut dir = String::new();
    for part in store.split('/') {
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(part);
        dirs.push(dir.clone());
    }
    dirs
}
