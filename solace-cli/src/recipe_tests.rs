use super::*;
use crate::testing::Scratch;

use serde_json::Value;
use serde_json::json;
use std::env;
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
    assert!(
        lockdown::parse_settings("[build]\noutputs = [\"priv/app.js\"]\nsteps = [{ name = \"a\", run = [\"x\"], timeout = 600 }]").is_ok()
    );
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
        concat!(
            "[commands]\nbuild-release = { run = [\"sh\", \"-c\", \"make\"], timeout = 900 }\n",
            "[build]\noutputs = [\"priv/app.js\"]\n",
            "steps = [{ name = \"release\", command = \"build-release\" }]\n"
        ),
    )]);
    let r = of(&top, "a").unwrap().unwrap();
    assert_eq!(
        r.steps,
        [Step {
            name: "release".into(),
            run: vec!["sh".into(), "-c".into(), "make".into()],
            command: Some("build-release".into()),
            timeout: 900
        }]
    );
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
    let out = lockdown::dispatch(&p, &v, &["build", "plan"], &mut input, None, crate::cli::BUDGET).unwrap().unwrap();
    assert!(out.contains("1. . (default)") && out.contains("- gleam: gleam build --target erlang"), "{out}");
    assert!(!p.root.join("build").exists());
    let e = lockdown::dispatch(&p, &v, &["build", "now"], &mut input, None, crate::cli::BUDGET).unwrap().unwrap_err().0;
    assert!(e.contains("`build plan`"), "{e}");
}

/// Sets `path`'s modification time.
fn touch(path: &Path, t: std::time::SystemTime) {
    fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

/// A root of `a` and `b` where `a` builds `priv/app.js` from `b` through the declared command
/// `build-release`.
fn record_fixture() -> Scratch {
    let top = Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        (
            "a/solace.toml",
            concat!(
                "[commands]\nbuild-release = { run = [\"x\"] }\n[build]\n",
                "outputs = [\"priv/app.js\"]\n",
                "steps = [{ name = \"release\", command = \"build-release\" }]\n"
            ),
        ),
        ("a/gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n[dependencies]\nb = { path = \"../b\" }\n"),
        ("a/src/a.gleam", ""),
        ("a/priv/app.js", ""),
        ("b/src/b.gleam", ""),
    ]);
    top
}

/// `record_fixture` with a fingerprint matching its output.
fn recorded() -> Scratch {
    let top = record_fixture();
    write_fingerprint(&top, "a", "priv/app.js");
    top
}

/// The `out of date` line of the plan, if any.
fn out_of_date(top: &Path) -> Option<String> {
    plan_text(top).unwrap().lines().find(|l| l.contains("out of date")).map(String::from)
}

fn write_fingerprint(top: &Path, project: &str, output: &str) {
    let root = scope::path(top, project);
    write_fingerprint_with(top, project, output, source_hashes(top, &root));
}

fn write_fingerprint_with(top: &Path, project: &str, output: &str, sources: Value) {
    let root = scope::path(top, project);
    let hash = sha256(fs::read(root.join(output)).unwrap());
    fs::write(root.join(format!("{output}.fingerprint")), json!({"output": hash, "sources": sources}).to_string()).unwrap();
}

/// A matching fingerprint is fresh whatever the files' times say, as after a checkout.
#[test]
fn a_matching_fingerprint_is_fresh() {
    let top = recorded();
    let now = std::time::SystemTime::now();
    touch(&top.join("a/priv/app.js"), now - std::time::Duration::from_secs(60));
    touch(&top.join("b/src/b.gleam"), now + std::time::Duration::from_secs(60));
    assert_eq!(out_of_date(&top), None);
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], Value::Null);
}

/// A source part whose contents changed since the fingerprint is named.
#[test]
fn a_changed_input_is_stale() {
    let top = recorded();
    fs::write(top.join("b/src/b.gleam"), "// changed\n").unwrap();
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: b/src changed since priv/app.js was built: run step `release`"));
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], "b/src changed since priv/app.js was built");
}

/// D-242: a missing fingerprint is named, rather than judged by file times.
#[test]
fn a_missing_fingerprint_is_reported() {
    let top = record_fixture();
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: priv/app.js.fingerprint is missing: run step `release`"));
}

/// D-242: an unreadable fingerprint is named, rather than judged by file times.
#[test]
fn an_unreadable_fingerprint_is_reported() {
    let top = record_fixture();
    fs::write(top.join("a/priv/app.js.fingerprint"), "not json").unwrap();
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: priv/app.js.fingerprint can't be read: run step `release`"));
}

