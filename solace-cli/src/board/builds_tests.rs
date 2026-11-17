use super::*;
use crate::testing::Scratch;
use serde_json::{Value, json};
use std::path::Path;

fn running(root: &Path, id: &str) -> Value {
    let build = json!({
        "id": id,
        "state": "running",
        "startedAt": 1,
        "endedAt": Value::Null,
        "steps": [
            {"project":".","name":"a","command":["sh","-c","true"],"timeout":5,"state":"passed","exitCode":0,"duration":1},
            {"project":".","name":"b","command":["sh","-c","true"],"timeout":5,"state":"running","exitCode":Value::Null,"duration":0},
            {"project":".","name":"c","command":["sh","-c","true"],"timeout":5,"state":"waiting","exitCode":Value::Null,"duration":0}
        ]
    });
    save(root, &build).unwrap();
    build
}

#[test]
fn a_canceled_build_skips_the_running_and_waiting_steps() {
    let root = Scratch::new();
    running(&root, "b1");
    let canceled = cancel(&root, "b1").unwrap();
    assert_eq!(canceled["state"], "canceled");
    let states: Vec<&str> = canceled["steps"].as_array().unwrap().iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["passed", "skipped", "skipped"]);
    assert!(get(&root, "b1").unwrap()["endedAt"].as_u64().is_some());
}

#[test]
fn a_step_log_reads_the_step_it_names() {
    let root = Scratch::new();
    running(&root, "b2");
    fs::create_dir_all(log_path(&root, "b2", 1).parent().unwrap()).unwrap();
    fs::write(log_path(&root, "b2", 1), "one\ntwo\n").unwrap();
    assert_eq!(step_log(&root, "b2", "1").unwrap(), "one\ntwo\n");
    assert_eq!(step_log(&root, "b2", "0").unwrap(), "");
    assert!(step_log(&root, "b2", "9").is_err());
    assert!(step_log(&root, "b2", "x").is_err());
}

#[test]
fn a_build_left_running_is_failed_as_interrupted() {
    let root = Scratch::new();
    running(&root, "b3");
    recover(&root).unwrap();
    let saved = get(&root, "b3").unwrap();
    assert_eq!(saved["state"], "failed");
    assert_eq!(saved["reason"], "interrupted");
    let states: Vec<&str> = saved["steps"].as_array().unwrap().iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["passed", "failed", "skipped"]);
}

#[test]
fn a_cancel_while_a_step_starts_stops_its_process() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new();
    let id = "cancel-start";
    let pid_file = root.join("cancel-start.pid");
    let pid_arg = pid_file.display().to_string();
    save(
        &root,
        &json!({
            "id": id,
            "state": "running",
            "startedAt": 1,
            "endedAt": Value::Null,
            "steps": [{
                "project": ".",
                "name": "slow",
                "command": ["sh", "-c", "echo $$ > \"$1\"; exec sleep 30", "sh", pid_arg],
                "timeout": 60,
                "state": "running",
                "exitCode": Value::Null,
                "duration": 0
            }]
        }),
    )
    .unwrap();
    RUNS.lock().unwrap().insert(id.to_string(), Arc::new(Mutex::new(None)));

    let top = root.to_path_buf();
    let cancel_id = id.to_string();
    let cancel = std::thread::spawn(move || cancel(&top, &cancel_id));
    let end = Instant::now() + Duration::from_secs(2);
    while get(&root, id).unwrap()["state"] != "canceled" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(get(&root, id).unwrap()["state"], "canceled");

    let started = Instant::now();
    let outcome = execute(&root, id, 0).unwrap();
    assert!(outcome.stopped);
    assert!(started.elapsed() < Duration::from_secs(1));

    if let Ok(text) = std::fs::read_to_string(&pid_file) {
        let pid: i32 = text.trim().parse().unwrap();
        let status = std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap();
        assert!(!status.success(), "step process {pid} outlived the cancel");
    }

    RUNS.lock().unwrap().remove(id);
    let canceled = cancel.join().unwrap().unwrap();
    assert_eq!(canceled["state"], "canceled");
}

#[test]
fn cancel_waits_until_the_run_has_stopped() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new();
    let id = "cancel-wait";
    running(&root, id);
    RUNS.lock().unwrap().insert(id.to_string(), Arc::new(Mutex::new(None)));

    let top = root.to_path_buf();
    let cancel_id = id.to_string();
    let cancel = std::thread::spawn(move || cancel(&top, &cancel_id));
    let end = Instant::now() + Duration::from_secs(2);
    while get(&root, id).unwrap()["state"] != "canceled" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(get(&root, id).unwrap()["state"], "canceled");

    std::thread::sleep(Duration::from_millis(50));
    assert!(!cancel.is_finished(), "cancel returned before its run was gone");

    RUNS.lock().unwrap().remove(id);
    let canceled = cancel.join().unwrap().unwrap();
    assert_eq!(canceled["state"], "canceled");
    assert!(!RUNS.lock().unwrap().contains_key(id));
}

