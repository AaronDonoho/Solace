//! `extract <symbols…> --to <module> [--to-project <folder>]` (D-103): moves
//! symbols to another module of the project, or of another project of the
//! task, with their identity, brings along the private helpers only they use,
//! rewrites every reference and import, moves the tests that reach only moved
//! code, and refuses a module or package cycle (`E5002`). One transaction,
//! gated on `check_all`. JavaScript FFI paths follow the module; into another
//! project the `.mjs` files go along (`E5007` names what keeps one), and the
//! reply names the suites that reach moved code, since FFI paths aren't type-checked.
use super::*;
use std::collections::BTreeSet;

pub(crate) mod ahead;
mod merge;
mod rust;
pub(super) mod ffi;
pub(super) mod deps;
pub(super) mod plan;
mod apply;

use std::collections::HashMap;
use crate::extract::ffi::ffi_carries;
use crate::extract::deps::{dependency_lines, merged};
use crate::extract::plan::{
    build_rewrite, check_cycles, check_module_clashes, check_opaque, check_symbol_clashes, dependent_plan, destination, module_notes,
    moves, MovePlan, parse, resolve_listed, scan_dependents, SymbolMoves, target_module,
};
#[cfg(test)] use crate::extract::deps::add_dependency;
#[cfg(test)] use crate::extract::plan::{item_split, make_pub};
#[cfg(test)] use crate::extract::apply::with_put_back;
use crate::extract::apply::{baseline, created_modules, extract_reply_text, nested, run_move, Baseline, MoveRun};
const MODULE_HINT: &str = "to move a whole module, use move src/x.gleam src/y.gleam";

impl Store {
    /// Whether the project in `dir` and this one depend on each other by path.
    fn connected(&self, dir: &Path) -> bool {
        let (mine, theirs) = (self.root.canonicalize().ok(), dir.canonicalize().ok());
        match (mine, theirs) {
            (Some(m), Some(t)) => deps::path_deps(&self.root).contains(&t) || deps::path_deps(dir).contains(&m),
            _ => false,
        }
    }

