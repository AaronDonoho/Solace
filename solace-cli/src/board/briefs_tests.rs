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
use crate::board::briefs::WAITING;
use crate::board::briefs::waiting_on;
use crate::board::briefs::START;
use crate::board::briefs::HELD_START;
use crate::board::briefs::TECH_DEBT_ASK;
use crate::board::briefs::FINDINGS_ASK;
use crate::board::briefs::needed_by;
use crate::board::briefs::note_fingerprint;
use crate::board::briefs::remember;
use crate::board::task_model::all_tasks;
use crate::scope;
use crate::cli::with_root;
use crate::cli::run_options;
use crate::board::runs::with_context;
/// D-186: an investigation states outcomes and constraints, not a plan that
/// assigns edits or submission calls.
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
        let prompt = brief(task, "investigate", "engineer");
        for required in [
            "Investigate this task",
            "Report the required behavior, constraints and acceptance criteria",
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
    let review = brief(&code, "review", "engineer");
    assert!(review.contains("Use those to verify (for example test suites)"), "{review}");
    assert!(!review.contains("Probe or run tests only when needed"), "{review}");
    let work = brief(&code, "work", "engineer");
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
        let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false, "engineer").0;
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
    let brief = continuation_brief(&t2, &json!({"id":"r1","kind":"work"}), false, &q.root, &q.root, false, "engineer").0;
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
    let brief =
        |kind: &str, resumed: bool| continuation_brief(&t, &json!({"id":"r1","kind":kind}), resumed, &p.root, &p.root, false, "engineer").0;
    let (reply, next) = (
        "\nSession start (the reply to `session start --task`):\n",
        "\nNext: read what the change needs in one call, one command per line: `inspect app.total",
    );
    let reference = crate::reference::agent_reference(true);
    for kind in ["work", "investigate"] {
        let b = brief(kind, false);
        assert!(b.contains(reply) && b.contains("Symbols most likely relevant to the task") && b.contains("-- app.total"), "{kind}: {b}");
        assert!(b.starts_with(reference.as_str()) && b.matches(reference.as_str()).count() == 1, "{kind}: {b}");
        assert!(!b.contains(START) && b.contains(HELD_START), "{kind}: {b}");
        let (at, stop) = (b.find(next).expect(&b), b.find("After the call that ends the run returns, stop").unwrap());
        assert!(b.find(reply).unwrap() < at && at < stop, "{kind}: the reply's first call comes just before the stop line: {b}");
        assert!(!b.contains("Feature (the feature") && !b.contains("Depends on"), "{kind}: {b}");
        assert!(!b.lines().any(|line| line.starts_with("`task peek")), "{kind}: {b}");
    }
    for kind in ["review", "discuss"] {
        let b = brief(kind, false);
        assert!(b.starts_with(reference.as_str()) && b.matches(reference.as_str()).count() == 1, "{kind}: {b}");
        assert!(!b.contains(reply) && !b.contains(HELD_START), "{kind}: {b}");
    }
    let dep = api(&s.root, "POST", "/tasks", json!({"title":"Dependency"})).unwrap();
    let mut needs = api(&s.root, "POST", "/tasks", json!({"title":"Needs one"})).unwrap();
    needs["dependsOn"] = json!([dep["id"]]);
    tasks::put(&s, &needs).unwrap();
    let b = continuation_brief(&needs, &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false, "engineer").0;
    assert!(b.lines().any(|line| line == "`task peek <id>` reads a listed task."), "{b}");
    assert!(!b.contains("`task peek feature` reads the feature"), "{b}");
    let f = tasks::strategy_api(&scope::top(&p.root), "POST", &["features"], json!({"title":"Search"})).map_err(|e| e.1.0).unwrap();
    let mut featured = api(&s.root, "POST", "/tasks", json!({"title":"Featured"})).unwrap();
    featured["feature"] = json!(f["id"]);
    tasks::put(&s, &featured).unwrap();
    let b = continuation_brief(&featured, &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false, "engineer").0;
    assert!(b.lines().any(|line| line == "`task peek feature` reads the feature."), "{b}");
    assert!(!b.lines().any(|line| line.starts_with("`task peek <id>`")), "{b}");
    let resumed = brief("work", true);
    assert!(
        resumed.contains("Use the continuation context below.")
            && !resumed.contains(reply)
            && !resumed.contains(HELD_START)
            && !resumed.starts_with(reference.as_str()),
        "{resumed}"
    );
    fs::remove_file(p.root.join("gleam.toml")).unwrap();
    let plain = brief("work", false);
    assert!(
        plain.contains(reply)
            && plain.contains("This project has plain files")
            && plain.contains("`grep <text> --in <path>`")
            && plain.starts_with(reference.as_str())
            && plain.matches(reference.as_str()).count() == 1,
        "{plain}"
    );
    // A folder with no store keeps the line asking for the call.
    let bare = testing::Scratch::new();
    let b = continuation_brief(&t, &json!({"id":"r1","kind":"work"}), false, &p.root, &bare, false, "engineer").0;
    assert!(b.contains(START) && !b.contains(reply) && b.starts_with(reference.as_str()), "{b}");
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
    let brief = continuation_brief(&t, &run, true, &p.root, &p.root, false, "engineer").0;
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
    assert!(continuation_brief(&task("t"), &run, false, &p.root, &p.root, false, "engineer").0.contains(&w));
    let finished = continuation_brief(&task("f"), &run, false, &p.root, &p.root, false, "engineer").0;
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
    let b = brief(&t, "work", "engineer");
    assert!(!b.contains("How to talk to the human"), "{b}");
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
        let prompt = brief(&t, kind, "engineer");
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
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind, "engineer");
        assert!(prompt.contains(UI_DETAILS_ASK), "{kind}: missing {UI_DETAILS_ASK}");
    }
    // D-181: source files are projections; only a module, symbol or other file is named.
    for kind in ["work", "investigate", "review", "discuss"] {
        assert!(
            brief(&t, kind, "engineer").contains(concat!(
                "Code held as symbols is read and cited by name (`lockdown.dispatch`, `checkout.pay`), ",
                "not by source path: source files are projections written by Solace.",
            )),
            "{kind}: missing the projection rule"
        );
    }
    let plan = brief(&t, "investigate", "engineer");
    assert!(
        plan.contains("Report the required behavior, constraints and acceptance criteria")
            && plan.contains("each criterion a result a worker or reviewer can confirm")
            && plan.contains("list a risk only when you can't settle it, and say why")
            && !plan.contains("submission shape or checks")
            && !plan.contains("names every module or symbol it changes"),
        "{plan}"
    );
    // D-119 point 6: check what is already answered before asking.
    for kind in ["work", "investigate"] {
        assert!(brief(&t, kind, "engineer").contains("check the Notes so far and Questions below, if any"), "{kind}");
        // D-153: each option shows an example of its outcome.
        assert!(brief(&t, kind, "engineer").contains("give each option a short example of the outcome the human would see"), "{kind}");
    }
    // A scope request names the folder and puts the human's steps in order,
    // even in Director mode.
    for kind in ["work", "investigate"] {
        for involvement in ["engineer", "director"] {
            let prompt = brief(&t, kind, involvement);
            assert!(prompt.contains(crate::scope::ASK_FOR_PROJECT), "{kind}/{involvement}: missing the scope rule");
        }
    }
    assert!(
        brief(&t, "work", "director").contains("project folder names in a scope request are the exception"),
        "director mode must allow a project folder in a scope request"
    );
    for kind in ["review", "discuss"] {
        assert!(!brief(&t, kind, "engineer").contains(crate::scope::ASK_FOR_PROJECT), "{kind}: unexpected scope rule");
    }
    assert!(brief(&t, "investigate", "engineer").contains("Investigate this task"));
    // D-147: a report never hands a file change to the human.
    assert!(
        brief(&t, "investigate", "engineer")
            .contains("The human never edits files or runs commands; any needed change, a settings change included, is the worker's")
    );
    // D-156/D-186: an investigation reports requirements, not a plan for edits.
    assert!(
        brief(&t, "investigate", "engineer").contains("anything you didn't check")
            && !brief(&t, "investigate", "engineer").contains("A plan is the smallest complete change")
    );
    assert!(brief(&t, "work", "engineer").contains("what you skipped and risks the human must know"));
    assert!(brief(&t, "review", "engineer").contains("Flag code the task didn't need, places it missed"));
    for kind in ["work", "investigate", "discuss"] {
        assert!(!brief(&t, kind, "engineer").contains("smallest complete change"), "{kind}");
    }
    for kind in ["work", "discuss"] {
        assert!(!brief(&t, kind, "engineer").contains("Flag code the task didn't need"), "{kind}");
    }
    // Maintainability: each role gets the direction it acts on, and both share
    // the comment and UI rules.
    let work = brief(&t, "work", "engineer");
    let review = brief(&t, "review", "engineer");
    assert!(
        work.contains(concat!(
            "Prove each acceptance criterion through the entry point - for example: a request through the ",
            "server's routing, not the function behind it; a save confirmed by a fresh read."
        )),
        "work: missing the proof rule"
    );
    assert!(
        review.contains(concat!(
            "Red-team the changes: assume they don't work until you prove they meet the task's goal and ",
            "acceptance criteria and ",
            "break nothing they reach."
        )),
        "review: missing the proof rule"
    );
    assert!(
        !work.contains("Red-team the changes") && !review.contains("Prove each acceptance criterion"),
        "proof rules must stay with their roles"
    );
    for kind in ["work", "review"] {
        assert!(
            brief(&t, kind, "engineer").contains(
                "Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI."
            ),
            "{kind}: missing the comment and UI rules"
        );
    }
    assert!(
        review.contains(
            "Flag user-interface details the task didn't ask for, including labels, wording, indicators, colors, new buttons or screens, and layout."
        ),
        "review: missing the unasked UI flag"
    );
    assert!(!work.contains("Flag user-interface details"), "work: unexpected reviewer UI flag");
    for rule in [
        "Reuse existing code before adding new code, extracting common patterns",
        "small, stateless, self-documenting functions",
        "small and focused around one domain or component",
        "Put new code in the module that owns its concern",
        "A bug fix's test must fail on the old code",
        "Don't remove a test or change its expected value unless the task says to",
    ] {
        assert!(work.contains(rule), "work: missing {rule}");
        assert!(!review.contains(rule), "review: unexpected {rule}");
    }
    for kind in ["review", "investigate", "discuss"] {
        let prompt = brief(&t, kind, "engineer");
        for rule in [
            "Put new code in the module that owns its concern",
            "A bug fix's test must fail on the old code",
            "Don't remove a test or change its expected value unless the task says to",
        ] {
            assert!(!prompt.contains(rule), "{kind}: unexpected {rule}");
        }
        assert!(!prompt.contains("2414798176c466e4"), "{kind}: stale refactor task reference");
    }
    for rule in [
        "Flag a removed or weakened test, an expected value changed without reason, and a bug fix whose test would pass on the old code",
        "Flag duplication and large modules",
        "minor ones as tech debt",
        "cross-domain concerns in the same module",
        "excessively long functions and dead code",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
        assert!(!work.contains(rule), "work: unexpected {rule}");
    }
    for rule in [
        "Choose **Needs work** only when the change doesn't meet the task's goal or acceptance criteria, has real bugs, or takes a direction that could be hazardous",
        "say so plainly in a `### Summary` right after the assessment label",
        "code repeated once or twice isn't tech debt by itself",
        "don't push for an abstraction unless it clearly pays for itself",
        "Judge an edge case by what happens if it occurs, not by how likely it is",
        "Every item to fix later goes in the `### Tech debt` section and nowhere else",
        "Findings hold only what blocks this task or explains the assessment",
    ] {
        assert!(review.contains(rule), "review: missing {rule}");
        assert!(!work.contains(rule), "work: unexpected {rule}");
    }
    assert!(!review.contains("an unlikely, harmless edge case"), "review: old edge-case rule remains");
    assert!(!review.contains("Never file tech debt for an unlikely, harmless edge case"), "review: old tech-debt rule remains");
    assert_eq!(review.matches("Judge an edge case by what happens").count(), 1, "review: the edge-case bar must appear once");
    let followup = "the latest review is part of this task, whatever its assessment";
    assert!(work.contains(followup), "work: missing the review follow-up rule");
    for kind in ["review", "investigate", "discuss"] {
        assert!(!brief(&t, kind, "engineer").contains(followup), "{kind}: unexpected review follow-up rule");
    }
    for involvement in ["engineer", "collaborator", "director"] {
        assert!(
            brief(&t, "review", involvement).contains("say so plainly in a `### Summary` right after the assessment label"),
            "review/{involvement}: missing the Summary rule"
        );
    }
    // D-182: only a work run is told to run every test, once, after its last change.
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind, "engineer");
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
    let d = brief(&talk, "discuss", "engineer");
    assert!(d.contains("Long A.") && d.contains("Keep.\n\n### All\n\nOf it.") && d.contains("Long B."), "{d}");
    let r = brief(&t, "review", "engineer");
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
        assert!(!brief(&t, kind, "engineer").contains("Only a `task note` call records the feedback"), "{kind}");
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
    assert!(
        !work.contains("End the `task done` summary with a `### Tech debt` section"),
        "a work run is not asked for step and tool faults as tech debt"
    );
    assert!(
        !work.contains("ordinary friction, code-quality notes and restating risks are not tech debt"),
        "a work run is not told what is not tech debt"
    );
    for kind in ["work", "investigate", "discuss"] {
        let prompt = brief(&t, kind, "engineer");
        assert!(!prompt.contains(FINDINGS_ASK), "{kind}: unexpected findings detail");
        assert!(!prompt.contains(TECH_DEBT_ASK), "{kind}: unexpected tech debt ask");
    }
    // D-198: a review of a task with a feature is asked the same.
    let mut featured = t.clone();
    featured["feature"] = json!("f1");
    assert!(brief(&featured, "review", "engineer").contains(TECH_DEBT_ASK), "a featured review is asked for tech debt");
    // A task across projects (D-61) says where the agent works and how it reaches the rest.
    let mut wide = t.clone();
    wide["scope"] = json!(["a", "web"]);
    wide["home"] = json!("a");
    wide["changes"][0]["project"] = json!("web");
    wide["test_commands"] = json!([
        {"project":"a","commands":["rust-acceptance"]},
        {"project":"web","commands":["spacing_parity"]}
    ]);
    let w = brief(&wide, "work", "engineer");
    assert!(w.contains("covers the projects a, web. You work in a; for another of them, add `--project <folder>`"), "{w}");
    assert!(w.contains("Task changes:\n- web: calc.add"), "{w}");
    assert!(w.contains("and `run <name>` for: a: rust-acceptance; web: spacing_parity."), "{w}");
    for kind in ["work", "investigate"] {
        assert_eq!(
            brief(&wide, kind, "engineer").matches(crate::scope::ASK_FOR_PROJECT).count(),
            1,
            "{kind}: the scope rule should appear exactly once"
        );
    }
    for kind in ["review", "discuss"] {
        assert_eq!(brief(&wide, kind, "engineer").matches(crate::scope::ASK_FOR_PROJECT).count(), 0, "{kind}: unexpected scope rule");
    }
    assert!(!work.contains("clear names for new symbols"), "work: the old clear-name rule remains");
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
    let brief = continuation_brief(&t, &run, false, &p.root, &p.root, false, "engineer").0;
    assert!(brief.contains("and `run <name>` for: rust-acceptance, spacing_parity."), "{brief}");
    assert!(!brief.contains("for: unit") && !brief.contains("for: rust-acceptance, spacing_parity, review"), "{brief}");
    // A project with no marked command gets only `test --all`.
    let q = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let u = open(&q.root).unwrap();
    let t = api(&u.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    let brief = continuation_brief(&t, &run, false, &q.root, &q.root, false, "engineer").0;
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
    let brief = continuation_brief(&t, &run, false, &w.root, &w.root, false, "engineer").0;
    assert!(brief.contains("and `run <name>` for: web: spacing_parity."), "{brief}");
    // A malformed settings file is surfaced, not treated as no marked commands.
    let bad = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&bad.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"T"})).unwrap();
    fs::write(bad.root.join("solace.toml"), "[commands]\nspacing_parity = { run = [\"true\"], test = \"yes\" }\n").unwrap();
    let brief = continuation_brief(&t, &run, false, &bad.root, &bad.root, false, "engineer").0;
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
        let brief = continuation_brief(&t, &run, false, &mixed.root, &mixed.root, false, "engineer").0;
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
    let brief = continuation_brief(&task("t"), &json!({"id":"r1","kind":"work"}), false, &p.root, &p.root, false, "engineer").0;
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

