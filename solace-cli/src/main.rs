//! `solace`: the Phase 1a semantic store and agent protocol over Gleam
//! (specs/01-identity.md, 02-projection.md, 03-protocol.md).
//!
//! The store (`.solace/store.db`) is the source of truth. After every change
//! it writes the Gleam files back out so `gleam` can check, test and build
//! them; agents never touch those files.

mod board;
mod gleam;
mod lockdown;

use gleam::{Item, Kind, Ref};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

const REFERENCE: &str = include_str!("reference.txt");
/// The largest reply, in bytes (about 8k tokens). Agents cut longer tool
/// output (Codex at about 10k tokens), so `inspect` and `view` trim by
/// priority to fit rather than being cut mid-document (D-14). `solace mcp
/// --budget <bytes>` sets it per agent.
const BUDGET: usize = 32_000;
const STDLIB: &[&str] = &[
    "bool",
    "bit_array",
    "bytes_tree",
    "dict",
    "dynamic",
    "float",
    "function",
    "int",
    "io",
    "list",
    "option",
    "order",
    "pair",
    "result",
    "set",
    "string",
    "string_tree",
    "uri",
];

/// A command's failure: its message and exit code (1 diagnostics, 2 usage).
#[derive(Debug)]
struct Failure(String, i32);
type Out = Result<String, Failure>;

fn fail<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(msg.into(), 1))
}
fn usage<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(format!("{}\n\nRun `solace help` for usage.", msg.into()), 2))
}
impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Self {
        Failure(format!("store error: {e}"), 1)
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure(format!("io error: {e}"), 1)
    }
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut options = Vec::new();
    while let Some(i) = args.iter().position(|a| a == "--option") {
        if i + 1 >= args.len() {
            break;
        }
        args.remove(i);
        options.push(args.remove(i));
    }
    let old = args.iter().position(|a| a == "--old").and_then(|i| {
        if i + 1 >= args.len() {
            return None;
        }
        args.remove(i);
        Some(args.remove(i))
    });
    if args.first().map(String::as_str) == Some("mcp") {
        let budget = match args.get(1..) {
            Some([flag, n]) if flag == "--budget" => n.parse().unwrap_or_else(|_| panic!("--budget takes a number of bytes, got {n}")),
            _ => BUDGET,
        };
        return mcp(budget);
    }
    let code = match run_options(&args, &mut stdin, old.as_deref(), BUDGET, &options) {
        Ok(out) => {
            print!("{out}");
            0
        }
        Err(Failure(msg, code)) => {
            println!("{msg}");
            code
        }
    };
    std::process::exit(code);
}

/// `input` supplies the new code for submit commands; `old`, the text that
/// `submit <name> --replace` replaces (MCP only); `budget`, the bytes left
/// for this reply.
fn run_options(args: &[String], input: &mut dyn FnMut() -> Out, old: Option<&str>, budget: usize, options: &[String]) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.first() == Some(&"serve") {
        if board::context().is_some() {
            return lockdown::error("E7010", "serve", "host command");
        }
        let s = Store::open()?;
        let port = match a.as_slice() {
            ["serve"] => 3030,
            ["serve", "--port", p] => p.parse().map_err(|_| Failure("invalid port".into(), 2))?,
            _ => return usage("serve [--port N]"),
        };
        return board::serve(&s.root, port);
    }
    if a.first() == Some(&"task") && board::context().is_none() {
        return lockdown::error("E7010", "task", "task commands work only in a run started from the task board; this session has no task");
    }
    let Ok(store) = Store::open() else {
        board::workflow(&lockdown::settings(&env::current_dir()?)?)?;
        return run_inner(args, input, old, budget);
    };
    let context = board::context();
    let guard = if let Some((tid, rid)) = &context {
        board::agent_guard(&store.root, tid, rid, if a.is_empty() { &["help"] } else { &a })?
    } else {
        board::lock(&store.root)?
    };
    let store = board::open(&store.root)?;
    lockdown::recover(&store.root)?;
    board::workflow(&lockdown::settings(&store.root)?)?;
    if let Some((tid, rid)) = &context {
        if a.first() == Some(&"task") {
            return board::task_command(&store, tid, rid, &a, input, options, guard);
        }
    }
    let changing = a.first().is_some_and(|a| matches!(*a, "submit" | "delete" | "write" | "remove" | "run"));
    let before = if changing && context.is_some() {
        let mut v = board::snapshot(&store)?;
        v.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        Some(v)
    } else {
        None
    };
    let result = run_inner(args, input, old, budget);
    if let (Some(before), Some((tid, rid))) = (before, context) {
        let mut after = board::snapshot(&store)?;
        after.as_object_mut().unwrap().extend(board::file_snapshot(&store.root, &a)?.as_object().unwrap().clone());
        board::record(&store, &tid, &rid, &before, &after)?;
    }
    result
}

fn run_inner(args: &[String], input: &mut dyn FnMut() -> Result<String, Failure>, old: Option<&str>, budget: usize) -> Out {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(a.as_slice(), [] | ["help"] | ["--help"]) {
        return Ok(REFERENCE.to_string());
    }
    if let ["agent-config", agent, dir] = a.as_slice() {
        return lockdown::agent_config(
            agent,
            Path::new(dir),
            &env::current_exe()?,
            board::system()?["task_timeout"].as_u64().unwrap_or(3600),
        );
    }
    if a[0] == "import" {
        lockdown::settings(&env::current_dir()?)?;
        return import(&env::current_dir()?);
    }
    let store = Store::open()?;
    let settings = lockdown::settings(&store.root)?;
    lockdown::recover(&store.root)?;
    if let Some(out) = lockdown::dispatch(&store, &settings, &a, input, old, budget) {
        return out;
    }
    match a.as_slice() {
        ["session", "start", "--task"] => store.session_task(&input()?),
        ["session", "start", "--task", task @ ..] => store.session_task(&task.join(" ")),
        ["submit", name, "--replace"] => match old {
            // an empty replacement deletes the old text
            Some(old) => store.submit_replace(name, old, &input().unwrap_or_default()),
            None => usage("`submit <name> --replace` needs the exact text to replace in `old` and its replacement in `text`."),
        },
        ["session", rest @ ..] => store.session(&operands(rest.strip_prefix(&["start"]).unwrap_or(rest))),
        ["view", rest @ ..] if !operands(rest).is_empty() => store.view(&operands(rest), rest.contains(&"--bodies"), budget),
        ["inspect", rest @ ..] if !operands(rest).is_empty() => store.inspect(&operands(rest), rest.contains(&"--context"), budget),
        ["refs", name] => store.refs(name),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        ["submit"] => store.submit_symbols(None, None, &input()?),
        ["submit", "--new", module] => store.submit_symbols(None, Some(module), &input()?),
        ["submit", "--imports", module] => store.submit(Target::Imports(module), &input()?),
        ["submit", name] if !name.contains('.') && store.is_module(name)? => store.submit(Target::Module(name), &input()?),
        ["submit", name] => store.submit_symbols(Some(name), None, &input()?),
        ["delete", name] => store.delete(name),
        ["check"] => command_report(store.check(), budget),
        ["test"] => command_report(store.test(), budget),
        ["work", "add"] => store.work_add(&input()?),
        ["work", "add", text @ ..] => store.work_add(&text.join(" ")),
        ["work", "list"] | ["work"] => store.work_list(),
        ["work", "done", id] => store.work_done(id),
        _ => usage(format!(
            "Unknown command: {}\nCommands: session, inspect, view, refs, find, submit, delete, check, test, work, files, read, write, remove, build, format, run, task.",
            a.join(" ")
        )),
    }
}

fn command_report(result: Out, budget: usize) -> Out {
    match result {
        Ok(text) => Ok(lockdown::trim_output(&("exit code: 0\n".to_string() + &text), budget)),
        Err(Failure(text, code)) => Err(Failure(lockdown::trim_output(&format!("exit code: {code}\n{text}"), budget), code)),
    }
}

/// Arguments that aren't flags (`--scope` just introduces them).
fn operands<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| !a.starts_with("--")).collect()
}

fn stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

