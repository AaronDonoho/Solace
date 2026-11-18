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
        ("app/Cargo.lock", "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n"),
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
        fs::read_to_string(root.join("app/Cargo.lock")).unwrap(),
        "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.2.0\"\n"
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
    assert_eq!(
        fs::read_to_string(root.join("app/Cargo.lock")).unwrap(),
        "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n"
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

#[test]
fn gleam_lock_files_follow_the_written_local_packages() {
    let manifest = concat!(
        "# local packages\n",
        "[requirements]\n",
        "gleam_stdlib = [\"hex\", \"1.0.0\"]\n",
        "{ name = \"components\", version = \"1.0.0\", build_tools = [\"gleam\"], source = \"local\", path = \"../components\" }\n",
        "{ name = \"registry\", version = \"0.1.1\", build_tools = [\"gleam\"], source = \"hex\" }\n",
    );
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"components\"]\n"),
        ("app/gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        ("app/manifest.toml", manifest),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    write(&root, &list).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/manifest.toml")).unwrap(),
        concat!(
            "# local packages\n",
            "[requirements]\n",
            "gleam_stdlib = [\"hex\", \"1.0.0\"]\n",
            "{ name = \"components\", version = \"0.1.1\", build_tools = [\"gleam\"], source = \"local\", path = \"../components\" }\n",
            "{ name = \"registry\", version = \"0.1.1\", build_tools = [\"gleam\"], source = \"hex\" }\n",
        )
    );
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(fs::read_to_string(root.join("app/manifest.toml")).unwrap(), manifest);
}

#[test]
fn cargo_lock_files_follow_the_written_crates() {
    let lock = concat!(
        "version = 4\n",
        "\n",
        "[[package]]\n",
        "name = \"app\"\n",
        "version = \"1.0.0\"\n",
        "\n",
        "[[package]]\n",
        "name = \"registry\"\n",
        "version = \"0.1.1\"\n",
        "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        "checksum = \"abc\"\n",
    );
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"1.0.0\"\n"),
        ("app/Cargo.lock", lock),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    write(&root, &list).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/Cargo.lock")).unwrap(),
        concat!(
            "version = 4\n",
            "\n",
            "[[package]]\n",
            "name = \"app\"\n",
            "version = \"0.1.1\"\n",
            "\n",
            "[[package]]\n",
            "name = \"registry\"\n",
            "version = \"0.1.1\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n",
        )
    );
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(fs::read_to_string(root.join("app/Cargo.lock")).unwrap(), lock);
}

#[test]
fn npm_lock_files_follow_the_written_workspace_packages() {
    let lock = concat!(
        "{\n",
        "  \"name\": \"app\",\n",
        "  \"version\": \"1.0.0\",\n",
        "  \"lockfileVersion\": 3,\n",
        "  \"requires\": true,\n",
        "  \"packages\": {\n",
        "    \"\": {\n",
        "      \"name\": \"app\",\n",
        "      \"version\": \"1.0.0\"\n",
        "    },\n",
        "    \"packages/child\": {\n",
        "      \"name\": \"child\",\n",
        "      \"version\": \"1.0.0\"\n",
        "    },\n",
        "    \"node_modules/registry\": {\n",
        "      \"version\": \"0.1.1\"\n",
        "    }\n",
        "  }\n",
        "}\n",
    );
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"app/packages/child\"]\n"),
        ("app/package.json", "{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\"\n}\n"),
        ("app/packages/child/package.json", "{\n  \"name\": \"child\",\n  \"version\": \"1.0.0\"\n}\n"),
        ("app/package-lock.json", lock),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    write(&root, &list).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/package-lock.json")).unwrap(),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.1\",\n",
            "  \"lockfileVersion\": 3,\n",
            "  \"requires\": true,\n",
            "  \"packages\": {\n",
            "    \"\": {\n",
            "      \"name\": \"app\",\n",
            "      \"version\": \"0.1.1\"\n",
            "    },\n",
            "    \"packages/child\": {\n",
            "      \"name\": \"child\",\n",
            "      \"version\": \"0.1.1\"\n",
            "    },\n",
            "    \"node_modules/registry\": {\n",
            "      \"version\": \"0.1.1\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        )
    );
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(fs::read_to_string(root.join("app/package-lock.json")).unwrap(), lock);
}

