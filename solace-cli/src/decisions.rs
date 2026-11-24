//! The decision log: one `DECISIONS.md` at a board's top folder, holding what
//! a human settled. An agent proposes an entry; only an approval appends it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::fsx;
#[cfg(test)]
#[path = "decisions_tests.rs"]
mod tests;

pub(crate) const FILE: &str = "DECISIONS.md";

/// How many titles a brief lists.
const RECENT: usize = 10;

const HEADER: &str = concat!(
    "# Decisions\n\n",
    "Settled questions: don't re-ask them. Each record keeps what was decided, and why when that isn't obvious. ",
    "Every record is decided unless marked **pending**; a pending record gives its options instead of an answer. ",
    "Details live in the specs, PROPOSAL.md and git history.\n\n",
);

const RULES: &str = concat!(
    "\nRecord a decision when a human settles something later work must follow: what the product does or refuses to do; ",
    "a rule for how work is done; a format, file, command or interface others rely on; ",
    "a choice between options with a lasting trade-off; or a change to or reversal of an earlier decision, naming it. ",
    "Not a decision: anything the code or tests already show, a detail of one task, a call you made, ",
    "or a fix that restores intended behavior. ",
    "A proposal titled `## D-<n> <title>` changes record `D-n`; a number that names no record is refused. ",
    "Solace writes each record's who, task and date line, so a body never carries one.",
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
    fs::create_dir_all(top.join(".solace"))?;
    fsx::write_atomic(&p, HEADER)?;
    Ok(())
}

/// A proposed entry: its title, and what was decided with the `Why:` line.
/// `edit` names the record whose wording it replaces; none makes a new record.
pub struct Entry {
    pub title: String,
    pub body: String,
    pub edit: Option<u32>,
}

/// Reads an agent's proposal: a `## <title>` line, what was decided, and a
/// `Why:` line. A `## D-<n> ...` title proposes new wording for record `D-n`.
pub(crate) fn entry(text: &str) -> Result<Entry, crate::Failure> {
    let text = text.trim();
    let (head, body) = text.split_once('\n').unwrap_or((text, ""));
    let head = head.trim().strip_prefix("## ").map(str::trim).unwrap_or("");
    let (edit, title) = match head.strip_prefix("D-") {
        Some(rest) => match rest.split_once(' ') {
            Some((n, title)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                let n = n.parse::<u32>().map_err(|_| crate::Failure("E6021 the decision number is too large".into(), 1))?;
                (Some(n), title.trim())
            }
            _ => (None, head),
        },
        None => (None, head),
    };
    let body = body.trim();
    if title.is_empty() || body.is_empty() {
        return Err(crate::Failure("E6021 a decision needs a `## <title>` line and what was decided".into(), 1));
    }
    if !body.lines().any(|l| l.trim().to_ascii_lowercase().starts_with("why:")) {
        return Err(crate::Failure("E6021 a decision needs a `Why:` line".into(), 1));
    }
    // A `## ` line would start a record of its own once the entry is in the log.
    if body.lines().any(|l| l.starts_with("## ")) {
        return Err(crate::Failure("E6021 only the title line of a decision starts with `## `".into(), 1));
    }
    Ok(Entry { title: title.to_string(), body: body.to_string(), edit })
}

/// Appends an approved entry with the next number, or replaces the wording of
/// record `e.edit`, and says which one it made. `base` is an edit's wording
/// when it was proposed: an approval over different wording writes nothing
/// (D-255).
pub(crate) fn append(top: &Path, e: &Entry, who: &str, origin: &str, base: Option<&str>) -> Result<String, crate::Failure> {
    ensure(top)?;
    let mut text = fs::read_to_string(file(top))?;
    let date = today();
    let Some(n) = e.edit else {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.ends_with("\n\n") {
            text.push('\n');
        }
        let n = next(&text);
        text += &format!("## D-{n} {}\n({who}, {origin}, {date})\n{}\n\n", e.title, e.body);
        fsx::write_atomic(&file(top), text.as_bytes())?;
        return Ok(format!("D-{n} {}", e.title));
    };
    let all = records(&text);
    let r = all.iter().find(|r| r.id == format!("D-{n}")).ok_or_else(|| crate::Failure(format!("E6021 D-{n} names no decision"), 1))?;
    if base.is_some_and(|b| b != r.text) {
        return Err(crate::Failure(format!("E6021 D-{n} changed since the proposal; it was not applied"), 1));
    }
    let start = r.lines.0 - 1;
    let count = r.text.lines().count();
    let old: Vec<&str> = text.lines().skip(start).take(count).collect();
    // Attribution and earlier change lines stay; the body is replaced. A
    // record written before the attribution moved to line 2 keeps its last line.
    let body = &old[1..];
    let kept: Vec<&str> = match body.first().copied().filter(|l| attribution(l)) {
        Some(first) => std::iter::once(first).chain(body.iter().skip(1).copied().take_while(|l| l.starts_with("Changed by ("))).collect(),
        None => body.last().copied().filter(|l| attribution(l)).into_iter().collect(),
    };
    let mut record = format!("## D-{n} {}\n", e.title);
    for l in kept {
        record += l;
        record.push('\n');
    }
    record += &format!("Changed by ({who}, {origin}, {date})\n{}\n", e.body);
    let mut offset = 0;
    let mut starts = vec![];
    for l in text.split_inclusive('\n') {
        starts.push(offset);
        offset += l.len();
    }
    let from = starts[start];
    let to = starts.get(start + count).copied().unwrap_or(text.len());
    text.replace_range(from..to, &record);
    fsx::write_atomic(&file(top), text.as_bytes())?;
    Ok(format!("D-{n} {}", e.title))
}

