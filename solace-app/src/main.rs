//! The Solace desktop app (D-80): a start screen, a tray, and `solace serve`
//! run as a sidecar. The board itself is the server's own page, shown in an
//! app window or the default browser.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod workspaces;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// The port `solace serve` tries first, so bookmarks and the browser's
/// storage survive restarts.
const PREFERRED_PORT: u16 = 3030;

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
    let mut cmd = Command::new(shell);
    without_appimage(&mut cmd);
    let out = cmd.args(["-lc", "printf %s \"$PATH\""]).output().ok()?;
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

/// Variables an AppImage's runtime sets for its bundled GTK, WebKit and
/// Python. Left in, they reach the server and everything it runs (agents,
/// gleam, bun, python test commands) and break them.
const APPIMAGE_VARS: &[&str] = &[
    "PYTHONHOME", "PYTHONPATH", "PERLLIB", "LD_LIBRARY_PATH", "LD_PRELOAD",
    "GIO_MODULE_DIR", "GIO_EXTRA_MODULES", "GDK_PIXBUF_MODULE_FILE", "GDK_PIXBUF_MODULEDIR",
    "GSETTINGS_SCHEMA_DIR", "GST_PLUGIN_SYSTEM_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GTK_PATH", "GTK_EXE_PREFIX", "GTK_DATA_PREFIX", "QT_PLUGIN_PATH",
    "GDK_BACKEND", "GTK_THEME", "GI_TYPELIB_PATH", "GTK_IM_MODULE_FILE",
    "APPDIR", "APPIMAGE", "OWD", "ARGV0",
];

/// Gives `cmd` the user's environment instead of the AppImage's: drops the
/// runtime's variables and its entries in `XDG_DATA_DIRS`. Outside an
/// AppImage (`.deb`, dev runs) nothing changes.
fn without_appimage(cmd: &mut Command) {
    if std::env::var_os("APPIMAGE").is_none() {
        return;
    }
    let appdir = std::env::var("APPDIR").unwrap_or_default();
    for v in APPIMAGE_VARS {
        cmd.env_remove(v);
    }
    if let Ok(dirs) = std::env::var("XDG_DATA_DIRS") {
        let kept: Vec<&str> = dirs.split(':').filter(|d| !d.is_empty() && (appdir.is_empty() || !d.starts_with(&appdir))).collect();
        if kept.is_empty() {
            cmd.env_remove("XDG_DATA_DIRS");
        } else {
            cmd.env("XDG_DATA_DIRS", kept.join(":"));
        }
    }
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
    without_appimage(&mut cmd);
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    // The server ends with the app, even when the app is killed or crashes.
    // The signal follows the spawning thread, Tauri's main thread for a
    // sync command, and survives the server's `exec` restart.
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
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", bin.display()))?;
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let _ = out.read_line(&mut line);
    if let Some(addr) = line.trim().strip_prefix("Solace listening on http://") {
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
        return Ok((child, port));
    }
    let _ = child.wait();
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    Err(format!("{line}{err}").trim().to_string())
}

