use super::*;
use crate::testing::*;
use std::env;
use std::path::Path;
use std::fs;

/// D-27: the `skills` key names the folder; anything that isn't a folder
/// inside the project is refused.
#[test]
fn the_skills_key_names_a_folder_inside_the_project() {
    for key in ["house", "house/", "a/b", "skills"] {
        let v = parse_settings(&format!("skills = \"{key}\"\n")).unwrap();
        assert_eq!(skills_dir(&v), key.trim_end_matches('/'));
    }
    for bad in [
        "3",
        "true",
        "[\"a\"]",
        "\"/etc\"",
        "\"../x\"",
        "\"a/../b\"",
        "\"\"",
        "\".\"",
        "\"a/..\"",
        "\"src\"",
        "\"test\"",
        "\"src/x\"",
        "\"test/x\"",
        "\".solace\"",
        "\".solace/x\"",
        "\"./src\"",
        "\"./test/x\"",
        "\"./\"",
        "\"./a/../b\"",
    ] {
        assert!(parse_settings(&format!("skills = {bad}\n")).is_err(), "{bad}");
    }
}

/// D-158 point 4, D-175: only a Cargo.toml's dependency, workspace and build keys are
/// settings; a description edit is an ordinary watched write.
#[test]
fn cargo_toml_settings_are_the_dependency_and_build_keys() {
    let base = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    assert!(!cargo_settings_changed(base.as_bytes(), &base.replace("0.1.0", "0.2.0")));
    assert!(!cargo_settings_changed(base.as_bytes(), &format!("{base}description = \"a fixture\"\n")));
    for extra in [
        "[dependencies]\nserde = \"1\"\n",
        "[dev-dependencies]\nserde = \"1\"\n",
        "[build-dependencies]\ncc = \"1\"\n",
        "[target.'cfg(unix)'.dependencies]\nlibc = \"1\"\n",
        "[target.'cfg(windows)'.dev-dependencies]\nwinapi = \"1\"\n",
        "[patch.crates-io]\nserde = { path = \"serde\" }\n",
        "[replace]\n\"serde\" = { path = \"serde\" }\n",
        "[lib]\nproc-macro = true\n",
        "[workspace]\nmembers = [\"member\"]\n",
        "[workspace]\ndefault-members = [\".\", \"member\"]\n",
        "[workspace]\nexclude = [\"member\"]\n",
        "[workspace.dependencies]\nserde = \"1\"\n",
    ] {
        assert!(cargo_settings_changed(base.as_bytes(), &format!("{base}\n{extra}")), "{extra}");
    }
    assert!(!cargo_settings_changed(base.as_bytes(), &format!("{base}\n[workspace]\nresolver = \"2\"\n")));
    let build = base.replace("edition = \"2021\"", "edition = \"2021\"\nbuild = \"build.rs\"");
    assert!(cargo_settings_changed(base.as_bytes(), &build));
    let links = base.replace("edition = \"2021\"", "edition = \"2021\"\nlinks = \"x\"");
    assert!(cargo_settings_changed(base.as_bytes(), &links));
    assert!(cargo_settings_changed(base.as_bytes(), "[package\n"), "text that doesn't parse counts as changed");
}

/// D-175: a crate below the root's manifest and build script are settings
/// in a Rust project, and nowhere else (D-158 point 4).
#[test]
fn a_nested_crates_manifest_and_build_script_are_settings() {
    let rust = Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n")]);
    for name in ["build.rs", "member/build.rs", "member/Cargo.toml", "build/x/build.rs", "node_modules/x/Cargo.toml"] {
        assert!(settings_file(&rust, name), "{name}");
    }
    assert!(!settings_file(&rust, "target/debug/build/x/build.rs"), "target/ stays unwatched");
    assert!(!settings_file(&rust, "Cargo.toml"), "the root's manifest is watched unless a protected key changes");
    let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n")]);
    assert!(!settings_file(&gleam, "build.rs") && !settings_file(&gleam, "member/Cargo.toml"));
}

