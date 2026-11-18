//! Versions (D-251): the one version a build writes into every manifest in the
//! workspace, the files that hold it, and the lines a failed build restores.

use crate::Failure;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
#[derive(Clone)]
pub struct Edit {
    pub project: String,
    pub file: String,
    pub line: usize,
    pub before: String,
    pub after: String,
}

#[derive(Clone)]
struct Found {
    project: String,
    file: String,
    line: usize,
    prefix: String,
    suffix: String,
    version: String,
}

#[derive(Clone, Copy)]
enum Kind {
    Cargo,
    Gleam,
    Json,
}

/// Every manifest version in the workspace, with the next version and whether
/// they agree.
pub fn json(top: &Path) -> Result<Value, Failure> {
    let found = found(top)?;
    let mut versions = vec![];
    let mut distinct: Vec<&str> = vec![];
    for f in &found {
        versions.push(json!({"project": f.project, "file": f.file, "version": f.version}));
        if !distinct.contains(&f.version.as_str()) {
            distinct.push(&f.version);
        }
    }
    Ok(json!({
        "versions": versions,
        "agree": distinct.len() <= 1,
        "proposal": proposal(top),
        "newest": newest(top).unwrap_or_default(),
    }))
}

/// Refuses a version that is not semver or already has a release.
pub fn check(top: &Path, version: &str) -> Result<(), Failure> {
    if semver(version).is_none() {
        return Err(Failure(format!("E7001 version {version} is not semver like 1.2.3"), 1));
    }
    if taken(top, version) {
        return Err(Failure(format!("E7001 version {version} already has a release"), 1));
    }
    Ok(())
}

pub fn edits(top: &Path, version: &str) -> Result<Vec<Edit>, Failure> {
    let manifests = found(top)?;
    let mut all = manifests.clone();
    all.extend(locks(top, &manifests)?);
    Ok(all
        .into_iter()
        .map(|f| Edit {
            project: f.project,
            file: f.file,
            line: f.line,
            before: format!("{}{}{}", f.prefix, f.version, f.suffix),
            after: format!("{}{}{}", f.prefix, version, f.suffix),
        })
        .collect())
}

/// Writes each edit, putting back the lines it already wrote if one fails.
pub fn write(top: &Path, edits: &[Edit]) -> Result<(), Failure> {
    let mut written = vec![];
    for edit in edits {
        if let Err(e) = write_line(top, edit, &edit.after) {
            restore_edits(top, &written)?;
            return Err(e);
        }
        written.push(edit.clone());
    }
    Ok(())
}

pub fn restore_edits(top: &Path, edits: &[Edit]) -> Result<(), Failure> {
    for edit in edits {
        write_line(top, edit, &edit.before)?;
    }
    Ok(())
}

pub fn entries(edits: &[Edit]) -> Value {
    Value::Array(
        edits.iter().map(|e| json!({"project": e.project, "file": e.file, "line": e.line, "before": e.before, "after": e.after})).collect(),
    )
}

/// Puts a build's version lines back. A build with none, or one already
/// restored, is left alone.
pub fn restore(top: &Path, build: &Value) -> Result<(), Failure> {
    if build["versionsRestored"].as_bool() == Some(true) {
        return Ok(());
    }
    let Some(files) = build["versionFiles"].as_array() else { return Ok(()) };
    for file in files {
        let (Some(project), Some(name), Some(before), Some(after)) =
            (file["project"].as_str(), file["file"].as_str(), file["before"].as_str(), file["after"].as_str())
        else {
            continue;
        };
        let path = crate::scope::path(top, project).join(name);
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let at = file["line"].as_u64().map(|n| n as usize).filter(|n| line_is(&text, *n, after)).or_else(|| find_line(&text, after));
        let Some(at) = at else { continue };
        if let Some(next) = rewrite(&text, at, before) {
            fs::write(&path, next)?;
        }
    }
    Ok(())
}

fn proposal(top: &Path) -> String {
    let base = latest(top).and_then(|v| semver(&v)).unwrap_or([0, 1, 0]);
    format!("{}.{}.{}", base[0], base[1], base[2] + 1)
}

