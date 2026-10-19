use super::*;

const SRC: &str = "//// Checkout.\n\nimport gleam/list\nimport gleam/order as ordering\nimport pilot/order.{type Order, Line as L}\n\n// section\n\n/// Pays.\npub fn pay(a: Int) -> Result(\n  Int,\n  Nil,\n) {\n  list.map([], fn(x) { L(x) })\n  // helper(\"no\")\n  helper(\"a.b\")\n}\n\n@deprecated(\"x\")\nfn helper(s: String) {\n  s\n}\n\npub type Status {\n  Draft\n  /// doc\n  Cancelled(reason: String)\n}\n\nconst limit = 3\n";

#[test]
fn splits_items_with_leading_comments() {
    let m = split(SRC);
    assert!(m.header.starts_with("//// Checkout.") && m.header.contains("import pilot/order"));
    let names: Vec<_> = m.items.iter().map(|i| (i.kind.as_str(), i.name.as_str())).collect();
    assert_eq!(names, [("fn", "pay"), ("fn", "helper"), ("type", "Status"), ("const", "limit")]);
    assert!(m.items[0].text.starts_with("/// Pays."));
    assert!(m.items[1].text.starts_with("@deprecated"));
    assert!(m.header.ends_with("// section"));
}

#[test]
fn signatures_and_constructors() {
    let m = split(SRC);
    assert_eq!(signature(&m.items[0]), "/// Pays.\npub fn pay(a: Int) -> Result(\n  Int,\n  Nil,\n)");
    // The whole doc, and not a constructor's doc for the type's.
    let item = Item { text: "/// Pays.\n// note\n/// Never twice.\npub fn pay() {\n  1\n}".into(), ..m.items[0].clone() };
    assert_eq!(signature(&item), "/// Pays.\n/// Never twice.\npub fn pay()");
    assert!(!signature(&m.items[2]).starts_with("/// doc"));
    assert_eq!(constructors(&m.items[2].text), ["Draft", "Cancelled"]);
}

#[test]
fn parses_imports() {
    let m = split(SRC);
    let i = imports(&m.header);
    let got: Vec<_> = i.iter().map(|i| (i.module.as_str(), i.alias.as_str(), i.unqualified.len())).collect();
    assert_eq!(got, [("gleam/list", "list", 0), ("gleam/order", "ordering", 0), ("pilot/order", "order", 2)]);
    assert_eq!(i[2].unqualified[1], ("Line".to_string(), "L".to_string()));
}

/// The `alias.name` references in `code`.
fn qualified(code: &str) -> Vec<String> {
    references(code).iter().filter_map(|r| if let Ref::Qualified(a, b) = r { Some(format!("{a}.{b}")) } else { None }).collect()
}
/// The bare-name references in `code`.
fn bare_names(code: &str) -> Vec<String> {
    references(code).iter().filter_map(|r| if let Ref::Bare(b) = r { Some(b.clone()) } else { None }).collect()
}

#[test]
fn finds_references_outside_strings_and_comments() {
    let m = split(SRC);
    let bare = bare_names(&m.items[0].text);
    assert_eq!(qualified(&m.items[0].text), ["list.map"]);
    assert!(bare.iter().any(|b| b == "helper") && bare.iter().any(|b| b == "L") && !bare.iter().any(|b| b == "b"));
}

/// Spans are byte offsets into the original text, also after a string
/// literal, a trailing comment and multibyte text.
#[test]
fn reference_spans_are_byte_offsets_of_the_source() {
    let code = "pub fn f(t: Task) {\n  let s = \"é Task\" // Task\n  board.go(Task(1), s)\n}";
    let spans: Vec<(String, &str)> = references_spans(code)
        .into_iter()
        .map(|(r, a, b)| (match r { Ref::Bare(n) => n, Ref::Qualified(a, m) => format!("{a}.{m}") }, &code[a..b]))
        .filter(|(n, _)| n == "Task" || n == "board.go")
        .collect();
    assert_eq!(spans, [("Task".into(), "Task"), ("board.go".into(), "board.go"), ("Task".into(), "Task")]);
    assert!(bound_names("pub fn f(task: Int) {\n  let x = 1\n  x\n}").is_superset(&["task".to_string(), "x".to_string()].into()));
}

