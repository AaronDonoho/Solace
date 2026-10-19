//! Reading a Rust store by name (D-158): `view`, `inspect`, `find` and the session.

use super::*;
use crate::testing::*;

/// A crate whose `src/a/agents.rs` makes module `a::agents`, so a suffix name
/// (`agents::deep`) has to resolve.
const LIB: &str = "//! A sample crate.\n\npub mod a;\n\n/// The store.\npub struct Store {\n    pub root: String,\n}\n\nimpl Store {\n    /// Opens it.\n    pub fn open() -> Self {\n        Store { root: String::new() }\n    }\n}\n\npub fn helper() -> i32 {\n    Store::open();\n    1\n}\n";

/// The `a::agents` module: a container and its method, the shape of the
/// Done-when name `board::agents.Store::open`.
const AGENTS: &str = "pub struct Panel {\n    pub id: u32,\n}\n\nimpl Panel {\n    pub fn show() -> u32 {\n        3\n    }\n}\n\npub fn deep() -> i32 {\n    2\n}\n";

/// The sample crate, read into a new store.
fn sample() -> (Scratch, Store) {
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", LIB),
        ("src/a.rs", "pub mod agents;\n"),
        ("src/a/agents.rs", AGENTS),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    (root, store)
}

/// A name that ends in a separator has no leaf to suggest by, so the error
/// names no similar symbols.
#[test]
fn a_name_ending_in_a_separator_gets_no_similar_hint() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    assert_eq!(similar_hint(&ix, "crate."), "");
    assert_eq!(similar_hint(&ix, "crate::"), "");
}

/// `view <name>` shows the item's own text.
#[test]
fn view_shows_a_rust_item_by_name() {
    let (_root, store) = sample();
    let out = store.view(&["crate.Store::open"], false, usize::MAX).unwrap();
    assert!(out.contains("crate.Store::open (fn)"), "{out}");
    assert!(out.contains("pub fn open() -> Self"), "{out}");
}

/// `view <module>` outlines it, nested, with the file's signatures.
#[test]
fn view_outlines_a_rust_module() {
    let (_root, store) = sample();
    let out = store.view(&["crate"], false, usize::MAX).unwrap();
    assert!(out.contains("//! A sample crate."), "{out}");
    assert!(out.contains("pub struct Store"), "{out}");
    assert!(out.contains("impl Store"), "{out}");
    assert!(out.contains("\n  pub fn open() -> Self"), "a child is indented by its depth: {out}");
    assert!(out.contains("pub fn helper() -> i32"), "{out}");
}

/// The module's body is its file, byte for byte.
#[test]
fn a_rust_module_body_is_the_file() {
    let (root, store) = sample();
    let out = store.view(&["crate"], true, usize::MAX).unwrap();
    assert_eq!(out, fs::read_to_string(root.join("src/lib.rs")).unwrap());
}

/// `inspect` shows the item with its Rust kind and the Rust names it uses (D-170).
#[test]
fn inspect_shows_a_rust_item() {
    let (_root, store) = sample();
    let out = store.inspect(&["crate.Store::open"], false, usize::MAX).unwrap();
    assert!(out.contains("crate.Store::open (fn)"), "{out}");
    let out = store.inspect(&["crate.helper"], true, usize::MAX).unwrap();
    assert!(out.contains("crate.helper (fn)"), "{out}");
    assert!(out.contains("== Uses ==") && out.contains("crate.Store::open"), "{out}");
}

/// `find` shows the stored kind, not Gleam's `fn`.
#[test]
fn find_shows_the_rust_kind() {
    let (_root, store) = sample();
    let out = store.find("Store").unwrap();
    assert!(out.contains("crate.Store (struct)"), "{out}");
}

/// A module suffix after `::` resolves, a type wins over its `impl`, and a
/// module-qualified `a::agents.Panel::show` resolves.
#[test]
fn names_resolve_by_suffix_and_a_type_wins() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    let i = store.resolve(&ix, "agents::deep").unwrap();
    assert_eq!(ix.syms[i].qname(), "a::agents.deep");
    assert!(store.resolve_module(&ix, "agents").is_some());
    let i = store.resolve(&ix, "a::agents.Panel::show").unwrap();
    assert_eq!(ix.syms[i].qname(), "a::agents.Panel::show");
    let i = store.resolve(&ix, "Store").unwrap();
    assert_eq!(ix.syms[i].kind_text, "struct", "`Store` names the type, not its impl");
}

/// A `.rs` file operand still reaches `orientation`.
#[test]
fn a_rust_file_operand_reaches_orientation() {
    let (root, _store) = sample();
    let out = orientation::view(&root, &toml::Value::Table(Default::default()), &["view", "src/lib.rs::open"], usize::MAX);
    let Some(Ok(text)) = out else { panic!("a `.rs` operand is a file view") };
    assert!(text.contains("pub fn open"), "{text}");
}

/// A Rust session lists its modules and names, and carries D-158 point 2's lines.
#[test]
fn a_rust_session_lists_names_and_the_brief() {
    let (_root, store) = sample();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("crate — A sample crate."), "{out}");
    assert!(out.contains("Store, Store::open, helper"), "{out}");
    assert!(out.contains("a::agents — "), "{out}");
    assert!(out.contains("Rust in Solace: every item has its own name, once per module"), "{out}");
}

