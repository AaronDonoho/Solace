//! Rejected submissions are scratch text, never live symbols.
use super::*;

fn owner() -> String {
    crate::board::context().map(|(task, _)| task).unwrap_or_else(|| env::var("SOLACE_TASK").unwrap_or_else(|_| "cli".into()))
}

/// A draft's base (D-46): the text of the symbols it replaces and the headers
/// of their modules (or of the module it adds to), so edits elsewhere, such as
/// to a callee, leave it current.
fn base(s: &Store, ix: &Index, args: &[String], text: &str) -> Out {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut names: Vec<String> = text.lines().filter_map(|l| l.strip_prefix(BLOCK_HEADER)).map(block_name).collect();
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
    let mut out = vec![];
    for (id, args, current) in entries(s, task)? {
        out.push(format!("{id}: {} ({})", json!(args), status(current)));
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
        0 => fail(format!("E0105 `old` does not occur; read the current text and copy it exactly.{}", miss_hint(text, old))),
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
    text.trim_start()
        .lines()
        .next()
        .is_some_and(|l| ["*** Begin Patch", "*** Update", "*** Add", "*** Delete"].iter().any(|p| l.starts_with(p)))
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

/// Ends an E0108 whose fault is in the whole patch, not in one line (D-141):
/// its draft's footer then leaves out `--replace`. A whole-patch fault's
/// message ends with this sentence; a one-line fault quotes its line, so its
/// message ends with a backtick or `)`, never with this sentence.
pub const RESEND: &str = "Send the corrected patch again, whole, as `text`.";

/// The footer of a refusal kept as draft `id`, retried with `cmd` (`submit` or
/// `write`): it says nothing was saved unless the message already says so,
/// `--replace` fixes a fault in one block or line, and a fault in the whole
/// patch is resent; a draft that isn't a patch says the whole text. The kind is
/// read at the message's end, where a quoted patch line can't reach it. A
/// replacement kept in a refused retry says the draft holds it.
pub fn retry_advice(message: &str, id: &str, cmd: &str, what: &str, several: bool, patch: bool, replaced: bool) -> String {
    let (noun, plural, nothing) =
        if cmd == "write" { ("line", "lines", "Nothing was written.") } else { ("block", "blocks", "Nothing was saved.") };
    let quiet = message.starts_with(if cmd == "write" { "nothing written:" } else { "Not saved:" });
    let prefix = if quiet { String::new() } else { format!("{nothing} ") };
    let lines = if patch { format!(", which are {PATCH_LINES}") } else { String::new() };
    let fix = if several {
        let whole = if patch { "patch" } else { "text" };
        format!(
            concat!(
                "Fix only the failing {}: `{} --draft {} --replace` with `old` and `text`{}; ",
                "the other {} stay in the draft, so don't resend the whole {}.",
            ),
            noun, cmd, id, lines, plural, whole
        )
    } else {
        format!(
            concat!(
                "Fix the failing {}: `{} --draft {} --replace` with `old` and `text`{}; ",
                "the draft keeps your text, so don't resend the whole text.",
            ),
            noun, cmd, id, lines
        )
    };
    let tail = format!("Draft {id} retained{what}. Read: `view --draft {id}`. List: `drafts`.");
    let kept = if replaced {
        format!(
            concat!(
                " The replacement was applied: the draft now holds it, so the next `old` must match ",
                "the draft as it is now (`view --draft {}`).",
            ),
            id
        )
    } else {
        String::new()
    };
    if message.trim_end().ends_with(RESEND) || message.trim_end().ends_with(WHOLE) {
        format!("{prefix}{tail}{kept}")
    } else {
        format!("{prefix}{tail}{kept} {fix}")
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
    let whole = |e: Failure| Failure(format!("{} {RESEND}", e.0), e.1);
    let head = if files { " File" } else { "" };
    let kinds = if files {
        "`*** Add File: <path>`, `*** Update File: <path>` or `*** Delete File: <path>`"
    } else {
        "`*** Update: <name>` or `*** Add: <name>`"
    };
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
            l if l.starts_with("*** Move") => {
                let stored = if l.contains(" to:") { sections.last().map(|(_, target, ..)| target.clone()) } else { Some(target(l)) };
                let advice = match stored {
                    Some(name) => format!("`submit {name}` with the stored name replaces it and may rename it, keeping its id."),
                    None => "submit the stored name with the new text; it replaces the stored symbol and may rename it, keeping its id."
                        .to_string(),
                };
                return Err(here(format!("a patch can't rename; {advice}")));
            }
            l if l.starts_with("***") => {
                return Err(whole(here(format!("`{}` is no section header; a section starts with {kinds}.", clip(l)))));
            }
            l => {
                let Some((kind, target, at, hunks)) = sections.last_mut() else {
                    let what = if files { "a file patch" } else { "a patch" };
                    return Err(whole(here(if l.trim().is_empty() {
                        format!("{what} must start with `*** Begin Patch`; remove the blank line before it.")
                    } else {
                        format!("{what} starts with `*** Begin Patch`, then {kinds}.")
                    })));
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
                    // an Add with no `+` line at all is a fault in the whole patch
                    let mut rest = raw.lines().skip(n).take_while(|x| !x.starts_with("***"));
                    if hunks[0].1.is_empty() && !rest.any(|x| x.starts_with('+')) {
                        return Err(whole(bad(format!(
                            "`*** Add{head}: {target}` (line {at}) has no `+` lines; every line of an Add, empty ones too, starts with `+`."
                        ))));
                    }
                    return Err(here(format!(
                        "in `*** Add{head}: {target}` every line, empty ones too, starts with `+`, \
                         and `old` and `text` are {PATCH_LINES}; this one doesn't: `{}`",
                        clip(l)
                    )));
                } else if update && !l.is_empty() && !l.starts_with([' ', '-', '+']) {
                    return Err(here(format!(
                        "in `*** Update{head}: {target}` each line starts with ` `, `-` or `+` (or `@@`), \
                         and `old` and `text` are {PATCH_LINES}; this one doesn't: `{}`",
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
        return Err(whole(bad(format!("the patch has no {kinds} section."))));
    }
    Ok(sections)
}

/// Applies a section's hunks to `lines` in order, as `apply_patch` does: each
/// from where the last one ended, matched exactly, then ignoring trailing
/// space. A code hunk (`at_end` false) that matches neither way falls back to
/// ignoring space at both ends, but only when it matches in exactly one place;
/// its context and `-` lines keep the target's text, and its `+` lines take
/// the target's indentation (D-204). Otherwise leading space is part of the
/// line: the first hunk line the target has only with other leading space is
/// refused, naming that line (nearest the hunk) and how much space it has
/// (D-142); hunk lines all present but not adjacent are refused as such,
/// naming a blank line the target has none of. A hunk that only adds lines
/// goes after its `@@` line; with none, it goes at the end if `at_end` (a
/// file), else is refused (a symbol).
/// `show` is the command that shows the target's current text.
fn apply_hunks(target: &str, show: &str, lines: &mut Vec<String>, hunks: Vec<Hunk>, at_end: bool) -> Result<bool, Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    let mut cursor = 0;
    let mut indented = false;
    for (at, hunk, hl) in hunks {
        let old: Vec<&str> = hunk.iter().filter(|l| !l.starts_with('+')).map(|l| &l[1..]).collect();
        let mut from = cursor;
        if let Some(at) = &at {
            let found = (cursor..lines.len()).chain(0..cursor).find(|&k| lines[k].trim().starts_with(at.as_str()));
            from = found.ok_or_else(|| {
                bad(format!("in {target} (hunk at line {hl}), no line starts with `@@ {at}`; `{show}` shows the current text."))
            })? + 1;
        }
        let at_k = if old.is_empty() {
            match at {
                Some(_) => Some((from, false)),
                None if at_end => Some((lines.len(), false)),
                None => {
                    return Err(bad(format!(
                        "in {target} (hunk at line {hl}), a hunk that only adds lines needs `@@ <the line before them>`."
                    )));
                }
            }
        } else {
            // leading space is part of the line; only trailing space is forgiven (D-142)
            let same: [&dyn Fn(&str, &str) -> bool; 2] = [&|a, b| a == b, &|a, b| a.trim_end() == b.trim_end()];
            let fits = |k: usize, eq: &dyn Fn(&str, &str) -> bool| {
                k + old.len() <= lines.len() && old.iter().zip(&lines[k..]).all(|(o, l)| eq(o, l))
            };
            match same.iter().find_map(|eq| (from..lines.len()).chain(0..from).find(|&k| fits(k, *eq))) {
                Some(k) => Some((k, false)),
                None if !at_end => loose_match(&old, lines),
                None => None,
            }
        };
        let Some((k, loose)) = at_k else {
            let first = old.iter().find(|l| !l.trim().is_empty()).unwrap_or(&"");
            // the first hunk line the target doesn't have exactly (trailing space aside): the line at fault
            let at_fault = old.iter().filter(|l| !l.trim().is_empty()).find(|l| !lines.iter().any(|x| x.trim_end() == l.trim_end()));
            // a line that is only part of one longer line: patch lines are whole lines (D-120)
            let part = old.iter().map(|l| l.trim()).find(|l| !l.is_empty() && !lines.iter().any(|x| x.trim() == *l));
            let within: Vec<usize> = part.map_or(vec![], |p| (0..lines.len()).filter(|&k| lines[k].contains(p)).collect());
            // the line at fault, found with other leading space: name its line nearest the hunk, so the patch copies it whole (D-142)
            let shifted = match (part, at_fault) {
                (None, Some(l)) => {
                    // where the hunk line belongs: search outward from there, both ways, so the nearest occurrence wins
                    let want = from + old.iter().position(|o| o == l).unwrap_or(0);
                    (0..lines.len()).filter(|&k| lines[k].trim() == l.trim()).min_by_key(|&k| k.abs_diff(want))
                }
                _ => None,
            };
            let hint = match (part, within.as_slice(), shifted) {
                (Some(p), [k], _) => {
                    let (line, fix) = if at_end {
                        (format!("line {}", k + 1), format!("`write {target}`"))
                    } else {
                        ("a longer line".to_string(), "`submit <name> --replace`".to_string())
                    };
                    format!(
                        concat!(
                            " `{}` is only part of {}: `{}`. ",
                            "A patch line is a whole line: copy all of it, or change just that text with {} and `old` and `text`.",
                        ),
                        clip(p),
                        line,
                        clip(lines[*k].trim()),
                        fix
                    )
                }
                (None, _, Some(k)) => format!(
                    " `{}` is line {} with {}; a patch line is a whole line: copy all of it.",
                    clip(lines[k].trim()),
                    k + 1,
                    count(lines[k].len() - lines[k].trim_start().len(), "leading space")
                ),
                _ => String::new(),
            };
            let more = if old.len() > 1 { format!(" (and {} more)", old.len() - 1) } else { String::new() };
            // when the D-120 hint shows, the headline names the same line the hint names
            let quote = match (part, within.as_slice()) {
                (Some(p), [_]) => p,
                _ => at_fault.map(|l| l.trim()).unwrap_or(first.trim()),
            };
            let msg = if part.is_none() && shifted.is_none() && !first.trim().is_empty() {
                let blank = old.iter().any(|l| l.trim().is_empty()) && !lines.iter().any(|x| x.trim_end().is_empty());
                format!(
                    "these lines aren't adjacent in the file: `{}`{more}; copy them in order from `{show}`.{}",
                    first.trim(),
                    if blank { " The hunk has a blank line; the target has none." } else { "" }
                )
            } else {
                format!("these lines weren't found: `{}`{more}; copy them exactly from `{show}`.{hint}", quote)
            };
            return Err(bad(format!("in {target} (hunk at line {hl}), {msg}")));
        };
        indented |= loose;
        let new = replacement(&hunk, &old, lines, k, loose);
        cursor = k + new.len();
        lines.splice(k..k + old.len(), new);
    }
    Ok(indented)
}

/// A file's new text from a `write` patch section (D-104): an Add's `+` lines,
/// an Update's hunks applied to `current`, or `None` for a Delete.
pub fn file_text(path: &str, current: &str, kind: Section, hunks: Vec<Hunk>) -> Result<Option<String>, Failure> {
    match kind {
        Section::Delete => Ok(None),
        Section::Add => Ok(Some(hunks[0].1.iter().map(|l| format!("{}\n", &l[1..])).collect())),
        Section::Update => {
            let mut lines: Vec<String> = current.lines().map(String::from).collect();
            let _ = apply_hunks(path, &format!("read {path}"), &mut lines, hunks, true)?;
            let mut text = lines.join("\n");
            // the file keeps its last newline; one that had no text gets one
            if !lines.is_empty() && (current.ends_with('\n') || current.is_empty()) {
                text.push('\n');
            }
            Ok(Some(text))
        }
    }
}

/// One `// ── <module>.<name> ──` block: a module's imports (its header), once,
/// before the text of the first item they go with.
fn push_block(blocks: &mut String, header: &mut String, module: &str, name: &str, text: &str) {
    push_named(blocks, header, &format!("{module}.{name}"), text);
}

/// Pushes one block per item a whole-module patch changed or added (D-168); a
/// container's own text can't go through a named submit, so it is refused.
fn push_module_blocks(
    blocks: &mut String,
    ix: &Index,
    module: &str,
    items: &[crate::rust_submit::RustItem],
    at_line: usize,
    bad: &impl Fn(String) -> Failure,
) -> Result<(), Failure> {
    for item in items {
        let changed = match item.stored {
            Some(i) => ix.syms[i].text != item.text || ix.syms[i].kind_text != item.kind,
            None => true,
        };
        if changed {
            if crate::rust_submit::container_kind(item.kind) || !item.children.is_empty() {
                let how = match item.stored {
                    Some(_) => "change it with `submit <module>`".to_string(),
                    None => store::block_note(item.kind, None),
                };
                return Err(bad(format!("the patch changes the `{}` block {} (section at line {at_line}); {how}.", item.kind, item.name,)));
            }
            push_block(blocks, &mut String::new(), module, &item.name, &item.text);
        }
        push_module_blocks(blocks, ix, module, &item.children, at_line, bad)?;
    }
    Ok(())
}

/// The blocks from a patch, without the flag that says a hunk matched through
/// indentation.
#[cfg(test)]
pub fn patch_blocks(s: &Store, raw: &str) -> Result<String, Failure> {
    patch_blocks_with_indent(s, raw).map(|(blocks, _)| blocks)
}

pub fn submit(s: &Store, a: &[&str], input: &mut dyn FnMut() -> Out, old: Option<&str>) -> Out {
    // `--all`: `--replace` changes every occurrence of `old` (D-46)
    let all = a.contains(&"--all");
    let a: Vec<&str> = a.iter().copied().filter(|x| *x != "--all").collect();
    let mut replaced = 0;
    let ix = s.index()?;
    let (id, args, raw) = match a.as_slice() {
        ["submit", "--draft", id, "--imports", module, ..] | ["submit", "--imports", module, "--draft", id, ..] => {
            return fail(format!("E0109 send these as two calls: `submit --imports {module}` first, then `submit --draft {id}`"));
        }
        ["submit", "--draft", id] | ["submit", "--draft", id, "--replace"] => {
            let (args, text, expected) = load(s, id)?;
            if args.first().is_some_and(|c| c == "write") {
                return fail(format!("E0106 draft {id} is a file patch; retry it with `write --draft {id}`"));
            }
            if base(s, &ix, &args, &text)? != expected {
                return fail(format!(
                    concat!(
                        "E0107 draft {} is stale: a symbol it replaces (or its module's imports) changed since it was created. ",
                        "`view --draft {}` preserves your text; inspect current source and make a new submission.",
                    ),
                    id, id
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
    let mut indented = false;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match refs.as_slice() {
        // a patch that applies is submitted as the blocks it changes; if those are rejected, the draft keeps them
        ["submit"] if is_patch(&raw) => patch_blocks_with_indent(s, &raw).and_then(|(blocks, loose)| {
            indented = loose;
            let result = s.submit_symbols_blocks(None, None, &blocks, true);
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
            let out = if indented {
                format!("{out}\nA hunk matched through indentation: its added lines took the target's indentation.")
            } else {
                out
            };
            Ok(if replaced > 1 { format!("Replaced {replaced} occurrences.\n{out}") } else { out })
        }
        Err(Failure(message, code)) => {
            let base = base(s, &ix, &args, &raw)?;
            s.db.execute(
                "INSERT OR REPLACE INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,?)",
                params![id, owner(), json!(args).to_string(), raw, base],
            )?;
            let what = if as_blocks { " (the patch applied; the draft holds the blocks it changed)" } else { "" };
            let several = several_blocks(s, &raw);
            let advice = retry_advice(&message, &id, "submit", what, several, is_patch(&raw), replaced > 0);
            Err(Failure(format!("{message}\n\n{advice}"), code))
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
        let s = board_project(&[(
            "src/app.gleam",
            concat!(
                "import gleam/int\n\npub fn main() {\n  let x = 1\n  x\n}\n\n",
                "pub fn other() {\n  2\n}\n\n",
                "pub fn one() {\n let x = 1\n x\n}\n\n",
                "pub fn rep(x: Int) -> Int {\n  x\n  let a = 1\n  let b = 2\n  let c = 3\n  let d = 4\n  x\n}\n",
            ),
        )]);
        let original = board::snapshot(&s).unwrap();
        let cases: [(&str, &str); 18] = [
            ("*** Begin Patch\n*** Delete: app.main\n*** End Patch\n", "a patch can't delete"),
            ("*** Begin Patch\n*** Move: app.main\n*** End Patch\n", "a patch can't rename"),
            ("+pub fn x() { 1 }\n", "a patch starts with `*** Begin Patch`"),
            ("*** Begin Patch\n*** End Patch\n", "the patch has no"),
            ("*** Begin Patch\n*** Add: app.x\nfn x() { 1 }\n*** End Patch\n", "has no `+` lines"),
            ("*** Begin Patch\n*** Add: app.x\n+// only a comment\n*** End Patch\n", "defines no function"),
            ("*** Begin Patch\n*** Update: app.main\nx\n*** End Patch\n", "each line starts with"),
            ("*** Begin Patch\n*** Update: app.main\n+  let y = 2\n*** End Patch\n", "needs `@@ <the line before them>`"),
            ("*** Begin Patch\n*** Update: app.main\n@@ nosuchline\n+  let y = 2\n*** End Patch\n", "no line starts with `@@ nosuchline`"),
            (
                "*** Begin Patch\n*** Update: app.main\n-  let z = 9\n+  let z = 8\n*** End Patch\n",
                "these lines weren't found: `let z = 9`",
            ),
            ("*** Begin Patch\n*** Update: app.main\n-  let z = 9\n-  let w = 9\n*** End Patch\n", "(and 1 more)"),
            (
                "*** Begin Patch\n*** Update: app.main\n-x = 1\n+x = 2\n*** End Patch\n",
                "`x = 1` is only part of a longer line: `let x = 1`",
            ),
            (
                "*** Begin Patch\n*** Update: app.main\n-  let x = 1\n-}\n+  let x = 2\n*** End Patch\n",
                "these lines aren't adjacent in the file",
            ),
            (
                "*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n-\n-  let x = 1\n+pub fn main() {\n+  let x = 2\n*** End Patch\n",
                "The hunk has a blank line; the target has none.",
            ),
            (
                "*** Begin Patch\n*** Update: app.main\n-let x = 1\n-x = 1\n+let x = 2\n+x = 2\n*** End Patch\n",
                "these lines weren't found: `x = 1` (and 1 more)",
            ),
            ("*** Begin Patch\n*** Update: app.main\n-  let x = 1\n+  let x = 1\n*** End Patch\n", "the patch changes nothing"),
            (
                "*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n-  let x = 1\n-  x\n-}\n*** End Patch\n",
                "leaves nothing of app.main",
            ),
            ("*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n+pub fn primary() {\n*** End Patch\n", "renames app.main"),
        ];
        for (patch, why) in cases {
            let e = patch_blocks(&s, patch).unwrap_err().0;
            assert!(e.contains("E0108") && e.contains(why), "{why}: {e}");
        }
        // the hint names the first hunk line with no exact match, not the hunk's first line (D-142)
        let e = patch_blocks(
            &s,
            "*** Begin Patch\n*** Update: app.rep\n-pub fn rep(x: Int) -> Int {\n-x\n-}\n+pub fn rep(x: Int) -> Int {\n+y\n+}\n*** End Patch\n",
        )
        .unwrap_err()
        .0;
        assert!(e.contains("`x` is line 2 with 2 leading spaces") && !e.contains("`pub fn rep(x: Int) -> Int {` is line"), "{e}");
        // the headline quotes the line at fault, not the hunk's first line
        assert!(e.contains("these lines weren't found: `x` (and 2 more)"), "{e}");
        // hunk lines all present but not adjacent: the refusal says so, with no indentation hint
        let e = patch_blocks(&s, "*** Begin Patch\n*** Update: app.main\n-  let x = 1\n-}\n+  let x = 2\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("aren't adjacent in the file") && !e.contains("leading space"), "{e}");
        // when the D-120 hint shows, the headline quotes the line the hint names, not the misindented line before it
        let e = patch_blocks(&s, "*** Begin Patch\n*** Update: app.main\n-let x = 1\n-x = 1\n+let x = 2\n+x = 2\n*** End Patch\n")
            .unwrap_err()
            .0;
        assert!(e.contains("these lines weren't found: `x = 1`") && e.contains("`x = 1` is only part of a longer line"), "{e}");
        // the search goes outward from where the hunk line belongs: an occurrence 2 lines before it beats one 3 lines after it
        let e = patch_blocks(&s, "*** Begin Patch\n*** Update: app.rep\n@@ let a\n-x\n+y\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("`x` is line 2 with 2 leading spaces"), "{e}");
        // Each refusal says where in the patch it is; a prefix error quotes the line before.
        let bare = "*** Begin Patch\n*** Add: app.x\n+pub fn x() {\n+  1\n}\n*** End Patch\n";
        let e = patch_blocks(&s, bare).unwrap_err().0;
        assert!(e.contains("line 5 of the patch") && e.contains("`}`") && e.contains("(line 4 is `+  1`)"), "{e}");
        let hunk = "*** Begin Patch\n*** Update: app.main\n@@ nosuchline\n+  let y = 2\n*** End Patch\n";
        assert!(patch_blocks(&s, hunk).unwrap_err().0.contains("in app.main (hunk at line 3)"));
        let rename = "*** Begin Patch\n*** Update: app.main\n-pub fn main() {\n+pub fn primary() {\n*** End Patch\n";
        let e = patch_blocks(&s, rename).unwrap_err().0;
        assert!(
            e.contains("renames app.main (section at line 2)")
                && e.contains("`submit app.main` with the new text renames it and keeps its id")
                && e.contains("change its callers in the same submit")
                && !e.contains("`*** Add:`"),
            "{e}"
        );
        let e = patch_blocks(&s, "*** Begin Patch\n*** Move: app.main\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("submit app.main") && e.contains("keeping its id"), "{e}");
        let moved = "*** Begin Patch\n*** Update: app.main\n@@\n-  let x = 1\n+  let x = 2\n*** Move to: app.primary\n*** End Patch\n";
        let e = patch_blocks(&s, moved).unwrap_err().0;
        assert!(e.contains("submit app.main") && !e.contains("submit app.primary"), "{e}");
        let after = "*** Begin Patch\n*** Update: app.other\n@@\n-  2\n+  3\n*** Move: app.main\n*** End Patch\n";
        let e = patch_blocks(&s, after).unwrap_err().0;
        assert!(e.contains("submit app.main") && !e.contains("submit app.other"), "{e}");
        // Units are joined by a NUL line; only a hunk naming that line can remove it.
        let spans = "*** Begin Patch\n*** Update: app\n-}\n-\u{0}\n-pub fn other() {\n+  y\n*** End Patch\n";
        assert!(patch_blocks(&s, spans).unwrap_err().0.contains("a hunk spans two symbols"), "spans");
        let imports = "*** Begin Patch\n*** Update: app\n-import gleam/int\n+import gleam/list\n*** End Patch\n";
        let e = patch_blocks(&s, imports).unwrap_err().0;
        assert!(
            e.contains("changes only app's imports")
                && e.contains("submit --imports app")
                && !e.contains("imports are added automatically"),
            "imports: {e}"
        );
        assert_eq!(board::snapshot(&s).unwrap(), original);
    }

    /// A patch that doesn't apply is kept as written: the error names its draft
    /// and the line at fault, a `--replace` fixes one line and retries, and the
    /// draft goes stale when a symbol it updates changes.
    #[test]
    fn an_unparsable_patch_is_kept_and_fixed_in_place() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let original = board::snapshot(&s).unwrap();
        let patch =
            "*** Begin Patch\n*** Add: app.one\n+pub fn one() {\n+  1\n}\n*** Add: app.two\n+pub fn two() {\n+  2\n+}\n*** End Patch\n";
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

    /// D-141: a fault in the whole patch, not in one line (an Add with no
    /// `+` line, no section, a line before the first header, an unknown
    /// header, a file section sent to `submit`) says to resend the patch, not
    /// `--replace`; one line missing its `+` keeps `--replace`, even when its
    /// text is the resend sentence.
    /// A new module comes from `+` Add sections, each section's imports kept,
    /// and E1003 points to that form.
    #[test]
    fn whole_patch_faults_say_to_resend() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("src/util.gleam", "pub fn two() { 2 }\n")]);
        for (patch, why) in [
            (
                "*** Begin Patch\n*** Add: kb.one\npub fn one() { 1 }\n*** End Patch\n",
                "`*** Add: kb.one` (line 2) has no `+` lines; every line of an Add, empty ones too, starts with `+`",
            ),
            ("*** Begin Patch\n*** End Patch\n", "the patch has no `*** Update: <name>` or `*** Add: <name>` section"),
            (
                "\n*** Begin Patch\n*** Add: app.x\n+pub fn x() { 1 }\n*** End Patch\n",
                "line 1 of the patch: a patch must start with `*** Begin Patch`; remove the blank line before it",
            ),
            (
                "*** Begin Patch\n*** Ad: app.x\n+pub fn x() { 1 }\n*** End Patch\n",
                "line 2 of the patch: `*** Ad: app.x` is no section header",
            ),
            ("*** Begin Patch\n*** Add: app.x\n+// only a comment\n*** End Patch\n", "defines no function"),
            ("*** Begin Patch\n*** Update File: notes\n-one\n+1\n*** End Patch\n", "is no symbol or module"),
        ] {
            let e = submit(&s, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
            assert!(
                e.contains("E0108")
                    && e.contains(why)
                    && e.contains(RESEND)
                    && e.contains("Draft")
                    && e.contains("Nothing was saved.")
                    && !e.contains("--replace"),
                "{why}: {e}"
            );
        }
        // One line without `+`, an empty one too, is a one-line fault: `--replace` fixes it.
        let empty = "*** Begin Patch\n*** Add: app.x\n+pub fn x() {\n\n+  1\n+}\n*** End Patch\n";
        let e = submit(&s, &["submit"], &mut || Ok(empty.into()), None).unwrap_err().0;
        assert!(
            e.contains("line 4 of the patch: in `*** Add: app.x` every line, empty ones too, starts with `+`")
                && e.contains(PATCH_LINES)
                && e.contains("Nothing was saved.")
                && e.contains("Fix the failing block")
                && e.contains("don't resend the whole text")
                && !e.contains("other blocks")
                && e.contains("--replace")
                && !e.contains(RESEND),
            "{e}"
        );
        // A quoted line that is the resend sentence is still a one-line fault.
        let quoted = format!("*** Begin Patch\n*** Add: app.x\n+pub fn x() {{ 1 }}\n{RESEND}\n*** End Patch\n");
        let e = submit(&s, &["submit"], &mut || Ok(quoted.clone()), None).unwrap_err().0;
        assert!(e.contains("line 4 of the patch") && e.contains("Nothing was saved.") && e.contains("--replace"), "{e}");
        // A new module from two Add sections keeps its module comment and both sections' imports.
        let module = concat!(
            "*** Begin Patch\n*** Add: kb.one\n+//// Knowledge.\n+import app\n+\n+pub fn one() -> Int { app.main() }\n",
            "*** Add: kb.two\n+import util\n+\n+pub fn two() -> Int { util.two() }\n*** End Patch\n",
        );
        let out = submit(&s, &["submit"], &mut || Ok(module.into()), None).unwrap();
        assert!(out.contains("kb.one") && out.contains("kb.two"), "{out}");
        let text = std::fs::read_to_string(s.root.join("src/kb.gleam")).unwrap();
        assert!(text.starts_with("//// Knowledge.") && text.contains("import app\n") && text.contains("import util\n"), "{text}");
        // An unknown module's imports point to the one way to create it.
        let e = submit(&s, &["submit", "--imports", "nope"], &mut || Ok("import app".into()), None).unwrap_err().0;
        assert!(e.starts_with("E1003 unknown module: nope; create it with a patch `*** Add: nope`"), "{e}");
    }

    /// Hunks match through trailing whitespace, and code hunks through space at
    /// either end when the fit is unique (D-204); a unified diff's
    /// `@@ -3,4 +3,5 @@` gives no context line.
    #[test]
    fn patch_matching_ignores_trailing_space_and_unified_headers() {
        let s = board_project(&[("src/app.gleam", "pub fn main() {\n  let x = 1\n  x\n}\n")]);
        let trailing = "*** Begin Patch\n*** Update: app.main\n-  let x = 1   \n+  let x = 2\n*** End Patch\n";
        let blocks = patch_blocks(&s, trailing).unwrap();
        assert!(blocks.starts_with("// ── app.main ──\n") && blocks.contains("let x = 2") && !blocks.contains("let x = 1"), "{blocks}");
        // a code hunk also matches through leading space, and its additions take the target's depth (D-204)
        let indented = "*** Begin Patch\n*** Update: app.main\n-let x = 1\n+let x = 3\n*** End Patch\n";
        assert!(patch_blocks(&s, indented).unwrap().contains("\n  let x = 3\n"));
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
        s.db.execute(
            "INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)",
            params![owner(), json!(["submit", "x", "y", "z"]).to_string(), "text", ""],
        )
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
        let missing = concat!(
            "*** Begin Patch\n*** Update File: src/calc.gleam\n@@\n",
            "-pub fn add(a: Int) -> Int {\n+pub fn add(a: Int, b: Int) -> Int {\n",
            "*** Update: app.main\n-  let x = calc.nope(1)\n*** End Patch\n",
        );
        let err = submit(&s, &["submit"], &mut || Ok(missing.into()), None).unwrap_err().0;
        assert!(err.starts_with("E0108 in app.main (hunk at line 7), these lines weren't found: `let x = calc.nope(1)`"), "{err}");
        assert!(
            err.contains("Fix only the failing block")
                && err.contains("the other blocks stay")
                && err.contains("don't resend the whole patch"),
            "{err}"
        );
        assert_eq!(board::snapshot(&s).unwrap(), original);
        let patch = concat!(
            "*** Begin Patch\n*** Update File: src/calc.gleam\n@@ pub fn add(\n",
            "-pub fn add(a: Int) -> Int {\n-  a + 1\n+pub fn add(a: Int, b: Int) -> Int {\n+  a + b\n",
            "*** Update File: src/app.gleam\n@@\n-  let x = calc.add(1)\n+  let x = calc.add(1, 1)\n",
            "@@\n-  let y = calc.add(2)\n+  let y = calc.add(2, 1)\n",
            "*** Add File: calc.twice\n+pub fn twice(a: Int) -> Int {\n+  add(a, a)\n+}\n*** End Patch\n",
        );
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
        assert!(
            !err.contains("Nothing was saved.")
                && err.contains("Fix the failing block")
                && !err.contains("other blocks")
                && err.contains("--replace"),
            "{err}"
        );
        let id: String = s.db.query_row("SELECT id FROM drafts ORDER BY rowid DESC LIMIT 1", [], |r| r.get(0)).unwrap();
        assert!(view(&s, &id).unwrap().starts_with("// ── calc.sub ──\npub fn sub"));
        // A type error after several blocks says the other blocks stay.
        let wrong_both = concat!("*** Update: calc.add\n-  a + b\n+  a -. b\n", "*** Update: calc.sub\n-  a - 1\n+  a -. 1.0\n",);
        let err = submit(&s, &["submit"], &mut || Ok(wrong_both.into()), None).unwrap_err().0;
        assert!(
            err.contains("Fix only the failing block")
                && err.contains("the other blocks stay")
                && err.contains("don't resend the whole text"),
            "{err}"
        );
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

    /// A Rust update that adds an item after unchanged context adds it; it is
    /// not a rename of the stored symbol.
    #[test]
    fn an_add_after_unchanged_context_is_an_add() {
        let (_root, s) = rust_project("pub fn main() {\n    1\n}\n");
        let patch = concat!(
            "*** Begin Patch\n*** Update: crate.main\n@@\n",
            " pub fn main() {\n     1\n }\n",
            "+pub fn extra() {\n+    2\n+}\n",
            "*** End Patch\n",
        );
        let blocks = patch_blocks(&s, patch).unwrap();
        assert!(blocks.contains("// ── crate.main ──"), "{blocks}");
        assert!(blocks.contains("// ── crate.extra ──") && blocks.contains("pub fn extra()"), "{blocks}");
        let (_root, s) = rust_project("pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n");
        let patch = concat!(
            "*** Begin Patch\n*** Update: crate.Store::open\n@@\n",
            "     pub fn open(&self) -> i32 {\n",
            "         2\n",
            "     }\n",
            "+    pub fn extra(&self) -> i32 {\n",
            "+        3\n",
            "+    }\n",
            "*** End Patch\n",
        );
        let blocks = patch_blocks(&s, patch).unwrap();
        assert!(blocks.contains("// ── crate.Store::open ──"), "{blocks}");
        assert!(blocks.contains("// ── crate.Store::extra ──") && blocks.contains("pub fn extra"), "{blocks}");
    }

    /// A `module:` prefix and a source path name the header that works; an
    /// unknown name lists the nearest stored names.
    #[test]
    fn an_unknown_target_names_the_header_or_the_names() {
        let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let header = patch_blocks(&s, "*** Begin Patch\n*** Update: module:crate\n*** End Patch\n").unwrap_err().0;
        assert!(header.contains("`*** Update: crate`") && header.contains("help submit"), "{header}");
        let path = patch_blocks(&s, "*** Begin Patch\n*** Update: src/lib.rs\n*** End Patch\n").unwrap_err().0;
        assert!(path.contains("`*** Update: crate`"), "{path}");
        let unknown = patch_blocks(&s, "*** Begin Patch\n*** Update: crate.nope\n*** End Patch\n").unwrap_err().0;
        assert!(unknown.contains("Similar names:") && unknown.contains("crate.one"), "{unknown}");
    }

    /// A type added with its impl and a later method's Add make one impl block
    /// for the submit to save (D-194).
    #[test]
    fn a_patch_add_holds_one_impl() {
        let (_root, s) = rust_project("pub fn one() -> i32 {\n    1\n}\n");
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Add: crate.X\n",
            "+pub struct X;\n",
            "*** Add: crate.X::new\n",
            "+pub fn new() -> Self {\n",
            "+    Self\n",
            "+}\n",
            "*** End Patch\n",
        );
        let blocks = patch_blocks(&s, patch).unwrap();
        assert_eq!(blocks.matches("impl X {").count(), 1, "{blocks}");
        assert!(blocks.contains("pub fn new() -> Self"), "{blocks}");
        assert!(!blocks.contains("// ── crate.X.new ──"), "{blocks}");
    }

    /// Impls for one type with different arguments stay separate (D-194).
    #[test]
    fn a_patch_keeps_impls_with_different_arguments_apart() {
        let (_root, s) = rust_project("pub struct X<T>(T);\n");
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Add: crate\n",
            "+impl X<i32> {\n",
            "+    pub fn a() {}\n",
            "+}\n",
            "+impl X<u8> {\n",
            "+    pub fn b() {}\n",
            "+}\n",
            "*** End Patch\n",
        );
        let blocks = patch_blocks(&s, patch).unwrap();
        assert!(blocks.contains("impl X<i32>") && blocks.contains("impl X<u8>"), "{blocks}");
    }

    /// A method Add cannot choose between two impls of one type (D-194).
    #[test]
    fn a_patch_refuses_an_ambiguous_method_add() {
        let (_root, s) = rust_project("pub struct X<T>(T);\n");
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Add: crate\n",
            "+impl X<i32> {\n",
            "+    pub fn a() {}\n",
            "+}\n",
            "+impl X<u8> {\n",
            "+    pub fn b() {}\n",
            "+}\n",
            "*** Add: crate.X::new\n",
            "+pub fn c() {}\n",
            "*** End Patch\n",
        );
        let err = patch_blocks(&s, patch).unwrap_err().0;
        assert!(err.contains("matches more than one impl") && err.contains("X<i32>") && err.contains("X<u8>"), "{err}");
    }

    /// A `submit --new` with several symbols says the other blocks stay and to
    /// resend the whole text, not a patch.
    #[test]
    fn a_multi_symbol_new_submit_says_the_other_blocks_stay() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let text = "pub fn one() -> Int { 1 }\n\npub fn two() -> Int { \"two\" }\n";
        let err = submit(&s, &["submit", "--new", "app"], &mut || Ok(text.into()), None).unwrap_err().0;
        assert!(
            err.contains("Fix only the failing block")
                && err.contains("the other blocks stay")
                && err.contains("don't resend the whole text")
                && !err.contains("whole patch"),
            "{err}"
        );
    }

    /// A dead-code warning the submit leaves on another symbol is named under
    /// its own heading.
    #[test]
    fn a_submit_names_a_warning_it_leaves_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(out.contains("New warnings elsewhere:"), "{out}");
        assert!(out.contains("app.tools: warning:"), "{out}");
    }

    /// A warning that was there before the submit stays quiet.
    #[test]
    fn a_submit_does_not_repeat_a_warning_from_before() {
        let s = project(&[("src/app.gleam", "fn unused() {\n  1\n}\n\npub fn main() {\n  1\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  2\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
        assert!(!out.contains("app.unused"), "{out}");
    }

    /// A whole-module submit names a warning the compiler newly reports elsewhere.
    #[test]
    fn a_whole_module_submit_names_a_warning_it_leaves_elsewhere() {
        let s = project(&[
            ("src/app.gleam", "pub fn value() {\n  1\n}\n"),
            ("src/other.gleam", "import app\n\npub fn use_value() {\n  app.value()\n}\n"),
        ]);
        s.check_report(false).unwrap();
        let out = s.submit(crate::Target::Module("app"), "@deprecated(\"old\")\npub fn value() {\n  1\n}\n").unwrap();
        assert!(out.contains("New warnings elsewhere:"), "{out}");
        assert!(out.contains("other.use_value: warning:"), "{out}");
    }

    /// Before any check has run there is no baseline, so no warning is new.
    #[test]
    fn a_submit_before_a_check_does_not_name_a_warning_elsewhere() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  1\n}\n").unwrap();
        assert!(!out.contains("New warnings elsewhere"), "{out}");
    }

    /// Both warning sections share one footer.
    #[test]
    fn a_submit_prints_the_diagnostics_footer_once() {
        let s = project(&[("src/app.gleam", "fn tools() {\n  1\n}\n\npub fn main() {\n  tools()\n}\n")]);
        s.check_report(false).unwrap();
        let out = s.submit_symbols(Some("app.main"), None, "pub fn main() {\n  todo\n}\n").unwrap();
        assert!(out.contains("\nWarnings:"), "{out}");
        assert!(out.contains("\nNew warnings elsewhere:"), "{out}");
        assert_eq!(out.matches("Use `check --warnings` for full diagnostics.").count(), 1, "{out}");
    }

    /// A module's warning stays in the baseline when a later check omits it.
    #[test]
    fn a_cached_modules_warning_is_not_new_on_a_later_submit() {
        let s = project(&[("src/a.gleam", "fn unused() {\n  1\n}\n"), ("src/b.gleam", "pub fn main() {\n  1\n}\n")]);
        s.check_report(false).unwrap();
        for body in ["  2\n", "  3\n"] {
            let out = s.submit(crate::Target::Module("b"), &format!("pub fn main() {{\n{body}}}\n")).unwrap();
            assert!(!out.contains("a.unused"), "{out}");
        }
    }

    /// E0105 names the line nearest `old`'s first non-blank line: the one
    /// equal once space is trimmed, else the one sharing the longest start;
    /// when only leading space differs, both counts are named (D-142).
    #[test]
    fn a_miss_names_the_nearest_line() {
        let s = board_project(&[("src/app.gleam", "pub fn five() -> Int {\n  let a = 1\n  let b = a + 1\n  b\n}\n")]);
        let miss = |old: &str| submit(&s, &["submit", "app.five", "--replace"], &mut || Ok("x".into()), Some(old)).unwrap_err().0;
        let e = miss("  let b = a + 2");
        assert!(e.contains("E0105 `old` does not occur") && e.contains("The nearest line is 3: `  let b = a + 1`"), "{e}");
        let e = miss("   let b = a + 1");
        assert!(e.contains("The nearest line is 3, with 2 leading spaces: `let b = a + 1`; your `old` line has 3"), "{e}");
        let e = miss("  let a = 1\n  let b = a + 2");
        assert!(e.contains("The nearest line is 3: `  let b = a + 1`") && !e.contains("The nearest line is 2"), "{e}");
        // An `old` of only whitespace, like empty text, has no line to name.
        let e = miss("   ");
        assert!(e.contains("does not occur") && !e.contains("The nearest line"), "{e}");
        let e = miss("zzz");
        assert!(e.contains("does not occur") && !e.contains("The nearest line"), "{e}");
        let e = replace("", Some("x"), "y", false).unwrap_err().0;
        assert!(e.contains("does not occur") && !e.contains("The nearest line"), "{e}");
    }

    /// A patch draft missing its `-`, `+` or ` ` prefixes names the line and
    /// says to include them; it never applies the loose match.
    #[test]
    fn a_patch_miss_says_to_include_the_prefix() {
        let patch = "*** Begin Patch\n*** Add: app.x\n+pub fn x() -> Int {\n+  1\n+}\n*** End Patch\n";
        let e = replace(patch, Some("pub fn x() -> Int {\n  1\n}"), "y", false).unwrap_err().0;
        assert!(
            e.contains("does not occur")
                && e.contains("The nearest line is 3, but a patch line starts with `-`, `+` or ` `: include its prefix"),
            "{e}"
        );
        let e = replace(patch, Some("\npub fn x() -> Int {\n  1\n}"), "y", false).unwrap_err().0;
        assert!(e.contains("The nearest line is 3, but a patch line starts with `-`, `+` or ` `: include its prefix"), "{e}");
    }

    /// A replacement is kept even when its retry is refused: the footer says
    /// the draft holds it and the next `old` must match that text.
    #[test]
    fn a_failed_retry_keeps_the_replacement() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let first = submit(&s, &["submit", "app.main"], &mut || Ok("pub fn main() { missing }".into()), None).unwrap_err().0;
        let id = first.split("Draft ").nth(1).and_then(|rest| rest.split(' ').next()).unwrap().to_string();
        let e = submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok("also_missing".into()), Some("missing")).unwrap_err().0;
        assert!(
            e.contains("The replacement was applied: the draft now holds it")
                && e.contains("the next `old` must match the draft as it is now")
                && e.contains(&format!("`view --draft {id}`")),
            "{e}"
        );
        assert!(view(&s, &id).unwrap().contains("also_missing"));
    }

    /// A module patch saves a changed item as rustfmt writes it, even when the
    /// hunk's `+` line is indented and spaced wrong (D-158 point 5).
    #[test]
    fn a_module_patch_formats_the_item_it_changes() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
        let (root, store) = rust_project(lib);
        let patch = "*** Begin Patch\n*** Update: crate\n@@\n-        2\n+     3+4\n*** End Patch\n";
        let out = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("Saved crate.Store::open"), "{out}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib.replace("        2\n", "        3 + 4\n"),);
    }

    /// A module patch pushes only the item it changed; an item rustfmt would
    /// reformat but the patch didn't touch keeps its bytes (D-158 point 5).
    #[test]
    fn a_module_patch_pushes_only_the_changed_item() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32{2}\n";
        let (root, store) = rust_project(lib);
        let patch = "*** Begin Patch\n*** Update: crate\n@@\n-    1\n+  3\n*** End Patch\n";
        let blocks = patch_blocks(&store, patch).unwrap();
        assert!(blocks.contains("// ── crate.one ──"), "{blocks}");
        assert!(!blocks.contains("crate.two"), "{blocks}");
        submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("src/lib.rs")).unwrap(),
            "pub fn one() -> i32 {\n    3\n}\n\npub fn two() -> i32{2}\n",
        );
    }

    /// A module patch whose text doesn't parse is refused, naming and quoting
    /// its line in the module, and nothing is saved.
    #[test]
    fn a_module_patch_syntax_error_names_a_line() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = concat!(
            "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n\n",
            "    pub fn other(&self) -> i32 {\n        3\n    }\n}\n",
        );
        let (root, store) = rust_project(lib);
        let patch = "*** Begin Patch\n*** Update: crate\n@@\n-        3\n+        let = ;\n*** End Patch\n";
        let err = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
        assert!(err.starts_with("E0108 line 9: "), "{err}");
        assert!(err.contains("`let = ;`"), "{err}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    }

    /// A crate root's `mod x;` lines don't stop a module patch: only the item
    /// the patch changed is formatted, never the whole file.
    #[test]
    fn a_module_patch_changes_a_crate_root_with_mod_lines() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "mod helper;\n\npub fn one() -> i32 {\n    1\n}\n";
        let root =
            Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", lib), ("src/helper.rs", "pub fn two() -> i32 {\n    2\n}\n")]);
        crate::import(&root).unwrap();
        let store = crate::Store::at(&root).unwrap();
        let patch = "*** Begin Patch\n*** Update: crate\n@@\n-    1\n+  3\n*** End Patch\n";
        let out = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("Saved crate.one"), "{out}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib.replace("    1\n", "    3\n"),);
    }

    /// A rustfmt error in a module patch names the line in the module, not in
    /// the changed item, and nothing is saved.
    #[test]
    fn a_module_patch_names_a_rustfmt_error_in_the_module() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
        let root = Scratch::new()
            .with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2015\"\n"), ("src/lib.rs", lib)]);
        crate::import(&root).unwrap();
        let store = crate::Store::at(&root).unwrap();
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Update: crate\n",
            "@@ pub fn two\n",
            " pub fn two() -> i32 {\n",
            "-    2\n",
            "+    3\n",
            " }\n",
            "+\n",
            "+pub async fn three() -> i32 {\n",
            "+    3\n",
            "+}\n",
            "*** End Patch\n",
        );
        let err = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
        assert!(err.contains("E0100 syntax error in `crate.three` at line 9"), "{err}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    }

    /// A rustfmt error inside an `impl` names the method's line in the module,
    /// and nothing is saved.
    #[test]
    fn a_nested_module_patch_names_a_rustfmt_error_in_the_module() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub struct Store;\n\nimpl Store {\n    pub fn open(&self) -> i32 {\n        2\n    }\n}\n";
        let root = Scratch::new()
            .with(&[("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2015\"\n"), ("src/lib.rs", lib)]);
        crate::import(&root).unwrap();
        let store = crate::Store::at(&root).unwrap();
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Update: crate\n",
            "@@\n",
            "-    pub fn open(&self) -> i32 {\n",
            "+    pub async fn open(&self) -> i32 {\n",
            "*** End Patch\n",
        );
        let err = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap_err().0;
        assert!(err.contains("E0100 syntax error in `crate.Store::open` at line 4"), "{err}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    }

    #[test]
    fn drop_refuses_another_owners_draft() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        for (id, task) in [("mine", owner()), ("theirs", "another-task".to_string())] {
            s.db.execute(
                "INSERT INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,?)",
                params![id, task, json!(["submit", "app.main"]).to_string(), "text", ""],
            )
            .unwrap();
        }
        let e = drop_one(&s, "theirs").unwrap_err().0;
        assert!(e.contains("E0106") && e.contains("not found"), "{e}");
        let left: i64 = s.db.query_row("SELECT COUNT(*) FROM drafts WHERE id='theirs'", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 1, "another owner's draft stays");
        let out = drop_one(&s, "mine").unwrap();
        assert!(out.contains("Dropped draft mine"), "{out}");
        assert!(list(&s).unwrap().is_empty());
    }

    #[test]
    fn draft_and_imports_are_two_calls() {
        let s = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        for args in [["submit", "--draft", "d1", "--imports", "app"], ["submit", "--imports", "app", "--draft", "d1"]] {
            let e = submit(&s, &args, &mut || Ok(String::new()), None).unwrap_err().0;
            assert!(
                e.starts_with("E0109") && e.contains("two calls") && e.contains("`submit --imports app` first, then `submit --draft d1`"),
                "{args:?}: {e}"
            );
        }
    }

    #[test]
    fn drop_command_drops_one_owned_draft() {
        let p = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        for (id, task) in [("mine", owner()), ("theirs", "another-task".to_string())] {
            p.db.execute(
                "INSERT INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,?)",
                params![id, task, json!(["submit", "app.main"]).to_string(), "text", ""],
            )
            .unwrap();
        }
        let run = |id: &str| {
            let args: Vec<String> = ["drafts", "drop", id].iter().map(|x| x.to_string()).collect();
            crate::with_root(&p.root, || crate::run_options(&args, &mut || Ok(String::new()), None, 10_000, &[]))
        };
        let out = run("mine").unwrap();
        assert!(out.contains("Dropped draft mine"), "{out}");
        let e = run("theirs").unwrap_err().0;
        assert!(e.contains("E0106") && e.contains("not found"), "{e}");
        let left: i64 = p.db.query_row("SELECT COUNT(*) FROM drafts WHERE id='theirs'", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 1, "another owner's draft stays");
    }

    /// A loose code hunk keeps the target's context and `-` lines, shifts its
    /// `+` lines by the indentation difference, and refuses an ambiguous fit
    /// (D-204).
    #[test]
    fn a_loose_hunk_keeps_the_targets_lines_and_shifts_additions() {
        let s = board_project(&[("src/app.gleam", "pub fn main() {\n  let x = 1\n  x\n}\n")]);
        let patch = "*** Begin Patch\n*** Update: app.main\n-let x = 1\n x\n+let x = 2\n*** End Patch\n";
        let blocks = patch_blocks(&s, patch).unwrap();
        assert!(blocks.contains("\n  x\n  let x = 2\n"), "{blocks}");

        let hunk = |lines: &[&str]| (None, lines.iter().map(|l| l.to_string()).collect(), 1);
        let mut lines: Vec<String> = vec!["\t\tlet x = 1".into()];
        apply_hunks("x", "view x", &mut lines, vec![hunk(&["-    let x = 1", "+    let x = 2"])], false).unwrap();
        assert_eq!(lines, ["\t\tlet x = 2"]);

        let mut lines: Vec<String> = vec!["".into(), "\t\tlet x = 1".into()];
        apply_hunks("x", "view x", &mut lines, vec![hunk(&[" ", "-    let x = 1", "+    let x = 2"])], false).unwrap();
        assert_eq!(lines, ["", "\t\tlet x = 2"]);

        let mut lines: Vec<String> = vec!["  let x = 1".into()];
        apply_hunks("x", "view x", &mut lines, vec![hunk(&["-    let x = 1", "+  let y = 2"])], false).unwrap();
        assert_eq!(lines, ["let y = 2"]);

        let twice = board_project(&[("src/app.gleam", "pub fn main() {\n  let a = 1\n  let a = 1\n}\n")]);
        let e = patch_blocks(&twice, "*** Begin Patch\n*** Update: app.main\n-let a = 1\n+let a = 2\n*** End Patch\n").unwrap_err().0;
        assert!(e.contains("E0108") && e.contains("`let a = 1`") && e.contains("leading space"), "{e}");
    }

    /// A save whose hunk matched through indentation says so (D-204).
    #[test]
    fn a_loose_match_says_so_in_the_save_reply() {
        let s = board_project(&[("src/app.gleam", "pub fn main() {\n  let x = 1\n  x\n}\n")]);
        let patch = "*** Begin Patch\n*** Update: app.main\n-let x = 1\n+let x = 2\n*** End Patch\n";
        let out = submit(&s, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("matched through indentation"), "{out}");
        assert!(s.view(&["app.main"], false, BUDGET).unwrap().contains("let x = 2"));
    }

    /// A `write` patch keeps D-142's strict leading-space rule (D-204).
    #[test]
    fn a_file_patch_stays_strict_about_indentation() {
        let hunk = (None, vec!["-one".to_string(), "+1".to_string()], 1);
        let e = file_text("notes", "  one\n", Section::Update, vec![hunk]).unwrap_err().0;
        assert!(e.contains("E0108") && e.contains("these lines weren't found: `one`") && e.contains("with 2 leading spaces"), "{e}");
    }

    /// A patch naming an existing block refuses with `format <module>`, not a new
    /// block's route, and changes nothing.
    #[test]
    fn a_patch_naming_an_existing_block_says_to_format() {
        let lib = "pub fn one() -> i32 {\n    1\n}\n\nmod tests {\n    #[test]\n    fn two() {}\n}\n";
        let (root, s) = rust_project(lib);
        let e = patch_blocks(&s, "*** Begin Patch\n*** Update: crate.tests\n@@\n-    fn two() {}\n+    fn three() {}\n*** End Patch\n")
            .unwrap_err()
            .0;
        assert!(e.contains("E0108"), "{e}");
        assert!(e.contains("format crate"), "{e}");
        assert!(!e.contains("submit --new"), "{e}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    }

    /// A draft saved in a task run is visible to the commands that view and
    /// drop it.
    #[test]
    fn a_context_owns_the_drafts_it_saves() {
        let p = board_project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
        let mut t = crate::board::api(&p.root, "POST", "/tasks", json!({"title": "Drafts"})).unwrap();
        let tid = t["id"].as_str().unwrap().to_string();
        t["runs"] = json!([{"id": "run-1", "kind": "work", "state": "running"}]);
        crate::tasks::put(&p, &t).unwrap();
        let id = crate::board::with_context(&tid, "run-1", || {
            assert!(submit(&p, &["submit", "app.main"], &mut || Ok("pub fn main(".into()), None).is_err());
            entries(&p, &tid).unwrap()[0].0.clone()
        });
        let run = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|x| x.to_string()).collect();
            crate::with_root(&p.root, || crate::run_options(&args, &mut || Ok(String::new()), None, 10_000, &[]))
        };
        let viewed = crate::board::with_context(&tid, "run-1", || run(&["view", "--draft", id.as_str()])).unwrap();
        assert_eq!(viewed, "pub fn main(");
        let dropped = crate::board::with_context(&tid, "run-1", || run(&["drafts", "drop", id.as_str()])).unwrap();
        assert!(dropped.contains("Dropped draft"), "{dropped}");
        assert!(entries(&p, &tid).unwrap().is_empty());
    }

    /// A Rust symbol patch that matches through indentation says so, and the
    /// saved item is rustfmt's text (D-204).
    #[test]
    fn a_rust_symbol_patch_matches_through_indentation() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub fn one() -> i32 {\n    1\n}\n";
        let (root, store) = rust_project(lib);
        let patch = "*** Begin Patch\n*** Update: crate.one\n@@\n-  1\n+   2\n*** End Patch\n";
        let out = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("matched through indentation"), "{out}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), "pub fn one() -> i32 {\n    2\n}\n");
    }

    /// A Rust module patch that matches through indentation says so, and the
    /// saved item is rustfmt's text (D-204).
    #[test]
    fn a_rust_module_patch_matches_through_indentation() {
        if !crate::language::cargo::installed() {
            return;
        }
        let lib = "pub fn one() -> i32 {\n    1\n}\n";
        let (root, store) = rust_project(lib);
        let patch = "*** Begin Patch\n*** Update: crate\n@@\n-  1\n+   2\n*** End Patch\n";
        let out = submit(&store, &["submit"], &mut || Ok(patch.into()), None).unwrap();
        assert!(out.contains("matched through indentation"), "{out}");
        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), "pub fn one() -> i32 {\n    2\n}\n");
    }
}

