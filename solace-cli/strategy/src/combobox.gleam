//// A text input that suggests records as the human types.

import gleam/dynamic/decode
import gleam/int
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import lustre/attribute
import lustre/element
import lustre/element/html
import lustre/event
import text_order

pub type State {
  State(open: Bool, text: Option(String), active: Option(Int))
}

/// A suggestion: what choosing it sends, what it is called, and the extra
/// text that tells two records with one title apart.
pub type Item {
  Item(id: String, label: String, hint: String, terms: List(String))
}

pub type Handlers(msg) {
  Handlers(
    on_text: fn(String) -> msg,
    on_open: msg,
    on_key: fn(String) -> msg,
    on_choose: fn(String) -> msg,
    on_blur: msg,
  )
}

pub fn blank() -> State {
  State(False, None, None)
}

pub fn typing(state: State, text: String) -> State {
  State(True, Some(text), case state.text == Some(text) {
    True -> state.active
    False -> None
  })
}

/// Opens the list at the current choice.
pub fn opened(state: State, items: List(Item), chosen: String) -> State {
  State(True, None, case state.open {
    True -> state.active
    False -> index_of(matches(items, ""), chosen)
  })
}

pub fn suggestions(state: State, items: List(Item)) -> List(Item) {
  matches(items, option.unwrap(state.text, ""))
}

/// The records whose label, id or other terms hold `query`, ignoring case.
/// Prefixes come first, then titles, then ids.
pub fn matches(items: List(Item), query: String) -> List(Item) {
  let query = string.lowercase(query)
  list.filter(items, fn(item) {
    list.any(item.terms, fn(term) {
      string.contains(string.lowercase(term), query)
    })
  })
  |> list.sort(fn(a, b) {
    int.compare(rank(a, query), rank(b, query))
    |> order.break_tie(text_order.compare(a.label, b.label))
    |> order.break_tie(string.compare(a.id, b.id))
  })
}

pub fn key(
  state: State,
  key: String,
  items: List(Item),
  chosen: String,
) -> #(State, Option(String)) {
  let found = suggestions(state, items)
  case key {
    "ArrowDown" -> #(arrowed(state, found, items, chosen, 1), None)
    "ArrowUp" -> #(arrowed(state, found, items, chosen, -1), None)
    "Enter" ->
      case state.open, active_item(found, state.active) {
        True, Some(item) -> #(blank(), Some(item.id))
        True, None ->
          case found {
            [item, ..] -> #(blank(), Some(item.id))
            [] -> #(blank(), None)
          }
        False, _ -> #(state, None)
      }
    "Escape" -> #(blank(), None)
    _ -> #(state, None)
  }
}

pub fn view(
  name: String,
  key: String,
  state: State,
  chosen: String,
  items: List(Item),
  handlers: Handlers(msg),
) -> element.Element(msg) {
  let found = suggestions(state, items)
  let shown = case state.text {
    Some(text) -> text
    None -> chosen_label(items, chosen)
  }
  html.div([attribute.class("combobox")], [
    html.label([attribute.attribute("for", key <> "-input")], [
      html.text(name),
    ]),
    html.div([attribute.class("choice")], [
      html.input(input_attributes(
        key,
        state,
        shown,
        active_id(key, state, found),
        handlers,
      )),
      options_view(key, state.open, found, chosen, state.active, handlers),
    ]),
  ])
}

fn rank(item: Item, query: String) -> Int {
  case
    list.any(item.terms, fn(term) {
      string.starts_with(string.lowercase(term), query)
    })
  {
    True -> 0
    False -> 1
  }
}

fn moved(active: Option(Int), count: Int, step: Int) -> Option(Int) {
  case count {
    0 -> None
    _ ->
      case active {
        None ->
          case step {
            1 -> Some(0)
            _ -> Some(count - 1)
          }
        Some(index) -> Some(int.min(int.max(index + step, 0), count - 1))
      }
  }
}

