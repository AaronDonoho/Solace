//! `solace`: the semantic store and agent protocol over Gleam and Rust
//! projects (specs/01-identity.md, 02-projection.md, 03-protocol.md).
//!
//! The store (`.solace/store.db`) is the source of truth. After every change
//! it writes the project's files back out so the language's tools can check,
//! test and build them; agents never touch those files.

// Short-lived tuples inside one function read better spelled out than named.
#![allow(clippy::type_complexity)]

mod board;
mod drafts;
mod extract;
mod language;
mod rust_submit;
mod lockdown;
mod mcp;
mod migrate;
mod orientation;
mod outside;
mod reach;
mod reads;
mod recipe;
mod scope;
mod skills;
mod store;
mod tasks;
mod view;
mod workspace;

use language::{Import, Item, Kind, Language, Ref, cargo, gleam};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};
use std::fs;
use store::store_rust_file;

mod reference;
pub(crate) use reference::*;
mod index;
mod rust_probe;
mod fsx;
mod jsonx;
mod project_paths;
mod rust_move;
mod rust_check;
mod cli;
mod diagnostics;
mod failure;
mod command_tx;
mod store_core;
mod store_read;
mod store_write;
use crate::cli::{next_step, operands, path_dependencies};
pub(crate) use failure::{Failure, Out, fail, usage};
use crate::store_core::SCHEMA;
use crate::store_core::LATER_TABLES;
use crate::store_write::DependencyModule;
use crate::store_write::DependencyKey;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, code) = cli::main(args);
    print!("{out}");
    std::process::exit(code);
}

thread_local! {
    /// The `reason` argument of the current tool call, for a settings write's question.
    static REASON: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// D-152: the task's changes the last passing `test` found no test reaching,
    /// named for the reply, so the call's `task done` waits.
    static UNTESTED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub(crate) type Sym = index::Sym;

pub(crate) type ModuleRow = index::ModuleRow;

pub(crate) struct Store {
    /// Shared with the command's other stores of the same project, so they
    /// run inside its outer transaction (D-77 point 7).
    db: Rc<Connection>,
    root: PathBuf,
    /// The project's language, chosen once from its files; `None` for a
    /// project of plain files (D-60).
    lang: Option<&'static dyn Language>,
    /// Whether the store has the nested Rust item columns (`symbols.parent`, `modules.file`);
    /// `false` only for an older store opened by `peek` (D-158).
    nested: bool,
    /// Whether the store has `modules.owns_dir`; `false` only for an older store
    /// opened by `peek` (D-168).
    owns_dir: bool,
    /// Direct dependency modules, scanned again only when the manifest or the
    /// resolved package list changes.
    deps: std::cell::RefCell<Option<(DependencyKey, Rc<Vec<DependencyModule>>)>>,
}

thread_local! {
    /// The connections a command's outer transactions hold, by project root:
    /// the project's, and an extract's destination (D-103).
    static COMMAND_DB: RefCell<Vec<(PathBuf, Rc<Connection>)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) type Index = index::Index;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of `bytes`, in hex.
fn sha256(bytes: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(bytes))
}

/// A fresh id: 16 hex digits, unique across processes and calls.
fn unique_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    sha256(format!("{nanos}/{n}/{}", std::process::id()).as_bytes())[..16].to_string()
}

/// A new symbol identity: never derived from content, never reused (Spec 1 §2).
fn new_id(kind: Kind) -> String {
    let prefix = match kind {
        Kind::Fn => "fn",
        Kind::Type => "ty",
        Kind::Const => "cn",
    };
    format!("{prefix}:{}", unique_id())
}

