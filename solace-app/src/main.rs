//! The Solace desktop app (D-80): a start screen, a tray, and `solace serve`
//! run as a sidecar. The board itself is the server's own page, shown in an
//! app window or the default browser.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod workspaces;
mod supervisor;
mod wsl;
mod sidecar;

#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
/// The port `solace serve` tries first, so bookmarks and the browser's
/// storage survive restarts.
const PREFERRED_PORT: u16 = 3030;

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
/// Errors go back to the start screen as text, never a crash. On Windows this
/// can't run on the main thread: creating a window there deadlocks in WebView2.
/// Elsewhere it stays there, because the server's parent-death signal follows
/// the thread that spawns it (`local_serve`).
#[cfg_attr(windows, tauri::command(async))]
#[cfg_attr(not(windows), tauri::command)]
fn open_project(app: AppHandle, root: String, mode: String) -> Result<u16, String> {
    let root = if cfg!(windows) {
        PathBuf::from(wsl::linux_path(&root)?)
    } else {
        normalize_root(PathBuf::from(root))
    };
    let server = app.state::<workspaces::Server>();
    open(
        &server,
        &root,
        &mode,
        open_steps(
            sidecar::running_port,
            sidecar::start_server,
            sidecar::stop,
            |root, port, mode, switching| show(&app, root, port, mode, switching),
            || {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            },
        ),
    )
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(workspaces::Server::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            start_info,
            setup_state,
            install_wsl,
            import_distro,
            open_terminal
        ])
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
            supervisor::start(app.handle().clone());
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
                        sidecar::stop(closed.child);
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
        sidecar::stop(child);
    }
}

#[test]
fn stopping_every_workspace_stops_its_server() {
    let server = workspaces::Server::default();
    let root = Path::new("/work/stop");
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id() as i32;
    workspaces::set_workspace_server(&server, root, Some(child), 3030);
    stop_workspaces(&server);
    assert_eq!(workspaces::workspace_port(&server, root), None);
    // SAFETY: `pid` was this test's own child, now reaped by `stop`.
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
}

/// Opens `root`: attaches to a server already holding it, else starts one.
/// The port check, process and window come from the caller, so the flow runs
/// without the app.
fn open(
    server: &workspaces::Server,
    root: &Path,
    mode: &str,
    steps: Steps<'_>,
) -> Result<u16, String> {
    let Steps {
        running,
        start,
        mut stop,
        show,
        hide,
    } = steps;
    let mut switching = false;
    let mut fresh = false;
    let port = if let Some(port) = running(root) {
        match workspaces::workspace_port(server, root) {
            Some(existing) => {
                switching = existing != port;
                if switching {
                    workspaces::set_workspace_port(server, root, port);
                }
            }
            None => {
                workspaces::set_workspace_server(server, root, None, port);
            }
        }
        port
    } else {
        match workspaces::take_workspace_child(server, root) {
            Ok(child) => stop(child),
            Err(_) => {
                return Err("This project is starting up again; try again in a moment.".to_string());
            }
        }
        let (child, port) = match start(root, PREFERRED_PORT) {
            Ok(started) => started,
            Err(e) => {
                workspaces::cancel_restart(server, root);
                return Err(e);
            }
        };
        stop(workspaces::set_workspace_server(
            server,
            root,
            Some(child),
            port,
        ));
        switching = true;
        fresh = true;
        port
    };
    let shown = show(root, port, mode, switching);
    let (shown, child) = workspaces::finish_open(server, root, fresh, shown);
    stop(child);
    shown?;
    if mode == "browser" {
        workspaces::mark_browser(server, root);
    }
    hide();
    Ok(port)
}

#[test]
fn opening_a_running_server_attaches_to_it() {
    let server = workspaces::Server::default();
    let root = Path::new("/work/open-running");
    let mut shown = None;
    let mut hidden = false;
    let port = open(
        &server,
        root,
        "browser",
        open_steps(
            |_| Some(4040),
            |_, _| panic!("started a server for a running workspace"),
            sidecar::stop,
            |root, port, mode, switching| {
                shown = Some((root.to_path_buf(), port, mode.to_string(), switching));
                Ok(())
            },
            || hidden = true,
        ),
    )
    .unwrap();
    assert_eq!(port, 4040);
    assert_eq!(workspaces::workspace_port(&server, root), Some(4040));
    assert_eq!(
        shown,
        Some((root.to_path_buf(), 4040, "browser".to_string(), false))
    );
    assert!(workspaces::browser_open(&server));
    assert!(hidden);
}

#[test]
fn opening_a_running_server_keeps_or_changes_its_port() {
    let server = workspaces::Server::default();
    let same = Path::new("/work/open-same");
    workspaces::set_workspace_server(&server, same, None, 4040);
    let mut switching = None;
    open(
        &server,
        same,
        "window",
        open_steps(
            |_| Some(4040),
            |_, _| panic!("started a server"),
            sidecar::stop,
            |_, _, _, moved| {
                switching = Some(moved);
                Ok(())
            },
            || {},
        ),
    )
    .unwrap();
    assert_eq!(switching, Some(false));

    let changed = Path::new("/work/open-changed");
    workspaces::set_workspace_server(&server, changed, None, 3030);
    let mut switching = None;
    open(
        &server,
        changed,
        "window",
        open_steps(
            |_| Some(4040),
            |_, _| panic!("started a server"),
            sidecar::stop,
            |_, _, _, moved| {
                switching = Some(moved);
                Ok(())
            },
            || {},
        ),
    )
    .unwrap();
    assert_eq!(switching, Some(true));
    assert_eq!(workspaces::workspace_port(&server, changed), Some(4040));
}

