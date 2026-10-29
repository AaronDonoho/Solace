//! Reading the store: session, view, inspect, refs, impact and find.

use crate::Kind;

use std::collections::HashSet;
use std::fs;
use rusqlite::OptionalExtension;
impl crate::Store {
    pub(crate) fn find(&self, query: &str) -> crate::failure::Out {
        let ix = self.index()?;
        let query = unquoted(query);
        let q = query.to_lowercase();
        let mut hits: Vec<(bool, String)> = vec![];
        for s in &ix.syms {
            let lines: Vec<String> = s
                .text
                .lines()
                .enumerate()
                .filter(|(_, l)| l.to_lowercase().contains(&q))
                .map(|(n, l)| format!("  {:>3}: {}", n + 1, l.trim()))
                .collect();
            let name_hit = s.name.to_lowercase().contains(&q);
            if name_hit || !lines.is_empty() {
                let more = if lines.len() > 4 { format!("\n  … {} more lines", lines.len() - 4) } else { String::new() };
                let shown: Vec<String> = lines.into_iter().take(4).collect();
                hits.push((name_hit, format!("{} ({})\n{}{}", s.qname(), s.kind_text, shown.join("\n"), more)));
            }
        }
        if hits.is_empty() {
            return Ok(format!("No matches for \"{query}\".\n"));
        }
        hits.sort_by_key(|h| !h.0);
        let total = hits.len();
        let mut out: String = hits.iter().take(40).map(|h| h.1.trim_end().to_string() + "\n").collect();
        if total > 40 {
            out += &format!("… {} more symbols match; narrow the search.\n", total - 40);
        }
        Ok(out)
    }

}
/// A search query without the quotes, backticks or apostrophes around it
/// (`find "Customer(.."`: the quotes delimit the query, run 18).
fn unquoted(query: &str) -> &str {
    ['"', '\'', '`'].iter().find_map(|&c| query.strip_prefix(c)?.strip_suffix(c)).filter(|q| !q.is_empty()).unwrap_or(query)
}