/// D-177: a `[features]` edit is a setting, so a feature can't turn on an
/// optional dependency without approval.
#[test]
fn a_features_edit_is_a_setting() {
    let base = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    let before = format!("{base}\n[features]\ndefault = []\n");
    assert!(cargo_settings_changed(before.as_bytes(), &format!("{base}\n[features]\ndefault = [\"extra\"]\n")));
    assert!(!cargo_settings_changed(before.as_bytes(), &before.replace("0.1.0", "0.2.0")));
}

#[test]
fn a_reference_section_is_validated_on_its_own() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let v = settings(root).unwrap();
    let clean = fs::read_to_string(root.join("src/reference/project.txt")).unwrap();
    let dir = reference_scratch();
    let p = dir.join("src/reference/project.txt");
    fs::write(&p, &clean).unwrap();
    validate(&dir, &v, "src/reference/project.txt", &p, &clean).unwrap();
    let bad = clean.replacen("\n  build ", "\n   build ", 1);
    fs::write(&p, &bad).unwrap();
    let e = validate(&dir, &v, "src/reference/project.txt", &p, &bad).unwrap_err().0;
    assert!(e.contains("E6003") && e.contains("line 2"), "{e}");
}

fn reference_scratch() -> Scratch {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = Scratch::new();
    fs::create_dir_all(dir.join("tests")).unwrap();
    fs::copy(root.join("tests/reference_layout.py"), dir.join("tests/reference_layout.py")).unwrap();
    fs::create_dir_all(dir.join("src/reference")).unwrap();
    fs::copy(root.join("src/reference.rs"), dir.join("src/reference.rs")).unwrap();
    for entry in fs::read_dir(root.join("src/reference")).unwrap().flatten().filter(|entry| entry.path().is_file()) {
        fs::copy(entry.path(), dir.join("src/reference").join(entry.file_name())).unwrap();
    }
    dir
}

#[test]
fn a_reference_workflow_write_checks_its_summary_against_the_sections() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let v = settings(root).unwrap();
    let dir = reference_scratch();
    let workflow = dir.join("src/reference/workflow.txt");
    let clean = fs::read_to_string(&workflow).unwrap();
    validate(&dir, &v, "src/reference/workflow.txt", &workflow, &clean).unwrap();
    let bad = clean.replacen("  work add | list | done", "  frobnicate  x\n  work add | list | done", 1);
    fs::write(&workflow, &bad).unwrap();
    let e = validate(&dir, &v, "src/reference/workflow.txt", &workflow, &bad).unwrap_err().0;
    assert!(e.contains("E6003") && e.contains("no entry in \"Commands in full\" for `frobnicate`"), "{e}");
}

#[test]
fn a_reference_section_write_checks_the_summary_against_the_joined_sections() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let v = settings(root).unwrap();
    let dir = reference_scratch();
    let old = dir.join("src/reference/workflow.txt");
    let renamed = dir.join("src/reference/start.txt");
    fs::rename(&old, &renamed).unwrap();
    let source = dir.join("src/reference.rs");
    let declared = fs::read_to_string(&source).unwrap();
    let updated = declared.replace("reference/workflow.txt", "reference/start.txt");
    assert_ne!(declared, updated, "the replacement did not match");
    fs::write(&source, updated).unwrap();
    let reading = dir.join("src/reference/reading.txt");
    let clean = fs::read_to_string(&reading).unwrap();
    validate(&dir, &v, "src/reference/reading.txt", &reading, &clean).unwrap();
    let bad = clean.replacen("\n  impact [name...]", "\n  impacX [name...]", 1);
    assert_ne!(bad, clean, "the replacement did not match");
    fs::write(&reading, &bad).unwrap();
    let e = validate(&dir, &v, "src/reference/reading.txt", &reading, &bad).unwrap_err().0;
    assert!(e.contains("E6003") && e.contains("no entry in \"Commands in full\" for `impact`"), "{e}");
}

/// `[sandbox] read` is a list of non-empty paths, and no other sandbox key is
/// accepted (D-158 point 4).
#[cfg(test)]
#[test]
fn sandbox_read_is_a_list_of_paths() {
    assert!(parse_settings("[sandbox]\nread = [\"../fixtures\"]\n").is_ok());
    for bad in ["[sandbox]\nread = \"../fixtures\"\n", "[sandbox]\nread = [\"\"]\n", "[sandbox]\nwrite = []\n"] {
        assert!(parse_settings(bad).is_err(), "{bad}");
    }
}

