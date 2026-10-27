//! Resolving Rust references against a module's `use` lines and the crate's
//! item names (D-170).

use crate::testing::{CARGO, Scratch};
use crate::{Index, Store, import};

/// A one-crate package with `lib` as `src/lib.rs`, read into a store.
fn index(lib: &str, extra: &[(&str, &str)]) -> (Scratch, Index) {
    let (root, store) = store_of(lib, extra);
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
    assert!(crate::reach::closure(&ix, s).contains(&a), "the test reaches the call inside the macro");
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

/// A local, a field, a method call and a closure parameter that share an
/// item's name are never rewrite sites.
#[test]
fn a_local_field_method_or_closure_named_like_an_item_is_no_site() {
    let (_root, ix) = index(
        concat!(
            "pub struct Holder {\n    pub make: i32,\n}\n\n",
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "impl Holder {\n    pub fn make(&self) -> i32 {\n        2\n    }\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    let make = 1;\n",
            "    let h = Holder { make: 2 };\n",
            "    let f = |make: i32| make + 1;\n",
            "    make + h.make + h.make() + f(1)\n",
            "}\n",
        ),
        &[],
    );
    let make = index_of(&ix, "crate.make");
    let method = index_of(&ix, "crate.Holder::make");
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, make).is_empty());
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, method).is_empty());
}

/// A `match` arm binding and a destructured parameter bind only in their own
/// bodies, so neither is a rewrite site.
#[test]
fn a_match_binding_or_destructured_parameter_is_no_site() {
    let (_root, ix) = index(
        concat!(
            "pub struct Holder {\n    pub make: i32,\n}\n\n",
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn destructured(Holder { make }: Holder) -> i32 {\n    make\n}\n\n",
            "pub fn matched(value: Option<i32>) -> i32 {\n",
            "    match value {\n",
            "        Some(make) => make,\n",
            "        None => 0,\n",
            "    }\n",
            "}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// `vendor::make` and `std::collections::HashMap` have no crate item at their
/// head, so they never fall back to a crate item with the same leaf.
#[test]
fn a_foreign_path_does_not_resolve_to_a_crate_item() {
    let (_root, ix) = index(
        concat!(
            "pub struct HashMap;\n\n",
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    let _map = std::collections::HashMap::new();\n",
            "    vendor::make() + make()\n",
            "}\n",
        ),
        &[],
    );
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make"));
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].target, index_of(&ix, "crate.make"));
    assert_eq!(sites[0].via, super::Via::Own);
}

/// A call before a shadowing `let`, or after the block that held it, still
/// names the item.
#[test]
fn a_call_before_a_shadowing_let_and_after_its_block_is_a_site() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    let before = make();\n",
            "    {\n        let make = 2;\n        let inside = make;\n    }\n",
            "    let after = make();\n",
            "    before + after\n",
            "}\n",
        ),
        &[],
    );
    let make = index_of(&ix, "crate.make");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, make);
    let go = index_of(&ix, "crate.go");
    let hits: Vec<_> = sites.iter().filter(|s| s.symbol == Some(go)).collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|s| s.via == super::Via::Own));
}

/// A bare name an import binds, an `as` alias, a group member, and
/// `self::`/`super::`/`crate::` use paths are all reported.
#[test]
fn imports_aliases_and_groups_are_sites() {
    let (_root, ix) = index(
        concat!(
            "mod a;\n\nuse crate::a::{make as m, Task as T};\n\n",
            "pub fn root_make() -> i32 {\n    3\n}\n\n",
            "pub fn go(value: T) -> i32 {\n    m()\n}\n",
        ),
        &[(
            "src/a.rs",
            concat!(
                "use super::root_make;\nuse self::Task;\n\n",
                "pub struct Task;\n\npub fn make() -> i32 {\n    root_make()\n}\n\n",
                "impl Task {\n    pub fn new() -> i32 {\n        2\n    }\n}\n",
            ),
        )],
    );
    let header = ix.modules.iter().find(|m| m.path == "crate").unwrap().header.as_str();
    let sites = super::use_sites(&ix.syms, &ix.modules, "crate").unwrap();
    let make = index_of(&ix, "a.make");
    let task = index_of(&ix, "a.Task");
    assert_eq!(sites.len(), 2);
    let make_site = sites.iter().find(|s| s.target == make).unwrap();
    assert_eq!(make_site.via, super::Via::Import("m".into()));
    assert_eq!(&header[make_site.range.clone()], "make");
    assert_eq!(make_site.outside, 2);
    assert_eq!(&header[make_site.alias.clone().unwrap()], "as m");
    let task_site = sites.iter().find(|s| s.target == task).unwrap();
    assert_eq!(task_site.via, super::Via::Import("T".into()));
    assert_eq!(&header[task_site.alias.clone().unwrap()], "as T");

    let sub = ix.modules.iter().find(|m| m.path == "a").unwrap().header.as_str();
    let sub_sites = super::use_sites(&ix.syms, &ix.modules, "a").unwrap();
    assert_eq!(sub_sites.len(), 2);
    let root_make = index_of(&ix, "crate.root_make");
    let parent = sub_sites.iter().find(|s| s.target == root_make).unwrap();
    assert_eq!(&sub[parent.range.clone()], "super::root_make");
    assert_eq!(parent.outside, 0);
    let own = sub_sites.iter().find(|s| s.target == task).unwrap();
    assert_eq!(&sub[own.range.clone()], "self::Task");

    let go = index_of(&ix, "crate.go");
    let make_mentions = super::rewrite_sites(&ix.syms, &ix.modules, make);
    assert!(make_mentions.iter().any(|s| s.symbol == Some(go) && s.via == super::Via::Import("m".into())));
    let task_mentions = super::rewrite_sites(&ix.syms, &ix.modules, task);
    assert!(task_mentions.iter().any(|s| s.symbol == Some(go) && s.via == super::Via::Import("T".into())));
}

