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
use crate::outside_share::share_store;
use crate::outside_share::hidden_note;
use crate::outside_copies::{remove_old_copies, OLD_COPIES_REMOVED};
use crate::store_core::checkpoint;
#[cfg(test)]
#[path = "outside_tests.rs"]
mod tests;
pub const TITLE: &str = "Unexpected Changes";

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
        let entry = (crate::digest(bytes), size, time);
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
        stat(&path) == Some((*size, *time)) || fs::read(&path).is_ok_and(|b| &crate::digest(&b) == h)
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
        self.bad.contains_key(rel) && self.bad_at.get(rel).is_some_and(|h| *h != crate::digest(content))
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
        let h = crate::digest(bytes);
        if self.bad.get(rel) != Some(&h) {
            self.bad.insert(rel.to_string(), h);
            self.dirty = true;
        }
        if !self.bad_at.contains_key(rel) {
            self.bad_at.insert(rel.to_string(), crate::digest(projected));
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
        Some((format!("{}.{}", row[1].as_str()?, row[2].as_str()?), crate::digest(row[4].as_str()?.as_bytes())))
    } else if key.starts_with("module:") {
        Some((header_key(row[0].as_str()?), crate::digest(row[2].as_str()?.as_bytes())))
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
    let note = hidden_note(w.hidden.as_deref(), &s.root);
    w.hidden = note.hidden;
    w.dirty |= note.dirty;
    let hidden = note.text;
    prepare(s, &mut w)?;
    let disk = graph_files(s)?;
    let ix = s.index()?;
    settle_replaced(s, &mut w)?;
    remove_old_copies_once(s, &mut w, &disk)?;
    let command = matches!(mode, Mode::Command);
    let mut found = scan_disk(s, &ix, &mut w, &disk, mode)?;
    if found.changed.is_empty() && found.parked.is_empty() {
        return finish_no_changes(s, &mut w, &hidden, &mut found.fixed, &found.gone_bad);
    }
    if command {
        return record_command(s, &ix, &mut w, &hidden, found);
    }
    record_notice(s, &ix, &mut w, &hidden, found)
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
    fixed: Vec<(String, Option<Vec<u8>>)>,
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
                    out.fixed.push((rel.clone(), bytes.clone()));
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
fn refresh_fixed(s: &Store, fixed: &mut Vec<(String, Option<Vec<u8>>)>, gone_bad: &[String]) -> Result<Vec<String>, Failure> {
    fixed.sort_by(|a, b| a.0.cmp(&b.0));
    fixed.dedup_by(|a, b| a.0 == b.0);
    let mut endings: HashMap<String, Ending> = HashMap::new();
    for (rel, live) in fixed.iter() {
        for tid in crate::board::reviews::file_change_tasks(s, std::slice::from_ref(rel))? {
            let ending = fixed_ending(s, &tid, rel, live.as_deref())?;
            endings.entry(tid).and_modify(|e| *e = combine_endings(*e, ending)).or_insert(ending);
        }
    }
    for rel in gone_bad {
        for tid in crate::board::reviews::file_change_tasks(s, std::slice::from_ref(rel))? {
            endings.entry(tid).and_modify(|e| *e = combine_endings(*e, Ending::Reverted)).or_insert(Ending::Reverted);
        }
    }
    let mut names: Vec<String> = fixed.iter().map(|(rel, _)| rel.clone()).collect();
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
fn finish_no_changes(
    s: &Store,
    w: &mut Written,
    hidden: &str,
    fixed: &mut Vec<(String, Option<Vec<u8>>)>,
    gone_bad: &[String],
) -> Result<String, Failure> {
    let dropped = settle_changes(s, w, fixed, gone_bad)?;
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
fn fixed_ending(s: &Store, tid: &str, rel: &str, live: Option<&[u8]>) -> Result<Ending, Failure> {
    let key = format!("file:{rel}");
    let before = crate::board::reviews::before_of(s, tid)?;
    let Some((_, old)) = before.into_iter().find(|(k, _)| *k == key) else { return Ok(Ending::Integrated) };
    let Some(live) = live else { return Ok(Ending::Integrated) };
    Ok(if old.as_array().is_some_and(|bytes| jsonx::bytes(bytes) == live) { Ending::Reverted } else { Ending::Integrated })
}

/// Makes the written record agree with a store from before it kept one.
fn prepare(s: &Store, w: &mut Written) -> Result<(), Failure> {
    if !w.known {
        // A store from before D-58: what it holds is what Solace last wrote.
        let ix = s.index()?;
        for m in &ix.modules {
            let (content, _) = projection(&ix, m);
            w.files.insert(m.rel(), (crate::digest(content.as_bytes()), 0, 0));
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
    Ok(())
}

/// Rewrites the notes of tasks holding an outside edit whose mark is gone.
fn settle_replaced(s: &Store, w: &mut Written) -> Result<(), Failure> {
    let replaced = w.take_replaced();
    if replaced.is_empty() {
        return Ok(());
    }
    // The outside edit stays visible until the task finishes, but its mark is gone.
    for tid in crate::board::reviews::file_change_tasks(s, &replaced)? {
        refresh_note(s, &tid, Ending::Integrated)?;
    }
    Ok(())
}

fn remove_old_copies_once(s: &Store, w: &mut Written, disk: &[String]) -> Result<(), Failure> {
    if w.old_copies_removed {
        return Ok(());
    }
    remove_old_copies(&s.root, disk)?;
    w.old_copies_removed = true;
    w.dirty = true;
    Ok(())
}

/// What one look at the disk found and what the written record already knows.
struct Scan {
    changed: Vec<(String, Option<Vec<u8>>)>,
    parked: Vec<(String, Vec<u8>)>,
    fixed: Vec<(String, Option<Vec<u8>>)>,
    gone_bad: Vec<String>,
}

/// Compares the graph files with the written record: new bytes, repairs, and removals.
fn scan_disk(s: &Store, ix: &Index, w: &mut Written, disk: &[String], mode: Mode) -> Result<Scan, Failure> {
    let mut found = Scan { changed: vec![], parked: vec![], fixed: vec![], gone_bad: vec![] };
    for rel in disk {
        let path = s.root.join(rel);
        let now = stat(&path);
        match w.files.get(rel) {
            Some((_, size, time)) if now == Some((*size, *time)) => continue,
            _ => {}
        }
        let bytes = fs::read(&path)?;
        if w.files.get(rel).is_some_and(|(h, ..)| *h == crate::digest(&bytes)) {
            let was_bad = w.bad.contains_key(rel);
            w.record(rel, &bytes);
            if was_bad {
                found.fixed.push((rel.clone(), Some(bytes)));
            }
        } else if w.bad.get(rel) != Some(&crate::digest(&bytes)) {
            found.changed.push((rel.clone(), Some(bytes)));
        } else if matches!(mode, Mode::Notice) && !crate::board::reviews::holds_file_change(s, rel) {
            found.parked.push((rel.clone(), bytes));
        }
    }
    for rel in w.files.keys() {
        if !disk.contains(rel) && !w.bad.contains_key(rel) {
            found.changed.push((rel.clone(), None));
        }
    }
    let (gone_bad, removed) = dropped_gone_bad(w, disk, &found.changed, ix);
    found.gone_bad = gone_bad;
    found.changed.extend(removed);
    Ok(found)
}

/// Reads a command's changes into the store and returns the unreadable note.
fn record_command(s: &Store, ix: &Index, w: &mut Written, hidden: &str, found: Scan) -> Result<String, Failure> {
    let Scan { changed, mut fixed, gone_bad, .. } = found;
    let absorbed = absorb_changes(s, ix, w, &changed, &mut fixed, &gone_bad)?;
    retire_empty_unexpected(s, &absorbed.dropped)?;
    Ok(format!("{hidden}{}", unreadable_note(&absorbed.outcome.unreadable)))
}

/// The tasks open on this project and the one work run, if any, that is going.
struct OpenTasks {
    flow: Value,
    top: PathBuf,
    here: String,
    all: Vec<(PathBuf, Value)>,
    open: Vec<(PathBuf, Value)>,
    open_ids: HashSet<String>,
    running: Option<(PathBuf, String, String)>,
}

fn open_tasks(s: &Store) -> Result<OpenTasks, Failure> {
    let flow = board::flow(&s.root)?;
    let top = scope::top(&s.root);
    let here = scope::dir(&top, &s.root);
    let all = scope::tasks_in(&top, &here)?;
    let open: Vec<(PathBuf, Value)> = all.iter().filter(|(_, t)| board::column(&flow, t)["final"] != true).cloned().collect();
    let running = open.iter().find_map(|(home, t)| {
        let r = t["runs"].as_array().unwrap().iter().find(|r| r["kind"] == "work" && r["state"] == "running")?;
        Some((home.clone(), t["id"].as_str()?.to_string(), r["id"].as_str()?.to_string()))
    });
    let open_ids = open.iter().filter_map(|(_, t)| t["id"].as_str().map(String::from)).collect();
    Ok(OpenTasks { flow, top, here, all, open, open_ids, running })
}

/// The changes one absorb found, classified into what belongs to a task.
struct Review {
    before: Value,
    after: Value,
    unreadable: Vec<String>,
    unreadable_files: Vec<(String, Option<Vec<u8>>)>,
    changed_keys: Vec<String>,
    unexpected: Vec<String>,
    layout: Vec<String>,
    approved_layout: Vec<(String, Vec<String>)>,
    removed_layout: Vec<(String, Vec<String>)>,
    unexpected_tid: Option<String>,
    dropped: Vec<String>,
}

/// Sorts the changed keys into layouts and unexpected edits.
fn classify(s: &Store, ix: &Index, open: &OpenTasks, start: ReviewStart, absorbed: Absorbed) -> Result<Review, Failure> {
    let ReviewStart { mut before, touched, list } = start;
    let Absorbed { outcome, dropped } = absorbed;
    let mut after = board::snapshot(s)?;
    if open.running.is_none() && !outcome.unreadable_files.is_empty() {
        for (rel, bytes) in &outcome.unreadable_files {
            let key = format!("file:{rel}");
            let old = ix.modules.iter().find(|m| m.rel() == *rel).map(|m| projection(ix, m).0.into_bytes());
            before[key.clone()] = old.map_or(Value::Null, |b| json!(b));
            after[key] = bytes.as_ref().map_or(Value::Null, |b| json!(b));
        }
    }
    let keys: HashSet<&String> = before.as_object().unwrap().keys().chain(after.as_object().unwrap().keys()).collect();
    let mut changed_keys: Vec<String> = vec![];
    let unexpected_tid = open
        .open
        .iter()
        .map(|(_, t)| t)
        .find(|t| t["unexpected"] == true && board::task(&s.root, t["id"].as_str().unwrap_or("")).is_ok())
        .and_then(|t| t["id"].as_str().map(String::from));
    let target = open.running.as_ref().map(|(_, tid, _)| tid.clone()).or_else(|| unexpected_tid.clone());
    let layout_tasks = |key: &str| -> Result<Vec<String>, Failure> {
        Ok(crate::board::reviews::layout_tasks(s, key)?.into_iter().filter(|tid| open.open_ids.contains(tid)).collect())
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
    Ok(Review {
        before,
        after,
        unreadable: outcome.unreadable,
        unreadable_files: outcome.unreadable_files,
        changed_keys,
        unexpected,
        layout,
        approved_layout,
        removed_layout,
        unexpected_tid,
        dropped,
    })
}

/// Drops reverted changes, records approved layouts, opens an Unexpected task, and records endings.
fn settle_review(s: &Store, open: &mut OpenTasks, review: &mut Review) -> Result<(), Failure> {
    if open.running.is_none()
        && let Some(tid) = &review.unexpected_tid
        && !review.unexpected.is_empty()
    {
        let gone = crate::board::reviews::drop_unchanged(s, tid, &review.unexpected, &review.after)?;
        if !gone.is_empty() {
            review.unexpected.retain(|k| !gone.contains(k));
            review.changed_keys.retain(|k| !gone.contains(k));
            refresh_note(s, tid, Ending::Reverted)?;
            review.dropped.push(tid.clone());
        }
    }
    for (key, holders) in &review.approved_layout {
        crate::board::reviews::record_approved_layout(s, holders, &review.before, &review.after, key)?;
    }
    if open.running.is_none() && review.unexpected_tid.is_none() && (!review.unexpected.is_empty() || !review.unreadable.is_empty()) {
        let tid = create_unexpected_task(&open.top, &open.here, &open.flow, &open.all)?;
        open.open_ids.insert(tid.clone());
        review.unexpected_tid = Some(tid);
    }
    let write_target = open.running.as_ref().map(|(_, tid, _)| tid.as_str()).or(review.unexpected_tid.as_deref());
    crate::board::reviews::record_endings(s, &review.before, &review.after, &review.changed_keys, &open.open_ids, write_target)?;
    crate::board::reviews::drop_layout(s, &review.removed_layout)?;
    review.dropped.extend(review.removed_layout.iter().flat_map(|(_, tids)| tids.iter().cloned()));
    Ok(())
}

/// Records layout-only changes on the task that should review them.
fn finish_layout(s: &Store, tid: &str, review: &Review) -> Result<(), Failure> {
    if review.layout.is_empty() {
        return Ok(());
    }
    crate::board::reviews::record_layout(s, tid, &review.before, &review.after, &review.layout)
}

fn changed_names(unexpected: &[String], before: &Value, after: &Value) -> Vec<String> {
    let mut names: Vec<String> = unexpected
        .iter()
        .map(|k| match approval(k, if after[k].is_null() { &before[k] } else { &after[k] }) {
            Some((name, _)) => name,
            None => k.clone(),
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Records the review's unexpected changes on its running task or its Unexpected task, and replies.
fn record_review(s: &Store, hidden: &str, open: &OpenTasks, review: &Review) -> Result<String, Failure> {
    if let Some((_, tid, rid)) = &open.running {
        return record_running_review(s, hidden, open, review, tid, rid);
    }
    record_unexpected_review(s, hidden, open, review)
}

/// Reviews changes made with no command: joins a running task or files Unexpected Changes.
fn record_notice(s: &Store, ix: &Index, w: &mut Written, hidden: &str, found: Scan) -> Result<String, Failure> {
    let mut open = open_tasks(s)?;
    let mut found = found;
    if open.running.is_none() {
        found.changed.extend(std::mem::take(&mut found.parked).into_iter().map(|(rel, bytes)| (rel, Some(bytes))));
    }
    if found.changed.is_empty() {
        return finish_no_changes(s, w, hidden, &mut found.fixed, &found.gone_bad);
    }
    let start = begin_review(s, &open)?;
    let Scan { changed, mut fixed, gone_bad, .. } = found;
    let absorbed = absorb_changes(s, ix, w, &changed, &mut fixed, &gone_bad)?;
    let mut review = classify(s, ix, &open, start, absorbed)?;
    settle_review(s, &mut open, &mut review)?;
    if review.unexpected.is_empty() && review.unreadable.is_empty() {
        if let Some(tid) = layout_task(&open, &review) {
            finish_layout(s, tid, &review)?;
        }
        retire_empty_unexpected(s, &review.dropped)?;
        return Ok(hidden.to_string());
    }
    record_review(s, hidden, &open, &review)
}

struct Absorbed {
    outcome: ReadOutcome,
    dropped: Vec<String>,
}

fn absorb_changes(
    s: &Store,
    ix: &crate::Index,
    w: &mut Written,
    changed: &[(String, Option<Vec<u8>>)],
    fixed: &mut Vec<(String, Option<Vec<u8>>)>,
    gone_bad: &[String],
) -> Result<Absorbed, Failure> {
    let mut outcome = read_changed(s, ix, w, changed)?;
    fixed.append(&mut outcome.fixed);
    let dropped = settle_changes(s, w, fixed, gone_bad)?;
    Ok(Absorbed { outcome, dropped })
}

fn settle_changes(
    s: &Store,
    w: &mut Written,
    fixed: &mut Vec<(String, Option<Vec<u8>>)>,
    gone_bad: &[String],
) -> Result<Vec<String>, Failure> {
    w.save()?;
    refresh_fixed(s, fixed, gone_bad)
}

struct ReviewStart {
    before: Value,
    touched: HashSet<String>,
    list: BTreeMap<String, String>,
}

/// The state a review compares against, taken before changes are read in.
fn begin_review(s: &Store, open: &OpenTasks) -> Result<ReviewStart, Failure> {
    // What changed, and of that, what wasn't approved elsewhere.
    // Tasks from other projects whose scope includes this one count too (D-61).
    let mut touched: HashSet<String> = HashSet::new();
    for (_, t) in &open.open {
        let changes = board::changes_of(s, t)?;
        touched.extend(changes.iter().filter(|c| c["project"] == open.here).filter_map(|c| c["entity"].as_str().map(String::from)));
    }
    let list = approved(&s.root);
    let before = board::snapshot(s)?;
    Ok(ReviewStart { before, touched, list })
}

fn layout_task<'a>(open: &'a OpenTasks, review: &'a Review) -> Option<&'a str> {
    open.running.as_ref().map(|(_, tid, _)| tid.as_str()).or(review.unexpected_tid.as_deref())
}

struct ReviewChanges {
    before: Value,
    after: Value,
    names: Vec<String>,
    unreadable: String,
}

fn review_changes(review: &Review) -> ReviewChanges {
    let pick = |v: &Value| json!(review.unexpected.iter().map(|k| (k.clone(), v[k].clone())).collect::<serde_json::Map<_, _>>());
    let (before, after) = (pick(&review.before), pick(&review.after));
    let names = changed_names(&review.unexpected, &before, &after);
    let unreadable = unreadable_note(&review.unreadable);
    ReviewChanges { before, after, names, unreadable }
}

fn record_running_review(s: &Store, hidden: &str, open: &OpenTasks, review: &Review, tid: &str, rid: &str) -> Result<String, Failure> {
    let changes = review_changes(review);
    // Tasks change through the task server (D-77).
    if !review.unexpected.is_empty() {
        board::record(s, tid, rid, &changes.before, &changes.after, None)?;
    }
    finish_layout(s, tid, review)?;
    let text = format!(
        "{}{}",
        if changes.names.is_empty() {
            String::new()
        } else {
            format!("Changed outside Solace during this run and added to this task: {}.\n", changes.names.join(", "))
        },
        changes.unreadable
    );
    if !text.is_empty() {
        tasks::deliver(&open.top, json!({"kind":"note","task":tid,"run":rid,"author":"solace","text":text}))?;
    }
    retire_empty_unexpected(s, &review.dropped)?;
    Ok(if text.is_empty() { hidden.to_string() } else { format!("{hidden}Note: {text}") })
}

fn record_unexpected_review(s: &Store, hidden: &str, open: &OpenTasks, review: &Review) -> Result<String, Failure> {
    let changes = review_changes(review);
    let tid = review.unexpected_tid.as_deref().expect("an Unexpected Changes task is created before its changes are recorded");
    if !review.unexpected.is_empty() {
        board::record(s, tid, "outside", &changes.before, &changes.after, None)?;
    }
    for (rel, bytes) in &review.unreadable_files {
        if let Some(bytes) = bytes {
            crate::board::reviews::save_after(&s.root, tid, rel, bytes)?;
        }
    }
    finish_layout(s, tid, review)?;
    let compiled = match s.check() {
        Ok(out) => out,
        Err(Failure(out, _)) => out,
    };
    let compiled: String = compiled.chars().take(4000).collect();
    let changed_note =
        if changes.names.is_empty() { String::new() } else { format!("Changed outside Solace: {}.\n", changes.names.join(", ")) };
    let note = note_text(s, tid, Ending::Integrated)?;
    tasks::deliver(
        &open.top,
        json!({"kind":"outside_note","task":tid,"author":"solace","text":format!("{}\nCheck: {compiled}", note),
            "flag":"code changed outside Solace"}),
    )?;
    retire_empty_unexpected(s, &review.dropped)?;
    Ok(format!(
        "{hidden}Note: code changed outside Solace with no agent working; review it on the {TITLE} task. {changed_note}{}",
        changes.unreadable
    ))
}
