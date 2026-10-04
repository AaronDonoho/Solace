//! Migrating an existing project (D-59, Spec 17): Solace builds and checks a
//! complete copy beside the original, `../solace-migration/`, and the copy
//! takes the original's path only when the human adopts it. The original is
//! only ever read, until adoption renames it to `<name>.pre-solace`.
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub const COPY: &str = "solace-migration";
const STATE: &str = ".solace/migration.json";
const INVENTORY: &str = ".solace/migration-inventory.json";
/// Beside the folders, never inside one that moves.
const JOURNAL: &str = ".solace-migration-swap.json";
const LOCK: &str = ".solace-migration.lock";

/// The migration's record, kept in the copy: the original's path, the
/// projects and smoke checks, the stage, the baseline and the report.
pub fn load(dir: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(dir.join(STATE)).ok()?).ok()
}
/// Writes `path` whole or not at all.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
fn save(dir: &Path, state: &Value) -> Result<(), Failure> {
    fs::create_dir_all(dir.join(".solace"))?;
    // The record names the original's files: keep it out of git (a project's own `.solace/.gitignore` does already).
    if !dir.join(".solace/.gitignore").exists() {
        fs::write(dir.join(".solace/.gitignore"), "*\n")?;
    }
    write_atomic(&dir.join(STATE), &serde_json::to_vec_pretty(state).unwrap())
}
/// The original's inventory, in a file of its own so a status read doesn't load it.
fn recorded(dir: &Path) -> Value {
    fs::read(dir.join(INVENTORY)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}
fn refuse<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "migrate", message)
}
fn named(p: &Path, e: impl std::fmt::Display) -> Failure {
    Failure(format!("{}: {e}", p.display()), 1)
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}
fn project(root: &Path, dir: &str) -> PathBuf {
    if dir == "." { root.to_path_buf() } else { root.join(dir) }
}
fn pre_solace(original: &Path) -> PathBuf {
    let mut name = original.file_name().unwrap_or_default().to_os_string();
    name.push(".pre-solace");
    original.with_file_name(name)
}
fn original_of(state: &Value) -> PathBuf {
    PathBuf::from(state["original"].as_str().unwrap_or(""))
}
fn stage(state: &Value) -> &str {
    state["stage"].as_str().unwrap_or("checked")
}

/// The folder a migration command acts on from `top`: `top` itself when it
/// holds a migration, else the copy beside it.
pub fn locate(top: &Path) -> PathBuf {
    if load(top).is_some() { top.to_path_buf() } else { top.with_file_name(COPY) }
}

/// One migration operation at a time, from the board or a shell.
fn exclusive(parent: &Path) -> Result<fs::File, Failure> {
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(parent.join(LOCK))?;
    if f.try_lock().is_err() {
        return refuse("E7003", "another migration step is running; wait for it to finish");
    }
    Ok(f)
}
/// Every current project's lock in `top`, refused while an agent works in one.
fn hold(top: &Path) -> Result<Vec<fs::File>, Failure> {
    let mut held = vec![];
    for d in workspace::members(top)? {
        let root = project(top, &d);
        if !root.join(".solace/store.db").exists() {
            continue;
        }
        if root.join(".solace/board").exists() && board::tasks(&root)?.iter().any(board::active) {
            return refuse("E7003", format!("an agent is working in {d}; let it finish or stop it first"));
        }
        let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join(".solace/lock"))?;
        if f.try_lock().is_err() {
            return refuse("E7003", format!("{d} is busy with another command; try again when it finishes"));
        }
        held.push(f);
    }
    Ok(held)
}
/// Refuses a shell command while a board serves `dir` or one of its projects,
/// since the board would be left serving a folder that moved (fix 10).
fn not_served(dir: &Path) -> Result<(), Failure> {
    let mut roots = vec![dir.to_path_buf()];
    roots.extend(workspace::members(dir).unwrap_or_default().iter().map(|d| project(dir, d)));
    for r in roots {
        let Ok(f) = fs::OpenOptions::new().write(true).open(r.join(".solace/serve.lock")) else { continue };
        if f.try_lock().is_err() {
            return refuse("E7003", format!("`solace serve` is serving {}: use the board, or stop the server first", r.display()));
        }
    }
    Ok(())
}

/// A Gleam project's build output, which `gleam build` rewrites.
fn build_output(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n == "build") && p.parent().is_some_and(|d| d.join("gleam.toml").exists())
}
/// What a change in the copy is judged by: build output, `.git` and `.solace/` aside.
fn untracked(p: &Path) -> bool {
    build_output(p) || p.file_name().is_some_and(|n| n == ".git" || n == ".solace")
}

/// Every entry below `root`, relative and sorted, without following
/// symlinks; `skip` leaves an entry out (and everything below a folder it leaves out).
fn walk(root: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<Vec<(String, fs::Metadata)>, Failure> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).map_err(|e| named(&dir, e))? {
            let path = e.map_err(|e| named(&dir, e))?.path();
            if skip(&path) {
                continue;
            }
            let meta = fs::symlink_metadata(&path).map_err(|e| named(&path, e))?;
            if meta.is_dir() {
                stack.push(path.clone());
            }
            out.push((path.strip_prefix(root).unwrap().to_string_lossy().into_owned(), meta));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}
fn sha(p: &Path) -> Result<String, Failure> {
    let (mut h, mut f, mut buf) = (Sha256::new(), fs::File::open(p).map_err(|e| named(p, e))?, vec![0; 1 << 16]);
    loop {
        match f.read(&mut buf).map_err(|e| named(p, e))? {
            0 => return Ok(hex(&h.finalize())),
            n => h.update(&buf[..n]),
        }
    }
}
/// An entry's kind and permissions, a link's target, and a file's size,
/// modified time (`timed`) and SHA-256.
fn entry(p: &Path, m: &fs::Metadata, timed: bool) -> Result<String, Failure> {
    let (t, mode) = (m.file_type(), m.mode() & 0o7777);
    Ok(if t.is_symlink() {
        format!("link {}", fs::read_link(p)?.display())
    } else if t.is_dir() {
        format!("folder {mode:o}")
    } else if t.is_file() {
        let time = if timed { format!(" {}.{}", m.mtime(), m.mtime_nsec()) } else { String::new() };
        format!("file {mode:o} {}{time} {}", m.len(), sha(p)?)
    } else {
        format!("other {mode:o}")
    })
}

fn in_git(p: &Path, name: &str) -> bool {
    p.file_name().is_some_and(|n| n == name) && p.parent().and_then(|d| d.file_name()).is_some_and(|n| n == ".git")
}
/// A git index by what it records: each staged path, mode and content.
/// Any git command may rewrite the file to refresh its cached file times
/// (Aaron, 2026-10-04: that mustn't stop a migration); staging changes this.
fn git_index(p: &Path, m: &fs::Metadata) -> Result<String, Failure> {
    let out = Command::new("git").env("GIT_OPTIONAL_LOCKS", "0").arg("--git-dir").arg(p.parent().unwrap()).args(["ls-files", "--stage", "-z"]).output();
    match out {
        Ok(o) if o.status.success() => Ok(format!("git index {}", hex(&Sha256::digest(&o.stdout)))),
        _ => entry(p, m, true),
    }
}
/// Every entry of the original but its build output, which building it
/// rewrites, and git's short-lived `index.lock`.
fn inventory(root: &Path) -> Result<BTreeMap<String, String>, Failure> {
    let mut out = BTreeMap::new();
    for (rel, m) in walk(root, &|p| build_output(p) || in_git(p, "index.lock"))? {
        let p = root.join(&rel);
        let v = if in_git(&p, "index") && m.is_file() { git_index(&p, &m)? } else { entry(&p, &m, true)? };
        out.insert(rel, v);
    }
    Ok(out)
}
fn changes(original: &Path, recorded: &Value) -> Result<Vec<String>, Failure> {
    let now = inventory(original)?;
    let then: BTreeMap<String, String> = serde_json::from_value(recorded.clone()).unwrap_or_default();
    let mut out: Vec<String> = now
        .iter()
        .filter(|(k, v)| then.get(*k) != Some(v))
        .map(|(k, _)| if then.contains_key(k) { format!("changed: {k}") } else { format!("added: {k}") })
        .collect();
    out.extend(then.keys().filter(|k| !now.contains_key(*k)).map(|k| format!("removed: {k}")));
    Ok(out)
}

