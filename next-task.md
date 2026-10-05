I reviewed the Rust core (solace-cli/src, about 13k lines, plus its tests) and changed no code. I left out the Gleam projects under board/, migration/ and frontend/. All 69 tests pass (65 unit tests and 4 Python acceptance suites).

How I measured. I ran clippy (default lints plus some stricter ones), cargo fmt --check, and LLVM coverage. The acceptance tests stop the server with SIGTERM, so a normal coverage run never records the server's code. I used LLVM's continuous mode to capture it, which moved board.rs from 51% to 92%. Overall line coverage is 89.8%.

Note: another session edited gleam.rs, main.rs, reach.rs, view.rs and several docs while I worked (the D-65 signature/head split). The line numbers below match the files as they are now. Coverage was measured on the version just before those edits.

---

1. Bugs and inconsistencies

#: 1
Where: solace-cli/src/view.rs:698
What's wrong: Confirmed bug. scratch() copies a file only if its tier starts with '2' or '3'. Commit b155698 renamed the
tiers to words (watched, …) and missed this line. Since then, POST /tests review runs have left out every watched
non-Gleam file (data, fixtures in priv/), and the whole block is dead. The fix is tier == "watched". Also, scratch is
only ever called with Version::Current, so its03-731) can never run.
────────────────────────────────────────
#: 2                                                                                                                       Where: drafts.rs:87-94 vs lockdown.rs:342-347
What's wrong: The same error codes mean different things. In drafts, E0104 means "old is empty" (exit code 2). In lockdown,                                                                                                                  E0104 also means "not found" (exit code 1), anbers or --all. edited() is a weaker copy of
drafts::replace().
────────────────────────────────────────                                                                                   #: 3
Where: view.rs:275 vs reach.rs:8
What's wrong: Two definitions of "a test". The graph counts any *_test function in a test module, including private
helpers. reach::tests and test require pub fn (which matches what gleeunit runs).
────────────────────────────────────────
#: 4
Where: main.rs:126                                                                                                         What's wrong: solace mcp --budget abc panics irror.
────────────────────────────────────────                                                                                   #: 5
Where: main.rs:111-123
What's wrong: --option or --old as the last argument is silently left in the argument list.
────────────────────────────────────────
#: 6
Where: workspace.rs:305 vs lockdown.rs:380                                                                                 What's wrong: Two different package-name rules passes Solace's check with name = "1abc" or "_x"; new_project would refuse both.
────────────────────────────────────────
#: 7
Where: Several files                                                                                                       What's wrong: The lists of which commands chan host-only don't agree with each other (READS
main.rs:249, GLEAM :373, changing :218, agent_guard board.rs:1856, plus the absorb and verification lists). For example,
build is snapshotted as a change but allowed in read-only runs.                                                            ────────────────────────────────────────
#: 8
Where: board.rs:1464                                                                                                       What's wrong: continuation_brief does a .replaef()'s sentence (:1615). If someone rewords
brief, the replace silently stops doing anything.
────────────────────────────────────────
#: 9
Where: main.rs:3106
What's wrong: names.dedup() only removes neighbouring duplicates. If errors come back in the order A, B, A, the report says                                                                                                                       "Broken symbols (3)". The same pattern is in mt; I haven't seen it happen.
────────────────────────────────────────                                                                                   #: 10
Where: main.rs:2684
What's wrong: The doc comment for missing_imports sits above fn_body_start instead (missing_imports is at :2827).
────────────────────────────────────────
#: 11
Where: All 11 source files                                                                                                 What's wrong: There's no rustfmt.toml, and car file. view.rs mixes default-style formatting with packed one-liners (:626-634, :728-771).
────────────────────────────────────────
#: 12
Where: Several files                                                                                                       What's wrong: new_id(Kind::Fn, …) is used as a draft ids come out as fn:…, and
migrate.rs:255,340 strip the prefix by hand even though board::id() already does that.
                                                                                                                           2. Untested code (from coverage)
                                                                                                                           The biggest gaps:

