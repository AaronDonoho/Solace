use super::*;
use serde_json::json;
use testing::*;

#[test]
fn a_review_versions_the_store_and_reaches_its_changes() {
    let store = project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
    let ix = version(&store, Version::Current).unwrap();
    let task = json!({"id": "t", "changes": [{"entity": "symbol:fn:calc.add"}]});
    let reach = task_reach(&store, &ix, &ix, &task).unwrap();
    assert!(reach["tests"].is_array());
}

#[test]
fn a_scratch_copy_holds_the_project_and_its_build_folders() {
    let source = "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n";
    let s = project(&[("src/calc.gleam", source)]);
    fs::create_dir_all(s.root.join("build/packages/calc")).unwrap();
    fs::write(s.root.join("build/packages/calc/keep"), "built").unwrap();
    let dir = testing::Scratch::new();
    scratch(&s, &dir).unwrap();
    assert_eq!(fs::read_to_string(dir.join("src/calc.gleam")).unwrap(), source);
    assert_eq!(fs::read_to_string(dir.join("build/packages/calc/keep")).unwrap(), "built");
}
