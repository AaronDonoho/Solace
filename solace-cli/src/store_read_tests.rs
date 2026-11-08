//! Reading a Rust store by name (D-158): `view`, `inspect`, `find` and the session.

use super::*;
use crate::testing::*;

#[cfg(test)] use crate::cli::{run_inner, with_root};
/// A crate whose `src/a/agents.rs` makes module `a::agents`, so a suffix name
/// (`agents::deep`) has to resolve.
const LIB: &str = r#"//! A sample crate.

pub mod a;

/// The store.
pub struct Store {
    pub root: String,
}

impl Store {
    /// Opens it.
    pub fn open() -> Self {
        Store { root: String::new() }
    }
}

pub fn helper() -> i32 {
    Store::open();
    1
}
"#;

/// The `a::agents` module: a container and its method, the shape of the
/// Done-when name `board::agents.Store::open`.
const AGENTS: &str = r#"pub struct Panel {
    pub id: u32,
}

impl Panel {
    pub fn show() -> u32 {
        3
    }
}

pub fn deep() -> i32 {
    2
}
"#;

/// The sample crate, read into a new store.
fn sample() -> (Scratch, Store) {
    let root =
        Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", LIB), ("src/a.rs", "pub mod agents;\n"), ("src/a/agents.rs", AGENTS)]);
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
    let out = store.inspect(&["crate.Store::open"], false, None, usize::MAX).unwrap();
    assert!(out.contains("crate.Store::open (fn)"), "{out}");
    let out = store.inspect(&["crate.Store::open"], false, Some("1..1,4..4"), usize::MAX).unwrap();
    let line1 = out.lines().find(|line| line.starts_with("1: ")).unwrap();
    let line4 = out.lines().find(|line| line.starts_with("4: ")).unwrap();
    assert!(line1.trim_end().ends_with("/// Opens it."), "{out}");
    assert!(line4.trim_end().ends_with('}'), "{out}");
    assert!(out.lines().any(|line| line == "…"), "{out}");
    let out = store.inspect(&["crate.helper"], true, None, usize::MAX).unwrap();
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

/// A graph `.rs` operand is refused: a source file is read by symbol (D-181).
#[test]
fn a_rust_file_operand_reaches_orientation() {
    let (_root, store) = sample();
    let out = orientation::view(&store, &toml::Value::Table(Default::default()), &["view", "src/lib.rs::open"], usize::MAX);
    let Some(Err(f)) = out else { panic!("a graph path is refused, not viewed") };
    assert!(f.0.contains("E6001") && f.0.contains("view crate") && f.0.contains("inspect crate."), "{}", f.0);
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
    assert!(out.contains("a::agents.Panel::show"), "{out}");
    assert!(out.contains("the module is the one holding the `impl`"), "{out}");
    assert!(out.contains("generics may be left out (`Refs::new`)"), "{out}");
    assert!(out.contains("`find <text>` searches Rust source; `grep` searches other files only."), "{out}");
    assert!(!out.contains("<module>.tests::<name>"), "{out}");
}

/// A Gleam session doesn't carry the Rust lines.
#[test]
fn a_gleam_session_does_not_carry_the_brief() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let out = p.session(&[]).unwrap();
    assert!(!out.contains("Rust in Solace"), "{out}");
    assert!(!out.contains("<module>.<Type>::<fn>"), "{out}");
    assert!(!out.contains("<module>.tests::<name>"), "{out}");
    assert!(!out.contains("`find <text>` searches Rust source; `grep` searches other files only."), "{out}");
}

