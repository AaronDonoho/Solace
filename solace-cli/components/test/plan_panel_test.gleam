import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import lustre/element
import plan_panel
import solace_task

pub fn main() -> Nil {
  Nil
}

fn settings() -> plan_panel.Settings {
  plan_panel.Settings(
    id: "design-plan",
    address: "/design/runs",
    text_id: "design-text",
    draft_owner: "design:",
    labels: plan_panel.Labels(
      title: "Design",
      context: "Design context",
      whole: "This design",
      open_here: "A design run on this is open.",
      marker: "Design",
      marker_title: "A design run is open on this",
      run_open: "A design run on this is open",
      resize: "Resize the design panel",
      earlier: "Earlier runs",
      new_message: "New message ↓",
    ),
  )
}

fn panel() -> plan_panel.Panel(Nil) {
  plan_panel.Panel(
    runs: [],
    context: plan_panel.Context("design", "d1"),
    shown: None,
    entries: [],
    collapsed: False,
    width: 380,
    max_width: 600,
    text: "",
    sending: False,
    error: "",
    refreshing: False,
    models: [],
    model: "",
    effort: "",
    name: fn(_, _) { "" },
    on_collapse: Nil,
    on_drag: fn(_) { Nil },
    on_key: fn(_) { Nil },
    on_text: fn(_) { Nil },
    on_format: fn(_, _) { Nil },
    on_block: fn(_) { Nil },
    on_model: fn(_) { Nil },
    on_effort: fn(_) { Nil },
    on_try_again: Nil,
    on_start: Nil,
    on_send: Nil,
    on_answer: fn(_, _) { Nil },
    on_show: fn(_) { Nil },
    on_end: Nil,
    at_bottom: True,
    new_message: False,
    on_scroll: fn(_) { Nil },
    on_newest: Nil,
  )
}

pub fn a_page_uses_the_panel_with_its_own_context_draft_and_address_test() {
  let settings = settings()
  assert plan_panel.draft_key(settings) == "solace-draft:design-text:design:"
  assert plan_panel.runs_path(settings) == "/design/runs"
  assert plan_panel.run_path(settings, "r1") == "/design/runs/r1"
  let page = element.to_string(plan_panel.view("", settings, panel()))
  assert string.contains(page, ">Design</h2>")
  assert string.contains(page, "id=\"design-plan\"")
  assert string.contains(page, "Design context: design <em>d1</em>")
  assert string.contains(page, "id=\"design-text\"")
  assert !string.contains(page, "Plan context")
  assert !string.contains(page, "id=\"plan-text\"")
}

pub fn a_run_start_names_the_panels_context_test() {
  assert json.to_string(plan_panel.start_body(
      "fake",
      "",
      "Go",
      plan_panel.Context("design", "d1"),
    ))
    == "{\"model\":\"fake\",\"text\":\"Go\",\"context\":{\"kind\":\"design\",\"id\":\"d1\"}}"
}

pub fn the_panel_bounds_its_width_test() {
  assert plan_panel.min_width == 280
  assert plan_panel.default_width == 380
  assert plan_panel.key_step == 16
  assert plan_panel.clamp(100, 1000) == 280
  assert plan_panel.clamp(900, 1000) == 600
  assert plan_panel.clamp(500, 1000) == 500
  assert plan_panel.clamp(500, 0) == 500
  assert plan_panel.max_width(300) == 280
  assert string.contains(plan_panel.styles, ".plan{")
}

pub fn a_page_reads_the_runs_on_its_context_test() {
  let design = plan_panel.Context("design", "d1")
  let note = solace_task.Note("human", "", "hello", 1)
  let question = plan_panel.Question("q1", "Go?", ["Yes", "No"], "", None, 1, 0)
  let open =
    plan_panel.Run(
      id: "r1",
      model: "m",
      effort: "",
      state: "running",
      started: 2,
      ended: 0,
      outcome: "",
      context: design,
      notes: [note],
      questions: [question],
      used: "",
    )
  let ended =
    plan_panel.Run(
      id: "r2",
      model: "m",
      effort: "",
      state: "ended",
      started: 1,
      ended: 2,
      outcome: "planned",
      context: design,
      notes: [],
      questions: [],
      used: "",
    )
  let page = plan_panel.Run(..ended, id: "r3", context: plan_panel.page)
  let runs = [open, ended, page]
  assert plan_panel.open_on(runs, design) == Some(open)
  assert plan_panel.new_run(runs, design) == "r1"
  assert plan_panel.open_ids(runs, "design") == ["d1"]
  assert list.map(plan_panel.past(runs, design), fn(r) { r.id }) == ["r1", "r2"]
  assert plan_panel.size(open) == 2
  assert string.contains(plan_panel.state_text(open, []), "Working")
  assert plan_panel.default_width == 380
  let assert Ok(decoded) =
    json.parse(
      "{\"id\":\"r4\",\"state\":\"waiting\",\"context\":{\"kind\":\"design\",\"id\":\"d1\"}}",
      plan_panel.decoder(),
    )
  assert decoded.context == design
  let assert Ok(context) =
    json.parse(
      "{\"kind\":\"design\",\"id\":\"d9\"}",
      plan_panel.context_decoder(),
    )
  assert context == plan_panel.Context("design", "d9")
}

