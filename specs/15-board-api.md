# Spec 15: Task Board

**Status:** accepted 2026-09-29 · **Implements:** D-16 (points 5, 6), D-17, D-19, D-23, D-25 · **Needed by:** the D-16 work. Not in the PROPOSAL.md §14.4 table; numbered after it.

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
| `attention` | `{reason, time}` while the task needs a human, otherwise null (§6) |
| `stamps` | Stamps the task holds, such as `approved`, each with who gave it and when (§2) |
| `model`, `effort` | The agent, and the effort if one was chosen, of the last action that started a work run |
| `finished` | `{kind, outcome, time}` after a run ends successfully, until the next action or new run (D-37) |
| `notes` | `{author: human\|agent\|solace, run?, time, text}`, oldest first |
| `questions` | Raised by agents: `{id, text, options[], answer?, settings?}`. An answer is one option or free text. `settings: {base, proposed}` marks a proposed `solace.toml` change |
| `runs` | Agent runs: `{id, kind: investigate\|work\|review, model, effort?, state: running\|waiting\|ended, started, ended?, outcome?, log}`. `log` is where the agent's own session log is, for a later view of the run (§6) |
| `changes` | What the task's work runs changed (§5) |

Tasks are stored as one JSON file per task in `.solace/board/`, inside the project. Agents have no access to it (Spec 14 §1 rule 4). Boards read and change tasks only through the API (§4), never through the files.

## 2. Workflow configuration

The `[board]` section of `solace.toml` (humans only, Spec 14 §2) defines the workflow from a fixed set of building blocks:

- **Columns**, each with an `id`, a `label`, the `actions` allowed there, whether it is `final`, and optionally a gate: a `next` column and the stamps it `requires` before a task may move on to `next`. Moves to other columns, such as back to work or to `cancelled`, are not gated. They do remove the stamps the gate requires: a task moved anywhere but `next` after approval loses its approval.
- **Actions**, each a combination of these steps, applied in this order:
  1. `revert`: undo the task's changes (§5);
  2. `stamp` / `unstamp`: add or remove a stamp. Stamps are only ever given by a human request;
  3. `to`: move to a column. Refused (`E7006`) if this is the current column's `next` and the task lacks a stamp it `requires`;
  4. `run`: `investigate` and `work` hand the task back to its waiting agent of that kind if there is one, and otherwise start an investigate or work run. `new_work` ends any run and starts a fresh work agent. `review` starts a review run (§6). With `model = true`, the request must name a model, unless it continues a waiting agent; any request may name one.
- Tasks have no type (D-25): what an agent may do comes from the kind of run the column's actions start. Only work runs change the project.
- **Events**, the moves Solace makes by itself: `asked` (an agent asked a question), `finished` (an agent reported it was done) and `failed` (a run crashed, timed out, or exited without finishing). Each names a target column. No action can make these moves.

Two actions are built in and allowed in every column that isn't final:
- `pause` records `paused_from`. The task's agent is held at its next Solace call until `resume`, so it keeps its context. The only actions that end an agent are those with `new_work`, and `revert`.
- `resume` returns the task to `paused_from`. A paused task also allows the actions of `paused_from`.

Solace checks the configuration at start and on every change. An invalid workflow is `E6020` (Spec 14 §6): an unknown column, an action that is never allowed anywhere, an event with no target, a gate whose `next` is unknown, or a final column with actions.

## 3. Default workflow

```toml
[board]
events = { asked = "blocked", finished = "needs_review", failed = "blocked" }

[[board.columns]]
id = "backlog"
actions = ["plan", "cancel"]

[[board.columns]]
id = "planning"
actions = ["investigate", "assign", "complete", "to_backlog", "cancel"]

[[board.columns]]
id = "assigned"
actions = ["new_agent", "cancel"]

[[board.columns]]
id = "blocked"
actions = ["unblock", "new_agent", "review", "cancel"]

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

[board.actions]
plan       = { to = "planning" }
to_backlog = { to = "backlog" }
investigate = { run = "investigate", model = true }     # read-only; stays in its column
review     = { run = "review" }                         # stays in its column
assign     = { to = "assigned", run = "work", model = true }
unblock    = { to = "assigned", run = "work" }         # the waiting agent continues
new_agent  = { to = "assigned", run = "new_work" }     # a fresh agent, with the notes so far
approve    = { stamp = "approved" }
complete   = { to = "complete" }
reassign   = { unstamp = "approved", to = "assigned", run = "work" } # preserve work and address feedback
start_over = { revert = true, unstamp = "approved", to = "assigned", run = "new_work" }
cancel     = { revert = true, to = "cancelled" }
```

