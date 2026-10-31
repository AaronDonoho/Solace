use super::*;
use crate::migrate;
use crate::testing::*;

/// A background command returns at once, and one that can't start is
/// reported (D-158 point 3).
#[test]
fn a_background_command_does_not_wait() {
    let dir = Scratch::new();
    let start = std::time::Instant::now();
    background(&dir, &["sh".to_string(), "-c".to_string(), "sleep 3".to_string()], false).unwrap();
    assert!(start.elapsed().as_secs_f64() < 1.0, "waited {:?}", start.elapsed());
    assert!(background(&dir, &["no-such-command-anywhere".to_string()], false).is_err());
}
/// Command output over the budget keeps its first and last lines.
#[test]
fn long_output_keeps_head_and_tail() {
    let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
    let out = trim_output(&text, 500);
    assert!(out.len() <= 500, "{}", out.len());
    assert!(out.starts_with("line 1\nline 2\n") && out.trim_end().ends_with("line 1000"), "{out}");
    assert!(out.contains(" lines omitted ..."), "{out}");
    assert_eq!(trim_output("short\n", 500), "short\n");
}
/// A failing cargo test run keeps the failure, its panic, the summaries
/// and cargo's warning count, and nothing that passed (D-111).
#[test]
fn condense_cargo_failure() {
    let output = "   Compiling solace v0.1.0 (/x/solace-cli)
warning: unused variable: `x`
  --> src/main.rs:3:9
   |
3  |     let x = 1;
   |         ^ help: if this is intentional, prefix it with an underscore: `_x`
   |
   = note: `#[warn(unused_variables)]` on by default

warning: `solace` (bin \"solace\" test) generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.20s
     Running unittests src/main.rs (target/debug/deps/solace-1a2b)

running 3 tests
test board::tests::a ... ok
test board::tests::b ... FAILED
test board::tests::c ... ignored

failures:

---- board::tests::b stdout ----

thread 'board::tests::b' panicked at src/board.rs:10:5:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    board::tests::b

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

error: test failed, to rerun pass `--bin solace`
";
    let expected = "warning: `solace` (bin \"solace\" test) generated 1 warning

test board::tests::b ... FAILED

failures:

---- board::tests::b stdout ----

thread 'board::tests::b' panicked at src/board.rs:10:5:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    board::tests::b

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

error: test failed, to rerun pass `--bin solace`
";
    assert_eq!(condense(output), expected);
}
/// `run_parallel.py`: the failing test with its unittest output and the
/// totals; passing tests are left out.
#[test]
fn condense_acceptance_failure() {
    let output = "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.30s
     Running tests/acceptance.rs (target/debug/deps/acceptance-9f)

running 1 test
ok    board_acceptance.BoardTest.test_a (2.1s)
FAIL  view_acceptance.ViewTest.test_b (3.4s)
F
======================================================================
FAIL: test_b (view_acceptance.ViewTest.test_b)
----------------------------------------------------------------------
Traceback (most recent call last):
  File \"view_acceptance.py\", line 9, in test_b
    self.assertEqual(1, 2)
AssertionError: 1 != 2

----------------------------------------------------------------------
Ran 1 test in 3.400s

FAILED (failures=1)


ok    workspace_acceptance.WorkspaceTest.test_c (1.0s)
3 tests, 1 failed, 4.2s with 8 workers
test acceptance ... FAILED
";
    let expected = "FAIL  view_acceptance.ViewTest.test_b (3.4s)
F
======================================================================
FAIL: test_b (view_acceptance.ViewTest.test_b)
----------------------------------------------------------------------
Traceback (most recent call last):
  File \"view_acceptance.py\", line 9, in test_b
    self.assertEqual(1, 2)
AssertionError: 1 != 2

----------------------------------------------------------------------
Ran 1 test in 3.400s

FAILED (failures=1)

3 tests, 1 failed, 4.2s with 8 workers
test acceptance ... FAILED
";
    assert_eq!(condense(output), expected);
}
/// A passing run is its summaries and a warning count; a filter that
/// matched nothing still says so.
#[test]
fn condense_passing_run() {
    let output = "   Compiling a v0.1.0
warning: unused import
 --> src/a.rs:1:5

warning: dead code
 --> src/a.rs:2:4

    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.00s
     Running unittests src/main.rs (target/debug/deps/a-1)

running 2 tests
test a ... ok
test b ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/a-2)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

";
    assert_eq!(
        condense(output),
        "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n2 warnings not shown\n"
    );
    let none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.00s\n";
    assert_eq!(condense(none), "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.00s\n");
    assert_eq!(condense("hello\n"), "hello\n");
}
/// A build script's warning stays whole: it names the step to run (D-159).
#[test]
fn condense_keeps_a_build_script_warning() {
    let output = concat!(
        "   Compiling solace v0.1.0 (/x/solace-cli)\n",
        "warning: solace@0.1.0: solace-cli/frontend/priv/app.js is older than its sources: ",
        "run the [build] step `release` in solace-cli/frontend (`build plan` lists it)\n",
        "warning: unused import\n",
        " --> src/main.rs:1:5\n",
        "    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.00s\n",
        "     Running unittests src/main.rs (target/debug/deps/solace-1)\n",
        "\nrunning 1 test\ntest a ... ok\n\n",
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ",
        "finished in 0.00s\n"
    );
    assert_eq!(
        condense(output),
        concat!(
            "warning: solace@0.1.0: solace-cli/frontend/priv/app.js is older than its sources: ",
            "run the [build] step `release` in solace-cli/frontend (`build plan` lists it)\n\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ",
            "finished in 0.00s\n1 warning not shown\n"
        )
    );
}
/// Gleam's warnings, blank lines and all, become a count.
#[test]
fn condense_gleam_warning() {
    let output = "  Compiling pilot
warning: Todo found
  ┌─ /project/test/app_test.gleam:4:22
  │
4 │ pub fn todo_test() { todo }
  │                      ^^^^ This code is incomplete

This code will crash if it is run. Be sure to finish it before
running your program.

   Compiled in 0.20s
    Running pilot_test.main
1 passed, no failures
";
    assert_eq!(condense(output), "1 passed, no failures\n1 warning not shown\n");
}
/// Another program's lines that look like cargo's pass through.
#[test]
fn condense_keeps_other_programs_output() {
    let output = "Running step 3\nChecking config\nwarning: low disk\nstill here\nrunning late\nConnecting ... ok\n";
    assert_eq!(condense(output), "Running step 3\nChecking config\nstill here\nrunning late\nConnecting ... ok\n1 warning not shown\n");
}
#[test]
fn a_background_command_drops_the_agents_task_context() {
    let dir = Scratch::new().with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")]);
    let argv: Vec<String> = ["cargo", "check"].iter().map(|s| s.to_string()).collect();
    for sandbox in [false, true] {
        let command = prepared(&dir, &argv, sandbox).unwrap();
        let mut removed = 0;
        for (name, value) in command.get_envs() {
            if value.is_none() {
                for key in migrate::TASK_ENV {
                    if name == std::ffi::OsStr::new(key) {
                        removed += 1;
                    }
                }
            }
        }
        assert_eq!(removed, migrate::TASK_ENV.len(), "sandbox {sandbox}");
    }
}