/// `solace mcp`: the same commands as one MCP tool over stdio, so an agent
/// can pass code without shell quoting and needs no shell at all.
fn mcp(budget: usize) {
    use serde_json::{Value, json};
    let description = "Run a Solace command against this project's semantic store. `command` is what \
        follows `solace` on a command line, e.g. `session start`, `inspect checkout.pay tax.quote --context`, \
        `submit checkout.pay`. For submit commands, put the new code in `text`. Start with \
        `session start --task` with the task as `text`: it lists the symbols most likely relevant, \
        open work, and every command.";
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no reply
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "solace", "version": env!("CARGO_PKG_VERSION")}
            }),
            "tools/list" => json!({"tools": [{
                "name": "solace",
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "e.g. `view checkout` or `submit --new pilot/checkout`"},
                        "text": {"type": "string", "description": "New code, for submit commands (or a work record, for `work add`; the task, for `session start --task`; the note, summary or question, for `task note|done|ask`)"},
                        "options": {"type": "array", "items": {"type": "string"}, "description": "2–4 options for task ask"},
                        "old": {"type": "string", "description": "For `submit <name> --replace`: the exact text in the symbol that `text` replaces"}
                    },
                    "required": ["command"]
                }
            }]}),
            "tools/call" => {
                let args = &msg["params"]["arguments"];
                let text = args["text"].as_str().map(String::from);
                let mut input = || match &text {
                    Some(t) => Ok(t.clone()),
                    _ => usage("This command needs the new code in the `text` argument."),
                };
                // Several commands, one per line, run in order in one call.
                let commands: Vec<&str> =
                    args["command"].as_str().unwrap_or("").lines().map(|c| c.trim()).filter(|c| !c.is_empty()).collect();
                let (mut out, mut is_error) = (String::new(), false);
                for command in &commands {
                    let command = command.strip_prefix("solace").map_or(*command, |c| c.trim_start());
                    let words: Vec<String> = command.split_whitespace().map(String::from).collect();
                    if commands.len() > 1 {
                        out += &format!("==== solace {command} ====\n");
                    }
                    // commands in one call share the budget, since the agent cuts the reply as a whole
                    match run_options(
                        &words,
                        &mut input,
                        args["old"].as_str(),
                        budget.saturating_sub(out.len()),
                        &args["options"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>())
                            .unwrap_or_default(),
                    ) {
                        Ok(o) => out += &o,
                        Err(Failure(msg, _)) => {
                            out += &msg;
                            is_error = true;
                        }
                    }
                    if commands.len() > 1 && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                if commands.is_empty() {
                    (out, is_error) = (format!("No command given.\n\n{REFERENCE}"), true);
                }
                json!({"content": [{"type": "text", "text": out}], "isError": is_error})
            }
            "ping" => json!({}),
            _ => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}});
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = stdout.flush();
    }
}

// --- the store --------------------------------------------------------------

#[derive(Clone)]
struct Sym {
    id: String,
    module: String,
    name: String,
    kind: Kind,
    text: String,
}

impl Sym {
    fn qname(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
    fn item(&self) -> Item {
        Item { kind: self.kind, name: self.name.clone(), text: self.text.clone() }
    }
}

struct ModuleRow {
    path: String,
    dir: String,
    header: String,
}

struct Store {
    db: Connection,
    root: PathBuf,
}

/// All live symbols and modules, with the reference graph resolved.
struct Index {
    syms: Vec<Sym>,
    modules: Vec<ModuleRow>,
    uses: Vec<Vec<usize>>,
}

enum Target<'a> {
    Imports(&'a str),
    /// A whole module's text, as `view <module>` shows it.
    Module(&'a str),
}

const SCHEMA: &str = "
CREATE TABLE modules(path TEXT PRIMARY KEY, dir TEXT NOT NULL, header TEXT NOT NULL);
CREATE TABLE symbols(id TEXT PRIMARY KEY, module TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  text TEXT NOT NULL, ord INTEGER NOT NULL, revision TEXT NOT NULL, live INTEGER NOT NULL DEFAULT 1);
CREATE TABLE history(seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, symbol TEXT NOT NULL, action TEXT NOT NULL, detail TEXT NOT NULL);
CREATE TABLE broken(symbol TEXT PRIMARY KEY, diagnostic TEXT NOT NULL);
CREATE TABLE work(id INTEGER PRIMARY KEY, text TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1);
";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A new identity: never derived from content, never reused (Spec 1 §2).
fn new_id(kind: Kind, module: &str, name: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let seed = format!("{module}.{name}/{nanos}/{n}/{}", std::process::id());
    let prefix = match kind {
        Kind::Fn => "fn",
        Kind::Type => "ty",
        Kind::Const => "cn",
    };
    format!("{prefix}:{}", &hex(&Sha256::digest(seed.as_bytes()))[..16])
}

/// Phase 1a revision: a hash of the code without its comments or doc text.
fn revision(text: &str) -> String {
    let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    format!("sha256:{}", &hex(&Sha256::digest(code.as_bytes()))[..16])
}

fn import(root: &Path) -> Out {
    if !root.join("gleam.toml").exists() {
        return usage("Run `solace import` in a Gleam project (no gleam.toml here).");
    }
    let mut out = String::new();
    match fs::OpenOptions::new().write(true).create_new(true).open(root.join("solace.toml")) {
        Ok(mut file) => {
            file.write_all(b"# Solace project settings. Human-owned; agents may read this file.\n\
# File writes are limited to this many bytes.\n\
resource_limit = 262144\n\
# Classification only: unlisted files are also readable and writable.\n\
resources = []\n\
ignore = []\n\n\
[validators]\n\
# Optional checks, for example:\n\
# \"docs/**/*.json\" = [\"python3\", \"-m\", \"json.tool\", \"{file}\"]\n\n\
[commands]\n\
# Optional commands agents may run, for example:\n\
# app = { run = [\"gleam\", \"run\", \"--\"], args = true }\n")?;
            out += "Created solace.toml with default settings.\n";
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let dir = root.join(".solace");
    if dir.join("store.db").exists() {
        out += "This project already has a store (.solace/store.db); existing symbols were preserved.\n";
        return Ok(out);
    }
    fs::create_dir_all(&dir)?;
    let db = Connection::open(dir.join("store.db"))?;
    db.execute_batch(SCHEMA)?;
    let (mut modules, mut symbols) = (0, 0);
    for top in ["src", "test"] {
        let mut files = vec![];
        collect_gleam_files(&root.join(top), &mut files)?;
        files.sort();
        for file in files {
            let path = file.strip_prefix(root.join(top)).unwrap().with_extension("");
            let path = path.to_string_lossy().replace('\\', "/");
            let parsed = gleam::split(&fs::read_to_string(&file)?);
            db.execute("INSERT INTO modules VALUES (?1, ?2, ?3)", params![path, top, parsed.header])?;
            for (ord, item) in parsed.items.iter().enumerate() {
                let id = new_id(item.kind, &path, &item.name);
                db.execute(
                    "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, path, item.name, item.kind.as_str(), item.text, ord as i64, revision(&item.text)],
                )?;
                symbols += 1;
            }
            modules += 1;
        }
    }
    out += &format!("Imported {modules} modules, {symbols} symbols into .solace/store.db\n");
    Ok(out)
}

/// A Gleam module path: lowercase names of letters, digits and `_`, joined by `/`.
fn valid_module_path(m: &str) -> bool {
    m.split('/').all(|p| p.starts_with(|c: char| c.is_ascii_lowercase()) && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
}

fn collect_gleam_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_gleam_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "gleam") {
            out.push(path);
        }
    }
    Ok(())
}

impl Store {
    fn open() -> Result<Store, Failure> {
        let mut dir = env::current_dir()?;
        loop {
            let db = dir.join(".solace/store.db");
            if db.exists() {
                return Ok(Store { db: Connection::open(db)?, root: dir });
            }
            if !dir.pop() {
                return fail("No Solace store found (.solace/store.db). Run `solace import` in the project first.");
            }
        }
    }

    fn index(&self) -> Result<Index, Failure> {
        let mut stmt = self.db.prepare("SELECT id, module, name, kind, text FROM symbols WHERE live = 1 ORDER BY module, ord")?;
        let syms: Vec<Sym> = stmt
            .query_map([], |r| {
                Ok(Sym { id: r.get(0)?, module: r.get(1)?, name: r.get(2)?, kind: Kind::parse(&r.get::<_, String>(3)?), text: r.get(4)? })
            })?
            .collect::<Result<_, _>>()?;
        let mut stmt = self.db.prepare("SELECT path, dir, header FROM modules ORDER BY dir, path")?;
        let modules: Vec<ModuleRow> =
            stmt.query_map([], |r| Ok(ModuleRow { path: r.get(0)?, dir: r.get(1)?, header: r.get(2)? }))?.collect::<Result<_, _>>()?;

        // (module, name) -> symbol, for names and for type constructors
        let mut names: HashMap<(&str, &str), usize> = HashMap::new();
        for (i, s) in syms.iter().enumerate() {
            names.insert((&s.module, &s.name), i);
        }
        let mut ctors: Vec<(String, String, usize)> = vec![];
        for (i, s) in syms.iter().enumerate() {
            if s.kind == Kind::Type {
                for c in gleam::constructors(&s.text) {
                    ctors.push((s.module.clone(), c, i));
                }
            }
        }
        for (m, c, i) in &ctors {
            names.entry((m, c)).or_insert(*i);
        }
        let imports: HashMap<&str, Vec<gleam::Import>> = modules.iter().map(|m| (m.path.as_str(), gleam::imports(&m.header))).collect();
        let uses = syms
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let imps = &imports[s.module.as_str()];
                let mut found = vec![];
                for r in gleam::references(&s.text) {
                    let target = match &r {
                        Ref::Qualified(alias, member) => {
                            imps.iter().find(|imp| &imp.alias == alias).and_then(|imp| names.get(&(imp.module.as_str(), member.as_str())))
                        }
                        Ref::Bare(name) => names.get(&(s.module.as_str(), name.as_str())).or_else(|| {
                            imps.iter().find_map(|imp| {
                                imp.unqualified
                                    .iter()
                                    .find(|(_, local)| local == name)
                                    .and_then(|(orig, _)| names.get(&(imp.module.as_str(), orig.as_str())))
                            })
                        }),
                    };
                    if let Some(&t) = target
                        && t != i
                        && !found.contains(&t)
                    {
                        found.push(t);
                    }
                }
                found
            })
            .collect();
        Ok(Index { syms, modules, uses })
    }