impl crate::Store {
    /// The symbol index (one line per source symbol), test modules, and open
    /// work records expanded with the context of the symbols they name.
    pub(crate) fn session(&self, scopes: &[&str]) -> crate::failure::Out {
        const CAP: usize = 80;
        let tid = crate::board::context().map(|(t, _)| t);
        let rust = crate::language::is_rust(&self.root);
        if self.lang.is_none() && !rust {
            return self.plain_session("", tid.as_deref());
        }
        let ix = self.index()?;
        let in_scope =
            |m: &crate::ModuleRow| scopes.is_empty() || scopes.iter().any(|s| self.resolve_module(&ix, s).is_some_and(|r| r.path == m.path));
        let mut out = self.project_instructions();
        out += &String::from(
            "Symbols per module (types are capitalized). `inspect <name>...` gives signatures and context; \
             `view <module>` an outline.\n",
        );
        let mut tests = vec![];
        for m in ix.modules.iter().filter(|m| in_scope(m)) {
            let syms: Vec<&crate::Sym> = ix.syms.iter().filter(|s| s.module == m.path).collect();
            if crate::store::is_test(m) {
                tests.push(format!("{} ({})", m.path, syms.len()));
                continue;
            }
            // Names only: this is re-sent on every turn, so it must stay small (D-12).
            let names = crate::store::module_names(&ix, &m.path);
            let more = if names.len() > CAP { format!(", …{} more", names.len() - CAP) } else { String::new() };
            out += &format!("{} — {}\n  {}{more}\n", m.path, crate::store::module_doc(m), names[..names.len().min(CAP)].join(", "));
        }
        out += &format!("\nTest modules (tests): {}\n", tests.join(", "));
        if rust {
            out += &format!("\n{}", crate::store::rust_brief(&ix));
        }
        out += &self.session_end(&ix, crate::cli::EXAMPLE_CALL, tid.as_deref())?;
        Ok(out)
    }
    /// Session start in a project of plain files (D-60): no symbols to list.
    /// `deps` is the task's unfinished dependencies, for `--task` in a board
    /// run of task `tid`.
    fn plain_session(&self, deps: &str, tid: Option<&str>) -> crate::failure::Out {
        let mut out = self.project_instructions();
        out += deps;
        out += "This project has plain files and no code held as symbols of its own: `files` lists them, `read` and `write` change them, \
                and `run` runs the commands its solace.toml declares.\n";
        out += &self.session_end(&self.index()?, "for example `grep <text> --in <path>` and `read <path> --lines a..b`", tid)?;
        Ok(out)
    }
    /// The end of every session start: status, open work and its context,
    /// the other projects, the command reference, and last `call`, a first
    /// call to send, where the agent plans its reads (D-145). `tid` is the
    /// board run's task; the open work records are capped (D-151).
    fn session_end(&self, ix: &crate::Index, call: &str, tid: Option<&str>) -> crate::failure::Out {
        let next = crate::reference::next_call(call, crate::board::strategy_context().is_some());
        let (status, work) = self.status_within(crate::cli::WORK_CAP)?;
        Ok(format!("{status}{}{}\n{}{next}", self.work_context(ix, &work)?, self.other_projects(tid), crate::reference::agent_reference(tid.is_some())))
    }
    /// The symbols `text` names as `module.name`, in order, each once.
    pub(crate) fn named(&self, ix: &crate::Index, text: &str) -> Vec<usize> {
        let mut named = vec![];
        for word in text.split(|c: char| !(c.is_alphanumeric() || "_./".contains(c))) {
            let word = word.trim_matches(|c| c == '.' || c == '/');
            if word.contains('.')
                && let Ok(i) = self.resolve(ix, word)
                && !named.contains(&i)
            {
                named.push(i);
            }
        }
        named
    }
    /// Where the agent is, and the other projects under this project's Solace
    /// root, which reads reach with `--project` (D-57): in a task run, those
    /// its task may change apart from those it only reads, and the rules for
    /// `--project` and paths (D-123); `tid` is the board run's task.
    pub(crate) fn other_projects(&self, tid: Option<&str>) -> String {
        let Ok(Some(root)) = crate::workspace::enclosing(&self.root) else { return String::new() };
        let Ok(members) = crate::workspace::members(&root) else { return String::new() };
        let me = crate::scope::dir(&root, &self.root);
        let label = |m: &str| match crate::workspace::name(&crate::scope::path(&root, m)) {
            name if name.is_empty() && m == "." => ". (the root)".to_string(),
            name if name.is_empty() => m.to_string(),
            name => format!("{m} ({name})"),
        };
        // The root itself last: it is rarely where code lives.
        let mut others: Vec<&String> = members.iter().filter(|m| **m != me).collect();
        others.sort_by_key(|m| *m == ".");
        if others.is_empty() {
            return String::new();
        }
        let labels = |ms: Vec<&String>| ms.into_iter().map(|m| label(m)).collect::<Vec<_>>().join(", ");
        let task = tid.and_then(|t| crate::scope::of_task(&self.root, t));
        let mut out = format!("\nYou are in {}.\n", label(&me));
        match &task {
            Some(scoped) => {
                out += &format!("This task can change: {}.\n", labels(scoped.iter().collect()));
                let read: Vec<&String> = others.iter().copied().filter(|m| !scoped.contains(m)).collect();
                if !read.is_empty() {
                    out += &format!("Read only: {}.\n", labels(read));
                }
            }
            None => out += &format!("Other projects under this Solace root (read only): {}.\n", labels(others.clone())),
        }
        let example = task.as_ref().and_then(|s| others.iter().copied().find(|m| s.contains(m))).unwrap_or(others[0]);
        out += &format!(
            "`--project <folder>` runs one line in another project: its folder as listed, from the Solace root \
             (`--project {example}`), for that line only. Reads (view, inspect, refs, find, grep, files, read) \
             reach every project listed{}.\n\
             Paths in `read`, `grep --in`, `files`, `write` and `move` are relative to the project the line runs in; \
             `read`, `grep --in` and `files` also take a path from the Solace root into another project.\n",
            if task.is_some() { "; other commands, only this task's" } else { "" }
        );
        if task.is_some() {
            out += "If the task needs a change in a read-only project (or the root, `.`), ask the human with `task ask` \
                    to add that project, then make the change yourself; never put the change somewhere else instead.\n";
        }
        out
    }
    /// The project's AGENTS.md, which its humans write for agents (D-45), cut
    /// at a whole line past 8,000 bytes; empty when there is none.
    fn project_instructions(&self) -> String {
        const CAP: usize = 8000;
        let text = fs::read_to_string(self.root.join("AGENTS.md")).unwrap_or_default();
        if text.trim().is_empty() {
            return String::new();
        }
        let mut shown = String::new();
        let mut lines = text.lines().enumerate();
        for (n, line) in lines.by_ref() {
            if shown.len() + line.len() + 1 > CAP {
                shown += &format!("… `read AGENTS.md --lines {}..` for the rest\n", n + 1);
                break;
            }
            shown += line;
            shown += "\n";
        }
        format!(
            "Project instructions (AGENTS.md, from the project's maintainers; where they name files or shell commands, \
             use the matching Solace command):\n{shown}\n"
        )
    }
    /// The open work records `work` expanded with the context of the symbols they name.
    fn work_context(&self, ix: &crate::Index, work: &[String]) -> crate::failure::Out {
        let mut out = String::new();
        for text in work {
            let named = self.named(ix, text);
            if !named.is_empty() {
                out += "\n== Context of the symbols the work record names ==\n";
                let (full, mut shown) = (named.iter().copied().collect(), HashSet::new());
                for &i in &named {
                    out += &self.bundle(ix, i, true, 0, &full, &mut shown)?;
                    out += "\n";
                }
            }
        }
        Ok(out)
    }
    /// `session start --task`: instead of the full index, the symbols most
    /// likely relevant to the task (signature and why each was picked),
    /// within a fixed budget, plus the module list. In a board run it also
    /// names the task's feature (D-138) and unfinished dependencies (D-110).
    pub(crate) fn session_task(&self, task: &str) -> crate::failure::Out {
        let tid = crate::board::context().map(|(tid, _)| tid);
        let plan = tid.as_deref().map(|tid| crate::board::plan_reading(&crate::scope::top(&self.root), tid)).unwrap_or_default();
        self.task_session(task, tid.as_deref(), &plan)
    }
    /// `session start --task` for board run task `tid`, with `deps` before the
    /// symbols; the board builds a fresh run's brief with it (D-151), outside
    /// the run's environment.
    pub(crate) fn task_session(&self, task: &str, tid: Option<&str>, deps: &str) -> crate::failure::Out {
        const BUDGET: usize = 6500; // bytes, about 1.6k tokens
        let rust = crate::language::is_rust(&self.root);
        // The task's other projects with code, by folder: the list of symbols
        // draws on them too (D-123).
        let top = crate::scope::top(&self.root);
        let mine = crate::scope::dir(&top, &self.root);
        let others: Vec<(String, crate::Store, crate::Index)> = tid
            .and_then(|t| crate::scope::of_task(&self.root, t))
            .unwrap_or_default()
            .into_iter()
            .filter(|d| {
                let path = crate::scope::path(&top, d);
                *d != mine && (crate::language::detect(&path).is_some() || crate::language::is_rust(&path))
            })
            .filter_map(|d| {
                let s = crate::Store::peek(&crate::scope::path(&top, &d)).ok()?;
                let ix = s.index().ok()?;
                Some((d, s, ix))
            })
            .collect();
        if self.lang.is_none() && !rust && others.is_empty() {
            return self.plain_session(deps, tid);
        }
        let ix = self.index()?;
        let projects: Vec<(Option<&str>, &crate::Store, &crate::Index)> =
            std::iter::once((None, self, &ix)).chain(others.iter().map(|(d, s, x)| (Some(d.as_str()), s, x))).collect();
        let mut modules = String::from("Modules (`view <module>` for an outline):\n");
        let grouped = !others.is_empty();
        for (p, (folder, _, pix)) in projects.iter().enumerate() {
            if grouped && pix.modules.is_empty() {
                continue;
            }
            if grouped {
                modules += &format!("{}:\n", if p == 0 { mine.as_str() } else { folder.unwrap_or(mine.as_str()) });
            }
            let mut tests = vec![];
            for m in &pix.modules {
                if crate::store::is_test(m) {
                    tests.push(m.path.as_str());
                } else {
                    modules += &format!("  {} — {}\n", m.path, crate::store::module_doc(m));
                }
            }
            modules += &format!("Test modules: {}\n", tests.join(", "));
        }
        let rust_ix = if rust { Some(&ix) } else { others.iter().find(|(_, s, _)| crate::language::is_rust(&s.root)).map(|(_, _, x)| x) };
        if let Some(rust_ix) = rust_ix {
            modules += &format!("\n{}", crate::store::rust_brief(rust_ix));
        }

        // Direct matches: rare task words weigh more; a word in the name
        // counts 3, in the doc line 2, anywhere else in the code 1. Words are
        // counted over all the task's projects.
        let fields: Vec<Vec<[HashSet<String>; 3]>> = projects
            .iter()
            .map(|(_, _, ix)| {
                ix.syms
                    .iter()
                    .map(|s| {
                        let doc: String = s.text.lines().filter(|l| l.trim_start().starts_with("///")).collect::<Vec<_>>().join(" ");
                        [crate::words(&s.name).into_iter().collect(), crate::words(&doc).into_iter().collect(), crate::words(&s.text).into_iter().collect()]
                    })
                    .collect()
            })
            .collect();
        let n: usize = fields.iter().map(Vec::len).sum();
        let mut terms = crate::words(task);
        terms.sort();
        terms.dedup();
        let mut scores: Vec<Vec<f64>> = fields.iter().map(|f| vec![0.0f64; f.len()]).collect();
        let mut rare: Vec<(f64, String)> = vec![];
        for t in &terms {
            let df = fields.iter().flatten().filter(|f| f.iter().any(|w| w.contains(t))).count();
            if df == 0 {
                continue;
            }
            // squared, so one rare word (a code, a name) outweighs several common ones
            let idf = (1.0 + n as f64 / df as f64).ln().powi(2);
            rare.push((idf, t.clone()));
            for (score, fields) in scores.iter_mut().zip(&fields) {
                for (i, f) in fields.iter().enumerate() {
                    if let Some(k) = f.iter().position(|w| w.contains(t)) {
                        score[i] += idf * [3.0, 2.0, 1.0][k];
                    }
                }
            }
        }

        // One hop from each project's best matches: callers, callees and
        // covering tests; then the non-test symbols (fixtures) the top
        // covering tests use. Symbols the task names (`module.name`) come first.
        for (&(_, store, ix), score) in projects.iter().zip(scores.iter_mut()) {
            let direct = score.clone();
            let mut seeds: Vec<usize> = (0..direct.len()).filter(|&i| direct[i] > 0.0).collect();
            seeds.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]));
            seeds.truncate(10);
            let boost = |j: usize, s: f64, score: &mut Vec<f64>| {
                if direct[j] + s > score[j] {
                    score[j] = direct[j] + s;
                }
            };
            for &s in &seeds {
                for &j in &ix.uses[s] {
                    boost(j, direct[s] / 4.0, &mut *score);
                }
                let mut covering = vec![];
                for &j in &ix.used_by[s] {
                    if crate::is_test(ix, j) {
                        covering.push(j);
                    }
                    boost(j, direct[s] / 4.0, &mut *score);
                }
                covering.sort_by(|&a, &b| direct[b].total_cmp(&direct[a]).then(ix.syms[a].text.len().cmp(&ix.syms[b].text.len())));
                for &t in covering.iter().take(2) {
                    for &j in ix.uses[t].iter().filter(|&&j| !crate::is_test(ix, j)) {
                        boost(j, direct[s] / 8.0, &mut *score);
                    }
                }
            }
            for i in store.named(ix, task) {
                score[i] = f64::MAX;
            }
        }
        // Each entry is (project, symbol), the current project's first.
        let mut ranked: Vec<(usize, usize)> =
            scores.iter().enumerate().flat_map(|(p, s)| (0..s.len()).filter(move |&i| s[i] > 0.0).map(move |i| (p, i))).collect();
        let size = |(p, i): (usize, usize)| projects[p].2.syms[i].text.len();
        ranked.sort_by(|&a, &b| scores[b.0][b.1].total_cmp(&scores[a.0][a.1]).then(size(a).cmp(&size(b))));
        let score = &scores[0];

        // The task's own terms in string literals are evidence (D-44): its rarest
        // words, quoted phrases and capitalized names such as "Desk Lamp".
        rare.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut evidence_terms: Vec<String> = rare.into_iter().take(5).map(|r| r.1).collect();
        evidence_terms.extend(task.split('"').skip(1).step_by(2).filter(|p| (3..=40).contains(&p.len())).map(str::to_lowercase));
        evidence_terms.extend(capitalized_runs(task));
        // A test that reaches the top code symbols reproduces the task's case.
        let code: Vec<usize> = ranked
            .iter()
            .filter(|&&(p, _)| p == 0)
            .map(|&(_, i)| i)
            .filter(|&i| !crate::is_test(&ix, i) && ix.syms[i].kind == Kind::Fn && !ix.syms[i].text.contains("@external"))
            .take(2)
            .collect();
        let name = |p: usize, i: usize| {
            let (_, store, pix) = projects[p];
            let short = crate::reach::short(pix, i);
            if store.resolve(pix, &short).ok() == Some(i) { short } else { pix.syms[i].qname() }
        };
        let repro = if rust {
            String::new()
        } else {
            code.first()
                .and_then(|_| {
                    crate::reach::test_symbols(&ix).into_iter().map(|t| (t, crate::reach::closure(&ix, t))).filter(|(_, r)| r.contains(&code[0])).max_by(
                        |a, b| {
                            let reached = |r: &HashSet<usize>| code.iter().filter(|c| r.contains(c)).count();
                            reached(&a.1)
                                .cmp(&reached(&b.1))
                                .then(score[a.0].total_cmp(&score[b.0]))
                                .then(ix.syms[b.0].text.len().cmp(&ix.syms[a.0].text.len()))
                        },
                    )
                })
                .map(|(t, r)| {
                    let traced: Vec<String> = code.iter().filter(|c| r.contains(c)).map(|&c| name(0, c)).collect();
                    format!(
                        "To reproduce the task's case: `probe --from {} --trace {}`, with statements for the case in `text`.\n",
                        name(0, t),
                        traced.join(" ")
                    )
                })
                .unwrap_or_default()
        };
        let mut packet = String::new();
        let mut room = BUDGET.saturating_sub(modules.len() + repro.len());
        for &(p, i) in &ranked {
            // whole entries only: skip one that doesn't fit, try smaller ones
            let (folder, _, pix) = projects[p];
            let s = &pix.syms[i];
            let at = folder.map(|d| format!(" ({d})")).unwrap_or_default();
            let entry = format!("-- {}{at}\n{}\n", s.qname(), crate::index::signature(pix, s));
            let mut quoted = String::new();
            for e in evidence(&s.text, &evidence_terms) {
                let line = format!("  > {e}\n");
                if quoted.len() + line.len() <= 300 {
                    quoted += &line;
                }
            }
            for entry in [entry.clone() + &quoted, entry] {
                if entry.len() <= room {
                    room -= entry.len();
                    packet += &entry;
                    break;
                }
            }
        }
        let mut out = self.project_instructions();
        out += deps;
        let marked = if others.is_empty() { "" } else { "; one marked `(<folder>)` is in that project: add `--project <folder>`" };
        out += &format!(
            "Symbols most likely relevant to the task, best first (signatures; `>` lines quote code that names the task's terms; \
             `inspect` for code{marked}):\n{packet}{repro}\n{modules}"
        );
        let start: Vec<usize> = ranked.iter().filter(|&&(p, _)| p == 0).map(|&(_, i)| i).take(3).collect();
        let call = if !start.is_empty() {
            let names: Vec<String> = start.iter().map(|&i| name(0, i)).collect();
            format!("`inspect {} --context`", names.join(" "))
        } else if let Some(&(p, _)) = ranked.first() {
            let names: Vec<String> = ranked.iter().filter(|&&(q, _)| q == p).map(|&(_, i)| name(p, i)).take(3).collect();
            format!("`inspect {} --project {} --context`", names.join(" "), projects[p].0.unwrap_or(mine.as_str()))
        } else {
            crate::cli::EXAMPLE_CALL.to_string()
        };
        out += &self.session_end(&ix, &call, tid)?;
        Ok(out)
    }
    /// Open work records and broken symbols.
    pub(crate) fn status(&self) -> Result<String, crate::failure::Failure> {
        Ok(self.status_within(usize::MAX)?.0)
    }
    /// `status` with the open work records cut at a whole record within `cap`
    /// bytes, saying how many were left out (D-14), and the texts of those shown.
    fn status_within(&self, cap: usize) -> Result<(String, Vec<String>), crate::failure::Failure> {
        let mut out = String::new();
        let work = self.open_work()?;
        if !work.is_empty() {
            out += "\nOpen work records:\n";
        }
        let mut shown = vec![];
        for (id, text) in &work {
            let entry = format!("#{id}\n{}\n", text.trim_end());
            if out.len() + entry.len() > cap {
                break;
            }
            out += &entry;
            shown.push(text.clone());
        }
        if shown.len() < work.len() {
            out += &format!(
                "{} of {} open work records left out to fit the reply's size limit; `work list` shows them all.\n",
                work.len() - shown.len(),
                work.len()
            );
        }
        let broken: Vec<String> =
            self.db.prepare("SELECT symbol FROM broken ORDER BY symbol")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        if !broken.is_empty() {
            out += &format!("\nBroken symbols: {}\n", broken.join(", "));
        }
        Ok((out, shown))
    }
    /// A module is shown as an outline unless `bodies` is set. Past `budget`,
    /// module bodies fall back to outlines, and what still doesn't fit is
    /// listed to view separately (D-14).
    pub(crate) fn view(&self, names: &[&str], bodies: bool, budget: usize) -> crate::failure::Out {
        self.view_with_header(names, bodies, false, budget)
    }
    /// Several names give one document: each signature is printed once, and
    /// symbols asked for are printed in full only once (D-10, D-14). Over
    /// `budget`, the bundles are trimmed in priority order (callers, then the
    /// test example, then the signatures of what each uses); if that is not
    /// enough, the names that don't fit are listed to inspect separately.
    pub(crate) fn inspect(&self, names: &[&str], context: bool, budget: usize) -> crate::failure::Out {
        let ix = self.index()?;
        let (mut targets, mut unknown): (Vec<(&str, Option<usize>)>, Vec<String>) = (vec![], vec![]);
        for &name in names {
            match self.resolve(&ix, name) {
                Ok(i) => targets.push((name, Some(i))),
                // a module name gets its outline rather than an error
                Err(_) if self.resolve_module(&ix, name).is_some() => targets.push((name, None)),
                // so does a dependency's module or symbol, read from build/packages
                Err(_) if self.dependency(name).is_some() => targets.push((name, None)),
                // one bad name doesn't cost the others (run 11)
                Err(e) => unknown.push(e.0),
            }
        }
        if targets.is_empty() {
            return crate::failure::fail(unknown.join("\n"));
        }
        let unknown = if unknown.is_empty() { String::new() } else { unknown.join("\n") + "\n" };
        let full: HashSet<usize> = targets.iter().filter_map(|t| t.1).collect();
        const LAST: u8 = 3;
        for level in 0..=LAST {
            if level > 0 && !context {
                break;
            }
            // signatures an earlier command in this call printed count as shown
            let earlier: HashSet<usize> = (0..ix.syms.len()).filter(|&j| shown_earlier(&ix.syms[j].qname())).collect();
            let (mut out, mut shown, mut left) = (String::new(), earlier, vec![]);
            for &(name, i) in &targets {
                let before = shown.clone();
                let part = match i {
                    Some(i) => self.bundle(&ix, i, context, level, &full, &mut shown)?,
                    None => self.view(&[name], false, budget)?,
                } + "\n";
                let last = level == LAST || !context;
                if last && (!left.is_empty() || (!out.is_empty() && out.len() + part.len() > budget)) {
                    shown = before;
                    left.push(ix.syms.get(i.unwrap_or(usize::MAX)).map_or(name.to_string(), |s| s.qname()));
                    continue;
                }
                out += &part;
            }
            if out.len() > budget && level < LAST && context {
                continue;
            }
            let trimmed = [
                "",
                "callers as a count only (`solace refs <name>` lists them)",
                "tests by name only",
                "what each uses by name only (`view <name>` for its source)",
            ];
            if level > 0 {
                out += &format!("Trimmed to fit the size limit: {}.\n", trimmed[1..=level as usize].join("; "));
            }
            for &j in shown.iter().chain(full.iter().filter(|&&j| !left.contains(&ix.syms[j].qname()))) {
                mark_shown(ix.syms[j].qname());
            }
            for &j in full.iter().filter(|&&j| !left.contains(&ix.syms[j].qname())) {
                crate::reads::mark_served(&self.root, &ix.syms[j]);
            }
            if !left.is_empty() {
                out += &format!(
                    "Not shown, to fit the size limit: `solace inspect {}{}`\n",
                    left.join(" "),
                    if context { " --context" } else { "" }
                );
            }
            return Ok(out + &unknown);
        }
        unreachable!()
    }
    /// `level` is how far the context is trimmed (see `inspect`); `full` holds
    /// the symbols printed in full in this reply, `shown` the signatures
    /// printed so far.
    fn bundle(&self, ix: &crate::Index, i: usize, context: bool, level: u8, full: &HashSet<usize>, shown: &mut HashSet<usize>) -> crate::failure::Out {
        let s = &ix.syms[i];
        let mut out = format!("// ── {} ({}) ──\n{}\n", s.qname(), s.kind_text, s.text);
        if !context {
            return Ok(out);
        }
        let mut entry = |u: usize| {
            let q = ix.syms[u].qname();
            if full.contains(&u) {
                format!("-- {q} (in full in this reply)\n")
            } else if !shown.insert(u) {
                format!("-- {q} (above)\n")
            } else {
                format!("-- {q}\n{}\n", crate::index::signature(ix, &ix.syms[u]))
            }
        };
        let used_by = ix.used_by[i].clone();
        let (tests, callers): (Vec<usize>, Vec<usize>) = used_by.into_iter().partition(|&j| crate::is_test(ix, j));
        if !ix.uses[i].is_empty() {
            let mut uses = ix.uses[i].clone();
            uses.sort_by_key(|&u| (ix.syms[u].kind != Kind::Type, ix.syms[u].kind != Kind::Const, ix.syms[u].qname()));
            if level >= 3 {
                let names: Vec<String> = uses.iter().map(|&u| ix.syms[u].qname()).collect();
                out += &format!("\n== Uses ==\n  {}\n", names.join(", "));
            } else {
                out += "\n== Uses ==\n";
                for u in uses {
                    out += &entry(u);
                }
            }
        }
        if !callers.is_empty() && level >= 1 {
            out += &format!("\n== Used by ({}): solace refs {} ==\n", callers.len(), s.qname());
        } else if s.kind == Kind::Type {
            // the sites a new field breaks, then the other users by name (D-46)
            out += &type_users(ix, i, &callers, &tests, 20);
        } else if !callers.is_empty() {
            out += &format!("\n== Used by ({}) ==\n", callers.len());
            for &c in callers.iter().take(20) {
                out += &entry(c);
                out += &call_lines(ix, c, i);
            }
            if callers.len() > 20 {
                out += &format!("…and {} more (solace refs {})\n", callers.len() - 20, s.qname());
            }
        }
        if !tests.is_empty() {
            // The smallest test in full, as a worked example (saves a turn of
            // browsing test modules); the rest by name.
            const EXAMPLE_CAP: usize = 1600; // bytes, about 400 tokens
            let example = tests
                .iter()
                .copied()
                .filter(|t| level < 2 && !full.contains(t))
                .min_by_key(|&t| ix.syms[t].text.len())
                .filter(|&t| ix.syms[t].text.len() <= EXAMPLE_CAP);
            out += &format!("\n== Tests that use it ({}) ==\n", tests.len());
            if let Some(t) = example {
                out += &format!("-- {} (example)\n{}\n", ix.syms[t].qname(), ix.syms[t].text);
                // the test helpers it uses, so a new test can call them without guessing
                for &h in ix.uses[t].iter().filter(|&&h| crate::is_test(ix, h) && !crate::reach::is_test_fn(ix, h)) {
                    out += &entry(h);
                }
            }
            let names: Vec<String> = tests.iter().filter(|&&t| Some(t) != example).map(|&t| ix.syms[t].qname()).collect();
            if !names.is_empty() {
                out += &format!("  {}\n", names.join("\n  "));
            }
        }
        if let Some(diag) =
            self.db.query_row("SELECT diagnostic FROM broken WHERE symbol = ?1", [s.qname()], |r| r.get::<_, String>(0)).optional()?
        {
            out += &format!("\n== Broken ==\n{diag}\n");
        }
        Ok(out)
    }
    /// `refs <name>...`: each symbol's users, in the order named (D-102).
    pub(crate) fn refs_all(&self, names: &[&str], budget: usize) -> crate::failure::Out {
        let mut out = String::new();
        for name in names {
            out += &self.refs(name)?;
        }
        Ok(crate::lockdown::trim_output(&out, budget))
    }
    pub(crate) fn refs(&self, name: &str) -> crate::failure::Out {
        let ix = self.index()?;
        // `refs <module>.<Type>.<field>`: the readers of one field (D-44)
        if let Some((ty, field)) = name.rsplit_once('.')
            && field.starts_with(|c: char| c.is_lowercase())
            && let Ok(t) = self.resolve(&ix, ty)
            && ix.syms[t].kind == Kind::Type
        {
            return Ok(crate::reach::field_refs(&ix, t, field));
        }
        let i = self.resolve(&ix, name)?;
        let users = ix.used_by[i].clone();
        if users.is_empty() {
            return Ok(format!("Nothing uses {}.\n", ix.syms[i].qname()));
        }
        let mut out = format!("{} is used by {}:\n", ix.syms[i].qname(), users.len());
        let (exact, possible) = crate::index::users_split(&ix, i);
        if ix.syms[i].kind == Kind::Type {
            let (tests, users): (Vec<usize>, Vec<usize>) = exact.into_iter().partition(|&j| crate::is_test(&ix, j));
            out += &type_users(&ix, i, &users, &tests, usize::MAX);
            out += &possible_users(&ix, i, &possible);
            return Ok(out);
        }
        for u in exact {
            out += &ref_entry(&ix, u, i);
        }
        out += &possible_users(&ix, i, &possible);
        Ok(out)
    }
    /// `impact [name...]`: what changed code reaches (D-44). With no names, the
    /// task's changes in a board run, otherwise the changes since `test` last
    /// passed; a name that hasn't changed counts as changed throughout.
    pub(crate) fn impact(&self, names: &[&str], budget: usize) -> crate::failure::Out {
        let ix = self.index()?;
        let before = match crate::board::context() {
            Some((tid, _)) => {
                let mut st = self.db.prepare("SELECT entity, data FROM board_before WHERE task = ?1 AND entity LIKE 'symbol:%'")?;
                let rows: Vec<(String, String)> = st.query_map([&tid], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                let old = |data: &str| {
                    serde_json::from_str::<serde_json::Value>(data).ok().filter(|v| v[7] == 1).and_then(|v| v[4].as_str().map(String::from))
                };
                Some(rows.iter().map(|(k, d)| (k["symbol:".len()..].to_string(), old(d))).collect::<Vec<_>>())
            }
            None => self.since_pass()?,
        };
        let origins = if names.is_empty() {
            let Some(before) = before else {
                return Ok("Tests haven't passed in full yet, so there is no earlier state to compare with. Name the symbols: \
                     `impact <name>...`.\n"
                    .into());
            };
            crate::reach::origins(&ix, &before)
        } else {
            let mut out = vec![];
            for name in names {
                let i = self.resolve(&ix, name)?;
                let then: crate::Before = before.iter().flatten().filter(|(id, _)| *id == ix.syms[i].id).cloned().collect();
                out.extend(
                    Some(crate::reach::origins(&ix, &then))
                        .filter(|o| !o.is_empty())
                        .unwrap_or_else(|| vec![(i, (0..ix.syms[i].text.lines().count()).collect())]),
                );
            }
            out
        };
        if origins.is_empty() {
            return Ok("No source changed since tests last passed.\n".into());
        }
        let impact = crate::reach::flow(&ix, &origins);
        let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
        Ok(crate::reach::report(&ix, &impact, &crate::reach::reached(&crate::reach::suites(self, &ix)?, &syms), budget))
    }
    /// The submit reply's note on what the saved change reaches (D-44).
    /// `before`: (id, text before the submit; `None` if new).
    pub(crate) fn impact_note(&self, ix: &crate::Index, before: &[(String, Option<String>)]) -> Result<Vec<String>, crate::failure::Failure> {
        let origins = crate::reach::origins(ix, before);
        if origins.is_empty() {
            return Ok(vec![]);
        }
        let impact = crate::reach::flow(ix, &origins);
        let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
        Ok(crate::reach::note(ix, &impact, &crate::reach::reached(&crate::reach::suites(self, ix)?, &syms)))
    }
    pub(crate) fn view_with_header(&self, names: &[&str], bodies: bool, header: bool, budget: usize) -> crate::failure::Out {
        let ix = self.index()?;
        let (mut out, mut left) = (String::new(), vec![]);
        let fits = |out: &str, text: &str| out.is_empty() || out.len() + text.len() <= budget;
        for &name in names {
            let (text, served): (String, Vec<&crate::Sym>) = if let Some(m) = self.resolve_module(&ix, name) {
                let body = crate::store::module_text(&ix, m);
                if bodies && fits(&out, &body) {
                    (body, ix.syms.iter().filter(|s| s.module == m.path).collect())
                } else {
                    (crate::store::module_outline(&ix, m, &self.broken()?, bodies, header && !bodies), vec![])
                }
            } else {
                match self.resolve(&ix, name) {
                    Ok(i) => {
                        (format!("// ── {} ({}) ──\n{}\n\n", ix.syms[i].qname(), ix.syms[i].kind_text, ix.syms[i].text), vec![&ix.syms[i]])
                    }
                    Err(e) => (self.dependency(name).ok_or(e)?, vec![]),
                }
            };
            if left.is_empty() && fits(&out, &text) {
                out += &text;
                let printed: Vec<String> = match self.resolve_module(&ix, name) {
                    Some(m) => ix.syms.iter().filter(|s| s.module == m.path).map(crate::Sym::qname).collect(),
                    None => self.resolve(&ix, name).map(|i| vec![ix.syms[i].qname()]).unwrap_or_default(),
                };
                printed.into_iter().for_each(mark_shown);
                served.into_iter().for_each(|s| crate::reads::mark_served(&self.root, s));
            } else {
                left.push(name);
            }
        }
        if !left.is_empty() {
            out += &format!(
                "Not shown, to fit the size limit: `solace view {}{}`\n",
                left.join(" "),
                match (bodies, header) {
                    (true, _) => " --bodies",
                    (false, true) => " --header",
                    (false, false) => "",
                }
            );
        }
        Ok(out)
    }

}
/// Runs of capitalized words in the task, lowercased ("Desk Lamp", "TENOFF"),
/// leaving out a single word that starts a sentence.
fn capitalized_runs(task: &str) -> Vec<String> {
    let (mut out, mut run, mut start) = (vec![], vec![], true);
    for token in task.split_whitespace() {
        let word = token.trim_matches(|c: char| !c.is_alphanumeric());
        let capital = word.starts_with(|c: char| c.is_uppercase());
        if capital {
            run.push((word, start));
        }
        if !capital || token.ends_with(|c: char| !c.is_alphanumeric()) {
            if run.len() > 1 || run.first().is_some_and(|(w, s)| !s && w.len() >= 3) {
                out.push(run.iter().map(|r| r.0).collect::<Vec<_>>().join(" ").to_lowercase());
            }
            run.clear();
        }
        start = token.trim_end_matches(['"', '\'', ')']).ends_with(['.', '!', '?']);
    }
    out
}

