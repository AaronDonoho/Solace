use super::*;
use crate::testing::Scratch;
use std::path::PathBuf;

fn files(module: &str, path: &str, ranges: Vec<(usize, usize, &str)>) -> Vec<FileMap> {
    vec![FileMap {
        module: module.into(),
        file: PathBuf::from(path),
        ranges: ranges.into_iter().map(|(a, b, n)| (a, b, n.into())).collect(),
    }]
}

fn span(file: &str, line: usize, primary: bool) -> String {
    format!("{{\"file_name\":\"{file}\",\"line_start\":{line},\"line_end\":{line},\"is_primary\":{primary},\"expansion\":null}}")
}

fn message(level: &str, rendered: &str, spans: &str) -> String {
    format!(
        "{{\"reason\":\"compiler-message\",\"message\":{{\"rendered\":{},\"level\":\"{level}\",\"message\":\"m\",\"spans\":[{spans}]}}}}",
        serde_json::to_string(rendered).unwrap()
    )
}

/// An error is located at its primary span's symbol, with the file position
/// line rewritten (D-158 point 3).
#[test]
fn a_diagnostic_is_located_at_its_symbol() {
    let rendered =
        "error[E0308]: mismatched types\n --> src/lib.rs:3:5\n  |\n3 |     \"two\"\n  |     ^^^^^ expected `i32`, found `&str`\n";
    let diags = diagnostics(
        &message("error", rendered, &span("src/lib.rs", 3, true)),
        &files("crate", "/x/src/lib.rs", vec![(2, 4, "crate::one")]),
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].error);
    assert_eq!(diags[0].symbol.as_deref(), Some("crate::one"));
    assert!(diags[0].text.contains("  at crate::one, line 2"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("--> src/lib.rs:3:5"), "{}", diags[0].text);
}

/// An error inside a macro follows the expansion to the call site.
#[test]
fn a_macro_error_is_located_at_its_call_site() {
    let inner = concat!(
        "{\"file_name\":\"src/lib.rs\",\"line_start\":3,\"line_end\":3,\"is_primary\":true,",
        "\"expansion\":{\"span\":{\"file_name\":\"src/lib.rs\",\"line_start\":10,\"line_end\":10,",
        "\"is_primary\":false,\"expansion\":null},\"macro_decl_name\":\"assert_eq!\"}}",
    );
    let rendered = "error: no method named `nope`\n --> src/lib.rs:3:5\n";
    let diags = diagnostics(
        &message("error", rendered, inner),
        &files("crate", "/x/src/lib.rs", vec![(2, 4, "crate::one"), (8, 12, "crate::two")]),
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].symbol.as_deref(), Some("crate::two"));
    assert!(diags[0].text.contains("at crate::two, line 3"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("crate::one"), "{}", diags[0].text);
}

/// A message reported once per target is shown once, a span-less one is
/// dropped, and a warning is not an error.
#[test]
fn repeats_spanless_messages_and_warnings_are_handled() {
    let files = files("crate", "/x/src/lib.rs", vec![(1, 5, "crate::one")]);
    let error = message("error", "error: boom\n --> src/lib.rs:2:1\n", &span("src/lib.rs", 2, true));
    let warning = message("warning", "warning: unused\n --> src/lib.rs:2:1\n", &span("src/lib.rs", 2, true));
    let spanless = concat!(
        "{\"reason\":\"compiler-message\",\"message\":{\"rendered\":",
        "\"error: aborting due to 1 previous error\\n\",\"level\":\"error\",",
        "\"message\":\"aborting\",\"spans\":[]}}",
    );
    let output = [error.clone(), error, warning, spanless.to_string(), "Checking app v0.1.0".to_string()].join("\n");
    let diags = diagnostics(&output, &files);
    assert_eq!(diags.len(), 2);
    assert!(diags[0].error);
    assert!(!diags[1].error);
}

