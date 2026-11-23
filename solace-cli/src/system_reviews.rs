//! System reviews (D-265): a review run of the whole project for the review
//! kinds the human chose, its findings filed as Planning tasks under the
//! findings feature, and the log, one `SYSTEMREVIEWS.md` at a board's top
//! folder recording each completed review. Agents read the log; only Solace
//! writes it.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::board::task_model::{Column, set_links};
use crate::failure::{Failure, Out, usage};
use crate::fsx;
pub(crate) const FILE: &str = "SYSTEMREVIEWS.md";

const HEADER: &str = concat!(
    "# System reviews\n\n",
    "Each completed review of the whole project: its review kinds, the date, the version reviewed ",
    "and the tasks its findings created.\n\n",
);

/// The log's path at the board's top folder.
pub fn file(top: &Path) -> PathBuf {
    top.join(FILE)
}

/// Creates the log with its header when the board has none; an existing log is kept.
pub(crate) fn ensure(top: &Path) -> Result<(), crate::Failure> {
    let p = file(top);
    if p.exists() {
        return Ok(());
    }
    fs::create_dir_all(top)?;
    fsx::write_atomic(&p, HEADER)?;
    Ok(())
}

/// Adds one review's entry after everything the log already holds.
pub(crate) fn append(top: &Path, entry: &str) -> Result<(), crate::Failure> {
    ensure(top)?;
    let mut text = fs::read_to_string(file(top))?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() && !text.ends_with("\n\n") {
        text.push('\n');
    }
    text += entry.trim_end();
    text.push('\n');
    fsx::write_atomic(&file(top), text.as_bytes())?;
    Ok(())
}

