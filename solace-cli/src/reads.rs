//! A task run's reads (D-157): each symbol the MCP server printed in full, at
//! the revision served. The server can't see the agent compact its
//! conversation, but a re-read of a symbol unchanged since shows it, and the
//! reply then brings back what the run was working on.

use super::*;

thread_local! {
    /// The symbols this MCP call printed in full, in a task run; `None`
    /// elsewhere, so other runs keep no reads.
    static SERVED: RefCell<Option<Vec<Served>>> = const { RefCell::new(None) };
}

/// The most bytes of a catch's run outline.
const OUTLINE_CAP: usize = 3000;

/// Records symbol `s` of the project at `root` as served in full.
pub(super) fn mark_served(root: &Path, s: &Sym) {
    SERVED.with(|v| {
        if let Some(v) = v.borrow_mut().as_mut() {
            let project = scope::dir(&scope::top(root), root);
            let revision = revision(&s.text);
            v.push(Served { project, symbol: s.qname(), latest: revision.clone(), revision, recent: true });
        }
    })
}

fn take_served() -> Vec<Served> {
    SERVED.with(|v| v.borrow_mut().as_mut().map(std::mem::take)).unwrap_or_default()
}

/// The task run this MCP process serves: its reads and where they are kept.
pub(super) struct Run {
    top: PathBuf,
    tid: String,
    rid: String,
    reads: Reads,
}

impl Run {
    /// The task run, with the reads its run record holds so a resumed run goes
    /// on from them; `None` outside a task run, strategy runs included.
    pub(super) fn load() -> Option<Run> {
        Self::from_context(board::context())
    }

    /// The run for `context`'s task and run; `None` for a strategy run and for
    /// a run outside a task, whose context is none.
    fn from_context(context: Option<(String, String)>) -> Option<Run> {
        let (tid, rid) = context?;
        let top = scope::top(&Store::root().ok().or_else(|| env::current_dir().ok()).unwrap_or_default());
        let reads = Reads::from_json(&board::run_reads(&top, &tid, &rid));
        Some(Run { top, tid, rid, reads })
    }

    /// An MCP call starts: what it prints in full is recorded from here.
    pub(super) fn start_call() {
        SERVED.with(|v| *v.borrow_mut() = Some(vec![]));
    }

    /// The MCP call of `commands` ended: what its reply adds, within `budget`.
    /// The reads are saved on the run.
    pub(super) fn end_call(&mut self, commands: &[&str], budget: usize) -> String {
        let served = take_served();
        let changed = commands.iter().any(|c| {
            let words: Vec<String> = c.strip_prefix("solace").unwrap_or(c).split_whitespace().map(String::from).collect();
            verb_words(&words).first().is_some_and(|v| STOPS.contains(v))
        });
        let (top, tid, rid) = (&self.top, &self.tid, &self.rid);
        let out = if served.is_empty() && !changed {
            String::new()
        } else {
            let out = catch_up(&mut self.reads, top, served, changed, budget, &|b| board::run_outline(top, tid, rid, b));
            let _ = board::record_reads(top, tid, rid, self.reads.to_json());
            out
        };
        SERVED.with(|v| v.borrow_mut().take());
        out
    }
}

/// What a task run's reply adds after a call that read or changed code. A
/// re-read of a symbol unchanged since the run was served it means the
/// conversation was most likely compacted: once a window, the reply adds the
/// current views of the window's other symbols, then `outline` of the run,
/// within `budget`. A call with a change starts a new window, which keeps only
/// the symbols the change made differ from what was served.
fn catch_up(reads: &mut Reads, top: &Path, served: Vec<Served>, changed: bool, budget: usize, outline: &dyn Fn(usize) -> String) -> String {
    // A change command that failed or changed nothing the run read keeps the
    // window; one that did starts a new window, and its call's reads are no catch.
    let reset = changed && {
        let stores = opened(top, reads);
        reads.new_window(&|r| current_revision(&stores, r))
    };
    let again = if reset { None } else { reads.reread(&served).map(|r| r.symbol.clone()) };
    reads.record(&served);
    let Some(symbol) = again else {
        return String::new();
    };
    reads.caught = true;
    let mut out = format!("\n`{symbol}` hasn't changed since this run last read it, so the conversation was likely compacted.\n");
    let story = outline(OUTLINE_CAP.min(budget.saturating_sub(out.len())));
    let mut views = String::new();
    let mut left_out: Vec<String> = vec![];
    for (project, names) in reads.others(&served) {
        // room for the two headings
        let left = budget.saturating_sub(out.len() + story.len() + views.len() + 100);
        if left == 0 {
            left_out.extend(names);
            continue;
        }
        let Ok(s) = Store::at(&top.join(&project)) else { left_out.extend(names); continue };
        let Ok(ix) = s.index() else { left_out.extend(names); continue };
        let names: Vec<&str> = names.iter().map(String::as_str).filter(|n| s.resolve(&ix, n).is_ok()).collect();
        match s.view(&names, false, left) {
            Ok(v) if v.len() <= left => views += &v,
            _ => left_out.extend(names.iter().map(|n| n.to_string())),
        }
    }
    reads.record(&take_served());
    if !left_out.is_empty() {
        views += &format!("Not shown, to fit the size limit: {}\n", left_out.join(" "));
    }
    if !views.is_empty() {
        out += &format!("What this run read or changed since its last change, as it is now:\n{views}");
    }
    if !story.is_empty() {
        out += &format!("This run so far, one step per line:\n{story}");
    }
    out
}