/// Copies what's in `from` into the folder `to`, keeping permissions;
/// symlinks stay symlinks, and sockets, FIFOs and devices aren't copied.
fn copy_tree(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<(), Failure> {
    if !to.exists() {
        fs::create_dir(to)?;
    }
    let mut dirs = vec![(to.to_path_buf(), fs::metadata(from)?.permissions())];
    for (rel, m) in walk(from, skip)? {
        let (src, dst) = (from.join(&rel), to.join(&rel));
        let t = m.file_type();
        if t.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&src)?, &dst).map_err(|e| named(&dst, e))?;
        } else if t.is_dir() {
            match fs::create_dir(&dst) {
                Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(named(&dst, e)),
                _ => dirs.push((dst, m.permissions())),
            }
        } else if t.is_file() {
            fs::copy(&src, &dst).map_err(|e| named(&src, e))?;
        }
    }
    // Last, and deepest first, so a read-only folder doesn't block copying into it.
    for (d, perm) in dirs.into_iter().rev() {
        fs::set_permissions(&d, perm).map_err(|e| named(&d, e))?;
    }
    Ok(())
}
/// Deletes `p` and everything below it, read-only folders included, without following symlinks.
fn remove_tree(p: &Path) -> std::io::Result<()> {
    let m = fs::symlink_metadata(p)?;
    if !m.is_dir() {
        return fs::remove_file(p);
    }
    if m.mode() & 0o700 != 0o700 {
        fs::set_permissions(p, fs::Permissions::from_mode(m.mode() | 0o700))?;
    }
    for e in fs::read_dir(p)? {
        remove_tree(&e?.path())?;
    }
    fs::remove_dir(p)
}
/// Runs `f` in a throwaway copy of `from` in the system's temporary folder.
fn in_temp<T>(from: &Path, skip: &dyn Fn(&Path) -> bool, f: impl FnOnce(&Path) -> T) -> Result<T, Failure> {
    let tmp = env::temp_dir().join(format!("solace-{}", new_id(Kind::Fn, "migrate", "").replace("fn:", "")));
    let made = copy_tree(from, &tmp, skip);
    let out = made.map(|_| f(&tmp));
    let _ = remove_tree(&tmp);
    out
}

/// `p` with `.` and `..` folded away, without touching the filesystem.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}
/// Whether `p`'s real location, symlinks resolved, lies inside `root`.
fn inside(root: &Path, p: &Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else { return false };
    let (mut base, mut rest) = (p.to_path_buf(), PathBuf::new());
    loop {
        if let Ok(real) = fs::canonicalize(&base) {
            return lexical(&real.join(&rest)).starts_with(&root);
        }
        let Some(name) = base.file_name().map(PathBuf::from) else { return false };
        rest = name.join(&rest);
        if !base.pop() {
            return false;
        }
    }
}
/// The path from the folder `from` to `to`, both absolute.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let (f, t): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out: PathBuf = std::iter::repeat_n("..", f.len() - common).collect();
    out.extend(&t[common..]);
    if out.as_os_str().is_empty() { PathBuf::from(".") } else { out }
}

/// Whether bubblewrap can run commands here.
fn bwrap() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("bwrap")
            .args(["--dev-bind", "/", "/", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}
/// `argv` run in `dir`, seeing `protect` read-only when bubblewrap is available.
pub fn sandboxed(argv: &[String], dir: &Path, protect: Option<&Path>) -> Command {
    let mut c = match protect.filter(|p| p.exists() && bwrap()) {
        Some(p) => {
            let mut c = Command::new("bwrap");
            c.args(["--die-with-parent", "--dev-bind", "/", "/", "--ro-bind"]).arg(p).arg(p).arg("--chdir").arg(dir).arg("--").args(argv);
            c
        }
        None => {
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]);
            c
        }
    };
    c.current_dir(dir);
    c
}
/// The folder a command in `root` must not change: the original while a
/// migration is in progress, `<name>.pre-solace` once it's adopted.
pub fn protected(root: &Path) -> Option<PathBuf> {
    let (top, state) = root.ancestors().find_map(|a| Some((a, load(a)?)))?;
    let p = if state["adopted"] == true { pre_solace(top) } else { original_of(&state) };
    p.exists().then_some(p)
}

