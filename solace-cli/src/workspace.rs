//! The Solace root (D-57): one folder whose projects share one board. Every
//! project is the root itself (`.`) or a folder below it; each keeps its own
//! store, settings, lockdown and tasks. A root with a store and no `gleam.toml`
//! is a project of plain files (D-60).
//!
//! A root is explicit when its `solace.toml` lists `projects`, or when it has a
//! `solace.toml` and no `gleam.toml` (a folder of projects). A project with no
//! root above it is its own root, with itself as its only project.

use super::*;

fn invalid<T>(why: impl std::fmt::Display) -> Result<T, Failure> {
    Err(lockdown::invalid_settings(why))
}

fn has_store(dir: &Path) -> bool {
    dir.join(".solace/store.db").exists()
}

/// The `projects` a folder's `solace.toml` lists, if it lists any.
fn listing(dir: &Path) -> Option<toml::Value> {
    let v: toml::Value = fs::read_to_string(dir.join("solace.toml")).ok()?.parse().ok()?;
    v.get("projects").cloned()
}

/// Whether `dir` is an explicit root.
pub fn is_root(dir: &Path) -> bool {
    dir.join("solace.toml").exists() && (crate::language::detect(dir).is_none() || listing(dir).is_some())
}

/// The Solace root that holds `start`: the one explicit root among its
/// folders, else the outermost project. There is only ever one: a root inside
/// another root, or inside a project, is refused.
pub fn enclosing(start: &Path) -> Result<Option<PathBuf>, Failure> {
    let roots: Vec<&Path> = start.ancestors().filter(|d| is_root(d)).collect();
    match roots.as_slice() {
        [] => Ok(start.ancestors().filter(|d| has_store(d)).last().map(Path::to_path_buf)),
        [r] => match r.ancestors().skip(1).find(|d| has_store(d)) {
            Some(p) => invalid(format!(
                "{} lists projects inside the Solace project {}; list them in that project's solace.toml instead",
                r.display(),
                p.display()
            )),
            None => Ok(Some(r.to_path_buf())),
        },
        [inner, .., outer] => invalid(format!(
            "the Solace root {} is inside the root {}; one root holds all its projects, so list them in the outer one",
            inner.display(),
            outer.display()
        )),
    }
}

/// What `solace serve` serves: the root that holds `start`; with none, `start`
/// itself as an empty root (nothing is written there: existing code is migrated, D-59).
pub fn find(start: &Path) -> Result<PathBuf, Failure> {
    if let Some(root) = enclosing(start)? {
        members(&root)?;
        establish(&root)?;
        return Ok(root);
    }
    // A Gleam project Solace doesn't manage yet is served as it is, read only, so the board can offer to migrate it.
    Ok(start.to_path_buf())
}

/// A project folder as a root names it: relative, `/`-separated, with no `.`
/// or `..`, and reached without symlinks, so it stays under the root.
fn contained(root: &Path, d: &str) -> bool {
    let mut p = root.to_path_buf();
    !d.is_empty()
        && !d.contains('\\')
        && d.split('/').all(|c| {
            p.push(c);
            !c.is_empty() && c != "." && c != ".." && !fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())
        })
}

/// The folders the root's `solace.toml` lists (without `.`).
fn listed(root: &Path) -> Result<Vec<String>, Failure> {
    let text = match fs::read_to_string(root.join("solace.toml")) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let v: toml::Value = text.parse().or_else(invalid)?;
    if crate::language::detect(root).is_none()
        && !has_store(root)
        && let Some(k) = v.as_table().unwrap().keys().find(|k| *k != "projects")
    {
        return invalid(format!("`{k}`: a folder of projects' solace.toml holds only `projects`"));
    }
    let empty = vec![];
    let mut out = vec![];
    for p in v.get("projects").map_or(Some(&empty), |p| p.as_array()).ok_or(()).or_else(|_| invalid("projects"))? {
        match p.as_str() {
            Some(".") => return invalid("`.`: the root's own project is always included; list only folders below it"),
            Some(d) if contained(root, d) && is_root(&root.join(d)) => {
                return invalid(format!("{d} lists projects of its own; one root holds all its projects"));
            }
            Some(d) if contained(root, d) => out.push(d.to_string()),
            _ => return invalid(format!("{p}: expected a folder below the root, reached without symlinks")),
        }
    }
    Ok(out)
}

