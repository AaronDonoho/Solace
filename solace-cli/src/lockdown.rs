use super::*;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::{Duration, Instant};

pub fn error<T>(code: &str, at: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    fail(format!("{code} command refused\n  at {at}\n  {message}\n"))
}

/// A Gleam command in a project of plain files (D-60).
pub fn plain<T>(command: &str) -> Result<T, Failure> {
    let use_ = if board::strategy_context().is_some() { "files, grep and read" } else { "files, grep, read, write and run" };
    error("E6013", command, format!("this project has no Gleam code (no gleam.toml); use {use_}"))
}

/// The default `watch_limit`: text files up to this many bytes are watched.
pub const WATCH_LIMIT: usize = 262_144;

/// E6020: solace.toml can't be used, and why.
pub fn invalid_settings(why: impl std::fmt::Display) -> Failure {
    Failure(format!("E6020 invalid solace.toml\n  {why}"), 1)
}

pub fn settings(root: &Path) -> Result<toml::Value, Failure> {
    match fs::read_to_string(root.join("solace.toml")) {
        // A folder of projects' solace.toml only lists them (D-57); `workspace` checks it.
        Ok(s) if !root.join("gleam.toml").exists() && !root.join(".solace/store.db").exists() => s.parse().map_err(invalid_settings),
        Ok(s) => parse_settings(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Value::Table(Default::default())),
        Err(e) => Err(e.into()),
    }
}
/// Checks settings text without reading or writing the file.
pub fn parse_settings(s: &str) -> Result<toml::Value, Failure> {
    let v: toml::Value = s.parse().map_err(invalid_settings)?;
    let invalid = invalid_settings;
    for (old, fix) in [
        ("resources", "`resources` was removed: every text file is watched now; delete it"),
        ("ignore", "`ignore` is now `unwatched`; rename it"),
        ("resource_limit", "`resource_limit` is now `watch_limit`; rename it"),
    ] {
        if v.get(old).is_some() {
            return Err(invalid(fix));
        }
    }
    // `projects`: the folders below this root project (D-57); `workspace` checks them.
    if v.get("projects").is_some_and(|p| p.as_array().is_none_or(|a| a.iter().any(|d| d.as_str().is_none()))) {
        return Err(invalid("projects"));
    }
    for key in ["unwatched"] {
        if let Some(a) = v.get(key) {
            for p in a.as_array().ok_or_else(|| invalid(key))? {
                let p = p.as_str().ok_or_else(|| invalid(key))?;
                globset::Glob::new(p).map_err(|_| invalid(p))?;
            }
        }
    }
    if v.get("watch_limit").is_some_and(|x| x.as_integer().is_none_or(|n| n < 0)) {
        return Err(invalid("watch_limit"));
    }
    for key in ["commands", "validators"] {
        if let Some(t) = v.get(key) {
            for (name, c) in t.as_table().ok_or_else(|| invalid(key))? {
                let argv = if key == "commands" { c.get("run") } else { Some(c) };
                if argv.and_then(|a| a.as_array()).is_none_or(|a| a.is_empty() || a.iter().any(|x| x.as_str().is_none())) {
                    return Err(invalid(name));
                }
                if key == "validators" {
                    globset::Glob::new(name).map_err(|_| invalid(name))?;
                }
                // `checks`: the entry points a test suite drives (D-44), `"*"` for all
                if c.get("timeout").is_some_and(|x| x.as_integer().is_none_or(|n| n <= 0))
                    || c.get("args").is_some_and(|x| x.as_bool().is_none())
                    || c.get("allowed_in_reviews").is_some_and(|x| x.as_bool().is_none())
                    || c.get("checks").is_some_and(|x| x.as_array().is_none_or(|a| a.iter().any(|e| e.as_str().is_none())))
                {
                    return Err(invalid(name));
                }
            }
        }
    }
    Ok(v)
}
fn limit(v: &toml::Value) -> usize {
    v.get("watch_limit").and_then(|x| x.as_integer()).unwrap_or(WATCH_LIMIT as i64) as usize
}
fn matches(pattern: &str, path: &str) -> bool {
    if pattern.ends_with('/') {
        return path == pattern.trim_end_matches('/') || path.starts_with(pattern);
    }
    globset::Glob::new(pattern).is_ok_and(|g| g.compile_matcher().is_match(path))
}
fn patterns(v: &toml::Value, key: &str, path: &str) -> bool {
    v.get(key).and_then(|a| a.as_array()).is_some_and(|a| a.iter().any(|p| matches(p.as_str().unwrap(), path)))
}
// Reject aliases, including symlinks inside the root: an alias must never
// grant file access to source, protected files, or Solace's own records.
pub fn path(root: &Path, name: &str) -> Result<PathBuf, Failure> {
    let p = Path::new(name);
    if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
        return error("E6002", name, "expected a path inside the project");
    }
    let mut out = root.to_path_buf();
    for c in p.components() {
        out.push(c);
        if fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
            return error("E6002", name, "symlink paths are not accessible");
        }
    }
    Ok(out)
}
/// Solace's own files, and every file of another Solace project nested in
/// this one (it has its own store; D-57).
fn internal(root: &Path, name: &str) -> bool {
    let mut dir = root.to_path_buf();
    Path::new(name).components().any(|c| {
        dir.push(c);
        c.as_os_str() == ".solace" || dir.join(".solace/store.db").exists()
    })
}
fn gitignored(root: &Path, name: &str) -> bool {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    let mut parent = root.to_path_buf();
    builder.add(parent.join(".gitignore"));
    for c in Path::new(name).parent().unwrap_or(Path::new("")).components() {
        parent.push(c);
        builder.add(parent.join(".gitignore"));
    }
    builder.build().is_ok_and(|g| g.matched_path_or_any_parents(root.join(name), root.join(name).is_dir()).is_ignore())
}
/// A `.gitignore` anywhere in a project: what it ignores is unwatched, so only a human changes it.
pub fn is_gitignore(name: &str) -> bool {
    (name == ".gitignore" || name.ends_with("/.gitignore")) && !name.split('/').any(|c| c == ".solace")
}
/// Dependency folders are never watched, whatever the `.gitignore` says or when it was written.
fn dependencies(name: &str) -> bool {
    name.split('/').any(|c| c == "node_modules")
}
pub fn tier(root: &Path, v: &toml::Value, name: &str, proposed: Option<&[u8]>) -> String {
    // A project of plain files holds no symbols (D-60).
    if root.join("gleam.toml").exists() && (name.starts_with("src/") || name.starts_with("test/")) && name.ends_with(".gleam") {
        return "graph".into();
    }
    if name == "solace.toml" || name == "AGENTS.md" {
        return "protected".into();
    }
    if internal(root, name) {
        return "internal".into();
    }
    if is_gitignore(name) {
        return "protected".into();
    }
    if dependencies(name) {
        return "unwatched".into();
    }
    if name == ".git" || name.starts_with(".git/") || name == "build" || name.starts_with("build/") || name == "manifest.toml" {
        return "unwatched".into();
    }
    if gitignored(root, name) || patterns(v, "unwatched", name) {
        return "unwatched".into();
    }
    let size = proposed.map(|b| b.len() as u64).unwrap_or_else(|| fs::metadata(root.join(name)).map_or(0, |m| m.len()));
    if size > limit(v) as u64 {
        return "unwatched (over limit)".into();
    }
    let bytes = proposed.map(Vec::from).unwrap_or_else(|| fs::read(root.join(name)).unwrap_or_default());
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return "unwatched (binary)".into();
    }
    "watched".into()
}
fn session_file(root: &Path) -> PathBuf {
    let session = env::var("SOLACE_SESSION")
        .unwrap_or_else(|_| if env::args().any(|a| a == "mcp") { std::process::id().to_string() } else { "cli".into() });
    root.join(".solace").join(format!("reads-{}.json", sha256(session.as_bytes())))
}
fn reads(root: &Path) -> Value {
    fs::read(session_file(root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}))
}
fn remember(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Failure> {
    let mut r = reads(root);
    r[name] = json!(sha256(bytes));
    fs::write(session_file(root), r.to_string())?;
    Ok(())
}
/// Puts the project back after a crash (D-77 point 7), at the start of each
/// command and at serve start. It runs only under the project lock (`_held`):
/// a live command holds that lock until its records are dropped, so every
/// record found here is a dead command's. Checking that the run has ended
/// would be wrong, since a run whose command crashed keeps running and its
/// next command must adopt the record. A record with a task and a
/// run is an undo record: if its file changed, the write landed and the rows
/// never committed, so recovery adds them; if the file is as the record says,
/// the write never landed and the record is dropped. A record with no task is
/// the in-place validator journal (Spec 14 §64): the file is restored.
pub fn recover(root: &Path, store: &Store, _held: &[fs::File]) -> Result<(), Failure> {
    let pending = root.join(".solace/pending");
    let Ok(dir) = fs::read_dir(&pending) else { return Ok(()) };
    let mut records = vec![];
    for entry in dir {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json") {
            // A record that can't be read was never fully written, so its change
            // never started: drop it rather than fail every later command.
            match fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(record) => records.push((p, record)),
                None => fs::remove_file(&p)?,
            }
        }
    }
    if records.iter().any(|(_, r)| r["task"].is_string() && r["run"].is_string()) {
        // A command's outer transaction never committed: the code files go back
        // to the store, which is the source of truth.
        store.materialize(&store.index()?)?;
    }
    for (p, record) in records {
        if record["task"].is_string() && record["run"].is_string() {
            board::adopt_pending(store, &record)?;
        } else {
            let target = path(root, record["path"].as_str().ok_or_else(|| Failure("invalid pending record".into(), 1))?)?;
            if let Some(a) = record["bytes"].as_array() {
                fs::write(target, json_bytes(a))?;
            } else if target.exists() {
                fs::remove_file(target)?;
            }
        }
        fs::remove_file(p)?;
    }
    Ok(())
}
/// Marks that run `rid` of task `tid` is changing the project, before its outer
/// transaction begins, so a crash before the commit sends recovery to put the
/// code files back from the store (D-77 point 7).
pub fn mark(root: &Path, tid: &str, rid: &str) -> Result<(), Failure> {
    let dir = root.join(".solace/pending");
    fs::create_dir_all(&dir)?;
    write_atomic(&dir.join(format!("command-{rid}.json")), json!({"task":tid,"run":rid}).to_string().into_bytes())?;
    Ok(())
}
/// Whether the project has an undo record or command marker for task `tid`
/// that no recovery has adopted yet: a run was stopped mid-write, so its change
/// is on disk but not yet in the task's rows.
pub fn pending_for(root: &Path, tid: &str) -> bool {
    let Ok(dir) = fs::read_dir(root.join(".solace/pending")) else { return false };
    dir.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).any(|p| {
        fs::read(&p).is_ok_and(|b| serde_json::from_slice::<Value>(&b).is_ok_and(|v| v["task"].as_str() == Some(tid)))
    })
}
/// Drops the undo records a committed command wrote for run `rid`.
pub fn drop_records(root: &Path, rid: &str) -> Result<(), Failure> {
    let dir = root.join(".solace/pending");
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json")
            && fs::read(&p).is_ok_and(|b| serde_json::from_slice::<Value>(&b).is_ok_and(|v| v["run"].as_str() == Some(rid)))
        {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}
/// Every watched file path, relative to `root`.
pub fn watched_paths(root: &Path, v: &toml::Value) -> Result<Vec<String>, Failure> {
    let mut rows = vec![];
    tree_rows(root, v, root, &mut rows)?;
    Ok(rows.into_iter().filter(|(_, t)| t == "watched").map(|(n, _)| n).collect())
}
#[cfg(test)]
fn read(root: &Path, name: &str, args: &[&str], budget: usize) -> Out {
    Ok(read_part(root, name, args, budget)?.0)
}
/// A file's text within `budget`, and whether it was cut short.
fn read_part(root: &Path, name: &str, args: &[&str], budget: usize) -> Result<(String, bool), Failure> {
    let file = fs::File::open(path(root, name)?)?;
    // The lines asked for, in file order: the whole file without `--lines`.
    let mut ranges = vec![(1usize, usize::MAX)];
    let mut reader = BufReader::new(file);
    if let ["--lines", spec] = args {
        ranges = line_ranges(spec)?;
    } else if let ["--tail", n] = args {
        let n: usize = n.parse().map_err(|_| Failure("invalid tail count".into(), 2))?;
        let length = reader.get_ref().metadata()?.len();
        let mut pos = length;
        let mut newlines = 0;
        let mut buf = [0; 8192];
        while pos > 0 && newlines <= n && length - pos < budget as u64 + 8192 {
            let len = pos.min(buf.len() as u64) as usize;
            pos -= len as u64;
            reader.seek(SeekFrom::Start(pos))?;
            reader.read_exact(&mut buf[..len])?;
            newlines += buf[..len].iter().filter(|&&b| b == b'\n').count();
        }
        reader.seek(SeekFrom::Start(pos))?;
        let mut lines = std::collections::VecDeque::new();
        for line in reader.lines() {
            lines.push_back(line?);
            if lines.len() > n {
                lines.pop_front();
            }
        }
        // A tail shows only the file's end, so it doesn't count as reading it
        // for a whole-file write.
        let text = lines.into_iter().collect::<Vec<_>>().join("\n") + "\n";
        return Ok((trim_output(&text, budget), false));
    } else if !args.is_empty() {
        return usage("read <path> [--lines 3,7,10..12 | --tail n]");
    }
    let mut out = String::new();
    let mut truncated = false;
    let mut n = 0;
    let mut shown = 0;
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                break;
            }
            let end = available.iter().position(|&b| b == b'\n').map(|p| p + 1);
            let count = end.unwrap_or(available.len());
            if line.len() + count <= budget {
                line.extend_from_slice(&available[..count]);
            } else {
                oversized = true;
            }
            reader.consume(count);
            if end.is_some() {
                break;
            }
        }
        if line.is_empty() && !oversized {
            break;
        }
        n += 1;
        if n > ranges.last().unwrap().1 {
            break;
        }
        let Some(r) = ranges.iter().position(|&(a, b)| a <= n && n <= b) else { continue };
        let line = String::from_utf8_lossy(&line);
        // A gap between ranges is one `…` line.
        let gap = if shown > 0 && n > shown + 1 { "…\n" } else { "" };
        let text = if args.is_empty() { line.to_string() } else { format!("{gap}{n}: {line}") };
        if oversized || out.len() + text.len() + 100 > budget {
            let next = if args.is_empty() {
                format!("{n}..{}", n + 100)
            } else {
                line_list(&[(n, ranges[r].1)].into_iter().chain(ranges[r + 1..].iter().copied()).collect::<Vec<_>>())
            };
            out += &format!("Next: read {name} --lines {next}\n");
            truncated = true;
            break;
        }
        out += &text;
        shown = n;
    }
    // Watched files are bounded; unwatched logs are never hashed or loaded in full.
    // A whole-file read cut short leaves the file unread for a whole-file write.
    if (!truncated || !args.is_empty()) && fs::metadata(root.join(name))?.len() <= WATCH_LIMIT as u64 * 16 {
        remember(root, name, &fs::read(root.join(name))?)?;
    }
    if truncated && out.len() > budget {
        out.truncate(out.floor_char_boundary(budget));
    }
    Ok((out, truncated))
}
/// `--lines` (D-105): lines and ranges such as `150,156,200..210`, sorted and
/// merged into file order.
fn line_ranges(spec: &str) -> Result<Vec<(usize, usize)>, Failure> {
    let bad = || Failure("invalid line range: --lines takes lines and ranges such as 150,156,200..210".into(), 2);
    let mut asked = vec![];
    for part in spec.split(',') {
        let (a, b) = part.split_once("..").unwrap_or((part, part));
        let (a, b): (usize, usize) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
        if a == 0 || b < a {
            return Err(bad());
        }
        asked.push((a, b));
    }
    asked.sort();
    let mut ranges: Vec<(usize, usize)> = vec![];
    for (a, b) in asked {
        match ranges.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => ranges.push((a, b)),
        }
    }
    Ok(ranges)
}
/// Ranges in `--lines` form: `150,156,200..210`.
fn line_list(ranges: &[(usize, usize)]) -> String {
    ranges.iter().map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}..{b}") }).collect::<Vec<_>>().join(",")
}
pub fn trim_output(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.into();
    }
    let lines: Vec<_> = text.lines().collect();
    let mut head = String::new();
    let mut kept = 0;
    for l in lines.iter().take(20) {
        if head.len() + l.len() + 100 > budget {
            break;
        }
        head += l;
        head.push('\n');
        kept += 1;
    }
    let mut tail = Vec::new();
    let mut used = head.len() + 80;
    for l in lines[kept..].iter().rev() {
        if used + l.len() + 1 > budget {
            break;
        }
        used += l.len() + 1;
        tail.push(*l);
    }
    tail.reverse();
    let result = format!("{head}... {} lines omitted ...\n{}\n", lines.len() - kept - tail.len(), tail.join("\n"));
    result[..result.floor_char_boundary(budget.min(result.len()))].into()
}
/// Kills `child` and every process it started (its process group).
pub fn kill_group(child: &mut std::process::Child) {
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Waits for `child`, started in its own process group, up to `timeout`;
/// past it, kills the group and returns `None`.
pub fn wait_or_kill(child: &mut std::process::Child, timeout: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if start.elapsed() >= timeout {
            kill_group(child);
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn command(root: &Path, argv: &[String], timeout: u64, budget: usize) -> Out {
    use std::os::unix::process::CommandExt;
    fs::create_dir_all(root.join(".solace"))?;
    let log = root.join(".solace").join(format!("command-{}.log", unique_id()));
    let out = fs::File::create(&log)?;
    // During a migration the original is read-only to commands in the copy (Spec 17).
    let mut child = migrate::sandboxed(argv, root, migrate::protected(root).as_deref())
        .process_group(0)
        .stdout(out.try_clone()?)
        .stderr(out)
        .spawn()?;
    let result = match wait_or_kill(&mut child, Duration::from_secs(timeout))? {
        Some(status) => Ok(format!("exit code: {}\n", status.code().unwrap_or(-1))),
        None => error("E6012", &argv[0], format!("timed out after {timeout}s")),
    };
    let output = fs::read_to_string(&log)?;
    fs::remove_file(log)?;
    match result {
        Ok(prefix) => Ok(prefix.clone() + &trim_output(&output, budget.saturating_sub(prefix.len()))),
        Err(Failure(message, code)) => Err(Failure(message + &trim_output(&output, budget), code)),
    }
}
/// The file's new text: `new` whole, or with `old` replaced by `new` by the
/// rules of `submit --replace` (`all` replaces every occurrence).
pub fn edited(current: &[u8], new: &str, replace: bool, all: bool, old: Option<&str>) -> Out {
    // `old` given means an exact replacement, with or without `--replace`: a
    // whole-file write would drop everything around it.
    if !replace && old.is_none_or(str::is_empty) {
        return Ok(new.to_string());
    }
    let current = std::str::from_utf8(current).map_err(|_| Failure("E6002 text replacement requires a UTF-8 file".into(), 1))?;
    Ok(drafts::replace(current, old, new, all)?.0)
}
/// The paths a `write` or `remove` names (`remove` takes a list, D-102), for
/// a task's snapshot of the files a command changes.
pub fn paths<'a>(a: &[&'a str]) -> Vec<&'a str> {
    if !matches!(a[0], "write" | "remove") {
        return vec![];
    }
    a[1..].iter().copied().filter(|x| !x.starts_with("--")).collect()
}
/// The files a file patch (D-104) names, with their bytes now, for a task's
/// snapshot; a path it can't reach is left out, and the command refuses it.
pub fn patch_snapshot(root: &Path, patch: Option<&str>) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (_, name, ..) in patch.and_then(|t| drafts::sections(t, true).ok()).unwrap_or_default() {
        let name = file_name(Path::new(&name));
        if let Ok(p) = path(root, &name) {
            out.insert(format!("file:{name}"), json!(fs::read(p).ok()));
        }
    }
    out
}
/// Whether `journal` already holds an undo record from run `rid`.
fn same_run(journal: &Path, rid: &serde_json::Value) -> bool {
    fs::read(journal).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).is_some_and(|r| &r["run"] == rid)
}
fn write(root: &Path, v: &toml::Value, name: &str, new: &str, replace: bool, all: bool, old: Option<&str>) -> Out {
    // D-109: patch text is for `write` with no path; here it would become the file.
    if drafts::is_patch(new) {
        return usage("this text is a patch; send it with `write` and no path, which applies its `*** Update File: <path>` sections");
    }
    // `edited` treats a given `old` as a replacement; so does the read check.
    let replace = replace || old.is_some_and(|o| !o.is_empty());
    let p = path(root, name)?;
    let before = fs::read(&p).ok();
    let text = edited(before.as_deref().unwrap_or_default(), new, replace, all, old)?;
    if text.len() > limit(v) {
        return error("E6004", name, "proposed text exceeds watch_limit");
    }
    let t = tier(root, v, name, Some(text.as_bytes()));
    if t != "watched" && !t.starts_with("unwatched") {
        return error("E6002", name, format!("caught by tier rule: {t}"));
    }
    if !replace
        && let Some(ref b) = before
        && reads(root)[name].as_str() != Some(&sha256(b))
    {
        return error("E6005", name, "file is unread or changed since last read; `read` it again, or change part of it with `old` or a `write` patch (no path)");
    }
    // D-109: a whole-file write that would drop most of a long file is refused;
    // a `--lines` read allows a whole write, so the size is the check.
    let (was, now) = (before.as_deref().map(line_count), line_count(text.as_bytes()));
    if !replace
        && let Some(was) = was
        && was >= 200
        && now * 2 < was
    {
        return error(
            "E6002",
            name,
            format!(
                "would cut {name} from {was} lines to {now}; change part of it with `write` and a patch (no path), or `write {name}` with `old`; to replace it whole, `remove {name}` first, then `write {name}`"
            ),
        );
    }
    let journal = journal(root, name, &before)?;
    fs::create_dir_all(p.parent().unwrap())?;
    let result = fs::write(&p, &text).map_err(Failure::from).and_then(|_| validate(root, v, name, &p, &text));
    // A write that fails validation is put back and leaves no record, so
    // a failed command keeps no undo for it; one that succeeded keeps its
    // record until the outer transaction commits, even if the command then fails.
    settle(root, name, &p, &before, Some(text.as_bytes()), &journal, result.is_ok())?;
    result.map(|_| match was {
        Some(was) => format!("Wrote {name} ({was} → {})\n", count(now, "line")),
        None => format!("Wrote {name} ({})\n", count(now, "line")),
    })
}
/// A text's lines, the last one counted without its newline.
fn line_count(b: &[u8]) -> usize {
    b.iter().filter(|&&c| c == b'\n').count() + usize::from(!b.is_empty() && !b.ends_with(b"\n"))
}
/// D-109: a `read` path that doesn't exist, named with what it likely meant:
/// a line range is `--lines`, and a bare name is a symbol.
fn missing(root: &Path, name: &str, paths: &[&str]) -> Out {
    let lines = |s: &str| -> Option<String> {
        let (a, b) = s.split_once("..").or_else(|| s.split_once('-')).or_else(|| s.split_once(':')).unwrap_or((s, s));
        let num = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
        (num(a) && num(b)).then(|| if a == b { a.to_string() } else { format!("{a}..{b}") })
    };
    if let Some(range) = lines(name) {
        let others: Vec<&str> = paths.iter().copied().filter(|p| *p != name).collect();
        let file = if let [one] = others[..] { one } else { "<path>" };
        return usage(format!("read: `{name}` is not a file; for lines use `read {file} --lines {range}`"));
    }
    if let Some((file, at)) = name.rsplit_once(':')
        && let Some(range) = lines(at)
        && root.join(file).is_file()
    {
        return usage(format!("read: no such file `{name}`; for lines use `read {file} --lines {range}`"));
    }
    let symbol = if name.contains('/') || name.contains('.') {
        String::new()
    } else {
        format!("; read takes files, not symbols: `grep {name}` searches files, `find {name}` searches code in the store")
    };
    error("E6002", name, format!("no such file{symbol}"))
}
/// Saves `name`'s bytes before a change in `.solace/pending/`. In a board run
/// the record is an undo record: it stays until the command's outer
/// transaction commits, and recovery adds the rows from it (D-77 point 7);
/// otherwise it is the validator journal. Returns the record's path and
/// whether this change wrote it: an earlier change to this file in the
/// command keeps its record, which holds the original bytes.
fn journal(root: &Path, name: &str, before: &Option<Vec<u8>>) -> Result<(PathBuf, bool), Failure> {
    fs::create_dir_all(root.join(".solace/pending"))?;
    let journal = root.join(".solace/pending").join(format!("{}.json", sha256(name.as_bytes())));
    let record = match board::context() {
        Some((tid, rid)) => json!({"task":tid,"run":rid,"path":name,"bytes":before}),
        None => json!({"path":name,"bytes":before}),
    };
    let wrote = record["task"].is_null() || !same_run(&journal, &record["run"]);
    if wrote {
        write_atomic(&journal, record.to_string())?;
    }
    Ok((journal, wrote))
}
/// Checks a file written at `p` with `text`: a manifest needs a name and a
/// version, and each validator whose pattern matches must pass (E6003).
fn validate(root: &Path, v: &toml::Value, name: &str, p: &Path, text: &str) -> Result<(), Failure> {
    if name == "gleam.toml" {
        let manifest = text.parse::<toml::Value>().map_err(|e| Failure(format!("E6003 invalid manifest\n  {e}"), 1))?;
        if manifest.get("name").and_then(|x| x.as_str()).is_none_or(|s| !workspace::package_name(s))
            || manifest.get("version").and_then(|x| x.as_str()).is_none_or(|v| {
                let core = v.split(['-', '+']).next().unwrap_or("");
                let parts: Vec<_> = core.split('.').collect();
                parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
            })
        {
            return error("E6003", name, "manifest requires name and version");
        }
    }
    if let Some(validators) = v.get("validators").and_then(|x| x.as_table()) {
        for (pattern, argv) in validators {
            if matches(pattern, name) {
                let argv: Vec<String> =
                    argv.as_array().unwrap().iter().map(|x| x.as_str().unwrap().replace("{file}", &p.to_string_lossy())).collect();
                let out = command(root, &argv, 120, BUDGET).map_err(|e| Failure(format!("E6003 validator failed\n  {}", e.0), 1))?;
                if !out.starts_with("exit code: 0\n") {
                    return error("E6003", name, out);
                }
            }
        }
    }
    Ok(())
}
/// Ends a file change. A refused one puts the file back, with no undo to
/// record, and drops the record if this change wrote it (an earlier change's
/// record stays: that change still stands). A kept one drops the validator
/// journal (an undo record stays) and remembers the new bytes as read.
fn settle(
    root: &Path,
    name: &str,
    p: &Path,
    before: &Option<Vec<u8>>,
    after: Option<&[u8]>,
    (journal, wrote): &(PathBuf, bool),
    kept: bool,
) -> Result<(), Failure> {
    if !kept {
        match before {
            Some(b) => fs::write(p, b)?,
            None if p.exists() => fs::remove_file(p)?,
            None => {}
        }
        if *wrote {
            fs::remove_file(journal)?;
        }
        return Ok(());
    }
    if board::context().is_none() {
        fs::remove_file(journal)?;
    }
    if let Some(after) = after {
        remember(root, name, after)?;
    }
    Ok(())
}
/// One section of a `write` patch, checked and applied in memory: the file's
/// path, its bytes now, and its new text (`None` to delete it).
fn file_change(
    root: &Path,
    v: &toml::Value,
    name: &str,
    kind: drafts::Section,
    hunks: Vec<drafts::Hunk>,
) -> Result<(PathBuf, Option<Vec<u8>>, Option<String>), Failure> {
    use drafts::Section;
    let p = reachable(root, name)?;
    let t = tier(root, v, name, None);
    if t == "graph" {
        let module = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/")).unwrap().trim_end_matches(".gleam");
        return error("E6001", name, format!("source is held as symbols; change it with a `submit` patch (`*** Update: {module}`)"));
    }
    if t == "protected" {
        return error("E6002", name, "protected files change only through a human-approved proposal: in a work run, `write` it alone, with the exact new text and `reason`");
    }
    if t != "watched" && !t.starts_with("unwatched") {
        return error("E6002", name, format!("caught by tier rule: {t}"));
    }
    if p.is_dir() {
        return error("E6002", name, "a folder; a patch changes files");
    }
    let before = fs::read(&p).ok();
    match (kind, &before) {
        (Section::Add, Some(_)) => return error("E6002", name, "already exists; change it with `*** Update File`"),
        (Section::Update | Section::Delete, None) => return error("E6002", name, "no such file; create it with `*** Add File`"),
        (Section::Delete, _) if name == "gleam.toml" => return error("E6002", name, "gleam.toml can't be removed"),
        _ => {}
    }
    let current = match (kind, &before) {
        (Section::Update, Some(b)) => std::str::from_utf8(b).map_err(|_| Failure(format!("E6002 {name}: a patch changes only UTF-8 text"), 1))?,
        _ => "",
    };
    let text = drafts::file_text(name, current, kind, hunks)?;
    if let Some(text) = &text {
        if text.len() > limit(v) {
            return error("E6004", name, "proposed text exceeds watch_limit");
        }
        let t = tier(root, v, name, Some(text.as_bytes()));
        if t != "watched" && !t.starts_with("unwatched") {
            return error("E6002", name, format!("caught by tier rule: {t}"));
        }
    }
    Ok((p, before, text))
}
/// `write` with no path (D-104): a patch of `*** Add File`, `*** Update File`
/// and `*** Delete File` sections changes several files, whole or nothing.
/// Every section is checked and applied in memory before any file changes;
/// then every file is written before any is validated, so a validator sees
/// the project as it will be, and if one fails, every file goes back. An
/// update needs no earlier `read`: its hunks matching is the check.
fn write_patch(root: &Path, v: &toml::Value, raw: &str) -> Out {
    let mut changes: Vec<(String, PathBuf, Option<Vec<u8>>, Option<String>)> = vec![];
    let mut refused = vec![];
    for (kind, target, _, hunks) in drafts::sections(raw, true)? {
        let name = file_name(Path::new(&target));
        if changes.iter().any(|(n, ..)| *n == name) {
            refused.push(format!("E0108 the patch names {name} twice; give each file one section"));
            continue;
        }
        match file_change(root, v, &name, kind, hunks) {
            Ok((p, before, text)) => changes.push((name, p, before, text)),
            Err(e) => refused.push(e.0),
        }
    }
    if !refused.is_empty() {
        return Err(Failure(format!("nothing written:\n{}", refused.join("\n")), 1));
    }
    let changes: Vec<_> = changes.into_iter().filter(|(_, _, before, text)| text.as_deref().map(str::as_bytes) != before.as_deref()).collect();
    if changes.is_empty() {
        return fail("E0108 the patch changes nothing.");
    }
    let mut journals = vec![];
    let mut result = Ok(());
    for (name, p, before, text) in &changes {
        match journal(root, name, before) {
            Ok(j) => journals.push(j),
            Err(e) => {
                result = Err(e);
                break;
            }
        }
        let wrote = match text {
            Some(text) => fs::create_dir_all(p.parent().unwrap()).and_then(|_| fs::write(p, text)),
            None => fs::remove_file(p),
        };
        if let Err(e) = wrote {
            result = Err(e.into());
            break;
        }
    }
    if result.is_ok() {
        result = changes.iter().filter_map(|(name, p, _, text)| Some(validate(root, v, name, p, text.as_ref()?))).collect();
    }
    let kept = result.is_ok();
    for ((name, p, before, text), j) in changes.iter().zip(&journals) {
        settle(root, name, p, before, text.as_deref().map(str::as_bytes), j, kept)?;
    }
    if let Err(e) = result {
        return Err(Failure(format!("nothing written:\n{}", e.0), e.1));
    }
    Ok(changes
        .iter()
        .map(|(name, _, before, text)| match (before, text) {
            (_, None) => format!("Removed {name}\n"),
            (None, _) => format!("Added {name}\n"),
            _ => format!("Wrote {name}\n"),
        })
        .collect())
}
fn tree_rows(root: &Path, v: &toml::Value, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), Failure> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
        let t = tier(root, v, &name, None);
        if e.file_type()?.is_symlink() {
            out.push((name, "unwatched (symlink)".into()));
        } else if e.file_type()?.is_dir() {
            if t == "internal" {
                out.push((format!("{name}/"), "internal".into()));
            } else if name == "build" || name == ".git" || dependencies(&name) || patterns(v, "unwatched", &name) || gitignored(root, &name) {
                out.push((format!("{name}/"), format!("unwatched; {} files", count_files(&p)?)));
            } else {
                let mut children = vec![];
                tree_rows(root, v, &p, &mut children)?;
                if children.iter().any(|(_, tier): &(String, String)| tier == "graph" || tier == "watched" || tier == "protected") {
                    out.push((format!("{name}/"), String::new()));
                    out.extend(children);
                } else {
                    out.push((format!("{name}/"), format!("unwatched; {} files", count_files(&p)?)));
                }
            }
        } else {
            out.push((name, t));
        }
    }
    Ok(())
}
fn tree(root: &Path, v: &toml::Value, dir: &Path, out: &mut String) -> Result<(), Failure> {
    let mut rows = vec![];
    tree_rows(root, v, dir, &mut rows)?;
    for (name, tier) in rows {
        if tier.is_empty() {
            *out += &format!("{name}\n");
        } else {
            *out += &format!("{name} [{tier}]\n");
        }
    }
    Ok(())
}
fn count_files(p: &Path) -> Result<usize, Failure> {
    let mut n = 0;
    for e in fs::read_dir(p)? {
        let e = e?;
        n += if e.file_type()?.is_dir() { count_files(&e.path())? } else { 1 };
    }
    Ok(n)
}

