//// The Contract view page: a Lustre (Gleam, JavaScript target) web component, `<solace-contracts>`.

import gleam/dynamic
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import lustre
import lustre/attribute as a
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/element/svg as s
import lustre/event as e

/// Registers the element on its own, for `index.html`.
pub fn main() {
  let assert Ok(_) = register()
}

pub type Source

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./layout.mjs", "after_draw")
fn after_draw(callback: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(base: String, receive: fn(String) -> Nil) -> Source

@external(javascript, "./layout.mjs", "placement")
fn placement(
  nodes: String,
  edges: String,
  overview: Bool,
  anchor: String,
  root: dynamic.Dynamic,
  base: String,
) -> String

@external(javascript, "./layout.mjs", "fitted")
fn fitted(points: String, panel: Bool, root: dynamic.Dynamic) -> String

@external(javascript, "./layout.mjs", "task_id")
fn task_id() -> String

@external(javascript, "./layout.mjs", "viewport_width")
fn viewport_width(root: dynamic.Dynamic) -> Float

@external(javascript, "./layout.mjs", "viewport_height")
fn viewport_height(root: dynamic.Dynamic) -> Float

@external(javascript, "./layout.mjs", "keys")
fn keys(root: dynamic.Dynamic, receive: fn(String) -> Nil) -> Nil

@external(javascript, "./layout.mjs", "focus_node")
fn focus_node(root: dynamic.Dynamic, id: String) -> Nil

@external(javascript, "./layout.mjs", "panel_width")
fn panel_width() -> Float

@external(javascript, "./layout.mjs", "resizer")
fn resizer(root: dynamic.Dynamic) -> Nil

@external(javascript, "./api.mjs", "close_events")
fn close_events(source: Source) -> Nil

@external(javascript, "./api.mjs", "prevent_drag")
fn prevent_drag(root: dynamic.Dynamic) -> Nil

@external(javascript, "./api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

pub type Node {
  Node(
    id: String,
    name: String,
    kind: String,
    module: String,
    summary: String,
    signature: String,
    known: String,
    tests: Int,
    changed: Bool,
    removed: Bool,
    tier: String,
    sentence: String,
    checks: List(Check),
    public: Bool,
    takes: List(String),
    gives: String,
    requests: List(Capability),
    examples: List(Check),
  )
}

/// A request an entry point handles, in words: "Add item" with its fields (D-55.2).
pub type Capability {
  Capability(name: String, fields: List(String))
}

/// A source module, shown as an area of the project (D-55.1).
pub type Area {
  Area(id: String, name: String, summary: String)
}

pub type Check {
  Check(call: String, expected: String, text: String, was: String)
}

pub type Edge {
  Edge(from: String, to: String, kind: String, direct: Bool, known: String)
}

pub type Point {
  Point(id: String, x: Float, y: Float)
}

pub type Item {
  Item(
    id: String,
    change: String,
    before: String,
    after: String,
    effects_added: List(String),
    effects_removed: List(String),
    tier: String,
  )
}

pub type Test {
  Test(
    id: String,
    name: String,
    result: String,
    message: String,
    checks: List(Check),
  )
}

pub type Review {
  Review(
    summary: String,
    entries: List(Item),
    signatures: List(Item),
    tests: List(Test),
    files: List(Item),
    added: List(String),
    removed: List(String),
    untested: List(#(String, String)),
    reaches: List(String),
  )
}

pub type Graph {
  Graph(
    nodes: List(Node),
    edges: List(Edge),
    review: Option(Review),
    project: String,
    fingerprint: String,
    areas: List(Area),
  )
}

/// A project file opened from the Files tab, read-only, in place of the drawing.
pub type FileText {
  FileText(path: String, category: String, text: Option(String), note: String)
}

pub type Model {
  Model(
    graph: Graph,
    visible: List(String),
    expanded: List(String),
    positions: List(Point),
    selected: String,
    detail: Option(Node),
    source: String,
    diff: List(#(String, String)),
    search: String,
    tab: String,
    zoom: Float,
    pan_x: Float,
    pan_y: Float,
    dragging: Bool,
    drag_x: Float,
    drag_y: Float,
    travel: Float,
    focus: String,
    task: String,
    error: String,
    tests_requested: Bool,
    pane: Bool,
    file: Option(FileText),
    outline: Bool,
    asked: String,
    source_handle: Option(Source),
    /// The `base` attribute: this instance's API prefix, "" for the page's.
    prefix: String,
    /// The component's shadow root, once it has rendered.
    root: dynamic.Dynamic,
    connected: Bool,
  )
}

pub type Msg {
  Loaded(String)
  Select(String)
  Click(String)
  Focused(String)
  Key(String)
  Detail(String)
  Source(String)
  Search(String)
  Toggle(String)
  ExpandAll
  CollapseAll
  Tab(String)
  OpenFile(String)
  FileLoaded(String)
  CloseFile
  Reset
  Fit
  Zoom(Float)
  PanStart(Float, Float)
  PanMove(Float, Float)
  PanEnd
  RunTests
  TestsStarted(String)
  Live(String)
  Close
  SelectSource(String)
  ShowOutline(Bool)
  Show(String)
  Reveal(String)
  AskForSummaries
  Asked(String)
  BaseChanged(String)
  TaskChanged(String)
  Listening(Source)
  Attached(dynamic.Dynamic)
  Disconnected
  Connected
}

pub fn check_decoder() -> decode.Decoder(Check) {
  use call <- decode.optional_field("call", "", decode.string)
  use expected <- decode.optional_field("expected", "", decode.string)
  use text <- decode.optional_field("text", "", decode.string)
  use was <- decode.optional_field("was", "", decode.string)
  decode.success(Check(call, expected, text, was))
}

pub fn node_decoder() -> decode.Decoder(Node) {
  use id <- decode.field("id", decode.string)
  use name <- decode.field("name", decode.string)
  use kind <- decode.field("kind", decode.string)
  use module <- decode.optional_field("module", "", decode.string)
  use summary <- decode.optional_field("summary", "", decode.string)
  use signature <- decode.optional_field("signature", "", decode.string)
  use known <- decode.optional_field("known", "derived", decode.string)
  use tests <- decode.optional_field(
    "tests",
    0,
    decode.one_of(decode.int, [
      decode.map(decode.list(decode.string), list.length),
    ]),
  )
  use changed <- decode.optional_field("changed", False, decode.bool)
  use removed <- decode.optional_field("removed", False, decode.bool)
  use tier <- decode.optional_field("tier", "", decode.string)
  use sentence <- decode.optional_field("sentence", "", decode.string)
  use checks <- decode.optional_field(
    "checks",
    [],
    decode.list(check_decoder()),
  )
  use public <- decode.optional_field("public", False, decode.bool)
  use takes <- decode.optional_field("takes", [], decode.list(decode.string))
  use gives <- decode.optional_field("gives", "", decode.string)
  use requests <- decode.optional_field(
    "requests",
    [],
    decode.list({
      use name <- decode.field("name", decode.string)
      use fields <- decode.field("fields", decode.list(decode.string))
      decode.success(Capability(name, fields))
    }),
  )
  use examples <- decode.optional_field(
    "examples",
    [],
    decode.list(check_decoder()),
  )
  decode.success(Node(
    id,
    name,
    kind,
    module,
    summary,
    signature,
    known,
    tests,
    changed,
    removed,
    tier,
    sentence,
    checks,
    public,
    takes,
    gives,
    requests,
    examples,
  ))
}

/// A node the view makes itself, such as an area box.
fn made(id: String, kind: String, name: String, module: String) -> Node {
  Node(
    id,
    name,
    kind,
    module,
    "",
    "",
    "derived",
    0,
    False,
    False,
    "",
    "",
    [],
    False,
    [],
    "",
    [],
    [],
  )
}

pub fn edge_decoder() -> decode.Decoder(Edge) {
  use from <- decode.field("from", decode.string)
  use to <- decode.field("to", decode.string)
  use kind <- decode.field("kind", decode.string)
  use direct <- decode.optional_field("direct", False, decode.bool)
  use known <- decode.optional_field("known", "derived", decode.string)
  decode.success(Edge(from, to, kind, direct, known))
}

fn item_decoder() -> decode.Decoder(Item) {
  use id <- decode.field("id", decode.string)
  use change <- decode.optional_field("change", "changed", decode.string)
  use before <- decode.optional_field("before", "", decode.string)
  use after <- decode.optional_field("after", "", decode.string)
  use added <- decode.optional_field(
    "effects_added",
    [],
    decode.list(decode.string),
  )
  use removed <- decode.optional_field(
    "effects_removed",
    [],
    decode.list(decode.string),
  )
  use tier <- decode.optional_field("tier", "", decode.string)
  decode.success(Item(id, change, before, after, added, removed, tier))
}

fn test_decoder() -> decode.Decoder(Test) {
  use id <- decode.field("id", decode.string)
  use name <- decode.field("name", decode.string)
  use result <- decode.field("result", decode.string)
  use message <- decode.optional_field("message", "", decode.string)
  use checks <- decode.optional_field(
    "checks",
    [],
    decode.list(check_decoder()),
  )
  decode.success(Test(id, name, result, message, checks))
}

fn review_decoder() -> decode.Decoder(Review) {
  use summary <- decode.field("summary", decode.string)
  use entries <- decode.field("entries", decode.list(item_decoder()))
  use signatures <- decode.field("signatures", decode.list(item_decoder()))
  use tests <- decode.field("tests", decode.list(test_decoder()))
  use files <- decode.field("files", decode.list(item_decoder()))
  use added <- decode.field(
    "effects",
    decode.at(["added"], decode.list(decode.string)),
  )
  use removed <- decode.field(
    "effects",
    decode.at(["removed"], decode.list(decode.string)),
  )
  use untested <- decode.optional_field(
    "untested",
    [],
    decode.list(untested_decoder()),
  )
  use suites <- decode.optional_field("suites", [], decode.list(decode.string))
  use leaves <- decode.optional_field("leaves", [], decode.list(decode.string))
  let reaches =
    list.append(
      case suites {
        [] -> []
        _ -> ["Also reached: " <> string.join(suites, ", ")]
      },
      case leaves {
        [] -> []
        _ -> ["Leaves the program: " <> string.join(leaves, ", ")]
      },
    )
  decode.success(Review(
    summary,
    entries,
    signatures,
    tests,
    files,
    added,
    removed,
    untested,
    reaches,
  ))
}

/// A use the change reaches that no test checks: (node id, line to show).
fn untested_decoder() -> decode.Decoder(#(String, String)) {
  use id <- decode.field("id", decode.string)
  use name <- decode.field("name", decode.string)
  use steps <- decode.field("steps", decode.int)
  use via <- decode.field("via", decode.string)
  use certain <- decode.field("certain", decode.bool)
  let unit = case steps {
    1 -> " step"
    _ -> " steps"
  }
  let possible = case certain {
    True -> ""
    False -> " (possible)"
  }
  decode.success(#(
    id,
    name <> " · " <> int.to_string(steps) <> unit <> " · " <> via <> possible,
  ))
}

pub fn graph_decoder() -> decode.Decoder(Graph) {
  use nodes <- decode.field("nodes", decode.list(node_decoder()))
  use edges <- decode.field("edges", decode.list(edge_decoder()))
  use review <- decode.optional_field(
    "review",
    None,
    decode.map(review_decoder(), Some),
  )
  use project <- decode.optional_field(
    "project",
    "",
    decode.at(["name"], decode.string),
  )
  use fingerprint <- decode.optional_field(
    "project",
    "",
    decode.at(["fingerprint"], decode.string),
  )
  use areas <- decode.optional_field(
    "areas",
    [],
    decode.list({
      use id <- decode.field("id", decode.string)
      use name <- decode.field("name", decode.string)
      use summary <- decode.optional_field("summary", "", decode.string)
      decode.success(Area(id, name, summary))
    }),
  )
  decode.success(Graph(nodes, edges, review, project, fingerprint, areas))
}

// ponytail: fixed threshold; make it depend on the sheet's size if small projects want areas too.
/// Above this many entry points, the overview folds them into one box per
/// area (D-55.6); an area box contains its entry points and reaches what they reach.
const fold_above = 12

pub fn with_areas(graph: Graph) -> Graph {
  let entries =
    list.filter(graph.nodes, fn(n) { n.kind == "entry" && n.module != "" })
  case list.length(entries) > fold_above {
    False -> graph
    True -> {
      let modules = list.unique(list.map(entries, fn(n) { n.module }))
      let boxes =
        list.map(modules, fn(m) {
          let area =
            list.find(graph.areas, fn(a) { a.id == m })
            |> result.unwrap(Area(m, m, ""))
          Node(
            ..made("area:" <> m, "area", area.name, m),
            summary: area.summary,
          )
        })
      let contains =
        list.map(entries, fn(n) {
          Edge("area:" <> n.module, n.id, "contains", True, "derived")
        })
      let reaches =
        list.filter_map(graph.edges, fn(edge) {
          case edge.kind, list.find(entries, fn(n) { n.id == edge.from }) {
            "uses", Ok(n) ->
              Ok(Edge("area:" <> n.module, edge.to, "uses", False, "derived"))
            _, _ -> Error(Nil)
          }
        })
        |> list.unique
      Graph(
        ..graph,
        nodes: list.append(boxes, graph.nodes),
        edges: list.flatten([graph.edges, contains, reaches]),
      )
    }
  }
}

fn point_decoder() -> decode.Decoder(Point) {
  use id <- decode.field("id", decode.string)
  use x <- decode.field("x", decode.float)
  use y <- decode.field("y", decode.float)
  decode.success(Point(id, x, y))
}

pub fn overview(graph: Graph) -> List(String) {
  let front = case list.any(graph.nodes, fn(n) { n.kind == "area" }) {
    True -> "area"
    False -> "entry"
  }
  graph.nodes
  |> list.filter(fn(n) { n.kind == front || n.kind == "effect" })
  |> list.map(fn(n) { n.id })
}

pub fn reachability(graph: Graph) -> List(Edge) {
  list.filter(graph.edges, fn(edge) {
    edge.kind == "uses"
    && case list.find(graph.nodes, fn(node) { node.id == edge.from }) {
      Ok(node) -> node.kind == "entry" || node.kind == "area"
      Error(_) -> False
    }
  })
}

pub fn neighbors(graph: Graph, id: String) -> List(String) {
  graph.edges
  |> list.filter_map(fn(edge) {
    case edge.from == id, edge.to == id {
      True, _ -> Ok(edge.to)
      _, True -> Ok(edge.from)
      _, _ -> Error(Nil)
    }
  })
}

pub fn toggle(
  graph: Graph,
  visible: List(String),
  expanded: List(String),
  id: String,
) -> #(List(String), List(String)) {
  case list.contains(expanded, id) {
    True -> {
      let expanded = list.filter(expanded, fn(x) { x != id })
      let kept =
        list.flatten([
          overview(graph),
          expanded,
          list.flat_map(expanded, fn(x) { neighbors(graph, x) }),
        ])
      #(list.filter(visible, fn(x) { list.contains(kept, x) }), expanded)
    }
    // A node with no connections has nothing to expand.
    False ->
      case neighbors(graph, id) {
        [] -> #(visible, expanded)
        others -> #(list.unique(list.append(visible, others)), [id, ..expanded])
      }
  }
}

fn get(model: Model, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(model.prefix, path, "GET", "", fn(text) { send(msg(text)) })
  })
}

