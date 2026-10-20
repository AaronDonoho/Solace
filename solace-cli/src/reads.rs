//! A task run's reads (D-157): each symbol the MCP server printed in full, at
//! the revision served, and the window since its last change. `session
//! remember` reads them back after a compaction.

use super::*;

thread_local! {
    /// The symbols this MCP call printed in full, in a task run; `None`
    /// elsewhere, so other runs keep no reads.
    static SERVED: RefCell<Option<Vec<Served>>> = const { RefCell::new(None) };
    /// The reads the run held when this MCP call started; `session remember`
    /// reads this copy, so it is fresh even before `serve` applies the inbox.
    static WINDOW: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// The most bytes of a run outline in `session remember`.
const OUTLINE_CAP: usize = 3000;

/// The task run's reads as this MCP call started; `None` outside a task run or
/// before its first call.
pub(super) fn window_reads() -> Option<Value> {
    WINDOW.with(|v| v.borrow().clone())
}

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
    dirty: bool,
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
        Some(Run { top, tid, rid, reads, dirty: false })
    }

    /// An MCP call starts: what it prints in full is recorded from here, and
    /// `session remember` sees the run's reads as they are now.
    pub(super) fn start_call(&mut self) {
        SERVED.with(|v| *v.borrow_mut() = Some(vec![]));
        WINDOW.with(|v| *v.borrow_mut() = Some(self.reads.to_json()));
        self.dirty = false;
    }

    /// The MCP call ended: a read no command closed joins the window, and the
    /// reads are saved once. Commands update their own window as they finish.
    pub(super) fn end_call(&mut self) {
        let served = take_served();
        if !served.is_empty() {
            self.reads.record(&served);
            self.dirty = true;
        }
        SERVED.with(|v| v.borrow_mut().take());
        WINDOW.with(|v| v.borrow_mut().take());
        if self.dirty {
            let (top, tid, rid) = (&self.top, &self.tid, &self.rid);
            let _ = board::record_reads(top, tid, rid, self.reads.to_json());
            self.dirty = false;
        }
    }

    /// After one command: its served symbols and any change join the window
    /// before the next command, so `session remember` in the same call sees them.
    pub(super) fn end_command(&mut self, words: &[String]) {
        let served = take_served();
        // `mark_served` records only while `SERVED` is Some; arm it for the next command.
        SERVED.with(|v| *v.borrow_mut() = Some(vec![]));
        let changed = is_change(words);
        if !served.is_empty() || changed {
            if changed {
                let stores = opened(&self.top, &self.reads);
                self.reads.new_window(&|r| current_revision(&stores, r));
            }
            self.reads.record(&served);
            self.window();
            self.dirty = true;
        }
    }

    /// Shows `session remember` the run's reads as they are now, including a
    /// change the inbox has not applied yet.
    fn window(&self) {
        WINDOW.with(|v| *v.borrow_mut() = Some(self.reads.to_json()));
    }
}

