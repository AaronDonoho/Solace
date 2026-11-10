import design
import gleam/string
import gleeunit
import lustre/element
import panel

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn a_proposal_draws_its_changes_test() {
  let model = proposed_model(proposed_design())
  let html = element.to_string(design.view(model))
  assert string.contains(
    html,
    "class=\"design-element design-change-added\" data-design-id=\"added\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-removed\" data-design-id=\"removed\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-changed\" data-design-id=\"changed\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-moved\" data-design-id=\"mover1\"",
  )
  assert string.contains(
    html,
    "class=\"design-element\" data-design-id=\"same\"",
  )
  assert string.contains(html, "Proposed")
  assert string.contains(html, "Current")
  assert string.contains(html, "aria-label=\"Hide changes\"")
}

pub fn the_switch_shows_the_saved_design_test() {
  let model = proposed_model(proposed_design())
  let #(model, _) = design.update(model, design.ShowCurrent)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"removed\"")
  assert string.contains(html, "Before")
  assert !string.contains(html, "After")
  assert !string.contains(html, "data-design-id=\"added\"")
  assert !string.contains(
    html,
    "class=\"design-element design-change-removed\"",
  )
}

pub fn hiding_changes_draws_the_proposal_plainly_test() {
  let model = proposed_model(proposed_design())
  let #(model, _) = design.update(model, design.ToggleChanges)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "data-design-id=\"added\"")
  assert !string.contains(html, "data-design-id=\"removed\"")
  assert !string.contains(html, "class=\"design-element design-change-added\"")
}

pub fn a_create_proposal_draws_everything_added_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.Replied(True, create_run(created_design()))),
    )
  let html = element.to_string(design.view(model))
  assert string.contains(
    html,
    "class=\"design-element design-change-added\" data-design-id=\"newroot\"",
  )
  assert string.contains(
    html,
    "class=\"design-element design-change-added\" data-design-id=\"newtext\"",
  )
  assert !string.contains(html, "Current")
}

fn proposed_model(proposed: String) -> design.Model {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(design.new(design.UiUx), design.DesignsLoaded(designs()))
  let #(model, _) = design.update(model, design.SelectDesign(id))
  let #(model, _) = design.update(model, design.DesignLoaded(id, saved()))
  let #(model, _) =
    design.update(model, design.Panel(panel.Replied(True, run(proposed))))
  model
}

fn designs() -> String {
  "{\"designs\":[{\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\"}]}"
}

fn saved() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"root\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"same\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Same\"}},{\"id\":\"changed\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Before\"}},{\"id\":\"removed\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Removed\"}},{\"id\":\"mover1\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"One\"}},{\"id\":\"mover2\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Two\"}}]}]}"
}

fn proposed_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"root\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"same\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Same\"}},{\"id\":\"changed\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"After\"}},{\"id\":\"added\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Added\"}},{\"id\":\"mover2\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Two\"}},{\"id\":\"mover1\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"One\"}}]}]}"
}

fn created_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"fedcba9876543210\",\"title\":\"New\",\"tab\":\"ui\",\"elements\":[{\"id\":\"newroot\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"newtext\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Hi\"}}]}]}"
}

fn run(proposed: String) -> String {
  "{\"id\":\"r1\",\"kind\":\"design\",\"model\":\"fake\",\"state\":\"waiting\",\"started\":1,\"context\":{\"kind\":\"page\"},\"notes\":[],\"questions\":[{\"id\":\"q1\",\"text\":\"Change it\",\"options\":[\"Approve\",\"Reject\"],\"proposal\":{\"kind\":\"design\",\"id\":\"0123456789abcdef\",\"operations\":[],\"design\":"
  <> proposed
  <> "},\"time\":1,\"after\":0}]}"
}

fn create_run(proposed: String) -> String {
  "{\"id\":\"r2\",\"kind\":\"design\",\"model\":\"fake\",\"state\":\"waiting\",\"started\":1,\"context\":{\"kind\":\"page\"},\"notes\":[],\"questions\":[{\"id\":\"q2\",\"text\":\"Create it\",\"options\":[\"Approve\",\"Reject\"],\"proposal\":{\"kind\":\"design\",\"title\":\"New\",\"tab\":\"ui\",\"operations\":[],\"design\":"
  <> proposed
  <> "},\"time\":1,\"after\":0}]}"
}

pub fn an_answered_proposal_stops_drawing_the_stale_design_test() {
  let id = "0123456789abcdef"
  let #(model, _) =
    design.update(proposed_model(proposed_design()), design.CaughtUp)
  assert string.contains(
    element.to_string(design.view(model)),
    "data-design-id=\"added\"",
  )
  let #(_, open_request) =
    design.update(model, design.Live(open_live(proposed_design())))
  let #(model, request) =
    design.update(model, design.Live(answered_live(proposed_design())))
  assert request != open_request
  let answered = element.to_string(design.view(model))
  assert !string.contains(answered, "data-design-id=\"added\"")
  assert !string.contains(answered, "Before")
  let #(model, _) =
    design.update(model, design.DesignLoaded(id, accepted_design()))
  let reloaded = element.to_string(design.view(model))
  assert string.contains(reloaded, "Accepted")
  assert !string.contains(reloaded, "Before")
}

fn accepted_design() -> String {
  "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"root\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"same\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Accepted\"}}]}]}"
}

fn answered_live(proposed: String) -> String {
  "{\"designRun\":{\"id\":\"r1\",\"kind\":\"design\",\"model\":\"fake\",\"state\":\"waiting\",\"started\":1,\"context\":{\"kind\":\"page\"},\"notes\":[],\"questions\":[{\"id\":\"q1\",\"text\":\"Change it\",\"options\":[\"Approve\",\"Reject\"],\"proposal\":{\"kind\":\"design\",\"id\":\"0123456789abcdef\",\"operations\":[],\"design\":"
  <> proposed
  <> "},\"answer\":\"Approved and saved version 2\",\"time\":1,\"after\":0}]}}"
}

fn open_live(proposed: String) -> String {
  "{\"designRun\":" <> run(proposed) <> "}"
}
