import gleam/dynamic/decode
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import gleam/uri
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import skill_writes
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
    /// The tab shown.
    tab: Tab,
    /// Stops listening for theme changes; Some while the element is in the page.
    unsubscribe: Option(fn() -> Nil),
    /// The models this computer can pick, from the system settings.
    models: List(Entry),
    /// The languages on this computer, from the system settings.
    languages: List(Language),
    /// The version of the Solace program running this page.
    version: String,
    /// How agents address the human, how much they write, and how readily they ask.
    behavior: Behavior,
    /// The saved choices while a change is in flight, so a refusal can restore them.
    behavior_before: Option(Behavior),
    /// The choices whose save is still in flight, in the order they were made.
    behavior_pending: List(#(Setting, String)),
    /// The skills request waiting for its reply.
    skills_writing: Option(skill_writes.SkillWrite),
    /// The skills requests queued until the one in flight finishes.
    skills_waiting: List(skill_writes.SkillWrite),
    /// The newest server state whose reply was applied.
    skills_server: List(Skill),
    /// The skills in this workspace, with ticks still waiting shown on top.
    skills: List(Skill),
    /// The agent roles whose skills Settings shows.
    roles: List(Role),
    /// The model chosen for each role.
    assignments: List(Assignment),
    /// The skill being added or edited.
    skill_form: SkillForm,
    /// The model being added or edited.
    form: Form,
    /// Why the last change was refused, "" when it wasn't.
    error: String,
  )
}

pub type Msg {
  /// The tab chosen on the page.
  Select(Tab)
  /// The shell's `tab` attribute: the tab the address names.
  TabShown(Tab)
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
  /// The running program's version, or why it refused.
  AboutLoaded(String)
  /// The server's agent behavior choices, or why it refused a change.
  BehaviorLoaded(String)
  /// The server's roles, or why it refused.
  RolesLoaded(String)
  /// The server's model for each role, or why it refused.
  RoleModelsLoaded(String)
  /// The server's skills, or why it refused a change.
  SkillsLoaded(String)
  /// The server's skills after a save or delete; an empty form follows.
  SkillsSaved(String)
  /// The server's skills after a role-card tick, or why it was refused.
  SkillTickSaved(String)
  /// Opens a skill in the form.
  EditSkill(Skill)
  /// Empties the form for a new skill.
  NewSkill
  /// A skill form field, by name, and its new value.
  SkillField(String, String)
  SaveSkill
  /// Adds or removes one role on a saved skill.
  AssignSkill(String, String, Bool)
  /// Removes a skill file.
  DeleteSkill(String)
  /// Empties the skill form.
  CancelSkill
  /// Turns a language on or off in the system settings.
  ToggleLanguage(String, Bool)
  /// Saves one agent behavior choice in the system settings.
  ChooseBehavior(Setting, String)
  /// Saves one role's model.
  ChooseRoleModel(String, String)
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
    Select(tab) if tab == model.tab -> #(model, effect.none())
    Select(tab) -> #(
      Model(..model, tab:),
      event.emit("tab", json.string(tab_name(tab))),
    )
    TabShown(tab) -> #(Model(..model, tab:), effect.none())
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
        None -> {
          let #(model, skills_effect) =
            queue_skill_load(Model(..model, dark: theme() == "dark"))
          #(
            model,
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
              call_about(),
              call_behavior("GET", ""),
              call_roles("GET", ""),
              call_role_models("GET", ""),
              skills_effect,
            ]),
          )
        }
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
    RolesLoaded(text) ->
      case json.parse(text, roles_decoder()) {
        Ok(roles) -> #(Model(..model, roles:, error: ""), effect.none())
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    RoleModelsLoaded(text) ->
      case json.parse(text, assignments_decoder()) {
        Ok(assignments) -> #(
          Model(..model, assignments:, error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    SkillsLoaded(text) ->
      case model.skills_writing {
        Some(skill_writes.LoadWrite) -> skills_loaded(model, text)
        _ -> #(model, effect.none())
      }
    SkillsSaved(text) ->
      case model.skills_writing {
        Some(skill_writes.SaveWrite(_)) -> skills_saved(model, text)
        Some(skill_writes.DeleteWrite(_)) -> skills_saved(model, text)
        _ -> #(model, effect.none())
      }
    SkillTickSaved(text) ->
      case model.skills_writing {
        Some(skill_writes.TickWrite(_)) -> skills_loaded(model, text)
        _ -> #(model, effect.none())
      }
    EditSkill(skill) -> #(
      Model(
        ..model,
        skill_form: SkillForm(
          skill.name,
          skill.name,
          skill.description,
          skill.body,
        ),
        error: "",
      ),
      effect.none(),
    )
    NewSkill -> #(
      Model(..model, skill_form: blank_skill_form, error: ""),
      effect.none(),
    )
    SkillField(name, value) -> {
      let f = model.skill_form
      let skill_form = case name {
        "name" -> SkillForm(..f, name: value)
        "description" -> SkillForm(..f, description: value)
        "body" -> SkillForm(..f, body: value)
        _ -> f
      }
      #(Model(..model, skill_form:), effect.none())
    }
    SaveSkill ->
      start_skill_write(model, skill_writes.SaveWrite(skill_save_body(model)))
    AssignSkill(name, role, chosen) -> assign_skill(model, name, role, chosen)
    DeleteSkill(name) ->
      start_skill_write(model, skill_writes.DeleteWrite(name))
    CancelSkill -> #(
      Model(..model, skill_form: blank_skill_form, error: ""),
      effect.none(),
    )
    LanguagesLoaded(text) ->
      case json.parse(text, languages_decoder()) {
        Ok(languages) -> #(Model(..model, languages:, error: ""), effect.none())
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    AboutLoaded(text) ->
      case json.parse(text, version_decoder()) {
        Ok(version) -> #(Model(..model, version:, error: ""), effect.none())
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    BehaviorLoaded(text) ->
      case json.parse(text, behavior_decoder()) {
        Ok(loaded) -> {
          let #(behavior, pending) =
            behavior_after_reply(model.behavior, loaded, model.behavior_pending)
          #(
            Model(
              ..model,
              behavior:,
              behavior_before: case pending {
                [] -> None
                _ -> model.behavior_before
              },
              behavior_pending: pending,
              error: "",
            ),
            effect.none(),
          )
        }
        Error(_) -> {
          let behavior = case model.behavior_before {
            Some(before) -> before
            None -> model.behavior
          }
          #(
            Model(
              ..model,
              behavior:,
              behavior_before: None,
              behavior_pending: [],
              error: refusal(text),
            ),
            effect.none(),
          )
        }
      }
    ChooseBehavior(setting, value) -> {
      let before = case model.behavior_before {
        Some(before) -> before
        None -> model.behavior
      }
      #(
        Model(
          ..model,
          behavior: behavior_with(model.behavior, setting, value),
          behavior_before: Some(before),
          behavior_pending: [#(setting, value), ..model.behavior_pending],
        ),
        call_behavior("POST", behavior_body(setting_key(setting), value)),
      )
    }
    ChooseRoleModel(role, choice) -> #(
      model,
      call_role_models("POST", role_model_body(model.assignments, role, choice)),
    )
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
  let panels =
    list.map(tabs(), fn(tab) {
      html.div(
        [
          attribute.id("panel-" <> tab_name(tab)),
          attribute.role("tabpanel"),
          attribute.attribute("aria-labelledby", "tab-" <> tab_name(tab)),
          attribute.hidden(tab != model.tab),
        ],
        case tab {
          Agents -> [
            behavior_section(model),
            role_models_section(model),
            models_section(model),
            skills_section(model),
          ]
          Appearance -> [appearance_section(model.dark)]
          Languages -> [languages_section(model)]
          About -> [about_section(model)]
        },
      )
    })
  html.main(
    [attribute.class("settings")],
    list.flatten([
      [
        html.style([], view_styles),
        html.div(
          [
            attribute.class("tabs"),
            attribute.role("tablist"),
            attribute.attribute("aria-label", "Settings"),
          ],
          list.map(tabs(), fn(tab) { tab_button(tab, model.tab) }),
        ),
      ],
      panels,
      [
        case model.error {
          "" -> element.none()
          e ->
            html.p([attribute.class("error"), attribute.role("alert")], [
              html.text(e),
            ])
        },
      ],
    ]),
  )
}

