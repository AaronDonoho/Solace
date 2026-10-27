//! `probe` in a Rust store (D-158): throwaway statements run as a test in a
//! reused copy of the crate under `target/solace-probe/crate/`, so the
//! project's files and test records are never touched.

use super::*;

/// Runs a Rust probe in the crate's reusable scratch copy. A lock serializes
/// runs on it; each run refreshes it and restores only the probe text.
pub(crate) fn run(s: &Store, text: &str, module: Option<&str>) -> Out {
    let base = s.root.join("target").join("solace-probe");
    fs::create_dir_all(&base)?;
    let _lock = fsx::lock_file(&base.join("lock"))?;
    at(s, text, module, &base.join("crate"))
}

/// What a probe needs after its copy is ready: the index, the file and text
/// its errors point at, the command that runs it, and the test it runs.
struct Prepared {
    ix: Index,
    text: String,
    argv: Vec<String>,
    files: Vec<FileMap>,
    file: PathBuf,
    before: String,
    test: String,
}

/// A Rust probe: copy the store's crate to `dir`, add a test module holding
/// `text`, run that test with output, and report compiler errors against the
/// module.
fn at(s: &Store, text: &str, module: Option<&str>, dir: &Path) -> Out {
    let prepared = prepare(s, text, module, dir)?;
    let output = lockdown::command(&s.root, &prepared.argv, 600, usize::MAX, false, true);
    let _ = fs::write(&prepared.file, &prepared.before);
    report(&prepared, module, &output?)
}

fn prepare(s: &Store, text: &str, module: Option<&str>, dir: &Path) -> Result<Prepared, Failure> {
    if let Some(why) = language::reason(language::RUST)? {
        return Err(Failure(format!("E6013 {why}"), 1));
    }
    let ix = s.index()?;
    let manifest: toml::Value =
        fs::read_to_string(s.root.join("Cargo.toml"))?.parse().map_err(|e| Failure(format!("Cargo.toml: {e}"), 1))?;
    if let Some(name) = path_dependency(&manifest) {
        return fail(format!(
            "E1006 probe can't run here: Cargo.toml has a path dependency (`{name}`), which the scratch copy can't reach."
        ));
    }
    if manifest.get("workspace").is_none() {
        if let Some(key) = inherited_key(&manifest) {
            return fail(format!(
                "E1006 probe can't run here: Cargo.toml inherits `{key}` from its workspace, which the scratch copy can't resolve."
            ));
        }
    }
    let root =
        ix.modules.iter().find(|m| m.path == "crate").ok_or_else(|| Failure("E0101 this Rust store has no crate root.".into(), 1))?;
    let target = match module {
        Some(name) => match s.resolve_module(&ix, name) {
            Some(m) => m,
            None => {
                return fail(format!("E1001 unknown module: {name}; `--in` takes a module, e.g. `crate` (`view <module>` lists them)."));
            }
        },
        None => root,
    };
    if target.file.as_deref().is_some_and(|f| f.starts_with("tests/")) {
        return fail(format!("E1006 {} is a test crate; `--in` takes a module of the crate.", target.path));
    }
    let rel = target.rel();
    let mut files = sync(s, &ix, &manifest, dir)?;
    let path = dir.join(&rel);
    let before = fs::read_to_string(&path)?;
    if ix.modules.iter().any(|m| m.path == PROBE || m.path.ends_with(&format!("::{PROBE}"))) || has_inline_probe(&ix, target) {
        return fail(format!("E0101 a project module is named {PROBE}; probe can't run."));
    }
    let code = probe_code(text);
    let separator = if before.ends_with('\n') { "\n" } else { "\n\n" };
    let content = format!("{before}{separator}{code}");
    let start = content[..content.len() - code.len()].matches('\n').count() + 1;
    let wrapper_end = start + code.lines().count() - 1;
    let (first, last) = probe_span(&code, start);
    let mut ranges = vec![];
    if first > start {
        ranges.push((start, first - 1, crate::PROBE_START_RANGE.into()));
    }
    if last >= first {
        ranges.push((first, last, crate::PROBE_RANGE.into()));
    }
    let end_start = last.saturating_add(1);
    if wrapper_end >= end_start {
        ranges.push((end_start, wrapper_end, crate::PROBE_END_RANGE.into()));
    }
    fs::write(&path, &content)?;
    match files.iter_mut().find(|f| f.file == path) {
        Some(map) => map.ranges.extend(ranges),
        None => files.push(FileMap { module: target.path.clone(), file: path.clone(), ranges }),
    }
    let test = if target.path == "crate" { format!("{PROBE}::probe") } else { format!("{}::{PROBE}::probe", target.path) };
    let argv = probe_argv(dir, &s.root.join("target/solace-probe/target"), &manifest, root, &test);
    Ok(Prepared { ix, text: text.to_string(), argv, files, file: path, before, test })
}