fn visible_edges(model: Model) -> List(Edge) {
  let visible = fn(edge: Edge) {
    list.contains(model.visible, edge.from)
    && list.contains(model.visible, edge.to)
  }
  let folded = list.any(model.graph.nodes, fn(n) { n.kind == "area" })
  case model.expanded {
    // Area boxes sit in a grid and say what they reach on their face, instead of a fan of connectors.
    [] if folded -> []
    [] -> list.filter(reachability(model.graph), visible)
    // Only connectors that touch an opened node; the rest are listed in each node's panel.
    opened ->
      list.filter(model.graph.edges, fn(edge) {
        visible(edge)
        && {
          list.contains(opened, edge.from) || list.contains(opened, edge.to)
        }
      })
  }
}

fn layout(model: Model) -> Model {
  relayout(model, model.expanded == [])
}

/// `fresh`: lay everything out anew instead of keeping where nodes were drawn.
fn relayout(model: Model, fresh: Bool) -> Model {
  let nodes =
    list.filter(model.graph.nodes, fn(n) {
      list.contains(model.visible, n.id) && n.kind != "file"
    })
  let node_json =
    json.array(nodes, fn(n) { json.object([#("id", json.string(n.id))]) })
    |> json.to_string
  let edge_json =
    json.array(visible_edges(model), fn(edge) {
      json.object([
        #("from", json.string(edge.from)),
        #("to", json.string(edge.to)),
        #("kind", json.string(edge.kind)),
      ])
    })
    |> json.to_string
  case
    json.parse(
      placement(
        node_json,
        edge_json,
        fresh,
        case model.expanded {
          [anchor, ..] -> anchor
          [] -> ""
        },
        model.root,
        model.prefix,
      ),
      decode.list(point_decoder()),
    )
  {
    Ok(points) -> Model(..model, positions: points)
    Error(_) -> model
  }
}

fn fit_after_render() -> Effect(Msg) {
  effect.from(fn(send) { after_draw(fn() { send(Fit) }) })
}

fn fit(model: Model) -> Model {
  let points =
    json.array(model.positions, fn(p) {
      json.object([#("x", json.float(p.x)), #("y", json.float(p.y))])
    })
    |> json.to_string
  let decoder = {
    use zoom <- decode.field("zoom", decode.float)
    use x <- decode.field("x", decode.float)
    use y <- decode.field("y", decode.float)
    decode.success(#(zoom, x, y))
  }
  case json.parse(fitted(points, model.pane, model.root), decoder) {
    Ok(#(zoom, x, y)) -> Model(..model, zoom: zoom, pan_x: x, pan_y: y)
    Error(_) -> model
  }
}

fn review_needs_tests(review: Review) -> Bool {
  string.contains(review.summary, "tests not run yet")
}

fn file_decoder() -> decode.Decoder(FileText) {
  use path <- decode.field("path", decode.string)
  use category <- decode.field("category", decode.string)
  use text <- decode.field("text", decode.optional(decode.string))
  use size <- decode.field("size", decode.int)
  use truncated <- decode.field("truncated", decode.bool)
  let note = case text, truncated {
    None, _ ->
      "Not text (" <> int.to_string(size) <> " bytes), so it isn't shown."
    _, True -> "Showing the first 1 MiB of " <> int.to_string(size) <> " bytes."
    _, False -> ""
  }
  decode.success(FileText(path, category, text, note))
}

fn source_decoder() -> decode.Decoder(#(String, List(#(String, String)))) {
  let line = {
    use op <- decode.field(0, decode.string)
    use text <- decode.field(1, decode.string)
    decode.success(#(op, text))
  }
  use source <- decode.field("source", decode.string)
  use diff <- decode.optional_field("diff", [], decode.list(line))
  decode.success(#(source, diff))
}

/// Zooms about the middle of the drawing, so what is in view stays in view.
fn zoom(model: Model, amount: Float) -> Model {
  let zoom = float.clamp(model.zoom *. amount, 0.02, 2.5)
  let half_w = viewport_width(model.root) /. 2.0
  let half_h = viewport_height(model.root) /. 2.0
  Model(
    ..model,
    zoom: zoom,
    pan_x: model.pan_x +. half_w /. model.zoom -. half_w /. zoom,
    pan_y: model.pan_y +. half_h /. model.zoom -. half_h /. zoom,
  )
}

/// Pans the least distance that brings a node into view; a node already in view
/// doesn't move. `panel`: the detail panel covers the right of a wide sheet.
fn reveal(model: Model, id: String, panel: Bool) -> Model {
  let Point(_, x, y) = point(model, id)
  let width = case panel && viewport_width(model.root) >. 650.0 {
    True -> viewport_width(model.root) -. panel_width()
    False -> viewport_width(model.root)
  }
  let w = width /. model.zoom
  let h = viewport_height(model.root) /. model.zoom
  let into = fn(pan, at, size, view) {
    case at -. 16.0 <. pan, at +. size +. 16.0 >. pan +. view {
      True, _ -> at -. 16.0
      _, True -> at +. size +. 16.0 -. view
      _, _ -> pan
    }
  }
  Model(
    ..model,
    pan_x: into(model.pan_x, x, 220.0, w),
    pan_y: into(model.pan_y, y, 120.0, h),
  )
}

/// The nearest node from `from` in a direction, preferring ones in line with it.
pub fn nearest(
  points: List(Point),
  from: Point,
  direction: String,
) -> Result(String, Nil) {
  points
  |> list.filter_map(fn(p) {
    let dx = p.x -. from.x
    let dy = p.y -. from.y
    let #(ahead, aside) = case direction {
      "left" -> #(0.0 -. dx, dy)
      "right" -> #(dx, dy)
      "up" -> #(0.0 -. dy, dx)
      _ -> #(dy, dx)
    }
    case p.id != from.id && ahead >. 1.0 {
      True -> Ok(#(ahead +. 2.0 *. float.absolute_value(aside), p.id))
      False -> Error(Nil)
    }
  })
  |> list.sort(fn(a, b) { float.compare(a.0, b.0) })
  |> list.first
  |> result.map(fn(p) { p.1 })
}

fn keyboard(model: Model, key: String) -> #(Model, Effect(Msg)) {
  let step = 96.0 /. model.zoom
  let has_panel = model.pane
  let focused = list.any(model.positions, fn(p) { p.id == model.focus })
  let go = fn(model: Model, id) {
    #(
      Model(..reveal(model, id, has_panel), focus: id),
      effect.from(fn(_) { focus_node(model.root, id) }),
    )
  }
  case key, focused {
    "pan-left", _ -> #(
      Model(..model, pan_x: model.pan_x -. step),
      effect.none(),
    )
    "pan-right", _ -> #(
      Model(..model, pan_x: model.pan_x +. step),
      effect.none(),
    )
    "pan-up", _ -> #(Model(..model, pan_y: model.pan_y -. step), effect.none())
    "pan-down", _ -> #(
      Model(..model, pan_y: model.pan_y +. step),
      effect.none(),
    )
    "expand-all", _ -> update(model, ExpandAll)
    "collapse-all", _ -> update(model, CollapseAll)
    "zoom-in", _ -> #(zoom(model, 1.2), effect.none())
    "zoom-out", _ -> #(zoom(model, 0.833333), effect.none())
    "left", False | "right", False | "up", False | "down", False ->
      // Nothing focused yet: start from the node nearest the sheet's top left.
      case
        nearest(
          model.positions,
          Point("", model.pan_x, model.pan_y -. 2.0),
          "down",
        )
      {
        Ok(id) -> go(model, id)
        Error(_) -> #(model, effect.none())
      }
    "left", True | "right", True | "up", True | "down", True ->
      case nearest(model.positions, point(model, model.focus), key) {
        Ok(id) -> go(model, id)
        Error(_) -> #(model, effect.none())
      }
    "expand", True ->
      case list.contains(model.expanded, model.focus) {
        True -> #(model, effect.none())
        False -> {
          let #(model, _) = update(model, Toggle(model.focus))
          go(model, model.focus)
        }
      }
    "collapse", True ->
      case list.contains(model.expanded, model.focus) {
        True -> {
          let #(model, _) = update(model, Toggle(model.focus))
          go(model, model.focus)
        }
        False -> #(model, effect.none())
      }
    "open", True -> update(model, Select(model.focus))
    _, _ -> #(model, effect.none())
  }
}

/// Opens this instance's transport with its own `base`: the graph, and for a
/// task the event stream that reloads it. Runs on connection and when `base`
/// changes.
fn start(model: Model) -> #(Model, Effect(Msg)) {
  case model.source_handle {
    Some(source) -> close_events(source)
    None -> Nil
  }
  let model = Model(..model, source_handle: None)
  #(
    model,
    effect.batch([
      get(
        model,
        case model.task {
          "" -> "/graph"
          task -> "/graph?task=" <> task
        },
        Loaded,
      ),
      case model.task {
        "" -> effect.none()
        _ ->
          effect.from(fn(send) {
            send(Listening(events(model.prefix, fn(data) { send(Live(data)) })))
          })
      },
    ]),
  )
}

fn n(model: Model, id: String) -> Option(Node) {
  case list.find(model.graph.nodes, fn(node) { node.id == id }) {
    Ok(node) -> Some(node)
    Error(_) -> None
  }
}

fn point(model: Model, id: String) -> Point {
  list.find(model.positions, fn(p) { p.id == id })
  |> result.unwrap(Point(id, 0.0, 0.0))
}

fn num(x: Float) -> String {
  float.to_string(x)
}

fn ax(name: String, value: String) -> a.Attribute(Msg) {
  a.attribute(name, value)
}

/// A public function without a description, for the agent to describe (D-55.4).
pub fn undescribed(node: Node) -> Bool {
  node.public
  && node.summary == ""
  && { node.kind == "entry" || node.kind == "logic" }
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
        keys(root, fn(key) { send(Key(key)) })
        resizer(root)
      }),
    )
    Listening(source) -> #(
      Model(..model, source_handle: Some(source)),
      effect.none(),
    )
    Disconnected -> {
      case model.source_handle {
        Some(source) -> close_events(source)
        None -> Nil
      }
      #(Model(..model, source_handle: None, connected: False), effect.none())
    }
    // An attribute set before the element is added only takes effect then.
    BaseChanged(base) -> {
      let model = Model(..model, prefix: base)
      case model.connected {
        True -> start(model)
        False -> #(model, effect.none())
      }
    }
    // The address names another task, or none: the mounted page follows it.
    TaskChanged(task) ->
      case task == model.task {
        True -> #(model, effect.none())
        False -> {
          let model = fresh(model, task)
          case model.connected {
            True -> start(model)
            False -> #(model, effect.none())
          }
        }
      }
    Loaded(text) ->
      case json.parse(text, graph_decoder()) {
        Ok(graph) -> {
          let graph = with_areas(graph)
          let visible = case model.visible {
            [] -> overview(graph)
            _ -> model.visible
          }
          let next =
            layout(
              Model(
                ..model,
                graph: graph,
                visible: visible,
                error: "",
                tests_requested: model.tests_requested
                  || case graph.review {
                    Some(review) -> review_needs_tests(review)
                    None -> False
                  },
              ),
            )
          let next = case model.graph.nodes {
            [] -> fit(next)
            _ -> next
          }
          let run = case graph.review {
            Some(review) ->
              case review_needs_tests(review) && !model.tests_requested {
                True ->
                  effect.from(fn(send) {
                    request(
                      model.prefix,
                      "/tests",
                      "POST",
                      json.to_string(
                        json.object([#("task", json.string(model.task))]),
                      ),
                      fn(response) { send(TestsStarted(response)) },
                    )
                  })
                False -> effect.none()
              }
            _ -> effect.none()
          }
          #(
            next,
            effect.batch([
              run,
              case model.graph.nodes {
                [] -> fit_after_render()
                _ -> effect.none()
              },
            ]),
          )
        }
        Error(_) -> #(
          Model(
            ..model,
            error: "Graph could not be read. Reload this page to try again.",
          ),
          effect.none(),
        )
      }
    Select(id) -> {
      let #(visible, expanded) =
        toggle(
          model.graph,
          list.unique([id, ..model.visible]),
          model.expanded,
          id,
        )
      let next =
        layout(
          Model(
            ..model,
            selected: id,
            visible: visible,
            expanded: expanded,
            tab: "details",
            pane: True,
            file: None,
            source: "",
            diff: [],
            search: "",
            focus: id,
            outline: False,
          ),
        )
      let next = Model(..reveal(next, id, True), detail: n(model, id))
      case n(model, id) {
        // Removed nodes and area boxes have no details to fetch.
        Some(node) if node.removed || node.kind == "area" -> #(
          Model(..next, detail: Some(node)),
          effect.none(),
        )
        _ -> #(next, get(model, "/nodes/" <> id, Detail))
      }
    }
    Detail(text) ->
      case json.parse(text, node_decoder()) {
        Ok(node) -> #(
          Model(..model, detail: Some(node), error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, error: "Node details could not be read."),
          effect.none(),
        )
      }
    Click(id) ->
      case model.travel >. 4.0 {
        // The click that ends a pan selects nothing.
        True -> #(model, effect.none())
        False -> update(model, Select(id))
      }
    Focused(id) -> #(Model(..model, focus: id), effect.none())
    Key(key) -> keyboard(model, key)
    Source(text) ->
      case json.parse(text, source_decoder()) {
        Ok(#(source, diff)) -> #(
          Model(..model, source: source, diff: diff),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, error: "Source could not be read."),
          effect.none(),
        )
      }
    Search(text) -> #(Model(..model, search: text), effect.none())
    Toggle(id) -> {
      let #(visible, expanded) =
        toggle(model.graph, model.visible, model.expanded, id)
      #(
        layout(Model(..model, visible: visible, expanded: expanded)),
        effect.none(),
      )
    }
    Tab(tab) -> #(Model(..model, tab: tab, pane: True), effect.none())
    ExpandAll -> {
      let ids =
        model.graph.nodes
        |> list.filter(fn(node) { node.kind != "file" })
        |> list.map(fn(node) { node.id })
      let expanded =
        list.filter(ids, fn(id) { neighbors(model.graph, id) != [] })
      #(
        fit(relayout(Model(..model, visible: ids, expanded: expanded), True)),
        fit_after_render(),
      )
    }
    CollapseAll -> #(
      fit(layout(Model(..model, visible: overview(model.graph), expanded: []))),
      fit_after_render(),
    )
    OpenFile(path) -> #(model, get(model, "/files/" <> path, FileLoaded))
    FileLoaded(text) ->
      case json.parse(text, file_decoder()) {
        Ok(file) -> #(
          Model(..model, file: Some(file), error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, error: "That file could not be read."),
          effect.none(),
        )
      }
    CloseFile -> #(Model(..model, file: None), fit_after_render())
    Reset -> #(
      fit(layout(
        Model(
          ..model,
          visible: overview(model.graph),
          expanded: [],
          selected: "",
          detail: None,
          file: None,
          zoom: 1.0,
          pan_x: 0.0,
          pan_y: 0.0,
        ),
      )),
      fit_after_render(),
    )
    Fit -> #(fit(model), effect.none())
    Zoom(amount) -> #(zoom(model, amount), effect.none())
    PanStart(x, y) -> #(
      Model(..model, dragging: True, drag_x: x, drag_y: y, travel: 0.0),
      effect.none(),
    )
    PanMove(x, y) ->
      case model.dragging {
        True -> {
          let px = model.pan_x +. { model.drag_x -. x } /. model.zoom
          let py = model.pan_y +. { model.drag_y -. y } /. model.zoom
          let travel =
            model.travel
            +. float.absolute_value(x -. model.drag_x)
            +. float.absolute_value(y -. model.drag_y)
          #(
            Model(
              ..model,
              pan_x: px,
              pan_y: py,
              drag_x: x,
              drag_y: y,
              travel: travel,
            ),
            effect.none(),
          )
        }
        False -> #(model, effect.none())
      }
    PanEnd -> #(Model(..model, dragging: False), effect.none())
    RunTests -> #(
      model,
      effect.from(fn(send) {
        request(
          model.prefix,
          "/tests",
          "POST",
          json.to_string(json.object([#("task", json.string(model.task))])),
          fn(response) { send(TestsStarted(response)) },
        )
      }),
    )
    TestsStarted(_) -> #(model, effect.none())
    Live(text) ->
      case json.parse(text, decode.at(["task", "id"], decode.string)) {
        Ok(id) if id == model.task -> #(
          model,
          get(model, "/graph?task=" <> model.task, Loaded),
        )
        _ -> #(model, effect.none())
      }
    SelectSource(id) -> #(
      model,
      get(
        model,
        "/nodes/"
          <> id
          <> "?source=1"
          <> case model.task {
          "" -> ""
          task -> "&task=" <> task
        },
        Source,
      ),
    )
    // Closing folds the pane to its tabs; the selection and its details stay.
    Close -> #(Model(..model, pane: False), effect.none())
    ShowOutline(on) -> #(Model(..model, outline: on, file: None), case on {
      True -> effect.none()
      False -> fit_after_render()
    })
    // From the outline: open a function on the drawing, once the drawing is there to reveal it in.
    Show(id) -> {
      let #(next, fx) = update(model, Select(id))
      #(
        next,
        effect.batch([
          fx,
          effect.from(fn(send) { after_draw(fn() { send(Reveal(id)) }) }),
        ]),
      )
    }
    Reveal(id) -> #(reveal(model, id, model.pane), effect.none())
    AskForSummaries -> {
      let missing =
        list.filter(model.graph.nodes, undescribed)
        |> list.map(fn(node) { "- " <> node.module <> "." <> node.name })
      #(
        Model(..model, asked: "asking"),
        effect.from(fn(send) {
          request(
            model.prefix,
            "/tasks",
            "POST",
            json.to_string(
              json.object([
                #("title", json.string("Describe undocumented functions")),
                #(
                  "body",
                  json.string(
                    "The contract view shows each public function's doc comment as its summary. These have none. Give each a one-line doc comment saying what it does, in words someone who doesn't read code understands. Don't change any code.\n\n"
                    <> string.join(missing, "\n"),
                  ),
                ),
              ]),
            ),
            fn(response) { send(Asked(response)) },
          )
        }),
      )
    }
    Asked(text) ->
      case json.parse(text, decode.at(["id"], decode.string)) {
        Ok(_) -> #(Model(..model, asked: "asked"), effect.none())
        Error(_) -> #(
          Model(..model, asked: "", error: "The task could not be made."),
          effect.none(),
        )
      }
  }
}

