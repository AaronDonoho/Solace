use super::*;
#[test]
fn bytes_reads_a_number_array() {
    assert_eq!(
        bytes(&[serde_json::json!(65), serde_json::json!(255), serde_json::json!("x"), serde_json::json!(-1)]),
        [65, 255, 0, 0]
    );
}

#[test]
fn read_and_read_opt_read_the_json() {
    let root = crate::testing::Scratch::new().with(&[("a.json", "{\"x\":1}")]);
    let path = root.join("a.json");
    assert_eq!(read(&path).unwrap(), serde_json::json!({"x": 1}));
    assert_eq!(read_opt(&path), Some(serde_json::json!({"x": 1})));
}

#[test]
fn read_opt_accepts_missing_or_invalid_files() {
    let root = crate::testing::Scratch::new().with(&[("bad.json", "{")]);
    assert!(read_opt(&root.join("missing.json")).is_none());
    assert!(read_opt(&root.join("bad.json")).is_none());
    assert!(read(&root.join("bad.json")).is_err());
}

#[test]
fn write_and_write_pretty_write_whole_files() {
    let root = crate::testing::Scratch::new();
    let v = serde_json::json!({"a": 1});
    let compact = root.join("compact.json");
    write(&compact, &v).unwrap();
    assert_eq!(std::fs::read_to_string(&compact).unwrap(), "{\"a\":1}");
    let pretty = root.join("pretty.json");
    write_pretty(&pretty, &v).unwrap();
    assert_eq!(read(&pretty).unwrap(), v);
    assert!(std::fs::read_to_string(&pretty).unwrap().contains('\n'));
}

#[test]
fn replace_pretty_writes_a_synced_json_file() {
    let root = crate::testing::Scratch::new();
    let dir = root.join("made");
    replace_pretty(&dir, "a.json", &serde_json::json!({"a": 1})).unwrap();
    assert_eq!(read(&dir.join("a.json")).unwrap(), serde_json::json!({"a": 1}));
    assert!(!dir.join("a.json.tmp").exists());
}