/// Runs `argv` in `dir`: whether it exited with 0, and the end of its output.
fn run(dir: &Path, argv: &[&str], secs: u64, protect: Option<&Path>) -> (bool, String) {
    use std::os::unix::process::CommandExt;
    let log = env::temp_dir().join(format!("solace-{}.log", new_id(Kind::Fn, "run", "").replace("fn:", "")));
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let result = (|| -> Result<(bool, String), Failure> {
        let f = fs::File::create(&log)?;
        let mut child = sandboxed(&argv, dir, protect).process_group(0).stdin(Stdio::null()).stdout(f.try_clone()?).stderr(f).spawn()?;
        let start = std::time::Instant::now();
        let ok = loop {
            if let Some(s) = child.try_wait()? {
                break s.success();
            }
            if start.elapsed().as_secs() >= secs {
                let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
                let _ = child.wait();
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let text = strip_ansi(&String::from_utf8_lossy(&fs::read(&log)?));
        let cut = text.char_indices().rev().nth(20_000).map_or(0, |(i, _)| i);
        Ok((ok, text[cut..].to_string()))
    })();
    let _ = fs::remove_file(&log);
    result.unwrap_or_else(|Failure(e, _)| (false, e))
}

/// Deletes each project's compiled output, which holds absolute symlinks
/// (JavaScript's `priv`) to wherever it was built: the original, or a folder
/// that has since moved. Downloaded packages are kept.
fn clear_compiled(root: &Path, projects: &[String]) {
    for d in projects {
        let build = project(root, d).join("build");
        if inside(root, &build) {
            for out in ["dev", "prod", "lsp"] {
                let _ = remove_tree(&build.join(out));
            }
        }
    }
}

/// Build and tests of each project under `root`.
fn builds(root: &Path, projects: &[String], protect: Option<&Path>) -> Value {
    let mut ps = serde_json::Map::new();
    for d in projects {
        let dir = project(root, d);
        clear_compiled(root, std::slice::from_ref(d));
        let (build, bout) = run(&dir, &["gleam", "build"], 600, protect);
        let (test, tout) = if build { run(&dir, &["gleam", "test"], 600, protect) } else { (false, String::new()) };
        let failing: Vec<String> = failures(&tout).into_keys().collect();
        ps.insert(d.clone(), json!({"build":build,"test":test,"failing":failing,"output":bout + &tout}));
    }
    Value::Object(ps)
}
fn smokes(root: &Path, smoke: &[String], protect: Option<&Path>) -> Value {
    smoke
        .iter()
        .map(|c| {
            let (ok, output) = run(root, &["sh", "-c", c], 120, protect);
            json!({"command":c,"ok":ok,"output":output})
        })
        .collect()
}
fn suite(root: &Path, projects: &[String], smoke: &[String], protect: Option<&Path>) -> Value {
    json!({"projects":builds(root, projects, protect),"smoke":smokes(root, smoke, protect)})
}

fn item(status: &str, text: impl Into<String>, detail: &str) -> Value {
    let mut v = json!({"text":text.into()});
    if !status.is_empty() {
        v["status"] = json!(status);
    }
    if !detail.is_empty() {
        v["detail"] = json!(detail);
    }
    v
}
/// An item that could reach the original from the copy: the migration
/// stops for the user to look at these before running anything.
fn flag(mut v: Value) -> Value {
    v["flag"] = json!(true);
    v
}
/// Failed if any item failed, not checked if any wasn't, else verified.
fn check(name: &str, summary: impl Into<String>, items: Vec<Value>) -> Value {
    let has = |s: &str| items.iter().any(|i| i["status"] == s);
    let status = if has("failed") { "failed" } else if has("not checked") { "not checked" } else { "verified" };
    json!({"name":name,"status":status,"summary":summary.into(),"items":items})
}

/// One result against its baseline: a failure the original also had is
/// not blamed on the migration, but it isn't evidence either.
fn compare(before: bool, after: bool, what: &str, detail: &str) -> Value {
    match (before, after) {
        (true, true) => item("verified", format!("{what} passes"), ""),
        (true, false) => item("failed", format!("{what} fails, and passed before the migration"), detail),
        (false, false) => item("not checked", format!("{what} failed before the migration too"), detail),
        (false, true) => item("verified", format!("{what} passes, and failed before the migration"), ""),
    }
}
const UNBUILT: &str = "doesn't build, before or after the migration";
fn behaviour(base: &Value, now: &Value, smoke: bool) -> Vec<Value> {
    let mut items = vec![];
    for (d, c) in now["projects"].as_object().into_iter().flatten() {
        let b = &base["projects"][d];
        let out = c["output"].as_str().unwrap_or("");
        let (was, is) = (b["build"] == true, c["build"] == true);
        if !was && !is {
            items.push(item("not checked", format!("{d}: {UNBUILT}"), out));
            continue;
        }
        items.push(compare(was, is, &format!("{d}: build"), out));
        if !is {
            continue;
        }
        if c["test"] == true {
            items.push(item("verified", format!("{d}: tests pass"), ""));
            continue;
        }
        let failing = strings(&c["failing"]);
        if !was {
            let nothing = "the original didn't build, so there's nothing to compare with";
            items.extend(failing.iter().map(|t| item("not checked", format!("{d}: {t} fails; {nothing}"), out)));
            if failing.is_empty() {
                items.push(item("not checked", format!("{d}: gleam test fails; {nothing}"), out));
            }
            continue;
        }
        let before = strings(&b["failing"]);
        for t in &failing {
            items.push(if before.contains(t) {
                item("not checked", format!("{d}: {t} failed before the migration too"), "")
            } else {
                item("failed", format!("{d}: {t} fails, and passed before the migration"), out)
            });
        }
        if failing.is_empty() {
            items.push(compare(b["test"] == true, false, &format!("{d}: gleam test"), out));
        }
    }
    if smoke {
        let none = vec![];
        let checks = now["smoke"].as_array().unwrap_or(&none);
        if checks.is_empty() {
            items.push(item("not checked", "No smoke checks were named, so the app's behaviour isn't checked", ""));
        }
        for (i, c) in checks.iter().enumerate() {
            let what = format!("smoke check `{}`", c["command"].as_str().unwrap_or(""));
            items.push(compare(base["smoke"][i]["ok"] == true, c["ok"] == true, &what, c["output"].as_str().unwrap_or("")));
        }
    }
    items
}
/// What the report says first: projects that don't build at all.
fn warnings(base: &Value, now: &Value) -> Vec<String> {
    now["projects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(d, c)| c["build"] != true && base["projects"][d.as_str()]["build"] != true)
        .map(|(d, _)| format!("{d}: {UNBUILT}. Errors in the original carry over into the migration."))
        .collect()
}

/// The Gleam files below `dir`'s `src/` and `test/`, not following symlinked folders.
fn gleam_files(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for top in ["src", "test"] {
        let mut files = vec![];
        let _ = collect_gleam_files(&dir.join(top), &mut files);
        out.extend(files.iter().map(|f| f.strip_prefix(dir).unwrap().to_string_lossy().into_owned()));
    }
    out.sort();
    out
}

/// The layout differences between `a` and `b`, by name, for the report.
fn explain(a: &str, b: &str) -> Vec<&'static str> {
    fn ends(s: &str) -> String {
        s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n") + if s.ends_with('\n') { "\n" } else { "" }
    }
    fn blanks(s: &str) -> String {
        s.lines().filter(|l| !l.is_empty()).map(|l| format!("{l}\n")).collect()
    }
    let steps: [(&'static str, fn(&str) -> String); 4] = [
        ("line endings", |s| s.replace("\r\n", "\n")),
        ("spaces at line ends", ends),
        ("final newline", |s| s.trim_end().to_string() + "\n"),
        ("blank lines between definitions", blanks),
    ];
    let (mut a, mut b) = (a.to_string(), b.to_string());
    let mut causes = vec![];
    for (name, f) in steps {
        if a == b {
            break;
        }
        let (na, nb) = (f(&a), f(&b));
        if na != a || nb != b {
            causes.push(name);
        }
        (a, b) = (na, nb);
    }
    causes
}
/// The lines (from 0) that begin inside a string literal.
fn string_lines(text: &str) -> Vec<usize> {
    let (mut out, mut line, mut in_string, mut comment) = (vec![], 0, false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                line += 1;
                comment = false;
                if in_string {
                    out.push(line);
                }
            }
            _ if comment => {}
            '\\' if in_string => {
                if chars.next() == Some('\n') {
                    line += 1;
                    out.push(line);
                }
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => comment = true,
            _ => {}
        }
    }
    out
}
/// The pieces of the module file `rel` whose text differs from the store's:
/// Solace stores each definition verbatim, so if none differs, the file can
/// only differ between definitions, which is layout.
fn changed_pieces(db: &Connection, rel: &str, text: &str) -> Result<Vec<String>, Failure> {
    let (dir, path) = rel.split_once('/').unwrap_or(("", rel));
    let path = path.trim_end_matches(".gleam");
    let header: Option<String> =
        db.query_row("SELECT header FROM modules WHERE path = ?1 AND dir = ?2", params![path, dir], |r| r.get(0)).optional()?;
    let mut stmt = db.prepare("SELECT name, text FROM symbols WHERE module = ?1 AND live = 1 ORDER BY ord")?;
    let stored: Vec<(String, String)> = stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let split = gleam::split(text);
    let mut out = vec![];
    if header.as_deref() != Some(split.header.as_str()) {
        out.push("the module header".to_string());
    }
    for (i, it) in split.items.iter().enumerate() {
        if stored.get(i).is_none_or(|(n, t)| *n != it.name || *t != it.text) {
            out.push(it.name.clone());
        }
    }
    out.extend(stored.iter().skip(split.items.len()).map(|(n, _)| n.clone()));
    // A line inside a multi-line string that looks like a definition would be split there,
    // and the blank line put between definitions would change the string.
    let lines: Vec<&str> = text.lines().collect();
    let within = string_lines(text);
    if within.iter().any(|&i| lines.get(i).is_some_and(|l| gleam::declaration(l).is_some() || gleam::is_leading(l))) {
        out.push("a multi-line string holding a line that looks like a definition".into());
    }
    if !within.is_empty() && text.contains('\r') {
        out.push("a multi-line string with Windows line endings".into());
    }
    out.dedup();
    Ok(out)
}
fn diff(a: &Path, b: &Path, label: &str) -> String {
    let (la, lb) = (format!("original/{label}"), format!("regenerated/{label}"));
    let out = Command::new("diff").args(["-u", "--label", &la, "--label", &lb]).arg(a).arg(b).output();
    out.map_or_else(|e| format!("could not run diff: {e}"), |o| String::from_utf8_lossy(&o.stdout).into_owned())
}
fn files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}
/// Each original Gleam file against its regeneration. Judged once, right
/// after the first regeneration: later work in the copy is reviewed through tasks.
fn reconstruction(original: &Path, copy: &Path, projects: &[String]) -> Result<Value, Failure> {
    let (mut items, mut same) = (vec![], vec![]);
    for d in projects {
        let (o, c) = (project(original, d), project(copy, d));
        let db = Connection::open(c.join(".solace/store.db"))?;
        let mut files = gleam_files(&o);
        files.extend(gleam_files(&c));
        files.sort();
        files.dedup();
        for rel in files {
            let label = if d == "." { rel.clone() } else { format!("{d}/{rel}") };
            match (fs::read(o.join(&rel)), fs::read(c.join(&rel))) {
                (Ok(a), Ok(b)) if a == b => same.push(label),
                (Ok(a), Ok(b)) => {
                    let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
                    let mut changed = changed_pieces(&db, &rel, &a)?;
                    changed.extend(changed_pieces(&db, &rel, &b)?.into_iter().map(|p| format!("{p} (regenerated)")));
                    let why = explain(&a, &b).join(", ");
                    let (status, text) = if changed.is_empty() {
                        ("verified", format!("{label}: differs in layout only ({why})"))
                    } else {
                        ("failed", format!("{label}: {} differ from the original: review the diff", changed.join(", ")))
                    };
                    items.push(item(status, text, &diff(&o.join(&rel), &c.join(&rel), &label)));
                }
                (Ok(_), Err(_)) => items.push(item("failed", format!("{label}: not regenerated; the store is missing it"), "")),
                _ => items.push(item("failed", format!("{label}: regenerated, but not in the original"), "")),
            }
        }
    }
    let summary = format!("{} byte-identical, {} different; judged once, after the first regeneration", files(same.len()), items.len());
    if !same.is_empty() {
        items.insert(0, item("verified", format!("{} byte-identical", files(same.len())), &same.join("\n")));
    }
    Ok(check("Reconstruction", summary, items))
}

/// Gleam files Solace can't read in: not UTF-8, or a name defined twice.
fn unreadable(copy: &Path, projects: &[String]) -> Vec<Value> {
    let mut items = vec![];
    for d in projects {
        let root = project(copy, d);
        for rel in gleam_files(&root) {
            let label = if d == "." { rel.clone() } else { format!("{d}/{rel}") };
            match fs::read(root.join(&rel)).map(String::from_utf8) {
                Err(e) => items.push(item("failed", format!("{label}: couldn't be read ({e})"), "")),
                Ok(Err(_)) => items.push(item("failed", format!("{label}: isn't UTF-8 text, so it can't be read in"), "")),
                Ok(Ok(text)) => {
                    let mut seen = HashSet::new();
                    for it in gleam::split(&text).items {
                        if !seen.insert(it.name.clone()) {
                            items.push(item("failed", format!("{label}: {} is defined twice", it.name), ""));
                        }
                    }
                }
            }
        }
    }
    items
}

fn coverage(original: &Path, copy: &Path, projects: &[String]) -> Result<Value, Failure> {
    let mut items = vec![];
    for d in projects {
        let db = Connection::open(project(copy, d).join(".solace/store.db"))?;
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0));
        let (m, s) = (count("SELECT count(*) FROM modules")?, count("SELECT count(*) FROM symbols")?);
        items.push(item("verified", format!("{d}: {m} Gleam modules read in, {s} definitions"), ""));
        let others = walk(&project(copy, d), &|p| build_output(p) || p.file_name().is_some_and(|n| n == ".solace"))?
            .into_iter()
            .filter(|(r, m)| m.is_file() && r.ends_with(".gleam") && !r.starts_with("src/") && !r.starts_with("test/"))
            .map(|(r, _)| r)
            .collect::<Vec<_>>();
        if !others.is_empty() {
            items.push(item("", format!("{d}: {} Gleam files outside src/ and test/ are copied as files, not read in", others.len()), &others.join("\n")));
        }
    }
    let all = walk(original, &|_| false)?;
    let odd: Vec<&str> = all.iter().filter(|(_, m)| !m.is_dir() && !m.is_file() && !m.is_symlink()).map(|(r, _)| r.as_str()).collect();
    items.push(item("verified", format!("{} files, folders and links copied with their permissions, every subfolder included", all.len() - odd.len()), ""));
    if !odd.is_empty() {
        items.push(item("not checked", format!("Not copied ({}): sockets, FIFOs and devices", odd.len()), &odd.join("\n")));
    }
    for f in workspace::found(original)? {
        let dir = f["dir"].as_str().unwrap_or("");
        if !projects.iter().any(|p| p == dir) {
            items.push(item("", format!("{dir} ({}): not selected, so copied as files and not read in", f["name"].as_str().unwrap_or("")), ""));
        }
    }
    let n = projects.len();
    Ok(check("Coverage", format!("{n} project{} read in", if n == 1 { "" } else { "s" }), items))
}