    // --- writing files --------------------------------------------------------

    /// Writes every module's file from the store. Returns, per file, the line
    /// range of each symbol, for mapping compiler output back to symbols.
    fn materialize(&self, ix: &Index) -> Result<Vec<FileMap>, Failure> {
        let mut maps = vec![];
        for m in &ix.modules {
            let mut content = String::new();
            let mut line = 1;
            let mut ranges = vec![];
            if !m.header.is_empty() {
                content.push_str(&m.header);
                content.push_str("\n\n");
                line += m.header.lines().count() + 1;
            }
            for s in ix.syms.iter().filter(|s| s.module == m.path) {
                let n = s.text.lines().count();
                ranges.push((line, line + n - 1, s.qname()));
                content.push_str(&s.text);
                content.push_str("\n\n");
                line += n + 1;
            }
            let content = content.trim_end().to_string() + "\n";
            let path = self.root.join(&m.dir).join(format!("{}.gleam", m.path));
            if fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                fs::create_dir_all(path.parent().unwrap())?;
                fs::write(&path, content)?;
            }
            maps.push(FileMap { module: m.path.clone(), file: path, ranges });
        }
        Ok(maps)
    }

    fn gleam(&self, args: &[&str]) -> Result<(bool, String), Failure> {
        let mut argv = vec!["gleam".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let text = lockdown::command(&self.root, &argv, 120, BUDGET)?;
        Ok((text.starts_with("exit code: 0\n"), strip_ansi(text.split_once('\n').map_or("", |(_, s)| s))))
    }

    /// Type-checks the project and records which symbols are broken.
    fn check_all(&self, ix: &Index) -> Result<Vec<Diag>, Failure> {
        let files = self.materialize(ix)?;
        let (_, output) = self.gleam(&["check"])?;
        let diags = diagnostics(&output, &files);
        self.db.execute("DELETE FROM broken", [])?;
        for d in diags.iter().filter(|d| d.error) {
            if let Some(sym) = &d.symbol {
                self.db.execute("INSERT OR IGNORE INTO broken VALUES (?1, ?2)", params![sym, d.text])?;
            }
        }
        Ok(diags)
    }

    // --- reading ---------------------------------------------------------------

    fn resolve(&self, ix: &Index, input: &str) -> Result<usize, Failure> {
        let (module, name) = match input.rsplit_once('.') {
            Some((m, n)) => (Some(m.replace('.', "/")), n),
            None => (None, input),
        };
        let found: Vec<usize> = (0..ix.syms.len())
            .filter(|&i| {
                let s = &ix.syms[i];
                s.name == name && module.as_ref().is_none_or(|m| s.module == *m || s.module.ends_with(&format!("/{m}")))
            })
            .collect();
        match found.as_slice() {
            [i] => Ok(*i),
            [] => {
                let lower = name.to_lowercase();
                let similar: Vec<String> =
                    ix.syms.iter().filter(|s| s.name.to_lowercase().contains(&lower)).take(10).map(|s| s.qname()).collect();
                let hint = if similar.is_empty() { String::new() } else { format!("\nSimilar names: {}", similar.join(", ")) };
                fail(format!("E1001 unknown name: {input}{hint}"))
            }
            many => {
                let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind.as_str())).collect();
                fail(format!("E1002 ambiguous name: {input} could be\n{}", list.join("\n")))
            }
        }
    }

    fn is_module(&self, input: &str) -> Result<bool, Failure> {
        Ok(self.resolve_module(&self.index()?, input).is_some())
    }

    fn resolve_module<'a>(&self, ix: &'a Index, input: &str) -> Option<&'a ModuleRow> {
        let input = input.replace('.', "/");
        let exact = ix.modules.iter().find(|m| m.path == input);
        let suffix: Vec<&ModuleRow> = ix.modules.iter().filter(|m| m.path.ends_with(&format!("/{input}"))).collect();
        exact.or(if suffix.len() == 1 { Some(suffix[0]) } else { None })
    }

    /// The symbol index (one line per source symbol), test modules, and open
    /// work records expanded with the context of the symbols they name.
    fn session(&self, scopes: &[&str]) -> Out {
        const CAP: usize = 80;
        let ix = self.index()?;
        let in_scope =
            |m: &ModuleRow| scopes.is_empty() || scopes.iter().any(|s| self.resolve_module(&ix, s).is_some_and(|r| r.path == m.path));
        let mut out = String::from(
            "Symbols per module (types are capitalized). `inspect <name>...` gives signatures and context; \
             `view <module>` an outline.\n",
        );
        let mut tests = vec![];
        for m in ix.modules.iter().filter(|m| in_scope(m)) {
            let syms: Vec<&Sym> = ix.syms.iter().filter(|s| s.module == m.path).collect();
            if m.dir == "test" {
                tests.push(format!("{} ({})", m.path, syms.len()));
                continue;
            }
            // Names only: this is re-sent on every turn, so it must stay small (D-12).
            let names: Vec<&str> = syms.iter().take(CAP).map(|s| s.name.as_str()).collect();
            let more = if syms.len() > CAP { format!(", …{} more", syms.len() - CAP) } else { String::new() };
            out += &format!("{} — {}\n  {}{more}\n", m.path, module_doc(m), names.join(", "));
        }
        out += &format!("\nTest modules (tests): {}\n", tests.join(", "));
        out += &self.status()?;
        out += &self.work_context(&ix)?;
        out += "\n";
        out += REFERENCE;
        Ok(out)
    }

    /// Open work records expanded with the context of the symbols they name.
    fn work_context(&self, ix: &Index) -> Out {
        let mut out = String::new();
        for (_, text) in self.open_work()? {
            let mut named = vec![];
            for word in text.split(|c: char| !(c.is_alphanumeric() || "_./".contains(c))) {
                let word = word.trim_matches(|c| c == '.' || c == '/');
                if word.contains('.')
                    && let Ok(i) = self.resolve(ix, word)
                    && !named.contains(&i)
                {
                    named.push(i);
                }
            }
            if !named.is_empty() {
                out += "\n== Context of the symbols the work record names ==\n";
                let (full, mut shown) = (named.iter().copied().collect(), HashSet::new());
                for &i in &named {
                    out += &self.bundle(ix, i, true, 0, &full, &mut shown)?;
                    out += "\n";
                }
            }
        }
        Ok(out)
    }

    /// `session start --task`: instead of the full index, the symbols most
    /// likely relevant to the task (signature and why each was picked),
    /// within a fixed budget, plus the module list.
    fn session_task(&self, task: &str) -> Out {
        const BUDGET: usize = 8000; // bytes, about 2k tokens
        let ix = self.index()?;
        let n = ix.syms.len();
        let mut modules = String::from("Modules (`view <module>` for an outline):\n");
        let mut tests = vec![];
        for m in &ix.modules {
            if m.dir == "test" {
                tests.push(m.path.as_str());
            } else {
                modules += &format!("  {} — {}\n", m.path, module_doc(m));
            }
        }
        modules += &format!("Test modules: {}\n", tests.join(", "));

        // Direct matches: rare task words weigh more; a word in the name
        // counts 3, in the doc line 2, anywhere else in the code 1.
        let fields: Vec<[HashSet<String>; 3]> = ix
            .syms
            .iter()
            .map(|s| {
                let doc: String = s.text.lines().filter(|l| l.trim_start().starts_with("///")).collect::<Vec<_>>().join(" ");
                [words(&s.name).into_iter().collect(), words(&doc).into_iter().collect(), words(&s.text).into_iter().collect()]
            })
            .collect();
        let mut terms = words(task);
        terms.sort();
        terms.dedup();
        let mut score = vec![0.0f64; n];
        let mut why: Vec<Vec<(f64, String)>> = vec![vec![]; n];
        for t in &terms {
            let df = fields.iter().filter(|f| f.iter().any(|w| w.contains(t))).count();
            if df == 0 {
                continue;
            }
            // squared, so one rare word (a code, a name) outweighs several common ones
            let idf = (1.0 + n as f64 / df as f64).ln().powi(2);
            for (i, f) in fields.iter().enumerate() {
                if let Some(k) = f.iter().position(|w| w.contains(t)) {
                    let w = idf * [3.0, 2.0, 1.0][k];
                    score[i] += w;
                    why[i].push((w, t.clone()));
                }
            }
        }
        let direct = score.clone();
        let matched: Vec<String> = why
            .into_iter()
            .map(|mut w| {
                w.sort_by(|a, b| b.0.total_cmp(&a.0));
                let top: Vec<&str> = w.iter().take(4).map(|(_, t)| t.as_str()).collect();
                if top.is_empty() { String::new() } else { format!("matches {}", top.join(", ")) }
            })
            .collect();
        let mut why = matched.clone();

        // One hop from the best matches: callers, callees and covering tests;
        // then the non-test symbols (fixtures) the top covering tests use.
        let mut seeds: Vec<usize> = (0..n).filter(|&i| direct[i] > 0.0).collect();
        seeds.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]));
        seeds.truncate(10);
        let boost = |j: usize, s: f64, reason: String, score: &mut Vec<f64>, why: &mut Vec<String>| {
            if direct[j] + s > score[j] {
                score[j] = direct[j] + s;
                why[j] = if matched[j].is_empty() { reason } else { format!("{}; {reason}", matched[j]) };
            }
        };
        for &s in &seeds {
            let q = ix.syms[s].qname();
            for &j in &ix.uses[s] {
                boost(j, direct[s] / 4.0, format!("used by {q}"), &mut score, &mut why);
            }
            let mut covering = vec![];
            for j in (0..n).filter(|&j| ix.uses[j].contains(&s)) {
                if is_test(&ix, j) {
                    covering.push(j);
                    boost(j, direct[s] / 4.0, format!("tests {q}"), &mut score, &mut why);
                } else {
                    boost(j, direct[s] / 4.0, format!("uses {q}"), &mut score, &mut why);
                }
            }
            covering.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));
            for &t in covering.iter().take(2) {
                for &j in ix.uses[t].iter().filter(|&&j| !is_test(&ix, j)) {
                    boost(j, direct[s] / 8.0, format!("used by the test {}", ix.syms[t].qname()), &mut score, &mut why);
                }
            }
        }

        // Symbols the task names (`module.name`) are listed first.
        let mut named = vec![];
        for word in task.split(|c: char| !(c.is_alphanumeric() || "_./".contains(c))) {
            let word = word.trim_matches(|c| c == '.' || c == '/');
            if word.contains('.')
                && let Ok(i) = self.resolve(&ix, word)
                && !named.contains(&i)
            {
                named.push(i);
            }
        }
        for &i in &named {
            score[i] = f64::MAX;
            why[i] = "named in the task".to_string();
        }
        let mut ranked: Vec<usize> = (0..n).filter(|&i| score[i] > 0.0).collect();
        ranked.sort_by(|&a, &b| score[b].total_cmp(&score[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));
        let mut packet = String::new();
        let mut room = BUDGET.saturating_sub(modules.len());
        let mut shown = 0;
        for &i in &ranked {
            // whole entries only: skip one that doesn't fit, try smaller ones
            let entry = format!("-- {}  ({})\n{}\n", ix.syms[i].qname(), why[i], gleam::signature(&ix.syms[i].item()));
            if entry.len() <= room {
                room -= entry.len();
                packet += &entry;
                shown += 1;
            }
        }
        let mut out = format!(
            "Symbols most likely relevant to the task, best first ({shown} of {} candidates; signatures only). \
             `inspect <name>... --context` gives bodies and context.\n{packet}\n{modules}",
            ranked.len()
        );
        out += &self.status()?;
        out += &self.work_context(&ix)?;
        out += "\n";
        out += REFERENCE;
        Ok(out)
    }

    fn open_work(&self) -> Result<Vec<(i64, String)>, Failure> {
        let mut stmt = self.db.prepare("SELECT id, text FROM work WHERE open = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Open work records and broken symbols.
    fn status(&self) -> Result<String, Failure> {
        let mut out = String::new();
        let work = self.open_work()?;
        out += &format!("\nOpen work records: {}\n", if work.is_empty() { "none" } else { "" });
        for (id, text) in work {
            out += &format!("#{id}\n{}\n", text.trim_end());
        }
        let broken: Vec<String> =
            self.db.prepare("SELECT symbol FROM broken ORDER BY symbol")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        out += &format!("Broken symbols: {}\n", if broken.is_empty() { "none".to_string() } else { broken.join(", ") });
        Ok(out)
    }

    /// A module is shown as an outline unless `bodies` is set. Past `budget`,
    /// module bodies fall back to outlines, and what still doesn't fit is
    /// listed to view separately (D-14).
    fn view(&self, names: &[&str], bodies: bool, budget: usize) -> Out {
        let ix = self.index()?;
        let (mut out, mut left) = (String::new(), vec![]);
        let fits = |out: &str, text: &str| out.is_empty() || out.len() + text.len() <= budget;
        for &name in names {
            let text = if let Some(m) = self.resolve_module(&ix, name) {
                let mut body = format!("// ── module {} ({}) ──\n", m.path, m.dir);
                if !m.header.is_empty() {
                    body += &format!("{}\n\n", m.header);
                }
                for s in ix.syms.iter().filter(|s| s.module == m.path) {
                    body += &format!("{}\n\n", s.text);
                }
                if bodies && fits(&out, &body) {
                    body
                } else {
                    let mut text =
                        format!("// ── module {} ({}), outline: `view <name>` for a body, `--bodies` for all ──\n", m.path, m.dir);
                    if bodies {
                        text += &format!("// bodies left out to fit the size limit; `view {} --bodies` alone shows them\n", m.path);
                    }
                    let doc: Vec<&str> = m.header.lines().filter(|l| l.starts_with("////")).collect();
                    if !doc.is_empty() {
                        text += &format!("{}\n\n", doc.join("\n"));
                    }
                    for s in ix.syms.iter().filter(|s| s.module == m.path) {
                        let broken = self.db.query_row("SELECT 1 FROM broken WHERE symbol = ?1", [s.qname()], |_| Ok(())).optional()?;
                        text += &format!("{}{}\n\n", gleam::signature(&s.item()), if broken.is_some() { "  // BROKEN" } else { "" });
                    }
                    text
                }
            } else {
                let s = &ix.syms[self.resolve(&ix, name)?];
                format!("// ── {} ({}) ──\n{}\n\n", s.qname(), s.kind.as_str(), s.text)
            };
            if left.is_empty() && fits(&out, &text) {
                out += &text;
            } else {
                left.push(name);
            }
        }
        if !left.is_empty() {
            out +=
                &format!("Not shown, to fit the size limit: `solace view {}{}`\n", left.join(" "), if bodies { " --bodies" } else { "" });
        }
        Ok(out)
    }

    /// Several names give one document: each signature is printed once, and
    /// symbols asked for are printed in full only once (D-10, D-14). Over
    /// `budget`, the bundles are trimmed in priority order (callers, then the
    /// test example, then the signatures of what each uses); if that is not
    /// enough, the names that don't fit are listed to inspect separately.
    fn inspect(&self, names: &[&str], context: bool, budget: usize) -> Out {
        let ix = self.index()?;
        let mut targets: Vec<(&str, Option<usize>)> = vec![];
        for &name in names {
            match self.resolve(&ix, name) {
                Ok(i) => targets.push((name, Some(i))),
                // a module name gets its outline rather than an error
                Err(_) if self.resolve_module(&ix, name).is_some() => targets.push((name, None)),
                Err(e) => return Err(e),
            }
        }
        let full: HashSet<usize> = targets.iter().filter_map(|t| t.1).collect();
        const LAST: u8 = 3;
        for level in 0..=LAST {
            if level > 0 && !context {
                break;
            }
            let (mut out, mut shown, mut left) = (String::new(), HashSet::new(), vec![]);
            for &(name, i) in &targets {
                let before = shown.clone();
                let part = match i {
                    Some(i) => self.bundle(&ix, i, context, level, &full, &mut shown)?,
                    None => self.view(&[name], false, budget)?,
                } + "\n";
                let last = level == LAST || !context;
                if last && (!left.is_empty() || (!out.is_empty() && out.len() + part.len() > budget)) {
                    shown = before;
                    left.push(ix.syms.get(i.unwrap_or(usize::MAX)).map_or(name.to_string(), |s| s.qname()));
                    continue;
                }
                out += &part;
            }
            if out.len() > budget && level < LAST && context {
                continue;
            }
            let trimmed = [
                "",
                "callers as a count only (`solace refs <name>` lists them)",
                "tests by name only",
                "what each uses by name only (`view <name>` for its source)",
            ];
            if level > 0 {
                out += &format!("Trimmed to fit the size limit: {}.\n", trimmed[1..=level as usize].join("; "));
            }
            if !left.is_empty() {
                out += &format!(
                    "Not shown, to fit the size limit: `solace inspect {}{}`\n",
                    left.join(" "),
                    if context { " --context" } else { "" }
                );
            }
            return Ok(out);
        }
        unreachable!()
    }

    /// `level` is how far the context is trimmed (see `inspect`); `full` holds
    /// the symbols printed in full in this reply, `shown` the signatures
    /// printed so far.
    fn bundle(&self, ix: &Index, i: usize, context: bool, level: u8, full: &HashSet<usize>, shown: &mut HashSet<usize>) -> Out {
        let s = &ix.syms[i];
        let mut out = format!("// ── {} ({}) ──\n{}\n", s.qname(), s.kind.as_str(), s.text);
        if !context {
            return Ok(out);
        }
        let mut entry = |u: usize| {
            let q = ix.syms[u].qname();
            if full.contains(&u) {
                format!("-- {q} (in full in this reply)\n")
            } else if !shown.insert(u) {
                format!("-- {q} (above)\n")
            } else {
                format!("-- {q}\n{}\n", gleam::signature(&ix.syms[u].item()))
            }
        };
        let used_by: Vec<usize> = (0..ix.syms.len()).filter(|&j| ix.uses[j].contains(&i)).collect();
        let (tests, callers): (Vec<usize>, Vec<usize>) = used_by.into_iter().partition(|&j| is_test(ix, j));
        if !ix.uses[i].is_empty() {
            let mut uses = ix.uses[i].clone();
            uses.sort_by_key(|&u| (ix.syms[u].kind != Kind::Type, ix.syms[u].kind != Kind::Const, ix.syms[u].qname()));
            if level >= 3 {
                let names: Vec<String> = uses.iter().map(|&u| ix.syms[u].qname()).collect();
                out += &format!("\n== Uses ==\n  {}\n", names.join(", "));
            } else {
                out += "\n== Uses ==\n";
                for u in uses {
                    out += &entry(u);
                }
            }
        }
        if !callers.is_empty() && level >= 1 {
            out += &format!("\n== Used by ({}): solace refs {} ==\n", callers.len(), s.qname());
        } else if !callers.is_empty() {
            out += &format!("\n== Used by ({}) ==\n", callers.len());
            for &c in callers.iter().take(20) {
                out += &entry(c);
            }
            if callers.len() > 20 {
                out += &format!("…and {} more (solace refs {})\n", callers.len() - 20, s.qname());
            }
        }
        if !tests.is_empty() {
            // The smallest test in full, as a worked example (saves a turn of
            // browsing test modules); the rest by name.
            const EXAMPLE_CAP: usize = 1600; // bytes, about 400 tokens
            let example = tests
                .iter()
                .copied()
                .filter(|t| level < 2 && !full.contains(t))
                .min_by_key(|&t| ix.syms[t].text.len())
                .filter(|&t| ix.syms[t].text.len() <= EXAMPLE_CAP);
            out += &format!("\n== Tests that use it ({}) ==\n", tests.len());
            if let Some(t) = example {
                out += &format!("-- {} (example)\n{}\n", ix.syms[t].qname(), ix.syms[t].text);
            }
            let names: Vec<String> = tests.iter().filter(|&&t| Some(t) != example).map(|&t| ix.syms[t].qname()).collect();
            if !names.is_empty() {
                out += &format!("  {}\n", names.join("\n  "));
            }
        }
        if let Some(diag) =
            self.db.query_row("SELECT diagnostic FROM broken WHERE symbol = ?1", [s.qname()], |r| r.get::<_, String>(0)).optional()?
        {
            out += &format!("\n== Broken ==\n{diag}\n");
        }
        Ok(out)
    }

    fn refs(&self, name: &str) -> Out {
        let ix = self.index()?;
        let i = self.resolve(&ix, name)?;
        let users: Vec<usize> = (0..ix.syms.len()).filter(|&j| ix.uses[j].contains(&i)).collect();
        if users.is_empty() {
            return Ok(format!("Nothing uses {}.\n", ix.syms[i].qname()));
        }
        let mut out = format!("{} is used by {}:\n", ix.syms[i].qname(), users.len());
        for u in users {
            out += &format!("-- {}\n{}\n", ix.syms[u].qname(), gleam::signature(&ix.syms[u].item()));
        }
        Ok(out)
    }

    fn find(&self, query: &str) -> Out {
        let ix = self.index()?;
        let q = query.to_lowercase();
        let mut hits: Vec<(bool, String)> = vec![];
        for s in &ix.syms {
            let lines: Vec<String> = s
                .text
                .lines()
                .enumerate()
                .filter(|(_, l)| l.to_lowercase().contains(&q))
                .map(|(n, l)| format!("  {:>3}: {}", n + 1, l.trim()))
                .collect();
            let name_hit = s.name.to_lowercase().contains(&q);
            if name_hit || !lines.is_empty() {
                let more = if lines.len() > 4 { format!("\n  … {} more lines", lines.len() - 4) } else { String::new() };
                let shown: Vec<String> = lines.into_iter().take(4).collect();
                hits.push((name_hit, format!("{} ({})\n{}{}", s.qname(), s.kind.as_str(), shown.join("\n"), more)));
            }
        }
        if hits.is_empty() {
            return Ok(format!("No matches for \"{query}\".\n"));
        }
        hits.sort_by_key(|h| !h.0);
        let total = hits.len();
        let mut out: String = hits.iter().take(40).map(|h| h.1.trim_end().to_string() + "\n").collect();
        if total > 40 {
            out += &format!("… {} more symbols match; narrow the search.\n", total - 40);
        }
        Ok(out)
    }

    // --- changing ----------------------------------------------------------------

    fn submit(&self, target: Target, raw: &str) -> Out {
        let raw: String = raw.lines().filter(|l| !l.starts_with("// ── ")).collect::<Vec<_>>().join("\n");
        let formatted = format_gleam(&raw)?;
        let parsed = gleam::split(&formatted);
        let ix = self.index()?;
        let (module, mut notes) = match target {
            Target::Imports(m) => {
                (self.resolve_module(&ix, m).ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))?.path.clone(), vec![])
            }
            Target::Module(m) => {
                (self.resolve_module(&ix, m).ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))?.path.clone(), vec![])
            }
        };
        let header = &ix.modules.iter().find(|m| m.path == module).unwrap().header;
        self.db.execute_batch("BEGIN")?;
        let result = (|| -> Result<(Option<String>, String), Failure> {
            let target_name: Option<String> = match target {
                Target::Module(_) => {
                    let existing: Vec<&Sym> = ix.syms.iter().filter(|s| s.module == module).collect();
                    let key = |name: &str, kind: Kind| (name.to_string(), kind == Kind::Type);
                    let submitted: HashSet<(String, bool)> = parsed.items.iter().map(|i| key(&i.name, i.kind)).collect();
                    if submitted.len() != parsed.items.len() {
                        return fail("E0103 the module text defines the same name twice.");
                    }
                    let missing: Vec<String> =
                        existing.iter().filter(|s| !submitted.contains(&key(&s.name, s.kind))).map(|s| s.name.clone()).collect();
                    if !missing.is_empty() {
                        return fail(format!(
                            "E0102 the module text leaves out: {}. Nothing was saved. Removing a symbol must be explicit (solace delete <name>).",
                            missing.join(", ")
                        ));
                    }
                    let mut header = if parsed.header.is_empty() { header.clone() } else { parsed.header.clone() };
                    let (mut changed, mut added) = (vec![], vec![]);
                    for item in &parsed.items {
                        for path in missing_imports(&ix, &header, &item.text) {
                            header = add_import(&header, &format!("import {path}"));
                            notes.push(format!("added `import {path}` to {module}"));
                        }
                        match existing.iter().find(|s| key(&s.name, s.kind) == key(&item.name, item.kind)) {
                            Some(e) if e.text == item.text => {}
                            Some(e) => {
                                self.db.execute(
                                    "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                                    params![e.id, item.text, revision(&item.text)],
                                )?;
                                self.history(&e.id, "edit", &item.name)?;
                                changed.push(item.name.clone());
                            }
                            None => {
                                let id = new_id(item.kind, &module, &item.name);
                                let ord: i64 = self.db.query_row(
                                    "SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1",
                                    [&module],
                                    |r| r.get(0),
                                )?;
                                self.db.execute(
                                    "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                    params![id, module, item.name, item.kind.as_str(), item.text, ord, revision(&item.text)],
                                )?;
                                self.history(&id, "create", &item.name)?;
                                added.push(item.name.clone());
                            }
                        }
                    }
                    self.set_header(&module, &header)?;
                    let list = |v: &Vec<String>| if v.is_empty() { "none".to_string() } else { v.join(", ") };
                    notes.push(format!("{module}: changed {}; added {}", list(&changed), list(&added)));
                    None
                }
                Target::Imports(_) => {
                    if !parsed.items.is_empty() {
                        return fail("E0101 --imports text may only contain imports and the module comment.");
                    }
                    self.set_header(&module, &parsed.header)?;
                    None
                }
            };
            let ix = self.index()?;
            let diags = self.check_all(&ix)?;
            let whole_module = matches!(target, Target::Module(_));
            let own = |d: &&Diag| match &target_name {
                Some(q) => d.symbol.as_deref() == Some(q.as_str()),
                None if whole_module => d.module.as_deref() == Some(module.as_str()),
                None => d.symbol.is_none() && d.module.as_deref() == Some(module.as_str()),
            };
            let own_errors: Vec<&Diag> = diags.iter().filter(|d| d.error).filter(own).collect();
            if !own_errors.is_empty() {
                let text: Vec<&str> = own_errors.iter().map(|d| d.text.as_str()).collect();
                return fail(format!("Not saved: the submitted text has errors.\n\n{}", text.join("\n\n")));
            }
            let what = if whole_module { module.clone() } else { format!("{module} imports") };
            let mut out = format!("Saved {}.\n", target_name.as_deref().unwrap_or(&what));
            for d in diags.iter().filter(|d| !d.error).filter(own) {
                out += &format!("\n{}\n", d.text);
            }
            out += &broken_report(&diags);
            Ok((target_name, out))
        })();
        match result {
            Ok((_, out)) => {
                self.db.execute_batch("COMMIT")?;
                let notes: String = notes.iter().map(|n| format!("Note: {n}\n")).collect();
                Ok(notes + &out)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK")?;
                self.materialize(&self.index()?)?;
                Err(e)
            }
        }
    }

    /// Replaces or adds symbols in one transaction, type-checked together, so a
    /// type change and its callers can land at once (PROPOSAL.md §7.4).
    /// `name`: the one symbol the text replaces. `new_in`: the module the text
    /// adds a symbol to. Neither: the text is blocks headed `// ── <name> ──`,
    /// as `view` and `inspect` print them; a name that doesn't exist yet adds
    /// a symbol to the module its prefix names.
    fn submit_symbols(&self, name: Option<&str>, new_in: Option<&str>, raw: &str) -> Out {
        let ix = self.index()?;
        let is_header = |l: &str| l.starts_with("// ── ");
        let blocks: Vec<(Option<String>, String)> =
            if raw.lines().filter(|l| is_header(l)).count() > 1 || (name.is_none() && new_in.is_none()) {
                let mut blocks: Vec<(Option<String>, String)> = vec![];
                for line in raw.lines() {
                    if let Some(h) = line.strip_prefix("// ── ") {
                        if h.starts_with("module ") {
                            return fail("E0101 a whole module goes through `submit <module>`.");
                        }
                        let n = h.split([' ', '(']).next().unwrap_or("").to_string();
                        blocks.push((Some(n), String::new()));
                    } else if let Some((_, text)) = blocks.last_mut() {
                        text.push_str(line);
                        text.push('\n');
                    } else if !line.trim().is_empty() {
                        return fail("E0101 text for `submit` with no name must be blocks, each headed `// ── <module>.<name> ──`.");
                    }
                }
                blocks
            } else {
                vec![(name.map(str::to_string), raw.lines().filter(|l| !is_header(l)).collect::<Vec<_>>().join("\n"))]
            };
        if blocks.is_empty() {
            return fail("E0101 nothing to submit.");
        }
        // (module, the symbol replaced, the new item, imports in the text)
        let mut plan: Vec<(String, Option<usize>, Item, String)> = vec![];
        // `submit --new` to a module that doesn't exist yet creates it.
        let mut created: Option<(String, &str)> = None;
        for (block_name, text) in &blocks {
            let parsed = gleam::split(&format_gleam(text)?);
            let [item] = parsed.items.as_slice() else {
                return fail(format!(
                    "E0101 each block holds one symbol; {} defines {} ({}).",
                    block_name.as_deref().unwrap_or("the text"),
                    parsed.items.len(),
                    parsed.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
                ));
            };
            let module_of = |m: &str| {
                self.resolve_module(&ix, m)
                    .or_else(|| m.rsplit_once('.').and_then(|(p, _)| self.resolve_module(&ix, p)))
                    .map(|r| r.path.clone())
                    .ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))
            };
            let (module, existing) = match (new_in, block_name) {
                (Some(m), _) => match module_of(m) {
                    Ok(path) => (path, None),
                    Err(_) if valid_module_path(m) => {
                        let dir = if m.rsplit('/').next().unwrap().ends_with("_test") { "test" } else { "src" };
                        created = Some((m.to_string(), dir));
                        (m.to_string(), None)
                    }
                    Err(e) => return Err(e),
                },
                (None, Some(n)) => match self.resolve(&ix, n) {
                    Ok(i) => (ix.syms[i].module.clone(), Some(i)),
                    Err(e) => match n.rsplit_once('.') {
                        Some((m, _)) if self.resolve_module(&ix, m).is_some() => (module_of(m)?, None),
                        _ => return Err(e),
                    },
                },
                (None, None) => unreachable!(),
            };
            let clash = ix.syms.iter().position(|s| {
                s.module == module
                    && s.name == item.name
                    && (s.kind == Kind::Type) == (item.kind == Kind::Type)
                    && existing != Some(ix.syms.iter().position(|x| x.id == s.id).unwrap())
            });
            if let Some(c) = clash {
                return fail(format!("E1004 {} already exists; submit to it instead.", ix.syms[c].qname()));
            }
            plan.push((module, existing, item.clone(), parsed.header));
        }

        let mut notes: Vec<String> = vec![];
        self.db.execute_batch("BEGIN")?;
        let result = (|| -> Out {
            if let Some((path, dir)) = &created {
                self.db.execute("INSERT INTO modules VALUES (?1, ?2, '')", params![path, dir])?;
                notes.push(format!("created module {path} ({dir}/{path}.gleam)"));
            }
            let mut saved: Vec<String> = vec![];
            for (module, existing, item, imports) in &plan {
                let mut header = self.header(module)?;
                for (path, items, _) in import_statements(imports) {
                    let merged = merge_import(&header, &path, &items);
                    if merged != header {
                        notes.push(format!("imported {path} in {module}"));
                        header = merged;
                    }
                }
                for path in missing_imports(&ix, &header, &item.text) {
                    header = merge_import(&header, &path, &[]);
                    notes.push(format!("added `import {path}` to {module}"));
                }
                self.set_header(module, &header)?;
                match existing {
                    Some(e) => {
                        let e = &ix.syms[*e];
                        self.db.execute(
                            "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, revision = ?5 WHERE id = ?1",
                            params![e.id, item.name, item.kind.as_str(), item.text, revision(&item.text)],
                        )?;
                        self.history(&e.id, "edit", &item.name)?;
                    }
                    None => {
                        let id = new_id(item.kind, module, &item.name);
                        let ord: i64 =
                            self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [module], |r| r.get(0))?;
                        self.db.execute(
                            "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![id, module, item.name, item.kind.as_str(), item.text, ord, revision(&item.text)],
                        )?;
                        self.history(&id, "create", &item.name)?;
                    }
                }
                saved.push(format!("{module}.{}", item.name));
            }
            let modules: HashSet<&str> = plan.iter().map(|p| p.0.as_str()).collect();
            let own = |d: &Diag| match &d.symbol {
                Some(s) => saved.contains(s),
                None => d.module.as_deref().is_some_and(|m| modules.contains(m)),
            };
            let mut diags = self.check_all(&self.index()?)?;
            // Unqualified types and constructors (`Option`, `None`, a project
            // type) that aren't imported: import them and check once more.
            // The compiler stops at the first module with errors, so repeat.
            for _ in 0..plan.len() + 2 {
                let mut fixed = false;
                for d in diags.iter().filter(|d| d.error && own(d)) {
                    if let (Some(m), Some((path, item))) = (&d.module, unimported_name(&ix, &d.text)) {
                        let header = self.header(m)?;
                        let merged = merge_import(&header, &path, std::slice::from_ref(&item));
                        if merged != header && path != *m {
                            self.set_header(m, &merged)?;
                            notes.push(format!("imported `{item}` from {path} in {m}"));
                            fixed = true;
                        }
                    }
                }
                if !fixed {
                    break;
                }
                diags = self.check_all(&self.index()?)?;
            }
            let own_errors: Vec<&str> = diags.iter().filter(|d| d.error && own(d)).map(|d| d.text.as_str()).collect();
            if !own_errors.is_empty() {
                return fail(format!("Not saved: the submitted text has errors.\n\n{}", own_errors.join("\n\n")));
            }
            let mut out = format!("Saved {}.\n", saved.join(", "));
            for d in diags.iter().filter(|d| !d.error && own(d)) {
                out += &format!("\n{}\n", d.text);
            }
            out += &broken_report(&diags);
            // The compiler stops at the first module with errors, so list
            // every user of a changed signature from the reference graph.
            if diags.iter().any(|d| d.error) {
                let after = self.index()?;
                let broken: HashSet<&str> = diags.iter().filter_map(|d| d.symbol.as_deref()).collect();
                let mut unchecked: Vec<String> = vec![];
                for (_, existing, item, _) in &plan {
                    let Some(e) = existing else { continue };
                    let before = &ix.syms[*e];
                    let Some(a) = after.syms.iter().position(|s| s.id == before.id) else { continue };
                    if gleam::signature(&before.item()) == gleam::signature(&item.clone()) {
                        continue;
                    }
                    for j in (0..after.syms.len()).filter(|&j| after.uses[j].contains(&a)) {
                        let n = after.syms[j].qname();
                        if !broken.contains(n.as_str()) && !saved.contains(&n) && !unchecked.contains(&n) {
                            unchecked.push(n);
                        }
                    }
                }
                if !unchecked.is_empty() {
                    out += &format!(
                        "\nA signature changed. These other users of it are not checked until the errors above are fixed:\n  {}\n",
                        unchecked.join("\n  ")
                    );
                }
            }
            Ok(out)
        })();
        match result {
            Ok(out) => {
                self.db.execute_batch("COMMIT")?;
                let notes: String = notes.iter().map(|n| format!("Note: {n}\n")).collect();
                Ok(notes + &out)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK")?;
                self.materialize(&self.index()?)?;
                Err(e)
            }
        }
    }

    /// Replaces one exact piece of a symbol's text, then submits the whole
    /// symbol as usual (formatted, checked, one transaction).
    fn submit_replace(&self, name: &str, old: &str, new: &str) -> Out {
        let ix = self.index()?;
        let s = &ix.syms[self.resolve(&ix, name)?];
        if old.is_empty() {
            return usage("`old` is empty; give the exact text to replace.");
        }
        match s.text.matches(old).count() {
            1 => self.submit_symbols(Some(&s.qname()), None, &s.text.replacen(old, new, 1)),
            0 => fail(format!(
                "Not saved: E0104 `old` does not occur in {}. It must match the current text exactly (`view {}` shows it).",
                s.qname(),
                s.qname()
            )),
            k => fail(format!(
                "Not saved: E0105 `old` occurs {k} times in {}; include more of the surrounding text so it occurs once.",
                s.qname()
            )),
        }
    }

    fn header(&self, module: &str) -> Result<String, Failure> {
        Ok(self.db.query_row("SELECT header FROM modules WHERE path = ?1", [module], |r| r.get(0))?)
    }

    fn set_header(&self, module: &str, header: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE modules SET header = ?2 WHERE path = ?1", params![module, header])?;
        Ok(())
    }

    fn history(&self, id: &str, action: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute(
            "INSERT INTO history(at, symbol, action, detail) VALUES (?1, ?2, ?3, ?4)",
            params![now() as i64, id, action, detail],
        )?;
        Ok(())
    }

    fn delete(&self, name: &str) -> Out {
        let ix = self.index()?;
        let i = self.resolve(&ix, name)?;
        let users: Vec<String> = (0..ix.syms.len()).filter(|&j| ix.uses[j].contains(&i)).map(|j| ix.syms[j].qname()).collect();
        if !users.is_empty() {
            return fail(format!("E5001 {} is still used by:\n  {}", ix.syms[i].qname(), users.join("\n  ")));
        }
        self.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [&ix.syms[i].id])?;
        self.history(&ix.syms[i].id, "retire", "")?;
        let diags = self.check_all(&self.index()?)?;
        Ok(format!("Deleted {}.\n{}", ix.syms[i].qname(), broken_report(&diags)))
    }

    fn check(&self) -> Out {
        let diags = self.check_all(&self.index()?)?;
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        let warnings = diags.len() - errors.len();
        if errors.is_empty() {
            Ok(format!("No errors ({warnings} warnings).\n"))
        } else {
            fail(format!("{} errors, {warnings} warnings:\n\n{}", errors.len(), errors.join("\n\n")))
        }
    }

    fn test(&self) -> Out {
        let ix = self.index()?;
        let files = self.materialize(&ix)?;
        let (ok, output) = self.gleam(&["test"])?;
        let diags = diagnostics(&output, &files);
        let errors: Vec<&str> = diags.iter().filter(|d| d.error).map(|d| d.text.as_str()).collect();
        if !errors.is_empty() {
            return fail(format!("Tests did not compile:\n\n{}", errors.join("\n\n")));
        }
        let mut out = String::new();
        for line in output.lines() {
            let line = line.trim_start_matches('.');
            let t = line.trim();
            if t.is_empty() || ["Compiling ", "Compiled ", "Running ", "Checking "].iter().any(|p| t.starts_with(p)) {
                continue;
            }
            out += &relocate(line, &files);
            out.push('\n');
        }
        if ok { Ok(out) } else { fail(out) }
    }

    fn work_add(&self, text: &str) -> Out {
        self.db.execute("INSERT INTO work(text) VALUES (?1)", [text])?;
        Ok(format!("Work record #{} added.\n", self.db.last_insert_rowid()))
    }

    fn work_list(&self) -> Out {
        self.status()
    }

    fn work_done(&self, id: &str) -> Out {
        let id: i64 = id.trim_start_matches('#').parse().map_err(|_| Failure(format!("Not a work record id: {id}"), 2))?;
        match self.db.execute("UPDATE work SET open = 0 WHERE id = ?1", [id])? {
            0 => fail(format!("No work record #{id}.")),
            _ => Ok(format!("Work record #{id} closed.\n")),
        }
    }
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

