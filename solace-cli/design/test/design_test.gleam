import design
import editing
import gleam/dict
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import lustre/dev/query
import lustre/dev/simulate
import lustre/effect
import lustre/element
import panel
import render
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
    == [design.Design("fedcba9876543210", "Plan", design.Architecture, 0)]
  assert design.designs_for(model, design.UiUx)
    == [design.Design("0123456789abcdef", "Home", design.UiUx, 0)]
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
    == Some(design.Design("0123456789abcdef", "Home", design.UiUx, 0))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Home</button>")
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
    == [design.Design("fedcba9876543210", "Plan", design.Architecture, 0)]
}

pub fn an_address_design_loads_before_the_list_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(design.new(design.UiUx), design.DesignShown(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Home</button>")
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
  assert string.contains(html, "Home</button>")
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
  assert string.contains(html, "Home</button>")
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
  "{\"schemaVersion\":1,\"id\":\"fedcba9876543210\",\"title\":\"Plan\",\"tab\":\"architecture\",\"elements\":[{\"id\":\"flow\",\"type\":\"flow\",\"properties\":{},\"children\":[{\"id\":\"start\",\"type\":\"node\",\"properties\":{\"label\":\"Start\",\"description\":\"The first step\"}},{\"id\":\"end\",\"type\":\"node\",\"properties\":{\"label\":\"End\",\"description\":\"The last step\"}},{\"id\":\"step\",\"type\":\"edge\",\"properties\":{\"from\":\"start\",\"to\":\"end\",\"label\":\"Next\"}}]}]}"
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
  let #(model, _) = design.update(model, design.CaughtUp)
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
      "{\"id\":\""
      <> id
      <> "\",\"title\":\""
      <> title
      <> "\",\"tab\":\"ui\",\"version\":0}"
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
    == Some(design.Design(id, "Home", design.UiUx, 0))
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
  assert string.contains(
    html,
    "class=\"design-element design-wireframe\" data-design-id=\"one\"",
  )
  let #(model, _) = design.update(model, design.Preview)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"one\"")
  assert !string.contains(html, "data-design-id=\"two\"")
  assert !string.contains(
    html,
    "class=\"design-element design-wireframe\" data-design-id=\"one\"",
  )
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

pub fn an_architecture_design_draws_its_flow_test() {
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
  assert string.contains(html, "Plan</button>")
  assert string.contains(html, "data-design-id=\"flow\"")
  assert string.contains(html, "data-design-id=\"start\"")
  assert string.contains(html, "data-design-id=\"end\"")
  assert string.contains(html, "data-design-id=\"step\"")
  assert string.contains(html, ">Start<")
  assert string.contains(html, ">The first step<")
  assert string.contains(html, ">Next<")
  assert string.contains(html, "d=\"M 244.0 84.0 H 294.0 V 84.0 H 344.0\"")
}

pub fn an_architecture_load_failure_shows_the_servers_reason_test() {
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
      design.DesignLoaded(
        id,
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown\"}",
      ),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Plan</button>")
  assert string.contains(html, "unknown")
  assert !string.contains(html, "E7023")
  assert !string.contains(html, "data-design-id")
}

pub fn an_architecture_version_draws_its_flow_test() {
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
  assert string.contains(html, "data-design-id=\"start\"")
  assert string.contains(html, "data-design-id=\"step\"")
  assert !string.contains(html, "Restore version 1")
  assert !string.contains(html, "class=\"design-changes\"")
  assert !string.contains(html, ">Preview<")
  let #(model, _) = design.update(model, design.ViewVersion(id, 2))
  let different =
    "{\"number\":2,\"time\":1600000000,\"source\":{\"kind\":\"human\"},\"design\":"
    <> string.replace(an_architecture_design(), "Start", "Begin")
    <> "}"
  let #(model, _) = design.update(model, design.VersionLoaded(id, 2, different))
  let #(model, _) =
    design.update(
      model,
      design.PreviousLoaded(id, 2, a_version_at(1, an_architecture_design())),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "class=\"design-changes\"")
  assert string.contains(html, "design-change-changed")
  assert !string.contains(html, ">Preview<")
  assert string.contains(html, "Restore version 2")
}

fn a_create_proposal_model() -> design.Model {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.Replied(True, a_create_run())),
    )
  model
}

fn a_create_run() -> String {
  "{\"id\":\"r9\",\"kind\":\"design\",\"model\":\"fake\",\"state\":\"waiting\",\"started\":1,\"context\":{\"kind\":\"page\"},\"notes\":[],\"questions\":[{\"id\":\"q9\",\"text\":\"Create it\",\"options\":[\"Approve\",\"Reject\"],\"proposal\":{\"kind\":\"design\",\"title\":\"New\",\"tab\":\"ui\",\"operations\":[],\"design\":{\"schemaVersion\":1,\"id\":\"fedcba9876543210\",\"title\":\"New\",\"tab\":\"ui\",\"elements\":[{\"id\":\"newroot\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[]}]}},\"time\":1,\"after\":0}]}"
}

pub fn the_preview_button_uses_the_shared_button_look_test() {
  let page = element.to_string(design.view(a_preview_model()))
  assert string.contains(page, solace_task.control_styles)
  assert string.contains(
    page,
    ".design-preview{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}",
  )
}

pub fn the_changes_button_uses_the_shared_button_look_test() {
  let page = element.to_string(design.view(a_create_proposal_model()))
  assert string.contains(page, solace_task.control_styles)
  assert string.contains(
    page,
    ".design-changes{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}",
  )
  assert string.contains(page, "class=\"design-changes\"")
  assert string.contains(page, "aria-pressed=\"true\"")
  assert string.contains(page, "aria-label=\"Hide changes\"")
  assert string.contains(page, "title=\"Hide changes\"")
  assert string.contains(page, "🔍")
  assert !string.contains(page, "class=\"design-choice\"")
}

pub fn the_changes_button_labels_both_states_test() {
  let #(model, _) =
    design.update(a_create_proposal_model(), design.ToggleChanges)
  let page = element.to_string(design.view(model))
  assert string.contains(page, "aria-pressed=\"false\"")
  assert string.contains(page, "aria-label=\"Show changes\"")
  assert string.contains(page, "title=\"Show changes\"")
  assert string.contains(
    page,
    ".design-changes[aria-pressed=true]{background:var(--edge);color:var(--paper);border-color:var(--edge)}",
  )
}

