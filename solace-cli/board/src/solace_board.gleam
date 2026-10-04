import gleam/dict
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
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/element/svg
import lustre/event as e

@external(javascript, "./api.mjs", "request")
fn request(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(receive: fn(String) -> Nil, ready: fn() -> Nil) -> Nil

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
fn wrap(id: String, text: String, before: String, after: String) -> String

@external(javascript, "./api.mjs", "images")
fn images(insert: fn(String, String) -> Nil, fail: fn(String) -> Nil) -> Nil

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
    changes: List(String),
    stamps: List(String),
    attention: String,
    finished: String,
    unexpected: Bool,
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
    workflow: Workflow,
    tasks: List(Task),
  )
}

/// A Gleam project found in the workspace folder.
pub type Found {
  Found(dir: String, name: String, imported: Bool, member: Bool)
}

pub type MigrationItem {
  MigrationItem(text: String, status: String, detail: String)
}

pub type MigrationCheck {
  MigrationCheck(
    name: String,
    status: String,
    summary: String,
    items: List(MigrationItem),
  )
}

pub type MigrationReport {
  MigrationReport(
    checked_at: String,
    current: Bool,
    checks: List(MigrationCheck),
    warnings: List(String),
  )
}

pub type Migration {
  Migration(
    state: String,
    top: String,
    original: String,
    copy: String,
    step: String,
    candidates: List(#(String, String)),
    report: Option(MigrationReport),
    flagged: List(MigrationItem),
    problem: String,
    blockers: List(String),
    undo: Bool,
  )
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
    projects_open: Bool,
    project_name: String,
    project_dir: String,
    project_target: String,
    migration: Option(Migration),
    migration_open: Bool,
    migration_projects: List(String),
    migration_smoke: String,
    migration_busy: Bool,
    migration_generation: Int,
    migration_error: String,
    migration_dismissed: Bool,
  )
}

pub type Msg {
  MigrationLoaded(Int, String)
  MigrationRefresh
  ToggleMigration
  MigrationProject(String, Bool)
  MigrationSmoke(String)
  MigrationAction(String)
  MigrationChanged(String)
  DismissMigration
  ProjectsLoaded(String)
  WorkflowLoaded(String, String)
  TasksLoaded(String, String)
  ModelsLoaded(String)
  TaskLoaded(String)
  Changed(String)
  Live(String)
  CaughtUp
  Select(String, String)
  Collapse(String)
  NewTaskIn(String)
  DraftLane(String)
  ToggleProjects
  AddProject(String)
  RemoveProject(String)
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
  Refresh(String)
  ReviewLoaded(String)
}

/// While a watched run is going, its transcript is fetched again this often.
const refresh_ms = 1500

pub fn main() {
  let assert Ok(_) =
    lustre.start(lustre.application(init, update, view), "#app", Nil)
  Nil
}

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(path, "GET", "", fn(text) { send(msg(text)) })
  })
}

