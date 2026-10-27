use super::*;
#[test]
fn a_file_is_written_whole() {
    let root = crate::testing::Scratch::new();
    let path = root.join("made/whole.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_atomic(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert!(!root.join("made/whole.txt.tmp").exists());
}

#[test]
fn a_replaced_file_is_synced_and_renamed() {
    let root = crate::testing::Scratch::new();
    let dir = root.join("made/nested");
    replace_bytes(&dir, "a.json", b"one").unwrap();
    assert_eq!(std::fs::read(dir.join("a.json")).unwrap(), b"one");
    replace_bytes(&dir, "a.json", b"two").unwrap();
    assert_eq!(std::fs::read(dir.join("a.json")).unwrap(), b"two");
    assert!(!dir.join("a.json.tmp").exists());
}

#[test]
fn a_try_lock_is_none_while_another_holds_it() {
    let root = crate::testing::Scratch::new();
    let path = root.join("lock");
    let held = lock_file(&path).unwrap();
    assert!(try_lock_file(&path).unwrap().is_none());
    drop(held);
    assert!(try_lock_file(&path).unwrap().is_some());
}
