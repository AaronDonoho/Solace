use std::collections::HashMap;
use std::path::Path;
use std::fs;
use std::path::PathBuf;
use crate::Store;
use crate::failure::Failure;
use crate::lockdown::error;
use crate::lockdown::files::tree_rows;
use crate::lockdown::settings::limit;
use crate::lockdown::settings::dependencies;
use crate::lockdown::settings::rust_setting;
use crate::lockdown::settings::in_skills;
#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
use crate::scope;
use crate::workspace;
use crate::board::runs::strategy_context;
pub(crate) fn matches(pattern: &str, path: &str) -> bool {
    if pattern.ends_with('/') {
        return path == pattern.trim_end_matches('/') || path.starts_with(pattern);
    }
    globset::Glob::new(pattern).is_ok_and(|g| g.compile_matcher().is_match(path))
}

pub(crate) fn patterns(v: &toml::Value, key: &str, path: &str) -> bool {
    v.get(key).and_then(|a| a.as_array()).is_some_and(|a| a.iter().any(|p| matches(p.as_str().unwrap(), path)))
}

// Reject aliases, including symlinks inside the root: an alias must never
// grant file access to source, protected files, or Solace's own records.
pub fn path(root: &Path, name: &str) -> Result<PathBuf, Failure> {
    let p = Path::new(name);
    if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
        return error("E6002", name, "expected a path inside the project");
    }
    let mut out = root.to_path_buf();
    for c in p.components() {
        out.push(c);
        if fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
            return error("E6002", name, "symlink paths are not accessible");
        }
    }
    Ok(out)
}

/// Solace's own files, and every file of another Solace project nested in
/// this one (it has its own store; D-57).
fn internal(root: &Path, name: &str) -> bool {
    let mut dir = root.to_path_buf();
    Path::new(name).components().any(|c| {
        dir.push(c);
        c.as_os_str() == ".solace" || dir.join(".solace/store.db").exists()
    })
}

/// The `.gitignore` files of a project, each read once and matched from its
/// own folder, as git does: a walk keeps one and asks it for every entry.
pub(crate) struct Ignores {
    pub(crate) root: PathBuf,
    pub(crate) folders: HashMap<PathBuf, Option<ignore::gitignore::Gitignore>>,
}

impl Ignores {
    pub(crate) fn new(root: &Path) -> Ignores {
        Ignores { root: root.to_path_buf(), folders: HashMap::new() }
    }
    /// Whether `name`, relative to the root, is ignored. As in git, a path in
    /// an ignored folder stays ignored: no `!pattern` below re-includes it.
    pub(crate) fn ignored(&mut self, name: &str) -> bool {
        let parts: Vec<&str> = name.split('/').collect();
        (1..=parts.len()).any(|k| self.decides(&parts[..k].join("/")))
    }
    /// Whether `name` itself is ignored: the deepest `.gitignore` that names
    /// it, or a folder above it, decides.
    fn decides(&mut self, name: &str) -> bool {
        let path = self.root.join(name);
        let is_dir = path.is_dir();
        let mut folders = vec![self.root.clone()];
        for c in Path::new(name).parent().unwrap_or(Path::new("")).components() {
            let next = folders.last().unwrap().join(c);
            folders.push(next);
        }
        for dir in folders.into_iter().rev() {
            let g = self.folders.entry(dir.clone()).or_insert_with(|| {
                let mut builder = ignore::gitignore::GitignoreBuilder::new(&dir);
                builder.add(dir.join(".gitignore"));
                builder.build().ok().filter(|g| !g.is_empty())
            });
            if let Some(g) = g {
                let m = g.matched_path_or_any_parents(&path, is_dir);
                if m.is_ignore() {
                    return true;
                }
                if m.is_whitelist() {
                    return false;
                }
            }
        }
        false
    }
}

/// A `.gitignore` anywhere in a project: what it ignores is unwatched, so only a human changes it.
pub fn is_gitignore(name: &str) -> bool {
    (name == ".gitignore" || name.ends_with("/.gitignore")) && !name.split('/').any(|c| c == ".solace")
}

