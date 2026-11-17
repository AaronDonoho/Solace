use super::*;
use crate::testing::Scratch;
use std::fs;

fn workspace() -> Scratch {
    Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        (
            "app/Cargo.toml",
            concat!("[package]\n", "name = \"app\"\n", "version = \"0.1.0\"\n", "\n", "[dependencies]\n", "tauri = { version = \"2\" }\n"),
        ),
        ("app/gleam.toml", "name = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nx = \"1.0.0\"\n"),
        ("app/package.json", "{\n  \"version\": \"0.1.0\",\n  \"dependencies\": {\n    \"x\": \"^1\"\n  }\n}\n"),
        ("app/tauri.conf.json", "{\n  \"version\": \"0.1.0\",\n  \"build\": {}\n}\n"),
    ])
}

#[test]
fn versions_are_read_written_and_restored_in_place() {
    let root = workspace();
    let all = json(&root).unwrap();
    assert_eq!(all["versions"].as_array().unwrap().len(), 4);
    assert_eq!(all["agree"], true);
    assert_eq!(all["proposal"], "0.1.1");

    let list = edits(&root, "0.2.0").unwrap();
    write(&root, &list).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/Cargo.toml")).unwrap(),
        concat!("[package]\n", "name = \"app\"\n", "version = \"0.2.0\"\n", "\n", "[dependencies]\n", "tauri = { version = \"2\" }\n")
    );
    assert_eq!(
        fs::read_to_string(root.join("app/gleam.toml")).unwrap(),
        "name = \"app\"\nversion = \"0.2.0\"\n\n[dependencies]\nx = \"1.0.0\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/package.json")).unwrap(),
        "{\n  \"version\": \"0.2.0\",\n  \"dependencies\": {\n    \"x\": \"^1\"\n  }\n}\n"
    );
    assert_eq!(fs::read_to_string(root.join("app/tauri.conf.json")).unwrap(), "{\n  \"version\": \"0.2.0\",\n  \"build\": {}\n}\n");

    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/gleam.toml")).unwrap(),
        "name = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nx = \"1.0.0\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/Cargo.toml")).unwrap(),
        concat!("[package]\n", "name = \"app\"\n", "version = \"0.1.0\"\n", "\n", "[dependencies]\n", "tauri = { version = \"2\" }\n")
    );
}

#[test]
fn disagreeing_versions_are_flagged() {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n"),
        ("app/gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"),
    ]);
    assert_eq!(json(&root).unwrap()["agree"], false);
}

#[test]
fn a_file_without_a_version_is_left_alone() {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion.workspace = true\n"),
        ("app/gleam.toml", "[dependencies]\nx = \"1.0.0\"\n"),
        ("app/package.json", "{\n  \"private\": true\n}\n"),
        ("app/tauri.conf.json", "{\n  \"build\": {}\n}\n"),
    ]);
    let all = json(&root).unwrap();
    assert_eq!(all["versions"].as_array().unwrap().len(), 0);
    assert!(edits(&root, "0.2.0").unwrap().is_empty());
}

#[test]
fn the_proposal_follows_the_last_release_and_a_taken_version_is_refused() {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n"),
        ("build-output/0.2.0/release.json", "{\"version\": \"0.2.0\"}\n"),
    ]);
    assert_eq!(proposal(&root), "0.2.1");
    assert!(check(&root, "0.2.0").is_err());
    assert!(check(&root, "1.2").is_err());
    assert!(check(&root, "0.2.1").is_ok());
}
