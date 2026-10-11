//// `<solace-task>`: one task's details, notes, questions, runs, changes and
//// scope, shared by the board and the Strategy page. It asks the server for
//// its task and makes its own changes. A host that already follows `/events`
//// sets `hosted` and hands the task's events down in the `live` property;
//// otherwise the element follows its project's stream itself.

import gleam/dict
import gleam/dynamic
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import lustre
import lustre/attribute as a
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/event as e

@external(javascript, "./task_api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./task_api.mjs", "events")
fn events(
  base: String,
  receive: fn(String) -> Nil,
  ready: fn() -> Nil,
) -> Source

@external(javascript, "./task_api.mjs", "close_events")
fn close_events(source: Source) -> Nil

@external(javascript, "./task_api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

@external(javascript, "./task_api.mjs", "clock")
fn clock(seconds: Int) -> String

@external(javascript, "./task_api.mjs", "confirm")
fn confirm(question: String) -> Bool

@external(javascript, "./task_api.mjs", "markdown")
fn markdown(text: String, base: String) -> String

@external(javascript, "./task_api.mjs", "load")
fn load(key: String) -> String

@external(javascript, "./task_api.mjs", "store")
fn store(key: String, value: String) -> Nil

/// The markdown editor `id`'s text with its selection wrapped in `before` and
/// `after`; `root` is the shadow root that holds the editor.
@external(javascript, "./task_api.mjs", "wrap")
pub fn wrap(
  root: dynamic.Dynamic,
  id: String,
  text: String,
  before: String,
  after: String,
) -> String

/// The markdown editor `id`'s text with its selected lines made a block of
/// `kind`: "heading", "list", "numbered", "quote" or "code".
@external(javascript, "./task_api.mjs", "block")
pub fn block(
  root: dynamic.Dynamic,
  id: String,
  text: String,
  kind: String,
) -> String

@external(javascript, "./task_api.mjs", "images")
fn images(
  root: dynamic.Dynamic,
  insert: fn(String, String) -> Nil,
  fail: fn(String) -> Nil,
) -> Nil

@external(javascript, "./task_api.mjs", "reveal_task")
fn reveal_task(root: dynamic.Dynamic) -> Nil

@external(javascript, "./task_api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

pub type Source

pub type Column {
  Column(id: String, label: String, actions: List(String), final: Bool)
}

pub type Workflow {
  Workflow(columns: List(Column))
}

pub type Note {
  Note(author: String, run: String, text: String, time: Int)
}

/// One step of a run's transcript: a message, thinking, a tool call, token
/// usage, an error or raw output.
pub type Entry {
  Entry(
    kind: String,
    text: String,
    title: String,
    input: String,
    output: String,
    status: String,
  )
}

pub type Question {
  Question(
    id: String,
    text: String,
    options: List(String),
    answer: String,
    settings: Option(#(String, String)),
  )
}

pub type Run {
  Run(
    id: String,
    kind: String,
    state: String,
    model: String,
    effort: String,
    log: String,
    outcome: String,
    started: Int,
    ended: Int,
  )
}

pub type Link {
  Link(name: String, url: String)
}

/// One changed symbol, split so the Changes tab can group it.
pub type Change {
  Change(project: String, module: String, name: String)
}

pub type Task {
  Task(
    id: String,
    title: String,
    body: String,
    column: String,
    rank: Float,
    paused: String,
    notes: List(Note),
    questions: List(Question),
    runs: List(Run),
    links: List(Link),
    changes: List(Change),
    stamps: List(String),
    attention: String,
    finished: String,
    unexpected: Bool,
    /// The projects the task may change (D-61); empty for a task saved before.
    scope: List(String),
    /// Whether the task was archived: off the board, still in the archive (D-95).
    archived: Bool,
  )
}

pub type Agent {
  Agent(
    id: String,
    label: String,
    available: Bool,
    reason: String,
    effort: String,
    efforts: List(String),
  )
}

/// A bold heading, such as "Reviewed Fix checkout", and what happened. It
/// names its task (`dir` is the project it is stored in, filled in by the
/// host) and the tab that shows the news: "notes", "questions" or "runs".
pub type Notice {
  Notice(heading: String, text: String, dir: String, id: String, tab: String)
}

/// A project of the workspace, for the Scope tab: its folder, its name on
/// screen and its API prefix.
pub type Project {
  Project(dir: String, label: String, base: String)
}

pub type Model {
  Model(
    /// The `task` attribute: the id of the task shown.
    id: String,
    /// The `base` attribute: the API prefix of the task's project.
    base: String,
    /// The `dir` attribute: the folder of the project the task is stored in.
    dir: String,
    /// The `workspace` attribute: the root has several projects, so the
    /// Scope tab shows.
    workspace: Bool,
    /// The `hosted` attribute: the host hands down the task's events, so no
    /// stream of its own is opened.
    hosted: Bool,
    task: Option(Task),
    /// Counts the tasks opened, so a reply for one shown before is dropped.
    generation: Int,
    workflow: Workflow,
    projects: List(Project),
    models: List(Agent),
    model: String,
    effort: String,
    text: String,
    title: String,
    body: String,
    rank: String,
    error: String,
    editing: Bool,
    tab: String,
    previews: List(String),
    /// The action sent and not yet answered: its buttons are disabled
    /// meanwhile, so one click makes one request. "" when none.
    acting: String,
    watching: String,
    entries: List(Entry),
    /// The live transcript polling chain; an ended chain schedules nothing.
    poll: Int,
    review_summary: String,
    /// The task a notice asked to show, scrolled into view once it loads.
    revealing: String,
    /// Whether this element's own stream has replayed what it missed.
    caught_up: Bool,
    /// The number of the last message the host handed down that was applied.
    seen: Int,
    source: Option(Source),
    /// The shadow root, once it has rendered.
    root: dynamic.Dynamic,
    connected: Bool,
    /// Copy ID: how many copies were asked for (a reply or a timer for an
    /// older one changes nothing), and what its label says for a moment
    /// after the last: "Copied", "Copy failed", or "" for Copy ID.
    copy: #(Int, String),
  )
}

pub type Msg {
  TaskChanged(String)
  BaseChanged(String)
  DirChanged(String)
  WorkspaceChanged(Bool)
  HostedChanged(Bool)
  /// One `/events` message, from the host or from this element's stream.
  Live(String)
  /// Numbered `/events` messages from the host, oldest first; the ones past
  /// the last seen are applied in order.
  Handed(List(#(Int, String)))
  CaughtUp
  /// A notice's button: show the task on this tab and scroll it into view.
  Reveal(String, String)
  Connected
  Disconnected
  Attached(dynamic.Dynamic)
  Listening(Source)
  TaskLoaded(Int, String)
  /// The task fetched again while it is shown (see `refresh`).
  Refetched(Int, String)
  WorkflowLoaded(Int, String)
  ModelsLoaded(String)
  ProjectsLoaded(String)
  ReviewLoaded(String)
  Changed(String)
  Acted(String)
  AgentSelected(String)
  EffortSelected(String)
  Text(String)
  Title(String)
  Body(String)
  Rank(String)
  Edit(Bool)
  Tab(String)
  Preview(String, Bool)
  Format(String, String, String)
  /// The editor's selected lines become a block: "heading", "list",
  /// "numbered", "quote" or "code" (see `block` in task_api.mjs).
  Block(String, String)
  Failed(String)
  Save
  Action(String)
  AddNote
  DeleteNote(Int)
  Answer(String, String)
  SetScope(String, Bool)
  Watch(String)
  Unwatch
  TranscriptLoaded(String, String)
  Refresh(Int, String)
  TranscriptPolled(Int, String, String)
  /// Copy ID: the task's reference, `task:<id>`, goes to the clipboard (D-98).
  CopyId
  /// Copy number `n` is done: whether the clipboard took it.
  CopyDone(Int, Bool)
  /// Copy number `n`'s moment is over: the label says Copy ID again.
  CopyReset(Int)
}

/// While a watched run is going, its transcript is fetched again this often.
const refresh_ms = 1500

/// Browser storage keys for the last agent and effort chosen.
const agent_key = "solace-board-agent"

const effort_key = "solace-board-effort"

/// The Changes tab heading for files, listed after the code.
pub const files_heading = "Files"

/// The Changes tab heading for changes that are neither symbols nor files.
pub const other_heading = "Other"

/// Registers `<solace-task>`. A page whose other parts registered it already
/// is fine: the board, the shell and Strategy may each ask.
pub fn register() -> Result(Nil, lustre.Error) {
  let app =
    lustre.component(init, update, view, [
      component.on_attribute_change("task", fn(v) { Ok(TaskChanged(v)) }),
      component.on_attribute_change("base", fn(v) { Ok(BaseChanged(v)) }),
      component.on_attribute_change("dir", fn(v) { Ok(DirChanged(v)) }),
      component.on_attribute_change("workspace", fn(v) {
        Ok(WorkspaceChanged(v != "false"))
      }),
      component.on_attribute_change("hosted", fn(v) {
        Ok(HostedChanged(v != "false"))
      }),
      component.on_property_change(
        "live",
        decode.list({
          use n <- decode.field("n", decode.int)
          use text <- decode.field("text", decode.string)
          decode.success(#(n, text))
        })
          |> decode.map(Handed),
      ),
      component.on_property_change("reveal", {
        use tab <- decode.field("tab", decode.string)
        use id <- decode.field("id", decode.string)
        decode.success(Reveal(tab, id))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ])
  case lustre.register(app, "solace-task") {
    Error(lustre.ComponentAlreadyRegistered(_)) -> Ok(Nil)
    other -> other
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(
    Model(
      id: "",
      base: "",
      dir: "",
      workspace: False,
      hosted: False,
      task: None,
      generation: 0,
      workflow: Workflow([]),
      projects: [],
      models: [],
      model: load(agent_key),
      effort: load(effort_key),
      text: "",
      title: "",
      body: "",
      rank: "0",
      error: "",
      editing: False,
      tab: "details",
      previews: [],
      acting: "",
      watching: "",
      entries: [],
      poll: 0,
      review_summary: "",
      revealing: "",
      caught_up: False,
      seen: 0,
      source: None,
      root: no_root(),
      connected: False,
      copy: #(0, ""),
    ),
    // Attributes arrive before the element is added; nothing is asked until then.
    effect.none(),
  )
}

fn get(base: String, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(base, path, "GET", "", fn(text) { send(msg(text)) })
  })
}

/// A change to the task; the reply is the task as it is now.
fn send(
  model: Model,
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(
      model.base,
      path,
      method,
      json.to_string(json.object(fields)),
      fn(text) { dispatch(Changed(text)) },
    )
  })
}

fn error(text: String) -> String {
  json.parse(text, decode.at(["message"], decode.string))
  |> result.unwrap("Invalid API response")
}

/// Asks for the task and its project's workflow, and follows the task's
/// events unless the host hands them down.
fn fetch(model: Model) -> Effect(Msg) {
  case model.connected, model.id {
    True, id if id != "" -> {
      let generation = model.generation
      effect.batch([
        get(model.base, "/tasks/" <> id, TaskLoaded(generation, _)),
        get(model.base, "/workflow", WorkflowLoaded(generation, _)),
        listen(model),
      ])
    }
    _, _ -> effect.none()
  }
}

/// Shows the task the attributes name now: what belonged to the one shown
/// before goes.
fn open(model: Model) -> #(Model, Effect(Msg)) {
  let model =
    Model(
      ..stop(model),
      task: None,
      generation: model.generation + 1,
      workflow: Workflow([]),
      text: "",
      watching: "",
      entries: [],
      review_summary: "",
      editing: False,
      previews: [],
      acting: "",
      error: "",
      copy: #(model.copy.0 + 1, ""),
    )
  #(model, fetch(model))
}

fn listen(model: Model) -> Effect(Msg) {
  case model.hosted {
    True -> effect.none()
    False ->
      effect.from(fn(send) {
        send(
          Listening(
            events(model.base, fn(text) { send(Live(text)) }, fn() {
              send(CaughtUp)
            }),
          ),
        )
      })
  }
}

/// Closes this element's own stream; a new one replays what it missed first.
fn close(model: Model) -> Model {
  case model.source {
    Some(source) -> close_events(source)
    None -> Nil
  }
  Model(..model, source: None, caught_up: False)
}

/// Closes the stream and ends the transcript polling chain.
fn stop(model: Model) -> Model {
  Model(..close(model), poll: model.poll + 1)
}

fn transcript(model: Model, rid: String) -> Effect(Msg) {
  case model.task {
    Some(t) ->
      get(model.base, "/tasks/" <> t.id <> "/runs/" <> rid, fn(text) {
        TranscriptPolled(model.poll, rid, text)
      })
    None -> effect.none()
  }
}

/// The review summary of a task with changes, for the Changes tab.
fn review(model: Model, t: Task) -> Effect(Msg) {
  case t.changes {
    [] -> effect.none()
    _ -> get(model.base, "/graph?task=" <> t.id, ReviewLoaded)
  }
}

/// Scrolls the task into view and puts focus on its selected tab, once it
/// has rendered.
fn reveal() -> Effect(Msg) {
  effect.before_paint(fn(_, root) { reveal_task(root) })
}

/// The projects a task covers; a task saved before D-61 covers its own.
pub fn scope_of(home: String, t: Task) -> List(String) {
  case t.scope {
    [] -> [home]
    scope -> scope
  }
}

/// The task now lives in `dir`, one of its projects (D-61): it is asked for
/// there from now on, and the host hears of it in `task-moved`.
fn follow(model: Model, dir: String) -> #(Model, Effect(Msg)) {
  let base =
    list.find(model.projects, fn(p) { p.dir == dir })
    |> result.map(fn(p) { p.base })
    |> result.unwrap(model.base)
  let moved = Model(..close(model), dir: dir, base: base)
  #(
    moved,
    effect.batch([
      e.emit(
        "task-moved",
        json.object([
          #("id", json.string(model.id)),
          #("dir", json.string(dir)),
          #("base", json.string(base)),
        ]),
      ),
      case moved.connected {
        True -> listen(moved)
        False -> effect.none()
      },
    ]),
  )
}

/// The task left this project: it is opened from the first other project it
/// covers.
fn moved_away(model: Model) -> #(Model, Effect(Msg)) {
  let scope = case model.task {
    Some(t) -> scope_of(model.dir, t)
    None -> []
  }
  case
    list.find(model.projects, fn(p) {
      p.dir != model.dir && list.contains(scope, p.dir)
    })
  {
    Ok(p) -> {
      let #(model, fx) = follow(model, p.dir)
      #(
        model,
        effect.batch([
          fx,
          get(model.base, "/tasks/" <> model.id, TaskLoaded(model.generation, _)),
        ]),
      )
    }
    Error(_) -> #(model, effect.none())
  }
}

