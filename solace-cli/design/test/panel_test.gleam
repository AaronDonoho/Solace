import design
import gleam/json
import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/element
import panel
import plan_panel

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn the_panel_shows_on_both_tabs_test() {
  let model = design.new(design.Architecture)
  assert string.contains(
    element.to_string(design.view(model)),
    "id=\"design-plan\"",
  )
  let #(ui, _) = design.update(model, design.Select(design.UiUx))
  let html = element.to_string(design.view(ui))
  assert string.contains(html, "id=\"design-plan\"")
  assert string.contains(html, "design-list")
}

pub fn a_run_starts_from_the_panel_test() {
  install_drafts()
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.Panel(panel.Text("Plan it")),
    )
  let #(model, _) = design.update(model, design.Panel(panel.Start))
  assert model.panel.sending
  assert model.panel.clearing == Some("Plan it")
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "running", [], [])),
      )),
    )
  assert model.panel.open == "r1"
  assert model.panel.text == ""
  restore_drafts()
}

pub fn a_reply_shows_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "waiting", [said("Here is a screen", 2)], [])),
      )),
    )
  assert string.contains(
    element.to_string(design.view(model)),
    "Here is a screen",
  )
}

pub fn the_draft_survives_a_reload_test() {
  install_drafts()
  let #(model, _) =
    design.update(
      design.new(design.Architecture),
      design.Panel(panel.Text("Unsent")),
    )
  assert model.panel.text == "Unsent"
  assert design.new(design.Architecture).panel.text == "Unsent"
  restore_drafts()
}

pub fn attention_when_a_run_replies_or_proposes_test() {
  let run = fn(state, questions) {
    plan_panel.Run(
      id: "r1",
      model: "m",
      effort: "",
      state: state,
      started: 1,
      ended: 0,
      outcome: "",
      context: plan_panel.page,
      notes: [],
      questions: questions,
      used: "",
    )
  }
  let ask = fn(id, answer) {
    plan_panel.Question(id, "Add a screen?", ["Yes", "No"], answer, None, 0, -1)
  }
  assert panel.needs_attention([run("running", [])], [run("waiting", [])])
  assert !panel.needs_attention([run("waiting", [])], [run("waiting", [])])
  assert !panel.needs_attention([], [run("running", [])])
  assert panel.needs_attention([run("running", [])], [
    run("running", [ask("q1", "")]),
  ])
  assert !panel.needs_attention([run("running", [ask("q1", "")])], [
    run("running", [ask("q1", "Yes")]),
  ])
}

pub fn the_panel_keeps_its_width_and_collapses_test() {
  let model = design.new(design.Architecture)
  let html = element.to_string(design.view(model))
  assert string.contains(html, "role=\"separator\"")
  assert string.contains(html, "aria-valuenow=\"380\"")
  assert string.contains(html, "--plan-width:380px")
  let #(wider, _) = design.update(model, design.Panel(panel.Key("ArrowLeft")))
  assert wider.panel.width == 396
  let #(narrower, _) =
    design.update(model, design.Panel(panel.Key("ArrowRight")))
  assert narrower.panel.width == 364
  let #(clamped, _) = design.update(model, design.Panel(panel.Width(100)))
  assert clamped.panel.width == 280
  let #(collapsed, _) = design.update(model, design.Panel(panel.Collapse))
  assert collapsed.panel.collapsed
  assert !string.contains(
    element.to_string(design.view(collapsed)),
    "role=\"separator\"",
  )
}

fn run(
  id: String,
  state: String,
  notes: List(json.Json),
  questions: List(json.Json),
) -> json.Json {
  json.object([
    #("id", json.string(id)),
    #("kind", json.string("design")),
    #("model", json.string("fake")),
    #("state", json.string(state)),
    #("started", json.int(1)),
    #("context", json.object([#("kind", json.string("page"))])),
    #("notes", json.array(notes, fn(note) { note })),
    #("questions", json.array(questions, fn(question) { question })),
  ])
}

fn said(text: String, time: Int) -> json.Json {
  json.object([
    #("author", json.string("agent")),
    #("text", json.string(text)),
    #("time", json.int(time)),
  ])
}

@external(javascript, "./draft_storage.mjs", "install")
fn install_drafts() -> Nil

@external(javascript, "./draft_storage.mjs", "restore")
fn restore_drafts() -> Nil

/// The designer's model is picked from the role defaults once both replies
/// are in, whichever arrives first, and the pick shows it.
pub fn the_designers_model_is_chosen_test() {
  let agents =
    json.to_string(
      json.array([agent("designer/model", "Designer model")], fn(a) { a }),
    )
  let roles =
    json.to_string(
      json.array([role("designer", "designer/model")], fn(r) { r }),
    )
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.ModelsRefreshed(agents)),
    )
  let #(model, _) =
    design.update(model, design.Panel(panel.RoleModelsLoaded(roles)))
  assert model.panel.model == "designer/model"
  assert string.contains(
    element.to_string(design.view(model)),
    "Designer model",
  )
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.RoleModelsLoaded(roles)),
    )
  let #(model, _) =
    design.update(model, design.Panel(panel.ModelsRefreshed(agents)))
  assert model.panel.model == "designer/model"
}