/// Ends the sidecar: SIGTERM, a moment to finish, then SIGKILL. Running
/// agents are in their own groups; the next `serve` recovers them.
fn stop(child: Option<Child>) {
    if let Some(mut child) = child {
        // SAFETY: `child` is a process this app spawned and has not reaped,
        // so its id is live and SIGTERM is valid for it.
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
}

/// Shows `root`'s board at `port`: an open window follows a changed server,
/// a live one is brought forward, and a browser leaves windows alone.
fn show(
    app: &AppHandle,
    root: &Path,
    port: u16,
    mode: &str,
    switching: bool,
) -> Result<(), String> {
    if mode == "browser" {
        open_browser(&board_url(port))?;
    }
    let server = app.state::<workspaces::Server>();
    let window = workspaces::workspace_window(&server, root);
    match workspaces::board_place(window.as_deref(), mode) {
        workspaces::BoardPlace::Browser => {}
        workspaces::BoardPlace::Existing => {
            match window.and_then(|label| app.get_webview_window(&label)) {
                Some(window) => update_board(app, &window, root, port, switching)?,
                None => open_board_window(app, root, port)?,
            }
        }
        workspaces::BoardPlace::New => open_board_window(app, root, port)?,
    }
    Ok(())
}

/// Opens `root`: attaches to a server already holding it, else starts one.
/// Errors go back to the start screen as text, never a crash.
#[tauri::command]
fn open_project(app: AppHandle, root: String, mode: String) -> Result<u16, String> {
    let root = normalize_root(PathBuf::from(root));
    let server = app.state::<workspaces::Server>();
    let mut switching = false;
    let mut fresh = false;
    let port = if let Some(port) = running_port(&root) {
        match workspaces::workspace_port(&server, &root) {
            Some(existing) => {
                switching = existing != port;
                if switching {
                    workspaces::set_workspace_port(&server, &root, port);
                }
            }
            None => {
                workspaces::set_workspace_server(&server, &root, None, port);
            }
        }
        port
    } else {
        let (child, port) = match spawn(&root, PREFERRED_PORT) {
            Err(e) if e.contains("in use") || e.contains("Address") => spawn(&root, 0)?,
            other => other?,
        };
        stop(workspaces::set_workspace_server(
            &server,
            &root,
            Some(child),
            port,
        ));
        switching = true;
        fresh = true;
        port
    };
    let shown = show(&app, &root, port, &mode, switching);
    let (shown, child) = workspaces::finish_open(&server, &root, fresh, shown);
    stop(child);
    shown?;
    if mode == "browser" {
        workspaces::mark_browser(&server, &root);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    Ok(port)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(workspaces::Server::default())
        .invoke_handler(tauri::generate_handler![open_project])
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open board", true, None::<&str>)?;
            let workspace =
                MenuItem::with_id(app, "workspace", "Open workspace", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Solace", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &workspace, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        let mut shown = false;
                        for label in workspaces::window_labels(&app.state::<workspaces::Server>()) {
                            if let Some(window) = app.get_webview_window(&label) {
                                shown = true;
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        if !shown {
                            focus_main(app);
                        }
                    }
                    "workspace" => focus_main(app),
                    "quit" => {
                        stop_workspaces(&app.state::<workspaces::Server>());
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        // A board window really closes, stopping its server unless a browser
        // keeps it. The last visible window quits, unless a browser remains.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let label = window.label();
                let server = app.state::<workspaces::Server>();
                if label == "main" {
                    if other_visible(&app, label) || workspaces::browser_open(&server) {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        stop_workspaces(&server);
                        app.exit(0);
                    }
                } else if let Some(closed) = workspaces::close_window(&server, label) {
                    if !closed.browser {
                        stop(closed.child);
                    }
                    if !other_visible(&app, label) && !workspaces::browser_open(&server) {
                        app.exit(0);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Solace")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop_workspaces(&app.state::<workspaces::Server>());
            }
        });
}

/// What an open board window does to reach a chosen project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoardAction {
    Focus,
    Navigate,
    Reload,
}

/// How an open board window reaches `port`: a window already showing it is
/// brought forward; during a switch a matching address reloads; otherwise the
/// window navigates to the new address.
fn board_action(shown_port: Option<u16>, switching: bool, port: u16) -> BoardAction {
    if !switching && shown_port == Some(port) {
        BoardAction::Focus
    } else if shown_port == Some(port) {
        BoardAction::Reload
    } else {
        BoardAction::Navigate
    }
}

/// Opens `url` in the user's browser.
fn open_browser(url: &str) -> Result<(), String> {
    // From an AppImage, a browser started here would inherit the
    // runtime's variables; open it with the user's environment instead.
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some() {
        let mut cmd = Command::new("xdg-open");
        without_appimage(&mut cmd);
        return cmd
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("couldn't open the browser: {e}"));
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[test]
fn board_action_test() {
    assert_eq!(
        crate::board_action(Some(3030), false, 4040),
        crate::BoardAction::Navigate
    );
    assert_eq!(
        crate::board_action(Some(3030), false, 3030),
        crate::BoardAction::Focus
    );
    assert_eq!(
        crate::board_action(Some(3030), true, 3030),
        crate::BoardAction::Reload
    );
    assert_eq!(
        crate::board_action(Some(3030), true, 4040),
        crate::BoardAction::Navigate
    );
    assert_eq!(
        crate::board_action(None, true, 3030),
        crate::BoardAction::Navigate
    );
    assert_eq!(
        crate::board_action(None, false, 5050),
        crate::BoardAction::Navigate
    );
}

/// Moves an open board window to `port`, or brings it forward when it already
/// shows that project.
fn update_board(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    root: &Path,
    port: u16,
    switching: bool,
) -> Result<(), String> {
    let server = app.state::<workspaces::Server>();
    let shown = workspaces::workspace_shown_port(&server, root);
    let action = board_action(shown, switching, port);
    match action {
        BoardAction::Navigate => {
            let url = tauri::Url::parse(&board_url(port)).map_err(|e| e.to_string())?;
            window.navigate(url).map_err(|e| e.to_string())?;
        }
        BoardAction::Reload => window
            .eval("window.location.reload()")
            .map_err(|e| e.to_string())?,
        BoardAction::Focus => {}
    }
    if action != BoardAction::Focus {
        workspaces::set_shown_port(&server, root, Some(port));
    }
    let _ = window.show();
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// The address of the board served at `port`.
fn board_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

/// One spelling per workspace, so a trailing slash or symlink can't open it twice.
fn normalize_root(root: PathBuf) -> PathBuf {
    root.canonicalize().unwrap_or(root)
}

/// A board window's title: the workspace folder's name.
fn workspace_title(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Solace".to_string())
}

/// Opens `root`'s board in a new window labeled for its workspace.
fn open_board_window(app: &AppHandle, root: &Path, port: u16) -> Result<(), String> {
    let server = app.state::<workspaces::Server>();
    let label = workspaces::next_window_label(&server);
    let url = tauri::Url::parse(&board_url(port)).map_err(|e| e.to_string())?;
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(workspace_title(root))
        .inner_size(1280.0, 820.0)
        .build()
        .map_err(|e| e.to_string())?;
    workspaces::set_workspace_window(&server, root, &label);
    workspaces::set_shown_port(&server, root, Some(port));
    Ok(())
}

/// Brings the start screen forward.
fn focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Whether another visible window remains for the app to stay open.
fn other_visible(app: &AppHandle, label: &str) -> bool {
    app.webview_windows()
        .values()
        .any(|window| window.label() != label && window.is_visible().unwrap_or(false))
}

#[test]
fn workspace_title_test() {
    assert_eq!(workspace_title(Path::new("/work/Project A")), "Project A");
    assert_eq!(workspace_title(Path::new("/")), "Solace");
}

#[cfg(unix)]
#[test]
fn root_normalization_test() {
    let base = std::env::temp_dir().join(format!("solace-app-root-{}", std::process::id()));
    let root = base.join("workspace");
    let link = base.join("link");
    fs::create_dir_all(&root).unwrap();
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink(&root, &link).unwrap();
    assert_eq!(normalize_root(link.clone()), normalize_root(root.clone()));
    assert_eq!(normalize_root(root.join("")), normalize_root(root.clone()));
    let _ = fs::remove_file(&link);
    let _ = fs::remove_dir_all(&base);
}

/// Stops every server the app started for a workspace.
fn stop_workspaces(server: &workspaces::Server) {
    for child in workspaces::stop_all(server) {
        stop(child);
    }
}
