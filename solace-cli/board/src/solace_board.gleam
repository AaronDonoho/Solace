import gleam/dict
import gleam/dynamic
import gleam/dynamic/decode
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
import solace_task

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

@external(javascript, "./api.mjs", "reload")
fn reload() -> Nil

@external(javascript, "./api.mjs", "load")
fn load(key: String) -> String

@external(javascript, "./api.mjs", "store")
fn store(key: String, value: String) -> Nil

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
    workflow: solace_task.Workflow,
    tasks: List(solace_task.Task),
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
    /// The task chosen on the board: the project it is stored in, and its id.
    /// Its details are `<solace-task>`, which loads them itself.
    selected_card: Option(#(String, String)),
    error: String,
    creating: Bool,
    draft_title: String,
    draft_body: String,
    previews: List(String),
    notices: List(solace_task.Notice),
    caught_up: Bool,
    replay: List(String),
    collapsed: List(String),
    draft_lane: String,
    /// The projects a new task covers; empty means the draft lane alone.
    draft_scope: List(String),
    /// Whether a create is waiting for the server; Create task is disabled
    /// meanwhile, so one submit makes one task (D-77).
    submitting: Bool,
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
    /// The live messages about the open task, numbered, oldest first and
    /// kept to the last few, handed to its `<solace-task>` so it needs no
    /// stream of its own. It applies each once, so none is lost when several
    /// arrive before a redraw, and a replay after a reconnect reaches it.
    live: List(#(Int, String)),
    /// The number of the last message handed down.
    live_n: Int,
    /// The tab and task a notice's button asked to show, and how many times
    /// one was asked, so asking again is news to `<solace-task>`.
    reveal: #(String, String, Int),
  )
}

pub type Msg {
  ProjectsLoaded(String)
  WorkflowLoaded(String, String)
  Live(String)
  CaughtUp
  Select(String, String)
  /// A finished card's Archive button: its task leaves the board (D-95).
  Archive(String, String)
  Archived(String)
  /// `<solace-task>` says the open task now lives in another project.
  TaskMoved(String, String)
  Collapse(String)
  NewTaskIn(String)
  DraftScope(String, Bool)
  ToggleProjects
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
  NewTask
  DraftTitle(String)
  DraftBody(String)
  Create
  Created(String)
  Preview(String, Bool)
  Format(String, String, String)
  /// The editor's selected lines become a block: "heading", "list",
  /// "numbered", "quote" or "code" (see `block` in the components project).
  Block(String, String)
  Dismiss(solace_task.Notice)
  /// Close All: every toast goes.
  DismissAll
  /// A notice's button: its task opens on the notice's tab and scrolls into view.
  Open(solace_task.Notice)
  /// A task link (D-98): the shell asks the board to open this task, stored
  /// in project `dir`, and bring its details into view.
  Focus(String, String)
  BaseChanged(String)
  Listening(Source)
  Attached(dynamic.Dynamic)
  Disconnected
  Connected
}

pub fn main() {
  let assert Ok(_) = register()
}

