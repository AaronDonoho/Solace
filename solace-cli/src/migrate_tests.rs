use super::*;

#[test]
fn explains_layout_differences() {
    assert_eq!(explain("a\r\nb\r\n", "a\nb\n"), vec!["Windows line endings"]);
    assert_eq!(
        explain("fn a() {}\n\n\n\nfn b() {}", "fn a() {}\n\nfn b() {}\n"),
        vec!["the newline at the end of the file", "blank lines between definitions"]
    );
}

#[test]
fn finds_lines_inside_strings() {
    let src = "pub fn a() {\n  \"one\n\npub fn b() {}\n\\\"still\"\n}\n// \"not a string\npub fn c() { \"x\" }\n";
    assert_eq!(string_lines(src), vec![2, 3, 4]);
}

#[test]
fn a_definition_inside_a_string_is_not_layout() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(SCHEMA).unwrap();
    let src = "pub fn a() {\n  \"\npub fn b() {}\n\"\n}\n";
    let m = gleam::split(src);
    db.execute("INSERT INTO modules(path, dir, header) VALUES ('app', 'src', ?1)", [&m.header]).unwrap();
    for (i, it) in m.items.iter().enumerate() {
        db.execute(
            "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, 'app', ?2, 'fn', ?3, ?4, '')",
            params![i as i64, it.name, it.text, i as i64],
        )
        .unwrap();
    }
    assert_eq!(
        changed_pieces(&gleam::Gleam, &db, "src/app.gleam", src).unwrap(),
        vec!["a text value spanning several lines, holding a line Solace would take for a new definition"]
    );
    assert!(!changed_pieces(&gleam::Gleam, &db, "src/app.gleam", "pub fn a() {\n  \"\n").unwrap().is_empty());
}

#[test]
fn a_failure_the_original_had_is_not_blamed() {
    let base = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.old_test"]}},"smoke":[{"ok":true}]});
    let now = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.old_test","app_test.new_test"],"output":""}},
        "smoke":[{"command":"true","ok":false,"output":""}]});
    let items = behavior(&base, &now, true);
    let status = |t: &str| items.iter().find(|i| i["text"].as_str().unwrap().contains(t)).unwrap()["status"].clone();
    assert_eq!(status("old_test"), "not checked");
    assert_eq!(status("new_test"), "failed");
    assert_eq!(status("check command"), "failed");
    assert_eq!(check(TESTS, "", items)["status"], "failed");
}

#[test]
fn tests_are_reported_when_only_the_migration_builds() {
    let base = json!({"projects":{".":{"build":false,"test":false,"failing":[]}},"smoke":[]});
    let now = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.t"],"output":""}},"smoke":[]});
    let items = behavior(&base, &now, false);
    assert!(items.iter().any(|i| i["text"].as_str().unwrap().contains("app_test.t fails; your project didn't build before the migration")));
    let none = json!({"projects":{".":{"build":false,"test":false,"failing":[],"output":""}},"smoke":[]});
    assert_eq!(warnings(&base, &none).len(), 1);
}

#[test]
fn relative_paths() {
    assert_eq!(relative(Path::new("/a/b"), Path::new("/a/c/d")), PathBuf::from("../c/d"));
    assert_eq!(relative(Path::new("/a"), Path::new("/a/x")), PathBuf::from("x"));
}

fn project_with_lock() -> (crate::testing::Project, fs::File) {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let held = lock(&p.root).unwrap();
    (p, held)
}

#[test]
fn hold_refuses_a_lock_held_for_the_whole_wait() {
    let (p, _held) = project_with_lock();
    let e = hold(&p.root).unwrap_err();
    assert!(e.0.starts_with("E7003"), "{}", e.0);
    assert!(e.0.contains("is busy with another command"), "{}", e.0);
}

#[test]
fn hold_takes_a_lock_released_during_the_wait() {
    let (p, held) = project_with_lock();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(held);
    });
    assert!(hold_within(&p.root, Duration::from_secs(10)).is_ok());
}

#[test]
fn nothing_to_migrate_says_why() {
    let empty = crate::testing::Scratch::new();
    let none = status(&empty).unwrap();
    assert_eq!(none["state"], "none");
    assert_eq!(none["reason"], nothing_reason(false));
    let root = crate::testing::Scratch::new().with(&[("solace.toml", "projects = []\n")]);
    let none = status(&root).unwrap();
    assert_eq!(none["state"], "none");
    assert_eq!(none["reason"], nothing_reason(true));
}

