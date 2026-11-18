//// The Design page's agent panel: its runs, conversation, model pick and
//// width (D-240). Every design run covers the whole page, so the page passes
//// no context of its own.

import gleam/dict.{type Dict}
import gleam/dynamic.{type Dynamic}
import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import lustre/attribute
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import plan_panel
import render
import solace_task

/// The panel's state: the runs, the one shown, its transcript, the message box
/// and the model pick.
pub type Model {
  Model(
    runs: List(plan_panel.Run),
    proposals: List(Proposed),
    open: String,
    collapsed: Bool,
    width: Int,
    text: String,
    /// Notes waiting to go with the next message, in the order they were added.
    notes: List(Note),
    /// The text in the note box beside the selected elements.
    note_text: String,
    /// The message a successful start or send clears, None when none.
    clearing: Option(String),
    /// The notes a successful start or send clears, None when none.
    clearing_notes: Option(List(Note)),
    sending: Bool,
    error: String,
    entries: List(solace_task.Entry),
    poll: Int,
    at_bottom: Bool,
    new_message: Bool,
    jump: Bool,
    connected: Bool,
    models: List(solace_task.Agent),
    role_models: List(solace_task.RoleModel),
    model: String,
    effort: String,
    source: solace_task.ModelSource,
    refreshing: Bool,
    root: Dynamic,
  )
}

/// The panel's messages.
pub type Msg {
  Collapse
  Drag(Int)
  Width(Int)
  WidthDone
  Key(String)
  Text(String)
  NoteText(String)
  AddNote(String, List(String))
  RemoveNote(Int)
  Format(String, String)
  Block(String)
  ModelPicked(String)
  Effort(String)
  ModelsLoaded(String)
  RoleModelsLoaded(String)
  ModelsRefreshed(String)
  TryAgain
  Start
  Send
  Answer(String, String)
  End
  Replied(Bool, String)
  Show(String)
  Loaded(Int, String, String)
  Refresh(Int, String)
  Scrolled(Bool)
  Newest
  Attached(Dynamic)
}

/// The Design page's words, draft and run address.
pub fn settings() -> plan_panel.Settings {
  plan_panel.Settings(
    id: "design-plan",
    address: "/design/runs",
    text_id: "design-text",
    draft_owner: "design:",
    labels: labels(),
  )
}

fn labels() -> plan_panel.Labels {
  plan_panel.Labels(
    title: "Design",
    context: "Design context",
    whole: "This design",
    open_here: "A design run on this is open.",
    marker: "Design",
    marker_title: "A design run is open on this",
    run_open: "A design run on this is open",
    resize: "Resize the design panel",
    earlier: "Earlier runs",
    new_message: "New message ↓",
  )
}

pub fn new() -> Model {
  Model(
    runs: [],
    proposals: [],
    open: "",
    collapsed: False,
    width: stored_width(),
    text: plan_panel.load_draft(settings()),
    notes: [],
    note_text: "",
    clearing: None,
    clearing_notes: None,
    sending: False,
    error: "",
    entries: [],
    poll: 0,
    at_bottom: True,
    new_message: False,
    jump: False,
    connected: False,
    models: [],
    role_models: [],
    model: "",
    effort: "",
    source: solace_task.RoleDefault,
    refreshing: False,
    root: no_root(),
  )
}

pub fn connect(model: Model) -> Model {
  Model(..model, connected: True)
}