/// Director: plain words, product questions, calls recorded as calls made.
#[test]
fn director_briefs_speak_in_plain_words() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind, "director");
        for rule in [
            "How to talk to the human (Director)",
            "Don't use code, symbol names, file paths or error codes",
            "Ask about what the software should do, never how",
            "one option is marked as recommended",
            "`### Calls made` heading",
            "never goes in `DECISIONS.md`",
            "`### Summary` heading",
            "Detail meant for later agents, review findings and `### Tech debt` items may name code, symbols and files",
        ] {
            assert!(prompt.contains(rule), "{kind}: missing {rule}\n{prompt}");
        }
        if kind == "review" {
            assert!(prompt.contains("the assessment label is the first line and the `### Summary` heading comes after it"), "{prompt}");
        }
        assert!(!prompt.contains("what a command prints"), "{kind}: {prompt}");
        assert!(!prompt.contains("How to talk to the human (Collaborator)"), "{kind}: {prompt}");
    }
}

/// Collaborator: terms explained, trade-offs in a sentence, one recommendation.
#[test]
fn collaborator_briefs_explain_terms_and_trade_offs() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    for kind in ["work", "investigate", "review", "discuss"] {
        let prompt = brief(&t, kind, "collaborator");
        for rule in [
            "How to talk to the human (Collaborator)",
            "explain each technical term the first time it appears",
            "Summarize each trade-off in one sentence",
            "one question option as recommended",
            "`### Summary` heading",
            "Detail meant for later agents, review findings and `### Tech debt` items may use technical terms",
        ] {
            assert!(prompt.contains(rule), "{kind}: missing {rule}\n{prompt}");
        }
        if kind == "review" {
            assert!(prompt.contains("the assessment label is the first line and the `### Summary` heading comes after it"), "{prompt}");
        }
        assert!(!prompt.contains("How to talk to the human (Director)"), "{kind}: {prompt}");
    }
}