/// The refusal for an `*** Update:` that names no symbol (E0108): a module or
/// a source path gets the header that works, and another name the nearest names.
fn bad_target(s: &Store, ix: &Index, target: &str, at_line: usize) -> Failure {
    let name = target.strip_prefix("module:").unwrap_or(target);
    let module = s
        .resolve_module(ix, name)
        .map(|m| m.path.clone())
        .or_else(|| ["", "src/", "test/"].iter().find_map(|prefix| crate::lockdown::module_of_file(s, &format!("{prefix}{name}"))));
    if let Some(module) = module {
        return Failure(
            format!(
                "E0108 `*** Update: {target}` (line {at_line}) is no symbol; \
                 a module changes with `*** Update: {module}` (see `help submit`)."
            ),
            1,
        );
    }
    Failure(format!("E0108 `*** Update: {target}` (line {at_line}) is no symbol. {RESEND}{}", store::similar_hint(ix, target)), 1)
}

/// The refusal for an update that leaves none of `qname`.
fn nothing_left(qname: &str, at_line: usize) -> String {
    format!("the patch leaves nothing of {qname} (section at line {at_line}); use `delete <name>`.")
}

/// The refusal for an update that would rename its symbol.
fn renamed(qname: &str, at_line: usize) -> String {
    format!(
        concat!(
            "the patch renames {qname} (section at line {at_line}); ",
            "`submit {qname}` with the new text renames it and keeps its id; ",
            "change its callers in the same submit.",
        ),
        qname = qname,
        at_line = at_line
    )
}

