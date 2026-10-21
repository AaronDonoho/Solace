import gleam/dynamic/decode
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/uri
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
    /// The models set up on this computer, from the system settings.
    models: List(Entry),
    /// The languages on this computer, from the system settings.
    languages: List(Language),
    /// The model being added or edited.
    form: Form,
    /// Why the last change was refused, "" when it wasn't.
    error: String,
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
  /// The server's model list, or why it refused a change.
  ModelsLoaded(String)
  /// The server's language list, or why it refused a change.
  LanguagesLoaded(String)
  /// Turns a language on or off in the system settings.
  ToggleLanguage(String, Bool)
  /// Opens a model in the form.
  EditModel(Entry)
  /// Empties the form for a new model.
  NewModel
  /// A form field, by name, and its new value.
  Field(String, String)
  SaveModel
  DeleteModel(Entry)
}

/// Listening starts when the element connects (`Connected`), so an element
/// removed and added again listens once.
pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(initial(theme() == "dark"), effect.none())
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
          effect.batch([
            effect.from(fn(dispatch) {
              dispatch(
                Subscribed(
                  subscribe(theme_key, fn(value) {
                    dispatch(ThemeChanged(value))
                  }),
                ),
              )
            }),
            call("/models", "GET", ""),
            call_languages("GET", ""),
          ]),
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
    // A saved or deleted model leaves an empty form, so a pasted key is gone.
    ModelsLoaded(text) ->
      case json.parse(text, entries_decoder()) {
        Ok(models) -> #(
          Model(..model, models:, form: blank_form, error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    LanguagesLoaded(text) ->
      case json.parse(text, languages_decoder()) {
        Ok(languages) -> #(Model(..model, languages:, error: ""), effect.none())
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    ToggleLanguage(id, enabled) -> #(
      model,
      call_languages("POST", language_body(id, enabled)),
    )
    EditModel(e) -> #(
      Model(
        ..model,
        form: Form(e.id, e.label, e.agent, e.model, e.base_url, "", e.has_key),
        error: "",
      ),
      effect.none(),
    )
    NewModel -> #(Model(..model, form: blank_form, error: ""), effect.none())
    Field(name, value) -> {
      let f = model.form
      let form = case name {
        "label" -> Form(..f, label: value)
        "agent" -> Form(..f, agent: value)
        "model" -> Form(..f, model: value)
        "base_url" -> Form(..f, base_url: value)
        "key" -> Form(..f, key: value)
        _ -> f
      }
      #(Model(..model, form:), effect.none())
    }
    SaveModel -> {
      let f = model.form
      let fields = [
        #("label", json.string(f.label)),
        #("agent", json.string(f.agent)),
        #("model", json.string(f.model)),
        #("base_url", json.string(f.base_url)),
        #("key", json.string(f.key)),
      ]
      let fields = case f.id {
        "" -> fields
        id -> [#("id", json.string(id)), ..fields]
      }
      #(model, call("/models", "POST", json.to_string(json.object(fields))))
    }
    DeleteModel(e) -> #(
      model,
      effect.from(fn(send) {
        case
          confirm(
            "Delete model \""
            <> e.label
            <> "\" and its saved key? Tasks can no longer start it.",
          )
        {
          True ->
            request(
              "/models/" <> uri.percent_encode(e.id),
              "DELETE",
              "",
              fn(text) { send(ModelsLoaded(text)) },
            )
          False -> Nil
        }
      }),
    )
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
    languages_section(model),
    models_section(model),
    case model.error {
      "" -> element.none()
      e ->
        html.p([attribute.class("error"), attribute.role("alert")], [
          html.text(e),
        ])
    },
  ])
}

/// The light colour set; the dark one is the page's (`index.html`, and the
/// shell's for `/settings`).
const view_styles =
  solace_task.control_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6}
