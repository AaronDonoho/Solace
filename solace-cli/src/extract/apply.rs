//! Applying an extract: the journals, file changes and checks that make a planned move, or put everything back.

use super::*;
use crate::Store;
use crate::Index;
use crate::revision;
use crate::extract::plan::tail;
use crate::extract::deps::add_dependency;
use crate::extract::deps::Dest;
use crate::extract::deps::Dependent;
use crate::extract::ffi::module_dir;
use crate::extract::ffi::ffi_norm;
use crate::extract::ffi::js_imports;
use crate::extract::ffi::js_files;
use crate::extract::ffi::out_dir;
use crate::extract::plan::write_headers;
use crate::extract::plan::MovePlan;
use crate::failure::Failure;
use crate::failure::fail;
/// Ends every file change, kept or put back, even after one fails; the first error is returned.
pub(super) fn end_all<F: FnOnce(bool) -> Result<(), Failure>>(ends: Vec<F>, kept: bool) -> Result<(), Failure> {
    let mut first = None;
    for end in ends {
        if let Err(e) = end(kept)
            && first.is_none()
        {
            first = Some(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// The failure `e`, with the error of putting back what came before it, if any.
pub(crate) fn with_put_back(e: Failure, ended: Result<(), Failure>) -> Failure {
    match ended {
        Ok(()) => e,
        Err(end) => Failure(format!("{}\n\nPutting back also failed: {}", e.0, end.0), e.1),
    }
}

/// `body` in a transaction of each store, the first outermost; the last one
/// takes away the files it created when the body fails.
pub(crate) fn nested<T>(stores: &[&Store], created: &[PathBuf], body: &mut dyn FnMut() -> Result<T, Failure>) -> Result<T, Failure> {
    match stores {
        [] => body(),
        [last] => last.transaction(created, || body()),
        [first, rest @ ..] => first.transaction(&[], || nested(rest, created, body)),
    }
}

/// What an extract reports after its stores commit: suites that stopped
/// resolving and the modules it retired.
pub(crate) struct Finished {
    pub(crate) gone: Vec<String>,
    pub(crate) retired: Vec<String>,
}

pub(crate) fn extract_reply_text(r: &MovePlan<'_>, done: &Finished, budget: usize) -> Result<String, Failure> {
    let mut out = extract_summary(r, done);
    out += &extract_suites(r)?;
    out += &extract_javascript(r);
    out += &extract_outside(r);
    Ok(lockdown::trim_output(&out, budget))
}

fn extract_summary(r: &MovePlan<'_>, done: &Finished) -> String {
    let ix = r.ix;
    let q = |i: usize| ix.syms[i].qname();
    let mut out = format!("Moved {}", r.listed.iter().map(|&i| q(i)).collect::<Vec<_>>().join(", "));
    if !r.helpers.is_empty() {
        let names: Vec<&str> = r.helpers.iter().map(|&i| ix.syms[i].name.as_str()).collect();
        out += &format!(" (+{} helper{}: {})", r.helpers.len(), if r.helpers.len() == 1 { "" } else { "s" }, names.join(", "));
    }
    out += &format!(" to {}{}\n", r.to, r.dest.as_ref().map_or(String::new(), |d| format!(" in {}", d.dir)));
    let mut gained: Vec<String> = r.dep.iter().map(|_| workspace::name(&r.s.root)).collect();
    gained.extend(r.deps.iter().zip(&r.dep_lines).filter(|(_, l)| l.is_some()).map(|(dp, _)| workspace::name(&dp.store.root)));
    if let Some(d) = r.dest.as_ref().filter(|_| !gained.is_empty()) {
        out += &format!("Dependency added: {} -> {}\n", gained.join(", "), workspace::name(&d.store.root));
    }
    if !r.already.is_empty() {
        out += &format!("Already in {}: {}\n", r.to, r.already.join(", "));
    }
    if !r.now_pub.is_empty() {
        out += &format!("Now pub: {} (new revision)\n", r.now_pub.iter().map(|&i| ix.syms[i].name.as_str()).collect::<Vec<_>>().join(", "));
    }
    if r.plan.edits > 0 {
        out += &format!(
            "Rewrote {} reference{} in {} module{}\n",
            r.plan.edits,
            if r.plan.edits == 1 { "" } else { "s" },
            r.plan.edited.len(),
            if r.plan.edited.len() == 1 { "" } else { "s" }
        );
    }
    for (dp, p) in r.deps.iter().zip(&r.dplans).filter(|(_, p)| p.edits > 0) {
        let plural = |k: usize| if k == 1 { "" } else { "s" };
        out += &format!(
            "Rewrote {} reference{} in {} module{} in {}\n",
            p.edits,
            plural(p.edits),
            p.edited.len(),
            plural(p.edited.len()),
            dp.dir
        );
    }
    if !r.moved_tests.is_empty() {
        out += &format!(
            "Tests moved: {}\n",
            r.moved_tests.iter().map(|&t| format!("{} -> {}", q(t), r.newmod[t])).collect::<Vec<_>>().join(", ")
        );
    }
    for c in &r.carries {
        let imports = if c.text.is_some() { " (its imports rewritten)" } else { "" };
        out += &format!("Moved file {} to {}: {}{imports}\n", c.from, r.dest.as_ref().map_or("", |d| d.dir.as_str()), c.to);
    }
    if !r.ffi.is_empty() {
        out += &format!("Rewrote FFI paths: {}\n", r.ffi.join(", "));
    }
    for m in r.created.iter().map(|c| &c.0) {
        out += &format!("Created module {m}\n");
    }
    if !done.retired.is_empty() {
        out += &format!("Retired module {}\n", done.retired.join(", "));
    }
    for note in &r.notes {
        out += &format!("{note}\n");
    }
    for g in done.gone.iter().filter(|g| !g.is_empty()) {
        out += &format!("Not rewritten: {g} no longer resolves\n");
    }
    out
}

fn extract_suites(r: &MovePlan<'_>) -> Result<String, Failure> {
    let ix = r.ix;
    let ids: HashSet<&str> = (0..ix.syms.len()).filter(|&i| r.moved[i]).map(|i| ix.syms[i].id.as_str()).collect();
    let at = |x: &Index| -> Vec<usize> { (0..x.syms.len()).filter(|&i| ids.contains(x.syms[i].id.as_str())).collect() };
    let mut suites = vec![];
    match &r.dest {
        None => {
            let x = r.s.index()?;
            suites.extend(reach::reached(&reach::suites(r.s, &x)?, &at(&x)));
        }
        Some(d) => {
            let x = d.store.index()?;
            let hits = at(&x);
            for (name, reached) in reach::suites(&d.store, &x)? {
                if !hits.is_empty() && reached.as_ref().is_none_or(|r| hits.iter().any(|j| r.contains(j))) {
                    suites.push(format!("{name} in {0} (`run {name} --project {0}`)", d.dir));
                }
            }
            let callers: HashSet<&str> = (0..ix.syms.len())
                .filter(|&h| !r.moved[h] && crate::index::uses_split(ix, h).0.iter().any(|&u| r.moved[u]))
                .map(|h| ix.syms[h].id.as_str())
                .collect();
            let sx = r.s.index()?;
            let there: Vec<usize> = (0..sx.syms.len()).filter(|&i| callers.contains(sx.syms[i].id.as_str())).collect();
            if !there.is_empty() {
                suites.extend(reach::reached(&reach::suites(r.s, &sx)?, &there));
            }
            for (dp, p) in r.deps.iter().zip(&r.dplans) {
                let rewritten: HashSet<&str> =
                    dp.ix.syms.iter().enumerate().filter(|(i, s)| p.newtext[*i] != s.text).map(|(_, s)| s.id.as_str()).collect();
                if rewritten.is_empty() {
                    continue;
                }
                let x = dp.store.index()?;
                let there: Vec<usize> = (0..x.syms.len()).filter(|&i| rewritten.contains(x.syms[i].id.as_str())).collect();
                for (name, reached) in reach::suites(&dp.store, &x)? {
                    if !there.is_empty() && reached.as_ref().is_none_or(|r| there.iter().any(|j| r.contains(j))) {
                        suites.push(format!("{name} in {0} (`run {name} --project {0}`)", dp.dir));
                    }
                }
            }
        }
    }
    if !suites.is_empty() {
        Ok(format!("Also reached: {}\n", suites.join(", ")))
    } else if !r.ffi.is_empty() || !r.carries.is_empty() {
        Ok(concat!(
            "JavaScript FFI paths are not type-checked, and no declared suite reaches the moved code: ",
            "run the JavaScript tests, or declare them in solace.toml with `checks` (D-44)\n",
        )
        .to_string())
    } else {
        Ok(String::new())
    }
}

fn extract_javascript(r: &MovePlan<'_>) -> String {
    let ix = r.ix;
    let mut moved_names: HashSet<String> = HashSet::new();
    for i in (0..ix.syms.len()).filter(|&i| r.moved[i]) {
        moved_names.insert(ix.syms[i].name.clone());
        if ix.syms[i].kind == Kind::Type {
            moved_names.extend(ix.lang.constructors(&ix.syms[i].text));
        }
    }
    let olds: Vec<Vec<String>> = r
        .old_modules
        .iter()
        .map(|m| {
            let mut p = module_dir(m);
            p.push(format!("{}.mjs", tail(m)));
            p
        })
        .collect();
    let mut js_hits = vec![];
    for f in js_files(&r.s.root) {
        let Ok(text) = fs::read_to_string(r.s.root.join(&f)) else { continue };
        for imp in js_imports(&text) {
            if olds.contains(&ffi_norm(&out_dir(&f), &imp.spec)) && imp.names.iter().any(|x| moved_names.contains(x)) {
                js_hits.push(format!("{f}:{}", imp.line));
            }
        }
    }
    if js_hits.is_empty() {
        return String::new();
    }
    let (s, it) = if js_hits.len() == 1 { ("", "it") } else { ("s", "them") };
    format!("Not rewritten: JavaScript import{s} of moved code from its old module ({}): change {it} with `write`\n", js_hits.join(", "))
}

fn extract_outside(r: &MovePlan<'_>) -> String {
    let ix = r.ix;
    let mut needles: Vec<String> =
        (0..ix.syms.len()).filter(|&i| r.moved[i]).map(|i| format!("{}.{}", ix.syms[i].module, ix.syms[i].name)).collect();
    needles.extend(r.old_modules.iter().map(|m| format!("-m {m}")));
    let (mut hits, mut files): (Vec<String>, BTreeSet<String>) = (vec![], BTreeSet::new());
    if let Ok(settings) = lockdown::settings(&r.s.root)
        && let Ok(paths) = lockdown::watched_paths(&r.s.root, &settings)
    {
        for path in paths {
            let Ok(text) = fs::read_to_string(r.s.root.join(&path)) else { continue };
            for (k, line) in text.lines().enumerate() {
                let found = needles.iter().any(|nd| {
                    line.match_indices(nd.as_str()).any(|(at, _)| {
                        let word = |c: char| c.is_alphanumeric() || c == '_' || c == '/';
                        !line[..at].ends_with(word) && !line[at + nd.len()..].starts_with(word)
                    })
                });
                if found {
                    hits.push(format!("{path}:{}", k + 1));
                    files.insert(path.clone());
                }
            }
        }
    }
    if hits.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = hits.iter().take(5).map(String::as_str).collect();
    let more = if hits.len() > 5 { ", …" } else { "" };
    format!(
        "Not rewritten: {} place{} in {} file{} ({}{more})\n",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" },
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        shown.join(", ")
    )
}

/// A move's transaction: the plan, and what the stores held before it.
pub(crate) struct MoveRun<'a> {
    pub(crate) plan: &'a MovePlan<'a>,
    pub(crate) before: &'a HashMap<String, String>,
    pub(crate) dest_broken: &'a HashMap<String, String>,
    pub(crate) deps_broken: &'a [HashMap<String, String>],
    pub(crate) suites_before: &'a [(String, String)],
}

pub(crate) fn run_move(r: &MoveRun<'_>) -> Result<Finished, Failure> {
    let plan = r.plan;
    let tgt = plan.dest.as_ref().map_or(plan.s, |d| &d.store);

    apply_symbols(r, tgt)?;
    let finals = apply_headers(r, tgt)?;
    let retired = retire_modules(r, &finals)?;
    let carried = carry_ffi_files(plan, tgt)?;
    let tomls = match add_dependencies(plan) {
        Ok(tomls) => tomls,
        Err(e) => return Err(with_put_back(e, end_all(carried, false))),
    };
    let checked = check_moved(r, retired);
    // every end runs, so a failing one never leaves the rest undecided
    let (a, b) = (end_all(tomls, checked.is_ok()), end_all(carried, checked.is_ok()));
    a?;
    b?;
    checked
}

pub(crate) fn created_modules(
    s: &Store,
    dest: &Option<Dest>,
    ix: &Index,
    to: &str,
    test_to: &str,
    target_is_test: bool,
    moved: &[bool],
    newmod: &[String],
    moved_tests: &[usize],
) -> Result<Vec<(String, &'static str)>, Failure> {
    let has = |m: &str| dest.as_ref().map_or_else(|| ix.modules.iter().any(|r| r.path == m), |d| d.has(m));
    let mut created: Vec<(String, &'static str)> = vec![];
    if !has(to) && (0..ix.syms.len()).any(|i| moved[i] && newmod[i] == to) {
        created.push((to.to_string(), if target_is_test { "test" } else { "src" }));
    }
    if !moved_tests.is_empty() && !target_is_test && !has(test_to) && !created.iter().any(|c| c.0 == test_to) {
        created.push((test_to.to_string(), "test"));
    }
    let tgt = dest.as_ref().map_or(s, |d| &d.store);
    if created.iter().any(|(m, dir)| tgt.root.join(dir).join(format!("{m}.gleam")).exists()) {
        return fail("E0101 the new module's file already exists outside the store; import or resolve it before creating this module");
    }
    Ok(created)
}

pub(crate) struct Baseline {
    pub(crate) suites_before: Vec<(String, String)>,
    pub(crate) before: HashMap<String, String>,
    pub(crate) dest_broken: HashMap<String, String>,
    pub(crate) deps_broken: Vec<HashMap<String, String>>,
}

pub(crate) fn baseline(s: &Store, ix: &Index, dest: &Option<Dest>, deps: &[Dependent]) -> Result<Baseline, Failure> {
    let suites_before = reach::entries(s)?.into_iter().filter(|(_, e)| s.resolve(ix, e).is_ok()).collect();
    let before = s.broken()?;
    let dest_broken = match dest {
        Some(d) => d.store.broken()?,
        None => HashMap::new(),
    };
    let deps_broken = deps.iter().map(|dp| dp.store.broken()).collect::<Result<Vec<_>, _>>()?;
    Ok(Baseline { suites_before, before, dest_broken, deps_broken })
}

fn apply_symbols(r: &MoveRun<'_>, tgt: &Store) -> Result<(), Failure> {
    let (s, ix, dest, plan, moved, newmod, created, mine) =
        (r.plan.s, r.plan.ix, &r.plan.dest, &r.plan.plan, &r.plan.moved, &r.plan.newmod, &r.plan.created, r.plan.mine);
    for (path, dir) in created {
        tgt.db.execute("INSERT INTO modules(path, dir, header) VALUES (?1, ?2, ?3)", params![path, dir, ""])?;
    }
    for i in 0..ix.syms.len() {
        let sym = &ix.syms[i];
        if moved[i]
            && let Some(d) = dest
        {
            // the same identity in the destination; the source keeps the retired row and its history
            let (kind, rev): (String, String) =
                s.db.query_row("SELECT kind, revision FROM symbols WHERE id = ?1", [&sym.id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let rev = if plan.newtext[i] != sym.text { revision(&plan.newtext[i]) } else { rev };
            let ord: i64 =
                tgt.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&newmod[i]], |row| row.get(0))?;
            tgt.db.execute(
                "INSERT OR REPLACE INTO symbols(id,module,name,kind,text,ord,revision,live) VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                params![sym.id, newmod[i], sym.name, kind, plan.newtext[i], ord, rev],
            )?;
            tgt.history(&sym.id, "move", &format!("{mine}: {}", sym.module))?;
            s.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [&sym.id])?;
            s.history(&sym.id, "move", &format!("to {}: {}", d.dir, newmod[i]))?;
            continue;
        }
        if moved[i] {
            let ord: i64 =
                s.db.query_row("SELECT COALESCE(MAX(ord), 0) + 1 FROM symbols WHERE module = ?1", [&newmod[i]], |row| row.get(0))?;
            s.db.execute("UPDATE symbols SET module = ?2, ord = ?3 WHERE id = ?1", params![sym.id, newmod[i], ord])?;
            s.history(&sym.id, "move", &sym.module)?;
        }
        if plan.newtext[i] != sym.text {
            if moved[i] {
                s.db.execute(
                    "UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1",
                    params![sym.id, plan.newtext[i], revision(&plan.newtext[i])],
                )?;
            } else {
                s.set_symbol_text(&sym.id, &sym.name, &plan.newtext[i])?;
            }
        }
    }
    Ok(())
}

fn apply_headers(r: &MoveRun<'_>, tgt: &Store) -> Result<Vec<(String, String)>, Failure> {
    let (s, ix, dest, plan, created) = (r.plan.s, r.plan.ix, &r.plan.dest, &r.plan.plan, &r.plan.created);
    let (to, test_to, deps, dplans) = (r.plan.to.as_str(), r.plan.test_to.as_str(), &r.plan.deps, &r.plan.dplans);
    let mut headers: Vec<(String, String, &Store)> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), s)).collect();
    headers.extend(created.iter().map(|(m, _)| (m.clone(), String::new(), tgt)));
    if let Some(d) = dest {
        headers.extend(d.ix.modules.iter().filter(|m| m.path == to || m.path == test_to).map(|m| (m.path.clone(), m.header.clone(), tgt)));
    }
    let finals = write_headers(plan, &headers)?;
    for (dp, p) in deps.iter().zip(dplans) {
        for (i, sym) in dp.ix.syms.iter().enumerate() {
            if p.newtext[i] != sym.text {
                dp.store.set_symbol_text(&sym.id, &sym.name, &p.newtext[i])?;
            }
        }
        let modules: Vec<(String, String, &Store)> = dp.ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), &dp.store)).collect();
        write_headers(p, &modules)?;
    }
    Ok(finals)
}