fn module_doc(m: &ModuleRow) -> &str {
    m.header.lines().find(|l| l.starts_with("////")).map_or("", |l| l.trim_start_matches('/').trim())
}

fn is_test(ix: &Index, i: usize) -> bool {
    ix.modules.iter().any(|m| m.path == ix.syms[i].module && m.dir == "test")
}

/// Formats Gleam source, which doubles as the syntax check.
fn format_gleam(source: &str) -> Result<String, Failure> {
    let mut child =
        Command::new("gleam").args(["format", "--stdin"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(source.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let msg = strip_ansi(&String::from_utf8_lossy(&out.stderr)).replace("<stdin>", "submitted text");
        fail(format!("Not saved: E0100 syntax error.\n\n{}", msg.trim()))
    }
}

/// `import` paths needed for `alias.name` references to modules the header
/// doesn't import yet: project modules by last path segment, or the stdlib.
fn missing_imports(ix: &Index, header: &str, text: &str) -> Vec<String> {
    let aliases: HashSet<String> = gleam::imports(header).into_iter().map(|i| i.alias).collect();
    let mut out = vec![];
    for r in gleam::references(text) {
        let Ref::Qualified(alias, _) = r else { continue };
        if aliases.contains(&alias) {
            continue;
        }
        let project: Vec<&ModuleRow> =
            ix.modules.iter().filter(|m| m.dir == "src" && m.path.rsplit('/').next() == Some(alias.as_str())).collect();
        let path = match project.as_slice() {
            [m] => Some(m.path.clone()),
            _ if STDLIB.contains(&alias.as_str()) => Some(format!("gleam/{alias}")),
            _ => None,
        };
        if let Some(p) = path
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// Each import in a header: (module, unqualified items as written, e.g.
/// `type Option` or `None`, and the line range it spans).
fn import_statements(header: &str) -> Vec<(String, Vec<String>, (usize, usize))> {
    let lines: Vec<&str> = header.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let Some(rest) = lines[i].strip_prefix("import ") else {
            i += 1;
            continue;
        };
        let start = i;
        while !lines[i].contains('}') && lines[start].contains(".{") && i + 1 < lines.len() {
            i += 1;
        }
        let stmt = lines[start..=i].join(" ");
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/').collect();
        let items = match (stmt.find(".{"), stmt.find('}')) {
            (Some(a), Some(b)) => {
                stmt[a + 2..b].split(',').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect()
            }
            _ => vec![],
        };
        out.push((module, items, (start, i)));
        i += 1;
    }
    out
}

/// Adds unqualified `items` to the header's import of `module`, or adds the
/// import. The result is formatted.
fn merge_import(header: &str, module: &str, items: &[String]) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let Some((_, existing, (start, end))) = import_statements(header).into_iter().find(|(m, _, _)| m == module) else {
        let line = if items.is_empty() { format!("import {module}") } else { format!("import {module}.{{{}}}", items.join(", ")) };
        let added = add_import(header, &line);
        return format_gleam(&added).map_or(added, |f| f.trim().to_string());
    };
    if items.iter().all(|i| existing.contains(i)) {
        return header.to_string();
    }
    let stmt = lines[start..=end].join(" ");
    let suffix = stmt.rfind('}').map_or_else(|| stmt[format!("import {module}").len()..].to_string(), |b| stmt[b + 1..].to_string());
    let mut all = existing;
    all.extend(items.iter().filter(|i| !all.contains(i)).cloned().collect::<Vec<_>>());
    let line = format!("import {module}.{{{}}}{}", all.join(", "), suffix.trim_end());
    let mut out: Vec<String> = lines[..start].iter().map(|s| s.to_string()).collect();
    out.push(line);
    out.extend(lines[end + 1..].iter().map(|s| s.to_string()));
    let joined = out.join("\n");
    format_gleam(&joined).map_or(joined, |f| f.trim().to_string())
}

/// From a compiler error about an unknown type or constructor, the import
/// that provides it: (module, item), when exactly one module does.
fn unimported_name(ix: &Index, error: &str) -> Option<(String, String)> {
    let between = |pre: &str, post: &str| {
        error.find(pre).and_then(|a| error[a + pre.len()..].find(post).map(|b| error[a + pre.len()..a + pre.len() + b].to_string()))
    };
    let (name, is_type) = match (between("The type `", "` is not defined"), between("constructor `", "` is not in scope")) {
        (Some(t), _) => (t, true),
        (None, Some(c)) => (c, false),
        _ => return None,
    };
    const STD: &[(&str, &str, bool)] = &[
        ("Option", "gleam/option", true),
        ("Some", "gleam/option", false),
        ("None", "gleam/option", false),
        ("Dict", "gleam/dict", true),
        ("Set", "gleam/set", true),
        ("Order", "gleam/order", true),
        ("Lt", "gleam/order", false),
        ("Eq", "gleam/order", false),
        ("Gt", "gleam/order", false),
    ];
    let item = if is_type { format!("type {name}") } else { name.clone() };
    if let Some((_, m, _)) = STD.iter().find(|(n, _, t)| *n == name && *t == is_type) {
        return Some((m.to_string(), item));
    }
    let mut found: Vec<&str> = ix
        .syms
        .iter()
        .filter(|s| s.kind == Kind::Type && gleam::signature(&s.item()).contains("pub type"))
        .filter(|s| if is_type { s.name == name } else { gleam::constructors(&s.text).contains(&name) })
        .map(|s| s.module.as_str())
        .collect();
    found.dedup();
    match found.as_slice() {
        [m] => Some((m.to_string(), item)),
        _ => None,
    }
}

/// Adds an import line after the existing imports (or after the module doc).
fn add_import(header: &str, line: &str) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let at = lines.iter().rposition(|l| l.starts_with("import ") || l.starts_with("////")).map_or(0, |i| i + 1);
    // a multi-line import ends at its closing brace
    let at = if at > 0 && lines[at - 1].ends_with('{') {
        lines[at..].iter().position(|l| l.starts_with('}')).map_or(at, |p| at + p + 1)
    } else {
        at
    };
    let mut out: Vec<&str> = lines[..at].to_vec();
    if at > 0 && lines[at - 1].starts_with("////") {
        out.push("");
    }
    out.push(line);
    out.extend_from_slice(&lines[at..]);
    out.join("\n").trim().to_string()
}

// --- compiler output ---------------------------------------------------------------

struct FileMap {
    module: String,
    file: PathBuf,
    /// (first line, last line, qualified name)
    ranges: Vec<(usize, usize, String)>,
}

struct Diag {
    error: bool,
    module: Option<String>,
    symbol: Option<String>,
    text: String,
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Where a file position falls: (module, symbol and its first line).
fn locate<'a>(files: &'a [FileMap], path: &str, line: usize) -> Option<(&'a str, Option<(&'a str, usize)>)> {
    let f = files.iter().find(|f| f.file.to_string_lossy() == path || f.file.ends_with(path))?;
    let sym = f.ranges.iter().find(|r| r.0 <= line && line <= r.1).map(|r| (r.2.as_str(), r.0));
    Some((&f.module, sym))
}

