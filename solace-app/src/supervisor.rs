//! Watches each workspace's server after it opens: replaces one that died or
//! stopped answering, and reloads board windows after the machine wakes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Manager};
use crate::workspaces::{self, ChildState, WorkspaceSnapshot};



const TICK: Duration = Duration::from_secs(5);

const UNANSWERED_LIMIT: u32 = 6;

const WAKE_GAP: Duration = Duration::from_secs(15);

const RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    Attached,
    Healthy,
    Busy,
    Exited(std::process::ExitStatus),
    Unanswered,
    Missing,
    Restarting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Leave,
    Reload,
    Restart,
}

fn next_action(check: Check, misses: u32, woke: bool) -> (Action, u32) {
    match check {
        Check::Attached => (if woke { Action::Reload } else { Action::Leave }, misses),
        Check::Healthy | Check::Busy => (if woke { Action::Reload } else { Action::Leave }, 0),
        Check::Restarting => (Action::Leave, misses),
        Check::Exited(_) | Check::Missing => (Action::Restart, 0),
        Check::Unanswered => {
            let misses = misses.saturating_add(1);
            if misses >= UNANSWERED_LIMIT {
                (Action::Restart, 0)
            } else if woke {
                (Action::Reload, misses)
            } else {
                (Action::Leave, misses)
            }
        }
    }
}

pub(crate) fn start(app: AppHandle) {
    // This thread lives as long as the app: a replacement server inherits its
    // parent-death signal from here.
    std::thread::spawn(move || {
        let mut tracked: HashMap<PathBuf, Tracked> = HashMap::new();
        let mut previous = None;
        loop {
            std::thread::sleep(TICK);
            let woke = woke(&mut previous);
            let server = app.state::<workspaces::Server>();
            let snapshots = workspaces::workspace_snapshots(&server);
            for (root, done) in tick_all(
                &mut tracked,
                &snapshots,
                woke,
                Instant::now(),
                |root, port| restart(&server, root, port, crate::sidecar::start_server),
                |root, port| follow(&app, root, port),
                |root, port| reload(&app, root, port),
            ) {
                for line in done_messages(&root, done) {
                    eprintln!("{line}");
                }
            }
        }
    });
}

fn check(snapshot: &WorkspaceSnapshot) -> Check {
    if !snapshot.owned {
        return Check::Attached;
    }
    if snapshot.restarting {
        return Check::Restarting;
    }
    match snapshot.child {
        ChildState::Missing => Check::Missing,
        ChildState::Exited(status) => Check::Exited(status),
        ChildState::Running => match crate::sidecar::health(snapshot.port) {
            crate::sidecar::Health::Healthy => Check::Healthy,
            crate::sidecar::Health::Busy => Check::Busy,
            crate::sidecar::Health::Unanswered => Check::Unanswered,
        },
    }
}

fn restart(
    server: &workspaces::Server,
    root: &Path,
    port: u16,
    start: impl FnOnce(&Path, u16) -> Result<(std::process::Child, u16), String>,
) -> Result<u16, String> {
    match workspaces::take_workspace_child(server, root) {
        Ok(child) => crate::sidecar::stop(child),
        Err(why) => return Err(why.to_string()),
    }
    match start(root, port) {
        Ok((child, port)) => {
            crate::sidecar::stop(workspaces::set_workspace_server(
                server,
                root,
                Some(child),
                port,
            ));
            Ok(port)
        }
        Err(e) => {
            workspaces::cancel_restart(server, root);
            Err(e)
        }
    }
}

fn follow(app: &AppHandle, root: &Path, port: u16) -> Result<(), String> {
    let server = app.state::<workspaces::Server>();
    let Some(label) = workspaces::workspace_window(&server, root) else {
        return Ok(());
    };
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    crate::update_board(app, &window, root, port, true)
}

