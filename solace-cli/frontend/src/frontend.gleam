import contracts
import gleam/uri.{type Uri}
import lustre
import lustre/attribute
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import migration
import modem
import solace_board

pub fn main() -> Nil {
  let assert Ok(_) = solace_board.register()
  let assert Ok(_) = contracts.register()
  let assert Ok(_) = migration.register()
  let assert Ok(_) =
    lustre.application(init, update, view)
    |> lustre.start("#app", Nil)
  Nil
}

pub type Route {
  Board
  Migrate
  Contracts(base: String)
}

pub type Msg {
  RouteChanged(Route)
}

pub fn route(uri: Uri) -> Route {
  case uri.path_segments(uri.path) {
    [] -> Board
    ["migrate"] -> Migrate
    ["contracts"] -> Contracts("")
    ["p", dir, "contracts"] -> Contracts("/p/" <> dir)
    _ -> Board
  }
}

pub fn page(route: Route) -> Element(msg) {
  case route {
    Board -> element.element("solace-board", [], [])
    Migrate -> element.element("solace-migration", [], [])
    Contracts(base) -> {
      let attributes = case base {
        "" -> []
        _ -> [attribute.attribute("base", base)]
      }
      element.element("solace-contracts", attributes, [])
    }
  }
}

pub fn init(_flags: Nil) -> #(Route, Effect(Msg)) {
  let initial = case modem.initial_uri() {
    Ok(uri) -> route(uri)
    Error(_) -> Board
  }
  #(initial, modem.init(fn(uri) { RouteChanged(route(uri)) }))
}

pub fn update(_model: Route, msg: Msg) -> #(Route, Effect(Msg)) {
  let RouteChanged(route) = msg
  #(route, effect.none())
}

fn link(path: String, label: String, current: Bool) -> Element(msg) {
  let attributes = case current {
    True -> [
      attribute.href(path),
      attribute.class("current"),
      attribute.attribute("aria-current", "page"),
    ]
    False -> [attribute.href(path)]
  }
  html.a(attributes, [html.text(label)])
}

pub fn view(route: Route) -> Element(msg) {
  element.fragment([
    html.nav([attribute.class("menu")], [
      link("/", "Board", route == Board),
      link("/migrate", "Migration", route == Migrate),
      link("/contracts", "Contracts", case route {
        Contracts(_) -> True
        _ -> False
      }),
    ]),
    page(route),
  ])
}

/// Whether a drag starting at an element is stopped: only a box you can type in may be dragged from.
pub fn blocks_drag(in_text_box: Bool) -> Bool {
  !in_text_box
}

/// Whether a click must skip modem's in-page router so the browser keeps its new-tab behaviour:
/// a click with a modifier key held, on a menu link.
pub fn keeps_new_tab(
  ctrl ctrl: Bool,
  meta meta: Bool,
  shift shift: Bool,
  alt alt: Bool,
  on_menu_link on_menu_link: Bool,
) -> Bool {
  { ctrl || meta || shift || alt } && on_menu_link
}
