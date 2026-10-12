//! Strategy runs (D-93): an agent plans with the human on the Strategy page.
//! The run is a root record, not a task's; its agent reads and replies, and
//! changes no code. A module of `board`, whose helpers it uses.

use super::*;

/// Launches strategy run `rid`'s agent on its own thread. A plain-text reply
/// is its turn: a fresh agent, briefed with every note, takes the next one.
pub(super) fn start_strategy(top: PathBuf, rid: String) {
    std::thread::spawn(move || {
        let result = (|| {
            let mut result = strategy_process(&top, &rid);
            while let Err(Failure(_, REPLIED)) = result {
                wait_strategy(&top, &rid)?;
                result = strategy_process(&top, &rid);
            }
            result
        })();
        if let Err(e) = result {
            let _ = Watch::Strategy(&top).fail(&rid, &e.0);
        }
    });
}
/// Runs strategy run `rid`'s agent until it exits. It always starts a fresh
/// conversation, in a folder beside the run's log, removed after.
fn strategy_process(top: &Path, rid: &str) -> Result<(), Failure> {
    let Some(run) = tasks::strategy_run(top, rid) else { return err("E7023", format!("unknown strategy run {rid}")) };
    if run["state"] == "ended" {
        return Ok(());
    }
    let config = system()?;
    let m = chosen_model(&config, &run)?;
    let timeout = config["task_timeout"].as_u64().unwrap_or(3600);
    let log = PathBuf::from(run["log"].as_str().unwrap_or(""));
    let dir = log.with_extension("");
    fs::create_dir_all(&dir)?;
    let prompt = strategy_brief(top, &run);
    let launched = spawn_agent(top, &dir, &log, &m, &prompt, &[], None, persists(&m), [STRATEGY, rid], timeout);
    let supervised = launched.and_then(|mut child| supervise(Watch::Strategy(top), rid, &mut child, &log, false, Duration::from_secs(timeout), GRACEFUL_EXIT));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    supervised.map(drop)
}
/// A strategy run's role (Spec 15 §6).
const STRATEGY_ROLE: &str = "Plan with the human: the tasks, features, milestones and vision on the Strategy page. This run is read-only: it changes no code, and every plan change is the human's. To propose one, send `strategy task|feature|milestone` (the id to edit, none to create) and the request's fields as a JSON object in `text`, or `strategy vision` with the new vision in `text`, alone in its call; it waits for the human's approval, and the reply says which, with a new record's id. The fields: a task `title`, `body`, `feature`, `milestone`, `dependsOn` (task ids), with `scope` (project folders) only when creating; a feature `title`, `body`, `milestone`, `state` (proposed, planned, active, complete or canceled); a milestone `title`, `body`, `targetDate`, `state`; and `rank` for any of them only when editing. Any other field is refused. A `title` names the work in a few words (about 3–7), prefixed with the feature's short name when that helps (`extract: JS FFI files`); mechanism, conditions, error codes and lists go in `body`. The human sees a proposal as its fields: `body` rendered as Markdown, the others as plain text. Read the plan with `strategy list` (a line per record, then the vision; `--archived` adds archived tasks) and `strategy show <task|feature|milestone> <id>...` (several ids at once), which gives the ids the record links to, then the record in brief: a task's notes, questions and runs one line each. `strategy show note <task> <notes>` gives those notes in full (`3,7,10..12`, or `all`); a reply stops at the end of a note and names the rest in a `Next:` line, and a note too long for one reply comes in parts, its `Next:` line ending in `--from <n>` to read on. A task's run log is `strategy show run <task> <run>` (one line per step), then `strategy show run <task> <run> 3,7,10..12` for those in full, or `all`, stopping the same way. Read code with `view`, `inspect`, `refs`, `find`, `grep`, `files` and `read` (`help <command>` gives its options), adding `--project <folder>` for a project below the root; `grep` searches its text as written, case-insensitive (quotes around the whole text are dropped), with no options, regex or path filter; `read <path> --lines 5..9` reads lines of a file. Cite code by name (`board.rs::within`, `checkout.pay`), not `file:line`: line numbers move. The human's message is the last note. Reply only with `strategy note`: `command` is just `strategy note`, and the whole reply goes in `text`, in Markdown. The call waits and returns the human's next message. Text outside `strategy note` shows only in this run's live transcript on the page, so anything the human should keep goes in a `strategy note`. Keep replying until a call says the run has ended, then stop.";
/// How many decision titles a strategy run's brief lists, the latest.
const RECENT_DECISIONS: usize = 10;
/// A strategy run's prompt: its role, the record it started on, the vision, decision titles and notes.
pub(super) fn strategy_brief(top: &Path, run: &Value) -> String {
    let mut out = format!("You are planning with the human on this project's Strategy page. You work only through the Solace tool: there is no shell and no file access.\n{STRATEGY_ROLE}\n");
    let c = &run["context"];
    match (c["kind"].as_str(), c["id"].as_str()) {
        (Some(kind @ ("task" | "feature" | "milestone")), Some(id)) => match strategy_show(top, kind, id) {
            Ok(record) => out += &format!("\nThis run is about the {kind} {id}:\n{}\n", serde_json::to_string_pretty(&record).unwrap()),
            Err(_) => out += &format!("\nThis run is about the {kind} {id}, which no longer exists.\n"),
        },
        _ => out += "\nThis run is about the whole Strategy page.\n",
    }
    let vision = tasks::vision(top)["text"].as_str().unwrap_or("").trim().to_string();
    if !vision.is_empty() {
        out += &format!("\nVision:\n{vision}\n");
    }
    // Reads without `--project` go to the root's own project; a folder of projects has none.
    let projects = workspace::members(top).unwrap_or_default();
    let own = projects.iter().any(|p| p == ".");
    if !own {
        out += &format!("\nThe root is a folder of projects, not a project: add `--project <folder>` to every read. Its projects: {}.\n", projects.join(", "));
    }
    let decisions = fs::read_to_string(top.join("DECISIONS.md")).unwrap_or_default();
    let titles: Vec<String> = decisions.lines().filter_map(|l| l.strip_prefix("## ")).filter(|l| l.starts_with("D-")).map(|l| format!("- {l}")).collect();
    if !titles.is_empty() {
        let how = if own { "`grep D-<n>`, then `read DECISIONS.md --lines a..b`, reads one" } else { "titles only: the file is in no project, so this run can't read it" };
        // The most recent only, so the brief stays short as decisions accumulate.
        let earlier = titles.len().saturating_sub(RECENT_DECISIONS);
        let rest = match earlier {
            0 => String::new(),
            n if own => format!("\n… {n} earlier decisions: `grep D-` lists them."),
            n => format!("\n… {n} earlier decisions not shown."),
        };
        out += &format!("\nRecent decisions (titles from DECISIONS.md; {how}):\n{}{rest}\n", titles[earlier..].join("\n"));
    }
    let notes: Vec<String> = run["notes"]
        .as_array()
        .map(|ns| ns.iter().map(|n| format!("- {}: {}", n["author"].as_str().unwrap_or(""), n["text"].as_str().unwrap_or("").trim())).collect())
        .unwrap_or_default();
    if !notes.is_empty() {
        out += &format!("\nNotes so far:\n{}\n", notes.join("\n"));
    }
    out
}
/// `strategy list` (D-101): a line per record, then the vision; archived tasks only with `archived`.
pub(super) fn strategy_list(top: &Path, archived: bool) -> Out {
    let q = |v: &Value| v.as_str().unwrap_or("-").to_string();
    let tasks = all_tasks(top)?;
    let mut out = String::from("Tasks:\n");
    for t in tasks.iter().filter(|t| archived || t["archivedTime"].is_null()) {
        out += &format!("{} {} f:{} m:{} {}\n", q(&t["id"]), q(&t["column"]), q(&t["feature"]), q(&t["milestone"]), q(&t["title"]));
    }
    out += "\nFeatures:\n";
    for f in tasks::records(top, "features") {
        out += &format!("{} {} m:{} {}\n", q(&f["id"]), q(&f["state"]), q(&f["milestone"]), q(&f["title"]));
    }
    out += "\nMilestones:\n";
    for m in tasks::records(top, "milestones") {
        out += &format!("{} {} #{} {}\n", q(&m["id"]), q(&m["state"]), m["rank"].as_f64().unwrap_or(0.), q(&m["title"]));
    }
    out += &format!("\nVision:\n{}\n", tasks::vision(top)["text"].as_str().unwrap_or(""));
    Ok(out)
}
/// `strategy show`: one task, feature or milestone as `record`, with `links`, the
/// ids of the records it links to.
pub(super) fn strategy_show(top: &Path, kind: &str, id: &str) -> Result<Value, Failure> {
    let all = all_tasks(top)?;
    let ids = |of: &[&Value]| -> Vec<Value> { of.iter().map(|v| v["id"].clone()).collect() };
    let (record, links) = match kind {
        "task" => {
            let Some(t) = all.iter().find(|t| t["id"] == id) else { return err("E7023", format!("unknown task {id}")) };
            let runs: Vec<&Value> = t["runs"].as_array().map(|rs| rs.iter().map(|r| &r["id"]).collect()).unwrap_or_default();
            (compact_task(t), json!({"feature": t["feature"], "milestone": t["milestone"], "dependsOn": t["dependsOn"], "runs": runs}))
        }
        "feature" => {
            let Some(f) = tasks::record(top, "features", id) else { return err("E7023", format!("unknown feature {id}")) };
            let held: Vec<&Value> = all.iter().filter(|t| t["feature"] == id).collect();
            let links = json!({"milestone": f["milestone"], "tasks": ids(&held)});
            (f, links)
        }
        "milestone" => {
            let Some(m) = tasks::record(top, "milestones", id) else { return err("E7023", format!("unknown milestone {id}")) };
            let held: Vec<&Value> = all.iter().filter(|t| t["milestone"] == id).collect();
            let features = tasks::records(top, "features");
            let features: Vec<&Value> = features.iter().filter(|f| f["milestone"] == id || held.iter().any(|t| t["feature"] == f["id"])).collect();
            (m, json!({"features": ids(&features), "tasks": ids(&held)}))
        }
        _ => return err("E7001", "strategy show takes task, feature or milestone, and an id; or run, a task id and a run id"),
    };
    // `links` sorts first, so a cut reply loses the record's end, never the links.
    Ok(json!({"links": links, "record": record}))
}
/// `strategy show note` (D-101): a task's notes in full, named by number (`3,7..9`) or `all`.
pub(super) fn strategy_note_show(top: &Path, tid: &str, notes: &str) -> Result<Value, Failure> {
    let Some(t) = all_tasks(top)?.into_iter().find(|t| t["id"] == tid) else { return err("E7023", format!("unknown task {tid}")) };
    let all = t["notes"].as_array().cloned().unwrap_or_default();
    if all.is_empty() {
        return err("E7001", format!("task {tid} has no notes"));
    }
    let numbered = |i: usize| {
        let mut n = all[i].clone();
        n["note"] = json!(i + 1);
        n
    };
    let picked = match notes {
        "all" => json!((0..all.len()).map(numbered).collect::<Vec<_>>()),
        spec => json!(step_list(spec, all.len())?.into_iter().map(|n| numbered(n - 1)).collect::<Vec<_>>()),
    };
    Ok(json!({"task": tid, "notes": picked}))
}
/// A step list like `3,7,10..12`: numbers and ranges within `1..=n`.
fn step_list(spec: &str, n: usize) -> Result<Vec<usize>, Failure> {
    let bad = || Failure(format!("E7001 steps are numbers and ranges such as 3,7,10..12, within 1..{n}; or `all`"), 1);
    let mut out = vec![];
    for part in spec.split(',') {
        let (a, b) = part.split_once("..").unwrap_or((part, part));
        let (a, b): (usize, usize) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
        if a == 0 || a > b || b > n {
            return Err(bad());
        }
        out.extend(a..=b);
    }
    Ok(out)
}
/// A line cut to 100 characters, with `…` when more was cut.
fn cut_line(what: &str) -> String {
    let cut: String = what.chars().take(100).collect();
    let more = if cut.len() < what.len() { "…" } else { "" };
    format!("{cut}{more}")
}
/// One line for a transcript entry: its number, kind, and text's first line cut to 100.
fn step_line(i: usize, e: &Value) -> String {
    let kind = e["kind"].as_str().unwrap_or("");
    let what = match kind {
        "tool" => format!("{} ({})", e["title"].as_str().unwrap_or("").lines().next().unwrap_or(""), e["status"].as_str().unwrap_or("")),
        _ => e["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string(),
    };
    format!("{} {kind}: {}", i + 1, cut_line(&what))
}
/// One line for a note: its number, author, time, and first line cut to 100.
fn note_line(i: usize, n: &Value) -> String {
    let time = n["time"].as_u64().map(|t| t.to_string()).unwrap_or(String::new());
    format!("{} {} {time}: {}", i + 1, n["author"].as_str().unwrap_or(""), cut_line(n["text"].as_str().unwrap_or("").lines().next().unwrap_or("")))
}
/// One line for a question: its number, text's first line, and answer.
fn question_line(i: usize, q: &Value) -> String {
    format!("{} {}: {}", i + 1, cut_line(q["text"].as_str().unwrap_or("").lines().next().unwrap_or("")), q["answer"].as_str().unwrap_or("unanswered"))
}
/// One line for a run: its id, kind, state and model.
fn run_line(r: &Value) -> String {
    let field = |k: &str| r[k].as_str().unwrap_or("");
    format!("{} {} {} {}", field("id"), field("kind"), field("state"), field("model"))
}
/// A task for `strategy show` (D-101): its own fields, notes, questions and runs one line each, no `conversation` and no run `log`s.
fn compact_task(t: &Value) -> Value {
    let lines = |key: &str, make: fn(usize, &Value) -> String| -> Vec<Value> {
        t[key].as_array().map(|a| a.iter().enumerate().map(|(i, v)| json!(make(i, v))).collect()).unwrap_or_default()
    };
    json!({
        "id": t["id"], "title": t["title"], "body": t["body"], "column": t["column"], "scope": t["scope"],
        "feature": t["feature"], "milestone": t["milestone"], "dependsOn": t["dependsOn"],
        "closedTime": t["closedTime"], "archivedTime": t["archivedTime"],
        "notes": lines("notes", note_line), "questions": lines("questions", question_line),
        "runs": t["runs"].as_array().map(|rs| rs.iter().map(|r| json!(run_line(r))).collect::<Vec<_>>()).unwrap_or_default(),
    })
}
/// `strategy show run`: a task's run without its `log`, and its transcript's
/// steps: one line each, in full for `steps` (`3,7,10..12`) or `all`.
pub(super) fn strategy_run_show(top: &Path, tid: &str, rid: &str, steps: Option<&str>) -> Result<Value, Failure> {
    let Some(t) = all_tasks(top)?.into_iter().find(|t| t["id"] == tid) else { return err("E7023", format!("unknown task {tid}")) };
    let Some(mut run) = t["runs"].as_array().and_then(|rs| rs.iter().find(|r| r["id"] == rid)).cloned() else {
        return err("E7023", format!("task {tid} has no run {rid}"));
    };
    // Token counts tell an agent nothing about the work, so they are not steps.
    let entries: Vec<Value> = transcript(Path::new(run["log"].as_str().unwrap_or(""))).into_iter().filter(|e| e["kind"] != "usage").collect();
    run.as_object_mut().map(|o| o.remove("log"));
    if entries.is_empty() && steps.is_some() {
        return err("E7001", format!("run {rid} has no steps"));
    }
    let numbered = |i: usize| {
        let mut e = entries[i].clone();
        e["step"] = json!(i + 1);
        e
    };
    let steps = match steps {
        None => json!(entries.iter().enumerate().map(|(i, e)| step_line(i, e)).collect::<Vec<_>>()),
        Some("all") => json!((0..entries.len()).map(numbered).collect::<Vec<_>>()),
        Some(spec) => json!(step_list(spec, entries.len())?.into_iter().map(|n| numbered(n - 1)).collect::<Vec<_>>()),
    };
    Ok(json!({"run": run, "steps": steps}))
}
/// A reply cut to `budget` bytes, saying so.
fn within(text: String, budget: usize) -> String {
    if text.len() <= budget {
        return text;
    }
    let cut = (0..=budget.saturating_sub(160)).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    let ids = if text.starts_with('[') { "fewer ids, or " } else { "" };
    format!("{}\n… cut at the reply limit; ask for {ids}notes and steps by number (`strategy show note|run`).\n", &text[..cut])
}
/// A reply of notes or steps (under `key`, each numbered by `number`) within
/// `budget` (D-105): it stops at the end of one and names the rest in a
/// `Next: <next> <numbers>` line, so none is cut midway. A first one over the
/// limit by itself comes in parts: as much of its longest text as fits, and a
/// `Next:` line ending in `--from <n>`. `from` starts the first one's longest
/// text at character `from`.
fn in_parts(mut reply: Value, key: &str, number: &str, next: &str, from: usize, budget: usize) -> Out {
    let mut items = match reply[key].take() {
        Value::Array(a) => a,
        _ => vec![],
    };
    let numbers: Vec<u64> = items.iter().filter_map(|i| i[number].as_u64()).collect();
    let first = items.first().cloned().unwrap_or(Value::Null);
    let long = longest_text(&first);
    let chars: Vec<char> = long.as_ref().map(|(_, t)| t.chars().collect()).unwrap_or_default();
    if from > 0 {
        let Some((field, _)) = &long else { return err("E7001", "--from reads on in the text of the first note or step named, and it has none") };
        if from >= chars.len() {
            return err("E7001", format!("--from {from} is past the end: its {field} has {} characters", chars.len()));
        }
        items[0] = with_part(&first, field, &chars, from, chars.len());
    }
    let rest = |taken: usize| if taken < items.len() { format!("Next: {next} {}\n", number_list(&numbers[taken..])) } else { String::new() };
    let render = |some: &[Value]| {
        let mut part = reply.clone();
        part[key] = json!(some);
        format!("{}\n", serde_json::to_string_pretty(&part).unwrap())
    };
    let all = render(&items);
    if items.is_empty() || all.len() <= budget {
        return Ok(within(all, budget));
    }
    // Each item's size in the reply, indented, never more than it takes: a
    // count these sizes don't fit doesn't fit, and one they fit is checked.
    let sizes: Vec<usize> = items
        .iter()
        .map(|i| {
            let s = serde_json::to_string_pretty(i).unwrap();
            s.len() + 4 * s.lines().count() + 2
        })
        .collect();
    let (mut taken, mut used) = (0, render(&[]).len());
    while taken < items.len() && used + sizes[taken] + rest(taken + 1).len() <= budget {
        used += sizes[taken];
        taken += 1;
    }
    while taken > 0 {
        let text = render(&items[..taken]) + &rest(taken);
        if text.len() <= budget {
            return Ok(text);
        }
        taken -= 1;
    }
    // The first is over the limit by itself: as much of its longest text as fits.
    if let Some((field, _)) = &long {
        let piece = |b: usize| {
            let more = if b < chars.len() { format!("Next: {next} {} --from {b}\n", number_list(&numbers)) } else { rest(1) };
            render(&[with_part(&first, field, &chars, from, b)]) + &more
        };
        let (mut lo, mut hi) = (from, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if piece(mid).len() <= budget { lo = mid } else { hi = mid - 1 }
        }
        if lo > from {
            return Ok(piece(lo));
        }
    }
    Ok(within(render(&items[..1]), budget.saturating_sub(rest(1).len())) + &rest(1))
}
/// An item's longest text field and its text: the one `--from` reads on in.
fn longest_text(item: &Value) -> Option<(String, String)> {
    item.as_object()?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).max_by_key(|(_, t)| t.len())
}
/// `item` with its `field` cut to characters `a..b` of `text`, and a `part` saying which.
fn with_part(item: &Value, field: &str, text: &[char], a: usize, b: usize) -> Value {
    let mut item = item.clone();
    item[field] = json!(text[a..b].iter().collect::<String>());
    if a > 0 || b < text.len() {
        item["part"] = json!(format!("{field}, characters {a}..{b} of {}", text.len()));
    }
    item
}
/// `--from <n>` after a list of notes or steps: where the first one's text starts.
fn from_flag(rest: &[&str]) -> Result<usize, Failure> {
    match rest {
        [] => Ok(0),
        ["--from", n] => n.parse().map_err(|_| Failure("E7001 --from takes a character number, as a `Next:` line gives it".into(), 1)),
        _ => err("E7001", "after the notes or steps, only `--from <n>` may follow"),
    }
}
/// Numbers in the `3,7,10..12` form, consecutive ones as a range.
fn number_list(numbers: &[u64]) -> String {
    let mut parts: Vec<(u64, u64)> = vec![];
    for &n in numbers {
        match parts.last_mut() {
            Some(last) if n == last.1 + 1 => last.1 = n,
            _ => parts.push((n, n)),
        }
    }
    parts.iter().map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}..{b}") }).collect::<Vec<_>>().join(",")
}
/// Waits until strategy run `rid`'s agent may act, taking no project lock: an
/// ended run runs nothing, and while it waits for the human only `strategy` passes.
pub fn strategy_guard(top: &Path, rid: &str, a: &[&str]) -> Result<(), Failure> {
    loop {
        let Some(run) = tasks::strategy_run(top, rid) else { return err("E7023", format!("unknown strategy run {rid}")) };
        if run["state"] == "ended" {
            return err("E7003", "run ended");
        }
        if run["state"] == "waiting" && a.first() != Some(&"strategy") {
            if !tasks::serving(top) {
                return err("E7003", "the run waits for the human and no board server is running; stop now");
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        return Ok(());
    }
}
/// `strategy list | show | note` in strategy run `rid`, which `strategy_guard` let act.
pub fn strategy_command(top: &Path, rid: &str, a: &[&str], input: &mut dyn FnMut() -> Out, budget: usize) -> Out {
    let pretty = |v: Value| within(format!("{}\n", serde_json::to_string_pretty(&v).unwrap()), budget);
    match a {
        ["strategy", "list"] => Ok(within(strategy_list(top, false)?, budget)),
        ["strategy", "list", "--archived"] => Ok(within(strategy_list(top, true)?, budget)),
        ["strategy", "show", "run", tid, rid] => Ok(pretty(strategy_run_show(top, tid, rid, None)?)),
        ["strategy", "show", "run", tid, rid, steps, rest @ ..] => {
            let from = from_flag(rest)?;
            in_parts(strategy_run_show(top, tid, rid, Some(steps))?, "steps", "step", &format!("strategy show run {tid} {rid}"), from, budget)
        }
        ["strategy", "show", "note", tid, notes, rest @ ..] => {
            let from = from_flag(rest)?;
            in_parts(strategy_note_show(top, tid, notes)?, "notes", "note", &format!("strategy show note {tid}"), from, budget)
        }
        ["strategy", "show", kind, ids @ ..] if !ids.is_empty() => {
            let r: Result<Vec<Value>, Failure> = ids.iter().map(|id| strategy_show(top, kind, id)).collect();
            let r = r?;
            Ok(pretty(if r.len() == 1 { r[0].clone() } else { json!(r) }))
        }
        ["strategy", "note", rest @ ..] => {
            if rest.first().is_some_and(|w| w.starts_with("--")) {
                return err("E7015", "nothing was sent. Resend with `command` set to just `strategy note` and the whole reply in the `text` argument");
            }
            let text = if rest.is_empty() { input()? } else { rest.join(" ") };
            if text.trim().is_empty() {
                return err("E7001", "strategy note needs the reply in `text`");
            }
            // The agent sends the task server a message; the run waits for the human.
            let sent = tasks::send(top, json!({"kind":"strategy_note","run":rid,"text":text}))?;
            if !tasks::applied(top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(top, &sent) {
                return err("E7001", REJECTED);
            }
            wait_strategy(top, rid)
        }
        ["strategy", kind @ ("task" | "feature" | "milestone" | "vision"), rest @ ..] => {
            if rest.first().is_some_and(|w| w.starts_with("--")) {
                return err("E7015", format!("nothing was sent. Resend with `command` set to just `strategy {kind}` (and the id to edit) and the proposal in the `text` argument"));
            }
            let id = match (*kind, rest) {
                (_, []) => None,
                ("task" | "feature" | "milestone", [id]) => Some(*id),
                _ => return usage("strategy task|feature|milestone [<id>] with the fields as JSON in `text`, or strategy vision with the new text in `text`"),
            };
            let text = input()?;
            if text.trim().is_empty() {
                return err("E7001", format!("strategy {kind} needs the proposal in `text`"));
            }
            // Checked now as the request would be; applied only when the human approves.
            let question = tasks::proposal(top, rid, kind, id, &text)?;
            let qid = question["id"].as_str().unwrap_or("").to_string();
            let sent = tasks::send(top, json!({"kind":"strategy_propose","run":rid,"question":question}))?;
            if !tasks::applied(top, &sent) {
                return Ok(format!("{}\n", tasks::QUEUED));
            }
            if tasks::rejected(top, &sent) {
                return err("E7001", REJECTED);
            }
            wait_answer(top, rid, &qid)
        }
        _ => usage(
            "strategy list [--archived] | strategy show <task|feature|milestone> <id>... | strategy show note <task> <notes> [--from <n>] | strategy show run <task> <run> [<steps>|all] [--from <n>] | strategy note, with the reply in `text` | strategy task|feature|milestone [<id>] or strategy vision, with the proposal in `text`",
        ),
    }
}
/// Holds a strategy run's proposal until the human answers it or ends the run.
/// Waiting doesn't count toward the timeout.
fn wait_answer(top: &Path, rid: &str, qid: &str) -> Out {
    loop {
        std::thread::sleep(Duration::from_millis(50));
        if !tasks::serving(top) {
            return err("E7003", "the board server stopped while waiting");
        }
        let Some(run) = tasks::strategy_run(top, rid) else { return err("E7023", format!("unknown strategy run {rid}")) };
        if run["state"] == "ended" {
            return Ok("The strategy run has ended. Stop now, without further calls.\n".into());
        }
        let answer = run["questions"].as_array().and_then(|qs| qs.iter().find(|q| q["id"] == qid)).map(|q| q["answer"].clone());
        if let Some(Value::String(a)) = answer {
            tasks::send(top, json!({"kind":"strategy_resume","run":rid}))?;
            return Ok(format!("The human answered the proposal: {a}\n"));
        }
    }
}
/// Holds a strategy run's reply until the human writes again or ends the run,
/// as `wait_discussion` does. Waiting doesn't count toward the timeout.
fn wait_strategy(top: &Path, rid: &str) -> Out {
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // No server, so the human can't reply: return rather than wait forever.
        if !tasks::serving(top) {
            return err("E7003", "the board server stopped while waiting");
        }
        let Some(run) = tasks::strategy_run(top, rid) else { return err("E7023", format!("unknown strategy run {rid}")) };
        if run["state"] == "ended" {
            return Ok("The strategy run has ended. Stop now, without further calls.\n".into());
        }
        let notes = run["notes"].as_array().cloned().unwrap_or_default();
        let after = notes.iter().rposition(|n| n["author"] == "agent").map_or(0, |l| l + 1);
        let new: Vec<&str> = notes[after..].iter().filter(|n| n["author"] == "human").filter_map(|n| n["text"].as_str()).collect();
        if !new.is_empty() {
            tasks::send(top, json!({"kind":"strategy_resume","run":rid}))?;
            return Ok(format!("The human replied:\n\n{}\n", new.join("\n\n")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-105: a reply of notes stops at a whole note and names the rest; a
    /// first note over the limit by itself comes in parts, read on with `--from`.
    #[test]
    fn a_reply_of_notes_stops_at_a_whole_note() {
        let note = |n: u64| json!({"note": n, "text": "x".repeat(300)});
        let reply = json!({"task": "t1", "notes": [note(1), note(2), note(3), note(5)]});
        let two = format!("{}\n", serde_json::to_string_pretty(&json!({"task": "t1", "notes": [note(1), note(2)]})).unwrap());
        let next = "Next: strategy show note t1 3,5\n";
        let shown = |from: usize, budget: usize| in_parts(reply.clone(), "notes", "note", "strategy show note t1", from, budget);
        assert_eq!(shown(0, two.len() + next.len()).unwrap(), two.clone() + next);
        // One byte less leaves room for one note only.
        assert!(shown(0, two.len() + next.len() - 1).unwrap().ends_with("Next: strategy show note t1 2..3,5\n"));
        // Everything fits: no `Next:` line.
        let whole = shown(0, 10_000).unwrap();
        assert!(!whole.contains("Next:") && whole.contains("\"note\": 5"), "{whole}");
        // The first note alone is over the limit: its text comes in parts.
        let cut = shown(0, 250).unwrap();
        let at: usize = cut.trim_end().rsplit("--from ").next().unwrap().parse().unwrap();
        assert!(cut.len() <= 250 && at > 0 && cut.ends_with(&format!("Next: strategy show note t1 1..3,5 --from {at}\n")), "{cut}");
        assert!(cut.contains(&format!("\"part\": \"text, characters 0..{at} of 300\"")), "{cut}");
        let on = shown(at, 10_000).unwrap();
        assert!(on.contains(&format!("\"part\": \"text, characters {at}..300 of 300\"")) && on.contains(&format!("\"text\": \"{}\"", "x".repeat(300 - at))), "{on}");
        assert!(!on.contains("Next:") && on.contains("\"note\": 5"), "{on}");
        assert!(shown(300, 10_000).unwrap_err().0.starts_with("E7001"));
        assert_eq!(from_flag(&["--from", "12"]).unwrap(), 12);
        assert!(from_flag(&["--from", "x"]).is_err() && from_flag(&["2"]).is_err());
        assert_eq!(number_list(&[3, 7, 10, 11, 12, 2]), "3,7,10..12,2");
    }
}