/// The module a graph file belongs to (D-158, D-181): Gleam's path under `src/`
/// or `test/`, the Rust store's module row for that file, or the module a new
/// Rust path certainly names.
pub(crate) fn module_of_file(s: &Store, name: &str) -> Option<String> {
    if name.ends_with(".gleam")
        && let Some(rest) = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/"))
    {
        let module = rest.trim_end_matches(".gleam");
        if name.starts_with("test/") {
            return Some(test_module_name(module));
        }
        return Some(module.to_string());
    }
    if let Ok(module) = s.db.query_row("SELECT path FROM modules WHERE file = ?1", [name], |r| r.get::<_, String>(0)) {
        return Some(module);
    }
    rust_module_of_path(&s.root, name)
}

pub fn tier(root: &Path, v: &toml::Value, name: &str, proposed: Option<&[u8]>) -> String {
    tier_and_bytes(root, v, &mut Ignores::new(root), name, proposed).0
}

/// `tier`, with the file's bytes when they were read to decide the tier, so a
/// caller that wants a size reads the file once (D-119 point 4).
pub(crate) fn tier_and_bytes(root: &Path, v: &toml::Value, ignores: &mut Ignores, name: &str, proposed: Option<&[u8]>) -> (String, Option<Vec<u8>>) {
    // A project of plain files holds no symbols (D-60).
    if crate::language::detect(root).is_some() && (name.starts_with("src/") || name.starts_with("test/")) && name.ends_with(".gleam") {
        return ("graph".into(), None);
    }
    // A Rust project's `.rs` files are symbols too (D-158 point 1).
    if crate::language::is_rust(root) && (name.starts_with("src/") || name.starts_with("tests/")) && name.ends_with(".rs") {
        return ("graph".into(), None);
    }
    if name == "solace.toml" || name == "AGENTS.md" {
        return ("protected".into(), None);
    }
    // Cargo's and rustup's settings run programs at check time (D-158 point 4, D-174);
    // a nested project's own files stay internal (D-57).
    if crate::language::is_rust(root) && rust_setting(name) && !internal(root, name) {
        return ("protected".into(), None);
    }
    if in_skills(v, name) {
        return ("protected".into(), None);
    }
    if internal(root, name) {
        return ("internal".into(), None);
    }
    if is_gitignore(name) {
        return ("protected".into(), None);
    }
    if dependencies(name) {
        return ("unwatched".into(), None);
    }
    if name == ".git"
        || name.starts_with(".git/")
        || name == "build"
        || name.starts_with("build/")
        || name == "manifest.toml"
        || name == "target"
        || name.starts_with("target/")
    {
        return ("unwatched".into(), None);
    }
    if ignores.ignored(name) || patterns(v, "unwatched", name) {
        return ("unwatched".into(), None);
    }
    let bytes = proposed.map(Vec::from).unwrap_or_else(|| fs::read(root.join(name)).unwrap_or_default());
    if bytes.len() as u64 > limit(v) as u64 {
        return ("unwatched (over limit)".into(), Some(bytes));
    }
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return ("unwatched (binary)".into(), Some(bytes));
    }
    ("watched".into(), Some(bytes))
}

/// Every watched file path, relative to `root`.
pub fn watched_paths(root: &Path, v: &toml::Value) -> Result<Vec<String>, Failure> {
    let mut rows = vec![];
    tree_rows(root, v, root, &mut rows)?;
    Ok(rows.into_iter().filter(|(_, t, _)| t == "watched").map(|(n, _, _)| n).collect())
}

/// Every graph file path, relative to `root`: `files` lists them, and a view
/// refuses them by path because a source file is a projection (D-181).
pub fn graph_paths(root: &Path, v: &toml::Value) -> Result<Vec<String>, Failure> {
    let mut rows = vec![];
    tree_rows(root, v, root, &mut rows)?;
    Ok(rows.into_iter().filter(|(_, t, _)| t == "graph").map(|(n, _, _)| n).collect())
}

