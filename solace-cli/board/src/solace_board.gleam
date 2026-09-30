import gleam/dict
import gleam/dynamic/decode
import gleam/float
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import lustre
import lustre/attribute as a
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/event as e

@external(javascript, "./api.mjs", "request")
fn request(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(receive: fn(String) -> Nil) -> Nil

pub type Column {
  Column(id: String, label: String, actions: List(String), final: Bool)
}

pub type Note {
  Note(author: String, text: String)
}

pub type Question {
  Question(id: String, text: String, options: List(String), answer: String)
}

pub type Run {
  Run(
    id: String,
    kind: String,
    state: String,
    model: String,
    log: String,
    outcome: String,
  )
}

pub type Link {
  Link(name: String, url: String)
}

pub type Task {
  Task(
    id: String,
    title: String,
    body: String,
    column: String,
    rank: Float,
    kind: String,
    paused: String,
    notes: List(Note),
    questions: List(Question),
    runs: List(Run),
    links: List(Link),
    changes: List(String),
    stamps: List(String),
    attention: String,
  )
}

pub type Agent {
  Agent(id: String, label: String, available: Bool, reason: String)
}

pub type Workflow {
  Workflow(columns: List(Column), types: List(String))
}

pub type Model {
  Model(
    workflow: Workflow,
    tasks: List(Task),
    selected: Option(Task),
    models: List(Agent),
    model: String,
    text: String,
    title: String,
    body: String,
    kind: String,
    error: String,
    rank: String,
  )
}

pub type Msg {
  WorkflowLoaded(String)
  TasksLoaded(String)
  ModelsLoaded(String)
  TaskLoaded(String)
  Changed(String)
  Live(String)
  Select(String)
  AgentSelected(String)
  Text(String)
  Title(String)
  Body(String)
  Kind(String)
  Rank(String)
  Create
  Save
  Action(String)
  AddNote
  Answer(String, String)
}

pub fn main() {
  let assert Ok(_) =
    lustre.start(lustre.application(init, update, view), "#app", Nil)
  Nil
}

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(path, "GET", "", fn(text) { send(msg(text)) })
  })
}

fn send(
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(path, method, json.to_string(json.object(fields)), fn(text) {
      dispatch(Changed(text))
    })
  })
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(
    Model(
      Workflow([], []),
      [],
      None,
      [],
      "",
      "",
      "",
      "",
      "implementation",
      "",
      "0",
    ),
    effect.batch([
      get("/workflow", WorkflowLoaded),
      get("/tasks", TasksLoaded),
      get("/models", ModelsLoaded),
      effect.from(fn(send) { events(fn(text) { send(Live(text)) }) }),
    ]),
  )
}