/// A failed check with no located diagnostic is reported as cargo's own
/// error, whether cargo wrapped it as a spanless diagnostic or printed it
/// (D-158 point 4).
#[test]
fn a_failed_check_without_a_location_reports_cargos_error() {
    let json = message("error", "error: failed to run custom build command for `app`\n--- stderr\nno network\n", "");
    let output = [
        "{\"reason\":\"compiler-artifact\",\"target\":{}}",
        "    Checking app v0.1.0 (/x)",
        json.as_str(),
        "warning: build failed, waiting for other jobs to finish...",
    ]
    .join("\n");
    let text = failure_text(&output);
    assert!(!text.contains("compiler-artifact"), "{text}");
    assert!(!text.contains("Checking app"), "{text}");
    assert!(!text.contains("{\"reason\""), "{text}");
    assert!(text.starts_with("error: failed to run custom build command"), "{text}");
    assert!(text.contains("no network"), "{text}");
}

/// libtest output: the totals across binaries, and each failure's block by
/// libtest name (D-158 point 3).
#[test]
fn libtest_failures_and_totals_are_read() {
    let output = concat!(
        "running 2 tests\ntest board::tests::works ... ok\ntest board::tests::bad ... FAILED\n\n",
        "failures:\n\n---- board::tests::bad stdout ----\n",
        "thread 'board::tests::bad' panicked at src/lib.rs:2:5:\nassertion failed\n\n",
        "failures:\n    board::tests::bad\n\n",
        "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
    );
    assert_eq!(totals(output), (1, 1));
    let failed = failures(output);
    assert_eq!(failed.len(), 1);
    assert!(failed["board::tests::bad"].1.contains("assertion failed"), "{failed:?}");
}

/// `test`'s filters name the tests to run, exactly, after `--`; without one
/// every test runs (D-170).
#[test]
fn test_args_filter_by_libtest_name() {
    let args = test_args(Path::new("/x"), &["tests::a".to_string(), "b".to_string()]);
    assert_eq!(&args[..5], ["test", "--message-format=json", "--offline", "--target-dir", "/x/target"]);
    assert_eq!(&args[5..], ["--", "--exact", "tests::a", "b"]);
    assert_eq!(test_args(Path::new("/x"), &[]).len(), 5, "no filter runs every test");
}

/// The sandbox's arguments (D-158 point 4): no network, the home folder and
/// the project read-only, only `target/` writable, then the command. Building
/// them writes nothing, and a moved Cargo or rustup home is mounted where it is.
#[test]
fn cargo_sandbox_arguments_keep_only_target_writable() {
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    let argv: Vec<String> = ["cargo", "check"].iter().map(|s| s.to_string()).collect();
    let args = crate::migrate::cargo_sandbox_args(
        &argv,
        &dir,
        Some(Path::new("/home/x")),
        Path::new("/home/x/.cargo"),
        Path::new("/home/x/.rustup"),
    )
    .unwrap();
    let line = args.join(" ");
    assert!(line.contains("--die-with-parent --unshare-net --ro-bind / /"), "{line}");
    assert!(line.contains("--tmpfs /home/x"), "{line}");
    assert!(line.contains("--ro-bind-try /home/x/.cargo/registry /home/x/.cargo/registry"), "{line}");
    assert!(line.contains("--ro-bind-try /home/x/.cargo/git /home/x/.cargo/git"), "{line}");
    assert!(line.contains("--ro-bind-try /home/x/.rustup /home/x/.rustup"), "{line}");
    let (root, target) = (dir.display().to_string(), format!("{}/target", dir.display()));
    assert!(line.contains(&format!("--ro-bind {root} {root}")), "{line}");
    assert!(line.contains(&format!("--bind {target} {target}")), "{line}");
    assert!(line.contains(&format!("--bind {root}/Cargo.lock {root}/Cargo.lock")), "{line}");
    assert_eq!(&args[args.len() - 2..], ["cargo", "check"]);
    assert!(!dir.join("Cargo.lock").exists(), "building the arguments writes nothing");
    let moved =
        crate::migrate::cargo_sandbox_args(&argv, &dir, Some(Path::new("/home/x")), Path::new("/opt/cargo"), Path::new("/opt/rustup"))
            .unwrap();
    let line = moved.join(" ");
    assert!(line.contains("--ro-bind-try /opt/cargo/registry /opt/cargo/registry"), "{line}");
    assert!(line.contains("--ro-bind-try /opt/rustup /opt/rustup"), "{line}");
}

