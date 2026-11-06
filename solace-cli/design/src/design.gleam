import gleam/json
import gleam/list
import gleam/string
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import solace_task

/// Registers the element on its own, for `index.html`.
pub fn main() -> Nil {
  let assert Ok(_) = register()
  Nil
}

pub type Tab {
  Architecture
  UiUx
}

pub type Model {
  Model(tab: Tab)
}

pub type Msg {
  Select(Tab)
  /// The shell shows this tab: a load, Back or Forward.
  TabShown(Tab)
}

pub fn tabs() -> List(Tab) {
  [Architecture, UiUx]
}

/// The tab's name in the URL: `/design?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    Architecture -> "architecture"
    UiUx -> "ui-ux"
  }
}

pub fn label(tab: Tab) -> String {
  case tab {
    Architecture -> "Architecture"
    UiUx -> "UI/UX"
  }
}

/// The tab a URL query names ("?tab=ui-ux"); Architecture when it names
/// none or an unknown one.
pub fn tab_from(query: String) -> Tab {
  let named =
    query
    |> string.replace("?", "")
    |> string.split("&")
    |> list.find_map(fn(pair) {
      let name = string.remove_prefix(pair, "tab=")
      case name == pair {
        True -> Error(Nil)
        False -> Ok(name)
      }
    })
  case named {
    Ok(name) ->
      case list.find(tabs(), fn(tab) { tab_name(tab) == name }) {
        Ok(tab) -> tab
        Error(_) -> Architecture
      }
    Error(_) -> Architecture
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(Model(Architecture), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Select(tab) if tab == model.tab -> #(model, effect.none())
    Select(tab) -> #(Model(tab), event.emit("tab", json.string(tab_name(tab))))
    TabShown(tab) -> #(Model(tab), effect.none())
  }
}

pub fn view(model: Model) -> Element(Msg) {
  html.div([attribute.class("design")], [
    html.style([], view_styles),
    html.div(
      [
        attribute.class("tabs"),
        attribute.role("tablist"),
        attribute.attribute("aria-label", "Design"),
      ],
      list.map(tabs(), fn(tab) {
        let current = tab == model.tab
        html.button(
          [
            attribute.id("tab-" <> tab_name(tab)),
            attribute.role("tab"),
            attribute.attribute("aria-selected", case current {
              True -> "true"
              False -> "false"
            }),
            attribute.attribute("aria-controls", "panel-" <> tab_name(tab)),
            event.on_click(Select(tab)),
          ],
          [html.text(label(tab))],
        )
      }),
    ),
    ..list.map(tabs(), fn(tab) {
      html.section(
        [
          attribute.id("panel-" <> tab_name(tab)),
          attribute.role("tabpanel"),
          attribute.attribute("aria-labelledby", "tab-" <> tab_name(tab)),
          attribute.hidden(tab != model.tab),
        ],
        [],
      )
    })
  ])
}

const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec}
.design{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;--shell-height:56px;--tabs-height:44px;--view-height:calc(100dvh - var(--shell-height));height:var(--view-height);overflow:hidden;background:var(--sheet);color:var(--ink);display:flex;flex-direction:column}
.design>[role=tabpanel]{flex:1;min-height:0;overflow:auto;padding:var(--space-6)}
.design>[role=tabpanel][hidden]{display:none}
@media (max-width:900px){.design{display:block;height:auto;overflow:visible}.design>[role=tabpanel]{overflow:visible}}
"

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("tab", fn(name) {
        Ok(TabShown(tab_from("?tab=" <> name)))
      }),
      component.adopt_styles(False),
    ]),
    "solace-design",
  )
}
