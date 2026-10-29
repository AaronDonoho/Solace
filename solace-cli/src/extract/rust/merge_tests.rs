#![cfg(test)]
use crate::testing::{CARGO, Scratch, rust_project};
use crate::{ Store};
use crate::failure::Out;

use super::super::tests::{id, ready, text};



fn fold(s: &Store, args: &[&str]) -> Out {
    s.merge(args, usize::MAX)
}

/// A duplicate function folds into its copy, keeps the survivor's identity
/// and revision, and its caller follows the survivor.
#[test]
fn a_function_merges_into_its_copy() {
    if !ready() {
        return;
    }
    let (_root, s) =
        rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn go() -> i32 {\n    two()\n}\n");
    let keep = id(&s, "crate.one");
    let gone = id(&s, "crate.two");
    let revision: String = s.db.query_row("SELECT revision FROM symbols WHERE id = ?1", [&keep], |r| r.get(0)).unwrap();
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    assert!(out.contains("Redirected 1 reference in 1 module"), "{out}");
    assert_eq!(id(&s, "crate.one"), keep);
    let after: String = s.db.query_row("SELECT revision FROM symbols WHERE id = ?1", [&keep], |r| r.get(0)).unwrap();
    assert_eq!(after, revision, "the survivor's text did not change");
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> i32 {\n    one()\n}");
    assert!(s.resolve(&s.index().unwrap(), "crate.two").is_err());
    let link: (String, String) =
        s.db.query_row("SELECT action, detail FROM history WHERE symbol = ?1 ORDER BY rowid DESC LIMIT 1", [&gone], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(link, ("merged-into".to_string(), keep));
}

#[test]
fn a_cross_module_merge_redirects_an_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn two() -> i32 {\n    2\n}\n"),
        ("src/callers.rs", "use crate::b::two;\n\npub fn go() -> i32 {\n    two()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["a.one", "b.two"]).unwrap();
    assert!(out.contains("Redirected 1 reference in 1 module"), "{out}");
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.contains("use crate::a::one;"), "{}", callers.header);
    assert_eq!(text(&s, "callers.go"), "pub fn go() -> i32 {\n    one()\n}");
    assert!(ix.modules.iter().all(|m| m.path != "b"), "the emptied module is retired");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_test_that_reaches_only_the_retired_function_moves() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        (
            "src/b.rs",
            "pub fn two() -> i32 {\n    2\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn two_works() {\n        assert_eq!(two(), 2);\n    }\n}\n",
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["a.one", "b.two"]).unwrap();
    assert!(out.contains("Tests moved: b.tests::two_works -> a"), "{out}");
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "a::tests::two_works").is_ok());
    assert!(s.resolve(&ix, "b::tests::two_works").is_err());
}

#[test]
fn a_test_that_reaches_staying_code_stays() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub fn one() -> i32 {\n    1\n}\n\n",
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn both_work() {\n        assert_eq!(one() + two(), 3);\n    }\n}\n",
    ));
    fold(&s, &["crate.one", "crate.two"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "crate.one").is_ok());
    assert!(s.resolve(&ix, "crate.two").is_err());
    assert!(s.resolve(&ix, "crate.tests::both_work").is_ok());
    let test = text(&s, "crate.tests::both_work");
    assert!(test.contains("one() + one()"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_of_different_signatures_is_refused() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub fn one(x: i32) -> i32 {\n    x\n}\n\npub fn two(x: i32, y: i32) -> i32 {\n    x + y\n}\n");
    let err = fold(&s, &["crate.one", "crate.two"]).unwrap_err().0;
    assert!(err.starts_with("E5008"), "{err}");
    assert!(err.contains("crate.one") && err.contains("crate.two") && err.contains("takes"), "{err}");
    assert!(text(&s, "crate.one").contains("fn one(x: i32)"));
}

#[test]
fn a_type_is_refused_by_merge() {
    let (_root, s) = rust_project("pub struct S;\n\npub fn two() -> i32 {\n    2\n}\n");
    let err = fold(&s, &["crate.S", "crate.two"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("is a type"), "{err}");
}

#[test]
fn a_merge_cargo_failure_rolls_back() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod c;\nmod b;\n"),
        ("src/a.rs", "pub fn two() -> i32 {\n    2\n}\n"),
        ("src/b.rs", "mod hidden;\n"),
        ("src/b/hidden.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/c.rs", "use crate::a::two;\n\npub fn go() -> i32 {\n    two()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let before = super::super::tests::written(&root, "src/c.rs");
    let history: i64 = s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
    let err = fold(&s, &["b::hidden.one", "a.two"]).unwrap_err().0;
    assert!(err.contains("does not type-check"), "{err}");
    assert_eq!(super::super::tests::written(&root, "src/c.rs"), before);
    assert!(s.resolve(&s.index().unwrap(), "a.two").is_ok());
    assert_eq!(s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0)).unwrap(), history);
}

