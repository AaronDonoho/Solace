use serde_json::Value;
use serde_json::json;
use std::path::PathBuf;
use std::path::Path;
use std::collections::HashSet;
use std::fs;
use crate::board::task_model::open;
use crate::board::runs::last_words;
use crate::board::runs::text;
use crate::board::reviews::changes_of;
use crate::board::reviews::review_state;
use crate::board::reviews::previous_review;
use crate::board::reviews::review_checkpoint;
use crate::board::reviews::task_diff;
use crate::board::runs::transcript;
use crate::board::task_model::run_at;
use crate::Store;
use crate::failure::Out;
#[cfg(test)]
#[path = "briefs_tests.rs"]
mod tests;
use crate::scope;
use crate::drafts;
use crate::skills;
use crate::lockdown;
use crate::board::task_model::all_tasks;
use crate::board::task_model::flow;
use crate::board::task_model::task_line;
use crate::tasks;
use crate::board::err;
use crate::board::task_model::task;
use crate::board::task_model::column_name;
use crate::failure::Failure;
use crate::failure::usage;
use crate::board::reviews::scope_stores;
use crate::board::task_model::task_dir;
use crate::sha256;
use crate::board::runs::run_reads;
use crate::board::runs::run_outline;
use crate::board::runs::run_record;
use crate::board::task_model::task_path;
use crate::board::task_model::read_task;
use crate::reads;

/// Follow-up review context comes from live notes joined to their runs, never transcripts.
fn review_context(s: &Store, t: &Value) -> Out {
    let mut out = String::from("\nReview context:\n");
    if let Some(previous) = previous_review(t) {
        let rid = previous["id"].as_str().unwrap();
        out += &format!("Previous completed review: {rid}\n");
        let feedback: Vec<_> = t["notes"].as_array().unwrap().iter().filter(|n| n["run"] == rid && n["author"] == "agent").collect();
        if feedback.is_empty() {
            out += "Previous feedback was deleted or is unavailable; do not infer its assessment.\n";
        }
        for n in feedback {
            out += &format!("Review feedback ({rid}): {}\n", n["text"].as_str().unwrap_or(""));
        }
        let at = run_at(t, rid)?;
        for n in t["notes"].as_array().unwrap() {
            let subsequent = n["run"].as_str().and_then(|id| run_at(t, id).ok()).filter(|i| *i > at);
            if let Some(i) = subsequent {
                out += &format!(
                    "Subsequent {} ({}) — {}: {}\n",
                    t["runs"][i]["kind"].as_str().unwrap_or("run"),
                    t["runs"][i]["id"].as_str().unwrap(),
                    n["author"].as_str().unwrap_or(""),
                    n["text"].as_str().unwrap_or("")
                );
            } else if n["author"] == "human" {
                out += &format!("Human feedback: {}\n", n["text"].as_str().unwrap_or(""));
            }
        }
    }
    match review_checkpoint(s, t) {
        Ok(checkpoint) => {
            out += "The task's changes are below: the delta since the last review. Assess the delta, prior unresolved findings and affected dependencies. Run `task diff` only when the human asks for a fresh full review, or for parts left out below.\n";
            let current = review_state(s, t)?;
            if checkpoint["changed_during_review"] != false {
                out += "Project or instructions changed during the previous review, or stability is unknown. Its checkpoint represents review-start content only; recheck affected code and context before carrying forward any assessment.\n";
            }
            if checkpoint["context"] == current["context"]
                && checkpoint["instructions"] == current["instructions"]
                && checkpoint["changed_during_review"] == false
            {
                out += "Code and relevant context are unchanged since the previous review. A short report carrying forward the previous assessment is sufficient if its feedback is available. Unchanged code must not turn Needs work into Looks good.\n";
            } else {
                out += "Code or relevant context changed since the previous review. Even an empty task delta does not establish unchanged dependencies or instructions; inspect affected context and rerun relevant verification.\n";
            }
            out += &task_diff(s, t["id"].as_str().unwrap(), &["--since-review"], 8000)?;
        }
        Err(reason) => {
            out += &format!("No usable delta baseline: {reason}. The task's changes are below (the whole task); assess all of them.\n");
            out += &task_diff(s, t["id"].as_str().unwrap(), &[], 8000)?;
        }
    }
    Ok(out)
}

