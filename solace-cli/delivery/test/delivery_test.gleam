import delivery
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/element

pub fn main() -> Nil {
  gleeunit.main()
}

fn step(
  project: String,
  name: String,
  state: String,
  duration: Int,
) -> delivery.Step {
  delivery.Step(
    project,
    name,
    ["gleam", "build"],
    60,
    state,
    None,
    duration,
    None,
    None,
  )
}

fn make_build(steps: List(delivery.Step)) -> delivery.Build {
  delivery.Build("b1", "running", 100, None, steps)
}

fn model_with(build: delivery.Build) -> delivery.Model {
  delivery.Model(..delivery.initial(), build: Some(build))
}

fn html(model: delivery.Model) -> String {
  element.to_string(delivery.view(model))
}

pub fn every_step_state_shows_test() {
  let states = ["waiting", "running", "passed", "failed", "skipped"]
  list.each(states, fn(state) {
    let model = model_with(make_build([step("web", "compile", state, 1200)]))
    assert string.contains(html(model), "class=\"step " <> state <> "\"")
    assert string.contains(html(model), delivery.state_label(state))
  })
}

pub fn the_cancel_button_shows_only_while_running_test() {
  let running = model_with(make_build([step("web", "compile", "running", 0)]))
  assert string.contains(html(running), ">Cancel</button>")
  let passed =
    model_with(
      delivery.Build("b1", "passed", 100, Some(1300), [
        step("web", "compile", "passed", 1200),
      ]),
    )
  assert !string.contains(html(passed), ">Cancel</button>")
}

pub fn a_failed_step_shows_its_summary_test() {
  let summary =
    delivery.Summary("compile", "gleam build", Some(1), [
      "error: unknown variable",
      "  at app.gleam",
    ])
  let failed =
    delivery.Step(
      "web",
      "compile",
      ["gleam", "build"],
      60,
      "failed",
      Some(1),
      900,
      Some(summary),
      None,
    )
  let page = html(model_with(make_build([failed])))
  assert string.contains(page, "class=\"failure\"")
  assert string.contains(page, "Build failed")
  assert string.contains(page, "compile · gleam build")
  assert string.contains(page, "Exit code 1")
  assert string.contains(page, "error: unknown variable")
}

pub fn the_plan_shows_waiting_steps_test() {
  let plan =
    delivery.Recipe("web", [delivery.PlanStep("build", ["gleam", "build"], 60)])
  let model = delivery.Model(..delivery.initial(), plan: [plan])
  let page = html(model)
  assert string.contains(page, "class=\"step waiting\"")
  assert string.contains(page, "Waiting")
  assert string.contains(page, "No builds yet.")
}

pub fn a_live_log_appends_to_the_open_step_test() {
  let build =
    delivery.Build("b1", "running", 100, None, [
      step("web", "compile", "running", 0),
    ])
  let model =
    delivery.Model(
      ..delivery.initial(),
      caught_up: True,
      build: Some(build),
      log: Some(delivery.Log("b1", 0, "first\n")),
    )
  let #(next, _) =
    delivery.update(
      model,
      delivery.Live(
        "{\"buildLog\":{\"id\":\"b1\",\"step\":0,\"text\":\"second\\n\"}}",
      ),
    )
  assert string.contains(html(next), "first")
  assert string.contains(html(next), "second")
}

pub fn a_duration_reads_in_seconds_or_milliseconds_test() {
  assert delivery.duration_text(0) == "0 ms"
  assert delivery.duration_text(999) == "999 ms"
  assert delivery.duration_text(1200) == "1.2 s"
}

pub fn the_build_tab_is_the_only_tab_test() {
  assert delivery.tabs() == [delivery.BuildTab]
  assert delivery.tab_name(delivery.BuildTab) == "build"
  assert delivery.label(delivery.BuildTab) == "Build"
  assert delivery.tab_from("") == delivery.BuildTab
  assert delivery.tab_from("tab=build") == delivery.BuildTab
  assert delivery.tab_from("tab=nope") == delivery.BuildTab
}

pub fn a_failed_step_files_or_opens_its_task_test() {
  let summary =
    delivery.Summary("compile", "gleam build", Some(1), [
      "error: unknown variable",
    ])
  let failed =
    delivery.Step(
      "web",
      "compile",
      ["gleam", "build"],
      60,
      "failed",
      Some(1),
      900,
      Some(summary),
      None,
    )
  let page = html(model_with(make_build([failed])))
  assert string.contains(page, ">File a task</button>")

  let filed =
    delivery.Step(
      "web",
      "compile",
      ["gleam", "build"],
      60,
      "failed",
      Some(1),
      900,
      Some(summary),
      Some("t1"),
    )
  let page = html(model_with(make_build([filed])))
  assert string.contains(page, "href=\"/task/t1\"")
  assert string.contains(page, ">Open task</a>")
  assert !string.contains(page, ">File a task</button>")
}

pub fn filing_a_task_opens_the_prefilled_form_test() {
  let summary =
    delivery.Summary("compile", "gleam build", Some(1), [
      "error: unknown variable",
    ])
  let failed =
    delivery.Step(
      "web",
      "compile",
      ["gleam", "build"],
      60,
      "failed",
      Some(1),
      900,
      Some(summary),
      None,
    )
  let #(model, _) =
    delivery.update(model_with(make_build([failed])), delivery.FileTask(0))
  let page = html(model)
  assert string.contains(page, "build: web compile fails")
  assert string.contains(page, "Project: web")
  assert string.contains(page, "/delivery?build=b1")
  assert string.contains(page, "Command: `gleam build`")
  assert string.contains(page, "error: unknown variable")
  assert string.contains(page, "[Open the build](/delivery?build=b1)")
  assert string.contains(page, ">Create task</button>")
}