#[test]
fn opening_without_a_server_starts_one() {
    let server = workspaces::Server::default();
    let root = Path::new("/work/open-fresh");
    let mut shown = None;
    let mut hidden = false;
    let port = open(
        &server,
        root,
        "window",
        open_steps(
            |_| None,
            |_, _| {
                let child = Command::new("sleep").arg("30").spawn().unwrap();
                Ok((child, 4041))
            },
            sidecar::stop,
            |_, port, _, switching| {
                shown = Some((port, switching));
                Ok(())
            },
            || hidden = true,
        ),
    )
    .unwrap();
    assert_eq!(port, 4041);
    assert_eq!(workspaces::workspace_port(&server, root), Some(4041));
    assert_eq!(shown, Some((4041, true)));
    assert!(hidden);
    let mut child = workspaces::take_workspace_child(&server, root)
        .unwrap()
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_refused_or_failed_open_leaves_the_workspace_ready_to_try_again() {
    let server = workspaces::Server::default();
    let busy = Path::new("/work/open-busy");
    workspaces::set_workspace_server(&server, busy, None, 3030);
    workspaces::take_workspace_child(&server, busy).unwrap();
    let err = open(
        &server,
        busy,
        "window",
        open_steps(
            |_| None,
            |_, _| panic!("started a server during a restart"),
            sidecar::stop,
            |_, _, _, _| panic!("showed a refused open"),
            || {},
        ),
    )
    .unwrap_err();
    assert!(err.contains("starting up again"), "{err}");

    let failed = Path::new("/work/open-failed");
    workspaces::set_workspace_server(&server, failed, None, 3030);
    let mut shown = false;
    let err = open(
        &server,
        failed,
        "window",
        open_steps(
            |_| None,
            |_, _| Err("no solace".into()),
            sidecar::stop,
            |_, _, _, _| {
                shown = true;
                Ok(())
            },
            || {},
        ),
    )
    .unwrap_err();
    assert_eq!(err, "no solace");
    assert!(!shown);
    assert_eq!(workspaces::workspace_port(&server, failed), Some(3030));
    assert!(workspaces::take_workspace_child(&server, failed).is_ok());
}

#[test]
fn a_failed_show_after_a_fresh_start_stops_its_server() {
    let server = workspaces::Server::default();
    let root = Path::new("/work/open-unshown");
    let mut stopped = false;
    let err = open(
        &server,
        root,
        "window",
        open_steps(
            |_| None,
            |_, _| {
                let child = Command::new("sleep").arg("30").spawn().unwrap();
                Ok((child, 4042))
            },
            |child| {
                stopped |= child.is_some();
                sidecar::stop(child)
            },
            |_, _, _, _| Err("no window".into()),
            || {},
        ),
    )
    .unwrap_err();
    assert_eq!(err, "no window");
    assert!(stopped);
    assert_eq!(workspaces::workspace_port(&server, root), None);
}

/// What `open` does to a workspace: find or start its server, then show it.
struct Steps<'a> {
    running: Box<dyn FnOnce(&Path) -> Option<u16> + 'a>,
    start: Box<dyn FnOnce(&Path, u16) -> Result<(Child, u16), String> + 'a>,
    stop: Box<dyn FnMut(Option<Child>) + 'a>,
    show: Box<dyn FnOnce(&Path, u16, &str, bool) -> Result<(), String> + 'a>,
    hide: Box<dyn FnOnce() + 'a>,
}

/// Boxes the steps `open` runs into a `Steps`.
fn open_steps<'a>(
    running: impl FnOnce(&Path) -> Option<u16> + 'a,
    start: impl FnOnce(&Path, u16) -> Result<(Child, u16), String> + 'a,
    stop: impl FnMut(Option<Child>) + 'a,
    show: impl FnOnce(&Path, u16, &str, bool) -> Result<(), String> + 'a,
    hide: impl FnOnce() + 'a,
) -> Steps<'a> {
    Steps {
        running: Box::new(running),
        start: Box::new(start),
        stop: Box::new(stop),
        show: Box::new(show),
        hide: Box::new(hide),
    }
}

/// What the start screen needs to know about this system.
#[tauri::command]
fn start_info() -> serde_json::Value {
    serde_json::json!({ "windows": cfg!(windows), "projects": wsl::PROJECTS })
}

/// How far first-launch setup has come; always done off Windows.
#[tauri::command(async)]
fn setup_state() -> String {
    let setup = if cfg!(windows) {
        wsl::setup()
    } else {
        wsl::Setup::Ready
    };
    setup.name().to_string()
}

/// Turns WSL on.
#[tauri::command(async)]
fn install_wsl() -> Result<(), String> {
    wsl::install()
}

/// Sets up the distro from the copy the installer brought.
#[tauri::command(async)]
fn import_distro(app: AppHandle) -> Result<(), String> {
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA isn't set")?;
    wsl::import(
        &resources.join("solace-distro.tar"),
        Path::new(&local_app_data),
    )
}

/// Opens a terminal in the distro.
#[tauri::command]
fn open_terminal() -> Result<(), String> {
    wsl::open_terminal()
}
