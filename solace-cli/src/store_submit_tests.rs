//! `submit` in a Rust store (D-168): items saved by name, checked with cargo.

use super::*;
use crate::testing::*;

use serde_json::Value;
fn text_of(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

/// The text of the one kept draft.
fn kept(store: &Store) -> String {
    store.db.query_row("SELECT text FROM drafts", [], |r| r.get(0)).unwrap()
}

/// Editing a function and a method leaves every other byte of the file alone.
#[test]
fn an_edit_keeps_the_rest_of_the_file() {
    if !cargo::installed() {
        return;
    }
    let lib =
        "pub fn one() -> i32 {\n    1\n}\n\npub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(Some("crate.one"), None, "pub fn one() -> i32 {\n    3\n}\n").unwrap();
    assert!(out.contains("Saved crate.one"), "{out}");
    let edited = lib.replace("    1\n", "    3\n");
    assert_eq!(text_of(&root, "src/lib.rs"), edited);
    let out = store.submit_symbols(Some("crate.Store::open"), None, "pub fn open(&self) -> i32 {\n    4\n}\n").unwrap();
    assert!(out.contains("Saved crate.Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), edited.replace("        2\n", "        4\n"));
}

/// `--new` adds a top-level item and a method, with the file's blank lines.
#[test]
fn new_adds_items_with_the_right_spacing() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) {}\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(None, Some("crate"), "pub fn extra() -> i32 {\n    5\n}\n").unwrap();
    assert!(out.contains("Saved crate.extra"), "{out}");
    let added = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) {}\n}\n\npub fn extra() -> i32 {\n    5\n}\n";
    assert_eq!(text_of(&root, "src/lib.rs"), added);
    let out = store.submit_symbols(None, Some("crate.Store"), "pub fn close(&self) {}\n").unwrap();
    assert!(out.contains("Saved crate.Store::close"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        r#"pub struct Store;

impl Store {
    pub fn open(&self) {}

    pub fn close(&self) {}
}

pub fn extra() -> i32 {
    5
}
"#
    );
}

/// A refusal from the Rust splitter replies "Not saved" and keeps the draft.
#[test]
fn a_split_refusal_keeps_the_draft() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "include!(\"table.rs\");\n";
    let err = drafts::submit(&store, &["submit", "crate.one"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved") && err.contains("include!"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A stored item the whole module leaves out, a `mod` line in the text, and a
/// named block each say where the work goes.
#[test]
fn whole_modules_and_named_blocks_say_where_the_work_goes() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {}\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    1\n}\n";
    let err = drafts::submit(&store, &["submit", "crate"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("E0102 the module text leaves out: Store"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    let err = store.submit_symbols(None, Some("crate"), "mod extra;\n").unwrap_err().0;
    assert!(err.contains("`mod extra;` names another module") && err.contains("submit --new <module>"), "{err}");
    let err = store.submit_symbols(None, Some("crate"), "mod tests {\n    mod inner;\n}\n").unwrap_err().0;
    assert!(err.contains("names another module") && err.contains("submit --new <module>"), "{err}");
    let err = store.submit_symbols(Some("crate.Store"), None, "impl Store {\n    pub fn open(&self) {}\n}\n").unwrap_err().0;
    assert!(err.contains("submit --new <module>") && !err.contains("task:"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(store.index().unwrap().modules.len(), 1, "nothing new was stored");
}

/// A type error replies "Not saved" with the round note, and keeps the draft.
#[test]
fn a_type_error_keeps_the_draft() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    \"two\"\n}\n";
    let err = drafts::submit(&store, &["submit", "crate.one"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved: the submitted text has errors."), "{err}");
    assert!(err.contains("Rust checks borrows only once types check"), "{err}");
    assert!(
        !err.contains("Nothing was saved.")
            && err.contains("Fix the failing block")
            && !err.contains("other blocks")
            && err.contains("--replace"),
        "{err}"
    );
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// `--imports` takes a header of `use` lines, and refuses more than that.
#[test]
fn imports_replace_the_header_only() {
    if !cargo::installed() {
        return;
    }
    let lib = "use std::fmt;\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit(crate::store::Target::Imports("crate"), "use std::fmt;\nuse std::io;\n").unwrap();
    assert!(out.contains("Saved crate imports"), "{out}");
    assert_eq!(store.header("crate").unwrap(), "use std::fmt;\nuse std::io;\n");
    let header = "use std::fmt;\nuse std::io;\npub fn one() -> i32 {\n    1\n}\n";
    assert_eq!(text_of(&root, "src/lib.rs"), header);
    let err = store.submit(crate::store::Target::Imports("crate"), "use std::fmt;\n\npub fn extra() -> i32 {\n    1\n}\n").unwrap_err().0;
    assert!(err.contains("--imports text may only contain imports"), "{err}");
    let err = store.submit(crate::store::Target::Imports("crate"), "use std::fmt;\nmod extra;\n").unwrap_err().0;
    assert!(
        err.contains("must leave out the module's `mod` lines or keep the same declarations, in any order")
            && err.contains("view crate --header"),
        "{err}"
    );
    assert_eq!(text_of(&root, "src/lib.rs"), header);
}

/// A block named `<T as Tr>::f` settles on the trait impl's method.
#[test]
fn a_block_named_with_a_trait_impl_submits() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"pub struct Thing;

impl std::fmt::Display for Thing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "thing")
    }
}
"#;
    let (root, store) = rust_project(lib);
    let block = r#"// ── crate.<Thing as std::fmt::Display>::fmt ──
fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "other")
}
"#;
    let out = store.submit_symbols(None, None, block).unwrap();
    assert!(out.contains("Saved crate.<Thing as std::fmt::Display>::fmt"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("\"thing\"", "\"other\""));
}

/// A patch updates a method and adds one, leaving the rest of the file alone.
#[test]
fn a_patch_updates_and_adds_a_method() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let update = r#"*** Begin Patch
*** Update: crate.Store::open
@@
-    pub fn open(&self) -> i32 {
-        2
-    }
+    pub fn open(&self) -> i32 {
+        3
+    }
*** End Patch
"#;
    let out = drafts::submit(&store, &["submit"], &mut || Ok(update.into()), None).unwrap();
    assert!(out.contains("Saved crate.Store::open"), "{out}");
    let edited = lib.replace("        2\n", "        3\n");
    assert_eq!(text_of(&root, "src/lib.rs"), edited);
    let add = "*** Begin Patch\n*** Add: crate.Store\n+pub fn close(&self) {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(add.into()), None).unwrap();
    assert!(out.contains("Saved crate.Store::close"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), edited.replace("    }\n}\n", "    }\n\n    pub fn close(&self) {}\n}\n"));
}

/// Undoing a Rust submit restores the file byte for byte.
#[test]
fn undo_restores_the_file() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let before = board::snapshot(&store).unwrap();
    store.submit_symbols(Some("crate.Store::open"), None, "pub fn open(&self) -> i32 {\n    7\n}\n").unwrap();
    assert_ne!(text_of(&root, "src/lib.rs"), lib);
    let states: Vec<(String, Value)> = before.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    board::restore(&store, &states).unwrap();
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// Editing a method of an `impl` inside an inline `mod` keeps every other byte.
#[test]
fn an_edit_inside_an_inline_mod_keeps_the_file() {
    if !cargo::installed() {
        return;
    }
    let lib =
        "mod tests {\n    pub struct Store;\n\n    impl Store {\n        pub fn open(&self) -> i32 {\n            2\n        }\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(Some("crate.tests::Store::open"), None, "pub fn open(&self) -> i32 {\n    3\n}\n").unwrap();
    assert!(out.contains("Saved crate.tests::Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("            2\n", "            3\n"));
}

/// An `extern "C"` block's item is saved under the parent's prefix, not the
/// block's name (D-162).
#[test]
fn an_extern_item_is_saved_under_the_parents_prefix() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod tests {\n    extern \"C\" {\n        pub fn getpid() -> i32;\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(Some("crate.tests::getpid"), None, "pub fn getpid() -> u32;\n").unwrap();
    assert!(out.contains("Saved crate.tests::getpid"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("-> i32", "-> u32"));
}

/// A method's multi-line string literal keeps its lines, even nested.
#[test]
fn a_nested_method_keeps_a_multi_line_string_literal() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"mod tests {
    pub struct Store;

    impl Store {
        pub fn open(&self) -> &str {
            "one
two"
        }
    }
}
"#;
    let (root, store) = rust_project(lib);
    let text = "pub fn open(&self) -> &str {\n    \"one\nthree\"\n}\n";
    let out = store.submit_symbols(Some("crate.tests::Store::open"), None, text).unwrap();
    assert!(out.contains("Saved crate.tests::Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("two", "three"));
}

/// A block whose text carries imports is refused, naming `--imports`.
#[test]
fn a_block_with_imports_names_the_imports_command() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "use std::fmt;\n\npub fn one() -> i32 {\n    2\n}\n";
    let err = drafts::submit(&store, &["submit", "crate.one"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("block crate.one") && err.contains("`use std::fmt;`") && err.contains("submit --imports"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// Removing a `use` an item needs is refused, and the file is unchanged.
#[test]
fn imports_that_break_an_item_are_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "use std::collections::HashMap;\n\npub fn one() -> HashMap<u8, u8> {\n    HashMap::new()\n}\n";
    let (root, store) = rust_project(lib);
    let err = store.submit(crate::store::Target::Imports("crate"), "").unwrap_err().0;
    assert!(err.contains("Not saved: the submitted text has errors."), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// An `*** Update:` that names only a container is refused, as `submit` is;
/// an `*** Add:` of a new impl block for a stored type saves.
#[test]
fn a_patch_naming_only_a_container_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"pub struct Other;

impl std::fmt::Display for Other {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "other")
    }
}
"#;
    let (root, store) = rust_project(lib);
    let patch =
        "*** Begin Patch\n*** Update: crate.<Other as std::fmt::Display>\n@@\n-    fn fmt(&self) {\n+    fn fmt(&self) {\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(err.contains("format crate") && err.contains("item by item") && !err.contains("submit --new <module>"), "{err}");
    assert!(!err.contains("task:"), "{err}");
    let add = "*** Begin Patch\n*** Add: crate\n+impl Other {\n+    fn o(&self) {}\n+}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(add.into()), None).unwrap();
    assert!(out.contains("Saved crate.Other"), "{out}");
    assert!(text_of(&root, "src/lib.rs").contains("impl Other {\n    fn o(&self) {}\n}"));
}

/// A `pub use` re-export in a block's header is refused, as a plain `use` is.
#[test]
fn a_block_with_a_pub_use_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub use std::fmt;\n\npub fn one() -> i32 {\n    2\n}\n";
    let err = drafts::submit(&store, &["submit", "crate.one"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("block crate.one") && err.contains("`pub use std::fmt;`") && err.contains("submit --imports"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A patch on a nested method, whose stored text is already indented, rewraps
/// exactly: every other byte of the file stays (D-168).
#[test]
fn a_patch_updates_a_nested_method() {
    if !cargo::installed() {
        return;
    }
    let lib =
        "mod tests {\n    pub struct Store;\n\n    impl Store {\n        pub fn open(&self) -> i32 {\n            2\n        }\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let patch = r#"*** Begin Patch
*** Update: crate.tests::Store::open
@@
-        pub fn open(&self) -> i32 {
-            2
-        }
+        pub fn open(&self) -> i32 {
+            3
+        }
*** End Patch
"#;
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.tests::Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("            2\n", "            3\n"));
}

/// A nested method's multi-line C-string literal keeps its lines too.
#[test]
fn a_nested_method_keeps_a_multi_line_c_string() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"mod tests {
    pub struct Store;

    impl Store {
        pub fn lines(&self) -> &'static core::ffi::CStr {
            c"one
two"
        }
    }
}
"#;
    let (root, store) = rust_project(lib);
    let text = "pub fn lines(&self) -> &'static core::ffi::CStr {\n    c\"one\nthree\"\n}\n";
    let out = store.submit_symbols(Some("crate.tests::Store::lines"), None, text).unwrap();
    assert!(out.contains("Saved crate.tests::Store::lines"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("two", "three"));
}

/// `--new` saves a whole `impl` block with its items (D-168).
#[test]
fn new_saves_a_whole_impl_block() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub struct X;\n");
    let block = r#"impl std::fmt::Display for X {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "x")
    }
}
"#;
    let out = store.submit_symbols(None, Some("crate"), block).unwrap();
    assert!(out.contains("Saved crate.<X as std::fmt::Display>"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), format!("pub struct X;\n\n{block}"));
}

/// A type with no `impl` takes its first method as a new block, and the next
/// one goes item by item (D-168).
#[test]
fn new_saves_a_types_first_method() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub struct Store;\n");
    let block = "impl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n";
    let out = store.submit_symbols(None, Some("crate"), block).unwrap();
    assert!(out.contains("Saved crate.Store"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), format!("pub struct Store;\n\n{block}"));
    let out = store.submit_symbols(None, Some("crate.Store"), "pub fn close(&self) {}\n").unwrap();
    assert!(out.contains("Saved crate.Store::close"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n\n    pub fn close(&self) {}\n}\n"
    );
}

/// An item added to an empty `impl X {}` joins it where its `}` was (D-168).
#[test]
fn adding_to_an_empty_impl_fills_it() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub struct Store;\n\nimpl Store {}\n");
    let out = store.submit_symbols(None, Some("crate.Store"), "pub fn open(&self) {}\n").unwrap();
    assert!(out.contains("crate.Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) {}\n}\n");
    let out = store.submit_symbols(None, Some("crate.Store"), "pub fn close(&self) {}\n").unwrap();
    assert!(out.contains("crate.Store::close"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) {}\n\n    pub fn close(&self) {}\n}\n"
    );
}