fn brief(t: &Value, kind: &str, involvement: &str) -> String {
    super::brief(t, kind, involvement, "standard", "balanced")
}

fn continuation_brief(
    t: &Value,
    run: &Value,
    resumed: bool,
    root: &std::path::Path,
    here: &std::path::Path,
    custom: bool,
    involvement: &str,
) -> (String, Vec<std::path::PathBuf>) {
    super::continuation_brief(t, run, resumed, root, here, custom, involvement, "standard", "balanced")
}

/// Detail changes how much a note says; asking changes when the agent asks.
#[test]
fn detail_and_asking_briefs_follow_their_settings() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    let concise = super::brief(&t, "work", "engineer", "concise", "balanced");
    assert!(concise.contains("How much to write (Concise)"), "{concise}");
    assert!(concise.contains("few lines") && concise.contains("outcome first"), "{concise}");
    assert!(!concise.contains("How much to write (Thorough)"), "{concise}");
    let standard = super::brief(&t, "work", "engineer", "standard", "balanced");
    assert!(!standard.contains("How much to write"), "{standard}");
    let thorough = super::brief(&t, "work", "engineer", "thorough", "balanced");
    assert!(thorough.contains("How much to write (Thorough)"), "{thorough}");
    assert!(thorough.contains("alternatives considered") && thorough.contains("how the result was checked"), "{thorough}");
    let often = super::brief(&t, "work", "engineer", "standard", "often");
    assert!(often.contains("When to ask (Often)"), "{often}");
    assert!(often.contains("more than one reasonable reading"), "{often}");
    let balanced = super::brief(&t, "work", "engineer", "standard", "balanced");
    assert!(balanced.contains("When to ask (Balanced)"), "{balanced}");
    assert!(balanced.contains("redoing work") && balanced.contains("what the human sees"), "{balanced}");
    let rarely = super::brief(&t, "work", "engineer", "standard", "rarely");
    assert!(rarely.contains("When to ask (Rarely)"), "{rarely}");
    assert!(
        rarely.contains("`### Calls made` heading") && rarely.contains("can't be undone") && rarely.contains("contradict the task"),
        "{rarely}"
    );
    for prompt in [&often, &balanced, &rarely] {
        assert!(prompt.contains("clearly contradicts a decision, the vision, its feature or another task"), "{prompt}");
    }
    for kind in ["work", "investigate", "review", "discuss"] {
        assert!(super::brief(&t, kind, "engineer", "concise", "rarely").contains("How much to write (Concise)"), "{kind}");
        assert!(super::brief(&t, kind, "engineer", "thorough", "often").contains("When to ask (Often)"), "{kind}");
    }
}