fn send(
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(path, method, json.to_string(json.object(fields)), fn(text) {
      dispatch(Changed(text))
    })
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
      lanes: [Lane("", "", "", "", Workflow([]), [])],
      workspace: False,
      listed: False,
      found: [],
      lane: "",
      selected: None,
      models: [],
      model: "",
      effort: "",
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
      projects_open: False,
      project_name: "",
      project_dir: "",
      project_target: "erlang",
      migration: None,
      migration_open: False,
      migration_projects: [],
      migration_smoke: "",
      migration_busy: False,
      migration_generation: 0,
      migration_error: "",
      migration_dismissed: False,
    ),
    effect.batch([
      get("/migration", MigrationLoaded(0, _)),
      get("/projects", ProjectsLoaded),
      get("/models", ModelsLoaded),
      effect.from(fn(send) {
        events(fn(text) { send(Live(text)) }, fn() { send(CaughtUp) })
      }),
      effect.from(fn(send) {
        images(fn(id, image) { send(Format(id, image, "")) }, fn(text) {
          send(Failed(text))
        })
      }),
    ]),
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
      get(base(model) <> "/tasks/" <> t.id <> "/runs/" <> rid, fn(text) {
        TranscriptLoaded(rid, text)
      })
    None -> effect.none()
  }
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    MigrationRefresh -> #(
      model,
      get("/migration", MigrationLoaded(model.migration_generation, _)),
    )
    MigrationLoaded(generation, text) -> {
      // Keep one polling chain, including through a server restart. A reply
      // fetched before an action was accepted must not end its progress panel.
      let next = case
        json.parse(text, migration_decoder()),
        model.migration_busy
      {
        Ok(migration), False if generation == model.migration_generation -> {
          case model.migration {
            Some(old)
              if old.state == "running"
              && { migration.state != old.state || migration.top != old.top }
            -> reload()
            _ -> Nil
          }
          let failure =
            json.parse(text, decode.at(["error"], decode.string))
            |> result.unwrap("")
          Model(
            ..model,
            migration: Some(migration),
            migration_error: case failure {
              "" -> model.migration_error
              message -> message
            },
          )
        }
        _, _ -> model
      }
      let delay = case next.migration {
        Some(m) if m.state == "running" -> 1000
        _ -> 5000
      }
      #(
        next,
        effect.from(fn(send) { after(delay, fn() { send(MigrationRefresh) }) }),
      )
    }
    ToggleMigration -> {
      let projects = case model.migration {
        Some(migration) -> list.map(migration.candidates, fn(c) { c.0 })
        None -> []
      }
      #(
        Model(
          ..model,
          migration_open: !model.migration_open,
          migration_projects: projects,
        ),
        effect.none(),
      )
    }
    MigrationProject(dir, checked) -> #(
      Model(..model, migration_projects: case checked {
        True -> list.unique([dir, ..model.migration_projects])
        False -> list.filter(model.migration_projects, fn(p) { p != dir })
      }),
      effect.none(),
    )
    MigrationSmoke(text) -> #(
      Model(..model, migration_smoke: text),
      effect.none(),
    )
    DismissMigration -> #(
      Model(..model, migration_dismissed: True),
      effect.none(),
    )
    MigrationAction(action) -> migration_action(model, action)
    MigrationChanged(text) -> {
      let code = json.parse(text, decode.at(["code"], decode.string))
      case code, model.migration {
        Ok(code), _ -> #(
          Model(
            ..model,
            migration_busy: False,
            migration_error: code <> ": " <> error(text),
          ),
          effect.none(),
        )
        _, Some(migration) -> #(
          Model(
            ..model,
            migration_busy: False,
            migration_open: False,
            migration: Some(
              Migration(
                ..migration,
                state: "running",
                step: "Waiting for the server…",
              ),
            ),
            migration_error: case
              json.parse(text, decode.at(["state"], decode.string))
            {
              Ok(_) -> ""
              Error(_) -> error(text)
            },
          ),
          effect.none(),
        )
        _, None -> #(
          Model(..model, migration_busy: False, migration_error: error(text)),
          effect.none(),
        )
      }
    }
    // Every lane is loaded again: the list changes rarely.
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
            Model(
              ..model,
              lanes: lanes,
              workspace: workspace,
              listed: True,
              found: found,
              projects_open: model.projects_open || { workspace && lanes == [] },
              project_name: "",
              project_dir: "",
              error: "",
            ),
            effect.batch(
              list.flat_map(lanes, fn(l) {
                [
                  get(l.base <> "/workflow", WorkflowLoaded(l.dir, _)),
                  get(l.base <> "/tasks", TasksLoaded(l.dir, _)),
                ]
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
    TasksLoaded(dir, text) ->
      case json.parse(text, decode.list(task_decoder())) {
        Ok(tasks) -> #(
          map_lane(Model(..model, error: ""), dir, fn(l) {
            Lane(..l, tasks: tasks)
          }),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    ModelsLoaded(text) ->
      case json.parse(text, decode.list(agent_decoder())) {
        Ok(models) -> #(
          Model(
            ..model,
            models: models,
            model: list.find(models, fn(a) { a.available })
              |> result.map(fn(a) { a.id })
              |> result.unwrap(""),
          ),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    TaskLoaded(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> #(
          Model(
            ..model,
            selected: Some(task),
            title: task.title,
            body: task.body,
            rank: float.to_string(task.rank),
            error: "",
          ),
          case task.changes {
            [] -> effect.none()
            _ -> get(base(model) <> "/graph?task=" <> task.id, ReviewLoaded)
          },
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    Changed(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> #(
          Model(
            ..model,
            selected: Some(task),
            title: task.title,
            body: task.body,
            rank: float.to_string(task.rank),
            text: "",
            error: "",
            editing: False,
            previews: [],
          ),
          effect.batch([
            get(base(model) <> "/tasks", TasksLoaded(model.lane, _)),
            case task.changes {
              [] -> effect.none()
              _ -> get(base(model) <> "/graph?task=" <> task.id, ReviewLoaded)
            },
          ]),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    // What a new connection missed is replayed: only its end result shows,
    // and it raises no notices.
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> #(live(model, text, True), case projects_event(text) {
      True -> get("/projects", ProjectsLoaded)
      False -> effect.none()
    })
    CaughtUp -> #(
      list.fold(
        list.reverse(model.replay),
        Model(..model, caught_up: True, replay: []),
        fn(m, text) { live(m, text, False) },
      ),
      effect.none(),
    )
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
        text: "",
        watching: "",
        entries: [],
        review_summary: "",
        editing: False,
        previews: [],
      ),
      get(base_of(model, dir) <> "/tasks/" <> id, TaskLoaded),
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
      Model(..model, creating: True, draft_lane: dir),
      effect.none(),
    )
    DraftLane(dir) -> #(Model(..model, draft_lane: dir), effect.none())
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
          projects_request("/projects", "POST", [#("dir", json.string(dir))])
        False -> effect.none()
      },
    )
    RemoveProject(dir) ->
      case
        confirm(
          "Remove "
          <> dir
          <> " from this board? Its tasks and Solace setup stay in its folder, and it can be added again.",
        )
      {
        True -> #(
          model,
          projects_request(
            "/projects/" <> uri.percent_encode(dir),
            "DELETE",
            [],
          ),
        )
        False -> #(model, effect.none())
      }
    ProjectName(text) -> #(Model(..model, project_name: text), effect.none())
    ProjectDir(text) -> #(Model(..model, project_dir: text), effect.none())
    ProjectTarget(text) -> #(
      Model(..model, project_target: text),
      effect.none(),
    )
    CreateProject -> #(
      model,
      projects_request("/projects/new", "POST", [
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
    Refresh(rid) if rid == model.watching -> #(model, transcript(model, rid))
    Refresh(_) -> #(model, effect.none())
    TranscriptLoaded(rid, text) if rid == model.watching -> {
      let decoder = {
        use state <- decode.field("state", decode.string)
        use entries <- decode.field("transcript", decode.list(entry_decoder()))
        decode.success(#(state, entries))
      }
      case json.parse(text, decoder) {
        Ok(#(state, entries)) -> #(
          Model(..model, entries: entries),
          case state {
            "ended" -> effect.none()
            _ ->
              effect.from(fn(send) {
                after(refresh_ms, fn() { send(Refresh(rid)) })
              })
          },
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    }
    TranscriptLoaded(_, _) -> #(model, effect.none())
    // Efforts differ between models, so a new model starts at its default.
    AgentSelected(id) -> #(Model(..model, model: id, effort: ""), effect.none())
    EffortSelected(effort) -> #(Model(..model, effort: effort), effect.none())
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
    Create -> #(
      model,
      effect.from(fn(dispatch) {
        let body =
          json.object([
            #("title", json.string(model.draft_title)),
            #("body", json.string(model.draft_body)),
          ])
        request(
          base_of(model, draft_lane(model)) <> "/tasks",
          "POST",
          json.to_string(body),
          fn(text) { dispatch(Created(text)) },
        )
      }),
    )
    // Only a task that was really created clears the draft.
    Created(text) ->
      case json.parse(text, task_decoder()) {
        Ok(_) -> {
          store(draft_title_key, "")
          store(draft_body_key, "")
          update(
            Model(
              ..model,
              lane: draft_lane(model),
              creating: False,
              draft_title: "",
              draft_body: "",
            ),
            Changed(text),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
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
        effect.from(fn(send) { send(msg(wrap(id, text, before, after))) }),
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
              send(base(model) <> "/tasks/" <> t.id, "PATCH", [
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
            _ -> True
          }
          case proceed {
            True -> #(
              model,
              send(
                base(model) <> "/tasks/" <> t.id <> "/actions",
                "POST",
                fields,
              ),
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
          send(base(model) <> "/tasks/" <> t.id <> "/notes", "POST", [
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
  let dir = case json.parse(text, decode.at(["project"], decode.string)) {
    Ok(dir) -> dir
    Error(_) ->
      list.find(model.lanes, fn(l) { l.base == "" })
      |> result.map(fn(l) { l.dir })
      |> result.unwrap("")
  }
  case lane_of(model, dir) {
    Error(_) -> model
    Ok(lane) ->
      case json.parse(text, decode.at(["task"], task_decoder())) {
        Ok(task) -> {
          let selected = case model.selected {
            Some(t) if t.id == task.id && model.lane == dir -> Some(task)
            other -> other
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
            Error(_) -> model
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
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(path, method, json.to_string(json.object(fields)), fn(text) {
      dispatch(ProjectsLoaded(text))
    })
  })
}

fn projects_decoder() -> decode.Decoder(#(Bool, List(Lane), List(Found))) {
  let lane = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use name <- decode.field("name", decode.string)
    use base <- decode.field("base", decode.string)
    decode.success(Lane(dir, label, name, base, Workflow([]), []))
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

pub fn migration_decoder() -> decode.Decoder(Migration) {
  let nullable_text =
    decode.optional(decode.string) |> decode.map(option.unwrap(_, ""))
  let candidate = {
    use dir <- decode.field("dir", decode.string)
    use name <- decode.field("name", decode.string)
    decode.success(#(dir, name))
  }
  let item = {
    use text <- decode.field("text", decode.string)
    use status <- decode.optional_field("status", "", nullable_text)
    use detail <- decode.optional_field("detail", "", nullable_text)
    decode.success(MigrationItem(text, status, detail))
  }
  let check = {
    use name <- decode.field("name", decode.string)
    use status <- decode.field("status", decode.string)
    use summary <- decode.field("summary", decode.string)
    use items <- decode.field("items", decode.list(item))
    decode.success(MigrationCheck(name, status, summary, items))
  }
  let report = {
    use checked_at <- decode.field(
      "checked_at",
      decode.one_of(decode.string, [decode.map(decode.int, clock)]),
    )
    use current <- decode.field("current", decode.bool)
    use checks <- decode.field("checks", decode.list(check))
    use warnings <- decode.optional_field(
      "warnings",
      [],
      decode.list(decode.string),
    )
    decode.success(MigrationReport(checked_at, current, checks, warnings))
  }
  use state <- decode.field("state", decode.string)
  use top <- decode.field("top", decode.string)
  use original <- decode.optional_field("original", "", nullable_text)
  use copy <- decode.optional_field("copy", "", nullable_text)
  use step <- decode.optional_field("step", "", nullable_text)
  use candidates <- decode.optional_field(
    "candidates",
    [],
    decode.list(candidate),
  )
  use report <- decode.optional_field("report", None, decode.optional(report))
  use flagged <- decode.optional_field("flagged", [], decode.list(item))
  use problem <- decode.optional_field("problem", "", nullable_text)
  use blockers <- decode.optional_field(
    "blockers",
    [],
    decode.list(decode.string),
  )
  use undo <- decode.optional_field("undo", False, decode.bool)
  decode.success(Migration(
    state,
    top,
    original,
    copy,
    step,
    candidates,
    report,
    flagged,
    problem,
    blockers,
    undo,
  ))
}

fn migration_action(model: Model, action: String) -> #(Model, Effect(Msg)) {
  case model.migration {
    None -> #(model, effect.none())
    Some(m) -> {
      let allowed =
        !model.migration_busy
        && case action {
          "start" -> m.state == "available" && model.migration_projects != []
          "confirm" -> m.state == "confirm_paths"
          "recheck" -> m.state == "in_progress"
          "abandon" ->
            list.contains(
              ["in_progress", "confirm_paths", "failed", "copying"],
              m.state,
            )
          "adopt" -> m.state == "in_progress" && m.blockers == []
          "undo" -> m.state == "adopted" && m.undo
          _ -> False
        }
      let question = case action {
        "confirm" ->
          "Continue the migration? You have looked at the files listed."
        "adopt" -> {
          let unchecked = case m.report {
            None -> ["Migration report not checked"]
            Some(report) ->
              list.flat_map(report.checks, fn(c) {
                let items =
                  list.filter(c.items, fn(i) { i.status == "not checked" })
                  |> list.map(fn(i) { c.name <> ": " <> i.text })
                case c.status {
                  "not checked" -> [c.name <> ": " <> c.summary, ..items]
                  _ -> items
                }
              })
          }
          "Adopt this migration? The original will be renamed to "
          <> m.original
          <> ".pre-solace and the copy takes its path ("
          <> m.original
          <> ")."
          <> "\n\nErrors in the original carry over: a build or test that fails before the migration fails after it too."
          <> case unchecked {
            [] -> ""
            _ -> "\n\nNot checked:\n• " <> string.join(unchecked, "\n• ")
          }
        }
        "abandon" ->
          "Abandon migration? This deletes the copy at "
          <> m.copy
          <> ". The original at "
          <> m.original
          <> " stays untouched."
        "undo" ->
          "Undo adoption? Restore the original kept at "
          <> m.original
          <> ".pre-solace to "
          <> m.original
          <> ". Work done after adoption stays in the renamed migration copy."
        _ -> ""
      }
      case allowed && { question == "" || confirm(question) } {
        False -> #(model, effect.none())
        True -> {
          let fields = case action {
            "start" -> [
              #("projects", json.array(model.migration_projects, json.string)),
              #(
                "smoke",
                string.split(model.migration_smoke, "\n")
                  |> list.map(string.trim)
                  |> list.filter(fn(s) { s != "" })
                  |> json.array(json.string),
              ),
            ]
            _ -> []
          }
          #(
            Model(
              ..model,
              migration_busy: True,
              migration_generation: model.migration_generation + 1,
              migration_error: "",
            ),
            effect.from(fn(send) {
              request(
                "/migration/" <> action,
                "POST",
                json.to_string(json.object(fields)),
                fn(text) { send(MigrationChanged(text)) },
              )
            }),
          )
        }
      }
    }
  }
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
    decode.list(decode.at(["name"], decode.string)),
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
  case list.find(workflow.columns, fn(c) { c.id == task.column }) {
    Ok(c) if c.final -> []
    Ok(c) ->
      list.append(c.actions, case task.paused {
        "" -> ["pause"]
        _ -> ["resume"]
      })
    Error(_) -> []
  }
}

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

fn migration_view(model: Model) -> Element(Msg) {
  case model.migration {
    None -> h.text("")
    Some(m) ->
      h.section([a.class("migration")], [
        case model.migration_error {
          "" -> h.text("")
          message -> h.p([a.role("alert")], [h.text(message)])
        },
        case m.state {
          "available" ->
            h.div([], [
              button("Migrate this project", ToggleMigration),
              case model.migration_open {
                False -> h.text("")
                True ->
                  h.section(
                    [
                      a.class("editor"),
                      a.role("dialog"),
                      a.attribute("aria-labelledby", "migration-title"),
                    ],
                    [
                      h.h2([a.id("migration-title")], [
                        h.text("Migrate this project"),
                      ]),
                      h.p([], [
                        h.text(
                          "Solace copies the whole folder to ../solace-migration/ and never changes the original. Choose at least one project to set up in the copy.",
                        ),
                      ]),
                      h.p([], [
                        h.text(
                          "Errors in the original carry over: a build or test that fails before the migration fails after it too.",
                        ),
                      ]),
                      h.fieldset([a.disabled(model.migration_busy)], [
                        h.legend([], [h.text("Projects to migrate")]),
                        h.div(
                          [],
                          list.map(m.candidates, fn(c) {
                            h.label([], [
                              h.input([
                                a.type_("checkbox"),
                                a.checked(list.contains(
                                  model.migration_projects,
                                  c.0,
                                )),
                                e.on_check(MigrationProject(c.0, _)),
                              ]),
                              h.text(c.1 <> " (" <> c.0 <> ")"),
                            ])
                          }),
                        ),
                      ]),
                      h.label([], [
                        h.text("Smoke check commands (optional)"),
                        h.textarea(
                          [
                            a.value(model.migration_smoke),
                            a.disabled(model.migration_busy),
                            a.attribute(
                              "aria-describedby",
                              "migration-smoke-hint",
                            ),
                            e.on_input(MigrationSmoke),
                          ],
                          model.migration_smoke,
                        ),
                      ]),
                      h.small([a.id("migration-smoke-hint")], [
                        h.text(
                          "One shell command per line, run in the root folder. Each must exit 0. Without any, app behaviour is marked not checked.",
                        ),
                      ]),
                      h.button(
                        [
                          a.class("primary"),
                          a.disabled(
                            model.migration_projects == []
                            || model.migration_busy,
                          ),
                          e.on_click(MigrationAction("start")),
                        ],
                        [h.text("Start")],
                      ),
                      button("Close", ToggleMigration),
                    ],
                  )
              },
            ])
          "running" ->
            h.section(
              [
                a.class("editor"),
                a.role("status"),
                a.attribute("aria-live", "polite"),
              ],
              [
                h.h2([], [h.text("Migration in progress")]),
                h.p([], [h.text(m.step)]),
                h.small([], [
                  h.text(
                    "This page will reload when the operation finishes. Reconnecting automatically if the server restarts.",
                  ),
                ]),
              ],
            )
          "in_progress" | "confirm_paths" | "failed" | "copying" ->
            h.div([], [
              h.div([a.class("editor migration-banner")], [
                h.strong([], [
                  h.text(case m.state {
                    "confirm_paths" -> "Look at these paths before continuing"
                    "failed" -> "Migration stopped"
                    "copying" -> "Copying the original"
                    _ -> "Migration in progress"
                  }),
                ]),
                h.p([], [h.text("Copy: "), h.code([], [h.text(m.copy)])]),
                h.p([], [h.text("Original: "), h.code([], [h.text(m.original)])]),
              ]),
              case m.state {
                "confirm_paths" ->
                  h.div([], [
                    h.p([], [
                      h.text(
                        "These could reach your original from inside the copy. Look at each one before continuing; nothing has run yet.",
                      ),
                    ]),
                    h.ul([], list.map(m.flagged, migration_item)),
                  ])
                "failed" ->
                  h.p([a.role("alert")], [h.strong([], [h.text(m.problem)])])
                "copying" ->
                  h.p([], [
                    h.text(
                      "A copy of the original is being made, or an earlier one was interrupted.",
                    ),
                  ])
                _ -> h.text("")
              },
              case m.report, m.state {
                Some(_), _ | _, "in_progress" -> migration_report(m.report)
                _, _ -> h.text("")
              },
              case m.state {
                "in_progress" ->
                  h.div([], [
                    h.div([a.id("migration-blockers")], [
                      case m.blockers {
                        [] -> h.text("")
                        _ -> h.p([], [h.text("Adoption is blocked:")])
                      },
                      h.ul(
                        [],
                        list.map(m.blockers, fn(b) { h.li([], [h.text(b)]) }),
                      ),
                    ]),
                    h.div([a.class("actions")], [
                      h.button(
                        [
                          a.disabled(model.migration_busy),
                          e.on_click(MigrationAction("recheck")),
                        ],
                        [h.text("Recheck")],
                      ),
                      h.button(
                        [
                          a.disabled(model.migration_busy || m.blockers != []),
                          a.attribute("aria-describedby", "migration-blockers"),
                          e.on_click(MigrationAction("adopt")),
                        ],
                        [h.text("Adopt")],
                      ),
                    ]),
                  ])
                "confirm_paths" ->
                  h.button(
                    [
                      a.class("primary"),
                      a.disabled(model.migration_busy),
                      e.on_click(MigrationAction("confirm")),
                    ],
                    [h.text("Continue")],
                  )
                _ -> h.text("")
              },
              h.button(
                [
                  a.disabled(model.migration_busy),
                  e.on_click(MigrationAction("abandon")),
                ],
                [h.text("Abandon migration")],
              ),
            ])
          "adopted" if m.undo && !model.migration_dismissed ->
            h.div([a.class("notice editor")], [
              h.strong([], [h.text("Migration adopted")]),
              h.p([], [
                h.text("The original is kept at "),
                h.code([], [h.text(m.original <> ".pre-solace")]),
              ]),
              h.button(
                [
                  a.disabled(model.migration_busy),
                  e.on_click(MigrationAction("undo")),
                ],
                [h.text("Undo adoption")],
              ),
              button("Dismiss", DismissMigration),
            ])
          _ -> h.text("")
        },
        case model.migration_busy {
          True ->
            h.p([a.role("status")], [h.text("Starting migration operation…")])
          False -> h.text("")
        },
      ])
  }
}

fn migration_item(item: MigrationItem) -> Element(Msg) {
  h.li([], [
    h.text(item.text <> " "),
    migration_status(item.status),
    case item.detail {
      "" -> h.text("")
      detail ->
        h.details([a.class("entry")], [
          h.summary([], [h.text("Details")]),
          h.pre([a.class("diff")], [
            h.text(detail),
          ]),
        ])
    },
  ])
}

fn migration_report(report: Option(MigrationReport)) -> Element(Msg) {
  h.details([a.class("editor"), a.attribute("open", "")], [
    h.summary([], [h.text("Migration report")]),
    case report {
      None ->
        h.p([], [
          h.text("No report yet. Recheck to run the checks."),
        ])
      Some(report) ->
        h.div([], [
          h.div(
            [],
            list.map(report.warnings, fn(warning) {
              h.p([a.role("alert")], [h.strong([], [h.text(warning)])])
            }),
          ),
          h.small([], [
            h.text(
              "Checked: "
              <> {
                int.parse(report.checked_at)
                |> result.map(clock)
                |> result.unwrap(report.checked_at)
              },
            ),
          ]),
          case report.current {
            True -> h.text("")
            False ->
              h.p([], [
                migration_status("out of date"),
                h.text(" The copy changed. Recheck before adopting."),
              ])
          },
          h.div(
            [],
            list.map(report.checks, fn(c) {
              h.article([], [
                h.h3([], [
                  h.text(c.name <> " "),
                  migration_status(c.status),
                ]),
                h.p([], [h.text(c.summary)]),
                h.ul([], list.map(c.items, migration_item)),
              ])
            }),
          ),
        ])
    },
    h.p([], [h.text("These checks are evidence, not proof of every behaviour.")]),
  ])
}

fn migration_status(status: String) -> Element(Msg) {
  case status {
    "" -> h.text("")
    _ ->
      h.span(
        [a.class("status migration-" <> string.replace(status, " ", "-"))],
        [h.text(status)],
      )
  }
}

pub fn view(model: Model) -> Element(Msg) {
  h.main([], [
    migration_view(model),
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
            True ->
              h.label([], [
                h.text("Project"),
                h.select(
                  [e.on_input(DraftLane)],
                  list.map(model.lanes, fn(l) {
                    h.option(
                      [a.value(l.dir), a.selected(l.dir == draft_lane(model))],
                      l.label,
                    )
                  }),
                ),
              ])
            False -> h.text("")
          },
          title_input(model.draft_title, DraftTitle),
          editor(model, "draft-body", "", "Description"),
          button("Create task", Create),
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

/// One lane per project (a single project's has no header). Lanes with the
/// same workflow share one header row; otherwise each names its columns.
fn lanes_view(model: Model) -> Element(Msg) {
  let shared = model.workspace && shared_columns(model.lanes)
  case model.workspace, model.lanes {
    False, [lane] -> columns_view(lane, True)
    True, [] ->
      h.p([a.class("muted")], [
        h.text(
          "No projects on this board yet. Add or create one under Projects.",
        ),
      ])
    _, lanes ->
      h.div([a.class("lanes")], [
        case shared, lanes {
          True, [first, ..] ->
            h.div(
              [a.class("board lane-head"), a.attribute("aria-hidden", "true")],
              list.map(first.workflow.columns, fn(c) {
                h.div([a.class("column")], [h.h2([], [h.text(c.label)])])
              }),
            )
          _, _ -> h.text("")
        },
        ..list.map(lanes, fn(lane) {
          let open = !list.contains(model.collapsed, lane.dir)
          let count = list.length(lane.tasks)
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
                  h.strong([], [h.text(lane.label)]),
                  case lane.name == lane.label {
                    True -> h.text("")
                    False -> h.small([], [h.text(" " <> lane.name)])
                  },
                ],
              ),
              h.small([], [
                h.text(
                  int.to_string(count)
                  <> case count {
                    1 -> " task"
                    _ -> " tasks"
                  },
                ),
              ]),
              h.button([e.on_click(NewTaskIn(lane.dir))], [h.text("+ Task")]),
              h.a([a.href(lane.base <> "/view")], [h.text("Contract view")]),
            ]),
            case open {
              True -> columns_view(lane, !shared)
              False -> h.text("")
            },
          ])
        })
      ])
  }
}

fn columns_view(lane: Lane, labels: Bool) -> Element(Msg) {
  h.div(
    [a.class("board")],
    list.map(lane.workflow.columns, fn(c) {
      let tasks =
        lane.tasks
        |> list.filter(fn(t) { t.column == c.id })
        |> list.sort(fn(a, b) {
          case a.rank <. b.rank {
            True -> order.Lt
            False -> order.Gt
          }
        })
      h.section([a.class("column"), a.attribute("aria-label", c.label)], [
        case labels {
          True -> h.h2([], [h.text(c.label)])
          False -> h.text("")
        },
        ..list.map(tasks, fn(t) {
          h.button([a.class("card"), e.on_click(Select(lane.dir, t.id))], [
            case t.attention {
              "" -> h.text("")
              _ ->
                h.strong([a.class("attention")], [h.text("Needs attention ")])
            },
            case t.stamps {
              [] -> h.text("")
              stamps ->
                h.span([a.class("stamp"), a.title(string.join(stamps, ", "))], [
                  h.text("✓"),
                  h.span([a.class("sr-only")], [
                    h.text(" " <> string.join(stamps, ", ") <> ": "),
                  ]),
                ])
            },
            case t.unexpected {
              False -> h.text("")
              True -> hazard()
            },
            h.text(t.title),
            status_badge(t),
          ])
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
                      h.text("Existing code is brought in by migrating."),
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
    h.div(
      [a.class("actions")],
      list.append(
        list.map(actions(workflow(model), t), fn(name) {
          button(name, Action(name))
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
      "questions" -> questions_view(t)
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
        h.ul([], list.map(t.changes, fn(c) { h.li([], [h.text(c)]) })),
      ]
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
            True -> h.option([a.value(m.id)], m.label)
            False ->
              h.option(
                [a.value(m.id), a.disabled(True)],
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
            h.option([a.value("")], case m.effort {
              "" -> "Default"
              effort -> "Default (" <> effort <> ")"
            }),
            ..list.map(m.efforts, fn(effort) {
              h.option([a.value(effort)], effort)
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

fn questions_view(t: Task) -> List(Element(Msg)) {
  list.map(list.reverse(t.questions), fn(q) {
    let #(diff, other) = case q.settings {
      Some(#(base, proposed)) -> #(
        [h.pre([a.class("diff")], [h.text(settings_diff(base, proposed))])],
        "Reject with the editor's text",
      )
      None -> #([], "Answer with the editor's text")
    }
    h.article([], [
      h.p([], [h.text(q.text)]),
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
    "", Ok(r) if r.state == "waiting" -> "Waiting for your answer"
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