/// One item is formatted, a nested one inside its parent's opening at the
/// project's indent, and never a whole file (D-158 point 5, D-168).
#[test]
fn one_item_is_formatted_with_the_projects_rustfmt() {
    if search_path("rustfmt").is_none() {
        return;
    }
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("rustfmt.toml", "max_width = 30\n"),
    ]);
    assert_eq!(format_item(&dir, "pub fn one(  ) -> i32 {   1 }\n", None).unwrap(), "pub fn one() -> i32 {\n    1\n}\n");
    let nested = format_item(&dir, "fn two( &self ) -> i32 { 2 }\n", Some("impl _ {")).unwrap();
    assert_eq!(nested, "impl _ {\n    fn two(&self) -> i32 {\n        2\n    }\n}\n");
}

/// `hard_tabs` in the project's rustfmt settings indents the wrapper's item
/// with tabs (D-158 point 5, D-168).
#[test]
fn a_projects_hard_tabs_setting_is_kept() {
    if search_path("rustfmt").is_none() {
        return;
    }
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("rustfmt.toml", "hard_tabs = true\n"),
    ]);
    let nested = format_item(&dir, "fn two( &self ) -> i32 { 2 }\n", Some("impl _ {")).unwrap();
    assert_eq!(nested, "impl _ {\n\tfn two(&self) -> i32 {\n\t\t2\n\t}\n}\n");
}

/// A type error in a Rust project names the symbol it is in (D-158 point 3).
#[test]
fn a_rust_check_names_the_broken_symbol() {
    if !installed() {
        return;
    }
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub fn one() -> i32 {\n    \"two\"\n}\n"),
    ]);
    crate::import(&dir).unwrap();
    let store = crate::Store::at(&dir).unwrap();
    let message = store.check().unwrap_err().0;
    assert!(message.contains("crate.one"), "{message}");
}

/// In a workspace cargo reports paths relative to the workspace root, so more
/// than one file can end with the reported one; the project's own file is the
/// shallowest match (D-158 point 3).
#[test]
fn a_workspace_members_error_lands_on_the_projects_own_file() {
    let mut maps = files("generated", "/x/gen/src/lib.rs", vec![(1, 5, "generated::one")]);
    maps.extend(files("crate", "/x/src/lib.rs", vec![(1, 5, "crate::one")]));
    let diags = diagnostics(&message("error", "error: boom\n --> src/lib.rs:2:1\n", &span("src/lib.rs", 2, true)), &maps);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].module.as_deref(), Some("crate"), "{}", diags[0].text);
    assert_eq!(diags[0].symbol.as_deref(), Some("crate::one"));
}

/// A failed test's captured output is kept whole, even when it holds a line
/// shaped like a libtest status line (D-158 point 3).
#[test]
fn a_failed_tests_captured_output_is_kept_whole() {
    let output = concat!(
        "running 1 test\ntest m::bad ... FAILED\n\nfailures:\n\n---- m::bad stdout ----\n",
        "test m::ok ... ok\nvalue: 3\n\nfailures:\n    m::bad\n\n",
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
    );
    let failed = failures(output);
    assert_eq!(failed.len(), 1);
    let text = &failed["m::bad"].1;
    assert!(text.contains("test m::ok ... ok"), "{text}");
    assert!(text.contains("value: 3"), "{text}");
}

/// `edition.workspace = true` takes the edition from the workspace root; a
/// manifest without one keeps rustfmt's default (D-158 point 5).
#[test]
fn the_edition_can_come_from_the_workspace() {
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"sub\"]\n\n[workspace.package]\nedition = \"2024\"\n"),
        ("sub/Cargo.toml", "[package]\nname = \"sub\"\nversion = \"0.1.0\"\nedition.workspace = true\n"),
    ]);
    assert_eq!(edition(&dir.join("sub")), "2024");
    assert_eq!(edition(&dir), "2021");
}