/// A test run from a task must not see the agent's task context (D-71): the
/// same five variables the `rust-*` commands unset with `env -u`.
#[test]
fn a_sandboxed_command_drops_the_agents_task_context() {
    let dir =
        crate::testing::Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")]);
    let argv: Vec<String> = ["cargo", "check"].iter().map(|s| s.to_string()).collect();
    let command = cargo_sandboxed(&argv, &dir).unwrap();
    let mut removed = 0;
    for (name, value) in command.get_envs() {
        if value.is_none() {
            for key in TASK_ENV {
                if name == std::ffi::OsStr::new(key) {
                    removed += 1;
                }
            }
        }
    }
    assert_eq!(removed, TASK_ENV.len());
}

/// A sandbox mounts only the folders a project declares in `[sandbox] read`,
/// so a member's build script cannot read the whole Solace root (D-158 point 4).
#[test]
fn a_sandbox_in_a_solace_root_mounts_only_the_declared_reads() {
    let top = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\"]\n"),
        ("app/solace.toml", "[sandbox]\nread = [\"../fixtures\", \"/opt/shared\"]\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    ]);
    let root = top.join("app");
    let argv: Vec<String> = ["cargo", "check"].iter().map(|s| s.to_string()).collect();
    let args =
        cargo_sandbox_args(&argv, &root, Some(Path::new("/home/x")), Path::new("/home/x/.cargo"), Path::new("/home/x/.rustup"), None, None)
            .unwrap();
    let line = args.join(" ");
    let top = top.display().to_string();
    assert!(line.contains(&format!("--ro-bind-try {top}/fixtures {top}/fixtures")), "{line}");
    assert!(line.contains("--ro-bind-try /opt/shared /opt/shared"), "{line}");
    assert!(line.contains(&format!("--ro-bind {top}/app {top}/app")), "{line}");
    assert!(line.contains(&format!("--bind {top}/app/target {top}/app/target")), "{line}");
    assert!(!line.contains(&format!("--ro-bind {top} {top}")), "the whole root is not mounted: {line}");
}

#[test]
fn a_sandbox_with_unreadable_settings_is_refused() {
    let (root, _store) = crate::testing::rust_project("pub fn one() -> i32 { 1 }\n");
    fs::write(root.join("solace.toml"), "[sandbox]\nread = 3\n").unwrap();
    let argv: Vec<String> = ["cargo", "check"].iter().map(|s| s.to_string()).collect();
    let e = cargo_sandbox_args(&argv, &root, None, Path::new("/home/x/.cargo"), Path::new("/home/x/.rustup"), None, None).unwrap_err();
    assert!(e.0.starts_with("E6020"), "{}", e.0);
}

#[test]
fn a_declared_command_drops_the_agents_task_context() {
    let mut command = Command::new("true");
    drop_task_context(&mut command);
    let mut removed = 0;
    for (name, value) in command.get_envs() {
        if value.is_none() {
            for key in TASK_ENV {
                if name == std::ffi::OsStr::new(key) {
                    removed += 1;
                }
            }
        }
    }
    assert_eq!(removed, TASK_ENV.len());
}

/// A folder a project declares in `[sandbox] write` is writable, bound after
/// the read-only project so it wins; nothing else in the project is (D-158 point 4).
#[test]
fn a_sandbox_makes_only_the_declared_writes_writable() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "[sandbox]\nwrite = [\"gen\"]\n"),
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    ]);
    let argv: Vec<String> = ["cargo", "test"].iter().map(|s| s.to_string()).collect();
    let args =
        cargo_sandbox_args(&argv, &root, Some(Path::new("/home/x")), Path::new("/home/x/.cargo"), Path::new("/home/x/.rustup"), None, None)
            .unwrap();
    let dir = root.display().to_string();
    let line = args.join(" ");
    let project = line.find(&format!("--ro-bind {dir} {dir}")).expect("the project is read-only");
    let declared = line.find(&format!("--bind {dir}/gen {dir}/gen")).expect("gen is writable");
    assert!(project < declared, "{line}");
    let writable: Vec<&String> = args.windows(2).filter(|w| w[0] == "--bind").map(|w| &w[1]).collect();
    assert_eq!(writable, [&format!("{dir}/target"), &format!("{dir}/gen"), &format!("{dir}/Cargo.lock")], "{line}");
    assert!(!root.join("gen").exists(), "building the arguments writes nothing");
}