/// A whole inline `mod` block and a whole `extern` block save with their items.
#[test]
fn new_saves_an_inline_mod_and_an_extern_block() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let inline = "mod tests {\n    pub fn t() -> i32 {\n        2\n    }\n}\n";
    let out = store.submit_symbols(None, Some("crate"), inline).unwrap();
    assert!(out.contains("Saved crate.tests"), "{out}");
    let extern_block = "extern \"C\" {\n    pub fn getpid() -> i32;\n}\n";
    let out = store.submit_symbols(None, Some("crate"), extern_block).unwrap();
    assert!(out.contains("Saved crate.extern \"C\""), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), format!("pub fn one() -> i32 {{\n    1\n}}\n\n{inline}\n{extern_block}"));
}

/// The first item of an empty `extern "C" {}` lands inside it (D-168).
#[test]
fn an_extern_block_takes_its_first_item() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("extern \"C\" {}\n");
    let out = store.submit_symbols(None, Some("crate.extern \"C\""), "pub fn getpid() -> i32;\n").unwrap();
    assert!(out.contains("crate.getpid"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "extern \"C\" {\n    pub fn getpid() -> i32;\n}\n");
}

/// A whole block nests in an existing inline `mod`; naming the mod refuses it
/// (D-168).
#[test]
fn a_whole_block_nests_in_an_inline_mod() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod tests {\n    pub struct Store;\n\n    pub fn t() -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let block = "impl Store {\n    pub fn open(&self) {}\n}\n";
    let err = store.submit_symbols(Some("crate.tests"), None, block).unwrap_err().0;
    assert!(err.contains("submit --new <module>") && !err.contains("task:"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib, "a named mod's block saved nothing");
    let out = store.submit_symbols(None, Some("crate.tests"), block).unwrap();
    assert!(out.contains("crate.tests::Store::open"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        r#"mod tests {
    pub struct Store;

    pub fn t() -> i32 {
        2
    }

    impl Store {
        pub fn open(&self) {}
    }
}
"#
    );
}

/// Every row a new block or a filled container adds is recorded as created
/// (D-168).
#[test]
fn a_new_blocks_rows_are_recorded_as_created() {
    if !cargo::installed() {
        return;
    }
    let created = |store: &Store, detail: &str| -> i64 {
        store.db.query_row("SELECT COUNT(*) FROM history WHERE action = 'create' AND detail = ?1", [detail], |r| r.get(0)).unwrap()
    };
    let (_root, store) = rust_project("pub struct Store;\n");
    store.submit_symbols(None, Some("crate"), "impl Store {\n    pub fn open(&self) {}\n}\n").unwrap();
    assert_eq!((created(&store, "Store"), created(&store, "Store::open")), (1, 1));
    let (_root, store) = rust_project("pub struct Store;\n\nimpl Store {}\n");
    store.submit_symbols(None, Some("crate.Store"), "pub fn open(&self) {}\n").unwrap();
    assert_eq!((created(&store, "Store"), created(&store, "Store::open")), (0, 1), "the container is edited, its item created");
}

/// A module the store doesn't have is created by `--new`: its file beside the
/// crate root, the `mod` line its parent's header takes, and a clean check
/// (D-168).
#[test]
fn new_creates_a_module_with_its_file_and_mod_line() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(None, Some("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("Saved brand.side"), "{out}");
    assert!(out.contains("created module brand (src/brand.rs; added mod brand; to crate)"), "{out}");
    assert_eq!(text_of(&root, "src/brand.rs"), "pub fn side() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "src/lib.rs"), format!("mod brand;\n\n{lib}"));
    assert_eq!(store.index().unwrap().modules.len(), 2);
}

/// A new module's text carries its own header, docs and `use` lines included,
/// and its `mod` line goes after the parent's last one (D-168).
#[test]
fn new_takes_the_header_of_a_new_module() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    store.submit_symbols(None, Some("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap();
    let text = "//! Words.\n\nuse std::collections::HashMap;\n\npub fn show() -> usize {\n    HashMap::<u8, u8>::new().len()\n}\n";
    let out = store.submit_symbols(None, Some("words"), text).unwrap();
    assert!(out.contains("created module words (src/words.rs; added mod words; to crate)"), "{out}");
    assert!(text_of(&root, "src/words.rs").starts_with("//! Words.\n\nuse std::collections::HashMap;\n\npub fn show()"), "{out}");
    assert!(text_of(&root, "src/lib.rs").starts_with("mod brand;\nmod words;\n\n"), "{out}");
    assert_eq!(store.index().unwrap().modules.len(), 3);
}

/// A new module still refuses a `mod x;` in its text, and refused text leaves
/// no file behind (D-168).
#[test]
fn a_refused_new_module_leaves_no_file() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = store.submit_symbols(None, Some("other"), "mod y;\n").unwrap_err().0;
    assert!(err.contains("names another module") && err.contains("submit --new <module>"), "{err}");
    let err = store.submit_symbols(None, Some("other"), "include!(\"table.rs\");\n").unwrap_err().0;
    assert!(err.contains("Not saved") && err.contains("include!"), "{err}");
    assert!(!root.join("src/other.rs").exists(), "a refused module writes no file");
    assert_eq!(store.index().unwrap().modules.len(), 1);
}

/// A new nested module lands beside its parent's file, not in a folder named
/// after itself (D-168).
#[test]
fn new_creates_a_nested_module_beside_its_parents_file() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod board;\n"),
        ("src/board.rs", "pub mod agents;\n"),
        ("src/board/agents.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit_symbols(None, Some("board::agents::cache"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("Saved board::agents::cache.two"), "{out}");
    assert!(out.contains("created module board::agents::cache (src/board/agents/cache.rs; added mod cache; to board::agents)"), "{out}");
    assert_eq!(text_of(&root, "src/board/agents/cache.rs"), "pub fn two() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "src/board/agents.rs"), "mod cache;\n\npub fn one() -> i32 {\n    1\n}\n");
}

/// A patch's `*** Add: <module>` creates the module too, its `mod` line going
/// after the header's last one (D-168).
#[test]
fn a_patch_add_creates_a_module_after_the_headers_last_mod_line() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\n\nuse std::fmt;\n\npub fn one() -> std::fmt::Result {\n    Ok(())\n}\n"),
        ("src/a.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: brand\n+pub fn side() -> i32 {\n+    2\n+}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("created module brand (src/brand.rs; added mod brand; to crate)"), "{out}");
    assert_eq!(text_of(&root, "src/brand.rs"), "pub fn side() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "src/lib.rs"), "mod a;\nmod brand;\n\nuse std::fmt;\n\npub fn one() -> std::fmt::Result {\n    Ok(())\n}\n");
}

/// `integration::x` creates a test crate root, `tests/x.rs`, which no `mod`
/// line names (D-168, D-169).
#[test]
fn new_creates_a_test_crate_root() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(None, Some("integration::x"), "pub fn t() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("created module integration::x (tests/x.rs)"), "{out}");
    assert_eq!(text_of(&root, "tests/x.rs"), "pub fn t() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "src/lib.rs"), lib, "a test root takes no `mod` line");
    assert_eq!(store.index().unwrap().modules.len(), 2);
}

/// A new module's file already there outside the store is refused, as Gleam's
/// is (D-168).
#[test]
fn a_new_modules_existing_file_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    fs::write(root.join("src/brand.rs"), "pub fn mine() -> i32 {\n    9\n}\n").unwrap();
    let err = store.submit_symbols(None, Some("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap_err().0;
    assert!(err.contains("already exists outside the store"), "{err}");
    assert_eq!(text_of(&root, "src/brand.rs"), "pub fn mine() -> i32 {\n    9\n}\n");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(store.index().unwrap().modules.len(), 1);
}

/// A module name Rust reserves is refused and leaves no file and no `mod` line
/// (D-168): `mod type;` doesn't parse, so the crate would stop building.
#[test]
fn a_reserved_module_name_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    for name in ["type", "fn", "async", "self", "_"] {
        let err = store.submit_symbols(None, Some(name), "pub fn side() -> i32 {\n    2\n}\n").unwrap_err().0;
        assert!(err.contains("E1003") && err.contains(name), "{name}: {err}");
    }
    assert!(!root.join("src/type.rs").exists() && !root.join("src/_.rs").exists(), "a refused module writes no file");
    assert_eq!(text_of(&root, "src/lib.rs"), lib, "the parent's header is untouched");
    assert_eq!(store.index().unwrap().modules.len(), 1);
}
/// A `use` in a block submitted into an inline `mod` is refused, as a top-level
/// one is: it would land as the container's child, and imports go through
/// `submit --imports` (D-168).
#[test]
fn a_nested_block_with_imports_names_the_imports_command() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod tests {\n    pub fn one() -> i32 {\n        1\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let text = "use std::fmt;\n\npub fn two() -> i32 {\n    2\n}\n";
    let err = store.submit_symbols(None, Some("crate.tests"), text).unwrap_err().0;
    assert!(err.contains("`use std::fmt;`") && err.contains("submit --imports"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}
/// A symbol three levels deep — inside a `mod` inside a `mod` — is named and
/// saved as its parents name it (D-168).
#[test]
fn an_edit_inside_a_three_level_nesting_keeps_the_file() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod a {\n    mod b {\n        pub fn f() -> i32 {\n            1\n        }\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit_symbols(Some("crate.a::b::f"), None, "pub fn f() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("Saved crate.a::b::f"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("            1\n", "            2\n"));
}

/// An error the new `mod` line causes in its parent's header is this submit's:
/// the module is not saved (D-168).
#[test]
fn a_parent_header_error_refuses_the_new_module() {
    if !cargo::installed() {
        return;
    }
    let lib = "use std::fmt::Debug as brand;\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let err = store.submit_symbols(None, Some("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap_err().0;
    assert!(err.contains("Not saved: the submitted text has errors."), "{err}");
    assert!(!root.join("src/brand.rs").exists(), "a refused module writes no file");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(store.index().unwrap().modules.len(), 1);
}

/// The new `mod` line goes after the header's last `mod` declaration, `pub` and
/// `#[path]` lines included (D-168).
#[test]
fn a_new_mod_line_goes_after_pub_and_path_declarations() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n#[path = \"b.rs\"]\nmod b;\n\nuse std::fmt;\n\npub fn one() -> std::fmt::Result {\n    Ok(())\n}\n"),
        ("src/a.rs", ""),
        ("src/b.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit_symbols(None, Some("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("created module brand (src/brand.rs; added mod brand; to crate)"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        "pub mod a;\n#[path = \"b.rs\"]\nmod b;\nmod brand;\n\nuse std::fmt;\n\npub fn one() -> std::fmt::Result {\n    Ok(())\n}\n"
    );
}

/// A name landing on a module the store already has under a test crate's
/// prefix is refused: `integration::x::y` when `integration::y` exists (D-169).
#[test]
fn a_nested_test_module_already_stored_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    store.submit_symbols(None, Some("integration::x"), "pub fn t() -> i32 {\n    2\n}\n").unwrap();
    store.submit_symbols(None, Some("integration::y"), "pub fn u() -> i32 {\n    3\n}\n").unwrap();
    let err = store.submit_symbols(None, Some("integration::x::y"), "pub fn v() -> i32 {\n    4\n}\n").unwrap_err().0;
    assert!(err.contains("E1003") && err.contains("integration::x::y"), "{err}");
    assert_eq!(text_of(&root, "tests/x.rs"), "pub fn t() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "tests/y.rs"), "pub fn u() -> i32 {\n    3\n}\n");
    assert_eq!(store.index().unwrap().modules.len(), 3);
}

/// A `#[path]` module places its children beside its file, where the importer
/// read them: a new `foo::x` goes to `src/x.rs`, not `src/foo/x.rs` (D-168).
#[test]
fn a_path_module_places_its_child_beside_it() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "#[path = \"foo.rs\"]\nmod foo;\n"),
        ("src/foo.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit_symbols(None, Some("foo::x"), "pub fn two() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("created module foo::x (src/x.rs; added mod x; to foo)"), "{out}");
    assert_eq!(text_of(&root, "src/x.rs"), "pub fn two() -> i32 {\n    2\n}\n");
    assert_eq!(text_of(&root, "src/foo.rs"), "mod x;\n\npub fn one() -> i32 {\n    1\n}\n");
}

/// A module resubmitted as `view` shows it changes no byte and no id (D-168).
#[test]
fn a_whole_module_submitted_unchanged_keeps_every_byte_and_id() {
    if !cargo::installed() {
        return;
    }
    let lib =
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let ix = store.index().unwrap();
    let text = super::module_text(&ix, &ix.modules[0]);
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("Unchanged: crate"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(ids(&store), before);
}

/// A whole module changes one method and adds one item, and nothing else: the
/// other ids stay (D-168).
#[test]
fn a_whole_module_changes_one_method_and_adds_one() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let id = |s: &Store, name: &str| s.index().unwrap().syms.iter().find(|x| x.name == name).map(|x| x.id.clone()).unwrap();
    let open = id(&store, "Store::open");
    let ix = store.index().unwrap();
    let text = super::module_text(&ix, &ix.modules[0]);
    let edited = text.replace("        2\n", "        7\n") + "\npub fn two() -> i32 {\n    3\n}\n";
    let out = store.submit(crate::store::Target::Module("crate"), &edited).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert!(out.contains("changed Store::open") && out.contains("added two"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("        2\n", "        7\n") + "\npub fn two() -> i32 {\n    3\n}\n");
    assert_eq!(id(&store, "Store::open"), open, "the edited method keeps its id");
}

/// Two `impl Store` blocks keep their own ids: items pair in order (D-168).
#[test]
fn a_whole_module_keeps_the_ids_of_two_impl_blocks() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"pub struct Store;

impl Store {
    pub fn open(&self) -> i32 {
        1
    }
}

impl Store {
    pub fn close(&self) -> i32 {
        2
    }
}
"#;
    let (root, store) = rust_project(lib);
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let ix = store.index().unwrap();
    let text = super::module_text(&ix, &ix.modules[0]);
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("Unchanged: crate"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(ids(&store), before);
}

/// A type error in a whole module replies "Not saved" and keeps the draft (D-168).
#[test]
fn a_whole_module_type_error_keeps_the_draft() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    \"two\"\n}\n";
    let err = drafts::submit(&store, &["submit", "crate"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved: the submitted text has errors."), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A whole module can't change the file's `mod` list (D-168).
#[test]
fn a_whole_module_refuses_a_changed_mod_list() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    1\n}\n\nmod extra;\n";
    let err = drafts::submit(&store, &["submit", "crate"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved") && err.contains("submit --new <module>"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A splitter refusal in a whole module keeps the draft (D-168).
#[test]
fn a_whole_module_splitter_refusal_keeps_the_draft() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    1\n}\n\ninclude!(\"table.rs\");\n";
    let err = drafts::submit(&store, &["submit", "crate"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved") && err.contains("include!"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A patch's `*** Update: <module>` edits one item of the module, leaving the
/// rest of the file alone (D-168).
#[test]
fn a_patch_updating_a_whole_module_edits_one_item() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let patch = "*** Begin Patch\n*** Update: crate\n@@\n-        2\n+        3\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("        2\n", "        3\n"));
}

/// A module the store doesn't have is created by `submit <module>`, through the
/// `--new` path (D-168, D-171).
#[test]
fn a_whole_module_the_store_doesnt_have_is_created() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit(crate::store::Target::Module("brand"), "pub fn side() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("Saved brand.side"), "{out}");
    assert_eq!(text_of(&root, "src/brand.rs"), "pub fn side() -> i32 {\n    2\n}\n");
    assert!(text_of(&root, "src/lib.rs").starts_with("mod brand;\n\n"), "{}", text_of(&root, "src/lib.rs"));
}

/// A container whose own opening changed is formatted whole; its items keep
/// their ids and bytes (D-168).
#[test]
fn a_whole_module_formats_a_changed_container_opening() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod tests {\n    pub fn t() -> i32 {\n        1\n    }\n}\n";
    let (root, store) = rust_project(lib);
    let id = |s: &Store, name: &str| s.index().unwrap().syms.iter().find(|x| x.name == name).map(|x| x.id.clone()).unwrap();
    let t = id(&store, "tests::t");
    let text = "#[allow(dead_code)]\nmod tests {\n    pub fn t() -> i32 {\n        1\n    }\n}\n";
    let out = store.submit(crate::store::Target::Module("crate"), text).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), text);
    assert_eq!(id(&store, "tests::t"), t);
}

/// A whole module adds an `impl` block with its items in one submit (D-168).
#[test]
fn a_whole_module_adds_a_new_container_with_its_items() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n";
    let (root, store) = rust_project(lib);
    let text = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n";
    let out = store.submit(crate::store::Target::Module("crate"), text).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert!(out.contains("added Store, Store::open"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), text);
}

/// A patch on a module with imports edits one item and leaves the header alone
/// (D-168).
#[test]
fn a_patch_updating_a_module_with_imports_edits_one_item() {
    if !cargo::installed() {
        return;
    }
    let lib = "use std::fmt;\n\npub fn one() -> fmt::Result {\n    Ok(())\n}\n";
    let (root, store) = rust_project(lib);
    let patch = "*** Begin Patch\n*** Update: crate\n@@\n-    Ok(())\n+    let _ = 1;\n+    Ok(())\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.one"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "use std::fmt;\n\npub fn one() -> fmt::Result {\n    let _ = 1;\n    Ok(())\n}\n");
}

/// A patch that changes a module's imports is refused, naming `--imports`
/// (D-168).
#[test]
fn a_patch_changing_a_modules_imports_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "use std::fmt;\n\npub fn one() -> fmt::Result {\n    Ok(())\n}\n";
    let (root, store) = rust_project(lib);
    let patch = "*** Begin Patch\n*** Update: crate\n@@\n-use std::fmt;\n+use std::io;\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(
        err.contains("changes crate's header")
            && err.contains("view crate --header")
            && err.contains("submit --imports crate")
            && !err.contains("added automatically"),
        "{err}"
    );
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// A whole module whose text defines a name twice is refused without saving
/// (D-168): the splitter catches it.
#[test]
fn a_whole_module_with_a_duplicate_item_is_refused() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "pub fn one() -> i32 {\n    1\n}\n\npub fn one() -> i32 {\n    2\n}\n";
    let err = drafts::submit(&store, &["submit", "crate"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("Not saved") && err.contains("defined twice"), "{err}");
    assert_eq!(kept(&store), text);
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// An item added to an inline module works even when its opening imports
/// (`use super::*;`): only the submitted text is checked for imports (D-168).
#[test]
fn an_add_to_an_inline_mod_with_an_import_is_saved() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod tasks;\n"),
        ("src/tasks.rs", "mod tests {\n    use super::*;\n\n    fn helper() -> i32 {\n        1\n    }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: tasks::tests.ONE\n+const ONE: i32 = 1;\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved tasks.tests::ONE"), "{out}");
    assert_eq!(
        text_of(&root, "src/tasks.rs"),
        "mod tests {\n    use super::*;\n\n    fn helper() -> i32 {\n        1\n    }\n\n    const ONE: i32 = 1;\n}\n"
    );
}

/// A module prefix resolves as a module, not as a container: adding to the
/// file module `tests` lands there, not in another module's inline `mod tests`
/// (D-168).
#[test]
fn a_module_prefix_is_not_a_container() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod tests;\nmod other;\n"),
        ("src/tests.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("src/other.rs", "mod tests {\n    pub fn inner() -> i32 {\n        2\n    }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: tests.zz_probe\n+pub fn zz_probe() -> i32 {\n+    9\n+}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved tests.zz_probe"), "{out}");
    assert_eq!(text_of(&root, "src/tests.rs"), "pub fn one() -> i32 {\n    1\n}\n\npub fn zz_probe() -> i32 {\n    9\n}\n");
    assert_eq!(text_of(&root, "src/other.rs"), "mod tests {\n    pub fn inner() -> i32 {\n        2\n    }\n}\n");
}

/// A block that fails to parse is named with its own line, not one counted in
/// another block or in rustfmt's temp file.
#[test]
fn a_multi_block_syntax_error_names_the_block_and_its_line() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let text = "// ── crate.one ──\nfn one() -> i32 {\n    1\n}\n// ── crate.two ──\n// second\nfn two( {\n}\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(text.into()), None).unwrap_err().0;
    assert!(err.contains("crate.two"), "{err}");
    assert!(err.contains("line 3") && !err.contains("line 7"), "{err}");
    assert!(!err.contains(".solace/format-"), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// An `*** Add` into an inline `mod` names the container it was added to,
/// even when the item doesn't parse.
#[test]
fn an_add_to_an_inline_mod_names_its_container() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod tasks;\n"),
        ("src/tasks.rs", "mod tests {\n    use super::*;\n\n    fn helper() -> i32 {\n        1\n    }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: tasks::tests.ONE\n+let one = 1;\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(err.contains("tasks::tests.ONE"), "{err}");
    assert!(err.contains("added inside `tasks.tests` (mod)"), "{err}");
    assert!(!err.contains(".solace/format-"), "{err}");
}

/// A Rust patch rename is refused with the command that renames by stored name.
#[test]
fn a_patch_rename_names_submit_with_the_stored_name() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let patch = "*** Begin Patch\n*** Update: crate.one\n@@\n-pub fn one() -> i32 {\n+pub fn two() -> i32 {\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(
        err.contains("renames crate.one")
            && err.contains("`submit crate.one` with the new text renames it and keeps its id")
            && err.contains("change its callers in the same submit")
            && !err.contains("`*** Add:`"),
        "{err}"
    );
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

#[test]
fn a_unique_container_prefix_still_adds_to_it() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod other;\n"),
        ("src/other.rs", "mod tests {\n    pub fn one() -> i32 {\n        1\n    }\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: tests.zz_probe\n+pub fn zz_probe() -> i32 {\n+    9\n+}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved other.tests::zz_probe"), "{out}");
    let text = text_of(&root, "src/other.rs");
    assert!(text.contains("pub fn one()") && text.contains("pub fn zz_probe() -> i32 {\n        9\n    }"), "{text}");
}

#[test]
fn an_ambiguous_container_prefix_is_refused() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "mod tests {}\n"),
        ("src/b.rs", "mod tests {}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: tests.zz_probe\n+pub fn zz_probe() -> i32 {\n+    9\n+}\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(err.contains("E1002 ambiguous name: tests") && err.contains("a.tests") && err.contains("b.tests"), "{err}");
}

/// Formatting a whole module saves every item rustfmt's and keeps every id
/// (D-180).
#[test]
fn a_whole_module_format_keeps_every_id() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n    2\n}\n}\n\npub fn one( ) -> i32 {\n        1\n}\n";
    let (root, store) = rust_project(lib);
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n\npub fn one() -> i32 {\n    1\n}\n"
    );
    assert_eq!(ids(&store), before);
}

/// A whole-module format leaves rustfmt's default module reordering off, so a
/// module with unsorted `mod` lines still saves (D-180).
#[test]
fn a_whole_module_format_keeps_an_unsorted_mod_list() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "mod b;\nmod a;\n\npub fn one( ) -> i32 { 1 }\n"),
        ("src/a.rs", "pub fn a() -> i32 { 1 }\n"),
        ("src/b.rs", "pub fn b() -> i32 { 1 }\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "mod b;\nmod a;\n\npub fn one() -> i32 {\n    1\n}\n");
}