/// The brief, plus the images its notes reference, which are attached to the prompt.
/// `here` is the project the run's agent works in (D-123).
pub(crate) fn continuation_brief(
    t: &Value,
    run: &Value,
    resumed: bool,
    root: &Path,
    here: &Path,
    custom: bool,
    involvement: &str,
    detail: &str,
    asking: &str,
) -> (String, Vec<PathBuf>) {
    let kind = run["kind"].as_str().unwrap_or("");
    let mut packet = t.clone();
    if resumed {
        let seen: HashSet<&str> = t["conversation"]["notes_seen"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        packet["notes"] =
            json!(t["notes"].as_array().unwrap().iter().filter(|n| !seen.contains(note_fingerprint(n).as_str())).collect::<Vec<_>>());
    }
    let tid = t["id"].as_str().unwrap_or("");
    let mut images = vec![];
    for n in packet["notes"].as_array_mut().into_iter().flatten() {
        let text = note_images(n["text"].as_str().unwrap_or(""), &format!("/tasks/{tid}/files/"), &mut images);
        n["text"] = json!(text);
    }
    let images: Vec<PathBuf> = images.iter().filter_map(|name| image_path(root, tid, name).ok()).collect();
    packet["home"] = json!(scope::dir(&scope::top(root), root));
    // The task's changes come from the scope projects' stores (D-77 point 4).
    packet["changes"] = json!(open(root).map(|s| changes_of(&s, &packet).unwrap_or_default()).unwrap_or_default());
    // The brief says where the agent works.
    packet["home"] = json!(scope::dir(&scope::top(root), here));
    // D-182: a work brief names the marked test commands of every project it covers.
    if kind == "work" {
        let (tests, errors) = marked_tests(&scope::top(root), &packet);
        packet["test_commands"] = json!(tests);
        if !errors.is_empty() {
            packet["test_commands_error"] = json!(errors.join("; "));
        }
    }
    let mut out = brief(&packet, kind, involvement, detail, asking);
    out += &plan_sections(&scope::top(root), t);
    if resumed {
        out = out.replace(START, "Use the continuation context below.");
    }
    // D-151: a fresh run's brief holds its session start reply, so its first
    // call reads; the brief has its feature and dependencies already.
    let reply = if resumed || !out.contains(START) { None } else { session_reply(t, here) };
    if reply.is_some() {
        out = out.replace(START, HELD_START);
    }
    if !resumed {
        // Fresh briefs start with the reference.
        out = format!("{}\n\n{out}", crate::reference::agent_reference(true));
    }
    if !matches!(kind, "review" | "discuss") && t["runs"].as_array().is_some_and(|r| r.len() > 1) {
        if resumed {
            out += "\nContinue the author's conversation. The role and permissions above replace those of earlier runs. Skip broad orientation; inspect current versions of symbols you need before editing.\n";
        } else {
            out += "\nFresh conversation: use this handoff and the notes above to continue existing work. Reconcile partial edits before proceeding.\n";
        }
        for r in t["runs"].as_array().unwrap().iter().rev().filter(|r| r["id"] != run["id"]).take(3) {
            out += &format!("Previous {}: {}\n", r["kind"].as_str().unwrap_or("run"), r["outcome"].as_str().unwrap_or("unfinished"));
            if let Some(log) = r["log"].as_str()
                && let Some(said) = last_words(&transcript(Path::new(log)))
            {
                out += &format!("Last activity: {said}\n");
            }
        }
    }
    if let Ok(s) = open(root) {
        if kind == "review" {
            match review_context(&s, t) {
                Ok(context) => out += &context,
                Err(e) => out += &format!("\nReview context unavailable: {}. Run `task diff` to see the whole task and assess it.\n", e.0),
            }
        }
        // A discussion runs no checks, so it gets the decision titles instead of the verification record.
        if kind == "discuss" {
            let decisions = fs::read_to_string(scope::top(root).join("DECISIONS.md")).unwrap_or_default();
            let titles: Vec<String> =
                decisions.lines().filter_map(|l| l.strip_prefix("## ")).filter(|l| l.starts_with("D-")).map(|l| format!("- {l}")).collect();
            if !titles.is_empty() {
                out += &format!("\nDecisions (DECISIONS.md; `grep D-<n>` then `read` for a record):\n{}\n", titles.join("\n"));
            }
        } else if let Some(checks) = t["verification"].as_object() {
            let current = review_state(&s, t).ok().and_then(|v| v["context"].as_str().map(str::to_string));
            out += "\nLast recorded verification (rerun stale checks before claiming success):\n";
            for (command, v) in checks {
                let stale = current.as_deref() != v["base"].as_str();
                out += &format!(
                    "- {command}: {}{}\n{}\n",
                    if v["passed"] == true { "passed" } else { "failed" },
                    if stale { " (stale: project changed)" } else { "" },
                    v["summary"].as_str().unwrap_or("")
                );
            }
        }
        if let Ok(drafts) = task_drafts(&s, t)
            && !drafts.is_empty()
        {
            out += "\nRetained drafts:\n";
            for (dir, id, args, current) in drafts {
                out += &format!("{dir}: {id}: {} ({}): view `view --draft {id} --project {dir}`\n", json!(args), drafts::status(current));
            }
        }
    }
    let workspace = scope::top(root);
    out += &vision_section(&workspace, kind);
    out += &personas_section(&workspace);
    let mut roots: Vec<&Path> = vec![&workspace];
    if here != workspace.as_path() {
        roots.push(here);
    }
    out += &skills::section_for(&roots, Some(here), kind);
    if !images.is_empty() {
        out += "\nImages in the notes are attached to this message as [image 1], [image 2], ... in order.\n";
        if custom {
            for (i, p) in images.iter().enumerate() {
                out += &format!("[image {}: {}]\n", i + 1, p.display());
            }
        }
    }
    if let Some(reply) = reply {
        out += &format!("\nSession start (the reply to `session start --task`):\n{reply}");
    }
    // A discussion's `task note` returns the human's next message, so the run
    // ends with the call that says so, not with the first note.
    out +=
        "\nAfter the call that ends the run returns, stop: no more calls, and a final response of one line at most; then exit normally.\n";
    (out, images)
}

/// The reply `session start --task` gives task `t`'s agent, with `task_text`'s
/// ranking of the task's title, description and, for a Tech Debt task, its
/// findings, in `here`, the project its run works in (D-151, D-228);
/// none when that folder has no store, or when building it fails, which is
/// logged so the brief's plain start line can be traced. The brief carries
/// the reference above this reply.
fn session_reply(t: &Value, here: &Path) -> Option<String> {
    if !here.join(".solace/store.db").exists() {
        return None;
    }
    let task = task_text(t);
    let tid = t["id"].as_str()?;
    match Store::at(here).and_then(|s| s.brief_task_session(task.trim_end(), Some(tid), "")) {
        Ok(reply) => Some(reply),
        Err(e) => {
            eprintln!("session start for the brief of task {tid}: {}", e.0);
            None
        }
    }
}

pub(crate) fn brief(t: &Value, kind: &str, involvement: &str, detail: &str, asking: &str) -> String {
    let start = START;
    let example = if involvement == "director" {
        "give each option a short example of what the human will see happen, not how the code would change"
    } else {
        "give each option a short example of the outcome the human would see (what a user sees, what a command prints, what a page shows, what a file or record ends up as), not how the code would change"
    };
    let ask = format!(
        "If you need a decision from the human, check the Notes so far and Questions below, if any, for what is already answered (a discussion run can't `task ask`, so its answers are notes); else ask with `task ask` (`text` and 2–4 `options`); the call waits for the answer. In the question's `text`, {example}; the `options` stay short labels matching the bold names in the examples. A question carries its own explanation and is the only command in its call: don't add a `task note` beside it. {} ",
        crate::scope::ASK_FOR_PROJECT
    );
    let settings = "To change `solace.toml` or `AGENTS.md`, `write` it alone, with the why in `reason`.";
    let responses = "`text` and `options` are arguments of the Solace tool, beside `command`: in a task message, `command` is just the command (such as `task note`), and the whole message, however long, goes in `text`, never in `command` and never as a `--text` flag. Write human-facing task messages in Markdown, including the text sent to task tools. Divide the body into relevant sections with short headings (for example, ### Findings and ### Verification), with blank lines around headings and lists. Omit irrelevant sections. Keep messages short: lead with the answer or outcome, then only what the human needs to act on; don't restate the task or narrate your steps. When asking a question, explain what you need and why it matters (for example, ### Question and ### Why this matters); in `text`, give each option a short example of its outcome in the human's terms, its name in bold, and keep the `options` labels short, matching the bold names.";
    let read_only = "This run is read-only: reading, `check`, `test`, `build` and `probe` work; `submit`, `delete`, `extract`, `merge`, `write`, `remove` and `format` are refused, and `run <name>` works only for commands marked `allowed_in_reviews` in `solace.toml`.";
    let verify = "Use those to verify (for example test suites).";
    let define = "Probe or run tests only when needed to more thoroughly define the issue.";
    let common = "Code held as symbols is read and cited by name (`lockdown.dispatch`, `checkout.pay`), not by source path: source files are projections written by Solace.";
    let list = |k: &str| t[k].as_array().cloned().unwrap_or_default();
    let home = text(&t["home"]);
    let projects: Vec<String> = list("scope").iter().map(text).collect();
    // D-182: the commands each project marks as tests, so a work brief names
    // them once instead of making the agent guess among every declared command.
    let marked = t["test_commands"].as_array().cloned().unwrap_or_default();
    let tests = if marked.is_empty() {
        String::new()
    } else {
        let suites = marked
            .iter()
            .map(|p| {
                let names: Vec<&str> = p["commands"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let names = names.join(", ");
                if projects.len() > 1 { format!("{}: {names}", text(&p["project"])) } else { names }
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(", and `run <name>` for: {suites}")
    };
    let tests = match t["test_commands_error"].as_str() {
        Some(e) if marked.is_empty() => format!("{tests}; the marked test commands could not be read: {e}"),
        Some(e) => format!("{tests}; the marked test commands could not be read for: {e}"),
        None => tests,
    };
    // Every review asks for detailed findings and files its minor ones as tech debt (D-112, D-198, D-199).
    let debt = format!(" {} {}", FINDINGS_ASK, TECH_DEBT_ASK);
    // A discussion shares the investigator's role but has its own brief.
    let role = match kind {
        "discuss" => discussion_role(read_only, verify),
        _ => match crate::board::roles::for_run(kind).map(|role| role.id) {
            Some("reviewer") => review_role(read_only, verify, &debt),
            Some("worker") => work_role(start, &ask, settings, &tests),
            _ => investigation_role(read_only, define, start, &ask),
        },
    };
    let mut out = format!(
        "You are working on a task from this project's task board. You work only through the Solace tool: there is no shell and no file access.\n{role}\n"
    );
    out += &behavior_sections(involvement, detail, asking, "task note", kind);
    out += &format!("{common}\n{responses}\n\n{}", task_sections(t));
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            out += &format!("\n{title}:\n{}\n", lines.join("\n"));
        }
    };
    // A task across projects (D-61): where the agent works, and how it reaches the others.
    if projects.len() > 1 {
        section(
            "Projects",
            vec![format!(
                "This task covers the projects {}. You work in {home}; for another of them, add `--project <folder>` (its folder as listed, from the Solace root) to any command (`inspect`, `submit`, `test`, `write`...). A project not listed is out of this task's reach.",
                projects.join(", "),
            )],
        );
    }
    let shown = |c: &Value| match c["project"].as_str() {
        Some(p) if projects.len() > 1 => format!("- {p}: {}", text(&c["name"])),
        _ => format!("- {}", text(&c["name"])),
    };
    section("Task changes", list("changes").iter().map(shown).collect());
    out
}

/// The `[commands]` each project of the task marks with `test = true`, in
/// project order and sorted by name (D-182). A project with none is left out;
/// a settings file that cannot be read is named, and the other projects' marks
/// are kept.
fn marked_tests(top: &Path, t: &Value) -> (Vec<Value>, Vec<String>) {
    let home = t["home"].as_str().unwrap_or("");
    let mut found = vec![];
    let mut errors = vec![];
    for dir in scope::of(t, home) {
        let settings = match lockdown::settings(&scope::path(top, &dir)) {
            Ok(settings) => settings,
            Err(e) => {
                errors.push(format!("project {dir}: {}", e.0));
                continue;
            }
        };
        let Some(commands) = settings.get("commands").and_then(|c| c.as_table()) else { continue };
        let mut names: Vec<&str> =
            commands.iter().filter(|(_, c)| c.get("test").and_then(|x| x.as_bool()) == Some(true)).map(|(name, _)| name.as_str()).collect();
        names.sort();
        if !names.is_empty() {
            found.push(json!({"project": dir, "commands": names}));
        }
    }
    (found, errors)
}

/// The review role's direction in a task brief.
fn review_role(read_only: &str, verify: &str, debt: &str) -> String {
    format!(
        "Review the changes this task made and give feedback. Red-team the changes: assume they don't work until you prove they fully meet the need and break nothing they reach. The task, its notes and its changes are all in this brief (the delta since the last review, or the whole task, in the review context below), so don't send `session start --task`. Run `task diff` only when the human asks for a fresh full review, or for parts the brief left out. For each concern, inspect every symbol it needs in one call (`inspect a b c --context`); split only when the reply is trimmed. A call is a turn, so batch the symbols one concern needs; start from the changed symbols the review context names. Keep the concern-by-concern focus. Don't review generated files (build output such as a bundled `priv/app.js`, lockfiles, installed dependencies); test them instead, and rebuild them with the project's commands if a check needs them. Flag code the task didn't need, places it missed, and any validation, data-loss handling or security it removed. Flag duplication and large modules, filing minor ones as tech debt; don't allow cross-domain concerns in the same module, and flag excessively long functions and dead code as tech debt. Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI. Flag user-interface details the task didn't ask for, including labels, wording, indicators, colors, new buttons or screens, and layout. {read_only} {verify} Only a `task note` call records the feedback; a final message is not recorded. If the call is refused, resend it, and don't claim it was recorded unless the reply says `Recorded.` Finish with `task note`, the feedback in `text`. Begin the feedback with exactly one assessment label: **Assessment: Looks good** or **Assessment: Needs work**. Follow it with findings and verification that explain the assessment, including any checks not performed.{debt}"
    )
}

/// The discussion role's direction in a task brief.
fn discussion_role(read_only: &str, verify: &str) -> String {
    format!(
        "Discuss this task with the human, who asks questions and works out ideas with you. Answer from the high-level context below (the task, its notes and changes); read code only when a question needs it, and briefly. {read_only} {verify} The human's message is the last note. Reply only with `task note`, the reply in `text`, never as plain text; the call waits and returns the human's next message. When a call says it reached its wait limit, reply with `task wait`; it sends nothing. Keep replying until a call says the discussion has ended, then stop without further calls."
    )
}

/// The work role's direction in a task brief.
fn work_role(start: &str, ask: &str, settings: &str, tests: &str) -> String {
    format!(
        "Make the change this task describes. {start} {ask} {settings} Send every change with its check in the same call: `submit` then `test`, `write` then `run <name>`. Prove each acceptance criterion through the entry point - for example: a request through the server's routing, not the function behind it; a save confirmed by a fresh read. Before `task done`, after your last change, run `test --all` in each project with symbol-held source{tests}. Fix the code where a test fails because of your changes; in the summary, report failing tests your changes don't affect and any test you could not run. Reuse existing code before adding new code, extracting common patterns. Prefer small, stateless, self-documenting functions; keep modules and functions small and focused around one domain or component. Use clear names for new symbols. Keep thoughts, decisions and directives out of code comments and the UI, and details or decisions of implementation or development out of the UI. Follow the module boundaries the refactor task:2414798176c466e4 creates: put new code in the module that owns its concern. Finish with `task done`, a summary in `text`: what you changed, how you checked it, and, if any, what you skipped and risks the human must know. {WORK_TECH_DEBT_ASK}"
    )
}

/// The investigation role's direction in a task brief.
fn investigation_role(read_only: &str, define: &str, start: &str, ask: &str) -> String {
    format!(
        "Investigate this task before any work on it: answer what it asks, or find out what the change involves. Report the required behavior, constraints, acceptance criteria and risks, and anything you didn't check. Name the symbols, modules and non-source files that need attention when that helps define the work. Don't prescribe implementation, edits, submission shape or checks. The human never edits files or runs commands; any needed change, a settings change included, is the worker's to make. If a smaller option would change what the human gets, offer it with `task ask`. {read_only} {define} {start} {ask} Finish with `task done`, the report in `text`."
    )
}

/// The most of the vision a brief carries (D-91): all of it, since a save is
/// held to the same limit (D-92). Only a vision saved before the limit is cut.
const VISION_CAP: usize = tasks::VISION_LIMIT;

/// The most of a feature note a brief carries (D-138), as for the vision.
const FEATURE_CAP: usize = VISION_CAP;

/// Task `t`'s feature for its brief and `session start --task` (D-138, D-203):
/// the feature's title, state and whole body, then the most recent of the
/// task's investigation report, assignment note (the human's, outside any run)
/// and review feedback, cut at a line end within `FEATURE_CAP` characters.
/// Empty when the task has no feature, or its feature is gone.
pub fn feature_section(top: &Path, t: &Value) -> String {
    let Some(f) = t["feature"].as_str().and_then(|fid| tasks::record(top, "features", fid)) else { return String::new() };
    let runs = t["runs"].as_array().cloned().unwrap_or_default();
    let kind_of = |n: &Value| runs.iter().find(|r| r["id"] == n["run"]).map(|r| text(&r["kind"])).unwrap_or_default();
    let label = |n: &Value| match (n["author"].as_str(), n["run"].as_str(), kind_of(n).as_str()) {
        (Some("agent"), Some(rid), "investigate") => Some(format!("investigation report (run {rid})")),
        (Some("agent"), Some(rid), "review") => Some(format!("review feedback (run {rid})")),
        (Some("human"), None, _) => Some("assignment".to_string()),
        _ => None,
    };
    let mut out = format!(
        "\nFeature (the feature this task is part of; its state and body, then its latest direction):\n- {}\n- state: {}\n",
        text(&f["title"]),
        text(&f["state"])
    );
    let body = text(&f["body"]);
    if !body.is_empty() {
        out += &format!("\n{body}\n");
    }
    // An empty note gives no direction, so the next one is used.
    let latest =
        t["notes"].as_array().into_iter().flatten().rev().find_map(|n| Some((label(n)?, text(&n["text"]))).filter(|(_, x)| !x.is_empty()));
    if let Some((label, note)) = latest {
        let mut shown = note.clone();
        if note.chars().count() > FEATURE_CAP {
            let cut: String = note.chars().take(FEATURE_CAP).collect();
            shown = match cut.rfind('\n') {
                Some(i) if i > 0 => cut[..i].trim_end().to_string(),
                _ => cut,
            };
            shown += "\n(cut short; the whole note is in the task's notes on the board)";
        }
        out += &format!("\n- {label}: {shown}\n");
    }
    out
}

/// What a brief says when a dependency is unfinished (D-110): advice, nothing is refused.
pub(crate) const WAITING: &str = "This task depends on unfinished work. Check whether you can do your part without it. If you can't, ask the human (`task ask`) before building on code that doesn't exist yet.";

/// Task `tid`'s dependencies for its brief and `session start --task` (D-110,
/// D-203): each with its id, title and column, from any project of the root,
/// the unfinished ones marked, then advice only when one is unfinished.
/// Unfinished is as on the board: not in a final column, so complete and
/// canceled are finished. Empty only when the task has no dependencies.
pub fn waiting_on(top: &Path, tid: &str) -> String {
    let (Ok(all), Ok(w)) = (all_tasks(top), flow(top)) else { return String::new() };
    let Some(t) = all.iter().find(|o| o["id"] == tid) else { return String::new() };
    let (mut lines, mut unfinished) = (vec![], false);
    for id in t["dependsOn"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        // A deleted task is no work to wait for.
        let Some(d) = all.iter().find(|o| o["id"] == id) else { continue };
        let open = crate::board::task_model::column(&w, d)["final"] != true;
        unfinished |= open;
        lines.push(task_line(d, &t["home"], &w, open));
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("\nDepends on:\n{}\n", lines.join("\n"));
    if unfinished {
        out += &format!("{WAITING}\n");
    }
    out
}

/// The root's vision for an investigate, work or discuss brief (D-91): the
/// whole text, or for an older, longer one its first `VISION_CAP` characters,
/// cut at a line end when one is in reach. None for a review, or when there is
/// no vision.
fn vision_section(top: &Path, kind: &str) -> String {
    let text = tasks::vision(top)["text"].as_str().unwrap_or("").trim().to_string();
    if kind == "review" || text.is_empty() {
        return String::new();
    }
    let mut shown = text.clone();
    if text.chars().count() > VISION_CAP {
        let cut: String = text.chars().take(VISION_CAP).collect();
        shown = match cut.rfind('\n') {
            Some(i) if i > 0 => cut[..i].trim_end().to_string(),
            _ => cut,
        };
        shown += "\n(cut short; the whole vision is on the Strategy page)";
    }
    format!("\nVision (what the product is for, kept on the Strategy page; let it guide your choices):\n{shown}\n")
}

/// An agent's prompt: what its run is for, how to start, ask and finish, then
/// what it needs from the task (Spec 15 §6), as plain text.
/// A fresh run's first step; a resumed run continues instead.
pub(crate) const START: &str = "Start with `session start --task`, with the task's title and description as `text`.";

/// A fresh run's first step when its brief holds the session start reply (D-151).
pub(crate) const HELD_START: &str = "The reply to `session start --task`, with the task's title and description, is at the end of this brief: start with the call its `Next:` line names. Send `session start --task` again only to see that reply again, for example after the conversation is compacted.";

/// What every review is asked to add (D-112, D-198, D-199).
pub(crate) const TECH_DEBT_ASK: &str = "Put minor, non-blocking findings that should be fixed later in a Tech debt section at the end of the feedback, each written like a finding. Findings that block this task belong in the findings, not there. Leave out items already in the work summary's `### Tech debt` section: they are filed already. When the task is completed, Solace files each item on a Tech Debt task, and a section in any other shape is dropped: the heading is exactly `### Tech debt`; each item is one `- ` bullet at the start of a line; its details are lines indented under it; no line after the heading starts with `#`; no other heading is inside the section, and nothing follows it except a new heading. For example:\n\n### Tech debt\n\n- `board.brief`: <what is wrong>. Change it to <what> because <why>.\n  Check: <test or command and its expected result>.";

/// The task's context as a brief gives it: title, body, links, notes and
/// questions. Its feature and dependencies are added where a brief adds them.
pub(crate) fn task_sections(t: &Value) -> String {
    let mut out = format!("Task: {}\n", text(&t["title"]));
    if !text(&t["body"]).is_empty() {
        out += &format!("\n{}\n", text(&t["body"]));
    }
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            out += &format!("\n{title}:\n{}\n", lines.join("\n"));
        }
    };
    let list = |k: &str| t[k].as_array().cloned().unwrap_or_default();
    section("Links", list("links").iter().map(|l| format!("- {}: {}", text(&l["name"]), text(&l["url"]))).collect());
    section(
        "Notes so far",
        list("notes")
            .iter()
            .map(|n| {
                let body = text(&n["text"]);
                let run = list("runs").into_iter().find(|r| r["id"] == n["run"]);
                match run {
                    Some(r) => format!("- {} ({} run {}): {body}", text(&n["author"]), text(&r["kind"]), text(&r["id"])),
                    None => format!("- {}: {body}", text(&n["author"])),
                }
            })
            .collect(),
    );
    section(
        "Questions",
        list("questions")
            .iter()
            .map(|q| match q["answer"].as_str() {
                Some(a) => format!("- {}\n  Answer: {a}", text(&q["text"])),
                None => format!("- {}\n  (not answered yet)", text(&q["text"])),
            })
            .collect(),
    );
    out
}

/// What every review's findings must say (D-199).
pub(crate) const FINDINGS_ASK: &str = "Write each finding so a worker can act on it without investigating again: name the symbol (`module.name`) or file, what is wrong, what to change it to and why, and how to check it is done (a test to add or run, or what a command or page should show).";

/// The tasks whose `dependsOn` names `tid` (D-203), for its brief and
/// `session start --task`: one line each, from any project. Empty when none.
pub fn needed_by(top: &Path, tid: &str) -> String {
    let (Ok(all), Ok(w)) = (all_tasks(top), flow(top)) else { return String::new() };
    let Some(t) = all.iter().find(|o| o["id"] == tid) else { return String::new() };
    let lines: Vec<String> = all
        .iter()
        .filter(|o| o["dependsOn"].as_array().is_some_and(|d| d.iter().any(|x| x.as_str() == Some(tid))))
        .map(|o| task_line(o, &t["home"], &w, false))
        .collect();
    if lines.is_empty() { String::new() } else { format!("\nNeeded by:\n{}\n", lines.join("\n")) }
}

/// The plan around task `t` (D-203): its feature, dependencies, dependents,
/// then how to read one.
pub(crate) fn plan_sections(top: &Path, t: &Value) -> String {
    let tid = t["id"].as_str().unwrap_or("");
    let feature = feature_section(top, t);
    let deps = waiting_on(top, tid);
    let dependents = needed_by(top, tid);
    let mut hints: Vec<&str> = vec![];
    if !deps.is_empty() || !dependents.is_empty() {
        hints.push("`task peek <id>` reads a listed task");
    }
    if !feature.is_empty() {
        hints.push("`task peek feature` reads the feature");
    }
    let hint = if hints.is_empty() { String::new() } else { format!("\n{}.\n", hints.join("; ")) };
    format!("{feature}{deps}{dependents}{hint}")
}

/// `plan_sections` for task `tid`, empty when it is gone.
pub fn plan_reading(top: &Path, tid: &str) -> String {
    let Some(t) = all_tasks(top).ok().and_then(|all| all.into_iter().find(|o| o["id"] == tid)) else { return String::new() };
    plan_sections(top, &t)
}

pub(crate) const WORK_TECH_DEBT_ASK: &str = "End the `task done` summary with a `### Tech debt` section only for a bug in a step or tool, or a step that cost far more than it gave: a reply that was oversized or mostly about code the task didn't touch, a reply too unclear to act on so the step was repeated, or a check that can't pass for reasons outside the task. Name the command or step, what was wasted, and what would fix it. Leave the section out when there is none; ordinary friction, code-quality notes and restating risks are not tech debt. Its shape is exact: the heading is exactly `### Tech debt`; each item is one `- ` bullet at the start of a line; its details are lines indented under it; no line after the heading starts with `#`; no other heading is inside the section, and nothing follows it except a new heading. For example:\n\n### Tech debt\n\n- `<command or step>`: <what was wasted>. <what would fix it>.";

/// `task peek` (D-203): a task the run may read, or its feature, as JSON.
pub fn task_peek(s: &Store, tid: &str, args: &[&str], budget: usize) -> Out {
    let Some(subject) = args.first().copied() else {
        return usage("task peek <task> [--notes [3,7,10..12]] [--from <n>] | task peek feature");
    };
    if tasks::bare_id(subject) == "feature" {
        if args.len() > 1 {
            return usage("task peek feature takes no other arguments");
        }
        return peek_feature(&s.root, tid);
    }
    peek_task(&s.root, tid, subject, &args[1..], budget)
}

/// `task peek <task>`: the target's fields, then its notes when asked.
fn peek_task(root: &Path, tid: &str, subject: &str, rest: &[&str], budget: usize) -> Out {
    let top = scope::top(root);
    let t = task(root, tid)?;
    let (Ok(all), Ok(w)) = (all_tasks(&top), flow(&top)) else {
        return err("E7001", "the plan around this task could not be read");
    };
    let id = tasks::bare_id(subject);
    let allowed = readable_tasks(&t, &all);
    if !allowed.iter().any(|x| x.as_str() == id) {
        let lines: Vec<String> = allowed
            .iter()
            .filter_map(|x| all.iter().find(|o| o["id"] == x.as_str()))
            .map(|o| task_line(o, &t["home"], &w, false))
            .collect();
        return err("E7001", format!("task {id} is not one this run may read. It may read:\n{}", lines.join("\n")));
    }
    let Some(o) = all.iter().find(|o| o["id"] == id) else { return err("E7023", format!("unknown task {id}")) };
    let (notes, spec, from) = peek_flags(rest)?;
    let feature = o["feature"].as_str().and_then(|fid| {
        tasks::record(&top, "features", fid).map(|f| {
            json!({
                "id": format!("feature:{fid}"),
                "title": text(&f["title"]),
                "state": text(&f["state"]),
            })
        })
    });
    let deps: Vec<String> = o["dependsOn"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|d| all.iter().find(|x| x["id"] == d))
        .map(|d| task_line(d, &o["home"], &w, crate::board::task_model::column(&w, d)["final"] != true))
        .collect();
    let mut reply = json!({
        "task": format!("task:{id}"),
        "title": text(&o["title"]),
        "column": column_name(&w, o),
        "feature": feature,
        "dependsOn": deps,
        "body": text(&o["body"]),
    });
    if notes {
        reply["notes"] = selected_notes(o, spec.as_deref())?;
        return crate::board::strategy::in_parts(reply, "notes", "note", &format!("task peek {id} --notes"), from, budget);
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&reply).unwrap()))
}