#[test]
fn bun_lock_files_follow_the_written_workspace_packages() {
    let lock = concat!(
        "{\n",
        "  \"lockfileVersion\": 1,\n",
        "  \"workspaces\": {\n",
        "    \"\": {\n",
        "      \"name\": \"app\",\n",
        "      \"dependencies\": {\n",
        "        \"child\": \"workspace:packages/child\"\n",
        "      }\n",
        "    },\n",
        "    \"packages/child\": {\n",
        "      \"name\": \"child\",\n",
        "      \"version\": \"1.0.0\"\n",
        "    },\n",
        "    \"packages/other\": {\n",
        "      \"name\": \"other\",\n",
        "      \"version\": \"0.1.1\"\n",
        "    }\n",
        "  },\n",
        "  \"packages\": {\n",
        "    \"child@workspace:packages/child\": [\"child@workspace:packages/child\", {}, \"workspace:packages/child\"],\n",
        "  }\n",
        "}\n",
    );
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"app/packages/child\"]\n"),
        ("app/package.json", "{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\"\n}\n"),
        ("app/packages/child/package.json", "{\n  \"name\": \"child\",\n  \"version\": \"1.0.0\"\n}\n"),
        ("app/bun.lock", lock),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    write(&root, &list).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/bun.lock")).unwrap(),
        concat!(
            "{\n",
            "  \"lockfileVersion\": 1,\n",
            "  \"workspaces\": {\n",
            "    \"\": {\n",
            "      \"name\": \"app\",\n",
            "      \"dependencies\": {\n",
            "        \"child\": \"workspace:packages/child\"\n",
            "      }\n",
            "    },\n",
            "    \"packages/child\": {\n",
            "      \"name\": \"child\",\n",
            "      \"version\": \"0.1.1\"\n",
            "    },\n",
            "    \"packages/other\": {\n",
            "      \"name\": \"other\",\n",
            "      \"version\": \"0.1.1\"\n",
            "    }\n",
            "  },\n",
            "  \"packages\": {\n",
            "    \"child@workspace:packages/child\": [\"child@workspace:packages/child\", {}, \"workspace:packages/child\"],\n",
            "  }\n",
            "}\n",
        )
    );
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(fs::read_to_string(root.join("app/bun.lock")).unwrap(), lock);
}

fn installed(program: &str) -> bool {
    std::process::Command::new(program).arg("--version").output().is_ok_and(|out| out.status.success())
}

fn tool_ok(dir: &Path, program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program).args(args).current_dir(dir).output().is_ok_and(|out| out.status.success())
}

#[test]
fn a_gleam_build_accepts_the_written_versions() {
    if !installed("gleam") {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"components\"]\n"),
        (
            "app/gleam.toml",
            "name = \"app\"\ntarget = \"javascript\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n",
        ),
        ("app/src/app.gleam", "pub fn main() { 1 }\n"),
        ("components/gleam.toml", "name = \"components\"\ntarget = \"javascript\"\nversion = \"1.0.0\"\n"),
        ("components/src/components.gleam", "pub fn one() { 1 }\n"),
    ]);
    assert!(tool_ok(&root.join("app"), "gleam", &["build", "--target", "javascript"]));
    let list = edits(&root, "0.1.1").unwrap();
    assert!(list.iter().any(|e| e.file == "manifest.toml"));
    write(&root, &list).unwrap();
    assert!(tool_ok(&root.join("app"), "gleam", &["build", "--target", "javascript"]));
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
}