/// String literals in `text` holding one of `terms` (lowercase) at a word's
/// start, each shown as
/// the smallest bracketed expression around it, on one line:
/// `coupon("TENOFF", FixedOff(1000))`. A block or a long expression gives the
/// literal's line instead.
fn evidence(text: &str, terms: &[String]) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    // chars inside string literals and comments, and the literals' spans
    let (mut inside, mut literals, mut k) = (vec![false; chars.len()], vec![], 0);
    while k < chars.len() {
        let start = k;
        if chars[k] == '"' {
            k += 1;
            while k < chars.len() && chars[k] != '"' {
                k += if chars[k] == '\\' { 2 } else { 1 };
            }
            literals.push((start, k.min(chars.len() - 1)));
        } else if chars[k] == '/' && chars.get(k + 1) == Some(&'/') {
            while k < chars.len() && chars[k] != '\n' {
                k += 1;
            }
        } else {
            k += 1;
            continue;
        }
        for c in inside.iter_mut().take((k + 1).min(chars.len())).skip(start) {
            *c = true;
        }
        k += 1;
    }
    let one_line = |cs: &[char]| -> String {
        let s: String = cs.iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        s.replace(", )", ")").replace(", ]", "]").replace("( ", "(").replace(" )", ")").replace("[ ", "[").replace(" ]", "]")
    };
    let mut out: Vec<String> = vec![];
    for (s, e) in literals {
        let literal = chars[s..=e].iter().collect::<String>().to_lowercase();
        // at a word's start: "lamp" in "LAMP-OAK", not "come" in "WELCOME15"
        let starts_word = |t: &String| literal.match_indices(t.as_str()).any(|(p, _)| !literal[..p].ends_with(char::is_alphanumeric));
        if !terms.iter().any(starts_word) {
            continue;
        }
        let mut depth = 0;
        let open = (0..s).rev().filter(|&j| !inside[j]).find(|&j| match chars[j] {
            ')' | ']' | '}' => {
                depth += 1;
                false
            }
            '(' | '[' | '{' if depth == 0 => true,
            '(' | '[' | '{' => {
                depth -= 1;
                false
            }
            _ => false,
        });
        let expression = open.filter(|&o| chars[o] != '{').and_then(|o| {
            let mut depth = 0;
            let close = (o..chars.len()).filter(|&j| !inside[j]).find(|&j| match chars[j] {
                '(' | '[' | '{' => {
                    depth += 1;
                    false
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    depth == 0
                }
                _ => false,
            })?;
            let mut from = o;
            while from > 0 && (chars[from - 1].is_alphanumeric() || "_.".contains(chars[from - 1])) {
                from -= 1;
            }
            Some(one_line(&chars[from..=close])).filter(|x| x.len() <= 200)
        });
        let line = || {
            let a = chars[..s].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
            let b = chars[s..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| s + p);
            one_line(&chars[a..b]).chars().take(200).collect()
        };
        let shown = expression.unwrap_or_else(line);
        if !out.contains(&shown) {
            out.push(shown);
        }
    }
    out
}

