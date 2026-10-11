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

Events to the host: `task-moved` (`{id, dir, base}`: the task now lives in another of its projects)
and `notice` (`{heading, text, id, tab}`, from its own stream only; a host raises its own).

Its styles set no colours: the page's custom properties (`--paper`, `--ink`, ...) reach it from its host,
so the board's dark theme applies. A page that hosts it outside the board defines them.

The last agent and effort chosen are kept in the browser under `solace-board-agent` and
`solace-board-effort`.

## Proposal display

`solace_task.proposal_view` shows a strategy run's proposal (D-93, Spec 15 §6) in human terms, for the
Strategy page's planning panel: "Create task *Index*", "Change feature *Search*" or "Replace the vision",
the proposed fields (title first, the body and the vision's text as Markdown), a line for each record
Approve also sets back to `planned`, then the question's options until it is answered and the answer
after. `proposal_decoder` reads `{kind, id?, fields, base?, reopens?}`; `proposal_styles` is its look,
with colours from the page. `entry_view`, `entry_decoder`, `note_decoder` and `agent_decoder` are public
for the same panel.

## Checks

```sh
gleam test           # the Gleam tests
gleam build && bun test test/task_api.test.mjs   # the browser helpers and the element's flows
```
