use super::*;
use crate::testing::Scratch;
use std::fs;
use std::path::{Path, PathBuf};

const SRC: &str = r#"//! Doc.
use std::fmt;

// Opens.
/// The store.
pub struct Store<T> {
    t: T,
}

impl<T> Store<T> {
    pub fn open() {}
}

// section

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self { Failure } // trailing
}

mod tests {
    use super::*;
    fn opens() {}
}

unsafe extern "C" {
    fn getpid() -> i32;
}

fn capture() -> impl Fn() + use<> {
    || {}
}
"#;

fn names(pieces: &[Piece], out: &mut Vec<String>) {
    for p in pieces {
        out.push(p.name.clone());
        names(&p.children, out);
    }
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn messages(source: &str) -> Vec<(usize, String, String)> {
    split(source, "board").unwrap().refusals.into_iter().map(|r| (r.line, r.quote, r.message)).collect()
}

const CARGO: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";

/// The (file, module) pairs of a package of `files`, or why it is refused.
fn package_of(files: &[(&str, &str)]) -> Result<Vec<(String, String)>, String> {
    let root = Scratch::new().with(files);
    package(&root).map(|p| p.into_iter().map(|(path, module, _)| (path, module)).collect()).map_err(|e| e.0)
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected.iter().map(|(p, m)| (p.to_string(), m.to_string())).collect()
}

#[test]
fn every_file_of_this_crate_joins_back_exactly() {
    let mut paths = vec![];
    rs_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut paths);
    assert!(paths.len() > 20, "{paths:?}");
    fn ends(text: &str, pieces: &[Piece], file: &Path) {
        for p in pieces {
            assert!(text[p.item.clone()].ends_with(['}', ';']), "{}: {} is cut at its end", file.display(), p.name);
            ends(text, &p.children, file);
        }
    }
    for p in paths {
        let text = fs::read_to_string(&p).unwrap();
        let f = split(&text, "crate").unwrap_or_else(|e| panic!("{}: {}", p.display(), e.0));
        assert!(f.join() == text, "{} joins back", p.display());
        let refused: Vec<String> = f.refusals.iter().map(|r| format!("{}: {}", r.line, r.message)).collect();
        assert!(refused.is_empty(), "{}: {refused:?}", p.display());
        ends(&text, &f.items, &p);
    }
}

#[test]
fn items_take_their_leading_comments_and_are_named_in_rust_syntax() {
    let f = split(SRC, "crate").unwrap();
    assert_eq!(f.join(), SRC);
    assert!(f.refusals.is_empty());
    assert_eq!(f.header, "//! Doc.\nuse std::fmt;\n\n");
    let mut got = vec![];
    names(&f.items, &mut got);
    assert_eq!(
        got,
        [
            "Store",
            "Store<T>",
            "Store<T>::open",
            "<Failure as From<std::io::Error>>",
            "<Failure as From<std::io::Error>>::from",
            "tests",
            "tests::opens",
            "extern \"C\"",
            "getpid",
            "capture"
        ]
    );
    assert!(f.items[0].text.starts_with("// Opens.\n/// The store.\npub struct Store<T>"));
    // A comment with a blank line below it stays with the item above.
    assert!(f.items[1].tail.ends_with("}\n\n// section\n\n"), "{:?}", f.items[1].tail);
    assert!(f.items[2].tail.starts_with(" // trailing\n}"), "{:?}", f.items[2].tail);
    assert_eq!(f.items[3].text, "mod tests {\n    use super::*;\n");
    assert_eq!(f.items[1].children[0].kind, "fn");
}

#[test]
fn a_syntax_error_names_its_line() {
    let e = split("fn a() {}\nfn b( {}\n", "crate").err().unwrap();
    assert!(e.0.starts_with("line 2: ") && e.0.ends_with("`fn b( {}`"), "{}", e.0);
}