/// `task peek feature`: the task's feature and its tasks.
fn peek_feature(root: &Path, tid: &str) -> Out {
    let top = scope::top(root);
    let t = task(root, tid)?;
    let Some(fid) = t["feature"].as_str() else { return err("E7001", format!("task {tid} has no feature")) };
    let Some(f) = tasks::record(&top, "features", fid) else {
        return err("E7001", format!("task {tid}'s feature {fid} is gone"));
    };
    let (Ok(all), Ok(w)) = (all_tasks(&top), flow(&top)) else {
        return err("E7001", "the plan around this task could not be read");
    };
    let tasks: Vec<String> =
        all.iter().filter(|o| o["feature"].as_str() == Some(fid)).map(|o| task_line(o, &t["home"], &w, false)).collect();
    let reply = json!({
        "feature": format!("feature:{fid}"),
        "title": text(&f["title"]),
        "state": text(&f["state"]),
        "body": text(&f["body"]),
        "tasks": tasks,
    });
    Ok(format!("{}\n", serde_json::to_string_pretty(&reply).unwrap()))
}

/// `--notes [3,7,10..12]` and `--from <n>` after `task peek <task>`.
fn peek_flags(rest: &[&str]) -> Result<(bool, Option<String>, usize), Failure> {
    let (mut notes, mut spec, mut from) = (false, None, 0);
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--notes" => {
                notes = true;
                if let Some(given) = rest.get(i + 1).filter(|x| !x.starts_with("--")) {
                    spec = Some((*given).to_string());
                    i += 1;
                    if rest.get(i + 1).is_some_and(|x| !x.starts_with("--")) {
                        return usage("notes take one list, e.g. `--notes 1,2,8..12`");
                    }
                }
            }
            "--from" => {
                let Some(n) = rest.get(i + 1) else { return usage("--from needs a character number") };
                from = n.parse().map_err(|_| Failure("E7001 --from takes a character number, as a `Next:` line gives it".into(), 1))?;
                i += 1;
            }
            other => return usage(format!("task peek has no {other}")),
        }
        i += 1;
    }
    if from > 0 && !notes {
        return usage("--from reads on in the notes; add `--notes`");
    }
    Ok((notes, spec, from))
}

