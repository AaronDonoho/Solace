use super::*;
use crate::diagnostics::Suggestion;

fn path(text: &str) -> Vec<String> {
    text.split("::").map(str::to_string).collect()
}

fn suggestion(message: &str, replacement: &str) -> Suggestion {
    Suggestion {
        message: message.to_string(),
        spans: vec![SuggestionSpan {
            file_name: "src/a.rs".into(),
            line_start: 1,
            line_end: 1,
            column_start: 1,
            column_end: 1,
            replacement: Some(replacement.into()),
        }],
    }
}

#[test]
fn a_lone_in_crate_import_is_chosen() {
    let candidates = vec![Candidate { line: "use crate::t::value;".into(), path: path("crate::t::value"), trait_import: false }];
    let old = OldBindings { by_module: HashMap::new() };
    assert_eq!(choose(&candidates, &old, "b").map(|c| c.line.as_str()), Some("use crate::t::value;"));
}

#[test]
fn two_candidates_without_a_former_binding_are_left_alone() {
    let candidates = vec![
        Candidate { line: "use crate::a::twin;".into(), path: path("crate::a::twin"), trait_import: false },
        Candidate { line: "use crate::b::twin;".into(), path: path("crate::b::twin"), trait_import: false },
    ];
    let old = OldBindings { by_module: HashMap::new() };
    assert!(choose(&candidates, &old, "caller").is_none());
}

#[test]
fn the_former_binding_picks_one_of_several_candidates() {
    let candidates = vec![
        Candidate { line: "use crate::t::Duration;".into(), path: path("crate::t::Duration"), trait_import: false },
        Candidate { line: "use std::time::Duration;".into(), path: path("std::time::Duration"), trait_import: false },
    ];
    let mut old = OldBindings { by_module: HashMap::new() };
    old.by_module.insert(("caller".into(), "Duration".into()), vec![path("crate::t::Duration")]);
    assert_eq!(choose(&candidates, &old, "caller").map(|c| c.line.as_str()), Some("use crate::t::Duration;"));
}

#[test]
fn a_similar_name_guess_is_not_an_import_candidate() {
    let diag = Diag {
        error: true,
        module: Some("caller".into()),
        symbol: None,
        text: "error[E0432]: unresolved import `crate::graph_x`".into(),
        suggestions: vec![suggestion("there is a crate or module with a similar name", "use crate::graph;")],
    };
    assert!(import_candidates(&diag).is_empty());
}

#[test]
fn a_bare_path_is_not_an_import_candidate() {
    assert!(candidate("self::graph::direct_effects", "import").is_none());
    assert!(candidate("use crate::reach::flow::origins;", "import").is_some());
}

#[test]
fn private_errors_name_their_item() {
    let diag = |text: &str| Diag { error: true, module: None, symbol: None, text: text.into(), suggestions: vec![] };
    assert_eq!(private_error(&diag("error[E0603]: function `helper` is private")), Some(("E0603", "helper".into())));
    assert_eq!(private_error(&diag("error[E0624]: method `open` is private")), Some(("E0624", "open".into())));
    assert_eq!(private_error(&diag("error[E0616]: field `value` of struct `S` is private")), Some(("E0616", "value".into())));
    assert!(private_error(&diag("error[E0001]: something else")).is_none());
}

#[test]
fn a_top_level_symbol_is_a_former_binding() {
    let candidates = vec![
        Candidate { line: "use crate::old::helper;".into(), path: path("crate::old::helper"), trait_import: false },
        Candidate { line: "use crate::other::helper;".into(), path: path("crate::other::helper"), trait_import: false },
    ];
    let mut old = OldBindings { by_module: HashMap::new() };
    record_symbol(&mut old.by_module, "old", "helper");
    assert_eq!(choose(&candidates, &old, "old").map(|c| c.line.as_str()), Some("use crate::old::helper;"));
}