/// The light colour set; the dark one is the page's (`index.html`, and the
/// shell's for `/settings`).
const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6}
.settings{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:calc(100dvh - 56px);box-sizing:border-box;background:var(--sheet);color:var(--ink);--tabs-height:44px;--tab-pad:0 18px}
.settings>[role=tabpanel]{padding:var(--space-6)}
.settings>[role=tabpanel][hidden]{display:none}
.settings section{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}
.settings h2{margin:0 0 10px;font-size:var(--h2)}
.settings p{color:var(--soft);margin:8px 0}
.settings section+section{margin-top:var(--space-6)}
.settings h3{margin:16px 0 8px;font-size:inherit}
.about-license{margin-top:12px}.about-license>summary{cursor:pointer}.about-license .md h4{margin:16px 0 8px;font-size:var(--h4)}.about-license .md p,.about-license .md li{color:var(--ink)}.about-license .md ul{margin:8px 0;padding-left:20px}
.models{margin:0;padding:0 0 0 18px}.models li{margin:6px 0}.models button{margin-right:8px}
.languages{margin:0;padding:0;list-style:none}.languages li{margin:6px 0}
.behavior-groups{margin-top:8px}
.behavior-group{margin:14px 0}
.choice-pick{display:flex;flex-wrap:wrap;gap:0;width:max-content;max-width:100%;margin:6px 0}
.choice-stop{position:relative;flex:none;margin:0;cursor:pointer}
.choice-stop+.choice-stop{margin-left:-1px}
.choice-stop input{position:absolute;width:1px;height:1px;margin:0;opacity:0;pointer-events:none}
.choice-stop .choice-label{display:flex;align-items:center;justify-content:center;box-sizing:border-box;min-height:var(--control-h);padding:0 18px;border:1px solid var(--edge);background:var(--paper);font-size:15px;white-space:nowrap}
.choice-stop:hover .choice-label{background:var(--hover)}
.choice-stop input:checked+.choice-label{background:var(--ink);border-color:var(--ink);color:var(--paper);font-weight:600}
.choice-stop:hover input:checked+.choice-label{background:var(--hover);color:var(--ink)}
.choice-stop input:focus-visible+.choice-label{outline:3px solid var(--focus);outline-offset:2px}
.choice-help{display:none;position:absolute;left:0;top:calc(100% + 6px);z-index:2;width:max-content;max-width:min(18rem,80vw);padding:6px 10px;border:1px solid var(--edge);background:var(--paper);color:var(--ink);font-size:13px;font-weight:400;line-height:1.4;white-space:normal}
.choice-stop:last-child .choice-help{left:auto;right:0}
.choice-stop:hover .choice-help,.choice-stop:focus-within .choice-help{display:block}
.role-models{margin-top:8px}.role-model{margin:14px 0}.role-model-picks{display:grid;gap:8px;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));max-width:640px}
.role-model .field{max-width:none}
.field{display:grid;gap:4px;margin:8px 0;max-width:480px}
.settings .error{color:var(--ink);font-weight:600;padding:0 var(--space-6) var(--space-6)}
.switch{display:flex;align-items:center;gap:10px;width:fit-content;cursor:pointer}
.switch input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}
.switch input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}
.switch input:checked{background:var(--focus)}.switch input:checked::before{transform:translateX(14px)}
.switch input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.skills{margin:0;padding:0 0 0 18px}.skills li{margin:6px 0}.skills button{margin-left:8px}
.skill-roles{display:flex;flex-wrap:wrap;gap:12px;margin:8px 0}
.skill-body{min-height:8rem;font:inherit}
@media (prefers-reduced-motion:reduce){.switch input,.switch input::before{transition:none}}
"