fn next(text: &str) -> u32 {
    text.lines()
        .filter_map(|l| l.strip_prefix("## D-"))
        .filter_map(|rest| rest.split_once(' ').map(|(n, _)| n))
        .filter_map(|n| n.parse().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// One record: its id, its title, the whole record as Markdown, and its
/// first and last line in the log.
pub struct Record {
    pub id: String,
    pub title: String,
    pub text: String,
    pub lines: (usize, usize),
}

/// Every record, oldest first.
pub(crate) fn records(text: &str) -> Vec<Record> {
    let lines: Vec<&str> = text.lines().collect();
    let starts: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].starts_with("## ")).collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(k, &start)| {
            let end = starts.get(k + 1).copied().unwrap_or(lines.len());
            let rest = lines[start][3..].trim().strip_prefix("D-")?;
            let (id, title) = rest.split_once(' ').unwrap_or((rest, ""));
            let record = lines[start..end].join("\n").trim_end().to_string();
            let last = start + record.lines().count();
            Some(Record { id: format!("D-{id}"), title: title.trim().to_string(), text: record, lines: (start + 1, last) })
        })
        .collect()
}

/// The log as the Strategy page reads it: newest first.
pub(crate) fn api(top: &Path) -> serde_json::Value {
    let text = fs::read_to_string(file(top)).unwrap_or_default();
    let mut records = records(&text);
    records.reverse();
    serde_json::json!({
        "decisions": records.iter().map(|r| serde_json::json!({"id": r.id, "title": r.title, "text": r.text})).collect::<Vec<_>>()
    })
}

/// The brief's decision section: recent titles, how to read one, the rules,
/// and how the run proposes an entry.
pub(crate) fn section(top: &Path, kind: &str) -> String {
    let text = fs::read_to_string(file(top)).unwrap_or_default();
    let all = records(&text);
    let mut out = String::new();
    if !all.is_empty() {
        let earlier = all.len().saturating_sub(RECENT);
        let titles: Vec<String> =
            all[earlier..].iter().map(|r| format!("- {} {} (lines {}..{})", r.id, r.title, r.lines.0, r.lines.1)).collect();
        out += &format!(
            "\nRecent decisions (titles from DECISIONS.md, each with its lines; `read DECISIONS.md --lines a..b` reads one record):\n{}\n",
            titles.join("\n")
        );
        if earlier > 0 {
            out += &format!("… {earlier} earlier decisions: `read DECISIONS.md` lists them.\n");
        }
    }
    out += RULES;
    out += match kind {
        "review" => "\nA review can't propose one: name any contradiction with a decision in your findings.\n",
        "strategy" => {
            concat!(
                "\nPropose one with `strategy decision` and the entry in `text`: a `## <title>` line, what was decided, and a `Why:` line, ",
                "or `## D-<n> <title>` to change that record. It is applied only when the human approves it.\n"
            )
        }
        "design" => "\nA decision is proposed by the run that settled it, not by a design run.\n",
        _ => {
            concat!(
                "\nPropose one in the run the human settled it, one per proposal: `task decision` with the entry in `text`: a `## <title>` line, ",
                "what was decided, and a `Why:` line, or `## D-<n> <title>` to change that record. It is applied only when the human approves it. ",
                "Solace writes the who, task and date line, so a body never carries one.\n"
            )
        }
    };
    out
}

/// Today's date, as a decision's attribution line writes it: the computer's
/// local day, or UTC when no offset can be found.
pub(crate) fn today() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    date_at(seconds, crate::board::local_offset(seconds).unwrap_or(0))
}

/// Record `D-n`'s text as it stands, for an edit: a number that names no
/// record is refused when the edit is proposed (D-255).
pub(crate) fn current(top: &Path, n: u32) -> Result<String, crate::Failure> {
    let text = fs::read_to_string(file(top)).unwrap_or_default();
    records(&text)
        .into_iter()
        .find(|r| r.id == format!("D-{n}"))
        .map(|r| r.text)
        .ok_or_else(|| crate::Failure(format!("E6021 D-{n} names no decision"), 1))
}

/// The date `seconds` since the epoch falls on at `offset` seconds from UTC.
fn date_at(seconds: i64, offset: i64) -> String {
    let days = (seconds + offset).div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Whether `line` is an entry's attribution line.
fn attribution(line: &str) -> bool {
    line.starts_with('(') && line.ends_with(')') && (line.contains(", board task ") || line.contains(", strategy run "))
}

/// The date `seconds` since the epoch falls on, as a decision's attribution writes it.
pub(crate) fn date_of(seconds: u64) -> String {
    date_at(seconds as i64, crate::board::local_offset(seconds as i64).unwrap_or(0))
}
