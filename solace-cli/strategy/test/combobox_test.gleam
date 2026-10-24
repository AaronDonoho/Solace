import combobox
import gleam/list
import gleam/option.{None, Some}

pub fn matches_are_case_insensitive_literal_substrings_with_prefixes_first_test() {
  let items = [
    combobox.Item("a", "Abe", "", ["Abe", "a"]),
    combobox.Item("b", "Bee", "", ["Bee", "b"]),
    combobox.Item("c", "Alpha", "", ["Alpha", "c"]),
  ]
  assert ids(combobox.matches(items, "BE")) == ["b", "a"]
  assert ids(combobox.matches(items, "alp")) == ["c"]
  assert ids(combobox.matches(items, "ALPH")) == ["c"]
  assert ids(combobox.matches(items, "ALPA")) == []
  assert ids(combobox.matches(items, "alpah")) == []
}

pub fn ids_match_and_ties_fall_back_to_the_id_test() {
  let items = [
    combobox.Item("m2", "Later", "", ["Later", "m2"]),
    combobox.Item("b", "Same", "", ["Same", "b"]),
    combobox.Item("a", "Same", "", ["Same", "a"]),
  ]
  assert ids(combobox.matches(items, "M2")) == ["m2"]
  assert ids(combobox.matches(items, "same")) == ["a", "b"]
}

pub fn keys_move_choose_and_escape_test() {
  let items = [
    combobox.Item("b", "Same", "", ["Same", "b"]),
    combobox.Item("a", "Same", "", ["Same", "a"]),
  ]
  let state = combobox.typing(combobox.blank(), "same")
  let #(state, picked) = combobox.key(state, "ArrowDown", items, "a")
  assert picked == None
  assert state.active == Some(0)
  let #(state, picked) = combobox.key(state, "ArrowDown", items, "a")
  assert picked == None
  assert state.active == Some(1)
  let #(state, picked) = combobox.key(state, "ArrowUp", items, "a")
  assert picked == None
  assert state.active == Some(0)
  let #(state, picked) = combobox.key(state, "Enter", items, "a")
  assert picked == Some("a")
  assert !state.open
  let typing = combobox.typing(combobox.blank(), "zz")
  let #(state, picked) = combobox.key(typing, "Escape", items, "a")
  assert picked == None
  assert !state.open
  assert state.text == None
  let #(state, picked) = combobox.key(typing, "Tab", items, "a")
  assert picked == None
  assert state == typing
  // A closed list opens at the current choice, without moving past it.
  let #(state, picked) = combobox.key(combobox.blank(), "ArrowDown", items, "b")
  assert picked == None
  assert state.open
  assert state.active == Some(1)
  let #(state, picked) = combobox.key(combobox.blank(), "ArrowUp", items, "a")
  assert picked == None
  assert state.open
  assert state.active == Some(0)
  // Enter and Escape keep their defaults while the list is closed.
  let closed = combobox.blank()
  let #(state, picked) = combobox.key(closed, "Enter", items, "a")
  assert picked == None
  assert state == closed
  let #(state, picked) = combobox.key(closed, "Escape", items, "a")
  assert picked == None
  assert state == closed
  assert combobox.handles_key("ArrowDown", False)
  assert combobox.handles_key("Enter", True)
  assert !combobox.handles_key("Enter", False)
  assert !combobox.handles_key("Escape", False)
  assert !combobox.handles_key("Tab", True)
}

fn ids(items: List(combobox.Item)) -> List(String) {
  list.map(items, fn(item) { item.id })
}
