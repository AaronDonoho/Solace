use super::*;
use crate::testing::Scratch;

fn invalid(text: &str) -> String {
    lockdown::parse_settings(text).unwrap_err().0
}

/// D-131: each fault in `[build]` is E6020 naming its field.
#[test]
fn a_bad_build_section_names_its_field() {
    for (text, field) in [
        ("[build]\nsteps = []", "build.steps:"),
        ("[build]\nsteps = [{ run = [\"x\"] }]", "build.steps[0].name"),
        ("[build]\nsteps = [{ name = \"a\" }]", "needs `run`"),
        ("[build]\nsteps = [{ name = \"a\", run = [\"x\"], command = \"c\" }]", "not both"),
        ("[build]\nsteps = [{ name = \"a\", run = [] }]", "build.steps[0].run"),
        ("[build]\nsteps = [{ name = \"a\", run = [\"x\"] }, { name = \"a\", run = [\"y\"] }]", "build.steps[1].name"),
        ("[build]\nsteps = [{ name = \"a\", command = \"nope\" }]\n[commands]\nok = { run = [\"x\"] }", "has no `nope`; declared: ok"),
        ("[build]\nsteps = [{ name = \"a\", run = [\"x\"], timeout = 0 }]", "build.steps[0].timeout"),
        ("[build]\nafter = \"board\"\nsteps = [{ name = \"a\", run = [\"x\"] }]", "build.after:"),
        ("[build]\noutputs = [\"../out\"]\nsteps = [{ name = \"a\", run = [\"x\"] }]", "build.outputs[0]"),
        ("[build]\nsteps = [{ name = \"a\", run = [\"x\"] }]\nwhen = 1", "build.when"),
    ] {
        let e = invalid(text);
        assert!(e.starts_with("E6020") && e.contains(field), "{text}: {e}");
    }
    assert!(lockdown::parse_settings("[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"a\", run = [\"x\"], timeout = 600 }]").is_ok());
}

/// A root of folders `a`, `b` and `c`, each with `files`. One with a
/// `solace.toml` gets a `gleam.toml` too, so it is a project, not a root.
fn workspace(files: &[(&str, &str)]) -> Scratch {
    let mut all = vec![("solace.toml", "projects = [\"a\", \"b\", \"c\"]\n")];
    for d in ["a", "b", "c"] {
        if files.iter().any(|(p, _)| *p == format!("{d}/solace.toml")) {
            all.push(match d {
                "a" => ("a/gleam.toml", "name = \"a\"\n"),
                "b" => ("b/gleam.toml", "name = \"b\"\n"),
                _ => ("c/gleam.toml", "name = \"c\"\n"),
            });
        }
        if !files.iter().any(|(p, _)| p.starts_with(&format!("{d}/"))) {
            all.push(match d {
                "a" => ("a/README.md", ""),
                "b" => ("b/README.md", ""),
                _ => ("c/README.md", ""),
            });
        }
    }
    all.extend(files);
    Scratch::new().with(&all)
}

fn recipe(r: &str) -> String {
    format!("[build]\n{r}")
}

/// `after` names other projects of the root, not the project itself.
#[test]
fn after_names_projects_of_the_workspace() {
    let top = workspace(&[]);
    let v = |t: &str| lockdown::parse_settings(&recipe(t)).unwrap();
    assert!(check_after(&top, "a", &v("after = [\"b\", \"c/\"]\nsteps = [{ name = \"x\", run = [\"x\"] }]")).is_ok());
    let e = check_after(&top, "a", &v("after = [\"d\"]\nsteps = [{ name = \"x\", run = [\"x\"] }]")).unwrap_err().0;
    assert!(e.contains("build.after[0]: `d` is not a project") && e.contains("a, b, c"), "{e}");
    let e = check_after(&top, "a", &v("after = [\"a\"]\nsteps = [{ name = \"x\", run = [\"x\"] }]")).unwrap_err().0;
    assert!(e.contains("is this project"), "{e}");
}

fn runs(r: &Recipe) -> Vec<String> {
    r.steps.iter().map(|s| s.run.join(" ")).collect()
}

