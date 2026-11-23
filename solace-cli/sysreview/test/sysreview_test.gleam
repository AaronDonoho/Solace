import gleam/list
import gleam/option
import gleam/string
import gleeunit
import lustre/element
import schedule
import sysreview

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn every_kind_starts_chosen_test() {
  let model = sysreview.initial()
  assert model.chosen == sysreview.kinds()
  assert !sysreview.start_disabled(model)
  let page = element.to_string(sysreview.view(model))
  assert string.contains(page, "Run review")
  assert list.length(string.split(page, "checked")) == 4
}

pub fn unticking_every_kind_disables_the_run_test() {
  let #(one, _) =
    sysreview.update(
      sysreview.initial(),
      sysreview.ToggleKind(sysreview.CodeQuality, False),
    )
  assert one.chosen == [sysreview.Security, sysreview.UserExperience]
  let #(back, _) =
    sysreview.update(one, sysreview.ToggleKind(sysreview.CodeQuality, True))
  assert back.chosen == sysreview.kinds()
  let none =
    list.fold(sysreview.kinds(), sysreview.initial(), fn(model, kind) {
      sysreview.update(model, sysreview.ToggleKind(kind, False)).0
    })
  assert none.chosen == []
  assert sysreview.start_disabled(none)
  assert string.contains(
    element.to_string(sysreview.view(none)),
    "<button class=\"review-start\" disabled>",
  )
  let #(still, _) = sysreview.update(none, sysreview.StartReview)
  assert !still.starting
}

pub fn a_start_names_the_chosen_kinds_test() {
  assert sysreview.start_body([sysreview.Security, sysreview.UserExperience])
    == "{\"kinds\":[\"security\",\"user-experience\"]}"
  assert sysreview.start_body(sysreview.kinds())
    == "{\"kinds\":[\"security\",\"code-quality\",\"user-experience\"]}"
  let #(starting, _) =
    sysreview.update(sysreview.initial(), sysreview.StartReview)
  assert starting.starting
  assert sysreview.start_disabled(starting)
  assert string.contains(
    element.to_string(sysreview.view(starting)),
    "Starting…",
  )
}

pub fn a_started_review_is_confirmed_test() {
  let #(starting, _) =
    sysreview.update(sysreview.initial(), sysreview.StartReview)
  let #(started, _) =
    sysreview.update(
      starting,
      sysreview.ReviewStarted(
        [sysreview.Security, sysreview.CodeQuality],
        "{\"id\":\"r1\",\"state\":\"running\"}",
      ),
    )
  assert !started.starting
  assert started.error == ""
  assert string.contains(
    element.to_string(sysreview.view(started)),
    "Review started: Security, Code quality. Its findings will appear as tasks in Planning.",
  )
  assert !sysreview.start_disabled(started)
}

pub fn a_refused_start_shows_why_test() {
  let #(starting, _) =
    sysreview.update(sysreview.initial(), sysreview.StartReview)
  let #(refused, _) =
    sysreview.update(
      starting,
      sysreview.ReviewStarted(
        sysreview.kinds(),
        "{\"code\":\"E7001\",\"message\":\"E7001 x is not a review kind\"}",
      ),
    )
  assert !refused.starting
  assert refused.started == []
  let page = element.to_string(sysreview.view(refused))
  assert string.contains(page, "E7001 x is not a review kind")
  assert !string.contains(page, "Review started")
}

pub fn past_reviews_show_in_a_table_newest_first_test() {
  let #(shown, _) =
    sysreview.update(sysreview.initial(), sysreview.Select(sysreview.PastTab))
  let #(listed, _) =
    sysreview.update(
      shown,
      sysreview.PastLoaded(
        "[{\"id\":\"r2\",\"started\":200,\"ended\":260,\"kinds\":[\"security\",\"code-quality\"],\"version\":\"0.2.0\",\"findings\":[\"0123456789abcdef\",\"fedcba9876543210\"]},"
        <> "{\"id\":\"r1\",\"started\":100,\"ended\":150,\"kinds\":[\"user-experience\"],\"version\":null,\"findings\":[]}]",
      ),
    )
  let page = element.to_string(sysreview.view(listed))
  assert string.contains(
    page,
    "<button aria-controls=\"panel-past\" aria-selected=\"true\" id=\"tab-past\" role=\"tab\">Past reviews</button>",
  )
  assert string.contains(
    page,
    "<th>Started</th><th>Ended</th><th>Kinds</th><th>Version</th><th>Findings</th>",
  )
  let rows = [
    sysreview.clock(200)
      <> "</button></td><td>"
      <> sysreview.clock(260)
      <> "</td><td>Security, Code quality</td><td>0.2.0</td><td>2</td>",
    sysreview.clock(100)
      <> "</button></td><td>"
      <> sysreview.clock(150)
      <> "</td><td>User experience</td><td>None named</td><td>0</td>",
  ]
  let assert [newer, older] =
    list.map(rows, fn(row) {
      let assert [before, _] = string.split(page, row)
      string.length(before)
    })
  assert newer < older
  assert !string.contains(page, "review-start")
  assert !string.contains(page, "alertdialog")
}

