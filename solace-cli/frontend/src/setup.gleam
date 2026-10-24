//// The first-run checklist: what this computer still needs, in plain words.

import gleam/dynamic/decode
import gleam/json
import gleam/list
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event

pub type Item {
  Item(id: String, label: String, ok: Bool, why: String, steps: List(Step))
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
  decode.success(Item(id, label, ok, why, steps))
}

/// The checklist, or nothing when every tool passes.
pub fn view(items: List(Item), again: msg) -> Element(msg) {
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
