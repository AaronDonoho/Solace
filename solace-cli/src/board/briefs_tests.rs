use super::*;
use serde_json::json;
#[cfg(test)] use crate::board::briefs::VISION_CAP;
#[cfg(test)] use crate::board::briefs::FEATURE_CAP;
use crate::tasks;
use crate::testing;
use serde_json::Value;
use std::fs;
use crate::board::task_model::open;
use crate::board::task_model::store_dir;
use crate::board::server::api;
use crate::board::briefs::continuation_brief;
use crate::board::briefs::brief;
use crate::board::briefs::WAITING;
use crate::board::briefs::waiting_on;
use crate::board::briefs::START;
use crate::board::briefs::HELD_START;
use crate::board::briefs::TECH_DEBT_ASK;
use crate::board::briefs::FINDINGS_ASK;
use crate::board::briefs::needed_by;
use crate::board::briefs::WORK_TECH_DEBT_ASK;
use crate::board::briefs::note_fingerprint;
use crate::board::briefs::remember;
use crate::board::task_model::all_tasks;
use crate::scope;
use crate::cli::with_root;
use crate::cli::run_options;
use crate::board::runs::with_context;

/// D-186: an investigation states outcomes and constraints, not a plan that
/// assigns edits, submission calls or checks.
#[test]
fn investigate_briefs_report_requirements_not_edits() {
    let code = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [{"entity": "symbol:x", "name": "calc.add", "run": "r1"}],
        "test_commands": [{"project": "a", "commands": ["rust-acceptance"]}],
    });
    let settings = json!({
        "id": "t2",
        "title": "Tune the build",
        "body": "Allow the new test command.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [{"entity": "file:solace.toml", "name": "solace.toml", "run": "r2"}],
    });
    for (label, task, named) in [("code change", &code, "calc.add"), ("settings change", &settings, "solace.toml")] {
        let prompt = brief(task, "investigate");
        for required in [
            "Investigate this task",
            "Report the required behavior, constraints, acceptance criteria and risks",
            "Name the symbols, modules and non-source files that need attention",
            "anything you didn't check",
            named,
        ] {
            assert!(prompt.contains(required), "{label}: missing {required}\n{prompt}");
        }
        assert!(prompt.contains(&format!("Task changes:\n- {named}")), "{label}: missing the change it names\n{prompt}");
        for directive in [
            "`submit` then `test`",
            "`write` then `run",
            "`test --all`",
            "settings proposal",
            "To change `solace.toml` or `AGENTS.md`, `write` it alone",
        ] {
            assert!(!prompt.contains(directive), "{label}: unexpected {directive}\n{prompt}");
        }
        assert!(prompt.contains("This run is read-only"), "{label}: missing the read-only notice\n{prompt}");
        assert!(
            prompt.contains("Probe or run tests only when needed to more thoroughly define the issue."),
            "{label}: missing the limit on probes and tests\n{prompt}"
        );
        assert!(!prompt.contains("Use those to verify (for example test suites)"), "{label}\n{prompt}");
    }
    let review = brief(&code, "review");
    assert!(review.contains("Use those to verify (for example test suites)"), "{review}");
    assert!(!review.contains("Probe or run tests only when needed"), "{review}");
    let work = brief(&code, "work");
    for owned in [
        "`submit` then `test`",
        "`write` then `run <name>`",
        "`test --all`",
        "To change `solace.toml` or `AGENTS.md`, `write` it alone, with the why in `reason`.",
    ] {
        assert!(work.contains(owned), "work: missing {owned}\n{work}");
    }
}

/// D-91: investigate, work and discuss briefs carry the vision, cut at a line
/// end within the cap; a review's doesn't, and no vision adds nothing.
#[test]
fn a_brief_carries_the_vision_up_to_the_cap() {
    let root = testing::Scratch::new();
    assert_eq!(vision_section(&root, "work"), "");
    let line = format!("{}\n", "v".repeat(99));
    let long = line.repeat(12);
    fs::create_dir_all(root.join(".solace/strategy")).unwrap();
    fs::write(root.join(".solace/strategy/vision.md"), &long).unwrap();
    for kind in ["investigate", "work", "discuss"] {
        let s = vision_section(&root, kind);
        assert!(s.starts_with("\nVision ("), "{kind}: {s}");
        assert_eq!(s.matches(&"v".repeat(99)).count(), 10, "{kind}: ten whole lines fit in {VISION_CAP} characters");
        assert!(s.contains("(cut short;"), "{kind}");
    }
    assert_eq!(vision_section(&root, "review"), "");
    fs::write(root.join(".solace/strategy/vision.md"), "For teams.\n").unwrap();
    assert!(vision_section(&root, "work").ends_with(":\nFor teams.\n"));
}