pub fn a_clicked_review_shows_its_details_until_closed_test() {
  let #(shown, _) =
    sysreview.update(sysreview.initial(), sysreview.Select(sysreview.PastTab))
  let #(listed, _) =
    sysreview.update(
      shown,
      sysreview.PastLoaded(
        "[{\"id\":\"r2\",\"started\":200,\"ended\":260,\"kinds\":[\"security\"],\"version\":\"0.2.0\",\"findings\":[\"0123456789abcdef\"]},"
        <> "{\"id\":\"r1\",\"started\":100,\"ended\":150,\"kinds\":[\"user-experience\"],\"version\":null,\"findings\":[]}]",
      ),
    )
  let #(open, _) = sysreview.update(listed, sysreview.OpenReview("r2"))
  let dialog = element.to_string(sysreview.view(open))
  assert string.contains(dialog, "role=\"alertdialog\"")
  assert string.contains(
    dialog,
    "<dt>Started</dt><dd>" <> sysreview.clock(200) <> "</dd>",
  )
  assert string.contains(
    dialog,
    "<dt>Ended</dt><dd>" <> sysreview.clock(260) <> "</dd>",
  )
  assert string.contains(dialog, "<dt>Kinds</dt><dd>Security</dd>")
  assert string.contains(dialog, "<dt>Version</dt><dd>0.2.0</dd>")
  assert string.contains(
    dialog,
    "<a data-task=\"0123456789abcdef\" href=\"/task/0123456789abcdef\">task:0123456789abcdef</a>",
  )
  let #(other, _) = sysreview.update(open, sysreview.OpenReview("r1"))
  let none = element.to_string(sysreview.view(other))
  assert string.contains(none, "<dt>Version</dt><dd>None named</dd>")
  assert string.contains(none, "<dt>Findings</dt><dd>None</dd>")
  let #(closed, _) = sysreview.update(other, sysreview.CloseReview)
  assert !string.contains(
    element.to_string(sysreview.view(closed)),
    "alertdialog",
  )
}

pub fn no_completed_reviews_says_so_test() {
  let #(shown, _) =
    sysreview.update(sysreview.initial(), sysreview.Select(sysreview.PastTab))
  let #(empty, _) = sysreview.update(shown, sysreview.PastLoaded("[]"))
  let page = element.to_string(sysreview.view(empty))
  assert string.contains(page, "No completed reviews yet.")
  assert !string.contains(page, "<table")
  let #(refused, _) =
    sysreview.update(
      shown,
      sysreview.PastLoaded("{\"message\":\"E7001 unknown route\"}"),
    )
  assert string.contains(
    element.to_string(sysreview.view(refused)),
    "E7001 unknown route",
  )
}

pub fn the_log_shows_as_markdown_with_task_links_test() {
  let #(shown, _) =
    sysreview.update(sysreview.initial(), sysreview.Select(sysreview.LogTab))
  let #(read, _) =
    sysreview.update(
      shown,
      sysreview.LogLoaded(
        "{\"text\":\"# System reviews\\n\\n## 2026-11-22: Security\\n\\n- Version: 0.2.0\\n- Findings: task:0123456789abcdef\\n\"}",
      ),
    )
  let page = element.to_string(sysreview.view(read))
  assert string.contains(
    page,
    "<button aria-controls=\"panel-log\" aria-selected=\"true\" id=\"tab-log\" role=\"tab\">Log</button>",
  )
  assert string.contains(page, ">System reviews</h")
  assert string.contains(page, ">2026-11-22: Security</h")
  assert string.contains(page, "Version: 0.2.0")
  assert string.contains(
    page,
    "<a href=\"/task/0123456789abcdef\" data-task=\"0123456789abcdef\">task:0123456789abcdef</a>",
  )
  assert !string.contains(page, "# System reviews")
  assert !string.contains(page, "<textarea")
  assert !string.contains(page, "review-start")
}