@external(javascript, "./draft_storage.mjs", "install")
fn install_drafts() -> Nil

@external(javascript, "./draft_storage.mjs", "restore")
fn restore_drafts() -> Nil

pub fn a_page_keeps_its_own_draft_test() {
  install_drafts()
  let settings = settings()
  plan_panel.save_draft(settings, "Hello")
  assert plan_panel.load_draft(settings) == "Hello"
  plan_panel.clear_draft(settings)
  assert plan_panel.load_draft(settings) == ""
  restore_drafts()
}

pub fn a_page_names_the_marker_with_its_own_words_test() {
  let marker = element.to_string(plan_panel.marker(settings().labels))
  assert string.contains(marker, ">Design</span>")
  assert string.contains(marker, "title=\"A design run is open on this\"")
}

pub fn notes_draw_in_the_panel_and_a_start_carries_them_test() {
  let page =
    element.to_string(plan_panel.view_with_notes(
      "",
      settings(),
      panel(),
      plan_panel.marker(settings().labels),
    ))
  assert string.contains(page, "class=\"planning\"")
  let body =
    json.to_string(
      plan_panel.start_body_with_notes("fake", "", "Go", plan_panel.page, [
        json.object([#("text", json.string("A note"))]),
      ]),
    )
  assert string.contains(body, "\"notes\":[{\"text\":\"A note\"}]")
}

pub fn tabs_show_at_the_top_and_replace_the_agent_body_test() {
  let tabs = [
    plan_panel.Tab("Edit", True, Nil),
    plan_panel.Tab("Agent", False, Nil),
  ]
  let edit = plan_panel.marker(settings().labels)
  let page =
    element.to_string(plan_panel.view_with_notes_and_tabs(
      "",
      settings(),
      panel(),
      tabs,
      Some(edit),
      element.none(),
    ))
  assert string.contains(page, "class=\"plan-tabs\"")
  assert string.contains(page, ">Edit</button>")
  assert string.contains(page, "class=\"planning\"")
  assert !string.contains(page, "plan-compose")
  let agent =
    element.to_string(plan_panel.view_with_notes_and_tabs(
      "",
      settings(),
      panel(),
      [
        plan_panel.Tab("Edit", False, Nil),
        plan_panel.Tab("Agent", True, Nil),
      ],
      None,
      element.none(),
    ))
  assert string.contains(agent, ">Agent</button>")
  assert string.contains(agent, "plan-compose")
}

/// On the Strategy page's pick, the first pick removes the placeholder and
/// each model's option keeps its key, so the box shows the model picked.
pub fn a_first_pick_keeps_each_model_option_test() {
  let agent = fn(id) {
    solace_task.Agent(
      id: id,
      label: id,
      available: True,
      reason: "",
      effort: "",
      efforts: [],
      agent: "",
      retry: False,
      sign_in: False,
      model: "",
    )
  }
  let draw = fn(model) {
    element.to_string(plan_panel.view_with_notes(
      "",
      settings(),
      plan_panel.Panel(
        ..panel(),
        models: [agent("a"), agent("b")],
        model: model,
      ),
      plan_panel.marker(settings().labels),
    ))
  }
  let before = draw("")
  assert string.contains(before, "Select a model")
  assert string.contains(before, "data-lustre-key=\"a\"")
  assert string.contains(before, "data-lustre-key=\"b\"")
  let after = draw("b")
  assert !string.contains(after, "Select a model")
  assert string.contains(after, "data-lustre-key=\"a\"")
  assert string.contains(after, "data-lustre-key=\"b\" selected")
}

/// The conversation can take focus, so Ctrl+A there selects only its text.
pub fn the_conversation_takes_its_own_keys_test() {
  let page = element.to_string(plan_panel.view("", settings(), panel()))
  assert string.contains(page, "tabindex=\"-1\"")
}