/// The refusal of any agent command that would change the log.
pub(crate) fn refusal<T>(name: &str) -> Result<T, crate::Failure> {
    crate::lockdown::error("E6002", name, "only Solace writes this file: it adds an entry when a system review completes. `read` it")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_goes_after_what_the_log_holds() {
        let root = crate::testing::Scratch::new();
        fs::write(file(&root), "# System reviews\n\n## Earlier\nKept.").unwrap();
        append(&root, "## Security, 2026-11-22\nTasks: t1\n").unwrap();
        assert_eq!(
            fs::read_to_string(file(&root)).unwrap(),
            "# System reviews\n\n## Earlier\nKept.\n\n## Security, 2026-11-22\nTasks: t1\n"
        );
    }

    #[test]
    fn appending_to_a_board_with_no_log_creates_it_first() {
        let root = crate::testing::Scratch::new();
        append(&root, "## Code quality, 2026-11-22").unwrap();
        let text = fs::read_to_string(file(&root)).unwrap();
        assert!(text.starts_with(HEADER) && text.ends_with("\n\n## Code quality, 2026-11-22\n"), "{text}");
    }

    #[test]
    fn ensure_keeps_an_existing_log() {
        let root = crate::testing::Scratch::new();
        fs::write(file(&root), "# System reviews\n\n## Kept\n").unwrap();
        ensure(&root).unwrap();
        assert_eq!(fs::read_to_string(file(&root)).unwrap(), "# System reviews\n\n## Kept\n");
    }

    /// A project at the root, with nothing reviewed yet.
    fn reviewed() -> crate::testing::Project {
        crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")])
    }

    /// A new system review run of `kinds` on a fake reviewer model, its agent not started.
    fn review(top: &Path, kinds: &[&str]) -> String {
        let config = top.join("review-config.toml");
        fs::write(
        &config,
        "[[models]]\nid = \"fake\"\nlabel = \"Fake\"\nagent = \"codex\"\nrun = [\"sh\"]\n\n[role_models.reviewer]\ndefault = \"fake\"\n",
    )
    .unwrap();
        let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
        let run = crate::board::setup::with_config(&config, || begin(top, &kinds).unwrap());
        run["id"].as_str().unwrap().to_string()
    }

    /// The agent's `review findings` in run `rid`.
    fn report(top: &Path, rid: &str, findings: Value) -> Out {
        let mut input = move || -> Out { Ok(findings.to_string()) };
        command(top, rid, &["review", "findings"], &mut input)
    }

    /// The tasks run `rid` filed, in the order it lists them.
    fn findings(top: &Path, rid: &str) -> Vec<Value> {
        let run = crate::tasks::strategy::strategy_run(top, rid).unwrap();
        let all = crate::board::task_model::all_tasks(top).unwrap();
        run["findings"].as_array().unwrap().iter().map(|id| all.iter().find(|t| t["id"] == *id).unwrap().clone()).collect()
    }

    #[test]
    fn findings_land_in_planning_under_the_findings_feature_and_are_logged() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let rid = review(&top, &["security", "code-quality"]);
        let out = report(
            &top,
            &rid,
            json!([
                {"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."},
                {"kind": "code-quality", "title": "Split the server", "body": "`server.api` is too long.", "scope": ["."]}
            ]),
        )
        .unwrap();
        let tasks = findings(&top, &rid);
        let feature = crate::tasks::records(&top, "features").into_iter().find(|f| f["findings"] == true).unwrap();
        assert_eq!(feature["state"], "active");
        let titles: Vec<&str> = tasks.iter().map(|t| t["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Check the token", "Split the server"]);
        for t in &tasks {
            assert_eq!((t["column"].as_str(), &t["feature"]), (Some("planning"), &feature["id"]));
            assert!(out.contains(&format!("task:{}", t["id"].as_str().unwrap())), "{out}");
        }
        assert!(tasks[0]["notes"].to_string().contains(&format!("Found by the security review of system review run {rid}")));
        let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
        assert_eq!((run["state"].as_str(), run["outcome"].as_str()), (Some("ended"), Some("finished")));
        let logged = format!(
            "## {}: Security, Code quality\n\n- Version: 1.0.0\n- Findings: task:{}, task:{}\n",
            crate::decisions::today(),
            tasks[0]["id"].as_str().unwrap(),
            tasks[1]["id"].as_str().unwrap()
        );
        let log = fs::read_to_string(file(&top)).unwrap();
        assert!(log.ends_with(&logged), "{log}");
    }

    /// D-128 point 3 and D-134: a completed findings feature takes the next
    /// review's findings, in Planning, and is active again.
    #[test]
    fn findings_after_the_feature_completed_go_to_it_and_make_it_active() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let mut done = crate::tasks::strategy_api(&top, "POST", &["features"], json!({"title": "Findings", "state": "complete"})).unwrap();
        done["findings"] = json!(true);
        crate::tasks::store::put_record(&top, "features", &done).unwrap();
        let rid = review(&top, &["user-experience"]);
        report(&top, &rid, json!([{"kind": "user-experience", "title": "Name the button", "body": "The Save button has no label."}]))
            .unwrap();
        let tasks = findings(&top, &rid);
        assert_eq!((tasks[0]["feature"].clone(), tasks[0]["column"].as_str()), (done["id"].clone(), Some("planning")));
        assert_eq!(crate::tasks::record(&top, "features", done["id"].as_str().unwrap()).unwrap()["state"], "active");
    }

    #[test]
    fn a_canceled_findings_feature_is_not_reused() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let mut gone = crate::tasks::strategy_api(&top, "POST", &["features"], json!({"title": "Findings", "state": "canceled"})).unwrap();
        gone["findings"] = json!(true);
        crate::tasks::store::put_record(&top, "features", &gone).unwrap();
        let rid = review(&top, &["security"]);
        report(&top, &rid, json!([{"kind": "security", "title": "Check the token", "body": "Any token passes."}])).unwrap();
        let feature = crate::tasks::record(&top, "features", findings(&top, &rid)[0]["feature"].as_str().unwrap()).unwrap();
        assert_ne!(feature["id"], gone["id"]);
        assert_eq!((feature["findings"].as_bool(), feature["state"].as_str()), (Some(true), Some("active")));
    }

    #[test]
    fn a_review_with_no_findings_is_logged_with_none() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let rid = review(&top, &["security"]);
        let out = report(&top, &rid, json!([])).unwrap();
        assert!(out.starts_with("No findings were filed."), "{out}");
        assert!(crate::tasks::records(&top, "features").iter().all(|f| f["findings"] != true), "no feature without findings");
        let log = fs::read_to_string(file(&top)).unwrap();
        assert!(log.ends_with(&format!("## {}: Security\n\n- Version: 1.0.0\n- Findings: none\n", crate::decisions::today())), "{log}");
        assert_eq!(crate::tasks::strategy::strategy_run(&top, &rid).unwrap()["state"], "ended");
    }

    /// One finding outside the review's kinds, or without a body, files none of
    /// them and writes no entry, so the agent can resend.
    #[test]
    fn a_wrong_finding_files_nothing_and_keeps_the_run_open() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let rid = review(&top, &["security"]);
        let before = crate::board::task_model::all_tasks(&top).unwrap().len();
        for findings in [
            json!([
                {"kind": "security", "title": "Fine", "body": "Fine."},
                {"kind": "user-experience", "title": "Other", "body": "Other."}
            ]),
            json!([{"kind": "security", "title": "Fine", "body": " "}]),
            json!([{"kind": "security", "title": "Fine", "body": "Fine.", "scope": ["nowhere"]}]),
            json!({"kind": "security"}),
        ] {
            let e = report(&top, &rid, findings).unwrap_err();
            assert!(e.0.starts_with("E7001"), "{}", e.0);
        }
        assert_eq!(crate::board::task_model::all_tasks(&top).unwrap().len(), before);
        assert!(!file(&top).exists() || !fs::read_to_string(file(&top)).unwrap().contains("Findings:"));
        assert_eq!(crate::tasks::strategy::strategy_run(&top, &rid).unwrap()["state"], "running");
    }

    #[test]
    fn a_review_records_its_kinds_once_and_the_version_it_reviews() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        fs::write(top.join("gleam.toml"), "name = \"app\"\nversion = \"1.2.0\"\n").unwrap();
        let kinds = requested_kinds(&json!({"kinds": ["security", "security", "code-quality"]})).unwrap();
        assert_eq!(kinds, ["security", "code-quality"]);
        let rid = review(&top, &["security", "code-quality"]);
        let run = crate::tasks::strategy::strategy_run(&top, &rid).unwrap();
        assert_eq!(run["kind"], crate::board::roles::SYSTEM_REVIEW_RUN);
        assert_eq!(
            (run["context"]["kinds"].clone(), run["context"]["version"].as_str()),
            (json!(["security", "code-quality"]), Some("1.2.0"))
        );
        assert!(top.join(format!(".solace/system-reviews/{rid}.json")).exists());
        let b = brief(&top, &run, "engineer", "", "");
        assert!(b.contains("for: Security, Code quality") && b.contains("one of security, code-quality"), "{b}");
        assert!(b.contains(crate::board::briefs::FINDINGS_ASK) && b.contains("`review findings`"), "{b}");
    }

    #[test]
    fn an_entry_without_a_version_says_none_is_named() {
        let root = crate::testing::Scratch::new();
        assert_eq!(reviewed_version(&root), None);
        let kinds = vec!["user-experience".to_string()];
        assert_eq!(entry("2026-11-22", &kinds, None, &[]), "## 2026-11-22: User experience\n\n- Version: none named\n- Findings: none\n");
    }

    #[test]
    fn the_kinds_the_page_sends_are_known() {
        // The System Review page's start body, as `sysreview.start_body` writes it for every kind.
        let body = serde_json::json!({"kinds": ["security", "code-quality", "user-experience"]});
        assert_eq!(requested_kinds(&body).unwrap(), ["security", "code-quality", "user-experience"]);
    }

    #[test]
    fn past_reviews_are_the_finished_ones_newest_first() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let at = |rid: &str, started: u64, ended: u64| {
            let mut run = crate::tasks::strategy::strategy_run(&top, rid).unwrap();
            run["started"] = json!(started);
            run["ended"] = json!(ended);
            crate::tasks::strategy::put_run(&top, &run).unwrap();
        };
        let older = review(&top, &["security"]);
        report(&top, &older, json!([])).unwrap();
        at(&older, 100, 150);
        let newer = review(&top, &["security", "code-quality"]);
        report(&top, &newer, json!([{"kind": "security", "title": "Check the token", "body": "`auth.check` trusts any token."}])).unwrap();
        at(&newer, 200, 260);
        review(&top, &["user-experience"]);
        let board = std::sync::Arc::new(crate::board::server::Board::new(top.clone(), 0));
        let listed = board.projects_api("GET", "/system-reviews", &json!(null)).unwrap();
        let task = findings(&top, &newer)[0]["id"].clone();
        assert_eq!(
            listed,
            json!([
                {"id": newer, "started": 200, "ended": 260, "kinds": ["security", "code-quality"], "version": "1.0.0", "findings": [task]},
                {"id": older, "started": 100, "ended": 150, "kinds": ["security"], "version": "1.0.0", "findings": []}
            ])
        );
    }

    #[test]
    fn the_log_reads_through_its_route() {
        let p = reviewed();
        let top = crate::scope::top(&p.root);
        let rid = review(&top, &["security"]);
        report(&top, &rid, json!([])).unwrap();
        let board = std::sync::Arc::new(crate::board::server::Board::new(top.clone(), 0));
        let log = board.projects_api("GET", "/system-reviews/log", &json!(null)).unwrap();
        let text = log["text"].as_str().unwrap();
        assert_eq!(text, std::fs::read_to_string(file(&top)).unwrap());
        assert!(text.starts_with("# System reviews\n"), "{text}");
        assert!(text.contains(": Security\n\n- Version: 1.0.0\n- Findings: none\n"), "{text}");
    }
}