fn add_blocks(c: &mut SectionContext<'_>, rust: bool, hunks: &[Hunk]) -> Result<(), Failure> {
    let text: String = hunks[0].1.iter().map(|l| format!("{}\n", &l[1..])).collect();
    if rust {
        return add_rust_blocks(c, &text);
    }
    let split = c.s.lang().split(&text);
    if split.items.is_empty() {
        return Err(defines_nothing(c));
    }
    let module = match c.target.rsplit_once('.') {
        Some((m, n)) if split.items.iter().any(|item| item.name == n) => m.to_string(),
        _ => c.target.to_string(),
    };
    let mut header = split.header;
    for item in split.items {
        push_block(c.blocks, &mut header, &module, &item.name, &item.text);
    }
    Ok(())
}

fn rust_update_blocks(c: &mut SectionContext<'_>, hunks: Vec<Hunk>) -> Result<bool, Failure> {
    if !c.target.contains('.')
        && let Some(m) = c.s.resolve_module(c.ix, c.target)
    {
        return rust_module_update_blocks(c, m, hunks);
    }
    rust_symbol_update_blocks(c, hunks)
}

fn unit_update_blocks(c: &mut SectionContext<'_>, hunks: Vec<Hunk>) -> Result<bool, Failure> {
    let blocks = &mut *c.blocks;
    // The target's text, a unit per symbol (and a module's header first).
    let (module, mut units): (String, Vec<(Option<usize>, String)>) = match c.s.resolve_module(c.ix, c.target) {
        Some(m) => (
            m.path.clone(),
            std::iter::once((None, m.header.clone()))
                .chain(c.ix.syms.iter().enumerate().filter(|(_, x)| x.module == m.path).map(|(i, x)| (Some(i), x.text.clone())))
                .collect(),
        ),
        None => {
            let i = c.s.resolve(c.ix, c.target).map_err(|e| (c.not_code)(e))?;
            (c.ix.syms[i].module.clone(), vec![(Some(i), c.ix.syms[i].text.clone())])
        }
    };
    // One list of lines, with a separator no hunk can match between units.
    const SEP: &str = "\u{0}";
    let mut lines: Vec<String> =
        units.iter().map(|u| u.1.clone()).collect::<Vec<_>>().join(&format!("\n{SEP}\n")).lines().map(String::from).collect();
    let indented = apply_hunks(c.target, &format!("view {}", c.target), &mut lines, hunks, false)?;
    let joined = lines.join("\n");
    let texts: Vec<&str> = joined.split(&format!("\n{SEP}\n")).collect();
    if texts.len() != units.len() {
        return Err((c.bad)(format!(
            "in {} (section at line {}), a hunk spans two symbols; give each symbol its own hunk.",
            c.target, c.at_line
        )));
    }
    for (u, t) in units.iter_mut().zip(&texts) {
        u.1 = t.trim_end().to_string();
    }
    // A changed header's imports go with the module's first changed symbol.
    let mut header = match &units[0] {
        (None, h) if h.trim() != c.s.resolve_module(c.ix, &module).map_or("", |m| m.header.trim()) => h.clone(),
        _ => String::new(),
    };
    for (i, text) in units.iter().filter_map(|(i, t)| Some((i.as_ref()?, t))) {
        if *text == c.ix.syms[*i].text {
            continue;
        }
        let split = c.s.lang().split(text);
        if split.items.is_empty() {
            return Err((c.bad)(nothing_left(&c.ix.syms[*i].qname(), c.at_line)));
        }
        let Some(own) = split.items.iter().find(|item| item.name == c.ix.syms[*i].name) else {
            return Err((c.bad)(renamed(&c.ix.syms[*i].qname(), c.at_line)));
        };
        let mut head = std::mem::take(&mut header);
        push_block(blocks, &mut head, &module, &c.ix.syms[*i].name, &own.text);
        for item in split.items.iter().filter(|item| item.name != c.ix.syms[*i].name) {
            push_block(blocks, &mut head, &module, &item.name, &item.text);
        }
    }
    if !header.is_empty() {
        return Err((c.bad)(format!(
            concat!(
                "the patch changes only {}'s imports (section at line {}); ",
                "to replace them, use `submit --imports {}` with the module's imports and comment.",
            ),
            module, c.at_line, module
        )));
    }
    Ok(indented)
}

