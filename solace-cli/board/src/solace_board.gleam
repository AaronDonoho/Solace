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
import gleam/uri
import lustre
import lustre/attribute as a
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/element/svg
import lustre/event as e

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(
  base: String,
  receive: fn(String) -> Nil,
  ready: fn() -> Nil,
) -> Source

@external(javascript, "./api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "clock")
fn clock(seconds: Int) -> String

@external(javascript, "./api.mjs", "reload")
fn reload() -> Nil

@external(javascript, "./api.mjs", "confirm")
fn confirm(question: String) -> Bool

@external(javascript, "./api.mjs", "markdown")
fn markdown(text: String, base: String) -> String

@external(javascript, "./api.mjs", "load")
fn load(key: String) -> String

@external(javascript, "./api.mjs", "store")
fn store(key: String, value: String) -> Nil

@external(javascript, "./api.mjs", "wrap")
fn wrap(
  root: dynamic.Dynamic,
  id: String,
  text: String,
  before: String,
  after: String,
) -> String

@external(javascript, "./api.mjs", "images")
fn images(
  root: dynamic.Dynamic,
  insert: fn(String, String) -> Nil,
  fail: fn(String) -> Nil,
) -> Nil

pub type Column {
  Column(id: String, label: String, actions: List(String), final: Bool)
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

/// A bold heading, such as "Reviewed Fix checkout", and what happened.
pub type Notice {
  Notice(heading: String, text: String)
}

pub type Workflow {
  Workflow(columns: List(Column))
}

/// One project's tasks on the board (D-57). The root's own project has dir
/// "." and base ""; the others are served under `base`, `/p/<dir>`.
pub type Lane {
  Lane(
    dir: String,
    label: String,
    name: String,
    base: String,
    /// The project's compilation target, "" when the server didn't say.
    target: String,
    workflow: Workflow,
    tasks: List(Task),
  )
}

/// A Gleam project found in the workspace folder.
pub type Found {
  Found(dir: String, name: String, imported: Bool, member: Bool)
}

pub type Model {
  Model(
    lanes: List(Lane),
    /// Whether the root has several projects (or is a folder of them), so
    /// lanes show; a project on its own looks as it always has.
    workspace: Bool,
    /// Whether the project list has loaded once.
    listed: Bool,
    found: List(Found),
    /// The dir of the selected task's lane.
    lane: String,
    selected: Option(Task),
    /// The task chosen on the board, including while its details load.
    selected_card: Option(#(String, String)),
    selection_generation: Int,
    models: List(Agent),
    model: String,
    effort: String,
    text: String,
    title: String,
    body: String,
    error: String,
    rank: String,
    watching: String,
    entries: List(Entry),
    review_summary: String,
    creating: Bool,
    draft_title: String,
    draft_body: String,
    editing: Bool,
    tab: String,
    previews: List(String),
    notices: List(Notice),
    caught_up: Bool,
    replay: List(String),
    collapsed: List(String),
    draft_lane: String,
    /// The projects a new task covers; empty means the draft lane alone.
    draft_scope: List(String),
    /// Whether a create is waiting for the server; Create task is disabled
    /// meanwhile, so one submit makes one task (D-77).
    submitting: Bool,
    /// The action sent and not yet answered: its buttons are disabled
    /// meanwhile, so one click makes one request. "" when none.
    acting: String,
    projects_open: Bool,
    project_name: String,
    project_dir: String,
    project_target: String,
    /// The project being removed: its dir, whether the second step (deleting
    /// its files) shows, and the name typed to confirm it.
    removing: Option(#(String, Bool, String)),
    source: Option(Source),
    /// The `base` attribute: this instance's API prefix, "" for the page's.
    prefix: String,
    /// The component's shadow root, once it has rendered.
    root: dynamic.Dynamic,
    connected: Bool,
    /// The live transcript polling chain; an ended chain schedules nothing.
    poll: Int,
  )
}

pub type Msg {
  ProjectsLoaded(String)
  WorkflowLoaded(String, String)
  ModelsLoaded(String)
  TaskLoaded(String)
  SelectedTaskLoaded(Int, String, String, String)
  Changed(String)
  Acted(String)
  Live(String)
  CaughtUp
  Select(String, String)
  Collapse(String)
  NewTaskIn(String)
  DraftScope(String, Bool)
  SetScope(String, Bool)
  ToggleProjects
  ToggleTheme
  AddProject(String)
  RemoveProject(String)
  RemoveOnly
  RemoveFiles
  RemoveTyped(String)
  RemoveDelete
  RemoveCancel
  ProjectName(String)
  ProjectDir(String)
  ProjectTarget(String)
  CreateProject
  AgentSelected(String)
  EffortSelected(String)
  Text(String)
  Title(String)
  Body(String)
  Rank(String)
  NewTask
  DraftTitle(String)
  DraftBody(String)
  Create
  Created(String)
  Edit(Bool)
  Tab(String)
  Preview(String, Bool)
  Format(String, String, String)
  Dismiss(Notice)
  Failed(String)
  Save
  Action(String)
  AddNote
  DeleteNote(Int)
  Answer(String, String)
  Watch(String)
  Unwatch
  TranscriptLoaded(String, String)
  Refresh(Int, String)
  TranscriptPolled(Int, String, String)
  ReviewLoaded(String)
  BaseChanged(String)
  Listening(Source)
  Attached(dynamic.Dynamic)
  Disconnected
  Connected
}

/// While a watched run is going, its transcript is fetched again this often.
const refresh_ms = 1500

pub fn main() {
  let assert Ok(_) = register()
}

fn get(model: Model, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(model.prefix, path, "GET", "", fn(text) { send(msg(text)) })
  })
}

fn send(
  model: Model,
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(
      model.prefix,
      path,
      method,
      json.to_string(json.object(fields)),
      fn(text) { dispatch(Changed(text)) },
    )
  })
}

/// Browser storage keys for the unsent new-task draft.
const draft_title_key = "solace-board-draft-title"

const draft_body_key = "solace-board-draft-body"

const collapsed_key = "solace-board-collapsed"

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  let draft_title = load(draft_title_key)
  let draft_body = load(draft_body_key)
  #(
    Model(
      lanes: [Lane("", "", "", "", "", Workflow([]), [])],
      workspace: False,
      listed: False,
      found: [],
      lane: "",
      selected: None,
      selected_card: None,
      selection_generation: 0,
      models: [],
      model: load(agent_key),
      effort: load(effort_key),
      text: "",
      title: "",
      body: "",
      error: "",
      rank: "0",
      watching: "",
      entries: [],
      review_summary: "",
      creating: draft_title != "" || draft_body != "",
      draft_title: draft_title,
      draft_body: draft_body,
      editing: False,
      tab: "details",
      previews: [],
      notices: [],
      caught_up: False,
      replay: [],
      collapsed: string.split(load(collapsed_key), "\n")
        |> list.filter(fn(d) { d != "" }),
      draft_lane: "",
      draft_scope: [],
      submitting: False,
      acting: "",
      projects_open: False,
      project_name: "",
      project_dir: "",
      project_target: "erlang",
      removing: None,
      source: None,
      prefix: "",
      root: no_root(),
      connected: False,
      poll: 0,
    ),
    // Lustre runs init once for every instance, before its `base` is set:
    // the transport starts when the instance connects.
    effect.none(),
  )
}

fn error(text: String) -> String {
  json.parse(text, decode.at(["message"], decode.string))
  |> result.unwrap("Invalid API response")
}

fn lane_of(model: Model, dir: String) -> Result(Lane, Nil) {
  list.find(model.lanes, fn(l) { l.dir == dir })
}

fn base_of(model: Model, dir: String) -> String {
  lane_of(model, dir) |> result.map(fn(l) { l.base }) |> result.unwrap("")
}

/// The API prefix of the selected task's project.
fn base(model: Model) -> String {
  base_of(model, model.lane)
}