pub fn disconnected(model: Model) -> Model {
  Model(
    ..model,
    connected: False,
    poll: model.poll + 1,
    clearing: None,
    clearing_notes: None,
  )
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Collapse -> #(Model(..model, collapsed: !model.collapsed), effect.none())
    Drag(x) -> #(
      model,
      effect.from(fn(send) {
        drag(x, model.width, fn(w) { send(Width(w)) }, fn() { send(WidthDone) })
      }),
    )
    Width(width) -> #(
      Model(..model, width: plan_panel.clamp(width, window_width())),
      effect.none(),
    )
    WidthDone -> #(model, store_width(model.width))
    // The handle is the panel's left edge: Left widens it.
    Key(key) -> {
      let window = window_width()
      let width =
        case key {
          "ArrowLeft" -> model.width + plan_panel.key_step
          "ArrowRight" -> model.width - plan_panel.key_step
          "Home" -> plan_panel.min_width
          "End" -> plan_panel.max_width(window)
          _ -> model.width
        }
        |> plan_panel.clamp(window)
      #(Model(..model, width: width), store_width(width))
    }
    Text(text) -> {
      plan_panel.save_draft(settings(), text)
      #(Model(..model, text: text), effect.none())
    }
    NoteText(text) -> #(Model(..model, note_text: text), effect.none())
    AddNote(design, elements) -> add_note(model, design, elements)
    RemoveNote(index) -> #(
      Model(..model, notes: remove_at(model.notes, index)),
      effect.none(),
    )
    Format(before, after) -> #(
      model,
      effect.from(fn(send) {
        send(
          Text(solace_task.wrap(
            model.root,
            settings().text_id,
            model.text,
            before,
            after,
          )),
        )
      }),
    )
    Block(kind) -> #(
      model,
      effect.from(fn(send) {
        send(
          Text(solace_task.block(
            model.root,
            settings().text_id,
            model.text,
            kind,
          )),
        )
      }),
    )
    // Efforts differ between models, so a new model starts at its default.
    ModelPicked(id) -> #(
      Model(..model, model: id, effort: "", source: solace_task.Human),
      effect.none(),
    )
    Effort(effort) -> #(Model(..model, effort: effort), effect.none())
    TryAgain if model.refreshing -> #(model, effect.none())
    TryAgain -> #(
      Model(..model, refreshing: True),
      effect.from(fn(send) {
        request(
          "",
          "/models/try-again",
          "POST",
          json.to_string(json.object([])),
          fn(text) { send(ModelsLoaded(text)) },
        )
      }),
    )
    ModelsLoaded(text) -> #(models_loaded(model, text), effect.none())
    ModelsRefreshed(text) -> #(models_refreshed(model, text), effect.none())
    RoleModelsLoaded(text) -> #(
      case json.parse(text, solace_task.role_models_decoder()) {
        Ok(role_models) ->
          choose_model(Model(..model, role_models: role_models))
        Error(_) -> model
      },
      effect.none(),
    )
    Start -> start(model)
    Send -> send(model)
    Answer(qid, option) -> answer(model, qid, option)
    End -> finish(model)
    // The reply's run is shown, and its transcript fetched again.
    Replied(clears, text) ->
      case json.parse(text, plan_panel.decoder()) {
        Ok(run) -> {
          let sent_text = case model.clearing {
            Some(sent_text) -> sent_text
            None -> ""
          }
          let clear_text = clears && model.text == sent_text
          let sent_notes = case model.clearing_notes {
            Some(notes) -> notes
            None -> []
          }
          case clear_text {
            True -> plan_panel.clear_draft(settings())
            False -> Nil
          }
          let was_open = model.open
          let next =
            Model(
              ..model,
              runs: keep(model.runs, run),
              open: run.id,
              sending: False,
              error: "",
              text: case clear_text {
                True -> ""
                False -> model.text
              },
              clearing: None,
              clearing_notes: None,
              notes: case clears {
                True -> without_all(model.notes, sent_notes)
                False -> model.notes
              },
              jump: model.jump || run.id != was_open,
            )
          let next = case json.parse(text, proposed_run_decoder()) {
            Ok(#(rid, proposals)) -> with_proposals(next, rid, proposals)
            Error(_) -> next
          }
          track(model, fetch(next, run.id))
        }
        Error(_) -> #(
          Model(
            ..model,
            sending: False,
            error: error(text),
            clearing: None,
            clearing_notes: None,
          ),
          effect.none(),
        )
      }
    Show(rid) -> {
      let next =
        Model(
          ..model,
          open: rid,
          entries: [],
          error: "",
          collapsed: False,
          at_bottom: True,
          new_message: False,
          jump: rid != "",
        )
      case rid {
        "" -> #(Model(..next, poll: next.poll + 1), effect.none())
        _ -> fetch(next, rid)
      }
    }
    // While the agent works its transcript is fetched again, in one chain.
    Loaded(poll, rid, text) if poll == model.poll && rid == model.open -> {
      let decoder = {
        use run <- decode.then(plan_panel.decoder())
        use entries <- decode.field(
          "transcript",
          decode.list(solace_task.entry_decoder()),
        )
        decode.success(#(run, entries))
      }
      case json.parse(text, decoder) {
        Ok(#(run, entries)) -> {
          let next =
            Model(
              ..model,
              runs: keep(model.runs, run),
              entries: entries,
              jump: False,
            )
          let next = case json.parse(text, proposed_run_decoder()) {
            Ok(#(rid, proposals)) -> with_proposals(next, rid, proposals)
            Error(_) -> next
          }
          let poll_fx = case run.state == "running" && model.connected {
            True ->
              effect.from(fn(send) {
                after(refresh_ms, fn() { send(Refresh(poll, rid)) })
              })
            False -> effect.none()
          }
          case model.jump {
            True -> #(
              Model(..next, at_bottom: True, new_message: False),
              effect.batch([poll_fx, scroll()]),
            )
            False -> track(model, #(next, poll_fx))
          }
        }
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    }
    Loaded(_, _, _) -> #(model, effect.none())
    Refresh(poll, rid) if poll == model.poll && rid == model.open -> #(
      model,
      get_run(poll, rid),
    )
    Refresh(_, _) -> #(model, effect.none())
    Scrolled(near) -> #(
      Model(..model, at_bottom: near, new_message: model.new_message && !near),
      effect.none(),
    )
    Newest -> #(Model(..model, at_bottom: True, new_message: False), scroll())
    Attached(root) -> #(Model(..model, root: root), effect.none())
  }
}