/// Where `name` is in `root`, if an agent may reach it: not Solace's own files,
/// not out of the project or into a project nested in it, and no symlink.
fn reachable(root: &Path, name: &str) -> Result<PathBuf, Failure> {
    if Path::new(name).components().any(|c| c.as_os_str() == ".solace") {
        return error("E6006", name, "Solace internal files have no agent access");
    }
    // A path out of the project, or into a project nested in it, is another project's.
    if internal(root, name) || name.split('/').any(|c| c == "..") {
        // A strategy run changes nothing and has no task to ask about (D-93).
        let ask = if board::strategy_context().is_some() {
            ""
        } else {
            "; to change a project this task doesn't cover, ask the human with `task ask` rather than putting the change elsewhere"
        };
        return error("E6006", name, format!("outside this project: another project's files are reached with `--project <folder>`{ask}"));
    }
    path(root, name)
}

/// `move <from> <to> [--to-project <folder>]`: the source, the destination, and
/// the destination's project, `root` itself without `--to-project` (D-84).
pub fn move_args<'a>(root: &Path, a: &[&'a str]) -> Result<(&'a str, &'a str, PathBuf), Failure> {
    let (mut operands, mut project) = (vec![], None);
    let mut rest = a[1..].iter();
    while let Some(&x) = rest.next() {
        if x == "--to-project" {
            project = Some(*rest.next().ok_or_else(|| Failure("--to-project needs a project folder; session start lists them".into(), 2))?);
        } else {
            operands.push(x);
        }
    }
    let [from, to] = operands[..] else { return usage("move <from> <to> [--to-project <folder>]") };
    let Some(dir) = project.map(project_dir) else { return Ok((from, to, root.to_path_buf())) };
    let top = scope::top(root);
    if !workspace::members(&top)?.iter().any(|m| m == dir) {
        return error("E6007", dir, "not a project under this Solace root");
    }
    Ok((from, to, scope::path(&top, dir)))
}