/// Field mentions stay in the over-approximating graph, so test reach does not
/// change even though they are not rewrite sites.
#[test]
fn a_field_mention_still_reaches_an_item_for_the_graph() {
    let (_root, ix) = index("pub struct Holder;\npub fn make() -> i32 {\n    1\n}\npub fn go(h: Holder) -> i32 {\n    h.make\n}\n", &[]);
    assert!(uses(&ix, "crate.go", "crate.make"));
}

/// `if let`, `while let` and `for` pattern bindings cover their own blocks.
#[test]
fn if_let_while_let_and_for_bindings_are_no_sites() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go(values: Vec<Option<i32>>) -> i32 {\n",
            "    if let Some(make) = values.first() {\n        let _ = make;\n    }\n",
            "    while let Some(make) = values.first() {\n        let _ = make;\n        break;\n    }\n",
            "    for make in values {\n        let _ = make;\n    }\n",
            "    0\n",
            "}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// A local `fn`, `const`, `static`, `type`, `struct`, `enum` or `trait`
/// shadows a same-named item in its block.
#[test]
fn local_declarations_named_like_an_item_are_no_sites() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    {\n        fn make() -> i32 { 2 }\n        let _ = make();\n    }\n",
            "    {\n        const make: i32 = 2;\n        let _ = make;\n    }\n",
            "    {\n        static make: i32 = 2;\n        let _ = make;\n    }\n",
            "    {\n        type make = i32;\n        let _: make = 0;\n    }\n",
            "    {\n        struct make;\n        let _ = make;\n    }\n",
            "    {\n        enum make { A }\n        let _: make = make::A;\n    }\n",
            "    {\n        trait make {}\n        let _: &dyn make = &();\n    }\n",
            "    0\n",
            "}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// A `use` row nested in an inline module is reported with its own symbol range.
#[test]
fn a_use_symbol_inside_a_module_is_a_site() {
    let (_root, ix) = index(
        concat!(
            "pub mod inner {\n    pub fn go() -> i32 {\n        1\n    }\n\n",
            "    use crate::make;\n\n    pub fn more() -> i32 {\n        make()\n    }\n}\n\n",
            "pub fn make() -> i32 {\n    1\n}\n",
        ),
        &[],
    );
    let use_sym = ix.syms.iter().position(|s| s.kind_text == "use").expect("a use symbol");
    let sites = super::use_sites(&ix.syms, &ix.modules, &ix.syms[use_sym].module).unwrap();
    let make = index_of(&ix, "crate.make");
    assert!(sites.iter().any(|s| s.symbol == Some(use_sym) && s.target == make));
}

/// Qualified paths say they reached their item by path, including `self::`,
/// `super::` and `crate::`.
#[test]
fn qualified_self_super_and_crate_paths_resolve() {
    let (_root, ix) = index(
        concat!("mod a;\n\npub fn root_make() -> i32 {\n    1\n}\n\n", "pub fn go() -> i32 {\n    crate::a::make() + a::make()\n}\n",),
        &[("src/a.rs", "pub fn make() -> i32 {\n    2\n}\n\npub fn inner() -> i32 {\n    self::make() + super::root_make()\n}\n")],
    );
    let make = index_of(&ix, "a.make");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, make);
    assert_eq!(sites.iter().filter(|s| s.via == super::Via::Path).count(), 3);
    let root = index_of(&ix, "crate.root_make");
    let root_sites = super::rewrite_sites(&ix.syms, &ix.modules, root);
    assert!(root_sites.iter().any(|s| s.via == super::Via::Path));
}

/// A binding covers the `for` or `if let` body, not the expression that
/// produces its value.
#[test]
fn for_and_if_let_conditions_are_sites() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> Option<i32> {\n    Some(1)\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    if let Some(make) = make() {\n        let _ = make;\n    }\n",
            "    for make in make().into_iter() {\n        let _ = make;\n    }\n",
            "    0\n",
            "}\n",
        ),
        &[],
    );
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make"));
    assert_eq!(sites.iter().filter(|s| s.symbol == Some(index_of(&ix, "crate.go"))).count(), 2);
}

/// `let make = make();` names the item in its initializer; a local `fn make`
/// declared later in a block shadows the whole block.
#[test]
fn a_let_initializer_is_a_site_and_a_later_local_item_shadows_its_block() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n    let make = make();\n    make\n}\n\n",
            "pub fn early() -> i32 {\n    let first = make();\n    fn make() -> i32 {\n        2\n    }\n    first\n}\n",
        ),
        &[],
    );
    let make = index_of(&ix, "crate.make");
    let go = index_of(&ix, "crate.go");
    let early = index_of(&ix, "crate.early");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, make);
    let in_go: Vec<_> = sites.iter().filter(|s| s.symbol == Some(go)).collect();
    assert_eq!(in_go.len(), 1);
    assert_eq!(&ix.syms[go].text[in_go[0].range.clone()], "make");
    assert!(ix.syms[go].text[in_go[0].range.end..].starts_with("()"));
    assert!(!sites.iter().any(|s| s.symbol == Some(early)));
}