pub fn connector(from: Point, to: Point) -> String {
  let Point(_, x1, y1) = from
  let Point(_, x2, y2) = to
  case x2 >=. x1 +. 240.0, x1 >=. x2 +. 240.0 {
    True, _ -> {
      let mid = { x1 +. 220.0 +. x2 } /. 2.0
      "M "
      <> num(x1 +. 220.0)
      <> " "
      <> num(y1 +. 60.0)
      <> " H "
      <> num(mid)
      <> " V "
      <> num(y2 +. 60.0)
      <> " H "
      <> num(x2)
    }
    _, True -> {
      let mid = { x1 +. x2 +. 220.0 } /. 2.0
      "M "
      <> num(x1)
      <> " "
      <> num(y1 +. 60.0)
      <> " H "
      <> num(mid)
      <> " V "
      <> num(y2 +. 60.0)
      <> " H "
      <> num(x2 +. 220.0)
    }
    _, _ -> {
      let start = case y2 >. y1 {
        True -> y1 +. 120.0
        False -> y1
      }
      let end = case y2 >. y1 {
        True -> y2
        False -> y2 +. 120.0
      }
      "M "
      <> num(x1 +. 110.0)
      <> " "
      <> num(start)
      <> " V "
      <> num({ start +. end } /. 2.0)
      <> " H "
      <> num(x2 +. 110.0)
      <> " V "
      <> num(end)
    }
  }
}