/// Where `name` is in `root`, if an agent may reach it: not Solace's own files,
/// not out of the project or into a project nested in it, and no symlink.
pub(crate) fn reachable(root: &Path, name: &str) -> Result<PathBuf, Failure> {
    if Path::new(name).is_absolute() {
        return error("E6002", name, "expected a path inside the project");
    }
    if Path::new(name).components().any(|c| c.as_os_str() == ".solace") {
        return error("E6006", name, "Solace internal files have no agent access");
    }
    // A path out of the project, or into a project nested in it, is another project's.
    if internal(root, name) || name.split('/').any(|c| c == "..") {
        // A strategy run changes nothing and has no task to ask about (D-93).
        let ask = if strategy_context().is_some() || crate::board::runs::design_context().is_some() {
            String::new()
        } else {
            format!(". {}", crate::scope::ASK_FOR_PROJECT)
        };
        let form = match elsewhere(root, name, false) {
            Some((dir, _, rest)) => format!("; it is {rest} in {dir}: add --project {dir}"),
            None => String::new(),
        };
        return error(
            "E6006",
            name,
            format!("outside this project: another project's files are reached with `--project <folder>`{form}{ask}"),
        );
    }
    path(root, name)
}

/// The project a path names from the Solace root (`solace-cli/strategy/src/x`)
/// when the path isn't in `root`'s own project: the project's folder, its
/// root, and the path inside it (`.` for the folder itself). With `exists`,
/// only when the path is there.
pub(crate) fn elsewhere(root: &Path, name: &str, exists: bool) -> Option<(String, PathBuf, String)> {
    if name.is_empty() || (root.join(name).exists() && !internal(root, name)) {
        return None;
    }
    let top = scope::top(root);
    let here = scope::dir(&top, root);
    let mut members = workspace::members(&top).ok()?;
    members.sort_by_key(|m| std::cmp::Reverse(m.len()));
    members.into_iter().filter(|m| m != "." && *m != here).find_map(|m| {
        let rest = if name == m { ".".to_string() } else { name.strip_prefix(&format!("{m}/"))?.to_string() };
        let there = scope::path(&top, &m);
        (!exists || path(&there, &rest).ok()?.exists()).then_some((m, there, rest))
    })
}

/// A refusal for a path that isn't there: what it was resolved against and,
/// when another project has it, the form that reaches it.
pub(crate) fn not_found(root: &Path, name: &str, what: &str) -> String {
    let top = scope::top(root);
    let mut s = format!("{what} (paths are relative to {})", scope::dir(&top, root));
    if let Some((dir, _, rest)) = elsewhere(root, name, true) {
        s += &format!("; it is {rest} in {dir}: add --project {dir}");
    } else if let Some(rest) = without_project_folder(root, name) {
        let held = Store::peek(root).ok().and_then(|s| {
            let module = module_of_file(&s, &rest)?;
            module_held(&s, &module).then(|| graph_hint(&s, &rest))
        });
        match held {
            Some(hint) => s += &format!("; {hint}"),
            None => s += &format!("; did you mean `{rest}`?"),
        }
    } else {
        match in_one_project(root, name).as_slice() {
            [] => {}
            [dir] => s += &format!("; it is in project {dir}: add --project {dir}"),
            dirs => s += &format!("; it is in projects {}: add --project <dir>", dirs.join(", ")),
        }
    }
    s
}

/// A change to a path that isn't in `root` but names another project from the
/// Solace root is refused: making it here would put a copy inside this project.
pub(crate) fn not_here(root: &Path, name: &str) -> Result<(), Failure> {
    if fs::symlink_metadata(root.join(name)).is_err()
        && let Some((dir, _, rest)) = elsewhere(root, name, false)
    {
        let base = scope::dir(&scope::top(root), root);
        return error("E6002", name, format!("no such file (paths are relative to {base}); it is {rest} in {dir}: add --project {dir}"));
    }
    Ok(())
}

/// The module a new Rust file certainly names (D-181): only paths whose module
/// does not depend on a `mod` declaration.
fn rust_module_of_path(root: &Path, name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("tests/") {
        let stem = rest.strip_suffix(".rs")?;
        return (stem != "mod" && !stem.is_empty() && !stem.contains('/')).then(|| format!("integration::{stem}"));
    }
    let stem = name.strip_prefix("src/")?.strip_suffix(".rs")?;
    if stem == "lib" {
        return Some("crate".to_string());
    }
    if stem == "main" {
        return (!root.join("src/lib.rs").exists()).then(|| "crate".to_string());
    }
    if stem.starts_with("bin/") {
        return None;
    }
    let stem = stem.strip_suffix("/mod").unwrap_or(stem);
    (!matches!(stem, "lib" | "main" | "mod") && !stem.is_empty()).then(|| stem.replace('/', "::"))
}