/// `session start --task` carries the Rust lines too.
#[test]
fn a_task_session_carries_the_brief() {
    let (_root, store) = sample();
    let out = store.task_session("change helper", None, "").unwrap();
    assert!(out.contains("Rust in Solace: every item has its own name"), "{out}");
    assert!(out.contains("a::agents.Panel::show"), "{out}");
    assert!(out.contains("the module is the one holding the `impl`"), "{out}");
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
const REACH_LIB: &str = r#"//! Sample.

pub mod other;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn sub(a: i32, b: i32) -> i32 {
    a - b
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
"#;
const REACH_TESTS: &str = r#"use crate::add;

fn helper() -> i32 {
    add(1, 1)
}

#[test]
fn add_works() {
    assert_eq!(add(1, 2), 3);
}

#[test]
fn helper_works() {
    assert_eq!(helper(), 2);
}
"#;
const REACH_OTHER: &str = r#"pub fn twice(n: i32) -> i32 {
    n * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twice_works() {
        assert_eq!(twice(1), 2);
    }
}
"#;
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

/// A crate with a real method and a real test in an inline `mod tests`.
const INLINE: &str = r#"pub struct Store;

impl Store {
    pub fn open() -> Self {
        Store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens() {
        Store::open();
    }
}
"#;

/// A crate with a free function and a top-level `#[test]` outside `mod tests`.
const FREE: &str = r#"pub fn only() -> i32 {
    1
}

#[test]
fn outside() {}
"#;

/// A Rust session names a real inline test and the header that adds another.
#[test]
fn a_rust_session_names_an_inline_test() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", INLINE)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("crate.Store::open"), "{out}");
    assert!(out.contains("crate.tests::opens"), "{out}");
    assert!(out.contains("*** Add: crate::tests.<test>"), "{out}");
}

/// A crate with no method and no inline test leaves those lines out, even
/// with a top-level `#[test]`.
#[test]
fn a_rust_session_without_examples_omits_their_lines() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", FREE)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(!out.contains("<module>.<Type>::<fn>"), "{out}");
    assert!(!out.contains("<module>.tests::<name>"), "{out}");
    assert!(out.contains("`find <text>` searches Rust source; `grep` searches other files only."), "{out}");
}

/// A crate with a real test in a nested inline `mod tests`, after a `tests`
/// module that has no test.
const NESTED: &str = r#"#[cfg(test)]
mod tests {
    fn helper() {}
}

#[cfg(test)]
mod outer {
    #[cfg(test)]
    mod tests {
        #[tokio::test]
        async fn works() {}
    }
}
"#;

/// A test-only method comes before a real one, so the brief must skip it.
const METHOD_CHOICE: &str = r#"pub struct First;

impl First {
    #[cfg(test)]
    pub fn test_only() -> Self {
        First
    }
}

pub struct Second;

impl Second {
    pub fn real() -> Self {
        Second
    }
}
"#;

/// A session finds a real test in a nested inline `mod tests`, even when an
/// earlier `tests` module has none, and names the full path to add one.
#[test]
fn a_rust_session_finds_a_nested_inline_test() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", NESTED)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(!out.contains("<module>.<Type>::<fn>"), "{out}");
    assert!(out.contains("crate.outer::tests::works"), "{out}");
    assert!(out.contains("*** Add: crate::outer::tests.<test>"), "{out}");
    assert!(out.contains("<module>.outer::tests::<name>"), "{out}");
}

/// A test-only `impl` method doesn't become the method example.
#[test]
fn a_rust_session_prefers_a_non_test_method() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", METHOD_CHOICE)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("crate.Second::real"), "{out}");
    assert!(!out.contains("crate.First::test_only"), "{out}");
}

/// An `impl` with only a test-only method: the brief still names it.
const TEST_ONLY_METHOD: &str = r#"pub struct Only;

impl Only {
    #[cfg(test)]
    pub fn test_only() -> Self {
        Only
    }
}
"#;

/// With no non-test method, a test-only `impl` method is the example.
#[test]
fn a_rust_session_falls_back_to_a_test_only_method() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", TEST_ONLY_METHOD)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("crate.Only::test_only"), "{out}");
}

/// A crate whose only test is under a non-`tests` module inside `mod tests`.
const DEEP: &str = r#"#[cfg(test)]
mod tests {
    mod inner {
        #[test]
        fn works() {}
    }
}
"#;

/// The ancestor walk finds `tests` above a non-`tests` module.
#[test]
fn a_rust_session_finds_a_test_below_a_module_inside_tests() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", DEEP)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("crate.tests::inner::works"), "{out}");
    assert!(out.contains("*** Add: crate::tests.<test>"), "{out}");
}