/// The root's projects: `.` when the root is a set-up project (Gleam or
/// plain files, D-60), then the listed folders.
pub fn members(root: &Path) -> Result<Vec<String>, Failure> {
    let mut out = if has_store(root) { vec![".".to_string()] } else { vec![] };
    out.extend(listed(root)?);
    Ok(out)
}

/// `solace.toml` with its `projects` line set, keeping everything else.
fn with_projects(text: &str, dirs: &[String]) -> String {
    let line = format!("projects = [{}]", dirs.iter().map(|d| json!(d).to_string()).collect::<Vec<_>>().join(", "));
    let lines: Vec<&str> = text.lines().collect();
    let (mut out, mut placed, mut in_table, mut i) = (vec![], false, false, 0);
    while i < lines.len() {
        let t = lines[i].trim_start();
        if t.starts_with('[') && !in_table {
            in_table = true;
            if !placed {
                out.push(line.as_str());
                placed = true;
            }
        }
        if !in_table && t.strip_prefix("projects").is_some_and(|r| r.trim_start().starts_with('=')) {
            // The old value, which may run over several lines.
            let mut depth = 0;
            loop {
                let mut quoted = false;
                for c in lines[i].chars() {
                    match c {
                        '"' => quoted = !quoted,
                        '[' if !quoted => depth += 1,
                        ']' if !quoted => depth -= 1,
                        _ => {}
                    }
                }
                i += 1;
                if depth <= 0 || i >= lines.len() {
                    break;
                }
            }
            if !placed {
                out.push(line.as_str());
                placed = true;
            }
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    if !placed {
        out.push(line.as_str());
    }
    out.join("\n") + "\n"
}

/// Every root is a project (D-60): one without `gleam.toml` is set up as a
/// project of plain files, with its own lane. A root that is an unmanaged
/// Gleam project is left alone; it comes in by migration.
fn establish(root: &Path) -> Result<(), Failure> {
    if crate::language::detect(root).is_none() && !has_store(root) {
        import(root)?;
    }
    Ok(())
}

fn save(root: &Path, dirs: &[String]) -> Result<(), Failure> {
    establish(root)?;
    let file = root.join("solace.toml");
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "# Solace root: the projects in this folder, shown on one board. Each has its own solace.toml.\n".into()
        }
        Err(e) => return Err(e.into()),
    };
    let new = with_projects(&text, dirs);
    if crate::language::detect(root).is_some() || has_store(root) {
        lockdown::parse_settings(&new)?;
    }
    fs::write(&file, &new)?;
    Ok(())
}

/// The package name in a project's `gleam.toml`.
pub fn name(dir: &Path) -> String {
    field(dir, "name").unwrap_or_default()
}

/// The compile target in a project's `gleam.toml` (Gleam's default is
/// `erlang`), or empty for a project with no Gleam code.
pub fn target(dir: &Path) -> String {
    match field(dir, "target") {
        Some(t) => t,
        None if dir.join("gleam.toml").exists() => "erlang".into(),
        None => String::new(),
    }
}

fn field(dir: &Path, key: &str) -> Option<String> {
    fs::read_to_string(dir.join("gleam.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Value>().ok())
        .and_then(|v| v.get(key).and_then(|n| n.as_str()).map(String::from))
}

/// Gleam projects in the root (itself as `.`) and below, skipping build
/// output, dependencies, ignored files and symlinks: `{dir, name, imported, member}`.
pub fn found(root: &Path) -> Result<Vec<Value>, Failure> {
    let members = members(root)?;
    let mut out = vec![];
    // ponytail: depth 5 keeps a stray serve in a home folder quick; raise it if real repos nest deeper.
    let walk = ignore::WalkBuilder::new(root)
        .max_depth(Some(5))
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some("build" | "node_modules" | ".solace")))
        .build();
    for e in walk.flatten() {
        if e.file_name() != "gleam.toml" || !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let dir = e.path().parent().unwrap();
        let rel = dir.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        let rel = if rel.is_empty() { ".".to_string() } else { rel };
        out.push(json!({"dir":rel,"name":name(dir),"imported":has_store(dir),"member":members.contains(&rel)}));
    }
    out.sort_by(|a, b| a["dir"].as_str().cmp(&b["dir"].as_str()));
    Ok(out)
}

/// Whether `inner` lies in `outer`'s Gleam source, where a nested project
/// would be read as modules.
fn in_source(outer: &str, inner: &str) -> bool {
    let prefix = if outer == "." { String::new() } else { format!("{outer}/") };
    ["src", "test"].iter().any(|s| inner.starts_with(&format!("{prefix}{s}/")))
}