/// `text` without its comment and doc lines.
fn without_comment_lines(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// The name in a `// ── <name> (<kind>) ──` header (D-168): up to the closing
/// dashes, without the kind. A name may hold spaces (`<T as Tr>::f`).
fn block_name(header: &str) -> String {
    let name = header.split(" ──").next().unwrap_or("").trim();
    name.strip_suffix(')').and_then(|n| n.rsplit_once(" (")).map_or(name, |(name, _)| name).to_string()
}

/// Phase 1a revision: a hash of the code without its comments or doc text.
fn revision(text: &str) -> String {
    format!("sha256:{}", &sha256(without_comment_lines(text).as_bytes())[..16])
}

const STARTER_SETTINGS: &str = "# Solace project settings. Protected: agents may read this file.
# Text files up to this many bytes are watched; file writes are limited to it.
watch_limit = 262144
# Files Solace doesn't watch or search (agents can still read and write them).
unwatched = []

[validators]
# Optional checks, for example:
# \"docs/**/*.json\" = [\"python3\", \"-m\", \"json.tool\", \"{file}\"]

[commands]
# Optional commands agents may run, for example:
# app = { run = [\"gleam\", \"run\", \"--\"], args = true }
";

/// Sets up a project: `solace.toml` if missing, and a store holding the Gleam
/// code under `src/` and `test/`. Without `gleam.toml` the store stays empty:
/// a project of plain files (D-60).
fn import(root: &Path) -> Out {
    import_with(root, cargo::installed())
}

/// Imports `root`, fetching a Rust project's dependencies with `cargo` when
/// `has_cargo` (D-158 point 4); without it, the import goes ahead and says so.
fn import_with(root: &Path, has_cargo: bool) -> Out {
    let mut out = String::new();
    let dir = root.join(".solace");
    let stored = dir.join("store.db").exists();
    // A new Cargo package is read whole first, so a refused one leaves no settings or store behind.
    let rust = if stored {
        None
    } else {
        match language::detect(root) {
            None if root.join("Cargo.toml").exists() => Some(language::rust::package(root)?),
            _ => None,
        }
    };
    // Fetch before any write, so a failed fetch leaves the project as it was.
    let dependencies = dependencies(root, has_cargo)?;
    match fs::OpenOptions::new().write(true).create_new(true).open(root.join("solace.toml")) {
        Ok(mut file) => {
            file.write_all(STARTER_SETTINGS.as_bytes())?;
            out += "Created solace.toml with default settings.\n";
        }
        // A root's list of projects alone gains the settings its own project needs (D-60).
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let text = fs::read_to_string(root.join("solace.toml"))?;
            if text.parse::<toml::Value>().is_ok_and(|v| v.as_table().is_some_and(|t| t.keys().all(|k| k == "projects"))) {
                fs::write(root.join("solace.toml"), format!("{}\n{STARTER_SETTINGS}", text.trim_end()))?;
                out += "Added default settings to solace.toml.\n";
            }
        }
        Err(e) => return Err(e.into()),
    }
    if stored {
        out += "This project already has a store (.solace/store.db); existing symbols were preserved.\n";
        out += &dependencies;
        return Ok(out);
    }
    fs::create_dir_all(&dir)?;
    let db = Connection::open(dir.join("store.db"))?;
    db.busy_timeout(Duration::from_secs(10))?;
    db.execute_batch(SCHEMA)?;
    db.execute_batch(LATER_TABLES)?;
    let (mut modules, mut symbols) = (0, 0);
    let mut written = outside::Written::load(root);
    written.known = true;
    if let Some(lang) = language::detect(root) {
        for rel in lang.source_files(root)? {
            let (top, file) = rel.split_once('/').unwrap();
            let path = Path::new(file).with_extension("").to_string_lossy().replace('\\', "/");
            let file = root.join(&rel);
            let text = fs::read_to_string(&file).map_err(|e| Failure(format!("{}: {e}", file.display()), 1))?;
            written.record(&outside::file_of(top, &path), text.as_bytes());
            let parsed = lang.split(&text);
            db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, top, parsed.header])?;
            for (ord, item) in parsed.items.iter().enumerate() {
                insert_symbol(&db, &path, item, Some(ord as i64))?;
                symbols += 1;
            }
            modules += 1;
        }
    }
    let rust_imported = rust.is_some();
    for (rel, module, file) in rust.into_iter().flatten() {
        written.record(&rel, file.join().as_bytes());
        store_rust_file(&db, &module, rel.split_once('/').unwrap().0, &rel, &file)?;
        modules += 1;
        symbols = db.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    }
    written.save()?;
    // The first check is warm, without making the import wait (D-158 point 3).
    if rust_imported && has_cargo {
        cargo::warm(root);
    }
    let store = Store::new(Rc::new(db), root, true, true);
    outside::approve_all(&store)?;
    out += &format!("Imported {modules} modules, {symbols} symbols into .solace/store.db\n");
    out += &dependencies;
    Ok(out)
}

