use super::*;
use crate::testing::Scratch;
use std::fs;

/// A `tool` in `root`'s `bin/` that prints `version`; the folder it is in.
fn fake_tool(root: &Path, tool: &str, version: &str) -> std::path::PathBuf {
    let path = root.join("bin").join(tool);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("#!/bin/sh\necho {version}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
    }
    root.join("bin")
}

fn settings(text: &str) -> toml::Value {
    crate::lockdown::parse_settings(text).unwrap()
}

#[test]
fn test_tools_is_a_list_of_tools_with_optional_minimums() {
    let ok = settings("test_tools = [\"gleam\", \"node >= 19\", \"bun>=1.2.3\"]\n");
    assert_eq!(
        declared(&ok).unwrap(),
        vec![
            Need { tool: "gleam".into(), minimum: None, tests: vec![] },
            Need { tool: "node".into(), minimum: Some("19".into()), tests: vec![] },
            Need { tool: "bun".into(), minimum: Some("1.2.3".into()), tests: vec![] },
        ]
    );
    for bad in [
        "test_tools = \"gleam\"\n",
        "test_tools = [1]\n",
        "test_tools = [\"\"]\n",
        "test_tools = [\"node >= nineteen\"]\n",
        "test_tools = [\"node >=\"]\n",
        "test_tools = [\"two words\"]\n",
    ] {
        let err = crate::lockdown::parse_settings(bad).unwrap_err().0;
        assert!(err.starts_with("E6020") && err.contains("test_tools"), "{bad}: {err}");
    }
}

#[test]
fn a_gleam_project_on_javascript_needs_its_runtime() {
    let none = settings("");
    let erlang = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n")]);
    assert_eq!(needs(&erlang, &none).unwrap(), vec![]);
    let node = Scratch::new().with(&[("gleam.toml", "name = \"app\"\ntarget = \"javascript\"\n")]);
    assert_eq!(needs(&node, &none).unwrap(), vec![Need { tool: "node".into(), minimum: None, tests: vec![] }]);
    let bun = Scratch::new().with(&[("gleam.toml", "name = \"app\"\ntarget = \"javascript\"\n\n[javascript]\nruntime = \"bun\"\n")]);
    assert_eq!(needs(&bun, &none).unwrap(), vec![Need { tool: "bun".into(), minimum: None, tests: vec![] }]);
    let declared = settings("test_tools = [\"node >= 19\"]\n");
    assert_eq!(needs(&node, &declared).unwrap(), vec![Need { tool: "node".into(), minimum: Some("19".into()), tests: vec![] }]);
}

#[test]
fn a_tool_at_or_past_its_minimum_passes() {
    let root = Scratch::new().with(&[(".solace/.keep", ""), ("solace.toml", "test_tools = [\"node >= 19\"]\n")]);
    let bin = fake_tool(&root, "node", "v19.0.0");
    crate::language::with_tool_path(std::slice::from_ref(&bin), || assert!(require(&root, "", &[]).is_ok()));
    fake_tool(&root, "node", "v22.4.1");
    crate::language::with_tool_path(std::slice::from_ref(&bin), || assert!(require(&root, "", &[]).is_ok()));
    fake_tool(&root, "node", "v18.20.3");
    let err = crate::language::with_tool_path(&[bin], || require(&root, "app", &[]).unwrap_err().0);
    assert_eq!(err, "E6013 Tests not run in app: node 18.20.3 is older than the 19 these tests need");
}

#[test]
fn an_unreadable_version_is_named() {
    let root = Scratch::new().with(&[(".solace/.keep", ""), ("solace.toml", "test_tools = [\"node >= 19\"]\n")]);
    let bin = fake_tool(&root, "node", "unknown");
    let err = crate::language::with_tool_path(&[bin], || require(&root, "", &[]).unwrap_err().0);
    assert_eq!(err, "E6013 Tests not run: couldn't read `node --version`; these tests need 19 or newer");
}

