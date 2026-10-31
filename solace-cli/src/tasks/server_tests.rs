use super::*;
use crate::testing;
use std::sync::mpsc;
use std::time::Duration;
use std::fs;

/// The server takes its lock only once the root has a `.solace`.
#[test]
fn a_tick_takes_the_server_lock_once_the_root_has_solace() {
    let root = testing::Scratch::new();
    let (_jobs, queue) = mpsc::channel::<Job>();
    let mut held = None;
    tick(&root, &mut held, &queue);
    assert!(held.is_none() && !root.join(".solace").exists());
    fs::create_dir_all(root.join(".solace")).unwrap();
    tick(&root, &mut held, &queue);
    assert!(held.is_some() && serving(&root));
    drop(held);
    let end = std::time::Instant::now() + Duration::from_secs(1);
    while serving(&root) && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!serving(&root));
}

