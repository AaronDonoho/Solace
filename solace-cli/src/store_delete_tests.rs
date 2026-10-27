//! `delete` in a Rust store (D-173): named items, the items they hold and the
//! modules they emptied, checked with cargo.

use super::*;
use crate::testing::*;

/// The file's text, as the store wrote it.
fn text_of(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

/// An unused item goes, and the file loses only its text.
#[test]
fn deleting_an_unused_item_removes_it() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.two"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.two."), "{out}");
    assert!(store.resolve(&store.index().unwrap(), "crate.two").is_err());
    assert_eq!(text_of(&root, "src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n");
}

/// A name cargo still finds a use for is refused, naming the user, and the
/// store and the file keep their bytes.
#[test]
fn a_used_item_is_refused_and_nothing_changes() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    one()\n}\n";
    let (root, store) = rust_project(lib);
    let before = text_of(&root, "src/lib.rs");
    let err = store.delete_all(&["crate.one"], usize::MAX).unwrap_err().0;
    assert!(err.starts_with("E5001 crate.one is still used by:"), "{err}");
    assert!(err.contains("crate.two"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), before);
    assert!(store.resolve(&store.index().unwrap(), "crate.one").is_ok());
}

/// A type's inherent `impl` shares its name, so deleting the type is refused
/// rather than leaving the `impl` behind.
#[test]
fn a_type_its_impl_still_uses_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let before = text_of(&root, "src/lib.rs");
    let err = store.delete_all(&["crate.Store"], usize::MAX).unwrap_err().0;
    assert!(err.starts_with("E5001 crate.Store is still used by:"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), before);
    assert!(store.resolve(&store.index().unwrap(), "crate.Store").is_ok());
}

