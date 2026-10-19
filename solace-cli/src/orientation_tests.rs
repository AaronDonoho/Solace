use super::*;
use testing::*;

const SAMPLE: &str = r####"//! A module.
use std::fs;

/// The store.
#[derive(Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// Opens it.
    pub fn open() -> Self {
        let brace = '{';
        let s = "}";
        Store { root: PathBuf::new() }
    }

    pub(crate) fn view(&self, names: &[&str]) -> Out {
        Ok(String::new())
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

pub fn view() {}

const GLEAM: &[&str] = &[
    "submit",
];

pub fn outer() {
    fn nested() {}
    let src = r#"pub fn main() {
}
fn fake() {
}"#;
}

#[cfg(test)]
// The tests are here.
#[path = "sample_tests.rs"]
mod more;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opens() {
        assert!(Store::open().root.as_os_str().is_empty());
    }
}
"####;

const PY: &str = "import os\n\n\nclass View(unittest.TestCase):\n    \"\"\"Views.\"\"\"\n\n    def setUp(self):\n        self.x = '''\ndef fake():\n'''\n        x = \"'''\"\n\n    @skip\n    def test_one(self):\n        def inner():\n            pass\n        inner()\n\n\ndef helper(a,\n           b):\n    \"\"\"Helps.\"\"\"\n    return a\n";

#[test]
fn rust_items_take_their_docs_and_skip_strings_and_nested_fns() {
    let items = rust(SAMPLE).unwrap();
    let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
    assert_eq!(
        got,
        [
            ("Store", 4, 8, 0),
            ("Store", 10, 21, 0),
            ("Store::open", 11, 16, 1),
            ("Store::view", 18, 20, 1),
            ("<Failure as fmt::Display>", 23, 25, 0),
            ("<Failure as fmt::Display>::fmt", 24, 24, 1),
            ("view", 27, 27, 0),
            ("GLEAM", 29, 31, 0),
            ("outer", 33, 39, 0),
            ("more", 41, 44, 0),
            ("tests", 46, 53, 0),
            ("tests::opens", 49, 52, 1),
        ]
    );
    let sig = |n: &str, k: &str| items.iter().find(|i| i.name == n && i.kind == k).unwrap().sig.clone();
    assert_eq!(sig("Store", "struct"), "pub struct Store");
    assert_eq!(sig("<Failure as fmt::Display>", "impl"), "impl fmt::Display for Failure");
    assert_eq!(sig("Store::view", "fn"), "pub(crate) fn view(&self, names: &[&str]) -> Out");
    assert_eq!(sig("GLEAM", "const"), "const GLEAM: &[&str]");
    assert_eq!(sig("more", "mod"), "mod more → sample_tests.rs");
    assert_eq!(items[0].doc, "The store.");
}

#[test]
fn the_signature_starts_after_a_wrapped_attribute_and_imports_are_left_out() {
    let items = rust("use a::b;\n\nfn x() {}\n\nuse c::d;\n\n#[derive(\n    Debug,\n)]\n/// A.\npub struct A {\n}\n").unwrap();
    let got: Vec<(&str, &str, &str)> = items.iter().map(|i| (i.name.as_str(), i.sig.as_str(), i.doc.as_str())).collect();
    assert_eq!(got, [("x", "fn x()", ""), ("A", "pub struct A", "A.")]);
}

#[test]
fn names_match_in_full_or_by_suffix() {
    // A plain `//` comment above an item's doc is not part of the item (D-119 point 4).
    let store = |t: &str| {
        let i = rust(t).unwrap().into_iter().find(|i| i.kind == "struct").unwrap();
        (i.start, i.end, i.doc)
    };
    assert_eq!(store("// A note.\n/// The store.\npub struct Store {\n}\n"), (2, 4, "The store.".to_string()));
    assert_eq!(store("/// The store.\n// a note\npub struct Store {\n}\n"), (1, 4, "The store.".to_string()));
    let items = rust(SAMPLE).unwrap();
    let one = |n: &str| find(&items, n, "::").iter().map(|i| (i.name.clone(), i.kind)).collect::<Vec<_>>();
    assert_eq!(one("Store"), [("Store".to_string(), "struct")], "the type, not its impl");
    assert_eq!(one("view"), [("view".to_string(), "fn")], "a full name wins over a suffix");
    assert_eq!(one("Store::view"), [("Store::view".to_string(), "fn")]);
    assert_eq!(one("opens"), [("tests::opens".to_string(), "fn")]);
    assert!(one("nested").is_empty() && one("fake").is_empty() && one("main").is_empty());
    let two = rust("impl A {\n    fn new() {}\n}\nimpl B {\n    fn new() {}\n}\n").unwrap();
    assert_eq!(find(&two, "new", "::").len(), 2);
}

#[test]
fn python_items_follow_indentation() {
    let items = python(PY);
    let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
    assert_eq!(got, [("View", 4, 17, 0), ("View.setUp", 7, 11, 1), ("View.test_one", 13, 17, 1), ("helper", 20, 23, 0)]);
    assert_eq!((items[0].doc.as_str(), items[3].doc.as_str(), items[3].sig.as_str()), ("Views.", "Helps.", "def helper(a, b)"));
}

#[test]
fn a_quote_in_a_comment_opens_no_string() {
    // A quote in a comment opens no string, so the line below it is code.
    let items = python("x = 1 # '''\ndef a():\n    pass\n");
    let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
    assert_eq!(got, [("a", 2, 3, 0)]);
}