fn index_of(items: List(Item), id: String) -> Option(Int) {
  case
    list.index_map(items, fn(item, index) { #(item.id, index) })
    |> list.find(fn(pair) { pair.0 == id })
  {
    Ok(pair) -> Some(pair.1)
    Error(_) -> None
  }
}

fn active_item(items: List(Item), active: Option(Int)) -> Option(Item) {
  case active {
    Some(index) ->
      case list.drop(items, index) {
        [item, ..] -> Some(item)
        [] -> None
      }
    None -> None
  }
}

fn chosen_label(items: List(Item), chosen: String) -> String {
  case chosen {
    "" -> ""
    id ->
      list.find(items, fn(item) { item.id == id })
      |> result.map(fn(item) { item.label })
      |> result.unwrap(id)
  }
}

fn bool_text(value: Bool) -> String {
  case value {
    True -> "true"
    False -> "false"
  }
}

fn arrowed(
  state: State,
  found: List(Item),
  items: List(Item),
  chosen: String,
  step: Int,
) -> State {
  case state.open {
    False -> opened(state, items, chosen)
    True ->
      State(True, state.text, moved(state.active, list.length(found), step))
  }
}

fn input_attributes(
  key: String,
  state: State,
  shown: String,
  active: Option(String),
  handlers: Handlers(msg),
) -> List(attribute.Attribute(msg)) {
  [
    attribute.id(key <> "-input"),
    attribute.type_("text"),
    attribute.value(shown),
    attribute.placeholder("All"),
    attribute.attribute("role", "combobox"),
    attribute.attribute("aria-autocomplete", "list"),
    attribute.attribute("aria-expanded", bool_text(state.open)),
    attribute.attribute("aria-controls", key <> "-list"),
    case active {
      Some(id) -> attribute.attribute("aria-activedescendant", id)
      None -> attribute.none()
    },
    attribute.attribute("autocomplete", "off"),
    event.on_input(handlers.on_text),
    event.on_click(handlers.on_open),
    event.on("blur", decode.success(handlers.on_blur)),
    key_attribute(state, handlers.on_key),
  ]
}

fn key_attribute(
  state: State,
  on_key: fn(String) -> msg,
) -> attribute.Attribute(msg) {
  // Other keys, Tab among them, keep their default.
  event.advanced("keydown", {
    use pressed <- decode.field("key", decode.string)
    let handler =
      event.handler(
        on_key(pressed),
        prevent_default: True,
        stop_propagation: False,
      )
    case handles_key(pressed, state.open) {
      True -> decode.success(handler)
      False -> decode.failure(handler, "a combobox key")
    }
  })
}

/// Whether the combobox acts on `key` while its list is `open`; a key it
/// leaves alone keeps the browser's default.
pub fn handles_key(key: String, open: Bool) -> Bool {
  case key {
    "ArrowDown" | "ArrowUp" -> True
    "Enter" | "Escape" -> open
    _ -> False
  }
}

fn options_view(
  key: String,
  open: Bool,
  found: List(Item),
  chosen: String,
  active: Option(Int),
  handlers: Handlers(msg),
) -> element.Element(msg) {
  html.ul(
    [
      attribute.id(key <> "-list"),
      attribute.role("listbox"),
      attribute.class("options"),
      attribute.style("display", case open {
        True -> "block"
        False -> "none"
      }),
      // A press on the list or its scrollbar keeps the input focused.
      event.advanced(
        "mousedown",
        decode.success(event.handler(
          handlers.on_open,
          prevent_default: True,
          stop_propagation: False,
        )),
      ),
    ],
    case open {
      False -> []
      True ->
        case found {
          [] -> [
            html.li(
              [attribute.class("no-match"), attribute.role("presentation")],
              [html.text("No matches.")],
            ),
          ]
          _ ->
            list.index_map(found, fn(item, index) {
              option_view(key, item, index, chosen, active, handlers.on_choose)
            })
        }
    },
  )
}

fn option_view(
  key: String,
  item: Item,
  index: Int,
  chosen: String,
  active: Option(Int),
  on_choose: fn(String) -> msg,
) -> element.Element(msg) {
  let selected = item.id == chosen
  html.li(
    [
      attribute.id(option_id(key, index)),
      attribute.role("option"),
      attribute.attribute("aria-selected", bool_text(selected)),
      case active == Some(index) {
        True -> attribute.class("active")
        False -> attribute.none()
      },
      // The option handles its own press, so the list's handler stays out.
      event.advanced(
        "mousedown",
        decode.success(event.handler(
          on_choose(item.id),
          prevent_default: True,
          stop_propagation: True,
        )),
      ),
    ],
    [
      html.span([attribute.class("label")], [html.text(item.label)]),
      case item.hint {
        "" -> element.none()
        hint -> html.span([attribute.class("hint")], [html.text(hint)])
      },
    ],
  )
}

fn active_id(key: String, state: State, found: List(Item)) -> Option(String) {
  case state.open, active_item(found, state.active), state.active {
    True, Some(_), Some(index) -> Some(option_id(key, index))
    _, _, _ -> None
  }
}

/// The id of the list's option at `index`, which the input names as its
/// active option.
pub fn option_id(key: String, index: Int) -> String {
  key <> "-option-" <> int.to_string(index)
}