pub fn register() -> Result(Nil, lustre.Error) {
  // Every page that registers Settings follows a theme chosen in another window.
  follow_theme()
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("tab", fn(name) {
        Ok(TabShown(tab_from("?tab=" <> name)))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-settings",
  )
}

/// A model `GET /models` lists: one set up in the system settings, a
/// built-in one, or one discovered from an agent.
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
    /// Whether the human set it up under Models.
    configured: Bool,
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
    tab: Agents,
    unsubscribe: None,
    models: [],
    languages: [],
    version: "",
    behavior: default_behavior,
    behavior_before: None,
    behavior_pending: [],
    skills_writing: None,
    skills_waiting: [],
    skills_server: [],
    skills: [],
    roles: [],
    assignments: [],
    skill_form: blank_skill_form,
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

/// The models in a `GET /models` reply: set-up, built-in and discovered ones.
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
    decode.success(Entry(
      id:,
      label:,
      agent:,
      model:,
      base_url:,
      has_key:,
      reason:,
      configured:,
    ))
  })
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
    html.ul(
      [attribute.class("models")],
      list.map(list.filter(model.models, fn(e) { e.configured }), model_row),
    ),
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
    html.h2([attribute.id("languages")], [
      html.text("Enabled programming languages"),
    ]),
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

/// One of the three agent behavior choices.
pub type Setting {
  Involvement
  Detail
  Asking
}

/// One option of an agent behavior setting.
pub type Choice {
  Choice(value: String, label: String, help: String)
}

/// The machine-wide choices that shape how agents work with the human.
pub type Behavior {
  Behavior(involvement: String, detail: String, asking: String)
}

pub const default_behavior =
  Behavior(involvement: "collaborator", detail: "standard", asking: "balanced")

/// How agents talk with the human, how much they write, and how readily they ask.
pub fn behavior_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "behavior")], [
    html.h2([attribute.id("behavior")], [html.text("Agent behavior")]),
    html.p([], [
      html.text(
        "How agents talk with you and when they ask. These choices apply on this computer.",
      ),
    ]),
    html.div(
      [attribute.class("behavior-groups")],
      list.map([Involvement, Detail, Asking], fn(setting) {
        behavior_group(model.behavior, setting)
      }),
    ),
  ])
}

fn setting_title(setting: Setting) -> String {
  case setting {
    Involvement -> "Involvement"
    Detail -> "Detail"
    Asking -> "When to ask"
  }
}

fn setting_key(setting: Setting) -> String {
  case setting {
    Involvement -> "involvement"
    Detail -> "detail"
    Asking -> "asking"
  }
}

fn setting_choices(setting: Setting) -> List(Choice) {
  case setting {
    Involvement -> [
      Choice(
        "director",
        "Director",
        "Plain words, no code, and product questions only.",
      ),
      Choice(
        "collaborator",
        "Collaborator",
        "Key trade-offs, explained simply.",
      ),
      Choice("engineer", "Engineer", "Full technical detail, code included."),
    ]
    Detail -> [
      Choice("concise", "Concise", "Only what you need to decide or act."),
      Choice("standard", "Standard", "The usual amount of explanation."),
      Choice(
        "thorough",
        "Thorough",
        "The full reasoning and every relevant detail.",
      ),
    ]
    Asking -> [
      Choice("often", "Often", "Ask whenever unsure, even on small choices."),
      Choice("balanced", "Balanced", "Ask when a choice changes what you get."),
      Choice("rarely", "Rarely", "Decide unless the choice is hard to undo."),
    ]
  }
}