/// D-138: a task's feature section names the feature and carries the latest
/// of its investigation report, assignment note and review feedback, cut at
/// a line end within the cap; no feature, no section.
#[test]
fn a_brief_carries_the_features_latest_direction() {
    let top = testing::Scratch::new();
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search","body":"Everything, everywhere.","state":"active"}))
        .map_err(|e| e.1.0)
        .unwrap();
    let runs = json!([{"id":"i1","kind":"investigate"},{"id":"w1","kind":"work"},{"id":"r1","kind":"review"},{"id":"d1","kind":"discuss"}]);
    let mut t = json!({"id":"t","runs":runs,"notes":[]});
    assert_eq!(feature_section(&top, &t), "");
    t["feature"] = json!("gone");
    assert_eq!(feature_section(&top, &t), "");
    t["feature"] = f["id"].clone();
    assert_eq!(
        feature_section(&top, &t),
        "\nFeature (the feature this task is part of; its state and body, then its latest direction):\n- Search\n- state: active\n\nEverything, everywhere.\n"
    );
    let note = |author: &str, run: Value, text: &str| json!({"author":author,"run":run,"text":text});
    t["notes"] = json!([note("agent", json!("i1"), "plan"), note("solace", Value::Null, "exited")]);
    assert!(feature_section(&top, &t).ends_with("\n- investigation report (run i1): plan\n"));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, "go"));
    t["notes"].as_array_mut().unwrap().push(note("agent", json!("w1"), "done"));
    assert!(feature_section(&top, &t).ends_with("\n- assignment: go\n"));
    t["notes"].as_array_mut().unwrap().push(note("agent", json!("r1"), "looks good"));
    t["notes"].as_array_mut().unwrap().push(note("human", json!("d1"), "chat"));
    assert!(feature_section(&top, &t).ends_with("\n- review feedback (run r1): looks good\n"));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, "  "));
    assert!(feature_section(&top, &t).ends_with("\n- review feedback (run r1): looks good\n"), "an empty note is skipped");
    let line = format!("{}\n", "n".repeat(99));
    t["notes"].as_array_mut().unwrap().push(note("human", Value::Null, &line.repeat(12)));
    let s = feature_section(&top, &t);
    assert_eq!(s.matches(&"n".repeat(99)).count(), 10, "ten whole lines fit in {FEATURE_CAP} characters");
    assert!(s.ends_with("\n(cut short; the whole note is in the task's notes on the board)\n"), "{s}");
}

#[test]
fn note_images_are_checked_numbered_and_encoded() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    assert_eq!(image_kind(b"<svg onload=alert(1)>"), None);
    assert_eq!(image_type("0123456789abcdef.png"), Some("image/png"));
    for bad in ["../x.png", "0123456789abcdef.svg", "x.png"] {
        assert_eq!(image_type(bad), None);
    }
    let a = "/tasks/t1/files/0123456789abcdef.png";
    let mut found = vec![];
    let text = note_images(&format!("see ![one]({a}) and ![x](http://e/x.png) ![again]({a}) ![broken"), "/tasks/t1/files/", &mut found);
    assert_eq!(text, "see [image 1] and ![x](http://e/x.png) [image 1] ![broken");
    assert_eq!(found, ["0123456789abcdef.png"]);
}

/// D-27: a run's brief lists the skills of the project it works in — name,
/// description and the path to `read` — for every kind; a project with no
/// skills has no Skills section, and the brief carries no skill contents.
#[test]
fn a_brief_lists_the_skills_of_its_project() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("skills/design/SKILL.md", "---\nname: design\ndescription: Layout and color.\n---\n# Design\n\nPick a palette.\n"),
        ("skills/voice.md", "Speak plainly\n"),
    ]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    for kind in ["work", "investigate", "discuss", "review"] {
        let run = json!({"id":"r1","kind":kind});
        let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false).0;
        assert!(brief.contains("- design: Layout and color. (`read skills/design/SKILL.md`)"), "{kind}: {brief}");
        assert!(brief.contains("- voice: Speak plainly (`read skills/voice.md`)"), "{kind}: {brief}");
        assert!(brief.contains("when the task matches a skill's description"), "{kind}: {brief}");
        assert!(brief.contains("After the call that ends the run returns, stop: no more calls"), "{kind}: {brief}");
        assert!(!brief.contains("finishing tool"), "{kind}: a discussion's first note is not its end: {brief}");
        assert!(!brief.contains("Pick a palette"), "{kind}: the brief carries no skill contents");
    }
    let q = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let u = open(&q.root).unwrap();
    let t2 = api(&u.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let brief = continuation_brief(&t2, &json!({"id":"r1","kind":"work"}), false, &q.root, &q.root, false).0;
    assert!(!brief.contains("Skills ("), "{brief}");
}

