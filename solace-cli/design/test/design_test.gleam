import design
import gleam/list
import gleam/result
import gleam/string
import gleeunit
import lustre/element
import solace_task

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn both_tabs_are_listed_test() {
  assert design.tabs() == [design.Architecture, design.UiUx]
  assert design.label(design.Architecture) == "Architecture"
  assert design.label(design.UiUx) == "UI/UX"
  let html = element.to_string(design.view(design.Model(design.Architecture)))
  assert string.contains(html, "Architecture")
  assert string.contains(html, "UI/UX")
}

pub fn a_tab_round_trips_through_the_address_test() {
  list.each(design.tabs(), fn(tab) {
    assert design.tab_from("?tab=" <> design.tab_name(tab)) == tab
  })
}

pub fn an_unknown_tab_falls_back_to_the_first_test() {
  assert design.tab_from("?tab=nope") == design.Architecture
  assert design.tab_from("") == design.Architecture
}

pub fn a_tab_choice_changes_the_page_test() {
  let #(model, _) =
    design.update(design.Model(design.Architecture), design.Select(design.UiUx))
  assert model == design.Model(design.UiUx)
  let #(model, _) = design.update(model, design.TabShown(design.Architecture))
  assert model == design.Model(design.Architecture)
}

pub fn register_outside_browser_test() {
  assert result.is_error(design.register())
}

/// Design's tab bar carries the shared tab styles, so it cannot drift from
/// the other tabbed pages.
pub fn tab_styles_are_shared_test() {
  let page = element.to_string(design.view(design.Model(design.Architecture)))
  assert string.contains(page, solace_task.tab_styles)
}