/// The kinds a system review covers: the id a request names, and the name an
/// entry and a brief show.
pub(crate) const KINDS: [(&str, &str); 3] =
    [("security", "Security"), ("code-quality", "Code quality"), ("user-experience", "User experience")];

const FINDINGS_FORMAT: &str = "E7001 `review findings` takes the findings in `text` as a JSON list, `[]` for none";

const FINDINGS_TITLE: &str = "System review findings";

const FINDINGS_BODY: &str =
    "What system reviews found in the project, one task per finding. `SYSTEMREVIEWS.md` lists each review and its tasks.";

/// A review kind's name, when `id` names one.
fn kind_name(id: &str) -> Option<&'static str> {
    KINDS.iter().find(|(kind, _)| *kind == id).map(|(_, name)| *name)
}

/// The names of `kinds`, in order.
fn kind_names(kinds: &[String]) -> String {
    kinds.iter().filter_map(|k| kind_name(k)).collect::<Vec<_>>().join(", ")
}

/// The review kinds a start request names, each once; none, or one that is
/// not a review kind, is refused.
pub(crate) fn requested_kinds(body: &Value) -> Result<Vec<String>, Failure> {
    let known = KINDS.iter().map(|(kind, _)| *kind).collect::<Vec<_>>().join(", ");
    let Some(list) = body["kinds"].as_array().filter(|list| !list.is_empty()) else {
        return Err(Failure(format!("E7001 a system review needs `kinds`, a list of one or more of: {known}"), 1));
    };
    let mut kinds: Vec<String> = vec![];
    for k in list {
        let Some(id) = k.as_str().filter(|id| kind_name(id).is_some()) else {
            return Err(Failure(format!("E7001 {k} is not a review kind; the kinds are: {known}"), 1));
        };
        if !kinds.iter().any(|x| x == id) {
            kinds.push(id.to_string());
        }
    }
    Ok(kinds)
}

