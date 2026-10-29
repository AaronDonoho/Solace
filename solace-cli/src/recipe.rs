//! Build recipes (D-131): how each project builds, declared in its
//! `solace.toml` `[build]` or taken from the files it has, and the order the
//! workspace builds them in.
//! A passing `run` of a recipe's last step records what it built (D-166), so
//! `build plan` judges staleness by contents, not file times.

use super::*;
use std::path::{Component, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
/// A step's time limit, in seconds, when neither it nor its command sets one (as for `run`).
const TIMEOUT: i64 = 120;

/// What a project's outputs are built from (D-159): its sources, and the manifests whose
/// contents reach the build.
const SOURCES: [&str; 4] = ["src", "gleam.toml", "package.json", "bun.lock"];

/// A source newer than an output by less than this counts as the same build: `git checkout`
/// writes files milliseconds apart, while an edit that needs a rebuild comes seconds later.
const FRESH: Duration = Duration::from_secs(5);

/// One step of a recipe: what it runs, and the command it reuses, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub name: String,
    pub run: Vec<String>,
    pub command: Option<String>,
    pub timeout: i64,
}

/// A project's recipe: `declared` in its `[build]`, or else its default.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub project: String,
    pub declared: bool,
    pub after: Vec<String>,
    pub outputs: Vec<String>,
    pub steps: Vec<Step>,
}

