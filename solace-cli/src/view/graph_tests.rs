use super::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

#[test]
fn c1_graph_has_entries_effects_and_tests() {
    let ix = crate::index::test_index(&[
        (
            "src",
            "app",
            r#"import gleam/io
@external(erlang, "file", "write_file")
fn write(path: String, text: String) -> Nil
@external(erlang, "init", "get_plain_arguments")
fn args() -> List(String)
pub fn main() {
  let xs = args()
  io.println("hello")
  write("out", "hi")
}"#,
        ),
        (
            "test",
            "app_test",
            r#"import app
pub fn main_test() {
  assert app.main() == Nil
}"#,
        ),
    ]);
    let g = graph_index(&ix, &[], &HashSet::new());
    assert!(g["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "fn:app.main" && n["kind"] == "entry"));
    for cat in ["prints", "writes files", "environment"] {
        assert!(g["edges"].as_array().unwrap().iter().any(|e| e["from"] == "fn:app.main" && e["to"] == format!("effect:{cat}")));
    }
    assert!(g["edges"].as_array().unwrap().iter().any(|e| e["kind"] == "tests" && e["to"] == "fn:app.main"));
}

#[test]
fn c2_pilot_date_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
    let mut sources = vec![];
    let mut owned = vec![];
    for dir in ["src", "test"] {
        let mut paths = vec![];
        gleam::collect_gleam_files(&root.join(dir), &mut paths).unwrap();
        for path in paths {
            let module = path.strip_prefix(root.join(dir)).unwrap().with_extension("").to_string_lossy().to_string();
            owned.push((dir.to_string(), module, fs::read_to_string(path).unwrap()));
        }
    }
    for (dir, path, text) in &owned {
        sources.push((dir.as_str(), path.as_str(), text.as_str()));
    }
    let g = graph_index(&crate::index::test_index(&sources), &[], &HashSet::new());
    assert!(g["nodes"].as_array().unwrap().iter().any(|n| n["module"] == "pilot/api" && n["name"] == "handle" && n["kind"] == "entry"));
    assert!(g["nodes"].as_array().unwrap().iter().any(|n| {
        n["module"] == "pilot/date_test"
            && n["checks"].as_array().is_some_and(|cs| {
                cs.iter().any(|c| c["call"] == "date.add_business_days(Date(2026, 3, 5), 2)" && c["expected"] == "Date(2026, 3, 9)")
            })
    }));
}

#[test]
fn plain_words_requests_state_and_areas() {
    assert_eq!(words("Result(List(Order), Error)"), "a list of Order or an error");
    assert_eq!(words("#(Store, Response)"), "Store and Response");
    assert_eq!(words("option.Option(Int)"), "a number or nothing");
    assert_eq!(words("Result(Nil, String)"), "success or an error");
    assert_eq!(words("Dict(String, Int)"), "a number by text");
    let changer = |m: &str| {
        format!("import app/db.{{type Db}}\npub fn change(d: Db, order_id: Int) -> Db {{ d }}\npub fn read_{m}(d: Db) -> Int {{ 1 }}")
    };
    let (a, b) = (changer("a"), changer("b"));
    let ix = crate::index::test_index(&[
        (
            "src",
            "app/db",
            "//// The program's data.\n////\n//// More detail.\npub type Db {\n  Db(n: Int)\n}\npub fn new() -> Db { Db(0) }",
        ),
        ("src", "app/a", &a),
        ("src", "app/b", &b),
        (
            "src",
            "app/api",
            concat!(
                "import app/db.{type Db}\n",
                "pub type Colour {\n  Red\n  Green\n  Blue\n}\n",
                "pub fn paint(c: Colour) -> Int { 1 }\n",
                "pub type Request {\n  CreateOrder(customer_id: Int)\n  AddItem(order_id: Int, sku: String)\n  Cancel(Int)\n}\n",
                "pub fn handle(d: Db, request: Request) -> #(Db, String) { #(d, \"\") }",
            ),
        ),
    ]);
    let g = graph_index(&ix, &[], &HashSet::new());
    let node = |module: &str, name: &str| {
        g["nodes"].as_array().unwrap().iter().find(|n| n["module"] == module && n["name"] == name).unwrap().clone()
    };
    let handle = node("app/api", "handle");
    assert_eq!(handle["takes"], json!(["Db", "Request"]));
    assert_eq!(handle["gives"], "Db and text");
    assert_eq!(
        handle["requests"],
        json!([
            {"name": "Create order", "fields": ["customer id"]},
            {"name": "Add item", "fields": ["order id", "sku"]},
            {"name": "Cancel", "fields": ["a value"]},
        ])
    );
    assert_eq!(node("app/api", "paint")["requests"], json!([]));
    assert_eq!(node("app/a", "change")["takes"], json!(["Db", "order id"]));
    let effect = |from: &Value, to: &str| {
        g["edges"].as_array().unwrap().iter().any(|e| e["from"] == from["id"] && e["to"] == to && e["kind"] == "uses")
    };
    assert!(effect(&handle, "effect:changes Db"));
    assert!(effect(&node("app/a", "change"), "effect:changes Db"));
    assert!(effect(&node("app/b", "read_b"), "effect:reads Db"));
    assert!(!effect(&node("app/db", "new"), "effect:changes Db"));
    assert!(
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == "effect:changes Db" && n["summary"].as_str().unwrap().contains("returns a changed one"))
    );
    assert_eq!(g["areas"][3], json!({"id":"app/db","name":"db","summary":"The program's data."}));
    let ix = crate::index::test_index(&[("src", "app/db", "pub type Db {\n  Db(n: Int)\n}"), ("src", "app/a", &a), ("src", "app/b", &b)]);
    assert!(!graph_index(&ix, &[], &HashSet::new())["nodes"].as_array().unwrap().iter().any(|n| n["kind"] == "effect"));
}
