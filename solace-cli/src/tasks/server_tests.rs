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

#[test]
fn responding_needs_a_task_server_for_the_root() {
    assert_eq!(responding(Path::new("/work/no-task-server"), Duration::from_millis(1)), Answering::No);
}

#[test]
fn answering_is_busy_while_an_operation_is_queued() {
    let elsewhere = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
    let (jobs, _queue) = mpsc::channel::<Job>();
    let server = Server { jobs, thread: elsewhere, top: PathBuf::from("/work/busy") };
    assert_eq!(answering(Some(&server), std::thread::current().id(), Duration::from_millis(10)), Answering::Busy);
}

#[test]
fn answering_is_yes_when_the_queue_returns() {
    let elsewhere = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
    let (jobs, queue) = mpsc::channel::<Job>();
    let server = Server { jobs, thread: elsewhere, top: PathBuf::from("/work/yes") };
    std::thread::spawn(move || {
        while let Ok(job) = queue.recv() {
            job();
        }
    });
    assert_eq!(answering(Some(&server), std::thread::current().id(), Duration::from_millis(500)), Answering::Yes);
}

#[test]
fn answering_on_the_server_thread_is_yes() {
    let (jobs, _queue) = mpsc::channel::<Job>();
    let server = Server { jobs, thread: std::thread::current().id(), top: PathBuf::from("/work/self") };
    assert_eq!(answering(Some(&server), std::thread::current().id(), Duration::from_millis(1)), Answering::Yes);
}

#[test]
fn answering_is_no_when_the_server_queue_is_gone() {
    let elsewhere = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
    let (jobs, queue) = mpsc::channel::<Job>();
    drop(queue);
    let server = Server { jobs, thread: elsewhere, top: PathBuf::from("/work/gone") };
    assert_eq!(answering(Some(&server), std::thread::current().id(), Duration::from_millis(1)), Answering::No);
}