/// A method's own name is not in bare scope inside its body.
#[test]
fn a_method_name_does_not_hide_a_free_fn() {
    let (_root, ix) = index(
        concat!(
            "pub fn parse() -> i32 {\n    1\n}\n\n",
            "pub struct Reader;\n\n",
            "impl Reader {\n    pub fn parse(&self) -> i32 {\n        parse()\n    }\n}\n",
        ),
        &[],
    );
    let parse = index_of(&ix, "crate.parse");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, parse);
    assert_eq!(sites.len(), 1);
    assert_ne!(sites[0].symbol, Some(parse));
}

/// `'static`, `*const`, `fn()` types, a `union` method and a generic `Fn(…)`
/// bound declare nothing; a struct update's base is a site.
#[test]
fn keyword_types_and_struct_updates_bind_nothing() {
    let (_root, ix) = index(
        concat!(
            "pub struct Config;\n\n",
            "pub fn kinds(c: &'static Config, p: *const Config, f: fn() -> Config, s: &Config) -> Config {\n",
            "    s.union(s);\n    Config\n}\n\n",
            "pub fn value() -> u32 {\n    1\n}\n\n",
            "pub fn run<F: Fn(u32) -> u32>(value: F) -> u32 {\n    value(2)\n}\n\n",
            "pub struct Holder {\n    pub a: i32,\n    pub b: i32,\n}\n\n",
            "pub fn defaults() -> Holder {\n    Holder { a: 0, b: 0 }\n}\n\n",
            "pub fn update() -> Holder {\n    Holder { a: 1, ..defaults() }\n}\n",
        ),
        &[],
    );
    let config = index_of(&ix, "crate.Config");
    let kinds = index_of(&ix, "crate.kinds");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, config);
    assert_eq!(sites.iter().filter(|s| s.symbol == Some(kinds)).count(), 6);
    let value = index_of(&ix, "crate.value");
    let run = index_of(&ix, "crate.run");
    assert!(!super::rewrite_sites(&ix.syms, &ix.modules, value).iter().any(|s| s.symbol == Some(run)));
    let defaults = index_of(&ix, "crate.defaults");
    let update = index_of(&ix, "crate.update");
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, defaults).iter().any(|s| s.symbol == Some(update)));
}

/// `crate::a::Task::new()`, an imported `Task::new()` and `a::Kind::A` give a
/// site for the type; a group member's range starts at its own root.
#[test]
fn a_type_prefix_of_a_path_is_a_site() {
    let (_root, ix) = index(
        concat!(
            "mod a;\n\nuse crate::{a::Task, a::make};\n\n",
            "pub fn go() -> i32 {\n    let _k = a::Kind::A;\n    crate::a::Task::new() + Task::new() + make()\n}\n",
        ),
        &[(
            "src/a.rs",
            concat!(
                "pub struct Task;\n\npub enum Kind {\n    A,\n}\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "impl Task {\n    pub fn new() -> i32 {\n        2\n    }\n}\n",
            ),
        )],
    );
    let go = index_of(&ix, "crate.go");
    let text = &ix.syms[go].text;
    let task = index_of(&ix, "a.Task");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, task);
    let mut written: Vec<&str> = sites.iter().filter(|s| s.symbol == Some(go)).map(|s| &text[s.range.clone()]).collect();
    written.sort_unstable();
    assert_eq!(written, ["Task", "crate::a::Task"]);
    let kind = index_of(&ix, "a.Kind");
    assert!(ix.uses[go].contains(&kind), "{:?}", ix.uses[go]);
    let kinds = super::rewrite_sites(&ix.syms, &ix.modules, kind);
    assert_eq!(kinds.iter().map(|s| &text[s.range.clone()]).collect::<Vec<_>>(), ["a::Kind"]);
    let header = ix.modules.iter().find(|m| m.path == "crate").unwrap().header.as_str();
    let mut members: Vec<&str> =
        super::use_sites(&ix.syms, &ix.modules, "crate").unwrap().iter().map(|s| &header[s.range.clone()]).collect();
    members.sort_unstable();
    assert_eq!(members, ["a::Task", "a::make"]);
}

/// A bare name reached only through `use a::*;` says so.
#[test]
fn a_name_a_glob_brings_is_via_glob() {
    let (_root, ix) =
        index("mod a;\n\nuse a::*;\n\npub fn go() -> i32 {\n    make()\n}\n", &[("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n")]);
    let make = index_of(&ix, "a.make");
    let go = index_of(&ix, "crate.go");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, make);
    assert!(sites.iter().any(|s| s.symbol == Some(go) && s.via == super::Via::Glob));
}

/// A named import from another crate hides a glob that brings a crate item.
#[test]
fn a_named_external_import_hides_a_glob() {
    let (_root, ix) = index(
        "mod a;\n\nuse vendor::make;\nuse a::*;\n\npub fn go() -> i32 {\n    make()\n}\n",
        &[("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n")],
    );
    let make = index_of(&ix, "a.make");
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, make).is_empty());
}

