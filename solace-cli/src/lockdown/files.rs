use std::io::BufReader;
use std::io::SeekFrom;
use serde_json::Value;
use serde_json::json;
use std::io::BufRead;
use std::io::Seek;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::fs;
use crate::Store;
use crate::sha256;
use crate::count;
use crate::file_name;
use crate::failure::Failure;
use crate::failure::Out;
use crate::failure::fail;
use crate::failure::usage;
use crate::lockdown::error;
use crate::lockdown::settings::WATCH_LIMIT;
use crate::lockdown::commands::trim_output;
use crate::lockdown::paths::patterns;
use crate::lockdown::paths::path;
use crate::lockdown::paths::Ignores;
use crate::lockdown::paths::tier;
use crate::lockdown::paths::tier_and_bytes;
use crate::lockdown::paths::reachable;
use crate::lockdown::paths::elsewhere;
use crate::lockdown::paths::not_found;
use crate::lockdown::paths::not_here;
use crate::lockdown::paths::graph_error;
use crate::lockdown::paths::module_of_path;
use crate::lockdown::paths::new_module_hint;
use crate::lockdown::settings::settings;
use crate::lockdown::settings::limit;
use crate::lockdown::settings::dependencies;
use crate::lockdown::settings::cargo_setting_note;
use crate::lockdown::settings::cargo_settings_changed;
use crate::lockdown::settings::in_skills;
use crate::lockdown::settings::validate;
use crate::jsonx;
#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
use crate::drafts;
use crate::fsx;
use crate::scope;
use crate::workspace;
use crate::lockdown::recovery::session_file;
use crate::lockdown::recovery::reads;
use crate::lockdown::recovery::remember;
use crate::lockdown::recovery::journal;
use crate::lockdown::recovery::settle;
use crate::lockdown::recovery::undo_record;
use crate::lockdown::recovery::moves_file;
/// A file's text within `budget`, and whether it was cut short.
pub(crate) fn read_part(root: &Path, name: &str, args: &[&str], budget: usize) -> Result<(String, bool), Failure> {
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
pub(crate) fn line_ranges(spec: &str) -> Result<Vec<(usize, usize)>, Failure> {
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

pub(crate) fn write(root: &Path, v: &toml::Value, name: &str, new: &str, replace: bool, all: bool, old: Option<&str>) -> Out {
    // D-109: patch text is for `write` with no path; here it would become the file.
    if drafts::is_patch(new) {
        return usage("this text is a patch; send it with `write` and no path, which applies its `*** Update File: <path>` sections");
    }
    // `edited` treats a given `old` as a replacement; so does the read check.
    let replace = replace || old.is_some_and(|o| !o.is_empty());
    let p = path(root, name)?;
    let before = fs::read(&p).ok();
    let text = edited(before.as_deref().unwrap_or_default(), new, replace, all, old)?;
    if crate::language::is_rust(root) && name == "Cargo.toml" && cargo_settings_changed(before.as_deref().unwrap_or_default(), &text) {
        return error("E6002", name, cargo_setting_note());
    }
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
        return error(
            "E6005",
            name,
            "file is unread or changed since last read; `read` it again, or change part of it with `old` or a `write` patch (no path)",
        );
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
                concat!(
                    "would cut {} from {} lines to {}; change part of it with `write` and a patch (no path), ",
                    "or `write {}` with `old`; to replace it whole, `remove {}` first, then `write {}`"
                ),
                name, was, now, name, name, name
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
pub(crate) fn missing(root: &Path, name: &str, paths: &[&str]) -> Out {
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
    error("E6002", name, format!("{}{symbol}", not_found(root, name, "no such file")))
}

/// A file Solace changes itself for a command (`extract`'s gleam.toml
/// dependency, D-103), its undo record written ahead as for `write`. The
/// function returned ends the change: kept, or put back.
pub fn change_file(root: &Path, name: &str, text: &str) -> Result<impl FnOnce(bool) -> Result<(), Failure> + use<>, Failure> {
    let p = path(root, name)?;
    let before = fs::read(&p).ok();
    let journal = journal(root, name, &before)?;
    if let Err(e) = fs::write(&p, text) {
        settle(root, name, &p, &before, None, &journal, false)?;
        return Err(e.into());
    }
    let (root, name, text) = (root.to_path_buf(), name.to_string(), text.to_string());
    Ok(move |kept: bool| settle(&root, &name, &p, &before, Some(text.as_bytes()), &journal, kept))
}

/// One section of a `write` patch, checked and applied in memory: the file's
/// path, its bytes now, and its new text (`None` to delete it).
fn file_change(
    store: &Store,
    v: &toml::Value,
    name: &str,
    kind: drafts::Section,
    hunks: Vec<drafts::Hunk>,
) -> Result<(PathBuf, Option<Vec<u8>>, Option<String>), Failure> {
    use drafts::Section;
    let root = &store.root;
    if crate::lockdown::paths::design_path(root, name) {
        return crate::lockdown::paths::design_error(name);
    }
    let p = reachable(root, name)?;
    not_here(root, name)?;
    let t = tier(root, v, name, None);
    if t == "graph" {
        return graph_error(store, name);
    }
    if t == "protected" {
        if in_skills(v, name) {
            return error("E6002", name, "skills are the humans' house rules: `read` them; agents can't change them");
        }
        return error(
            "E6002",
            name,
            format!(
                "protected files change only through a human-approved proposal: in a work run, \
                 `write {name}` alone, with the exact new text in `text` and the why in `reason`. {}",
                drafts::WHOLE
            ),
        );
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
        (Section::Update, Some(b)) => {
            std::str::from_utf8(b).map_err(|_| Failure(format!("E6002 {name}: a patch changes only UTF-8 text"), 1))?
        }
        _ => "",
    };
    let text = drafts::file_text(name, current, kind, hunks)?;
    if let Some(text) = &text {
        if crate::language::is_rust(root) && name == "Cargo.toml" && cargo_settings_changed(before.as_deref().unwrap_or_default(), text) {
            return error("E6002", name, cargo_setting_note());
        }
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
fn write_patch(store: &Store, v: &toml::Value, raw: &str) -> Out {
    let root = &store.root;
    let mut changes: Vec<(String, PathBuf, Option<Vec<u8>>, Option<String>)> = vec![];
    let mut refused = vec![];
    for (kind, target, _, hunks) in drafts::sections(raw, true)? {
        let name = file_name(Path::new(&target));
        if changes.iter().any(|(n, ..)| *n == name) {
            refused.push(format!("E0108 the patch names {name} twice; give each file one section"));
            continue;
        }
        match file_change(store, v, &name, kind, hunks) {
            Ok((p, before, text)) => changes.push((name, p, before, text)),
            Err(e) => refused.push(e.0),
        }
    }
    if !refused.is_empty() {
        return Err(Failure(format!("nothing written:\n{}", refused.join("\n")), 1));
    }
    let changes: Vec<_> =
        changes.into_iter().filter(|(_, _, before, text)| text.as_deref().map(str::as_bytes) != before.as_deref()).collect();
    if changes.is_empty() {
        return fail(format!("E0108 the patch changes nothing. {}", drafts::RESEND));
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

/// A file patch whose refusal is kept as a draft (D-120): a bad line is fixed
/// with `write --draft <id> --replace`; a fault in the whole patch is resent
/// (D-141). A retry is the whole patch again, whole or nothing (D-104). A
/// replacement kept in a refused retry is named.
pub(crate) fn patch_with_draft(store: &Store, v: &toml::Value, id: Option<&str>, raw: &str, budget: usize, replaced: bool) -> Out {
    match write_patch(store, v, raw) {
        Ok(out) => {
            if let Some(id) = id {
                drafts::discard(store, id)?;
            }
            Ok(trim_output(&out, budget))
        }
        Err(Failure(message, code)) => {
            let id = drafts::keep_file_patch(store, id, raw)?;
            let several = drafts::sections(raw, true).is_ok_and(|s| s.len() > 1);
            let advice = drafts::retry_advice(&message, &id, "write", "", several, true, replaced);
            Err(Failure(format!("{message}\n\n{advice}"), code))
        }
    }
}

pub(crate) fn tree_rows(root: &Path, v: &toml::Value, dir: &Path, out: &mut Vec<(String, String, Option<String>)>) -> Result<(), Failure> {
    rows_in(root, v, &mut crate::lockdown::paths::Ignores::new(root), dir, out)
}

/// `tree_rows`, with one `Ignores` for the whole walk.
fn rows_in(
    root: &Path,
    v: &toml::Value,
    ignores: &mut Ignores,
    dir: &Path,
    out: &mut Vec<(String, String, Option<String>)>,
) -> Result<(), Failure> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
        let (t, bytes) = tier_and_bytes(root, v, ignores, &name, None);
        if e.file_type()?.is_symlink() {
            out.push((name, "unwatched (symlink)".into(), None));
        } else if e.file_type()?.is_dir() {
            if t == "internal" {
                out.push((format!("{name}/"), "internal".into(), None));
            } else if name == "build" || name == ".git" || dependencies(&name) || patterns(v, "unwatched", &name) || ignores.ignored(&name)
            {
                out.push((format!("{name}/"), format!("unwatched; {} files", count_files(&p)?), None));
            } else {
                let mut children = vec![];
                rows_in(root, v, ignores, &p, &mut children)?;
                if children
                    .iter()
                    .any(|(_, tier, _): &(String, String, Option<String>)| tier == "graph" || tier == "watched" || tier == "protected")
                {
                    out.push((format!("{name}/"), String::new(), None));
                    out.extend(children);
                } else {
                    out.push((format!("{name}/"), format!("unwatched; {} files", count_files(&p)?), None));
                }
            }
        } else {
            let size = measure(root, v, &name, &t, bytes.as_deref());
            out.push((name, t, size));
        }
    }
    Ok(())
}

pub(crate) fn tree(root: &Path, v: &toml::Value, dir: &Path, out: &mut String) -> Result<(), Failure> {
    let mut rows = vec![];
    tree_rows(root, v, dir, &mut rows)?;
    for (name, tier, size) in rows {
        *out += &listed(&name, &tier, size.as_deref());
    }
    Ok(())
}

/// One line of `files`: an expanded folder alone, any other entry with its
/// tier, and a file other than graph source with its size (D-119).
pub(crate) fn listed(name: &str, tier: &str, size: Option<&str>) -> String {
    if tier.is_empty() {
        return format!("{name}\n");
    }
    match size {
        Some(n) => format!("{name} [{tier}; {n}]\n"),
        None => format!("{name} [{tier}]\n"),
    }
}

/// A file's line count when it is UTF-8 text, else its size in bytes, as is
/// one over `watch_limit`, which is not read. Folders, graph source, symlinks
/// and Solace's own files have none.
/// `bytes` is the file's content when the caller already read it, so a file
/// listed by `files` is read once (D-119 point 4).
pub(crate) fn measure(root: &Path, v: &toml::Value, name: &str, tier: &str, bytes: Option<&[u8]>) -> Option<String> {
    if name.ends_with('/') || tier == "graph" || tier == "internal" || tier.contains("symlink") {
        return None;
    }
    let size = match bytes {
        Some(b) => b.len() as u64,
        None => fs::metadata(root.join(name)).ok()?.len(),
    };
    if size > limit(v) as u64 {
        return Some(format!("{size} bytes"));
    }
    let read;
    let bytes = match bytes {
        Some(b) => b,
        None => {
            read = fs::read(root.join(name)).ok()?;
            &read
        }
    };
    match std::str::from_utf8(bytes) {
        Ok(text) if !bytes.contains(&0) => Some(format!("{} lines", text.lines().count())),
        _ => Some(format!("{size} bytes")),
    }
}

fn count_files(p: &Path) -> Result<usize, Failure> {
    let mut n = 0;
    for e in fs::read_dir(p)? {
        let e = e?;
        n += if e.file_type()?.is_dir() { count_files(&e.path())? } else { 1 };
    }
    Ok(n)
}

/// `move <from> <to> [--to-project <folder>]`: the source, the destination, and
/// the destination's project, `root` itself without `--to-project` (D-84).
/// `--to-project` takes the folder as `--project` does: from the Solace root,
/// or `../x` from this project (D-126 point 3).
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
    let Some(dir) = project else { return Ok((from, to, root.to_path_buf())) };
    let top = scope::top(root);
    let members = workspace::members(&top)?;
    let dir = crate::project_paths::project_folder(&top, Some(root), dir, &members);
    if !members.iter().any(|m| *m == dir) {
        return error("E6007", dir.as_str(), "not a project under this Solace root");
    }
    Ok((from, to, scope::path(&top, &dir)))
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
        "graph" => match crate::Store::peek(root) {
            Ok(s) => graph_error(&s, name),
            Err(_) => match module_of_path(root, name) {
                Some(module) => error("E6001", name, format!("this source file isn't in the store yet: {}", new_module_hint(&module))),
                None => error(
                    "E6001",
                    name,
                    concat!(
                        "this source file isn't in the store yet, and Solace can't tell which module ",
                        "it would be: submit --new <module> with the module's name"
                    ),
                ),
            },
        },
        "internal" => error("E6006", name, "Solace's own files and other projects' files can't be moved"),
        t if t == "watched" || t.starts_with("unwatched") => Ok(()),
        t => error("E6002", name, format!("{t} files can't be moved")),
    }
}

/// Renames `from` to `to` in one step that never replaces anything at `to`.
fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renameat2(
                olddirfd: i32,
                oldpath: *const std::ffi::c_char,
                newdirfd: i32,
                newpath: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
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
    if crate::lockdown::paths::design_path(src, &from) {
        return crate::lockdown::paths::design_error(&from);
    }
    if crate::lockdown::paths::design_path(dst, &to) {
        return crate::lockdown::paths::design_error(&to);
    }
    let (p, q) = (reachable(src, &from)?, reachable(dst, &to)?);
    not_here(src, &from)?;
    not_here(dst, &to)?;
    let same = crate::project_paths::same_project(src, dst);
    let shown = |root: &Path, name: &str| if same { name.to_string() } else { format!("{}: {name}", scope::dir(&scope::top(root), root)) };
    let (shown_from, shown_to) = (shown(src, &from), shown(dst, &to));
    let intent = json!({"to": q.to_string_lossy()});
    let mut moves: Value = jsonx::read_opt(&moves_file(src)).unwrap_or(json!({}));
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
    fsx::write_atomic(&moves_file(src), moves.to_string())?;
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
        fsx::write_atomic(&moves_file(src), moves.to_string())?;
        const EXDEV: i32 = 18;
        return match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                error("E6002", &to, "something already exists there; a move never replaces a file or folder")
            }
            _ if e.raw_os_error() == Some(EXDEV) => {
                error("E6002", &to, "the two places are on different file systems, so the move can't be one step")
            }
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

/// A file `extract` takes into another project with the code that uses it
/// (D-103, by D-84's rules): checked as a move is, undo records first, one
/// rename that never replaces, then `text` written at the destination when
/// given. The function returned ends it: kept, or put back with its records dropped.
pub fn carry_file(
    src: &Path,
    from: &str,
    dst: &Path,
    to: &str,
    text: Option<String>,
) -> Result<impl FnOnce(bool) -> Result<(), Failure> + use<>, Failure> {
    let (sv, dv) = (settings(src)?, settings(dst)?);
    let (p, q) = (reachable(src, from)?, reachable(dst, to)?);
    if !fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
        return error("E6002", from, "no file there to take along");
    }
    if fs::symlink_metadata(&q).is_ok() {
        return error("E6002", to, "something already exists there; a move never replaces a file or folder");
    }
    movable(src, &sv, from)?;
    movable(dst, &dv, to)?;
    let bytes = fs::read(&p)?;
    let mut records = vec![];
    records.extend(undo_record(src, from, Some(bytes.clone()))?);
    records.extend(undo_record(dst, to, None)?);
    fn drop_records(records: &[PathBuf]) -> Result<(), Failure> {
        for r in records {
            fs::remove_file(r)?;
        }
        Ok(())
    }
    // Folders made for the destination go again with the file.
    let mut made = vec![];
    let mut parent = q.parent().unwrap().to_path_buf();
    while fs::symlink_metadata(&parent).is_err() {
        made.push(parent.clone());
        parent.pop();
    }
    let moved = fs::create_dir_all(q.parent().unwrap()).and_then(|_| rename_new(&p, &q)).and_then(|_| match &text {
        Some(t) => fs::write(&q, t),
        None => Ok(()),
    });
    if let Err(e) = moved {
        if fs::symlink_metadata(&p).is_err() {
            fs::write(&q, &bytes)?;
            rename_new(&q, &p)?;
        }
        for d in &made {
            let _ = fs::remove_dir(d);
        }
        drop_records(&records)?;
        return Err(e.into());
    }
    Ok(move |kept: bool| {
        if kept {
            return Ok(());
        }
        fs::write(&q, &bytes)?;
        rename_new(&q, &p)?;
        for d in &made {
            let _ = fs::remove_dir(d);
        }
        drop_records(&records)
    })
}

/// The footer of a reply that shows a line cut by `around`.
pub(crate) const GREP_CUT: &str = concat!(
    "Lines starting or ending with … are cut from longer lines; read the whole line with ",
    "read <path> --lines <n>..<n>, and to change part of one use write <path> --replace with old/text."
);

/// How `grep` reads its query, for the hint after a miss that looks like a shell habit (D-109).
pub const GREP_SYNTAX: &str = concat!(
    "grep takes literal text, already case-insensitive, searched as written; quotes around ",
    "the whole query are dropped; no regex; the only options, at the end: `--in <path>` ",
    "(one file or folder), `--context n` (1 to 5); `--project <folder>` picks the project"
);

/// `grep <text> [--in <path>] [--context n]`: case-insensitive search of the
/// files Solace watches, and the protected settings. Source symbols are
/// `find`'s; `--in <file>` also searches a text file over the watch limit,
/// and other unwatched files aren't searched (D-52). The text is searched as
/// written, never refused (D-109); the options count only as its last words
/// (D-119). The reply stops at a whole file within `budget`.
pub(crate) fn grep(root: &Path, v: &toml::Value, query: &str, budget: usize) -> Out {
    let (raw, within, context) = grep_options(query.trim())?;
    // `--in` from the Solace root into another project searches there.
    if let Some((dir, there, rest)) = within.and_then(|p| elsewhere(root, &file_name(Path::new(p)), true)) {
        let more = if context > 0 { format!(" --context {context}") } else { String::new() };
        let out = grep(&there, &settings(&there)?, &format!("{raw} --in {rest}{more}"), budget)?;
        return Ok(format!("==== {rest} ({dir}) ====\n{out}"));
    }
    let query = one_quoted(raw);
    if query.is_empty() {
        return Err(Failure("grep: give the text to search for\n\nRun `solace help` for usage.".into(), 2));
    }
    let q = query.to_lowercase();
    let mut rows = vec![];
    tree_rows(root, v, root, &mut rows)?;
    let (place, over_limit_file, over_limit_bytes) = match within {
        Some(p) => match grep_in(root, v, p, &mut rows)? {
            Ok(found) => found,
            Err(not_searched) => return Ok(not_searched),
        },
        None => ("watched files".into(), false, None),
    };
    let graph = rows.iter().any(|(_, t, _)| t == "graph");
    let mut hits = vec![];
    for (name, _, _) in rows.into_iter().filter(|(name, t, _)| t == "watched" || t == "protected" || (over_limit_file && name == &place)) {
        let text = if over_limit_file && name == place {
            over_limit_bytes.as_deref().and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned)
        } else {
            fs::read_to_string(root.join(&name)).ok()
        };
        let Some(text) = text else { continue };
        hits.extend(grep_file(&name, &text, &q, context));
    }
    if hits.is_empty() {
        let source = if graph { "Source is held as symbols (use `find`).\n" } else { "" };
        return Ok(format!("No matches for \"{query}\" in {place}.\n{}{source}", grep_hint(raw, query)));
    }
    let (mut out, mut shown) = (String::new(), 0);
    let mut cut_shown = false;
    for (k, (block, cut)) in hits.iter().enumerate() {
        // Room for the footer, once a shown file has a cut line, and for the
        // line that names the files left out, unless this is the last.
        let foot = if cut_shown || *cut { GREP_CUT.len() + 1 } else { 0 };
        let room = foot + if k + 1 == hits.len() { 0 } else { 140 };
        if shown == 40 || out.len() + block.len() + room > budget {
            break;
        }
        out += block;
        shown += 1;
        cut_shown |= *cut;
    }
    if shown < hits.len() {
        // The files left out, named so the agent narrows (D-119 point 2): the
        // grammar follows the count, and a budget that fits no file says so.
        // When `--in` already names the only place searched, narrowing is
        // circular: the tail points at the rest of the call, and names
        // `--context` only when the agent used it.
        let left = hits.len() - shown;
        let more = if left == 1 { "file matches" } else { "files match" };
        let narrow = match (within, context) {
            (Some(_), 0) => "run grep alone in the call",
            (Some(_), _) => "run grep alone in the call, or drop `--context`",
            (None, _) => "narrow with `--in <path>`",
        };
        out += &if shown == 0 {
            format!("… the budget fits no file; {} {more}, {narrow}.\n", left)
        } else {
            format!("… {} more {more}, not shown; {narrow}.\n", left)
        };
    }
    // One footer for the reply, when a line it shows was cut at either end.
    if cut_shown {
        out += GREP_CUT;
        out += "\n";
    }
    Ok(out)
}

/// The options at the end of a grep query, in either order: the text before
/// them, `--in <path>` and `--context n` (1 to 5; 0 for none). Anywhere else
/// they are part of the text.
fn grep_options(raw: &str) -> Result<(&str, Option<&str>, usize), Failure> {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let (mut n, mut within, mut context) = (words.len(), None, None);
    while n >= 2 {
        match words[n - 2] {
            "--in" if within.is_none() => within = Some(words[n - 1]),
            "--context" if context.is_none() => context = Some(words[n - 1]),
            _ => break,
        }
        n -= 2;
    }
    // The words are slices of `raw`: the text ends where the last of them does.
    let text = if n == 0 { "" } else { &raw[..words[n - 1].as_ptr() as usize - raw.as_ptr() as usize + words[n - 1].len()] };
    let context = match context {
        None => 0,
        Some(c) => match c.parse::<usize>() {
            Ok(c @ 1..=5) => c,
            _ => return usage(format!("grep: --context takes a number from 1 to 5, not `{c}`")),
        },
    };
    Ok((text, within, context))
}

/// Narrows `rows` to the path of `--in`, checked as `read` checks a path, and
/// gives its name, whether it is an over-limit file named explicitly and, for
/// one, its bytes. A path that exists but isn't searched is named instead (the
/// inner `Err`); one that doesn't exist is refused.
fn grep_in(
    root: &Path,
    v: &toml::Value,
    p: &str,
    rows: &mut Vec<(String, String, Option<String>)>,
) -> Result<Result<(String, bool, Option<Vec<u8>>), String>, Failure> {
    let name = file_name(Path::new(p));
    if name.is_empty() {
        return Ok(Ok(("watched files".into(), false, None)));
    }
    if Path::new(&name).components().any(|c| c.as_os_str() == ".solace") {
        return Ok(Err(format!("{name} is Solace's own files; not searched\n")));
    }
    let target = reachable(root, &name)?;
    let (t, bytes) = tier_and_bytes(root, v, &mut Ignores::new(root), &name, None);
    if t == "graph" {
        return Ok(Err(format!("{name} is source held as symbols (use `find`); not searched\n")));
    }
    if !target.exists() {
        return error("E6002", &name, not_found(root, &name, "no such file or folder"));
    }
    let held = |t: &str| t == "watched" || t == "protected";
    if target.is_dir() {
        let dir = format!("{name}/");
        rows.retain(|(n, _, _)| n.starts_with(&dir));
        if !rows.iter().any(|(_, t, _)| held(t)) {
            return Ok(Err(format!("{name}/ holds no watched or protected files; not searched\n")));
        }
        Ok(Ok((name, false, None)))
    } else {
        let over_limit = t == "unwatched (over limit)";
        if !held(&t) && !over_limit {
            return Ok(Err(format!("{name} is {t}; not searched\n")));
        }
        if over_limit {
            let bytes = bytes.as_deref().unwrap_or_default();
            if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
                return Ok(Err(format!("{name} is unwatched (binary); not searched\n")));
            }
        }
        rows.retain(|(n, _, _)| *n == name);
        if over_limit && rows.is_empty() {
            rows.push((name.clone(), t, None));
        }
        Ok(Ok((name, over_limit, if over_limit { bytes } else { None })))
    }
}

