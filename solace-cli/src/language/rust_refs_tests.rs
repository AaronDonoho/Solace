//! Resolving Rust references against a module's `use` lines and the crate's
//! item names (D-170).

use crate::testing::{CARGO, Scratch};
use crate::{Index, Store, import};

/// A one-crate package with `lib` as `src/lib.rs`, read into a store.
fn index(lib: &str, extra: &[(&str, &str)]) -> (Scratch, Index) {
    let mut files: Vec<(&str, &str)> = vec![("Cargo.toml", CARGO), ("src/lib.rs", lib)];
    files.extend_from_slice(extra);
    let root = Scratch::new().with(&files);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let ix = store.index().unwrap();
    (root, ix)
}

fn index_of(ix: &Index, q: &str) -> usize {
    ix.syms.iter().position(|s| s.qname() == q).unwrap_or_else(|| panic!("no {q}"))
}

fn uses(ix: &Index, from: &str, to: &str) -> bool {
    ix.uses[index_of(ix, from)].contains(&index_of(ix, to))
}

fn tested(ix: &Index, q: &str) -> bool {
    crate::is_test(ix, index_of(ix, q))
}

/// A call inside `assert!`/`format!` is a token in a macro's arguments, so it
/// is a reference.
#[test]
fn a_call_inside_a_macro_is_a_reference() {
    let (_root, ix) = index(
        concat!(
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n",
            "    fn adds() {\n        assert_eq!(add(1, 2), 3);\n    }\n}\n",
        ),
        &[],
    );
    let s = index_of(&ix, "crate.tests::adds");
    let a = index_of(&ix, "crate.add");
    assert!(ix.uses[s].contains(&a));
    assert!(ix.at[s].iter().any(|&(t, _)| t == a), "the reference has a line");
}

/// `use … as` and `{…}` groups bind the names they rename.
#[test]
fn use_aliases_and_groups_resolve_to_their_items() {
    let (_root, ix) = index(
        concat!(
            "use crate::{one as uno, two};\n\n",
            "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\n",
            "pub fn both() -> i32 {\n    uno() + two()\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.both", "crate.one"));
    assert!(uses(&ix, "crate.both", "crate.two"));
}

/// A glob import reaches the parent's items, and marks the module's code as
/// test code.
#[test]
fn a_glob_import_reaches_the_parents_items() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[test]\n    fn t() {\n        assert_eq!(one(), 1);\n    }\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.tests::t", "crate.one"));
    assert!(tested(&ix, "crate.tests::t"));
    assert!(!tested(&ix, "crate.one"));
}

/// `Self::` reaches the enclosing impl's own items.
#[test]
fn self_reaches_the_impls_other_items() {
    let (_root, ix) = index(
        concat!(
            "pub struct Store;\n\nimpl Store {\n    pub fn a(&self) -> i32 {\n        self.b()\n    }\n\n",
            "    pub fn b(&self) -> i32 {\n        1\n    }\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.Store::a", "crate.Store::b"));
}

/// A method name links to every type that has it.
#[test]
fn a_method_name_links_to_every_type_that_has_it() {
    let (_root, ix) = index(
        concat!(
            "pub struct A;\npub struct B;\n\nimpl A {\n    pub fn show(&self) -> i32 {\n        1\n    }\n}\n\n",
            "impl B {\n    pub fn show(&self) -> i32 {\n        2\n    }\n}\n\n",
            "pub fn go(a: &A, b: &B) -> i32 {\n    a.show() + b.show()\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.go", "crate.A::show"));
    assert!(uses(&ix, "crate.go", "crate.B::show"));
}

/// A mention of a type reaches its trait impls' items, so a display behind
/// `format!` is seen.
#[test]
fn a_type_mention_reaches_its_trait_impls() {
    let (_root, ix) = index(
        concat!(
            "use std::fmt;\n\npub struct Store;\n\n",
            "impl fmt::Display for Store {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n",
            "        write!(f, \"s\")\n    }\n}\n\n",
            "pub fn label(s: &Store) -> String {\n    format!(\"{s}\")\n}\n",
        ),
        &[],
    );
    let names: Vec<String> = ix.uses[index_of(&ix, "crate.label")].iter().map(|&u| ix.syms[u].name.clone()).collect();
    assert!(names.iter().any(|n| n == "<Store as fmt::Display>::fmt"), "{names:?}");
}