#[test]
fn the_same_name_twice_is_refused() {
    let twice = messages("#[cfg(unix)]\nfn f() {}\n#[cfg(windows)]\nfn f() {}\n");
    assert_eq!(
        twice,
        [(
            4,
            "fn f() {}".to_string(),
            "Not saved: `f` is defined twice in `board` (`#[cfg(unix)]`, `#[cfg(windows)]`). Keep one `f` and put the platform code inside it: `#[cfg(unix)] { … }` or `if cfg!(unix) { … }`.".to_string()
        )]
    );
    let methods = messages("impl A {\n    fn f() {}\n}\nimpl A {\n    fn f() {}\n}\nconst _: () = ();\nconst _: () = ();\n");
    assert_eq!(methods.len(), 1);
    assert!(methods[0].2.starts_with("Not saved: `A::f` is defined twice in `board`. Keep one `A::f`"), "{methods:?}");
    let inner = messages("mod tests {\n    fn t() {}\n    fn t() {}\n}\nfn t() {}\n");
    assert_eq!(inner.len(), 1);
    assert!(inner[0].2.starts_with("Not saved: `t` is defined twice in `board::tests`."), "{inner:?}");
}

#[test]
fn item_making_macros_and_include_are_refused() {
    let lazy = messages("lazy_static! {\n    static ref X: u8 = 1;\n}\n");
    assert_eq!(
        lazy,
        [(1, "lazy_static! {".to_string(), "Not saved: `lazy_static! { … }` at module level makes items Solace can't hold as symbols. Write them out: `static X: LazyLock<T> = LazyLock::new(|| …);`.".to_string())]
    );
    let include = messages("include!(\"x.rs\");\n");
    assert_eq!(
        include,
        [(1, "include!(\"x.rs\");".to_string(), "Not saved: `include!(\"x.rs\")` pastes in code Solace doesn't hold. Make it a module (`mod x;`); for data, use `include_str!(\"x.txt\")`.".to_string())]
    );
    let allowed = split("macro_rules! m {\n    () => {};\n}\nthread_local! {\n    static DB: u8 = 1;\n}\n", "board").unwrap();
    assert!(allowed.refusals.is_empty());
    assert_eq!(allowed.items.iter().map(|p| (p.kind, p.name.as_str())).collect::<Vec<_>>(), [("macro_rules!", "m"), ("thread_local!", "DB")]);
}

#[test]
fn a_path_outside_the_parents_folder_is_refused() {
    let out = messages("#[path = \"../x.rs\"]\nmod x;\n#[path = \"board_tests.rs\"]\nmod tests;\n");
    assert_eq!(
        out,
        [(1, "#[path = \"../x.rs\"]".to_string(), "Not saved: `#[path = \"../x.rs\"]` is outside its parent's folder. A `#[path]` module is a `.rs` file beside its parent: `#[path = \"board_tests.rs\"] mod tests;`.".to_string())]
    );
    let cfg = messages("#[cfg_attr(unix, path = \"os/unix.rs\")]\nmod os;\n#[cfg_attr(test, path = \"os_tests.rs\")]\nmod t;\n");
    assert_eq!(cfg.len(), 1);
    assert!(cfg[0].2.starts_with("Not saved: `#[path = \"os/unix.rs\"]`"), "{cfg:?}");
    let spellings = messages("#[path=\"../a.rs\"]\nmod a;\n#[path = r\"../b.rs\"]\nmod b;\n#[cfg_attr(unix, path=\"os/c.rs\")]\nmod c;\n");
    assert_eq!(spellings.iter().map(|m| m.0).collect::<Vec<_>>(), [1, 3, 5], "{spellings:?}");
    let nested = messages("#[cfg_attr(unix, cfg_attr(test, path = \"../x.rs\"))]\nmod x;\n");
    assert_eq!(nested.len(), 1, "{nested:?}");
    let f = split("fn a() {}\n\n#[path=\"x_tests.rs\"]\nmod t;\n", "crate").unwrap();
    assert_eq!(f.items[1].file.as_deref(), Some("x_tests.rs"));
}
#[test]
fn a_bom_and_a_shebang_stay_in_the_header() {
    for prefix in ["\u{feff}", "#!/usr/bin/env run-cargo-script\n", "\u{feff}#!/x\n"] {
        let source = format!("{prefix}/// A.\nfn a() {{}}\n\nfn b() {{}}\n");
        let f = split(&source, "crate").unwrap();
        assert_eq!(f.join(), source);
        assert_eq!(f.header, prefix);
        assert_eq!(&source[f.items[1].item.clone()], "fn b() {}");
        assert_eq!(&source[f.items[0].head..f.items[0].item.end], "fn a() {}");
    }
    let e = split("#!/x\nfn b( {}\n", "crate").err().unwrap();
    assert!(e.0.starts_with("line 2: "), "{}", e.0);
    // An inner attribute is no shebang.
    assert_eq!(split("#![allow(dead_code)]\nfn a() {}\n", "crate").unwrap().header, "#![allow(dead_code)]\n");
}

