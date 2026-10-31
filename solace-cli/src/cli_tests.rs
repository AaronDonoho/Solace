use super::*;
use crate::testing;
use serde_json::json;
use crate::failure::Out;
use crate::board::server::api;
use crate::board::runs::with_context;
/// D-145: from the second successful single read in a row, the reply hints to batch.
#[test]
fn a_second_single_read_in_a_row_gets_the_hint() {
    let mut streak = 0;
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["solace grep x"], false), crate::cli::READ_HINT);
    assert_eq!(read_hint(&mut streak, &["view a.f --project lib"], false), crate::cli::READ_HINT);
    // A batch, a failed read and a change each start the count again.
    assert_eq!(read_hint(&mut streak, &["read a.md", "read b.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], true), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["write a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["--project lib inspect a.f"], false), "");
    assert_eq!(read_hint(&mut streak, &["--project lib inspect a.f"], false), crate::cli::READ_HINT);
    assert_eq!(read_hint(&mut streak, &[], false), "");
    assert_eq!(streak, 0);
}
#[test]
fn a_failed_change_stops_the_call() {
    // D-119: a failed write skips the read and the run after it.
    let mut ran = vec![];
    let (out, error) = batch(&["write a.md", "read b.md", "run rust-unit"], 10_000, &mut fake("write", &mut ran));
    assert!(error);
    assert_eq!(ran, ["write a.md"]);
    assert!(!out.contains("==== solace read") && !out.contains("==== solace run"), "{out}");
    assert!(out.ends_with("E6002 write failed\n1 of 3 commands failed: write a.md; skipped: read b.md; run rust-unit\n"), "{out}");
    // A failed read does not stop the call; with nothing skipped, the line is D-109's.
    let mut ran = vec![];
    let (out, error) = batch(&["read b.md", "write a.md"], 10_000, &mut fake("read", &mut ran));
    assert!(error);
    assert_eq!(ran, ["read b.md", "write a.md"]);
    assert!(out.ends_with("1 of 2 commands failed: read b.md\n"), "{out}");
    // A failed change as the last command skips nothing; a failed run stops nothing.
    let mut ran = vec![];
    let (out, _) = batch(&["read b.md", "solace write a.md"], 10_000, &mut fake("write", &mut ran));
    assert!(out.ends_with("1 of 2 commands failed: write a.md\n"), "{out}");
    let mut ran = vec![];
    let (out, _) = batch(&["run rust-unit", "read b.md"], 10_000, &mut fake("run", &mut ran));
    assert_eq!(ran, ["run rust-unit", "read b.md"]);
    assert!(out.ends_with("1 of 2 commands failed: run rust-unit\n"), "{out}");
    // Nothing failed: no last line, and no error.
    let (out, error) = batch(&["read b.md", "write a.md"], 10_000, &mut fake("none", &mut vec![]));
    assert!(!error && !out.contains("commands failed"), "{out}");
}
/// task:da797576d0cfaaac: a line that is only `--project <dir>` fails
/// alone, and the rest of the call runs.
#[test]
fn a_project_with_no_command_is_refused_and_the_call_goes_on() {
    let mut ran = vec![];
    let (out, error) = batch(&["--project solace-cli/board", "read b.md"], 10_000, &mut fake("none", &mut ran));
    assert!(error);
    assert_eq!(ran, ["read b.md"]);
    assert!(out.contains(crate::cli::PROJECT_ONLY), "{out}");
    assert!(out.contains("at the end of each command line it applies to"), "{out}");
    assert!(out.ends_with("1 of 2 commands failed: --project solace-cli/board\n"), "{out}");
    // A line with no `--project` at all is no such line: it runs as before.
    let mut lens = vec![];
    let (_, error) = batch(&["solace"], 10_000, &mut |w: &[String], _| -> Out {
        lens.push(w.len());
        Ok(String::new())
    });
    assert!(!error && lens == [0]);
}
/// task:da797576d0cfaaac: a panic is that line's error and stops the call;
/// the tool outlives it, so the next call runs.
#[test]
fn a_panicking_command_is_an_error_and_stops_the_call() {
    let mut ran = vec![];
    let (out, error) = batch(&["read a.md", "view x", "read b.md"], 10_000, &mut |w: &[String], _| -> Out {
        ran.push(w.join(" "));
        if w[0] == "view" {
            panic!("boom");
        }
        Ok("fine".to_string())
    });
    assert!(error);
    assert_eq!(ran, ["read a.md", "view x"]);
    assert!(out.contains("fine"), "{out}");
    assert!(
        out.ends_with(concat!(
            "Solace hit an internal error running `view x`: boom; nothing after it ran\n",
            "1 of 3 commands failed: view x; skipped: read b.md\n",
        )),
        "{out}"
    );
    let mut ran = vec![];
    let (out, error) = batch(&["read a.md"], 10_000, &mut fake("none", &mut ran));
    assert!(!error && ran == ["read a.md"], "{out}");
}
#[test]
fn a_batched_task_done_waits_for_a_passing_test() {
    // D-152: a failed test holds back the task done after it.
    let mut ran = vec![];
    let (out, error) = batch(&["submit", "test", "task done fixed"], 10_000, &mut fake_test(false, None, &mut ran));
    assert!(error);
    assert_eq!(ran, ["submit", "test"]);
    assert!(!out.contains("==== solace task done"), "{out}");
    assert!(out.ends_with(&format!("1 of 3 commands failed: test; skipped: task done fixed (the test failed)\n{HELD}")), "{out}");
    // A pass that leaves a change no test reaches holds it back too, and is no error.
    let mut ran = vec![];
    let (out, error) = batch(&["submit", "test", "task done fixed"], 10_000, &mut fake_test(true, Some("app.three"), &mut ran));
    assert!(!error);
    assert_eq!(ran, ["submit", "test"]);
    assert!(out.ends_with(&format!("Skipped: task done fixed (no test reaches app.three)\n{HELD}")), "{out}");
    // A clean pass finishes, a test in another project too.
    let mut ran = vec![];
    let (out, error) =
        batch(&["submit", "--project lib test", "task done fixed"], 10_000, &mut fake_test(true, None, &mut ran));
    assert!(!error);
    assert_eq!(ran, ["submit", "--project lib test", "task done fixed"]);
    assert!(!out.contains("Skipped") && !out.contains(crate::cli::HELD), "{out}");
    // A task done before the test, or in a call with no test, runs.
    let mut ran = vec![];
    batch(&["task done fixed", "test"], 10_000, &mut fake_test(false, None, &mut ran));
    assert_eq!(ran, ["task done fixed", "test"]);
    let mut ran = vec![];
    let (out, _) = batch(&["submit", "task done fixed"], 10_000, &mut fake_test(false, Some("app.three"), &mut ran));
    assert_eq!(ran, ["submit", "task done fixed"]);
    assert!(!out.contains("Skipped"), "{out}");
}
/// The unknown-option refusal comes before other refusals, at both entry
/// points: `run_inner`, and `--project` through `read_project`.
#[test]
fn an_unknown_option_is_refused_before_other_refusals() {
    let args = vec!["view".to_string(), "app.main".to_string(), "--lines".to_string(), "1..2".to_string()];
    let here = std::path::Path::new(".");
    let e = run_inner(here, here, &args, &mut || Ok(String::new()), None, 10_000).unwrap_err().0;
    assert!(e.starts_with("view has no --lines; --lines belongs to inspect and read"), "{e}");
    let e = read_project("no-such-project", &args, 10_000).unwrap_err().0;
    assert!(e.starts_with("view has no --lines; --lines belongs to inspect and read"), "{e}");
}
/// D-158: `--project <rust>` reads Rust items with inspect, refs and find.
#[test]
fn read_project_reads_a_rust_project() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n\npub fn caller() -> i32 {\n    helper()\n}\n"),
    ]);
    crate::import(&root).unwrap();
    crate::import(&root.join("r")).unwrap();
    for (args, want) in [
        (&["inspect", "crate.helper"][..], "crate.helper (fn)"),
        (&["refs", "crate.helper"][..], "crate.caller"),
        (&["find", "helper"][..], "crate.helper"),
    ] {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let got = with_root(&root, || read_project("r", &args, 10_000)).unwrap();
        assert!(got.contains(want), "{args:?}: {got}");
    }
}

