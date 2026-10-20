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
    assert_eq!(super::format_missing(false, false), Some(crate::language::cargo::MISSING));
    assert_eq!(super::format_missing(true, false), Some(crate::language::cargo::RUSTFMT_MISSING));
    assert_eq!(super::format_missing(true, true), None);
}

#[test]
fn a_long_line_counts_columns_not_characters() {
    assert!(super::long_line_note("crate.x", &"\t".repeat(26), 100).is_some());
    assert!(super::long_line_note("crate.x", &"界".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"\u{1fa70}".repeat(51), 100).is_some());
    assert!(super::long_line_note("crate.x", &"e\u{301}".repeat(60), 100).is_none());
}
