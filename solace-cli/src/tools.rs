//! Solace's own Gleam and Erlang (D-264): the releases this version of Solace
//! pins, the folder they are unpacked into, and where their programs are.
//! Every check, test and build runs these copies, never one found elsewhere.

use crate::failure::{Failure, fail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;



/// One pinned archive of a tool, and the systems it runs on.
pub(crate) struct Release {
    tool: &'static str,
    version: &'static str,
    systems: &'static [&'static str],
    url: &'static str,
    sha256: &'static str,
}

/// The tools Solace installs, in the checklist's order.
pub(crate) const TOOLS: [&str; 2] = ["gleam", "erlang"];

const RELEASES: [Release; 10] = [
    Release {
        tool: "gleam",
        version: "1.19.2",
        systems: &["x86_64-linux-gnu", "x86_64-linux-musl"],
        url: "https://github.com/gleam-lang/gleam/releases/download/v1.19.2/gleam-v1.19.2-x86_64-unknown-linux-musl.tar.gz",
        sha256: "2ba2afb66ca9f6dcc01a4a6e9169b6942963bad6483bec9571ddc078cfe6f41f",
    },
    Release {
        tool: "gleam",
        version: "1.19.2",
        systems: &["aarch64-linux-gnu", "aarch64-linux-musl"],
        url: "https://github.com/gleam-lang/gleam/releases/download/v1.19.2/gleam-v1.19.2-aarch64-unknown-linux-musl.tar.gz",
        sha256: "d1d1a9d53f6b317c0563a4806873c8ccace54291ead023373728af379402d4f1",
    },
    Release {
        tool: "gleam",
        version: "1.19.2",
        systems: &["x86_64-macos"],
        url: "https://github.com/gleam-lang/gleam/releases/download/v1.19.2/gleam-v1.19.2-x86_64-apple-darwin.tar.gz",
        sha256: "c578192530507b868c4a6c8cb7ad5d2ab0830890257f41036f17c82853f19d05",
    },
    Release {
        tool: "gleam",
        version: "1.19.2",
        systems: &["aarch64-macos"],
        url: "https://github.com/gleam-lang/gleam/releases/download/v1.19.2/gleam-v1.19.2-aarch64-apple-darwin.tar.gz",
        sha256: "fa7c6113e162ab1d979f72e7dde3e907b87e4b30c518af178768e577d180324d",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["x86_64-linux-gnu"],
        url: "https://github.com/gleam-community/erlang-linux-builds/releases/download/OTP-29.1.2/erlang-29.1.2-x64-glibc.tar.gz",
        sha256: "e91eacbb7435109c3a6448d2bba7a1a0308262f9c203593c2a02c4cfa99635dc",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["x86_64-linux-musl"],
        url: "https://github.com/gleam-community/erlang-linux-builds/releases/download/OTP-29.1.2/erlang-29.1.2-x64-musl.tar.gz",
        sha256: "731080a1922e5204dce8a2e99e7e5c8c53f43a2a851b28f2ca0dd5e989d727b7",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["aarch64-linux-gnu"],
        url: "https://github.com/gleam-community/erlang-linux-builds/releases/download/OTP-29.1.2/erlang-29.1.2-arm64-glibc.tar.gz",
        sha256: "fcfef178ea250d0d283f79309ed6e520378a4752c0a10b7a520e0e8ec81bb235",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["aarch64-linux-musl"],
        url: "https://github.com/gleam-community/erlang-linux-builds/releases/download/OTP-29.1.2/erlang-29.1.2-arm64-musl.tar.gz",
        sha256: "5b185dcaba0f6e04848215d3d911b3a8ff10ad5547794f8eb73d8b564c5e6362",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["x86_64-macos"],
        url: "https://github.com/erlef/otp_builds/releases/download/OTP-29.1.2/OTP-29.1.2-macos-amd64.tar.gz",
        sha256: "bac2e78f48c30ecfdbe2b2ea219856f0225530f978b344ef8bbd6a8f7e5f58c0",
    },
    Release {
        tool: "erlang",
        version: "29.1.2",
        systems: &["aarch64-macos"],
        url: "https://github.com/erlef/otp_builds/releases/download/OTP-29.1.2/OTP-29.1.2-macos-arm64.tar.gz",
        sha256: "579af3bb043e0692503f7f24c6e65b620bdf1802a13567a7247d1d18f1ec5470",
    },
];