/// A wrong separator still lists the nearest stored names: the same leaf and
/// the names in the same module.
#[test]
fn a_wrong_separator_still_suggests_the_nearest_names() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    for miss in ["crate::hel", "crate.tests.name"] {
        let e = store.resolve(&ix, miss).unwrap_err();
        assert!(e.0.contains("crate.helper"), "{miss}: {}", e.0);
        assert!(e.0.contains("crate.Store"), "{miss}: {}", e.0);
        assert!(e.0.contains("<module>.<container>::<name>"), "{miss}: {}", e.0);
    }
}

/// A common leaf stops the hint at ten names instead of scanning every symbol.
#[test]
fn a_common_leaf_lists_at_most_ten_names() {
    let lib: String = (0..12).map(|n| format!("pub fn item{n}() -> i32 {{\n    {n}\n}}\n")).collect::<Vec<_>>().join("\n");
    let (_root, store) = crate::testing::rust_project(&lib);
    let ix = store.index().unwrap();
    let hint = similar_hint(&ix, "crate.item");
    assert_eq!(hint.matches("crate.item").count(), 10, "{hint}");
}

/// A generic method's name may leave out its type's generics (D-193).
#[test]
fn a_view_name_may_leave_out_generics() {
    let lib = concat!(
        "pub struct Refs<'a> {\n",
        "    text: &'a str,\n",
        "}\n\n",
        "impl<'a> Refs<'a> {\n",
        "    pub fn new(text: &'a str) -> Self {\n",
        "        Refs { text }\n",
        "    }\n",
        "}\n"
    );
    let (_root, store) = crate::testing::rust_project(lib);
    let out = store.view(&["crate.Refs::new"], false, usize::MAX).unwrap();
    assert!(out.contains("crate.Refs<'a>::new (fn)"), "{out}");
}

/// An exact name beats a generic-free one, even when both are in the store.
#[test]
fn an_exact_name_wins_over_a_loose_match() {
    let lib = "pub mod a;\npub mod b;\n";
    let a =
        "pub struct Foo<T> {\n    value: T,\n}\n\nimpl<T> Foo<T> {\n    pub fn new(value: T) -> Self {\n        Foo { value }\n    }\n}\n";
    let b = "pub struct Foo;\n\nimpl Foo {\n    pub fn new() -> Self {\n        Foo\n    }\n}\n";
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", lib), ("src/a.rs", a), ("src/b.rs", b)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let ix = store.index().unwrap();
    let i = store.resolve(&ix, "Foo::new").unwrap();
    assert_eq!(ix.syms[i].qname(), "b.Foo::new");
}

/// Two generic impls with one method are ambiguous, and E1002 lists both full
/// names (D-193).
#[test]
fn an_ambiguous_generic_method_lists_both_names() {
    let lib = concat!(
        "pub struct Foo<T> {\n",
        "    value: T,\n",
        "}\n\n",
        "impl Foo<u8> {\n",
        "    pub fn new() -> Self {\n",
        "        Foo { value: 0 }\n",
        "    }\n",
        "}\n\n",
        "impl Foo<i32> {\n",
        "    pub fn new() -> Self {\n",
        "        Foo { value: 0 }\n",
        "    }\n",
        "}\n"
    );
    let (_root, store) = crate::testing::rust_project(lib);
    let ix = store.index().unwrap();
    let e = store.resolve(&ix, "crate.Foo::new").unwrap_err();
    assert!(e.0.contains("E1002"), "{}", e.0);
    assert!(e.0.contains("crate.Foo<u8>::new"), "{}", e.0);
    assert!(e.0.contains("crate.Foo<i32>::new"), "{}", e.0);
}

/// Only a generic group after an identifier is dropped; a trait impl's name,
/// which starts with `<`, stays whole.
#[test]
fn without_generics_leaves_a_trait_impl_whole() {
    assert_eq!(without_generics("Refs<'a>::new"), "Refs::new");
    assert_eq!(without_generics("<Failure as From<u8>>::from"), "<Failure as From<u8>>::from");
}

#[test]
fn a_module_prefix_names_view() {
    let (_root, store) = sample();
    let ix = store.index().unwrap();
    let e = store.resolve(&ix, "module:crate").unwrap_err().0;
    assert!(e.contains("E1001 unknown name: module:crate"), "{e}");
    assert!(e.contains("`module:` names a module, not a symbol: use `view crate`"), "{e}");
}

