//! Reach and impact (D-44): which tests can see a change, and where a change's
//! data goes. Built on the reference graph and the formatter's layout, like
//! the rest of the Gleam support: derived, never a guarantee.
use super::*;
use std::collections::VecDeque;

use std::collections::HashMap;
pub(super) mod flow;
mod suites;
use crate::reach::flow::Use;
use crate::reach::flow::Impact;
/// Test functions: public `_test` functions in test modules (Gleam), or
/// `#[test]` functions (Rust, D-170).
pub fn test_symbols(ix: &Index) -> Vec<usize> {
    (0..ix.syms.len())
        .filter(|&i| {
            let s = &ix.syms[i];
            is_test(ix, i) && s.kind == Kind::Fn && is_test_fn(ix, i)
        })
        .collect()
}

/// Whether `i` is a test function: a Gleam `pub …_test`, or a Rust `#[test]`.
pub fn is_test_fn(ix: &Index, i: usize) -> bool {
    let s = &ix.syms[i];
    if ix.rust {
        language::rust_refs::has_test_attr(&s.text)
    } else {
        s.name.ends_with("_test") && s.text.contains(&format!("pub fn {}(", s.name))
    }
}

/// A test's libtest name (D-170): its module path and name, `crate` dropped,
/// and the `integration::<file>` prefix of a `tests/*.rs` crate dropped.
pub fn libtest_name(ix: &Index, i: usize) -> String {
    let s = &ix.syms[i];
    let prefix = libtest_prefix(ix, &s.module);
    if prefix.is_empty() { s.name.clone() } else { format!("{prefix}::{}", s.name) }
}

fn libtest_prefix(ix: &Index, module: &str) -> String {
    if module == "crate" {
        return String::new();
    }
    if let Some(rest) = module.strip_prefix("integration::") {
        // A file that is its own `tests/*.rs` binary is that binary's crate
        // root; a helper module (`tests/common/mod.rs`) keeps its own name.
        let root = ix
            .modules
            .iter()
            .find(|m| m.path == module)
            .and_then(|m| m.file.as_deref())
            .and_then(|f| f.strip_prefix("tests/"))
            .and_then(|f| f.strip_suffix(".rs"))
            .is_some_and(|stem| stem == rest && !stem.contains('/'));
        return if root { String::new() } else { rest.to_string() };
    }
    module.to_string()
}

/// Everything a symbol can run or read: its closure over `uses`.
// a DFS per test; switch to SCC condensation if suites reach tens of thousands of tests.
pub fn closure(ix: &Index, i: usize) -> HashSet<usize> {
    let (mut seen, mut stack) = (HashSet::new(), vec![i]);
    while let Some(j) = stack.pop() {
        if seen.insert(j) {
            stack.extend(&ix.uses[j]);
        }
    }
    seen
}

/// What a test checks (D-36.3): what it references, also through helpers in
/// its own test module.
pub fn covered(ix: &Index, t: usize) -> HashSet<usize> {
    let module = &ix.syms[t].module;
    let (mut out, mut seen, mut stack) = (HashSet::new(), HashSet::new(), vec![t]);
    while let Some(j) = stack.pop() {
        if seen.insert(j) {
            for &u in &ix.uses[j] {
                if ix.syms[u].module == *module {
                    stack.push(u)
                } else {
                    out.insert(u);
                }
            }
        }
    }
    out
}

/// A test's reach, as a hash: the revision and module of everything it can
/// run, and the manifests of every project it can reach across.
pub fn hash(ix: &Index, reach: &HashSet<usize>, revisions: &[String], roots: &[&Path]) -> String {
    let mut parts: Vec<String> = reach.iter().map(|&j| format!("{} {} {}\n", ix.syms[j].id, ix.syms[j].module, revisions[j])).collect();
    parts.sort();
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    // A Rust project's tests also depend on its manifest, lockfile and build
    // script; a Gleam project's on its manifest and lockfile (D-170).
    let manifests: &[&str] = if ix.rust { &["Cargo.toml", "Cargo.lock", "build.rs"] } else { &["gleam.toml", "manifest.toml"] };
    for root in roots {
        for f in manifests {
            h.update(fs::read(root.join(f)).unwrap_or_default());
        }
    }
    hex(&h.finalize())
}

/// Changed source symbols as flow origins, each with the lines that changed.
/// `before` holds (id, text before; `None` if new).
pub fn origins(ix: &Index, before: &[(String, Option<String>)]) -> Vec<(usize, Vec<usize>)> {
    flow::origins(ix, before)
}

/// Every symbol (tests too) that reads one of `fields`.
pub fn readers(ix: &Index, fields: &[(usize, String)]) -> HashSet<usize> {
    flow::readers(ix, fields)
}

/// `refs <module>.<Type>.<field>`: the code that reads one field.
pub fn field_refs(ix: &Index, ty: usize, field: &str) -> String {
    flow::field_refs(ix, ty, field)
}