/// What every section of a patch is turned into blocks with.
struct SectionContext<'a> {
    s: &'a Store,
    ix: &'a Index,
    target: &'a str,
    at_line: usize,
    blocks: &'a mut String,
    pending: &'a mut Vec<PendingImpl>,
    bad: &'a dyn Fn(String) -> Failure,
    not_code: &'a dyn Fn(Failure) -> Failure,
}

/// A `*** Update:` that names a module applies its hunks to the module's
/// whole file (D-168): every changed or added item goes as one block; the
/// header has its own commands. Only the items the patch changed are
/// formatted, so the rest keep their bytes, as `submit <module>` does.
fn rust_module_update_blocks(c: &mut SectionContext<'_>, m: &ModuleRow, hunks: Vec<Hunk>) -> Result<bool, Failure> {
    let module = m.path.clone();
    let before = store::module_text(c.ix, m);
    let mut lines: Vec<String> = before.lines().map(String::from).collect();
    let indented = apply_hunks(c.target, &format!("view {}", c.target), &mut lines, hunks, false)?;
    let after = lines.join("\n");
    let file = language::rust::split(&after, &module).map_err(|e| (c.bad)(e.0))?;
    if !file.refusals.is_empty() {
        let lines: Vec<String> = file.refusals.iter().map(|r| format!("line {} (`{}`): {}", r.line, r.quote, r.message)).collect();
        return Err((c.bad)(lines.join("\n")));
    }
    if file.header.trim_end() != c.s.header(&module)?.trim_end() {
        return Err((c.bad)(format!(
            concat!(
                "the patch changes {}'s imports (section at line {}); ",
                "imports are added automatically (`submit --imports {}` replaces them).",
            ),
            module, c.at_line, module
        )));
    }
    let mut items = crate::rust_submit::rust_module_items(c.ix, &module, &file.items)?;
    c.s.rust_format_module(c.ix, &module, None, &mut items, false, file.header.matches('\n').count() + 1)?;
    push_module_blocks(c.blocks, c.ix, &module, &items, c.at_line, &c.bad)?;
    Ok(indented)
}

