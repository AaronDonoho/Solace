//! The project's skills (D-27): instruction files in a folder the humans own.
//! A run's brief lists them, one line each; the agent `read`s the ones whose
//! description matches its task. Solace never runs a skill's scripts: a skill
//! that needs a tool names a `[commands]` entry the human adds to solace.toml.

use super::*;

use std::collections::HashMap;
/// One skill the brief lists: its name, its one-line description, and the
/// path to `read`.
pub struct Skill {
    pub name: String,
    pub line: String,
    pub path: String,
}

/// How many characters a brief line carries for one skill.
const LINE_CAP: usize = 200;

/// What the brief says above the skills it lists (D-27).
const ADVICE: &str = "the project's house rules, from its humans; when the task matches a skill's description, `read` it before starting, and read any files it points to";

/// The project's skills (D-27): each subfolder of the skills folder that holds
/// a `SKILL.md`, and each markdown file at its top. A `README.md` at the top
/// is the folder's own note for humans, not a skill. Sorted by name; no
/// skills folder, no skills.
pub fn list(root: &Path) -> Vec<Skill> {
    let dir = lockdown::settings(root).map(|v| lockdown::skills_dir(&v)).unwrap_or_else(|_| lockdown::DEFAULT_SKILLS.to_string());
    let base = root.join(&dir);
    let Ok(entries) = fs::read_dir(&base) else { return vec![] };
    let mut out = vec![];
    for e in entries.flatten() {
        let stem = e.file_name().to_string_lossy().to_string();
        let (path, file, base_name) = if e.file_type().is_ok_and(|t| t.is_dir()) {
            (format!("{dir}/{stem}/SKILL.md"), base.join(&stem).join("SKILL.md"), stem.clone())
        } else if stem.ends_with(".md") && stem != "README.md" {
            (format!("{dir}/{stem}"), base.join(&stem), stem.trim_end_matches(".md").to_string())
        } else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let (front, body) = front_matter(&text);
        let named = |k: &str| front.get(k).filter(|v| !v.is_empty()).cloned();
        out.push(Skill {
            name: named("name").unwrap_or(base_name),
            line: cap(&named("description").unwrap_or_else(|| first_line(body))),
            path,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    out
}

/// The brief's Skills section (D-27): one line per skill — name, description
/// and the path to `read` — and when to read one. It never carries a skill's
/// contents. Empty when the project has no skills, so the brief stays as it was.
pub fn section(root: &Path) -> String {
    let skills = list(root);
    if skills.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = skills.iter().map(|s| format!("- {}: {} (`read {}`)", s.name, s.line, s.path)).collect();
    format!("\nSkills ({ADVICE}):\n{}\n", lines.join("\n"))
}

/// A skill file's front matter (D-27): the `name` and `description` between
/// `---` lines at its start, quotes stripped, with the text after the block.
/// A file that doesn't open with the block, or never closes it, has none.
fn front_matter(text: &str) -> (HashMap<String, String>, &str) {
    let mut map = HashMap::new();
    let mut off = 0;
    let mut open = false;
    // `split_inclusive`, not `lines`: `lines` drops the `\r` of a CRLF line,
    // so an offset counted from its length lands inside the closing `---`.
    for l in text.split_inclusive('\n') {
        let line = l.trim_end_matches(['\r', '\n']);
        if !open {
            if off != 0 || line != "---" {
                return (HashMap::new(), text);
            }
            open = true;
        } else if line == "---" {
            let rest = text.get(off + l.len()..).unwrap_or("");
            return (map, rest);
        } else if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_lowercase();
            if matches!(k.as_str(), "name" | "description") {
                map.insert(k, v.trim().trim_matches('"').trim_matches('\'').trim().to_string());
            }
        }
        off += l.len();
    }
    (HashMap::new(), text)
}

/// The line a skill with no description is listed by (D-27): the first
/// non-empty line of its body, with leading `#` marks removed.
fn first_line(body: &str) -> String {
    body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or("(no description)").to_string()
}

/// A brief line's length cap.
fn cap(s: &str) -> String {
    s.chars().take(LINE_CAP).collect()
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
