//! `solace`: the Phase 1a semantic store and agent protocol over Gleam
//! (specs/01-identity.md, 02-projection.md, 03-protocol.md).
//!
//! The store (`.solace/store.db`) is the source of truth. After every change
//! it writes the Gleam files back out so `gleam` can check, test and build
//! them; agents never touch those files.

mod gleam;

use gleam::{Item, Kind, Ref};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

const REFERENCE: &str = include_str!("reference.txt");
const STDLIB: &[&str] = &[
    "bool", "bit_array", "bytes_tree", "dict", "dynamic", "float", "function", "int", "io", "list", "option",
    "order", "pair", "result", "set", "string", "string_tree", "uri",
];

/// A command's failure: its message and exit code (1 diagnostics, 2 usage).
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
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("mcp") {
        return mcp();
    }
    let code = match run(&args, &mut stdin) {
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

/// `input` supplies the new code for submit commands.
fn run(args: &[String], input: &mut dyn FnMut() -> Result<String, Failure>) -> Out {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(a.as_slice(), [] | ["help"] | ["--help"]) {
        return Ok(REFERENCE.to_string());
    }
    if a[0] == "import" {
        return import(&env::current_dir()?);
    }
    let store = Store::open()?;
    match a.as_slice() {
        ["session", ..] => store.session(),
        ["view", names @ ..] if names.iter().any(|n| !n.starts_with("--")) => {
            store.view(&names.iter().copied().filter(|n| !n.starts_with("--")).collect::<Vec<_>>())
        }
        ["inspect", name, rest @ ..] => store.inspect(name, rest.contains(&"--context")),
        ["refs", name] => store.refs(name),
        ["find", query @ ..] if !query.is_empty() => store.find(&query.join(" ")),
        ["submit", "--new", module] => store.submit(Target::New(module), &input()?),
        ["submit", "--imports", module] => store.submit(Target::Imports(module), &input()?),
        ["submit", name] if !name.contains('.') && store.is_module(name)? => store.submit(Target::Module(name), &input()?),
        ["submit", name] => store.submit(Target::Existing(name), &input()?),
        ["delete", name] => store.delete(name),
        ["check"] => store.check(),
        ["test"] => store.test(),
        ["work", "add", text @ ..] if !text.is_empty() => store.work_add(&text.join(" ")),
        ["work", "list"] | ["work"] => store.work_list(),
        ["work", "done", id] => store.work_done(id),
        _ => usage(format!("Unknown command: solace {}", a.join(" "))),
    }
}

fn stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    if text.trim().is_empty() {
        return usage("Expected the new code on stdin, e.g.  solace submit checkout.pay <<'EOF' ... EOF");
    }
    Ok(text)
}

