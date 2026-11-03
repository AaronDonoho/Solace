//// The Personas tab (D-216): its records, form, saves and deletes.

import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import lustre/element
import personas
import strategy

fn step(model: strategy.Model, msg: strategy.Msg) -> strategy.Model {
  strategy.update(model, msg).0
}

/// A caught-up page with the root's own project.
fn page(tab: strategy.Tab) -> strategy.Model {
  strategy.new(tab)
  |> step(strategy.ProjectsLoaded(
    "{\"workspace\":false,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"root\",\"base\":\"\"}],\"found\":[]}",
  ))
  |> step(strategy.CaughtUp)
}

fn persona(id: String, name: String, revision: String) -> personas.Persona {
  personas.Persona(id, name, "Ship it", "Code", "Risk", "More", revision)
}

fn persona_json(id: String, name: String, revision: String) -> String {
  "{\"id\":\""
  <> id
  <> "\",\"name\":\""
  <> name
  <> "\",\"objectives\":\"Ship it\",\"tools\":\"Code\",\"concerns\":\"Risk\",\"other\":\"More\",\"revision\":\""
  <> revision
  <> "\"}"
}

pub fn the_decoder_reads_every_field_and_defaults_absent_ones_test() {
  assert json.parse(persona_json("p1", "Ada", "r1"), personas.decoder())
    == Ok(persona("p1", "Ada", "r1"))
  assert json.parse("{\"id\":\"p2\"}", personas.decoder())
    == Ok(personas.Persona("p2", "", "", "", "", "", ""))
}

pub fn the_create_and_replace_bodies_test() {
  let draft = personas.draft_of(persona("p1", "Ada", "r1"))
  assert json.to_string(personas.create_body(draft))
    == "{\"name\":\"Ada\",\"objectives\":\"Ship it\",\"tools\":\"Code\",\"concerns\":\"Risk\",\"other\":\"More\"}"
  assert json.to_string(personas.replace_body(draft))
    == "{\"name\":\"Ada\",\"objectives\":\"Ship it\",\"tools\":\"Code\",\"concerns\":\"Risk\",\"other\":\"More\",\"base\":\"r1\"}"
}

pub fn the_count_covers_every_field_and_holds_an_empty_name_test() {
  let draft =
    personas.Draft(..personas.new_draft(), name: "Ada", objectives: "Ship")
  assert personas.count(draft) == 7
  assert personas.count_text(draft) == "7 / 1000 character limit"
  assert personas.valid(draft)
  assert !personas.valid(personas.new_draft())
  assert !personas.valid(personas.Draft(..draft, name: "  "))
  let long = personas.Draft(..draft, objectives: string.repeat("a", 1001))
  assert personas.count(long) > personas.limit
  assert !personas.valid(long)
}

pub fn the_form_names_each_field_and_shows_its_hint_test() {
  let model = step(page(strategy.Personas), strategy.NewPersona)
  let html = element.to_string(strategy.view(model))
  assert string.contains(html, "placeholder=\"Name\"")
  assert string.contains(html, "id=\"persona-objectives\"")
  assert string.contains(html, "id=\"persona-tools\"")
  assert string.contains(html, "id=\"persona-concerns\"")
  assert string.contains(html, "id=\"persona-other\"")
  assert string.contains(
    html,
    "placeholder=\"What does this persona want to accomplish?\"",
  )
  assert string.contains(
    html,
    "placeholder=\"What capabilities, resources, or means can the persona use to pursue their objectives?\"",
  )
  assert string.contains(
    html,
    "placeholder=\"What risks, constraints, trade-offs, or undesirable outcomes matter to this persona?\"",
  )
  assert string.contains(
    html,
    "placeholder=\"Additional context that helps explain this persona&#39;s perspective, reasoning, or behavior.\"",
  )
  assert string.contains(html, "0 / 1000 character limit")
}

pub fn a_draft_keeps_its_base_and_offers_reload_test() {
  let model =
    page(strategy.Personas)
    |> step(strategy.Live(
      "{\"personas\":[" <> persona_json("p1", "Ada", "r1") <> "]}",
    ))
    |> step(strategy.EditPersona("p1"))
    |> step(strategy.PersonaName("Grace"))
    |> step(strategy.Live(
      "{\"personas\":[" <> persona_json("p1", "Ada Lovelace", "r2") <> "]}",
    ))
  assert model.persona_state.draft
    == Some(personas.Draft(
      Some("p1"),
      "Grace",
      "Ship it",
      "Code",
      "Risk",
      "More",
      "r1",
    ))
  let html = element.to_string(strategy.view(model))
  assert string.contains(html, "This persona changed since you began editing")
  assert string.contains(html, ">Reload<")
  let model = step(model, strategy.ReloadPersona)
  assert model.persona_state.draft
    == Some(personas.Draft(
      Some("p1"),
      "Ada Lovelace",
      "Ship it",
      "Code",
      "Risk",
      "More",
      "r2",
    ))
}