/// A run already waiting when the page first loads is not new, so catching up
/// flags no attention.
pub fn a_caught_up_replay_of_a_waiting_run_flags_no_attention_test() {
  let model = design.new(design.UiUx)
  let #(replayed, _) =
    design.update(
      model,
      design.Live("{\"designRun\":{\"id\":\"r1\",\"state\":\"waiting\"}}"),
    )
  let #(_, fx) = design.settle(replayed)
  assert fx == effect.none()
}

fn a_selection_simulation(model: design.Model) {
  simulate.start(
    simulate.application(
      fn(_) { #(model, effect.none()) },
      design.update,
      design.view,
    ),
    Nil,
  )
}

fn element_with_id(id: String) -> query.Query {
  query.element(query.data("design-id", id))
}

pub fn a_click_selects_the_clicked_element_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(a_preview_model()),
      on: element_with_id("go"),
    )
  assert simulate.model(simulation).selected == ["go"]
  assert string.contains(
    element.to_string(simulate.view(simulation)),
    "design-selected",
  )
}

pub fn a_click_on_a_nested_element_selects_itself_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(a_preview_model()),
      on: element_with_id("go"),
    )
  let selected = simulate.model(simulation).selected
  assert selected == ["go"]
  assert !list.contains(selected, "one")
}

pub fn a_plain_click_replaces_and_a_ctrl_click_adds_or_removes_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(a_preview_model()),
      on: element_with_id("go"),
    )
  let simulation = simulate.click(simulation, on: element_with_id("act"))
  assert simulate.model(simulation).selected == ["act"]
  let ctrl_click = fn(simulation) {
    simulate.event(simulation, on: element_with_id("go"), name: "click", data: [
      #("ctrlKey", json.bool(True)),
    ])
  }
  let simulation = ctrl_click(simulation)
  assert simulate.model(simulation).selected == ["act", "go"]
  let simulation = ctrl_click(simulation)
  assert simulate.model(simulation).selected == ["act"]
}

pub fn escape_or_empty_canvas_clears_the_selection_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(a_preview_model()),
      on: element_with_id("go"),
    )
  let escaped = simulate.message(simulation, design.Escape)
  assert simulate.model(escaped).selected == []
  let cleared =
    simulate.click(simulation, on: query.element(query.class("design-screens")))
  assert simulate.model(cleared).selected == []
}

pub fn preview_clicks_still_navigate_test() {
  let #(model, _) = design.update(a_preview_model(), design.Preview)
  let simulation =
    simulate.click(a_selection_simulation(model), on: element_with_id("go"))
  assert simulate.model(simulation).preview == Some("two")
  assert simulate.model(simulation).selected == []
}

pub fn opening_another_design_clears_the_selection_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  assert model.selected == ["go"]
  let #(model, _) =
    design.update(model, design.SelectDesign("fedcba9876543210"))
  assert model.selected == []
}

pub fn a_reload_that_removes_a_selected_element_clears_it_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  assert model.selected == []
}

pub fn a_reload_keeps_a_selection_only_the_proposal_has_test() {
  let id = "0123456789abcdef"
  let assert Ok(proposal) = render.decode(a_ui_design("proposal"))
  let base = a_preview_model()
  let model =
    design.Model(
      ..base,
      view_proposal: True,
      panel: panel.Model(..base.panel, proposals: [
        panel.Proposed("r9", "ui", proposal),
      ]),
    )
  let #(model, _) =
    design.update(model, design.ElementClicked("proposal", False))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_linked_design()))
  assert model.selected == ["proposal"]
}

pub fn a_reload_keeps_a_selection_only_a_viewed_version_has_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_design_with_history(), design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(model, design.VersionLoaded(id, 1, a_version_design("older")))
  let #(model, _) = design.update(model, design.ElementClicked("older", False))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("screen")))
  assert model.selected == ["older"]
}

pub fn a_region_replaces_the_selection_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.RegionSelected(["one", "two"], False))
  assert model.selected == ["one", "two"]
}

pub fn a_ctrl_region_adds_new_ids_and_an_empty_region_clears_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.RegionSelected(["one", "two"], True))
  assert model.selected == ["go", "one", "two"]
  let #(model, _) = design.update(model, design.RegionSelected(["two"], True))
  assert model.selected == ["go", "one", "two"]
  let #(model, _) = design.update(model, design.RegionSelected([], True))
  assert model.selected == []
}

pub fn a_note_box_and_marker_follow_the_selection_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  assert string.contains(
    element.to_string(design.view(model)),
    "id=\"design-note-text\"",
  )
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Rename it")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  assert model.panel.notes
    == [panel.Note("0123456789abcdef", ["go"], "Rename it")]
  let rendered = element.to_string(design.view(model))
  assert string.contains(rendered, "data-note=\"1\"")
  assert string.contains(rendered, "Rename it")
  let #(model, _) = design.update(model, design.Panel(panel.RemoveNote(0)))
  let rendered = element.to_string(design.view(model))
  assert !string.contains(rendered, "data-note=")
  assert !string.contains(rendered, "Rename it")
}

pub fn a_note_on_another_design_has_no_marker_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Other design")))
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("another-design", ["go"])))
  let rendered = element.to_string(design.view(model))
  assert string.contains(rendered, "class=\"design-note-row\"")
  assert !string.contains(rendered, "data-note=")
}

pub fn notes_on_the_same_element_show_every_marker_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("First")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("Second")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  let rendered = element.to_string(design.view(model))
  assert string.contains(rendered, "data-note=\"1, 2\"")
  assert string.contains(rendered, "First")
  assert string.contains(rendered, "Second")
  let #(model, _) = design.update(model, design.Panel(panel.RemoveNote(0)))
  let rendered = element.to_string(design.view(model))
  assert string.contains(rendered, "data-note=\"1\"")
}

fn an_architecture_model() -> design.Model {
  let id = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.DesignsLoaded(an_architecture_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, an_architecture_design()))
  model
}

pub fn an_architecture_node_selects_by_id_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(an_architecture_model()),
      on: element_with_id("start"),
    )
  assert simulate.model(simulation).selected == ["start"]
  assert string.contains(
    element.to_string(simulate.view(simulation)),
    "design-node design-selected",
  )
}

