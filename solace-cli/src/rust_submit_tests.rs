//! `submit`'s `#[path]` handling (D-173): a declaration names its file however
//! its path is written.

use super::*;

/// A path resolves `.` and `..` parts lexically, as the file system would; a
/// path above the project keeps the `..` that leaves it.
#[test]
fn a_path_resolves_its_dot_parts() {
    assert_eq!(normalized_path("tests/../common/mod.rs"), "common/mod.rs");
    assert_eq!(normalized_path("./tests/./a.rs"), "tests/a.rs");
    assert_eq!(normalized_path("tests//a.rs"), "tests/a.rs");
    assert_eq!(normalized_path("tests/../../a.rs"), "../a.rs");
}

/// A declaration's `#[path]` is resolved before it is compared with the row's
/// file, so a store written with a climbing path still finds it (D-173).
#[test]
fn a_path_that_climbs_matches_the_file_it_names() {
    let parent = ModuleRow {
        path: "integration::a".into(),
        dir: "tests".into(),
        header: "#[path = \"../common/mod.rs\"]\nmod common;\n".into(),
        file: Some("tests/a.rs".into()),
        owns_dir: Some(true),
    };
    let row = ModuleRow {
        path: "integration::common".into(),
        dir: "common".into(),
        header: String::new(),
        file: Some("common/mod.rs".into()),
        owns_dir: Some(true),
    };
    assert!(declares_module(&parent, "common", &row));
    let outside = ModuleRow { file: Some("../common/mod.rs".into()), ..row };
    assert!(!declares_module(&parent, "common", &outside));
}

#[test]
fn a_format_says_which_tool_is_missing() {
    let missing = crate::language::cargo::MISSING.to_string();
    assert_eq!(super::format_missing(Some(missing.clone()), false), Some(missing));
    assert_eq!(super::format_missing(None, false), Some(crate::language::cargo::RUSTFMT_MISSING.to_string()));
    assert_eq!(super::format_missing(None, true), None);
}

#[test]
fn a_long_line_counts_columns_not_characters() {
    let flag = "\u{1f1fa}\u{1f1f8}";
    assert!(super::long_line_note("crate.x", &"\t".repeat(26), 100).is_some());
    assert!(super::long_line_note("crate.x", &"界".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"\u{1f1e6}\u{200b}".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &flag.repeat(26), 100).is_none());
    assert!(super::long_line_note("crate.x", &flag.repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"\u{1f680}".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"\u{1fa70}".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"e\u{301}".repeat(60), 100).is_none());
}

/// A format's text still can't add or drop a `mod` (D-180).
#[test]
fn a_format_refuses_a_changed_mod_list() {
    let lib = "pub fn one() -> i32 {\n    1\n}\n\nmod tests;\n";
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", crate::testing::CARGO),
        ("src/lib.rs", lib),
        ("src/tests.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = crate::Store::at(&root).unwrap();
    for text in ["pub fn one() -> i32 {\n    1\n}\n\nmod tests;\nmod extra;\n", "pub fn one() -> i32 {\n    1\n}\n"] {
        let err = store.submit_module_rust("crate", text, true).unwrap_err().0;
        assert!(err.contains("`mod` list can't change"), "{err}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    }
}

/// A malformed settings file is reported, not read as a missing toolchain.
#[test]
fn formatting_reports_invalid_settings() {
    let (_root, store) = crate::testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    let bad = crate::testing::Scratch::new().with(&[("system.toml", "[languages]\nrust = 1\n")]);
    let e = crate::board::with_config(&bad.join("system.toml"), || store.format_module_rust("crate")).unwrap_err();
    assert!(e.0.contains("E6020"), "{}", e.0);
}

/// A stored `mod` between two imports keeps that place when the uses change.
#[test]
fn stored_mods_keep_their_place_between_imports() {
    let old = "use a::a;\nmod between;\nuse b::b;\n";
    let new = "use a::a;\nuse c::c;\nuse b::b;";
    let mods = [language::rust::FileMod { name: "between".into(), file: None, text: "mod between;".into() }];
    assert_eq!(with_stored_mods(new, old, &mods), "use a::a;\nmod between;\nuse c::c;\nuse b::b;");
}

/// A stored `mod` before the imports goes above the first `use`'s attributes.
#[test]
fn stored_mods_stay_before_a_uses_attributes() {
    let old = "mod a;\n#[cfg(unix)]\nuse b::c;\n";
    let new = "#[cfg(unix)]\nuse b::c;";
    let mods = [language::rust::FileMod { name: "a".into(), file: None, text: "mod a;".into() }];
    assert_eq!(with_stored_mods(new, old, &mods), "mod a;\n#[cfg(unix)]\nuse b::c;");
}

/// A stored `mod` between imports keeps its place when a `use` spans lines.
#[test]
fn stored_mods_keep_their_place_between_multiline_uses() {
    let old = "use a::{\n    x,\n};\nmod between;\nuse b::b;\n";
    let new = "use a::{\n    x,\n};\nuse c::c;\nuse b::b;";
    let mods = [language::rust::FileMod { name: "between".into(), file: None, text: "mod between;".into() }];
    assert_eq!(with_stored_mods(new, old, &mods), "use a::{\n    x,\n};\nmod between;\nuse c::c;\nuse b::b;");
}

/// A stored `mod` after the submitted imports goes there when uses are dropped.
#[test]
fn stored_mods_after_the_last_import_when_uses_are_dropped() {
    let old = "use a::a;\nuse b::b;\nmod m;\nuse c::c;\n";
    let new = "use a::a;";
    let mods = [language::rust::FileMod { name: "m".into(), file: None, text: "mod m;".into() }];
    assert_eq!(with_stored_mods(new, old, &mods), "use a::a;\nmod m;");
}

/// A stored `mod` pushed past dropped uses does not leave a trailing blank line.
#[test]
fn stored_mods_after_the_last_import_drop_their_blank_after() {
    let old = "use a::a;\nuse b::b;\nmod m;\n\nuse c::c;\n";
    let new = "use a::a;";
    let mods = [language::rust::FileMod { name: "m".into(), file: None, text: "mod m;".into() }];
    assert_eq!(with_stored_mods(new, old, &mods), "use a::a;\nmod m;");
}