┌───────────────────────┬─────────────────────────────────────────────────────────────────────────────────────────────┐
│         Area          │                                       What never runs                                       │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ Whole-module and      │ Store::submit from main.rs:1709 to 1801: everything past the "missing symbols" check,       │    │ --imports submit      │ including the commity E0102 error.                             │ ├───────────────────────┼────────────────────────────────────────────────────────────────┤
│ impact command        │ All of Store::impact (main.rs:1596-1627). Only reach::report has a unit test.               │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ work add / list /     │ work_add, work_list, work_done, the open-work part of status, and work_context.             │    │ done                  │                                                                │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ Automatic imports     │ The retry loop in submit_symbols (:1988-2004), the unimported_name lookup in the project    │    │ after a type error    │ (:2928-2942), and meing import (:2894-2903).                   │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ Review tests          │ view::scratch and run_tests (POST /tests) never run, which is why bug #1 went unnoticed.    │    │                       │ GET /nodes/<id> with                                           │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ The MCP server loop   │ initialize, tools/list, several commands in one call (==== headers), the "looks like code"  │
│ in main.rs            │ refusal, and the empty-command reply.                                                       │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ Changes made outside  │ absorb during a running agent's run (outside.rs:365-381), deleted files (:282, :305,        │
│ Solace                │ read_in :193-198), non-UTF-8 files, and Mode::Command.                                      │    ├───────────────────────┼────────────────────────────────────────────────────────────────┤
│ Lockdown (file        │ read --lines a..b (lockdown.rs:191-195) and trim_output when output is over budget          │    │ commands)             │ (:280-302).                                                    │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│ reach                 │ refs Type.field (field_refs), and suites (the checks setting on declared commands).         │
├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────┤
│                       │ Restoring modules on revert (restore :626-635), the server-restart recovery in              │
│ board                 │ recover_project, GET /tasks/<id>/runs/<rid>, move_task's rename, and most error branches in │    │                       │  workflow and parse_                                           │ ├───────────────────────┼────────────────────────────────────────────────────────────────┤
│ Errors                │ E1002 (ambiguous name), E5001 (delete of a symbol still in use), E1004, E0101 variants, and │
│                       │  the swap-failure paths in forward / reverse / undo in migrate.                             │
└───────────────────────┴─────────────────────────────────────────────────────────────────────────────────────────────┘   
3. Weak or redundant tests
                                                                                                                           - Oversized scenario tests. submit_new_createscross submit, probe, trace and weak asserts)and rejected_text_survives_and_retries_atomically (which ends with unrelated check --warnings asserts) stop at the first failure and hide everything after it.                                                                                    - Unrelated asserts tucked into other tests. lide inspect_dedups_and_fits_budget, and alockdown::tier check is inside packet_quotes_evidence_and_reproduction.
- Wrong doc comment. main.rs:3189 has a "D-14 multi-name inspect" comment on agent_reference_leaves_out_host_and_task_commands.
- Copy-pasted comment and cleanup. All four outside.rs tests carry "Completing a task cleans up its conversation…", even the ones that complete no task. They clean up with let _ = remove_dir_all.
- Temp folders leak when a test fails. No test uses a drop guard. About 12 tests repeat the same five lines of project       setup, and there are already four near-identutside::project, lockdown::fixture,workspace::scratch).                                                                                                     - Unit tests depend on the bench. Four of themnspect_dedups_and_fits_budget,test_runs_what_reaches_a_change, c2_pilot_date_check, cli_fixture_test_states), so a change to the bench can break core tests.
- Redundant pair. cli_fixture_test_states runs a real gleam test (slow) just to check failures() parsing, which test_failure_blocks already covers. Better to keep its output as a golden file.
- Testing Gleam, not Solace. a6_manifest_validation ends by asserting that Gleam rejects a missing path dependency.        - Repetition and naming. gleam.rs tests repeatlter five times. Test names mix c1, c2, c4, c3   order, a1…a10, and plain sentences, and therule in lockdown for no clear reason.

