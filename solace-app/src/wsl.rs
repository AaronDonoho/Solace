//! The Windows app's way into the Solace distro (D-224). `solace` is a Linux
//! program and the app can't see the distro's filesystem, so everything that
//! touches a project goes through `wsl.exe`. The Linux and macOS builds use none
//! of this at run time; it compiles there so their tests cover it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(test)]
#[path = "wsl_tests.rs"]
mod tests;

/// The distro's name in WSL. Every command names it: a machine's default
/// distro can be anything, Docker Desktop's included.
pub(crate) const DISTRO: &str = "Solace";

/// Where the folder picker opens: the home folder inside the distro.
pub(crate) const PROJECTS: &str = r"\\wsl.localhost\Solace\home\solace";

/// The folder for the distro's disk, below `%LOCALAPPDATA%`. Never below
/// `dev.solace.app`: the uninstaller's delete-data option removes that one, and
/// client projects live on this disk.
const DISK_FOLDER: &str = r"Solace\wsl";

/// How far first-launch setup has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setup {
    NeedsWsl,
    NeedsDistro,
    Ready,
}

impl Setup {
    /// The name the start screen knows this state by.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Setup::NeedsWsl => "needs-wsl",
            Setup::NeedsDistro => "needs-distro",
            Setup::Ready => "ready",
        }
    }
}

/// `program` with no console window flashing up, because the app has no
/// console of its own.
fn hidden(program: &str) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `wsl.exe`, its own messages in UTF-8 (they're UTF-16 otherwise).
fn wsl() -> Command {
    let mut cmd = hidden("wsl.exe");
    cmd.env("WSL_UTF8", "1");
    cmd
}

/// `argv` as the distro's `solace-desktop` command, run in the distro.
fn desktop(argv: &[&str]) -> Command {
    let mut cmd = in_distro(wsl());
    cmd.args(["--exec", "solace-desktop"]).args(argv);
    cmd
}

/// The Linux path of a folder the app was given: the folder picker's
/// `\\wsl.localhost\Solace\...` in any of its spellings, or a path that is Linux
/// already. Anything else isn't inside the distro, so `solace` can't reach it.
pub(crate) fn linux_path(path: &str) -> Result<String, String> {
    if path.starts_with('/') && !path.starts_with("//") {
        return Ok(path.to_string());
    }
    let outside = || {
        format!(
            "{path} is outside Solace's Linux environment, so Solace can't open it. \
             Choose a folder under {PROJECTS}."
        )
    };
    let path_with_slashes = path.replace('\\', "/");
    let share = path_with_slashes
        .strip_prefix("//?/UNC/")
        .or_else(|| path_with_slashes.strip_prefix("//"))
        .ok_or_else(outside)?;
    let mut parts = share.split('/').filter(|part| !part.is_empty());
    let host = parts.next().unwrap_or("");
    let distro = parts.next().unwrap_or("");
    let is_wsl = host.eq_ignore_ascii_case("wsl.localhost") || host.eq_ignore_ascii_case("wsl$");
    if !is_wsl || !distro.eq_ignore_ascii_case(DISTRO) {
        return Err(outside());
    }
    Ok(format!("/{}", parts.collect::<Vec<_>>().join("/")))
}

/// Where setup stands, from `wsl --list --quiet`'s output and whether
/// `wsl --status` worked. A distro in the list settles it: the list can fail
/// on a machine with none, even though WSL itself works.
pub(crate) fn classify(listed: &str, status_ok: bool) -> Setup {
    if listed
        .lines()
        .any(|name| name.trim().eq_ignore_ascii_case(DISTRO))
    {
        Setup::Ready
    } else if status_ok {
        Setup::NeedsDistro
    } else {
        Setup::NeedsWsl
    }
}

/// Where setup stands on this machine.
pub(crate) fn setup() -> Setup {
    let listed = wsl()
        .args(["--list", "--quiet"])
        .stdin(Stdio::null())
        .output();
    let listed = listed.map(|out| decoded(&out.stdout)).unwrap_or_default();
    let status = wsl().arg("--status").stdin(Stdio::null()).output();
    classify(&listed, status.is_ok_and(|out| out.status.success()))
}

/// Turns WSL on, without a distro: an administrator prompt, and Windows may
/// want a restart afterward. `--web-download` skips the Microsoft Store, which
/// company machines often block.
pub(crate) fn install() -> Result<(), String> {
    // PowerShell, for the elevation prompt `wsl.exe` can't ask for itself.
    let mut cmd = hidden("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", INSTALL_SCRIPT]);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't start the WSL install: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(failure("WSL couldn't be turned on", &out))
    }
}

