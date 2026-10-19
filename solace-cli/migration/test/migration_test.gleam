import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/element
import migration
import migration/api
import solace_task

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn failed_without_a_report_shows_the_problem_test() {
  let model =
    migration.Model(
      migration: Some(api.Migration(
        state: "failed",
        top: "/work/project",
        original: "/work/project",
        copy: "/work/solace-migration",
        step: "",
        candidates: [],
        report: None,
        flagged: [],
        problem: "The copy does not build",
        blockers: [],
        undo: False,
      )),
      migration_open: False,
      migration_projects: [],
      migration_smoke: "",
      migration_busy: False,
      migration_generation: 0,
      migration_error: "",
      migration_dismissed: False,
      migration_connected: True,
      migration_poll_id: 0,
      projects: migration.no_projects,
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(html, "The migration stopped")
  assert string.contains(html, "The copy does not build")
  assert string.contains(html, "Delete the copy")
  assert string.contains(html, "Your original folder: ")
  assert !string.contains(html, "What Solace checked")
}

pub fn adopted_without_undo_offers_no_way_back_test() {
  let model =
    migration.Model(
      migration: Some(api.Migration(
        state: "adopted",
        top: "/work/project",
        original: "/work/project",
        copy: "/work/solace-migration",
        step: "",
        candidates: [],
        report: None,
        flagged: [],
        problem: "",
        blockers: [],
        undo: False,
      )),
      migration_open: False,
      migration_projects: [],
      migration_smoke: "",
      migration_busy: False,
      migration_generation: 0,
      migration_error: "",
      migration_dismissed: False,
      migration_connected: True,
      migration_poll_id: 0,
      projects: migration.no_projects,
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(
    html,
    "<h1>Workspace</h1><section class=\"bring-in\"><h2>Bring in an existing project</h2>",
  )
  assert !string.contains(html, "Switch back")
  assert !string.contains(html, ".pre-solace")
}

pub fn a_check_time_that_is_not_seconds_is_shown_as_it_is_test() {
  let model =
    migration.Model(
      migration: Some(api.Migration(
        state: "in_progress",
        top: "/work/solace-migration",
        original: "/work/project",
        copy: "/work/solace-migration",
        step: "",
        candidates: [],
        report: Some(
          api.MigrationReport(
            checked_at: "abc",
            current: True,
            checks: [],
            warnings: [],
          ),
        ),
        flagged: [],
        problem: "",
        blockers: [],
        undo: False,
      )),
      migration_open: False,
      migration_projects: [],
      migration_smoke: "",
      migration_busy: False,
      migration_generation: 0,
      migration_error: "",
      migration_dismissed: False,
      migration_connected: True,
      migration_poll_id: 0,
      projects: migration.no_projects,
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(html, "Last checked abc")
}

pub fn registering_outside_a_browser_is_an_error_test() {
  let assert Error(_) = migration.register()
}

/// The Projects section shows once the list loads, above the migration
/// section: Remove for every project but the root's own, Add for one found
/// already set up, and code not set up points to the section below.
pub fn projects_section_test() {
  let #(model, _) = migration.init(Nil)
  let html = element.to_string(migration.view(model))
  assert !string.contains(html, "class=\"projects\"")
  // Nothing to bring in until the migration loads.
  assert !string.contains(html, "Bring in an existing project")
  let #(model, _) =
    migration.update(
      model,
      migration.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"app\",\"base\":\"\"},{\"dir\":\"a\",\"label\":\"a\",\"name\":\"pa\",\"base\":\"/p/a\"}],\"found\":[{\"dir\":\"tools\",\"name\":\"tools\",\"imported\":true,\"member\":false},{\"dir\":\"old\",\"name\":\"old\",\"imported\":false,\"member\":false},{\"dir\":\"rs\",\"name\":\"solace\",\"imported\":false,\"member\":false,\"language\":\"rust\"}]}",
        True,
      ),
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(
    html,
    "<h1>Workspace</h1><section class=\"projects\"><h2>"
      <> solace_task.ui_label("projects")
      <> "</h2>",
  )
  assert string.contains(
    html,
    "<strong>root</strong> app <small>(this folder)</small>",
  )
  assert string.contains(
    html,
    "<strong>a</strong> pa <button>"
      <> solace_task.ui_label("remove")
      <> "</button>",
  )
  assert string.contains(
    html,
    ">" <> solace_task.ui_label("add") <> "</button>",
  )
  assert string.contains(
    html,
    "<strong>rs</strong> solace (not set up) <button>"
      <> solace_task.ui_label("add")
      <> "</button>",
  )
  assert string.contains(
    html,
    "brought in under Bring in an existing project, below.",
  )
  assert !string.contains(html, "href=")
  assert string.contains(
    html,
    ">" <> solace_task.ui_label("create_project") <> "</button>",
  )
  // A reply that isn't a list shows its message.
  let #(model, _) =
    migration.update(
      model,
      migration.ProjectsLoaded(
        "{\"code\":\"E7003\",\"message\":\"busy\"}",
        False,
      ),
    )
  assert string.contains(
    element.to_string(migration.view(model)),
    "<p role=\"alert\">busy</p>",
  )
}

pub fn removing_a_project_test() {
  let #(model, _) = migration.init(Nil)
  let #(model, _) =
    migration.update(
      model,
      migration.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}",
        True,
      ),
    )
  let shows = fn(model, text) {
    migration.view(model) |> element.to_string |> string.contains(text)
  }
  assert !shows(model, "Remove and delete files")
  // Step one asks whether to keep the files.
  let #(model, _) = migration.update(model, migration.RemoveProject("a"))
  assert model.projects.removing == Some(#("a", False, ""))
  assert shows(model, solace_task.ui_label("remove_only"))
  assert shows(model, "Remove and delete files")
  // The dialog is the shared modal from components.
  assert shows(
    model,
    "<div class=\"modal-backdrop\"><div aria-modal=\"true\" class=\"modal\" role=\"alertdialog\">",
  )
  assert !shows(model, solace_task.ui_label("delete_permanently"))
  // Step two warns, and Delete stays disabled until the name matches.
  let #(model, _) = migration.update(model, migration.RemoveFiles)
  assert model.projects.removing == Some(#("a", True, ""))
  assert shows(model, "It cannot be undone")
  assert shows(
    model,
    " disabled>" <> solace_task.ui_label("delete_permanently"),
  )
  let #(model, _) = migration.update(model, migration.RemoveTyped("b"))
  assert shows(
    model,
    " disabled>" <> solace_task.ui_label("delete_permanently"),
  )
  let #(model, _) = migration.update(model, migration.RemoveTyped("a"))
  assert shows(model, solace_task.ui_label("delete_permanently"))
  assert !shows(
    model,
    " disabled>" <> solace_task.ui_label("delete_permanently"),
  )
  // Cancel closes the dialog.
  let #(model, _) = migration.update(model, migration.RemoveCancel)
  assert model.projects.removing == None
  // A reloaded list closes it too.
  let #(model, _) = migration.update(model, migration.RemoveProject("b"))
  let #(model, _) =
    migration.update(
      model,
      migration.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}",
        False,
      ),
    )
  assert model.projects.removing == None
}

pub fn typed_fields_update_the_projects_test() {
  let #(model, _) = migration.init(Nil)
  assert model.projects.target == "erlang"
  let #(model, _) = migration.update(model, migration.ProjectName("web"))
  let #(model, _) = migration.update(model, migration.ProjectDir("apps/web"))
  let #(model, _) =
    migration.update(model, migration.ProjectTarget("javascript"))
  assert #(model.projects.name, model.projects.dir, model.projects.target)
    == #("web", "apps/web", "javascript")
}

pub fn removing_needs_the_second_step_and_the_exact_name_test() {
  let #(model, _) = migration.init(Nil)
  // Nothing is being removed: every removal message is a no-op.
  let #(same, _) = migration.update(model, migration.RemoveOnly)
  assert same == model
  let #(same, _) = migration.update(model, migration.RemoveFiles)
  assert same == model
  let #(same, _) = migration.update(model, migration.RemoveTyped("a"))
  assert same == model
  let #(same, _) = migration.update(model, migration.RemoveDelete)
  assert same == model
  // Typing before the second step is ignored, and Delete before it sends nothing.
  let #(first, _) = migration.update(model, migration.RemoveProject("a"))
  let #(same, _) = migration.update(first, migration.RemoveTyped("a"))
  assert same == first
  let #(same, _) = migration.update(first, migration.RemoveDelete)
  assert same == first
  // A wrong name keeps the dialog open.
  let #(second, _) = migration.update(first, migration.RemoveFiles)
  let #(wrong, _) = migration.update(second, migration.RemoveTyped("b"))
  let #(same, _) = migration.update(wrong, migration.RemoveDelete)
  assert same == wrong
  // Adding a directory that was not found does nothing.
  let #(same, _) = migration.update(model, migration.AddProject("zzz"))
  assert same == model
}

/// A reload, such as after a reconnect, keeps what was typed; the reply to
/// a change clears it.
pub fn a_reload_keeps_typed_fields_test() {
  let list = "{\"workspace\":true,\"projects\":[],\"found\":[]}"
  let #(model, _) = migration.init(Nil)
  let #(model, _) = migration.update(model, migration.ProjectName("web"))
  let #(model, _) = migration.update(model, migration.ProjectDir("apps/web"))
  let #(model, _) = migration.update(model, migration.MigrationConnected)
  let #(model, _) =
    migration.update(model, migration.ProjectsLoaded(list, True))
  assert #(model.projects.name, model.projects.dir) == #("web", "apps/web")
  let #(model, _) =
    migration.update(model, migration.ProjectsLoaded(list, False))
  assert #(model.projects.name, model.projects.dir) == #("", "")
}

/// A reload keeps an open removal while its project is listed, and closes
/// it once the project has left the list — removed from another window,
/// for example.
pub fn a_reload_closes_a_removal_whose_project_is_gone_test() {
  let two =
    "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}"
  let one =
    "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}"
  let #(model, _) = migration.init(Nil)
  let #(model, _) = migration.update(model, migration.ProjectsLoaded(two, True))
  let #(model, _) = migration.update(model, migration.RemoveProject("b"))
  assert model.projects.removing == Some(#("b", False, ""))
  // A reload that still lists the project keeps the dialog open.
  let #(model, _) = migration.update(model, migration.ProjectsLoaded(two, True))
  assert model.projects.removing == Some(#("b", False, ""))
  // A reload where the project is gone closes it.
  let #(model, _) = migration.update(model, migration.ProjectsLoaded(one, True))
  assert model.projects.removing == None
}