/// The selected task's workflow.
pub fn workflow(model: Model) -> Workflow {
  lane_of(model, model.lane)
  |> result.map(fn(l) { l.workflow })
  |> result.unwrap(Workflow([]))
}

fn map_lane(model: Model, dir: String, f: fn(Lane) -> Lane) -> Model {
  Model(
    ..model,
    lanes: list.map(model.lanes, fn(l) {
      case l.dir == dir {
        True -> f(l)
        False -> l
      }
    }),
  )
}

/// The lane a new task goes to: the one chosen, else the first.
fn draft_lane(model: Model) -> String {
  case lane_of(model, model.draft_lane), model.lanes {
    Ok(l), _ -> l.dir
    _, [l, ..] -> l.dir
    _, [] -> ""
  }
}

fn transcript(model: Model, rid: String) -> Effect(Msg) {
  case model.selected {
    Some(t) ->
      get(model, base(model) <> "/tasks/" <> t.id <> "/runs/" <> rid, fn(text) {
        TranscriptPolled(model.poll, rid, text)
      })
    None -> effect.none()
  }
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Connected -> {
      let #(model, fx) = start(Model(..model, connected: True))
      #(
        model,
        effect.batch([
          fx,
          effect.before_paint(fn(send, root) { send(Attached(root)) }),
        ]),
      )
    }
    Attached(root) -> #(
      Model(..model, root: root),
      effect.from(fn(send) {
        prevent_drag(root)
        images(root, fn(id, image) { send(Format(id, image, "")) }, fn(text) {
          send(Failed(text))
        })
      }),
    )
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    Disconnected -> #(Model(..stop(model), connected: False), effect.none())
    // An attribute set before the element is added only takes effect then.
    BaseChanged(base) -> {
      let model = Model(..model, prefix: base)
      case model.connected {
        True -> start(model)
        False -> #(model, effect.none())
      }
    }
    // Every lane's workflow is loaded again: the list changes rarely. Its
    // tasks come on the event stream.
    ProjectsLoaded(text) ->
      case json.parse(text, projects_decoder()) {
        // A project on its own that gains another needs the merged event
        // stream, which a fresh page opens.
        Ok(#(True, _, _)) if model.listed && !model.workspace -> {
          reload()
          #(model, effect.none())
        }
        Ok(#(workspace, lanes, found)) -> {
          let lanes =
            list.map(lanes, fn(l) {
              case lane_of(model, l.dir) {
                Ok(old) -> Lane(..l, workflow: old.workflow, tasks: old.tasks)
                Error(_) -> l
              }
            })
          #(
            settle(
              Model(
                ..model,
                lanes: lanes,
                workspace: workspace,
                listed: True,
                found: found,
                projects_open: model.projects_open
                  || { workspace && lanes == [] },
                project_name: "",
                project_dir: "",
                removing: None,
                error: "",
              ),
            ),
            effect.batch(
              list.flat_map(lanes, fn(l) {
                [get(model, l.base <> "/workflow", WorkflowLoaded(l.dir, _))]
              }),
            ),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    WorkflowLoaded(dir, text) ->
      case json.parse(text, workflow_decoder()) {
        Ok(w) -> #(
          map_lane(Model(..model, error: ""), dir, fn(l) {
            Lane(..l, workflow: w)
          }),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
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
    SelectedTaskLoaded(generation, dir, id, text)
      if generation == model.selection_generation
      && model.selected_card == Some(#(dir, id))
    -> update(model, TaskLoaded(text))
    SelectedTaskLoaded(_, _, _, _) -> #(model, effect.none())
    TaskLoaded(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> #(
          Model(
            ..model,
            selected: Some(task),
            selected_card: Some(#(model.lane, task.id)),
            title: task.title,
            body: task.body,
            rank: float.to_string(task.rank),
            error: "",
          ),
          case task.changes {
            [] -> effect.none()
            _ ->
              get(model, base(model) <> "/graph?task=" <> task.id, ReviewLoaded)
          },
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    // The reply to an action; it may have waited for the agent's command (D-77 point 5).
    Acted(text) -> update(Model(..model, acting: ""), Changed(text))
    Changed(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> {
          // A task whose home left its projects now lives in the first one left (D-61).
          let model = case
            list.contains(scope_of(model.lane, task), model.lane)
          {
            True -> model
            False ->
              Model(
                ..model,
                lane: list.first(task.scope) |> result.unwrap(model.lane),
              )
          }
          #(
            Model(
              ..model,
              selected: Some(task),
              selected_card: Some(#(model.lane, task.id)),
              title: task.title,
              body: task.body,
              rank: float.to_string(task.rank),
              text: "",
              error: "",
              editing: False,
              previews: [],
            ),
            case task.changes {
              [] -> effect.none()
              _ ->
                get(
                  model,
                  base(model) <> "/graph?task=" <> task.id,
                  ReviewLoaded,
                )
            },
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    // What a new connection missed is replayed: only its end result shows,
    // and it raises no notices.
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> {
      let fx = case projects_event(text) {
        True -> get(model, "/projects", ProjectsLoaded)
        False -> effect.none()
      }
      case knows(model, text) {
        True -> #(live(model, text, True), fx)
        // A project not listed yet: held until the list names it.
        False -> #(Model(..model, replay: [text, ..model.replay]), fx)
      }
    }
    CaughtUp -> #(settle(Model(..model, caught_up: True)), effect.none())
    ReviewLoaded(text) -> {
      let summary =
        json.parse(text, decode.at(["review", "summary"], decode.string))
        |> result.unwrap("")
      #(Model(..model, review_summary: summary), effect.none())
    }
    Select(dir, id) -> #(
      Model(
        ..model,
        lane: dir,
        selected: None,
        selected_card: Some(#(dir, id)),
        selection_generation: model.selection_generation + 1,
        text: "",
        watching: "",
        entries: [],
        review_summary: "",
        editing: False,
        previews: [],
      ),
      get(model, base_of(model, dir) <> "/tasks/" <> id, SelectedTaskLoaded(
        model.selection_generation + 1,
        dir,
        id,
        _,
      )),
    )
    Collapse(dir) -> {
      let collapsed = case list.contains(model.collapsed, dir) {
        True -> list.filter(model.collapsed, fn(d) { d != dir })
        False -> [dir, ..model.collapsed]
      }
      store(collapsed_key, string.join(collapsed, "\n"))
      #(Model(..model, collapsed: collapsed), effect.none())
    }
    NewTaskIn(dir) -> #(
      Model(..model, creating: True, draft_lane: dir, draft_scope: [dir]),
      effect.none(),
    )
    DraftScope(dir, on) -> {
      let chosen = case on {
        True -> [dir, ..model.draft_scope]
        False -> list.filter(model.draft_scope, fn(d) { d != dir })
      }
      // None chosen is allowed while editing, so the starting lane can be
      // swapped for another; the server refuses a task with no project.
      #(
        Model(..model, draft_scope: in_lane_order(model, chosen)),
        effect.none(),
      )
    }
    SetScope(dir, on) ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> {
          let current = scope_of(model.lane, t)
          let wanted = case on {
            True -> [dir, ..current]
            False -> list.filter(current, fn(d) { d != dir })
          }
          #(
            model,
            send(model, base(model) <> "/tasks/" <> t.id <> "/scope", "POST", [
              #("scope", json.array(in_lane_order(model, wanted), json.string)),
            ]),
          )
        }
      }
    ToggleTheme -> #(model, effect.from(fn(_) { toggle_theme() }))
    ToggleProjects -> #(
      Model(..model, projects_open: !model.projects_open),
      effect.none(),
    )
    AddProject(dir) -> #(
      model,
      case
        list.any(model.found, fn(f) { f.dir == dir && f.imported && !f.member })
      {
        True ->
          projects_request(model, "/projects", "POST", [
            #("dir", json.string(dir)),
          ])
        False -> effect.none()
      },
    )
    // Removing asks first whether to keep the files; deleting them takes a
    // second warning and the project's name (D-74).
    RemoveProject(dir) -> #(
      Model(..model, removing: Some(#(dir, False, ""))),
      effect.none(),
    )
    RemoveOnly ->
      case model.removing {
        Some(#(dir, _, _)) -> #(
          model,
          projects_request(
            model,
            "/projects/" <> uri.percent_encode(dir),
            "DELETE",
            [],
          ),
        )
        None -> #(model, effect.none())
      }
    RemoveFiles ->
      case model.removing {
        Some(#(dir, _, _)) -> #(
          Model(..model, removing: Some(#(dir, True, ""))),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    RemoveTyped(text) ->
      case model.removing {
        Some(#(dir, True, _)) -> #(
          Model(..model, removing: Some(#(dir, True, text))),
          effect.none(),
        )
        _ -> #(model, effect.none())
      }
    RemoveDelete ->
      case model.removing {
        Some(#(dir, True, typed)) if typed == dir -> #(
          model,
          projects_request(
            model,
            "/projects/" <> uri.percent_encode(dir),
            "DELETE",
            [#("delete", json.bool(True)), #("name", json.string(typed))],
          ),
        )
        _ -> #(model, effect.none())
      }
    RemoveCancel -> #(Model(..model, removing: None), effect.none())
    ProjectName(text) -> #(Model(..model, project_name: text), effect.none())
    ProjectDir(text) -> #(Model(..model, project_dir: text), effect.none())
    ProjectTarget(text) -> #(
      Model(..model, project_target: text),
      effect.none(),
    )
    CreateProject -> #(
      model,
      projects_request(model, "/projects/new", "POST", [
        #("name", json.string(model.project_name)),
        #("dir", json.string(model.project_dir)),
        #("target", json.string(model.project_target)),
      ]),
    )
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
    NewTask -> #(Model(..model, creating: !model.creating), effect.none())
    DraftTitle(text) -> {
      store(draft_title_key, text)
      #(Model(..model, draft_title: text), effect.none())
    }
    DraftBody(text) -> {
      store(draft_body_key, text)
      #(Model(..model, draft_body: text), effect.none())
    }
    Create if model.submitting -> #(model, effect.none())
    // One request id per submit: the server returns the same task for a repeat of it.
    Create -> #(
      Model(..model, submitting: True),
      effect.from(fn(dispatch) {
        let body =
          json.object([
            #("title", json.string(model.draft_title)),
            #("body", json.string(model.draft_body)),
            #("scope", json.array(model.draft_scope, json.string)),
            #("request", json.string(request_id())),
          ])
        request(
          model.prefix,
          base_of(model, draft_home(model)) <> "/tasks",
          "POST",
          json.to_string(body),
          fn(text) { dispatch(Created(text)) },
        )
      }),
    )
    // Only a task that was really created clears the draft.
    Created(text) -> {
      let model = Model(..model, submitting: False)
      case json.parse(text, task_decoder()) {
        Ok(_) -> {
          store(draft_title_key, "")
          store(draft_body_key, "")
          update(
            Model(
              ..model,
              lane: draft_home(model),
              draft_scope: [],
              creating: False,
              draft_title: "",
              draft_body: "",
            ),
            Changed(text),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    }
    Edit(editing) ->
      case model.selected {
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
    Dismiss(notice) -> #(
      Model(..model, notices: list.filter(model.notices, fn(n) { n != notice })),
      effect.none(),
    )
    Failed(message) -> #(Model(..model, error: message), effect.none())
    Save ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) ->
          case float.parse(model.rank) {
            Ok(rank) -> #(
              model,
              send(model, base(model) <> "/tasks/" <> t.id, "PATCH", [
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
      case model.selected {
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
                  model.prefix,
                  base(model) <> "/tasks/" <> t.id <> "/actions",
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
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> #(
          Model(..model, tab: "notes"),
          send(model, base(model) <> "/tasks/" <> t.id <> "/notes", "POST", [
            #("text", json.string(model.text)),
          ]),
        )
      }
    DeleteNote(i) ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) ->
          case confirm("Delete this note? It can't be restored.") {
            False -> #(model, effect.none())
            True -> #(
              model,
              send(
                model,
                base(model)
                  <> "/tasks/"
                  <> t.id
                  <> "/notes/"
                  <> int.to_string(i),
                "DELETE",
                [],
              ),
            )
          }
      }
    Answer(qid, option) ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> {
          let answer = case option {
            "" -> #("text", json.string(model.text))
            option -> #("option", json.string(option))
          }
          #(
            model,
            send(
              model,
              base(model) <> "/tasks/" <> t.id <> "/questions/" <> qid,
              "POST",
              [
                answer,
              ],
            ),
          )
        }
      }
  }
}

/// Applies one `/events` message; a workspace's name their `project`.
fn live(model: Model, text: String, notify: Bool) -> Model {
  // A project on its own sends no `project`: it is the root's.
  let dir = event_dir(model, text)
  case lane_of(model, dir) {
    Error(_) -> model
    Ok(lane) ->
      case json.parse(text, decode.at(["task"], task_decoder())) {
        Ok(task) -> {
          // Only the selected task's home project may refresh its details.
          let #(selected, model) = case model.selected, model.selected_card {
            Some(t), Some(#(home, id))
              if t.id == task.id && id == task.id && home == dir
            -> #(Some(task), Model(..model, lane: dir))
            other, _ -> #(other, model)
          }
          let new = case
            list.find(lane.tasks, fn(t) { t.id == task.id }),
            notify
          {
            Ok(old), True ->
              notices(old, task)
              |> list.map(fn(n) {
                case model.workspace {
                  True -> Notice(..n, heading: lane.label <> ": " <> n.heading)
                  False -> n
                }
              })
            _, _ -> []
          }
          map_lane(
            Model(
              ..model,
              selected: selected,
              notices: list.append(model.notices, new),
            ),
            dir,
            fn(l) {
              Lane(..l, tasks: [
                task,
                ..list.filter(l.tasks, fn(t) { t.id != task.id })
              ])
            },
          )
        }
        Error(_) ->
          case json.parse(text, decode.at(["workflow"], workflow_decoder())) {
            Ok(w) -> map_lane(model, dir, fn(l) { Lane(..l, workflow: w) })
            Error(_) ->
              // A new connection starts from the project's tasks as they are.
              case
                json.parse(
                  text,
                  decode.at(["tasks"], decode.list(task_decoder())),
                )
              {
                Ok(tasks) ->
                  map_lane(model, dir, fn(l) { Lane(..l, tasks: tasks) })
                Error(_) ->
                  case json.parse(text, decode.at(["removed"], decode.string)) {
                    Ok(id) ->
                      map_lane(model, dir, fn(l) {
                        Lane(
                          ..l,
                          tasks: list.filter(l.tasks, fn(t) { t.id != id }),
                        )
                      })
                      |> follow_move(dir, id)
                    Error(_) -> model
                  }
              }
          }
      }
  }
}

/// Whether a live update says the workspace's project list changed.
fn projects_event(text: String) -> Bool {
  json.parse(text, decode.at(["projects"], decode.list(decode.string)))
  |> result.is_ok
}

/// A `/projects` change; the reply is the new list, or an error.
fn projects_request(
  model: Model,
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(
      model.prefix,
      path,
      method,
      json.to_string(json.object(fields)),
      fn(text) { dispatch(ProjectsLoaded(text)) },
    )
  })
}

fn projects_decoder() -> decode.Decoder(#(Bool, List(Lane), List(Found))) {
  let lane = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use name <- decode.field("name", decode.string)
    use base <- decode.field("base", decode.string)
    use target <- decode.optional_field("target", "", decode.string)
    decode.success(Lane(dir, label, name, base, target, Workflow([]), []))
  }
  let found = {
    use dir <- decode.field("dir", decode.string)
    use name <- decode.field("name", decode.string)
    use imported <- decode.field("imported", decode.bool)
    use member <- decode.field("member", decode.bool)
    decode.success(Found(dir, name, imported, member))
  }
  use workspace <- decode.field("workspace", decode.bool)
  use lanes <- decode.field("projects", decode.list(lane))
  use found <- decode.optional_field("found", [], decode.list(found))
  decode.success(#(workspace, lanes, found))
}

/// What the human should hear about in a live update: a run that ended or a
/// question an agent asked.
pub fn notices(old: Task, new: Task) -> List(Notice) {
  let ended =
    list.filter(new.runs, fn(r) {
      r.state == "ended"
      && list.any(old.runs, fn(o) { o.id == r.id && o.state != "ended" })
    })
    |> list.map(fn(r) {
      case r.outcome {
        "finished" ->
          Notice("Finished " <> new.title, "The agent finished its work.")
        "reported" ->
          Notice("Investigated " <> new.title, "The agent's report is ready.")
        "reviewed" -> Notice("Reviewed " <> new.title, "The review is ready.")
        "" -> Notice("Stopped " <> new.title, "The " <> r.kind <> " run ended.")
        outcome ->
          Notice(
            "Stopped " <> new.title,
            "The " <> r.kind <> " run ended: " <> outcome <> ".",
          )
      }
    })
  let asked =
    list.filter(new.questions, fn(q) {
      q.answer == "" && !list.any(old.questions, fn(o) { o.id == q.id })
    })
    |> list.map(fn(q) { Notice("Question on " <> new.title, q.text) })
  list.append(ended, asked)
}

/// The text an editor shows, and the message that changes it.
fn field(model: Model, id: String) -> #(String, fn(String) -> Msg) {
  case id {
    "draft-body" -> #(model.draft_body, DraftBody)
    "edit-body" -> #(model.body, Body)
    _ -> #(model.text, Text)
  }
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
      use project <- decode.optional_field("project", "", decode.string)
      // `module.symbol`: the symbol follows the last dot.
      let parts = string.split(name, ".")
      let #(module, symbol) = case list.reverse(parts) {
        [symbol, ..rest] if rest != [] -> #(
          string.join(list.reverse(rest), "."),
          symbol,
        )
        _ -> #("", name)
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
  ))
}

