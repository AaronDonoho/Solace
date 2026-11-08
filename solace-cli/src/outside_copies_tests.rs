use super::*;



const APP: &str = "pub fn one() -> Int {\n  1\n}\n\npub fn two() -> Int {\n  2\n}\n";

/// The `app` module, `APP`, on a board.
fn app() -> crate::testing::Project {
    let p = crate::testing::project(&[("src/app.gleam", APP)]);
    crate::board::open(&p.root).unwrap();
    p
}

/// A repair copy is the only one for its path, git-ignored, and never scanned.
#[test]
fn a_kept_copy_replaces_the_last_one_and_is_never_project_source() {
    let p = crate::testing::project(&[("src/app.gleam", APP)]);
    let lost = "pub fn lost() -> Int {\n  9\n}\n";
    let kept = keep(&p.root, "src/app.gleam", lost.as_bytes()).unwrap();
    assert_eq!(kept, ".solace/outside/src/app.gleam");
    let mut ignores = crate::lockdown::paths::Ignores::new(&p.root);
    assert!(ignores.ignored(&kept));
    let v = crate::lockdown::settings::settings(&p.root).unwrap();
    let files = crate::lockdown::dispatch(&p, &v, &["files"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap();
    assert!(files.contains(".solace/ [internal]") && !files.contains("outside/src/app.gleam"), "{files}");
    let grep = crate::lockdown::dispatch(&p, &v, &["grep", "lost"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap();
    assert!(grep.starts_with("No matches for \"lost\""), "{grep}");
    keep(&p.root, "src/app.gleam", b"newer").unwrap();
    assert_eq!(std::fs::read_to_string(p.root.join(&kept)).unwrap(), "newer");
    assert_eq!(std::fs::read_dir(p.root.join(".solace/outside")).unwrap().count(), 1);
}

/// A 16-hex source folder is a current path key, not an old id.
#[test]
fn a_16_hex_source_folder_keeps_its_copy() {
    let s = app();
    let name = "0123456789abcdef";
    let copy = s.root.join(".solace/outside").join(name);
    std::fs::create_dir_all(&copy).unwrap();
    remove_old_copies(&s.root, &[format!("{name}/app.gleam")]).unwrap();
    assert!(copy.exists());
}

/// Copies from the old id-keyed layout go; path-keyed copies stay.
#[test]
fn old_id_copies_are_removed() {
    let s = app();
    let old = s.root.join(".solace/outside").join(crate::unique_id());
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("app.gleam"), "lost").unwrap();
    keep(&s.root, "src/app.gleam", b"kept").unwrap();
    crate::outside::absorb(&s, crate::outside::Mode::Command).unwrap();
    assert!(!old.exists());
    assert!(s.root.join(".solace/outside/src/app.gleam").exists());
}
