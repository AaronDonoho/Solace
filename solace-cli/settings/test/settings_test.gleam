import gleam/dynamic
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/dev/query
import lustre/dev/simulate
import lustre/effect
import lustre/element
import settings
import skill_writes
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
  assert string.contains(page, "Theme</h2>")
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
  assert string.contains(page, "Enabled programming languages</h2>")
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

/// Skills load flat, with their roles and labels; the form adds and edits one
/// without asking about roles, and a refusal keeps the list while saying why.
pub fn skills_load_edit_and_refuse_from_the_server_test() {
  let roles =
    settings.update(
      connected_page(),
      settings.RolesLoaded(
        "[{\"id\":\"worker\",\"name\":\"Worker\",\"purpose\":\"Makes changes.\"},{\"id\":\"reviewer\",\"name\":\"Reviewer\",\"purpose\":\"Checks changes.\"}]",
      ),
    ).0
  let review =
    settings.Skill(
      "house-review",
      "Review UI changes.",
      "Check contrast.",
      [
        "reviewer",
      ],
      True,
      False,
    )
  let loaded =
    settings.update(
      roles,
      settings.SkillsLoaded(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"],\"managed\":true,\"all_roles\":false},{\"name\":\"general\",\"description\":\"General rules.\",\"body\":\"Keep it simple.\",\"roles\":[],\"managed\":false,\"all_roles\":true},{\"name\":\"idle\",\"description\":\"Nobody's rules.\",\"body\":\"Nothing.\",\"roles\":[],\"managed\":true,\"all_roles\":false}]",
      ),
    ).0
  assert loaded.roles
    == [
      settings.Role("worker", "Worker", "Makes changes."),
      settings.Role("reviewer", "Reviewer", "Checks changes."),
    ]
  assert loaded.skills
    == [
      review,
      settings.Skill(
        "general",
        "General rules.",
        "Keep it simple.",
        [],
        False,
        True,
      ),
      settings.Skill("idle", "Nobody's rules.", "Nothing.", [], True, False),
    ]
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "Skills</h2>")
  assert string.contains(page, "house-review")
  assert string.contains(page, "Review UI changes.")
  assert string.contains(page, "All roles")
  assert string.contains(page, "Not used by any role")
  assert string.contains(page, "Add a skill")
  // The form never asks about roles, and editing keeps the skill's roles.
  let editing = settings.update(loaded, settings.EditSkill(review)).0
  assert editing.skill_form
    == settings.SkillForm(
      "house-review",
      "house-review",
      "Review UI changes.",
      "Check contrast.",
    )
  let form = element.to_string(settings.skill_form(editing.skill_form))
  assert string.contains(form, "Edit house-review")
  assert string.contains(form, "Cancel")
  assert !string.contains(form, "checkbox")
  assert !string.contains(form, "skill-role")
  assert settings.skill_save_body(editing)
    == "{\"original\":\"house-review\",\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"]}"
  // A new skill has no roles and saves none.
  let fresh = settings.update(loaded, settings.NewSkill).0
  assert fresh.skill_form == settings.SkillForm("", "", "", "")
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
    )
  assert settings.skill_save_body(typed)
    == "{\"original\":\"\",\"name\":\"commit-style\",\"description\":\"Write small commits.\",\"body\":\"Keep every commit small.\",\"roles\":[]}"
  // A save leaves the page as it is until the server replies.
  let saving = settings.update(typed, settings.SaveSkill).0
  assert saving.skill_form == typed.skill_form
  assert settings.update(editing, settings.CancelSkill).0.skill_form
    == settings.blank_skill_form
  // A tick's reply keeps the form and the ticked roles; a save clears it.
  let assigning =
    settings.update(
      editing,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let ticked =
    settings.update(
      assigning,
      settings.SkillTickSaved(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\",\"worker\"],\"managed\":true,\"all_roles\":false}]",
      ),
    ).0
  assert ticked.skill_form == editing.skill_form
  assert ticked.skills
    == [
      settings.Skill(
        "house-review",
        "Review UI changes.",
        "Check contrast.",
        [
          "reviewer",
          "worker",
        ],
        True,
        False,
      ),
    ]
  let saving_edit = settings.update(editing, settings.SaveSkill).0
  let saved =
    settings.update(
      saving_edit,
      settings.SkillsSaved(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"],\"managed\":true,\"all_roles\":false}]",
      ),
    ).0
  assert saved.skill_form == settings.blank_skill_form
  let reloading = settings.update(loaded, settings.Connected).0
  let refused =
    settings.update(
      reloading,
      settings.SkillsLoaded("{\"message\":\"Give the skill a body.\"}"),
    ).0
  assert refused.skills == loaded.skills
  assert refused.error == "Give the skill a body."
  assert string.contains(
    element.to_string(settings.view(refused)),
    "Give the skill a body.",
  )
  assert !string.contains(element.to_string(settings.view(refused)), "E7001")
  assert settings.update(reloading, settings.SkillsLoaded("oops")).0.error
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
    |> simulate.message(settings.NewSkill)
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
  assert string.contains(page, "Roles</h2>")
  assert string.contains(page, "Choose the default model for each role")
  assert !string.contains(page, "Choose the model each role starts with")
  assert !string.contains(page, ">Model</label>")
  assert string.contains(page, "aria-label=\"Model for Worker\"")
  assert string.contains(page, "Edit Skills")
  assert string.contains(page, "href=\"#skills\"")
  assert string.contains(page, ">Define skills</a>")
  assert settings.update(loaded, settings.ShowSkills).0 == loaded
  assert string.contains(page, "Worker")
  assert string.contains(page, "Reviewer")
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

/// The tabs are listed in order, and each shows only its own sections.
pub fn tabs_show_their_own_sections_test() {
  let page = element.to_string(settings.view(settings.initial(True)))
  let position = fn(label) {
    let assert [before, _] = string.split(page, ">" <> label <> "</button>")
    string.length(before)
  }
  let section = fn(label) {
    let assert [before, _] = string.split(page, ">" <> label <> "</h2>")
    string.length(before)
  }
  assert int.compare(position("Agents"), position("Appearance"))
    == int.compare(0, 1)
  assert int.compare(position("Appearance"), position("Languages"))
    == int.compare(0, 1)
  assert int.compare(position("Languages"), position("About"))
    == int.compare(0, 1)
  let assert [_, agents, appearance, languages, about] =
    string.split(page, "<div aria-labelledby=\"tab-")
  assert string.contains(agents, "id=\"panel-agents\"")
  assert !string.contains(agents, " hidden")
  assert int.compare(section("Agent behavior"), section("Models"))
    == int.compare(0, 1)
  assert int.compare(section("Models"), section("Roles")) == int.compare(0, 1)
  assert int.compare(section("Roles"), section("Skills")) == int.compare(0, 1)
  assert string.contains(agents, "Agent behavior</h2>")
  assert string.contains(agents, "Roles</h2>")
  assert string.contains(agents, "Models</h2>")
  assert string.contains(agents, "Skills</h2>")
  assert !string.contains(agents, "Dark mode")
  let absent = fn(panel, headings) {
    list.each(headings, fn(heading) {
      assert !string.contains(panel, heading)
    })
  }
  absent(agents, [
    "Theme</h2>",
    "Enabled programming languages</h2>",
    "About</h2>",
  ])
  assert string.contains(appearance, " hidden")
  assert string.contains(appearance, "id=\"panel-appearance\"")
  assert string.contains(appearance, "Theme</h2>")
  assert string.contains(appearance, "Dark mode")
  absent(appearance, [
    "Agent behavior</h2>",
    "Roles</h2>",
    "Models</h2>",
    "Skills</h2>",
    "Enabled programming languages</h2>",
    "About</h2>",
  ])
  assert string.contains(languages, " hidden")
  assert string.contains(languages, "id=\"panel-languages\"")
  assert string.contains(languages, "Enabled programming languages</h2>")
  absent(languages, [
    "Agent behavior</h2>",
    "Roles</h2>",
    "Models</h2>",
    "Skills</h2>",
    "Theme</h2>",
    "Dark mode",
    "About</h2>",
  ])
  assert string.contains(about, " hidden")
  assert string.contains(about, "id=\"panel-about\"")
  assert string.contains(about, "About</h2>")
  assert !string.contains(about, "Software version")
  assert string.contains(about, "Sustainable Use License 1.0")
  absent(about, [
    "Agent behavior</h2>",
    "Roles</h2>",
    "Models</h2>",
    "Skills</h2>",
    "Theme</h2>",
    "Dark mode",
    "Enabled programming languages</h2>",
  ])
}

pub fn tab_names_round_trip_test() {
  assert settings.tab_from("") == settings.Agents
  assert settings.tab_from("?tab=appearance") == settings.Appearance
  assert settings.tab_from("?tab=languages") == settings.Languages
  assert settings.tab_from("?tab=about") == settings.About
  assert settings.tab_from("?tab=nope") == settings.Agents
  assert settings.tab_name(settings.Agents) == "agents"
  assert settings.tab_name(settings.Appearance) == "appearance"
  assert settings.tab_name(settings.Languages) == "languages"
  assert settings.tab_name(settings.About) == "about"
}

/// A refused change stays outside the panels, so choosing another tab keeps it.
pub fn a_refusal_shows_on_every_tab_test() {
  let refused =
    settings.update(settings.initial(False), settings.LanguagesLoaded("oops")).0
  let agents = element.to_string(settings.view(refused))
  let languages =
    settings.update(refused, settings.Select(settings.Languages)).0
    |> settings.view
    |> element.to_string
  let alert = fn(page) {
    let assert [_, tail] = string.split(page, "role=\"alert\"")
    tail
  }
  assert alert(agents) == ">Solace could not be reached</p></main>"
  assert alert(languages) == ">Solace could not be reached</p></main>"
}

/// Outside a browser registering reports the error instead of crashing.
pub fn register_outside_browser_test() {
  assert case settings.register() {
    Ok(_) -> False
    Error(_) -> True
  }
}

pub fn choosing_a_tab_tells_the_shell_test() {
  let agents = settings.initial(False)
  let #(languages, choosing) =
    settings.update(agents, settings.Select(settings.Languages))
  assert languages.tab == settings.Languages
  let #(_, same) =
    settings.update(languages, settings.Select(settings.Languages))
  let #(_, shown) =
    settings.update(languages, settings.TabShown(settings.Appearance))
  assert choosing != same
  assert same == shown
  effect.perform(
    choosing,
    fn(_) { Nil },
    fn(name, data) {
      assert name == "tab"
      assert json.to_string(data) == "\"languages\""
    },
    fn(_) { Nil },
    fn() { dynamic.nil() },
    fn(_, _) { Nil },
    fn(_, _) { Nil },
    fn(_) { Nil },
  )
}

/// The About tab shows the version the server sent, with the full license
/// closed under its own header.
pub fn about_shows_the_version_and_opens_the_license_test() {
  let before =
    settings.update(settings.initial(False), settings.Select(settings.About)).0
    |> settings.view
    |> element.to_string
  assert !string.contains(before, "Software version")
  assert string.contains(before, "<details class=\"about-license\">")
  let loaded =
    settings.update(
      settings.initial(False),
      settings.AboutLoaded("{\"version\":\"9.8.7\"}"),
    ).0
  assert loaded.version == "9.8.7"
  let page =
    settings.update(loaded, settings.Select(settings.About)).0
    |> settings.view
    |> element.to_string
  assert string.contains(page, "Software version: 9.8.7")
  assert string.contains(page, "<details class=\"about-license\">")
  assert string.contains(page, "<summary>Sustainable Use License 1.0</summary>")
  assert !string.contains(page, "<details class=\"about-license\" open")
  assert string.contains(page, "Copyright (c) 2026 Aaron Donoho")
  assert string.contains(page, "<h4>Limitations</h4>")
  assert string.contains(
    page,
    "The “licensor” is the entity offering these terms.",
  )
  assert string.contains(
    page,
    "“Trademark” means trademarks, service marks, and similar rights.",
  )
  assert string.contains(page, "similar rights.")
  let refused =
    settings.update(
      loaded,
      settings.AboutLoaded("{\"message\":\"E7001 version required\"}"),
    ).0
  assert refused.version == "9.8.7"
  assert refused.error == "E7001 version required"
}

/// Each role's card lists every skill; a managed skill's roles change on their
/// own, while a hand-written one is shown but cannot be ticked.
pub fn skills_assign_from_a_role_card_test() {
  let roles =
    settings.update(
      connected_page(),
      settings.RolesLoaded(
        "[{\"id\":\"worker\",\"name\":\"Worker\",\"purpose\":\"Makes changes.\"},{\"id\":\"reviewer\",\"name\":\"Reviewer\",\"purpose\":\"Checks changes.\"}]",
      ),
    ).0
  let loaded =
    settings.update(
      roles,
      settings.SkillsLoaded(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"],\"managed\":true,\"all_roles\":false},{\"name\":\"general\",\"description\":\"General rules.\",\"body\":\"Keep it simple.\",\"roles\":[],\"managed\":false,\"all_roles\":true}]",
      ),
    ).0
  let page = element.to_string(settings.view(loaded))
  assert string.contains(page, "role-skill")
  assert string.contains(page, "Edit Skills")
  assert !string.contains(page, "Define skills")
  assert string.contains(page, "value=\"house-review\"")
  assert string.contains(page, "value=\"general\"")
  assert string.contains(page, "disabled")
  let review =
    settings.Skill(
      "house-review",
      "Review UI changes.",
      "Check contrast.",
      [
        "reviewer",
      ],
      True,
      False,
    )
  assert settings.skill_roles_body(review, "worker", True)
    == "{\"original\":\"house-review\",\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"worker\",\"reviewer\"]}"
  assert settings.skill_roles_body(review, "reviewer", False)
    == "{\"original\":\"house-review\",\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[]}"
  // A tick shows straight away, so a second tick builds on it rather than
  // dropping the first. A reply changes only that skill's roles.
  let ticked =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  assert ticked.skills
    == [
      settings.Skill(
        "house-review",
        "Review UI changes.",
        "Check contrast.",
        [
          "worker",
          "reviewer",
        ],
        True,
        False,
      ),
      settings.Skill(
        "general",
        "General rules.",
        "Keep it simple.",
        [],
        False,
        True,
      ),
    ]
  let unticked =
    settings.update(
      ticked,
      settings.AssignSkill("house-review", "reviewer", False),
    ).0
  assert unticked.skills
    == [
      settings.Skill(
        "house-review",
        "Review UI changes.",
        "Check contrast.",
        [
          "worker",
        ],
        True,
        False,
      ),
      settings.Skill(
        "general",
        "General rules.",
        "Keep it simple.",
        [],
        False,
        True,
      ),
    ]
  // A refused tick restores the roles it had before.
  let refused =
    settings.update(ticked, settings.SkillTickSaved("{\"message\":\"No.\"}")).0
  assert refused.skills == loaded.skills
  assert refused.error == "No."
  let after =
    settings.update(
      ticked,
      settings.SkillTickSaved(
        "[{\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"worker\",\"reviewer\"],\"managed\":true,\"all_roles\":false},{\"name\":\"general\",\"description\":\"General rules.\",\"body\":\"Keep it simple.\",\"roles\":[],\"managed\":false,\"all_roles\":true}]",
      ),
    ).0
  assert after.skills
    == [
      settings.Skill(
        "house-review",
        "Review UI changes.",
        "Check contrast.",
        [
          "worker",
          "reviewer",
        ],
        True,
        False,
      ),
      settings.Skill(
        "general",
        "General rules.",
        "Keep it simple.",
        [],
        False,
        True,
      ),
    ]
  // A hand-written skill cannot be assigned, even if the message arrives.
  assert settings.update(
      after,
      settings.AssignSkill("general", "worker", False),
    ).0
    == after
}

fn house_review(roles: List(String)) -> settings.Skill {
  settings.Skill(
    "house-review",
    "Review UI changes.",
    "Check contrast.",
    roles,
    True,
    False,
  )
}

fn house_review_reply(roles: List(String)) -> String {
  "["
  <> json.to_string(
    json.object([
      #("name", json.string("house-review")),
      #("description", json.string("Review UI changes.")),
      #("body", json.string("Check contrast.")),
      #("roles", json.array(roles, of: json.string)),
      #("managed", json.bool(True)),
      #("all_roles", json.bool(False)),
    ]),
  )
  <> "]"
}

fn house_review_loaded() -> settings.Model {
  settings.update(
    connected_page(),
    settings.SkillsLoaded(house_review_reply([])),
  ).0
}

fn house_review_tick(role: String, chosen: Bool) -> skill_writes.SkillTick {
  skill_writes.SkillTick("house-review", role, chosen)
}

/// A refused tick goes back, while another tick still waiting keeps showing.
pub fn a_refused_tick_keeps_another_tick_waiting_test() {
  let loaded = house_review_loaded()
  let reviewer_tick = house_review_tick("reviewer", True)
  let worker =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let both =
    settings.update(
      worker,
      settings.AssignSkill("house-review", "reviewer", True),
    ).0
  let refused =
    settings.update(both, settings.SkillTickSaved("{\"message\":\"No.\"}")).0
  assert refused.skills == [house_review(["reviewer"])]
  assert refused.skills_writing == Some(skill_writes.TickWrite(reviewer_tick))
  assert refused.skills_waiting == []
  assert refused.error == "No."
}

/// Two ticks on one role, both refused, leave the role as the server had it.
pub fn a_refused_tick_chain_returns_to_the_start_test() {
  let loaded = house_review_loaded()
  let off_tick = house_review_tick("worker", False)
  let on =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let off =
    settings.update(on, settings.AssignSkill("house-review", "worker", False)).0
  assert off.skills == loaded.skills
  let refused_on =
    settings.update(off, settings.SkillTickSaved("{\"message\":\"No.\"}")).0
  assert refused_on.skills == loaded.skills
  assert refused_on.skills_writing == Some(skill_writes.TickWrite(off_tick))
  let refused_off =
    settings.update(
      refused_on,
      settings.SkillTickSaved("{\"message\":\"No.\"}"),
    ).0
  assert refused_off.skills == loaded.skills
  assert refused_off.skills_writing == None
  assert refused_off.skills_waiting == []
}

/// A second tick waits for the first reply, so the server keeps the chosen order.
pub fn a_second_tick_waits_for_the_first_reply_test() {
  let loaded = house_review_loaded()
  let on_tick = house_review_tick("worker", True)
  let off_tick = house_review_tick("worker", False)
  let on =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  assert on.skills_writing == Some(skill_writes.TickWrite(on_tick))
  assert on.skills_waiting == []
  let off =
    settings.update(on, settings.AssignSkill("house-review", "worker", False)).0
  assert off.skills_writing == Some(skill_writes.TickWrite(on_tick))
  assert off.skills_waiting == [skill_writes.TickWrite(off_tick)]
  assert off.skills == loaded.skills
  let accepted_on =
    settings.update(
      off,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  assert accepted_on.skills_server == [house_review(["worker"])]
  assert accepted_on.skills_writing == Some(skill_writes.TickWrite(off_tick))
  assert accepted_on.skills_waiting == []
  assert accepted_on.skills == [house_review([])]
  let accepted_off =
    settings.update(
      accepted_on,
      settings.SkillTickSaved(house_review_reply([])),
    ).0
  assert accepted_off.skills == [house_review([])]
  assert accepted_off.skills_writing == None
  assert accepted_off.skills_waiting == []
}

/// A tick queued behind a refusal is sent with the server's roles, not the refused one.
pub fn a_refused_queued_tick_is_not_resent_test() {
  let loaded = house_review_loaded()
  let reviewer_tick = house_review_tick("reviewer", True)
  let worker =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let both =
    settings.update(
      worker,
      settings.AssignSkill("house-review", "reviewer", True),
    ).0
  let refused =
    settings.update(both, settings.SkillTickSaved("{\"message\":\"No.\"}")).0
  assert refused.skills_writing == Some(skill_writes.TickWrite(reviewer_tick))
  assert settings.skill_tick_body(refused, reviewer_tick)
    == "{\"original\":\"house-review\",\"name\":\"house-review\",\"description\":\"Review UI changes.\",\"body\":\"Check contrast.\",\"roles\":[\"reviewer\"]}"
}

/// A successful tick reply clears its request and shows the server's roles.
pub fn a_confirmed_tick_clears_the_queue_test() {
  let loaded = house_review_loaded()
  let ticked =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let confirmed =
    settings.update(
      ticked,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  assert confirmed.skills == [house_review(["worker"])]
  assert confirmed.skills_writing == None
  assert confirmed.skills_waiting == []
}

/// A refused later tick leaves the server's accepted first tick in place.
pub fn a_refused_second_tick_keeps_the_confirmed_first_tick_test() {
  let loaded = house_review_loaded()
  let on =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let off =
    settings.update(on, settings.AssignSkill("house-review", "worker", False)).0
  let accepted_on =
    settings.update(
      off,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  assert accepted_on.skills == [house_review([])]
  let refused =
    settings.update(
      accepted_on,
      settings.SkillTickSaved("{\"message\":\"No.\"}"),
    ).0
  assert refused.skills == [house_review(["worker"])]
  assert refused.skills_writing == None
}

/// Reconnecting keeps the queue and puts the reload behind it.
pub fn a_reconnect_keeps_queued_skill_writes_test() {
  let loaded = house_review_loaded()
  let worker_tick = house_review_tick("worker", True)
  let reviewer_tick = house_review_tick("reviewer", True)
  let worker =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let queued =
    settings.update(
      worker,
      settings.AssignSkill("house-review", "reviewer", True),
    ).0
  assert queued.skills_writing == Some(skill_writes.TickWrite(worker_tick))
  assert queued.skills_waiting == [skill_writes.TickWrite(reviewer_tick)]
  let reconnected = settings.update(queued, settings.Connected).0
  assert reconnected.skills_writing == Some(skill_writes.TickWrite(worker_tick))
  assert reconnected.skills_waiting
    == [skill_writes.TickWrite(reviewer_tick), skill_writes.LoadWrite]
  let again = settings.update(reconnected, settings.Connected).0
  assert again.skills_waiting == reconnected.skills_waiting
  let worker_replied =
    settings.update(
      reconnected,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  assert worker_replied.skills_writing
    == Some(skill_writes.TickWrite(reviewer_tick))
  assert worker_replied.skills_waiting == [skill_writes.LoadWrite]
}

/// A write queued before a reattach shows its result once it is sent.
pub fn a_write_queued_before_a_reconnect_shows_its_reply_test() {
  let loaded = house_review_loaded()
  let reviewer_tick = house_review_tick("reviewer", True)
  let worker =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let queued =
    settings.update(
      worker,
      settings.AssignSkill("house-review", "reviewer", True),
    ).0
  let reconnected = settings.update(queued, settings.Connected).0
  assert reconnected.skills == [house_review(["reviewer", "worker"])]
  let worker_replied =
    settings.update(
      reconnected,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  assert worker_replied.skills == [house_review(["reviewer", "worker"])]
  assert worker_replied.skills_writing
    == Some(skill_writes.TickWrite(reviewer_tick))
  let reviewer_replied =
    settings.update(
      worker_replied,
      settings.SkillTickSaved(house_review_reply(["worker", "reviewer"])),
    ).0
  assert reviewer_replied.skills == [house_review(["worker", "reviewer"])]
  assert reviewer_replied.skills_writing == Some(skill_writes.LoadWrite)
  let refreshed =
    settings.update(
      reviewer_replied,
      settings.SkillsLoaded(house_review_reply(["worker", "reviewer"])),
    ).0
  assert refreshed.skills == [house_review(["worker", "reviewer"])]
  assert refreshed.skills_writing == None
}

/// A save queued before a reattach reports the server's refusal.
pub fn a_save_queued_before_a_reconnect_reports_its_refusal_test() {
  let loaded = house_review_loaded()
  let worker_tick = house_review_tick("worker", True)
  let worker =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let saving = settings.update(worker, settings.SaveSkill).0
  assert saving.skills_writing == Some(skill_writes.TickWrite(worker_tick))
  let assert [skill_writes.SaveWrite(_)] = saving.skills_waiting
  let reconnected = settings.update(saving, settings.Connected).0
  let worker_replied =
    settings.update(
      reconnected,
      settings.SkillTickSaved(house_review_reply(["worker"])),
    ).0
  let assert Some(skill_writes.SaveWrite(_)) = worker_replied.skills_writing
  assert worker_replied.skills_waiting == [skill_writes.LoadWrite]
  let refused =
    settings.update(
      worker_replied,
      settings.SkillsSaved("{\"message\":\"No.\"}"),
    ).0
  assert refused.error == "No."
  assert refused.skills_writing == Some(skill_writes.LoadWrite)
  assert refused.skills == [house_review(["worker"])]
}

/// A successful save reply shows the server's skills and empties the form.
pub fn a_saved_skill_clears_the_form_test() {
  let loaded = house_review_loaded()
  let editing = settings.update(loaded, settings.EditSkill(house_review([]))).0
  let saving = settings.update(editing, settings.SaveSkill).0
  let saved =
    settings.update(
      saving,
      settings.SkillsSaved(house_review_reply(["worker"])),
    ).0
  assert saving.skill_form == editing.skill_form
  assert saved.skills == [house_review(["worker"])]
  assert saved.skill_form == settings.blank_skill_form
  assert saved.error == ""
}

/// Reconnecting loads the server's current roles.
pub fn reconnecting_accepts_a_change_made_elsewhere_test() {
  let loaded = house_review_loaded()
  let on =
    settings.update(
      loaded,
      settings.AssignSkill("house-review", "worker", True),
    ).0
  let confirmed =
    settings.update(on, settings.SkillTickSaved(house_review_reply(["worker"]))).0
  let reconnected = settings.update(confirmed, settings.Connected).0
  assert reconnected.skills == [house_review(["worker"])]
  let elsewhere =
    settings.update(reconnected, settings.SkillsLoaded(house_review_reply([]))).0
  assert elsewhere.skills == [house_review([])]
  assert elsewhere.skills_writing == None
}

fn connected_page() -> settings.Model {
  settings.update(settings.initial(False), settings.Connected).0
}

/// A skills reply that no request asked for leaves the page as it was.
pub fn a_skills_reply_with_nothing_in_flight_is_ignored_test() {
  let loaded = house_review_loaded()
  let editing = settings.update(loaded, settings.EditSkill(house_review([]))).0
  let loaded_late =
    settings.update(
      editing,
      settings.SkillsLoaded(house_review_reply(["worker"])),
    ).0
  assert loaded_late == editing
  let saved_late =
    settings.update(
      editing,
      settings.SkillsSaved(house_review_reply(["worker"])),
    ).0
  assert saved_late == editing
}