#[test]
fn record_fields_and_field_reads() {
    let ty = "pub type Quote {\n  Quote(\n    /// the lines\n    lines: List(Line),\n    total_cents: Int,\n  )\n  Empty\n  Pair(Int, label: String)\n}";
    let none: Vec<Option<String>> = vec![];
    assert_eq!(
        record_fields(ty),
        [
            ("Quote".to_string(), vec![Some("lines".to_string()), Some("total_cents".to_string())]),
            ("Empty".to_string(), none),
            ("Pair".to_string(), vec![None, Some("label".to_string())]),
        ]
    );
    let code = "pub fn f(q: Quote) {\n  let t = q.total_cents\n  let Quote(lines: ls, ..) = q\n  let r = Quote(..q, total_cents: tax.rate)\n  case g(q).lines {\n    Pair(_, label:) -> label\n  }\n  r.inner.depth\n}";
    let got: Vec<(String, usize)> = field_reads(code);
    let got: Vec<(&str, usize)> = got.iter().map(|(f, l)| (f.as_str(), *l)).collect();
    assert_eq!(got, [("total_cents", 1), ("lines", 4), ("inner", 7), ("depth", 7), ("lines", 2), ("label", 5)]);
}

#[test]
fn locals_shadow_top_level_names() {
    let code = "pub fn f(quote: Quote, shipping: S) -> Int {\n  let tax = tax.quote(1)\n  let assert Ok(#(db, o)) = g(o)\n  list.map([], fn(order) { order.id })\n  case x {\n    Ok(line) if line > 0 -> line + tax.total + db\n    order.Paid -> order.x\n  }\n  shipping.cost + quote.total + h(label: 1)\n}";
    let q = qualified(code);
    let bare = bare_names(code);
    assert_eq!(q, ["tax.quote", "list.map", "order.Paid", "order.x"]);
    assert!(bare.contains(&"g".to_string()) && bare.contains(&"o".to_string()) && bare.contains(&"h".to_string()));
    assert!(!bare.contains(&"quote".to_string()) && !bare.contains(&"line".to_string()) && !bare.contains(&"label".to_string()));
    // a `use` or `let` pattern binds its names; it doesn't refer to symbols of that name
    let bare = bare_names("pub fn f() {\n  use request <- try(g())\n  let tax = 1\n  request + tax\n}");
    assert!(
        bare.contains(&"g".to_string()) && !bare.contains(&"request".to_string()) && !bare.contains(&"tax".to_string()),
        "{bare:?}"
    );
    // a binding starts after its whole right-hand side, however many lines it takes
    let code = "pub fn f() {\n  use customer <- result.try(\n    customer.Customer(id: 1)\n    |> customer.validate,\n  )\n  customer.email\n}";
    assert_eq!(qualified(code), ["result.try", "customer.Customer", "customer.validate"]);
}

/// `declaration` takes `opaque`, and wants a name: `type` and `fn` alone,
/// `const1`, and a word that is no declaration give nothing.
#[test]
fn declarations_take_opaque_and_refuse_an_empty_name() {
    assert_eq!(declaration("pub opaque type T"), Some((Kind::Type, "T".to_string())));
    assert_eq!(declaration("opaque fn x() { 1 }"), Some((Kind::Fn, "x".to_string())));
    assert_eq!(declaration("const x = 1"), Some((Kind::Const, "x".to_string())));
    for bad in ["type ", "fn ", "const ", "pub", "pub fn", "const1 = 1", "mytype", ""] {
        assert_eq!(declaration(bad), None, "{bad}");
    }
}

/// A body's `{` is the first one outside brackets; a declaration with no
/// body keeps its whole head.
#[test]
fn body_start_skips_braces_inside_brackets() {
    assert_eq!(body_start("fn f(a: Int) { 1 }"), Some(13));
    assert_eq!(body_start("fn f(a: #(Int, Int)) { 1 }"), Some(21));
    assert_eq!(body_start("fn f(a: Int) -> Int"), None);
    let item = Item { kind: Kind::Fn, name: "f".into(), text: "pub fn f() -> Int".into() };
    assert_eq!(head(&item), "pub fn f() -> Int", "no body brace: the head is the whole declaration");
}

