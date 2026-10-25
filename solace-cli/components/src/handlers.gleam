//// Dispatches a message to the first handler that answers it.

import gleam/option.{type Option, None, Some}

/// The first handler's reply, or `fallback` when none answers.
pub fn first(handlers: List(fn() -> Option(a)), fallback: a) -> a {
  case handlers {
    [] -> fallback
    [handle, ..rest] ->
      case handle() {
        Some(reply) -> reply
        None -> first(rest, fallback)
      }
  }
}
