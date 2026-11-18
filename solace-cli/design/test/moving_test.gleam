//// Tests the rules for moving an element and where a dragged element lands.

import gleam/list
import gleam/option.{None, Some}
import gleeunit
import moving
import region
import render

pub fn main() -> Nil {
  gleeunit.main()
}

fn a_design() -> render.Design {
  let assert Ok(design) =
    render.decode(
      "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"one\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[{\"id\":\"bar\",\"type\":\"row\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[{\"id\":\"go\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Go\"}},{\"id\":\"stop\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Stop\"}}]},{\"id\":\"body\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Hi\"}}]},{\"id\":\"two\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[]}]}",
    )
  design
}

fn ids(nodes: List(render.Node)) -> List(String) {
  list.flat_map(nodes, fn(node) { [node.id, ..ids(node.children)] })
}

fn bounds() -> List(#(String, region.Rect)) {
  [
    #("one", region.Rect(0.0, 0.0, 400.0, 300.0)),
    #("bar", region.Rect(10.0, 50.0, 390.0, 100.0)),
    #("go", region.Rect(20.0, 60.0, 100.0, 90.0)),
    #("stop", region.Rect(110.0, 60.0, 190.0, 90.0)),
    #("body", region.Rect(10.0, 110.0, 390.0, 140.0)),
    #("two", region.Rect(0.0, 320.0, 400.0, 620.0)),
  ]
}

pub fn an_index_counts_the_siblings_without_the_moved_element_test() {
  let assert Ok(moved) =
    moving.move(a_design(), "go", moving.Place(Some("bar"), 1))
  assert ids(moved.elements) == ["one", "bar", "stop", "go", "body", "two"]
}

pub fn an_element_moves_into_another_container_test() {
  let assert Ok(moved) =
    moving.move(a_design(), "body", moving.Place(Some("two"), 0))
  assert ids(moved.elements) == ["one", "bar", "go", "stop", "two", "body"]
  assert moving.place_of(moved, "body") == Some(moving.Place(Some("two"), 0))
}

pub fn the_design_rules_refuse_a_move_test() {
  let design = a_design()
  assert moving.move(design, "two", moving.Place(Some("bar"), 0)) == Error(Nil)
  assert moving.move(design, "go", moving.Place(None, 0)) == Error(Nil)
  assert moving.move(design, "body", moving.Place(Some("go"), 0)) == Error(Nil)
  assert moving.move(design, "bar", moving.Place(Some("bar"), 0)) == Error(Nil)
  assert moving.move(design, "go", moving.Place(Some("bar"), 2)) == Error(Nil)
  assert moving.move(design, "gone", moving.Place(Some("bar"), 0)) == Error(Nil)
}

pub fn a_screen_reorders_among_the_screens_test() {
  let assert Ok(moved) = moving.move(a_design(), "two", moving.Place(None, 0))
  assert ids(moved.elements) == ["two", "one", "bar", "go", "stop", "body"]
}

pub fn the_pointer_half_of_a_sibling_picks_before_or_after_test() {
  let design = a_design()
  assert moving.drop_place(design, "stop", 30.0, 75.0, bounds())
    == Some(moving.Place(Some("bar"), 0))
  assert moving.drop_place(design, "body", 90.0, 75.0, bounds())
    == Some(moving.Place(Some("bar"), 1))
}

pub fn a_container_under_the_pointer_takes_the_element_test() {
  assert moving.drop_place(a_design(), "go", 200.0, 400.0, bounds())
    == Some(moving.Place(Some("two"), 0))
  assert moving.drop_place(a_design(), "body", 380.0, 75.0, bounds())
    == Some(moving.Place(Some("bar"), 2))
}

pub fn a_screen_over_a_row_lands_among_the_screens_test() {
  assert moving.drop_place(a_design(), "two", 30.0, 75.0, bounds())
    == Some(moving.Place(None, 0))
}

pub fn outside_every_element_only_a_screen_has_a_place_test() {
  assert moving.drop_place(a_design(), "body", 500.0, 700.0, bounds()) == None
  assert moving.drop_place(a_design(), "one", 500.0, 700.0, bounds())
    == Some(moving.Place(None, 1))
}