#[test]
fn the_head_follows_attributes_docs_and_comments() {
    let source = "#[derive(\n    Debug,\n)]\n/* note */\n// more\nstruct A;\n";
    let p = &split(source, "crate").unwrap().items[0];
    assert_eq!(&source[p.head..p.item.end], "struct A;");
}

#[test]
fn the_head_follows_a_nested_block_comment() {
    let source = "#[derive(Debug)]\n/* a /* b */ c */\nstruct A;\n";
    let p = &split(source, "crate").unwrap().items[0];
    assert_eq!(&source[p.head..p.item.end], "struct A;");
}

#[test]
fn a_negative_impl_keeps_its_bang() {
    let f = split("impl !Send for X {}\n", "crate").unwrap();
    assert_eq!(f.items[0].name, "<X as !Send>");
}

#[test]
fn leading_file_modules_stay_in_the_header_and_every_one_is_listed() {
    let source = "//! Doc.\n#![allow(dead_code)]\nmod a;\nuse b::c;\npub mod d;\n\nfn f() {}\n\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n\nmod inline {\n    mod e;\n}\n";
    let f = split(source, "crate").unwrap();
    assert_eq!(f.join(), source);
    assert_eq!(f.header, "//! Doc.\n#![allow(dead_code)]\nmod a;\nuse b::c;\npub mod d;\n\n");
    assert_eq!(f.mods, [("a".to_string(), None), ("d".to_string(), None), ("tests".to_string(), Some("x_tests.rs".to_string()))]);
    assert_eq!(f.items.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["f", "tests", "inline"]);
    assert_eq!(f.items[2].children[0].name, "inline::e");
}

#[test]
fn a_package_reaches_its_files_each_way_a_module_names_one() {
    let got = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/main.rs", "mod a;\nmod b;\n\nfn main() {}\n\n#[cfg(test)]\n#[path = \"main_tests.rs\"]\nmod tests;\n"),
        ("src/a.rs", "mod c;\n\n#[path = \"a_tests.rs\"]\nmod tests;\n"),
        ("src/a/c.rs", "fn c() {}\n"),
        ("src/a_tests.rs", "fn t() {}\n"),
        ("src/b/mod.rs", "mod d;\n"),
        ("src/b/d.rs", ""),
        ("src/main_tests.rs", ""),
        ("tests/acceptance.rs", "mod common;\n"),
        ("tests/common/mod.rs", ""),
    ]);
    assert_eq!(
        got,
        Ok(pairs(&[
            ("src/main.rs", "crate"),
            ("src/a.rs", "a"),
            ("src/a/c.rs", "a::c"),
            ("src/a_tests.rs", "a::tests"),
            ("src/b/mod.rs", "b"),
            ("src/b/d.rs", "b::d"),
            ("src/main_tests.rs", "tests"),
            ("tests/acceptance.rs", "integration::acceptance"),
            ("tests/common/mod.rs", "integration::common"),
        ]))
    );
    let missing = package_of(&[("Cargo.toml", CARGO), ("src/lib.rs", "mod gone;\n")]);
    assert_eq!(missing, Err("src/lib.rs: `mod gone;` has no file: neither src/gone.rs nor src/gone/mod.rs exists.".to_string()));
}

#[test]
fn a_new_modules_file_and_name_follow_its_parent() {
    assert_eq!(child_file("src/lib.rs", true, "cache"), "src/cache.rs");
    assert_eq!(child_file("src/board/agents.rs", false, "cache"), "src/board/agents/cache.rs");
    assert_eq!(child_file("src/board/agents/mod.rs", true, "cache"), "src/board/agents/cache.rs");
    assert_eq!(child_file("src/a_tests.rs", true, "cache"), "src/cache.rs");
    assert_eq!(child_file("tests/x.rs", true, "helper"), "tests/helper.rs");
    assert!(owns_dir("crate", "src/lib.rs"));
    assert!(owns_dir("b", "src/b/mod.rs"));
    assert!(owns_dir("a::tests", "src/a_tests.rs"));
    assert!(owns_dir("integration::x", "tests/x.rs"));
    assert!(!owns_dir("a", "src/a.rs"));
    assert!(!owns_dir("tests", "src/tests.rs"));
    assert_eq!(child_module("crate", "src/lib.rs", true, "cache"), "cache");
    assert_eq!(child_module("board", "src/board.rs", false, "agents"), "board::agents");
    assert_eq!(child_module("integration::x", "tests/x.rs", true, "y"), "integration::y");
}

