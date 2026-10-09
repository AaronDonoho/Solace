
## D-82 Blocked is not a column; a held or stopped agent is attention
Amends Spec 15 §1, §2, §3 and §6 (Aaron, 2026-10-08).
1. The default workflow has no `blocked` column. `asked` and `failed` have no target: the task stays in its column (normally `assigned`) and gets `attention`. Answering the last question continues the agent directly (D-88). A workflow may still name targets for `asked` and `failed`; only `finished` must have one.
2. Attention from a held question (work or investigation), a settings proposal, or a failed or restarted work run carries `hold: true`. It stays through other actions and clears when the run continues (an answer, `investigate`), is restarted (`new_agent`, `start_over`, `reassign`), is stopped, or the task reaches a final column. Any other attention clears on the next action, as before.
3. A task saved in `blocked`, under a workflow without that column, loads in `assigned` with held attention (its own, or "the agent needed an answer or stopped").
4. `blocked` is reserved for a task that depends on another task. Nothing sets it yet and the task has no field for it.
Why: every known case of "blocked" is an agent needing a human, which attention already says; a column for it moved the card away from the work it belongs to.

## D-88 Answering continues the agent; there is no Unblock
Amends D-30, D-82 and Spec 15 §2, §6 (Aaron, 2026-10-09).
1. Answering a waiting agent's last open question hands the answers straight to its run, in any column and any workflow. No workflow action is looked up, so none is guessed, moved, stamped or replaced. A paused task waits for `resume`.
2. The default workflow has no `unblock`. After a failed or cut-off work run, `assigned` offers `reassign`, which brings back the previous conversation when it is compatible (D-33) and otherwise starts a fresh agent with the notes. `new_agent` always starts fresh.
Why: `unblock` did nothing a human could see, yet answering secretly pressed it; removing it from a workflow silently stopped answers from reaching the agent.

## D-83 Tests are short and bounded
(Aaron, 2026-10-08.)
1. A test is kept short. A full suite that takes long to run is a defect to fix, not to wait out: a slow test is shortened, or split, when it is found.
2. Every wait in a test (a loop, a join, a poll for a file or a reply) has a deadline and fails with a message when it passes. No test can hang the run.
3. Tests don't share process-wide state: a global such as the task server acts only for the root it serves, so tests running in parallel can't see each other's.
Why: a `tasks::` run timed out because one test's global server made other tests wait forever; the whole run, not just that test, was lost.

## D-84 Agents move files and folders
Amends Spec 14 §3 (Aaron, 2026-10-08).
1. `move <from> <to> [--to-project <folder>]` moves a watched or unwatched file, or a folder, within a project or into another project the task's scope covers (`--project` picks the source's, as for any command). Outside a board run, a move stays in one project.
2. Graph files (a projection of the store), protected files, internal files, `gleam.toml` and symlinks never move, named directly or inside a folder.
3. A move never replaces anything: when something exists at the destination, it is refused (`E6002`) and nothing changes.
4. Atomic: everything is checked before anything changes, and the move is one rename that refuses to replace (`renameat2` with `RENAME_NOREPLACE`). Undo records go first, so a crash after the rename still records the move. A move between file systems is refused, since it can't be one step.
5. Idempotent: repeating a move the session already made, with the source gone and the destination there, changes nothing and says so.
6. The task records each moved file as removed at the source and added at the destination, in each side's project; revert and cancel put it back.
Why: agents had to rewrite a file and remove the original to move it, which loses binary and unwatched files and is not atomic; the human asked for moves within the task's scope that never overwrite.

## D-85 The contract view is its own app
Amends D-62 (Aaron, 2026-10-08).
1. The contract view is its own Gleam project, `solace-cli/contracts` (package `contracts`, JavaScript target), like the migration page. Its component is `<solace-contracts>`, with the optional `base` attribute it had as `<solace-view>`.
2. The shell depends on it by path and registers it. The page is `/contracts`, and `/p/<dir>/contracts` for a workspace project; both render `<solace-contracts>` (with `base` for a project) below the shell's menu bar. `/view` is gone: the server serves the shell at `/contracts`, the board's "Contract view" links and a task's review link (`/contracts?task=<id>`) point there, and an unknown path such as `/view` shows the board. Amends Spec 16's `GET /view`.
3. The menu bar on every page has a **Contracts** link (`/contracts`), marked current on any contract page, including a project's. The menu is 56px high; the contract page fills the rest of the window through `--contracts-height`.
4. The board project holds only the board. The contract view's FFI (`layout.mjs`, its part of `api.mjs`) and the dagre dependency live in `contracts`; the shell keeps dagre because the FFI is copied into its build.
5. Saved browser state keeps its keys (`solace-view-positions:v2:…`, `solace-view-panel-width`), so node positions and the panel width survive the move.
Why: each page is its own app with its own build and tests, as the migration page already is; the tag names the app it belongs to.