fn retire_modules(r: &MoveRun<'_>, finals: &[(String, String)]) -> Result<Vec<String>, Failure> {
    let (s, ix, newmod, old_modules) = (r.plan.s, r.plan.ix, &r.plan.newmod, &r.plan.old_modules);
    let mut retired = vec![];
    for m in old_modules {
        let empty = !(0..ix.syms.len()).any(|i| newmod[i] == *m);
        let imported = finals.iter().any(|(o, h)| o != m && ix.lang.imports(h).iter().any(|i| i.module == *m));
        if empty && !imported {
            s.db.execute("DELETE FROM modules WHERE path = ?1", [m])?;
            retired.push(m.clone());
        }
    }
    Ok(retired)
}

/// The JavaScript files the moved code takes along; one that fails puts back those before it.
fn carry_ffi_files(plan: &MovePlan<'_>, tgt: &Store) -> Result<Vec<impl FnOnce(bool) -> Result<(), Failure>>, Failure> {
    let mut carried = vec![];
    for c in &plan.carries {
        match lockdown::carry_file(&plan.s.root, &c.from, &tgt.root, &c.to, c.text.clone()) {
            Ok(end) => carried.push(end),
            Err(e) => return Err(with_put_back(e, end_all(carried, false))),
        }
    }
    Ok(carried)
}

