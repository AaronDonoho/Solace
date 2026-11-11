import changes
import gleam/dict
import gleam/list
import gleam/string
import gleeunit
import lustre/element
import render

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn an_added_element_is_marked_test() {
  let current = design([screen("a", [text("one", "One")])])
  let proposed = design([screen("a", [text("one", "One"), text("two", "Two")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "two") == Ok(render.Added)
  assert dict.get(marked.marks, "one") == Error(Nil)
  assert dict.get(marked.marks, "a") == Error(Nil)
}

pub fn a_removed_element_is_marked_and_stays_in_the_drawing_test() {
  let current = design([screen("a", [text("one", "One"), text("two", "Two")])])
  let proposed = design([screen("a", [text("one", "One")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "two") == Ok(render.Removed)
  assert dict.get(marked.marks, "one") == Error(Nil)
  assert has_id(marked.design.elements, "two")
}

pub fn a_changed_element_is_marked_test() {
  let current = design([screen("a", [text("one", "One")])])
  let proposed = design([screen("a", [text("one", "One!")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "one") == Ok(render.Changed)
  assert dict.get(marked.marks, "a") == Error(Nil)
}

pub fn a_moved_element_is_marked_test() {
  let current = design([screen("a", [text("one", "One"), text("two", "Two")])])
  let proposed = design([screen("a", [text("two", "Two"), text("one", "One")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "one") == Ok(render.Moved)
  assert dict.get(marked.marks, "two") == Ok(render.Moved)
}

pub fn an_unchanged_element_is_not_marked_test() {
  let drawing = design([screen("a", [text("one", "One")])])
  let marked = changes.mark(drawing, drawing)
  assert marked.marks == dict.new()
}

pub fn an_added_sibling_does_not_move_the_others_test() {
  let current = design([screen("a", [text("one", "One"), text("two", "Two")])])
  let proposed =
    design([
      screen("a", [
        text("one", "One"),
        text("three", "Three"),
        text("two", "Two"),
      ]),
    ])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "three") == Ok(render.Added)
  assert dict.get(marked.marks, "one") == Error(Nil)
  assert dict.get(marked.marks, "two") == Error(Nil)
}

pub fn a_removed_sibling_does_not_move_the_others_test() {
  let current =
    design([
      screen("a", [
        text("one", "One"),
        text("two", "Two"),
        text("three", "Three"),
      ]),
    ])
  let proposed =
    design([screen("a", [text("one", "One"), text("three", "Three")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "two") == Ok(render.Removed)
  assert dict.get(marked.marks, "one") == Error(Nil)
  assert dict.get(marked.marks, "three") == Error(Nil)
}

pub fn a_changed_link_marks_the_element_changed_test() {
  let current =
    render.Design(
      "d",
      "Home",
      [screen("a", [text("one", "One")])],
      dict.insert(dict.new(), "one", "two"),
    )
  let proposed =
    render.Design(
      "d",
      "Home",
      [screen("a", [text("one", "One")])],
      dict.insert(dict.new(), "one", "three"),
    )
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "one") == Ok(render.Changed)
}

pub fn a_changed_and_moved_element_is_marked_both_test() {
  let current = design([screen("a", [text("one", "One"), text("two", "Two")])])
  let proposed =
    design([screen("a", [text("two", "Two"), text("one", "Changed")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "one") == Ok(render.ChangedAndMoved)
  assert dict.get(marked.marks, "two") == Ok(render.Moved)
}

pub fn a_new_design_is_all_added_test() {
  let marked = changes.all_added(design([screen("a", [text("one", "One")])]))
  assert dict.get(marked.marks, "a") == Ok(render.Added)
  assert dict.get(marked.marks, "one") == Ok(render.Added)
}

pub fn a_marked_element_carries_its_class_test() {
  let marks = dict.insert(dict.new(), "one", render.Added)
  let html = element.to_string(render.draw_marked(text("one", "One"), marks))
  assert string.contains(html, "design-change-added")
}

fn design(elements: List(render.Node)) -> render.Design {
  render.Design("d", "Home", elements, dict.new())
}

fn screen(id: String, children: List(render.Node)) -> render.Node {
  render.Node(
    id,
    render.Screen(render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )),
    children,
    render.no_overrides(),
  )
}

fn text(id: String, content: String) -> render.Node {
  render.Node(
    id,
    render.Text(render.TextProperties(
      render.Hug,
      render.Hug,
      render.TextLeft,
      content,
      render.Body,
    )),
    [],
    render.no_overrides(),
  )
}

fn has_id(elements: List(render.Node), id: String) -> Bool {
  list.any(elements, fn(node) { node.id == id || has_id(node.children, id) })
}

pub fn a_removed_screen_stays_in_the_drawing_test() {
  let current =
    design([screen("a", [text("one", "One")]), screen("b", [text("x", "X")])])
  let proposed = design([screen("a", [text("one", "One")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "b") == Ok(render.Removed)
  assert has_id(marked.design.elements, "b")
}

pub fn a_child_moved_out_of_a_removed_parent_draws_once_test() {
  let current = design([screen("a", [screen("p", [text("c", "C")])])])
  let proposed = design([screen("a", [text("c", "C")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "p") == Ok(render.Removed)
  assert count_id(marked.design.elements, "c") == 1
}

fn count_id(elements: List(render.Node), id: String) -> Int {
  list.fold(elements, 0, fn(count, node) {
    let here = case node.id == id {
      True -> 1
      False -> 0
    }
    count + here + count_id(node.children, id)
  })
}

fn flow(id: String, children: List(render.Node)) -> render.Node {
  render.Node(id, render.Flow, children, render.no_overrides())
}

fn diagram_node(id: String, label: String) -> render.Node {
  render.Node(
    id,
    render.DiagramNode(render.DiagramNodeProperties(label, "")),
    [],
    render.no_overrides(),
  )
}

fn diagram_edge(
  id: String,
  from: String,
  to: String,
  label: String,
) -> render.Node {
  render.Node(
    id,
    render.DiagramEdge(render.DiagramEdgeProperties(from, to, label)),
    [],
    render.no_overrides(),
  )
}

pub fn reordering_flow_children_does_not_move_them_test() {
  let current =
    design([flow("f", [diagram_node("one", "One"), diagram_node("two", "Two")])])
  let proposed =
    design([flow("f", [diagram_node("two", "Two"), diagram_node("one", "One")])])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "one") == Error(Nil)
  assert dict.get(marked.marks, "two") == Error(Nil)
}

pub fn a_changed_edge_is_marked_changed_test() {
  let current =
    design([
      flow("f", [
        diagram_node("a", "A"),
        diagram_node("b", "B"),
        diagram_edge("e", "a", "b", "Next"),
      ]),
    ])
  let proposed =
    design([
      flow("f", [
        diagram_node("a", "A"),
        diagram_node("b", "B"),
        diagram_edge("e", "a", "b", "Later"),
      ]),
    ])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "e") == Ok(render.Changed)
}

pub fn moving_a_flow_node_and_edge_to_another_flow_marks_them_moved_test() {
  let current =
    design([
      flow("f", [
        diagram_node("a", "A"),
        diagram_node("b", "B"),
        diagram_edge("e", "a", "b", "Next"),
      ]),
    ])
  let proposed =
    design([
      flow("g", [
        diagram_node("a", "A"),
        diagram_node("b", "B"),
        diagram_edge("e", "a", "b", "Next"),
      ]),
    ])
  let marked = changes.mark(current, proposed)
  assert dict.get(marked.marks, "a") == Ok(render.Moved)
  assert dict.get(marked.marks, "e") == Ok(render.Moved)
  assert dict.get(marked.marks, "f") == Ok(render.Removed)
  assert dict.get(marked.marks, "g") == Ok(render.Added)
}