fn rust_symbol_update_blocks(c: &mut SectionContext<'_>, hunks: Vec<Hunk>) -> Result<bool, Failure> {
    let blocks = &mut *c.blocks;
    let Some(i) = crate::rust_submit::rust_symbol(c.ix, c.target)? else {
        if let Some(container) = crate::rust_submit::rust_container(c.ix, c.target)? {
            let block = &c.ix.syms[container];
            return Err((c.bad)(format!("{}.", store::block_note(&block.kind_text, Some(block.module.as_str())))));
        }
        return Err((c.not_code)(bad_target(c.s, c.ix, c.target, c.at_line)));
    };
    let mut lines: Vec<String> = c.ix.syms[i].text.lines().map(String::from).collect();
    let indented = apply_hunks(c.target, &format!("view {}", c.target), &mut lines, hunks, false)?;
    let text = lines.join("\n");
    if text == c.ix.syms[i].text {
        return Ok(indented);
    }
    let parent = c.ix.syms[i]
        .parent
        .as_ref()
        .and_then(|p| c.ix.syms.iter().find(|x| &x.id == p))
        .map(|p| (p.name.as_str(), p.text.as_str(), p.kind_text.as_str()));
    let qname = c.ix.syms[i].qname();
    let items = store::submit_items(&c.s.root, &c.ix.syms[i].module, &qname, parent, &text, false)?;
    if items.is_empty() {
        return Err((c.bad)(nothing_left(&c.ix.syms[i].qname(), c.at_line)));
    }
    let stored = c.ix.syms[i].name.clone();
    let Some(own) = items.iter().find(|item| item.name == stored) else {
        return Err((c.bad)(renamed(&c.ix.syms[i].qname(), c.at_line)));
    };
    let module = c.ix.syms[i].module.clone();
    push_block(blocks, &mut String::new(), &module, &stored, &own.text);
    for item in items.iter().filter(|item| item.name != stored) {
        let text = c.s.rust_block_text(c.ix, &module, &item.name, &item.text);
        push_block(blocks, &mut String::new(), &module, &item.name, &text);
    }
    Ok(indented)
}

