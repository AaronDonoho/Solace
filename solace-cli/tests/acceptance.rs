#[test]
fn board_fake_agent_acceptance() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/board_acceptance.py"))
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .status()
        .unwrap();
    assert!(status.success());
}