/// Director + Rarely records a call made once, not in both sections.
#[test]
fn director_rarely_records_calls_made_once() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    let prompt = super::brief(&t, "work", "director", "standard", "rarely");
    assert_eq!(prompt.matches("### Calls made").count(), 1, "{prompt}");
    assert!(!prompt.contains(".."), "{prompt}");
    assert!(prompt.contains("When to ask (Rarely)"), "{prompt}");
}

/// Engineer + Rarely records a call made with one full stop, not two.
#[test]
fn engineer_rarely_brief_has_no_double_full_stop() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    let prompt = super::brief(&t, "work", "engineer", "standard", "rarely");
    assert!(prompt.contains("### Calls made") && !prompt.contains(".."), "{prompt}");
}

/// Fresh briefs of different tasks, kinds and projects share the reference start.
#[test]
fn fresh_briefs_of_different_projects_share_the_reference_start() {
    let reference = crate::reference::agent_reference(true);
    let mut starts = vec![];
    for (file, title, kind) in [("src/app.gleam", "First task", "work"), ("src/other.gleam", "Second task", "investigate")] {
        let p = testing::project(&[(file, "pub fn main() { 1 }\n")]);
        let s = open(&p.root).unwrap();
        let t = api(&s.root, "POST", "/tasks", json!({"title": title})).unwrap();
        let b = continuation_brief(&t, &json!({"id":"r1","kind":kind}), false, &p.root, &p.root, false, "engineer").0;
        assert!(b.starts_with(reference.as_str()), "{kind}: {b}");
        assert_eq!(b.matches(reference.as_str()).count(), 1, "{kind}: {b}");
        starts.push(b);
    }
    assert_eq!(&starts[0][..reference.len()], &starts[1][..reference.len()]);
}