/// rustfmt settings are looked for in the project's folder and each one above,
/// the nearest first (D-158 point 5).
#[test]
fn the_rustfmt_settings_can_come_from_a_parent_folder() {
    let dir = Scratch::new().with(&[("rustfmt.toml", "max_width = 30\n"), ("sub/Cargo.toml", "")]);
    assert_eq!(config_path(&dir.join("sub")), Some(dir.join("rustfmt.toml").to_string_lossy().to_string()));
    let nearer = Scratch::new().with(&[("rustfmt.toml", "max_width = 30\n"), ("sub/rustfmt.toml", "max_width = 40\n")]);
    assert_eq!(config_path(&nearer.join("sub")), Some(nearer.join("sub/rustfmt.toml").to_string_lossy().to_string()));
}

/// A failed fetch reports cargo's message and puts Cargo.lock back byte for
/// byte, or takes away the one cargo started (D-176).
#[test]
fn a_failed_fetch_keeps_the_lockfile() {
    if !installed() {
        return;
    }
    let dir = Scratch::new().with(&[
        (".cargo/config.toml", "# A failed fetch is the point; cargo's retries only slow the test.\n[net]\nretry = 0\n"),
        (
            "Cargo.toml",
            concat!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
                "[dependencies]\ndep = { git = \"file:///nonexistent/solace-missing\" }\n",
            ),
        ),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    let message = fetch(&dir).unwrap_err().0;
    assert!(message.contains("cargo fetch failed"), "{message}");
    assert!(message.contains("solace-missing"), "{message}");
    assert!(!dir.join("Cargo.lock").exists(), "a failed first fetch leaves no lock");
    assert!(!dir.join(".solace").exists(), "a failed first fetch leaves no command folder");
    std::fs::write(dir.join("Cargo.lock"), "kept\n").unwrap();
    let message = fetch(&dir).unwrap_err().0;
    assert!(message.contains("cargo fetch failed"), "{message}");
    assert_eq!(std::fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "kept\n");
}

/// A machine without cargo imports a Rust project, and says what it didn't
/// fetch (D-176).
#[test]
fn import_without_cargo_says_dependencies_were_not_fetched() {
    let root = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    let out = crate::import_with(&root, false).unwrap();
    assert!(out.contains("Dependencies not fetched: Needs the Rust toolchain (`cargo`)"), "{out}");
    assert!(root.join(".solace/store.db").exists());
    assert!(!root.join("Cargo.lock").exists(), "nothing fetched");
    let out = crate::import_with(&root, false).unwrap();
    assert!(out.contains("already has a store") && out.contains("Dependencies not fetched"), "{out}");
}

/// D-176: a member's failed fetch puts the workspace's `Cargo.lock` back, not
/// the member's, which cargo doesn't write.
#[test]
fn a_failed_member_fetch_keeps_the_workspace_lockfile() {
    if !installed() {
        return;
    }
    let root = Scratch::new().with(&[
        (".cargo/config.toml", "# A failed fetch is the point; cargo's retries only slow the test.\n[net]\nretry = 0\n"),
        ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n"),
        ("member/Cargo.toml", "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("member/src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
    ]);
    fetch(&root.join("member")).unwrap();
    let before = std::fs::read(root.join("Cargo.lock")).unwrap();
    std::fs::write(
        root.join("member/Cargo.toml"),
        concat!(
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
            "[dependencies]\ndep = { git = \"file:///nonexistent/solace-missing\" }\n",
        ),
    )
    .unwrap();
    let message = fetch(&root.join("member")).unwrap_err().0;
    assert!(message.contains("cargo fetch failed"), "{message}");
    assert!(message.contains("solace-missing"), "{message}");
    assert_eq!(std::fs::read(root.join("Cargo.lock")).unwrap(), before);
    assert!(!root.join("member/Cargo.lock").exists(), "the member has no lock of its own");
}

/// D-176: a member's fetch and the root's reopen lock the same file, since
/// both change the workspace's `Cargo.lock`.
#[test]
fn a_member_fetch_and_a_root_reopen_share_the_fetch_lock() {
    if !installed() {
        return;
    }
    let root = Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n"),
        ("member/Cargo.toml", "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("member/src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
        (".solace/store.db", ""),
    ]);
    assert_eq!(fetch_lock_dir(&root).unwrap(), root.to_path_buf());
    assert_eq!(fetch_lock_dir(&root.join("member")).unwrap(), root.to_path_buf(), "a member uses the root's lock");
    let held = fetch_lock(&root, true).unwrap().expect("the root has a store");
    let path = root.join(".solace/fetch.lock");
    assert!(crate::try_lock_file(&path).unwrap().is_none(), "the root's reopen holds the member's fetch lock");
    drop(held);
    assert!(fetch_lock(&root.join("member"), false).unwrap().is_some(), "a member's first import shares the root's lock");
}

/// D-176: when Cargo's workspace is above the Solace root, the fetch lock is
/// still the Solace root's, so its reopen and a member fetch exclude each other.
#[test]
fn a_fetch_locks_its_solace_root_above_a_cargo_workspace_member() {
    let workspace = Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n"),
        ("member/Cargo.toml", "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("member/src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
        ("member/.solace/store.db", ""),
    ]);
    assert_eq!(fetch_lock_dir(&workspace.join("member")).unwrap(), workspace.join("member"));
}