/// `solace mcp`: the same commands as one MCP tool over stdio, so an agent
/// can pass code without shell quoting and needs no shell at all.
fn mcp() {
    use serde_json::{Value, json};
    let description = "Run a Solace command against this project's semantic store. `command` is what \
        follows `solace` on a command line, e.g. `session start`, `inspect checkout.pay --context`, \
        `submit checkout.pay`. For submit commands, put the new code in `text`. Start with \
        `session start`: it lists the modules and every command.";
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
                        "text": {"type": "string", "description": "New code, for submit commands"}
                    },
                    "required": ["command"]
                }
            }]}),
            "tools/call" => {
                let args = &msg["params"]["arguments"];
                let command = args["command"].as_str().unwrap_or("");
                let command = command.trim().strip_prefix("solace").map_or(command, |c| c.trim_start());
                let words: Vec<String> = command.split_whitespace().map(String::from).collect();
                let text = args["text"].as_str().map(String::from);
                let mut input = || match &text {
                    Some(t) if !t.trim().is_empty() => Ok(t.clone()),
                    _ => usage("This command needs the new code in the `text` argument."),
                };
                let (out, is_error) = match run(&words, &mut input) {
                    Ok(out) => (out, false),
                    Err(Failure(msg, _)) => (msg, true),
                };
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
    Existing(&'a str),
    New(&'a str),
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
    let dir = root.join(".solace");
    if dir.join("store.db").exists() {
        return fail("This project already has a store (.solace/store.db).");
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
    Ok(format!("Imported {modules} modules, {symbols} symbols into .solace/store.db\n"))
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
        let imports: HashMap<&str, Vec<gleam::Import>> =
            modules.iter().map(|m| (m.path.as_str(), gleam::imports(&m.header))).collect();
        let uses = syms
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let imps = &imports[s.module.as_str()];
                let mut found = vec![];
                for r in gleam::references(&s.text) {
                    let target = match &r {
                        Ref::Qualified(alias, member) => imps
                            .iter()
                            .find(|imp| &imp.alias == alias)
                            .and_then(|imp| names.get(&(imp.module.as_str(), member.as_str()))),
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
        let out = Command::new("gleam").args(args).current_dir(&self.root).output()?;
        let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        Ok((out.status.success(), strip_ansi(&text)))
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
                let list: Vec<String> =
                    many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind.as_str())).collect();
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

    fn session(&self) -> Out {
        let ix = self.index()?;
        let mut out = String::from("Modules (symbols, description):\n");
        for m in &ix.modules {
            let count = ix.syms.iter().filter(|s| s.module == m.path).count();
            let doc = m.header.lines().find(|l| l.starts_with("////")).map_or("", |l| l.trim_start_matches('/').trim());
            out += &format!("  {:<4} {:<28} {:>3}  {}\n", m.dir, m.path, count, doc);
        }
        out += &self.status()?;
        out += "\n";
        out += REFERENCE;
        Ok(out)
    }

    /// Open work records and broken symbols.
    fn status(&self) -> Result<String, Failure> {
        let mut out = String::new();
        let work: Vec<(i64, String)> =
            self.db.prepare("SELECT id, text FROM work WHERE open = 1")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        out += &format!("\nOpen work records: {}\n", if work.is_empty() { "none" } else { "" });
        for (id, text) in work {
            out += &format!("  #{id} {text}\n");
        }
        let broken: Vec<String> = self.db.prepare("SELECT symbol FROM broken ORDER BY symbol")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        out += &format!("Broken symbols: {}\n", if broken.is_empty() { "none".to_string() } else { broken.join(", ") });
        Ok(out)
    }

    fn view(&self, names: &[&str]) -> Out {
        let ix = self.index()?;
        let mut out = String::new();
        for name in names {
            if let Some(m) = self.resolve_module(&ix, name) {
                out += &format!("// ── module {} ({}) ──\n", m.path, m.dir);
                if !m.header.is_empty() {
                    out += &format!("{}\n\n", m.header);
                }
                for s in ix.syms.iter().filter(|s| s.module == m.path) {
                    out += &format!("{}\n\n", s.text);
                }
            } else {
                let s = &ix.syms[self.resolve(&ix, name)?];
                out += &format!("// ── {} ({}) ──\n{}\n\n", s.qname(), s.kind.as_str(), s.text);
            }
        }
        Ok(out)
    }

    fn inspect(&self, name: &str, context: bool) -> Out {
        let ix = self.index()?;
        let i = self.resolve(&ix, name)?;
        let s = &ix.syms[i];
        let mut out = format!("// ── {} ({}) ──\n{}\n", s.qname(), s.kind.as_str(), s.text);
        if !context {
            return Ok(out);
        }
        let used_by: Vec<usize> = (0..ix.syms.len()).filter(|&j| ix.uses[j].contains(&i)).collect();
        let (tests, callers): (Vec<usize>, Vec<usize>) = used_by.into_iter().partition(|&j| is_test(&ix, j));
        if !ix.uses[i].is_empty() {
            out += "\n== Uses ==\n";
            let mut uses = ix.uses[i].clone();
            uses.sort_by_key(|&u| (ix.syms[u].kind != Kind::Type, ix.syms[u].kind != Kind::Const, ix.syms[u].qname()));
            for u in uses {
                out += &format!("-- {}\n{}\n", ix.syms[u].qname(), gleam::signature(&ix.syms[u].item()));
            }
        }
        if !callers.is_empty() {
            out += &format!("\n== Used by ({}) ==\n", callers.len());
            for &c in callers.iter().take(20) {
                out += &format!("-- {}\n{}\n", ix.syms[c].qname(), gleam::signature(&ix.syms[c].item()));
            }
            if callers.len() > 20 {
                out += &format!("…and {} more (solace refs {})\n", callers.len() - 20, s.qname());
            }
        }
        if !tests.is_empty() {
            let names: Vec<String> = tests.iter().map(|&t| ix.syms[t].qname()).collect();
            out += &format!("\n== Tests that mention it ({}) ==\n  {}\n", tests.len(), names.join("\n  "));
        }
        if let Some(diag) = self.db.query_row("SELECT diagnostic FROM broken WHERE symbol = ?1", [s.qname()], |r| r.get::<_, String>(0)).optional()? {
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
            Target::Imports(m) => (self.resolve_module(&ix, m).ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))?.path.clone(), vec![]),
            Target::New(m) | Target::Module(m) => {
                (self.resolve_module(&ix, m).ok_or_else(|| Failure(format!("E1003 unknown module: {m}"), 1))?.path.clone(), vec![])
            }
            Target::Existing(name) => (ix.syms[self.resolve(&ix, name)?].module.clone(), vec![]),
        };
        let header = &ix.modules.iter().find(|m| m.path == module).unwrap().header;
        let ix_before = &ix;

        self.db.execute_batch("BEGIN")?;
        let result = (|| -> Result<(Option<String>, String), Failure> {
            let target_name = match target {
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
                                let ord: i64 = self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&module], |r| r.get(0))?;
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
                Target::New(_) | Target::Existing(_) => {
                    let [item] = parsed.items.as_slice() else {
                        return fail(format!(
                            "E0101 submit one symbol at a time: the text defines {} ({}).",
                            parsed.items.len(),
                            parsed.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
                        ));
                    };
                    let existing = match target {
                        Target::Existing(name) => Some(&ix.syms[self.resolve(&ix, name)?]),
                        _ => None,
                    };
                    let clash = ix.syms.iter().find(|s| {
                        s.module == module
                            && s.name == item.name
                            && (s.kind == Kind::Type) == (item.kind == Kind::Type)
                            && existing.is_none_or(|e| e.id != s.id)
                    });
                    if let Some(c) = clash {
                        return fail(format!("E1004 {} already exists; submit to it instead.", c.qname()));
                    }
                    let mut header = header.clone();
                    for line in parsed.header.lines().filter(|l| l.starts_with("import ")) {
                        if !header.lines().any(|h| h == line) {
                            header = add_import(&header, line);
                            notes.push(format!("added `{line}` to {module}"));
                        }
                    }
                    for path in missing_imports(&ix, &header, &item.text) {
                        header = add_import(&header, &format!("import {path}"));
                        notes.push(format!("added `import {path}` to {module}"));
                    }
                    self.set_header(&module, &header)?;
                    match existing {
                        Some(e) => {
                            self.db.execute(
                                "UPDATE symbols SET name = ?2, kind = ?3, text = ?4, revision = ?5 WHERE id = ?1",
                                params![e.id, item.name, item.kind.as_str(), item.text, revision(&item.text)],
                            )?;
                            self.history(&e.id, "edit", &item.name)?;
                        }
                        None => {
                            let id = new_id(item.kind, &module, &item.name);
                            let ord: i64 = self.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&module], |r| r.get(0))?;
                            self.db.execute(
                                "INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                params![id, module, item.name, item.kind.as_str(), item.text, ord, revision(&item.text)],
                            )?;
                            self.history(&id, "create", &item.name)?;
                        }
                    }
                    Some(format!("{module}.{}", item.name))
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
            // The compiler stops at the first module with errors, so list
            // every user of a changed signature from the reference graph.
            if let (Target::Existing(name), Some(q)) = (&target, &target_name) {
                let before = &ix_before.syms[self.resolve(&ix_before, name)?];
                let after_ix = self.index()?;
                if let Some(a) = after_ix.syms.iter().position(|s| s.qname() == *q)
                    && gleam::signature(&before.item()) != gleam::signature(&after_ix.syms[a].item())
                {
                    let broken: HashSet<&str> = diags.iter().filter_map(|d| d.symbol.as_deref()).collect();
                    let unchecked: Vec<String> = (0..after_ix.syms.len())
                        .filter(|&j| after_ix.uses[j].contains(&a))
                        .map(|j| after_ix.syms[j].qname())
                        .filter(|n| !broken.contains(n.as_str()))
                        .collect();
                    if !unchecked.is_empty() && diags.iter().any(|d| d.error) {
                        out += &format!(
                            "\nThe signature changed. These other users of it are not checked until the errors above are fixed:\n  {}\n",
                            unchecked.join("\n  ")
                        );
                    }
                }
            }
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

    fn set_header(&self, module: &str, header: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE modules SET header = ?2 WHERE path = ?1", params![module, header])?;
        Ok(())
    }

    fn history(&self, id: &str, action: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute("INSERT INTO history(at, symbol, action, detail) VALUES (?1, ?2, ?3, ?4)", params![now() as i64, id, action, detail])?;
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

fn is_test(ix: &Index, i: usize) -> bool {
    ix.modules.iter().any(|m| m.path == ix.syms[i].module && m.dir == "test")
}

/// Formats Gleam source, which doubles as the syntax check.
fn format_gleam(source: &str) -> Result<String, Failure> {
    let mut child = Command::new("gleam").args(["format", "--stdin"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
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

/// Adds an import line after the existing imports (or after the module doc).
fn add_import(header: &str, line: &str) -> String {
    let lines: Vec<&str> = header.lines().collect();
    let at = lines.iter().rposition(|l| l.starts_with("import ") || l.starts_with("////")).map_or(0, |i| i + 1);
    // a multi-line import ends at its closing brace
    let at = if at > 0 && lines[at - 1].ends_with('{') { lines[at..].iter().position(|l| l.starts_with('}')).map_or(at, |p| at + p + 1) } else { at };
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
                if offset > 0 && !digits.is_empty() && t[digits.len()..].starts_with(" │")
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
    let mut names: Vec<String> = errors.iter().map(|d| d.symbol.clone().or(d.module.clone().map(|m| m + " imports")).unwrap_or("?".into())).collect();
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