/// D-218: investigate, work, discuss and review briefs carry every persona,
/// with only its non-empty fields; review still leaves the vision out.
#[test]
fn a_brief_carries_every_persona_in_creation_order() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"Personas"})).unwrap();
    let strategy = p.root.join(".solace/strategy");
    fs::create_dir_all(&strategy).unwrap();
    fs::write(strategy.join("vision.md"), "For teams.\n").unwrap();
    let brief = |kind: &str| continuation_brief(&t, &json!({"id":"r1","kind":kind}), true, &p.root, &p.root, false, "engineer").0;
    assert!(!brief("work").contains("Personas ("), "no personas, no section");
    fs::write(
        strategy.join("personas.json"),
        concat!(
            r#"{"personas":[{"name":"Ada","objectives":"Ship the calculator.","tools":"","#,
            r#""concerns":"Wrong totals.","other":"  "},{"name":"Linus","objectives":"One.\nTwo.","#,
            r#""tools":"","concerns":"","other":""},{"name":"Grace","objectives":"","#,
            r#""tools":"A debugger.","concerns":"","other":""}],"retired":[]}"#
        ),
    )
    .unwrap();
    let work = brief("work");
    let section =
        concat!("\nPersonas (who the software is for, or who works on it, kept on the Strategy page; ", "let them guide your choices):\n");
    assert!(work.contains(section), "{work}");
    assert!(work.contains("Ada\n  Objectives: Ship the calculator.\n  Concerns: Wrong totals.\n"), "{work}");
    assert!(work.contains("Linus\n  Objectives: One.\n    Two.\n"), "{work}");
    assert!(work.contains("Grace\n  Tools: A debugger.\n"), "{work}");
    assert!(!work[work.find("Ada").unwrap()..work.find("Grace").unwrap()].contains("Tools:"), "{work}");
    assert!(!work[work.find("Grace").unwrap()..].contains("Objectives:"), "{work}");
    for kind in ["investigate", "discuss", "review"] {
        assert!(brief(kind).contains(section), "{kind}");
    }
    let review = brief("review");
    assert!(!review.contains("\nVision ("), "{review}");
    assert!(review.contains(section), "{review}");
    assert!(work.contains("\nVision ("), "{work}");
    // A damaged file reads as none rather than failing the brief.
    fs::write(strategy.join("personas.json"), "{").unwrap();
    assert!(!brief("work").contains("Personas ("), "{}", brief("work"));
}

/// D-27: a reviewer brief carries the reviewer's skill whole, leaves a worker
/// skill out, and still lists an unscoped skill one line.
#[test]
fn a_reviewer_brief_holds_its_skill_in_full_and_leaves_out_a_worker_skill() {
    let p = testing::project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        (
            "skills/house-review.md",
            "---\nname: house-review\ndescription: Review UI changes.\nroles: reviewer\n---\nCheck contrast and focus.\n",
        ),
        (
            "skills/commit-style.md",
            "---\nname: commit-style\ndescription: Write small commits.\nroles: worker\n---\nKeep every commit small.\n",
        ),
        ("skills/general.md", "General rules\n"),
    ]);
    let s = open(&p.root).unwrap();
    let t = api(&s.root, "POST", "/tasks", json!({"title":"Review"})).unwrap();
    let review = continuation_brief(&t, &json!({"id":"r1","kind":"review"}), false, &p.root, &p.root, false, "engineer").0;
    assert!(review.contains("### house-review") && review.contains("Check contrast and focus."), "{review}");
    assert!(!review.contains("### commit-style") && !review.contains("Keep every commit small."), "{review}");
    assert!(review.contains("- general: General rules (`read skills/general.md`)"), "{review}");
    let work = continuation_brief(&t, &json!({"id":"r2","kind":"work"}), false, &p.root, &p.root, false, "engineer").0;
    assert!(work.contains("### commit-style") && work.contains("Keep every commit small."), "{work}");
    assert!(!work.contains("### house-review") && !work.contains("Check contrast and focus."), "{work}");
    assert!(work.contains("- general: General rules"), "{work}");
}