/// D-176: a fetch waits for the fetch lock a reopen holds, so the two never
/// change `Cargo.lock` at once.
#[test]
fn a_fetch_waits_for_the_fetch_lock() {
    if !installed() {
        return;
    }
    let root = Scratch::new().with(&[
        (".cargo/config.toml", "# A failed fetch is the point; cargo's retries only slow the test.\n[net]\nretry = 0\n"),
        (
            "Cargo.toml",
            concat!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
                "[dependencies]\ndep = { git = \"file:///nonexistent/solace-missing\" }\n",
            ),
        ),
        ("src/lib.rs", "pub fn one() -> i32 { 1 }\n"),
        (".solace/store.db", ""),
    ]);
    let path = fetch_lock_dir(&root).unwrap().join(".solace/fetch.lock");
    let held = crate::lock_file(&path).unwrap();
    let folder = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(fetch(&folder).is_err());
    });
    assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "fetch ran while the lock was held");
    drop(held);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(60)).ok(), Some(true), "the fetch failed once the lock was free");
    worker.join().unwrap();
}

/// A second format of the same item and settings reuses the first result.
#[test]
fn a_formatted_item_is_reused() {
    if search_path("rustfmt").is_none() {
        return;
    }
    let dir = Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")]);
    let text = "fn cached( ) -> i32 { 3 }\n";
    let one = format_item(&dir, text, None).unwrap();
    assert_eq!(format_item(&dir, text, None).unwrap(), one);
    let key = (text.to_string(), edition(&dir), None);
    assert!(FORMATTED.lock().unwrap().contains_key(&key));
    FORMATTED.lock().unwrap().insert(key, "sentinel\n".to_string());
    assert_eq!(format_item(&dir, text, None).unwrap(), "sentinel\n");
}

/// A whole module's text is formatted with the project's settings, and neither
/// its written file nor its child modules are touched (D-180).
#[test]
fn a_whole_module_is_formatted_without_its_children() {
    if search_path("rustfmt").is_none() {
        return;
    }
    let dir = Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("rustfmt.toml", "hard_tabs = true\n"),
        ("src/lib.rs", "mod b;\nmod a;\n\npub fn one( ) -> i32 { 1 }\n"),
        ("src/a.rs", "pub fn a( ) -> i32 { 2 }\n"),
        ("src/b.rs", "pub fn b( ) -> i32 { 3 }\n"),
    ]);
    let lib = dir.join("src/lib.rs");
    let child = dir.join("src/a.rs");
    let source = fs::read_to_string(&lib).unwrap();
    let out = format_module(&dir, &source).unwrap();
    assert_eq!(out, "mod b;\nmod a;\n\npub fn one() -> i32 {\n\t1\n}\n");
    assert_eq!(fs::read_to_string(&lib).unwrap(), source);
    assert_eq!(fs::read_to_string(&child).unwrap(), "pub fn a( ) -> i32 { 2 }\n");
}

