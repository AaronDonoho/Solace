//! Rust projects' tools behind the store (D-158 points 3 and 5): `cargo
//! check`'s JSON diagnostics located by symbol, `cargo test`'s failures by
//! libtest name, rustfmt for one item at a time, and the message for a
//! machine without the Rust toolchain.

use crate::fsx;
use crate::diagnostics::strip_ansi;
use crate::diagnostics::locate;
use crate::failure::fail;
use crate::diagnostics::FileMap;
use crate::failure::Failure;
use crate::diagnostics::Diag;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::lockdown::commands::command;
use crate::lockdown::commands::background;
use crate::lockdown::commands::passed;

/// A Rust project's commands need this on `PATH`; without it they are refused.
pub const MISSING: &str = "Needs the Rust toolchain (`cargo`)";

/// With a Rust check that found errors: borrow errors appear only once the
/// type errors are fixed, so one fix can take a second round (D-158 point 3).
pub const ROUND_NOTE: &str = "Fixing these may show more errors: Rust checks borrows only once types check.";

/// Whether `cargo` can be used: it is on `PATH` and Rust is turned on in the
/// system settings. An unreadable or invalid settings file leaves it unusable.
/// Callers that must report that error use `language::reason` instead.
/// Looked up on every call, so a toolchain installed while a server runs is
/// found without restarting it.
pub fn installed() -> bool {
    search_path("cargo").is_some() && crate::language::enabled(crate::language::RUST).unwrap_or(false)
}

fn search_path(program: &str) -> Option<PathBuf> {
    crate::language::paths::search_path(program)
}

/// The project's own build folder, so every check reuses it (D-158 point 3).
fn target_dir(root: &Path) -> String {
    root.join("target").to_string_lossy().to_string()
}

