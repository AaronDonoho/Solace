import pilot/date.{Date}
import pilot/seed

pub fn epoch_is_a_monday_test() {
  let d = date.from_unix_ms(seed.epoch)
  assert d == Date(2026, 3, 2)
  assert date.weekday(d) == date.Monday
}

pub fn round_trip_days_test() {
  assert date.from_days(0) == Date(1970, 1, 1)
  assert date.to_days(Date(2000, 2, 29)) == 11_016
  assert date.from_days(11_016) == Date(2000, 2, 29)
  assert date.from_days(date.to_days(Date(2026, 12, 31)) + 1)
    == Date(2027, 1, 1)
}

pub fn to_unix_ms_test() {
  assert date.to_unix_ms(Date(2026, 3, 2)) == seed.epoch
}

pub fn business_days_skip_weekends_test() {
  // Thursday + 2 business days = Monday
  assert date.add_business_days(Date(2026, 3, 5), 2) == Date(2026, 3, 9)
  // Friday + 1 = Monday
  assert date.add_business_days(Date(2026, 3, 6), 1) == Date(2026, 3, 9)
  // Saturday + 1 = Monday
  assert date.add_business_days(Date(2026, 3, 7), 1) == Date(2026, 3, 9)
  // Monday + 5 = next Monday
  assert date.add_business_days(Date(2026, 3, 2), 5) == Date(2026, 3, 9)
}

pub fn days_between_and_iso_test() {
  assert date.days_between(Date(2026, 2, 27), Date(2026, 3, 2)) == 3
  assert date.to_iso(Date(2026, 3, 2)) == "2026-03-02"
}
