use super::*;

/// FFI paths follow the compiled module: nested, back up, between siblings,
/// and out of the package.
#[test]
fn ffi_paths_follow_the_module() {
    let moved = |from: &str, to: &str, path: &str| ffi_rel(&module_dir(to), &ffi_norm(&module_dir(from), path));
    assert_eq!(moved("board", "ui/task", "./x.mjs"), "../x.mjs");
    assert_eq!(moved("ui/task", "board", "../x.mjs"), "./x.mjs");
    assert_eq!(moved("ui/task", "board", "./x.mjs"), "./ui/x.mjs");
    assert_eq!(moved("ui/a/m", "ui/b/n", "./x.mjs"), "../a/x.mjs");
    assert_eq!(moved("a/m", "b/n", "./ffi/x.mjs"), "../a/ffi/x.mjs");
    assert_eq!(moved("a/m", "a/n", "./x.mjs"), "./x.mjs");
    assert_eq!(moved("board", "ui/task", "../gleam_stdlib/x.mjs"), "../../gleam_stdlib/x.mjs");
    let text = "@external(erlang, \"erlang\", \"abs\")\n@external(javascript, \"./x.mjs\", \"abs\")\npub fn abs(x: Int) -> Int";
    let spans = ffi_spans(text);
    assert_eq!(spans.len(), 1);
    assert_eq!(&text[spans[0].0..spans[0].1], "./x.mjs");
    assert!(ffi_spans("@external(javascript, \"lib\", \"f\")\npub fn f() -> Int").is_empty());
}

/// Relative specifiers of every import form, with their lines and brace names.
#[test]
fn js_imports_are_found() {
    let js = concat!(
        "import { a, b as c } from \"./x.mjs\";\nimport \"./side.mjs\";\n",
        "const m = await import('../y.mjs');\n",
        "export {\n  d,\n} from \"../gleam_stdlib/gleam.mjs\";\nimport lib from \"lib\";\nconst from_here = 1;\n",
    );
    let found: Vec<(String, usize, Vec<String>)> =
        js_imports(js).into_iter().map(|i| (js[i.start..i.end].to_string(), i.line, i.names)).collect();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        found,
        vec![
            ("./x.mjs".into(), 1, s(&["a", "b"])),
            ("./side.mjs".into(), 2, vec![]),
            ("../y.mjs".into(), 3, vec![]),
            ("../gleam_stdlib/gleam.mjs".into(), 6, s(&["d"])),
        ]
    );
}

/// Comments, strings and property calls are not imports; a namespace import
/// names what it uses as `m.name`, and a `$`-escaped name matches its plain one.
#[test]
fn js_imports_skip_noise_and_read_namespaces() {
    let js = concat!(
        "// import { a } from \"./c.mjs\";\n/* from \"./d.mjs\" */\n",
        "const s = \"import x from './e.mjs'\";\n",
        "const t = Array.from(\"./f.mjs\");\n",
        "import * as m from \"./board.mjs\";\n",
        "import { type$ } from \"./g.mjs\";\nm.abs(1);\nobj.m.name;\nobj?.m.other;\n",
    );
    let found: Vec<(String, Vec<String>)> = js_imports(js).into_iter().map(|i| (js[i.start..i.end].to_string(), i.names)).collect();
    assert_eq!(found, vec![("./board.mjs".to_string(), vec!["abs".to_string()]), ("./g.mjs".to_string(), vec!["type".to_string()])]);
}