fn notice(n: Notice) -> Effect(Msg) {
  e.emit(
    "notice",
    json.object([
      #("heading", json.string(n.heading)),
      #("text", json.string(n.text)),
      #("id", json.string(n.id)),
      #("tab", json.string(n.tab)),
    ]),
  )
}

/// Applies one `/events` message about this task: its new state, its
/// project's workflow, or its removal from the project; a change to the
/// workspace's list of projects fetches the Scope tab's again. A workspace's
/// messages name their `project`; one for another project is not this task's,
/// unless it names this task, which is then fetched again (see `stale`).
fn live(model: Model, text: String) -> #(Model, Effect(Msg)) {
  let ours = case json.parse(text, decode.at(["project"], decode.string)) {
    Ok(dir) -> dir == model.dir
    Error(_) -> True
  }
  // What a new stream of its own replays is old: the task was fetched whole.
  case model.hosted || model.caught_up, stale(model, text), ours {
    False, _, _ | True, False, False -> #(model, effect.none())
    True, True, _ -> #(model, refresh(model))
    True, False, True ->
      case json.parse(text, decode.at(["task"], task_decoder())) {
        Ok(task) if task.id == model.id -> {
          // Events carry no `changes` (D-77): keep the list until it is fetched again.
          let old = model.task
          let task = case old {
            Some(t) -> Task(..task, changes: t.changes)
            None -> task
          }
          let model = Model(..model, task: Some(task))
          // A host raises its own notices.
          let news = case old, model.hosted {
            Some(t), False -> list.map(notices(t, task), notice)
            _, _ -> []
          }
          #(model, effect.batch([refresh(model), ..news]))
        }
        Ok(_) -> #(model, effect.none())
        Error(_) ->
          case json.parse(text, decode.at(["workflow"], workflow_decoder())) {
            Ok(w) -> #(Model(..model, workflow: w), effect.none())
            Error(_) ->
              case json.parse(text, decode.at(["removed"], decode.string)) {
                Ok(id) if id == model.id -> moved_away(model)
                _ ->
                  case projects_changed(text) {
                    True -> #(model, get("", "/projects", ProjectsLoaded))
                    False -> #(model, effect.none())
                  }
              }
          }
      }
  }
}

