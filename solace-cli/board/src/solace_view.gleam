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
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/element/svg as s
import lustre/event as e

@external(javascript, "./api.mjs", "request")
fn request(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./layout.mjs", "after_draw")
fn after_draw(callback: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(receive: fn(String) -> Nil) -> Nil

@external(javascript, "./layout.mjs", "placement")
fn placement(
  nodes: String,
  edges: String,
  overview: Bool,
  anchor: String,
) -> String

@external(javascript, "./layout.mjs", "fitted")
fn fitted(points: String, panel: Bool) -> String

@external(javascript, "./layout.mjs", "task_id")
fn task_id() -> String

@external(javascript, "./layout.mjs", "viewport_width")
fn viewport_width() -> Float

@external(javascript, "./layout.mjs", "viewport_height")
fn viewport_height() -> Float

@external(javascript, "./layout.mjs", "keys")
fn keys(receive: fn(String) -> Nil) -> Nil

@external(javascript, "./layout.mjs", "focus_node")
fn focus_node(id: String) -> Nil

@external(javascript, "./layout.mjs", "panel_width")
fn panel_width() -> Float

@external(javascript, "./layout.mjs", "resizer")
fn resizer() -> Nil

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
  )
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
  )
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
    files: Bool,
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
    review_open: Bool,
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
  Files
  ToggleFiles
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
  ))
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
  decode.success(Graph(nodes, edges, review, project, fingerprint))
}

fn point_decoder() -> decode.Decoder(Point) {
  use id <- decode.field("id", decode.string)
  use x <- decode.field("x", decode.float)
  use y <- decode.field("y", decode.float)
  decode.success(Point(id, x, y))
}

pub fn overview(graph: Graph) -> List(String) {
  graph.nodes
  |> list.filter(fn(n) { n.kind == "entry" || n.kind == "effect" })
  |> list.map(fn(n) { n.id })
}

pub fn reachability(graph: Graph) -> List(Edge) {
  list.filter(graph.edges, fn(edge) {
    edge.kind == "uses"
    && case list.find(graph.nodes, fn(node) { node.id == edge.from }) {
      Ok(node) -> node.kind == "entry"
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
    False -> #(list.unique(list.append(visible, neighbors(graph, id))), [
      id,
      ..expanded
    ])
  }
}

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