/// A module and a function of one name are both named, so `view` asks by kind.
#[test]
fn a_module_and_a_function_of_one_name_are_named_by_kind() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", "mod g {\n    pub fn inside() {}\n}\n\npub fn g() {}\n")]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let err = store.view(&["crate.g"], false, usize::MAX).unwrap_err().0;
    assert!(err.starts_with("E1002") && err.contains("(mod)") && err.contains("(fn)"), "{err}");
}

#[test]
fn view_shows_module_header() {
    let source = "//! A sample crate.\nuse std::fmt;\n\npub fn answer() -> i32 {\n    42\n}\n";
    let (root, store) = testing::rust_project(source);
    let outline = store.view(&["crate"], false, usize::MAX).unwrap();
    assert!(outline.contains("//! A sample crate."), "{outline}");
    assert!(!outline.contains("use std::fmt;") && !outline.contains("42"), "{outline}");

    let args = ["view", "crate", "--header"].map(str::to_string);
    let header =
        with_root(&root, || run_inner(&root, &root, &args, &mut || Ok(String::new()), None, usize::MAX)).unwrap();
    assert!(header.contains("//! A sample crate.") && header.contains("use std::fmt;"), "{header}");
    assert!(header.contains("pub fn answer() -> i32") && !header.contains("42"), "{header}");

    let body_args = ["view", "crate", "--bodies", "--header"].map(str::to_string);
    let body =
        with_root(&root, || run_inner(&root, &root, &body_args, &mut || Ok(String::new()), None, usize::MAX))
            .unwrap();
    assert_eq!(body, source);

    let missing = ["view", "missing", "--header"].map(str::to_string);
    let error = with_root(&root, || run_inner(&root, &root, &missing, &mut || Ok(String::new()), None, usize::MAX))
        .unwrap_err();
    assert!(error.0.contains("unknown name"), "{}", error.0);
}

/// `--header` keeps a Rust module's items nested under their `impl`, as the
/// ordinary outline does.
#[test]
fn view_header_keeps_a_rust_module_outline_nested() {
    let source = concat!(
        "//! A sample crate.\n",
        "use std::fmt;\n\n",
        "pub struct Store;\n\n",
        "impl Store {\n",
        "    /// Opens it.\n",
        "    pub fn open() -> Self {\n",
        "        Store\n",
        "    }\n",
        "}\n"
    );
    let (_root, store) = testing::rust_project(source);
    let outline = store.view(&["crate"], false, usize::MAX).unwrap();
    let header = store.view_with_header(&["crate"], false, true, usize::MAX).unwrap();
    assert!(header.contains("//! A sample crate.") && header.contains("use std::fmt;"), "{header}");
    assert!(header.contains("\n  pub fn open() -> Self"), "{header}");
    assert!(!header.contains("        Store"), "{header}");
    let header_items = header.split_once("pub struct Store").map(|(_, rest)| rest).unwrap();
    let outline_items = outline.split_once("pub struct Store").map(|(_, rest)| rest).unwrap();
    assert_eq!(header_items, outline_items, "{header}\n---\n{outline}");
}

/// A Gleam module shows its `////` comment and imports once, with no bodies.
#[test]
fn view_header_shows_a_gleam_comment_and_import_once() {
    let source = "//// A sample module.\n\nimport gleam/io\n\npub fn main() -> Int {\n  io.debug(1)\n  42\n}\n";
    let project = project(&[("src/app.gleam", source)]);
    let header = project.view_with_header(&["app"], false, true, usize::MAX).unwrap();
    assert_eq!(header.matches("//// A sample module.").count(), 1, "{header}");
    assert_eq!(header.matches("import gleam/io").count(), 1, "{header}");
    assert!(header.contains("pub fn main() -> Int"), "{header}");
    assert!(!header.contains("42"), "{header}");
}