/// Imports the distro from `tar` onto a new disk below `local_app_data`, as
/// WSL 2 whatever the machine's default. WSL refuses a name it already has, so
/// this can't replace a distro holding client projects.
pub(crate) fn import(tar: &Path, local_app_data: &Path) -> Result<(), String> {
    let disk = local_app_data.join(DISK_FOLDER);
    fs::create_dir_all(&disk).map_err(|e| format!("couldn't create {}: {e}", disk.display()))?;
    let out = import_command(tar, &disk)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't start wsl.exe: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(failure("The Linux environment couldn't be set up", &out))
    }
}

fn failure(what: &str, out: &std::process::Output) -> String {
    let said = format!("{}{}", decoded(&out.stderr), decoded(&out.stdout));
    format!("{what}: {}", said.trim())
}

/// The port of a server already holding `root`, read inside the distro,
/// because a lock taken there can't be seen from Windows.
pub(crate) fn running_port(root: &Path) -> Option<u16> {
    let root = root.to_str()?;
    let out = desktop(&["running", root])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// The command that serves `root` until its input closes. The app keeps that
/// input open for as long as it wants the server, and Windows closes it when
/// the app crashes, so the server never outlives the app.
pub(crate) fn serve(root: &Path, port: u16) -> Command {
    desktop(&["serve", &root.to_string_lossy(), &port.to_string()])
}

/// `port`, or a free one when a Windows program holds it (or `port` is 0). A
/// server in the distro binds a port Windows is using without complaint, and
/// then `localhost` on Windows reaches the other program.
pub(crate) fn free_port(port: u16) -> u16 {
    let bound = |port: u16| {
        TcpListener::bind(("127.0.0.1", port))
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .ok()
    };
    (port != 0)
        .then(|| bound(port))
        .flatten()
        .or_else(|| bound(0))
        .unwrap_or(port)
}

/// Opens a terminal in the distro's home folder, where agents and tools are
/// installed and signed in to.
pub(crate) fn open_terminal() -> Result<(), String> {
    terminal()
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open a terminal: {e}"))
}

/// The PowerShell that turns WSL on. `wsl.exe` runs in its own elevated
/// console, so its output never comes back: a declined prompt (or a failed
/// start) and a nonzero exit each print a line here for the person to read.
const INSTALL_SCRIPT: &str = r#"try {
    $p = Start-Process wsl.exe -Verb RunAs -Wait -PassThru -ErrorAction Stop `
        -ArgumentList '--install','--no-distribution','--web-download'
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
if ($p.ExitCode -ne 0) {
    [Console]::Error.WriteLine("wsl.exe --install exited with code $($p.ExitCode)")
}
exit $p.ExitCode"#;

/// The text of a tool's output. `wsl.exe` prints UTF-16LE when it ignores
/// `WSL_UTF8` (older versions). A byte order mark, a NUL byte, or text that
/// isn't valid UTF-8 means UTF-16LE; UTF-16LE CJK can hold no NUL. An odd last
/// byte is dropped.
fn decoded(bytes: &[u8]) -> String {
    if !bytes.contains(&0)
        && !bytes.starts_with(&[0xff, 0xfe])
        && std::str::from_utf8(bytes).is_ok()
    {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// `tar` imported from `disk` as the distro, as WSL 2.
fn import_command(tar: &Path, disk: &Path) -> Command {
    let mut cmd = wsl();
    cmd.args(["--import", DISTRO])
        .arg(disk)
        .arg(tar)
        .args(["--version", "2"]);
    cmd
}

/// `cmd` aimed at the distro.
fn in_distro(mut cmd: Command) -> Command {
    cmd.args(["-d", DISTRO]);
    cmd
}

/// `wsl.exe` opening a terminal in the distro's home folder.
fn terminal() -> Command {
    let mut cmd = in_distro(Command::new("wsl.exe"));
    cmd.args(["--cd", "~"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    cmd
}