/// Starts a system review of the kinds `body` names, its agent on its own thread.
pub(crate) fn start(top: &Path, body: &Value) -> Result<Value, Failure> {
    let run = begin(top, &requested_kinds(body)?)?;
    crate::board::runs::start_system_review(top.to_path_buf(), run["id"].as_str().unwrap_or("").to_string());
    Ok(run)
}

/// Records a system review run of `kinds` with the version it reviews; its
/// agent is launched apart.
fn begin(top: &Path, kinds: &[String]) -> Result<Value, Failure> {
    let context = json!({"kind": "project", "kinds": kinds, "version": reviewed_version(top)});
    let message = format!("Review the whole project for: {}.", kind_names(kinds));
    crate::tasks::strategy::start_workspace_run(top, crate::tasks::strategy::SYSTEM_REVIEW_FAMILY, context, &message)
}

/// The version the workspace's manifests hold, every one of them when they differ.
fn reviewed_version(top: &Path) -> Option<String> {
    let found = crate::versions::json(top).ok()?;
    let mut versions: Vec<String> = vec![];
    for v in found["versions"].as_array()?.iter().filter_map(|v| v["version"].as_str()) {
        if !versions.iter().any(|x| x == v) {
            versions.push(v.to_string());
        }
    }
    (!versions.is_empty()).then(|| versions.join(", "))
}

