import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/dev/query
import lustre/dev/simulate
import lustre/element
import settings
import solace_task

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn the_switch_is_on_when_dark_test() {
  let on = element.to_string(settings.theme_control(True))
  let off = element.to_string(settings.theme_control(False))
  assert string.contains(on, "checked")
  assert !string.contains(off, "checked")
  assert string.contains(on, "type=\"checkbox\"")
  assert string.contains(on, "role=\"switch\"")
  assert string.contains(off, "Dark mode</label>")
}

pub fn the_page_holds_the_dark_mode_switch_test() {
  let page = element.to_string(settings.view(settings.initial(True)))
  // The menu names the page, so it has no Settings heading of its own.
  assert !string.contains(page, "<h1>Settings</h1>")
  assert string.contains(page, "Appearance</h2>")
  assert string.contains(page, element.to_string(settings.theme_control(True)))
}

/// The switch and a change from another window both decide whether it shows on.
pub fn the_switch_and_other_windows_set_the_theme_test() {
  let off = settings.initial(False)
  let on = settings.initial(True)
  assert settings.update(off, settings.SetDark(True)).0 == on
  assert settings.update(on, settings.SetDark(False)).0 == off
  assert settings.update(off, settings.ThemeChanged("dark")).0 == on
  assert settings.update(on, settings.ThemeChanged("light")).0 == off
}

/// Connecting starts listening once; disconnecting stops it.
pub fn listening_follows_the_element_in_and_out_of_the_page_test() {
  let stop = fn() { Nil }
  let off = settings.initial(False)
  let listening = settings.update(off, settings.Subscribed(stop)).0
  assert listening == settings.Model(..off, unsubscribe: Some(stop))
  // Connected again while listening keeps the one subscription.
  assert settings.update(listening, settings.Connected).0 == listening
  assert settings.update(listening, settings.Disconnected).0 == off
  assert settings.update(off, settings.Disconnected).0 == off
}

/// The controls' styles name no color of their own, and the page uses them.
pub fn control_styles_use_only_tokens_test() {
  let styles = solace_task.control_styles
  assert !string.contains(styles, "#")
  assert !string.contains(styles, "rgb")
  assert !string.contains(styles, "hsl")
  assert string.contains(styles, "option,optgroup{background:var(--paper)")
  let page = element.to_string(settings.view(settings.initial(True)))
  assert string.contains(page, "option,optgroup{background:var(--paper)")
}

/// The settings section fills the page: it has no width cap of its own.
pub fn section_fills_the_page_test() {
  let page = element.to_string(settings.view(settings.initial(True)))
  let assert [css, ..] = page |> string.split("</style>")
  assert string.contains(
    css,
    ".settings section{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}",
  )
  assert !string.contains(css, ".settings section{max-width")
}

/// The list shows the models set up here, each with why it can't start; a
/// refused change says why and keeps the list.
pub fn models_list_only_configured_ones_with_their_reasons_test() {
  let reply =
    "[{\"id\":\"cc\",\"label\":\"Command Code\",\"agent\":\"codex\",\"model\":\"deepseek\",\"base_url\":\"https://h/v1\",\"has_key\":false,\"reason\":\"No key yet; add one in Settings\",\"configured\":true},{\"id\":\"opus\",\"label\":\"Opus\",\"agent\":\"claude\",\"model\":\"opus\",\"base_url\":null,\"reason\":null,\"configured\":false}]"
  let loaded =
    settings.update(settings.initial(False), settings.ModelsLoaded(reply)).0
  assert loaded.models
    == [
      settings.Entry(
        id: "cc",
        label: "Command Code",
        agent: "codex",
        model: "deepseek",
        base_url: "https://h/v1",
        has_key: False,
        reason: "No key yet; add one in Settings",
        configured: True,
      ),
      settings.Entry(
        id: "opus",
        label: "Opus",
        agent: "claude",
        model: "opus",
        base_url: "",
        has_key: False,
        reason: "",
        configured: False,
      ),
    ]
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Models</h2>")
  assert string.contains(page, "No key yet; add one in Settings")
  assert !string.contains(page, "Opus")
  let refused =
    settings.update(
      loaded,
      settings.ModelsLoaded("{\"message\":\"E7001 base_url required\"}"),
    ).0
  assert refused.models == loaded.models
  assert refused.error == "E7001 base_url required"
  assert string.contains(
    element.to_string(settings.view(refused)),
    "E7001 base_url required",
  )
  assert settings.update(loaded, settings.ModelsLoaded("oops")).0.error
    == "Solace could not be reached"
}

