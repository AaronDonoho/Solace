import gleam/json
import gleam/option.{None}
import gleam/result
import gleeunit/should
import solace_view

pub fn graph_decoder_and_selection_test() {
  let text =
    "{\"nodes\":[{\"id\":\"fn:app.main\",\"kind\":\"entry\",\"name\":\"main\",\"summary\":\"Starts the app\",\"signature\":\"main() -> Nil\",\"known\":\"derived\",\"tests\":0},{\"id\":\"fn:app.work\",\"kind\":\"logic\",\"name\":\"work\"},{\"id\":\"effect:prints\",\"kind\":\"effect\",\"name\":\"prints\"}],\"edges\":[{\"from\":\"fn:app.main\",\"to\":\"fn:app.work\",\"kind\":\"calls\"},{\"from\":\"fn:app.main\",\"to\":\"effect:prints\",\"kind\":\"uses\"}]}"
  let graph =
    json.parse(text, solace_view.graph_decoder())
    |> result.unwrap(solace_view.Graph([], [], None, "", ""))
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