fn report(p: &Prepared, module: Option<&str>, output: &str) -> Out {
    let ok = lockdown::passed(output);
    let raw = output.split_once('\n').map_or("", |(_, rest)| rest);
    let diags = cargo::diagnostics(raw, &p.files);
    let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
    let mut out = if errors.is_empty() { clean(raw, &p.files, &p.test) } else { format!("Compile error:\n\n{}", errors.join("\n\n")) };
    if !ok {
        if out.trim().is_empty() {
            out = clean(&cargo::failure_text(raw), &p.files, &p.test);
        }
        if module.is_none() {
            out += &private_hint(&p.ix, &diags);
        }
        out += &format!("\n\nThe probe's text:\n{}", probe_echo(&p.text));
        return fail(timed(&out, output));
    }
    let out = if out.trim().is_empty() { "The probe produced no output.".to_string() } else { out };
    Ok(timed(&out, output))
}

/// The files a build needs beside the crate's modules: the project's watched,
/// protected and unwatched files outside `target/`, and Cargo's settings.
/// The copied manifest is its own workspace root, away from any workspace above.
fn copy_crate(s: &Store, dir: &Path, manifest: &toml::Value) -> Result<Vec<String>, Failure> {
    let mut names = view::copy_files(
        s,
        dir,
        |name, tier| {
            (tier == "watched" || tier == "protected" || tier.starts_with("unwatched"))
                && name != "Cargo.toml"
                && name != "Cargo.lock"
                && name != "solace.toml"
                && name != "target"
                && !name.starts_with("target/")
        },
        &["build.rs", "rust-toolchain", "rust-toolchain.toml", "rustfmt.toml", ".cargo/config", ".cargo/config.toml"],
    )?;
    copy_manifest(s, dir, manifest)?;
    copy_workspace_lock(&s.root, dir)?;
    names.push("Cargo.toml".into());
    Ok(names)
}

/// The probe's `#[cfg(test)]` module: `text` is its test body, and a lone
/// expression is printed with `dbg!` as Gleam prints one with `echo`.
fn probe_code(text: &str) -> String {
    let (body, _) = probe_body(text);
    let body: String = body.lines().map(|line| format!("{}{line}\n", crate::PROBE_INDENT)).collect();
    format!("#[cfg(test)]\nmod {PROBE} {{\n    use super::*;\n\n    #[test]\n    fn probe() {{\n{body}    }}\n}}\n")
}

