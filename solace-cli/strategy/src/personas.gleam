//// The Personas tab (D-216): who the software is for, or who works on it.

import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import solace_task

/// A persona as the stream carries it. Only `id` is required.
pub type Persona {
  Persona(
    id: String,
    name: String,
    objectives: String,
    tools: String,
    concerns: String,
    other: String,
    revision: String,
  )
}

pub fn decoder() -> decode.Decoder(Persona) {
  use id <- decode.field("id", decode.string)
  use name <- decode.optional_field("name", "", decode.string)
  use objectives <- decode.optional_field("objectives", "", decode.string)
  use tools <- decode.optional_field("tools", "", decode.string)
  use concerns <- decode.optional_field("concerns", "", decode.string)
  use other <- decode.optional_field("other", "", decode.string)
  use revision <- decode.optional_field("revision", "", decode.string)
  decode.success(Persona(id, name, objectives, tools, concerns, other, revision))
}

/// A persona being written: `id` is None for a new one, and `base` is the
/// revision the edit began from.
pub type Draft {
  Draft(
    id: Option(String),
    name: String,
    objectives: String,
    tools: String,
    concerns: String,
    other: String,
    base: String,
  )
}

/// One of a persona's Markdown fields.
pub type Field {
  ObjectivesField
  ToolsField
  ConcernsField
  OtherField
}

/// The persona form's state and what it sends.
pub type Editor(msg) {
  Editor(
    draft: Option(Draft),
    /// A save or delete sent and not yet answered.
    saving: Bool,
    error: String,
    on_new: msg,
    on_edit: fn(String) -> msg,
    on_cancel: msg,
    on_name: fn(String) -> msg,
    on_text: fn(Field, String) -> msg,
    on_format: fn(Field, String, String) -> msg,
    on_block: fn(Field, String) -> msg,
    on_save: msg,
    on_reload: msg,
    deleting: Option(String),
    on_delete: fn(String) -> msg,
    on_confirm_delete: msg,
    on_cancel_delete: msg,
  )
}

/// The most characters one persona may hold in all its fields together
/// (D-216); the server refuses more.
pub const limit = 1000

pub fn new_draft() -> Draft {
  Draft(None, "", "", "", "", "", "")
}

pub fn draft_of(persona: Persona) -> Draft {
  Draft(
    Some(persona.id),
    persona.name,
    persona.objectives,
    persona.tools,
    persona.concerns,
    persona.other,
    persona.revision,
  )
}

pub fn field_text(draft: Draft, field: Field) -> String {
  case field {
    ObjectivesField -> draft.objectives
    ToolsField -> draft.tools
    ConcernsField -> draft.concerns
    OtherField -> draft.other
  }
}

pub fn set_field(draft: Draft, field: Field, text: String) -> Draft {
  case field {
    ObjectivesField -> Draft(..draft, objectives: text)
    ToolsField -> Draft(..draft, tools: text)
    ConcernsField -> Draft(..draft, concerns: text)
    OtherField -> Draft(..draft, other: text)
  }
}

pub fn field_id(field: Field) -> String {
  case field {
    ObjectivesField -> "persona-objectives"
    ToolsField -> "persona-tools"
    ConcernsField -> "persona-concerns"
    OtherField -> "persona-other"
  }
}

pub fn field_label(field: Field) -> String {
  case field {
    ObjectivesField -> "Objectives"
    ToolsField -> "Tools"
    ConcernsField -> "Concerns"
    OtherField -> "Other"
  }
}

pub fn field_placeholder(field: Field) -> String {
  case field {
    ObjectivesField -> "What does this persona want to accomplish?"
    ToolsField ->
      "What capabilities, resources, or means can the persona use to pursue their objectives?"
    ConcernsField ->
      "What risks, constraints, trade-offs, or undesirable outcomes matter to this persona?"
    OtherField ->
      "Additional context that helps explain this persona's perspective, reasoning, or behavior."
  }
}