/// D-151: a fresh work or investigate brief holds the session start reply in
/// place of the line asking for it, before the stop line; a resumed brief, a
/// review's and a discussion's don't, and a project of plain files gets its own reply.
#[test]
fn a_fresh_brief_holds_the_session_start_reply() {
    let p = testing::project(&[("src/app.gleam", "/// Totals an order.\npub fn total() -> Int {\n  1\n}\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"Fix the order total","body":"app.total is off by one"})).unwrap();
    let brief = |kind: &str, resumed: bool| continuation_brief(&t, &json!({"id":"r1","kind":kind}), resumed, &p.root, &p.root, false).0;
    let (reply, next) = (
        "\nSession start (the reply to `session start --task`):\n",
        "\nNext: read what the change needs in one call, one command per line: `inspect app.total",
    );
    for kind in ["work", "investigate"] {
        let b = brief(kind, false);
        assert!(b.contains(reply) && b.contains("Symbols most likely relevant to the task") && b.contains("-- app.total"), "{kind}: {b}");
        assert!(!b.contains(START) && b.contains(HELD_START), "{kind}: {b}");
        let (at, stop) = (b.find(next).expect(&b), b.find("After the call that ends the run returns, stop").unwrap());
        assert!(b.find(reply).unwrap() < at && at < stop, "{kind}: the reply's first call comes just before the stop line: {b}");
        assert!(!b.contains("Feature (the feature") && !b.contains("Depends on"), "{kind}: {b}");
        assert!(!b.lines().any(|line| line.starts_with("`task peek")), "{kind}: {b}");
    }
    for kind in ["review", "discuss"] {
        let b = brief(kind, false);
        assert!(!b.contains(reply) && !b.contains(HELD_START), "{kind}: {b}");
    }
    let dep = api(&s.root, "POST", "/tasks", json!({"title":"Dependency"})).unwrap();
    let mut needs = api(&s.root, "POST", "/tasks", json!({"title":"Needs one"})).unwrap();
    needs["dependsOn"] = json!([dep["id"]]);
    tasks::put(&s, &needs).unwrap();
    let b = continuation_brief(&needs, &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false).0;
    assert!(b.lines().any(|line| line == "`task peek <id>` reads a listed task."), "{b}");
    assert!(!b.contains("`task peek feature` reads the feature"), "{b}");
    let f = tasks::strategy_api(&scope::top(&p.root), "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
    let mut featured = api(&s.root, "POST", "/tasks", json!({"title":"Featured"})).unwrap();
    featured["feature"] = json!(f["id"]);
    tasks::put(&s, &featured).unwrap();
    let b = continuation_brief(&featured, &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false).0;
    assert!(b.lines().any(|line| line == "`task peek feature` reads the feature."), "{b}");
    assert!(!b.lines().any(|line| line.starts_with("`task peek <id>`")), "{b}");
    let resumed = brief("work", true);
    assert!(
        resumed.contains("Use the continuation context below.") && !resumed.contains(reply) && !resumed.contains(HELD_START),
        "{resumed}"
    );
    fs::remove_file(p.root.join("gleam.toml")).unwrap();
    let plain = brief("work", false);
    assert!(
        plain.contains(reply) && plain.contains("This project has plain files") && plain.contains("`grep <text> --in <path>`"),
        "{plain}"
    );
    // A folder with no store keeps the line asking for the call.
    let bare = testing::Scratch::new();
    let b = continuation_brief(&t, &json!({"id":"r1","kind":"work"}), false, &p.root, &bare, false).0;
    assert!(b.contains(START) && !b.contains(reply), "{b}");
}

/// `session remember` is for a task run: a discussion's brief holds its
/// context, so it is refused, and a run that isn't there is too.
#[test]
fn session_remember_is_refused_in_a_discussion_and_for_an_unknown_run() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["runs"] = json!([{"id":"r1","kind":"discuss","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let e = remember(&p.root, tid, "r1", 1000).unwrap_err();
    assert!(e.0.starts_with("E7010"), "{}", e.0);
    let e = remember(&p.root, tid, "r9", 1000).unwrap_err();
    assert!(e.0.starts_with("E7003"), "{}", e.0);
}

/// A resumed brief drops notes the conversation already saw; `session
/// remember` still gives them, so a compacted run can read its task again.
#[test]
fn session_remember_keeps_notes_a_resumed_brief_filters_out() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["notes"] = json!([{"author":"agent","text":"The investigator's plan."}]);
    t["conversation"] = json!({"notes_seen":[note_fingerprint(&t["notes"][0])]});
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let run = json!({"id":"r1","kind":"work","state":"running"});
    let brief = continuation_brief(&t, &run, true, &p.root, &p.root, false).0;
    assert!(!brief.contains("The investigator's plan."), "{brief}");
    let out = remember(&p.root, tid, "r1", 10_000).unwrap();
    assert!(out.contains("The investigator's plan."), "{out}");
}

/// D-110: a brief names the task's unfinished dependencies, from any project,
/// with advice; with none, or only finished ones (canceled counts), it says nothing.
#[test]
fn briefs_name_unfinished_dependencies() {
    // `lib` is listed as a project so a dependency homed there keeps its home:
    // a home that is no project is re-homed on load (D-160).
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"lib\"]\n"),
        ("lib/.solace/keep", ""),
    ]);
    let top = scope::top(&p.root);
    let home = scope::dir(&top, &p.root);
    let dir = store_dir(&top);
    fs::create_dir_all(&dir).unwrap();
    let put = |id: &str, home: &str, column: &str, deps: &[&str]| {
        let t = json!({"id":id,"title":format!("Task {id}"),"body":"","home":home,"column":column,"rank":1,"dependsOn":deps,
            "runs":[],"notes":[],"questions":[],"links":[],"closedTime":null,"archivedTime":null});
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("a", "lib", "assigned", &[]);
    put("b", &home, "complete", &[]);
    put("c", &home, "cancelled", &[]);
    put("t", &home, "planning", &["a", "b", "c"]);
    put("f", &home, "planning", &["b", "c"]);
    put("n", &home, "backlog", &[]);
    let w = waiting_on(&top, "t");
    for part in [
        "Depends on:\n",
        "task:a Task a (Assigned, in lib) — unfinished",
        "task:b Task b (Complete)\n",
        "task:c Task c (Canceled)\n",
        WAITING,
    ] {
        assert!(w.contains(part), "{part} missing from:\n{w}");
    }
    // Finished dependencies are listed without the warning.
    let f_deps = waiting_on(&top, "f");
    assert!(f_deps.contains("Depends on:\n") && f_deps.contains("task:b Task b (Complete)") && !f_deps.contains(WAITING), "{f_deps}");
    assert_eq!(waiting_on(&top, "n"), String::new());
    // The run prompt carries it; one with only finished dependencies still lists them.
    let task = |id: &str| all_tasks(&top).unwrap().into_iter().find(|t| t["id"] == id).unwrap();
    let run = json!({"id":"r1","kind":"work"});
    assert!(continuation_brief(&task("t"), &run, false, &p.root, &p.root, false).0.contains(&w));
    let finished = continuation_brief(&task("f"), &run, false, &p.root, &p.root, false).0;
    assert!(finished.contains("Depends on:") && finished.contains("task:b") && !finished.contains(WAITING), "{finished}");
    // A task brought back to the backlog counts as unfinished again.
    put("b", &home, "backlog", &[]);
    assert!(waiting_on(&top, "f").contains("task:b Task b (Backlog) — unfinished"));
}

#[test]
fn briefs_hold_what_the_agent_needs_as_text() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "rank": 3,
        "stamps": {},
        "links": [{"name": "spec", "url": "http://x"}],
        "notes": [{"author": "human", "text": "Keep it small", "time": 1, "run": null}],
        "questions": [{"id": "q", "text": "Syntax?", "options": ["a", "b"], "answer": "b"}],
        "runs": [{"id": "r1"}],
        "changes": [{"entity": "symbol:x", "name": "calc.add", "run": "r1"}],
        "test_commands": [{"project": "a", "commands": ["rust-acceptance", "spacing_parity"]}],
    });
    let b = brief(&t, "work");
    for part in [
        "Make the change",
        "`task done`, a summary in `text`",
        "Task: Add coolcalc\n\nA CLI calculator.\n",
        "- spec: http://x",
        "- human: Keep it small",
        "- Syntax?\n  Answer: b",
    ] {
        assert!(b.contains(part), "{part} missing from:\n{b}");
    }
    for noise in ["{", "r1", "rank"] {
        assert!(!b.contains(noise), "{noise} in:\n{b}");
    }
    // Symbol-held source is named without tying it to Gleam.
    assert!(b.contains("symbol-held source") && !b.contains("with Gleam code"), "{b}");
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind);
        for guidance in [
            "Write human-facing task messages in Markdown",
            "including the text sent to task tools",
            "relevant sections with short headings",
            "blank lines around headings and lists",
            "explain what you need and why it matters",
            "Keep messages short: lead with the answer or outcome",
            "give each option a short example of its outcome",
        ] {
            assert!(prompt.contains(guidance), "{kind}: missing {guidance}");
        }
        assert_eq!(prompt.contains("**Assessment: Looks good**"), kind == "review");
        assert_eq!(prompt.contains("**Assessment: Needs work**"), kind == "review");
    }
    // D-181: source files are projections; only a module, symbol or other file is named.
    for kind in ["work", "investigate", "review", "discuss"] {
        assert!(
            brief(&t, kind).contains(concat!(
                "Code held as symbols is read and cited by name (`lockdown.dispatch`, `checkout.pay`), ",
                "not by source path: source files are projections written by Solace.",
            )),
            "{kind}: missing the projection rule"
        );
    }
    let plan = brief(&t, "investigate");
    assert!(
        plan.contains("Report the required behavior, constraints, acceptance criteria and risks")
            && !plan.contains("names every module or symbol it changes"),
        "{plan}"
    );
    // D-119 point 6: check what is already answered before asking.
    for kind in ["work", "investigate"] {
        assert!(brief(&t, kind).contains("check the Notes so far and Questions below, if any"), "{kind}");
        // D-153: each option shows an example of its outcome.
        assert!(brief(&t, kind).contains("give each option a short example of the outcome the human would see"), "{kind}");
    }
    assert!(brief(&t, "investigate").contains("Investigate this task"));
    // D-147: a report never hands a file change to the human.
    assert!(
        brief(&t, "investigate")
            .contains("The human never edits files or runs commands; any needed change, a settings change included, is the worker's")
    );
    // D-156/D-186: an investigation reports requirements, not a plan for edits.
    assert!(
        brief(&t, "investigate").contains("anything you didn't check")
            && !brief(&t, "investigate").contains("A plan is the smallest complete change")
    );
    assert!(brief(&t, "work").contains("what you skipped and risks the human must know"));
    assert!(brief(&t, "review").contains("Flag code the task didn't need, places it missed"));
    for kind in ["work", "investigate", "discuss"] {
        assert!(!brief(&t, kind).contains("smallest complete change"), "{kind}");
    }
    for kind in ["work", "discuss"] {
        assert!(!brief(&t, kind).contains("Flag code the task didn't need"), "{kind}");
    }
    // Maintainability: each role gets the direction it acts on, and both share
    // the comment and UI rules.
    let work = brief(&t, "work");
    let review = brief(&t, "review");
    for kind in ["work", "review"] {
        assert!(
            brief(&t, kind).contains(
                "Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI."
            ),
            "{kind}: missing the comment and UI rules"
        );
    }
    for rule in [
        "Reuse existing code before adding new code, extracting common patterns",
        "small, stateless, self-documenting functions",
        "small and focused around one domain or component",
        "clear names for new symbols",
        "the module that owns its concern",
        "task:2414798176c466e4",
    ] {
        assert!(work.contains(rule), "work: missing {rule}");
        assert!(!review.contains(rule), "review: unexpected {rule}");
    }
    for rule in [
        "Flag duplication and large modules",
        "minor ones as tech debt",
        "cross-domain concerns in the same module",
        "excessively long functions and dead code",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
        assert!(!work.contains(rule), "work: unexpected {rule}");
    }
    // D-182: only a work run is told to run every test, once, after its last change.
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind);
        assert_eq!(
            prompt.contains("Before `task done`, after your last change, run `test --all` in each project with symbol-held source"),
            kind == "work",
            "{kind}"
        );
        assert_eq!(prompt.contains("and `run <name>` for: rust-acceptance, spacing_parity."), kind == "work", "{kind}");
    }
    assert!(!b.contains("the test commands its `solace.toml` declares"), "{b}");
    // A discussion keeps every note whole: no command reads a cut note's full text.
    let mut talk = t.clone();
    talk["notes"] = json!([
        {"author":"agent","text":"### Summary\n\nPlan A.\n\n### Details\n\nLong A.","run":null},
        {"author":"human","text":"### Mine\n\nKeep.\n\n### All\n\nOf it.","run":null},
        {"author":"agent","text":"### Summary\n\nPlan B.\n\n### Details\n\nLong B.","run":null}
    ]);
    let d = brief(&talk, "discuss");
    assert!(d.contains("Long A.") && d.contains("Keep.\n\n### All\n\nOf it.") && d.contains("Long B."), "{d}");
    let r = brief(&t, "review");
    assert!(
        r.contains(
            "For each concern, inspect every symbol it needs in one call (`inspect a b c --context`); split only when the reply is trimmed"
        ) && r
            .contains("A call is a turn, so batch the symbols one concern needs; start from the changed symbols the review context names")
            && !r.contains("small batches"),
        "{r}"
    );
    assert!(
        r.contains("Only a `task note` call records the feedback")
            && r.contains("don't claim it was recorded unless the reply says `Recorded.`"),
        "{r}"
    );
    for kind in ["work", "investigate", "discuss"] {
        assert!(!brief(&t, kind).contains("Only a `task note` call records the feedback"), "{kind}");
    }
    assert!(
        r.contains("Task changes:\n- calc.add")
            && r.contains("`task note`, the feedback in `text`")
            && r.contains("`format` are refused, and `run <name>` works only for commands marked `allowed_in_reviews`")
            && !r.contains("task done")
    );
    assert!(!b.contains("Projects:"), "one project needs no Projects section");
    assert!(review.contains(FINDINGS_ASK), "a review is asked for detailed findings");
    for rule in [
        "the heading is exactly `### Tech debt`",
        "each item is one `- ` bullet at the start of a line",
        "its details are lines indented under it",
        "no line after the heading starts with `#`",
        "no other heading is inside the section, and nothing follows it except a new heading",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
    }
    assert!(review.contains(TECH_DEBT_ASK), "a review is asked for tech debt");
    assert!(
        review.contains("Leave out items already in the work summary's `### Tech debt` section: they are filed already."),
        "a review is told not to repeat work tech debt"
    );
    assert!(work.contains(WORK_TECH_DEBT_ASK), "a work run is asked for its summary's tech debt");
    assert!(
        work.contains("ordinary friction, code-quality notes and restating risks are not tech debt"),
        "a work run is told what is not tech debt"
    );
    for kind in ["work", "investigate", "discuss"] {
        let prompt = brief(&t, kind);
        assert!(!prompt.contains(FINDINGS_ASK), "{kind}: unexpected findings detail");
        assert!(!prompt.contains(TECH_DEBT_ASK), "{kind}: unexpected tech debt ask");
    }
    // D-198: a review of a task with a feature is asked the same.
    let mut featured = t.clone();
    featured["feature"] = json!("f1");
    assert!(brief(&featured, "review").contains(TECH_DEBT_ASK), "a featured review is asked for tech debt");
    // A task across projects (D-61) says where the agent works and how it reaches the rest.
    let mut wide = t.clone();
    wide["scope"] = json!(["a", "web"]);
    wide["home"] = json!("a");
    wide["changes"][0]["project"] = json!("web");
    wide["test_commands"] = json!([
        {"project":"a","commands":["rust-acceptance"]},
        {"project":"web","commands":["spacing_parity"]}
    ]);
    let w = brief(&wide, "work");
    assert!(w.contains("covers the projects a, web. You work in a; for another of them, add `--project <folder>`"), "{w}");
    assert!(w.contains("Task changes:\n- web: calc.add"), "{w}");
    assert!(w.contains("and `run <name>` for: a: rust-acceptance; web: spacing_parity."), "{w}");
}

