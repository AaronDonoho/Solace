import design
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import lustre/effect
import lustre/element
import solace_task

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn both_tabs_are_listed_test() {
  assert design.tabs() == [design.Architecture, design.UiUx]
  assert design.label(design.Architecture) == "Architecture"
  assert design.label(design.UiUx) == "UI/UX"
  let html = element.to_string(design.view(design.new(design.Architecture)))
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
    design.update(design.new(design.Architecture), design.Select(design.UiUx))
  assert model.tab == design.UiUx
  assert model.open == ""
  let #(model, request) =
    design.update(model, design.TabShown(design.Architecture))
  assert model.tab == design.Architecture
  assert request == effect.none()
}

pub fn register_outside_browser_test() {
  assert result.is_error(design.register())
}

/// Design's tab bar carries the shared tab styles, so it cannot drift from
/// the other tabbed pages.
pub fn tab_styles_are_shared_test() {
  let page = element.to_string(design.view(design.new(design.Architecture)))
  assert string.contains(page, solace_task.tab_styles)
}

pub fn each_tab_lists_only_its_own_designs_test() {
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(
        "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\"},{\"id\":\"fedcba9876543210\",\"title\":\"Plan\",\"tab\":\"architecture\"}]}",
      ),
    )
  assert design.designs_for(model, design.Architecture)
    == [design.Design("fedcba9876543210", "Plan", design.Architecture)]
  assert design.designs_for(model, design.UiUx)
    == [design.Design("0123456789abcdef", "Home", design.UiUx)]
}

pub fn a_tab_with_no_designs_says_so_test() {
  let html = element.to_string(design.view(design.new(design.Architecture)))
  assert string.contains(html, "No designs yet.")
}

pub fn picking_a_design_opens_it_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(
        "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\"}]}",
      ),
    )
  let #(model, _) =
    design.update(model, design.SelectDesign("0123456789abcdef"))
  assert design.open_design(model)
    == Some(design.Design("0123456789abcdef", "Home", design.UiUx))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Home</h2>")
}

pub fn an_address_round_trips_the_open_design_test() {
  assert design.design_from("?design=fedcba9876543210") == "fedcba9876543210"
  assert design.design_from("?tab=ui-ux&design=fedcba9876543210")
    == "fedcba9876543210"
  assert design.design_from("") == ""
}

pub fn a_bad_or_other_tab_design_opens_nothing_test() {
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(
        "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\"}]}",
      ),
    )
  let #(other_tab, _) =
    design.update(model, design.DesignShown("0123456789abcdef"))
  assert design.open_design(other_tab) == None
  let #(malformed, _) = design.update(model, design.DesignShown("../nope"))
  assert design.open_design(malformed) == None
  assert design.open_design(model) == None
}

pub fn a_design_that_disappears_closes_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(
        "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\"}]}",
      ),
    )
  let #(model, _) =
    design.update(model, design.SelectDesign("0123456789abcdef"))
  let #(model, _) =
    design.update(model, design.DesignsLoaded("{\"designs\":[]}"))
  assert design.open_design(model) == None
  assert !string.contains(
    element.to_string(design.view(model)),
    "<h2>Home</h2>",
  )
}

pub fn a_live_design_change_updates_the_list_test() {
  let #(model, _) =
    design.update(design.new(design.Architecture), design.CaughtUp)
  let #(model, _) =
    design.update(
      model,
      design.Live(
        "{\"designs\":[{\"id\":\"fedcba9876543210\",\"title\":\"Plan\",\"tab\":\"architecture\"}]}",
      ),
    )
  assert design.designs_for(model, design.Architecture)
    == [design.Design("fedcba9876543210", "Plan", design.Architecture)]
}

pub fn an_address_design_loads_before_the_list_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(design.new(design.UiUx), design.DesignShown(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Home</h2>")
  assert string.contains(html, "data-design-id=\"screen\"")
}

pub fn a_design_loads_and_draws_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Home</h2>")
  assert string.contains(html, "data-design-id=\"screen\"")
  assert string.contains(html, "data-design-id=\"content\"")
  assert list.length(string.split(html, "data-design-id=\"screen\"")) == 2
}

pub fn a_failed_design_load_shows_a_message_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(
      model,
      design.DesignLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown design\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Home</h2>")
  assert string.contains(html, "This design could not be loaded.")
  assert design.designs_for(model, design.UiUx) != []
}

