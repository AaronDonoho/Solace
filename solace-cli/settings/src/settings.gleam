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
    /// The models this computer can pick, from the system settings.
    models: List(Entry),
    /// The languages on this computer, from the system settings.
    languages: List(Language),
    /// How agents address the human, how much they write, and how readily they ask.
    behavior: Behavior,
    /// The saved choices while a change is in flight, so a refusal can restore them.
    behavior_before: Option(Behavior),
    /// The choices whose save is still in flight, in the order they were made.
    behavior_pending: List(#(Setting, String)),
    /// The skills Settings manages in this workspace.
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
  /// The server's agent behavior choices, or why it refused a change.
  BehaviorLoaded(String)
  /// The server's roles, or why it refused.
  RolesLoaded(String)
  /// The server's model for each role, or why it refused.
  RoleModelsLoaded(String)
  /// The server's managed skills, or why it refused a change.
  SkillsLoaded(String)
  /// Opens a skill in the form.
  EditSkill(Skill)
  /// Empties the form for a new skill in a role.
  NewSkill(String)
  /// A skill form field, by name, and its new value.
  SkillField(String, String)
  /// Adds or removes a role on the skill in the form.
  ChooseSkillRole(String, Bool)
  SaveSkill
  /// Removes a skill from one role.
  RemoveSkill(String, String)
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
            call_behavior("GET", ""),
            call_roles("GET", ""),
            call_role_models("GET", ""),
            call_skills("GET", ""),
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
      case json.parse(text, skills_decoder()) {
        Ok(skills) -> #(
          Model(..model, skills:, skill_form: blank_skill_form, error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    EditSkill(skill) -> #(
      Model(
        ..model,
        skill_form: SkillForm(
          skill.name,
          skill.name,
          skill.description,
          skill.body,
          skill.roles,
        ),
        error: "",
      ),
      effect.none(),
    )
    NewSkill(role) -> #(
      Model(..model, skill_form: SkillForm("", "", "", "", [role]), error: ""),
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
    ChooseSkillRole(role, chosen) -> {
      let f = model.skill_form
      let roles = case chosen {
        True ->
          case list.contains(f.roles, role) {
            True -> f.roles
            False -> [role, ..f.roles]
          }
        False -> list.filter(f.roles, fn(id) { id != role })
      }
      #(Model(..model, skill_form: SkillForm(..f, roles:)), effect.none())
    }
    SaveSkill -> #(model, call_skills("POST", skill_body(model.skill_form)))
    RemoveSkill(name, role) ->
      case list.find(model.skills, fn(skill) { skill.name == name }) {
        Ok(skill) -> {
          let roles = list.filter(skill.roles, fn(id) { id != role })
          case roles {
            [] -> #(model, delete_skill(name))
            _ -> #(
              model,
              call_skills(
                "POST",
                skill_body(SkillForm(
                  name,
                  name,
                  skill.description,
                  skill.body,
                  roles,
                )),
              ),
            )
          }
        }
        Error(_) -> #(model, effect.none())
      }
    DeleteSkill(name) -> #(model, delete_skill(name))
    CancelSkill -> #(
      Model(..model, skill_form: blank_skill_form, error: ""),
      effect.none(),
    )
    LanguagesLoaded(text) ->
      case json.parse(text, languages_decoder()) {
        Ok(languages) -> #(Model(..model, languages:, error: ""), effect.none())
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
  html.main([attribute.class("settings")], [
    html.style([], view_styles),
    html.section([attribute.attribute("aria-labelledby", "appearance")], [
      html.h2([attribute.id("appearance")], [html.text("Appearance")]),
      theme_control(model.dark),
    ]),
    behavior_section(model),
    role_models_section(model),
    languages_section(model),
    models_section(model),
    skills_section(model),
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
.settings .error{color:var(--ink);font-weight:600}
.switch{display:flex;align-items:center;gap:10px;width:fit-content;cursor:pointer}
.switch input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}
.switch input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}
.switch input:checked{background:var(--focus)}.switch input:checked::before{transform:translateX(14px)}
.switch input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.role-skills-group{margin:14px 0}
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
    unsubscribe: None,
    models: [],
    languages: [],
    behavior: default_behavior,
    behavior_before: None,
    behavior_pending: [],
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

/// A skill Settings manages.
pub type Skill {
  Skill(name: String, description: String, body: String, roles: List(String))
}

/// The skill form; `original` is "" for a new skill.
pub type SkillForm {
  SkillForm(
    original: String,
    name: String,
    description: String,
    body: String,
    roles: List(String),
  )
}

pub const blank_skill_form = SkillForm("", "", "", "", [])

/// The roles in a `GET /roles` reply.
pub fn roles_decoder() -> decode.Decoder(List(Role)) {
  decode.list({
    use id <- decode.field("id", decode.string)
    use name <- decode.field("name", decode.string)
    use purpose <- decode.field("purpose", decode.string)
    decode.success(Role(id:, name:, purpose:))
  })
}

/// The managed skills in a `GET /skills` reply.
pub fn skills_decoder() -> decode.Decoder(List(Skill)) {
  decode.list({
    use name <- decode.field("name", decode.string)
    use description <- decode.optional_field("description", "", decode.string)
    use body <- decode.optional_field("body", "", decode.string)
    use roles <- decode.optional_field("roles", [], decode.list(decode.string))
    decode.success(Skill(name:, description:, body:, roles:))
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

fn skill_body(f: SkillForm) -> String {
  json.to_string(
    json.object([
      #("original", json.string(f.original)),
      #("name", json.string(f.name)),
      #("description", json.string(f.description)),
      #("body", json.string(f.body)),
      #("roles", json.array(f.roles, of: json.string)),
    ]),
  )
}

/// The skills a run in each role receives, with a form to add or edit one.
pub fn skills_section(model: Model) -> Element(Msg) {
  html.section([attribute.attribute("aria-labelledby", "skills")], [
    html.h2([attribute.id("skills")], [html.text("Skills")]),
    html.p([], [
      html.text(
        "Instructions for agents in each role. A skill saved here applies to every project in this workspace.",
      ),
    ]),
    html.div(
      [attribute.class("role-skills")],
      list.map(model.roles, fn(role) { role_skills(model, role) }),
    ),
    skill_form(model.skill_form, model.roles),
  ])
}

fn role_skills(model: Model, role: Role) -> Element(Msg) {
  let mine =
    list.filter(model.skills, fn(skill) { list.contains(skill.roles, role.id) })
  html.div([attribute.class("role-skills-group")], [
    html.h3([], [html.text(role.name)]),
    html.p([attribute.class("reason")], [html.text(role.purpose)]),
    case mine {
      [] -> html.p([], [html.text("No skills yet.")])
      _ ->
        html.ul(
          [attribute.class("skills")],
          list.map(mine, fn(skill) { skill_row(skill, role.id) }),
        )
    },
    html.button([attribute.type_("button"), event.on_click(NewSkill(role.id))], [
      html.text("Add a skill"),
    ]),
  ])
}

fn skill_row(skill: Skill, role: String) -> Element(Msg) {
  html.li([], [
    html.strong([], [html.text(skill.name)]),
    html.text(" " <> skill.description),
    html.button([attribute.type_("button"), event.on_click(EditSkill(skill))], [
      html.text("Edit"),
    ]),
    html.button(
      [attribute.type_("button"), event.on_click(RemoveSkill(skill.name, role))],
      [html.text("Remove")],
    ),
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

fn skill_role_choice(f: SkillForm, role: Role) -> Element(Msg) {
  html.label([attribute.class("switch")], [
    html.input([
      attribute.type_("checkbox"),
      attribute.name("skill-role"),
      attribute.value(role.id),
      attribute.checked(list.contains(f.roles, role.id)),
      event.on_check(fn(chosen) { ChooseSkillRole(role.id, chosen) }),
    ]),
    html.text(role.name),
  ])
}

/// The skill form; its roles say which runs receive it.
pub fn skill_form(f: SkillForm, roles: List(Role)) -> Element(Msg) {
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
      html.div(
        [attribute.class("skill-roles")],
        list.map(roles, fn(role) { skill_role_choice(f, role) }),
      ),
      html.button([attribute.type_("submit")], [html.text("Save skill")]),
      case f.original {
        "" -> element.none()
        _ ->
          html.button(
            [attribute.type_("button"), event.on_click(DeleteSkill(f.original))],
            [html.text("Delete skill")],
          )
      },
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
      send(SkillsLoaded(text))
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