#[test]
fn a_cargo_build_locked_accepts_the_written_versions() {
    if !installed("cargo") {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"1.0.0\"\nedition = \"2021\"\n"),
        ("app/src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
        ("app/Cargo.lock", "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"1.0.0\"\n"),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    assert!(list.iter().any(|e| e.file == "Cargo.lock"));
    write(&root, &list).unwrap();
    assert!(tool_ok(&root.join("app"), "cargo", &["build", "--locked"]));
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
}

#[test]
fn npm_ci_accepts_the_written_versions() {
    if !installed("npm") {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"app/packages/child\"]\n"),
        ("app/package.json", "{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\",\n  \"workspaces\": [\"packages/*\"]\n}\n"),
        ("app/packages/child/package.json", "{\n  \"name\": \"child\",\n  \"version\": \"1.0.0\"\n}\n"),
        (
            "app/package-lock.json",
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"1.0.0\",\n",
                "  \"lockfileVersion\": 3,\n",
                "  \"requires\": true,\n",
                "  \"packages\": {\n",
                "    \"\": {\n",
                "      \"name\": \"app\",\n",
                "      \"version\": \"1.0.0\",\n",
                "      \"workspaces\": [\"packages/*\"]\n",
                "    },\n",
                "    \"packages/child\": {\n",
                "      \"name\": \"child\",\n",
                "      \"version\": \"1.0.0\"\n",
                "    },\n",
                "    \"node_modules/child\": {\n",
                "      \"resolved\": \"packages/child\",\n",
                "      \"link\": true\n",
                "    }\n",
                "  }\n",
                "}\n",
            ),
        ),
    ]);
    let list = edits(&root, "0.1.1").unwrap();
    assert!(list.iter().any(|e| e.file == "package-lock.json"));
    write(&root, &list).unwrap();
    assert!(tool_ok(&root.join("app"), "npm", &["ci"]));
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
}

#[test]
fn bun_install_frozen_accepts_the_written_versions() {
    if !installed("bun") {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"app/packages/child\"]\n"),
        ("app/package.json", "{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\",\n  \"workspaces\": [\"packages/*\"]\n}\n"),
        ("app/packages/child/package.json", "{\n  \"name\": \"child\",\n  \"version\": \"1.0.0\"\n}\n"),
    ]);
    assert!(tool_ok(&root.join("app"), "bun", &["install"]));
    let list = edits(&root, "0.1.1").unwrap();
    assert!(list.iter().any(|e| e.file == "bun.lock"));
    write(&root, &list).unwrap();
    assert!(tool_ok(&root.join("app"), "bun", &["install", "--frozen-lockfile"]));
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
}

/// D-176: a workspace member's lock is the workspace's, in the folder above it.
#[test]
fn a_cargo_workspace_members_lock_follows_the_written_crate() {
    if !installed("cargo") {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"member\"]\n"),
        ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n"),
        ("member/Cargo.toml", "[package]\nname = \"member\"\nversion = \"1.0.0\"\nedition = \"2021\"\n"),
        ("member/src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    assert!(tool_ok(&root, "cargo", &["generate-lockfile"]));
    let lock = root.join("Cargo.lock");
    let before = fs::read_to_string(&lock).unwrap();
    assert!(before.contains("name = \"member\"\nversion = \"1.0.0\""));
    let list = edits(&root, "0.1.1").unwrap();
    assert!(list.iter().any(|e| e.file == "../Cargo.lock"));
    write(&root, &list).unwrap();
    assert!(fs::read_to_string(&lock).unwrap().contains("name = \"member\"\nversion = \"0.1.1\""));
    assert!(tool_ok(&root.join("member"), "cargo", &["build", "--locked"]));
    restore(&root, &serde_json::json!({"versionFiles": entries(&list)})).unwrap();
    assert_eq!(fs::read_to_string(&lock).unwrap(), before);
}

#[test]
fn a_project_version_is_read_from_any_manifest() {
    let root = Scratch::new().with(&[("app/tauri.conf.json", "{\n  \"version\": \"1.2.3\"\n}\n")]);
    assert_eq!(at(&root.join("app")).as_deref(), Some("1.2.3"));
    assert_eq!(of(&root, "app").as_deref(), Some("1.2.3"));
}

#[test]
fn the_newest_release_is_read_for_a_warning() {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = []\n"),
        ("build-output/0.2.0/release.json", "{\"version\": \"0.2.0\", \"time\": 1}\n"),
        ("build-output/0.1.1/release.json", "{\"version\": \"0.1.1\", \"time\": 2}\n"),
    ]);
    assert_eq!(newest(&root).as_deref(), Some("0.2.0"));
    assert_eq!(proposal(&root), "0.1.2");
}
