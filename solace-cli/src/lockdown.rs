use super::*;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::{Duration, Instant};

pub fn error<T>(code: &str, at: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    fail(format!("{code} command refused\n  at {at}\n  {message}\n"))
}

/// A Gleam command in a project of plain files (D-60).
pub fn plain<T>(command: &str) -> Result<T, Failure> {
    error("E6013", command, "this project has no Gleam code (no gleam.toml); use files, grep, read, write and run")
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
pub fn recover(root: &Path) -> Result<(), Failure> {
    let pending = root.join(".solace/pending");
    if !pending.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&pending)? {
        let p = entry?.path();
        if p.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let record: Value = serde_json::from_slice(&fs::read(&p)?).map_err(|e| Failure(e.to_string(), 1))?;
        let target = path(root, record["path"].as_str().ok_or_else(|| Failure("invalid pending record".into(), 1))?)?;
        if let Some(a) = record["bytes"].as_array() {
            fs::write(target, json_bytes(a))?;
        } else if target.exists() {
            fs::remove_file(target)?;
        }
        fs::remove_file(p)?;
    }
    Ok(())
}
fn read(root: &Path, name: &str, args: &[&str], budget: usize) -> Out {
    let file = fs::File::open(path(root, name)?)?;
    let (mut start, mut end) = (1usize, usize::MAX);
    let mut reader = BufReader::new(file);
    if let ["--lines", range] = args {
        let (a, b) = range.split_once("..").ok_or_else(|| Failure("expected --lines a..b".into(), 2))?;
        start = a.parse().unwrap_or(0);
        end = b.parse().unwrap_or(0);
        if start == 0 || end < start {
            return usage("invalid line range");
        }
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
        return Ok(trim_output(&text, budget));
    } else if !args.is_empty() {
        return usage("read <path> [--lines a..b | --tail n]");
    }
    let mut out = String::new();
    let mut truncated = false;
    let mut n = 0;
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
        if n < start {
            continue;
        }
        if n > end {
            break;
        }
        let line = String::from_utf8_lossy(&line);
        let text = if args.is_empty() { line.to_string() } else { format!("{n}: {line}") };
        if oversized || out.len() + text.len() + 100 > budget {
            out += &format!("Next: read {name} --lines {n}..{}\n", n + 100);
            truncated = true;
            break;
        }
        out += &text;
    }
    // Watched files are bounded; unwatched logs are never hashed or loaded in full.
    if fs::metadata(root.join(name))?.len() <= WATCH_LIMIT as u64 * 16 {
        remember(root, name, &fs::read(root.join(name))?)?;
    }
    if truncated && out.len() > budget {
        out.truncate(out.floor_char_boundary(budget));
    }
    Ok(out)
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
fn write(root: &Path, v: &toml::Value, name: &str, new: &str, replace: bool, all: bool, old: Option<&str>) -> Out {
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
        return error("E6005", name, "file is unread or changed since last read; read it again or use --replace");
    }
    fs::create_dir_all(root.join(".solace/pending"))?;
    let journal = root.join(".solace/pending").join(format!("{}.json", sha256(name.as_bytes())));
    fs::write(&journal, json!({"path":name,"bytes":before}).to_string())?;
    fs::create_dir_all(p.parent().unwrap())?;
    let result = (|| -> Out {
        fs::write(&p, &text)?;
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
        Ok(format!("Wrote {name}\n"))
    })();
    if result.is_err() {
        recover(root)?;
    } else {
        fs::remove_file(journal)?;
        remember(root, name, text.as_bytes())?;
    }
    result
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
            } else if name == "build" || name == ".git" || patterns(v, "unwatched", &name) || gitignored(root, &name) {
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
    if matches!(a[0], "files" | "read" | "write" | "remove") {
        let name = a.get(1).copied().unwrap_or(".");
        let name = file_name(Path::new(name));
        return Some((|| {
            if Path::new(&name).components().any(|c| c.as_os_str() == ".solace") {
                return error("E6006", &name, "Solace internal files have no agent access");
            }
            // A path out of the project, or into a project nested in it, is another project's.
            if internal(root, &name) || name.split('/').any(|c| c == "..") {
                return error(
                    "E6006",
                    &name,
                    "outside this project: another project's files are reached with `--project <folder>`; \
                     to change a project this task doesn't cover, ask the human with `task ask` rather than putting the change elsewhere",
                );
            }
            let p = path(root, &name)?;
            let t = tier(root, v, &name, None);
            if a[0] != "files" && t == "graph" {
                let module = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/")).unwrap().trim_end_matches(".gleam");
                let fix = if p.exists() {
                    format!("inspect {module}, view {module}, or submit {module}")
                } else {
                    format!("create the module by adding a symbol to it: submit --new {module}")
                };
                return error("E6001", &name, format!("source is held as symbols; fix: {fix}"));
            }
            match a[0] {
                "files" => {
                    let mut out = String::new();
                    if p.is_dir() {
                        tree(root, v, &p, &mut out)?;
                    } else {
                        out = format!("{name} [{t}]\n");
                    }
                    Ok(out)
                }
                "read" => read(root, &name, &a[2.min(a.len())..], budget),
                "write" => {
                    if t == "protected" {
                        return error("E6002", &name, "protected files are human-only; request a decision");
                    }
                    write(root, v, &name, &input()?, a.contains(&"--replace"), a.contains(&"--all"), old)
                }
                "remove" => {
                    if name == "gleam.toml" || (t != "watched" && !t.starts_with("unwatched")) {
                        return error(
                            "E6002",
                            &name,
                            format!("only watched and unwatched files can be removed, and never gleam.toml; {t}"),
                        );
                    }
                    fs::remove_file(p)?;
                    Ok(format!("Removed {name}\n"))
                }
                _ => unreachable!(),
            }
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
                            "declared commands: {}",
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
                json!({"mcpServers":{"solace":{"command":binary,"args":["mcp","--budget","80000"]}}}).to_string(),
            )?;
            fs::write(dir.join("settings.json"),json!({"disableAllHooks":true,"permissions":{"allow":["mcp__solace__solace","WebFetch","WebSearch"],"deny":["Bash","Read","Write","Edit","Glob","Grep","Agent","Task"]},"enabledPlugins":{}}).to_string())?;
            json!({"args":["--strict-mcp-config","--mcp-config",dir.join("mcp.json"),"--settings",dir.join("settings.json"),"--setting-sources","","--disable-slash-commands","--tools","WebFetch,WebSearch","--allowedTools","mcp__solace__solace","WebFetch","WebSearch"],"env":{"MCP_TOOL_TIMEOUT":(timeout.max(3600)*1000).to_string()},"config":"settings.json"})
        }
        _ => return usage("agent-config codex|claude <dir>"),
    };
    fs::write(dir.join("launch.json"), launch.to_string())?;
    Ok(format!("{launch}\n"))
}

/// `grep <text>`: case-insensitive search of the files Solace watches, and the
/// protected settings. Source symbols are `find`'s; unwatched files aren't
/// searched (D-52).
fn grep(root: &Path, v: &toml::Value, query: &str) -> Out {
    let query = unquoted(query);
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
        return Ok(format!("No matches for \"{query}\" in watched files.\n"));
    }
    let total = hits.len();
    let mut out: String = hits.into_iter().take(40).collect();
    if total > 40 {
        out += &format!("… {} more files match; narrow the search.\n", total - 40);
    }
    Ok(out)
}
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
        ] {
            fs::create_dir_all(s.root.join(p).parent().unwrap()).unwrap();
            fs::write(s.root.join(p), b).unwrap();
        }
        let got = call(&s, &v, &["files"], "", None).unwrap();
        assert_eq!(
            got,
            ".git/ [unwatched; 1 files]\n.gitignore [watched]\n.solace/ [internal]\nAGENTS.md [protected]\ngleam.toml [watched]\nmanifest.toml [unwatched]\nmisc [watched]\npriv/\npriv/a.html [watched]\npriv/b.bin [unwatched (binary)]\npriv/generated/ [unwatched; 1 files]\npriv/ignored [unwatched]\npriv/large [unwatched (over limit)]\nsolace.toml [protected]\nsrc/\nsrc/main.gleam [graph]\n"
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
        assert!(call(&s, &v, &["grep", "absent"], "", None).unwrap().starts_with("No matches"));
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
        for bad in ["3..2", "0..1", "x"] {
            assert_eq!(read(&s.root, "notes", &["--lines", bad], 1000).unwrap_err().1, 2, "{bad}");
        }
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
        recover(&s.root).unwrap();
        assert_eq!(fs::read(s.root.join("priv/a")).unwrap(), b"original");
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