/// Solace's own Gleam and Erlang (D-264) are the only tools of the home folder
/// a sandbox shows, read-only after the home folder is emptied: never the
/// folders that hold them, nor Cargo's home with its login token, nor `~/.ssh`.
#[test]
fn a_sandbox_shows_only_the_tools_folder_of_the_home_folder() {
    let (root, _store) = crate::testing::rust_project("pub fn one() -> i32 { 1 }\n");
    let argv: Vec<String> = ["cargo", "test"].iter().map(|s| s.to_string()).collect();
    let tools = Path::new("/home/x/.local/share/solace/tools");
    let args = cargo_sandbox_args(
        &argv,
        &root,
        Some(Path::new("/home/x")),
        Path::new("/home/x/.cargo"),
        Path::new("/home/x/.rustup"),
        Some(Path::new("/home/x/.cargo/bin")),
        Some(tools),
    )
    .unwrap();
    let line = args.join(" ");
    let emptied = line.find("--tmpfs /home/x ").unwrap();
    let shown =
        line.find("--ro-bind-try /home/x/.local/share/solace/tools /home/x/.local/share/solace/tools").expect("the tools are shown");
    assert!(emptied < shown, "{line}");
    let bound: Vec<&str> = args.windows(2).filter(|w| w[0].contains("bind")).map(|w| w[1].as_str()).collect();
    for hidden in ["/home/x", "/home/x/.cargo", "/home/x/.local", "/home/x/.local/share", "/home/x/.local/share/solace", "/home/x/.ssh"] {
        assert!(!bound.contains(&hidden), "{hidden} is bound: {line}");
    }
    assert!(!line.contains("credentials"), "{line}");
}

/// A program found in the home folder itself, or in a folder above it, would
/// show the whole home folder: it is left out.
#[test]
fn a_folder_holding_the_home_folder_is_never_shown() {
    let (root, _store) = crate::testing::rust_project("pub fn one() -> i32 { 1 }\n");
    let argv: Vec<String> = ["cargo", "test"].iter().map(|s| s.to_string()).collect();
    for (program, tools) in [("/home/x", "/home"), ("/", "relative/tools")] {
        let args = cargo_sandbox_args(
            &argv,
            &root,
            Some(Path::new("/home/x")),
            Path::new("/home/x/.cargo"),
            Path::new("/home/x/.rustup"),
            Some(Path::new(program)),
            Some(Path::new(tools)),
        )
        .unwrap();
        let bound: Vec<&str> = args.windows(2).filter(|w| w[0] == "--ro-bind-try").map(|w| w[1].as_str()).collect();
        assert!(!bound.contains(&program) && !bound.contains(&tools), "{args:?}");
    }
}

/// Inside a real sandbox, Cargo's login token and `~/.ssh` can't be read, and
/// Solace's tools folder can.
#[test]
fn secrets_in_the_home_folder_are_unreadable_in_the_sandbox() {
    if !sandbox_works() {
        return;
    }
    let home = crate::testing::Scratch::new().with(&[
        (".cargo/credentials.toml", "[registry]\ntoken = \"secret\"\n"),
        (".ssh/id_ed25519", "secret\n"),
        (".local/share/solace/tools/gleam-1/gleam", "tool\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("app/Cargo.lock", ""),
    ]);
    let root = home.join("app");
    fs::create_dir_all(root.join("target")).unwrap();
    let tools = home.join(".local/share/solace/tools");
    let read = |file: &Path| {
        let argv: Vec<String> = vec!["cat".into(), file.to_string_lossy().into_owned()];
        let args = cargo_sandbox_args(&argv, &root, Some(&home), &home.join(".cargo"), &home.join(".rustup"), None, Some(&tools)).unwrap();
        Command::new("bwrap").args(args).output().unwrap()
    };
    for secret in [".cargo/credentials.toml", ".ssh/id_ed25519"] {
        let out = read(&home.join(secret));
        assert!(!out.status.success() && out.stdout.is_empty(), "{secret} was read: {}", String::from_utf8_lossy(&out.stdout));
    }
    let tool = read(&tools.join("gleam-1/gleam"));
    assert_eq!(String::from_utf8_lossy(&tool.stdout), "tool\n", "{}", String::from_utf8_lossy(&tool.stderr));
}
