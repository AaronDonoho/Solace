//// Calendar dates (UTC) derived from Unix millisecond timestamps.

import gleam/int
import gleam/order
import gleam/string

pub type Date {
  Date(year: Int, month: Int, day: Int)
}

pub type Weekday {
  Monday
  Tuesday
  Wednesday
  Thursday
  Friday
  Saturday
  Sunday
}

pub const ms_per_day = 86_400_000

/// The calendar date containing the timestamp. Timestamps before 1970 are
/// not supported.
pub fn from_unix_ms(ms: Int) -> Date {
  from_days(ms / ms_per_day)
}

/// Midnight UTC at the start of the date, in Unix milliseconds.
pub fn to_unix_ms(date: Date) -> Int {
  to_days(date) * ms_per_day
}

/// Days since 1970-01-01.
pub fn to_days(date: Date) -> Int {
  let y = case date.month <= 2 {
    True -> date.year - 1
    False -> date.year
  }
  let era = y / 400
  let year_of_era = y - era * 400
  let shifted_month = case date.month > 2 {
    True -> date.month - 3
    False -> date.month + 9
  }
  let day_of_year = { 153 * shifted_month + 2 } / 5 + date.day - 1
  let day_of_era =
    year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year
  era * 146_097 + day_of_era - 719_468
}

/// The date `days` days after 1970-01-01.
pub fn from_days(days: Int) -> Date {
  let z = days + 719_468
  let era = z / 146_097
  let day_of_era = z - era * 146_097
  let year_of_era =
    {
      day_of_era
      - day_of_era
      / 1460
      + day_of_era
      / 36_524
      - day_of_era
      / 146_096
    }
    / 365
  let day_of_year =
    day_of_era - { 365 * year_of_era + year_of_era / 4 - year_of_era / 100 }
  let mp = { 5 * day_of_year + 2 } / 153
  let day = day_of_year - { 153 * mp + 2 } / 5 + 1
  let month = case mp < 10 {
    True -> mp + 3
    False -> mp - 9
  }
  let year = year_of_era + era * 400
  let year = case month <= 2 {
    True -> year + 1
    False -> year
  }
  Date(year:, month:, day:)
}

pub fn weekday(date: Date) -> Weekday {
  // 1970-01-01 was a Thursday.
  case { to_days(date) + 3 } % 7 {
    0 -> Monday
    1 -> Tuesday
    2 -> Wednesday
    3 -> Thursday
    4 -> Friday
    5 -> Saturday
    _ -> Sunday
  }
}

pub fn is_weekend(date: Date) -> Bool {
  case weekday(date) {
    Saturday | Sunday -> True
    _ -> False
  }
}

pub fn add_days(date: Date, days: Int) -> Date {
  from_days(to_days(date) + days)
}

/// The date `days` business days after `date`, skipping Saturdays and
/// Sundays. Starting on a weekend counts from the following Monday.
pub fn add_business_days(date: Date, days: Int) -> Date {
  case days <= 0 {
    True -> date
    False -> {
      let next = add_days(date, 1)
      case is_weekend(next) {
        True -> add_business_days(next, days)
        False -> add_business_days(next, days - 1)
      }
    }
  }
}

pub fn days_between(from: Date, to: Date) -> Int {
  to_days(to) - to_days(from)
}

pub fn compare(a: Date, b: Date) -> order.Order {
  int.compare(to_days(a), to_days(b))
}

/// `YYYY-MM-DD`.
pub fn to_iso(date: Date) -> String {
  string.pad_start(int.to_string(date.year), 4, "0")
  <> "-"
  <> string.pad_start(int.to_string(date.month), 2, "0")
  <> "-"
  <> string.pad_start(int.to_string(date.day), 2, "0")
}