/// `cargo check`'s arguments, as `Store::cargo` runs them.
pub fn check_args(root: &Path) -> Vec<String> {
    let target = target_dir(root);
    ["check", "--all-targets", "--message-format=json", "--offline", "--target-dir", target.as_str()]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `cargo test`'s arguments, as `Store::cargo` runs them: the tests named by
/// libtest name run with `--exact`, and every test runs when there are none.
pub fn test_args(root: &Path, filters: &[String]) -> Vec<String> {
    let target = target_dir(root);
    let mut args: Vec<String> =
        ["test", "--message-format=json", "--offline", "--target-dir", target.as_str()].iter().map(|s| s.to_string()).collect();
    if !filters.is_empty() {
        args.push("--".into());
        args.push("--exact".into());
        args.extend(filters.iter().cloned());
    }
    args
}

/// `cargo build`'s arguments, as `build` runs them (D-158 point 4): the
/// project's own target folder, offline; the caller adds `cargo`.
pub fn build_args(root: &Path) -> Vec<String> {
    let target = target_dir(root);
    ["build", "--offline", "--target-dir", target.as_str()].iter().map(|s| s.to_string()).collect()
}

/// Starts a check in the background so the first one is warm, without making
/// its caller wait (D-158 point 3).
pub fn warm(root: &Path) {
    let mut argv = vec!["cargo".to_string()];
    argv.extend(check_args(root));
    // Through `lockdown::background`, the path every command takes, so a
    // migration's sandbox covers this build too.
    let _ = background(root, &argv, true);
}

/// Fetches the project's dependencies outside the sandbox, where the network is
/// allowed (D-158 point 4). Every check runs `--offline`, so this runs after an
/// import and after an approved Cargo.toml change. A failed fetch puts the
/// workspace's `Cargo.lock` back, so the step that asked for it changes nothing.
pub fn fetch(root: &Path) -> Result<(), Failure> {
    let had_solace = root.join(".solace").exists();
    let lock = lock_file(root);
    let _fetch = fetch_lock(root, had_solace)?;
    let before = fs::read(&lock).ok();
    let out = fetch_online(root, &lock, before, had_solace);
    if out.is_ok() {
        let _ = fs::remove_file(root.join(".solace/fetch-failed"));
    }
    out
}

/// Fetches only when the cache is missing something: an offline fetch first,
/// then a real one (D-176). Called with the fetch lock held.
fn fetch_if_needed_now(root: &Path, lock: &Path) -> Result<(), Failure> {
    let had_solace = root.join(".solace").exists();
    let before = fs::read(lock).ok();
    let argv: Vec<String> = ["cargo", "fetch", "--offline"].iter().map(|s| s.to_string()).collect();
    if matches!(command(root, &argv, 120, 4000, false, false), Ok(out) if passed(&out)) {
        return Ok(());
    }
    fetch_online(root, lock, before, had_solace)
}

/// A reopen fetch, with its result kept on the project's lane (D-176). The
/// fetch lock keeps it from overlapping an approved fetch, which would restore
/// an older `Cargo.lock` and put back a failure the approval cleared.
pub fn reopen(root: &Path) -> Result<(), Failure> {
    let had_solace = root.join(".solace").exists();
    let lock = lock_file(root);
    let _fetch = fetch_lock(root, had_solace)?;
    let marker = root.join(".solace/fetch-failed");
    let out = fetch_if_needed_now(root, &lock);
    match &out {
        Ok(()) => {
            let _ = fs::remove_file(&marker);
        }
        Err(e) => {
            let _ = fs::write(&marker, e.0.trim());
        }
    }
    out
}

/// Serializes fetches of the Solace root that contains `root`: a member's
/// approved fetch and the root's reopen can change the same `Cargo.lock`, which
/// may sit above both (D-176). The first import of a standalone project has no
/// `.solace` and no approved fetch or reopen to overlap; a member's first import
/// shares its root's lock.
fn fetch_lock(root: &Path, had_solace: bool) -> Result<Option<fs::File>, Failure> {
    let dir = fetch_lock_dir(root)?;
    if !had_solace && dir == root {
        return Ok(None);
    }
    if !dir.join(".solace").is_dir() {
        return Ok(None);
    }
    Ok(Some(fsx::lock_file(&dir.join(".solace/fetch.lock"))?))
}

/// The Solace root whose fetch lock `root` takes, or `root` itself when it has
/// no root above it.
fn fetch_lock_dir(root: &Path) -> Result<PathBuf, Failure> {
    Ok(crate::workspace::enclosing(root)?.unwrap_or_else(|| root.to_path_buf()))
}

/// The `Cargo.lock` Cargo uses for `root`: a workspace member's is the
/// workspace's, not one in the member (D-176). Cargo names the workspace's
/// manifest; without cargo, fall back to the folder's own lock.
pub(crate) fn lock_file(root: &Path) -> PathBuf {
    let argv: Vec<String> = ["cargo", "locate-project", "--workspace", "--message-format", "plain"].iter().map(|s| s.to_string()).collect();
    match command(root, &argv, 120, 4000, false, false) {
        Ok(out) if passed(&out) => out
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|l| !l.is_empty())
            .and_then(|p| Path::new(p).parent())
            .map_or_else(|| root.join("Cargo.lock"), |d| d.join("Cargo.lock")),
        _ => root.join("Cargo.lock"),
    }
}

fn fetch_online(root: &Path, lock: &Path, before: Option<Vec<u8>>, had_solace: bool) -> Result<(), Failure> {
    let argv: Vec<String> = ["cargo", "fetch"].iter().map(|s| s.to_string()).collect();
    match command(root, &argv, 600, 4000, false, false) {
        Ok(out) if passed(&out) => Ok(()),
        Ok(out) => Err(fetch_failed(root, lock, before, had_solace, format!("cargo fetch failed:\n{}", out.trim_end()))),
        Err(e) => Err(fetch_failed(root, lock, before, had_solace, format!("cargo fetch failed: {}", e.0.trim_end()))),
    }
}

/// Puts `Cargo.lock` back after a failed fetch, removes a `.solace/` the fetch
/// itself created, and returns the failure.
fn fetch_failed(root: &Path, lock: &Path, before: Option<Vec<u8>>, had_solace: bool, message: String) -> Failure {
    match before {
        Some(bytes) => {
            let _ = fs::write(lock, bytes);
        }
        None => {
            let _ = fs::remove_file(lock);
        }
    }
    if !had_solace {
        let _ = fs::remove_dir(root.join(".solace"));
    }
    Failure(message, 1)
}