/// Editing fills the form but never the key; a saved list empties the form.
pub fn the_form_edits_a_model_and_never_shows_its_key_test() {
  let entry =
    settings.Entry(
      id: "cc",
      label: "CC",
      agent: "codex",
      model: "m",
      base_url: "https://h/v1",
      has_key: True,
      reason: "",
      configured: True,
    )
  let editing =
    settings.update(settings.initial(False), settings.EditModel(entry)).0
  assert editing.form
    == settings.Form("cc", "CC", "codex", "m", "https://h/v1", "", True)
  let form = element.to_string(settings.model_form(editing.form))
  assert string.contains(form, "Key saved; leave blank to keep it")
  assert string.contains(form, "Edit CC")
  assert string.contains(form, "Cancel")
  let typed =
    [
      #("label", "L"),
      #("agent", "claude"),
      #("model", "m2"),
      #("base_url", "u"),
      #("key", "sk"),
      #("other", "x"),
    ]
    |> list.fold(editing, fn(m, f) {
      settings.update(m, settings.Field(f.0, f.1)).0
    })
  assert typed.form == settings.Form("cc", "L", "claude", "m2", "u", "sk", True)
  assert settings.update(typed, settings.NewModel).0.form == settings.blank_form
  assert settings.update(typed, settings.ModelsLoaded("[]")).0.form
    == settings.blank_form
  let blank = element.to_string(settings.model_form(settings.blank_form))
  assert string.contains(blank, "Add a model")
  assert string.contains(blank, "Paste the key")
  assert !string.contains(blank, "Cancel")
  // Saving and deleting send requests and leave the page as it is until the reply.
  assert settings.update(typed, settings.SaveModel).0 == typed
  let new = settings.Model(..typed, form: settings.blank_form)
  assert settings.update(new, settings.SaveModel).0 == new
  assert settings.update(typed, settings.DeleteModel(entry)).0 == typed
}

/// The Languages section shows each switch and missing state; a reply updates
/// the list, and a refusal says why and keeps it.
pub fn languages_show_and_toggle_test() {
  let off =
    "[{\"id\":\"gleam\",\"label\":\"Gleam\",\"enabled\":false,\"installed\":true},{\"id\":\"rust\",\"label\":\"Rust\",\"enabled\":true,\"installed\":false}]"
  let loaded =
    settings.update(settings.initial(False), settings.LanguagesLoaded(off)).0
  assert loaded.languages
    == [
      settings.Language("gleam", "Gleam", False, True),
      settings.Language("rust", "Rust", True, False),
    ]
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Languages</h2>")
  assert string.contains(page, ">Gleam</label>")
  assert string.contains(page, ">Rust</label>")
  assert string.contains(page, "Not found on this computer")
  assert string.contains(page, "even when its toolchain is installed")
  // A toggle leaves the page as it is until the server replies.
  assert settings.update(loaded, settings.ToggleLanguage("gleam", True)).0
    == loaded
  let on =
    "[{\"id\":\"gleam\",\"label\":\"Gleam\",\"enabled\":true,\"installed\":true},{\"id\":\"rust\",\"label\":\"Rust\",\"enabled\":true,\"installed\":false}]"
  let toggled = settings.update(loaded, settings.LanguagesLoaded(on)).0
  assert toggled.languages
    == [
      settings.Language("gleam", "Gleam", True, True),
      settings.Language("rust", "Rust", True, False),
    ]
  let refused =
    settings.update(
      loaded,
      settings.LanguagesLoaded(
        "{\"message\":\"E7001 enabled must be true or false\"}",
      ),
    ).0
  assert refused.languages == loaded.languages
  assert refused.error == "E7001 enabled must be true or false"
  assert string.contains(
    element.to_string(settings.view(refused)),
    "E7001 enabled must be true or false",
  )
  assert settings.update(loaded, settings.LanguagesLoaded("oops")).0.error
    == "Solace could not be reached"
}