pub fn an_architecture_edge_selects_by_id_test() {
  let simulation =
    simulate.click(
      a_selection_simulation(an_architecture_model()),
      on: element_with_id("step"),
    )
  assert simulate.model(simulation).selected == ["step"]
  assert string.contains(
    element.to_string(simulate.view(simulation)),
    "design-edge design-selected",
  )
}

pub fn an_architecture_region_selects_nodes_and_edges_test() {
  let model = an_architecture_model()
  let html = element.to_string(design.view(model))
  assert string.contains(tag_of(html, "flow"), "data-region")
  assert string.contains(tag_of(html, "step"), "class=\"design-edge\"")
  assert string.contains(
    tag_of(html, "start"),
    "class=\"design-element design-node\"",
  )
  assert string.contains(
    tag_of(html, "end"),
    "class=\"design-element design-node\"",
  )
  assert before(html, "data-design-id=\"step\"", "data-design-id=\"start\"")
  assert before(html, "data-design-id=\"start\"", "data-design-id=\"end\"")
  let #(model, _) =
    design.update(model, design.RegionSelected(["step", "start", "end"], False))
  assert model.selected == ["step", "start", "end"]
}

pub fn an_architecture_note_holds_the_selected_edge_test() {
  let #(model, _) =
    design.update(an_architecture_model(), design.ElementClicked("step", False))
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Name this arrow")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  assert model.panel.notes
    == [panel.Note("fedcba9876543210", ["step"], "Name this arrow")]
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-note=\"1\"")
  assert string.contains(html, "class=\"design-edge-note\"")
  assert string.contains(html, ">1</text>")
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("Second")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  assert string.contains(element.to_string(design.view(model)), ">1, 2</text>")
}

fn tag_of(html: String, id: String) -> String {
  let marker = "data-design-id=\"" <> id <> "\""
  case string.split_once(html, marker) {
    Ok(#(before, after)) ->
      case list.last(string.split(before, "<")), string.split_once(after, ">") {
        Ok(tag), Ok(#(rest, _)) -> tag <> marker <> rest
        _, _ -> ""
      }
    Error(_) -> ""
  }
}

fn a_design_list_versioned(version: Int) -> String {
  "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"version\":"
  <> int.to_string(version)
  <> "}]}"
}

pub fn a_live_version_change_refreshes_an_expanded_entry_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(1)),
    )
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  let #(model, request) =
    design.update(model, design.Live(a_design_list_versioned(2)))
  assert request != effect.none()
  assert list.contains(model.expanded, id)
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list_with_four()))
  assert string.contains(element.to_string(design.view(model)), "Version 4")
}

pub fn a_live_version_change_refreshes_an_expanded_entry_with_an_open_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(1)),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(model, _) = design.update(model, design.ToggleHistory(id))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list()))
  let #(model, request) =
    design.update(model, design.Live(a_design_list_versioned(2)))
  assert request != effect.none()
  assert list.contains(model.expanded, id)
  assert design.open_design(model)
    == Some(design.Design(id, "Home", design.UiUx, 2))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(id, a_version_list_with_four()))
  assert string.contains(element.to_string(design.view(model)), "Version 4")
}

pub fn the_title_becomes_a_text_box_and_saves_the_new_title_test() {
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.RenameStart)
  assert model.renaming == Some("Home")
  assert string.contains(
    element.to_string(design.view(model)),
    "design-title-input",
  )
  let #(model, request) = design.update(model, design.RenameSaved("Homepage"))
  assert request != effect.none()
  assert model.renaming == None
  let #(model, _) = design.update(model, design.RenameStart)
  let #(model, _) = design.update(model, design.RenameKey("Escape"))
  assert model.renaming == None
}

pub fn delete_asks_then_closes_the_design_and_undo_brings_it_back_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.DeleteAsk)
  assert model.confirm_delete
  assert string.contains(
    element.to_string(design.view(model)),
    "Delete this design and all its versions?",
  )
  let #(model, request) = design.update(model, design.Delete)
  assert request != effect.none()
  assert !model.confirm_delete
  let #(model, _) =
    design.update(model, design.Deleted(id, a_design_list_of([])))
  assert model.open == ""
  assert model.undo == Some(id)
  assert design.open_design(model) == None
  assert string.contains(element.to_string(design.view(model)), "Undo</button>")
  let #(model, request) = design.update(model, design.UndoDelete(id))
  assert request != effect.none()
  let #(model, _) = design.update(model, design.Undone(a_design_list()))
  assert model.undo == None
  assert list.any(design.designs_for(model, design.UiUx), fn(design) {
    design.id == id
  })
  let #(model, _) =
    design.update(model, design.Deleted(id, a_design_list_of([])))
  assert string.contains(element.to_string(design.view(model)), "Undo</button>")
  let #(model, _) = design.update(model, design.UndoExpired(id))
  assert model.undo == None
  assert !string.contains(
    element.to_string(design.view(model)),
    "Undo</button>",
  )
}

pub fn deleting_a_design_drops_its_waiting_notes_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Move it")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  assert model.panel.notes != []
  let #(model, _) =
    design.update(model, design.Deleted(id, a_design_list_of([])))
  assert model.panel.notes == []
}

pub fn deleting_one_design_keeps_another_designs_history_test() {
  let a = "0123456789abcdef"
  let b = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(a))
  let #(model, _) =
    design.update(model, design.DesignLoaded(a, a_ui_design("screen")))
  let #(model, _) = design.update(model, design.ToggleHistory(b))
  let #(model, _) =
    design.update(model, design.HistoryLoaded(b, a_version_list()))
  let #(model, _) =
    design.update(model, design.Deleted(a, a_design_list_of([#(b, "Plan")])))
  assert model.open == ""
  assert list.contains(model.expanded, b)
  assert list.any(model.histories, fn(history) { history.id == b })
  assert list.any(design.designs_for(model, design.UiUx), fn(design) {
    design.id == b
  })
}

pub fn deleting_another_design_keeps_an_open_proposals_changes_test() {
  let a = "0123456789abcdef"
  let b = "fedcba9876543210"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(b))
  let #(model, _) =
    design.update(model, design.Panel(panel.Replied(True, a_create_run())))
  assert string.contains(element.to_string(design.view(model)), "New")
  let #(model, _) =
    design.update(model, design.Deleted(a, a_design_list_of([#(b, "Plan")])))
  assert model.open == b
  assert string.contains(element.to_string(design.view(model)), "New")
}

pub fn a_failed_delete_keeps_the_design_open_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let #(model, _) = design.update(model, design.DeleteAsk)
  let #(model, _) =
    design.update(
      model,
      design.Deleted(
        id,
        "{\"code\":\"E7001\",\"message\":\"E7001 the design could not be deleted\"}",
      ),
    )
  assert model.open == id
  assert model.undo == None
  assert !model.confirm_delete
  assert string.contains(
    element.to_string(design.view(model)),
    "could not be deleted",
  )
}

pub fn loading_a_list_keeps_a_note_for_a_design_it_does_not_list_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.NoteText("Keep me")),
    )
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote("deadbeefdeadbeef", ["gone"])),
    )
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  assert model.panel.notes
    == [panel.Note("deadbeefdeadbeef", ["gone"], "Keep me")]
}