/// D-57: without an owning store, a read finds the root from the working folder.
#[test]
fn read_project_finds_its_root_from_the_working_folder() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    crate::import(&root.join("r")).unwrap();
    let got = read_project_in(&["inspect", "crate.helper"], &root.join("r"), None, "r", 10_000).unwrap();
    assert!(got.contains("crate.helper (fn)"), "{got}");
    let outside = testing::Scratch::new();
    let e = read_project_in(&["inspect", "crate.helper"], &outside, None, "r", 10_000).unwrap_err();
    assert!(e.0.contains("No Solace root here"), "{}", e.0);
}

/// A command line with only `--project <dir>` is refused, not a panic.
#[test]
fn a_project_with_no_command_is_refused_by_the_cli() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let t = api(&p.root, "POST", "/tasks", json!({"title":"Project"})).unwrap();
    let tid = t["id"].as_str().unwrap();
    let e = with_root(&p.root, || {
        with_context(tid, "r", || {
            run_options(&["--project".to_string(), ".".to_string()], &mut || Ok(String::new()), None, 10_000, &[])
        })
    })
    .unwrap_err();
    assert!(e.0.starts_with(crate::cli::PROJECT_ONLY.trim_end()));
    assert_eq!(e.1, 2);
}

#[test]
fn a_trait_impl_name_is_typed_through_a_batch_line() {
    let (root, _) = testing::rust_project(concat!(
        "pub struct Failure;\n\n",
        "impl From<std::io::Error> for Failure {\n",
        "    fn from(_: std::io::Error) -> Self {\n        Self\n    }\n}\n",
    ));
    let (out, error) = with_root(&root, || {
        let mut run = |w: &[String], budget| run_options(w, &mut || Ok(String::new()), None, budget, &[]);
        batch(
            &[
                "view crate.<Failure as From<std::io::Error>>",
                "inspect crate.<Failure as From<std::io::Error>>::from",
                "refs crate.<Failure as From<std::io::Error>>",
                "refs crate.<Failure as From<std::io::Error>>::from",
            ],
            20_000,
            &mut run,
        )
    });
    assert!(!error, "{out}");
    assert!(out.contains("// ── crate.<Failure as From<std::io::Error>> (impl) ──"), "{out}");
    assert!(out.contains("// ── crate.<Failure as From<std::io::Error>>::from (fn) ──"), "{out}");
    assert!(out.contains("==== solace refs crate.<Failure as From<std::io::Error>> ===="), "{out}");
    assert!(out.contains("==== solace refs crate.<Failure as From<std::io::Error>>::from ===="), "{out}");
}