/// What `session remember` replies: the task's context, then the window's
/// symbols other than those the call printed, as `view` prints them, then
/// `outline` of the run, within `budget`. What did not fit is named; when the
/// context itself does not fit, it is cut and the notice says so. It reads
/// only: it records nothing and has no once-per-window rule.
pub(super) fn remember_reply(context: &str, reads: &Value, top: &Path, outline: &dyn Fn(usize) -> String, budget: usize) -> String {
    let reads = Reads::from_json(reads);
    const VIEWS_HEAD: &str = "What this run read or changed since its last change, as it is now:\n";
    const STORY_HEAD: &str = "This run so far, one step per line:\n";
    const NOT_SHOWN: &str = "Not shown, to fit the size limit: ";
    const NOTES: &str = "the task's notes";
    // Open each project once, and keep only names it still has: a symbol deleted
    // since it was read is no view to name as left out.
    let groups: Vec<(String, Store, Index, Vec<String>)> = reads
        .others(&[])
        .into_iter()
        .filter_map(|(project, names)| {
            let s = Store::at(&top.join(&project)).ok()?;
            let ix = s.index().ok()?;
            let names: Vec<String> = names.into_iter().filter(|n| s.resolve(&ix, n).is_ok()).collect();
            (!names.is_empty()).then_some((project, s, ix, names))
        })
        .collect();
    let names = groups.iter().flat_map(|(_, _, _, names)| names.iter()).map(|n| n.len() + 1).sum::<usize>();
    // The headings are added last. The first pass reserves neither symbol names
    // nor a notice; when something is left out, a second pass makes that room.
    let render = |notice: bool, reserved: usize| -> (String, bool, bool) {
        let fixed = VIEWS_HEAD.len() + STORY_HEAD.len() + if notice { NOT_SHOWN.len() + reserved + 2 } else { 0 };
        // A context over its room is cut, and the notice says the task's notes.
        let cut = context.len() > budget.saturating_sub(fixed);
        let room = fixed + if cut { NOTES.len() + 1 } else { 0 };
        let mut left_out: Vec<String> = if cut { vec![NOTES.to_string()] } else { vec![] };
        let mut out = cut_text(context, budget.saturating_sub(room)).to_string();
        let story = outline(OUTLINE_CAP.min(budget.saturating_sub(out.len() + room)));
        let mut views = String::new();
        let mut view_left_out = false;
        for (_, s, _, names) in &groups {
            let left = budget.saturating_sub(out.len() + story.len() + views.len() + room);
            if left == 0 {
                left_out.extend(names.iter().cloned());
                view_left_out = true;
                continue;
            }
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            match s.view(&names, false, left) {
                Ok(v) if v.len() <= left => views += &v,
                _ => {
                    left_out.extend(names.iter().map(|n| n.to_string()));
                    view_left_out = true;
                }
            }
        }
        if !left_out.is_empty() {
            views += &format!("{NOT_SHOWN}{}\n", left_out.join(" "));
        }
        if !views.is_empty() {
            out += &format!("{VIEWS_HEAD}{views}");
        }
        if !story.is_empty() {
            out += &format!("{STORY_HEAD}{story}");
        }
        (out, view_left_out, !left_out.is_empty())
    };
    // The ask is read-only: the views it prints are not new reads for the run.
    let (out, _, _) = without_serving(|| {
        let (out, view_left, any_left) = render(false, 0);
        if !any_left {
            (out, false, false)
        } else if view_left {
            render(true, names)
        } else {
            render(true, 0)
        }
    });
    // A budget too small even for the notice still gets a reply within it.
    cut_text(&out, budget).to_string()
}

/// `s` cut to `room` bytes at a line end where one is near, so a reply that
/// must cut the task's context ends on a whole line.
fn cut_text(s: &str, room: usize) -> &str {
    if s.len() <= room {
        return s;
    }
    let mut end = room;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    match s[..end].rfind('\n') {
        Some(i) if i > 0 => &s[..i],
        _ => &s[..end],
    }
}

/// Whether a command's words name a change command (D-157).
fn is_change(words: &[String]) -> bool {
    verb_words(words).first().is_some_and(|v| STOPS.contains(v))
}

/// Runs `f` without recording what it prints as served: `session remember`
/// must not change the run's reads.
fn without_serving<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Vec<Served>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SERVED.with(|v| *v.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SERVED.with(|v| v.borrow_mut().take()));
    f()
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
    stores
        .iter()
        .find(|(p, _, _)| *p == r.project)
        .map(|(_, s, ix)| s.resolve(ix, &r.symbol).map(|i| revision(&ix.syms[i].text)).unwrap_or_default())
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

/// A task run's reads, each symbol once, latest last; `recent` marks the
/// window since the last change command.
#[derive(Default)]
struct Reads {
    symbols: Vec<Served>,
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
        Reads { symbols }
    }

    fn to_json(&self) -> Value {
        let symbols: Vec<Value> = self
            .symbols
            .iter()
            .map(|r| json!({"project":r.project,"symbol":r.symbol,"revision":r.revision,"recent":r.recent,"latest":r.latest}))
            .collect();
        json!({"symbols":symbols})
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
    /// A symbol with an empty latest revision is one the run saw deleted: it is
    /// gone from its project, so it is not a view to name as left out.
    fn others(&self, served: &[Served]) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = vec![];
        for r in self.symbols.iter().filter(|r| r.recent && !r.latest.is_empty() && !served.iter().any(|s| s.same(r))) {
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
