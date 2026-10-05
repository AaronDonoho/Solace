import gleam/uri.{type Uri}
import lustre
import lustre/attribute
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import migration
import modem
import solace_board
import solace_view

pub fn main() -> Nil {
  let assert Ok(_) = solace_board.register()
  let assert Ok(_) = solace_view.register()
  let assert Ok(_) = migration.register()
  let assert Ok(_) =
    lustre.application(init, update, view)
    |> lustre.start("#app", Nil)
  Nil
}

pub type Route {
  Board
  Migrate
  View(base: String)
}

type Msg {
  RouteChanged(Route)
}

pub fn route(uri: Uri) -> Route {
  case uri.path_segments(uri.path) {
    [] -> Board
    ["migrate"] -> Migrate
    ["view"] -> View("")
    ["p", dir, "view"] -> View("/p/" <> dir)
    _ -> Board
  }
}

pub fn page(route: Route) -> Element(msg) {
  case route {
    Board -> element.element("solace-board", [], [])
    Migrate -> element.element("solace-migration", [], [])
    View(base) -> {
      let attributes = case base {
        "" -> []
        _ -> [attribute.attribute("base", base)]
      }
      element.element("solace-view", attributes, [])
    }
  }
}

fn init(_flags: Nil) -> #(Route, Effect(Msg)) {
  let initial = case modem.initial_uri() {
    Ok(uri) -> route(uri)
    Error(_) -> Board
  }
  #(initial, modem.init(fn(uri) { RouteChanged(route(uri)) }))
}

fn update(_model: Route, msg: Msg) -> #(Route, Effect(Msg)) {
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
    ]),
    page(route),
  ])
}
