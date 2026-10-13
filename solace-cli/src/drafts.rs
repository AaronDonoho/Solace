//! Rejected submissions are scratch text, never live symbols.
use super::*;

fn owner() -> String {
    env::var("SOLACE_TASK").unwrap_or_else(|_| "cli".into())
}

/// A draft's base (D-46): the text of the symbols it replaces and the headers
/// of their modules (or of the module it adds to), so edits elsewhere, such as
/// to a callee, leave it current.
fn base(s: &Store, ix: &Index, args: &[String], text: &str) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut names: Vec<String> =
        text.lines().filter_map(|l| l.strip_prefix("// ── ")).map(|h| h.split([' ', '(']).next().unwrap_or("").to_string()).collect();
    let (mut parts, mut modules) = (vec![], vec![]);
    // a patch draft: what its sections update or add to; a module section covers the whole module
    if is_patch(text) {
        for target in text.lines().filter(|l| l.starts_with("*** Update") || l.starts_with("*** Add")).map(patch_target) {
            match s.resolve_module(ix, &target) {
                Some(m) if !target.contains('.') => {
                    parts.extend(ix.syms.iter().filter(|x| x.module == m.path).map(|x| format!("{}\n{}", x.qname(), x.text)));
                    modules.push(m.path.clone());
                }
                _ => names.push(target),
            }
        }
    }
    match a.as_slice() {
        // the module it adds to, or whose imports it replaces
        [_, "--new" | "--imports", m] => modules.push(s.resolve_module(ix, m).map_or(m.to_string(), |m| m.path.clone())),
        // a whole module: every symbol in it
        [_, m]
            if !m.contains('.')
                && let Some(m) = s.resolve_module(ix, m) =>
        {
            parts.extend(ix.syms.iter().filter(|x| x.module == m.path).map(|x| format!("{}\n{}", x.qname(), x.text)));
            modules.push(m.path.clone());
        }
        [_, name] => names.push(name.to_string()),
        _ => {}
    }
    for name in names {
        match s.resolve(ix, &name) {
            Ok(i) => {
                parts.push(format!("{}\n{}", ix.syms[i].qname(), ix.syms[i].text));
                modules.push(ix.syms[i].module.clone());
            }
            // a new symbol: its module, which may not exist yet
            Err(_) => modules.push(name.rsplit_once('.').map_or(name.as_str(), |p| p.0).to_string()),
        }
    }
    for m in modules {
        let header = s.resolve_module(ix, &m).map(|r| format!("module {}\n{}", r.path, r.header));
        parts.push(header.unwrap_or_else(|| format!("no module {m}")));
    }
    Ok(sha256(parts.join("\n\n").as_bytes()))
}

pub fn list(s: &Store) -> Out {
    list_for(s, &owner())
}

