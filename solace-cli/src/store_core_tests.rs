use super::*;



#[test]
fn a_busy_checkpoint_is_reported() {
    assert!(checkpointed(0).is_ok());
    let e = checkpointed(1).unwrap_err();
    assert!(e.0.starts_with("E7028"), "{}", e.0);
}

/// Checkpointing puts changes held in the WAL into the file a commit copies.
#[test]
fn checkpoint_puts_the_whole_store_in_store_db() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    let s = crate::board::task_model::open(&p.root).unwrap();
    s.db.query_row("PRAGMA wal_autocheckpoint=0", [], |_| Ok(())).unwrap();
    s.db.execute("INSERT INTO board_before VALUES ('t','symbol:x','null')", []).unwrap();
    let wal = s.root.join(".solace/store.db-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0, "the change is not in the WAL");
    checkpoint(&s).unwrap();
    // A commit copies `store.db` alone, so the change must be in that file.
    let copy = s.root.join(".solace/copied.db");
    std::fs::copy(s.root.join(".solace/store.db"), &copy).unwrap();
    let c = rusqlite::Connection::open(&copy).unwrap();
    let rows: i64 = c.query_row("SELECT COUNT(*) FROM board_before", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

/// The first open removes the old shared board log, and its note text with it.
#[test]
fn opening_a_store_wipes_its_old_board_events_once() {
    let root = crate::testing::Scratch::new().with(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    crate::import(&root).unwrap();
    let file = root.join(".solace/store.db");
    let note = "a conversation that must not be committed";
    let db = Connection::open(&file).unwrap();
    db.execute_batch("CREATE TABLE board_events(seq INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL)").unwrap();
    db.execute("INSERT INTO board_events(data) VALUES (?1)", [serde_json::json!({"task":{"notes":[{"text":note}]}}).to_string()]).unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(db);
    assert!(std::fs::read(&file).unwrap().windows(note.len()).any(|w| w == note.as_bytes()), "the old note is not in store.db to start");
    let s = crate::board::task_model::open(&root).unwrap();
    let old: i64 =
        s.db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='board_events'", [], |r| r.get(0)).unwrap();
    assert_eq!(old, 0, "the old table is still there");
    drop(s);
    let before = std::fs::read(&file).unwrap();
    assert!(!before.windows(note.len()).any(|w| w == note.as_bytes()), "the note text is still in store.db");
    drop(crate::board::task_model::open(&root).unwrap());
    assert_eq!(std::fs::read(&file).unwrap(), before, "a second open cleaned the store again");
}

/// A store that never had the old log opens without creating or rewriting it.
#[test]
fn a_store_without_the_old_board_log_opens_unchanged() {
    let root = crate::testing::Scratch::new().with(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    crate::import(&root).unwrap();
    let file = root.join(".solace/store.db");
    let s = crate::board::task_model::open(&root).unwrap();
    let old: i64 =
        s.db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='board_events'", [], |r| r.get(0)).unwrap();
    assert_eq!(old, 0, "opening created the old table");
    drop(s);
    let before = std::fs::read(&file).unwrap();
    drop(crate::board::task_model::open(&root).unwrap());
    assert_eq!(std::fs::read(&file).unwrap(), before, "opening a clean store rewrote it");
}