/// The Agent behavior section loads from the server, saves a choice, and
/// explains each choice on hover or keyboard focus.
pub fn behavior_choices_load_save_and_explain_test() {
  let initial = settings.initial(False)
  assert initial.behavior
    == settings.Behavior("collaborator", "standard", "balanced")
  let loaded =
    settings.update(
      initial,
      settings.BehaviorLoaded(
        "{\"involvement\":\"engineer\",\"detail\":\"thorough\",\"asking\":\"rarely\"}",
      ),
    ).0
  assert loaded.behavior == settings.Behavior("engineer", "thorough", "rarely")
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Agent behavior</h2>")
  list.each(
    [
      "Director", "Collaborator", "Engineer", "Concise", "Standard", "Thorough",
      "Often", "Balanced", "Rarely", "Plain words, no code",
      "Full technical detail, code included.",
      "Ask when a choice changes what you get",
    ],
    fn(text) {
      assert string.contains(page, text)
    },
  )
  assert string.contains(page, ".choice-stop:hover .choice-help")
  assert string.contains(page, ".choice-stop:focus-within .choice-help")
  assert string.contains(page, "min-height:var(--control-h)")
  assert !string.contains(page, "as today")
  // A choice shows at once, so a refusal can put the old one back.
  let chosen =
    settings.update(loaded, settings.ChooseBehavior(settings.Detail, "concise")).0
  assert chosen.behavior == settings.Behavior("engineer", "concise", "rarely")
  assert chosen.behavior_before == Some(loaded.behavior)
  let saved =
    settings.update(
      chosen,
      settings.BehaviorLoaded(
        "{\"involvement\":\"engineer\",\"detail\":\"concise\",\"asking\":\"rarely\"}",
      ),
    ).0
  assert saved.behavior == settings.Behavior("engineer", "concise", "rarely")
  assert saved.behavior_before == None
  let refused =
    settings.update(
      chosen,
      settings.BehaviorLoaded(
        "{\"message\":\"E7001 detail must be concise, standard or thorough\"}",
      ),
    ).0
  assert refused.behavior == loaded.behavior
  assert refused.behavior_before == None
  assert refused.error == "E7001 detail must be concise, standard or thorough"
  assert string.contains(
    element.to_string(settings.view(refused)),
    "E7001 detail must be concise, standard or thorough",
  )
  assert settings.update(loaded, settings.BehaviorLoaded("oops")).0.error
    == "Solace could not be reached"
}

/// Two saves in flight: the first reply must not undo the second choice.
pub fn a_reply_for_one_behavior_choice_keeps_the_other_in_flight_test() {
  let loaded =
    settings.update(
      settings.initial(False),
      settings.BehaviorLoaded(
        "{\"involvement\":\"engineer\",\"detail\":\"thorough\",\"asking\":\"rarely\"}",
      ),
    ).0
  let detail =
    settings.update(loaded, settings.ChooseBehavior(settings.Detail, "concise")).0
  let both =
    settings.update(detail, settings.ChooseBehavior(settings.Asking, "often")).0
  let first =
    settings.update(
      both,
      settings.BehaviorLoaded(
        "{\"involvement\":\"engineer\",\"detail\":\"concise\",\"asking\":\"rarely\"}",
      ),
    ).0
  assert first.behavior == settings.Behavior("engineer", "concise", "often")
  let second =
    settings.update(
      first,
      settings.BehaviorLoaded(
        "{\"involvement\":\"engineer\",\"detail\":\"concise\",\"asking\":\"often\"}",
      ),
    ).0
  assert second.behavior == settings.Behavior("engineer", "concise", "often")
  assert second.behavior_before == None
}

