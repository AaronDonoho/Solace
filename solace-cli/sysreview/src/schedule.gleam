//// A schedule of system reviews, as the server's schedule routes list and take it.

import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None}

pub type Repeat {
  Daily
  Weekly
  Monthly
  Once
}

/// A schedule: the ids of its review kinds, how it repeats at `time` (HH:MM),
/// on `weekday` (1 for Monday), `day` of the month or `date` (YYYY-MM-DD) as
/// `repeat` needs, and when it next comes due, in seconds; none once a one-off
/// time has passed. A schedule not yet saved has no id.
pub type Schedule {
  Schedule(
    id: String,
    kinds: List(String),
    repeat: Repeat,
    time: String,
    weekday: Int,
    day: Int,
    date: String,
    next: Option(Int),
  )
}

/// A schedule not yet saved, of `kinds`, daily at 09:00.
pub fn new(kinds: List(String)) -> Schedule {
  Schedule(
    id: "",
    kinds:,
    repeat: Daily,
    time: "09:00",
    weekday: 1,
    day: 1,
    date: "",
    next: None,
  )
}

pub fn repeats() -> List(Repeat) {
  [Daily, Weekly, Monthly, Once]
}

/// The id the server names a repeat by.
pub fn repeat_id(repeat: Repeat) -> String {
  case repeat {
    Daily -> "daily"
    Weekly -> "weekly"
    Monthly -> "monthly"
    Once -> "once"
  }
}

pub fn repeat_from_id(id: String) -> Result(Repeat, Nil) {
  list.find(repeats(), fn(repeat) { repeat_id(repeat) == id })
}

pub fn repeat_label(repeat: Repeat) -> String {
  case repeat {
    Daily -> "Daily"
    Weekly -> "Weekly"
    Monthly -> "Monthly"
    Once -> "Once on a date"
  }
}

/// The weekdays, 1 for Monday to 7 for Sunday.
pub fn weekdays() -> List(Int) {
  [1, 2, 3, 4, 5, 6, 7]
}

pub fn weekday_name(weekday: Int) -> String {
  case weekday {
    1 -> "Monday"
    2 -> "Tuesday"
    3 -> "Wednesday"
    4 -> "Thursday"
    5 -> "Friday"
    6 -> "Saturday"
    _ -> "Sunday"
  }
}

/// The days of a month a schedule can name, 1 to 31; a shorter month runs on its last day.
pub fn days() -> List(Int) {
  list.index_map(list.repeat(Nil, 31), fn(_, i) { i + 1 })
}

/// How the schedule repeats, as its row shows it: "Weekly, Mondays 09:00".
pub fn repeat_text(schedule: Schedule) -> String {
  case schedule.repeat {
    Daily -> "Daily, " <> schedule.time
    Weekly ->
      "Weekly, " <> weekday_name(schedule.weekday) <> "s " <> schedule.time
    Monthly ->
      "Monthly, day " <> int.to_string(schedule.day) <> ", " <> schedule.time
    Once -> "Once, " <> schedule.date <> " " <> schedule.time
  }
}

/// Whether the schedule names all it needs to be saved: a kind, a time and,
/// to run once, a date.
pub fn ready(schedule: Schedule) -> Bool {
  schedule.kinds != []
  && schedule.time != ""
  && { schedule.repeat != Once || schedule.date != "" }
}

pub fn decoder() -> decode.Decoder(Schedule) {
  use id <- decode.field("id", decode.string)
  use kinds <- decode.field("kinds", decode.list(decode.string))
  use repeat <- decode.field("repeat", repeat_decoder())
  use time <- decode.field("time", decode.string)
  use weekday <- decode.optional_field("weekday", 1, decode.int)
  use day <- decode.optional_field("day", 1, decode.int)
  use date <- decode.optional_field("date", "", decode.string)
  use next <- decode.optional_field("next", None, decode.optional(decode.int))
  decode.success(Schedule(
    id:,
    kinds:,
    repeat:,
    time:,
    weekday:,
    day:,
    date:,
    next:,
  ))
}

pub fn repeat_decoder() -> decode.Decoder(Repeat) {
  use id <- decode.then(decode.string)
  case repeat_from_id(id) {
    Ok(repeat) -> decode.success(repeat)
    Error(_) -> decode.failure(Daily, "Repeat")
  }
}

/// A save request's body: the kinds, the repeat, the time and the field the repeat needs.
pub fn body(schedule: Schedule) -> String {
  let when = case schedule.repeat {
    Daily -> []
    Weekly -> [#("weekday", json.int(schedule.weekday))]
    Monthly -> [#("day", json.int(schedule.day))]
    Once -> [#("date", json.string(schedule.date))]
  }
  json.to_string(
    json.object([
      #("kinds", json.array(schedule.kinds, json.string)),
      #("repeat", json.string(repeat_id(schedule.repeat))),
      #("time", json.string(schedule.time)),
      ..when
    ]),
  )
}
