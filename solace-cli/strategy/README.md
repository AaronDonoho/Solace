# strategy

The Strategy page (D-90): a Lustre web component, `<solace-strategy>`, compiled to JavaScript. The shell
(`solace-cli/frontend`) registers it and shows it at `/strategy`.

It draws a tablist with four panels, one per tab: Vision, Features, Roadmap, Tasks. The tab shown is kept in the
URL (`/strategy?tab=tasks`), so a reload returns to it; an unknown or missing value shows Vision.

## Record state labels

Feature and milestone state labels share one color mapping: `proposed` is purple, `upcoming` is blue, `active` is
green, `complete` is black, and `canceled` is gray. The labels have readable text in both themes and stay uppercase;
an unknown state keeps the neutral label. Task workflow columns and the `<solace-task>` status badge are separate.

## Vision tab

The root's vision (D-91): one Markdown document, shown with `md` and edited with `editor` from the components
project (and its `editor_styles`).

- **Data.** The root's `/events` replays `{vision: {text, revision}}` and sends it after every write.
- **Limit.** At most 1000 characters, counted as code points as the server does (D-92). While editing, a line
  below the text box reads "973 / 1000 character limit"; past the limit it is an alert and **Save** is disabled.
- **Saving.** `PUT /vision {text, base}`, where `base` is the revision the edit began from. The server refuses
  another (`E7026`): the draft stays, and once the stream shows a newer vision the page says so and offers
  **Reload**, which takes the saved text.

## Features tab

The root's features grouped by target milestone (D-87), with complete and canceled features in one `Closed`
section after the milestones, and a form to create and edit them, drawn by `src/features.gleam` (D-90: one
component per tab).

- **Data.** The root's `/events` replays `{milestones}` and `{features}` and sends `{milestone}`, `{feature}`,
  `{removedMilestone}` and `{removedFeature}` as they change. A feature is `{id, title, body, milestone?, rank,
  state}` and a milestone `{id, title, body, rank, targetDate?, state}`; every field but `id` is optional (no state
  reads as `proposed`, no rank as 0).
- **Groups.** Milestones in rank order (then id), empty ones too ("No features."), then "No milestone" for the
  open features with none (or one not loaded), shown only when there are any, then "Closed" for every feature
  whose state is `complete` or `canceled`, shown only when there are any. A closed feature appears only under
  "Closed", even when it has a milestone. Inside every group, features by rank, then id. The Roadmap tab is
  unchanged and still shows features under their milestones.
- **A feature** shows its title, a state badge, its body as Markdown (`solace_task.md`) and its linked tasks: the
  tasks of every project with `feature == id`. A task in the `complete` or `cancelled` column shows the run
  transcript's green ✓ or red ✗; other tasks show neither. Clicking a task sends `Open(dir, id)`: the task opens
  below the list on the Tasks tab, which is selected.
- **Create and edit.** **New feature** opens a form above the groups, and a feature's **Edit** opens it in the
  feature's place: a title, the description in `solace_task.editor` (textarea `feature-text`), a state select
  (`proposed`, `upcoming`, `active`, `complete`, `canceled`) and a milestone select with "(none)". **Save** and
  **Cancel** sit below the description. **Save** sends
  `POST /features {title, body, state, milestone?, request}`, where `request` is an id chosen when the form opened,
  so a repeated save makes one feature; an edit sends `PATCH /features/{id}` with only the changed fields (a cleared
  milestone as null), and an edit that changed nothing just closes. One save at a time. The reply ends the edit; a
  refusal (`E7001`, such as a missing title, or `E7023`, an unknown milestone) shows in an alert and keeps the form.
- **Reorder.** A feature's **Up** and **Down** move it inside its group with one `PATCH /features/{id} {rank}`:
  the midpoint of its new neighbours' ranks, or the neighbour's rank ∓ 1 at either end (`features.move_rank`). The
  first feature's Up and the last one's Down are disabled. Features with equal ranks (a missing rank reads as 0)
  keep their order by id, so a move between them changes nothing visible.
- **Start.** A `proposed` or `upcoming` feature's **Start** sends `PATCH /features/{id} {state: "active"}` (D-154);
  the server keeps its start time and color (D-114) and plans its Backlog tasks (D-134). An active, complete or
  canceled feature has no Start. A refused start shows in the alert above the groups, as a refused move does.
