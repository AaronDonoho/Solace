use super::*;
use crate::testing::Scratch;

/// A Gleam archive holding a stand-in `gleam`, and its checksum.
fn gleam_archive(dir: &Path) -> (Vec<u8>, String) {
    let src = dir.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("gleam"), "#!/bin/sh\necho gleam 1.19.2\n").unwrap();
    let archive = dir.join("gleam.tar.gz");
    let ok = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&src).arg("gleam").status().unwrap().success();
    assert!(ok);
    let bytes = fs::read(&archive).unwrap();
    let sum = crate::sha256(&bytes);
    (bytes, sum)
}

fn pinned_gleam(sha256: &str) -> Release {
    Release {
        tool: "gleam",
        version: version("gleam"),
        systems: &[],
        url: "https://example.invalid/gleam.tar.gz",
        sha256: sha256.to_string().leak(),
    }
}

#[test]
fn a_download_that_misses_its_checksum_installs_nothing() {
    let dir = Scratch::new();
    let (bytes, _) = gleam_archive(&dir);
    let tools = dir.join("tools");
    let fetch = |_: &str, to: &Path| -> Result<(), Failure> { Ok(fs::write(to, &bytes)?) };
    let e = install_into(&pinned_gleam(&"0".repeat(64)), &tools, &fetch).unwrap_err();
    assert_eq!(e.0, "The Gleam download didn't match its checksum, so nothing was installed.");
    assert_eq!(fs::read_dir(&tools).unwrap().count(), 0, "neither the install nor its download is left");
    with_folder(&tools, || assert_eq!(crate::language::tool_path("gleam"), None));
}

#[test]
fn a_failed_download_names_the_tool_and_installs_nothing() {
    let dir = Scratch::new();
    let tools = dir.join("tools");
    let fetch = |_: &str, _: &Path| -> Result<(), Failure> { fail("curl: (6) Could not resolve host: github.com") };
    let e = install_into(&pinned_gleam(&"0".repeat(64)), &tools, &fetch).unwrap_err();
    assert_eq!(e.0, "Couldn't download Gleam: curl: (6) Could not resolve host: github.com");
    assert_eq!(fs::read_dir(&tools).unwrap().count(), 0);
}

#[test]
fn an_installed_tool_is_found_before_one_on_path() {
    let dir = Scratch::new();
    let (bytes, sum) = gleam_archive(&dir);
    let tools = dir.join("tools");
    let fetch = |_: &str, to: &Path| -> Result<(), Failure> { Ok(fs::write(to, &bytes)?) };
    install_into(&pinned_gleam(&sum), &tools, &fetch).unwrap();
    let other = dir.join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("gleam"), "").unwrap();
    let found = crate::language::with_tool_path(&[other], || with_folder(&tools, || crate::language::tool_path("gleam")));
    assert_eq!(found, Some(tools.join(format!("gleam-{}", version("gleam"))).join("gleam")));
    let names: Vec<String> = fs::read_dir(&tools).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec![format!("gleam-{}", version("gleam"))], "the download is not kept");
}

#[test]
fn erlang_programs_are_found_in_its_bin_folder_and_put_first_on_path() {
    let tools = Path::new("/home/x/.local/share/solace/tools");
    with_folder(tools, || {
        let erlang = tools.join(format!("erlang-{}", version("erlang"))).join("bin");
        for program in ["erl", "erlc", "escript"] {
            assert_eq!(program_folder(program), Some(erlang.clone()));
        }
        assert_eq!(program_folder("node"), None);
        let mut cmd = Command::new("true");
        put_on_path(&mut cmd);
        let path = cmd.get_envs().find(|(k, _)| *k == "PATH").and_then(|(_, v)| v).unwrap().to_owned();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[..2], [tools.join(format!("gleam-{}", version("gleam"))), erlang]);
    });
}

#[test]
fn every_tool_is_pinned_once_per_system() {
    for system in ["x86_64-linux-gnu", "x86_64-linux-musl", "aarch64-linux-gnu", "aarch64-linux-musl", "x86_64-macos", "aarch64-macos"] {
        for tool in TOOLS {
            assert_eq!(RELEASES.iter().filter(|r| r.tool == tool && r.systems.contains(&system)).count(), 1, "{tool} on {system}");
        }
    }
    for r in &RELEASES {
        assert_eq!(r.version, version(r.tool), "one version per tool");
        assert!(r.url.starts_with("https://") && r.sha256.len() == 64, "{}", r.url);
    }
}

#[test]
fn only_gleam_and_erlang_are_installed() {
    let e = install("node").unwrap_err();
    assert!(e.0.starts_with("E7001"), "{}", e.0);
}

