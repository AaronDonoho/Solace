#![cfg(test)]

use super::*;
use crate::testing::*;



fn ready() -> bool {
    crate::language::cargo::installed()
}

fn text(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
}

fn id(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.syms[s.resolve(&ix, name).unwrap()].id.clone()
}

fn extract(s: &Store, args: &[&str]) -> Out {
    s.extract(args, usize::MAX)
}

fn written(root: &std::path::Path, rel: &str) -> String {
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
    assert!(text(&s, "crate.go").contains("crate::helpers::make()"), "{}", text(&s, "crate.go"));
    assert!(text(&s, "helpers.make").starts_with("pub(crate) fn make"), "{}", text(&s, "helpers.make"));
    assert!(text(&s, "helpers.helper").starts_with("fn helper"), "{}", text(&s, "helpers.helper"));
    let warnings: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| !d.error).map(|d| d.text).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(written(&root, "src/lib.rs"), "mod helpers;\n\npub fn go() -> i32 {\n    crate::helpers::make()\n}\n");
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

/// A test that reaches moved and staying code is refused.
#[test]
fn a_test_split_is_refused_with_e5002() {
    if !ready() {
        return;
    }
    let lib = concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "pub fn other() -> i32 {\n    2\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n}\n",
    );
    let lib = lib.replace("assert_eq!(make(), 1)", "assert_eq!(make() + other(), 3)");
    let (_root, s) = rust_project(&lib);
    let err = extract(&s, &["crate.make", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E5002"), "{err}");
    assert!(err.contains("make_works") && err.contains("crate.make") && err.contains("crate.other"), "{err}");
}

/// A method and an impl row cannot be selected on their own; a use item moves.
#[test]
fn an_impl_method_or_impl_row_is_refused_and_a_use_moves() {
    if !ready() {
        return;
    }
    let (root, s) = rust_project(concat!(
        "pub struct S;\n\nimpl S {\n    pub fn new() -> Self {\n        S\n    }\n}\n\n",
        "pub trait Named {\n    fn name(&self) -> u8;\n}\n\n",
        "impl Named for S {\n    fn name(&self) -> u8 {\n        0\n    }\n}\n\n",
        "use std::fmt;\n\npub fn keep() -> fmt::Result {\n    Ok(())\n}\n",
    ));
    let err = extract(&s, &["crate.S::new", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("nested method"), "{err}");
    let err = extract(&s, &["crate.<S as Named>", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E6013") && err.contains("impl block"), "{err}");
    let out = extract(&s, &["crate.std::fmt", "--to", "helpers"]).unwrap();
    assert!(out.contains("Moved crate.std::fmt"), "{out}");
    let ix = s.index().unwrap();
    assert!(ix.syms.iter().any(|sym| sym.module == "helpers" && sym.kind_text == "use"));
    let source = written(&root, "src/lib.rs");
    assert!(source.contains("use std::fmt;"), "{source}");
    let errors: Vec<String> = s.check_all(&s.index().unwrap()).unwrap().into_iter().filter(|d| d.error).map(|d| d.text).collect();
    assert!(errors.is_empty(), "{errors:?}");
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

#[test]
fn a_retained_private_helper_becomes_pub_crate() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(
        "pub fn go() -> i32 {\n    make() + helper()\n}\n\nfn make() -> i32 {\n    helper()\n}\n\nfn helper() -> i32 {\n    1\n}\n",
    );
    let out = extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    assert!(out.contains("Now pub(crate):") && out.contains("crate.helper"), "{out}");
    assert!(text(&s, "crate.helper").starts_with("pub(crate) fn helper"), "{}", text(&s, "crate.helper"));
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
    assert!(helpers.contains("#[cfg(test)]") && helpers.contains("use super::*"), "{helpers}");
}

/// One eligible test moves while another stays; the source container stays too.
#[test]
fn a_partial_test_move_leaves_staying_tests() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let ix = s.index().unwrap();
    let container = ix.syms.iter().find(|x| x.module == "helpers" && x.name == "tests").unwrap();
    assert!(container.text.contains("#[cfg(test)]") && container.text.contains("use super::*"), "{}", container.text);
    assert!(s.resolve(&ix, "helpers.tests::make_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::other_works").is_ok());
    assert!(s.resolve(&ix, "crate.tests::make_works").is_err());
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

/// A moved test container does not merge into the target's container of the same name.
#[test]
fn a_test_container_already_in_the_target_is_refused_with_e1004() {
    if !ready() {
        return;
    }
    let (_root, s) = rust_project(concat!(
        "fn make() -> i32 {\n    1\n}\n\n",
        "fn other() -> i32 {\n    2\n}\n\n",
        "#[cfg(test)]\nmod tests {\n    use super::*;\n\n",
        "    #[test]\n    fn make_works() {\n        assert_eq!(make(), 1);\n    }\n\n",
        "    #[test]\n    fn other_works() {\n        assert_eq!(other(), 2);\n    }\n}\n",
    ));
    extract(&s, &["crate.make", "--to", "helpers"]).unwrap();
    let err = extract(&s, &["crate.other", "--to", "helpers"]).unwrap_err().0;
    assert!(err.starts_with("E1004"), "{err}");
    assert!(err.contains("tests"), "{err}");
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
