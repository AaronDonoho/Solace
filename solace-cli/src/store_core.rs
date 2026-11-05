//! The store's core: opening and peeking at the store, projection, transactions and schema setup.

use rusqlite::Connection;
use rusqlite::params;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::rc::Rc;
use std::env;
use std::fs;
use crate::language::Kind;
use crate::failure::Failure;
use crate::{Index, Store, Sym};
use crate::lockdown::commands::command;
use crate::lockdown::commands::passed;

/// Ends a command's outer transaction on every path out of `run_options`: one
/// left open by an early `?` rolls back, and the shared connection is let go,
/// so a longer-lived caller never keeps the store's write lock.
pub(crate) struct OuterTransaction(pub(crate) Rc<Connection>);

impl Drop for OuterTransaction {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
        crate::COMMAND_DB.set(vec![]);
    }
}

pub(crate) const SCHEMA: &str = "
CREATE TABLE modules(path TEXT PRIMARY KEY, dir TEXT NOT NULL, header TEXT NOT NULL, file TEXT, owns_dir INTEGER);
CREATE TABLE symbols(id TEXT PRIMARY KEY, module TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  text TEXT NOT NULL, ord INTEGER NOT NULL, revision TEXT NOT NULL, live INTEGER NOT NULL DEFAULT 1,
  parent TEXT, sep TEXT, tail TEXT NOT NULL DEFAULT '');
CREATE TABLE history(seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, symbol TEXT NOT NULL, action TEXT NOT NULL, detail TEXT NOT NULL);
CREATE TABLE broken(symbol TEXT PRIMARY KEY, diagnostic TEXT NOT NULL);
CREATE TABLE work(id INTEGER PRIMARY KEY, text TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1);
";