/// A type's users for `inspect --context` and `refs`: the sites that
/// construct or match it with every field, one line each, then the other
/// non-test users by name (at most `cap`).
fn type_users(ix: &crate::Index, ty: usize, users: &[usize], tests: &[usize], cap: usize) -> String {
    let mut out = String::new();
    let (mut built, mut others, mut updates) = (vec![], vec![], 0);
    for &j in users.iter().chain(tests) {
        updates += crate::sites(ix, j, ty, true).len();
        let at = crate::sites(ix, j, ty, false);
        if at.is_empty() && !tests.contains(&j) {
            others.push(ix.syms[j].qname());
        }
        built.extend(at.into_iter().map(|(n, line)| format!("-- {}, line {n}: {line}\n", ix.syms[j].qname())));
    }
    let q = ix.syms[ty].qname();
    if !built.is_empty() {
        out += &format!("\n== Constructed by ({}): every field listed, so a new field breaks these ==\n{}", built.len(), built.concat());
        if updates > 0 {
            out += &format!("Record updates and patterns with `..` need no change; {updates} not listed.\n");
        }
    }
    if !others.is_empty() {
        let more = if others.len() > cap { format!(", …and {} more (solace refs {q})", others.len() - cap) } else { String::new() };
        out += &format!(
            "\n== Other users ({}); `refs {q}.<field>` lists a field's readers ==\n  {}{more}\n",
            others.len(),
            others.iter().take(cap).cloned().collect::<Vec<_>>().join(", ")
        );
    }
    out
}