fn reload(app: &AppHandle, root: &Path, port: u16) -> Result<(), String> {
    let server = app.state::<workspaces::Server>();
    let Some(label) = workspaces::workspace_window(&server, root) else {
        return Ok(());
    };
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    window
        .eval("window.location.reload()")
        .map_err(|e| e.to_string())?;
    workspaces::set_shown_port(&server, root, Some(port));
    let _ = window.show();
    Ok(())
}

fn woke(previous: &mut Option<(Instant, SystemTime)>) -> bool {
    let (mono, wall) = (Instant::now(), SystemTime::now());
    let slept = previous.is_some_and(|(last_mono, last_wall)| {
        slept_between(
            wall.duration_since(last_wall).unwrap_or_default(),
            mono.saturating_duration_since(last_mono),
        )
    });
    *previous = Some((mono, wall));
    slept
}

fn slept_between(wall: Duration, mono: Duration) -> bool {
    wall.saturating_sub(mono) > WAKE_GAP
}

#[test]
fn a_healthy_server_is_left_alone() {
    assert_eq!(next_action(Check::Healthy, 4, false), (Action::Leave, 0));
}

#[test]
fn an_exited_server_is_restarted() {
    assert_eq!(
        next_action(Check::Exited(exit_status()), 4, false),
        (Action::Restart, 0)
    );
    assert_eq!(next_action(Check::Missing, 4, false), (Action::Restart, 0));
}

#[test]
fn an_attached_server_is_never_restarted() {
    assert_eq!(next_action(Check::Attached, 9, false), (Action::Leave, 9));
}

#[test]
fn a_stuck_server_is_restarted_only_after_repeated_checks() {
    let mut misses = 0;
    for _ in 1..UNANSWERED_LIMIT {
        let (action, next) = next_action(Check::Unanswered, misses, false);
        assert_eq!(action, Action::Leave);
        misses = next;
    }
    assert_eq!(
        next_action(Check::Unanswered, misses, false),
        (Action::Restart, 0)
    );
}

#[test]
fn a_wake_reloads_before_a_stuck_server_is_restarted() {
    assert_eq!(next_action(Check::Unanswered, 0, true), (Action::Reload, 1));
    assert_eq!(next_action(Check::Healthy, 0, true), (Action::Reload, 0));
    assert_eq!(next_action(Check::Attached, 0, true), (Action::Reload, 0));
}

#[test]
fn a_sleep_is_a_large_gap_between_the_clocks() {
    assert!(slept_between(
        Duration::from_secs(60),
        Duration::from_secs(1)
    ));
    assert!(!slept_between(
        Duration::from_secs(6),
        Duration::from_secs(5)
    ));
}

#[test]
fn check_follows_the_child_state() {
    let root = PathBuf::from("/work/a");
    let attached = WorkspaceSnapshot {
        root: root.clone(),
        port: 3030,
        owned: false,
        restarting: false,
        child: ChildState::Missing,
    };
    assert_eq!(check(&attached), Check::Attached);
    let missing = WorkspaceSnapshot {
        root: root.clone(),
        port: 3030,
        owned: true,
        restarting: false,
        child: ChildState::Missing,
    };
    assert_eq!(check(&missing), Check::Missing);
    let status = exit_status();
    let exited = WorkspaceSnapshot {
        root,
        port: 3030,
        owned: true,
        restarting: false,
        child: ChildState::Exited(status),
    };
    assert_eq!(check(&exited), Check::Exited(status));
}

#[test]
fn the_first_tick_is_not_a_wake() {
    let mut previous = None;
    assert!(!woke(&mut previous));
    assert!(!woke(&mut previous));
}