/// D-27: a run in a nested project carries the workspace's role skills whole,
/// but not its unscoped skills, whose paths point outside the run's project.
#[test]
fn a_nested_project_run_gets_workspace_role_skills_only() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"app\", \"lib\"]\n"),
        ("app/gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"),
        ("app/src/app.gleam", "pub fn main() { 1 }\n"),
        ("lib/gleam.toml", "name = \"lib\"\nversion = \"1.0.0\"\n"),
        ("lib/src/lib.gleam", "pub fn main() { 1 }\n"),
        (
            "skills/house-review.md",
            "---\nname: house-review\ndescription: Review UI changes.\nroles: reviewer\n---\nCheck contrast and focus.\n",
        ),
        ("skills/general.md", "General rules\n"),
    ]);
    let t = json!({
        "id": "t1", "title": "Review", "body": "A change.", "stamps": {},
        "links": [], "notes": [], "questions": [], "runs": [], "changes": [],
    });
    let here = root.0.join("lib");
    let (brief, _) = continuation_brief(&t, &json!({"id":"r1","kind":"review"}), false, &root, &here, false, "engineer");
    assert!(brief.contains("### house-review") && brief.contains("Check contrast and focus."), "{brief}");
    assert!(!brief.contains("- general:") && !brief.contains("General rules"), "{brief}");
}

#[test]
fn a_note_without_an_id_keeps_its_old_hash_key() {
    let note = json!({"author":"human","run":null,"text":"one","time":1});
    assert_eq!(note_fingerprint(&note), crate::sha256(note.to_string().as_bytes()));
}

/// D-228: a Tech Debt task leaves its shared description out of the ranking
/// and reads its findings, so the first call names the code they name.
#[test]
fn tech_debt_findings_lead_the_first_call() {
    use crate::board::reviews::TECH_DEBT_BODY;
    let p = testing::project(&[
        ("src/outside.gleam", "pub fn tests() -> Int {\n  1\n}\n"),
        ("src/board/reviews.gleam", "pub fn record_approved_layout() -> Int {\n  1\n}\n\npub fn restore() -> Int {\n  2\n}\n"),
    ]);
    let t = json!({
        "id": "t1",
        "title": "Tech Debt",
        "body": TECH_DEBT_BODY,
        "techDebt": true,
        "notes": [{
            "author": "solace",
            "text": concat!(
                "Tech debt from the review of task:t1 (review run rv):\n\n",
                "`outside.tests`, `board::reviews.record_approved_layout` and `board::reviews.restore` disagree."
            ),
        }],
    });
    let ranked = task_text(&t);
    assert!(ranked.contains("outside.tests") && !ranked.contains(TECH_DEBT_BODY), "{ranked}");
    let reply = session_reply(&t, &p.root).expect("a reply");
    let next = reply.lines().find(|l| l.starts_with("Next:")).expect(&reply);
    for name in ["outside.tests", "reviews.record_approved_layout", "reviews.restore"] {
        assert!(next.contains(name), "{next}");
    }
    assert!(!next.contains("TECH_DEBT_ASK") && !next.contains("TECH_DEBT_BODY"), "{next}");
}

/// D-228: many named findings still fill the three suggested slots, and two
/// tests in one module count as one symbol.
#[test]
fn tech_debt_findings_fill_three_slots() {
    use crate::board::reviews::TECH_DEBT_BODY;
    let p = testing::project(&[
        ("src/outside.gleam", "pub fn tests() -> Int {\n  1\n}\n"),
        ("src/board/reviews.gleam", "pub fn record_approved_layout() -> Int {\n  1\n}\n\npub fn restore() -> Int {\n  2\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n\npub fn other() -> Int {\n  2\n}\n"),
    ]);
    let t = json!({
        "id": "t1",
        "title": "Tech Debt",
        "body": TECH_DEBT_BODY,
        "techDebt": true,
        "notes": [{
            "author": "solace",
            "text": concat!(
                "Tech debt from the work on task:t1 (work run w):\n\n",
                "`outside.tests::alpha`, `outside.tests::beta`, `board::reviews.restore`, ",
                "`board::reviews.record_approved_layout`, `app.main` and `app.other`."
            ),
        }],
    });
    let reply = session_reply(&t, &p.root).expect("a reply");
    let next = reply.lines().find(|l| l.starts_with("Next:")).expect(&reply);
    let names = next.split("`inspect ").nth(1).and_then(|s| s.split(" --context`").next()).expect(next);
    assert_eq!(names.split_whitespace().count(), 3, "{next}");
    assert!(!next.contains("TECH_DEBT_ASK") && !next.contains("TECH_DEBT_BODY"), "{next}");
}

/// D-260: every task's notes join its title and description in the ranking.
/// A Tech Debt task still leaves out its shared description, and a finding is
/// read without the line naming its task and run.
#[test]
fn task_text_keeps_an_edited_description_and_reads_every_tasks_notes() {
    use crate::board::reviews::TECH_DEBT_BODY;
    let edited = json!({
        "title": "Tech Debt",
        "body": "Findings at the start of the call.",
        "techDebt": true,
        "notes": [{
            "author": "solace",
            "text": "Tech debt from the review of task:t1 (review run rv):\n\n`app.main`: split it.",
        }],
    });
    let text = task_text(&edited);
    assert!(text.contains("Findings at the start of the call."), "{text}");
    assert!(text.contains("`app.main`: split it."), "{text}");
    assert!(!text.contains(TECH_DEBT_BODY), "{text}");

    let ordinary = json!({
        "title": "Ordinary",
        "body": "A normal description.",
        "notes": [
            {
                "author": "solace",
                "text": "Tech debt from the review of task:t2 (review run rv2):\n\n`app.other`: split it.",
            },
            {
                "author": "solace",
                "text": "Tech debt from the work on task:t3 (work run w1):\n\n`app.third`: name it.",
            },
        ],
    });
    assert_eq!(task_text(&ordinary), "Ordinary\nA normal description.\n`app.other`: split it.\n`app.third`: name it.");
}