#[test]
fn a_start_is_refused_while_a_canceled_build_is_still_stopping() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new();
    let id = "still-stopping";
    RUNS.lock().unwrap().insert(id.to_string(), Arc::new(Mutex::new(None)));

    let result = start(&root, &json!({}));
    RUNS.lock().unwrap().remove(id);

    let e = result.unwrap_err();
    assert!(e.0.starts_with("E7003"), "{}", e.0);
    assert!(e.0.contains("still stopping"), "{}", e.0);
}

#[test]
fn a_failed_step_files_a_task_once() {
    let root = Scratch::new();
    let build = json!({
        "id": "b-task",
        "state": "failed",
        "startedAt": 1,
        "endedAt": 2,
        "steps": [{
            "project": ".",
            "name": "compile",
            "command": ["gleam", "build"],
            "timeout": 5,
            "state": "failed",
            "exitCode": 1,
            "duration": 3,
            "summary": {"step":"compile","command":"gleam build","exitCode":1,"lines":["error: unknown variable"]}
        }]
    });
    save(&root, &build).unwrap();
    fs::create_dir_all(log_path(&root, "b-task", 0).parent().unwrap()).unwrap();
    fs::write(log_path(&root, "b-task", 0), "first error\nsecond line\n").unwrap();

    assert_eq!(file_task(&root, "b-task", "0", &json!({})).unwrap_err().0, "E7001 body must be text");
    let body = "### Build failure\n\n**. · compile**\n\nExit code 1\n\nCommand: `gleam build`\n\n- error: unknown variable\n\n[Open the build](/delivery?build=b-task)\n";
    let task = file_task(&root, "b-task", "0", &json!({"body": body})).unwrap();
    let id = task["id"].as_str().unwrap();
    assert_eq!(task["title"], "build: . compile fails");
    assert_eq!(task["scope"], json!(["."]));
    assert_eq!(task["links"], json!([{"name":"Build","url":"/delivery?build=b-task"}]));
    assert_eq!(task["notes"][0]["text"], "first error\nsecond line\n");
    assert!(task["body"].as_str().unwrap().contains("error: unknown variable"));
    assert!(task["body"].as_str().unwrap().contains("Command: `gleam build`"));
    assert!(task["body"].as_str().unwrap().contains("/delivery?build=b-task"));
    assert_eq!(get(&root, "b-task").unwrap()["steps"][0]["task"], id);

    let again = file_task(&root, "b-task", "0", &json!({"title":"another"})).unwrap();
    assert_eq!(again["id"], id);
    assert_eq!(crate::board::task_model::all_tasks(&root).unwrap().len(), 1);
}

#[test]
fn a_passed_build_copies_and_hashes_its_outputs() {
    let root = Scratch::new().with(&[
        (
            "solace.toml",
            concat!(
                "[releases]\noutput = \"out\"\n",
                "[build]\noutputs = [\"priv/app.js\"]\n",
                "steps = [{ name = \"build\", run = [\"sh\", \"-c\", \"true\"] }]\n"
            ),
        ),
        ("priv/app.js", "built\n"),
    ]);
    let build = json!({
        "id": "b1",
        "state": "running",
        "steps": [{"project": ".", "name": "build", "command": ["sh", "-c", "true"]}]
    });
    crate::releases::record(&root, &build).unwrap();
    assert_eq!(fs::read_to_string(root.join("out/b1/priv/app.js")).unwrap(), "built\n");
    let release = crate::jsonx::read_opt(&root.join("out/b1/release.json")).unwrap();
    assert_eq!(release["version"], "b1");
    assert_eq!(release["build"], "b1");
    assert_eq!(release["artifacts"][0]["path"], "priv/app.js");
    assert_eq!(release["artifacts"][0]["size"], 6);
    assert_eq!(release["artifacts"][0]["sha256"], crate::sha256(b"built\n"));
    assert_eq!(release["commit"], Value::Null);

    crate::releases::record(
        &root,
        &json!({
            "id": "b2",
            "steps": [{"project": ".", "command": ["sh", "-c", "true"]}]
        }),
    )
    .unwrap();
    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(ignore.lines().filter(|l| l.trim() == "out/").count(), 1);

    let list = crate::releases::listed(&root).unwrap();
    assert_eq!(list["releases"].as_array().unwrap().len(), 2);
    assert_eq!(crate::releases::get(&root, "b1").unwrap()["version"], "b1");
    assert!(crate::releases::get(&root, "missing").is_err());
}