fn behavior_value(behavior: Behavior, setting: Setting) -> String {
  case setting {
    Involvement -> behavior.involvement
    Detail -> behavior.detail
    Asking -> behavior.asking
  }
}

fn behavior_group(behavior: Behavior, setting: Setting) -> Element(Msg) {
  let title = setting_title(setting)
  html.div([attribute.class("behavior-group")], [
    html.h3([], [html.text(title)]),
    html.div(
      [
        attribute.class("choice-pick"),
        attribute.attribute("role", "group"),
        attribute.attribute("aria-label", title),
      ],
      list.map(setting_choices(setting), fn(choice) {
        choice_stop(
          setting,
          choice,
          choice.value == behavior_value(behavior, setting),
        )
      }),
    ),
  ])
}

fn choice_stop(setting: Setting, choice: Choice, chosen: Bool) -> Element(Msg) {
  html.label([attribute.class("choice-stop")], [
    html.input([
      attribute.type_("radio"),
      attribute.name(setting_key(setting)),
      attribute.value(choice.value),
      attribute.checked(chosen),
      event.on_input(fn(value) { ChooseBehavior(setting, value) }),
    ]),
    html.span([attribute.class("choice-label")], [html.text(choice.label)]),
    html.span(
      [attribute.class("choice-help"), attribute.attribute("role", "tooltip")],
      [html.text(choice.help)],
    ),
  ])
}

/// The three agent behavior choices in a `GET /agent-settings` reply.
pub fn behavior_decoder() -> decode.Decoder(Behavior) {
  use involvement <- decode.field("involvement", decode.string)
  use detail <- decode.field("detail", decode.string)
  use asking <- decode.field("asking", decode.string)
  decode.success(Behavior(involvement:, detail:, asking:))
}

/// The agent behavior route; its reply is the new choices, or a refusal.
fn call_behavior(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/agent-settings", method, body, fn(text) {
      send(BehaviorLoaded(text))
    })
  })
}

fn behavior_body(key: String, value: String) -> String {
  json.to_string(
    json.object([
      #("key", json.string(key)),
      #("value", json.string(value)),
    ]),
  )
}

fn behavior_with(
  behavior: Behavior,
  setting: Setting,
  value: String,
) -> Behavior {
  case setting {
    Involvement -> Behavior(..behavior, involvement: value)
    Detail -> Behavior(..behavior, detail: value)
    Asking -> Behavior(..behavior, asking: value)
  }
}

/// The state after a reply: choices the server hasn't confirmed yet keep the
/// page's value, so a reply for another save can't undo them.
fn behavior_after_reply(
  local: Behavior,
  loaded: Behavior,
  pending: List(#(Setting, String)),
) -> #(Behavior, List(#(Setting, String))) {
  let unconfirmed =
    list.filter(pending, fn(choice) {
      let #(setting, value) = choice
      behavior_value(loaded, setting) != value
    })
  let behavior =
    list.fold(unconfirmed, loaded, fn(behavior, choice) {
      let #(setting, _) = choice
      behavior_with(behavior, setting, behavior_value(local, setting))
    })
  #(behavior, unconfirmed)
}

/// An agent role whose skills Settings shows.
pub type Role {
  Role(id: String, name: String, purpose: String)
}

/// A skill in this workspace.
pub type Skill {
  Skill(
    name: String,
    description: String,
    body: String,
    roles: List(String),
    /// Whether Settings may edit or delete it.
    managed: Bool,
    /// Whether the file has no `roles` key, so every role receives it.
    all_roles: Bool,
  )
}

/// The skill form; `original` is "" for a new skill.
pub type SkillForm {
  SkillForm(original: String, name: String, description: String, body: String)
}

pub const blank_skill_form = SkillForm("", "", "", "")

/// The roles in a `GET /roles` reply.
pub fn roles_decoder() -> decode.Decoder(List(Role)) {
  decode.list({
    use id <- decode.field("id", decode.string)
    use name <- decode.field("name", decode.string)
    use purpose <- decode.field("purpose", decode.string)
    decode.success(Role(id:, name:, purpose:))
  })
}

/// The skills in a `GET /skills` reply.
pub fn skills_decoder() -> decode.Decoder(List(Skill)) {
  decode.list({
    use name <- decode.field("name", decode.string)
    use description <- decode.optional_field("description", "", decode.string)
    use body <- decode.optional_field("body", "", decode.string)
    use roles <- decode.optional_field("roles", [], decode.list(decode.string))
    use managed <- decode.optional_field("managed", False, decode.bool)
    use all_roles <- decode.optional_field("all_roles", False, decode.bool)
    decode.success(Skill(
      name:,
      description:,
      body:,
      roles:,
      managed:,
      all_roles:,
    ))
  })
}

fn call_roles(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/roles", method, body, fn(text) { send(RolesLoaded(text)) })
  })
}