/// A probe's own output: cargo's progress and libtest's status lines are noise.
fn clean(output: &str, files: &[FileMap], test: &str) -> String {
    let mut out = String::new();
    for line in strip_ansi(output).lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{')
            || test_runner_noise(trimmed, test)
            || cargo_noise(trimmed)
            || (trimmed.starts_with("test ")
                && (trimmed.ends_with(" ... ok") || trimmed.ends_with(" ... FAILED") || trimmed.ends_with(" ... ignored")))
        {
            continue;
        }
        out.push_str(&probe_locations(line, files));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// `cargo test` for one probe test, on the crate's only target and with the
/// test's output shown. `target_dir` is the probe's own build folder, so it
/// outlives the copy without sharing the project's profile artifacts.
fn probe_argv(dir: &Path, target_dir: &Path, manifest: &toml::Value, root: &ModuleRow, test: &str) -> Vec<String> {
    let mut argv: Vec<String> =
        ["cargo", "test", "--offline", "--message-format=json", "--manifest-path"].iter().map(|s| s.to_string()).collect();
    argv.push(dir.join("Cargo.toml").to_string_lossy().to_string());
    argv.push("--target-dir".into());
    argv.push(target_dir.to_string_lossy().to_string());
    let file = root.file.as_deref().unwrap_or("");
    if manifest.get("lib").is_some() || file == "src/lib.rs" {
        argv.push("--lib".into());
    } else {
        let name = if file == "src/main.rs" {
            manifest.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string()
        } else {
            Path::new(file).file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string()
        };
        if !name.is_empty() {
            argv.push("--bin".into());
            argv.push(name);
        }
    }
    argv.extend(["--".into(), "--exact".into(), "--nocapture".into(), test.to_string()]);
    argv
}

/// The first path dependency in a manifest: a scratch copy can't follow it.
fn path_dependency(manifest: &toml::Value) -> Option<String> {
    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut found = vec![];
    for kind in kinds {
        collect_paths(manifest.get(kind), &mut found);
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for spec in targets.values() {
            for kind in kinds {
                collect_paths(spec.get(kind), &mut found);
            }
        }
    }
    if let Some(table) = manifest.get("patch").and_then(toml::Value::as_table) {
        for spec in table.values() {
            collect_paths(Some(spec), &mut found);
        }
    }
    if let Some(table) = manifest.get("replace").and_then(toml::Value::as_table) {
        for (name, spec) in table {
            if spec.get("path").and_then(toml::Value::as_str).is_some() {
                found.push(name.clone());
            }
        }
    }
    found.into_iter().next()
}

fn collect_paths(table: Option<&toml::Value>, out: &mut Vec<String>) {
    let Some(table) = table.and_then(toml::Value::as_table) else { return };
    for (name, spec) in table {
        if spec.get("path").and_then(toml::Value::as_str).is_some() {
            out.push(name.clone());
        }
    }
}

/// When a root probe can't reach a private item, name the `--in` module that can.
fn private_hint(ix: &Index, diags: &[Diag]) -> String {
    let Some(item) = diags.iter().filter(|d| d.error && d.text.contains("E0603")).find_map(|d| d.text.split('`').nth(1)) else {
        return String::new();
    };
    let mut modules: Vec<&str> = ix.syms.iter().filter(|s| s.name == item && s.module != "crate").map(|s| s.module.as_str()).collect();
    modules.sort();
    modules.dedup();
    match modules.as_slice() {
        [module] => format!("\nHint: `{module}.{item}` is private from the crate root; run `probe --in {module}` to reach it.\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(file: &str) -> crate::index::ModuleRow {
        crate::index::ModuleRow {
            path: "crate".into(),
            dir: "src".into(),
            header: String::new(),
            file: Some(file.into()),
            owns_dir: Some(false),
        }
    }

    fn manifest(text: &str) -> toml::Value {
        text.parse().unwrap()
    }

    /// A function whose name starts with `assert` is still a lone expression.
    #[test]
    fn a_lone_expression_is_printed() {
        assert!(probe_code("assert_ok(1)").contains("dbg!(assert_ok(1));"));
        assert!(!probe_code("assert_eq!(1, 1);").contains("dbg!("));
    }

    /// Cargo's build folder outlives the crate copy, so its dependencies are reused.
    #[test]
    fn the_target_dir_outlives_the_copy() {
        let dir = Path::new("/x/target/solace-probe/crate");
        let argv =
            probe_argv(dir, Path::new("/x/target"), &manifest("[package]\nname = \"app\"\n"), &root("src/lib.rs"), "solace_probe::probe");
        let at = argv.iter().position(|a| a == "--target-dir").unwrap();
        assert_eq!(argv[at + 1], "/x/target");
        assert!(!Path::new(&argv[at + 1]).starts_with(dir));
        assert_eq!(
            argv.iter().position(|a| a == "--manifest-path").map(|i| argv[i + 1].as_str()),
            Some("/x/target/solace-probe/crate/Cargo.toml")
        );
    }

    /// A `[lib]` table keeps a moved lib target on `--lib`, not `--bin`.
    #[test]
    fn a_lib_table_runs_the_lib_target() {
        let dir = Path::new("/x/target/solace-probe/run");
        let argv = probe_argv(
            dir,
            Path::new("/x/target"),
            &manifest("[package]\nname = \"app\"\n\n[lib]\npath = \"src/core.rs\"\n"),
            &root("src/core.rs"),
            "solace_probe::probe",
        );
        assert!(argv.contains(&"--lib".to_string()) && !argv.contains(&"--bin".to_string()));
    }

    /// Only dependency tables carry a path dependency; a `[lib] path` is not one.
    #[test]
    fn only_dependency_tables_name_a_path_dependency() {
        assert_eq!(path_dependency(&manifest("[package]\nname = \"app\"\n\n[lib]\npath = \"src/lib.rs\"\n")), None);
        assert_eq!(
            path_dependency(&manifest("[package]\nname = \"app\"\n\n[dependencies]\nx = { path = \"../x\" }\n")).as_deref(),
            Some("x")
        );
        assert_eq!(
            path_dependency(&manifest("[package]\nname = \"app\"\n\n[target.'cfg(unix)'.dev-dependencies]\ny = { path = \"../y\" }\n"))
                .as_deref(),
            Some("y")
        );
        assert_eq!(path_dependency(&manifest("[patch.crates-io]\nz = { path = \"../z\" }\n")).as_deref(), Some("z"));
        assert_eq!(path_dependency(&manifest("[replace]\n\"w:0.1.0\" = { path = \"../w\" }\n")).as_deref(), Some("w:0.1.0"));
    }

    /// A file the store no longer has goes from the reused copy.
    #[test]
    fn stale_files_leave_the_copy() {
        let dir = crate::testing::Scratch::new().with(&[("src/lib.rs", "one\n"), ("src/old.rs", "two\n")]);
        let wanted: HashSet<String> = ["src/lib.rs".to_string()].into_iter().collect();
        remove_stale(&dir, &wanted).unwrap();
        assert!(dir.join("src/lib.rs").is_file());
        assert!(!dir.join("src/old.rs").exists());
    }

    /// A lock cargo made in the copy stays, and a folder emptied by stale files goes.
    #[test]
    fn a_lock_stays_and_an_empty_folder_goes() {
        let dir = crate::testing::Scratch::new().with(&[("Cargo.lock", "lock\n"), ("src/lib.rs", "one\n"), ("src/old/old.rs", "two\n")]);
        let wanted: HashSet<String> = ["src/lib.rs".to_string()].into_iter().collect();
        remove_stale(&dir, &wanted).unwrap();
        assert!(dir.join("Cargo.lock").is_file());
        assert!(dir.join("src/lib.rs").is_file());
        assert!(!dir.join("src/old").exists());
    }

    /// A package, dependency, target or lints key is named by its table.
    #[test]
    fn an_inherited_key_is_named_from_its_table() {
        assert_eq!(inherited_key(&manifest("[package]\nname = \"app\"\nversion.workspace = true\n")).as_deref(), Some("package.version"));
        assert_eq!(
            inherited_key(&manifest("[package]\nname = \"app\"\nversion.workspace = true\n\n[dependencies]\nx = { workspace = true }\n"))
                .as_deref(),
            Some("package.version")
        );
        assert_eq!(
            inherited_key(&manifest("[package]\nname = \"app\"\n\n[dependencies]\nx = { workspace = true }\n")).as_deref(),
            Some("dependencies.x")
        );
        assert_eq!(
            inherited_key(&manifest("[package]\nname = \"app\"\n\n[target.'cfg(unix)'.dev-dependencies]\ny = { workspace = true }\n"))
                .as_deref(),
            Some("target.cfg(unix).dev-dependencies.y")
        );
        assert_eq!(inherited_key(&manifest("[package]\nname = \"app\"\n\n[lints]\nworkspace = true\n")).as_deref(), Some("lints"));
        assert_eq!(inherited_key(&manifest("[package]\nname = \"app\"\n\n[package.metadata]\nworkspace = true\n")), None);
    }

    /// A member that inherits a key is refused before cargo sees the empty workspace.
    #[test]
    fn a_workspace_member_that_inherits_a_key_is_refused() {
        let (root, s) = crate::testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let text = "[package]\nname = \"app\"\nversion.workspace = true\n";
        fs::write(root.join("Cargo.toml"), text).unwrap();
        let dir = crate::testing::Scratch::new();
        let err = prepare(&s, "println!(\"{}\", crate::one());", None, &dir).err().unwrap().0;
        assert!(err.starts_with("E1006") && err.contains("version"), "{err}");
        assert!(fs::read_dir(&*dir).unwrap().next().is_none());
    }

    /// An existing copy keeps its lock, so cargo's trimmed lock survives.
    #[test]
    fn a_project_lock_needs_no_workspace_lookup() {
        let root = crate::testing::Scratch::new().with(&[("Cargo.lock", "project\n")]);
        let dir = crate::testing::Scratch::new().with(&[("Cargo.lock", "copy\n")]);
        copy_workspace_lock(&root, &dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "copy\n");
    }

    /// A member without its own lock takes the workspace's, so the probe keeps it.
    #[test]
    fn a_workspace_members_lock_is_copied() {
        if !crate::language::cargo::installed() {
            return;
        }
        let root = crate::testing::Scratch::new().with(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n"),
            ("Cargo.lock", "lock\n"),
            ("member/Cargo.toml", "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            ("member/src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ]);
        let dir = crate::testing::Scratch::new();
        copy_workspace_lock(&root.join("member"), &dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "lock\n");
    }

    /// A workspace root's own `[workspace]` table can resolve inherited keys.
    #[test]
    fn a_workspace_root_keeps_its_inherited_keys() {
        let (root, s) = crate::testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let text =
            concat!("[package]\nname = \"app\"\nversion.workspace = true\n\n", "[workspace]\n[workspace.package]\nversion = \"0.1.0\"\n",);
        fs::write(root.join("Cargo.toml"), text).unwrap();
        let dir = crate::testing::Scratch::new();
        prepare(&s, "println!(\"{}\", crate::one());", None, &dir).unwrap();
        assert!(dir.join("Cargo.toml").is_file());
    }

    /// A folder the tree lists as one row is copied whole; build output stays out.
    #[test]
    fn left_out_folders_are_copied_and_build_output_is_not() {
        let root = crate::testing::Scratch::new().with(&[
            ("src/lib.rs", "pub fn one() {}\n"),
            ("src/assets/icon.bin", "x"),
            ("src/assets/target/keep.bin", "x"),
            (".git/config", "x"),
            ("frontend/.solace/store.db", "x"),
            ("frontend/build/out.js", "x"),
            ("frontend/node_modules/pkg/index.js", "x"),
            ("frontend/priv/app.js", "x"),
            ("frontend/src/build/x.js", "x"),
            ("frontend/target/stale", "x"),
            ("node_modules/pkg/index.js", "x"),
            ("target/old.txt", "x"),
        ]);
        let dir = crate::testing::Scratch::new();
        let wanted: HashSet<String> = ["src/lib.rs".to_string()].into_iter().collect();
        let names = copy_left_out(&root, &dir, &wanted).unwrap();
        assert_eq!(
            names,
            vec![
                "frontend/priv/app.js".to_string(),
                "frontend/src/build/x.js".to_string(),
                "src/assets/icon.bin".to_string(),
                "src/assets/target/keep.bin".to_string(),
            ]
        );
        assert_eq!(fs::read_to_string(dir.join("frontend/priv/app.js")).unwrap(), "x");
        assert_eq!(fs::read_to_string(dir.join("frontend/src/build/x.js")).unwrap(), "x");
        assert_eq!(fs::read_to_string(dir.join("src/assets/icon.bin")).unwrap(), "x");
        assert_eq!(fs::read_to_string(dir.join("src/assets/target/keep.bin")).unwrap(), "x");
        assert!(!dir.join(".git").exists());
        assert!(!dir.join("frontend/.solace").exists());
        assert!(!dir.join("frontend/build").exists());
        assert!(!dir.join("frontend/node_modules").exists());
        assert!(!dir.join("frontend/target").exists());
        assert!(!dir.join("node_modules").exists());
        assert!(!dir.join("target").exists());
    }

    #[test]
    fn a_symlinked_folder_is_copied() {
        #[cfg(unix)]
        {
            let shared = crate::testing::Scratch::new().with(&[("icon.bin", "x")]);
            let root = crate::testing::Scratch::new();
            std::os::unix::fs::symlink(&*shared, root.join("assets")).unwrap();
            let dir = crate::testing::Scratch::new();
            let names = copy_left_out(&root, &dir, &HashSet::new()).unwrap();
            assert_eq!(names, vec!["assets/icon.bin".to_string()]);
            assert_eq!(fs::read_to_string(dir.join("assets/icon.bin")).unwrap(), "x");
        }
    }

    #[test]
    fn a_symlink_into_the_target_folder_is_not_copied() {
        #[cfg(unix)]
        {
            let root = crate::testing::Scratch::new().with(&[("target/solace-probe/crate/src/lib.rs", "x")]);
            std::os::unix::fs::symlink(root.join("target/solace-probe"), root.join("assets")).unwrap();
            let dir = crate::testing::Scratch::new();
            let names = copy_left_out(&root, &dir, &HashSet::new()).unwrap();
            assert!(names.is_empty());
            assert!(!dir.join("assets").exists());
        }
    }

    #[test]
    fn a_project_lock_is_copied_when_the_copy_has_none() {
        let root = crate::testing::Scratch::new().with(&[("Cargo.lock", "project\n")]);
        let dir = crate::testing::Scratch::new();
        copy_workspace_lock(&root, &dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "project\n");
    }

    /// A lock the project updates replaces the copy's on the next probe.
    #[test]
    fn a_newer_project_lock_replaces_the_copy() {
        let dir = crate::testing::Scratch::new().with(&[("Cargo.lock", "copy\n")]);
        let root = crate::testing::Scratch::new().with(&[("Cargo.lock", "project\n")]);
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::OpenOptions::new().write(true).open(dir.join("Cargo.lock")).unwrap().set_modified(old).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(root.join("Cargo.lock"))
            .unwrap()
            .set_modified(old + std::time::Duration::from_secs(1))
            .unwrap();
        copy_workspace_lock(&root, &dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "project\n");
    }

    /// The range covers the generated body, including a lone expression but not an empty one.
    #[test]
    fn a_probes_range_is_its_text_not_the_wrapper() {
        assert_eq!(probe_span(&probe_code("let x = 1;\nlet y = 2;\n"), 7), (13, 14));
        assert_eq!(probe_span(&probe_code("\n1 + 1\n"), 7), (13, 13));
        let empty = probe_span(&probe_code(""), 7);
        assert!(empty.1 < empty.0, "{empty:?}");
    }

    /// Runtime and panic locations in the probe name its line; project code keeps its own.
    #[test]
    fn probe_runtime_locations_use_the_probes_lines() {
        let files = vec![FileMap {
            module: "crate".into(),
            file: PathBuf::from("/x/src/lib.rs"),
            ranges: vec![(8, 9, "crate.one".into()), (15, 17, crate::PROBE_RANGE.into())],
        }];
        let output = concat!(
            "[src/lib.rs:15:9] probe_code(\"1\").len() = 105\n",
            "thread 'solace_probe::probe' panicked at src/lib.rs:17:18:\n",
            "panicked at src/lib.rs:15:\n",
            "panicked at src/lib.rs:8:5:\n",
            "missing at src/lib.rs:no\n",
        );
        let out = clean(output, &files, "solace_probe::probe");
        assert!(out.contains("[probe, line 1] probe_code"), "{out}");
        assert!(out.contains("panicked at probe, line 3:"), "{out}");
        assert!(out.contains("panicked at probe, line 1:"), "{out}");
        assert!(out.contains("panicked at src/lib.rs:8:5:"), "{out}");
        assert!(out.contains("missing at src/lib.rs:no"), "{out}");
        assert!(!out.contains("src/lib.rs:15:9"), "{out}");
    }

    /// A failed probe echoes the probe's text, not its generated wrapper.
    #[test]
    fn a_probes_echo_numbers_its_own_lines() {
        assert_eq!(probe_echo("let x = 1;\nlet y = 2;\n"), "1 | let x = 1;\n2 | let y = 2;\n");
        assert_eq!(probe_echo("\n1 + 1\n"), "1 | 1 + 1\n");
        assert!(!probe_echo("let x = 1;\n").contains("mod solace_probe"));
    }

    /// A failed probe keeps its output and panic, without libtest's summary
    /// or cargo's rerun line.
    #[test]
    fn a_failed_probes_test_runner_lines_are_noise() {
        let files = vec![];
        for test in ["solace_probe::probe", "parser::solace_probe::probe"] {
            let output = format!(
                concat!(
                    "before\n\n",
                    "thread '{test}' (7) panicked at src/lib.rs:9:5:\n",
                    "assertion `left == right` failed: boom\n",
                    "  left: 0\n right: 1\n",
                    "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n",
                    "failures:\n\n",
                    "---- {test} stdout ----\n\n",
                    "failures:\n",
                    "    {test}\n\n",
                    "error: test failed, to rerun pass `--bin app`\n",
                ),
                test = test
            );
            let expected = format!(
                "before\n\nthread '{test}' (7) panicked at src/lib.rs:9:5:\n\
                 assertion `left == right` failed: boom\n  left: 0\n right: 1"
            );
            assert_eq!(clean(&output, &files, test), expected);
        }
    }

    /// A probe's own output that begins like cargo or libtest keeps its line;
    /// only their exact forms are dropped.
    #[test]
    fn a_probes_own_output_is_not_mistaken_for_the_runners() {
        let output = concat!(
            "running the parser\n",
            "running one test\n",
            "running 1 test\n",
            "running 2 tests\n",
            "Finished parsing\n",
            "Finished `test` profile [unoptimized + debuginfo] target(s) in 0.01s\n",
            "Finished dev [unoptimized + debuginfo] target(s) in 0.01s\n",
            "Compiling notes v1\n",
            "Compiling parser v0.1.0 (/x)\n",
            "Running the parser\n",
            "Running unittests src/lib.rs (target/debug/deps/probe-1)\n",
            "Running `target/debug/deps/probe-1`\n",
            "test result: uncertain\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "Doc-tests are useful\n",
            "Doc-tests probe\n",
            "Blocking waiting room\n",
            "Blocking waiting for file lock on build directory\n",
            "warning: build failed to parse\n",
            "warning: build failed, waiting for other jobs to finish...\n",
        );
        let expected = concat!(
            "running the parser\n",
            "running one test\n",
            "Finished parsing\n",
            "Compiling notes v1\n",
            "Running the parser\n",
            "test result: uncertain\n",
            "Doc-tests are useful\n",
            "Blocking waiting room\n",
            "warning: build failed to parse",
        );
        assert_eq!(clean(output, &[], "solace_probe::probe"), expected);
    }

    /// Every path through `report` ends with the command's elapsed time.
    #[test]
    fn a_reply_ends_with_the_probes_time() {
        let p = Prepared {
            ix: crate::index::test_index(&[]),
            text: "1 + 1".into(),
            argv: vec![],
            files: vec![],
            file: PathBuf::new(),
            before: String::new(),
            test: "solace_probe::probe".into(),
        };
        assert_eq!(report(&p, None, "exit code: 0 (1.2s)\n1 + 1 = 2\n").unwrap(), "1 + 1 = 2\n\n(1.2s)");
        assert_eq!(report(&p, None, "exit code: 0 (0.4s)\n").unwrap(), "The probe produced no output.\n\n(0.4s)");
        let panic =
            report(&p, None, "exit code: 101 (2.5s)\nthread 'solace_probe::probe' panicked at src/lib.rs:2:1:\nboom\n").unwrap_err();
        assert!(panic.0.ends_with("\n\n(2.5s)"), "{}", panic.0);
        let compile = report(&p, None, "exit code: 1 (3.0s)\n").unwrap_err();
        assert!(compile.0.ends_with("\n\n(3.0s)"), "{}", compile.0);
        let untimed = report(&p, None, "exit code: 0\n1\n").unwrap_err().0;
        assert!(!untimed.ends_with("()"), "{untimed}");
    }
}

const STATEMENT_STARTS: &[&str] = &[
    "let ",
    "use ",
    "fn ",
    "pub ",
    "mod ",
    "struct ",
    "enum ",
    "impl ",
    "trait ",
    "type ",
    "const ",
    "static ",
    "if ",
    "for ",
    "while ",
    "loop ",
    "match ",
    "return ",
    "break ",
    "continue ",
    "assert!",
    "assert_eq!",
    "assert_ne!",
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "dbg!",
    "panic!",
    "todo!",
    "unimplemented!",
    "unreachable!",
];

/// The copy's files as the store and project have them now. Files no longer
/// wanted are removed, so a deleted module can't linger in a later build, and
/// folders left empty go.
fn sync(s: &Store, ix: &Index, manifest: &toml::Value, dir: &Path) -> Result<Vec<FileMap>, Failure> {
    fs::create_dir_all(dir)?;
    let files = s.materialize_to(ix, dir, &HashMap::new())?;
    let mut wanted: HashSet<String> = ix.modules.iter().map(ModuleRow::rel).collect();
    wanted.extend(copy_crate(s, dir, manifest)?);
    let left_out = copy_left_out(&s.root, dir, &wanted)?;
    wanted.extend(left_out);
    remove_stale(dir, &wanted)?;
    Ok(files)
}

/// Removes files no longer wanted and folders left empty. The lock stays even
/// when it is not wanted, so cargo does not resolve dependencies again.
fn remove_stale(dir: &Path, wanted: &HashSet<String>) -> Result<(), Failure> {
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(here) = dirs.pop() {
        for entry in fs::read_dir(&here)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                dirs.push(path);
                continue;
            }
            let rel = relative_name(dir, &path);
            if rel != "Cargo.lock" && !wanted.contains(&rel) {
                fs::remove_file(&path)?;
            }
        }
    }
    remove_empty_dirs(dir, dir)?;
    Ok(())
}

fn copy_manifest(s: &Store, dir: &Path, manifest: &toml::Value) -> Result<(), Failure> {
    let mut text = fs::read_to_string(s.root.join("Cargo.toml"))?;
    if manifest.get("workspace").is_none() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("\n[workspace]\n");
    }
    write_if_changed(&dir.join("Cargo.toml"), &text)
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), Failure> {
    if fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

/// The project's lock, or a member's at the workspace root, is copied when the
/// copy has none or an older one, so a probe resolves the dependencies the
/// project last did. Cargo keeps its trimmed copy up to date, so a later probe
/// leaves that copy alone.
fn copy_workspace_lock(root: &Path, dir: &Path) -> Result<(), Failure> {
    let to = dir.join("Cargo.lock");
    let own = root.join("Cargo.lock");
    let from = if own.is_file() { own } else { crate::language::cargo::lock_file(root) };
    if !from.is_file() {
        return Ok(());
    }
    if to.is_file() && fs::metadata(&from)?.modified()? <= fs::metadata(&to)?.modified()? {
        return Ok(());
    }
    fs::copy(&from, &to)?;
    Ok(())
}

/// Removes the empty folders under `root`, not `root` itself.
fn remove_empty_dirs(dir: &Path, root: &Path) -> Result<(), Failure> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_empty_dirs(&path, root)?;
        }
    }
    if dir != root && fs::read_dir(dir)?.next().is_none() {
        fs::remove_dir(dir)?;
    }
    Ok(())
}

