//! `merge <survivor> <retired> [--to-project <folder>]` (D-117): folds a
//! duplicate into its copy. The survivor keeps its identity; the other is
//! retired with a `merged-into` history entry, and every reference to it, here
//! and in the dependents the task covers, is redirected through `extract`'s
//! rewrite. One transaction, gated on `check_all`.
use super::*;

const FORM: &str = "merge <survivor> <retired> [--to-project <folder>]";

struct Args {
    survivor: String,
    retired: String,
    to_project: Option<String>,
}

fn parse(args: &[&str]) -> Result<Args, Failure> {
    let (mut names, mut to_project) = (vec![], None);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--to-project" => {
                to_project = Some(it.next().ok_or_else(|| Failure(format!("--to-project needs the survivor's project folder: {FORM}"), 2))?.to_string())
            }
            _ if a.starts_with("--") => return usage(format!("unknown option {a}: {FORM}")),
            _ => names.push(a.to_string()),
        }
    }
    match <[String; 2]>::try_from(names) {
        Ok([survivor, retired]) => Ok(Args { survivor, retired, to_project }),
        Err(_) => usage(format!("merge takes two names, the one to keep first: {FORM}")),
    }
}

/// How two types' constructors differ by name, arity and labels; empty when they match.
fn constructor_differences(keep: &Sym, drop: &Sym) -> Vec<String> {
    let (a, b) = (gleam::record_fields(&keep.text), gleam::record_fields(&drop.text));
    let label = |o: &Option<String>| o.clone().unwrap_or_else(|| "no label".into());
    let mut out = vec![];
    for (name, fields) in &b {
        match a.iter().find(|(n, _)| n == name) {
            None => out.push(format!("{name} only in {}", drop.qname())),
            Some((_, kept)) if kept.len() != fields.len() => {
                out.push(format!("{name} has {} fields in {} and {} in {}", kept.len(), keep.qname(), fields.len(), drop.qname()))
            }
            Some((_, kept)) => {
                for (k, (x, y)) in kept.iter().zip(fields).enumerate() {
                    if x != y {
                        out.push(format!("{name} field {}: {} in {}, {} in {}", k + 1, label(x), keep.qname(), label(y), drop.qname()));
                    }
                }
            }
        }
    }
    for (name, _) in a.iter().filter(|(n, _)| !b.iter().any(|(m, _)| m == n)) {
        out.push(format!("{name} only in {}", keep.qname()));
    }
    out
}

/// "x (by m.f), …": the uses of `gone`'s name by `users`.
fn uses_list(name: &str, users: &[String]) -> String {
    users.iter().map(|u| format!("{name} (by {u})")).collect::<Vec<_>>().join(", ")
}

/// The gleam.toml line that makes the project in folder `from` depend on `d`,
/// and where it goes.
fn dependency_line(from: &str, d: &Dest) -> String {
    let toml = if from == "." { "gleam.toml".to_string() } else { format!("{from}/gleam.toml") };
    format!("{} = {{ path = \"{}\" }} to {toml}", workspace::name(&d.store.root), relative(from, &d.dir))
}

/// The rewrite of the symbols before `upto` in `x`, with every reference to
/// `r` redirected to `s` (indices in `x`).
fn plan_for(x: &Index, upto: usize, headers: HashMap<String, String>, r: usize, s: usize) -> Plan<'_> {
    let mut moved = vec![false; x.syms.len()];
    moved[r] = true;
    let mut newtext: Vec<String> = x.syms.iter().map(|s| s.text.clone()).collect();
    // the retired symbol's text no longer counts toward its module's imports
    newtext[r] = String::new();
    Plan {
        ix: x,
        names: name_map(x),
        headers,
        foreign: HashSet::new(),
        moved,
        newmod: x.syms.iter().map(|s| s.module.clone()).collect(),
        newtext,
        needs: BTreeMap::new(),
        upto,
        edits: 0,
        edited: BTreeSet::new(),
        redirect: Some((r, s)),
        hits: 0,
        hit_in: BTreeSet::new(),
    }
}

impl Store {
    /// The other projects of the run's scope, each with its folder and index:
    /// where a type's copies outside this project are. Built once per hint.
    fn scope_indexes(&self) -> Vec<(String, Index)> {
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let scope_of: Option<Vec<String>> = scope::of_run(&self.root);
        let mut out = vec![];
        for d in workspace::members(&top).unwrap_or_default() {
            let dir = scope::path(&top, &d);
            if d == mine || !scope_of.as_ref().is_none_or(|ps| ps.contains(&d)) || !dir.join("gleam.toml").exists() {
                continue;
            }
            if let Some(other) = board::open(&dir).ok().and_then(|st| st.index().ok()) {
                out.push((d, other));
            }
        }
        out
    }

    /// Types named as `gone` is: in this project's other modules, then in the
    /// other projects of the run's scope (`others`), with their folder.
    /// Functions and constants get none: a shared name like `new` rarely means
    /// a duplicate.
    fn copies(ix: &Index, others: &[(String, Index)], gone: &Sym) -> Vec<(Option<String>, Sym)> {
        if gone.kind != Kind::Type {
            return vec![];
        }
        let same = |x: &Sym| x.name == gone.name && x.kind == Kind::Type;
        let mut out: Vec<(Option<String>, Sym)> = ix.syms.iter().filter(|x| same(x) && x.module != gone.module).map(|x| (None, x.clone())).collect();
        for (d, other) in others {
            out.extend(other.syms.iter().filter(|x| same(x)).map(|x| (Some(d.clone()), x.clone())));
        }
        out
    }

    /// For a refused delete or a rejected submit: the `merge` that folds `gone`
    /// into its same-name copy, or what to make match first; empty with no copy.
    pub(crate) fn merge_hint(&self, ix: &Index, gone: &Sym, uses: usize) -> String {
        if gone.kind != Kind::Type {
            return String::new();
        }
        Self::hint_among(ix, &self.scope_indexes(), gone, uses)
    }

