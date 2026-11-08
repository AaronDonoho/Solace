use crate::outside::*;
use rusqlite::params;
use serde_json::Value;
use serde_json::json;

const APP: &str = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n";

/// The `app` module, `APP`, on a board.
fn app() -> crate::testing::Project {
    let p = crate::testing::project(&[("src/app.gleam", APP)]);
    board::open(&p.root).unwrap();
    p
}

fn file(s: &crate::Store) -> String {
    fs::read_to_string(s.root.join("src/app.gleam")).unwrap()
}

fn text(s: &crate::Store, name: &str) -> Option<String> {
    s.db.query_row("SELECT text FROM symbols WHERE module = 'app' AND name = ?1 AND live = 1", [name], |r| r.get(0)).optional().unwrap()
}

/// D-58.1/4/5: an edit made outside Solace is read in, never written
/// over, and waits on Unexpected Changes, which blocks work until a
/// human cancels it, restoring Solace's code.
#[test]
fn outside_edits_are_read_in_and_reviewed() {
    let s = app();
    assert!(s.root.join(".solace/approved").exists() && s.root.join(".solace/written").exists());
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let edited = APP.replace("  2", "  22") + "\npub fn three() -> Int {\n  3\n}\n";
    fs::write(s.root.join("src/app.gleam"), &edited).unwrap();
    // A write in the middle of a command leaves the changed file alone.
    s.materialize(&s.index().unwrap()).unwrap();
    assert_eq!(file(&s), edited);
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains(crate::outside::TITLE) && note.contains("app.three, app.two"), "{note}");
    assert_eq!(text(&s, "two").as_deref(), Some("pub fn two() -> Int {\n  22\n}"));
    assert!(text(&s, "three").is_some());
    s.check().unwrap();
    assert_eq!(file(&s), edited);
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap();
    assert_eq!(t["column"], "needs_review");
    assert!(t["notes"][0]["text"].as_str().unwrap().contains("Check: No errors"), "{}", t["notes"]);
    // Nothing new: no second note.
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let other = board::api(&s.root, "POST", "/tasks", json!({"title":"Other"})).unwrap();
    let oid = other["id"].as_str().unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"plan"})).unwrap();
    let refused = board::api(&s.root, "POST", &format!("/tasks/{oid}/actions"), json!({"action":"assign","model":"x"})).unwrap_err();
    assert!(refused.0.starts_with("E7012"), "{}", refused.0);
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert_eq!(file(&s), APP);
    assert!(text(&s, "three").is_none());
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
}

/// D-58.6: code whose hashes are on the approved list (a pull of work
/// done through Solace) is read in without a task; completing a task
/// puts its changes on the list.
#[test]
fn approved_code_is_read_in_quietly() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let pulled = APP.replace("  1", "  11");
    let mut list = crate::outside::approved(&s.root);
    list.insert("app.one".into(), crate::digest(b"pub fn one() -> Int {\n  11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/app.gleam"), &pulled).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  11\n}"));
    assert!(board::tasks(&s.root).unwrap().is_empty());
    // A hand edit isn't approved; completing its task approves it.
    fs::write(s.root.join("src/app.gleam"), pulled.replace("  2", "  5")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("app.two"));
    let t = board::tasks(&s.root).unwrap().remove(0);
    let tid = t["id"].as_str().unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
    assert_eq!(crate::outside::approved(&s.root).get("app.two"), Some(&crate::digest(b"pub fn two() -> Int {\n  5\n}")));
}

/// D-229: a file Solace can't read into symbols is left as it is and
/// reported once; once fixed it is read in.
#[test]
fn unreadable_files_are_left_alone() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("defines `two` twice"));
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    s.materialize(&s.index().unwrap()).unwrap();
    assert_eq!(file(&s), twice);
    fs::write(s.root.join("src/app.gleam"), APP.replace("  2", "  3")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("app.two"));
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert_eq!(crate::outside::absorb(&s, Mode::Command).unwrap(), "");
    assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  4\n}"));
    assert!(board::tasks(&s.root).unwrap().is_empty());
}

