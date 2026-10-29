#![cfg(test)]

use super::*;
use crate::testing::*;



pub(super) fn ready() -> bool {
    crate::language::cargo::installed()
}

pub(super) fn text(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
}

pub(super) fn id(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.syms[s.resolve(&ix, name).unwrap()].id.clone()
}

fn extract(s: &Store, args: &[&str]) -> Out {
    s.extract(args, usize::MAX)
}

pub(super) fn written(root: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

/// A private helper moves with its caller, the caller's old references are
/// qualified, and the caller becomes `pub(crate)` because code stays behind.
#[test]
fn a_fn_and_its_private_helper_move_and_callers_compile() {
    if !ready() {
        return;
    }
    let (root, s) =
        rust_project("pub fn go() -> i32 {\n    make()\n}\n\nfn make() -> i32 {\n    helper()\n}\n\nfn helper() -> i32 {\n    1\n}\n");
    let before = id(&s, "crate.make");
    let out = extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    assert!(out.contains("Moved crate.make (+1 helper: helper) to helpers"), "{out}");
    assert_eq!(id(&s, "helpers.make"), before);
    assert_eq!(text(&s, "crate.go"), "pub fn go() -> i32 {\n    make()\n}");
    assert!(text(&s, "helpers.make").starts_with("pub(crate) fn make"), "{}", text(&s, "helpers.make"));
    assert!(text(&s, "helpers.helper").starts_with("fn helper"), "{}", text(&s, "helpers.helper"));
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("use crate::helpers::make;"), "{source}");
    assert!(source.contains("pub fn go() -> i32 {\n    make()\n}"), "{source}");
}

/// A target module the store doesn't have is created, with its `mod` line.
#[test]
fn a_new_target_module_is_created() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let out = extract(&s, &["crate.one", "--to", "helpers"]).unwrap();
    assert!(out.contains("Created module helpers (src/helpers.rs)"), "{out}");
    assert!(root.join("src/helpers.rs").exists());
    assert!(written(&root, "src/lib.rs").contains("mod helpers;"), "{}", written(&root, "src/lib.rs"));
    assert!(written(&root, "src/helpers.rs").contains("pub fn one()"));
}

/// A name the target module already has is refused before anything changes.
#[test]
fn a_name_clash_gives_e1004() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub fn make() -> i32 {\n    1\n}\n");
    s.submit_symbols(None, Some("helpers"), "pub fn make() -> i32 {\n    2\n}\n").unwrap();
    let err = extract(&s, &["crate.make", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004 helpers.make already exists in helpers"), "{err}");
    assert!(s.resolve(&s.index().unwrap(), "crate.make").is_ok());
}

/// A cargo failure rolls back the rows, the new file and the original text.
#[test]
fn a_cargo_failure_rolls_back() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub struct S {\n    field: i32,\n}\n\npub fn make() -> S {\n    S { field: 1 }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let history: i64 = s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
    let before = written(&root, "src/a.rs");
    let err = extract(&s, &["a.make", "--to", "b"]).unwrap_err().0;
    assert!(err.starts_with("Not moved:"), "{err}");
    assert!(err.contains("field"), "{err}");
    assert_eq!(written(&root, "src/a.rs"), before);
    assert!(!root.join("src/b.rs").exists());
    assert!(s.index().unwrap().modules.iter().all(|m| m.path != "b"));
    assert_eq!(text(&s, "a.make"), "pub fn make() -> S {\n    S { field: 1 }\n}");
    assert_eq!(s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0)).unwrap(), history);
}

/// A foreign `vendor::make` and a shadowing `let make` are not the item, so
/// their bytes stay exactly as written.
#[test]
fn vendor_make_and_a_local_let_make_are_untouched() {
    if !ready() {
        return;
    }
    let manifest = concat!(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
        "[dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest),
        ("vendor/Cargo.toml", "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("vendor/src/lib.rs", "pub fn make() -> i32 {\n    5\n}\n"),
        ("src/lib.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn go() -> i32 {\n    let make = 2;\n    make + vendor::make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let before = text(&s, "crate.go");
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    assert_eq!(text(&s, "crate.go"), before);
}

/// An import that binds the moved name is retargeted and its bare use stays.
#[test]
fn an_import_is_retargeted_and_check_warnings_stays_clean() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod callers;\n\nfn make() -> i32 {\n    1\n}\n"),
        ("src/callers.rs", "use crate::make;\n\npub fn go() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.contains("use crate::helpers::make;"), "{}", callers.header);
    assert_eq!(text(&s, "callers.go"), "pub fn go() -> i32 {\n    make()\n}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A test that reaches moved and staying code stays and is rewritten.
#[test]
fn a_test_split_stays_and_is_rewritten() {
    if !ready() {
        return;
    }
    let lib = concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "pub fn other() -> i32 {\n    2\n}\n\n",
        "pub fn keep() -> i32 {\n    make() + other()\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make() + other(), 3);\n    }\n}\n",
    );
    let (_root, s) = rust_project(&lib);
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.make").is_ok());
    assert!(s.resolve(&ix, "crate.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_err());
    let test = text(&s, "crate.tests::make_works");
    assert!(test.contains("make() + other()"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Rust `--to-project` is refused while same-project extract is available.
#[test]
fn to_project_is_refused() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = extract(&s, &["crate.one", "--to", "helpers", "--to-project", "other"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("D-185"), "{err}");
}

#[test]
fn an_unknown_target_names_the_namespace_follow_up() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = extract(&s, &["crate.one", "--to", "not a name"]).unwrap_err().0;
    assert!(err.starts_with("E1003") && err.contains("use --to <module>"), "{err}");
}

/// A re-export of the moved item follows it; imports the move didn't touch stay.
#[test]
fn a_pub_use_is_retargeted_and_unrelated_imports_stay() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod other;\npub use crate::a::make;\n\npub fn shared() -> i32 {\n    1\n}\n"),
        ("src/a.rs", "pub fn make() -> i32 {\n    crate::shared()\n}\n"),
        ("src/other.rs", "#[allow(unused_imports)]\nuse crate::shared;\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let header = |path: &str| ix.modules.iter().find(|m| m.path == path).unwrap().header.clone();
    assert!(header("crate").contains("pub use crate::b::make;"), "{}", header("crate"));
    assert!(header("other").contains("use crate::shared;"), "{}", header("other"));
}

/// A moved member of a `pub use` group, on one line or several, stays re-exported.
#[test]
fn a_grouped_pub_use_keeps_its_visibility() {
    if !ready() {
        return;
    }
    for group in ["pub use crate::a::{make, other};\n", "pub use crate::a::{\n    make,\n    other,\n};\n"] {
        let lib = format!("pub mod a;\n{group}");
        let root = Scratch::new().with(&[
            ("Cargo.toml", CARGO),
            ("src/lib.rs", &lib),
            ("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn other() -> i32 {\n    2\n}\n"),
        ]);
        crate::import(&root).unwrap();
        let s = Store::at(&root).unwrap();
        extract(&s, &["a.make", "--to", "b"]).unwrap();
        let ix = s.index().unwrap();
        let header = &ix.modules.iter().find(|m| m.path == "crate").unwrap().header;
        assert!(header.contains("pub use crate::b::make;"), "{header}");
        assert!(header.contains("other"), "{header}");
    }
}

#[test]
fn cargo_is_installed_for_rust_extract() {
    assert!(ready(), "Rust extract tests need cargo on PATH");
}

#[test]
fn a_trait_method_moving_with_its_trait_is_no_clash() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub trait T {\n    fn make(&self) -> i32;\n}\n");
    s.submit_symbols(None, Some("helpers"), "pub fn make() -> i32 {\n    2\n}\n").unwrap();
    let out = extract(&s, &["crate.T", "--to", "helpers"]).unwrap();
    assert!(out.contains("Moved crate.T to helpers"), "{out}");
}

/// Code moved into a child module still sees a private item that stays in
/// the crate root, so the item keeps its visibility.
#[test]
fn a_staying_private_helper_stays_private() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(
        "pub fn go() -> i32 {\n    make() + helper()\n}\n\nfn make() -> i32 {\n    helper()\n}\n\nfn helper() -> i32 {\n    1\n}\n",
    );
    let out = extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    assert!(out.contains("Now pub(crate): helpers.make"), "{out}");
    assert!(!out.contains("crate.helper"), "{out}");
    assert!(text(&s, "crate.helper").starts_with("fn helper"), "{}", text(&s, "crate.helper"));
    assert!(text(&s, "helpers.make").contains("crate::helper()"), "{}", text(&s, "helpers.make"));
}

#[test]
fn an_import_used_only_in_a_macro_body_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod callers;\n"),
        ("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n"),
        ("src/callers.rs", "use crate::a::make;\n\npub fn go() -> String {\n    format!(\"{}\", make())\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.contains("use crate::b::make;"), "{}", callers.header);
    assert_eq!(text(&s, "callers.go"), "pub fn go() -> String {\n    format!(\"{}\", make())\n}");
}

#[test]
fn an_allow_unused_import_the_move_did_not_touch_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n\npub fn shared() -> i32 {\n    5\n}\n"),
        (
            "src/a.rs",
            concat!(
                "#[allow(unused_imports)]\nuse crate::shared;\n\n",
                "pub fn make() -> i32 {\n    crate::shared()\n}\n\npub fn keep() -> i32 {\n    1\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(a.header.contains("use crate::shared;"), "{}", a.header);
}

#[test]
fn moved_code_using_an_external_import_names_the_follow_up() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "use std::collections::HashMap;\n\npub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::HashMap;"), "{}", b.header);
}

#[test]
fn an_inline_module_use_keeps_its_braces() {
    if !ready() {
        return;
    }
    for m in ["pub mod m { pub use crate::a::{make, x}; }\n", "pub mod m {\n    pub use crate::a::{make, x};\n}\n"] {
        let lib = format!("pub mod a;\n{m}");
        let root = Scratch::new().with(&[
            ("Cargo.toml", CARGO),
            ("src/lib.rs", &lib),
            ("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn x() -> i32 {\n    2\n}\n"),
        ]);
        crate::import(&root).unwrap();
        let s = Store::at(&root).unwrap();
        extract(&s, &["a.make", "--to", "b"]).unwrap();
        let inner = text(&s, "crate.m");
        assert!(inner.contains("pub use crate::b::make;"), "{inner}");
        assert!(inner.contains("x"), "{inner}");
    }
}

#[test]
fn an_attribute_is_not_a_reexport() {
    let text = "#[cfg(test)] use a::make;\nmod m { use b::more; }\npub use a::other;\n#[cfg(test)]\npub use a::third;";
    let ranges = crate::language::rust::import_ranges(text).unwrap();
    assert_eq!(super::use_visibility(text, &ranges[0].1), "");
    assert_eq!(super::use_visibility(text, &ranges[1].1), "");
    assert_eq!(super::use_visibility(text, &ranges[2].1), "pub ");
    assert_eq!(super::use_visibility(text, &ranges[3].1), "pub ");
}

#[test]
fn an_import_only_the_moved_code_used_leaves_its_module() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod util;\n"),
        ("src/util.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
        ("src/a.rs", "use crate::util::helper;\n\npub fn make() -> i32 {\n    helper()\n}\n\npub fn keep() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(!a.header.contains("helper"), "{}", a.header);
}

#[test]
fn moved_code_using_an_import_the_destination_has_moves() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::collections::HashMap;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n\npub fn keep() -> i32 {\n    1\n}\n",
            ),
        ),
        ("src/b.rs", "use std::collections::HashMap;\n\npub fn other() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    assert!(ix.syms.iter().any(|x| x.module == "b" && x.name == "make"));
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::HashMap;"), "{}", b.header);
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(!a.header.contains("HashMap"), "{}", a.header);
}

#[test]
fn an_allowed_import_only_the_moved_code_used_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod util;\n"),
        ("src/util.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
        (
            "src/a.rs",
            concat!(
                "#[allow(unused_imports)]\nuse crate::util::helper;\n\n",
                "pub fn make() -> i32 {\n    helper()\n}\n\npub fn keep() -> i32 {\n    1\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(a.header.contains("use crate::util::helper;"), "{}", a.header);
}

#[test]
fn an_import_a_remaining_macro_body_uses_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod util;\n"),
        ("src/util.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
        (
            "src/a.rs",
            concat!(
                "use crate::util::helper;\n\npub fn make() -> i32 {\n    helper()\n}\n\n",
                "pub fn keep() -> String {\n    format!(\"{}\", helper())\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(a.header.contains("use crate::util::helper;"), "{}", a.header);
}

#[test]
fn an_external_reexport_only_the_moved_code_used_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub use std::collections::HashMap;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n\npub fn keep() -> i32 {\n    1\n}\n",
            ),
        ),
        ("src/b.rs", "use std::collections::HashMap;\n\npub fn other() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(a.header.contains("pub use std::collections::HashMap;"), "{}", a.header);
}

/// A move rewrites an inline module's import and leaves its call through it.
#[test]
fn an_inline_module_calls_through_its_rewritten_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!("pub mod a;\n\n", "pub mod m {\n    use crate::a::make;\n\n    pub fn go() -> i32 {\n        make()\n    }\n}\n",),
        ),
        ("src/a.rs", "pub fn make() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let lib = written(&root, "src/lib.rs");
    assert!(lib.contains("use crate::b::make;"), "{lib}");
    assert!(lib.contains("make()"), "{lib}");
}

#[test]
fn a_grouped_external_import_loses_only_the_moved_member() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::collections::{HashMap, HashSet};\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n\n",
                "pub fn keep() -> HashSet<i32> {\n    HashSet::new()\n}\n",
            ),
        ),
        ("src/b.rs", "use std::collections::HashMap;\n\npub fn other() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let a = written(&root, "src/a.rs");
    assert!(!a.contains("HashMap"), "{a}");
    assert!(a.contains("HashSet"), "{a}");
}

#[test]
fn an_external_use_symbol_only_the_moved_code_used_leaves_its_module() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub fn keep() -> i32 {\n    1\n}\n\n",
                "use std::collections::HashMap;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n",
            ),
        ),
        ("src/b.rs", "use std::collections::HashMap;\n\npub fn other() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let a = written(&root, "src/a.rs");
    assert!(!a.contains("HashMap"), "{a}");
}

/// A local variable named like an external import does not refuse the move.
#[test]
fn a_local_name_is_no_external_import_mention() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::hint::black_box as make;\n\n",
                "pub fn chosen() -> i32 {\n    let make = 1;\n    make\n}\n\n",
                "pub fn keep() -> i32 {\n    std::hint::black_box(1)\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.chosen", "--to", "b"]).unwrap();
}

/// A type takes its inherent and trait impls, and a trait-impl method is not promoted.
#[test]
fn a_type_moves_with_its_inherent_and_trait_impls() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub trait Named {\n    fn name(&self) -> &'static str;\n}\n\n",
        "pub struct Task {\n    id: i32,\n}\n\n",
        "impl Task {\n    pub fn new() -> Self {\n        Task { id: 1 }\n    }\n\n    pub fn id(&self) -> i32 {\n        self.id\n    }\n}\n\n",
        "impl Named for Task {\n    fn name(&self) -> &'static str {\n        \"task\"\n    }\n}\n\n",
        "pub fn label(t: &Task) -> (&'static str, i32) {\n    (t.name(), t.id())\n}\n",
    ));
    let out = extract(&s, &["crate.Task", "--to", "model"]).unwrap();
    assert!(out.contains("Moved crate.Task to model"), "{out}");
    assert!(out.contains("Impls moved:"), "{out}");
    let model = written(&root, "src/model.rs");
    assert!(model.contains("impl Task {"), "{model}");
    assert!(model.contains("Named for Task"), "{model}");
    assert!(!model.contains("pub(crate) fn name"), "{model}");
    let lib = written(&root, "src/lib.rs");
    assert!(lib.contains("pub trait Named"), "{lib}");
    assert!(!lib.contains("impl Task {"), "{lib}");
    assert!(!lib.contains("Named for Task"), "{lib}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// An impl for a same-leaf type elsewhere stays where it is.
#[test]
fn a_same_leaf_impl_for_another_type_stays() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub struct Task;\n\n",
        "impl Task {\n    pub fn local() -> Self {\n        Task\n    }\n}\n\n",
        "pub mod vendor {\n    pub struct Task;\n}\n\n",
        "impl vendor::Task {\n    pub fn extra() -> Self {\n        vendor::Task\n    }\n}\n",
    ));
    extract(&s, &["crate.Task", "--to", "model"]).unwrap();
    let lib = written(&root, "src/lib.rs");
    assert!(lib.contains("impl vendor::Task"), "{lib}");
    assert!(!lib.contains("impl Task {"), "{lib}");
    let model = written(&root, "src/model.rs");
    assert!(model.contains("impl Task {"), "{model}");
}

/// A test container moves whole when everything it holds moves.
#[test]
fn a_test_inside_a_moved_module_moves() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub fn make() -> i32 {\n    1\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n}\n",
    ));
    let out = extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    assert!(out.contains("Tests moved"), "{out}");
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::make_works").is_err());
    let helpers = written(&root, "src/helpers.rs");
    assert!(helpers.contains("#[cfg(test)]") && helpers.contains("fn make_works"), "{helpers}");
}

/// One eligible test moves while another stays; the source container stays too.
#[test]
fn a_partial_test_move_leaves_staying_tests() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::other_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::make_works").is_err());
    let helpers = written(&root, "src/helpers.rs");
    assert!(helpers.contains("#[cfg(test)]\n#[path = \"helpers_tests.rs\"]\nmod tests;\n"), "{helpers}");
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use super::*;"), "{tests}");
    assert!(tests.contains("assert_eq!(make(), 1);"), "{tests}");
    assert!(tests.contains("fn make_works"), "{tests}");
    assert!(!tests.contains("other_works"), "{tests}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A source helper and a test-local helper move with the test that uses them.
#[test]
fn a_test_using_private_helpers_moves_with_them() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    helper()\n}\n\n",
        "fn helper() -> i32 {\n    1\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    fn check(value: i32) {\n        assert_eq!(value, 1);\n    }\n\n",
        "    #[test]\n    fn make_works() {\n        check(make());\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.helper").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::check").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
}

