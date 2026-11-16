use super::*;
#[test]
fn broken_symbols_counted_once() {
    let d = |sym: &str| Diag { error: true, module: None, symbol: Some(sym.into()), text: format!("error: {sym}"), suggestions: vec![] };
    let out = broken_report(&HashMap::new(), &[d("a.f"), d("a.g"), d("a.f")]);
    assert!(out.contains("Broken symbols (2): a.f, a.g"), "{out}");
}
#[test]
fn grouped_privacy_errors_name_the_fix_once() {
    let mut diags: Vec<Diag> = (0..6)
        .map(|n| Diag {
            error: true,
            module: None,
            symbol: Some(format!("crate.site{n}")),
            text: format!("error[E0603]: function `helper` is private\n  at crate.site{n}, line 1"),
            suggestions: vec![],
        })
        .collect();
    diags.extend((0..6).map(|n| Diag {
        error: true,
        module: None,
        symbol: Some(format!("crate.call{n}")),
        text: format!("error[E0599]: no method named `open` found for struct `Store` in the current scope\n  at crate.call{n}, line 1"),
        suggestions: vec![],
    }));
    diags.push(Diag {
        error: true,
        module: None,
        symbol: Some("crate.other".into()),
        text: "error: something else".into(),
        suggestions: vec![],
    });
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`helper` is private (6 places): make it `pub(crate)`"), "{out}");
    assert!(out.contains("`open` is missing on `Store` (6 places): add the method or fix the call"), "{out}");
    assert_eq!(out.matches("error: something else").count(), 1, "{out}");
    for n in 0..6 {
        assert!(out.contains(&format!("at crate.site{n}, line 1")), "{out}");
    }
}

#[test]
fn grouped_sites_are_capped_and_name_their_modules() {
    let modules = ["shown", "shown", "shown", "shown", "shown", "shown", "board", "drafts", "board", "lockdown", "drafts", "lockdown"];
    let diags: Vec<Diag> =
        modules.iter().copied().enumerate().map(|(n, module)| grouped_site(None, Some(format!("{module}.site{n}")))).collect();
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`0` is private on `Failure` (12 places): make it `pub(crate)`"), "{out}");
    for n in 0..6 {
        assert!(out.contains(&format!("at shown.site{n}, line 1")), "{out}");
    }
    assert!(!out.contains("at board.site6, line 1"), "{out}");
    assert!(out.contains("… and 6 more in board, drafts, lockdown"), "{out}");
}

#[test]
fn grouped_sites_name_the_located_module_once() {
    let diags: Vec<Diag> = (0..8).map(|n| grouped_site(Some("board"), Some(format!("crate::call{n}")))).collect();
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`0` is private on `Failure` (8 places): make it `pub(crate)`"), "{out}");
    for n in 0..6 {
        assert!(out.contains(&format!("at crate::call{n}, line 1")), "{out}");
    }
    assert!(!out.contains("at crate::call6, line 1"), "{out}");
    assert!(out.contains("… and 2 more in board"), "{out}");
}

#[test]
fn grouped_sites_without_a_module_still_name_the_count() {
    let diags: Vec<Diag> = (0..7).map(|_| grouped_site(None, None)).collect();
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`0` is private on `Failure` (7 places): make it `pub(crate)`"), "{out}");
    assert!(out.contains("… and 1 more"), "{out}");
    assert!(!out.contains("… and 1 more in"), "{out}");
}

#[test]
fn a_cause_at_the_cap_has_no_more_line() {
    let diags: Vec<Diag> = (0..6).map(|n| grouped_site(Some("board"), Some(format!("board.site{n}")))).collect();
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`0` is private on `Failure` (6 places): make it `pub(crate)`"), "{out}");
    assert!(!out.contains("… and"), "{out}");
}

fn grouped_site(module: Option<&str>, symbol: Option<String>) -> Diag {
    let at = symbol.as_deref().unwrap_or("?").to_string();
    Diag {
        error: true,
        module: module.map(str::to_string),
        symbol,
        text: format!("error[E0616]: field `0` of struct `Failure` is private\n  at {at}, line 1"),
        suggestions: vec![],
    }
}