fn error(text: String) -> String {
  json.parse(text, decode.at(["message"], decode.string))
  |> result.unwrap("Invalid API response")
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    WorkflowLoaded(text) ->
      case json.parse(text, workflow_decoder()) {
        Ok(w) -> #(
          Model(
            ..model,
            workflow: w,
            kind: list.first(w.types) |> result.unwrap("implementation"),
            error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    TasksLoaded(text) ->
      case json.parse(text, decode.list(task_decoder())) {
        Ok(tasks) -> #(Model(..model, tasks: tasks, error: ""), effect.none())
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    ModelsLoaded(text) ->
      case json.parse(text, decode.list(agent_decoder())) {
        Ok(models) -> #(
          Model(
            ..model,
            models: models,
            model: list.find(models, fn(a) { a.available })
              |> result.map(fn(a) { a.id })
              |> result.unwrap(""),
          ),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    TaskLoaded(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> #(
          Model(
            ..model,
            selected: Some(task),
            title: task.title,
            body: task.body,
            rank: float.to_string(task.rank),
            error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    Changed(text) ->
      case json.parse(text, task_decoder()) {
        Ok(task) -> #(
          Model(
            ..model,
            selected: Some(task),
            title: task.title,
            body: task.body,
            rank: float.to_string(task.rank),
            text: "",
            error: "",
          ),
          get("/tasks", TasksLoaded),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    Live(text) ->
      case json.parse(text, decode.at(["task"], task_decoder())) {
        Ok(task) -> {
          let tasks = [
            task,
            ..list.filter(model.tasks, fn(t) { t.id != task.id })
          ]
          let selected = case model.selected {
            Some(t) if t.id == task.id -> Some(task)
            other -> other
          }
          #(Model(..model, tasks: tasks, selected: selected), effect.none())
        }
        Error(_) ->
          case json.parse(text, decode.at(["workflow"], workflow_decoder())) {
            Ok(w) -> #(Model(..model, workflow: w), effect.none())
            Error(_) -> #(model, effect.none())
          }
      }
    Select(id) -> #(Model(..model, text: ""), get("/tasks/" <> id, TaskLoaded))
    AgentSelected(id) -> #(Model(..model, model: id), effect.none())
    Text(text) -> #(Model(..model, text: text), effect.none())
    Title(text) -> #(Model(..model, title: text), effect.none())
    Body(text) -> #(Model(..model, body: text), effect.none())
    Rank(rank) -> #(Model(..model, rank: rank), effect.none())
    Kind(kind) -> #(Model(..model, kind: kind), effect.none())
    Create -> #(
      model,
      send("/tasks", "POST", [
        #("title", json.string(model.title)),
        #("body", json.string(model.body)),
        #("type", json.string(model.kind)),
      ]),
    )
    Save ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) ->
          case float.parse(model.rank) {
            Ok(rank) -> #(
              model,
              send("/tasks/" <> t.id, "PATCH", [
                #("title", json.string(model.title)),
                #("body", json.string(model.body)),
                #("rank", json.float(rank)),
              ]),
            )
            Error(_) -> #(
              Model(..model, error: "Rank must be a number"),
              effect.none(),
            )
          }
      }
    Action(name) ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> {
          let fields = [#("action", json.string(name))]
          let fields = case model.model {
            "" -> fields
            id -> [#("model", json.string(id)), ..fields]
          }
          let fields = case model.text {
            "" -> fields
            text -> [#("text", json.string(text)), ..fields]
          }
          #(model, send("/tasks/" <> t.id <> "/actions", "POST", fields))
        }
      }
    AddNote ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> #(
          model,
          send("/tasks/" <> t.id <> "/notes", "POST", [
            #("text", json.string(model.text)),
          ]),
        )
      }
    Answer(qid, option) ->
      case model.selected {
        None -> #(model, effect.none())
        Some(t) -> {
          let answer = case option {
            "" -> #("text", json.string(model.text))
            option -> #("option", json.string(option))
          }
          #(
            model,
            send("/tasks/" <> t.id <> "/questions/" <> qid, "POST", [answer]),
          )
        }
      }
  }
}