/// A `use` inside a function body hides a same-named crate item after it.
#[test]
fn a_use_in_a_block_binds_its_name() {
    let (_root, ix) = index("pub fn make() -> i32 {\n    1\n}\n\npub fn go() -> i32 {\n    use vendor::make;\n    make()\n}\n", &[]);
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// The `for` of `impl Trait for X` or `for<'a>` binds nothing in a later loop.
#[test]
fn impl_headers_and_higher_ranked_types_are_no_loops() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn one() {\n    struct S;\n    trait Tr {}\n    impl Tr for S {}\n    drop(make);\n",
            "    for x in [1] {\n        let _ = make() + x;\n    }\n}\n\n",
            "pub fn two() {\n    let _f: for<'a> fn(&'a i32) = |_| {};\n    drop(make);\n",
            "    for x in [1] {\n        let _ = make() + x;\n    }\n}\n",
        ),
        &[],
    );
    assert_eq!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).len(), 4);
}

/// `&raw const make` borrows the item; it declares nothing.
#[test]
fn a_raw_const_borrow_is_a_site() {
    let (_root, ix) = index("pub static make: i32 = 1;\n\npub fn go() -> *const i32 {\n    &raw const make\n}\n", &[]);
    assert_eq!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).len(), 1);
}

/// A closure after `&` or a struct field's `:` binds its parameters.
#[test]
fn closures_after_a_borrow_or_a_field_bind_their_parameters() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\npub struct H {\n    pub h: fn(i32) -> i32,\n}\n\n",
            "pub fn go() -> i32 {\n    let f = &|make: i32| make + 1;\n    let g = H { h: |make| make };\n    f(2) + (g.h)(3)\n}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// A `let` named like a module hides the bare name, not `module::item`.
#[test]
fn a_local_value_does_not_hide_a_module_path() {
    let (_root, ix) = index(
        "mod store;\n\npub fn go() -> i32 {\n    let store = 1;\n    store::helper() + store\n}\n",
        &[("src/store.rs", "pub fn helper() -> i32 {\n    1\n}\n")],
    );
    assert_eq!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "store.helper")).len(), 1);
}

/// `use ` inside an attribute's string starts no `use` tree; `pub(crate) use` does.
#[test]
fn use_inside_a_line_is_no_use_tree() {
    let (_root, ix) = index(
        "#![doc = \"to use a::make;\"]\n\nmod a;\n\npub(crate) use a::make;\n",
        &[("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n")],
    );
    assert_eq!(super::use_sites(&ix.syms, &ix.modules, "crate").unwrap().len(), 1);
}

/// A `use crate::make;` inside a function leaves `make()` naming the item.
#[test]
fn a_block_use_of_a_crate_item_is_a_site() {
    let (_root, ix) = index("pub fn make() -> i32 {\n    1\n}\n\npub fn go() -> i32 {\n    use crate::make;\n    make()\n}\n", &[]);
    let go = index_of(&ix, "crate.go");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make"));
    assert_eq!(sites.iter().filter(|s| s.symbol == Some(go)).count(), 2);
    let import = sites.iter().find(|s| s.symbol == Some(go) && &ix.syms[go].text[s.range.clone()] == "crate::make").unwrap();
    assert_eq!(import.via, super::Via::Import("make".into()));
}