`reassign` moves a task from needs review to assigned, preserves its code and change records, removes approval, and starts a work run with the notes and review feedback. It uses the previous work model unless the human selects another. It is refused while a review run is still active. `start_over` additionally reverts the task before starting again.

`revert` also ends the task's agent, if it has one.

Two rules hold for every workflow:
- A `work` or `new_work` step is refused while any question is unanswered, unless the request has a `text` (`E7002`). The answers, or the `text`, go to the agent.
- A second run is refused while the task has one going (`E7003`). A work run is refused while another task in the project has a work run going (`E7005`).

## 4. HTTP

`solace serve` serves the API on `127.0.0.1` (local only; there is no authentication). Bodies are JSON. Errors are `{code, message, allowed?}`.

| Request | Effect |
| --- | --- |
| `GET /workflow` | Columns and their actions, as configured, so a board can draw itself |
| `GET /tasks` | All tasks, without `notes` and `changes`. `runs` are included so a board can show which tasks have an agent working |
| `GET /tasks/{id}` | One task, in full |
| `GET /tasks/{id}/runs/{rid}` | The run, plus `transcript`: the agent's log read into entries `{kind, text?, title?, input?, output?, status?}`. `kind` is `message`, `thinking`, `tool` (with `status` `running`, `ok` or `failed`), `usage`, `error` or `output` (lines that aren't agent events). A board fetches it again while the run is going (§6) |
| `POST /tasks` `{title, body, links?}` | Creates a task at the end of the first column |
| `PATCH /tasks/{id}` `{title?, body?, links?, rank?}` | Edits a task. Refused while a run is going (`E7003`) or in a final column (`E7004`) |
| `POST /tasks/{id}/actions` `{action, model?, effort?, text?}` | §2. Returns 200 with the task, or 409 with `E7001` (not allowed here; `allowed` lists what is) or `E7002`–`E7008` (`E7007`, `E7008`: a revert blocked by a later task) |
| `POST /tasks/{id}/notes` `{text}` | Adds a human note |
| `POST /tasks/{id}/files` (the image's bytes as the body) | Stores an image (PNG, JPEG, GIF or WebP by content; at most 5 MB) and returns `{name, url}`. A note uses it as `![alt](url)` (D-37) |
| `GET /tasks/{id}/files/{name}` | The stored image |
| `DELETE /tasks/{id}/notes/{i}` | Deletes the note at position `i` (0 is the oldest), whoever wrote it. Notes are only ever appended, so a position stays valid until a note before it is deleted |
| `POST /tasks/{id}/questions/{qid}` `{option?, text?}` | Saves the answer and continues the same waiting run once all questions are answered, unless paused; uses its allowed workflow continuation (§6). For a settings question, `Approve` writes `proposed` if `solace.toml` still equals `base` and is valid; otherwise the answer records that it was not applied |
| `GET /models` | The models this Solace install knows, each with `agent`, `efforts` (the efforts an action may choose; empty for custom runners), `available` and, when unavailable, `reason` (§6) |
| `GET /events` | Server-sent events: `{task}` in full after every change, and `{workflow}` after a configuration change, in order. A client that reconnects with `Last-Event-ID` gets what it missed; a new one gets every change so far. Once a connection has sent what was missed, it sends one `ready` event (`event: ready`), so a client can apply the replay at once and treat only later events as news |

A `text` sent with an action is added as a human note before the action runs.

`solace serve` also serves the default board at `/` and a guide for humans at `/guide`.

### One root, several projects (D-57)

A **Solace root** is one folder; its projects are the root itself (`.`, when it has `gleam.toml`) and folders below it. The root's `solace.toml` lists the folders: `projects = ["bench/pilot", "web"]`, each relative, without `.` or `..`, reached without symlinks, and not itself listing projects. A root without `gleam.toml` holds only that key; a root project's `solace.toml` also holds its own settings. A project with no root above it is its own root, with itself as its only project, which is every project before D-57. Each project keeps its own store, settings, lockdown, tasks and runs.

There is one root: `solace serve` and `solace create` use the root that holds their folder, so they act the same anywhere below it. That is the one folder above that lists projects (or holds only a `solace.toml`), else the outermost project. A root inside another root, or inside a project, is refused (`E6020`). With no root, `serve` makes its folder an empty root, and `create` sets up the project there, or, in a folder without `gleam.toml`, offers each Gleam project below (skipping build output, dependencies, ignored folders and symlinks) and lists the ones confirmed. `create` in a project below a root lists it in that root. A server holds each project it serves (`.solace/serve.lock`); a second server for the same project is refused.

The root's own project answers at `/` as above; every other project's API is under `/p/{dir}`, `dir` percent-encoded (`/p/bench%2Fpilot/tasks`, `/p/bench%2Fpilot/view`). A note's image URL stays project-relative (`/tasks/{id}/files/{name}`); a client prefixes the project's `base`. The root adds:

| Request | Effect |
| --- | --- |
| `GET /projects` | `{workspace, projects: [{dir, label, name, base}], found: [{dir, name, imported, member}]}`. `workspace` is true when the root lists projects or has no `gleam.toml`, so a board shows lanes. `label` is the folder (the root's own folder name for `.`), `name` comes from `gleam.toml`, and `base` is the API prefix (`""` for `.`). `found` lists the Gleam projects in the root |
| `POST /projects` `{dir}` | Lists the project at `dir` again, keeping its store; only a project that already has a store (one removed from the list, D-59). Existing code without a store comes in by migration (below). Refused if `dir` lies in another project's `src/` or `test/`, lists projects of its own, or another server holds it |
| `POST /projects/new` `{name, dir?, target?}` | `gleam new` at `dir` (default `name`), with `--template javascript` for `target: "javascript"`, without its own git repository inside one; sets it up and lists it. Refused if `dir` exists or `name` isn't a package name |
| `DELETE /projects/{dir}` | Takes the project off the list; its `.solace` stays. Refused for `.` and while one of its agents is working (`E7003`) |
| `GET /models` | As above |
| `GET /events` | When `workspace`: every project's events, each with `project: dir`, and `{projects: [dir...]}` when the list changes. The event id is a JSON object of each project's position, so `Last-Event-ID` resumes all of them. A project added later starts at its latest event. One `ready` follows the replay. Otherwise the root project's stream as above. `/p/{dir}/events` is one project's stream alone |

Reads never take a project's lock, so humans and agents read any number of projects at once; an agent reads other projects with `--project` (Spec 3 §2). A task, its runs and its changes belong to one project, and each project has at most one work run (§6).

**Later: tasks across projects (not built).** A task may cut across several projects. It will take each project's lock and work-run slot as it changes that project, in folder order so two such tasks can't deadlock, and its changes and undo will span those projects.

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
- **Endpoints.** A model may name an `endpoint = {kind, base_url, key_env?}`. `kind = "openai"` goes with `agent = "codex"` and is passed to Codex as a custom provider using the Responses API. `kind = "anthropic"` goes with `agent = "claude"` and is passed to Claude Code as `ANTHROPIC_BASE_URL`, plus `ANTHROPIC_AUTH_TOKEN` taken from `key_env`. Solace stores no credentials: agents use their own sign-in, and endpoint keys are read from the named environment variable when the agent starts. A mismatched kind is `E6020`.
- **Availability.** `/models` marks a model unavailable, with the reason, when its agent program isn't on `PATH`, when the agent isn't signed in, or when the endpoint's `key_env` isn't set. Only local things are checked, so the check costs nothing. Starting a run on an unavailable model is refused (`E7011`), and the task stays where it is.
- **Attention.** Solace sets `attention` when a task needs a human: an agent asked a question, a run failed (the note includes the agent's last message, or else the end of its output), or the server restarted while an agent was working. It is cleared by the next successful action request on the task.
- A **work run** starts the agent with Spec 14 §5's configuration. Its prompt is plain text, not the task's JSON: what the run is for and how to start, ask and finish, then the task's title, body, links, notes and questions with their answers. Images in the briefed notes are attached to the prompt (Codex `-i`, Claude Code a stream-json image block) and named `[image N]` in the text; a custom runner gets `[image N: path]`. The run's kind decides the role line, and a read-only run's brief says which commands are refused. On top of Spec 14, the agent's Solace tool gets:
  - `task note <text>`: adds an agent note;
  - `write solace.toml` (D-38) validates the proposed text (`E6020` if invalid), then adds a settings question with options Approve and Reject and waits exactly like `task ask`. Only Approve writes the file;
  - `task ask` (`text`, 2–4 `options`): adds a question and fires `asked`. **The agent waits:** answering the last pending question automatically continues the same run through its allowed workflow action (`unblock` by default for work). The call returns the answers and any continuation `text`. Explicit `unblock` with `text` still works. The run keeps its context. `new_agent` instead ends it and starts a fresh one, which gets the notes and answers in its prompt;
  - `task done <summary>`: adds the summary as a note, fires `finished`, and ends the run.
- An **investigate run** gets the same prompt, with a role line asking for findings or a plan. It is read-only like a review run: `write`, `submit`, `delete`, `remove`, `format` and `run` are refused (`E7010`). Its `task done` report is added as a note and ends the run, and fires no event: the task stays where it is. Its `task ask` fires no event either. The task stays in its column, marked as needing attention, and answering its pending questions automatically continues the waiting agent. Explicit `investigate` with an answer still works. A later work run gets the report in its prompt with the other notes.
- A **review run** gets the same prompt plus the names of the task's `changes`. It is read-only (`E7010`), ends by adding its feedback with `task note`, and fires no event.
- A run past the task timeout (a system setting; default 60 min) is stopped and fires `failed`. Time spent waiting on a question or paused doesn't count.
- **Pause and continuation:** answering a paused run saves the answers without continuing it; `resume` continues it once all questions are answered. Answer submission never starts a new agent, replaces a run, grants a stamp or reverts work. A custom workflow without an allowed continuation keeps the answers and flags the missing action. Repeated answer submissions do not start another run.
- **Keeping an agent waiting** depends on the agent's tool-call timeout. `agent-config` (Spec 14 §5) sets it to the task timeout or more (Codex: the MCP server's `tool_timeout_sec`; Claude Code: `MCP_TOOL_TIMEOUT`). If the agent gives up anyway, the run fires `failed`, and the task continues with a new agent.
- Each run records `log`, the path of the agent's own session log. `GET /tasks/{id}/runs/{rid}` reads it (Codex `exec --json` or Claude Code `stream-json`) into a transcript, so a human can follow the agent's work live or read it afterwards. The view is read only (D-19, D-24).

The bundled board confirms `cancel` and `complete` before sending an action request (D-32). Dialogs name the task and explain reverting changes versus keeping changes and removing undo records. Dismissing sends no request and preserves the form. Confirmation does not bypass API gates.

### Conversation continuity (D-33)

Investigation, work and reassigned fixes resume the author's compatible native conversation. Reviews always start fresh. Compatibility requires matching provider, model, endpoint and effort; explicit `new_work` and revert actions start fresh. Missing/incompatible state and custom runners receive a handoff automatically. A session-load failure retries once with a fresh conversation only before tools execute. Other failures, including usage limits, still require human action.

Finishing tools end the board run immediately, then allow up to 30 seconds for graceful process exit. Further tool calls from the ended run are refused. Follow-up launches wait for that exit. Author state lasts until finalization or replacement; review state is deleted after exit. Only an exited process yields reusable state. Claude keychain-only authentication on macOS uses fresh nonpersistent conversations to preserve sign-in.

Briefs contain changed entities, feedback, answers, recent outcomes/activity, drafts and recorded check/test/build results. Results are marked stale when source or watched-file contents change. Resumed authors receive new notes and skip broad orientation; deleting older notes must not hide newer feedback. Fresh handoffs include all retained notes.

## 7. Out of scope

Several projects on one server, several people, remote access and authentication, and more than one work run per project at a time. Asking a running agent side questions that don't disturb its work (D-19).

## Tests (acceptance)

- A1. With the default workflow, every configured action is tested from each of its columns, and every other column and action pair returns `E7001` with the right `allowed`.
- A2. No action moves a task to an event's target column except through that column's actions. `asked`, `finished` and `failed` move it only as configured.
- A3. A custom workflow with an extra column and action works end to end. The invalid workflows in §2 each give `E6020`.
- A4. `complete` from `needs_review` without `approved` is refused (`E7006`). After `approve`, `complete` moves the task to the `complete` column. `start_over` and `cancel` from `needs_review` are not gated, and `start_over` removes the stamp. The stamp records who gave it and when. A task approved and then moved back with `start_over` or `new_agent` no longer holds `approved`.
- A5. With a fake agent: `task ask` blocks the task, and the question appears on it and in `/events`, while the agent's call stays open. `unblock` is refused until the question is answered or a `text` is given. After the last answer is submitted, the same agent process (same run id) receives the answer and continues without an extra `unblock` request. A paused run waits for `resume`. Custom continuation action names work, and repeated answers never launch a replacement run. `new_agent` instead ends it and starts a run with a new id whose prompt has the answer. `task done` moves the task to `needs_review`.
- A6. A fake agent's run edits two symbols, creates one, deletes one, writes a watched file and creates a new one. `cancel` restores the store and those files byte-for-byte to how they were before the task.
- A7. Task B changes a symbol after task A did. Cancelling A is refused with `E7007`, naming the symbol and B, and nothing changes. After B is cancelled, cancelling A succeeds.
- A7c. Task B adds a caller of a function task A added, without editing any of A's symbols. Cancelling A is refused with `E7008`, naming B's caller, and the store and files are unchanged.
- A7b. After a task deletes a symbol and edits another, `find`, `inspect`, `view`, `refs` and `session start --task` show no trace of the deleted symbol's earlier text or of the edited symbol's old text, and give the same output as a store with no before-state records.
- A8. After `pause`, the agent's next Solace call is held and its process stays alive. `resume` releases the call and returns the task to `paused_from`. Time paused doesn't count toward the task timeout.
- A12. `/models` includes the presets and reports a model whose program is missing as unavailable, with a reason. Assigning it returns `E7011` and leaves the task in its column. A failed run and an agent's question each set `attention`, and the next action clears it.
- A13. A Codex log and a Claude Code log each give a transcript with the agent's messages in order and each tool call's command, output and status. A run that exits without finishing sets `attention` with the agent's last message rather than raw log lines. The default board shows which tasks have an agent working, formats notes and messages as markdown with any HTML escaped, and refreshes a followed run's transcript until it ends.
- A9. A review run's `write` or `submit` returns `E7010`, and the task stays in its column. So does an investigate run's. An investigate run's question keeps the task in planning, and `investigate` with an answer continues the same run. Its `task done` report appears on the task, which stays in planning and can be completed from there.
- A10. `/events` delivers every change in order, and a reconnect with `Last-Event-ID` returns the missed ones. A connection sends `ready` once it has sent what was missed.
- A11. The default board's only connection to Solace is this API: it runs against a stub server implementing §4 and passes its own tests there. Every file command on `.solace/board/` returns `E6006`.

- A14. Reassigning after review retains existing symbol and file edits and before-state records, passes review feedback and human instructions to the new work run, and clears approval. The revised result requires fresh approval, and cancelling restores the state before the original work run.
- A16. `/models` lists each model's `efforts`. An action's `effort` is recorded on the run and reaches the agent; an effort the model doesn't offer is `E7001`. A successful `task done` sets `finished`, and the next action clears it. An uploaded PNG is served back as `image/png`; SVG or over 5 MB is `E7001`. A note referencing an uploaded image reaches the agent's brief as `[image 1: path]`.
- A15. Fake native Codex and Claude launchers retain author conversations through investigate/work/reassign, use fresh reviewers, fall back on lost sessions, enforce current run permissions, and clean up on completion/cancellation. Model changes and start-over create fresh conversations; quota errors are not retried. Graceful exit persists state after the finishing tool returns; hung shutdown is bounded. Deleted notes do not swallow later feedback; intervening edits mark previous checks stale.
- A17. With a fake agent in a work run, an invalid `write solace.toml` is `E6020` and adds no question. A valid one waits on a settings question; Reject leaves the file unchanged and the agent receives the answer; Approve writes it; Approve after a human edit to the file leaves it unchanged and says so. A review run's `write solace.toml` is `E7010`.
- U1. An edit to a module file while no run is going is read in by the board's poll, the file keeps the edit, and an `Unexpected Changes` task with the hazard flag holds the change and its compile result; another task's `assign` is `E7012`; `cancel` restores the file and the store (`outside::tests`, `tests/workspace_acceptance.py`). Only the edited project gets the task.
- U2. A file changed during a command isn't written over; a file defining a name twice is left as it is and reported once; a store with no record counts what it holds as written.
- U3. A change whose hash is on `.solace/approved` is read in without a task; completing an `Unexpected Changes` task adds its changes to the list.
- W1. One root (`tests/workspace_acceptance.py`): `solace create` in a folder offers each Gleam project and adds only those confirmed; lanes are keyed by folder when package names repeat; `/p/{dir}` reaches only its project; merged events carry `project` and resume from a combined id; a second server for a project, started from inside it, is refused; `create` in a project below the root lists it there; a project created from the board has a store; removal keeps it.
- W2. Nothing leaves the root: symlinked and `..` folders, a project that lists projects, and a root inside a root or a project are refused. A root that is a project serves itself at `/` and its other projects under `/p/`; its settings keep their comments when `projects` is written; it can't be removed.
- W3. Agents in two projects work at the same time; each reads the other's code with `--project`, and changes there are refused (Spec 3 A8).