/// A whole-module format nests in the command's transaction, so a later
/// failure restores the file and the store (D-77 point 7).
#[test]
fn a_whole_module_format_rolls_back_with_the_command() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "pub fn one( ) -> i32 { 1 }\n";
    let (root, store) = rust_project(lib);
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let out = store.transaction::<String>(&[], || {
        store.format_module_rust("crate")?;
        fail("stop")
    });
    assert!(out.is_err());
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
    assert_eq!(ids(&store), before);
}

/// A line rustfmt leaves longer than `max_width` is named in the submit reply.
#[test]
fn a_line_rustfmt_leaves_is_named_in_the_submit_reply() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn check() {\n    assert!(true, \"ok\");\n}\n";
    let (root, store) = rust_project(lib);
    let text = format!("pub fn check() {{\n    assert!(true, \"{}\");\n}}\n", "x".repeat(120));
    let out = store.submit_symbols(Some("crate.check"), None, &text).unwrap();
    assert!(out.contains("Saved crate.check"), "{out}");
    let saved = text_of(&root, "src/lib.rs");
    let line = saved.lines().position(|line| line.chars().count() > 100).expect("rustfmt left the long line") + 1;
    assert!(out.contains(&format!("crate.check: line {line}")), "{out}");
    assert!(out.contains("rustfmt's max_width (100)"), "{out}");
    assert!(out.contains("as written"), "{out}");
}

/// An item rustfmt can lay out within `max_width` gets no long-line note.
#[test]
fn an_item_that_fits_gets_no_long_line_note() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn check() {\n    assert!(true, \"ok\");\n}\n";
    let (_root, store) = rust_project(lib);
    let text = "pub fn check() {\n    assert!(true, \"fine\");\n}\n";
    let out = store.submit_symbols(Some("crate.check"), None, text).unwrap();
    assert!(out.contains("Saved crate.check"), "{out}");
    assert!(!out.contains("rustfmt's max_width"), "{out}");
}

/// `format` names a long line rustfmt leaves, even when the file is unchanged.
#[test]
fn a_format_names_a_line_rustfmt_leaves_even_when_it_saves_nothing() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = format!("pub const MESSAGE: &str = \"{}\";\n", "x".repeat(120));
    let (root, store) = rust_project(&lib);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Unchanged: crate."), "{out}");
    let saved = text_of(&root, "src/lib.rs");
    let line = saved.lines().position(|line| line.chars().count() > 100).expect("rustfmt left the long line") + 1;
    assert!(out.contains(&format!("crate.MESSAGE: line {line}")), "{out}");
    assert!(out.contains("as written"), "{out}");
    assert!(crate::outside::Written::load(&root).unchanged("src/lib.rs"));
}

/// A nested item's name is not prefixed twice in the long-line note.
#[test]
fn a_format_names_an_overlong_nested_method_once() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib =
        format!("pub struct Store;\n\nimpl Store {{\n    pub fn open(&self) -> &str {{\n        \"{}\"\n    }}\n}}\n", "x".repeat(120));
    let (_root, store) = rust_project(&lib);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("crate.Store::open: line 2"), "{out}");
    assert!(!out.contains("Store::Store"), "{out}");
}

