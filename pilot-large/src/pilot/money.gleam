//// Helpers for amounts of money. All amounts are integer cents.

import gleam/int
import gleam/list
import gleam/string

/// `percent` percent of `amount`, rounded down to the cent.
pub fn percent(amount: Int, percent: Int) -> Int {
  amount * percent / 100
}

/// `amount` multiplied by a rate in basis points (1/100 of a percent),
/// rounded half up to the cent.
pub fn basis_points(amount: Int, bps: Int) -> Int {
  { amount * bps + 5000 } / 10_000
}

/// `share` of `total` spread over `part` of `whole`, rounded down.
/// Used to apportion order-level discounts to individual lines.
pub fn apportion(total: Int, part: Int, whole: Int) -> Int {
  case whole {
    0 -> 0
    _ -> total * part / whole
  }
}

pub fn sum(amounts: List(Int)) -> Int {
  list.fold(amounts, 0, int.add)
}

/// Formats cents as dollars, e.g. `format(123456)` is `"$1,234.56"`.
pub fn format(cents: Int) -> String {
  let sign = case cents < 0 {
    True -> "-"
    False -> ""
  }
  let cents = int.absolute_value(cents)
  let dollars = group_thousands(int.to_string(cents / 100))
  let rest = string.pad_start(int.to_string(cents % 100), 2, "0")
  sign <> "$" <> dollars <> "." <> rest
}

fn group_thousands(digits: String) -> String {
  case string.length(digits) > 3 {
    True -> {
      let split = string.length(digits) - 3
      group_thousands(string.slice(digits, 0, split))
      <> ","
      <> string.slice(digits, split, 3)
    }
    False -> digits
  }
}