pub fn task_decoder() -> decode.Decoder(Task) {
  use id <- decode.field("id", decode.string)
  use title <- decode.field("title", decode.string)
  use body <- decode.field("body", decode.string)
  use column <- decode.field("column", decode.string)
  use rank <- decode.field("rank", decode.float)
  use kind <- decode.field("type", decode.string)
  use paused <- decode.optional_field(
    "paused_from",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use notes <- decode.optional_field("notes", [], decode.list(note_decoder()))
  use questions <- decode.optional_field(
    "questions",
    [],
    decode.list(question_decoder()),
  )
  use runs <- decode.optional_field("runs", [], decode.list(run_decoder()))
  use links <- decode.optional_field("links", [], decode.list(link_decoder()))
  use changes <- decode.optional_field(
    "changes",
    [],
    decode.list(decode.at(["name"], decode.string)),
  )
  use stamps <- decode.optional_field(
    "stamps",
    [],
    decode.dict(decode.string, decode.dynamic) |> decode.map(dict.keys),
  )
  use attention <- decode.optional_field(
    "attention",
    "",
    decode.one_of(decode.at(["reason"], decode.string), [decode.success("")]),
  )
  decode.success(Task(
    id,
    title,
    body,
    column,
    rank,
    kind,
    paused,
    notes,
    questions,
    runs,
    links,
    changes,
    stamps,
    attention,
  ))
}

fn note_decoder() -> decode.Decoder(Note) {
  use author <- decode.field("author", decode.string)
  use text <- decode.field("text", decode.string)
  decode.success(Note(author, text))
}

fn question_decoder() -> decode.Decoder(Question) {
  use id <- decode.field("id", decode.string)
  use text <- decode.field("text", decode.string)
  use options <- decode.field("options", decode.list(decode.string))
  use answer <- decode.optional_field("answer", "", decode.string)
  decode.success(Question(id, text, options, answer))
}

fn run_decoder() -> decode.Decoder(Run) {
  use id <- decode.field("id", decode.string)
  use kind <- decode.field("kind", decode.string)
  use state <- decode.field("state", decode.string)
  use model <- decode.field("model", decode.string)
  use log <- decode.field("log", decode.string)
  use outcome <- decode.optional_field("outcome", "", decode.string)
  decode.success(Run(id, kind, state, model, log, outcome))
}

fn link_decoder() -> decode.Decoder(Link) {
  use name <- decode.field("name", decode.string)
  use url <- decode.field("url", decode.string)
  decode.success(Link(name, url))
}

fn agent_decoder() -> decode.Decoder(Agent) {
  use id <- decode.field("id", decode.string)
  use label <- decode.field("label", decode.string)
  use available <- decode.optional_field("available", True, decode.bool)
  use reason <- decode.optional_field(
    "reason",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  decode.success(Agent(id, label, available, reason))
}

pub fn workflow_decoder() -> decode.Decoder(Workflow) {
  use columns <- decode.field("columns", decode.list(column_decoder()))
  use types <- decode.field("types", decode.dict(decode.string, decode.dynamic))
  use default <- decode.optional_field("default_type", "", decode.string)
  let types = case default {
    "" -> dict.keys(types)
    _ -> [default, ..list.filter(dict.keys(types), fn(t) { t != default })]
  }
  decode.success(Workflow(columns, types))
}

fn column_decoder() -> decode.Decoder(Column) {
  use id <- decode.field("id", decode.string)
  use label <- decode.optional_field("label", id, decode.string)
  use actions <- decode.optional_field(
    "actions",
    [],
    decode.list(decode.string),
  )
  use final <- decode.optional_field("final", False, decode.bool)
  decode.success(Column(id, label, actions, final))
}

pub fn actions(workflow: Workflow, task: Task) -> List(String) {
  case list.find(workflow.columns, fn(c) { c.id == task.column }) {
    Ok(c) if c.final -> []
    Ok(c) ->
      list.append(c.actions, case task.paused {
        "" -> ["pause"]
        _ -> ["resume"]
      })
    Error(_) -> []
  }
}

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

pub fn view(model: Model) -> Element(Msg) {
  h.main([], [
    h.h1([], [h.text("Solace tasks")]),
    h.p([], [h.a([a.href("/guide")], [h.text("Guide")])]),
    h.p([a.role("alert")], [h.text(model.error)]),
    h.section([a.class("editor")], [
      h.label([], [
        h.text("Title"),
        h.input([a.value(model.title), e.on_input(Title)]),
      ]),
      h.label([], [
        h.text("Description"),
        h.textarea([a.value(model.body), e.on_input(Body)], model.body),
      ]),
      h.label([], [
        h.text("Task type"),
        h.select(
          [a.value(model.kind), e.on_input(Kind)],
          list.map(model.workflow.types, fn(t) { h.option([a.value(t)], t) }),
        ),
      ]),
      h.label([], [
        h.text("Rank (selected task)"),
        h.input([a.type_("number"), a.value(model.rank), e.on_input(Rank)]),
      ]),
      button("Create task", Create),
      button("Save selected task", Save),
    ]),
    h.div(
      [a.class("board")],
      list.map(model.workflow.columns, fn(c) {
        let tasks =
          model.tasks
          |> list.filter(fn(t) { t.column == c.id })
          |> list.sort(fn(a, b) {
            case a.rank <. b.rank {
              True -> order.Lt
              False -> order.Gt
            }
          })
        h.section([a.class("column")], [
          h.h2([], [h.text(c.label)]),
          ..list.map(tasks, fn(t) {
            h.button([a.class("card"), e.on_click(Select(t.id))], [
              case t.attention {
                "" -> h.text("")
                _ -> h.strong([a.class("attention")], [h.text("Needs attention ")])
              },
              h.text(t.title),
              h.small([], [
                h.text(
                  t.kind
                  <> case t.paused {
                    "" -> ""
                    _ -> " · paused"
                  },
                ),
              ]),
            ])
          })
        ])
      }),
    ),
    case model.selected {
      None -> h.text("Select a task to see its details.")
      Some(t) -> detail(model, t)
    },
  ])
}

fn detail(model: Model, t: Task) -> Element(Msg) {
  h.section([a.class("detail")], [
    h.h2([], [h.text(t.title)]),
    h.pre([], [h.text(t.body)]),
    h.p([], [h.text(t.column <> " · " <> t.kind)]),
    case t.attention {
      "" -> h.text("")
      reason ->
        h.p([a.class("attention"), a.role("status")], [
          h.text("Needs attention: " <> reason),
        ])
    },
    h.div(
      [],
      list.map(t.links, fn(link) {
        h.a([a.href(link.url)], [h.text(link.name)])
      }),
    ),
    h.p(
      [],
      list.map(t.stamps, fn(s) { h.span([], [h.text("Stamp: " <> s <> " ")]) }),
    ),
    h.label([], [
      h.text("Agent model"),
      h.select(
        [a.value(model.model), e.on_input(AgentSelected)],
        list.map(model.models, fn(m) {
          case m.available {
            True -> h.option([a.value(m.id)], m.label)
            False ->
              h.option(
                [a.value(m.id), a.disabled(True)],
                m.label <> " (unavailable: " <> m.reason <> ")",
              )
          }
        }),
      ),
    ]),
    h.label([], [
      h.text("Note / answer / instructions"),
      h.textarea([a.value(model.text), e.on_input(Text)], model.text),
    ]),
    button("Add note", AddNote),
    h.div(
      [a.class("actions")],
      list.map(actions(model.workflow, t), fn(name) {
        button(name, Action(name))
      }),
    ),
    h.h3([], [h.text("Questions")]),
    h.div(
      [],
      list.map(t.questions, fn(q) {
        h.article([], [
          h.p([], [h.text(q.text)]),
          case q.answer {
            "" ->
              h.div(
                [],
                list.append(
                  list.map(q.options, fn(o) { button(o, Answer(q.id, o)) }),
                  [button("Answer with text above", Answer(q.id, ""))],
                ),
              )
            answer -> h.p([], [h.text("Answer: " <> answer)])
          },
        ])
      }),
    ),
    h.h3([], [h.text("Notes")]),
    h.ol(
      [],
      list.map(t.notes, fn(n) {
        h.li([], [h.strong([], [h.text(n.author <> ": ")]), h.text(n.text)])
      }),
    ),
    h.h3([], [h.text("Runs")]),
    h.ul(
      [],
      list.map(t.runs, fn(r) {
        h.li([], [
          h.text(
            r.kind <> " · " <> r.model <> " · " <> r.state <> " " <> r.outcome,
          ),
          h.small([], [h.text(r.log)]),
        ])
      }),
    ),
    h.h3([], [h.text("Changes")]),
    h.ul([], list.map(t.changes, fn(c) { h.li([], [h.text(c)]) })),
  ])
}
