# Spec 15: Task Board

**Status:** accepted 2026-09-29 · **Implements:** D-16 (points 5, 6), D-17, D-19, D-23, D-25, D-87, D-88, D-100, D-93, D-98, D-99, D-112, D-113, D-114, D-130 · **Needed by:** the D-16 work. Not in the PROPOSAL.md §14.4 table; numbered after it.

Humans direct agents through tasks on a board. Solace owns each task's state, starts and stops agents, and enforces the workflow. A board only shows tasks and passes on what the human asks for. The API is the smallest one a Solace-compatible board needs. Solace ships a default board built only on this API; any other board, or an adapter for one, can replace it.

The workflow (columns, and the actions allowed in each) comes from configuration. Solace ships a default workflow (§3). Everything is local: one project, one machine.

## 1. Task

| Field | Meaning |
| --- | --- |
| `id` | Assigned by Solace |
| `title`, `body` | Written by the human. The body is Markdown and may name contracts and symbols |
| `links` | Contracts and symbols the task refers to, as `{id, name, url}`; `url` opens it in the Solace UX |
| `column`, `rank` | Where the task is, and its order within the column (lower first) |
| `paused_from` | Set while paused: the column the task returns to |
| `attention` | `{reason, time, hold?}` while the task needs a human, otherwise null; `hold` is true while its agent is held on a question or stopped (§6, D-82) |
| `stamps` | Stamps the task holds, such as `approved`, each with who gave it and when (§2) |
| `model`, `effort` | The agent, and the effort if one was chosen, of the last action that started a work run |
| `finished` | `{kind, outcome, time}` after a run ends successfully, until the next action or new run (D-37) |
| `notes` | `{author: human\|agent\|solace, run?, time, text}`, oldest first |
| `questions` | Raised by agents: `{id, text, options[], answer?, settings?}`. An answer is one option or free text. `settings: {base, proposed}` marks a proposed `solace.toml` change |
| `runs` | Agent runs: `{id, kind: investigate\|work\|review\|discuss, model, effort?, state: running\|waiting\|ended, started, ended?, outcome?, log}`. `log` is where the agent's own session log is, for a later view of the run (§6) |
| `changes` | What the task's work runs changed (§5), each `{entity, name, run, project}`; `project` is the folder the change was made in. Computed from the scope projects' stores by task id, not stored on the task (D-77) |
| `scope` | The projects the task may change, as folders (D-61). At least one; a task saved before D-61 has none and covers its own project. A proposed task's `scope` names projects of the workspace (D-129) |
| `feature`, `milestone` | The feature and the milestone the task belongs to, each optional (§1, Features and milestones; D-87). A task given a feature while it has no milestone takes the feature's milestone; after that the two can differ |
| `dependsOn` | Ids of the tasks this one depends on. Recorded and validated (existing tasks, not itself, no cycles), not enforced (D-87); a run's brief names the unfinished ones (§6, D-110) |
| `createdTime` | When the task was created, in seconds; never changes (D-130) |
| `closedTime` | When the task first reached a final column; never cleared |
| `archivedTime` | When the task was archived (`POST /tasks/{id}/archive`), otherwise null; cleared when the task leaves the final columns. The board hides an archived task; it stays in `GET /tasks` and the events (D-95) |
| `techDebt` | True on a feature's Tech Debt task, which completed tasks' review tech debt is filed on (§6, D-112); absent otherwise. The board marks its card "Tech debt" |

Tasks saved before D-87 have none of `feature`, `milestone`, `dependsOn`, `closedTime` and `archivedTime`: read them as null, or `[]` for `dependsOn`. Tasks saved before D-130 get `createdTime` when they load: the earlier of their first note's time and their first run's start, or else their file's modified time.

Tasks are stored as one JSON file per task in the root's task store, `<root>/.solace/tasks/` (D-77), not in a project. A task never moves. The task server (`tasks.rs`, inside `serve`, API `/tasks/...`) is the only writer: it applies operations one at a time, in order, each file written atomically (write, then rename). Agents have no access to the store (Spec 14 §1 rule 4); they send messages that wait durably in `<root>/.solace/tasks/inbox/` until the server applies them. Boards, and later the archive and Strategy pages, read and change tasks only through the task server's API (§4), never through the files. `changes` is not stored on the task: it is read from each scope project's store by task id. Run logs, conversations and review checkpoints stay in each project's `.solace/board/<tid>/`.

### Features and milestones (D-87)

A **milestone** is a planned release; a **feature** is a capability of the product. Both are records at the Solace root, not tasks (D-25): they have no column, runs, scope or workflow.

| Record | Fields |
| --- | --- |
| Milestone | `{id, title, body, rank, targetDate?, state}`. Milestones are ordered by `rank` (lower first) |
| Feature | `{id, title, body, milestone?, rank, state, startTime?, colorSlot?}`. `milestone` is the feature's target. `rank` orders features within their milestone, or among features with no milestone. `startTime` is when the feature first became `active`, in seconds; the server sets it and never clears it (null until then; a feature saved before it was kept, already active or complete, takes its file's time). `colorSlot` is set with `startTime`, one past the highest any feature holds, and never changes (null until then; a started feature saved before it was kept takes one after every kept slot, in `startTime` order). Clients color tasks by their feature's `colorSlot` (D-113, D-114) |

`state` is one of `proposed | upcoming | active | complete | canceled`, the same list for both (D-128; a record saved `planned` reads as `upcoming`). **Proposed** is suggested and not agreed; **upcoming** is agreed and queued; **active** means work has started. A new record starts `proposed` unless its create names a state. Any state can change to any other. A `canceled` record keeps its links to tasks; only a delete clears them.

A feature follows its tasks (D-128): an `active` feature with tasks becomes `complete` when every task linked to it is in a final column (canceled and Tech Debt tasks too), and a `complete` one with a task out of the final columns becomes `active` again; the server checks when a task is created, moves column or changes feature. The board shows a task with a feature only while the feature is `active` or `complete`.

Links are optional. A task has at most one feature and at most one milestone; a feature has at most one target milestone. A feature appears in every milestone that holds one of its tasks, as well as in its target. Deleting a feature or milestone clears every link to it in the same operation; it never deletes tasks.

Each record is one JSON file, at `<root>/.solace/strategy/milestones/<id>.json` and `<root>/.solace/strategy/features/<id>.json`. The task server is their only writer: it writes each file atomically, as it does tasks, and assigns ids the same way as task ids. Clients read and change them only through the API (§4); agents have no access (Spec 14 §1 rule 4).

### Vision (D-91)

The root has one **vision**: what the product is for and who uses it, as Markdown at `<root>/.solace/strategy/vision.md`. It is one document, not records, and has no id or history. Its `revision` is `sha256:` and the hex SHA-256 of its text; a missing file reads as the empty text. The task server is its only writer and writes it atomically; agents have no access (Spec 14 §1 rule 4). Clients read and change it only through the API (§4). It holds at most 1000 characters, counted as Unicode code points (D-92). Investigate, work and discuss briefs carry it whole; review briefs don't. A vision saved before the limit and longer than it is cut to its first 1000 characters, at a line end when one is in reach.

