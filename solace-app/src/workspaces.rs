//! The workspaces the app holds open: their servers, windows and browsers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;



#[derive(Default)]
pub(crate) struct Server {
    roots: Mutex<HashMap<PathBuf, Workspace>>,
    next_window: Mutex<u32>,
}

struct Workspace {
    /// None when the app attached to a server started elsewhere.
    child: Option<Child>,
    port: u16,
    /// The label of the board window while it is open.
    window: Option<String>,
    /// The port the board window shows, which decides reload or navigate.
    shown_port: Option<u16>,
    /// Whether a browser was opened on this server.
    browser: bool,
}

pub(crate) struct ClosedWorkspace {
    pub(crate) child: Option<Child>,
    /// The entry and its server stay for a browser.
    pub(crate) browser: bool,
}

pub(crate) fn workspace_port(server: &Server, root: &Path) -> Option<u16> {
    server
        .roots
        .lock()
        .unwrap()
        .get(root)
        .map(|workspace| workspace.port)
}

pub(crate) fn workspace_window(server: &Server, root: &Path) -> Option<String> {
    server
        .roots
        .lock()
        .unwrap()
        .get(root)
        .and_then(|workspace| workspace.window.clone())
}

pub(crate) fn workspace_shown_port(server: &Server, root: &Path) -> Option<u16> {
    server
        .roots
        .lock()
        .unwrap()
        .get(root)
        .and_then(|workspace| workspace.shown_port)
}

/// Records a server for `root`, returning the child it replaced.
pub(crate) fn set_workspace_server(
    server: &Server,
    root: &Path,
    child: Option<Child>,
    port: u16,
) -> Option<Child> {
    let mut roots = server.roots.lock().unwrap();
    let workspace = roots.entry(root.to_path_buf()).or_insert(Workspace {
        child: None,
        port,
        window: None,
        shown_port: None,
        browser: false,
    });
    workspace.port = port;
    std::mem::replace(&mut workspace.child, child)
}

pub(crate) fn set_workspace_port(server: &Server, root: &Path, port: u16) {
    if let Some(workspace) = server.roots.lock().unwrap().get_mut(root) {
        workspace.port = port;
    }
}

pub(crate) fn mark_browser(server: &Server, root: &Path) {
    if let Some(workspace) = server.roots.lock().unwrap().get_mut(root) {
        workspace.browser = true;
    }
}

pub(crate) fn next_window_label(server: &Server) -> String {
    let mut next = server.next_window.lock().unwrap();
    *next += 1;
    format!("board-{next}")
}

pub(crate) fn set_workspace_window(server: &Server, root: &Path, label: &str) {
    if let Some(workspace) = server.roots.lock().unwrap().get_mut(root) {
        workspace.window = Some(label.to_string());
    }
}

pub(crate) fn set_shown_port(server: &Server, root: &Path, port: Option<u16>) {
    if let Some(workspace) = server.roots.lock().unwrap().get_mut(root) {
        workspace.shown_port = port;
    }
}

pub(crate) fn window_labels(server: &Server) -> Vec<String> {
    server
        .roots
        .lock()
        .unwrap()
        .values()
        .filter_map(|workspace| workspace.window.clone())
        .collect()
}

pub(crate) fn browser_open(server: &Server) -> bool {
    server
        .roots
        .lock()
        .unwrap()
        .values()
        .any(|workspace| workspace.browser)
}

/// Forgets `label`'s window: a server kept for a browser stays, else its
/// entry and child are returned for stopping.
pub(crate) fn close_window(server: &Server, label: &str) -> Option<ClosedWorkspace> {
    let mut roots = server.roots.lock().unwrap();
    let (root, workspace) = roots
        .iter_mut()
        .find(|(_, workspace)| workspace.window.as_deref() == Some(label))?;
    if workspace.browser {
        workspace.window = None;
        workspace.shown_port = None;
        return Some(ClosedWorkspace {
            child: None,
            browser: true,
        });
    }
    let root = root.clone();
    let workspace = roots.remove(&root)?;
    Some(ClosedWorkspace {
        child: workspace.child,
        browser: false,
    })
}

/// Removes every workspace, returning the children to stop.
pub(crate) fn stop_all(server: &Server) -> Vec<Option<Child>> {
    let roots = std::mem::take(&mut *server.roots.lock().unwrap());
    roots
        .into_values()
        .map(|workspace| workspace.child)
        .collect()
}

/// Returns the show result and, when a fresh open failed, the child to stop.
/// A recorded window keeps its entry so it can still be closed or reopened.
pub(crate) fn finish_open(
    server: &Server,
    root: &Path,
    fresh: bool,
    shown: Result<(), String>,
) -> (Result<(), String>, Option<Child>) {
    if !fresh || shown.is_ok() {
        return (shown, None);
    }
    let mut roots = server.roots.lock().unwrap();
    let child = match roots.entry(root.to_path_buf()) {
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            if entry.get().window.is_some() {
                let workspace = entry.get_mut();
                workspace.shown_port = None;
                workspace.child.take()
            } else {
                entry.remove().child
            }
        }
        std::collections::hash_map::Entry::Vacant(_) => None,
    };
    (shown, child)
}

