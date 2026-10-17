use super::*;

/// A run's reads keep each symbol once, at the revision served last, and come
/// back from the task store as they were.
#[test]
fn a_runs_reads_keep_each_symbol_once_and_round_trip() {
    let read = |s: &str, rev: &str| Served { project: ".".into(), symbol: s.into(), revision: rev.into(), recent: true, latest: rev.into() };
    let mut reads = Reads::default();
    reads.record(&[read("a.f", "1"), read("a.g", "1"), read("a.f", "2")]);
    assert_eq!(reads.symbols, [read("a.g", "1"), read("a.f", "2")]);
    assert!(reads.reread(&[read("a.f", "1")]).is_none(), "changed since it was served");
    assert_eq!(reads.reread(&[read("a.h", "1"), read("a.g", "1")]), Some(&read("a.g", "1")));
    reads.caught = true;
    assert!(reads.reread(&[read("a.g", "1")]).is_none(), "once a window");
    let back = Reads::from_json(&reads.to_json());
    assert_eq!((back.symbols, back.caught), (reads.symbols.clone(), true));
    assert!(Reads::from_json(&Value::Null).symbols.is_empty());
}

/// In a task run, re-reading a symbol unchanged since it was served adds the
/// window's other symbols and the run's outline, once a window; a re-read of a
/// symbol changed since is no catch, and a change keeps only what it changed
/// in the window. Other runs record nothing.
#[test]
fn a_reread_of_an_unchanged_symbol_adds_the_window_once() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let outline = |_: usize| "1 tool: view calc.add (completed)\n".to_string();
    let call = |reads: &mut Reads, names: &[&str], changed: bool| {
        Run::start_call();
        if !names.is_empty() {
            store.view(names, false, 10_000).unwrap();
        }
        catch_up(reads, &top, take_served(), changed, 10_000, &outline)
    };
    let mut reads = Reads::default();
    assert_eq!(call(&mut reads, &["calc.add", "calc.three"], false), "");
    // A failed change keeps the window, so the catch stays used.
    let out = call(&mut reads, &["calc.add"], false);
    assert!(out.contains("hasn't changed"), "{out}");
    assert_eq!(call(&mut reads, &[], true), "", "a change that changed nothing");
    assert_eq!(call(&mut reads, &["calc.add"], false), "", "still caught after a failed change");
    reads.caught = false;
    let out = call(&mut reads, &["calc.add"], false);
    assert!(out.contains("`calc.add` hasn't changed") && out.contains("pub fn three") && out.contains("1 tool: view calc.add"), "{out}");
    assert!(!out.contains("pub fn add"), "the re-read symbol is in the reply already: {out}");
    assert_eq!(call(&mut reads, &["calc.add"], false), "", "once a window");
    store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int { b + a }").unwrap();
    // Read in the same call as the change: no catch, and the window's catch is left.
    assert_eq!(call(&mut reads, &["calc.three"], true), "");
    assert_eq!(call(&mut reads, &["calc.add"], false), "", "changed since it was served");
    let out = call(&mut reads, &["calc.add"], false);
    assert!(out.contains("hasn't changed") && out.contains("pub fn three"), "read since the change: {out}");
    // `calc.add` stays changed from what was served, but a later change that
    // changes nothing starts no window, so the catch stays used.
    assert_eq!(call(&mut reads, &[], true), "");
    assert_eq!(call(&mut reads, &["calc.three"], false), "", "no window after a change that changed nothing");
    SERVED.with(|v| *v.borrow_mut() = None);
    store.view(&["calc.add"], false, 10_000).unwrap();
    assert!(take_served().is_empty() && SERVED.with(|v| v.borrow().is_none()));
}

/// A strategy run and a run outside a task keep no reads: their context is none.
#[test]
fn a_run_outside_a_task_and_a_strategy_run_keep_no_reads() {
    assert!(Run::from_context(None).is_none());
}

/// The views a catch adds name what the reply's size limit leaves out.
#[test]
fn a_catch_names_the_views_that_do_not_fit() {
    let store = testing::project(&[(
        "src/calc.gleam",
        "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n",
    )]);
    let top = scope::top(&store.root);
    let outline = |_: usize| String::new();
    let mut reads = Reads::default();
    Run::start_call();
    store.view(&["calc.add", "calc.three"], false, 10_000).unwrap();
    assert_eq!(catch_up(&mut reads, &top, take_served(), false, 10_000, &outline), "");
    Run::start_call();
    store.view(&["calc.add"], false, 10_000).unwrap();
    let out = catch_up(&mut reads, &top, take_served(), false, 200, &outline);
    assert!(out.contains("hasn't changed"), "{out}");
    assert!(out.contains("Not shown, to fit the size limit: calc.three"), "{out}");
    SERVED.with(|v| *v.borrow_mut() = None);
}