pub fn a_late_design_reply_is_ignored_test() {
  let first = "0123456789abcdef"
  let second = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(first))
  let #(model, _) = design.update(model, design.SelectDesign(second))
  let #(model, _) =
    design.update(model, design.DesignLoaded(first, a_ui_design("late")))
  assert !string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"late\"",
  )
  let #(model, _) =
    design.update(model, design.DesignLoaded(second, a_ui_design("second")))
  assert string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"second\"",
  )
}

pub fn a_closed_design_clears_its_drawing_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, _) =
    design.update(model, design.DesignsLoaded("{\"designs\":[]}"))
  let html = element.to_string(design.view(model))
  assert design.open_design(model) == None
  assert !string.contains(html, "data-design-id=\"screen\"")
}

pub fn an_unknown_element_type_shows_a_message_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(
      model,
      design.DesignLoaded(id, a_ui_design_with_type("slider")),
    )
  assert string.contains(
    element.to_string(design.view(model)),
    "This design could not be loaded.",
  )
}

fn a_design_list() -> String {
  a_design_list_of([
    #("0123456789abcdef", "Home"),
    #("fedcba9876543210", "Plan"),
  ])
}

fn a_ui_design(element_id: String) -> String {
  a_ui_design_with_type_and_id("screen", element_id)
}

fn a_ui_design_with_type(ty: String) -> String {
  a_ui_design_with_type_and_id(ty, "screen")
}

fn a_ui_design_with_type_and_id(ty: String, element_id: String) -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\""
  <> element_id
  <> "\",\"type\":\""
  <> ty
  <> "\",\"properties\":{\"width\":\"fill\",\"height\":{\"fixed\":300}},\"children\":[{\"id\":\"content\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"align\":\"left\"}}]}]}"
}

pub fn several_screens_are_all_drawn_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let body =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"first\",\"type\":\"screen\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\"}},{\"id\":\"second\",\"type\":\"screen\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\"}}]}"
  let #(model, _) = design.update(model, design.DesignLoaded(id, body))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"first\"")
  assert string.contains(html, "data-design-id=\"second\"")
}

pub fn architecture_designs_keep_only_their_title_test() {
  let id = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(an_architecture_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, an_architecture_design()))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Plan</h2>")
  assert !string.contains(html, "data-design-id")
  assert !string.contains(html, "This design could not be loaded.")
  let #(model, _) =
    design.update(
      model,
      design.DesignLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Plan</h2>")
  assert !string.contains(html, "data-design-id")
  assert !string.contains(html, "This design could not be loaded.")
}

pub fn showing_the_open_design_again_keeps_its_drawing_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, request) = design.update(model, design.DesignShown(id))
  assert request == effect.none()
  assert string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"screen\"",
  )
}

fn an_architecture_list() -> String {
  "{\"designs\":[{\"id\":\"fedcba9876543210\",\"title\":\"Plan\",\"tab\":\"architecture\"}]}"
}

fn an_architecture_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"fedcba9876543210\",\"title\":\"Plan\",\"tab\":\"architecture\",\"elements\":[{\"id\":\"loose\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\"}}]}"
}

pub fn a_design_load_failure_shows_the_servers_reason_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(
      model,
      design.DesignLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown design 0123456789abcdef\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "This design could not be loaded.")
  assert string.contains(html, "unknown design")
  assert !string.contains(html, "E7023")
}

fn a_design_list_titled(title: String) -> String {
  a_design_list_of([#("0123456789abcdef", title)])
}

pub fn a_live_list_change_with_an_open_design_reloads_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(_, request) =
    design.update(model, design.Live(a_design_list_titled("Home revised")))
  assert request != effect.none()
}

pub fn a_live_list_without_changes_makes_no_request_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, settled) = design.update(model, design.CaughtUp)
  assert settled == effect.none()
  let #(_, request) = design.update(model, design.Live(a_design_list()))
  assert request == effect.none()
}

pub fn a_returning_design_reloads_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(model, request) =
    design.update(
      model,
      design.Live(a_design_list_of([#("fedcba9876543210", "Plan")])),
    )
  assert request == effect.none()
  let #(_, request) = design.update(model, design.Live(a_design_list()))
  assert request != effect.none()
}

pub fn a_caught_up_replay_that_changes_the_list_reloads_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.Live(a_design_list_titled("Home revised")))
  let #(_, request) = design.update(model, design.CaughtUp)
  assert request != effect.none()
}

