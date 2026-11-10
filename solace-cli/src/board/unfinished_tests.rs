use super::*;
use crate::board::runs::with_context;
use crate::testing;
use rusqlite::params;
use serde_json::json;
use std::fs;



fn app() -> testing::Project {
    testing::project(&[
        ("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n"),
        (
            "test/calc_test.gleam",
            concat!(
                "import calc\n\n",
                "pub fn add_test() {\n  assert calc.add(1, 2) == 3\n}\n\n",
                "pub fn sub_test() {\n  assert calc.sub(3, 1) == 2\n}\n",
            ),
        ),
    ])
}

fn put_task(dir: &std::path::Path, id: &str, title: &str, column: &str) {
    let t = json!({
        "id":id,"title":title,"body":"","home":".","column":column,"rank":1.,
        "scope":["."],"dependsOn":[],"runs":[],"notes":[],"questions":[],"links":[],
        "closedTime":null,"archivedTime":null,
    });
    fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
}

fn touch(p: &Store, task: &str, entity: &str) {
    p.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", params![task, entity, "run", entity]).unwrap();
}

/// Only an unfinished task whose change a test that actually ran can reach is
/// named: not one reached only by a skipped test, and not a finished one.
#[test]
fn included_names_unfinished_tasks_a_test_that_ran_reaches() {
    let p = app();
    let dir = crate::board::task_model::store_dir(&crate::scope::top(&p.root));
    fs::create_dir_all(&dir).unwrap();
    for (id, title, column) in [
        ("t", "Running", "assigned"),
        ("u", "Reached", "assigned"),
        ("v", "Unreached", "assigned"),
        ("w", "Finished", "complete"),
        ("x", "Canceled", "cancelled"),
        ("m", "Container", "assigned"),
    ] {
        put_task(&dir, id, title, column);
    }
    let ix = p.index().unwrap();
    let id = |name: &str| ix.syms.iter().find(|s| s.name == name).unwrap().id.clone();
    touch(&p, "t", &format!("symbol:{}", id("add")));
    touch(&p, "u", &format!("symbol:{}", id("add")));
    touch(&p, "v", &format!("symbol:{}", id("sub")));
    touch(&p, "w", &format!("symbol:{}", id("add")));
    touch(&p, "x", &format!("symbol:{}", id("add")));
    touch(&p, "m", "module:calc");
    let at = ix.syms.iter().position(|s| s.name == "add_test").unwrap();
    let ran = crate::reach::closure(&ix, at);
    let projects = HashSet::from([".".to_string()]);
    let mut lines = with_context("t", "run", || included(&p, &ix, &ran, &projects));
    lines.sort();
    assert_eq!(
        lines,
        ["included unfinished changes from task \"Container\" in .", "included unfinished changes from task \"Reached\" in .",]
    );
}

/// A compile failure can come from anywhere in the crate, so every unfinished
/// task with a code or manifest change is named, and nothing else is.
#[test]
fn a_compile_failure_names_every_unfinished_task_with_crate_changes() {
    let p = app();
    let dir = crate::board::task_model::store_dir(&crate::scope::top(&p.root));
    fs::create_dir_all(&dir).unwrap();
    for (id, title, column) in [
        ("t", "Running", "assigned"),
        ("c", "Code", "assigned"),
        ("m", "Manifest", "assigned"),
        ("l", "Lock", "assigned"),
        ("b", "Build", "assigned"),
        ("d", "Doc", "assigned"),
        ("f", "Finished", "complete"),
    ] {
        put_task(&dir, id, title, column);
    }
    let ix = p.index().unwrap();
    let add = ix.syms.iter().find(|s| s.name == "add").unwrap().id.clone();
    touch(&p, "t", "file:Cargo.toml");
    touch(&p, "c", &format!("symbol:{add}"));
    touch(&p, "m", "file:Cargo.toml");
    touch(&p, "l", "file:Cargo.lock");
    touch(&p, "b", "file:build.rs");
    touch(&p, "d", "file:README.md");
    touch(&p, "f", "file:Cargo.toml");
    let mut lines = with_context("t", "run", || compile_failure(&p));
    lines.sort();
    assert_eq!(
        lines,
        [
            "included unfinished changes from task \"Build\" in .",
            "included unfinished changes from task \"Code\" in .",
            "included unfinished changes from task \"Lock\" in .",
            "included unfinished changes from task \"Manifest\" in .",
        ]
    );
}