/// `max_width` reads the nearest rustfmt settings and keeps rustfmt's default
/// 100 without one.
#[test]
fn max_width_comes_from_the_projects_rustfmt_settings_or_defaults_to_100() {
    let none = Scratch::new().with(&[("Cargo.toml", "")]);
    assert_eq!(max_width(&none), 100);
    let other = Scratch::new().with(&[("Cargo.toml", ""), ("rustfmt.toml", "edition = \"2021\"\n")]);
    assert_eq!(max_width(&other), 100);
    let set = Scratch::new().with(&[("Cargo.toml", ""), ("rustfmt.toml", "max_width = 30\n")]);
    assert_eq!(max_width(&set), 30);
    let parent = Scratch::new().with(&[("rustfmt.toml", "max_width = 40\n"), ("sub/Cargo.toml", "")]);
    assert_eq!(max_width(&parent.join("sub")), 40);
}

/// A whole module's syntax error names its line and leaves the project's files
/// alone (D-180).
#[test]
fn a_whole_module_syntax_error_names_a_line() {
    if search_path("rustfmt").is_none() {
        return;
    }
    let dir = Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")]);
    let e = format_module(&dir, "pub fn two( {} \n").unwrap_err().0;
    assert!(e.contains("E0100 syntax error at line") && !e.contains("format-"), "{e}");
}

/// One `cargo check --message-format=json` line for a private function.
const PRIVATE_CHECK: &str = concat!(
    r#"{"reason":"compiler-message","message":{"rendered":"error[E0603]: function `helper` is private\n --> src/lib.rs:5:9\n  |\n"#,
    r#"5 |     crate::helper();\n  |            ^^^^^^ private function\n","level":"error","message":"function `helper` is private","#,
    r#""spans":[{"file_name":"src/lib.rs","line_start":5,"line_end":5,"is_primary":true,"expansion":null}]}}"#,
);

/// One `cargo check --message-format=json` line for a missing method.
const MISSING_METHOD_CHECK: &str = concat!(
    r#"{"reason":"compiler-message","message":{"rendered":"error[E0599]: no method named `open` found for struct `Store`"#,
    r#" in the current scope\n --> src/lib.rs:6:9\n  |\n6 |     store.open();\n"#,
    r#"  |           ^^^^ method not found in `Store`\n","level":"error","#,
    r#""message":"no method named `open` found for struct `Store` in the current scope","#,
    r#""spans":[{"file_name":"src/lib.rs","line_start":6,"line_end":6,"is_primary":true,"expansion":null}]}}"#,
);

/// Captured check output gives the title and the two names a shared cause groups by.
#[test]
fn a_grouped_error_title_comes_from_cargos_output() {
    let output = format!("{PRIVATE_CHECK}\n{MISSING_METHOD_CHECK}");
    let diags = diagnostics(&output, &files("crate", "/x/src/lib.rs", vec![(1, 10, "crate::one")]));
    assert_eq!(diags.len(), 2);
    let causes: Vec<String> = diags.iter().map(|d| crate::error_cause(d).unwrap().1).collect();
    assert_eq!(causes, ["`helper` is private", "`open` is missing on `Store`"]);
}