/// The persona's size as the server counts it: Unicode code points across
/// every field.
pub fn count(draft: Draft) -> Int {
  string.to_utf_codepoints(
    draft.name
    <> draft.objectives
    <> draft.tools
    <> draft.concerns
    <> draft.other,
  )
  |> list.length
}

pub fn count_text(draft: Draft) -> String {
  int.to_string(count(draft))
  <> " / "
  <> int.to_string(limit)
  <> " character limit"
}

/// Whether a save may be sent: the name is not blank and the fields fit.
pub fn valid(draft: Draft) -> Bool {
  string.trim(draft.name) != "" && count(draft) <= limit
}

fn fields(draft: Draft) -> List(#(String, json.Json)) {
  [
    #("name", json.string(draft.name)),
    #("objectives", json.string(draft.objectives)),
    #("tools", json.string(draft.tools)),
    #("concerns", json.string(draft.concerns)),
    #("other", json.string(draft.other)),
  ]
}

/// The body of `POST /personas`.
pub fn create_body(draft: Draft) -> json.Json {
  json.object(fields(draft))
}

/// The body of `PUT /personas/{id}`, over the revision the edit began from.
pub fn replace_body(draft: Draft) -> json.Json {
  json.object(list.append(fields(draft), [#("base", json.string(draft.base))]))
}

/// Why the open draft cannot be saved over the saved persona, "" when it can.
pub fn stale_text(draft: Draft, personas: List(Persona)) -> String {
  case draft.id {
    None -> ""
    Some(id) ->
      case list.find(personas, fn(persona) { persona.id == id }) {
        Ok(persona) ->
          case persona.revision == draft.base {
            True -> ""
            False ->
              "This persona changed since you began editing, so this draft can't be saved over it. Reload takes the saved persona; your draft is lost."
          }
        Error(_) ->
          "This persona was removed since you began editing. Reload closes this form."
      }
  }
}

/// The Personas tab: a form while one is open, then every persona, in the
/// server's order.
pub fn view(
  base: String,
  personas: List(Persona),
  editor: Editor(msg),
) -> List(Element(msg)) {
  let alert = case editor.draft, editor.error {
    None, message if message != "" -> [
      html.p([attribute.role("alert")], [html.text(message)]),
    ]
    _, _ -> []
  }
  let open = case editor.draft {
    None -> []
    Some(draft) -> [form(base, personas, editor, draft)]
  }
  let saved = case personas {
    [] -> [
      html.p([], [
        html.text(
          "No personas yet. Describe who the software is for, or who works on it.",
        ),
      ]),
    ]
    personas ->
      list.map(personas, fn(persona) { persona_view(base, persona, editor) })
  }
  list.flatten([
    alert,
    [
      html.div([attribute.class("vision-actions")], [
        button(
          solace_task.ui_label("new_persona"),
          editor.on_new,
          editor.draft != None || editor.saving,
        ),
      ]),
    ],
    open,
    saved,
  ])
}

fn form(
  base: String,
  personas: List(Persona),
  editor: Editor(msg),
  draft: Draft,
) -> Element(msg) {
  let over = count(draft) > limit
  let stale = stale_text(draft, personas)
  html.div(
    [attribute.class("feature-form persona-form"), attribute.id("persona-form")],
    [
      html.h3([], [
        html.text(case draft.id {
          None -> solace_task.ui_label("new_persona")
          Some(_) -> solace_task.ui_label("edit")
        }),
      ]),
      html.div([attribute.class("filters")], [
        html.label([], [
          html.text("Name"),
          html.input([
            attribute.type_("text"),
            attribute.value(draft.name),
            attribute.attribute("placeholder", "Name"),
            event.on_input(editor.on_name),
          ]),
        ]),
      ]),
      field_editor(base, editor, draft, ObjectivesField),
      field_editor(base, editor, draft, ToolsField),
      field_editor(base, editor, draft, ConcernsField),
      field_editor(base, editor, draft, OtherField),
      html.p(
        [
          attribute.class("vision-count"),
          case over {
            True -> attribute.role("alert")
            False -> attribute.attribute("aria-live", "polite")
          },
        ],
        [html.text(count_text(draft))],
      ),
      case stale {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      },
      case editor.error {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      },
      html.div([attribute.class("vision-actions")], [
        button(
          solace_task.ui_label("save"),
          editor.on_save,
          editor.saving || !valid(draft),
        ),
        button(solace_task.ui_label("cancel"), editor.on_cancel, editor.saving),
        ..case stale {
          "" -> []
          _ -> [button("Reload", editor.on_reload, editor.saving)]
        }
      ]),
    ],
  )
}

fn field_editor(
  base: String,
  editor: Editor(msg),
  draft: Draft,
  field: Field,
) -> Element(msg) {
  solace_task.editor(
    solace_task.EditorConfig(
      id: field_id(field),
      task: "",
      base: base,
      label: field_label(field),
      placeholder: field_placeholder(field),
      text: field_text(draft, field),
      on_input: fn(text) { editor.on_text(field, text) },
      on_format: fn(before, after) { editor.on_format(field, before, after) },
      on_block: fn(kind) { editor.on_block(field, kind) },
      actions: [],
    ),
  )
}

fn persona_view(
  base: String,
  persona: Persona,
  editor: Editor(msg),
) -> Element(msg) {
  let busy = editor.draft != None || editor.saving
  let shown =
    markdown_fields()
    |> list.filter_map(fn(field) {
      case persona_field(persona, field) {
        "" -> Error(Nil)
        text ->
          Ok(
            html.div([attribute.class("persona-field")], [
              html.h4([], [html.text(field_label(field))]),
              solace_task.md(base, text),
            ]),
          )
      }
    })
  html.section([attribute.class("persona")], [
    html.h3([], [
      html.text(persona.name),
      button(solace_task.ui_label("edit"), editor.on_edit(persona.id), busy),
      button(solace_task.ui_label("delete"), editor.on_delete(persona.id), busy),
    ]),
    case editor.deleting == Some(persona.id), editor.saving {
      True, False ->
        html.div([attribute.class("vision-actions")], [
          html.p([], [
            html.text("Delete "),
            html.em([], [html.text(persona.name)]),
            html.text("? This can't be undone."),
          ]),
          button(
            solace_task.ui_label("delete"),
            editor.on_confirm_delete,
            False,
          ),
          button(solace_task.ui_label("cancel"), editor.on_cancel_delete, False),
        ])
      _, _ -> element.none()
    },
    ..shown
  ])
}

/// The Personas tab's state: the saved list and the open form.
pub type State {
  State(
    personas: List(Persona),
    draft: Option(Draft),
    /// A save or delete sent and not yet answered.
    saving: Bool,
    error: String,
    /// The persona whose delete is being confirmed.
    deleting: Option(String),
  )
}

/// What the page sends after a change; the page owns the HTTP call.
pub type Request {
  Idle
  SaveRequest(Draft)
  DeleteRequest(String)
}

/// A change to the Personas tab.
pub type Msg {
  New
  Edit(String)
  Cancel
  Reload
  Name(String)
  Text(Field, String)
  Save
  /// The reply to a save, or why the save was refused.
  Saved(Result(Persona, String))
  Delete(String)
  CancelDelete
  ConfirmDelete
  /// The reply to a delete, or why the delete was refused.
  Deleted(Result(Persona, String))
}

pub fn blank() -> State {
  State([], None, False, "", None)
}

/// The persona whose delete is being confirmed, when it is still saved.
pub fn confirming(state: State) -> Option(String) {
  case state.deleting {
    Some(id) ->
      case list.any(state.personas, fn(persona) { persona.id == id }) {
        True -> Some(id)
        False -> None
      }
    None -> None
  }
}

pub fn persona_field(persona: Persona, field: Field) -> String {
  case field {
    ObjectivesField -> persona.objectives
    ToolsField -> persona.tools
    ConcernsField -> persona.concerns
    OtherField -> persona.other
  }
}

/// The Markdown fields, in the order the form shows them.
pub fn markdown_fields() -> List(Field) {
  [ObjectivesField, ToolsField, ConcernsField, OtherField]
}

pub fn update(state: State, msg: Msg) -> #(State, Request) {
  case msg {
    New ->
      case state.draft == None && !state.saving {
        True -> #(State(..state, draft: Some(new_draft()), error: ""), Idle)
        False -> #(state, Idle)
      }
    Edit(id) ->
      case state.draft == None && !state.saving {
        True -> #(open(state, id), Idle)
        False -> #(state, Idle)
      }
    Cancel -> #(State(..state, draft: None, error: "", deleting: None), Idle)
    Reload -> #(reload(state), Idle)
    Name(name) -> #(edit(state, fn(draft) { Draft(..draft, name: name) }), Idle)
    Text(field, text) -> #(
      edit(state, fn(draft) { set_field(draft, field, text) }),
      Idle,
    )
    Save -> begin_save(state)
    Saved(reply) -> #(saved(state, reply), Idle)
    Delete(id) ->
      case state.draft == None && !state.saving {
        True -> #(State(..state, deleting: Some(id), error: ""), Idle)
        False -> #(state, Idle)
      }
    CancelDelete ->
      case !state.saving {
        True -> #(State(..state, deleting: None, error: ""), Idle)
        False -> #(state, Idle)
      }
    ConfirmDelete -> begin_delete(state)
    Deleted(reply) -> #(deleted(state, reply), Idle)
  }
}

fn open(state: State, id: String) -> State {
  case list.find(state.personas, fn(persona) { persona.id == id }) {
    Ok(persona) -> State(..state, draft: Some(draft_of(persona)), error: "")
    Error(_) -> state
  }
}

fn reload(state: State) -> State {
  case state.draft {
    Some(draft) ->
      State(
        ..state,
        draft: case draft.id {
          None -> Some(new_draft())
          Some(id) ->
            case list.find(state.personas, fn(persona) { persona.id == id }) {
              Ok(persona) -> Some(draft_of(persona))
              Error(_) -> None
            }
        },
        error: "",
      )
    None -> state
  }
}

fn edit(state: State, change: fn(Draft) -> Draft) -> State {
  case state.draft {
    Some(draft) -> State(..state, draft: Some(change(draft)))
    None -> state
  }
}

fn begin_save(state: State) -> #(State, Request) {
  case state.draft, state.saving {
    Some(draft), False ->
      case valid(draft) {
        True -> #(State(..state, saving: True, error: ""), SaveRequest(draft))
        False -> #(state, Idle)
      }
    _, _ -> #(state, Idle)
  }
}

