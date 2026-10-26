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
    /// Why the project can't run its commands, "" when it can (D-167).
    needs: String,
    workflow: solace_task.Workflow,
    tasks: List(solace_task.Task),
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
    /// The detail band's width in pixels when it sits at the right edge; 0
    /// is the CSS default.
    detail_width: Int,
    /// Whether the window is at least 1.1 times as wide as it is tall, so
    /// an open task's band sits at the right edge instead of the bottom.
    side: Bool,
    /// A drag of the resize handle in progress: the pointer's start, the
    /// band's size then, and the largest size that keeps the board's
    /// column bar on screen.
    resizing: Option(#(Float, Float, Int)),
    /// Copy ID in the bar: how many copies were asked for, and what it says
    /// for a moment after the last ("" for Copy ID).
    copy: #(Int, String),
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
    /// color their tasks' cards (D-114).
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
  /// The resize handle's pointerdown: the pointer's x, y and id.
  ResizeStart(Float, Float, Int)
  /// A drag has begun: the pointer's x or y, the band's size then, and the
  /// largest size that keeps the board's column bar on screen.
  ResizeBegin(Float, Float, Int)
  /// The handle's pointermove: the pointer's x and y.
  ResizeMove(Float, Float)
  /// The drag ends: the size is kept.
  ResizeEnd
  /// The handle's keydown: the key's name.
  ResizeKey(String)
  /// Whether the window is at least 1.1 times as wide as it is tall.
  Sided(Bool)
  /// The bar's Copy ID: the open task's reference goes to the clipboard.
  CopyId
  /// Copy number `n` is done: whether the clipboard took it.
  CopyDone(Int, Bool)
  /// Copy number `n`'s moment has passed.
  CopyReset(Int)
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
        Lane("", "", "", "", "", "", solace_task.Workflow([], dict.new()), []),
      ],
      workspace: False,
      listed: False,
      selected_card: None,
      error: "",
      notices: [],
      caught_up: False,
      replay: [],
      collapsed: string.split(load(collapsed_key), "\n")
        |> list.filter(fn(d) { d != "" }),
      detail_collapsed: load(detail_key) == "1",
      detail_height: load(detail_height_key) |> int.parse |> result.unwrap(0),
      detail_width: load(detail_width_key) |> int.parse |> result.unwrap(0),
      side: False,
      resizing: None,
      copy: #(0, ""),
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
      effect.from(fn(send) {
        prevent_drag(root)
        watch_side(root, fn(side) { send(Sided(side)) })
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
        Ok(#(True, _)) if model.listed && !model.workspace -> {
          reload()
          #(model, effect.none())
        }
        Ok(#(workspace, lanes)) -> {
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
      expanded(
        Model(..model, selected_card: Some(#(dir, id)), live: [], copy: #(
          model.copy.0 + 1,
          "",
        )),
      ),
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
    // answers the band's size now and the largest size that leaves the
    // board its column bar. On the side the drag is across, so it follows x.
    ResizeStart(x, y, pointer_id) ->
      case model.selected_card, model.detail_collapsed {
        Some(_), False -> #(
          model,
          effect.from(fn(dispatch) {
            let #(start, size, max) =
              begin_resize(
                model.root,
                pointer_id,
                along(model, x, y),
                model.side,
              )
            dispatch(ResizeBegin(start, size, max))
          }),
        )
        _, _ -> #(model, effect.none())
      }
    ResizeBegin(y, height, max) -> #(
      Model(..model, resizing: Some(#(y, height, max))),
      effect.none(),
    )
    // Dragging up, or left on the side, grows the band; the size is clamped
    // so the board always keeps its column bar.
    ResizeMove(x, y) ->
      case model.resizing {
        Some(#(start, start_size, max)) -> #(
          sized(
            model,
            clamp_size(
              resize_size(start, start_size, along(model, x, y)),
              min_size(model),
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
          effect.from(fn(_) { store_size(model) }),
        )
        None -> #(model, effect.none())
      }
    // The handle's keys: the arrows step the size, Home returns the
    // default. Up grows the band at the bottom, Left grows it on the side.
    ResizeKey(key) -> {
      let #(grow, shrink) = case model.side {
        True -> #("ArrowLeft", "ArrowRight")
        False -> #("ArrowUp", "ArrowDown")
      }
      case key == grow || key == shrink || key == "Home" {
        True -> {
          // The default size is the CSS's: a step starts from what shows.
          let now = case size(model) {
            0 -> detail_now(model.root, model.side)
            s -> s
          }
          let max = detail_max(model.side)
          let min = min_size(model)
          let next = case key {
            _ if key == grow -> clamp_size(now + 24, min, max)
            _ if key == shrink -> clamp_size(now - 24, min, max)
            _ -> 0
          }
          let model = sized(model, next)
          #(model, effect.from(fn(_) { store_size(model) }))
        }
        False -> #(model, effect.none())
      }
    }
    Sided(side) -> #(Model(..model, side: side), effect.none())
    // Pasted into any text, the reference is a link to the task (D-98).
    CopyId ->
      case model.selected_card {
        None -> #(model, effect.none())
        Some(#(_, id)) -> {
          let n = model.copy.0 + 1
          #(
            Model(..model, copy: #(n, "")),
            effect.from(fn(send) {
              solace_task.copy_reference(id, fn(ok) { send(CopyDone(n, ok)) })
            }),
          )
        }
      }
    CopyDone(n, ok) if n == model.copy.0 -> #(
      Model(..model, copy: #(n, solace_task.copy_said(ok))),
      effect.from(fn(send) {
        solace_task.after_copied(fn() { send(CopyReset(n)) })
      }),
    )
    CopyReset(n) if n == model.copy.0 -> #(
      Model(..model, copy: #(n, "")),
      effect.none(),
    )
    CopyDone(_, _) | CopyReset(_) -> #(model, effect.none())
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
          copy: #(model.copy.0 + 1, ""),
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
          copy: #(model.copy.0 + 1, ""),
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

fn projects_decoder() -> decode.Decoder(#(Bool, List(Lane))) {
  let lane = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use name <- decode.field("name", decode.string)
    use base <- decode.field("base", decode.string)
    use target <- decode.optional_field("target", "", decode.string)
    use needs <- decode.optional_field("needs", "", decode.string)
    decode.success(
      Lane(
        dir,
        label,
        name,
        base,
        target,
        needs,
        solace_task.Workflow([], dict.new()),
        [],
      ),
    )
  }
  use workspace <- decode.field("workspace", decode.bool)
  use lanes <- decode.field("projects", decode.list(lane))
  decode.success(#(workspace, lanes))
}

pub fn view(model: Model) -> Element(Msg) {
  h.main([], [
    h.style(
      [],
      board_styles
        <> detail_band_styles
        <> selected_card_styles
        <> notice_styles
        <> archive_styles
        <> waiting_styles,
    ),
    // The error line shows only with an error: an empty paragraph in the
    // page's flex column takes a line above the lanes.
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
            case n.id {
              "" -> h.text("")
              _ ->
                h.button([a.class("show"), e.on_click(Open(n))], [
                  h.text(solace_task.ui_label("show_task")),
                ])
            },
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
    h.div(
      [
        a.class(case on_side(model), model.detail_collapsed {
          True, True -> "work side collapsed"
          True, False -> "work side"
          False, _ -> "work"
        }),
      ],
      [
        // Beside the band the board narrows to its smallest columns, then
        // the band covers it. (`a.none()` would add an empty class.)
        h.div(
          [
            a.class("lanes-area"),
            ..case spread(model) {
              True -> [
                a.style(
                  "margin-right",
                  "min("
                    <> band_width(model)
                    <> ", max(0px, 100% - "
                    <> int.to_string(lanes_min(model))
                    <> "px))",
                ),
              ]
              False -> []
            }
          ],
          [lanes_view(model)],
        ),
        h.div(
          [
            a.class(case model.detail_collapsed, model.resizing {
              True, _ -> "detail-band collapsed"
              False, Some(_) -> "detail-band resizing"
              False, None -> "detail-band"
            }),
            ..case spread(model) {
              True -> [a.style("width", band_width(model))]
              False -> []
            }
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
        h.text("No projects on this board yet. Add or create one on the "),
        h.a([a.href("/workspace")], [h.text("Workspace page")]),
        h.text("."),
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
/// has an Archive button in its upper right corner (D-95); the card's text
/// fills the card and its first line wraps around the button, which a floated
/// spacer inside the card makes room for.
/// A card waiting on an unfinished dependency is dimmed and says what it
/// waits on; it still opens as any other.
/// A card's stripe is its feature's color, gray with no feature or one not
/// started yet; a line under the title names the feature and the tooltip
/// spells it out, so color is not its only sign (D-114, D-155).
/// A card shows its title cut to 200 characters, the whole title in its
/// tooltip; it shows no scope line, which the task's Scope tab lists.
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
          let feature_name = case t.feature {
            Some(f) -> list.key_find(names, f) |> result.unwrap("")
            None -> ""
          }
          let feature = case feature_name {
            "" -> ""
            name -> "Feature: " <> name
          }
          let short = card_title(t.title)
          let tip =
            case short == t.title {
              True -> [waiting, feature]
              False -> ["Title: " <> t.title, waiting, feature]
            }
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
                h.text(short),
                case feature_name {
                  "" -> h.text("")
                  _ ->
                    h.small([a.class("card-feature")], [h.text(feature_name)])
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
    case lane.needs {
      "" -> h.text("")
      needs ->
        h.small([a.class("lane-needs attention")], [h.text(" " <> needs)])
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
/// the server marks `onBoard` whose projects include this lane's (D-61, D-196).
fn cards(model: Model, lane: Lane) -> List(#(String, solace_task.Task)) {
  list.flat_map(model.lanes, fn(l) {
    l.tasks
    |> list.filter(fn(t) {
      t.on_board && list.contains(scope_of(l.dir, t), lane.dir)
    })
    |> list.map(fn(t) { #(l.dir, t) })
  })
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

/// The board's own look: the lanes, columns, cards and notices it renders.
/// The task detail's elements live in `<solace-task>`'s shadow root, so the
/// rules for them are `solace_task.task_styles`'s, not here.
const board_styles =
  solace_task.control_styles
  <> ":host{display:block;--well:#eceee6;--hover:#e7eeea;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--edge:var(--ink);--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a;--focus:#174f78;box-sizing:border-box;padding:0 var(--space-6);height:calc(100dvh - 56px);user-select:none;-webkit-user-select:none}\nmain{color:var(--ink);background:var(--sheet)}button{font:inherit;padding:var(--control-pad);margin:var(--space-1);border:1px solid var(--edge);border-radius:0;cursor:pointer}.board{display:flex;gap:12px;overflow-x:auto;margin:24px 0}.column{min-width:190px;flex:1;border-radius:0}.card{display:block;width:100%;text-align:left}small{display:block;color:var(--soft);overflow-wrap:anywhere}[role=alert]{color:var(--revision)}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.muted{color:var(--soft)}\n.column{min-height:240px}\n\n.status.finished{background:var(--add);color:var(--ok-text)}\n.lanes{display:grid;gap:0;margin:24px 0;overflow-x:auto;padding-bottom:4px}.lanes .board{margin:0;overflow:visible}.lane .board{margin:8px 0}.lane{border:1px solid var(--line);border-radius:0;padding:var(--space-1) var(--space-2);background:var(--paper)}.lane .column{min-height:110px}.lane-title{align-items:center;gap:12px}.lane-title small{display:inline}.lane-title button{padding:2px 10px;min-height:var(--control-h-small)}.lane-toggle{border:0;background:transparent;font-size:17px}.lane-head{margin:0;padding:0 var(--space-2)}.lane-head .column{min-height:0;padding:0 12px;background:transparent}\n.stamp{color:var(--ok-text);font-weight:700;margin-right:6px}.hazard{display:inline-block;width:18px;height:18px;margin-right:6px;vertical-align:-3px}.hazard svg{width:100%;height:100%}.hazard path{fill:#f2c94c;stroke:#7a5200;stroke-width:1.5;stroke-linejoin:round}.hazard path.mark{fill:none;stroke:#3d2900;stroke-width:2.5;stroke-linecap:round}.notices{position:fixed;right:16px;bottom:16px;z-index:10;display:grid;gap:8px;max-width:min(380px,calc(100% - 32px))}.notices .notice{position:relative;background:var(--ink);color:var(--paper);padding:12px 52px 12px 14px;border-radius:0;box-shadow:0 4px 16px var(--shadow)}.notice strong{display:block;overflow-wrap:anywhere}.notice .close{position:absolute;top:6px;right:6px;margin:0;padding:6px;line-height:0;background:transparent;color:var(--paper);border:0}.notice .close:hover{background:color-mix(in srgb,var(--paper) 20%,transparent)}.notice .close svg{width:24px;height:24px;fill:none;stroke:currentColor;stroke-width:2.5;stroke-linecap:round}
:host{background:var(--sheet);color:var(--ink)}
main{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;color:var(--ink);background:none}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button:disabled{color:var(--soft)}a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}

.primary{background:var(--ink);color:var(--paper);border-color:var(--edge);font-weight:700}.primary:hover{background:var(--focus);border-color:var(--focus);color:var(--paper)}
.column{background:var(--well);border:1px solid var(--line);padding:0 12px 12px}
.column h2{font-size:13px;text-transform:uppercase;letter-spacing:.1em;margin:0 -12px 12px;padding:10px 12px;background:var(--paper);border-bottom:2px solid var(--edge)}
.lanes{display:block;overflow:visible}.lane-head{position:sticky;top:0;z-index:5;background:var(--paper);border-bottom:2px solid var(--edge)}.lane-head .column{background:transparent;border-color:transparent;border-top:0;border-bottom:0}.lane-head h2{margin:0 -12px;border-bottom:0}.lane .lane-head{margin:0;padding:0}
.lane{border-color:var(--edge);background:var(--paper)}.lane-title{padding:6px 0}.lane-toggle{font-weight:700;background:transparent}.lane-toggle:hover{background:var(--hover)}
.card{position:relative;background:var(--paper);border-color:var(--edge);padding:10px 12px 10px 22px;line-height:1.35;--tab:#999999}
.card::after{content:\"\";position:absolute;inset:0 auto 0 0;width:8px;background:var(--tab)}
.card:hover{background:var(--hover)}.card+.card{margin-top:8px}
.attention{font-weight:700}.stamp{margin-right:6px}
.debt{display:inline-block;font-size:11px;text-transform:uppercase;letter-spacing:.08em;border:1px solid var(--test);color:var(--test);padding:0 6px;margin-right:6px}.card:has(.debt){--tab:var(--test)}
.status{border:1px solid var(--line);background:var(--paper);font-size:11px;text-transform:uppercase;letter-spacing:.08em;padding:1px 8px}.status.finished{background:var(--add);color:var(--ok-text);border-color:var(--ok-text)}.working::before{background:var(--entry)}

.notices .notice{background:var(--ink);color:var(--paper);box-shadow:4px 8px 24px var(--shadow)}.notice .close{color:var(--paper)}
@media(prefers-reduced-motion:no-preference){button,.card{transition:background-color .12s ease-out}}
.lane+.lane{margin-top:-1px}
.lane-title a{display:inline-flex;align-items:center;gap:8px;padding:6px 12px;border:1px solid var(--edge);background:var(--paper);color:var(--ink);text-decoration:none;font-weight:700}
.lane-title a:hover{background:var(--hover)}
.lane-title a::after{content:\"\";width:14px;height:14px;background:currentColor;-webkit-mask:url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M4 12h15M13 5l7 7-7 7'/%3E%3C/svg%3E\") center/contain no-repeat;mask:url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M4 12h15M13 5l7 7-7 7'/%3E%3C/svg%3E\") center/contain no-repeat}"

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
      // The band's bar shows the title, Copy ID, column and status.
      a.attribute("bar", ""),
      case model.workspace {
        True -> a.attribute("workspace", "")
        False -> a.none()
      },
      // A dragged height is the task element's own; 0 leaves the CSS
      // default. On the side the band's width is dragged instead.
      case model.detail_height, on_side(model) {
        0, _ | _, True -> a.none()
        h, False -> a.style("height", int.to_string(h) <> "px")
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

/// The spacer is as tall as the small control the Archive button is, and 64px
/// wide for its label at the 12px font the button's own rule sets.
const archive_styles =
  ".card-wrap{position:relative}.card-wrap+.card-wrap{margin-top:8px}.card-wrap:has(.archive) .card::before{content:\"\";float:right;width:64px;height:var(--control-h-small)}.card-wrap .archive{position:absolute;top:6px;right:6px;margin:0;padding:2px 8px;font-size:12px}"

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
  ".card.waiting{opacity:.55}.card.waiting:hover,.card.waiting[aria-current=true],.card.waiting:focus-visible{opacity:.85}.card .waiting-on,.card .card-feature{font-size:13px}"

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
    Ok(_) ->
      apply_first(model, dir, text, notify, [
        task_update,
        workflow_update,
        tasks_update,
        unreadable_update,
        removed_update,
      ])
  }
}

const detail_key = "solace-board-detail"

const detail_band_styles =
  "
main{display:flex;flex-direction:column;height:100%}
.work{flex:1;min-height:0;display:flex;flex-direction:column;position:relative}
.lanes-area{flex:1;min-height:0;overflow-y:auto;overscroll-behavior:contain}
.lanes-area{scrollbar-gutter:stable}
.lanes-area > .lanes{margin:0}
.lanes-area .board{margin:0}
.lane-head{box-shadow:0 -10px 0 var(--paper)}
.detail-band{flex:none;display:flex;flex-direction:column;position:relative;border-top:2px solid var(--edge);background:var(--paper)}
.detail-band > solace-task{flex:none;height:min(45dvh,calc(100dvh - 190px));min-height:0;overflow-y:auto;visibility:visible;transition:height .18s ease-out,visibility .18s}
.detail-band.collapsed > solace-task{height:0 !important;overflow:hidden;visibility:hidden}
.detail-band.resizing > solace-task{transition:none}
.detail-resize{position:absolute;top:-7px;left:0;right:0;z-index:1;height:12px;margin:0;cursor:ns-resize;touch-action:none;display:flex;align-items:center;justify-content:center;color:var(--line)}
.detail-resize::before{content:\"\";width:40px;height:4px;border-top:2px solid currentColor;border-bottom:2px solid currentColor}
.detail-resize:hover::before,.detail-resize:focus-visible::before{color:var(--ink)}
.detail-bar{display:flex;flex-wrap:wrap;gap:4px 12px;align-items:center;padding:2px 0}
.detail-bar .copy-id{flex:none;margin:0;padding:0 10px;font-size:13px}
.detail-bar .meta{margin:0}
.detail-toggle{border:0;background:transparent;font-weight:700;font-size:17px;margin:0;padding:2px 10px;min-height:var(--control-h-small)}
.detail-toggle:hover{background:var(--hover)}
.work.side > .detail-band{position:absolute;top:0;right:0;bottom:0;z-index:6;box-sizing:border-box;min-width:32rem;border-top:0;border-left:2px solid var(--edge);padding-left:12px;box-shadow:-8px 0 16px var(--shadow)}
.work.side > .detail-band > solace-task{flex:1 1 0;height:auto}
.work.side.collapsed{flex-direction:row}
.work.side.collapsed > .lanes-area{min-width:0}
.work.side.collapsed > .detail-band{position:static;min-width:0;padding:0 4px;box-shadow:none}
.work.side.collapsed > .detail-band > solace-task{display:none}
.work.side .detail-resize{left:-7px;right:auto;top:0;bottom:0;width:12px;height:auto;cursor:ew-resize}
.work.side .detail-resize::before{width:4px;height:40px;border:0;border-left:2px solid currentColor;border-right:2px solid currentColor}
"

/// The detail band's bar: the open task's title with the collapse control,
/// the same glyph pattern as a lane's toggle, then Copy ID, its column and
/// its status, on the title's line while there is room. The task element
/// leaves these out (its `bar` attribute). Collapsed on the side, it is
/// only an expand button. With no task open the bar is empty: there is
/// nothing to collapse.
fn detail_bar(model: Model) -> Element(Msg) {
  let found = detail_task(model)
  let title = fn(id) {
    case found {
      Ok(#(_, t)) -> t.title
      Error(_) -> id
    }
  }
  case model.selected_card, on_side(model) && model.detail_collapsed {
    None, _ -> h.header([a.class("detail-bar")], [])
    Some(#(_, id)), True -> {
      let label = "Show the task details: " <> title(id)
      h.header([a.class("detail-bar")], [
        h.button(
          [
            a.class("detail-toggle"),
            a.aria_expanded(False),
            a.attribute("aria-label", label),
            a.title(label),
            e.on_click(ToggleDetail),
          ],
          [h.text("◂")],
        ),
      ])
    }
    Some(#(_, id)), False ->
      h.header([a.class("detail-bar")], [
        h.button(
          [
            a.class("detail-toggle"),
            a.aria_expanded(!model.detail_collapsed),
            e.on_click(ToggleDetail),
          ],
          [
            // On the side the band collapses to the right.
            h.text(case model.detail_collapsed || on_side(model) {
              True -> "▸ "
              False -> "▾ "
            }),
            h.text(title(id)),
          ],
        ),
        solace_task.copy_button(id, model.copy.1, CopyId),
        case found {
          Ok(#(workflow, t)) ->
            h.span([a.class("meta")], solace_task.meta(workflow, t))
          Error(_) -> h.text("")
        },
      ])
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

const detail_height_key = "solace-board-detail-height"

/// Starts a drag of the detail band's handle: the browser keeps the pointer
/// on the handle, so its pointermove and pointerup keep arriving as the
/// pointer travels over the page, and answers the pointer's position, the
/// band's size now (its width on the side), and the largest size that
/// keeps the board's column bar on screen.
@external(javascript, "./api.mjs", "begin_resize")
fn begin_resize(
  root: dynamic.Dynamic,
  pointer_id: Int,
  at: Float,
  side: Bool,
) -> #(Float, Float, Int)

/// The largest detail size that keeps the board's column bar on screen, or
/// on the side a strip of the board.
@external(javascript, "./api.mjs", "detail_max")
fn detail_max(side: Bool) -> Int

/// The detail band's resize handle: a grip at its top edge, or its left
/// edge on the side, dragged with the pointer or stepped with the arrow
/// keys, Home for the default size. It shows only when the band is open on
/// a task: a collapsed band has no size to change.
fn detail_resize(model: Model) -> Element(Msg) {
  case model.selected_card, model.detail_collapsed {
    Some(_), False ->
      h.div(
        [
          a.class("detail-resize"),
          a.role("separator"),
          a.attribute("aria-orientation", case model.side {
            True -> "vertical"
            False -> "horizontal"
          }),
          a.attribute("aria-label", "Resize the task details"),
          a.attribute("aria-valuenow", int.to_string(size(model))),
          a.attribute("tabindex", "0"),
          a.title("Drag to resize, or press arrow keys"),
          e.prevent_default(
            e.on("pointerdown", {
              use x <- decode.field("clientX", decode.float)
              use y <- decode.field("clientY", decode.float)
              use id <- decode.field("pointerId", decode.int)
              decode.success(ResizeStart(x, y, id))
            }),
          ),
          e.on("pointermove", {
            use x <- decode.field("clientX", decode.float)
            use y <- decode.field("clientY", decode.float)
            decode.success(ResizeMove(x, y))
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

/// The band's size now, its width on the side, for a key step from the CSS
/// default.
@external(javascript, "./api.mjs", "detail_now")
fn detail_now(root: dynamic.Dynamic, side: Bool) -> Int

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

const detail_width_key = "solace-board-detail-width"

/// The pointer's position along the band's resize axis: x on the side, y
/// at the bottom.
fn along(model: Model, x: Float, y: Float) -> Float {
  case model.side {
    True -> x
    False -> y
  }
}

/// The band's size along its resize axis: its width on the side, its
/// task's height at the bottom; 0 is the CSS default.
fn size(model: Model) -> Int {
  case model.side {
    True -> model.detail_width
    False -> model.detail_height
  }
}

/// The band at `size` along its resize axis.
fn sized(model: Model, size: Int) -> Model {
  case model.side {
    True -> Model(..model, detail_width: size)
    False -> Model(..model, detail_height: size)
  }
}

/// Keeps the band's size along its resize axis for the next visit.
fn store_size(model: Model) -> Nil {
  case model.side {
    True -> store(detail_width_key, int.to_string(model.detail_width))
    False -> store(detail_height_key, int.to_string(model.detail_height))
  }
}

/// Whether the open task's band sits at the right edge: the window is wide
/// and a task shows. Collapsed, it is a strip holding its expand button; an
/// empty band stays a line at the bottom.
pub fn on_side(model: Model) -> Bool {
  model.side && model.selected_card != None
}

/// The band's width on the side: the dragged width, or the CSS default,
/// never under 32rem, so the board's margin beside it matches it.
fn band_width(model: Model) -> String {
  case model.detail_width {
    0 -> "max(32rem, min(42vw, 720px))"
    w -> int.to_string(int.max(w, side_min())) <> "px"
  }
}

/// The narrowest the board gets beside the band, in pixels: its widest
/// lane's columns at their smallest. Past that, the band covers the board.
fn lanes_min(model: Model) -> Int {
  let columns =
    list.fold(model.lanes, 1, fn(most, l: Lane) {
      int.max(most, list.length(board_columns(l.workflow)))
    })
  lanes_width(columns)
}

/// Tells `send` whether the window is at least 1.1 times as wide as it is
/// tall, now and on every change.
@external(javascript, "./api.mjs", "watch_side")
fn watch_side(root: dynamic.Dynamic, send: fn(Bool) -> Nil) -> Nil

/// The open task and its lane's workflow, taken from the lane that stores
/// it; an error when the task is not listed.
fn detail_task(
  model: Model,
) -> Result(#(solace_task.Workflow, solace_task.Task), Nil) {
  case model.selected_card {
    None -> Error(Nil)
    Some(#(dir, id)) -> {
      use l <- result.try(list.find(model.lanes, fn(l: Lane) { l.dir == dir }))
      use t <- result.map(list.find(l.tasks, fn(t) { t.id == id }))
      #(l.workflow, t)
    }
  }
}

/// The detail band keeps a readable height, or on the side at least 32rem
/// of width, and leaves the board room for its column bar: `max` comes
/// from the browser.
fn clamp_size(size: Int, min: Int, max: Int) -> Int {
  int.clamp(size, min, int.max(min, max))
}

/// The band's smallest size along its resize axis.
fn min_size(model: Model) -> Int {
  case model.side {
    True -> side_min()
    False -> detail_min()
  }
}

/// The side band's smallest width, 32rem, in pixels.
@external(javascript, "./api.mjs", "side_min")
fn side_min() -> Int

/// The band's size for a pointer at `at` in a drag that began at `start`
/// with the band `start_size` along the drag (its height, or its width on
/// the side); the browser rounds the pixel.
@external(javascript, "./api.mjs", "resize_size")
fn resize_size(start: Float, start_size: Float, at: Float) -> Int

/// Whether the band is open on the side, so it takes its width and the
/// board narrows beside it.
fn spread(model: Model) -> Bool {
  on_side(model) && !model.detail_collapsed
}

/// The width of `columns` board columns at their smallest, with the
/// lane's edges and the scroll bar's gutter, from the sizes in
/// `board_styles` (`.column`, `.board`, `.lane`) and the browser's
/// scroll bar (16px outside one).
pub fn lanes_width(columns: Int) -> Int {
  columns
  * { column_min + 2 * column_pad + 2 * column_border }
  + { columns - 1 }
  * column_gap
  + lane_edges
  + scroll_gutter()
}

/// `.column`'s `min-width`, in pixels.
const column_min = 190

/// `.column`'s left and right padding, in pixels.
const column_pad = 12

/// `.board`'s gap between columns, in pixels.
const column_gap = 12

/// A lane's padding and border on both sides, in pixels.
const lane_edges = 18

/// The width `.lanes-area` keeps for its scroll bar, in pixels, measured
/// in the browser: scroll bars differ between platforms.
@external(javascript, "./api.mjs", "scroll_gutter")
fn scroll_gutter() -> Int

/// `.column`'s border width, in pixels.
const column_border = 1

/// The title shown on a card: at most 200 characters, with an ellipsis when
/// it is cut; the whole title is in the card's tooltip.
fn card_title(title: String) -> String {
  case string.length(title) > 200 {
    True -> string.slice(title, 0, 200) <> "…"
    False -> title
  }
}

/// A project's tasks from a new connection's snapshot, read one at a time:
/// each task the board can read, and the id and title of each it can't, so a
/// record it can't read hides only that task.
fn task_list(
  text: String,
) -> Result(#(List(solace_task.Task), List(#(String, String))), Nil) {
  use raw <- result.try(
    json.parse(text, decode.at(["tasks"], decode.dynamic))
    |> result.replace_error(Nil),
  )
  use items <- result.try(
    decode.run(raw, decode.list(decode.dynamic))
    |> result.replace_error(Nil),
  )
  let #(tasks, unreadable) =
    list.fold(items, #([], []), fn(state, item) {
      case decode.run(item, solace_task.task_decoder()) {
        Ok(task) -> #([task, ..state.0], state.1)
        Error(_) -> #(state.0, [task_label(item), ..state.1])
      }
    })
  Ok(#(list.reverse(tasks), list.reverse(unreadable)))
}

/// The id and title a task the board can't read still names, for its notice.
fn task_label(task: dynamic.Dynamic) -> #(String, String) {
  let label = {
    use id <- decode.optional_field("id", "", decode.string)
    use title <- decode.optional_field("title", "", decode.string)
    decode.success(#(id, title))
  }
  decode.run(task, label) |> result.unwrap(#("", ""))
}

/// A task from a live event the board can't read, as its notice names it.
fn unreadable_task(text: String) -> Result(#(String, String), Nil) {
  json.parse(text, decode.at(["task"], decode.dynamic))
  |> result.replace_error(Nil)
  |> result.map(task_label)
}

/// The notices for tasks a message named but the board can't read: one per
/// task, so none is lost without the human being told.
fn unreadable_tasks(
  model: Model,
  dir: String,
  labels: List(#(String, String)),
) -> Model {
  list.fold(labels, model, fn(model, label) {
    let #(id, title) = label
    let heading = unreadable_heading(model, dir, title, id)
    case
      list.any(model.notices, fn(n) { is_unreadable_notice(model, dir, id, n) })
    {
      True -> model
      False ->
        Model(
          ..model,
          notices: list.append(model.notices, [
            solace_task.Notice(
              heading,
              unreadable_text(model, dir, id),
              dir,
              id,
              "details",
            ),
          ]),
        )
    }
  })
}

fn unreadable_heading(
  model: Model,
  dir: String,
  title: String,
  id: String,
) -> String {
  let named = case title, id {
    "", "" -> "a task"
    "", _ -> "task " <> id
    _, _ -> "task " <> title
  }
  unreadable_prefix(model, dir) <> named
}

/// Applies the first update that reads `text`, since one message has one shape.
fn apply_first(
  model: Model,
  dir: String,
  text: String,
  notify: Bool,
  updates: List(fn(Model, String, String, Bool) -> Result(Model, Nil)),
) -> Model {
  case updates {
    [] -> model
    [update, ..rest] ->
      case update(model, dir, text, notify) {
        Ok(model) -> model
        Error(_) -> apply_first(model, dir, text, notify, rest)
      }
  }
}

fn task_update(
  model: Model,
  dir: String,
  text: String,
  notify: Bool,
) -> Result(Model, Nil) {
  use task <- result.try(
    json.parse(text, decode.at(["task"], solace_task.task_decoder()))
    |> result.replace_error(Nil),
  )
  let new = case lane_of(model, dir), notify {
    Ok(lane), True -> task_notices(model, lane, task)
    _, _ -> []
  }
  Ok(
    map_lane(
      Model(..model, notices: list.append(model.notices, new)),
      dir,
      fn(l) {
        Lane(..l, tasks: [
          task,
          ..list.filter(l.tasks, fn(t) { t.id != task.id })
        ])
      },
    ),
  )
}

/// The notices a task's live update raises, named by its lane in a workspace.
fn task_notices(
  model: Model,
  lane: Lane,
  task: solace_task.Task,
) -> List(solace_task.Notice) {
  case list.find(lane.tasks, fn(t) { t.id == task.id }) {
    Error(_) -> []
    Ok(old) ->
      solace_task.notices(old, task)
      |> list.map(fn(n) {
        let n = solace_task.Notice(..n, dir: lane.dir)
        case model.workspace {
          True ->
            solace_task.Notice(..n, heading: lane.label <> ": " <> n.heading)
          False -> n
        }
      })
  }
}

fn workflow_update(
  model: Model,
  dir: String,
  text: String,
  _notify: Bool,
) -> Result(Model, Nil) {
  use workflow <- result.try(
    json.parse(text, decode.at(["workflow"], solace_task.workflow_decoder()))
    |> result.replace_error(Nil),
  )
  Ok(map_lane(model, dir, fn(l) { Lane(..l, workflow: workflow) }))
}

fn tasks_update(
  model: Model,
  dir: String,
  text: String,
  _notify: Bool,
) -> Result(Model, Nil) {
  use pair <- result.try(task_list(text))
  let #(tasks, unreadable) = pair
  let model = map_lane(model, dir, fn(l) { Lane(..l, tasks: tasks) })
  Ok(unreadable_tasks(model, dir, unreadable))
}

fn unreadable_update(
  model: Model,
  dir: String,
  text: String,
  _notify: Bool,
) -> Result(Model, Nil) {
  use label <- result.try(unreadable_task(text))
  Ok(unreadable_tasks(model, dir, [label]))
}

fn removed_update(
  model: Model,
  dir: String,
  text: String,
  _notify: Bool,
) -> Result(Model, Nil) {
  use id <- result.try(
    json.parse(text, decode.at(["removed"], decode.string))
    |> result.replace_error(Nil),
  )
  Ok(
    map_lane(model, dir, fn(l) {
      Lane(..l, tasks: list.filter(l.tasks, fn(t) { t.id != id }))
    })
    |> follow_move(dir, id),
  )
}

/// What an unreadable task's notice says its card shows: an older version
/// when its lane still holds the card, hidden when it does not.
fn unreadable_text(model: Model, dir: String, id: String) -> String {
  let shown = case lane_of(model, dir) {
    Ok(lane) -> list.any(lane.tasks, fn(t) { t.id == id })
    Error(_) -> False
  }
  case shown {
    True ->
      "The board can't read this task's record, so its card shows an older version."
    False -> "The board can't read this task's record, so its card is hidden."
  }
}

/// Whether `notice` is the unreadable notice for task `id` in `dir`, so a
/// title change doesn't add a second one.
fn is_unreadable_notice(
  model: Model,
  dir: String,
  id: String,
  notice: solace_task.Notice,
) -> Bool {
  notice.dir == dir
  && notice.id == id
  && notice.tab == "details"
  && string.starts_with(notice.heading, unreadable_prefix(model, dir))
}

/// The heading start shared by every unreadable notice for `dir`: its lane's
/// name in a workspace, then the kind.
fn unreadable_prefix(model: Model, dir: String) -> String {
  case model.workspace, list.find(model.lanes, fn(l) { l.dir == dir }) {
    True, Ok(lane) -> lane.label <> ": Can't read "
    _, _ -> "Can't read "
  }
}
