// Every Python acceptance test runs in its own process, in parallel (tests/run_parallel.py).
// SOLACE_ACCEPTANCE_JOBS sets the worker count; SOLACE_ACCEPTANCE_JOBS=1 runs them one at a time.
#[test]
#[ignore = "runs the Python acceptance suites; use `run rust-acceptance`"]
fn acceptance() {
    let out = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/run_parallel.py"))
        .args(["board_acceptance", "view_acceptance", "workspace_acceptance", "migrate_acceptance"])
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "acceptance failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
