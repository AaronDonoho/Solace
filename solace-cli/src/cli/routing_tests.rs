#[cfg(test)] use crate::cli::routing::unknown_option;
#[cfg(test)] use crate::cli::routing::probe_args;
#[cfg(test)] use crate::cli::routing::next_step;
/// A read command given an option it doesn't take is refused by name.
#[test]
fn an_option_a_command_doesnt_take_is_refused_by_name() {
    let e = unknown_option(&["view", "strategy.update", "--lines", "50..80"]).unwrap().unwrap_err().0;
    assert!(e.starts_with("view has no --lines; --lines belongs to inspect and read. view takes: --bodies, --header"), "{e}");
    let e = unknown_option(&["view", "strategy.update", "--context"]).unwrap().unwrap_err().0;
    assert!(e.contains("`--context` belongs to `inspect`: `inspect <name> --context`"), "{e}");
    assert!(unknown_option(&["read", "a.md", "--lines", "1..2"]).is_none());
    assert!(unknown_option(&["inspect", "a.b", "--context"]).is_none());
    assert!(unknown_option(&["inspect", "a.b", "--lines", "1..2"]).is_none());
    assert!(unknown_option(&["view", "a", "--bodies"]).is_none());
    assert!(unknown_option(&["view", "a", "--header"]).is_none());
    assert!(unknown_option(&["view", "--draft", "x"]).is_none(), "view --draft is in the reference");
    assert!(unknown_option(&["grep", "--x"]).is_none());
}
#[test]
fn probe_args_parse() {
    assert_eq!(probe_args(&["--from", "t", "--trace", "a.b", "c"]).unwrap(), (Some("t"), vec!["a.b", "c"], None));
    assert_eq!(probe_args(&["--in", "parser"]).unwrap(), (None, vec![], Some("parser")));
    assert_eq!(probe_args(&[]).unwrap(), (None, vec![], None));
    assert!(
        probe_args(&["--trace"]).is_err()
            && probe_args(&["--from"]).is_err()
            && probe_args(&["--in"]).is_err()
            && probe_args(&["x"]).is_err()
    );
}

/// D-184: a pass in a work run names the next step, with a test first for a change no test reaches.
#[test]
fn a_pass_names_the_next_step() {
    assert_eq!(next_step(None, true), "Next: if every part of the task is changed, finish with task done.\n");
    assert_eq!(
        next_step(Some("app.two"), true),
        "Next: add a test for app.two; then, if every part of the task is changed, finish with task done.\n"
    );
    assert_eq!(next_step(None, false), "");
    assert_eq!(next_step(Some("app.two"), false), "");
}

/// A check whose task stays missing is refused after the lookup retries.
#[test]
fn a_check_still_reaches_its_project_when_the_task_lookup_misses() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"settings\"]\n"),
        ("settings/gleam.toml", "name = \"settings\"\nversion = \"1.0.0\"\ntarget = \"javascript\"\n"),
    ]);
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("gone", "r1", || crate::cli::projects::scoped("settings", &["test".to_string()]))
    });
    assert_eq!(found.unwrap_err().0, "E7001 unknown task");
}

/// A check reaches its project when the task appears while the lookup retries.
#[test]
fn a_task_that_appears_during_the_lookup_retry_reaches_its_project() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"settings\"]\n"),
        ("settings/gleam.toml", "name = \"settings\"\nversion = \"1.0.0\"\ntarget = \"javascript\"\n"),
    ]);
    let dir = crate::board::store_dir(&root);
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t1.json"), r#"{"id":"t1","column":"backlog","home":".","scope":[".","settings"],"runs":[]}"#).unwrap();
    });
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("t1", "r1", || crate::cli::projects::scoped("settings", &["test".to_string()]))
    });
    writer.join().unwrap();
    assert_eq!(found.unwrap(), Some(root.join("settings")));
}

#[test]
fn a_project_outside_a_tasks_scope_names_it_once() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
    ]);
    let dir = crate::board::store_dir(&root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.json"), r#"{"id":"t1","column":"backlog","home":".","scope":[".","a"],"runs":[]}"#).unwrap();
    let found = super::with_root(&root, || {
        crate::board::runs::with_context("t1", "r1", || crate::cli::projects::scoped("b", &["test".to_string()]))
    });
    let e = found.unwrap_err().0;
    assert!(e.contains("not one of this task's projects (., a). To change a project"), "{e}");
    assert_eq!(e.matches("add it").count(), 1, "{e}");
    assert!(!e.contains("if the task needs it"), "{e}");
}