/// `test` in a Rust project that declares Gleam stops before cargo runs
/// when Gleam isn't installed.
#[test]
fn a_missing_declared_tool_stops_the_run_in_one_line() {
    let (root, store) = crate::testing::rust_project(ONE_WITH_A_TEST);
    fs::write(root.join("solace.toml"), "test_tools = [\"gleam\"]\n").unwrap();
    let tools = root.join("tools");
    let run = |system| {
        crate::language::with_tool_path(&[], || {
            crate::tools::with_folder(&tools, || crate::tools::with_system(system, || store.test(&["--all"]).unwrap_err().0))
        })
    };
    assert_eq!(
        run(Some("x86_64-linux-gnu")),
        "E6013 Tests not run: `gleam` is not installed; install it with Install on Solace's checklist"
    );
    assert_eq!(run(None), "E6013 Tests not run: `gleam` is not installed (not on PATH)");
}

/// `test` in a Gleam project on Node stops before `gleam test` runs when
/// Node is older than the project declares.
#[test]
fn an_old_runtime_stops_a_gleam_run() {
    let store = Scratch::new()
        .with(&[
            ("gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\ntarget = \"javascript\"\n"),
            ("src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
            ("test/app_test.gleam", "import app\n\npub fn one_test() {\n  assert app.one() == 1\n}\n"),
        ])
        .import();
    fs::write(store.root.join("solace.toml"), "test_tools = [\"node >= 19\"]\n").unwrap();
    let bin = fake_tool(&store.root, "node", "v18.2.0");
    let err = crate::language::with_tool_path(&[bin], || store.test(&[]).unwrap_err().0);
    assert_eq!(err, "E6013 Tests not run: node 18.2.0 is older than the 19 these tests need");
}

/// `tools` in a `[commands]` entry: an old one stops `run`, a new enough
/// one passes, and a malformed list is refused with the command's name.
#[test]
fn a_command_checks_the_tools_it_lists() {
    let root = Scratch::new().with(&[(".solace/.keep", "")]);
    let v = settings("[commands]\nsuite = { run = [\"true\"], tools = [\"python3 >= 3.10\"] }\n");
    let command = &v["commands"]["suite"];
    let bin = fake_tool(&root, "python3", "Python 3.9.2");
    let err = crate::language::with_tool_path(std::slice::from_ref(&bin), || require_command(&root, command).unwrap_err().0);
    assert_eq!(err, "E6013 Not run: python3 3.9.2 is older than the 3.10 these tests need");
    fake_tool(&root, "python3", "Python 3.12.1");
    crate::language::with_tool_path(&[bin], || assert!(require_command(&root, command).is_ok()));
    let bad = crate::lockdown::parse_settings("[commands]\nsuite = { run = [\"true\"], tools = [\"python3 >= x\"] }\n");
    assert!(bad.unwrap_err().0.contains("commands.suite.tools"));
}

#[test]
fn a_tool_can_name_the_tests_that_need_it() {
    let ok = settings("test_tools = [\"gleam\", { tool = \"node >= 19\", tests = [\"workspace::\", \"versions::\"] }]\n");
    assert_eq!(
        declared(&ok).unwrap(),
        vec![
            Need { tool: "gleam".into(), minimum: None, tests: vec![] },
            Need { tool: "node".into(), minimum: Some("19".into()), tests: vec!["workspace::".into(), "versions::".into()] },
        ]
    );
    for bad in [
        "test_tools = [{ tool = \"node\" }]\n",
        "test_tools = [{ tool = \"node\", tests = [] }]\n",
        "test_tools = [{ tool = \"node\", tests = [\"\"] }]\n",
        "test_tools = [{ tool = \"node\", tests = \"workspace::\" }]\n",
        "test_tools = [{ tool = \"node >= x\", tests = [\"workspace::\"] }]\n",
        "test_tools = [{ tests = [\"workspace::\"] }]\n",
        "test_tools = [{ tool = \"node\", tests = [\"workspace::\"], when = 1 }]\n",
    ] {
        let err = crate::lockdown::parse_settings(bad).unwrap_err().0;
        assert!(err.starts_with("E6020") && err.contains("test_tools"), "{bad}: {err}");
    }
    let command =
        crate::lockdown::parse_settings("[commands]\nsuite = { run = [\"true\"], tools = [{ tool = \"node\", tests = [\"x\"] }] }\n");
    assert!(command.unwrap_err().0.contains("commands.suite.tools"));
}

#[test]
fn a_tool_is_checked_only_for_the_tests_that_need_it() {
    let root = Scratch::new()
        .with(&[(".solace/.keep", ""), ("solace.toml", "test_tools = [{ tool = \"node >= 19\", tests = [\"workspace::\"] }]\n")]);
    let bin = fake_tool(&root, "node", "v18.0.0");
    crate::language::with_tool_path(std::slice::from_ref(&bin), || {
        assert!(require(&root, "", &["board::tests::a_board_test".into()]).is_ok());
        assert!(require(&root, "", &[]).is_ok());
        let err = require(&root, "", &["board::tests::a_board_test".into(), "workspace::tests::has_gleam".into()]).unwrap_err().0;
        assert_eq!(err, "E6013 Tests not run: node 18.0.0 is older than the 19 these tests need");
    });
}

/// `test` in a Rust project stops before cargo runs when a test it will run
/// is one a missing tool is linked to, by its libtest name, and isn't held
/// back when the tool is linked to other tests.
#[test]
fn a_run_stops_only_when_its_tests_need_the_missing_tool() {
    let (root, store) = crate::testing::rust_project(ONE_WITH_A_TEST);
    fs::write(root.join("solace.toml"), "test_tools = [{ tool = \"no-such-tool\", tests = [\"tests::one_\"] }]\n").unwrap();
    let err = store.test(&["--all"]).unwrap_err().0;
    assert_eq!(err, "E6013 Tests not run: `no-such-tool` is not installed (not on PATH)");
    fs::write(root.join("solace.toml"), "test_tools = [{ tool = \"no-such-tool\", tests = [\"nothing_matches::\"] }]\n").unwrap();
    let out = store.test(&["--all"]).unwrap();
    assert!(out.starts_with("Ran all 1 tests."), "{out}");
}

/// A library with one function and the test of it.
const ONE_WITH_A_TEST: &str = concat!(
    "pub fn one() -> i32 {\n    1\n}\n\n",
    "#[cfg(test)]\nmod tests {\n    #[test]\n    fn one_is_one() {\n        assert_eq!(super::one(), 1);\n    }\n}\n"
);

#[test]
fn a_runtime_declared_for_some_tests_is_still_checked_for_the_others() {
    let scoped = "test_tools = [{ tool = \"node >= 19\", tests = [\"workspace::\"] }]\n";
    let root =
        Scratch::new().with(&[(".solace/.keep", ""), ("gleam.toml", "name = \"app\"\ntarget = \"javascript\"\n"), ("solace.toml", scoped)]);
    assert_eq!(
        needs(&root, &settings(scoped)).unwrap(),
        vec![
            Need { tool: "node".into(), minimum: Some("19".into()), tests: vec!["workspace::".into()] },
            Need { tool: "node".into(), minimum: None, tests: vec![] },
        ]
    );
    let err = crate::language::with_tool_path(&[], || require(&root, "", &["board::tests::a_board_test".into()]).unwrap_err().0);
    assert_eq!(err, "E6013 Tests not run: `node` is not installed (not on PATH)");
}

/// `run` of a command whose `tools` names a missing tool stops in one
/// line, before its program runs or its files are snapshotted.
#[test]
fn a_run_stops_while_a_listed_tool_is_missing() {
    let s = crate::testing::project(&[("src/main.gleam", "pub fn main() { 1 }\n")]);
    let v: toml::Value = "[commands]\nsuite={run=[\"touch\",\"ran\"],tools=[\"solace-absent-tool\"]}".parse().unwrap();
    let err = crate::language::with_tool_path(&[], || crate::testing::dispatch(&s, &v, &["run", "suite"], "").unwrap_err().0);
    assert_eq!(err, "E6013 Not run: `solace-absent-tool` is not installed (not on PATH)");
    assert!(!s.root.join("ran").exists());
}