/// A file's matching lines (up to 10) with `context` lines around them, as
/// the reply shows them, and whether any shown line was cut; `None` when
/// nothing matches `q`.
fn grep_file(name: &str, text: &str, q: &str, context: usize) -> Option<(String, bool)> {
    // Matching is on the trimmed line; the reply shows the line as written, so
    // a match's indentation is kept (D-119 point 1).
    let raw: Vec<&str> = text.lines().collect();
    let lines: Vec<&str> = raw.iter().map(|l| l.trim()).collect();
    let found: Vec<Option<usize>> = lines.iter().map(|l| find_lower(l, q)).collect();
    let hits: Vec<usize> = (0..lines.len()).filter(|&i| found[i].is_some()).collect();
    if hits.is_empty() {
        return None;
    }
    let shown = &hits[..hits.len().min(10)];
    // Windows around the shown matches; those that touch merge.
    let mut windows: Vec<(usize, usize)> = vec![];
    for &i in shown {
        let w = (i.saturating_sub(context), (i + context).min(lines.len() - 1));
        match windows.last_mut() {
            Some(last) if w.0 <= last.1 + 1 => last.1 = w.1,
            _ => windows.push(w),
        }
    }
    let mut out = if hits.len() > 10 { format!("{name} ({} matches, showing 10)\n", hits.len()) } else { format!("{name}\n") };
    let mut cut = false;
    for (k, &(a, b)) in windows.iter().enumerate() {
        if k > 0 && context > 0 {
            out += "  …\n";
        }
        for j in a..=b {
            // A line that matches but is past the 10 shown is context, not a
            // match line (D-119 point 2).
            let (mark, (line, was_cut)) = match found[j].filter(|_| shown.binary_search(&j).is_ok()) {
                Some(at) => (':', around(raw[j], at + raw[j].len() - raw[j].trim_start().len(), q.len())),
                None => ('-', around(raw[j], 0, 0)),
            };
            cut |= was_cut;
            out += &format!("  {:>4}{mark} {line}\n", j + 1);
        }
    }
    Some((out, cut))
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
    let filter = n >= 2 && (words[n - 2] == "--path" || path(words[n - 1]));
    if !(option || quoted || filter || raw.contains("\\|") || raw.contains("\\\"")) {
        return String::new();
    }
    let filter = if filter { " To search one file or folder, end the query with `--in <path>`." } else { "" };
    format!("Searched for: {query}.{filter} {GREP_SYNTAX}\n")
}