/// Fetches a Rust project's dependencies, or says why it couldn't (D-158 point 4).
fn dependencies(root: &Path, has_cargo: bool) -> Out {
    if !language::is_rust(root) {
        return Ok(String::new());
    }
    if let Some(why) = language::reason(language::RUST)? {
        return Ok(format!("Dependencies not fetched: {why}\n"));
    }
    if has_cargo {
        cargo::fetch(root)?;
        Ok(String::new())
    } else {
        Ok(format!("Dependencies not fetched: {}\n", language::missing(language::RUST)))
    }
}

/// Stores a new symbol at `ord`, or after its module's last; returns its id.
fn insert_symbol(db: &Connection, module: &str, item: &Item, ord: Option<i64>) -> Result<String, Failure> {
    let ord = match ord {
        Some(o) => o,
        None => db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [module], |r| r.get(0))?,
    };
    let id = new_id(item.kind);
    db.execute(
        "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, module, item.name, item.kind.as_str(), item.text, ord, revision(&item.text)],
    )?;
    Ok(id)
}

/// Search terms: identifiers split at `_` and case changes, lowercased, a
/// plural `s` dropped; short words, numbers and common English words skipped.
fn words(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "when", "should", "must", "into", "are", "was", "were", "been", "has", "have",
        "not", "but", "any", "all", "its", "also", "each", "only", "than", "then", "there", "their", "they", "which", "will", "would",
        "can", "could", "does", "did", "you", "your", "new", "add", "make", "use", "using", "test", "tests", "existing", "keep", "one",
        "other", "what", "how", "may", "per", "such",
    ];
    let mut out = vec![];
    for tok in text.split(|c: char| !c.is_alphanumeric()) {
        let mut cur = String::new();
        let mut prev_lower = false;
        for c in tok.chars() {
            if c.is_uppercase() && prev_lower {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.extend(c.to_lowercase());
        }
        out.push(cur);
    }
    out.into_iter()
        .map(|w| if w.len() > 4 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_string() } else { w })
        .filter(|w| w.len() >= 3 && !w.chars().all(|c| c.is_ascii_digit()) && !STOP.contains(&w.as_str()))
        .collect()
}

fn is_test(ix: &Index, i: usize) -> bool {
    ix.test[i]
}

/// The lines where symbol `j` refers to `i`: (line from 1, that source line).
fn ref_lines(ix: &Index, j: usize, i: usize) -> Vec<(usize, String)> {
    let lines: Vec<&str> = ix.syms[j].text.lines().collect();
    let mut at: Vec<usize> = ix.at[j].iter().filter(|r| r.0 == i).map(|r| r.1).collect();
    at.dedup();
    at.into_iter().filter_map(|n| Some((n + 1, lines.get(n)?.trim().to_string()))).collect()
}

/// Where `j` constructs or matches type `ty` listing every field (no `..`):
/// the places that break when a field is added (D-46). With `updates`, the
/// record updates and patterns with `..` instead, which don't.
// a generic type's annotation such as `Box(Int)` also matches; check the position if that misleads.
fn sites(ix: &Index, j: usize, ty: usize, updates: bool) -> Vec<(usize, String)> {
    let ctors = ix.lang.constructors(&ix.syms[ty].text);
    let text = &ix.syms[j].text;
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(k, _)| k + 1)).collect();
    ref_lines(ix, j, ty)
        .into_iter()
        .filter(|(n, line)| {
            let at = starts[n - 1] + text[starts[n - 1]..].len() - text[starts[n - 1]..].trim_start().len();
            ctors.iter().any(|c| {
                line.match_indices(&format!("{c}(")).any(|(k, _)| {
                    let whole = !line[..k].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
                    whole && updates == language::split_args(&text[at + k + c.len() + 1..]).iter().any(|(_, a)| a.trim().starts_with(".."))
                })
            })
        })
        .collect()
}