pub fn a_live_version_change_reloads_the_open_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(1)),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(_, request) =
    design.update(model, design.Live(a_design_list_versioned(2)))
  assert request != effect.none()
}

pub fn the_note_panel_is_see_through_and_movable_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let page = element.to_string(design.view(model))
  assert string.contains(
    page,
    "background:color-mix(in srgb,var(--paper) 60%,transparent);border:2px solid #c2410c;box-shadow",
  )
  assert !string.contains(page, "border:2px solid #c2410c;opacity")
  assert string.contains(page, "class=\"design-note-handle\"")
  assert string.contains(page, "data-note-handle>Move</div>")
  assert string.contains(page, ">Move<")
}

pub fn the_add_note_button_uses_the_shared_button_look_test() {
  let #(model, _) =
    design.update(a_preview_model(), design.ElementClicked("go", False))
  let page = element.to_string(design.view(model))
  assert string.contains(page, solace_task.control_styles)
  assert string.contains(
    page,
    ".design-note button{align-self:flex-end;padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}",
  )
  assert !string.contains(page, ".design-note button:disabled")
  assert !string.contains(page, "background:#c2410c;border:1px solid #c2410c")
}

fn a_version_at(number: Int, drawing: String) -> String {
  "{\"number\":"
  <> int.to_string(number)
  <> ",\"time\":1600000000,\"source\":{\"kind\":\"human\"},\"design\":"
  <> drawing
  <> "}"
}

pub fn every_version_after_the_first_shows_changes_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_design_with_history(), design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(
      model,
      design.VersionLoaded(id, 2, a_version_at(2, a_ui_design("second"))),
    )
  let #(model, _) =
    design.update(
      model,
      design.PreviousLoaded(id, 2, a_version_at(1, a_ui_design("first"))),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "class=\"design-changes\"")
  assert string.contains(html, "aria-label=\"Hide changes\"")
  assert string.contains(
    html,
    "class=\"design-element design-change-added design-wireframe\" data-design-id=\"second\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-removed design-wireframe\" data-design-id=\"first\"",
  )
  let #(model, _) = design.update(model, design.ViewVersion(id, 1))
  let #(model, _) =
    design.update(
      model,
      design.VersionLoaded(id, 1, a_version_at(1, a_ui_design("first"))),
    )
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "class=\"design-changes\"")
}

pub fn the_open_design_shows_changes_against_the_previous_version_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(2)),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("second")))
  let #(model, _) =
    design.update(
      model,
      design.PreviousLoaded(id, 2, a_version_at(1, a_ui_design("first"))),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "class=\"design-changes\"")
  assert string.contains(
    html,
    "class=\"design-element design-change-added design-wireframe\" data-design-id=\"second\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-removed design-wireframe\" data-design-id=\"first\"",
  )
  let #(single, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(1)),
    )
  let #(single, _) = design.update(single, design.SelectDesign(id))
  let #(single, _) =
    design.update(single, design.DesignLoaded(id, a_ui_design("only")))
  let html = element.to_string(design.view(single))
  assert !string.contains(html, "class=\"design-changes\"")
}

pub fn a_history_version_previews_its_own_screens_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(a_design_with_history(), design.ViewVersion(id, 2))
  let #(model, _) =
    design.update(
      model,
      design.VersionLoaded(id, 2, a_version_at(2, a_linked_design())),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, ">Preview<")
  assert string.contains(html, "data-design-id=\"one\"")
  assert string.contains(html, "data-design-id=\"two\"")
  let #(model, _) = design.update(model, design.Preview)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"one\"")
  assert !string.contains(html, "data-design-id=\"two\"")
  assert string.contains(html, ">Exit preview<")
  let #(model, _) = design.update(model, design.PreviewClick("go"))
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"two\"")
  assert !string.contains(html, "data-design-id=\"one\"")
}

pub fn a_previous_version_for_an_older_shown_version_marks_no_changes_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list_versioned(1)),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("first")))
  let #(model, _) =
    design.update(
      model,
      design.PreviousLoaded(id, 1, a_version_at(0, a_ui_design("zeroth"))),
    )
  let #(model, _) =
    design.update(model, design.DesignsLoaded(a_design_list_versioned(3)))
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "data-design-id=\"zeroth\"")
  assert !string.contains(html, "class=\"design-element design-change-removed")
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_ui_design("third")))
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "class=\"design-element design-change-added")
  assert !string.contains(html, "class=\"design-element design-change-removed")
  let #(model, _) =
    design.update(
      model,
      design.PreviousLoaded(id, 3, a_version_at(2, a_ui_design("second"))),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(html, "class=\"design-changes\"")
  assert string.contains(
    html,
    "class=\"design-element design-change-removed design-wireframe\" data-design-id=\"second\"",
  )
  assert !string.contains(html, "data-design-id=\"first\"")
}

fn a_button_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"screen\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":{\"fixed\":300}},\"children\":[{\"id\":\"go\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Go\"}}]}]}"
}

fn an_editing_model() -> design.Model {
  let #(model, _) = design.update(a_selected_model(), design.EditStart)
  model
}

fn waiting_json(model: design.Model, id: String) -> String {
  case dict.get(model.waiting, id) {
    Ok(changes) -> json.to_string(editing.operations(changes))
    Error(_) -> "{}"
  }
}

@external(javascript, "./draft_storage.mjs", "install")
fn install_edits() -> Nil

@external(javascript, "./draft_storage.mjs", "restore")
fn restore_edits() -> Nil

