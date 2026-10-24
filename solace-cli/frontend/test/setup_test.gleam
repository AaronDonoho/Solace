import frontend
import gleam/list
import gleam/string
import lustre/element
import setup

pub fn the_checklist_lists_only_missing_tools_test() {
  let reply =
    "{\"platform\":\"arch\",\"items\":[{\"id\":\"git\",\"label\":\"Git\",\"ok\":false,\"why\":\"Git keeps project history.\",\"steps\":[{\"text\":\"sudo pacman -S git\",\"command\":true}]},{\"id\":\"gleam\",\"label\":\"Gleam\",\"ok\":true,\"why\":\"\",\"steps\":[]}]}"
  assert setup.decode(reply)
    == Ok([
      setup.Item("git", "Git", False, "Git keeps project history.", [
        setup.Step("sudo pacman -S git", True),
      ]),
    ])
  assert setup.decode("not json") == Error(Nil)
}

pub fn the_checklist_is_hidden_when_every_tool_passes_test() {
  let page = element.to_string(setup.view([], frontend.CheckSetup))
  assert string.contains(page, "hidden")
  assert !string.contains(page, "Recommended setup")
}

pub fn the_checklist_names_the_tool_why_and_install_step_test() {
  let item =
    setup.Item("gleam", "Gleam", False, "Gleam builds your projects.", [
      setup.Step("sudo pacman -S gleam", True),
    ])
  let page = element.to_string(setup.view([item], frontend.CheckSetup))
  assert string.contains(page, "class=\"setup\"")
  assert string.contains(page, "Recommended setup")
  assert string.contains(page, "Gleam builds your projects.")
  assert string.contains(page, "sudo pacman -S gleam")
  assert string.contains(page, "<code>sudo pacman -S gleam</code>")
  assert string.contains(page, "Check again")
}

pub fn a_check_again_reply_shows_and_then_hides_the_checklist_test() {
  let item =
    setup.Item("git", "Git", False, "Git keeps history.", [
      setup.Step("sudo apt install git", True),
    ])
  let model =
    frontend.Model(
      frontend.Board,
      [frontend.Board],
      frontend.no_focus,
      [],
      [],
      0,
      [item],
    )
  assert string.contains(
    element.to_string(frontend.view(model)),
    "Recommended setup",
  )
  let missing =
    "{\"items\":[{\"id\":\"git\",\"label\":\"Git\",\"ok\":false,\"why\":\"Git keeps history.\",\"steps\":[{\"text\":\"sudo apt install git\",\"command\":true}]}]}"
  let #(model, _) = frontend.update(model, frontend.SetupLoaded(missing))
  assert list.length(model.setup) == 1
  let #(model, _) =
    frontend.update(model, frontend.SetupLoaded("{\"items\":[]}"))
  assert model.setup == []
  assert !string.contains(
    element.to_string(frontend.view(model)),
    "Recommended setup",
  )
}

pub fn a_broken_reply_keeps_the_checklist_shown_test() {
  let item =
    setup.Item("git", "Git", False, "Git keeps history.", [
      setup.Step("sudo apt install git", True),
    ])
  let model =
    frontend.Model(
      frontend.Board,
      [frontend.Board],
      frontend.no_focus,
      [],
      [],
      0,
      [item],
    )
  let #(model, _) = frontend.update(model, frontend.SetupLoaded("not json"))
  assert model.setup == [item]
}

pub fn a_later_failure_shows_the_checklist_again_test() {
  let model =
    frontend.Model(
      frontend.Board,
      [frontend.Board],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  assert !string.contains(
    element.to_string(frontend.view(model)),
    "Recommended setup",
  )
  let missing =
    "{\"items\":[{\"id\":\"git\",\"label\":\"Git\",\"ok\":false,\"why\":\"Git keeps history.\",\"steps\":[{\"text\":\"sudo apt install git\",\"command\":true}]}]}"
  let #(model, _) = frontend.update(model, frontend.SetupLoaded(missing))
  assert list.length(model.setup) == 1
  assert string.contains(
    element.to_string(frontend.view(model)),
    "Recommended setup",
  )
}

pub fn the_checklist_separates_explanations_from_commands_test() {
  let item =
    setup.Item("agent", "An agent", False, "No agent is signed in.", [
      setup.Step("curl -fsSL https://claude.ai/install.sh | bash", True),
      setup.Step("Then run it once and sign in.", False),
    ])
  let page = element.to_string(setup.view([item], frontend.CheckSetup))
  assert string.contains(
    page,
    "<code>curl -fsSL https://claude.ai/install.sh | bash</code>",
  )
  assert string.contains(page, "Then run it once and sign in.")
  assert !string.contains(page, "<code>Then run it once and sign in.</code>")
}