/// `cargo check --message-format=json`'s compiler messages as diagnostics
/// located by symbol (D-158 point 3). Each message's primary span is mapped
/// to its symbol and followed out of a macro to its call site; one with no
/// span is dropped, and one reported once per target is shown once.
pub fn diagnostics(output: &str, files: &[FileMap]) -> Vec<Diag> {
    let mut diags: Vec<Diag> = vec![];
    let mut seen: Vec<String> = vec![];
    for line in output.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        if value["reason"] != "compiler-message" {
            continue;
        }
        let message = &value["message"];
        let Some(span) = primary_span(message) else { continue };
        let Some(rendered) = message["rendered"].as_str() else { continue };
        let path = span["file_name"].as_str().unwrap_or("");
        let row = span["line_start"].as_u64().unwrap_or(0) as usize;
        let place = location(files, path, row);
        let (module, symbol) = match locate(files, path, row) {
            Some((module, Some((name, _)))) if !is_probe_range(name) => (Some(module.to_string()), Some(name.to_string())),
            Some((module, _)) => (Some(module.to_string()), None),
            None => (None, None),
        };
        let text = rewrite(rendered, files, place.as_deref());
        if seen.contains(&text) {
            continue;
        }
        seen.push(text.clone());
        diags.push(Diag { error: message["level"] == "error", module, symbol, text, suggestions: suggestions(message) });
    }
    diags
}

/// A `cargo check` that failed with no located diagnostic — a build script
/// that panicked or couldn't reach the network, say — as the text to report
/// (D-158 point 4). A diagnostic cargo emitted without a span is kept;
/// `--message-format=json`'s other lines and the progress lines are dropped.
pub fn failure_text(output: &str) -> String {
    let mut parts: Vec<String> = vec![];
    for line in strip_ansi(output).lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{') {
            let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
            if value["reason"] == "compiler-message" && value["message"]["level"] == "error" && primary_span(&value["message"]).is_none() {
                if let Some(rendered) = value["message"]["rendered"].as_str().map(str::trim_end).filter(|r| !r.is_empty()) {
                    parts.push(rendered.to_string());
                }
            }
            continue;
        }
        if ["Checking ", "Compiling ", "Finished ", "warning: build failed"].iter().any(|p| trimmed.starts_with(p)) {
            continue;
        }
        parts.push(line.to_string());
    }
    let text = parts.join("\n").trim().to_string();
    if text.is_empty() { "cargo check failed".to_string() } else { text }
}

/// The primary span of a message, followed out of any macro expansion to the
/// call site that produced it.
fn primary_span(message: &Value) -> Option<Value> {
    let span = message["spans"].as_array()?.iter().find(|s| s["is_primary"] == true)?;
    let mut span = span.clone();
    while let Some(inner) = span.get("expansion").and_then(|e| e.get("span")).filter(|s| !s.is_null()) {
        span = inner.clone();
    }
    Some(span)
}