fn note_decoder() -> decode.Decoder(Note) {
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

fn entry_decoder() -> decode.Decoder(Entry) {
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

fn agent_decoder() -> decode.Decoder(Agent) {
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

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

pub fn view(model: Model) -> Element(Msg) {
  h.main([], [
    h.style([], board_styles <> selected_card_styles <> modal_styles),
    h.header([a.class("top")], [
      h.h1([], [h.text("Solace tasks")]),
      h.a([a.href("/guide")], [h.text("Guide")]),
      h.button(
        [a.aria_expanded(model.projects_open), e.on_click(ToggleProjects)],
        [h.text("Projects")],
      ),
      h.button([a.class("primary"), e.on_click(NewTask)], [
        h.text(case model.creating {
          True -> "Close new task"
          False -> "New task"
        }),
      ]),
      h.button(
        [
          a.class("theme"),
          a.attribute("aria-label", "Switch between light and dark theme"),
          a.title("Switch between light and dark theme"),
          e.on_click(ToggleTheme),
        ],
        [
          svg.svg(
            [
              a.class("sun"),
              a.attribute("viewBox", "0 0 24 24"),
              a.attribute("aria-hidden", "true"),
            ],
            [
              svg.path([
                a.attribute(
                  "d",
                  "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
                ),
              ]),
            ],
          ),
          svg.svg(
            [
              a.class("moon"),
              a.attribute("viewBox", "0 0 24 24"),
              a.attribute("aria-hidden", "true"),
            ],
            [
              svg.path([
                a.attribute("d", "M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z"),
              ]),
            ],
          ),
        ],
      ),
    ]),
    h.p([a.role("alert")], [h.text(model.error)]),
    h.div(
      [a.class("notices")],
      list.map(model.notices, fn(n) {
        h.div([a.class("notice"), a.role("alert")], [
          h.strong([], [h.text(n.heading)]),
          h.p([], [h.text(n.text)]),
          h.button(
            [
              a.class("close"),
              a.attribute("aria-label", "Dismiss"),
              a.title("Dismiss"),
              e.on_click(Dismiss(n)),
            ],
            [
              svg.svg(
                [
                  a.attribute("viewBox", "0 0 24 24"),
                  a.attribute("aria-hidden", "true"),
                ],
                [svg.path([a.attribute("d", "M6 6l12 12M18 6L6 18")])],
              ),
            ],
          ),
        ])
      }),
    ),
    case model.projects_open {
      True -> projects_view(model)
      False -> h.text("")
    },
    case model.creating {
      False -> h.text("")
      True ->
        h.section([a.class("editor")], [
          case model.workspace {
            True -> projects_pick(model, model.draft_scope, False, DraftScope)
            False -> h.text("")
          },
          title_input(model.draft_title, DraftTitle),
          editor(model, "draft-body", "", "Description"),
          h.button([a.disabled(model.submitting), e.on_click(Create)], [
            h.text(case model.submitting {
              True -> "Creating…"
              False -> "Create task"
            }),
          ]),
        ])
    },
    lanes_view(model),
    case model.selected {
      None -> h.text("Select a task to see its details.")
      Some(t) -> detail(model, t)
    },
  ])
}

/// Whether every lane has the same columns, so one header row serves all.
pub fn shared_columns(lanes: List(Lane)) -> Bool {
  case lanes {
    [first, ..rest] -> list.all(rest, fn(l) { l.workflow == first.workflow })
    [] -> True
  }
}

/// One lane per project. A single project's header has no toggle and no
/// + Task, since the New task button is above. Lanes with the same workflow
/// share one header row; otherwise each lane names its own columns. A header
/// row is a bar that sticks to the top while its lanes are on screen.
fn lanes_view(model: Model) -> Element(Msg) {
  let shared = model.workspace && shared_columns(model.lanes)
  case model.workspace, model.lanes {
    False, [lane] ->
      h.div([], [
        h.header([a.class("lane-title")], [
          h.span([], lane_name(lane)),
          task_count(cards(model, lane)),
          h.a([a.href(lane.base <> "/view")], [h.text("Contract view")]),
        ]),
        columns_view(model, lane, True),
      ])
    True, [] ->
      h.p([a.class("muted")], [
        h.text(
          "No projects on this board yet. Add or create one under Projects.",
        ),
      ])
    _, lanes ->
      h.div([a.class("lanes")], [
        case shared, lanes {
          True, [first, ..] -> head_view(first.workflow.columns)
          _, _ -> h.text("")
        },
        ..list.map(lanes, fn(lane) {
          let open = !list.contains(model.collapsed, lane.dir)
          h.section([a.class("lane")], [
            h.header([a.class("lane-title")], [
              h.button(
                [
                  a.class("lane-toggle"),
                  a.aria_expanded(open),
                  e.on_click(Collapse(lane.dir)),
                ],
                [
                  h.text(case open {
                    True -> "▾ "
                    False -> "▸ "
                  }),
                  ..lane_name(lane)
                ],
              ),
              task_count(cards(model, lane)),
              h.button([e.on_click(NewTaskIn(lane.dir))], [h.text("+ Task")]),
              h.a([a.href(lane.base <> "/view")], [h.text("Contract view")]),
            ]),
            ..case open, shared {
              True, True -> [columns_view(model, lane, False)]
              True, False -> [
                head_view(lane.workflow.columns),
                columns_view(model, lane, False),
              ]
              False, _ -> []
            }
          ])
        })
      ])
  }
}

/// A lane's columns, with every task that covers its project (D-61); a card
/// opens its task from the project it is stored in.
fn columns_view(model: Model, lane: Lane, labels: Bool) -> Element(Msg) {
  h.div(
    [a.class("board")],
    list.map(lane.workflow.columns, fn(c) {
      let tasks =
        cards(model, lane)
        |> list.filter(fn(card) { { card.1 }.column == c.id })
        |> list.sort(fn(a, b) {
          case { a.1 }.rank <. { b.1 }.rank {
            True -> order.Lt
            False -> order.Gt
          }
        })
      h.section([a.class("column"), a.attribute("aria-label", c.label)], [
        case labels {
          True -> h.h2([], [h.text(c.label)])
          False -> h.text("")
        },
        ..list.map(tasks, fn(card) {
          let #(home, t) = card
          let current = model.selected_card == Some(#(home, t.id))
          h.button(
            [
              a.class(case current {
                True -> "card selected"
                False -> "card"
              }),
              a.attribute("aria-current", case current {
                True -> "true"
                False -> "false"
              }),
              e.on_click(Select(home, t.id)),
            ],
            [
              case t.attention {
                "" -> h.text("")
                _ ->
                  h.strong([a.class("attention")], [h.text("Needs attention ")])
              },
              case t.stamps {
                [] -> h.text("")
                stamps ->
                  h.span(
                    [a.class("stamp"), a.title(string.join(stamps, ", "))],
                    [
                      h.text("✓"),
                      h.span([a.class("sr-only")], [
                        h.text(" " <> string.join(stamps, ", ") <> ": "),
                      ]),
                    ],
                  )
              },
              case t.unexpected {
                False -> h.text("")
                True -> hazard()
              },
              h.text(t.title),
              case t.scope {
                [_, _, ..] ->
                  h.small([a.class("muted")], [
                    h.text(string.join(
                      list.map(t.scope, label_of(model, _)),
                      " · ",
                    )),
                  ])
                _ -> h.text("")
              },
              status_badge(t),
            ],
          )
        })
      ])
    }),
  )
}

/// The workspace's projects: remove one, add one found in the folder, or
/// create a new Gleam project.
fn projects_view(model: Model) -> Element(Msg) {
  let others = list.filter(model.found, fn(f) { !f.member })
  h.section([a.class("editor projects")], [
    h.h2([], [h.text("Projects")]),
    h.ul(
      [],
      list.map(model.lanes, fn(l) {
        h.li([], [
          h.strong([], [h.text(l.label)]),
          h.text(" " <> l.name <> " "),
          // The root's own project always stays on its board.
          case l.dir {
            "." -> h.small([], [h.text("(this folder)")])
            dir -> button("Remove", RemoveProject(dir))
          },
        ])
      }),
    ),
    removal_view(model),
    case others {
      [] -> h.text("")
      _ ->
        h.div([], [
          h.h3([], [h.text("Found in this folder")]),
          h.ul(
            [],
            list.map(others, fn(f) {
              h.li([], [
                h.strong([], [
                  h.text(case f.dir {
                    "." -> "This folder"
                    dir -> dir
                  }),
                ]),
                h.text(
                  " "
                  <> f.name
                  <> case f.imported {
                    True -> " (already set up) "
                    False -> " (not set up) "
                  },
                ),
                case f.imported {
                  True -> button("Add", AddProject(f.dir))
                  False ->
                    h.small([], [
                      h.text("Existing code is brought in on the "),
                      h.a([a.href("/migrate")], [h.text("Migration page")]),
                      h.text("."),
                    ])
                },
              ])
            }),
          ),
        ])
    },
    h.h3([], [h.text("New project")]),
    h.div([a.class("new-project")], [
      h.input([
        a.placeholder("Package name, e.g. my_app"),
        a.attribute("aria-label", "Package name"),
        a.value(model.project_name),
        e.on_input(ProjectName),
      ]),
      h.input([
        a.placeholder(case model.project_name {
          "" -> "Folder (default: the package name)"
          name -> "Folder (default: " <> name <> ")"
        }),
        a.attribute("aria-label", "Folder"),
        a.value(model.project_dir),
        e.on_input(ProjectDir),
      ]),
      h.label([], [
        h.text("Runs on"),
        h.select([a.value(model.project_target), e.on_input(ProjectTarget)], [
          h.option([a.value("erlang")], "Erlang (BEAM)"),
          h.option([a.value("javascript")], "JavaScript"),
        ]),
      ]),
      button("Create project", CreateProject),
    ]),
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
    h.h2([], [h.text(t.title)]),
    h.p([a.class("meta")], [h.text(t.column), status_badge(t)]),
    case t.attention {
      "" -> h.text("")
      reason ->
        h.div([a.class("attention"), a.role("status")], [
          h.strong([], [h.text("Needs attention")]),
          md(base(model), reason),
        ])
    },
    agent_choice(model),
    composer(model, t),
    case model.acting {
      "" -> h.text("")
      name ->
        h.p([a.class("muted"), a.role("status")], [h.text(acting_text(t, name))])
    },
    h.div(
      [a.class("actions")],
      list.append(
        list.map(actions(workflow(model), t), fn(name) {
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
      "notes" -> notes_view(base(model), t)
      "questions" -> questions_view(base(model), t)
      "runs" -> list.map(list.reverse(t.runs), fn(r) { run_view(model, r) })
      "changes" -> [
        case t.changes {
          [] -> h.text("")
          _ ->
            h.p([], [
              h.text(model.review_summary <> " "),
              h.a([a.href(base(model) <> "/view?task=" <> t.id)], [
                h.text("Contract view"),
              ]),
            ])
        },
        h.div(
          [],
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
      editor(model, "edit-body", t.id, "Description"),
      h.label([], [
        h.text("Rank (lower comes first)"),
        h.input([a.type_("number"), a.value(model.rank), e.on_input(Rank)]),
      ]),
      button("Save", Save),
      button("Cancel", Edit(False)),
    ]
    False -> [
      md(base(model), t.body),
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

fn title_input(value: String, msg: fn(String) -> Msg) -> Element(Msg) {
  h.input([
    a.class("title"),
    a.placeholder("Title"),
    a.attribute("aria-label", "Title"),
    a.value(value),
    e.on_input(msg),
  ])
}

fn composer(model: Model, t: Task) -> Element(Msg) {
  editor(model, "note-text", t.id, "Note, answer or instructions")
}

/// A markdown editor with Write/Preview and formatting tools; `id` names
/// what it edits (see `field`). Images pasted, dropped or picked here are uploaded by
/// `api.mjs` to `task` and inserted as markdown. A task not yet created has no
/// image tool.
fn editor(
  model: Model,
  id: String,
  task: String,
  label: String,
) -> Element(Msg) {
  let #(text, msg) = field(model, id)
  let preview = list.contains(model.previews, id)
  let tool = fn(label, title, before, after) {
    h.button(
      [a.type_("button"), a.title(title), e.on_click(Format(id, before, after))],
      [h.text(label)],
    )
  }
  h.div([a.class("composer")], [
    h.div([a.class("toolbar")], [
      h.button(
        [
          a.type_("button"),
          a.aria_pressed(bool_text(!preview)),
          e.on_click(Preview(id, False)),
        ],
        [h.text("Write")],
      ),
      h.button(
        [
          a.type_("button"),
          a.aria_pressed(bool_text(preview)),
          e.on_click(Preview(id, True)),
        ],
        [h.text("Preview")],
      ),
      h.span([a.class("separator")], []),
      tool("B", "Bold", "**", "**"),
      tool("`code`", "Code", "`", "`"),
      tool("• List", "List", "\n- ", ""),
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
              a.attribute("data-base", base(model)),
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
            text -> md(base(model), text)
          },
        ])
      False ->
        h.textarea(
          [
            a.id(id),
            a.class("md-editor"),
            a.attribute("data-task", task),
            a.attribute("data-base", base(model)),
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
            e.on_input(msg),
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
    _, Error(_) ->
      case t.finished {
        "finished" -> "Agent finished"
        "reported" -> "Report ready"
        "reviewed" -> "Review ready"
        _ -> ""
      }
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

fn status_badge(t: Task) -> Element(Msg) {
  case status(t) {
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

/// Markdown; images uploaded to the board load from the project at `base`.
fn md(base: String, text: String) -> Element(Msg) {
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
              list.map(entries, entry_view(base(model), _)),
            )
        }
    },
  ])
}

fn entry_view(base: String, e: Entry) -> Element(Msg) {
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

/// The hazard mark on an Unexpected Changes card (D-58).
fn hazard() -> Element(Msg) {
  let what = "Unexpected changes: code changed outside Solace"
  h.span([a.class("hazard"), a.title(what)], [
    svg.svg(
      [a.attribute("viewBox", "0 0 24 24"), a.attribute("aria-hidden", "true")],
      [
        svg.path([a.attribute("d", "M12 3L2 21h20L12 3z")]),
        svg.path([a.attribute("d", "M12 10v5M12 18v.01"), a.class("mark")]),
      ],
    ),
    h.span([a.class("sr-only")], [h.text(what <> ": ")]),
  ])
}

/// The project's folder, then its package name when that differs, then its target.
fn lane_name(lane: Lane) -> List(Element(Msg)) {
  [
    h.strong([], [h.text(lane.label)]),
    case lane.name == lane.label {
      True -> h.text("")
      False -> h.small([], [h.text(" " <> lane.name)])
    },
    case lane.target {
      "" -> h.text("")
      target -> h.small([a.class("lane-target muted")], [h.text(" " <> target)])
    },
  ]
}

fn task_count(cards: List(#(String, Task))) -> Element(Msg) {
  let count = list.length(cards)
  h.small([], [
    h.text(
      int.to_string(count)
      <> case count {
        1 -> " task"
        _ -> " tasks"
      },
    ),
  ])
}

/// The projects a task covers; a task saved before D-61 covers its own.
fn scope_of(home: String, t: Task) -> List(String) {
  case t.scope {
    [] -> [home]
    scope -> scope
  }
}

/// The tasks a lane shows, each with the project it is stored in: every task
/// on the board whose projects include this lane's (D-61).
fn cards(model: Model, lane: Lane) -> List(#(String, Task)) {
  list.flat_map(model.lanes, fn(l) {
    l.tasks
    |> list.filter(fn(t) { list.contains(scope_of(l.dir, t), lane.dir) })
    |> list.map(fn(t) { #(l.dir, t) })
  })
}

fn label_of(model: Model, dir: String) -> String {
  lane_of(model, dir) |> result.map(fn(l) { l.label }) |> result.unwrap(dir)
}

/// Project folders in the order the lanes show them, each once.
fn in_lane_order(model: Model, dirs: List(String)) -> List(String) {
  model.lanes
  |> list.map(fn(l) { l.dir })
  |> list.filter(list.contains(dirs, _))
}

/// The project a new task is stored in: the draft lane when it is one of the
/// task's projects, else the first of them.
fn draft_home(model: Model) -> String {
  let chosen = model.draft_scope
  case list.contains(chosen, draft_lane(model)), chosen {
    True, _ | _, [] -> draft_lane(model)
    False, [first, ..] -> first
  }
}

/// A switch per project on the board, for the projects a task can change.
fn projects_pick(
  model: Model,
  chosen: List(String),
  disabled: Bool,
  msg: fn(String, Bool) -> Msg,
) -> Element(Msg) {
  h.fieldset([a.class("projects-pick"), a.disabled(disabled)], [
    h.legend([], [h.text("Projects this task can change")]),
    ..list.map(model.lanes, fn(l) {
      h.label([], [
        h.input([
          a.type_("checkbox"),
          a.role("switch"),
          a.checked(list.contains(chosen, l.dir)),
          e.on_check(msg(l.dir, _)),
        ]),
        h.text(l.label),
      ])
    })
  ])
}

pub type Source

@external(javascript, "./api.mjs", "close_events")
fn close_events(source: Source) -> Nil

@external(javascript, "./api.mjs", "prevent_drag")
fn prevent_drag(root: dynamic.Dynamic) -> Nil

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("base", fn(base) { Ok(BaseChanged(base)) }),
      component.on_disconnect(Disconnected),
      component.on_connect(Connected),
      component.adopt_styles(False),
    ]),
    "solace-board",
  )
}

const board_styles = ".projects-pick{margin:8px 0}.projects-pick label{display:flex;align-items:center;gap:10px;margin:6px 0;cursor:pointer}.projects-pick input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}.projects-pick input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}.projects-pick input:checked{background:var(--focus)}.projects-pick input:checked::before{transform:translateX(14px)}.projects-pick input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}.projects-pick[disabled] label,.projects-pick[disabled] input{opacity:.5;cursor:default}@media (prefers-reduced-motion:reduce){.projects-pick input,.projects-pick input::before{transition:none}}\n\n:host{display:block;--well:#eceee6;--hover:#e7eeea;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--edge:var(--ink);--sun:none;--moon:inline-block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a;--focus:#174f78;box-sizing:border-box;padding:0 24px 48px;min-height:100dvh;user-select:none;-webkit-user-select:none}input,textarea,pre,code,.md,.view-code,.detail > h2,.detail > .panel > article > p,.detail > .panel > p,.detail > .panel > ul > li,.run > header > strong,.run > header > .state,.run > small{user-select:text;-webkit-user-select:text}\nmain{font:16px system-ui,sans-serif;color:var(--ink);background:var(--sheet)}button,input,textarea,select{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:0}button{cursor:pointer;background:var(--paper)}button:hover{background:var(--hover)}button:focus-visible{outline:3px solid var(--focus)}label{display:block}textarea{display:block;width:min(95%,700px);min-height:70px}input{width:min(80%,600px)}.board{display:flex;gap:12px;overflow-x:auto;margin:24px 0}.column{min-width:190px;flex:1;background:var(--well);padding:12px;border-radius:0}.column h2{font-size:17px}.card{display:block;width:100%;text-align:left}.detail,.editor{border:1px solid var(--line);padding:16px;border-radius:0}small{display:block;color:var(--soft);overflow-wrap:anywhere}pre{white-space:pre-wrap;font:inherit}.actions{display:flex;flex-wrap:wrap;gap:6px}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}.note,.run{border:1px solid var(--line);border-radius:0;padding:8px 12px;margin:8px 0;background:var(--paper)}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.md p,.md ul,.md ol{margin:6px 0}.md pre,.transcript pre{background:var(--code);padding:8px;border-radius:0;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace;max-height:320px}code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:0}.transcript{border-left:3px solid var(--line);padding-left:10px;margin-top:8px}.entry{margin:6px 0}details.entry>summary{cursor:pointer;color:var(--soft)}.tool .icon{display:inline-block;width:1.2em}.tool.ok .icon{color:var(--ok-text)}.tool.failed .icon{color:var(--revision)}.muted{color:var(--soft)}.note .delete{margin-left:auto;font-size:13px;padding:2px 8px}\n.top{align-items:center;gap:16px}.top h1{margin:0}.top .primary{margin-left:auto}.primary{background:var(--focus);color:var(--paper);border-color:var(--focus)}.primary:hover{background:var(--focus)}.column{min-height:240px}.card+.card{margin-top:6px}\n.meta{display:flex;gap:10px;align-items:center;color:var(--soft);margin:4px 0 12px}.agent{display:flex;flex-wrap:wrap;gap:4px 16px}.agent label{display:flex;flex-direction:column}\n.composer{border:1px solid var(--edge);border-radius:0;margin:8px 4px;max-width:760px;background:var(--paper)}.composer textarea{width:100%;max-width:none;box-sizing:border-box;border:0;border-top:1px solid var(--line);border-radius:0 0 6px 6px;margin:0;min-height:110px}.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:2px;padding:4px}.toolbar button,.toolbar .button{margin:0;padding:4px 10px;font-size:14px}.sr-only{position:absolute;width:1px;height:1px;opacity:0;overflow:hidden}.toolbar .button{position:relative;display:inline-block;border:1px solid var(--edge);border-radius:0;background:var(--paper);cursor:pointer}.toolbar .button:hover{background:var(--hover)}.toolbar .button:focus-within{outline:3px solid var(--focus)}.toolbar [aria-pressed=true]{background:var(--well);font-weight:600}.separator{width:1px;align-self:stretch;background:var(--line);margin:0 6px}.preview{border-top:1px solid var(--line);padding:8px 12px;min-height:94px}\n.tabs{display:flex;flex-wrap:wrap;gap:0;border-bottom:1px solid var(--line);margin-top:16px}.tabs button{margin:0 0 -1px;border-radius:0 0 0;border-color:transparent;background:transparent}.tabs [aria-selected=true]{border-color:var(--line) var(--line) var(--sheet);background:var(--sheet);font-weight:600}.panel{padding-top:8px}\ndetails.note>summary{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap;cursor:pointer;list-style:none}details.note>summary::-webkit-details-marker{display:none}details.note>summary::before{content:\"▸\";color:var(--soft);width:1em}details.note[open]>summary::before{content:\"▾\"}details.note[open]>summary{margin-bottom:4px}.md img{max-width:100%;height:auto;border:1px solid var(--line);border-radius:0}.status.finished{background:var(--add);color:var(--ok-text)}.diff{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:8px;border-radius:0;max-width:760px}\n.lanes{display:grid;gap:0;margin:24px 0;overflow-x:auto;padding-bottom:4px}.lanes .board{margin:0;overflow:visible}.lane .board{margin:8px 0}.lane{border:1px solid var(--line);border-radius:0;padding:4px 8px;background:var(--paper)}.lane .column{min-height:110px}.lane-title{align-items:center;gap:12px}.lane-title small{display:inline}.lane-title button{padding:4px 10px}.lane-toggle{border:0;background:transparent;font-size:17px}.lane-head{margin:0;padding:0 9px}.lane-head .column{min-height:0;padding:0 12px;background:transparent}.lane-head h2{margin:6px 0}\n.projects{margin-top:16px}.projects h2{margin-top:0}.projects h3{font-size:15px;margin:16px 0 4px}.projects ul{list-style:none;padding:0;margin:0}.projects li{display:flex;align-items:center;gap:6px;white-space:normal;margin:0}.projects li button{padding:4px 10px}.new-project{display:flex;flex-wrap:wrap;align-items:flex-end;gap:4px}.new-project input{width:auto;flex:1;min-width:220px}.new-project label{display:flex;flex-direction:column}\n.title{display:block;box-sizing:border-box;width:calc(100% - 8px);max-width:762px;margin:8px 4px}.stamp{color:var(--ok-text);font-weight:700;margin-right:6px}.hazard{display:inline-block;width:18px;height:18px;margin-right:6px;vertical-align:-3px}.hazard svg{width:100%;height:100%}.hazard path{fill:#f2c94c;stroke:#7a5200;stroke-width:1.5;stroke-linejoin:round}.hazard path.mark{fill:none;stroke:#3d2900;stroke-width:2.5;stroke-linecap:round}.notices{position:fixed;right:16px;bottom:16px;z-index:10;display:grid;gap:8px;max-width:min(380px,calc(100% - 32px))}.notices .notice{position:relative;background:var(--ink);color:var(--paper);padding:12px 52px 12px 14px;border-radius:0;box-shadow:0 4px 16px var(--shadow)}.notice strong{display:block;overflow-wrap:anywhere}.notice p{margin:4px 0 0;overflow-wrap:anywhere}.notice .close{position:absolute;top:6px;right:6px;margin:0;padding:6px;line-height:0;background:transparent;color:var(--paper);border:0}.notice .close:hover{background:color-mix(in srgb,var(--paper) 20%,transparent)}.notice .close svg{width:24px;height:24px;fill:none;stroke:currentColor;stroke-width:2.5;stroke-linecap:round}
:host{background:var(--sheet);color:var(--ink)}
main{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;color:var(--ink);background:none}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button,input,textarea,select{border:1px solid var(--edge);background:var(--paper);color:var(--ink);caret-color:var(--focus)}input::placeholder,textarea::placeholder{color:var(--soft);opacity:1}
button:hover{background:var(--hover)}button:disabled{color:var(--soft)}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
textarea{font-family:inherit}
.top{margin:0 -24px 24px;padding:12px 22px;background:var(--paper);border-bottom:2px solid var(--edge)}.top h1{font-size:28px}
.primary{background:var(--ink);color:var(--paper);border-color:var(--edge);font-weight:700}.primary:hover{background:var(--focus);border-color:var(--focus);color:var(--paper)}
.theme{margin:0 0 0 4px;padding:6px;line-height:0}.theme svg{width:18px;height:18px;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round}.theme .sun{display:var(--sun)}.theme .moon{display:var(--moon)}
.column{background:var(--well);border:1px solid var(--line);padding:0 12px 12px}
.column h2{font-size:13px;text-transform:uppercase;letter-spacing:.1em;margin:0 -12px 12px;padding:10px 12px;background:var(--paper);border-bottom:2px solid var(--edge)}
.lanes{display:block;overflow:visible}.lane-head{position:sticky;top:0;z-index:5;background:var(--paper);border-bottom:2px solid var(--edge)}.lane-head .column{background:transparent;border:0}.lane-head h2{margin:0 -12px;border-bottom:0}.lane .lane-head{margin:0;padding:0}
.lane{border-color:var(--edge);background:var(--paper)}.lane-title{padding:6px 0}.lane-toggle{font-weight:700;background:transparent}.lane-toggle:hover{background:var(--hover)}
.card{position:relative;background:var(--paper);border-color:var(--edge);padding:10px 12px 10px 22px;line-height:1.35;--tab:var(--file)}
.card::before{content:\"\";position:absolute;inset:0 auto 0 0;width:8px;background:var(--tab)}
.card:has(.working){--tab:var(--logic)}.card:has(.stamp){--tab:var(--entry)}.card:has(.attention){--tab:var(--effect)}.card:has(.hazard){--tab:var(--revision)}
.card:hover{background:var(--hover)}.card+.card{margin-top:8px}
.attention{font-weight:700}.stamp{margin-right:6px}
.status{border:1px solid var(--line);background:var(--paper);font-size:11px;text-transform:uppercase;letter-spacing:.08em;padding:1px 8px}.status.finished{background:var(--add);color:var(--ok-text);border-color:var(--ok-text)}.working::before{background:var(--entry)}
.detail,.editor,.composer,.note,.run,.projects{background:var(--paper);border-color:var(--edge)}.detail{padding:20px 24px}
.tabs{border-bottom:2px solid var(--edge)}.tabs button{border:0;border-right:1px solid var(--line);background:transparent;margin:0;padding:10px 16px}.tabs [aria-selected=true]{background:var(--paper);box-shadow:inset 0 -3px var(--ink);border-color:var(--line)}
.composer textarea{border:0;border-top:1px solid var(--edge)}.toolbar .button{border-color:var(--edge);background:var(--paper)}.toolbar .button:hover{background:var(--hover)}.toolbar [aria-pressed=true]{background:var(--ink);color:var(--paper)}
article{border-bottom-color:var(--line)}.transcript{border-left:1px solid var(--edge)}
.md pre,.transcript pre,.diff,code{background:var(--code);color:var(--ink)}
.notices .notice{background:var(--ink);color:var(--paper);box-shadow:4px 8px 24px var(--shadow)}.notice .close{color:var(--paper)}
@media(prefers-reduced-motion:no-preference){button,.card{transition:background-color .12s ease-out}}
.lane+.lane{margin-top:-1px}
.top a,.lane-title a{display:inline-flex;align-items:center;gap:8px;padding:6px 12px;border:1px solid var(--edge);background:var(--paper);color:var(--ink);text-decoration:none;font-weight:700}
.top a:hover,.lane-title a:hover{background:var(--hover)}
.top a::after,.lane-title a::after{content:\"\";width:14px;height:14px;background:currentColor;-webkit-mask:url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M4 12h15M13 5l7 7-7 7'/%3E%3C/svg%3E\") center/contain no-repeat;mask:url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M4 12h15M13 5l7 7-7 7'/%3E%3C/svg%3E\") center/contain no-repeat}"

/// Opens this instance's transport with its own `base`: the event stream, one
/// the followed run's transcript and the first loads.
/// Runs on connection and when `base` changes; the fresh stream replays what
/// it missed.
fn start(model: Model) -> #(Model, Effect(Msg)) {
  let model = Model(..stop(model), caught_up: False, replay: [])
  #(
    model,
    effect.batch([
      get(model, "/projects", ProjectsLoaded),
      get(model, "/models", ModelsLoaded),
      case model.watching {
        "" -> effect.none()
        rid -> transcript(model, rid)
      },
      effect.from(fn(send) {
        send(
          Listening(
            events(model.prefix, fn(text) { send(Live(text)) }, fn() {
              send(CaughtUp)
            }),
          ),
        )
      }),
    ]),
  )
}

/// Closes the event stream and ends the transcript polling chain.
fn stop(model: Model) -> Model {
  case model.source {
    Some(source) -> close_events(source)
    None -> Nil
  }
  Model(..model, source: None, poll: model.poll + 1)
}

@external(javascript, "./api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

const selected_card_styles = "
button.card[aria-current=true],button.card[aria-current=true]:hover{background:var(--hover);border-color:var(--ink);box-shadow:0 0 0 2px var(--ink),4px 6px 14px var(--shadow)}button.card[aria-current=true]:focus-visible{outline:3px solid var(--focus);outline-offset:5px}@media(forced-colors:active){button.card[aria-current=true],button.card[aria-current=true]:hover{border-color:Highlight;box-shadow:0 0 0 2px Highlight}}"

/// The open task left its home project: if it moved to another project it
/// covers (D-61), it is opened from there.
fn follow_move(model: Model, dir: String, id: String) -> Model {
  case model.selected, model.selected_card {
    Some(t), Some(#(home, card_id))
      if home == dir && card_id == id && t.id == id
    ->
      case
        list.find_map(model.lanes, fn(l) {
          case l.dir != dir && list.contains(t.scope, l.dir) {
            True ->
              list.find(l.tasks, fn(x) { x.id == id })
              |> result.map(fn(x) { #(l.dir, x) })
            False -> Error(Nil)
          }
        })
      {
        Ok(#(to, task)) ->
          Model(
            ..model,
            lane: to,
            selected: Some(task),
            selected_card: Some(#(to, id)),
          )
        Error(_) -> model
      }
    _, _ -> model
  }
}

/// Browser storage keys for the last agent and effort chosen.
const agent_key = "solace-board-agent"

const effort_key = "solace-board-effort"

/// One changed symbol, split so the Changes tab can group it.
pub type Change {
  Change(project: String, module: String, name: String)
}

/// Changes as an outline: source modules before test modules, each module's
/// symbols alphabetical. Headings name the project too when `multi_project`.
pub fn change_groups(
  changes: List(Change),
  multi_project: Bool,
) -> List(#(String, List(String))) {
  let is_test = fn(c: Change) {
    case string.ends_with(c.module, "_test") {
      True -> 1
      False -> 0
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

@external(javascript, "./api.mjs", "toggle_theme")
fn toggle_theme() -> Nil

/// The column names as one bar, shared by lanes with one workflow or one per
/// lane when they differ. It is the first thing in its container, so it sticks
/// to the top of the page until the last lane in that container scrolls past.
fn head_view(columns: List(Column)) -> Element(Msg) {
  h.div(
    [a.class("board lane-head"), a.attribute("aria-hidden", "true")],
    list.map(columns, fn(c) {
      h.div([a.class("column")], [h.h2([], [h.text(c.label)])])
    }),
  )
}

/// The dialog for removing a project: keep its files, or delete them after a
/// second warning and typing its name.
fn removal_view(model: Model) -> Element(Msg) {
  case model.removing {
    None -> h.text("")
    Some(#(dir, False, _)) ->
      modal([
        h.p([], [
          h.text(
            "Remove "
            <> dir
            <> " from this board? Its tasks and Solace setup stay in its folder, and it can be added again. Should its folder and files be deleted too?",
          ),
        ]),
        button("Remove only", RemoveOnly),
        button("Remove and delete files…", RemoveFiles),
        button("Cancel", RemoveCancel),
      ])
    Some(#(dir, True, typed)) ->
      modal([
        h.p([], [
          h.strong([], [
            h.text(
              "This permanently deletes "
              <> dir
              <> " and everything in it. It cannot be undone.",
            ),
          ]),
        ]),
        h.label([], [
          h.text("Type " <> dir <> " to confirm"),
          h.input([
            a.attribute("aria-label", "Project name"),
            a.attribute("autofocus", ""),
            a.value(typed),
            e.on_input(RemoveTyped),
          ]),
        ]),
        h.button([a.disabled(typed != dir), e.on_click(RemoveDelete)], [
          h.text("Delete permanently"),
        ]),
        button("Cancel", RemoveCancel),
      ])
  }
}

const modal_styles = ".modal-backdrop{position:fixed;inset:0;z-index:20;display:flex;align-items:center;justify-content:center;background:var(--shadow);padding:16px}.removal{background:var(--paper);color:var(--ink);border:2px solid var(--edge);box-shadow:0 8px 32px var(--shadow);padding:20px 24px;width:min(480px,100%);max-height:100%;overflow:auto}.removal p{margin:0 0 12px;overflow-wrap:anywhere}.removal input{width:calc(100% - 8px);box-sizing:border-box}.removal button:disabled{opacity:.5;cursor:not-allowed}"

/// A modal dialog: a backdrop over the page that blocks the board behind it.
fn modal(body: List(Element(Msg))) -> Element(Msg) {
  h.div([a.class("modal-backdrop")], [
    h.div(
      [
        a.class("removal"),
        a.attribute("role", "alertdialog"),
        a.attribute("aria-modal", "true"),
      ],
      body,
    ),
  ])
}

/// A new id for a request, so the server can tell a repeat of it.
@external(javascript, "./api.mjs", "request_id")
fn request_id() -> String

/// The Scope tab: a switch per project the task can change, locked while the
/// agent works.
fn scope_view(model: Model, t: Task) -> Element(Msg) {
  let working =
    t.paused == "" && list.any(t.runs, fn(r) { r.state == "running" })
  h.div([], [
    projects_pick(model, scope_of(model.lane, t), working, SetScope),
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

/// The project a live message is for: the one it names, else the root's.
fn event_dir(model: Model, text: String) -> String {
  case json.parse(text, decode.at(["project"], decode.string)) {
    Ok(dir) -> dir
    Error(_) ->
      list.find(model.lanes, fn(l) { l.base == "" })
      |> result.map(fn(l) { l.dir })
      |> result.unwrap("")
  }
}

/// Whether a live message can be applied now: its lane is loaded, or it is
/// the list of projects, which names no lane.
fn knows(model: Model, text: String) -> Bool {
  projects_event(text) || result.is_ok(lane_of(model, event_dir(model, text)))
}

/// Applies the held messages whose lane is loaded; the others wait for the
/// project list to name it (a snapshot can arrive before it).
fn settle(model: Model) -> Model {
  case model.caught_up {
    False -> model
    True -> {
      let #(known, unknown) =
        list.partition(list.reverse(model.replay), fn(t) { knows(model, t) })
      list.fold(
        known,
        Model(..model, replay: list.reverse(unknown)),
        fn(m, text) { live(m, text, False) },
      )
    }
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

fn action_button(name: String, busy: Bool) -> Element(Msg) {
  h.button([e.on_click(Action(name)), a.disabled(busy)], [h.text(name)])
}