/// Resubmitting the same overlong item still names the line rustfmt left.
#[test]
fn resubmitting_an_overlong_item_names_the_line() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub fn check() {\n    assert!(true, \"ok\");\n}\n";
    let (root, store) = rust_project(lib);
    let text = format!("pub fn check() {{\n    assert!(true, \"{}\");\n}}\n", "x".repeat(120));
    store.submit_symbols(Some("crate.check"), None, &text).unwrap();
    let out = store.submit_symbols(Some("crate.check"), None, &text).unwrap();
    assert!(out.contains("Unchanged: crate.check"), "{out}");
    let saved = text_of(&root, "src/lib.rs");
    let line = saved.lines().position(|line| line.chars().count() > 100).expect("rustfmt left the long line") + 1;
    assert!(out.contains(&format!("crate.check: line {line}")), "{out}");
    assert!(out.contains("as written"), "{out}");
}

/// A whole-module submit names an overlong line in a changed nested method.
#[test]
fn a_whole_module_submit_names_an_overlong_nested_line() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> &str {\n        \"ok\"\n    }\n}\n";
    let (_root, store) = rust_project(lib);
    let text =
        format!("pub struct Store;\n\nimpl Store {{\n    pub fn open(&self) -> &str {{\n        \"{}\"\n    }}\n}}\n", "x".repeat(120));
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert!(out.contains("crate.Store::open: line 2"), "{out}");
    assert!(!out.contains("Store::Store"), "{out}");
}

/// A multi-block submit names a long line in a block that stayed unchanged.
#[test]
fn a_multi_block_submit_names_an_overlong_unchanged_item() {
    if !cargo::installed() {
        return;
    }
    let long = "x".repeat(120);
    let two = format!("pub fn two() -> &'static str {{\n    \"{long}\"\n}}\n");
    let lib = format!("pub fn one() -> i32 {{\n    1\n}}\n\n{two}");
    let (_root, store) = rust_project(&lib);
    let text = format!("// ── crate.one ──\npub fn one() -> i32 {{\n    2\n}}\n// ── crate.two ──\n{two}");
    let out = store.submit_symbols(None, None, &text).unwrap();
    assert!(out.contains("Saved crate.one"), "{out}");
    assert!(out.contains("crate.two: line 2"), "{out}");
}

/// A container's trailing comment over `max_width` is named too.
#[test]
fn a_format_names_an_overlong_container_tail() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = format!(
        "pub struct Store;\n\nimpl Store {{\n    pub fn open(&self) -> i32 {{\n        2\n    }}\n    // {}\n}}\n",
        "x".repeat(120)
    );
    let (_root, store) = rust_project(&lib);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("crate.Store: line 5"), "{out}");
}