/// A file read in with a bad module name is refused with its reason; one
/// deleted outside retires its symbols and drops its module.
#[test]
fn read_in_refuses_a_bad_name_and_retires_a_deleted_file() {
    let s = app();
    let msg = crate::outside::read_in(&s, "src/Bad-Name.gleam", Some(APP)).unwrap().unwrap_err();
    assert!(msg.contains("isn't a Gleam module name"), "{msg}");
    let modules = |s: &crate::Store| s.db.query_row("SELECT count(*) FROM modules WHERE path = 'app'", [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(modules(&s), 1);
    crate::outside::read_in(&s, "src/app.gleam", None).unwrap().unwrap();
    assert!(text(&s, "one").is_none() && text(&s, "two").is_none());
    assert_eq!(modules(&s), 0);
}

/// D-158: reading a module in keeps the file a Rust module is written to.
#[test]
fn read_in_keeps_a_modules_file() {
    let s = app();
    s.db.execute("UPDATE modules SET file = 'src/app.rs' WHERE path = 'app'", []).unwrap();
    let edited = APP.replace("  1", "  11");
    crate::outside::read_in(&s, "src/app.gleam", Some(&edited)).unwrap().unwrap();
    let file: Option<String> = s.db.query_row("SELECT file FROM modules WHERE path = 'app'", [], |r| r.get(0)).unwrap();
    assert_eq!(file.as_deref(), Some("src/app.rs"));
}

/// Moving a symbol without editing it changes its order only; the text stays.
#[test]
fn a_reordered_symbol_keeps_its_text_and_takes_its_new_place() {
    let s = app();
    let ord = |name: &str| {
        s.db.query_row("SELECT ord FROM symbols WHERE module = 'app' AND name = ?1 AND live = 1", [name], |r| r.get::<_, i64>(0)).unwrap()
    };
    let before = (ord("one"), ord("two"));
    assert!(before.0 < before.1);
    let swapped = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  1\n}\n";
    crate::outside::read_in(&s, "src/app.gleam", Some(swapped)).unwrap().unwrap();
    assert!(ord("two") < ord("one"));
    assert_eq!(text(&s, "one").as_deref(), Some("pub fn one() -> Int {\n  1\n}"));
}

/// `unchanged` is true where nothing is lost by writing: no record, or no file.
#[test]
fn unchanged_follows_the_record() {
    let s = app();
    let mut w = crate::outside::Written::load(&s.root);
    assert!(w.known && w.unchanged("src/app.gleam") && w.unchanged("src/absent.gleam"));
    fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  9")).unwrap();
    assert!(!w.unchanged("src/app.gleam"), "edited since Solace wrote it");
    fs::write(s.root.join("src/app.gleam"), APP).unwrap();
    assert!(w.unchanged("src/app.gleam"), "the same bytes again, by hash");
    w.forget("src/app.gleam");
    assert!(!w.unchanged("src/app.gleam"), "a file with no entry is not Solace's");
    let fresh = crate::outside::Written::load(&crate::testing::Scratch::new());
    assert!(!fresh.known && fresh.unchanged("src/app.gleam"));
}

/// A project of plain files holds no symbols, and an approval needs the
/// fields of the entry it describes.
#[test]
fn plain_files_have_no_graph_and_odd_entries_no_approval() {
    let plain = crate::testing::Scratch::new().with(&[("notes/readme.txt", "hi")]);
    crate::import(&plain).unwrap();
    assert!(crate::outside::graph_files(&crate::Store::at(&plain).unwrap()).unwrap().is_empty());
    assert_eq!(crate::outside::approval("task:1", &json!(["a"])), None);
    assert_eq!(crate::outside::approval("symbol:1", &json!([0, "app", 5, 0, "text"])), None);
    assert_eq!(crate::outside::approval("module:1", &json!(["app.gleam", 0, null])), None);
    assert!(crate::outside::approval("symbol:1", &json!([0, "app", "one", 0, "text"])).is_some_and(|(k, _)| k == "app.one"));
}

/// A store from before D-58 has no record: what it holds counts as
/// written, so a file edited since is read in rather than overwritten.
#[test]
fn old_stores_start_from_what_they_hold() {
    let s = app();
    fs::remove_file(s.root.join(".solace/written")).unwrap();
    fs::remove_file(s.root.join(".solace/approved")).unwrap();
    fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  7")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("app.one"));
    assert_eq!(crate::outside::approved(&s.root).get("app.one"), Some(&crate::digest(b"pub fn one() -> Int {\n  1\n}")));
    assert_eq!(file(&s), APP.replace("  1", "  7"));
}

/// Cleanup runs once; an old-id folder made afterward is left alone.
#[test]
fn old_copies_are_removed_once() {
    let s = app();
    let first = s.root.join(".solace/outside").join(crate::unique_id());
    fs::create_dir_all(&first).unwrap();
    crate::outside::absorb(&s, Mode::Command).unwrap();
    assert!(!first.exists());
    let later = s.root.join(".solace/outside").join(crate::unique_id());
    fs::create_dir_all(&later).unwrap();
    crate::outside::absorb(&s, Mode::Command).unwrap();
    assert!(later.exists());
}

const LIB: &str = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";

fn rust_app() -> (crate::testing::Scratch, crate::Store) {
    let (root, s) = crate::testing::rust_project(LIB);
    board::open(&s.root).unwrap();
    (root, s)
}

fn rust_file(s: &crate::Store) -> String {
    fs::read_to_string(s.root.join("src/lib.rs")).unwrap()
}

fn rust_text(s: &crate::Store, name: &str) -> Option<String> {
    s.db.query_row("SELECT text FROM symbols WHERE module = 'crate' AND name = ?1 AND live = 1", [name], |r| r.get(0)).optional().unwrap()
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains(crate::outside::TITLE) && note.contains("crate.three, crate.two"), "{note}");
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
    let (_root, s) = crate::testing::rust_project(concat!(
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap();
    assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    11\n}"));
    assert_eq!(rust_file(&s), edited);
    assert_eq!(crate::outside::approved(&s.root).get("crate.one"), Some(&crate::digest(b"pub fn one() -> i32 {\n    11\n}")));
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let pulled = LIB.replace("    1", "    11");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), &pulled).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert_eq!(note, "Note: Changed outside Solace during this run and added to this task: crate.two.\n");
    let t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
    let changes = board::changes_of(&s, &t).unwrap();
    assert!(changes.iter().any(|c| c["entity"].as_str().is_some_and(|e| e.starts_with("symbol:"))), "{}", json!(changes));
    assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
}

