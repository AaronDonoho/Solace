use super::*;
use crate::Index;
use crate::reach::suites::short;
use std::collections::HashSet;
/// `refs Type.field`: who reads one field, sure or possible (D-44).
#[test]
fn field_refs_list_the_readers_of_a_field() {
    let ix = crate::index::test_index(&[
        ("src", "price", "pub type Quote {\n  Quote(discount: Int, total: Int)\n}\n\npub type Rate {\n  Rate(total: Int)\n}"),
        (
            "src",
            "report",
            "import price\n\npub fn discount(q: price.Quote) -> Int {\n  q.discount\n}\n\npub fn total(x) -> Int {\n  x.total\n}",
        ),
    ]);
    let quote = ix.syms.iter().position(|s| s.name == "Quote").unwrap();
    let out = field_refs(&ix, quote, "discount");
    assert!(out.starts_with("price.Quote.discount is read by 1:\n-- report.discount\n"), "{out}");
    let out = field_refs(&ix, quote, "total");
    assert!(out.contains("-- report.total (possible: another type has this field)"), "{out}");
    assert_eq!(field_refs(&ix, quote, "nope"), "price.Quote has no field nope.\n");
}

fn names(ix: &Index, impact: &Impact) -> Vec<String> {
    impact.uses.iter().filter(|u| u.steps > 0).map(|u| short(ix, u.sym)).collect()
}

#[test]
fn flow_follows_lets_fields_and_helpers() {
    let ix = crate::index::test_index(&[
        ("src", "money", "pub fn percent(x: Int, p: Int) -> Int {\n  x * p / 100\n}"),
        (
            "src",
            "price",
            concat!(
                "import money\n\npub type Quote {\n  Quote(discount: Int, total: Int, note: String)\n}\n\n",
                "pub type Rate {\n  Rate(total: Int)\n}\n\n",
                "pub fn quote(cents: Int, extra: Int) -> Quote {\n  let discount = money.percent(cents, 5)\n",
                "  let total = cents + extra\n  let note = \"fixed\"\n",
                "  Quote(discount: discount, total: total, note:)\n}\n\n",
                "pub fn wrap(q: Quote) -> Quote {\n  Quote(..q, total: q.total + 1, note: \"x\")\n}"
            ),
        ),
        (
            "src",
            "report",
            concat!(
                "import price\n\npub fn discounts(qs: List(price.Quote)) -> Int {\n  case qs {\n",
                "    [q, ..] -> q.discount\n    [] -> 0\n  }\n}\n\n",
                "pub fn totals(q) -> Int {\n  q.total\n}\n\n",
                "pub fn rate(r: price.Rate) -> Int {\n  r.total\n}"
            ),
        ),
        ("src", "other", "import money\n\npub fn tip(x: Int) -> Int {\n  money.percent(x, 10)\n}"),
        ("test", "price_test", "import price\n\npub fn quote_test() {\n  assert price.quote(100, 0).total == 100\n}"),
    ]);
    let quote = ix.syms.iter().position(|s| s.name == "quote").unwrap();
    // the changed `let discount` (line 1) feeds only `discount`
    let impact = gleam_flow(&ix, &[(quote, vec![1])]);
    let fields: Vec<&str> = impact.fields.iter().map(|(_, f)| f.as_str()).collect();
    assert_eq!(fields, ["discount"]);
    let found = names(&ix, &impact);
    // a reader in another module, reading the field from stored data
    assert!(found.contains(&"report.discounts".to_string()), "{found:?}");
    assert!(!found.iter().any(|n| n == "report.totals" || n == "report.rate"), "{found:?}");
    // the helper called with tainted data doesn't pull in its other callers
    assert!(!found.contains(&"other.tip".to_string()), "{found:?}");
    // a field two types declare: certain where the reader names one, possible where it names none
    let impact = gleam_flow(&ix, &[(quote, vec![2])]);
    let total = |n: &str| impact.uses.iter().find(|u| short(&ix, u.sym) == n).map(|u| u.certain);
    assert_eq!(total("report.totals"), Some(false));
    assert_eq!(total("report.rate"), None);
    assert!(impact.fields.iter().any(|(_, f)| f == "total") && !impact.fields.iter().any(|(_, f)| f == "discount"));
    // a record update: a field read from a tainted record counts only if that field is tainted
    let wrap = ix.syms.iter().position(|s| s.name == "wrap").unwrap();
    let fields = |marks: &[&str]| -> Vec<String> {
        let marks = marks.iter().map(|m| m.to_string()).collect();
        taint(&ix, &records(&ix), wrap, &HashSet::new(), &marks, &["q".into()]).fields.into_iter().map(|f| f.1).collect()
    };
    assert!(fields(&[]).is_empty());
    assert_eq!(fields(&[".total"]), ["total"]);
    assert_eq!(gleam_flow(&ix, &[(wrap, vec![0])]).fields.len(), 3, "a signature change taints every field");
}