/// Whether two folders are the same project.
pub fn same_project(a: &Path, b: &Path) -> bool {
    a == b || fs::canonicalize(a).ok().is_some_and(|x| fs::canonicalize(b).ok() == Some(x))
}

/// `rel` below `name`; `name` itself when `rel` is empty.
fn joined(name: &str, rel: &str) -> String {
    if rel.is_empty() { name.to_string() } else { format!("{name}/{rel}") }
}

/// The files a move of `p` (named `name`) takes, relative to it: "" for a file,
/// every file below a folder. A symlink, or anything that is neither file nor
/// folder, refuses the move.
fn moved_files(name: &str, p: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), Failure> {
    let kind = fs::symlink_metadata(p)?.file_type();
    if kind.is_symlink() || !(kind.is_file() || kind.is_dir()) {
        return error("E6002", &joined(name, rel), "only files and folders can be moved; this is a symlink or a special file");
    }
    if kind.is_file() {
        out.push(rel.to_string());
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(p)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        moved_files(name, &e.path(), &joined(rel, &e.file_name().to_string_lossy()), out)?;
    }
    Ok(())
}

/// Refuses a move of `name` out of or into its place in `root`: graph files are
/// a projection of the store, and protected and internal files aren't the agent's.
fn movable(root: &Path, v: &toml::Value, name: &str) -> Result<(), Failure> {
    if name == "gleam.toml" {
        return error("E6002", name, "gleam.toml can't be moved");
    }
    match tier(root, v, name, Some(b"")).as_str() {
        "graph" => error("E6001", name, "Gleam source is held as symbols, and its file is a projection of the store: it can't be moved"),
        "internal" => error("E6006", name, "Solace's own files and other projects' files can't be moved"),
        t if t == "watched" || t.starts_with("unwatched") => Ok(()),
        t => error("E6002", name, format!("{t} files can't be moved")),
    }
}

