//// The Settings page, `<solace-settings>`, and the store every page reads its
//// settings from. A setting lives in the browser under its key; a change
//// reaches whoever subscribed to that key, on this page or in another window.

import gleam/option.{type Option, None, Some}
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

/// The theme's key in the store: "light", "dark", or "" to follow the system.
/// Every `index.html` reads it before first paint.
pub const theme_key = "solace-theme"

/// A setting's stored value, "" when unset or when the browser keeps none.
@external(javascript, "./store.mjs", "load")
pub fn load(key: String) -> String

/// Stores a setting, "" to clear it, and tells every subscriber.
@external(javascript, "./store.mjs", "save")
pub fn save(key: String, value: String) -> Nil

/// Calls `handler` with each new value of a setting, changed on this page or
/// in another window; returns the function that unsubscribes.
@external(javascript, "./store.mjs", "subscribe")
pub fn subscribe(key: String, handler: fn(String) -> Nil) -> fn() -> Nil

/// The theme the page shows now, "" outside a browser.
@external(javascript, "./store.mjs", "theme")
fn theme() -> String

/// Shows the theme on the page, stores it and tells every subscriber.
@external(javascript, "./store.mjs", "set_theme")
pub fn set_theme(value: String) -> Nil

@external(javascript, "./store.mjs", "follow_theme")
fn follow_theme() -> Nil

pub type Model {
  Model(
    dark: Bool,
    /// Stops listening for theme changes; Some while the element is in the page.
    unsubscribe: Option(fn() -> Nil),
  )
}

pub type Msg {
  /// The Dark mode switch: on or off.
  SetDark(Bool)
  /// The theme setting changed, here or in another window; "" follows the system.
  ThemeChanged(String)
  /// The element was added to the page: it listens for theme changes.
  Connected
  /// Listening began; the function stops it.
  Subscribed(fn() -> Nil)
  /// The element left the page: it stops listening.
  Disconnected
}

/// Listening starts when the element connects (`Connected`), so an element
/// removed and added again listens once.
pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(Model(dark: theme() == "dark", unsubscribe: None), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    SetDark(dark) -> #(
      Model(..model, dark:),
      effect.from(fn(_) {
        set_theme(case dark {
          True -> "dark"
          False -> "light"
        })
      }),
    )
    // The page has applied the system's theme by now (`follow_theme`).
    ThemeChanged("") -> #(
      Model(..model, dark: theme() == "dark"),
      effect.none(),
    )
    ThemeChanged(value) -> #(
      Model(..model, dark: value == "dark"),
      effect.none(),
    )
    Connected ->
      case model.unsubscribe {
        Some(_) -> #(model, effect.none())
        // A change made while it was away shows now.
        None -> #(
          Model(..model, dark: theme() == "dark"),
          effect.from(fn(dispatch) {
            dispatch(
              Subscribed(
                subscribe(theme_key, fn(value) { dispatch(ThemeChanged(value)) }),
              ),
            )
          }),
        )
      }
    Subscribed(stop) -> #(
      Model(..model, unsubscribe: Some(stop)),
      effect.none(),
    )
    Disconnected ->
      case model.unsubscribe {
        None -> #(model, effect.none())
        Some(stop) -> #(
          Model(..model, unsubscribe: None),
          effect.from(fn(_) { stop() }),
        )
      }
  }
}

/// The Dark mode switch: a labelled checkbox, on when the page is dark.
pub fn theme_control(dark: Bool) -> Element(Msg) {
  html.label([attribute.class("switch")], [
    html.input([
      attribute.type_("checkbox"),
      attribute.role("switch"),
      attribute.checked(dark),
      event.on_check(SetDark),
    ]),
    html.text("Dark mode"),
  ])
}

pub fn view(model: Model) -> Element(Msg) {
  html.main([attribute.class("settings")], [
    html.style([], view_styles),
    html.section([attribute.attribute("aria-labelledby", "appearance")], [
      html.h2([attribute.id("appearance")], [html.text("Appearance")]),
      theme_control(model.dark),
    ]),
  ])
}

/// The light colour set; the dark one is the page's (`index.html`, and the
/// shell's for `/settings`).
const view_styles =
  solace_task.control_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6}
.settings{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:calc(100dvh - 56px);box-sizing:border-box;padding:var(--space-6);background:var(--sheet);color:var(--ink)}
.settings section{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}
.settings h2{margin:0 0 10px;font-size:19px}
.settings p{color:var(--soft);margin:8px 0}
.switch{display:flex;align-items:center;gap:10px;width:fit-content;cursor:pointer}
.switch input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}
.switch input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}
.switch input:checked{background:var(--focus)}.switch input:checked::before{transform:translateX(14px)}
.switch input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
@media (prefers-reduced-motion:reduce){.switch input,.switch input::before{transition:none}}
"

pub fn register() -> Result(Nil, lustre.Error) {
  // Every page that registers Settings follows a theme chosen in another window.
  follow_theme()
  lustre.register(
    lustre.component(init, update, view, [
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-settings",
  )
}