/// The lines where `caller` calls function `f`, quoted under its entry.
fn call_lines(ix: &crate::Index, caller: usize, f: usize) -> String {
    if ix.syms[f].kind != Kind::Fn {
        return String::new();
    }
    crate::ref_lines(ix, caller, f).iter().map(|(n, line)| format!("  > line {n}: {line}\n")).collect()
}

fn shown_earlier(q: &str) -> bool {
    crate::SHOWN.with(|s| s.borrow().as_ref().is_some_and(|s| s.contains(q)))
}

fn mark_shown(q: String) {
    crate::SHOWN.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.insert(q);
        }
    })
}

/// One user's entry under `refs`, printed once per reply.
fn ref_entry(ix: &crate::Index, user: usize, target: usize) -> String {
    let q = ix.syms[user].qname();
    let mut out =
        if shown_earlier(&q) { format!("-- {q} (above)\n") } else { format!("-- {q}\n{}\n", crate::index::signature(ix, &ix.syms[user])) };
    out += &call_lines(ix, user, target);
    mark_shown(q);
    out
}

/// The `refs` section for uses only type inference could settle.
fn possible_users(ix: &crate::Index, target: usize, users: &[usize]) -> String {
    if users.is_empty() {
        return String::new();
    }
    let mut out = format!("\n== Possible uses ({}): only type inference could settle these ==\n", users.len());
    for &u in users {
        out += &ref_entry(ix, u, target);
    }
    out
}

