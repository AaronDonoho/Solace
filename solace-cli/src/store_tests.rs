use super::*;
use crate::testing::*;

#[test]
fn a_split_rust_file_is_stored_nested_and_written_back_exactly() {
    let board = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/board.rs")).unwrap();
    let sources = [
        ("board", board.as_str()),
        ("one_line", "impl A { fn f() {} }\n"),
        ("no_newline", "const A: u8 = 1;\nconst B: u8 = 2;"),
        ("nested", "impl<T> Store<T> {\n    pub fn open() {}\n\n    fn close() {}\n}\n\nmod tests {\n    fn t() {}\n}\n\nstruct S;\n"),
    ];
    let root = Scratch::new();
    let db = Connection::open(root.join("store.db")).unwrap();
    db.execute_batch(SCHEMA).unwrap();
    for (m, text) in sources {
        store_rust_file(&db, m, "src", &format!("src/{m}.rs"), &language::rust::split(text, m).unwrap()).unwrap();
    }
    let store = Store { db: Rc::new(db), root: root.to_path_buf(), lang: None, nested: true, owns_dir: true };
    let ix = store.index().unwrap();
    for (m, text) in sources {
        let row = ix.modules.iter().find(|r| r.path == m).unwrap();
        assert_eq!(row.rel(), format!("src/{m}.rs"));
        assert!(projection(&ix, row).0 == text, "{m} is written back exactly");
    }
    let sym = |m: &str, n: &str| ix.syms.iter().find(|s| s.module == m && s.name == n).unwrap();
    assert_eq!(sym("nested", "Store<T>::open").parent.as_deref(), Some(sym("nested", "Store<T>").id.as_str()));
    assert_eq!((sym("nested", "Store<T>").tail.as_str(), sym("nested", "Store<T>").sep.as_deref()), ("\n}", Some("\n\n")));
    assert_eq!(sym("nested", "Store<T>::open").sep.as_deref(), Some("\n\n"));
    assert_eq!(sym("no_newline", "A").sep.as_deref(), Some("\n"));
    assert_eq!(sym("no_newline", "B").sep.as_deref(), Some(""));
    assert!(sym("no_newline", "A").id.starts_with("cn:"));
    assert!(sym("nested", "S").id.starts_with("ty:") && sym("nested", "tests::t").id.starts_with("fn:"));
}

#[test]
fn an_older_store_gains_the_nested_item_columns() {
    let root = Scratch::new();
    fs::create_dir_all(root.join(".solace")).unwrap();
    let db = Connection::open(root.join(".solace/store.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE modules(path TEXT PRIMARY KEY, dir TEXT NOT NULL, header TEXT NOT NULL);
CREATE TABLE symbols(id TEXT PRIMARY KEY, module TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  text TEXT NOT NULL, ord INTEGER NOT NULL, revision TEXT NOT NULL, live INTEGER NOT NULL DEFAULT 1);
INSERT INTO modules VALUES ('app', 'src', '');
INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES ('fn:1', 'app', 'main', 'fn', 'pub fn main() { 1 }', 0, '');",
    )
    .unwrap();
    drop(db);
    // Peeking changes no table, so the older columns are read as Gleam's.
    let ix = Store::peek(&root).unwrap().index().unwrap();
    assert!(ix.syms[0].parent.is_none() && ix.modules[0].file.is_none());
    assert!(ix.modules[0].owns_dir.is_none());
    let ix = Store::at(&root).unwrap().index().unwrap();
    assert!(ix.syms[0].parent.is_none() && ix.syms[0].sep.is_none() && ix.syms[0].tail.is_empty() && ix.modules[0].file.is_none());
    assert!(ix.modules[0].owns_dir.is_none());
    assert_eq!(projection(&ix, &ix.modules[0]).0, "pub fn main() { 1 }\n");
}

#[test]
fn a_copy_of_this_crate_is_imported_and_written_back_exactly() {
    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = vec![];
    rs_files(&crate_dir.join("src"), &mut paths);
    rs_files(&crate_dir.join("tests"), &mut paths);
    let rels: Vec<String> = paths.iter().map(|p| p.strip_prefix(crate_dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    let root = Scratch::new();
    fs::copy(crate_dir.join("Cargo.toml"), root.join("Cargo.toml")).unwrap();
    // The lockfile pins the versions already in cargo's cache, so the import
    // resolves them offline inside Solace's sandbox.
    fs::copy(crate_dir.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();
    for rel in &rels {
        fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        fs::copy(crate_dir.join(rel), root.join(rel)).unwrap();
    }
    import(&root).unwrap();
    for rel in &rels {
        fs::remove_file(root.join(rel)).unwrap();
    }
    let store = Store::at(&root).unwrap();
    let ix = store.index().unwrap();
    store.materialize(&ix).unwrap();
    for rel in &rels {
        assert!(fs::read(root.join(rel)).ok() == fs::read(crate_dir.join(rel)).ok(), "{rel} is written back exactly");
    }
    let module = |p: &str| ix.modules.iter().find(|m| m.path == p).unwrap();
    assert!(module("crate").header.contains("\nmod board;\n"));
    assert_eq!(module("board::tests").rel(), "src/board_tests.rs");
    assert_eq!(module("integration::acceptance").rel(), "tests/acceptance.rs");
    assert_eq!(module("crate").owns_dir, Some(true), "a crate root's modules are beside it");
    assert_eq!(module("board").owns_dir, Some(false), "a plain file's modules are in a folder");
    assert_eq!(module("board::tests").owns_dir, Some(true), "a `#[path]` file's modules are beside it");
}

#[test]
fn a_plain_error_keeps_not_saved() {
    let e = rust_syntax_error(Failure("rustfmt failed".into(), 3), "crate.one", None, "crate", 0, 5);
    assert_eq!(e.0, "Not saved: rustfmt failed");
    assert_eq!(e.1, 3);
    let e = rust_syntax_error(Failure("Not saved: elsewhere".into(), 1), "crate.one", None, "crate", 0, 5);
    assert_eq!(e.0, "Not saved: elsewhere");
}

#[test]
fn a_warning_stays_in_the_baseline_when_its_module_reports_nothing() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    p.record_warnings(&[crate::Diag {
        error: false,
        module: Some("a".into()),
        symbol: Some("a.unused".into()),
        text: "warning: unused".into(),
    }])
    .unwrap();
    p.record_warnings(&[]).unwrap();
    let baseline = p.warning_baseline().unwrap().unwrap();
    assert!(baseline.iter().any(|key| key.starts_with("a.unused\n")), "{baseline:?}");
}

#[test]
fn an_older_store_gains_the_warning_module_column() {
    let root = Scratch::new();
    fs::create_dir_all(root.join(".solace")).unwrap();
    let db = Connection::open(root.join(".solace/store.db")).unwrap();
    db.execute_batch("CREATE TABLE warnings(key TEXT PRIMARY KEY);\nINSERT INTO warnings VALUES ('old');").unwrap();
    drop(db);
    let store = Store::at(&root).unwrap();
    let module: String = store.db.query_row("SELECT module FROM warnings", [], |r| r.get(0)).unwrap();
    assert_eq!(module, "project");
}