/// A helper under `#[cfg(test)]` is test code, so it is not listed as a caller.
#[test]
fn a_helper_under_cfg_test_is_test_code() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    fn helper() -> i32 {\n        one()\n    }\n\n",
            "    #[test]\n    fn t() {\n        assert_eq!(helper(), 1);\n    }\n}\n",
        ),
        &[],
    );
    assert!(tested(&ix, "crate.tests::helper"));
    assert!(tested(&ix, "crate.tests::t"));
    assert!(!tested(&ix, "crate.one"));
}

/// An `impl` holding one `#[cfg(test)]` method is not itself test code: only
/// that method is.
#[test]
fn a_cfg_test_method_does_not_make_the_impl_test_code() {
    let (_root, ix) = index(
        concat!(
            "pub struct Store;\n\nimpl Store {\n    pub fn a(&self) -> i32 {\n        1\n    }\n\n",
            "    #[cfg(test)]\n    fn helper(&self) -> i32 {\n        2\n    }\n}\n",
        ),
        &[],
    );
    assert!(!tested(&ix, "crate.Store::a"));
    assert!(tested(&ix, "crate.Store::helper"));
}

/// A mention of `#[cfg(test)]` or `#[test]` in a doc comment or a string is
/// not an attribute: the item is not test code.
#[test]
fn a_mention_of_a_test_attribute_is_not_an_attribute() {
    let (_root, ix) = index(
        r##"/// A note on `#[cfg(test)]` code.
pub fn docs() -> &'static str {
    r#"
#[test]
fn x() {}
"#
}
"##,
        &[],
    );
    assert!(!tested(&ix, "crate.docs"));
    assert!(!crate::reach::is_test_fn(&ix, index_of(&ix, "crate.docs")));
}

/// An attribute spanning lines before `#[test]` is still an attribute, so the
/// function is a test.
#[test]
fn a_multiline_attribute_before_test_keeps_it_a_test() {
    let (_root, ix) = index(
        concat!(
            "pub fn add() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[should_panic(\n        expected = \"nope\"\n    )]\n    #[test]\n",
            "    fn boom() {\n        assert_eq!(add(), 2);\n    }\n}\n",
        ),
        &[],
    );
    assert!(crate::reach::is_test_fn(&ix, index_of(&ix, "crate.tests::boom")));
}

/// `#[cfg(all(test, …))]` is test-only, so its items are test code.
#[test]
fn an_all_test_cfg_is_test_code() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(all(test, unix))]\nmod tests {\n    use super::*;\n\n",
            "    fn helper() -> i32 {\n        one()\n    }\n}\n",
        ),
        &[],
    );
    assert!(tested(&ix, "crate.tests::helper"));
    assert!(!tested(&ix, "crate.one"));
}

/// `#[tokio::test]` is a test, so test reach selects it.
#[test]
fn a_path_test_attribute_is_a_test() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[tokio::test]\n    async fn one_works() {\n        assert_eq!(one(), 1);\n    }\n}\n",
        ),
        &[],
    );
    let t = index_of(&ix, "crate.tests::one_works");
    assert!(crate::reach::is_test_fn(&ix, t));
    assert!(crate::reach::test_symbols(&ix).contains(&t));
}

/// A header line's attributes are read one group at a time: `#[cfg(test)]` beside
/// another attribute still marks its `mod` test-only, and a one-line
/// `#[cfg(test)] mod x;` counts too (D-170).
#[test]
fn a_header_line_with_attributes_is_read() {
    let (_root, ix) = index(
        "#[cfg(test)] #[path = \"t.rs\"]\nmod t;\n\n#[cfg(test)] mod u;\n\npub fn one() -> i32 {\n    1\n}\n",
        &[("src/t.rs", "pub fn a() -> i32 {\n    2\n}\n"), ("src/u.rs", "pub fn b() -> i32 {\n    3\n}\n")],
    );
    assert!(tested(&ix, "t.a"));
    assert!(tested(&ix, "u.b"));
    assert!(!tested(&ix, "crate.one"));
}