/// Each project the window's symbols live in, opened once for a change
/// command, so each symbol's revision is read without reopening the store.
fn opened(top: &Path, reads: &Reads) -> Vec<(String, Store, Index)> {
    let mut out: Vec<(String, Store, Index)> = vec![];
    for r in reads.symbols.iter().filter(|r| r.recent) {
        if out.iter().any(|(p, _, _)| *p == r.project) {
            continue;
        }
        let Ok(s) = Store::at(&top.join(&r.project)) else { continue };
        let Ok(ix) = s.index() else { continue };
        out.push((r.project.clone(), s, ix));
    }
    out
}

/// The revision of `r`'s symbol in its project now; an empty string once it's
/// gone, which differs from every revision the run was served, and `None`
/// when its store won't open — a store that won't open is no change.
fn current_revision(stores: &[(String, Store, Index)], r: &Served) -> Option<String> {
    stores.iter().find(|(p, _, _)| *p == r.project).map(|(_, s, ix)| s.resolve(ix, &r.symbol).map(|i| revision(&ix.syms[i].text)).unwrap_or_default())
}

/// A symbol a task run was served, at the revision served; `recent` while it
/// was read or changed since the run's last change command.
#[derive(Clone, Debug, PartialEq)]
struct Served {
    project: String,
    symbol: String,
    revision: String,
    recent: bool,
    /// The revision when the run last looked: served, or at the last window
    /// start, so a later change command sees only what it changed itself.
    latest: String,
}

impl Served {
    fn same(&self, other: &Served) -> bool {
        self.project == other.project && self.symbol == other.symbol
    }
}

/// A task run's reads, each symbol once, latest last, and whether the window
/// since the last change command has had its catch.
#[derive(Default)]
struct Reads {
    symbols: Vec<Served>,
    caught: bool,
}

impl Reads {
    fn from_json(v: &Value) -> Reads {
        let field = |r: &Value, k: &str| r[k].as_str().unwrap_or("").to_string();
        let symbols = v["symbols"].as_array().map_or(vec![], |a| {
            a.iter()
                .map(|r| Served {
                    project: field(r, "project"),
                    symbol: field(r, "symbol"),
                    revision: field(r, "revision"),
                    recent: r["recent"] == true,
                    latest: r["latest"].as_str().map_or_else(|| field(r, "revision"), String::from),
                })
                .collect()
        });
        Reads { symbols, caught: v["caught"] == true }
    }

    fn to_json(&self) -> Value {
        let symbols: Vec<Value> = self
            .symbols
            .iter()
            .map(|r| json!({"project":r.project,"symbol":r.symbol,"revision":r.revision,"recent":r.recent,"latest":r.latest}))
            .collect();
        json!({"symbols":symbols,"caught":self.caught})
    }

    /// The first of `served` the run was served before at the same revision,
    /// unless the window has had its catch.
    fn reread<'a>(&self, served: &'a [Served]) -> Option<&'a Served> {
        if self.caught {
            return None;
        }
        served.iter().find(|s| self.symbols.iter().any(|r| r.same(s) && r.revision == s.revision))
    }

    /// When a recent symbol's revision `now` differs from the one the run last
    /// looked at, starts a window that keeps only those, and says so; else
    /// changes nothing.
    fn new_window(&mut self, now: &dyn Fn(&Served) -> Option<String>) -> bool {
        let current: Vec<Option<String>> = self.symbols.iter().map(|r| r.recent.then(|| now(r)).flatten()).collect();
        let differ: Vec<bool> = self.symbols.iter().zip(&current).map(|(r, c)| c.as_ref().is_some_and(|c| *c != r.latest)).collect();
        if !differ.contains(&true) {
            return false;
        }
        for ((r, d), c) in self.symbols.iter_mut().zip(differ).zip(current) {
            r.recent = d;
            if let Some(c) = c {
                r.latest = c;
            }
        }
        self.caught = false;
        true
    }

    fn record(&mut self, served: &[Served]) {
        for s in served {
            self.symbols.retain(|r| !r.same(s));
            self.symbols.push(s.clone());
        }
        // The history is saved on the run after every call, so it stays
        // bounded: the oldest entry the window no longer needs goes first, so
        // a long window keeps the symbols it read.
        const CAP: usize = 60;
        let mut over = self.symbols.len().saturating_sub(CAP);
        self.symbols.retain(|r| {
            if over == 0 || r.recent {
                return true;
            }
            over -= 1;
            false
        });
        self.symbols.drain(..self.symbols.len().saturating_sub(CAP));
    }

    /// The window's symbols other than `served`, by project, in the order read.
    fn others(&self, served: &[Served]) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = vec![];
        for r in self.symbols.iter().filter(|r| r.recent && !served.iter().any(|s| s.same(r))) {
            match out.iter_mut().find(|(p, _)| *p == r.project) {
                Some((_, names)) => names.push(r.symbol.clone()),
                None => out.push((r.project.clone(), vec![r.symbol.clone()])),
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "reads_tests.rs"]
mod tests;