### Strategy runs (D-93)

A **strategy run** is an agent planning with the human on the Strategy page. It belongs to no task: each is one JSON file at `<root>/.solace/strategy/runs/<id>.json`, written atomically by the task server alone, with ids assigned as for tasks.

| Field | Meaning |
| --- | --- |
| `id`, `model`, `effort?`, `state`, `started`, `ended?`, `outcome?`, `log` | As a task's run (§1 `runs`); `kind` is always `strategy`, `outcome` is `planned` or `stopped` |
| `context` | What the run was started on: `{kind: "page"}`, or `{kind: "task"\|"feature"\|"milestone", id}` |
| `notes` | As a task's `notes`; the human's messages, the agent's replies, and `solace` notes recording each answered proposal |
| `questions` | As a task's `questions`, each with a `proposal` (§6) and `time`: when it was asked, set by the server (D-118; none on a question saved before) |

Its scope is every task, feature and milestone and the vision; it reads code in any project and changes none. Every change it proposes waits for the human's approval.

## 2. Workflow configuration

The `[board]` section of `solace.toml` (humans only, Spec 14 §2) defines the workflow from a fixed set of building blocks. Every project on a board shares one workflow, the root's (D-61): a project below a root ignores a `[board]` of its own.

- **Columns**, each with an `id`, a `label`, the `actions` allowed there, whether it is `final`, and optionally a gate: a `next` column and the stamps it `requires` before a task may move on to `next`. Moves to other columns, such as back to work or to `cancelled`, are not gated. They do remove the stamps the gate requires: a task moved anywhere but `next` after approval loses its approval. A final column offers only the actions it lists, none of the built-in ones (D-108). An action that moves a task out of a final column adds a `solace` note saying the human brought it back; the move clears `archivedTime` and keeps `closedTime`.
- **Actions**, each with an optional `label` that its button shows instead of its name (D-108), and a combination of these steps, applied in this order:
  1. `revert`: undo the task's changes (§5);
  2. `stamp` / `unstamp`: add or remove a stamp. Stamps are only ever given by a human request;
  3. `to`: move to a column. Refused (`E7006`) if this is the current column's `next` and the task lacks a stamp it `requires`;
  4. `run`: `investigate` and `work` hand the task back to its waiting agent of that kind if there is one, and otherwise start an investigate or work run. `new_work` ends any run and starts a fresh work agent. `review` starts a review run (§6). With `model = true`, the request must name a model, unless it continues a waiting agent; any request may name one.
- Tasks have no type (D-25): what an agent may do comes from the kind of run the column's actions start. Only work runs change the project.
- **Events**, the moves Solace makes by itself: `asked` (a work agent asked a question or proposed a settings change), `finished` (an agent reported it was done) and `failed` (a work run crashed, timed out, or exited without finishing). `finished` names a target column. `asked` and `failed` may name one; without it the task stays in its column and is flagged for attention (D-82). No action can make these moves.

Five actions are built in and allowed in every column that isn't final:
- `pause` records `paused_from`. The task's agent is held at its next Solace call until `resume`, so it keeps its context. The only actions that end an agent are those with `new_work`, and `revert`.
- `resume` returns the task to `paused_from`. A paused task also allows the actions of `paused_from`.
- `stop` (D-72) ends every open run with outcome `stopped`, kills its agent and throws away its session; changes are kept, the task stays in its column, and no event fires. Refused without an open run (`E7003`). It also ends an agent, as `new_work` and `revert` do.
- `discuss` (D-72) starts a read-only `discuss` run with `{model, effort?, text}`; `text` is required (`E7001`) and is the discussion's first note. Refused while a run is open (`E7003`). The agent replies with `task note`, which waits for the human's next note (`task done` and `task ask` are refused); waiting doesn't count toward the timeout. An agent that instead replies in plain text and exits cleanly has taken its turn (D-76): the reply becomes its note, the run waits, and the human's next note starts a fresh agent briefed with every note. While it is open, a human note is the discussion's next turn.
- `end_discussion` ends the open discussion with outcome `discussed` (`E7003` without one). When a discussion ends, by this or by `stop`, Solace replaces its notes with one `solace` note: a verbatim transcript, each turn labeled `**Human:**` or `**Agent (<model>):**` on its own line, followed by a blank line and the turn's text. A discussion's brief keeps every note whole, lists the decision titles from `DECISIONS.md`, and leaves out the verification record.

Solace checks the configuration at start and on every change. An invalid workflow is `E6020` (Spec 14 §6): an unknown column, an action that is never allowed anywhere, an event with no target, or a gate whose `next` is unknown.

## 3. Default workflow

```toml
[board]
events = { finished = "needs_review" }   # asked and failed leave the task where it is, flagged for attention

[[board.columns]]
id = "backlog"
actions = ["plan", "cancel"]

[[board.columns]]
id = "planning"
actions = ["investigate", "assign", "complete", "to_backlog", "cancel"]

[[board.columns]]
id = "assigned"
actions = ["reassign", "new_agent", "review", "cancel"]

[[board.columns]]
id = "needs_review"
actions = ["approve", "complete", "review", "reassign", "start_over", "cancel"]
next = "complete"
requires = ["approved"]                # an agent's result is on the task: a human signs it off

[[board.columns]]
id = "complete"
final = true

[[board.columns]]
id = "cancelled"
final = true
actions = ["to_backlog"]   # a canceled task can come back (D-108)

[board.actions]
plan       = { to = "planning" }
to_backlog = { to = "backlog", label = "Back to backlog" }
investigate = { run = "investigate", model = true }     # read-only; stays in its column
review     = { run = "review" }                         # stays in its column
assign     = { to = "assigned", run = "work", model = true }
new_agent  = { to = "assigned", run = "new_work" }     # a fresh agent, with the notes so far
approve    = { stamp = "approved" }
complete   = { to = "complete" }
reassign   = { unstamp = "approved", to = "assigned", run = "work" } # preserve work and address feedback
start_over = { revert = true, unstamp = "approved", to = "assigned", run = "new_work" }
cancel     = { revert = true, to = "cancelled" }
```

`reassign` moves a task from needs review to assigned, preserves its code and change records, removes approval, and starts a work run with the notes and review feedback. In `assigned` it brings back an agent that failed or was cut off: the previous conversation when it is compatible (D-33), else a fresh agent with the notes. It uses the previous work model unless the human selects another. It is refused while a review run is still active. `start_over` additionally reverts the task before starting again.

`revert` also ends the task's agent, if it has one.

Two rules hold for every workflow:
- A `work` or `new_work` step is refused while any question is unanswered, unless the request has a `text` (`E7002`). The answers, or the `text`, go to the agent.
- A second run is refused while the task has one going (`E7003`). A work run is refused while another task has a work run going in any project of its scope (`E7005`), or an `Unexpected Changes` task there is open (`E7012`). The run holds all of them until it ends (D-61).