/// A second move appends to the target's test file without repeating its header.
#[test]
fn a_second_test_move_appends_to_the_targets_test_file() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    extract(&s, &["crate.other", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::other_works").is_ok());
    let tests = written(&root, "src/helpers_tests.rs");
    assert_eq!(tests.matches("fn make_works").count(), 1, "{tests}");
    assert_eq!(tests.matches("fn other_works").count(), 1, "{tests}");
    let helpers = written(&root, "src/helpers.rs");
    assert_eq!(helpers.matches("mod tests;").count(), 1, "{helpers}");
}

#[test]
fn impl_trait_range_keeps_generic_arguments() {
    let text = "impl Named<u8> for Task {}";
    let range = impl_trait_range(text).unwrap();
    assert_eq!(&text[range], "Named");
}

#[test]
fn qualify_trait_impls_rewrites_the_header() {
    let (_root, s) = rust_project(
        r#"pub trait Named {
    fn name(&self) -> u8;
}

pub struct Task;

impl Named for Task {
    fn name(&self) -> u8 {
        0
    }
}
"#,
    );
    let ix = s.index().unwrap();
    let impl_at = ix.syms.iter().position(|x| x.kind_text == "impl").unwrap();
    let mut text = ix.syms.iter().map(|x| x.text.clone()).collect::<Vec<_>>();
    let mut newmod = ix.syms.iter().map(|x| x.module.clone()).collect::<Vec<_>>();
    newmod[impl_at] = "model".to_string();
    qualify_trait_impls(&ix, &vec![true; ix.syms.len()], &newmod, &mut text);
    assert!(text[impl_at].contains("crate::Named"), "{}", text[impl_at]);
}

/// A trait impl's generic arguments stay with the trait path when it is qualified.
#[test]
fn a_generic_trait_impl_path_is_qualified() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(
        r#"pub trait Named<T> {
    fn name(&self) -> T;
}

pub struct Task;

impl Named<u8> for Task {
    fn name(&self) -> u8 {
        0
    }
}
"#,
    );
    extract(&s, &["crate.Task", "--to", "model"]).unwrap();
    let model = written(&root, "src/model.rs");
    assert!(model.contains("impl crate::Named<u8> for Task"), "{model}");
    assert!(!model.contains("impl Named<u8> for Task"), "{model}");
}

/// Moving a type with an ambiguous impl owner names both candidates.
#[test]
fn an_ambiguous_impl_move_is_refused() {
    let syms = vec![
        ambiguous_sym("type:a.Task", "Task", "struct Task;"),
        ambiguous_sym("type:b.Task", "Task", "struct Task;"),
        ambiguous_sym("impl:Task", "Task", "impl Task {}"),
    ];
    let modules = vec![crate::index::ModuleRow {
        path: "crate".to_string(),
        dir: "src".to_string(),
        header: String::new(),
        file: Some("lib.rs".to_string()),
        owns_dir: Some(false),
    }];
    // Rust has no separate `Language` implementation here: `rust = true` selects
    // the Rust reference graph, and the stored language value is not read.
    let ix = crate::index::build(&crate::language::gleam::Gleam, syms, modules, true);
    let err = moving(&ix, &[0]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("ambiguous") && err.contains("Task"), "{err}");
}

fn ambiguous_sym(id: &str, name: &str, text: &str) -> crate::index::Sym {
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

/// A whole moved test container does not merge into the target's same-named container.
#[test]
fn a_whole_test_container_clash_is_refused_with_e1004() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(
        r#"fn make() -> i32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_works() {
        assert_eq!(make(), 1);
    }
}
"#,
    );
    s.submit_symbols(
        None,
        Some("helpers"),
        r#"#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing() {}
}
"#,
    )
    .unwrap();
    let err = extract(&s, &["crate.make", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("tests"), "{err}");
}

#[test]
fn an_inline_import_at_the_destination_does_not_carry_a_move() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", concat!("use std::collections::HashMap;\n\n", "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n",)),
        (
            "src/b.rs",
            concat!(
                "pub mod inner {\n",
                "    use std::collections::HashMap;\n\n",
                "    pub fn other() -> HashMap<i32, i32> {\n        HashMap::new()\n    }\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::HashMap;"), "{}", b.header);
}

/// A moved function keeps the glob, trait and type imports it needs, and the
/// source keeps only the imports its staying code uses.
#[test]
fn a_moved_fn_carries_a_glob_a_trait_and_a_type() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod traits;\n"),
        (
            "src/traits.rs",
            concat!(
                "pub trait Speak {\n    fn speak(&self) -> i32;\n}\n\n",
                "impl Speak for i32 {\n    fn speak(&self) -> i32 {\n        *self\n    }\n}\n",
            ),
        ),
        (
            "src/a.rs",
            concat!(
                "use std::collections::*;\n",
                "use crate::traits::Speak;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    let mut map = HashMap::new();\n    map.insert(1, 2);\n    map\n}\n\n",
                "pub fn talk() -> i32 {\n    (1).speak()\n}\n\n",
                "pub fn keep() -> i32 {\n    1\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() -> i32 {\n    3\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["a.make", "a.talk", "--to", "b"]).unwrap();
    assert!(out.contains("Moved a.make, a.talk to b"), "{out}");
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::*;"), "{}", b.header);
    assert!(b.header.contains("use crate::traits::Speak;"), "{}", b.header);
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(!a.header.contains("collections::*"), "{}", a.header);
    assert!(!a.header.contains("Speak"), "{}", a.header);
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A target import with the same local name makes the carried one an alias,
/// and the moved code names the alias.
#[test]
fn a_conflicting_import_is_aliased() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "use std::collections::HashMap;\n\npub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
        ("src/b.rs", "pub struct HashMap;\n\npub fn other() -> i32 {\n    3\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::HashMap as HashMap_2;"), "{}", b.header);
    let make = &ix.syms[ix.syms.iter().position(|x| x.module == "b" && x.name == "make").unwrap()];
    assert!(make.text.contains("HashMap_2::new()"), "{}", make.text);
}

/// An import the moved code never names stays in the source and is not copied.
#[test]
fn an_import_only_staying_code_uses_is_not_copied() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::collections::HashMap;\n",
                "use std::collections::HashSet;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n\n",
                "pub fn keep() -> HashSet<i32> {\n    HashSet::new()\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() -> i32 {\n    3\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(a.header.contains("HashSet") && !a.header.contains("HashMap"), "{}", a.header);
    assert!(b.header.contains("HashMap") && !b.header.contains("HashSet"), "{}", b.header);
}

/// A function and a module of one name live in different namespaces.
#[test]
fn a_function_beside_a_module_moves() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod helpers;\n\npub fn Foo() -> i32 {\n    1\n}\n"),
        ("src/helpers.rs", "pub mod Foo;\n\npub fn other() -> i32 {\n    3\n}\n"),
        ("src/helpers/Foo.rs", "pub fn inside() -> i32 {\n    4\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["crate.Foo", "--to", "helpers"]).unwrap();
    assert!(out.contains("Moved crate.Foo to helpers"), "{out}");
}

/// A struct and a module share the type namespace, so extract refuses it
/// before cargo sees the duplicate.
#[test]
fn a_struct_beside_a_module_is_refused_with_e1004() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod helpers;\n\npub struct Foo;\n"),
        ("src/helpers.rs", "pub mod Foo;\n\npub fn other() -> i32 {\n    3\n}\n"),
        ("src/helpers/Foo.rs", "pub fn inside() -> i32 {\n    4\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = extract(&s, &["crate.Foo", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("Foo"), "{err}");
}

/// A carried trait import aliases when the target already has that name;
/// the method call needs the trait in scope, not its name.
#[test]
fn a_carried_trait_import_is_aliased() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod traits;\n"),
        (
            "src/traits.rs",
            concat!(
                "pub trait Speak {\n    fn speak(&self) -> i32;\n}\n\n",
                "impl Speak for i32 {\n    fn speak(&self) -> i32 {\n        *self\n    }\n}\n",
            ),
        ),
        ("src/a.rs", "use crate::traits::Speak;\n\npub fn talk() -> i32 {\n    (1).speak()\n}\n\npub fn keep() -> i32 {\n    2\n}\n"),
        ("src/b.rs", "pub trait Speak {\n    fn loud(&self) -> i32;\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.talk", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use crate::traits::Speak as Speak_2;"), "{}", b.header);
    let talk = &ix.syms[ix.syms.iter().position(|x| x.module == "b" && x.name == "talk").unwrap()];
    assert!(talk.text.contains("(1).speak()"), "{}", talk.text);
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn an_inline_modules_import_is_not_carried() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "mod inner {\n    use std::hint::black_box as make;\n}\n\n",
                "fn make() -> i32 {\n    1\n}\n\n",
                "pub fn go() -> i32 {\n    make()\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.go", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    let helpers = ix.modules.iter().find(|m| m.path == "helpers").unwrap();
    assert!(!helpers.header.contains("black_box"), "{}", helpers.header);
}

#[test]
fn header_modules_with_attributes_are_named() {
    assert_eq!(super::header_mod_names("#[cfg(test)] mod tests;\npub mod other;\n").unwrap(), vec!["tests", "other"]);
}

#[test]
fn a_word_ending_in_mod_is_not_a_declaration() {
    let text = "fn go() { some_mod name }";
    let at = text.find("name").unwrap();
    assert!(!super::is_mod_declaration(text, &(at..at + 4)));
    let declaration = "pub mod name;";
    let at = declaration.find("name").unwrap();
    assert!(super::is_mod_declaration(declaration, &(at..at + 4)));
}

#[test]
fn prelude_derive_and_macro_names_are_known() {
    for name in ["fn", "i32", "std", "Debug", "Hash", "format", "assert_eq", "vec", "println"] {
        assert!(super::is_prelude_name(name), "{name}");
    }
    assert!(!super::is_prelude_name("HashMap"));
}

/// A locally imported name is explained without carrying an external glob.
#[test]
fn a_locally_imported_name_does_not_need_an_external_glob() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod types;\n\n",
                "use std::fmt::*;\n\n",
                "use crate::types::Value;\n\n",
                "pub fn make() -> Value {\n    Value::default()\n}\n\n",
                "pub fn stay() -> Value {\n    Value::default()\n}\n",
            ),
        ),
        ("src/types.rs", "#[derive(Default)]\npub struct Value;\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    let helpers = ix.modules.iter().find(|m| m.path == "helpers").unwrap();
    assert!(!helpers.header.contains("fmt"), "{}", helpers.header);
    let unused: Vec<String> = s
        .check_all(&s.index().unwrap())
        .unwrap()
        .into_iter()
        .filter(|d| !d.error && d.text.contains("unused import"))
        .map(|d| d.text)
        .collect();
    assert!(unused.is_empty(), "{unused:?}");
}

/// A move drops an empty inherent impl, keeping one with a member and an
/// empty trait impl.
#[test]
fn a_move_drops_an_empty_inherent_impl() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub trait Named {}\n\n",
        "pub struct Task;\n\n",
        "impl Task {\n    pub fn id() -> i32 {\n        1\n    }\n}\n\n",
        "impl Task {}\n\n",
        "impl Named for Task {}\n\n",
        "pub fn keep() {}\n",
    ));
    let out = extract(&s, &["crate.Task", "crate.Named", "--to", "model"]).unwrap();
    assert!(out.contains("Moved crate.Named, crate.Task to model"), "{out}");
    assert!(out.contains("Dropped empty impl model.Task."), "{out}");
    let model = written(&root, "src/model.rs");
    assert!(model.contains("pub fn id"), "{model}");
    assert_eq!(model.matches("impl Task").count(), 1, "{model}");
    assert!(model.contains("impl Named for Task"), "{model}");
    let lib = written(&root, "src/lib.rs");
    assert!(!lib.contains("impl Task"), "{lib}");
    assert!(lib.contains("pub fn keep"), "{lib}");
    let ix = s.index().unwrap();
    let inherent: Vec<_> =
        ix.syms.iter().filter(|sym| sym.module == "model" && sym.kind_text == "impl" && !sym.name.starts_with('<')).collect();
    assert_eq!(inherent.len(), 1, "{:?}", inherent.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert!(ix.syms.iter().any(|c| c.parent.as_deref() == Some(inherent[0].id.as_str())));
}

#[test]
fn header_modules_in_a_truncated_header_are_named() {
    assert_eq!(super::header_mod_names("pub mod other;\nmod inline {\n").unwrap(), vec!["other"]);
}

#[test]
fn a_header_module_list_that_does_not_parse_is_reported() {
    assert!(super::header_mod_names("pub mod ;\n").is_err());
}

#[test]
fn an_unused_moved_use_does_not_stay() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("pub fn keep() -> i32 {\n    1\n}\n\nuse std::fmt;\n");
    extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(!source.contains("fmt"), "{source}");
}

#[test]
fn an_unused_moved_pub_use_does_not_stay() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("pub fn keep() -> i32 {\n    1\n}\n\npub use std::fmt;\n");
    extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(!source.contains("fmt"), "{source}");
}

#[test]
fn a_staying_moved_use_keeps_its_attributes() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("pub fn keep() -> fmt::Result {\n    Ok(())\n}\n\n#[allow(unused_imports)] use std::fmt;\n");
    extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("#[allow(unused_imports)] use std::fmt;"), "{source}");
}

/// A moved `pub use` that staying code needs comes back without re-exporting.
#[test]
fn a_staying_moved_pub_use_comes_back_private() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("pub fn keep() -> fmt::Result {\n    Ok(())\n}\n\npub use std::fmt;\n");
    extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("use std::fmt;"), "{source}");
    assert!(!source.contains("pub use std::fmt;"), "{source}");
    let destination = written(&root, "src/helpers.rs");
    assert!(destination.contains("pub use std::fmt;"), "{destination}");
}

/// A moved attributed `pub use` that staying code needs comes back privately.
#[test]
fn a_staying_moved_pub_use_keeps_its_attribute_and_loses_pub() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project("#[cfg(test)]\npub fn keep() -> fmt::Result {\n    Ok(())\n}\n\n#[cfg(test)]\npub use std::fmt;\n");
    extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("#[cfg(test)] use std::fmt;"), "{source}");
    assert!(!source.contains("pub use std::fmt;"), "{source}");
    let destination = written(&root, "src/helpers.rs");
    assert!(destination.contains("#[cfg(test)]\npub use std::fmt;"), "{destination}");
}