/// Nested brackets keep one part; a trailing comma and a missing close
/// give the parts there are, and a lone close gives none.
#[test]
fn split_args_nests_and_keeps_only_the_parts_there_are() {
    assert_eq!(split_args("a(b, c), [d, e])"), [(0, "a(b, c)".to_string()), (8, " [d, e]".to_string())]);
    assert_eq!(split_args("a, b,)"), [(0, "a".to_string()), (2, " b".to_string())]);
    assert_eq!(split_args("a, b"), [(0, "a".to_string()), (2, " b".to_string())]);
    assert_eq!(split_args(")"), []);
}

/// A field label starts with a lowercase letter; `Label:` is a value, not a label.
#[test]
fn record_fields_wants_a_lowercase_label() {
    let ty = "pub type Shape {\n  Shape(Int, Label: String)\n}";
    assert_eq!(record_fields(ty), [("Shape".to_string(), vec![None, None])]);
}

/// Only an `import` at a line start counts: one in a doc comment, and one
/// after other text on its line, are left out. A `.{` with no close takes
/// the rest of the header as its list.
#[test]
fn imports_only_at_a_line_start() {
    let i = imports("//// uses import gleam/int\nimport gleam/string\nx import gleam/float\n");
    assert_eq!(i.len(), 1);
    assert_eq!(i[0].module, "gleam/string");
    let i = imports("import gleam/option.{Some as S\nas o\n");
    assert_eq!((i[0].module.as_str(), i[0].alias.as_str()), ("gleam/option", "option"));
    assert_eq!(i[0].unqualified[0], ("Some".to_string(), "S\nas o".to_string()));
}

/// String literals and comments are blanked out; the quotes and the line
/// breaks are kept, so the lines still match the source's. An escaped
/// quote is skipped, so the string ends at the quote that closes it.
#[test]
fn strip_comments_blanks_strings_and_comments() {
    assert_eq!(strip_comments("let s = \"a // b\""), "let s = \"      \"");
    assert_eq!(strip_comments("let s = \"\\\"\" // c\nx"), "let s = \"  \"     \nx");
    assert_eq!(strip_comments("let s = \"a\nb\" // c\nx"), "let s = \" \n \"     \nx");
    assert_eq!(strip_comments("// whole\nx"), "        \nx");
    // offsets still match the source's, also after multibyte text
    let s = "let s = \"é\" // ü\nx";
    assert_eq!(strip_comments(s).len(), s.len());
}

/// `..base` in a record update names a value, not a field; `module::` is
/// two names, not a label.
#[test]
fn a_record_update_base_is_a_value_and_a_double_colon_is_no_label() {
    let code = "pub fn f(q: Quote) {\n  let r = Quote(..base, x: 1)\n  r\n}";
    let reads = field_reads(code);
    assert!(!reads.iter().any(|(f, _)| f == "base"), "{reads:?}");
    assert!(bare_names(code).contains(&"base".to_string()), "{:?}", bare_names(code));
    assert_eq!(bare_names("a::b"), ["a", "b"]);
}

/// A binding lasts to the line indented no more than its `let`: the `x`
/// after the function is a name, the one inside is not. Keywords in a
/// pattern are not names, and `mod.Ctor` binds only `x`.
#[test]
fn a_binding_ends_at_its_indent_and_keywords_are_no_names() {
    let code = "pub fn f() {\n  let x = 1\n  x\n}\nx";
    let bare = bare_names(code);
    assert_eq!(bare.iter().filter(|b| *b == "x").count(), 1, "{bare:?}");
    assert_eq!(pattern_names("Ok(a) if b -> c"), ["a", "b", "c"]);
    assert_eq!(pattern_names("Ok(a) as b"), ["a", "b"]);
    assert_eq!(pattern_names("mod.Ctor(x)"), ["x"]);
    assert_eq!(pattern_names("_"), ["_"]);
}