/// `rendered` without colors, its first location replaced by the message's
/// primary span and later ones by their own spans, as `gleam::diagnostics`
/// rewrites Gleam's.
fn rewrite(rendered: &str, files: &[FileMap], primary: Option<&str>) -> String {
    let clean = strip_ansi(rendered);
    let mut out: Vec<String> = vec![];
    let mut path: Option<String> = None;
    let mut first = true;
    let mut probe = false;
    for line in clean.lines() {
        if let Some((at, row)) = source_location(line) {
            path = Some(at.to_string());
            let place = if first { primary.map(str::to_string) } else { None }.or_else(|| location(files, at, row));
            first = false;
            probe = locate(files, at, row).is_some_and(|(_, sym)| sym.is_some_and(|(name, _)| name == crate::PROBE_RANGE));
            out.push(place.unwrap_or_else(|| line.to_string()));
            continue;
        }
        if let Some(current) = path.as_deref() {
            if let Some((line, at_probe)) = probe_gutter(line, current, files) {
                probe = at_probe;
                out.push(line);
                continue;
            }
            if probe {
                if let Some(line) = probe_continuation(line) {
                    out.push(line);
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    out.join("\n").trim_end().to_string()
}

/// libtest's totals across every test binary: (passed, failed).
pub fn totals(output: &str) -> (usize, usize) {
    let mut sums = (0, 0);
    for line in strip_ansi(output).lines() {
        let Some(rest) = line.trim_start().strip_prefix("test result:") else { continue };
        let number = |word: &str| -> Option<usize> { rest.split(word).next()?.split_whitespace().last()?.parse().ok() };
        if let (Some(passed), Some(failed)) = (number(" passed"), number(" failed")) {
            sums.0 += passed;
            sums.1 += failed;
        }
    }
    sums
}

/// Failed tests in `cargo test` output by libtest name (`board::tests::x`),
/// each with the block libtest prints for it.
pub fn failures(output: &str) -> BTreeMap<String, (String, String)> {
    fn flush(name: &mut Option<String>, lines: &mut Vec<String>, out: &mut BTreeMap<String, String>) {
        if let Some(name) = name.take() {
            out.insert(name, lines.join("\n").trim_end().to_string());
            lines.clear();
        }
    }
    let clean = strip_ansi(output);
    let (mut out, mut name, mut lines): (BTreeMap<String, String>, Option<String>, Vec<String>) = (BTreeMap::new(), None, vec![]);
    for line in clean.lines() {
        if let Some(test) = line.trim_start().strip_prefix("---- ").and_then(|rest| rest.strip_suffix(" stdout ----")) {
            flush(&mut name, &mut lines, &mut out);
            name = Some(test.trim().to_string());
            lines.push(line.to_string());
            continue;
        }
        // A test's captured output can hold a line shaped like a status line;
        // while reading one, only the block's end is a marker.
        if name.is_some() {
            if line.starts_with("failures:") || line.starts_with("test result:") {
                flush(&mut name, &mut lines, &mut out);
            } else {
                lines.push(line.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("test ")
            && let Some((test, status)) = rest.rsplit_once(" ... ")
        {
            flush(&mut name, &mut lines, &mut out);
            if status.trim() == "FAILED" {
                out.entry(test.trim().to_string()).or_default();
            }
            continue;
        }
        if line.starts_with("failures:") || line.starts_with("test result:") {
            flush(&mut name, &mut lines, &mut out);
            continue;
        }
    }
    flush(&mut name, &mut lines, &mut out);
    out.into_iter().map(|(name, text)| (name, ("wrong".to_string(), text))).collect()
}

/// Formats one item with rustfmt and the project's `rustfmt.toml` (D-158
/// point 5): only the item, never a whole file, since the crate isn't
/// rustfmt-clean. `opening` is a nested item's own container (`impl Store {`, a
/// `trait`, an inline `mod`, an `extern` block): the item is formatted inside it
/// and the caller takes the child out again (D-168). The Rust `submit` task
/// calls this.
pub fn format_item(root: &Path, text: &str, opening: Option<&str>) -> Result<String, Failure> {
    let source = match opening {
        Some(opening) => {
            let opening = if opening.ends_with('\n') { opening.to_string() } else { format!("{opening}\n") };
            format!("{opening}{text}\n}}\n")
        }
        None => text.to_string(),
    };
    let edition = edition(root);
    let config = config_path(root);
    let config_key = config.as_ref().and_then(|path| fs::read(path).ok());
    let key = (source.clone(), edition.clone(), config_key);
    if let Some(formatted) = FORMATTED.lock().unwrap().get(&key) {
        return Ok(formatted.clone());
    }
    let dir = root.join(".solace");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("format-{}.rs", crate::unique_id()));
    fs::write(&file, &source)?;
    let mut argv: Vec<String> = vec!["rustfmt".into(), "--edition".into(), edition];
    if let Some(config) = config {
        argv.push("--config-path".into());
        argv.push(config);
    }
    argv.push(file.to_string_lossy().to_string());
    let output = Command::new(&argv[0]).args(&argv[1..]).current_dir(root).output();
    let formatted = fs::read_to_string(&file).unwrap_or(source);
    fs::remove_file(&file).ok();
    let output = output?;
    if !output.status.success() {
        let message = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        let lines = text.lines().count().max(1);
        let line = rustfmt_line(&message, opening.map_or(0, |opening| opening.lines().count())).unwrap_or(1).min(lines);
        let path = file.to_string_lossy();
        let summary = message
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("error"))
            .or_else(|| {
                message.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with("-->") && !line.contains("format-"))
            })
            .unwrap_or("the text doesn't parse")
            .replace(&*path, "the submitted text");
        return fail(format!("Not saved: E0100 syntax error at line {line}: {summary}"));
    }
    let formatted = format!("{}\n", formatted.trim_end());
    let mut cache = FORMATTED.lock().unwrap();
    if cache.len() >= 1024 {
        cache.clear();
    }
    cache.insert(key, formatted.clone());
    Ok(formatted)
}

/// The package's edition from its Cargo.toml, following
/// `edition.workspace = true` up to the workspace root; rustfmt's default
/// without one.
fn edition(root: &Path) -> String {
    let manifest = |dir: &Path| fs::read_to_string(dir.join("Cargo.toml")).ok().and_then(|text| text.parse::<toml::Value>().ok());
    let own = manifest(root);
    let package = own.as_ref().and_then(|v| v.get("package"));
    if let Some(edition) = package.and_then(|p| p.get("edition")).and_then(|e| e.as_str()) {
        return edition.to_string();
    }
    if package.and_then(|p| p.get("edition")).and_then(|e| e.get("workspace")).and_then(|w| w.as_bool()).unwrap_or(false) {
        for dir in root.ancestors() {
            let found = manifest(dir).and_then(|v| {
                v.get("workspace").and_then(|w| w.get("package")).and_then(|p| p.get("edition")).and_then(|e| e.as_str()).map(String::from)
            });
            if let Some(edition) = found {
                return edition;
            }
        }
    }
    "2021".into()
}

/// The project's rustfmt settings file, under either name rustfmt reads, when
/// it has one: the project's own folder, then each folder above it, so a
/// workspace member picks up the settings at the workspace root (D-158 point 5).
fn config_path(root: &Path) -> Option<String> {
    root.ancestors()
        .flat_map(|dir| ["rustfmt.toml", ".rustfmt.toml"].map(|name| dir.join(name)))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
#[path = "cargo_tests.rs"]
mod tests;

/// The line `rustfmt` names in its stderr, relative to `text`: `offset` is the
/// wrapper's lines before it. The temp file's path stays out of the reply.
fn rustfmt_line(stderr: &str, offset: usize) -> Option<usize> {
    let at = stderr.lines().find_map(|line| line.trim().strip_prefix("--> "))?;
    let (head, _column) = at.rsplit_once(':')?;
    let (_, line) = head.rsplit_once(':')?;
    line.trim().parse::<usize>().ok().map(|line| line.saturating_sub(offset).max(1))
}

static FORMATTED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, String, Option<Vec<u8>>), String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Formats a whole module's text with the project's `rustfmt.toml` and edition
/// (D-180), leaving child modules alone. The text goes through a scratch file,
/// so a failed run changes no module file.
pub fn format_module(root: &Path, source: &str) -> Result<String, Failure> {
    let dir = root.join(".solace");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("format-{}.rs", crate::unique_id()));
    fs::write(&file, source)?;
    let mut argv: Vec<String> = vec!["rustfmt".into(), "--edition".into(), edition(root)];
    if let Some(config) = config_path(root) {
        argv.push("--config-path".into());
        argv.push(config);
    }
    argv.push("--config".into());
    argv.push("skip_children=true,reorder_modules=false".into());
    argv.push(file.to_string_lossy().to_string());
    let output = Command::new(&argv[0]).args(&argv[1..]).current_dir(root).output();
    let formatted = fs::read_to_string(&file).unwrap_or_else(|_| source.to_string());
    fs::remove_file(&file).ok();
    let output = output?;
    if !output.status.success() {
        let message = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        let lines = source.lines().count().max(1);
        let line = rustfmt_line(&message, 0).unwrap_or(1).min(lines);
        let path = file.to_string_lossy();
        let summary = message
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("error"))
            .or_else(|| message.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with("-->")))
            .unwrap_or("the text doesn't parse")
            .replace(&*path, "the module");
        return fail(format!("Not saved: E0100 syntax error at line {line}: {summary}"));
    }
    Ok(formatted)
}

/// A Rust project's `format` needs this on `PATH`; without it the command is
/// refused.
pub const RUSTFMT_MISSING: &str = "Needs the Rust toolchain (`rustfmt`)";

/// Whether `rustfmt` is on `PATH`, looked up on every call like `cargo`.
pub fn rustfmt_installed() -> bool {
    search_path("rustfmt").is_some()
}

/// The project's rustfmt `max_width`, from the same settings file `format_item`
/// and `format_module` pass to rustfmt; rustfmt's default 100 without the setting.
pub(crate) fn max_width(root: &Path) -> usize {
    config_path(root)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| text.parse::<toml::Value>().ok())
        .and_then(|settings| settings.get("max_width").and_then(|width| width.as_integer()))
        .map_or(100, |width| width.max(1) as usize)
}