/// Two `extern "C"` blocks of one name in a module are refused, not picked.
#[test]
fn an_ambiguous_container_within_a_module_is_refused() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("extern \"C\" {}\n\nextern \"C\" {}\n");
    let ix = store.index().unwrap();
    // The second name misses the direct lookup, so the cut loop must refuse it.
    for input in ["crate.extern \"C\"", "crate.extern \"C\".member"] {
        let err = store.rust_container_in(&ix, input).unwrap_err().0;
        assert!(err.contains("E1002 ambiguous name: crate.extern \"C\"") && err.contains("crate.extern \"C\" (extern)"), "{err}");
    }
}

/// A container tail that changed on its own still has its long line named.
#[test]
fn a_whole_module_submit_names_an_overlong_changed_tail() {
    if !cargo::installed() {
        return;
    }
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n    // short\n}\n";
    let (_root, store) = rust_project(lib);
    let text = format!(
        "pub struct Store;\n\nimpl Store {{\n    pub fn open(&self) -> i32 {{\n        2\n    }}\n    // {}\n}}\n",
        "x".repeat(120)
    );
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert!(out.contains("crate.Store: line 5"), "{out}");
}

/// A whole-module format saves a module whose child `mod` follows its items
/// (D-180), keeping every id.
#[test]
fn a_whole_module_format_keeps_a_trailing_mod_list() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "pub fn one( ) -> i32 { 1 }\n\nmod tests;\n"),
        ("src/tests.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "pub fn one() -> i32 {\n    1\n}\n\nmod tests;\n");
    assert_eq!(ids(&store), before);
}

/// A whole-module submit accepts a child `mod` that follows its items (D-168).
#[test]
fn a_whole_module_submit_keeps_a_trailing_mod_list() {
    if !cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n\nmod tests;\n"),
        ("src/tests.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let text = "pub fn one() -> i32 {\n    2\n}\n\nmod tests;\n";
    let out = store.submit(crate::store::Target::Module("crate"), text).unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), text);
}

/// A file left behind by a format is named on submit, and `format` repairs it
/// before the sequence continues.
#[test]
fn a_stale_module_file_is_named_and_format_repairs_it() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib =
        "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    store.submit_symbols(None, Some("crate.Store"), "pub fn two(&self) -> i32 {\n    2\n}\n").unwrap();
    store.submit_symbols(None, Some("crate"), "pub fn three() -> i32 {\n    3\n}\n").unwrap();
    // rustfmt ran on the file, but the record kept the text from before it.
    fs::write(root.join("src/lib.rs"), lib).unwrap();
    let err = store.submit_symbols(None, Some("crate"), "pub fn four() -> i32 {\n    Store.two() + three()\n}\n").unwrap_err().0;
    assert!(err.contains("E0104"), "{err}");
    assert!(err.contains("crate.Store::two") && err.contains("crate.three"), "{err}");
    assert!(err.contains("`format crate`"), "{err}");
    assert!(err.contains("replace it with the store's text"), "{err}");
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Repaired src/lib.rs") && out.contains("crate.Store::two"), "{out}");
    let kept = crate::testing::kept_copy(&root, &out);
    assert_eq!(fs::read_to_string(kept).unwrap(), lib);
    assert!(crate::outside::Written::load(&root).unchanged("src/lib.rs"));
    let text = text_of(&root, "src/lib.rs");
    assert!(text.contains("pub fn two") && text.contains("pub fn three"), "{text}");
    let out = store.submit_symbols(None, Some("crate"), "pub fn four() -> i32 {\n    Store.two() + three()\n}\n").unwrap();
    assert!(out.contains("Saved crate.four"), "{out}");
    assert!(text_of(&root, "src/lib.rs").contains("pub fn four"), "{}", text_of(&root, "src/lib.rs"));
}

/// A drift message names every differing symbol and the one repair command.
#[test]
fn a_drift_message_names_the_symbols_and_the_repair_command() {
    let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        1\n    }\n}\n";
    let (root, store) = rust_project(lib);
    fs::write(root.join("src/lib.rs"), lib.replace("        1\n", "        2\n")).unwrap();
    let err = store.check_report(false).unwrap_err().0;
    assert!(err.contains("E0104"), "{err}");
    assert!(err.contains("crate.Store::open"), "{err}");
    assert!(err.contains("`format crate`"), "{err}");
}

/// A repair is reported even when the formatted text then fails its check:
/// the file was already replaced, so its copy must be named.
#[test]
fn a_format_keeps_its_repair_note_when_the_check_fails() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "pub fn one( ) -> i32 {\n    \"no\"\n}\n";
    let (root, store) = rust_project(lib);
    let disk = "pub fn one() -> i32 {\n    1\n}\n";
    fs::write(root.join("src/lib.rs"), disk).unwrap();
    let err = store.format_module_rust("crate").unwrap_err().0;
    assert!(err.contains("Repaired src/lib.rs") && err.contains("kept at .solace/outside/"), "{err}");
    let kept = crate::testing::kept_copy(&root, &err);
    assert_eq!(fs::read_to_string(kept).unwrap(), disk);
}

