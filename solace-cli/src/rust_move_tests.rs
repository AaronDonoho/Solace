use super::*;
use crate::import;
use crate::testing::{CARGO, Scratch};
use crate::board::reviews::snapshot;
use crate::board::reviews::restore;

const A: &str = "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\nmod tests;\n";

fn crate_with_tests() -> (Scratch, Store) {
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", A),
        ("src/a/tests.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    (root, s)
}

fn id(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.syms[s.resolve(&ix, name).unwrap()].id.clone()
}

fn moved(s: &Store, from: &str, to: &str) -> Out {
    let settings = toml::Value::Table(Default::default());
    crate::lockdown::dispatch(s, &settings, &["move", from, to], &mut || Ok(String::new()), None, 10_000).unwrap()
}

fn text(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn a_module_file_moves_and_keeps_its_ids() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (root, s) = crate_with_tests();
    let before = id(&s, "a::tests.one_is_one");
    s.db.execute("UPDATE modules SET dir = 'src/a' WHERE path = 'a::tests'", []).unwrap();
    assert_eq!(dir(&s, "a::tests"), "src/a");
    let snapshot = snapshot(&s).unwrap();
    let out = moved(&s, "src/a/tests.rs", "src/a_tests.rs").unwrap();
    assert!(out.contains("Moved src/a/tests.rs to src/a_tests.rs"), "{out}");
    assert_eq!(id(&s, "a::tests.one_is_one"), before);
    assert_eq!(dir(&s, "a::tests"), "src");
    assert!(!root.join("src/a/tests.rs").exists());
    assert!(text(&root, "src/a_tests.rs").contains("fn one_is_one()"));
    assert!(text(&root, "src/a.rs").ends_with("#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n"), "{}", text(&root, "src/a.rs"));
    let states: Vec<(String, serde_json::Value)> = snapshot.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    restore(&s, &states).unwrap();
    assert!(root.join("src/a/tests.rs").exists());
    assert!(!root.join("src/a_tests.rs").exists());
    assert_eq!(dir(&s, "a::tests"), "src/a");
    assert_eq!(text(&root, "src/a.rs"), A);
}

#[test]
fn a_header_mod_line_takes_a_path_and_drops_it_at_home() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod b;\n\npub fn two() -> i32 {\n    b::one() + 1\n}\n"),
        ("src/b.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    moved(&s, "src/b.rs", "src/b_file.rs").unwrap();
    assert!(text(&root, "src/lib.rs").starts_with("#[path = \"b_file.rs\"]\nmod b;\n"), "{}", text(&root, "src/lib.rs"));
    moved(&s, "src/b_file.rs", "src/b.rs").unwrap();
    assert!(text(&root, "src/lib.rs").starts_with("mod b;\n"), "{}", text(&root, "src/lib.rs"));
    assert!(!root.join("src/b_file.rs").exists());
}

#[test]
fn a_module_move_is_refused_before_anything_changes() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (root, s) = crate_with_tests();
    fs::write(root.join("src/taken.rs"), "").unwrap();
    for (from, to, says) in [
        ("src/a/tests.rs", "src/taken.rs", "already exists"),
        ("src/a/tests.rs", "tests/a.rs", "parent's folder"),
        ("src/a/tests.rs", "src/a_tests.txt", "parent's folder"),
        ("src/a.rs", "src/b.rs", "modules of its own"),
        ("src/lib.rs", "src/root.rs", "held as symbols"),
    ] {
        let err = moved(&s, from, to).unwrap_err().0;
        assert!(err.contains(says), "{from} -> {to}: {err}");
    }
    assert!(text(&root, "src/a/tests.rs").contains("fn one_is_one()"));
    assert_eq!(text(&root, "src/a.rs"), A);
}

fn dir(s: &Store, name: &str) -> String {
    let ix = s.index().unwrap();
    ix.modules.iter().find(|m| m.path == name).unwrap().dir.clone()
}

#[test]
fn a_spaced_path_on_the_mod_line_is_replaced() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\n#[ path = \"a_tests_old.rs\" ] mod tests;\n"),
        ("src/a_tests_old.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    moved(&s, "src/a_tests_old.rs", "src/a_tests.rs").unwrap();
    assert_eq!(text(&root, "src/a.rs"), "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n");
}

