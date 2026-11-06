//! Changes made outside Solace (D-58). Solace remembers what it last wrote to
//! each source file (`.solace/written`) and never writes over a file that
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
#[cfg(test)] use crate::lockdown::settings::settings;
#[cfg(test)] use crate::board::task_model::open;
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
/// `@ hash path` for the store's projection when it was marked, `% path why`
/// for the reason, `? digest` for the ignore file that last hid the store, and
/// `# old-copies-removed` once the old id-keyed copies are gone.
/// `# rust-bad-marks-cleared` once an older store's Rust refusals were retried.
pub struct Written {
    root: PathBuf,
    files: BTreeMap<String, (String, u64, u128)>,
    bad: BTreeMap<String, String>,
    /// When each unreadable file was marked: the digest of the store's
    /// projected text then, so only a later store change replaces it.
    bad_at: BTreeMap<String, String>,
    /// Why each unreadable file could not be read in, for the task's note.
    bad_why: BTreeMap<String, String>,
    /// Files a change at the store has replaced since an unreadable edit, so
    /// their note is refreshed once the store's text is on disk.
    redo: std::collections::BTreeSet<String>,
    /// Digest of the ignore files that last hid the store, so the note is said once per change.
    hidden: Option<String>,
    /// False for a store from before D-58 that has no record yet.
    pub known: bool,
    /// True once copies under the old id-keyed layout have been removed.
    old_copies_removed: bool,
    /// True once the Rust files an older store refused were retried.
    rust_bad_cleared: bool,
    dirty: bool,
}

impl Written {
    pub fn load(root: &Path) -> Written {
        let text = fs::read_to_string(root.join(".solace/written"));
        let (mut files, mut bad, mut bad_at, mut bad_why, mut hidden) =
            (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), None);
        let mut redo = std::collections::BTreeSet::new();
        let mut old_copies_removed = false;
        let mut rust_bad_cleared = false;
        for line in text.as_deref().unwrap_or("").lines() {
            if line == OLD_COPIES_REMOVED {
                old_copies_removed = true;
                continue;
            }
            if line == RUST_BAD_CLEARED {
                rust_bad_cleared = true;
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
            if let Some((h, path)) = line.strip_prefix("@ ").and_then(|l| l.split_once(' ')) {
                bad_at.insert(path.to_string(), h.to_string());
                continue;
            }
            if let Some((path, why)) = line.strip_prefix("% ").and_then(|l| l.split_once(' ')) {
                bad_why.insert(path.to_string(), why.to_string());
                continue;
            }
            if let Some(path) = line.strip_prefix("~ ") {
                redo.insert(path.to_string());
                continue;
            }
            let mut p = line.splitn(4, ' ');
            if let (Some(h), Some(size), Some(time), Some(path)) = (p.next(), p.next(), p.next(), p.next()) {
                files.insert(path.to_string(), (h.to_string(), size.parse().unwrap_or(0), time.parse().unwrap_or(0)));
            }
        }
        Written {
            root: root.into(),
            files,
            bad,
            bad_at,
            bad_why,
            redo,
            hidden,
            known: text.is_ok(),
            old_copies_removed,
            rust_bad_cleared,
            dirty: false,
        }
    }