#[test]
fn design_links_reach_work_and_review_briefs() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let ui = "0123456789abcdef";
    crate::design::save(
        &p.root,
        &json!({
            "schemaVersion": crate::design::SCHEMA_VERSION,
            "id": ui,
            "title": "Home",
            "tab": "ui",
            "elements": [{
                "id": "root",
                "type": "screen",
                "properties": {
                    "width": "fill", "height": "fill", "gap": 8, "padding": 16,
                    "align": "start", "justify": "center"
                },
                "children": [{
                    "id": "heading",
                    "type": "text",
                    "properties": {"align": "left", "width": "hug", "height": "hug", "text": "Home"}
                }]
            }]
        }),
        crate::design::Source::Human,
    )
    .unwrap();
    let architecture = "fedcba9876543210";
    std::fs::create_dir_all(p.root.join("designs")).unwrap();
    std::fs::write(
        p.root.join("designs").join(format!("{architecture}.json")),
        json!({
            "schemaVersion": crate::design::SCHEMA_VERSION,
            "id": architecture,
            "title": "Layers",
            "tab": "architecture",
            "elements": []
        })
        .to_string(),
    )
    .unwrap();
    let brief = |body: &str, kind: &str| {
        let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Designs"})).unwrap();
        t["body"] = json!(body);
        continuation_brief(&t, &json!({"id":"r1","kind":kind}), false, &p.root, &p.root, false, "engineer").0
    };
    for body in [format!("Build it from design:{ui}."), format!("Build [Home](design:{ui}).")] {
        let work = brief(&body, "work");
        assert!(work.contains(&format!("design:{ui}: Home. Read it with `design show design:{ui}`.")), "{work}");
        assert!(work.contains("reference material: write ordinary code") && work.contains("`help lustre`"), "{work}");
        let review = brief(&body, "review");
        assert!(review.contains("elements, their order and their text") && review.contains("not pixel by pixel"), "{review}");
        assert!(!review.contains("Use `help lustre`"), "{review}");
    }
    let architecture_work = brief(&format!("Use design:{architecture}."), "work");
    assert!(architecture_work.contains("Layers") && !architecture_work.contains("Use `help lustre`"), "{architecture_work}");
    let architecture_review = brief(&format!("Use design:{architecture}."), "review");
    assert!(architecture_review.contains("parts and how they connect"), "{architecture_review}");
    assert!(!architecture_review.contains("built screen against"), "{architecture_review}");
    let none = "No design here.";
    for kind in ["work", "review"] {
        let b = brief(none, kind);
        assert!(!b.contains("Designs named by the task") && !b.contains("design show"), "{kind}: {b}");
    }
    for kind in ["investigate", "discuss"] {
        let b = brief(&format!("Build design:{ui}."), kind);
        assert!(!b.contains("Designs named by the task"), "{kind}: {b}");
    }
    let code = brief(&format!("Write `design:{ui}` literally."), "work");
    assert!(!code.contains("Designs named by the task"), "{code}");
    let missing = brief("Build design:aaaaaaaaaaaaaaaa.", "work");
    assert!(missing.contains("could not be read") && missing.contains("E7023"), "{missing}");
    let invalid = "bbbbbbbbbbbbbbbb";
    std::fs::write(p.root.join("designs").join(format!("{invalid}.json")), "{}").unwrap();
    let invalid = brief(&format!("Build design:{invalid}."), "work");
    assert!(invalid.contains("could not be read") && invalid.contains("E7033"), "{invalid}");
    let malformed = brief("Build design:xyz.", "work");
    assert!(malformed.contains("could not be read") && malformed.contains("E7001"), "{malformed}");
    let mut t = api(&s.root, "POST", "/tasks", json!({"title":"Remember"})).unwrap();
    t["body"] = json!(format!("Build design:{ui}."));
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running"}]);
    tasks::put(&s, &t).unwrap();
    let remembered = remember(&scope::top(&p.root), t["id"].as_str().unwrap(), "r1", 10_000).unwrap();
    assert!(remembered.contains(&format!("design:{ui}: Home. Read it with `design show design:{ui}`.")), "{remembered}");
    crate::design::delete(&p.root, ui).unwrap();
    let gone = brief(&format!("Build design:{ui}."), "work");
    assert!(gone.contains("could not be read") && gone.contains("E7023"), "{gone}");
}

#[test]
fn design_links_need_a_word_boundary() {
    let ids = |text: &str| {
        let mut ids = Vec::new();
        scan_design_links(text, &mut ids);
        ids
    };
    assert!(ids("redesign:0123456789abcdef").is_empty());
    assert_eq!(ids("(design:0123456789abcdef)"), ["0123456789abcdef"]);
    assert_eq!(ids("design:0123456789abcdef"), ["0123456789abcdef"]);
    assert_eq!(ids("`design:0123456789abcdef`design:fedcba9876543210"), ["fedcba9876543210"]);
    assert_eq!(ids("design:0123456789abcdef design:0123456789abcdef"), ["0123456789abcdef"]);
}

