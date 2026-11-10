//! Starts and checks the `solace serve` sidecar for a workspace: its process,
//! the banner it prints, and the health it reports.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The bundled CLI, next to the app's executable (Tauri's externalBin).
fn solace_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let beside = exe.with_file_name("solace");
    if beside.exists() {
        beside
    } else {
        "solace".into()
    }
}

/// The user's login-shell PATH: a GUI launch gets a bare one, and agents,
/// gleam and bun need the shell's.
fn login_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = Command::new(shell);
    crate::without_appimage(&mut cmd);
    let out = cmd.args(["-lc", "printf %s \"$PATH\""]).output().ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !path.is_empty()).then_some(path)
}

/// The port of a server already holding `root`, when it answers. On Windows
/// the project is in the distro, out of the app's reach.
pub(crate) fn running_port(root: &Path) -> Option<u16> {
    let port = if cfg!(windows) {
        crate::wsl::running_port(root)?
    } else {
        locked_port(root)?
    };
    answers(port).then_some(port)
}

/// Whether a Solace server answers on `port`.
fn answers(port: u16) -> bool {
    http_ok(port, "/projects")
}

/// Starts `solace serve --port <port>` in `root`; its port, or what it said
/// when it stopped.
fn spawn(root: &Path, port: u16) -> Result<(Child, u16), String> {
    let mut cmd = if cfg!(windows) {
        crate::wsl::serve(root, port)
    } else {
        local_serve(root, port)
    };
    // On Windows the server's life is tied to this input staying open (`stop`).
    let input = if cfg!(windows) {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    cmd.stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = cmd.spawn();
    let mut child = started.map_err(|e| {
        format!(
            "couldn't start {}: {e}",
            cmd.get_program().to_string_lossy()
        )
    })?;
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let addr = match read_banner(&mut out) {
        Ok(addr) => addr,
        Err(said) => {
            let _ = child.wait();
            let mut err = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            return Err(format!("{said}{err}").trim().to_string());
        }
    };
    // Keep draining stdout so the server never blocks on a full pipe.
    std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
    // Its stderr too: the server reports there, and a full pipe would
    // block the thread writing, maybe while it holds a lock. Write
    // errors (a launcher may close the app's stderr) don't stop the reading.
    let mut err = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = err.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = std::io::stderr().write_all(&buf[..n]);
        }
    });
    let port = addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(port);
    if cfg!(windows) && !answers_from_windows(port) {
        stop(Some(child));
        return Err(format!(
            "Solace started, but Windows can't reach it on port {port}. \
             Try running `wsl --shutdown` in a terminal, then open Solace again."
        ));
    }
    Ok((child, port))
}

/// Ends the sidecar: SIGTERM, a moment to finish, then SIGKILL. Running
/// agents are in their own groups; the next `serve` recovers them. On Windows
/// the server is in the distro, so closing its input asks `solace-desktop` to
/// do the same there, and the last step only ends `wsl.exe`.
pub(crate) fn stop(child: Option<Child>) {
    if let Some(mut child) = child {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        drop(child.stdin.take());
        #[cfg(unix)]
        // SAFETY: `child` is a process this app spawned and has not reaped,
        // so its id is live and SIGTERM is valid for it.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM)
        };
        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn health(port: u16) -> Health {
    match http_status(port, "/health") {
        Some(200) => Health::Healthy,
        Some(503) => Health::Busy,
        // A server built before the health route still answers `/projects`.
        Some(404) if answers(port) => Health::Healthy,
        _ => Health::Unanswered,
    }
}

fn http_ok(port: u16, path: &str) -> bool {
    http_status(port, path) == Some(200)
}

/// Starts a server on `port`, falling back to any free port when it is taken.
pub(crate) fn start_server(root: &Path, port: u16) -> Result<(Child, u16), String> {
    let usable = |port| {
        if cfg!(windows) {
            crate::wsl::free_port(port)
        } else {
            port
        }
    };
    match spawn(root, usable(port)) {
        Err(e) if e.contains("in use") || e.contains("Address") => spawn(root, usable(0)),
        other => other,
    }
}

#[test]
fn http_ok_reads_the_reply_status() {
    assert!(http_ok(serve_routes(|_| "200 OK", 1), "/health"));
    assert!(!http_ok(
        serve_routes(|_| "503 Service Unavailable", 1),
        "/health"
    ));
    assert!(!http_ok(serve_routes(|_| "404 Not Found", 1), "/health"));
    assert!(answers(serve_routes(|_| "200 OK", 1)));
}

/// What a Solace server on a port says about its task server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    Healthy,
    /// The task server is still working on an operation; leave it alone.
    Busy,
    /// No reply, or the task server has stopped.
    Unanswered,
}