/// A `test` only inside a nested `any`, or a feature named `test`, does not
/// make an item test-only: it is built outside tests too.
#[test]
fn a_nested_or_named_test_cfg_is_not_test_only() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(all(unix, any(test, feature = \"x\")))]\nfn nested() -> i32 {\n    one()\n}\n\n",
            "#[cfg(all(feature = \"test\", unix))]\nfn named() -> i32 {\n    one()\n}\n\n",
            "#[cfg(any(test, unix))]\nfn either() -> i32 {\n    one()\n}\n",
        ),
        &[],
    );
    assert!(!tested(&ix, "crate.nested"));
    assert!(!tested(&ix, "crate.named"));
    assert!(!tested(&ix, "crate.either"));
}

/// A bracket inside a string does not unbalance an attribute, so the `#[test]`
/// below it is still seen.
#[test]
fn a_bracket_in_a_string_does_not_hide_the_next_attribute() {
    let (_root, ix) = index(
        concat!(
            "pub fn add() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[should_panic(expected = \"[\")]\n    #[test]\n",
            "    fn boom() {\n        assert_eq!(add(), 2);\n    }\n}\n",
        ),
        &[],
    );
    assert!(crate::reach::is_test_fn(&ix, index_of(&ix, "crate.tests::boom")));
}

/// A macro call reaches the macro it names.
#[test]
fn a_macro_call_reaches_its_macro() {
    let (_root, ix) =
        index("macro_rules! twice {\n    ($n:expr) => {\n        $n * 2\n    };\n}\n\npub fn go() -> i32 {\n    twice!(3)\n}\n", &[]);
    assert!(uses(&ix, "crate.go", "crate.twice"));
}

/// An attribute written on the same line as its item still counts: a one-line
/// `#[cfg(test)] mod` and a one-line `#[cfg(test)] fn` are test code (D-170).
#[test]
fn a_one_line_cfg_test_item_is_test_code() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)] mod tests { fn helper() -> i32 { 1 } }\n\n",
            "#[cfg(test)] fn alone() -> i32 {\n    2\n}\n",
        ),
        &[],
    );
    assert!(tested(&ix, "crate.tests::helper"));
    assert!(tested(&ix, "crate.alone"));
}

/// Every attribute on one line is seen: a `#[test]` after another attribute is
/// a test, and a one-line `#[cfg(test)] #[allow(…)] mod` is test code (D-170).
#[test]
fn attributes_on_one_line_are_all_seen() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[allow(dead_code)] #[test]\nfn t() {\n    assert_eq!(one(), 1);\n}\n\n",
            "#[cfg(test)] #[allow(dead_code)] mod tests {\n    fn helper() -> i32 {\n        2\n    }\n}\n",
        ),
        &[],
    );
    assert!(crate::reach::is_test_fn(&ix, index_of(&ix, "crate.t")));
    assert!(tested(&ix, "crate.tests::helper"));
}

/// A bracket inside a raw string or a char literal doesn't join the attributes
/// after it (D-170).
#[test]
fn a_bracket_in_a_literal_does_not_hide_the_next_attribute() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[doc = r#\"a\"[b\"#]\n    #[test]\n    fn raw() {\n        assert_eq!(one(), 1);\n    }\n\n",
            "    #[allow('[')]\n    #[test]\n    fn quoted() {\n        assert_eq!(one(), 1);\n    }\n}\n",
        ),
        &[],
    );
    assert!(crate::reach::is_test_fn(&ix, index_of(&ix, "crate.tests::raw")));
    assert!(crate::reach::is_test_fn(&ix, index_of(&ix, "crate.tests::quoted")));
}

/// A header's `#[cfg(all(test, …))] mod x;` makes the file's module test-only,
/// as an item's does (D-170).
#[test]
fn a_header_all_test_cfg_module_is_test_code() {
    let (_root, ix) = index(
        "#[cfg(all(test, unix))]\nmod tests;\n\npub fn one() -> i32 {\n    1\n}\n",
        &[("src/tests.rs", "pub fn helper() -> i32 {\n    2\n}\n")],
    );
    assert!(tested(&ix, "tests.helper"));
    assert!(!tested(&ix, "crate.one"));
}

/// `#[rstest]` marks a test, so test reach selects it (D-170).
#[test]
fn a_rstest_attribute_is_a_test() {
    let (_root, ix) = index(
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[rstest]\n    fn one_works() {\n        assert_eq!(one(), 1);\n    }\n}\n",
        ),
        &[],
    );
    let t = index_of(&ix, "crate.tests::one_works");
    assert!(crate::reach::is_test_fn(&ix, t));
    assert!(crate::reach::test_symbols(&ix).contains(&t));
}
