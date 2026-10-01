//! Rejected submissions are scratch text, never live symbols.
use super::*;
use serde_json::json;

fn owner() -> String {
    env::var("SOLACE_TASK").unwrap_or_else(|_| "cli".into())
}

fn setup(s: &Store) -> Result<(), Failure> {
    s.db.execute_batch("CREATE TABLE IF NOT EXISTS drafts(id TEXT PRIMARY KEY, owner TEXT NOT NULL, args TEXT NOT NULL, text TEXT NOT NULL, base TEXT NOT NULL)")?;
    Ok(())
}

pub fn list(s: &Store) -> Out {
    list_for(s, &owner())
}

pub fn list_for(s: &Store, task: &str) -> Out {
    setup(s)?;
    let mut q = s.db.prepare("SELECT id, args FROM drafts WHERE owner=? ORDER BY rowid")?;
    let rows = q.query_map([task], |r| Ok(format!("{}: {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?.join("\n"))
}

fn load(s: &Store, id: &str) -> Result<(Vec<String>, String, String), Failure> {
    setup(s)?;
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

fn replace(text: &str, old: Option<&str>, new: &str) -> Out {
    let old = old.filter(|x| !x.is_empty()).ok_or_else(|| Failure("E0104 provide nonempty exact text in `old`".into(), 2))?;
    if text.matches(old).count() != 1 {
        return fail("E0105 `old` must occur exactly once; view the draft/current symbol before replacing");
    }
    Ok(text.replacen(old, new, 1))
}

pub fn submit(s: &Store, a: &[&str], input: &mut dyn FnMut() -> Out, old: Option<&str>) -> Out {
    setup(s)?;
    // ponytail: fingerprint the whole source store; narrow to touched modules if unrelated edits frequently invalidate drafts.
    let base = hex(&Sha256::digest(board::snapshot(s)?.to_string().as_bytes()));
    let (id, args, raw) = match a {
        ["submit", "--draft", id] | ["submit", "--draft", id, "--replace"] => {
            let (args, text, expected) = load(s, id)?;
            if base != expected {
                return fail(format!(
                    "E0107 draft {id} is stale: source changed since it was created. `view --draft {id}` preserves your text; inspect current source and make a new submission."
                ));
            }
            let raw = if a.len() == 4 { replace(&text, old, &input()?)? } else { text };
            (id.to_string(), args, raw)
        }
        ["submit", name, "--replace"] => {
            let ix = s.index()?;
            let symbol = &ix.syms[s.resolve(&ix, name)?];
            let raw = replace(&symbol.text, old, &input()?)?;
            (new_id(Kind::Fn, "draft", ""), vec!["submit".into(), symbol.qname()], raw)
        }
        ["submit"] | ["submit", _] | ["submit", "--new" | "--imports", _] => {
            (new_id(Kind::Fn, "draft", ""), a.iter().map(|x| x.to_string()).collect(), input()?)
        }
        _ => {
            return usage("submit [name | --new module | --imports module | --draft id [--replace]]");
        }
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match refs.as_slice() {
        ["submit"] => s.submit_symbols(None, None, &raw),
        ["submit", "--new", module] => s.submit_symbols(None, Some(module), &raw),
        ["submit", "--imports", module] => s.submit(Target::Imports(module), &raw),
        ["submit", name] if !name.contains('.') && s.is_module(name)? => s.submit(Target::Module(name), &raw),
        ["submit", name] => s.submit_symbols(Some(name), None, &raw),
        _ => return fail("E0106 invalid draft command"),
    };
    match result {
        Ok(out) => {
            s.db.execute("DELETE FROM drafts WHERE id=? AND owner=?", params![id, owner()])?;
            Ok(out)
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
    setup(s)?;
    s.db.execute("DELETE FROM drafts WHERE owner=?", [task])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_text_survives_and_retries_atomically() {
        let root = env::temp_dir().join(new_id(Kind::Fn, "draft-test", ""));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("gleam.toml"), "name=\"app\"\nversion=\"1.0.0\"\n").unwrap();
        fs::write(root.join("src/app.gleam"), "pub fn main() { 1 }\n").unwrap();
        import(&root).unwrap();
        let s = board::open(&root).unwrap();
        let original = board::snapshot(&s).unwrap();
        let raw = "// ── app.main ──\npub fn main() { helper() }\n// ── app.helper ──\nfn helper() { -. 2 }\n";
        assert!(submit(&s, &["submit"], &mut || Ok(raw.into()), None).unwrap_err().0.contains("Draft"));
        let id: String = s.db.query_row("SELECT id FROM drafts", [], |r| r.get(0)).unwrap();
        assert_eq!(view(&s, &id).unwrap(), raw);
        assert_eq!(board::snapshot(&s).unwrap(), original);
        // A second failed attempt replaces the same draft, including invalid syntax.
        assert!(submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok("unknown()".into()), Some("-. 2")).is_err());
        assert!(view(&s, &id).unwrap().contains("unknown()"));
        assert_eq!(list(&s).unwrap().lines().count(), 1);
        assert_eq!(board::snapshot(&s).unwrap(), original);
        let out = submit(&s, &["submit", "--draft", &id, "--replace"], &mut || Ok("2".into()), Some("unknown()")).unwrap();
        assert!(out.contains("Saved"));
        assert!(list(&s).unwrap().is_empty());
        assert!(s.view(&["app.main"], false, BUDGET).unwrap().contains("helper()"));
        // Module creation and whole-module submissions use the same draft path.
        for args in [vec!["submit", "--new", "other"], vec!["submit", "app"]] {
            assert!(submit(&s, &args, &mut || Ok("pub fn main() { absent() }".into()), None).is_err());
        }
        assert!(!root.join("src/other.gleam").exists(), "rejected modules must leave no compiler input behind");
        let stale: String = s.db.query_row("SELECT id FROM drafts ORDER BY rowid LIMIT 1", [], |r| r.get(0)).unwrap();
        s.submit_symbols(Some("app.helper"), None, "fn helper() { 3 }").unwrap();
        assert!(submit(&s, &["submit", "--draft", &stale], &mut || Ok(String::new()), None).unwrap_err().0.contains("E0107"));
        assert!(view(&s, &stale).unwrap().contains("absent()"));
        s.db.execute("UPDATE drafts SET owner='another-task' WHERE id=?", [&stale]).unwrap();
        assert!(view(&s, &stale).unwrap_err().0.contains("E0106"));
        clear(&s, "another-task").unwrap();
        assert!(view(&s, &stale).is_err());
        let warning = s.submit_symbols(None, Some("app"), "fn unused() { 1 }").unwrap();
        assert!(warning.contains("app.unused: warning:"), "{warning}");
        assert!(warning.contains("check --warnings"));
        assert!(!warning.contains("│"), "{warning}");
        let full = s.check_report(true).unwrap();
        assert!(full.contains("warning:") && full.contains("│"), "{full}");
        fs::remove_dir_all(root).unwrap();
    }
}
