//! Compiler diagnostics: source places, errors, warnings, and broken-symbol reports.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
pub(crate) struct FileMap {
    pub(crate) module: String,
    pub(crate) file: PathBuf,
    /// (first line, last line, qualified name)
    pub(crate) ranges: Vec<(usize, usize, String)>,
}

pub(crate) struct Diag {
    pub(crate) error: bool,
    pub(crate) module: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) text: String,
    pub(crate) suggestions: Vec<Suggestion>,
}

pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(c) if ('!'..='/').contains(&c) => {
                chars.next();
                while chars.peek().is_some_and(|c| ('!'..='/').contains(c)) {
                    chars.next();
                }
                if chars.peek().is_some_and(|c| ('0'..='~').contains(c)) {
                    chars.next();
                }
            }
            Some(c) if ('0'..='~').contains(&c) => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Where a file position falls: (module, symbol and its first line). Cargo in
/// a workspace reports a path relative to the workspace root, so more than one
/// file can end with it; an exact match is the shallowest, and the shallowest
/// is the project's own file.
pub(crate) fn locate<'a>(files: &'a [FileMap], path: &str, line: usize) -> Option<(&'a str, Option<(&'a str, usize)>)> {
    let f =
        files.iter().filter(|f| f.file.to_string_lossy() == path || f.file.ends_with(path)).min_by_key(|f| f.file.components().count())?;
    let sym = f.ranges.iter().find(|r| r.0 <= line && line <= r.1).map(|r| (r.2.as_str(), r.0));
    Some((&f.module, sym))
}

fn warning_summary<'a>(diags: impl Iterator<Item = &'a Diag>) -> String {
    let lines: Vec<String> = diags.map(warning_text).collect();
    if lines.is_empty() { String::new() } else { format!("\nWarnings:\n{}\n", lines.join("\n")) }
}

pub(crate) fn broken_report(before: &HashMap<String, String>, diags: &[Diag]) -> String {
    let errors: Vec<&Diag> = diags.iter().filter(|d| d.error).collect();
    let mut names: Vec<String> =
        errors.iter().map(|d| d.symbol.clone().or(d.module.clone().map(|m| m + " imports")).unwrap_or("?".into())).collect();
    crate::unique(&mut names);
    let mut fixed: Vec<&str> = before.keys().map(String::as_str).filter(|k| !names.iter().any(|n| n == k)).collect();
    fixed.sort();
    let fixed = if fixed.is_empty() { String::new() } else { format!("Fixed: {}.\n", fixed.join(", ")) };
    if errors.is_empty() {
        return format!("{fixed}The project type-checks.\n");
    }
    let mut out = format!("\nBroken symbols ({}): {}\n{fixed}", names.len(), names.join(", "));
    let own =
        |sym: &str| errors.iter().filter(|d| d.symbol.as_deref() == Some(sym)).map(|d| d.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let same: Vec<&String> = names.iter().filter(|n| before.get(*n).is_some_and(|t| *t == own(n))).collect();
    if !same.is_empty() {
        out += "Still broken, with the errors shown before (`check` shows them again):\n";
        for n in &same {
            let all: Vec<&&Diag> = errors.iter().filter(|d| d.symbol.as_ref() == Some(n)).collect();
            let more = if all.len() > 1 { format!(" (and {} more errors in it)", all.len() - 1) } else { String::new() };
            out += &format!("  {}: {}{more}\n", place(all[0]), error_message(all[0]));
        }
    }
    // new errors: one summary per shared cause, then the rest grouped by message
    let new: Vec<&Diag> = errors.iter().copied().filter(|d| !d.symbol.as_ref().is_some_and(|s| same.contains(&s))).collect();
    let (caused, rest): (Vec<&Diag>, Vec<&Diag>) = new.into_iter().partition(|d| error_cause(d).is_some());
    if !caused.is_empty() {
        out += &format!("\n{}\n", group_errors(&caused));
    }
    let mut groups: Vec<(String, &Diag, Vec<&Diag>)> = vec![];
    for d in rest {
        let m = error_message(d);
        match groups.iter_mut().find(|g| g.0 == m) {
            Some(g) => g.2.push(d),
            None => groups.push((m, d, vec![])),
        }
    }
    for (_, first, rest) in groups.iter().take(5) {
        out += &format!("\n{}\n", first.text);
        if !rest.is_empty() {
            out += &format!("same error: {} more\n", rest.len());
            for d in rest {
                out += &format!("  at {}: {}\n", place(d), error_line(d));
            }
        }
    }
    let more: usize = groups.iter().skip(5).map(|g| 1 + g.2.len()).sum();
    if more > 0 {
        out += &format!("\n… {more} more errors (`check`)\n");
    }
    out
}

/// `<symbol>, line N` from a diagnostic's `at` line.
pub(crate) fn place(d: &Diag) -> String {
    d.text.lines().find_map(|l| l.trim_start().strip_prefix("at ")).unwrap_or("?").to_string()
}

/// An error's title and the compiler's note at its caret, e.g. "Incorrect
/// arity: Expected 6 arguments, got 5".
fn error_message(d: &Diag) -> String {
    error_kind(&d.text)
}

/// The source line an error is at, from its excerpt.
fn error_line(d: &Diag) -> String {
    let n = place(d).rsplit_once("line ").map_or(String::new(), |(_, n)| n.to_string());
    d.text
        .lines()
        .find_map(|l| l.trim_start().strip_prefix(n.as_str()).and_then(|r| r.strip_prefix(" │")))
        .map_or(String::new(), |r| r.trim_start_matches(|c: char| " │╭╰─·".contains(c)).trim_end().to_string())
}

/// The shared cause of a Rust error that repeats at many sites: a key to group
/// by and the cause written for the summary, or `None` to print it in full.
pub(crate) fn error_cause(d: &Diag) -> Option<(String, String)> {
    let title = d.text.lines().next()?.trim();
    let code = title.strip_prefix("error[")?.split_once(']')?.0;
    if !matches!(code, "E0603" | "E0624" | "E0616" | "E0599") {
        return None;
    }
    let items: Vec<&str> = title.split('`').skip(1).step_by(2).collect();
    if items.is_empty() {
        return None;
    }
    let item = items[0];
    let cause = match code {
        "E0603" | "E0624" => format!("`{item}` is private"),
        "E0616" if items.len() > 1 => format!("`{item}` is private on `{}`", items[1]),
        "E0599" if items.len() > 1 => format!("`{item}` is missing on `{}`", items[1]),
        _ => return None,
    };
    let site = d.module.as_deref().unwrap_or("");
    Some((format!("{code} {site} {}", items.join(" ")), cause))
}

/// The fix for a shared cause, named once instead of at every site.
fn cause_fix(key: &str) -> &'static str {
    if key.starts_with("E0599") { "add the method or fix the call" } else { "make it `pub(crate)`" }
}