/// Shows the run shown in the panel, if any.
pub fn shown(model: Model) -> Option(plan_panel.Run) {
  list.find(model.runs, fn(r) { r.id == model.open })
  |> option.from_result
}

/// Applies the run part of one `/events` message, and says whether it was a
/// design run, whose model list is refreshed.
pub fn live(model: Model, event: Dynamic) -> #(Model, Bool) {
  case at(event, "designRuns", decode.list(plan_panel.decoder())) {
    Ok(runs) -> {
      let model =
        Model(
          ..model,
          runs: list.map(runs, fn(r) {
            case list.find(model.runs, fn(old) { old.id == r.id }) {
              Ok(old) -> plan_panel.Run(..r, notes: old.notes)
              Error(_) -> r
            }
          }),
        )
      let model = case
        at(event, "designRuns", decode.list(proposed_run_decoder()))
      {
        Ok(proposed) -> Model(..model, proposals: all_proposals(proposed))
        Error(_) -> model
      }
      #(model, True)
    }
    Error(_) ->
      case at(event, "designRun", plan_panel.decoder()) {
        Ok(run) -> {
          let model = Model(..model, runs: keep(model.runs, run))
          let model = case at(event, "designRun", proposed_run_decoder()) {
            Ok(#(rid, proposals)) -> with_proposals(model, rid, proposals)
            Error(_) -> model
          }
          #(model, True)
        }
        Error(_) ->
          case at(event, "roleModels", solace_task.role_models_decoder()) {
            Ok(role_models) -> #(
              choose_model(Model(..model, role_models: role_models)),
              False,
            )
            Error(_) -> #(model, False)
          }
      }
  }
}

/// Whether a live change of the design runs needs the human, which flags the
/// Design tab while another page shows: a run went waiting, so its agent
/// replied, or a run asked a new proposal.
pub fn needs_attention(
  old: List(plan_panel.Run),
  new: List(plan_panel.Run),
) -> Bool {
  list.any(new, fn(r) {
    case list.find(old, fn(o) { o.id == r.id }) {
      Ok(o) ->
        { r.state == "waiting" && o.state != "waiting" }
        || list.any(r.questions, fn(q) {
          q.answer == "" && !list.any(o.questions, fn(p) { p.id == q.id })
        })
      Error(_) ->
        r.state == "waiting" || list.any(r.questions, fn(q) { q.answer == "" })
    }
  })
}

/// A live change of the shown run's state fetches its transcript again,
/// which polls while the agent works.
pub fn follow(old: Model, new: Model) -> #(Model, Effect(Msg)) {
  case shown(old), shown(new) {
    Some(a), Some(b) if a.state != b.state -> fetch(new, b.id)
    _, _ -> #(new, effect.none())
  }
}

/// Asks for the shown run's transcript in a new polling chain.
pub fn resume(model: Model) -> #(Model, Effect(Msg)) {
  case model.open {
    "" -> #(model, effect.none())
    rid -> fetch(model, rid)
  }
}