/// The skills route; its reply is the managed skills, or a refusal.
fn call_skills(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/skills", method, body, fn(text) { send(SkillsLoaded(text)) })
  })
}

fn skill_body(f: SkillForm, roles: List(String)) -> String {
  json.to_string(
    json.object([
      #("original", json.string(f.original)),
      #("name", json.string(f.name)),
      #("description", json.string(f.description)),
      #("body", json.string(f.body)),
      #("roles", json.array(roles, of: json.string)),
    ]),
  )
}

/// The workspace's skills, with a form to add or edit one.
pub fn skills_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "skills")], [
    html.h2([attribute.id("skills")], [html.text("Skills")]),
    html.p([], [
      html.text(
        "Instructions for agents. A skill saved here applies to every project in this workspace.",
      ),
    ]),
    case model.skills {
      [] -> html.p([], [html.text("No skills yet.")])
      skills ->
        html.ul([attribute.class("skills")], list.map(skills, skill_row))
    },
    html.button([attribute.type_("button"), event.on_click(NewSkill)], [
      html.text("Add a skill"),
    ]),
    skill_form(model.skill_form),
  ])
}

fn role_skills(model: Model, role: Role) -> Element(Msg) {
  case model.skills {
    [] -> element.none()
    skills ->
      html.div(
        [attribute.class("skill-roles")],
        list.map(skills, fn(skill) { skill_role_choice(skill, role) }),
      )
  }
}

fn skill_row(skill: Skill) -> Element(Msg) {
  html.li([], [
    html.strong([], [html.text(skill.name)]),
    html.text(" " <> skill.description),
    case skill_status(skill) {
      "" -> element.none()
      status ->
        html.span([attribute.class("skill-status")], [
          html.text(" (" <> status <> ")"),
        ])
    },
    case skill.managed {
      False -> element.none()
      True ->
        html.button(
          [attribute.type_("button"), event.on_click(EditSkill(skill))],
          [html.text("Edit")],
        )
    },
    case skill.managed {
      False -> element.none()
      True ->
        html.button(
          [
            attribute.type_("button"),
            event.on_click(DeleteSkill(skill.name)),
          ],
          [html.text("Delete")],
        )
    },
  ])
}

fn skill_text_field(
  label: String,
  field: String,
  value: String,
) -> Element(Msg) {
  html.label([attribute.class("field")], [
    html.text(label),
    html.input([
      attribute.type_("text"),
      attribute.name("skill-" <> field),
      attribute.value(value),
      event.on_input(fn(value) { SkillField(field, value) }),
    ]),
  ])
}

fn skill_role_choice(skill: Skill, role: Role) -> Element(Msg) {
  let chosen = skill.all_roles || list.contains(skill.roles, role.id)
  html.label([attribute.class("switch")], [
    html.input([
      attribute.type_("checkbox"),
      attribute.name("role-skill"),
      attribute.value(skill.name),
      attribute.checked(chosen),
      attribute.disabled(!skill.managed),
      event.on_check(fn(chosen) { AssignSkill(skill.name, role.id, chosen) }),
    ]),
    html.text(skill.name),
  ])
}

/// The skill form: its name, description and body.
pub fn skill_form(f: SkillForm) -> Element(Msg) {
  html.form(
    [attribute.class("model-form"), event.on_submit(fn(_) { SaveSkill })],
    [
      html.h3([], [
        html.text(case f.original {
          "" -> "Add a skill"
          _ -> "Edit " <> f.name
        }),
      ]),
      skill_text_field("Name", "name", f.name),
      skill_text_field("Description", "description", f.description),
      html.label([attribute.class("field")], [
        html.text("Body"),
        html.textarea(
          [
            attribute.class("skill-body"),
            attribute.name("skill-body"),
            event.on_input(fn(value) { SkillField("body", value) }),
          ],
          f.body,
        ),
      ]),
      html.button([attribute.type_("submit")], [html.text("Save skill")]),
      case f.original {
        "" -> element.none()
        _ ->
          html.button([attribute.type_("button"), event.on_click(CancelSkill)], [
            html.text("Cancel"),
          ])
      },
    ],
  )
}

/// The delete route for one skill; its reply is the managed skills, or a refusal.
fn delete_skill(name: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/skills/" <> uri.percent_encode(name), "DELETE", "", fn(text) {
      send(SkillsSaved(text))
    })
  })
}

/// The model a role starts with.
pub type Assignment {
  Assignment(role: String, default: String)
}

/// The role models in a `GET /role-models` reply.
pub fn assignments_decoder() -> decode.Decoder(List(Assignment)) {
  decode.list({
    use role <- decode.field("role", decode.string)
    use default <- decode.optional_field("default", "", decode.string)
    decode.success(Assignment(role:, default:))
  })
}

fn assignment_for(assignments: List(Assignment), role: String) -> Assignment {
  case list.find(assignments, fn(a) { a.role == role }) {
    Ok(a) -> a
    Error(_) -> Assignment(role, "")
  }
}

fn assignment_body(a: Assignment) -> String {
  json.to_string(
    json.object([
      #("role", json.string(a.role)),
      #("default", json.string(a.default)),
    ]),
  )
}