/// The text an editor shows, and the message that changes it.
fn field(model: Model, id: String) -> #(String, fn(String) -> Msg) {
  case id {
    "edit-body" -> #(model.body, Body)
    _ -> #(model.text, Text)
  }
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    // Added again (or for the first time): the task is asked for afresh, and a
    // followed run's transcript refreshes in one chain.
    Connected -> {
      let model = Model(..model, connected: True)
      #(
        model,
        effect.batch([
          get("", "/models", ModelsLoaded),
          get("", "/projects", ProjectsLoaded),
          fetch(model),
          case model.watching {
            "" -> effect.none()
            rid -> transcript(model, rid)
          },
          effect.before_paint(fn(send, root) { send(Attached(root)) }),
        ]),
      )
    }
    Attached(root) -> #(
      Model(..model, root: root),
      effect.from(fn(send) {
        images(root, fn(id, image) { send(Format(id, image, "")) }, fn(text) {
          send(Failed(text))
        })
      }),
    )
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    Disconnected -> #(Model(..stop(model), connected: False), effect.none())
    TaskChanged(id) if id == model.id -> #(model, effect.none())
    TaskChanged(id) -> open(Model(..model, id: id))
    BaseChanged(base) if base == model.base -> #(model, effect.none())
    BaseChanged(base) -> open(Model(..model, base: base))
    DirChanged(dir) -> #(Model(..model, dir: dir), effect.none())
    WorkspaceChanged(on) -> #(Model(..model, workspace: on), effect.none())
    HostedChanged(True) -> #(Model(..close(model), hosted: True), effect.none())
    HostedChanged(False) -> {
      let model = Model(..model, hosted: False)
      #(model, case model.connected, model.source {
        True, None -> listen(model)
        _, _ -> effect.none()
      })
    }
    Live(text) -> live(model, text)
    Handed(messages) -> {
      let fresh =
        messages
        |> list.filter(fn(m) { m.0 > model.seen })
        |> list.sort(fn(a, b) { int.compare(a.0, b.0) })
      list.fold(fresh, #(model, effect.none()), fn(acc, m) {
        let #(so_far, fx) = acc
        let #(next, more) =
          live(Model(..so_far, seen: int.max(so_far.seen, m.0)), m.1)
        #(next, effect.batch([fx, more]))
      })
    }
    // Its own stream (re)connected: anything missed meanwhile is fetched.
    CaughtUp -> #(Model(..model, caught_up: True), refresh(model))
    Reveal("", _) -> #(model, effect.none())
    // The host may name the task before or after it opens it: an open task
    // shows now, another once it loads.
    Reveal(tab, id) ->
      case model.task {
        Some(t) if t.id == id -> #(
          Model(..model, tab: tab, revealing: ""),
          reveal(),
        )
        _ -> #(Model(..model, tab: tab, revealing: id), effect.none())
      }
    TaskLoaded(generation, text) if generation == model.generation ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> {
          let shown = model.revealing != "" && model.revealing == task.id
          let model =
            Model(
              ..model,
              task: Some(task),
              title: task.title,
              body: task.body,
              rank: float.to_string(task.rank),
              error: "",
              revealing: case shown {
                True -> ""
                False -> model.revealing
              },
            )
          #(
            model,
            effect.batch([
              review(model, task),
              case shown {
                True -> reveal()
                False -> effect.none()
              },
            ]),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    TaskLoaded(_, _) -> #(model, effect.none())
    // The whole task is taken; fields being edited are the model's, so they
    // stay as typed.
    Refetched(generation, text) ->
      case model.task, json.parse(text, task_decoder()) {
        Some(t), Ok(fresh)
          if generation == model.generation && t.id == fresh.id
        -> #(Model(..model, task: Some(fresh)), review(model, fresh))
        _, _ -> #(model, effect.none())
      }
    WorkflowLoaded(generation, text) if generation == model.generation ->
      case json.parse(text, workflow_decoder()) {
        Ok(w) -> #(Model(..model, workflow: w), effect.none())
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    WorkflowLoaded(_, _) -> #(model, effect.none())
    ModelsLoaded(text) ->
      case json.parse(text, decode.list(agent_decoder())) {
        Ok(models) -> {
          // The remembered agent stays if it is still available; its effort
          // stays if the agent still offers it.
          let kept =
            list.find(models, fn(a) { a.available && a.id == model.model })
          let chosen = case kept {
            Ok(a) -> Ok(a)
            Error(_) -> list.find(models, fn(a) { a.available })
          }
          let id = result.map(chosen, fn(a) { a.id }) |> result.unwrap("")
          let effort = case kept {
            Ok(a) ->
              case list.contains(a.efforts, model.effort) {
                True -> model.effort
                False -> ""
              }
            Error(_) -> ""
          }
          #(
            Model(..model, models: models, model: id, effort: effort),
            effect.none(),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    // The Scope tab's projects; without them it lists none.
    ProjectsLoaded(text) ->
      case json.parse(text, projects_decoder()) {
        Ok(projects) -> #(Model(..model, projects: projects), effect.none())
        Error(_) -> #(model, effect.none())
      }
    ReviewLoaded(text) -> {
      let summary =
        json.parse(text, decode.at(["review", "summary"], decode.string))
        |> result.unwrap("")
      #(Model(..model, review_summary: summary), effect.none())
    }
    // The reply to an action; it may have waited for the agent's command (D-77 point 5).
    Acted(text) -> update(Model(..model, acting: ""), Changed(text))
    Changed(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> {
          // A reply carries no `changes` (D-77): keep the list until it is fetched again.
          let task = case model.task {
            Some(old) if old.id == task.id -> Task(..task, changes: old.changes)
            _ -> task
          }
          // A task whose home left its projects now lives in the first one left (D-61).
          let #(model, moved) = case
            model.dir != ""
            && !list.contains(scope_of(model.dir, task), model.dir),
            task.scope
          {
            True, [dir, ..] -> follow(model, dir)
            _, _ -> #(model, effect.none())
          }
          let model =
            Model(
              ..model,
              task: Some(task),
              title: task.title,
              body: task.body,
              rank: float.to_string(task.rank),
              text: "",
              error: "",
              editing: False,
              previews: [],
            )
          #(model, effect.batch([moved, refresh(model)]))
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    Watch(rid) -> #(
      Model(..model, watching: rid, entries: []),
      transcript(model, rid),
    )
    Unwatch -> #(Model(..model, watching: "", entries: []), effect.none())
    // One refresh chain per connection, including through a server restart: a
    // step from an ended chain (disconnected, or restarted) fetches and
    // schedules nothing.
    Refresh(poll, rid) if poll == model.poll && rid == model.watching -> #(
      model,
      transcript(model, rid),
    )
    Refresh(_, _) -> #(model, effect.none())
    TranscriptPolled(poll, rid, text) if poll == model.poll -> {
      let #(next, _) = update(model, TranscriptLoaded(rid, text))
      let running =
        rid == next.watching
        && json.parse(text, decode.at(["state"], decode.string))
        |> result.map(fn(state) { state != "ended" })
        |> result.unwrap(False)
      #(next, case running {
        True ->
          effect.from(fn(send) {
            after(refresh_ms, fn() { send(Refresh(poll, rid)) })
          })
        False -> effect.none()
      })
    }
    TranscriptPolled(_, _, _) -> #(model, effect.none())
    TranscriptLoaded(rid, text) if rid == model.watching -> {
      let decoder = {
        use state <- decode.field("state", decode.string)
        use entries <- decode.field("transcript", decode.list(entry_decoder()))
        decode.success(#(state, entries))
      }
      case json.parse(text, decoder) {
        Ok(#(_, entries)) -> #(Model(..model, entries: entries), effect.none())
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    }
    TranscriptLoaded(_, _) -> #(model, effect.none())
    // Efforts differ between models, so a new model starts at its default.
    AgentSelected(id) -> {
      store(agent_key, id)
      store(effort_key, "")
      #(Model(..model, model: id, effort: ""), effect.none())
    }
    EffortSelected(effort) -> {
      store(effort_key, effort)
      #(Model(..model, effort: effort), effect.none())
    }
    Text(text) -> #(Model(..model, text: text), effect.none())
    Title(text) -> #(Model(..model, title: text), effect.none())
    Body(text) -> #(Model(..model, body: text), effect.none())
    Rank(rank) -> #(Model(..model, rank: rank), effect.none())
    Edit(editing) ->
      case model.task {
        Some(t) -> #(
          Model(
            ..model,
            editing: editing,
            title: t.title,
            body: t.body,
            rank: float.to_string(t.rank),
          ),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    Tab(tab) -> #(Model(..model, tab: tab), effect.none())
    Preview(id, on) -> #(
      Model(..model, previews: case on {
        True -> [id, ..model.previews]
        False -> list.filter(model.previews, fn(p) { p != id })
      }),
      effect.none(),
    )
    Format(id, before, after) -> {
      let #(text, msg) = field(model, id)
      #(
        model,
        effect.from(fn(send) {
          send(msg(wrap(model.root, id, text, before, after)))
        }),
      )
    }
    Block(id, kind) -> {
      let #(text, msg) = field(model, id)
      #(
        model,
        effect.from(fn(send) { send(msg(block(model.root, id, text, kind))) }),
      )
    }
    Failed(message) -> #(Model(..model, error: message), effect.none())
    Save ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) ->
          case float.parse(model.rank) {
            Ok(rank) -> #(
              model,
              send(model, "/tasks/" <> t.id, "PATCH", [
                #("title", json.string(model.title)),
                #("body", json.string(model.body)),
                #("rank", json.float(rank)),
              ]),
            )
            Error(_) -> #(
              Model(..model, error: "Rank must be a number"),
              effect.none(),
            )
          }
      }
    Action(_) if model.acting != "" -> #(model, effect.none())
    Action(name) ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let fields = [#("action", json.string(name))]
          let fields = case model.model {
            "" -> fields
            id -> [#("model", json.string(id)), ..fields]
          }
          let fields = case model.effort {
            "" -> fields
            effort -> [#("effort", json.string(effort)), ..fields]
          }
          let fields = case model.text {
            "" -> fields
            text -> [#("text", json.string(text)), ..fields]
          }
          let proceed = case name {
            "cancel" ->
              confirm(
                "Cancel task \""
                <> t.title
                <> "\"? This stops its agents and reverts its changes. This cannot be undone.",
              )
            "complete" ->
              confirm(
                "Complete task \""
                <> t.title
                <> "\"? This keeps its changes and closes the task. Its undo records will be removed, so it cannot be reopened or reverted through the board.",
              )
            "stop" ->
              confirm(
                "Stop the agent working on \""
                <> t.title
                <> "\"? This ends its session; its changes are kept, and a later run starts a fresh agent.",
              )
            _ -> True
          }
          case proceed {
            True -> #(
              Model(..model, acting: name),
              effect.from(fn(dispatch) {
                request(
                  model.base,
                  "/tasks/" <> t.id <> "/actions",
                  "POST",
                  json.to_string(json.object(fields)),
                  fn(text) { dispatch(Acted(text)) },
                )
              }),
            )
            False -> #(model, effect.none())
          }
        }
      }
    AddNote ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) -> #(
          Model(..model, tab: "notes"),
          send(model, "/tasks/" <> t.id <> "/notes", "POST", [
            #("text", json.string(model.text)),
          ]),
        )
      }
    DeleteNote(i) ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) ->
          case confirm("Delete this note? It can't be restored.") {
            False -> #(model, effect.none())
            True -> #(
              model,
              send(
                model,
                "/tasks/" <> t.id <> "/notes/" <> int.to_string(i),
                "DELETE",
                [],
              ),
            )
          }
      }
    Answer(qid, option) ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let answer = case option {
            "" -> #("text", json.string(model.text))
            option -> #("option", json.string(option))
          }
          #(
            model,
            send(model, "/tasks/" <> t.id <> "/questions/" <> qid, "POST", [
              answer,
            ]),
          )
        }
      }
    SetScope(dir, on) ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let current = scope_of(model.dir, t)
          let wanted = case on {
            True -> [dir, ..current]
            False -> list.filter(current, fn(d) { d != dir })
          }
          // In the order the projects are listed, each once.
          let ordered =
            model.projects
            |> list.map(fn(p) { p.dir })
            |> list.filter(list.contains(wanted, _))
          #(
            model,
            send(model, "/tasks/" <> t.id <> "/scope", "POST", [
              #("scope", json.array(ordered, json.string)),
            ]),
          )
        }
      }
    // Pasted into any text, the reference is a link to the task (D-98).
    CopyId ->
      case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let n = model.copy.0 + 1
          #(
            Model(..model, copy: #(n, "")),
            effect.from(fn(send) {
              copy_text(reference(t.id), fn(ok) { send(CopyDone(n, ok)) })
            }),
          )
        }
      }
    CopyDone(n, ok) if n == model.copy.0 -> #(
      Model(
        ..model,
        copy: #(n, case ok {
          True -> "Copied"
          False -> "Copy failed"
        }),
      ),
      effect.from(fn(send) { after(copied_ms, fn() { send(CopyReset(n)) }) }),
    )
    CopyReset(n) if n == model.copy.0 -> #(
      Model(..model, copy: #(n, "")),
      effect.none(),
    )
    CopyDone(_, _) | CopyReset(_) -> #(model, effect.none())
  }
}