/// The task ids a run working on `t` may read (D-203): its own, its
/// dependencies, its dependents, and tasks named `task:<id>` in its body or notes.
fn readable_tasks(t: &Value, all: &[Value]) -> Vec<String> {
    let mut ids = vec![];
    if let Some(id) = t["id"].as_str() {
        add_allowed(&mut ids, all, id);
    }
    for id in t["dependsOn"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        add_allowed(&mut ids, all, id);
    }
    for o in all {
        if o["dependsOn"].as_array().is_some_and(|d| d.iter().any(|x| x.as_str() == t["id"].as_str())) {
            if let Some(id) = o["id"].as_str() {
                add_allowed(&mut ids, all, id);
            }
        }
    }
    for id in named_task_ids(t) {
        add_allowed(&mut ids, all, &id);
    }
    ids
}

/// Adds `id` to an allowed list once, when the task exists.
fn add_allowed(ids: &mut Vec<String>, all: &[Value], id: &str) {
    if !ids.iter().any(|x| x.as_str() == id) && all.iter().any(|o| o["id"] == id) {
        ids.push(id.to_string());
    }
}

/// The task ids linked as `task:<id>` in a task's body or notes.
fn named_task_ids(t: &Value) -> Vec<String> {
    let mut ids = vec![];
    scan_task_links(t["body"].as_str().unwrap_or(""), &mut ids);
    for n in t["notes"].as_array().into_iter().flatten() {
        scan_task_links(n["text"].as_str().unwrap_or(""), &mut ids);
    }
    ids
}