.settings{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:calc(100dvh - 56px);box-sizing:border-box;padding:var(--space-6);background:var(--sheet);color:var(--ink)}
.settings section{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}
.settings h2{margin:0 0 10px;font-size:var(--h2)}
.settings p{color:var(--soft);margin:8px 0}
.settings section+section{margin-top:var(--space-6)}
.settings h3{margin:16px 0 8px;font-size:inherit}
.models{margin:0;padding:0 0 0 18px}.models li{margin:6px 0}.models button{margin-right:8px}
.languages{margin:0;padding:0;list-style:none}.languages li{margin:6px 0}
.field{display:grid;gap:4px;margin:8px 0;max-width:480px}
.settings .error{color:var(--ink);font-weight:600}
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

/// A model set up in the system settings, as `GET /models` lists it.
pub type Entry {
  Entry(
    id: String,
    label: String,
    agent: String,
    model: String,
    base_url: String,
    /// Whether Solace keeps a key for it; the key itself never reaches the page.
    has_key: Bool,
    /// Why it can't start, "" when it can.
    reason: String,
  )
}

/// The model form; `id` is "" for a new model, and `key` is "" to keep the stored one.
pub type Form {
  Form(
    id: String,
    label: String,
    agent: String,
    model: String,
    base_url: String,
    key: String,
    has_key: Bool,
  )
}

pub const blank_form = Form("", "", "codex", "", "", "", False)

/// The page before it hears from anywhere.
pub fn initial(dark: Bool) -> Model {
  Model(
    dark:,
    unsubscribe: None,
    models: [],
    languages: [],
    form: blank_form,
    error: "",
  )
}

@external(javascript, "./api.mjs", "request")
fn request(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "confirm")
fn confirm(text: String) -> Bool

/// A model route; its reply is the model list, or a refusal.
fn call(path: String, method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request(path, method, body, fn(text) { send(ModelsLoaded(text)) })
  })
}

/// The configured models in a `GET /models` reply; built-in and discovered
/// ones are the agents' own and aren't set up here.
pub fn entries_decoder() -> decode.Decoder(List(Entry)) {
  let text = decode.one_of(decode.string, [decode.success("")])
  decode.list({
    use id <- decode.field("id", decode.string)
    use label <- decode.field("label", decode.string)
    use agent <- decode.optional_field("agent", "", text)
    use model <- decode.optional_field("model", "", text)
    use base_url <- decode.optional_field("base_url", "", text)
    use has_key <- decode.optional_field("has_key", False, decode.bool)
    use reason <- decode.optional_field("reason", "", text)
    use configured <- decode.optional_field("configured", False, decode.bool)
    decode.success(#(
      configured,
      Entry(id:, label:, agent:, model:, base_url:, has_key:, reason:),
    ))
  })
  |> decode.map(
    list.filter_map(_, fn(e: #(Bool, Entry)) {
      case e.0 {
        True -> Ok(e.1)
        False -> Error(Nil)
      }
    }),
  )
}

fn refusal(text: String) -> String {
  json.parse(
    text,
    decode.one_of(decode.at(["message"], decode.string), [
      decode.at(["error"], decode.string),
    ]),
  )
  |> result.unwrap("Solace could not be reached")
}

/// The models set up here, each with why it can't start, and the form.
pub fn models_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "models")], [
    html.h2([attribute.id("models")], [html.text("Models")]),
    html.p([], [
      html.text(
        "A model on its own endpoint, with its API key. Solace keeps the key in a file only you can read, and never shows it again.",
      ),
    ]),
    html.ul([attribute.class("models")], list.map(model.models, model_row)),
    model_form(model.form),
  ])
}

fn model_row(e: Entry) -> Element(Msg) {
  html.li([], [
    html.strong([], [html.text(e.label)]),
    html.text(" " <> e.model),
    case e.reason {
      "" -> element.none()
      r -> html.p([attribute.class("reason")], [html.text(r)])
    },
    html.button([attribute.type_("button"), event.on_click(EditModel(e))], [
      html.text("Edit"),
    ]),
    html.button([attribute.type_("button"), event.on_click(DeleteModel(e))], [
      html.text("Delete"),
    ]),
  ])
}