pub fn open_folder_shows_only_in_the_desktop_app_test() {
  let release =
    delivery.Release(
      "b1",
      [delivery.Artifact("priv/app.js", 6, "abc123")],
      None,
    )
  let browser =
    delivery.Model(
      ..model_with(make_build([])),
      release: Some(release),
      releases: [release],
    )
  let page = html(browser)
  assert string.contains(page, "Release b1")
  assert string.contains(page, "priv/app.js")
  assert string.contains(page, "6 bytes")
  assert string.contains(page, "abc123")
  assert string.contains(page, "Download")
  assert !string.contains(page, "Open folder")
  let desktop = delivery.Model(..browser, desktop: True)
  assert string.contains(html(desktop), "Open folder")
}

pub fn a_browser_tab_starts_without_desktop_access_test() {
  let #(model, _) = delivery.init(Nil)
  assert !model.desktop
}

pub fn a_passed_build_has_a_release_to_load_test() {
  let passed =
    delivery.Build("b1", "passed", 100, Some(1300), [
      step("web", "compile", "passed", 1200),
    ])
  assert delivery.released(True, Some(passed))
  assert !delivery.released(False, Some(passed))
  assert !delivery.released(True, Some(make_build([])))
  assert !delivery.released(True, None)
}

pub fn the_version_files_and_a_disagreement_show_test() {
  let model =
    delivery.Model(
      ..delivery.initial(),
      version: "0.1.1",
      versions: [
        delivery.VersionFile("app", "Cargo.toml", "0.1.0"),
        delivery.VersionFile("app", "gleam.toml", "1.0.0"),
      ],
      versions_agree: False,
    )
  let page = html(model)
  assert string.contains(page, "0.1.0")
  assert string.contains(page, "1.0.0")
  assert string.contains(page, "disagree")
}

pub fn loading_versions_fills_the_version_box_test() {
  let #(model, _) =
    delivery.update(
      delivery.initial(),
      delivery.VersionsLoaded(
        "{\"versions\":[{\"project\":\"app\",\"file\":\"gleam.toml\",\"version\":\"0.1.0\"}],\"agree\":true,\"proposal\":\"0.1.1\",\"newest\":\"0.1.0\"}",
      ),
    )
  assert model.version == "0.1.1"
  assert list.length(model.versions) == 1
  assert model.newest == "0.1.0"
}

pub fn an_older_version_is_warned_and_still_builds_test() {
  let model =
    delivery.Model(..delivery.initial(), version: "0.1.1", newest: "0.2.0")
  let page = html(model)
  assert string.contains(page, "older than the newest release")
  assert !delivery.start_disabled(model)
  let current = delivery.Model(..model, version: "0.2.0")
  assert !string.contains(html(current), "older than the newest release")
}

pub fn an_older_release_can_be_picked_and_its_notes_draft_shown_test() {
  let newest = delivery.Release("2.0.0", [], None)
  let older =
    delivery.Release(
      "1.0.0",
      [],
      Some(delivery.Notes("- [Ship it](task:0123456789abcdef)", "draft")),
    )
  let model =
    delivery.Model(
      ..delivery.initial(),
      releases: [newest, older],
      release: Some(newest),
    )
  let #(picked, _) = delivery.update(model, delivery.SelectRelease("1.0.0"))
  let page = html(picked)
  assert string.contains(page, "Release 1.0.0")
  assert string.contains(page, "Ship it")
  assert string.contains(page, "task:0123456789abcdef")
  assert string.contains(page, "Approve")
  assert string.contains(page, "Reject")
}

pub fn a_draft_can_be_edited_and_approved_test() {
  let draft =
    delivery.Release("1.0.0", [], Some(delivery.Notes("old draft", "draft")))
  let model =
    delivery.Model(
      ..delivery.initial(),
      releases: [draft],
      release: Some(draft),
    )
  let page = html(model)
  assert string.contains(page, "old draft")
  assert string.contains(page, "notes-edit")
  assert string.contains(page, "Approve")
  assert string.contains(page, "Reject")
  let #(edited, _) = delivery.update(model, delivery.NotesChanged("new notes"))
  assert edited.notes_edit == Some("new notes")
  assert string.contains(html(edited), "new notes")
  let #(saving, _) = delivery.update(edited, delivery.ApproveNotes)
  assert saving.saving_notes
  assert string.contains(html(saving), "Saving…")
}

pub fn approved_notes_show_with_the_release_test() {
  let release =
    delivery.Release(
      "1.0.0",
      [],
      Some(delivery.Notes("- [Ship it](task:0123456789abcdef)", "approved")),
    )
  let model =
    delivery.Model(
      ..delivery.initial(),
      releases: [release],
      release: Some(release),
    )
  let page = html(model)
  assert string.contains(page, "Release notes")
  assert string.contains(page, "Ship it")
  assert string.contains(page, "/task/0123456789abcdef")
  assert !string.contains(page, "notes-edit")
  assert !string.contains(page, "Approve")
}