- **Delete.** **Delete** asks inside the page: "Delete *title*? Its N tasks lose their link to it; the tasks stay."
  (N counts the linked tasks of every project). Confirming sends `DELETE /features/{id}`; the server clears the
  tasks' `feature`, and each keeps the milestone it had; its `{task}` and `{removedFeature}` events update the page.
  A refused move or delete shows in an alert above the groups.
- **New task.** Each feature ends with a small form: a title and **Add task** (disabled while the title is blank).
  It sends `POST {base}/tasks {title, body: "", request}`, then `PATCH {base}/tasks/{id} {feature}` through the
  same `Link`/`Patched` as the Tasks tab's inline links. `request` is chosen per feature and kept until a create
  succeeds, so a retry makes one task. In a workspace a project select picks `base`, defaulting to the root's own
  project; with a single project there is no picker. One create at a time. Each step reports on its feature: a
  refused create keeps the title ("Couldn't create the task: …"), and a refused link says "The task was created
  without the link to this feature: …". The Tasks tab has its own **New task** form (below), and the two share
  the one-create-at-a-time limit.

## Roadmap tab

The root's milestones, each with its features and their tasks, drawn by `src/roadmap.gleam` (D-90).

- **Data.** The same stream as the Features tab, and every project's tasks with their `dependsOn` (a task saved
  before D-87 reads as `[]`).
- **Groups.** Milestones by target date, earliest first; those with no date follow by rank then id, empty ones
  too. A task is in its own milestone, else in its feature's target. A milestone holds the features that target it
  (D-188), each once, by rank then id, each with its tasks there; then its tasks with no feature, under "No feature".
  A feature never appears under a milestone only because it has a task there. Last, "No milestone", when it holds
  anything: the features in no milestone or one not loaded. A task with neither a milestone nor a feature is not on
  the roadmap (the Tasks tab lists it).
- **A milestone** starts collapsed. Its heading shows the title, a state badge, its `targetDate` and its feature
  progress, with **Edit** and **Delete**; progress counts the group's features that are not `canceled` (D-87 point
  2), and of those, the ones that are `complete`: "2 / 3 features complete". One counted feature reads
  "1 / 1 feature complete", and none reads "0 / 0 features complete". The count stays in the heading whether the
  milestone is open or closed. Clicking the heading opens its body as Markdown, its features and their tasks;
  clicking again closes it. "No milestone" has no heading controls and stays open. A milestone link (D-122) opens
  it before scrolling to it; a feature link opens its target milestone, and **Edit** also opens it. The open
  milestones last for the page session only.
  **New milestone** opens a form above the groups, and **Edit** opens it in place of the heading: a title, a date
  input for `targetDate`, the description (textarea `milestone-text`) and a state select. **Save** and **Cancel**
  sit below the description. **Save** sends `POST /milestones {title, body, state, targetDate?, request}`, or
  `PATCH /milestones/{id}` with only the changed
  fields (a cleared date as null), as the feature form does; a refusal shows in an alert and keeps the form. An
  **Edit** also lists the features targeting the milestone, by rank then id, each with **Remove**, then a search over
  the other features by title.
- **Delete.** "Delete *title*? Its N features and M tasks lose their link to it; they stay." (the features that
  target it, the tasks whose milestone it is). Confirming sends `DELETE /milestones/{id}`; the server's events
  update the page.
- **Features in a milestone.** An edited milestone lists the features targeting it, by rank then id, each with
  **Remove**; below is a search over the other features, with **Add**. Every word of the search must be in the title,
  ignoring case, and the first 10 show. **Add** sends `PATCH /features/{id} {milestone}` and **Remove** sends
  `{milestone: null}`; both apply at once, so **Save** and **Cancel** do not undo them. Adding a feature in another
  milestone moves it. A refusal shows in the form's alert above **Save**.
- **Dependencies.** A task with dependencies says "depends on" and their titles (with the project, for another
  project's task). A task's **Dependencies** opens its editor, one task at a time: each dependency with **Remove**,
  then a search over every task of the root, in any project or milestone (the server checks `dependsOn` across the
  root, D-87). Every word must be in the title; open tasks come first, then closed ones, by title; the first 10 show.
  It leaves out the task itself, its dependencies, and the tasks that depend on it, directly or through others,
  since those would make a cycle. **Add** or **Remove** sends `PATCH {base}/tasks/{id} {dependsOn}` with the whole
  list. The server's refusal shows in the alert above the groups: `E7024` (itself or a
  missing task), `E7025` (a cycle), `E7003` (a run is going, D-100) or `E7004` (a final column).

## Tasks tab

