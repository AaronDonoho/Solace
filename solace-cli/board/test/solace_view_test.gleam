import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None}
import gleam/result
import gleeunit/should
import solace_view

pub fn graph_decoder_and_selection_test() {
  let text =
    "{\"nodes\":[{\"id\":\"fn:app.main\",\"kind\":\"entry\",\"name\":\"main\",\"summary\":\"Starts the app\",\"signature\":\"main() -> Nil\",\"known\":\"derived\",\"tests\":0},{\"id\":\"fn:app.work\",\"kind\":\"logic\",\"name\":\"work\"},{\"id\":\"effect:prints\",\"kind\":\"effect\",\"name\":\"prints\"}],\"edges\":[{\"from\":\"fn:app.main\",\"to\":\"fn:app.work\",\"kind\":\"calls\"},{\"from\":\"fn:app.main\",\"to\":\"effect:prints\",\"kind\":\"uses\"}]}"
  let graph =
    json.parse(text, solace_view.graph_decoder())
    |> result.unwrap(solace_view.Graph([], [], None, "", "", []))
  solace_view.overview(graph) |> should.equal(["fn:app.main", "effect:prints"])
  solace_view.neighbors(graph, "fn:app.main")
  |> should.equal(["fn:app.work", "effect:prints"])
  solace_view.reachability(graph)
  |> should.equal([
    solace_view.Edge("fn:app.main", "effect:prints", "uses", False, "derived"),
  ])
  let #(visible, expanded) =
    solace_view.toggle(graph, solace_view.overview(graph), [], "fn:app.main")
  visible |> should.equal(["fn:app.main", "effect:prints", "fn:app.work"])
  let #(collapsed, _) =
    solace_view.toggle(graph, visible, expanded, "fn:app.main")
  collapsed |> should.equal(["fn:app.main", "effect:prints"])
}

pub fn check_decoder_test() {
  let check =
    json.parse(
      "{\"call\":\"add(1, 2)\",\"expected\":\"3\",\"was\":\"2\"}",
      solace_view.check_decoder(),
    )
  check |> should.equal(Ok(solace_view.Check("add(1, 2)", "3", "", "2")))
}

pub fn connectors_meet_the_facing_box_sides_test() {
  let origin = solace_view.Point("a", 0.0, 0.0)
  let right = solace_view.Point("b", 320.0, 0.0)
  let below = solace_view.Point("c", 0.0, 156.0)
  solace_view.connector(origin, right)
  |> should.equal("M 220.0 60.0 H 270.0 V 60.0 H 320.0")
  solace_view.connector(right, origin)
  |> should.equal("M 320.0 60.0 H 270.0 V 60.0 H 220.0")
  solace_view.connector(origin, below)
  |> should.equal("M 110.0 120.0 V 138.0 H 110.0 V 156.0")
  solace_view.connector(below, origin)
  |> should.equal("M 110.0 156.0 V 138.0 H 110.0 V 120.0")
}

pub fn arrow_keys_move_to_the_nearest_box_in_line_test() {
  let a = solace_view.Point("a", 0.0, 0.0)
  let points = [
    a,
    solace_view.Point("right", 252.0, 0.0),
    solace_view.Point("far_right", 504.0, 0.0),
    solace_view.Point("down_right", 252.0, 156.0),
    solace_view.Point("down", 0.0, 156.0),
  ]
  solace_view.nearest(points, a, "right") |> should.equal(Ok("right"))
  solace_view.nearest(points, a, "down") |> should.equal(Ok("down"))
  solace_view.nearest(points, a, "left") |> should.equal(Error(Nil))
  solace_view.nearest(points, solace_view.Point("x", 252.0, 156.0), "up")
  |> should.equal(Ok("right"))
}

pub fn nodes_without_connections_do_not_expand_test() {
  let graph =
    solace_view.Graph(
      [],
      [solace_view.Edge("a", "b", "calls", True, "derived")],
      None,
      "",
      "",
      [],
    )
  solace_view.toggle(graph, ["lonely"], [], "lonely")
  |> should.equal(#(["lonely"], []))
  solace_view.toggle(graph, ["a"], [], "a")
  |> should.equal(#(["a", "b"], ["a"]))
}

fn entry(i: Int, module: String) -> String {
  "{\"id\":\"e"
  <> int.to_string(i)
  <> "\",\"kind\":\"entry\",\"name\":\"e"
  <> int.to_string(i)
  <> "\",\"module\":\""
  <> module
  <> "\"}"
}

/// Many entry points fold into one box per area; an area contains its entry
/// points and reaches what they reach (D-55.6). Few entry points don't fold.
pub fn areas_fold_many_entries_test() {
  let entries =
    int.range(from: 1, to: 14, with: [], run: list.prepend)
    |> list.reverse
    |> list.map(fn(i) {
      entry(i, case i <= 6 {
        True -> "app/orders"
        False -> "app/reports"
      })
    })
  let text =
    "{\"nodes\":["
    <> list.fold(
      entries,
      "{\"id\":\"effect:changes Store\",\"kind\":\"effect\",\"name\":\"changes Store\"}",
      fn(acc, x) { acc <> "," <> x },
    )
    <> "],\"edges\":[{\"from\":\"e1\",\"to\":\"effect:changes Store\",\"kind\":\"uses\"}],\"areas\":[{\"id\":\"app/orders\",\"name\":\"orders\",\"summary\":\"Orders.\"}]}"
  let assert Ok(graph) = json.parse(text, solace_view.graph_decoder())
  let graph = solace_view.with_areas(graph)
  solace_view.overview(graph)
  |> should.equal([
    "area:app/orders",
    "area:app/reports",
    "effect:changes Store",
  ])
  solace_view.reachability(graph)
  |> list.map(fn(edge) { edge.from })
  |> should.equal(["e1", "area:app/orders"])
  let assert Ok(orders) =
    list.find(graph.nodes, fn(n) { n.id == "area:app/orders" })
  #(orders.name, orders.summary) |> should.equal(#("orders", "Orders."))
  solace_view.connection_groups(graph, "area:app/orders")
  |> should.equal([
    #("Effects", ["effect:changes Store"]),
    #("Ways in", ["e1", "e2", "e3", "e4", "e5", "e6"]),
  ])
  solace_view.connection_groups(graph, "e1")
  |> should.equal([
    #("Effects", ["effect:changes Store"]),
    #("Area", ["area:app/orders"]),
  ])
  let assert Ok(few) =
    json.parse(
      "{\"nodes\":[" <> entry(1, "app/orders") <> "],\"edges\":[]}",
      solace_view.graph_decoder(),
    )
  solace_view.overview(solace_view.with_areas(few)) |> should.equal(["e1"])
}

pub fn io_line_and_undescribed_test() {
  let assert Ok(node) =
    json.parse(
      "{\"id\":\"f\",\"kind\":\"logic\",\"name\":\"get_payment\",\"public\":true,\"takes\":[\"Store\",\"id\"],\"gives\":\"Payment or an error\",\"requests\":[{\"name\":\"Add item\",\"fields\":[\"order id\"]}]}",
      solace_view.node_decoder(),
    )
  solace_view.io_line(node) |> should.equal("Store, id → Payment or an error")
  node.requests
  |> should.equal([solace_view.Capability("Add item", ["order id"])])
  solace_view.undescribed(node) |> should.be_true
  solace_view.undescribed(solace_view.Node(..node, summary: "Finds a payment."))
  |> should.be_false
  solace_view.io_line(solace_view.Node(..node, takes: []))
  |> should.equal("→ Payment or an error")
}
