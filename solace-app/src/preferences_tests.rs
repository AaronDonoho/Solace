use super::*;

fn test_path(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("solace-preferences-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("preferences.json")
}

#[test]
fn a_closed_board_windows_size_opens_the_next_one() {
    let path = test_path("size");
    let store = Store::open(path.clone());
    assert_eq!(store.size(), DEFAULT_SIZE);
    store.set_size(1500.0, 900.0).unwrap();
    store.set_size(0.0, 0.0).unwrap();
    assert_eq!(Store::open(path.clone()).size(), (1500.0, 900.0));
    fs::write(&path, r#"{"size":[0,820]}"#).unwrap();
    assert_eq!(Store::open(path).size(), DEFAULT_SIZE);
}

#[test]
fn settings_survive_a_restart_and_a_cleared_one_stays_cleared() {
    let path = test_path("settings");
    let store = Store::open(path.clone());
    store.set_setting("solace-font-size", "20").unwrap();
    store.set_setting("solace-theme", "dark").unwrap();
    store.set_setting("solace-theme", "").unwrap();
    let settings = Store::open(path).settings();
    let expected: BTreeMap<String, String> = [
        ("solace-font-size".to_string(), "20".to_string()),
        ("solace-theme".to_string(), String::new()),
    ]
    .into();
    assert_eq!(settings, expected);
    let script = seed_script(&settings);
    assert!(script.contains(r#"{"solace-font-size":"20","solace-theme":""}"#));
    assert!(script.contains("sessionStorage.getItem('solace-seeded')"));
}

#[test]
fn a_save_replaces_the_whole_file() {
    let path = test_path("whole");
    let store = Store::open(path.clone());
    store
        .set_setting("solace-theme", &"dark".repeat(100))
        .unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let reader = fs::File::open(&path).unwrap();
    store.set_setting("solace-theme", "").unwrap();
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved, to_json(&store.kept.lock().unwrap()));
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut seen = String::new();
        (&reader).read_to_string(&mut seen).unwrap();
        assert_eq!(seen, before);
    }
    drop(reader);
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("preferences.json")]);
}