/// D-182: a work brief names the commands each project marks `test = true`,
/// sorted; a project with none adds no `run` clause.
#[test]
fn briefs_name_the_test_commands_each_project_marks() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        (
            "solace.toml",
            concat!(
                "[commands]\nunit = { run = [\"gleam\", \"test\"] }\n",
                "spacing_parity = { run = [\"true\"], test = true }\n",
                "rust-acceptance = { run = [\"true\"], test = true }\n",
                "review = { run = [\"true\"], allowed_in_reviews = true }\n",
            ),
        ),
    ]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let run = json!({"id":"r1","kind":"work"});
    let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false).0;
    assert!(brief.contains("and `run <name>` for: rust-acceptance, spacing_parity."), "{brief}");
    assert!(!brief.contains("for: unit") && !brief.contains("for: rust-acceptance, spacing_parity, review"), "{brief}");
    // A project with no marked command gets only `test --all`.
    let q = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let u = open(&q.root).unwrap();
    let t = api(&u.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let brief = continuation_brief(&t, &run, false, &q.root, &q.root, false).0;
    assert!(brief.contains("run `test --all` in each project with symbol-held source."), "{brief}");
    assert!(!brief.contains("`run <name>` for:"), "{brief}");
    // Two projects in scope, one marked: name the project so `run` lands there.
    let w = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"web\"]\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() { 1 }\n"),
        ("web/solace.toml", "[commands]\nspacing_parity = { run = [\"true\"], test = true }\n"),
    ]);
    let s = open(&w.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["scope"] = json!([".", "web"]);
    t["home"] = json!(".");
    let brief = continuation_brief(&t, &run, false, &w.root, &w.root, false).0;
    assert!(brief.contains("and `run <name>` for: web: spacing_parity."), "{brief}");
    // A malformed settings file is surfaced, not treated as no marked commands.
    let bad = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&bad.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    fs::write(bad.root.join("solace.toml"), "[commands]\nspacing_parity = { run = [\"true\"], test = \"yes\" }\n").unwrap();
    let brief = continuation_brief(&t, &run, false, &bad.root, &bad.root, false).0;
    assert!(brief.contains("the marked test commands could not be read: project .: E6020"), "{brief}");
    // A bad project is named without hiding another project's marks in either order.
    let mixed = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"web\"]\n[commands]\nrust-acceptance = { run = [\"true\"], test = true }\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() { 1 }\n"),
        ("web/solace.toml", "[commands]\nspacing_parity = { run = [\"true\"], test = \"yes\" }\n"),
    ]);
    let s = open(&mixed.root).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    t["home"] = json!(".");
    for scope in [json!([".", "web"]), json!(["web", "."])] {
        t["scope"] = scope;
        let brief = continuation_brief(&t, &run, false, &mixed.root, &mixed.root, false).0;
        assert!(brief.contains("and `run <name>` for: .: rust-acceptance;"), "{brief}");
        assert!(brief.contains("the marked test commands could not be read for: project web: E6020"), "{brief}");
    }
}