pub fn a_save_names_its_base_and_its_reply_ends_the_edit_test() {
  let model =
    page(strategy.Personas)
    |> step(strategy.Live(
      "{\"personas\":[" <> persona_json("p1", "Ada", "r1") <> "]}",
    ))
    |> step(strategy.EditPersona("p1"))
    |> step(strategy.PersonaText(personas.ObjectivesField, "Ship faster"))
    |> step(strategy.SavePersona)
  assert model.persona_state.saving
  let refused =
    step(
      model,
      strategy.PersonaSaved(
        "{\"code\":\"E7028\",\"message\":\"E7028 the persona changed\"}",
      ),
    )
  assert refused.persona_state.draft != None
  assert refused.persona_state.error == "E7028 the persona changed"
  assert list.length(string.split(
      element.to_string(strategy.view(refused)),
      "E7028 the persona changed",
    ))
    == 2
  let saved =
    step(model, strategy.PersonaSaved(persona_json("p1", "Ada", "r2")))
  assert saved.persona_state.draft == None
  assert saved.persona_state.saving == False
  assert saved.persona_state.personas
    == [personas.Persona("p1", "Ada", "Ship it", "Code", "Risk", "More", "r2")]
  // A new persona needs a name before Save does anything.
  let fresh = step(saved, strategy.NewPersona)
  let blank = step(fresh, strategy.SavePersona)
  assert blank.persona_state.saving == False
  let named =
    fresh
    |> step(strategy.PersonaName("Grace"))
    |> step(strategy.SavePersona)
  assert named.persona_state.saving
  let created =
    step(named, strategy.PersonaSaved(persona_json("p2", "Grace", "r1")))
  assert list.length(created.persona_state.personas) == 2
}

pub fn a_delete_asks_first_and_the_reply_removes_the_persona_test() {
  let model =
    page(strategy.Personas)
    |> step(strategy.Live(
      "{\"personas\":[" <> persona_json("p1", "Ada", "r1") <> "]}",
    ))
  let asking = step(model, strategy.DeletePersona("p1"))
  assert string.contains(
    element.to_string(strategy.view(asking)),
    "Delete <em>Ada</em>? This can&#39;t be undone.",
  )
  let sent = step(asking, strategy.ConfirmDeletePersona)
  assert sent.persona_state.saving
  let refused =
    step(
      sent,
      strategy.PersonaDeleted(
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown persona\"}",
      ),
    )
  assert refused.persona_state.deleting == Some("p1")
  assert refused.persona_state.error == "E7023 unknown persona"
  assert list.length(string.split(
      element.to_string(strategy.view(refused)),
      "E7023 unknown persona",
    ))
    == 2
  let deleted =
    step(sent, strategy.PersonaDeleted(persona_json("p1", "Ada", "r1")))
  assert deleted.persona_state.personas == []
  assert deleted.persona_state.deleting == None
  assert deleted.persona_state.saving == False
}

pub fn the_form_keeps_one_draft_and_one_save_test() {
  let ada = persona("p1", "Ada", "r1")
  let state = personas.State([ada], None, False, "", None)
  let #(editing, _) = personas.update(state, personas.Edit("p1"))
  assert editing.draft == Some(personas.draft_of(ada))
  let #(unknown, _) = personas.update(state, personas.Edit("p2"))
  assert unknown == state
  let #(still_editing, _) = personas.update(editing, personas.New)
  assert still_editing == editing
  let #(typed, _) = personas.update(editing, personas.Name("Grace"))
  assert typed.draft
    == Some(personas.Draft(
      Some("p1"),
      "Grace",
      "Ship it",
      "Code",
      "Risk",
      "More",
      "r1",
    ))
  let #(saving, request) = personas.update(editing, personas.Save)
  assert request == personas.SaveRequest(personas.draft_of(ada))
  assert saving.saving
  let #(blocked, request) = personas.update(saving, personas.Save)
  assert request == personas.Idle
  assert blocked == saving
  let #(still_saving, _) = personas.update(saving, personas.Edit("p1"))
  assert still_saving == saving
  let #(blank, _) = personas.update(state, personas.New)
  let #(unsaved, request) = personas.update(blank, personas.Save)
  assert request == personas.Idle
  assert !unsaved.saving
}

pub fn a_delete_asks_and_a_removed_persona_closes_its_form_test() {
  let ada = persona("p1", "Ada", "r1")
  let state = personas.State([ada], None, False, "", None)
  let #(editing, _) = personas.update(state, personas.Edit("p1"))
  let #(delete_blocked, _) = personas.update(editing, personas.Delete("p1"))
  assert delete_blocked == editing
  let #(asking, _) = personas.update(state, personas.Delete("p1"))
  assert asking.deleting == Some("p1")
  assert personas.confirming(asking) == Some("p1")
  let #(cancelled, _) = personas.update(asking, personas.CancelDelete)
  assert cancelled.deleting == None
  let #(nothing, request) = personas.update(state, personas.ConfirmDelete)
  assert request == personas.Idle
  assert nothing == state
  let #(deleting, request) = personas.update(asking, personas.ConfirmDelete)
  assert request == personas.DeleteRequest("p1")
  assert deleting.saving
  let #(still_asking, _) = personas.update(deleting, personas.CancelDelete)
  assert still_asking == deleting
  assert personas.confirming(personas.State(..asking, personas: [])) == None
  let #(removed, _) =
    personas.update(personas.State(..editing, personas: []), personas.Reload)
  assert removed.draft == None
  let #(fresh, _) = personas.update(state, personas.New)
  let #(reloaded, _) = personas.update(fresh, personas.Reload)
  assert reloaded.draft == Some(personas.new_draft())
}