/// Rewrites `path/to/file.gleam:LINE` references in a line to symbol positions.
fn relocate(line: &str, files: &[FileMap]) -> String {
    let Some(end) = line.find(".gleam:") else { return line.to_string() };
    let start = line[..end].rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
    let path = &line[start..end + 6];
    let rest = &line[end + 7..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let (Ok(n), Some((module, sym))) = (digits.parse::<usize>(), locate(files, path, digits.parse().unwrap_or(0))) else {
        return line.to_string();
    };
    let tail = &rest[digits.len()..];
    let tail = tail.strip_prefix(':').map_or(tail, |t| t.trim_start_matches(|c: char| c.is_ascii_digit()));
    let place = match sym {
        Some((q, first)) => format!("{q}, line {}", n - first + 1),
        None => format!("{module} imports"),
    };
    format!("{}{place}{tail}", &line[..start])
}

/// Splits compiler output into diagnostics located by symbol.
fn diagnostics(output: &str, files: &[FileMap]) -> Vec<Diag> {
    let mut blocks: Vec<Vec<&str>> = vec![];
    for line in output.lines() {
        let t = line.trim_start();
        if line.starts_with("error") || line.starts_with("warning") {
            blocks.push(vec![line]);
        } else if ["Compiling ", "Compiled ", "Checking ", "Running "].iter().any(|p| t.starts_with(p)) {
            continue;
        } else if let Some(b) = blocks.last_mut() {
            b.push(line);
        }
    }
    blocks
        .into_iter()
        .map(|b| {
            let error = b[0].starts_with("error");
            let mut module = None;
            let mut symbol = None;
            let mut offset = 0;
            let mut text = vec![];
            for line in &b {
                if let Some(loc) = line.trim_start().strip_prefix("┌─ ") {
                    let mut parts = loc.rsplitn(3, ':');
                    let (_col, row, path) = (parts.next(), parts.next(), parts.next());
                    if let (Some(path), Some(Ok(row))) = (path, row.map(|r| r.parse::<usize>()))
                        && let Some((m, sym)) = locate(files, path, row)
                    {
                        module = Some(m.to_string());
                        match sym {
                            Some((q, first)) => {
                                symbol = Some(q.to_string());
                                offset = first - 1;
                                text.push(format!("  at {q}, line {}", row - offset));
                            }
                            None => text.push(format!("  at {m} imports")),
                        }
                        continue;
                    }
                }
                // renumber the code excerpt to lines within the symbol
                let t = line.trim_start();
                let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                if offset > 0
                    && !digits.is_empty()
                    && t[digits.len()..].starts_with(" │")
                    && let Ok(n) = digits.parse::<usize>()
                    && n > offset
                {
                    text.push(format!("{:>4}{}", n - offset, &t[digits.len()..]));
                    continue;
                }
                text.push(line.to_string());
            }
            Diag { error, module, symbol, text: text.join("\n").trim_end().to_string() }
        })
        .collect()
}

fn broken_report(diags: &[Diag]) -> String {
    let errors: Vec<&Diag> = diags.iter().filter(|d| d.error).collect();
    if errors.is_empty() {
        return "The project type-checks.\n".to_string();
    }
    let mut names: Vec<String> =
        errors.iter().map(|d| d.symbol.clone().or(d.module.clone().map(|m| m + " imports")).unwrap_or("?".into())).collect();
    names.dedup();
    let mut out = format!("\nBroken symbols ({}): {}\n", names.len(), names.join(", "));
    for d in errors.iter().take(5) {
        out += &format!("\n{}\n", d.text);
    }
    if errors.len() > 5 {
        out += &format!("\n… {} more errors (solace check)\n", errors.len() - 5);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-14: a multi-name inspect prints each signature once, and trims, then
    /// defers names, to stay within the budget.
    #[test]
    fn inspect_dedups_and_fits_budget() {
        let root = env::temp_dir().join(format!("solace-inspect-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pilot-large");
        assert!(Command::new("cp").arg("-r").arg(&src).arg(&root).status().unwrap().success());
        let _ = fs::remove_dir_all(root.join(".solace"));
        import(&root).ok().unwrap();
        let store = Store { db: Connection::open(root.join(".solace/store.db")).unwrap(), root: root.clone() };
        let names = ["store.put_shipment", "shipments.dispatch", "shipments.deliver", "fulfillment.ship"];
        let ok = |r: Out| r.ok().unwrap();

        let out = ok(store.inspect(&names, true, usize::MAX));
        assert_eq!(out.matches("-- pilot/shipments.Shipment\n").count(), 1);
        assert!(out.contains("-- pilot/shipments.Shipment (above)"));
        assert!(!out.contains("Trimmed") && !out.contains("Not shown"));

        let small = ok(store.inspect(&names, true, 3_000));
        assert!(small.contains("Trimmed to fit the size limit"));
        assert!(small.contains("Not shown, to fit the size limit: `solace inspect "));
        assert!(small.len() < out.len());
        for n in names {
            assert!(
                small.contains(&format!("// ── pilot/{n} ")) || small.lines().last().unwrap().contains(&format!("pilot/{n}")),
                "{n} lost"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_creates_settings_without_overwriting() {
        for existing in [None, Some("# custom\nresources = [\"docs/**\"]\n")] {
            let root = env::temp_dir().join(new_id(Kind::Fn, "import", "settings"));
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("gleam.toml"), "name = \"app\"\nversion = \"1.0.0\"\n").unwrap();
            fs::write(root.join("src/app.gleam"), "pub fn main() { 1 }\n").unwrap();
            if let Some(text) = existing {
                fs::write(root.join("solace.toml"), text).unwrap();
            }
            let out = import(&root).unwrap();
            let text = fs::read_to_string(root.join("solace.toml")).unwrap();
            if let Some(existing) = existing {
                assert_eq!(text, existing);
                assert!(!out.contains("Created solace.toml"));
            } else {
                assert!(out.contains("Created solace.toml"));
                assert_eq!(lockdown::settings(&root).unwrap()["resource_limit"].as_integer(), Some(262144));
                assert!(lockdown::settings(&root).unwrap()["commands"].as_table().unwrap().is_empty());
            }
            let store = fs::read(root.join(".solace/store.db")).unwrap();
            assert!(import(&root).unwrap().contains("existing symbols were preserved"));
            assert_eq!(fs::read_to_string(root.join("solace.toml")).unwrap(), text);
            fs::remove_file(root.join("solace.toml")).unwrap();
            fs::write(root.join("src/app.gleam"), "pub fn changed_on_disk() { 2 }\n").unwrap();
            assert!(import(&root).unwrap().contains("Created solace.toml"));
            assert!(lockdown::settings(&root).is_ok());
            assert_eq!(fs::read(root.join(".solace/store.db")).unwrap(), store);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn submit_new_creates_a_module() {
        let root = env::temp_dir().join(format!("solace-new-module-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("gleam.toml"), "name = \"app\"\nversion = \"1.0.0\"\n").unwrap();
        fs::write(root.join("src/app.gleam"), "pub fn main() { 1 }\n").unwrap();
        import(&root).unwrap();
        let store = Store { db: Connection::open(root.join(".solace/store.db")).unwrap(), root: root.clone() };
        // Code with errors creates nothing.
        assert!(store.submit_symbols(None, Some("calc"), "pub fn add(a: Int) -> Int { a <> 1 }").is_err());
        assert!(!store.is_module("calc").unwrap());
        let out = store.submit_symbols(None, Some("calc"), "pub fn add(a: Int, b: Int) -> Int { a + b }").unwrap();
        assert!(out.contains("created module calc (src/calc.gleam)"), "{out}");
        assert!(store.is_module("calc").unwrap());
        let out = store.submit_symbols(None, Some("calc_test"), "pub fn add_test() { assert calc.add(1, 2) == 3 }").unwrap();
        assert!(out.contains("(test/calc_test.gleam)"), "{out}");
        assert!(store.submit_symbols(None, Some("Bad"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E1003"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_words() {
        assert_eq!(
            words("Add the TENOFF coupon to refundsByMonth in order_totals, 10 tests"),
            ["tenoff", "coupon", "refund", "month", "order", "total"]
        );
    }
}