/// Whether the store holds `module` (D-181).
fn module_held(s: &Store, module: &str) -> bool {
    s.db.query_row("SELECT 1 FROM modules WHERE path = ?1", [module], |_| Ok(())).is_ok()
}

/// The shared hint for a graph path (D-181): read by name when the module is
/// held, and a new module only when its name is certain.
pub(crate) fn graph_hint(s: &Store, name: &str) -> String {
    match module_of_file(s, name) {
        Some(module) if module_held(s, &module) => {
            format!(
                concat!(
                    "source is held as symbols; view {module} --header{sfx} shows its imports and mod lines, ",
                    "view {module} --bodies{sfx} the whole file, or inspect {module}.<symbol> for one item",
                ),
                module = module,
                sfx = project_suffix(s)
            )
        }
        Some(module) => format!("this source file isn't in the store yet: {}", new_module_hint(&module)),
        None => concat!(
            "this source file isn't in the store yet, and Solace can't tell which module ",
            "it would be: submit --new <module> with the module's name"
        )
        .to_string(),
    }
}

/// A graph path refused with its shared hint (D-181).
pub(crate) fn graph_error<T>(s: &Store, name: &str) -> Result<T, Failure> {
    error("E6001", name, graph_hint(s, name))
}

/// The module a path names without consulting the store: Gleam's path under
/// `src/` or `test/`, or a new Rust path whose module is certain (D-181).
pub(crate) fn module_of_path(root: &Path, name: &str) -> Option<String> {
    if name.ends_with(".gleam")
        && let Some(rest) = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/"))
    {
        let module = rest.trim_end_matches(".gleam");
        return Some(if name.starts_with("test/") { test_module_name(module) } else { module.to_string() });
    }
    rust_module_of_path(root, name)
}

/// The submit hint for a new graph path. The caller has already applied Gleam's
/// test suffix when the path is under `test/`.
pub(crate) fn new_module_hint(module: &str) -> String {
    format!("submit --new {module}")
}

/// A Gleam test module's name: the `test/` stem with `_test`, unless it
/// already ends in `_test`.
fn test_module_name(module: &str) -> String {
    if module.ends_with("_test") { module.to_string() } else { format!("{module}_test") }
}

/// ` --project <dir>` when this store is another project under the same root as
/// the one the command runs in; empty otherwise.
fn project_suffix(s: &Store) -> String {
    let top = scope::top(&s.root);
    let Ok(here) = Store::root() else { return String::new() };
    if scope::top(&here) != top {
        return String::new();
    }
    let dir = scope::dir(&top, &s.root);
    if scope::dir(&top, &here) == dir { String::new() } else { format!(" --project {dir}") }
}

/// The path with a leading copy of its project's folder removed, when that
/// shorter path is the one that exists.
fn without_project_folder(root: &Path, name: &str) -> Option<String> {
    let here = scope::dir(&scope::top(root), root);
    let last = here.rsplit('/').next().unwrap_or(here.as_str());
    [here.as_str(), last].into_iter().filter(|p| *p != "." && !p.is_empty()).find_map(|p| {
        let rest = name.strip_prefix(p)?.strip_prefix('/')?;
        (!rest.is_empty() && path(root, rest).ok()?.exists()).then(|| rest.to_string())
    })
}

/// The projects, other than the root's own, that have `name` at the same path.
/// Only a command in the root project looks here.
fn in_one_project(root: &Path, name: &str) -> Vec<String> {
    let top = scope::top(root);
    if scope::dir(&top, root) != "." {
        return vec![];
    }
    workspace::members(&top).unwrap_or_default().into_iter().filter(|d| d != "." && scope::path(&top, d).join(name).exists()).collect()
}

/// Whether `name` is the workspace root's `designs` folder or a path in it.
pub(crate) fn design_path(root: &Path, name: &str) -> bool {
    let top = crate::scope::top(root);
    let place = crate::scope::dir(&top, root);
    let folder = crate::design::DESIGNS_DIR;
    if place.as_str() == folder {
        return false;
    }
    let path = crate::file_name(&Path::new(&place).join(name));
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// A design file changes only through the design agent, never a file command.
pub(crate) fn design_error<T>(name: &str) -> Result<T, Failure> {
    error("E6002", name, "designs change through the design agent; agents can't change a design file directly")
}