#[test]
fn a_rust_merge_to_project_is_refused() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n");
    let err = fold(&s, &["crate.one", "crate.two", "--to-project", "other"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("--to-project") && err.contains("D-207"), "{err}");
}

#[test]
fn a_merge_ignores_parameter_names() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(
        "pub fn one(value: i32) -> i32 {\n    value\n}\n\npub fn two(x: i32) -> i32 {\n    x\n}\n\npub fn go() -> i32 {\n    two(2)\n}\n",
    );
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> i32 {\n    one(2)\n}");
}

#[test]
fn a_merge_drops_an_import_only_the_retired_function_used() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(
        "use std::cmp::Ordering;\n\npub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    let _ = Ordering::Less;\n    2\n}\n",
    );
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    let ix = s.index().unwrap();
    let header = &ix.modules.iter().find(|m| m.path == "crate").unwrap().header;
    assert!(!header.contains("Ordering"), "{header}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn constants_and_statics_merge_into_their_copies() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub const ONE: i32 = 1;\n\n",
        "pub const TWO: i32 = 2;\n\n",
        "pub static THREE: i32 = 3;\n\n",
        "pub static FOUR: i32 = 4;\n\n",
        "pub fn go() -> i32 {\n    ONE + THREE\n}\n",
    ));
    let out = fold(&s, &["crate.ONE", "crate.TWO"]).unwrap();
    assert!(out.contains("Merged crate.TWO into crate.ONE"), "{out}");
    let out = fold(&s, &["crate.THREE", "crate.FOUR"]).unwrap();
    assert!(out.contains("Merged crate.FOUR into crate.THREE"), "{out}");
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> i32 {\n    ONE + THREE\n}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_of_different_kinds_is_refused() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub const ONE: i32 = 1;\n\npub fn two() -> i32 {\n    2\n}\n");
    let err = fold(&s, &["crate.ONE", "crate.two"]).unwrap_err().0;
    assert!(err.starts_with("E5008") && err.contains("constant") && err.contains("function"), "{err}");
}

#[test]
fn a_survivor_that_uses_the_retired_item_is_refused() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    two()\n}\n\npub fn two() -> i32 {\n    2\n}\n");
    let err = fold(&s, &["crate.one", "crate.two"]).unwrap_err().0;
    assert!(err.starts_with("E5002") && err.contains("crate.one uses crate.two"), "{err}");
}

#[test]
fn a_private_survivor_used_elsewhere_is_refused() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        ("src/a.rs", "fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn two() -> i32 {\n    2\n}\n"),
        ("src/callers.rs", "use crate::b::two;\n\npub fn go() -> i32 {\n    two()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = fold(&s, &["a.one", "b.two"]).unwrap_err().0;
    assert!(err.starts_with("E5009") && err.contains("a.one") && err.contains("callers.go"), "{err}");
}

#[test]
fn a_method_is_refused_by_merge() {
    if !ready() {
        return;
    }
    let (_root, s) =
        rust_project("pub struct S;\n\nimpl S {\n    pub fn two(&self) -> i32 {\n        2\n    }\n}\n\npub fn one() -> i32 {\n    1\n}\n");
    let err = fold(&s, &["crate.one", "crate.S::two"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("impl methods"), "{err}");
}

#[test]
fn a_module_is_refused_by_merge() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub mod inner {\n    pub fn two() -> i32 {\n        2\n    }\n}\n\npub fn one() -> i32 {\n    1\n}\n");
    let err = fold(&s, &["crate.one", "crate.inner"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("is a module"), "{err}");
}

#[test]
fn a_merge_of_one_symbol_is_refused() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = fold(&s, &["crate.one", "crate.one"]).unwrap_err().0;
    assert!(err.contains("is one symbol") && err.contains("merge takes two"), "{err}");
}