/// D-58.1 for Rust: a removed file retires its symbols.
#[test]
fn a_removed_rust_file_retires_its_symbols() {
    let (_root, s) = crate::testing::rust_project(LIB);
    fs::remove_file(s.root.join("src/lib.rs")).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Command).unwrap(), "");
    assert!(rust_text(&s, "one").is_none());
    assert!(s.index().unwrap().modules.is_empty());
}

/// D-229 for Rust: an old refusal is retried once, not hidden by its hash.
#[test]
fn an_old_bad_mark_does_not_hide_a_rust_edit() {
    let (_root, s) = crate::testing::rust_project(LIB);
    let edited = LIB.replace("    2", "    22");
    fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
    fs::write(s.root.join(".solace/written"), format!("! {} src/lib.rs\n", crate::digest(edited.as_bytes()))).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Command).unwrap(), "");
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("crate.one"), "{note}");
    assert_eq!(rust_text(&s, "one").as_deref(), Some("pub fn one() -> i32 {\n    \"one\"\n}"));
}

/// D-229 for Rust: a file Solace can't split is left as it is and reported once.
#[test]
fn a_rust_file_that_cannot_be_split_is_left_alone() {
    let (_root, s) = crate::testing::rust_project(LIB);
    let bad = "pub fn one() -> i32 {\n    1\n";
    fs::write(s.root.join("src/lib.rs"), bad).unwrap();
    let note = crate::outside::absorb(&s, Mode::Command).unwrap();
    assert!(note.contains("Not read in:"), "{note}");
    assert_eq!(rust_file(&s), bad);
    assert_eq!(crate::outside::absorb(&s, Mode::Command).unwrap(), "");
}

