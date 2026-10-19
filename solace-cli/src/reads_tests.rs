use super::*;

/// Reads `names` in `store` in full, as an MCP call would, and records them.
fn read_names(store: &Store, names: &[&str], reads: &mut Reads) {
    SERVED.with(|v| *v.borrow_mut() = Some(vec![]));
    store.view(names, false, 10_000).unwrap();
    reads.record(&take_served());
    SERVED.with(|v| *v.borrow_mut() = None);
}

fn served(s: &str, rev: &str) -> Served {
    Served { project: ".".into(), symbol: s.into(), revision: rev.into(), recent: true, latest: rev.into() }
}

/// A run's reads keep each symbol once, at the revision served last, and come
/// back from the task store as they were. An old `caught` key is ignored.
#[test]
fn a_runs_reads_keep_each_symbol_once_and_round_trip() {
    let mut reads = Reads::default();
    reads.record(&[served("a.f", "1"), served("a.g", "1"), served("a.f", "2")]);
    assert_eq!(reads.symbols, [served("a.g", "1"), served("a.f", "2")]);
    let back = Reads::from_json(&reads.to_json());
    assert_eq!(back.symbols, reads.symbols);
    assert!(back.to_json().get("caught").is_none());
    assert!(Reads::from_json(&Value::Null).symbols.is_empty());
    assert!(Reads::from_json(&json!({"symbols":[],"caught":true})).symbols.is_empty());
}

/// A strategy run and a run outside a task keep no reads: their context is none.
#[test]
fn a_run_outside_a_task_and_a_strategy_run_keep_no_reads() {
    assert!(Run::from_context(None).is_none());
}

/// `session remember` gives the task and its notes, then the window's views,
/// then the run's steps, within a small budget, and gives the same reply again.
#[test]
fn remember_gives_the_task_the_window_and_the_steps() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let log = store.root.join("run.jsonl");
    let step = json!({"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","tool":"solace","arguments":{"command":"view calc.add"},"result":{"content":[{"type":"text","text":"pub fn add"}]},"status":"completed"}});
    std::fs::write(&log, step.to_string()).unwrap();
    let s = board::open(&store.root).unwrap();
    let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":"Fix calc","body":"calc.add is off"})).unwrap();
    t["notes"] = json!([
        {"author":"human","text":"Keep the total."},
        {"author":"agent","text":"The plan is in the notes."}
    ]);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running","log":log.to_str().unwrap()}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    // Read both, then change `calc.add`: the window keeps what the change made differ.
    let mut reads = Reads::default();
    read_names(&store, &["calc.add", "calc.three"], &mut reads);
    store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int { b + a }").unwrap();
    let stores = opened(&top, &reads);
    assert!(reads.new_window(&|r| current_revision(&stores, r)));
    let ask = |budget| {
        WINDOW.with(|v| *v.borrow_mut() = Some(reads.to_json()));
        board::remember(&top, tid, "r1", budget).unwrap()
    };
    let first = ask(700);
    let second = ask(700);
    assert_eq!(first, second, "the ask is repeatable");
    let note = first.find("The plan is in the notes.").expect(&first);
    let view = first.find("pub fn add").expect(&first);
    let step = first.find("1 tool: solace view calc.add").expect(&first);
    assert!(note < view && view < step, "notes, then views, then steps: {first}");
    assert!(first.contains("What this run read or changed since its last change"), "{first}");
    assert!(first.contains("This run so far, one step per line:"), "{first}");
    // A small budget leaves out what does not fit and names it.
    let small = ask(190);
    assert!(small.len() <= 190, "{} bytes: {small}", small.len());
    assert!(small.contains("Not shown, to fit the size limit:"), "{small}");
}

/// The views a remember reply adds name what its size limit leaves out.
#[test]
fn remember_names_the_views_that_do_not_fit() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.add", "calc.three"], &mut reads);
    let out = remember_reply("Task: T\n", &reads.to_json(), &top, &|_| String::new(), 200);
    assert!(out.contains("Task: T"), "{out}");
    assert!(out.contains("Not shown, to fit the size limit: calc.add calc.three"), "{out}");
}

/// The reply keeps its size limit even when a remember reply leaves views out:
/// the headings and the notice naming them share the budget with the rest.
#[test]
fn remember_keeps_the_size_limit_when_it_names_left_out_views() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.add", "calc.three"], &mut reads);
    let out = remember_reply("Task: T\n", &reads.to_json(), &top, &|b| "s".repeat(b), 400);
    assert!(out.contains("Not shown, to fit the size limit: calc.add calc.three"), "{out}");
    assert!(out.len() <= 400, "the reply keeps its size limit: {} bytes", out.len());
}