/// Errors of one cause print as one line naming the item and the fix, with the
/// first few sites and a count of the rest; every other error prints in full.
pub(crate) fn group_errors(errors: &[&Diag]) -> String {
    const SITE_CAP: usize = 6;
    let mut groups: Vec<(String, String, Vec<&Diag>)> = vec![];
    let mut plain: Vec<&str> = vec![];
    for d in errors {
        match error_cause(d) {
            Some((key, cause)) => match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.2.push(d),
                None => groups.push((key, cause, vec![d])),
            },
            None => plain.push(&d.text),
        }
    }
    let mut parts: Vec<String> = vec![];
    for (key, cause, ds) in groups {
        if ds.len() == 1 {
            parts.push(ds[0].text.clone());
            continue;
        }
        let mut part = format!("{cause} ({} places): {}", ds.len(), cause_fix(&key));
        for &d in ds.iter().take(SITE_CAP) {
            part += &format!("\n  at {}", place(d));
        }
        if ds.len() > SITE_CAP {
            let mut modules: Vec<&str> = vec![];
            for &d in ds.iter().skip(SITE_CAP) {
                if let Some(module) = site_module(d) {
                    modules.push(module);
                }
            }
            crate::unique(&mut modules);
            part += &format!("\n… and {} more", ds.len() - SITE_CAP);
            if !modules.is_empty() {
                part += &format!(" in {}", modules.join(", "));
            }
        }
        parts.push(part);
    }
    parts.extend(plain.into_iter().map(String::from));
    parts.join("\n\n")
}

/// One warning as a submit prints it.
fn warning_text(d: &Diag) -> String {
    format!("  {}: {}", warning_place(d), warning_line(d))
}

/// A warning's place and its `warning:` line. The rendered `at` line is left
/// out: it moves when unrelated code above changes.
pub(crate) fn warning_key(d: &Diag) -> String {
    format!("{}\n{}", warning_place(d), warning_line(d))
}