fn http_status(port: u16, path: &str) -> Option<u16> {
    let timeout = Duration::from_secs(2);
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    let mut head = [0u8; 12];
    s.write_all(req.as_bytes()).ok()?;
    s.read_exact(&mut head).ok()?;
    std::str::from_utf8(&head[9..12]).ok()?.parse().ok()
}

#[test]
fn health_tells_a_busy_server_from_a_stopped_one() {
    assert_eq!(health(serve_routes(|_| "200 OK", 1)), Health::Healthy);
    assert_eq!(
        health(serve_routes(|_| "503 Service Unavailable", 1)),
        Health::Busy
    );
    assert_eq!(
        health(serve_routes(|_| "500 Internal Server Error", 1)),
        Health::Unanswered
    );
}

#[test]
fn health_falls_back_when_the_server_has_no_health_route() {
    assert_eq!(
        health(serve_routes(
            |path| if path == "/health" {
                "404 Not Found"
            } else {
                "200 OK"
            },
            2
        )),
        Health::Healthy
    );
    assert_eq!(
        health(serve_routes(|_| "404 Not Found", 2)),
        Health::Unanswered
    );
}

#[cfg(test)]
pub(crate) fn serve_routes(status_for: fn(&str) -> &'static str, count: usize) -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 256];
            let n = stream.read(&mut request).unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..n]).to_string();
            let path = text.split(' ').nth(1).unwrap_or("");
            let status = status_for(path);
            let _ = stream
                .write_all(format!("HTTP/1.0 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes());
        }
    });
    port
}

/// The port in `root`'s serve lock. The port in the file is trusted only
/// while its lock is held (D-80).
fn locked_port(root: &Path) -> Option<u16> {
    let path = root.join(".solace/serve.lock");
    let f = fs::OpenOptions::new().write(true).open(&path).ok()?;
    if f.try_lock().is_ok() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    Some(v["port"].as_u64()? as u16)
}

/// Whether a server just started in the distro answers from Windows. WSL
/// carries a new port across a moment after the server says it's listening.
fn answers_from_windows(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if answers(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// `solace serve --port <port>` run directly, in `root`.
fn local_serve(root: &Path, port: u16) -> Command {
    let bin = solace_bin();
    let mut cmd = Command::new(&bin);
    cmd.args(["serve", "--port", &port.to_string()])
        .current_dir(root)
        .env("SOLACE_BIN", &bin);
    crate::without_appimage(&mut cmd);
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    // The server ends with the app, even when the app is killed or crashes.
    // The signal follows the thread that spawns it, and survives the server's
    // `exec` restart, so it must be spawned from a thread that lives as long
    // as the app (`open_project`'s command thread or `supervisor`'s own).
    #[cfg(target_os = "linux")]
    // SAFETY: `pre_exec` runs in the forked child; the calls below are
    // async-signal-safe and only set the parent-death signal.
    unsafe {
        use std::os::unix::process::CommandExt;
        let parent = libc::getpid();
        cmd.pre_exec(move || {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    cmd
}

/// Reads the server's output up to its "Solace listening on http://" line and
/// returns the address after it. Anything `wsl.exe` or the launcher prints
/// first is skipped. When the output ends without the line, everything read
/// comes back as the error.
fn read_banner(out: &mut impl BufRead) -> Result<String, String> {
    let mut seen = String::new();
    loop {
        let mut line = String::new();
        if !matches!(out.read_line(&mut line), Ok(n) if n > 0) {
            return Err(seen);
        }
        if let Some(addr) = line.trim().strip_prefix("Solace listening on http://") {
            return Ok(addr.to_string());
        }
        seen.push_str(&line);
    }
}

#[test]
fn the_banner_is_found_on_the_first_line() {
    let mut out = std::io::Cursor::new("Solace listening on http://127.0.0.1:4321\nmore\n");
    assert_eq!(read_banner(&mut out).unwrap(), "127.0.0.1:4321");
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "more\n");
}

#[test]
fn the_banner_is_found_after_other_lines() {
    let mut out = std::io::Cursor::new("welcome\r\n\nSolace listening on http://[::1]:99\r\n");
    assert_eq!(read_banner(&mut out).unwrap(), "[::1]:99");
}

#[test]
fn a_missing_banner_returns_what_was_printed() {
    let mut out = std::io::Cursor::new("one\ntwo no newline");
    assert_eq!(read_banner(&mut out).unwrap_err(), "one\ntwo no newline");
    assert_eq!(read_banner(&mut std::io::Cursor::new("")).unwrap_err(), "");
}
