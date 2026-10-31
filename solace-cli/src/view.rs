use super::*;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use serde_json::json;
use std::env;
use std::collections::HashMap;
mod graph;
mod review;
mod api;
pub(crate) use graph::direct_effects as graph_effects;
use crate::view::graph::graph_index;
use crate::view::review::changed_here;
use crate::view::review::task_here;
use crate::view::review::Version;
use crate::view::review::version;
use crate::view::review::task_reach;
use crate::view::review::review_graphs;
use crate::view::review::run_tests;
pub fn graph(s: &Store) -> Result<Value, Failure> {
    let ix = s.index()?;
    let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
    let mut changed = HashSet::new();
    let w = board::flow(&s.root)?;
    let top = scope::top(&s.root);
    for (_, t) in scope::tasks_in(&top, &scope::dir(&top, &s.root))? {
        let t = changed_here(s, t);
        if w["columns"].as_array().unwrap().iter().any(|c| c["id"] == t["column"] && c["final"] == true) {
            continue;
        }
        for c in t["changes"].as_array().unwrap_or(&vec![]) {
            if let Some(e) = c["entity"].as_str() {
                changed.insert(e.to_string());
            }
        }
    }
    let mut graph = graph_index(&ix, &files, &changed);
    let name = Some(workspace::name(&s.root))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| s.root.file_name().unwrap_or_default().to_string_lossy().into_owned());
    graph["project"] = json!({"name": name, "fingerprint": board::fingerprint(s)?});
    Ok(graph)
}

pub fn review(s: &Store, task: &Value) -> Result<Value, Failure> {
    let tid = task["id"].as_str().unwrap();
    let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
    let (old, now) = (version(s, Version::Before(tid))?, version(s, Version::Current)?);
    let before = graph_index(&old, &files, &HashSet::new());
    let after = graph_index(&now, &files, &HashSet::new());
    let reach = task_reach(s, &old, &now, task)?;
    let mut t = task.clone();
    let base = board::fingerprint(s)?;
    if t["review_tests"]["base"] != base {
        t["review_tests"] = Value::Null;
    }
    Ok(review_graphs(&before, &after, &t, &reach, &lockdown::settings(&s.root)?, &s.root))
}
/// `%xx` escapes in a URL path decoded; invalid UTF-8 replaced.
pub fn percent_decode(s: &str) -> String {
    api::percent_decode(s)
}