/// What the human should hear about in a live update: a run that ended, a
/// discussion reply, or a question an agent asked.
pub fn notices(old: Task, new: Task) -> List(Notice) {
  let ended =
    list.filter(new.runs, fn(r) {
      r.state == "ended"
      && r.kind != "discuss"
      && list.any(old.runs, fn(o) { o.id == r.id && o.state != "ended" })
    })
    |> list.map(fn(r) {
      // A run that left a note shows it; else its run is the place to look.
      let tab = case list.any(new.notes, fn(n) { n.run == r.id }) {
        True -> "notes"
        False -> "runs"
      }
      let notice = fn(heading, text) { Notice(heading, text, "", new.id, tab) }
      case r.outcome {
        "finished" ->
          notice("Finished " <> new.title, "The agent finished its work.")
        "reported" ->
          notice("Investigated " <> new.title, "The agent's report is ready.")
        "reviewed" -> notice("Reviewed " <> new.title, "The review is ready.")
        "" -> notice("Stopped " <> new.title, "The " <> r.kind <> " run ended.")
        outcome ->
          notice(
            "Stopped " <> new.title <> ": " <> outcome,
            "The " <> r.kind <> " run ended: " <> outcome <> ".",
          )
      }
    })
  let asked =
    list.filter(new.questions, fn(q) {
      q.answer == "" && !list.any(old.questions, fn(o) { o.id == q.id })
    })
    |> list.map(fn(q) {
      Notice("Question on " <> new.title, q.text, "", new.id, "questions")
    })
  // An agent's reply sets its discussion run waiting in the same update that
  // adds the note, so the run going waiting is the reply. Tasks loaded at
  // startup come without their notes, so the run's state is what to watch.
  let replied =
    list.filter(new.runs, fn(r) {
      r.kind == "discuss"
      && r.state == "waiting"
      && list.any(old.runs, fn(o) { o.id == r.id && o.state != "waiting" })
    })
    |> list.map(fn(r) {
      let text = case
        list.filter(new.notes, fn(n) { n.author == "agent" && n.run == r.id })
        |> list.sort(fn(a, b) { int.compare(b.time, a.time) })
        |> list.first
      {
        Ok(note) -> note.text
        Error(_) -> "The agent replied."
      }
      Notice("Reply on " <> new.title, text, "", new.id, "notes")
    })
  list.flatten([ended, replied, asked])
}

pub fn task_decoder() -> decode.Decoder(Task) {
  use id <- decode.field("id", decode.string)
  use title <- decode.field("title", decode.string)
  use body <- decode.field("body", decode.string)
  use column <- decode.field("column", decode.string)
  use rank <- decode.field("rank", decode.float)
  use paused <- decode.optional_field(
    "paused_from",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use notes <- decode.optional_field("notes", [], decode.list(note_decoder()))
  use questions <- decode.optional_field(
    "questions",
    [],
    decode.list(question_decoder()),
  )
  use runs <- decode.optional_field("runs", [], decode.list(run_decoder()))
  use links <- decode.optional_field("links", [], decode.list(link_decoder()))
  use changes <- decode.optional_field(
    "changes",
    [],
    decode.list({
      use name <- decode.field("name", decode.string)
      use entity <- decode.optional_field("entity", "", decode.string)
      use project <- decode.optional_field("project", "", decode.string)
      // A symbol's name is `module.symbol`, the symbol after the last dot; a
      // file's is `file:path`, and anything else is shown whole.
      let #(module, symbol) = case
        entity,
        list.reverse(string.split(name, "."))
      {
        "file:" <> path, _ -> #(files_heading, path)
        "symbol:" <> _, [symbol, ..rest] if rest != [] -> #(
          string.join(list.reverse(rest), "."),
          symbol,
        )
        _, _ -> #(other_heading, name)
      }
      decode.success(Change(project, module, symbol))
    }),
  )
  use stamps <- decode.optional_field(
    "stamps",
    [],
    decode.dict(decode.string, decode.dynamic) |> decode.map(dict.keys),
  )
  use attention <- decode.optional_field(
    "attention",
    "",
    decode.one_of(decode.at(["reason"], decode.string), [decode.success("")]),
  )
  use finished <- decode.optional_field(
    "finished",
    "",
    decode.one_of(decode.at(["outcome"], decode.string), [decode.success("")]),
  )
  use unexpected <- decode.optional_field("unexpected", False, decode.bool)
  use scope <- decode.optional_field("scope", [], decode.list(decode.string))
  use archived <- decode.optional_field(
    "archivedTime",
    False,
    decode.one_of(decode.int |> decode.map(fn(_) { True }), [
      decode.success(False),
    ]),
  )
  decode.success(Task(
    id,
    title,
    body,
    column,
    rank,
    paused,
    notes,
    questions,
    runs,
    links,
    changes,
    stamps,
    attention,
    finished,
    unexpected,
    scope,
    archived,
  ))
}