Every task of every project, one row each: title, column, project (in a workspace), feature, milestone,
`createdTime` (D-130) and `closedTime`, both as the reader's local date and time.

Backlog tasks are listed here too: the board leaves Backlog out (D-133), so this page is where a
Backlog task is planned (**Plan**, D-132) and where a planned one goes back on hold (**Back to
backlog**). A task's details offer **Back to planning** from Assigned, Review and Canceled (D-133).

- **Data.** `GET /projects`, each project's `GET {base}/workflow`, and the root's `/events`. A workspace root's own
  `GET /tasks` holds only its project, so the list comes from the merged stream: its replay sends each project's
  `{tasks}` (with `project`), then the root's `{milestones}` and `{features}`, and the root task store's tasks
  whose home is no project (with `noProject: true`, D-129); updates follow live (Spec 15).
- **A task with no project (D-129).** The root task store can hold a task whose home is no project of the
  workspace: it is in no board lane. The tab lists it with its project cell reading "No project", the project
  filter offers "(no project)", and a task link opens it in the bottom band with its details read from the root;
  the human can cancel and archive it there. Its column labels come from the root's workflow.
- **Sort and filter on the client.** Filters for column, project, feature and milestone ("(none)" keeps tasks
  without one); the search needs every word in the title or the body. Notes and changes are not in `GET /tasks`,
  so they are not searched; a server search is left to the mass-edit/archive work. Filters and search apply
  first, then the sort, then paging. A query with text ranks the tasks it keeps: its id or the whole phrase in
  the title first, then every word in the title, then the rest, each group in the default order. A clicked
  heading's sort wins over the ranking.
- **Sort by a heading.** Until a heading is clicked: closed tasks newest first, then open ones by title. Every
  heading is a button (`SortBy`): the first click sorts ascending, the next descending, and so on; the sorted
  heading alone shows ▲ or ▼ and sets `aria-sort`. Column follows the workflow's column order; project sorts by
  label with "No project" last; feature and milestone by title; created and closed by time. A task without the
  value (no feature, milestone or time) goes last either way, and ties fall back to the title, then the id. A
  new sort goes back to page 1. The sort stays across filter changes and tab switches for the page session, not
  across reloads; the open row stays open and its details stay in the bottom band.
- **Inline links.** Each row's feature select sends `PATCH {base}/tasks/{id}` with `{feature}` (null clears it),
  allowed in final columns and during runs (D-87, D-100). The Milestone cell is a read-only link to
  `/milestone/{id}` (D-122): the loaded title, or the id when the milestone is not loaded; a task with no
  milestone shows `—`. A task with a feature takes its feature's milestone; the server refuses a direct change
  (`E7001`).