/// Block imports that resolve to another crate item hide the target's name.
#[test]
fn block_uses_of_other_items_hide_their_local_name() {
    let (_root, ix) = index(
        concat!(
            "mod a;\nmod other;\n\npub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    let one = {\n        use crate::other::make;\n        make()\n    };\n",
            "    let two = {\n        use a::make;\n        make()\n    };\n",
            "    let three = {\n        use std::hint::black_box as make;\n        make(one)\n    };\n",
            "    one + two + three\n}\n",
        ),
        &[("src/a.rs", "pub fn make() -> i32 {\n    2\n}\n"), ("src/other.rs", "pub fn make() -> i32 {\n    3\n}\n")],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// A block import that resolves to the target is an import site.
#[test]
fn block_uses_of_the_target_are_import_sites() {
    let (_root, ix) = index(
        "mod a;\n\npub fn make() -> i32 {\n    1\n}\n\npub fn go() -> i32 {\n    use a::make;\n    make()\n}\n",
        &[("src/a.rs", "pub fn make() -> i32 {\n    2\n}\n")],
    );
    let make = index_of(&ix, "a.make");
    let go = index_of(&ix, "crate.go");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, make);
    let import = sites.iter().find(|s| s.symbol == Some(go) && &ix.syms[go].text[s.range.clone()] == "a::make").unwrap();
    assert_eq!((import.target, import.via.clone()), (make, super::Via::Import("make".into())));
}

/// A use after an attribute, or inside a one-line inline module, has a tree.
#[test]
fn one_line_attribute_and_inline_module_uses_are_trees() {
    let text = "#[cfg(test)] use a::make;\nmod m { use b::more; }\n";
    let trees: Vec<&str> = super::use_trees(text).unwrap().into_iter().map(|(_, tree)| tree).collect();
    assert_eq!(trees, vec!["a::make", "b::more"]);
}

/// A closure after `<`, `!`, `+`, `-`, `*` or `[` binds its parameters.
#[test]
fn closures_after_operators_and_in_generic_contexts_bind_their_parameters() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "macro_rules! touch {\n    ($($t:tt)*) => {};\n}\n\n",
            "pub fn go() {\n",
            "    touch!(<|make| make>);\n",
            "    touch!(!|make| make);\n",
            "    touch!(+|make| make);\n",
            "    touch!(-|make| make);\n",
            "    touch!(*|make| make);\n",
            "    touch!([|make| make]);\n",
            "}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

#[test]
fn an_ambiguous_block_use_does_not_hide_the_target() {
    let (_root, mut ix) = index(
        concat!("mod x;\n\npub fn make() -> i32 {\n    1\n}\n\n", "pub fn go() -> i32 {\n    use crate::x::make;\n    make()\n}\n",),
        &[("src/x.rs", "pub fn make() -> i32 {\n    2\n}\n")],
    );
    let mut duplicate = ix.syms.iter().find(|s| s.qname() == "x.make").unwrap().clone();
    duplicate.id = "extra:x.make".into();
    duplicate.kind_text = "struct".into();
    ix.syms.push(duplicate);
    let go = index_of(&ix, "crate.go");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make"));
    assert!(sites.iter().any(|s| s.symbol == Some(go) && &ix.syms[go].text[s.range.clone()] == "make"));
}

#[test]
fn a_header_that_does_not_parse_is_reported() {
    let (_root, mut ix) = index("pub fn make() -> i32 {\n    1\n}\n", &[]);
    ix.modules.iter_mut().find(|m| m.path == "crate").unwrap().header = "use a::{;".into();
    let err = super::use_sites(&ix.syms, &ix.modules, "crate").unwrap_err();
    assert!(err.0.contains("imports in crate don't parse"), "{}", err.0);
}

#[test]
fn a_use_symbol_that_does_not_parse_is_reported() {
    let (_root, mut ix) = index(
        "pub mod inner {\n    pub fn go() -> i32 {\n        1\n    }\n\n    use crate::make;\n}\n\npub fn make() -> i32 {\n    1\n}\n",
        &[],
    );
    let i = ix.syms.iter().position(|s| s.kind_text == "use").unwrap();
    let module = ix.syms[i].module.clone();
    ix.syms[i].text = "use crate::{;".into();
    let err = super::use_sites(&ix.syms, &ix.modules, &module).unwrap_err();
    assert!(err.0.contains("don't parse"), "{}", err.0);
}

/// A local variable named like an import is not a mention of it.
#[test]
fn a_local_binding_is_no_mentioned_name() {
    let (_root, ix) = index("use vendor::make;\n\npub fn go() -> i32 {\n    let make = 1;\n    make\n}\n", &[]);
    let go = index_of(&ix, "crate.go");
    let names = super::references(&ix.syms, &ix.modules).mentioned;
    assert!(!names[go].contains("make"), "{:?}", names[go]);
}

/// An unqualified call reaches a sibling inside the same inline module.
#[test]
fn a_call_to_an_item_in_an_inline_module_resolves() {
    let (_root, ix) = index(
        concat!(
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    fn check(value: i32) {\n        assert_eq!(value, 1);\n    }\n\n",
            "    #[test]\n    fn make_works() {\n        check(1);\n    }\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.tests::make_works", "crate.tests::check"));
}

/// A test module's helper is seen from the test module only.
#[test]
fn a_test_helper_does_not_leak_to_its_enclosing_module() {
    let (_root, ix) = index(
        concat!(
            "fn helper() -> i32 {\n    1\n}\n\n",
            "fn production() -> i32 {\n    helper()\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    fn helper() -> i32 {\n        2\n    }\n\n",
            "    #[test]\n    fn works() {\n        assert_eq!(helper(), 2);\n    }\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.production", "crate.helper"));
    assert!(!uses(&ix, "crate.production", "crate.tests::helper"));
    assert!(uses(&ix, "crate.tests::works", "crate.tests::helper"));
    assert!(!uses(&ix, "crate.tests::works", "crate.helper"));
}

/// An impl belongs to the type its header names, not to a same-leaf type elsewhere.
#[test]
fn an_impl_owner_is_resolved_by_path() {
    let (_root, ix) = index(
        concat!(
            "pub struct Task;\n\nimpl Task {\n    pub fn local() {}\n}\n\n",
            "pub mod vendor {\n    pub struct Task;\n}\n\n",
            "impl vendor::Task {\n    pub fn extra() {}\n}\n",
        ),
        &[],
    );
    let local = ix.syms.iter().position(|s| s.name == "Task" && s.module == "crate").unwrap();
    let impls: Vec<usize> = (0..ix.syms.len()).filter(|&i| ix.syms[i].kind_text == "impl").collect();
    assert_eq!(impls.len(), 2);
    use crate::language::rust_refs::{ImplOwner, impl_owner_batch};
    let owners: Vec<ImplOwner> = impl_owner_batch(&ix.syms, &ix.modules, &impls);
    assert!(owners.iter().any(|o| matches!(o, ImplOwner::One(t) if *t == local)));
    assert!(owners.iter().any(|o| matches!(o, ImplOwner::One(t) if *t != local)));
}

/// A crafted index can have two same-named types for one impl owner.
#[test]
fn an_ambiguous_impl_owner_names_both_types() {
    let syms = vec![
        sym("type:a.Task", "Task", "struct Task;"),
        sym("type:b.Task", "Task", "struct Task;"),
        sym("impl:Task", "Task", "impl Task {}"),
    ];
    let owners = crate::language::rust_refs::impl_owner_batch(&syms, &[module()], &[2]);
    assert!(matches!(&owners[..], [crate::language::rust_refs::ImplOwner::Ambiguous(ids)] if ids == &vec![0, 1]), "{owners:?}");
}

fn sym(id: &str, name: &str, text: &str) -> crate::index::Sym {
    crate::index::Sym {
        id: id.to_string(),
        module: "crate".to_string(),
        name: name.to_string(),
        kind: crate::language::Kind::Type,
        kind_text: if text.starts_with("impl") { "impl" } else { "struct" }.to_string(),
        text: text.to_string(),
        parent: None,
        sep: None,
        tail: String::new(),
    }
}

fn module() -> crate::index::ModuleRow {
    crate::index::ModuleRow {
        path: "crate".to_string(),
        dir: "src".to_string(),
        header: String::new(),
        file: Some("lib.rs".to_string()),
        owns_dir: Some(false),
    }
}

#[test]
fn an_inline_modules_import_stays_in_that_module() {
    let (_root, ix) = index(
        concat!(
            "mod a;\n\n",
            "pub mod before {\n",
            "    use crate::a::Task;\n\n",
            "    pub fn inside() -> Task {\n        Task\n    }\n}\n\n",
            "pub mod after {\n",
            "    pub fn first() {}\n\n",
            "    use crate::a::Other;\n\n",
            "    pub fn inside() -> Other {\n        Other\n    }\n}\n\n",
            "pub fn outside() -> Task {\n    Task\n}\n\n",
            "pub fn outside_other() -> Other {\n    Other\n}\n",
        ),
        &[("src/a.rs", "pub struct Task;\npub struct Other;\n")],
    );
    assert!(uses(&ix, "crate.before::inside", "a.Task"));
    assert!(uses(&ix, "crate.after::inside", "a.Other"));
    assert!(!uses(&ix, "crate.outside", "a.Task"));
    assert!(!uses(&ix, "crate.outside_other", "a.Other"));
}

#[test]
fn self_and_super_in_inline_modules_resolve() {
    let (_root, ix) = index(
        concat!(
            "mod outer {\n",
            "    pub fn outer_make() -> i32 {\n        1\n    }\n\n",
            "    mod inner {\n",
            "        pub fn inner_make() -> i32 {\n            2\n        }\n\n",
            "        pub fn go() -> i32 {\n            self::inner_make() + super::outer_make()\n        }\n",
            "    }\n}\n",
        ),
        &[],
    );
    assert!(uses(&ix, "crate.outer::inner::go", "crate.outer::inner::inner_make"));
    assert!(uses(&ix, "crate.outer::inner::go", "crate.outer::outer_make"));
}

#[test]
fn calls_in_a_file_module_and_through_its_parents_use_resolve() {
    let (_root, ix) = index(
        "fn before() {}\n\nmod outer;\n",
        &[
            (
                "src/outer.rs",
                concat!(
                    "fn before() {}\n\n",
                    "fn local() {}\n\n",
                    "mod inner;\n\n",
                    "mod checks;\n\n",
                    "use inner::{helper};\n\n",
                    "pub fn parent_calls() {\n    helper();\n    local();\n}\n\n",
                    "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                    "    #[test]\n    fn test_calls() {\n        helper();\n    }\n}\n",
                ),
            ),
            ("src/outer/checks.rs", "use super::*;\n\n#[test]\nfn test_calls() {\n    helper();\n}\n"),
            ("src/outer/inner.rs", "pub fn helper() {}\n\npub fn local_calls() {\n    helper();\n}\n"),
        ],
    );
    assert_eq!(ix.syms[index_of(&ix, "outer.inner")].kind_text, "mod");
    assert!(!super::Refs::new(&ix.syms, &ix.modules).inline_modules.contains_key("crate::outer"));
    assert!(uses(&ix, "outer::inner.local_calls", "outer::inner.helper"));
    assert!(uses(&ix, "outer.parent_calls", "outer::inner.helper"));
    assert!(uses(&ix, "outer.parent_calls", "outer.local"));
    assert!(uses(&ix, "outer.tests::test_calls", "outer::inner.helper"));
    assert!(uses(&ix, "outer::checks.test_calls", "outer::inner.helper"));
}

#[test]
fn a_generic_binding_after_a_comma_is_not_a_mention() {
    let names: Vec<String> = super::scanned("fn f() -> impl Pair<Item = u8, Into = u16> { todo!() }")
        .into_iter()
        .filter_map(|spanned| match spanned.mention {
            super::Mention::Path(segs) => segs.first().cloned(),
            _ => None,
        })
        .collect();
    assert!(!names.contains(&"Item".to_string()) && !names.contains(&"Into".to_string()), "{names:?}");
}

fn store_of(lib: &str, extra: &[(&str, &str)]) -> (Scratch, Store) {
    let mut files: Vec<(&str, &str)> = vec![("Cargo.toml", CARGO), ("src/lib.rs", lib)];
    files.extend_from_slice(extra);
    let root = Scratch::new().with(&files);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    (root, store)
}

fn possible(ix: &Index, from: &str, to: &str) -> bool {
    ix.possible[index_of(ix, from)].iter().any(|&(t, _)| t == index_of(ix, to))
}

#[test]
fn a_same_named_item_in_another_module_is_not_a_use() {
    let (_root, store) = store_of(
        "mod other;\nuse other::*;\n\npub fn batch() -> i32 {\n    1\n}\n\npub fn go() -> i32 {\n    batch()\n}\n",
        &[("src/other.rs", "pub fn batch() -> i32 {\n    2\n}\n")],
    );
    let ix = store.index().unwrap();
    assert!(uses(&ix, "crate.go", "crate.batch"));
    assert!(!uses(&ix, "crate.go", "other.batch"));
    assert!(!possible(&ix, "crate.go", "other.batch"));
    let reach = crate::reach::closure(&ix, index_of(&ix, "crate.go"));
    assert!(!reach.contains(&index_of(&ix, "other.batch")), "test reach does not follow the other module's item");
    let out = store.refs("other.batch").unwrap();
    assert!(out.starts_with("Nothing uses other.batch."), "{out}");
}

#[test]
fn a_same_named_local_variable_is_not_a_use() {
    let (_root, store) = store_of(
        concat!(
            "pub fn batch() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n    let batch = 2;\n    batch\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[test]\n    fn t() {\n        let batch = 3;\n        assert_eq!(batch, 3);\n    }\n}\n",
        ),
        &[],
    );
    let ix = store.index().unwrap();
    let batch = index_of(&ix, "crate.batch");
    assert!(!uses(&ix, "crate.go", "crate.batch"));
    assert!(!uses(&ix, "crate.tests::t", "crate.batch"));
    let t = index_of(&ix, "crate.tests::t");
    assert!(!crate::reach::closure(&ix, t).contains(&batch), "test reach does not follow a local variable");
    let out = store.refs("crate.batch").unwrap();
    assert!(out.starts_with("Nothing uses crate.batch."), "{out}");
}

#[test]
fn a_method_call_through_a_value_is_a_possible_use() {
    let (_root, store) = store_of(
        concat!(
            "pub struct A;\n\nimpl A {\n    pub fn show(&self) -> i32 {\n        1\n    }\n}\n\n",
            "pub fn go(a: &A) -> i32 {\n    a.show()\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[test]\n    fn calls_go() {\n        assert_eq!(go(&A), 1);\n    }\n}\n",
        ),
        &[],
    );
    let ix = store.index().unwrap();
    let show = index_of(&ix, "crate.A::show");
    assert!(possible(&ix, "crate.go", "crate.A::show"));
    let t = index_of(&ix, "crate.tests::calls_go");
    assert!(crate::reach::closure(&ix, t).contains(&show), "test reach follows a possible use");
    let out = store.refs("crate.A::show").unwrap();
    assert!(out.contains("== Possible uses (1):"), "{out}");
    assert!(out.contains("-- crate.go\n"), "{out}");
}

#[test]
fn a_same_named_parameter_is_not_a_use() {
    let (_root, store) = store_of("pub fn batch() -> i32 {\n    1\n}\n\npub fn go(batch: i32) -> i32 {\n    batch\n}\n", &[]);
    let ix = store.index().unwrap();
    let batch = index_of(&ix, "crate.batch");
    assert!(!uses(&ix, "crate.go", "crate.batch"));
    assert!(!crate::reach::closure(&ix, index_of(&ix, "crate.go")).contains(&batch), "test reach does not follow a parameter");
    let out = store.refs("crate.batch").unwrap();
    assert!(out.starts_with("Nothing uses crate.batch."), "{out}");
}

#[test]
fn an_ambiguous_bare_type_still_reaches_its_trait_impls() {
    let (_root, ix) = index(
        concat!(
            "use std::fmt;\n\npub struct Batch {}\n\n",
            "impl fmt::Display for Batch {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n",
            "        write!(f, \"batch\")\n    }\n}\n\n",
            "#[allow(non_snake_case)]\npub fn Batch() -> i32 {\n    1\n}\n\n",
            "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
            "    #[test]\n    fn mentions_batch() {\n        assert_eq!(Batch(), 1);\n",
            "        let _ = format!(\"{}\", Batch {});\n    }\n}\n",
        ),
        &[],
    );
    let t = index_of(&ix, "crate.tests::mentions_batch");
    let fmt = index_of(&ix, "crate.<Batch as fmt::Display>::fmt");
    assert!(possible(&ix, "crate.tests::mentions_batch", "crate.<Batch as fmt::Display>::fmt"));
    assert!(crate::reach::closure(&ix, t).contains(&fmt), "test reach keeps the trait impl of an ambiguous bare type");
}

/// A `||` condition is not a closure, even when a closure follows in the
/// same body; the call between them is still a use.
#[test]
fn a_logical_or_before_a_closure_does_not_hide_a_use() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go(text: &str) -> i32 {\n",
            "    if text.is_empty() || text.len() > 1 {\n",
            "        return make();\n",
            "    }\n",
            "    let f = |m: &str| m.len() as i32;\n",
            "    f(text)\n",
            "}\n",
        ),
        &[],
    );
    assert_eq!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).len(), 1);
}

