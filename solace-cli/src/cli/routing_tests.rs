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