/// Lists the project at `dir` again, keeping its store. Existing code without
/// a store comes in by migration (D-59), never in place.
pub fn add(root: &Path, dir: &str) -> Out {
    if !has_store(&scope::path(root, dir)) {
        return fail(format!("{dir} isn't set up. Existing code comes into Solace by migrating it (Migrate this project), not in place."));
    }
    setup(root, dir)
}

/// Sets up the project at `dir` (keeping an existing store) and lists it:
/// for a new project, and for a migration's copy.
pub fn setup(root: &Path, dir: &str) -> Out {
    if dir == "." {
        return Ok(format!("{}: {}", root.display(), import(root)?));
    }
    if !contained(root, dir) {
        return lockdown::error("E6002", dir, "expected a folder below the root, reached without symlinks");
    }
    let mut dirs = listed(root)?;
    if dirs.iter().any(|d| d == dir) {
        return fail(format!("{dir} is already a project of this root."));
    }
    let path = root.join(dir);
    if !path.join("gleam.toml").exists() {
        return fail(format!("{dir} has no gleam.toml."));
    }
    if is_root(&path) {
        return fail(format!("{dir} lists projects of its own; one root holds all its projects."));
    }
    let mut all = dirs.clone();
    if root.join("gleam.toml").exists() && has_store(root) {
        all.push(".".into());
    }
    if let Some(d) = all.iter().find(|d| in_source(d, dir) || in_source(dir, d)) {
        return fail(format!("{dir} and {d} would overlap: a project can't sit inside another's src/ or test/."));
    }
    let out = import(&path)?;
    dirs.push(dir.to_string());
    save(root, &dirs)?;
    Ok(format!("{dir}: {out}"))
}

/// Takes `dir` off the list; its `.solace` stays.
pub fn remove(root: &Path, dir: &str) -> Out {
    if dir == "." {
        return fail("The root's own project always stays on its board.");
    }
    let mut dirs = listed(root)?;
    let before = dirs.len();
    dirs.retain(|d| d != dir);
    if dirs.len() == before {
        return fail(format!("{dir} is not a project of this root."));
    }
    save(root, &dirs)?;
    Ok(format!("Removed {dir} from the board; its Solace store is kept.\n"))
}

/// The folder of the listed project `dir`, once it may be deleted with its
/// files (D-74): `name` is its folder or package name, it is reached without
/// symlinks, and it holds no other listed project.
pub fn deletable(root: &Path, dir: &str, name: &str) -> Result<PathBuf, Failure> {
    let refuse = |why: String| Err(Failure(format!("E7014 {why}"), 1));
    if dir == "." {
        return refuse("the root's own project can't be deleted".into());
    }
    let dirs = listed(root)?;
    if !dirs.iter().any(|d| d == dir) {
        return fail(format!("{dir} is not a project of this root."));
    }
    let path = root.join(dir);
    let package = self::name(&path);
    if name != dir && (package.is_empty() || name != package) {
        return refuse(format!("type the project's name ({dir}) to delete its folder"));
    }
    let mut at = root.to_path_buf();
    for part in dir.split('/') {
        at.push(part);
        if fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return refuse(format!("{dir} is reached through a symlink; delete it yourself"));
        }
    }
    if let Some(other) = dirs.iter().find(|d| d.starts_with(&format!("{dir}/"))) {
        return refuse(format!("{dir} holds the project {other}; remove that one first"));
    }
    Ok(path)
}