pub fn an_unreadable_log_says_why_test() {
  let #(shown, _) =
    sysreview.update(sysreview.initial(), sysreview.Select(sysreview.LogTab))
  let #(refused, _) =
    sysreview.update(
      shown,
      sysreview.LogLoaded("{\"message\":\"E7001 no such file\"}"),
    )
  let page = element.to_string(sysreview.view(refused))
  assert string.contains(page, "E7001 no such file")
  assert string.contains(page, "role=\"alert\"")
}

/// A weekly schedule, and a one-off one whose time has passed, as the server lists them.
fn listed_schedules() -> String {
  "[{\"id\":\"a\",\"kinds\":[\"security\",\"code-quality\"],\"repeat\":\"weekly\",\"time\":\"09:00\",\"weekday\":1,\"next\":1800000000},"
  <> "{\"id\":\"b\",\"kinds\":[\"user-experience\"],\"repeat\":\"once\",\"time\":\"10:00\",\"date\":\"2026-12-01\",\"next\":null}]"
}

pub fn schedules_list_in_their_tab_test() {
  assert list.map(sysreview.tabs(), sysreview.tab_label)
    == ["Run review", "Schedules", "Past reviews", "Log"]
  let #(shown, _) =
    sysreview.update(
      sysreview.initial(),
      sysreview.Select(sysreview.SchedulesTab),
    )
  let #(listed, _) =
    sysreview.update(shown, sysreview.SchedulesLoaded(listed_schedules()))
  let page = element.to_string(sysreview.view(listed))
  assert string.contains(page, "Add schedule")
  assert string.contains(
    page,
    "<th>Kinds</th><th>Repeats</th><th>Next run</th>",
  )
  assert string.contains(page, "Security, Code quality")
  assert string.contains(
    page,
    "<td>Weekly, Mondays 09:00</td><td>"
      <> sysreview.clock(1_800_000_000)
      <> "</td>",
  )
  assert string.contains(page, "<td>Once, 2026-12-01 10:00</td><td>None</td>")
  let #(empty, _) = sysreview.update(shown, sysreview.SchedulesLoaded("[]"))
  assert string.contains(
    element.to_string(sysreview.view(empty)),
    "No schedules yet.",
  )
  let #(refused, _) =
    sysreview.update(
      shown,
      sysreview.SchedulesLoaded("{\"message\":\"E7001 unknown route\"}"),
    )
  assert string.contains(
    element.to_string(sysreview.view(refused)),
    "E7001 unknown route",
  )
}

pub fn an_added_schedule_is_listed_once_saved_test() {
  let #(adding, _) =
    sysreview.update(sysreview.initial(), sysreview.AddSchedule)
  let form = element.to_string(sysreview.view(adding))
  assert string.contains(form, "role=\"alertdialog\"")
  assert string.contains(form, "Once on a date")
  assert !string.contains(form, "Delete")
  let #(weekly, _) = sysreview.update(adding, sysreview.EditRepeat("weekly"))
  let #(wednesday, _) = sysreview.update(weekly, sysreview.EditWeekday("3"))
  let #(timed, _) = sysreview.update(wednesday, sysreview.EditTime("17:45"))
  let assert option.Some(s) = timed.editing
  assert schedule.repeat_text(s) == "Weekly, Wednesdays 17:45"
  assert string.contains(element.to_string(sysreview.view(timed)), "Wednesday")
  let #(saving, _) = sysreview.update(timed, sysreview.SaveSchedule)
  assert saving.saving
  assert sysreview.save_disabled(saving)
  let #(saved, _) =
    sysreview.update(saving, sysreview.ScheduleSaved(listed_schedules()))
  assert saved.editing == option.None
  assert !saved.saving
  let assert option.Some(all) = saved.schedules
  assert list.length(all) == 2
}

pub fn a_schedule_needs_a_kind_and_a_date_to_save_test() {
  let #(adding, _) =
    sysreview.update(sysreview.initial(), sysreview.AddSchedule)
  assert !sysreview.save_disabled(adding)
  let none =
    list.fold(sysreview.kinds(), adding, fn(model, kind) {
      sysreview.update(model, sysreview.EditKind(kind, False)).0
    })
  assert sysreview.save_disabled(none)
  assert string.contains(
    element.to_string(sysreview.view(none)),
    "<button class=\"schedule-save\" disabled>",
  )
  let #(still, _) = sysreview.update(none, sysreview.SaveSchedule)
  assert !still.saving
  let #(one, _) =
    sysreview.update(none, sysreview.EditKind(sysreview.CodeQuality, True))
  let #(once, _) = sysreview.update(one, sysreview.EditRepeat("once"))
  assert sysreview.save_disabled(once)
  let #(dated, _) = sysreview.update(once, sysreview.EditDate("2026-12-01"))
  assert !sysreview.save_disabled(dated)
  let assert option.Some(s) = dated.editing
  assert s.kinds == ["code-quality"]
}