fn visible_edges(model: Model) -> List(Edge) {
  let visible = fn(edge: Edge) {
    list.contains(model.visible, edge.from)
    && list.contains(model.visible, edge.to)
  }
  case model.expanded {
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
        model.expanded == [],
        case model.expanded {
          [anchor, ..] -> anchor
          [] -> ""
        },
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
  case json.parse(fitted(points, model.review_open), decoder) {
    Ok(#(zoom, x, y)) -> Model(..model, zoom: zoom, pan_x: x, pan_y: y)
    Error(_) -> model
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  let task = task_id()
  let path = case task {
    "" -> "/graph"
    _ -> "/graph?task=" <> task
  }
  #(
    Model(
      Graph([], [], None, "", ""),
      [],
      [],
      [],
      "",
      None,
      "",
      [],
      "",
      False,
      1.0,
      0.0,
      0.0,
      False,
      0.0,
      0.0,
      0.0,
      "",
      task,
      "",
      False,
      task != "",
    ),
    effect.batch([
      get(path, Loaded),
      effect.from(fn(send) {
        keys(fn(key) { send(Key(key)) })
        resizer()
      }),
      case task {
        "" -> effect.none()
        _ -> effect.from(fn(send) { events(fn(data) { send(Live(data)) }) })
      },
    ]),
  )
}

fn review_needs_tests(review: Review) -> Bool {
  string.contains(review.summary, "tests not run yet")
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Loaded(text) ->
      case json.parse(text, graph_decoder()) {
        Ok(graph) -> {
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
            files: False,
            source: "",
            diff: [],
            search: "",
            review_open: False,
            focus: id,
          ),
        )
      let next = Model(..reveal(next, id, True), detail: n(model, id))
      case n(model, id) {
        Some(node) if node.removed -> #(
          Model(..next, detail: Some(node)),
          effect.none(),
        )
        _ -> #(next, get("/nodes/" <> id, Detail))
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
    Files -> #(
      Model(
        ..model,
        files: True,
        selected: "",
        detail: None,
        review_open: False,
      ),
      effect.none(),
    )
    ToggleFiles -> #(Model(..model, files: False), effect.none())
    Reset -> #(
      fit(layout(
        Model(
          ..model,
          visible: overview(model.graph),
          expanded: [],
          selected: "",
          detail: None,
          files: False,
          review_open: model.task != "",
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
          get("/graph?task=" <> model.task, Loaded),
        )
        _ -> #(model, effect.none())
      }
    SelectSource(id) -> #(
      model,
      get(
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
    Close -> #(
      Model(
        ..model,
        selected: "",
        detail: None,
        files: False,
        source: "",
        diff: [],
        review_open: False,
      ),
      effect.none(),
    )
  }
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
  let half_w = viewport_width() /. 2.0
  let half_h = viewport_height() /. 2.0
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
  let width = case panel && viewport_width() >. 650.0 {
    True -> viewport_width() -. panel_width()
    False -> viewport_width()
  }
  let w = width /. model.zoom
  let h = viewport_height() /. model.zoom
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
  let has_panel = model.detail != None || model.files || model.review_open
  let focused = list.any(model.positions, fn(p) { p.id == model.focus })
  let go = fn(model: Model, id) {
    #(
      Model(..reveal(model, id, has_panel), focus: id),
      effect.from(fn(_) { focus_node(id) }),
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
    <> node.signature
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
  let expander = case list.contains(model.expanded, node.id), more {
    True, _ -> [expander_mark("−", "Expanded; press C to collapse")]
    False, 0 -> []
    False, _ -> [
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
      s.rect([a.class("view-box"), ax("width", "220"), ax("height", "120")]),
      s.rect([a.class("view-tab"), ax("width", "7"), ax("height", "120")]),
      s.text(
        [a.class("view-kind"), ax("x", "20"), ax("y", "19")],
        string.uppercase(node.kind),
      ),
      s.foreign_object(
        [ax("x", "20"), ax("y", "29"), ax("width", "188"), ax("height", "84")],
        [
          h.div([a.class("view-face")], [
            h.p([a.class("view-name")], [h.text(node.name)]),
            case node.summary {
              "" ->
                h.p([a.class("view-summary none")], [
                  h.text("No summary recorded"),
                ])
              text -> h.p([a.class("view-summary")], [h.text(text)])
            },
            h.p([a.class("view-signature")], [h.text(node.signature)]),
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
              node.tests == 0 && node.kind != "effect" && node.kind != "test"
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

/// A tab on a box's bottom-right corner: "−" when expanded, "+N" for N connections not shown.
fn expander_mark(text: String, title: String) -> Element(Msg) {
  s.g([a.class("view-expander"), ax("transform", "translate(220 120)")], [
    s.title([], [h.text(title)]),
    s.rect([
      ax("x", "-20"),
      ax("y", "-10"),
      ax("width", "40"),
      ax("height", "20"),
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

fn review_panel(review: Review) -> Element(Msg) {
  let entries =
    list.map(review.entries, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.id))], [
        h.text(
          item.change
          <> " · "
          <> item.id
          <> " + "
          <> string.join(item.effects_added, ", ")
          <> " − "
          <> string.join(item.effects_removed, ", "),
        ),
      ])
    })
  let signatures =
    list.map(review.signatures, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.id))], [
        h.text(item.id <> ": " <> item.before <> " → " <> item.after),
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
      h.p([], [h.text(item.change <> " · " <> item.id <> " · " <> item.tier)])
    })
  let untested =
    list.map(review.untested, fn(item) {
      h.button([a.class("view-row"), e.on_click(Select(item.0))], [
        h.text(item.1),
      ])
    })
  h.aside(
    [a.class("view-panel view-review"), a.aria_label("Task review")],
    list.flatten([
      [
        h.div([a.class("view-panel-head")], [
          h.h2([], [h.text("Revision review")]),
          h.button([e.on_click(Close)], [h.text("Close")]),
        ]),
        h.p([a.class("view-review-summary")], [h.text(review.summary)]),
        h.h3([], [h.text("Entry points & effects")]),
      ],
      entries,
      [
        h.p([], [
          h.text(
            "Effects added: "
            <> string.join(review.added, ", ")
            <> ". Removed: "
            <> string.join(review.removed, ", "),
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
    ]),
  )
}

fn detail_panel(model: Model, node: Node) -> Element(Msg) {
  let related = neighbors(model.graph, node.id)
  let connections =
    list.map(related, fn(id) {
      h.button([a.class("view-row"), e.on_click(Select(id))], [
        h.text(case n(model, id) {
          Some(other) -> other.kind <> " · " <> other.name
          None -> id
        }),
      ])
    })
  let checks =
    list.map(node.checks, fn(check) { h.p([], [h.text(check_line(check))]) })
  h.aside(
    [a.class("view-panel"), a.aria_label("Node detail")],
    list.flatten([
      [
        h.div([a.class("view-panel-head")], [
          h.h2([], [h.text(node.name)]),
          h.button([e.on_click(Close)], [h.text("Close")]),
        ]),
        h.p([a.class("view-detail-kind")], [
          h.text(node.kind <> " · " <> node.known),
        ]),
        h.p([], [h.text(node.summary)]),
        h.p([a.class("view-code")], [h.text(node.signature)]),
        h.button([e.on_click(Toggle(node.id))], [
          h.text(case list.contains(model.expanded, node.id) {
            True -> "Collapse connections"
            False -> "Expand connections"
          }),
        ]),
        h.h3([], [h.text("Connections")]),
      ],
      connections,
      case checks {
        [] -> []
        _ -> [h.h3([], [h.text("Checks")]), ..checks]
      },
      case node.removed && model.task == "" {
        True -> []
        False -> [
          h.button([e.on_click(SelectSource(node.id))], [h.text("Show source")]),
          source_text(model),
        ]
      },
    ]),
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

fn files_panel(model: Model) -> Element(Msg) {
  h.aside([a.class("view-panel"), a.aria_label("Project files")], [
    h.div([a.class("view-panel-head")], [
      h.h2([], [h.text("Project files")]),
      h.button([e.on_click(ToggleFiles)], [h.text("Close")]),
    ]),
    ..list.map(
      list.filter(model.graph.nodes, fn(node) { node.kind == "file" }),
      fn(node) {
        h.div([a.class("view-file")], [
          h.h3([], [h.text(node.name)]),
          h.p([], [h.text(node.sentence)]),
        ])
      },
    )
  ])
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
  let elements =
    list.append(
      list.map(edges, fn(edge) { line(model, edge, model.expanded == []) }),
      list.map(visible, fn(node) { box(model, node) }),
    )
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
          <> num(viewport_width() /. model.zoom)
          <> " "
          <> num(viewport_height() /. model.zoom),
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

fn legend_items() -> Element(Msg) {
  h.div([a.class("view-legend-content")], [
    h.div(
      [a.class("view-legend-kinds")],
      list.map(["entry", "logic", "effect", "test", "file"], fn(kind) {
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
      h.span([], [h.text("+3: connections not shown · −: expanded")]),
    ]),
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

pub fn view(model: Model) -> Element(Msg) {
  let results = search_results(model)
  let panel = case model.files, model.detail, model.graph.review {
    True, _, _ -> [files_panel(model)]
    _, Some(node), _ -> [detail_panel(model, node)]
    _, _, Some(review) if model.review_open -> [review_panel(review)]
    _, _, _ -> []
  }
  h.main(
    [
      a.class(
        "view-page"
        <> case panel {
          [] -> ""
          _ -> " has-panel"
        },
      ),
    ],
    list.append(
      [
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
          h.button([e.on_click(Files)], [h.text("Files")]),
        ]),
        h.div(
          [a.class("view-search-results")],
          list.map(results, fn(node) {
            h.button([e.on_click(Select(node.id))], [
              h.text(node.kind <> " · " <> node.name <> " — " <> node.summary),
            ])
          }),
        ),
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
              h.button([e.on_click(Reset)], [h.text("Overview")]),
            ]),
          ]),
          h.div([a.class("view-drawing")], [svg_view(model)]),
        ]),
      ],
      list.append(
        case panel {
          [] -> []
          _ -> [grip(), ..panel]
        },
        [
          h.div([a.class("view-error"), a.role("alert")], [h.text(model.error)]),
        ],
      ),
    ),
  )
}