/// What a connector says, read along its arrow: "main calls work".
pub fn wire_text(model: Model, edge: Edge, overview: Bool) -> String {
  let name = fn(id) {
    case n(model, id) {
      Some(node) -> node.name
      None -> id
    }
  }
  let verb = case edge.kind, overview {
    "uses", True -> " reaches "
    "uses", False -> " uses "
    kind, _ -> " " <> kind <> " "
  }
  name(edge.from) <> verb <> name(edge.to)
}

fn line(model: Model, edge: Edge, overview: Bool) -> Element(Msg) {
  let d = connector(point(model, edge.from), point(model, edge.to))
  s.g([a.class("view-link")], [
    s.title([], [h.text(wire_text(model, edge, overview))]),
    // A wide invisible stroke so the connector is easy to point at.
    s.path([a.class("view-wire-hit"), ax("d", d)]),
    s.path([
      a.class("view-wire"),
      ax("d", d),
      ax("marker-end", "url(#arrow)"),
      ax("stroke-dasharray", case edge.known != "derived" && !overview {
        True -> ""
        False -> "6 5"
      }),
    ]),
  ])
}

/// How many of a node's connections aren't drawn yet.
pub fn hidden(model: Model, id: String) -> Int {
  neighbors(model.graph, id)
  |> list.unique
  |> list.filter(fn(other) { !list.contains(model.visible, other) })
  |> list.length
}

fn revision(model: Model, node: Node) -> String {
  case node.kind, model.graph.review {
    // An area is changed when anything it contains is.
    "area", Some(_) ->
      case
        list.any(model.graph.edges, fn(edge) {
          edge.kind == "contains"
          && edge.from == node.id
          && case n(model, edge.to) {
            Some(member) -> revision(model, member) != ""
            None -> False
          }
        })
      {
        True -> "B"
        False -> ""
      }
    _, _ -> plain_revision(model, node)
  }
}

fn plain_revision(model: Model, node: Node) -> String {
  case model.graph.review {
    Some(review) ->
      case
        node.removed,
        list.contains(review.added, node.id),
        list.any(review.entries, fn(item) {
          item.id == node.id && item.change == "added"
        }),
        node.changed
      {
        True, _, _, _ -> "C"
        _, True, _, _ -> "A"
        _, _, True, _ -> "A"
        _, _, _, True -> "B"
        _, _, _, _ -> ""
      }
    None -> ""
  }
}

fn box(model: Model, node: Node) -> Element(Msg) {
  let Point(_, x, y) = point(model, node.id)
  let label =
    node.kind
    <> ": "
    <> node.name
    <> ". "
    <> node.summary
    <> ". "
    <> face_line(model, node)
  let mark = revision(model, node)
  let classes =
    "view-node "
    <> node.kind
    <> case mark != "" {
      True -> " changed"
      False -> ""
    }
    <> case node.removed {
      True -> " removed"
      False -> ""
    }
    <> case model.selected == node.id {
      True -> " selected"
      False -> ""
    }
  let more = hidden(model, node.id)
  let expander = case
    list.contains(model.expanded, node.id),
    more,
    neighbors(model.graph, node.id)
  {
    True, _, _ -> [expander_mark("−", "Expanded; press C to collapse")]
    False, _, [] -> []
    False, 0, _ -> [
      expander_mark("+", "Press Space to draw this box's connections"),
    ]
    False, _, _ -> [
      expander_mark(
        "+" <> int.to_string(more),
        int.to_string(more) <> " more connections; press Space to expand",
      ),
    ]
  }
  s.g(
    [
      a.class(classes),
      ax("transform", "translate(" <> num(x) <> " " <> num(y) <> ")"),
      a.tabindex(0),
      a.role("button"),
      a.aria_label(label),
      a.data("node", node.id),
      e.on_click(Click(node.id)),
      e.on("focus", decode.success(Focused(node.id))),
    ],
    [
      s.title([], [h.text(label)]),
      // The keyboard focus ring: around the box only, whatever hangs off it.
      s.rect([
        a.class("view-ring"),
        ax("x", "-7"),
        ax("y", "-7"),
        ax("width", "234"),
        ax("height", "134"),
      ]),
      s.rect([a.class("view-box"), ax("width", "220"), ax("height", "120")]),
      s.rect([a.class("view-tab"), ax("width", "7"), ax("height", "120")]),
      // The kind is the band's colour, not a word on every box (D-55.7).
      s.foreign_object(
        [ax("x", "20"), ax("y", "22"), ax("width", "188"), ax("height", "92")],
        [
          h.div([a.class("view-face")], [
            h.p([a.class("view-name")], [h.text(node.name)]),
            h.p([a.class("view-summary")], [h.text(node.summary)]),
            h.p([a.class("view-io")], [h.text(face_line(model, node))]),
          ]),
        ],
      ),
      s.text(
        [
          a.class("view-badge"),
          ax("x", "205"),
          ax("y", "19"),
          ax("text-anchor", "end"),
        ],
        case mark {
          "" ->
            case
              node.tests == 0
              && { node.kind == "entry" || node.kind == "logic" }
            {
              True -> "NO TESTS"
              False -> ""
            }
          _ -> "REV " <> mark
        },
      ),
      ..expander
    ],
  )
}

/// What a function takes and gives, in words: "Store, id → Payment or an error".
pub fn io_line(node: Node) -> String {
  case node.takes, node.gives {
    [], "" -> ""
    [], gives -> "→ " <> gives
    takes, "" -> string.join(takes, ", ")
    takes, gives -> string.join(takes, ", ") <> " → " <> gives
  }
}

fn plural(count: Int, one: String, many: String) -> String {
  int.to_string(count)
  <> " "
  <> case count {
    1 -> one
    _ -> many
  }
}

/// A box's last line: an area's ways in, a test's checks, a function's inputs and output.
fn face_line(model: Model, node: Node) -> String {
  case node.kind {
    "area" ->
      string.join(
        [
          plural(
            list.count(model.graph.edges, fn(edge) {
              edge.kind == "contains" && edge.from == node.id
            }),
            "way in",
            "ways in",
          ),
          ..effects_of(model, node.id)
        ],
        " · ",
      )
    "test" ->
      case node.checks {
        [] -> ""
        checks -> plural(list.length(checks), "check", "checks")
      }
    _ -> io_line(node)
  }
}

/// The effects a node reaches, by name.
fn effects_of(model: Model, id: String) -> List(String) {
  list.filter_map(model.graph.edges, fn(edge) {
    case edge.kind == "uses" && edge.from == id, n(model, edge.to) {
      True, Some(effect) -> Ok(effect.name)
      _, _ -> Error(Nil)
    }
  })
  |> list.unique
}

/// A frame around the entry points an expanded area contains. The area's box stays
/// outside, joined by its connectors: framing both would take in boxes below the area.
fn frame(model: Model, area: Node) -> Element(Msg) {
  let members =
    list.filter_map(model.graph.edges, fn(edge) {
      case edge.kind == "contains" && edge.from == area.id {
        True ->
          case list.find(model.positions, fn(p) { p.id == edge.to }) {
            Ok(p) -> Ok(p)
            Error(_) -> Error(Nil)
          }
        False -> Error(Nil)
      }
    })
  let points = members
  let xs = list.map(points, fn(p) { p.x })
  let ys = list.map(points, fn(p) { p.y })
  let low = fn(values) { list.fold(values, 1.0e9, float.min) }
  let high = fn(values) { list.fold(values, -1.0e9, float.max) }
  let x = low(xs) -. 12.0
  let y = low(ys) -. 12.0
  s.g([a.class("view-frame")], [
    s.rect([
      ax("x", num(x)),
      ax("y", num(y)),
      ax("width", num(high(xs) +. 232.0 -. x)),
      ax("height", num(high(ys) +. 144.0 -. y)),
    ]),
  ])
}

/// A tab hanging under a box's bottom-right edge, clear of its border: "−" when
/// expanded, "+N" for N connections not shown, "+" when all are shown but not drawn.
fn expander_mark(text: String, title: String) -> Element(Msg) {
  s.g([a.class("view-expander"), ax("transform", "translate(198 133)")], [
    s.title([], [h.text(title)]),
    s.rect([
      ax("x", "-22"),
      ax("y", "-8"),
      ax("width", "44"),
      ax("height", "16"),
    ]),
    s.text([ax("text-anchor", "middle"), ax("y", "4")], text),
  ])
}