/// A path relative to its base, with no `..`: an output inside the project,
/// or a project folder below the root.
fn inside(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !p.contains('\\')
        && path.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

/// A project folder as `workspace::members` names it.
fn folder(p: &str) -> String {
    let p = p.trim_end_matches('/');
    match p.strip_prefix("./") {
        Some(rest) if !rest.is_empty() => rest.into(),
        _ if p.is_empty() => ".".into(),
        _ => p.into(),
    }
}

/// Checks `[build]`'s shape (D-131); `E6020` names the field at fault. Which
/// projects `after` names is `check_after`'s, since it needs the root.
pub fn check(v: &toml::Value) -> Result<(), Failure> {
    let Some(b) = v.get("build") else {
        return Ok(());
    };
    let bad = |m: String| Err(lockdown::invalid_settings(m));
    let Some(b) = b.as_table() else {
        return bad("build: expected a table with steps, and optionally after and outputs".into());
    };
    if let Some(k) = b.keys().find(|k| !matches!(k.as_str(), "after" | "outputs" | "steps")) {
        return bad(format!("build.{k}: unknown field; [build] takes after, outputs and steps"));
    }
    for (key, base) in [("after", "Solace root"), ("outputs", "project")] {
        let Some(x) = b.get(key) else { continue };
        let Some(a) = x.as_array() else {
            return bad(format!("build.{key}: expected an array of paths"));
        };
        for (i, p) in a.iter().enumerate() {
            match p.as_str() {
                Some(p) if inside(p) => {}
                Some(p) => return bad(format!("build.{key}[{i}]: `{p}` is not a path inside the {base}")),
                None => return bad(format!("build.{key}[{i}]: expected a path")),
            }
        }
    }
    let Some(steps) = b.get("steps").and_then(|s| s.as_array()).filter(|s| !s.is_empty()) else {
        return bad("build.steps: expected an array of at least one step, such as { name = \"bundle\", run = [...] }".into());
    };
    let commands = v.get("commands").and_then(|c| c.as_table());
    let mut names = HashSet::new();
    for (i, s) in steps.iter().enumerate() {
        let at = format!("build.steps[{i}]");
        let Some(s) = s.as_table() else {
            return bad(format!("{at}: expected a table such as {{ name = \"bundle\", run = [...] }}"));
        };
        if let Some(k) = s.keys().find(|k| !matches!(k.as_str(), "name" | "run" | "command" | "timeout")) {
            return bad(format!("{at}.{k}: unknown field; a step takes name, run or command, and timeout"));
        }
        let Some(name) = s.get("name").and_then(|n| n.as_str()).filter(|n| !n.trim().is_empty()) else {
            return bad(format!("{at}.name: every step needs a name"));
        };
        if !names.insert(name) {
            return bad(format!("{at}.name: another step is already named `{name}`"));
        }
        match (s.get("run"), s.get("command")) {
            (Some(_), Some(_)) => return bad(format!("{at}: `run` or `command`, not both")),
            (None, None) => return bad(format!("{at}: needs `run` (the arguments to run) or `command` (a name in [commands])")),
            (Some(r), None) => {
                if r.as_array().is_none_or(|a| a.is_empty() || a.iter().any(|x| x.as_str().is_none())) {
                    return bad(format!("{at}.run: expected a non-empty array of strings"));
                }
            }
            (None, Some(c)) => {
                let Some(c) = c.as_str() else {
                    return bad(format!("{at}.command: expected a command's name"));
                };
                if commands.and_then(|t| t.get(c)).is_none() {
                    let declared = commands.map(|t| t.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default();
                    return bad(format!("{at}.command: [commands] has no `{c}`; declared: {declared}"));
                }
            }
        }
        if s.get("timeout").is_some_and(|t| t.as_integer().is_none_or(|n| n <= 0)) {
            return bad(format!("{at}.timeout: expected a positive number of seconds"));
        }
    }
    Ok(())
}

fn after_of(v: &toml::Value) -> Vec<String> {
    v.get("build")
        .and_then(|b| b.get("after"))
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(folder).collect())
        .unwrap_or_default()
}

/// `after` names other projects of the workspace (`workspace::members`).
fn after_in(dir: &str, after: &[String], members: &[String]) -> Result<(), Failure> {
    for (i, a) in after.iter().enumerate() {
        if a == dir {
            return Err(lockdown::invalid_settings(format!("build.after[{i}]: `{a}` is this project; name the projects built before it")));
        }
        if !members.contains(a) {
            return Err(lockdown::invalid_settings(format!(
                "build.after[{i}]: `{a}` is not a project of this workspace; its projects: {}",
                members.join(", ")
            )));
        }
    }
    Ok(())
}

/// Checks that the `after` of project `dir`'s settings `v` names projects of the root `top`.
pub fn check_after(top: &Path, dir: &str, v: &toml::Value) -> Result<(), Failure> {
    after_in(dir, &after_of(v), &workspace::members(top)?)
}

fn argv(x: Option<&toml::Value>) -> Vec<String> {
    x.and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The recipe a checked `[build]` declares; a `command` step runs that command's `run`.
fn declared(dir: &str, v: &toml::Value) -> Recipe {
    let b = &v["build"];
    let steps = b.get("steps").and_then(|s| s.as_array()).into_iter().flatten().map(|s| {
        let command = s.get("command").and_then(|c| c.as_str()).map(String::from);
        let c = command.as_deref().and_then(|c| v.get("commands")?.get(c));
        let timeout = s.get("timeout").or_else(|| c.and_then(|c| c.get("timeout"))).and_then(|t| t.as_integer());
        Step {
            name: s.get("name").and_then(|n| n.as_str()).unwrap_or_default().into(),
            run: argv(s.get("run").or_else(|| c.and_then(|c| c.get("run")))),
            command,
            timeout: timeout.unwrap_or(TIMEOUT),
        }
    });
    Recipe { project: dir.into(), declared: true, after: after_of(v), outputs: argv(b.get("outputs")), steps: steps.collect() }
}

/// The recipe a project gets from its files (D-131 point 3): install its
/// packages, compile its Gleam, run its `build` script, build its crate.
/// `None` when it has none of `package.json`, `gleam.toml` and `Cargo.toml`.
pub fn default(root: &Path, dir: &str) -> Option<Recipe> {
    let step = |name: &str, run: &[&str]| Step {
        name: name.into(),
        run: run.iter().map(|s| s.to_string()).collect(),
        command: None,
        timeout: TIMEOUT,
    };
    let package = fs::read_to_string(root.join("package.json")).ok().map(|t| serde_json::from_str::<Value>(&t).unwrap_or(Value::Null));
    let bun = root.join("bun.lock").exists() || root.join("bun.lockb").exists();
    let mut steps = vec![];
    if package.is_some() {
        steps.push(match (bun, root.join("package-lock.json").exists()) {
            (true, _) => step("install", &["bun", "install"]),
            (false, true) => step("install", &["npm", "ci"]),
            (false, false) => step("install", &["npm", "install"]),
        });
    }
    if root.join("gleam.toml").exists() {
        steps.push(step("gleam", &["gleam", "build", "--target", workspace::target(root).as_str()]));
    }
    if package.as_ref().is_some_and(|p| p["scripts"]["build"].is_string()) {
        steps.push(step("build", &[if bun { "bun" } else { "npm" }, "run", "build"]));
    }
    if root.join("Cargo.toml").exists() {
        steps.push(step("cargo", &["cargo", "build", "--release", "--locked"]));
    }
    (!steps.is_empty()).then(|| Recipe { project: dir.into(), declared: false, after: vec![], outputs: vec![], steps })
}

/// Project `dir`'s recipe: its `[build]`, else its default.
pub fn of(top: &Path, dir: &str) -> Result<Option<Recipe>, Failure> {
    let root = scope::path(top, dir);
    let named = |e: Failure| Failure(format!("{}\n  in {dir}/solace.toml", e.0.trim_end()), e.1);
    let v = lockdown::settings(&root).map_err(named)?;
    if v.get("build").is_none() {
        return Ok(default(&root, dir));
    }
    check(&v).map_err(named)?;
    Ok(Some(declared(dir, &v)))
}

/// Every project's recipe, in build order: each after the projects its
/// `after` names, else in the root's order. Projects with no recipe are left
/// out; a cycle is refused and named.
pub fn plan(top: &Path) -> Result<Vec<Recipe>, Failure> {
    let members = workspace::members(top)?;
    let mut recipes = vec![];
    for dir in &members {
        if let Some(r) = of(top, dir)? {
            after_in(dir, &r.after, &members).map_err(|e| Failure(format!("{}\n  in {dir}/solace.toml", e.0.trim_end()), e.1))?;
            recipes.push(r);
        }
    }
    order(recipes)
}

/// `recipes` sorted so each comes after those its `after` names.
fn order(recipes: Vec<Recipe>) -> Result<Vec<Recipe>, Failure> {
    let index: HashMap<&str, usize> = recipes.iter().enumerate().map(|(i, r)| (r.project.as_str(), i)).collect();
    // 0: not reached, 1: on the current path, 2: placed.
    let mut state = vec![0u8; recipes.len()];
    let (mut path, mut placed) = (vec![], vec![]);
    fn visit(
        i: usize,
        rs: &[Recipe],
        index: &HashMap<&str, usize>,
        state: &mut [u8],
        path: &mut Vec<usize>,
        placed: &mut Vec<usize>,
    ) -> Result<(), Failure> {
        if state[i] == 2 {
            return Ok(());
        }
        if state[i] == 1 {
            let from = path.iter().position(|&p| p == i).unwrap_or(0);
            let mut names: Vec<&str> = path[from..].iter().map(|&p| rs[p].project.as_str()).collect();
            names.push(rs[i].project.as_str());
            return Err(Failure(
                format!(
                    "E6020 build order has a cycle: {}\n  each is built after the next ([build] after); remove one of them",
                    names.join(" → ")
                ),
                1,
            ));
        }
        state[i] = 1;
        path.push(i);
        for a in &rs[i].after {
            if let Some(&j) = index.get(a.as_str()) {
                visit(j, rs, index, state, path, placed)?;
            }
        }
        path.pop();
        state[i] = 2;
        placed.push(i);
        Ok(())
    }
    for i in 0..recipes.len() {
        visit(i, &recipes, &index, &mut state, &mut path, &mut placed)?;
    }
    Ok(placed.into_iter().map(|i| recipes[i].clone()).collect())
}

/// `build plan`: each project's recipe in build order, marked declared or default.
pub fn plan_text(top: &Path) -> Out {
    let recipes = plan(top)?;
    if recipes.is_empty() {
        return Ok(concat!(
            "No project has a build recipe: none declares [build] in its solace.toml, ",
            "and none has gleam.toml, Cargo.toml or package.json.\n"
        )
        .into());
    }
    let mut out = format!("Build plan, in order ({}):\n", count(recipes.len(), "project"));
    for (i, r) in recipes.iter().enumerate() {
        out += &format!("{}. {} ({})\n", i + 1, r.project, if r.declared { "declared" } else { "default" });
        if !r.after.is_empty() {
            out += &format!("   after: {}\n", r.after.join(", "));
        }
        if !r.outputs.is_empty() {
            out += &format!("   outputs: {}\n", r.outputs.join(", "));
            if let Some(why) = stale(top, r) {
                let steps: Vec<String> = r.steps.iter().map(|s| format!("`{}`", s.name)).collect();
                out += &format!("   out of date: {why}: run {} {}\n", if steps.len() == 1 { "step" } else { "steps" }, steps.join(", "));
            }
        }
        for s in &r.steps {
            let via = s.command.as_ref().map(|c| format!(" (command {c})")).unwrap_or_default();
            let limit = if s.timeout == TIMEOUT { String::new() } else { format!(" [timeout {} s]", s.timeout) };
            out += &format!("   - {}{via}: {}{limit}\n", s.name, s.run.join(" "));
        }
    }
    Ok(out)
}

/// `GET /build/plan`: the plan as the page reads it.
pub fn plan_json(top: &Path) -> Result<Value, Failure> {
    let recipes = plan(top)?;
    let shown = |r: &Recipe| {
        let steps: Vec<Value> = r
            .steps
            .iter()
            .map(|s| {
                let mut j = json!({"name": s.name, "run": s.run, "timeout": s.timeout});
                if let Some(c) = &s.command {
                    j["command"] = json!(c);
                }
                j
            })
            .collect();
        let source = if r.declared { "declared" } else { "default" };
        json!({"project": r.project, "source": source, "after": r.after, "outputs": r.outputs, "stale": stale(top, r), "steps": steps})
    };
    Ok(json!(recipes.iter().map(shown).collect::<Vec<_>>()))
}

/// `path` with `.` and `..` worked out by name, as a file manager does.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The newest modification time at `path`, a file or a folder tree.
fn newest(path: &Path) -> Option<SystemTime> {
    let own = fs::metadata(path).and_then(|m| m.modified()).ok();
    // A symlink counts by its target's time and isn't followed: one inside `src` pointing up the
    // tree would recurse until the stack overflows.
    let entry = |e: fs::DirEntry| {
        if e.file_type().is_ok_and(|t| t.is_symlink()) { fs::metadata(e.path()).and_then(|m| m.modified()).ok() } else { newest(&e.path()) }
    };
    fs::read_dir(path).into_iter().flatten().flatten().filter_map(entry).chain(own).max()
}

/// The projects whose files the project at `root` builds from: itself, and each path
/// dependency its `gleam.toml` lists.
fn built_from(root: &Path) -> Vec<PathBuf> {
    let manifest = fs::read_to_string(root.join("gleam.toml")).ok().and_then(|t| t.parse::<toml::Value>().ok());
    let deps = manifest.as_ref().and_then(|m| m.get("dependencies")).and_then(|d| d.as_table());
    let paths = deps.into_iter().flat_map(|d| d.values()).filter_map(|d| d.get("path")?.as_str());
    [lexical(root)].into_iter().chain(paths.map(|p| lexical(&root.join(p)))).collect()
}

/// The recipe whose last step is `command`, when it names outputs (D-166): a passing `run` of
/// that command records what it built.
fn recording(top: &Path, project: &str, command: &str) -> Result<Option<Recipe>, Failure> {
    let Some(r) = of(top, project)? else { return Ok(None) };
    Ok((!r.outputs.is_empty() && r.steps.last().and_then(|s| s.command.as_deref()) == Some(command)).then_some(r))
}

/// Each source part of the project at `root`, and its hash now (D-166).
fn source_hashes(top: &Path, root: &Path) -> Value {
    Value::Object(source_parts(top, root).into_iter().map(|(name, part)| (name, json!(part_hash(&part)))).collect())
}

/// D-166: what a recording build's `sources` hold, hashed now. The caller takes it before the
/// command runs, so a source the build itself changes isn't recorded as built. `None` when this
/// run records nothing.
pub fn built_sources(top: &Path, project: &str, command: &str) -> Result<Option<Value>, Failure> {
    if recording(top, project, command)?.is_none() {
        return Ok(None);
    }
    Ok(Some(source_hashes(top, &scope::path(top, project))))
}

/// D-166: a passing `run` of a recipe's last step `command` records what it built in the project's
/// `.solace/built.json`: the hash of each output now, and of each source part as `sources` held it.
/// A source touched without changing, or a file added later, is then judged by contents, not times.
pub fn record_built(top: &Path, project: &str, command: &str, sources: Value) -> Result<(), Failure> {
    let Some(r) = recording(top, project, command)? else { return Ok(()) };
    let root = scope::path(top, project);
    let mut outputs = serde_json::Map::new();
    for o in &r.outputs {
        // An output the command didn't make leaves the file-time rule in charge.
        let Ok(bytes) = fs::read(root.join(o)) else { return Ok(()) };
        outputs.insert(o.clone(), json!(sha256(bytes)));
    }
    fs::create_dir_all(root.join(".solace"))?;
    fs::write(root.join(".solace/built.json"), json!({"command": command, "outputs": outputs, "sources": sources}).to_string())?;
    Ok(())
}

/// The source parts of the project at `root` (D-159): its own and its path dependencies'
/// `SOURCES`, each named from the root, as `b/src`.
fn source_parts(top: &Path, root: &Path) -> Vec<(String, PathBuf)> {
    let top = lexical(top);
    built_from(root)
        .into_iter()
        .flat_map(|p| SOURCES.map(move |s| (p.clone(), s)))
        .map(|(p, s)| {
            let part = p.join(s);
            (part.strip_prefix(&top).unwrap_or(&part).display().to_string(), part)
        })
        .collect()
}

/// The files below `path`, by relative name and contents; a symlink counts as the file it names
/// and isn't followed into a folder, as in `newest`.
fn fingerprints(path: &Path, rel: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    match fs::read_dir(path) {
        Ok(read) => {
            for e in read.flatten() {
                let name = rel.join(e.file_name());
                if e.file_type().is_ok_and(|t| t.is_symlink()) {
                    if let Ok(bytes) = fs::read(e.path()) {
                        out.push((name.display().to_string(), bytes));
                    }
                } else {
                    fingerprints(&e.path(), &name, out);
                }
            }
        }
        // A file, or a part that isn't there: nothing to walk.
        Err(_) => {
            if let Ok(bytes) = fs::read(path) {
                out.push((rel.display().to_string(), bytes));
            }
        }
    }
}

/// The hash of a source part (D-166): the names and contents of its files, sorted.
fn part_hash(path: &Path) -> String {
    let mut files = vec![];
    fingerprints(path, Path::new(""), &mut files);
    files.sort();
    let mut blob = vec![];
    for (name, bytes) in files {
        blob.extend_from_slice(name.as_bytes());
        blob.push(0);
        blob.extend_from_slice(&bytes);
        blob.push(0);
    }
    sha256(blob)
}

/// D-166: what the recorded build says changed since it ran. `None` when there is no record or an
/// output isn't the recorded one (it was rebuilt some other way): the caller falls back to file
/// times. `Some(None)` means the record applies and nothing changed.
fn recorded_change(root: &Path, r: &Recipe, parts: &[(String, PathBuf)]) -> Option<Option<String>> {
    let record: Value = jsonx::read_opt(&root.join(".solace/built.json"))?;
    // The record is for the last step as it was: a recipe that now names another command
    // says nothing about this build.
    if record["command"].as_str() != r.steps.last().and_then(|s| s.command.as_deref()) {
        return None;
    }
    for o in &r.outputs {
        let bytes = fs::read(root.join(o)).ok()?;
        if record["outputs"][o.as_str()].as_str() != Some(sha256(bytes).as_str()) {
            return None;
        }
    }
    let changed: Vec<String> = parts
        .iter()
        .filter(|(name, part)| record["sources"][name.as_str()].as_str() != Some(part_hash(part).as_str()))
        .map(|(name, _)| name.clone())
        .collect();
    Some((!changed.is_empty()).then(|| format!("{} changed since {} was built", changed.join(", "), r.outputs.join(", "))))
}

/// Why a recipe's outputs need its steps run (D-159, D-166): one is missing, or a source of the
/// project or of a path dependency changed since it was built. `None` when they are current or
/// the recipe names no outputs.
fn stale(top: &Path, r: &Recipe) -> Option<String> {
    let root = scope::path(top, &r.project);
    let mut built: Option<SystemTime> = None;
    for o in &r.outputs {
        match fs::metadata(root.join(o)).and_then(|m| m.modified()) {
            Ok(t) => built = Some(built.map_or(t, |b| b.min(t))),
            Err(_) => return Some(format!("{o} is missing")),
        }
    }
    let built = built?;
    let parts = source_parts(top, &root);
    if let Some(change) = recorded_change(&root, r, &parts) {
        return change;
    }
    let changed: Vec<String> =
        parts.iter().filter(|(_, part)| newest(part).is_some_and(|t| t > built + FRESH)).map(|(name, _)| name.clone()).collect();
    (!changed.is_empty()).then(|| format!("{} changed after {} was built", changed.join(", "), r.outputs.join(", ")))
}

#[cfg(test)]
#[path = "recipe_tests.rs"]
mod tests;