/// Extensions that make a word a file path for `grep_hint`.
const FILE_EXTENSIONS: &[&str] =
    &["rs", "gleam", "md", "txt", "toml", "json", "js", "mjs", "ts", "py", "html", "css", "erl", "sh", "yml", "yaml", "lock", "csv"];

/// A long line cut to about 200 characters around a match at byte `at`
/// (cuts snap to character boundaries). The flag says the line was cut at
/// either end.
fn around(line: &str, at: usize, len: usize) -> (String, bool) {
    if line.len() <= 200 {
        return (line.to_string(), false);
    }
    let floor = |i: usize| (0..=i.min(line.len())).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
    let (start, end) = (floor(at.saturating_sub(80)), floor(at + len + 120));
    let shown = format!("{}{}{}", if start > 0 { "…" } else { "" }, &line[start..end], if end < line.len() { "…" } else { "" });
    (shown, start > 0 || end < line.len())
}

pub fn tree_files(root: &Path, settings: &toml::Value) -> Result<Vec<(String, String)>, Failure> {
    let mut rows = vec![];
    tree_rows(root, settings, root, &mut rows)?;
    Ok(rows.into_iter().filter(|(_, tier, _)| !tier.is_empty() && tier != "internal" && tier != "graph").map(|(n, t, _)| (n, t)).collect())
}