pub fn api(s: &Store, method: &str, parts: &[&str], query: &HashMap<String, String>, body: &Value) -> Result<Value, Failure> {
    api::route(s, method, parts, query, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;
    #[test]
    fn c4_files_match_cli_listing() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", "unwatched=[\"priv/gen/\"]\n"), ("priv/a", "hello")])
            .import();
        let root = &p.root;
        let settings = lockdown::settings(root).unwrap();
        let files = lockdown::tree_files(root, &settings).unwrap();
        let mut listing = String::new();
        lockdown::dispatch(&board::open(root).unwrap(), &settings, &["files"], &mut || Ok(String::new()), None, 1000)
            .unwrap()
            .map(|s| listing = s)
            .unwrap();
        for (name, tier) in files {
            // A file's tier may be followed by its line count or size (D-119).
            let (whole, counted) = (format!("{name} [{tier}]"), format!("{name} [{tier}; "));
            assert!(listing.lines().any(|l| l == whole || l.starts_with(&counted)), "{listing}");
        }
    }
    #[test]
    fn graph_api_routes() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("gleam.toml", "name=\"fixture\"\nversion=\"1.0.0\"\n")])
            .import();
        let root = p.root.clone();
        let g = board::api(&root, "GET", "/graph", json!({})).unwrap();
        assert_eq!(g["project"]["name"], "fixture");
        assert_eq!(g["project"]["fingerprint"].as_str().unwrap().len(), 64);
        let expected = board::fingerprint(&board::open(&root).unwrap()).unwrap();
        assert_eq!(g["project"]["fingerprint"], expected);
        let id = g["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "entry").unwrap()["id"].as_str().unwrap();
        let node = board::api(&root, "GET", &format!("/nodes/{id}?source=1"), json!({})).unwrap();
        assert!(node["source"].as_str().unwrap().contains("pub fn main"));
        // With a task, the source comes with its line diff from before the task.
        let store = board::open(&root).unwrap();
        let mut old = board::snapshot(&store).unwrap()[format!("symbol:{id}")].clone();
        old[4] = json!("pub fn main() {\n  0\n}");
        store.db.execute("INSERT INTO board_before VALUES ('t1',?1,?2)", [format!("symbol:{id}"), old.to_string()]).unwrap();
        let diff = board::api(&root, "GET", &format!("/nodes/{id}?source=1&task=t1"), json!({})).unwrap();
        assert_eq!(diff["diff"], json!([["-", "pub fn main() {"], ["-", "  0"], ["-", "}"], ["+", "pub fn main() { 1 }"]]));
        assert!(board::api(&root, "GET", "/nodes/fn:app.nope?source=1&task=t1", json!({})).is_err());
        // Files: listed ones only, read-only text; graph, internal and unlisted paths are refused.
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/read me.md"), "hello\n").unwrap();
        fs::write(root.join("docs/logo.bin"), b"\0\x01").unwrap();
        let f = board::api(&root, "GET", "/files/docs/read%20me.md", json!({})).unwrap();
        assert_eq!((f["text"].as_str(), f["category"].as_str()), (Some("hello\n"), Some("watched")));
        assert!(board::api(&root, "GET", "/files/docs/logo.bin", json!({})).unwrap()["text"].is_null());
        for bad in ["/files/src/app.gleam", "/files/.solace/store.db", "/files/../etc/passwd", "/files/docs/"] {
            assert!(board::api(&root, "GET", bad, json!({})).is_err(), "{bad}");
        }
        fs::write(root.join("gleam.toml"), "version=\"1.0.0\"\n").unwrap();
        let fallback = board::api(&root, "GET", "/graph", json!({})).unwrap();
        assert_eq!(fallback["project"]["name"], root.file_name().unwrap().to_str().unwrap());
        assert!(board::api(&root, "GET", "/graph?task=no-such-task", json!({})).unwrap_err().0.contains("E7001"));
    }
    #[test]
    fn c3_review_shows_a_new_reachable_write() {
        let before =
            graph_index(&crate::index::test_index(&[("src", "app", "pub fn main() { work() }\nfn work() { 1 }")]), &[], &HashSet::new());
        let after = graph_index(
            &crate::index::test_index(&[(
                "src",
                "app",
                "import simplifile\npub fn main() { work() }\nfn work() { simplifile.write(\"a\", \"b\") }",
            )]),
            &[],
            &HashSet::new(),
        );
        let task = json!({"id":"t","changes":[{"entity":"symbol:fn:app.work"}],"review_tests":null});
        let r = review_graphs(
            &before,
            &after,
            &task,
            &json!({"tests":[],"untested":[{"id":"fn:app.main"}]}),
            &"".parse().unwrap(),
            Path::new("/tmp"),
        );
        assert_eq!(r["effects"]["added"], json!(["effect:writes files"]));
        assert!(r["summary"].as_str().unwrap().contains("1 untested uses"), "{}", r["summary"]);
        assert!(
            r["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["id"] == "fn:app.main" && e["effects_added"] == json!(["effect:writes files"]))
        );
    }
    /// Real `gleam test` output, colors and all: a failed assert, a `todo`
    /// and a passing test.
    #[test]
    fn gleam_test_output_states() {
        let failed = gleam::failures(include_str!("../tests/golden/gleam-test-failures.txt"));
        assert_eq!(failed["app_test.wrong_test"].0, "wrong");
        assert!(failed["app_test.wrong_test"].1.contains("left: 2\nright: 3"), "{failed:?}");
        assert_eq!(failed["app_test.todo_test"].0, "unfinished");
        assert!(!failed.contains_key("app_test.passing_test"));
    }
    /// A file shown up to 1 MiB: a cut through a multi-byte character at the
    /// end is text, not binary.
    #[test]
    fn a_cut_through_a_multibyte_character_is_still_text() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", "watch_limit = 1100000\nunwatched = []\n")])
            .import();
        let root = p.root.clone();
        fs::create_dir_all(root.join("docs")).unwrap();
        let mut bytes = vec![b'a'; (1 << 20) - 1];
        bytes.extend("é".bytes());
        fs::write(root.join("docs/big.txt"), &bytes).unwrap();
        let f = board::api(&root, "GET", "/files/docs/big.txt", json!({})).unwrap();
        assert_eq!(f["size"], json!((1 << 20) + 1));
        assert_eq!(f["truncated"], json!(true));
        assert_eq!(f["text"].as_str().unwrap().len(), (1 << 20) - 1, "the cut drops the partial character");
    }

    /// `/nodes/{id}` without `source`: the neighbours from the graph's edges,
    /// and at most six examples from the tests that call it.
    #[test]
    fn node_details_list_neighbours_and_examples() {
        let p = Scratch::new()
            .with(&[
                ("src/app.gleam", "pub fn main(n: Int) -> Int {\n  helper(n)\n}\n\nfn helper(n: Int) -> Int {\n  n\n}\n"),
                (
                    "test/app_test.gleam",
                    concat!(
                        "import app\n\npub fn main_test() {\n",
                        "  assert app.main(1) == 1\n  assert app.main(2) == 2\n  assert app.main(3) == 3\n",
                        "  assert app.main(4) == 4\n  assert app.main(5) == 5\n  assert app.main(6) == 6\n",
                        "  assert app.main(7) == 7\n}\n",
                    ),
                ),
                ("gleam.toml", "name=\"fixture\"\nversion=\"1.0.0\"\n"),
            ])
            .import();
        let root = p.root.clone();
        let g = board::api(&root, "GET", "/graph", json!({})).unwrap();
        let id_of =
            |name: &str| g["nodes"].as_array().unwrap().iter().find(|n| n["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
        let node = board::api(&root, "GET", &format!("/nodes/{}", id_of("main")), json!({})).unwrap();
        assert_eq!(node["callees"], json!([id_of("helper")]));
        assert_eq!(node["callers"], json!([]));
        assert_eq!(node["tests"], json!([id_of("main_test")]));
        let examples = node["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 6, "seven checks, six shown: {examples:?}");
        assert_eq!(examples[0]["call"], json!("app.main(1)"));
        assert!(examples.iter().all(|c| c["call"].as_str().unwrap().contains("app.main(")));
    }

    /// A review of a signature change: the entry and the signature are listed,
    /// an unfinished test shows, and a check whose expected value moved carries
    /// `was`.
    #[test]
    fn review_graphs_show_signatures_unfinished_and_changed_checks() {
        let before = graph_index(
            &crate::index::test_index(&[
                ("src", "app", "pub fn main(n: Int) -> Int {\n  n\n}"),
                ("test", "app_test", "import app\n\npub fn main_test() {\n  assert app.main(1) == 1\n}"),
            ]),
            &[],
            &HashSet::new(),
        );
        let after = graph_index(
            &crate::index::test_index(&[
                ("src", "app", "pub fn main(n: Int, k: Int) -> Int {\n  n\n}"),
                ("test", "app_test", "import app\n\npub fn main_test() {\n  assert app.main(1) == 2\n}"),
            ]),
            &[],
            &HashSet::new(),
        );
        let task = json!({
            "id": "t",
            "changes": [{"entity": "symbol:fn:app.main"}],
            "review_tests": {
                "base": "x",
                "state": "done",
                "results": [{
                    "id": "fn:app_test.main_test",
                    "name": "main_test",
                    "result": "unfinished",
                    "checks": [{"call": "app.main(1)", "expected": "2"}],
                }],
            },
        });
        let r = review_graphs(
            &before,
            &after,
            &task,
            &json!({"tests":["fn:app_test.main_test"],"untested":[],"suites":[],"leaves":[]}),
            &"".parse().unwrap(),
            Path::new("/tmp"),
        );
        assert_eq!(
            r["signatures"],
            json!([{"id":"fn:app.main","before":"pub fn main(n: Int) -> Int","after":"pub fn main(n: Int, k: Int) -> Int"}])
        );
        assert_eq!(r["tests"].as_array().unwrap().len(), 1, "{:?}", r["tests"]);
        assert_eq!(r["tests"][0]["result"], json!("unfinished"));
        assert_eq!(r["tests"][0]["checks"][0]["was"], json!("1"), "the check's old expected value");
        assert!(r["summary"].as_str().unwrap().contains("1 signature changed, 1 test unfinished, 0 untested uses"), "{}", r["summary"]);
        assert!(!r["summary"].as_str().unwrap().contains("not run yet"), "{}", r["summary"]);
    }

    #[test]
    fn test_failure_blocks() {
        let f = gleam::failures(
            "todo test/app_test.gleam:3\n test: app_test.todo_test\nassert test/app_test.gleam:6\n test: app_test.wrong_test\n",
        );
        assert_eq!(f["app_test.todo_test"].0, "unfinished");
        assert_eq!(f["app_test.wrong_test"].0, "wrong");
        assert!(!f.contains_key("app_test.ok_test"));
        assert_eq!(f["app_test.wrong_test"].1, "assert test/app_test.gleam:6\ntest: app_test.wrong_test");
    }

    #[test]
    fn review_api_returns_only_the_summary() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("gleam.toml", "name=\"fixture\"\nversion=\"1.0.0\"\n")])
            .import();
        let root = p.root.clone();
        let t = board::api(&root, "POST", "/tasks", json!({"title": "Review"})).unwrap();
        let id = t["id"].as_str().unwrap();
        let graph = board::api(&root, "GET", &format!("/graph?task={id}"), json!({})).unwrap();
        let review = board::api(&root, "GET", &format!("/graph/review?task={id}"), json!({})).unwrap();
        assert_eq!(review["summary"], graph["review"]["summary"]);
        assert_eq!(review.as_object().unwrap().len(), 1);
        assert!(board::api(&root, "GET", "/graph/review", json!({})).is_err());
    }
}