/// The role model route; its reply is every role's assignments, or a refusal.
fn call_role_models(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/role-models", method, body, fn(text) {
      send(RoleModelsLoaded(text))
    })
  })
}

/// The model each role starts with.
pub fn role_models_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "role-models")], [
    html.h2([attribute.id("role-models")], [html.text("Role models")]),
    html.p([], [
      html.text(
        "Choose the model each role starts with. With None, pick one on each task.",
      ),
    ]),
    case model.models {
      [] ->
        html.p([attribute.class("reason")], [
          html.text(
            "No models are available yet. Set one up under Models or install an agent.",
          ),
        ])
      _ -> element.none()
    },
    case model.roles {
      [] -> html.p([], [html.text("No roles yet.")])
      roles ->
        html.div(
          [attribute.class("role-models")],
          list.map(roles, fn(role) { role_model_row(model, role) }),
        )
    },
  ])
}

fn role_model_row(model: Model, role: Role) -> Element(Msg) {
  let assignment = assignment_for(model.assignments, role.id)
  html.div([attribute.class("role-model")], [
    html.h3([], [html.text(role.name)]),
    html.p([attribute.class("reason")], [html.text(role.purpose)]),
    html.div([attribute.class("role-model-picks")], [
      role_model_pick(model.models, role, assignment),
    ]),
    role_skills(model, role),
  ])
}

fn role_model_pick(
  models: List(Entry),
  role: Role,
  assignment: Assignment,
) -> Element(Msg) {
  html.label([attribute.class("field")], [
    html.text("Model"),
    html.select(
      [
        attribute.name("role-" <> role.id <> "-model"),
        event.on_change(fn(choice) { ChooseRoleModel(role.id, choice) }),
      ],
      model_options(models, assignment.default),
    ),
  ])
}

fn model_options(models: List(Entry), chosen: String) -> List(Element(Msg)) {
  let options = [
    html.option(
      [attribute.value(""), attribute.selected(chosen == "")],
      "None (choose on each task)",
    ),
    ..list.map(models, fn(e) {
      html.option(
        [attribute.value(e.id), attribute.selected(chosen == e.id)],
        e.label,
      )
    })
  ]
  case chosen {
    "" -> options
    id ->
      case list.find(models, fn(e) { e.id == id }) {
        Ok(_) -> options
        Error(_) ->
          list.append(options, [
            html.option(
              [attribute.value(id), attribute.selected(True)],
              id <> " (not set up here)",
            ),
          ])
      }
  }
}

/// The POST body for one role's model choice.
pub fn role_model_body(
  assignments: List(Assignment),
  role: String,
  choice: String,
) -> String {
  let assignment = assignment_for(assignments, role)
  assignment_body(Assignment(..assignment, default: choice))
}

/// The Settings page's tabs, in the order they show.
pub type Tab {
  Agents
  Appearance
  Languages
  About
}

pub fn tabs() -> List(Tab) {
  [Agents, Appearance, Languages, About]
}

pub fn label(tab: Tab) -> String {
  case tab {
    Agents -> "Agents"
    Appearance -> "Appearance"
    Languages -> "Languages"
    About -> "About"
  }
}

/// The tab's name in the URL: `/settings?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    Agents -> "agents"
    Appearance -> "appearance"
    Languages -> "languages"
    About -> "about"
  }
}

/// The tab a URL query names ("?tab=languages"); Agents when it names none or an unknown one.
pub fn tab_from(query: String) -> Tab {
  let named =
    query
    |> string.replace("?", "")
    |> string.split("&")
    |> list.find(fn(pair) { string.starts_with(pair, "tab=") })
  case named {
    Ok(pair) -> {
      let name = string.replace(pair, "tab=", "")
      case list.find(tabs(), fn(tab) { tab_name(tab) == name }) {
        Ok(tab) -> tab
        Error(_) -> Agents
      }
    }
    Error(_) -> Agents
  }
}

fn tab_button(tab: Tab, current: Tab) -> Element(Msg) {
  let selected = tab == current
  html.button(
    [
      attribute.id("tab-" <> tab_name(tab)),
      attribute.role("tab"),
      attribute.attribute("aria-selected", case selected {
        True -> "true"
        False -> "false"
      }),
      attribute.attribute("aria-controls", "panel-" <> tab_name(tab)),
      event.on_click(Select(tab)),
    ],
    [html.text(label(tab))],
  )
}

fn appearance_section(dark: Bool) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "appearance")], [
    html.h2([attribute.id("appearance")], [html.text("Theme")]),
    theme_control(dark),
  ])
}

/// The running program's version in a `GET /about` reply.
pub fn version_decoder() -> decode.Decoder(String) {
  use version <- decode.field("version", decode.string)
  decode.success(version)
}

/// The about route; its reply is the running program's version, or a refusal.
fn call_about() -> Effect(Msg) {
  effect.from(fn(send) {
    request("/about", "GET", "", fn(text) { send(AboutLoaded(text)) })
  })
}