fn rust_row(s: &crate::Store, name: &str) -> (String, Option<String>, i64) {
    s.db.query_row("SELECT id, parent, ord FROM symbols WHERE module = 'crate' AND name = ?1 AND live = 1", [name], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .unwrap()
}

fn rust_projection(s: &crate::Store) -> String {
    let ix = s.index().unwrap();
    let m = ix.modules.iter().find(|m| m.path == "crate").unwrap();
    crate::store::projection(&ix, m).0
}

fn rust_cancel(s: &crate::Store) {
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
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
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let (a, b) = (rust_row(&s, "S::a"), rust_row(&s, "S::b"));
    let edited = concat!(
        "pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n",
        "    pub fn b(&self) -> i32 {\n        2\n    }\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
    let t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
    let tid = t["id"].as_str().unwrap();
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert_eq!(rust_file(&s), LIB);
    assert_eq!(rust_projection(&s), LIB);
    assert_eq!((rust_row(&s, "one"), rust_row(&s, "two")), (one, two));
}

fn rust_work_task(s: &crate::Store, title: &str, rid: &str) -> Value {
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
    a["runs"][0]["state"] = json!("ended");
    tasks::put(&s, &a).unwrap();
    let _b = rust_work_task(&s, "B", "rb");
    let edited = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
    a["runs"][0]["state"] = json!("ended");
    tasks::put(&s, &a).unwrap();
    let b = rust_work_task(&s, "B", "rb");
    let edited = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let rows = (rust_row(&s, "one"), rust_row(&s, "two"), rust_row(&s, "three"));
    let edited = original.replace("    1", "    11");
    fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let notes = t["notes"].as_array().unwrap().len();
    let swapped = edited.replace(
        "pub fn two() -> i32 {\n    2\n}\n\npub fn three() -> i32 {\n    3\n}\n",
        "pub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    2\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), &swapped).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let p = crate::testing::project(&[("src/app.gleam", original)]);
    board::open(&p.root).unwrap();
    let s = p;
    let edited = original.replace("  1", "  11");
    fs::write(s.root.join("src/app.gleam"), &edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("app.one"));
    let swapped = edited.replace(
        "pub fn two() -> Int {\n  2\n}\n\npub fn three() -> Int {\n  3\n}\n",
        "pub fn three() -> Int {\n  3\n}\n\npub fn two() -> Int {\n  2\n}\n",
    );
    fs::write(s.root.join("src/app.gleam"), &swapped).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    rust_cancel(&s);
    let expected = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    assert_eq!(rust_file(&s), expected);
    assert_eq!(rust_projection(&s), expected);
    assert_ne!(rust_row(&s, "one").2, rust_row(&s, "two").2);
}

/// A recorded file with no module row is forgotten, not refused.
#[test]
fn a_written_entry_with_no_module_is_forgotten() {
    let (_root, s) = crate::testing::rust_project(LIB);
    let mut w = crate::outside::Written::load(&s.root);
    w.record("src/gone.rs", b"pub fn gone() {}");
    w.save().unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Command).unwrap(), "");
    assert!(crate::outside::Written::load(&s.root).files.get("src/gone.rs").is_none());
}

/// An approved function added beside an unexpected edit survives cancel.
#[test]
fn an_approved_added_function_survives_cancel() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, s) = rust_app();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.three".into(), crate::digest(b"pub fn three() -> i32 {\n    3\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let edited = format!("{}\npub fn three() -> i32 {{\n    3\n}}\n", LIB.replace("    2", "    22"));
    fs::write(s.root.join("src/lib.rs"), &edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let approved_text = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    fs::write(s.root.join("src/lib.rs"), LIB.replace("    2", "    22")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let added = format!("{LIB}\npub fn three() -> i32 {{\n    3\n}}\n");
    fs::write(s.root.join("src/lib.rs"), &added).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
    let moved = "pub fn three() -> i32 {\n    3\n}\n\npub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    fs::write(s.root.join("src/lib.rs"), moved).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    fs::write(s.root.join("src/lib.rs"), LIB.replace("    2", "    22")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let both = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    22\n}\n";
    fs::write(s.root.join("src/lib.rs"), both).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.three".into(), crate::digest(b"pub fn three() -> i32 {\n    3\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let inserted = "pub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    22\n}\n";
    fs::write(s.root.join("src/lib.rs"), inserted).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    rust_cancel(&s);
    let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    assert_eq!(rust_file(&s), expected);
    assert_eq!(rust_projection(&s), expected);
    let (one, three, two) = (rust_row(&s, "one").2, rust_row(&s, "three").2, rust_row(&s, "two").2);
    assert!(one < three && three < two, "{one} {three} {two}");
}

fn rust_cancel_task(s: &crate::Store, tid: &str) {
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let uid = u["id"].as_str().unwrap().to_string();
    let b = rust_work_task(&s, "B", "rb");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let approved_text = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
    a["runs"][0]["state"] = json!("ended");
    tasks::put(&s, &a).unwrap();
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let approved_text = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), approved_text).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let both = concat!("pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), both).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.three"));
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let swapped = "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let u = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.one");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    22\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    list.remove("crate.two");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.one");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut a = board::task(&s.root, a["id"].as_str().unwrap()).unwrap();
    a["runs"][0]["state"] = json!("ended");
    tasks::put(&s, &a).unwrap();
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.one");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let p = crate::testing::project(&[("src/app.gleam", original)]);
    board::open(&p.root).unwrap();
    let s = p;
    let a = rust_work_task(&s, "A", "ra");
    let swapped = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  1\n}\n\npub fn three() -> Int {\n  3\n}\n";
    fs::write(s.root.join("src/app.gleam"), swapped).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.insert("app.one".into(), crate::digest(b"pub fn one() -> Int {\n  11\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let approved_text = "pub fn two() -> Int {\n  2\n}\n\npub fn one() -> Int {\n  11\n}\n\npub fn three() -> Int {\n  3\n}\n";
    fs::write(s.root.join("src/app.gleam"), approved_text).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.remove("app.two");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let remaining = "pub fn one() -> Int {\n  11\n}\n\npub fn three() -> Int {\n  3\n}\n";
    fs::write(s.root.join("src/app.gleam"), remaining).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let swapped = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), swapped).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    list.remove("crate.two");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
    rust_cancel(&s);
    let expected = "pub fn one() -> i32 {\n    1\n}";
    assert_eq!(rust_file(&s), expected);
    assert_eq!(rust_projection(&s), expected);
}