4. Imports
                                                                                                                           rustc and clippy report no unused imports. The use serde_json::{Value, json} is repeated in 7 modules (one top-level import in main.rs would reach them through use super::*), and main.rs:2419 writes out std::collections::BTreeMap in full. Separately, the explicit busy_timeout(5s) calls (board.rs:21, main.rs:332) do nothing, because rusqlite already sets 5 s by default.

Clippy's default lints give 20 warnings: 9 collapsible ifs, 7 overly complex types, too_many_arguments on
propose_settings, map_entry, question_mark and add 37 auto-fixable ones, such as unnecessarycollect() calls (main.rs:2094, :2897) and resolve_module's unused &self.

5. Repeated code worth extracting (biggest payoff first)

1. scope::path(top, dir) already exists, but the same if dir == "." { root } else { root.join(dir) } is written out 7 more times: main.rs:325, workspace.rs:246, migrate.rs:60, and board.rs:2050, :2192, :2208. The same logic appears again in
   other_projects.
2. Adding or retiring a symbol in the store (insert, ord, history entry) is written 4 times (import, submit,
   submit_symbols, outside::read_in). Retiring these Store::insert_symbol and Store::retire.
3. submit and submit_symbols share the transaction handling (BEGIN, closure, COMMIT or ROLLBACK and rewrite files), the "Not saved…" message and the notes. Also, the Target::Imports and Target::Module branches at main.rs:1683-1688 are identical, and the (name, is_type) key closure is defined 3 times.
4. Opening a lock file with OpenOptions::new().create(true).truncate(false).write(true)… is written 7 times (in board.rs and migrate.rs). One lock_file(path, wait) would do.
5. Running a process with a timeout is written migrate::run, both with the same
   kill-the-process-group loop.
6. Reverse references. (0..n).filter(|j| ix.uses[j].contains(&i)) appears 7 or more times, and reach::flow builds its own callers. Store used_by on Index once.
7. Helpers for task JSON in board.rs: finding a run by id (about 8 times), ending a run (3 times: end_runs, finish,
   task_command), creating a task (twice, once the Unexpected Changes task), project_of(written out again in outside.rs:330 and view.rs:403), and turning a JSON byte array back into bytes (4 times).
8. Small duplicates:
   - percent_decode: one copy in view.rs and o
   - Stripping quotes from a query: once in find, once in grep.
   - The yes/no prompt: once in migrate, once in workspace.
   - The E6020 error builder: 4 versions.
   - Writing a file atomically (temp file, then rename): 3 copies.
   - The task's before/<sha> path: 4 copies.
   - The store fingerprint hash: 4 copies.
   - Plural helpers: 6 or more.
   - Reading the project name from gleam.toml: view::graph repeats workspace::name.
   - The "defines twice" check: once in outsid
   - aliases() exists, but missing_imports writes it out again.
   - The end of a session reply: written in alnd session_task.
   - Finding symbols named in free text: once in work_context, once in session_task.
9. Two text-scanning jobs are written twice each:
   - Root identifiers: reach::roots and weak_asserts's own roots.
   - Finding where a function body starts: fn_body_start (tracks [ as well) and the Kind::Fn cut in gleam::head (doesn't),
     so they can disagree.

6. Recommended next steps, in order

1. Fix view.rs:698 and add a POST /tests accepe the test reads. Delete the deadVersion::Before branches in scratch.
2. One table of commands (name → reads / chang allowed in read-only runs), used byrun_options, run_inner, agent_guard, READS and GLEAM.
3. Fill the coverage gaps that touch your data: whole-module and --imports submit, impact, work, the automatic-import
   retry, absorb during a run, and revert rest
4. Unify the duplicated helpers: E0104/E0105 (place), the test definition (reach::testseverywhere), the package-name rule, and items 1, 2, 4 and 5 in §5.
5. Test fixtures: one TempProject helper with a drop guard, a small fixture project in tests/fixtures instead of ../bench/pilot-large, and split up the oversized tests.
6. Formatting: add rustfmt.toml (max_width = 1ax") and format everything in one commit. Thenclear the default clippy warnings.
7. Later: typed structs for the snapshot row (now read by position, row[1] and row[4], in 5 places) and for tasks. Every .as_array().unwrap() on task JSON can curre a task file is malformed.