/// The links in the copy that lead out of it. With `fix`, a link into the
/// original is pointed at the same place in the copy instead (listed in
/// `fixed`); without, it fails. Returns the items, and the links out of a
/// project's `src/`, `test/`, `build/` or `.solace/`, which Solace's own
/// writes would follow.
fn links(original: &Path, copy: &Path, projects: &[String], fix: bool) -> Result<(Vec<Value>, Vec<String>), Failure> {
    let guarded: Vec<PathBuf> =
        projects.iter().flat_map(|d| ["src", "test", "build", ".solace"].map(|s| project(Path::new(""), d).join(s))).collect();
    let (mut items, mut fatal) = (vec![], vec![]);
    for (rel, m) in walk(copy, &|p| p.file_name().is_some_and(|n| n == ".git"))? {
        if !m.is_symlink() {
            continue;
        }
        let link = copy.join(&rel);
        let target = fs::read_link(&link)?;
        let resolved = lexical(&link.parent().unwrap().join(&target));
        if let (Ok(rest), true) = (resolved.strip_prefix(original), fix) {
            let new = relative(link.parent().unwrap(), &copy.join(rest));
            fs::remove_file(&link)?;
            std::os::unix::fs::symlink(&new, &link)?;
            items.push(item("verified", format!("{rel} linked into the original; it now links to the same place in the copy ({})", new.display()), ""));
        } else if resolved.starts_with(original) {
            items.push(flag(item("failed", format!("{rel} links into the original, to {}", target.display()), "")));
        } else if !resolved.starts_with(copy) {
            let hit = guarded.iter().any(|g| Path::new(&rel).starts_with(g) || g.starts_with(&rel));
            if hit {
                fatal.push(rel.clone());
            }
            let status = if hit { "failed" } else { "not checked" };
            items.push(flag(item(status, format!("{rel} links outside the migrated folder, to {}", target.display()), "")));
        }
    }
    Ok((items, fatal))
}
/// Where a `.git` file, `commondir` or `alternates` entry points, if outside the copy.
fn git_pointers(copy: &Path) -> Result<Vec<(String, PathBuf)>, Failure> {
    let mut out = vec![];
    let skip = |p: &Path| build_output(p) || p.parent().is_some_and(|d| d.file_name().is_some_and(|n| n == ".git" || n == ".solace"));
    for (rel, m) in walk(copy, &skip)? {
        let p = copy.join(&rel);
        if !p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        let mut found = vec![];
        if m.is_file() {
            if let Some(dir) = fs::read_to_string(&p).ok().and_then(|t| t.trim().strip_prefix("gitdir:").map(|s| s.trim().to_string())) {
                found.push(p.parent().unwrap().join(dir));
            }
        } else if m.is_dir() {
            if let Ok(t) = fs::read_to_string(p.join("commondir")) {
                found.push(p.join(t.trim()));
            }
            for l in fs::read_to_string(p.join("objects/info/alternates")).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()) {
                found.push(p.join("objects").join(l.trim()));
            }
        }
        out.extend(found.into_iter().map(|t| lexical(&t)).filter(|t| !t.starts_with(copy)).map(|t| (rel.clone(), t)));
    }
    Ok(out)
}
fn paths(original: &Path, copy: &Path, state: &Value) -> Result<Value, Failure> {
    let projects = strings(&state["projects"]);
    let mut items: Vec<Value> = state["relinked"].as_array().cloned().unwrap_or_default();
    items.extend(links(original, copy, &projects, false)?.0);
    for (rel, m) in walk(copy, &untracked)? {
        if !(m.is_file() && rel.rsplit('/').next() == Some("gleam.toml")) {
            continue;
        }
        let dir = copy.join(&rel).parent().unwrap().to_path_buf();
        let toml: toml::Value = fs::read_to_string(copy.join(&rel))?.parse().unwrap_or(toml::Value::Table(Default::default()));
        for table in ["dependencies", "dev-dependencies", "dev_dependencies"] {
            for (name, spec) in toml.get(table).and_then(|t| t.as_table()).into_iter().flatten() {
                let Some(path) = spec.get("path").and_then(|p| p.as_str()) else { continue };
                if !inside(copy, &dir.join(path)) {
                    items.push(flag(item(
                        "failed",
                        format!("{rel}: dependency {name} at {path} is outside the migrated folder; move it into the folder first"),
                        "",
                    )));
                }
            }
        }
    }
    for (rel, t) in git_pointers(copy)? {
        let status = if t.starts_with(original) { "failed" } else { "not checked" };
        items.push(flag(item(status, format!("{rel} points git at {}, outside the migrated folder", t.display()), "")));
    }
    let needle = original.to_string_lossy().into_owned();
    let mut large = vec![];
    for (rel, m) in walk(copy, &untracked)? {
        if !m.is_file() {
            continue;
        }
        if m.len() > 262_144 {
            large.push(rel);
        } else if fs::read_to_string(copy.join(&rel)).is_ok_and(|t| t.contains(&needle)) {
            items.push(flag(item("not checked", format!("{rel} mentions the original's path; check what it does with it"), "")));
        }
    }
    if !large.is_empty() {
        items.push(item("not checked", format!("{} over 256 KiB weren't searched for the original's path", files(large.len())), &large.join("\n")));
    }
    for c in strings(&state["smoke"]).iter().filter(|c| c.contains(&needle)) {
        items.push(flag(item("not checked", format!("smoke check `{c}` names the original's path"), "")));
    }
    items.push(if bwrap() {
        item("verified", "Commands ran with the original read-only (bubblewrap)", "")
    } else {
        item("not checked", "Commands weren't isolated from the original: install bubblewrap (bwrap) for a guarantee", "")
    });
    Ok(check("Paths", "What could reach the original from inside the copy", items))
}
fn flagged(copy: &Path, state: &Value) -> Result<Vec<Value>, Failure> {
    Ok(paths(&original_of(state), copy, state)?["items"].as_array().into_iter().flatten().filter(|i| i["flag"] == true).cloned().collect())
}