/// A live design run refreshes the models; other live changes do not.
pub fn live_design_runs_refresh_the_models_test() {
  assert panel.models_event("{\"designRun\":{\"id\":\"r\"}}")
  assert panel.models_event("{\"designRuns\":[]}")
  assert !panel.models_event("{\"feature\":{\"id\":\"f\"}}")
  assert !panel.models_event("{\"workflow\":{}}")
}

fn agent(id: String, label: String) -> json.Json {
  json.object([#("id", json.string(id)), #("label", json.string(label))])
}

fn role(role: String, default: String) -> json.Json {
  json.object([
    #("role", json.string(role)),
    #("default", json.string(default)),
  ])
}

pub fn a_note_holds_the_selected_ids_and_text_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.NoteText("Make it blue")),
    )
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("d1", ["button", "row"])))
  assert model.panel.notes
    == [panel.Note("d1", ["button", "row"], "Make it blue")]
  assert model.panel.note_text == ""
}

pub fn a_blank_note_is_not_added_test() {
  let #(model, _) =
    design.update(design.new(design.UiUx), design.Panel(panel.NoteText("   ")))
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("d1", ["button"])))
  assert model.panel.notes == []
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("Text")))
  let #(model, _) = design.update(model, design.Panel(panel.AddNote("d1", [])))
  assert model.panel.notes == []
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("", ["button"])))
  assert model.panel.notes == []
}

pub fn a_message_body_carries_notes_and_their_elements_test() {
  let body =
    json.to_string(
      panel.send_body("Please fix", [
        panel.Note("d1", ["go", "one"], "Rename it"),
      ]),
    )
  assert string.contains(
    body,
    "\"notes\":[{\"design\":\"d1\",\"elements\":[\"go\",\"one\"],\"text\":\"Rename it\"}]",
  )
  assert !string.contains(
    json.to_string(panel.send_body("Please fix", [])),
    "notes",
  )
}

pub fn a_selected_buttons_note_is_posted_and_a_reply_naming_it_shows_test() {
  let #(model, _) =
    design.update(design.new(design.UiUx), design.SelectDesign("d1"))
  let #(model, _) = design.update(model, design.ElementClicked("go", False))
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Rename the Next button")))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.AddNote(model.open, model.selected)),
    )
  let note = panel.Note("d1", ["go"], "Rename the Next button")
  assert model.panel.notes == [note]
  let body = json.to_string(panel.send_body("Please update it", [note]))
  assert string.contains(body, "\"elements\":[\"go\"]")
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "running", [], [])),
      )),
    )
  let #(model, _) =
    design.update(model, design.Panel(panel.Text("Please update it")))
  let #(model, _) = design.update(model, design.Panel(panel.Send))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.Replied(
        True,
        json.to_string(
          run(
            "r1",
            "running",
            [
              said("The Next button now says Continue", 2),
            ],
            [],
          ),
        ),
      )),
    )
  assert model.panel.notes == []
  assert string.contains(
    element.to_string(design.view(model)),
    "The Next button now says Continue",
  )
}

pub fn a_refused_reply_keeps_the_notes_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "running", [], [])),
      )),
    )
  let #(model, _) =
    design.update(model, design.Panel(panel.NoteText("Keep me")))
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("d1", ["button"])))
  let #(model, _) = design.update(model, design.Panel(panel.Text("Please")))
  let #(model, _) = design.update(model, design.Panel(panel.Send))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.Replied(
        True,
        "{\"message\":\"unknown element button\"}",
      )),
    )
  assert model.panel.notes == [panel.Note("d1", ["button"], "Keep me")]
  assert !model.panel.sending
}

pub fn notes_added_while_sending_wait_for_the_next_message_test() {
  let #(model, _) =
    design.update(
      design.new(design.UiUx),
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "running", [], [])),
      )),
    )
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("First")))
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("d1", ["button"])))
  let #(model, _) = design.update(model, design.Panel(panel.Text("Please")))
  let #(model, _) = design.update(model, design.Panel(panel.Send))
  let #(model, _) = design.update(model, design.Panel(panel.NoteText("Second")))
  let #(model, _) =
    design.update(model, design.Panel(panel.AddNote("d1", ["row"])))
  let #(model, _) =
    design.update(
      model,
      design.Panel(panel.Replied(
        True,
        json.to_string(run("r1", "running", [], [])),
      )),
    )
  assert model.panel.notes == [panel.Note("d1", ["row"], "Second")]
}