thread_local! {
    /// The symbols whose signature or text an earlier command in this MCP call
    /// printed, so `inspect` and `refs` can say `(above)` (D-46); `None`
    /// outside a call.
    static SHOWN: std::cell::RefCell<Option<HashSet<String>>> = const { std::cell::RefCell::new(None) };
}

/// Where a function's body opens: the first `{` outside parentheses after
/// its `fn name(`.
fn fn_body_start(s: &Sym) -> Option<usize> {
    let at = s.text.find(&format!("fn {}(", s.name))?;
    Some(at + language::body_start(&s.text[at..])?)
}

/// A function's parameters from its signature: (label if any, name).
fn params(signature: &str, name: &str) -> Vec<(Option<String>, String)> {
    let Some((_, rest)) = fn_header(signature, name) else { return vec![] };
    let Some(args) = fn_args(rest) else { return vec![] };
    language::split_args(args)
        .iter()
        .map(|(_, p)| p.split(':').next().unwrap_or("").split_whitespace().collect::<Vec<_>>())
        .filter(|words| !words.is_empty())
        .map(|words| (if words.len() > 1 { Some(words[0].to_string()) } else { None }, words[words.len() - 1].to_string()))
        .collect()
}

/// Writes the serving port into a lock this server holds (D-80). A reader
/// trusts it only while the lock is held.
fn record_port(f: &fs::File, port: u16) {
    let _ = f.set_len(0);
    let _ = std::io::Write::write_all(&mut &*f, format!("{{\"port\":{port},\"pid\":{}}}", std::process::id()).as_bytes());
}

/// `n` and the noun, plural unless `n` is 1: "1 file", "2 files".
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// A project file's name as Solace lists it: `/`-separated, with `.` removed
/// and parent parts resolved for relative paths. A rooted path stays as
/// written, so an error echoes what the agent sent.
fn file_name(path: &Path) -> String {
    if path.components().any(|part| matches!(part, std::path::Component::RootDir | std::path::Component::Prefix(_))) {
        return path.to_string_lossy().into_owned();
    }
    let mut parts: Vec<String> = vec![];
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else {
                    parts.push("..".to_string());
                }
            }
            part => parts.push(part.as_os_str().to_string_lossy().into_owned()),
        }
    }
    parts.join("/")
}

/// Symbols as they were before a change: (id, text then; `None` if new).
type Before = Vec<(String, Option<String>)>;

/// Notes for a reply, a line each.
fn note_lines(notes: &[String]) -> String {
    notes.iter().map(|n| format!("Note: {n}\n")).collect()
}

/// Drops repeats anywhere in `v`, keeping the first of each, in order.
fn unique<T: PartialEq>(v: &mut Vec<T>) {
    let mut out = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    *v = out;
}

/// Fixtures for every module's tests.
#[cfg(test)]
#[path = "testing.rs"]
mod testing;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[cfg(test)]
thread_local! {
    /// A test's project root, in place of the working folder's.
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The name of the probe text's line range in its generated module.
pub(crate) const PROBE_RANGE: &str = "probe";

/// The name of the generated module's lines before the probe text.
pub(crate) const PROBE_START_RANGE: &str = "probe start";

/// The name of the generated module's lines after the probe text.
pub(crate) const PROBE_END_RANGE: &str = "probe end";

/// The indentation the probe's wrapper adds to each of its lines.
pub(crate) const PROBE_INDENT: &str = "        ";

/// Where a function's `fn` is in `signature`: `name`'s, and the text after
/// its name, starting at its generic parameters or its arguments.
pub(crate) fn fn_header<'a>(signature: &'a str, name: &str) -> Option<(usize, &'a str)> {
    let prefix = format!("fn {name}");
    signature.match_indices(&prefix).find_map(|(at, _)| {
        let rest = &signature[at + prefix.len()..];
        matches!(rest.chars().next(), Some('(' | '<')).then_some((at, rest))
    })
}

/// A function's arguments, from just after its `(`, past its generic parameters.
fn fn_args(rest: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if !rest[..i].ends_with('-') => depth = depth.saturating_sub(1),
            '(' if depth == 0 => return Some(&rest[i + 1..]),
            _ => {}
        }
    }
    None
}
