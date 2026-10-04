# Spec 16: Contract View

**Status:** accepted 2026-09-30 (D-35) · **Implements:** D-16 (point 4), D-20 (points 2, 3), D-28 and D-52 (file categories), D-35, PROPOSAL.md §7 · **Needed by:** the D-16 human UX. Not in the PROPOSAL.md §14.4 table; numbered after it.

The contract view is how a human sees and checks a project in Solace. A human does not read source code there. They see what the system does and what it touches: its entry points, its logic, its side effects, its tests and its files. They can run any part of it with data they choose and see or hear what comes out. When an agent finishes a task, the review starts here: what the task changed in those terms, whether its tests pass, and whether the human's earlier runs now give different results.

Humans give creative input and judgement; agents plan, code, debug and write tests (D-20). So the view shows data in, data out and effects, never how the code got there, and a human's judgement on a run becomes a task for an agent, not test code the human owns.

This first version covers less than PROPOSAL.md §7. Guarantees with evidence levels (§7.5) and the program's decisions in business terms wait for the Solace language, because plain Gleam has no declared contracts or effects. Everything here is either derived from the project or seen in a run, and is marked with how it was obtained.

"Gleam:" marks what applies to a Gleam project. The view itself is language-neutral (D-16.3): it shows a graph and doesn't parse code. A language adapter produces the graph from Solace's store (D-35.1).

## 1. The graph

| Node kind | What it is | Gleam |
| --- | --- | --- |
| `entry` | A way into the system | `pub fn main`, and every public function that no source function calls (in `pilot-large`, `api.handle`) |
| `logic` | A unit of behavior: a function | Each public or reached function symbol |
| `effect` | Something the system does to the outside world: files, printing, network, a database, the clock, randomness, the environment, another process | Calls into a table of known effectful functions (`gleam/io`, `simplifile`, `gleam/erlang/os`, …) and every `@external` |
| `test` | A test, written by an agent | Each `*_test` function in `test/` |
| `file` | A project file that isn't source, with its category | Every file from `solace files` except graph and internal ones |

| Edge | Meaning |
| --- | --- |
| `calls` | Logic reaches other logic |
| `uses` | Logic causes an effect, directly or through what it calls |
| `tests` | A test references this node (D-36.3) |
| `reads`, `writes` | An effect touches a file, when that can be told |

Every node and edge says how it is known: `declared` (written in the code, as the Solace language will allow), `derived` (found by reading the code) or `observed` (seen in a run). A derived effect is a claim the adapter makes, and the view doesn't present it as a guarantee.

A node's detail panel shows its name, kind, plain-language summary (the symbol's description, if any), signature in the target's types and connections. A function also has a **Run** button (§3). A test shows its checks as data in → data out (D-35.5): Gleam: each `assert <call> == <expected>` as one line, "`<call>` → `<expected>`"; any other `assert` as written. Source text is one click further, on request (PROPOSAL.md §7.3, item 5).

## 2. What a human can do

- **Overview.** The graph is a drawn diagram: a layered layout with pan and zoom, nodes coloured by kind (Aaron, 2026-09-30). It opens on entry points and effects, with the logic between them collapsed. Colours mark node kinds. Badges mark nodes with no tests, and nodes changed by an open task.
- **Search and follow.** A human can find any node by name or summary, then expand its callers, callees, effects and tests.
- **Moving around (D-53).** Dragging the sheet, from anywhere including a node, pans it and selects nothing. Opening a node doesn't move the sheet unless the node is out of view or under the panel, and then only as far as brings it into view. Keys: arrows or WASD move between nodes, with Shift they pan, Ctrl+Up/Down zoom, Space expands a node, C collapses it, Enter opens it. A changed node has a plain border in the revision colour.
- **Files and categories.** A files panel lists every non-source file with its category and, in words, what agents may do with it (D-28, D-52). Example: "Watched: agents can read and write it; changes are undone with the task."
- **Run.** A human runs any public function with data they enter, in one step or several (§3).
- **Make a task from a run.** When a run is wrong, or right and worth keeping as a test, the human says what it should do, and Solace puts a task on the board (§3).
- **Review.** A task's changes, test results and replayed runs (§4).

## 3. Runs

**Steps.** A run is a list of steps. Each step calls one public function. Its inputs are entered through forms built from the parameter types, which Solace has in its store:

- `Int`, `Float`, `String`, `Bool`: a field;
- a custom type: a choice of its constructors, then a form for the chosen one's fields (Gleam: `Request` offers `CreateOrder`, `AddItem`, …; `AddItem` asks for `order_id`, `sku` and `quantity`);
- a list, tuple, `Dict` or `Set`: a form per item or entry;
- any type, including function types and opaque types: an earlier step's result, a field or tuple element of it, or the result of a public function returning that type, itself run with a form (Gleam: a `Store` from `seed.demo_store()`, a `Clock` from `clock.fixed(at)`).

A run may also give command-line arguments and standard input, which a `main` needs.

**Where it runs.** In a scratch copy of the project at the chosen version: the current one, or the one before a given task (Spec 15 §5). Solace generates the calling code in the copy, runs it, and reads each result back as a value the view can show. Nothing runs in the project itself, so a run can't change real files. Before a run, the view lists the effects derived for the functions it calls. Effects that leave the scratch copy (network, other processes) need the human's confirmation.

**What it shows.** For each step:

- its inputs and result, records as fields;
- when the result, one of its tuple elements or an `Ok` payload has the same type as an input (Gleam: `Store` in, `Store` out), what changed between them, as before and after;
- printed output;
- files created, changed or removed in the copy, shown by type with the browser's own elements (text, images, audio, video, HTML, PDF) and anything else as a download;
- a crash with its message. Reaching a `todo` shows the step as *unfinished* (D-36.2).