#[test]
fn an_osc_hyperlink_keeps_only_its_text() {
    let link = "\u{1b}]8;;https://example.com\u{1b}\\text\u{1b}]8;;\u{1b}\\";
    assert_eq!(strip_ansi(link), "text");
    assert_eq!(strip_ansi("\u{1b}]0;title\u{7}done"), "done");
    assert_eq!(strip_ansi("a\u{1b}]8;;unterminated"), "a");
}

#[test]
fn a_stray_escape_keeps_the_text_after_it() {
    assert_eq!(strip_ansi("a\u{1b} 123 lost text"), "a 123 lost text");
}

#[test]
fn color_and_charset_escapes_are_dropped() {
    assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}(B\u{1b}[m"), "red");
    assert_eq!(strip_ansi("\u{1b}[31"), "");
}

#[test]
fn two_byte_escapes_are_dropped() {
    assert_eq!(strip_ansi("\u{1b}7saved\u{1b}8"), "saved");
}

#[test]
fn an_errors_kind_is_its_title_and_caret_note() {
    let text = "error: Incorrect arity\n  --> a.gleam\n   | ^ Expected 6 arguments, got 5";
    assert_eq!(error_kind(text), "Incorrect arity: Expected 6 arguments, got 5");
    assert_eq!(error_kind("error: Missing value"), "Missing value");
}

#[test]
fn a_stored_error_matches_its_title_and_caret_note() {
    let stored = "error: Incorrect arity\n   | ^ Expected 6 arguments, got 5";
    let d = |text: &str| Diag { error: true, module: None, symbol: Some("a.f".into()), text: text.into(), suggestions: vec![] };
    assert!(stored_has_kind(stored, &d(stored)));
    assert!(!stored_has_kind(stored, &d("error: Incorrect arity\n   | ^ Expected 6 arguments, got 4")));
    assert!(!stored_has_kind(stored, &d("error: Unknown variable\n   | ^ x")));
}

#[test]
fn a_short_assertion_block_is_unchanged() {
    let block =
        "assert test/app_test.gleam:3\n test: app_test.wrong_test\n code: assert 1 + 1 == 3\n left: 2\nright: 3\n info: Assertion failed.";
    assert_eq!(shorten_assertions(block), block);
}

#[test]
fn a_long_value_shows_the_short_expected_text() {
    let page = format!("{:?}", format!("<button data-note=\"1\">{}</button>", "x".repeat(400)));
    let expected = format!("{:?}", "data-note=\"1, 2\"");
    let block = format!(
        "assert test/design_test.gleam:10\n test: design_test.notes_test\n \
         code: assert string.contains(rendered, \"data-note=\\\"1, 2\\\"\")\n \
         0: {page}\n 1: {expected}\n info: Assertion failed."
    );
    let out = shorten_assertions(&block);
    assert!(out.contains("data-note=\"1\">"), "{out}");
    assert!(out.contains("chars; test --full shows it whole)"), "{out}");
    assert!(!out.contains(&"x".repeat(200)), "{out}");
    assert!(out.contains(" test: design_test.notes_test"), "{out}");
    assert!(out.contains(" code: assert string.contains"), "{out}");
    assert!(out.contains(" info: Assertion failed."), "{out}");
}

#[test]
fn two_long_values_show_where_they_first_differ() {
    let left = format!("{:?}", format!("{}LEFT{}", "a".repeat(200), "b".repeat(200)));
    let right = format!("{:?}", format!("{}RIGHT{}", "a".repeat(200), "b".repeat(200)));
    let block = format!("assertion `left == right` failed\n  left: {left}\n right: {right}");
    let out = shorten_assertions(&block);
    assert!(out.contains("LEFT") && out.contains("RIGHT"), "{out}");
    assert_eq!(out.matches("chars; test --full shows it whole)").count(), 2, "{out}");
    assert!(!out.contains(&"a".repeat(200)), "{out}");
}

#[test]
fn a_colored_value_label_is_shortened() {
    let block = format!("\u{1b}[36m left\u{1b}[39m: {}", "z".repeat(300));
    let out = shorten_assertions(&block);
    assert!(out.starts_with("\u{1b}[36m left\u{1b}[39m: "), "{out}");
    assert!(out.ends_with("(300 chars; test --full shows it whole)"), "{out}");
}

/// A numbered value line in a test's own output is not an assertion value.
#[test]
fn a_value_line_outside_an_assertion_block_is_unchanged() {
    let line = format!(" 0: {}", "y".repeat(300));
    assert_eq!(shorten_assertions(&line), line);
}
