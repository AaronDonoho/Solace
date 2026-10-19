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