pub fn list_for(s: &Store, task: &str) -> Out {
    let mut q = s.db.prepare("SELECT id, args, text, base FROM drafts WHERE owner=? ORDER BY rowid")?;
    let rows: Vec<(String, String, String, String)> =
        q.query_map([task], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut out = vec![];
    let ix = s.index()?;
    for (id, args, text, then) in rows {
        let parsed = serde_json::from_str::<Vec<String>>(&args).unwrap_or_default();
        // a kept `write` patch has no base: its hunks are matched again when it is retried (D-120)
        let current = parsed.first().is_some_and(|c| c == "write") || base(s, &ix, &parsed, &text)? == then;
        let status = if current { "current" } else { "stale: what it replaces changed since; check it isn't superseded" };
        out.push(format!("{id}: {args} ({status})"));
    }
    Ok(out.join("\n"))
}

fn load(s: &Store, id: &str) -> Result<(Vec<String>, String, String), Failure> {
    let row: Option<(String, String, String)> =
        s.db.query_row("SELECT args,text,base FROM drafts WHERE id=? AND owner=?", params![id, owner()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let (args, text, base) = row.ok_or_else(|| Failure("E0106 draft not found in this task/session".into(), 1))?;
    let args = serde_json::from_str(&args).map_err(|e| Failure(e.to_string(), 1))?;
    Ok((args, text, base))
}

pub fn view(s: &Store, id: &str) -> Out {
    Ok(load(s, id)?.1)
}

/// `old` → `new` in `text`: exactly one occurrence, or every one with `all`.
/// Returns the text and how many were replaced.
pub fn replace(text: &str, old: Option<&str>, new: &str, all: bool) -> Result<(String, usize), Failure> {
    let old = old.filter(|x| !x.is_empty()).ok_or_else(|| Failure("E0104 provide nonempty exact text in `old`".into(), 2))?;
    let at: Vec<usize> = text.match_indices(old).map(|(k, _)| text[..k].matches('\n').count() + 1).collect();
    match at.len() {
        0 => fail("E0105 `old` does not occur; read the current text and copy it exactly"),
        1 => Ok((text.replacen(old, new, 1), 1)),
        n if all => Ok((text.replace(old, new), n)),
        n => fail(format!(
            "E0105 `old` occurs {n} times (lines {}); add surrounding text to pick one, or use `--replace --all` to replace every one",
            at.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Text in `apply_patch` form: `*** Begin Patch`, or a first line `*** Update…`,
/// `*** Add…` or `*** Delete…`.
pub fn is_patch(text: &str) -> bool {
    text.trim_start().lines().next().is_some_and(|l| ["*** Begin Patch", "*** Update", "*** Add", "*** Delete"].iter().any(|p| l.starts_with(p)))
}

/// A section's target: `src/pilot/seed.gleam` or `pilot/seed`, or a symbol.
fn patch_target(line: &str) -> String {
    let name = line.split_once(':').map_or("", |p| p.1).trim();
    let name = ["/src/", "/test/"].iter().find_map(|d| name.rsplit_once(d).map(|p| p.1)).unwrap_or(name);
    let name = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/")).unwrap_or(name);
    name.strip_suffix(".gleam").unwrap_or(name).to_string()
}

/// A patch line quoted in an error: long ones are cut.
fn clip(line: &str) -> String {
    match line.char_indices().nth(100) {
        Some((k, _)) => format!("{}…", &line[..k]),
        None => line.to_string(),
    }
}

/// A patch's hunk: its `@@` context line, its lines (` `, `-` or `+` first),
/// and its line in the patch.
pub type Hunk = (Option<String>, Vec<String>, usize);

/// What a patch section does to its target.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Section {
    Update,
    Add,
    Delete,
}

/// A patch's sections: what each does, its target, the line of its header,
/// and its hunks. `files`: a `write` patch (D-104), whose targets are paths
/// (`*** Update File: <path>`) and which may delete a file; otherwise a
/// `submit` patch (D-47), whose targets are symbols or modules.
pub fn sections(raw: &str, files: bool) -> Result<Vec<(Section, String, usize, Vec<Hunk>)>, Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    let head = if files { " File" } else { "" };
    let path = |l: &str| l.split_once(':').map_or("", |p| p.1).trim().to_string();
    let mut sections: Vec<(Section, String, usize, Vec<Hunk>)> = vec![];
    let mut prev = "";
    for (n, line) in raw.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        // a refusal at this line, with the line before it, so a bare `}` can be placed
        let here = |m: String| {
            let before = if n > 1 { format!(" (line {} is `{}`)", n - 1, clip(prev)) } else { String::new() };
            bad(format!("line {n} of the patch: {m}{before}"))
        };
        let target = |l: &str| if files { path(l) } else { patch_target(l) };
        match line {
            l if l.starts_with("*** Begin Patch") || l.starts_with("*** End") => {}
            l if l.starts_with("*** Update") => sections.push((Section::Update, target(l), n, vec![])),
            l if l.starts_with("*** Add") => sections.push((Section::Add, target(l), n, vec![(None, vec![], n + 1)])),
            l if l.starts_with("*** Delete") && files => sections.push((Section::Delete, target(l), n, vec![])),
            l if l.starts_with("*** Delete") => return Err(here("a patch can't delete; use `delete <name>`.".into())),
            l if l.starts_with("*** Move") && files => return Err(here("a patch can't move a file; use `move <from> <to>`.".into())),
            l if l.starts_with("*** Move") => return Err(here("a patch can't rename; submit the symbol under its new name.".into())),
            l => {
                let Some((kind, target, _, hunks)) = sections.last_mut() else {
                    return Err(here(if files {
                        "a file patch starts with `*** Begin Patch`, then `*** Add File: <path>`, `*** Update File: <path>` or `*** Delete File: <path>`.".into()
                    } else {
                        "a patch starts with `*** Begin Patch`, then `*** Update: <name>` or `*** Add: <name>`.".into()
                    }));
                };
                let update = *kind == Section::Update;
                if *kind == Section::Delete {
                    if !l.trim().is_empty() {
                        return Err(here(format!("`*** Delete File: {target}` takes no lines; this one is `{}`", clip(l))));
                    }
                } else if let Some(at) = l.strip_prefix("@@").filter(|_| update) {
                    let at = at.trim().trim_end_matches("@@").trim();
                    // a unified diff's `@@ -3,4 +3,5 @@` gives no context line
                    let numbers = at.starts_with('-') && at[1..].starts_with(|c: char| c.is_ascii_digit());
                    hunks.push(((!at.is_empty() && !numbers).then(|| at.to_string()), vec![], n));
                } else if !update && !l.starts_with('+') {
                    return Err(here(format!("in `*** Add{head}: {target}` every line starts with `+`; this one doesn't: `{}`", clip(l))));
                } else if update && !l.is_empty() && !l.starts_with([' ', '-', '+']) {
                    return Err(here(format!(
                        "in `*** Update{head}: {target}` each line starts with ` `, `-` or `+` (or `@@`); this one doesn't: `{}`",
                        clip(l)
                    )));
                } else {
                    if hunks.is_empty() {
                        hunks.push((None, vec![], n));
                    }
                    hunks.last_mut().unwrap().1.push(if l.is_empty() { " ".into() } else { l.to_string() });
                }
            }
        }
        prev = line;
    }
    if sections.is_empty() {
        return Err(bad(if files {
            "the patch has no `*** Add File: <path>`, `*** Update File: <path>` or `*** Delete File: <path>` section.".into()
        } else {
            "the patch has no `*** Update: <name>` or `*** Add: <name>` section.".into()
        }));
    }
    Ok(sections)
}

/// Applies a section's hunks to `lines` in order, as `apply_patch` does: each
/// from where the last one ended, matched exactly, then ignoring trailing
/// space, then trimmed. A hunk that only adds lines goes after its `@@` line;
/// with none, it goes at the end if `at_end` (a file), else is refused (a
/// symbol). `show` is the command that shows the target's current text.
fn apply_hunks(target: &str, show: &str, lines: &mut Vec<String>, hunks: Vec<Hunk>, at_end: bool) -> Result<(), Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    let mut cursor = 0;
    for (at, hunk, hl) in hunks {
        let old: Vec<&str> = hunk.iter().filter(|l| !l.starts_with('+')).map(|l| &l[1..]).collect();
        let new: Vec<String> = hunk.iter().filter(|l| !l.starts_with('-')).map(|l| l[1..].to_string()).collect();
        let mut from = cursor;
        if let Some(at) = &at {
            let found = (cursor..lines.len()).chain(0..cursor).find(|&k| lines[k].trim().starts_with(at.as_str()));
            from = found.ok_or_else(|| bad(format!("in {target} (hunk at line {hl}), no line starts with `@@ {at}`; `{show}` shows the current text.")))? + 1;
        }
        let at_k = if old.is_empty() {
            match at {
                Some(_) => Some(from),
                None if at_end => Some(lines.len()),
                None => return Err(bad(format!("in {target} (hunk at line {hl}), a hunk that only adds lines needs `@@ <the line before them>`."))),
            }
        } else {
            let same: [&dyn Fn(&str, &str) -> bool; 3] = [&|a, b| a == b, &|a, b| a.trim_end() == b.trim_end(), &|a, b| a.trim() == b.trim()];
            let fits = |k: usize, eq: &dyn Fn(&str, &str) -> bool| k + old.len() <= lines.len() && old.iter().zip(&lines[k..]).all(|(o, l)| eq(o, l));
            same.iter().find_map(|eq| (from..lines.len()).chain(0..from).find(|&k| fits(k, *eq)))
        };
        let Some(k) = at_k else {
            let first = old.iter().find(|l| !l.trim().is_empty()).unwrap_or(&"");
            // a line that is only part of one longer line: patch lines are whole lines (D-120)
            let part = old.iter().map(|l| l.trim()).find(|l| !l.is_empty() && !lines.iter().any(|x| x.trim() == *l));
            let within: Vec<usize> = part.map_or(vec![], |p| (0..lines.len()).filter(|&k| lines[k].contains(p)).collect());
            let hint = match (part, within.as_slice()) {
                (Some(p), [k]) => {
                    let (line, fix) = if at_end {
                        (format!("line {}", k + 1), format!("`write {target}`"))
                    } else {
                        ("a longer line".to_string(), "`submit <name> --replace`".to_string())
                    };
                    format!(
                        " `{}` is only part of {line}: `{}`. A patch line is a whole line: copy all of it, or change just that text with {fix} and `old` and `text`.",
                        clip(p),
                        clip(lines[*k].trim())
                    )
                }
                _ => String::new(),
            };
            return Err(bad(format!(
                "in {target} (hunk at line {hl}), these lines weren't found: `{}`{}; copy them exactly from `{show}`.{hint}",
                first.trim(),
                if old.len() > 1 { format!(" (and {} more)", old.len() - 1) } else { String::new() }
            )));
        };
        cursor = k + new.len();
        lines.splice(k..k + old.len(), new);
    }
    Ok(())
}

/// A file's new text from a `write` patch section (D-104): an Add's `+` lines,
/// an Update's hunks applied to `current`, or `None` for a Delete.
pub fn file_text(path: &str, current: &str, kind: Section, hunks: Vec<Hunk>) -> Result<Option<String>, Failure> {
    match kind {
        Section::Delete => Ok(None),
        Section::Add => Ok(Some(hunks[0].1.iter().map(|l| format!("{}\n", &l[1..])).collect())),
        Section::Update => {
            let mut lines: Vec<String> = current.lines().map(String::from).collect();
            apply_hunks(path, &format!("read {path}"), &mut lines, hunks, true)?;
            let mut text = lines.join("\n");
            // the file keeps its last newline; one that had no text gets one
            if !lines.is_empty() && (current.ends_with('\n') || current.is_empty()) {
                text.push('\n');
            }
            Ok(Some(text))
        }
    }
}

/// Blocks for a plain `submit` from a patch (D-47): `*** Update: <symbol or
/// module>` sections of `@@` hunks (` ` context, `-` removed, `+` added) change
/// stored symbols, applied in order as `apply_patch` does, each from where the
/// last one ended; `*** Add: <module>.<name>` (or a new module) adds `+` lines.
/// Returns one block per symbol changed or added.
pub fn patch_blocks(s: &Store, raw: &str) -> Result<String, Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    let sections = sections(raw, false)?;
    let ix = s.index()?;
    let mut blocks = String::new();
    // Each item of `text` as a block in `module`, but for the one equal to
    // `stored`; extra items (code added after a symbol) become new symbols.
    let mut emit = |module: &str, header: &str, text: &str, stored: &str| {
        let mut header = if header.is_empty() { String::new() } else { format!("{header}\n\n") };
        for item in gleam::split(text).items.iter().filter(|i| i.text != stored) {
            blocks += &format!("// ── {module}.{} ──\n{}{}\n", item.name, std::mem::take(&mut header), item.text);
        }
    };
    for (kind, target, at_line, hunks) in sections {
        // A `*** Update File:` header may name a symbol; one that names no code is a file, for `write`.
        let header = raw.lines().nth(at_line - 1).unwrap_or("");
        let not_code = |e: Failure| {
            if !header.contains(" File:") {
                return e;
            }
            let path = header.split_once(':').map_or("", |p| p.1).trim();
            bad(format!("`{path}` (line {at_line}) is no symbol or module; a file other than code changes with `write` with no path, this patch as `text`."))
        };
        if kind == Section::Add {
            let text: String = hunks[0].1.iter().map(|l| format!("{}\n", &l[1..])).collect();
            let parsed = gleam::split(&text);
            let module = match target.rsplit_once('.') {
                Some((m, n)) if parsed.items.iter().any(|i| i.name == n) => m.to_string(),
                _ => target.clone(),
            };
            if parsed.items.is_empty() {
                return Err(not_code(bad(format!("`*** Add: {target}` (line {at_line}) defines no function, type or constant."))));
            }
            emit(&module, &parsed.header, &text, "");
            continue;
        }
        // The target's text, a unit per symbol (and a module's header first).
        let (module, mut units): (String, Vec<(Option<usize>, String)>) = match s.resolve_module(&ix, &target) {
            Some(m) => (
                m.path.clone(),
                std::iter::once((None, m.header.clone()))
                    .chain(ix.syms.iter().enumerate().filter(|(_, x)| x.module == m.path).map(|(i, x)| (Some(i), x.text.clone())))
                    .collect(),
            ),
            None => {
                let i = s.resolve(&ix, &target).map_err(not_code)?;
                (ix.syms[i].module.clone(), vec![(Some(i), ix.syms[i].text.clone())])
            }
        };
        // One list of lines, with a separator no hunk can match between units.
        const SEP: &str = "\u{0}";
        let mut lines: Vec<String> =
            units.iter().map(|u| u.1.clone()).collect::<Vec<_>>().join(&format!("\n{SEP}\n")).lines().map(String::from).collect();
        apply_hunks(&target, &format!("view {target}"), &mut lines, hunks, false)?;
        let joined = lines.join("\n");
        let texts: Vec<&str> = joined.split(&format!("\n{SEP}\n")).collect();
        if texts.len() != units.len() {
            return Err(bad(format!("in {target} (section at line {at_line}), a hunk spans two symbols; give each symbol its own hunk.")));
        }
        for (u, t) in units.iter_mut().zip(&texts) {
            u.1 = t.trim_end().to_string();
        }
        // A changed header's imports go with the module's first changed symbol.
        let mut header = match &units[0] {
            (None, h) if h.trim() != s.resolve_module(&ix, &module).map_or("", |m| m.header.trim()) => h.clone(),
            _ => String::new(),
        };
        for (i, text) in units.iter().filter_map(|(i, t)| Some((i.as_ref()?, t))) {
            if *text == ix.syms[*i].text {
                continue;
            }
            match gleam::split(text).items.first() {
                None => return Err(bad(format!("the patch leaves nothing of {} (section at line {at_line}); use `delete <name>`.", ix.syms[*i].qname()))),
                Some(item) if item.name != ix.syms[*i].name => {
                    return Err(bad(format!("the patch renames {} (section at line {at_line}); submit it whole under its new name.", ix.syms[*i].qname())));
                }
                _ => {}
            }
            emit(&module, &std::mem::take(&mut header), text, &ix.syms[*i].text);
        }
        if !header.is_empty() {
            return Err(bad(format!(
                "the patch changes only {module}'s imports (section at line {at_line}); imports are added automatically (`submit --imports {module}` replaces them)."
            )));
        }
    }
    if blocks.is_empty() {
        return Err(bad("the patch changes nothing.".into()));
    }
    Ok(blocks)
}

pub fn submit(s: &Store, a: &[&str], input: &mut dyn FnMut() -> Out, old: Option<&str>) -> Out {
    // `--all`: `--replace` changes every occurrence of `old` (D-46)
    let all = a.contains(&"--all");
    let a: Vec<&str> = a.iter().copied().filter(|x| *x != "--all").collect();
    let mut replaced = 0;
    let ix = s.index()?;
    let (id, args, raw) = match a.as_slice() {
        ["submit", "--draft", id] | ["submit", "--draft", id, "--replace"] => {
            let (args, text, expected) = load(s, id)?;
            if args.first().is_some_and(|c| c == "write") {
                return fail(format!("E0106 draft {id} is a file patch; retry it with `write --draft {id}`"));
            }
            if base(s, &ix, &args, &text)? != expected {
                return fail(format!(
                    "E0107 draft {id} is stale: a symbol it replaces (or its module's imports) changed since it was created. `view --draft {id}` preserves your text; inspect current source and make a new submission."
                ));
            }
            let raw = if a.len() == 4 {
                let (t, n) = replace(&text, old, &input()?, all)?;
                replaced = n;
                t
            } else {
                text
            };
            (id.to_string(), args, raw)
        }
        ["submit", name, "--replace"] => {
            let symbol = &ix.syms[s.resolve(&ix, name)?];
            let (raw, n) = replace(&symbol.text, old, &input()?, all)?;
            replaced = n;
            (unique_id(), vec!["submit".into(), symbol.qname()], raw)
        }
        // a patch (D-47) is kept as written until it applies; then the draft holds the blocks
        ["submit"] | ["submit", _] | ["submit", "--new" | "--imports", _] => {
            let text = input()?;
            if a.len() <= 2 && is_patch(&text) {
                (unique_id(), vec!["submit".into()], text)
            } else {
                (unique_id(), a.iter().map(|x| x.to_string()).collect(), text)
            }
        }
        _ => {
            return usage("submit [name [--replace [--all]] | --new module | --imports module | --draft id [--replace [--all]]]");
        }
    };
    let mut raw = raw;
    let mut as_blocks = false;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match refs.as_slice() {
        // a patch that applies is submitted as the blocks it changes; if those are rejected, the draft keeps them
        ["submit"] if is_patch(&raw) => patch_blocks(s, &raw).and_then(|blocks| {
            let result = s.submit_symbols(None, None, &blocks);
            if result.is_err() {
                raw = blocks;
                as_blocks = true;
            }
            result
        }),
        ["submit"] => s.submit_symbols(None, None, &raw),
        ["submit", "--new", module] => s.submit_symbols(None, Some(module), &raw),
        ["submit", "--imports", module] => s.submit(Target::Imports(module), &raw),
        ["submit", name] if !name.contains('.') && s.resolve_module(&ix, name).is_some() => s.submit(Target::Module(name), &raw),
        // A whole module for a module that doesn't exist yet creates it.
        ["submit", name] if !name.contains('.') && s.resolve(&ix, name).is_err() => s.submit_symbols(None, Some(name), &raw),
        ["submit", name] => s.submit_symbols(Some(name), None, &raw),
        _ => return fail("E0106 invalid draft command"),
    };
    match result {
        Ok(out) => {
            s.db.execute("DELETE FROM drafts WHERE id=? AND owner=?", params![id, owner()])?;
            Ok(if replaced > 1 { format!("Replaced {replaced} occurrences.\n{out}") } else { out })
        }
        Err(Failure(message, code)) => {
            let base = base(s, &ix, &args, &raw)?;
            s.db.execute(
                "INSERT OR REPLACE INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,?)",
                params![id, owner(), json!(args).to_string(), raw, base],
            )?;
            let what = if as_blocks { " (the patch applied; the draft holds the blocks it changed)" } else { "" };
            Err(Failure(
                format!(
                    "{message}\n\nDraft {id} retained{what}. Read: `view --draft {id}`. Fix and retry: `submit --draft {id} --replace` with `old` and `text`. List: `drafts`."
                ),
                code,
            ))
        }
    }
}

pub fn clear(s: &Store, task: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM drafts WHERE owner=?", [task])?;
    Ok(())
}

/// A refused `write` patch (D-120), kept as written under `id` (a new one if
/// none) so a bad line is fixed in place. It has no base: a retry matches its
/// hunks against the files as they are then.
pub fn keep_file_patch(s: &Store, id: Option<&str>, raw: &str) -> Result<String, Failure> {
    let id = id.map_or_else(unique_id, str::to_string);
    s.db.execute(
        "INSERT OR REPLACE INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,'')",
        params![id, owner(), json!(["write"]).to_string(), raw],
    )?;
    Ok(id)
}

/// A kept `write` patch's text; a `submit` draft says how it is retried.
pub fn file_patch(s: &Store, id: &str) -> Result<String, Failure> {
    let (args, text, _) = load(s, id)?;
    if args.first().is_none_or(|c| c != "write") {
        return fail(format!("E0106 draft {id} is a submit; retry it with `submit --draft {id}`"));
    }
    Ok(text)
}

/// Removes a draft once its retry applies.
pub fn discard(s: &Store, id: &str) -> Result<(), Failure> {
    s.db.execute("DELETE FROM drafts WHERE id=? AND owner=?", params![id, owner()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    /// A project of `files` on a board.
    fn board_project(files: &[(&str, &str)]) -> Project {
        let p = project(files);
        board::open(&p.root).unwrap();
        p
    }

    #[test]
    fn rejected_text_survives_and_retries_atomically() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let original = board::snapshot(&s).unwrap();
        let raw = "// ── app.main ──\npub fn main() { helper() }\n// ── app.helper ──\nfn helper() { -. 2 }\n";
        assert!(submit(&s, &["submit"], &mut || Ok(raw.into()), None).unwrap_err().0.contains("Draft"));
        let id: String = s.db.query_row("SELECT id FROM drafts", [], |r| r.get(0)).unwrap();
        assert_eq!(view(&s, &id).unwrap(), raw);
        assert_eq!(board::snapshot(&s).unwrap(), original);
        // A second failed attempt replaces the same draft, including invalid syntax.
        assert!(submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok("unknown()".into()), Some("-. 2")).is_err());
        assert!(view(&s, &id).unwrap().contains("unknown()"));
        assert!(list(&s).unwrap().ends_with("(current)"));
        assert_eq!(board::snapshot(&s).unwrap(), original);
        let out = submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok("2".into()), Some("unknown()")).unwrap();
        assert!(out.contains("Saved"));
        assert!(list(&s).unwrap().is_empty());
        assert!(s.view(&["app.main"], false, BUDGET).unwrap().contains("helper()"));
        // Module creation and whole-module submissions use the same draft path.
        for args in [vec!["submit", "--new", "other"], vec!["submit", "app"]] {
            assert!(submit(&s, &args, &mut || Ok("pub fn main() { absent() }".into()), None).is_err());
        }
        assert!(!s.root.join("src/other.gleam").exists(), "rejected modules must leave no compiler input behind");
        // Stale only when what a draft replaces changed (D-46): the whole-module
        // draft of `app` after app.helper changes, not the new module's draft.
        let ids: Vec<String> =
            s.db.prepare("SELECT id FROM drafts ORDER BY rowid")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
        s.submit_symbols(Some("app.helper"), None, "fn helper() { 3 }").unwrap();
        let listed = list(&s).unwrap();
        assert!(listed.lines().next().unwrap().ends_with("(current)") && listed.lines().nth(1).unwrap().contains("(stale"), "{listed}");
        let stale = ids[1].clone();
        assert!(submit(&s, &["submit", "--draft", &stale], &mut || Ok(String::new()), None).unwrap_err().0.contains("E0107"));
        assert!(view(&s, &stale).unwrap().contains("absent()"));
        // A caller's draft retries after its callee changes, and the rejection
        // names the callee to submit with it.
        s.submit_symbols(None, Some("app"), "pub fn twice(x: Int) -> Int { x * 2 }\npub fn four() -> Int { twice(2) }").unwrap();
        let err = submit(&s, &["submit", "app.four"], &mut || Ok("pub fn four() -> Int { twice(2, 0) }".into()), None).unwrap_err().0;
        assert!(err.contains("If you are changing `app.twice` too, submit both in one `submit`"), "{err}");
        let caller = s.db.query_row("SELECT id FROM drafts ORDER BY rowid DESC LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        s.submit_symbols(Some("app.twice"), None, "pub fn twice(x: Int, y: Int) -> Int { x * 2 + y }").unwrap();
        let out = submit(&s, &["submit", "--draft", &caller], &mut || Ok(String::new()), None).unwrap();
        assert!(out.contains("Saved app.four") && out.contains("type-checks"), "{out}");
        s.db.execute("UPDATE drafts SET owner='another-task' WHERE id=?", [&stale]).unwrap();
        assert!(view(&s, &stale).unwrap_err().0.contains("E0106"));
        clear(&s, "another-task").unwrap();
        assert!(view(&s, &stale).is_err());
    }

    /// Every way a patch is refused says why (E0108) and changes nothing.
    #[test]
    fn patch_refusals_name_the_problem() {
        let s = board_project(&[("src/app.gleam", "import gleam/int\n\npub fn main() {\n  let x = 1\n  x\n}\n\npub fn other() {\n  2\n}\n")]);
        let original = board::snapshot(&s).unwrap();
        let cases: [(&str, &str); 15] = [
            ("*** Begin Patch\n*** Delete: app.main\n*** End Patch\n", "a patch can't delete"),
            ("*** Begin Patch\n*** Move: app.main\n*** End Patch\n", "a patch can't rename"),
            ("+pub fn x() { 1 }\n", "a patch starts with `*** Begin Patch`"),
            ("*** Begin Patch\n*** End Patch\n", "the patch has no"),
            ("*** Begin Patch\n*** Add: app.x\nfn x() { 1 }\n*** End Patch\n", "every line starts with `+`"),
            ("*** Begin Patch\n*** Add: app.x\n+// only a comment\n*** End Patch\n", "defines no function"),
            ("*** Begin Patch\n*** Update: app.main\nx\n*** End Patch\n", "each line starts with"),
            ("*** Begin Patch\n*** Update: app.main\n+  let y = 2\n*** End Patch\n", "needs `@@ <the line before them>`"),
            ("*** Begin Patch\n*** Update: app.main\n@@ nosuchline\n+  let y = 2\n*** End Patch\n", "no line starts with `@@ nosuchline`"),
            ("*** Begin Patch\n*** Update: app.main\n-  let z = 9\n+  let z = 8\n*** End Patch\n", "these lines weren't found: `let z = 9`"),
            ("*** Begin Patch\n*** Update: app.main\n-  let z = 9\n-  let w = 9\n*** End Patch\n", "(and 1 more)"),
            ("*** Begin Patch\n*** Update: app.main\n-x = 1\n+x = 2\n*** End Patch\n", "`x = 1` is only part of a longer line: `let x = 1`"),
            ("*** Begin Patch\n*** Update: app.main\n-  let x = 1\n+  let x = 1\n*** End Patch\n", "the patch changes nothing"),
            ("*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n-  let x = 1\n-  x\n-}\n*** End Patch\n", "leaves nothing of app.main"),
            ("*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n+pub fn primary() {\n*** End Patch\n", "renames app.main"),
        ];
        for (patch, why) in cases {
            let e = patch_blocks(&s, patch).unwrap_err().0;
            assert!(e.contains("E0108") && e.contains(why), "{why}: {e}");
        }
        // Each refusal says where in the patch it is; a prefix error quotes the line before.
        let bare = "*** Begin Patch\n*** Add: app.x\n+pub fn x() {\n+  1\n}\n*** End Patch\n";
        let e = patch_blocks(&s, bare).unwrap_err().0;
        assert!(e.contains("line 5 of the patch") && e.contains("`}`") && e.contains("(line 4 is `+  1`)"), "{e}");
        let hunk = "*** Begin Patch\n*** Update: app.main\n@@ nosuchline\n+  let y = 2\n*** End Patch\n";
        assert!(patch_blocks(&s, hunk).unwrap_err().0.contains("in app.main (hunk at line 3)"));
        let rename = "*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n+pub fn primary() {\n*** End Patch\n";
        assert!(patch_blocks(&s, rename).unwrap_err().0.contains("renames app.main (section at line 2)"));
        // Units are joined by a NUL line; only a hunk naming that line can remove it.
        let spans = "*** Begin Patch\n*** Update: app\n-}\n-\u{0}\n-pub fn other() {\n+  y\n*** End Patch\n";
        assert!(patch_blocks(&s, spans).unwrap_err().0.contains("a hunk spans two symbols"), "spans");
        let imports = "*** Begin Patch\n*** Update: app\n-import gleam/int\n+import gleam/list\n*** End Patch\n";
        assert!(patch_blocks(&s, imports).unwrap_err().0.contains("changes only app's imports"), "imports");
        assert_eq!(board::snapshot(&s).unwrap(), original);
    }

    /// A patch that doesn't apply is kept as written: the error names its draft
    /// and the line at fault, a `--replace` fixes one line and retries, and the
    /// draft goes stale when a symbol it updates changes.
    #[test]
    fn an_unparsable_patch_is_kept_and_fixed_in_place() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let original = board::snapshot(&s).unwrap();
        let patch = "*** Begin Patch\n*** Add: app.one\n+pub fn one() {\n+  1\n}\n*** Add: app.two\n+pub fn two() {\n+  2\n+}\n*** End Patch\n";
        let err = submit(&s, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
        assert!(err.contains("E0108") && err.contains("line 5 of the patch") && err.contains("Draft"), "{err}");
        let id: String = s.db.query_row("SELECT id FROM drafts", [], |r| r.get(0)).unwrap();
        assert_eq!(view(&s, &id).unwrap(), patch);
        assert_eq!(board::snapshot(&s).unwrap(), original);
        assert!(list(&s).unwrap().ends_with("(current)"));
        let fix = |out: &str| submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok(out.into()), Some("\n}\n*** Add: app.two"));
        let out = fix("\n+}\n*** Add: app.two").unwrap();
        assert!(out.contains("app.one") && out.contains("app.two"), "{out}");
        assert!(list(&s).unwrap().is_empty());
        // A patch draft is stale once a symbol it updates changes.
        let missing = "*** Begin Patch\n*** Update: app.main\n-  nothing\n+  2\n*** End Patch\n";
        assert!(submit(&s, &["submit"], &mut || Ok(missing.into()), None).is_err());
        let id: String = s.db.query_row("SELECT id FROM drafts", [], |r| r.get(0)).unwrap();
        assert!(list(&s).unwrap().ends_with("(current)"));
        s.submit_symbols(Some("app.main"), None, "pub fn main() { 3 }").unwrap();
        assert!(list(&s).unwrap().contains("(stale"));
        assert!(submit(&s, &["submit", "--draft", &id], &mut || Ok(String::new()), None).unwrap_err().0.contains("E0107"));
    }

    /// Hunks match through trailing whitespace and indentation, and a unified
    /// diff's `@@ -3,4 +3,5 @@` gives no context line.
    #[test]
    fn patch_matching_tolerates_whitespace_and_unified_headers() {
        let s = board_project(&[("src/app.gleam", "pub fn main() {\n  let x = 1\n  x\n}\n")]);
        let trailing = "*** Begin Patch\n*** Update: app.main\n-  let x = 1   \n+  let x = 2\n*** End Patch\n";
        let blocks = patch_blocks(&s, trailing).unwrap();
        assert!(blocks.starts_with("// ── app.main ──\n") && blocks.contains("let x = 2") && !blocks.contains("let x = 1"), "{blocks}");
        let indented = "*** Begin Patch\n*** Update: app.main\n-let x = 1\n+  let x = 3\n*** End Patch\n";
        assert!(patch_blocks(&s, indented).unwrap().contains("let x = 3"));
        let unified = "*** Begin Patch\n*** Update: app.main\n@@ -1,4 +1,4 @@\n-  let x = 1\n+  let x = 4\n*** End Patch\n";
        assert!(patch_blocks(&s, unified).unwrap().contains("let x = 4"));
        // `*** Add` of a name that is a symbol of a module goes into that module; any other name is a new module.
        let add = "*** Begin Patch\n*** Add: app.extra\n+pub fn extra() { 1 }\n*** End Patch\n";
        assert!(patch_blocks(&s, add).unwrap().starts_with("// ── app.extra ──\n"));
        let new_module = "*** Begin Patch\n*** Add: fresh\n+pub fn extra() { 1 }\n*** End Patch\n";
        assert!(patch_blocks(&s, new_module).unwrap().starts_with("// ── fresh.extra ──\n"));
    }

    /// A submit that fits no shape gets the usage line; a stored draft with an
    /// unknown command, `E0106`; a replace with no `old`, `E0104`.
    #[test]
    fn submit_refuses_shapes_it_does_not_know() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let e = submit(&s, &["submit", "a", "b", "c"], &mut || Ok(String::new()), None).unwrap_err().0;
        assert!(e.contains("submit [name [--replace"), "{e}");
        for old in [None, Some("")] {
            let e = submit(&s, &["submit", "app.main", "--replace"], &mut || Ok("2".into()), old).unwrap_err().0;
            assert!(e.contains("E0104"), "{old:?}: {e}");
        }
        s.db.execute("INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)", params![owner(), json!(["submit", "x", "y", "z"]).to_string(), "text", ""])
            .unwrap();
        let e = submit(&s, &["submit", "--draft", "d1"], &mut || Ok(String::new()), None).unwrap_err().0;
        assert!(e.contains("E0106") || e.contains("E0107"), "{e}");
    }

    /// A submit names its warnings in one line each; `check --warnings` shows them in full.
    #[test]
    fn warnings_are_summarized_on_submit() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let warning = s.submit_symbols(None, Some("app"), "fn unused() { 1 }").unwrap();
        assert!(warning.contains("app.unused: warning:"), "{warning}");
        assert!(warning.contains("check --warnings"));
        assert!(!warning.contains("│"), "{warning}");
        let full = s.check_report(true).unwrap();
        assert!(full.contains("warning:") && full.contains("│"), "{full}");
    }

    /// D-47: a patch changes a signature and its callers in two modules in
    /// one checked submit, repeated hunks apply in order, `*** Add` adds a
    /// symbol, and a hunk that doesn't match changes nothing.
    #[test]
    fn patch_changes_symbols_across_modules() {
        let s = board_project(&[
            ("src/calc.gleam", "pub fn add(a: Int) -> Int {\n  a + 1\n}\n\npub fn sub(a: Int) -> Int {\n  a - 1\n}\n"),
            ("src/app.gleam", "import calc\n\npub fn main() -> Int {\n  let x = calc.add(1)\n  let y = calc.add(2)\n  x + y\n}\n"),
        ]);
        let original = board::snapshot(&s).unwrap();
        let missing = "*** Begin Patch\n*** Update File: src/calc.gleam\n@@\n-pub fn add(a: Int) -> Int {\n+pub fn add(a: Int, b: Int) -> Int {\n*** Update: app.main\n-  let x = calc.nope(1)\n*** End Patch\n";
        let err = submit(&s, &["submit"], &mut || Ok(missing.into()), None).unwrap_err().0;
        assert!(err.starts_with("E0108 in app.main (hunk at line 7), these lines weren't found: `let x = calc.nope(1)`"), "{err}");
        assert_eq!(board::snapshot(&s).unwrap(), original);
        let patch = "*** Begin Patch\n*** Update File: src/calc.gleam\n@@ pub fn add(\n-pub fn add(a: Int) -> Int {\n-  a + 1\n+pub fn add(a: Int, b: Int) -> Int {\n+  a + b\n*** Update File: src/app.gleam\n@@\n-  let x = calc.add(1)\n+  let x = calc.add(1, 1)\n@@\n-  let y = calc.add(2)\n+  let y = calc.add(2, 1)\n*** Add File: calc.twice\n+pub fn twice(a: Int) -> Int {\n+  add(a, a)\n+}\n*** End Patch\n";
        let out = submit(&s, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("calc.add") && out.contains("app.main") && out.contains("calc.twice") && !out.contains("calc.sub"), "{out}");
        let main = s.view(&["app.main"], false, BUDGET).unwrap();
        assert!(main.contains("calc.add(1, 1)") && main.contains("calc.add(2, 1)"), "{main}");
        // Repeated identical hunks apply in order, one occurrence each.
        let again = "*** Update: app.main\n@@ -2,1 +2,1 @@\n-  let x = calc.add(1, 1)\n+  let x = calc.add(1, 5)\n";
        submit(&s, &["submit"], &mut || Ok(again.into()), None).unwrap();
        assert!(s.view(&["app.main"], false, BUDGET).unwrap().contains("calc.add(1, 5)"));
        let rename = "*** Update: calc.sub\n-pub fn sub(a: Int) -> Int {\n+pub fn minus(a: Int) -> Int {\n";
        assert!(submit(&s, &["submit"], &mut || Ok(rename.into()), None).unwrap_err().0.contains("renames calc.sub"));
        // A type error keeps the resulting blocks as a draft.
        let wrong = "*** Update: calc.sub\n-  a - 1\n+  a -. 1.0\n";
        let err = submit(&s, &["submit"], &mut || Ok(wrong.into()), None).unwrap_err().0;
        assert!(err.contains("Draft"), "{err}");
        assert!(err.contains("the draft holds the blocks"), "{err}");
        let id: String = s.db.query_row("SELECT id FROM drafts ORDER BY rowid DESC LIMIT 1", [], |r| r.get(0)).unwrap();
        assert!(view(&s, &id).unwrap().starts_with("// ── calc.sub ──\npub fn sub"));
    }

    /// D-46: E0105 names each occurrence's line; `--all` replaces every one.
    #[test]
    fn replace_all_and_occurrence_lines() {
        let s = board_project(&[(
            "src/app.gleam",
            "pub fn five() -> Int {\n  let a = 1\n  let b = a + 1\n  let c = b + 1\n  let d = c + 1\n  d + 1\n}\n",
        )]);
        let err = submit(&s, &["submit", "app.five", "--replace"], &mut || Ok(" + 2".into()), Some(" + 1")).unwrap_err().0;
        assert!(err.contains("occurs 4 times (lines 3, 4, 5, 6)") && err.contains("--all"), "{err}");
        assert!(
            submit(&s, &["submit", "app.five", "--replace"], &mut || Ok("x".into()), Some("nowhere"))
                .unwrap_err()
                .0
                .contains("does not occur")
        );
        let out = submit(&s, &["submit", "app.five", "--replace", "--all"], &mut || Ok(" + 2".into()), Some(" + 1")).unwrap();
        assert!(out.starts_with("Replaced 4 occurrences.") && out.contains("Saved app.five"), "{out}");
        assert_eq!(s.view(&["app.five"], false, BUDGET).unwrap().matches(" + 2").count(), 4);
    }
}
