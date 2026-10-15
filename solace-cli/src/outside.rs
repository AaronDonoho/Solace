//! Changes made outside Solace (D-58). Solace remembers what it last wrote to
//! each Gleam file (`.solace/written`) and never writes over a file that
//! differs from that; such a file is read into the store instead, and the
//! change goes to the running task or to Unexpected Changes for review.
//! `.solace/approved` holds each symbol's and header's hash as last approved,
//! so code that came from Solace (a pull, a branch switch) is read in quietly.
use super::*;
use std::collections::BTreeMap;

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
/// and `! hash path` for a version it couldn't read in and already reported.
pub struct Written {
    root: PathBuf,
    files: BTreeMap<String, (String, u64, u128)>,
    bad: BTreeMap<String, String>,
    /// False for a store from before D-58 that has no record yet.
    pub known: bool,
    dirty: bool,
}

impl Written {
    pub fn load(root: &Path) -> Written {
        let text = fs::read_to_string(root.join(".solace/written"));
        let (mut files, mut bad) = (BTreeMap::new(), BTreeMap::new());
        for line in text.as_deref().unwrap_or("").lines() {
            if let Some((h, path)) = line.strip_prefix("! ").and_then(|l| l.split_once(' ')) {
                bad.insert(path.to_string(), h.to_string());
                continue;
            }
            let mut p = line.splitn(4, ' ');
            if let (Some(h), Some(size), Some(time), Some(path)) = (p.next(), p.next(), p.next(), p.next()) {
                files.insert(path.to_string(), (h.to_string(), size.parse().unwrap_or(0), time.parse().unwrap_or(0)));
            }
        }
        Written { root: root.into(), files, bad, known: text.is_ok(), dirty: false }
    }

    pub fn save(&mut self) -> Result<(), Failure> {
        if self.dirty {
            let mut text: String = self.files.iter().map(|(p, (h, size, time))| format!("{h} {size} {time} {p}\n")).collect();
            text.extend(self.bad.iter().map(|(p, h)| format!("! {h} {p}\n")));
            write_atomic(&self.root.join(".solace/written"), text)?;
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
        let gone: Vec<String> =
            self.files.keys().filter(|rel| !keep.contains(*rel) && !self.bad.contains_key(*rel) && !extract.contains(*rel)).cloned().collect();
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

/// Every Gleam file under `src/` and `test/`, except a probe left behind.
fn graph_files(root: &Path) -> Result<Vec<String>, Failure> {
    let mut out = vec![];
    // A project of plain files holds no symbols (D-60).
    if root.join("gleam.toml").exists() {
        out = source_files(root)?;
        out.retain(|rel| *rel != file_of("test", PROBE));
    }
    Ok(out)
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
    write_atomic(&root.join(".solace/approved"), text)
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
    let snap = board::snapshot(s)?;
    let map = snap.as_object().unwrap().iter().filter_map(|(k, v)| approval(k, v)).collect();
    save_approved(&s.root, &map)?;
    fs::write(s.root.join(".solace/.gitignore"), "# Only the approved list is shared (D-58).\n*\n!.gitignore\n!approved\n")?;
    Ok(())
}

/// A completed task's changes become approved: each entity's name before the
/// task leaves the list, and its current state joins it.
// ponytail: takes each entity as it is now, even if a later open task also changed it.
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
    let (dir, file) = rel.split_once('/').unwrap();
    let module = file.strip_suffix(".gleam").unwrap();
    if !valid_module_path(module) {
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
    let parsed = gleam::split(text);
    if let Some(twice) = gleam::repeated(&parsed.items).first() {
        return Ok(Err(format!("{rel} defines `{twice}` twice; Solace left it as it is and reads it again once fixed")));
    }
    s.db.execute("INSERT OR REPLACE INTO modules VALUES (?1, ?2, ?3)", params![module, dir, parsed.header])?;
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
    let mut w = Written::load(&s.root);
    if !w.known {
        // A store from before D-58: what it holds is what Solace last wrote.
        let ix = s.index()?;
        for m in &ix.modules {
            let (content, _) = projection(&ix, m);
            w.files.insert(file_of(&m.dir, &m.path), (digest(content.as_bytes()), 0, 0));
        }
        w.known = true;
        w.dirty = true;
    }
    if !s.root.join(".solace/approved").exists() {
        approve_all(s)?;
    }
    let disk = graph_files(&s.root)?;
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
        return Ok(String::new());
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
        return Ok(unreadable_note);
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
        return Ok(String::new());
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
        return Ok(format!("Note: {text}"));
    }
    let tid = match open
        .iter()
        .map(|(_, t)| t)
        .find(|t| t["unexpected"] == true && board::task(&s.root, t["id"].as_str().unwrap_or("")).is_ok())
    {
        Some(t) => t["id"].as_str().unwrap().to_string(),
        None => {
            let rank = all.iter().filter_map(|(_, t)| t["rank"].as_f64()).fold(0., f64::min) - 1.;
            let mut t = board::new_task(
                TITLE,
                "Code changed outside Solace while no agent was working. Complete this task to keep the changes, or cancel it to restore the code Solace had. No agent starts work until it is completed or canceled.",
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
    Ok(format!("Note: code changed outside Solace with no agent working; review it on the {TITLE} task. {changed_note}{unreadable_note}"))
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

    /// D-58.7: code whose hashes are on the approved list (a pull of work
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
        assert!(
            changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))),
            "{}",
            json!(changes)
        );
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

    /// Moving a symbol without editing it changes its order only; the text stays.
    #[test]
    fn a_reordered_symbol_keeps_its_text_and_takes_its_new_place() {
        let s = app();
        let ord = |name: &str| s.db.query_row("SELECT ord FROM symbols WHERE module = 'app' AND name = ?1 AND live = 1", [name], |r| r.get::<_, i64>(0)).unwrap();
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
        assert!(graph_files(&plain).unwrap().is_empty());
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
}