fn cancel_after_outside_edit(original: &str, edited: &str, changed: &str) {
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains(changed));
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    list.remove("crate.two");
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited = concat!(
        "pub fn four() -> i32 {\n    4\n}\n\n",
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn three() -> i32 {\n    3\n}\n\n",
        "pub fn one() -> i32 {\n    11\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.one".into(), crate::digest(b"pub fn one() -> i32 {\n    11\n}"));
    list.remove("crate.four");
    list.insert("crate.five".into(), crate::digest(b"pub fn five() -> i32 {\n    5\n}"));
    list.insert("crate.three".into(), crate::digest(b"pub fn three() -> i32 {\n    33\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let after = concat!(
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn three() -> i32 {\n    33\n}\n\n",
        "pub fn five() -> i32 {\n    5\n}\n\n",
        "pub fn one() -> i32 {\n    11\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), after).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.two");
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    11\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited = concat!(
        "pub fn four() -> i32 {\n    4\n}\n\n",
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn three() -> i32 {\n    3\n}\n\n",
        "pub fn one() -> i32 {\n    11\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.four");
    list.insert("crate.five".into(), crate::digest(b"pub fn five() -> i32 {\n    5\n}"));
    list.insert("crate.three".into(), crate::digest(b"pub fn three() -> i32 {\n    33\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let after = concat!(
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn three() -> i32 {\n    33\n}\n\n",
        "pub fn one() -> i32 {\n    11\n}\n\n",
        "pub fn five() -> i32 {\n    5\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), after).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    list.insert("crate.five".into(), crate::digest(b"pub fn five() -> i32 {\n    5\n}"));
    crate::outside::save_approved(&s.root, &list).unwrap();
    let after =
        concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n\n", "pub fn five() -> i32 {\n    5\n}\n",);
    fs::write(s.root.join("src/lib.rs"), after).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.two".into(), crate::digest(b"pub fn two() -> i32 {\n    22\n}"));
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.two".into(), crate::digest(b"pub fn two() -> i32 {\n    22\n}"));
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), original.replace("    1", "    11")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.two");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let after = concat!("pub fn one() -> i32 {\n    11\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), after).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let b = rust_work_task(&s, "B", "rb");
    let reordered =
        concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    1\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), reordered).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.one");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let after = concat!("pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), after).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
    rust_cancel_task(&s, b["id"].as_str().unwrap());
    assert_eq!(rust_file(&s), after);
    assert_eq!(rust_projection(&s), after);
}

fn cancel_after_removal_with_an_unapproved_five(edited: &str, later: &str) {
    let original =
        concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited =
        concat!("pub fn three() -> i32 {\n    3\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn one() -> i32 {\n    11\n}\n",);
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), "pub fn two() -> i32 {\n    22\n}\n\npub fn one() -> i32 {\n    11\n}\n").unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), original.replace("    1", "    11")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let b = rust_work_task(&s, "B", "rb");
    let with_two = concat!(
        "pub fn one() -> i32 {\n    11\n}\n\n",
        "pub fn two() -> i32 {\n    22\n}\n\n",
        "pub fn three() -> i32 {\n    3\n}\n\n",
        "pub fn four() -> i32 {\n    4\n}\n",
    );
    fs::write(s.root.join("src/lib.rs"), with_two).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.two"));
    let mut b = board::task(&s.root, b["id"].as_str().unwrap()).unwrap();
    b["runs"][0]["state"] = json!("ended");
    tasks::put(&s, &b).unwrap();
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.four");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let without_four =
        concat!("pub fn one() -> i32 {\n    11\n}\n\n", "pub fn two() -> i32 {\n    22\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n",);
    fs::write(s.root.join("src/lib.rs"), without_four).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
    let key = {
        let snap = board::snapshot(&s).unwrap();
        snap.as_object().unwrap().iter().find(|(_, v)| v[2].as_str() == Some("three")).map(|(k, _)| k.clone()).unwrap()
    };
    let tid = b["id"].as_str().unwrap();
    let count: i64 =
        s.db.query_row("SELECT COUNT(*) FROM board_before WHERE task=?1 AND entity=?2", params![tid, key], |r| r.get(0)).unwrap();
    let layout: i64 =
        s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task=?1 AND entity=?2 AND run='layout'", params![tid, key], |r| r.get(0))
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains(changed));
    let mut list = crate::outside::approved(&s.root);
    for (name, text) in approved_text {
        list.insert((*name).into(), crate::digest(text.as_bytes()));
    }
    for name in removed {
        list.remove(*name);
    }
    crate::outside::save_approved(&s.root, &list).unwrap();
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let original =
        concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn three() -> i32 {\n    3\n}\n\n\n",);
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let t = rust_work_task(&s, "A", "ra");
    let reordered = "pub fn two() -> i32 {\n    2\n}\n\npub fn one() -> i32 {\n    1\n}\n\npub fn three() -> i32 {\n    3\n}\n";
    fs::write(s.root.join("src/lib.rs"), reordered).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    let mut list = crate::outside::approved(&s.root);
    list.insert("crate.two".into(), crate::digest(b"pub fn two() -> i32 {\n    22\n}"));
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    22\n}\n";
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let t = rust_work_task(&s, "A", "ra");
    let edited = original.replace("    1", "    11");
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> i32 {\n    11\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
    rust_cancel_task(&s, t["id"].as_str().unwrap());
    let expected = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    assert_eq!(rust_file(&s), expected);
    assert_eq!(rust_projection(&s), expected);
}