Sound or windows a program produces directly appear on the human's machine, because the server is local.

**Remembered.** Every run's steps are kept per project, can be listed, and can be run again.

**Make a task.** From a run, the human writes what it should have done. Solace creates a task on the board (Spec 15) whose description carries the steps, the results the program gave and the human's expectation. The agent writes the test. Because the run was on the old code, a bug report made this way meets D-36.5 by construction.

## 4. Review

A task in *needs review* (or any task with changes) shows a one-line summary on the board, such as "1 effects added, 0 signature changed, 0 test unfinished, 3 untested uses", linking to the view filtered to that task. The line sits in the task's detail panel. The filtered view shows:

1. entry points and effects that were added, removed or changed. A new effect is flagged. Example: "`main` now writes files";
2. logic whose signature changed;
3. the task's tests (tests the task changed, plus tests that reach a symbol it changed or read a record field its change flows into, D-44), run by Solace at review time, each *unfinished*, *wrong* or *passing* (D-36.3). A changed expected value shows as "→ new, was old". D-36's flags and their resolutions appear here once D-36 is built;
4. uses the change reaches that no test checks (D-36.3 coverage: what a test references, also through helpers in its own module), nearest first, each with how it is reached and whether that is certain or possible; plus the declared suites that reach the change (Spec 14 §2) and where its data leaves the program. The summary counts them ("3 untested uses") (D-44);
5. remembered runs that reach a changed symbol through `calls` edges, replayed on the version before the task and on the current one. Results that differ are shown side by side for the human to judge; a difference is not a failure;
6. non-source files the task touched, with their categories;
7. source text, only on request; for a symbol the task changed, added or removed, as a line diff from before the task (D-53).

The board's *Changes* list links each entry to its node. Solace computes all of this from the task's before-state (Spec 15 §5) and the current project, so nothing an agent says enters it.

## 5. Serving

`solace serve` serves the view beside the board at `/view`, from the same binary, using the same local-only HTTP API style as Spec 15:

| Request | Effect |
| --- | --- |
| `GET /graph` `?task=` | The graph (§1); with `task`, filtered and annotated for that task (§4) |
| `GET /view` | The view's page |
| `GET /nodes/{id}` | One node's detail panel, including a function's input forms and a test's lines |
| `GET /nodes/{id}?source=1[&task=]` | The symbol's source; with a task, also its line diff from before the task, also for a symbol the task removed |
| `GET /types/{id}` | A type's constructors and the public functions that return it, for forms |
| `POST /tests` `{task}` | Runs the task's tests for review (§4.3) |
| `POST /runs` `{version, steps}` | Starts a run (§3) and remembers it; returns its id |
| `GET /runs` | Remembered runs |
| `GET /runs/{id}` | The run's per-step results, changes, output, files and effects |
| `GET /runs/{id}/files/{path}` | A file the run produced |
| `POST /runs/{id}/task` `{expected}` | Makes a board task from the run |
| `POST /replays` `{task}` | Replays the remembered runs that reach the task's changes, before and after (§4.4) |

## 6. Build order

1. The graph and review without replay (§1, §2 overview to files, §4 items 1–3, 5, 6).
2. Runs and making a task from a run (§3).
3. Remembered runs and replay in review (§4.4).

Each step ends when its claims below pass.

## 7. Later

Call tracing and decisions in business terms (D-35.2; the latter with the Solace language). Recognising framework route handlers as entry points. Blocking outside effects through policy. Custom presentation for domain values, such as a notification shown as an email.

## Tests (acceptance)

Step 1:
- C1. For a small CLI project (a `main` that parses arguments, prints and writes a file, with tests), the graph lists `main` as `entry`, every `gleam/io` and `@external` use as a `derived` `effect` reached by `uses` edges, and each test function with `tests` edges to what it references.
- C2. In `pilot-large`, `api.handle` is an `entry`, and `date_test`'s `assert date.add_business_days(Date(2026, 3, 5), 2) == Date(2026, 3, 9)` shows as `date.add_business_days(Date(2026, 3, 5), 2)` → `Date(2026, 3, 9)`.
- C3. For a task that adds a call to `simplifile.write`, `GET /graph?task=` flags a new file-writing effect on each entry point that reaches it, and the board's summary line counts it, with its untested uses (D-44).
- C4. The files panel shows each file's category and the sentence for it, matching `solace files`.
- C4a. A drag that starts on a node pans and opens nothing; a click opens a node without moving the sheet; the keys in §2 move focus, pan, zoom, expand and collapse. For a symbol a task changed, Show source gives its before/after line diff.

Step 2:
- C5. `api.handle`'s `Request` form offers each constructor of `Request`, and `AddItem` asks for `order_id: Int`, `sku: String` and `quantity: Int`.
- C6. In `pilot-large`, a run that starts from `seed.demo_store()` and calls `api.handle` with `CreateOrder`, then with `AddItem` on the first step's store, gives the same responses as the same calls in a test, shows the new order in the store's changes, and leaves the project's files unchanged.
- C7. In the CLI project, a run shows `main`'s printed output and the file it wrote, and leaves the project's files unchanged. A run that reaches a `todo` shows its step as unfinished.
- C8. Making a task from a run creates a board task whose description holds the run's steps, its results and the human's expectation.

Step 3:
- C9. After a task changes a tax rule, replay re-runs a remembered run that reaches the tax function and shows its before and after results differing, and does not replay a remembered run that doesn't reach it.