/// D-131 point 3: each kind of project gets its default; one with none is skipped.
#[test]
fn each_project_kind_gets_its_default() {
    let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\ntarget = \"javascript\"\n")]);
    assert_eq!(runs(&default(&gleam, "g").unwrap()), ["gleam build --target javascript"]);
    let erlang = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n")]);
    assert_eq!(runs(&default(&erlang, "e").unwrap()), ["gleam build --target erlang"]);
    let cargo = Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
    assert_eq!(runs(&default(&cargo, "r").unwrap()), ["cargo build --release --locked"]);
    let bun = Scratch::new().with(&[("package.json", "{\"scripts\":{\"build\":\"vite build\"}}"), ("bun.lock", "")]);
    assert_eq!(runs(&default(&bun, "b").unwrap()), ["bun install", "bun run build"]);
    let npm = Scratch::new().with(&[("package.json", "{\"scripts\":{\"build\":\"x\"}}"), ("package-lock.json", "{}")]);
    assert_eq!(runs(&default(&npm, "n").unwrap()), ["npm ci", "npm run build"]);
    let bare = Scratch::new().with(&[("package.json", "{}")]);
    assert_eq!(runs(&default(&bare, "p").unwrap()), ["npm install"]);
    // The board's and frontend's shape: packages installed before the Gleam build.
    let both = Scratch::new().with(&[("package.json", "{}"), ("bun.lock", ""), ("gleam.toml", "name = \"x\"\ntarget = \"javascript\"\n")]);
    let r = default(&both, "board").unwrap();
    assert_eq!(runs(&r), ["bun install", "gleam build --target javascript"]);
    assert!(!r.declared && r.steps.iter().all(|s| s.timeout == 120));
    let none = Scratch::new().with(&[("README.md", "")]);
    assert_eq!(default(&none, "docs"), None);
}

/// Projects come after those they name in `after`, else in the root's order.
#[test]
fn the_plan_follows_after() {
    let top = workspace(&[
        ("a/solace.toml", "[build]\nafter = [\"c\"]\nsteps = [{ name = \"x\", run = [\"x\"] }]\n"),
        ("b/README.md", ""),
        ("c/Cargo.toml", "[package]\nname = \"c\"\n"),
    ]);
    let plan = plan(&top).unwrap();
    let order: Vec<(&str, bool)> = plan.iter().map(|r| (r.project.as_str(), r.declared)).collect();
    assert_eq!(order, [("c", false), ("a", true)]);
    let text = plan_text(&top).unwrap();
    assert!(text.contains("1. c (default)") && text.contains("2. a (declared)\n   after: c\n   - x: x"), "{text}");
    let page = plan_json(&top).unwrap();
    assert_eq!(page[1]["source"], "declared");
    assert_eq!(page[0]["steps"][0]["run"], json!(["cargo", "build", "--release", "--locked"]));
}

/// A cycle in `after` is refused, naming its projects.
#[test]
fn a_cycle_is_refused_and_named() {
    let step = "steps = [{ name = \"x\", run = [\"x\"] }]\n";
    let top = workspace(&[
        ("a/solace.toml", &format!("[build]\nafter = [\"b\"]\n{step}")),
        ("b/solace.toml", &format!("[build]\nafter = [\"c\"]\n{step}")),
        ("c/solace.toml", &format!("[build]\nafter = [\"a\"]\n{step}")),
    ]);
    let e = plan(&top).unwrap_err().0;
    assert!(e.starts_with("E6020 build order has a cycle: a → b → c → a"), "{e}");
    let top = workspace(&[("a/solace.toml", &format!("[build]\nafter = [\"d\"]\n{step}"))]);
    let e = plan(&top).unwrap_err().0;
    assert!(e.contains("`d` is not a project") && e.contains("in a/solace.toml"), "{e}");
}

/// A `command =` step reuses a declared command's run and timeout.
#[test]
fn a_step_reuses_a_declared_command() {
    let top = workspace(&[(
        "a/solace.toml",
        "[commands]\nbuild-release = { run = [\"sh\", \"-c\", \"make\"], timeout = 900 }\n[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"release\", command = \"build-release\" }]\n",
    )]);
    let r = of(&top, "a").unwrap().unwrap();
    assert_eq!(r.steps, [Step { name: "release".into(), run: vec!["sh".into(), "-c".into(), "make".into()], command: Some("build-release".into()), timeout: 900 }]);
    assert_eq!(r.outputs, ["priv/app.js"]);
    assert!(plan_text(&top).unwrap().contains("- release (command build-release): sh -c make [timeout 900 s]"));
}

