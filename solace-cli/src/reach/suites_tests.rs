use super::*;
use crate::reach::flow::{Impact, Use};
use testing::*;

/// Declared commands with `checks` are suites: they reach what their
/// entries reach, a `gleam run -m` module's `main` included.
#[test]
fn declared_suites_reach_their_entries() {
    let store = project(&[
        ("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n"),
        ("src/e2e.gleam", "import calc\n\npub fn main() {\n  calc.sub(2, 1)\n}\n"),
    ]);
    fs::write(
        store.root.join("solace.toml"),
        concat!(
            "[commands]\ne2e = { run = [\"gleam\", \"run\", \"-m\", \"e2e\"], ",
            "checks = [\"calc.add\"] }\nall = { run = [\"make\"], checks = [\"*\"] }\n"
        ),
    )
    .unwrap();
    let ix = store.index().unwrap();
    let suites = suites(&store, &ix).unwrap();
    let id = |name: &str| ix.syms.iter().position(|s| s.name == name).unwrap();
    assert_eq!(reached(&suites, &[id("add")]), ["e2e (`run e2e`)", "all (`run all`)"]);
    assert_eq!(reached(&suites, &[id("sub")]), ["e2e (`run e2e`)", "all (`run all`)"]);
    assert!(reached(&suites, &[]).is_empty());
}

/// The note and the report cut long lists: fields past eight, untested uses
/// past three, possible fields past four, and uses past the size limit.
#[test]
fn the_note_and_the_report_cut_their_lists() {
    let ix = crate::index::test_index(&[
        (
            "src",
            "main",
            concat!(
                "pub fn a() {\n  1\n}\n\npub fn b() {\n  1\n}\n\n",
                "pub fn c() {\n  1\n}\n\npub fn d() {\n  1\n}\n\n",
                "pub fn e() {\n  1\n}\n\npub fn f() {\n  1\n}\n\n",
                "pub fn g() {\n  1\n}\n\npub fn h() {\n  1\n}\n\n",
                "pub fn i() {\n  1\n}\n\npub fn j() {\n  1\n}\n\n",
                "pub fn k() {\n  1\n}\n\npub fn l() {\n  1\n}"
            ),
        ),
        ("src", "shape", "pub type T {\n  T(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int, i: Int)\n}"),
    ]);
    let t = ix.syms.iter().position(|s| s.name == "T").unwrap();
    let mut impact = Impact::default();
    impact.uses.push(Use { sym: 0, steps: 0, via: "changed".to_string(), certain: true });
    for i in 1..5 {
        impact.uses.push(Use { sym: i, steps: 1, via: "calls main.a".to_string(), certain: true });
    }
    for k in 0..6 {
        impact.uses.push(Use { sym: 5 + k, steps: 1, via: format!("reads T.{}", (b'a' + k as u8) as char), certain: false });
    }
    impact.fields = (0..9).map(|k| (t, format!("{}", (b'a' + k as u8) as char))).collect();
    impact.leaves = vec![(0, "prints".to_string())];
    let lines = note(&ix, &impact, &["e2e (`run e2e`)".to_string()]);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].starts_with("Reaches 4 uses (and 6 possible): callers, and readers of T.a, .b, .c, .d, .e, .f, .g, .h, and 1 more."),
        "{lines:?}"
    );
    assert!(lines[1].starts_with("No test of their own: main.b (1 step), main.c (1 step), main.d (1 step) and 1 more"), "{lines:?}");
    assert_eq!(lines[2], "Leaves the program: main.a (prints). Also reached: e2e (`run e2e`).");
    let r = report(&ix, &impact, &["e2e (`run e2e`)".to_string()], 10_000);
    assert!(r.starts_with("main.a reaches 4 uses, nearest first:\n  main.b — 1 step, calls main.a; no test of its own\n"), "{r}");
    assert!(r.contains("And 6 possible uses: readers of a field another type also declares (T.a, T.b, T.c, T.d and 2 more)"), "{r}");
    let r = report(&ix, &impact, &[], 10);
    assert!(r.contains("  … 4 more not shown, to fit the size limit"), "{r}");
}

/// A declared command whose entries don't resolve is a suite that reaches
/// nothing, and it is skipped silently.
#[test]
fn suites_skip_entries_that_do_not_resolve() {
    let store = project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
    fs::write(store.root.join("solace.toml"), "[commands]\nghost = { run = [\"make\"], checks = [\"calc.gone\"] }\n").unwrap();
    let ix = store.index().unwrap();
    let suites = suites(&store, &ix).unwrap();
    assert_eq!(suites.len(), 1);
    assert!(suites[0].1.as_ref().is_some_and(|r| r.is_empty()), "{suites:?}");
    assert!(reached(&suites, &[0]).is_empty(), "nothing is reached, so no suite is listed");
}

/// A possible field use is counted and named, but not listed as a reader.
#[test]
fn a_possible_use_is_counted_not_listed() {
    let ix = crate::index::test_index(&[
        ("src", "price", "pub type Quote {\n  Quote(total: Int)\n}\n\npub type Rate {\n  Rate(total: Int)\n}"),
        ("src", "report", "pub fn totals(q) -> Int {\n  q.total\n}\n\npub fn rate(r: price.Rate) -> Int {\n  r.total\n}"),
    ]);
    let quote = ix.syms.iter().position(|s| s.name == "Quote").unwrap();
    let totals = ix.syms.iter().position(|s| s.name == "totals").unwrap();
    let impact = Impact {
        uses: vec![
            Use { sym: quote, steps: 0, via: "changed".to_string(), certain: true },
            Use { sym: totals, steps: 1, via: "reads Quote.total".to_string(), certain: false },
        ],
        fields: vec![(quote, "total".to_string())],
        leaves: vec![],
    };
    let r = report(&ix, &impact, &[], 10_000);
    assert!(r.contains("And 1 possible use: readers of a field another type also declares (Quote.total)"), "{r}");
    assert!(!r.contains("report.totals"), "{r}");
    let n = note(&ix, &impact, &[]);
    assert!(n[0].contains("(and 1 possible)") && !n.iter().any(|l| l.contains("report.totals")), "{n:?}");
}

/// A certain field reader is named by its field and marked as untested.
#[test]
fn a_field_reader_is_named_with_its_field() {
    let ix = crate::index::test_index(&[
        ("src", "price", "pub type Quote {\n  Quote(discount: Int)\n}"),
        ("src", "report", "pub fn discounts(q) -> Int {\n  q.discount\n}"),
    ]);
    let quote = ix.syms.iter().position(|s| s.name == "Quote").unwrap();
    let discounts = ix.syms.iter().position(|s| s.name == "discounts").unwrap();
    let impact = Impact {
        uses: vec![
            Use { sym: quote, steps: 0, via: "changed".to_string(), certain: true },
            Use { sym: discounts, steps: 1, via: "reads Quote.discount".to_string(), certain: true },
        ],
        fields: vec![(quote, "discount".to_string())],
        leaves: vec![],
    };
    let n = note(&ix, &impact, &[]);
    assert!(n[0].starts_with("Reaches 1 use: readers of Quote.discount."), "{n:?}");
    assert!(n[1].starts_with("No test of their own: report.discounts (1 step, reads discount)"), "{n:?}");
}