/// Runs `f` with `PATH` read as `dirs`, so a test can act as a machine with or
/// without a tool.
#[cfg(test)]
pub(crate) fn with_path<T>(dirs: &[PathBuf], f: impl FnOnce() -> T) -> T {
    crate::language::paths::with_path(dirs, f)
}

/// A rendered `-->` or `:::` `path:line:col` as its path and line.
fn source_location(line: &str) -> Option<(&str, usize)> {
    let line = line.trim_start();
    let location = line.strip_prefix("--> ").or_else(|| line.strip_prefix("::: "))?;
    let (path, row, column, _) = parse_location(location)?;
    column?;
    Some((path, row))
}

/// A scratch line as its line in its symbol or in the probe's text; whether
/// it is a probe line, whose wrapper indent the quoted text loses.
fn probe_gutter(line: &str, path: &str, files: &[FileMap]) -> Option<(String, bool)> {
    let trimmed = line.trim_start();
    let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || !trimmed[digits..].starts_with(" |") {
        return None;
    }
    let row = trimmed[..digits].parse().ok()?;
    let (_, Some((name, first))) = locate(files, path, row)? else { return None };
    let indent = &line[..line.len() - trimmed.len()];
    let number = match name {
        crate::PROBE_START_RANGE | crate::PROBE_END_RANGE => String::new(),
        _ => (row + 1 - first).to_string(),
    };
    let rest = &trimmed[digits..];
    let probe = name == crate::PROBE_RANGE;
    let rest = match rest.strip_prefix(" | ") {
        Some(text) if probe => format!(" | {}", without_probe_indent(text).unwrap_or_else(|| text.to_string())),
        _ => rest.to_string(),
    };
    Some((format!("{indent}{number:>width$}{rest}", width = digits), probe))
}

