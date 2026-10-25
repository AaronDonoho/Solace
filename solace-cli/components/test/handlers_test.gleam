//// The first-handler dispatcher.

import gleam/option.{None, Some}
import handlers

pub fn the_first_answer_wins_test() {
  assert handlers.first([fn() { Some(1) }, fn() { Some(2) }], 0) == 1
}

pub fn a_later_answer_is_used_when_earlier_handlers_pass_test() {
  assert handlers.first([fn() { None }, fn() { Some(2) }], 0) == 2
}

pub fn the_fallback_is_used_when_no_handler_answers_test() {
  assert handlers.first([fn() { None }, fn() { None }], 0) == 0
}