/// A test run that uses a crate the manifest doesn't declare names the
/// Cargo.toml change, not rustc's `cargo add` hint (D-210).
#[test]
fn an_undeclared_crate_in_a_test_names_cargo_toml() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = crate::testing::rust_project(
        "use foo_not_a_dep::Bar;\n\npub fn one() -> i32 {\n    1\n}\n\n#[test]\nfn it_works() {\n    assert_eq!(one(), 1);\n}\n",
    );
    let err = store.test(&["--all"]).unwrap_err().0;
    assert!(err.contains("`foo_not_a_dep` isn't a dependency of this project; add it to `Cargo.toml` first."), "{err}");
    assert!(!err.contains("cargo add"), "{err}");
}

/// A move reruns the tests that reach the moved code, not every test in the
/// modules whose headers it changed.
#[test]
fn a_move_runs_only_the_tests_that_reach_it() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = rust_project(concat!(
        "pub fn one() -> i32 {\n    1\n}\n\n",
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn go() -> i32 {\n    one() + two()\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn go_works() {\n        assert_eq!(go(), 3);\n    }\n\n",
        "    #[test]\n    fn two_works() {\n        assert_eq!(two(), 2);\n    }\n}\n",
    ));
    let all = store.test(&[]).unwrap();
    assert!(all.starts_with("Ran all 2 tests."), "{all}");
    store.extract(&["crate.one", "--to", "helpers"], usize::MAX).unwrap();
    let out = store.test(&[]).unwrap();
    assert!(out.starts_with("Ran 1 of 2 tests;"), "{out}");
    let ix = store.index().unwrap();
    let text = module_text(&ix, ix.modules.iter().find(|m| m.path == "crate").unwrap());
    let broken = text.replace(" + two()\n}", " + two() + 1\n}");
    assert_ne!(broken, text, "go's body was not found to break");
    store.submit(Target::Module("crate"), &broken).unwrap();
    let out = store.test(&[]).unwrap_err().0;
    assert!(out.starts_with("Ran 1 of 2 tests;"), "{out}");
    assert!(out.contains("go_works"), "the failing test is not named: {out}");
    assert!(!out.contains("two_works"), "two_works ran instead of go_works: {out}");
}

/// A test reaching a process-spawning helper runs when its reach changes, not
/// on every run.
#[test]
fn a_process_test_skips_when_nothing_it_reaches_changed() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = rust_project(concat!(
        "pub fn spawn() -> bool {\n",
        "    std::process::Command::new(std::env::current_exe().unwrap())\n",
        "        .arg(\"--list\")\n",
        "        .status()\n",
        "        .unwrap()\n",
        "        .success()\n",
        "}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn spawn_works() {\n        assert!(spawn());\n    }\n}\n",
    ));
    let all = store.test(&[]).unwrap();
    assert!(all.starts_with("Ran all 1 tests."), "{all}");
    let out = store.test(&[]).unwrap();
    assert!(out.starts_with("Ran 0 of 1 tests;"), "{out}");
    store
        .submit_symbols(
            Some("crate.spawn"),
            None,
            concat!(
                "pub fn spawn() -> bool {\n",
                "    std::process::Command::new(std::env::current_exe().unwrap())\n",
                "        .arg(\"--list\")\n",
                "        .env(\"SOLACE_TEST_PROBE\", \"1\")\n",
                "        .status()\n",
                "        .unwrap()\n",
                "        .success()\n",
                "}\n",
            ),
        )
        .unwrap();
    let out = store.test(&[]).unwrap();
    assert!(out.starts_with("Ran all 1 tests."), "{out}");
}

/// The CLI reply keeps the reference, while the brief's copy carries it above the reply.
#[test]
fn a_cli_session_start_keeps_the_reference() {
    let (_root, store) = sample();
    let reference = crate::reference::agent_reference(true);
    assert!(store.task_session("change helper", Some("t1"), "").unwrap().contains(&reference));
    assert!(!store.brief_task_session("change helper", Some("t1"), "").unwrap().contains(&reference));
}

#[test]
fn find_matches_a_query_with_escaped_quotes() {
    let (_root, store) = crate::testing::rust_project("pub fn label() -> &'static str {\n    \"say \\\"kind\\\" now\"\n}\n");
    for query in [r#""\"kind\"""#, r#""kind""#, r#"say \"kind\" now"#] {
        let out = store.find(query).unwrap();
        assert!(out.contains("crate.label"), "{query}: {out}");
    }
}