#[test]
fn a_gleam_cancel_after_removing_the_last_row_keeps_the_ending() {
    let original = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n\npub fn three() -> Int {\n  3\n}\n";
    let p = crate::testing::project(&[("src/app.gleam", original)]);
    board::open(&p.root).unwrap();
    let s = p;
    let edited = original.replace("  1", "  11");
    fs::write(s.root.join("src/app.gleam"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("app.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("app.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> Int {\n  11\n}\n\npub fn two() -> Int {\n  2\n}\n";
    fs::write(s.root.join("src/app.gleam"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let t = rust_work_task(&s, "A", "ra");
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let t = rust_work_task(&s, "A", "ra");
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}";
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    let edited = original.replace("    1", "    11");
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
    let mut list = crate::outside::approved(&s.root);
    list.remove("crate.three");
    crate::outside::save_approved(&s.root, &list).unwrap();
    let later = "pub fn one() -> i32 {\n    11\n}\n";
    fs::write(s.root.join("src/lib.rs"), later).unwrap();
    let _ = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let original =
        concat!("pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n", "pub fn one() -> i32 {\n    1\n}\n",);
    let edited =
        concat!("pub struct S;\n\nimpl S {\n    pub fn a(&self) -> i32 {\n        1\n    }\n}\n\n", "pub fn one() -> i32 {\n    2\n}\n",);
    let (_root, s) = crate::testing::rust_project(original);
    board::open(&s.root).unwrap();
    fs::write(s.root.join("src/lib.rs"), edited).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("crate.one"));
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::Written::load(&s.root).bad.is_empty());
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
}

/// D-229: once an unreadable edit can be read in, its file change becomes
/// symbol changes and Complete works.
#[test]
fn an_unreadable_edit_that_is_fixed_shows_code_and_completes() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let fixed = APP.replace("  1", "  11");
    fs::write(s.root.join("src/app.gleam"), &fixed).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let (_root, s) = crate::testing::rust_project(LIB);
    board::open(&s.root).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = "pub fn one() -> i32 {\n    1\n";
    fs::write(s.root.join("src/lib.rs"), bad).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/lib.rs"), "{note}");
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/lib.rs"));
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
    let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap_err();
    assert!(e.0.contains("E7032") && e.0.contains("can't read"), "{}", e.0);
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert_eq!(rust_file(&s), LIB);
    assert!(crate::outside::Written::load(&s.root).bad.is_empty());
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
}

/// D-229 for Rust: an agent redoes the edit through the store; the file
/// is replaced, its outside diff stays, and Complete works.
#[test]
fn an_unreadable_rust_edit_fixed_through_the_store_completes() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, s) = crate::testing::rust_project(LIB);
    board::open(&s.root).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = "pub fn one() -> i32 {\n    7\n";
    fs::write(s.root.join("src/lib.rs"), bad).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    s.submit_symbols(Some("crate.one"), None, "pub fn one() -> i32 {\n    11\n}\n").unwrap();
    let fixed = LIB.replace("    1", "    11");
    assert_eq!(rust_file(&s), fixed);
    assert!(crate::outside::Written::load(&s.root).bad.is_empty());
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let (_root, s) = crate::testing::rust_project(LIB);
    board::open(&s.root).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = "pub fn one() -> i32 {\n    1\n";
    fs::write(s.root.join("src/lib.rs"), bad).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let fixed = LIB.replace("    1", "    11");
    fs::write(s.root.join("src/lib.rs"), &fixed).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/Bad-Name.gleam"), "pub fn x() -> Int {\n  1\n}\n").unwrap();
    fs::write(s.root.join("src/raw.gleam"), [0xff, 0xfe]).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/Bad-Name.gleam") && note.contains("file:src/raw.gleam"), "{note}");
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let notes = t["notes"].as_array().unwrap().len();
    for (entity, why) in [("file:src/Bad-Name.gleam", "isn't a Gleam module name"), ("file:src/raw.gleam", "isn't UTF-8")] {
        assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == entity), "{entity}");
        assert!(t["notes"][0]["text"].as_str().unwrap().contains(why), "{}", t["notes"]);
    }
    fs::write(s.root.join("src/raw.gleam"), [0xff, 0xfd]).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let t = board::task(&s.root, &tid).unwrap();
    assert_eq!(t["notes"].as_array().unwrap().len(), notes);
    let text = t["notes"][0]["text"].as_str().unwrap();
    assert!(text.contains("src/Bad-Name.gleam") && text.contains("src/raw.gleam"), "{text}");
    fs::write(s.root.join("src/app.gleam"), APP.replace("  1", "  9")).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let t = board::task(&s.root, &tid).unwrap();
    let text = t["notes"][0]["text"].as_str().unwrap();
    assert!(text.contains("app.one") && text.contains("src/Bad-Name.gleam") && text.contains("src/raw.gleam"), "{text}");
}