/// Installs this computer's pinned tools into the real tools folder, over
/// the network, and runs Erlang from there.
#[test]
#[ignore = "downloads the pinned tools"]
fn the_pinned_tools_install_on_this_computer() {
    for tool in TOOLS {
        install(tool).unwrap();
    }
    let gleam = crate::language::tool_path("gleam").expect("gleam is installed");
    assert!(Command::new(gleam).arg("--version").output().unwrap().status.success());
    let erl = crate::language::tool_path("erl").expect("erl is installed");
    let out =
        Command::new(erl).args(["-noshell", "-eval", "io:format(\"~s\", [erlang:system_info(otp_release)]), halt()."]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "29", "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_computer_with_no_download_finds_its_tools_on_path() {
    let dir = Scratch::new();
    let other = dir.join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("gleam"), "").unwrap();
    let tools = dir.join("tools");
    let found = crate::language::with_tool_path(&[other.clone()], || {
        with_folder(&tools, || with_system(Some("riscv64-linux-gnu"), || crate::language::tool_path("gleam")))
    });
    assert_eq!(found, Some(other.join("gleam")));
    with_folder(&tools, || with_system(None, || assert_eq!(program_folder("erl"), None)));
    with_folder(&tools, || with_system(Some("x86_64-linux-gnu"), || assert!(program_folder("erl").is_some())));
}

#[test]
fn a_tool_folder_without_its_program_is_installed_again() {
    let dir = Scratch::new();
    let (bytes, sum) = gleam_archive(&dir);
    let tools = dir.join("tools");
    let dest = install_folder(&tools, "gleam");
    fs::create_dir_all(&dest).unwrap();
    with_folder(&tools, || assert_eq!(crate::language::tool_path("gleam"), None));
    let fetch = |_: &str, to: &Path| -> Result<(), Failure> { Ok(fs::write(to, &bytes)?) };
    install_into(&pinned_gleam(&sum), &tools, &fetch).unwrap();
    assert!(dest.join("gleam").is_file());
    with_folder(&tools, || assert_eq!(crate::language::tool_path("gleam"), Some(dest.join("gleam"))));
    let refuse = |_: &str, _: &Path| -> Result<(), Failure> { fail("downloaded again") };
    install_into(&pinned_gleam(&sum), &tools, &refuse).unwrap();
}

/// Saving, formatting, building, checking and a declared `gleam` command
/// stop with Install named when Solace's Gleam isn't installed, and the
/// `gleam` on PATH never runs (D-264).
#[test]
fn a_gleam_project_stops_without_solaces_gleam() {
    let s = crate::testing::project(&[("src/main.gleam", "pub fn main() { 1 }\n")]);
    let v: toml::Value = "[commands]\nsuite={run=[\"gleam\",\"test\"]}".parse().unwrap();
    let install = "`gleam` is not installed; install it with Install on Solace's checklist";
    let (errors, ran) = crate::testing::with_tools_elsewhere(|| {
        vec![
            s.submit_symbols(None, Some("main"), "pub fn main() { 2 }").unwrap_err().0,
            crate::testing::dispatch(&s, &v, &["build"], "").unwrap_err().0,
            crate::testing::dispatch(&s, &v, &["format"], "").unwrap_err().0,
            s.tool(&["check"]).unwrap_err().0,
            crate::testing::dispatch(&s, &v, &["run", "suite"], "").unwrap_err().0,
        ]
    });
    for e in &errors {
        assert!(e.starts_with("E6013") && e.contains(install), "{e}");
    }
    assert!(!ran, "no other gleam runs");
}

/// With Solace's Gleam installed but not its Erlang, a Gleam command on
/// the Erlang target stops before it starts, so no other `erl` runs.
#[test]
fn gleam_on_the_erlang_target_stops_without_solaces_erlang() {
    use std::os::unix::fs::PermissionsExt;
    let s = crate::testing::project(&[("src/main.gleam", "pub fn main() { 1 }\n")]);
    let v: toml::Value = "[commands]\nsuite={run=[\"gleam\",\"test\"]}".parse().unwrap();
    let solace_ran = s.root.join("solace-gleam-ran");
    let (err, ran) = crate::testing::with_tools_elsewhere(|| {
        let gleam = crate::tools::program_folder("gleam").unwrap().join("gleam");
        std::fs::create_dir_all(gleam.parent().unwrap()).unwrap();
        std::fs::write(&gleam, format!("#!/bin/sh\ntouch '{}'\n", solace_ran.display())).unwrap();
        std::fs::set_permissions(&gleam, std::fs::Permissions::from_mode(0o755)).unwrap();
        crate::testing::dispatch(&s, &v, &["run", "suite"], "").unwrap_err().0
    });
    assert_eq!(err, "E6013 `erl` is not installed; install it with Install on Solace's checklist");
    assert!(!ran && !solace_ran.exists(), "nothing runs");
}
