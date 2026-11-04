use super::*;
use std::ffi::OsStr;
use std::io::Write;
use std::path::PathBuf;

fn args(cmd: &Command) -> Vec<&OsStr> {
    cmd.get_args().collect()
}

#[test]
fn every_spelling_of_the_distro_becomes_a_linux_path() {
    for spelling in [
        r"\\wsl.localhost\Solace\home\solace\app",
        r"\\wsl$\Solace\home\solace\app",
        r"\\?\UNC\wsl.localhost\Solace\home\solace\app",
        r"\\?\UNC\wsl$\Solace\home\solace\app",
        r"\\WSL.LOCALHOST\solace\home\solace\app",
        "//wsl.localhost/Solace/home/solace/app",
        r"\\wsl.localhost\Solace\home\solace\app\",
    ] {
        assert_eq!(
            linux_path(spelling).as_deref(),
            Ok("/home/solace/app"),
            "{spelling}"
        );
    }
}

#[test]
fn a_path_with_spaces_keeps_them() {
    assert_eq!(
        linux_path(r"\\wsl.localhost\Solace\home\solace\My App").as_deref(),
        Ok("/home/solace/My App")
    );
}

#[test]
fn the_distros_top_folder_is_the_linux_root() {
    assert_eq!(linux_path(r"\\wsl.localhost\Solace").as_deref(), Ok("/"));
}

#[test]
fn a_linux_path_is_kept() {
    assert_eq!(
        linux_path("/home/solace/app").as_deref(),
        Ok("/home/solace/app")
    );
}

#[test]
fn a_folder_outside_the_distro_is_refused_and_named() {
    for outside in [
        r"C:\Users\me\code",
        "code",
        "",
        r"\\wsl.localhost\Ubuntu\home\me",
        r"\\wsl$\Ubuntu\home\me",
        r"\\server\share\code",
        r"\\?\C:\Users\me",
        "//",
    ] {
        let refused = linux_path(outside).unwrap_err();
        assert!(refused.contains(outside), "{refused}");
        assert!(refused.contains(PROJECTS), "{refused}");
    }
}

#[test]
fn a_listed_distro_is_ready_whatever_the_status() {
    assert_eq!(
        classify("docker-desktop\nSolace\nUbuntu\n", true),
        Setup::Ready
    );
    assert_eq!(classify("Solace\r\n", false), Setup::Ready);
    assert_eq!(classify("solace\n", true), Setup::Ready);
}

#[test]
fn working_wsl_without_the_distro_needs_the_distro() {
    assert_eq!(
        classify("docker-desktop\nUbuntu\n", true),
        Setup::NeedsDistro
    );
    assert_eq!(classify("", true), Setup::NeedsDistro);
}

#[test]
fn a_distro_with_a_longer_name_is_not_ours() {
    assert_eq!(
        classify("Solace-old\nsolace-build\n", true),
        Setup::NeedsDistro
    );
}

#[test]
fn no_list_and_no_status_needs_wsl() {
    assert_eq!(classify("", false), Setup::NeedsWsl);
}

#[test]
fn setup_states_have_the_names_the_start_screen_uses() {
    assert_eq!(Setup::NeedsWsl.name(), "needs-wsl");
    assert_eq!(Setup::NeedsDistro.name(), "needs-distro");
    assert_eq!(Setup::Ready.name(), "ready");
}

#[test]
fn wsl_commands_name_the_distro() {
    let cmd = serve(Path::new("/home/solace/My App"), 3030);
    assert_eq!(cmd.get_program(), "wsl.exe");
    assert_eq!(
        args(&cmd),
        [
            "-d",
            "Solace",
            "--exec",
            "solace-desktop",
            "serve",
            "/home/solace/My App",
            "3030"
        ]
    );
    let cmd = desktop(&["running", "/home/solace/app"]);
    assert_eq!(
        args(&cmd),
        [
            "-d",
            "Solace",
            "--exec",
            "solace-desktop",
            "running",
            "/home/solace/app"
        ]
    );
}

#[test]
fn wsl_speaks_utf8() {
    let cmd = wsl();
    assert!(
        cmd.get_envs()
            .any(|(key, value)| key == "WSL_UTF8" && value == Some(OsStr::new("1")))
    );
}

#[test]
fn a_free_port_is_kept() {
    // Tests run in parallel, so another one can take a port once it's freed.
    // Trying several ports keeps that from failing this test; a broken
    // `free_port` keeps none of them.
    let kept = (0..20).any(|_| {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        free_port(port) == port
    });
    assert!(kept);
}

#[test]
fn a_taken_port_is_replaced() {
    let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = held.local_addr().unwrap().port();
    let other = free_port(port);
    assert_ne!(other, port);
    assert_ne!(other, 0);
}

#[test]
fn port_zero_asks_for_any_free_port() {
    assert_ne!(free_port(0), 0);
}

#[test]
fn utf8_output_is_decoded_as_it_is() {
    assert_eq!(decoded("Solace\r\nÜbung\n".as_bytes()), "Solace\r\nÜbung\n");
    assert_eq!(decoded(b"bad \xff byte"), "bad \u{fffd} byte");
}

/// `text` as `wsl.exe` prints it when it ignores `WSL_UTF8`.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn utf16_output_with_no_byte_order_mark_is_decoded() {
    assert_eq!(decoded(&utf16("Solace\r\n")), "Solace\r\n");
}

#[test]
fn utf16_output_loses_its_byte_order_mark() {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(utf16("Solace\r\n"));
    assert_eq!(decoded(&bytes), "Solace\r\n");
}