#[test]
fn a_cfg_attr_path_is_refused() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\n#[cfg_attr(unix, path = \"a_tests_old.rs\")]\nmod tests;\n"),
        ("src/a_tests_old.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = moved(&s, "src/a_tests_old.rs", "src/a_tests.rs").unwrap_err().0;
    assert!(err.contains("cfg_attr"), "{err}");
    assert!(text(&root, "src/a_tests_old.rs").contains("fn one_is_one()"));
    assert!(text(&root, "src/a.rs").contains("cfg_attr"));
}

#[test]
fn a_different_path_is_replaced() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\n#[path = \"old.rs\"]\nmod tests;\n"),
        ("src/old.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    moved(&s, "src/old.rs", "src/a_tests.rs").unwrap();
    assert_eq!(text(&root, "src/a.rs"), "pub fn one() -> i32 {\n    1\n}\n\n#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n");
    assert!(text(&root, "src/a_tests.rs").contains("fn one_is_one()"));
    assert!(!root.join("src/old.rs").exists());
}

#[test]
fn a_stale_module_row_takes_the_new_path() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\npub mod taken;\n"),
        ("src/a.rs", A),
        ("src/a/tests.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
        ("src/taken.rs", "pub fn taken() {}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    fs::remove_file(root.join("src/taken.rs")).unwrap();
    let err = moved(&s, "src/a/tests.rs", "src/taken.rs").unwrap_err().0;
    assert!(err.contains("already exists"), "{err}");
    assert!(text(&root, "src/a/tests.rs").contains("fn one_is_one()"));
}

#[test]
fn an_integration_test_module_is_refused() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("tests/x.rs", "#[test]\nfn works() {}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    let err = moved(&s, "tests/x.rs", "tests/x_file.rs").unwrap_err().0;
    assert!(err.contains("no stored parent module"), "{err}");
    assert!(root.join("tests/x.rs").exists());
}

#[test]
fn a_failed_check_restores_the_file_and_its_folder() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n\nmod tests;\n"),
        (
            "src/a/tests.rs",
            concat!(
                "use super::*;\n\nconst DATA: &str = include_str!(\"data.txt\");\n\n",
                "fn size() -> usize {\n    DATA.len()\n}\n\n",
                "#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n    assert!(size() > 0);\n}\n"
            ),
        ),
        ("src/a/data.txt", "hello\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    s.db.execute("UPDATE modules SET dir = 'src/a' WHERE path = 'a::tests'", []).unwrap();
    let err = moved(&s, "src/a/tests.rs", "src/a_tests.rs").unwrap_err().0;
    assert!(err.contains("does not type-check"), "{err}");
    assert!(root.join("src/a/tests.rs").exists());
    assert!(!root.join("src/a_tests.rs").exists());
    assert_eq!(text(&root, "src/a.rs"), "pub fn one() -> i32 {\n    1\n}\n\nmod tests;\n");
    assert_eq!(dir(&s, "a::tests"), "src/a");
}

#[test]
fn a_parent_with_no_mod_line_is_refused() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "#[cfg(test)]\nmod tests;\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/a/tests.rs", "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    s.set_header("a", "").unwrap();
    let err = moved(&s, "src/a/tests.rs", "src/a_tests.rs").unwrap_err().0;
    assert!(err.contains("has no `mod tests;` line"), "{err}");
    assert!(root.join("src/a/tests.rs").exists());
    assert!(!root.join("src/a_tests.rs").exists());
}

#[test]
fn a_path_like_doc_string_does_not_hide_the_mod_line() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            r##"#[doc = "a]b"] #[doc = r#"a path = "x"] b"#] mod b;

pub fn two() -> i32 {
    b::one() + 1
}
"##,
        ),
        ("src/b.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    let s = Store::at(&root).unwrap();
    moved(&s, "src/b.rs", "src/b_file.rs").unwrap();
    let header = text(&root, "src/lib.rs");
    assert!(header.starts_with("#[path = \"b_file.rs\"]\n#[doc = \"a]b\"] #[doc = r#\"a path = \"x\"] b\"#] mod b;\n"), "{header}");
    assert_eq!(header.matches("#[path").count(), 1, "{header}");
}