    /// `extract <symbols…> --to <module>` (D-103).
    pub(crate) fn extract(&self, args: &[&str], budget: usize) -> Out {
        let p = parse(args)?;
        if crate::language::is_rust(&self.root) {
            if p.to_project.is_some() {
                return fail(concat!(
                    "E6013 `extract --to-project` is not available for Rust; ",
                    "extract within this project with --to <module> (D-185)"
                ));
            }
            return rust::extract(self, &p, budget);
        }
        let ix = self.index()?;
        let top = scope::top(&self.root);
        let mine = scope::dir(&top, &self.root);
        let members = workspace::members(&top).unwrap_or_default();

        let dest = destination(self, &p, &top, &members)?;
        let to = target_module(self, &ix, &p, &dest, &mine, &members)?;

        let (listed, already) = resolve_listed(self, &ix, &p.names, &dest, &to)?;
        if listed.is_empty() {
            return Ok(format!("{} already in {to}; nothing changed.\n", already.join(", ")));
        }
        let target_is_test = plan::tail(&to).ends_with("_test");
        if listed.iter().any(|&i| is_test(&ix, i)) && !target_is_test {
            return fail("test symbols go only to a test module (*_test): use --to <module>_test");
        }
        if listed.iter().any(|&i| !is_test(&ix, i)) && target_is_test {
            return fail("source symbols do not go to a test module (*_test): use --to <module>");
        }

        let notes = module_notes(self, &dest, &ix, &to, &top, &mine, &members)?;

        let SymbolMoves { moved, helpers, moved_tests, test_to, newmod } = moves(&ix, &listed, &to, target_is_test);
        let n = ix.syms.len();

        check_module_clashes(&ix, &dest, &top, &mine, &members, &to, &test_to, &moved, &newmod)?;
        check_symbol_clashes(&ix, &dest, &moved, &newmod)?;
        check_opaque(&ix, &moved)?;
        let source_needs = check_cycles(&ix, &moved, &newmod, &dest, &self.root, &mine)?;

        let (names, deps) = scan_dependents(self, &ix, &top, &mine, &members, &dest, &moved)?;

        // JavaScript FFI files: with --to-project, those the moved code uses go along
        let carries = match &dest {
            Some(d) => ffi_carries(&self.root, &ix, &moved, &newmod, d)?,
            None => vec![],
        };

        let (plan, now_pub, ffi) = build_rewrite(&ix, &dest, &to, &test_to, &moved, &newmod, &names, &carries)?;

        // the dependents' references, resolved across their store and the source's
        let joint: Vec<Index> = deps.iter().map(|dp| merged(&dp.ix, &ix)).collect();
        let dplans = deps.iter().zip(&joint).map(|(dp, jx)| dependent_plan(dp, jx, &moved, &newmod)).collect::<Result<Vec<_>, _>>()?;

        let created = created_modules(self, &dest, &ix, &to, &test_to, target_is_test, &moved, &newmod, &moved_tests)?;
        let tgt: &Store = dest.as_ref().map_or(self, |d| &d.store);
        let created_files: Vec<PathBuf> = created.iter().map(|(m, dir)| tgt.root.join(dir).join(format!("{m}.gleam"))).collect();

        let (dep, dep_lines) = dependency_lines(self, &dest, &deps, &dplans, source_needs, &mine, &to, &test_to);

        let Baseline { suites_before, before, dest_broken, deps_broken } = baseline(self, &ix, &dest, &deps)?;

        let old_modules: BTreeSet<String> = (0..n).filter(|&i| moved[i]).map(|i| ix.syms[i].module.clone()).collect();
        let move_plan = MovePlan {
            s: self,
            ix: &ix,
            listed,
            helpers,
            to,
            test_to,
            mine: &mine,
            dest,
            dep,
            deps,
            dep_lines,
            already,
            now_pub,
            plan,
            dplans,
            moved_tests,
            newmod,
            carries,
            ffi,
            created,
            notes,
            moved,
            old_modules,
        };
        let finished = {
            let mut stores: Vec<&Store> = vec![self];
            stores.extend(move_plan.deps.iter().map(|dp| &dp.store));
            stores.extend(move_plan.dest.as_ref().map(|d| &d.store));
            let run = MoveRun {
                plan: &move_plan,
                before: &before,
                dest_broken: &dest_broken,
                deps_broken: &deps_broken,
                suites_before: &suites_before,
            };
            let mut body = || run_move(&run);
            nested(&stores, &created_files, &mut body)?
        };
        extract_reply_text(&move_plan, &finished, budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;

    const BOARD: &str = concat!(
        "pub type Task {\n  Task(id: Int, lane: Int)\n}\n\n",
        "pub fn new_task(id: Int) -> Task {\n  Task(id, lane_of(id))\n}\n\n",
        "fn lane_of(id: Int) -> Int {\n  id % 3\n}\n\n",
        "pub fn bump(t: Task) -> Task {\n  let _note = \"a Task here\" // Task\n  Task(..t, lane: t.lane + 1)\n}\n\n",
        "pub fn lane(t: Task) -> Int {\n  case t {\n    Task(lane: l, ..) -> l\n  }\n}\n",
    );

    fn text(s: &Store, name: &str) -> String {
        let ix = s.index().unwrap();
        ix.syms[s.resolve(&ix, name).unwrap()].text.clone()
    }
    fn header(s: &Store, module: &str) -> String {
        s.index().unwrap().modules.iter().find(|m| m.path == module).map(|m| m.header.clone()).unwrap_or_default()
    }
    fn extract(s: &Store, args: &[&str]) -> Out {
        s.extract(args, usize::MAX)
    }

    /// A type moves; annotations, constructor calls, patterns, record updates
    /// and a reference after a string literal or comment are all rewritten,
    /// and the symbol keeps its identity.
    #[test]
    fn rewrites_every_position_and_keeps_identity() {
        let store = project(&[("src/board.gleam", BOARD)]);
        let ix = store.index().unwrap();
        let id = ix.syms[store.resolve(&ix, "board.Task").unwrap()].id.clone();
        let out = extract(&store, &["board.Task", "--to", "src/task.gleam"]).unwrap();
        assert!(out.starts_with("Moved board.Task to task\n"), "{out}");
        assert!(out.contains("Created module task") && out.contains("Rewrote 7 references in 1 module"), "{out}");
        let ix = store.index().unwrap();
        let moved = &ix.syms[store.resolve(&ix, "task.Task").unwrap()];
        assert_eq!(moved.id, id);
        assert_eq!(moved.module, "task");
        assert!(
            text(&store, "board.new_task").contains("-> task.Task {\n  task.Task(id, lane_of(id))"),
            "{}",
            text(&store, "board.new_task")
        );
        let bump = text(&store, "board.bump");
        assert!(
            bump.contains("(t: task.Task) -> task.Task")
                && bump.contains("\"a Task here\" // Task")
                && bump.contains("task.Task(..t, lane: t.lane + 1)"),
            "{bump}"
        );
        assert!(text(&store, "board.lane").contains("task.Task(lane: l, ..) -> l"));
        assert!(header(&store, "board").contains("import task"));
        // run again: nothing to do
        let again = extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(again, "task.Task already in task; nothing changed.\n");
    }

    /// An unqualified import follows the symbol, constructor-only items too,
    /// and the old import goes when nothing else uses it.
    #[test]
    fn unqualified_imports_move_with_the_symbol() {
        let app = "import board.{type Task, Task}\n\npub fn mk() -> Task {\n  Task(1, 2)\n}\n";
        let only_ctor = "import board.{Task}\n\npub fn mk2(id: Int) {\n  Task(id, 1)\n}\n";
        let store = project(&[("src/board.gleam", BOARD), ("src/app.gleam", app), ("src/other.gleam", only_ctor)]);
        extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(text(&store, "app.mk"), "pub fn mk() -> Task {\n  Task(1, 2)\n}");
        assert!(
            header(&store, "app").contains("import task.{type Task, Task}") && !header(&store, "app").contains("board"),
            "{}",
            header(&store, "app")
        );
        assert!(
            header(&store, "other").contains("import task.{Task}") && !header(&store, "other").contains("board"),
            "{}",
            header(&store, "other")
        );
    }

    /// Moved code names staying code through its module, and `target.x` is bare;
    /// a private symbol that moved code uses becomes pub (new revision).
    #[test]
    fn moved_code_refers_back_and_target_references_become_bare() {
        let board = concat!(
            "pub type Task {\n  Task(id: Int)\n}\n\n",
            "fn lane_order(t: Task) -> Int {\n  t.id\n}\n\n",
            "pub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\n",
            "pub fn first(t: Task) -> Int {\n  lane_order(t)\n}\n\n",
            "pub fn twice(n: Int) -> Int {\n  util.double(n)\n}\n",
        );
        let store = project(&[
            ("src/board.gleam", &format!("import util\n\n{board}")),
            ("src/util.gleam", "pub fn double(n: Int) -> Int {\n  n * 2\n}\n"),
        ]);
        let out = extract(&store, &["board.sort", "--to", "task"]).unwrap();
        assert!(out.contains("Now pub: lane_order (new revision)") && !out.contains("helper"), "{out}");
        let sort = text(&store, "task.sort");
        assert!(sort.contains("board.lane_order(a) <= board.lane_order(b)") && sort.contains("a: board.Task"), "{sort}");
        assert!(header(&store, "task").contains("import board"));
        assert!(text(&store, "board.lane_order").starts_with("pub fn lane_order"));
        // `util.double` is bare once `twice` moves into util
        let out = extract(&store, &["board.twice", "--to", "util"]).unwrap();
        assert!(out.contains("Rewrote 1 reference in 1 module"), "{out}");
        assert!(text(&store, "util.twice").contains("  double(n)"), "{}", text(&store, "util.twice"));
        assert!(!header(&store, "board").contains("util"), "{}", header(&store, "board"));
    }

    /// Private helpers only the moved symbol uses come along; a helper used
    /// from both sides stays and becomes pub.
    #[test]
    fn private_helpers_come_along_or_become_pub() {
        let store = project(&[("src/board.gleam", BOARD)]);
        let out = extract(&store, &["board.new_task", "--to", "task"]).unwrap();
        assert!(out.contains("Moved board.new_task (+1 helper: lane_of) to task"), "{out}");
        assert!(store.resolve(&store.index().unwrap(), "task.lane_of").is_ok());
        let shared = format!("{BOARD}\npub fn reset(id: Int) -> Int {{\n  lane_of(id) + 0\n}}\n");
        let store = project(&[("src/board.gleam", &shared)]);
        let out = extract(&store, &["board.new_task", "--to", "task"]).unwrap();
        assert!(!out.contains("helper") && out.contains("Now pub: lane_of (new revision)"), "{out}");
        assert!(text(&store, "task.new_task").contains("board.lane_of(id)"));
    }

    /// A qualifier that is a local name is never aliased around: the
    /// unqualified import is used instead.
    #[test]
    fn a_qualifier_that_is_a_local_name_gets_the_unqualified_form() {
        let board = format!("{BOARD}\npub fn make(task: Int) -> Task {{\n  Task(task, 1)\n}}\n");
        let store = project(&[("src/board.gleam", &board)]);
        extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert_eq!(text(&store, "board.make"), "pub fn make(task: Int) -> Task {\n  Task(task, 1)\n}");
        assert!(header(&store, "board").contains("import task.{type Task, Task}"), "{}", header(&store, "board"));
    }

    /// A name the target already defines is refused, naming it.
    #[test]
    fn a_name_taken_in_the_target_is_refused() {
        let store =
            project(&[("src/board.gleam", "pub fn show() -> Int {\n  1\n}\n"), ("src/task.gleam", "pub fn show() -> Int {\n  2\n}\n")]);
        let err = extract(&store, &["board.show", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E1004 task.show already exists in task"), "{err}");
        assert!(err.ends_with("; to fold board.show into it, use merge task.show board.show"), "{err}");
    }

    /// Moving code that staying code calls, and that calls staying code, would
    /// make a module loop: refused with every boundary use, then the retry works.
    #[test]
    fn a_module_cycle_is_refused_with_every_boundary_use() {
        let board = concat!(
            "pub type Task {\n  Task(id: Int)\n}\n\n",
            "pub fn lane_order(t: Task) -> Int {\n  t.id\n}\n\n",
            "pub fn sort(a: Task, b: Task) -> Bool {\n  lane_order(a) <= lane_order(b)\n}\n\n",
            "pub fn top(a: Task, b: Task) -> Bool {\n  sort(a, b)\n}\n",
        );
        let store = project(&[("src/board.gleam", board)]);
        let err = extract(&store, &["board.sort", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E5002 moved code uses 2 symbols that stay: "), "{err}");
        assert!(err.contains("lane_order (by sort)") && err.contains("Task (by sort)") && err.contains("would bring 2 in total"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.sort").is_ok(), "nothing changed");
        extract(&store, &["board.sort", "board.lane_order", "board.Task", "--to", "task"]).unwrap();
        assert!(text(&store, "board.top").contains("task.sort(a, b)"));
    }

    /// An opaque type stays with the code that builds it.
    #[test]
    fn an_opaque_type_is_refused_apart_from_its_constructors() {
        let board = concat!(
            "pub opaque type Box {\n  Box(n: Int)\n}\n\n",
            "pub fn new_box(n: Int) -> Box {\n  Box(n)\n}\n\n",
            "pub fn open(b: Box) -> Box {\n  b\n}\n",
        );
        let store = project(&[("src/board.gleam", board)]);
        let err = extract(&store, &["board.Box", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("Box is opaque; its constructors are used by board.new_box; extract them too"), "{err}");
        extract(&store, &["board.Box", "board.new_box", "--to", "task"]).unwrap();
        assert!(text(&store, "board.open").contains("task.Box"));
    }

    /// A test that reaches only moved code moves to `<target>_test`; one that
    /// also reaches staying code is rewritten in place.
    #[test]
    fn tests_split_by_what_they_reach() {
        let board = format!("{BOARD}\npub fn keep() -> Int {{\n  1\n}}\n");
        let tests = concat!(
            "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n\n",
            "pub fn mixed_test() {\n  let t = board.Task(1, 2)\n  assert board.keep() == t.id\n}\n",
        );
        let store = project(&[("src/board.gleam", &board), ("test/board_test.gleam", tests)]);
        let out = extract(&store, &["board.Task", "--to", "task"]).unwrap();
        assert!(out.contains("Tests moved: board_test.round_trip_test -> task_test"), "{out}");
        assert!(store.resolve(&store.index().unwrap(), "task_test.round_trip_test").is_ok());
        assert!(text(&store, "board_test.mixed_test").contains("task.Task(1, 2)"));
        assert!(header(&store, "board_test").contains("import task"), "{}", header(&store, "board_test"));
        assert!(store.index().unwrap().modules.iter().any(|m| m.path == "task_test" && m.dir == "test"));
    }

    /// An emptied source module is retired, with its file; importers follow.
    #[test]
    fn an_emptied_source_is_retired() {
        let store = project(&[
            ("src/board.gleam", "pub fn one() -> Int {\n  1\n}\n"),
            ("src/other.gleam", "import board\n\npub fn two() -> Int {\n  board.one() + 1\n}\n"),
        ]);
        let out = extract(&store, &["board.one", "--to", "task"]).unwrap();
        assert!(out.contains("Retired module board"), "{out}");
        assert!(!store.root.join("src/board.gleam").exists() && store.root.join("src/task.gleam").exists());
        assert!(text(&store, "other.two").contains("task.one() + 1") && !header(&store, "other").contains("board"));
        assert_eq!(extract(&store, &["board.one", "--to", "task"]).unwrap(), "task.one already in task; nothing changed.\n");
    }

    /// A result that does not compile changes nothing.
    #[test]
    fn a_failed_type_check_leaves_the_store_unchanged() {
        // the target imports the source without using it, so the move closes a real import cycle
        let store = project(&[
            ("src/board.gleam", "pub fn a() -> Int {\n  1\n}\n\npub fn b() -> Int {\n  a()\n}\n"),
            ("src/task.gleam", "import board\n\npub fn z() -> Int {\n  0\n}\n"),
        ]);
        let err = extract(&store, &["board.a", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("Not moved: the result does not type-check, so nothing changed."), "{err}");
        let ix = store.index().unwrap();
        assert!(store.resolve(&ix, "board.a").is_ok() && store.resolve(&ix, "task.a").is_err());
        assert_eq!(text(&store, "board.b"), "pub fn b() -> Int {\n  a()\n}");
        assert!(store.root.join("src/board.gleam").exists());
        assert!(fs::read_to_string(store.root.join("src/board.gleam")).unwrap().contains("pub fn a()"));
    }

    /// Operand forms: a module in the list, tests and sources apart, a missing name.
    #[test]
    fn operands_are_checked() {
        let tests = "pub fn only_test() {\n  assert 1 == 1\n}\n";
        let store = project(&[("src/board.gleam", BOARD), ("test/board_test.gleam", tests)]);
        let err = extract(&store, &["board.gleam", "--to", "task"]).unwrap_err().0;
        assert_eq!(err, MODULE_HINT);
        let err = extract(&store, &["board", "--to", "task"]).unwrap_err().0;
        assert_eq!(err, MODULE_HINT);
        let err = extract(&store, &["board_test.only_test", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("test symbols go only to a test module"), "{err}");
        let err = extract(&store, &["board.lane", "--to", "task_test"]).unwrap_err().0;
        assert!(err.contains("source symbols do not go to a test module"), "{err}");
        let err = extract(&store, &["board.nothing", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E1001"), "{err}");
        assert!(extract(&store, &["board.lane"]).unwrap_err().0.contains("extract needs a target module"));
        assert!(extract(&store, &["board.lane", "--to", "task", "--to-project", "../x"]).unwrap_err().0.starts_with("E6007"));
        // a test symbol goes to a test module
        extract(&store, &["board_test.only_test", "--to", "test/task_test.gleam"]).unwrap();
        assert!(store.resolve(&store.index().unwrap(), "task_test.only_test").is_ok());
    }

    /// A suite entry that stops resolving, and a watched file that names the
    /// moved symbol, are listed.
    #[test]
    fn suites_and_watched_files_are_named() {
        let toml = "[commands]\nghost = { run = [\"make\"], checks = [\"board.main\"] }\n";
        let script = "gleam run -m board\nboard.main()\nunrelated\n";
        let store = project(&[("src/board.gleam", "pub fn main() {\n  Nil\n}\n"), ("solace.toml", toml), ("script.py", script)]);
        let out = extract(&store, &["board.main", "--to", "task"]).unwrap();
        assert!(out.contains("Not rewritten: suite ghost entry board.main no longer resolves"), "{out}");
        assert!(out.contains("Not rewritten: 2 places in 1 file (script.py:1, script.py:2)"), "{out}");
    }

    /// A target that lives in a connected project needs `--to-project`; a project
    /// that depends on this one and uses a moved symbol is named.
    #[test]
    fn other_projects_are_named() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import board\n\npub fn view(t: board.Task) -> Int {\n  t.id\n}\n"),
        ]);
        for d in ["web", "components", "front"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.lane", "--to", "solace_task"]).unwrap_err().0;
        assert_eq!(err, "solace_task is in components; add --to-project components");
        let err = extract(&store, &["board.lane", "--to", "components/solace_task"]).unwrap_err().0;
        assert_eq!(err, "use --to solace_task --to-project components");
        let err = extract(&store, &["board.Task", "--to", "task"]).unwrap_err().0;
        assert!(err.starts_with("E5003") && err.contains("front: front.view uses board.Task"), "{err}");
    }

    /// D-103: a dependent the command holds (one the task covers) is rewritten
    /// and gains the destination dependency; one it doesn't hold is refused.
    #[test]
    fn a_covered_dependent_is_rewritten() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import board\n\npub fn view(t: board.Task) -> Int {\n  board.lane(t)\n}\n"),
            ("web/solace.toml", "[commands]\nboard = { run = [\"true\"], checks = [\"board.bump\"] }\n"),
            ("front/solace.toml", "[commands]\nui = { run = [\"true\"], checks = [\"front.view\"] }\n"),
        ]);
        for d in ["web", "components", "front"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let args = ["board.Task", "board.new_task", "board.lane", "--to", "solace_task", "--to-project", "components"];
        assert!(extract(&store, &args).unwrap_err().0.starts_with("E5003"));
        let front = Store::at(&root.join("front")).unwrap();
        COMMAND_DB.with_borrow_mut(|c| c.push((front.root.clone(), front.db.clone())));
        let out = extract(&store, &args);
        COMMAND_DB.set(vec![]);
        let out = out.unwrap();
        assert!(out.contains("Dependency added: web, front -> components\n"), "{out}");
        assert!(out.contains("Rewrote 2 references in 1 module in front\n"), "{out}");
        // the suites that reach code now calling across packages, here and in the dependent
        assert!(out.contains("board (`run board`)") && out.contains("ui in front (`run ui --project front`)"), "{out}");
        assert_eq!(text(&front, "front.view"), "pub fn view(t: solace_task.Task) -> Int {\n  solace_task.lane(t)\n}");
        assert_eq!(header(&front, "front"), "import solace_task");
        assert_eq!(
            fs::read_to_string(root.join("front/gleam.toml")).unwrap(),
            concat!(
                "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "web = { path = \"../web\" }\ncomponents = { path = \"../components\" }\n",
            )
        );
    }

    /// Two projects under a root, `web` and `components`, each with its store.
    fn two_projects(components_toml: &str) -> Scratch {
        let tests = "import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n";
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\n\n[dev-dependencies]\n"),
            ("web/src/board.gleam", BOARD),
            ("web/test/board_test.gleam", tests),
            ("components/gleam.toml", components_toml),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        root
    }

    /// D-103: symbols move into another project with their identity; the source
    /// gains the dependency, the destination doesn't.
    #[test]
    fn a_move_into_another_project_adds_the_dependency_to_the_source() {
        let components = "name = \"components\"\nversion = \"1.0.0\"\n";
        let root = two_projects(components);
        let store = Store::at(&root.join("web")).unwrap();
        let ix = store.index().unwrap();
        let id = ix.syms[store.resolve(&ix, "board.Task").unwrap()].id.clone();
        let out = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert!(out.starts_with("Moved board.Task to solace_task in components\nDependency added: web -> components\n"), "{out}");
        assert!(out.contains("Tests moved: board_test.round_trip_test -> solace_task_test"), "{out}");
        assert_eq!(
            fs::read_to_string(root.join("web/gleam.toml")).unwrap(),
            "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n\n[dev-dependencies]\n"
        );
        assert_eq!(fs::read_to_string(root.join("components/gleam.toml")).unwrap(), components);
        let dest = Store::at(&root.join("components")).unwrap();
        let dix = dest.index().unwrap();
        assert_eq!(dix.syms[dest.resolve(&dix, "solace_task.Task").unwrap()].id, id);
        assert!(dest.resolve(&dix, "solace_task_test.round_trip_test").is_ok());
        assert!(store.resolve(&store.index().unwrap(), "board.Task").is_err());
        assert!(text(&store, "board.new_task").contains("-> solace_task.Task {"), "{}", text(&store, "board.new_task"));
        assert!(fs::read_to_string(root.join("components/src/solace_task.gleam")).unwrap().contains("pub type Task"));
        // run again: nothing to do
        let again = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert_eq!(again, "solace_task.Task already in solace_task; nothing changed.\n");
    }

    /// D-103: a destination that already depends on the source can't take code
    /// the source still uses: refused with every use, and nothing changes.
    #[test]
    fn a_package_cycle_is_refused_with_the_staying_uses() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n");
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.Task", "--to", "solace_task", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E5002 staying code uses 3 moved symbols and components already depends on web"), "{err}");
        assert!(err.contains("Task (by new_task)") && err.contains("Task (by bump)") && err.contains("Task (by lane)"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.Task").is_ok());
        assert!(!fs::read_to_string(root.join("web/gleam.toml")).unwrap().contains("components"));
        // a module name the source has is refused too
        let err = extract(&store, &["board.lane", "--to", "board", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E1004 module board is in this project too"), "{err}");
    }

    /// A module the destination already has, whose name the source keeps too,
    /// is refused before anything is written (Gleam module names are unique
    /// across connected packages).
    #[test]
    fn a_destination_module_named_like_one_the_source_keeps_is_refused() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", BOARD),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/board.gleam", "pub fn y() -> Int {\n  1\n}\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.lane", "--to", "board", "--to-project", "components"]).unwrap_err().0;
        assert!(err.starts_with("E1004 module board is in this project too and stays"), "{err}");
        assert!(store.resolve(&store.index().unwrap(), "board.lane").is_ok());
    }

    /// A destination that already depends on the source may take code that uses
    /// what stays, as long as nothing staying uses the moved code.
    #[test]
    fn a_destination_that_reaches_the_source_takes_code_that_uses_staying_symbols() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n");
        let store = Store::at(&root.join("web")).unwrap();
        let out = extract(&store, &["board.lane", "--to", "solace_task", "--to-project", "components"]).unwrap();
        assert!(out.starts_with("Moved board.lane to solace_task in components\n"), "{out}");
        assert!(!out.contains("Dependency added"), "{out}");
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(text(&dest, "solace_task.lane").contains("board.Task"), "{}", text(&dest, "solace_task.lane"));
        assert!(!fs::read_to_string(root.join("web/gleam.toml")).unwrap().contains("components"));
    }

    /// Emptying a source module across projects retires it, and the source
    /// gains the dependency on the destination.
    #[test]
    fn emptying_a_module_into_another_project_retires_it() {
        let root = two_projects("name = \"components\"\nversion = \"1.0.0\"\n");
        let store = Store::at(&root.join("web")).unwrap();
        let out = extract(
            &store,
            &["board.Task", "board.new_task", "board.bump", "board.lane", "--to", "solace_task", "--to-project", "components"],
        )
        .unwrap();
        assert!(out.contains("Retired module board"), "{out}");
        // the test goes too, so nothing left in web uses components
        assert!(!out.contains("Dependency added"), "{out}");
        assert_eq!(
            fs::read_to_string(root.join("web/gleam.toml")).unwrap(),
            "name = \"web\"\nversion = \"1.0.0\"\n\n[dependencies]\n\n[dev-dependencies]\n"
        );
        let ix = store.index().unwrap();
        assert!(ix.modules.iter().all(|m| m.path != "board"), "board should be gone");
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(dest.resolve(&dest.index().unwrap(), "solace_task.bump").is_ok());
    }

    #[test]
    fn helpers_for_paths() {
        assert_eq!(ffi::relative("solace-cli/board", "solace-cli/components"), "../components");
        assert_eq!(ffi::relative(".", "a/b"), "a/b");
        assert_eq!(add_dependency("name = \"a\"\n", "b = 1"), "name = \"a\"\n\n[dependencies]\nb = 1\n");
        assert_eq!(
            add_dependency("[dependencies]\nx = 1\n\n[dev-dependencies]\ny = 2\n", "b = 1"),
            "[dependencies]\nx = 1\nb = 1\n\n[dev-dependencies]\ny = 2\n"
        );
        assert_eq!(item_split("type Task"), ("Task".to_string(), "Task".to_string()));
        assert_eq!(item_split("Foo as Bar"), ("Foo".to_string(), "Bar".to_string()));
        assert_eq!(make_pub("// doc\nfn f() {\n  1\n}"), "// doc\npub fn f() {\n  1\n}");
    }

    /// A failure keeps its message and code; a failed put-back is added to it.
    #[test]
    fn a_failed_put_back_is_named() {
        let e = || Failure("E6002 x: no file there".into(), 1);
        assert_eq!(with_put_back(e(), Ok(())).0, "E6002 x: no file there");
        let both = with_put_back(e(), Err(Failure("disk full".into(), 1)));
        assert_eq!((both.0.as_str(), both.1), ("E6002 x: no file there\n\nPutting back also failed: disk full", 1));
    }

    const ABS: &str =
        "@external(erlang, \"erlang\", \"abs\")\n@external(javascript, \"./board_ffi.mjs\", \"abs\")\npub fn abs(x: Int) -> Int\n";

    /// In one project the file stays and a move to another folder rewrites the
    /// path; a JavaScript import of moved code is named, and so is the suite
    /// that reaches it, or the lack of one.
    #[test]
    fn a_move_to_another_folder_rewrites_ffi_paths() {
        let files = [
            ("src/board.gleam", ABS),
            ("src/board_ffi.mjs", "export function abs(x) { return Math.abs(x) }\n"),
            ("src/app.mjs", "import { abs } from \"./board.mjs\";\n"),
        ];
        let store = project(&files);
        let out = extract(&store, &["board.abs", "--to", "ui/task"]).unwrap();
        assert!(out.contains("Rewrote FFI paths: board.abs ./board_ffi.mjs -> ../board_ffi.mjs\n"), "{out}");
        assert!(out.contains("no declared suite reaches the moved code"), "{out}");
        assert!(out.contains("Not rewritten: JavaScript import of moved code from its old module (src/app.mjs:1)"), "{out}");
        let abs = text(&store, "ui/task.abs");
        assert!(
            abs.contains("@external(javascript, \"../board_ffi.mjs\", \"abs\")") && abs.contains("@external(erlang, \"erlang\", \"abs\")"),
            "{abs}"
        );
        assert!(store.root.join("src/board_ffi.mjs").exists());
        // a suite that checks everything is named instead
        let toml = "[commands]\njs = { run = [\"true\"], checks = [\"*\"] }\n";
        let store = project(&[files[0], files[1], ("solace.toml", toml)]);
        let out = extract(&store, &["board.abs", "--to", "ui/task"]).unwrap();
        assert!(out.contains("Also reached: js (`run js`)\n") && !out.contains("no declared suite"), "{out}");
    }

    /// Into another project the file goes along, beside the module that uses
    /// it, with its imports rewritten; refused, naming them, when the
    /// destination has the file, staying code uses it, another file imports it,
    /// or it imports what stays.
    #[test]
    fn a_move_into_another_project_carries_ffi_files() {
        let neg =
            "\n@external(erlang, \"erlang\", \"-\")\n@external(javascript, \"./board_ffi.mjs\", \"neg\")\npub fn neg(x: Int) -> Int\n";
        let ffi = "import { Ok } from \"./gleam.mjs\";\nimport { h } from \"./util.mjs\";\nexport function abs(x) { return Math.abs(x) }\n";
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"components\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", &format!("{ABS}{neg}")),
            ("web/src/board_ffi.mjs", ffi),
            ("web/src/util.mjs", "export const h = 1;\n"),
            ("web/src/other.mjs", "import { abs } from \"./board_ffi.mjs\";\n"),
            ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
            ("components/src/solace_task.gleam", "pub fn z() -> Int {\n  0\n}\n"),
            ("components/src/ui/board_ffi.mjs", "taken\n"),
        ]);
        for d in ["web", "components"] {
            import(&root.join(d)).unwrap();
        }
        let store = Store::at(&root.join("web")).unwrap();
        let go = |names: &[&str]| {
            let mut args = names.to_vec();
            args.extend(["--to", "ui/task", "--to-project", "components"]);
            extract(&store, &args)
        };
        let err = go(&["board.abs"]).unwrap_err().0;
        assert!(err.starts_with("E5007 components: src/ui/board_ffi.mjs already exists"), "{err}");
        fs::remove_file(root.join("components/src/ui/board_ffi.mjs")).unwrap();
        let err = go(&["board.abs"]).unwrap_err().0;
        assert!(err.starts_with("E5007 staying code uses a JavaScript file") && err.contains("board.neg (./board_ffi.mjs)"), "{err}");
        let err = go(&["board.abs", "board.neg"]).unwrap_err().0;
        assert!(err.contains("src/other.mjs:1 imports ./board_ffi.mjs"), "{err}");
        fs::remove_file(root.join("web/src/other.mjs")).unwrap();
        let err = go(&["board.abs", "board.neg"]).unwrap_err().0;
        assert!(err.contains("imports what stays") && err.contains("src/board_ffi.mjs:2 imports ./util.mjs"), "{err}");
        assert!(root.join("web/src/board_ffi.mjs").exists() && !root.join("components/src/ui/board_ffi.mjs").exists());
        let ffi = ffi.replace("import { h } from \"./util.mjs\";\n", "");
        fs::write(root.join("web/src/board_ffi.mjs"), &ffi).unwrap();
        let out = go(&["board.abs", "board.neg"]).unwrap();
        assert!(out.contains("Moved file src/board_ffi.mjs to components: src/ui/board_ffi.mjs (its imports rewritten)\n"), "{out}");
        assert!(!out.contains("Rewrote FFI paths"), "{out}");
        assert!(!root.join("web/src/board_ffi.mjs").exists());
        let carried = fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap();
        assert_eq!(carried, ffi.replace("\"./gleam.mjs\"", "\"../gleam.mjs\""));
        let dest = Store::at(&root.join("components")).unwrap();
        assert!(text(&dest, "ui/task.abs").contains("\"./board_ffi.mjs\""));
        // a carry that isn't kept puts the file back as it was
        let end = lockdown::carry_file(&dest.root, "src/ui/board_ffi.mjs", &store.root, "src/x.mjs", Some("changed".into())).unwrap();
        assert_eq!(fs::read_to_string(root.join("web/src/x.mjs")).unwrap(), "changed");
        end(false).unwrap();
        assert!(!root.join("web/src/x.mjs").exists());
        assert_eq!(fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap(), carried);
        // and takes away the folders it made for it, not those that were there
        let end = lockdown::carry_file(&dest.root, "src/ui/board_ffi.mjs", &store.root, "src/a/b/x.mjs", None).unwrap();
        assert!(root.join("web/src/a/b/x.mjs").exists());
        end(false).unwrap();
        assert!(!root.join("web/src/a").exists() && root.join("web/src").is_dir());
        assert_eq!(fs::read_to_string(root.join("components/src/ui/board_ffi.mjs")).unwrap(), carried);
    }

    /// A dependent project whose store will not open is an error, not a skip.
    #[test]
    fn a_dependent_store_that_will_not_open_is_reported() {
        let root = Scratch::new().with(&[
            ("solace.toml", "projects = [\"web\", \"front\"]\n"),
            ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
            ("web/src/board.gleam", BOARD),
            ("front/gleam.toml", "name = \"front\"\nversion = \"1.0.0\"\n\n[dependencies]\nweb = { path = \"../web\" }\n"),
            ("front/src/front.gleam", "import board\n\npub fn view(t: board.Task) -> Int {\n  t.id\n}\n"),
            ("front/.solace/store.db", "not a database"),
        ]);
        import(&root.join("web")).unwrap();
        let store = Store::at(&root.join("web")).unwrap();
        let err = extract(&store, &["board.Task", "--to", "task"]).unwrap_err().0;
        assert!(err.contains("front") && err.contains("database"), "{err}");
    }
}