/// Gleam's rule for package names: lowercase letters, digits and `_`, starting with a letter.
pub fn package_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A new Gleam project at `dir` (`gleam new`), set up and listed.
pub fn new_project(root: &Path, dir: &str, name: &str, javascript: bool) -> Out {
    if !contained(root, dir) {
        return lockdown::error("E6002", dir, "expected a folder below the root, reached without symlinks");
    }
    if !package_name(name) {
        return fail(format!("{name:?} isn't a Gleam package name: use lowercase letters, digits and _, starting with a letter."));
    }
    if root.join(dir).exists() {
        return fail(format!("{dir} already exists."));
    }
    let mut cmd = Command::new("gleam");
    cmd.args(["new", dir, "--name", name, "--skip-github"]).current_dir(root);
    if javascript {
        cmd.args(["--template", "javascript"]);
    }
    // Inside a repository, the project joins it rather than starting its own.
    if root.ancestors().any(|d| d.join(".git").exists()) {
        cmd.arg("--skip-git");
    }
    let out = cmd.output().map_err(|e| Failure(format!("could not run gleam: {e}"), 1))?;
    if !out.status.success() {
        return fail(format!("gleam new failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
    setup(root, dir)
}

/// `solace create <name> [--javascript] [--yes]`: a brand-new Gleam project
/// in the folder `<name>`, after confirmation (D-59). Existing code is migrated instead.
pub fn create(here: &Path, args: &[&str]) -> Out {
    let names: Vec<&&str> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let [name] = names.as_slice() else {
        return usage("create <name> [--javascript] [--yes]: creates a new Gleam project in the folder <name>");
    };
    if here.join("gleam.toml").exists() && !has_store(here) {
        return fail(
            "This folder is a Gleam project Solace doesn't manage. Existing code comes into Solace by migrating it: run `solace serve` here and choose Migrate this project.",
        );
    }
    let root = enclosing(here)?.unwrap_or_else(|| here.to_path_buf());
    let rel = here.strip_prefix(&root).unwrap().join(name);
    let dir = rel.to_string_lossy().replace('\\', "/");
    if here.join(name).exists() {
        return fail(format!("{} already exists; give a name that isn't taken.", here.join(name).display()));
    }
    let question = format!("Create the Gleam project {name} at {} in the Solace root {}?", here.join(name).display(), root.display());
    if !migrate::confirm(&question, args.contains(&"--yes"))? {
        return Ok("\nNothing was created.\n".into());
    }
    Ok(format!("\n{}", new_project(&root, &dir, name, args.contains(&"--javascript"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    use testing::Scratch;

    fn project(root: &Path, d: &str) {
        fs::create_dir_all(root.join(d).join("src")).unwrap();
        fs::write(root.join(d).join("gleam.toml"), format!("name = \"{}\"\n", d.replace(['/', ' ', '.'], "_"))).unwrap();
    }

    #[test]
    fn members_are_folders_below_the_root() {
        let root = Scratch::new();
        assert!(members(&root).unwrap().is_empty());
        fs::write(root.join("solace.toml"), "projects = [\"a\", \"b/c\"]\n").unwrap();
        assert_eq!(members(&root).unwrap(), ["a", "b/c"]);
        for bad in ["projects = [\"../x\"]", "projects = [\"/x\"]", "projects = [\"a/./b\"]", "projects = [\".\"]", "watch_limit = 1"] {
            fs::write(root.join("solace.toml"), bad).unwrap();
            assert!(members(&root).unwrap_err().0.contains("E6020"), "{bad}");
        }
        // A symlink would lead out of the root.
        let outside = Scratch::new();
        std::os::unix::fs::symlink(&*outside, root.join("link")).unwrap();
        fs::write(root.join("solace.toml"), "projects = [\"link\"]\n").unwrap();
        assert!(members(&root).unwrap_err().0.contains("without symlinks"));
        // Listing a project makes the root a project of plain files too (D-60).
        save(&root, &["x y".into()]).unwrap();
        assert_eq!(members(&root).unwrap(), [".", "x y"]);
    }

    #[test]
    fn listing_keeps_the_rest_of_the_settings() {
        let text = "# keep\nwatch_limit = 10\nprojects = [\n  \"a\",\n  \"b[1]\",\n]\nunwatched = []\n\n[validators]\n# also kept\n";
        let new = with_projects(text, &["c".into()]);
        assert_eq!(new, "# keep\nwatch_limit = 10\nprojects = [\"c\"]\nunwatched = []\n\n[validators]\n# also kept\n");
        assert_eq!(with_projects("a = 1\n[t]\nx = 2\n", &["c".into()]), "a = 1\nprojects = [\"c\"]\n[t]\nx = 2\n");
        assert_eq!(with_projects("a = 1\n", &[]), "a = 1\nprojects = []\n");
    }

    #[test]
    fn found_skips_build_dependencies_and_ignored_folders() {
        let root = Scratch::new();
        for d in ["a", "a/inner", "a/build/packages/dep", "node_modules/x", "ignored", "b"] {
            project(&root, d);
        }
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("solace.toml"), "projects = [\"b\"]\n").unwrap();
        let f = found(&root).unwrap();
        let dirs: Vec<&str> = f.iter().map(|v| v["dir"].as_str().unwrap()).collect();
        assert_eq!(dirs, ["a", "a/inner", "b"]);
        assert_eq!(f[2]["member"], true);
        assert_eq!(f[1]["name"], "a_inner");
    }

    /// D-74: deleting a project's folder is refused unless the name is typed, the
    /// folder is reached without symlinks, and no other listed project is inside it.
    #[test]
    fn deletable_refuses_what_it_should_not_delete() {
        let root = Scratch::new();
        for d in ["a", "a/inner", "b", "real"] {
            project(&root, d);
        }
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        fs::write(root.join("solace.toml"), "projects = [\"a\", \"a/inner\", \"b\"]\n").unwrap();
        let refused = |dir: &str, name: &str, why: &str| {
            let e = deletable(&root, dir, name).unwrap_err();
            assert!(e.0.contains(why), "{dir} {name}: {}", e.0);
            e
        };
        assert!(refused(".", ".", "root's own project").0.contains("E7014"));
        assert!(deletable(&root, "zzz", "zzz").unwrap_err().0.contains("not a project of this root"));
        assert!(refused("b", "other", "type the project's name").0.contains("E7014"));
        assert!(refused("b", "", "type the project's name").0.contains("E7014"));
        assert!(refused("a", "a", "holds the project a/inner").0.contains("E7014"));
        // The folder name, or the package name in its gleam.toml, confirms.
        assert_eq!(deletable(&root, "b", "b").unwrap(), root.join("b"));
        assert_eq!(deletable(&root, "a/inner", "a/inner").unwrap(), root.join("a/inner"));
        assert_eq!(deletable(&root, "a/inner", "a_inner").unwrap(), root.join("a/inner"));
        // A folder with no package name is confirmed by its folder name only.
        fs::write(root.join("b/gleam.toml"), "").unwrap();
        refused("b", "b_pkg", "type the project's name");
        assert!(deletable(&root, "b", "b").is_ok());
        // A symlinked folder never gets this far: `listed` refuses it first, so
        // `deletable`'s own symlink check is a second line of defence.
        fs::write(root.join("solace.toml"), "projects = [\"a\", \"b\", \"link\"]\n").unwrap();
        assert!(deletable(&root, "link", "link").unwrap_err().0.contains("without symlinks"));
    }

    /// `new_project` refuses before it runs `gleam`, so none of these need the toolchain.
    #[test]
    fn new_project_refuses_before_running_gleam() {
        let root = Scratch::new();
        fs::create_dir_all(root.join("taken")).unwrap();
        for dir in ["../x", "/x", "a/../b", "a\\b", ""] {
            assert!(new_project(&root, dir, "fine", false).unwrap_err().0.contains("E6002"), "{dir}");
        }
        for name in ["Bad", "1st", "a-b", "", "a b"] {
            assert!(new_project(&root, "fresh", name, false).unwrap_err().0.contains("isn't a Gleam package name"), "{name}");
        }
        assert!(new_project(&root, "taken", "fine", true).unwrap_err().0.contains("already exists"));
        assert!(package_name("a_1") && !package_name("_a"));
    }

    fn has_gleam() -> bool {
        Command::new("gleam").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    /// The flags `new_project` hands to `gleam new`: the erlang default, `--javascript`,
    /// and `--skip-git` inside a repository. Needs `gleam` on PATH; without it nothing runs.
    #[test]
    fn new_project_passes_the_toolchain_its_flags() {
        if !has_gleam() {
            return;
        }
        // Not inside a repository: the project starts its own.
        let root = Scratch::new();
        new_project(&root, "erl", "erl", false).unwrap();
        assert_eq!(target(&root.join("erl")), "erlang");
        assert!(root.join("erl/.git").exists(), "outside a repository the project starts its own");
        new_project(&root, "js", "js", true).unwrap();
        assert_eq!(target(&root.join("js")), "javascript");
        assert!(members(&root).unwrap().contains(&"js".to_string()), "a new project is listed");
        // Inside a repository it joins that one.
        let repo = Scratch::new();
        fs::create_dir_all(repo.join(".git")).unwrap();
        new_project(&repo, "inner", "inner", false).unwrap();
        assert!(!repo.join("inner/.git").exists(), "inside a repository there is no nested one");
    }

    /// A name `gleam new` rejects comes back with gleam's own output.
    #[test]
    fn new_project_reports_a_failing_gleam_new() {
        if !has_gleam() {
            return;
        }
        let root = Scratch::new();
        // Package names that collide with Gleam's own are refused by the toolchain, not by `package_name`.
        let err = new_project(&root, "gleam", "gleam", false).unwrap_err().0;
        assert!(err.contains("gleam new failed"), "{err}");
        assert!(!members(&root).unwrap().contains(&"gleam".to_string()));
    }

    #[test]
    fn target_defaults_to_erlang_for_a_gleam_project_only() {
        let root = Scratch::new();
        project(&root, "erl");
        project(&root, "js");
        fs::write(root.join("js/gleam.toml"), "name = \"js\"\ntarget = \"javascript\"\n").unwrap();
        fs::create_dir_all(root.join("plain")).unwrap();
        assert_eq!(target(&root.join("erl")), "erlang");
        assert_eq!(target(&root.join("js")), "javascript");
        assert_eq!(target(&root.join("plain")), "");
        assert!(!contained(&root, "a\\b"));
    }

    #[test]
    fn one_root_holds_every_project() {
        let root = Scratch::new();
        for d in ["a", "a/src/x", "a/tools", "inner/b"] {
            project(&root, d);
        }
        // Existing code isn't set up in place (D-59); a new project or a migration's copy is.
        assert!(add(&root, "a").unwrap_err().0.contains("migrating"));
        setup(&root, "a").unwrap();
        assert!(setup(&root, "a").unwrap_err().0.contains("already"));
        assert!(setup(&root, "a/src/x").unwrap_err().0.contains("overlap"));
        assert!(setup(&root, "../a").is_err());
        setup(&root, "a/tools").unwrap();
        assert_eq!(members(&root).unwrap(), [".", "a", "a/tools"]);
        // The outer project's agents can't reach the nested one.
        let v = lockdown::settings(&root.join("a")).unwrap();
        assert_eq!(lockdown::tier(&root.join("a"), &v, "tools/gleam.toml", None), "internal");
        assert_eq!(lockdown::tier(&root.join("a"), &v, "tools", None), "internal");
        assert_eq!(lockdown::tier(&root.join("a"), &v, "gleam.toml", None), "watched");
        // Inside a member, and anywhere below, the root is the same one.
        assert_eq!(enclosing(&root.join("a/tools/src")).unwrap(), Some(root.0.clone()));
        assert_eq!(find(&root.join("a")).unwrap(), root.0);
        // A second root inside it, or a root inside a project, is refused.
        fs::write(root.join("inner/solace.toml"), "projects = [\"b\"]\n").unwrap();
        assert!(enclosing(&root.join("inner/b")).unwrap_err().0.contains("is inside the root"));
        fs::remove_file(root.join("inner/solace.toml")).unwrap();
        fs::create_dir_all(root.join("a/docs")).unwrap();
        fs::write(root.join("a/docs/solace.toml"), "projects = []\n").unwrap();
        assert!(enclosing(&root.join("a/docs")).unwrap_err().0.contains("is inside the root"));
        setup(&root, "inner/b").unwrap();
        assert_eq!(members(&root).unwrap(), [".", "a", "a/tools", "inner/b"]);
        remove(&root, "a").unwrap();
        assert_eq!(members(&root).unwrap(), [".", "a/tools", "inner/b"]);
        assert!(root.join("a/.solace/store.db").exists());
        // A removed project keeps its store, so it can be listed again.
        add(&root, "a").unwrap();
        assert_eq!(members(&root).unwrap(), [".", "a/tools", "inner/b", "a"]);
    }

    #[test]
    fn the_root_can_be_a_project_itself() {
        let root = Scratch::new();
        project(&root, ".");
        project(&root, "frontend");
        fs::write(root.join("src/app.gleam"), "pub fn one() { 1 }\n").unwrap();
        setup(&root, ".").unwrap();
        assert_eq!(enclosing(&root).unwrap(), Some(root.0.clone()));
        assert_eq!(members(&root).unwrap(), ["."]);
        assert!(!is_root(&root));
        // Setting up a project below it makes the root explicit; its settings stay.
        setup(&root, "frontend").unwrap();
        assert!(is_root(&root));
        assert_eq!(members(&root).unwrap(), [".", "frontend"]);
        let settings = fs::read_to_string(root.join("solace.toml")).unwrap();
        assert!(settings.contains("watch_limit = 262144") && settings.contains("projects = [\"frontend\"]"));
        assert!(lockdown::settings(&root).is_ok());
        assert!(remove(&root, ".").is_err());
        assert_eq!(found(&root).unwrap()[0]["dir"], ".");
        // Its agents can't reach the project below it.
        let v = lockdown::settings(&root).unwrap();
        assert_eq!(lockdown::tier(&root, &v, "frontend/gleam.toml", None), "internal");
    }
}
