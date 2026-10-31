use super::*;

/// A test file keeps one `use super::*`, adding it only when it is missing.
#[test]
fn a_test_file_header_gets_one_super_glob() {
    assert_eq!(with_super(""), "use super::*;\n\n");
    assert_eq!(with_super("use std::fmt;\n"), "use std::fmt;\nuse super::*;\n\n");
    assert_eq!(with_super("use super::*;\n"), "use super::*;\n\n");
}