/// D-229: a non-canonical read leaves a digest that differs from the
/// projection; a later unreadable save is still left on disk.
#[test]
fn an_unreadable_edit_is_not_overwritten_by_a_check() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/app.gleam"), format!("{APP}\n\n")).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/app.gleam"), "{note}");
    assert_eq!(file(&s), twice);
    let _ = s.check();
    assert_eq!(file(&s), twice);
}

/// D-229: without a task holding the change, a submit keeps D-58's refusal.
#[test]
fn an_unreadable_edit_without_a_task_is_never_overwritten() {
    let s = app();
    crate::outside::absorb(&s, Mode::Command).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Command).unwrap().contains("Not read in:"));
    let e = s.submit_symbols(Some("app.one"), None, "pub fn one() -> Int {\n  5\n}\n").unwrap_err();
    assert!(e.0.starts_with("E0104"), "{}", e.0);
    assert_eq!(file(&s), twice);
}

/// D-229: a new unreadable file can be created by the store over its file.
#[test]
fn a_new_unreadable_file_can_be_redone_through_the_store() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = "pub fn x() -> Int {\n  1\n}\n\npub fn x() -> Int {\n  2\n}\n";
    fs::write(s.root.join("src/fresh.gleam"), bad).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:src/fresh.gleam"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    s.submit_symbols(None, Some("fresh"), "pub fn x() -> Int {\n  3\n}\n").unwrap();
    let fixed = "pub fn x() -> Int {\n  3\n}\n";
    assert_eq!(fs::read_to_string(s.root.join("src/fresh.gleam")).unwrap(), fixed);
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/Bad-Name.gleam"), "pub fn x() -> Int {\n  1\n}\n").unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:src/Bad-Name.gleam"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    fs::remove_file(s.root.join("src/Bad-Name.gleam")).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
    assert!(crate::outside::Written::load(&s.root).bad.is_empty());
    let t = board::task(&s.root, &tid).unwrap();
    assert!(!board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/Bad-Name.gleam"));
    let flow = board::flow(&s.root).unwrap();
    assert_eq!(board::column(&flow, &t)["final"], true);
}

/// D-229: a fix that changes no symbol still refreshes the task's note.
#[test]
fn a_hand_fix_without_a_symbol_change_clears_the_reason() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/app.gleam"), format!("{APP}\n\n")).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("Not read in:"), "{bad}");
    let e = s.submit_symbols(Some("crate.one"), None, "pub fn one() -> i32 {\n    11\n}\n").unwrap_err();
    assert!(e.0.starts_with("E0104"), "{}", e.0);
    assert_eq!(rust_file(&s), bad);
}