pub fn view(model: Model) -> Element(Msg) {
  plan_panel.view_with_notes(
    "",
    settings(),
    plan_panel.Panel(
      runs: model.runs,
      context: plan_panel.page,
      shown: shown(model),
      entries: model.entries,
      collapsed: model.collapsed,
      width: model.width,
      max_width: plan_panel.max_width(window_width()),
      text: model.text,
      sending: model.sending,
      error: model.error,
      refreshing: model.refreshing,
      models: model.models,
      model: model.model,
      effort: model.effort,
      name: fn(_, _) { "" },
      on_collapse: Collapse,
      on_drag: Drag,
      on_key: Key,
      on_text: Text,
      on_format: Format,
      on_block: Block,
      on_model: ModelPicked,
      on_effort: Effort,
      on_try_again: TryAgain,
      on_start: Start,
      on_send: Send,
      on_answer: Answer,
      on_show: Show,
      on_end: End,
      at_bottom: model.at_bottom,
      new_message: model.new_message,
      on_scroll: Scrolled,
      on_newest: Newest,
    ),
    notes_view(model),
  )
}

fn start(model: Model) -> #(Model, Effect(Msg)) {
  case string.trim(model.text) {
    "" -> #(model, effect.none())
    _ ->
      send_request(
        model,
        plan_panel.runs_path(settings()),
        plan_panel.start_body_with_notes(
          model.model,
          model.effort,
          model.text,
          plan_panel.page,
          list.map(model.notes, note_json),
        ),
        True,
      )
  }
}

fn send(model: Model) -> #(Model, Effect(Msg)) {
  case shown(model), string.trim(model.text) {
    Some(run), text if text != "" && run.state != "ended" ->
      send_request(
        model,
        plan_panel.run_path(settings(), run.id) <> "/notes",
        send_body(model.text, model.notes),
        True,
      )
    _, _ -> #(model, effect.none())
  }
}

fn answer(model: Model, qid: String, option: String) -> #(Model, Effect(Msg)) {
  case shown(model) {
    Some(run) ->
      send_request(
        model,
        plan_panel.run_path(settings(), run.id) <> "/questions/" <> qid,
        json.object([#("option", json.string(option))]),
        False,
      )
    None -> #(model, effect.none())
  }
}

fn finish(model: Model) -> #(Model, Effect(Msg)) {
  case shown(model) {
    Some(run) ->
      send_request(
        model,
        plan_panel.run_path(settings(), run.id) <> "/end",
        json.object([]),
        False,
      )
    None -> #(model, effect.none())
  }
}

/// One run request at a time; its reply is `Replied`.
fn send_request(
  model: Model,
  path: String,
  body: json.Json,
  clears: Bool,
) -> #(Model, Effect(Msg)) {
  case model.sending {
    True -> #(model, effect.none())
    False -> #(
      Model(
        ..model,
        sending: True,
        error: "",
        clearing: case clears {
          True -> Some(model.text)
          False -> None
        },
        clearing_notes: case clears {
          True -> Some(model.notes)
          False -> None
        },
      ),
      post(path, body, fn(reply) { Replied(clears, reply) }),
    )
  }
}

fn fetch(model: Model, rid: String) -> #(Model, Effect(Msg)) {
  let poll = model.poll + 1
  #(Model(..model, poll: poll), get_run(poll, rid))
}

fn get_run(poll: Int, rid: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request("", plan_panel.run_path(settings(), rid), "GET", "", fn(text) {
      send(Loaded(poll, rid, text))
    })
  })
}

fn post(path: String, body: json.Json, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request("", path, "POST", json.to_string(body), fn(text) { send(msg(text)) })
  })
}

/// A new message or proposal in the shown run scrolls to it when the human was
/// at the run's end; else "New message ↓" shows, with no jump. Transcript
/// entries, drawn above the conversation, keep the human at the end when they
/// were there, with no button.
fn track(old: Model, step: #(Model, Effect(Msg))) -> #(Model, Effect(Msg)) {
  let #(next, fx) = step
  let grew = list.length(next.entries) > list.length(old.entries)
  case shown(old), shown(next) {
    Some(a), Some(b) if a.id == b.id && !next.jump ->
      case plan_panel.size(b) > plan_panel.size(a), next.at_bottom {
        True, True -> #(next, effect.batch([fx, scroll()]))
        True, False -> #(Model(..next, new_message: True), fx)
        False, True if grew -> #(next, effect.batch([fx, scroll()]))
        False, _ -> step
      }
    _, _ -> step
  }
}