/// Where changed data goes (D-44). From each changed symbol's changed lines,
/// taint follows `let` bindings within a function, then: callers of a tainted
/// function, at the call (Gleam is pure, so a call's result reaches the
/// caller's), readers of the record fields it writes, at the read, and the
/// field writes of helpers called with a tainted argument (only those, so a
/// shared helper doesn't pull in all its callers). The analysis stops where
/// the data stops; only displays are cut.
pub fn flow(ix: &Index, origins: &[(usize, Vec<usize>)]) -> Impact {
    flow::gleam_flow(ix, origins)
}

/// `pricing.quote` for `pilot/pricing.quote`.
pub fn short(ix: &Index, i: usize) -> String {
    suites::short(ix, i)
}

/// Declared commands that are test suites outside `gleam test` (`checks` in
/// solace.toml, D-44): (name, reach; `None` when it checks everything). For
/// `gleam run -m <module>`, the module's `main` is an entry point.
/// A declared command that tests: its name, and what it reaches (`None`: everything).
pub type Suite = (String, Option<HashSet<usize>>);

pub fn suites(s: &Store, ix: &Index) -> Result<Vec<Suite>, Failure> {
    suites::suites(s, ix)
}

/// Every entry point the declared suites name, resolved or not, as (suite,
/// entry): `extract` lists those that stop resolving (`suites` skips them).
pub fn entries(s: &Store) -> Result<Vec<(String, String)>, Failure> {
    suites::entries(s)
}

/// The suites whose reach holds one of `syms`, as `e2e (`run e2e`)`.
pub fn reached(suites: &[Suite], syms: &[usize]) -> Vec<String> {
    suites::reached(suites, syms)
}

/// The uses (not the changed symbols): certain ones nearest first, those
/// without a test of their own first within a step, then the possible ones
/// (run 20: these crowded out the callers whose tests reach the change):
/// (use, its tests).
pub fn ranked<'a>(ix: &Index, impact: &'a Impact) -> Vec<(&'a Use, Vec<usize>)> {
    suites::ranked(ix, impact)
}

/// The submit reply's note: at most three lines (D-44).
pub fn note(ix: &Index, impact: &Impact, suites: &[String]) -> Vec<String> {
    suites::note(ix, impact, suites)
}

/// `impact`: every use, nearest first, within `budget`.
pub fn report(ix: &Index, impact: &Impact, suites: &[String], budget: usize) -> String {
    suites::report(ix, impact, suites, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    /// A test's reach hash covers the revision and module of everything it
    /// reaches, and the manifests: a header alone is not part of it.
    #[test]
    fn the_reach_hash_follows_revisions_modules_and_manifests() {
        let store = project(&[("src/calc.gleam", "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
        let source = "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n";
        let ix = crate::index::test_index(&[("src", "calc", source)]);
        let reach: HashSet<usize> = [0].into();
        let roots = [store.root.as_path()];
        let base = hash(&ix, &reach, &["r1".to_string()], &roots);
        assert_eq!(base, hash(&ix, &reach, &["r1".to_string()], &roots), "the same inputs give the same hash");
        assert_ne!(base, hash(&ix, &reach, &["r2".to_string()], &roots), "a revision is part of it");
        let mut moved = crate::index::test_index(&[("src", "calc", source)]);
        moved.syms[0].module = "other".to_string();
        assert_ne!(base, hash(&moved, &reach, &["r1".to_string()], &roots), "a module is part of it");
        let headed = crate::index::test_index(&[("src", "calc", "import gleam/int\n\npub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n")]);
        assert_eq!(base, hash(&headed, &reach, &["r1".to_string()], &roots), "a header alone is not part of it");
        fs::write(store.root.join("gleam.toml"), "name = \"other\"\n").unwrap();
        assert_ne!(base, hash(&ix, &reach, &["r1".to_string()], &roots), "the manifest is part of it");
    }

    #[test]
    fn coverage_and_test_symbols() {
        let ix = crate::index::test_index(&[
            (
                "src",
                "app",
                concat!(
                    "import shellout\n\npub fn main() {\n  step()\n}\n\n",
                    "fn step() {\n  helper()\n}\n\nfn helper() {\n  1\n}\n\n",
                    "pub fn run() {\n  shellout.command(\"ls\", [], \".\", [])\n}\n\n",
                    "@external(erlang, \"erlang\", \"abs\")\npub fn abs(x: Int) -> Int"
                ),
            ),
            (
                "test",
                "app_test",
                concat!(
                    "import app\n\npub fn main_test() {\n  assert app.main() == 1\n}\n\n",
                    "pub fn run_test() {\n  check(app.run())\n}\n\n",
                    "fn check(x) {\n  app.abs(x)\n}"
                ),
            ),
        ]);
        let id = |n: &str| ix.syms.iter().position(|s| s.name == n).unwrap();
        let main_test = id("main_test");
        assert!(closure(&ix, main_test).contains(&id("helper")));
        // calling `main` doesn't make a helper two steps away tested
        assert!(covered(&ix, main_test).contains(&id("main")) && !covered(&ix, main_test).contains(&id("helper")));
        // helpers in the test's own module count
        assert!(covered(&ix, id("run_test")).contains(&id("abs")));
        assert_eq!(test_symbols(&ix), [main_test, id("run_test")]);
    }
}