/// Inherent methods chosen from one impl open an impl of the same type in
/// another module, and staying callers still compile.
#[test]
fn a_method_set_moves_into_a_target_impl() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub struct S {\n    value: i32,\n}\n\n",
        "impl S {\n    pub fn new(value: i32) -> Self {\n        S { value }\n    }\n\n",
        "    pub fn value(&self) -> i32 {\n        self.value\n    }\n}\n\n",
        "pub fn keep(s: &S) -> i32 {\n    s.value()\n}\n",
    ));
    let out = extract(&s, &["crate.S::new", "crate.S::value", "--to", "helpers"]).unwrap();
    assert!(out.contains("Moved crate.S::new, crate.S::value to helpers"), "{out}");
    let ix = s.index().unwrap();
    let at = ix.syms.iter().position(|sym| sym.module == "helpers" && sym.kind_text == "impl").unwrap();
    let parent = ix.syms[at].id.as_str();
    for name in ["S::new", "S::value"] {
        let method = ix.syms.iter().find(|sym| sym.module == "helpers" && sym.name == name).unwrap();
        assert_eq!(method.parent.as_deref(), Some(parent), "{}", method.qname());
    }
    let source = written(&root, "src/lib.rs");
    assert!(!source.contains("fn new") && !source.contains("fn value"), "{source}");
    let helpers = written(&root, "src/helpers.rs");
    assert!(helpers.contains("impl crate::S {") && helpers.contains("pub fn new"), "{helpers}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A chosen inherent impl row moves whole, with its methods under it.
#[test]
fn an_impl_row_moves_with_its_methods() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub struct S;\n\nimpl S {\n    pub fn new() -> Self {\n        S\n    }\n}\n"),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["a.<S>", "--to", "b"]).unwrap();
    assert!(out.contains("Moved a.S to b"), "{out}");
    let ix = s.index().unwrap();
    let at = ix.syms.iter().position(|sym| sym.module == "b" && sym.kind_text == "impl").unwrap();
    let method = ix.syms.iter().find(|sym| sym.module == "b" && sym.name == "S::new").unwrap();
    assert_eq!(method.parent.as_deref(), Some(ix.syms[at].id.as_str()));
    let source = written(&root, "src/a.rs");
    assert!(!source.contains("impl S"), "{source}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("impl crate::a::S {") && target.contains("pub fn new"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A trait impl moves whole; one of its methods alone is refused.
#[test]
fn a_trait_impl_moves_whole_and_a_part_is_refused() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub trait Named {\n    fn name(&self) -> u8;\n    fn kind(&self) -> u8;\n}\n\n",
                "pub struct S;\n\n",
                "impl Named for S {\n    fn name(&self) -> u8 {\n        0\n    }\n\n    fn kind(&self) -> u8 {\n        1\n    }\n}\n\n",
                "pub fn keep(s: &S) -> u8 {\n    s.name()\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = extract(&s, &["a.<S as Named>::name", "--to", "b"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("trait impl"), "{err}");
    let out = extract(&s, &["a.<S as Named>::name", "a.<S as Named>::kind", "--to", "b"]).unwrap();
    assert!(out.contains("Impls moved: a.<S as Named>"), "{out}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("impl crate::a::Named for crate::a::S"), "{target}");
    let source = written(&root, "src/a.rs");
    assert!(!source.contains("impl Named for"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A field a moved method reads is reachable from the target module without
/// becoming public.
#[test]
fn a_moved_methods_private_field_becomes_pub_crate() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub struct S {\n    value: i32,\n}\n\nimpl S {\n    pub fn value(&self) -> i32 {\n        self.value\n    }\n}\n"),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::value", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub struct S"), "{source}");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("impl crate::a::S {") && target.contains("self.value"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A method moving to a module where its import name is taken uses an alias.
#[test]
fn a_moved_method_carries_an_import_and_aliases_a_clash() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            "use std::collections::HashMap;\n\npub struct S;\n\nimpl S {\n    pub fn make() -> HashMap<i32, i32> {\n        HashMap::new()\n    }\n}\n",
        ),
        ("src/b.rs", "pub struct HashMap;\n\npub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::make", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use std::collections::HashMap as HashMap_2;"), "{}", b.header);
    let make = ix.syms.iter().find(|sym| sym.module == "b" && sym.name == "S::make").unwrap();
    assert!(make.text.contains("HashMap_2::new()"), "{}", make.text);
    let a = ix.modules.iter().find(|m| m.path == "a").unwrap();
    assert!(!a.header.contains("collections::HashMap"), "{}", a.header);
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A test that reaches only the moved method goes with it.
#[test]
fn a_test_moves_with_the_method_it_reaches() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub struct S;\n\nimpl S {\n    pub fn value() -> i32 {\n        1\n    }\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn value_works() {\n        assert_eq!(S::value(), 1);\n    }\n}\n",
    ));
    extract(&s, &["crate.S::value", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::value_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::value_works").is_err());
    let test = text(&s, "helpers.tests::value_works");
    assert!(test.contains("crate::S::value()"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A test that reaches both moved and staying methods stays and is rewritten.
#[test]
fn a_test_reaching_a_staying_method_stays() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub struct S;\n\nimpl S {\n    pub fn value() -> i32 {\n        1\n    }\n\n",
        "    pub fn other() -> i32 {\n        2\n    }\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn both_work() {\n        assert_eq!(S::value() + S::other(), 3);\n    }\n}\n",
    ));
    extract(&s, &["crate.S::value", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "crate.tests::both_work").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::both_work").is_err());
    let test = text(&s, "crate.tests::both_work");
    assert!(test.contains("S::value() + S::other()"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Several inherent impls for one type cannot be told apart by `<Type>`.
#[test]
fn two_inherent_impls_are_an_ambiguous_choice() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project("pub struct S;\n\nimpl S {\n    pub fn a() {}\n}\n\nimpl S {\n    pub fn b() {}\n}\n");
    let err = extract(&s, &["crate.<S>", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1002") && err.contains("S::a") && err.contains("S::b"), "{err}");
}

/// A method move that leaves a private tuple field unreachable rolls back.
#[test]
fn a_failed_method_move_rolls_back() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            "pub struct S<T>(T);\n\nimpl<T: Clone> S<T> {\n    pub fn duplicate(&self) -> T {\n        self.0.clone()\n    }\n}\n",
        ),
        ("src/b.rs", "use crate::a::S;\n\nimpl<T> S<T> {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let history: i64 = s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
    let before = written(&root, "src/a.rs");
    let before_b = written(&root, "src/b.rs");
    let err = extract(&s, &["a.S::duplicate", "--to", "b"]).unwrap_err().0;
    assert!(err.starts_with("Not moved:"), "{err}");
    assert_eq!(written(&root, "src/a.rs"), before);
    assert_eq!(written(&root, "src/b.rs"), before_b);
    assert_eq!(s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0)).unwrap(), history);
}

/// A private method that staying code calls becomes reachable from its new impl.
#[test]
fn a_private_method_called_by_staying_code_becomes_pub_crate() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub struct S;\n\nimpl S {\n    fn value() -> i32 {\n        1\n    }\n}\n\n",
        "pub fn keep() -> i32 {\n    S::value()\n}\n",
    ));
    extract(&s, &["crate.S::value", "--to", "helpers"]).unwrap();
    let moved = text(&s, "helpers.S::value");
    assert!(moved.contains("pub(crate) fn value"), "{moved}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Methods join the target module's impl of the same type when it already has one.
#[test]
fn moved_methods_join_an_existing_target_impl() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub struct S {\n    value: i32,\n}\n\nimpl S {\n    pub fn value(&self) -> i32 {\n        self.value\n    }\n}\n"),
        ("src/b.rs", "use crate::a::S;\n\nimpl S {\n    pub fn other() -> i32 {\n        0\n    }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::value", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let impls: Vec<_> = ix.syms.iter().filter(|sym| sym.module == "b" && sym.kind_text == "impl").collect();
    assert_eq!(impls.len(), 1, "{:?}", impls.iter().map(|sym| &sym.name).collect::<Vec<_>>());
    for name in ["S::other", "S::value"] {
        let method = ix.syms.iter().find(|sym| sym.module == "b" && sym.name == name).unwrap();
        assert_eq!(method.parent.as_deref(), Some(impls[0].id.as_str()), "{name}");
    }
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// An empty `impl Type {}` in the target opens to take the moved method.
#[test]
fn an_empty_target_impl_takes_a_moved_method() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub struct S;\n\nimpl S {\n    pub fn value() -> i32 {\n        1\n    }\n}\n"),
        ("src/b.rs", "use crate::a::S;\n\nimpl S {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::value", "--to", "b"]).unwrap();
    let target = written(&root, "src/b.rs");
    assert!(target.contains("impl S {\n    pub fn value"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A tuple field a moved method reads becomes reachable too.
#[test]
fn a_tuple_field_used_by_a_moved_method_becomes_pub_crate() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S(i32);\n\n",
                "impl S {\n",
                "    pub fn value(&self) -> i32 {\n",
                "        self.0\n",
                "    }\n\n",
                "    pub fn first(self) -> i32 {\n",
                "        let Self(value) = self;\n",
                "        value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::value", "a.S::first", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub struct S(pub(crate) i32)"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// An impl whose self type names no type in the crate gives E6013.
#[test]
fn an_impl_with_a_missing_owner_is_refused() {
    let syms = vec![ambiguous_sym("impl:Missing", "Missing", "impl Missing {}")];
    let modules = vec![crate::index::ModuleRow {
        path: "crate".to_string(),
        dir: "src".to_string(),
        header: String::new(),
        file: Some("lib.rs".to_string()),
        owns_dir: Some(false),
    }];
    let ix = crate::index::build(&crate::language::gleam::Gleam, syms, modules, true);
    let err = moving(&ix, &[0]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("Missing") && err.contains("no type"), "{err}");
}

/// A constructor's struct literal and `Self` literal make their fields reachable.
#[test]
fn a_moved_constructor_promotes_struct_literal_fields() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn new(value: i32) -> Self {\n",
                "        S { value }\n",
                "    }\n\n",
                "    pub fn doubled(value: i32) -> Self {\n",
                "        Self { value: value * 2 }\n",
                "    }\n\n",
                "    pub fn value(&self) -> i32 {\n",
                "        self.value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::new", "a.S::doubled", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("pub fn new") && target.contains("pub fn doubled"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A pattern that destructures the moved type makes its field reachable too.
#[test]
fn a_moved_method_promotes_a_pattern_field() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn read(self) -> i32 {\n",
                "        let Self { value, .. } = self;\n",
                "        value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::read", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A same-named field of another type is not a field of the moved type.
#[test]
fn a_field_on_another_type_does_not_promote_the_owner() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "pub struct Other {\n",
                "    pub(crate) value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn staying(&self) -> i32 {\n",
                "        self.value\n",
                "    }\n\n",
                "    pub fn peek(&self, other: &crate::a::Other) -> i32 {\n",
                "        other.value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::peek", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert_eq!(source.matches("pub(crate) value").count(), 1, "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A field read inside a macro makes its field reachable.
#[test]
fn a_field_read_in_a_macro_is_promoted() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn show(&self) -> String {\n",
                "        assert_eq!(self.value, 1);\n",
                "        format!(\"{}\", self.value)\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::show", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A macro whose tokens are not expressions still reveals an owner field.
#[test]
fn a_macro_token_fallback_finds_an_owner_field() {
    let used = super::visibility::method_fields("fn show(&self) { odd! { self.value => } }", "S");
    assert!(used.contains("value"), "{used:?}");
    let used = super::visibility::method_fields("fn show(&self) { odd! { self.0 => } }", "S");
    assert!(used.contains("0"), "{used:?}");
}

/// A field read through a local alias of the receiver becomes reachable.
#[test]
fn a_moved_method_promotes_a_local_alias() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn get(&self) -> i32 {\n",
                "        let me = self;\n",
                "        me.value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::get", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("pub fn get"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_field_read_in_an_async_block_is_promoted() {
    let used = super::visibility::method_fields("fn f(&self) { let _ = async { self.value }; }", "S");
    assert!(used.contains("value"), "{used:?}");
}

#[test]
fn a_field_read_from_a_looped_owner_collection_is_promoted() {
    let used = super::visibility::method_fields("fn f(items: &Vec<S>) { for s in &items { let _ = s.value; } }", "S");
    assert!(used.contains("value"), "{used:?}");
}

/// A field read through a typed local alias of the receiver becomes reachable.
#[test]
fn a_moved_method_promotes_a_typed_local_alias() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n",
                "    value: i32,\n",
                "}\n\n",
                "impl S {\n",
                "    pub fn get(&self) -> i32 {\n",
                "        let me: &S = self;\n",
                "        me.value\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::get", "--to", "b"]).unwrap();
    let source = written(&root, "src/a.rs");
    assert!(source.contains("pub(crate) value: i32"), "{source}");
    let target = written(&root, "src/b.rs");
    assert!(target.contains("pub fn get"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A typed local alias is an owner when its type is the owner, even without a
/// receiver initializer, and an untyped annotation still follows the receiver.
#[test]
fn a_typed_local_alias_binds_with_or_without_a_receiver() {
    let from_receiver = super::visibility::method_fields("fn f(&self) -> i32 { let me: &S = self; me.value }", "S");
    assert!(from_receiver.contains("value"), "{from_receiver:?}");
    let from_call = super::visibility::method_fields("fn f(&self) -> i32 { let me: &S = lookup(); me.value }", "S");
    assert!(from_call.contains("value"), "{from_call:?}");
    let inferred = super::visibility::method_fields("fn f(&self) -> i32 { let me: &_ = self; me.value }", "S");
    assert!(inferred.contains("value"), "{inferred:?}");
}

#[test]
fn a_shadowed_local_alias_is_not_an_owner_receiver() {
    let used = super::visibility::method_fields("fn f(&self) -> i32 { let me = self; let me = 3; me.value }", "S");
    assert!(used.is_empty(), "{used:?}");
}

#[test]
fn a_typed_closure_parameter_is_an_owner_receiver() {
    let used = super::visibility::method_fields("fn f(items: &Vec<S>) { let _ = items.iter().map(|s: &S| s.value); }", "S");
    assert!(used.contains("value"), "{used:?}");
}

#[test]
fn a_field_read_from_an_owner_iterator_is_promoted() {
    for method in ["iter", "iter_mut", "into_iter"] {
        let source = format!("fn f(items: &Vec<S>) {{ for s in items.{method}() {{ let _ = s.value; }} }}");
        let used = super::visibility::method_fields(&source, "S");
        assert!(used.contains("value"), "{method}: {used:?}");
    }
}

/// Every pattern form that binds an owner alias's name shadows it.
#[test]
fn a_pattern_that_rebinds_an_owner_alias_is_not_an_owner_receiver() {
    let cases = [
        ("for loop", "fn f(&self, items: &Vec<u8>) -> i32 { let me = self; for me in items { me.value } }"),
        ("if let", "fn f(&self, other: Option<u8>) -> i32 { let me = self; if let Some(me) = other { me.value } }"),
        ("match arm", "fn f(&self, other: Option<u8>) -> i32 { let me = self; match other { Some(me) => me.value, None => 0 } }"),
        ("destructuring let", "fn f(&self) -> i32 { let me = self; let (me, _) = (3, 4); me.value }"),
        ("closure parameter", "fn f(&self) -> i32 { let me = self; let _ = |(me, _)| me.value; 0 }"),
        ("while let", "fn f(&self, other: Option<u8>) -> i32 { let me = self; while let Some(me) = other { me.value; } 0 }"),
    ];
    for (pattern, text) in cases {
        let used = super::visibility::method_fields(text, "S");
        assert!(used.is_empty(), "{pattern}: {used:?}");
    }
}

/// A pattern's shadow ends with its scope: the outer alias is a receiver again.
#[test]
fn a_shadowing_pattern_restores_the_outer_alias() {
    let cases = [
        ("after a for loop", "fn f(&self, items: &Vec<u8>) -> i32 { let me = self; for me in items { } me.value }"),
        ("after an if let", "fn f(&self, other: Option<u8>) -> i32 { let me = self; if let Some(me) = other { } me.value }"),
        ("in an else branch", "fn f(&self, other: Option<u8>) -> i32 { let me = self; if let Some(me) = other { 0 } else { me.value } }"),
        (
            "in a let else block",
            "fn f(&self, other: Option<u8>) -> i32 { let me = self; let Some(me) = other else { return me.value }; 0 }",
        ),
        (
            "after a match",
            "fn f(&self, other: Option<u8>) -> i32 { let me = self; match other { Some(me) => me.value, None => 0 }; me.value }",
        ),
        ("after a while let", "fn f(&self, other: Option<u8>) -> i32 { let me = self; while let Some(me) = other { } me.value }"),
    ];
    for (scope, text) in cases {
        let used = super::visibility::method_fields(text, "S");
        assert!(used.contains("value"), "{scope}: {used:?}");
    }
}

/// An inline test container in the target still refuses a moved test file.
#[test]
fn an_inline_test_container_in_the_target_is_refused_with_e1004() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n}\n",
    ));
    s.submit_symbols(None, Some("helpers"), "#[cfg(test)]\nmod tests {\n    #[test]\n    fn existing() {}\n}\n").unwrap();
    let err = extract(&s, &["crate.make", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("tests"), "{err}");
}

/// A nested target's test file sits beside its own file, not at the crate root.
#[test]
fn a_nested_target_writes_its_test_file_beside_it() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod board;\n"),
        (
            "src/board.rs",
            concat!(
                "pub mod reviews;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
                "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
            ),
        ),
        ("src/board/reviews.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["board.make", "--to", "board::reviews"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "board::reviews.tests::make_works").is_ok());
    let row = ix.modules.iter().find(|m| m.path == "board::reviews::tests").unwrap();
    assert_eq!(row.dir, "src/board");
    let target = written(&root, "src/board/reviews.rs");
    assert!(target.contains("#[cfg(test)]\n#[path = \"reviews_tests.rs\"]\nmod tests;\n"), "{target}");
    let tests = written(&root, "src/board/reviews_tests.rs");
    assert!(tests.contains("fn make_works"), "{tests}");
    assert!(!tests.contains("other_works"), "{tests}");
}

/// Tests held in a `#[path]` file stay in a file when they move.
#[test]
fn tests_from_a_path_container_move_to_a_file() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod board;\n"),
        (
            "src/board.rs",
            concat!(
                "pub mod reviews;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "#[cfg(test)]\n#[path = \"board_tests.rs\"]\nmod tests;\n",
            ),
        ),
        (
            "src/board_tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    assert_eq!(make(), 1);\n}\n\n",
                "#[test]\nfn other_works() {\n    assert_eq!(other(), 2);\n}\n",
            ),
        ),
        ("src/board/reviews.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["board.make", "--to", "board::reviews"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "board::reviews.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "board.tests::other_works").is_ok());
    let source = written(&root, "src/board_tests.rs");
    assert!(source.contains("fn other_works") && !source.contains("fn make_works"), "{source}");
    let tests = written(&root, "src/board/reviews_tests.rs");
    assert!(tests.contains("fn make_works") && !tests.contains("fn other_works"), "{tests}");
}

/// A cargo failure removes the test file and restores the target's header and rows.
#[test]
fn a_failed_test_move_rolls_back_its_file() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub struct S {\n    field: i32,\n}\n\n",
                "pub fn make() -> S {\n    S { field: 1 }\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        let _ = make();\n    }\n\n",
                "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn existing() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let before = written(&root, "src/b.rs");
    let history: i64 = s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
    let err = extract(&s, &["a.make", "--to", "b"]).unwrap_err().0;
    assert!(err.starts_with("Not moved:"), "{err}");
    assert!(!root.join("src/b_tests.rs").exists());
    assert_eq!(written(&root, "src/b.rs"), before);
    let ix = s.index().unwrap();
    assert!(ix.modules.iter().all(|m| m.path != "b::tests"));
    assert_eq!(s.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0)).unwrap(), history);
}

/// A test file keeps one `use super::*`, adding it only when it is missing.
#[test]
fn a_test_file_header_gets_one_super_glob() {
    assert_eq!(with_super(""), "use super::*;\n");
    assert_eq!(with_super("use std::fmt;\n"), "use std::fmt;\nuse super::*;\n");
    assert_eq!(with_super("use super::*;\n"), "use super::*;\n");
}

/// Two moved tests that would share one name in the target are refused.
#[test]
fn two_source_tests_with_the_same_name_are_refused() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub fn make_a() -> i32 {\n    1\n}\n\n",
                "pub fn keep_a() -> i32 {\n    2\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        assert_eq!(make_a(), 1);\n    }\n\n",
                "    #[test]\n    fn keep_works() {\n        assert_eq!(keep_a(), 2);\n    }\n}\n",
            ),
        ),
        (
            "src/b.rs",
            concat!(
                "pub fn make_b() -> i32 {\n    3\n}\n\n",
                "pub fn keep_b() -> i32 {\n    4\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        assert_eq!(make_b(), 3);\n    }\n\n",
                "    #[test]\n    fn keep_works() {\n        assert_eq!(keep_b(), 4);\n    }\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = extract(&s, &["a.make_a", "b.make_b", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("make_works"), "{err}");
}

/// A whole inline test container keeps the imports its moved tests need.
#[test]
fn a_whole_inline_test_container_carries_its_imports() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::fmt;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "pub fn plain() -> i32 {\n    3\n}\n\n",
                "pub fn show(value: impl fmt::Debug) -> String {\n    format!(\"{value:?}\")\n}\n\n",
                "#[cfg(test)]\nmod tests {\n",
                "    use super::*;\n",
                "    use std::collections::HashMap;\n\n",
                "    #[test]\n    fn make_works() {\n        let mut map = HashMap::new();\n",
                "        map.insert(1, make());\n        assert_eq!(map[&1], 1);\n    }\n\n",
                "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n\n",
                "    #[test]\n    fn plain_works() {\n        assert_eq!(plain(), 3);\n    }\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.plain", "--to", "b"]).unwrap();
    extract(&s, &["a.make", "a.other", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "b.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "b.tests::other_works").is_ok());
    assert!(s.resolve(&ix, "a.tests::make_works").is_err());
    let tests = written(&root, "src/b_tests.rs");
    assert!(tests.contains("use std::collections::HashMap;"), "{tests}");
    assert!(!tests.contains("use std::fmt;"), "{tests}");
    assert!(tests.contains("fn make_works") && tests.contains("fn other_works"), "{tests}");
}

/// A whole `#[path]` test container carries its file header imports.
#[test]
fn a_path_test_file_header_import_moves_with_its_tests() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "pub fn plain() -> i32 {\n    3\n}\n\n",
                "#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n",
            ),
        ),
        (
            "src/a_tests.rs",
            concat!(
                "use super::*;\n",
                "use std::collections::HashMap;\n\n",
                "#[test]\nfn make_works() {\n    let mut map = HashMap::new();\n",
                "    map.insert(1, make());\n    assert_eq!(map[&1], 1);\n}\n\n",
                "#[test]\nfn other_works() {\n    assert_eq!(other(), 2);\n}\n\n",
                "#[test]\nfn plain_works() {\n    assert_eq!(plain(), 3);\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.plain", "--to", "b"]).unwrap();
    extract(&s, &["a.make", "a.other", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "b.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "b.tests::other_works").is_ok());
    let tests = written(&root, "src/b_tests.rs");
    assert!(tests.contains("use std::collections::HashMap;"), "{tests}");
    assert!(tests.contains("fn make_works") && tests.contains("fn other_works"), "{tests}");
}

/// An inline test container does not carry the source module's relative imports.
#[test]
fn an_inline_test_move_leaves_module_relative_imports_behind() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "mod inner;\n",
                "use inner::Thing;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "pub fn plain() -> i32 {\n    3\n}\n\n",
                "pub fn keep(thing: Thing) -> i32 {\n    thing.value()\n}\n\n",
                "#[cfg(test)]\nmod tests {\n",
                "    use super::*;\n",
                "    use std::collections::HashMap;\n\n",
                "    #[test]\n    fn make_works() {\n        let mut map = HashMap::new();\n",
                "        map.insert(1, make());\n        assert_eq!(map[&1], 1);\n    }\n\n",
                "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n\n",
                "    #[test]\n    fn plain_works() {\n        assert_eq!(plain(), 3);\n    }\n}\n",
            ),
        ),
        ("src/a/inner.rs", "pub struct Thing;\n\nimpl Thing {\n    pub fn value(&self) -> i32 {\n        4\n    }\n}\n"),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.plain", "--to", "b"]).unwrap();
    extract(&s, &["a.make", "a.other", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "b.tests::make_works").is_ok());
    let tests = written(&root, "src/b_tests.rs");
    assert!(tests.contains("use std::collections::HashMap;"), "{tests}");
    assert!(!tests.contains("use inner::Thing;"), "{tests}");
    let a = written(&root, "src/a.rs");
    assert!(a.contains("use inner::Thing;"), "{a}");
}

/// A moved non-test item's import need does not carry an inline test container's import.
#[test]
fn a_moved_fn_does_not_carry_an_unused_inline_test_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::collections::HashMap;\n\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "pub fn plain() -> i32 {\n    3\n}\n\n",
                "#[cfg(test)]\nmod tests {\n",
                "    use super::*;\n",
                "    use std::collections::HashMap;\n\n",
                "    #[test]\n    fn make_works() {\n        assert!(make().is_empty());\n    }\n\n",
                "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n\n",
                "    #[test]\n    fn plain_works() {\n        assert_eq!(plain(), 3);\n    }\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.plain", "--to", "b"]).unwrap();
    extract(&s, &["a.make", "a.other", "--to", "b"]).unwrap();
    let tests = written(&root, "src/b_tests.rs");
    assert!(!tests.contains("use std::collections::HashMap;"), "{tests}");
    assert!(tests.contains("fn make_works") && tests.contains("fn other_works"), "{tests}");
    let b = written(&root, "src/b.rs");
    assert!(b.contains("use std::collections::HashMap;"), "{b}");
}