/// A closure directly after another closure's closing bar still binds its
/// parameters.
#[test]
fn a_closure_after_a_closure_still_binds_its_parameters() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go() -> i32 {\n",
            "    let f = |make: i32| |make: i32| make + 1;\n",
            "    (f(1))(2)\n",
            "}\n",
        ),
        &[],
    );
    assert!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).is_empty());
}

/// Every static in a `thread_local!` block is named, including ones after the
/// first, which the item's symbol name does not expose.
#[test]
fn thread_local_names_read_every_static() {
    let text = "thread_local! {\n    /// The first.\n    static FIRST: u8 = 1;\n    /// The second.\n    static SECOND: u8 = 2;\n}\n";
    assert_eq!(super::thread_local_names(text), vec!["FIRST", "SECOND"]);
    assert!(super::thread_local_names("pub fn one() {}").is_empty());
}

/// A format capture names the item; escaped braces and positional slots do not.
#[test]
fn a_format_capture_is_a_site_and_a_use() {
    let (_root, ix) = index(
        concat!(
            "const PROBE: &str = \"p\";\n\n",
            "pub fn captured() -> String {\n    format!(\"a {PROBE} b\")\n}\n\n",
            "pub fn escaped() -> String {\n    format!(\"a {{PROBE}} b\")\n}\n\n",
            "pub fn positional() -> String {\n    format!(\"{}\", PROBE)\n}\n",
            "pub fn named() -> String {\n    format!(\"{PROBE}\", PROBE = 1)\n}\n",
        ),
        &[],
    );
    let probe = index_of(&ix, "crate.PROBE");
    let captured = index_of(&ix, "crate.captured");
    let escaped = index_of(&ix, "crate.escaped");
    let positional = index_of(&ix, "crate.positional");
    let named = index_of(&ix, "crate.named");
    assert!(ix.uses[captured].contains(&probe), "{:?}", ix.uses[captured]);
    assert!(!ix.uses[escaped].contains(&probe), "{:?}", ix.uses[escaped]);
    assert!(!ix.uses[named].contains(&probe), "{:?}", ix.uses[named]);
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, probe);
    assert!(sites.iter().all(|s| s.symbol != Some(named)), "{sites:?}");
    let mut kinds: Vec<(usize, bool)> = sites.iter().map(|s| (s.symbol.unwrap(), s.format)).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, [(captured, true), (positional, false)]);
}