/// Deleting a module's last item takes the module, its file and its `mod` line.
#[test]
fn the_last_item_takes_the_module_with_it() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(None, Some("child"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("created module child"), "{out}");
    assert!(root.join("src/child.rs").exists());
    assert!(text_of(&root, "src/lib.rs").contains("mod child;"));
    let out = store.delete_all(&["child.two"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted child.two; module child retired."), "{out}");
    assert!(!root.join("src/child.rs").exists());
    assert_eq!(text_of(&root, "src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n");
    assert!(store.index().unwrap().modules.iter().all(|m| m.path != "child"));
}

/// A container takes the items it holds with it.
#[test]
fn a_container_takes_its_items() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n\nmod inner {\n    pub fn two() -> i32 {\n        2\n    }\n\n    pub fn three() -> i32 {\n        3\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.inner"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.inner."), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert_eq!(text, "pub fn one() -> i32 {\n    1\n}\n", "{text}");
    assert!(store.resolve(&store.index().unwrap(), "crate.inner::two").is_err());
}

/// A method's delete leaves the rest of its `impl` alone.
#[test]
fn deleting_a_method_keeps_its_impl() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n\n    pub fn close(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.Store::open"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.Store::open."), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert_eq!(text, "pub struct Store;\n\nimpl Store {\n    pub fn close(&self) -> i32 {\n        2\n    }\n}\n", "{text}");
    assert!(store.resolve(&store.index().unwrap(), "crate.Store::open").is_err());
}

/// With several names cargo can't say which one an error comes from, so they
/// are refused together, each user listed once.
#[test]
fn several_names_are_refused_together() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn a() -> i32 {\n    1\n}\n\npub fn b() -> i32 {\n    2\n}\n\npub fn c() -> i32 {\n    a() + b()\n}\n";
    let (root, store) = rust_project(lib);
    let before = text_of(&root, "src/lib.rs");
    let err = store.delete_all(&["crate.a", "crate.b"], usize::MAX).unwrap_err().0;
    assert!(err.starts_with("E5001 nothing deleted.\ncrate.a, crate.b are still used by:\n  crate.c"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), before);
    assert!(store.resolve(&store.index().unwrap(), "crate.a").is_ok());
}

/// A test crate's file is Cargo's to find, so emptying it takes its row and
/// file, and leaves the module its name resolves to by suffix alone.
#[test]
fn emptying_a_test_crate_leaves_other_modules_alone() {
    if !cargo::installed() {
        return;
    }
    let p = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub mod integration;\n"),
        ("src/a/integration.rs", "mod acceptance;\n"),
        ("src/a/integration/acceptance.rs", "pub fn other() -> i32 {\n    1\n}\n"),
        ("tests/acceptance.rs", "pub fn t() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&p).unwrap();
    let store = Store::at(&p).unwrap();
    let out = store.delete_all(&["integration::acceptance.t"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted integration::acceptance.t; module integration::acceptance retired."), "{out}");
    assert!(!p.join("tests/acceptance.rs").exists());
    assert!(text_of(&p, "src/a/integration.rs").contains("mod acceptance;"), "an unrelated module keeps its declaration");
    assert!(store.index().unwrap().modules.iter().any(|m| m.path == "a::integration::acceptance"));
}

/// A helper in a test crate is declared in its parent, so emptying it removes
/// the declaration rather than leaving cargo a missing file.
#[test]
fn emptying_a_test_helper_removes_its_declaration() {
    if !cargo::installed() {
        return;
    }
    let p = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("tests/x.rs", "mod h;\n\npub fn t() -> i32 {\n    2\n}\n"),
        ("tests/h.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&p).unwrap();
    let store = Store::at(&p).unwrap();
    let out = store.delete_all(&["integration::h.helper"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted integration::h.helper; module integration::h retired."), "{out}");
    assert!(!p.join("tests/h.rs").exists());
    assert_eq!(text_of(&p, "tests/x.rs"), "pub fn t() -> i32 {\n    2\n}\n");
}

/// A `#[path]` helper's declaration names its file, not its module's default.
#[test]
fn emptying_a_path_named_helper_removes_its_declaration() {
    if !cargo::installed() {
        return;
    }
    let p = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "#[path = \"other.rs\"]\nmod h;\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/other.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&p).unwrap();
    let store = Store::at(&p).unwrap();
    let out = store.delete_all(&["h.helper"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted h.helper; module h retired."), "{out}");
    assert!(!p.join("src/other.rs").exists());
    assert_eq!(text_of(&p, "src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n");
}

/// A helper declared by several test crates loses its declaration in each.
#[test]
fn emptying_a_shared_helper_removes_every_declaration() {
    if !cargo::installed() {
        return;
    }
    let p = Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("tests/a.rs", "mod common;\n\npub fn a() -> i32 {\n    1\n}\n"),
        ("tests/b.rs", "mod common;\n\npub fn b() -> i32 {\n    2\n}\n"),
        ("tests/common/mod.rs", "pub fn setup() -> i32 {\n    3\n}\n"),
    ]);
    crate::import(&p).unwrap();
    let store = Store::at(&p).unwrap();
    let out = store.delete_all(&["integration::common.setup"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted integration::common.setup; module integration::common retired."), "{out}");
    assert!(!p.join("tests/common/mod.rs").exists());
    assert_eq!(text_of(&p, "tests/a.rs"), "pub fn a() -> i32 {\n    1\n}\n");
    assert_eq!(text_of(&p, "tests/b.rs"), "pub fn b() -> i32 {\n    2\n}\n");
}

/// Deleting an impl's last member takes the impl with it (D-187).
#[test]
fn deleting_the_last_method_drops_its_impl() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.Store::open"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.Store::open."), "{out}");
    assert!(out.contains("Dropped empty impl crate.Store."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "pub struct Store;\n");
    let ix = store.index().unwrap();
    assert!(!ix.syms.iter().any(|s| s.kind_text == "impl"), "{}", text_of(&root, "src/lib.rs"));
    assert!(store.resolve(&ix, "crate.Store::open").is_err());
}

/// Deleting an inline module's last test leaves no blank line before its `}`.
#[test]
fn deleting_the_last_test_of_an_inline_module_keeps_its_brace() {
    if !cargo::installed() {
        return;
    }
    let lib = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn first() {}\n\n    #[test]\n    fn last() {}\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.tests::last"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.tests::last."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "#[cfg(test)]\nmod tests {\n    #[test]\n    fn first() {}\n}\n");
}

/// Deleting an `impl`'s last method leaves no blank line before its `}`.
#[test]
fn deleting_an_impls_last_method_keeps_its_brace() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n\n",
        "    pub fn close(&self) -> i32 {\n        2\n    }\n}\n",
    );
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.Store::close"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.Store::close."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n");
}

/// Deleting a trait's last method leaves no blank line before its `}`.
#[test]
fn deleting_a_traits_last_item_keeps_its_brace() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub trait Greet {\n    fn hello(&self);\n\n    fn bye(&self);\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.Greet::bye"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.Greet::bye."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "pub trait Greet {\n    fn hello(&self);\n}\n");
}

/// Deleting several children, including the last, leaves no blank line
/// before the inline module's `}`.
#[test]
fn deleting_several_children_including_the_last_keeps_the_brace() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {}\n\n",
        "    #[test]\n    fn b() {}\n\n    #[test]\n    fn c() {}\n}\n",
    );
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.tests::b", "crate.tests::c"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.tests::b."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {}\n}\n");
}

/// A trait impl goes as a whole, and a defaulted trait method can go alone.
#[test]
fn deleting_a_trait_impl_or_its_method_removes_it() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub struct Failure;\n\n",
        "impl From<std::io::Error> for Failure {\n",
        "    fn from(_: std::io::Error) -> Self {\n        Self\n    }\n}\n",
    );
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.<Failure as From<std::io::Error>>"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.<Failure as From<std::io::Error>>."), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert!(!text.contains("impl From"), "{text}");

    let lib = concat!(
        "pub trait Named {\n    fn name(&self) -> u8 {\n        0\n    }\n}\n\n",
        "pub struct S;\n\n",
        "impl Named for S {\n    fn name(&self) -> u8 {\n        1\n    }\n}\n",
    );
    let (root, store) = rust_project(lib);
    let out = store.delete_all(&["crate.<S as Named>::name"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted crate.<S as Named>::name."), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert_eq!(text.matches("fn name").count(), 1, "{text}");
}