#[test]
fn utf16_output_keeps_its_non_ascii_text() {
    let said = "Es gibt keine Distribution: Übung ✓";
    assert_eq!(decoded(&utf16(said)), said);
}

#[test]
fn an_odd_last_byte_is_dropped() {
    assert_eq!(decoded(&[b'S', 0, b'o']), "S");
    assert_eq!(decoded(&[0xff, 0xfe, b'S', 0, b'o']), "S");
}

#[test]
fn no_output_decodes_to_no_text() {
    assert_eq!(decoded(b""), "");
}

#[test]
fn a_distro_listed_in_utf16_is_ready() {
    let listed = utf16("Ubuntu\r\nSolace\r\n");
    assert_eq!(classify(&decoded(&listed), true), Setup::Ready);
}

#[test]
fn the_install_script_stops_on_errors_and_reports_both_failures() {
    assert!(INSTALL_SCRIPT.contains("-Verb RunAs"));
    assert!(INSTALL_SCRIPT.contains("-ErrorAction Stop"));
    let catch = INSTALL_SCRIPT.split("catch {").nth(1).unwrap();
    let catch = catch.split('}').next().unwrap();
    assert!(catch.contains("[Console]::Error.WriteLine($_.Exception.Message)"));
    assert!(catch.contains("exit 1"));
    assert!(INSTALL_SCRIPT.contains("if ($p.ExitCode -ne 0)"));
    assert!(INSTALL_SCRIPT.contains("exited with code $($p.ExitCode)"));
    assert!(INSTALL_SCRIPT.ends_with("exit $p.ExitCode"));
}

/// What the fake `solace` of `run_desktop_script` prints.
const FAKE_SERVER_BANNER: &str = "Solace listening on http://127.0.0.1:4321\n";

/// Runs `distro/solace-desktop` with `sh` under its own temporary HOME. The HOME
/// holds `profile` as `.profile` (`FAKE_BIN` stands for the folder of a fake
/// `solace` that prints the banner; that folder is also on the inherited PATH)
/// and an empty project folder (`PROJECT` in `args`). The script's input is a
/// pipe holding one line, kept open until the script ends (as the app keeps it
/// open while the server runs). Returns its output and the HOME.
fn run_desktop_script(
    name: &str,
    profile: Option<&str>,
    args: &[&str],
) -> (std::process::Output, PathBuf) {
    let home = std::env::temp_dir().join(format!("solace-desktop-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let bin = home.join("bin");
    let project = home.join("project");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&project).unwrap();
    let fake = bin.join("solace");
    fs::write(
        &fake,
        "#!/bin/sh\necho \"Solace listening on http://127.0.0.1:$3\"\n",
    )
    .unwrap();
    assert!(
        Command::new("chmod")
            .arg("+x")
            .arg(&fake)
            .status()
            .unwrap()
            .success()
    );
    if let Some(text) = profile {
        let text = text.replace("FAKE_BIN", &bin.to_string_lossy());
        fs::write(home.join(".profile"), text).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("distro/solace-desktop");
    let mut child = Command::new("sh")
        .arg(script)
        .args(
            args.iter()
                .map(|a| a.replace("PROJECT", &project.to_string_lossy())),
        )
        .env("HOME", &home)
        .env("PATH", path)
        .env_remove("ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let _ = input.write_all(b"secret\n");
    // Closing the input tells the launcher to stop the server, so closing it
    // early would race with the fake server finishing by itself.
    let out = child.wait_with_output().unwrap();
    drop(input);
    (out, home)
}
#[test]
fn a_login_file_that_prints_does_not_reach_the_apps_output() {
    if !cfg!(unix) {
        return;
    }
    let profile = "echo noise; echo error noise >&2\nPATH=\"FAKE_BIN:$PATH\"\n";
    let (out, home) = run_desktop_script("prints", Some(profile), &["serve", "PROJECT", "4321"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), FAKE_SERVER_BANNER);
    assert!(out.status.success());
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_login_file_sourcing_a_missing_file_does_not_stop_the_launcher() {
    if !cfg!(unix) {
        return;
    }
    let profile = "PATH=\"FAKE_BIN:$PATH\"\n. /no/such/file\n";
    let (out, home) = run_desktop_script("missing", Some(profile), &["serve", "PROJECT", "4321"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), FAKE_SERVER_BANNER);
    assert!(out.status.success());
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_login_file_that_exits_leaves_the_inherited_path() {
    if !cfg!(unix) {
        return;
    }
    let (out, home) = run_desktop_script(
        "exits",
        Some("echo noise before exit\nexit 1\n"),
        &["serve", "PROJECT", "4321"],
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), FAKE_SERVER_BANNER);
    assert!(out.status.success());
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_login_file_cannot_read_the_launchers_input() {
    if !cfg!(unix) {
        return;
    }
    let profile = concat!(
        "IFS= read -r line\n",
        "printf %s \"$line\" > \"$HOME/profile_input\"\n",
        "PATH=\"FAKE_BIN:$PATH\"\n"
    );
    let (out, home) = run_desktop_script("reads", Some(profile), &["serve", "PROJECT", "4321"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), FAKE_SERVER_BANNER);
    assert_eq!(fs::read_to_string(home.join("profile_input")).unwrap(), "");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_home_without_a_login_file_still_runs_the_launcher() {
    if !cfg!(unix) {
        return;
    }
    let (out, home) = run_desktop_script("none", None, &["running", "PROJECT"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    let _ = fs::remove_dir_all(home);
}
