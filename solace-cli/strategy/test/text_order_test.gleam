//// `text_order`'s ordering for the Strategy page's titles.

import gleam/order
import text_order

pub fn titles_compare_without_case_then_exactly_test() {
  assert text_order.compare("apple", "Banana") == order.Lt
  assert text_order.compare("Apple", "apple") == order.Lt
  assert text_order.compare("apple", "apple") == order.Eq
}