fn preservation(original: &Path, recorded: &Value) -> Result<Value, Failure> {
    let ch = changes(original, recorded)?;
    let n = recorded.as_object().map_or(0, |o| o.len());
    Ok(if ch.is_empty() {
        let s = format!("The original matches its inventory: {n} entries with their permissions, untracked and ignored files included, build output excluded");
        check("Preservation", s, vec![])
    } else {
        let items = ch.iter().map(|c| item("failed", c, "")).collect();
        check("Preservation", format!("The original changed since the copy was taken: {} entries", ch.len()), items)
    })
}

fn recovery(copy: &Path, state: &Value, preserved: bool) -> Result<Value, Failure> {
    let projects = strings(&state["projects"]);
    let skip = |p: &Path| build_output(p) || p.file_name().is_some_and(|n| n == ".solace" || n == "solace.toml");
    let original = original_of(state);
    let plain = in_temp(copy, &skip, |t| suite(t, &projects, &[], Some(&original)))?;
    let mut items = behaviour(&state["baseline"], &plain, false);
    items.push(if preserved {
        item("verified", "Abandoning the copy needs no repair: the original matches its inventory", "")
    } else {
        item("failed", "The original changed since the copy was taken (see Preservation)", "")
    });
    Ok(check("Recovery", "The code built and tested with plain gleam, without .solace/ or solace.toml", items))
}

/// A summary of the copy's files, to tell when the checks are out of date:
/// by content, or (`quick`) by sizes and times, cheap enough to poll; a
/// change's time is set by any write, so a quick match means no change.
fn fingerprint(copy: &Path, quick: bool) -> Result<String, Failure> {
    let mut h = Sha256::new();
    for (rel, m) in walk(copy, &untracked)? {
        let line = if quick {
            format!("{:o} {} {}.{} {}.{}", m.mode(), m.len(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec())
        } else {
            entry(&copy.join(&rel), &m, false)?
        };
        h.update(format!("{rel} {line}\n"));
    }
    Ok(hex(&h.finalize()))
}
/// Whether the copy is as it was when the checks started.
fn current(copy: &Path, state: &Value, quick_ok: bool) -> Result<bool, Failure> {
    let f = &state["fingerprint"];
    if quick_ok && f["quick"].as_str() == Some(fingerprint(copy, true)?.as_str()) {
        return Ok(true);
    }
    Ok(f["content"].as_str() == Some(fingerprint(copy, false)?.as_str()))
}

/// Builds, tests and checks the copy against the baseline, and saves the report.
fn recheck_state(copy: &Path, state: &mut Value, step: &dyn Fn(&str)) -> Result<(), Failure> {
    let original = original_of(state);
    let protect = Some(original.as_path());
    let (projects, smoke) = (strings(&state["projects"]), strings(&state["smoke"]));
    step("Building and testing the regenerated project");
    let built = builds(copy, &projects, protect);
    // What the report vouches for: a change from here on leaves it out of date.
    let quick = fingerprint(copy, true)?;
    let before = fingerprint(copy, false)?;
    step("Running the smoke checks");
    let now = json!({"projects":built,"smoke":smokes(copy, &smoke, protect)});
    step("Checking paths and the original");
    let preserved = preservation(&original, &recorded(copy))?;
    let n = projects.len();
    let behave = check(
        "Behaviour",
        format!("Build, tests and smoke checks of {n} project{}, against the original's", if n == 1 { "" } else { "s" }),
        behaviour(&state["baseline"], &now, true),
    );
    let rebuilt = state["reconstruction"].clone();
    let mut checks = vec![coverage(&original, copy, &projects)?, rebuilt, behave, paths(&original, copy, state)?];
    step("Building the code without Solace");
    checks.push(preserved.clone());
    checks.push(recovery(copy, state, preserved["status"] == "verified")?);
    state["report"] = json!({"checked_at":now_text(),"checks":checks,"warnings":warnings(&state["baseline"], &now)});
    state["fingerprint"] = json!({"content":before,"quick":quick});
    save(copy, state)
}
fn now_text() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string()
}

/// The Gleam projects in `top` that a migration could bring in.
pub fn candidates(top: &Path) -> Result<Vec<Value>, Failure> {
    if workspace::enclosing(top)?.is_some() || load(top).is_some() {
        return Ok(vec![]);
    }
    Ok(workspace::found(top)?.into_iter().filter(|f| f["imported"] != true).map(|f| json!({"dir":f["dir"],"name":f["name"]})).collect())
}

/// Starts a migration of `top`: inventory, then the copy, with links into
/// the original pointed into the copy. If anything in the copy could reach
/// the original, it stops there for the user to look (unless `confirmed`);
/// otherwise it goes on to `proceed`. Returns the copy.
pub fn start(top: &Path, projects: &[String], smoke: &[String], confirmed: bool, step: &dyn Fn(&str)) -> Result<PathBuf, Failure> {
    let top = fs::canonicalize(top)?;
    let found = candidates(&top)?;
    if projects.is_empty() || projects.iter().any(|p| !found.iter().any(|f| f["dir"] == p.as_str())) {
        let names: Vec<&str> = found.iter().filter_map(|f| f["dir"].as_str()).collect();
        return fail(format!("Choose at least one project to migrate from: {}", names.join(", ")));
    }
    let parent = top.parent().ok_or_else(|| Failure("The folder has no parent to put the copy in.".into(), 1))?;
    let _lock = exclusive(parent)?;
    recover(parent)?;
    let copy = parent.join(COPY);
    if fs::symlink_metadata(&copy).is_ok() {
        return refuse("E7020", format!("a migration is already in progress at {}: adopt or abandon it first", copy.display()));
    }
    if fs::metadata(&top)?.dev() != fs::metadata(parent)?.dev() {
        return fail(format!("{} is on another filesystem from {}, so the copy couldn't be swapped in.", top.display(), parent.display()));
    }
    step("Recording the original's inventory");
    let inv = inventory(&top)?;
    fs::create_dir(&copy)?;
    let mut state = json!({"original":top,"projects":projects,"smoke":smoke,"adopted":false,"stage":"copying"});
    save(&copy, &state)?;
    let made = (|| {
        write_atomic(&copy.join(INVENTORY), &serde_json::to_vec(&inv).unwrap())?;
        step("Copying the whole folder to ../solace-migration");
        copy_tree(&top, &copy, &|_| false)?;
        clear_compiled(&copy, projects);
        step("Checking what in the copy could reach the original");
        let (relinked, fatal) = links(&top, &copy, projects, true)?;
        // Only the redirected links: the rest are found again by every Paths check.
        state["relinked"] = json!(relinked.into_iter().filter(|i| i["status"] == "verified").collect::<Vec<_>>());
        if !fatal.is_empty() {
            return fail(format!(
                "These links lead out of the copy from where Solace writes, so it can't go on safely:\n{}\nPoint them inside the folder in the original, then start again.",
                fatal.join("\n")
            ));
        }
        state["stage"] = json!("confirm_paths");
        save(&copy, &state)
    })();
    if let Err(e) = made {
        return stopped(&copy, &mut state, e);
    }
    if !confirmed && !flagged(&copy, &state)?.is_empty() {
        return Ok(copy);
    }
    proceed(&copy, &mut state, step)?;
    Ok(copy)
}