fn taken(top: &Path, version: &str) -> bool {
    crate::releases::listed(top).ok().is_some_and(|listed| {
        listed["releases"].as_array().is_some_and(|releases| releases.iter().any(|r| r["version"].as_str() == Some(version)))
    })
}

fn semver(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out[i] = part.parse().ok()?;
    }
    Some(out)
}

fn found(top: &Path) -> Result<Vec<Found>, Failure> {
    let mut out = vec![];
    for project in crate::workspace::members(top)? {
        let root = crate::scope::path(top, &project);
        for (file, kind) in
            [("Cargo.toml", Kind::Cargo), ("gleam.toml", Kind::Gleam), ("package.json", Kind::Json), ("tauri.conf.json", Kind::Json)]
        {
            let Ok(text) = fs::read_to_string(root.join(file)) else { continue };
            if let Some((line, prefix, suffix, version)) = find(kind, &text) {
                out.push(Found { project: project.clone(), file: file.to_string(), line, prefix, suffix, version });
            }
        }
    }
    Ok(out)
}

fn find(kind: Kind, text: &str) -> Option<(usize, String, String, String)> {
    match kind {
        Kind::Cargo => cargo(text),
        Kind::Gleam => gleam(text),
        Kind::Json => json_line(text),
    }
}

fn cargo(text: &str) -> Option<(usize, String, String, String)> {
    let mut table = String::new();
    for (i, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        let header = line.trim().split('#').next().unwrap_or("").trim();
        if header.starts_with('[') && header.ends_with(']') {
            table = header[1..header.len() - 1].trim().to_string();
            continue;
        }
        if table == "package"
            && let Some(found) = key_value(line, "version")
        {
            return Some((i, found.0, found.1, found.2));
        }
    }
    None
}

fn gleam(text: &str) -> Option<(usize, String, String, String)> {
    for (i, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        if line.trim_start().starts_with('[') {
            return None;
        }
        if let Some(found) = key_value(line, "version") {
            return Some((i, found.0, found.1, found.2));
        }
    }
    None
}

fn key_value(line: &str, key: &str) -> Option<(String, String, String)> {
    let start = line.len() - line.trim_start().len();
    let rest = line[start..].strip_prefix(key)?;
    if rest.starts_with('.') || !rest.trim_start().starts_with('=') {
        return None;
    }
    let eq = line[start + key.len()..].find('=')? + start + key.len();
    let open = line[eq + 1..].find('"')? + eq + 1;
    let close = line[open + 1..].find('"')? + open + 1;
    Some((line[..=open].to_string(), line[close..].to_string(), line[open + 1..close].to_string()))
}

