use crate::testing::*;
use super::*;
use crate::store_write::ImportMatch;

    /// A dead-code warning the submit leaves on another symbol is named under
    /// its own heading.
    #[test]
    fn a_submit_names_a_warning_it_leaves_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(out.contains("New warnings elsewhere:"), "{out}");
        assert!(out.contains("app.tools: warning:"), "{out}");
    }

    /// A warning that was there before the submit stays quiet.
    #[test]
    fn a_submit_does_not_repeat_a_warning_from_before() {
        let s = project(&[("src/app.gleam", "fn unused() {\n  1\n}\n\npub fn main() {\n  1\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  2\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
        assert!(!out.contains("app.unused"), "{out}");
    }

    /// Before any check has run there is no baseline, so no warning is new.
    #[test]
    fn a_submit_before_a_check_does_not_name_a_warning_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
    }

    /// Both warning sections share one footer.
    #[test]
    fn a_submit_prints_the_diagnostics_footer_once() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  todo\n}\n").unwrap();
        assert!(out.contains("\nWarnings:"), "{out}");
        assert!(out.contains("\nNew warnings elsewhere:"), "{out}");
        assert_eq!(out.matches("Use `check --warnings` for full diagnostics.").count(), 1, "{out}");
    }

#[test]
fn only_gleams_unknown_module_error_names_a_qualifier() {
    assert_eq!(unknown_module("Unknown module type\n\nThe module `helper` does not have a `Foo` type."), None);
    assert_eq!(unknown_module("Unknown module value\n\nThe module `helper` does not have a `x` value."), None);
    assert_eq!(unknown_module("Unknown module\n\nThe module `helper` does not exist."), Some("helper"));
    assert_eq!(unknown_module("No module has been found with the name `sqlight`."), Some("sqlight"));
}

#[test]
fn dependency_modules_come_from_direct_manifest_entries_only() {
    let root = Scratch::new().with(&[
        (
            "gleam.toml",
            "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ngleam_json = \"1.0\"\n\n[dev-dependencies]\ngleeunit = \"1.0\"\n",
        ),
        ("build/packages/gleam_json/src/gleam/json.gleam", ""),
        ("build/packages/gleam_stdlib/src/gleam/list.gleam", ""),
        ("build/packages/gleeunit/src/gleeunit.gleam", ""),
    ]);
    let modules = dependency_modules(&root);
    assert_eq!(modules.iter().map(|m| (m.path.as_str(), m.test)).collect::<Vec<_>>(), vec![("gleam/json", false), ("gleeunit", true)]);
}