/// D-203: a brief lists the tasks whose `dependsOn` names it.
#[test]
fn briefs_name_tasks_that_need_it() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = scope::top(&p.root);
    let home = scope::dir(&top, &p.root);
    let dir = store_dir(&top);
    fs::create_dir_all(&dir).unwrap();
    let put = |id: &str, title: &str, column: &str, deps: &[&str]| {
        let t = json!({"id":id,"title":title,"body":"","home":home,"column":column,"rank":1,"dependsOn":deps,
            "runs":[],"notes":[],"questions":[],"links":[],"closedTime":null,"archivedTime":null});
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("t", "Task t", "planning", &[]);
    put("n", "Task n", "assigned", &["t"]);
    put("other", "Task other", "planning", &[]);
    let needed = needed_by(&top, "t");
    assert!(needed.contains("Needed by:\n") && needed.contains("task:n Task n (Assigned)\n"), "{needed}");
    assert_eq!(needed_by(&top, "other"), String::new());
    let task = |id: &str| all_tasks(&top).unwrap().into_iter().find(|t| t["id"] == id).unwrap();
    let brief = continuation_brief(&task("t"), &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false).0;
    assert!(brief.contains(&needed) && brief.contains("task peek <id>"), "{brief}");
    assert!(!brief.contains("`task peek feature` reads the feature"), "{brief}");
}