#[test]
fn an_unresolved_name_is_refused_by_merge() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = fold(&s, &["crate.one", "crate.missing"]).unwrap_err().0;
    assert!(err.starts_with("E1001") && err.contains("crate.missing"), "{err}");
}

#[test]
fn a_merge_ignores_visibility() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn go() -> i32 {\n    two()\n}\n");
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> i32 {\n    one()\n}");
}

#[test]
fn a_merge_keeps_a_trait_import_elsewhere() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod other;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn two() -> i32 {\n    2\n}\n"),
        (
            "src/other.rs",
            "use std::fmt::Write;\n\npub fn go() -> String {\n    let mut s = String::new();\n    s.write_str(\"x\").unwrap();\n    s\n}\n",
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["a.one", "b.two"]).unwrap();
    assert!(out.contains("Merged b.two into a.one"), "{out}");
    let ix = s.index().unwrap();
    let header = &ix.modules.iter().find(|m| m.path == "other").unwrap().header;
    assert!(header.contains("use std::fmt::Write;"), "{header}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_keeps_a_trait_import_a_staying_function_uses() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "use std::fmt::Write;\n\n",
        "pub fn one() -> i32 {\n    1\n}\n\n",
        "pub fn two() -> i32 {\n    2\n}\n\n",
        "pub fn go() -> String {\n    let mut s = String::new();\n",
        "    s.write_str(\"x\").unwrap();\n    s\n}\n",
    ));
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    let ix = s.index().unwrap();
    let header = &ix.modules.iter().find(|m| m.path == "crate").unwrap().header;
    assert!(header.contains("use std::fmt::Write;"), "{header}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_ignores_parameter_names_after_generics() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub fn one<F: Fn(u32) -> u32>(f: F) -> u32 {\n    f(1)\n}\n\n",
        "pub fn two<F: Fn(u32) -> u32>(g: F) -> u32 {\n    g(2)\n}\n\n",
        "pub fn go() -> u32 {\n    two(|x| x + 1)\n}\n",
    ));
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> u32 {\n    one(|x| x + 1)\n}");
}

#[test]
fn a_merge_keeps_an_import_a_child_reaches_through_super_glob() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "use std::cmp::Ordering;\n\n",
                "pub fn one() -> i32 {\n    child::order()\n}\n\n",
                "pub fn two() -> i32 {\n    let _ = Ordering::Less;\n    2\n}\n\n",
                "#[path = \"child.rs\"]\nmod child;\n",
            ),
        ),
        ("src/child.rs", "use super::*;\n\npub fn order() -> i32 {\n    let _ = Ordering::Greater;\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["crate.one", "crate.two"]).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    let ix = s.index().unwrap();
    let header = &ix.modules.iter().find(|m| m.path == "crate").unwrap().header;
    assert!(header.contains("use std::cmp::Ordering;"), "{header}");
    assert!(s.resolve(&ix, "child.order").is_ok());
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_places_a_redirected_use_after_its_group() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn two() -> i32 {\n    2\n}\n\npub fn other() -> i32 {\n    3\n}\n"),
        (
            "src/callers.rs",
            concat!(
                "use crate::b::{other, two};\nuse std::cmp::Ordering;\n\n",
                "pub fn go() -> i32 {\n    let _ = Ordering::Less;\n    two() + other()\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["a.one", "b.two"]).unwrap();
    assert!(out.contains("Merged b.two into a.one"), "{out}");
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.contains("use crate::b::other;\nuse crate::a::one;"), "{}", callers.header);
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_merge_puts_a_redirected_use_where_its_item_was() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn two() -> i32 {\n    2\n}\n"),
        (
            "src/callers.rs",
            concat!(
                "use crate::b::{two};\nuse std::cmp::Ordering;\n\n",
                "pub fn go() -> i32 {\n    let _ = Ordering::Less;\n    two()\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = fold(&s, &["a.one", "b.two"]).unwrap();
    assert!(out.contains("Merged b.two into a.one"), "{out}");
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.starts_with("use crate::a::one;\nuse std::cmp::Ordering;"), "{}", callers.header);
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}