/// Where a chosen workspace appears: the browser, its existing window, or a
/// new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoardPlace {
    Browser,
    Existing,
    New,
}

pub(crate) fn board_place(window: Option<&str>, mode: &str) -> BoardPlace {
    if mode == "browser" {
        BoardPlace::Browser
    } else if window.is_some() {
        BoardPlace::Existing
    } else {
        BoardPlace::New
    }
}

#[test]
fn board_place_test() {
    assert_eq!(board_place(Some("board-1"), "browser"), BoardPlace::Browser);
    assert_eq!(board_place(None, "browser"), BoardPlace::Browser);
    assert_eq!(board_place(Some("board-1"), "app"), BoardPlace::Existing);
    assert_eq!(board_place(None, "app"), BoardPlace::New);
}

#[test]
fn workspace_tracking_test() {
    let server = Server::default();
    let a = Path::new("/work/a");
    let b = Path::new("/work/b");
    set_workspace_server(&server, a, None, 3030);
    set_workspace_server(&server, b, None, 3031);
    let a_label = next_window_label(&server);
    let b_label = next_window_label(&server);
    assert_ne!(a_label, b_label);
    assert_ne!(a_label, "main");
    set_workspace_window(&server, a, &a_label);
    set_workspace_window(&server, b, &b_label);
    set_shown_port(&server, a, Some(3030));
    assert_eq!(workspace_port(&server, a), Some(3030));
    assert_eq!(workspace_window(&server, a), Some(a_label.clone()));
    assert_eq!(workspace_window(&server, b), Some(b_label.clone()));
    assert_eq!(workspace_shown_port(&server, a), Some(3030));
    assert_eq!(window_labels(&server).len(), 2);

    set_workspace_port(&server, a, 4040);
    assert_eq!(workspace_port(&server, a), Some(4040));
    set_workspace_server(&server, a, None, 5050);
    assert_eq!(workspace_port(&server, a), Some(5050));
    assert_eq!(workspace_window(&server, a), Some(a_label));
    assert_eq!(workspace_shown_port(&server, a), Some(3030));
}

#[test]
fn closing_window_test() {
    let server = Server::default();
    let a = Path::new("/work/a");
    let b = Path::new("/work/b");
    set_workspace_server(&server, a, None, 3030);
    set_workspace_server(&server, b, None, 3031);
    set_workspace_window(&server, a, "board-1");
    set_workspace_window(&server, b, "board-2");
    mark_browser(&server, a);
    assert!(browser_open(&server));

    let kept = close_window(&server, "board-1").unwrap();
    assert!(kept.browser);
    assert!(kept.child.is_none());
    assert_eq!(workspace_port(&server, a), Some(3030));
    assert_eq!(workspace_window(&server, a), None);
    assert_eq!(workspace_shown_port(&server, a), None);

    let stopped = close_window(&server, "board-2").unwrap();
    assert!(!stopped.browser);
    assert!(stopped.child.is_none());
    assert_eq!(workspace_port(&server, b), None);
}

#[test]
fn stop_all_test() {
    let server = Server::default();
    let a = Path::new("/work/a");
    let b = Path::new("/work/b");
    set_workspace_server(&server, a, None, 3030);
    set_workspace_server(&server, b, None, 3031);
    mark_browser(&server, b);
    let children = stop_all(&server);
    assert_eq!(children.len(), 2);
    assert!(children.into_iter().all(|child| child.is_none()));
    assert_eq!(workspace_port(&server, a), None);
    assert_eq!(workspace_port(&server, b), None);
    assert!(!browser_open(&server));
}

#[test]
fn finish_open_test() {
    let server = Server::default();
    let a = Path::new("/work/a");

    set_workspace_server(&server, a, None, 4040);
    let (shown, child) = finish_open(&server, a, true, Err("couldn't show".into()));
    assert!(shown.is_err());
    assert!(child.is_none());
    assert_eq!(workspace_port(&server, a), None);

    set_workspace_server(&server, a, None, 4040);
    let (shown, child) = finish_open(&server, a, true, Ok(()));
    assert!(shown.is_ok());
    assert!(child.is_none());
    assert_eq!(workspace_port(&server, a), Some(4040));

    let (shown, child) = finish_open(&server, a, false, Err("couldn't show".into()));
    assert!(shown.is_err());
    assert!(child.is_none());
    assert_eq!(workspace_port(&server, a), Some(4040));

    set_workspace_window(&server, a, "board-1");
    set_shown_port(&server, a, Some(3030));
    set_workspace_server(&server, a, None, 5050);
    let (shown, child) = finish_open(&server, a, true, Err("couldn't show".into()));
    assert!(shown.is_err());
    assert!(child.is_none());
    assert_eq!(workspace_port(&server, a), Some(5050));
    assert_eq!(workspace_window(&server, a), Some("board-1".to_string()));
    assert_eq!(workspace_shown_port(&server, a), None);
}