#[test]
fn an_alias_to_an_unlisted_dependency_is_not_reused() {
    let store = project(&[
        ("build/packages/indirect/src/indirect.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/caller.gleam", "import indirect as x\n\npub fn call() -> Int {\n  x.value()\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let ix = store.index().unwrap();
    let deps = store.dependency_modules();
    assert!(matches!(import_match(&ix, &deps, "x", false), ImportMatch::None));
}

/// Only the selector's own modules and their submodules make the compiled
/// selection stale; another module's change leaves selection incremental.
#[test]
fn the_servers_own_source_newer_than_its_binary_is_stale() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod reach;\npub mod store_write;\npub mod view;\n"),
        ("src/reach.rs", "pub mod flow;\n\npub fn test_symbols() {}\n\npub fn closure() {}\n\npub fn hash() {}\n"),
        ("src/reach/flow.rs", "pub fn use_it() {}\n"),
        ("src/store_write.rs", "pub struct Store;\n\nimpl Store {\n    pub fn test() {}\n}\n"),
        ("src/view.rs", "pub fn page() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let ix = store.index().unwrap();
    let selected = selection_modules(&ix);
    assert_eq!(selected, ["reach".to_string(), "reach::flow".to_string(), "store_write".to_string()].into_iter().collect::<HashSet<_>>());
    let built = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(20));
    set_time(&root.join("src/reach.rs"), 10);
    set_time(&root.join("src/reach/flow.rs"), 10);
    set_time(&root.join("src/store_write.rs"), 10);
    set_time(&root.join("src/view.rs"), 30);
    assert!(!selection_source_is_newer(&root, built, &ix.modules, &selected), "a view change does not affect selection");
    assert!(selection_source_is_newer(&root, built, &ix.modules, &HashSet::new()), "no selector modules: every file counts");
    set_time(&root.join("src/reach.rs"), 30);
    assert!(selection_source_is_newer(&root, built, &ix.modules, &selected), "a reach change affects selection");
    set_time(&root.join("src/reach.rs"), 10);
    set_time(&root.join("src/store_write.rs"), 30);
    assert!(selection_source_is_newer(&root, built, &ix.modules, &selected), "a store_write change affects selection");
    set_time(&root.join("src/store_write.rs"), 10);
    set_time(&root.join("src/reach/flow.rs"), 30);
    assert!(selection_source_is_newer(&root, built, &ix.modules, &selected), "a reach submodule change affects selection");
}

fn set_time(path: &Path, secs: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        .unwrap();
}

/// A rebuild that replaces the binary file does not hide that this process is
/// still running the old code.
#[test]
fn a_rebuilt_binary_does_not_hide_the_servers_stale_code() {
    let root = Scratch::new().with(&[("src/lib.rs", "pub fn one() {}\n")]);
    let exe = root.join("solace");
    std::fs::write(&exe, "").unwrap();
    set_time(&exe, 20);
    let built = std::fs::metadata(&exe).unwrap().modified().unwrap();
    set_time(&exe, 40);
    let source = || std::iter::once("src/lib.rs".to_string());
    set_time(&root.join("src/lib.rs"), 30);
    assert!(source_is_newer_than(&root, Some(built), source()), "the captured time, not the replaced file, decides");
    set_time(&root.join("src/lib.rs"), 10);
    assert!(!source_is_newer_than(&root, Some(built), source()));
    assert!(!source_is_newer_than(&root, None, source()), "with no captured time there is nothing to compare");
    assert!(!source_is_newer_than(&root, Some(built), std::iter::once("src/missing.rs".to_string())), "a missing file is ignored");
}

/// The stale reason appears only when the fallback forced every test, never for
/// `--all`.
#[test]
fn the_stale_reason_is_shown_only_when_selection_forced_every_test() {
    let reason = "Ran all 2 tests: Solace's own code changed since this server started, so test selection is out of date.\n";
    assert_eq!(test_head(2, 2, &[], false, true), reason);
    assert_eq!(test_head(2, 2, &[], true, true), "Ran all 2 tests.\n");
    assert_eq!(test_head(2, 2, &[], false, false), "Ran all 2 tests.\n");
    assert_eq!(test_head(1, 2, &["crate.add"], false, false), "Ran 1 of 2 tests: those that reach crate.add.\n");
    assert_eq!(
        test_head(1, 2, &[], false, false),
        "Ran 1 of 2 tests; the other 1 reach nothing changed since they passed (`test --all` runs every test).\n"
    );
}

/// Every symbol the test-selection fallback names resolves, so a rename here
/// fails instead of silently shrinking the modules that force every test.
#[test]
fn every_selection_symbol_resolves_in_this_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(crate::store_core::SCHEMA).unwrap();
    let files = crate::language::rust::package(root).unwrap();
    crate::store::store_rust_package(&db, &files).unwrap();
    let store = Store::new(Rc::new(db), root, true, true);
    let ix = store.index().unwrap();
    for name in SELECTION_SYMBOLS {
        assert!(ix.syms.iter().any(|s| s.qname() == *name), "{name} names no symbol in this crate");
    }
}

/// A newer executable means the running copy is older than the saved code.
#[test]
fn a_newer_binary_means_the_running_copy_is_stale() {
    let earlier = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(20));
    let later = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(40));
    assert!(binary_is_newer(earlier, later));
    assert!(!binary_is_newer(later, earlier));
    assert!(!binary_is_newer(earlier, earlier), "the same binary is not a rebuild");
    assert!(!binary_is_newer(None, later), "with no captured time there is nothing to compare");
    assert!(!binary_is_newer(earlier, None), "a missing executable is not a rebuild");
}

#[test]
fn the_stale_binary_note_is_sent_once() {
    let earlier = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(20));
    let later = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(40));
    let sent = std::sync::atomic::AtomicBool::new(false);
    let looked = std::cell::Cell::new(false);
    assert!(
        stale_binary_notice_from(&sent, earlier, || {
            looked.set(true);
            later
        })
        .is_some()
    );
    assert!(looked.get());
    assert!(sent.load(std::sync::atomic::Ordering::Relaxed));
    looked.set(false);
    assert!(
        stale_binary_notice_from(&sent, earlier, || {
            looked.set(true);
            later
        })
        .is_none()
    );
    assert!(!looked.get(), "a sent note does not look at the binary again");
    let fresh = std::sync::atomic::AtomicBool::new(false);
    assert!(stale_binary_notice_from(&fresh, later, || earlier).is_none());
    assert!(!fresh.load(std::sync::atomic::Ordering::Relaxed), "an unchanged binary leaves the note unsent");
}