    /// `merge_hint` with the other projects of the scope already indexed.
    fn hint_among(ix: &Index, others: &[(String, Index)], gone: &Sym, uses: usize) -> String {
        let copies = Self::copies(ix, others, gone);
        let at = |d: &Option<String>| d.as_ref().map_or(String::new(), |d| format!(" ({d})"));
        let flag = |d: &Option<String>| d.as_ref().map_or(String::new(), |d| format!(" --to-project {d}"));
        let g = gone.qname();
        match copies.as_slice() {
            [] => String::new(),
            [(d, keep)] => {
                let k = keep.qname();
                let diffs = constructor_differences(keep, gone);
                if !is_pub_text(&keep.text) && !diffs.is_empty() {
                    format!(
                        "\n\nHint: {k}{} has the same name but is private, and their constructors differ: {}. Make it pub and make them match, then use merge {k} {g}{}",
                        at(d),
                        diffs.join("; "),
                        flag(d)
                    )
                } else if !is_pub_text(&keep.text) {
                    format!("\n\nHint: {k}{} has the same name but is private. Make it pub, then use merge {k} {g}{}", at(d), flag(d))
                } else if diffs.is_empty() {
                    let noun = if uses == 1 { "use" } else { "uses" };
                    format!("\n\nHint: {k}{} has the same name. To fold {g} into it and redirect its {uses} {noun}, use merge {k} {g}{}", at(d), flag(d))
                } else {
                    format!("\n\nHint: {k}{} has the same name, but their constructors differ: {}. Make them match, then use merge {k} {g}{}", at(d), diffs.join("; "), flag(d))
                }
            }
            _ => {
                let mark = |k: &Sym| match (is_pub_text(&k.text), constructor_differences(k, gone).is_empty()) {
                    (true, true) => "",
                    (false, true) => " (private)",
                    (true, false) => " (constructors differ)",
                    (false, false) => " (private, constructors differ)",
                };
                let names: Vec<String> = copies.iter().map(|(d, k)| format!("{}{}{}", k.qname(), at(d), mark(k))).collect();
                format!("\n\nHint: {} have the same name as {g}. To fold {g} into one and redirect its uses, use merge <copy> {g} [--to-project <folder>]", names.join(", "))
            }
        }
    }

