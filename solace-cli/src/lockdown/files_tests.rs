use super::*;
use crate::testing::*;
use std::fs;

#[test]
fn nested_tiers_and_settings_types() {
    let root = Scratch::new().with(&[
        ("gleam.toml", "name = \"app\"\n"),
        ("src/nested/source.gleam", "pub fn a() { 1 }"),
        ("priv/nested/index.html", "hello"),
    ]);
    let v = toml::Value::Table(Default::default());
    let mut out = String::new();
    tree(&root, &v, &root, &mut out).unwrap();
    assert!(out.contains("src/nested/source.gleam [graph]"));
    assert!(out.contains("priv/nested/index.html [watched; 1 lines]"));
    fs::write(root.join(".gitignore"), "priv/nested/\n").unwrap();
    out.clear();
    tree(&root, &v, &root, &mut out).unwrap();
    assert!(!out.contains("index.html"));
}
/// Moving a graph path with no store names a certain module, and keeps the
/// can't-tell hint for a path whose module depends on a declaration (D-181).
#[test]
fn move_names_a_certain_module_without_a_store() {
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/bin/x.rs", "fn main() {}\n"),
    ]);
    let v = toml::Value::Table(Default::default());
    let e = movable(&root, &v, "src/lib.rs").unwrap_err().0;
    assert!(e.contains("E6001") && e.contains("submit --new crate"), "{e}");
    let e = movable(&root, &v, "src/bin/x.rs").unwrap_err().0;
    assert!(e.contains("can't tell which module"), "{e}");
}

#[test]
fn grep_keeps_the_match_when_lowercasing_changes_the_line_length() {
    let text = format!("{}needle{}\n", "İ".repeat(301), "€".repeat(900));
    let (shown, cut) = grep_file("a.md", &text, "needle", 0).unwrap();
    assert!(shown.contains("needle") && cut, "{shown}");
}

#[test]
fn grep_finds_a_word_final_sigma_lowercased_as_a_whole() {
    for query in ["ΟΔΟΣ", "οδος"] {
        let (shown, _) = grep_file("a.md", "ΟΔΟΣ here\n", &query.to_lowercase(), 0).unwrap();
        assert!(shown.contains("ΟΔΟΣ"), "{query}: {shown}");
    }
}