    pub fn save(&mut self) -> Result<(), Failure> {
        if self.dirty {
            let mut text: String = self.hidden.iter().map(|h| format!("? {h}\n")).collect();
            if self.old_copies_removed {
                text.push_str(OLD_COPIES_REMOVED);
                text.push('\n');
            }
            if self.rust_bad_cleared {
                text.push_str(RUST_BAD_CLEARED);
                text.push('\n');
            }
            text.extend(self.files.iter().map(|(p, (h, size, time))| format!("{h} {size} {time} {p}\n")));
            text.extend(self.bad.iter().map(|(p, h)| format!("! {h} {p}\n")));
            text.extend(self.bad_at.iter().map(|(p, h)| format!("@ {h} {p}\n")));
            text.extend(self.bad_why.iter().map(|(p, w)| format!("% {p} {w}\n")));
            text.extend(self.redo.iter().map(|p| format!("~ {p}\n")));
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
        self.dirty |= self.bad_at.remove(rel).is_some();
        self.dirty |= self.bad_why.remove(rel).is_some();
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

    /// Clears the mark that `rel` could not be read in.
    pub fn clear_bad(&mut self, rel: &str) {
        self.dirty |= self.bad.remove(rel).is_some();
        self.dirty |= self.bad_at.remove(rel).is_some();
        self.dirty |= self.bad_why.remove(rel).is_some();
    }

    /// Whether writing `content` over `rel` may replace an unreadable outside
    /// edit: the store's text for that file has moved since it was marked. A
    /// call site also requires that a task holds the file change.
    pub fn may_replace(&self, rel: &str, content: &[u8]) -> bool {
        if self.unchanged(rel) {
            return true;
        }
        self.bad.contains_key(rel) && self.bad_at.get(rel).is_some_and(|h| *h != digest(content))
    }

    /// Notes that the store replaced an unreadable file, so its file change is
    /// dropped once the replacement reaches the disk.
    pub fn note_replacement(&mut self, rel: &str) {
        self.dirty |= self.redo.insert(rel.to_string());
    }

    /// The files the store replaced, cleared from the record.
    pub fn take_replaced(&mut self) -> Vec<String> {
        let out: Vec<String> = self.redo.iter().cloned().collect();
        if !out.is_empty() {
            self.redo.clear();
            self.dirty = true;
        }
        out
    }

    /// Whether `rel` has an unreadable outside edit that was reported.
    pub fn marked_bad(&self, rel: &str) -> bool {
        self.bad.contains_key(rel)
    }

    /// Records that `rel` could not be read in as `bytes`: its reason, and the
    /// store's projected text then. The first mark keeps its text, so later
    /// saves keep the task's first before state.
    pub fn mark_bad(&mut self, rel: &str, bytes: &[u8], projected: &[u8], why: &str) {
        let h = digest(bytes);
        if self.bad.get(rel) != Some(&h) {
            self.bad.insert(rel.to_string(), h);
            self.dirty = true;
        }
        if !self.bad_at.contains_key(rel) {
            self.bad_at.insert(rel.to_string(), digest(projected));
            self.dirty = true;
        }
        if self.bad_why.get(rel).map(String::as_str) != Some(why) {
            self.bad_why.insert(rel.to_string(), why.to_string());
            self.dirty = true;
        }
    }

    /// Why `rel` could not be read in when it was marked.
    pub fn why_bad(&self, rel: &str) -> Option<&str> {
        self.bad_why.get(rel).map(String::as_str)
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
    let before = crate::board::reviews::before_of(s, tid)?;
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
    if language::is_rust(&s.root) {
        return crate::rust_submit::read_outside(s, rel, text);
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
    if language::is_rust(&s.root) && !w.rust_bad_cleared {
        // An older store refused every Rust file; retry each once before trusting the mark.
        w.bad.clear();
        w.rust_bad_cleared = true;
        w.dirty = true;
    }
    if !s.root.join(".solace/approved").exists() {
        approve_all(s)?;
    }
    let disk = graph_files(s)?;
    let ix = s.index()?;
    let replaced = w.take_replaced();
    if !replaced.is_empty() {
        // The outside edit stays visible until the task finishes, but its mark is gone.
        for tid in crate::board::reviews::file_change_tasks(s, &replaced)? {
            refresh_note(s, &tid, Ending::Integrated)?;
        }
    }
    if !w.old_copies_removed {
        remove_old_copies(&s.root, &disk)?;
        w.old_copies_removed = true;
        w.dirty = true;
    }
    let mut changed: Vec<(String, Option<Vec<u8>>)> = vec![];
    let mut parked: Vec<(String, Vec<u8>)> = vec![];
    let mut fixed: Vec<String> = vec![];
    for rel in &disk {
        let path = s.root.join(rel);
        let now = stat(&path);
        match w.files.get(rel) {
            Some((_, size, time)) if now == Some((*size, *time)) => continue,
            _ => {}
        }
        let bytes = fs::read(&path)?;
        if w.files.get(rel).is_some_and(|(h, ..)| *h == digest(&bytes)) {
            if w.bad.contains_key(rel) {
                fixed.push(rel.clone());
            }
            w.record(rel, &bytes);
        } else if w.bad.get(rel) != Some(&digest(&bytes)) {
            changed.push((rel.clone(), Some(bytes)));
        } else if matches!(mode, Mode::Notice) && !crate::board::reviews::holds_file_change(s, rel) {
            parked.push((rel.clone(), bytes));
        }
    }
    for rel in w.files.keys() {
        if !disk.contains(rel) && !w.bad.contains_key(rel) {
            changed.push((rel.clone(), None));
        }
    }
    let (gone_bad, removed) = dropped_gone_bad(&mut w, &disk, &changed, &ix);
    changed.extend(removed);
    if changed.is_empty() && parked.is_empty() {
        return finish_no_changes(s, &mut w, &hidden, &mut fixed, &gone_bad);
    }
    if matches!(mode, Mode::Command) {
        let outcome = read_changed(s, &ix, &mut w, &changed)?;
        fixed.extend(outcome.fixed);
        w.save()?;
        let dropped = refresh_fixed(s, &mut fixed, &gone_bad)?;
        retire_empty_unexpected(s, &dropped)?;
        return Ok(format!("{hidden}{}", unreadable_note(&outcome.unreadable)));
    }
    // What changed, and of that, what wasn't approved elsewhere.
    // Tasks from other projects whose scope includes this one count too (D-61).
    let flow = board::flow(&s.root)?;
    let top = scope::top(&s.root);
    let here = scope::dir(&top, &s.root);
    let all = scope::tasks_in(&top, &here)?;
    let open: Vec<&(PathBuf, Value)> = all.iter().filter(|(_, t)| board::column(&flow, t)["final"] != true).collect();
    let running = open.iter().find_map(|(home, t)| {
        let r = t["runs"].as_array().unwrap().iter().find(|r| r["kind"] == "work" && r["state"] == "running")?;
        Some((home.clone(), t["id"].as_str()?.to_string(), r["id"].as_str()?.to_string()))
    });
    if running.is_none() {
        for (rel, bytes) in parked {
            changed.push((rel, Some(bytes)));
        }
    }
    if changed.is_empty() {
        return finish_no_changes(s, &mut w, &hidden, &mut fixed, &gone_bad);
    }
    let mut open_ids: HashSet<String> = open.iter().filter_map(|(_, t)| t["id"].as_str().map(String::from)).collect();
    let mut touched: HashSet<String> = HashSet::new();
    for (_, t) in &open {
        let changes = board::changes_of(s, t)?;
        touched.extend(changes.iter().filter(|c| c["project"] == here).filter_map(|c| c["entity"].as_str().map(String::from)));
    }
    let list = approved(&s.root);
    let mut before = board::snapshot(s)?;
    let outcome = read_changed(s, &ix, &mut w, &changed)?;
    fixed.extend(outcome.fixed);
    w.save()?;
    let mut dropped = refresh_fixed(s, &mut fixed, &gone_bad)?;
    let mut after = board::snapshot(s)?;
    let unreadable_note = unreadable_note(&outcome.unreadable);
    if running.is_none() && !outcome.unreadable_files.is_empty() {
        for (rel, bytes) in &outcome.unreadable_files {
            let key = format!("file:{rel}");
            let old = ix.modules.iter().find(|m| m.rel() == *rel).map(|m| projection(&ix, m).0.into_bytes());
            before[key.clone()] = old.map_or(Value::Null, |b| json!(b));
            after[key] = bytes.as_ref().map_or(Value::Null, |b| json!(b));
        }
    }
    let keys: HashSet<&String> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    let mut changed_keys: Vec<String> = vec![];
    let mut unexpected_tid = open
        .iter()
        .map(|(_, t)| t)
        .find(|t| t["unexpected"] == true && board::task(&s.root, t["id"].as_str().unwrap_or("")).is_ok())
        .and_then(|t| t["id"].as_str().map(String::from));
    let target = running.as_ref().map(|(_, tid, _)| tid.clone()).or_else(|| unexpected_tid.clone());
    let layout_tasks = |key: &str| -> Result<Vec<String>, Failure> {
        Ok(crate::board::reviews::layout_tasks(s, key)?.into_iter().filter(|tid| open_ids.contains(tid)).collect())
    };
    let (mut unexpected, mut layout, mut approved_layout, mut removed_layout) = (vec![], vec![], vec![], vec![]);
    for key in &keys {
        let key = *key;
        if before[key] == after[key] {
            continue;
        }
        changed_keys.push(key.clone());
        if key.starts_with("symbol:") && before[key].get(4).and_then(Value::as_str) == after[key].get(4).and_then(Value::as_str) {
            layout.push(key.clone());
        }
        let quiet = !touched.contains(key.as_str())
            && match approval(key, &after[key]) {
                Some((k, h)) => list.get(&k) == Some(&h),
                None => approval(key, &before[key]).is_some_and(|(k, _)| !list.contains_key(&k)),
            };
        if !quiet {
            unexpected.push(key.clone());
        } else {
            let decision = classify_quiet_layout(key, &before, &after, target.as_deref(), &layout_tasks)?;
            layout.extend(decision.layout);
            approved_layout.extend(decision.approved);
            removed_layout.extend(decision.removed);
        }
    }
    if running.is_none()
        && let Some(tid) = &unexpected_tid
        && !unexpected.is_empty()
    {
        let gone = crate::board::reviews::drop_unchanged(s, tid, &unexpected, &after)?;
        if !gone.is_empty() {
            unexpected.retain(|k| !gone.contains(k));
            changed_keys.retain(|k| !gone.contains(k));
            refresh_note(s, tid, Ending::Reverted)?;
            dropped.push(tid.clone());
        }
    }
    for (key, holders) in &approved_layout {
        crate::board::reviews::record_approved_layout(s, holders, &before, &after, key)?;
    }
    if running.is_none() && unexpected_tid.is_none() && (!unexpected.is_empty() || !outcome.unreadable.is_empty()) {
        let tid = create_unexpected_task(&top, &here, &flow, &all)?;
        open_ids.insert(tid.clone());
        unexpected_tid = Some(tid);
    }
    let write_target = running.as_ref().map(|(_, tid, _)| tid.as_str()).or(unexpected_tid.as_deref());
    crate::board::reviews::record_endings(s, &before, &after, &changed_keys, &open_ids, write_target)?;
    crate::board::reviews::drop_layout(s, &removed_layout)?;
    dropped.extend(removed_layout.iter().flat_map(|(_, tids)| tids.iter().cloned()));
    if unexpected.is_empty() && outcome.unreadable.is_empty() {
        if !layout.is_empty() {
            if let Some((_, tid, _)) = &running {
                crate::board::reviews::record_layout(s, tid, &before, &after, &layout)?;
            } else if let Some(tid) = &unexpected_tid {
                crate::board::reviews::record_layout(s, tid, &before, &after, &layout)?;
            }
        }
        retire_empty_unexpected(s, &dropped)?;
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
        n.dedup();
        n
    };
    // Tasks change through the task server (D-77).
    if let Some((_, tid, rid)) = running {
        if !unexpected.is_empty() {
            board::record(s, &tid, &rid, &bf, &af, None)?;
        }
        if !layout.is_empty() {
            crate::board::reviews::record_layout(s, &tid, &before, &after, &layout)?;
        }
        let text = format!(
            "{}{unreadable_note}",
            if names.is_empty() {
                String::new()
            } else {
                format!("Changed outside Solace during this run and added to this task: {}.\n", names.join(", "))
            }
        );
        if !text.is_empty() {
            tasks::deliver(&top, json!({"kind":"note","task":tid,"run":rid,"author":"solace","text":text}))?;
        }
        retire_empty_unexpected(s, &dropped)?;
        return Ok(if text.is_empty() { hidden } else { format!("{hidden}Note: {text}") });
    }
    let tid = unexpected_tid.expect("an Unexpected Changes task is created before its changes are recorded");
    if !unexpected.is_empty() {
        board::record(s, &tid, "outside", &bf, &af, None)?;
    }
    for (rel, bytes) in &outcome.unreadable_files {
        if let Some(bytes) = bytes {
            crate::board::reviews::save_after(&s.root, &tid, rel, bytes)?;
        }
    }
    if !layout.is_empty() {
        crate::board::reviews::record_layout(s, &tid, &before, &after, &layout)?;
    }
    let compiled = match s.check() {
        Ok(out) => out,
        Err(Failure(out, _)) => out,
    };
    let compiled: String = compiled.chars().take(4000).collect();
    let changed_note = if names.is_empty() { String::new() } else { format!("Changed outside Solace: {}.\n", names.join(", ")) };
    let note = note_text(s, &tid, Ending::Integrated)?;
    tasks::deliver(
        &top,
        json!({"kind":"outside_note","task":tid,"author":"solace","text":format!("{}\nCheck: {compiled}", note),
            "flag":"code changed outside Solace"}),
    )?;
    retire_empty_unexpected(s, &dropped)?;
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

    /// D-229: a file Solace can't read into symbols is left as it is and
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
        let mut ignores = crate::lockdown::paths::Ignores::new(&p.root);
        assert!(ignores.ignored(&kept));
        let v = settings(&p.root).unwrap();
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
        open(&s.root).unwrap();
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

    const LIB: &str = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";

    fn rust_app() -> (Scratch, Store) {
        let (root, s) = rust_project(LIB);
        board::open(&s.root).unwrap();
        (root, s)
    }

    fn rust_file(s: &Store) -> String {
        fs::read_to_string(s.root.join("src/lib.rs")).unwrap()
    }

    fn rust_text(s: &Store, name: &str) -> Option<String> {
        s.db.query_row("SELECT text FROM symbols WHERE module = 'crate' AND name = ?1 AND live = 1", [name], |r| r.get(0))
            .optional()
            .unwrap()
    }

    /// D-58.1/4/5 for Rust: an outside edit is read in by symbol, nested items
    /// included, and the file is never written over.
    #[test]
    fn rust_edits_are_read_in_and_reviewed() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let edited = LIB.replace("    2", "    22") + "\npub fn three() -> i32 {\n    3\n}\n";
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains(TITLE) && note.contains("crate.three, crate.two"), "{note}");
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    22\n}"));
        assert!(rust_text(&s, "three").is_some());
        assert_eq!(rust_file(&s), edited);
        let ix = s.index().unwrap();
        let m = ix.modules.iter().find(|m| m.path == "crate").unwrap();
        assert_eq!(crate::store::projection(&ix, m).0, edited);
        assert!(crate::store::drift(&ix, m, &edited).is_empty());
        s.materialize(&s.index().unwrap()).unwrap();
        assert_eq!(rust_file(&s), edited);
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        assert_eq!(t["column"], "needs_review");
        assert!(t["notes"][0]["text"].as_str().unwrap().contains("Check:"), "{}", t["notes"]);
    }

    /// D-58.1 for Rust: nested items keep their identity, and two `impl` blocks
    /// of one name pair in order.
    #[test]
    fn rust_nested_items_keep_their_ids() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_project(concat!(
            "pub struct S;\n\nimpl S {\n    pub fn one(&self) -> i32 {\n        1\n    }\n}\n\n",
            "impl S {\n    pub fn two(&self) -> i32 {\n        2\n    }\n}\n",
        ));
        board::open(&s.root).unwrap();
        let id = |name: &str| -> String {
            s.db.query_row("SELECT id FROM symbols WHERE module = 'crate' AND name = ?1 AND live = 1", [name], |r| r.get(0)).unwrap()
        };
        let (s_id, one, two) = (id("S"), id("S::one"), id("S::two"));
        let edited = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn one(&self) -> i32 {\n        11\n    }\n}\n\n",
            "impl S {\n    pub fn two(&self) -> i32 {\n        22\n    }\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("crate.S::one") && note.contains("crate.S::two"), "{note}");
        assert_eq!((id("S"), id("S::one"), id("S::two")), (s_id, one, two));
        assert_eq!(rust_text(&s, "S::one").as_deref(), Some("    pub fn one(&self) -> i32 {\n        11\n    }"));
        let ix = s.index().unwrap();
        let m = ix.modules.iter().find(|m| m.path == "crate").unwrap();
        assert!(crate::store::drift(&ix, m, edited).is_empty());
    }

    /// D-58.6 for Rust: completing keeps the change and puts it on the approved list.
    #[test]
    fn rust_complete_keeps_the_change() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let edited = LIB.replace("    1", "    11");
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    11\n}"));
        assert_eq!(rust_file(&s), edited);
        assert_eq!(approved(&s.root).get("crate.one"), Some(&digest(b"pub fn one() -> i32 {\n    11\n}")));
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
    }

    /// D-58.1 for Rust: cancel restores the file and the store.
    #[test]
    fn rust_cancel_restores_the_file_and_the_store() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let original: String =
            s.db.query_row("SELECT id FROM symbols WHERE module = 'crate' AND name = 'one' AND live = 1", [], |r| r.get(0)).unwrap();
        fs::write(s.root.join("src/lib.rs"), LIB.replace("    1", "    11")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{}/actions", t["id"].as_str().unwrap()), json!({"action":"cancel"})).unwrap();
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    1\n}"));
        let same: String =
            s.db.query_row("SELECT id FROM symbols WHERE module = 'crate' AND name = 'one' AND live = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(same, original);
    }

    /// D-58.6 for Rust: an approved hash is read in without a task.
    #[test]
    fn rust_approved_code_is_read_in_quietly() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let pulled = LIB.replace("    1", "    11");
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), &pulled).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    11\n}"));
        assert!(board::tasks(&s.root).unwrap().is_empty());
    }

    /// D-58.2 for Rust: an outside edit during a run joins that task.
    #[test]
    fn rust_edits_during_a_run_join_its_task() {
        let (_root, s) = rust_app();
        let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":"Work"})).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
        tasks::put(&s, &t).unwrap();
        fs::write(s.root.join("src/lib.rs"), LIB.replace("    2", "    9")).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert_eq!(note, "Note: Changed outside Solace during this run and added to this task: crate.two.\n");
        let t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{}", json!(changes));
        assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
    }

    /// D-58.1 for Rust: a removed file retires its symbols.
    #[test]
    fn a_removed_rust_file_retires_its_symbols() {
        let (_root, s) = rust_project(LIB);
        fs::remove_file(s.root.join("src/lib.rs")).unwrap();
        assert_eq!(absorb(&s, Mode::Command).unwrap(), "");
        assert!(rust_text(&s, "one").is_none());
        assert!(s.index().unwrap().modules.is_empty());
    }

    /// D-229 for Rust: an old refusal is retried once, not hidden by its hash.
    #[test]
    fn an_old_bad_mark_does_not_hide_a_rust_edit() {
        let (_root, s) = rust_project(LIB);
        let edited = LIB.replace("    2", "    22");
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        fs::write(s.root.join(".solace/written"), format!("! {} src/lib.rs\n", digest(edited.as_bytes()))).unwrap();
        assert_eq!(absorb(&s, Mode::Command).unwrap(), "");
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    22\n}"));
    }

    /// D-58 point 4 for Rust: code that doesn't compile is still read in.
    #[test]
    fn rust_code_that_does_not_compile_is_still_read_in() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let broken = LIB.replace("    1", "    \"one\"");
        fs::write(s.root.join("src/lib.rs"), &broken).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("crate.one"), "{note}");
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    \"one\"\n}"));
    }

    /// D-229 for Rust: a file Solace can't split is left as it is and reported once.
    #[test]
    fn a_rust_file_that_cannot_be_split_is_left_alone() {
        let (_root, s) = rust_project(LIB);
        let bad = "pub fn one() -> i32 {\n    1\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        let note = absorb(&s, Mode::Command).unwrap();
        assert!(note.contains("Not read in:"), "{note}");
        assert_eq!(rust_file(&s), bad);
        assert_eq!(absorb(&s, Mode::Command).unwrap(), "");
    }

    fn rust_row(s: &Store, name: &str) -> (String, Option<String>, i64) {
        s.db.query_row("SELECT id, parent, ord FROM symbols WHERE module = 'crate' AND name = ?1 AND live = 1", [name], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
    }

    fn rust_projection(s: &Store) -> String {
        let ix = s.index().unwrap();
        let m = ix.modules.iter().find(|m| m.path == "crate").unwrap();
        crate::store::projection(&ix, m).0
    }

    fn rust_cancel(s: &Store) {
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    }

    /// D-58.1 for Rust: cancel restores a function added after an unchanged one.
    #[test]
    fn rust_cancel_restores_an_appended_function() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let (one, two) = (rust_row(&s, "one"), rust_row(&s, "two"));
        let edited = format!("{LIB}\npub fn three() -> i32 {{\n    3\n}}\n");
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
        rust_cancel(&s);
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!((rust_row(&s, "one"), rust_row(&s, "two")), (one, two));
        assert!(rust_text(&s, "three").is_none());
    }

    /// D-58.1 for Rust: cancel restores the function removed from the end.
    #[test]
    fn rust_cancel_restores_a_removed_last_function() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let (one, two) = (rust_row(&s, "one"), rust_row(&s, "two"));
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n").unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        rust_cancel(&s);
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!((rust_row(&s, "one"), rust_row(&s, "two")), (one, two));
    }

    /// D-58.1 for Rust: cancel restores a swap whose moved function kept its text.
    #[test]
    fn rust_cancel_restores_a_reordered_and_edited_function() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let (one, two) = (rust_row(&s, "one"), rust_row(&s, "two"));
        let edited = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("crate.one"), "{note}");
        assert!(!note.contains("crate.two"), "{note}");
        rust_cancel(&s);
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!((rust_row(&s, "one"), rust_row(&s, "two")), (one, two));
    }

    /// D-58.1 for Rust: a method moved into another `impl S` keeps its id, and
    /// cancel restores the block the method left.
    #[test]
    fn rust_cancel_restores_a_removed_impl_block() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n",
            "impl S {\n    pub fn b(&self) -> i32 {\n        2\n    }\n}\n",
        );
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let (a, b) = (rust_row(&s, "S::a"), rust_row(&s, "S::b"));
        let edited = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n",
            "    pub fn b(&self) -> i32 {\n        2\n    }\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        assert_eq!(rust_row(&s, "S::a").0, a.0);
        assert_eq!(rust_row(&s, "S::b").0, b.0);
        rust_cancel(&s);
        assert_eq!(rust_file(&s), original);
        assert_eq!(rust_projection(&s), original);
        assert_eq!((rust_row(&s, "S::a"), rust_row(&s, "S::b")), (a, b));
    }

    /// D-58.2 for Rust: an outside edit during a run is recorded so cancel puts
    /// the file and the store back.
    #[test]
    fn rust_cancel_restores_an_edit_during_a_run() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let (one, two) = (rust_row(&s, "one"), rust_row(&s, "two"));
        let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":"Work"})).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
        tasks::put(&s, &t).unwrap();
        let edited = format!("{LIB}\npub fn three() -> i32 {{\n    3\n}}\n");
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
        let t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
        let tid = t["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!((rust_row(&s, "one"), rust_row(&s, "two")), (one, two));
    }

    fn rust_work_task(s: &Store, title: &str, rid: &str) -> Value {
        let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":title})).unwrap();
        t["runs"] = json!([{"id":rid,"kind":"work","state":"running"}]);
        tasks::put(s, &t).unwrap();
        t
    }

    /// A layout-only row still orders undo against a later task's edit.
    #[test]
    fn a_layout_only_cancel_conflicts_with_a_later_edit() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let a = rust_work_task(&s, "A", "ra");
        let swapped = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
        a["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &a).unwrap();
        let _b = rust_work_task(&s, "B", "rb");
        let edited = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let tid = a["id"].as_str().unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap_err();
        assert!(e.0.contains("E7007"), "{}", e.0);
        assert!(e.0.contains("crate.two"), "{}", e.0);
        assert!(!e.0.contains("symbol:"), "{}", e.0);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    22\n}"));
        assert_eq!(rust_file(&s), edited);
        assert_eq!(rust_projection(&s), edited);
    }

    /// A later task that has settled still prevents an earlier layout-only undo.
    #[test]
    fn a_settled_later_edit_still_conflicts_with_an_earlier_layout() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let a = rust_work_task(&s, "A", "ra");
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n").unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
        a["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &a).unwrap();
        let b = rust_work_task(&s, "B", "rb");
        let edited = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let b = board::task(&s.root, b["id"].as_str().unwrap()).unwrap();
        crate::board::reviews::settle(&s, &b).unwrap();
        let tid = a["id"].as_str().unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap_err();
        assert!(e.0.contains("E7007"), "{}", e.0);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    22\n}"));
        assert_eq!(rust_file(&s), edited);
        assert_eq!(rust_projection(&s), edited);
    }

    /// D-58.1 for Rust: a later reorder joins a task that is already open for review.
    #[test]
    fn rust_cancel_restores_a_reorder_after_an_open_task() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let rows = (rust_row(&s, "one"), rust_row(&s, "two"), rust_row(&s, "three"));
        let edited = original.replace("    1", "    11");
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let notes = t["notes"].as_array().unwrap().len();
        let swapped = edited.replace(
            "pub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n",
            "pub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    2\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), &swapped).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        assert_eq!(t["notes"].as_array().unwrap().len(), notes);
        assert_eq!(board::changes_of(&s, &t).unwrap().len(), 1);
        assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
        rust_cancel(&s);
        assert_eq!(rust_file(&s), original);
        assert_eq!(rust_projection(&s), original);
        assert_eq!((rust_row(&s, "one"), rust_row(&s, "two"), rust_row(&s, "three")), rows);
    }

    /// D-58.1 for Gleam: cancel restores a file reordered around one edit.
    #[test]
    fn a_layout_only_cancel_restores_a_gleam_file() {
        let original = concat!("pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n\n", "pub fn three() -> Int {\n  3\n}\n",);
        let p = project(&[("src/app.gleam", original)]);
        board::open(&p.root).unwrap();
        let s = p;
        let edited = original.replace("  1", "  11");
        fs::write(s.root.join("src/app.gleam"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("app.one"));
        let swapped = edited.replace(
            "pub fn two() -> Int {\n  2\n}\n\npub fn three() -> Int {\n  3\n}\n",
            "pub fn three() -> Int {\n  3\n}\n\npub fn two() -> Int {\n  2\n}\n",
        );
        fs::write(s.root.join("src/app.gleam"), &swapped).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(file(&s), original);
    }

    /// An approved text change keeps its bytes while cancel restores its place.
    #[test]
    fn an_approved_edit_keeps_its_place_on_cancel() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert_ne!(rust_row(&s, "one").2, rust_row(&s, "two").2);
    }

    /// A recorded file with no module row is forgotten, not refused.
    #[test]
    fn a_written_entry_with_no_module_is_forgotten() {
        let (_root, s) = rust_project(LIB);
        let mut w = Written::load(&s.root);
        w.record("src/gone.rs", b"pub fn gone() {}");
        w.save().unwrap();
        assert_eq!(absorb(&s, Mode::Command).unwrap(), "");
        assert!(Written::load(&s.root).files.get("src/gone.rs").is_none());
    }

    /// An approved function added beside an unexpected edit survives cancel.
    #[test]
    fn an_approved_added_function_survives_cancel() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.insert("crate.three".into(), digest(b"pub fn three() -> i32 {\n    3\n}"));
        save_approved(&s.root, &list).unwrap();
        let edited = format!("{}\npub fn three() -> i32 {{\n    3\n}}\n", LIB.replace("    2", "    22"));
        fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        rust_cancel(&s);
        let expected = format!("{LIB}pub fn three() -> i32 {{\n    3\n}}\n");
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    2\n}"));
        assert_eq!(rust_text(&s, "three").as_deref(), Some("pub fn three() -> i32 {\n    3\n}"));
        let (one, two, three) = (rust_row(&s, "one").2, rust_row(&s, "two").2, rust_row(&s, "three").2);
        assert!(one < two && two < three, "{one} {two} {three}");
    }

    /// An approved edit that arrives after a layout keeps its text and place.
    #[test]
    fn an_approved_edit_after_a_layout_is_kept_on_cancel() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let approved_text = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    11\n}"));
        assert!(rust_row(&s, "one").2 < rust_row(&s, "two").2);
    }

    /// A later save that only moves rows leaves the earlier edit undoable.
    #[test]
    fn a_move_after_an_edit_is_undone_by_cancel() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        fs::write(s.root.join("src/lib.rs"), LIB.replace("    2", "    22")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    2\n}"));
    }

    /// A row the task added is deleted on cancel, even after a later move.
    #[test]
    fn a_moved_added_row_is_removed_on_cancel() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let added = format!("{LIB}\npub fn three() -> i32 {{\n    3\n}}\n");
        fs::write(s.root.join("src/lib.rs"), &added).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
        let moved = "pub fn three() -> i32 {\n    3\n}\n\npub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        fs::write(s.root.join("src/lib.rs"), moved).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert!(rust_text(&s, "three").is_none());
    }

    /// An outside move during a run leaves the agent's edit undoable.
    #[test]
    fn an_outside_move_during_a_run_keeps_the_agents_edit_undoable() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let t = rust_work_task(&s, "A", "ra");
        let tid = t["id"].as_str().unwrap().to_string();
        let before = board::snapshot(&s).unwrap();
        s.submit_symbols(Some("crate.two"), None, "pub fn two() -> i32 {\n    22\n}\n").unwrap();
        let after = board::snapshot(&s).unwrap();
        board::record(&s, &tid, "ra", &before, &after, None).unwrap();
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let mut t = board::task(&s.root, &tid).unwrap();
        t["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &t).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(rust_file(&s), LIB);
        assert_eq!(rust_projection(&s), LIB);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    2\n}"));
    }

    /// An approved unmoved edit the task never touched adds no undo or conflict row.
    #[test]
    fn an_approved_edit_the_task_never_touched_records_nothing() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        fs::write(s.root.join("src/lib.rs"), LIB.replace("    2", "    22")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let both = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    22\n}\n";
        fs::write(s.root.join("src/lib.rs"), both).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let key = {
            let snap = board::snapshot(&s).unwrap();
            snap.as_object().unwrap().iter().find(|(_, v)| v[2].as_str() == Some("one")).map(|(k, _)| k.clone()).unwrap()
        };
        let before: i64 =
            s.db.query_row("SELECT COUNT(*) FROM board_before WHERE task=?1 AND entity=?2", params![tid, key], |r| r.get(0)).unwrap();
        let touches: i64 =
            s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task=?1 AND entity=?2", params![tid, key], |r| r.get(0)).unwrap();
        assert_eq!((before, touches), (0, 0));
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES ('other',?1,'work','crate.one')", [&key]).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// An approved row inserted between others keeps a distinct place on cancel.
    #[test]
    fn an_approved_insert_between_rows_gets_a_distinct_order() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.insert("crate.three".into(), digest(b"pub fn three() -> i32 {\n    3\n}"));
        save_approved(&s.root, &list).unwrap();
        let inserted = "pub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    22\n}\n";
        fs::write(s.root.join("src/lib.rs"), inserted).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        let (one, three, two) = (rust_row(&s, "one").2, rust_row(&s, "three").2, rust_row(&s, "two").2);
        assert!(one < three && three < two, "{one} {three} {two}");
    }

    fn rust_cancel_task(s: &Store, tid: &str) {
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    }

    /// An approved edit updates every open task that holds its row as layout,
    /// without giving a task that doesn't hold it a new undo row.
    #[test]
    fn an_approved_edit_keeps_its_text_for_another_open_task() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let uid = u["id"].as_str().unwrap().to_string();
        let b = rust_work_task(&s, "B", "rb");
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let approved_text = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let key = {
            let snap = board::snapshot(&s).unwrap();
            snap.as_object().unwrap().iter().find(|(_, v)| v[2].as_str() == Some("one")).map(|(k, _)| k.clone()).unwrap()
        };
        let count = |table: &str, tid: &str| -> i64 {
            s.db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE task=?1 AND entity=?2"), params![tid, key], |r| r.get(0)).unwrap()
        };
        assert_eq!((count("board_before", b["id"].as_str().unwrap()), count("board_touches", b["id"].as_str().unwrap())), (0, 0));
        let mut b = board::task(&s.root, b["id"].as_str().unwrap()).unwrap();
        b["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &b).unwrap();
        rust_cancel_task(&s, &uid);
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// An approved edit keeps its text when no run or Unexpected Changes task is open.
    #[test]
    fn an_approved_edit_after_a_layout_with_no_target_keeps_its_text() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let a = rust_work_task(&s, "A", "ra");
        let swapped = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
        a["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &a).unwrap();
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let approved_text = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        rust_cancel_task(&s, a["id"].as_str().unwrap());
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// An approved text change is kept even when another change in the same read joins the task.
    #[test]
    fn an_approved_edit_and_an_unexpected_edit_are_read_in_together() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        save_approved(&s.root, &list).unwrap();
        let both =
            concat!("pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), both).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
        rust_cancel_task(&s, u["id"].as_str().unwrap());
        let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "three").is_none());
    }

    /// An approved removal of a layout row stays gone when the holding task is canceled.
    #[test]
    fn an_approved_removal_of_a_layout_row_keeps_the_removal() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let mut list = approved(&s.root);
        list.remove("crate.one");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    22\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, u["id"].as_str().unwrap());
        let expected = "pub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "one").is_none());
    }

    /// A restored row that becomes last ends the file with one newline.
    #[test]
    fn a_cancel_after_an_approved_removal_of_the_last_row_ends_with_one_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let swapped = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        list.remove("crate.two");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// An approved removal stays gone when a running task holds the removed row's layout.
    #[test]
    fn an_approved_removal_during_a_run_keeps_the_removal() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let t = rust_work_task(&s, "A", "ra");
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n").unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.remove("crate.one");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        rust_cancel_task(&s, t["id"].as_str().unwrap());
        let expected = "pub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "one").is_none());
    }

    /// An approved removal stays gone when no run or Unexpected Changes task is open.
    #[test]
    fn an_approved_removal_after_a_layout_with_no_target_keeps_the_removal() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        let a = rust_work_task(&s, "A", "ra");
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n").unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
        a["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &a).unwrap();
        let mut list = approved(&s.root);
        list.remove("crate.one");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        rust_cancel_task(&s, a["id"].as_str().unwrap());
        let expected = "pub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "one").is_none());
    }

    /// A Gleam cancel keeps an approved edit and an approved removal.
    #[test]
    fn a_gleam_cancel_keeps_an_approved_edit_and_an_approved_removal() {
        let original = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n\npub fn three() -> Int {\n  3\n}\n";
        let p = project(&[("src/app.gleam", original)]);
        board::open(&p.root).unwrap();
        let s = p;
        let a = rust_work_task(&s, "A", "ra");
        let swapped = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  1\n}\n\npub fn three() -> Int {\n  3\n}\n";
        fs::write(s.root.join("src/app.gleam"), swapped).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.insert("app.one".into(), digest(b"pub fn one() -> Int {\n  11\n}"));
        save_approved(&s.root, &list).unwrap();
        let approved_text = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  11\n}\n\npub fn three() -> Int {\n  3\n}\n";
        fs::write(s.root.join("src/app.gleam"), approved_text).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.remove("app.two");
        save_approved(&s.root, &list).unwrap();
        let remaining = "pub fn one() -> Int {\n  11\n}\n\npub fn three() -> Int {\n  3\n}\n";
        fs::write(s.root.join("src/app.gleam"), remaining).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        rust_cancel_task(&s, a["id"].as_str().unwrap());
        assert_eq!(file(&s), remaining);
    }

    fn cancel_after_editing_one(original: &str) {
        cancel_after_outside_edit(original, &original.replace("    1", "    11"), "crate.one");
    }

    /// Cancel restores a file that ended without a newline.
    #[test]
    fn a_cancel_keeps_a_file_without_a_final_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_editing_one("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}");
    }

    /// Cancel restores a file that ended in blank lines.
    #[test]
    fn a_cancel_keeps_a_file_ending_in_blank_lines() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_editing_one("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\n\n");
    }

    /// An approved removal of the last row keeps a file's missing final newline.
    #[test]
    fn a_cancel_after_removing_the_last_row_keeps_no_final_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}";
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let swapped = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        list.remove("crate.two");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    fn cancel_after_outside_edit(original: &str, edited: &str, changed: &str) {
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains(changed));
        rust_cancel(&s);
        assert_eq!(rust_file(&s), original);
        assert_eq!(rust_projection(&s), original);
    }

    /// Cancel keeps a file with no final newline after an outside append.
    #[test]
    fn a_cancel_keeps_a_file_without_a_final_newline_after_appending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_outside_edit(
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}",
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n",
            "crate.three",
        );
    }

    /// Cancel keeps a file with no final newline after an outside swap.
    #[test]
    fn a_cancel_keeps_a_file_without_a_final_newline_after_swapping() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_outside_edit(
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}",
            "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n",
            "crate.one",
        );
    }

    /// Cancel keeps a file ending in blank lines after an outside swap.
    #[test]
    fn a_cancel_keeps_a_file_ending_in_blank_lines_after_swapping() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_outside_edit(
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n",
            "crate.one",
        );
    }

    /// Cancel keeps a file with a final newline after an outside append.
    #[test]
    fn a_cancel_keeps_a_file_with_a_final_newline_after_appending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_outside_edit(
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n",
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}",
            "crate.three",
        );
    }

    fn cancel_after_reorder_and_removal(edited: &str, after_removal: &str, removed: &[&str], expected: &str) {
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        cancel_after_editing_and_removing(
            original,
            edited,
            after_removal,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            removed,
            expected,
        );
    }

    /// Cancel gives the last restored row the ending of the row that was last before the edit.
    #[test]
    fn a_cancel_after_a_reorder_and_removing_the_last_row_keeps_the_new_last_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_reorder_and_removal(
            "pub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n",
            "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n",
            &["crate.three"],
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n",
        );
    }

    /// Cancel keeps order-independent endings when one absorb removes several rows.
    #[test]
    fn a_cancel_after_a_reorder_and_removing_two_rows_keeps_the_true_last_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        for _ in 0..16 {
            cancel_after_reorder_and_removal(
                "pub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n\npub fn one() -> i32 {\n    11\n}\n",
                "pub fn one() -> i32 {\n    11\n}\n",
                &["crate.two", "crate.three"],
                "pub fn one() -> i32 {\n    1\n}\n",
            );
        }
    }

    /// A row the task never held stays gone when another removed row hands its ending down.
    #[test]
    fn a_cancel_after_a_reorder_and_removing_an_unheld_row_keeps_it_gone() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        list.remove("crate.two");
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "two").is_none());
    }

    /// A cancel after a reorder keeps every approved function and undoes the rest.
    #[test]
    fn a_cancel_after_a_reorder_keeps_an_approved_edit_when_a_removed_last_row_hands_down() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited = concat!(
            "pub fn four() -> i32 {\n    4\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn one() -> i32 {\n    11\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.insert("crate.one".into(), digest(b"pub fn one() -> i32 {\n    11\n}"));
        list.remove("crate.four");
        list.insert("crate.five".into(), digest(b"pub fn five() -> i32 {\n    5\n}"));
        list.insert("crate.three".into(), digest(b"pub fn three() -> i32 {\n    33\n}"));
        save_approved(&s.root, &list).unwrap();
        let after = concat!(
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    33\n}\n\n",
            "pub fn five() -> i32 {\n    5\n}\n\n",
            "pub fn one() -> i32 {\n    11\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), after).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        assert_eq!(rust_projection(&s), rust_file(&s));
        for (name, text) in [
            ("one", "pub fn one() -> i32 {\n    1\n}"),
            ("two", "pub fn two() -> i32 {\n    2\n}"),
            ("three", "pub fn three() -> i32 {\n    33\n}"),
            ("five", "pub fn five() -> i32 {\n    5\n}"),
        ] {
            assert_eq!(rust_text(&s, name).as_deref(), Some(text));
        }
        assert!(!rust_file(&s).contains("pub fn four"));
    }

    /// An approved removal of an unheld row leaves it gone even when a held row
    /// is removed with it.
    #[test]
    fn a_cancel_after_a_reorder_and_removing_an_unheld_row_with_a_held_one_keeps_it_gone() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.two");
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "two").is_none());
    }

    /// A receiver with no undo row takes this absorb's approved text.
    #[test]
    fn a_cancel_after_replacing_the_last_row_keeps_the_approved_edit() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited = concat!(
            "pub fn four() -> i32 {\n    4\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn one() -> i32 {\n    11\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.four");
        list.insert("crate.five".into(), digest(b"pub fn five() -> i32 {\n    5\n}"));
        list.insert("crate.three".into(), digest(b"pub fn three() -> i32 {\n    33\n}"));
        save_approved(&s.root, &list).unwrap();
        let after = concat!(
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    33\n}\n\n",
            "pub fn one() -> i32 {\n    11\n}\n\n",
            "pub fn five() -> i32 {\n    5\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), after).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    33\n}\n\n",
            "pub fn five() -> i32 {\n    5\n}\n",
        );
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// A new row that shares the removed row's order is placed after the rows
    /// cancel restores, and takes the ending of the row that was last.
    #[test]
    fn a_cancel_after_removing_the_last_row_and_adding_a_row_keeps_the_new_order() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.three");
        list.insert("crate.five".into(), digest(b"pub fn five() -> i32 {\n    5\n}"));
        save_approved(&s.root, &list).unwrap();
        let after =
            concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n\n", "pub fn five() -> i32 {\n    5\n}\n",);
        fs::write(s.root.join("src/lib.rs"), after).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn five() -> i32 {\n    5\n}\n",);
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// Removing an unheld last row hands its ending to the row the task holds.
    #[test]
    fn a_cancel_after_removing_an_unheld_last_row_keeps_the_new_last_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let mut list = approved(&s.root);
        list.insert("crate.two".into(), digest(b"pub fn two() -> i32 {\n    22\n}"));
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// The ending an unheld last row hands down keeps a missing final newline.
    #[test]
    fn a_cancel_after_removing_an_unheld_last_row_keeps_no_final_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let mut list = approved(&s.root);
        list.insert("crate.two".into(), digest(b"pub fn two() -> i32 {\n    22\n}"));
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// A middle removal leaves the true last row's ending alone.
    #[test]
    fn a_cancel_after_removing_a_middle_row_keeps_the_last_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), original.replace("    1", "    11")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.two");
        save_approved(&s.root, &list).unwrap();
        let after = concat!("pub fn one() -> i32 {\n    11\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), after).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// Removing the first row hands no ending down and keeps the last one.
    #[test]
    fn a_cancel_after_removing_the_first_row_keeps_the_last_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let b = rust_work_task(&s, "B", "rb");
        let reordered =
            concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    1\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), reordered).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.remove("crate.one");
        save_approved(&s.root, &list).unwrap();
        let after = concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), after).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, b["id"].as_str().unwrap());
        assert_eq!(rust_file(&s), after);
        assert_eq!(rust_projection(&s), after);
    }

    fn cancel_after_removal_with_an_unapproved_five(edited: &str, later: &str) {
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert!(rust_text(&s, "five").is_none());
    }

    /// An unapproved row after the removed last row does not become the receiver.
    #[test]
    fn a_cancel_after_removing_the_last_row_before_an_unapproved_row_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_removal_with_an_unapproved_five(
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",),
            concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn five() -> i32 {\n    5\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",),
        );
    }

    /// An unapproved row after an earlier row does not become the receiver.
    #[test]
    fn a_cancel_after_removing_the_last_row_after_an_unapproved_row_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_removal_with_an_unapproved_five(
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",),
            concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n\n", "pub fn five() -> i32 {\n    5\n}\n",),
        );
    }

    /// An unapproved row at the end does not take the ending from the true last row.
    #[test]
    fn a_cancel_after_removing_the_last_row_and_an_unapproved_row_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        cancel_after_removal_with_an_unapproved_five(
            "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n",
            "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn five() -> i32 {\n    5\n}\n",
        );
    }

    /// An unapproved edit to the new last row still goes back on cancel.
    #[test]
    fn a_cancel_after_removing_the_last_row_keeps_an_unapproved_edit_undoable() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited =
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n").unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    /// Canceling after an unheld removal works, and no task gets a row for a
    /// function neither it nor the removal touched.
    #[test]
    fn a_cancel_after_removing_an_unheld_last_row_marks_no_other_task() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), original.replace("    1", "    11")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let b = rust_work_task(&s, "B", "rb");
        let with_two = concat!(
            "pub fn one() -> i32 {\n    11\n}\n\n",
            "pub fn two() -> i32 {\n    22\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        fs::write(s.root.join("src/lib.rs"), with_two).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
        let mut b = board::task(&s.root, b["id"].as_str().unwrap()).unwrap();
        b["runs"][0]["state"] = json!("ended");
        tasks::put(&s, &b).unwrap();
        let mut list = approved(&s.root);
        list.remove("crate.four");
        save_approved(&s.root, &list).unwrap();
        let without_four =
            concat!("pub fn one() -> i32 {\n    11\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        fs::write(s.root.join("src/lib.rs"), without_four).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        let key = {
            let snap = board::snapshot(&s).unwrap();
            snap.as_object().unwrap().iter().find(|(_, v)| v[2].as_str() == Some("three")).map(|(k, _)| k.clone()).unwrap()
        };
        let tid = b["id"].as_str().unwrap();
        let count: i64 =
            s.db.query_row("SELECT COUNT(*) FROM board_before WHERE task=?1 AND entity=?2", params![tid, key], |r| r.get(0)).unwrap();
        let layout: i64 = s
            .db
            .query_row("SELECT COUNT(*) FROM board_touches WHERE task=?1 AND entity=?2 AND run='layout'", params![tid, key], |r| r.get(0))
            .unwrap();
        assert_eq!((count, layout), (0, 0));
        rust_cancel(&s);
        assert_eq!(rust_text(&s, "three").as_deref(), Some("pub fn three() -> i32 {\n    3\n}"));
    }

    fn cancel_after_editing_and_removing(
        original: &str,
        edited: &str,
        later: &str,
        changed: &str,
        approved_text: &[(&str, &str)],
        removed: &[&str],
        expected: &str,
    ) {
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains(changed));
        let mut list = approved(&s.root);
        for (name, text) in approved_text {
            list.insert((*name).into(), digest(text.as_bytes()));
        }
        for name in removed {
            list.remove(*name);
        }
        save_approved(&s.root, &list).unwrap();
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_without_reordering_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let edited = original.replace("    1", "    11");
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.three"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_without_reordering_keeps_no_final_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}");
        let edited = original.replace("    1", "    11");
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}";
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}";
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.three"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_without_reordering_keeps_blank_lines() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n\n",
        );
        let edited = original.replace("    1", "    11");
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\n\n";
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.three"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_without_reordering_keeps_an_unapproved_edit_undoable() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let edited = original.replace("    1", "    11");
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    22\n}\n";
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.three"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_and_a_middle_row_without_reordering_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let edited = original.replace("    1", "    11");
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn three() -> i32 {\n    3\n}\n";
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n";
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.two", "crate.four"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_method_without_reordering_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub struct S;\n\nimpl S {\n",
            "    pub fn a(&self) -> i32 {\n        1\n    }\n\n",
            "    pub fn b(&self) -> i32 {\n        2\n    }\n\n",
            "    pub fn c(&self) -> i32 {\n        3\n    }\n}\n",
        );
        let edited = original.replace("        1\n", "        11\n");
        let later = concat!(
            "pub struct S;\n\nimpl S {\n",
            "    pub fn a(&self) -> i32 {\n        11\n    }\n\n",
            "    pub fn b(&self) -> i32 {\n        2\n    }\n}\n",
        );
        let expected = concat!(
            "pub struct S;\n\nimpl S {\n",
            "    pub fn a(&self) -> i32 {\n        1\n    }\n\n",
            "    pub fn b(&self) -> i32 {\n        2\n    }\n}\n",
        );
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.S::a",
            &[("crate.S::a", "    pub fn a(&self) -> i32 {\n        11\n    }")],
            &["crate.S::c"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_without_reordering_keeps_an_approved_edit() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let t = rust_work_task(&s, "A", "ra");
        let reordered = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n";
        fs::write(s.root.join("src/lib.rs"), reordered).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let mut list = approved(&s.root);
        list.insert("crate.two".into(), digest(b"pub fn two() -> i32 {\n    22\n}"));
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}\n";
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, t["id"].as_str().unwrap());
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_during_a_run_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let t = rust_work_task(&s, "A", "ra");
        let edited = original.replace("    1", "    11");
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, t["id"].as_str().unwrap());
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
    }

    #[test]
    fn a_gleam_cancel_after_removing_the_last_row_keeps_the_ending() {
        let original = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n\npub fn three() -> Int {\n  3\n}\n";
        let p = project(&[("src/app.gleam", original)]);
        board::open(&p.root).unwrap();
        let s = p;
        let edited = original.replace("  1", "  11");
        fs::write(s.root.join("src/app.gleam"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("app.one"));
        let mut list = approved(&s.root);
        list.remove("app.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> Int {\n  11\n}\n\npub fn two() -> Int {\n  2\n}\n";
        fs::write(s.root.join("src/app.gleam"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n";
        assert_eq!(file(&s), expected);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_when_a_run_holds_nothing_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let t = rust_work_task(&s, "A", "ra");
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, t["id"].as_str().unwrap());
        assert_eq!(rust_file(&s), later);
        assert_eq!(rust_projection(&s), later);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_when_a_run_holds_nothing_keeps_no_final_newline() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}");
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let t = rust_work_task(&s, "A", "ra");
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}";
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel_task(&s, t["id"].as_str().unwrap());
        assert_eq!(rust_file(&s), later);
        assert_eq!(rust_projection(&s), later);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_when_the_task_holds_another_level_keeps_the_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n",
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n",
        );
        let edited = original.replace("        1\n", "        11\n");
        let later = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        11\n    }\n}\n\n",
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n",
        );
        let expected = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n",
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n",
        );
        cancel_after_editing_and_removing(original, &edited, later, "crate.S::a", &[], &["crate.three"], expected);
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_after_a_reorder_keeps_the_restored_order() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let edited = original.replace("    1", "    11");
        let later =
            concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
        let expected =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.four"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_after_another_reorder_keeps_the_restored_order() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn three() -> i32 {\n    3\n}\n\n",
            "pub fn four() -> i32 {\n    4\n}\n",
        );
        let edited = original.replace("    1", "    11");
        let later =
            concat!("pub fn one() -> i32 {\n    11\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n",);
        let expected =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        cancel_after_editing_and_removing(
            original,
            &edited,
            later,
            "crate.one",
            &[("crate.one", "pub fn one() -> i32 {\n    11\n}")],
            &["crate.four"],
            expected,
        );
    }

    #[test]
    fn a_cancel_after_removing_the_last_row_keeps_an_unapproved_removal() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original =
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        let edited = original.replace("    1", "    11");
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let mut list = approved(&s.root);
        list.remove("crate.three");
        save_approved(&s.root, &list).unwrap();
        let later = "pub fn one() -> i32 {\n    11\n}\n";
        fs::write(s.root.join("src/lib.rs"), later).unwrap();
        let _ = absorb(&s, Mode::Notice).unwrap();
        rust_cancel(&s);
        let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        assert_eq!(rust_file(&s), expected);
        assert_eq!(rust_projection(&s), expected);
        assert_eq!(rust_text(&s, "two").as_deref(), Some("pub fn two() -> i32 {\n    2\n}"));
    }

    /// D-58: a level whose rows did not change records no ending of its own.
    #[test]
    fn an_untouched_impl_block_records_one_ending() {
        if !crate::language::cargo::installed() {
            return;
        }
        let original = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n",
            "pub fn one() -> i32 {\n    1\n}\n",
        );
        let edited = concat!(
            "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n",
            "pub fn one() -> i32 {\n    2\n}\n",
        );
        let (_root, s) = rust_project(original);
        board::open(&s.root).unwrap();
        fs::write(s.root.join("src/lib.rs"), edited).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap();
        let rows: i64 = s.db.query_row("SELECT COUNT(*) FROM board_ends WHERE task=?1", [tid], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    /// D-229: an unreadable Gleam edit is held as a file change, blocks
    /// Complete, and Cancel restores what Solace last wrote.
    #[test]
    fn an_unreadable_edit_is_a_file_change_that_can_be_cancelled() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/app.gleam"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(changes.iter().any(|c| c["entity"] == "file:src/app.gleam"), "{}", json!(changes));
        let diff = board::reviews::task_diff(&s, &tid, &[], 8000).unwrap();
        assert!(diff.contains("src/app.gleam") && diff.contains("pub fn two"), "{diff}");
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap_err();
        assert!(e.0.contains("E7032") && e.0.contains("can't read"), "{}", e.0);
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(file(&s), APP);
        assert!(Written::load(&s.root).bad.is_empty());
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
    }

    /// D-229: once an unreadable edit can be read in, its file change becomes
    /// symbol changes and Complete works.
    #[test]
    fn an_unreadable_edit_that_is_fixed_shows_code_and_completes() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let fixed = APP.replace("  1", "  11");
        fs::write(s.root.join("src/app.gleam"), &fixed).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(!note.contains("Not read in"), "{note}");
        let t = board::task(&s.root, &tid).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(!changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("file:"))), "{}", json!(changes));
        assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{}", json!(changes));
        assert!(t["notes"].as_array().unwrap().iter().all(|n| !n["text"].as_str().unwrap_or("").contains("Not read in")));
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(file(&s), fixed);
    }

    /// D-229 for Rust: an unsplittable edit is a file change, blocks Complete,
    /// and Cancel puts back Solace's file and clears the mark.
    #[test]
    fn an_unreadable_rust_edit_is_reviewed_and_can_be_cancelled() {
        let (_root, s) = rust_project(LIB);
        board::open(&s.root).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let bad = "pub fn one() -> i32 {\n    1\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/lib.rs"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/lib.rs"));
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap_err();
        assert!(e.0.contains("E7032") && e.0.contains("can't read"), "{}", e.0);
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(rust_file(&s), LIB);
        assert!(Written::load(&s.root).bad.is_empty());
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
    }

    /// D-229 for Rust: an agent redoes the edit through the store; the file
    /// is replaced, its outside diff stays, and Complete works.
    #[test]
    fn an_unreadable_rust_edit_fixed_through_the_store_completes() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_project(LIB);
        board::open(&s.root).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let bad = "pub fn one() -> i32 {\n    7\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        s.submit_symbols(Some("crate.one"), None, "pub fn one() -> i32 {\n    11\n}\n").unwrap();
        let fixed = LIB.replace("    1", "    11");
        assert_eq!(rust_file(&s), fixed);
        assert!(Written::load(&s.root).bad.is_empty());
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(changes.iter().any(|c| c["entity"] == "file:src/lib.rs"), "{}", json!(changes));
        let diff = board::reviews::task_diff(&s, &tid, &[], 8000).unwrap();
        assert!(diff.contains("    7"), "{diff}");
        assert!(t["notes"].as_array().unwrap().iter().all(|n| !n["text"].as_str().unwrap_or("").contains("Not read in")), "{}", t["notes"]);
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(rust_file(&s), fixed);
    }

    /// D-229 for Rust: once the edited file can be split, its file change
    /// becomes symbol changes and Complete works.
    #[test]
    fn an_unreadable_rust_edit_that_is_read_in_completes() {
        let (_root, s) = rust_project(LIB);
        board::open(&s.root).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let bad = "pub fn one() -> i32 {\n    1\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let fixed = LIB.replace("    1", "    11");
        fs::write(s.root.join("src/lib.rs"), &fixed).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(!note.contains("Not read in"), "{note}");
        let t = board::task(&s.root, &tid).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(!changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("file:"))), "{}", json!(changes));
        assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{}", json!(changes));
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(rust_file(&s), fixed);
        assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    11\n}"));
    }

    /// D-229: a file that isn't UTF-8 and one with a bad module name each hold
    /// a file change, and repeated saves replace the task's note rather than add one.
    #[test]
    fn other_unreadable_causes_are_file_changes_with_one_note() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/Bad-Name.gleam"), "pub fn x() -> Int {\n  1\n}\n").unwrap();
        fs::write(s.root.join("src/raw.gleam"), [0xff, 0xfe]).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/Bad-Name.gleam") && note.contains("file:src/raw.gleam"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let notes = t["notes"].as_array().unwrap().len();
        for (entity, why) in [("file:src/Bad-Name.gleam", "isn't a Gleam module name"), ("file:src/raw.gleam", "isn't UTF-8")] {
            assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == entity), "{entity}");
            assert!(t["notes"][0]["text"].as_str().unwrap().contains(why), "{}", t["notes"]);
        }
        fs::write(s.root.join("src/raw.gleam"), [0xff, 0xfd]).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::task(&s.root, &tid).unwrap();
        assert_eq!(t["notes"].as_array().unwrap().len(), notes);
        let text = t["notes"][0]["text"].as_str().unwrap();
        assert!(text.contains("src/Bad-Name.gleam") && text.contains("src/raw.gleam"), "{text}");
        fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  9")).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::task(&s.root, &tid).unwrap();
        let text = t["notes"][0]["text"].as_str().unwrap();
        assert!(text.contains("app.one") && text.contains("src/Bad-Name.gleam") && text.contains("src/raw.gleam"), "{text}");
    }

    /// D-229: a non-canonical read leaves a digest that differs from the
    /// projection; a later unreadable save is still left on disk.
    #[test]
    fn an_unreadable_edit_is_not_overwritten_by_a_check() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/app.gleam"), format!("{APP}\n\n")).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/app.gleam"), "{note}");
        assert_eq!(file(&s), twice);
        let _ = s.check();
        assert_eq!(file(&s), twice);
    }

    /// D-229: without a task holding the change, a submit keeps D-58's refusal.
    #[test]
    fn an_unreadable_edit_without_a_task_is_never_overwritten() {
        let s = app();
        absorb(&s, Mode::Command).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        assert!(absorb(&s, Mode::Command).unwrap().contains("Not read in:"));
        let e = s.submit_symbols(Some("app.one"), None, "pub fn one() -> Int {\n  5\n}\n").unwrap_err();
        assert!(e.0.starts_with("E0104"), "{}", e.0);
        assert_eq!(file(&s), twice);
    }

    /// D-229: a new unreadable file can be created by the store over its file.
    #[test]
    fn a_new_unreadable_file_can_be_redone_through_the_store() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let bad = "pub fn x() -> Int {\n  1\n}\n\npub fn x() -> Int {\n  2\n}\n";
        fs::write(s.root.join("src/fresh.gleam"), bad).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:src/fresh.gleam"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        s.submit_symbols(None, Some("fresh"), "pub fn x() -> Int {\n  3\n}\n").unwrap();
        let fixed = "pub fn x() -> Int {\n  3\n}\n";
        assert_eq!(fs::read_to_string(s.root.join("src/fresh.gleam")).unwrap(), fixed);
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/fresh.gleam"));
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
    }

    /// D-229: a marked file that is gone and has no module is cleared, not
    /// reported again on every poll.
    #[test]
    fn a_deleted_unreadable_new_file_clears_its_mark() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/Bad-Name.gleam"), "pub fn x() -> Int {\n  1\n}\n").unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:src/Bad-Name.gleam"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        fs::remove_file(s.root.join("src/Bad-Name.gleam")).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        assert!(Written::load(&s.root).bad.is_empty());
        let t = board::task(&s.root, &tid).unwrap();
        assert!(!board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/Bad-Name.gleam"));
        let flow = board::flow(&s.root).unwrap();
        assert_eq!(board::column(&flow, &t)["final"], true);
    }

    /// D-229: a fix that changes no symbol still refreshes the task's note.
    #[test]
    fn a_hand_fix_without_a_symbol_change_clears_the_reason() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/app.gleam"), format!("{APP}\n\n")).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        assert!(t["notes"].as_array().unwrap().iter().all(|n| !n["text"].as_str().unwrap_or("").contains("Not read in")), "{}", t["notes"]);
        let note = t["notes"].as_array().unwrap().iter().find(|n| n["outside"] == true).unwrap();
        assert_eq!(note["text"].as_str(), Some("The changes made outside Solace were integrated.\n"));
    }

    /// D-229: an unreadable edit during a run stays a note, and a submit keeps
    /// D-58's refusal because no file change holds its after copy.
    #[test]
    fn an_unreadable_edit_during_a_run_is_never_overwritten() {
        if !crate::language::cargo::installed() {
            return;
        }
        let (_root, s) = rust_app();
        rust_work_task(&s, "Work", "r1");
        let bad = "pub fn one() -> i32 {\n    1\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("Not read in:"), "{bad}");
        let e = s.submit_symbols(Some("crate.one"), None, "pub fn one() -> i32 {\n    11\n}\n").unwrap_err();
        assert!(e.0.starts_with("E0104"), "{}", e.0);
        assert_eq!(rust_file(&s), bad);
    }

    /// D-229: a file first marked unreadable during a command is picked up by
    /// the board's next poll, so it can be reviewed and canceled.
    #[test]
    fn an_unreadable_edit_marked_during_a_command_is_later_reviewed() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        assert!(absorb(&s, Mode::Command).unwrap().contains("Not read in:"));
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/app.gleam"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/app.gleam"));
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap_err();
        assert!(e.0.contains("E7032"), "{}", e.0);
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
        assert_eq!(file(&s), APP);
        assert!(Written::load(&s.root).bad.is_empty());
    }

    /// D-229: a file first marked during a run stays a note while it goes,
    /// then joins Unexpected Changes once the run stops.
    #[test]
    fn an_unreadable_edit_marked_during_a_run_is_held_after_it_stops() {
        let (_root, s) = rust_app();
        let t = rust_work_task(&s, "Work", "r1");
        let bad = "pub fn one() -> i32 {\n    1\n";
        fs::write(s.root.join("src/lib.rs"), bad).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("Not read in:"));
        assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
        let mut t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
        t["runs"][0]["state"] = json!("done");
        tasks::put(&s, &t).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/lib.rs"), "{note}");
        let unexpected = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        assert!(board::changes_of(&s, &unexpected).unwrap().iter().any(|c| c["entity"] == "file:src/lib.rs"));
    }

    /// An empty Unexpected Changes task no longer blocks work once its file is restored.
    #[test]
    fn a_restored_unreadable_file_closes_the_empty_unexpected_task() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        fs::write(s.root.join("src/app.gleam"), APP).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        let flow = board::flow(&s.root).unwrap();
        assert_eq!(board::column(&flow, &t)["final"], true);
        assert!(board::changes_of(&s, &t).unwrap().is_empty());
        assert!(t["attention"].is_null(), "{}", t["attention"]);
        assert!(t["finished"].is_null());
        let note = t["notes"].as_array().unwrap().iter().find(|n| n["outside"] == true).unwrap();
        assert_eq!(note["text"].as_str(), Some("The changes made outside Solace were reverted.\n"));
    }

    /// An old `!` mark without `@` or `%` still holds the file change for review.
    #[test]
    fn an_old_unreadable_mark_without_a_reason_is_still_reviewed() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let bad = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &bad).unwrap();
        let mut written = fs::read_to_string(s.root.join(".solace/written")).unwrap();
        written.push_str(&format!("! {} src/app.gleam\n", digest(bad.as_bytes())));
        fs::write(s.root.join(".solace/written"), written).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("file:src/app.gleam"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/app.gleam"));
    }

    /// A broken file before the first submit is fixed through the command route and can be completed.
    #[test]
    fn a_broken_file_is_redone_through_the_submit_and_test_commands() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let bad = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &bad).unwrap();
        let fixed = APP.replace("  2", "  22");
        let submit = vec!["submit".to_string(), "app.two".to_string()];
        let out = crate::cli::routing::with_root(&s.root, || {
            crate::cli::routing::run_options(&submit, &mut || Ok("pub fn two() -> Int {\n  22\n}\n".to_string()), None, 100_000, &[])
        })
        .unwrap();
        assert!(out.contains("Saved app.two"), "{out}");
        let test = vec!["test".to_string()];
        let tested = crate::cli::routing::with_root(&s.root, || {
            crate::cli::routing::run_options(&test, &mut || Ok(String::new()), None, 100_000, &[])
        })
        .unwrap();
        assert!(!tested.is_empty(), "test replied");
        assert_eq!(file(&s), fixed);
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
        assert_eq!(file(&s), fixed);
    }

    /// D-229: an emptied Unexpected task stays open while one of its runs is going.
    #[test]
    fn a_restored_file_leaves_the_unexpected_task_open_while_its_run_goes() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        let mut t = board::task(&s.root, &tid).unwrap();
        t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
        tasks::put(&s, &t).unwrap();
        fs::write(s.root.join("src/app.gleam"), APP).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        let flow = board::flow(&s.root).unwrap();
        assert_ne!(board::column(&flow, &t)["final"], true);
        assert_eq!(t["runs"][0]["state"], "running");
        assert!(board::changes_of(&s, &t).unwrap().is_empty());
    }

    /// D-229: restoring a readable edit by hand drops its change and closes the task.
    #[test]
    fn a_restored_readable_symbol_closes_the_empty_unexpected_task() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/app.gleam"), APP.replace("  2", "  9")).unwrap();
        let note = absorb(&s, Mode::Notice).unwrap();
        assert!(note.contains("app.two"), "{note}");
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        fs::write(s.root.join("src/app.gleam"), APP).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        let note = t["notes"].as_array().unwrap().iter().find(|n| n["outside"] == true).unwrap();
        assert_eq!(note["text"].as_str(), Some("The changes made outside Solace were reverted.\n"));
        let flow = board::flow(&s.root).unwrap();
        assert_eq!(board::column(&flow, &t)["final"], true);
        assert!(board::changes_of(&s, &t).unwrap().is_empty());
        let other = board::api(&s.root, "POST", "/tasks", json!({"title":"Other"})).unwrap();
        let oid = other["id"].as_str().unwrap();
        board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"plan"})).unwrap();
        let e = board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"assign","model":"x"})).unwrap_err();
        assert!(!e.0.starts_with("E7012"), "{}", e.0);
    }

    /// D-229: one restored file and one fixed file say both endings when the task empties.
    #[test]
    fn a_mixed_restore_and_fix_says_both_outcomes() {
        const OTHER: &str = "pub fn other() -> Int {\n  4\n}\n";
        let p = project(&[("src/app.gleam", APP), ("src/other.gleam", OTHER)]);
        let s = board::open(&p.root).unwrap();
        absorb(&s, Mode::Notice).unwrap();
        let bad = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
        fs::write(s.root.join("src/app.gleam"), &bad).unwrap();
        fs::write(s.root.join("src/other.gleam"), format!("{OTHER}\npub fn other() -> Int {{\n  5\n}}\n")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        fs::write(s.root.join("src/app.gleam"), APP).unwrap();
        fs::write(s.root.join("src/other.gleam"), format!("{OTHER}\n")).unwrap();
        assert_eq!(absorb(&s, Mode::Notice).unwrap(), "");
        let t = board::task(&s.root, &tid).unwrap();
        let flow = board::flow(&s.root).unwrap();
        assert_eq!(board::column(&flow, &t)["final"], true);
        assert!(board::changes_of(&s, &t).unwrap().is_empty());
        let note = t["notes"].as_array().unwrap().iter().find(|n| n["outside"] == true).unwrap();
        assert_eq!(note["text"].as_str(), Some("Some changes made outside Solace were reverted and others were integrated.\n"));
    }

    /// D-229: deleting a module file whose outside edit could not be read
    /// records its removed symbols instead of failing on the missing file.
    #[test]
    fn a_deleted_unreadable_module_file_records_its_removed_symbols() {
        let s = app();
        absorb(&s, Mode::Notice).unwrap();
        fs::write(s.root.join("src/app.gleam"), format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n")).unwrap();
        assert!(absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
        let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        fs::remove_file(s.root.join("src/app.gleam")).unwrap();
        let got = absorb(&s, Mode::Notice).unwrap();
        assert!(!got.contains("Not read in"), "{got}");
        let t = board::task(&s.root, &tid).unwrap();
        let changes = board::changes_of(&s, &t).unwrap();
        assert!(!changes.iter().any(|c| c["entity"] == "file:src/app.gleam"), "{changes:?}");
        assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{changes:?}");
        assert!(
            t["notes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| n["outside"] == true)
                .all(|n| !n["text"].as_str().unwrap_or("").contains("Not read in")),
            "{}",
            t["notes"]
        );
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
    open(&s.root).unwrap();
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

const RUST_BAD_CLEARED: &str = "# rust-bad-marks-cleared";

fn classify_quiet_layout(
    key: &str,
    before: &Value,
    after: &Value,
    target: Option<&str>,
    layout_tasks: &impl Fn(&str) -> Result<Vec<String>, Failure>,
) -> Result<QuietLayout, Failure> {
    let mut decision = QuietLayout::default();
    if key.starts_with("symbol:") && !before[key].is_null() && !after[key].is_null() && before[key].get(4) != after[key].get(4) {
        let holders = layout_tasks(key)?;
        let target_holds = target.is_some_and(|tid| holders.iter().any(|holder| holder == tid));
        if [5, 8, 9, 10].iter().any(|&i| before[key].get(i) != after[key].get(i)) || target_holds {
            // Approved text keeps its new bytes; cancel still restores its old place.
            decision.layout = Some(key.to_string());
        }
        if !holders.is_empty() {
            decision.approved = Some((key.to_string(), holders));
        }
    } else if key.starts_with("symbol:") && !before[key].is_null() && after[key].is_null() {
        decision.removed = Some((key.to_string(), layout_tasks(key)?));
    }
    Ok(decision)
}

#[derive(Default)]
struct QuietLayout {
    layout: Option<String>,
    approved: Option<(String, Vec<String>)>,
    removed: Option<(String, Vec<String>)>,
}

fn create_unexpected_task(top: &Path, here: &str, flow: &Value, all: &[(PathBuf, Value)]) -> Result<String, Failure> {
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
    tasks::deliver(top, json!({"kind":"create","new":t}))?;
    Ok(t["id"].as_str().unwrap().to_string())
}

/// The first file a task holds that is still unreadable, if any.
pub(crate) fn unreadable_change(s: &Store, t: &Value) -> Result<Option<String>, Failure> {
    let changes = crate::board::reviews::changes_of(s, t)?;
    for (dir, st) in crate::board::reviews::scope_stores(s, t)? {
        let w = Written::load(&st.root);
        for c in changes.iter().filter(|c| c["project"] == dir) {
            let Some(name) = c["entity"].as_str().and_then(|e| e.strip_prefix("file:")) else { continue };
            if !w.marked_bad(name) {
                continue;
            }
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

/// Rewrites a task's outside-changes note from the changes it still holds.
fn refresh_note(s: &Store, tid: &str, ending: Ending) -> Result<(), Failure> {
    let top = scope::top(&s.root);
    let text = note_text(s, tid, ending)?;
    tasks::deliver(&top, json!({"kind":"outside_note","task":tid,"author":"solace","text":text}))
}

/// The store's text for `rel`, or empty when the store has no module there.
fn projected(ix: &crate::Index, rel: &str) -> Vec<u8> {
    ix.modules.iter().find(|m| m.rel() == rel).map_or_else(Vec::new, |m| projection(ix, m).0.into_bytes())
}

/// What a task's outside-changes note says now: every change it holds, and why
/// each held file still can't be read in.
fn note_text(s: &Store, tid: &str, ending: Ending) -> Result<String, Failure> {
    let t = board::task(&s.root, tid)?;
    let changes = crate::board::reviews::changes_of(s, &t)?;
    let mut names: Vec<String> = vec![];
    let mut reasons: Vec<String> = vec![];
    for (dir, st) in crate::board::reviews::scope_stores(s, &t)? {
        let w = Written::load(&st.root);
        for c in changes.iter().filter(|c| c["project"] == dir) {
            if let Some(name) = c["name"].as_str() {
                names.push(name.to_string());
            }
            if let Some(name) = c["entity"].as_str().and_then(|e| e.strip_prefix("file:"))
                && let Some(why) = w.why_bad(name)
            {
                reasons.push(why.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    reasons.sort();
    reasons.dedup();
    if names.is_empty() && reasons.is_empty() {
        return Ok(match ending {
            Ending::Reverted => "The changes made outside Solace were reverted.\n",
            Ending::Integrated => "The changes made outside Solace were integrated.\n",
            Ending::Mixed => "Some changes made outside Solace were reverted and others were integrated.\n",
        }
        .to_string());
    }
    let mut text = String::new();
    if !names.is_empty() {
        text += &format!("Changed outside Solace: {}.\n", names.join(", "));
    }
    for why in reasons {
        text += &format!("Not read in: {why}.\n");
    }
    Ok(text)
}

#[derive(Default)]
struct ReadOutcome {
    fixed: Vec<String>,
    unreadable: Vec<String>,
    unreadable_files: Vec<(String, Option<Vec<u8>>)>,
}

/// Removes the marks of files that are gone and reports the module files that went with them.
fn dropped_gone_bad(
    w: &mut Written,
    disk: &[String],
    changed: &[(String, Option<Vec<u8>>)],
    ix: &crate::Index,
) -> (Vec<String>, Vec<(String, Option<Vec<u8>>)>) {
    let mut gone_bad = vec![];
    let mut removed = vec![];
    for rel in w.bad.keys() {
        if disk.contains(rel) || changed.iter().any(|(r, _)| r == rel) {
            continue;
        }
        if ix.modules.iter().any(|m| m.rel() == *rel) {
            removed.push((rel.clone(), None));
        } else {
            gone_bad.push(rel.clone());
        }
    }
    for rel in &gone_bad {
        w.clear_bad(rel);
    }
    (gone_bad, removed)
}

/// Reads the changed files into the store, keeping the ones that cannot be read as file changes.
fn read_changed(s: &Store, ix: &crate::Index, w: &mut Written, changed: &[(String, Option<Vec<u8>>)]) -> Result<ReadOutcome, Failure> {
    let mut out = ReadOutcome::default();
    for (rel, bytes) in changed {
        let text = match bytes.as_deref().map(std::str::from_utf8) {
            Some(Err(_)) => {
                let why = format!("{rel} isn't UTF-8 text; Solace left it as it is");
                w.mark_bad(rel, bytes.as_deref().unwrap_or_default(), &projected(ix, rel), &why);
                out.unreadable.push(why);
                out.unreadable_files.push((rel.clone(), bytes.clone()));
                continue;
            }
            Some(Ok(t)) => Some(t),
            None => None,
        };
        match read_in(s, rel, text)? {
            Ok(()) => {
                if w.bad.contains_key(rel) {
                    out.fixed.push(rel.clone());
                }
                match bytes {
                    Some(b) => w.record(rel, b),
                    None => {
                        w.clear_bad(rel);
                        w.forget(rel);
                    }
                }
            }
            Err(why) => {
                w.mark_bad(rel, bytes.as_deref().unwrap_or_default(), &projected(ix, rel), &why);
                out.unreadable.push(why);
                out.unreadable_files.push((rel.clone(), bytes.clone()));
            }
        }
    }
    Ok(out)
}

/// Drops the file changes for fixed and gone files, returning every task that held one.
fn refresh_fixed(s: &Store, fixed: &mut Vec<String>, gone_bad: &[String]) -> Result<Vec<String>, Failure> {
    fixed.sort();
    fixed.dedup();
    let mut endings: HashMap<String, Ending> = HashMap::new();
    for rel in fixed.iter() {
        for tid in crate::board::reviews::file_change_tasks(s, std::slice::from_ref(rel))? {
            let ending = fixed_ending(s, &tid, rel)?;
            endings.entry(tid).and_modify(|e| *e = combine_endings(*e, ending)).or_insert(ending);
        }
    }
    for rel in gone_bad {
        for tid in crate::board::reviews::file_change_tasks(s, std::slice::from_ref(rel))? {
            endings.entry(tid).and_modify(|e| *e = combine_endings(*e, Ending::Reverted)).or_insert(Ending::Reverted);
        }
    }
    let mut names = fixed.clone();
    names.extend(gone_bad.iter().cloned());
    names.sort();
    names.dedup();
    let mut dropped = crate::board::reviews::drop_file_changes(s, &names)?;
    for tid in &dropped {
        refresh_note(s, tid, endings.get(tid).copied().unwrap_or(Ending::Integrated))?;
    }
    dropped.sort();
    dropped.dedup();
    Ok(dropped)
}

/// Saves the written record and retires any Unexpected task left empty by the dropped changes.
fn finish_no_changes(s: &Store, w: &mut Written, hidden: &str, fixed: &mut Vec<String>, gone_bad: &[String]) -> Result<String, Failure> {
    w.save()?;
    let dropped = refresh_fixed(s, fixed, gone_bad)?;
    retire_empty_unexpected(s, &dropped)?;
    Ok(hidden.to_string())
}

/// Closes an Unexpected task that no longer holds a change.
fn retire_empty_unexpected(s: &Store, tids: &[String]) -> Result<(), Failure> {
    let top = scope::top(&s.root);
    let flow = board::flow(&s.root)?;
    for tid in tids {
        let Ok(t) = crate::board::task_model::task_at_home(&s.root, tid) else { continue };
        if t["unexpected"] != true || board::column(&flow, &t)["final"] == true {
            continue;
        }
        if !crate::board::reviews::changes_of(s, &t)?.is_empty() {
            continue;
        }
        tasks::deliver(&top, json!({"kind":"close","task":tid}))?;
    }
    Ok(())
}

fn unreadable_note(unreadable: &[String]) -> String {
    unreadable.iter().map(|u| format!("Not read in: {u}.\n")).collect()
}

/// What brought an Unexpected task's changes to nothing.
#[derive(Clone, Copy)]
enum Ending {
    Reverted,
    Integrated,
    Mixed,
}

fn combine_endings(a: Ending, b: Ending) -> Ending {
    match (a, b) {
        (Ending::Reverted, Ending::Reverted) => Ending::Reverted,
        (Ending::Integrated, Ending::Integrated) => Ending::Integrated,
        _ => Ending::Mixed,
    }
}

/// Whether the live file is back at the state `tid` held for it before the outside edit.
fn fixed_ending(s: &Store, tid: &str, rel: &str) -> Result<Ending, Failure> {
    let key = format!("file:{rel}");
    let before = crate::board::reviews::before_of(s, tid)?;
    let Some((_, old)) = before.into_iter().find(|(k, _)| *k == key) else { return Ok(Ending::Integrated) };
    let live = match fs::read(s.root.join(rel)) {
        Ok(live) => live,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ending::Integrated),
        Err(e) => return Err(e.into()),
    };
    Ok(if old.as_array().is_some_and(|bytes| jsonx::bytes(bytes) == live) { Ending::Reverted } else { Ending::Integrated })
}