/// A helper used beside staying code by a test in a `#[path]` file moves; the
/// test stays and its glob now names the helper's full path.
#[test]
fn a_path_test_that_uses_staying_code_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod board;\n"),
        (
            "src/board.rs",
            concat!(
                "fn make() -> i32 {\n    1\n}\n\n",
                "pub fn other() -> i32 {\n    2\n}\n\n",
                "pub fn keep() -> i32 {\n    make() + other()\n}\n\n",
                "#[cfg(test)]\n#[path = \"board_tests.rs\"]\nmod tests;\n",
            ),
        ),
        ("src/board_tests.rs", "use super::*;\n\n#[test]\nfn both_work() {\n    assert_eq!(make() + other(), 3);\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["board.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.make").is_ok());
    assert!(s.resolve(&ix, "board.tests::both_work").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::both_work").is_err());
    let test = written(&root, "src/board_tests.rs");
    assert!(test.contains("use crate::helpers::make;"), "{test}");
    assert!(test.contains("make() + other()"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A helper a staying test uses cannot move, so the test that uses it beside
/// moved code stays too.
#[test]
fn a_test_sharing_a_helper_with_a_staying_test_stays() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "pub fn other() -> i32 {\n    2\n}\n\n",
        "pub fn keep() -> i32 {\n    make() + other()\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    fn check(value: i32) {\n        assert!(value > 0);\n    }\n\n",
        "    #[test]\n    fn make_works() {\n        check(make());\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        check(other());\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.make").is_ok());
    assert!(s.resolve(&ix, "crate.tests::check").is_ok());
    assert!(s.resolve(&ix, "crate.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::other_works").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_err());
    let test = text(&s, "crate.tests::make_works");
    assert!(test.contains("check(make())"), "{test}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A test module made by `submit --new a::tests` takes a later extract's test.
#[test]
fn a_test_move_appends_to_a_submit_created_test_file() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod a;\n\n",
                "fn make() -> i32 {\n    1\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n}\n",
            ),
        ),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    s.submit_symbols(None, Some("a::tests"), "use super::*;\n\n#[test]\nfn a_works() {\n    assert_eq!(one(), 1);\n}\n").unwrap();
    extract(&s, &["crate.make", "--to", "a"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "a.tests::a_works").is_ok());
    assert!(s.resolve(&ix, "a.tests::make_works").is_ok());
    let module = written(&root, "src/a.rs");
    assert_eq!(module.matches("#[path = \"a_tests.rs\"]").count(), 1, "{module}");
    assert_eq!(module.matches("mod tests;").count(), 1, "{module}");
    let tests = written(&root, "src/a_tests.rs");
    assert!(tests.contains("fn a_works") && tests.contains("fn make_works"), "{tests}");
    assert_eq!(tests.matches("use super::*;").count(), 1, "{tests}");
}

#[test]
fn a_child_glob_keeps_its_parents_imports() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod parent;\npub mod store;\npub mod traits;\n\n",
                "pub fn use_moved() {\n    parent::moved();\n}\n\n",
                "pub fn use_parent() -> i32 {\n    parent::marker()\n}\n\n",
                "pub fn marker() -> i32 {\n    1\n}\n\n",
                "#[macro_export]\nmacro_rules! answer {\n    () => {\n        42\n    };\n}\n",
            ),
        ),
        ("src/store.rs", "pub struct Store;\n"),
        (
            "src/traits.rs",
            concat!(
                "pub trait Speak {\n    fn speak(&self) -> i32;\n}\n\n",
                "impl Speak for i32 {\n    fn speak(&self) -> i32 {\n        *self\n    }\n}\n",
            ),
        ),
        (
            "src/parent.rs",
            concat!(
                "use crate::store::Store;\n",
                "use std::collections::HashMap;\n",
                "use std::collections::*;\n",
                "use crate::traits::Speak;\n",
                "use crate::answer;\n\n",
                "pub fn moved() -> (Store, HashMap<i32, i32>, BTreeSet<i32>, i32) {\n",
                "    let mut map = HashMap::new();\n",
                "    map.insert(1, 2);\n",
                "    let mut set = BTreeSet::new();\n",
                "    set.insert((1).speak());\n",
                "    (Store, map, set, answer!())\n",
                "}\n\n",
                "pub fn marker() -> i32 {\n    mid::marker_too()\n}\n\n",
                "#[path = \"parent_mid.rs\"]\nmod mid;\n",
            ),
        ),
        (
            "src/parent_mid.rs",
            concat!(
                "use super::*;\n\n",
                "pub fn marker_too() -> i32 {\n    tests::imports_are_visible()\n}\n\n",
                "#[path = \"parent_tests.rs\"]\nmod tests;\n",
            ),
        ),
        (
            "src/parent_tests.rs",
            concat!(
                "use super::*;\n\n",
                "pub fn imports_are_visible() -> i32 {\n",
                "    let _: Store = Store;\n",
                "    let _: HashMap<i32, i32> = HashMap::new();\n",
                "    let _: BTreeSet<i32> = BTreeSet::new();\n",
                "    marker() + marker_too() + more_imports_are_visible() + answer!() + (1).speak()\n",
                "}\n\n",
                "pub fn more_imports_are_visible() -> i32 {\n",
                "    let _: Store = Store;\n",
                "    let _: HashMap<i32, i32> = HashMap::new();\n",
                "    let _: BTreeSet<i32> = BTreeSet::new();\n",
                "    marker() + marker_too() + answer!() + (1).speak()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let before = id(&s, "parent.moved");
    extract(&s, &["parent.moved", "--to", "moved"]).unwrap();
    assert_eq!(id(&s, "moved.moved"), before);
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "parent.mid.tests::imports_are_visible").is_ok());
    let parent = written(&root, "src/parent.rs");
    for import in ["use crate::store::Store;", "use std::collections::HashMap;", "use crate::traits::Speak;", "use crate::answer;"] {
        assert!(!parent.contains(import), "unused {import}: {parent}");
    }
    assert!(!parent.contains("use std::collections::*;"), "{parent}");
    let tests = written(&root, "src/parent_tests.rs");
    for import in [
        "use crate::store::Store;",
        "use std::collections::HashMap;",
        "use std::collections::BTreeSet;",
        "use crate::traits::Speak;",
        "use crate::answer;",
    ] {
        assert_eq!(tests.matches(import).count(), 1, "missing or repeated {import}: {tests}");
    }
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_child_glob_keeps_a_moved_use() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod helpers;\n\n",
                "pub fn keep() -> i32 {\n    mid::marker_too()\n}\n\n",
                "pub use std::collections::HashMap;\n\n",
                "pub fn marker() -> i32 {\n    1\n}\n\n",
                "#[path = \"mid.rs\"]\nmod mid;\n",
            ),
        ),
        ("src/helpers.rs", "pub fn other() -> i32 {\n    1\n}\n"),
        (
            "src/mid.rs",
            "use super::*;\n\npub fn marker_too() -> i32 {\n    tests::map_is_visible()\n}\n\n#[path = \"grand.rs\"]\nmod tests;\n",
        ),
        (
            "src/grand.rs",
            concat!(
                "use super::*;\n\n",
                "pub fn map_is_visible() -> i32 {\n",
                "    let _: HashMap<i32, i32> = HashMap::new();\n",
                "    marker() + marker_too()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.std::collections::HashMap", "--to", "helpers"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(!source.contains("use std::collections::HashMap;"), "{source}");
    assert!(!source.contains("allow(unused_imports)"), "{source}");
    assert!(!source.contains("pub use std::collections::HashMap;"), "{source}");
    let grand = written(&root, "src/grand.rs");
    assert!(grand.contains("use std::collections::HashMap;"), "{grand}");
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "mid.tests::map_is_visible").is_ok());
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Moving one test module's helpers leaves another test module's imports alone.
#[test]
fn a_test_module_move_keeps_a_siblings_imports() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n\npub struct Store;\n\npub type Out = String;\n"),
        ("src/a.rs", "pub mod more;\n\npub fn make() -> i32 {\n    1\n}\n\nmod tests;\n"),
        (
            "src/a/tests.rs",
            concat!(
                "#![cfg(test)]\n",
                "use crate::Store;\n\n",
                "pub fn ready() -> bool {\n    true\n}\n\n",
                "pub fn text(_s: &Store) -> String {\n    String::new()\n}\n\n",
                "pub fn id(_s: &Store) -> String {\n    String::new()\n}\n\n",
                "#[test]\nfn make_works() {\n    assert_eq!(super::make(), 1);\n    assert!(ready());\n}\n",
            ),
        ),
        ("src/a/more.rs", "pub fn other() -> i32 {\n    2\n}\n\nmod tests;\n"),
        (
            "src/a/more/tests.rs",
            concat!(
                "#![cfg(test)]\n",
                "use crate::{Out, Store};\n\n",
                "use super::super::tests::{id, ready, text};\n\n",
                "fn fold(s: &Store) -> Out {\n    id(s)\n}\n\n",
                "#[test]\nfn helpers_are_visible() {\n    assert!(ready());\n    let _ = text;\n    let _ = fold;\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a::tests.ready", "a::tests.text", "a::tests.id", "--to", "a::parked"]).unwrap();
    let ix = s.index().unwrap();
    let header = &ix.modules.iter().find(|m| m.path == "a::more::tests").unwrap().header;
    assert!(header.contains("use crate::{Out, Store};"), "{header}");
    assert!(header.contains("use crate::a::parked::id;"), "{header}");
    assert!(header.contains("use crate::a::parked::ready;"), "{header}");
    assert!(header.contains("use crate::a::parked::text;"), "{header}");
    let tests = written(&root, "src/a/more/tests.rs");
    assert!(tests.contains("use crate::{Out, Store};"), "{tests}");
}