/// The running program's version and the license it ships under.
pub fn about_section(model: Model) -> Element(Msg) {
  let version = case model.version {
    "" -> []
    version -> [html.p([], [html.text("Software version: " <> version)])]
  }
  html.section(
    [attribute.attribute("aria-labelledby", "about")],
    list.flatten([
      [html.h2([attribute.id("about")], [html.text("About")])],
      version,
      [
        html.details([attribute.class("about-license")], [
          html.summary([], [html.text("Sustainable Use License 1.0")]),
          solace_task.md("", license_text),
        ]),
      ],
    ]),
  )
}

/// The license this program ships under, from `LICENSE.md`.
const license_text =
  "# License\n\n"
  <> "Copyright (c) 2026 Aaron Donoho\n\n"
  <> "Portions of this software are licensed as follows:\n\n"
  <> "- All third party components incorporated into Solace are licensed under the original license provided by the owner of the applicable component. For example, the IBM Plex fonts are under the SIL Open Font License in `solace-cli/board/priv/fonts/LICENSE-OFL.txt`.\n"
  <> "- Programs and other work you build with Solace are yours. These terms do not apply to them.\n"
  <> "- All other content is available under the \"Sustainable Use License\" as defined below.\n\n"
  <> "## Sustainable Use License\n\n"
  <> "Version 1.0\n\n"
  <> "### Acceptance\n\n"
  <> "By using the software, you agree to all of the terms and conditions below.\n\n"
  <> "### Copyright License\n\n"
  <> "The licensor grants you a non-exclusive, royalty-free, worldwide, non-sublicensable, non-transferable license to use, copy, distribute, make available, and prepare derivative works of the software, in each case subject to the limitations below.\n\n"
  <> "### Limitations\n\n"
  <> "You may use or modify the software only for your own internal business purposes or for non-commercial or personal use. You may distribute the software or provide it to others only if you do so free of charge for non-commercial purposes. You may not alter, remove, or obscure any licensing, copyright, or other notices of the licensor in the software. Any use of the licensor’s trademarks is subject to applicable law.\n\n"
  <> "### Patents\n\n"
  <> "The licensor grants you a license, under any patent claims the licensor can license, or becomes able to license, to make, have made, use, sell, offer for sale, import and have imported the software, in each case subject to the limitations and conditions in this license. This license does not cover any patent claims that you cause to be infringed by modifications or additions to the software. If you or your company make any written claim that the software infringes or contributes to infringement of any patent, your patent license for the software granted under these terms ends immediately. If your company makes such a claim, your patent license ends immediately for work on behalf of your company.\n\n"
  <> "### Notices\n\n"
  <> "You must ensure that anyone who gets a copy of any part of the software from you also gets a copy of these terms. If you modify the software, you must include in any modified copies of the software a prominent notice stating that you have modified the software.\n\n"
  <> "### No Other Rights\n\n"
  <> "These terms do not imply any licenses other than those expressly granted in these terms.\n\n"
  <> "### Termination\n\n"
  <> "If you use the software in violation of these terms, such use is not licensed, and your license will automatically terminate. If the licensor provides you with a notice of your violation, and you cease all violation of this license no later than 30 days after you receive that notice, your license will be reinstated retroactively. However, if you violate these terms after such reinstatement, any additional violation of these terms will cause your license to terminate automatically and permanently.\n\n"
  <> "### No Liability\n\n"
  <> "As far as the law allows, the software comes as is, without any warranty or condition, and the licensor will not be liable to you for any damages arising out of these terms or the use or nature of the software, under any kind of legal claim.\n\n"
  <> "### Definitions\n\n"
  <> "The “licensor” is the entity offering these terms.\n\n"
  <> "The “software” is the software the licensor makes available under these terms, including any portion of it.\n\n"
  <> "“You” refers to the individual or entity agreeing to these terms.\n\n"
  <> "“Your company” is any legal entity, sole proprietorship, or other kind of organization that you work for, plus all organizations that have control over, are under the control of, or are under common control with that organization. Control means ownership of substantially all the assets of an entity, or the power to direct its management and policies by vote, contract, or otherwise. Control can be direct or indirect.\n\n"
  <> "“Your license” is the license granted to you for the software under these terms.\n\n"
  <> "“Use” means anything you do with the software requiring your license.\n\n"
  <> "“Trademark” means trademarks, service marks, and similar rights.\n"

fn skill_status(skill: Skill) -> String {
  case skill.all_roles {
    True -> "All roles"
    False ->
      case skill.roles {
        [] -> "Not used by any role"
        _ -> ""
      }
  }
}

/// The POST body for the skill in the form; an edit keeps that skill's roles.
pub fn skill_save_body(model: Model) -> String {
  let f = model.skill_form
  let roles = case
    list.find(model.skills, fn(skill) { skill.name == f.original })
  {
    Ok(skill) -> skill.roles
    Error(_) -> []
  }
  skill_body(f, roles)
}

/// The POST body that sets one skill's roles after a role card tick.
pub fn skill_roles_body(skill: Skill, role: String, chosen: Bool) -> String {
  skill_body(
    SkillForm(skill.name, skill.name, skill.description, skill.body),
    roles_with(skill.roles, role, chosen),
  )
}

