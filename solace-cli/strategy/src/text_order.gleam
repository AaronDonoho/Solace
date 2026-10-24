//// Text ordering the Strategy page's records share.

import gleam/order
import gleam/string

/// Text in the order a person expects: letters compare without regard to
/// case, and only equal-looking texts fall back to the exact comparison.
pub fn compare(a: String, b: String) -> order.Order {
  string.compare(string.lowercase(a), string.lowercase(b))
  |> order.lazy_break_tie(fn() { string.compare(a, b) })
}