#[derive(Default)]
struct Tracked {
    misses: u32,
    retry_at: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
enum Done {
    Waiting,
    Left,
    Reloaded {
        busy: bool,
    },
    Restarted {
        port: u16,
        why: String,
        follow_error: Option<String>,
    },
    ReloadFailed(String),
    RestartFailed(String),
}

/// Checks and, when needed, repairs one workspace's server. A restart that
/// fails waits `RETRY_AFTER` before another try.
fn tick_one(
    tracked: &mut Tracked,
    snapshot: &WorkspaceSnapshot,
    woke: bool,
    now: Instant,
    restart: impl FnOnce(&Path, u16) -> Result<u16, String>,
    follow: impl FnOnce(&Path, u16) -> Result<(), String>,
    reload: impl FnOnce(&Path, u16) -> Result<(), String>,
) -> Done {
    if tracked.retry_at.is_some_and(|at| now < at) {
        return Done::Waiting;
    }
    let check = check(snapshot);
    let reason = why(check, tracked.misses.saturating_add(1));
    let (action, misses) = next_action(check, tracked.misses, woke);
    tracked.misses = misses;
    match action {
        Action::Leave => Done::Left,
        Action::Reload => match reload(&snapshot.root, snapshot.port) {
            Ok(()) => Done::Reloaded {
                busy: check == Check::Busy,
            },
            Err(e) => Done::ReloadFailed(e),
        },
        Action::Restart => match restart(&snapshot.root, snapshot.port) {
            Ok(port) => {
                tracked.retry_at = None;
                let follow_error = follow(&snapshot.root, port).err();
                Done::Restarted {
                    port,
                    why: reason,
                    follow_error,
                }
            }
            Err(e) => {
                tracked.retry_at = Some(now + RETRY_AFTER);
                Done::RestartFailed(e)
            }
        },
    }
}

fn why(check: Check, misses: u32) -> String {
    match check {
        Check::Exited(status) => format!("it exited ({status})"),
        Check::Missing => "it was missing".to_string(),
        Check::Unanswered => format!("no answer after {misses} checks"),
        other => format!("{other:?}"),
    }
}

#[test]
fn a_busy_server_is_left_alone_even_after_many_misses() {
    assert_eq!(next_action(Check::Busy, 0, false), (Action::Leave, 0));
    assert_eq!(
        next_action(Check::Busy, UNANSWERED_LIMIT, false),
        (Action::Leave, 0)
    );
    assert_eq!(
        next_action(Check::Busy, UNANSWERED_LIMIT * 10, false),
        (Action::Leave, 0)
    );
    assert_eq!(next_action(Check::Busy, 0, true), (Action::Reload, 0));
}

#[test]
fn a_restart_in_progress_is_left_alone() {
    let snapshot = WorkspaceSnapshot {
        restarting: true,
        ..snapshot(ChildState::Missing)
    };
    assert_eq!(check(&snapshot), Check::Restarting);
    assert_eq!(next_action(check(&snapshot), 0, false), (Action::Leave, 0));
}

#[test]
fn an_exited_server_is_replaced_and_followed() {
    let server = workspaces::Server::default();
    let root = PathBuf::from("/work/exit-test");
    let old = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    workspaces::set_workspace_server(&server, &root, Some(old), 3030);
    let exited = WorkspaceSnapshot {
        root: root.clone(),
        port: 3030,
        owned: true,
        restarting: false,
        child: ChildState::Exited(exit_status()),
    };
    let mut tracked = Tracked::default();
    let followed = std::cell::Cell::new(None::<u16>);
    let done = tick_one(
        &mut tracked,
        &exited,
        false,
        Instant::now(),
        |root, port| {
            restart(&server, root, port, |_, _| {
                let child = std::process::Command::new("sleep")
                    .arg("30")
                    .spawn()
                    .unwrap();
                Ok((child, 3031))
            })
        },
        |_, port| {
            followed.set(Some(port));
            Ok(())
        },
        |_, _| Ok(()),
    );
    match done {
        Done::Restarted {
            port,
            why,
            follow_error,
        } => {
            assert_eq!(port, 3031);
            assert!(why.contains("exited"), "{why}");
            assert_eq!(follow_error, None);
        }
        other => panic!("expected a restart, got {other:?}"),
    }
    assert_eq!(
        followed.get(),
        Some(3031),
        "the open window follows the replacement"
    );
    assert_eq!(workspaces::workspace_port(&server, &root), Some(3031));
    let snapshot = workspaces::workspace_snapshots(&server).pop().unwrap();
    assert!(snapshot.owned && !snapshot.restarting);
    assert_eq!(snapshot.child, ChildState::Running);
    let mut child = workspaces::take_workspace_child(&server, &root)
        .unwrap()
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_failed_restart_waits_between_tries() {
    let exited = snapshot(ChildState::Exited(exit_status()));
    let mut tracked = Tracked::default();
    let now = Instant::now();
    let attempts = std::cell::Cell::new(0);
    let first = tick_one(
        &mut tracked,
        &exited,
        false,
        now,
        |_, _| {
            attempts.set(attempts.get() + 1);
            Err("no server".to_string())
        },
        |_, _| Ok(()),
        |_, _| Ok(()),
    );
    assert!(matches!(first, Done::RestartFailed(_)));
    assert_eq!(attempts.get(), 1);
    let again = tick_one(
        &mut tracked,
        &exited,
        false,
        now + Duration::from_secs(1),
        |_, _| {
            attempts.set(attempts.get() + 1);
            Err("no server".to_string())
        },
        |_, _| Ok(()),
        |_, _| Ok(()),
    );
    assert_eq!(again, Done::Waiting);
    assert_eq!(attempts.get(), 1);
    let later = tick_one(
        &mut tracked,
        &exited,
        false,
        now + RETRY_AFTER + Duration::from_secs(1),
        |_, _| {
            attempts.set(attempts.get() + 1);
            Ok(3030)
        },
        |_, _| Ok(()),
        |_, _| Ok(()),
    );
    assert!(matches!(later, Done::Restarted { .. }));
    assert_eq!(attempts.get(), 2);
}

#[cfg(test)]
fn snapshot(child: ChildState) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        root: PathBuf::from("/work/a"),
        port: 3030,
        owned: true,
        restarting: false,
        child,
    }
}