/// The gleam.toml dependency lines the source and the dependents gain; one that fails puts back those before it.
fn add_dependencies(plan: &MovePlan<'_>) -> Result<Vec<impl FnOnce(bool) -> Result<(), Failure>>, Failure> {
    let mut adds: Vec<(&Path, &str)> = vec![];
    if let Some((_, line)) = &plan.dep {
        adds.push((plan.s.root.as_path(), line.as_str()));
    }
    for (dp, line) in plan.deps.iter().zip(&plan.dep_lines) {
        if let Some(line) = line {
            adds.push((dp.store.root.as_path(), line.as_str()));
        }
    }
    let mut tomls = vec![];
    for (root, line) in adds {
        let changed = fs::read_to_string(root.join("gleam.toml"))
            .map_err(Failure::from)
            .and_then(|text| lockdown::change_file(root, "gleam.toml", &add_dependency(&text, line)));
        match changed {
            Ok(end) => tomls.push(end),
            Err(e) => return Err(with_put_back(e, end_all(tomls, false))),
        }
    }
    Ok(tomls)
}

/// The three stores' type checks, then the suites that stopped resolving.
fn check_moved(r: &MoveRun<'_>, retired: Vec<String>) -> Result<Finished, Failure> {
    let plan = r.plan;
    let s = plan.s;
    let mut bad: Vec<String> = vec![];
    // the destination's files first: Gleam reads a path dependency from disk
    if let Some(d) = &plan.dest {
        let diags = d.store.check_all(&d.store.index()?)?;
        bad.extend(
            diags
                .iter()
                .filter(|g| g.error && g.symbol.as_ref().is_none_or(|sym| !r.dest_broken.contains_key(sym)))
                .map(|g| format!("{}: {}", d.dir, g.text)),
        );
    }
    let ix2 = s.index()?;
    let diags = s.check_all(&ix2)?;
    bad.extend(diags.iter().filter(|g| g.error && g.symbol.as_ref().is_none_or(|sym| !r.before.contains_key(sym))).map(|g| g.text.clone()));
    // the dependents last: they read both from disk
    for (dp, broken) in plan.deps.iter().zip(r.deps_broken) {
        let diags = dp.store.check_all(&dp.store.index()?)?;
        bad.extend(
            diags
                .iter()
                .filter(|g| g.error && g.symbol.as_ref().is_none_or(|sym| !broken.contains_key(sym)))
                .map(|g| format!("{}: {}", dp.dir, g.text)),
        );
    }
    if !bad.is_empty() {
        return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", bad.join("\n\n")));
    }
    let gone: Vec<String> =
        r.suites_before.iter().filter(|(_, e)| s.resolve(&ix2, e).is_err()).map(|(suite, e)| format!("suite {suite} entry {e}")).collect();
    Ok(Finished { gone, retired })
}