/// `fn Foo` and `mod Foo` share no namespace, so a submit adds the function.
#[test]
fn a_function_beside_a_module_is_saved() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub mod Foo {}\n");
    let out = store.submit_symbols(None, Some("crate"), "pub fn Foo() -> i32 {\n    1\n}\n").unwrap();
    assert!(out.contains("Saved crate.Foo"), "{out}");
    assert!(text_of(&root, "src/lib.rs").contains("pub fn Foo"));
}

/// A unit struct names a type and a value, so `mod Foo` is an E1004 clash.
#[test]
fn a_struct_beside_a_module_is_refused() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("pub mod Foo {}\n");
    let err = store.submit_symbols(None, Some("crate"), "pub struct Foo;\n").unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("Foo"), "{err}");
}

/// A function and a constant share the value namespace, so cargo never sees
/// the duplicate: submit refuses it first.
#[test]
fn a_function_beside_a_const_is_refused() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("pub const Foo: i32 = 1;\n");
    let err = store.submit_symbols(None, Some("crate"), "pub fn Foo() {}\n").unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("Foo"), "{err}");
}

/// A `mod Foo;` declaration in the target's header is a type-namespace clash.
#[test]
fn a_struct_beside_a_file_module_is_refused() {
    if !cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", "pub mod Foo;\n"), ("src/Foo.rs", "pub fn inside() {}\n")]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let err = store.submit_symbols(None, Some("crate"), "pub struct Foo;\n").unwrap_err().0;
    assert!(err.starts_with("E1004") && err.contains("Foo"), "{err}");
}

/// A `mod Foo;` declaration is a type, so a value `fn Foo` does not clash.
#[test]
fn a_function_beside_a_file_module_is_saved() {
    if !cargo::installed() {
        return;
    }
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", "pub mod Foo;\n"), ("src/Foo.rs", "pub fn inside() {}\n")]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit_symbols(None, Some("crate"), "pub fn Foo() -> i32 {\n    1\n}\n").unwrap();
    assert!(out.contains("Saved crate.Foo"), "{out}");
    assert!(text_of(&root, "src/lib.rs").contains("pub fn Foo"));
}

/// Saving a module drops an empty inherent impl, keeping its alias, its user,
/// and a trait impl with no members.
#[test]
fn a_saved_module_drops_an_empty_inherent_impl() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub struct Sym;\n\n",
        "pub type Alias = Sym;\n\n",
        "pub trait Marker {}\n\n",
        "impl Alias {}\n\n",
        "impl Marker for Alias {}\n\n",
        "pub fn take(_: Alias) {}\n",
    );
    let (root, store) = rust_project(lib);
    let ix = store.index().unwrap();
    let text = super::module_text(&ix, &ix.modules[0]);
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("Dropped empty impl crate.Alias."), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert!(!text.contains("impl Alias {}"), "{text}");
    assert!(text.contains("pub type Alias = Sym;"), "{text}");
    assert!(text.contains("impl Marker for Alias {}"), "{text}");
    assert!(text.contains("pub fn take(_: Alias) {}"), "{text}");
    let ix = store.index().unwrap();
    assert!(!ix.syms.iter().any(|s| s.kind_text == "impl" && !s.name.starts_with('<')));
    assert!(ix.syms.iter().any(|s| s.kind_text == "impl" && s.name == "<Alias as Marker>"));
}

/// Editing an alias that shares its name with an empty inherent impl still
/// refuses a broken edit: dropping the impl doesn't hide the alias's errors.
#[test]
fn an_item_save_beside_an_empty_impl_still_checks_its_alias() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub struct Sym;\n\n",
        "pub struct Other;\n\n",
        "pub type Alias = Sym;\n\n",
        "impl Alias {}\n\n",
        "pub fn take(_: Alias) {}\n"
    );
    let (_root, store) = rust_project(lib);
    let broken = store.submit_symbols(Some("crate.Alias"), None, "pub type Alias = Missing;\n");
    assert!(broken.is_err(), "{broken:?}");
    let saved = store.submit_symbols(Some("crate.Alias"), None, "pub type Alias = Other;\n").unwrap();
    assert!(saved.contains("Saved crate.Alias."), "{saved}");
    assert!(saved.contains("Dropped empty impl crate.Alias."), "{saved}");
}

/// A dropped empty impl doesn't remove a type of the same name from the
/// changed list: the saved text is right, and so is the reply.
#[test]
fn a_saved_module_keeps_a_same_named_type_in_its_changed_note() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub struct Alias {\n    pub value: i32,\n}\n\nimpl Alias {}\n");
    let text = "pub struct Alias {\n    pub value: u64,\n}\n\nimpl Alias {}\n";
    let out = store.submit(crate::store::Target::Module("crate"), text).unwrap();
    assert!(out.contains("changed Alias"), "{out}");
    assert!(out.contains("Dropped empty impl crate.Alias."), "{out}");
    let written = text_of(&root, "src/lib.rs");
    assert!(written.contains("pub value: u64"), "{written}");
    assert!(!written.contains("impl Alias"), "{written}");
}

/// A dropped empty impl doesn't remove a newly added type of the same name
/// from the added list.
#[test]
fn a_saved_module_keeps_a_same_named_added_type_in_its_note() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn placeholder() {}\n\nimpl Alias {}\n");
    let text = concat!("pub fn placeholder() {}\n\n", "pub struct Alias {\n    pub value: i32,\n}\n\n", "impl Alias {}\n");
    let out = store.submit(crate::store::Target::Module("crate"), text).unwrap();
    assert!(out.contains("added Alias"), "{out}");
    assert!(out.contains("Dropped empty impl crate.Alias."), "{out}");
    let written = text_of(&root, "src/lib.rs");
    assert!(written.contains("pub struct Alias {"), "{written}");
    assert!(!written.contains("impl Alias"), "{written}");
}

/// A patch header names a generic method without its generics (D-193).
#[test]
fn an_update_name_may_leave_out_generics() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub struct Refs<'a> {\n",
        "    text: &'a str,\n",
        "}\n\n",
        "impl<'a> Refs<'a> {\n",
        "    pub fn new(text: &'a str) -> Self {\n",
        "        Refs { text }\n",
        "    }\n",
        "}\n"
    );
    let (root, store) = rust_project(lib);
    let patch = r#"*** Begin Patch
*** Update: crate.Refs::new
@@
-        Refs { text }
+        Self { text }
*** End Patch
"#;
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.Refs<'a>::new"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("Refs { text }", "Self { text }"));
}

/// A patch Add saves a type with the impl block it holds (D-194).
#[test]
fn a_patch_add_saves_a_type_with_its_impl() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let patch = concat!(
        "*** Begin Patch\n",
        "*** Add: crate.X\n",
        "+pub struct X {\n",
        "+    value: i32,\n",
        "+}\n",
        "+\n",
        "+impl X {\n",
        "+    pub fn new(value: i32) -> Self {\n",
        "+        Self { value }\n",
        "+    }\n",
        "+}\n",
        "*** End Patch\n",
    );
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.X, crate.X::new"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub struct X {\n    value: i32,\n}\n\n",
            "impl X {\n    pub fn new(value: i32) -> Self {\n        Self { value }\n    }\n}\n",
        )
    );
}

/// A method added after its type joins that type's impl, so the patch leaves
/// one `impl` (D-194).
#[test]
fn a_patch_adds_methods_to_a_type_it_added() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let patch = concat!(
        "*** Begin Patch\n",
        "*** Add: crate.X\n",
        "+pub struct X;\n",
        "*** Add: crate.X::new\n",
        "+pub fn new() -> Self {\n",
        "+    Self\n",
        "+}\n",
        "*** End Patch\n",
    );
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.X, crate.X::new"), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert_eq!(text.matches("impl X {").count(), 1, "{text}");
    assert_eq!(
        text,
        concat!(
            "pub fn one() -> i32 {\n    1\n}\n\n",
            "pub struct X;\n\n",
            "impl X {\n    pub fn new() -> Self {\n        Self\n    }\n}\n",
        )
    );
}

/// A new impl for a generic struct carries the struct's generics and drops
/// their defaults (D-194).
#[test]
fn a_patch_adds_a_method_to_a_generic_type() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let patch = concat!(
        "*** Begin Patch\n",
        "*** Add: crate.X\n",
        "+pub struct X<'a, T: Clone = i32> {\n",
        "+    value: &'a T,\n",
        "+}\n",
        "*** Add: crate.X::new\n",
        "+pub fn new(value: &'a T) -> Self {\n",
        "+    Self { value }\n",
        "+}\n",
        "*** End Patch\n",
    );
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved crate.X"), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert!(text.contains("impl<'a, T: Clone> X<'a, T> {"), "{text}");
    assert!(!text.contains("impl<'a, T: Clone = i32>"), "{text}");
}

/// A dead-code warning the submit leaves on another symbol is named under
/// its own heading.
#[test]
fn a_submit_names_a_dead_code_warning_it_leaves_elsewhere() {
    if !cargo::installed() {
        return;
    }
    let lib = r#"const TOOLS: i32 = 1;

pub fn uses_tools() -> i32 {
    TOOLS
}

#[cfg(test)]
mod tests {
    #[test]
    fn tools() {
        assert_eq!(super::TOOLS, 1);
    }
}
"#;
    let (_root, store) = rust_project(lib);
    store.check_report(false).unwrap();
    let out = store.submit_symbols(Some("crate.uses_tools"), None, "pub fn uses_tools() -> i32 {\n    2\n}\n").unwrap();
    assert!(out.contains("New warnings elsewhere:"), "{out}");
    assert!(out.contains("crate.TOOLS: warning:"), "{out}");
}

