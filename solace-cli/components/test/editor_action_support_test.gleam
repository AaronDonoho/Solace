// Test helpers for the markdown editor's composer actions.
import gleam/list
import gleam/string
import lustre/dev/query
import lustre/dev/simulate
import lustre/element
import solace_task

/// The composer action's whole button, from its opening tag through the
/// closing one.
pub fn action_button(rendered: String) -> String {
  let assert [before, after, ..] =
    string.split(rendered, "class=\"composer-action\"")
  let assert Ok(start) = string.split(before, "<button") |> list.last
  let assert [end, ..] = string.split(after, "</button>")
  "<button" <> start <> "class=\"composer-action\"" <> end <> "</button>"
}

/// An editor with `actions`, rendered.
pub fn rendered(actions: List(solace_task.EditorAction(Nil))) -> String {
  solace_task.editor(solace_task.EditorConfig(
    id: "body",
    task: "t1",
    base: "/p/a",
    label: "Description",
    text: "",
    on_input: fn(_) { Nil },
    on_format: fn(_, _) { Nil },
    on_block: fn(_) { Nil },
    actions: actions,
  ))
  |> element.to_string
}

/// A click on the composer's action: the model after it, and the messages it
/// sent.
pub fn click_composer(
  model: solace_task.Model,
) -> #(solace_task.Model, List(solace_task.Msg)) {
  simulate.simple(
    init: fn(_) { #(model, []) },
    update: fn(state, message) {
      let #(model, _) = solace_task.update(state.0, message)
      #(model, [message, ..state.1])
    },
    view: fn(state) { solace_task.view(state.0) },
  )
  |> simulate.start(Nil)
  |> simulate.click(on: query.element(matching: query.class("composer-action")))
  |> simulate.model
}