fn json_line(text: &str) -> Option<(usize, String, String, String)> {
    let mut depth = 0i32;
    for (i, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        let bytes = line.as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            match bytes[j] {
                b'"' => {
                    let start = j + 1;
                    let mut end = start;
                    while end < bytes.len() && bytes[end] != b'"' {
                        if bytes[end] == b'\\' {
                            end += 1;
                        }
                        end += 1;
                    }
                    if end >= bytes.len() {
                        return None;
                    }
                    if depth == 1 && &line[start..end] == "version" {
                        let mut colon = end + 1;
                        while colon < bytes.len() && bytes[colon].is_ascii_whitespace() {
                            colon += 1;
                        }
                        if colon < bytes.len() && bytes[colon] == b':' {
                            let mut value = colon + 1;
                            while value < bytes.len() && bytes[value].is_ascii_whitespace() {
                                value += 1;
                            }
                            if value < bytes.len() && bytes[value] == b'"' {
                                let from = value + 1;
                                let mut to = from;
                                while to < bytes.len() && bytes[to] != b'"' {
                                    if bytes[to] == b'\\' {
                                        to += 1;
                                    }
                                    to += 1;
                                }
                                if to < bytes.len() {
                                    return Some((i, line[..=value].to_string(), line[to..].to_string(), line[from..to].to_string()));
                                }
                            }
                        }
                    }
                    j = end + 1;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
    }
    None
}

fn strip(part: &str) -> &str {
    part.trim_end_matches('\n').trim_end_matches('\r')
}

fn write_line(top: &Path, edit: &Edit, line: &str) -> Result<(), Failure> {
    let path = crate::scope::path(top, &edit.project).join(&edit.file);
    let text = fs::read_to_string(&path)?;
    let next = rewrite(&text, edit.line, line).ok_or_else(|| Failure(format!("E7001 {} has no version line", path.display()), 1))?;
    fs::write(&path, next)?;
    Ok(())
}

fn rewrite(text: &str, index: usize, line: &str) -> Option<String> {
    let mut out = String::new();
    let mut found = false;
    for (i, part) in text.split_inclusive('\n').enumerate() {
        if i == index {
            found = true;
            let ending = if part.ends_with("\r\n") {
                "\r\n"
            } else if part.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(line);
            out.push_str(ending);
        } else {
            out.push_str(part);
        }
    }
    found.then_some(out)
}

fn line_is(text: &str, index: usize, line: &str) -> bool {
    text.split_inclusive('\n').nth(index).is_some_and(|part| strip(part) == line)
}

fn find_line(text: &str, line: &str) -> Option<usize> {
    let matches: Vec<usize> = text.split_inclusive('\n').enumerate().filter(|(_, part)| strip(part) == line).map(|(i, _)| i).collect();
    (matches.len() == 1).then(|| matches[0])
}

#[derive(Clone, Copy, PartialEq)]
enum ManifestKind {
    Cargo,
    Gleam,
    Npm,
}

struct Written {
    dir: PathBuf,
    name: Option<String>,
    kind: ManifestKind,
}

#[derive(Default)]
struct CargoBlock {
    name: Option<String>,
    version: Option<(usize, String, String, String)>,
    source: bool,
}

struct JsonField {
    line: usize,
    path: Vec<String>,
    key: String,
    prefix: String,
    suffix: String,
    value: String,
}

/// Lock-file lines for the packages those manifests say the build wrote.
fn locks(top: &Path, manifests: &[Found]) -> Result<Vec<Found>, Failure> {
    let written = written(top, manifests);
    let mut out = vec![];
    let mut cargo_locks = HashSet::new();
    for project in crate::workspace::members(top)? {
        let root = crate::scope::path(top, &project);
        if let Ok(text) = fs::read_to_string(root.join("manifest.toml")) {
            out.extend(gleam_lock(&project, "manifest.toml", &text, &root, &written));
        }
        if root.join("Cargo.toml").exists() {
            let lock = crate::language::cargo::lock_file(&root);
            if cargo_locks.insert(normalize(&lock)) {
                if let Ok(text) = fs::read_to_string(&lock) {
                    let file = relative_path(&root, &lock).to_string_lossy().into_owned();
                    out.extend(cargo_lock(&project, &file, &text, &written));
                }
            }
        }
        if let Ok(text) = fs::read_to_string(root.join("package-lock.json")) {
            out.extend(json_lock(&project, "package-lock.json", &text, &root, &written, "packages"));
        }
        if let Ok(text) = fs::read_to_string(root.join("bun.lock")) {
            out.extend(json_lock(&project, "bun.lock", &text, &root, &written, "workspaces"));
        }
    }
    Ok(out)
}

fn written(top: &Path, manifests: &[Found]) -> Vec<Written> {
    let mut out = vec![];
    for f in manifests {
        let kind = match f.file.as_str() {
            "Cargo.toml" => ManifestKind::Cargo,
            "gleam.toml" => ManifestKind::Gleam,
            "package.json" => ManifestKind::Npm,
            _ => continue,
        };
        let path = crate::scope::path(top, &f.project).join(&f.file);
        let Ok(text) = fs::read_to_string(path) else { continue };
        let name = match kind {
            ManifestKind::Cargo => cargo_name(&text),
            ManifestKind::Gleam => gleam_name(&text),
            ManifestKind::Npm => serde_json::from_str::<Value>(&text).ok().and_then(|v| v["name"].as_str().map(String::from)),
        };
        out.push(Written { dir: crate::scope::path(top, &f.project), name, kind });
    }
    out
}

fn cargo_name(text: &str) -> Option<String> {
    let mut table = String::new();
    for part in text.split_inclusive('\n') {
        let line = strip(part);
        let header = line.trim().split('#').next().unwrap_or("").trim();
        if header.starts_with('[') && header.ends_with(']') {
            table = header[1..header.len() - 1].trim().to_string();
            continue;
        }
        if table == "package"
            && let Some((_, _, name)) = key_value(line, "name")
        {
            return Some(name);
        }
    }
    None
}

fn gleam_name(text: &str) -> Option<String> {
    for part in text.split_inclusive('\n') {
        let line = strip(part);
        if line.trim_start().starts_with('[') {
            return None;
        }
        if let Some((_, _, name)) = key_value(line, "name") {
            return Some(name);
        }
    }
    None
}

fn gleam_lock(project: &str, file: &str, text: &str, base: &Path, written: &[Written]) -> Vec<Found> {
    let members: Vec<&Written> = written.iter().filter(|w| w.kind == ManifestKind::Gleam).collect();
    let mut out = vec![];
    for (i, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        if field(line, "source").as_deref() != Some("local") {
            continue;
        }
        let Some((prefix, suffix, version)) = quoted_field(line, "version") else { continue };
        let name = field(line, "name");
        let target = match field(line, "path") {
            Some(path) => members.iter().any(|w| normalize(&base.join(&path)) == normalize(&w.dir) && (name.is_none() || w.name == name)),
            None => members.iter().any(|w| name.is_some() && w.name == name),
        };
        if target {
            out.push(Found { project: project.into(), file: file.into(), line: i, prefix, suffix, version });
        }
    }
    out
}

fn cargo_lock(project: &str, file: &str, text: &str, written: &[Written]) -> Vec<Found> {
    let names: HashSet<&str> = written.iter().filter(|w| w.kind == ManifestKind::Cargo).filter_map(|w| w.name.as_deref()).collect();
    let mut out = vec![];
    let mut block = CargoBlock::default();
    let mut in_package = false;
    for (i, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        let head = line.trim_start();
        if head.starts_with("[[package]]") {
            flush_cargo(&mut out, project, file, &mut block, &names);
            in_package = true;
            continue;
        }
        if head.starts_with('[') && head.ends_with(']') {
            flush_cargo(&mut out, project, file, &mut block, &names);
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some((_, _, name)) = key_value(line, "name") {
            block.name = Some(name);
        }
        if let Some((prefix, suffix, version)) = key_value(line, "version") {
            block.version = Some((i, prefix, suffix, version));
        }
        if key_value(line, "source").is_some() {
            block.source = true;
        }
    }
    flush_cargo(&mut out, project, file, &mut block, &names);
    out
}

fn flush_cargo(out: &mut Vec<Found>, project: &str, file: &str, block: &mut CargoBlock, names: &HashSet<&str>) {
    let gathered = std::mem::take(block);
    if let (Some(name), Some((line, prefix, suffix, version))) = (gathered.name, gathered.version)
        && !gathered.source
        && names.contains(name.as_str())
    {
        out.push(Found { project: project.into(), file: file.into(), line, prefix, suffix, version });
    }
}

fn json_lock(project: &str, file: &str, text: &str, base: &Path, written: &[Written], section: &str) -> Vec<Found> {
    let members: Vec<&Written> = written.iter().filter(|w| w.kind == ManifestKind::Npm).collect();
    let here = members.iter().any(|w| normalize(&w.dir) == normalize(base));
    let mut out = vec![];
    for field in json_fields(text) {
        if field.key != "version" {
            continue;
        }
        let target = if field.path.len() == 1 && field.path[0].is_empty() {
            here
        } else if field.path.len() == 3 && field.path[0].is_empty() && field.path[1] == section {
            if field.path[2].is_empty() { here } else { members.iter().any(|w| same_key(&field.path[2], base, &w.dir)) }
        } else {
            false
        };
        if target {
            out.push(Found {
                project: project.into(),
                file: file.into(),
                line: field.line,
                prefix: field.prefix,
                suffix: field.suffix,
                version: field.value,
            });
        }
    }
    out
}

fn same_key(key: &str, base: &Path, dir: &Path) -> bool {
    normalize(Path::new(key)) == relative_path(base, dir)
}

fn relative_path(base: &Path, dir: &Path) -> PathBuf {
    let base = normalize(base);
    let dir = normalize(dir);
    let base: Vec<Component> = base.components().collect();
    let dir: Vec<Component> = dir.components().collect();
    let mut common = 0;
    while common < base.len() && common < dir.len() && base[common] == dir[common] {
        common += 1;
    }
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for part in &dir[common..] {
        out.push(part.as_os_str());
    }
    out
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(part.as_os_str()),
        }
    }
    out
}

fn json_fields(text: &str) -> Vec<JsonField> {
    let mut out = vec![];
    let mut stack: Vec<String> = vec![];
    for (line_no, part) in text.split_inclusive('\n').enumerate() {
        let line = strip(part);
        let bytes = line.as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            let c = bytes[j];
            if c == b'/' && j + 1 < bytes.len() && bytes[j + 1] == b'/' {
                break;
            }
            if c == b'"' {
                let Some((key, mut after)) = read_string(line, j) else { break };
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    after += 1;
                    while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                        after += 1;
                    }
                    if after < bytes.len() && bytes[after] == b'"' {
                        if let Some((value, next)) = read_string(line, after) {
                            out.push(JsonField {
                                line: line_no,
                                path: stack.clone(),
                                key,
                                prefix: line[..=after].to_string(),
                                suffix: line[next - 1..].to_string(),
                                value,
                            });
                            j = next;
                            continue;
                        }
                    } else if after < bytes.len() && (bytes[after] == b'{' || bytes[after] == b'[') {
                        stack.push(key);
                        j = after + 1;
                        continue;
                    } else {
                        j = after;
                        continue;
                    }
                }
                j = after;
                continue;
            }
            match c {
                b'{' | b'[' => stack.push(String::new()),
                b'}' | b']' => {
                    stack.pop();
                }
                _ => {}
            }
            j += 1;
        }
    }
    out
}