#[cfg(test)]
fn exit_status() -> std::process::ExitStatus {
    #[cfg(unix)]
    let mut cmd = std::process::Command::new("true");
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "exit 0"]);
        cmd
    };
    cmd.status().unwrap()
}

#[test]
fn a_busy_server_on_a_wake_is_reported() {
    let mut busy = snapshot(ChildState::Running);
    busy.port = crate::sidecar::serve_routes(|_| "503 Service Unavailable", 1);
    let mut tracked = Tracked::default();
    let mut reloaded = false;
    let done = tick_one(
        &mut tracked,
        &busy,
        true,
        Instant::now(),
        |_, _| Ok(0),
        |_, _| Ok(()),
        |_, _| {
            reloaded = true;
            Ok(())
        },
    );
    assert_eq!(done, Done::Reloaded { busy: true });
    assert!(reloaded);
}

#[test]
fn a_failed_follow_after_a_restart_is_reported() {
    let exited = snapshot(ChildState::Exited(exit_status()));
    let mut tracked = Tracked::default();
    let done = tick_one(
        &mut tracked,
        &exited,
        false,
        Instant::now(),
        |_, _| Ok(3031),
        |_, _| Err("no window".to_string()),
        |_, _| Ok(()),
    );
    match done {
        Done::Restarted {
            port, follow_error, ..
        } => {
            assert_eq!(port, 3031);
            assert_eq!(follow_error.as_deref(), Some("no window"));
        }
        other => panic!("expected a restart, got {other:?}"),
    }
}

fn tick_all(
    tracked: &mut HashMap<PathBuf, Tracked>,
    snapshots: &[WorkspaceSnapshot],
    woke: bool,
    now: Instant,
    mut restart: impl FnMut(&Path, u16) -> Result<u16, String>,
    mut follow: impl FnMut(&Path, u16) -> Result<(), String>,
    mut reload: impl FnMut(&Path, u16) -> Result<(), String>,
) -> Vec<(PathBuf, Done)> {
    let mut done = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let state = tracked.entry(snapshot.root.clone()).or_default();
        let result = tick_one(
            state,
            snapshot,
            woke,
            now,
            |root, port| restart(root, port),
            |root, port| follow(root, port),
            |root, port| reload(root, port),
        );
        done.push((snapshot.root.clone(), result));
    }
    done
}