fn search_results(model: Model) -> List(Node) {
  case string.trim(model.search) {
    "" -> []
    term ->
      list.filter(model.graph.nodes, fn(node) {
        string.contains(
          string.lowercase(node.name <> " " <> node.summary),
          string.lowercase(term),
        )
      })
      |> list.take(8)
  }
}

fn check_line(check: Check) -> String {
  case check.call {
    "" -> check.text
    _ ->
      check.call
      <> " → "
      <> check.expected
      <> case check.was {
        "" -> ""
        old -> ", was " <> old
      }
  }
}

fn review_panel(model: Model, review: Review) -> List(Element(Msg)) {
  let name = fn(id) {
    case n(model, id) {
      Some(node) -> node.name
      None -> id
    }
  }
  let entries =
    list.map(review.entries, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.id))], [
        h.text(
          item.change
          <> " · "
          <> name(item.id)
          <> " + "
          <> string.join(list.map(item.effects_added, name), ", ")
          <> " − "
          <> string.join(list.map(item.effects_removed, name), ", "),
        ),
      ])
    })
  let signatures =
    list.map(review.signatures, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.id))], [
        h.text(name(item.id) <> ": " <> item.before <> " → " <> item.after),
      ])
    })
  let tests =
    list.map(review.tests, fn(t) {
      h.div(
        [a.class("view-review-test")],
        list.append(
          [
            h.button([a.class("view-row"), e.on_click(Select(t.id))], [
              h.text(
                t.name
                <> " · "
                <> case t.result {
                  "failed to compile" -> "wrong · failed to compile"
                  other -> other
                },
              ),
            ]),
            h.p([], [h.text(t.message)]),
          ],
          list.map(t.checks, fn(check) { h.p([], [h.text(check_line(check))]) }),
        ),
      )
    })
  let files =
    list.map(review.files, fn(item) {
      h.p([], [
        h.text(item.change <> " · " <> name(item.id) <> " · " <> item.tier),
      ])
    })
  let untested =
    list.map(review.untested, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.0))], [
        h.text(item.1),
      ])
    })
  list.flatten([
    [
      h.h2([], [h.text("Revision review")]),
      h.p([a.class("view-review-summary")], [h.text(review.summary)]),
      h.h3([], [h.text("Entry points & effects")]),
    ],
    entries,
    [
      h.p([], [
        h.text(
          "Effects added: "
          <> string.join(list.map(review.added, name), ", ")
          <> ". Removed: "
          <> string.join(list.map(review.removed, name), ", "),
        ),
      ]),
      h.h3([], [h.text("Changed signatures")]),
    ],
    signatures,
    [
      h.h3([], [h.text("Tests")]),
      h.button([e.on_click(RunTests)], [h.text("Run tests again")]),
    ],
    tests,
    [h.h3([], [h.text("Untested uses the change reaches")])],
    untested,
    list.map(review.reaches, fn(line) { h.p([], [h.text(line)]) }),
    [h.h3([], [h.text("Files touched")])],
    files,
  ])
}