/// The review kinds of run `run`.
fn run_kinds(run: &Value) -> Vec<String> {
    run["context"]["kinds"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// A system review run's prompt: its kinds, the projects to read, and how to report.
pub(crate) fn brief(top: &Path, run: &Value, _involvement: &str, _detail: &str, _asking: &str) -> String {
    let kinds = run_kinds(run);
    let projects = crate::workspace::members(top).unwrap_or_default().join(", ");
    format!(
        concat!(
            "You are running a system review of the whole project for: {names}. ",
            "You work only through the Solace tool: there is no shell and no file access, and this run changes no code.\n",
            "Read every project ({projects}) with `view`, `inspect`, `refs`, `find`, `grep`, `files` and `read` ",
            "(`--project <folder>` for a project below the root), and `read DECISIONS.md` for what the human decided. ",
            "Report the real problems of these kinds in the project as it is now, each one work that a single task can do. ",
            "{ask}\n",
            "Finish with `review findings`, alone in its call, the findings in `text` as a JSON list of one object per finding: ",
            "`kind` (one of {ids}), `title` (the work in a few words), `body` (Markdown) and, when you know them, ",
            "`scope` (the project folders it changes). Send `[]` when there are none. ",
            "Each finding becomes a task in Planning, and the run ends.\n"
        ),
        names = kind_names(&kinds),
        projects = projects,
        ask = crate::board::briefs::FINDINGS_ASK,
        ids = kinds.join(", "),
    )
}

/// `review findings` in system review run `rid`, which its guard let act: the
/// findings are checked here, then filed by the task server.
pub(crate) fn command(top: &Path, rid: &str, a: &[&str], input: &mut dyn FnMut() -> Out) -> Out {
    if !matches!(a, ["review", "findings"]) {
        return usage("review findings, with the findings as a JSON list in `text`");
    }
    let Some(run) = crate::tasks::strategy::strategy_run(top, rid) else {
        return Err(Failure(format!("E7023 unknown system review run {rid}"), 1));
    };
    let findings: Value = serde_json::from_str(&input()?).map_err(|_| Failure(FINDINGS_FORMAT.into(), 1))?;
    finding_tasks(top, &run, &findings)?;
    crate::tasks::deliver(top, json!({"kind": "system_review_findings", "run": rid, "findings": findings}))?;
    let filed = crate::tasks::strategy::strategy_run(top, rid).map(|r| r["findings"].clone()).unwrap_or(Value::Null);
    let ids: Vec<String> = filed.as_array().into_iter().flatten().filter_map(Value::as_str).map(|id| format!("task:{id}")).collect();
    Ok(match ids.len() {
        0 => "No findings were filed. The review is logged in SYSTEMREVIEWS.md, and the run has ended.\n".to_string(),
        n => format!(
            "Filed {n} findings in Planning: {}. The review is logged in SYSTEMREVIEWS.md, and the run has ended.\n",
            ids.join(", ")
        ),
    })
}

/// The Planning tasks review run `run`'s findings make, checked and not yet
/// written: each finding is of one of the run's kinds, with a title and a body.
fn finding_tasks(top: &Path, run: &Value, findings: &Value) -> Result<Vec<Value>, Failure> {
    let Some(findings) = findings.as_array() else { return Err(Failure(FINDINGS_FORMAT.into(), 1)) };
    let kinds = run_kinds(run);
    let rid = run["id"].as_str().unwrap_or("");
    let mut tasks = vec![];
    for (i, f) in findings.iter().enumerate() {
        let n = i + 1;
        let Some(kind) = f["kind"].as_str().filter(|k| kinds.iter().any(|x| x == k)) else {
            return Err(Failure(format!("E7001 finding {n}: `kind` must be one of this review's kinds: {}", kinds.join(", ")), 1));
        };
        if !f["body"].as_str().is_some_and(|b| !b.trim().is_empty()) {
            return Err(Failure(format!("E7001 finding {n}: `body` is required"), 1));
        }
        let mut fields = json!({"title": f["title"], "body": f["body"]});
        if let Some(scope) = f.get("scope") {
            fields["scope"] = scope.clone();
        }
        let origin = format!("Found by the {} review of system review run {rid}", kind_name(kind).unwrap_or(kind).to_lowercase());
        let mut t = crate::tasks::strategy::proposed_task(top, &origin, &fields, Column::Planning)
            .map_err(|e| Failure(format!("{} (finding {n})", e.0), e.1))?;
        t["rank"] = json!(t["rank"].as_f64().unwrap_or(0.) + i as f64);
        tasks.push(t);
    }
    Ok(tasks)
}

/// Files review run `rid`'s findings as Planning tasks under the findings
/// feature, all or none, ends the run and logs the review (D-265 point 3).
/// The task server applies it; a run that has ended was filed already.
pub(crate) fn finish(top: &Path, rid: &str, findings: &Value) -> Result<(), Failure> {
    let Some(mut run) = crate::tasks::strategy::strategy_run(top, rid) else {
        return Err(Failure(format!("E7023 unknown system review run {rid}"), 1));
    };
    if run["state"] == "ended" {
        return Ok(());
    }
    let mut tasks = finding_tasks(top, &run, findings)?;
    let batch = crate::tasks::store::batch(top)?;
    let written: Result<(), Failure> = (|| {
        if tasks.is_empty() {
            return Ok(());
        }
        let feature = findings_feature(top)?;
        for t in &mut tasks {
            set_links(top, t, &json!({"feature": feature["id"]}))?;
            crate::tasks::store::write(top, t)?;
        }
        Ok(())
    })();
    match written {
        Ok(()) => crate::tasks::store::batch_commit(batch)?,
        Err(e) => {
            crate::tasks::store::batch_rollback(batch);
            return Err(e);
        }
    }
    let ids: Vec<String> = tasks.iter().filter_map(|t| t["id"].as_str().map(String::from)).collect();
    run["findings"] = json!(ids);
    crate::tasks::strategy::end_strategy(&mut run, "finished");
    crate::tasks::strategy::put_run(top, &run)?;
    append(top, &entry(&crate::decisions::today(), &run_kinds(&run), run["context"]["version"].as_str(), &ids))
}

/// The standing findings feature: the one marked `findings` that is active or
/// complete, else a new active one. A feature in another state keeps its tasks
/// off the board (D-128 point 4), so it is not reused.
fn findings_feature(top: &Path) -> Result<Value, Failure> {
    let standing = crate::tasks::records(top, "features")
        .into_iter()
        .find(|f| f["findings"] == true && matches!(f["state"].as_str(), Some("active" | "complete")));
    if let Some(f) = standing {
        return Ok(f);
    }
    let new = json!({"title": FINDINGS_TITLE, "body": FINDINGS_BODY, "state": "active"});
    let mut f = crate::tasks::strategy_api(top, "POST", &["features"], new).map_err(|(_, e)| e)?;
    f["findings"] = json!(true);
    crate::tasks::store::put_record(top, "features", &f)?;
    Ok(f)
}

/// A review's entry in the log: its date and kinds, the version reviewed and
/// the tasks its findings made.
fn entry(date: &str, kinds: &[String], version: Option<&str>, tasks: &[String]) -> String {
    let found =
        if tasks.is_empty() { "none".to_string() } else { tasks.iter().map(|t| format!("task:{t}")).collect::<Vec<_>>().join(", ") };
    format!("## {date}: {}\n\n- Version: {}\n- Findings: {found}\n", kind_names(kinds), version.unwrap_or("none named"))
}

/// Every completed review, newest first: when its run started and ended, its
/// kinds, the version reviewed and its finding tasks. `GET /system-reviews`.
pub(crate) fn past(top: &Path) -> Value {
    let mut reviews: Vec<Value> = crate::tasks::strategy::runs_list(top, crate::board::roles::SYSTEM_REVIEW_RUN)
        .into_iter()
        .filter(|run| run["state"] == "ended" && run["outcome"] == "finished")
        .map(|run| {
            json!({"id": run["id"], "started": run["started"], "ended": run["ended"], "kinds": run_kinds(&run),
                "version": run["context"]["version"], "findings": run["findings"].as_array().cloned().unwrap_or_default()})
        })
        .collect();
    reviews.reverse();
    json!(reviews)
}

/// The log's text, for the System Review page to show. `GET /system-reviews/log`.
pub(crate) fn log(top: &Path) -> Result<Value, Failure> {
    Ok(json!({"text": fs::read_to_string(file(top))?}))
}
