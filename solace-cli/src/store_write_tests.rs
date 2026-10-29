use crate::testing::*;
use super::*;
    /// A dead-code warning the submit leaves on another symbol is named under
    /// its own heading.
    #[test]
    fn a_submit_names_a_warning_it_leaves_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(out.contains("New warnings elsewhere:"), "{out}");
        assert!(out.contains("app.tools: warning:"), "{out}");
    }

    /// A warning that was there before the submit stays quiet.
    #[test]
    fn a_submit_does_not_repeat_a_warning_from_before() {
        let s = project(&[("src/app.gleam", "fn unused() {\n  1\n}\n\npub fn main() {\n  1\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  2\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
        assert!(!out.contains("app.unused"), "{out}");
    }

    /// Before any check has run there is no baseline, so no warning is new.
    #[test]
    fn a_submit_before_a_check_does_not_name_a_warning_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
    }

    /// Both warning sections share one footer.
    #[test]
    fn a_submit_prints_the_diagnostics_footer_once() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  todo\n}\n").unwrap();
        assert!(out.contains("\nWarnings:"), "{out}");
        assert!(out.contains("\nNew warnings elsewhere:"), "{out}");
        assert_eq!(out.matches("Use `check --warnings` for full diagnostics.").count(), 1, "{out}");
    }

#[test]
fn only_gleams_unknown_module_error_names_a_qualifier() {
    assert_eq!(unknown_module("Unknown module type\n\nThe module `helper` does not have a `Foo` type."), None);
    assert_eq!(unknown_module("Unknown module value\n\nThe module `helper` does not have a `x` value."), None);
    assert_eq!(unknown_module("Unknown module\n\nThe module `helper` does not exist."), Some("helper"));
    assert_eq!(unknown_module("No module has been found with the name `sqlight`."), Some("sqlight"));
}

#[test]
fn dependency_modules_come_from_direct_manifest_entries_only() {
    let root = Scratch::new().with(&[
        (
            "gleam.toml",
            "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ngleam_json = \"1.0\"\n\n[dev-dependencies]\ngleeunit = \"1.0\"\n",
        ),
        ("build/packages/gleam_json/src/gleam/json.gleam", ""),
        ("build/packages/gleam_stdlib/src/gleam/list.gleam", ""),
        ("build/packages/gleeunit/src/gleeunit.gleam", ""),
    ]);
    let modules = dependency_modules(&root);
    assert_eq!(modules.iter().map(|m| (m.path.as_str(), m.test)).collect::<Vec<_>>(), vec![("gleam/json", false), ("gleeunit", true)]);
}

#[test]
fn an_alias_to_an_unlisted_dependency_is_not_reused() {
    let store = project(&[
        ("build/packages/indirect/src/indirect.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/caller.gleam", "import indirect as x\n\npub fn call() -> Int {\n  x.value()\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let ix = store.index().unwrap();
    let deps = store.dependency_modules();
    assert!(matches!(import_match(&ix, &deps, "x", false), ImportMatch::None));
}