/// A work or investigation brief asks before a task that clearly contradicts a record.
#[test]
fn work_and_investigation_briefs_check_for_a_clear_contradiction() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    let bar = "If doing it would undo or break what one of them states, not merely differ in detail";
    for kind in ["work", "investigate"] {
        for asking in ["often", "balanced", "rarely"] {
            let b = super::brief(&t, kind, "engineer", "standard", asking);
            assert!(
                b.contains("check the task against the decisions, the vision, its feature and the tasks you can read"),
                "{kind}/{asking}: {b}"
            );
            assert!(b.contains(bar), "{kind}/{asking}: {b}");
            assert!(b.contains("name both sides in the human's terms"), "{kind}/{asking}: {b}");
            for option in ["go ahead", "change the task", "drop it"] {
                assert!(b.contains(option), "{kind}/{asking} {option}: {b}");
            }
        }
    }
    for kind in ["review", "discuss"] {
        for asking in ["often", "balanced", "rarely"] {
            let b = super::brief(&t, kind, "engineer", "standard", asking);
            assert!(!b.contains(bar), "{kind}/{asking}: {b}");
            assert!(!b.contains("check the task against the decisions"), "{kind}/{asking}: {b}");
        }
    }
}

#[test]
fn the_work_brief_has_the_naming_section() {
    let t = json!({
        "id": "t1",
        "title": "Add coolcalc",
        "body": "A CLI calculator.",
        "notes": [],
        "questions": [],
        "links": [],
        "changes": [],
    });
    let golden = concat!(
        "Naming: names are codebase vocabulary, not local descriptions; every name tells readers whether two things are the same concept or different ones, so it must tell the truth. ",
        "Scale effort to scope: local variables need little thought; exported types, modules, APIs and schemas need these rules. ",
        "Same concept, same name: search for the concept and its synonyms in code, tests and docs before naming; reuse established terms and patterns, and let existing usage beat style preferences. ",
        "Different concept, different name: don't reuse a name for something that differs from what it already means, and don't introduce a name (`UserRecord` beside `User`) unless it marks a real domain distinction. ",
        "New concepts take domain words from the user's request, spec or domain rather than invented terminology. ",
        "Name what it is, not its history or implementation: prefer `getSession()` over `getUserFromRedisCache()`, and don't use `V2`, `new`, `improved`, `legacy` or task-derived names as substitutes for meaningful names; use them only when the distinction is itself part of the concept or interface. ",
        "Use the simplest unambiguous name: longer is not more precise, and avoid `Manager` or `Processor` when a meaningful term exists. ",
        "Existing names: if the codebase has competing terms, first check whether they mark a real distinction; if not, follow the nearest established usage rather than introducing another synonym. ",
        "Don't rename unless the existing name is misleading or the task requires it. When you rename, update code, tests and docs together. ",
        "Renaming public or persisted names (API fields, schema columns, config keys, serialized names, event names) is a breaking change; treat it as one. ",
        "In your summary, note any new terms you introduced and inconsistencies you found."
    );
    let work = brief(&t, "work", "engineer");
    assert_eq!(work.matches(golden).count(), 1, "work: naming section must appear exactly once\n{work}");
    for kind in ["investigate", "review", "discuss"] {
        let prompt = brief(&t, kind, "engineer");
        assert!(!prompt.contains("Same concept, same name"), "{kind}: unexpected naming guidance\n{prompt}");
        assert!(!prompt.contains("breaking change"), "{kind}: unexpected naming guidance\n{prompt}");
    }
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let s = open(&p.root).unwrap();
    let task = api(&s.root, "POST", "/tasks", json!({"title": "A naming task"})).unwrap();
    let fresh = continuation_brief(&task, &json!({"id": "r1", "kind": "work"}), false, &p.root, &p.root, false, "engineer").0;
    assert!(!fresh.contains("clear names for new symbols"), "fresh work brief: the old clear-name rule remains\n{fresh}");
    assert_eq!(fresh.matches(golden).count(), 1, "fresh work brief: naming section must appear exactly once");
}

/// D-260: a note's named code leads an ordinary task's first call.
#[test]
fn a_notes_named_code_leads_an_ordinary_tasks_first_call() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n\npub fn other() -> Int {\n  2\n}\n")]);
    let t = json!({
        "id": "t1",
        "title": "Ordinary",
        "body": "A normal description.",
        "notes": [{"author": "agent", "text": "`app.other` handles it."}],
    });
    let ranked = task_text(&t);
    assert!(ranked.contains("app.other"), "{ranked}");
    let reply = session_reply(&t, &p.root).expect("a reply");
    let next = reply.lines().find(|l| l.starts_with("Next:")).expect(&reply);
    assert!(next.contains("app.other"), "{next}");
}

/// D-260: on a Tech Debt task, a note's named code outranks a title that
/// names something else. The opaque symbol matches the title, so the test
/// fails without the note.
#[test]
fn a_tech_debt_tasks_notes_lead_its_first_call_past_a_misleading_title() {
    let p = testing::project(&[
        ("src/reach/flow.gleam", "pub fn taint() -> Int {\n  1\n}\n"),
        ("src/opaque.gleam", "pub fn opaque_type() -> Int {\n  2\n}\n"),
    ]);
    let t = json!({
        "id": "t1",
        "title": "notes: keep note box contents opaque",
        "body": crate::board::reviews::TECH_DEBT_BODY,
        "techDebt": true,
        "notes": [{
            "author": "agent",
            "text": "The fix lives in `reach::flow.taint`.",
        }],
    });
    let reply = session_reply(&t, &p.root).expect("a reply");
    let next = reply.lines().find(|l| l.starts_with("Next:")).expect(&reply);
    assert!(next.contains("flow.taint"), "{next}");
}