- **New task.** **New task** above the filters opens a form there: a title, the description in
  `solace_task.editor` (textarea `new-task-text`, the feature form's tools), a project select in a workspace
  (defaulting to the root's own project; none with a single project), and feature and milestone selects with
  "(none)"; the milestone select hides while a feature is picked, since the task takes the feature's milestone.
  **Create**, below the description beside **Cancel** (disabled while the title is blank), sends
  `POST {base}/tasks {title, body, request}`, then, when a
  feature or milestone is picked, one `PATCH {base}/tasks/{id}`: the feature, whose milestone it takes, or else the
  milestone, through the same `Links`/`Patched` path as a row's Feature link. `request` is chosen when the form opens
  and kept until a create succeeds, so a retry makes one task; one create at a time, shared with the Features tab's
  form. A refused create keeps the draft ("Couldn't create the task: …"); a refused link says "The task was
  created without its feature: …" (or milestone), opens the task and leaves an empty form. On success the form
  closes and the task opens in the bottom band, as a row click does. **Cancel** closes the form, except while a create
  is sent.
- **Task links (D-98).** The shell's `focus` property, `{dir, id, n}`, opens the task on this tab, from any tab,
  and asks `<solace-task>` to scroll it into view (its `reveal`). The shell sends here a linked task that is
  archived, so off the board.
- **Details.** A row opens `<solace-task>` (components project) in a band pinned below the table and pager,
  `hosted`: this page hands it the task's events in `live`, with the updates of the tasks it depends on (for its
  Depends on line, from any project), and follows it when it moves to another project. The band's bar names the
  task, its column and status, and toggles its details; collapsed, only the bar shows, and the task stays mounted
  so its loaded state and events survive. Selecting a task expands the band; with no selection there is no band.
  The table takes the remaining height and scrolls. The page defines the full color set (light in the component's
  styles, dark in `index.html` and the shell), since `<solace-task>` sets none.

## Planning panel

A strategy run (D-93): an agent plans with the human. The panel is one component, `src/plan.gleam`, docked
beside every tab (after the tablist, outside the tab panels), so it stays open across tab switches. **Hide**
collapses it to one line: the shown run's state and model, or whether a run is open on the selection.

- **Data.** The root's `/events` replays `{strategyRuns}` (no notes) after `{vision}` and sends `{strategyRun}` in
  full after every change, with no `project`. A replayed run keeps the notes the page already has. The model list
  is `GET /models`, one entry per Claude family (see the components README). It loads with the page and refreshes
  when the Strategy page reconnects or catches up and after live run or task updates. A fresh list keeps the picked
  model and effort while they remain available and falls back only when the pick disappears; a refresh keeps an
  unavailable pick that is still listed, narrows or clears its effort, and leaves the old list unchanged when the
  refresh fails. The pick offers only models that can start, each shown by its label (`solace_task.agent_option`,
  D-150), and a remembered alias the list no longer shows becomes its family's entry (`solace_task.remembered`);
  under it, `solace_task.unavailable_view` gives the others' reasons, with Try again (`POST /models/try-again`) for
  a usage limit, and with none that can start, `solace_task.no_agents_view` says what to set up, or that every model
  is held by a usage limit. A run's state names the model its agent used when the start event reports one, otherwise
  its requested model, resolved to that same label; an unknown id stays raw.
- **Attention.** A live `{strategyRun}` where a run goes waiting (its agent replied) or asks a new proposal sends
  the host `attention` (D-107), which flags the Strategy tab while another page shows; a replay sends nothing.
- **Layout.** Below the shell's menu the page fills the window: the tab content scrolls on its own, and the panel
  runs from under the tab bar to the bottom of the window whatever its content. Its body scrolls inside it
  (`overscroll-behavior: contain`, so its scroll never moves the page). At 900px and narrower the panel stacks
  below the tabs as before, with no resize handle.
- **Width.** Drag the panel's left edge, or focus it (a `separator` with its value in pixels) and use the arrow
  keys (Left widens by 16px, Right narrows), Home (narrowest) and End (widest). It stays between 280px and 60% of
  the window. The width is kept in `localStorage` as `solace-strategy-plan-width`, stored when a drag ends or a
  key is pressed, and read when the page loads; without storage the panel is 380px.
- **Context.** Start plans the task open on the Tasks tab, else the whole project (`{kind: "page"}`). The panel
  says which: "Planning Context: Whole Project", or "Planning Context: task *title*" (feature and milestone the
  same way). A context with an open run offers **Open it** instead of Start (the server allows one, `E7003`).
- **Draft.** The message box has one draft for the page, not one per task, feature or milestone. Switching
  selection keeps what was typed. A successful Start or Send clears it only when the text is unchanged since
  the send.
- **Start.** The model and its effort when it offers several (an inline "Effort" label and rectangular
  segments, one per effort, with the model's default chosen until another is) sit side by side when
  they fit. Then the first message: `POST /strategy/runs {model, effort?, text, context}`. Choosing
  the default effort sends no effort. The reply's run is shown.
- **A run** shows what it is on, its state ("Working", "Waiting for you", "Ended: planned"), its notes (You, Agent,
  Solace), its proposals and the agent's transcript (`GET /strategy/runs/{rid}`, fetched again every 1.5 s while
  the agent works, in one chain that a reconnect or another run ends). While open: the message box, **Send**
  (`POST .../notes {text}`), **End** (`.../end`) and **Stop** (`.../stop`). An ended run is read-only.
- **Proposals** show with `solace_task.proposal_view` (components project): what is created or changed — an
  edit shows only the fields that change, as `old → new` with the body as a line diff, and says "No change"
  when none do (D-115, D-116) — the feature or milestone Approve sets back to `upcoming`, and
  **Approve**/**Reject**, plus **Approve to Planning** for a new task with no feature (D-201), which send
  `POST .../questions/{qid} {option}`; answered, the answer the server recorded.
- One request at a time; a refusal shows in the panel's alert.
- **Past runs.** "Runs on this" lists the runs on the context, newest first; each reopens.
- **Markers.** A task row, a feature card and a milestone heading with an open run show a pulsing "Planning" badge.

## Checks

`index.html` runs the component on its own, after `gleam build`. Run the tests with `gleam test`, then
`bun test test/api.test.mjs` (after `gleam build`) for the flow against a fake server.
