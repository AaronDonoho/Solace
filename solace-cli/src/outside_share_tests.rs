use super::*;

#[cfg(test)]
#[test]
fn ignores_store_sees_an_excluded_store_directory() {
    assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "."));
    assert!(ignores_store("**/.solace\n!**/.solace/store.db\n", "project"));
    assert!(ignores_store("project/.solace/\n!project/.solace/store.db\n", "project"));
    assert!(!ignores_store("!.solace/\n!**/.solace/store.db\n", "."));
    assert!(!ignores_store("other/.solace/\n!other/.solace/store.db\n", "project"));
    assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "project"));
    assert!(ignores_store(".solace/\n!**/.solace/store.db\n", "two/nested"));
    assert!(ignores_store("*/.solace/\n", "project"));
    assert!(!ignores_store("*/.solace/\n", "two/nested"));
    assert!(ignores_store("/.solace/\n", "."));
    assert!(!ignores_store("/.solace/\n", "project"));
}

#[cfg(test)]
#[test]
fn ignores_store_sees_store_file_patterns_at_their_paths() {
    assert!(ignores_store("*\n", "project"));
    assert!(ignores_store("**\n", "project"));
    assert!(ignores_store("**/*\n", "project"));
    assert!(ignores_store("store.db\n", "project"));
    assert!(ignores_store("*.db\n", "project"));
    assert!(ignores_store("store.*\n", "project"));
    assert!(ignores_store("**/.solace/store.db\n", "project"));
    assert!(!ignores_store("/store.db\n", "."));
    assert!(!ignores_store("/store.db\n", "project"));
    assert!(!ignores_store("/*.db\n", "project"));
    assert!(!ignores_store("/store.*\n", "project"));
    assert!(ignores_store("/*\n", "."));
    assert!(ignores_store("/*\n", "project"));
    assert!(ignores_store("/*\n!/.solace/store.db\n", "."));
    assert!(!ignores_store("/*\n!/.solace\n", "."));
    assert!(ignores_store("/project\n", "project"));
    assert!(!ignores_store("/project\n", "one"));
    assert!(ignores_store("project/\n", "project"));
    assert!(ignores_store("/.solace/store.db\n", "."));
    assert!(!ignores_store("/.solace/store.db\n", "project"));
    assert!(ignores_store("/project/.solace/store.db\n", "project"));
}

fn root_with_two_projects() -> crate::testing::Scratch {
    crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"one\", \"two/nested\"]\n"),
        (".solace/store.db", ""),
        ("one/.solace/store.db", ""),
        ("two/nested/.solace/store.db", ""),
    ])
}

/// D-179: one root block shares every store; setting up again leaves it alone.
#[test]
fn the_store_is_committed_with_the_project() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() { 1 }\n")]);
    assert!(!p.root.join(".solace/.gitignore").exists());

    let root = root_with_two_projects();
    share_store(&root.join("one")).unwrap();
    let once = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(once, STORE_IGNORE);
    share_store(&root.join("two/nested")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), STORE_IGNORE);
    assert_eq!(once.matches(BLOCK_START).count(), 1);
    assert!(!root.join("one/.solace/.gitignore").exists());
    assert!(!root.join("two/nested/.solace/.gitignore").exists());
}

#[test]
fn only_the_store_block_is_changed() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), format!("# keep before\n{BLOCK_START}# old contents\n{BLOCK_END}# keep after\n")).unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), format!("# keep before\n{STORE_IGNORE}# keep after\n"));

    fs::write(root.join(".gitignore"), "# no final newline").unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), format!("# no final newline\n{STORE_IGNORE}"));

    let partial = format!("# keep\n{BLOCK_START}# no end marker\n");
    fs::write(root.join(".gitignore"), &partial).unwrap();
    share_store(&root.join("one")).unwrap();
    let appended = format!("{partial}{STORE_IGNORE}");
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), appended);
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), appended);
}

#[test]
fn generated_store_ignores_are_removed_and_an_edited_one_stays() {
    let root = root_with_two_projects();
    let path = root.join("one/.solace/.gitignore");
    for old in OLD_STORE_IGNORES {
        fs::write(&path, old).unwrap();
        share_store(&root.join("one")).unwrap();
        assert!(!path.exists(), "{old:?}");
    }
    fs::write(&path, "# mine\n*\n!/custom\n").unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# mine\n*\n!/custom\n");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    share_store(&root.join("one")).unwrap();
    assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe]);
}

#[test]
fn a_folder_without_a_store_gets_no_ignore_file() {
    let bare = crate::testing::Scratch::new();
    share_store(&bare).unwrap();
    assert!(!bare.join(".solace").exists());
    assert!(!bare.join(".gitignore").exists());
}

/// D-191: the generated block keeps the store committed, and its markers match.
#[test]
fn the_generated_store_ignore_is_pinned() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn one() { 1 }\n")]);
    assert!(STORE_IGNORE.starts_with(BLOCK_START));
    assert!(STORE_IGNORE.ends_with(BLOCK_END));
    assert_eq!(
        fs::read_to_string(p.root.join(".gitignore")).unwrap(),
        concat!(
            "# Solace: shared store files begin; setup replaces the lines below (D-179).\n",
            "**/.solace/*\n",
            "!**/.solace/store.db\n",
            "!**/.solace/approved\n",
            "# Solace: shared store files end.\n",
        )
    );
}

#[test]
fn another_projects_store_directory_is_not_this_projects_warning() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), "two/nested/.solace/\n").unwrap();
    assert_eq!(hidden_note(None, &root.join("one")).text, "");
    assert!(hidden_note(None, &root.join("two/nested")).text.contains("still ignores store.db"));
}

#[test]
fn a_bare_store_directory_ignore_still_warns_for_a_nested_project() {
    let root = root_with_two_projects();
    fs::write(root.join(".gitignore"), ".solace/\n").unwrap();
    assert!(hidden_note(None, &root.join("two/nested")).text.contains("still ignores store.db"));
}

#[test]
fn an_ignore_file_that_still_hides_the_store_is_flagged() {
    let s = crate::testing::project(&[("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    crate::board::task_model::open(&s.root).unwrap();
    let ignore = s.root.join(".solace/.gitignore");
    fs::write(&ignore, "# mine\n*\n!/custom\n").unwrap();
    let note = hidden_note(None, &s.root);
    assert!(note.text.contains("still ignores store.db"), "{}", note.text);
    let again = hidden_note(note.hidden.as_deref(), &s.root);
    assert!(!again.text.contains("still ignores store.db"), "{}", again.text);
    assert!(!again.dirty);
    fs::write(&ignore, "# mine again\n*\n!/custom\n").unwrap();
    let changed = hidden_note(again.hidden.as_deref(), &s.root);
    assert!(changed.text.contains("still ignores store.db"), "{}", changed.text);
    assert!(changed.dirty);
    fs::write(&ignore, "# mine\n!store.db\n").unwrap();
    let fixed = hidden_note(changed.hidden.as_deref(), &s.root);
    assert!(!fixed.text.contains("still ignores store.db"), "{}", fixed.text);
    assert!(fixed.hidden.is_none());
    assert!(fixed.dirty);
    fs::write(&ignore, "# mine\n*\n!/custom\n").unwrap();
    let regressed = hidden_note(fixed.hidden.as_deref(), &s.root);
    assert!(regressed.text.contains("still ignores store.db"), "{}", regressed.text);
    assert!(regressed.dirty);
}

