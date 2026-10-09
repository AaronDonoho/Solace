import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/element
import migration
import migration/api

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
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(html, "Migration")
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
    )
  let html = element.to_string(migration.view(model))
  assert string.contains(html, "Last checked abc")
}

pub fn registering_outside_a_browser_is_an_error_test() {
  let assert Error(_) = migration.register()
}