/// D-159: Solace's own crate bundles the frontend, then builds itself. The
/// board's bundle serves its standalone page, so the board keeps the plan's default, not a release step.
#[test]
fn the_self_build_bundles_before_the_crate() {
    let top = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let frontend = of(top, "solace-cli/frontend").unwrap().unwrap();
    let root = of(top, ".").unwrap();
    assert!(root.is_none(), "the root has no build of its own");
    let crate_ = of(top, "solace-cli").unwrap().unwrap();
    assert!(frontend.declared && frontend.outputs == ["priv/app.js"], "{frontend:?}");
    assert_eq!(
        frontend.steps.iter().map(|s| (s.name.as_str(), s.command.as_deref(), s.timeout)).collect::<Vec<_>>(),
        [("release", Some("build-release"), 600)]
    );
    assert!(frontend.after.is_empty());
    assert_eq!(crate_.after, ["solace-cli/frontend"]);
    assert_eq!(crate_.steps[0].run[..2], ["cargo", "build"]);
    assert!(!of(top, "solace-cli/board").unwrap().unwrap().declared, "the board's bundle is not in the plan");
}

/// `build plan` answers through the command dispatch and runs nothing; `build` takes no other argument.
#[test]
fn build_plan_is_a_command() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let v = lockdown::settings(&p.root).unwrap();
    let mut input = || -> Out { Ok(String::new()) };
    let out = lockdown::dispatch(&p, &v, &["build", "plan"], &mut input, None, BUDGET).unwrap().unwrap();
    assert!(out.contains("1. . (default)") && out.contains("- gleam: gleam build --target erlang"), "{out}");
    assert!(!p.root.join("build").exists());
    let e = lockdown::dispatch(&p, &v, &["build", "now"], &mut input, None, BUDGET).unwrap().unwrap_err().0;
    assert!(e.contains("`build plan`"), "{e}");
}

/// Sets `path`'s modification time.
fn touch(path: &Path, t: std::time::SystemTime) {
    fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

/// D-159: `build plan` marks a declared output out of date when it is missing, or when a source
/// of its project or of a path dependency changed more than 5 s after it was built.
#[test]
fn the_plan_marks_outputs_older_than_their_sources() {
    let top = Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/solace.toml", "[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"release\", run = [\"x\"] }]\n"),
        ("a/gleam.toml", "name = \"a\"\n[dependencies]\nb = { path = \"../b\" }\n"),
        ("a/src/a.gleam", ""),
        ("a/priv/app.js", ""),
        ("b/src/b.gleam", ""),
    ]);
    let now = std::time::SystemTime::now();
    let built = now + std::time::Duration::from_secs(60);
    touch(&top.join("a/priv/app.js"), built);
    let marked = || plan_text(&top).unwrap().lines().find(|l| l.contains("out of date")).map(String::from);
    assert_eq!(marked(), None);
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], Value::Null);
    touch(&top.join("b/src/b.gleam"), built + std::time::Duration::from_secs(4));
    assert_eq!(marked(), None, "within 5 s is the same build");
    touch(&top.join("b/src/b.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(marked().as_deref(), Some("   out of date: b/src changed after priv/app.js was built: run step `release`"));
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], "b/src changed after priv/app.js was built");
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert!(marked().unwrap().contains("a/src, b/src changed"));
    fs::remove_file(top.join("a/priv/app.js")).unwrap();
    assert_eq!(marked().as_deref(), Some("   out of date: priv/app.js is missing: run step `release`"));
}

/// A root of `a` and `b` where `a` builds `priv/app.js` from `b` through the declared command
/// `build-release`, its output stamped at `built`.
fn record_fixture() -> (Scratch, std::time::SystemTime) {
    let top = Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/solace.toml", "[commands]\nbuild-release = { run = [\"x\"] }\n[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"release\", command = \"build-release\" }]\n"),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n[dependencies]\nb = { path = \"../b\" }\n"),
        ("a/src/a.gleam", ""),
        ("a/priv/app.js", ""),
        ("b/src/b.gleam", ""),
    ]);
    let built = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    touch(&top.join("a/priv/app.js"), built);
    (top, built)
}

