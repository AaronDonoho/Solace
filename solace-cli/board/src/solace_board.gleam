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
    notices: List(solace_task.Notice),
    caught_up: Bool,
    replay: List(String),
    collapsed: List(String),
    /// Whether the detail band is collapsed to its one-line bar.
    detail_collapsed: Bool,
    /// The detail band's task height in pixels; 0 is the CSS default.
    detail_height: Int,
    /// A drag of the resize handle in progress: the pointer's start, the
    /// band's height then, and the largest height that keeps the board's
    /// column bar on screen.
    resizing: Option(#(Float, Float, Int)),
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
    /// The root's features as (id, title, colorSlot, state), which name and
    /// color their tasks' cards (D-114) and decide whether they show (D-128).
    features: List(#(String, String, Option(Int), String)),
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
  /// Collapse the detail band to its bar, or expand it back.
  ToggleDetail
  /// The resize handle's pointerdown: the pointer's y and id.
  ResizeStart(Float, Int)
  /// A drag has begun: the pointer's y, the band's height then, and the
  /// largest height that keeps the board's column bar on screen.
  ResizeBegin(Float, Float, Int)
  /// The handle's pointermove: the pointer's y.
  ResizeMove(Float)
  /// The drag ends: the height is kept.
  ResizeEnd
  /// The handle's keydown: the key's name.
  ResizeKey(String)
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

const collapsed_key = "solace-board-collapsed"

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
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
      notices: [],
      caught_up: False,
      replay: [],
      collapsed: string.split(load(collapsed_key), "\n")
        |> list.filter(fn(d) { d != "" }),
      detail_collapsed: load(detail_key) == "1",
      detail_height: load(detail_height_key) |> int.parse |> result.unwrap(0),
      resizing: None,
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
      features: [],
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
      expanded(Model(..model, selected_card: Some(#(dir, id)), live: [])),
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
    ToggleDetail -> {
      let collapsed = !model.detail_collapsed
      store(detail_key, case collapsed {
        True -> "1"
        False -> "0"
      })
      #(Model(..model, detail_collapsed: collapsed), effect.none())
    }
    // The handle's drag: the browser captures the pointer on the handle, so
    // its moves keep arriving as the pointer travels over the page, and
    // answers the band's height now and the largest height that leaves the
    // board its column bar.
    ResizeStart(y, pointer_id) ->
      case model.selected_card, model.detail_collapsed {
        Some(_), False -> #(
          model,
          effect.from(fn(dispatch) {
            let #(start_y, height, max) =
              begin_resize(model.root, pointer_id, y)
            dispatch(ResizeBegin(start_y, height, max))
          }),
        )
        _, _ -> #(model, effect.none())
      }
    ResizeBegin(y, height, max) -> #(
      Model(..model, resizing: Some(#(y, height, max))),
      effect.none(),
    )
    // Dragging up (a smaller y) grows the band; the height is clamped so
    // the board always keeps its column bar.
    ResizeMove(y) ->
      case model.resizing {
        Some(#(start_y, start_h, max)) -> #(
          Model(
            ..model,
            detail_height: clamp_height(
              resize_height(start_y, start_h, y, max),
              max,
            ),
          ),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    ResizeEnd ->
      case model.resizing {
        Some(_) -> #(
          Model(..model, resizing: None),
          effect.from(fn(_) {
            store(detail_height_key, int.to_string(model.detail_height))
          }),
        )
        None -> #(model, effect.none())
      }
    // The handle's keys: the arrows step the height, Home returns the
    // default.
    ResizeKey(key) ->
      case key {
        "ArrowUp" | "ArrowDown" | "Home" -> {
          // The default height is the CSS's: a step starts from what shows.
          let now = case model.detail_height {
            0 -> detail_now(model.root)
            h -> h
          }
          let height = case key {
            "ArrowUp" -> clamp_height(now + 24, detail_max())
            "ArrowDown" -> clamp_height(now - 24, detail_max())
            _ -> 0
          }
          #(
            Model(..model, detail_height: height),
            effect.from(fn(_) {
              store(detail_height_key, int.to_string(height))
            }),
          )
        }
        _ -> #(model, effect.none())
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
    DismissAll -> #(Model(..model, notices: []), effect.none())
    Dismiss(notice) -> #(
      Model(..model, notices: list.filter(model.notices, fn(n) { n != notice })),
      effect.none(),
    )
    // `<solace-task>` shows the notice's tab and scrolls into view, now or
    // once the task loads.
    Open(notice) -> #(
      expanded(
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
      ),
      effect.none(),
    )
    Focus(_, "") -> #(model, effect.none())
    // `<solace-task>` scrolls into view now, or once the task loads.
    Focus(dir, id) -> #(
      expanded(
        Model(
          ..model,
          selected_card: Some(#(dir, id)),
          live: case model.selected_card == Some(#(dir, id)) {
            True -> model.live
            False -> []
          },
          reveal: #("details", id, model.reveal.2 + 1),
        ),
      ),
      effect.none(),
    )
  }
}

/// Applies one `/events` message; a workspace's name their `project`.
fn live(model: Model, text: String, notify: Bool) -> Model {
  case no_project(text) {
    True -> model
    False ->
      case features_event(model, text) {
        Ok(model) -> model
        Error(_) -> task_event(model, text, notify)
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
        <> detail_band_styles
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
        [h.text(solace_task.ui_label("projects"))],
      ),
    ]),
    // The error line shows only with an error: an empty paragraph in the
    // page's flex column takes a line between the header and the lanes.
    case model.error {
      "" -> h.text("")
      _ -> h.p([a.role("alert")], [h.text(model.error)])
    },
    h.div(
      [a.class("notices")],
      list.append(
        case list.length(model.notices) > 5 {
          True -> [
            h.button([a.class("close-all"), e.on_click(DismissAll)], [
              h.text(solace_task.ui_label("close_all")),
            ]),
          ]
          False -> []
        },
        list.map(model.notices, fn(n) {
          h.div([a.class("notice"), a.role("alert")], [
            h.strong([], [h.text(n.heading)]),
            h.button([a.class("show"), e.on_click(Open(n))], [
              h.text(solace_task.ui_label("show_task")),
            ]),
            h.button(
              [
                a.class("close"),
                a.attribute("aria-label", solace_task.ui_label("dismiss")),
                a.title(solace_task.ui_label("dismiss")),
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
    h.div([a.class("lanes-area")], [lanes_view(model)]),
    h.div(
      [
        a.class(case model.detail_collapsed, model.resizing {
          True, _ -> "detail-band collapsed"
          False, Some(_) -> "detail-band resizing"
          False, None -> "detail-band"
        }),
      ],
      [
        detail_resize(model),
        detail_bar(model),
        case model.selected_card, model.detail_collapsed {
          // The task element stays in the band when it is collapsed, hidden
          // by its CSS, so its loaded task and live events survive.
          None, True -> h.text("")
          None, False -> h.text("Select a task to see its details.")
          Some(#(dir, id)), _ -> task_view(model, dir, id)
        },
      ],
    ),
  ])
}

/// Whether every lane has the same columns, so one header row serves all.
pub fn shared_columns(lanes: List(Lane)) -> Bool {
  case lanes {
    [first, ..rest] -> list.all(rest, fn(l) { l.workflow == first.workflow })
    [] -> True
  }
}

/// One lane per project. A single project's header has no toggle. Tasks are
/// created on the Strategy page, not here. Lanes with the same workflow
/// share one header row; otherwise each lane names its own columns. A header
/// row is a bar that sticks to the top of the lanes area, the board page's
/// scroll area, while its lanes are on screen.
fn lanes_view(model: Model) -> Element(Msg) {
  let shared = model.workspace && shared_columns(model.lanes)
  case model.workspace, model.lanes {
    False, [lane] ->
      h.div([], [
        h.header([a.class("lane-title")], [
          h.span([], lane_name(lane)),
          task_count(cards(model, lane)),
          h.a([a.href(lane.base <> "/contracts")], [
            h.text(solace_task.ui_label("contract_view")),
          ]),
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
          True, [first, ..] -> head_view(board_columns(first.workflow))
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
              h.a([a.href(lane.base <> "/contracts")], [
                h.text(solace_task.ui_label("contract_view")),
              ]),
            ]),
            ..case open, shared {
              True, True -> [columns_view(model, lane, False)]
              True, False -> [
                head_view(board_columns(lane.workflow)),
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
/// A card's stripe is its feature's color, gray with no feature or one not
/// started yet; its tooltip and screen-reader text name the feature, so color
/// is not its only sign (D-114).
fn columns_view(model: Model, lane: Lane, labels: Bool) -> Element(Msg) {
  let colors =
    solace_task.feature_colors(list.map(model.features, fn(f) { #(f.0, f.2) }))
  let names = list.map(model.features, fn(f) { #(f.0, f.1) })
  h.div(
    [a.class("board")],
    list.map(board_columns(lane.workflow), fn(c) {
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
          let color = case t.feature {
            Some(f) -> list.key_find(colors, f) |> result.unwrap("")
            None -> ""
          }
          let feature = case t.feature {
            Some(f) ->
              case list.key_find(names, f) {
                Ok(name) if name != "" -> "Feature: " <> name
                _ -> ""
              }
            None -> ""
          }
          let tip =
            [waiting, feature]
            |> list.filter(fn(s) { s != "" })
            |> string.join("\n")
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
                list.append(
                  case tip {
                    "" -> []
                    _ -> [a.title(tip)]
                  },
                  case color {
                    "" -> []
                    _ -> [a.style("--tab", color)]
                  },
                ),
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
                case t.tech_debt {
                  False -> h.text("")
                  True -> debt_mark()
                },
                h.text(t.title),
                case feature {
                  "" -> h.text("")
                  _ -> h.span([a.class("sr-only")], [h.text(" · " <> feature)])
                },
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
                    a.attribute(
                      "aria-label",
                      solace_task.ui_label("archive") <> " " <> t.title,
                    ),
                    a.title("Remove from the board; it stays in the archive"),
                    e.on_click(Archive(home, t.id)),
                  ],
                  [h.text(solace_task.ui_label("archive"))],
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
    h.ul(
      [],
      list.map(model.lanes, fn(l) {
        h.li([], [
          h.strong([], [h.text(l.label)]),
          h.text(" " <> l.name <> " "),
          // The root's own project always stays on its board.
          case l.dir {
            "." -> h.small([], [h.text("(this folder)")])
            dir -> button(solace_task.ui_label("remove"), RemoveProject(dir))
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
                  True -> button(solace_task.ui_label("add"), AddProject(f.dir))
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
      button(solace_task.ui_label("create_project"), CreateProject),
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
/// one (D-95), nor one in Backlog, which means not active (D-133), nor one
/// whose feature is proposed, upcoming or canceled: its tasks show once it is
/// active, and stay when it completes (D-128). A task whose feature isn't
/// known (not loaded yet) shows.
fn cards(model: Model, lane: Lane) -> List(#(String, solace_task.Task)) {
  let waiting =
    model.features
    |> list.filter(fn(f) { f.3 != "active" && f.3 != "complete" })
    |> list.map(fn(f) { Some(f.0) })
  list.flat_map(model.lanes, fn(l) {
    l.tasks
    |> list.filter(fn(t) {
      !t.archived
      && t.column != backlog
      && list.contains(scope_of(l.dir, t), lane.dir)
      && !list.contains(waiting, t.feature)
    })
    |> list.map(fn(t) { #(l.dir, t) })
  })
}

fn label_of(model: Model, dir: String) -> String {
  lane_of(model, dir) |> result.map(fn(l) { l.label }) |> result.unwrap(dir)
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
  solace_task.control_styles
  <> ":host{display:block;--well:#eceee6;--hover:#e7eeea;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--edge:var(--ink);--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a;--focus:#174f78;box-sizing:border-box;padding:0 24px 0;height:calc(100dvh - 56px);user-select:none;-webkit-user-select:none}input,textarea,pre,code,.md,.view-code,.detail > h2,.detail > .panel > article > p,.detail > .panel > p,.detail > .panel > ul > li,.changes h4,.changes li,.run > header > strong,.run > header > .state,.run > small{user-select:text;-webkit-user-select:text}\nmain{font:16px system-ui,sans-serif;color:var(--ink);background:var(--sheet)}button,input,textarea,select{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:0}button{cursor:pointer}label{display:block}textarea{display:block;width:100%;box-sizing:border-box;min-height:70px}input{width:100%;box-sizing:border-box}.board{display:flex;gap:12px;overflow-x:auto;margin:24px 0}.column{min-width:190px;flex:1;background:var(--well);padding:12px;border-radius:0}.column h2{font-size:17px}.card{display:block;width:100%;text-align:left}.detail,.editor{border:1px solid var(--line);padding:16px;border-radius:0}small{display:block;color:var(--soft);overflow-wrap:anywhere}pre{white-space:pre-wrap;font:inherit}.actions{display:flex;flex-wrap:wrap;gap:6px}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}.note,.run{border:1px solid var(--line);border-radius:0;padding:8px 12px;margin:8px 0;background:var(--paper)}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.md p,.md ul,.md ol,.md blockquote{margin:6px 0}.md blockquote{padding-left:10px;border-left:3px solid var(--line);color:var(--soft)}.md pre,.transcript pre{background:var(--code);padding:8px;border-radius:0;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace;max-height:320px}code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:0}.transcript{border-left:3px solid var(--line);padding-left:10px;margin-top:8px}.entry{margin:6px 0}details.entry>summary{cursor:pointer;color:var(--soft)}.tool .icon{display:inline-block;width:1.2em}.tool.ok .icon{color:var(--ok-text)}.tool.failed .icon{color:var(--revision)}.muted{color:var(--soft)}.note .delete{margin-left:auto;font-size:13px;padding:2px 8px}\n.top{align-items:center;gap:16px}.top h1{margin:0}.top .primary{margin-left:auto}.primary{background:var(--focus);color:var(--paper);border-color:var(--focus)}.primary:hover{background:var(--focus)}.column{min-height:240px}.card+.card{margin-top:6px}\n.meta{display:flex;gap:10px;align-items:center;color:var(--soft);margin:4px 0 12px}.agent{display:flex;flex-wrap:wrap;gap:4px 16px}.agent label{display:flex;flex-direction:column}\n.tabs{display:flex;flex-wrap:wrap;gap:0;border-bottom:1px solid var(--line);margin-top:16px}.tabs button{margin:0 0 -1px;border-radius:0 0 0;border-color:transparent;background:transparent}.tabs [aria-selected=true]{border-color:var(--line) var(--line) var(--sheet);background:var(--sheet);font-weight:600}.panel{padding-top:8px}.changes h4{font-size:15px;margin:12px 0 4px}.changes ul{margin:0;padding-left:20px}.changes li{white-space:normal;margin:2px 0;font:13px 'IBM Plex Mono',ui-monospace,monospace}\ndetails.note>summary{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap;cursor:pointer;list-style:none}details.note>summary::-webkit-details-marker{display:none}details.note>summary::before{content:\"▸\";color:var(--soft);width:1em}details.note[open]>summary::before{content:\"▾\"}details.note[open]>summary{margin-bottom:4px}.md img{max-width:100%;height:auto;border:1px solid var(--line);border-radius:0}.status.finished{background:var(--add);color:var(--ok-text)}.diff{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:8px;border-radius:0;max-width:760px}\n.lanes{display:grid;gap:0;margin:24px 0;overflow-x:auto;padding-bottom:4px}.lanes .board{margin:0;overflow:visible}.lane .board{margin:8px 0}.lane{border:1px solid var(--line);border-radius:0;padding:4px 8px;background:var(--paper)}.lane .column{min-height:110px}.lane-title{align-items:center;gap:12px}.lane-title small{display:inline}.lane-title button{padding:4px 10px}.lane-toggle{border:0;background:transparent;font-size:17px}.lane-head{margin:0;padding:0 9px}.lane-head .column{min-height:0;padding:0 12px;background:transparent}.lane-head h2{margin:6px 0}\n.projects{margin-top:16px}.projects h3{font-size:15px;margin:16px 0 4px}.projects ul{list-style:none;padding:0;margin:0}.projects li{display:flex;align-items:center;gap:6px;white-space:normal;margin:0}.projects li button{padding:4px 10px}.new-project{display:flex;flex-wrap:wrap;align-items:flex-end;gap:4px}.new-project input{flex:1;min-width:220px}.new-project label{display:flex;flex-direction:column}\n.stamp{color:var(--ok-text);font-weight:700;margin-right:6px}.hazard{display:inline-block;width:18px;height:18px;margin-right:6px;vertical-align:-3px}.hazard svg{width:100%;height:100%}.hazard path{fill:#f2c94c;stroke:#7a5200;stroke-width:1.5;stroke-linejoin:round}.hazard path.mark{fill:none;stroke:#3d2900;stroke-width:2.5;stroke-linecap:round}.notices{position:fixed;right:16px;bottom:16px;z-index:10;display:grid;gap:8px;max-width:min(380px,calc(100% - 32px))}.notices .notice{position:relative;background:var(--ink);color:var(--paper);padding:12px 52px 12px 14px;border-radius:0;box-shadow:0 4px 16px var(--shadow)}.notice strong{display:block;overflow-wrap:anywhere}.notice .close{position:absolute;top:6px;right:6px;margin:0;padding:6px;line-height:0;background:transparent;color:var(--paper);border:0}.notice .close:hover{background:color-mix(in srgb,var(--paper) 20%,transparent)}.notice .close svg{width:24px;height:24px;fill:none;stroke:currentColor;stroke-width:2.5;stroke-linecap:round}
:host{background:var(--sheet);color:var(--ink)}
main{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;color:var(--ink);background:none}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button:disabled{color:var(--soft)}a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
textarea{font-family:inherit}
.top{margin:0 -24px 24px;padding:12px 22px;background:var(--paper);border-bottom:2px solid var(--edge)}.top h1{font-size:28px}
.primary{background:var(--ink);color:var(--paper);border-color:var(--edge);font-weight:700}.primary:hover{background:var(--focus);border-color:var(--focus);color:var(--paper)}
.column{background:var(--well);border:1px solid var(--line);padding:0 12px 12px}
.column h2{font-size:13px;text-transform:uppercase;letter-spacing:.1em;margin:0 -12px 12px;padding:10px 12px;background:var(--paper);border-bottom:2px solid var(--edge)}
.lanes{display:block;overflow:visible}.lane-head{position:sticky;top:0;z-index:5;background:var(--paper);border-bottom:2px solid var(--edge)}.lane-head .column{background:transparent;border:0}.lane-head h2{margin:0 -12px;border-bottom:0}.lane .lane-head{margin:0;padding:0}
.lane{border-color:var(--edge);background:var(--paper)}.lane-title{padding:6px 0}.lane-toggle{font-weight:700;background:transparent}.lane-toggle:hover{background:var(--hover)}
.card{position:relative;background:var(--paper);border-color:var(--edge);padding:10px 12px 10px 22px;line-height:1.35;--tab:#999999}
.card::before{content:\"\";position:absolute;inset:0 auto 0 0;width:8px;background:var(--tab)}
.card:hover{background:var(--hover)}.card+.card{margin-top:8px}
.attention{font-weight:700}.stamp{margin-right:6px}
.debt{display:inline-block;font-size:11px;text-transform:uppercase;letter-spacing:.08em;border:1px solid var(--test);color:var(--test);padding:0 6px;margin-right:6px}.card:has(.debt){--tab:var(--test)}
.status{border:1px solid var(--line);background:var(--paper);font-size:11px;text-transform:uppercase;letter-spacing:.08em;padding:1px 8px}.status.finished{background:var(--add);color:var(--ok-text);border-color:var(--ok-text)}.working::before{background:var(--entry)}
.detail,.editor,.note,.run,.projects{background:var(--paper);border-color:var(--edge)}.detail{padding:20px 24px}
.tabs{border-bottom:2px solid var(--edge)}.tabs button{border:0;border-right:1px solid var(--line);background:transparent;margin:0;padding:10px 16px}.tabs [aria-selected=true]{background:var(--paper);box-shadow:inset 0 -3px var(--ink);border-color:var(--line)}
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
        button(solace_task.ui_label("remove_only"), RemoveOnly),
        button("Remove and delete files…", RemoveFiles),
        button(solace_task.ui_label("cancel"), RemoveCancel),
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
          h.text(solace_task.ui_label("delete_permanently")),
        ]),
        button(solace_task.ui_label("cancel"), RemoveCancel),
      ])
  }
}

const modal_styles =
  ".modal-backdrop{position:fixed;inset:0;z-index:20;display:flex;align-items:center;justify-content:center;background:var(--shadow);padding:16px}.removal{background:var(--paper);color:var(--ink);border:2px solid var(--edge);box-shadow:0 8px 32px var(--shadow);padding:20px 24px;width:min(480px,100%);max-height:100%;overflow:auto}.removal p{margin:0 0 12px;overflow-wrap:anywhere}.removal input{margin:0}.removal button:disabled{opacity:.5;cursor:not-allowed}"

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
/// the list of projects or about the root's features, which name no lane,
/// or about the tasks in no project, which `live` ignores and so are never
/// held.
fn knows(model: Model, text: String) -> Bool {
  no_project(text)
  || projects_event(text)
  || result.is_ok(lane_of(model, event_dir(model, text)))
  || result.is_ok(features_event(model, text))
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
      // A dragged height is the task element's own; 0 leaves the CSS
      // default.
      case model.detail_height {
        0 -> a.none()
        h -> a.style("height", int.to_string(h) <> "px")
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

/// The mark on a feature's Tech Debt card (D-112).
fn debt_mark() -> Element(Msg) {
  h.span(
    [
      a.class("debt"),
      a.title("Tech debt: minor findings from this feature's reviews"),
    ],
    [h.text("Tech debt")],
  )
}

/// Applies a message about the root's features, which names no project
/// (D-87); an error for any other message.
fn features_event(model: Model, text: String) -> Result(Model, Nil) {
  let feature = {
    use id <- decode.field("id", decode.string)
    use title <- decode.optional_field("title", "", decode.string)
    use slot <- decode.optional_field(
      "colorSlot",
      None,
      decode.optional(decode.int),
    )
    use state <- decode.optional_field("state", "proposed", decode.string)
    decode.success(#(id, title, slot, state))
  }
  let without = fn(id) { list.filter(model.features, fn(f) { f.0 != id }) }
  case json.parse(text, decode.at(["features"], decode.list(feature))) {
    Ok(fs) -> Ok(Model(..model, features: fs))
    Error(_) ->
      case json.parse(text, decode.at(["feature"], feature)) {
        Ok(f) -> Ok(Model(..model, features: [f, ..without(f.0)]))
        Error(_) ->
          json.parse(text, decode.at(["removedFeature"], decode.string))
          |> result.map(fn(id) { Model(..model, features: without(id)) })
          |> result.replace_error(Nil)
      }
  }
}

/// Applies one `/events` message about a project's tasks or workflow.
fn task_event(model: Model, text: String, notify: Bool) -> Model {
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

const detail_key = "solace-board-detail"

const detail_band_styles =
  "
main{display:flex;flex-direction:column;height:100%}
.lanes-area{flex:1;min-height:0;overflow-y:auto;overscroll-behavior:contain}
.lanes-area{scrollbar-gutter:stable}
.lanes-area > .lanes{margin:0}
.lanes-area .board{margin:0}
.lane-head{box-shadow:0 -10px 0 var(--paper)}
.detail-band{flex:none;display:flex;flex-direction:column;border-top:2px solid var(--edge);background:var(--paper)}
.detail-band > solace-task{flex:none;height:min(45dvh,calc(100dvh - 190px));min-height:0;overflow-y:auto;visibility:visible;transition:height .18s ease-out,visibility .18s}
.detail-band.collapsed > solace-task{height:0 !important;overflow:hidden;visibility:hidden}
.detail-band.resizing > solace-task{transition:none}
.detail-resize{height:12px;margin:0;cursor:ns-resize;touch-action:none;display:flex;align-items:center;justify-content:center;color:var(--line)}
.detail-resize::before{content:\"\";width:40px;height:4px;border-top:2px solid currentColor;border-bottom:2px solid currentColor}
.detail-resize:hover::before,.detail-resize:focus-visible::before{color:var(--ink)}
.detail-bar{display:flex;gap:8px;align-items:center;padding:10px 0}
.detail-toggle{border:0;background:transparent;font-weight:700;font-size:17px;margin:0;padding:4px 10px}
.detail-toggle:hover{background:var(--hover)}
"

/// The detail band's bar: the open task's title with the collapse control,
/// the same glyph pattern as a lane's toggle. With no task open the bar is
/// empty: there is nothing to collapse.
fn detail_bar(model: Model) -> Element(Msg) {
  case model.selected_card {
    None -> h.header([a.class("detail-bar")], [])
    Some(_) ->
      h.header([a.class("detail-bar")], [
        h.button(
          [
            a.class("detail-toggle"),
            a.aria_expanded(!model.detail_collapsed),
            e.on_click(ToggleDetail),
          ],
          [
            h.text(case model.detail_collapsed {
              True -> "▸ "
              False -> "▾ "
            }),
            h.text(detail_title(model)),
          ],
        ),
      ])
  }
}

/// The open task's title, taken from the lane that stores it; its id when
/// the task is not listed.
fn detail_title(model: Model) -> String {
  case model.selected_card {
    None -> ""
    Some(#(dir, id)) ->
      case
        list.find(model.lanes, fn(l: Lane) { l.dir == dir })
        |> result.try(fn(l: Lane) { list.find(l.tasks, fn(t) { t.id == id }) })
      {
        Ok(t) -> t.title
        Error(_) -> id
      }
  }
}

/// Asking to show a task opens the detail band if it was collapsed.
fn expanded(model: Model) -> Model {
  case model.detail_collapsed {
    True -> {
      store(detail_key, "0")
      Model(..model, detail_collapsed: False)
    }
    False -> model
  }
}

/// The band's height for a pointer at `y` in a drag that began at `start_y`
/// with the band `start_h` high; the browser rounds the pixel.
@external(javascript, "./api.mjs", "resize_height")
fn resize_height(start_y: Float, start_h: Float, y: Float, max: Int) -> Int

const detail_height_key = "solace-board-detail-height"

/// The detail band keeps a readable height, and leaves the board room for
/// its column bar: `max` comes from the browser.
fn clamp_height(height: Int, max: Int) -> Int {
  let min = detail_min()
  case height {
    _ if height < min -> min
    _ if height > max -> max
    _ -> height
  }
}

/// Starts a drag of the detail band's handle: the browser keeps the pointer
/// on the handle, so its pointermove and pointerup keep arriving as the
/// pointer travels over the page, and answers the pointer's y, the band's
/// height now, and the largest height that keeps the board's column bar on
/// screen.
@external(javascript, "./api.mjs", "begin_resize")
fn begin_resize(
  root: dynamic.Dynamic,
  pointer_id: Int,
  y: Float,
) -> #(Float, Float, Int)

/// The largest detail height that keeps the board's column bar on screen.
@external(javascript, "./api.mjs", "detail_max")
fn detail_max() -> Int

/// The detail band's resize handle: a grip at its top edge, dragged with
/// the pointer or stepped with the arrow keys, Home for the default height.
/// It shows only when the band is open on a task: a collapsed band has no
/// height to change.
fn detail_resize(model: Model) -> Element(Msg) {
  case model.selected_card, model.detail_collapsed {
    Some(_), False ->
      h.div(
        [
          a.class("detail-resize"),
          a.role("separator"),
          a.attribute("aria-orientation", "horizontal"),
          a.attribute("aria-label", "Resize the task details"),
          a.attribute("aria-valuenow", int.to_string(model.detail_height)),
          a.attribute("tabindex", "0"),
          a.title("Drag to resize, or press arrow keys"),
          e.prevent_default(
            e.on("pointerdown", {
              use y <- decode.field("clientY", decode.float)
              use id <- decode.field("pointerId", decode.int)
              decode.success(ResizeStart(y, id))
            }),
          ),
          e.on("pointermove", {
            use y <- decode.field("clientY", decode.float)
            decode.success(ResizeMove(y))
          }),
          e.on("pointerup", decode.success(ResizeEnd)),
          e.on("pointercancel", decode.success(ResizeEnd)),
          e.on("keydown", {
            use key <- decode.field("key", decode.string)
            decode.success(ResizeKey(key))
          }),
        ],
        [],
      )
    _, _ -> h.text("")
  }
}

/// The smallest detail height, shared with the drag's arithmetic.
@external(javascript, "./api.mjs", "detail_min")
fn detail_min() -> Int

/// The band's task height now, for a key step from the CSS default.
@external(javascript, "./api.mjs", "detail_now")
fn detail_now(root: dynamic.Dynamic) -> Int

/// The column a task waits in while it is not active. The board leaves it
/// out, with its tasks; the Strategy page shows them (D-133).
const backlog = "backlog"

/// The columns the board draws: the workflow's, without Backlog (D-133), so
/// Planning comes first in the default workflow.
pub fn board_columns(
  workflow: solace_task.Workflow,
) -> List(solace_task.Column) {
  list.filter(workflow.columns, fn(c) { c.id != backlog })
}

/// Whether a live message is about the tasks in no project (D-129), which
/// only the Strategy page shows; the board ignores it.
fn no_project(text: String) -> Bool {
  json.parse(text, decode.at(["noProject"], decode.bool)) == Ok(True)
}