/// An inherent `impl` an Add section is still filling in (D-194), keyed by its
/// module and full self type: its opening, the items inside it, and its header.
struct PendingImpl {
    module: String,
    ty: String,
    opening: String,
    items: Vec<String>,
    header: String,
    added: bool,
}

/// The blocks an Add section gives (D-194): a type with its impl is one block
/// per item, a method for a type added earlier joins that type's pending impl,
/// and a whole new `impl` or `trait` block names its module.
fn add_rust_blocks(c: &mut SectionContext<'_>, text: &str) -> Result<(), Failure> {
    let container = c.s.rust_container_in(c.ix, c.target)?;
    let file = language::rust::split(text, c.target).map_err(|e| {
        store::rust_syntax_error(
            e,
            c.target,
            container.map(|i| (c.ix.syms[i].name.as_str(), c.ix.syms[i].text.as_str(), c.ix.syms[i].kind_text.as_str())),
            container.map_or("", |i| c.ix.syms[i].module.as_str()),
            0,
            0,
            text.lines().count().max(1),
        )
    })?;
    if file.items.is_empty() {
        return Err(defines_nothing(c));
    }
    let root = c.s.root.clone();
    let target = c.target;
    let mut header = file.header;
    if container.is_none() && add_pending_method(c, target, text, &mut header)? {
        return Ok(());
    }
    let module = match target.rsplit_once('.') {
        Some((m, n)) if file.items.iter().any(|p| bare_type(&p.name) == n) => m.to_string(),
        _ => target.to_string(),
    };
    for p in &file.items {
        if store::is_block(p) {
            if container.is_some() {
                return Err((c.bad)(format!("{}.", store::block_note(p.kind, None))));
            }
            if p.kind == "impl" && !p.name.starts_with('<') {
                add_pending_impl(c, &module, p, &mut header)?;
            } else {
                let block = formatted_block(&root, &p.whole())?;
                push_named(c.blocks, &mut header, &module, &block);
            }
        } else {
            let item = c.s.rust_block_text(c.ix, &module, &p.name, &p.text);
            push_block(c.blocks, &mut header, &module, &p.name, &item);
            if p.kind == "struct" {
                add_pending_struct(c, &module, &p.whole())?;
            }
        }
    }
    Ok(())
}