/// Skills load by role, the form adds, edits and cancels one, and a refusal
/// keeps the list while saying why.
pub fn skills_load_edit_and_refuse_from_the_server_test() {
  let roles =
    settings.update(
      settings.initial(False),
      settings.RolesLoaded(
        "[{\"id\":\"worker\",\"name\":\"Worker\",\"purpose\":\"Makes changes.\"},{\"id\":\"reviewer\",\"name\":\"Reviewer\",\"purpose\":\"Checks changes.\"}]",
      ),
    ).0
  let loaded =
    settings.update(
      roles,
      settings.SkillsLoaded(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"]}]",
      ),
    ).0
  assert loaded.roles
    == [
      settings.Role("worker", "Worker", "Makes changes."),
      settings.Role("reviewer", "Reviewer", "Checks changes."),
    ]
  assert loaded.skills
    == [
      settings.Skill("house-review", "Review UI changes.", "Check contrast.", [
        "reviewer",
      ]),
    ]
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Skills</h2>")
  assert string.contains(page, "Reviewer")
  assert string.contains(page, "Checks changes.")
  assert string.contains(page, "house-review")
  assert string.contains(page, "Review UI changes.")
  assert string.contains(page, "Add a skill")
  let fresh = settings.update(loaded, settings.NewSkill("reviewer")).0
  assert fresh.skill_form == settings.SkillForm("", "", "", "", ["reviewer"])
  let typed =
    [
      #("name", "commit-style"),
      #("description", "Write small commits."),
      #("body", "Keep every commit small."),
    ]
    |> list.fold(fresh, fn(model, field) {
      settings.update(model, settings.SkillField(field.0, field.1)).0
    })
  assert typed.skill_form
    == settings.SkillForm(
      "",
      "commit-style",
      "Write small commits.",
      "Keep every commit small.",
      ["reviewer"],
    )
  let both = settings.update(typed, settings.ChooseSkillRole("worker", True)).0
  assert both.skill_form.roles == ["worker", "reviewer"]
  let one = settings.update(both, settings.ChooseSkillRole("reviewer", False)).0
  assert one.skill_form.roles == ["worker"]
  // A save leaves the page as it is until the server replies.
  assert settings.update(typed, settings.SaveSkill).0 == typed
  let editing =
    settings.update(
      loaded,
      settings.EditSkill(
        settings.Skill("house-review", "Review UI changes.", "Check contrast.", [
          "reviewer",
        ]),
      ),
    ).0
  assert editing.skill_form
    == settings.SkillForm(
      "house-review",
      "house-review",
      "Review UI changes.",
      "Check contrast.",
      ["reviewer"],
    )
  let form =
    element.to_string(settings.skill_form(editing.skill_form, editing.roles))
  assert string.contains(form, "Edit house-review")
  assert string.contains(form, "Delete skill")
  assert string.contains(form, "Cancel")
  assert string.contains(form, "type=\"checkbox\"")
  assert settings.update(editing, settings.CancelSkill).0.skill_form
    == settings.blank_skill_form
  // Removing a role leaves the page as it is until the server replies.
  assert settings.update(
      loaded,
      settings.RemoveSkill("house-review", "reviewer"),
    ).0
    == loaded
  let refused =
    settings.update(
      loaded,
      settings.SkillsLoaded("{\"message\":\"Give the skill a body.\"}"),
    ).0
  assert refused.skills == loaded.skills
  assert refused.error == "Give the skill a body."
  assert string.contains(
    element.to_string(settings.view(refused)),
    "Give the skill a body.",
  )
  assert !string.contains(element.to_string(settings.view(refused)), "E7001")
  assert settings.update(loaded, settings.SkillsLoaded("oops")).0.error
    == "Solace could not be reached"
}

/// Typing in the rendered Name and Description boxes fills the skill form.
pub fn the_skill_form_accepts_typing_in_name_and_description_test() {
  let app =
    simulate.simple(
      init: settings.initial,
      update: fn(model, msg) { settings.update(model, msg).0 },
      view: settings.view,
    )
  let typed =
    simulate.start(app, False)
    |> simulate.message(settings.RolesLoaded(
      "[{\"id\":\"worker\",\"name\":\"Worker\",\"purpose\":\"Makes changes.\"}]",
    ))
    |> simulate.message(settings.NewSkill("worker"))
    |> simulate.input(
      on: query.element(matching: query.attribute("name", "skill-name")),
      value: "commit-style",
    )
    |> simulate.input(
      on: query.element(matching: query.attribute("name", "skill-description")),
      value: "Write small commits.",
    )
    |> simulate.input(
      on: query.element(matching: query.attribute("name", "skill-body")),
      value: "Keep every commit small.",
    )
  assert simulate.model(typed).skill_form
    == settings.SkillForm(
      "",
      "commit-style",
      "Write small commits.",
      "Keep every commit small.",
      ["worker"],
    )
  let page = element.to_string(simulate.view(typed))
  assert string.contains(page, "value=\"commit-style\"")
  assert string.contains(page, "value=\"Write small commits.\"")
  assert string.contains(page, "Keep every commit small.")
  assert simulate.model(simulate.message(typed, settings.SaveSkill)).skill_form
    == simulate.model(typed).skill_form
}

