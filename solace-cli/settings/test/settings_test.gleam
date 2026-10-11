import gleam/option.{None, Some}
import gleam/string
import gleeunit
import lustre/element
import settings

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
  let page =
    element.to_string(
      settings.view(settings.Model(dark: True, unsubscribe: None)),
    )
  assert string.contains(page, "<h1>Settings</h1>")
  assert string.contains(page, "Appearance</h2>")
  assert string.contains(page, element.to_string(settings.theme_control(True)))
}

/// The switch and a change from another window both decide whether it shows on.
pub fn the_switch_and_other_windows_set_the_theme_test() {
  let off = settings.Model(dark: False, unsubscribe: None)
  let on = settings.Model(dark: True, unsubscribe: None)
  assert settings.update(off, settings.SetDark(True)).0 == on
  assert settings.update(on, settings.SetDark(False)).0 == off
  assert settings.update(off, settings.ThemeChanged("dark")).0 == on
  assert settings.update(on, settings.ThemeChanged("light")).0 == off
}

/// Connecting starts listening once; disconnecting stops it.
pub fn listening_follows_the_element_in_and_out_of_the_page_test() {
  let stop = fn() { Nil }
  let off = settings.Model(dark: False, unsubscribe: None)
  let listening = settings.update(off, settings.Subscribed(stop)).0
  assert listening == settings.Model(dark: False, unsubscribe: Some(stop))
  // Connected again while listening keeps the one subscription.
  assert settings.update(listening, settings.Connected).0 == listening
  assert settings.update(listening, settings.Disconnected).0 == off
  assert settings.update(off, settings.Disconnected).0 == off
}
