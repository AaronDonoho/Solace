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
    let mut names: Vec<&str> =
        text.lines().filter_map(|l| l.strip_prefix("// ── ")).map(|h| h.split([' ', '(']).next().unwrap_or("")).collect();
    let (mut parts, mut modules) = (vec![], vec![]);
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
        [_, name] => names.push(name),
        _ => {}
    }
    for name in names {
        match s.resolve(ix, name) {
            Ok(i) => {
                parts.push(format!("{}\n{}", ix.syms[i].qname(), ix.syms[i].text));
                modules.push(ix.syms[i].module.clone());
            }
            // a new symbol: its module, which may not exist yet
            Err(_) => modules.push(name.rsplit_once('.').map_or(name, |p| p.0).to_string()),
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
        let current = base(s, &ix, &serde_json::from_str::<Vec<String>>(&args).unwrap_or_default(), &text)? == then;
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

/// Text in `apply_patch` form: `*** Begin Patch`, or a first line `*** Update…`
/// or `*** Add…`.
pub fn is_patch(text: &str) -> bool {
    text.trim_start().lines().next().is_some_and(|l| ["*** Begin Patch", "*** Update", "*** Add"].iter().any(|p| l.starts_with(p)))
}

/// A section's target: `src/pilot/seed.gleam` or `pilot/seed`, or a symbol.
fn patch_target(line: &str) -> String {
    let name = line.split_once(':').map_or("", |p| p.1).trim();
    let name = ["/src/", "/test/"].iter().find_map(|d| name.rsplit_once(d).map(|p| p.1)).unwrap_or(name);
    let name = name.strip_prefix("src/").or_else(|| name.strip_prefix("test/")).unwrap_or(name);
    name.strip_suffix(".gleam").unwrap_or(name).to_string()
}

/// Blocks for a plain `submit` from a patch (D-47): `*** Update: <symbol or
/// module>` sections of `@@` hunks (` ` context, `-` removed, `+` added) change
/// stored symbols, applied in order as `apply_patch` does, each from where the
/// last one ended; `*** Add: <module>.<name>` (or a new module) adds `+` lines.
/// Returns one block per symbol changed or added.
pub fn patch_blocks(s: &Store, raw: &str) -> Result<String, Failure> {
    let bad = |m: String| Failure(format!("E0108 {m}"), 1);
    // (update?, target, hunks of (context line, lines))
    let mut sections: Vec<(bool, String, Vec<(Option<String>, Vec<String>)>)> = vec![];
    for line in raw.lines() {
        match line {
            l if l.starts_with("*** Begin Patch") || l.starts_with("*** End") => {}
            l if l.starts_with("*** Update") => sections.push((true, patch_target(l), vec![])),
            l if l.starts_with("*** Add") => sections.push((false, patch_target(l), vec![(None, vec![])])),
            l if l.starts_with("*** Delete") => return Err(bad("a patch can't delete; use `delete <name>`.".into())),
            l if l.starts_with("*** Move") => return Err(bad("a patch can't rename; submit the symbol under its new name.".into())),
            l => {
                let Some((update, target, hunks)) = sections.last_mut() else {
                    return Err(bad("a patch starts with `*** Begin Patch`, then `*** Update: <name>` or `*** Add: <name>`.".into()));
                };
                if let Some(at) = l.strip_prefix("@@").filter(|_| *update) {
                    let at = at.trim().trim_end_matches("@@").trim();
                    // a unified diff's `@@ -3,4 +3,5 @@` gives no context line
                    let numbers = at.starts_with('-') && at[1..].starts_with(|c: char| c.is_ascii_digit());
                    hunks.push(((!at.is_empty() && !numbers).then(|| at.to_string()), vec![]));
                } else if !*update && !l.starts_with('+') {
                    return Err(bad(format!("in `*** Add: {target}` every line starts with `+`: {l}")));
                } else if *update && !l.is_empty() && !l.starts_with([' ', '-', '+']) {
                    return Err(bad(format!("in `*** Update: {target}` each line starts with ` `, `-` or `+` (or `@@`): {l}")));
                } else {
                    if hunks.is_empty() {
                        hunks.push((None, vec![]));
                    }
                    hunks.last_mut().unwrap().1.push(if l.is_empty() { " ".into() } else { l.to_string() });
                }
            }
        }
    }
    if sections.is_empty() {
        return Err(bad("the patch has no `*** Update: <name>` or `*** Add: <name>` section.".into()));
    }
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
    for (update, target, hunks) in sections {
        if !update {
            let text: String = hunks[0].1.iter().map(|l| format!("{}\n", &l[1..])).collect();
            let parsed = gleam::split(&text);
            let module = match target.rsplit_once('.') {
                Some((m, n)) if parsed.items.iter().any(|i| i.name == n) => m.to_string(),
                _ => target.clone(),
            };
            if parsed.items.is_empty() {
                return Err(bad(format!("`*** Add: {target}` defines no function, type or constant.")));
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
                let i = s.resolve(&ix, &target)?;
                (ix.syms[i].module.clone(), vec![(Some(i), ix.syms[i].text.clone())])
            }
        };
        // One list of lines, with a separator no hunk can match between units.
        const SEP: &str = "\u{0}";
        let mut lines: Vec<String> =
            units.iter().map(|u| u.1.clone()).collect::<Vec<_>>().join(&format!("\n{SEP}\n")).lines().map(String::from).collect();
        let mut cursor = 0;
        for (at, hunk) in hunks {
            let old: Vec<&str> = hunk.iter().filter(|l| !l.starts_with('+')).map(|l| &l[1..]).collect();
            let new: Vec<String> = hunk.iter().filter(|l| !l.starts_with('-')).map(|l| l[1..].to_string()).collect();
            let mut from = cursor;
            if let Some(at) = &at {
                let found = (cursor..lines.len()).chain(0..cursor).find(|&k| lines[k].trim().starts_with(at.as_str()));
                from = found
                    .ok_or_else(|| bad(format!("in {target}, no line starts with `@@ {at}`; `view {target}` shows the current text.")))?
                    + 1;
            }
            let at_k = if old.is_empty() {
                if at.is_none() {
                    return Err(bad(format!("in {target}, a hunk that only adds lines needs `@@ <the line before them>`.")));
                }
                Some(from)
            } else {
                let same: [&dyn Fn(&str, &str) -> bool; 3] =
                    [&|a, b| a == b, &|a, b| a.trim_end() == b.trim_end(), &|a, b| a.trim() == b.trim()];
                let fits = |k: usize, eq: &dyn Fn(&str, &str) -> bool| {
                    k + old.len() <= lines.len() && old.iter().zip(&lines[k..]).all(|(o, l)| eq(o, l))
                };
                same.iter().find_map(|eq| (from..lines.len()).chain(0..from).find(|&k| fits(k, *eq)))
            };
            let Some(k) = at_k else {
                let first = old.iter().find(|l| !l.trim().is_empty()).unwrap_or(&"");
                return Err(bad(format!(
                    "in {target}, these lines weren't found: `{}`{}; copy them exactly from `view {target}`.",
                    first.trim(),
                    if old.len() > 1 { format!(" (and {} more)", old.len() - 1) } else { String::new() }
                )));
            };
            cursor = k + new.len();
            lines.splice(k..k + old.len(), new);
        }
        let joined = lines.join("\n");
        let texts: Vec<&str> = joined.split(&format!("\n{SEP}\n")).collect();
        if texts.len() != units.len() {
            return Err(bad(format!("in {target}, a hunk spans two symbols; give each symbol its own hunk.")));
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
                None => return Err(bad(format!("the patch leaves nothing of {}; use `delete <name>`.", ix.syms[*i].qname()))),
                Some(item) if item.name != ix.syms[*i].name => {
                    return Err(bad(format!("the patch renames {}; submit it whole under its new name.", ix.syms[*i].qname())));
                }
                _ => {}
            }
            emit(&module, &std::mem::take(&mut header), text, &ix.syms[*i].text);
        }
        if !header.is_empty() {
            return Err(bad(format!(
                "the patch changes only {module}'s imports; imports are added automatically (`submit --imports {module}` replaces them)."
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
        // a patch (D-47) becomes the blocks of a plain `submit`; the draft keeps the blocks
        ["submit"] | ["submit", _] | ["submit", "--new" | "--imports", _] => {
            let text = input()?;
            if a.len() <= 2 && is_patch(&text) {
                (unique_id(), vec!["submit".into()], patch_blocks(s, &text)?)
            } else {
                (unique_id(), a.iter().map(|x| x.to_string()).collect(), text)
            }
        }
        _ => {
            return usage("submit [name [--replace [--all]] | --new module | --imports module | --draft id [--replace [--all]]]");
        }
    };
    let base = base(s, &ix, &args, &raw)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match refs.as_slice() {
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
            s.db.execute(
                "INSERT OR REPLACE INTO drafts(id,owner,args,text,base) VALUES (?,?,?,?,?)",
                params![id, owner(), json!(args).to_string(), raw, base],
            )?;
            Err(Failure(
                format!(
                    "{message}\n\nDraft {id} retained. Read: `view --draft {id}`. Fix and retry: `submit --draft {id} --replace` with `old` and `text`. List: `drafts`."
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
        assert!(err.starts_with("E0108 in app.main, these lines weren't found: `let x = calc.nope(1)`"), "{err}");
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
        let id: String = s.db.query_row("SELECT id FROM drafts", [], |r| r.get(0)).unwrap();
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