/// A rendered source position as the line shown to the agent.
fn location(files: &[FileMap], path: &str, row: usize) -> Option<String> {
    match locate(files, path, row) {
        Some((_, Some((name, first)))) if name == crate::PROBE_RANGE => Some(format!("  at probe, line {}", row + 1 - first)),
        Some((_, Some((name, _)))) if name == crate::PROBE_START_RANGE => Some("  at probe, start of text".to_string()),
        Some((_, Some((name, _)))) if name == crate::PROBE_END_RANGE => Some("  at probe, end of text".to_string()),
        Some((_, Some((name, first)))) => Some(format!("  at {name}, line {}", row + 1 - first)),
        Some((module, None)) => Some(format!("  at {module} imports")),
        None => None,
    }
}

/// Whether a line range belongs to the generated probe rather than a project symbol.
pub(crate) fn is_probe_range(name: &str) -> bool {
    name == crate::PROBE_RANGE || name == crate::PROBE_START_RANGE || name == crate::PROBE_END_RANGE
}

/// A `path:line[:column]` at the start of `text`, as its path, line,
/// optional column and the byte after the location.
pub(crate) fn parse_location(text: &str) -> Option<(&str, usize, Option<usize>, usize)> {
    for (at, c) in text.char_indices() {
        if c != ':' {
            continue;
        }
        let tail = &text[at + 1..];
        let row_len = tail.bytes().take_while(|b| b.is_ascii_digit()).count();
        if row_len == 0 {
            continue;
        }
        let Ok(row) = tail[..row_len].parse::<usize>() else { continue };
        let mut end = at + 1 + row_len;
        let mut column = None;
        if let Some(after) = text[end..].strip_prefix(':') {
            let col_len = after.bytes().take_while(|b| b.is_ascii_digit()).count();
            if col_len > 0 {
                column = Some(after[..col_len].parse().ok()?);
                end += 1 + col_len;
            }
        }
        return Some((&text[..at], row, column, end));
    }
    None
}

