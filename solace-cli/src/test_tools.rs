//! The tools a project's tests need (D-263): those its `solace.toml` lists in
//! `test_tools`, and the JavaScript runtime of a Gleam project built for
//! JavaScript. An entry can name the tests that need it by name prefix. `test`
//! checks the tools the tests it will run need, once, before any of them, and
//! runs nothing while one is missing or older than they need. A `[commands]` entry
//! lists its own in `tools`, which `run <name>` checks the same way.

use crate::Failure;
use std::path::Path;
#[cfg(test)]
#[path = "test_tools_tests.rs"]
mod tests;
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

/// Every tool the tests in `root` need: the declared ones, then its runtime
/// unless a declared entry for it applies to every test.
fn needs(root: &Path, settings: &toml::Value) -> Result<Vec<Need>, Failure> {
    let mut needs = declared(settings)?;
    if let Some(tool) = runtime(root)
        && !needs.iter().any(|n| n.tool == tool && n.tests.is_empty())
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
    crate::tools::not_installed(tool).unwrap_or_else(|| format!("`{tool}` is not installed (not on PATH)"))
}