## 4. HTTP

`solace serve` serves the API on `127.0.0.1` (local only; there is no authentication). Bodies are JSON. Errors are `{code, message, allowed?}`.

| Request | Effect |
| --- | --- |
| `GET /workflow` | Columns and their actions, as configured, so a board can draw itself |
| `GET /tasks` | All tasks, without `notes` and `changes`. `runs` are included so a board can show which tasks have an agent working |
| `GET /tasks/{id}` | One task, in full. At the root, also one whose home is no project of the workspace, with `noProject: true` (D-129) |
| `GET /tasks/{id}/runs/{rid}` | The run, plus `transcript`: the agent's log read into entries `{kind, text?, title?, input?, output?, status?}`. `kind` is `message`, `thinking`, `tool` (with `status` `running`, `ok` or `failed`), `usage`, `error` or `output` (lines that aren't agent events). A board fetches it again while the run is going (§6) |
| `POST /tasks` `{title, body, links?, scope?}` | Creates a task at the end of the first column. `scope` lists the projects it may change (default: this one) and must include this project, its first lane (`E7001`). `request` is a client-chosen id: a second create with the same `request` returns the task the first made, so a double submit makes one task (D-77) |
| `PATCH /tasks/{id}` `{title?, body?, links?, rank?, feature?, milestone?, dependsOn?}` | Edits a task. While a run is going only `feature` and `milestone` may change; any other field is refused (`E7003`, D-100). In a final column only `feature` and `milestone` may change; any other field is refused (`E7004`). An unknown feature or milestone id is `E7023`; a `dependsOn` naming a missing task or the task itself is `E7024`, and one that makes a cycle is `E7025`. Setting `feature` on a task with no `milestone` also sets the feature's milestone |
| `POST /tasks/{id}/actions` `{action, model?, effort?, text?}` | §2. Returns 200 with the task, or 409 with `E7001` (not allowed here; `allowed` lists what is) or `E7002`–`E7008` (`E7007`, `E7008`: a revert blocked by a later task) |
| `POST /tasks/{id}/scope` `{scope}` | Sets the projects the task may change (D-61). Refused while its agent is working, not paused or waiting on a question (`E7003`); in a final column (`E7004`); for a project the task has changed, or for the task's own project while a run is open (`E7013`). A run that is paused or waiting holds a project added now (`E7005`, `E7012`). When the task's own project leaves the scope, the task moves to the first project left, which sends `{removed: id}` from the old one |
| `POST /tasks/{id}/archive` | Archives a task in a final column: sets `archivedTime` to now, or keeps it if already set. Refused in any other column (`E7001`). The task is not deleted (D-95) |
| `POST /tasks/{id}/notes` `{text}` | Adds a human note |
| `POST /tasks/{id}/files` (the image's bytes as the body) | Stores an image (PNG, JPEG, GIF or WebP by content; at most 5 MB) and returns `{name, url}`. A note uses it as `![alt](url)` (D-37) |
| `GET /tasks/{id}/files/{name}` | The stored image |
| `DELETE /tasks/{id}/notes/{i}` | Deletes the note at position `i` (0 is the oldest), whoever wrote it. Notes are only ever appended, so a position stays valid until a note before it is deleted |
| `POST /tasks/{id}/questions/{qid}` `{option?, text?}` | Saves the answer and continues the same waiting run once all questions are answered, unless paused; the answers go straight to the waiting agent, and no workflow action is involved (D-88). For a settings question, `Approve` writes `proposed` if `solace.toml` still equals `base` and is valid; otherwise the answer records that it was not applied |
| `GET /models` | The models this Solace install knows, each with `agent`, `efforts` (the efforts an action may choose; empty for custom runners), `available` and, when unavailable, `reason` (§6) |
| `GET /events` | Server-sent events: `{task}` in full after every change, and `{workflow}` after a configuration change, in order. A client that reconnects with `Last-Event-ID` gets what it missed; a new one starts from the project's `{workflow}` and `{tasks: [...]}` (the list `GET /tasks` returns) as they are, then gets every later change (D-78). At the root, the replay also sends a separate `{tasks: [...], noProject: true}`: the root store's tasks whose home is no project, which `GET /tasks` leaves out; a later change to one comes as `{task}` with `noProject: true` (D-129). Once a connection has sent what was missed, it sends one `ready` event (`event: ready`), so a client can apply the replay at once and treat only later events as news |

A `text` sent with an action is added as a human note before the action runs.

An action that completes a task (moves it into a final column without `revert`) files its last review's tech debt (§6, D-112): the feature's Tech Debt task, new or with new notes, is written in the same operation, before the task, and sends its own `{task}` event.

### Features, milestones and the vision (D-87, D-91)

These answer at the root only, not under `/p/{dir}`.

| Request | Effect |
| --- | --- |
| `GET /milestones` | All milestones |
| `POST /milestones` `{title, body?, targetDate?, state?, request?}` | Creates a milestone at the end of the order, in `state` (default `proposed`). Uses `request` the same way as `POST /tasks` |
| `PATCH /milestones/{id}` `{title?, body?, rank?, targetDate?, state?}` | Edits it |
| `DELETE /milestones/{id}` | Deletes it and clears `milestone` on every feature and task linked to it |
| `GET /features` | All features |
| `POST /features` `{title, body?, milestone?, state?, request?}` | Creates a feature at the end of its group, in `state` (default `proposed`). Uses `request` the same way as `POST /tasks` |
| `PATCH /features/{id}` `{title?, body?, milestone?, rank?, state?}` | Edits it |
| `DELETE /features/{id}` | Deletes it and clears `feature` on every task linked to it |

An unknown feature or milestone id, in a path or a body, is `E7023`.

### Task, feature and milestone links (D-98, D-122)

| Request | Effect |
| --- | --- |
| `GET /tasks/find/{id}` | At the root only: `{id, dir, archived, title, project}`, the project folder the task is stored in (`.` for the root's own), whether it is archived, whichever project that is, its title, and `project`: false when its home is no project of the workspace (D-129). An unknown id is 404 with `E7023` |
| `GET /features/find/{id}` | `{id, title}` of the feature. An unknown id is 404 with `E7023`; a malformed one is 400 with `E7001` |
| `GET /milestones/find/{id}` | `{id, title}` of the milestone, as for a feature |

The server serves the shell at `/task/{id}`, `/feature/{id}` and `/milestone/{id}`, the addresses a link (`task:<id>`, `feature:<id>` or `milestone:<id>` in markdown) points to. The shell opens a task on the board, or on the Strategy page when it is archived; a feature on the Strategy page's Features tab, a milestone on its Roadmap tab. A bare link shows the record's title.

| Request (D-91) | Effect |
| --- | --- |
| `GET /vision` | The vision, `{text, revision}` |
| `PUT /vision` `{text, base}` | Replaces the text if `base` is the current revision, and returns the vision as it is now. Another `base` is refused with `E7026` (409) and writes nothing; a missing or non-string field is `E7001` (400), and text over 1000 characters is `E7027` (400, D-92) |

**Events.** The root's `/events` sends `{milestone}` and `{feature}` in full after every change, `{removedMilestone: id}` and `{removedFeature: id}` after a delete, and `{vision: {text, revision}}` after every vision write. They carry no `project` field and go on the root task log, so the `tasks` position in `Last-Event-ID` resumes them. A new connection's replay sends `{milestones: [...]}`, `{features: [...]}` and `{vision}` after `{tasks}`. A delete that clears links also sends `{task}` for each task it changed. `/p/{dir}/events` leaves them out.

### Build plan (D-131)

At the root only.

| Request | Effect |
| --- | --- |
| `GET /build/plan` | Every project's build recipe, in build order: `[{project, source, after, outputs, steps: [{name, run, timeout, command?}]}]`. `source` is `declared` (its `solace.toml` `[build]`) or `default` (from its files); a project with neither is left out. A `command` step's `run` is that command's. An invalid recipe, an `after` naming no project, or a cycle is 409 with `E6020` |

### Strategy runs (D-93)

These answer at the root only.

| Request | Effect |
| --- | --- |
| `GET /strategy/runs` | All strategy runs, without `notes` |
| `GET /strategy/runs/{rid}` | One run in full, plus `transcript` as `GET /tasks/{id}/runs/{rid}` gives it |
| `POST /strategy/runs` `{model, effort?, text, context}` | Starts a run; `text` is required (`E7001`) and is its first note. A `context` naming a missing record is `E7023`, or `E7001` for an unknown task. Refused while a run on the same context is open (`E7003`), or for an unavailable model (`E7011`) |
| `POST /strategy/runs/{rid}/notes` `{text}` | The human's next message: the waiting agent gets it. Refused once the run has ended (`E7003`) |
| `POST /strategy/runs/{rid}/questions/{qid}` `{option?, text?}` | Answers a proposal (§6); the waiting agent gets the answer |
| `POST /strategy/runs/{rid}/end` | Ends the open run with outcome `planned` (`E7003` if ended); its notes become one transcript note, as `end_discussion` does (§2) |
| `POST /strategy/runs/{rid}/stop` | Kills its agent and ends it with outcome `stopped`, with the same transcript note |

**Events.** The root's `/events` sends `{strategyRun}` in full after every change, with no `project` field, on the root task log. A new connection's replay sends `{strategyRuns: [...]}` (the list `GET /strategy/runs` returns) after `{vision}`.

`solace serve` also serves the default board at `/` and a guide for humans at `/guide`.

### One root, several projects (D-57)

A **Solace root** is one folder; its projects are the root itself (`.`, when it has a store) and folders below it. Every root is a project (D-60): a root without `gleam.toml` is set up as a project of plain files when `serve` starts there or a project is listed in it. The root's `solace.toml` lists the folders: `projects = ["bench/pilot", "web"]`, each relative, without `.` or `..`, reached without symlinks, and not itself listing projects. A root with no store holds only that key; a root project's `solace.toml` also holds its own settings. A project with no root above it is its own root, with itself as its only project, which is every project before D-57. Each project keeps its own store, settings, lockdown, tasks and runs.

There is one root: `solace serve` and `solace create` use the root that holds their folder, so they act the same anywhere below it. That is the one folder above that lists projects (or holds only a `solace.toml`), else the outermost project. A root inside another root, or inside a project, is refused (`E6020`). With no root, `serve` makes its folder an empty root, and `create` sets up the project there, or, in a folder without `gleam.toml`, offers each Gleam project below (skipping build output, dependencies, ignored folders and symlinks) and lists the ones confirmed. `create` in a project below a root lists it in that root. A server holds each project it serves (`.solace/serve.lock`); a second server for the same project is refused. While it holds the lock, the server writes `{"port":N,"pid":P}` into the file (D-80); a port read from a lock is trusted only while the lock is held.

A request is refused with 403 (`E7016`) unless its `Host` is `127.0.0.1:{port}` or `localhost:{port}` and, when it sends `Origin`, that is `http://127.0.0.1:{port}` or `http://localhost:{port}` (D-80). A request without those headers is accepted. `SOLACE_BIN`, when set in the server's environment, is the CLI that agents call instead of the server's own executable.

The root's own project answers at `/` as above; every other project's API is under `/p/{dir}`, `dir` percent-encoded (`/p/bench%2Fpilot/tasks`, `/p/bench%2Fpilot/view`). A note's image URL stays project-relative (`/tasks/{id}/files/{name}`); a client prefixes the project's `base`. The root adds:

| Request | Effect |
| --- | --- |
| `GET /projects` | `{workspace, projects: [{dir, label, name, target, base}], found: [{dir, name, imported, member}]}`. `workspace` is true when the root lists projects or has no `gleam.toml`, so a board shows lanes. `label` is the folder (the root's own folder name for `.`), `name` and `target` come from `gleam.toml` (`target` is `erlang` when unset, `""` with no `gleam.toml`), and `base` is the API prefix (`""` for `.`). `found` lists the Gleam projects in the root |
| `POST /projects` `{dir}` | Lists the project at `dir` again, keeping its store; only a project that already has a store (one removed from the list, D-59). Existing code without a store comes in by migration (below). Refused if `dir` lies in another project's `src/` or `test/`, lists projects of its own, or another server holds it |
| `POST /projects/new` `{name, dir?, target?}` | `gleam new` at `dir` (default `name`), with `--template javascript` for `target: "javascript"`, without its own git repository inside one; sets it up and lists it. Refused if `dir` exists or `name` isn't a package name |
| `DELETE /projects/{dir}` | Takes the project off the list; its `.solace` stays. Refused for `.` and while one of its agents is working (`E7003`). With `{delete: true, name}` (D-74) it also deletes the project's folder after unlisting it; refused (`E7014`) unless `name` is the folder or package name, the folder is reached without symlinks and holds no other listed project |
| `GET /models` | As above |
| `GET /events` | When `workspace`: every project's events, each with `project: dir`, and `{projects: [dir...]}` when the list changes. The event id is a JSON object of the task log's position (`tasks`, one log for the root's task store, D-77) and each project's position for its workflow events, so `Last-Event-ID` resumes all of them. A new connection starts from each project's `{workflow}` and `{tasks}`, then the root's `{milestones}` and `{features}`, which carry no `project` (D-87), then, after `{strategyRuns}`, a separate `{tasks: [...], noProject: true}` of the root store's tasks whose home is no project (D-129); a project added later sends the same pair after `{projects}` and then follows from its latest event (D-78). Later events from the root task store carry every task stored in it whatever its home, each with `noProject: true` when its home is no project (D-129). One `ready` follows the replay. Otherwise the root project's stream as above. `/p/{dir}/events` is one project's stream alone |

Reads never take a project's lock, so humans and agents read any number of projects at once; an agent reads other projects with `--project` (Spec 3 §2). Each project has at most one work run (§6).

**Tasks across projects (D-61).** A task's `scope` lists the projects it may change; it is stored in one of them and shown in every lane of its scope, in the same column. A work run holds every project in its scope from start to end. Its agent works in the task's project and changes another project of its scope by adding `--project <folder>` to any command; a project outside the scope is refused (`E6007`) and the agent asks the human. Each change is recorded in the project it was made in; `task diff`, the Contract view (`/p/{dir}/graph?task=` shows that project's part), `revert`, completion and changes made outside Solace during the run (D-58) span the scope. Changes to several projects lock them in folder order. `DELETE /projects/{dir}` is refused while an open task stored elsewhere covers it (`E7013`). A task whose home is no project of the workspace is in no lane (D-129): the Strategy page lists every task in the root's store whatever its home, marks such a task as having no project, and opens it from a link; the human can cancel and archive it.

### Migration (D-59, Spec 17)

The root also answers:

| Request | Effect |
| --- | --- |
| `GET /migration` | `{state, top, original, copy, step, candidates, report, blockers, undo, flagged, problem, error}`. `error` is the last step's failure, if it failed (null otherwise). `state`: `none` (nothing to migrate), `available` (Gleam projects with no store and no Solace root: `candidates` lists them as `[{dir, name}]`), `running` (`step` says what Solace is doing), `copying` (a copy was started and not finished: another process is making it, or it was interrupted), `confirm_paths` (the copy is made but nothing has run, because something in it could reach the original: `flagged` lists those items, shaped like report items), `failed` (the migration stopped partway and its copy is kept: `problem` says why, and `report`, if not null, holds what was checked, such as a failed Coverage naming unreadable files), `in_progress` (the board serves the copy), `adopted` (the board serves the adopted project while `<name>.pre-solace` exists). `top` is the folder this server serves, `original` and `copy` full paths (or null). `report` (null before one exists) is `{checked_at, current, checks: [{name, status, summary, items: [{text, status, detail}]}]}`, plus `warnings` (a list of sentences to show first, such as a project that doesn't build before or after the migration), where `status` is `verified`, `failed` or `not checked` (`current` false: the copy changed since the checks ran, and the client says so above the checks), an item's `status` is optional, and `detail` (optional) is preformatted text such as a diff or command output. `blockers` lists why Adopt isn't offered (empty when it is). `undo` is true when Undo adoption is possible |
| `POST /migration/start` `{projects: [dir], smoke: [command]}` | Starts a migration of the projects named (at least one of `candidates`); each smoke check is a shell command line run in the root folder. Returns at once with `state: running`. When it ends, the server restarts itself serving the copy, at the same port, in `confirm_paths`, `failed` or `in_progress`. Refused if `../solace-migration/` exists (`E7020`) |
| `POST /migration/confirm` | Continues a migration in `confirm_paths`, after the user has looked at `flagged`; the server restarts itself serving the copy when it ends |
| `POST /migration/recheck` | Runs every check again in the copy (`state: running` until done) |
| `POST /migration/adopt` | Adopts the copy (Spec 17 §6); refused while `blockers` isn't empty (`E7021`), an agent is working, or the original changed since its inventory (`E7022`, with the changed paths). On success the server restarts itself at the original path |
| `POST /migration/abandon` | Deletes the copy, in `in_progress`, `confirm_paths`, `failed` or `copying`; the server restarts itself serving the original |
| `POST /migration/undo` | Swaps the adopted project and `<name>.pre-solace` back; the server restarts itself serving the original |

A client shows the copy's full path and "Migration in progress" while `state` is `in_progress`, polls `GET /migration` while `running`, and reloads once the server answers again after a restart. While a step runs, the server refuses every other request that changes something with `E7003` ("a migration step is running"). One migration step runs at a time, from the board or a shell (`E7003`), and a shell can't act on a migration this server serves.

### Changes made outside Solace (D-58)

Solace never writes over a Gleam file under `src/` or `test/` that differs from what it last wrote there (`.solace/written`). Such a file was changed outside Solace: at the start of every command, and from the board's poll about once a second, Solace reads it into the store as new revisions (a removed file retires its symbols). It compares sizes and modified times and reads only files whose size or time changed. A file that changes while a command runs is left as it is and read at the next command.

- **During a work run** the changes join that task's `changes` and before-state records, so its review shows them and `revert` undoes them. The task gets a note naming them, and so does the agent's reply.
- **A process a command runs** (`format`, `build`, a declared command) changes code as part of that command: Solace reads it in right after, and in a run records it with the command's other changes.
- **With no work run going** they go to a task titled `Unexpected Changes` with `"unexpected": true`, created in `events.finished`'s column and joined by later outside changes while it is open. Its note names the changed symbols and gives the compile result. Completing it keeps the changes; `cancel` restores the code Solace had. While it is open, starting or continuing a work run on any other task is refused (`E7012`).
- **Unreadable files**, such as one defining a name twice, or one that isn't UTF-8, are left untouched, named once in a note, and read again once they change.
- **Approved code is read in quietly.** `.solace/approved` lists a hash of every symbol (`module.name`) and module header (`module (imports)`) as last approved: all of them when a project is set up, and each entity of a task when it reaches a final column with its changes kept. A changed symbol whose new text matches its listed hash, or a removed one no longer listed, needs no review unless an open task changed it. `create` writes `.solace/.gitignore` so that only `approved` is committed.
- Only graph files raise these changes. Watched and unwatched files are Solace's to write only when a task writes them.

## 5. A task's changes and `revert`

**What is guaranteed** (the contract, whatever the mechanism): `revert` returns everything a task changed to how it was before the task, or refuses and changes nothing. It never leaves the project half-reverted or failing to type-check because of the revert. `approve` stamps the task. Nothing else in this spec depends on where a task's work lives.

**How it is done for now** (D-17, D-18: one work run at a time): the work goes straight into the project as it is made. There is no branch and no copy. What makes `revert` possible is that Solace records, per task, the state **before the task first changed it**:

- **Symbols (graph):** the text and revision before the task's first change to each symbol, including symbols it created (so they can be removed) and deleted (so they can be restored). Every submit is tied to its run, and every run to its task.
- **Direct file edits (watched and unwatched files):** the bytes before the task's first write or removal, or a note that the file didn't exist yet. They are kept in `.solace/board/<id>/before/`. Declared commands snapshot watched files; unwatched command outputs are not tracked.

`revert` puts all of it back and clears `changes`. If a **later task** has changed any of the same symbols or files since, `revert` is refused (`E7007`) and nothing changes. The reply lists each of them and the task that changed it. The human can revert that task first, or leave things as they are. After restoring, Solace type-checks the project. If the revert breaks code that another open task changed (for example, a caller of a function this task added), Solace puts everything back as it was before the revert and refuses it (`E7008`), naming the broken symbols and their tasks.

**Agents never see these records.** They sit in their own table and folder, read only by `revert`, the review run's `changes` and the UX. `find`, `inspect`, `view`, `refs`, `session start` (including the task packet) and every count or index leave them out. A symbol deleted by a task is gone from all of these, even though its earlier text is kept for undo.

When a task reaches a final column, the records of the state before it are dropped. They are there so a task can be undone, not as history (D-16.8).

## 6. Runs

- **Models** come from the Solace system configuration, not the project: built-in presets for the current major Codex and Claude models (`src/default-models.toml`), plus entries in `~/.config/solace/config.toml` (or `$SOLACE_CONFIG`). An entry with a preset's `id` replaces it, and `presets = false` hides the presets. Each has an `id`, a `label`, an `agent` (`codex` or `claude`), and a `model`, with an optional `effort`, the default. An action may choose another of the model's `efforts` (`E7001` otherwise); Codex gets it as `model_reasoning_effort`, Claude Code as `--effort`. An action that doesn't name a model uses the task's `model` and `effort`.
- **Endpoints.** A model may name an `endpoint = {kind, base_url, key_env?}`. `kind = "openai"` goes with `agent = "codex"` and is passed to Codex as a custom provider using the Responses API. `kind = "anthropic"` goes with `agent = "claude"` and is passed to Claude Code as `ANTHROPIC_BASE_URL`, plus `ANTHROPIC_AUTH_TOKEN` taken from `key_env`. If the endpoint is plain `http` and its `GET /v1/models` reports the model's window (`meta.n_ctx`, as llama.cpp and Strata do; a server listing one model is matched whatever the configured name), Claude Code also gets it as `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, so it compacts before the server refuses the prompt (D-63). A value already set in Solace's environment wins. Solace stores no credentials: agents use their own sign-in, and endpoint keys are read from the named environment variable when the agent starts. A mismatched kind is `E6020`.
- **Availability.** `/models` marks a model unavailable, with the reason, when its agent program isn't on `PATH`, when the agent isn't signed in, or when the endpoint's `key_env` isn't set. Only local things are checked, so the check costs nothing. Starting a run on an unavailable model is refused (`E7011`), and the task stays where it is.
- **Attention.** Solace sets `attention` when a task needs a human: an agent asked a question, a run failed (the note includes the agent's last message, or else the end of its output), or the server restarted while an agent was working. It is cleared by the next successful action request on the task, except attention with `hold: true` (D-82): an agent held on a question or settings proposal, or a work run that failed or was cut off by a restart. That stays until the run continues, is restarted (`new_work`, `revert`), is stopped, or the task reaches a final column. A task no longer moves to a `blocked` column; `blocked` is reserved for tasks that depend on other tasks. A task saved in `blocked` under a workflow without that column loads in `assigned` with held attention.
- A **work run** starts the agent with Spec 14 §5's configuration. Its prompt is plain text, not the task's JSON: what the run is for and how to start, ask and finish, then the task's title, body, links, notes and questions with their answers. Images in the briefed notes are attached to the prompt (Codex `-i`, Claude Code a stream-json image block) and named `[image N]` in the text; a custom runner gets `[image N: path]`. The run's kind decides the role line, and a read-only run's brief says which commands are refused. When a task in `dependsOn` is unfinished (not in a final column; complete and canceled are finished), the prompt has a **Depends on** section: every dependency with its id, title and column (and its project if not the task's home), the unfinished ones marked, then advice to check whether the work can go on without them and otherwise to ask the human (`task ask`). Nothing is refused, and with no unfinished dependency the section is left out (D-110). A work run's role line has the agent run all tests of the projects the task covers before `task done`, fix those its changes broke, and report the rest (D-81). On top of Spec 14, the agent's Solace tool gets:
  - `task note <text>`: adds an agent note;
  - `write solace.toml` (D-38) validates the proposed text (`E6020` if invalid), then adds a settings question with options Approve and Reject and waits exactly like `task ask`. Only Approve writes the file. An optional `reason` argument is added to the question's text after "Change <file>" (D-89). `task ask` and a settings `write` must be the only command in their tool call; otherwise the call is refused with `E7017` and nothing runs;
  - `task ask` (`text`, 2–4 `options`): adds a question and fires `asked`. **The agent waits:** answering the last pending question automatically continues the same run, in any column and any workflow: the answers go straight to the waiting agent and no workflow action is involved (D-88). The call returns the answers and any continuation `text`. An action that continues the waiting run's kind (`reassign` in `assigned`) with a `text` still works, answering every open question with that text. The run keeps its context. `new_agent` instead ends it and starts a fresh one, which gets the notes and answers in its prompt;
  - `task done <summary>`: adds the summary as a note, fires `finished`, and ends the run.
  - The message of `task note|done|ask` goes in `text`; a first word after the subcommand that starts with `--` (such as `--text`) is refused with `E7015`, and nothing is sent (D-79).
- An **investigate run** gets the same prompt, with a role line asking for findings or a plan. It is read-only like a review run: `write`, `submit`, `delete`, `remove`, `format` and `run` are refused (`E7010`). Its `task done` report is added as a note and ends the run, and fires no event: the task stays where it is. Its `task ask` fires no event either. The task stays in its column, marked as needing attention, and answering its pending questions automatically continues the waiting agent. Explicit `investigate` with an answer still works. A later work run gets the report in its prompt with the other notes.
- A **review run** gets the same prompt plus the names of the task's `changes`. It is read-only (`E7010`), ends by adding its feedback with `task note`, and fires no event. On a task with a `feature`, its brief also asks for minor, non-blocking findings that later work in the feature should fix under a `### Tech debt` heading, one per bullet with its location (D-112). When the task is completed, Solace files each top-level bullet of its last review's `### Tech debt` (the agent note of its last run ended `reviewed`) as a `solace` note on the feature's open Tech Debt task (`techDebt: true`, not in a final column), naming `task:<id>` and the review run; with none open, it creates one in the first column of the task's project, titled `<feature title>: Tech Debt`, with the feature, its milestone and the task's `scope`. A finished Tech Debt task is not reopened. The completed task gets a note naming the Tech Debt task. No proposal is needed (amending D-93 point 4). A task with no feature, a cancel, or a review without the section files nothing.
- A **strategy run** (D-93) is started from the Strategy page (§4), not by an action. Its brief gives the role (plan with the human, read-only, every change is a proposal whose `title` names the work in a few words and leaves details to the `body`, D-106), its `context` record in full, the vision and the 10 most recent decision titles from `DECISIONS.md` (D-96), and its notes. Its tool description and refusals name only what it can do. Commands that change a project are refused (`E7010`), and it takes no project lock; reads with `--project` reach every project. `task note|done|ask` are refused; it has instead:
  - `strategy list` (D-101): one line per record — `Tasks:` with `<id> <column> f:<feature|-> m:<milestone|-> <title>`, `Features:` with `<id> <state> m:<milestone|-> <title>`, `Milestones:` with `<id> <state> #<rank> <title>`, then the vision. A task with an `archivedTime` is left out; `strategy list --archived` includes them;
  - `strategy show <task|feature|milestone> <id>`: the ids of the records it links to (a task's feature, milestone and `dependsOn`; a feature's milestone and tasks; a milestone's features and tasks; a task's `runs`, its run ids), then the record. A feature and a milestone are in full; a task (D-101) is in brief: its own fields (`id, title, body, column, scope, feature, milestone, dependsOn, createdTime, closedTime, archivedTime`), its `notes` as one line each (`<n> <author> <time>: <first line, cut to 100 characters>`), its `questions` as one line each (first line, then answer), and its `runs` as one line each (`<id> <kind> <state> <model>`), with no `conversation` and no run `log`s. Several ids give an array of those replies in the order named, each with its `links` and `record` (D-102);
  - `strategy show note <task> <notes> [--from <n>]` (D-101): those notes in full, each with its `note` number; `all` gives every note. A number outside the notes is `E7001`, an unknown task `E7023`. A reply stops at the end of the last note that fits the reply limit and names the rest in a `Next: strategy show note <task> <notes>` line (D-105). A first note over the limit by itself comes in parts: as much of its longest text field as fits, with `part: "<field>, characters a..b of n"`, and a `Next:` line ending in `--from <b>`. `--from <n>` starts the first note named at character `n` of that field; at or past its end it is `E7001`;
  - `strategy show run <task> <run> [<steps>|all] [--from <n>]` (D-99): a task's run without its `log`, and its transcript (§4, `GET /tasks/{id}/runs/{rid}`) as one line per numbered step; with `<steps>` (`3,7,10..12`) those steps in full, each with its `step` number, or `all` of them, stopping at a whole step and coming in parts as notes do (D-105). A step outside the transcript is `E7001`, an unknown task or run `E7023`;
  - `strategy note`: the agent's reply; it waits for the human's next note, as a discussion's `task note` does (§2), and a plain-text reply is its turn (D-76);
  - `strategy task|feature|milestone [<id>]` with the fields in `text` as JSON (an id edits that record, none creates one), and `strategy vision` with the new text in `text`: a **proposal**. The fields are those of `POST`/`PATCH` in §4 (D-105): a task `title, body, feature, milestone, dependsOn`, and `scope` only to create, where every folder must be a project of the workspace (D-129): one that isn't is refused when the proposal is sent, before the human is asked (`E7001`), naming the folder and the workspace's projects; a feature `title, body, milestone, state`; a milestone `title, body, targetDate, state`; `rank` only to edit. Any other field is refused (`E7001`), naming it and those the kind takes; the brief and `help strategy` list them. It is checked as the request would be, then becomes a question on the run with options Approve and Reject and `proposal: {kind, id?, fields, base?, reopens?, before?}`, and the agent waits. `base` is the vision's revision when proposed. `before` is the record's current value of each field the proposal names, as the request's values are; for `strategy vision`, the vision's text under `text` (D-115). A create carries none, and Approve ignores `before`. The human sees only what changes: an edit shows the fields that differ from `before`, a short one as `old → new` with a record id by its title and the `body` as a line diff in its order — the lines the two bodies share at the start and the end collapsed to one muted line each, removed lines struck, added lines marked, lines compared with their surrounding space removed and blank lines left out, so a body differing only in those is unchanged — with a toggle to the new body as Markdown; a proposal with nothing to change, the vision included, says "No change" (D-116). A create, and a proposal saved without `before`, show every field (D-115), a vision its text as Markdown. `reopens` lists the `complete` or `canceled` feature and milestone a task proposal links to; Approve also sets them `upcoming`. Approve applies the proposal as the request would be applied, with the same refusals (`E7003`, `E7004`, `E7023`–`E7027`); a refused apply changes nothing and the answer says why. Reject changes nothing. Each answer adds a `solace` note to the run, and an applied task proposal adds one to the task, naming the run. The agent gets the answer, with the new record's id for a create. Like `task ask`, a proposal must be the only command in its call (`E7017`).
  - Waiting doesn't count toward the timeout. A strategy command outside a strategy run is refused (`E7001`).
- A run past the task timeout (a system setting; default 60 min) is stopped and fires `failed`. Time spent waiting on a question or paused doesn't count.
- **Pause and continuation:** answering a paused run saves the answers without continuing it; `resume` continues it once all questions are answered. Answer submission never starts a new agent, replaces a run, grants a stamp or reverts work. Answering continues the waiting run directly, in any column and any workflow (D-88). Repeated answer submissions do not start another run.
- **Keeping an agent waiting** depends on the agent's tool-call timeout. `agent-config` (Spec 14 §5) sets it to the task timeout or more (Codex: the MCP server's `tool_timeout_sec`; Claude Code: `MCP_TOOL_TIMEOUT`). If the agent gives up anyway, the run fires `failed`, and the task continues with a new agent.
- Each run records `log`, the path of the agent's own session log. `GET /tasks/{id}/runs/{rid}` reads it (Codex `exec --json` or Claude Code `stream-json`) into a transcript, so a human can follow the agent's work live or read it afterwards. The view is read only (D-19, D-24).

The bundled board confirms `cancel` and `complete` before sending an action request (D-32). Dialogs name the task and explain reverting changes versus keeping changes and removing undo records. Dismissing sends no request and preserves the form. Confirmation does not bypass API gates.

### Conversation continuity (D-33)

Investigation, work and reassigned fixes resume the author's compatible native conversation. Reviews always start fresh. Compatibility requires matching provider, endpoint and runner; a changed model or effort keeps the session (D-75), and explicit `new_work` and revert actions start fresh. Missing/incompatible state and custom runners receive a handoff automatically. A session-load failure retries once with a fresh conversation only before tools execute. Other failures, including usage limits, still require human action.

Finishing tools end the board run immediately, then allow up to 30 seconds for graceful process exit. Further tool calls from the ended run are refused. Follow-up launches wait for that exit. Author state lasts until finalization or replacement; review state is deleted after exit. Only an exited process yields reusable state. Claude keychain-only authentication on macOS uses fresh nonpersistent conversations to preserve sign-in.

Briefs contain changed entities, feedback, answers, recent outcomes/activity, drafts and recorded check/test/build results. Results are marked stale when source or watched-file contents change. Resumed authors receive new notes and skip broad orientation; deleting older notes must not hide newer feedback. Fresh handoffs include all retained notes.

## 7. Out of scope

Several projects on one server, several people, remote access and authentication, and more than one work run per project at a time. Asking a running agent side questions that don't disturb its work (D-19).

## Tests (acceptance)

- A20. Tech debt (D-112), with a fake agent: a review of a task in a feature is asked for `### Tech debt` and one of a task with no feature isn't. Completing the first task creates `<feature title>: Tech Debt` in backlog, `techDebt: true`, with a note per finding naming `task:<id>`; a second completed task in the feature appends to it; a task with no feature files nothing. Unit tests (`board::tests`) cover the section's parsing, a cancel and a review without the section filing nothing, and a completed Tech Debt task not being reopened.
- A1. With the default workflow, every configured action is tested from each of its columns, and every other column and action pair returns `E7001` with the right `allowed`.
- A2. No action moves a task to an event's target column except through that column's actions. `asked`, `finished` and `failed` move it only as configured.
- A3. A custom workflow with an extra column and action works end to end. The invalid workflows in §2 each give `E6020`.
- A4. `complete` from `needs_review` without `approved` is refused (`E7006`). After `approve`, `complete` moves the task to the `complete` column. `start_over` and `cancel` from `needs_review` are not gated, and `start_over` removes the stamp. The stamp records who gave it and when. A task approved and then moved back with `start_over` or `new_agent` no longer holds `approved`.
- A5. With a fake agent: `task ask` flags the task (`attention.hold`, still in `assigned`), and the question appears on it and in `/events`, while the agent's call stays open. `reassign` is refused until the question is answered or a `text` is given. After the last answer is submitted, the same agent process (same run id) receives the answer and continues without an extra action request, even in a workflow whose column offers no work action. A paused run waits for `resume`. Repeated answers never launch a replacement run. `new_agent` instead ends it and starts a run with a new id whose prompt has the answer. `task done` moves the task to `needs_review`.
- A6. A fake agent's run edits two symbols, creates one, deletes one, writes a watched file and creates a new one. `cancel` restores the store and those files byte-for-byte to how they were before the task.
- A7. Task B changes a symbol after task A did. Cancelling A is refused with `E7007`, naming the symbol and B, and nothing changes. After B is cancelled, cancelling A succeeds.
- A7c. Task B adds a caller of a function task A added, without editing any of A's symbols. Cancelling A is refused with `E7008`, naming B's caller, and the store and files are unchanged.
- A7b. After a task deletes a symbol and edits another, `find`, `inspect`, `view`, `refs` and `session start --task` show no trace of the deleted symbol's earlier text or of the edited symbol's old text, and give the same output as a store with no before-state records.
- A8. After `pause`, the agent's next Solace call is held and its process stays alive. `resume` releases the call and returns the task to `paused_from`. Time paused doesn't count toward the task timeout.
- A12. `/models` includes the presets and reports a model whose program is missing as unavailable, with a reason. Assigning it returns `E7011` and leaves the task in its column. A failed run and an agent's question each set `attention`, and the next action clears it.
- A13. A Codex log and a Claude Code log each give a transcript with the agent's messages in order and each tool call's command, output and status. A run that exits without finishing sets `attention` with the agent's last message rather than raw log lines. The default board shows which tasks have an agent working, formats notes and messages as markdown with any HTML escaped, and refreshes a followed run's transcript until it ends.
- A9. A review run's `write` or `submit` returns `E7010`, and the task stays in its column. So does an investigate run's. An investigate run's question keeps the task in planning, and `investigate` with an answer continues the same run. Its `task done` report appears on the task, which stays in planning and can be completed from there.
- A10. `/events` delivers every change in order, and a reconnect with `Last-Event-ID` returns the missed ones. A connection sends `ready` once it has sent what was missed. A new connection first gets each project's `{workflow}` and `{tasks}` read from the task files, so a task file changed without an event shows (D-78).
- A11. The default board's only connection to Solace is this API: it runs against a stub server implementing §4 and passes its own tests there. Every file command on `.solace/board/` returns `E6006`.

- A14. Reassigning after review retains existing symbol and file edits and before-state records, passes review feedback and human instructions to the new work run, and clears approval. The revised result requires fresh approval, and cancelling restores the state before the original work run.
- A16. `/models` lists each model's `efforts`. An action's `effort` is recorded on the run and reaches the agent; an effort the model doesn't offer is `E7001`. A successful `task done` sets `finished`, and the next action clears it. An uploaded PNG is served back as `image/png`; SVG or over 5 MB is `E7001`. A note referencing an uploaded image reaches the agent's brief as `[image 1: path]`.
- A15. Fake native Codex and Claude launchers retain author conversations through investigate/work/reassign, use fresh reviewers, fall back on lost sessions, enforce current run permissions, and clean up on completion/cancellation. Model and effort changes keep the conversation, start-over creates a fresh one; quota errors are not retried. Graceful exit persists state after the finishing tool returns; hung shutdown is bounded. Deleted notes do not swallow later feedback; intervening edits mark previous checks stale.
- A17. With a fake agent in a work run, an invalid `write solace.toml` is `E6020` and adds no question. A valid one waits on a settings question; Reject leaves the file unchanged and the agent receives the answer; Approve writes it; Approve after a human edit to the file leaves it unchanged and says so. A review run's `write solace.toml` is `E7010`.
- A19. Strategy runs (D-93), with a fake agent: `POST /strategy/runs` on a feature starts a run, and a second one on that feature is `E7003`. `strategy list` and `strategy show` follow links from the feature to its tasks and milestone, `strategy show task` gives the task in brief with its notes one line each and `strategy show note` gives them in full by number, refusing a number out of range (D-101), and `strategy show run` outlines a finished task run's steps and refuses an unknown step or run (D-99). Code changes are `E7010`, and `task done` is refused. `strategy note` waits for the human's next note. A task proposal waits on a question: Reject creates nothing, and Approve creates the task with a note naming the run. A task proposal linked to a `complete` feature reopens it (`upcoming`) only on Approve. A vision proposal approved after a human edit is not applied, and the answer says so. Every answer is on the run's notes. `end` ends the run `planned` with one transcript note. `{strategyRun}` events reach the root's `/events` only.
- U1. An edit to a module file while no run is going is read in by the board's poll, the file keeps the edit, and an `Unexpected Changes` task with the hazard flag holds the change and its compile result; another task's `assign` is `E7012`; `cancel` restores the file and the store (`outside::tests`, `tests/workspace_acceptance.py`). Only the edited project gets the task.
- U2. A file changed during a command isn't written over; a file defining a name twice is left as it is and reported once; a store with no record counts what it holds as written.
- U3. A change whose hash is on `.solace/approved` is read in without a task; completing an `Unexpected Changes` task adds its changes to the list.
- W1. One root (`tests/workspace_acceptance.py`): `solace create` in a folder offers each Gleam project and adds only those confirmed; lanes are keyed by folder when package names repeat; `/p/{dir}` reaches only its project; merged events carry `project` and resume from a combined id; a second server for a project, started from inside it, is refused; `create` in a project below the root lists it there; a project created from the board has a store; removal keeps it.
- W2. Nothing leaves the root: symlinked and `..` folders, a project that lists projects, and a root inside a root or a project are refused. A root that is a project serves itself at `/` and its other projects under `/p/`; its settings keep their comments when `projects` is written; it can't be removed.
- W3. Agents in two projects work at the same time; each reads the other's code with `--project`, and changes there are refused (Spec 3 A8).
- W4. A task covering `a` and `b` (`test_a_task_changes_every_project_it_covers`): its agent changes and tests both, with `--project b` for `b`, and is refused `c`; `task diff` names each change's project. While it works, a task in `b` can't start a work run and its scope can't change. Between runs its scope widens and narrows, but not off a project it changed, and `b` can't be removed from the board. `cancel` restores both projects. A task whose own project leaves its scope moves to the first one left. A subproject's `[board]` is ignored; `/p/b/workflow` is the root's.
