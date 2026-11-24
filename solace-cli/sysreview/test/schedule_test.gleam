import gleam/dynamic/decode
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import schedule

pub fn a_save_sends_the_field_its_repeat_needs_test() {
  let daily = schedule.new(["security"])
  assert schedule.body(daily)
    == "{\"kinds\":[\"security\"],\"repeat\":\"daily\",\"time\":\"09:00\"}"
  assert schedule.body(
      schedule.Schedule(
        ..daily,
        repeat: schedule.Weekly,
        weekday: 3,
        time: "17:45",
      ),
    )
    == "{\"kinds\":[\"security\"],\"repeat\":\"weekly\",\"time\":\"17:45\",\"weekday\":3}"
  assert schedule.body(
      schedule.Schedule(..daily, repeat: schedule.Monthly, day: 31),
    )
    == "{\"kinds\":[\"security\"],\"repeat\":\"monthly\",\"time\":\"09:00\",\"day\":31}"
  assert schedule.body(
      schedule.Schedule(..daily, repeat: schedule.Once, date: "2026-12-01"),
    )
    == "{\"kinds\":[\"security\"],\"repeat\":\"once\",\"time\":\"09:00\",\"date\":\"2026-12-01\"}"
}

pub fn each_repeat_reads_as_its_row_shows_it_test() {
  let s = schedule.new(["security"])
  assert schedule.repeat_text(s) == "Daily, 09:00"
  assert schedule.repeat_text(
      schedule.Schedule(..s, repeat: schedule.Weekly, weekday: 1),
    )
    == "Weekly, Mondays 09:00"
  assert schedule.repeat_text(
      schedule.Schedule(..s, repeat: schedule.Monthly, day: 15, time: "07:30"),
    )
    == "Monthly, day 15, 07:30"
  assert schedule.repeat_text(
      schedule.Schedule(..s, repeat: schedule.Once, date: "2026-12-01"),
    )
    == "Once, 2026-12-01 09:00"
}

pub fn a_listed_schedule_reads_with_only_its_repeats_field_test() {
  let assert Ok(s) =
    json.parse(
      "{\"id\":\"a\",\"kinds\":[\"security\"],\"repeat\":\"once\",\"time\":\"10:00\",\"date\":\"2026-12-01\",\"next\":null}",
      schedule.decoder(),
    )
  assert s.repeat == schedule.Once
  assert s.date == "2026-12-01"
  assert s.next == None
  let assert Ok(w) =
    json.parse(
      "{\"id\":\"b\",\"kinds\":[],\"repeat\":\"weekly\",\"time\":\"09:00\",\"weekday\":5,\"next\":1800000000}",
      schedule.decoder(),
    )
  assert #(w.weekday, w.next) == #(5, Some(1_800_000_000))
  assert json.parse(
      "{\"id\":\"c\",\"kinds\":[],\"repeat\":\"hourly\",\"time\":\"09:00\"}",
      schedule.decoder(),
    )
    |> result.is_error
  assert list.length(schedule.days()) == 31
}

pub fn each_weekday_has_its_name_test() {
  assert list.map([1, 2, 3, 4, 5, 6, 7], schedule.weekday_name)
    == [
      "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
      "Sunday",
    ]
}