/// Holds an inherent impl from an Add section until the patch ends, so a later
/// method for its type joins this block instead of opening another (D-194).
fn add_pending_impl(c: &mut SectionContext<'_>, module: &str, block: &language::rust::Piece, header: &mut String) -> Result<(), Failure> {
    let root = c.s.root.clone();
    let text = formatted_block(&root, &block.whole())?;
    let file = language::rust::split(&text, module)?;
    let Some(one) = file.items.first() else {
        return Err(defines_nothing(c));
    };
    let p = pending_mut(c.pending, module, &one.name);
    if !p.added {
        p.opening = one.text.clone();
    }
    p.items.extend(one.children.iter().map(|child| child.whole()));
    p.added = true;
    if p.header.is_empty() {
        p.header = std::mem::take(header);
    }
    Ok(())
}

/// Remembers the impl opening a struct added in this patch would take, so a
/// later `*** Add: <module>.<Type>::<fn>` can open it (D-194).
fn add_pending_struct(c: &mut SectionContext<'_>, module: &str, text: &str) -> Result<(), Failure> {
    let (opening, ty) = language::rust::inherent_impl(text)?;
    let p = pending_mut(c.pending, module, &ty);
    if p.opening.is_empty() {
        p.opening = opening;
    }
    Ok(())
}

/// The pending impl for `module`'s type `ty`, made when this patch first needs it.
fn pending_mut<'a>(pending: &'a mut Vec<PendingImpl>, module: &str, ty: &str) -> &'a mut PendingImpl {
    let at = match pending.iter().position(|p| p.module == module && p.ty == ty) {
        Some(i) => i,
        None => {
            pending.push(PendingImpl {
                module: module.to_string(),
                ty: ty.to_string(),
                opening: String::new(),
                items: vec![],
                header: String::new(),
                added: false,
            });
            pending.len() - 1
        }
    };
    &mut pending[at]
}

/// Pushes each held impl once, at the end of the patch, so the blocks define
/// one `impl` per type (D-194).
fn render_pending_impls(root: &std::path::Path, blocks: &mut String, pending: &mut [PendingImpl]) -> Result<(), Failure> {
    for p in pending {
        if !p.added || p.opening.is_empty() {
            continue;
        }
        let body = p.items.join("\n\n");
        let mut whole = p.opening.clone();
        if !whole.ends_with('\n') {
            whole.push('\n');
        }
        whole.push_str(&body);
        if !body.is_empty() {
            whole.push('\n');
        }
        whole.push('}');
        let text = formatted_block(root, &whole)?;
        push_named(blocks, &mut p.header, &p.module, &text);
    }
    Ok(())
}

/// The module and bare type an `*** Add: <module>.<Type>::<fn>` names.
fn pending_target(target: &str) -> Option<(&str, &str)> {
    let (container, _) = target.rsplit_once("::")?;
    let (module, ty) = container.rsplit_once('.')?;
    (!module.is_empty() && !ty.is_empty()).then(|| (module, bare_type(ty)))
}

/// A type's name without its generic arguments: `X<T>` is `X`.
fn bare_type(name: &str) -> &str {
    name.split('<').next().unwrap_or(name)
}

/// One whole item as rustfmt writes it.
fn formatted_block(root: &std::path::Path, text: &str) -> Result<String, Failure> {
    crate::language::cargo::format_item(root, text, None).map(|formatted| formatted.trim_end().to_string())
}

/// An item from `submit_items` and everything nested in it, as one text.
fn submitted_whole(item: &store::Submitted) -> String {
    let mut out = item.text.clone();
    for child in &item.children {
        out.push_str(&submitted_whole(child));
    }
    out.push_str(&item.tail);
    out
}

/// The refusal for an Add whose text holds no item.
fn defines_nothing(c: &SectionContext<'_>) -> Failure {
    (c.not_code)((c.bad)(format!("`*** Add: {}` (line {}) defines no function, type or constant. {RESEND}", c.target, c.at_line)))
}

/// One `// ── <label> ──` block, with a module's imports once before its first item.
fn push_named(blocks: &mut String, header: &mut String, label: &str, text: &str) {
    let head = if header.is_empty() { String::new() } else { format!("{}\n\n", std::mem::take(header)) };
    blocks.push_str(&format!("{BLOCK_HEADER}{label} ──\n{head}{text}\n"));
}

/// The one pending impl an `*** Add: <module>.<Type>::<fn>` names, or E1002
/// when the patch adds more than one impl for that type.
fn pending_impl_at(pending: &[PendingImpl], module: &str, ty: &str) -> Result<Option<usize>, Failure> {
    let found: Vec<usize> =
        pending.iter().enumerate().filter(|(_, p)| p.module == module && bare_type(&p.ty) == ty).map(|(i, _)| i).collect();
    match found.as_slice() {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        many => {
            let names: Vec<&str> = many.iter().map(|&i| pending[i].ty.as_str()).collect();
            Err(Failure(
                format!(
                    "E1002 {module}.{ty} matches more than one impl in this patch ({}); submit its methods separately.",
                    names.join(", ")
                ),
                1,
            ))
        }
    }
}