#[test]
fn a_missing_output_fails_the_release_and_names_its_path() {
    let root = Scratch::new().with(&[(
        "solace.toml",
        concat!(
            "[releases]\noutput = \"out\"\n",
            "[build]\noutputs = [\"priv/missing.js\"]\n",
            "steps = [{ name = \"build\", run = [\"sh\", \"-c\", \"true\"] }]\n"
        ),
    )]);
    let e = crate::releases::record(&root, &json!({"id": "b1", "steps": [{"project": ".", "command": ["sh", "-c", "true"]}]})).unwrap_err();
    assert!(e.0.contains("priv/missing.js"), "{}", e.0);
    assert!(!root.join("out/b1").exists());
}

#[test]
fn release_settings_are_written_directly_and_read_back() {
    let root = Scratch::new().with(&[("solace.toml", "watch_limit = 100\n")]);
    assert_eq!(crate::releases::json_settings(&root), json!({"output": "build-output", "read_only": true}));
    let saved = crate::releases::save_settings(&root, &json!({"output": "dist", "read_only": false})).unwrap();
    assert_eq!(saved, json!({"output": "dist", "read_only": false}));
    let text = fs::read_to_string(root.join("solace.toml")).unwrap();
    assert!(text.contains("watch_limit = 100"));
    assert_eq!(text.matches("[releases]").count(), 1);
    assert!(text.contains("output = \"dist\""));
    assert!(text.contains("read_only = false"));
    crate::releases::save_settings(&root, &json!({"output": "dist", "read_only": true})).unwrap();
    let text = fs::read_to_string(root.join("solace.toml")).unwrap();
    assert_eq!(text.matches("[releases]").count(), 1);
    assert!(text.contains("read_only = true"));
    let before = fs::read_to_string(root.join("solace.toml")).unwrap();
    assert!(crate::releases::save_settings(&root, &json!({"output": "dist", "read_only": "no"})).is_err());
    assert_eq!(fs::read_to_string(root.join("solace.toml")).unwrap(), before);
}

#[test]
fn a_release_records_the_tree_before_the_build() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new().with(&[
        (
            "solace.toml",
            concat!(
                "[releases]\noutput = \"out\"\n",
                "[build]\noutputs = [\"priv/app.js\"]\n",
                "steps = [{ name = \"build\", run = [\"sh\", \"-c\", \"printf changed > priv/app.js\"] }]\n"
            ),
        ),
        ("priv/app.js", "built\n"),
        ("priv/app.js.fingerprint", "old\n"),
        (".solace/store.db", ""),
        (".gitignore", ".solace/\nout/\n"),
    ]);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").args(args).current_dir(&*root).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["-c", "user.name=Solace", "-c", "user.email=solace@example.com", "commit", "-qm", "start"]);
    let head = git(&["rev-parse", "HEAD"]).trim().to_string();

    let started = start(&root, &json!({})).unwrap();
    let id = started["id"].as_str().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while get(&root, id).unwrap()["state"] == "running" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(get(&root, id).unwrap()["state"], "passed");
    let release = crate::jsonx::read_opt(&root.join(format!("out/{id}/release.json"))).unwrap();
    assert_eq!(release["dirty"], false);
    assert_eq!(release["commit"], head);
}

static START_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_release_artifact_is_served_only_from_its_record() {
    let root = Scratch::new().with(&[
        (
            "solace.toml",
            concat!(
                "[releases]\noutput = \"out\"\n",
                "[build]\noutputs = [\"priv/app.js\"]\n",
                "steps = [{ name = \"build\", run = [\"sh\", \"-c\", \"true\"] }]\n"
            ),
        ),
        ("priv/app.js", "built\n"),
    ]);
    crate::releases::record(&root, &json!({"id": "b1", "steps": [{"project": ".", "command": ["sh", "-c", "true"]}]})).unwrap();
    let file = crate::releases::artifact(&root, "b1", "priv/app.js").unwrap();
    assert_eq!(fs::read_to_string(file).unwrap(), "built\n");
    for (version, path) in [("b1", "priv/other.js"), ("b1", "../out/b1/priv/app.js"), ("b1", ""), ("missing", "priv/app.js")] {
        let e = crate::releases::artifact(&root, version, path).unwrap_err();
        assert!(e.0.contains("E7023"), "{version} {path}: {}", e.0);
    }
}

