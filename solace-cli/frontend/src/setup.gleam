//// The first-run checklist: what this computer still needs, in plain words.

import gleam/dynamic/decode
import gleam/json
import gleam/list
import gleam/string
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event

pub type Item {
  Item(
    id: String,
    label: String,
    ok: Bool,
    why: String,
    steps: List(Step),
    /// Solace installs this tool itself, so the item offers **Install** (D-264).
    install: Bool,
    /// **Install** was pressed and hasn't answered yet.
    installing: Bool,
  )
}

/// The missing tools in a `GET /setup` reply, or nothing when the reply is not one.
pub fn decode(text: String) -> Result(List(Item), Nil) {
  case json.parse(text, decode.at(["items"], decode.list(item_decoder()))) {
    Ok(items) -> Ok(list.filter(items, fn(item) { !item.ok }))
    Error(_) -> Error(Nil)
  }
}

fn item_decoder() -> decode.Decoder(Item) {
  use id <- decode.field("id", decode.string)
  use label <- decode.field("label", decode.string)
  use ok <- decode.field("ok", decode.bool)
  use why <- decode.field("why", decode.string)
  use steps <- decode.field("steps", decode.list(step_decoder()))
  use install <- decode.optional_field("install", False, decode.bool)
  decode.success(Item(id, label, ok, why, steps, install, False))
}

/// The checklist, or nothing when every tool passes. A tool Solace installs
/// itself has an **Install** button, which `install` hears with its id.
pub fn view(
  items: List(Item),
  again: msg,
  install: fn(String) -> msg,
) -> Element(msg) {
  case items {
    [] ->
      html.div(
        [attribute.class("setup"), attribute.attribute("hidden", "")],
        [],
      )
    _ ->
      html.section(
        [
          attribute.class("setup"),
          attribute.attribute("aria-label", "Recommended setup"),
        ],
        [
          html.div([attribute.class("setup-head")], [
            html.h2([], [html.text("Recommended setup")]),
            html.button(
              [attribute.class("setup-again"), event.on_click(again)],
              [html.text("Check again")],
            ),
          ]),
          html.p([], [
            html.text(
              "These tools are highly recommended: each makes part of Solace work. Install any that are missing.",
            ),
          ]),
          html.ul(
            [],
            list.map(items, fn(item) {
              html.li([], [
                html.strong([], [html.text(item.label)]),
                html.text(". " <> item.why),
                html.div(
                  [attribute.class("setup-steps")],
                  list.map(item.steps, fn(step) {
                    case step.command {
                      True ->
                        html.div([attribute.class("setup-step")], [
                          html.code([], [html.text(step.text)]),
                        ])
                      False ->
                        html.div([attribute.class("setup-step")], [
                          html.text(step.text),
                        ])
                    }
                  }),
                ),
                install_button(item, install),
              ])
            }),
          ),
        ],
      )
  }
}

pub type Step {
  Step(text: String, command: Bool)
}

fn step_decoder() -> decode.Decoder(Step) {
  use text <- decode.field("text", decode.string)
  use command <- decode.field("command", decode.bool)
  decode.success(Step(text, command))
}

fn install_button(item: Item, install: fn(String) -> msg) -> Element(msg) {
  case item.install {
    False -> element.none()
    True ->
      html.div([attribute.class("setup-step")], [
        html.button(
          [
            attribute.disabled(item.installing),
            event.on_click(install(item.id)),
          ],
          [
            html.text(case item.installing {
              True -> "Installing…"
              False -> "Install"
            }),
          ],
        ),
      ])
  }
}

/// The checklist once **Install** is pressed for tool `id`.
pub fn installing(items: List(Item), id: String) -> List(Item) {
  list.map(items, fn(item) {
    case item.id == id {
      True -> Item(..item, installing: True)
      False -> item
    }
  })
}

/// The checklist once tool `id`'s install answered `reply`: the fresh
/// checklist it sends, or the item again with the reason it failed. Another
/// tool still installing stays busy.
pub fn installed(items: List(Item), id: String, reply: String) -> List(Item) {
  case decode(reply) {
    Ok(fresh) -> reloaded(list.filter(items, fn(item) { item.id != id }), fresh)
    Error(_) -> {
      let reason = json.parse(reply, decode.at(["message"], decode.string))
      list.map(items, fn(item) {
        case item.id == id, reason {
          True, Ok(message) ->
            Item(..item, why: install_failure(item, message), installing: False)
          True, Error(_) -> Item(..item, installing: False)
          False, _ -> item
        }
      })
    }
  }
}

/// Why tool `item` didn't install, from the refusal's `message`: an unknown
/// route means this version of Solace sent the request where nothing answers it.
fn install_failure(item: Item, message: String) -> String {
  case string.contains(message, "unknown route") {
    True ->
      "Solace couldn't install "
      <> item.label
      <> ": this version has a bug in its install request"
    False -> message
  }
}

/// The fresh checklist `fresh` in place of `items`: a tool still installing
/// in `items` stays busy until its own install answers.
pub fn reloaded(items: List(Item), fresh: List(Item)) -> List(Item) {
  let busy =
    list.filter_map(items, fn(item) {
      case item.installing {
        True -> Ok(item.id)
        False -> Error(Nil)
      }
    })
  list.map(fresh, fn(item) {
    Item(..item, installing: list.contains(busy, item.id))
  })
}