/// Adds an `*** Add: <module>.<Type>::<fn>` to the impl a type or impl added
/// earlier in this patch opened (D-194). False when this Add names no pending
/// impl, so the caller handles it as an ordinary item.
fn add_pending_method(c: &mut SectionContext<'_>, target: &str, text: &str, header: &mut String) -> Result<bool, Failure> {
    let Some((module, ty)) = pending_target(target) else {
        return Ok(false);
    };
    let Some(i) = pending_impl_at(c.pending, module, ty)? else {
        return Ok(false);
    };
    let opening = c.pending[i].opening.trim_end().to_string();
    if opening.is_empty() {
        return Ok(false);
    }
    let root = c.s.root.clone();
    let items = store::submit_items(&root, module, target, Some((ty, opening.as_str(), "impl")), text, false)?;
    let p = &mut c.pending[i];
    p.items.extend(items.iter().map(submitted_whole));
    p.added = true;
    if p.header.is_empty() {
        p.header = std::mem::take(header);
    }
    Ok(true)
}

/// Whether a kept `submit` draft holds more than one block: an applied patch's
/// blocks, a patch's sections, or a text that defines several items.
fn several_blocks(s: &Store, text: &str) -> bool {
    if text.lines().filter(|l| l.starts_with(BLOCK_HEADER)).count() > 1 {
        return true;
    }
    if is_patch(text) {
        return sections(text, false).is_ok_and(|s| s.len() > 1);
    }
    let items = if language::is_rust(&s.root) {
        language::rust::split(text, "crate").map_or(1, |file| file.items.len())
    } else {
        s.lang().split(text).items.len()
    };
    items > 1
}

/// The start of every block `patch_blocks` writes; block parsers and
/// `several_blocks` count these.
pub(crate) const BLOCK_HEADER: &str = "// ── ";

pub const WHOLE: &str = "Send the whole corrected text again in `text`.";

/// The suffix shared by every refusal that says `old` and `text` are patch
/// lines, prefixes included, so the wording can't drift apart.
const PATCH_LINES: &str = "lines of the patch, prefixes included";

/// The line of `text` closest to `old`'s first non-blank line: the first
/// equal once space is trimmed, else the one sharing the longest start with
/// it. Ties go to the first line. The flag says the trimmed lines are equal.
/// No line when none is equal or shares a start.
fn nearest_line(text: &str, old: &str) -> Option<(usize, bool)> {
    let want = old.lines().find(|line| !line.trim().is_empty())?;
    let mut best: Option<(usize, bool, usize)> = None;
    for (k, line) in text.lines().enumerate() {
        let same = line.trim() == want.trim();
        let shared = line.trim_start().chars().zip(want.trim_start().chars()).take_while(|(a, b)| a == b).count();
        if best.is_none_or(|(_, same0, shared0)| (same, shared) > (same0, shared0)) {
            best = Some((k, same, shared));
        }
    }
    best.filter(|&(_, same, shared)| same || shared > 0).map(|(k, same, _)| (k, same))
}

/// E0105's detail for a miss: the nearest line and its number, the leading
/// spaces when only they differ (D-142), or the patch line whose prefix `old`
/// left off. When `old`'s first line matches, the first later line that differs
/// is the one to fix.
fn miss_hint(text: &str, old: &str) -> String {
    let want = match old.lines().find(|line| !line.trim().is_empty()) {
        Some(line) => line,
        None => return String::new(),
    };
    if is_patch(text) {
        let bare: String = text
            .split_inclusive('\n')
            .map(|line| match line.chars().next() {
                Some('-' | '+' | ' ') => &line[1..],
                _ => line,
            })
            .collect();
        let ats: Vec<usize> = bare.match_indices(old).map(|(at, _)| at).collect();
        if ats.len() == 1 {
            let at = ats[0] + old.find(want).unwrap_or(0);
            return format!(
                " The nearest line is {}, but a patch line starts with `-`, `+` or ` `: include its prefix.",
                bare[..at].matches('\n').count() + 1
            );
        }
    }
    let Some((k, same)) = nearest_line(text, old) else { return String::new() };
    let line = text.lines().nth(k).unwrap_or("");
    if same {
        let have = line.len() - line.trim_start().len();
        let had = want.len() - want.trim_start().len();
        if have != had {
            return format!(
                " The nearest line is {}, with {}: `{}`; your `old` line has {}.",
                k + 1,
                count(have, "leading space"),
                clip(line.trim()),
                count(had, "leading space")
            );
        }
        if let Some(n) = following_miss(text, old, k) {
            return format!(" The nearest line is {}: `{}`.", n, clip(text.lines().nth(n - 1).unwrap_or("")));
        }
    }
    format!(" The nearest line is {}: `{}`.", k + 1, clip(line))
}

/// The first line after `old`'s first non-blank line that differs from `text`
/// at `k`, as its number in `text`; `None` when every following line matches or
/// the text ends first.
fn following_miss(text: &str, old: &str, k: usize) -> Option<usize> {
    let first = old.lines().position(|line| !line.trim().is_empty())?;
    let mut text_lines = text.lines().skip(k + 1);
    for (i, line) in old.lines().skip(first + 1).enumerate() {
        match text_lines.next() {
            Some(have) if have != line => return Some(k + 2 + i),
            Some(_) => {}
            None => return None,
        }
    }
    None
}

/// The drafts `task` owns, oldest first: each id, the command that made it,
/// and whether its base is still current.
pub fn entries(s: &Store, task: &str) -> Result<Vec<(String, Vec<String>, bool)>, Failure> {
    let mut q = s.db.prepare("SELECT id, args, text, base FROM drafts WHERE owner=? ORDER BY rowid")?;
    let rows: Vec<(String, String, String, String)> =
        q.query_map([task], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let ix = s.index()?;
    let mut out = vec![];
    for (id, args, text, then) in rows {
        let parsed = serde_json::from_str::<Vec<String>>(&args).unwrap_or_default();
        // a kept `write` patch has no base: its hunks are matched again when it is retried (D-120)
        let current = parsed.first().is_some_and(|c| c == "write") || base(s, &ix, &parsed, &text)? == then;
        out.push((id, parsed, current));
    }
    Ok(out)
}

/// Removes one draft this task owns; another owner's draft is refused, not ignored.
pub fn drop_owned(s: &Store, task: &str, id: &str) -> Result<(), Failure> {
    let removed = s.db.execute("DELETE FROM drafts WHERE id=? AND owner=?", params![id, task])?;
    if removed == 0 {
        return Err(Failure("E0106 draft not found in this task/session".into(), 1));
    }
    Ok(())
}

pub fn drop_one(s: &Store, id: &str) -> Out {
    drop_owned(s, &owner(), id)?;
    Ok(format!("Dropped draft {id}.\n"))
}

/// A listed draft's state: current, or stale because what it replaces changed.
pub fn status(current: bool) -> &'static str {
    if current { "current" } else { "stale: what it replaces changed since; check it isn't superseded" }
}

/// The one start position where every hunk line matches with space at both
/// ends ignored, if there is exactly one (D-204).
fn loose_match(old: &[&str], lines: &[String]) -> Option<(usize, bool)> {
    let mut starts = (0..=lines.len().checked_sub(old.len())?).filter(|&k| old.iter().zip(&lines[k..]).all(|(o, l)| o.trim() == l.trim()));
    let k = starts.next()?;
    starts.next().is_none().then_some((k, true))
}

/// The lines that replace a hunk's `old` lines: the patch's, or for a loose
/// match the target's for context and removals, with additions shifted to the
/// target's indentation (D-204).
fn replacement(hunk: &[String], old: &[&str], lines: &[String], k: usize, loose: bool) -> Vec<String> {
    if !loose {
        return hunk.iter().filter(|l| !l.starts_with('-')).map(|l| l[1..].to_string()).collect();
    }
    let anchor = old.iter().position(|line| !line.trim().is_empty()).unwrap_or(0);
    let patch_indent = indent(&old[anchor]);
    let target_indent = indent(&lines[k + anchor]);
    let mut at = k;
    let mut out = Vec::new();
    for line in hunk {
        match line.strip_prefix('+') {
            Some(added) => out.push(shift_indent(added, patch_indent, target_indent)),
            None => {
                let target = lines[at].clone();
                at += 1;
                if !line.starts_with('-') {
                    out.push(target);
                }
            }
        }
    }
    out
}

fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// `line` with its indentation moved by the difference between the patch's
/// and the target's indentation, never past the line's start (D-204).
fn shift_indent(line: &str, patch_indent: &str, target_indent: &str) -> String {
    if line.trim().is_empty() {
        return line.to_string();
    }
    let own = indent(line);
    let body = &line[own.len()..];
    if let Some(rest) = own.strip_prefix(patch_indent) {
        return format!("{target_indent}{rest}{body}");
    }
    let width = own.chars().count() as isize + target_indent.chars().count() as isize - patch_indent.chars().count() as isize;
    let unit = if target_indent.starts_with('\t') { "\t" } else { " " };
    format!("{}{body}", unit.repeat(width.max(0) as usize))
}

/// Blocks for a plain `submit` from a patch (D-47): `*** Update: <symbol or
/// module>` sections of `@@` hunks (` ` context, `-` removed, `+` added) change
/// stored symbols, applied in order as `apply_patch` does, each from where the
/// last one ended; `*** Add: <module>.<name>` (or a new module) adds `+` lines.
/// Returns one block per symbol changed or added, and whether a code hunk
/// matched through indentation (D-204), which the save reply names.
fn patch_blocks_with_indent(s: &Store, raw: &str) -> Result<(String, bool), Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    let sections = sections(raw, false)?;
    let ix = s.index()?;
    let rust = language::is_rust(&s.root);
    let mut blocks = String::new();
    let mut pending = Vec::new();
    let mut indented = false;
    for (kind, target, at_line, hunks) in sections {
        // A `*** Update File:` header may name a symbol; one that names no code is a file, for `write`.
        let header = raw.lines().nth(at_line - 1).unwrap_or("");
        let not_code = |e: Failure| {
            if !header.contains(" File:") {
                return if e.0.starts_with("E1001") { bad_target(s, &ix, &target, at_line) } else { e };
            }
            let path = header.split_once(':').map_or("", |p| p.1).trim();
            bad(format!(
                concat!(
                    "`{}` (line {}) is no symbol or module; ",
                    "a file other than code changes with `write` with no path, this patch as `text`. {}",
                ),
                path, at_line, RESEND
            ))
        };
        let mut c = SectionContext {
            s,
            ix: &ix,
            target: &target,
            at_line,
            blocks: &mut blocks,
            pending: &mut pending,
            bad: &bad,
            not_code: &not_code,
        };
        if kind == Section::Add {
            add_blocks(&mut c, rust, &hunks)?;
        } else if rust {
            indented |= rust_update_blocks(&mut c, hunks)?;
        } else {
            indented |= unit_update_blocks(&mut c, hunks)?;
        }
    }
    if rust {
        render_pending_impls(&s.root, &mut blocks, &mut pending)?;
    }
    if blocks.is_empty() {
        return Err(bad(format!("the patch changes nothing. {RESEND}")));
    }
    Ok((blocks, indented))
}