/// `record_fixture` with `build-release` recorded, as a passing `run` records it.
fn recorded() -> (Scratch, std::time::SystemTime) {
    let (top, built) = record_fixture();
    let sources = built_sources(&top, "a", "build-release").unwrap().unwrap();
    record_built(&top, "a", "build-release", sources).unwrap();
    (top, built)
}

/// The `out of date` line of the plan, if any.
fn out_of_date(top: &Path) -> Option<String> {
    plan_text(top).unwrap().lines().find(|l| l.contains("out of date")).map(String::from)
}

/// D-166: a recorded build judges sources by contents, so touching one without changing it is
/// not a change.
#[test]
fn a_recorded_build_ignores_a_touched_source() {
    let (top, built) = recorded();
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top), None);
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], Value::Null);
}

/// D-166: a source whose contents changed since the record is named.
#[test]
fn a_recorded_build_names_a_changed_source() {
    let (top, built) = recorded();
    fs::write(top.join("a/src/a.gleam"), "// changed\n").unwrap();
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/src changed since priv/app.js was built: run step `release`"));
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], "a/src changed since priv/app.js was built");
}

/// D-166: a file added after the build, a `package.json`, is named: its part's hash changed.
#[test]
fn a_recorded_build_names_a_new_manifest() {
    let (top, _) = recorded();
    fs::write(top.join("a/package.json"), "{}").unwrap();
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/package.json changed since priv/app.js was built: run step `release`"));
}

/// D-166: when an output changed since the record, the plan falls back to file times.
#[test]
fn a_changed_output_falls_back_to_file_times() {
    let (top, built) = recorded();
    fs::write(top.join("a/priv/app.js"), "// rebuilt\n").unwrap();
    touch(&top.join("a/priv/app.js"), built);
    fs::write(top.join("a/src/a.gleam"), "// changed\n").unwrap();
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/src changed after priv/app.js was built: run step `release`"));
}

/// D-166: the sources are hashed before the command runs, so a source the build itself changes
/// isn't recorded as built.
#[test]
fn a_source_changed_during_the_build_is_not_recorded_as_built() {
    let (top, built) = record_fixture();
    let sources = built_sources(&top, "a", "build-release").unwrap().unwrap();
    fs::write(top.join("a/src/a.gleam"), "// written during the build\n").unwrap();
    record_built(&top, "a", "build-release", sources).unwrap();
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/src changed since priv/app.js was built: run step `release`"));
}

/// D-166: only a recipe's last step command records; another command, or a project with no
/// recipe, hashes nothing.
#[test]
fn only_the_recording_command_hashes_its_sources() {
    let (top, _) = record_fixture();
    assert!(built_sources(&top, "a", "other").unwrap().is_none());
    assert!(built_sources(&top, "b", "build-release").unwrap().is_none());
    assert!(built_sources(&top, "a", "build-release").unwrap().is_some());
}

/// D-166: a record made for a step's old command says nothing once the recipe names another one.
#[test]
fn a_record_for_a_changed_command_falls_back_to_file_times() {
    let (top, built) = recorded();
    fs::write(
        top.join("a/solace.toml"),
        "[commands]\nbuild-release = { run = [\"x\"] }\nother = { run = [\"y\"] }\n[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"release\", command = \"other\" }]\n",
    )
    .unwrap();
    fs::write(top.join("a/src/a.gleam"), "// changed\n").unwrap();
    touch(&top.join("a/src/a.gleam"), built + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/src changed after priv/app.js was built: run step `release`"));
}

/// A symlinked folder inside a source folder that points back up the tree ends the walk.
#[test]
fn newest_does_not_follow_a_symlinked_folder() {
    let top = Scratch::new().with(&[("src/a.gleam", "")]);
    std::os::unix::fs::symlink(top.join("src"), top.join("src/up")).unwrap();
    assert!(newest(&top.join("src")).is_some());
}