fn a_design_list_of(designs: List(#(String, String))) -> String {
  "{\"designs\":["
  <> string.join(
    list.map(designs, fn(design) {
      let #(id, title) = design
      "{\"id\":\"" <> id <> "\",\"title\":\"" <> title <> "\",\"tab\":\"ui\"}"
    }),
    ",",
  )
  <> "]}"
}

fn a_version_list() -> String {
  "["
  <> "{\"number\":3,\"time\":1700000000,\"source\":{\"kind\":\"run\",\"id\":\"run\"}},"
  <> "{\"number\":2,\"time\":1600000000,\"source\":{\"kind\":\"human\"}},"
  <> "{\"number\":1,\"time\":1500000000,\"source\":{\"kind\":\"repo\"}}"
  <> "]"
}

fn a_version_list_with_four() -> String {
  "["
  <> "{\"number\":4,\"time\":1800000000,\"source\":{\"kind\":\"human\"}},"
  <> "{\"number\":3,\"time\":1700000000,\"source\":{\"kind\":\"run\",\"id\":\"run\"}},"
  <> "{\"number\":2,\"time\":1600000000,\"source\":{\"kind\":\"human\"}},"
  <> "{\"number\":1,\"time\":1500000000,\"source\":{\"kind\":\"repo\"}}"
  <> "]"
}

fn a_version_design(element_id: String) -> String {
  "{\"number\":1,\"time\":1500000000,\"source\":{\"kind\":\"repo\"},\"design\":"
  <> a_ui_design(element_id)
  <> "}"
}

fn a_design_with_history() -> design.Model {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  model
}

pub fn expanding_a_history_does_not_open_the_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, request) = design.update(model, design.ToggleHistory(id))
  assert request != effect.none()
  assert design.open_design(model) == None
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  assert string.contains(element.to_string(design.view(model)), "Version 3")
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  assert design.open_design(model) == None
  assert !string.contains(element.to_string(design.view(model)), "Version 3")
}

pub fn versions_show_newest_first_with_number_time_and_who_test() {
  let html = element.to_string(design.view(a_design_with_history()))
  assert string.contains(html, "Version 3")
  assert string.contains(html, "Version 2")
  assert string.contains(html, "Version 1")
  assert string.contains(html, "Agent")
  assert string.contains(html, "Person")
  assert string.contains(html, "Repo")
  assert string.contains(html, "datetime=\"2023-11-14T22:13:20.000Z\"")
  assert string.contains(html, "datetime=\"2020-09-13T12:26:40.000Z\"")
  assert string.contains(html, "datetime=\"2017-07-14T02:40:00.000Z\"")
  assert before(html, "Version 3", "Version 2")
  assert before(html, "Version 2", "Version 1")
}

pub fn clicking_a_version_shows_it_and_keeps_the_current_design_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let current = model.drawing
  let #(model, request) = design.update(model, design.ViewVersion(id, 1))
  assert request != effect.none()
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 1, a_version_design("older")))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"older\"")
  assert !string.contains(html, "data-design-id=\"screen\"")
  assert model.drawing == current
}

pub fn clicking_a_version_opens_its_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, request) = design.update(model, design.ViewVersion(id, 1))
  assert request != effect.none()
  assert design.open_design(model)
    == Some(design.Design(id, "Home", design.UiUx))
}

pub fn a_restore_adds_a_version_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 1, a_version_design("older")))
  let #(model, request) = design.update(model, design.Restore(id, 1))
  assert request != effect.none()
  let #(model, _) =
    design.update(model, design.Restored(id, 1, a_version_list_with_four()))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Version 4")
  assert !string.contains(html, "data-design-id=\"older\"")
  assert string.contains(html, "data-design-id=\"screen\"")
}

pub fn a_refused_restore_shows_its_message_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("older")))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_version_matching_the_current_design_has_no_restore_button_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 3))
  let #(model, _) =
    design.update(
      model,
      design.VersionLoaded(id, 3, a_version_design("screen")),
    )
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "Restore version 3")
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("older")))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Restore version 2")
}

