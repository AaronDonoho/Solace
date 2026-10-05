#[test]
fn board_fake_agent_acceptance() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/board_acceptance.py"))
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn view_acceptance() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/view_acceptance.py"))
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn workspace_acceptance() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workspace_acceptance.py"))
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn migrate_acceptance() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/migrate_acceptance.py"))
        .env("SOLACE_TEST_BIN", env!("CARGO_BIN_EXE_solace"))
        .status()
        .unwrap();
    assert!(status.success());
}