/// The first key a workspace member inherits with `workspace = true`.
fn inherited_key(manifest: &toml::Value) -> Option<String> {
    if let Some(package) = manifest.get("package").and_then(toml::Value::as_table) {
        for key in [
            "authors",
            "categories",
            "description",
            "documentation",
            "edition",
            "exclude",
            "homepage",
            "include",
            "keywords",
            "license",
            "license-file",
            "publish",
            "readme",
            "repository",
            "rust-version",
            "version",
        ] {
            if package.get(key).is_some_and(inherits_from_workspace) {
                return Some(format!("package.{key}"));
            }
        }
    }
    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    for kind in kinds {
        if let Some(key) = collect_inherited_keys(manifest.get(kind), kind) {
            return Some(key);
        }
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (spec, table) in targets {
            for kind in kinds {
                if let Some(key) = collect_inherited_keys(table.get(kind), &format!("target.{spec}.{kind}")) {
                    return Some(key);
                }
            }
        }
    }
    if manifest.get("lints").is_some_and(inherits_from_workspace) {
        return Some("lints".into());
    }
    None
}

fn collect_inherited_keys(table: Option<&toml::Value>, prefix: &str) -> Option<String> {
    let table = table.and_then(toml::Value::as_table)?;
    for (name, spec) in table {
        if inherits_from_workspace(spec) {
            return Some(format!("{prefix}.{name}"));
        }
    }
    None
}