#[cfg(test)]
thread_local! {
    static FOLDER_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static SYSTEM_OVERRIDE: std::cell::RefCell<Option<Option<String>>> = const { std::cell::RefCell::new(None) };
}

static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The tool a program belongs to, for the programs Solace installs.
pub(crate) fn tool_of(program: &str) -> Option<&'static str> {
    match program {
        "gleam" => Some("gleam"),
        "erl" | "erlc" | "escript" => Some("erlang"),
        _ => None,
    }
}

fn label(tool: &str) -> &'static str {
    if tool == "erlang" { "Erlang" } else { "Gleam" }
}

/// The version this Solace pins for `tool`.
fn version(tool: &str) -> &'static str {
    RELEASES.iter().find(|r| r.tool == tool).map_or("", |r| r.version)
}

/// The folder Solace's tools are unpacked into: `$XDG_DATA_HOME/solace/tools`,
/// else `~/.local/share/solace/tools`. It holds nothing but the tools.
pub(crate) fn folder() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(folder) = FOLDER_OVERRIDE.with(|f| f.borrow().clone()) {
        return Some(folder);
    }
    let absolute = |key: &str| std::env::var_os(key).map(PathBuf::from).filter(|p| p.is_absolute());
    let data = absolute("XDG_DATA_HOME").or_else(|| absolute("HOME").map(|home| home.join(".local").join("share")))?;
    Some(data.join("solace").join("tools"))
}

/// Runs `f` with the tools folder at `folder`.
#[cfg(test)]
pub(crate) fn with_folder<T>(folder: &Path, f: impl FnOnce() -> T) -> T {
    let old = FOLDER_OVERRIDE.with(|o| o.replace(Some(folder.to_path_buf())));
    let out = f();
    FOLDER_OVERRIDE.with(|o| *o.borrow_mut() = old);
    out
}

fn install_folder(folder: &Path, tool: &str) -> PathBuf {
    folder.join(format!("{tool}-{}", version(tool)))
}

/// Where an install keeps its programs: Erlang's in `bin`, Gleam's at the top.
fn bin(tool: &str, install: &Path) -> PathBuf {
    if tool == "erlang" { install.join("bin") } else { install.to_path_buf() }
}

fn bin_folders(folder: &Path) -> Vec<PathBuf> {
    TOOLS.iter().map(|tool| bin(tool, &install_folder(folder, tool))).collect()
}

/// The folder `program` is installed in, when it is one of Solace's tools and
/// Solace has a download of it for this computer.
pub(crate) fn program_folder(program: &str) -> Option<PathBuf> {
    let tool = tool_of(program)?;
    pinned(tool)?;
    Some(bin(tool, &install_folder(&folder()?, tool)))
}

/// Puts the tools' folders first on `cmd`'s `PATH`, so a program a tool starts
/// itself (Gleam starts `escript` and `erl`) is Solace's copy too.
pub(crate) fn put_on_path(cmd: &mut Command) {
    let Some(folder) = folder() else {
        return;
    };
    let mut dirs = bin_folders(&folder);
    let rest: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let rest: Vec<PathBuf> = rest.into_iter().filter(|d| !dirs.contains(d)).collect();
    dirs.extend(rest);
    if let Ok(path) = std::env::join_paths(dirs) {
        cmd.env("PATH", path);
    }
}

/// This computer's system as the releases name it: `x86_64-linux-gnu`, `aarch64-macos`.
fn system() -> Option<String> {
    #[cfg(test)]
    if let Some(system) = SYSTEM_OVERRIDE.with(|s| s.borrow().clone()) {
        return system;
    }
    let arch = match std::env::consts::ARCH {
        arch @ ("x86_64" | "aarch64") => arch,
        _ => return None,
    };
    let os = match std::env::consts::OS {
        "linux" if musl() => "linux-musl",
        "linux" => "linux-gnu",
        "macos" => "macos",
        _ => return None,
    };
    Some(format!("{arch}-{os}"))
}

fn musl() -> bool {
    fs::read_dir("/lib")
        .is_ok_and(|entries| entries.filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-")))
}

fn release(tool: &str, system: &str) -> Option<&'static Release> {
    RELEASES.iter().find(|r| r.tool == tool && r.systems.contains(&system))
}