fn warning_place(d: &Diag) -> &str {
    d.symbol.as_deref().or(d.module.as_deref()).unwrap_or("project")
}

fn warning_line(d: &Diag) -> &str {
    d.text.lines().find(|l| l.trim_start().starts_with("warning:")).unwrap_or("compiler warning").trim()
}

/// Warnings outside the saved symbols that the last check did not have.
fn new_warnings_summary<'a>(before: Option<&HashSet<String>>, diags: impl Iterator<Item = &'a Diag>) -> String {
    let Some(before) = before else { return String::new() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut lines: Vec<String> = vec![];
    for d in diags.filter(|d| !d.error) {
        let key = warning_key(d);
        if !before.contains(&key) && seen.insert(key) {
            lines.push(warning_text(d));
        }
    }
    if lines.is_empty() { String::new() } else { format!("\nNew warnings elsewhere:\n{}\n", lines.join("\n")) }
}

/// The warning sections of a submit reply, with their footer once.
pub(crate) fn warnings_report<'a>(
    before: Option<&HashSet<String>>,
    own: impl Iterator<Item = &'a Diag>,
    elsewhere: impl Iterator<Item = &'a Diag>,
) -> String {
    let own = warning_summary(own);
    let elsewhere = new_warnings_summary(before, elsewhere);
    if own.is_empty() && elsewhere.is_empty() {
        String::new()
    } else {
        format!("{own}{elsewhere}Use `check --warnings` for full diagnostics.\n")
    }
}

pub(crate) fn warning_module(d: &Diag) -> &str {
    d.module.as_deref().unwrap_or("project")
}

/// A site's module, falling back to the part of its name before the last dot.
fn site_module(d: &Diag) -> Option<&str> {
    d.module.as_deref().filter(|m| !m.is_empty()).or_else(|| d.symbol.as_deref()?.rsplit_once('.').map(|(m, _)| m))
}

/// One entry in rustc's output: a fix with the spans it rewrites, or a plain
/// span label such as "private method defined here".
pub(crate) struct Suggestion {
    pub(crate) message: String,
    pub(crate) spans: Vec<SuggestionSpan>,
}

/// A source range a suggestion rewrites; `replacement` is `None` for a note.
pub(crate) struct SuggestionSpan {
    pub(crate) file_name: String,
    pub(crate) line_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_start: usize,
    pub(crate) column_end: usize,
    pub(crate) replacement: Option<String>,
}

/// The errors not in a project's stored broken set: a save's dependent check
/// compares against what was already broken there. An error is new when its
/// symbol and kind were not stored, so a different error on a broken symbol is
/// still new. An error with no symbol is always new (D-103, D-117).
pub(crate) fn new_errors<'a>(before: &HashMap<String, String>, diags: &'a [Diag]) -> Vec<&'a Diag> {
    diags
        .iter()
        .filter(|d| d.error)
        .filter(|d| match d.symbol.as_ref().and_then(|s| before.get(s)) {
            Some(stored) => !stored_has_kind(stored, d),
            None => true,
        })
        .collect()
}

/// An error's title and the compiler's note at its caret, e.g. "Incorrect
/// arity: Expected 6 arguments, got 5". A moved error keeps its kind.
pub(crate) fn error_kind(text: &str) -> String {
    let (title, note) = title_and_note(text);
    if note.is_empty() { title.to_string() } else { format!("{title}: {note}") }
}

/// Whether `stored`, one symbol's diagnostics, already holds this error's
/// title and caret note. A moved line keeps them; a different error does not.
fn stored_has_kind(stored: &str, d: &Diag) -> bool {
    let (title, note) = title_and_note(&d.text);
    stored.lines().any(|line| strip_error_prefix(line) == title) && (note.is_empty() || stored.contains(note))
}

/// An error's title, without the `error:` prefix, and the compiler's note at
/// its caret, if any. A stored error keeps both when its line moves.
fn title_and_note(text: &str) -> (&str, &str) {
    let title = strip_error_prefix(text.lines().next().unwrap_or(""));
    let note = text.lines().rev().find_map(|l| l.rsplit_once('^')).map_or("", |(_, n)| n.trim());
    (title, note)
}

/// A diagnostic line without its `error:` prefix and surrounding space.
fn strip_error_prefix(line: &str) -> &str {
    line.trim_start_matches("error:").trim()
}