/// D-203: `task peek` reads the task, its dependencies, its dependents and a
/// task named in its body, and refuses any other task, naming the allowed ones.
#[test]
fn task_peek_reads_the_tasks_a_run_may_read() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let named = api(&s.root, "POST", "/tasks", json!({"title":"Named","body":"named body"})).unwrap();
    let dep = api(&s.root, "POST", "/tasks", json!({"title":"Dependency","body":"dep body"})).unwrap();
    let other = api(&s.root, "POST", "/tasks", json!({"title":"Other"})).unwrap();
    let mut t =
        api(&s.root, "POST", "/tasks", json!({"title":"Mine","body":format!("See task:{}.", named["id"].as_str().unwrap())})).unwrap();
    let mut dependent = api(&s.root, "POST", "/tasks", json!({"title":"Dependent"})).unwrap();
    dependent["dependsOn"] = json!([t["id"]]);
    t["dependsOn"] = json!([dep["id"]]);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    tasks::put(&s, &dependent).unwrap();
    let tid = t["id"].as_str().unwrap();
    let peek = |target: &str, extra: &[&str], budget: usize| {
        let mut args = vec!["task".to_string(), "peek".to_string(), target.to_string()];
        args.extend(extra.iter().map(|x| x.to_string()));
        with_root(&p.root, || with_context(tid, "r1", || run_options(&args, &mut || Ok(String::new()), None, budget, &[])))
    };
    let allowed = [
        (dep["id"].as_str().unwrap(), "Dependency"),
        (dependent["id"].as_str().unwrap(), "Dependent"),
        (named["id"].as_str().unwrap(), "Named"),
    ];
    for (id, title) in allowed {
        let out = peek(id, &[], 10_000).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["title"], title, "{out}");
        assert_eq!(v["feature"], Value::Null, "{out}");
    }
    let own = peek(tid, &[], 10_000).unwrap();
    assert!(own.contains(&format!("task:{}", dep["id"].as_str().unwrap())) && own.contains("See task:"), "{own}");
    // A task outside the allowed set is refused, and the refusal names each allowed task.
    let e = peek(other["id"].as_str().unwrap(), &[], 10_000).unwrap_err();
    let listed: Vec<&str> = e.0.lines().filter(|l| l.starts_with("task:")).collect();
    assert_eq!(listed.len(), 4, "{}", e.0);
    for id in [tid, dep["id"].as_str().unwrap(), dependent["id"].as_str().unwrap(), named["id"].as_str().unwrap()] {
        assert!(listed.iter().any(|l| l.contains(&format!("task:{id}"))), "{id} missing from {}", e.0);
    }
    assert!(!listed.iter().any(|l| l.contains(other["id"].as_str().unwrap())), "{}", e.0);
}