/// A `|` line under a probe's quoted line, without the wrapper's indent.
fn probe_continuation(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let marked = trimmed.strip_prefix("| ")?;
    let text = without_probe_indent(marked)?;
    let indent = &line[..line.len() - trimmed.len()];
    Some(format!("{indent}| {text}"))
}

/// `text` without the eight columns the probe wrapper adds after its leading
/// span markers; rustc may draw them as spaces or as underscores.
fn without_probe_indent(text: &str) -> Option<String> {
    let indent = crate::PROBE_INDENT.len();
    let underscores = "_".repeat(indent);
    let prefix = text.find(|c: char| !matches!(c, '|' | '/' | '\\' | ' ' | '_')).unwrap_or(text.len());
    let at = text[..prefix].find(crate::PROBE_INDENT).or_else(|| text[..prefix].find(&underscores))?;
    Some(format!("{}{}", &text[..at], &text[at + indent..]))
}

/// The crate names a project's manifest lets its code use: its own package and
/// every dependency, dev-dependency and build-dependency, those a target names
/// included. A dash in a dependency's name is an underscore in code (D-210).
pub fn dependency_names(root: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = text.parse().ok()?;
    Some(manifest_dependency_names(&manifest))
}

pub(crate) fn manifest_dependency_names(manifest: &toml::Value) -> Vec<String> {
    let mut names = vec![];
    for table in ["package", "lib"] {
        if let Some(name) = manifest.get(table).and_then(|t| t.get("name")).and_then(toml::Value::as_str) {
            names.push(name.replace('-', "_"));
        }
    }
    for kind in DEPENDENCY_TABLES {
        collect_dependency_names(manifest.get(kind), &mut names);
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for spec in targets.values() {
            for kind in DEPENDENCY_TABLES {
                collect_dependency_names(spec.get(kind), &mut names);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

fn collect_dependency_names(table: Option<&toml::Value>, names: &mut Vec<String>) {
    for name in table.and_then(toml::Value::as_table).into_iter().flatten().map(|(name, _)| name) {
        names.push(name.replace('-', "_"));
    }
}

/// `diagnostics`, with the crates this project's manifest declares.
pub fn diagnostics_for(root: &Path, output: &str, files: &[FileMap]) -> Vec<Diag> {
    match dependency_names(root) {
        Some(deps) => diagnostics_with_dependencies(output, files, &deps),
        None => diagnostics(output, files),
    }
}

/// `diagnostics`, with the crate names the project may use: rustc's
/// "use `cargo add`" hint for another crate becomes the rule that a dependency
/// is a Cargo.toml change the human approves (D-158 point 4, D-210).
pub(crate) fn diagnostics_with_dependencies(output: &str, files: &[FileMap], deps: &[String]) -> Vec<Diag> {
    let mut diags = diagnostics(output, files);
    for diag in &mut diags {
        diag.text = without_cargo_add_hint(&diag.text, deps);
    }
    diags
}

fn without_cargo_add_hint(text: &str, deps: &[String]) -> String {
    text.lines()
        .map(|line| match cargo_add_crate(line) {
            Some((indent, name)) if !deps.iter().any(|dep| dep == &name.replace('-', "_")) => {
                format!("{indent}= help: `{name}` isn't a dependency of this project; add it to `Cargo.toml` first.")
            }
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The leading indent and crate name of rustc's "use `cargo add`" help, if the
/// line is that help.
fn cargo_add_crate(line: &str) -> Option<(&str, &str)> {
    let indent = &line[..line.len() - line.trim_start().len()];
    let help = line.trim_start().strip_prefix("= help:")?;
    let name = help.split_once("use `cargo add ")?.1.split('`').next()?;
    (!name.is_empty()).then_some((indent, name))
}

/// The fixes rustc offers under a message, including notes that name where a
/// private item is defined.
fn suggestions(message: &Value) -> Vec<crate::diagnostics::Suggestion> {
    let mut out = vec![];
    collect_suggestions(message, &mut out);
    collect_span_labels(message, &mut out);
    out
}

fn collect_suggestions(message: &Value, out: &mut Vec<crate::diagnostics::Suggestion>) {
    for child in message["children"].as_array().into_iter().flatten() {
        let spans: Vec<crate::diagnostics::SuggestionSpan> = child["spans"].as_array().into_iter().flatten().map(suggestion_span).collect();
        if !spans.is_empty() {
            out.push(crate::diagnostics::Suggestion { message: child["message"].as_str().unwrap_or("").to_string(), spans });
        }
        collect_suggestions(child, out);
    }
}

fn suggestion_span(span: &Value) -> crate::diagnostics::SuggestionSpan {
    crate::diagnostics::SuggestionSpan {
        file_name: span["file_name"].as_str().unwrap_or("").to_string(),
        line_start: span["line_start"].as_u64().unwrap_or(0) as usize,
        line_end: span["line_end"].as_u64().unwrap_or(0) as usize,
        column_start: span["column_start"].as_u64().unwrap_or(0) as usize,
        column_end: span["column_end"].as_u64().unwrap_or(0) as usize,
        replacement: span["suggested_replacement"].as_str().map(str::to_string),
    }
}

/// A failed test's block with every project source location replaced by its
/// symbol and line, followed by the source line and the comments above it.
pub fn relocate_failure<'a>(text: &str, files: &[FileMap], source: impl Fn(&str) -> Option<&'a str>) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let (mut shown, mut rest, mut quotes) = (String::new(), line, vec![]);
        while let Some(dot) = rest.find(".rs:") {
            let before = &rest[..dot];
            let start = before
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace() || matches!(c, '[' | '(' | '"' | '\''))
                .map_or(0, |(i, c)| i + c.len_utf8());
            let Some((path, row, _column, len)) = parse_location(&rest[start..]) else {
                shown.push_str(&rest[..dot + 4]);
                rest = &rest[dot + 4..];
                continue;
            };
            let end = start + len;
            let Some((_, Some((name, first)))) = locate(files, path, row) else {
                shown.push_str(&rest[..end]);
                rest = &rest[end..];
                continue;
            };
            shown.push_str(&rest[..start]);
            shown.push_str(&format!("{name}, line {}", row + 1 - first));
            rest = &rest[end..];
            if let Some(body) = source(name)
                && let Some(quote) = source_quote(body, row + 1 - first)
            {
                quotes.push(quote);
            }
        }
        shown.push_str(rest);
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&shown);
        for quote in quotes {
            out.push('\n');
            out.push_str(&quote);
        }
    }
    out.trim_end().to_string()
}

/// A source line and the contiguous `//` comments directly above it.
fn source_quote(source: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let at = line.checked_sub(1)?;
    if at >= lines.len() {
        return None;
    }
    let mut first = at;
    while first > 0 && lines[first - 1].trim_start().starts_with("//") {
        first -= 1;
    }
    Some(lines[first..=at].iter().map(|line| line.trim_end()).collect::<Vec<_>>().join("\n"))
}

/// A diagnostic's own labeled spans, such as where a private method is defined.
fn collect_span_labels(message: &Value, out: &mut Vec<crate::diagnostics::Suggestion>) {
    for span in message["spans"].as_array().into_iter().flatten() {
        let Some(label) = span["label"].as_str().filter(|label| !label.is_empty()) else { continue };
        out.push(crate::diagnostics::Suggestion { message: label.to_string(), spans: vec![suggestion_span(span)] });
    }
}