/// Tables added after `SCHEMA`, made when a store opens so older stores have them too.
pub(crate) const LATER_TABLES: &str = "
-- per-test results and the source as of the last full pass (D-44)
CREATE TABLE IF NOT EXISTS test_runs(test TEXT PRIMARY KEY, reach TEXT NOT NULL, result TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS test_base(id TEXT PRIMARY KEY, text TEXT NOT NULL);
-- rejected submissions, kept to retry (D-46)
CREATE TABLE IF NOT EXISTS drafts(id TEXT PRIMARY KEY, owner TEXT NOT NULL, args TEXT NOT NULL, text TEXT NOT NULL, base TEXT NOT NULL);
-- the task board
CREATE TABLE IF NOT EXISTS board_events(seq INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS board_before(task TEXT, entity TEXT, data TEXT NOT NULL, PRIMARY KEY(task,entity));
CREATE TABLE IF NOT EXISTS board_touches(seq INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, entity TEXT, run TEXT, name TEXT);
CREATE TABLE IF NOT EXISTS board_ends(task TEXT, module TEXT NOT NULL, parent TEXT NOT NULL, last TEXT NOT NULL, sep TEXT NOT NULL, PRIMARY KEY(task,module,parent));
-- the warnings as of the last check, so a submit can name the new ones
CREATE TABLE IF NOT EXISTS warnings(key TEXT PRIMARY KEY, module TEXT NOT NULL DEFAULT 'project');
CREATE TABLE IF NOT EXISTS warning_checks(one INTEGER PRIMARY KEY CHECK (one = 1));
";

impl Store {
    /// The store of the project at `root`, with every table this version uses.
    pub(crate) fn at(root: &Path) -> Result<Store, Failure> {
        let shared = crate::COMMAND_DB
            .with_borrow(|c| c.iter().find(|(r, _)| crate::project_paths::same_project(r, root)).map(|(_, db)| db.clone()));
        if let Some(db) = shared {
            return Ok(crate::Store::new(db, root, true, true));
        }
        let db = Connection::open(root.join(".solace/store.db"))?;
        // WAL and a busy timeout: readers see the last committed state while a
        // command holds the store's write lock (D-77 point 8).
        db.busy_timeout(Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.execute_batch(LATER_TABLES)?;
        crate::Store::add_columns(&db)?;
        Ok(crate::Store::new(Rc::new(db), root, true, true))
    }
    /// The language that reads this store's code.
    pub(crate) fn lang(&self) -> &'static dyn crate::language::Language {
        crate::language::reading(self.lang)
    }
    /// Columns added after their tables: `board_touches`' `run` and `name`
    /// (D-77 point 4), nested Rust items (D-158). Older stores get them.
    fn add_columns(db: &Connection) -> Result<(), Failure> {
        for (table, col, decl) in [
            ("board_touches", "run", "TEXT"),
            ("board_touches", "name", "TEXT"),
            ("symbols", "parent", "TEXT"),
            ("symbols", "sep", "TEXT"),
            ("symbols", "tail", "TEXT NOT NULL DEFAULT ''"),
            ("modules", "file", "TEXT"),
            ("modules", "owns_dir", "INTEGER"),
            ("warnings", "module", "TEXT NOT NULL DEFAULT 'project'"),
        ] {
            // A store of the root's board alone has no `symbols` or `modules`.
            let exists: bool =
                db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| r.get(0))?;
            if exists && !has_column(db, table, col)? {
                db.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), [])?;
            }
        }
        Ok(())
    }
    /// Another project's store, only to read: a plain connection, which
    /// creates no tables and takes no lock (D-57).
    pub(crate) fn peek(root: &Path) -> Result<Store, Failure> {
        let file = root.join(".solace/store.db");
        if !file.exists() {
            return crate::failure::fail(format!("no Solace store in {}", root.display()));
        }
        let db = Connection::open(file)?;
        db.busy_timeout(Duration::from_secs(10))?;
        // An older store may lack the nested-item columns; the checks happen here, once
        // (D-158), so `index` reads its rows without touching `pragma_table_info`.
        let nested = has_column(&db, "symbols", "parent")? && has_column(&db, "modules", "file")?;
        let owns_dir = has_column(&db, "modules", "owns_dir")?;
        Ok(crate::Store::new(Rc::new(db), root, nested, owns_dir))
    }
    /// The project the working folder is in: the nearest folder up with a store.
    pub(crate) fn root() -> Result<PathBuf, Failure> {
        #[cfg(test)]
        if let Some(root) = crate::TEST_ROOT.with(|r| r.borrow().clone()) {
            return Ok(root);
        }
        let mut dir = env::current_dir()?;
        loop {
            if dir.join(".solace/store.db").exists() {
                return Ok(dir);
            }
            if !dir.pop() {
                return crate::failure::fail(
                    "No Solace store found (.solace/store.db). Bring existing code in by migrating it: \
                     run `solace serve` here and choose Migrate this project.",
                );
            }
        }
    }
    pub(crate) fn index(&self) -> Result<Index, Failure> {
        // A store only peeked at may be older than its nested-item columns (checked once in `peek`).
        let nested = if self.nested { "parent, sep, tail" } else { "NULL, NULL, ''" };
        let mut stmt =
            self.db.prepare(&format!("SELECT id, module, name, kind, text, {nested} FROM symbols WHERE live = 1 ORDER BY module, ord"))?;
        let syms: Vec<Sym> = stmt
            .query_map([], |r| {
                let kind: String = r.get(3)?;
                Ok(Sym {
                    id: r.get(0)?,
                    module: r.get(1)?,
                    name: r.get(2)?,
                    kind: Kind::parse(&kind),
                    kind_text: kind,
                    text: r.get(4)?,
                    parent: r.get(5)?,
                    sep: r.get(6)?,
                    tail: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let file = if self.nested { "file" } else { "NULL" };
        let owns = if self.owns_dir { "owns_dir" } else { "NULL" };
        let mut stmt = self.db.prepare(&format!("SELECT path, dir, header, {file}, {owns} FROM modules ORDER BY dir, path"))?;
        let modules: Vec<crate::ModuleRow> = stmt
            .query_map([], |r| {
                Ok(crate::ModuleRow { path: r.get(0)?, dir: r.get(1)?, header: r.get(2)?, file: r.get(3)?, owns_dir: r.get(4)? })
            })?
            .collect::<Result<_, _>>()?;

        Ok(crate::index::build(self.lang(), syms, modules, crate::language::is_rust(&self.root)))
    }
    // --- writing files ------------------------------------------------------------
    /// Writes every module's file from the store. Returns, per file, the line
    /// range of each symbol, for mapping compiler output back to symbols.
    pub(crate) fn materialize(&self, ix: &Index) -> Result<Vec<crate::diagnostics::FileMap>, Failure> {
        self.materialize_to(ix, &self.root, &HashMap::new())
    }
    /// `stops`: where an undo stops removing the folders a deleted module file
    /// leaves empty (D-136; `Written::prune`).
    pub(crate) fn materialize_to(
        &self,
        ix: &Index,
        target: &Path,
        stops: &HashMap<String, String>,
    ) -> Result<Vec<crate::diagnostics::FileMap>, Failure> {
        // A file changed outside Solace is never written over (D-58); the next command reads it in.
        let mut written = (target == self.root).then(|| crate::outside::Written::load(&self.root));
        let mut maps = vec![];
        for m in &ix.modules {
            let (content, ranges) = crate::store::projection(ix, m);
            let rel = m.rel();
            let path = target.join(&rel);
            if fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                if written.as_ref().is_some_and(|w| !w.unchanged(&rel)) {
                    maps.push(crate::diagnostics::FileMap { module: m.path.clone(), file: path, ranges });
                    continue;
                }
                fs::create_dir_all(path.parent().unwrap())?;
                fs::write(&path, &content)?;
            }
            if let Some(w) = written.as_mut() {
                w.record(&rel, content.as_bytes());
            }
            maps.push(crate::diagnostics::FileMap { module: m.path.clone(), file: path, ranges });
        }
        if let Some(w) = written.as_mut() {
            // A file whose module is gone goes with it, unless edited since.
            w.prune(&ix.modules.iter().map(|m| m.rel()).collect(), stops)?;
            w.save()?;
        }
        Ok(maps)
    }
    /// Solace's own short-lived rewrite of a module file (tests made private,
    /// traced functions), put back by `materialize` afterwards.
    pub(crate) fn rewrite(&self, path: &Path, content: &str) -> Result<(), Failure> {
        fs::write(path, content)?;
        let mut w = crate::outside::Written::load(&self.root);
        if w.known {
            w.record(&path.strip_prefix(&self.root).unwrap().to_string_lossy(), content.as_bytes());
            w.save()?;
        }
        Ok(())
    }
    /// Runs the language's tool (`gleam check`, `gleam test`) in the project.
    pub(crate) fn tool(&self, args: &[&str]) -> Result<(bool, String), Failure> {
        if let Some(id) = crate::language::id(&self.root)
            && let Some(why) = crate::language::reason(id)?
        {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let program = self.tool_program();
        let mut argv = vec![program.to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|s| s.to_string()));
        // whole, so every failure is parsed; the reply is trimmed later
        let text = command(&self.root, &argv, 120, usize::MAX, false, false)?;
        Ok((passed(&text), text.split_once('\n').map_or("", |(_, s)| s).to_string()))
    }
    /// Runs `cargo` in the project (D-158): the exit code and its output.
    pub(crate) fn cargo(&self, args: &[&str], timeout: u64) -> Result<(bool, String), Failure> {
        if let Some(why) = crate::language::reason(crate::language::RUST)? {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let mut argv = vec!["cargo".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let text = command(&self.root, &argv, timeout, usize::MAX, false, true)?;
        Ok((passed(&text), text.split_once('\n').map_or("", |(_, s)| s).to_string()))
    }
    /// Type-checks the project and records which symbols are broken, with
    /// their errors, so the next reply can report only what changed (D-46).
    pub(crate) fn check_all(&self, ix: &Index) -> Result<Vec<crate::diagnostics::Diag>, Failure> {
        let diags = self.diagnose(ix)?;
        self.record_diagnostics(&diags)?;
        Ok(diags)
    }
    /// The broken symbols as last checked, each with its errors.
    pub(crate) fn broken(&self) -> Result<HashMap<String, String>, Failure> {
        Ok(self
            .db
            .prepare("SELECT symbol, diagnostic FROM broken")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }
    pub(crate) fn resolve(&self, ix: &Index, input: &str) -> Result<usize, Failure> {
        let mut found: Vec<usize> = crate::store::matching(ix, input);
        // In a Rust store a type and its `impl` share a name; the item wins (D-158).
        if ix.rust {
            let named: Vec<usize> = found.iter().copied().filter(|&i| ix.syms[i].kind_text != "impl").collect();
            if !named.is_empty() {
                found = named;
            }
        }
        match found.as_slice() {
            [i] => Ok(*i),
            [] => {
                let hint = match input.strip_prefix("module:") {
                    Some(name) if !name.is_empty() => {
                        format!("; `module:` names a module, not a symbol: use `view {name}`")
                    }
                    _ => crate::store::similar_hint(ix, input),
                };
                crate::failure::fail(format!("E1001 unknown name: {input}{hint}"))
            }
            many => {
                let list: Vec<String> = many.iter().map(|&i| format!("  {} ({})", ix.syms[i].qname(), ix.syms[i].kind_text)).collect();
                crate::failure::fail(format!("E1002 ambiguous name: {input} could be\n{}", list.join("\n")))
            }
        }
    }
    /// A dependency's module (`lustre/component`) or public symbol
    /// (`lustre/component.register`), read from `build/packages/` and never
    /// stored (D-15): a module gives its public signatures, a symbol its doc
    /// and declaration. Bodies stay out; agents use the API, not the source.
    pub(crate) fn dependency(&self, input: &str) -> Option<String> {
        let source = |module: &str| -> Option<(String, String)> {
            let module = module.replace('.', "/");
            let packages = fs::read_dir(self.root.join("build/packages")).ok()?;
            packages.flatten().find_map(|p| {
                let text = fs::read_to_string(p.path().join("src").join(format!("{module}.gleam"))).ok()?;
                Some((format!("{} ({}, a dependency; read only)", module, p.file_name().to_string_lossy()), text))
            })
        };
        let declaration = |item: &crate::language::Item| {
            let sig = self.lang().signature(&crate::language::Item { text: crate::without_comment_lines(&item.text), ..item.clone() }).trim_end().to_string();
            // An opaque type's constructors aren't part of its API.
            if sig.contains("pub opaque type ") { sig.split(" {").next().unwrap_or(&sig).to_string() } else { sig }
        };
        let public = |item: &crate::language::Item| item.text.lines().any(|l| l.starts_with("pub "));
        if let Some((title, text)) = source(input) {
            let m = self.lang().split(&text);
            let mut out = format!("// ── module {title}, public API: `view {input}.<name>` for one with its docs ──\n");
            // The module doc's first paragraph; the rest is guides and links.
            let doc: Vec<&str> = m.header.lines().filter(|l| l.starts_with("////")).take_while(|l| l.trim() != "////").collect();
            if !doc.is_empty() {
                out += &format!("{}\n\n", doc.join("\n"));
            }
            for item in m.items.iter().filter(|i| public(i)) {
                let doc = item.text.lines().find(|l| l.starts_with("///")).map(|l| format!("{l}\n")).unwrap_or_default();
                out += &format!("{doc}{}\n\n", declaration(item));
            }
            return Some(out);
        }
        let (module, name) = input.rsplit_once('.')?;
        let (title, text) = source(module)?;
        let item = self.lang().split(&text).items.into_iter().find(|i| i.name == name && public(i))?;
        let doc: String = item.text.lines().filter(|l| l.starts_with("///")).map(|l| format!("{l}\n")).collect();
        Some(format!("// ── {name} in {title} ({}) ──\n{doc}{}\n\n", item.kind.as_str(), declaration(&item)))
    }
    pub(crate) fn resolve_module<'a>(&self, ix: &'a Index, input: &str) -> Option<&'a crate::ModuleRow> {
        crate::store::module_named(ix, input)
    }
    pub(crate) fn open_work(&self) -> Result<Vec<(i64, String)>, Failure> {
        let mut stmt = self.db.prepare("SELECT id, text FROM work WHERE open = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }
    /// Runs `body` in one store transaction: committed, or rolled back with
    /// `created` files removed and the project files written again from the store.
    pub(crate) fn transaction<T>(&self, created: &[PathBuf], body: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
        // A savepoint, so it nests inside a command's outer transaction (D-77 point 7).
        self.db.execute_batch("SAVEPOINT solace_tx")?;
        match body() {
            Ok(out) => {
                self.db.execute_batch("RELEASE solace_tx")?;
                Ok(out)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK TO solace_tx; RELEASE solace_tx")?;
                for path in created.iter().filter(|p| p.exists()) {
                    fs::remove_file(path)?;
                }
                self.materialize(&self.index()?)?;
                Err(e)
            }
        }
    }
    pub(crate) fn header(&self, module: &str) -> Result<String, Failure> {
        Ok(self.db.query_row("SELECT header FROM modules WHERE path = ?1", [module], |r| r.get(0))?)
    }
    pub(crate) fn set_header(&self, module: &str, header: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE modules SET header = ?2 WHERE path = ?1", params![module, header])?;
        Ok(())
    }
    /// Takes a symbol out of the project; its history stays.
    pub(crate) fn retire(&self, id: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [id])?;
        self.history(id, "retire", detail)
    }
    pub(crate) fn history(&self, id: &str, action: &str, detail: &str) -> Result<(), Failure> {
        self.db.execute(
            "INSERT INTO history(at, symbol, action, detail) VALUES (?1, ?2, ?3, ?4)",
            params![crate::now() as i64, id, action, detail],
        )?;
        Ok(())
    }
    /// The first Rust module file that no longer matches the store: its
    /// differing symbols, and the one command that repairs it.
    fn drift_error(&self, ix: &Index) -> Result<Option<Failure>, Failure> {
        if !crate::language::is_rust(&self.root) {
            return Ok(None);
        }
        for m in ix.modules.iter().filter(|m| m.file.is_some()) {
            let rel = m.rel();
            let Ok(disk) = fs::read_to_string(self.root.join(&rel)) else { continue };
            let names = crate::store::drift(ix, m, &disk);
            if !names.is_empty() {
                return Ok(Some(Failure(
                    format!(
                        "E0104 {rel} differs from the store in: {}. Nothing was saved. Run `format {}` to replace it \
                         with the store's text; the replaced file is kept under `.solace/outside/`.",
                        names.join(", "),
                        m.path
                    ),
                    1,
                )));
            }
        }
        Ok(None)
    }
    /// Accepts a module file's bytes as what Solace last wrote, so the next
    /// projection writes over a file left behind by an interrupted change.
    pub(crate) fn record_module_file(&self, m: &crate::ModuleRow, bytes: &[u8]) -> Result<(), Failure> {
        let rel = m.rel();
        let mut written = crate::outside::Written::load(&self.root);
        written.record(&rel, bytes);
        written.save()
    }
    /// Refuses while a module file differs from the store.
    pub(crate) fn refuse_drift(&self, ix: &Index) -> Result<(), Failure> {
        match self.drift_error(ix)? {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    /// The warnings of the last check; `None` before a check has ever run.
    pub(crate) fn warning_baseline(&self) -> Result<Option<HashSet<String>>, Failure> {
        let checked: i64 = self.db.query_row("SELECT COUNT(*) FROM warning_checks", [], |r| r.get(0))?;
        if checked == 0 {
            return Ok(None);
        }
        Ok(Some(self.db.prepare("SELECT key FROM warnings")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?))
    }
    /// Keeps this check's warnings, so a submit can name only the ones it adds.
    pub(crate) fn record_warnings(&self, diags: &[crate::diagnostics::Diag]) -> Result<(), Failure> {
        let warnings: Vec<&crate::diagnostics::Diag> = diags.iter().filter(|d| !d.error).collect();
        let modules: HashSet<&str> = warnings.iter().map(|d| crate::diagnostics::warning_module(d)).collect();
        for module in modules {
            self.db.execute("DELETE FROM warnings WHERE module = ?1", params![module])?;
        }
        for d in warnings {
            self.db.execute(
                "INSERT OR IGNORE INTO warnings(key, module) VALUES (?1, ?2)",
                params![crate::diagnostics::warning_key(d), crate::diagnostics::warning_module(d)],
            )?;
        }
        self.db.execute("INSERT OR IGNORE INTO warning_checks(one) VALUES (1)", [])?;
        Ok(())
    }
    /// A store whose shared fields are set the same way by every constructor.
    pub(crate) fn new(db: Rc<Connection>, root: &Path, nested: bool, owns_dir: bool) -> Store {
        crate::Store { db, root: root.into(), lang: crate::language::detect(root), nested, owns_dir, deps: std::cell::RefCell::new(None) }
    }
    /// Drops warnings for source a later cleanup removed.
    pub(crate) fn forget_warnings(&self, diags: &[&crate::diagnostics::Diag]) -> Result<(), Failure> {
        for d in diags {
            self.db.execute(
                "DELETE FROM warnings WHERE key = ?1 AND module = ?2",
                params![crate::diagnostics::warning_key(d), crate::diagnostics::warning_module(d)],
            )?;
        }
        Ok(())
    }

    pub(crate) fn set_symbol_text(&self, id: &str, name: &str, text: &str) -> Result<(), Failure> {
        self.db.execute("UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1", params![id, text, crate::revision(text)])?;
        self.history(id, "edit", name)
    }

    /// The module whose path is exactly `input`: creating a module must not
    /// land on one that only matches by suffix.
    pub(crate) fn resolve_module_exact<'a>(&self, ix: &'a Index, input: &str) -> Option<&'a crate::ModuleRow> {
        crate::store::module_exact(ix, input)
    }

    /// Type-checks without recording the result: a save reads a dependent
    /// project it must not change, and compares against that project's stored
    /// broken set.
    pub(crate) fn diagnose(&self, ix: &Index) -> Result<Vec<crate::diagnostics::Diag>, Failure> {
        if let Some(id) = crate::language::id(&self.root)
            && let Some(why) = crate::language::reason(id)?
        {
            return Err(Failure(format!("E6013 {why}"), 1));
        }
        let files = self.materialize(ix)?;
        self.refuse_drift(ix)?;
        let diags = if crate::language::is_rust(&self.root) {
            let args = crate::language::cargo::check_args(&self.root);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let (passed, output) = self.cargo(&argv, 120)?;
            let mut diags = crate::language::cargo::diagnostics_for(&self.root, &output, &files);
            // A build script that fails this way (no network, a refused
            // write) is cargo's exit with no located diagnostic, so the
            // check must add it or it would report a pass (D-158 point 4).
            if !passed && !diags.iter().any(|d| d.error) {
                diags.push(crate::diagnostics::Diag {
                    error: true,
                    module: None,
                    symbol: None,
                    text: crate::language::cargo::failure_text(&output),
                    suggestions: vec![],
                });
            }
            diags
        } else {
            let (_, output) = self.tool(&["check"])?;
            self.lang().diagnostics(&output, &files)
        };
        Ok(diags)
    }

    fn record_diagnostics(&self, diags: &[crate::diagnostics::Diag]) -> Result<(), Failure> {
        self.db.execute("DELETE FROM broken", [])?;
        for d in diags.iter().filter(|d| d.error) {
            if let Some(sym) = &d.symbol {
                self.db.execute(
                    "INSERT INTO broken VALUES (?1, ?2) ON CONFLICT(symbol) DO UPDATE SET diagnostic = \
                     diagnostic || char(10, 10) || excluded.diagnostic",
                    params![sym, d.text],
                )?;
            }
        }
        self.record_warnings(diags)?;
        Ok(())
    }

    /// A save checks the projects that depend on this one. A dependent the task
    /// cannot change refuses the save; one it covers is named in the reply, so
    /// `task done` waits until a later save fixes it. The caller remembers the
    /// baseline only after the save commits.
    pub(crate) fn check_dependents(&self) -> Result<DependentCheck, Failure> {
        let top = crate::scope::top(&self.root);
        let allowed = crate::scope::of_run(&self.root).unwrap_or_default();
        let (mut refused, mut covered, mut baseline) = (vec![], vec![], vec![]);
        for (dir, new, all) in self.dependent_errors_since()? {
            if new.is_empty() {
                continue;
            }
            let in_task = crate::scope::covers(&top, &allowed, &dir);
            let shown: Vec<&str> = new.iter().take(5).map(|e| e.text.as_str()).collect();
            let more = if new.len() > 5 { format!("\n… {} more", new.len() - 5) } else { String::new() };
            let listed = format!("{}{more}", shown.join("\n\n"));
            if in_task {
                let plural = if new.len() == 1 { "" } else { "s" };
                covered
                    .push(format!("{dir} has {} new error{plural} from this change; fix them before `task done`:\n\n{listed}", new.len()));
                let existing: Vec<(String, String)> = all
                    .into_iter()
                    .filter(|old| !new.iter().any(|new| new.symbol == old.symbol && new.kind == old.kind))
                    .map(|e| (e.symbol, e.kind))
                    .collect();
                baseline.push((dir, existing));
            } else {
                refused.push(format!(
                    "{dir} is not in the task: ask the human to add {dir} to the task, or change the code that broke it.\n\n{listed}"
                ));
            }
        }
        if !refused.is_empty() {
            return Err(Failure(
                format!(
                    "E5003 a project that depends on this one has new errors, and a save checks another project only when \
                     the task covers it; nothing was saved.\n\n{}",
                    refused.join("\n\n")
                ),
                1,
            ));
        }
        let message = if covered.is_empty() {
            String::new()
        } else {
            format!("\n{}\nThe task cannot finish until these errors are fixed.\n", covered.join("\n\n"))
        };
        Ok(DependentCheck { message, baseline })
    }

    /// The new and current errors in the workspace projects that depend on this
    /// one by path. New errors are those outside the dependent's stored broken
    /// set, by symbol and kind, so another error on an already broken symbol is
    /// new. A project that cannot be read names itself in the failure.
    pub(crate) fn dependent_errors_since(&self) -> Result<Vec<(String, Vec<DependentError>, Vec<DependentError>)>, Failure> {
        let top = crate::scope::top(&self.root);
        let members = crate::workspace::members(&top).unwrap_or_default();
        let mut out = vec![];
        for root in crate::extract::deps::dependents(&top, &self.root, &members) {
            let dir = crate::scope::dir(&top, &root);
            let check = (|| -> Result<Option<(Vec<DependentError>, Vec<DependentError>)>, Failure> {
                let dep = crate::Store::at(&root)?;
                let before = dep.broken()?;
                let diags = dep.diagnose(&dep.index()?)?;
                let error = |d: &crate::diagnostics::Diag| DependentError {
                    symbol: d.symbol.clone().unwrap_or_else(|| d.text.clone()),
                    kind: crate::diagnostics::error_kind(&d.text),
                    text: match &d.symbol {
                        Some(s) => format!("{s}: {}", d.text),
                        None => d.text.clone(),
                    },
                };
                let new: Vec<DependentError> = crate::diagnostics::new_errors(&before, &diags).into_iter().map(error).collect();
                let all: Vec<DependentError> = diags.iter().filter(|d| d.error).map(error).collect();
                Ok((!new.is_empty() || !all.is_empty()).then_some((new, all)))
            })();
            match check {
                Ok(Some((new, all))) => out.push((dir, new, all)),
                Ok(None) => {}
                Err(Failure(message, code)) => return Err(Failure(format!("{dir}: {message}"), code)),
            }
        }
        Ok(out)
    }

    /// Keeps the errors a covered dependent had before this task first reported
    /// it, so a later fixing save cannot make them look like this task's errors.
    pub(crate) fn remember_dependent_errors(&self, baseline: &[(String, Vec<(String, String)>)]) -> Result<(), Failure> {
        if baseline.is_empty() {
            return Ok(());
        }
        let Some((tid, _)) = crate::board::runs::context() else { return Ok(()) };
        let baseline = baseline.to_vec();
        let root = self.root.clone();
        crate::tasks::update(&root, &tid, move |t| {
            if !t["dependent_errors"].is_object() {
                t["dependent_errors"] = serde_json::json!({});
            }
            let saved = t["dependent_errors"].as_object_mut().unwrap();
            for (dir, errors) in baseline {
                saved.entry(dir).or_insert_with(|| serde_json::json!(errors));
            }
            Ok(())
        })
    }

    /// The language tool's program.
    pub(crate) fn tool_program(&self) -> PathBuf {
        let tool = self.lang().tool();
        crate::language::tool_path(tool).unwrap_or_else(|| PathBuf::from(tool))
    }

}

/// Whether `table` has the column `col`.
fn has_column(db: &Connection, table: &str, col: &str) -> Result<bool, Failure> {
    Ok(db.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"), params![col], |r| r.get(0))?)
}

/// An error in a dependent project, as the save and `task done` compare it.
pub(crate) struct DependentError {
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) text: String,
}

/// A save's dependent check: what to tell the agent, and the errors to remember
/// on the task once the save commits.
pub(crate) struct DependentCheck {
    pub(crate) message: String,
    pub(crate) baseline: Vec<(String, Vec<(String, String)>)>,
}