/// A crate module named like the item does not turn `drafts::view` into a module path.
#[test]
fn a_module_name_does_not_hide_an_item() {
    let (_root, ix) = index(
        "mod drafts;\nmod view;\n\npub fn go() {\n    drafts::view();\n}\n",
        &[("src/drafts.rs", "pub fn view() {}\n"), ("src/view.rs", "pub fn other() {}\n")],
    );
    let go = index_of(&ix, "crate.go");
    let view = index_of(&ix, "drafts.view");
    assert!(ix.uses[go].contains(&view), "{:?}", ix.uses[go]);
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, view);
    assert_eq!(sites.iter().filter(|s| s.symbol == Some(go)).count(), 1);
}

/// A path through a module's named re-export names the exported item.
#[test]
fn a_reexport_resolves_to_its_item() {
    let (_root, ix) = index(
        "mod board;\n\npub fn go() {\n    board::strategy_guard();\n}\n",
        &[
            ("src/board.rs", "mod strategy;\n\npub use strategy::strategy_guard;\n"),
            ("src/board/strategy.rs", "pub fn strategy_guard() {}\n"),
        ],
    );
    let go = index_of(&ix, "crate.go");
    let guard = index_of(&ix, "board::strategy.strategy_guard");
    assert!(ix.uses[go].contains(&guard), "{:?}", ix.uses[go]);
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, guard);
    assert_eq!(sites.iter().filter(|s| s.symbol == Some(go)).count(), 1);
}

