//// Audit trail of business actions.

import gleam/int
import gleam/list
import pilot/date

pub type Entry {
  Entry(at: Int, actor: String, action: String, subject: String, detail: String)
}

pub fn customer_subject(id: Int) -> String {
  "customer:" <> int.to_string(id)
}

pub fn order_subject(id: Int) -> String {
  "order:" <> int.to_string(id)
}

pub fn payment_subject(id: Int) -> String {
  "payment:" <> int.to_string(id)
}

pub fn shipment_subject(id: Int) -> String {
  "shipment:" <> int.to_string(id)
}

pub fn return_subject(id: Int) -> String {
  "return:" <> int.to_string(id)
}

/// Entries about one subject, oldest first.
pub fn for_subject(entries: List(Entry), subject: String) -> List(Entry) {
  list.filter(entries, fn(e) { e.subject == subject })
}

/// Entries with `from <= at < to`, oldest first.
pub fn between(entries: List(Entry), from: Int, to: Int) -> List(Entry) {
  list.filter(entries, fn(e) { e.at >= from && e.at < to })
}

pub fn format(entry: Entry) -> String {
  date.to_iso(date.from_unix_ms(entry.at))
  <> " "
  <> entry.actor
  <> " "
  <> entry.action
  <> " "
  <> entry.subject
  <> case entry.detail {
    "" -> ""
    detail -> ": " <> detail
  }
}