pub fn note_decoder() -> decode.Decoder(Note) {
  use author <- decode.field("author", decode.string)
  use run <- decode.optional_field(
    "run",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use text <- decode.field("text", decode.string)
  use time <- decode.optional_field("time", 0, decode.int)
  decode.success(Note(author, run, text, time))
}

pub fn entry_decoder() -> decode.Decoder(Entry) {
  let text = fn(name, next) {
    decode.optional_field(
      name,
      "",
      decode.one_of(decode.string, [decode.success("")]),
      next,
    )
  }
  use kind <- decode.field("kind", decode.string)
  use body <- text("text")
  use title <- text("title")
  use input <- text("input")
  use output <- text("output")
  use status <- text("status")
  decode.success(Entry(kind, body, title, input, output, status))
}

fn question_decoder() -> decode.Decoder(Question) {
  use id <- decode.field("id", decode.string)
  use text <- decode.field("text", decode.string)
  use options <- decode.field("options", decode.list(decode.string))
  use answer <- decode.optional_field("answer", "", decode.string)
  use settings <- decode.optional_field(
    "settings",
    None,
    decode.optional({
      use base <- decode.field("base", decode.string)
      use proposed <- decode.field("proposed", decode.string)
      decode.success(#(base, proposed))
    }),
  )
  decode.success(Question(id, text, options, answer, settings))
}

fn run_decoder() -> decode.Decoder(Run) {
  use id <- decode.field("id", decode.string)
  use kind <- decode.field("kind", decode.string)
  use state <- decode.field("state", decode.string)
  use model <- decode.field("model", decode.string)
  use effort <- decode.optional_field(
    "effort",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use log <- decode.field("log", decode.string)
  use outcome <- decode.optional_field("outcome", "", decode.string)
  use started <- decode.optional_field("started", 0, decode.int)
  use ended <- decode.optional_field("ended", 0, decode.int)
  decode.success(Run(
    id,
    kind,
    state,
    model,
    effort,
    log,
    outcome,
    started,
    ended,
  ))
}

fn link_decoder() -> decode.Decoder(Link) {
  use name <- decode.field("name", decode.string)
  use url <- decode.field("url", decode.string)
  decode.success(Link(name, url))
}

pub fn agent_decoder() -> decode.Decoder(Agent) {
  use id <- decode.field("id", decode.string)
  use label <- decode.field("label", decode.string)
  use available <- decode.optional_field("available", True, decode.bool)
  use reason <- decode.optional_field(
    "reason",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use effort <- decode.optional_field(
    "effort",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use efforts <- decode.optional_field(
    "efforts",
    [],
    decode.list(decode.string),
  )
  decode.success(Agent(id, label, available, reason, effort, efforts))
}

pub fn workflow_decoder() -> decode.Decoder(Workflow) {
  use columns <- decode.field("columns", decode.list(column_decoder()))
  decode.success(Workflow(columns))
}

fn column_decoder() -> decode.Decoder(Column) {
  use id <- decode.field("id", decode.string)
  use label <- decode.optional_field("label", id, decode.string)
  use actions <- decode.optional_field(
    "actions",
    [],
    decode.list(decode.string),
  )
  use final <- decode.optional_field("final", False, decode.bool)
  decode.success(Column(id, label, actions, final))
}

/// The projects in `GET /projects`, in the order the server lists them.
fn projects_decoder() -> decode.Decoder(List(Project)) {
  let project = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use base <- decode.field("base", decode.string)
    decode.success(Project(dir, label, base))
  }
  decode.at(["projects"], decode.list(project))
}

pub fn actions(workflow: Workflow, task: Task) -> List(String) {
  // Discuss starts between runs; stop ends any run, and a discussion can be ended.
  let built_in = case list.find(task.runs, fn(r) { r.state != "ended" }) {
    Error(_) -> ["discuss"]
    Ok(r) if r.kind == "discuss" -> ["end_discussion", "stop"]
    Ok(_) -> ["stop"]
  }
  case list.find(workflow.columns, fn(c) { c.id == task.column }) {
    Ok(c) if c.final -> []
    Ok(c) ->
      list.flatten([
        c.actions,
        case task.paused {
          "" -> ["pause"]
          _ -> ["resume"]
        },
        built_in,
      ])
    Error(_) -> []
  }
}

/// What an action in flight is waiting for: with a work run going, a code
/// change takes the project's lock, which the agent's command holds.
pub fn acting_text(t: Task, name: String) -> String {
  case list.any(t.runs, fn(r) { r.kind == "work" && r.state != "ended" }) {
    True ->
      "Waiting for the agent's current command to finish before " <> name <> "…"
    False -> "Sending " <> name <> "…"
  }
}

/// Lines only in the old settings, then lines only in the proposed ones.
/// ponytail: a line-set diff, not an LCS; fine for a short solace.toml.
pub fn settings_diff(base: String, proposed: String) -> String {
  let old = string.split(base, "\n")
  let new = string.split(proposed, "\n")
  list.append(
    list.filter(old, fn(l) { !list.contains(new, l) })
      |> list.map(fn(l) { "- " <> l }),
    list.filter(new, fn(l) { !list.contains(old, l) })
      |> list.map(fn(l) { "+ " <> l }),
  )
  |> string.join("\n")
}

/// An agent note names its run, as the run list does.
pub fn author(t: Task, n: Note) -> String {
  case n.author, list.find(t.runs, fn(r) { r.id == n.run }) {
    "agent", Ok(r) -> "agent · " <> run_label(r)
    who, _ -> who
  }
}

fn run_label(r: Run) -> String {
  r.kind
  <> " · "
  <> r.model
  <> case r.effort {
    "" -> ""
    effort -> " · " <> effort
  }
}

/// What the task's agent is doing right now, or what it finished.
pub fn status(t: Task) -> String {
  case t.paused, list.find(t.runs, fn(r) { r.state != "ended" }) {
    _, Error(_) -> outcome(t)
    "", Ok(r) if r.state == "waiting" && r.kind == "discuss" ->
      "Waiting for your reply"
    "", Ok(r) if r.state == "waiting" -> "Waiting for your answer"
    "", Ok(r) if r.kind == "discuss" -> "Agent discussing"
    "", Ok(r) if r.kind == "review" -> "Agent reviewing"
    "", Ok(r) if r.kind == "investigate" -> "Agent investigating"
    "", Ok(_) -> "Agent working"
    _, Ok(_) -> "Agent paused"
  }
}

/// Changes as an outline: source modules before test modules, each module's
/// symbols alphabetical. Headings name the project too when `multi_project`.
pub fn change_groups(
  changes: List(Change),
  multi_project: Bool,
) -> List(#(String, List(String))) {
  // Source, then tests, then files, then anything else.
  let is_test = fn(c: Change) {
    case c.module {
      m if m == files_heading -> 2
      m if m == other_heading -> 3
      m ->
        case string.ends_with(m, "_test") {
          True -> 1
          False -> 0
        }
    }
  }
  let sorted =
    list.sort(changes, fn(a, b) {
      string.compare(a.project, b.project)
      |> order.break_tie(int.compare(is_test(a), is_test(b)))
      |> order.break_tie(string.compare(a.module, b.module))
      |> order.break_tie(string.compare(a.name, b.name))
    })
  let heading = fn(c: Change) {
    case multi_project, c.project {
      True, p if p != "" -> p <> " · " <> c.module
      _, _ -> c.module
    }
  }
  list.fold(sorted, [], fn(groups, c) {
    let h = heading(c)
    case groups {
      [#(last, names), ..rest] if last == h -> [#(h, [c.name, ..names]), ..rest]
      _ -> [#(h, [c.name]), ..groups]
    }
  })
  |> list.map(fn(g) { #(g.0, list.reverse(g.1)) })
  |> list.reverse
}

pub fn view(model: Model) -> Element(Msg) {
  h.div([], [
    h.style([], task_styles),
    h.p([a.role("alert")], [h.text(model.error)]),
    case model.task {
      None -> h.text("")
      Some(t) -> detail(model, t)
    },
  ])
}

fn detail(model: Model, t: Task) -> Element(Msg) {
  let open_questions = list.any(t.questions, fn(q) { q.answer == "" })
  let tabs = [
    #("details", "Details"),
    #("notes", "Notes (" <> int.to_string(list.length(t.notes)) <> ")"),
    #(
      "questions",
      "Questions ("
        <> int.to_string(list.length(t.questions))
        <> ")"
        <> case open_questions {
        True -> " •"
        False -> ""
      },
    ),
    #("runs", "Runs (" <> int.to_string(list.length(t.runs)) <> ")"),
    #("changes", "Changes (" <> int.to_string(list.length(t.changes)) <> ")"),
  ]
  let tabs = case model.workspace {
    True -> list.append(tabs, [#("scope", "Scope")])
    False -> tabs
  }
  h.section([a.class("detail")], [
    h.div([a.class("heading")], [
      h.h2([], [h.text(t.title)]),
      h.button(
        [
          a.class("copy-id"),
          a.title(
            "Copies "
            <> reference(t.id)
            <> ": pasted into any text, it links to this task",
          ),
          e.on_click(CopyId),
        ],
        [
          h.text(case model.copy.1 {
            "" -> "Copy ID"
            said -> said
          }),
        ],
      ),
    ]),
    h.p([a.class("meta")], [
      h.text(t.column),
      status_badge_in(model.workflow, t),
    ]),
    case t.attention {
      "" -> h.text("")
      reason ->
        h.div([a.class("attention"), a.role("status")], [
          h.strong([], [h.text("Needs attention")]),
          md(model.base, reason),
        ])
    },
    agent_choice(model),
    composer(model),
    case model.acting {
      "" -> h.text("")
      name ->
        h.p([a.class("muted"), a.role("status")], [h.text(acting_text(t, name))])
    },
    h.div(
      [a.class("actions")],
      list.append(
        list.map(actions(model.workflow, t), fn(name) {
          action_button(name, model.acting != "")
        }),
        [button("Add note", AddNote)],
      ),
    ),
    h.div(
      [a.class("tabs"), a.role("tablist")],
      list.map(tabs, fn(tab) {
        h.button(
          [
            a.role("tab"),
            a.aria_selected(model.tab == tab.0),
            e.on_click(Tab(tab.0)),
          ],
          [h.text(tab.1)],
        )
      }),
    ),
    h.div([a.class("panel"), a.role("tabpanel")], case model.tab {
      "notes" -> notes_view(model.base, t)
      "questions" -> questions_view(model.base, t)
      "runs" -> list.map(list.reverse(t.runs), fn(r) { run_view(model, r) })
      "changes" -> [
        case t.changes {
          [] -> h.text("")
          _ ->
            h.p([], [
              h.text(model.review_summary <> " "),
              h.a([a.href(model.base <> "/contracts?task=" <> t.id)], [
                h.text("Contract view"),
              ]),
            ])
        },
        h.div(
          [a.class("changes")],
          list.map(change_groups(t.changes, list.length(t.scope) > 1), fn(g) {
            h.section([], [
              h.h4([], [h.text(g.0)]),
              h.ul([], list.map(g.1, fn(n) { h.li([], [h.text(n)]) })),
            ])
          }),
        ),
      ]
      "scope" -> [scope_view(model, t)]
      _ -> details_view(model, t)
    }),
  ])
}

fn details_view(model: Model, t: Task) -> List(Element(Msg)) {
  case model.editing {
    True -> [
      title_input(model.title, Title),
      task_editor(model, "edit-body", "Description"),
      h.label([], [
        h.text("Rank (lower comes first)"),
        h.input([a.type_("number"), a.value(model.rank), e.on_input(Rank)]),
      ]),
      button("Save", Save),
      button("Cancel", Edit(False)),
    ]
    False -> [
      md(model.base, t.body),
      h.div(
        [],
        list.map(t.links, fn(link) {
          h.a([a.href(link.url)], [h.text(link.name)])
        }),
      ),
      button("Edit", Edit(True)),
    ]
  }
}

fn agent_choice(model: Model) -> Element(Msg) {
  let chosen = list.find(model.models, fn(m) { m.id == model.model })
  h.div([a.class("agent")], [
    h.label([], [
      h.text("Agent model"),
      h.select(
        [a.value(model.model), e.on_input(AgentSelected)],
        list.map(model.models, fn(m) {
          case m.available {
            True ->
              h.option(
                [a.value(m.id), a.selected(m.id == model.model)],
                m.label,
              )
            False ->
              h.option(
                [
                  a.value(m.id),
                  a.disabled(True),
                  a.selected(m.id == model.model),
                ],
                m.label <> " (unavailable: " <> m.reason <> ")",
              )
          }
        }),
      ),
    ]),
    case chosen {
      Ok(m) if m.efforts != [] ->
        h.label([], [
          h.text("Effort"),
          h.select([a.value(model.effort), e.on_input(EffortSelected)], [
            h.option(
              [a.value(""), a.selected(model.effort == "")],
              case m.effort {
                "" -> "Default"
                effort -> "Default (" <> effort <> ")"
              },
            ),
            ..list.map(m.efforts, fn(effort) {
              h.option(
                [a.value(effort), a.selected(effort == model.effort)],
                effort,
              )
            })
          ]),
        ])
      _ -> h.text("")
    },
  ])
}

pub fn title_input(value: String, msg: fn(String) -> msg) -> Element(msg) {
  h.input([
    a.class("title"),
    a.placeholder("Title"),
    a.attribute("aria-label", "Title"),
    a.value(value),
    e.on_input(msg),
  ])
}

fn composer(model: Model) -> Element(Msg) {
  task_editor(model, "note-text", "Note, answer or instructions")
}

/// One of the task's editors: `id` names what it edits (see `field`).
fn task_editor(model: Model, id: String, label: String) -> Element(Msg) {
  let #(text, msg) = field(model, id)
  editor(
    id,
    model.id,
    model.base,
    label,
    text,
    list.contains(model.previews, id),
    msg,
    Preview(id, _),
    fn(before, after) { Format(id, before, after) },
    Block(id, _),
  )
}

/// A markdown editor with Write/Preview and formatting tools; `id` names the
/// textarea. Images pasted, dropped or picked here are uploaded by
/// `task_api.mjs` to `task` in the project at `base` and inserted as
/// markdown. A task not yet created has no image tool.
pub fn editor(
  id: String,
  task: String,
  base: String,
  label: String,
  text: String,
  preview: Bool,
  on_input: fn(String) -> msg,
  on_preview: fn(Bool) -> msg,
  on_format: fn(String, String) -> msg,
  on_block: fn(String) -> msg,
) -> Element(msg) {
  let tool = fn(label, title, before, after) {
    h.button(
      [a.type_("button"), a.title(title), e.on_click(on_format(before, after))],
      [h.text(label)],
    )
  }
  let block_tool = fn(label, title, kind) {
    h.button([a.type_("button"), a.title(title), e.on_click(on_block(kind))], [
      h.text(label),
    ])
  }
  h.div([a.class("composer")], [
    h.div([a.class("toolbar")], [
      h.button(
        [
          a.type_("button"),
          a.aria_pressed(bool_text(!preview)),
          e.on_click(on_preview(False)),
        ],
        [h.text("Write")],
      ),
      h.button(
        [
          a.type_("button"),
          a.aria_pressed(bool_text(preview)),
          e.on_click(on_preview(True)),
        ],
        [h.text("Preview")],
      ),
      h.span([a.class("separator")], []),
      block_tool("H", "Heading", "heading"),
      tool("B", "Bold", "**", "**"),
      tool("`code`", "Code", "`", "`"),
      block_tool("Code block", "Code block", "code"),
      block_tool("• List", "List", "list"),
      block_tool("1. List", "Numbered list", "numbered"),
      block_tool("Quote", "Quote", "quote"),
      tool("Link", "Link", "[", "](https://)"),
      case task {
        "" -> h.text("")
        _ ->
          h.label([a.class("button"), a.title("Add an image")], [
            h.text("Image"),
            h.input([
              a.type_("file"),
              a.class("md-image sr-only"),
              a.attribute("data-task", task),
              a.attribute("data-base", base),
              a.attribute("data-for", id),
              a.accept(["image/png", "image/jpeg", "image/gif", "image/webp"]),
            ]),
          ])
      },
    ]),
    case preview {
      True ->
        h.div([a.class("preview")], [
          case text {
            "" -> h.p([a.class("muted")], [h.text("Nothing to preview.")])
            text -> md(base, text)
          },
        ])
      False ->
        h.textarea(
          [
            a.id(id),
            a.class("md-editor"),
            a.attribute("data-task", task),
            a.attribute("data-base", base),
            a.attribute("aria-label", label),
            a.placeholder(
              label
              <> " (markdown)."
              <> case task {
                "" -> ""
                _ -> " Paste or drop images here."
              },
            ),
            a.value(text),
            e.on_input(on_input),
          ],
          text,
        )
    },
  ])
}

fn bool_text(value: Bool) -> String {
  case value {
    True -> "true"
    False -> "false"
  }
}

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

fn action_button(name: String, busy: Bool) -> Element(Msg) {
  h.button([e.on_click(Action(name)), a.disabled(busy)], [h.text(name)])
}

fn questions_view(base: String, t: Task) -> List(Element(Msg)) {
  list.map(list.reverse(t.questions), fn(q) {
    let #(diff, other) = case q.settings {
      Some(#(current, proposed)) -> #(
        [h.pre([a.class("diff")], [h.text(settings_diff(current, proposed))])],
        "Reject with the editor's text",
      )
      None -> #([], "Answer with the editor's text")
    }
    h.article([], [
      md(base, q.text),
      ..list.append(diff, [
        case q.answer {
          "" ->
            h.div(
              [],
              list.append(
                list.map(q.options, fn(o) { button(o, Answer(q.id, o)) }),
                [button(other, Answer(q.id, ""))],
              ),
            )
          answer -> h.p([], [h.text("Answer: " <> answer)])
        },
      ])
    ])
  })
}

/// Newest first. Each keeps its position, which names it to the API.
fn notes_view(base: String, t: Task) -> List(Element(Msg)) {
  t.notes
  |> list.index_map(fn(n, i) { #(n, i) })
  |> list.reverse
  |> list.map(fn(pair) {
    let #(n, i) = pair
    h.details([a.class("note"), a.open(True)], [
      h.summary([], [
        h.strong([], [h.text(author(t, n))]),
        h.small([], [h.text(clock(n.time))]),
        h.button(
          [
            a.class("delete"),
            a.attribute("aria-label", "Delete note"),
            e.on_click(DeleteNote(i)) |> e.prevent_default,
          ],
          [h.text("Delete")],
        ),
      ]),
      md(base, n.text),
    ])
  })
}

pub fn status_badge(t: Task) -> Element(msg) {
  badge(status(t))
}

/// Markdown; images uploaded to the board load from the project at `base`.
pub fn md(base: String, text: String) -> Element(msg) {
  element.unsafe_raw_html("", "div", [a.class("md")], markdown(text, base))
}

fn run_view(model: Model, r: Run) -> Element(Msg) {
  let watching = model.watching == r.id
  let when = case r.ended {
    0 -> "started " <> clock(r.started)
    ended -> clock(r.started) <> " – " <> clock(ended)
  }
  h.article([a.class("run")], [
    h.header([], [
      h.strong([], [h.text(run_label(r))]),
      h.span([a.class("state " <> r.state)], [
        h.text(case r.state, r.outcome {
          "ended", "" -> "ended"
          "ended", outcome -> "ended: " <> outcome
          state, _ -> state
        }),
      ]),
      case watching {
        True -> button("Hide activity", Unwatch)
        False ->
          button(
            case r.state {
              "ended" -> "Show activity"
              _ -> "Follow live"
            },
            Watch(r.id),
          )
      },
    ]),
    h.small([], [h.text(when <> " · log: " <> r.log)]),
    case watching {
      False -> h.text("")
      True ->
        case model.entries {
          [] -> h.p([a.class("muted")], [h.text("No activity yet.")])
          entries ->
            h.div(
              [a.class("transcript")],
              list.map(entries, entry_view(model.base, _)),
            )
        }
    },
  ])
}

/// One transcript step, shared with the Strategy page's planning panel.
pub fn entry_view(base: String, e: Entry) -> Element(msg) {
  case e.kind {
    "message" -> h.div([a.class("entry message")], [md(base, e.text)])
    "thinking" ->
      h.details([a.class("entry thinking")], [
        h.summary([], [h.text("Thinking")]),
        md(base, e.text),
      ])
    "tool" ->
      h.details([a.class("entry tool " <> e.status)], [
        h.summary([], [
          h.span([a.class("icon")], [
            h.text(case e.status {
              "ok" -> "✓"
              "failed" -> "✗"
              _ -> "…"
            }),
          ]),
          h.code([], [h.text(e.title)]),
        ]),
        case e.input {
          "" -> h.text("")
          input -> h.pre([a.class("input")], [h.text(input)])
        },
        h.pre([], [
          h.text(case e.status, e.output {
            "running", _ -> "Running…"
            _, "" -> "(no output)"
            _, output -> output
          }),
        ]),
      ])
    "usage" -> h.small([a.class("entry")], [h.text(e.text)])
    "error" -> h.p([a.class("entry"), a.role("alert")], [h.text(e.text)])
    _ ->
      h.details([a.class("entry")], [
        h.summary([], [h.text("Output")]),
        h.pre([], [h.text(e.text)]),
      ])
  }
}

/// The Scope tab: a switch per project the task can change, locked while the
/// agent works.
fn scope_view(model: Model, t: Task) -> Element(Msg) {
  let working =
    t.paused == "" && list.any(t.runs, fn(r) { r.state == "running" })
  h.div([], [
    projects_pick(
      list.map(model.projects, fn(p) { #(p.dir, p.label) }),
      scope_of(model.dir, t),
      working,
      SetScope,
    ),
    case working {
      True ->
        h.small([], [
          h.text(
            "You can change the projects once the agent stops working, is paused, or asks a question.",
          ),
        ])
      False -> h.text("")
    },
  ])
}

/// A switch per project, each a folder and its name on screen, for the
/// projects a task can change.
pub fn projects_pick(
  projects: List(#(String, String)),
  chosen: List(String),
  disabled: Bool,
  msg: fn(String, Bool) -> msg,
) -> Element(msg) {
  h.fieldset([a.class("projects-pick"), a.disabled(disabled)], [
    h.legend([], [h.text("Projects this task can change")]),
    ..list.map(projects, fn(p) {
      let #(dir, label) = p
      h.label([], [
        h.input([
          a.type_("checkbox"),
          a.role("switch"),
          a.checked(list.contains(chosen, dir)),
          e.on_check(msg(dir, _)),
        ]),
        h.text(label),
      ])
    })
  ])
}

/// The project switches' look, shared with the board's new-task form.
pub const projects_pick_styles =
  ".projects-pick{margin:8px 0}.projects-pick label{display:flex;align-items:center;gap:10px;margin:6px 0;cursor:pointer}.projects-pick input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}.projects-pick input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}.projects-pick input:checked{background:var(--focus)}.projects-pick input:checked::before{transform:translateX(14px)}.projects-pick input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}.projects-pick[disabled] label,.projects-pick[disabled] input{opacity:.5;cursor:default}@media (prefers-reduced-motion:reduce){.projects-pick input,.projects-pick input::before{transition:none}}\n"

/// The detail's look. Colours come from the page's custom properties, which
/// a nested shadow root inherits from its host: none are set here, so the
/// board's dark theme reaches the detail.
const task_styles =
  projects_pick_styles
  <> editor_styles
  <> ":host{display:block}input,textarea,pre,code,.md,.detail > .heading > h2,.detail > .panel > article > p,.detail > .panel > p,.detail > .panel > ul > li,.changes h4,.changes li,.run > header > strong,.run > header > .state,.run > small{user-select:text;-webkit-user-select:text}
button,input,textarea,select{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:0}button{cursor:pointer;background:var(--paper)}button:hover{background:var(--hover)}button:focus-visible{outline:3px solid var(--focus)}label{display:block}textarea{display:block;width:min(95%,700px);min-height:70px}input{width:min(80%,600px)}.detail{border:1px solid var(--line);padding:16px;border-radius:0}small{display:block;color:var(--soft);overflow-wrap:anywhere}pre{white-space:pre-wrap;font:inherit}.actions{display:flex;flex-wrap:wrap;gap:6px}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}.note,.run{border:1px solid var(--line);border-radius:0;padding:8px 12px;margin:8px 0;background:var(--paper)}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.md p,.md ul,.md ol,.md blockquote{margin:6px 0}.md blockquote{padding-left:10px;border-left:3px solid var(--line);color:var(--soft)}.md pre,.transcript pre{background:var(--code);padding:8px;border-radius:0;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace;max-height:320px}code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:0}.transcript{border-left:3px solid var(--line);padding-left:10px;margin-top:8px}.entry{margin:6px 0}details.entry>summary{cursor:pointer;color:var(--soft)}.tool .icon{display:inline-block;width:1.2em}.tool.ok .icon{color:var(--ok-text)}.tool.failed .icon{color:var(--revision)}.muted{color:var(--soft)}.note .delete{margin-left:auto;font-size:13px;padding:2px 8px}
.meta{display:flex;gap:10px;align-items:center;color:var(--soft);margin:4px 0 12px}.agent{display:flex;flex-wrap:wrap;gap:4px 16px}.agent label{display:flex;flex-direction:column}
.tabs{display:flex;flex-wrap:wrap;gap:0;border-bottom:1px solid var(--line);margin-top:16px}.tabs button{margin:0 0 -1px;border-radius:0 0 0;border-color:transparent;background:transparent}.tabs [aria-selected=true]{border-color:var(--line) var(--line) var(--sheet);background:var(--sheet);font-weight:600}.panel{padding-top:8px}.changes h4{font-size:15px;margin:12px 0 4px}.changes ul{margin:0;padding-left:20px}.changes li{white-space:normal;margin:2px 0;font:13px 'IBM Plex Mono',ui-monospace,monospace}
details.note>summary{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap;cursor:pointer;list-style:none}details.note>summary::-webkit-details-marker{display:none}details.note>summary::before{content:\"▸\";color:var(--soft);width:1em}details.note[open]>summary::before{content:\"▾\"}details.note[open]>summary{margin-bottom:4px}.md img{max-width:100%;height:auto;border:1px solid var(--line);border-radius:0}.status.finished{background:var(--add);color:var(--ok-text)}.diff{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:8px;border-radius:0;max-width:760px}
.title{display:block;box-sizing:border-box;width:calc(100% - 8px);max-width:762px;margin:8px 4px}
.heading{display:flex;align-items:center;flex-wrap:wrap;gap:4px 12px}.heading h2{margin:12px 0}.copy-id{flex:none;margin:0;padding:2px 10px;font-size:13px}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button,input,textarea,select{border:1px solid var(--edge);background:var(--paper);color:var(--ink);caret-color:var(--focus)}input::placeholder,textarea::placeholder{color:var(--soft);opacity:1}
button:hover{background:var(--hover)}button:disabled{color:var(--soft)}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
textarea{font-family:inherit}
.attention{font-weight:700}
.status{border:1px solid var(--line);background:var(--paper);font-size:11px;text-transform:uppercase;letter-spacing:.08em;padding:1px 8px}.status.finished{background:var(--add);color:var(--ok-text);border-color:var(--ok-text)}.working::before{background:var(--entry)}
.detail,.composer,.note,.run{background:var(--paper);border-color:var(--edge)}.detail{padding:20px 24px}
.tabs{border-bottom:2px solid var(--edge)}.tabs button{border:0;border-right:1px solid var(--line);background:transparent;margin:0;padding:10px 16px}.tabs [aria-selected=true]{background:var(--paper);box-shadow:inset 0 -3px var(--ink);border-color:var(--line)}
article{border-bottom-color:var(--line)}.transcript{border-left:1px solid var(--edge)}
.md pre,.transcript pre,.diff,code{background:var(--code);color:var(--ink)}
@media(prefers-reduced-motion:no-preference){button{transition:background-color .12s ease-out}}"

/// The markdown editor's look (`editor`), shared with the pages that use it.
/// Colours come from the page's custom properties.
pub const editor_styles =
  ".composer{border:1px solid var(--edge);border-radius:0;margin:8px 4px;max-width:760px;background:var(--paper)}.composer textarea{width:100%;max-width:none;box-sizing:border-box;border:0;border-top:1px solid var(--line);border-radius:0 0 6px 6px;margin:0;min-height:110px}.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:2px;padding:4px}.toolbar button,.toolbar .button{margin:0;padding:4px 10px;font-size:14px}.sr-only{position:absolute;width:1px;height:1px;opacity:0;overflow:hidden}.toolbar .button{position:relative;display:inline-block;border:1px solid var(--edge);border-radius:0;background:var(--paper);cursor:pointer}.toolbar .button:hover{background:var(--hover)}.toolbar .button:focus-within{outline:3px solid var(--focus)}.toolbar [aria-pressed=true]{background:var(--well);font-weight:600}.separator{width:1px;align-self:stretch;background:var(--line);margin:0 6px}.preview{border-top:1px solid var(--line);padding:8px 12px;min-height:94px}
.composer textarea{border:0;border-top:1px solid var(--edge)}.toolbar .button{border-color:var(--edge);background:var(--paper)}.toolbar .button:hover{background:var(--hover)}.toolbar [aria-pressed=true]{background:var(--ink);color:var(--paper)}
"

/// A strategy run's proposal (D-93, Spec 15 §6): a record to create (`id`
/// "") or change, or the vision's new text (`fields` holds `text`). `fields`
/// are the request's, each value as text, the title first; `base` is the
/// vision's revision when proposed; `reopens` are the `complete` or
/// `canceled` records, as (kind, id), that Approve also sets `planned`.
pub type Proposal {
  Proposal(
    kind: String,
    id: String,
    fields: List(#(String, String)),
    base: String,
    reopens: List(#(String, String)),
  )
}

pub fn proposal_decoder() -> decode.Decoder(Proposal) {
  let text = fn(name, next) {
    decode.optional_field(
      name,
      "",
      decode.one_of(decode.string, [decode.success("")]),
      next,
    )
  }
  use kind <- decode.field("kind", decode.string)
  use id <- text("id")
  use fields <- decode.optional_field(
    "fields",
    [],
    decode.dict(decode.string, field_text())
      |> decode.map(fn(d) { list.sort(dict.to_list(d), field_order) }),
  )
  use base <- text("base")
  use reopens <- decode.optional_field(
    "reopens",
    [],
    decode.list({
      use kind <- decode.field("kind", decode.string)
      use id <- decode.field("id", decode.string)
      decode.success(#(kind, id))
    }),
  )
  decode.success(Proposal(kind, id, fields, base, reopens))
}

/// A proposed field's value as text: a list joined, null as "(none)".
fn field_text() -> decode.Decoder(String) {
  decode.one_of(decode.string, [
    decode.list(decode.string)
      |> decode.map(fn(values) {
        case values {
          [] -> "(none)"
          _ -> string.join(values, ", ")
        }
      }),
    decode.int |> decode.map(int.to_string),
    decode.float |> decode.map(float.to_string),
    decode.bool
      |> decode.map(fn(on) {
        case on {
          True -> "yes"
          False -> "no"
        }
      }),
    decode.success("(none)"),
  ])
}

/// The title first, then the other fields by name.
fn field_order(a: #(String, String), b: #(String, String)) -> order.Order {
  case a.0, b.0 {
    "title", _ -> order.Lt
    _, "title" -> order.Gt
    x, y -> string.compare(x, y)
  }
}

/// A strategy run's proposal in human terms: what it creates or changes, its
/// fields (the vision's text as Markdown), the records Approve also sets
/// `planned`, then its options until it is answered, and the answer after.
/// `name(kind, id)` is a record's title, "" when it isn't loaded.
pub fn proposal_view(
  base: String,
  p: Proposal,
  options: List(String),
  answer: String,
  name: fn(String, String) -> String,
  busy: Bool,
  on_answer: fn(String) -> msg,
) -> Element(msg) {
  let title = case list.key_find(p.fields, "title") {
    Ok(title) -> title
    Error(_) -> name(p.kind, p.id)
  }
  let heading = case p.kind, p.id {
    "vision", _ -> [h.text("Replace the vision")]
    kind, id -> [
      h.text(case id {
        "" -> "Create " <> kind
        _ -> "Change " <> kind
      }),
      case title, id {
        "", "" -> h.text("")
        "", id -> h.text(" " <> id)
        title, _ -> h.em([], [h.text(" " <> title)])
      },
    ]
  }
  h.article([a.class("proposal")], [
    h.h4([], heading),
    case p.kind {
      "vision" -> md(base, list.key_find(p.fields, "text") |> result.unwrap(""))
      _ ->
        h.dl(
          [],
          list.flat_map(p.fields, fn(f) {
            [
              h.dt([], [h.text(f.0)]),
              h.dd([], [
                case f.0 {
                  "body" -> md(base, f.1)
                  _ -> h.text(f.1)
                },
              ]),
            ]
          }),
        )
    },
    ..list.append(
      list.map(p.reopens, fn(r) {
        let shown = case name(r.0, r.1) {
          "" -> r.1
          title -> title
        }
        h.p([a.class("reopens")], [
          h.text("Approving also sets the " <> r.0 <> " "),
          h.em([], [h.text(shown)]),
          h.text(" back to planned."),
        ])
      }),
      [
        case answer {
          "" ->
            h.div(
              [a.class("actions")],
              list.map(options, fn(o) {
                h.button([a.disabled(busy), e.on_click(on_answer(o))], [
                  h.text(o),
                ])
              }),
            )
          answer -> h.p([a.class("answer")], [h.text("Answer: " <> answer)])
        },
      ],
    )
  ])
}

/// The proposal display's look (`proposal_view`), for the pages that show
/// it. Colours come from the page's custom properties.
pub const proposal_styles =
  ".proposal{border:1px solid var(--edge);background:var(--paper);padding:8px 12px;margin:8px 0}.proposal h4{margin:0 0 6px;font-size:15px}.proposal dl{display:grid;grid-template-columns:max-content minmax(0,1fr);gap:2px 12px;margin:6px 0}.proposal dt{color:var(--soft);font-size:13px}.proposal dd{margin:0;overflow-wrap:anywhere}.proposal .reopens{color:var(--warn-text);font-weight:600}.proposal .actions{display:flex;gap:6px}.proposal .answer{font-weight:600}\n"

/// What the task's last run finished with, "" when nothing is to be looked at.
fn outcome(t: Task) -> String {
  case t.finished {
    "finished" -> "Agent finished"
    "reported" -> "Report ready"
    "reviewed" -> "Review ready"
    _ -> ""
  }
}

/// `status` for a task whose project's workflow is known: a task in a final
/// column has no agent at work, whatever runs its copy still shows going.
pub fn status_in(workflow: Workflow, t: Task) -> String {
  case list.find(workflow.columns, fn(c) { c.id == t.column }) {
    Ok(c) if c.final -> outcome(t)
    _ -> status(t)
  }
}

/// `status_badge` by `status_in`: no live agent in a final column.
pub fn status_badge_in(workflow: Workflow, t: Task) -> Element(msg) {
  badge(status_in(workflow, t))
}

fn badge(status: String) -> Element(msg) {
  case status {
    "" -> h.text("")
    s -> {
      let class = case s {
        "Agent working" | "Agent reviewing" | "Agent investigating" ->
          "status working"
        "Agent finished" | "Report ready" | "Review ready" -> "status finished"
        _ -> "status"
      }
      h.span([a.class(class), a.role("status")], [h.text(s)])
    }
  }
}

/// The message a host hands down once its stream has (re)connected and caught
/// up: an event may have been lost meanwhile, so the task is fetched again.
pub const resync = "{\"resync\":true}"

/// Whether a live message puts this element's copy of the task in doubt, so
/// it is fetched again rather than trusted: the host's `resync`, or an update
/// of this task named under another project than the one it is opened from.
pub fn stale(model: Model, text: String) -> Bool {
  case json.parse(text, decode.at(["resync"], decode.bool)) {
    Ok(_) -> True
    Error(_) ->
      case
        json.parse(text, decode.at(["project"], decode.string)),
        json.parse(text, decode.at(["task", "id"], decode.string))
      {
        Ok(dir), Ok(id) -> dir != model.dir && id == model.id
        _, _ -> False
      }
  }
}

/// Fetches the task again: its GET is the whole task as the server has it
/// now, the only one that carries its changes (D-77). Each event and reply
/// ends with one, so a lost or late message can't leave the copy stale.
fn refresh(model: Model) -> Effect(Msg) {
  case model.task {
    Some(t) ->
      get(model.base, "/tasks/" <> t.id, Refetched(model.generation, _))
    None -> effect.none()
  }
}

/// A task's reference in text, `task:<id>`: markdown draws it as a link to
/// the task (D-98).
pub fn reference(id: String) -> String {
  "task:" <> id
}

@external(javascript, "./task_api.mjs", "copy_text")
fn copy_text(text: String, done: fn(Bool) -> Nil) -> Nil

/// How long Copy ID says Copied, or Copy failed.
const copied_ms = 2000

/// Whether a live message says the workspace's list of projects changed.
pub fn projects_changed(text: String) -> Bool {
  json.parse(text, decode.at(["projects"], decode.list(decode.string)))
  |> result.is_ok
}
