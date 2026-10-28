use super::*;
#[test]
fn broken_symbols_counted_once() {
    let d = |sym: &str| Diag { error: true, module: None, symbol: Some(sym.into()), text: format!("error: {sym}") };
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
        })
        .collect();
    diags.extend((0..6).map(|n| Diag {
        error: true,
        module: None,
        symbol: Some(format!("crate.call{n}")),
        text: format!("error[E0599]: no method named `open` found for struct `Store` in the current scope\n  at crate.call{n}, line 1"),
    }));
    diags.push(Diag { error: true, module: None, symbol: Some("crate.other".into()), text: "error: something else".into() });
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
    }
}