fn done_messages(root: &Path, done: Done) -> Vec<String> {
    let at = root.display();
    match done {
        Done::Waiting | Done::Left => Vec::new(),
        Done::Reloaded { busy: true } => vec![format!("{at} is still busy after a wake")],
        Done::Reloaded { busy: false } => vec![format!("reloaded {at} after a wake")],
        Done::Restarted {
            port,
            why,
            follow_error,
        } => {
            let mut lines = vec![format!("restarted {at} on port {port}: {why}")];
            if let Some(e) = follow_error {
                lines.push(format!("showing {at} after restart: {e}"));
            }
            lines
        }
        Done::ReloadFailed(e) => vec![format!("reloading {at}: {e}")],
        Done::RestartFailed(e) => vec![format!("restarting {at}: {e}")],
    }
}

#[test]
fn a_tick_covers_every_workspace() {
    let a = WorkspaceSnapshot {
        root: PathBuf::from("/work/a"),
        port: 3030,
        owned: false,
        restarting: false,
        child: ChildState::Running,
    };
    let b = WorkspaceSnapshot {
        root: PathBuf::from("/work/b"),
        port: 3031,
        owned: true,
        restarting: false,
        child: ChildState::Exited(exit_status()),
    };
    let mut started = vec![];
    let done = tick_all(
        &mut HashMap::new(),
        &[a, b],
        false,
        Instant::now(),
        |root, port| {
            started.push((root.to_path_buf(), port));
            Ok(4040)
        },
        |_, _| Ok(()),
        |_, _| Ok(()),
    );
    assert_eq!(started, [(PathBuf::from("/work/b"), 3031)]);
    assert_eq!(done.len(), 2);
    assert_eq!(done[0], (PathBuf::from("/work/a"), Done::Left));
    match &done[1] {
        (
            root,
            Done::Restarted {
                port,
                why,
                follow_error,
            },
        ) => {
            assert_eq!(root, &PathBuf::from("/work/b"));
            assert_eq!(*port, 4040);
            assert!(why.contains("exited"), "{why}");
            assert!(follow_error.is_none());
        }
        other => panic!("expected a restart, got {other:?}"),
    }
}

#[test]
fn a_tick_reports_what_it_did() {
    let root = Path::new("/work/a");
    let at = root.display();
    assert!(done_messages(root, Done::Waiting).is_empty());
    assert!(done_messages(root, Done::Left).is_empty());
    assert_eq!(
        done_messages(root, Done::Reloaded { busy: true }),
        vec![format!("{at} is still busy after a wake")]
    );
    assert_eq!(
        done_messages(root, Done::Reloaded { busy: false }),
        vec![format!("reloaded {at} after a wake")]
    );
    assert_eq!(
        done_messages(
            root,
            Done::Restarted {
                port: 3030,
                why: "exited".into(),
                follow_error: None,
            },
        ),
        vec![format!("restarted {at} on port 3030: exited")]
    );
    assert_eq!(
        done_messages(
            root,
            Done::Restarted {
                port: 3030,
                why: "exited".into(),
                follow_error: Some("no window".into()),
            },
        ),
        vec![
            format!("restarted {at} on port 3030: exited"),
            format!("showing {at} after restart: no window"),
        ]
    );
    assert_eq!(
        done_messages(root, Done::ReloadFailed("gone".into())),
        vec![format!("reloading {at}: gone")]
    );
    assert_eq!(
        done_messages(root, Done::RestartFailed("gone".into())),
        vec![format!("restarting {at}: gone")]
    );
}