pub fn changing_a_text_and_a_color_waits_for_save_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) =
    design.update(model, design.EditProperty("style", "fill", "#123456"))
  let body = waiting_json(model, id)
  assert string.contains(body, "\"op\":\"set\"")
  assert string.contains(body, "\"property\":\"label\"")
  assert string.contains(body, "\"value\":\"Go now\"")
  assert string.contains(body, "\"property\":\"fill\"")
  assert string.contains(body, "\"value\":\"#123456\"")
  assert list.length(string.split(body, "\"op\":\"set\"")) == 3
  let html = element.to_string(design.view(model))
  assert string.contains(html, "Go now")
  assert string.contains(html, "background:#123456")
}

pub fn a_refused_save_keeps_the_drawing_and_waiting_changes_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let saved = model.drawing
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let waiting = waiting_json(model, id)
  let #(model, _) = design.update(model, design.SaveEdits)
  let #(model, _) =
    design.update(
      model,
      design.EditsSaved(
        id,
        "{\"code\":\"E7001\",\"message\":\"E7001 that label is too long\"}",
        [editing.text_change("go", "properties", "label", "Go now")],
      ),
    )
  assert model.drawing == saved
  assert waiting_json(model, id) == waiting
  assert string.contains(
    element.to_string(design.view(model)),
    "that label is too long",
  )
}

pub fn waiting_changes_survive_a_reload_and_another_design_test() {
  install_edits()
  let id = "0123456789abcdef"
  let other = "fedcba9876543210"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let waiting = waiting_json(model, id)
  let #(reloaded, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(reloaded, _) = design.update(reloaded, design.SelectDesign(id))
  let #(reloaded, _) =
    design.update(reloaded, design.DesignLoaded(id, a_button_design()))
  assert waiting_json(reloaded, id) == waiting
  let #(elsewhere, _) = design.update(reloaded, design.SelectDesign(other))
  let #(back, _) = design.update(elsewhere, design.SelectDesign(id))
  let #(back, _) =
    design.update(back, design.DesignLoaded(id, a_button_design()))
  assert waiting_json(back, id) == waiting
  restore_edits()
}

pub fn the_panel_has_edit_and_agent_tabs_test() {
  let model = an_editing_model()
  let html = element.to_string(design.view(model))
  assert string.contains(html, ">Edit</button>")
  assert string.contains(html, ">Agent</button>")
  assert string.contains(html, "id=\"design-text\"")
  let #(model, _) = design.update(model, design.EditTab)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "id=\"edit-properties-label\"")
  assert !string.contains(html, "id=\"design-text\"")
  let #(model, _) = design.update(model, design.AgentTab)
  assert string.contains(
    element.to_string(design.view(model)),
    "id=\"design-text\"",
  )
}

pub fn a_refused_rename_keeps_the_drawing_test() {
  let id = "0123456789abcdef"
  let model = a_design_with_history()
  let saved = model.drawing
  let #(model, _) = design.update(model, design.RenameSaved("Homepage"))
  let #(model, _) =
    design.update(
      model,
      design.Renamed(
        id,
        "{\"code\":\"E7001\",\"message\":\"E7001 the title was refused\"}",
      ),
    )
  assert model.drawing == saved
  assert string.contains(
    element.to_string(design.view(model)),
    "the title was refused",
  )
}

pub fn whole_number_properties_save_as_numbers_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) = design.update(model, design.ElementClicked("screen", False))
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "width", "200"))
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "gap", "8"))
  let #(model, _) =
    design.update(model, design.EditProperty("style", "size", "16"))
  let body = waiting_json(model, id)
  assert string.contains(body, "\"value\":{\"fixed\":200}")
  assert string.contains(body, "\"value\":8")
  assert string.contains(body, "\"value\":16")
  assert !string.contains(body, "\"value\":\"8\"")
  assert !string.contains(body, "\"value\":\"16\"")
  let html = element.to_string(design.view(model))
  assert string.contains(html, "width:200.0px")
  assert string.contains(html, "gap:8.0px")
  assert string.contains(html, "font-size:16.0px")
}

pub fn a_save_keeps_a_change_made_while_it_was_in_flight_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let sent = [editing.text_change("go", "properties", "label", "Go now")]
  let #(model, _) = design.update(model, design.SaveEdits)
  let #(model, _) =
    design.update(model, design.EditProperty("style", "fill", "#123456"))
  let #(model, _) =
    design.update(model, design.EditsSaved(id, a_button_design(), sent))
  let body = waiting_json(model, id)
  assert string.contains(body, "\"property\":\"fill\"")
  assert !string.contains(body, "\"property\":\"label\"")
  assert string.contains(
    element.to_string(design.view(model)),
    "background:#123456",
  )
}

pub fn a_save_reply_after_opening_another_design_clears_its_changes_test() {
  let id = "0123456789abcdef"
  let other = "fedcba9876543210"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let sent = [editing.text_change("go", "properties", "label", "Go now")]
  let #(model, _) = design.update(model, design.SelectDesign(other))
  let #(model, _) =
    design.update(model, design.EditsSaved(id, a_button_design(), sent))
  assert !string.contains(waiting_json(model, id), "\"property\":\"label\"")
}

pub fn a_short_hex_color_fills_the_swatch_test() {
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("style", "fill", "#abc"))
  let #(model, _) = design.update(model, design.EditTab)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "id=\"edit-style-fill-pick\"")
  assert string.contains(html, "value=\"#aabbcc\"")
}

pub fn whole_number_changes_survive_a_reload_test() {
  install_edits()
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) = design.update(model, design.ElementClicked("screen", False))
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "width", "200"))
  let waiting = waiting_json(model, id)
  let #(reloaded, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(reloaded, _) = design.update(reloaded, design.SelectDesign(id))
  let #(reloaded, _) =
    design.update(reloaded, design.DesignLoaded(id, a_button_design()))
  assert waiting_json(reloaded, id) == waiting
  restore_edits()
}

fn a_selected_model() -> design.Model {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_button_design()))
  let #(model, _) = design.update(model, design.ElementClicked("go", False))
  model
}

pub fn done_editing_asks_with_waiting_changes_test() {
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) = design.update(model, design.EditDone)
  assert model.edit_mode
  let html = element.to_string(design.view(model))
  assert string.contains(html, "role=\"alertdialog\"")
  assert string.contains(html, "Save")
  assert string.contains(html, "Discard")
  assert string.contains(html, "Keep editing")
}