#[test]
fn a_workspace_and_a_proc_macro_crate_are_refused() {
    let workspace = package_of(&[("Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\n")]);
    assert_eq!(workspace, Err("This is a workspace of 2 crates; add each member as its own project.".to_string()));
    let with_members = package_of(&[("Cargo.toml", &format!("{CARGO}[workspace]\nmembers = [\".\", \"tool\"]\n")), ("src/main.rs", "")]);
    assert_eq!(with_members, Err("This is a workspace of 2 crates; add each member as its own project.".to_string()));
    let proc_macro = package_of(&[("Cargo.toml", &format!("{CARGO}[lib]\nproc-macro = true\n")), ("src/lib.rs", "")]);
    assert_eq!(proc_macro, Err("`app` is a proc-macro crate; Solace doesn't hold proc-macro crates yet.".to_string()));
    let alone = package_of(&[("Cargo.toml", &format!("{CARGO}[workspace]\n")), ("src/main.rs", "fn main() {}\n")]);
    assert_eq!(alone, Ok(pairs(&[("src/main.rs", "crate")])));
    let globbed = package_of(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/old\"]\n"),
        ("crates/a/Cargo.toml", CARGO),
        ("crates/b/Cargo.toml", CARGO),
        ("crates/old/Cargo.toml", CARGO),
        ("crates/notes/x.txt", ""),
    ]);
    assert_eq!(globbed, Err("This is a workspace of 2 crates; add each member as its own project.".to_string()));
    let no_package = package_of(&[("Cargo.toml", "[workspace]\n")]);
    assert_eq!(no_package, Err("Cargo.toml has no `[package]`; Solace holds a package of one crate.".to_string()));
    let underscore = package_of(&[("Cargo.toml", &format!("{CARGO}[lib]\nproc_macro = true\n")), ("src/lib.rs", "")]);
    assert_eq!(underscore, Err("`app` is a proc-macro crate; Solace doesn't hold proc-macro crates yet.".to_string()));
}

#[test]
fn targets_moved_from_cargos_default_places_are_refused() {
    let manifest = format!(
        "[package]\nname = \"app\"\nautotests = false\n\n[lib]\npath = \"lib/app.rs\"\n\n[[bin]]\nname = \"app\"\npath = \"src/main.rs\"\n\n[[bin]]\nname = \"tool\"\npath = \"tools/tool.rs\"\n\n[[test]]\nname = \"it\"\npath = \"tests/it.rs\"\n"
    );
    let moved = package_of(&[("Cargo.toml", &manifest), ("src/main.rs", "")]);
    assert_eq!(
        moved,
        Err("Cargo.toml sets `[lib] path = \"lib/app.rs\"`, `[[bin]] path = \"tools/tool.rs\"`, `autotests = false`; Solace reads a package's files from Cargo's default places for now.".to_string())
    );
    let defaults = package_of(&[("Cargo.toml", &format!("{CARGO}\n[lib]\npath = \"src/lib.rs\"\n")), ("src/lib.rs", "")]);
    assert_eq!(defaults, Ok(pairs(&[("src/lib.rs", "crate")])));
}

#[test]
fn more_than_one_crate_is_refused_naming_its_roots() {
    let both = package_of(&[("Cargo.toml", CARGO), ("src/lib.rs", ""), ("src/main.rs", ""), ("src/bin/tool.rs", "")]);
    assert_eq!(both, Err("This package has 3 crates (src/lib.rs, src/main.rs, src/bin/tool.rs); Solace holds a package of one crate.".to_string()));
    let none = package_of(&[("Cargo.toml", CARGO)]);
    assert_eq!(none, Err("This package has no crate: no `src/main.rs`, `src/lib.rs` or program in `src/bin/`.".to_string()));
    let program = package_of(&[("Cargo.toml", CARGO), ("src/bin/tool/main.rs", "mod args;\n"), ("src/bin/tool/args.rs", "")]);
    assert_eq!(program, Ok(pairs(&[("src/bin/tool/main.rs", "crate"), ("src/bin/tool/args.rs", "args")])));
}