/// A warning that was there before the submit stays quiet.
#[test]
fn a_submit_does_not_repeat_a_warning_from_before() {
    if !cargo::installed() {
        return;
    }
    let lib = "const OLD: i32 = 1;\n\npub fn value() -> i32 {\n    1\n}\n";
    let (_root, store) = rust_project(lib);
    store.check_report(false).unwrap();
    let out = store.submit_symbols(Some("crate.value"), None, "pub fn value() -> i32 {\n    2\n}\n").unwrap();
    assert!(!out.contains("New warnings elsewhere"), "{out}");
    assert!(!out.contains("crate.OLD"), "{out}");
}

/// A whole-module Rust submit names a warning it leaves in another module.
#[test]
fn a_whole_module_submit_names_a_warning_it_leaves_elsewhere() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod other;\n\npub fn value() -> i32 {\n    other::helper()\n}\n";
    let other = "pub fn helper() -> i32 {\n    1\n}\n";
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", lib), ("src/other.rs", other)]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    store.check_report(false).unwrap();
    let ix = store.index().unwrap();
    let text = super::module_text(&ix, ix.modules.iter().find(|m| m.path == "crate").unwrap()).replace("other::helper()", "2");
    let out = store.submit(crate::store::Target::Module("crate"), &text).unwrap();
    assert!(out.contains("New warnings elsewhere:"), "{out}");
    assert!(out.contains("other.helper: warning:"), "{out}");
}

/// A patch named for a function beside a module changes only the function.
#[test]
fn a_patch_updates_the_function_beside_a_module() {
    if !cargo::installed() {
        return;
    }
    let lib = "mod g {\n    pub fn inside() {}\n}\n\npub fn g() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let update = concat!(
        "*** Begin Patch\n",
        "*** Update: crate.g\n",
        "@@\n",
        "-pub fn g() -> i32 {\n",
        "-    1\n",
        "-}\n",
        "+pub fn g() -> i32 {\n",
        "+    2\n",
        "+}\n",
        "*** End Patch\n"
    );
    let out = drafts::submit(&store, &["submit"], &mut || Ok(update.into()), None).unwrap();
    assert!(out.contains("Saved crate.g"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib.replace("    1\n", "    2\n"));
}

/// A format keeps rustfmt's one blank line after the header, not the old run (D-180).
#[test]
fn a_format_keeps_one_blank_line_after_the_header() {
    if !cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "use std::fmt;\n\n\n\npub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = rust_project(lib);
    let ids = |s: &Store| s.index().unwrap().syms.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    let before = ids(&store);
    let out = store.format_module_rust("crate").unwrap();
    assert!(out.contains("Saved crate."), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "use std::fmt;\n\npub fn one() -> i32 {\n    1\n}\n");
    assert_eq!(ids(&store), before);
}

/// `submit --new a.tests` makes a test module beside `a`, declared with
/// `#[cfg(test)]` and `#[path]` (D-206).
#[test]
fn new_creates_a_test_file_with_a_path_mod_line() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out =
        store.submit_symbols(None, Some("a.tests"), "use super::*;\n\n#[test]\nfn one_is_one() {\n    assert_eq!(one(), 1);\n}\n").unwrap();
    assert!(out.contains("created module a::tests (src/a_tests.rs; added mod tests; to a)"), "{out}");
    let tests = text_of(&root, "src/a_tests.rs");
    assert!(tests.contains("use super::*;") && tests.contains("fn one_is_one"), "{tests}");
    let module = text_of(&root, "src/a.rs");
    assert!(module.starts_with("#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n"), "{module}");
    assert!(store.index().unwrap().modules.iter().any(|m| m.path == "a::tests" && m.file.as_deref() == Some("src/a_tests.rs")));
}

/// An Add whose `::` would put its only item in a module of that name is
/// refused; a new module with another name or several items is unchanged.
#[test]
fn a_patch_add_refuses_an_item_wrapped_in_its_own_module() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod m;\n"),
        ("src/m.rs", "pub fn keep() -> i32 {\n    0\n}\n"),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let patch = "*** Begin Patch\n*** Add: m::f\n+fn f() {}\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(err.contains("would create module m::f") && err.contains("*** Add: m.f"), "{err}");
    assert!(!root.join("src/m/f.rs").exists());
    assert!(store.index().unwrap().modules.iter().all(|m| m.path != "m::f"));

    let patch = "*** Begin Patch\n*** Add: m.f\n+fn f() {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved m.f"), "{out}");

    let patch = "*** Begin Patch\n*** Add: m::helpers\n+fn f() {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("created module m::helpers"), "{out}");
    assert!(out.contains("Saved m::helpers.f"), "{out}");
    assert_eq!(text_of(&root, "src/m/helpers.rs"), "fn f() {}\n");

    let patch = "*** Begin Patch\n*** Add: m::f\n+fn f() {}\n+fn g() {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("created module m::f"), "{out}");
    assert!(out.contains("Saved m::f.f, m::f.g"), "{out}");

    let patch = "*** Begin Patch\n*** Add: m::f\n+fn h() {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved m::f.h"), "{out}");

    let inline = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod m;\n"),
        ("src/m.rs", "pub fn keep() -> i32 {\n    0\n}\n\nmod tests {\n    pub fn helper() -> i32 {\n        1\n    }\n}\n"),
    ]);
    crate::import(&inline).unwrap();
    let store = Store::at(&inline).unwrap();
    let patch = "*** Begin Patch\n*** Add: m::tests::t\n+fn t() {}\n*** End Patch\n";
    let err = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
    assert!(err.contains("*** Add: m::tests.t"), "{err}");

    let patch = "*** Begin Patch\n*** Add: m::tests.t\n+fn t() {}\n*** End Patch\n";
    let out = drafts::submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
    assert!(out.contains("Saved m.tests::t"), "{out}");
    let ix = store.index().unwrap();
    assert!(store.resolve(&ix, "m.tests::t").is_ok());
}

/// Every `*** Begin Patch … *** End Patch` block in the reference, with the
/// indentation it has there removed.
fn reference_patches() -> Vec<String> {
    let mut blocks = vec![];
    let mut open: Option<(usize, Vec<&str>)> = None;
    for line in crate::reference::REFERENCE.lines() {
        let text = line.trim_start();
        if text.starts_with("*** Begin Patch") {
            open = Some((line.len() - text.len(), vec![]));
        }
        if let Some((indent, lines)) = open.as_mut() {
            lines.push(line.get(*indent..).unwrap_or(""));
            if text.starts_with("*** End Patch") {
                blocks.push(lines.join("\n") + "\n");
                open = None;
            }
        }
    }
    blocks
}

/// The patches the reference shows agents must be ones `submit` and `write`
/// accept; a block that is only a placeholder (`...`) is skipped.
#[test]
fn the_reference_patches_parse_and_break_with_a_bad_header() {
    let blocks: Vec<String> = reference_patches().into_iter().filter(|b| !b.lines().any(|l| l.trim() == "...")).collect();
    assert!(blocks.len() >= 2, "{blocks:?}");
    for block in blocks {
        let files = block.contains("File:");
        let parsed = crate::drafts::sections(&block, files);
        assert!(parsed.is_ok_and(|s| !s.is_empty()), "{block}");
        let broken = block.replacen("\n*** ", "\n*** Bad", 1);
        assert!(crate::drafts::sections(&broken, files).is_err(), "{broken}");
    }
}

#[test]
fn a_method_for_a_type_without_an_impl_says_to_add_one() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("pub struct Y;\npub enum E { A }\npub type T = Y;\npub fn f() {}\n");
    let add = |name: &str| {
        let patch = format!("*** Begin Patch\n*** Add: {name}\n+pub fn c() -> i32 {{\n+    1\n+}}\n*** End Patch\n");
        drafts::submit(&store, &["submit"], &mut || Ok(patch.clone()), None).unwrap_err().0
    };
    let err = add("crate.Y::c");
    assert!(err.starts_with("E1003 crate.Y has no `impl`: add one with `*** Add: crate` and a whole `impl Y { … }` block"), "{err}");
    let err = add("crate.E::c");
    assert!(err.starts_with("E1003 crate.E has no"), "{err}");
    let err = add("crate.T::c");
    assert!(err.starts_with("E1003 crate.T has no"), "{err}");
    assert!(add("crate.Z::c").starts_with("E1003 unknown module: crate.Z::c"));
    assert!(add("crate.f::c").starts_with("E1003 unknown module: crate.f::c"));
}