/// Copies the project's files a test run needs into `dir`: those `wanted`
/// names and tiers, then each existing name in `extra`.
pub(crate) fn copy_files(s: &Store, dir: &Path, wanted: impl Fn(&str, &str) -> bool, extra: &[&str]) -> Result<Vec<String>, Failure> {
    let mut names: Vec<String> =
        tree_files(&s.root, &settings(&s.root)?)?.into_iter().filter(|(name, tier)| wanted(name, tier)).map(|(name, _)| name).collect();
    names.extend(extra.iter().map(|name| name.to_string()));
    names.sort();
    names.dedup();
    let mut copied = vec![];
    for name in names {
        let from = s.root.join(&name);
        if !from.is_file() {
            continue;
        }
        let to = dir.join(&name);
        crate::fsx::copy_if_changed(&from, &to)?;
        copied.push(name);
    }
    Ok(copied)
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

/// The paths a `write` or `remove` names (`remove` takes a list, D-102), for
/// a task's snapshot of the files a command changes.
pub fn paths<'a>(a: &[&'a str]) -> Vec<&'a str> {
    // a kept file patch (D-120) names its files in its draft
    if !matches!(a[0], "write" | "remove") || a.get(1) == Some(&"--draft") {
        return vec![];
    }
    a[1..].iter().copied().filter(|x| !x.starts_with("--")).collect()
}

/// The byte offset in `line` where its lowercased text first holds `q`.
/// Lowercasing is done whole, as the query was, so a letter whose case depends
/// on its neighbors (`Σ`) still matches; each original character's lowercased
/// byte length maps the offset back to the text shown.
fn find_lower(line: &str, q: &str) -> Option<usize> {
    let target = line.to_lowercase().find(q)?;
    let mut lower_at = 0;
    let mut nearest = 0;
    for (at, ch) in line.char_indices() {
        if lower_at >= target {
            return Some(at);
        }
        nearest = at;
        lower_at += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
    }
    Some(nearest)
}
