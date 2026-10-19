use super::*;
use crate::testing::Scratch;

/// The language comes from the project's files: `gleam.toml` makes a Gleam
/// project; without it the project is plain files, whose empty store reads
/// as Gleam.
#[test]
fn the_language_is_chosen_from_the_project_files() {
    let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n")]);
    let plain = Scratch::new().with(&[("notes.md", "notes\n")]);
    assert_eq!(detect(&gleam.0).map(|l| l.manifest()), Some("gleam.toml"));
    assert!(detect(&plain.0).is_none());
    assert_eq!(of(&plain.0).tool(), "gleam");
}

/// The store records its project's language when it opens, and none for a
/// project of plain files.
#[test]
fn a_store_records_its_language() {
    let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n"), ("src/app.gleam", "pub fn main() { 1 }\n")]);
    let plain = Scratch::new().with(&[("notes.md", "notes\n")]);
    for root in [&gleam.0, &plain.0] {
        crate::import(root).unwrap();
    }
    let named = |s: crate::Store| s.lang.map(|l| l.manifest());
    assert_eq!(named(crate::Store::at(&gleam.0).unwrap()), Some("gleam.toml"));
    assert_eq!(named(crate::Store::peek(&gleam.0).unwrap()), Some("gleam.toml"));
    assert_eq!(named(crate::Store::at(&plain.0).unwrap()), None);
    assert_eq!(named(crate::Store::peek(&plain.0).unwrap()), None);
}