pub fn a_clicked_schedule_opens_to_change_or_delete_test() {
  let #(listed, _) =
    sysreview.update(
      sysreview.initial(),
      sysreview.SchedulesLoaded(listed_schedules()),
    )
  let #(open, _) = sysreview.update(listed, sysreview.EditSchedule("b"))
  let form = element.to_string(sysreview.view(open))
  assert string.contains(form, "value=\"2026-12-01\"")
  assert string.contains(form, "value=\"10:00\"")
  assert string.contains(form, "Delete")
  let #(changed, _) = sysreview.update(open, sysreview.EditDate("2027-01-04"))
  let #(saving, _) = sysreview.update(changed, sysreview.SaveSchedule)
  let #(refused, _) =
    sysreview.update(
      saving,
      sysreview.ScheduleSaved(
        "{\"message\":\"E7001 that date and time has passed; choose a later one\"}",
      ),
    )
  assert !refused.saving
  assert string.contains(
    element.to_string(sysreview.view(refused)),
    "that date and time has passed",
  )
  let #(deleting, _) = sysreview.update(refused, sysreview.DeleteSchedule)
  assert deleting.saving
  let #(closed, _) = sysreview.update(refused, sysreview.CloseSchedule)
  assert closed.editing == option.None
  assert !string.contains(
    element.to_string(sysreview.view(closed)),
    "alertdialog",
  )
}

/// A weekly review missed and unread, and a one-off one already read, as the server lists them.
fn missed_reviews() -> String {
  "[{\"schedule\":\"a\",\"kinds\":[\"security\",\"code-quality\"],\"repeat\":\"weekly\",\"due\":1795424400,\"read\":false},"
  <> "{\"schedule\":\"b\",\"kinds\":[\"user-experience\"],\"repeat\":\"once\",\"due\":1795424400,\"read\":true}]"
}

pub fn a_missed_review_shows_above_the_tabs_until_answered_test() {
  assert !string.contains(
    element.to_string(sysreview.view(sysreview.initial())),
    "class=\"missed-reviews\"",
  )
  let #(heard, _) =
    sysreview.update(
      sysreview.initial(),
      sysreview.MissedHeard("{\"missedReviews\":" <> missed_reviews() <> "}"),
    )
  let page = element.to_string(sysreview.view(heard))
  assert string.contains(page, "Weekly security, code quality review was due ")
  assert string.contains(page, "Scheduled user experience review was due ")
  assert string.contains(page, "and didn&#39;t run.")
    || string.contains(page, "and didn't run.")
  assert list.length(string.split(page, "Run now")) == 3
  assert list.length(string.split(page, "Skip")) == 3
  let assert [banner, tabs] = string.split(page, "role=\"tablist\"")
  assert string.contains(banner, "class=\"missed-reviews\"")
  assert !string.contains(tabs, "class=\"missed-reviews\"")
  let #(refused, _) =
    sysreview.update(
      heard,
      sysreview.MissedAnswered(
        "{\"message\":\"E7023 no missed review for schedule a\"}",
      ),
    )
  assert refused.missed == heard.missed
  assert string.contains(
    element.to_string(sysreview.view(refused)),
    "no missed review for schedule a",
  )
  let #(answered, _) = sysreview.update(refused, sysreview.MissedAnswered("[]"))
  assert answered.missed == []
  assert !string.contains(
    element.to_string(sysreview.view(answered)),
    "class=\"missed-reviews\"",
  )
}

pub fn showing_the_page_keeps_its_missed_reviews_test() {
  let #(heard, _) =
    sysreview.update(
      sysreview.initial(),
      sysreview.MissedHeard("{\"missedReviews\":" <> missed_reviews() <> "}"),
    )
  let #(shown, _) = sysreview.update(heard, sysreview.Shown(True))
  assert shown.shown
  assert list.length(shown.missed) == 2
  let #(hidden, _) = sysreview.update(shown, sysreview.Shown(False))
  assert !hidden.shown
  assert sysreview.missed_path("a", "run") == "/system-reviews/missed/a/run"
}