/// Continues a migration stopped for the user to look at what could reach the original.
pub fn confirm_paths(copy: &Path, step: &dyn Fn(&str)) -> Result<PathBuf, Failure> {
    let _lock = exclusive(copy.parent().unwrap_or(copy))?;
    let mut state = record(copy)?;
    if stage(&state) != "confirm_paths" {
        return fail("This migration isn't waiting for paths to be confirmed.");
    }
    proceed(copy, &mut state, step)?;
    Ok(copy.to_path_buf())
}

/// The baseline, Solace set up in the copy, the source regenerated from the
/// store, and the checks.
fn proceed(copy: &Path, state: &mut Value, step: &dyn Fn(&str)) -> Result<(), Failure> {
    let made = (|| {
        let original = original_of(state);
        let (projects, smoke) = (strings(&state["projects"]), strings(&state["smoke"]));
        step("Building and testing the original, in a throwaway copy");
        let baseline = in_temp(copy, &build_output, |t| suite(t, &projects, &smoke, Some(&original)))?;
        state["baseline"] = baseline;
        let bad = unreadable(copy, &projects);
        if !bad.is_empty() {
            state["report"] = json!({"checked_at":now_text(),"warnings":[],"checks":[check("Coverage", "Solace couldn't read in every Gleam file", bad)]});
            return fail("Solace couldn't read in some of the Gleam files: see Coverage.");
        }
        step("Setting up Solace in the copy");
        let mut sorted = projects.clone();
        sorted.sort();
        for d in &sorted {
            workspace::setup(copy, d)?;
        }
        step("Regenerating the source from the store");
        for d in &projects {
            let root = project(copy, d);
            for rel in gleam_files(&root) {
                let f = root.join(rel);
                if !inside(copy, f.parent().unwrap()) {
                    return fail(format!("{} isn't inside the copy, so Solace won't delete it.", f.display()));
                }
                fs::remove_file(&f)?;
            }
            let store = board::open(&root)?;
            store.materialize(&store.index()?)?;
        }
        state["reconstruction"] = reconstruction(&original, copy, &projects)?;
        state["stage"] = json!("checked");
        recheck_state(copy, state, step)
    })();
    match made {
        Err(e) => stopped(copy, state, e).map(|_| ()),
        ok => ok,
    }
}

/// Keeps a migration that stopped partway, so the user can see why, and says so.
fn stopped(copy: &Path, state: &mut Value, e: Failure) -> Result<PathBuf, Failure> {
    let unchanged = changes(&original_of(state), &recorded(copy)).is_ok_and(|c| c.is_empty());
    state["stage"] = json!("failed");
    state["problem"] = json!(e.0);
    let mut msg = format!("The migration stopped: {}\n", e.0);
    msg += if unchanged {
        "The original matches its inventory: it's unchanged.\n"
    } else {
        "The original doesn't match its inventory: look at what changed before going on.\n"
    };
    msg += &match save(copy, state) {
        Ok(()) => format!("The copy at {} is kept so you can see why; Abandon (`solace migrate --abandon`) removes it.", copy.display()),
        Err(s) => format!("Saving the migration's record failed too ({}); remove {} yourself.", s.0, copy.display()),
    };
    Err(Failure(msg, e.1))
}

fn record(dir: &Path) -> Result<Value, Failure> {
    load(dir).ok_or_else(|| Failure(format!("No migration at {}.", dir.display()), 1))
}

pub fn recheck(copy: &Path, step: &dyn Fn(&str)) -> Result<(), Failure> {
    let _lock = exclusive(copy.parent().unwrap_or(copy))?;
    let mut state = record(copy)?;
    if stage(&state) != "checked" || state["adopted"] == true {
        return fail("Only a migration in progress, with its checks run, can be rechecked.");
    }
    let _held = hold(copy)?;
    recheck_state(copy, &mut state, step)
}

/// Why Adopt isn't offered yet.
fn blockers(state: &Value, current: bool) -> Vec<String> {
    let Some(checks) = state["report"]["checks"].as_array() else { return vec!["No checks have run yet".into()] };
    let mut out = vec![];
    if !current {
        out.push("The copy changed since the checks ran: Recheck".into());
    }
    for c in checks.iter().filter(|c| c["status"] == "failed") {
        out.push(format!("{} failed", c["name"].as_str().unwrap_or("")));
    }
    out
}

// --- swapping folders --------------------------------------------------------
// A swap is two renames. The journal beside the folders records each one as
// it's done, so an interrupted swap is put back by the next command.

fn journal(parent: &Path) -> PathBuf {
    parent.join(JOURNAL)
}
fn renames(j: &Value) -> Vec<(PathBuf, PathBuf)> {
    j["renames"].as_array().into_iter().flatten().map(|r| (PathBuf::from(r[0].as_str().unwrap_or("")), PathBuf::from(r[1].as_str().unwrap_or("")))).collect()
}
fn moved(from: &Path, to: &Path) -> bool {
    fs::symlink_metadata(from).is_err() && fs::symlink_metadata(to).is_ok()
}
/// Test-only: stops the process after a swap's `n`th rename.
fn fail_point(op: &str, n: usize) {
    if env::var("SOLACE_MIGRATE_FAIL_AFTER").is_ok_and(|v| v == format!("{op}:{n}")) {
        panic!("SOLACE_MIGRATE_FAIL_AFTER={op}:{n}");
    }
}
/// Does the journal's renames; if one fails, puts back those done.
fn forward(parent: &Path, j: &mut Value) -> Result<(), Failure> {
    write_atomic(&journal(parent), &serde_json::to_vec(j).unwrap())?;
    let op = j["op"].as_str().unwrap_or("").to_string();
    for (n, (from, to)) in renames(j).iter().enumerate() {
        if fs::symlink_metadata(to).is_ok() {
            reverse(parent, j)?;
            return fail(format!("{} exists, so nothing was swapped.", to.display()));
        }
        if let Err(e) = fs::rename(from, to) {
            reverse(parent, j)?;
            return Err(named(from, format!("couldn't move it to {}: {e}; the folders are as they were", to.display())));
        }
        j["done"] = json!(n + 1);
        write_atomic(&journal(parent), &serde_json::to_vec(j).unwrap())?;
        fail_point(&op, n + 1);
    }
    Ok(())
}
/// Puts back the renames done, restores the record's `adopted`, and removes
/// the journal. The filesystem, not the journal's count, says what was done,
/// since a rename can land just before the journal records it.
fn reverse(parent: &Path, j: &Value) -> Result<(), Failure> {
    for (from, to) in renames(j).iter().rev() {
        if !moved(from, to) {
            continue;
        }
        if let Err(e) = fs::rename(to, from) {
            return fail(format!(
                "Couldn't swap the folders back: {} should go back to {} ({e}). Nothing else was moved. {} is kept, and the next `solace migrate` or `solace serve` here tries again.",
                to.display(),
                from.display(),
                journal(parent).display()
            ));
        }
    }
    let home = PathBuf::from(j["home"].as_str().unwrap_or(""));
    if let Some(mut s) = load(&home) {
        s["adopted"] = j["adopted"].clone();
        save(&home, &s)?;
    }
    fs::remove_file(journal(parent))?;
    Ok(())
}
/// Puts back a swap that was interrupted, before anything else runs here.
pub fn recover(parent: &Path) -> Result<Option<String>, Failure> {
    let Ok(bytes) = fs::read(journal(parent)) else { return Ok(None) };
    let j: Value = serde_json::from_slice(&bytes).map_err(|e| named(&journal(parent), e))?;
    reverse(parent, &j)?;
    let (from, _) = renames(&j).into_iter().next().unwrap_or_default();
    Ok(Some(format!(
        "An interrupted {} was put back: {} is where it was before it started.\n",
        if j["op"] == "adopt" { "adoption" } else { "undo" },
        from.display()
    )))
}
/// `recover`, run from a shell or a starting server; it holds the migration lock.
pub fn recover_here(top: &Path) -> Result<Option<String>, Failure> {
    let Some(parent) = top.parent() else { return Ok(None) };
    if !journal(parent).exists() {
        return Ok(None);
    }
    let _lock = exclusive(parent)?;
    recover(parent)
}