fn get(model: Model, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(model.prefix, path, "GET", "", fn(text) { send(msg(text)) })
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
      lanes: [
        Lane("", "", "", "", "", solace_task.Workflow([], dict.new()), []),
      ],
      workspace: False,
      listed: False,
      found: [],
      selected_card: None,
      error: "",
      creating: draft_title != "" || draft_body != "",
      draft_title: draft_title,
      draft_body: draft_body,
      previews: [],
      notices: [],
      caught_up: False,
      replay: [],
      collapsed: string.split(load(collapsed_key), "\n")
        |> list.filter(fn(d) { d != "" }),
      draft_lane: "",
      draft_scope: [],
      submitting: False,
      projects_open: False,
      project_name: "",
      project_dir: "",
      project_target: "erlang",
      removing: None,
      source: None,
      prefix: "",
      root: no_root(),
      connected: False,
      live: [],
      live_n: 0,
      reveal: #("", "", 0),
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
      effect.from(fn(_) { prevent_drag(root) }),
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
      case json.parse(text, solace_task.workflow_decoder()) {
        Ok(w) -> #(
          map_lane(Model(..model, error: ""), dir, fn(l) {
            Lane(..l, workflow: w)
          }),
          effect.none(),
        )
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
        True -> {
          let next = live(model, text, True)
          // A new notice also flags the Board tab while another page shows.
          let fx = case raised(model.notices, next.notices) {
            True -> effect.batch([fx, e.emit("attention", json.null())])
            False -> fx
          }
          #(hand(next, text), fx)
        }
        // A project not listed yet: held until the list names it.
        False -> #(Model(..model, replay: [text, ..model.replay]), fx)
      }
    }
    CaughtUp -> #(
      resync(settle(Model(..model, caught_up: True))),
      effect.none(),
    )
    Select(dir, id) -> #(
      Model(..model, selected_card: Some(#(dir, id)), live: []),
      effect.none(),
    )
    // The task's live update takes the card off the board.
    Archive(dir, id) -> #(
      model,
      effect.from(fn(dispatch) {
        request(
          model.prefix,
          base_of(model, dir)
            <> "/tasks/"
            <> uri.percent_encode(id)
            <> "/archive",
          "POST",
          "{}",
          fn(text) { dispatch(Archived(text)) },
        )
      }),
    )
    // An archived open task closes its details.
    Archived(text) ->
      case json.parse(text, solace_task.task_decoder()) {
        Ok(t) -> #(
          Model(..model, error: "", selected_card: case model.selected_card {
            Some(#(_, id)) if id == t.id -> None
            other -> other
          }),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    TaskMoved(dir, id) ->
      case model.selected_card {
        Some(#(_, card)) if card == id -> #(
          Model(..model, selected_card: Some(#(dir, id))),
          effect.none(),
        )
        _ -> #(model, effect.none())
      }
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
    // Only a task that was really created clears the draft; it opens.
    Created(text) -> {
      let model = Model(..model, submitting: False)
      case json.parse(text, solace_task.task_decoder()) {
        Ok(t) -> {
          store(draft_title_key, "")
          store(draft_body_key, "")
          #(
            Model(
              ..model,
              selected_card: Some(#(draft_home(model), t.id)),
              draft_scope: [],
              creating: False,
              draft_title: "",
              draft_body: "",
              previews: [],
              error: "",
            ),
            effect.none(),
          )
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    }
    Preview(id, on) -> #(
      Model(..model, previews: case on {
        True -> [id, ..model.previews]
        False -> list.filter(model.previews, fn(p) { p != id })
      }),
      effect.none(),
    )
    // The board's one editor is the new task's description.
    Format(id, before, after) -> #(
      model,
      effect.from(fn(send) {
        send(
          DraftBody(solace_task.wrap(
            model.root,
            id,
            model.draft_body,
            before,
            after,
          )),
        )
      }),
    )
    Block(id, kind) -> #(
      model,
      effect.from(fn(send) {
        send(
          DraftBody(solace_task.block(model.root, id, model.draft_body, kind)),
        )
      }),
    )
    DismissAll -> #(Model(..model, notices: []), effect.none())
    Dismiss(notice) -> #(
      Model(..model, notices: list.filter(model.notices, fn(n) { n != notice })),
      effect.none(),
    )
    // `<solace-task>` shows the notice's tab and scrolls into view, now or
    // once the task loads.
    Open(notice) -> #(
      Model(
        ..model,
        selected_card: Some(#(notice.dir, notice.id)),
        live: case model.selected_card == Some(#(notice.dir, notice.id)) {
          True -> model.live
          False -> []
        },
        reveal: #(notice.tab, notice.id, model.reveal.2 + 1),
        notices: list.filter(model.notices, fn(n) { n != notice }),
      ),
      effect.none(),
    )
    Focus(_, "") -> #(model, effect.none())
    // `<solace-task>` scrolls into view now, or once the task loads.
    Focus(dir, id) -> #(
      Model(
        ..model,
        selected_card: Some(#(dir, id)),
        live: case model.selected_card == Some(#(dir, id)) {
          True -> model.live
          False -> []
        },
        reveal: #("details", id, model.reveal.2 + 1),
      ),
      effect.none(),
    )
  }
}