/// Role models load from the server, save one role's choice, and ignore an
/// old backup. The role picker lists every model the task page can offer.
pub fn role_models_load_save_test() {
  let roles =
    settings.update(
      settings.initial(False),
      settings.RolesLoaded(
        "[{\"id\":\"worker\",\"name\":\"Worker\",\"purpose\":\"Makes the change.\"},{\"id\":\"reviewer\",\"name\":\"Reviewer\",\"purpose\":\"Checks the change.\"}]",
      ),
    ).0
  let models =
    settings.update(
      roles,
      settings.ModelsLoaded(
        "[{\"id\":\"cc\",\"label\":\"Command Code\",\"agent\":\"codex\",\"model\":\"deepseek\",\"base_url\":\"https://h/v1\",\"has_key\":true,\"configured\":true},{\"id\":\"local\",\"label\":\"Local\",\"agent\":\"codex\",\"model\":\"qwen\",\"base_url\":\"https://l/v1\",\"has_key\":true,\"configured\":true},{\"id\":\"opus\",\"label\":\"Opus\",\"agent\":\"claude\",\"model\":\"opus\",\"base_url\":null,\"reason\":\"Start Ollama first\",\"configured\":false}]",
      ),
    ).0
  let loaded =
    settings.update(
      models,
      settings.RoleModelsLoaded(
        "[{\"role\":\"worker\",\"default\":\"cc\",\"backup\":\"local\"},{\"role\":\"reviewer\",\"default\":\"gone\",\"backup\":\"\"}]",
      ),
    ).0
  assert loaded.assignments
    == [
      settings.Assignment("worker", "cc"),
      settings.Assignment("reviewer", "gone"),
    ]
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Role models</h2>")
  assert string.contains(page, "Choose the model each role starts with")
  assert string.contains(page, "Worker")
  assert string.contains(page, "Reviewer")
  assert string.contains(page, "Model")
  assert string.contains(page, "Command Code")
  assert string.contains(page, "Opus")
  assert !string.contains(page, "Command Code (deepseek)")
  assert !string.contains(page, "Opus (opus) — Start Ollama first")
  assert string.contains(page, "gone (not set up here)")
  assert string.contains(page, "None (choose on each task)")
  assert !string.contains(page, "Backup")
  assert !string.contains(page, "Use fallback")
  assert settings.role_model_body(loaded.assignments, "worker", "local")
    == "{\"role\":\"worker\",\"default\":\"local\"}"
  let saved =
    settings.update(
      loaded,
      settings.RoleModelsLoaded(
        "[{\"role\":\"worker\",\"default\":\"local\"},{\"role\":\"reviewer\",\"default\":\"gone\"}]",
      ),
    ).0
  assert saved.assignments
    == [
      settings.Assignment("worker", "local"),
      settings.Assignment("reviewer", "gone"),
    ]
  // A missing assignment is empty, and every role still shows.
  let missing =
    settings.update(
      roles,
      settings.RoleModelsLoaded("[{\"role\":\"worker\",\"default\":\"\"}]"),
    ).0
  assert missing.assignments == [settings.Assignment("worker", "")]
  assert string.contains(element.to_string(settings.view(missing)), "Reviewer")
  // A choice saves even when the role list failed to load.
  let none = settings.update(roles, settings.RoleModelsLoaded("[]")).0
  assert settings.role_model_body(none.assignments, "worker", "cc")
    == "{\"role\":\"worker\",\"default\":\"cc\"}"
  assert settings.role_model_body(none.assignments, "worker", "")
    == "{\"role\":\"worker\",\"default\":\"\"}"
  // A refusal keeps the choices while saying why.
  let refused =
    settings.update(
      loaded,
      settings.RoleModelsLoaded(
        "{\"message\":\"E7001 role must be a known role\"}",
      ),
    ).0
  assert refused.assignments == loaded.assignments
  assert refused.error == "E7001 role must be a known role"
  assert string.contains(
    element.to_string(settings.view(refused)),
    "E7001 role must be a known role",
  )
  assert settings.update(loaded, settings.RoleModelsLoaded("oops")).0.error
    == "Solace could not be reached"
  // With no models, the page says where to find one.
  let bare = settings.update(roles, settings.ModelsLoaded("[]")).0
  assert string.contains(
    element.to_string(settings.view(bare)),
    "No models are available yet",
  )
}