/// A new symbol seeds every line; a comment-only change is no change at
/// all; changed lines that are comments don't seed.
#[test]
fn origins_seed_the_changed_lines_and_skip_what_is_not_code() {
    let ix = crate::index::test_index(&[
        ("src", "main", "pub fn a() -> Int {\n  1\n}\n\npub fn b() -> Int {\n  2\n  // c\n  3\n}"),
        ("test", "main_test", "import main\n\npub fn a_test() {\n  assert main.a() == 1\n}"),
    ]);
    let b = ix.syms.iter().position(|s| s.name == "b").unwrap();
    assert!(
        origins(&ix, &[("fn:main.a".to_string(), Some("pub fn a() -> Int {\n  // note\n  1\n}".to_string()))]).is_empty(),
        "a comment-only change is not code"
    );
    assert!(
        origins(
            &ix,
            &[("fn:main_test.a_test".to_string(), Some("import main\n\npub fn a_test() {\n  assert main.a() == 2\n}".to_string()))]
        )
        .is_empty(),
        "a test's own change is not an origin"
    );
    assert_eq!(origins(&ix, &[("fn:main.b".to_string(), None)]), [(b, vec![0, 1, 2, 3, 4])], "a new symbol seeds every line");
    assert_eq!(
        origins(&ix, &[("fn:main.b".to_string(), Some("pub fn b() -> Int {\n  2\n  // c\n}".to_string()))]),
        [(b, vec![3])],
        "the added code line seeds, its closing brace doesn't"
    );
    assert_eq!(
        origins(&ix, &[("fn:main.b".to_string(), Some("pub fn b() -> Int {\n  2\n}".to_string()))]),
        [(b, vec![3])],
        "the added code line seeds; the added comment line doesn't"
    );
}

/// A changed type seeds its users at the lines that name it, not at a call.
#[test]
fn a_type_change_seeds_the_lines_that_name_it() {
    let ix = crate::index::test_index(&[
        ("src", "shape", "pub type Shape {\n  Shape(a: Int)\n}"),
        ("src", "use", "import shape\n\npub fn f(s: shape.Shape) -> Int {\n  1\n}"),
    ]);
    let shape = ix.syms.iter().position(|s| s.name == "Shape").unwrap();
    let impact = gleam_flow(&ix, &[(shape, vec![0])]);
    assert_eq!(names(&ix, &impact), ["use.f"], "seeded at the line naming the type");
    assert_eq!(impact.uses.iter().find(|u| u.sym == shape).map(|u| u.steps), Some(0));
}

/// A piped call taints its first parameter when the pipeline's head is
/// tainted; a positional argument reaches the parameter at its position;
/// an argument that isn't tainted reaches none.
#[test]
fn piped_and_positional_arguments_taint_the_parameters_they_carry() {
    let ix = crate::index::test_index(&[
        ("src", "lib", "pub fn double(n: Int) -> Int {\n  n * 2\n}"),
        ("src", "main", "import lib\n\npub fn f(x: Int, y: Int) -> Int {\n  x |> lib.double()\n  lib.double(x)\n  lib.double(y)\n}"),
    ]);
    let double = ix.syms.iter().position(|s| s.name == "double").unwrap();
    let f = ix.syms.iter().position(|s| s.name == "f").unwrap();
    let marks: HashSet<String> = ["lib.double".to_string()].into();
    let t = taint(&ix, &records(&ix), f, &HashSet::new(), &marks, &["x".into()]);
    assert_eq!(t.calls, [(double, vec!["n".to_string()])], "the piped head and `double(x)` carry x, `double(y)` nothing");
    let t = taint(&ix, &records(&ix), f, &HashSet::new(), &marks, &[]);
    assert!(t.any && t.calls.is_empty(), "nothing is tainted, so no parameter is: {:?}", t.calls);
}

/// A body whose layout isn't the formatter's can't be segmented: the taint
/// takes the whole symbol, so every field of a record it writes counts.
#[test]
fn an_unformatted_body_is_taken_as_a_whole() {
    let ix = crate::index::test_index(&[
        ("src", "quote", "pub type Quote {\n  Quote(a: Int, b: Int)\n}"),
        ("src", "odd", "import quote\n\npub fn f(n: Int) -> quote.Quote {\n    let q = quote.Quote(a: n)\n  quote.Quote(..q, b: 1)\n}"),
    ]);
    let f = ix.syms.iter().position(|s| s.name == "f").unwrap();
    let mut fields: Vec<String> =
        taint(&ix, &records(&ix), f, &[1].into(), &HashSet::new(), &["n".into()]).fields.into_iter().map(|(_, g)| g).collect();
    fields.sort();
    fields.dedup();
    assert_eq!(fields, ["a", "b"], "the whole symbol is tainted, so every field counts");
}

/// A caller using an unqualified import — its own name or its `as` name —
/// is seeded by the symbol it writes, as an alias is.
#[test]
fn unqualified_imports_count_as_writing_the_symbol() {
    let ix = crate::index::test_index(&[
        ("src", "lib", "pub fn make() -> Int {\n  1\n}"),
        ("src", "plain", "import lib.{make}\n\npub fn a() {\n  make()\n}"),
        ("src", "renamed", "import lib.{make as m}\n\npub fn b() {\n  m()\n}"),
        ("src", "aliased", "import lib\n\npub fn c() {\n  lib.make()\n}"),
    ]);
    let make = ix.syms.iter().position(|s| s.name == "make").unwrap();
    assert_eq!(names(&ix, &gleam_flow(&ix, &[(make, vec![0, 1])])), ["plain.a", "renamed.b", "aliased.c"]);
}
