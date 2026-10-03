use super::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::{Duration, Instant};

pub fn error<T>(code: &str, at: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    fail(format!("{code} command refused\n  at {at}\n  {message}\n"))
}

pub fn settings(root: &Path) -> Result<toml::Value, Failure> {
    match fs::read_to_string(root.join("solace.toml")) {
        Ok(s) => parse_settings(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Value::Table(Default::default())),
        Err(e) => Err(e.into()),
    }
}
/// Checks settings text without reading or writing the file.
pub fn parse_settings(s: &str) -> Result<toml::Value, Failure> {
    let v: toml::Value = s.parse().map_err(|e| Failure(format!("E6020 invalid solace.toml\n  {e}"), 1))?;
    let invalid = |s: &str| Failure(format!("E6020 invalid solace.toml\n  {s}"), 1);
    for key in ["resources", "ignore"] {
        if let Some(a) = v.get(key) {
            for p in a.as_array().ok_or_else(|| invalid(key))? {
                let p = p.as_str().ok_or_else(|| invalid(key))?;
                globset::Glob::new(p).map_err(|_| invalid(p))?;
            }
        }
    }
    if v.get("resource_limit").is_some_and(|x| x.as_integer().is_none_or(|n| n < 0)) {
        return Err(invalid("resource_limit"));
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
    v.get("resource_limit").and_then(|x| x.as_integer()).unwrap_or(262144) as usize
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
// Reject aliases, including symlinks inside the root: a resource alias must
// never grant file access to source, settings, or Solace's own records.
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
fn internal(name: &str) -> bool {
    Path::new(name).components().any(|c| c.as_os_str() == ".solace")
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
    if (name.starts_with("src/") || name.starts_with("test/")) && name.ends_with(".gleam") {
        return "1 source".into();
    }
    if name == "gleam.toml" {
        return "2 manifest".into();
    }
    if name == "solace.toml" || name == "AGENTS.md" {
        return "settings".into();
    }
    if internal(name) {
        return "internal".into();
    }
    if name == ".git" || name.starts_with(".git/") || name == "build" || name.starts_with("build/") || name == "manifest.toml" {
        return "4 ignored".into();
    }
    if gitignored(root, name) || patterns(v, "ignore", name) {
        return "4 ignored".into();
    }
    if patterns(v, "resources", name) {
        let size = proposed.map(|b| b.len() as u64).unwrap_or_else(|| fs::metadata(root.join(name)).map_or(0, |m| m.len()));
        if size > limit(v) as u64 {
            return "4 ignored (over limit)".into();
        }
        let bytes = proposed.map(Vec::from).unwrap_or_else(|| fs::read(root.join(name)).unwrap_or_default());
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return "4 ignored (binary)".into();
        }
        return "3 resource".into();
    }
    "4 ignored".into()
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn session_file(root: &Path) -> PathBuf {
    let session = env::var("SOLACE_SESSION")
        .unwrap_or_else(|_| if env::args().any(|a| a == "mcp") { std::process::id().to_string() } else { "cli".into() });
    root.join(".solace").join(format!("reads-{}.json", hash(session.as_bytes())))
}
fn reads(root: &Path) -> Value {
    fs::read(session_file(root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}))
}
fn remember(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Failure> {
    let mut r = reads(root);
    r[name] = json!(hash(bytes));
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
            fs::write(target, a.iter().map(|x| x.as_u64().unwrap() as u8).collect::<Vec<_>>())?;
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
        let text = lines.into_iter().collect::<Vec<_>>().join("\n") + "\n";
        if length <= 262144 * 16 {
            remember(root, name, &fs::read(root.join(name))?)?;
        }
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
    // Resources are bounded; ignored logs are never hashed or loaded in full.
    if fs::metadata(root.join(name))?.len() <= 262144 * 16 {
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
pub fn command(root: &Path, argv: &[String], timeout: u64, budget: usize) -> Out {
    use std::os::unix::process::CommandExt;
    fs::create_dir_all(root.join(".solace"))?;
    let log = root.join(".solace").join(format!("command-{}.log", new_id(Kind::Fn, "", "")));
    let out = fs::File::create(&log)?;
    let mut child =
        Command::new(&argv[0]).args(&argv[1..]).current_dir(root).process_group(0).stdout(out.try_clone()?).stderr(out).spawn()?;
    let start = Instant::now();
    let result = loop {
        if let Some(status) = child.try_wait()? {
            break Ok(format!("exit code: {}\n", status.code().unwrap_or(-1)));
        }
        if start.elapsed() >= Duration::from_secs(timeout) {
            let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
            let _ = child.kill();
            let _ = child.wait();
            break error("E6012", &argv[0], format!("timed out after {timeout}s"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = fs::read_to_string(&log)?;
    fs::remove_file(log)?;
    match result {
        Ok(prefix) => Ok(prefix.clone() + &trim_output(&output, budget.saturating_sub(prefix.len()))),
        Err(Failure(message, code)) => Err(Failure(message + &trim_output(&output, budget), code)),
    }
}
/// The file's new text: `new` whole, or with `old` replaced by `new`.
pub fn edited(current: &[u8], new: &str, replace: bool, old: Option<&str>) -> Out {
    if !replace {
        return Ok(new.to_string());
    }
    let old = old.filter(|s| !s.is_empty()).ok_or_else(|| Failure("E0104 expected nonempty old text".into(), 1))?;
    let current = std::str::from_utf8(current).map_err(|_| Failure("E6002 text replacement requires a UTF-8 file".into(), 1))?;
    match current.matches(old).count() {
        1 => Ok(current.replacen(old, new, 1)),
        0 => fail("E0104 old text not found"),
        _ => fail("E0105 old text occurs more than once"),
    }
}
fn write(root: &Path, v: &toml::Value, name: &str, new: &str, replace: bool, old: Option<&str>) -> Out {
    let p = path(root, name)?;
    let before = fs::read(&p).ok();
    let text = edited(before.as_deref().unwrap_or_default(), new, replace, old)?;
    if text.len() > limit(v) {
        return error("E6004", name, "proposed text exceeds resource_limit");
    }
    let t = tier(root, v, name, Some(text.as_bytes()));
    if !matches!(t.chars().next(), Some('2' | '3' | '4')) {
        return error("E6002", name, format!("caught by tier rule: {t}"));
    }
    if !replace
        && let Some(ref b) = before
        && reads(root)[name].as_str() != Some(&hash(b))
    {
        return error("E6005", name, "file is unread or changed since last read; read it again or use --replace");
    }
    fs::create_dir_all(root.join(".solace/pending"))?;
    let journal = root.join(".solace/pending").join(format!("{}.json", hash(name.as_bytes())));
    fs::write(&journal, json!({"path":name,"bytes":before}).to_string())?;
    fs::create_dir_all(p.parent().unwrap())?;
    let result = (|| -> Out {
        fs::write(&p, &text)?;
        if name == "gleam.toml" {
            let manifest = text.parse::<toml::Value>().map_err(|e| Failure(format!("E6003 invalid manifest\n  {e}"), 1))?;
            if manifest
                .get("name")
                .and_then(|x| x.as_str())
                .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
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
fn tree_rows(
    root: &Path,
    v: &toml::Value,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), Failure> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
        let t = tier(root, v, &name, None);
        if e.file_type()?.is_symlink() {
            out.push((name, "4 ignored (symlink)".into()));
        } else if e.file_type()?.is_dir() {
            if t == "internal" {
                out.push((format!("{name}/"), "internal".into()));
            } else if name == "build"
                || name == ".git"
                || patterns(v, "ignore", &name)
                || gitignored(root, &name)
            {
                out.push((
                    format!("{name}/"),
                    format!("4 ignored; {} files", count_files(&p)?),
                ));
            } else {
                let mut children = vec![];
                tree_rows(root, v, &p, &mut children)?;
                if children.iter().any(|(_, tier): &(String, String)| {
                    tier.starts_with('1')
                        || tier.starts_with('2')
                        || tier.starts_with('3')
                        || tier.contains("over limit")
                        || tier.contains("binary")
                        || tier == "settings"
                }) {
                    out.push((format!("{name}/"), String::new()));
                    out.extend(children);
                } else {
                    out.push((
                        format!("{name}/"),
                        format!("4 ignored; {} files", count_files(&p)?),
                    ));
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
    if matches!(a[0], "files" | "read" | "write" | "remove") {
        let name = a.get(1).copied().unwrap_or(".");
        let name = Path::new(name)
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        return Some((|| {
            if internal(&name) {
                return error("E6006", &name, "Solace internal files have no agent access");
            }
            let p = path(root, &name)?;
            let t = tier(root, v, &name, None);
            if a[0] != "files" && t.starts_with('1') {
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
                    if t == "settings" {
                        return error("E6002", &name, "settings are human-only; request a decision");
                    }
                    write(root, v, &name, &input()?, a.contains(&"--replace"), old)
                }
                "remove" => {
                    if !matches!(t.chars().next(), Some('3' | '4')) {
                        return error("E6002", &name, format!("only resource or ignored files can be removed; {t}"));
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
            let mut config = format!("sandbox_mode = \"read-only\"\napproval_policy = \"never\"\nweb_search = \"live\"\n[features]\n");
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

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Store, toml::Value) {
        let root = env::temp_dir().join(new_id(Kind::Fn, "lockdown", "test"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("priv")).unwrap();
        fs::write(root.join("gleam.toml"), "name = \"fixture\"\nversion = \"1.0.0\"\n").unwrap();
        fs::write(root.join("src/main.gleam"), "pub fn main() { 1 }\n").unwrap();
        import(&root).unwrap();
        let v = "resources = [\"priv/**\"]\nresource_limit = 100\nignore = [\"priv/generated/\"]\n".parse().unwrap();
        (Store { db: Connection::open(root.join(".solace/store.db")).unwrap(), root }, v)
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
            ("misc", b"x"),
            (".gitignore", b"priv/ignored\n"),
        ] {
            fs::write(s.root.join(p), b).unwrap();
        }
        let got = call(&s, &v, &["files"], "", None).unwrap();
        assert_eq!(
            got,
            ".gitignore [4 ignored]\n.solace/ [internal]\ngleam.toml [2 manifest]\nmanifest.toml [4 ignored]\nmisc [4 ignored]\npriv/\npriv/a.html [3 resource]\npriv/b.bin [4 ignored (binary)]\npriv/ignored [4 ignored]\npriv/large [4 ignored (over limit)]\nsolace.toml [settings]\nsrc/\nsrc/main.gleam [1 source]\n"
        );
        fs::remove_dir_all(s.root).unwrap();
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
        fs::remove_dir_all(s.root).unwrap();
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
        fs::remove_dir_all(s.root).unwrap();
    }
    #[test]
    fn a4_new_resource_and_size() {
        let (s, v) = fixture();
        call(&s, &v, &["write", "README.md"], "usage", None).unwrap();
        call(&s, &v, &["write", "bin/coolcalc"], "#!/bin/sh\ngleam run -- \"$@\"\n", None).unwrap();
        assert!(call(&s, &v, &["files", "README.md"], "", None).unwrap().contains("4 ignored"));
        assert!(s.find("usage").unwrap().contains("No matches"));
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        fs::write(s.root.join("README.md"), "usage changed").unwrap();
        assert!(call(&s, &v, &["write", "README.md"], "new", None).unwrap_err().0.contains("E6005"));
        call(&s, &v, &["write", "README.md", "--replace"], "new", Some("usage")).unwrap();
        assert_eq!(fs::read_to_string(s.root.join("README.md")).unwrap(), "new changed");
        call(&s, &v, &["read", "README.md"], "", None).unwrap();
        call(&s, &v, &["write", "README.md"], "replacement", None).unwrap();
        call(&s, &v, &["remove", "bin/coolcalc"], "", None).unwrap();
        assert!(!s.root.join("bin/coolcalc").exists());
        fs::write(s.root.join("binary"), b"x\xff").unwrap();
        assert!(call(&s, &v, &["write", "binary", "--replace"], "y", Some("x")).unwrap_err().0.contains("UTF-8"));
        assert_eq!(fs::read(s.root.join("binary")).unwrap(), b"x\xff");
        assert!(call(&s, &v, &["write", "README.md"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
        call(&s, &v, &["write", "priv/new"], "x", None).unwrap();
        assert!(call(&s, &v, &["write", "priv/large"], &"x".repeat(101), None).unwrap_err().0.contains("E6004"));
        fs::remove_dir_all(s.root).unwrap();
    }
    #[test]
    fn a5_validation_in_place_and_recovery() {
        let (s, _) = fixture();
        let v:toml::Value=r#"resources = ["priv/**"]
[validators]
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
        fs::remove_dir_all(s.root).unwrap();
    }
    #[test]
    fn a6_manifest_validation() {
        let (s, mut v) = fixture();
        v["resource_limit"] = toml::Value::Integer(1000);
        let before = fs::read(s.root.join("gleam.toml")).unwrap();
        call(&s, &v, &["read", "gleam.toml"], "", None).unwrap();
        assert!(call(&s, &v, &["write", "gleam.toml"], "=broken", None).unwrap_err().0.contains("E6003"));
        assert_eq!(fs::read(s.root.join("gleam.toml")).unwrap(), before);
        call(
            &s,
            &v,
            &["write", "gleam.toml"],
            "name=\"fixture\"\nversion=\"1.0.0\"\n[dependencies]\nmissing={path=\"./does_not_exist\"}",
            None,
        )
        .unwrap();
        assert!(s.check().is_err());
        fs::remove_dir_all(s.root).unwrap();
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
        fs::remove_dir_all(s.root).unwrap();
    }
    #[test]
    fn a8_settings_human_only_and_reload() {
        let (s, v) = fixture();
        assert!(call(&s, &v, &["write", "solace.toml"], "", None).is_err());
        fs::write(s.root.join("solace.toml"), "resources=[\"docs/**\"]").unwrap();
        call(&s, &settings(&s.root).unwrap(), &["write", "docs/new"], "x", None).unwrap();
        fs::write(s.root.join("solace.toml"), "resources=1").unwrap();
        assert!(settings(&s.root).unwrap_err().0.contains("E6020"));
        fs::remove_dir_all(s.root).unwrap();
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
        fs::remove_dir_all(s.root).unwrap();
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
        fs::remove_dir_all(s.root).unwrap();
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    #[test]
    fn nested_tiers_and_settings_types() {
        let root = env::temp_dir().join(new_id(Kind::Fn, "nested", "tiers"));
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir_all(root.join("priv/nested")).unwrap();
        fs::write(root.join("src/nested/source.gleam"), "pub fn a() { 1 }").unwrap();
        fs::write(root.join("priv/nested/index.html"), "hello").unwrap();
        let v: toml::Value = "resources=[\"priv/**/*.html\"]".parse().unwrap();
        let mut out = String::new();
        tree(&root, &v, &root, &mut out).unwrap();
        assert!(out.contains("src/nested/source.gleam [1 source]"));
        assert!(out.contains("priv/nested/index.html [3 resource]"));
        fs::write(root.join(".gitignore"), "priv/nested/\n").unwrap();
        out.clear();
        tree(&root, &v, &root, &mut out).unwrap();
        assert!(!out.contains("index.html"));
        fs::remove_dir_all(root).unwrap();
    }
}
pub fn tree_files(root: &Path, settings: &toml::Value) -> Result<Vec<(String, String)>, Failure> {
    let mut rows = vec![];
    tree_rows(root, settings, root, &mut rows)?;
    Ok(rows
        .into_iter()
        .filter(|(_, tier)| !tier.is_empty() && tier != "internal" && !tier.starts_with('1'))
        .collect())
}