/// Swaps the copy into the original's place (Spec 17 §6). Returns the path it now has.
pub fn adopt(copy: &Path, step: &dyn Fn(&str)) -> Result<PathBuf, Failure> {
    let parent = copy.parent().unwrap_or(copy);
    let _lock = exclusive(parent)?;
    recover(parent)?;
    let mut state = record(copy)?;
    if copy.file_name() != Some(COPY.as_ref()) || state["adopted"] == true {
        return fail(format!("{} isn't a migration in progress.", copy.display()));
    }
    if stage(&state) != "checked" {
        return refuse("E7021", "not ready to adopt: the checks haven't run");
    }
    let _held = hold(copy)?;
    let b = blockers(&state, current(copy, &state, false)?);
    if !b.is_empty() {
        return refuse("E7021", format!("not ready to adopt:\n{}", b.join("\n")));
    }
    let original = original_of(&state);
    step("Comparing the original with its inventory");
    let ch = changes(&original, &recorded(copy))?;
    if !ch.is_empty() {
        return refuse("E7022", format!("the original changed since the copy was taken, so swapping would lose that work:\n{}", ch.join("\n")));
    }
    let pre = pre_solace(&original);
    if fs::symlink_metadata(&pre).is_ok() {
        return fail(format!("{} already exists; move it away first.", pre.display()));
    }
    step("Swapping the folders");
    let mut j = json!({"op":"adopt","renames":[[original, pre], [copy, original]],"done":0,"home":copy,"adopted":false});
    forward(parent, &mut j)?;
    step("Building and testing at the original path");
    let now = suite(&original, &strings(&state["projects"]), &strings(&state["smoke"]), Some(&pre));
    let failed: Vec<String> =
        behaviour(&state["baseline"], &now, true).into_iter().filter(|i| i["status"] == "failed").map(|i| i["text"].as_str().unwrap_or("").to_string()).collect();
    if !failed.is_empty() {
        reverse(parent, &j)?;
        return fail(format!("At the original path these failed, so both folders were swapped back:\n{}", failed.join("\n")));
    }
    state["adopted"] = json!(true);
    save(&original, &state)?;
    fs::remove_file(journal(parent))?;
    Ok(original)
}

/// Deletes the copy. Returns the original's path.
pub fn abandon(copy: &Path) -> Result<(PathBuf, bool), Failure> {
    let parent = copy.parent().unwrap_or(copy);
    let _lock = exclusive(parent)?;
    recover(parent)?;
    let state = record(copy)?;
    if copy.file_name() != Some(COPY.as_ref()) {
        return fail(format!("{} isn't a migration copy, so it isn't deleted.", copy.display()));
    }
    if state["adopted"] == true {
        return fail("This migration was adopted; use Undo adoption instead.");
    }
    let _held = hold(copy)?;
    let original = original_of(&state);
    let unchanged = changes(&original, &recorded(copy)).is_ok_and(|c| c.is_empty());
    remove_tree(copy).map_err(|e| Failure(format!("Couldn't delete all of {}: {e}. Delete what's left yourself.", copy.display()), 1))?;
    Ok((original, unchanged))
}

/// Swaps an adopted project and `<name>.pre-solace` back; the copy is again
/// `../solace-migration/`, in progress. Returns the original's path.
pub fn undo(top: &Path) -> Result<PathBuf, Failure> {
    let parent = top.parent().unwrap_or(top);
    let _lock = exclusive(parent)?;
    recover(parent)?;
    let state = record(top)?;
    let pre = pre_solace(top);
    if state["adopted"] != true || !pre.exists() {
        return fail(format!("Nothing to undo: {} isn't there.", pre.display()));
    }
    let _held = hold(top)?;
    let ch = changes(&pre, &recorded(top))?;
    if !ch.is_empty() {
        return refuse("E7022", format!("{} changed since it was set aside, so it isn't swapped back:\n{}", pre.display(), ch.join("\n")));
    }
    let copy = top.with_file_name(COPY);
    if fs::symlink_metadata(&copy).is_ok() {
        return refuse("E7020", format!("{} exists; move it away first", copy.display()));
    }
    let mut j = json!({"op":"undo","renames":[[top, copy], [pre, top]],"done":0,"home":top,"adopted":true});
    forward(parent, &mut j)?;
    let mut moved = record(&copy)?;
    moved["adopted"] = json!(false);
    save(&copy, &moved)?;
    fs::remove_file(journal(parent))?;
    Ok(top.to_path_buf())
}

/// `GET /migration`, apart from a step that is running.
pub fn status(top: &Path) -> Result<Value, Failure> {
    let mut out = json!({"state":"none","top":top,"original":null,"copy":null,"step":null,"candidates":[],"report":null,
        "blockers":[],"undo":false,"flagged":[],"problem":null});
    let Some(mut state) = load(top) else {
        // A board serving the original still shows the migration beside it.
        let copy = top.with_file_name(COPY);
        if load(&copy).is_some_and(|s| original_of(&s) == top) {
            let mut v = status(&copy)?;
            v["top"] = json!(top);
            return Ok(v);
        }
        let c = candidates(top)?;
        if !c.is_empty() {
            out["state"] = json!("available");
            out["candidates"] = json!(c);
        }
        return Ok(out);
    };
    out["original"] = state["original"].clone();
    out["copy"] = json!(top);
    if state["adopted"] == true {
        out["copy"] = Value::Null;
        if pre_solace(top).exists() {
            out["state"] = json!("adopted");
            out["original"] = json!(top);
            out["undo"] = json!(true);
        }
        return Ok(out);
    }
    match stage(&state) {
        "copying" => out["state"] = json!("copying"),
        "confirm_paths" => {
            out["state"] = json!("confirm_paths");
            out["flagged"] = json!(flagged(top, &state)?);
        }
        "failed" => {
            out["state"] = json!("failed");
            out["problem"] = state["problem"].clone();
            out["report"] = state["report"].clone();
        }
        _ => {
            let now = current(top, &state, true)?;
            if let Some(r) = state.get_mut("report").filter(|r| r.is_object()) {
                r["current"] = json!(now);
                if !now {
                    for c in r["checks"].as_array_mut().into_iter().flatten() {
                        c["status"] = json!("out of date");
                    }
                }
            }
            out["state"] = json!("in_progress");
            out["blockers"] = json!(blockers(&state, now));
            out["report"] = state["report"].clone();
        }
    }
    Ok(out)
}

fn text(v: &Value) -> String {
    let mut out = format!("Migration: {}\n", v["state"].as_str().unwrap_or(""));
    for c in v["candidates"].as_array().into_iter().flatten() {
        out += &format!("Can migrate: {} ({})\n", c["dir"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""));
    }
    for (k, label) in [("original", "Original"), ("copy", "Copy")] {
        if let Some(p) = v[k].as_str() {
            out += &format!("{label}: {p}\n");
        }
    }
    if let Some(p) = v["problem"].as_str() {
        out += &format!("\n{p}\n");
    }
    let flagged = v["flagged"].as_array().cloned().unwrap_or_default();
    if !flagged.is_empty() {
        out += "\nThese could reach your original from inside the copy. Look at each one; nothing has run yet:\n";
        for i in &flagged {
            out += &format!("  - [{}] {}\n", i["status"].as_str().unwrap_or(""), i["text"].as_str().unwrap_or(""));
        }
        out += "Then `solace migrate --confirm-paths` goes on, or `solace migrate --abandon` deletes the copy.\n";
    }
    for w in strings(&v["report"]["warnings"]) {
        out += &format!("\n{w}\n");
    }
    for c in v["report"]["checks"].as_array().into_iter().flatten() {
        out += &format!("\n{}: {} ({})\n", c["name"].as_str().unwrap_or(""), c["status"].as_str().unwrap_or(""), c["summary"].as_str().unwrap_or(""));
        for i in c["items"].as_array().into_iter().flatten() {
            let s = i["status"].as_str().map_or(String::new(), |s| format!("[{s}] "));
            out += &format!("  - {s}{}\n", i["text"].as_str().unwrap_or(""));
            if i["status"] == "failed"
                && let Some(d) = i["detail"].as_str()
            {
                out += &d.lines().map(|l| format!("      {l}\n")).collect::<String>();
            }
        }
    }
    let b = strings(&v["blockers"]);
    if v["state"] == "in_progress" {
        out += &if b.is_empty() { "\nReady to adopt: `solace migrate --adopt`.\n".to_string() } else { format!("\nNot ready to adopt:\n  {}\n", b.join("\n  ")) };
    }
    if v["state"] == "failed" || v["state"] == "copying" {
        out += "\n`solace migrate --abandon` deletes the copy.\n";
    }
    out
}