    /// For a rejected submit whose errors name two types of one name (`Task` or
    /// `board.Task` beside `solace_task.Task`): the merge hint, once. `ix` is
    /// the index the errors were checked against.
    ///
    /// It reads the Gleam compiler's error text word by word: a type from
    /// another module is written `module.Type` (the module's last path part),
    /// and a type of the failing symbol's own module may be written bare. Two
    /// such words for one type name, from different modules, make the pair:
    ///
    /// ```text
    /// Type mismatch
    ///
    /// Expected type:
    ///
    ///     Task
    ///
    /// Found type:
    ///
    ///     solace_task.Task
    /// ```
    pub(crate) fn same_name_hint(&self, ix: &Index, errors: &[&Diag]) -> String {
        // the scope's other projects, indexed once and only when a pair is found
        let others = std::cell::OnceCell::new();
        for d in errors {
            let here = d.symbol.as_deref().and_then(|q| ix.syms.iter().find(|s| s.qname() == q)).map(|s| s.module.clone());
            let words: Vec<&str> = d.text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '/')).map(|w| w.trim_matches('.')).collect();
            let qualified: Vec<(&str, &str)> = words
                .iter()
                .filter_map(|w| w.rsplit_once('.'))
                .filter(|(m, t)| !m.is_empty() && m.starts_with(|c: char| c.is_ascii_lowercase()) && t.starts_with(|c: char| c.is_ascii_uppercase()))
                .collect();
            for &(_, t) in &qualified {
                let last = |m: &str| m.rsplit('/').next().unwrap_or(m).to_string();
                let mut mods: Vec<String> = vec![];
                if let Some(h) = &here
                    && words.contains(&t)
                {
                    mods.push(last(h));
                }
                for &(m, u) in &qualified {
                    if u == t && !mods.contains(&last(m)) {
                        mods.push(last(m));
                    }
                }
                if mods.len() < 2 {
                    continue;
                }
                // the one in the failing symbol's module goes first: it is the one left behind
                mods.sort_by_key(|m| here.as_ref().is_none_or(|h| last(h) != *m));
                for m in &mods {
                    let Some(i) = ix.syms.iter().position(|s| last(&s.module) == *m && s.name == t && s.kind == Kind::Type) else { continue };
                    let others = others.get_or_init(|| self.scope_indexes());
                    let copies = Self::copies(ix, others, &ix.syms[i]);
                    if copies.iter().any(|(_, c)| mods.contains(&last(&c.module))) {
                        return Self::hint_among(ix, others, &ix.syms[i], ix.used_by[i].len());
                    }
                }
            }
        }
        String::new()
    }

    /// `merge <survivor> <retired> [--to-project <folder>]` (D-117).
    pub(crate) fn merge(&self, args: &[&str], budget: usize) -> Out {
        let p = parse(args)?;
        let ix = self.index()?;
        let n = ix.syms.len();
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let members = workspace::members(&top).unwrap_or_default();

        // the survivor's project, with --to-project
        let dest = match &p.to_project {
            None => None,
            Some(d) => {
                // `../x` and `./x` resolve against this project, as `--project` does (D-126 point 3).
                let d = project_folder(&top, Some(&self.root), d, &members);
                if !members.iter().any(|m| *m == d) {
                    return fail(format!("E6007 {d} is not a project under this Solace root; its projects: {}", members.join(", ")));
                }
                let dir = scope::path(&top, &d);
                if lockdown::same_project(&dir, &self.root) {
                    None
                } else if !dir.join("gleam.toml").exists() {
                    return fail(format!("E6013 {d} has no Gleam code to merge into"));
                } else {
                    let store = Store::at(&dir)?;
                    let ix = store.index()?;
                    Some(Dest { dir: d.to_string(), store, ix })
                }
            }
        };

        // the two symbols: the survivor's index is after this project's in `jx`
        let r = self.resolve(&ix, &p.retired)?;
        let joint: Index;
        let (jx, s): (&Index, usize) = match &dest {
            Some(d) => {
                let s = d.store.resolve(&d.ix, &p.survivor)?;
                joint = merged(&ix, &d.ix);
                (&joint, n + s)
            }
            None => match self.resolve(&ix, &p.survivor) {
                Ok(s) => (&ix, s),
                Err(e) if e.0.starts_with("E1001") => {
                    // a survivor in exactly one other project: say how to name it
                    // (in a board run, only the task's own projects: others are refused)
                    let scope_of: Option<Vec<String>> = scope::of_run(&self.root);
                    let found: Vec<&String> = members
                        .iter()
                        .filter(|d| **d != mine && scope_of.as_ref().is_none_or(|ps| ps.contains(d)) && scope::path(&top, d).join("gleam.toml").exists())
                        .filter(|d| board::open(&scope::path(&top, d)).ok().and_then(|st| st.resolve(&st.index().ok()?, &p.survivor).ok()).is_some())
                        .collect();
                    return match found.as_slice() {
                        [d] => fail(format!("E1001 {} is not in this project; it is in {d}: use merge {} {} --to-project {d}", p.survivor, p.survivor, p.retired)),
                        _ => Err(e),
                    };
                }
                Err(e) => return Err(e),
            },
        };
        let (keep, gone) = (&jx.syms[s], &ix.syms[r]);
        if dest.is_none() && s == r {
            return usage(format!("{} is one symbol; merge takes two: {FORM}", gone.qname()));
        }
        let at = |d: &Option<Dest>| d.as_ref().map_or(String::new(), |d| format!(" ({})", d.dir));
        let what = |x: &Sym| if x.kind == Kind::Type { "a type" } else { "a function or constant" };
        if (keep.kind == Kind::Type) != (gone.kind == Kind::Type) {
            return fail(format!(
                "E5008 {} is {} and {} is {}; merge folds a symbol into one of the same kind",
                gone.qname(),
                what(gone),
                keep.qname(),
                what(keep)
            ));
        }
        if gone.kind == Kind::Type {
            let diffs = constructor_differences(keep, gone);
            if !diffs.is_empty() {
                return fail(format!(
                    "E5008 the constructors of {} and {} differ; make them match by name, arity and labels, then merge:\n  {}",
                    keep.qname(),
                    gone.qname(),
                    diffs.join("\n  ")
                ));
            }
        }
        if jx.uses[s].contains(&r) {
            return fail(format!("E5002 {} uses {}, which would then be itself: change {} first", keep.qname(), gone.qname(), keep.qname()));
        }

        // this project's uses of the retired symbol, the retired symbol aside
        let users: Vec<usize> = ix.used_by[r].iter().copied().filter(|&j| j != r).collect();
        let user_names: Vec<String> = users.iter().map(|&j| ix.syms[j].qname()).collect();
        if !is_pub_text(&keep.text) && users.iter().any(|&j| ix.syms[j].module != keep.module) {
            return fail(format!("E5009 {} is private and {} would use it: make it pub first, then merge", keep.qname(), user_names.join(", ")));
        }

        // a module cycle, or with --to-project a package cycle or a missing dependency
        match &dest {
            None => {
                let edges = |redirect: bool| {
                    let mut e: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                    for h in (0..n).filter(|&h| !(redirect && h == r)) {
                        for &u in &ix.uses[h] {
                            let u = if redirect && u == r { s } else { u };
                            if ix.syms[h].module != ix.syms[u].module {
                                e.entry(ix.syms[h].module.clone()).or_default().insert(ix.syms[u].module.clone());
                            }
                        }
                    }
                    e
                };
                if cyclic(&edges(true)) && !cyclic(&edges(false)) {
                    let apart: Vec<String> = users.iter().filter(|&&j| ix.syms[j].module != keep.module).map(|&j| ix.syms[j].qname()).collect();
                    return fail(format!(
                        "E5002 redirecting {} to {} closes an import loop: {}; change those uses first",
                        gone.qname(),
                        keep.qname(),
                        uses_list(&gone.name, &apart)
                    ));
                }
            }
            Some(d) if !users.is_empty() => {
                if depends_on(&d.store.root, &self.root) {
                    return fail(format!(
                        "E5002 {} already depends on {mine}, so redirecting to {} would close a package loop: {}; change those uses first",
                        d.dir,
                        keep.qname(),
                        uses_list(&gone.name, &user_names)
                    ));
                }
                if !d.store.root.canonicalize().is_ok_and(|c| path_deps(&self.root).contains(&c)) {
                    let modules: BTreeSet<&str> = users.iter().map(|&j| ix.syms[j].module.as_str()).collect();
                    return fail(format!(
                        "E5009 {} use{} {}, and {mine} does not depend on {}: add {} first (`write`), then merge",
                        modules.iter().copied().collect::<Vec<_>>().join(", "),
                        if modules.len() == 1 { "s" } else { "" },
                        gone.qname(),
                        d.dir,
                        dependency_line(&mine, d)
                    ));
                }
            }
            Some(_) => {}
        }

        // projects that depend on this one: those the task covers are rewritten,
        // others that use the retired symbol are refused (as extract refuses them)
        let mut outside_users: Vec<String> = vec![];
        let mut deps: Vec<Dependent> = vec![];
        let gone_names: Vec<String> = std::iter::once(gone.name.clone()).chain(if gone.kind == Kind::Type { gleam::constructors(&gone.text) } else { vec![] }).collect();
        for d in members.iter().filter(|d| **d != mine && dest.as_ref().is_none_or(|t| t.dir != **d)) {
            let dir = scope::path(&top, d);
            let Some(me) = self.root.canonicalize().ok() else { break };
            if !path_deps(&dir).contains(&me) {
                continue;
            }
            if covered(&dir) {
                let store = Store::at(&dir)?;
                let ix = store.index()?;
                deps.push(Dependent { dir: d.clone(), store, ix });
                continue;
            }
            let Ok(other) = board::open(&dir) else { continue };
            let Ok(dix) = other.index() else { continue };
            for u in &dix.syms {
                let header = dix.modules.iter().find(|m| m.path == u.module).map_or("", |m| m.header.as_str());
                let imps = gleam::imports(header);
                let hit = gleam::references(&u.text).into_iter().any(|rf| match &rf {
                    Ref::Qualified(alias, member) => imps.iter().any(|i| &i.alias == alias && i.module == gone.module && gone_names.contains(member)),
                    Ref::Bare(b) => imps.iter().any(|i| i.module == gone.module && i.unqualified.iter().any(|(o, l)| l == b && gone_names.contains(o))),
                });
                if hit {
                    outside_users.push(format!("{d}: {} uses {}", u.qname(), gone.qname()));
                }
            }
        }
        if !outside_users.is_empty() {
            return fail(format!(
                "E5003 a project that depends on this one uses {}, and merge rewrites another project only when the task covers it: add the project to the task, or change those uses first\n{}",
                gone.qname(),
                outside_users.join("\n")
            ));
        }
        let joints: Vec<Index> = deps.iter().map(|dp| merged(&dp.ix, jx)).collect();
        for (dp, djx) in deps.iter().zip(&joints) {
            let k = dp.ix.syms.len();
            let users_there: Vec<usize> = djx.used_by[k + r].iter().copied().filter(|&j| j < k).collect();
            if users_there.is_empty() {
                continue;
            }
            if !is_pub_text(&keep.text) {
                let names: Vec<String> = users_there.iter().map(|&j| format!("{}: {}", dp.dir, djx.syms[j].qname())).collect();
                return fail(format!("E5009 {} is private and {} would use it: make it pub first, then merge", keep.qname(), names.join(", ")));
            }
            let Some(d) = &dest else { continue };
            if depends_on(&d.store.root, &dp.store.root) {
                let names: Vec<String> = users_there.iter().map(|&j| djx.syms[j].qname()).collect();
                return fail(format!(
                    "E5002 {} already depends on {}, so redirecting to {} would close a package loop: {}; change those uses first",
                    d.dir,
                    dp.dir,
                    keep.qname(),
                    uses_list(&gone.name, &names)
                ));
            }
            if !d.store.root.canonicalize().is_ok_and(|c| path_deps(&dp.store.root).contains(&c)) {
                let modules: BTreeSet<&str> = users_there.iter().map(|&j| djx.syms[j].module.as_str()).collect();
                return fail(format!(
                    "E5009 {} in {} use{} {}, and {} does not depend on {}: add {} first (`write --project {}`), then merge",
                    modules.iter().copied().collect::<Vec<_>>().join(", "),
                    dp.dir,
                    if modules.len() == 1 { "s" } else { "" },
                    gone.qname(),
                    dp.dir,
                    d.dir,
                    dependency_line(&dp.dir, d),
                    dp.dir
                ));
            }
        }

        // private helpers only the retired symbol used: named, not deleted
        let mut unused: Vec<usize> = vec![];
        let mut dropped = vec![false; n];
        dropped[r] = true;
        bring_helpers(&ix, &mut dropped, &mut unused);

        // the rewrite: every reference to the retired symbol goes to the survivor
        let mut plan = plan_for(jx, n, ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect(), r, s);
        plan.rewrite()?;
        let mut dplans: Vec<Plan> = vec![];
        for (dp, djx) in deps.iter().zip(&joints) {
            let k = dp.ix.syms.len();
            let mut dplan = plan_for(djx, k, dp.ix.modules.iter().map(|m| (m.path.clone(), m.header.clone())).collect(), k + r, k + s);
            dplan.rewrite()?;
            dplans.push(dplan);
        }

        let before = self.broken()?;
        let deps_broken = deps.iter().map(|dp| dp.store.broken()).collect::<Result<Vec<_>, _>>()?;
        let survivor_id = keep.id.clone();
        let body = || {
            for i in (0..n).filter(|&i| i != r) {
                let x = &ix.syms[i];
                if plan.newtext[i] != x.text {
                    self.db.execute("UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1", params![x.id, plan.newtext[i], revision(&plan.newtext[i])])?;
                    self.history(&x.id, "edit", &x.name)?;
                }
            }
            self.db.execute("UPDATE symbols SET live = 0 WHERE id = ?1", [&gone.id])?;
            self.history(&gone.id, "merged-into", &survivor_id)?;
            let headers: Vec<(String, String, &Store)> = ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), self)).collect();
            let finals = write_headers(&plan, &headers)?;
            for (dp, p) in deps.iter().zip(&dplans) {
                for (i, x) in dp.ix.syms.iter().enumerate() {
                    if p.newtext[i] != x.text {
                        dp.store.db.execute("UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1", params![x.id, p.newtext[i], revision(&p.newtext[i])])?;
                        dp.store.history(&x.id, "edit", &x.name)?;
                    }
                }
                let modules: Vec<(String, String, &Store)> = dp.ix.modules.iter().map(|m| (m.path.clone(), m.header.clone(), &dp.store)).collect();
                write_headers(p, &modules)?;
            }
            // the retired symbol's module goes with it when emptied, unless another module imports it
            let m = &gone.module;
            let empty = !(0..n).any(|i| i != r && ix.syms[i].module == *m);
            let imported = finals.iter().any(|(o, h)| o != m && gleam::imports(h).iter().any(|i| i.module == *m));
            let retired = empty && !imported;
            if retired {
                self.db.execute("DELETE FROM modules WHERE path = ?1", [m])?;
            }
            let mut bad: Vec<String> = vec![];
            let diags = self.check_all(&self.index()?)?;
            bad.extend(diags.iter().filter(|g| g.error && g.symbol.as_ref().is_none_or(|x| !before.contains_key(x))).map(|g| g.text.clone()));
            for (dp, broken) in deps.iter().zip(&deps_broken) {
                let diags = dp.store.check_all(&dp.store.index()?)?;
                bad.extend(diags.iter().filter(|g| g.error && g.symbol.as_ref().is_none_or(|x| !broken.contains_key(x))).map(|g| format!("{}: {}", dp.dir, g.text)));
            }
            if !bad.is_empty() {
                let more = if bad.len() > 5 { format!("\n\n… {} more", bad.len() - 5) } else { String::new() };
                return fail(format!("Not merged: the result does not type-check, so nothing changed.\n\n{}{more}", bad[..bad.len().min(5)].join("\n\n")));
            }
            Ok(if retired { format!("Retired module {m}\n") } else { String::new() })
        };
        let mut stores: Vec<&Store> = vec![self];
        stores.extend(deps.iter().map(|dp| &dp.store));
        let mut body = body;
        let retired = nested(&stores, &[], &mut body)?;

        // the reply: names and counts
        let plural = |k: usize| if k == 1 { "" } else { "s" };
        let mut out = format!("Merged {} into {}{}\n", gone.qname(), keep.qname(), at(&dest));
        let hits = plan.hits + dplans.iter().map(|p| p.hits).sum::<usize>();
        let modules = plan.hit_in.len() + dplans.iter().map(|p| p.hit_in.len()).sum::<usize>();
        let projects = usize::from(plan.hits > 0) + dplans.iter().filter(|p| p.hits > 0).count();
        if hits > 0 {
            let across = if projects > 1 { format!(" ({projects} projects)") } else { String::new() };
            out += &format!("Redirected {hits} reference{} in {modules} module{}{across}\n", plural(hits), plural(modules));
        }
        if !unused.is_empty() {
            out += &format!("Now unused: {}\n", unused.iter().map(|&i| ix.syms[i].qname()).collect::<Vec<_>>().join(", "));
        }
        out += &retired;
        Ok(lockdown::trim_output(&out, budget))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    const BOARD: &str = "pub type Task {\n  Task(id: Int, lane: Int)\n}\n\npub fn new_task(id: Int) -> Task {\n  Task(id, lane_of(id))\n}\n\nfn lane_of(id: Int) -> Int {\n  id % 3\n}\n\npub fn bump(t: Task) -> Task {\n  Task(..t, lane: t.lane + 1)\n}\n\npub fn lane(t: Task) -> Int {\n  case t {\n    Task(lane: l, ..) -> l\n  }\n}\n";

    fn text(s: &Store, name: &str) -> String {
        let ix = s.index().unwrap();
        ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
    }
    fn header(s: &Store, module: &str) -> String {
        s.index().unwrap().modules.iter().find(|m| m.path == module).map(|m| m.header.clone()).unwrap_or_default()
    }
    fn id(s: &Store, name: &str) -> String {
        let ix = s.index().unwrap();
        ix.syms[s.resolve(&ix, name).unwrap()].id.clone()
    }
    fn merge(s: &Store, args: &[&str]) -> Out {
        s.merge(args, usize::MAX)
    }
    /// I3: no live symbol uses the retired identity, which is no longer live, and
    /// its history links it to the survivor.
    fn retired_into(s: &Store, gone: &str, keep: &str) {
        let live: bool = s.db.query_row("SELECT live FROM symbols WHERE id = ?1", [gone], |r| r.get::<_, i64>(0)).unwrap() == 1;
        assert!(!live, "the retired symbol is not live");
        let ix = s.index().unwrap();
        assert!(ix.syms.iter().all(|x| x.id != gone));
        let k = ix.syms.iter().position(|x| x.id == keep).unwrap();
        assert!(!ix.used_by[k].is_empty(), "the survivor has the references");
        let link: (String, String) = s
            .db
            .query_row("SELECT action, detail FROM history WHERE symbol = ?1 ORDER BY rowid DESC LIMIT 1", [gone], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(link, ("merged-into".to_string(), keep.to_string()));
    }

    /// Two functions in different modules: callers, a test's unqualified import
    /// and a different name all go to the survivor; the retired symbol's private
    /// helper is named as now unused and kept.
    #[test]
    fn merges_two_functions_across_modules() {
        let store = project(&[
            ("src/a.gleam", "pub fn total(x: Int) -> Int {\n  x + 1\n}\n"),
            ("src/b.gleam", "pub fn sum(x: Int) -> Int {\n  x + one()\n}\n\nfn one() -> Int {\n  1\n}\n\npub fn keep() -> Int {\n  2\n}\n"),
            ("src/app.gleam", "import b\n\npub fn run() -> Int {\n  b.sum(2) + b.keep()\n}\n"),
            ("test/app_test.gleam", "import b.{sum}\n\npub fn sum_test() {\n  assert sum(1) == 2\n}\n"),
        ]);
        let (keep, gone) = (id(&store, "a.total"), id(&store, "b.sum"));
        let out = merge(&store, &["a.total", "b.sum"]).unwrap();
        assert_eq!(out, "Merged b.sum into a.total\nRedirected 2 references in 2 modules\nNow unused: b.one\n");
        assert_eq!(text(&store, "app.run"), "pub fn run() -> Int {\n  a.total(2) + b.keep()\n}");
        assert!(header(&store, "app").contains("import a") && header(&store, "app").contains("import b"), "{}", header(&store, "app"));
        assert!(text(&store, "app_test.sum_test").contains("assert a.total(1) == 2"), "{}", text(&store, "app_test.sum_test"));
        assert_eq!(header(&store, "app_test"), "import a");
        assert!(store.resolve(&store.index().unwrap(), "b.one").is_ok());
        retired_into(&store, &gone, &keep);
    }

    /// A type with its constructors: annotations, calls, patterns and record
    /// updates in its own module, and unqualified imports, `{Task}` alone too.
    #[test]
    fn merges_a_type_with_its_constructors() {
        let app = "import board.{type Task, Task}\n\npub fn mk() -> Task {\n  Task(1, 2)\n}\n";
        let only_ctor = "import board.{Task}\n\npub fn mk2(id: Int) {\n  Task(id, 1)\n}\n";
        let store = project(&[
            ("src/board.gleam", BOARD),
            ("src/task.gleam", "pub type Task {\n  Task(id: Int, lane: Int)\n}\n"),
            ("src/app.gleam", app),
            ("src/other.gleam", only_ctor),
        ]);
        let (keep, gone) = (id(&store, "task.Task"), id(&store, "board.Task"));
        let before = board::snapshot(&store).unwrap();
        let out = merge(&store, &["task.Task", "board.Task"]).unwrap();
        assert!(out.starts_with("Merged board.Task into task.Task\nRedirected "), "{out}");
        assert!(text(&store, "board.new_task").contains("-> task.Task {\n  task.Task(id, lane_of(id))"), "{}", text(&store, "board.new_task"));
        assert!(text(&store, "board.bump").contains("task.Task(..t, lane: t.lane + 1)"));
        assert!(text(&store, "board.lane").contains("task.Task(lane: l, ..) -> l"));
        assert_eq!(text(&store, "app.mk"), "pub fn mk() -> Task {\n  Task(1, 2)\n}");
        assert!(header(&store, "app").contains("import task.{type Task, Task}") && !header(&store, "app").contains("board"), "{}", header(&store, "app"));
        assert!(header(&store, "other").contains("import task.{Task}") && !header(&store, "other").contains("board"), "{}", header(&store, "other"));
        retired_into(&store, &gone, &keep);
        // the task's undo record covers the retired symbol: its before-state is live
        let after = board::snapshot(&store).unwrap();
        let key = format!("symbol:{gone}");
        assert!(before.get(&key).is_some() && after.get(&key).is_none());
    }

    /// A revert puts both symbols back: the retired one live again, the
    /// references as they were.
    #[test]
    fn a_revert_restores_both_symbols() {
        let store = project(&[("src/board.gleam", BOARD), ("src/task.gleam", "pub type Task {\n  Task(id: Int, lane: Int)\n}\n")]);
        let (keep, gone) = (id(&store, "task.Task"), id(&store, "board.Task"));
        let before = board::snapshot(&store).unwrap();
        let original = text(&store, "board.new_task");
        merge(&store, &["task.Task", "board.Task"]).unwrap();
        assert_ne!(text(&store, "board.new_task"), original);
        let after = board::snapshot(&store).unwrap();
        let states: Vec<(String, Value)> =
            before.as_object().unwrap().iter().filter(|(k, v)| after.get(k.as_str()) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
        assert!(states.iter().any(|(k, _)| *k == format!("symbol:{gone}")), "the retired symbol is in the before-state");
        board::restore(&store, &states).unwrap();
        assert_eq!(id(&store, "board.Task"), gone);
        assert_eq!(id(&store, "task.Task"), keep);
        assert_eq!(text(&store, "board.new_task"), original);
        assert_eq!(board::snapshot(&store).unwrap(), before);
    }

    /// A constructor named like the retired type is not renamed with the type.
    #[test]
    fn a_constructor_named_like_the_retired_type_keeps_its_name() {
        let store = project(&[
            ("src/board.gleam", "pub type Task {\n  Task(id: Int)\n}\n\npub fn mk(x: Int) -> Task {\n  Task(x)\n}\n"),
            ("src/card.gleam", "pub type Card {\n  Task(id: Int)\n}\n"),
        ]);
        merge(&store, &["card.Card", "board.Task"]).unwrap();
        assert_eq!(text(&store, "board.mk"), "pub fn mk(x: Int) -> card.Card {\n  card.Task(x)\n}");
    }

    /// A bare constructor named like the retired type keeps its name in values
    /// and patterns; the type is renamed in signatures, annotations and type arguments.
    #[test]
    fn a_bare_constructor_named_like_the_retired_type_keeps_its_name() {
        let board = "pub type Idle {\n  Idle\n}\n\npub fn rank(s: Idle) -> Int {\n  case s {\n    Idle -> 1\n  }\n}\n\npub fn all() -> List(Idle) {\n  let xs: List(Idle) = [Idle]\n  xs\n}\n";
        let store = project(&[("src/board.gleam", board), ("src/state.gleam", "pub type State {\n  Idle\n}\n")]);
        merge(&store, &["state.State", "board.Idle"]).unwrap();
        assert_eq!(text(&store, "board.rank"), "pub fn rank(s: state.State) -> Int {\n  case s {\n    state.Idle -> 1\n  }\n}");
        assert_eq!(text(&store, "board.all"), "pub fn all() -> List(state.State) {\n  let xs: List(state.State) = [state.Idle]\n  xs\n}");
    }

    /// A label (`state: Idle`) in a call, a record update or a pattern is a value,
    /// not an annotation, so the constructor keeps its name.
    #[test]
    fn a_label_before_a_bare_constructor_is_not_an_annotation() {
        let board = "pub type Idle {\n  Idle\n}\n\npub type Box {\n  Box(state: Idle)\n}\n\npub fn mk(b: Box) -> Box {\n  let c = Box(state: Idle)\n  let d = Box(..b, state: Idle)\n  c\n}\n";
        let store = project(&[("src/board.gleam", board), ("src/state.gleam", "pub type State {\n  Idle\n}\n")]);
        merge(&store, &["state.State", "board.Idle"]).unwrap();
        assert_eq!(
            text(&store, "board.mk"),
            "pub fn mk(b: Box) -> Box {\n  let c = Box(state: state.Idle)\n  let d = Box(..b, state: state.Idle)\n  c\n}"
        );
    }

    /// A bracket in a string before a call does not change what a label is; a
    /// label in a pattern is a value too.
    #[test]
    fn labels_ignore_strings_before_a_call_and_patterns() {
        let board = "pub type Idle {\n  Idle\n}\n\npub type Box {\n  Box(phase: Idle)\n}\n\npub fn mk(b: Box, dofn: Int) -> Int {\n  let s = \"(\"\n  let c = Box(phase: Idle)\n  case b {\n    Box(phase: Idle) -> dofn\n  }\n}\n";
        let store = project(&[("src/board.gleam", board), ("src/state.gleam", "pub type State {\n  Idle\n}\n")]);
        merge(&store, &["state.State", "board.Idle"]).unwrap();
        assert_eq!(
            text(&store, "board.mk"),
            "pub fn mk(b: Box, dofn: Int) -> Int {\n  let s = \"(\"\n  let c = Box(phase: state.Idle)\n  case b {\n    Box(phase: state.Idle) -> dofn\n  }\n}"
        );
    }

    /// A word ending in `fn` on the line before a record update is not the
    /// keyword, and a `)` in a string between a call's `(` and the reference
    /// does not close the call.
    #[test]
    fn labels_ignore_words_ending_in_fn_and_brackets_in_strings_on_the_scan_path() {
        let board = "pub type Idle {\n  Idle\n}\n\npub type Box {\n  Box(phase: Idle)\n}\n\npub fn f(s: String, i: Idle) -> Idle {\n  i\n}\n\npub fn mk(b: Box, dofn: Int) -> Box {\n  let d = dofn\n  let e = f(\")\", Idle)\n  Box(..b, phase: Idle)\n}\n";
        let store = project(&[("src/board.gleam", board), ("src/state.gleam", "pub type State {\n  Idle\n}\n")]);
        merge(&store, &["state.State", "board.Idle"]).unwrap();
        assert_eq!(
            text(&store, "board.mk"),
            "pub fn mk(b: Box, dofn: Int) -> Box {\n  let d = dofn\n  let e = f(\")\", state.Idle)\n  Box(..b, phase: state.Idle)\n}"
        );
    }

    #[test]
    fn a_different_kind_or_shape_is_refused() {
        let store = project(&[
            ("src/a.gleam", "pub type Thing {\n  Thing\n}\n\npub type Task {\n  Task(id: Int, column: Int)\n  Done\n}\n"),
            ("src/board.gleam", &format!("{BOARD}\npub fn thing() -> Int {{\n  1\n}}\n")),
        ]);
        let err = merge(&store, &["a.Thing", "board.thing"]).unwrap_err().0;
        assert!(err.starts_with("E5008 board.thing is a function or constant and a.Thing is a type"), "{err}");
        let err = merge(&store, &["a.Task", "board.Task"]).unwrap_err().0;
        assert!(err.starts_with("E5008 the constructors of a.Task and board.Task differ"), "{err}");
        assert!(err.contains("Task field 2: column in a.Task, lane in board.Task") && err.contains("Done only in a.Task"), "{err}");
        assert!(merge(&store, &["a.Task"]).unwrap_err().0.contains("merge takes two names"));
        assert!(merge(&store, &["a.nothing", "board.Task"]).unwrap_err().0.starts_with("E1001"));
        assert!(store.resolve(&store.index().unwrap(), "board.Task").is_ok(), "nothing changed");
    }

    const COPY: &str = "pub type Task {\n  Task(id: Int, lane: Int)\n}\n";

    /// A refused delete of a used symbol with a same-name copy names the merge.
    #[test]
    fn a_refused_delete_with_a_copy_names_merge() {
        let store = project(&[("src/a.gleam", COPY), ("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(err.starts_with("E5001 board.Task is still used by:"), "{err}");
        assert!(err.ends_with("\n\nHint: a.Task has the same name. To fold board.Task into it and redirect its 3 uses, use merge a.Task board.Task"), "{err}");
        assert_eq!(err.matches("Hint:").count(), 1);

        let store = project(&[("src/a.gleam", "pub type Task {\n  Task(id: Int, column: Int)\n}\n"), ("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(
            err.ends_with("Hint: a.Task has the same name, but their constructors differ: Task field 2: column in a.Task, lane in board.Task. Make them match, then use merge a.Task board.Task"),
            "{err}"
        );

        let store = project(&[("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(err.starts_with("E5001") && !err.contains("Hint:"), "{err}");
    }

    /// A copy in another project of the scope: the hint adds --to-project.
    #[test]
    fn a_refused_delete_with_a_copy_in_another_project() {
        let plain = |name: &str| format!("name = \"{name}\"\nversion = \"1.0.0\"\n");
        let root = two_projects(&plain("web"), &plain("components"));
        let store = Store::at(&root.join("web")).unwrap();
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(err.contains("Hint: solace_task.Task (components) has the same name."), "{err}");
        assert!(err.ends_with("use merge solace_task.Task board.Task --to-project components"), "{err}");
        // the board case: a submit in web mixing board.Task with solace_task.Task
        let root = two_projects(&format!("{}\n[dependencies]\ncomponents = {{ path = \"../components\" }}\n", plain("web")), &plain("components"));
        let store = Store::at(&root.join("web")).unwrap();
        let err = store.submit_symbols(Some("board.bump"), None, "import solace_task\n\npub fn bump(t: solace_task.Task) -> Task {\n  t\n}").unwrap_err().0;
        assert!(err.starts_with("Not saved"), "{err}");
        assert!(err.contains("Hint: solace_task.Task (components) has the same name.") && err.contains("--to-project components"), "{err}");
        assert!(!err.contains("Unknown module"), "{err}");
        assert_eq!(err.matches("Hint: solace_task.Task").count(), 1, "{err}");
    }

    /// Only types: a same-name function gets no hint; a private copy is named as private;
    /// several refused symbols give one hint.
    #[test]
    fn the_hint_is_for_public_types_once() {
        let store = project(&[("src/a.gleam", "pub fn lane(x: Int) -> Int {\n  x\n}\n"), ("src/board.gleam", BOARD), ("src/app.gleam", "import board\n\npub fn l(t: board.Task) -> Int {\n  board.lane(t)\n}\n")]);
        let err = store.delete("board.lane").unwrap_err().0;
        assert!(err.starts_with("E5001") && !err.contains("Hint:"), "{err}");

        let store = project(&[("src/a.gleam", "type Task {\n  Task(id: Int, lane: Int)\n}\n"), ("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(err.ends_with("Hint: a.Task has the same name but is private. Make it pub, then use merge a.Task board.Task"), "{err}");

        // private and differing: both fixes are named
        let store = project(&[("src/a.gleam", "type Task {\n  Task(id: Int, column: Int)\n}\n"), ("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(
            err.ends_with("Hint: a.Task has the same name but is private, and their constructors differ: Task field 2: column in a.Task, lane in board.Task. Make it pub and make them match, then use merge a.Task board.Task"),
            "{err}"
        );

        // several copies: the private one is marked
        let store = project(&[("src/a.gleam", COPY), ("src/c.gleam", "type Task {\n  Task(id: Int, lane: Int)\n}\n"), ("src/board.gleam", BOARD)]);
        let err = store.delete("board.Task").unwrap_err().0;
        assert!(err.contains("c.Task (private)") && !err.contains("a.Task (") && err.contains("have the same name as board.Task"), "{err}");

        let store = project(&[("src/a.gleam", COPY), ("src/board.gleam", BOARD), ("src/app.gleam", "import board\n\npub fn l(t: board.Task) -> Int {\n  board.lane(t)\n}\n")]);
        let err = store.delete_all(&["board.Task", "board.lane"], usize::MAX).unwrap_err().0;
        assert!(err.starts_with("E5001 nothing deleted."), "{err}");
        assert_eq!(err.matches("Hint:").count(), 1, "{err}");
    }

    /// A submit rejected for mixing two same-name types gets the hint; a plain type error doesn't.
    #[test]
    fn a_rejected_submit_mixing_same_name_types_names_merge() {
        let store = project(&[("src/a.gleam", COPY), ("src/board.gleam", BOARD)]);
        let err = store.submit_symbols(Some("board.bump"), None, "pub fn bump(t: a.Task) -> Task {\n  t\n}").unwrap_err().0;
        assert!(err.starts_with("Not saved"), "{err}");
        assert!(err.contains("Hint: a.Task has the same name. To fold board.Task into it"), "{err}");
        assert_eq!(err.matches("Hint:").count(), 1, "{err}");
        let err = store.submit_symbols(Some("board.bump"), None, "pub fn bump(t: Task) -> Task {\n  1\n}").unwrap_err().0;
        assert!(err.starts_with("Not saved") && !err.contains("Hint:"), "{err}");
    }

    /// Redirecting a use that would make two modules import each other is refused, naming the use.
    #[test]
    fn a_module_cycle_is_refused() {
        let store = project(&[
            ("src/task.gleam", "import board\n\npub fn one() -> Int {\n  board.base()\n}\n"),
            ("src/board.gleam", "pub fn base() -> Int {\n  1\n}\n\npub fn one() -> Int {\n  1\n}\n\npub fn b() -> Int {\n  one()\n}\n"),
        ]);
        let err = merge(&store, &["task.one", "board.one"]).unwrap_err().0;
        assert!(err.starts_with("E5002 redirecting board.one to task.one closes an import loop: one (by board.b)"), "{err}");
        assert_eq!(text(&store, "board.b"), "pub fn b() -> Int {\n  one()\n}");
    }

    /// Two functions that don't type-check as one change nothing.
    #[test]
    fn a_failed_type_check_leaves_the_store_unchanged() {
        let store = project(&[
            ("src/a.gleam", "pub fn total(x: Int) -> Int {\n  x + 1\n}\n"),
            ("src/b.gleam", "pub fn name(x: String) -> String {\n  x\n}\n"),
            ("src/app.gleam", "import b\n\npub fn run() -> String {\n  b.name(\"x\")\n}\n"),
        ]);
        let before = board::snapshot(&store).unwrap();
        let err = merge(&store, &["a.total", "b.name"]).unwrap_err().0;
        assert!(err.starts_with("Not merged: the result does not type-check, so nothing changed."), "{err}");
        assert_eq!(board::snapshot(&store).unwrap(), before);
        assert!(fs::read_to_string(store.root.join("src/app.gleam")).unwrap().contains("b.name(\"x\")"));
    }

    /// `web` and `components` under a root; components has the same Task as web's board.
    fn two_projects(web_toml: &str, components_toml: &str) -> Scratch {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", web_toml),
            ("web/src/board.gleam", BOARD),
            ("web/test/board_test.gleam", "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n"),
            ("components/gleam.toml", components_toml),
            ("components/src/solace_task.gleam", "pub type Task {\n  Task(id: Int, lane: Int)\n}\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        root
    }

    /// Into another project: a missing dependency and a package loop are refused;
    /// with the dependency the uses go to the survivor there.
    #[test]
    fn a_survivor_in_another_project() {
        let plain = |name: &str| format!("name = \"{name}\"\nversion = \"1.0.0\"\n");
        let args = ["solace_task.Task", "board.Task", "--to-project", "components"];
        let root = two_projects(&plain("web"), &plain("components"));
        let store = Store::at(&root.join("web")).unwrap();
        let err = merge(&store, &args).unwrap_err().0;
        assert!(err.starts_with("E5009 board, board_test use board.Task, and web does not depend on components: add components = { path = \"../components\" } to web/gleam.toml"), "{err}");
        let err = merge(&store, &args[..2]).unwrap_err().0;
        assert!(err.contains("it is in components: use merge solace_task.Task board.Task --to-project components"), "{err}");

        let root = two_projects(&plain("web"), &format!("{}\n[dependencies]\nweb = {{ path = \"../web\" }}\n", plain("components")));
        let store = Store::at(&root.join("web")).unwrap();
        let err = merge(&store, &args).unwrap_err().0;
        assert!(err.starts_with("E5002 components already depends on web") && err.contains("Task (by board.new_task)"), "{err}");

        let root = two_projects(&format!("{}\n[dependencies]\ncomponents = {{ path = \"../components\" }}\n", plain("web")), &plain("components"));
        let store = Store::at(&root.join("web")).unwrap();
        let gone = id(&store, "board.Task");
        let out = merge(&store, &args).unwrap();
        assert!(out.starts_with("Merged board.Task into solace_task.Task (components)\nRedirected "), "{out}");
        assert!(text(&store, "board.new_task").contains("-> solace_task.Task {"), "{}", text(&store, "board.new_task"));
        assert!(text(&store, "board_test.round_trip_test").contains("solace_task.Task(1, 2)"));
        assert!(header(&store, "board_test").contains("import solace_task") && !header(&store, "board_test").contains("import board"));
        assert!(store.index().unwrap().syms.iter().all(|x| x.id != gone));
    }

    /// A private survivor is refused when a dependent the task covers uses the
    /// retired symbol, as it is for this project's users: one line, nothing changed.
    #[test]
    fn a_private_survivor_used_by_a_covered_dependent_is_refused() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/old.gleam", "pub type Task {\n  Task(id: Int)\n}\n"),
            ("web/src/board.gleam", "type Card {\n  Task(id: Int)\n}\n"),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import old\n\npub fn view(t: old.Task) -> Int {\n  t.id\n}\n"),
        ]);
        for d in ["web", "front"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let front = Store::at(&root.join("front")).unwrap();
        COMMAND_DB.with_borrow_mut(|c| c.push((front.root.clone(), front.db.clone())));
        let err = merge(&store, &["board.Card", "old.Task"]);
        COMMAND_DB.set(vec![]);
        let err = err.unwrap_err().0;
        assert!(err.starts_with("E5009 board.Card is private and front: front.view would use it: make it pub first"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "old.Task").is_ok(), "nothing changed");
    }
}