/// D-203: `--notes` adds one list of notes or every one in full, and a long
/// note is paged with `--from` as `strategy show note` is.
#[test]
fn task_peek_notes_page() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let mut dep = api(&s.root, "POST", "/tasks", json!({"title":"Dependency"})).unwrap();
    dep["notes"] = json!([
        {"author":"human","text":"first","time":1,"run":null},
        {"author":"human","text":"n".repeat(400),"time":2,"run":null},
        {"author":"human","text":"third","time":3,"run":null},
        {"author":"human","text":"fourth","time":4,"run":null},
        {"author":"human","text":"fifth","time":5,"run":null},
        {"author":"human","text":"sixth","time":6,"run":null},
        {"author":"human","text":"seventh","time":7,"run":null},
        {"author":"human","text":"eighth","time":8,"run":null},
        {"author":"human","text":"ninth","time":9,"run":null}
    ]);
    tasks::put(&s, &dep).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Mine"})).unwrap();
    t["dependsOn"] = json!([dep["id"]]);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let tid = t["id"].as_str().unwrap();
    let did = dep["id"].as_str().unwrap();
    let peek = |extra: &[&str], budget: usize| {
        let mut args = vec!["task".to_string(), "peek".to_string(), did.to_string()];
        args.extend(extra.iter().map(|x| x.to_string()));
        with_root(&p.root, || {
            with_context(tid, "r1", || run_options(&args, &mut || Ok(String::new()), None, budget, &[]))
        })
    };
    let first = peek(&["--notes"], 650).unwrap();
    assert!(first.contains("\"note\": 1") && first.contains(&format!("Next: task peek {did} --notes 2")), "{first}");
    let second = peek(&["--notes", "2"], 500).unwrap();
    assert!(second.contains("\"note\": 2") && second.contains("\"part\": \"text, characters 0.."), "{second}");
    let next = second.lines().find(|l| l.starts_with("Next: ")).expect(&second);
    let from = next.rsplit("--from ").next().unwrap().trim();
    let more = peek(&["--notes", "2", "--from", from], 500).unwrap();
    assert!(more.contains("\"note\": 2") && !more.contains("characters 0.."), "{more}");
    let bad = peek(&["--notes", "10"], 10_000).unwrap_err();
    assert!(bad.0.contains("notes") && bad.0.contains("1..9") && !bad.0.contains("steps"), "{}", bad.0);
    let split = peek(&["--notes", "1", "2"], 10_000).unwrap_err();
    assert!(split.0.contains("notes take one list") && split.0.contains("--notes 1,2,8..12"), "{}", split.0);
    let picked: Value = serde_json::from_str(&peek(&["--notes", "1,8..9"], 10_000).unwrap()).unwrap();
    let numbers: Vec<_> = picked["notes"].as_array().unwrap().iter().filter_map(|n| n["note"].as_u64()).collect();
    assert_eq!(numbers, vec![1, 8, 9]);
}