/// In a board run, the undo record for `name` before a move changes it, as
/// `remove` writes one. Returns a record this call wrote, so a failed move can drop it.
fn undo_record(root: &Path, name: &str, bytes: Option<Vec<u8>>) -> Result<Option<PathBuf>, Failure> {
    let Some((tid, rid)) = board::context() else { return Ok(None) };
    let dir = root.join(".solace/pending");
    fs::create_dir_all(&dir)?;
    let record = dir.join(format!("{}.json", sha256(name.as_bytes())));
    // An earlier change to this file in the command keeps its record: it holds the original bytes.
    if same_run(&record, &json!(rid)) {
        return Ok(None);
    }
    write_atomic(&record, json!({"task":tid,"run":rid,"path":name,"bytes":bytes}).to_string())?;
    Ok(Some(record))
}

/// This session's moves, by source, so a repeated move finds its own.
fn moves_file(root: &Path) -> PathBuf {
    let reads = session_file(root);
    reads.with_file_name(reads.file_name().unwrap().to_string_lossy().replacen("reads-", "moves-", 1))
}

/// Renames `from` to `to` in one step that never replaces anything at `to`.
fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renameat2(olddirfd: i32, oldpath: *const std::ffi::c_char, newdirfd: i32, newpath: *const std::ffi::c_char, flags: u32) -> i32;
        }
        const AT_FDCWD: i32 = -100;
        const RENAME_NOREPLACE: u32 = 1;
        const EINVAL: i32 = 22;
        let (a, b) = (std::ffi::CString::new(from.as_os_str().as_bytes())?, std::ffi::CString::new(to.as_os_str().as_bytes())?);
        // SAFETY: both are NUL-terminated paths that live through the call.
        if unsafe { renameat2(AT_FDCWD, a.as_ptr(), AT_FDCWD, b.as_ptr(), RENAME_NOREPLACE) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        // A file system without RENAME_NOREPLACE falls through: the project lock keeps the check below sound.
        if e.raw_os_error() != Some(EINVAL) {
            return Err(e);
        }
    }
    if fs::symlink_metadata(to).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}

