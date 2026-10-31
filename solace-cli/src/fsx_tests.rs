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

#[test]
fn try_lock_file_within_succeeds_when_lock_is_released_during_wait() {
    let root = crate::testing::Scratch::new();
    let path = root.join("lock");
    let held = lock_file(&path).unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(held);
    });
    let result = try_lock_file_within(&path, Duration::from_millis(500)).unwrap();
    assert!(result.is_some());
}

#[test]
fn try_lock_file_within_fails_if_lock_is_never_released() {
    let root = crate::testing::Scratch::new();
    let path = root.join("lock");
    let _held = lock_file(&path).unwrap();
    let started = Instant::now();
    let result = try_lock_file_within(&path, Duration::from_millis(60)).unwrap();
    let waited = started.elapsed();
    assert!(result.is_none());
    assert!(waited < Duration::from_millis(90), "the wait honors its deadline: {waited:?}");
}

#[test]
fn a_copy_keeps_the_source_time() {
    let dir = crate::testing::Scratch::new().with(&[("from", "one\n")]);
    let (from, to) = (dir.join("from"), dir.join("to"));
    copy_if_changed(&from, &to).unwrap();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "one\n");
    assert_eq!(std::fs::metadata(&to).unwrap().modified().unwrap(), std::fs::metadata(&from).unwrap().modified().unwrap());
}

#[test]
fn a_copy_with_the_same_contents_takes_the_source_time() {
    let dir = crate::testing::Scratch::new().with(&[("from", "one\n")]);
    let (from, to) = (dir.join("from"), dir.join("to"));
    copy_if_changed(&from, &to).unwrap();
    let source = std::fs::metadata(&from).unwrap().modified().unwrap();
    let older = source - std::time::Duration::from_secs(3600);
    let file = std::fs::OpenOptions::new().write(true).open(&to).unwrap();
    file.set_modified(older).unwrap();
    copy_if_changed(&from, &to).unwrap();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "one\n");
    assert_eq!(std::fs::metadata(&to).unwrap().modified().unwrap(), source);
}