/// D-203: `task peek feature` gives the feature's state, whole body and its
/// tasks with their columns; a task with no feature is refused.
#[test]
fn task_peek_feature_lists_its_tasks() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        ("solace.toml", "projects = [\"lib\"]\n"),
        ("lib/.solace/keep", ""),
    ]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&p.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search","body":"Everything.","state":"active"}))
        .map_err(|e| e.1.0)
        .unwrap();
    let fid = f["id"].as_str().unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Mine"})).unwrap();
    t["feature"] = json!(fid);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let mut sibling = api(&s.root, "POST", "/tasks", json!({"title":"Sibling"})).unwrap();
    sibling["feature"] = json!(fid);
    sibling["home"] = json!("lib");
    tasks::put(&s, &sibling).unwrap();
    let tid = t["id"].as_str().unwrap();
    let peek = |tid: &str, target: &str| {
        let args = vec!["task".to_string(), "peek".to_string(), target.to_string()];
        with_root(&p.root, || with_context(tid, "r1", || run_options(&args, &mut || Ok(String::new()), None, 10_000, &[])))
    };
    let out = peek(tid, "feature").unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (v["title"].as_str(), v["state"].as_str(), v["body"].as_str()),
        (Some("Search"), Some("active"), Some("Everything.")),
        "{out}"
    );
    let tasks = v["tasks"].as_array().unwrap();
    assert!(tasks.iter().any(|x| x.as_str().unwrap().contains(t["id"].as_str().unwrap())), "{out}");
    assert!(tasks.iter().any(|x| x.as_str().unwrap().contains(sibling["id"].as_str().unwrap())), "{out}");
    let sibling_line = tasks.iter().map(|x| x.as_str().unwrap()).find(|l| l.contains(sibling["id"].as_str().unwrap())).unwrap();
    assert!(sibling_line.contains(", in lib"), "{out}");
    // A task with no feature is refused.
    let mut plain = api(&s.root, "POST", "/tasks", json!({"title":"Plain"})).unwrap();
    plain["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &plain).unwrap();
    let e = peek(plain["id"].as_str().unwrap(), "feature").unwrap_err();
    assert!(e.0.contains("has no feature"), "{}", e.0);
}

/// D-203: `session remember` carries the same plan sections as the brief.
#[test]
fn remember_carries_the_plan_around_the_task() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let top = scope::top(&p.root);
    let f = tasks::strategy_api(&top, "POST", &["features"], json!({"title":"Search","body":"Everything.","state":"active"}))
        .map_err(|e| e.1.0)
        .unwrap();
    let dep = api(&s.root, "POST", "/tasks", json!({"title":"Dependency"})).unwrap();
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Mine"})).unwrap();
    t["feature"] = json!(f["id"]);
    t["dependsOn"] = json!([dep["id"]]);
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let mut dependent = api(&s.root, "POST", "/tasks", json!({"title":"Dependent"})).unwrap();
    dependent["dependsOn"] = json!([t["id"]]);
    tasks::put(&s, &dependent).unwrap();
    let remembered = remember(&top, t["id"].as_str().unwrap(), "r1", 10_000).unwrap();
    for part in [
        "Feature (the feature this task is part of",
        "- state: active",
        "Everything.",
        "Depends on:",
        "Needed by:",
        "task peek <id>",
        "task peek feature",
    ] {
        assert!(remembered.contains(part), "{part} missing from:\n{remembered}");
    }
}