/// Adds the id after each `task:` in `text`.
fn scan_task_links(text: &str, ids: &mut Vec<String>) {
    for part in text.split("task:").skip(1) {
        let id: String = part.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !id.is_empty() {
            ids.push(id);
        }
    }
}

/// The task's notes named by `spec` (`3,7,10..12`, or `all`), each with its
/// `note` number.
fn selected_notes(t: &Value, spec: Option<&str>) -> Result<Value, Failure> {
    let all = t["notes"].as_array().cloned().unwrap_or_default();
    let picked = match spec {
        None | Some("all") => (1..=all.len()).collect(),
        Some(spec) => crate::board::strategy::parse_number_list(spec, all.len(), "notes")?,
    };
    Ok(json!(
        picked
            .into_iter()
            .map(|n| {
                let mut note = all[n - 1].clone();
                note["note"] = json!(n);
                note
            })
            .collect::<Vec<_>>()
    ))
}

/// Replaces each markdown image of an uploaded file with `[image N]`, numbering
/// files in first-seen order. Other images and text are left as written.
fn note_images(text: &str, prefix: &str, found: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("![") {
        let after = &rest[i..];
        let Some(open) = after.find("](") else { break };
        let Some(close) = after[open..].find(')').map(|c| open + c) else { break };
        out += &rest[..i];
        match after[open + 2..close].strip_prefix(prefix).filter(|n| image_type(n).is_some()) {
            Some(name) => {
                let n = found.iter().position(|x| x == name).unwrap_or_else(|| {
                    found.push(name.to_string());
                    found.len() - 1
                });
                out += &format!("[image {}]", n + 1);
            }
            None => out += &after[..=close],
        }
        rest = &after[close + 1..];
    }
    out + rest
}