/// A probe's own lines replace the scratch file's numbers, and a note in project code counts from its symbol.
#[test]
fn probe_lines_replace_scratch_numbers_and_notes_keep_their_symbol() {
    let rendered = concat!(
        "error[E0308]: mismatched types\n",
        "  --> src/probe.rs:15:9\n",
        "   |\n",
        "15 |         let x: i32 = \"nope\";\n",
        "   |         ^^^^^^^^^^^^^^\n",
        "   |\n",
        "note: function defined here\n",
        "  --> src/probe.rs:8:4\n",
        "   |\n",
        " 8 | pub fn probe_code(text: &str) -> String {\n",
        "   |    ^^^^^^^^^^\n",
    );
    let spans = format!("{},{}", span("src/probe.rs", 15, true), span("src/probe.rs", 8, false));
    let diags = diagnostics(
        &message("error", rendered, &spans),
        &files("crate", "/x/src/probe.rs", vec![(8, 9, "crate.probe_code"), (15, 16, "probe")]),
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].text.contains("  at probe, line 1"), "{}", diags[0].text);
    assert!(diags[0].text.contains("  at crate.probe_code, line 1"), "{}", diags[0].text);
    assert!(diags[0].text.contains("\n 1 | let x: i32 = \"nope\";"), "{}", diags[0].text);
    assert!(diags[0].text.contains("\n   | ^^^^^^^^^^^^^^"), "{}", diags[0].text);
    assert!(diags[0].text.contains("\n 1 | pub fn probe_code"), "{}", diags[0].text);
    assert!(diags[0].text.contains("\n   |    ^^^^^^^^^^"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("|         ^"), "{}", diags[0].text);
    assert!(!diags[0].text.contains(" 8 |"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("15 |"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("--> "), "{}", diags[0].text);
}

/// An error on the generated wrapper names the end of the probe without a scratch gutter.
#[test]
fn probe_wrapper_lines_name_the_end_of_the_text() {
    let rendered =
        concat!("error: this file contains an unclosed delimiter\n", "  --> src/probe.rs:20:1\n", "   |\n", "20 | }\n", "   | ^\n",);
    let diags = diagnostics(
        &message("error", rendered, &span("src/probe.rs", 20, true)),
        &files("crate", "/x/src/probe.rs", vec![(15, 16, "probe"), (17, 20, crate::PROBE_END_RANGE)]),
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].text.contains("  at probe, end of text"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("imports"), "{}", diags[0].text);
    assert!(!diags[0].text.contains("20 |"), "{}", diags[0].text);
}

/// A rendered source marker needs both its line and column.
#[test]
fn a_rendered_location_needs_a_column() {
    assert_eq!(source_location(" --> src/lib.rs:3:5"), Some(("src/lib.rs", 3)));
    assert_eq!(source_location(" --> src/lib.rs:3"), None);
}

/// A multi-line span keeps its markers and carets aligned after the wrapper indent goes.
#[test]
fn probe_multiline_markers_lose_the_wrapper_indent() {
    let rendered = concat!(
        "error[E0308]: mismatched types\n",
        "  --> src/probe.rs:15:9\n",
        "   |\n",
        "15 |           let x: i32 = (\n",
        "   |  ________________---___^\n",
        "   | |                |\n",
        "   | |                expected due to this\n",
        "16 | |             1,\n",
        "17 | |         );\n",
        "   | |_________^ expected `i32`, found `({integer},)`\n",
    );
    let spans = span("src/probe.rs", 15, true);
    let diags = diagnostics(&message("error", rendered, &spans), &files("crate", "/x/src/probe.rs", vec![(15, 17, "probe")]));
    assert_eq!(diags.len(), 1);
    let text = &diags[0].text;
    assert!(text.contains("\n 1 |   let x: i32 = ("), "{text}");
    assert!(text.contains("\n   |  ________---___^"), "{text}");
    assert!(text.contains("\n 2 | |     1,"), "{text}");
    assert!(text.contains("\n 3 | | );"), "{text}");
    assert!(text.contains("\n   | |_^"), "{text}");
    assert!(!text.contains("|         "), "{text}");
}

/// A probe line with no wrapper indent keeps the spaces its code quotes.
#[test]
fn a_probe_line_without_indent_keeps_quoted_spaces() {
    let rendered = concat!(
        "error[E0308]: mismatched types\n",
        "  --> src/probe.rs:15:9\n",
        "   |\n",
        "15 | let s = \"        x\";\n",
        "   | ^^^^^^^^^^^^^^^^^^^^\n",
    );
    let diags = diagnostics(
        &message("error", rendered, &span("src/probe.rs", 15, true)),
        &files("crate", "/x/src/probe.rs", vec![(15, 16, "probe")]),
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].text.contains("\n 1 | let s = \"        x\";"), "{}", diags[0].text);
}