/// `--imports` keeps a module's `mod` lines and their attributes, whether the
/// text repeats them or leaves them out (D-168).
#[test]
fn adding_a_use_to_a_fsx_shaped_module_keeps_its_mods() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod fsx;\n"),
        (
            "src/fsx.rs",
            concat!(
                "//! File primitives.\n\n",
                "use std::fs;\n\n",
                "#[cfg(test)]\n",
                "#[path = \"fsx_tests.rs\"]\n",
                "mod tests;\n\n",
                "pub fn size(path: &str) -> u64 {\n",
                "    fs::metadata(path).map(|m| m.len()).unwrap_or(0)\n",
                "}\n",
            ),
        ),
        ("src/fsx_tests.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let header = concat!(
        "//! File primitives.\n\n",
        "use std::fs;\n",
        "use std::time::{Duration, Instant};\n\n",
        "#[cfg(test)]\n",
        "#[path = \"fsx_tests.rs\"]\n",
        "mod tests;\n",
    );
    let out = store.submit(crate::store::Target::Imports("fsx"), header).unwrap();
    assert!(out.contains("Saved fsx imports"), "{out}");
    let saved = text_of(&root, "src/fsx.rs");
    assert!(saved.contains("use std::time::{Duration, Instant};"), "{saved}");
    assert!(saved.contains("#[cfg(test)]\n#[path = \"fsx_tests.rs\"]\nmod tests;\n"), "{saved}");
    assert!(saved.contains("pub fn size"), "{saved}");
    let uses_only = "use std::fs;\nuse std::time::{Duration, Instant};\nuse std::thread;\n";
    let out = store.submit(crate::store::Target::Imports("fsx"), uses_only).unwrap();
    assert!(out.contains("Saved fsx imports"), "{out}");
    let saved = text_of(&root, "src/fsx.rs");
    assert!(saved.contains("use std::thread;\n\n#[cfg(test)]\n#[path = \"fsx_tests.rs\"]\nmod tests;\n"), "{saved}");
    let no_cfg =
        concat!("//! File primitives.\n\n", "use std::fs;\n", "use std::thread;\n\n", "#[path = \"fsx_tests.rs\"]\n", "mod tests;\n",);
    let err = store.submit(crate::store::Target::Imports("fsx"), no_cfg).unwrap_err().0;
    assert!(
        err.contains("must leave out the module's `mod` lines or keep the same declarations, in any order")
            && err.contains("view fsx --header"),
        "{err}"
    );
    assert!(!err.contains(". a new module"), "{err}");
    assert_eq!(text_of(&root, "src/fsx.rs"), saved);
}

/// A `use`-only header puts the stored `mod` lines back before or after the
/// imports as they were, not all after the submitted text (D-168).
#[test]
fn a_use_only_submit_keeps_the_stored_mods_around_its_uses() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        (
            "src/lib.rs",
            concat!(
                "mod a;\nuse b::c;\npub mod d;\n\n",
                "pub fn one() -> i32 {\n    c()\n}\n\n",
                "pub mod b {\n    pub fn c() -> i32 {\n        1\n    }\n}\n",
            ),
        ),
        ("src/a.rs", ""),
        ("src/b.rs", "pub fn c() -> i32 {\n    1\n}\n"),
        ("src/d.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit(crate::store::Target::Imports("crate"), "use b::c;\nuse std::time::Duration;\n").unwrap();
    assert!(out.contains("Saved crate imports"), "{out}");
    assert_eq!(
        text_of(&root, "src/lib.rs"),
        concat!(
            "mod a;\nuse b::c;\nuse std::time::Duration;\npub mod d;\n",
            "pub fn one() -> i32 {\n    c()\n}\n\n",
            "pub mod b {\n    pub fn c() -> i32 {\n        1\n    }\n}\n",
        )
    );
}

/// A use-only submit may remove imports on either side of a stored `mod` (D-168).
#[test]
fn a_use_only_submit_can_drop_imports_around_a_stored_mod() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", concat!("use std::fmt;\nuse std::io;\nmod a;\nuse std::cmp;\n\n", "pub fn one() -> i32 {\n    1\n}\n",)),
        ("src/a.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit(crate::store::Target::Imports("crate"), "use std::fmt;\n").unwrap();
    assert!(out.contains("Saved crate imports"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), concat!("use std::fmt;\nmod a;\n", "pub fn one() -> i32 {\n    1\n}\n",));
}

#[test]
fn a_use_only_submit_may_reorder_stored_mods() {
    if !cargo::installed() {
        return;
    }
    let root = crate::testing::Scratch::new().with(&[
        ("Cargo.toml", CARGO),
        ("src/lib.rs", "mod a;\nmod b;\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/a.rs", ""),
        ("src/b.rs", ""),
    ]);
    crate::import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.submit(crate::store::Target::Imports("crate"), "mod b;\nmod a;\n").unwrap();
    assert!(out.contains("Saved crate imports"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "mod b;\nmod a;\npub fn one() -> i32 {\n    1\n}\n");
}

#[test]
fn an_unchanged_header_without_a_blank_line_drops_it() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("use std::fmt;\n\npub fn one() -> i32 {\n    1\n}\n");
    let out = store.submit(crate::store::Target::Imports("crate"), "use std::fmt;").unwrap();
    assert!(out.contains("Saved crate imports"), "{out}");
    assert_eq!(store.header("crate").unwrap(), "use std::fmt;\n");
}

/// `--imports` names an inline module and replaces only its own imports.
#[test]
fn an_import_added_to_an_inline_module_keeps_its_items() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub mod helper {\n",
        "    pub fn write() {}\n",
        "}\n\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    use super::*;\n\n",
        "    #[test]\n",
        "    fn write_runs() {\n",
        "        write();\n",
        "    }\n",
        "}\n",
    );
    let (root, store) = rust_project(lib);
    let out = store.submit(crate::store::Target::Imports("tests"), "use super::*;\nuse crate::helper::write;\n").unwrap();
    assert!(out.contains("Saved crate.tests imports"), "{out}");
    let text = text_of(&root, "src/lib.rs");
    assert!(
        text.contains(concat!(
            "mod tests {\n",
            "    use super::*;\n",
            "    use crate::helper::write;\n\n",
            "    #[test]\n",
            "    fn write_runs() {\n",
            "        write();\n",
            "    }\n",
            "}",
        )),
        "{text}"
    );
}

/// `--imports` on an inline module takes imports only: items and `mod` lines
/// are refused and nothing is saved.
#[test]
fn an_inline_imports_submit_refuses_items_and_mods() {
    if !cargo::installed() {
        return;
    }
    let lib = "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn one() {}\n}\n";
    let (root, store) = rust_project(lib);
    for text in ["use super::*;\nfn extra() {}\n", "use super::*;\nmod extra;\n"] {
        let err = store.submit(crate::store::Target::Imports("tests"), text).unwrap_err().0;
        assert!(err.contains("E0101 --imports text may only contain imports"), "{err}");
        assert_eq!(text_of(&root, "src/lib.rs"), lib);
    }
}

/// Dropping an import an inline test needs is refused, and the file is unchanged.
#[test]
fn an_inline_import_a_test_needs_is_not_dropped() {
    if !cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub mod helper {\n",
        "    pub fn write() {}\n",
        "}\n\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    use super::*;\n",
        "    use crate::helper::write;\n\n",
        "    #[test]\n",
        "    fn write_runs() {\n",
        "        write();\n",
        "    }\n",
        "}\n",
    );
    let (root, store) = rust_project(lib);
    let err = store.submit(crate::store::Target::Imports("tests"), "use super::*;\n").unwrap_err().0;
    assert!(err.contains("Not saved: the submitted text has errors."), "{err}");
    assert_eq!(text_of(&root, "src/lib.rs"), lib);
}

/// An empty inline module takes imports inside its braces.
#[test]
fn an_import_added_to_an_empty_inline_module_keeps_its_brace() {
    if !cargo::installed() {
        return;
    }
    let (root, store) = rust_project("#[cfg(test)]\nmod tests {}\n");
    let out = store.submit(crate::store::Target::Imports("tests"), "use std::fmt;\n").unwrap();
    assert!(out.contains("Saved crate.tests imports"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "#[cfg(test)]\nmod tests {\n    use std::fmt;\n}\n");
    let out = store.submit(crate::store::Target::Imports("tests"), "").unwrap();
    assert!(out.contains("Saved crate.tests imports"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "#[cfg(test)]\nmod tests {}\n");
}

/// Only a module takes `--imports`: an `impl` named the same way is not one.
#[test]
fn an_imports_submit_to_another_container_is_unknown() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("pub struct S;\n\nimpl S {}\n");
    let err = store.submit(crate::store::Target::Imports("S"), "use std::fmt;\n").unwrap_err().0;
    assert!(err.contains("E1003 unknown module: S"), "{err}");
}

/// `--imports` with no text clears an inline module's imports and keeps its items.
#[test]
fn clearing_an_inline_modules_imports_keeps_its_items() {
    if !cargo::installed() {
        return;
    }
    let lib = "#[cfg(test)]\nmod tests {\n    use std::fmt;\n\n    #[test]\n    fn one() {}\n}\n";
    let (root, store) = rust_project(lib);
    let out = store.submit(crate::store::Target::Imports("tests"), "").unwrap();
    assert!(out.contains("Saved crate.tests imports"), "{out}");
    assert_eq!(text_of(&root, "src/lib.rs"), "#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {}\n}\n");
}

/// An item below an inline module does not name the module for `--imports`.
#[test]
fn an_imports_submit_names_the_inline_module_not_its_items() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {}\n}\n");
    let err = store.submit(crate::store::Target::Imports("tests::one"), "use std::fmt;\n").unwrap_err().0;
    assert!(err.contains("E1003 unknown module: tests::one"), "{err}");
}

/// A duplicate in an inline `--imports` text names the module, not the save label.
#[test]
fn an_imports_submit_names_the_module_in_a_duplicate_refusal() {
    if !cargo::installed() {
        return;
    }
    let (_root, store) = rust_project("#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {}\n}\n");
    let err = store.submit(crate::store::Target::Imports("tests"), "use super::*;\nmod extra;\nmod extra;\n").unwrap_err().0;
    assert!(err.contains("defined twice in `crate.tests`"), "{err}");
    assert!(!err.contains("crate.tests imports"), "{err}");
}