#[test]
fn a_failed_build_restores_the_version_files() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/solace.toml", "[build]\nsteps = [{ name = \"fail\", run = [\"sh\", \"-c\", \"false\"] }]\n"),
        ("app/gleam.toml", "name = \"app\"\nversion = \"0.1.0\"\n"),
    ]);
    let started = start(&root, &json!({"version": "0.2.0"})).unwrap();
    assert_eq!(started["version"], "0.2.0");
    assert_eq!(started["versionFiles"][0]["before"], "version = \"0.1.0\"");
    let id = started["id"].as_str().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while get(&root, id).unwrap()["state"] == "running" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    let build = get(&root, id).unwrap();
    assert_eq!(build["state"], "failed");
    assert_eq!(build["versionsRestored"], true);
    assert_eq!(fs::read_to_string(root.join("app/gleam.toml")).unwrap(), "name = \"app\"\nversion = \"0.1.0\"\n");
}

#[test]
fn a_passed_build_writes_one_version_and_names_the_release_for_it() {
    let _serial = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        (
            "app/solace.toml",
            concat!(
                "[build]\n",
                "outputs = [\"out.js\"]\n",
                "steps = [{ name = \"build\", run = [\"sh\", \"-c\", \"grep -q 'version = \\\"0.2.0\\\"' gleam.toml && printf built > out.js\"] }]\n"
            ),
        ),
        ("app/gleam.toml", "name = \"app\"\nversion = \"0.1.0\"\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n"),
    ]);
    let started = start(&root, &json!({"version": "0.2.0"})).unwrap();
    let id = started["id"].as_str().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while get(&root, id).unwrap()["state"] == "running" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(get(&root, id).unwrap()["state"], "passed");
    assert_eq!(fs::read_to_string(root.join("app/gleam.toml")).unwrap(), "name = \"app\"\nversion = \"0.2.0\"\n");
    assert_eq!(fs::read_to_string(root.join("app/Cargo.toml")).unwrap(), "[package]\nname = \"app\"\nversion = \"0.2.0\"\n");
    let release = crate::jsonx::read_opt(&root.join("build-output/0.2.0/release.json")).unwrap();
    assert_eq!(release["version"], "0.2.0");
    assert_eq!(release["build"], id);
    assert_eq!(release["versions"][0]["before"], "version = \"0.1.0\"");
    assert_eq!(release["versions"][0]["after"], "version = \"0.2.0\"");
    let e = start(&root, &json!({"version": "0.2.0"})).unwrap_err();
    assert!(e.0.contains("already has a release"), "{}", e.0);
}

#[test]
fn a_canceled_build_restores_the_version_files() {
    let root = Scratch::new().with(&[("app/gleam.toml", "version = \"0.2.0\"\n")]);
    save(
        &root,
        &json!({
            "id": "b1",
            "state": "running",
            "startedAt": 1,
            "endedAt": Value::Null,
            "versionFiles": [{
                "project": "app",
                "file": "gleam.toml",
                "line": 0,
                "before": "version = \"0.1.0\"",
                "after": "version = \"0.2.0\""
            }],
            "steps": [{
                "project": "app",
                "name": "slow",
                "command": ["sh", "-c", "true"],
                "timeout": 5,
                "state": "running",
                "exitCode": Value::Null,
                "duration": 0
            }]
        }),
    )
    .unwrap();
    let canceled = cancel(&root, "b1").unwrap();
    assert_eq!(canceled["state"], "canceled");
    assert_eq!(fs::read_to_string(root.join("app/gleam.toml")).unwrap(), "version = \"0.1.0\"\n");
    assert_eq!(get(&root, "b1").unwrap()["versionsRestored"], true);
}

#[test]
fn an_interrupted_build_restores_the_version_files() {
    let root = Scratch::new().with(&[("app/gleam.toml", "version = \"0.2.0\"\n")]);
    save(
        &root,
        &json!({
            "id": "b1",
            "state": "running",
            "startedAt": 1,
            "endedAt": Value::Null,
            "versionFiles": [{
                "project": "app",
                "file": "gleam.toml",
                "line": 0,
                "before": "version = \"0.1.0\"",
                "after": "version = \"0.2.0\""
            }],
            "steps": [{
                "project": "app",
                "name": "slow",
                "command": ["sh", "-c", "true"],
                "timeout": 5,
                "state": "running",
                "exitCode": Value::Null,
                "duration": 0
            }]
        }),
    )
    .unwrap();
    recover(&root).unwrap();
    assert_eq!(fs::read_to_string(root.join("app/gleam.toml")).unwrap(), "version = \"0.1.0\"\n");
    let build = get(&root, "b1").unwrap();
    assert_eq!(build["state"], "failed");
    assert_eq!(build["versionsRestored"], true);
}