/// Applies one `/events` message; a workspace's name their `project`.
fn live(model: Model, text: String, notify: Bool) -> Model {
  // A project on its own sends no `project`: it is the root's.
  let dir = event_dir(model, text)
  case lane_of(model, dir) {
    Error(_) -> model
    Ok(lane) ->
      case json.parse(text, decode.at(["task"], solace_task.task_decoder())) {
        Ok(task) -> {
          let new = case
            list.find(lane.tasks, fn(t) { t.id == task.id }),
            notify
          {
            Ok(old), True ->
              solace_task.notices(old, task)
              |> list.map(fn(n) {
                let n = solace_task.Notice(..n, dir: dir)
                case model.workspace {
                  True ->
                    solace_task.Notice(
                      ..n,
                      heading: lane.label <> ": " <> n.heading,
                    )
                  False -> n
                }
              })
            _, _ -> []
          }
          map_lane(
            Model(..model, notices: list.append(model.notices, new)),
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
          case
            json.parse(
              text,
              decode.at(["workflow"], solace_task.workflow_decoder()),
            )
          {
            Ok(w) -> map_lane(model, dir, fn(l) { Lane(..l, workflow: w) })
            Error(_) ->
              // A new connection starts from the project's tasks as they are.
              case
                json.parse(
                  text,
                  decode.at(["tasks"], decode.list(solace_task.task_decoder())),
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
    decode.success(
      Lane(
        dir,
        label,
        name,
        base,
        target,
        solace_task.Workflow([], dict.new()),
        [],
      ),
    )
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

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

pub fn view(model: Model) -> Element(Msg) {
  h.main([], [
    h.style(
      [],
      board_styles
        <> selected_card_styles
        <> modal_styles
        <> notice_styles
        <> archive_styles
        <> waiting_styles,
    ),
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
      list.append(
        case list.length(model.notices) > 5 {
          True -> [
            h.button([a.class("close-all"), e.on_click(DismissAll)], [
              h.text("Close All"),
            ]),
          ]
          False -> []
        },
        list.map(model.notices, fn(n) {
          h.div([a.class("notice"), a.role("alert")], [
            h.strong([], [h.text(n.heading)]),
            h.button([a.class("show"), e.on_click(Open(n))], [
              h.text("Show task"),
            ]),
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
              solace_task.projects_pick(
                list.map(model.lanes, fn(l) { #(l.dir, l.label) }),
                model.draft_scope,
                False,
                DraftScope,
              )
            False -> h.text("")
          },
          solace_task.title_input(model.draft_title, DraftTitle),
          solace_task.editor(
            "draft-body",
            "",
            base_of(model, draft_lane(model)),
            "Description",
            model.draft_body,
            list.contains(model.previews, "draft-body"),
            DraftBody,
            Preview("draft-body", _),
            fn(before, after) { Format("draft-body", before, after) },
            Block("draft-body", _),
          ),
          h.button([a.disabled(model.submitting), e.on_click(Create)], [
            h.text(case model.submitting {
              True -> "Creating…"
              False -> "Create task"
            }),
          ]),
        ])
    },
    lanes_view(model),
    case model.selected_card {
      None -> h.text("Select a task to see its details.")
      Some(#(dir, id)) -> task_view(model, dir, id)
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
          h.a([a.href(lane.base <> "/contracts")], [h.text("Contract view")]),
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
              h.a([a.href(lane.base <> "/contracts")], [
                h.text("Contract view"),
              ]),
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
/// opens its task from the project it is stored in. A card in a final column
/// has an Archive button in its upper right corner (D-95).
/// A card waiting on an unfinished dependency is dimmed and says what it
/// waits on; it still opens as any other.
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
          let waiting = case waiting_on(model, t) {
            [] -> ""
            titles -> "Waiting on " <> string.join(titles, ", ")
          }
          h.div([a.class("card-wrap")], [
            h.button(
              list.append(
                [
                  a.class(
                    case current {
                      True -> "card selected"
                      False -> "card"
                    }
                    <> case waiting {
                      "" -> ""
                      _ -> " waiting"
                    },
                  ),
                  a.attribute("aria-current", case current {
                    True -> "true"
                    False -> "false"
                  }),
                  e.on_click(Select(home, t.id)),
                ],
                case waiting {
                  "" -> []
                  _ -> [a.title(waiting)]
                },
              ),
              [
                case t.attention {
                  "" -> h.text("")
                  _ ->
                    h.strong([a.class("attention")], [
                      h.text("Needs attention "),
                    ])
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
                case waiting {
                  "" -> h.text("")
                  _ -> h.small([a.class("waiting-on")], [h.text(waiting)])
                },
                solace_task.status_badge_in(lane.workflow, t),
              ],
            ),
            case c.final {
              True ->
                h.button(
                  [
                    a.class("archive"),
                    a.attribute("aria-label", "Archive " <> t.title),
                    a.title("Remove from the board; it stays in the archive"),
                    e.on_click(Archive(home, t.id)),
                  ],
                  [h.text("Archive")],
                )
              False -> h.text("")
            },
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

fn task_count(cards: List(#(String, solace_task.Task))) -> Element(Msg) {
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
fn scope_of(home: String, t: solace_task.Task) -> List(String) {
  case t.scope {
    [] -> [home]
    scope -> scope
  }
}

/// The tasks a lane shows, each with the project it is stored in: every task
/// on the board whose projects include this lane's (D-61), but not an archived
/// one (D-95).
fn cards(model: Model, lane: Lane) -> List(#(String, solace_task.Task)) {
  list.flat_map(model.lanes, fn(l) {
    l.tasks
    |> list.filter(fn(t) {
      !t.archived && list.contains(scope_of(l.dir, t), lane.dir)
    })
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

pub type Source

@external(javascript, "./api.mjs", "close_events")
fn close_events(source: Source) -> Nil

@external(javascript, "./api.mjs", "prevent_drag")
fn prevent_drag(root: dynamic.Dynamic) -> Nil

pub fn register() -> Result(Nil, lustre.Error) {
  // The open task's details are `<solace-task>`, registered once per page.
  let _ = solace_task.register()
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("base", fn(base) { Ok(BaseChanged(base)) }),
      // The shell's `focus` property, `{dir, id, n}`: `n` makes asking again news.
      component.on_property_change("focus", {
        use dir <- decode.field("dir", decode.string)
        use id <- decode.field("id", decode.string)
        decode.success(Focus(dir, id))
      }),
      component.on_disconnect(Disconnected),
      component.on_connect(Connected),
      component.adopt_styles(False),
    ]),
    "solace-board",
  )
}

const board_styles =
  ".projects-pick{margin:8px 0}.projects-pick label{display:flex;align-items:center;gap:10px;margin:6px 0;cursor:pointer}.projects-pick input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}.projects-pick input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}.projects-pick input:checked{background:var(--focus)}.projects-pick input:checked::before{transform:translateX(14px)}.projects-pick input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}.projects-pick[disabled] label,.projects-pick[disabled] input{opacity:.5;cursor:default}@media (prefers-reduced-motion:reduce){.projects-pick input,.projects-pick input::before{transition:none}}\n\n:host{display:block;--well:#eceee6;--hover:#e7eeea;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--edge:var(--ink);--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a;--focus:#174f78;box-sizing:border-box;padding:0 24px 48px;min-height:100dvh;user-select:none;-webkit-user-select:none}input,textarea,pre,code,.md,.view-code,.detail > h2,.detail > .panel > article > p,.detail > .panel > p,.detail > .panel > ul > li,.changes h4,.changes li,.run > header > strong,.run > header > .state,.run > small{user-select:text;-webkit-user-select:text}\nmain{font:16px system-ui,sans-serif;color:var(--ink);background:var(--sheet)}button,input,textarea,select{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:0}button{cursor:pointer;background:var(--paper)}button:hover{background:var(--hover)}button:focus-visible{outline:3px solid var(--focus)}label{display:block}textarea{display:block;width:min(95%,700px);min-height:70px}input{width:min(80%,600px)}.board{display:flex;gap:12px;overflow-x:auto;margin:24px 0}.column{min-width:190px;flex:1;background:var(--well);padding:12px;border-radius:0}.column h2{font-size:17px}.card{display:block;width:100%;text-align:left}.detail,.editor{border:1px solid var(--line);padding:16px;border-radius:0}small{display:block;color:var(--soft);overflow-wrap:anywhere}pre{white-space:pre-wrap;font:inherit}.actions{display:flex;flex-wrap:wrap;gap:6px}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}.note,.run{border:1px solid var(--line);border-radius:0;padding:8px 12px;margin:8px 0;background:var(--paper)}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.md p,.md ul,.md ol,.md blockquote{margin:6px 0}.md blockquote{padding-left:10px;border-left:3px solid var(--line);color:var(--soft)}.md pre,.transcript pre{background:var(--code);padding:8px;border-radius:0;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace;max-height:320px}code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:0}.transcript{border-left:3px solid var(--line);padding-left:10px;margin-top:8px}.entry{margin:6px 0}details.entry>summary{cursor:pointer;color:var(--soft)}.tool .icon{display:inline-block;width:1.2em}.tool.ok .icon{color:var(--ok-text)}.tool.failed .icon{color:var(--revision)}.muted{color:var(--soft)}.note .delete{margin-left:auto;font-size:13px;padding:2px 8px}\n.top{align-items:center;gap:16px}.top h1{margin:0}.top .primary{margin-left:auto}.primary{background:var(--focus);color:var(--paper);border-color:var(--focus)}.primary:hover{background:var(--focus)}.column{min-height:240px}.card+.card{margin-top:6px}\n.meta{display:flex;gap:10px;align-items:center;color:var(--soft);margin:4px 0 12px}.agent{display:flex;flex-wrap:wrap;gap:4px 16px}.agent label{display:flex;flex-direction:column}\n.composer{border:1px solid var(--edge);border-radius:0;margin:8px 4px;max-width:760px;background:var(--paper)}.composer textarea{width:100%;max-width:none;box-sizing:border-box;border:0;border-top:1px solid var(--line);border-radius:0 0 6px 6px;margin:0;min-height:110px}.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:2px;padding:4px}.toolbar button,.toolbar .button{margin:0;padding:4px 10px;font-size:14px}.sr-only{position:absolute;width:1px;height:1px;opacity:0;overflow:hidden}.toolbar .button{position:relative;display:inline-block;border:1px solid var(--edge);border-radius:0;background:var(--paper);cursor:pointer}.toolbar .button:hover{background:var(--hover)}.toolbar .button:focus-within{outline:3px solid var(--focus)}.toolbar [aria-pressed=true]{background:var(--well);font-weight:600}.separator{width:1px;align-self:stretch;background:var(--line);margin:0 6px}.preview{border-top:1px solid var(--line);padding:8px 12px;min-height:94px}\n.tabs{display:flex;flex-wrap:wrap;gap:0;border-bottom:1px solid var(--line);margin-top:16px}.tabs button{margin:0 0 -1px;border-radius:0 0 0;border-color:transparent;background:transparent}.tabs [aria-selected=true]{border-color:var(--line) var(--line) var(--sheet);background:var(--sheet);font-weight:600}.panel{padding-top:8px}.changes h4{font-size:15px;margin:12px 0 4px}.changes ul{margin:0;padding-left:20px}.changes li{white-space:normal;margin:2px 0;font:13px 'IBM Plex Mono',ui-monospace,monospace}\ndetails.note>summary{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap;cursor:pointer;list-style:none}details.note>summary::-webkit-details-marker{display:none}details.note>summary::before{content:\"▸\";color:var(--soft);width:1em}details.note[open]>summary::before{content:\"▾\"}details.note[open]>summary{margin-bottom:4px}.md img{max-width:100%;height:auto;border:1px solid var(--line);border-radius:0}.status.finished{background:var(--add);color:var(--ok-text)}.diff{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:8px;border-radius:0;max-width:760px}\n.lanes{display:grid;gap:0;margin:24px 0;overflow-x:auto;padding-bottom:4px}.lanes .board{margin:0;overflow:visible}.lane .board{margin:8px 0}.lane{border:1px solid var(--line);border-radius:0;padding:4px 8px;background:var(--paper)}.lane .column{min-height:110px}.lane-title{align-items:center;gap:12px}.lane-title small{display:inline}.lane-title button{padding:4px 10px}.lane-toggle{border:0;background:transparent;font-size:17px}.lane-head{margin:0;padding:0 9px}.lane-head .column{min-height:0;padding:0 12px;background:transparent}.lane-head h2{margin:6px 0}\n.projects{margin-top:16px}.projects h2{margin-top:0}.projects h3{font-size:15px;margin:16px 0 4px}.projects ul{list-style:none;padding:0;margin:0}.projects li{display:flex;align-items:center;gap:6px;white-space:normal;margin:0}.projects li button{padding:4px 10px}.new-project{display:flex;flex-wrap:wrap;align-items:flex-end;gap:4px}.new-project input{width:auto;flex:1;min-width:220px}.new-project label{display:flex;flex-direction:column}\n.title{display:block;box-sizing:border-box;width:calc(100% - 8px);max-width:762px;margin:8px 4px}.stamp{color:var(--ok-text);font-weight:700;margin-right:6px}.hazard{display:inline-block;width:18px;height:18px;margin-right:6px;vertical-align:-3px}.hazard svg{width:100%;height:100%}.hazard path{fill:#f2c94c;stroke:#7a5200;stroke-width:1.5;stroke-linejoin:round}.hazard path.mark{fill:none;stroke:#3d2900;stroke-width:2.5;stroke-linecap:round}.notices{position:fixed;right:16px;bottom:16px;z-index:10;display:grid;gap:8px;max-width:min(380px,calc(100% - 32px))}.notices .notice{position:relative;background:var(--ink);color:var(--paper);padding:12px 52px 12px 14px;border-radius:0;box-shadow:0 4px 16px var(--shadow)}.notice strong{display:block;overflow-wrap:anywhere}.notice .close{position:absolute;top:6px;right:6px;margin:0;padding:6px;line-height:0;background:transparent;color:var(--paper);border:0}.notice .close:hover{background:color-mix(in srgb,var(--paper) 20%,transparent)}.notice .close svg{width:24px;height:24px;fill:none;stroke:currentColor;stroke-width:2.5;stroke-linecap:round}
:host{background:var(--sheet);color:var(--ink)}
main{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;color:var(--ink);background:none}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button,input,textarea,select{border:1px solid var(--edge);background:var(--paper);color:var(--ink);caret-color:var(--focus)}input::placeholder,textarea::placeholder{color:var(--soft);opacity:1}
button:hover{background:var(--hover)}button:disabled{color:var(--soft)}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
textarea{font-family:inherit}
.top{margin:0 -24px 24px;padding:12px 22px;background:var(--paper);border-bottom:2px solid var(--edge)}.top h1{font-size:28px}
.primary{background:var(--ink);color:var(--paper);border-color:var(--edge);font-weight:700}.primary:hover{background:var(--focus);border-color:var(--focus);color:var(--paper)}
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

/// Opens this instance's transport with its own `base`: the event stream and
/// the project list. Runs on connection and when `base` changes; the fresh
/// stream replays what it missed.
fn start(model: Model) -> #(Model, Effect(Msg)) {
  let model = Model(..stop(model), caught_up: False, replay: [])
  #(
    model,
    effect.batch([
      get(model, "/projects", ProjectsLoaded),
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

/// Closes the event stream.
fn stop(model: Model) -> Model {
  case model.source {
    Some(source) -> close_events(source)
    None -> Nil
  }
  Model(..model, source: None)
}

@external(javascript, "./api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

const selected_card_styles = "
button.card[aria-current=true],button.card[aria-current=true]:hover{background:var(--hover);border-color:var(--ink);box-shadow:0 0 0 2px var(--ink),4px 6px 14px var(--shadow)}button.card[aria-current=true]:focus-visible{outline:3px solid var(--focus);outline-offset:5px}@media(forced-colors:active){button.card[aria-current=true],button.card[aria-current=true]:hover{border-color:Highlight;box-shadow:0 0 0 2px Highlight}}"

/// The open task left its home project: if another project holds it now (it
/// moved to another project it covers, D-61), it is opened from there.
fn follow_move(model: Model, dir: String, id: String) -> Model {
  case model.selected_card {
    Some(#(home, card)) if home == dir && card == id ->
      case
        list.find(model.lanes, fn(l) {
          l.dir != dir && list.any(l.tasks, fn(t) { t.id == id })
        })
      {
        Ok(l) -> Model(..model, selected_card: Some(#(l.dir, id)))
        Error(_) -> model
      }
    _ -> model
  }
}

/// The column names as one bar, shared by lanes with one workflow or one per
/// lane when they differ. It is the first thing in its container, so it sticks
/// to the top of the page until the last lane in that container scrolls past.
fn head_view(columns: List(solace_task.Column)) -> Element(Msg) {
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
        fn(m, text) { hand(live(m, text, False), text) },
      )
    }
  }
}

const notice_styles =
  ".notice .show{margin:8px 0 0;padding:4px 10px;background:transparent;color:var(--paper);border-color:var(--paper)}.notice .show:hover{background:color-mix(in srgb,var(--paper) 20%,transparent)}.notice .show:focus-visible{outline-color:var(--paper)}.notices .close-all{justify-self:end;margin:0;padding:4px 10px;background:var(--ink);color:var(--paper);border-color:var(--paper)}.notices .close-all:hover{background:color-mix(in srgb,var(--paper) 20%,var(--ink))}.notices .close-all:focus-visible{outline-color:var(--paper)}"

/// Whether a live message is for the open task's details: its own update
/// under any project (`<solace-task>` fetches itself again when one names
/// another than its home, D-61), the update of a task it depends on, for its
/// Depends on line, its project's workflow, or a change to the list of
/// projects, which its Scope tab shows. Its removal is the board's to follow.
fn for_detail(model: Model, text: String) -> Bool {
  case model.selected_card {
    None -> False
    Some(#(home, id)) ->
      case json.parse(text, decode.at(["task", "id"], decode.string)) {
        Ok(task) -> task == id || list.contains(depends_of(model, id), task)
        Error(_) ->
          projects_event(text)
          || event_dir(model, text) == home
          && json.parse(text, decode.at(["workflow"], decode.dynamic))
          |> result.is_ok
      }
  }
}

/// The API prefix the open task's project answers at: a workspace project's
/// own `/p/<dir>`, else this board's.
fn task_base(model: Model, dir: String) -> String {
  let base = base_of(model, dir)
  case string.starts_with(base, "/p/") {
    True -> base
    False -> model.prefix <> base
  }
}

/// The open task's details: `<solace-task>` from the components project,
/// which loads and changes the task itself. The board hands it the task's
/// events (`live`) and a notice's request to show it (`reveal`), and follows
/// it when it moves to another of its projects.
fn task_view(model: Model, dir: String, id: String) -> Element(Msg) {
  let #(tab, shown, asked) = model.reveal
  element.element(
    "solace-task",
    [
      a.attribute("base", task_base(model, dir)),
      a.attribute("dir", dir),
      a.attribute("task", id),
      a.attribute("hosted", ""),
      case model.workspace {
        True -> a.attribute("workspace", "")
        False -> a.none()
      },
      a.property(
        "live",
        json.array(model.live, fn(m) {
          json.object([#("n", json.int(m.0)), #("text", json.string(m.1))])
        }),
      ),
      a.property(
        "reveal",
        json.object([
          #("tab", json.string(tab)),
          #("id", json.string(shown)),
          #("n", json.int(asked)),
        ]),
      ),
      e.on("task-moved", {
        use dir <- decode.subfield(["detail", "dir"], decode.string)
        use id <- decode.subfield(["detail", "id"], decode.string)
        decode.success(TaskMoved(dir, id))
      }),
    ],
    [],
  )
}

/// Adds a live message to those handed to the open task's details, if it is
/// for them; only the last few are kept.
fn hand(model: Model, text: String) -> Model {
  case for_detail(model, text) {
    False -> model
    True -> push(model, text)
  }
}

const archive_styles =
  ".card-wrap{position:relative}.card-wrap+.card-wrap{margin-top:8px}.card-wrap:has(.archive) .card{padding-right:84px}.card-wrap .archive{position:absolute;top:6px;right:6px;margin:0;padding:2px 8px;font-size:12px}"

fn push(model: Model, text: String) -> Model {
  let n = model.live_n + 1
  Model(
    ..model,
    live_n: n,
    live: list.append(model.live, [#(n, text)])
      |> list.reverse
      |> list.take(50)
      |> list.reverse,
  )
}

/// Once the stream has (re)connected and caught up, the open task's details
/// fetch the task again: an event lost meanwhile can't leave them stale.
fn resync(model: Model) -> Model {
  case model.selected_card {
    None -> model
    Some(_) -> push(model, solace_task.resync)
  }
}

/// The titles of the tasks `t` depends on that are unfinished: not in a final
/// column of their lane's workflow now (D-87). The lanes hold every task of
/// the root, so one in another project is found too; an id in no lane is left
/// out, since its title is unknown.
fn waiting_on(model: Model, t: solace_task.Task) -> List(String) {
  list.filter_map(t.depends, fn(id) {
    use #(lane, dep) <- result.try(
      list.find_map(model.lanes, fn(l) {
        list.find(l.tasks, fn(d) { d.id == id })
        |> result.map(fn(d) { #(l, d) })
      }),
    )
    case solace_task.finished(lane.workflow, dep.column) {
      True -> Error(Nil)
      False -> Ok(dep.title)
    }
  })
}

const waiting_styles =
  ".card.waiting{opacity:.55}.card.waiting:hover,.card.waiting[aria-current=true],.card.waiting:focus-visible{opacity:.85}.card .waiting-on{font-size:13px}"

/// Whether a live update raised a notice: the host hears `attention`, which
/// flags the Board tab while another page shows. `live` only adds notices.
pub fn raised(
  old: List(solace_task.Notice),
  new: List(solace_task.Notice),
) -> Bool {
  list.length(new) > list.length(old)
}

/// The ids of the tasks task `id` depends on, as the lanes hold it.
fn depends_of(model: Model, id: String) -> List(String) {
  list.find_map(model.lanes, fn(l) { list.find(l.tasks, fn(t) { t.id == id }) })
  |> result.map(fn(t) { t.depends })
  |> result.unwrap([])
}