/// D-229: a file first marked unreadable during a command is picked up by
/// the board's next poll, so it can be reviewed and canceled.
#[test]
fn an_unreadable_edit_marked_during_a_command_is_later_reviewed() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Command).unwrap().contains("Not read in:"));
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/app.gleam"), "{note}");
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/app.gleam"));
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"approve"})).unwrap();
    let e = board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"complete"})).unwrap_err();
    assert!(e.0.contains("E7032"), "{}", e.0);
    board::api(&s.root, "POST", &format!("/tasks/{tid}/actions"), json!({"action":"cancel"})).unwrap();
    assert_eq!(file(&s), APP);
    assert!(crate::outside::Written::load(&s.root).bad.is_empty());
}

/// D-229: a file first marked during a run stays a note while it goes,
/// then joins Unexpected Changes once the run stops.
#[test]
fn an_unreadable_edit_marked_during_a_run_is_held_after_it_stops() {
    let (_root, s) = rust_app();
    let t = rust_work_task(&s, "Work", "r1");
    let bad = "pub fn one() -> i32 {\n    1\n";
    fs::write(s.root.join("src/lib.rs"), bad).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("Not read in:"));
    assert_eq!(board::tasks(&s.root).unwrap().len(), 1);
    let mut t = board::task(&s.root, t["id"].as_str().unwrap()).unwrap();
    t["runs"][0]["state"] = json!("done");
    tasks::put(&s, &t).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/lib.rs"), "{note}");
    let unexpected = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    assert!(board::changes_of(&s, &unexpected).unwrap().iter().any(|c| c["entity"] == "file:src/lib.rs"));
}

/// An empty Unexpected Changes task no longer blocks work once its file is restored.
#[test]
fn a_restored_unreadable_file_closes_the_empty_unexpected_task() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    fs::write(s.root.join("src/app.gleam"), APP).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &bad).unwrap();
    let mut written = fs::read_to_string(s.root.join(".solace/written")).unwrap();
    written.push_str(&format!("! {} src/app.gleam\n", crate::digest(bad.as_bytes())));
    fs::write(s.root.join(".solace/written"), written).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("file:src/app.gleam"), "{note}");
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    assert!(board::changes_of(&s, &t).unwrap().iter().any(|c| c["entity"] == "file:src/app.gleam"));
}

/// A broken file before the first submit is fixed through the command route and can be completed.
#[test]
fn a_broken_file_is_redone_through_the_submit_and_test_commands() {
    let s = app();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
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
    let tested =
        crate::cli::routing::with_root(&s.root, || crate::cli::routing::run_options(&test, &mut || Ok(String::new()), None, 100_000, &[]))
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let twice = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &twice).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    let mut t = board::task(&s.root, &tid).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    fs::write(s.root.join("src/app.gleam"), APP).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/app.gleam"), APP.replace("  2", "  9")).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("app.two"), "{note}");
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    fs::write(s.root.join("src/app.gleam"), APP).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    let p = crate::testing::project(&[("src/app.gleam", APP), ("src/other.gleam", OTHER)]);
    let s = board::open(&p.root).unwrap();
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    let bad = format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n");
    fs::write(s.root.join("src/app.gleam"), &bad).unwrap();
    fs::write(s.root.join("src/other.gleam"), format!("{OTHER}\npub fn other() -> Int {{\n  5\n}}\n")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    fs::write(s.root.join("src/app.gleam"), APP).unwrap();
    fs::write(s.root.join("src/other.gleam"), format!("{OTHER}\n")).unwrap();
    assert_eq!(crate::outside::absorb(&s, Mode::Notice).unwrap(), "");
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
    crate::outside::absorb(&s, Mode::Notice).unwrap();
    fs::write(s.root.join("src/app.gleam"), format!("{APP}\npub fn two() -> Int {{\n  3\n}}\n")).unwrap();
    assert!(crate::outside::absorb(&s, Mode::Notice).unwrap().contains("file:src/app.gleam"));
    let t = board::tasks(&s.root).unwrap().into_iter().find(|t| t["unexpected"] == true).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    fs::remove_file(s.root.join("src/app.gleam")).unwrap();
    let got = crate::outside::absorb(&s, Mode::Notice).unwrap();
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
/// A root ignore that keeps hiding the store is reported through absorb.
#[test]
fn a_root_ignore_that_still_hides_the_store_is_flagged() {
    let s = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    crate::board::task_model::open(&s.root).unwrap();
    let root = s.root.join(".gitignore");
    fs::write(&root, format!("{}*.db\n", crate::outside_share::STORE_IGNORE)).unwrap();
    let note = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(note.contains("still ignores store.db"), "{note}");
    fs::write(&root, format!("{}*.db\n!**/.solace/store.db\n", crate::outside_share::STORE_IGNORE)).unwrap();
    let fixed = crate::outside::absorb(&s, Mode::Notice).unwrap();
    assert!(!fixed.contains("still ignores store.db"), "{fixed}");
}