/// A batch runner that records each command and fails the one named `fail`.
fn fake<'a>(fail: &'static str, ran: &'a mut Vec<String>) -> impl FnMut(&[String], usize) -> Out + 'a {
    move |w: &[String], _| {
        ran.push(w.join(" "));
        if w[0] == fail { Err(Failure(format!("E6002 {fail} failed"), 1)) } else { Ok("fine".into()) }
    }
}

/// A batch runner whose `test` passes or fails, leaving `untested` named as
/// the task's changes no test reaches.
fn fake_test<'a>(pass: bool, untested: Option<&'static str>, ran: &'a mut Vec<String>) -> impl FnMut(&[String], usize) -> Out + 'a {
    move |w: &[String], _| {
        ran.push(w.join(" "));
        if !w.iter().any(|x| x == "test") {
            return Ok("fine".into());
        }
        crate::UNTESTED.with(|u| *u.borrow_mut() = untested.map(String::from));
        if pass { Ok("passed".into()) } else { Err(Failure("1 failure".into(), 1)) }
    }
}

#[test]
fn main_refuses_a_non_numeric_mcp_budget() {
    let (out, code) = crate::cli::main(vec!["mcp".to_string(), "--budget".to_string(), "x".to_string()]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--budget takes a number of bytes, got x"), "{out}");
    assert!(out.ends_with('\n'), "{out}");
}

#[test]
fn main_refuses_unknown_mcp_arguments() {
    let (out, code) = crate::cli::main(vec!["mcp".to_string(), "extra".to_string()]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("mcp [--budget N]"), "{out}");
    assert!(out.ends_with('\n'), "{out}");
}

#[test]
fn main_requires_values_for_option_flags() {
    for flag in ["--option", "--old"] {
        let (out, code) = crate::cli::main(vec![flag.to_string()]);
        assert_eq!(code, 2, "{flag}: {out}");
        assert!(out.contains(&format!("{flag} needs a value")), "{flag}: {out}");
    }
}

#[test]
fn main_returns_zero_for_help() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let (out, code) = crate::cli::routing::with_root(&p.root, || crate::cli::main(vec!["help".to_string()]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("solace"), "{out}");
}

#[test]
fn main_returns_one_for_a_failing_command() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let (out, code) = crate::cli::routing::with_root(&p.root, || crate::cli::main(vec!["view".to_string(), "app.missing".to_string()]));
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("unknown name"), "{out}");
}

#[test]
fn path_dependencies_name_the_manifest_paths() {
    let root = testing::Scratch::new()
        .with(&[("gleam.toml", "[dependencies]\nlib = { path = \"../lib\" }\n\n[dev-dependencies]\ntool = { path = \"tool\" }\n")]);
    let found = crate::cli::path_dependencies(&root);
    assert!(found.contains(&root.join("../lib")), "{found:?}");
    assert!(found.contains(&root.join("tool")), "{found:?}");
}

#[test]
fn module_elsewhere_names_the_other_project() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() { 1 }\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ("b/src/plan.gleam", "pub fn answer() { 42 }\n"),
    ]);
    crate::import(&root.join("a")).unwrap();
    crate::import(&root.join("b")).unwrap();
    assert_eq!(crate::cli::module_elsewhere(&root.join("a"), "plan"), Some("b".to_string()));
    assert_eq!(crate::cli::module_elsewhere(&root.join("a"), "missing"), None);
}

/// A parser panic leaves the span table dirty; the next command must still
/// run, and its own parse must start at the beginning of the table.
#[test]
fn a_command_after_a_caught_parser_panic_still_works() {
    std::thread::spawn(|| {
        let (_, error) = batch(&["view x"], 10_000, &mut |_: &[String], _: usize| -> Out {
            let _: proc_macro2::TokenStream = "fn go() {}\n".parse().unwrap();
            panic!("boom");
        });
        assert!(error);
        assert_eq!(crate::testing::rust_span_start(), "bytes(1..2)");
        let (out, error) = batch(&["read a.md"], 10_000, &mut |_: &[String], _: usize| -> Out {
            let _: proc_macro2::TokenStream = "fn go() {}\n".parse().unwrap();
            Ok("fine".to_string())
        });
        assert!(!error && out.contains("fine"), "{out}");
        assert_eq!(crate::testing::rust_span_start(), "bytes(1..2)");
    })
    .join()
    .unwrap();
}