pub fn done_editing_without_changes_turns_it_off_test() {
  let model = an_editing_model()
  let #(model, _) = design.update(model, design.EditDone)
  assert !model.edit_mode
  let html = element.to_string(design.view(model))
  assert !string.contains(html, "Keep editing")
  assert !string.contains(html, "Done editing")
}

pub fn saving_from_the_exit_prompt_saves_one_version_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) =
    design.update(model, design.EditProperty("style", "fill", "#123456"))
  let sent = [
    editing.text_change("go", "properties", "label", "Go now"),
    editing.text_change("go", "style", "fill", "#123456"),
  ]
  let #(model, _) = design.update(model, design.EditDone)
  let #(model, _) = design.update(model, design.EditSave)
  assert !model.confirm_exit
  assert model.edit_mode
  let #(model, _) =
    design.update(model, design.EditsSaved(id, a_button_design(), sent))
  assert !model.edit_mode
  assert waiting_json(model, id) == "{\"operations\":[]}"
}

pub fn a_refused_exit_prompt_save_keeps_editing_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let waiting = waiting_json(model, id)
  let sent = [editing.text_change("go", "properties", "label", "Go now")]
  let #(model, _) = design.update(model, design.EditDone)
  let #(model, _) = design.update(model, design.EditSave)
  let #(model, _) =
    design.update(
      model,
      design.EditsSaved(
        id,
        "{\"code\":\"E7001\",\"message\":\"E7001 that label is too long\"}",
        sent,
      ),
    )
  assert model.edit_mode
  assert !model.confirm_exit
  assert waiting_json(model, id) == waiting
  assert string.contains(
    element.to_string(design.view(model)),
    "that label is too long",
  )
}

pub fn discarding_from_the_exit_prompt_turns_edit_mode_off_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) = design.update(model, design.EditDone)
  let #(model, _) = design.update(model, design.EditDiscard)
  assert !model.edit_mode
  assert !model.confirm_exit
  assert waiting_json(model, id) == "{\"operations\":[]}"
  assert !string.contains(element.to_string(design.view(model)), "Keep editing")
}

pub fn keeping_editing_closes_the_prompt_test() {
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) = design.update(model, design.EditDone)
  let #(model, _) = design.update(model, design.EditCancel)
  assert model.edit_mode
  assert !model.confirm_exit
  assert !string.contains(element.to_string(design.view(model)), "Keep editing")
}

pub fn a_property_change_outside_edit_mode_adds_nothing_test() {
  let id = "0123456789abcdef"
  let model = a_selected_model()
  let #(model, _) = design.update(model, design.EditTab)
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  assert !model.edit_mode
  assert waiting_json(model, id) == "{\"operations\":[]}"
  let html = element.to_string(design.view(model))
  assert string.contains(html, "id=\"edit-properties-label\"")
  assert string.contains(html, "disabled")
}

pub fn a_design_with_waiting_changes_opens_in_edit_mode_test() {
  install_edits()
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(edited, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  assert waiting_json(edited, id) != "{\"operations\":[]}"
  let #(reloaded, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(reloaded, _) = design.update(reloaded, design.SelectDesign(id))
  let #(reloaded, _) =
    design.update(reloaded, design.DesignLoaded(id, a_button_design()))
  assert reloaded.edit_mode
  restore_edits()
}

pub fn a_live_reload_keeps_edit_mode_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_button_design()))
  assert model.edit_mode
}

pub fn the_panel_save_keeps_edit_mode_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let sent = [editing.text_change("go", "properties", "label", "Go now")]
  let #(model, _) = design.update(model, design.SaveEdits)
  let #(model, _) =
    design.update(model, design.EditsSaved(id, a_button_design(), sent))
  assert model.edit_mode
  assert !model.confirm_exit
  assert waiting_json(model, id) == "{\"operations\":[]}"
}

pub fn the_exit_save_after_discarding_turns_edit_mode_off_test() {
  let id = "0123456789abcdef"
  let model = an_editing_model()
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Go now"))
  let #(model, _) = design.update(model, design.EditDone)
  assert model.confirm_exit
  let #(model, _) = design.update(model, design.DiscardEdits)
  let #(model, _) = design.update(model, design.EditSave)
  assert !model.leaving
  assert !model.edit_mode
  assert !model.confirm_exit
  assert waiting_json(model, id) == "{\"operations\":[]}"
}

fn a_moving_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"one\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[{\"id\":\"bar\",\"type\":\"row\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[{\"id\":\"go\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Go\"}},{\"id\":\"stop\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Stop\"}}]},{\"id\":\"body\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Hi\"}}]},{\"id\":\"two\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\"},\"children\":[]}]}"
}

fn a_moving_model(edit: Bool) -> design.Model {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, a_moving_design()))
  case edit {
    True -> design.update(model, design.EditStart).0
    False -> model
  }
}

/// Drags `id` to `x`, `y` over the moving design's drawn bounds and drops it.
fn drag(model: design.Model, id: String, x: Float, y: Float) -> design.Model {
  let bounds = [
    #("one", 0.0, 0.0, 400.0, 300.0),
    #("bar", 10.0, 50.0, 390.0, 100.0),
    #("go", 20.0, 60.0, 100.0, 90.0),
    #("stop", 110.0, 60.0, 190.0, 90.0),
    #("body", 10.0, 110.0, 390.0, 140.0),
    #("two", 0.0, 320.0, 400.0, 620.0),
  ]
  let #(model, _) = design.update(model, design.DragStarted(id))
  let #(model, _) = design.update(model, design.Dragged(x, y, bounds))
  let #(model, _) = design.update(model, design.Dropped)
  model
}

pub fn dragging_an_element_earlier_in_its_row_waits_as_one_move_test() {
  let model = drag(a_moving_model(True), "stop", 30.0, 75.0)
  assert waiting_json(model, "0123456789abcdef")
    == "{\"operations\":[{\"op\":\"move\",\"id\":\"stop\",\"parent\":\"bar\",\"index\":0}]}"
  let html = element.to_string(design.view(model))
  assert before(html, "data-design-id=\"stop\"", "data-design-id=\"go\"")
}

pub fn dragging_an_element_into_another_screen_waits_as_one_move_test() {
  let model = drag(a_moving_model(True), "go", 200.0, 400.0)
  assert waiting_json(model, "0123456789abcdef")
    == "{\"operations\":[{\"op\":\"move\",\"id\":\"go\",\"parent\":\"two\",\"index\":0}]}"
  let html = element.to_string(design.view(model))
  assert before(html, "data-design-id=\"two\"", "data-design-id=\"go\"")
}

