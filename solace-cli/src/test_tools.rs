//! The tools a project's tests need (D-263): those its `solace.toml` lists in
//! `test_tools`, and the JavaScript runtime of a Gleam project built for
//! JavaScript. An entry can name the tests that need it by name prefix. `test`
//! checks the tools the tests it will run need, once, before any of them, and
//! runs nothing while one is missing or older than they need. A `[commands]` entry
//! lists its own in `tools`, which `run <name>` checks the same way.

use crate::Failure;
use std::path::Path;
/// A tool the tests need, and the oldest version they accept as written.
#[derive(Debug, PartialEq)]
pub(crate) struct Need {
    pub(crate) tool: String,
    pub(crate) minimum: Option<String>,
    /// The name prefixes of the tests that need it; none means every test.
    pub(crate) tests: Vec<String>,
}

/// One `test_tools` entry: `"node"`, or `"node >= 19"` with a minimum version.
fn need(entry: &str) -> Option<Need> {
    let (tool, minimum) = match entry.split_once(">=") {
        Some((tool, minimum)) => (tool.trim(), Some(minimum.trim())),
        None => (entry.trim(), None),
    };
    if tool.is_empty() || tool.contains(char::is_whitespace) || minimum.is_some_and(|m| minimum_version(m).is_none()) {
        return None;
    }
    Some(Need { tool: tool.to_string(), minimum: minimum.map(str::to_string), tests: vec![] })
}

/// A minimum as written, `19`, `19.1` or `19.1.2`, with missing parts 0.
fn minimum_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// The tools `test_tools` lists, refused when an entry isn't one `test_need` reads.
fn declared(settings: &toml::Value) -> Result<Vec<Need>, Failure> {
    let expected = concat!(
        "expected a list of tools, each `name`, `name >= version` or `{ tool = \"name >= version\", tests = [\"prefix\"] }`, ",
        "such as [\"gleam\", { tool = \"node >= 19\", tests = [\"workspace::\"] }]"
    );
    listed(settings.get("test_tools"), "test_tools", test_need, expected)
}

pub(crate) fn check(settings: &toml::Value) -> Result<(), Failure> {
    declared(settings)?;
    for (name, command) in settings.get("commands").and_then(|c| c.as_table()).into_iter().flatten() {
        command_tools(command, &format!("commands.{name}.tools"))?;
    }
    Ok(())
}

/// The runtime a Gleam project built for JavaScript runs its tests on; Gleam's default is Node.
fn runtime(root: &Path) -> Option<String> {
    if crate::workspace::target(root) != "javascript" {
        return None;
    }
    let manifest: toml::Value = std::fs::read_to_string(root.join("gleam.toml")).ok()?.parse().ok()?;
    let runtime = manifest.get("javascript").and_then(|j| j.get("runtime")).and_then(|r| r.as_str());
    Some(runtime.unwrap_or("node").to_string())
}

/// Every tool the tests in `root` need: the declared ones, then its runtime unless declared.
fn needs(root: &Path, settings: &toml::Value) -> Result<Vec<Need>, Failure> {
    let mut needs = declared(settings)?;
    if let Some(tool) = runtime(root)
        && !needs.iter().any(|n| n.tool == tool)
    {
        needs.push(Need { tool, minimum: None, tests: vec![] });
    }
    Ok(needs)
}

/// Why `need` isn't met here, if it isn't.
fn problem(root: &Path, need: &Need) -> Option<String> {
    let tool = &need.tool;
    let Some(path) = crate::language::tool_path(tool) else {
        return Some(missing(tool));
    };
    let wanted = need.minimum.as_deref()?;
    let floor = minimum_version(wanted)?;
    let argv = [path.to_string_lossy().into_owned(), "--version".to_string()];
    let found = crate::lockdown::commands::command(root, &argv, 5, 4000, false, false)
        .ok()
        .and_then(|out| out.split_once('\n').and_then(|(_, text)| crate::board::version(text)));
    match found {
        Some(v) if v >= floor => None,
        Some((major, minor, patch)) => Some(format!("{tool} {major}.{minor}.{patch} is older than the {wanted} these tests need")),
        None => Some(format!("couldn't read `{tool} --version`; these tests need {wanted} or newer")),
    }
}

/// Refuses a run of `tests`, by name, in `root` while a tool they need is
/// missing or too old; `dir` names a dependent project, "" the project tested.
pub(crate) fn require(root: &Path, dir: &str, tests: &[String]) -> Result<(), Failure> {
    let settings = crate::lockdown::settings(root)?;
    let Some(why) = needs(root, &settings)?.iter().filter(|n| needed_by(n, tests)).find_map(|n| problem(root, n)) else {
        return Ok(());
    };
    if dir.is_empty() {
        crate::fail(format!("E6013 Tests not run: {why}"))
    } else {
        crate::fail(format!("E6013 Tests not run in {dir}: {why}"))
    }
}

#[cfg(test)]
mod tests {
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
}

/// The tools a list of entries names, refused under `key` with `expected` when an entry isn't one `read` reads.
fn listed(list: Option<&toml::Value>, key: &str, read: fn(&toml::Value) -> Option<Need>, expected: &str) -> Result<Vec<Need>, Failure> {
    let Some(list) = list else { return Ok(vec![]) };
    let expected = || crate::lockdown::invalid_settings(format!("{key}: {expected}"));
    list.as_array().ok_or_else(expected)?.iter().map(|entry| read(entry).ok_or_else(expected)).collect()
}

/// Refuses `run` of a declared `command` in `root` while a tool its `tools` lists is missing or too old.
pub(crate) fn require_command(root: &Path, command: &toml::Value) -> Result<(), Failure> {
    let Some(why) = command_tools(command, "tools")?.iter().find_map(|n| problem(root, n)) else { return Ok(()) };
    crate::fail(format!("E6013 Not run: {why}"))
}

/// One `test_tools` entry: a tool as `need` reads it, or a table naming it in
/// `tool` and the name prefixes of the tests that need it in `tests`.
fn test_need(entry: &toml::Value) -> Option<Need> {
    let Some(table) = entry.as_table() else { return need(entry.as_str()?) };
    if table.keys().any(|key| key != "tool" && key != "tests") {
        return None;
    }
    let tests: Vec<String> = table
        .get("tests")?
        .as_array()?
        .iter()
        .map(|test| test.as_str().filter(|test| !test.is_empty()).map(str::to_string))
        .collect::<Option<_>>()?;
    let need = need(table.get("tool")?.as_str()?)?;
    (!tests.is_empty()).then_some(Need { tests, ..need })
}

/// Whether one of `tests`, by name, needs `need`.
fn needed_by(need: &Need, tests: &[String]) -> bool {
    need.tests.is_empty() || tests.iter().any(|test| need.tests.iter().any(|prefix| test.starts_with(prefix.as_str())))
}

/// The tools a `[commands]` entry lists in `tools`, refused under `key` when one isn't a tool `need` reads.
fn command_tools(command: &toml::Value, key: &str) -> Result<Vec<Need>, Failure> {
    let expected = "expected a list of tools, each `name` or `name >= version`, such as [\"gleam\", \"node >= 19\"]";
    listed(command.get("tools"), key, |entry| need(entry.as_str()?), expected)
}

/// Why `tool` isn't found: Solace's own tools are installed from its checklist
/// (D-264), any other is looked for on PATH.
fn missing(tool: &str) -> String {
    if crate::tools::program_folder(tool).is_some() {
        format!("`{tool}` is not installed; install it with Install on Solace's checklist")
    } else {
        format!("`{tool}` is not installed (not on PATH)")
    }
}