fn inherits_from_workspace(value: &toml::Value) -> bool {
    value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
}

/// Copies the folders `tree_files` reports as one row, or leaves out as another
/// project, so a build script and `include_*` see every path beside the crate.
fn copy_left_out(root: &Path, dir: &Path, wanted: &HashSet<String>) -> Result<Vec<String>, Failure> {
    let mut names = vec![];
    let mut seen = HashSet::new();
    if let Ok(canonical) = root.canonicalize() {
        seen.insert(canonical);
    }
    let target = root.join("target").canonicalize().ok();
    copy_left_out_in(root, dir, root, wanted, target.as_deref(), &mut seen, &mut names)?;
    Ok(names)
}

fn copy_left_out_in(
    root: &Path,
    dir: &Path,
    from: &Path,
    wanted: &HashSet<String>,
    target: Option<&Path>,
    seen: &mut HashSet<std::path::PathBuf>,
    names: &mut Vec<String>,
) -> Result<(), Failure> {
    if probe_skips_target(target, &from.canonicalize()?) {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(from)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from_path = entry.path();
        let kind = entry.file_type()?;
        if !kind.is_dir() && !(kind.is_symlink() && from_path.is_dir()) {
            continue;
        }
        let name = relative_name(root, &from_path);
        if !probe_copies_dir(&name) {
            continue;
        }
        if wanted.iter().any(|file| file.starts_with(&format!("{name}/"))) {
            copy_left_out_in(root, dir, &from_path, wanted, target, seen, names)?;
        } else {
            let nested = from_path.join(".solace/store.db").exists();
            copy_missing_tree(root, &from_path, &dir.join(&name), nested, target, seen, names)?;
        }
    }
    Ok(())
}