/// D-242: an output whose hash differs from its fingerprint is stale.
#[test]
fn a_changed_output_is_stale() {
    let top = recorded();
    fs::write(top.join("a/priv/app.js"), "// rebuilt elsewhere\n").unwrap();
    assert_eq!(
        out_of_date(&top).as_deref(),
        Some("   out of date: priv/app.js does not match priv/app.js.fingerprint: run step `release`")
    );
}

/// D-242: an output missing beside its fingerprint is stale.
#[test]
fn a_missing_output_is_reported() {
    let top = recorded();
    fs::remove_file(top.join("a/priv/app.js")).unwrap();
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: priv/app.js is missing: run step `release`"));
}

/// D-159: `manifest.toml` is an input part too.
#[test]
fn a_new_manifest_is_named() {
    let top = recorded();
    fs::write(top.join("a/manifest.toml"), "# dependencies\n").unwrap();
    assert_eq!(
        out_of_date(&top).as_deref(),
        Some("   out of date: a/manifest.toml changed since priv/app.js was built: run step `release`")
    );
}

/// D-242: the fingerprint's sources are hashed before the build, so a source the build itself
/// changes is not counted as built.
#[test]
fn a_source_changed_during_the_build_is_not_recorded_as_built() {
    let top = record_fixture();
    let sources = source_hashes(&top, &top.join("a"));
    fs::write(top.join("a/src/a.gleam"), "// written during the build\n").unwrap();
    write_fingerprint_with(&top, "a", "priv/app.js", sources);
    assert_eq!(out_of_date(&top).as_deref(), Some("   out of date: a/src changed since priv/app.js was built: run step `release`"));
}

/// A symlinked folder inside a source folder that points back up the tree ends the walk.
#[test]
fn fingerprints_does_not_follow_a_symlinked_folder() {
    let top = Scratch::new().with(&[("src/a.gleam", "")]);
    std::os::unix::fs::symlink(top.join("src"), top.join("src/up")).unwrap();
    assert_eq!(part_hash(&top.join("src")).len(), 64);
}

/// A passing last-step run writes the fingerprint that makes its outputs current.
#[test]
fn a_passing_last_step_records_its_output_fingerprint() {
    let project = Scratch::new()
        .with(&[
            ("gleam.toml", "name = \"a\"\nversion = \"1.0.0\"\n"),
            ("src/main.gleam", "pub fn main() { 1 }\n"),
            ("priv/app.js", "built\n"),
            (
                "solace.toml",
                concat!(
                    "[commands]\nbuild-release = { run = [\"cargo\", \"--version\"] }\n",
                    "[build]\noutputs = [\"priv/app.js\"]\n",
                    "steps = [{ name = \"release\", command = \"build-release\" }]\n"
                ),
            ),
        ])
        .import();
    let v = crate::lockdown::settings(&project.store.root).unwrap();
    let top = crate::scope::top(&project.store.root);
    assert!(plan_json(&top).unwrap()[0]["stale"].is_string());
    let out = crate::lockdown::dispatch(&project.store, &v, &["run", "build-release"], &mut || Ok(String::new()), None, 1000).unwrap();
    assert!(out.is_ok(), "{}", out.err().unwrap().0);
    assert_eq!(plan_json(&top).unwrap()[0]["stale"], Value::Null);
}

fn part_fixture() -> Scratch {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/test/fixtures/part");
    let scratch = Scratch::new();
    for name in ["gleam.toml", "file.txt", "nested/file.txt"] {
        let to = scratch.join(name);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(source.join(name), &to).unwrap();
    }
    std::os::unix::fs::symlink("file.txt", scratch.join("link.txt")).unwrap();
    scratch
}

/// The bundle's JavaScript hasher and the plan's Rust hasher agree on the shared fixture.
#[test]
fn the_shared_part_fixture_hashes_the_same_in_the_bundle() {
    assert_eq!(part_hash(&part_fixture()), "ae8589fc75d5d3f357cd2e90b5077723d43c84a1012965f0ab63408f978bee1b");
}

/// The Rust side sees table-style path dependencies; the bundle's reader must too.
#[test]
fn a_table_style_dependency_is_a_source_part() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/test/fixtures/part");
    let names: Vec<String> = source_parts(&fixture, &fixture).into_iter().map(|(name, _)| name).collect();
    assert!(names.contains(&"inline/src".to_string()), "{names:?}");
    assert!(names.contains(&"table/src".to_string()), "{names:?}");
    assert!(!names.iter().any(|name| name.starts_with("commented/")), "{names:?}");
}
