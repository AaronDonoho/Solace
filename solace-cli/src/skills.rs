//! The project's skills (D-27): instruction files in a folder the humans own.
//! A run's brief lists the unscoped skills of the project it works in, and
//! carries the skills for its role in full. Solace never runs a skill's
//! scripts: a skill that needs a tool names a `[commands]` entry the human
//! adds to solace.toml.

use super::*;

use std::collections::HashMap;
use std::path::PathBuf;
use serde_json::{json, Value};
/// One skill the brief lists: its name, its one-line description, and the
/// path to `read`. A skill with a `roles` key belongs only to the roles it
/// names, and to none when it names none; one with no `roles` key belongs to
/// every role, as skills always did.
pub struct Skill {
    pub name: String,
    pub line: String,
    pub path: String,
    pub body: String,
    pub roles: Vec<String>,
    /// Whether the skill file has a `roles` key.
    pub scoped: bool,
}

/// How many characters a brief line carries for one skill.
const LINE_CAP: usize = 200;

/// What the brief says above the skills for every role (D-27).
const ADVICE: &str = "the project's house rules, from its humans; when the task matches a skill's description, `read` it before starting, and read any files it points to";

/// The project's skills (D-27): each subfolder of the skills folder that holds
/// a `SKILL.md`, and each markdown file at its top. A `README.md` at the top
/// is the folder's own note for humans, not a skill. Sorted by name; no
/// skills folder, no skills.
pub fn list(root: &Path) -> Vec<Skill> {
    let (dir, base) = location(root);
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
        let roles = roles_of(&front);
        let scoped = front.contains_key("roles");
        out.push(Skill {
            name: named("name").unwrap_or(base_name),
            line: cap(&named("description").unwrap_or_else(|| first_line(body))),
            path,
            body: body.trim().to_string(),
            scoped,
            roles,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    out
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
            if matches!(k.as_str(), "name" | "description" | "roles") {
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

/// What the brief says above a run's role skills, which it carries in full.
const ROLE_ADVICE: &str = "these skills are for this run's role; follow them";

/// The most characters a skill saved from Settings may hold.
const LIMIT: usize = 1000;

/// A run's Skills section (D-27): `own`'s skills with no `roles` key, one line
/// each, and the full text of the skills assigned to `kind`'s role from every
/// root. A skill for another role is left out, and so is a skill with no key
/// outside `own`, whose path would point somewhere the run can't read.
pub fn section_for(roots: &[&Path], own: Option<&Path>, kind: &str) -> String {
    let role = crate::board::roles::for_run(kind);
    let mut seen: Vec<&Path> = vec![];
    let mut listed = vec![];
    let mut held = vec![];
    for root in roots {
        if seen.contains(root) {
            continue;
        }
        seen.push(root);
        for skill in list(root) {
            if !skill.scoped {
                if own == Some(*root) {
                    listed.push(format!("- {}: {} (`read {}`)", skill.name, skill.line, skill.path));
                }
            } else if role.is_some_and(|role| skill.roles.iter().any(|id| id == role.id)) {
                held.push(format!("### {}\n{}\n", skill.name, skill.body));
            }
        }
    }
    let mut out = String::new();
    if !listed.is_empty() {
        out += &format!("\nSkills ({ADVICE}):\n{}\n", listed.join("\n"));
    }
    if !held.is_empty() {
        let name = role.map(|role| role.name).unwrap_or("");
        out += &format!("\nSkills for the {name} role ({ROLE_ADVICE}):\n{}\n", held.join("\n"));
    }
    out
}

/// The skills Settings lists: every skill in the folder, whether Settings
/// manages it, and whether it reaches every role.
pub fn api(root: &Path) -> Value {
    let (dir, _) = location(root);
    let file_depth = dir.split('/').count() + 1;
    Value::Array(
        list(root)
            .into_iter()
            .map(|skill| {
                json!({
                    "name": skill.name,
                    "description": skill.line,
                    "body": skill.body,
                    "roles": skill.roles,
                    "managed": skill.scoped && skill.path.split('/').count() == file_depth,
                    "all_roles": !skill.scoped,
                })
            })
            .collect(),
    )
}

/// Saves a skill from Settings. `original` names the skill before an edit, so
/// a renamed skill leaves no file behind.
pub fn save(root: &Path, body: &Value) -> Result<(), Failure> {
    let name = skill_name(body["name"].as_str())?;
    let original = match body["original"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(old) => Some(skill_name(Some(old))?),
        None => None,
    };
    let description = body["description"].as_str().unwrap_or("").trim();
    if description.contains(['\n', '\r']) || description == "---" {
        return fail("Keep the description on one line.");
    }
    let text = body["body"].as_str().unwrap_or("").trim();
    if text.is_empty() {
        return fail("Give the skill a body.");
    }
    let count = text.chars().count();
    if count > LIMIT {
        return fail(format!("A skill can hold at most {LIMIT} characters; this one has {count}."));
    }
    let roles = role_ids(&body["roles"])?;
    let (_, dir) = location(root);
    let target = dir.join(format!("{name}.md"));
    let previous = original.as_ref().map(|old| dir.join(format!("{old}.md")));
    if dir.join(&name).is_dir() {
        return fail(format!("A skill folder already uses the name {name}."));
    }
    if let Some(previous) = &previous {
        if !previous.is_file() {
            return fail(format!("There is no skill named {}.", original.as_deref().unwrap_or("")));
        }
        if !settings_skill(previous) {
            return fail(format!("{} is not a skill Settings manages.", original.as_deref().unwrap_or("")));
        }
        if original.as_deref() != Some(name.as_str()) && target.exists() {
            return fail(format!("A skill named {name} already exists."));
        }
    } else if target.exists() {
        return fail(format!("A skill named {name} already exists."));
    }
    fs::create_dir_all(&dir)?;
    let file = format!("---\nname: {name}\ndescription: {description}\nroles: {}\n---\n\n{text}\n", roles.join(", "));
    fsx::write_atomic(&target, file.as_bytes())?;
    if let Some(previous) = previous.filter(|old| *old != target) {
        fs::remove_file(previous)?;
    }
    Ok(())
}

/// Removes a skill Settings manages. A folder skill is left to its human.
pub fn remove(root: &Path, name: &str) -> Result<(), Failure> {
    let name = skill_name(Some(name))?;
    let (_, dir) = location(root);
    let file = dir.join(format!("{name}.md"));
    if !file.is_file() {
        return fail(format!("There is no skill named {name}."));
    }
    if !settings_skill(&file) {
        return fail(format!("{name} is not a skill Settings manages."));
    }
    fs::remove_file(file)?;
    Ok(())
}

/// The skills folder's name and path (D-27).
fn location(root: &Path) -> (String, PathBuf) {
    let dir = lockdown::settings(root).map(|v| lockdown::skills_dir(&v)).unwrap_or_else(|_| lockdown::DEFAULT_SKILLS.to_string());
    let base = root.join(&dir);
    (dir, base)
}

/// A skill name as it becomes a file name: no path, no hidden name, and
/// `README` stays the folder's own note.
fn skill_name(value: Option<&str>) -> Result<String, Failure> {
    let name = value.unwrap_or("").trim().trim_end_matches(".md").trim();
    if name.is_empty() {
        return fail("Give the skill a name.");
    }
    if name.eq_ignore_ascii_case("README") {
        return fail("README is the skills folder's own note, not a skill.");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
        return fail("A skill name can use only letters, digits, spaces, - and _.");
    }
    Ok(name.to_string())
}

/// The role ids a Settings save names, each one this Solace knows. An empty
/// list is a skill no role receives.
fn role_ids(value: &Value) -> Result<Vec<String>, Failure> {
    let mut out: Vec<String> = vec![];
    for id in value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|id| !id.is_empty()) {
        if crate::board::roles::from_id(id).is_none() {
            return fail(format!("Unknown role {id}."));
        }
        if !out.iter().any(|known| known == id) {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

/// The roles a skill file names, empty when it has no `roles` key.
fn roles_of(front: &HashMap<String, String>) -> Vec<String> {
    front
        .get("roles")
        .map(|roles| roles.split(|c: char| c == ',' || c.is_whitespace()).filter(|id| !id.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// A refused skill save.
fn fail<T>(message: impl std::fmt::Display) -> Result<T, Failure> {
    Err(Failure(message.to_string(), 1))
}

/// Whether a skill file is one Settings manages: its front matter has a
/// `roles` key, even when it names none.
fn settings_skill(file: &Path) -> bool {
    fs::read_to_string(file).is_ok_and(|text| {
        let (front, _) = front_matter(&text);
        front.contains_key("roles")
    })
}