/// Moves a file or folder from `from` in project `src` to `to` in project `dst`
/// (D-84), whole or not at all: everything is checked before anything changes,
/// and the move is one rename that never replaces what is at `to`. Repeating a
/// move this session made changes nothing.
pub fn move_files(src: &Path, sv: &toml::Value, from: &str, dst: &Path, dv: &toml::Value, to: &str) -> Out {
    let (from, to) = (file_name(Path::new(from)), file_name(Path::new(to)));
    if from.is_empty() || to.is_empty() {
        return error("E6002", "move", "name a file or folder inside the project on both sides");
    }
    let (p, q) = (reachable(src, &from)?, reachable(dst, &to)?);
    let same = same_project(src, dst);
    let shown = |root: &Path, name: &str| if same { name.to_string() } else { format!("{}: {name}", scope::dir(&scope::top(root), root)) };
    let (shown_from, shown_to) = (shown(src, &from), shown(dst, &to));
    let intent = json!({"to": q.to_string_lossy()});
    let mut moves: Value = fs::read(moves_file(src)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if fs::symlink_metadata(&p).is_err() {
        if fs::symlink_metadata(&q).is_ok() && moves[from.as_str()] == intent {
            return Ok(format!("Already moved {shown_from} to {shown_to}; nothing changed\n"));
        }
        return error("E6002", &from, "nothing to move: no file or folder there");
    }
    if fs::symlink_metadata(&q).is_ok() {
        return error("E6002", &to, "something already exists there; a move never replaces a file or folder");
    }
    if same && to.starts_with(&format!("{from}/")) {
        return error("E6002", &to, "a folder can't move into itself");
    }
    let mut files = vec![];
    moved_files(&from, &p, "", &mut files)?;
    for f in &files {
        movable(src, sv, &joined(&from, f))?;
        movable(dst, dv, &joined(&to, f))?;
    }
    // Checked; from here the move happens whole or not at all. Undo records go
    // first, so a crash after the rename still records it (D-77 point 7).
    let mut records = vec![];
    for f in &files {
        let bytes = fs::read(if f.is_empty() { p.clone() } else { p.join(f) }).ok();
        records.extend(undo_record(src, &joined(&from, f), bytes)?);
        records.extend(undo_record(dst, &joined(&to, f), None)?);
    }
    moves[from.as_str()] = intent;
    write_atomic(&moves_file(src), moves.to_string())?;
    // Folders made for the destination go again if the rename fails.
    let mut made = vec![];
    let mut parent = q.parent().unwrap().to_path_buf();
    while fs::symlink_metadata(&parent).is_err() {
        made.push(parent.clone());
        parent.pop();
    }
    if let Err(e) = fs::create_dir_all(q.parent().unwrap()).and_then(|_| rename_new(&p, &q)) {
        for d in &made {
            let _ = fs::remove_dir(d);
        }
        for r in &records {
            fs::remove_file(r)?;
        }
        moves.as_object_mut().unwrap().remove(&from);
        write_atomic(&moves_file(src), moves.to_string())?;
        const EXDEV: i32 = 18;
        return match e.kind() {
            std::io::ErrorKind::AlreadyExists => error("E6002", &to, "something already exists there; a move never replaces a file or folder"),
            _ if e.raw_os_error() == Some(EXDEV) => error("E6002", &to, "the two places are on different file systems, so the move can't be one step"),
            _ => Err(e.into()),
        };
    }
    // What this session read keeps counting as read under the new name.
    let mut old = reads(src);
    let carried: Vec<(String, Value)> =
        files.iter().filter_map(|f| Some((joined(&to, f), old.as_object_mut()?.remove(&joined(&from, f))?))).collect();
    if !carried.is_empty() {
        if same {
            for (k, h) in carried {
                old[k.as_str()] = h;
            }
        } else {
            let mut new = reads(dst);
            for (k, h) in carried {
                new[k.as_str()] = h;
            }
            fs::write(session_file(dst), new.to_string())?;
        }
        fs::write(session_file(src), old.to_string())?;
    }
    let what = if files.len() == 1 && files[0].is_empty() { String::new() } else { format!(" ({})", count(files.len(), "file")) };
    Ok(format!("Moved {shown_from} to {shown_to}{what}\n"))
}

pub fn dispatch(
    store: &Store,
    v: &toml::Value,
    a: &[&str],
    input: &mut dyn FnMut() -> Out,
    old: Option<&str>,
    budget: usize,
) -> Option<Out> {
    let root = &store.root;
    if a[0] == "grep" {
        return Some(grep(root, v, &a[1..].join(" ")));
    }
    if a[0] == "move" {
        return Some((|| {
            let (from, to, dest) = move_args(root, a)?;
            let dv = if same_project(root, &dest) { v.clone() } else { settings(&dest)? };
            move_files(root, v, from, &dest, &dv, to)
        })());
    }
    if matches!(a[0], "files" | "read" | "write" | "remove") {
        // D-102: `files` and `read` take a list of paths, answered in order;
        // `write` and `remove` change one file and refuse a list.
        // A `--lines` or `--tail` range belongs to a single path.
        let mut paths = vec![];
        let mut flags = vec![];
        let mut i = 1;
        while i < a.len() {
            if a[i] == "--lines" || a[i] == "--tail" {
                let next = a.get(i + 1).copied().unwrap_or("");
                flags.extend([a[i], next]);
                i += 2;
            } else if a[i].starts_with("--") {
                flags.push(a[i]);
                i += 1;
            } else {
                paths.push(a[i]);
                i += 1;
            }
        }
        // `write` with no path takes a patch that changes several files (D-104).
        if a[0] == "write" && paths.is_empty() {
            return Some((|| {
                let raw = input()?;
                if !flags.is_empty() || !drafts::is_patch(&raw) {
                    return usage(
                        "write with no path takes a file patch in `text`: `*** Begin Patch`, then `*** Add File: <path>`, `*** Update File: <path>` or `*** Delete File: <path>` sections; `write <path>` changes one file",
                    );
                }
                write_patch(root, v, &raw).map(|out| trim_output(&out, budget))
            })());
        }
        if paths.is_empty() {
            paths.push(".");
        }
        return Some((|| {
            if paths.len() > 1 && a[0] == "write" {
                return usage("write <path> changes one file; for several, `write` with no path takes a file patch (`*** Update File: <path>` sections)");
            }
            if paths.len() > 1 && !flags.is_empty() {
                return usage("a --lines or --tail range takes one path: read <path> --lines 3,7,10..12");
            }
            // D-109: a path that doesn't exist is named before anything is read.
            if a[0] == "read" {
                for name in &paths {
                    let name = file_name(Path::new(name));
                    if !reachable(root, &name)?.exists() && tier(root, v, &name, None) != "graph" {
                        return missing(root, &name, &paths);
                    }
                }
            }
            // Several files share the reply: each is headed with its path, and
            // the paths that don't fit are named for the next read.
            let many = paths.len() > 1;
            let mut out = String::new();
            // `remove` checks every path before it removes any: whole or nothing.
            let (mut doomed, mut refused) = (vec![], vec![]);
            for (k, name) in paths.iter().enumerate() {
                if a[0] == "read" && many && budget.saturating_sub(out.len()) < 400 {
                    out += &format!("Next: read {}\n", paths[k..].join(" "));
                    break;
                }
                let name = file_name(Path::new(name));
                let p = reachable(root, &name)?;
                let t = tier(root, v, &name, None);
                if a[0] != "files" && t == "graph" {
                    let module = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/")).unwrap().trim_end_matches(".gleam");
                    let fix = if p.exists() {
                        format!("inspect {module}, view {module}, or submit {module}")
                    } else {
                        format!("create the module by adding a symbol to it: submit --new {module}")
                    };
                    let e = error("E6001", &name, format!("source is held as symbols; fix: {fix}"));
                    if a[0] == "remove" {
                        refused.extend(e.err());
                        continue;
                    }
                    return e;
                }
                match a[0] {
                    "files" => {
                        if p.is_dir() {
                            tree(root, v, &p, &mut out)?;
                        } else {
                            out += &format!("{name} [{t}]\n");
                        }
                    }
                    "read" => {
                        if many {
                            out += &format!("==== {name} ====\n");
                        }
                        let (text, cut) = read_part(root, &name, &flags, if many { budget.saturating_sub(out.len() + 200) } else { budget })?;
                        out += &text;
                        if cut && k + 1 < paths.len() {
                            out += &format!("Next: read {}\n", paths[k + 1..].join(" "));
                            break;
                        }
                    }
                    "write" => {
                        if t == "protected" {
                            return error("E6002", &name, "protected files change only through a human-approved proposal: in a work run, `write` it with the exact new text");
                        }
                        return write(root, v, &name, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old);
                    }
                    "remove" => {
                        if name == "gleam.toml" || (t != "watched" && !t.starts_with("unwatched")) {
                            refused.extend(
                                error::<()>("E6002", &name, format!("only watched and unwatched files can be removed, and never gleam.toml; {t}")).err(),
                            );
                        } else if !p.is_file() {
                            refused.extend(error::<()>("E6002", &name, "no such file").err());
                        } else {
                            doomed.push((name, p));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            if let [one] = &refused[..]
                && paths.len() == 1
            {
                return Err(Failure(one.0.clone(), one.1));
            }
            if !refused.is_empty() {
                let all: Vec<String> = refused.iter().map(|f| f.0.clone()).collect();
                return Err(Failure(format!("nothing removed:\n{}", all.join("\n")), refused[0].1));
            }
            for (name, p) in doomed {
                if let Some((tid, rid)) = board::context() {
                    fs::create_dir_all(root.join(".solace/pending"))?;
                    let record = root.join(".solace/pending").join(format!("{}.json", sha256(name.as_bytes())));
                    // An earlier change to this file in the command keeps its record: it holds the original bytes.
                    if !same_run(&record, &json!(rid)) {
                        write_atomic(&record, json!({"task":tid,"run":rid,"path":name,"bytes":fs::read(&p).ok()}).to_string())?;
                    }
                }
                fs::remove_file(p)?;
                out += &format!("Removed {name}\n");
            }
            Ok(trim_output(&out, budget))
        })());
    }
    if matches!(a[0], "build" | "format" | "run") {
        return Some((|| {
            let (argv, timeout) = if a[0] == "run" {
                let name = a.get(1).copied().unwrap_or("");
                let commands = v.get("commands").and_then(|x| x.as_table());
                let Some(c) = commands.and_then(|c| c.get(name)) else {
                    return error(
                        "E6010",
                        name,
                        format!(
                            "declared commands: {}; to add one, `write solace.toml` in a work run proposes it for approval",
                            commands.map(|c| c.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()
                        ),
                    );
                };
                if a.len() > 2 && c.get("args").and_then(|x| x.as_bool()) != Some(true) {
                    return error("E6011", name, "this command does not allow extra arguments");
                }
                let mut argv: Vec<String> = c["run"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()).collect();
                argv.extend(a[2..].iter().map(|s| s.to_string()));
                (argv, c.get("timeout").and_then(|x| x.as_integer()).unwrap_or(120) as u64)
            } else if !root.join("gleam.toml").exists() {
                return plain(a[0]);
            } else {
                (vec!["gleam".into(), a[0].into()], 120)
            };
            // A declared run writes one undo record with its whole snapshot of
            // watched files before the process runs (D-77 point 7).
            if let Some((tid, rid)) = board::context() {
                fs::create_dir_all(root.join(".solace/pending"))?;
                let files: Vec<Value> = watched_paths(root, v)?
                    .into_iter()
                    .map(|name| json!({"path": name, "bytes": fs::read(root.join(&name)).ok()}))
                    .collect();
                write_atomic(&root.join(".solace/pending").join(format!("run-{rid}.json")), json!({"task":tid,"run":rid,"files":files}).to_string())?;
            }
            store.materialize(&store.index()?)?;
            let out = command(root, &argv, timeout, budget)?;
            if out.starts_with("exit code: 0\n") { Ok(out) } else { fail(out) }
        })());
    }
    None
}

pub fn agent_config(agent: &str, dir: &Path, binary: &Path, timeout: u64) -> Out {
    fs::create_dir_all(dir)?;
    let binary = binary.to_string_lossy();
    let launch = match agent {
        "codex" => {
            // Codex calls tools from JavaScript (`exec`); one script can batch several
            // solace replies, so its output limit is raised past Codex's default 10k tokens.
            let hint = "The tool is the MCP tool `solace`; from `exec`, call it as \
                `await tools.mcp__solace__solace({command: \"session start --task\", text: \"<the task>\"})`, \
                with the new code in `text` for submit commands. One `exec` script can make several \
                solace calls, so read everything you need in one step.";
            let mut config = format!(
                "sandbox_mode = \"read-only\"\napproval_policy = \"never\"\nweb_search = \"live\"\ntool_output_token_limit = 30000\ndeveloper_instructions = {}\n[features]\n",
                json!(hint)
            );
            for f in [
                "shell_tool",
                "unified_exec",
                "apply_patch_freeform",
                "multi_agent",
                "apps",
                "plugins",
                "goals",
                "image_generation",
                "view_image",
                "browser_use",
                "computer_use",
                "in_app_browser",
                "hooks",
                "memories",
                "tool_suggest",
            ] {
                config += &format!("{f} = false\n");
            }
            config += &format!(
                "[mcp_servers.solace]\ncommand = {}\nargs = [\"mcp\", \"--budget\", \"32000\"]\nenv_vars = [\"SOLACE_TASK\", \"SOLACE_RUN\", \"SOLACE_SESSION\", \"SOLACE_CONFIG\"]\ndefault_tools_approval_mode = \"approve\"\ntool_timeout_sec = {}\n",
                json!(binary),
                timeout.max(3600)
            );
            fs::write(dir.join("config.toml"), config)?;
            json!({"args":[],"env":{"CODEX_HOME":dir},"config":"config.toml"})
        }
        "claude" => {
            fs::write(
                dir.join("mcp.json"),
                json!({"mcpServers":{"solace":{"command":binary,"args":["mcp","--budget","40000"]}}}).to_string(),
            )?;
            fs::write(dir.join("settings.json"),json!({"disableAllHooks":true,"permissions":{"allow":["mcp__solace__solace","WebFetch","WebSearch"],"deny":["Bash","Read","Write","Edit","Glob","Grep","Agent","Task"]},"enabledPlugins":{}}).to_string())?;
            json!({"args":["--strict-mcp-config","--mcp-config",dir.join("mcp.json"),"--settings",dir.join("settings.json"),"--setting-sources","","--disable-slash-commands","--tools","WebFetch,WebSearch","--allowedTools","mcp__solace__solace","WebFetch","WebSearch"],"env":{"MCP_TOOL_TIMEOUT":(timeout.max(3600)*1000).to_string()},"config":"settings.json"})
        }
        _ => return usage("agent-config codex|claude <dir>"),
    };
    fs::write(dir.join("launch.json"), launch.to_string())?;
    Ok(format!("{launch}\n"))
}

/// How `grep` reads its query, for the hint after a miss that looks like a shell habit (D-109).
pub const GREP_SYNTAX: &str = "grep takes literal text, already case-insensitive, searched as written; quotes around the whole query are dropped; no options or regex; `--project <folder>` picks the project";

/// `grep <text>`: case-insensitive search of the files Solace watches, and the
/// protected settings. Source symbols are `find`'s; unwatched files aren't
/// searched (D-52). The text is searched as written, never refused (D-109).
fn grep(root: &Path, v: &toml::Value, query: &str) -> Out {
    let raw = query.trim();
    let query = one_quoted(raw);
    if query.is_empty() {
        return Err(Failure("grep: give the text to search for\n\nRun `solace help` for usage.".into(), 2));
    }
    let q = query.to_lowercase();
    let mut rows = vec![];
    tree_rows(root, v, root, &mut rows)?;
    let mut hits = vec![];
    for (name, _) in rows.into_iter().filter(|(_, t)| t == "watched" || t == "protected") {
        let Ok(text) = fs::read_to_string(root.join(&name)) else { continue };
        let lines: Vec<String> = text
            .lines()
            .enumerate()
            .filter_map(|(n, l)| Some(format!("  {:>4}: {}", n + 1, around(l.trim(), l.trim().to_lowercase().find(&q)?, q.len()))))
            .collect();
        if !lines.is_empty() {
            let more = if lines.len() > 6 { format!("\n  … {} more lines", lines.len() - 6) } else { String::new() };
            hits.push(format!("{name}\n{}{more}\n", lines[..lines.len().min(6)].join("\n")));
        }
    }
    if hits.is_empty() {
        return Ok(format!("No matches for \"{query}\" in watched files.\n{}", grep_hint(raw, query)));
    }
    let total = hits.len();
    let mut out: String = hits.into_iter().take(40).collect();
    if total > 40 {
        out += &format!("… {} more files match; narrow the search.\n", total - 40);
    }
    Ok(out)
}
/// A query without the quotes around it, only when it is one quoted term:
/// `"write" =>` keeps its quotes.
fn one_quoted(query: &str) -> &str {
    ['"', '\'', '`']
        .iter()
        .find_map(|&c| query.strip_prefix(c)?.strip_suffix(c).filter(|q| !q.is_empty() && !q.contains(c)))
        .unwrap_or(query)
}
/// D-109: after a miss, one line for a query that looks like a shell habit: an
/// option, `\|`, an escaped quote, a quoted term with more after it, or a path
/// filter at the end. A plain miss gets nothing.
fn grep_hint(raw: &str, query: &str) -> String {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let n = words.len();
    let option = raw.strip_prefix('-').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_alphabetic() || c == '-'));
    let quoted = ['"', '\''].iter().any(|&c| raw.starts_with(c) && raw[1..].find(c).is_some_and(|i| i + 2 < raw.len()));
    // A path: only path characters, and a folder (`src/a`) or a file extension
    // (`a.rs`); code such as `a.len` or `x /` is not one.
    let path = |w: &str| {
        let w = w.trim_matches(|c| c == '"' || c == '\'');
        w.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-./".contains(&c))
            && (w.split('/').filter(|s| !s.is_empty()).count() > 1
                || w.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && FILE_EXTENSIONS.contains(&ext)))
    };
    let filter = n >= 2 && (matches!(words[n - 2], "--path" | "--in") || path(words[n - 1]));
    if !(option || quoted || filter || raw.contains("\\|") || raw.contains("\\\"")) {
        return String::new();
    }
    let filter = if filter { " grep has no path filter; it searches every watched file." } else { "" };
    format!("Searched for: {query}.{filter} {GREP_SYNTAX}\n")
}
/// Extensions that make a word a file path for `grep_hint`.
const FILE_EXTENSIONS: &[&str] =
    &["rs", "gleam", "md", "txt", "toml", "json", "js", "mjs", "ts", "py", "html", "css", "erl", "sh", "yml", "yaml", "lock", "csv"];
/// A long line cut to about 200 characters around a match at byte `at`
/// (lowercasing can shift bytes, so cuts snap to character boundaries).
fn around(line: &str, at: usize, len: usize) -> String {
    if line.len() <= 200 {
        return line.to_string();
    }
    let floor = |i: usize| (0..=i.min(line.len())).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
    let (start, end) = (floor(at.saturating_sub(80)), floor(at + len + 120));
    format!("{}{}{}", if start > 0 { "…" } else { "" }, &line[start..end], if end < line.len() { "…" } else { "" })
}
pub fn tree_files(root: &Path, settings: &toml::Value) -> Result<Vec<(String, String)>, Failure> {
    let mut rows = vec![];
    tree_rows(root, settings, root, &mut rows)?;
    Ok(rows.into_iter().filter(|(_, tier)| !tier.is_empty() && tier != "internal" && tier != "graph").collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;
    fn fixture() -> (Project, toml::Value) {
        let dir = Scratch::new()
            .with(&[("gleam.toml", "name = \"fixture\"\nversion = \"1.0.0\"\n"), ("src/main.gleam", "pub fn main() { 1 }\n")]);
        fs::create_dir_all(dir.join("priv")).unwrap();
        let v = "watch_limit = 100\nunwatched = [\"priv/generated/\"]\n".parse().unwrap();
        (dir.import(), v)
    }
    fn call(s: &Store, v: &toml::Value, args: &[&str], text: &str, old: Option<&str>) -> Out {
        dispatch(s, v, args, &mut || Ok(text.into()), old, 1000).unwrap()
    }
    #[test]
    fn a1_tiers_golden() {
        let (s, v) = fixture();
        for (p, b) in [
            ("priv/a.html", b"ok".as_slice()),
            ("priv/b.bin", b"\0"),
            ("priv/large", &[b'x'; 101]),
            ("priv/ignored", b"x"),
            ("manifest.toml", b"x"),
            ("solace.toml", b""),
            ("AGENTS.md", b"x"),
            ("misc", b"x"),
            (".gitignore", b"priv/ignored\n"),
            ("priv/generated/out", b"x"),
            (".git/config", b"x"),
            ("node_modules/pkg/index.js", b"x"),
            ("sub/node_modules/pkg/index.js", b"x"),
            ("sub/.gitignore", b"x"),
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let got = call(&s, &v, &["files"], "", None).unwrap();
        assert_eq!(
            got,
            ".git/ [unwatched; 1 files]\n.gitignore [protected]\n.solace/ [internal]\nAGENTS.md [protected]\ngleam.toml [watched]\nmanifest.toml [unwatched]\nmisc [watched]\nnode_modules/ [unwatched; 1 files]\npriv/\npriv/a.html [watched]\npriv/b.bin [unwatched (binary)]\npriv/generated/ [unwatched; 1 files]\npriv/ignored [unwatched]\npriv/large [unwatched (over limit)]\nsolace.toml [protected]\nsrc/\nsrc/main.gleam [graph]\nsub/\nsub/.gitignore [protected]\nsub/node_modules/ [unwatched; 1 files]\n"
        );
    }
    #[test]
    fn grep_searches_watched_files_only() {
        let (s, _) = fixture();
        let v = "watch_limit = 2000\nunwatched = [\"priv/generated/\"]\n".parse().unwrap();
        let long = format!("{}needle é{}", "word ".repeat(60), " tail".repeat(60));
        for (p, b) in [
            ("docs/a.md", format!("intro\nThe Needle here\n{long}\n")),
            ("solace.toml", "# needle in settings\n".into()),
            ("priv/generated/out", "needle\n".into()),
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        fs::write(s.root.join("src/main.gleam"), "pub fn main() { \"needle\" }\n").unwrap();
        let got = call(&s, &v, &["grep", "\"needle\""], "", None).unwrap();
        // Watched and protected files, with line numbers; never source or unwatched files.
        assert!(got.starts_with("docs/a.md\n     2: The Needle here\n     3: …"), "{got}");
        assert!(got.contains("needle é") && got.contains("solace.toml\n     1: # needle in settings"), "{got}");
        assert!(!got.contains("generated") && !got.contains("main.gleam"), "{got}");
        assert!(got.lines().nth(3).unwrap().len() < 260, "{got}");
        // A plain miss gets no hint.
        assert_eq!(call(&s, &v, &["grep", "absent"], "", None).unwrap(), "No matches for \"absent\" in watched files.\n");
        assert!(call(&s, &v, &["grep", "\"The", "Needle\""], "", None).unwrap().starts_with("docs/a.md"));
    }
    /// D-109: grep searches code text as written, quotes included unless they
    /// wrap the whole query, and a miss that looks like a shell habit gets a hint.
    #[test]
    fn grep_searches_code_text_and_hints_after_a_misuse() {
        let (s, _) = fixture();
        let v = "watch_limit = 2000\n".parse().unwrap();
        fs::create_dir_all(s.root.join("docs")).unwrap();
        fs::write(s.root.join("docs/code.rs"), "\"write\" => go(),\nlet x = \"write\" | y;\nfn f() -> Result<(), E>\n| a \\| b |\n").unwrap();
        for (q, line) in [
            (&["grep", "\"write\"", "=>"][..], 1),
            (&["grep", "\"write\"", "|"], 2),
            (&["grep", "->", "Result"], 3),
            (&["grep", "a", "\\|", "b"], 4),
        ] {
            let got = call(&s, &v, q, "", None).unwrap();
            assert!(got.starts_with("docs/code.rs\n") && got.contains(&format!("     {line}: ")), "{q:?}: {got}");
        }
        // One quoted term is searched without its quotes.
        let got = call(&s, &v, &["grep", "\"write\""], "", None).unwrap();
        assert!(got.contains("     1: ") && got.contains("     2: "), "{got}");
        for (q, filter) in [
            (&["grep", "-i", "needle"][..], false),
            (&["grep", "\"x\"", "src/a.rs"], true),
            (&["grep", "foo", "--path", "src"], true),
        ] {
            let got = call(&s, &v, q, "", None).unwrap();
            assert!(got.starts_with("No matches") && got.contains("Searched for: ") && got.contains("literal text, already case-insensitive"), "{q:?}: {got}");
            assert_eq!(got.contains("no path filter"), filter, "{q:?}: {got}");
        }
        assert!(call(&s, &v, &["grep", "-i", "needle"], "", None).unwrap().contains("Searched for: -i needle."));
        // Code at the end of a query is not taken for a path: a plain miss.
        for q in [&["grep", "let", "n", "=", "a.len"][..], &["grep", "x", "=", "a", "/"], &["grep", "self.name"]] {
            assert_eq!(call(&s, &v, q, "", None).unwrap(), format!("No matches for \"{}\" in watched files.\n", q[1..].join(" ")), "{q:?}");
        }
        // A folder or a file is: the hint says grep has no path filter.
        for q in [&["grep", "foo", "src/app"][..], &["grep", "foo", "notes.md"]] {
            assert!(call(&s, &v, q, "", None).unwrap().contains("no path filter"), "{q:?}");
        }
    }
    /// D-109: `write <path>` refuses patch text and a drastic shrink, and its
    /// reply gives the file's lines before and after.
    #[test]
    fn write_refuses_a_patch_and_a_drastic_shrink() {
        let (s, _) = fixture();
        let v: toml::Value = "watch_limit = 100000\n".parse().unwrap();
        fs::write(s.root.join("a.txt"), "one\ntwo\n").unwrap();
        call(&s, &v, &["read", "a.txt"], "", None).unwrap();
        let e = call(&s, &v, &["write", "a.txt"], "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+uno\n*** End Patch\n", None).unwrap_err();
        assert!(e.0.contains("this text is a patch; send it with `write` and no path") && e.1 == 2, "{e:?}");
        assert_eq!(fs::read_to_string(s.root.join("a.txt")).unwrap(), "one\ntwo\n");
        assert_eq!(call(&s, &v, &["write", "a.txt"], "one\ntwo\nthree\n", None).unwrap(), "Wrote a.txt (2 → 3 lines)\n");
        assert_eq!(call(&s, &v, &["write", "b.txt"], "x", None).unwrap(), "Wrote b.txt (1 line)\n");
        // A partial read allows a whole write, so a shrink to under half of a long file is refused.
        let long: String = (1..=300).map(|n| format!("line {n}\n")).collect();
        fs::write(s.root.join("long.rs"), &long).unwrap();
        call(&s, &v, &["read", "long.rs", "--lines", "1..5"], "", None).unwrap();
        let e = call(&s, &v, &["write", "long.rs"], "fn main() {}\n", None).unwrap_err().0;
        assert!(e.contains("would cut long.rs from 300 lines to 1") && e.contains("`remove long.rs` first"), "{e}");
        assert_eq!(fs::read_to_string(s.root.join("long.rs")).unwrap(), long);
        let half: String = long.lines().take(150).map(|l| format!("{l}\n")).collect();
        assert_eq!(call(&s, &v, &["write", "long.rs"], &half, None).unwrap(), "Wrote long.rs (300 → 150 lines)\n");
        // To replace it whole: remove it, then write it new.
        call(&s, &v, &["remove", "long.rs"], "", None).unwrap();
        assert_eq!(call(&s, &v, &["write", "long.rs"], "fn main() {}\n", None).unwrap(), "Wrote long.rs (1 line)\n");
    }
    /// D-109: a `read` path that doesn't exist is named with what it likely
    /// meant, and nothing is read.
    #[test]
    fn read_names_a_missing_path() {
        let (s, v) = fixture();
        fs::write(s.root.join("a.rs"), "fn a() {}\n").unwrap();
        for range in ["1340-1400", "1340..1400", "1340:1400"] {
            let e = call(&s, &v, &["read", "a.rs", range], "", None).unwrap_err();
            assert!(e.0.contains(&format!("`{range}` is not a file")) && e.0.contains("`read a.rs --lines 1340..1400`") && e.1 == 2, "{e:?}");
        }
        let e = call(&s, &v, &["read", "a.rs:3-4"], "", None).unwrap_err().0;
        assert!(e.contains("`read a.rs --lines 3..4`"), "{e}");
        let e = call(&s, &v, &["read", "some_fn"], "", None).unwrap_err().0;
        assert!(e.contains("some_fn") && e.contains("not symbols") && e.contains("`grep some_fn`"), "{e}");
        let e = call(&s, &v, &["read", "docs/gone.md"], "", None).unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("docs/gone.md") && e.contains("no such file") && !e.contains("symbols"), "{e}");
    }
    #[test]
    fn a2_source_internal_and_path_aliases() {
        let (s, v) = fixture();
        for cmd in ["read", "write", "remove"] {
            let e = call(&s, &v, &[cmd, "src/main.gleam"], "x", None).unwrap_err();
            assert!(e.0.contains("E6001") && e.0.contains("submit main"));
        }
        for cmd in ["files", "read", "write", "remove"] {
            assert!(call(&s, &v, &[cmd, ".solace/board/x"], "x", None).unwrap_err().0.contains("E6006"));
            // The root's task store (D-77), from the root and from a project inside it.
            assert!(call(&s, &v, &[cmd, ".solace/tasks/x.json"], "x", None).unwrap_err().0.contains("E6006"));
            assert!(call(&s, &v, &[cmd, "../.solace/tasks/x.json"], "x", None).unwrap_err().0.contains("E6006"));
            // The vision (D-91).
            assert!(call(&s, &v, &[cmd, ".solace/strategy/vision.md"], "x", None).unwrap_err().0.contains("E6006"));
        }
        std::os::unix::fs::symlink(s.root.join("src/main.gleam"), s.root.join("priv/link")).unwrap();
        assert!(call(&s, &v, &["write", "priv/link"], "x", None).is_err());
        assert!(call(&s, &v, &["read", "../secret"], "", None).is_err());
    }
    #[test]
    fn a3_large_log_budget_and_tail() {
        let (s, _) = fixture();
        let mut f = fs::File::create(s.root.join("log")).unwrap();
        let block = "123456789\n".repeat(10000);
        for _ in 0..1000 {
            f.write_all(block.as_bytes()).unwrap();
        }
        let tail = read(&s.root, "log", &["--tail", "10"], 1000).unwrap();
        assert_eq!(tail.lines().count(), 10);
        let head = read(&s.root, "log", &[], 1000).unwrap();
        assert!(head.len() <= 1000 && head.contains("Next: read log --lines"));
    }
    #[test]
    fn read_lines_takes_a_range() {
        let (s, _) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\nthree\nfour\n").unwrap();
        let out = read(&s.root, "notes", &["--lines", "2..3"], 1000).unwrap();
        assert!(out.contains("two") && out.contains("three") && !out.contains("one") && !out.contains("four"), "{out}");
        for bad in ["3..2", "0..1", "x", "1,,2", "2,"] {
            assert_eq!(read(&s.root, "notes", &["--lines", bad], 1000).unwrap_err().1, 2, "{bad}");
        }
        // D-105: lines and ranges, in file order, merged, a gap shown as `…`.
        assert_eq!(read(&s.root, "notes", &["--lines", "4,1..2"], 1000).unwrap(), "1: one\n2: two\n…\n4: four\n");
        assert_eq!(read(&s.root, "notes", &["--lines", "2..3,3"], 1000).unwrap(), "2: two\n3: three\n");
        // Cut at the limit, the rest is named in the same form.
        assert_eq!(read(&s.root, "notes", &["--lines", "1,3..4"], 110).unwrap(), "1: one\nNext: read notes --lines 3..4\n");
    }
    /// D-102: `read` and `files` answer a list of paths in order; a `--lines`
    /// or `--tail` range takes one path.
    #[test]
    fn read_and_files_take_several_paths() {
        let (s, v) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\n").unwrap();
        fs::write(s.root.join("log"), "a\nb\n").unwrap();
        let out = call(&s, &v, &["read", "notes", "log"], "", None).unwrap();
        assert!(out.contains("==== notes ====\none\ntwo\n==== log ====\na\nb\n"), "{out}");
        // Files past the budget are named, not cut silently, and stay unread.
        fs::write(s.root.join("big"), "x\n".repeat(1000)).unwrap();
        let out = call(&s, &v, &["read", "big", "notes"], "", None).unwrap();
        assert!(out.len() <= 1000 && out.contains("==== big ====") && out.ends_with("Next: read notes\n"), "{out}");
        assert!(call(&s, &v, &["write", "big"], "y", None).unwrap_err().0.contains("E6005"));
        // `write` changes one file and refuses a list.
        let err = call(&s, &v, &["write", "notes", "log"], "x", None).unwrap_err();
        assert!(err.0.contains("changes one file") && err.1 == 2, "{err:?}");
        // `remove` takes a list, whole or nothing: a refused path removes none.
        let err = call(&s, &v, &["remove", "notes", "gleam.toml", "src/main.gleam", "gone"], "", None).unwrap_err();
        assert!(err.0.starts_with("nothing removed:") && err.0.contains("gleam.toml") && err.0.contains("E6001"), "{err:?}");
        assert!(err.0.contains("gone") && s.root.join("notes").exists(), "{err:?}");
        let out = call(&s, &v, &["remove", "notes", "log"], "", None).unwrap();
        assert_eq!(out, "Removed notes\nRemoved log\n");
        assert!(!s.root.join("notes").exists() && !s.root.join("log").exists());
        fs::write(s.root.join("notes"), "one\ntwo\n").unwrap();
        fs::write(s.root.join("log"), "a\nb\n").unwrap();
        let lines = call(&s, &v, &["read", "notes", "--lines", "2..2"], "", None).unwrap();
        assert!(lines.contains("two") && !lines.contains("one"), "{lines}");
        fs::create_dir_all(s.root.join("priv")).unwrap();
        fs::write(s.root.join("priv/notes.txt"), "x\n").unwrap();
        let many = call(&s, &v, &["files", "src", "priv"], "", None).unwrap();
        assert!(many.contains("src/main.gleam") && many.contains("priv"), "{many}");
        let err = call(&s, &v, &["read", "notes", "log", "--lines", "1..1"], "", None).unwrap_err();
        assert!(err.0.contains("takes one path") && err.1 == 2, "{err:?}");
    }
    /// Command output over the budget keeps its first and last lines.
    #[test]
    fn long_output_keeps_head_and_tail() {
        let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let out = trim_output(&text, 500);
        assert!(out.len() <= 500, "{}", out.len());
        assert!(out.starts_with("line 1\nline 2\n") && out.trim_end().ends_with("line 1000"), "{out}");
        assert!(out.contains(" lines omitted ..."), "{out}");
        assert_eq!(trim_output("short\n", 500), "short\n");
    }
    #[test]
    fn a4_new_watched_file_and_size() {
        let (s, v) = fixture();
        call(&s, &v, &["write", "README.md"], "usage", None).unwrap();
        call(&s, &v, &["write", "bin/coolcalc"], "#!/bin/sh\ngleam run -- \"$@\"\n", None).unwrap();
        assert!(call(&s, &v, &["files", "README.md"], "", None).unwrap().contains("[watched]"));
        assert!(s.find("usage").unwrap().contains("No matches"));
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        fs::write(s.root.join("README.md"), "usage changed").unwrap();
        assert!(call(&s, &v, &["write", "README.md"], "new", None).unwrap_err().0.contains("E6005"));
        call(&s, &v, &["write", "README.md", "--replace"], "new", Some("usage")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("README.md")).unwrap(), "new changed");
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        call(&s, &v, &["write", "README.md"], "replacement", None).unwrap();
        // Files follow the same replacement rules as symbols (E0105, `--all`).
        call(&s, &v, &["write", "README.md", "--replace"], "two two", Some("replacement")).unwrap();
        assert!(call(&s, &v, &["write", "README.md", "--replace"], "x", Some("two")).unwrap_err().0.contains("E0105 `old` occurs 2 times"));
        call(&s, &v, &["write", "README.md", "--replace", "--all"], "2", Some("two")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("README.md")).unwrap(), "2 2");
        call(&s, &v, &["remove", "bin/coolcalc"], "", None).unwrap();
        assert!(call(&s, &v, &["remove", "gleam.toml"], "", None).unwrap_err().0.contains("E6002"));
        assert!(!s.root.join("bin/coolcalc").exists());
        fs::write(s.root.join("binary"), b"x\xff").unwrap();
        assert!(call(&s, &v, &["write", "binary", "--replace"], "y", Some("x")).unwrap_err().0.contains("UTF-8"));
        assert_eq!(fs::read(s.root.join("binary")).unwrap(), b"x\xff");
        assert!(call(&s, &v, &["write", "README.md"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
        call(&s, &v, &["write", "priv/new"], "x", None).unwrap();
        assert!(call(&s, &v, &["write", "priv/large"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
    }
    /// D-84: a file or folder moves whole, never onto anything, and a repeat changes nothing.
    #[test]
    fn move_takes_files_and_folders_and_never_replaces() {
        let (s, v) = fixture();
        for (p, b) in [("priv/a", "a"), ("priv/b", "b"), ("docs/x/one", "1"), ("docs/x/deep/two", "2")] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let mv = |from: &str, to: &str| call(&s, &v, &["move", from, to], "", None);
        assert_eq!(mv("priv/a", "new/dir/a").unwrap(), "Moved priv/a to new/dir/a\n");
        assert_eq!(fs::read_to_string(s.root.join("new/dir/a")).unwrap(), "a");
        assert!(!s.root.join("priv/a").exists());
        assert!(mv("priv/a", "new/dir/a").unwrap().starts_with("Already moved priv/a to new/dir/a"));
        // Onto an existing file or folder: refused, and both stay as they were.
        let e = mv("priv/b", "new/dir/a").unwrap_err().0;
        assert!(e.contains("E6002") && e.contains("already exists"), "{e}");
        assert!(mv("priv/b", "docs").unwrap_err().0.contains("already exists"));
        assert_eq!(fs::read_to_string(s.root.join("priv/b")).unwrap(), "b");
        assert_eq!(fs::read_to_string(s.root.join("new/dir/a")).unwrap(), "a");
        assert_eq!(mv("docs/x", "notes/x").unwrap(), "Moved docs/x to notes/x (2 files)\n");
        assert_eq!(fs::read_to_string(s.root.join("notes/x/deep/two")).unwrap(), "2");
        assert!(!s.root.join("docs/x").exists());
        assert!(mv("notes", "notes/inner").unwrap_err().0.contains("into itself"));
        assert!(mv("nothing", "else").unwrap_err().0.contains("nothing to move"));
        assert_eq!(call(&s, &v, &["move", "priv/b"], "", None).unwrap_err().1, 2);
    }
    /// D-84: graph, protected and internal files, gleam.toml and symlinks stay
    /// put, named directly or inside a folder, and a refused move changes nothing.
    #[test]
    fn move_refuses_what_isnt_the_agents() {
        let (s, v) = fixture();
        for (p, b) in [("AGENTS.md", "x"), ("priv/a", "a"), ("box/.gitignore", "x"), ("box/keep", "k"), ("inner/.solace/store.db", "")] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        fs::create_dir_all(s.root.join("links")).unwrap();
        std::os::unix::fs::symlink(s.root.join("priv/a"), s.root.join("links/l")).unwrap();
        for (from, to, code) in [
            ("src/main.gleam", "priv/main.gleam", "E6001"),
            ("priv/a", "src/a.gleam", "E6001"),
            ("src", "lib", "E6001"),
            ("AGENTS.md", "priv/agents", "E6002"),
            ("priv/a", "AGENTS2/.gitignore", "E6002"),
            ("box", "crate", "E6002"),
            ("gleam.toml", "priv/g", "E6002"),
            ("links", "elsewhere", "E6002"),
            ("priv/a", ".solace/a", "E6006"),
            ("priv/a", "../a", "E6006"),
            ("inner", "outer", "E6006"),
            ("priv/a", "inner/a", "E6006"),
        ] {
            let e = call(&s, &v, &["move", from, to], "", None).unwrap_err().0;
            assert!(e.contains(code), "{from} -> {to}: {e}");
        }
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "a");
        assert!(s.root.join("src/main.gleam").exists() && s.root.join("box/keep").exists());
        for gone in ["lib", "crate", "elsewhere", "AGENTS2", "outer"] {
            assert!(!s.root.join(gone).exists(), "{gone}");
        }
    }
    /// D-84: a move between projects checks the destination by its own project's rules.
    #[test]
    fn move_between_projects() {
        let ((a, v), (b, w)) = (fixture(), fixture());
        fs::write(a.root.join("priv/a"), "a").unwrap();
        call(&a, &v, &["read", "priv/a"], "", None).unwrap();
        let out = move_files(&a.root, &v, "priv/a", &b.root, &w, "docs/a").unwrap();
        assert!(out.starts_with("Moved ") && out.contains("docs/a"), "{out}");
        assert_eq!(fs::read_to_string(b.root.join("docs/a")).unwrap(), "a");
        assert!(!a.root.join("priv/a").exists());
        // What was read counts as read under its new name.
        call(&b, &w, &["write", "docs/a"], "b", None).unwrap();
        fs::write(a.root.join("priv/c"), "c").unwrap();
        assert!(move_files(&a.root, &v, "priv/c", &b.root, &w, "src/c.gleam").unwrap_err().0.contains("E6001"));
        assert!(move_files(&a.root, &v, "priv/c", &b.root, &w, "docs/a").unwrap_err().0.contains("already exists"));
        assert!(a.root.join("priv/c").exists());
    }
    #[test]
    fn a5_validation_in_place_and_recovery() {
        let (s, _) = fixture();
        let v:toml::Value=r#"[validators]
"priv/**" = ["python3", "-c", "import pathlib,sys; p=pathlib.Path(sys.argv[1]); print('validator output'); assert p.read_text() == 'good' and p.with_name('sibling').read_text() == 'yes'", "{file}"]
"#.parse().unwrap();
        fs::write(s.root.join("priv/sibling"), "yes").unwrap();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        call(&s, &v, &["read", "priv/a"], "", None).unwrap();
        let e = call(&s, &v, &["write", "priv/a"], "bad", None).unwrap_err();
        assert!(e.0.contains("E6003") && e.0.contains("validator output"));
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
        assert!(call(&s, &v, &["write", "priv/new"], "bad", None).is_err());
        assert!(!s.root.join("priv/new").exists());
        call(&s, &v, &["write", "priv/a"], "good", None).unwrap();
        fs::write(s.root.join(".solace/pending/test.json"), json!({"path":"priv/a","bytes":b"original".to_vec()}).to_string()).unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
    }
    /// D-104: `write` with no path takes a file patch that adds, updates and
    /// deletes several files, with no earlier read, whole or nothing.
    #[test]
    fn a_file_patch_writes_several_files_whole_or_nothing() {
        let (s, v) = fixture();
        fs::write(s.root.join("notes"), "one\ntwo\nthree\n").unwrap();
        fs::write(s.root.join("old"), "gone\n").unwrap();
        let patch = "*** Begin Patch\n*** Update File: notes\n@@ one\n-two\n+2\n*** Add File: docs/new.md\n+# New\n+\n+text\n*** Delete File: old\n*** End Patch\n";
        assert_eq!(call(&s, &v, &["write"], patch, None).unwrap(), "Wrote notes\nAdded docs/new.md\nRemoved old\n");
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\n");
        assert_eq!(fs::read_to_string(s.root.join("docs/new.md")).unwrap(), "# New\n\ntext\n");
        assert!(!s.root.join("old").exists());
        // Lines added with no `@@` line go at the end of the file.
        call(&s, &v, &["write"], "*** Begin Patch\n*** Update File: notes\n+four\n*** End Patch\n", None).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\nfour\n");
        // Any refused section writes nothing, and each refusal is named.
        let update = "*** Update File: notes\n-one\n+1\n";
        for (section, why) in [
            ("*** Update File: docs/new.md\n-nope\n+x\n", "E0108 in docs/new.md (hunk at line 6), these lines weren't found: `nope`"),
            ("*** Update File: src/main.gleam\n-x\n+y\n", "E6001"),
            ("*** Add File: solace.toml\n+x\n", "human-approved proposal"),
            ("*** Add File: .solace/x\n+x\n", "E6006"),
            ("*** Add File: docs/new.md\n+x\n", "already exists; change it with `*** Update File`"),
            ("*** Delete File: missing\n", "no such file; create it with `*** Add File`"),
            ("*** Delete File: gleam.toml\n", "can't be removed"),
            ("*** Add File: priv/large\n+", "E6004"),
            ("*** Update File: notes\n-two\n+3\n", "names notes twice"),
        ] {
            let section = if why == "E6004" { format!("{section}{}\n", "x".repeat(101)) } else { section.to_string() };
            let e = call(&s, &v, &["write"], &format!("*** Begin Patch\n{update}{section}*** End Patch\n"), None).unwrap_err().0;
            assert!(e.starts_with("nothing written:\n") && e.contains(why), "{why}: {e}");
            assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "one\n2\nthree\nfour\n", "{why}");
        }
        for (patch, why) in [
            ("*** Begin Patch\n*** Update File: notes\n-one\n+one\n*** End Patch\n", "E0108 the patch changes nothing"),
            ("*** Begin Patch\n*** Update File: notes\n*** Move to: other\n*** End Patch\n", "use `move <from> <to>`"),
            ("*** Begin Patch\n*** Delete File: notes\nx\n*** End Patch\n", "takes no lines"),
            ("*** Begin Patch\n*** Add File: priv/x\nx\n*** End Patch\n", "in `*** Add File: priv/x` every line starts with `+`"),
            ("*** Begin Patch\n*** End Patch\n", "the patch has no `*** Add File: <path>`"),
        ] {
            let e = call(&s, &v, &["write"], patch, None).unwrap_err().0;
            assert!(e.contains(why), "{why}: {e}");
        }
        // With no path, `text` must be a file patch; with a list, `write` points to one.
        let e = call(&s, &v, &["write"], "plain text", None).unwrap_err();
        assert!(e.0.contains("takes a file patch") && e.1 == 2, "{e:?}");
        let e = call(&s, &v, &["write", "notes", "docs/new.md"], "x", None).unwrap_err();
        assert!(e.0.contains("changes one file") && e.0.contains("*** Update File"), "{e:?}");
        // A `submit` patch that names a file other than code says to use `write`.
        let e = drafts::patch_blocks(&s, "*** Begin Patch\n*** Update File: notes\n-one\n+1\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("`notes` (line 2) is no symbol or module; a file other than code changes with `write`"), "{e}");
        let e = drafts::patch_blocks(&s, "*** Begin Patch\n*** Add File: docs/x.md\n+# X\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("`docs/x.md` (line 2) is no symbol or module"), "{e}");
    }
    /// A file patch writes every file before it validates any; one that fails
    /// puts every file back and leaves no journal.
    #[test]
    fn a_file_patch_that_fails_validation_puts_every_file_back() {
        let (s, _) = fixture();
        let v: toml::Value = r#"[validators]
"priv/**" = ["python3", "-c", "import pathlib,sys; p=pathlib.Path(sys.argv[1]); print('validator output'); assert p.read_text() == 'good\\n' and pathlib.Path('notes').read_text() == 'new\\n'", "{file}"]
"#
        .parse()
        .unwrap();
        fs::write(s.root.join("notes"), "old\n").unwrap();
        let patch = |text: &str| format!("*** Begin Patch\n*** Update File: notes\n-old\n+new\n*** Add File: priv/a\n+{text}\n*** End Patch\n");
        let e = call(&s, &v, &["write"], &patch("bad"), None).unwrap_err().0;
        assert!(e.starts_with("nothing written:\nE6003") && e.contains("validator output"), "{e}");
        assert_eq!(fs::read_to_string(s.root.join("notes")).unwrap(), "old\n");
        assert!(!s.root.join("priv/a").exists());
        assert!(fs::read_dir(s.root.join(".solace/pending")).unwrap().next().is_none());
        // The validator sees the other file of the patch already written.
        call(&s, &v, &["write"], &patch("good"), None).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "good\n");
    }
    /// A pending undo record whose file changed: recovery adds the rows (D-77 point 7).
    #[test]
    fn undo_record_recovery_adds_rows_when_the_file_changed() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(s.root.join(".solace/pending/x.json"), json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string()).unwrap();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let rows: Vec<(String, String, String)> = s
            .db
            .prepare("SELECT task,entity,name FROM board_touches WHERE task='t1'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [("t1".to_string(), "file:priv/a".to_string(), "file:priv/a".to_string())]);
        assert_eq!(fs::read(board::before_file(&s.root, "t1", "priv/a")).unwrap(), b"original");
        assert!(!s.root.join(".solace/pending/x.json").exists());
    }
    /// A pending undo record whose file never changed: recovery drops it, adds nothing.
    #[test]
    fn undo_record_recovery_drops_an_unchanged_file() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(s.root.join(".solace/pending/x.json"), json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string()).unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let n: i64 = s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task='t1'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!s.root.join(".solace/pending/x.json").exists());
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
    }
    /// A declared run's record: rows for changed files, and for files the process created.
    #[test]
    fn declared_run_record_adds_rows_for_changed_and_created_files() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "original").unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        // The snapshot lists every watched file, as `run` takes it; one it doesn't list was created.
        let manifest = fs::read(s.root.join("gleam.toml")).unwrap();
        let files = json!([{"path":"gleam.toml","bytes":manifest},{"path":"priv/a","bytes":b"original".to_vec()}]);
        fs::write(s.root.join(".solace/pending/run-r1.json"), json!({"task":"t1","run":"r1","files":files}).to_string()).unwrap();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        fs::write(s.root.join("priv/created"), "new").unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let entities: Vec<String> = s
            .db
            .prepare("SELECT entity FROM board_touches WHERE task='t1' ORDER BY entity")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(entities, ["file:priv/a", "file:priv/created"]);
        assert_eq!(fs::read(board::before_file(&s.root, "t1", "priv/created").with_extension("absent")).unwrap(), b"");
    }
    /// A second change to a file keeps only its own run's record, so a refused
    /// write never deletes the earlier change's record.
    #[test]
    fn same_run_matches_only_its_own_record() {
        let (s, _) = fixture();
        let journal = s.root.join(".solace/pending/x.json");
        assert!(!same_run(&journal, &json!("r1")));
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(&journal, json!({"task":"t1","run":"r1","path":"priv/a","bytes":null}).to_string()).unwrap();
        assert!(same_run(&journal, &json!("r1")));
        assert!(!same_run(&journal, &json!("r2")));
        fs::write(&journal, "{torn").unwrap();
        assert!(!same_run(&journal, &json!("r1")));
    }
    /// A crash after the commit: the rows are already there, recovery adds no duplicate.
    #[test]
    fn recovery_skips_rows_already_committed() {
        let (s, _) = fixture();
        fs::write(s.root.join("priv/a"), "changed").unwrap();
        s.db.execute("INSERT INTO board_touches(task,entity,run,name) VALUES ('t1','file:priv/a','r1','file:priv/a')", []).unwrap();
        fs::create_dir_all(s.root.join(".solace/pending")).unwrap();
        fs::write(s.root.join(".solace/pending/x.json"), json!({"task":"t1","run":"r1","path":"priv/a","bytes":b"original".to_vec()}).to_string()).unwrap();
        recover(&s.root, &s, &[board::lock(&s.root).unwrap()]).unwrap();
        let n: i64 = s.db.query_row("SELECT COUNT(*) FROM board_touches WHERE task='t1'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        assert!(!s.root.join(".solace/pending/x.json").exists());
    }
    #[test]
    fn a6_manifest_validation() {
        let (s, mut v) = fixture();
        v["watch_limit"] = toml::Value::Integer(1000);
        let before = fs::read(s.root.join("gleam.toml")).unwrap();
        call(&s, &v, &["read", "gleam.toml"], "", None).unwrap();
        assert!(call(&s, &v, &["write", "gleam.toml"], "=broken", None).unwrap_err().0.contains("E6003"));
        assert_eq!(fs::read(s.root.join("gleam.toml")).unwrap(), before);
        // The package name follows the rule `new_project` uses.
        assert!(call(&s, &v, &["write", "gleam.toml"], "name=\"1abc\"\nversion=\"1.0.0\"\n", None).unwrap_err().0.contains("E6003"));
        assert_eq!(fs::read(s.root.join("gleam.toml")).unwrap(), before);
        // Dependencies are Gleam's to check, not Solace's.
        call(
            &s,
            &v,
            &["write", "gleam.toml"],
            "name=\"fixture\"\nversion=\"1.0.0\"\n[dependencies]\nmissing={path=\"./does_not_exist\"}",
            None,
        )
        .unwrap();
    }
    #[test]
    fn a7_stale_and_exact_replace() {
        let (s, v) = fixture();
        call(&s, &v, &["write", "priv/a"], "first", None).unwrap();
        call(&s, &v, &["read", "priv/a"], "", None).unwrap();
        fs::write(s.root.join("priv/a"), "first changed").unwrap();
        assert!(call(&s, &v, &["write", "priv/a"], "new", None).unwrap_err().0.contains("E6005"));
        call(&s, &v, &["write", "priv/a", "--replace"], "second", Some("first")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "second changed");
        // `old` without --replace still replaces only that text (an agent wiped a decision log this way).
        call(&s, &v, &["write", "priv/a"], "third", Some("second")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("priv/a")).unwrap(), "third changed");
        // The same for a settings proposal, which is built by `edited` alone.
        assert_eq!(
            edited(b"projects = []\n[commands]\n", "[commands]\nx = 1", false, false, Some("[commands]")).unwrap(),
            "projects = []\n[commands]\nx = 1\n"
        );
        // A tail read doesn't allow a whole-file write.
        fs::write(s.root.join("priv/b"), "one\ntwo\n").unwrap();
        call(&s, &v, &["read", "priv/b", "--tail", "1"], "", None).unwrap();
        assert!(call(&s, &v, &["write", "priv/b"], "new", None).unwrap_err().0.contains("E6005"));
    }
    #[test]
    fn a8_protected_human_only_and_reload() {
        let (s, v) = fixture();
        assert!(call(&s, &v, &["write", "solace.toml"], "", None).is_err());
        fs::write(s.root.join("solace.toml"), "unwatched=[\"docs/**\"]").unwrap();
        call(&s, &settings(&s.root).unwrap(), &["write", "docs/new"], "x", None).unwrap();
        fs::write(s.root.join("solace.toml"), "unwatched=1").unwrap();
        assert!(settings(&s.root).unwrap_err().0.contains("E6020"));
        for (old, fix) in [("resources=[]", "delete it"), ("ignore=[]", "now `unwatched`"), ("resource_limit=1", "now `watch_limit`")] {
            assert!(parse_settings(old).unwrap_err().0.contains(fix));
        }
    }
    #[test]
    fn a9_declared_args_unknown_timeout() {
        let (s, _) = fixture();
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],checks=\"*\"}").unwrap_err().0.contains("E6020"));
        assert!(parse_settings("[commands]\ne2e={run=[\"x\"],checks=[\"*\"]}").is_ok());
        let v: toml::Value = "[commands]\nsmoke={run=[\"sleep\",\"5\"],timeout=1}".parse().unwrap();
        assert!(call(&s, &v, &["run", "smoke", "extra"], "", None).unwrap_err().0.contains("E6011"));
        let e = call(&s, &v, &["run", "nope"], "", None).unwrap_err();
        assert!(e.0.contains("E6010") && e.0.contains("smoke"));
        assert!(call(&s, &v, &["run", "smoke"], "", None).unwrap_err().0.contains("E6012"));
    }
    #[test]
    fn a10_agent_config_golden() {
        let (s, _) = fixture();
        for agent in ["codex", "claude"] {
            let dir = s.root.join(agent);
            agent_config(agent, &dir, Path::new("/bin/solace"), 3600).unwrap();
            let files =
                if agent == "codex" { vec!["config.toml", "launch.json"] } else { vec!["mcp.json", "settings.json", "launch.json"] };
            for file in files {
                let actual = fs::read_to_string(dir.join(file)).unwrap().replace(&dir.to_string_lossy().to_string(), "/agent");
                let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{agent}-{file}"));
                assert_eq!(actual, fs::read_to_string(golden).unwrap());
            }
        }
    }
    #[test]
    fn nested_tiers_and_settings_types() {
        let root = Scratch::new().with(&[
            ("gleam.toml", "name = \"app\"\n"),
            ("src/nested/source.gleam", "pub fn a() { 1 }"),
            ("priv/nested/index.html", "hello"),
        ]);
        let v = toml::Value::Table(Default::default());
        let mut out = String::new();
        tree(&root, &v, &root, &mut out).unwrap();
        assert!(out.contains("src/nested/source.gleam [graph]"));
        assert!(out.contains("priv/nested/index.html [watched]"));
        fs::write(root.join(".gitignore"), "priv/nested/\n").unwrap();
        out.clear();
        tree(&root, &v, &root, &mut out).unwrap();
        assert!(!out.contains("index.html"));
    }
}