/// Downloads this computer's pinned `tool`, checks it against its checksum and
/// unpacks it into the tools folder. One install runs at a time.
pub(crate) fn install(tool: &str) -> Result<(), Failure> {
    if !TOOLS.contains(&tool) {
        return fail(format!("E7001 Solace installs gleam or erlang, not \"{tool}\"."));
    }
    let Some(release) = pinned(tool) else {
        return fail(format!("Solace has no {} download for this computer.", label(tool)));
    };
    let Some(folder) = folder() else {
        return fail(format!("Solace couldn't find the home folder to install {} into.", label(tool)));
    };
    let _one = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    install_into(release, &folder, &download)
}

/// `install` from `fetch`, which saves a URL's bytes to a file. A download
/// that misses its checksum, or doesn't unpack, leaves nothing installed. A
/// folder that lacks the tool's program is installed again.
fn install_into(release: &Release, folder: &Path, fetch: &dyn Fn(&str, &Path) -> Result<(), Failure>) -> Result<(), Failure> {
    let name = format!("{}-{}", release.tool, release.version);
    let dest = folder.join(&name);
    if holds(release.tool, &dest) {
        return Ok(());
    }
    let part = folder.join(format!(".{name}.part"));
    let _ = fs::remove_dir_all(&part);
    fs::create_dir_all(&part)?;
    let label = label(release.tool);
    let result = (|| {
        let archive = part.join("archive.tar.gz");
        fetch(release.url, &archive).map_err(|e| Failure(format!("Couldn't download {label}: {}", e.0), 1))?;
        if crate::sha256(fs::read(&archive)?) != release.sha256 {
            return fail(format!("The {label} download didn't match its checksum, so nothing was installed."));
        }
        let unpacked = part.join("unpacked");
        fs::create_dir_all(&unpacked)?;
        let out = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&unpacked).output()?;
        if !out.status.success() {
            return fail(format!("Couldn't unpack {label}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let Some(root) = unpacked_root(&unpacked, release.tool) else {
            return fail(format!("The {label} download didn't hold {label}, so nothing was installed."));
        };
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::rename(root, &dest)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&part);
    result
}

/// The folder of an unpacked archive that holds the tool where `bin` looks:
/// the archive's top, or its one folder.
fn unpacked_root(unpacked: &Path, tool: &str) -> Option<PathBuf> {
    if holds(tool, unpacked) {
        return Some(unpacked.to_path_buf());
    }
    fs::read_dir(unpacked).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.is_dir() && holds(tool, p))
}

fn download(url: &str, to: &Path) -> Result<(), Failure> {
    let out = Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .map_err(|e| Failure(format!("curl didn't run ({e})"), 1))?;
    if !out.status.success() {
        return fail(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

/// Whether a test put the tools folder elsewhere (`with_folder`).
#[cfg(test)]
pub(crate) fn folder_overridden() -> bool {
    FOLDER_OVERRIDE.with(|f| f.borrow().is_some())
}

/// Solace's installed copy of `program`, when it is one of its tools.
pub(crate) fn program(name: &str) -> Option<PathBuf> {
    program_folder(name).map(|dir| dir.join(name)).filter(|p| p.is_file())
}

/// Runs `f` as if this computer's system were `system` (`None`: one the
/// releases don't name).
#[cfg(test)]
pub(crate) fn with_system<T>(system: Option<&str>, f: impl FnOnce() -> T) -> T {
    let old = SYSTEM_OVERRIDE.with(|o| o.replace(Some(system.map(str::to_string))));
    let out = f();
    SYSTEM_OVERRIDE.with(|o| *o.borrow_mut() = old);
    out
}

/// This computer's pinned release of `tool`, when Solace has a download for it.
fn pinned(tool: &str) -> Option<&'static Release> {
    system().and_then(|s| release(tool, &s))
}

/// Whether Solace has a download of `tool` for this computer.
pub(crate) fn downloadable(tool: &str) -> bool {
    pinned(tool).is_some()
}

/// Whether folder `install` holds `tool`'s program where `bin` looks.
fn holds(tool: &str, install: &Path) -> bool {
    let program = if tool == "erlang" { "erl" } else { "gleam" };
    bin(tool, install).join(program).is_file()
}

/// Why Solace won't start `program`, one of its own tools' programs, on a
/// computer it has a download for but hasn't installed it on: no copy found
/// elsewhere stands in for its own (D-264).
pub(crate) fn not_installed(program: &str) -> Option<String> {
    program_folder(program).map(|_| format!("`{program}` is not installed; install it with Install on Solace's checklist"))
}
