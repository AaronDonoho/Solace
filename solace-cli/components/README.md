# components

Elements shared by Solace's pages: Lustre web components compiled to JavaScript. The board, the shell
(`solace-cli/frontend`) and Strategy depend on this project by path.

## `<solace-task>`

One task's details: Details, Notes, Questions, Runs, Changes and, in a workspace, Scope, with the agent
choice, the note editor and the task's actions. It is `solace_task.gleam`, with its browser helpers in
`src/task_api.mjs` (markdown, editor formatting, image uploads, requests and the event stream).

```html
<solace-task base="/p/web" dir="web" task="<id>" workspace></solace-task>
```

- `base`: the API prefix of the task's project ("" for the root's). The element asks for the task
  (`GET /tasks/{id}`) and its workflow (`GET /workflow`) there, and makes its changes there.
- `dir`: the folder of the project the task is stored in; it picks this project's messages out of a
  workspace's merged stream and anchors the Scope tab.
- `task`: the task's id. `workspace`: show the Scope tab.
- The model list comes from `GET /models` and the Scope tab's projects from `GET /projects`.
- `hosted`: the host already follows `/events` and hands the task's messages down in the `live`
  property, so the element opens no stream of its own. Without it, the element follows `{base}/events`.
- `reveal` property, `{tab, id, n}`: show `tab` for task `id` and scroll it into view (a notice's button).

**Copy ID**, beside the title, copies the task's reference, `task:<id>` (`solace_task.reference`), and says
Copied for 2 s, or Copy failed when neither the clipboard nor the fallback took it. A later click's label isn't
cleared early by an earlier click's timer. Pasted into any text Solace renders, it is a link to the task (D-98): `markdown` in
`task_api.mjs` draws `task:<id>` and `[label](task:<id>)` as links to `/task/<id>`, which the shell follows.

**Depends on.** When the task's `dependsOn` isn't empty, the Details tab lists each dependency as a link to
`/task/<id>` (D-98), with its title and its column. A dependency not in a final column is marked
unfinished, as on the board (`solace_task.finished`; cancelled counts as finished). It is display only: the
Roadmap tab on Strategy edits dependencies. A dependency may be stored in any project, so the element asks
the root's `GET /tasks/find/{id}` where it is, then `GET {base}/tasks/{id}` there. One that can't be found
shows its id. The line follows a dependency's `{task}` updates under any project, and asks for a dependency
added to `dependsOn`. A host hands those updates down (the board and Strategy do). Without `hosted`, the
element's own `{base}/events` brings only its project's dependencies live; others refresh on a resync.

Events to the host: `task-moved` (`{id, dir, base}`: the task now lives in another of its projects)
and `notice` (`{heading, text, id, tab}`, from its own stream only; a host raises its own).

Its styles set no colors: the page's custom properties (`--paper`, `--ink`, ...) reach it from its host,
so the board's dark theme applies. A page that hosts it outside the board defines them.

The last agent and effort chosen are kept in the browser under `solace-board-agent` and
`solace-board-effort`.

## Proposal display

`solace_task.proposal_view` shows a strategy run's proposal (D-93, Spec 15 §6) in human terms, for the
Strategy page's planning panel: "Create task *Index*", "Change feature *Search*" or "Replace the vision",
then a line for each record Approve also sets back to `planned`, the question's options until it is
answered and the answer after. An edit shows only the fields that change (D-115): a short one as
`old → new`, a record id by its title, and the body as a line diff in its order — shared lines collapsed,
removed lines struck, added ones marked, ignoring the space around lines and blank lines — with a toggle
showing the new body as Markdown; the vision's text diffs against the vision the same way. A proposal with
nothing to change, the vision included, says "No change". A create, and a proposal saved before `before`
was kept, show every field, the body (and the vision's text) as Markdown. `proposal_decoder` reads
`{kind, id?, fields, base?, reopens?, before?}`; `proposal_styles` is its look, with colors from the
page. `entry_view`, `entry_decoder`, `note_decoder` and `agent_decoder` are public for the same panel.

## Checks

```sh
gleam test           # the Gleam tests
gleam build && bun test test/task_api.test.mjs   # the browser helpers and the element's flows
```