pub fn a_late_version_reply_is_ignored_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("late")))
  assert !string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"late\"",
  )
  let #(model, _) =
    design.update(
      model,
      design.VersionLoaded(id, 1, a_version_design("chosen")),
    )
  assert string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"chosen\"",
  )
}

pub fn a_late_history_reply_is_ignored_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  assert !string.contains(element.to_string(design.view(model)), "Version 3")
}

pub fn a_failed_history_load_shows_the_servers_message_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.HistoryLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown design\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown design")
  assert !string.contains(html, "E7023")
}

pub fn a_restore_that_changes_nothing_is_success_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("older")))
  let #(model, _) =
    design.update(model, design.Restored(id, 2, a_version_list()))
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "role=\"alert\"")
  assert string.contains(html, "data-design-id=\"screen\"")
  assert string.contains(html, "Version 3")
}

fn before(text: String, first: String, second: String) -> Bool {
  case string.split_once(text, first) {
    Ok(#(_, after_first)) ->
      case string.split_once(after_first, second) {
        Ok(_) -> True
        Error(_) -> False
      }
    Error(_) -> False
  }
}

pub fn an_architecture_version_shows_only_its_title_test() {
  let id = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(an_architecture_list()),
    )
  let #(model, request) = design.update(model, design.SelectDesign(id))
  assert request != effect.none()
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, an_architecture_design()))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let version =
    "{\"number\":1,\"time\":1500000000,\"source\":{\"kind\":\"repo\"},\"design\":"
    <> an_architecture_design()
    <> "}"
  let #(model, _) = design.update(model, design.VersionLoaded(id, 1, version))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "<h2>Plan</h2>")
  assert !string.contains(html, "data-design-id")
  assert !string.contains(html, "Restore version 1")
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let different =
    "{\"number\":2,\"time\":1600000000,\"source\":{\"kind\":\"human\"},\"design\":"
    <> string.replace(an_architecture_design(), "loose", "other")
    <> "}"
  let #(model, _) = design.update(model, design.VersionLoaded(id, 2, different))
  assert string.contains(
    element.to_string(design.view(model)),
    "Restore version 2",
  )
}

pub fn expanding_without_an_open_design_survives_list_updates_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  assert string.contains(element.to_string(design.view(model)), "Version 3")
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(model, _) = design.update(model, design.Live(a_design_list()))
  assert string.contains(element.to_string(design.view(model)), "Version 3")
}

pub fn a_design_removed_from_the_list_loses_its_history_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  let #(model, _) =
    design.update(
      model,
      design.DesignsLoaded(a_design_list_of([#("fedcba9876543210", "Plan")])),
    )
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "Version 3")
}

pub fn a_refused_restore_after_collapsing_shows_its_message_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("older")))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_restore_reply_after_moving_on_updates_the_history_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 1, a_version_design("older")))
  let #(model, _) = design.update(model, design.Restore(id, 1))
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, request) =
    design.update(model, design.Restored(id, 1, a_version_list_with_four()))
  assert request != effect.none()
  assert string.contains(element.to_string(design.view(model)), "Version 4")
}

pub fn a_refused_restore_after_moving_on_shows_in_the_list_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 2, a_version_design("older")))
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
  let #(model, _) = design.update(model, design.ViewVersion(id, 3))
  assert !string.contains(
    element.to_string(design.view(model)),
    "unknown version 2",
  )
}

pub fn a_late_history_reply_does_not_replace_a_restored_list_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) =
    design.update(model, design.Restored(id, 1, a_version_list_with_four()))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  assert string.contains(element.to_string(design.view(model)), "Version 4")
}

pub fn a_refused_restore_after_moving_on_and_collapsing_shows_its_message_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.Restore(id, 2))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_failed_history_reply_keeps_a_restored_list_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) =
    design.update(model, design.Restored(id, 1, a_version_list_with_four()))
  let #(model, _) =
    design.update(
      model,
      design.HistoryLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown design\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Version 4")
  assert !string.contains(html, "unknown design")
  assert !string.contains(html, "E7023")
}

pub fn a_refused_restore_survives_the_design_loading_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_refused_restore_after_viewing_another_version_shows_its_message_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) = design.update(model, design.Restore(id, 2))
  let #(model, _) = design.update(model, design.ViewVersion(id, 3))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_refused_restore_on_the_architecture_tab_shows_its_message_test() {
  let id = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(an_architecture_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert !string.contains(html, "E7023")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

pub fn a_failed_history_load_survives_picking_the_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_design_with_history(), design.ToggleHistory(id))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.HistoryLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown design\"}",
      ),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown design")
  assert !string.contains(html, "E7023")
}