/// A remember reply with no room names the task's notes and the left-out
/// views without opening their stores.
#[test]
fn remember_with_no_room_names_the_task_notes_and_left_out_views() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.add", "calc.three"], &mut reads);
    let out = remember_reply("Task: T\n\nNotes so far:\n- human: the plan\n", &reads.to_json(), &top, &|b| "x".repeat(b), 190);
    assert!(out.contains("Not shown, to fit the size limit: the task's notes calc.add calc.three"), "{out}");
    assert!(out.len() <= 190, "the reply keeps its size limit: {} bytes", out.len());
}

/// Deleting a symbol the run read is a change: it starts a new window that
/// keeps only what the change made differ.
#[test]
fn a_delete_of_a_read_symbol_starts_a_window() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.add", "calc.three"], &mut reads);
    store.delete_all(&["calc.three"], usize::MAX).unwrap();
    let stores = opened(&top, &reads);
    assert!(reads.new_window(&|r| current_revision(&stores, r)));
    assert!(reads.symbols.iter().any(|r| r.symbol == "calc.three" && r.recent && r.latest.is_empty()));
    assert!(reads.symbols.iter().any(|r| r.symbol == "calc.add" && !r.recent));
    assert!(reads.others(&[]).is_empty(), "a deleted symbol is no view");
}

/// A deleted symbol differs from every revision the run was served; a store
/// that won't open is no change.
#[test]
fn a_deleted_symbol_differs_and_an_unopened_store_does_not() {
    let store = testing::project(&[("src/calc.gleam", "pub fn one() -> Int {\n  1\n}\n")]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.one"], &mut reads);
    let stores = opened(&top, &reads);
    let r = reads.symbols[0].clone();
    assert!(current_revision(&stores, &r).is_some_and(|c| !c.is_empty()), "present: its revision");
    store.delete_all(&["calc.one"], usize::MAX).unwrap();
    let stores = opened(&top, &reads);
    assert_eq!(current_revision(&stores, &r), Some(String::new()), "deleted: differs from what was served");
    let gone = Served { project: "no/such/project".into(), ..r.clone() };
    assert_eq!(current_revision(&stores, &gone), None, "a store that won't open is no change");
}

/// Over the cap the oldest entry the window no longer needs goes first, so a
/// long window keeps the symbols it read.
#[test]
fn the_cap_drops_the_oldest_entry_the_window_no_longer_needs() {
    let read = |n: usize, recent: bool| Served { project: ".".into(), symbol: format!("a.f{n}"), revision: "1".into(), recent, latest: "1".into() };
    let mut reads = Reads::default();
    reads.record(&(0..=60).map(|i| read(i, i > 0)).collect::<Vec<_>>());
    assert_eq!(reads.symbols.len(), 60);
    assert!(!reads.symbols.iter().any(|r| r.symbol == "a.f0"), "the oldest entry that isn't recent goes first");
    assert!(reads.symbols.iter().any(|r| r.symbol == "a.f1"), "the oldest recent entry survives");
    let mut all = Reads::default();
    all.record(&(0..=60).map(|i| read(i, true)).collect::<Vec<_>>());
    assert_eq!(all.symbols.len(), 60);
    assert!(!all.symbols.iter().any(|r| r.symbol == "a.f0"), "when all are recent, the oldest goes");
}

/// A change in a call updates the window before the next command, so a
/// `session remember` in the same call shows the change, not the old read.
#[test]
fn a_change_before_remember_in_the_same_call_starts_its_window() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let s = board::open(&store.root).unwrap();
    let mut t = board::api(&s.root, "POST", "/tasks", json!({"title":"Fix calc"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let mut run = Run { top: top.clone(), tid: tid.into(), rid: "r1".into(), reads: Reads::default(), dirty: false };
    run.start_call();
    store.view(&["calc.add", "calc.three"], false, 10_000).unwrap();
    run.end_command(&["view".into(), "calc.add".into(), "calc.three".into()]);
    store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int { b + a }").unwrap();
    run.end_command(&["submit".into(), "calc.add".into()]);
    let out = board::remember(&top, tid, "r1", 10_000).unwrap();
    assert!(out.contains("b + a"), "the window shows the change: {out}");
    assert!(!out.contains("a + b"), "the window does not show the old body: {out}");
    assert!(!out.contains("pub fn three"), "the unchanged symbol is outside the window: {out}");
    SERVED.with(|v| v.borrow_mut().take());
    WINDOW.with(|v| v.borrow_mut().take());
}

/// `session remember` prints the window's views, but printing them is not a
/// read: the ask changes nothing, so a second ask is the same.
#[test]
fn remember_does_not_record_the_views_it_prints() {
    let store = testing::project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
    let top = scope::top(&store.root);
    let mut reads = Reads::default();
    read_names(&store, &["calc.add"], &mut reads);
    SERVED.with(|v| *v.borrow_mut() = Some(vec![]));
    let out = remember_reply("Task: T\n", &reads.to_json(), &top, &|_| String::new(), 10_000);
    assert!(out.contains("pub fn add"), "{out}");
    assert!(take_served().is_empty(), "the ask recorded a view as a read");
    SERVED.with(|v| v.borrow_mut().take());
}