#[test]
fn an_escaped_quote_closes_no_string() {
    // `\"` in a one-line string is not its end, so the `'''` after it
    // opens no string and the line below is code.
    let items = python("x = \"a\\\"b'''\"\ndef a():\n    pass\n");
    let got: Vec<(&str, usize, usize, usize)> = items.iter().map(|i| (i.name.as_str(), i.start, i.end, i.level)).collect();
    assert_eq!(got, [("a", 2, 3, 0)]);
}

#[test]
fn view_gives_an_outline_or_an_item_and_names_a_miss() {
    let dir = Scratch::new().with(&[("src/lib.rs", SAMPLE), ("src/other/lib.rs", "fn a() {}\n"), ("tests/check.py", PY)]);
    let v = toml::Value::Table(Default::default());
    let run = |a: &[&str], budget: usize| view(&dir, &v, a, budget).expect("a file form");
    let outline = run(&["view", "src/lib.rs"], 32_000).unwrap();
    assert!(outline.starts_with("src/lib.rs: 53 lines, 12 items\n"), "{outline}");
    assert!(outline.contains("  4..8 pub struct Store — The store.\n"), "{outline}");
    assert!(outline.contains("    11..16 pub fn open() -> Self — Opens it.\n"), "{outline}");
    assert!(outline.contains("  41..44 mod more → sample_tests.rs\n"), "{outline}");
    let item = run(&["view", "src/lib.rs::Store::open"], 32_000).unwrap();
    assert_eq!(
        item,
        "src/lib.rs:11..16\n11:     /// Opens it.\n12:     pub fn open() -> Self {\n13:         let brace = '{';\n14:         let s = \"}\";\n15:         Store { root: PathBuf::new() }\n16:     }\n"
    );
    let e = run(&["view", "lib.rs"], 32_000).unwrap();
    assert!(e.contains("E1002") && e.contains("src/other/lib.rs"), "{e}");
    let e = run(&["view", "src/lib.rs::nested"], 32_000).unwrap();
    assert!(e.contains("E1001") && e.contains("grep nested --in src/lib.rs --context 3"), "{e}");
    let broken = Scratch::new().with(&[("src/bad.rs", "fn a() {}\nfn b( {}\n")]);
    let e = view(&broken, &v, &["view", "src/bad.rs"], 32_000).unwrap().unwrap();
    assert!(e.starts_with("src/bad.rs: line 2: ") && e.ends_with("`read src/bad.rs` reads it.\n"), "{e}");
    // A refusal of one operand is its own part of the reply: the other is answered.
    let mixed = run(&["view", "src/lib.rs::nested", "src/other/lib.rs"], 32_000).unwrap();
    assert!(
        mixed.contains("E1001") && mixed.contains("grep nested --in src/lib.rs --context 3") && mixed.contains("src/other/lib.rs: 1 lines, 1 items\n"),
        "{mixed}"
    );
    assert!(run(&["view", "check.py::test_one"], 32_000).unwrap().starts_with("tests/check.py:13..17\n13:     @skip\n"));
    // Over the budget: names only, then a cut at a whole line.
    let short = run(&["view", "src/lib.rs"], outline.len() - 1).unwrap();
    assert!(short.contains("  4..8 struct Store\n") && short.contains("    11..16 fn Store::open\n"), "{short}");
    assert!(short.contains("Signatures left out to fit the reply") && !short.contains("The store."), "{short}");
    let cut = run(&["view", "src/lib.rs"], 280).unwrap();
    assert!(cut.contains("more items left out to fit the reply") && cut.len() < 400, "{cut}");
    // Symbols are the store's, and the two don't mix.
    assert!(view(&dir, &v, &["view", "app.main"], 32_000).is_none());
    assert!(run(&["view", "src/lib.rs", "app.main"], 32_000).unwrap_err().0.contains("not both"));
    let gleam = Scratch::new().with(&[("gleam.toml", "name = \"app\"\n")]);
    assert!(view(&gleam, &v, &["view", "app.rs"], 32_000).is_none(), "a Gleam symbol `app.rs`");
}

#[test]
fn the_scanner_reads_this_crate() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(&src).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&p).unwrap();
        let lines = text.lines().count();
        let mut last = 0;
        for it in rust(&text).unwrap().iter().filter(|i| i.level == 0) {
            assert!(last < it.start && it.start <= it.end && it.end <= lines, "{}: {it:?}", p.display());
            last = it.end;
        }
    }
    let read = |f: &str| rust(&fs::read_to_string(src.join(f)).unwrap()).unwrap();
    let board = read("board.rs");
    assert!(board.iter().any(|i| i.name == "tests" && i.sig.ends_with("→ board_tests.rs")));
    assert!(board.iter().any(|i| i.name == "brief" && i.kind == "fn"));
    let view = read("view.rs");
    assert!(!view.iter().any(|i| i.sig.contains("fn write(path: String")), "Gleam in a raw string is not an item");
    assert!(view.iter().any(|i| i.name == "tests::c1_graph_has_entries_effects_and_tests"));
    let lockdown = read("lockdown.rs");
    assert!(lockdown.iter().any(|i| i.name == "dispatch"));
    // The top-level `drop_records`, not the one nested in `carry_file`.
    assert_eq!(lockdown.iter().filter(|i| i.name.ends_with("drop_records")).count(), 1);
}