fn scroll() -> Effect(Msg) {
  effect.after_paint(fn(_, root) { scroll_plan(root) })
}

fn models_loaded(model: Model, text: String) -> Model {
  case json.parse(text, decode.list(solace_task.agent_decoder())) {
    Ok(models) -> keep_choice(Model(..model, models: models, refreshing: False))
    Error(_) -> Model(..model, refreshing: False)
  }
}

fn models_refreshed(model: Model, text: String) -> Model {
  case json.parse(text, decode.list(solace_task.agent_decoder())) {
    Ok(models) -> keep_choice(Model(..model, models: models))
    Error(_) -> model
  }
}

fn keep_choice(model: Model) -> Model {
  case model.source {
    solace_task.Human ->
      case
        list.find(model.models, fn(a) { a.id == model.model && a.available })
      {
        Ok(a) ->
          Model(..model, effort: solace_task.kept_effort(a, model.effort))
        Error(_) ->
          choose_model(Model(..model, source: solace_task.RoleDefault))
      }
    solace_task.RoleDefault -> choose_model(model)
  }
}

fn choose_model(model: Model) -> Model {
  case model.source {
    solace_task.Human -> model
    solace_task.RoleDefault -> {
      let id =
        solace_task.role_model_chosen(
          model.models,
          model.role_models,
          "designer",
        )
      let effort = case id == model.model {
        True -> solace_task.effort_for(model.models, id, model.effort)
        False -> ""
      }
      Model(..model, model: id, effort: effort, source: solace_task.RoleDefault)
    }
  }
}

fn keep(
  runs: List(plan_panel.Run),
  run: plan_panel.Run,
) -> List(plan_panel.Run) {
  case list.any(runs, fn(r) { r.id == run.id }) {
    True ->
      list.map(runs, fn(r) {
        case r.id == run.id {
          True -> run
          False -> r
        }
      })
    False -> list.append(runs, [run])
  }
}

fn at(
  event: Dynamic,
  key: String,
  decoder: decode.Decoder(a),
) -> Result(a, Nil) {
  decode.run(event, decode.at([key], decoder))
  |> result.replace_error(Nil)
}

fn error(text: String) -> String {
  case json.parse(text, decode.at(["message"], decode.string)) {
    Ok(message) -> message
    Error(_) -> text
  }
}

/// Where the browser keeps the panel's width.
const width_key = "solace-design-plan-width"

fn stored_width() -> Int {
  load(width_key)
  |> int.parse
  |> result.unwrap(plan_panel.default_width)
  |> plan_panel.clamp(window_width())
}

fn store_width(width: Int) -> Effect(Msg) {
  effect.from(fn(_) { store(width_key, int.to_string(width)) })
}

/// While the shown run's agent works, its transcript is fetched again this often.
const refresh_ms = 1500

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "load")
fn load(key: String) -> String

@external(javascript, "./api.mjs", "store")
fn store(key: String, value: String) -> Nil

@external(javascript, "./api.mjs", "window_width")
fn window_width() -> Int

@external(javascript, "./api.mjs", "drag")
fn drag(x: Int, width: Int, move: fn(Int) -> Nil, done: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "scroll_plan")
fn scroll_plan(root: Dynamic) -> Nil

@external(javascript, "./api.mjs", "no_root")
fn no_root() -> Dynamic

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request("", path, "GET", "", fn(text) { send(msg(text)) })
  })
}

/// The models the pick offers, asked again so one that became available or
/// unavailable since is seen. The first reply opens the pick.
pub fn refresh_models(model: Model) -> Effect(Msg) {
  case model.models {
    [] -> get("/models", ModelsLoaded)
    _ -> get("/models", ModelsRefreshed)
  }
}

/// The models the pick offers and each role's default, for a new connection.
pub fn load_models(model: Model) -> Effect(Msg) {
  effect.batch([
    refresh_models(model),
    get("/role-models", RoleModelsLoaded),
  ])
}

