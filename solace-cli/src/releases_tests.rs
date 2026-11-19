use super::*;



const A: u64 = 1_700_000_000;

const B: u64 = 1_700_100_000;

fn release(top: &Path, version: &str, time: u64) {
    let dir = top.join(folder(top)).join(version);
    std::fs::create_dir_all(&dir).unwrap();
    let record = json!({"version": version, "time": time, "artifacts": []});
    std::fs::write(dir.join("release.json"), record.to_string()).unwrap();
}

fn feature(top: &Path, id: &str, title: &str) {
    crate::tasks::store::put_record(top, "features", &json!({"id": id, "title": title, "state": "complete", "rank": 1.})).unwrap();
}

fn task(store: &crate::Store, title: &str, feature: Option<&str>, closed: u64) -> String {
    let mut t = crate::board::task_model::new_task(title, "", &json!("complete"), 1.);
    t["closedTime"] = json!(closed);
    t["feature"] = feature.map_or(Value::Null, |id| json!(id));
    crate::tasks::store::put(store, &t).unwrap();
    t["id"].as_str().unwrap().to_string()
}

fn setup() -> crate::testing::Project {
    crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")])
}

#[test]
fn a_release_draft_covers_only_work_closed_since_the_release_before_it() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    release(&top, "2.0.0", B);
    task(&p.store, "Before the first release", None, A - 10);
    let inside = task(&p.store, "Between the releases", None, A + 1);
    task(&p.store, "After the second release", None, B + 10);
    let text = draft(&top, "2.0.0").unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("Between the releases"), "{text}");
    assert!(!text.contains("Before the first release"), "{text}");
    assert!(!text.contains("After the second release"), "{text}");
    assert!(text.contains(&format!("task:{inside}")), "{text}");
}

#[test]
fn the_oldest_release_covers_everything_closed_before_it() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", B);
    task(&p.store, "Before the only release", None, A);
    task(&p.store, "After the only release", None, B + 10);
    let text = draft(&top, "1.0.0").unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("Before the only release"), "{text}");
    assert!(!text.contains("After the only release"), "{text}");
}

#[test]
fn a_draft_groups_tasks_under_their_feature_and_links_each_task() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    feature(&top, "aaaaaaaaaaaaaaaa", "Checkout");
    release(&top, "1.0.0", B);
    let in_feature = task(&p.store, "Pay with a card", Some("aaaaaaaaaaaaaaaa"), A);
    let loose = task(&p.store, "Fix a typo", None, A + 1);
    let text = draft(&top, "1.0.0").unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("## Checkout"), "{text}");
    assert!(text.contains("## Other work"), "{text}");
    assert!(text.contains(&format!("[Pay with a card](task:{in_feature})")), "{text}");
    assert!(text.contains(&format!("[Fix a typo](task:{loose})")), "{text}");
}

#[test]
fn a_draft_covers_decisions_made_since_the_previous_release() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    release(&top, "2.0.0", B);
    task(&p.store, "Ship checkout", None, A + 1);
    let date = crate::decisions::date_of(B);
    let before = crate::decisions::date_of(A - 86_400);
    std::fs::write(
        crate::decisions::file(&top),
        format!(
            "## D-1 From the task\n(human, board task \"Ship checkout\", {date})\nIt is done.\n\n\
             ## D-2 From the page\n(human, strategy run on the Strategy page, {date})\nIt is decided.\n\n\
             ## D-3 Before\n(human, strategy run on the Strategy page, {before})\nOld.\n"
        ),
    )
    .unwrap();
    let text = draft(&top, "2.0.0").unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("D-1: From the task"), "{text}");
    assert!(text.contains("D-2: From the page"), "{text}");
    assert!(!text.contains("D-3"), "{text}");
}

#[test]
fn listing_releases_starts_no_notes_run_and_saves_no_draft() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    let listed = listed(&top).unwrap();
    assert_eq!(listed["releases"][0]["notes"], Value::Null);
    assert!(crate::tasks::strategy::notes_runs(&top).is_empty());
    assert!(saved(&top, "1.0.0").is_none());
}

#[test]
fn a_saved_draft_is_listed_with_its_release() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    let notes = json!({"version": "1.0.0", "state": "draft", "text": "- [One](task:0123456789abcdef)"});
    save(&top, "1.0.0", &notes).unwrap();
    let listed = listed(&top).unwrap();
    assert_eq!(listed["releases"][0]["notes"]["text"], notes["text"]);
}

#[test]
fn a_reopened_task_is_left_out_of_the_draft() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    release(&top, "2.0.0", B);
    let w = crate::board::task_model::flow(&top).unwrap();
    let open = w["columns"].as_array().unwrap().iter().find(|c| c["final"] != true).unwrap()["id"].clone();
    let mut t = crate::board::task_model::new_task("Reopened after closing", "", &open, 1.);
    t["closedTime"] = json!(A + 1);
    crate::tasks::store::put(&p.store, &t).unwrap();
    let text = draft(&top, "2.0.0").unwrap()["text"].as_str().unwrap().to_string();
    assert!(!text.contains("Reopened after closing"), "{text}");
}

#[test]
fn asking_again_while_a_notes_run_is_open_keeps_the_saved_draft() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    let dir = top.join(folder(&top)).join("1.0.0");
    std::fs::write(dir.join("notes.json"), json!({"version":"1.0.0","state":"draft","text":"kept"}).to_string()).unwrap();
    let runs = top.join(".solace/release-notes");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::write(
        runs.join("open.json"),
        json!({
            "id": "open",
            "kind": crate::board::roles::NOTES_RUN,
            "state": "running",
            "context": {"kind": "release", "id": "1.0.0"},
            "notes": [],
            "questions": []
        })
        .to_string(),
    )
    .unwrap();
    let e = crate::releases::ask(&top, "1.0.0").unwrap_err();
    assert!(e.0.contains("E7003"), "{}", e.0);
    assert_eq!(crate::releases::saved(&top, "1.0.0").unwrap()["text"], "kept");
}

#[test]
fn asking_for_notes_after_approval_keeps_the_approved_notes() {
    let p = setup();
    let top = crate::scope::top(&p.root);
    release(&top, "1.0.0", A);
    let dir = top.join(folder(&top)).join("1.0.0");
    std::fs::write(dir.join("notes.json"), json!({"version":"1.0.0","state":"draft","text":"old draft"}).to_string()).unwrap();
    crate::releases::approve(&top, "1.0.0", "# Approved").unwrap();
    let e = crate::releases::ask(&top, "1.0.0").unwrap_err();
    assert!(e.0.contains("E7003"), "{}", e.0);
    let notes = crate::releases::saved(&top, "1.0.0").unwrap();
    assert_eq!(notes["text"], "# Approved");
    assert_eq!(notes["state"], "approved");
    assert!(crate::tasks::strategy::notes_runs(&top).is_empty());
}