/// Solace's stores, version control and installed dependencies stay out.
fn probe_copies_dir(name: &str) -> bool {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    !matches!(leaf, ".solace" | ".git" | ".hg" | ".svn" | "node_modules")
}

fn copy_missing_tree(
    root: &Path,
    from: &Path,
    to: &Path,
    nested: bool,
    target: Option<&Path>,
    seen: &mut HashSet<std::path::PathBuf>,
    names: &mut Vec<String>,
) -> Result<(), Failure> {
    let canonical = from.canonicalize()?;
    if probe_skips_target(target, &canonical) || !seen.insert(canonical) {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    let mut entries: Vec<_> = fs::read_dir(from)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        let name = relative_name(root, &path);
        let child_nested = nested || path.join(".solace/store.db").exists();
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            if probe_copies_dir(&name) && !(child_nested && probe_skips_in_nested(root, &path)) {
                copy_missing_tree(root, &path, &to.join(entry.file_name()), child_nested, target, seen, names)?;
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        view::copy_if_changed(&path, &to.join(entry.file_name()))?;
        names.push(name);
    }
    Ok(())
}

/// A nested project's own build output stays out of the probe copy.
fn probe_skips_in_nested(root: &Path, path: &Path) -> bool {
    let leaf = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let is_build = matches!(leaf, "target" | "build");
    is_build && path.parent().is_some_and(|parent| parent != root && parent.join(".solace/store.db").exists())
}

fn relative_name(root: &Path, path: &Path) -> String {
    crate::file_name(path.strip_prefix(root).unwrap_or(path))
}

/// The probe's own `target` folder stays out, even through a link.
fn probe_skips_target(target: Option<&Path>, canonical: &Path) -> bool {
    target.is_some_and(|target| canonical.starts_with(target))
}

/// The first and last lines of the generated body, which starts at `start`.
fn probe_span(code: &str, start: usize) -> (usize, usize) {
    let lines: Vec<&str> = code.lines().collect();
    let body = lines.iter().position(|line| line.trim() == "fn probe() {").unwrap_or(5);
    let close = lines.iter().rposition(|line| *line == "    }").unwrap_or(lines.len().saturating_sub(1));
    let first = start + body + 1;
    (first, start + close.saturating_sub(1))
}

/// A runtime or panic location inside the probe as `probe, line N`; other code keeps its own.
fn probe_locations(line: &str, files: &[FileMap]) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(dot) = rest.find(".rs:") {
        let before = &rest[..dot];
        let start = before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace() || matches!(c, '[' | '(' | '"' | '\''))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let Some((path, row, _column, len)) = cargo::parse_location(&rest[start..]) else {
            out.push_str(&rest[..dot + 4]);
            rest = &rest[dot + 4..];
            continue;
        };
        let end = start + len;
        match crate::locate(files, path, row) {
            Some((_, Some((name, first)))) if name == crate::PROBE_RANGE => {
                out.push_str(&rest[..start]);
                out.push_str(&format!("probe, line {}", row + 1 - first));
                rest = &rest[end..];
            }
            _ => {
                out.push_str(&rest[..end]);
                rest = &rest[end..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The probe's text with the line numbers its diagnostics use.
fn probe_echo(text: &str) -> String {
    let (_, shown) = probe_body(text);
    let lines: Vec<&str> = shown.lines().collect();
    let width = lines.len().to_string().len();
    lines.iter().enumerate().map(|(i, line)| format!("{:>width$} | {line}\n", i + 1, width = width)).collect()
}

/// The generated test body and the text its echo shows: a lone expression is
/// wrapped in `dbg!` for the body but echoed as written.
fn probe_body(text: &str) -> (String, &str) {
    let one = text.trim();
    let statement = one.is_empty()
        || one.contains('\n')
        || one.ends_with(';')
        || STATEMENT_STARTS.iter().any(|k| one.starts_with(k))
        || one.starts_with('#');
    if statement { (text.to_string(), text) } else { (format!("dbg!({one});"), one) }
}

/// Libtest's run header, result summary, failure summary and backtrace hint,
/// and cargo's test-failure line, name a runner a probe reply can't use.
fn test_runner_noise(line: &str, test: &str) -> bool {
    let running = line
        .strip_prefix("running ")
        .and_then(|rest| rest.strip_suffix(" tests").or_else(|| rest.strip_suffix(" test")))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    running
        || line.starts_with("test result: ok.")
        || line.starts_with("test result: FAILED.")
        || line == "failures:"
        || line == test
        || line == "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace"
        || line.starts_with("error: test failed, to rerun pass `")
        || (line.starts_with("---- ") && line.ends_with(" stdout ----"))
}

/// Cargo's progress lines, in the forms cargo prints. A probe's own line that
/// begins with the same word still reaches the human.
fn cargo_noise(line: &str) -> bool {
    let compiling = line.strip_prefix("Compiling ").is_some_and(|rest| {
        let mut words = rest.split_whitespace();
        words.next().is_some()
            && words.next().is_some_and(|version| {
                version.strip_prefix('v').is_some_and(|v| v.contains('.') && v.starts_with(|c: char| c.is_ascii_digit()))
            })
    });
    let finished = line.strip_prefix("Finished ").is_some_and(|rest| {
        if let Some(rest) = rest.strip_prefix('`') {
            rest.split_once('`').is_some_and(|(profile, after)| !profile.is_empty() && after.starts_with(" profile "))
        } else {
            let Some((profile, after)) = rest.split_once(" [") else { return false };
            !profile.is_empty() && profile.chars().all(|c| !c.is_whitespace()) && after.contains("] target(s) in ")
        }
    });
    let running = line.strip_prefix("Running ").is_some_and(|rest| {
        ["unittests ", "tests/", "benches/", "examples/"].iter().any(|kind| rest.starts_with(kind))
            || (rest.starts_with('`') && rest.ends_with('`'))
    });
    let doc_tests =
        line.strip_prefix("Doc-tests ").is_some_and(|package| !package.is_empty() && package.chars().all(|c| !c.is_whitespace()));
    compiling
        || finished
        || running
        || doc_tests
        || line.starts_with("Blocking waiting for file lock")
        || line == "warning: build failed"
        || line.starts_with("warning: build failed, waiting for other jobs to finish")
}

/// Whether `target` declares `solace_probe` inline; the probe adds its own
/// module of that name beside the target's items.
fn has_inline_probe(ix: &Index, target: &ModuleRow) -> bool {
    ix.syms.iter().any(|s| s.module == target.path && s.parent.is_none() && s.kind_text == "mod" && s.name == PROBE)
}

/// `out` ending with the time `lockdown::command` measured.
fn timed(out: &str, output: &str) -> String {
    let time = output.lines().next().and_then(|line| line.rsplit_once('(')).and_then(|(_, time)| time.strip_suffix(')'));
    match time {
        Some(time) => format!("{}\n\n({time})", out.trim_end()),
        None => out.trim_end().to_string(),
    }
}