#[test]
fn modules_solace_cant_hold_yet_fail_the_package_naming_their_files() {
    // A library module named `integration` still shares its prefix with an integration test file.
    let same_name = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod integration;\n"),
        ("src/integration.rs", "mod x;\n"),
        ("src/integration/x.rs", ""),
        ("tests/x.rs", ""),
    ]);
    assert_eq!(same_name, Err("Two modules are named `integration::x` (src/integration/x.rs, tests/x.rs); Solace holds one module of each name for now.".to_string()));
    let twice = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "mod x;\n#[path = \"x.rs\"]\nmod y;\n"), ("src/x.rs", "")]);
    assert_eq!(twice, Err("src/x.rs is two modules (`x`, `y`); Solace holds a file as one module for now.".to_string()));
    let cycle = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "#[path = \"main.rs\"]\nmod again;\n")]);
    assert_eq!(cycle, Err("src/main.rs is two modules (`crate`, `again`); Solace holds a file as one module for now.".to_string()));
    let outside = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "#[path = \"../x.rs\"]\nmod x;\n"), ("x.rs", "")]).unwrap_err();
    assert!(outside.starts_with("src/main.rs: line 1 (`#[path = \"../x.rs\"]`): Not saved: `#[path = \"../x.rs\"]` is outside"), "{outside}");
    let inline = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "mod a {\n    mod b;\n}\n")]);
    assert_eq!(inline, Err("src/main.rs: `a::b` is a `mod x;` inside an inline `mod { … }`; Solace doesn't hold one there yet.".to_string()));
    let refused = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "fn main() {}\n\nlazy_static! {\n    static ref X: u8 = 1;\n}\n")]).unwrap_err();
    assert!(refused.starts_with("src/main.rs: line 3 (`lazy_static! {`): Not saved: `lazy_static! { … }`"), "{refused}");
    let every = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/main.rs", "mod one;\nmod two;\n"),
        ("src/one.rs", "lazy_static! {\n    static ref A: u8 = 1;\n}\n"),
        ("src/two.rs", "lazy_static! {\n    static ref B: u8 = 1;\n}\n"),
    ])
    .unwrap_err();
    assert!(every.contains("src/one.rs: line 1") && every.contains("src/two.rs: line 1"), "{every}");
    let shared = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", ""),
        ("tests/a.rs", "mod common;\n"),
        ("tests/b.rs", "mod common;\n"),
        ("tests/common/mod.rs", "lazy_static! {\n    static ref X: u8 = 1;\n}\n"),
    ])
    .unwrap_err();
    assert_eq!(shared.matches("tests/common/mod.rs: line 1").count(), 1, "{shared}");
    let syntax = package_of(&[("Cargo.toml", CARGO), ("src/main.rs", "fn b( {}\n")]).unwrap_err();
    assert!(syntax.starts_with("src/main.rs: line 1: "), "{syntax}");
}

#[test]
fn a_helper_shared_by_test_crates_is_held_once() {
    let got = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", ""),
        ("tests/a.rs", "mod common;\n"),
        ("tests/b.rs", "mod common;\n"),
        ("tests/common/mod.rs", "pub fn setup() {}\n"),
    ]);
    assert_eq!(got, Ok(pairs(&[("src/lib.rs", "crate"), ("tests/a.rs", "integration::a"), ("tests/common/mod.rs", "integration::common"), ("tests/b.rs", "integration::b")])));
}

#[test]
fn a_library_module_and_a_test_helper_may_share_a_name() {
    let got = package_of(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod common;\n"),
        ("src/common.rs", ""),
        ("tests/a.rs", "mod common;\n"),
        ("tests/common/mod.rs", ""),
    ]);
    assert_eq!(got, Ok(pairs(&[("src/lib.rs", "crate"), ("src/common.rs", "common"), ("tests/a.rs", "integration::a"), ("tests/common/mod.rs", "integration::common")])));
}

/// A module-level import is found whether or not it is re-exported; a `mod x;`
/// and an item's own imports are not (D-168).
#[test]
fn a_header_holds_its_imports() {
    assert!(imports("use std::fmt;\n\n"));
    assert!(imports("pub use std::fmt::Debug;\n"));
    assert!(imports("pub(crate) use fmt;\n"));
    assert!(!imports("//! doc\npub mod x;\n"));
    assert!(!imports("pub fn one() {\n    use std::fmt;\n}\n"));
    assert!(!imports(""));
}

