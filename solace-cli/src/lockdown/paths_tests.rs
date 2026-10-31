use super::*;
use crate::testing::*;

#[test]
fn paths_name_a_rust_module_only_when_certain() {
    let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
    for (path, module) in [
        ("src/lib.rs", Some("crate")),
        ("src/main.rs", None),
        ("src/a.rs", Some("a")),
        ("src/a/mod.rs", Some("a")),
        ("src/a/b.rs", Some("a::b")),
        ("src/a/b/mod.rs", Some("a::b")),
        ("tests/x.rs", Some("integration::x")),
        ("tests/x/mod.rs", None),
        ("src/bin/x.rs", None),
        ("examples/x.rs", None),
        ("benches/x.rs", None),
        ("build.rs", None),
    ] {
        assert_eq!(module_of_file(&s, path).as_deref(), module, "{path}");
    }
    let no_lib = Scratch::new();
    assert_eq!(rust_module_of_path(&no_lib, "src/main.rs").as_deref(), Some("crate"));
}