pub fn picking_a_design_clears_a_restore_refusal_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_design_with_history(), design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  assert string.contains(
    element.to_string(design.view(model)),
    "unknown version 2",
  )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  assert !string.contains(
    element.to_string(design.view(model)),
    "unknown version 2",
  )
}

pub fn moving_on_clears_a_restore_refusal_held_on_another_design_test() {
  let id = "0123456789abcdef"
  let other = "fedcba9876543210"
  let refusal = "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}"
  let base = a_design_with_history()
  let #(refused, _) = design.update(base, design.Restored(id, 2, refusal))
  assert string.contains(
    element.to_string(design.view(refused)),
    "unknown version 2",
  )
  let #(selected, _) = design.update(refused, design.SelectDesign(other))
  assert !string.contains(
    element.to_string(design.view(selected)),
    "unknown version 2",
  )
  let #(refused, _) = design.update(base, design.Restored(id, 2, refusal))
  let #(viewed, _) = design.update(refused, design.ViewVersion(other, 1))
  assert !string.contains(
    element.to_string(design.view(viewed)),
    "unknown version 2",
  )
  let #(refused, _) = design.update(base, design.Restored(id, 2, refusal))
  let #(shown, _) = design.update(refused, design.DesignShown(other))
  assert !string.contains(
    element.to_string(design.view(shown)),
    "unknown version 2",
  )
}

pub fn a_refused_restore_survives_the_history_list_reloading_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let #(model, _) = design.update(model, design.Restore(id, 2))
  let #(model, _) = design.update(model, design.ViewVersion(id, 3))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(
      model,
      design.Restored(
        id,
        2,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown version 2\"}",
      ),
    )
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list_with_four()))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "unknown version 2")
  assert list.length(string.split(html, "unknown version 2")) == 2
}

fn a_linked_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"one\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":{\"fixed\":300}},\"children\":[{\"id\":\"go\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Next\",\"navigates_to\":\"two\"}},{\"id\":\"act\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Act\",\"action\":\"save\"}}]},{\"id\":\"two\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":{\"fixed\":300}},\"children\":[{\"id\":\"back\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Back\",\"navigates_to\":\"one\"}}]}]}"
}

fn a_preview_model() -> design.Model {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_linked_design()))
  model
}

pub fn the_preview_button_shows_the_first_screen_test() {
  let model = a_preview_model()
  let html = element.to_string(design.view(model))
  assert string.contains(html, ">Preview<")
  assert string.contains(html, "data-design-id=\"one\"")
  assert string.contains(html, "data-design-id=\"two\"")
  let #(model, _) = design.update(model, design.Preview)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"one\"")
  assert !string.contains(html, "data-design-id=\"two\"")
  assert string.contains(html, ">Exit preview<")
}

pub fn clicking_a_link_shows_the_target_screen_test() {
  let #(model, _) = design.update(a_preview_model(), design.Preview)
  let #(model, _) = design.update(model, design.PreviewClick("go"))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"two\"")
  assert !string.contains(html, "data-design-id=\"one\"")
}

pub fn clicking_an_action_changes_nothing_test() {
  let #(model, _) = design.update(a_preview_model(), design.Preview)
  let before = element.to_string(design.view(model))
  let #(model, _) = design.update(model, design.PreviewClick("act"))
  assert element.to_string(design.view(model)) == before
}

pub fn exiting_the_preview_shows_every_screen_test() {
  let #(model, _) = design.update(a_preview_model(), design.Preview)
  let #(model, _) = design.update(model, design.ExitPreview)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"one\"")
  assert string.contains(html, "data-design-id=\"two\"")
  assert string.contains(html, ">Preview<")
}

pub fn a_preview_leaves_when_its_screen_disappears_test() {
  let #(model, _) = design.update(a_preview_model(), design.Preview)
  let #(model, _) = design.update(model, design.PreviewClick("go"))
  let #(model, _) =
    design.update(
      model,
      design.DesignLoaded("0123456789abcdef", a_ui_design("screen")),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"screen\"")
  assert string.contains(html, ">Preview<")
}
