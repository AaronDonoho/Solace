//// Tests the rule that decides which drawn elements a region selects.

import gleeunit
import region

pub fn main() -> Nil {
  gleeunit.main()
}

fn rect(left: Float, top: Float, right: Float, bottom: Float) -> region.Rect {
  region.Rect(left, top, right, bottom)
}

pub fn a_wholly_inside_element_is_selected_test() {
  assert region.ids_inside(rect(0.0, 0.0, 100.0, 100.0), [
      #("button", rect(10.0, 10.0, 40.0, 30.0)),
    ])
    == ["button"]
}

pub fn a_partly_inside_element_is_not_selected_test() {
  assert region.ids_inside(rect(0.0, 0.0, 100.0, 100.0), [
      #("row", rect(50.0, 0.0, 150.0, 100.0)),
    ])
    == []
}

pub fn an_element_on_the_boundary_is_selected_test() {
  assert region.ids_inside(rect(0.0, 0.0, 100.0, 100.0), [
      #("edge", rect(0.0, 0.0, 100.0, 100.0)),
    ])
    == ["edge"]
}

pub fn a_parent_and_its_children_are_all_selected_when_inside_test() {
  assert region.ids_inside(rect(-1.0, -1.0, 101.0, 101.0), [
      #("row", rect(0.0, 0.0, 100.0, 100.0)),
      #("one", rect(5.0, 5.0, 20.0, 20.0)),
      #("two", rect(25.0, 5.0, 40.0, 20.0)),
    ])
    == ["row", "one", "two"]
}

pub fn children_are_selected_when_their_parent_sticks_out_test() {
  assert region.ids_inside(rect(4.0, 4.0, 50.0, 50.0), [
      #("row", rect(0.0, 0.0, 100.0, 100.0)),
      #("one", rect(10.0, 10.0, 20.0, 20.0)),
      #("two", rect(25.0, 10.0, 35.0, 20.0)),
    ])
    == ["one", "two"]
}

pub fn an_empty_region_selects_nothing_test() {
  assert region.ids_inside(rect(50.0, 50.0, 50.0, 50.0), [
      #("row", rect(40.0, 40.0, 60.0, 60.0)),
    ])
    == []
}

pub fn a_region_from_either_pointer_order_has_the_same_bounds_test() {
  assert region.between(100.0, 80.0, 0.0, 0.0) == rect(0.0, 0.0, 100.0, 80.0)
}