pub fn a_screen_dragged_over_a_row_moves_among_the_screens_test() {
  let model = drag(a_moving_model(True), "two", 30.0, 75.0)
  assert waiting_json(model, "0123456789abcdef")
    == "{\"operations\":[{\"op\":\"move\",\"id\":\"two\",\"parent\":null,\"index\":0}]}"
}

pub fn a_refused_drop_adds_nothing_and_restores_the_design_test() {
  let model = a_moving_model(True)
  let saved = element.to_string(design.view(model))
  let #(dragging, _) = design.update(model, design.DragStarted("body"))
  let #(dragging, _) =
    design.update(
      dragging,
      design.Dragged(500.0, 700.0, [#("one", 0.0, 0.0, 400.0, 300.0)]),
    )
  assert element.to_string(design.view(dragging)) == saved
  let #(dropped, _) = design.update(dragging, design.Dropped)
  assert waiting_json(dropped, "0123456789abcdef") == "{\"operations\":[]}"
  assert element.to_string(design.view(dropped)) == saved
}

pub fn a_drop_where_the_element_already_is_adds_nothing_test() {
  let model = drag(a_moving_model(True), "go", 30.0, 75.0)
  assert waiting_json(model, "0123456789abcdef") == "{\"operations\":[]}"
}

pub fn a_drag_outside_edit_mode_moves_nothing_test() {
  let model = drag(a_moving_model(False), "stop", 30.0, 75.0)
  assert waiting_json(model, "0123456789abcdef") == "{\"operations\":[]}"
  let html = element.to_string(design.view(model))
  assert before(html, "data-design-id=\"go\"", "data-design-id=\"stop\"")
}

pub fn edit_mode_drags_move_elements_and_offers_no_notes_test() {
  let #(viewing, _) =
    design.update(a_moving_model(False), design.ElementClicked("go", False))
  let html = element.to_string(design.view(viewing))
  assert string.contains(html, "data-region")
  assert !string.contains(html, "data-move")
  assert string.contains(html, "data-note-box")
  let #(editing, _) = design.update(viewing, design.EditStart)
  let html = element.to_string(design.view(editing))
  assert string.contains(html, "data-move")
  assert !string.contains(html, "data-region")
  assert !string.contains(html, "data-note-box")
}

pub fn a_waiting_move_survives_a_reload_and_is_saved_with_other_changes_test() {
  install_edits()
  let id = "0123456789abcdef"
  let model = drag(a_moving_model(True), "stop", 30.0, 75.0)
  let #(model, _) = design.update(model, design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Start"))
  let waiting = waiting_json(model, id)
  assert waiting
    == "{\"operations\":[{\"op\":\"move\",\"id\":\"stop\",\"parent\":\"bar\",\"index\":0},{\"op\":\"set\",\"id\":\"go\",\"field\":\"properties\",\"property\":\"label\",\"value\":\"Start\"}]}"
  let reloaded = a_moving_model(False)
  assert waiting_json(reloaded, id) == waiting
  let html = element.to_string(design.view(reloaded))
  assert before(html, "data-design-id=\"stop\"", "data-design-id=\"go\"")
  restore_edits()
}

/// Drags a side or corner of `id` to a drawn size and lets go.
fn resize_drag(
  model: design.Model,
  id: String,
  side: String,
  width: Float,
  height: Float,
) -> design.Model {
  let #(model, _) =
    design.update(model, design.Resized(id, side, width, height))
  let #(model, _) = design.update(model, design.ResizeDropped)
  model
}

pub fn resizing_a_side_sets_one_fixed_size_and_a_corner_sets_both_test() {
  let id = "0123456789abcdef"
  let #(dragging, _) =
    design.update(
      an_editing_model(),
      design.Resized("go", "right", 240.4, 90.0),
    )
  assert string.contains(
    element.to_string(design.view(dragging)),
    "width:240.0px",
  )
  assert !string.contains(waiting_json(dragging, id), "\"op\"")
  let side = resize_drag(an_editing_model(), "go", "right", 240.4, 90.0)
  let body = waiting_json(side, id)
  assert string.contains(body, "\"property\":\"width\"")
  assert string.contains(body, "\"value\":{\"fixed\":240}")
  assert !string.contains(body, "\"property\":\"height\"")
  let corner =
    resize_drag(an_editing_model(), "go", "bottom-right", 240.0, 120.6)
  let body = waiting_json(corner, id)
  assert string.contains(body, "\"property\":\"width\"")
  assert string.contains(body, "\"value\":{\"fixed\":240}")
  assert string.contains(body, "\"property\":\"height\"")
  assert string.contains(body, "\"value\":{\"fixed\":121}")
}

pub fn a_resize_to_zero_or_less_adds_nothing_and_restores_the_design_test() {
  let id = "0123456789abcdef"
  let before = element.to_string(design.view(an_editing_model()))
  let zero = resize_drag(an_editing_model(), "go", "right", 0.0, 90.0)
  assert !string.contains(waiting_json(zero, id), "\"op\"")
  assert element.to_string(design.view(zero)) == before
  let negative = resize_drag(an_editing_model(), "go", "bottom", 80.0, -4.0)
  assert !string.contains(waiting_json(negative, id), "\"op\"")
  assert element.to_string(design.view(negative)) == before
}

pub fn a_resize_outside_edit_mode_or_with_several_selected_changes_nothing_test() {
  let id = "0123456789abcdef"
  let outside = resize_drag(a_selected_model(), "go", "right", 240.0, 90.0)
  assert !string.contains(waiting_json(outside, id), "\"op\"")
  let #(multi, _) =
    design.update(an_editing_model(), design.ElementClicked("screen", True))
  let multi = resize_drag(multi, "go", "right", 240.0, 90.0)
  assert !string.contains(waiting_json(multi, id), "\"op\"")
}

pub fn resize_handles_show_only_for_one_selected_element_in_edit_mode_test() {
  assert !string.contains(
    element.to_string(design.view(a_selected_model())),
    "data-resize",
  )
  assert string.contains(
    element.to_string(design.view(an_editing_model())),
    "data-resize",
  )
  let #(multi, _) =
    design.update(an_editing_model(), design.ElementClicked("screen", True))
  assert !string.contains(element.to_string(design.view(multi)), "data-resize")
}

fn an_architecture_editing_model(id: String) -> design.Model {
  let #(model, _) = design.update(an_architecture_model(), design.EditStart)
  let #(model, _) = design.update(model, design.ElementClicked(id, False))
  let #(model, _) = design.update(model, design.EditTab)
  model
}

pub fn changing_a_node_label_in_an_architecture_diagram_waits_and_redraws_test() {
  let id = "fedcba9876543210"
  let model = an_architecture_editing_model("start")
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Begin"))
  let #(model, _) =
    design.update(
      model,
      design.EditProperty("properties", "description", "The beginning"),
    )
  let body = waiting_json(model, id)
  assert string.contains(body, "\"op\":\"set\"")
  assert string.contains(body, "\"id\":\"start\"")
  assert string.contains(body, "\"property\":\"label\"")
  assert string.contains(body, "\"value\":\"Begin\"")
  assert string.contains(body, "\"property\":\"description\"")
  assert string.contains(body, "\"value\":\"The beginning\"")
  assert list.length(string.split(body, "\"op\":\"set\"")) == 3
  let html = element.to_string(design.view(model))
  assert drawn_text(html, "design-node-label", "Begin")
  assert drawn_text(html, "design-node-description", "The beginning")
}

pub fn changing_an_edge_label_in_an_architecture_diagram_waits_and_redraws_test() {
  let id = "fedcba9876543210"
  let model = an_architecture_editing_model("step")
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Then"))
  let body = waiting_json(model, id)
  assert string.contains(body, "\"op\":\"set\"")
  assert string.contains(body, "\"id\":\"step\"")
  assert string.contains(body, "\"property\":\"label\"")
  assert string.contains(body, "\"value\":\"Then\"")
  assert list.length(string.split(body, "\"op\":\"set\"")) == 2
  let html = element.to_string(design.view(model))
  assert drawn_text(html, "design-edge-label", "Then")
}

pub fn an_architecture_edit_lists_only_node_and_edge_fields_test() {
  let node_html =
    element.to_string(design.view(an_architecture_editing_model("start")))
  assert string.contains(node_html, "edit-properties-label")
  assert string.contains(node_html, "edit-properties-description")
  assert !string.contains(node_html, "edit-properties-width")
  assert !string.contains(node_html, "edit-properties-height")
  assert !string.contains(node_html, "edit-style-")
  let edge_html =
    element.to_string(design.view(an_architecture_editing_model("step")))
  assert string.contains(edge_html, "edit-properties-label")
  assert !string.contains(edge_html, "edit-properties-description")
  assert !string.contains(edge_html, "edit-properties-width")
  assert !string.contains(edge_html, "edit-properties-height")
  assert !string.contains(edge_html, "edit-style-")
}

pub fn a_refused_architecture_save_keeps_the_diagram_and_waiting_changes_test() {
  let id = "fedcba9876543210"
  let model = an_architecture_editing_model("start")
  let saved = model.drawing
  let #(model, _) =
    design.update(model, design.EditProperty("properties", "label", "Begin"))
  let waiting = waiting_json(model, id)
  let #(model, _) = design.update(model, design.SaveEdits)
  let #(model, _) =
    design.update(
      model,
      design.EditsSaved(
        id,
        "{\"code\":\"E7001\",\"message\":\"E7001 that label is too long\"}",
        [editing.text_change("start", "properties", "label", "Begin")],
      ),
    )
  assert model.drawing == saved
  assert waiting_json(model, id) == waiting
  assert string.contains(
    element.to_string(design.view(model)),
    "that label is too long",
  )
}

fn drawn_text(html: String, class: String, text: String) -> Bool {
  case string.split_once(html, "class=\"" <> class <> "\"") {
    Ok(#(_, after)) ->
      case string.split_once(after, ">") {
        Ok(#(_, content)) -> string.starts_with(content, text)
        Error(_) -> False
      }
    Error(_) -> False
  }
}

pub fn a_live_list_is_not_replaced_by_an_older_list_test() {
  let #(model, _) = design.update(design.new(design.UiUx), design.CaughtUp)
  let #(model, _) =
    design.update(
      model,
      design.Live(
        a_design_list_of([
          #("0123456789abcdef", "Home"),
          #("1122334455667788", "New"),
        ]),
      ),
    )
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  assert design.designs_for(model, design.UiUx)
    == [
      design.Design("0123456789abcdef", "Home", design.UiUx, 0),
      design.Design("1122334455667788", "New", design.UiUx, 0),
    ]
}

pub fn an_older_list_after_a_rename_keeps_a_live_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.DesignsLoaded(a_design_list()),
    )
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.CaughtUp)
  let #(model, _) =
    design.update(
      model,
      design.Live(
        a_design_list_of([
          #(id, "Home revised"),
          #("1122334455667788", "New"),
        ]),
      ),
    )
  let #(model, _) =
    design.update(model, design.Renamed(id, a_ui_design("screen")))
  let #(model, _) = design.update(model, design.DesignsLoaded(a_design_list()))
  assert design.designs_for(model, design.UiUx)
    == [
      design.Design(id, "Home revised", design.UiUx, 0),
      design.Design("1122334455667788", "New", design.UiUx, 0),
    ]
}

pub fn an_older_delete_reply_keeps_a_live_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) = design.update(design.new(design.UiUx), design.CaughtUp)
  let #(model, _) =
    design.update(
      model,
      design.Live(
        a_design_list_of([
          #(id, "Home"),
          #("1122334455667788", "New"),
        ]),
      ),
    )
  let #(model, _) = design.update(model, design.Deleted(id, a_design_list()))
  assert design.designs_for(model, design.UiUx)
    == [
      design.Design(id, "Home", design.UiUx, 0),
      design.Design("1122334455667788", "New", design.UiUx, 0),
    ]
}

pub fn an_older_undo_reply_keeps_a_live_design_test() {
  let #(model, _) = design.update(design.new(design.UiUx), design.CaughtUp)
  let #(model, _) =
    design.update(
      model,
      design.Live(
        a_design_list_of([
          #("0123456789abcdef", "Home"),
          #("1122334455667788", "New"),
        ]),
      ),
    )
  let #(model, _) = design.update(model, design.Undone(a_design_list()))
  assert design.designs_for(model, design.UiUx)
    == [
      design.Design("0123456789abcdef", "Home", design.UiUx, 0),
      design.Design("1122334455667788", "New", design.UiUx, 0),
    ]
}