/// The save route; its reply is the listed skills, and empties the form.
fn call_skills_saved(method: String, body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/skills", method, body, fn(text) { send(SkillsSaved(text)) })
  })
}

fn roles_with(roles: List(String), role: String, chosen: Bool) -> List(String) {
  case chosen {
    True ->
      case list.contains(roles, role) {
        True -> roles
        False -> [role, ..roles]
      }
    False -> list.filter(roles, fn(id) { id != role })
  }
}

fn skills_set_role(
  skills: List(Skill),
  name: String,
  role: String,
  chosen: Bool,
) -> List(Skill) {
  list.map(skills, fn(skill) {
    case skill.name == name {
      True -> Skill(..skill, roles: roles_with(skill.roles, role, chosen))
      False -> skill
    }
  })
}

/// The role-card tick route; its reply is the skills, or why it was refused.
fn call_skill_tick(body: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("/skills", "POST", body, fn(text) { send(SkillTickSaved(text)) })
  })
}

fn skills_with_ticks(
  server: List(Skill),
  ticks: List(skill_writes.SkillTick),
) -> List(Skill) {
  list.fold(ticks, server, fn(skills, tick) {
    skills_set_role(skills, tick.name, tick.role, tick.chosen)
  })
}

fn assign_skill(
  model: Model,
  name: String,
  role: String,
  chosen: Bool,
) -> #(Model, Effect(Msg)) {
  case
    list.find(model.skills, fn(skill) { skill.name == name && skill.managed })
  {
    Ok(_) ->
      start_skill_write(
        model,
        skill_writes.TickWrite(skill_writes.SkillTick(name, role, chosen)),
      )
    Error(_) -> #(model, effect.none())
  }
}

/// Sends a skills write, or holds it until the one in flight has a reply.
fn start_skill_write(
  model: Model,
  write: skill_writes.SkillWrite,
) -> #(Model, Effect(Msg)) {
  let sending = case model.skills_writing {
    None -> True
    Some(_) -> False
  }
  let #(writing, waiting) =
    skill_writes.start(model.skills_writing, model.skills_waiting, write)
  let model =
    refresh_skills(
      Model(..model, skills_writing: writing, skills_waiting: waiting),
    )
  case sending {
    True -> #(model, skill_write_effect(model, write))
    False -> #(model, effect.none())
  }
}

fn skill_write_effect(
  model: Model,
  write: skill_writes.SkillWrite,
) -> Effect(Msg) {
  case write {
    skill_writes.TickWrite(tick) ->
      call_skill_tick(skill_tick_body(model, tick))
    skill_writes.SaveWrite(body) -> call_skills_saved("POST", body)
    skill_writes.DeleteWrite(name) -> delete_skill(name)
    skill_writes.LoadWrite -> call_skills("GET", "")
  }
}

/// Sends the next held write after a reply, or marks the writes done.
fn resume_skill_writes(model: Model) -> #(Model, Effect(Msg)) {
  case model.skills_writing {
    Some(_) -> #(model, effect.none())
    None -> {
      let #(writing, waiting) = skill_writes.next(model.skills_waiting)
      let model =
        refresh_skills(
          Model(..model, skills_writing: writing, skills_waiting: waiting),
        )
      case writing {
        Some(write) -> #(model, skill_write_effect(model, write))
        None -> #(model, effect.none())
      }
    }
  }
}

/// The POST body for one role-card tick, against the server's last known skills.
pub fn skill_tick_body(model: Model, tick: skill_writes.SkillTick) -> String {
  case list.find(model.skills_server, fn(skill) { skill.name == tick.name }) {
    Ok(skill) -> skill_roles_body(skill, tick.role, tick.chosen)
    Error(_) -> ""
  }
}

fn refresh_skills(model: Model) -> Model {
  Model(
    ..model,
    skills: skills_with_ticks(
      model.skills_server,
      skill_writes.ticks(model.skills_writing, model.skills_waiting),
    ),
  )
}

fn finish_skill_write(model: Model) -> #(Model, Effect(Msg)) {
  resume_skill_writes(refresh_skills(Model(..model, skills_writing: None)))
}

fn skills_loaded(model: Model, text: String) -> #(Model, Effect(Msg)) {
  case json.parse(text, skills_decoder()) {
    Ok(loaded) ->
      finish_skill_write(Model(..model, skills_server: loaded, error: ""))
    Error(_) -> finish_skill_write(Model(..model, error: refusal(text)))
  }
}

fn skills_saved(model: Model, text: String) -> #(Model, Effect(Msg)) {
  case json.parse(text, skills_decoder()) {
    Ok(loaded) ->
      finish_skill_write(
        Model(
          ..model,
          skills_server: loaded,
          skill_form: blank_skill_form,
          error: "",
        ),
      )
    Error(_) -> finish_skill_write(Model(..model, error: refusal(text)))
  }
}

fn queue_skill_load(model: Model) -> #(Model, Effect(Msg)) {
  let #(writing, waiting) =
    skill_writes.queue_load(model.skills_writing, model.skills_waiting)
  resume_skill_writes(refresh_skills(
    Model(..model, skills_writing: writing, skills_waiting: waiting),
  ))
}