fn saved(state: State, reply: Result(Persona, String)) -> State {
  case reply {
    Ok(persona) ->
      State(
        personas: keep(state.personas, persona),
        draft: None,
        saving: False,
        deleting: None,
        error: "",
      )
    Error(message) -> State(..state, saving: False, error: message)
  }
}

fn begin_delete(state: State) -> #(State, Request) {
  case state.deleting, state.saving {
    Some(id), False -> #(
      State(..state, saving: True, error: ""),
      DeleteRequest(id),
    )
    _, _ -> #(state, Idle)
  }
}

fn deleted(state: State, reply: Result(Persona, String)) -> State {
  case reply {
    Ok(persona) ->
      State(
        ..state,
        personas: list.filter(state.personas, fn(p) { p.id != persona.id }),
        deleting: None,
        saving: False,
        error: "",
      )
    Error(message) -> State(..state, saving: False, error: message)
  }
}

fn keep(personas: List(Persona), persona: Persona) -> List(Persona) {
  case list.any(personas, fn(p) { p.id == persona.id }) {
    True ->
      list.map(personas, fn(p) {
        case p.id == persona.id {
          True -> persona
          False -> p
        }
      })
    False -> list.append(personas, [persona])
  }
}

fn button(label: String, msg: msg, disabled: Bool) -> Element(msg) {
  html.button(
    [
      attribute.type_("button"),
      attribute.disabled(disabled),
      event.on_click(msg),
    ],
    [html.text(label)],
  )
}