fn confirm(question: &str, yes: bool) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    Ok(std::io::stdin().lines().next().and_then(|l| l.ok()).is_some_and(|l| l.trim().to_lowercase().starts_with('y')))
}
const CARRIES: &str = "Errors in the original carry over: a build or test that fails before the migration fails after it too.";

/// `solace migrate [--project DIR]... [--smoke CMD]... [--confirm-paths] [--yes]`,
/// and `--report`, `--recheck`, `--confirm-paths`, `--adopt`, `--abandon`,
/// `--undo`, from the original's folder or the copy's.
pub fn command(args: &[&str]) -> Out {
    let here = env::current_dir()?;
    let top = workspace::enclosing(&here)?.unwrap_or_else(|| here.clone());
    if let Some(done) = recover_here(&top)? {
        return Ok(format!("{done}Run the command again from there.\n"));
    }
    let copy = locate(&top);
    let yes = args.contains(&"--yes");
    let confirmed = args.contains(&"--confirm-paths");
    let step = |s: &str| println!("{s}…");
    let values = |flag: &str| args.windows(2).filter(|w| w[0] == flag).map(|w| w[1].to_string()).collect::<Vec<_>>();
    let action = args.iter().find(|a| ["--report", "--recheck", "--adopt", "--abandon", "--undo"].contains(a)).copied();
    let action = action.or_else(|| (confirmed && load(&copy).is_some_and(|s| stage(&s) == "confirm_paths")).then_some("--confirm-paths"));
    if action.is_some_and(|a| a != "--report") {
        not_served(if action == Some("--undo") { &top } else { &copy })?;
    }
    match action {
        // `status` of the folder itself also finds a copy beside it, and lists candidates when there's none.
        Some("--report") => Ok(text(&status(&top)?)),
        Some("--recheck") => {
            recheck(&copy, &step)?;
            Ok(text(&status(&copy)?))
        }
        Some("--confirm-paths") => {
            confirm_paths(&copy, &step)?;
            Ok(text(&status(&copy)?))
        }
        Some("--adopt") => {
            let state = record(&copy)?;
            let original = state["original"].as_str().unwrap_or("").to_string();
            let report = status(&copy)?;
            let unchecked: Vec<String> = report["report"]["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|c| {
                    let name = c["name"].as_str().unwrap_or("").to_string();
                    c["items"].as_array().into_iter().flatten().filter(|i| i["status"] == "not checked").map(move |i| format!("{name}: {}", i["text"].as_str().unwrap_or("")))
                })
                .collect();
            println!("{CARRIES}");
            if !unchecked.is_empty() {
                println!("Not checked:\n  {}", unchecked.join("\n  "));
            }
            let q = format!("Rename {original} to {original}.pre-solace and move the copy into its place?");
            if !confirm(&q, yes)? {
                return Ok("Nothing was changed.\n".into());
            }
            let at = adopt(&copy, &step)?;
            Ok(format!(
                "Migration complete: {} is now the Solace project; the original is kept at {}.pre-solace.\nA shell that was in either folder has moved with it: run `cd {}`.\n",
                at.display(),
                at.display(),
                at.display()
            ))
        }
        Some("--abandon") => {
            if !confirm(&format!("Delete {}? The original isn't touched.", copy.display()), yes)? {
                return Ok("Nothing was changed.\n".into());
            }
            let (original, unchanged) = abandon(&copy)?;
            Ok(if unchanged {
                format!("Deleted the copy; {} matches its inventory, as it was.\n", original.display())
            } else {
                format!("Deleted the copy. {} doesn't match its inventory: it changed during the migration.\n", original.display())
            })
        }
        Some("--undo") => {
            if !confirm(&format!("Put the original back at {}, and the migrated project back at ../{COPY}?", top.display()), yes)? {
                return Ok("Nothing was changed.\n".into());
            }
            let original = undo(&top)?;
            Ok(format!(
                "{} is the original again; the migrated project is at ../{COPY}.\nA shell that was in either folder has moved with it: run `cd {}`.\n",
                original.display(),
                original.display()
            ))
        }
        _ => {
            let found = candidates(&top)?;
            if found.is_empty() {
                return fail("Nothing to migrate here: run `solace migrate` in a folder holding Gleam projects that Solace doesn't manage yet.");
            }
            let mut projects = values("--project");
            if projects.is_empty() {
                projects = found.iter().filter_map(|f| f["dir"].as_str().map(String::from)).collect();
            }
            let q = format!("{CARRIES}\nCopy {} to ../{COPY} and migrate {} there? The original isn't changed.", top.display(), projects.join(", "));
            if !confirm(&q, yes)? {
                return Ok("Nothing was changed.\n".into());
            }
            let copy = start(&top, &projects, &values("--smoke"), confirmed, &step)?;
            Ok(text(&status(&copy)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_layout_differences() {
        assert_eq!(explain("a\r\nb\r\n", "a\nb\n"), vec!["line endings"]);
        assert_eq!(explain("fn a() {}\n\n\n\nfn b() {}", "fn a() {}\n\nfn b() {}\n"), vec!["final newline", "blank lines between definitions"]);
    }

    #[test]
    fn finds_lines_inside_strings() {
        let src = "pub fn a() {\n  \"one\n\npub fn b() {}\n\\\"still\"\n}\n// \"not a string\npub fn c() { \"x\" }\n";
        assert_eq!(string_lines(src), vec![2, 3, 4]);
    }

    #[test]
    fn a_definition_inside_a_string_is_not_layout() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let src = "pub fn a() {\n  \"\npub fn b() {}\n\"\n}\n";
        let m = gleam::split(src);
        db.execute("INSERT INTO modules VALUES ('app', 'src', ?1)", [&m.header]).unwrap();
        for (i, it) in m.items.iter().enumerate() {
            db.execute("INSERT INTO symbols(id, module, name, kind, text, ord, revision) VALUES (?1, 'app', ?2, 'fn', ?3, ?4, '')", params![i as i64, it.name, it.text, i as i64])
                .unwrap();
        }
        assert_eq!(changed_pieces(&db, "src/app.gleam", src).unwrap(), vec!["a multi-line string holding a line that looks like a definition"]);
        assert!(changed_pieces(&db, "src/app.gleam", "pub fn a() {\n  \"\n").unwrap().len() > 0);
    }

    #[test]
    fn a_failure_the_original_had_is_not_blamed() {
        let base = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.old_test"]}},"smoke":[{"ok":true}]});
        let now = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.old_test","app_test.new_test"],"output":""}},
            "smoke":[{"command":"true","ok":false,"output":""}]});
        let items = behaviour(&base, &now, true);
        let status = |t: &str| items.iter().find(|i| i["text"].as_str().unwrap().contains(t)).unwrap()["status"].clone();
        assert_eq!(status("old_test"), "not checked");
        assert_eq!(status("new_test"), "failed");
        assert_eq!(status("smoke"), "failed");
        assert_eq!(check("Behaviour", "", items)["status"], "failed");
    }

    #[test]
    fn tests_are_reported_when_only_the_migration_builds() {
        let base = json!({"projects":{".":{"build":false,"test":false,"failing":[]}},"smoke":[]});
        let now = json!({"projects":{".":{"build":true,"test":false,"failing":["app_test.t"],"output":""}},"smoke":[]});
        let items = behaviour(&base, &now, false);
        assert!(items.iter().any(|i| i["text"].as_str().unwrap().contains("app_test.t fails; the original didn't build")));
        let none = json!({"projects":{".":{"build":false,"test":false,"failing":[],"output":""}},"smoke":[]});
        assert_eq!(warnings(&base, &none).len(), 1);
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative(Path::new("/a/b"), Path::new("/a/c/d")), PathBuf::from("../c/d"));
        assert_eq!(relative(Path::new("/a"), Path::new("/a/x")), PathBuf::from("x"));
    }
}