fn read_string(line: &str, start: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return Some((line[start + 1..i].to_string(), i + 1));
        }
        i += 1;
    }
    None
}

fn field(line: &str, key: &str) -> Option<String> {
    quoted_field(line, key).map(|(_, _, value)| value)
}

fn quoted_field(line: &str, key: &str) -> Option<(String, String, String)> {
    let mut from = 0;
    while let Some(at) = line[from..].find(key).map(|n| from + n) {
        let boundary = at == 0 || line[..at].chars().next_back().is_some_and(|c| c.is_whitespace() || c == '{' || c == ',');
        let after = at + key.len();
        if boundary {
            let rest = &line[after..];
            if let Some(eq) = rest.find('=')
                && rest[..eq].chars().all(char::is_whitespace)
                && let Some(open) = rest[eq + 1..].find('"')
            {
                let start = after + eq + 1 + open;
                let close = line[start + 1..].find('"').map(|n| n + start + 1)?;
                return Some((line[..=start].to_string(), line[close..].to_string(), line[start + 1..close].to_string()));
            }
        }
        from = after;
    }
    None
}

/// The version a project's own manifests hold, in the order a build writes them.
pub fn at(root: &Path) -> Option<String> {
    [("Cargo.toml", Kind::Cargo), ("gleam.toml", Kind::Gleam), ("package.json", Kind::Json), ("tauri.conf.json", Kind::Json)]
        .iter()
        .find_map(|(file, kind)| {
            let text = fs::read_to_string(root.join(file)).ok()?;
            find(*kind, &text).map(|(_, _, _, version)| version)
        })
}

/// The version the project `dir` holds, when its manifests name one.
pub fn of(top: &Path, dir: &str) -> Option<String> {
    at(&crate::scope::path(top, dir))
}

/// The highest released semver version, for a warning about an older one.
fn newest(top: &Path) -> Option<String> {
    released(top).into_iter().max_by_key(|(_, parts)| *parts).map(|(version, _)| version)
}

/// Each release's semver version, in the order the release list holds them.
fn released(top: &Path) -> Vec<(String, [u64; 3])> {
    let Some(list) = crate::releases::listed(top).ok() else {
        return vec![];
    };
    let Some(records) = list.get("releases").and_then(Value::as_array) else {
        return vec![];
    };
    records
        .iter()
        .filter_map(|r| {
            let version = r["version"].as_str()?;
            semver(version).map(|parts| (version.to_string(), parts))
        })
        .collect()
}

/// The most recent release's semver version, for the next version proposed.
fn latest(top: &Path) -> Option<String> {
    released(top).first().map(|(version, _)| version.clone())
}
