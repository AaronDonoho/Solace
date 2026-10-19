import gleam/list
import gleam/option.{Some}
import gleam/string
import gleeunit
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