/// A qualified type still reaches the methods its trait impls provide.
#[test]
fn a_qualified_type_keeps_its_trait_impl_methods_possible() {
    let (_root, ix) = index(
        concat!("mod a;\n\n", "pub fn go() -> a::Task {\n    a::Task::default()\n}\n",),
        &[("src/a.rs", concat!("pub struct Task;\n\n", "impl Default for Task {\n    fn default() -> Self {\n        Task\n    }\n}\n",))],
    );
    assert!(uses(&ix, "crate.go", "a.Task"));
    assert!(possible(&ix, "crate.go", "a.<Task as Default>::default"));
    assert!(!possible(&ix, "crate.go", "a.Task"));
}

/// A missing child module does not fall back to a crate module of the same name.
#[test]
fn a_missing_child_module_does_not_resolve_a_crate_module() {
    let (_root, ix) = index(
        "mod drafts;\nmod view;\n\npub fn go() {\n    drafts::view::thing();\n}\n",
        &[("src/drafts.rs", "pub fn other() {}\n"), ("src/view.rs", "pub fn thing() {}\n")],
    );
    let go = index_of(&ix, "crate.go");
    let thing = index_of(&ix, "view.thing");
    let sites = super::rewrite_sites(&ix.syms, &ix.modules, thing);
    assert!(sites.iter().all(|s| s.symbol != Some(go)), "{sites:?}");
}

/// A Unicode escape's brace is not a format capture; an escaped backslash is.
#[test]
fn a_unicode_escape_is_not_a_format_capture() {
    let (_root, ix) = index(
        concat!(
            "pub fn e9() -> i32 {\n    9\n}\n\n",
            "pub fn unicode() -> String {\n    format!(\"\\u{e9}\")\n}\n\n",
            "pub fn literal() -> String {\n    format!(\"\\\\u{e9}\")\n}\n",
        ),
        &[],
    );
    assert!(!uses(&ix, "crate.unicode", "crate.e9"));
    assert!(uses(&ix, "crate.literal", "crate.e9"));
}

/// A call between two `||` operators is a use even when a closure follows.
#[test]
fn a_call_between_logical_ors_before_a_closure_is_a_use() {
    let (_root, ix) = index(
        concat!(
            "pub fn make() -> i32 {\n    1\n}\n\n",
            "pub fn go(a: bool, b: bool) -> i32 {\n",
            "    if a || make() == 1 || b {\n",
            "        return 1;\n",
            "    }\n",
            "    let f = |m: &str| m.len() as i32;\n",
            "    f(\"\")\n",
            "}\n",
        ),
        &[],
    );
    assert_eq!(super::rewrite_sites(&ix.syms, &ix.modules, index_of(&ix, "crate.make")).len(), 1);
}
