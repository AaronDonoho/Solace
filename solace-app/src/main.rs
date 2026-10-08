//! The Solace desktop app (D-80): a start screen, a tray, and `solace serve`
//! run as a sidecar. The board itself is the server's own page, shown in an
//! app window or the default browser.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// The port `solace serve` tries first, so bookmarks and the browser's
/// storage survive restarts.
const PREFERRED_PORT: u16 = 3030;

#[derive(Default)]
struct Server {
    child: Mutex<Option<Child>>,
    port: Mutex<Option<u16>>,
}

/// The bundled CLI, next to the app's executable (Tauri's externalBin).
fn solace_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let beside = exe.with_file_name("solace");
    if beside.exists() { beside } else { "solace".into() }
}

/// The user's login-shell PATH: a GUI launch gets a bare one, and agents,
/// gleam and bun need the shell's.
fn login_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let out = Command::new(shell).args(["-lc", "printf %s \"$PATH\""]).output().ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !path.is_empty()).then_some(path)
}

/// The port of a server already holding `root`'s serve lock. The port in the
/// file is trusted only while its lock is held (D-80).
fn running_port(root: &Path) -> Option<u16> {
    let path = root.join(".solace/serve.lock");
    let f = fs::OpenOptions::new().write(true).open(&path).ok()?;
    if f.try_lock().is_ok() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    let port = v["port"].as_u64()? as u16;
    answers(port).then_some(port)
}

/// Whether a Solace server answers on `port`.
fn answers(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /projects HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    let mut head = [0u8; 12];
    s.write_all(req.as_bytes()).is_ok() && s.read_exact(&mut head).is_ok() && head.ends_with(b"200")
}

/// Starts `solace serve --port <port>` in `root`; its port, or what it said
/// when it stopped.
fn spawn(root: &Path, port: u16) -> Result<(Child, u16), String> {
    let bin = solace_bin();
    let mut cmd = Command::new(&bin);
    cmd.args(["serve", "--port", &port.to_string()])
        .current_dir(root)
        .env("SOLACE_BIN", &bin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start {}: {e}", bin.display()))?;
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let _ = out.read_line(&mut line);
    if let Some(addr) = line.trim().strip_prefix("Solace listening on http://") {
        // Keep draining stdout so the server never blocks on a full pipe.
        std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
        let port = addr.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(port);
        return Ok((child, port));
    }
    let _ = child.wait();
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    Err(format!("{line}{err}").trim().to_string())
}

/// Ends the sidecar: SIGTERM, a moment to finish, then SIGKILL. Running
/// agents are in their own groups; the next `serve` recovers them.
fn stop(server: &Server) {
    if let Some(mut child) = server.child.lock().unwrap().take() {
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    *server.port.lock().unwrap() = None;
}

/// Shows the board at `port` in the chosen place.
fn show(app: &AppHandle, port: u16, mode: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/");
    if mode == "browser" {
        return tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string());
    }
    if let Some(w) = app.get_webview_window("board") {
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let url = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    WebviewWindowBuilder::new(app, "board", WebviewUrl::External(url))
        .title("Solace")
        .inner_size(1280.0, 820.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens `root`: attaches to a server already holding it, else starts one.
/// Errors go back to the start screen as text, never a crash.
#[tauri::command]
fn open_project(app: AppHandle, root: String, mode: String) -> Result<u16, String> {
    let root = PathBuf::from(root);
    let server = app.state::<Server>();
    let port = if let Some(port) = running_port(&root) {
        port
    } else {
        stop(&server);
        let (child, port) = match spawn(&root, PREFERRED_PORT) {
            Err(e) if e.contains("in use") || e.contains("Address") => spawn(&root, 0)?,
            other => other?,
        };
        *server.child.lock().unwrap() = Some(child);
        port
    };
    *server.port.lock().unwrap() = Some(port);
    show(&app, port, &mode)?;
    Ok(port)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Server::default())
        .invoke_handler(tauri::generate_handler![open_project])
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open board", true, None::<&str>)?;
            let switch = MenuItem::with_id(app, "switch", "Switch project", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Solace", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &switch, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => match *app.state::<Server>().port.lock().unwrap() {
                        Some(port) => {
                            let _ = show(app, port, "app");
                        }
                        None => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                            }
                        }
                    },
                    "switch" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        stop(&app.state::<Server>());
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        // Closing a window keeps Solace running: a browser tab may still use
        // the server. Quit is in the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Solace")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop(&app.state::<Server>());
            }
        });
}