/// A Gleam session doesn't carry the Rust lines.
#[test]
fn a_gleam_session_does_not_carry_the_brief() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let out = p.session(&[]).unwrap();
    assert!(!out.contains("Rust in Solace"), "{out}");
}

/// `session start --task` carries the Rust lines too.
#[test]
fn a_task_session_carries_the_brief() {
    let (_root, store) = sample();
    let out = store.task_session("change helper", None, "").unwrap();
    assert!(out.contains("Rust in Solace: every item has its own name"), "{out}");
}

/// A Rust index builds the reference graph from the names its text mentions:
/// `helper` calls `Store::open`, so `open`'s users name `helper` (D-170).
#[test]
fn a_rust_index_builds_a_reference_graph() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    assert!(ix.rust);
    let at = |q: &str| ix.syms.iter().position(|s| s.qname() == q).unwrap();
    assert!(ix.uses[at("crate.helper")].contains(&at("crate.Store::open")));
    assert!(ix.used_by[at("crate.Store::open")].contains(&at("crate.helper")));
    assert!(ix.at[at("crate.helper")].iter().any(|&(t, _)| t == at("crate.Store::open")), "a reference has its line");
    assert!(!ix.test[at("crate.helper")], "a source function is not test code");
}

/// `refs` lists a Rust function's callers, and `impact <name>` reaches them
/// with the call's name, as they do for a Gleam name (D-170).
#[test]
fn refs_and_impact_answer_rust_names() {
    let (_root, store) = sample();
    let out = store.refs("crate.Store::open").unwrap();
    assert!(out.starts_with("crate.Store::open is used by 1:"), "{out}");
    assert!(out.contains("-- crate.helper"), "{out}");
    let out = store.impact(&["crate.Store::open"], usize::MAX).unwrap();
    assert!(out.contains("crate.helper — 1 step, calls crate.Store::open; no test of its own"), "{out}");
}

/// A crate whose tests sit in a `#[path]` file, an inline module and a
/// `tests/` crate, with a helper only the tests call (D-170).
const REACH_LIB: &str = "//! Sample.\n\npub mod other;\n\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n\n#[cfg(test)]\n#[path = \"lib_tests.rs\"]\nmod tests;\n";
const REACH_TESTS: &str = "use crate::add;\n\nfn helper() -> i32 {\n    add(1, 1)\n}\n\n#[test]\nfn add_works() {\n    assert_eq!(add(1, 2), 3);\n}\n\n#[test]\nfn helper_works() {\n    assert_eq!(helper(), 2);\n}\n";
const REACH_OTHER: &str = "pub fn twice(n: i32) -> i32 {\n    n * 2\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn twice_works() {\n        assert_eq!(twice(1), 2);\n    }\n}\n";
const REACH_INTEGRATION: &str = "use app::sub;\n\n#[test]\nfn sub_works() {\n    assert_eq!(sub(2, 1), 1);\n}\n";

/// The sample crate of the test-reach checks, read into a new store.
fn reach_sample() -> (Scratch, Store) {
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", REACH_LIB),
        ("src/lib_tests.rs", REACH_TESTS),
        ("src/other.rs", REACH_OTHER),
        ("tests/acceptance.rs", REACH_INTEGRATION),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    (root, store)
}

/// Test reach picks Rust `#[test]` functions by libtest name, and a change
/// selects exactly the tests that reach it (D-170).
#[test]
fn test_reach_selects_rust_tests_by_libtest_name() {
    let (_root, store) = reach_sample();
    let ix = store.index().unwrap();
    let tests = crate::reach::test_symbols(&ix);
    let names: Vec<String> = tests.iter().map(|&t| crate::reach::libtest_name(&ix, t)).collect();
    assert_eq!(names, ["sub_works", "other::tests::twice_works", "tests::add_works", "tests::helper_works"], "{names:?}");
    let reached = |q: &str| -> Vec<String> {
        let i = ix.syms.iter().position(|s| s.qname() == q).unwrap();
        tests.iter().filter(|&&t| crate::reach::closure(&ix, t).contains(&i)).map(|&t| crate::reach::libtest_name(&ix, t)).collect()
    };
    assert_eq!(reached("crate.add"), ["tests::add_works", "tests::helper_works"]);
    assert_eq!(reached("crate.sub"), ["sub_works"]);
    assert_eq!(reached("other.twice"), ["other::tests::twice_works"]);
}

/// The Done-when check: `test <fn>` runs exactly the `#[test]`s a full
/// `cargo test` shows reach it, by libtest name (D-170).
#[test]
fn a_change_runs_the_rust_tests_that_reach_it() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = reach_sample();
    let out = store.test(&["crate.add"]).unwrap();
    assert!(out.starts_with("Ran 2 of 4 tests: those that reach crate.add."), "{out}");
}

/// An unknown name still suggests by its leaf, the part after the last `.`/`::`.
#[test]
fn an_unknown_name_suggests_by_its_leaf() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    for miss in ["crate.hel", "crate::hel"] {
        let e = store.resolve(&ix, miss).unwrap_err();
        assert!(e.0.contains("Similar names: crate.helper"), "{miss}: {}", e.0);
    }
}