/// A node's connections in groups named once (D-55.5, D-55.7): Calls, Called by, …
pub fn connection_groups(
  graph: Graph,
  id: String,
) -> List(#(String, List(String))) {
  [
    #("calls", True, "Calls"),
    #("calls", False, "Called by"),
    #("uses", True, "Effects"),
    #("uses", False, "Used by"),
    #("tests", True, "Tests"),
    #("tests", False, "Tested by"),
    #("contains", True, "Ways in"),
    #("contains", False, "Area"),
  ]
  |> list.filter_map(fn(group) {
    let #(kind, out, label) = group
    let ids =
      list.filter_map(graph.edges, fn(edge) {
        case edge.kind == kind, out && edge.from == id, !out && edge.to == id {
          True, True, _ -> Ok(edge.to)
          True, _, True -> Ok(edge.from)
          _, _, _ -> Error(Nil)
        }
      })
      |> list.unique
    case ids {
      [] -> Error(Nil)
      _ -> Ok(#(label, ids))
    }
  })
}

fn detail_panel(model: Model, node: Node) -> List(Element(Msg)) {
  let groups = connection_groups(model.graph, node.id)
  let connections =
    list.map(groups, fn(group) {
      h.details([a.class("view-group")], [
        h.summary([], [
          h.text(group.0 <> " (" <> int.to_string(list.length(group.1)) <> ")"),
        ]),
        ..list.map(group.1, fn(id) {
          h.button(
            [a.class("view-row"), e.on_click(Select(id))],
            case n(model, id) {
              Some(other) -> [
                h.text(other.name),
                h.small([], [h.text(other.summary)]),
              ]
              None -> [h.text(id)]
            },
          )
        })
      ])
    })
  let lines = fn(title, checks) {
    case checks {
      [] -> []
      _ -> [
        h.h3([], [h.text(title)]),
        h.ul(
          [a.class("view-examples")],
          list.map(checks, fn(check) { h.li([], [h.text(check_line(check))]) }),
        ),
      ]
    }
  }
  list.flatten([
    [
      h.h2([], [h.text(node.name)]),
      h.p([a.class("view-detail-kind")], [
        h.text(node.kind <> " · " <> node.known),
      ]),
      case node.summary {
        "" -> element.none()
        summary -> h.p([a.class("view-detail-summary")], [h.text(summary)])
      },
      case node.kind, io_line(node) {
        "area", _ | _, "" -> element.none()
        _, io -> h.p([a.class("view-detail-io")], [h.text(io)])
      },
      case groups {
        [] -> h.p([a.class("view-file-role")], [h.text("No connections.")])
        _ ->
          h.button([e.on_click(Toggle(node.id))], [
            h.text(case list.contains(model.expanded, node.id) {
              True -> "Collapse connections"
              False -> "Expand connections"
            }),
          ])
      },
    ],
    case node.requests {
      [] -> []
      requests -> [
        h.h3([], [
          h.text(
            "Requests it handles ("
            <> int.to_string(list.length(requests))
            <> ")",
          ),
        ]),
        requests_list(requests),
      ]
    },
    lines("Examples", node.examples),
    lines("Checks", node.checks),
    connections,
    case node.signature {
      "" -> []
      signature -> [
        h.h3([], [h.text("Signature")]),
        h.p([a.class("view-code")], [h.text(signature)]),
      ]
    },
    case
      node.removed
      && model.task == ""
      || node.kind == "area"
      || node.kind == "effect"
    {
      True -> []
      False -> [
        h.button([e.on_click(SelectSource(node.id))], [h.text("Show source")]),
        source_text(model),
      ]
    },
  ])
}

/// The pane's tabs; Review only for a task, Details once a node is opened.
fn pane(model: Model) -> List(Element(Msg)) {
  let tabs =
    list.flatten([
      case model.graph.review {
        Some(_) -> [#("review", "Review")]
        None -> []
      },
      case model.detail {
        Some(_) -> [#("details", "Details")]
        None -> []
      },
      [#("files", "Files")],
    ])
  let current = case list.any(tabs, fn(tab) { tab.0 == model.tab }) {
    True -> model.tab
    False -> "files"
  }
  let strip =
    h.div(
      [a.class("view-tabs"), a.role("tablist"), a.aria_label("Panel")],
      list.append(
        list.map(tabs, fn(tab) {
          h.button(
            [
              a.role("tab"),
              ax("aria-selected", case model.pane && tab.0 == current {
                True -> "true"
                False -> "false"
              }),
              e.on_click(Tab(tab.0)),
            ],
            [h.text(tab.1)],
          )
        }),
        case model.pane {
          True -> [
            h.button([a.class("view-tabs-close"), e.on_click(Close)], [
              h.text("Close"),
            ]),
          ]
          False -> []
        },
      ),
    )
  case model.pane {
    False -> [h.nav([a.class("view-pane-folded")], [strip])]
    True -> [
      grip(),
      h.aside([a.class("view-panel"), a.role("tabpanel")], [
        strip,
        ..case current, model.detail, model.graph.review {
          "review", _, Some(review) -> review_panel(model, review)
          "details", Some(node), _ -> detail_panel(model, node)
          _, _, _ -> files_panel(model)
        }
      ]),
    ]
  }
}

/// A file's text, read-only, where the drawing was.
fn file_view(file: FileText) -> Element(Msg) {
  h.section([a.class("view-reader"), a.aria_label("File " <> file.path)], [
    h.div([a.class("view-reader-head")], [
      h.div([], [
        h.h2([], [h.text(file.path)]),
        h.p([], [
          h.text(string.capitalise(category(file.category)) <> " · read-only"),
        ]),
      ]),
      h.button([e.on_click(CloseFile)], [h.text("Back to the drawing")]),
    ]),
    case file.note {
      "" -> element.none()
      note -> h.p([a.class("view-reader-note")], [h.text(note)])
    },
    case file.text {
      Some(text) ->
        h.pre(
          [a.class("view-reader-text")],
          list.map(string.split(text, "\n"), fn(line) {
            h.span([], [h.text(line)])
          }),
        )
      None -> element.none()
    },
  ])
}

/// What the project does, by area, before any drawing (D-55.1).
fn outline_view(model: Model) -> Element(Msg) {
  let functions =
    list.filter(model.graph.nodes, fn(node) {
      node.public
      && !node.removed
      && { node.kind == "entry" || node.kind == "logic" }
    })
  let main =
    list.filter(functions, fn(node) {
      node.kind == "entry" && { node.requests != [] || node.name == "main" }
    })
  let areas =
    list.filter_map(model.graph.areas, fn(area) {
      // Entry points first, then the rest, each in name order.
      let #(entries, rest) =
        list.filter(functions, fn(node) { node.module == area.id })
        |> list.partition(fn(node) { node.kind == "entry" })
      case list.append(entries, rest) {
        [] -> Error(Nil)
        members -> Ok(#(area, members))
      }
    })
  let missing = list.count(model.graph.nodes, undescribed)
  h.section([a.class("view-outline"), a.aria_label("What the project does")], [
    h.div(
      [a.class("view-outline-body")],
      list.flatten([
        [
          h.h2([], [h.text("What " <> model.graph.project <> " does")]),
          h.p([a.class("view-outline-lead")], [
            h.text(
              plural(list.length(functions), "function", "functions")
              <> " in "
              <> plural(list.length(areas), "area", "areas")
              <> ". Open one to see it on the drawing.",
            ),
          ]),
        ],
        case missing, model.asked {
          0, _ -> []
          _, "asked" -> [
            h.p([a.class("view-outline-notice")], [
              h.text(
                "Task added to the board: describe the undocumented functions.",
              ),
            ]),
          ]
          _, asked -> [
            h.div([a.class("view-outline-notice")], [
              h.p([], [
                h.text(
                  plural(
                    missing,
                    "public function has",
                    "public functions have",
                  )
                  <> " no description yet.",
                ),
              ]),
              h.button(
                [e.on_click(AskForSummaries), a.disabled(asked == "asking")],
                [h.text("Ask an agent to describe them")],
              ),
            ]),
          ]
        },
        case main {
          [] -> []
          _ -> [
            h.h3([], [h.text("Main ways in")]),
            ..list.flat_map(main, fn(node) {
              [
                outline_row(model, node),
                case node.requests {
                  [] -> element.none()
                  requests ->
                    h.div([a.class("view-outline-requests")], [
                      h.p([], [
                        h.text(
                          "Requests it handles ("
                          <> int.to_string(list.length(requests))
                          <> ")",
                        ),
                      ]),
                      requests_list(requests),
                    ])
                },
              ]
            })
          ]
        },
        [h.h3([], [h.text("By area")])],
        list.map(areas, fn(pair) {
          let #(area, members) = pair
          let unchecked = list.count(members, fn(node) { node.tests == 0 })
          h.details([a.class("view-area")], [
            h.summary([], [
              h.span([a.class("view-area-name")], [h.text(area.name)]),
              h.span([a.class("view-area-counts")], [
                h.text(
                  plural(list.length(members), "function", "functions")
                  <> case unchecked {
                    0 -> ""
                    _ -> " · " <> int.to_string(unchecked) <> " not checked"
                  },
                ),
              ]),
              case area.summary {
                "" -> element.none()
                summary ->
                  h.span([a.class("view-area-summary")], [h.text(summary)])
              },
            ]),
            ..list.map(members, fn(node) { outline_row(model, node) })
          ])
        }),
      ]),
    ),
  ])
}

fn drawing_sheet(model: Model) -> Element(Msg) {
  h.div([a.class("view-sheet")], [
    h.div([a.class("view-sheet-head")], [
      legend(),
      h.p([a.class("view-drawing-note")], [
        h.text(
          case
            list.any(model.graph.nodes, fn(node) {
              node.kind == "effect" && !node.removed
            })
          {
            True -> ""
            False ->
              case model.graph.nodes {
                [] -> "Loading the project drawing…"
                _ ->
                  "No effects found: nothing here prints, writes files or talks to the network (derived)"
              }
          },
        ),
      ]),
      h.div([a.class("view-tools")], [
        h.button([e.on_click(Zoom(1.2)), a.aria_label("Zoom in")], [
          h.text("+"),
        ]),
        h.button([e.on_click(Zoom(0.833333)), a.aria_label("Zoom out")], [
          h.text("−"),
        ]),
        icon_button(
          ExpandAll,
          "Expand all (Shift+Space)",
          "M7 9l5-5 5 5M7 15l5 5 5-5",
        ),
        icon_button(
          CollapseAll,
          "Collapse all (Shift+C)",
          "M7 4l5 5 5-5M7 20l5-5 5 5",
        ),
        icon_button(
          Reset,
          "Back to the default view",
          "M3 11.5 12 4l9 7.5M5.5 9.5V20h5v-6h3v6h5V9.5",
        ),
      ]),
    ]),
    h.div([a.class("view-drawing")], [svg_view(model)]),
  ])
}

fn requests_list(requests: List(Capability)) -> Element(Msg) {
  h.ul(
    [a.class("view-requests")],
    list.map(requests, fn(request) {
      h.li([], [
        h.text(request.name),
        case request.fields {
          [] -> element.none()
          fields -> h.small([], [h.text(" " <> string.join(fields, ", "))])
        },
      ])
    }),
  )
}

/// The source, or its diff from before the task when the task changed it.
fn source_text(model: Model) -> Element(Msg) {
  case list.any(model.diff, fn(line) { line.0 != " " }) {
    False -> h.pre([a.class("view-source")], [h.text(model.source)])
    True ->
      h.pre(
        [a.class("view-source view-diff"), a.aria_label("Changes by this task")],
        list.map(model.diff, fn(line) {
          let #(class, mark) = case line.0 {
            "+" -> #("added", "+ ")
            "-" -> #("removed", "− ")
            _ -> #("same", "  ")
          }
          h.span([a.class(class)], [h.text(mark <> line.1)])
        }),
      )
  }
}

/// The panel's left edge: drag it, or focus it and press Left/Right, to resize the panel.
fn grip() -> Element(Msg) {
  h.div(
    [
      a.class("view-panel-grip"),
      a.role("separator"),
      a.tabindex(0),
      a.aria_label("Resize panel"),
      ax("aria-orientation", "vertical"),
    ],
    [],
  )
}

/// Files grouped by what Solace does with them, each group explained once.
fn files_panel(model: Model) -> List(Element(Msg)) {
  let files = list.filter(model.graph.nodes, fn(node) { node.kind == "file" })
  let open = case model.file {
    Some(file) -> file.path
    None -> ""
  }
  [
    h.h2([], [h.text("Project files")]),
    ..list.flat_map(
      [
        #(
          "watched",
          "Watched",
          "Agents can change these; undoing a task restores them.",
        ),
        #(
          "protected",
          "Protected",
          "Agents can read these; their changes wait for your approval.",
        ),
        #(
          "unwatched",
          "Unwatched",
          "Solace doesn't search these or undo commands' changes to them.",
        ),
      ],
      fn(group) {
        case list.filter(files, fn(node) { category(node.tier) == group.0 }) {
          [] -> []
          members -> [
            h.h3([], [h.text(group.1)]),
            h.p([a.class("view-file-role")], [h.text(group.2)]),
            h.ul(
              [a.class("view-files")],
              list.map(members, fn(node) { file_row(node, open) }),
            ),
          ]
        }
      },
    )
  ]
}

/// "unwatched (binary)" and "unwatched; 495 files" are both unwatched.
pub fn category(tier: String) -> String {
  case string.split(tier, " ") {
    [first, ..] -> string.replace(first, ";", "")
    [] -> tier
  }
}

fn file_row(node: Node, open: String) -> Element(Msg) {
  let extra = string.drop_start(node.tier, string.length(category(node.tier)))
  let extra = string.trim(string.replace(extra, ";", ""))
  let extra = string.replace(string.replace(extra, "(", ""), ")", "")
  let label = case extra {
    "" -> [h.text(node.name)]
    _ -> [h.text(node.name <> " "), h.small([], [h.text(extra)])]
  }
  case string.ends_with(node.name, "/") {
    // A collapsed directory: a count, nothing to open.
    True -> h.li([a.class("view-files-dir")], label)
    False ->
      h.li([], [
        h.button(
          [
            a.class("view-row"),
            ax("aria-current", case node.name == open {
              True -> "true"
              False -> "false"
            }),
            e.on_click(OpenFile(node.name)),
          ],
          label,
        ),
      ])
  }
}

/// One function in the outline: name, summary, then inputs, effects and whether tests check it.
fn outline_row(model: Model, node: Node) -> Element(Msg) {
  let tested = case node.tests {
    0 -> "not checked"
    count -> "checked by " <> plural(count, "test", "tests")
  }
  let meta =
    [io_line(node), ..list.append(effects_of(model, node.id), [tested])]
    |> list.filter(fn(part) { part != "" })
  h.button([a.class("view-outline-row"), e.on_click(Show(node.id))], [
    h.span([a.class("view-outline-name")], [h.text(node.name)]),
    case node.summary {
      "" -> element.none()
      summary -> h.span([a.class("view-outline-summary")], [h.text(summary)])
    },
    h.span([a.class("view-outline-meta")], [h.text(string.join(meta, " · "))]),
  ])
}

fn legend() -> Element(Msg) {
  h.aside([a.class("view-legend"), a.aria_label("Diagram legend")], [
    h.div([a.class("view-legend-desktop")], [
      h.h2([], [h.text("Legend")]),
      legend_items(),
    ]),
    h.details([a.class("view-legend-mobile")], [
      h.summary([], [h.text("Legend")]),
      legend_items(),
    ]),
  ])
}

fn legend_items() -> Element(Msg) {
  h.div([a.class("view-legend-content")], [
    h.div(
      [a.class("view-legend-kinds")],
      list.map(["area", "entry", "logic", "effect", "test", "file"], fn(kind) {
        h.span([a.class("view-legend-kind")], [
          h.span(
            [a.class("view-swatch " <> kind), ax("aria-hidden", "true")],
            [],
          ),
          h.text(string.capitalise(kind)),
        ])
      }),
    ),
    h.div([a.class("view-legend-lines")], [
      h.span([], [
        s.svg([ax("viewBox", "0 0 40 16"), ax("aria-hidden", "true")], [
          s.path([a.class("view-wire"), ax("d", "M 1 8 H 39")]),
        ]),
        h.text("Observed / declared"),
      ]),
      h.span([], [
        s.svg([ax("viewBox", "0 0 40 16"), ax("aria-hidden", "true")], [
          s.path([
            a.class("view-wire"),
            ax("d", "M 1 8 H 39"),
            ax("stroke-dasharray", "6 5"),
          ]),
        ]),
        h.text("Derived"),
      ]),
      h.span([], [
        s.svg([ax("viewBox", "0 0 40 20"), ax("aria-hidden", "true")], [
          s.rect([
            a.class("view-revision-sample"),
            ax("x", "2"),
            ax("y", "3"),
            ax("width", "36"),
            ax("height", "14"),
          ]),
        ]),
        h.text("Changed by the task: REV A added, B changed, C removed"),
      ]),
      h.span([], [
        h.text("→ points from a function to what it calls, uses or tests"),
      ]),
      h.span([], [
        h.text("+3: expand to show 3 more connections · −: expanded"),
      ]),
    ]),
  ])
}

fn icon_button(msg: Msg, label: String, path: String) -> Element(Msg) {
  h.button(
    [
      e.on_click(msg),
      a.class("view-icon"),
      a.aria_label(label),
      ax("title", label),
    ],
    [
      s.svg([ax("viewBox", "0 0 24 24"), ax("aria-hidden", "true")], [
        s.path([ax("d", path)]),
      ]),
    ],
  )
}

fn mouse_decoder(make: fn(Float, Float) -> Msg) -> decode.Decoder(Msg) {
  use x <- decode.field("clientX", decode.float)
  use y <- decode.field("clientY", decode.float)
  decode.success(make(x, y))
}

fn svg_view(model: Model) -> Element(Msg) {
  let visible =
    list.filter(model.graph.nodes, fn(node) {
      list.contains(model.visible, node.id) && node.kind != "file"
    })
  let edges = visible_edges(model)
  let frames =
    list.filter_map(model.expanded, fn(id) {
      case n(model, id) {
        Some(node) if node.kind == "area" -> Ok(frame(model, node))
        _ -> Error(Nil)
      }
    })
  let elements =
    list.flatten([
      frames,
      list.map(edges, fn(edge) { line(model, edge, model.expanded == []) }),
      list.map(visible, fn(node) { box(model, node) }),
    ])
  s.svg(
    [
      a.class("view-diagram"),
      ax("preserveAspectRatio", "xMinYMin meet"),
      a.aria_label(
        "Project contract diagram; select a node to read its details",
      ),
      ax(
        "viewBox",
        num(model.pan_x)
          <> " "
          <> num(model.pan_y)
          <> " "
          <> num(viewport_width(model.root) /. model.zoom)
          <> " "
          <> num(viewport_height(model.root) /. model.zoom),
      ),
      e.on("pointerdown", mouse_decoder(PanStart)),
      e.on("pointermove", mouse_decoder(PanMove)),
      e.on("pointerup", decode.success(PanEnd)),
      e.on("pointerleave", decode.success(PanEnd)),
      e.prevent_default(e.on(
        "wheel",
        decode.map(decode.at(["deltaY"], decode.float), fn(delta) {
          case delta >. 0.0 {
            True -> Zoom(0.9)
            False -> Zoom(1.1)
          }
        }),
      )),
    ],
    [
      s.defs([], [
        s.marker(
          [
            ax("id", "arrow"),
            ax("viewBox", "0 0 10 10"),
            ax("refX", "9"),
            ax("refY", "5"),
            ax("markerWidth", "7"),
            ax("markerHeight", "7"),
            ax("orient", "auto"),
          ],
          [s.path([ax("d", "M 1 1 L 9 5 L 1 9"), a.class("view-arrow")])],
        ),
      ]),
      ..elements
    ],
  )
}

const view_styles = ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a;--focus:#174f78;--well:#eceee6;--hover:#e7eeea;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--edge:var(--ink);--sun:none;--moon:inline-block}\n.view-page{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:var(--contracts-height,100dvh);position:relative;background:var(--sheet);color:var(--ink)}\n.view-page *{box-sizing:border-box;scrollbar-color:var(--line) var(--paper)}.view-page input{caret-color:var(--focus)}.view-page input::placeholder{color:var(--soft);opacity:1}.view-page a{text-underline-offset:3px}.view-page ::selection{background:var(--sel);color:var(--ink)}\n.view-page button,.view-page input{font:inherit;border-radius:0;margin:0;color:var(--ink)}\n.view-page button{background:var(--paper);border:1px solid var(--edge);padding:6px 12px}.view-page button:hover{background:var(--hover)}.view-page button:focus-visible,.view-page input:focus-visible,.view-node:focus-visible{outline:3px solid var(--focus);outline-offset:3px}\n.view-page h1,.view-page h2,.view-page h3{line-height:1.2;letter-spacing:-.02em}.view-page h1{font-size:28px;margin:0}.view-page h2{font-size:19px;margin:0 0 10px}.view-page h3{font-size:15px;margin:24px 0 8px;text-transform:uppercase;letter-spacing:.06em}\n.view-page p{margin:8px 0}.view-top{height:100px;padding:12px 22px;display:flex;gap:30px;align-items:center;border-bottom:2px solid var(--edge);background:var(--paper);position:relative;z-index:3}.view-top>div:first-child{min-width:230px}.view-back{color:var(--focus);font-size:13px;text-underline-offset:3px}.view-search{flex:1;max-width:500px}.view-search label{font-size:12px;text-transform:uppercase;letter-spacing:.06em;margin-bottom:4px}.view-search input{background:var(--paper);width:100%;max-width:none;padding:8px;border:1px solid var(--edge)}.view-search-results{position:absolute;z-index:8;left:min(282px,25vw);top:88px;width:min(500px,65vw);background:var(--paper);border:1px solid var(--edge);max-height:300px;overflow:auto}.view-search-results:empty{display:none}.view-search-results button{display:block;width:100%;text-align:left;border:0;border-bottom:1px solid var(--grid)}\n.view-page{height:var(--contracts-height,100dvh);display:flex;flex-direction:column;background-image:linear-gradient(var(--grid) 1px,transparent 1px),linear-gradient(90deg,var(--grid) 1px,transparent 1px);background-size:24px 24px}\n.view-top{flex:none}\n.view-sheet{flex:1;min-height:0;position:relative;display:flex;flex-direction:column;gap:20px;padding:24px}\n.view-sheet-head{display:flex;align-items:flex-start;gap:24px;flex:none}\n.view-drawing{flex:1;min-height:0;min-width:0}\n.view-diagram{width:100%;height:100%;display:block;cursor:grab;touch-action:none;user-select:none;-webkit-user-select:none}.view-diagram *{-webkit-user-drag:none}\n.view-diagram:active{cursor:grabbing}\n.view-tools{display:flex;gap:4px;margin-left:auto;flex:none}\n.view-tools button{min-width:36px}\n.view-legend{background:var(--paper);border:1px solid var(--edge);padding:12px 16px}\n.view-legend{flex:none;width:360px;max-width:100%}\n.view-legend h2{font-size:14px;text-transform:uppercase;letter-spacing:.1em}\n.view-legend-mobile{display:none}\n.view-legend-kinds,.view-legend-lines{display:flex;flex-wrap:wrap;gap:8px 12px;font-size:12px}\n.view-legend-lines{margin-top:10px}\n.view-legend-kind,.view-legend-lines span{display:inline-flex;align-items:center;gap:6px}\n.view-swatch{margin:0;width:15px;height:17px;border:1px solid var(--edge);background:var(--paper);position:relative}\n.view-swatch:before{content:\"\";position:absolute;inset:0 auto 0 0;width:5px;background:var(--kind)}\n.view-swatch.entry{--kind:var(--entry)}.view-swatch.logic{--kind:var(--logic)}.view-swatch.effect{--kind:var(--effect)}.view-swatch.test{--kind:var(--test)}.view-swatch.file{--kind:var(--file)}.view-swatch.area{--kind:var(--area)}\n.view-legend-lines svg{width:32px;height:18px;flex:none}\n.view-revision-sample{fill:var(--paper);stroke:var(--revision);stroke-width:2}\n.view-drawing-note{max-width:48ch;font-size:13px;line-height:1.5;color:var(--soft);margin:0!important}\n.view-drawing-note:empty{display:none}\n.view-page .view-project{font-size:13px;color:var(--soft);margin:2px 0 0;overflow-wrap:anywhere}\n.view-wire{fill:none;stroke:var(--ink);stroke-width:1.6}\n.view-arrow{fill:none;stroke:var(--ink);stroke-width:1.4}\n.view-node{cursor:pointer;color:var(--ink)}\n.view-box{fill:var(--paper);stroke:var(--edge);stroke-width:1.8}\n.view-node.entry .view-tab{fill:var(--entry)}.view-node.logic .view-tab{fill:var(--logic)}.view-node.effect .view-tab{fill:var(--effect)}.view-node.test .view-tab{fill:var(--test)}.view-node.file .view-tab{fill:var(--file)}.view-node.area .view-tab{fill:var(--area)}.view-node.area .view-box{stroke-width:2.6}.view-face .view-summary.none{color:var(--soft);font-style:italic}\n.view-node.removed .view-box{stroke-dasharray:12 6;fill:var(--paper)}\n.view-node.changed .view-box{stroke:var(--revision);stroke-width:2.4}\n.view-node.selected .view-box{stroke-width:5}\n.view-ring{fill:none;stroke:none}.view-page .view-node:focus-visible{outline:none}.view-node:focus-visible .view-ring{stroke:var(--focus);stroke-width:3}\n.view-expander rect{fill:var(--ink)}.view-expander text{fill:var(--paper)!important;font-size:12px;font-weight:700}\n.view-wire-hit{fill:none;stroke:transparent;stroke-width:12;pointer-events:stroke}\n.view-panel-grip{position:absolute;z-index:6;top:100px;right:calc(min(var(--panel-width,410px),100%) - 4px);width:8px;height:calc(var(--contracts-height,100dvh) - 100px);cursor:ew-resize;touch-action:none}.view-panel-grip:hover,.view-panel-grip:focus-visible{background:var(--line);outline:none}\n:host(.view-resizing){cursor:ew-resize;user-select:none}\n.view-tabs{display:flex;align-items:stretch;background:var(--sheet);border-bottom:2px solid var(--edge)}\n.view-panel .view-tabs{position:sticky;top:-24px;z-index:1;margin:-24px -24px 20px}\n.view-page .view-tabs button{border:0;border-right:1px solid var(--line);background:transparent;padding:10px 16px}\n.view-page .view-tabs button[aria-selected=true]{background:var(--paper);font-weight:700;box-shadow:inset 0 -3px var(--ink)}\n.view-page .view-tabs .view-tabs-close{margin-left:auto;border-right:0;border-left:1px solid var(--line)}\n.view-pane-folded{position:absolute;z-index:4;top:30px;right:22px}.view-pane-folded .view-tabs{border:1px solid var(--edge)}\n.view-icon svg{width:18px;height:18px;display:block;fill:none;stroke:currentColor;stroke-width:2;stroke-linejoin:round;stroke-linecap:round}\n.view-file-role{font-size:13px;color:var(--soft)}.view-page .view-files{list-style:none;padding:0;margin:0}.view-page .view-files li{white-space:normal;margin:0}\n.view-files small{display:inline;color:var(--soft);font-size:12px}.view-files-dir{padding:6px 13px;color:var(--soft)}\n.view-page .view-row[aria-current=true]{background:var(--hover);font-weight:700}\n.view-reader{flex:1;min-height:0;display:flex;flex-direction:column;padding:24px;gap:12px}\n.view-reader-head{display:flex;align-items:start;justify-content:space-between;gap:16px}.view-reader-head h2{overflow-wrap:anywhere}.view-reader-head p{margin:0;color:var(--soft);font-size:13px}\n.view-reader-note{color:var(--soft);font-size:13px}\n.view-reader-text{flex:1;min-height:0;overflow:auto;margin:0;background:var(--paper);border:1px solid var(--edge);padding:12px 0;font:13px/1.5 'IBM Plex Mono',ui-monospace,monospace;white-space:pre;counter-reset:line}\n.view-reader-text span{display:block;padding-right:16px}.view-reader-text span::before{counter-increment:line;content:counter(line);display:inline-block;width:5ch;margin-right:2ch;text-align:right;color:var(--soft);user-select:none}\n.view-node.changed .view-badge,.view-node.removed .view-badge{fill:var(--revision)}\n.view-node text{fill:var(--ink);font-family:'IBM Plex Sans',system-ui,sans-serif}\n.view-kind{font-size:9px;letter-spacing:1.1px}\n.view-face{font-family:'IBM Plex Sans',system-ui,sans-serif;line-height:1.3;pointer-events:none}\n.view-face .view-name{font-size:16px;font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;margin:0 0 4px}\n.view-face .view-summary{font-size:11px;line-height:15px;height:30px;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden;overflow-wrap:anywhere;margin:0 0 5px}\n.view-face .view-io{font-size:11px;color:var(--soft);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;margin:0}.view-face .view-summary:empty{display:none}\n.view-badge{font-size:8px;font-weight:700}\n.view-panel{position:absolute;z-index:5;top:100px;right:0;width:min(var(--panel-width,410px),100%);height:calc(var(--contracts-height,100dvh) - 100px);overflow:auto;background:var(--paper);border-left:2px solid var(--edge);padding:24px;box-shadow:-9px 8px 24px var(--shadow)}.view-panel-head{display:flex;align-items:start;justify-content:space-between;gap:12px}.view-panel-head h2{overflow-wrap:anywhere}.view-detail-kind{text-transform:uppercase;letter-spacing:.1em;color:var(--soft);font-size:12px}.view-code,.view-source{font:12px/1.5 'IBM Plex Mono',ui-monospace,monospace;overflow-wrap:anywhere;white-space:pre-wrap;border-top:1px solid var(--line);padding-top:8px}.view-row{display:block;width:100%;text-align:left;margin:4px 0!important;overflow-wrap:anywhere}.view-review-summary{font-weight:700;border-top:2px solid var(--edge);border-bottom:1px solid var(--edge);padding:12px 0}.view-diff span{display:block}.view-diff .added{background:var(--add)}.view-diff .removed{background:var(--del)}.view-review-test,.view-file{border-bottom:1px solid var(--line);padding:8px 0}.view-review-test p{overflow-wrap:anywhere}.view-error{position:absolute}.view-error:not(:empty){position:absolute;left:20px;bottom:20px;background:var(--paper);border:2px solid var(--revision);padding:12px;color:var(--revision);z-index:9}\n.view-frame rect{fill:var(--frame);stroke:var(--area);stroke-width:1.4;stroke-dasharray:10 4 2 4}.view-frame text{fill:var(--area);font:700 13px 'IBM Plex Sans',system-ui,sans-serif;text-transform:uppercase;letter-spacing:.08em}\n.view-switch{display:flex;flex:none}.view-page .view-switch button{padding:8px 14px}.view-page .view-switch button+button{border-left:0}.view-page .view-switch button[aria-pressed=true]{background:var(--ink);color:var(--paper)}\n.view-search-results small,.view-row small{display:block;color:var(--soft);font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.view-search-results small:empty,.view-row small:empty{display:none}\n.view-outline{flex:1;min-height:0;overflow:auto;padding:28px 24px 48px}.view-outline-body{max-width:860px;margin:0 auto;background:var(--paper);border:1px solid var(--edge);padding:28px 32px}\n.view-page .view-outline h2{font-size:26px}.view-outline-lead{color:var(--soft)}.view-page .view-outline h3{margin-top:28px;border-bottom:2px solid var(--edge);padding-bottom:6px}\n.view-outline-notice{display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap;border:1px solid var(--line);border-top:4px solid var(--effect);background:var(--sheet);padding:10px 14px;margin:16px 0}.view-outline-notice p{margin:0}\n.view-page .view-outline-row{display:grid;gap:2px;width:100%;text-align:left;border:0;border-bottom:1px solid var(--grid);background:transparent;padding:10px 4px}.view-page .view-outline-row:hover{background:var(--hover)}\n.view-outline-name{font-weight:700;font-size:16px}.view-outline-summary{font-size:14px}.view-outline-meta{font-size:12.5px;color:var(--soft)}\n.view-outline-requests{margin:4px 0 12px 4px}.view-outline-requests p{font-size:12px;text-transform:uppercase;letter-spacing:.06em;color:var(--soft);margin:8px 0 4px}\n.view-requests{list-style:none;padding:0;margin:0;columns:2 220px;column-gap:24px;font-size:14px}.view-requests li{break-inside:avoid;padding:2px 0}.view-requests small{color:var(--soft)}\n.view-area{border-bottom:1px solid var(--line)}.view-area>summary{cursor:pointer;padding:12px 4px;display:grid;grid-template-columns:auto 1fr;gap:2px 12px;align-items:baseline}.view-area>summary:hover{background:var(--hover)}.view-area>summary:focus-visible{outline:3px solid var(--focus);outline-offset:2px}\n.view-area-name{font-weight:700;font-size:17px;text-transform:capitalize}.view-area-counts{font-size:12.5px;color:var(--soft);justify-self:end}.view-area-summary{grid-column:1/-1;font-size:14px;color:var(--soft)}.view-area[open]>summary{border-bottom:1px solid var(--grid)}.view-area .view-outline-row{padding-left:20px}\n.view-detail-summary{font-size:16px}.view-detail-io{color:var(--soft);font-size:14px}.view-examples{padding-left:18px;margin:0;font:12.5px/1.6 'IBM Plex Mono',ui-monospace,monospace;overflow-wrap:anywhere}\n.view-group{border-top:1px solid var(--line)}.view-group>summary{cursor:pointer;padding:8px 0;font-size:13px;text-transform:uppercase;letter-spacing:.06em;font-weight:700}.view-group>summary:focus-visible{outline:3px solid var(--focus)}\n@media(min-width:700px){\n.view-page.has-panel .view-reader{padding-right:calc(var(--panel-width,410px) + 24px)}\n.view-page.has-panel .view-outline{padding-right:calc(var(--panel-width,410px) + 24px)}\n.view-page.has-panel .view-sheet-head{padding-right:var(--panel-width,410px);flex-wrap:wrap}\n}\n@media(max-width:699px){\n.view-page{height:auto;min-height:var(--contracts-height,100dvh)}\n.view-top{height:138px;min-height:138px;flex-wrap:wrap;gap:8px 14px;padding:10px 14px}\n.view-top>div:first-child{min-width:0;flex:1}.view-top h1{font-size:22px}\n.view-search{order:3;flex-basis:100%;max-width:none}.view-search-results{left:14px;top:130px;width:calc(100% - 28px)}\n.view-sheet{flex:none;height:calc(var(--contracts-height,100dvh) - 138px);min-height:360px;padding:14px;gap:14px}\n.view-sheet-head{flex-wrap:wrap;gap:10px;align-items:start}\n.view-legend{width:auto;padding:0;border:0;background:transparent}\n.view-legend-desktop{display:none}.view-legend-mobile{display:block}\n.view-legend-mobile summary{cursor:pointer;min-height:36px;padding:6px 10px;border:1px solid var(--edge);background:var(--paper)}\n.view-legend-mobile summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}\n.view-legend:has(details[open]){order:2;width:100%}\n.view-legend-mobile .view-legend-content{padding:12px;background:var(--paper);border:1px solid var(--edge);border-top:0}\n.view-drawing-note{order:3;flex-basis:100%;max-width:none;font-size:12px}\n.view-tools{margin-left:auto}\n.view-switch{order:2}.view-outline{padding:14px}.view-outline-body{padding:18px}.view-area-counts{justify-self:start;grid-column:1/-1}.view-panel{top:138px;height:calc(var(--contracts-height,100dvh) - 138px);width:100%;padding:18px}.view-panel .view-tabs{top:-18px;margin:-18px -18px 16px}.view-pane-folded{top:12px;right:14px}.view-reader{height:calc(var(--contracts-height,100dvh) - 138px);padding:14px}.view-panel-grip{display:none}\n}\n@media(prefers-reduced-motion:reduce){.view-page *{scroll-behavior:auto!important}}"

pub fn view(model: Model) -> Element(Msg) {
  let results = search_results(model)
  h.main(
    [
      a.class(
        "view-page"
        <> case model.pane {
          True -> " has-panel"
          False -> ""
        },
      ),
    ],
    list.append(
      [
        h.style([], view_styles),
        h.header([a.class("view-top")], [
          h.div([], [
            h.a([ax("href", "/"), a.class("view-back")], [h.text("← Board")]),
            h.h1([], [h.text("Contract view")]),
            h.p([a.class("view-project")], [
              h.text(
                model.graph.project
                <> case model.task {
                  "" -> ""
                  id -> " · reviewing task " <> id
                },
              ),
            ]),
          ]),
          h.div([a.class("view-search")], [
            h.label([ax("for", "view-search")], [
              h.text("Find a function or effect"),
            ]),
            h.input([
              a.id("view-search"),
              a.attribute("type", "search"),
              a.attribute("placeholder", "Search name or summary"),
              a.aria_label("Search name or summary"),
              e.on_input(Search),
            ]),
          ]),
          h.div(
            [a.class("view-switch"), a.role("group"), a.aria_label("Show")],
            list.map([#(True, "Outline"), #(False, "Drawing")], fn(choice) {
              h.button(
                [
                  ax("aria-pressed", case model.outline == choice.0 {
                    True -> "true"
                    False -> "false"
                  }),
                  e.on_click(ShowOutline(choice.0)),
                ],
                [h.text(choice.1)],
              )
            }),
          ),
        ]),
        h.div(
          [a.class("view-search-results")],
          list.map(results, fn(node) {
            h.button([e.on_click(Select(node.id))], [
              h.text(node.name),
              h.small([], [h.text(" " <> node.summary)]),
            ])
          }),
        ),
        case model.file, model.outline {
          Some(file), _ -> file_view(file)
          None, True -> outline_view(model)
          None, False -> drawing_sheet(model)
        },
      ],
      list.append(pane(model), [
        h.div([a.class("view-error"), a.role("alert")], [h.text(model.error)]),
      ]),
    ),
  )
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  let task = task_id()
  let blank =
    Model(
      Graph([], [], None, "", "", []),
      [],
      [],
      [],
      "",
      None,
      "",
      [],
      "",
      "",
      1.0,
      0.0,
      0.0,
      False,
      0.0,
      0.0,
      0.0,
      "",
      "",
      "",
      False,
      False,
      None,
      False,
      "",
      None,
      "",
      no_root(),
      False,
    )
  #(
    fresh(blank, task),
    // Lustre runs init once for every instance, before its `base` is set:
    // the graph loads when the instance connects.
    effect.none(),
  )
}

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("base", fn(base) { Ok(BaseChanged(base)) }),
      component.on_attribute_change("task", fn(task) { Ok(TaskChanged(task)) }),
      component.on_disconnect(Disconnected),
      component.on_connect(Connected),
      component.adopt_styles(False),
    ]),
    "solace-contracts",
  )
}

/// Clears the record shown, keeping the element and its transport, and opens `task`.
fn fresh(model: Model, task: String) -> Model {
  Model(
    ..model,
    graph: Graph([], [], None, "", "", []),
    visible: [],
    expanded: [],
    positions: [],
    selected: "",
    detail: None,
    source: "",
    diff: [],
    search: "",
    tab: case task {
      "" -> "files"
      _ -> "review"
    },
    zoom: 1.0,
    pan_x: 0.0,
    pan_y: 0.0,
    dragging: False,
    drag_x: 0.0,
    drag_y: 0.0,
    travel: 0.0,
    focus: "",
    task: task,
    error: "",
    tests_requested: False,
    pane: task != "",
    file: None,
    outline: task == "",
    asked: "",
  )
}

/// Fresh node positions for a flow diagram, from the drawing's layout.
pub fn flow_positions(nodes: String, edges: String) -> String {
  flow_layout(nodes, edges, 1440.0)
}

@external(javascript, "./layout.mjs", "flow_positions")
fn flow_layout(nodes: String, edges: String, width: Float) -> String