fn text_field(
  label: String,
  name: String,
  value: String,
  kind: String,
) -> Element(Msg) {
  html.label([attribute.class("field")], [
    html.text(label),
    html.input([
      attribute.type_(kind),
      attribute.name(name),
      attribute.value(value),
      event.on_input(Field(name, _)),
    ]),
  ])
}

/// The key field starts empty: a saved key is never sent back to the page.
pub fn model_form(f: Form) -> Element(Msg) {
  html.form(
    [attribute.class("model-form"), event.on_submit(fn(_) { SaveModel })],
    [
      html.h3([], [
        html.text(case f.id {
          "" -> "Add a model"
          _ -> "Edit " <> f.label
        }),
      ]),
      text_field("Label", "label", f.label, "text"),
      html.label([attribute.class("field")], [
        html.text("Agent"),
        html.select(
          [attribute.name("agent"), event.on_change(Field("agent", _))],
          [
            html.option(
              [attribute.value("codex"), attribute.selected(f.agent == "codex")],
              "Codex (OpenAI Responses API)",
            ),
            html.option(
              [
                attribute.value("claude"),
                attribute.selected(f.agent == "claude"),
              ],
              "Claude Code (Anthropic API)",
            ),
          ],
        ),
      ]),
      text_field("Model id", "model", f.model, "text"),
      text_field("Endpoint base URL", "base_url", f.base_url, "url"),
      html.label([attribute.class("field")], [
        html.text("API key"),
        html.input([
          attribute.type_("password"),
          attribute.name("key"),
          attribute.autocomplete("off"),
          attribute.value(f.key),
          attribute.placeholder(case f.has_key {
            True -> "Key saved; leave blank to keep it"
            False -> "Paste the key"
          }),
          event.on_input(Field("key", _)),
        ]),
      ]),
      html.button([attribute.type_("submit")], [html.text("Save model")]),
      case f.id {
        "" -> element.none()
        _ ->
          html.button([attribute.type_("button"), event.on_click(NewModel)], [
            html.text("Cancel"),
          ])
      },
    ],
  )
}

/// A language Solace can use on this computer, as `GET /languages` lists it.
pub type Language {
  Language(id: String, label: String, enabled: Bool, installed: Bool)
}

/// The languages in a `GET /languages` reply.
pub fn languages_decoder() -> decode.Decoder(List(Language)) {
  decode.list({
    use id <- decode.field("id", decode.string)
    use label <- decode.field("label", decode.string)
    use enabled <- decode.field("enabled", decode.bool)
    use installed <- decode.field("installed", decode.bool)
    decode.success(Language(id:, label:, enabled:, installed:))
  })
}

/// The languages route; its reply is the language list, or a refusal.
fn call_languages(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/languages", method, body, fn(text) { send(LanguagesLoaded(text)) })
  })
}

fn language_body(id: String, enabled: Bool) -> String {
  json.to_string(
    json.object([
      #("id", json.string(id)),
      #("enabled", json.bool(enabled)),
    ]),
  )
}

/// The languages on this computer, each with its switch and missing state.
pub fn languages_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "languages")], [
    html.h2([attribute.id("languages")], [html.text("Languages")]),
    html.p([], [
      html.text(
        "Turn a language off on this computer, even when its toolchain is installed. Its projects stay readable, but changes, checks and tests are refused until it is turned back on.",
      ),
    ]),
    html.ul(
      [attribute.class("languages")],
      list.map(model.languages, language_row),
    ),
  ])
}

fn language_row(l: Language) -> Element(Msg) {
  html.li([], [
    html.label([attribute.class("switch")], [
      html.input([
        attribute.type_("checkbox"),
        attribute.role("switch"),
        attribute.checked(l.enabled),
        event.on_check(fn(enabled) { ToggleLanguage(l.id, enabled) }),
      ]),
      html.text(l.label),
    ]),
    case l.installed {
      True -> element.none()
      False ->
        html.p([attribute.class("reason")], [
          html.text(
            "Not found on this computer; its projects say its toolchain is missing.",
          ),
        ])
    },
  ])
}