/// Whether a live update is a design run, whose model list is refreshed so a
/// config edit reaches the pick.
pub fn models_event(text: String) -> Bool {
  case json.parse(text, decode.dynamic) {
    Ok(event) ->
      ["designRun", "designRuns"]
      |> list.any(fn(key) { result.is_ok(at(event, key, decode.dynamic)) })
    Error(_) -> False
  }
}

/// A design proposal the panel decoded from a run: the design it would save,
/// and the tab a new design would join.
pub type Proposed {
  Proposed(run: String, tab: String, design: render.Design)
}

fn all_proposals(proposed: List(#(String, List(Proposed)))) -> List(Proposed) {
  list.flatten(list.map(proposed, fn(pair) { pair.1 }))
}

fn with_proposals(
  model: Model,
  run: String,
  proposals: List(Proposed),
) -> Model {
  Model(
    ..model,
    proposals: list.append(
      list.filter(model.proposals, fn(proposed) { proposed.run != run }),
      proposals,
    ),
  )
}

/// The open design proposals in one run, decoded from the run's own JSON.
fn proposed_run_decoder() -> decode.Decoder(#(String, List(Proposed))) {
  use run <- decode.field("id", decode.string)
  use state <- decode.optional_field(
    "state",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use questions <- decode.optional_field(
    "questions",
    [],
    decode.list(question_proposal_decoder(run)),
  )
  let proposals =
    list.filter_map(questions, fn(question) {
      case question {
        Some(proposed) -> Ok(proposed)
        None -> Error(Nil)
      }
    })
  case state {
    "ended" -> decode.success(#(run, []))
    _ -> decode.success(#(run, proposals))
  }
}

fn question_proposal_decoder(run: String) -> decode.Decoder(Option(Proposed)) {
  use answer <- decode.optional_field(
    "answer",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use proposal <- decode.optional_field(
    "proposal",
    None,
    design_proposal_decoder(),
  )
  case answer, proposal {
    "", Some(#(tab, design)) -> decode.success(Some(Proposed(run, tab, design)))
    _, _ -> decode.success(None)
  }
}

fn design_proposal_decoder() -> decode.Decoder(Option(#(String, render.Design))) {
  use kind <- decode.field("kind", decode.string)
  case kind {
    "design" -> {
      use tab <- decode.optional_field(
        "tab",
        "",
        decode.one_of(decode.string, [decode.success("")]),
      )
      use design <- decode.optional_field(
        "design",
        None,
        decode.optional(render.design_decoder()),
      )
      case design {
        Some(design) -> decode.success(Some(#(tab, design)))
        None -> decode.success(None)
      }
    }
    _ -> decode.success(None)
  }
}

/// A note waiting to go with the next message: the design it points into, the
/// elements it points at, and what the human wrote.
pub type Note {
  Note(design: String, elements: List(String), text: String)
}

/// The panel numbers each noted element shows on the open design, in the
/// panel's order and comma-separated; notes on other designs are left out.
pub fn markers(model: Model, design: String) -> Dict(String, String) {
  let #(_, markers) =
    list.fold(model.notes, #(1, dict.new()), fn(state, note) {
      let #(number, markers) = state
      let markers = case note.design == design {
        True ->
          list.fold(note.elements, markers, fn(markers, id) {
            let label = int.to_string(number)
            dict.insert(markers, id, case dict.get(markers, id) {
              Ok(existing) -> existing <> ", " <> label
              Error(_) -> label
            })
          })
        False -> markers
      }
      #(number + 1, markers)
    })
  markers
}

/// The waiting notes, in the order they will be sent.
pub fn notes_view(model: Model) -> Element(Msg) {
  case model.notes {
    [] -> element.none()
    notes -> html.ul([attribute.class("design-notes")], note_rows(notes))
  }
}

fn note_rows(notes: List(Note)) -> List(Element(Msg)) {
  let #(rows, _) =
    list.fold(notes, #([], 0), fn(state, note) {
      let #(rows, index) = state
      #([note_row(index, note), ..rows], index + 1)
    })
  list.reverse(rows)
}

fn note_row(index: Int, note: Note) -> Element(Msg) {
  html.li([attribute.class("design-note-row")], [
    html.span([attribute.class("design-note-number")], [
      html.text(int.to_string(index + 1)),
    ]),
    html.span([attribute.class("design-note-text")], [
      html.text(note.text),
    ]),
    html.span([attribute.class("design-note-elements")], [
      html.text(string.join(note.elements, ", ")),
    ]),
    html.button(
      [
        attribute.type_("button"),
        attribute.class("design-note-remove"),
        attribute.attribute("aria-label", "Remove note"),
        event.on_click(RemoveNote(index)),
      ],
      [html.text("×")],
    ),
  ])
}

/// The body of a message with the notes it carries.
pub fn send_body(text: String, notes: List(Note)) -> json.Json {
  json.object(
    list.flatten([
      [#("text", json.string(text))],
      case notes {
        [] -> []
        notes -> [#("notes", json.array(notes, note_json))]
      },
    ]),
  )
}

fn note_json(note: Note) -> json.Json {
  json.object([
    #("design", json.string(note.design)),
    #("elements", json.array(note.elements, json.string)),
    #("text", json.string(note.text)),
  ])
}

fn add_note(
  model: Model,
  design: String,
  elements: List(String),
) -> #(Model, Effect(Msg)) {
  case string.trim(model.note_text), design, elements {
    "", _, _ -> #(model, effect.none())
    _, "", _ -> #(model, effect.none())
    _, _, [] -> #(model, effect.none())
    _, _, _ -> #(
      Model(
        ..model,
        notes: list.append(model.notes, [
          Note(design, elements, model.note_text),
        ]),
        note_text: "",
      ),
      effect.none(),
    )
  }
}

fn remove_at(notes: List(Note), index: Int) -> List(Note) {
  let #(kept, _) =
    list.fold(notes, #([], 0), fn(state, note) {
      let #(kept, at) = state
      case at == index {
        True -> #(kept, at + 1)
        False -> #([note, ..kept], at + 1)
      }
    })
  list.reverse(kept)
}

fn without_all(notes: List(Note), sent: List(Note)) -> List(Note) {
  list.fold(sent, notes, without)
}

fn without(notes: List(Note), sent: Note) -> List(Note) {
  case notes {
    [] -> []
    [first, ..rest] if first == sent -> rest
    [first, ..rest] -> [first, ..without(rest, sent)]
  }
}

/// Drops notes that point into `id`, so a deleted design cannot stop the next
/// message.
pub fn forget_design(model: Model, id: String) -> Model {
  Model(
    ..model,
    notes: list.filter(model.notes, fn(note) { note.design != id }),
  )
}

/// The panel with an Edit tab and an Agent tab. The page supplies the edit
/// view and maps the agent's messages into its own.
pub fn view_tabbed(
  model: Model,
  edit: Element(msg),
  editing: Bool,
  on_edit: msg,
  on_agent: msg,
  lift: fn(Msg) -> msg,
) -> Element(msg) {
  let agent =
    plan_panel.Panel(
      runs: model.runs,
      context: plan_panel.page,
      shown: shown(model),
      entries: model.entries,
      collapsed: model.collapsed,
      width: model.width,
      max_width: plan_panel.max_width(window_width()),
      text: model.text,
      sending: model.sending,
      error: model.error,
      refreshing: model.refreshing,
      models: model.models,
      model: model.model,
      effort: model.effort,
      name: fn(_, _) { "" },
      on_collapse: lift(Collapse),
      on_drag: fn(x) { lift(Drag(x)) },
      on_key: fn(key) { lift(Key(key)) },
      on_text: fn(text) { lift(Text(text)) },
      on_format: fn(before, after) { lift(Format(before, after)) },
      on_block: fn(block) { lift(Block(block)) },
      on_model: fn(name) { lift(ModelPicked(name)) },
      on_effort: fn(effort) { lift(Effort(effort)) },
      on_try_again: lift(TryAgain),
      on_start: lift(Start),
      on_send: lift(Send),
      on_answer: fn(qid, option) { lift(Answer(qid, option)) },
      on_show: fn(id) { lift(Show(id)) },
      on_end: lift(End),
      at_bottom: model.at_bottom,
      new_message: model.new_message,
      on_scroll: fn(bottom) { lift(Scrolled(bottom)) },
      on_newest: lift(Newest),
    )
  plan_panel.view_with_notes_and_tabs(
    "",
    settings(),
    agent,
    [
      plan_panel.Tab("Edit", editing, on_edit),
      plan_panel.Tab("Agent", !editing, on_agent),
    ],
    case editing {
      True -> Some(edit)
      False -> None
    },
    element.map(notes_view(model), lift),
  )
}