## D-86 The shell finds text in the page
(Aaron, 2026-10-08.)
1. Ctrl+F (Cmd+F) opens a find bar on every page, also from a text box; the shell prevents the browser's own find. Enter and Shift+Enter step through matches with wrap-around and centre the match in each scroll area and the page where there is room, when it isn't already fully in view. A new query starts at the first match in view, else the first below it. Visually hidden text (1px `.sr-only` labels) doesn't count. Esc closes from anywhere in the bar (or the page when nothing has focus) and returns focus to the element inside the component that had it. A live re-search keeps the current match when its text is still there; opening or closing a `<details>` re-searches. The bar shows "n of m" or "No matches".
2. The search walks the document and every open shadow root, case-insensitively, skipping `style`/`script`/`template` text and text with no boxes on screen. A match lies within one text node; a phrase split across elements is not found. At most 1000 matches.
3. Matches are marked by boxes drawn over the text from `range.getClientRects()`, in a fixed layer with `pointer-events: none`, never by changing the DOM Lustre renders and never by changing `user-select`: cards stay unselectable and draggable. Boxes are clipped to every scroll area around the match, skipped where something else covers the text (a sticky lane header, a modal, a notice, the find bar), and redrawn on scroll (captured on the document and each shadow root, since scroll events don't cross shadow boundaries), resize and re-search. Not the CSS Custom Highlight API: WebKit doesn't paint it on `user-select: none` text (WebKit bug 278455).
4. The bar and the marks layer are plain DOM owned by `shell.mjs`, outside `#app`; the decisions (`find_opens`, `next_match`, `count_text`) live in `frontend.gleam` and are tested. Changes inside a component's shadow root re-run the search (debounced); the bar's and the marks' own changes don't.
Why: Tauri's webview has no find, and native find in WebKit misses shadow DOM, where every page lives.

## D-87 Features and milestones are root records kept by the task server
Amends Spec 15 §1 and §4 (Aaron, 2026-10-09).
1. A **milestone** is a planned release. A **feature** is a capability of the product. Both are records at the Solace root, kept by the task server, which is their only writer. They are not tasks (D-25) and have no column, runs, scope or workflow.
2. Links are optional. A task has at most one feature and at most one milestone. A feature has an optional *target* milestone. A task given a feature with no milestone of its own takes the feature's milestone; after that the two can differ. A feature appears in every milestone that holds one of its tasks.
3. Tasks gain `feature`, `milestone`, `dependsOn` (task ids), `closedTime` (when the task first reached a final column; never cleared) and `archivedTime` (when the task was archived; null otherwise). How a task is archived, and how the board hides it, is decided later.
4. `feature` and `milestone` can be changed on a task in a final column. Other edits there stay refused (`E7004`).
5. Deleting a feature or milestone clears every link to it in the same operation. It never deletes tasks.
6. `dependsOn` is recorded and validated (existing tasks, no self-reference, no cycles), but nothing enforces it yet. D-82 point 4 keeps `blocked` for its later use.
7. Their events go on the root's stream only, with no `project` field; `/p/{dir}/events` leaves them out.
8. Milestones and features share one list of states: `proposed | planned | active | complete | canceled`. A new record starts `proposed` unless its create names a state. Any state can change to any other. A `canceled` record keeps its links to tasks; only a delete clears them.
Why: the Feature/Milestone store and the Strategy page need a settled model to be built and reviewed against.

## D-88 A task's feature and milestone can change while a run is going
Amends D-87 point 4 and Spec 15 §4 (Aaron, 2026-10-09).
1. `PATCH /tasks/{id}` with only `feature` and `milestone` is accepted while a run is going, as in a final column. Any other field during a run is still refused (`E7003`), `dependsOn` included.
Why: the links are for planning and don't affect the agent; a long run or an unanswered question shouldn't hold planning up.