/// Note images: content-checked raster formats only (SVG can carry script).
pub(crate) const IMAGE_LIMIT: usize = 5 * 1024 * 1024;

fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// The media type of a stored image's name, which is `<16 hex>.<ext>`.
pub(crate) fn image_type(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match ext {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub(crate) fn image_path(root: &Path, tid: &str, name: &str) -> Result<PathBuf, Failure> {
    let p = task_dir(root, tid)?.join("files").join(name);
    if image_type(name).is_none() || !p.exists() {
        return err("E7001", "unknown image");
    }
    Ok(p)
}

pub fn upload(root: &Path, tid: &str, bytes: &[u8]) -> Result<Value, Failure> {
    task(root, tid)?;
    if bytes.len() > IMAGE_LIMIT {
        return err("E7001", "image over 5 MB");
    }
    let ext = image_kind(bytes).ok_or_else(|| Failure("E7001 only PNG, JPEG, GIF or WebP images".into(), 1))?;
    let name = format!("{}.{ext}", &sha256(bytes)[..16]);
    let dir = task_dir(root, tid)?.join("files");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(&name), bytes)?;
    Ok(json!({"name":name,"url":format!("/tasks/{tid}/files/{name}")}))
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

pub(crate) fn note_fingerprint(note: &Value) -> String {
    match note["id"].as_str() {
        Some(id) => format!("id:{id}"),
        None => sha256(note.to_string().as_bytes()),
    }
}

/// The task's drafts, with their project folders, from every project in its scope.
pub(crate) fn task_drafts(s: &Store, t: &Value) -> Result<Vec<(String, String, Vec<String>, bool)>, Failure> {
    let tid = t["id"].as_str().unwrap_or("");
    let mut out = vec![];
    for (dir, st) in scope_stores(s, t)? {
        for (id, args, current) in drafts::entries(&st, tid)? {
            out.push((dir.clone(), id, args, current));
        }
    }
    Ok(out)
}

/// `session remember`: the task's context and the run's own record, for an
/// agent that noticed its conversation was compacted. It reads only and is
/// repeatable; a discussion's brief already holds its context, so it is
/// refused, and so is a run that isn't there (D-157 point 7).
pub(crate) fn remember(top: &Path, tid: &str, rid: &str, budget: usize) -> Out {
    let Some(run) = run_record(top, tid, rid) else { return err("E7003", format!("unknown run {rid}")) };
    if run["kind"] == "discuss" {
        return err("E7010", "a discussion's brief holds its context; reply with `task note`");
    }
    let t = read_task(top, &task_path(top, tid)?)?;
    let mut context = task_sections(&t);
    context += &plan_sections(top, &t);
    let reads = reads::window_reads().unwrap_or_else(|| run_reads(top, tid, rid));
    Ok(reads::remember_reply(&context, &reads, top, &|b| run_outline(top, tid, rid, b), budget))
}

/// The guidance for the chosen involvement level; an engineer keeps the detailed brief.
pub(super) fn involvement_section(involvement: &str, note: &str, kind: &str) -> Option<String> {
    match involvement {
        "director" => Some(director_section(note, kind)),
        "collaborator" => Some(collaborator_section(kind)),
        _ => None,
    }
}

/// How every run at the Director level addresses the human.
fn director_section(note: &str, kind: &str) -> String {
    format!(
        concat!(
            "How to talk to the human (Director): use plain words in questions and in the `### Summary` part of a note. ",
            "Detail meant for later agents, review findings and `### Tech debt` items may name code, symbols and files. ",
            "Don't use code, symbol names, file paths or error codes in those human-facing parts unless the human asks for them; project folder names in a scope request are the exception. ",
            "Ask about what the software should do, never how. ",
            "{}",
            "When a choice can't be put that way, make the call yourself and {}. ",
            "A note keeps the human's explanation under a `### Summary` heading near its top, with detail meant for later agents below it. ",
            "{}",
            "The technical references below are for you, not for the human."
        ),
        if kind == "strategy" {
            ""
        } else {
            "Each question option says what the human will see happen, and one option is marked as recommended in its label (`Recommended: …`). "
        },
        calls_made_sentence(note),
        review_order(kind)
    )
}

/// How every run at the Collaborator level addresses the human.
fn collaborator_section(kind: &str) -> String {
    format!(
        concat!(
            "How to talk to the human (Collaborator): explain each technical term the first time it appears in questions and in the `### Summary` part of a note. ",
            "Detail meant for later agents, review findings and `### Tech debt` items may use technical terms. ",
            "Summarize each trade-off in one sentence. ",
            "{}",
            "A note keeps the human's explanation under a `### Summary` heading near its top, with detail meant for later agents below it. ",
            "{}"
        ),
        if kind == "strategy" { "" } else { "Mark one question option as recommended in its label (`Recommended: …`). " },
        review_order(kind)
    )
}

/// A review's assessment label comes first, so its summary follows it.
fn review_order(kind: &str) -> &'static str {
    if kind == "review" { "In a review, the assessment label is the first line and the `### Summary` heading comes after it. " } else { "" }
}

/// The agent behavior guidance shared by task and strategy briefs.
pub(super) fn behavior_sections(involvement: &str, detail: &str, asking: &str, note: &str, kind: &str) -> String {
    let mut out = String::new();
    for section in [involvement_section(involvement, note, kind), detail_section(detail), Some(asking_section(asking, note, involvement))]
        .into_iter()
        .flatten()
    {
        out += &section;
        out += "\n";
    }
    out += UI_DETAILS_ASK;
    out += "\n";
    out
}

/// How much a note says at the chosen detail level; standard is the usual brief.
fn detail_section(detail: &str) -> Option<String> {
    let text = match detail {
        "concise" => "How much to write (Concise): keep notes and summaries to a few lines, with the outcome first.",
        "thorough" => {
            "How much to write (Thorough): notes and summaries also cover the alternatives considered and how the result was checked."
        }
        _ => return None,
    };
    Some(text.to_string())
}

/// How readily the agent asks instead of deciding.
fn asking_section(asking: &str, note: &str, involvement: &str) -> String {
    let (title, guidance) = match asking {
        "often" => ("Often", "ask whenever the task allows more than one reasonable reading".to_string()),
        "rarely" => {
            let record = if involvement == "director" { String::new() } else { format!(" and {}", calls_made_sentence(note)) };
            (
                "Rarely",
                format!("decide whatever can be undone{record}. Ask only about things that can't be undone or that contradict the task"),
            )
        }
        _ => ("Balanced", "ask when a wrong guess would mean redoing work or would change what the human sees".to_string()),
    };
    format!("When to ask ({title}): {guidance}. This never stops you from asking when something conflicts with a decision or the vision.")
}

/// How a choice the agent made instead of asking is recorded.
fn calls_made_sentence(note: &str) -> String {
    format!("record it with `{note}` under a `### Calls made` heading, giving a plain reason; a call made never goes in `DECISIONS.md`")
}

/// The root's personas for a brief (D-218), in creation order: each name, then
/// only its non-empty fields. None when there are no personas.
pub(crate) fn personas_section(top: &Path) -> String {
    let personas = tasks::personas(top);
    if personas.is_empty() {
        return String::new();
    }
    let mut out = String::from(concat!(
        "\nPersonas (who the software is for, or who works on it, kept on the Strategy page; ",
        "let them guide your choices):\n"
    ));
    for (i, p) in personas.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out += &format!("{}\n", p["name"].as_str().unwrap_or("").trim());
        for (key, label) in tasks::strategy::PERSONA_FIELDS {
            let text = p[key].as_str().unwrap_or("").trim();
            if !text.is_empty() {
                out += &format!("  {label}: {}\n", text.replace('\n', "\n    "));
            }
        }
    }
    out
}

/// The task's title, description and, for a Tech Debt task, its findings, as
/// `session start --task` ranks them (D-151, D-228).
pub(crate) fn task_text(t: &Value) -> String {
    let text = |k: &str| t[k].as_str().unwrap_or("").trim().to_string();
    let tech_debt = t["techDebt"] == true;
    let body = text("body");
    let mut out = format!("{}\n", text("title"));
    if !tech_debt || body != crate::board::reviews::TECH_DEBT_BODY {
        out.push_str(&body);
    }
    if tech_debt {
        out.push('\n');
        out.push_str(&crate::board::reviews::tech_debt_findings(t));
    }
    out
}

/// What every brief asks about user-interface details.
pub(crate) const UI_DETAILS_ASK: &str = "Don't add user-interface details (labels, wording, indicators, colors, new buttons or screens, layout) the task or human didn't ask for; ask the human when one matters, and report user-interface behavior that is broken.";