#[test]
fn moved_struct_fields_used_by_staying_crate_root_code_become_pub_crate() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "struct Failure(String, i32);\n\n",
                "pub(crate) fn report() -> Failure {\n    Failure(String::from(\"x\"), 1)\n}\n\n",
                "pub(crate) fn code(f: &Failure) -> (&str, i32) {\n    (f.0.as_str(), f.1)\n}\n\n",
                "fn main() {\n    println!(\"{:?}\", code(&report()));\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "diagnostics"]).unwrap();
    let target = written(&root, "src/diagnostics.rs");
    assert!(target.contains("pub(crate) struct Failure(pub(crate) String, pub(crate) i32)"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(source.contains("crate::diagnostics::Failure"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_crate_root_import_moves_to_a_child_module_as_a_crate_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "mod language;\n\n",
                "use language::Language;\n\n",
                "struct Thing;\n\n",
                "impl Thing {\n    fn label(&self) -> &'static str {\n        1.name()\n    }\n}\n\n",
                "fn main() {\n    println!(\"{}\", Thing.label());\n}\n",
            ),
        ),
        (
            "src/language.rs",
            concat!(
                "pub trait Language {\n    fn name(&self) -> &'static str;\n}\n\n",
                "impl Language for i32 {\n    fn name(&self) -> &'static str {\n        \"one\"\n    }\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Thing", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("use crate::language::Language;"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(!source.contains("use language::Language;"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn moved_code_names_a_staying_crate_root_item_by_its_crate_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "mod reference;\n\n",
                "use reference::REFERENCE;\n\n",
                "pub(crate) const ROOT_REFERENCE: &str = \"root\";\n\n",
                "fn options_of() -> (&'static str, &'static str) {\n    (REFERENCE, ROOT_REFERENCE)\n}\n\n",
                "fn main() {\n    println!(\"{:?}\", options_of());\n}\n",
            ),
        ),
        ("src/reference.rs", "pub(crate) const REFERENCE: &str = \"x\";\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.options_of", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("crate::reference::REFERENCE"), "{target}");
    assert!(target.contains("crate::ROOT_REFERENCE"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(!source.contains("use reference::REFERENCE;"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn moved_code_carries_the_trait_its_method_call_needs() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "use std::io::Read;\n\n",
                "fn read_input() -> std::io::Result<String> {\n",
                "    let mut text = String::new();\n",
                "    std::io::stdin().read_to_string(&mut text)?;\n",
                "    Ok(text)\n",
                "}\n\n",
                "fn main() {\n    let _ = read_input();\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.read_input", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("use std::io::Read;"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(!source.contains("use std::io::Read;"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_generic_associated_type_binding_stays_unqualified() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "mod language;\n\n",
                "pub use language::Item;\n\n",
                "pub struct Diag;\n\n",
                "fn diags() -> impl Iterator<Item = &'static Diag> {\n    std::iter::empty()\n}\n\n",
                "fn main() {\n    let _ = diags();\n}\n",
            ),
        ),
        ("src/language.rs", "pub struct Item;\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.diags", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("Iterator<Item = "), "{target}");
    assert!(!target.contains("crate::language::Item"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// The diagnostics types and helpers move from the crate root, and fields
/// that staying code constructs, matches or reads become reachable.
#[test]
fn the_crate_root_diagnostics_types_move_to_diagnostics() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "#[derive(Debug)]\n",
                "struct Failure(String, i32);\n\n",
                "impl From<std::io::Error> for Failure {\n",
                "    fn from(e: std::io::Error) -> Self {\n",
                "        Failure(e.to_string(), 1)\n",
                "    }\n",
                "}\n\n",
                "type Out = Result<String, Failure>;\n\n",
                "struct Diag {\n",
                "    error: bool,\n",
                "    module: Option<String>,\n",
                "    text: String,\n",
                "}\n\n",
                "struct FileMap {\n",
                "    module: String,\n",
                "    file: std::path::PathBuf,\n",
                "    ranges: Vec<(usize, usize, String)>,\n",
                "}\n\n",
                "fn fail(message: &str) -> Failure {\n",
                "    Failure(message.to_string(), 1)\n",
                "}\n\n",
                "fn place(d: &Diag) -> String {\n",
                "    d.module.clone().unwrap_or_else(|| d.text.clone())\n",
                "}\n\n",
                "fn locate<'a>(files: &'a [FileMap], path: &str) -> Option<(&'a str, usize)> {\n",
                "    let map = files.iter().find(|f| f.file.to_str() == Some(path))?;\n",
                "    Some((map.module.as_str(), map.ranges.len()))\n",
                "}\n\n",
                "fn status() -> Out {\n",
                "    Ok(\"done\".to_string())\n",
                "}\n\n",
                "fn read() -> Result<String, Failure> {\n",
                "    Ok(std::fs::read_to_string(\"x\")?)\n",
                "}\n\n",
                "fn main() {\n",
                "    let f = fail(\"bad\");\n",
                "    println!(\"{} {}\", f.0, f.1);\n",
                "    let Failure(message, code) = fail(\"worse\");\n",
                "    println!(\"{} {}\", message, code);\n",
                "    let d = Diag { error: true, module: None, text: \"x\".to_string() };\n",
                "    println!(\"{} {}\", place(&d), d.error);\n",
                "    let m = FileMap { module: \"a\".to_string(), file: \"a.rs\".into(), ranges: vec![] };\n",
                "    let out: Out = status();\n",
                "    println!(\"{:?} {:?}\", locate(&[m], \"a.rs\"), out);\n",
                "    match read() {\n",
                "        Ok(text) => println!(\"{}\", text.len()),\n",
                "        Err(Failure(message, code)) => println!(\"{} {}\", message, code),\n",
                "    }\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(
        &s,
        &[
            "crate.Failure",
            "crate.Out",
            "crate.Diag",
            "crate.FileMap",
            "crate.fail",
            "crate.place",
            "crate.locate",
            "crate.status",
            "--to",
            "diagnostics",
        ],
    )
    .unwrap();
    let target = written(&root, "src/diagnostics.rs");
    for field in [
        "pub(crate) String",
        "pub(crate) i32",
        "pub(crate) error: bool",
        "pub(crate) module: Option<String>",
        "pub(crate) text: String",
        "pub(crate) module: String",
        "pub(crate) file: std::path::PathBuf",
        "pub(crate) ranges: Vec<(usize, usize, String)>",
    ] {
        assert!(target.contains(field), "{field} in {target}");
    }
    let source = written(&root, "src/main.rs");
    assert!(source.contains("use crate::diagnostics::Failure;"), "{source}");
    assert!(source.contains("use crate::diagnostics::Diag;"), "{source}");
    assert!(source.contains("let Failure(message, code) = fail(\"worse\");"), "{source}");
    assert!(source.contains("let d = Diag {"), "{source}");
    assert!(target.contains("impl From<std::io::Error> for Failure"), "{target}");
    assert!(!source.contains("impl From<std::io::Error> for Failure"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// `stdin` and `options_of` move from the crate root with their trait and
/// root imports, and staying code keeps the imports it still uses.
#[test]
fn the_crate_root_cli_helpers_move_to_cli() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "mod reference;\n\n",
                "use std::io::Read;\n\n",
                "use reference::REFERENCE;\n\n",
                "#[derive(Debug)]\n",
                "pub(crate) struct Failure(String, i32);\n\n",
                "impl From<std::io::Error> for Failure {\n",
                "    fn from(e: std::io::Error) -> Self {\n",
                "        Failure(e.to_string(), 1)\n",
                "    }\n",
                "}\n\n",
                "fn stdin() -> Result<String, Failure> {\n",
                "    let mut text = String::new();\n",
                "    std::io::stdin().read_to_string(&mut text)?;\n",
                "    Ok(text)\n",
                "}\n\n",
                "fn options_of(command: &str) -> Vec<&'static str> {\n",
                "    let mut out = vec![];\n",
                "    for line in REFERENCE.lines() {\n",
                "        if line.trim_start().starts_with(command) {\n",
                "            out.push(\"--x\");\n",
                "        }\n",
                "    }\n",
                "    out\n",
                "}\n\n",
                "fn main() {\n",
                "    let _ = stdin();\n",
                "    let failure = Failure(\"bad\".to_string(), 1);\n",
                "    println!(\"{} {}\", failure.0, failure.1);\n",
                "    println!(\"{:?}\", options_of(\"run\"));\n",
                "}\n",
            ),
        ),
        ("src/reference.rs", "pub(crate) const REFERENCE: &str = \"  run  --x\\n\";\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.stdin", "crate.options_of", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("use std::io::Read;"), "{target}");
    assert!(target.contains("crate::reference::REFERENCE"), "{target}");
    assert!(target.contains("crate::Failure"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(!source.contains("use std::io::Read;"), "{source}");
    assert!(!source.contains("use reference::REFERENCE;"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// `str::lines` is inherent: a `BufRead` import in the source is not carried.
#[test]
fn a_string_lines_call_does_not_carry_buf_read() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "use std::io::BufRead;\n\n",
                "fn count_words(text: &str) -> usize {\n",
                "    text.lines().count()\n",
                "}\n\n",
                "fn read_lines(reader: &mut impl BufRead) -> usize {\n",
                "    reader.lines().count()\n",
                "}\n\n",
                "fn main() {\n",
                "    let mut cursor = std::io::Cursor::new(b\"a\\n\".to_vec());\n",
                "    println!(\"{}\", count_words(\"a b\") + read_lines(&mut cursor));\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.count_words", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(!target.contains("BufRead"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(source.contains("use std::io::BufRead;"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A staying read through a moved constructor's return type reaches every
/// field, even when the staying code never names the struct.
#[test]
fn moved_struct_fields_used_through_a_moved_constructor_become_pub_crate() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "#[derive(Debug)]\n",
                "struct Failure(String, i32);\n\n",
                "fn report() -> Failure {\n",
                "    Failure(\"x\".to_string(), 1)\n",
                "}\n\n",
                "fn main() {\n",
                "    let failure = report();\n",
                "    println!(\"{} {}\", failure.0, failure.1);\n",
                "    println!(\"{}\", report().1);\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "crate.report", "--to", "diagnostics"]).unwrap();
    let target = written(&root, "src/diagnostics.rs");
    assert!(target.contains("pub(crate) struct Failure(pub(crate) String, pub(crate) i32)"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A trait impl named by its row moves whole.
#[test]
fn a_trait_impl_row_moves_whole() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "pub trait Named {\n    fn name(&self) -> u8;\n}\n\n",
                "pub struct S;\n\n",
                "impl Named for S {\n    fn name(&self) -> u8 {\n        0\n    }\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn other() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["a.<S as Named>", "--to", "b"]).unwrap();
    assert!(out.contains("Impls moved: a.<S as Named>"), "{out}");
    assert!(written(&root, "src/b.rs").contains("impl crate::a::Named for crate::a::S"));
    assert!(!written(&root, "src/a.rs").contains("impl Named for S"));
}

/// A promoted moved method is named where it lands in the reply.
#[test]
fn a_moved_item_needing_visibility_is_named_by_its_new_path() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "pub struct S;\n\n",
        "impl S {\n",
        "    fn make() -> S {\n",
        "        S\n",
        "    }\n",
        "}\n\n",
        "pub fn keep() -> S {\n",
        "    S::make()\n",
        "}\n",
    ));
    let out = extract(&s, &["crate.S::make", "--to", "helpers"]).unwrap();
    let line = out.lines().find(|l| l.starts_with("Now pub(crate):")).unwrap_or("");
    assert_eq!(line, "Now pub(crate): helpers.S::make", "{out}");
}

#[test]
fn a_moved_method_stays_private_for_a_child_user() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub struct S;\n\nimpl S {\n    fn hidden(&self) -> i32 {\n        1\n    }\n}\n"),
        ("src/b.rs", "pub mod child;\n"),
        ("src/b/child.rs", "use crate::a::S;\n\npub fn use_it() -> i32 {\n    S.hidden()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.S::hidden", "--to", "b"]).unwrap();
    let target = written(&root, "src/b.rs");
    assert!(target.contains("fn hidden(&self)"), "{target}");
    assert!(!target.contains("pub(crate) fn hidden"), "{target}");
    assert!(!target.contains("pub fn hidden"), "{target}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn two_redirected_uses_in_one_group_keep_their_order() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        (
            "src/a.rs",
            concat!("pub fn one() -> i32 {\n    1\n}\n\n", "pub fn two() -> i32 {\n    2\n}\n\n", "pub fn other() -> i32 {\n    3\n}\n",),
        ),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/callers.rs", concat!("use crate::a::{two, one, other};\n\n", "pub fn go() -> i32 {\n    one() + two() + other()\n}\n",)),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.one", "a.two", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.contains("use crate::b::one;\nuse crate::b::two;"), "{}", callers.header);
}

#[test]
fn a_redirected_use_follows_the_group_it_left() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod callers;\n"),
        ("src/a.rs", "pub fn other() -> i32 {\n    3\n}\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        (
            "src/callers.rs",
            concat!(
                "use crate::a::{other, one};\nuse std::cmp::Ordering;\n\n",
                "pub fn go() -> i32 {\n    let _ = Ordering::Less;\n    one() + other()\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.one", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let callers = ix.modules.iter().find(|m| m.path == "callers").unwrap();
    assert!(callers.header.starts_with("use crate::a::other;\nuse crate::b::one;\nuse std::cmp::Ordering;"), "{}", callers.header);
}

/// A staying call after `||`, with a later closure beside it, is a use and
/// takes the moved function's path.
#[test]
fn a_staying_call_after_a_logical_or_is_rewritten() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "#[derive(Debug)]\n",
                "pub(crate) struct Failure(String, i32);\n\n",
                "fn fail<T>(message: String) -> Result<T, Failure> {\n",
                "    Err(Failure(message, 1))\n",
                "}\n\n",
                "fn usage<T>(message: &str) -> Result<T, Failure> {\n",
                "    fail(format!(\"{}\\n\", message))\n",
                "}\n\n",
                "fn check(blocks: &[String]) -> Result<(), Failure> {\n",
                "    for text in blocks {\n",
                "        if text.is_empty() || (text.len() > 1) {\n",
                "            return fail(format!(\n",
                "                \"bad block {text}\",\n",
                "            ));\n",
                "        }\n",
                "        let length = |m: &str| m.len();\n",
                "        let _ = length(text);\n",
                "    }\n",
                "    Ok(())\n",
                "}\n\n",
                "fn main() {\n",
                "    let out: Result<(), Failure> = usage(\"go\");\n",
                "    if let Err(Failure(message, code)) = out {\n",
                "        println!(\"{} {}\", message, code);\n",
                "    }\n",
                "    let _ = check(&[]);\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.fail", "crate.usage", "--to", "diagnostics"]).unwrap();
    let target = written(&root, "src/diagnostics.rs");
    assert!(target.contains("fn fail"), "{target}");
    assert!(target.contains("fn usage"), "{target}");
    let source = written(&root, "src/main.rs");
    assert!(source.contains("use crate::diagnostics::fail;"), "{source}");
    assert!(source.contains("use crate::diagnostics::usage;"), "{source}");
    assert!(source.contains("return fail(format!("), "{source}");
    assert!(source.contains("let out: Result<(), Failure> = usage(\"go\")"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A `#[cfg(test)]` helper outside a test container moves as an ordinary item,
/// and a test in another module still reaches it through its new path.
#[test]
fn a_cfg_test_helper_outside_tests_moves_as_an_ordinary_item() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "#[cfg(test)]\n",
        "pub(crate) fn helper() -> i32 {\n    1\n}\n\n",
        "pub fn other() -> i32 {\n    2\n}\n\n",
        "mod one {\n",
        "    #[cfg(test)]\n",
        "    mod tests {\n",
        "        use super::super::*;\n\n",
        "        #[test]\n",
        "        fn works() {\n",
        "            assert_eq!(helper() + other(), 3);\n",
        "        }\n",
        "    }\n",
        "}\n",
    ));
    extract(&s, &["crate.helper", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.helper").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::helper").is_err());
    let caller = text(&s, "crate.one::tests::works");
    assert!(caller.contains("assert_eq!(helper() + other(), 3);"), "{caller}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(written(&root, "src/helpers.rs").contains("#[cfg(test)]"));
}

/// A moved function using a second `thread_local!` static gets its full path,
/// since that static has no symbol the ordinary import rewrite could use.
#[test]
fn a_moved_fn_names_a_hidden_thread_local_static() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "thread_local! {\n",
        "    /// The first.\n",
        "    static FIRST: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };\n",
        "    /// The second.\n",
        "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
        "}\n\n",
        "pub fn first() -> u8 {\n    FIRST.with(|v| v.get())\n}\n\n",
        "pub fn second() -> u8 {\n    SECOND.with(|v| v.get())\n}\n",
        "\n",
        "pub fn use_second() -> u8 {\n    second()\n}\n",
    ));
    extract(&s, &["crate.second", "--to", "helpers"]).unwrap();
    let moved = text(&s, "helpers.second");
    assert!(moved.contains("crate::SECOND.with"), "{moved}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A moved function keeps its own module's `SECOND`, though a hidden
/// `thread_local!` static elsewhere has the same name.
#[test]
fn a_moved_fn_keeps_a_same_named_item_in_its_own_module() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod b;\n\n",
                "thread_local! {\n",
                "    static FIRST: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };\n",
                "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "pub(crate) const SECOND: u8 = 7;\n\npub fn value() -> u8 {\n    SECOND\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["b.value", "--to", "helpers"]).unwrap();
    let moved = text(&s, "helpers.value");
    assert!(moved.contains("b::SECOND"), "{moved}");
    assert!(!moved.contains("crate::SECOND"), "{moved}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A moved function reaches a hidden `thread_local!` static its module imports.
#[test]
fn a_moved_fn_names_the_thread_local_its_module_imports() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod b;\n\n",
                "thread_local! {\n",
                "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
                "}\n\n",
                "pub fn use_value() -> u8 {\n",
                "    b::value()\n",
                "}\n",
            ),
        ),
        ("src/b.rs", "use crate::SECOND;\n\npub fn value() -> u8 {\n    SECOND.with(|v| v.get())\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["b.value", "--to", "helpers"]).unwrap();
    let moved = text(&s, "helpers.value");
    assert!(moved.contains("crate::SECOND.with"), "{moved}");
    let diagnostics = s.check_all(&s.index().unwrap()).unwrap();
    let errors: Vec<String> = diagnostics.iter().filter(|d| d.error).map(|d| d.text.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let warnings: Vec<String> = diagnostics.into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Two modules' hidden statics of one name each get their own path, and a move
/// to a sibling makes the static visible.
#[test]
fn a_moved_fn_names_its_own_modules_hidden_thread_local() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "thread_local! {\n",
                "    static FIRST: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };\n",
                "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
                "}\n\n",
                "pub fn one() -> u8 {\n    SECOND.with(|v| v.get())\n}\n",
            ),
        ),
        (
            "src/b.rs",
            concat!(
                "thread_local! {\n",
                "    static FIRST: std::cell::Cell<u8> = const { std::cell::Cell::new(2) };\n",
                "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(3) };\n",
                "}\n\n",
                "pub fn two() -> u8 {\n    SECOND.with(|v| v.get())\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.one", "b.two", "--to", "helpers"]).unwrap();
    let one = text(&s, "helpers.one");
    let two = text(&s, "helpers.two");
    assert!(one.contains("crate::a::SECOND.with"), "{one}");
    assert!(two.contains("crate::b::SECOND.with"), "{two}");
    for block in ["a.FIRST", "b.FIRST"] {
        let text = text(&s, block);
        assert!(text.contains("pub(crate) static SECOND:"), "{block}: {text}");
    }
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn paths_nested_in_a_moved_item_keep_the_crate_root() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "mod outside;\n",
                "mod drafts;\n",
                "mod board;\n",
                "pub mod view;\n\n",
                "fn run_inner() {\n",
                "    outside::absorb(outside::Mode::Notice);\n",
                "    drafts::view();\n",
                "    board::strategy_guard();\n",
                "}\n\n",
                "pub fn entry() {\n",
                "    run_inner();\n",
                "}\n",
            ),
        ),
        ("src/outside.rs", "pub enum Mode {\n    Notice,\n}\n\npub fn absorb(_: Mode) {}\n"),
        ("src/drafts.rs", "pub fn view() {}\n"),
        ("src/view.rs", "pub fn other() {}\n"),
        ("src/board.rs", "mod strategy;\n\npub use strategy::strategy_guard;\n"),
        ("src/board/strategy.rs", "pub fn strategy_guard() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.run_inner", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("crate::outside::absorb(crate::outside::Mode::Notice)"), "{target}");
    assert!(target.contains("crate::drafts::view()"), "{target}");
    assert!(target.contains("crate::board::strategy_guard()"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_format_capture_of_a_staying_constant_is_imported() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub(crate) const PROBE: &str = \"probe\";\n\n",
                "fn prepare() -> String {\n",
                "    format!(\"probe {PROBE}\")\n",
                "}\n\n",
                "pub fn entry() -> String {\n",
                "    prepare()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.prepare", "--to", "probe"]).unwrap();
    let target = written(&root, "src/probe.rs");
    assert!(target.contains("use crate::PROBE;"), "{target}");
    assert!(target.contains("format!(\"probe {PROBE}\")"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_staying_test_imports_a_moved_constant_a_format_string_names() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub(crate) const HELD: &str = \"held\";\n\n",
                "pub fn held() -> &'static str {\n",
                "    HELD\n",
                "}\n\n",
                "#[cfg(test)]\n",
                "mod tests {\n",
                "    use super::*;\n\n",
                "    #[test]\n",
                "    fn shows_held() {\n",
                "        assert_eq!(held(), \"held\");\n",
                "        assert_eq!(format!(\"{HELD}\"), \"held\");\n",
                "    }\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.HELD", "--to", "limits"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("use crate::limits::HELD;"), "{source}");
    assert!(source.contains("format!(\"{HELD}\")"), "{source}");
    let target = written(&root, "src/limits.rs");
    assert!(target.contains("pub(crate) const HELD"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_type_moved_with_its_caller_is_named_by_its_new_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "impl Within {\n",
                "    fn enter() -> Within {\n",
                "        Within\n",
                "    }\n",
                "}\n\n",
                "pub struct Within;\n\n",
                "pub fn run_options() -> Within {\n",
                "    Within::enter()\n",
                "}\n\n",
                "pub fn entry() -> Within {\n",
                "    run_options()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Within", "crate.run_options", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("Within::enter()"), "{target}");
    assert!(!target.contains("crate::Within::enter()"), "{target}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A format capture cannot be imported under a name the destination binds.
#[test]
fn a_format_capture_name_taken_in_the_target_is_refused() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub(crate) const PROBE: &str = \"probe\";\n\n",
                "fn prepare() -> String {\n",
                "    format!(\"probe {PROBE}\")\n",
                "}\n\n",
                "pub mod probe;\n",
            ),
        ),
        ("src/probe.rs", "pub const PROBE: &str = \"taken\";\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = extract(&s, &["crate.prepare", "--to", "probe"]).unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("probe.PROBE"), "{err}");
    assert!(s.resolve(&s.index().unwrap(), "crate.prepare").is_ok());
}

/// A doc comment mentioning the static does not take its visibility.
#[test]
fn a_doc_comment_naming_the_static_is_not_promoted() {
    let text = concat!(
        "thread_local! {\n",
        "    /// A static SECOND: the second value.\n",
        "    #[cfg(test)]\n",
        "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
        "}\n",
    );
    let out = super::promote_hidden_static(text, "SECOND");
    assert!(out.contains("/// A static SECOND:"), "{out}");
    assert!(out.contains("pub(crate) static SECOND:"), "{out}");
    assert!(!out.contains("/// A pub(crate)"), "{out}");
    let raw = concat!(
        "thread_local! {\n",
        "    static NOTE: &str = r#\"\n",
        "static SECOND: x\n",
        "\"#;\n",
        "    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n",
        "}\n",
    );
    let out = super::promote_hidden_static(raw, "SECOND");
    assert!(out.contains("pub(crate) static SECOND: std::cell::Cell"), "{out}");
    assert!(out.contains("\nstatic SECOND: x\n"), "{out}");
    assert_eq!(out.matches("pub(crate) static SECOND:").count(), 1, "{out}");
}

/// A moved function names an aliased hidden static, and its source keeps no
/// unused import.
#[test]
fn a_moved_fn_names_an_aliased_thread_local_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", concat!("pub mod a;\npub mod b;\n\n", "pub fn run() -> u8 {\n    b::value()\n}\n",)),
        ("src/a.rs", "thread_local! {\n    static SECOND: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };\n}\n"),
        (
            "src/b.rs",
            concat!(
                "use crate::a::SECOND as S;\n\n",
                "pub fn value() -> u8 {\n    S.with(|v| v.get())\n}\n\n",
                "pub fn other() -> u8 {\n    2\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["b.value", "--to", "helpers"]).unwrap();
    let moved = text(&s, "helpers.value");
    assert!(moved.contains("crate::a::SECOND.with"), "{moved}");
    let block = text(&s, "a.SECOND");
    assert!(block.contains("pub(crate) static SECOND:"), "{block}");
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(!b.header.contains("SECOND"), "{}", b.header);
    let diagnostics = s.check_all(&ix).unwrap();
    let errors: Vec<String> = diagnostics.iter().filter(|d| d.error).map(|d| d.text.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let warnings: Vec<String> = diagnostics.into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A file-backed test moves with the helper only it reaches.
#[test]
fn a_file_backed_test_moves_with_its_helper() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!("pub fn make() -> i32 {\n    1\n}\n\n", "pub fn keep() -> i32 {\n    make()\n}\n\n", "#[cfg(test)]\nmod tests;\n",),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "fn grouped_site() -> i32 {\n    make()\n}\n\n",
                "#[test]\nfn grouped_reply() {\n    assert_eq!(grouped_site(), 1);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::grouped_reply").is_ok());
    assert!(s.resolve(&ix, "helpers.tests::grouped_site").is_ok());
    assert!(s.resolve(&ix, "helpers.grouped_site").is_err());
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("fn grouped_reply") && tests.contains("fn grouped_site"), "{tests}");
    let helpers = written(&root, "src/helpers.rs");
    assert!(!helpers.contains("grouped_site"), "{helpers}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A moved test carries the crate-root module it saw through `use super::*`.
#[test]
fn a_moved_test_carries_a_crate_root_module() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "mod testing;\n\n",
                "pub mod helpers;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn keep() -> i32 {\n    let _ = testing::Scratch::new();\n    make()\n}\n\n",
                "#[cfg(test)]\nmod tests;\n",
            ),
        ),
        ("src/testing.rs", "pub struct Scratch;\n\nimpl Scratch {\n    pub fn new() -> Self {\n        Scratch\n    }\n}\n"),
        ("src/helpers.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    let _scratch = testing::Scratch::new();\n",
                "    assert_eq!(make(), 1);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "tests::make_works", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use crate::testing;"), "{tests}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A file-backed test that also reaches staying code stays, and the reply
/// names the test and a staying symbol it reaches.
#[test]
fn a_file_backed_test_that_reaches_staying_code_stays_and_is_named() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!("pub fn make() -> i32 {\n    1\n}\n\n", "pub fn keep() -> i32 {\n    2\n}\n\n", "#[cfg(test)]\nmod tests;\n",),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    assert_eq!(make(), 1);\n}\n\n",
                "#[test]\nfn both_work() {\n    assert_eq!(make() + keep(), 3);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "tests.both_work").is_ok());
    assert!(out.contains("stays: tests.both_work (reaches crate.keep, which stays)"), "{out}");
}

/// Several signatures in one module that name a moved type share one import.
#[test]
fn several_moved_type_mentions_share_one_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new(_message: &str) -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        (
            "src/keep.rs",
            concat!(
                "pub fn make(message: &str) -> crate::Failure {\n",
                "    crate::Failure::new(message)\n",
                "}\n\n",
                "pub fn fail(message: &str) -> crate::Failure {\n",
                "    crate::Failure::new(message)\n",
                "}\n\n",
                "pub fn wrap(result: crate::Failure) -> crate::Failure {\n",
                "    result\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert_eq!(keep.matches("use crate::failure::Failure;").count(), 1, "{keep}");
    assert!(keep.contains("pub fn make(message: &str) -> Failure"), "{keep}");
    assert!(keep.contains("pub fn fail(message: &str) -> Failure"), "{keep}");
    assert!(keep.contains("pub fn wrap(result: Failure) -> Failure"), "{keep}");
    assert!(keep.contains("Failure::new(message)"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A module that defines its own `Failure` keeps the moved one's full path.
#[test]
fn a_module_with_its_own_failure_keeps_the_full_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new(_message: &str) -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        (
            "src/keep.rs",
            concat!(
                "pub struct Failure;\n\n",
                "pub fn make(message: &str) -> crate::Failure {\n",
                "    crate::Failure::new(message)\n",
                "}\n\n",
                "pub fn local() -> Failure {\n",
                "    Failure\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert!(!keep.contains("use crate::failure::Failure;"), "{keep}");
    assert!(keep.contains("pub fn make(message: &str) -> crate::failure::Failure"), "{keep}");
    assert!(keep.contains("crate::failure::Failure::new(message)"), "{keep}");
    assert!(keep.contains("pub fn local() -> Failure"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// An import that already names a moved type is retargeted, not duplicated.
#[test]
fn an_existing_import_of_a_moved_type_is_not_duplicated() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new(_message: &str) -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        (
            "src/keep.rs",
            concat!("use crate::Failure;\n\n", "pub fn make(message: &str) -> Failure {\n", "    Failure::new(message)\n", "}\n",),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert_eq!(keep.matches("use crate::failure::Failure;").count(), 1, "{keep}");
    assert!(!keep.contains("use crate::Failure;"), "{keep}");
    assert!(keep.contains("pub fn make(message: &str) -> Failure"), "{keep}");
    assert!(keep.contains("Failure::new(message)"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// An inline test container's `use super::*` carries a crate root module.
#[test]
fn an_inline_test_carries_a_crate_root_module() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "mod testing;\n\n",
                "pub mod helpers;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn keep() -> i32 {\n    let _ = testing::Scratch::new(0);\n    0\n}\n\n",
                "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
                "    #[test]\n    fn make_works() {\n        let _scratch = testing::Scratch::new(0);\n",
                "        assert_eq!(make(), 1);\n    }\n\n",
                "    #[test]\n    fn keep_works() {\n        assert_eq!(keep(), 0);\n    }\n}\n",
            ),
        ),
        ("src/testing.rs", "pub use std::cell::Cell as Scratch;\n"),
        ("src/helpers.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use crate::testing;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A file-backed test carries an import its crate root saw through `use super::*`.
#[test]
fn a_file_backed_test_carries_a_crate_root_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "use std::collections::HashMap;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn keep() -> i32 {\n    let _ = HashMap::<i32, i32>::new();\n    make()\n}\n\n",
                "#[cfg(test)]\nmod tests;\n",
            ),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    let mut map = HashMap::new();\n",
                "    map.insert(1, make());\n    assert_eq!(map[&1], 1);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use std::collections::HashMap;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn a_plain_import_replaces_a_kept_test_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new(_message: &str) -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        (
            "src/keep.rs",
            concat!(
                "#[cfg(test)] use crate::Failure;\n\n",
                "pub fn make(message: &str) -> crate::Failure {\n",
                "    crate::Failure::new(message)\n",
                "}\n\n",
                "#[cfg(test)]\n",
                "pub fn check() {\n",
                "    let _ = make(\"x\");\n",
                "    let _ = Failure::new(\"x\");\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert_eq!(keep.matches("use crate::failure::Failure;").count(), 1, "{keep}");
    assert!(!keep.contains("#[cfg(test)] use"), "{keep}");
    assert!(keep.contains("pub fn make(message: &str) -> Failure"), "{keep}");
    assert!(keep.contains("Failure::new(message)"), "{keep}");
    assert!(keep.contains("let _ = Failure::new(\"x\");"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_test_only_mention_gets_a_test_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "#[allow(dead_code)]\n",
                "pub struct Failure;\n\n",
                "pub fn keep() -> i32 {\n",
                "    1\n",
                "}\n\n",
                "#[cfg(test)]\n",
                "pub fn check() {\n",
                "    let _ = keep();\n",
                "    let _: Failure = Failure;\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert_eq!(source.matches("#[cfg(test)] use crate::failure::Failure;").count(), 1, "{source}");
    assert!(!source.lines().any(|line| line.trim() == "use crate::failure::Failure;"), "{source}");
    assert!(source.contains("let _: Failure = Failure;"), "{source}");
    assert!(!source.contains("let _: crate::failure::Failure"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_header_glob_keeps_the_full_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub mod other;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new() -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/other.rs", "pub struct Failure;\n"),
        (
            "src/keep.rs",
            concat!(
                "use crate::other::*;\n\n",
                "pub fn local() -> Failure {\n",
                "    Failure\n",
                "}\n\n",
                "pub fn make() -> crate::Failure {\n",
                "    crate::Failure::new()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert!(!keep.contains("use crate::failure::Failure;"), "{keep}");
    assert!(keep.contains("pub fn make() -> crate::failure::Failure"), "{keep}");
    assert!(keep.contains("crate::failure::Failure::new()"), "{keep}");
    assert!(keep.contains("pub fn local() -> Failure"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_use_symbol_glob_keeps_the_full_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod keep;\n\n",
                "pub mod other;\n\n",
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new() -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/other.rs", "pub struct Failure;\n"),
        (
            "src/keep.rs",
            concat!(
                "pub fn local() -> Failure {\n",
                "    Failure\n",
                "}\n\n",
                "pub use crate::other::*;\n\n",
                "pub fn make() -> crate::Failure {\n",
                "    crate::Failure::new()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "failure"]).unwrap();
    let keep = written(&root, "src/keep.rs");
    assert!(!keep.contains("use crate::failure::Failure;"), "{keep}");
    assert!(keep.contains("pub use crate::other::*;"), "{keep}");
    assert!(keep.contains("pub fn make() -> crate::failure::Failure"), "{keep}");
    assert!(keep.contains("crate::failure::Failure::new()"), "{keep}");
    assert!(keep.contains("pub fn local() -> Failure"), "{keep}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_prelude_named_move_keeps_the_full_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub struct String;\n\n",
                "impl String {\n",
                "    pub fn new() -> String {\n",
                "        String\n",
                "    }\n",
                "}\n\n",
                "pub fn make() -> crate::String {\n",
                "    crate::String::new()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.String", "--to", "owned"]).unwrap();
    let source = written(&root, "src/lib.rs");
    assert!(!source.contains("use crate::owned::String;"), "{source}");
    assert!(source.contains("pub fn make() -> crate::owned::String"), "{source}");
    assert!(source.contains("crate::owned::String::new()"), "{source}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn an_inline_test_module_mention_keeps_the_full_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub struct Failure;\n\n",
                "impl Failure {\n",
                "    pub fn new() -> Failure {\n",
                "        Failure\n",
                "    }\n",
                "}\n\n",
                "pub fn entry() -> Failure {\n",
                "    Failure::new()\n",
                "}\n\n",
                "#[cfg(test)]\n",
                "mod tests {\n",
                "    #[test]\n",
                "    fn works() {\n",
                "        let _ = crate::Failure::new();\n",
                "    }\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Failure", "--to", "helpers"]).unwrap();
    let helpers = written(&root, "src/helpers.rs");
    assert!(helpers.contains("mod tests"), "{helpers}");
    assert!(helpers.contains("crate::helpers::Failure::new()"), "{helpers}");
    assert!(!helpers.contains("use crate::helpers::Failure;"), "{helpers}");
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A root glob no moved test uses stays out of the moved tests.
#[test]
fn an_unused_ancestor_glob_stays_out_of_moved_tests() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "use std::fmt::*;\n\n",
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(!tests.contains("use std::fmt::*;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A root glob a moved test uses moves with it.
#[test]
fn a_used_ancestor_glob_moves_with_tests() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "use std::collections::*;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "fn other() -> i32 {\n    2\n}\n\n",
                "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
                "#[cfg(test)]\nmod tests;\n",
            ),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    let mut set = BTreeSet::new();\n",
                "    set.insert(make());\n    assert_eq!(set.len(), 1);\n}\n\n",
                "#[test]\nfn other_works() {\n    assert_eq!(other(), 2);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use std::collections::*;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A custom-named source test container keeps a moved mention unqualified.
#[test]
fn a_custom_named_test_container_keeps_a_moved_mention_unqualified() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
        "#[cfg(test)]\nmod unit_tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "crate.unit_tests::other_works").is_ok());
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("assert_eq!(make(), 1);"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A carried ancestor glob cargo reports unused is dropped from the test file.
#[test]
fn an_unused_carried_ancestor_glob_is_dropped() {
    if !ready() {
        return;
    }
    let (root, s) = carried_glob_project();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use std::collections::*;"), "{tests}");
    assert!(!tests.contains("use std::fmt::*;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A glob the user wrote in the target test file is not one the move added.
#[test]
fn a_user_written_glob_in_the_target_test_file_stays() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod helpers;\n\n",
                "use std::fmt::*;\n\n",
                "use std::collections::*;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "fn other() -> i32 {\n    2\n}\n\n",
                "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
                "#[cfg(test)]\nmod tests;\n",
            ),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    let mut set = BTreeSet::new();\n",
                "    set.insert(make());\n    assert_eq!(set.len(), 1);\n}\n\n",
                "#[test]\nfn other_works() {\n    assert_eq!(other(), 2);\n}\n",
            ),
        ),
        ("src/helpers.rs", "#[cfg(test)]\n#[path = \"helpers_tests.rs\"]\nmod tests;\n\npub fn existing() -> i32 {\n    0\n}\n"),
        ("src/helpers_tests.rs", "use std::fmt::*;\n\n#[test]\nfn existing() {}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use std::fmt::*;"), "{tests}");
}

/// The file-backed fixture whose moved test uses only `BTreeSet`, leaving the
/// carried `std::fmt::*` glob unused.
fn carried_glob_project() -> (Scratch, Store) {
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "use std::fmt::*;\n\n",
                "use std::collections::*;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "fn other() -> i32 {\n    2\n}\n\n",
                "pub fn both() -> i32 {\n    make() + other()\n}\n\n",
                "#[cfg(test)]\nmod tests;\n",
            ),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n\n",
                "#[test]\nfn make_works() {\n    let mut set = BTreeSet::new();\n",
                "    set.insert(make());\n    assert_eq!(set.len(), 1);\n}\n\n",
                "#[test]\nfn other_works() {\n    assert_eq!(other(), 2);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    (root, s)
}

/// A dropped glob leaves no stale baseline entry, so its return is reported as
/// a new warning.
#[test]
fn a_dropped_carried_glob_does_not_hide_a_later_warning() {
    if !ready() {
        return;
    }
    let (_root, s) = carried_glob_project();
    let before = s.warning_baseline().unwrap().unwrap();
    assert!(!before.iter().any(|key| key.contains("unused import: `std::fmt::*`")), "{before:?}");
    let header = format!("{}use std::fmt::*;\n", s.header("helpers::tests").unwrap());
    s.set_header("helpers::tests", &header).unwrap();
    s.materialize(&s.index().unwrap()).unwrap();
    let diags = s.check_all(&s.index().unwrap()).unwrap();
    let report = crate::diagnostics::warnings_report(Some(&before), std::iter::empty(), diags.iter());
    assert!(report.contains("unused import: `std::fmt::*`"), "{report}");
}

/// A test moving from a crate-root test module carries its crate-root module
/// glob as a crate path, so the new child test file can see the module.
#[test]
fn a_moved_test_roots_a_crate_root_module_glob() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod testing;\n\n",
                "pub fn make() -> i32 {\n    1\n}\n\n",
                "pub fn keep() -> i32 {\n    make()\n}\n",
                "\n#[cfg(test)]\n#[path = \"crate_tests.rs\"]\nmod tests;\n",
            ),
        ),
        ("src/testing.rs", "pub struct Scratch;\n\nimpl Scratch {\n    pub fn new() -> Self {\n        Scratch\n    }\n}\n"),
        (
            "src/crate_tests.rs",
            concat!(
                "use super::*;\n",
                "use testing::*;\n\n",
                "#[test]\nfn make_works() {\n    let _ = Scratch::new();\n    assert_eq!(make(), 1);\n}\n\n",
                "#[test]\nfn keep_works() {\n    let _ = Scratch::new();\n    assert_eq!(keep(), 1);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "tests::make_works", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("use crate::testing::*;"), "{tests}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A new module gets the crate-root imports its moved helper names, without
/// the module being pre-created with `use super::*`.
#[test]
fn a_new_module_carries_the_parent_imports_its_code_needs() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod language;\n\n",
                "use language::Ref;\n\n",
                "pub enum ImportMatch {\n    One,\n    None,\n}\n\n",
                "pub struct Store;\n\n",
                "impl Store {\n",
                "    pub fn write(&self, text: &str) -> bool {\n        helper(text)\n    }\n",
                "}\n\n",
                "pub fn use_write(s: &Store, text: &str) -> bool {\n    s.write(text)\n}\n\n",
                "fn helper(text: &str) -> bool {\n",
                "    for r in language::scans(text) {\n",
                "        let Ref::Bare(_) = r else { continue };\n",
                "        let ImportMatch::One = language::match_one(text) else { continue };\n",
                "        return true;\n",
                "    }\n    false\n}\n\n",
                "pub fn keep(r: Ref, text: &str) -> bool {\n",
                "    let _ = (r, Ref::Many(Vec::new()));\n",
                "    let _ = language::scans(text);\n",
                "    let _ = language::match_one(text);\n",
                "    false\n}\n",
            ),
        ),
        (
            "src/language.rs",
            concat!(
                "pub enum Ref {\n    Bare(String),\n    Many(Vec<String>),\n}\n\n",
                "pub fn scans(text: &str) -> Vec<Ref> {\n    vec![Ref::Bare(text.to_string())]\n}\n\n",
                "pub fn match_one(text: &str) -> crate::ImportMatch {\n    let _ = text;\n    crate::ImportMatch::One\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Store::write", "--to", "store_write"]).unwrap();
    let target = written(&root, "src/store_write.rs");
    assert_eq!(target.matches("use crate::ImportMatch;").count(), 1, "{target}");
    assert_eq!(target.matches("use crate::language::Ref;").count(), 1, "{target}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A child module's import moves to a new child as a crate path, so the new
/// module sees the same item.
#[test]
fn a_child_module_import_moves_to_a_new_child_as_a_crate_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod language;\n"),
        (
            "src/a.rs",
            concat!(
                "use super::language::Ref;\n\n",
                "pub fn write(text: &str) -> bool {\n",
                "    let Ref::Bare(_) = crate::language::make(text) else { return false };\n",
                "    true\n}\n\n",
                "pub fn keep(text: &str) -> bool {\n",
                "    let _ = Ref::Many(Vec::new());\n",
                "    write(text)\n}\n",
            ),
        ),
        (
            "src/language.rs",
            concat!(
                "pub enum Ref {\n    Bare(String),\n    Many(Vec<String>),\n}\n\n",
                "pub fn make(text: &str) -> Ref {\n    Ref::Bare(text.to_string())\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.write", "--to", "a::w"]).unwrap();
    let target = written(&root, "src/a/w.rs");
    assert_eq!(target.matches("use crate::language::Ref;").count(), 1, "{target}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A name used in both a pattern and a type is imported at most once, with no
/// unused import left behind.
#[test]
fn a_name_in_a_pattern_and_a_type_is_carried_once() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod language;\n\n",
                "use language::Ref;\n\n",
                "pub enum ImportMatch {\n    One,\n    None,\n}\n\n",
                "pub struct Store;\n\n",
                "impl Store {\n",
                "    pub fn write(&self, text: &str) -> bool {\n        helper(language::scans(text).remove(0), text)\n    }\n",
                "}\n\n",
                "fn helper(r: Ref, text: &str) -> bool {\n",
                "    let Ref::Bare(_) = r else { return false };\n",
                "    let ImportMatch::One = language::match_one(text) else { return false };\n",
                "    true\n}\n\n",
                "pub fn keep(r: Ref, text: &str) -> bool {\n",
                "    let _ = (r, Ref::Many(Vec::new()));\n",
                "    let _ = language::scans(text);\n",
                "    let _ = language::match_one(text);\n",
                "    false\n}\n",
            ),
        ),
        (
            "src/language.rs",
            concat!(
                "pub enum Ref {\n    Bare(String),\n    Many(Vec<String>),\n}\n\n",
                "pub fn scans(text: &str) -> Vec<Ref> {\n    vec![Ref::Bare(text.to_string())]\n}\n\n",
                "pub fn match_one(text: &str) -> crate::ImportMatch {\n    let _ = text;\n    crate::ImportMatch::One\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.Store::write", "--to", "store_write"]).unwrap();
    let target = written(&root, "src/store_write.rs");
    let ref_imports = target.lines().filter(|line| line.trim_start().starts_with("use crate::language::Ref")).count();
    assert!(ref_imports <= 1, "{target}");
    assert_eq!(target.matches("use crate::ImportMatch;").count(), 1, "{target}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A bare import of a child module moves to a new child as a crate path.
#[test]
fn a_child_relative_import_moves_to_a_new_child_as_a_crate_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        (
            "src/a.rs",
            concat!(
                "pub mod kinds;\n\n",
                "use kinds::Ref;\n\n",
                "pub fn write(text: &str) -> bool {\n",
                "    let Ref::Bare(_) = crate::a::kinds::make(text) else { return false };\n",
                "    true\n}\n\n",
                "pub fn keep(text: &str) -> bool {\n",
                "    let _ = Ref::Many(Vec::new());\n",
                "    write(text)\n}\n",
            ),
        ),
        (
            "src/a/kinds.rs",
            concat!(
                "pub enum Ref {\n    Bare(String),\n    Many(Vec<String>),\n}\n\n",
                "pub fn make(text: &str) -> Ref {\n    Ref::Bare(text.to_string())\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.write", "--to", "a::w"]).unwrap();
    let target = written(&root, "src/a/w.rs");
    assert_eq!(target.matches("use crate::a::kinds::Ref;").count(), 1, "{target}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A private child module is raised just far enough for a caller beside its parent.
#[test]
fn a_private_target_module_is_raised_for_a_caller_outside_its_parent() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "mod inner;\n"),
        ("src/a/inner.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/b.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn call() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["b.make", "--to", "a::inner"]).unwrap();
    assert!(out.contains("Now pub(super): mod a::inner"), "{out}");
    let a = written(&root, "src/a.rs");
    assert!(a.contains("pub(super) mod inner;"), "{a}");
}

/// A caller inside the parent already reaches a private child module.
#[test]
fn a_caller_inside_the_parent_leaves_a_private_target_module_alone() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "mod inner;\n\npub fn call() -> i32 {\n    crate::b::make()\n}\n"),
        ("src/a/inner.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/b.rs", "pub fn make() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["b.make", "--to", "a::inner"]).unwrap();
    assert!(!out.contains(": mod "), "{out}");
    let a = written(&root, "src/a.rs");
    assert!(a.contains("mod inner;") && !a.contains("pub(super)") && !a.contains("pub(crate)"), "{a}");
}

/// A deeper path gives each module the narrowest visibility its callers need.
#[test]
fn a_deeper_target_raises_each_module_only_as_far_as_its_callers_need() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod x;\nmod y;\n"),
        ("src/x.rs", "mod a;\n"),
        ("src/x/a.rs", "mod inner;\n"),
        ("src/x/a/inner.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/y.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn call() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["y.make", "--to", "x::a::inner"]).unwrap();
    assert!(out.contains("Now pub(super): mod x::a"), "{out}");
    assert!(out.contains("Now pub(crate): mod x::a::inner"), "{out}");
    assert!(written(&root, "src/x.rs").contains("pub(super) mod a;"), "{}", written(&root, "src/x.rs"));
    assert!(written(&root, "src/x/a.rs").contains("pub(crate) mod inner;"), "{}", written(&root, "src/x/a.rs"));
}

/// A module whose declaration already reaches every caller keeps its visibility.
#[test]
fn a_module_that_is_already_visible_is_left_alone() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "pub(crate) mod inner;\n"),
        ("src/a/inner.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/b.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn call() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["b.make", "--to", "a::inner"]).unwrap();
    assert!(!out.contains(": mod "), "{out}");
    let a = written(&root, "src/a.rs");
    assert!(a.contains("pub(crate) mod inner;"), "{a}");
}

/// The raise is part of the module's before-state, so undo puts it back.
#[test]
fn undo_restores_a_raised_module_visibility() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "mod inner;\n"),
        ("src/a/inner.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/b.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn call() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let before = crate::board::snapshot(&s).unwrap();
    extract(&s, &["b.make", "--to", "a::inner"]).unwrap();
    assert!(written(&root, "src/a.rs").contains("pub(super) mod inner;"));
    crate::board::restore(&s, &[("module:a".to_string(), before["module:a"].clone())]).unwrap();
    let a = written(&root, "src/a.rs");
    assert!(a.contains("mod inner;"), "{a}");
    assert!(!a.contains("pub(super)"), "{a}");
}

/// A target the move creates gets the same raise as an existing one.
#[test]
fn a_created_target_module_is_raised_for_a_caller_outside_its_parent() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/b.rs", "pub fn make() -> i32 {\n    1\n}\n\npub fn call() -> i32 {\n    make()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let out = extract(&s, &["b.make", "--to", "a::inner"]).unwrap();
    assert!(out.contains("Created module a::inner (src/a/inner.rs)"), "{out}");
    assert!(out.contains("Now pub(super): mod a::inner"), "{out}");
    let a = written(&root, "src/a.rs");
    assert!(a.contains("pub(super) mod inner;"), "{a}");
}

/// Attributes stay on the `mod` line, and an inline module raises in place.
#[test]
fn a_module_raise_keeps_attributes_and_inline_braces() {
    let callers = BTreeSet::from(["b".to_string()]);
    let (header, visibility) = super::visibility::raise_module("#[cfg(test)]\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert_eq!(visibility, "pub(super)");
    assert!(header.contains("#[cfg(test)]\npub(super) mod inner;"), "{header}");
    let (inline, visibility) = super::visibility::raise_module("pub(super) mod inner {\n}\n", "inner", "x::a", &callers).unwrap();
    assert_eq!(visibility, "pub(crate)");
    assert!(inline.contains("pub(crate) mod inner {"), "{inline}");
}

/// A multi-line attribute stays attached, and `mod` text in a block comment is not the declaration.
#[test]
fn a_module_raise_skips_multiline_attributes_and_block_comments() {
    let callers = BTreeSet::from(["b".to_string()]);
    let (header, visibility) = super::visibility::raise_module("#[cfg(all(\n    test\n))]\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert_eq!(visibility, "pub(super)");
    assert!(header.contains("#[cfg(all(\n    test\n))]\npub(super) mod inner;"), "{header}");
    let (header, visibility) = super::visibility::raise_module("/*\nmod inner;\n*/\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert_eq!(visibility, "pub(super)");
    assert!(header.contains("/*\nmod inner;\n*/\npub(super) mod inner;"), "{header}");
    assert!(super::visibility::raise_module("/*\nmod inner;\n*/\n", "inner", "a", &callers).is_none());
    let (header, _) = super::visibility::raise_module("use x; /*\nmod inner;\n*/\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert!(header.contains("/*\nmod inner;\n*/\npub(super) mod inner;"), "{header}");
    let (header, _) = super::visibility::raise_module("// mod inner;\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert_eq!(header, "// mod inner;\npub(super) mod inner;\n");
    let (header, _) = super::visibility::raise_module("const S: &str = \"/*\";\nmod inner;\n", "inner", "a", &callers).unwrap();
    assert_eq!(header, "const S: &str = \"/*\";\npub(super) mod inner;\n");
}

/// A raw string's backslash or inner quote, and an escaped quote char, do not end a literal early.
#[test]
fn a_module_raise_skips_raw_strings_and_char_literals() {
    let callers = BTreeSet::from(["b".to_string()]);
    for prefix in [
        "const S: &str = r\"\\\";\n",
        "const S: &str = r#\"a\"/*\"#;\n",
        "const C: char = '\\\"';\nconst S: &str = \"x\";\n",
        "fn f<'a>(x: &'a str) {} /* c */\n",
    ] {
        let (header, visibility) = super::visibility::raise_module(&format!("{prefix}mod inner;\n"), "inner", "a", &callers).unwrap();
        assert_eq!(visibility, "pub(super)");
        assert_eq!(header, format!("{prefix}pub(super) mod inner;\n"));
    }
}

/// A child module that shares a crate-root module's name still roots the
/// carried import at the child.
#[test]
fn a_child_module_import_wins_over_a_crate_root_module_of_the_same_name() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod kinds;\npub mod a;\n"),
        (
            "src/a.rs",
            concat!(
                "pub mod kinds;\n\n",
                "use kinds::Ref;\n\n",
                "pub fn write(text: &str) -> bool {\n",
                "    let Ref::Bare(_) = crate::a::kinds::make(text) else { return false };\n",
                "    true\n}\n\n",
                "pub fn keep(text: &str) -> bool {\n",
                "    let _ = Ref::Many(Vec::new());\n",
                "    write(text)\n}\n",
            ),
        ),
        ("src/kinds.rs", ""),
        (
            "src/a/kinds.rs",
            concat!(
                "pub enum Ref {\n    Bare(String),\n    Many(Vec<String>),\n}\n\n",
                "pub fn make(text: &str) -> Ref {\n    Ref::Bare(text.to_string())\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.write", "--to", "a::w"]).unwrap();
    let target = written(&root, "src/a/w.rs");
    assert_eq!(target.matches("use crate::a::kinds::Ref;").count(), 1, "{target}");
    assert!(!target.contains("use crate::kinds::Ref;"), "{target}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A crate-root import staying child code reaches through `use super::*`
/// stays in reach after the move.
#[test]
fn a_crate_root_import_staying_children_reach_is_kept_or_copied() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod board;\n",
                "pub mod lockdown;\n\n",
                "use std::env;\n\n",
                "pub fn root() -> Option<String> {\n",
                "    env::var(\"ROOT\").ok()\n",
                "}\n",
            ),
        ),
        ("src/board.rs", "use super::*;\n\npub fn board() -> Option<String> {\n    env::var(\"BOARD\").ok()\n}\n"),
        ("src/lockdown.rs", "use super::*;\n\npub fn lock() -> Option<String> {\n    env::var(\"LOCK\").ok()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.root", "--to", "roots"]).unwrap();
    let lib = written(&root, "src/lib.rs");
    let board = written(&root, "src/board.rs");
    let lockdown = written(&root, "src/lockdown.rs");
    assert!(
        lib.contains("use std::env;") || (board.contains("use std::env;") && lockdown.contains("use std::env;")),
        "{lib}\n{board}\n{lockdown}"
    );
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A trait reached through a type path (`i32::from_str`) is carried, though
/// the moved text never names the trait.
#[test]
fn a_moved_item_carries_a_trait_used_through_a_type_path() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/main.rs",
            concat!(
                "use std::str::FromStr;\n\n",
                "fn parse(text: &str) -> i32 {\n",
                "    i32::from_str(text).unwrap()\n",
                "}\n\n",
                "fn main() {\n",
                "    let _ = parse(\"1\");\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.parse", "--to", "cli"]).unwrap();
    let target = written(&root, "src/cli.rs");
    assert!(target.contains("use std::str::FromStr;"), "{target}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A `thread_local!` block moved to another module raises the static its
/// staying callers reach, so their imports resolve.
#[test]
fn a_moved_thread_local_reached_from_staying_code_becomes_pub_crate() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod board;\n\n",
                "thread_local! {\n",
                "    static DB: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };\n",
                "}\n\n",
                "pub fn root() -> u8 {\n",
                "    DB.with(|v| v.get())\n",
                "}\n",
            ),
        ),
        ("src/board.rs", "use super::*;\n\npub fn board() -> u8 {\n    DB.with(|v| v.get())\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.DB", "--to", "dbs"]).unwrap();
    let moved = text(&s, "dbs.DB");
    assert!(moved.contains("pub(crate) static DB"), "{moved}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// Moving a helper that uses `Duration` into a module grouping that import
/// adds no second one.
#[test]
fn a_moved_helper_does_not_repeat_a_grouped_duration_import() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "pub mod a;\n\n",
                "use std::time::Duration;\n\n",
                "pub fn wait() -> Duration {\n",
                "    Duration::from_millis(1)\n",
                "}\n\n",
                "pub fn keep() -> i32 {\n",
                "    1\n",
                "}\n",
            ),
        ),
        (
            "src/a.rs",
            concat!(
                "use super::*;\n",
                "use std::time::{Duration, Instant};\n\n",
                "pub fn now() -> Instant {\n",
                "    Instant::now()\n",
                "}\n\n",
                "pub fn kept() -> i32 {\n",
                "    keep()\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.wait", "--to", "a"]).unwrap();
    let a = written(&root, "src/a.rs");
    assert!(a.contains("use std::time::{Duration, Instant};"), "{a}");
    assert!(!a.contains("use std::time::Duration;"), "{a}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A name or path a module already imports, under any spelling, is not added
/// a second time.
#[test]
fn an_import_is_not_added_when_its_name_or_path_is_already_imported() {
    let duration = ["std".to_string(), "time".to_string(), "Duration".to_string()];
    for header in
        ["use std::time::Duration;\n", "use std::time::{Duration, Instant};\n", "use std::time::Duration as D;\n", "use std::time::*;\n"]
    {
        let mut headers = HashMap::from([("a".to_string(), header.to_string())]);
        super::glob_reach::add_module_import("a", &duration, "Duration", &mut headers).unwrap();
        assert_eq!(headers["a"], header);
    }
}

/// A carried import clashing with an item moving to the same target is
/// aliased, and the moved code names the alias.
#[test]
fn an_import_clashing_with_a_moved_item_is_aliased() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "use std::collections::HashMap;\n\npub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n"),
        ("src/b.rs", "pub struct HashMap;\n\npub fn other() -> i32 {\n    3\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "b.HashMap", "--to", "w"]).unwrap();
    let ix = s.index().unwrap();
    let w = ix.modules.iter().find(|m| m.path == "w").unwrap();
    assert!(w.header.contains("use std::collections::HashMap as HashMap_2;"), "{}", w.header);
    let make = &ix.syms[ix.syms.iter().position(|x| x.module == "w" && x.name == "make").unwrap()];
    assert!(make.text.contains("HashMap_2::new()"), "{}", make.text);
    assert!(make.text.contains("HashMap_2<i32, i32>"), "{}", make.text);
}

/// An import of an item moving to the same target is not carried, so the
/// moving item keeps its name.
#[test]
fn an_import_of_a_moving_item_is_not_aliased() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "use crate::b::Thing;\n\npub fn make() -> Thing {\n    Thing\n}\n"),
        ("src/b.rs", "pub struct Thing;\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "b.Thing", "--to", "w"]).unwrap();
    let ix = s.index().unwrap();
    let w = ix.modules.iter().find(|m| m.path == "w").unwrap();
    assert!(!w.header.contains("Thing_2"), "{}", w.header);
    let make = &ix.syms[ix.syms.iter().position(|x| x.module == "w" && x.name == "make").unwrap()];
    assert!(!make.text.contains("Thing_2"), "{}", make.text);
    assert!(make.text.contains("-> Thing"), "{}", make.text);
}

/// A carried import keeps its gate, so the target compiles without that gate.
#[test]
fn a_carried_import_keeps_its_attribute() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let lib = concat!(
        "#[cfg(feature = \"x\")]\n",
        "mod gated {\n",
        "    pub trait Speak {\n        fn speak(&self) -> i32;\n    }\n\n",
        "    impl Speak for i32 {\n        fn speak(&self) -> i32 {\n            *self\n        }\n    }\n\n",
        "    pub trait Loud {\n        fn loud(&self) -> i32;\n    }\n\n",
        "    impl Loud for i32 {\n        fn loud(&self) -> i32 {\n            *self + 1\n        }\n    }\n",
        "}\n\n",
        "#[cfg(feature = \"x\")]\n",
        "use gated::Speak;\n",
        "#[cfg(feature = \"x\")]\n",
        "use gated::Loud as Shout;\n",
        "#[cfg(feature = \"x\")]\n",
        "use std::collections::*;\n\n",
        "#[cfg(feature = \"x\")]\n",
        "pub fn make() -> i32 {\n",
        "    let mut tree = BTreeMap::new();\n",
        "    tree.insert(1, 2);\n",
        "    (1).speak() + (2).loud() + tree.len() as i32\n",
        "}\n",
    );
    let root = Scratch::new().with(&[("Cargo.toml", manifest.as_str()), ("src/lib.rs", lib)]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let target = written(&root, "src/helpers.rs");
    assert!(target.contains("#[cfg(feature = \"x\")] use crate::gated::Speak;"), "{target}");
    assert!(target.contains("#[cfg(feature = \"x\")] use crate::gated::Loud as Shout;"), "{target}");
    assert!(target.contains("#[cfg(feature = \"x\")] use std::collections::*;"), "{target}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// An ungated carry widens a gated copy in the target, and an import that is
/// already available is not added a second time.
#[test]
fn a_carried_import_replaces_a_gated_target_copy() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "use std::collections::HashMap;\n",
                "use std::fmt;\n\n",
                "pub fn make(value: impl fmt::Debug) -> HashMap<i32, i32> {\n",
                "    let mut map = HashMap::new();\n",
                "    map.insert(format!(\"{value:?}\").len() as i32, 1);\n",
                "    map\n",
                "}\n",
            ),
        ),
        (
            "src/b.rs",
            concat!(
                "#[cfg(feature = \"x\")]\n",
                "use std::collections::HashMap;\n",
                "#[allow(unused_imports)] use std::fmt;\n\n",
                "pub fn keep() -> i32 {\n    0\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let b = written(&root, "src/b.rs");
    assert_eq!(b.lines().filter(|line| line.trim() == "use std::collections::HashMap;").count(), 1, "{b}");
    assert!(!b.contains("#[cfg(feature = \"x\")]"), "{b}");
    assert_eq!(b.lines().filter(|line| line.trim() == "#[allow(unused_imports)] use std::fmt;").count(), 1, "{b}");
    assert!(!b.lines().any(|line| line.trim() == "use std::fmt;"), "{b}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    check_features(&root, "x");
}

/// A gated import covers only a carry under the same gate.
#[test]
fn a_gated_import_covers_only_its_own_gate() {
    let site = |attributes: &str| {
        let mut site = crate::language::rust_refs::ImportSite::named(vec!["std".to_string(), "fmt".to_string()]);
        site.attributes = attributes.to_string();
        site
    };
    assert!(super::imports::covers(&site(""), &site("#[cfg(feature = \"x\")]")));
    assert!(super::imports::covers(&site("#[allow(unused_imports)]"), &site("")));
    assert!(super::imports::covers(&site("#[cfg(feature = \"x\")]"), &site("#[cfg(feature = \"x\")]")));
    assert!(!super::imports::covers(&site("#[cfg(feature = \"x\")]"), &site("")));
    assert!(!super::imports::covers(&site("#[cfg(feature = \"x\")]"), &site("#[cfg(feature = \"y\")]")));
}

/// A gated import a moved test needs keeps its gate in the test file.
#[test]
fn a_carried_test_import_keeps_its_gate() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        (
            "src/lib.rs",
            concat!("pub fn make() -> i32 {\n    1\n}\n\n", "pub fn keep() -> i32 {\n    2\n}\n\n", "#[cfg(test)]\nmod tests;\n",),
        ),
        (
            "src/tests.rs",
            concat!(
                "use super::*;\n",
                "#[cfg(feature = \"x\")]\n",
                "use std::collections::HashMap;\n\n",
                "#[cfg(feature = \"x\")]\n",
                "#[test]\n",
                "fn make_works() {\n",
                "    let mut map = HashMap::new();\n",
                "    map.insert(1, make());\n",
                "    assert_eq!(map[&1], 1);\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let tests = written(&root, "src/helpers_tests.rs");
    assert!(tests.contains("#[cfg(feature = \"x\")] use std::collections::HashMap;"), "{tests}");
    assert!(!tests.lines().any(|line| line.trim() == "use std::collections::HashMap;"), "{tests}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// An ungated carry drops the gate on a gated `use` item after an earlier item.
#[test]
fn a_carried_import_ungates_a_gated_use_symbol() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", concat!("use std::collections::HashMap;\n\n", "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n",)),
        ("src/b.rs", concat!("pub fn keep() -> i32 {\n    0\n}\n\n", "#[cfg(feature = \"x\")]\n", "use std::collections::HashMap;\n",)),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let b = written(&root, "src/b.rs");
    assert!(!b.contains("#[cfg(feature = \"x\")]"), "{b}");
    assert_eq!(b.lines().filter(|line| line.trim() == "use std::collections::HashMap;").count(), 1, "{b}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A carry under another gate is added beside the target's gated copy.
#[test]
fn a_carried_import_keeps_a_different_gate() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\ny = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        (
            "src/a.rs",
            concat!(
                "#[cfg(feature = \"y\")]\n",
                "use std::collections::HashMap;\n\n",
                "#[cfg(feature = \"y\")]\n",
                "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n",
            ),
        ),
        ("src/b.rs", concat!("#[cfg(feature = \"x\")]\n", "use std::collections::HashMap;\n\n", "pub fn keep() -> i32 {\n    0\n}\n",)),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let b = written(&root, "src/b.rs");
    assert!(b.contains("#[cfg(feature = \"x\")]\nuse std::collections::HashMap;"), "{b}");
    assert!(b.contains("#[cfg(feature = \"y\")] use std::collections::HashMap"), "{b}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    check_features(&root, "x");
    check_features(&root, "y");
    check_features(&root, "x,y");
}

/// A carried test import aliases when a moved test item takes its name.
#[test]
fn a_carried_test_import_clashing_with_a_moved_test_item_is_aliased() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\npub mod c;\n"),
        ("src/a.rs", concat!("pub fn make() -> i32 {\n    1\n}\n\n", "#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n",)),
        (
            "src/a_tests.rs",
            concat!(
                "use super::*;\n",
                "use std::collections::HashMap;\n\n",
                "#[test]\nfn make_works() {\n    let mut map = HashMap::<i32, i32>::new();\n",
                "    map.insert(1, make());\n    assert_eq!(map[&1], 1);\n}\n",
            ),
        ),
        ("src/b.rs", "pub fn existing() -> i32 {\n    0\n}\n"),
        ("src/c.rs", concat!("pub fn test_only() -> i32 {\n    2\n}\n\n", "#[cfg(test)]\n#[path = \"c_tests.rs\"]\nmod tests;\n",)),
        (
            "src/c_tests.rs",
            concat!(
                "use super::*;\n\n",
                "struct HashMap;\n\n",
                "#[test]\nfn test_only_works() {\n    let _ = HashMap;\n    assert_eq!(test_only(), 2);\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "c.test_only", "--to", "b"]).unwrap();
    let tests = written(&root, "src/b_tests.rs");
    assert!(tests.contains("use std::collections::HashMap as HashMap_2;"), "{tests}");
    assert!(tests.contains("HashMap_2::<i32, i32>::new()"), "{tests}");
    assert!(tests.contains("struct HashMap;"), "{tests}");
}

/// rustc can name several traits that would fix the same error; none is safe
/// to guess from, while a lone suggestion is still carried.
#[test]
fn a_trait_import_is_suggested_only_when_one_is_offered() {
    let one = concat!(
        "error[E0599]: no method named `speak` found for type `i32` in the current scope\n",
        "help: trait `Speak` which provides `speak` is implemented but not in scope; perhaps you want to import it\n",
        "  |\n",
        "1 + use crate::a::Speak;\n",
    );
    let paths = super::imports::suggested_trait_imports(one);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].join("::"), "crate::a::Speak");

    let two = concat!(
        "error[E0599]: no method named `speak` found for type `i32` in the current scope\n",
        "help: trait `Speak` which provides `speak` is implemented but not in scope; perhaps you want to import it\n",
        "  |\n",
        "1 + use crate::a::Speak;\n",
        "help: there is another trait with a method of the same name\n",
        "  |\n",
        "1 + use crate::b::Speak;\n",
    );
    assert!(super::imports::suggested_trait_imports(two).is_empty());

    let broken = concat!(
        "error[E0599]: no method named `speak` found for type `i32` in the current scope\n",
        "help: trait `Speak` which provides `speak` is implemented but not in scope; perhaps you want to import it\n",
        "  |\n",
        "1 + use crate::a::Speak;\n",
        "help: there is another trait with a method of the same name\n",
        "  |\n",
        "1 + use crate::b::{;\n",
    );
    assert!(super::imports::suggested_trait_imports(broken).is_empty());
}

/// A static and its only user move together, so the static does not need to
/// become visible outside the module that now declares it.
#[test]
fn a_moved_thread_local_needed_only_where_it_lands_stays_private() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "thread_local! {\n",
                "    static DB: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };\n",
                "}\n\n",
                "pub fn value() -> u8 {\n",
                "    DB.with(|v| v.get())\n",
                "}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["crate.DB", "crate.value", "--to", "dbs"]).unwrap();
    let moved = text(&s, "dbs.DB");
    assert!(moved.contains("static DB:"), "{moved}");
    assert!(!moved.contains("pub(crate) static DB:"), "{moved}");
    let diagnostics = s.check_all(&s.index().unwrap()).unwrap();
    let errors: Vec<String> = diagnostics.iter().filter(|d| d.error).map(|d| d.text.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// An import of the moved item in its destination is dropped, so the item is
/// not imported into its own module.
#[test]
fn a_targets_import_of_the_moved_item_is_dropped() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn f() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "use crate::a::f;\n\npub fn go() -> i32 {\n    f()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.f", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(!b.header.contains("use crate::a::f;"), "{}", b.header);
    assert!(!b.header.contains("use crate::b::f;"), "{}", b.header);
    let go = ix.syms.iter().find(|x| x.module == "b" && x.name == "go").unwrap();
    assert_eq!(go.text, "pub fn go() -> i32 {\n    f()\n}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A grouped import in the destination loses only the members that move.
#[test]
fn a_targets_group_import_loses_only_the_moved_members() {
    if !ready() {
        return;
    }
    let cases: [(&[&str], bool); 2] = [(&["a.f", "--to", "b"], true), (&["a.f", "a.h", "--to", "b"], false)];
    for (args, keeps_h) in cases {
        let root = Scratch::new().with(&[
            ("Cargo.toml", CARGO),
            ("src/lib.rs", "pub mod a;\npub mod b;\n"),
            ("src/a.rs", "pub fn f() -> i32 {\n    1\n}\n\npub fn h() -> i32 {\n    2\n}\n"),
            ("src/b.rs", "use crate::a::{f, h};\n\npub fn go() -> i32 {\n    f() + h()\n}\n"),
        ]);
        crate::import(&root).unwrap();
        let s = Store::at(&root).unwrap();
        extract(&s, args).unwrap();
        let ix = s.index().unwrap();
        let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
        assert!(!b.header.contains("f"), "{}", b.header);
        assert_eq!(b.header.contains("h"), keeps_h, "{}", b.header);
        let go = ix.syms.iter().find(|x| x.module == "b" && x.name == "go").unwrap();
        assert_eq!(go.text, "pub fn go() -> i32 {\n    f() + h()\n}");
        let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
        assert!(messages.is_empty(), "{messages:?}");
    }
}

/// `use super::f;` in a destination child module is dropped too.
#[test]
fn a_targets_super_import_of_the_moved_item_is_dropped() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub mod b;\n\npub fn f() -> i32 {\n    1\n}\n"),
        ("src/a/b.rs", "use super::f;\n\npub fn go() -> i32 {\n    f()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.f", "--to", "a::b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "a::b").unwrap();
    assert!(!b.header.contains("use "), "{}", b.header);
    let go = ix.syms.iter().find(|x| x.module == "a::b" && x.name == "go").unwrap();
    assert_eq!(go.text, "pub fn go() -> i32 {\n    f()\n}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// Dropping a destination's `pub use` keeps the item as visible as the re-export.
#[test]
fn a_targets_reexport_of_the_moved_item_is_dropped_and_the_item_stays_public() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub(crate) fn f() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub use crate::a::f;\n\npub fn go() -> i32 {\n    f()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.f", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(!b.header.contains("use "), "{}", b.header);
    let f = ix.syms.iter().find(|x| x.module == "b" && x.name == "f").unwrap();
    assert_eq!(f.text, "pub fn f() -> i32 {\n    1\n}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// An aliased import does not bind the item under its own name, so it follows it.
#[test]
fn a_targets_aliased_import_of_the_moved_item_is_retargeted() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn f() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "use crate::a::f as ff;\n\npub fn go() -> i32 {\n    ff()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.f", "--to", "b"]).unwrap();
    let ix = s.index().unwrap();
    let b = ix.modules.iter().find(|m| m.path == "b").unwrap();
    assert!(b.header.contains("use crate::b::f as ff;"), "{}", b.header);
    let go = ix.syms.iter().find(|x| x.module == "b" && x.name == "go").unwrap();
    assert_eq!(go.text, "pub fn go() -> i32 {\n    ff()\n}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// A child module does not inherit its parent's items, so its import stays.
#[test]
fn a_child_modules_import_of_the_moved_item_is_retargeted() {
    if !ready() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn f() -> i32 {\n    1\n}\n"),
        ("src/b.rs", "pub mod child;\n"),
        ("src/b/child.rs", "use crate::a::f;\n\npub fn go() -> i32 {\n    f()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.f", "--to", "b"]).unwrap();
    let child = written(&root, "src/b/child.rs");
    assert!(child.contains("use crate::b::f;"), "{child}");
    assert!(child.contains("f()"), "{child}");
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
}

/// Checks the scratch crate with `features` on, failing with the compiler's output.
fn check_features(root: &std::path::Path, features: &str) {
    let out = std::process::Command::new("cargo")
        .args(["check", "--all-targets", "--offline", "--target-dir"])
        .arg(root.join("target"))
        .arg("--features")
        .arg(features)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cargo check --features {features} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An ungated carry into a gated group ungates only the member it needs.
#[test]
fn a_carried_import_ungates_only_its_group_member() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        ("src/lib.rs", concat!("pub mod a;\n", "pub mod b;\n", "pub mod plain;\n", "#[cfg(feature = \"x\")]\n", "pub mod gated;\n",)),
        (
            "src/plain.rs",
            concat!(
                "pub trait B {\n",
                "    fn b(&self) -> i32;\n",
                "}\n\n",
                "impl B for i32 {\n",
                "    fn b(&self) -> i32 {\n",
                "        *self\n",
                "    }\n",
                "}\n",
            ),
        ),
        ("src/gated.rs", "pub struct C;\n"),
        ("src/a.rs", concat!("use crate::plain::B;\n\n", "pub fn make() -> i32 {\n    1.b()\n}\n",)),
        ("src/b.rs", concat!("#[cfg(feature = \"x\")]\n", "use crate::{plain::B, gated::C};\n\n", "pub fn keep() -> i32 {\n    0\n}\n",)),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let b = written(&root, "src/b.rs");
    assert!(b.contains("use crate::plain::B;"), "{b}");
    assert!(b.contains("#[cfg(feature = \"x\")]\nuse crate::gated::C;"), "{b}");
    assert!(!b.contains("plain::B,"), "{b}");
    assert!(!b.contains("#[cfg(feature = \"x\")]\nuse crate::plain::B;"), "{b}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    check_features(&root, "x");
}

/// An ungated carry drops every gated copy of its key, leaving one ungated.
#[test]
fn a_carried_import_drops_complementary_gated_copies() {
    if !ready() {
        return;
    }
    let manifest = format!("{CARGO}\n[features]\nx = []\n");
    let root = Scratch::new().with(&[
        ("Cargo.toml", manifest.as_str()),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", concat!("use std::collections::HashMap;\n\n", "pub fn make() -> HashMap<i32, i32> {\n    HashMap::new()\n}\n",)),
        (
            "src/b.rs",
            concat!(
                "#[cfg(feature = \"x\")]\n",
                "use std::collections::HashMap;\n",
                "#[cfg(not(feature = \"x\"))]\n",
                "use std::collections::HashMap;\n\n",
                "pub fn keep() -> i32 {\n    0\n}\n",
            ),
        ),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    extract(&s, &["a.make", "--to", "b"]).unwrap();
    let b = written(&root, "src/b.rs");
    assert_eq!(b.lines().filter(|line| line.trim() == "use std::collections::HashMap;").count(), 1, "{b}");
    assert!(!b.contains("#[cfg"), "{b}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
    check_features(&root, "x");
}

/// A gated group that `without_cfg` can't edit one member of falls back to a
/// line of its own, and the other gated copy still goes.
#[test]
fn a_gated_group_copy_without_an_editable_cfg_still_ends_with_one_binding() {
    let text = concat!(
        "#[cfg(feature = \"x\")]\n",
        "use { std::collections::HashMap, std::fmt };\n",
        "#[cfg(not(feature = \"x\"))]\n",
        "use std::collections::HashMap;\n",
    );
    let key: (String, Vec<String>, bool) =
        ("HashMap".to_string(), ["std", "collections", "HashMap"].iter().map(|name| (*name).to_string()).collect(), false);
    let (out, changed, ungated) = super::imports::ungate_import(text, "b", &key, true).unwrap();
    assert!(changed, "{out}");
    assert!(ungated, "{out}");
    assert_eq!(out.lines().filter(|line| line.contains("HashMap")).count(), 1, "{out}");
    assert!(!out.contains("not(feature = \"x\")"), "{out}");
    assert!(out.contains("#[cfg(feature = \"x\")]\nuse std::fmt;"), "{out}");
}

/// Collapsing a group whose only member renames `self` keeps the alias.
#[test]
fn a_collapsed_self_alias_keeps_its_alias() {
    assert_eq!(super::collapse_single_member_groups("crate::a::{self as m}"), "crate::a as m");
    assert_eq!(super::collapse_single_member_groups("crate::a::{self}"), "crate::a");
    assert_eq!(super::collapse_single_member_groups("crate::a::{self as m,}"), "crate::a as m");
    assert_eq!(super::collapse_single_member_groups("crate::a::{self,}"), "crate::a");
    assert_eq!(super::collapse_single_member_groups("crate::a::{self as m, x}"), "crate::a::{self as m, x}");
}

/// A `use` inside a fn body that names the moved item is dropped: the item is
/// already in scope in that module.
#[test]
fn a_body_import_of_the_moved_item_is_dropped() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::f;\n    f()\n}\n", &["a.f"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    f()\n}");
}

/// A grouped body import drops the moved member and keeps the staying one.
#[test]
fn a_grouped_body_import_keeps_its_staying_member() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::{f, g};\n    f() + g()\n}\n", &["a.f"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::a::g;\n    f() + g()\n}");
}

/// A renamed body import follows the moved item to its new path.
#[test]
fn a_renamed_body_import_is_retargeted() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::f as h;\n    h()\n}\n", &["a.f"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::b::f as h;\n    h()\n}");
}

/// A glob body import is not mistaken for the import of the moved item.
#[test]
fn a_glob_body_import_keeps_its_other_members() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::*;\n    f() + g()\n}\n", &["a.f"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::a::*;\n    f() + g()\n}");
}

/// A body group drops its moved member whether it is last or the only one.
#[test]
fn a_body_group_import_drops_the_moved_member_at_any_position() {
    let cases = [("use crate::a::{g, f};", "f() + g()", "    use crate::a::g;\n"), ("use crate::a::{f};", "f()", "")];
    for (line, body, kept) in cases {
        let b = format!("pub fn go() -> i32 {{\n    {line}\n    {body}\n}}\n");
        let Some(go) = body_import_after_extract(&b, &["a.f"]) else {
            return;
        };
        assert_eq!(go, format!("pub fn go() -> i32 {{\n{kept}    {body}\n}}"), "{line}");
    }
}

/// Two moved members of one body group produce one edit that keeps the third.
#[test]
fn two_of_three_body_group_members_move_together() {
    let Some(go) =
        body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::{f, g, h};\n    f() + g() + h()\n}\n", &["a.f", "a.g"])
    else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::a::h;\n    f() + g() + h()\n}");
}

/// A body group whose members all move loses its line in one edit.
#[test]
fn all_body_group_members_move_together() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::{f, g};\n    f() + g()\n}\n", &["a.f", "a.g"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    f() + g()\n}");
}

/// A multi-line body group loses its moved member.
#[test]
fn a_multiline_body_group_import_drops_the_moved_member() {
    let Some(go) = body_import_after_extract(
        "pub fn go() -> i32 {\n    use crate::a::{\n        f,\n        g,\n    };\n    f() + g()\n}\n",
        &["a.f"],
    ) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::a::g;\n    f() + g()\n}");
}

/// `go`'s text after extracting `names` from `a` to `b`, with check clean.
fn body_import_after_extract(b_src: &str, names: &[&str]) -> Option<String> {
    if !ready() {
        return None;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn f() -> i32 {\n    1\n}\n\npub fn g() -> i32 {\n    2\n}\n\npub fn h() -> i32 {\n    3\n}\n"),
        ("src/b.rs", b_src),
    ]);
    crate::import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let mut args: Vec<&str> = names.to_vec();
    args.extend(["--to", "b"]);
    extract(&s, &args).unwrap();
    let ix = s.index().unwrap();
    let go = ix.syms.iter().find(|x| x.module == "b" && x.name == "go")?;
    let text = go.text.clone();
    let messages: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().map(|d| d.text).collect();
    assert!(messages.is_empty(), "{messages:?}");
    Some(text)
}

/// A renamed member of a body group keeps its alias at the destination.
#[test]
fn a_renamed_member_in_a_body_group_follows_the_move() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::{f as h, g};\n    h() + g()\n}\n", &["a.f", "a.g"])
    else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::b::f as h;\n    h() + g()\n}");
}

/// A body group splits when only its renamed member moves.
#[test]
fn a_body_group_split_keeps_a_staying_member_and_moves_the_alias() {
    let Some(go) = body_import_after_extract("pub fn go() -> i32 {\n    use crate::a::{f as h, g};\n    h() + g()\n}\n", &["a.f"]) else {
        return;
    };
    assert_eq!(go, "pub fn go() -> i32 {\n    use crate::a::g;\n    use crate::b::f as h;\n    h() + g()\n}");
}

/// A removal that misses the import tree leaves the item for the per-site rewrite.
#[test]
fn a_body_use_removal_outside_its_tree_is_refused() {
    let text = "pub fn go() -> i32 {\n    use crate::a::{f, g};\n    f() + g()\n}\n";
    let at = text.find("f, g").unwrap();
    let item = super::body_use_item(text, &(at..at + 1)).unwrap();
    let removal = item.start..item.start + 1;
    assert!(super::body_use_edit(text, item, &[removal], &[]).is_none());
}
