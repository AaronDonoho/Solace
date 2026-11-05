//// `<solace-task>`: one task's details, notes, questions, runs, changes and
//// scope, shared by the board and the Strategy page. It asks the server for
//// its task and makes its own changes. A host that already follows `/events`
//// sets `hosted` and hands the task's events down in the `live` property;
//// otherwise the element follows its project's stream itself.

import gleam/dict.{type Dict}
import gleam/dynamic
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import gleam/uri
import handlers
import lustre
import lustre/attribute as a
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/event as e

@external(javascript, "./task_api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./task_api.mjs", "events")
pub fn events(
  base: String,
  receive: fn(String) -> Nil,
  ready: fn() -> Nil,
) -> Source

@external(javascript, "./task_api.mjs", "close_events")
pub fn close_events(source: Source) -> Nil

@external(javascript, "./task_api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

@external(javascript, "./task_api.mjs", "clock")
fn clock(seconds: Int) -> String

@external(javascript, "./task_api.mjs", "confirm")
fn confirm(question: String) -> Bool

@external(javascript, "./task_api.mjs", "markdown")
fn markdown(text: String, base: String) -> String

/// The markdown editor `id`'s text with its selection wrapped in `before` and
/// `after`; `root` is the shadow root that holds the editor.
@external(javascript, "./task_api.mjs", "wrap")
pub fn wrap(
  root: dynamic.Dynamic,
  id: String,
  text: String,
  before: String,
  after: String,
) -> String

/// The markdown editor `id`'s text with its selected lines made a block of
/// `kind`: "heading", "list", "numbered", "quote" or "code".
@external(javascript, "./task_api.mjs", "block")
pub fn block(
  root: dynamic.Dynamic,
  id: String,
  text: String,
  kind: String,
) -> String

@external(javascript, "./task_api.mjs", "images")
fn images(
  root: dynamic.Dynamic,
  insert: fn(String, String) -> Nil,
  fail: fn(String) -> Nil,
) -> Nil

@external(javascript, "./task_api.mjs", "reveal_task")
fn reveal_task(root: dynamic.Dynamic) -> Nil

@external(javascript, "./task_api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

pub type Source

pub type Column {
  Column(
    id: String,
    label: String,
    actions: List(String),
    final: Bool,
    /// The role whose default the model pick follows in this column.
    role: String,
  )
}

pub type Workflow {
  /// `labels` names the actions that set a `label` (D-108); any other shows its name.
  Workflow(columns: List(Column), labels: Dict(String, String))
}

pub type Note {
  Note(author: String, run: String, text: String, time: Int)
}

/// One step of a run's transcript: a message, thinking, a tool call, token
/// usage, an error or raw output.
pub type Entry {
  Entry(
    kind: String,
    text: String,
    title: String,
    input: String,
    output: String,
    status: String,
  )
}

pub type Question {
  Question(
    id: String,
    text: String,
    options: List(String),
    answer: String,
    settings: Option(#(String, String)),
  )
}

pub type Run {
  Run(
    id: String,
    kind: String,
    state: String,
    model: String,
    effort: String,
    log: String,
    outcome: String,
    started: Int,
    ended: Int,
    /// The model the agent's start event named, when it named one: an alias
    /// shows the model the run used.
    used: String,
    /// Whether the run ended in failure, as the server records it.
    failed: Bool,
  )
}

pub type Link {
  Link(name: String, url: String)
}

/// One changed symbol, split so the Changes tab can group it.
pub type Change {
  Change(project: String, module: String, name: String)
}

pub type Task {
  Task(
    id: String,
    title: String,
    body: String,
    column: String,
    rank: Float,
    paused: String,
    notes: List(Note),
    questions: List(Question),
    runs: List(Run),
    links: List(Link),
    changes: List(Change),
    stamps: List(String),
    attention: String,
    finished: String,
    unexpected: Bool,
    /// A feature's Tech Debt task, which reviews' tech debt is filed on (D-112).
    tech_debt: Bool,
    /// The projects the task may change (D-61); empty for a task saved before.
    scope: List(String),
    /// Whether the task was archived: off the board, still in the archive (D-95).
    archived: Bool,
    /// Whether the board shows the task, as the server computes it (D-196).
    on_board: Bool,
    /// The ids of the tasks it depends on (D-87); empty for a task saved before.
    depends: List(String),
    /// The id of the feature it belongs to, None for none (D-87).
    feature: Option(String),
    /// Whether the task's home is no project of the board (D-129).
    no_project: Bool,
  )
}

pub type Agent {
  Agent(
    id: String,
    label: String,
    available: Bool,
    reason: String,
    effort: String,
    efforts: List(String),
    /// The agent kind whose sign-in it uses, "" when not given.
    agent: String,
    /// Unavailable for a hold a run left: a usage limit or an expired sign-in.
    retry: Bool,
    /// The mark is an expired sign-in, cleared by signing in and Try again.
    sign_in: Bool,
    /// The model string the agent runs, "" for a model with a `run` argv.
    model: String,
  )
}

/// A bold heading, such as "Reviewed Fix checkout", and what happened. It
/// names its task (`dir` is the project it is stored in, filled in by the
/// host) and the tab that shows the news: "notes", "questions" or "runs".
pub type Notice {
  Notice(heading: String, text: String, dir: String, id: String, tab: String)
}

/// A project of the workspace, for the Scope tab: its folder, its name on
/// screen and its API prefix.
pub type Project {
  Project(dir: String, label: String, base: String)
}

pub type Model {
  Model(
    /// The `task` attribute: the id of the task shown.
    id: String,
    /// The `base` attribute: the API prefix of the task's project.
    base: String,
    /// The `dir` attribute: the folder of the project the task is stored in.
    dir: String,
    /// The `workspace` attribute: the root has several projects, so the
    /// Scope tab shows.
    workspace: Bool,
    /// The `hosted` attribute: the host hands down the task's events, so no
    /// stream of its own is opened.
    hosted: Bool,
    /// The `strategy` attribute: the Strategy page shows the task, so a task
    /// in Planning may go back to the backlog there (D-132).
    strategy: Bool,
    /// The `bar` attribute: the host's bar shows the task's title, Copy ID,
    /// column and status, so the details leave them out.
    bar: Bool,
    task: Option(Task),
    /// Counts the tasks opened, so a reply for one shown before is dropped.
    generation: Int,
    /// While another task loads, the parts of it that have arrived: the task
    /// shown before stays until both are in, then they swap at once, so
    /// switching tasks doesn't blank the details. None when nothing loads.
    staged: Option(#(Option(Task), Option(Workflow))),
    workflow: Workflow,
    projects: List(Project),
    models: List(Agent),
    /// The model assigned to each role.
    role_models: List(RoleModel),
    /// The role each action's run step belongs to, from the workflow.
    action_roles: Dict(String, String),
    model: String,
    effort: String,
    /// Where the current model pick came from.
    model_source: ModelSource,
    text: String,
    /// Whether the Notes tab shows its editor (Add note was pressed).
    writing: Bool,
    /// Whether the note editor's send waits for its reply: its send is
    /// disabled meanwhile, so one click makes one request.
    noting: Bool,
    title: String,
    body: String,
    rank: String,
    error: String,
    editing: Bool,
    tab: String,
    /// The action sent and not yet answered: its buttons are disabled
    /// meanwhile, so one click makes one request. "" when none.
    acting: String,
    /// A model refresh sent and not yet answered: its button is disabled
    /// meanwhile, so one click makes one request.
    refreshing: Bool,
    watching: String,
    entries: List(Entry),
    /// The live transcript polling chain; an ended chain schedules nothing.
    poll: Int,
    review_summary: String,
    /// The task a notice asked to show, scrolled into view once it loads.
    revealing: String,
    /// Whether this element's own stream has replayed what it missed.
    caught_up: Bool,
    /// The number of the last message the host handed down that was applied.
    seen: Int,
    source: Option(Source),
    /// The shadow root, once it has rendered.
    root: dynamic.Dynamic,
    connected: Bool,
    /// Copy ID: how many copies were asked for (a reply or a timer for an
    /// older one changes nothing), and what its label says for a moment
    /// after the last: "Copied", "Copy failed", or "" for Copy ID.
    copy: #(Int, String),
    /// The Details tab's **Depends on** line, by task id: None while a
    /// dependency is asked for, or when it can't be found (its id shows).
    deps: List(#(String, Option(Dep))),
    /// The Details tab's **Feature** line: the feature asked for and its
    /// title, None while it loads or when it can't be found (its id shows).
    feature: Option(#(String, Option(String))),
    /// The **Needed by** part of the line: the tasks that depend on this one,
    /// by id. None until they are asked for.
    dependents: Option(List(#(String, Dep))),
  )
}

pub type Msg {
  TaskChanged(String)
  BaseChanged(String)
  DirChanged(String)
  WorkspaceChanged(Bool)
  HostedChanged(Bool)
  StrategyChanged(Bool)
  BarChanged(Bool)
  /// One `/events` message, from the host or from this element's stream.
  Live(String)
  /// Numbered `/events` messages from the host, oldest first; the ones past
  /// the last seen are applied in order.
  Handed(List(#(Int, String)))
  CaughtUp
  /// A notice's button: show the task on this tab and scroll it into view.
  Reveal(String, String)
  Connected
  Disconnected
  Attached(dynamic.Dynamic)
  Listening(Source)
  TaskLoaded(Int, String)
  /// The task fetched again while it is shown (see `refresh`).
  Refetched(Int, String)
  WorkflowLoaded(Int, String)
  ModelsLoaded(String)
  /// The server's model for each role.
  RoleModelsLoaded(String)
  /// The models fetched again with the task (see `refresh`): a failed reply
  /// changes nothing, and a pick that can start stays.
  ModelsRefreshed(String)
  /// Try again: every model's availability is checked again and the holds
  /// a run left are cleared.
  TryAgain
  ProjectsLoaded(String)
  ReviewLoaded(String)
  Changed(String)
  /// A change's reply, with the task opening it was sent for: a reply for
  /// another opening is dropped.
  ChangedAt(Int, String)
  /// A change's reply that clears `editor`'s draft for the task it carries
  /// when the text sent is still the editor's.
  ChangedClearing(Int, String, String, String)
  /// An action's reply, with the task opening it was sent for: a reply for
  /// another opening is dropped.
  Acted(Int, String)
  /// An action's reply that carried text, clearing the note draft for the
  /// task it carries when the text sent is still the editor's.
  ActedClearing(Int, String, String, String)
  AgentSelected(String)
  EffortSelected(String)
  Text(String)
  Title(String)
  Body(String)
  Rank(String)
  Edit(Bool)
  Tab(String)
  Format(String, String, String)
  /// The editor's selected lines become a block: "heading", "list",
  /// "numbered", "quote" or "code" (see `block` in task_api.mjs).
  Block(String, String)
  Failed(String)
  Save
  Action(String)
  /// Opens the Notes tab's editor, where a note is written.
  WriteNote
  /// The editor's send: adds its text as a note.
  AddNote
  /// A note's reply: the task opening it was sent for, the text sent, and the
  /// reply. A reply for another opening is dropped.
  Noted(Int, String, String)
  DeleteNote(Int)
  Answer(String, String)
  SetScope(String, Bool)
  Watch(String)
  Unwatch
  TranscriptLoaded(String, String)
  Refresh(Int, String)
  TranscriptPolled(Int, String, String)
  /// Copy ID: the task's reference, `task:<id>`, goes to the clipboard (D-98).
  CopyId
  /// Copy number `n` is done: whether the clipboard took it.
  CopyDone(Int, Bool)
  /// Copy number `n`'s moment is over: the label says Copy ID again.
  CopyReset(Int)
  /// Where dependency `id` is stored (`GET /tasks/find/{id}` at the root).
  DepFound(Int, String, String)
  /// Dependency `id` as its project has it.
  DepLoaded(Int, String, String)
  /// The task's feature (`GET /features/find/{id}` at the root).
  FeatureLoaded(Int, String)
  /// The tasks that depend on this one (`GET /tasks/{id}/dependents`).
  DependentsLoaded(Int, String)
}

/// While a watched run is going, its transcript is fetched again this often.
const refresh_ms = 1500

/// The Changes tab heading for files, listed after the code.
pub const files_heading = "Files"

/// The Changes tab heading for changes that are neither symbols nor files.
pub const other_heading = "Other"

/// Registers `<solace-task>`, and `<solace-md-editor>` for every page's
/// markdown editors. A page whose other parts registered them already is
/// fine: the board, the shell and Strategy may each ask.
pub fn register() -> Result(Nil, lustre.Error) {
  define_editor()
  let app =
    lustre.component(init, update, view, [
      component.on_attribute_change("task", fn(v) { Ok(TaskChanged(v)) }),
      component.on_attribute_change("base", fn(v) { Ok(BaseChanged(v)) }),
      component.on_attribute_change("dir", fn(v) { Ok(DirChanged(v)) }),
      component.on_attribute_change("workspace", fn(v) {
        Ok(WorkspaceChanged(v != "false"))
      }),
      component.on_attribute_change("hosted", fn(v) {
        Ok(HostedChanged(v != "false"))
      }),
      component.on_attribute_change("strategy", fn(v) {
        Ok(StrategyChanged(v != "false"))
      }),
      component.on_attribute_change("bar", fn(v) {
        Ok(BarChanged(v != "false"))
      }),
      component.on_property_change(
        "live",
        decode.list({
          use n <- decode.field("n", decode.int)
          use text <- decode.field("text", decode.string)
          decode.success(#(n, text))
        })
          |> decode.map(Handed),
      ),
      component.on_property_change("reveal", {
        use tab <- decode.field("tab", decode.string)
        use id <- decode.field("id", decode.string)
        decode.success(Reveal(tab, id))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ])
  case lustre.register(app, "solace-task") {
    Error(lustre.ComponentAlreadyRegistered(_)) -> Ok(Nil)
    other -> other
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(
    Model(
      id: "",
      base: "",
      dir: "",
      workspace: False,
      hosted: False,
      strategy: False,
      bar: False,
      task: None,
      generation: 0,
      staged: None,
      workflow: Workflow([], dict.new()),
      projects: [],
      models: [],
      role_models: [],
      action_roles: dict.new(),
      model: "",
      effort: "",
      model_source: RoleDefault,
      text: "",
      writing: False,
      noting: False,
      title: "",
      body: "",
      rank: "0",
      error: "",
      editing: False,
      tab: "details",
      acting: "",
      refreshing: False,
      watching: "",
      entries: [],
      poll: 0,
      review_summary: "",
      revealing: "",
      caught_up: False,
      seen: 0,
      source: None,
      root: no_root(),
      connected: False,
      copy: #(0, ""),
      deps: [],
      feature: None,
      dependents: None,
    ),
    // Attributes arrive before the element is added; nothing is asked until then.
    effect.none(),
  )
}

fn get(base: String, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(base, path, "GET", "", fn(text) { send(msg(text)) })
  })
}

/// A change to the task; the reply is the task as it is now.
fn send(
  model: Model,
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
  reply: fn(Int, String) -> Msg,
) -> Effect(Msg) {
  effect.from(fn(dispatch) {
    request(
      model.base,
      path,
      method,
      json.to_string(json.object(fields)),
      fn(text) { dispatch(reply(model.generation, text)) },
    )
  })
}

fn error(text: String) -> String {
  json.parse(text, decode.at(["message"], decode.string))
  |> result.unwrap("Invalid API response")
}

/// Asks for the task and its project's workflow, and follows the task's
/// events unless the host hands them down.
fn fetch(model: Model) -> Effect(Msg) {
  case model.connected, model.id {
    True, id if id != "" -> {
      let generation = model.generation
      effect.batch([
        get(model.base, "/tasks/" <> id, TaskLoaded(generation, _)),
        get(model.base, "/workflow", WorkflowLoaded(generation, _)),
        listen(model),
      ])
    }
    _, _ -> effect.none()
  }
}

/// Shows the task the attributes name now: what belonged to the one shown
/// before goes, once the new one has loaded (see `settle`).
fn open(model: Model) -> #(Model, Effect(Msg)) {
  // Nothing is asked for without an id or a connection: the task shown
  // before would wait, inert, for a reply that never comes.
  let staging = model.task != None && model.connected && model.id != ""
  let model = case staging {
    True ->
      Model(
        ..stop(model),
        generation: model.generation + 1,
        staged: Some(#(None, None)),
        copy: #(model.copy.0 + 1, model.copy.1),
        error: "",
        model_source: RoleDefault,
        acting: "",
        writing: False,
        noting: False,
      )
    False ->
      Model(
        ..cleared(stop(model)),
        generation: model.generation + 1,
        workflow: Workflow([], dict.new()),
        action_roles: dict.new(),
        copy: #(model.copy.0 + 1, ""),
      )
  }
  #(model, fetch(model))
}

fn listen(model: Model) -> Effect(Msg) {
  case model.hosted {
    True -> effect.none()
    False ->
      effect.from(fn(send) {
        send(
          Listening(
            events(model.base, fn(text) { send(Live(text)) }, fn() {
              send(CaughtUp)
            }),
          ),
        )
      })
  }
}

/// Closes this element's own stream; a new one replays what it missed first.
fn close(model: Model) -> Model {
  case model.source {
    Some(source) -> close_events(source)
    None -> Nil
  }
  Model(..model, source: None, caught_up: False)
}

/// Closes the stream and ends the transcript polling chain.
fn stop(model: Model) -> Model {
  Model(..close(model), poll: model.poll + 1)
}

fn transcript(model: Model, rid: String) -> Effect(Msg) {
  case model.task {
    Some(t) ->
      get(model.base, "/tasks/" <> t.id <> "/runs/" <> rid, fn(text) {
        TranscriptPolled(model.poll, rid, text)
      })
    None -> effect.none()
  }
}

/// The review summary of a task with changes, for the Changes tab.
fn review(model: Model, t: Task) -> Effect(Msg) {
  case t.changes {
    [] -> effect.none()
    _ -> get(model.base, "/graph/review?task=" <> t.id, ReviewLoaded)
  }
}

/// Scrolls the task into view and puts focus on its selected tab, once it
/// has rendered.
fn reveal() -> Effect(Msg) {
  effect.before_paint(fn(_, root) { reveal_task(root) })
}

/// The projects a task covers; a task saved before D-61 covers its own.
pub fn scope_of(home: String, t: Task) -> List(String) {
  case t.scope {
    [] -> [home]
    scope -> scope
  }
}

/// The task now lives in `dir`, one of its projects (D-61): it is asked for
/// there from now on, and the host hears of it in `task-moved`.
fn follow(model: Model, dir: String) -> #(Model, Effect(Msg)) {
  let base =
    list.find(model.projects, fn(p) { p.dir == dir })
    |> result.map(fn(p) { p.base })
    |> result.unwrap(model.base)
  let moved = Model(..close(model), dir: dir, base: base)
  #(
    moved,
    effect.batch([
      e.emit(
        "task-moved",
        json.object([
          #("id", json.string(model.id)),
          #("dir", json.string(dir)),
          #("base", json.string(base)),
        ]),
      ),
      case moved.connected {
        True -> listen(moved)
        False -> effect.none()
      },
    ]),
  )
}

/// The task left this project: it is opened from the first other project it
/// covers.
fn moved_away(model: Model) -> #(Model, Effect(Msg)) {
  // While another task loads, the shown one is not the task that moved.
  let moving = case model.staged {
    Some(#(t, _)) -> t
    None -> model.task
  }
  let scope = case moving {
    Some(t) -> scope_of(model.dir, t)
    None -> []
  }
  case
    list.find(model.projects, fn(p) {
      p.dir != model.dir && list.contains(scope, p.dir)
    })
  {
    Ok(p) -> {
      let #(model, fx) = follow(model, p.dir)
      #(
        model,
        effect.batch([
          fx,
          get(model.base, "/tasks/" <> model.id, TaskLoaded(model.generation, _)),
        ]),
      )
    }
    // A task still loading has no scope to follow: the one shown before
    // goes rather than wait, inert, for a reply that may never show it.
    Error(_) ->
      case model.staged {
        Some(#(None, _)) -> #(unstage(model), effect.none())
        _ -> #(model, effect.none())
      }
  }
}

fn notice(n: Notice) -> Effect(Msg) {
  e.emit(
    "notice",
    json.object([
      #("heading", json.string(n.heading)),
      #("text", json.string(n.text)),
      #("id", json.string(n.id)),
      #("tab", json.string(n.tab)),
    ]),
  )
}

/// Applies one `/events` message about this task: its new state, its
/// project's workflow, or its removal from the project; a change to the
/// workspace's list of projects fetches the Scope tab's again. A workspace's
/// messages name their `project`; one for another project is not this task's,
/// unless it names this task, which is then fetched again (see `stale`).
fn live(model: Model, text: String) -> #(Model, Effect(Msg)) {
  // A task with no project (D-129) is named under its home folder, which is
  // no project to open it from: its messages are this element's all the same.
  let ours = case json.parse(text, decode.at(["project"], decode.string)) {
    Ok(dir) -> dir == model.dir || homeless_event(text)
    Error(_) -> True
  }
  // A dependency's or dependent's update counts under any project: it may
  // live in another.
  let dep = case
    model.task,
    json.parse(text, decode.at(["task"], task_decoder()))
  {
    Some(t), Ok(d) if d.id != t.id ->
      case
        list.contains(t.depends, d.id)
        || needed_by(model, t, d) != model.dependents
      {
        True -> Ok(#(t, d))
        False -> Error(Nil)
      }
    _, _ -> Error(Nil)
  }
  // What a new stream of its own replays is old: the task was fetched whole.
  case model.hosted || model.caught_up, stale(model, text), ours, dep {
    False, _, _, _ -> #(model, effect.none())
    True, _, _, Ok(#(t, d)) -> {
      let deps = case list.contains(t.depends, d.id) {
        True -> list.key_set(model.deps, d.id, Some(Dep(d.title, d.column)))
        False -> model.deps
      }
      #(
        Model(..model, deps: deps, dependents: needed_by(model, t, d)),
        effect.none(),
      )
    }
    True, False, False, _ -> #(model, effect.none())
    True, True, _, _ -> {
      // After a resync the dependencies, feature and dependents may have
      // changed too.
      let model = case json.parse(text, decode.at(["resync"], decode.bool)) {
        Ok(_) -> Model(..model, deps: [], feature: None, dependents: None)
        Error(_) -> model
      }
      let #(model, deps) = want_deps(model)
      // `refresh` asks for the task shown, not the one being loaded.
      let again = case model.staged {
        Some(_) -> reload(model)
        None -> refresh(model)
      }
      #(model, effect.batch([again, deps]))
    }
    True, False, True, _ ->
      case json.parse(text, decode.at(["task"], task_decoder())) {
        // The task being loaded: its newer state waits with it.
        Ok(fresh) if fresh.id == model.id && model.staged != None ->
          case model.staged {
            Some(#(Some(t), w)) -> {
              let fresh = Task(..fresh, changes: t.changes)
              let #(model, fx) =
                settle(Model(..model, staged: Some(#(Some(fresh), w))))
              let news = case model.hosted {
                False -> list.map(notices(t, fresh), notice)
                True -> []
              }
              #(model, effect.batch([fx, ..news]))
            }
            // The reply on its way may be older than the event: ask again.
            Some(#(None, _)) -> #(model, reload(model))
            None -> #(model, effect.none())
          }
        Ok(fresh) if fresh.id == model.id -> {
          // Events carry no `changes` (D-77): keep the list until it is fetched again.
          let old = model.task
          let fresh = case old {
            Some(t) -> Task(..fresh, changes: t.changes)
            None -> fresh
          }
          let model = Model(..model, task: Some(fresh))
          let #(model, deps) = want_deps(model)
          // A host raises its own notices.
          let news = case old, model.hosted {
            Some(t), False -> list.map(notices(t, fresh), notice)
            _, _ -> []
          }
          #(model, effect.batch([refresh(model), deps, ..news]))
        }
        Ok(_) -> #(model, effect.none())
        Error(_) ->
          case
            json.parse(
              text,
              decode.at(["workflow"], workflow_with_roles_decoder()),
            )
          {
            Ok(#(w, roles)) ->
              case model.staged {
                Some(#(t, _)) ->
                  settle(
                    Model(
                      ..model,
                      action_roles: roles,
                      staged: Some(#(t, Some(w))),
                    ),
                  )
                None -> #(
                  Model(..model, workflow: w, action_roles: roles),
                  effect.none(),
                )
              }
            Error(_) ->
              case json.parse(text, decode.at(["removed"], decode.string)) {
                Ok(id) if id == model.id -> moved_away(model)
                _ ->
                  case projects_changed(text) {
                    True -> #(model, get("", "/projects", ProjectsLoaded))
                    False -> #(model, effect.none())
                  }
              }
          }
      }
  }
}

/// The text an editor shows, and the message that changes it.
fn field(model: Model, id: String) -> #(String, fn(String) -> Msg) {
  case id {
    "edit-body" -> #(model.body, Body)
    _ -> #(model.text, Text)
  }
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  [
    fn() { connection_events(model, msg) },
    fn() { live_events(model, msg) },
    fn() { opening_events(model, msg) },
    fn() { model_events(model, msg) },
    fn() { reply_events(model, msg) },
    fn() { transcript_events(model, msg) },
    fn() { editor_events(model, msg) },
    fn() { scope_events(model, msg) },
    fn() { dependency_events(model, msg) },
  ]
  |> handlers.first(#(model, effect.none()))
}

/// What the human should hear about in a live update: a run that ended, a
/// discussion reply, or a question an agent asked.
pub fn notices(old: Task, new: Task) -> List(Notice) {
  let ended =
    list.filter(new.runs, fn(r) {
      r.state == "ended"
      && r.kind != "discuss"
      && list.any(old.runs, fn(o) { o.id == r.id && o.state != "ended" })
    })
    |> list.map(fn(r) {
      // A run that left a note shows it; else its run is the place to look.
      let tab = case list.any(new.notes, fn(n) { n.run == r.id }) {
        True -> "notes"
        False -> "runs"
      }
      let notice = fn(heading, text) { Notice(heading, text, "", new.id, tab) }
      case r.outcome {
        "finished" ->
          notice("Finished " <> new.title, "The agent finished its work.")
        "reported" ->
          notice("Investigated " <> new.title, "The agent's report is ready.")
        "reviewed" -> notice("Reviewed " <> new.title, "The review is ready.")
        "" -> notice("Stopped " <> new.title, "The " <> r.kind <> " run ended.")
        outcome ->
          notice(
            "Stopped " <> new.title <> ": " <> outcome,
            "The " <> r.kind <> " run ended: " <> outcome <> ".",
          )
      }
    })
  let asked =
    list.filter(new.questions, fn(q) {
      q.answer == "" && !list.any(old.questions, fn(o) { o.id == q.id })
    })
    |> list.map(fn(q) {
      Notice("Question on " <> new.title, q.text, "", new.id, "questions")
    })
  // An agent's reply sets its discussion run waiting in the same update that
  // adds the note, so the run going waiting is the reply. Tasks loaded at
  // startup come without their notes, so the run's state is what to watch.
  let replied =
    list.filter(new.runs, fn(r) {
      r.kind == "discuss"
      && r.state == "waiting"
      && list.any(old.runs, fn(o) { o.id == r.id && o.state != "waiting" })
    })
    |> list.map(fn(r) {
      let text = case
        list.filter(new.notes, fn(n) { n.author == "agent" && n.run == r.id })
        |> list.sort(fn(a, b) { int.compare(b.time, a.time) })
        |> list.first
      {
        Ok(note) -> note.text
        Error(_) -> "The agent replied."
      }
      Notice("Reply on " <> new.title, text, "", new.id, "notes")
    })
  list.flatten([ended, replied, asked])
}

pub fn task_decoder() -> decode.Decoder(Task) {
  use id <- decode.field("id", decode.string)
  use title <- decode.field("title", decode.string)
  use body <- decode.field("body", decode.string)
  use column <- decode.field("column", decode.string)
  use rank <- decode.field("rank", decode.float)
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
    decode.list({
      use name <- decode.field("name", decode.string)
      use entity <- decode.optional_field("entity", "", decode.string)
      use project <- decode.optional_field("project", "", decode.string)
      // A symbol's name is `module.symbol`, the symbol after the last dot; a
      // file's is `file:path`, and anything else is shown whole.
      let #(module, symbol) = case
        entity,
        list.reverse(string.split(name, "."))
      {
        "file:" <> path, _ -> #(files_heading, path)
        "symbol:" <> _, [symbol, ..rest] if rest != [] -> #(
          string.join(list.reverse(rest), "."),
          symbol,
        )
        _, _ -> #(other_heading, name)
      }
      decode.success(Change(project, module, symbol))
    }),
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
  use finished <- decode.optional_field(
    "finished",
    "",
    decode.one_of(decode.at(["outcome"], decode.string), [decode.success("")]),
  )
  use unexpected <- decode.optional_field("unexpected", False, decode.bool)
  use tech_debt <- decode.optional_field("techDebt", False, decode.bool)
  use scope <- decode.optional_field("scope", [], decode.list(decode.string))
  use archived <- decode.optional_field(
    "archivedTime",
    False,
    decode.one_of(decode.int |> decode.map(fn(_) { True }), [
      decode.success(False),
    ]),
  )
  use on_board <- decode.optional_field("onBoard", !archived, decode.bool)
  use depends <- decode.optional_field(
    "dependsOn",
    [],
    decode.list(decode.string),
  )
  use feature <- decode.optional_field(
    "feature",
    None,
    decode.optional(decode.string),
  )
  use no_project <- decode.optional_field("noProject", False, decode.bool)
  decode.success(Task(
    id,
    title,
    body,
    column,
    rank,
    paused,
    notes,
    questions,
    runs,
    links,
    changes,
    stamps,
    attention,
    finished,
    unexpected,
    tech_debt,
    scope,
    archived,
    on_board,
    depends,
    feature,
    no_project,
  ))
}

pub fn note_decoder() -> decode.Decoder(Note) {
  use author <- decode.field("author", decode.string)
  use run <- decode.optional_field(
    "run",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use text <- decode.field("text", decode.string)
  use time <- decode.optional_field("time", 0, decode.int)
  decode.success(Note(author, run, text, time))
}

pub fn entry_decoder() -> decode.Decoder(Entry) {
  let text = fn(name, next) {
    decode.optional_field(
      name,
      "",
      decode.one_of(decode.string, [decode.success("")]),
      next,
    )
  }
  use kind <- decode.field("kind", decode.string)
  use body <- text("text")
  use title <- text("title")
  use input <- text("input")
  use output <- text("output")
  use status <- text("status")
  decode.success(Entry(kind, body, title, input, output, status))
}

fn question_decoder() -> decode.Decoder(Question) {
  use id <- decode.field("id", decode.string)
  use text <- decode.field("text", decode.string)
  use options <- decode.field("options", decode.list(decode.string))
  use answer <- decode.optional_field("answer", "", decode.string)
  use settings <- decode.optional_field(
    "settings",
    None,
    decode.optional({
      use base <- decode.field("base", decode.string)
      use proposed <- decode.field("proposed", decode.string)
      decode.success(#(base, proposed))
    }),
  )
  decode.success(Question(id, text, options, answer, settings))
}

fn run_decoder() -> decode.Decoder(Run) {
  use id <- decode.field("id", decode.string)
  use kind <- decode.field("kind", decode.string)
  use state <- decode.field("state", decode.string)
  use model <- decode.field("model", decode.string)
  use effort <- decode.optional_field(
    "effort",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use log <- decode.field("log", decode.string)
  use outcome <- decode.optional_field("outcome", "", decode.string)
  use started <- decode.optional_field("started", 0, decode.int)
  use ended <- decode.optional_field("ended", 0, decode.int)
  use used <- decode.optional_field(
    "used",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use failed <- decode.optional_field("failed", False, decode.bool)
  decode.success(Run(
    id,
    kind,
    state,
    model,
    effort,
    log,
    outcome,
    started,
    ended,
    used,
    failed,
  ))
}

fn link_decoder() -> decode.Decoder(Link) {
  use name <- decode.field("name", decode.string)
  use url <- decode.field("url", decode.string)
  decode.success(Link(name, url))
}

pub fn agent_decoder() -> decode.Decoder(Agent) {
  use id <- decode.field("id", decode.string)
  use label <- decode.field("label", decode.string)
  use available <- decode.optional_field("available", True, decode.bool)
  use reason <- decode.optional_field(
    "reason",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use effort <- decode.optional_field(
    "effort",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use efforts <- decode.optional_field(
    "efforts",
    [],
    decode.list(decode.string),
  )
  use agent <- decode.optional_field(
    "agent",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use retry <- decode.optional_field("retry", False, decode.bool)
  use sign_in <- decode.optional_field("sign_in", False, decode.bool)
  use model <- decode.optional_field(
    "model",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  decode.success(Agent(
    id,
    label,
    available,
    reason,
    effort,
    efforts,
    agent,
    retry,
    sign_in,
    model,
  ))
}

pub fn workflow_decoder() -> decode.Decoder(Workflow) {
  workflow_with_roles_decoder()
  |> decode.map(fn(pair) { pair.0 })
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
  use role <- decode.optional_field("role", "", decode.string)
  decode.success(Column(id, label, actions, final, role))
}

/// The projects in `GET /projects`, in the order the server lists them.
fn projects_decoder() -> decode.Decoder(List(Project)) {
  let project = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use base <- decode.field("base", decode.string)
    decode.success(Project(dir, label, base))
  }
  decode.at(["projects"], decode.list(project))
}

/// `strategy`: the Strategy page shows the task (D-132).
pub fn actions(workflow: Workflow, task: Task, strategy: Bool) -> List(String) {
  // Discuss and End Discussion sit in the notes editor; stop ends any run.
  let built_in = case list.find(task.runs, fn(r) { r.state != "ended" }) {
    Error(_) -> []
    Ok(_) -> ["stop"]
  }
  let offered = case
    list.find(workflow.columns, fn(c) { c.id == task.column })
  {
    // A final column offers only what it lists: no agent works there (D-108).
    Ok(c) if c.final -> c.actions
    Ok(c) ->
      list.flatten([
        c.actions,
        case task.paused {
          "" -> ["pause"]
          _ -> ["resume"]
        },
        built_in,
      ])
    Error(_) -> []
  }
  // A task with a feature goes back to the backlog only through its feature,
  // and from Planning only on the Strategy page (D-132).
  let back = case
    task.feature,
    list.find(workflow.columns, fn(c) { c.id == task.column })
  {
    Some(_), _ -> False
    None, Ok(c) -> c.final || strategy
    None, Error(_) -> False
  }
  list.filter(offered, fn(name) { name != "to_backlog" || back })
}

/// What an action in flight is waiting for: with a work run going, a code
/// change takes the project's lock, which the agent's command holds.
pub fn acting_text(t: Task, name: String) -> String {
  case list.any(t.runs, fn(r) { r.kind == "work" && r.state != "ended" }) {
    True ->
      "Waiting for the agent's current command to finish before " <> name <> "…"
    False -> "Sending " <> name <> "…"
  }
}

/// Lines only in the old settings, then lines only in the proposed ones.
/// ponytail: a line-set diff, not an LCS; fine for a short solace.toml.
pub fn settings_diff(base: String, proposed: String) -> String {
  let old = string.split(base, "\n")
  let new = string.split(proposed, "\n")
  list.append(
    list.filter(old, fn(l) { !list.contains(new, l) })
      |> list.map(fn(l) { "- " <> l }),
    list.filter(new, fn(l) { !list.contains(old, l) })
      |> list.map(fn(l) { "+ " <> l }),
  )
  |> string.join("\n")
}

/// An agent note names its run, as the run list does.
pub fn author(models: List(Agent), t: Task, n: Note) -> String {
  case n.author, list.find(t.runs, fn(r) { r.id == n.run }) {
    "agent", Ok(r) -> "agent · " <> run_label(models, r)
    who, _ -> who
  }
}

fn run_label(models: List(Agent), r: Run) -> String {
  r.kind
  <> " · "
  <> run_model_label(models, r.model, r.used)
  <> case r.effort {
    "" -> ""
    effort -> " · " <> effort
  }
}

/// What the task's agent is doing right now, or what it finished.
pub fn status(t: Task) -> String {
  case t.paused, list.find(t.runs, fn(r) { r.state != "ended" }) {
    _, Error(_) -> outcome(t)
    "", Ok(r) if r.state == "waiting" && r.kind == "discuss" ->
      "Waiting for your reply"
    "", Ok(r) if r.state == "waiting" -> "Waiting for your answer"
    "", Ok(r) if r.kind == "discuss" -> "Agent discussing"
    "", Ok(r) if r.kind == "review" -> "Agent reviewing"
    "", Ok(r) if r.kind == "investigate" -> "Agent investigating"
    "", Ok(_) -> "Agent working"
    _, Ok(_) -> "Agent paused"
  }
}

/// Changes as an outline: source modules before test modules, each module's
/// symbols alphabetical. Headings name the project too when `multi_project`.
pub fn change_groups(
  changes: List(Change),
  multi_project: Bool,
) -> List(#(String, List(String))) {
  // Source, then tests, then files, then anything else.
  let is_test = fn(c: Change) {
    case c.module {
      m if m == files_heading -> 2
      m if m == other_heading -> 3
      m ->
        case string.ends_with(m, "_test") {
          True -> 1
          False -> 0
        }
    }
  }
  let sorted =
    list.sort(changes, fn(a, b) {
      string.compare(a.project, b.project)
      |> order.break_tie(int.compare(is_test(a), is_test(b)))
      |> order.break_tie(string.compare(a.module, b.module))
      |> order.break_tie(string.compare(a.name, b.name))
    })
  let heading = fn(c: Change) {
    case multi_project, c.project {
      True, p if p != "" -> p <> " · " <> c.module
      _, _ -> c.module
    }
  }
  list.fold(sorted, [], fn(groups, c) {
    let h = heading(c)
    case groups {
      [#(last, names), ..rest] if last == h -> [#(h, [c.name, ..names]), ..rest]
      _ -> [#(h, [c.name]), ..groups]
    }
  })
  |> list.map(fn(g) { #(g.0, list.reverse(g.1)) })
  |> list.reverse
}

pub fn view(model: Model) -> Element(Msg) {
  h.div([], [
    h.style([], task_styles),
    // An empty paragraph keeps its margins: under the board's bar it would
    // leave a gap above the details.
    case model.error {
      "" -> h.text("")
      error -> h.p([a.role("alert")], [h.text(error)])
    },
    case model.task {
      None -> h.text("")
      Some(t) -> detail(model, t)
    },
  ])
}

fn detail(model: Model, t: Task) -> Element(Msg) {
  let open_questions = list.any(t.questions, fn(q) { q.answer == "" })
  let tabs = [
    #("details", "Details"),
    #("notes", "Notes (" <> int.to_string(list.length(t.notes)) <> ")"),
    #(
      "questions",
      "Questions ("
        <> int.to_string(list.length(t.questions))
        <> ")"
        <> case open_questions {
        True -> " •"
        False -> ""
      },
    ),
    #("runs", "Runs (" <> int.to_string(list.length(t.runs)) <> ")"),
    #("changes", "Changes (" <> int.to_string(list.length(t.changes)) <> ")"),
  ]
  let tabs = case model.workspace {
    True -> list.append(tabs, [#("scope", "Scope")])
    False -> tabs
  }
  // The task shown before stays while another loads, but can't be used: its
  // clicks would reach the new one.
  let loading = case model.staged {
    Some(_) -> [a.attribute("inert", ""), a.attribute("aria-busy", "true")]
    None -> []
  }
  h.section([a.class("detail"), ..loading], [
    case model.bar {
      True -> h.text("")
      False ->
        h.div([a.class("heading")], [
          h.h2([], [h.text(t.title)]),
          copy_button(t.id, model.copy.1, CopyId),
        ])
    },
    case model.bar {
      True -> h.text("")
      False -> h.p([a.class("meta")], meta(model.workflow, t))
    },
    // What the human does, then the task's record: side by side on a wide
    // element (`task_styles`), one above the other otherwise.
    h.div([a.class("act")], [
      case t.attention {
        "" -> h.text("")
        reason ->
          h.div([a.class("attention"), a.role("status")], [
            h.strong([], [h.text("Needs attention")]),
            md(model.base, reason),
          ])
      },
      agent_choice(model),
      case model.acting {
        "" -> h.text("")
        name ->
          h.p([a.class("muted"), a.role("status")], [
            h.text(acting_text(t, action_label(model.workflow, name))),
          ])
      },
      h.div(
        [a.class("actions")],
        list.map(actions(model.workflow, t, model.strategy), fn(name) {
          action_view(model, name, action_label(model.workflow, name))
        }),
      ),
    ]),
    h.div([a.class("record")], [
      h.div(
        [a.class("tabs"), a.role("tablist")],
        list.map(tabs, fn(tab) {
          h.button(
            [
              a.role("tab"),
              a.aria_selected(model.tab == tab.0),
              e.on_click(Tab(tab.0)),
            ],
            [h.text(tab.1)],
          )
        }),
      ),
      h.div([a.class("panel"), a.role("tabpanel")], case model.tab {
        "notes" -> notes_view(model, t)
        "questions" -> questions_view(model, t)
        "runs" -> list.map(list.reverse(t.runs), fn(r) { run_view(model, r) })
        "changes" -> [
          case t.changes {
            [] -> h.text("")
            _ ->
              h.p([], [
                h.text(model.review_summary <> " "),
                h.a([a.href(model.base <> "/contracts?task=" <> t.id)], [
                  h.text(ui_label("contract_view")),
                ]),
              ])
          },
          h.div(
            [a.class("changes")],
            list.map(change_groups(t.changes, list.length(t.scope) > 1), fn(g) {
              h.section([], [
                h.h4([], [h.text(g.0)]),
                h.ul([], list.map(g.1, fn(n) { h.li([], [h.text(n)]) })),
              ])
            }),
          ),
        ]
        "scope" -> [scope_view(model, t)]
        _ -> details_view(model, t)
      }),
    ]),
  ])
}

fn details_view(model: Model, t: Task) -> List(Element(Msg)) {
  case model.editing {
    True -> [
      title_input(model.title, Title),
      task_editor(model, "edit-body", "Description", []),
      h.label([], [
        h.text("Rank (lower comes first)"),
        h.input([a.type_("number"), a.value(model.rank), e.on_input(Rank)]),
      ]),
      button(ui_label("save"), Save),
      button(ui_label("cancel"), Edit(False)),
    ]
    False -> [
      feature_view(model, t),
      depends_view(model, t),
      md(model.base, t.body),
      h.div(
        [],
        list.map(t.links, fn(link) {
          h.a([a.href(link.url)], [h.text(link.name)])
        }),
      ),
      button(ui_label("edit"), Edit(True)),
    ]
  }
}

/// The model and its effort. A pick shows what is chosen, so it names
/// itself; while none is, a placeholder says what to pick. Only models that
/// can start are offered; with none that can, a line says why each can't
/// and what to set up.
fn agent_choice(model: Model) -> Element(Msg) {
  let ready = list.filter(model.models, fn(m) { m.available })
  let chosen = list.find(ready, fn(m) { m.id == model.model })
  case model.models, ready {
    [_, ..], [] ->
      h.div([a.class("agent")], [
        refresh_button(TryAgain, model.refreshing),
        no_agents_view(model.models),
        unavailable_view(model.models),
      ])
    _, _ -> agent_pick(model, ready, chosen)
  }
}

pub fn title_input(value: String, msg: fn(String) -> msg) -> Element(msg) {
  h.input([
    a.class("title"),
    a.placeholder("Title"),
    a.attribute("aria-label", "Title"),
    a.value(value),
    e.on_input(msg),
  ])
}

fn composer(model: Model, t: Task) -> Element(Msg) {
  task_editor(model, "note-text", "Note", note_actions(model, t))
}

/// One of the task's editors: `id` names what it edits (see `field`).
fn task_editor(
  model: Model,
  id: String,
  label: String,
  actions: List(EditorAction(Msg)),
) -> Element(Msg) {
  let #(text, msg) = field(model, id)
  editor(EditorConfig(
    id: id,
    task: model.id,
    base: model.base,
    label: label,
    placeholder: "",
    text: text,
    on_input: msg,
    on_format: fn(before, after) { Format(id, before, after) },
    on_block: Block(id, _),
    actions: actions,
  ))
}

/// A markdown editor with formatting tools, which styles the markdown as it
/// is written: `<solace-md-editor>` (`md_editor.mjs`, CodeMirror), named by
/// `id`. Its marks show on the lines being edited and hide elsewhere. Images
/// pasted, dropped or picked here are uploaded by `task_api.mjs` to `task` in
/// the project at `base` and inserted as markdown. A task not yet created has
/// no image tool. `actions` sit inside the editor at its bottom right; each
/// one's content is words, or the send icon when empty. The text keeps clear
/// of them.
pub fn editor(config: EditorConfig(msg)) -> Element(msg) {
  let EditorConfig(
    id,
    task,
    base,
    label,
    placeholder,
    text,
    on_input,
    on_format,
    on_block,
    actions,
  ) = config
  let hint = case placeholder {
    "" -> label <> " (markdown)."
    hint -> hint
  }
  let tool = fn(label, title, before, after) {
    h.button(
      [a.type_("button"), a.title(title), e.on_click(on_format(before, after))],
      [h.text(label)],
    )
  }
  let block_tool = fn(label, title, kind) {
    h.button([a.type_("button"), a.title(title), e.on_click(on_block(kind))], [
      h.text(label),
    ])
  }
  let classes = case
    actions,
    list.any(actions, fn(action) { action.content != "" })
  {
    [], _ -> "composer"
    _, True -> "composer has-actions has-action-words"
    _, False -> "composer has-actions"
  }
  h.div(
    [
      a.class(classes),
    ],
    [
      h.div([a.class("toolbar")], [
        block_tool("H", "Heading", "heading"),
        tool("B", "Bold", "**", "**"),
        tool("`code`", "Code", "`", "`"),
        block_tool("Code block", "Code block", "code"),
        block_tool("• List", "List", "list"),
        block_tool("1. List", "Numbered list", "numbered"),
        block_tool("Quote", "Quote", "quote"),
        tool("Link", "Link", "[", "](https://)"),
        case task {
          "" -> h.text("")
          _ ->
            h.label([a.class("button"), a.title("Add an image")], [
              h.text("Image"),
              h.input([
                a.type_("file"),
                a.class("md-image sr-only"),
                a.attribute("data-task", task),
                a.attribute("data-base", base),
                a.attribute("data-for", id),
                a.accept(["image/png", "image/jpeg", "image/gif", "image/webp"]),
              ]),
            ])
        },
      ]),
      element.element(
        "solace-md-editor",
        [
          a.id(id),
          a.class("md-editor"),
          a.attribute("data-task", task),
          a.attribute("data-base", base),
          a.attribute("label", label),
          a.attribute(
            "placeholder",
            hint
              <> case task {
              "" -> ""
              _ -> " Paste or drop images here."
            },
          ),
          a.property("value", json.string(text)),
          e.on(
            "md-input",
            decode.at(["detail"], decode.string) |> decode.map(on_input),
          ),
        ],
        [],
      ),
      case actions {
        [] -> h.text("")
        actions ->
          h.div([a.class("composer-actions")], list.map(actions, editor_action))
      },
    ],
  )
}

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

fn action_button(name: String, label: String, busy: Bool) -> Element(Msg) {
  h.button([e.on_click(Action(name)), a.disabled(busy)], [h.text(label)])
}

fn questions_view(model: Model, t: Task) -> List(Element(Msg)) {
  let first = first_unanswered(t)
  let writing = case first {
    Some(id) -> [
      task_editor(model, "note-text", ui_label("answer"), [
        EditorAction(
          "",
          ui_label("answer"),
          Answer(id, ""),
          string.trim(model.text) == "",
        ),
      ]),
    ]
    None -> []
  }
  list.append(
    writing,
    list.map(list.reverse(t.questions), fn(q) {
      let #(diff, other) = case q.settings {
        Some(#(current, proposed)) -> #(
          [h.pre([a.class("diff")], [h.text(settings_diff(current, proposed))])],
          "Reject with the editor's text",
        )
        None -> #([], ui_label("answer"))
      }
      let free_text = case q.settings {
        Some(_) -> True
        None -> Some(q.id) != first
      }
      h.article([], [
        md(model.base, q.text),
        ..list.append(diff, [
          case q.answer {
            "" ->
              h.div(
                [],
                list.append(
                  list.map(q.options, fn(o) { button(o, Answer(q.id, o)) }),
                  case free_text {
                    True -> [button(other, Answer(q.id, ""))]
                    False -> []
                  },
                ),
              )
            answer -> answer_view(model.base, answer)
          },
        ])
      ])
    }),
  )
}

/// Newest first, under the Add note button while the editor is closed, and
/// under the editor once it opens. Each note keeps its position, which names
/// it to the API.
fn notes_view(model: Model, t: Task) -> List(Element(Msg)) {
  let notes =
    t.notes
    |> list.index_map(fn(n, i) { #(n, i) })
    |> list.reverse
    |> list.map(fn(pair) {
      let #(n, i) = pair
      h.details([a.class("note"), a.open(True)], [
        h.summary([], [
          h.button(
            [
              a.class("delete"),
              a.attribute("aria-label", ui_label("delete_note")),
              e.on_click(DeleteNote(i)) |> e.prevent_default,
            ],
            [h.text(ui_label("delete"))],
          ),
          h.strong([], [h.text(author(model.models, t, n))]),
          h.small([], [h.text(clock(n.time))]),
        ]),
        md(model.base, n.text),
      ])
    })
  let note_tools = case model.writing {
    True -> []
    False -> [h.div([a.class("note-tools")], [add_note_button()])]
  }
  let note_editor = case model.writing {
    True -> [composer(model, t)]
    False -> []
  }
  list.flatten([note_tools, note_editor, notes])
}

pub fn status_badge(t: Task) -> Element(msg) {
  badge(status(t))
}

/// Markdown; images uploaded to the board load from the project at `base`.
pub fn md(base: String, text: String) -> Element(msg) {
  element.unsafe_raw_html("", "div", [a.class("md")], markdown(text, base))
}

fn run_view(model: Model, r: Run) -> Element(Msg) {
  let watching = model.watching == r.id
  let when = case r.ended {
    0 -> "started " <> clock(r.started)
    ended -> clock(r.started) <> " – " <> clock(ended)
  }
  h.article([a.class("run")], [
    h.header([], [
      h.strong([], [h.text(run_label(model.models, r))]),
      h.span([a.class("state " <> r.state)], [
        h.text(case r.state, r.outcome {
          "ended", "" -> "ended"
          "ended", outcome -> "ended: " <> outcome
          state, _ -> state
        }),
      ]),
      case watching {
        True -> button(ui_label("hide_activity"), Unwatch)
        False -> button(ui_label("watch"), Watch(r.id))
      },
    ]),
    h.small([], [h.text(when <> " · log: " <> r.log)]),
    case watching {
      False -> h.text("")
      True ->
        case model.entries {
          [] -> h.p([a.class("muted")], [h.text("No activity yet.")])
          entries ->
            h.div(
              [a.class("transcript")],
              list.map(entries, entry_view(model.base, _)),
            )
        }
    },
  ])
}

/// One transcript step, shared with the Strategy page's planning panel.
pub fn entry_view(base: String, e: Entry) -> Element(msg) {
  case e.kind {
    "message" -> h.div([a.class("entry message")], [md(base, e.text)])
    "thinking" ->
      h.details([a.class("entry thinking")], [
        h.summary([], [h.text("Thinking")]),
        md(base, e.text),
      ])
    "tool" ->
      h.details([a.class("entry tool " <> e.status)], [
        h.summary([], [
          status_mark(e.status, ""),
          h.code([], [h.text(e.title)]),
        ]),
        case e.input {
          "" -> h.text("")
          input -> h.pre([a.class("input")], [h.text(input)])
        },
        h.pre([], [
          h.text(case e.status, e.output {
            "running", _ -> "Running…"
            _, "" -> "(no output)"
            _, output -> output
          }),
        ]),
      ])
    "usage" -> h.small([a.class("entry")], [h.text(e.text)])
    "error" -> h.p([a.class("entry"), a.role("alert")], [h.text(e.text)])
    _ ->
      h.details([a.class("entry")], [
        h.summary([], [h.text("Output")]),
        h.pre([], [h.text(e.text)]),
      ])
  }
}

/// The Scope tab: a switch per project the task can change. While the agent
/// works, projects can be added but not removed (D-127).
fn scope_view(model: Model, t: Task) -> Element(Msg) {
  let working =
    t.paused == "" && list.any(t.runs, fn(r) { r.state == "running" })
  h.div([], [
    projects_pick(
      list.map(model.projects, fn(p) { #(p.dir, p.label) }),
      scope_of(model.dir, t),
      working,
      SetScope,
    ),
    case working {
      True ->
        h.small([], [
          h.text(
            "You can add projects now; remove them once the agent stops working, is paused, or asks a question.",
          ),
        ])
      False -> h.text("")
    },
  ])
}

/// A switch per project, each a folder and its name on screen, for the
/// projects a task can change. `locked`: the chosen ones can't be turned off.
pub fn projects_pick(
  projects: List(#(String, String)),
  chosen: List(String),
  locked: Bool,
  msg: fn(String, Bool) -> msg,
) -> Element(msg) {
  h.fieldset([a.class("projects-pick")], [
    h.legend([], [h.text(ui_label("scope_projects"))]),
    ..list.map(projects, fn(p) {
      let #(dir, label) = p
      let on = list.contains(chosen, dir)
      h.label([], [
        h.input([
          a.type_("checkbox"),
          a.role("switch"),
          a.checked(on),
          a.disabled(locked && on),
          e.on_check(msg(dir, _)),
        ]),
        h.text(label),
      ])
    })
  ])
}

/// The project switches' look: a rounded track and a round thumb.
pub const projects_pick_styles =
  ".projects-pick{margin:8px 0}.projects-pick label{display:flex;align-items:center;gap:10px;margin:6px 0;cursor:pointer}.projects-pick input{appearance:none;-webkit-appearance:none;position:relative;flex:none;width:34px;height:20px;padding:0;margin:0;border:1px solid var(--edge);border-radius:10px;background:var(--well);cursor:pointer;transition:background .15s}.projects-pick input::before{content:\"\";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--paper);border:1px solid var(--edge);box-sizing:border-box;transition:transform .15s}.projects-pick input:checked{background:var(--focus)}.projects-pick input:checked::before{transform:translateX(14px)}.projects-pick input:focus-visible{outline:3px solid var(--focus);outline-offset:2px}.projects-pick[disabled] label,.projects-pick[disabled] input{opacity:.5;cursor:default}@media (prefers-reduced-motion:reduce){.projects-pick input,.projects-pick input::before{transition:none}}\n"

/// The detail's look. Colors come from the page's custom properties, which
/// a nested shadow root inherits from its host: none are set here, so the
/// board's dark theme reaches the detail.
const task_styles =
  control_styles
  <> tool_styles
  <> projects_pick_styles
  <> editor_styles
  <> table_styles
  <> model_styles
  <> ":host{display:block;container-type:inline-size}input,textarea,pre,code,.md,.detail > .heading > h2,.record > .panel > article > p,.record > .panel > p,.record > .panel > ul > li,.changes h4,.changes li,.run > header > strong,.run > header > .state,.run > small{user-select:text;-webkit-user-select:text}
button,input,textarea,select{font:inherit;padding:var(--control-pad);margin:var(--space-1);border:1px solid var(--edge);border-radius:0}button{cursor:pointer;background:var(--paper)}button:hover{background:var(--hover)}button:focus-visible{outline:3px solid var(--focus)}label{display:block}textarea{display:block;width:100%;box-sizing:border-box;min-height:70px}input{width:100%;box-sizing:border-box}.detail{border:1px solid var(--line);padding:16px;border-radius:0}small{display:block;color:var(--soft);overflow-wrap:anywhere}pre{white-space:pre-wrap;font:inherit}.actions{display:flex;flex-wrap:wrap;gap:6px}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}.attention{color:var(--warn-text)}.status{display:inline-block;font-size:13px;padding:1px 8px;border-radius:0;background:var(--well);margin:4px 0}.working::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--ok-text);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before{animation:none}}.note,.run{border:1px solid var(--line);border-radius:0;padding:8px 12px;margin:8px 0;background:var(--paper)}header{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}header small{display:inline}.md p,.md ul,.md ol,.md blockquote{margin:6px 0}.md blockquote{padding-left:10px;border-left:3px solid var(--line);color:var(--soft)}.md pre,.transcript pre{background:var(--code);padding:8px;border-radius:0;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace;max-height:320px}code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:0}.transcript{border-left:3px solid var(--line);padding-left:10px;margin-top:8px}.entry{margin:6px 0}details.entry>summary{cursor:pointer;color:var(--soft)}.muted{color:var(--soft)}.note .delete{float:right;margin:0 0 4px 8px;font-size:13px;padding:0 8px}
.meta{display:flex;gap:10px;align-items:center;color:var(--soft);margin:4px 0 12px}.agent{display:flex;flex-wrap:wrap;gap:4px}.agent>.refresh-models{margin-right:0}.agent>.effort-pick{margin-left:8px}.agent label{display:flex;flex-direction:column}
.tabs{display:flex;flex-wrap:wrap;gap:0;border-bottom:1px solid var(--line);margin-top:16px}.tabs button{margin:0 0 -1px;border-radius:0 0 0;border-color:transparent;background:transparent}.tabs [aria-selected=true]{border-color:var(--line) var(--line) var(--sheet);background:var(--sheet);font-weight:600}.panel{padding-top:8px}.changes h4{font-size:15px;margin:12px 0 4px}.changes ul{margin:0;padding-left:20px}.changes li{white-space:normal;margin:2px 0;font:13px 'IBM Plex Mono',ui-monospace,monospace}
details.note>summary{display:block;min-height:var(--control-h-small);cursor:pointer;list-style:none}details.note>summary>small{display:inline}details.note>summary>*+*{margin-left:8px}details.note>summary::-webkit-details-marker{display:none}details.note>summary::before{content:\"▸\";color:var(--soft);margin-right:8px}details.note[open]>summary::before{content:\"▾\"}details.note[open]>summary{margin-bottom:4px}.md img{max-width:100%;height:auto;border:1px solid var(--line);border-radius:0}.status.finished{background:var(--add);color:var(--ok-text)}.diff{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:8px;border-radius:0;max-width:760px}
.title{display:block;box-sizing:border-box;width:calc(100% - 8px);margin:8px 4px}
.heading{display:flex;align-items:center;flex-wrap:wrap;gap:4px 12px}.heading h2{margin:12px 0}.copy-id{flex:none;margin:0;padding:0 10px;font-size:13px}
@container (min-width:58.5rem){.detail{display:grid;grid-template-columns:minmax(0,0.65fr) minmax(0,1.35fr);column-gap:2rem;align-items:start}.detail > .heading,.detail > .meta{grid-column:1 / -1}.record > .tabs{margin-top:0}}
*{scrollbar-color:var(--line) var(--paper)}::selection{background:var(--sel);color:var(--ink)}
h1,h2,h3{line-height:1.2;letter-spacing:-.02em}a{color:var(--focus);text-underline-offset:3px}
button,input,textarea,select{border:1px solid var(--edge);background:var(--paper);color:var(--ink);caret-color:var(--focus)}input::placeholder,textarea::placeholder{color:var(--soft);opacity:1}
button:hover{background:var(--hover)}button:disabled{color:var(--soft)}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
textarea{font-family:inherit}
.depends .unfinished{color:var(--warn-text);font-weight:600}
.feature,.depends{margin:var(--space-1) 0}
.attention{font-weight:700}
.status{border:1px solid var(--line);background:var(--paper);font-size:11px;text-transform:uppercase;letter-spacing:.08em;padding:1px 8px}.status.finished{background:var(--add);color:var(--ok-text);border-color:var(--ok-text)}.working::before{background:var(--entry)}
.detail,.composer,.note,.run{background:var(--paper);border-color:var(--edge)}.detail{padding:10px 12px}
.tabs{border-bottom:2px solid var(--edge)}.tabs button{border:0;border-right:1px solid var(--line);background:transparent;margin:0;padding:5px 8px}.tabs [aria-selected=true]{background:var(--paper);box-shadow:inset 0 -3px var(--ink);border-color:var(--line)}
.note,.run{padding:4px 6px}article{padding:4px}.panel{padding-top:4px}
article{border-bottom-color:var(--line)}.transcript{border-left:1px solid var(--edge)}
.md pre,.transcript pre,.diff,code{background:var(--code);color:var(--ink)}
@media(prefers-reduced-motion:no-preference){button{transition:background-color .12s ease-out}}
.detail[aria-busy=true]{animation:stale 0s .3s forwards}@keyframes stale{to{opacity:.6}}"
  <> effort_pick_styles

/// The markdown editor's look (`editor`), shared with the pages that use it.
/// Colors come from the page's custom properties. The Image tool is a `label`
/// around a file picker, so it takes the toolbar's box model: its border-box
/// min-height is the toolbar's small height, as a button's is. An action sits
/// at the composer's bottom right; the send icon keeps a right inset on every
/// line, while worded actions reserve their measured room on the last line
/// only, so the other lines keep their width. The actions stay on one line,
/// take at most half the composer, and truncate a label too wide for it.
pub const editor_styles =
  ".composer{position:relative;border:1px solid var(--edge);border-radius:0;margin:8px 4px;background:var(--paper)}.composer solace-md-editor{display:block;border-top:1px solid var(--line)}.composer.has-actions{--md-line-pad:calc(var(--control-h-small) + var(--space-3))}.composer.has-action-words{--md-action-gap:var(--space-2);--md-line-pad:0px}.composer-actions{position:absolute;right:var(--space-2);bottom:var(--space-2);z-index:1;display:flex;flex-wrap:nowrap;justify-content:flex-end;gap:var(--space-2);max-width:calc(50% - var(--space-2))}.composer-action{display:inline-flex;align-items:center;justify-content:center;flex:0 1 auto;min-width:var(--control-h-small);min-height:var(--control-h-small);margin:0;padding:0 var(--space-2);overflow:hidden}.composer-action-text{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.composer-action svg{display:block;flex:none}.note-tools{display:flex;margin:8px 4px}.add-note{display:inline-flex;align-items:center;justify-content:center;width:var(--control-h-small);height:var(--control-h-small);padding:0;margin:0}.add-note svg{display:block}.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:2px}.toolbar button,.toolbar .button{margin:0;padding:4px 10px;font-size:14px}.sr-only{position:absolute;width:1px;height:1px;opacity:0;overflow:hidden}.toolbar .button{position:relative;display:inline-flex;align-items:center;box-sizing:border-box;border:1px solid var(--edge);border-radius:0;background:var(--paper);cursor:pointer}.toolbar .button:hover{background:var(--hover)}.toolbar .button:focus-within{outline:3px solid var(--focus)}
.composer solace-md-editor{border-top:1px solid var(--edge)}.toolbar .button{border-color:var(--edge);background:var(--paper)}.toolbar .button:hover{background:var(--hover)}
"

/// A strategy run's proposal (D-93, Spec 15 §6): a record to create (`id`
/// "") or change, or the vision's new text (`fields` holds `text`). `fields`
/// are the request's, each value as text, the title first; `base` is the
/// vision's revision when proposed; `reopens` are the `complete` or
/// `canceled` records, as (kind, id), that Approve also sets `planned`;
/// `before` is the record's value of each proposed field when it was made.
pub type Proposal {
  Proposal(
    kind: String,
    id: String,
    fields: List(#(String, String)),
    base: String,
    reopens: List(#(String, String)),
    /// The record's current value of each proposed field, as text; the
    /// vision's text under `text`. Empty for a create, and for a proposal
    /// saved before D-115.
    before: Dict(String, String),
  )
}

pub fn proposal_decoder() -> decode.Decoder(Proposal) {
  let text = fn(name, next) {
    decode.optional_field(
      name,
      "",
      decode.one_of(decode.string, [decode.success("")]),
      next,
    )
  }
  use kind <- decode.field("kind", decode.string)
  use id <- text("id")
  use fields <- decode.optional_field(
    "fields",
    [],
    decode.dict(decode.string, field_text())
      |> decode.map(fn(d) { list.sort(dict.to_list(d), field_order) }),
  )
  use base <- text("base")
  use reopens <- decode.optional_field(
    "reopens",
    [],
    decode.list({
      use kind <- decode.field("kind", decode.string)
      use id <- decode.field("id", decode.string)
      decode.success(#(kind, id))
    }),
  )
  use before <- decode.optional_field(
    "before",
    dict.new(),
    decode.dict(decode.string, field_text()),
  )
  decode.success(Proposal(kind, id, fields, base, reopens, before))
}

/// A proposed field's value as text: a list joined, null as "(none)".
fn field_text() -> decode.Decoder(String) {
  decode.one_of(decode.string, [
    decode.list(decode.string)
      |> decode.map(fn(values) {
        case values {
          [] -> "(none)"
          _ -> string.join(values, ", ")
        }
      }),
    decode.int |> decode.map(int.to_string),
    decode.float |> decode.map(float.to_string),
    decode.bool
      |> decode.map(fn(on) {
        case on {
          True -> "yes"
          False -> "no"
        }
      }),
    decode.success("(none)"),
  ])
}

/// The title first, then the other fields by name.
fn field_order(a: #(String, String), b: #(String, String)) -> order.Order {
  case a.0, b.0 {
    "title", _ -> order.Lt
    _, "title" -> order.Gt
    x, y -> string.compare(x, y)
  }
}

/// A strategy run's proposal in human terms: what it creates or changes. An
/// edit's heading links the record, titled by `name`; a create shows the
/// proposed title. An edit shows only the fields that change: a short one as
/// `old → new`, each
/// record id a link titled by `name`, and the body as a line diff with one
/// line of context on each side of a change and each other run of unchanged
/// lines collapsed to one muted line, the new body shown as Markdown below;
/// the vision's new text shows the same diff. Removed lines take the deletion
/// tint and added lines the addition tint, with no strikethrough. A body or
/// text that differs only in the space around its lines, or in blank lines,
/// counts as unchanged. One that changes nothing, the vision included, says
/// "No change". A create, and a proposal saved before `before`, show every
/// field and each record id as a link, the vision its text as Markdown; the
/// records Approve also sets `upcoming` follow, then its options until it is
/// answered, and the answer after, with its record references as links.
/// `name(kind, id)` is a record's title, "" when it isn't loaded.
pub fn proposal_view(
  base: String,
  p: Proposal,
  options: List(String),
  answer: String,
  name: fn(String, String) -> String,
  busy: Bool,
  on_answer: fn(String) -> msg,
) -> Element(msg) {
  let heading = case p.kind, p.id {
    "vision", _ -> [h.text("Replace the vision")]
    kind, "" -> [
      h.text("Create " <> kind),
      case list.key_find(p.fields, "title") {
        Ok(title) -> h.em([], [h.text(" " <> title)])
        Error(_) -> h.text("")
      },
    ]
    kind, id -> [
      h.text("Change " <> kind <> " "),
      record_link(kind, id, name),
    ]
  }
  // D-115: a proposal that knows the record's current values shows only the
  // fields that change, the vision's text among them; a create, and a
  // proposal saved before, show all.
  let comparing = !dict.is_empty(p.before)
  let changed = case comparing {
    True ->
      list.filter(p.fields, fn(f) {
        case dict.get(p.before, f.0), f.0 {
          Ok(old), "body" | Ok(old), "text" ->
            text_lines(old) != text_lines(f.1)
          old, _ -> old != Ok(f.1)
        }
      })
    False -> p.fields
  }
  let fields_view = case comparing, changed, p.kind {
    False, _, "vision" ->
      case list.key_find(p.fields, "text") {
        Ok(text) -> [md(base, text)]
        Error(_) -> []
      }
    False, _, _ -> [
      h.dl(
        [],
        list.flat_map(p.fields, fn(f) {
          [
            h.dt([], [h.text(f.0)]),
            h.dd([], case f.0 {
              "body" -> [md(base, f.1)]
              _ -> short_value(f.0, f.1, name)
            }),
          ]
        }),
      ),
    ]
    True, [], _ -> [h.p([a.class("nochange")], [h.text("No change")])]
    True, fields, _ ->
      case p.kind {
        "vision" ->
          case list.key_find(p.fields, "text") {
            Ok(new) -> [
              text_diff(dict.get(p.before, "text") |> result.unwrap(""), new),
              new_body(base, new),
            ]
            Error(_) -> []
          }
        _ ->
          list.append(
            [
              h.dl(
                [],
                list.flat_map(fields, fn(f) {
                  [
                    h.dt([], [h.text(f.0)]),
                    h.dd([], case f.0 {
                      "body" -> [
                        text_diff(
                          dict.get(p.before, "body") |> result.unwrap(""),
                          f.1,
                        ),
                      ]
                      _ -> old_to_new(p, f, name)
                    }),
                  ]
                }),
              ),
            ],
            case list.key_find(p.fields, "body") {
              Ok(new) -> [new_body(base, new)]
              Error(_) -> []
            },
          )
      }
  }
  h.article([a.class("proposal")], [
    h.h4([], heading),
    ..list.append(
      fields_view,
      list.append(
        list.map(p.reopens, fn(r) {
          h.p([a.class("reopens")], [
            h.text("Approving also sets the " <> r.0 <> " "),
            record_link(r.0, r.1, name),
            h.text(" back to upcoming."),
          ])
        }),
        [
          case answer {
            "" ->
              h.div(
                [a.class("actions")],
                list.map(options, fn(o) {
                  h.button([a.disabled(busy), e.on_click(on_answer(o))], [
                    h.text(o),
                  ])
                }),
              )
            answer -> answer_view(base, answer)
          },
        ],
      ),
    )
  ])
}

/// The proposal display's look (`proposal_view`), for the pages that show
/// it. Colors come from the page's custom properties; removed and added lines
/// take the page's deletion and addition tints, and a collapsed run is muted.
pub const proposal_styles =
  ".proposal{border:1px solid var(--edge);background:var(--paper);padding:8px 12px;margin:8px 0}.proposal h4{margin:0 0 6px;font-size:15px}.proposal dl{display:grid;grid-template-columns:max-content minmax(0,1fr);gap:2px 12px;margin:6px 0}.proposal dt{color:var(--soft);font-size:13px}.proposal dd{margin:0;overflow-wrap:anywhere}.proposal .reopens{color:var(--warn-text);font-weight:600}.proposal .actions{display:flex;gap:6px}.proposal .answer{font-weight:600}.proposal .nochange{color:var(--soft);font-style:italic}.proposal .body-diff{margin:6px 0;max-width:760px}.proposal .body-diff p{margin:0;font:inherit;white-space:pre-wrap;overflow-wrap:anywhere}.proposal .body-diff p.old-value{background:var(--del)}.proposal .body-diff p.new-value{background:var(--add)}.proposal .body-diff p.unchanged{color:var(--soft);font-style:italic}.proposal .body-diff p:last-child{margin-bottom:0}.proposal details.new-body{margin:6px 0}.proposal details.new-body>summary{cursor:pointer;color:var(--soft)}\n"

/// What the task's last run finished with, "" when nothing is to be looked at.
fn outcome(t: Task) -> String {
  case t.finished {
    "finished" -> "Agent finished"
    "reported" -> "Report ready"
    "reviewed" -> "Review ready"
    _ -> ""
  }
}

/// `status` for a task whose project's workflow is known: a task in a final
/// column has no agent at work, whatever runs its copy still shows going.
pub fn status_in(workflow: Workflow, t: Task) -> String {
  case list.find(workflow.columns, fn(c) { c.id == t.column }) {
    Ok(c) if c.final -> outcome(t)
    _ -> status(t)
  }
}

/// `status_badge` by `status_in`: no live agent in a final column.
pub fn status_badge_in(workflow: Workflow, t: Task) -> Element(msg) {
  badge(status_in(workflow, t))
}

fn badge(status: String) -> Element(msg) {
  case status {
    "" -> h.text("")
    s -> {
      let class = case s {
        "Agent working" | "Agent reviewing" | "Agent investigating" ->
          "status working"
        "Agent finished" | "Report ready" | "Review ready" -> "status finished"
        _ -> "status"
      }
      h.span([a.class(class), a.role("status")], [h.text(s)])
    }
  }
}

/// The message a host hands down once its stream has (re)connected and caught
/// up: an event may have been lost meanwhile, so the task is fetched again.
pub const resync = "{\"resync\":true}"

/// Whether a live message puts this element's copy of the task in doubt, so
/// it is fetched again rather than trusted: the host's `resync`, or an update
/// of this task named under another project than the one it is opened from,
/// unless it has no project (D-129) and so is named under its home folder.
pub fn stale(model: Model, text: String) -> Bool {
  case json.parse(text, decode.at(["resync"], decode.bool)) {
    Ok(_) -> True
    Error(_) ->
      case
        json.parse(text, decode.at(["project"], decode.string)),
        json.parse(text, decode.at(["task", "id"], decode.string))
      {
        Ok(dir), Ok(id) ->
          dir != model.dir && id == model.id && !homeless_event(text)
        _, _ -> False
      }
  }
}

/// Fetches the task again: its GET is the whole task as the server has it
/// now, the only one that carries its changes (D-77). Each event and reply
/// ends with one, so a lost or late message can't leave the copy stale.
/// The models come with it, so a `config.toml` edit reaches the pick without
/// a page load.
fn refresh(model: Model) -> Effect(Msg) {
  case model.task {
    Some(t) ->
      effect.batch([
        get(model.base, "/tasks/" <> t.id, Refetched(model.generation, _)),
        get("", "/models", ModelsRefreshed),
        get("", "/role-models", RoleModelsLoaded),
      ])
    None -> effect.none()
  }
}

/// A task's reference in text, `task:<id>`: markdown draws it as a link to
/// the task (D-98).
pub fn reference(id: String) -> String {
  "task:" <> id
}

@external(javascript, "./task_api.mjs", "copy_text")
fn copy_text(text: String, done: fn(Bool) -> Nil) -> Nil

/// How long Copy ID says Copied, or Copy failed.
const copied_ms = 2000

/// Whether a live message says the workspace's list of projects changed.
pub fn projects_changed(text: String) -> Bool {
  json.parse(text, decode.at(["projects"], decode.list(decode.string)))
  |> result.is_ok
}

/// What an action's button says: its `label`, or else its name (D-108).
pub fn action_label(workflow: Workflow, name: String) -> String {
  result.unwrap(dict.get(workflow.labels, name), name)
}

/// A dependency's title and column, as the project that stores it has them.
pub type Dep {
  Dep(title: String, column: String)
}

/// Asks for the dependencies of the shown task not asked for yet, from the
/// root: a dependency may be stored in any project. Its feature and the
/// tasks that depend on it are asked for once too.
fn want_deps(model: Model) -> #(Model, Effect(Msg)) {
  let missing = case model.task {
    Some(t) ->
      list.filter(t.depends, fn(id) {
        result.is_error(list.key_find(model.deps, id))
      })
    None -> []
  }
  let generation = model.generation
  let #(feature, ask_feature) = case model.task {
    Some(t) ->
      case t.feature, model.feature {
        Some(id), Some(#(asked, _)) if asked == id -> #(
          model.feature,
          effect.none(),
        )
        Some(id), _ -> #(
          Some(#(id, None)),
          get("", "/features/find/" <> id, FeatureLoaded(generation, _)),
        )
        None, _ -> #(None, effect.none())
      }
    None -> #(model.feature, effect.none())
  }
  let #(dependents, ask_dependents) = case model.task, model.dependents {
    Some(t), None -> #(
      Some([]),
      get("", "/tasks/" <> t.id <> "/dependents", DependentsLoaded(
        generation,
        _,
      )),
    )
    _, _ -> #(model.dependents, effect.none())
  }
  #(
    Model(
      ..model,
      deps: list.fold(missing, model.deps, fn(deps, id) {
        list.key_set(deps, id, None)
      }),
      feature: feature,
      dependents: dependents,
    ),
    effect.batch([
      ask_feature,
      ask_dependents,
      ..list.map(missing, fn(id) {
        get("", "/tasks/find/" <> id, DepFound(generation, id, _))
      })
    ]),
  )
}

/// The API prefix of project `dir`: as `GET /projects` lists it, else "" for
/// the root's own and `/p/<dir>` for another.
fn base_of(model: Model, dir: String) -> String {
  case list.find(model.projects, fn(p) { p.dir == dir }), dir {
    Ok(p), _ -> p.base
    Error(_), "." -> ""
    Error(_), _ -> "/p/" <> uri.percent_encode(dir)
  }
}

/// Whether a task in `column` is finished: the column is final in the
/// workflow, as `complete` and `cancelled` are (D-87).
pub fn finished(workflow: Workflow, column: String) -> Bool {
  list.any(workflow.columns, fn(c) { c.final && c.id == column })
}

/// The dependencies line: **Depends on** the tasks this one depends on, then
/// **Needed by** the tasks that depend on it, each shown by `dep_view`.
fn depends_view(model: Model, t: Task) -> Element(Msg) {
  let on =
    list.map(t.depends, fn(id) {
      #(id, list.key_find(model.deps, id) |> result.unwrap(None))
    })
  let needed =
    model.dependents
    |> option.unwrap([])
    |> list.map(fn(e) { #(e.0, Some(e.1)) })
  let part = fn(label, entries: List(#(String, Option(Dep)))) {
    case entries {
      [] -> []
      _ -> [
        h.strong([], [h.text(label)]),
        ..list.intersperse(
          list.map(entries, fn(e) { dep_view(model, e.0, e.1) }),
          h.text(", "),
        )
      ]
    }
  }
  case on, needed {
    [], [] -> h.text("")
    _, _ ->
      h.p(
        [a.class("depends")],
        list.flatten([
          part("Depends on ", on),
          case on, needed {
            [_, ..], [_, ..] -> [h.text("; ")]
            _, _ -> []
          },
          part("Needed by ", needed),
        ]),
      )
  }
}

/// The 16 feature colors in rainbow order, red to rose; each is readable as a
/// stripe on the light and dark themes, at least 3:1 against white (D-114).
pub const feature_palette = [
  "#d7263d", "#e8571e", "#d97400", "#b08600", "#9a9200", "#5f9a1a", "#3fa34d",
  "#16a37a", "#0f9696", "#1c9ad6", "#2f72d8", "#3f4fd1", "#6a3fd1", "#9440c9",
  "#c23ab2", "#d93a7f",
]

/// The color of the feature that started `index`th (from 0): each next one is
/// 5 places on along the rainbow, skipping 4, so features started one after
/// another are easy to tell apart. As 5 and 16 share no factor, all 16 colors
/// come before one repeats.
pub fn feature_color(index: Int) -> String {
  let at = { index * 5 } % 16
  case list.drop(feature_palette, at) {
    [color, ..] -> color
    [] -> ""
  }
}

/// Each started feature's id with its color, given the features as (id,
/// colorSlot): the slot the server kept when it started (D-114), so deleting
/// one moves no other's color. A feature not yet started has none.
pub fn feature_colors(
  features: List(#(String, Option(Int))),
) -> List(#(String, String)) {
  features
  |> list.filter_map(fn(f) {
    case f.1 {
      Some(slot) -> Ok(#(f.0, feature_color(slot)))
      None -> Error(Nil)
    }
  })
}

/// A changed short field as `old → new`: a record id is a link titled by
/// `name`, falling back to `kind:id`.
fn old_to_new(
  p: Proposal,
  f: #(String, String),
  name: fn(String, String) -> String,
) -> List(Element(msg)) {
  list.flatten([
    short_value(f.0, dict.get(p.before, f.0) |> result.unwrap("(none)"), name),
    [h.text(" → ")],
    short_value(f.0, f.1, name),
  ])
}

/// The proposed body rendered as Markdown; its section starts open.
fn new_body(base: String, text: String) -> Element(msg) {
  h.details([a.class("new-body"), a.open(True)], [
    h.summary([], [h.text("New body")]),
    md(base, text),
  ])
}

/// A body's lines as change detection compares them: trimmed, blank ones left
/// out, so space around lines and blank lines alone make no change.
fn text_lines(text: String) -> List(String) {
  string.split(text, "\n")
  |> list.map(string.trim)
  |> list.filter(fn(l) { l != "" })
}

/// The text of the fixed buttons the pages share: the board's, the task
/// detail's, Strategy's and Settings' buttons read these, not inline strings.
/// Keys name the button; an action's own label (D-108) comes from the
/// workflow, not from here.
pub const ui_labels: List(#(String, String)) = [
  #("new_task", "New task"),
  #("create_task", "Create task"),
  #("archive", "Archive"),
  #("show_task", "Show task"),
  #("close_all", "Close All"),
  #("dismiss", "Dismiss"),
  #("save", "Save"),
  #("edit", "Edit"),
  #("add_note", "Add note"),
  #("discuss", "Discuss"),
  #("end_discussion", "End Discussion"),
  #("delete", "Delete"),
  #("copy_id", "Copy ID"),
  #("answer", "Answer"),
  #("watch", "Watch"),
  #("start", "Start"),
  #("send", "Send"),
  #("end", "End"),
  #("stop", "Stop"),
  #("new_run", "New run"),
  #("cancel", "Cancel"),
  #("new_feature", "New feature"),
  #("new_milestone", "New milestone"),
  #("new_persona", "New persona"),
  #("hide_activity", "Hide activity"),
  #("open_run", "Open run"),
  #("show", "Show"),
  #("hide", "Hide"),
  #("model", "Model"),
  #("plan", "Plan"),
  #("plan_context", "Plan context"),
  #("projects", "Projects"),
  #("scope_projects", "Projects this task can change"),
  #("contract_view", "Contract view"),
  #("remove", "Remove"),
  #("add", "Add"),
  #("create_project", "Create project"),
  #("find_task", "Find a task"),
  #("dependencies", "Dependencies"),
  #("effort", "Effort"),
  #("pick_model", "Select a model"),
  #("try_again", "Try again"),
  #("project", "Project"),
  #("remove_only", "Remove only"),
  #("delete_permanently", "Delete permanently"),
  #("edit_milestone", "Edit milestone"),
  #("delete_note", "Delete note"),
  #("plan_marker", "A plan run is open on this"),
  #("plan_open_here", "A plan run on this is open."),
]

/// What a shared button says: its text in `ui_labels`. An unknown key
/// says nothing, so a raw key never reaches a button.
pub fn ui_label(key: String) -> String {
  result.unwrap(list.key_find(ui_labels, key), "")
}

/// What a column is called: its label from the workflow, or its id when the
/// workflow doesn't list it.
pub fn column_label(workflow: Workflow, id: String) -> String {
  case list.find(workflow.columns, fn(c) { c.id == id }) {
    Ok(c) -> c.label
    Error(_) -> id
  }
}

/// Buttons, inputs, text areas, selects and their option lists, in the
/// theme's tokens only, so the dark theme reaches the open list of a select
/// too, and the two control heights of `spacing_styles`. Pages put it first,
/// so their own rules for a control win.
pub const control_styles =
  spacing_styles
  <> "button,input,textarea,select{border:1px solid var(--edge);background:var(--paper);color:var(--ink);caret-color:var(--focus)}option,optgroup{background:var(--paper);color:var(--ink)}option:disabled{color:var(--soft)}input::placeholder,textarea::placeholder{color:var(--soft);opacity:1}button:hover,select:hover{background:var(--hover)}button:disabled{color:var(--soft)}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible{outline:3px solid var(--focus);outline-offset:3px}
button,select,input:not([type=checkbox]):not([type=radio]):not([type=file]){box-sizing:border-box;min-height:var(--control-h)}button.edit,button.toggle,button.act,button.copy-id,button.delete,.toolbar button,.toolbar .button,.unavailable button{min-height:var(--control-h-small)}
"

/// A select's placeholder option: shown only while nothing is picked, it
/// names the choice and can't be pressed. Pages share it for their model
/// picks.
pub fn placeholder_option(text: String, picked: String) -> List(Element(msg)) {
  case picked {
    "" -> [h.option([a.value(""), a.disabled(True), a.selected(True)], text)]
    _ -> []
  }
}

/// Defines `<solace-md-editor>` in a browser, loading CodeMirror; elsewhere
/// it does nothing.
@external(javascript, "./task_api.mjs", "define_editor")
fn define_editor() -> Nil

/// Whether a live message marks a task with no project (D-129).
fn homeless_event(text: String) -> Bool {
  json.parse(text, decode.at(["noProject"], decode.bool)) == Ok(True)
}

/// Where a dependency is asked for, from its `GET /tasks/find/{id}` reply:
/// the root's base for one with no project (D-129), else its project's.
/// An error when the reply names no project folder (the task wasn't found).
pub fn dep_base(model: Model, text: String) -> Result(String, Nil) {
  case
    json.parse(text, decode.at(["dir"], decode.string)),
    json.parse(text, decode.at(["project"], decode.bool))
  {
    Ok(_), Ok(False) -> Ok(base_of(model, "."))
    Ok(dir), _ -> Ok(base_of(model, dir))
    Error(_), _ -> Error(Nil)
  }
}

fn agent_pick(
  model: Model,
  ready: List(Agent),
  chosen: Result(Agent, Nil),
) -> Element(Msg) {
  let picked = case list.find(ready, fn(m) { m.id == model.model }) {
    Ok(_) -> model.model
    Error(_) -> ""
  }
  h.div([a.class("agent")], [
    refresh_button(TryAgain, model.refreshing),
    h.select(
      [
        a.value(picked),
        a.attribute("aria-label", ui_label("model")),
        e.on_input(AgentSelected),
      ],
      list.append(
        placeholder_option(ui_label("pick_model"), picked),
        list.map(ready, fn(m) {
          h.option([a.value(m.id), a.selected(m.id == picked)], agent_option(m))
        }),
      ),
    ),
    case chosen {
      Ok(m) if m.efforts != [] ->
        effort_pick(
          "effort-" <> model.id,
          model.effort,
          m.effort,
          m.efforts,
          EffortSelected,
        )
      _ -> h.text("")
    },
  ])
}

/// Under a model pick: how many models can't start now, opening why each
/// can't, so the human knows what to install or sign in to.
pub fn unavailable_view(models: List(Agent)) -> Element(msg) {
  let any_ready = list.any(models, fn(m) { m.available })
  case list.filter(models, fn(m) { !m.available }) {
    [] -> h.text("")
    gone ->
      h.details([a.class("unavailable")], [
        h.summary([], [
          h.text(unavailable_count(list.length(gone), any_ready)),
        ]),
        h.ul(
          [],
          list.map(gone, fn(m) {
            h.li([], [h.text(m.label <> ": " <> m.reason)])
          }),
        ),
      ])
  }
}

/// "2 more models unavailable" under a pick; without one, "2 models unavailable".
pub fn unavailable_count(n: Int, more: Bool) -> String {
  int.to_string(n)
  <> case more {
    True -> " more "
    False -> " "
  }
  <> case n {
    1 -> "model unavailable"
    _ -> "models unavailable"
  }
}

/// What to set up when no model can start: an agent program, signed in
/// (the README's "Get an AI agent"). When every model is held the install
/// steps would mislead: the agents are installed, and `unavailable_view`
/// shows each reason; the refresh button above checks them again.
pub fn no_agents_view(models: List(Agent)) -> Element(msg) {
  case list.any(models, fn(m) { !m.retry }) {
    False ->
      h.div([a.class("no-agents"), a.role("status")], [
        h.p([], [
          h.text(case list.any(models, fn(m) { m.sign_in }) {
            True ->
              "Every model is held. Each reason is shown below; the ⟳ button above checks them again."
            False ->
              "Every model is held by a usage limit. Each reason is shown below; the ⟳ button above checks them again."
          }),
        ]),
      ])
    True ->
      h.div([a.class("no-agents"), a.role("status")], [
        h.p([], [
          h.text(
            "No model can start yet. Install one of these agents, run it once and sign in, then reload this page:",
          ),
        ]),
        h.ul([], [
          h.li([], [
            h.strong([], [h.text("Claude Code: ")]),
            h.code([], [
              h.text("curl -fsSL https://claude.ai/install.sh | bash"),
            ]),
            h.text(", then run "),
            h.code([], [h.text("claude")]),
            h.text("."),
          ]),
          h.li([], [
            h.strong([], [h.text("Codex CLI: ")]),
            h.code([], [h.text("brew install codex")]),
            h.text(
              " on macOS, or a release binary from github.com/openai/codex/releases, then run ",
            ),
            h.code([], [h.text("codex")]),
            h.text("."),
          ]),
        ]),
      ])
  }
}

/// The model pick's lines: the unavailable models and what to set up. Pages
/// with a pick add it to their styles; colors come from the page.
pub const model_styles =
  ".unavailable,.no-agents{flex-basis:100%;font-size:13px;color:var(--soft);margin:2px 0}.unavailable>summary{cursor:pointer}.unavailable ul,.no-agents ul{margin:4px 0;padding-left:20px}.unavailable li,.no-agents li{white-space:normal;margin:2px 0}.unavailable button{font-size:13px;padding:1px 8px;margin:0 0 0 8px}.no-agents{color:var(--ink)}\n"

/// The spacing and control sizes every page shares: pages and the detail
/// use these instead of their own numbers. Custom properties inherit,
/// so a nested shadow root gets them from its host too. Two control heights:
/// `--control-h` for buttons, inputs and selects, `--control-h-small` for
/// the small ones beside text (Edit, Copy ID, the editor's toolbar). The
/// heading sizes `--h2`, `--h3`, `--h4` and the tab buttons' padding
/// `--tab-pad` are the sizes the pages settled on, so a page names them
/// instead of repeating the numbers.
pub const spacing_styles =
  ":host{--space-1:4px;--space-2:8px;--space-3:12px;--space-4:16px;--space-5:20px;--space-6:24px;--control-h:36px;--control-h-small:32px;--control-pad:6px 12px;--pad-card:12px 16px;--pad-section:20px 24px;--h2:19px;--h3:16px;--h4:15px;--tab-pad:10px 16px}\n"

/// A task's column, status and whether it has no project: the line under
/// its title, or beside it in the board's bar.
pub fn meta(workflow: Workflow, t: Task) -> List(Element(msg)) {
  [
    h.text(column_label(workflow, t.column)),
    status_badge_in(workflow, t),
    case t.no_project {
      True -> h.text(" • no project")
      False -> h.text("")
    },
  ]
}

/// The Copy ID button: `said` is what it says for a moment after a copy,
/// "" for Copy ID.
pub fn copy_button(id: String, said: String, on_click: msg) -> Element(msg) {
  h.button(
    [
      a.class("copy-id"),
      a.title(
        "Copies "
        <> reference(id)
        <> ": pasted into any text, it links to this task",
      ),
      e.on_click(on_click),
    ],
    [
      h.text(case said {
        "" -> ui_label("copy_id")
        said -> said
      }),
    ],
  )
}

/// Puts a task's reference on the clipboard (D-98); `done` hears whether it
/// took it.
pub fn copy_reference(id: String, done: fn(Bool) -> Nil) -> Nil {
  copy_text(reference(id), done)
}

/// What Copy ID says for a moment after a copy.
pub fn copy_said(ok: Bool) -> String {
  case ok {
    True -> "Copied"
    False -> "Copy failed"
  }
}

/// Calls `then` once Copy ID's moment of saying Copied has passed.
pub fn after_copied(then: fn() -> Nil) -> Nil {
  after(copied_ms, then)
}

/// A model pick's option: the model's one name, its label (D-150). Every entry has a label, a
/// `run` argv entry has no model string, and an alias's label follows the model a run used.
pub fn agent_option(m: Agent) -> String {
  m.label
}

/// A modal dialog: a backdrop over the page that blocks the page behind it.
/// Pages add `modal_styles` to their `<style>` and put their content in `body`.
pub fn modal(body: List(Element(msg))) -> Element(msg) {
  h.div([a.class("modal-backdrop")], [
    h.div(
      [
        a.class("modal"),
        a.attribute("role", "alertdialog"),
        a.attribute("aria-modal", "true"),
      ],
      body,
    ),
  ])
}

/// The modal dialog's look (`modal`), for the pages that show one. Colors
/// come from the page's custom properties, `--shadow` among them.
pub const modal_styles =
  ".modal-backdrop{position:fixed;inset:0;z-index:20;display:flex;align-items:center;justify-content:center;background:color-mix(in srgb,var(--ink) 15%,transparent);padding:16px}.modal{background:var(--paper);color:var(--ink);border:2px solid var(--edge);box-shadow:0 8px 32px var(--shadow);padding:20px 24px;width:min(480px,100%);max-height:100%;overflow:auto}\n"

/// Drops what belonged to the task shown before.
fn cleared(model: Model) -> Model {
  Model(
    ..model,
    task: None,
    staged: None,
    model: "",
    effort: "",
    model_source: RoleDefault,
    text: "",
    writing: False,
    noting: False,
    watching: "",
    entries: [],
    review_summary: "",
    editing: False,
    acting: "",
    error: "",
    copy: #(model.copy.0, ""),
    deps: [],
    feature: None,
    dependents: None,
  )
}

/// Swaps in the task being loaded once it and its workflow are both in.
fn settle(model: Model) -> #(Model, Effect(Msg)) {
  case model.staged {
    Some(#(Some(task), Some(w))) ->
      show(Model(..cleared(model), workflow: w), task)
    _ -> #(model, effect.none())
  }
}

/// A load that fails while another task is staged drops the one shown
/// before: it would otherwise stay on screen as if it were the new one. All
/// that was its goes with it, as `open` does, since a later reply may show
/// the new task.
fn unstage(model: Model) -> Model {
  case model.staged {
    Some(_) ->
      Model(
        ..cleared(model),
        workflow: Workflow([], dict.new()),
        action_roles: dict.new(),
      )
    None -> model
  }
}

/// Shows `task`, fetched whole.
fn show(model: Model, task: Task) -> #(Model, Effect(Msg)) {
  let shown = model.revealing != "" && model.revealing == task.id
  let keep_tab = shown && model.tab != "details"
  let model =
    choose_role_model(resume(
      Model(
        ..model,
        task: Some(task),
        title: task.title,
        body: task.body,
        rank: float.to_string(task.rank),
        error: "",
        revealing: case shown {
          True -> ""
          False -> model.revealing
        },
      ),
      task,
      keep_tab,
    ))
  let #(model, deps) = want_deps(model)
  #(
    model,
    effect.batch([
      review(model, task),
      deps,
      case shown {
        True -> reveal()
        False -> effect.none()
      },
    ]),
  )
}

/// Asks again for the task being loaded; its reply fills the staged slot.
fn reload(model: Model) -> Effect(Msg) {
  get(model.base, "/tasks/" <> model.id, TaskLoaded(model.generation, _))
}

/// The **Needed by** list once task `d` is as an event has it: in it while
/// `d` depends on task `t` and isn't archived, else out of it.
fn needed_by(model: Model, t: Task, d: Task) -> Option(List(#(String, Dep))) {
  case model.dependents {
    Some(dependents) ->
      case list.contains(d.depends, t.id) && !d.archived {
        True -> Some(list.key_set(dependents, d.id, Dep(d.title, d.column)))
        False -> Some(list.filter(dependents, fn(e) { e.0 != d.id }))
      }
    None -> None
  }
}

/// The **Feature** line: a link to the task's feature with its title; one
/// that can't be found shows its id. No feature, no line.
fn feature_view(model: Model, t: Task) -> Element(Msg) {
  case t.feature {
    None -> h.text("")
    Some(id) -> {
      let title = case model.feature {
        Some(#(asked, Some(title))) if asked == id -> title
        _ -> id
      }
      h.p([a.class("feature")], [
        h.strong([], [h.text("Feature ")]),
        h.a([a.href("/feature/" <> id), a.attribute("data-feature", id)], [
          h.text(title),
        ]),
      ])
    }
  }
}

/// One task of the dependencies line: a link to it (D-98) with its column,
/// an unfinished one marked; one that can't be found shows its id.
fn dep_view(model: Model, id: String, dep: Option(Dep)) -> Element(Msg) {
  let link = fn(label) {
    h.a([a.href("/task/" <> id), a.attribute("data-task", id)], [
      h.text(label),
    ])
  }
  case dep {
    Some(d) -> {
      let done = finished(model.workflow, d.column)
      let column =
        list.find(model.workflow.columns, fn(c) { c.id == d.column })
        |> result.map(fn(c) { c.label })
        |> result.unwrap(d.column)
      h.span(
        [
          a.class(case done {
            True -> "dep"
            False -> "dep unfinished"
          }),
        ],
        [
          link(d.title),
          h.text(
            " ("
            <> column
            <> case done {
              True -> ")"
              False -> ", unfinished)"
            },
          ),
        ],
      )
    }
    None -> h.span([a.class("dep")], [link(id)])
  }
}

/// The declarations every small control shares: the page's font at the small
/// control's 13px, on paper with the shared edge, its height and a pointing
/// cursor. Where the controls differ is their padding and margins, so each
/// rule adds those itself.
pub const small_control =
  "font:inherit;font-size:13px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;min-height:var(--control-h-small);cursor:pointer"

/// Whether a reply says its request's connection was lost, so an action may
/// have run even though its reply never arrived.
fn connection_lost(text: String) -> Bool {
  json.parse(text, decode.at(["connectionLost"], decode.bool))
  |> result.unwrap(False)
}

/// What the UI says while it checks a task after a lost reply.
const checking = "Connection lost; checking the task…"

pub fn run_model_label(
  models: List(Agent),
  model: String,
  used: String,
) -> String {
  let id = case used {
    "" -> model
    used -> used
  }
  case list.find(models, fn(m) { m.id == id }) {
    Ok(m) -> agent_option(m)
    Error(_) -> id
  }
}

/// The run transcript's task-status mark, shared with pages that list tasks.
/// `label` names the outcome to a screen reader; "" leaves the icon unnamed,
/// as a transcript tool's status is.
pub fn status_mark(status: String, label: String) -> Element(msg) {
  h.span([a.class("tool " <> status)], [
    h.span(
      [
        a.class("icon"),
        case label {
          "" -> a.none()
          _ -> a.role("img")
        },
        case label {
          "" -> a.none()
          _ -> a.attribute("aria-label", label)
        },
      ],
      [
        h.text(case status {
          "ok" -> "✓"
          "failed" -> "✗"
          _ -> "…"
        }),
      ],
    ),
  ])
}

/// The ✓/✗ task marks' layout and colors, shared by the task detail and the
/// pages that list tasks.
pub const tool_styles =
  ".tool .icon{display:inline-block;width:1.2em}.tool.ok .icon{color:var(--ok-text)}.tool.failed .icon{color:var(--revision)}\n"

/// A record reference as a link, titled by `name`, or `kind:id` while the
/// record isn't loaded.
fn record_link(
  kind: String,
  id: String,
  name: fn(String, String) -> String,
) -> Element(msg) {
  case id {
    "" | "(none)" -> h.text(id)
    _ -> {
      let shown = case name(kind, id) {
        "" -> kind <> ":" <> id
        title -> title
      }
      h.a([a.href("/" <> kind <> "/" <> id), a.attribute("data-" <> kind, id)], [
        h.text(shown),
      ])
    }
  }
}

/// A short field's value: a record id as its link, `dependsOn` as links to
/// its comma-joined task ids, anything else as text.
fn short_value(
  field: String,
  value: String,
  name: fn(String, String) -> String,
) -> List(Element(msg)) {
  case field {
    "feature" | "milestone" -> [record_link(field, value, name)]
    "dependsOn" ->
      case value {
        "" | "(none)" -> [h.text(value)]
        _ ->
          string.split(value, ", ")
          |> list.map(fn(id) { record_link("task", id, name) })
          |> list.intersperse(h.text(", "))
      }
    _ -> [h.text(value)]
  }
}

/// The effort pick's look: an inline name and fixed rem segments that never
/// wrap, drawn as the page's small controls, with every segment shaded on
/// hover and the chosen one in the menu's ink-and-paper contrast.
pub const effort_pick_styles =
  ".effort-pick{display:flex;align-items:center;gap:.5rem;border:0;padding:0;margin:.5rem 0;width:max-content;min-width:0}.effort-pick>.effort-name{font-size:.8125rem;color:var(--soft);line-height:1;white-space:nowrap}.effort-pick .effort-stops{display:flex;flex-wrap:nowrap}.effort-pick label{position:relative;display:block;flex:none;width:2.375rem;margin:0;font-size:.8125rem;line-height:1;cursor:pointer}.effort-pick label+label{margin-left:-1px}.effort-pick input{position:absolute;width:1px;height:1px;margin:0;opacity:0;pointer-events:none}.effort-pick label span{display:flex;align-items:center;justify-content:center;box-sizing:border-box;width:100%;height:var(--control-h-small);padding:0 .125rem;white-space:nowrap;overflow:hidden;"
  <> small_control
  <> ";transition:background .15s}.effort-pick label:hover span{background:var(--hover)}.effort-pick input:checked+span{background:var(--ink);color:var(--paper);font-weight:600}.effort-pick label:hover input:checked+span{background:var(--hover);color:var(--ink)}.effort-pick input:focus-visible{outline:none}.effort-pick input:focus-visible+span{outline:3px solid var(--focus);outline-offset:2px}@media (prefers-reduced-motion:reduce){.effort-pick span{transition:none}}"

/// A segmented effort's short label; the full name stays in its title.
pub fn effort_label(effort: String) -> String {
  case effort {
    "low" -> "Low"
    "medium" -> "Med"
    "high" -> "High"
    "xhigh" -> "X-Hi"
    "max" -> "Max"
    _ -> string.capitalise(effort)
  }
}

// Effort labels shrink to fit their fixed segment, down to 0.8em; the
// title keeps the full name.
fn effort_size(label: String) -> String {
  let n = int.min(5, int.max(4, string.length(label)))
  "calc(4em / " <> int.to_string(n) <> ")"
}

/// The effort choice: each effort in the agent's order, as a segmented
/// control of native radios with its name beside it. The model's default
/// shows as chosen while none is; choosing it sends "", so the task keeps
/// following the model, and any other effort sends its name (D-149).
pub fn effort_pick(
  group: String,
  chosen: String,
  default_effort: String,
  efforts: List(String),
  on_pick: fn(String) -> msg,
) -> Element(msg) {
  h.div(
    [
      a.class("effort-pick"),
      a.attribute("role", "group"),
      a.attribute("aria-label", ui_label("effort")),
    ],
    [
      h.span([a.class("effort-name")], [h.text(ui_label("effort"))]),
      h.div(
        [a.class("effort-stops")],
        list.map(efforts, fn(effort) {
          effort_stop(
            group,
            effort,
            default_effort,
            effort_chosen(effort, chosen, default_effort),
            on_pick,
          )
        }),
      ),
    ],
  )
}

fn effort_stop(
  group: String,
  effort: String,
  default_effort: String,
  checked: Bool,
  on_pick: fn(String) -> msg,
) -> Element(msg) {
  let label = effort_label(effort)
  h.label([a.class("effort-stop"), a.title(effort)], [
    h.input([
      a.type_("radio"),
      a.name(group),
      a.value(effort_value(effort, default_effort)),
      a.checked(checked),
      e.on_input(on_pick),
    ]),
    h.span([a.style("font-size", effort_size(label))], [h.text(label)]),
  ])
}

/// The value an effort segment sends: the model's default sends "", so a
/// choice of it keeps following the model, and any other sends its name.
pub fn effort_value(effort: String, default_effort: String) -> String {
  case effort == default_effort {
    True -> ""
    False -> effort
  }
}

fn effort_chosen(
  effort: String,
  chosen: String,
  default_effort: String,
) -> Bool {
  case chosen {
    "" -> effort == default_effort
    _ -> effort == chosen
  }
}

/// An answer as Markdown, so a record reference in it is a link.
pub fn answer_view(base: String, answer: String) -> Element(msg) {
  h.div([a.class("answer")], [md(base, "Answer: " <> answer)])
}

/// The icon button left of a model pick: it rechecks every model and clears
/// the holds a run left, except a usage limit whose reset has not come,
/// sending `on_click`.
pub fn refresh_button(on_click: msg, busy: Bool) -> Element(msg) {
  h.button(
    [
      a.type_("button"),
      a.class("refresh-models"),
      a.attribute("aria-label", ui_label("try_again")),
      a.title(ui_label("try_again")),
      a.disabled(busy),
      e.on_click(on_click),
    ],
    [h.text("⟳")],
  )
}

/// One of the buttons at an editor's bottom right: words, or the send icon
/// when `content` is empty, and the message its press sends.
pub type EditorAction(msg) {
  EditorAction(content: String, name: String, message: msg, disabled: Bool)
}

fn editor_action(action: EditorAction(msg)) -> Element(msg) {
  let press = case action.disabled {
    True -> []
    False -> [e.on_click(action.message)]
  }
  h.button(
    [
      a.type_("button"),
      a.class("composer-action"),
      a.attribute("aria-label", action.name),
      a.disabled(action.disabled),
      ..press
    ],
    [
      case action.content {
        "" -> send_icon()
        words -> h.span([a.class("composer-action-text")], [h.text(words)])
      },
    ],
  )
}

fn send_icon() -> Element(msg) {
  let svg = "http://www.w3.org/2000/svg"
  element.namespaced(
    svg,
    "svg",
    [
      a.attribute("viewBox", "0 0 24 24"),
      a.attribute("width", "20"),
      a.attribute("height", "20"),
      a.attribute("fill", "currentColor"),
      a.attribute("aria-hidden", "true"),
      a.attribute("focusable", "false"),
    ],
    [
      element.namespaced(
        svg,
        "path",
        [a.attribute("d", "M2 21L23 12 2 3v7l15 2-15 2z")],
        [],
      ),
    ],
  )
}

/// What the note editor offers: its send, then Discuss when no run is open,
/// or End Discussion while a discussion is going.
fn note_actions(model: Model, t: Task) -> List(EditorAction(Msg)) {
  let active = list.find(t.runs, fn(r) { r.state != "ended" })
  let blank = string.trim(model.text) == ""
  let send = fn(label) {
    EditorAction("", label, AddNote, blank || model.noting)
  }
  case active {
    Ok(r) if r.kind == "discuss" -> [
      send(ui_label("send")),
      EditorAction(
        ui_label("end_discussion"),
        ui_label("end_discussion"),
        Action("end_discussion"),
        False,
      ),
    ]
    Ok(_) -> [send(ui_label("add_note"))]
    Error(_) -> [
      send(ui_label("add_note")),
      EditorAction(
        ui_label("discuss"),
        ui_label("discuss"),
        Action("discuss"),
        blank,
      ),
    ]
  }
}

/// The fields of an action's request: its name, the chosen model and effort,
/// and the editor's text when it isn't empty.
pub fn action_body(model: Model, name: String) -> String {
  let #(chosen, effort) = action_choice(model, name)
  let fields = [#("action", json.string(name))]
  let fields = case chosen {
    "" -> fields
    id -> [#("model", json.string(id)), ..fields]
  }
  let fields = case effort {
    "" -> fields
    effort -> [#("effort", json.string(effort)), ..fields]
  }
  let fields = case model.text {
    "" -> fields
    text -> [#("text", json.string(text)), ..fields]
  }
  json.to_string(json.object(fields))
}

/// The plus button at the top of the Notes stack: it opens the editor and
/// sends nothing.
fn add_note_button() -> Element(Msg) {
  h.button(
    [
      a.class("add-note"),
      a.title(ui_label("add_note")),
      a.attribute("aria-label", ui_label("add_note")),
      e.on_click(WriteNote),
    ],
    [plus_icon()],
  )
}

fn plus_icon() -> Element(msg) {
  let svg = "http://www.w3.org/2000/svg"
  element.namespaced(
    svg,
    "svg",
    [
      a.attribute("viewBox", "0 0 24 24"),
      a.attribute("width", "20"),
      a.attribute("height", "20"),
      a.attribute("fill", "currentColor"),
      a.attribute("aria-hidden", "true"),
      a.attribute("focusable", "false"),
    ],
    [
      element.namespaced(
        svg,
        "path",
        [a.attribute("d", "M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z")],
        [],
      ),
    ],
  )
}

/// The oldest question no one has answered.
pub fn first_unanswered(t: Task) -> Option(String) {
  case list.find(t.questions, fn(q) { q.answer == "" }) {
    Ok(q) -> Some(q.id)
    Error(_) -> None
  }
}

@external(javascript, "./task_api.mjs", "focus_editor")
fn focus_editor(root: dynamic.Dynamic, id: String) -> Nil

/// Everything an editor needs to draw: its identity and text, the messages
/// its input and tools send, and the actions at its bottom right.
pub type EditorConfig(msg) {
  EditorConfig(
    id: String,
    task: String,
    base: String,
    label: String,
    /// The hint shown while the editor is empty; "" shows the label.
    placeholder: String,
    text: String,
    on_input: fn(String) -> msg,
    on_format: fn(String, String) -> msg,
    on_block: fn(String) -> msg,
    actions: List(EditorAction(msg)),
  )
}

fn write_note(model: Model) -> #(Model, Effect(Msg)) {
  let text = case model.id {
    "" -> model.text
    id ->
      case draft_load("note-text", id) {
        "" -> model.text
        draft -> draft
      }
  }
  #(
    Model(..model, tab: "notes", writing: True, text: text),
    effect.before_paint(fn(_, root) { focus_editor(root, "note-text") }),
  )
}

fn add_note(model: Model) -> #(Model, Effect(Msg)) {
  case model.task {
    None -> #(model, effect.none())
    Some(_) if model.noting -> #(model, effect.none())
    Some(t) -> {
      let text = model.text
      #(
        Model(..model, tab: "notes", noting: True),
        send(
          model,
          "/tasks/" <> t.id <> "/notes",
          "POST",
          [#("text", json.string(text))],
          fn(generation, reply) { Noted(generation, text, reply) },
        ),
      )
    }
  }
}

fn delete_note(model: Model, i: Int) -> #(Model, Effect(Msg)) {
  case model.task {
    None -> #(model, effect.none())
    Some(t) ->
      case confirm("Delete this note? It can't be restored.") {
        False -> #(model, effect.none())
        True -> #(
          model,
          send(
            model,
            "/tasks/" <> t.id <> "/notes/" <> int.to_string(i),
            "DELETE",
            [],
            ChangedAt,
          ),
        )
      }
  }
}

fn answer_question(
  model: Model,
  qid: String,
  option: String,
) -> #(Model, Effect(Msg)) {
  case model.task {
    None -> #(model, effect.none())
    Some(t) -> {
      let answer = case option {
        "" -> #("text", json.string(model.text))
        option -> #("option", json.string(option))
      }
      let sent = case option {
        "" -> model.text
        _ -> ""
      }
      #(
        model,
        send(
          model,
          "/tasks/" <> t.id <> "/questions/" <> qid,
          "POST",
          [
            answer,
          ],
          fn(generation, text) {
            ChangedClearing(generation, "note-text", sent, text)
          },
        ),
      )
    }
  }
}

/// The storage key of a draft: the editor's id and the record or context it edits.
@external(javascript, "./task_api.mjs", "draft_key")
pub fn draft_key(editor: String, owner: String) -> String

/// A draft saved for `editor` on `owner`, "" when there is none.
@external(javascript, "./task_api.mjs", "load_draft")
pub fn draft_load(editor: String, owner: String) -> String

/// Saves a draft, removing it when the text is blank.
@external(javascript, "./task_api.mjs", "save_draft")
pub fn draft_save(editor: String, owner: String, text: String) -> Nil

/// Removes a draft.
@external(javascript, "./task_api.mjs", "clear_draft")
pub fn draft_clear(editor: String, owner: String) -> Nil

/// Puts back the drafts this task has, opening the editors they belong to. A
/// draft and the task's own text are separate: a stored draft is only there
/// when the human typed it.
fn resume(model: Model, task: Task, keep_tab: Bool) -> Model {
  let note = draft_load("note-text", task.id)
  let body = draft_load("edit-body", task.id)
  let tab = case keep_tab {
    True -> model.tab
    False ->
      case body, note {
        "", "" -> model.tab
        "", _ ->
          case first_unanswered(task) {
            Some(_) -> "questions"
            None -> "notes"
          }
        _, _ -> "details"
      }
  }
  Model(
    ..model,
    text: note,
    writing: note != "",
    body: case body {
      "" -> task.body
      body -> body
    },
    editing: body != "",
    tab: tab,
  )
}

fn changed(
  model: Model,
  clearing: String,
  sent: String,
  text: String,
) -> #(Model, Effect(Msg)) {
  case json.parse(text, task_decoder()) {
    // A reply for the task shown before another opened.
    Ok(task) if task.id != model.id -> #(model, effect.none())
    Ok(task) -> {
      // A reply carries no `changes` (D-77): keep the list until it is fetched again.
      let task = case model.task {
        Some(old) if old.id == task.id -> Task(..task, changes: old.changes)
        _ -> task
      }
      // A task whose home left its projects now lives in the first one left (D-61).
      // A task with no project (D-129) has no projects to follow.
      let #(model, moved) = case
        !task.no_project
        && model.dir != ""
        && !list.contains(scope_of(model.dir, task), model.dir),
        task.scope
      {
        True, [dir, ..] -> follow(model, dir)
        _, _ -> #(model, effect.none())
      }
      let clears = case clearing {
        "note-text" -> model.text == sent
        "edit-body" -> model.body == sent
        _ -> False
      }
      case clears {
        True -> draft_clear(clearing, task.id)
        False -> Nil
      }
      let closes_edit = clearing == "edit-body" && clears
      let editing = model.editing && !closes_edit
      let note_cleared = clearing == "note-text" && clears
      let model =
        Model(
          ..model,
          task: Some(task),
          title: case editing {
            True -> model.title
            False -> task.title
          },
          body: case editing {
            True -> model.body
            False -> task.body
          },
          rank: case editing {
            True -> model.rank
            False -> float.to_string(task.rank)
          },
          text: case note_cleared {
            True -> ""
            False -> model.text
          },
          writing: case note_cleared {
            True -> False
            False -> model.writing
          },
          error: "",
          editing: editing,
        )
      let #(model, deps) = want_deps(model)
      #(model, effect.batch([moved, refresh(model), deps]))
    }
    Error(_) ->
      case connection_lost(text) {
        True -> #(Model(..model, error: checking), refresh(model))
        False -> #(Model(..model, error: error(text)), effect.none())
      }
  }
}

fn acted(
  model: Model,
  clearing: String,
  sent: String,
  text: String,
) -> #(Model, Effect(Msg)) {
  case model.acting, connection_lost(text) {
    "", _ -> #(model, effect.none())
    _, True -> {
      let model = Model(..model, acting: "", error: checking)
      #(model, refresh(model))
    }
    _, False -> changed(Model(..model, acting: ""), clearing, sent, text)
  }
}

fn connection_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    // Added again (or for the first time): the task is asked for afresh, and a
    // followed run's transcript refreshes in one chain.
    Connected -> {
      let model = Model(..model, connected: True)
      Some(#(
        model,
        effect.batch([
          get("", "/models", ModelsLoaded),
          get("", "/role-models", RoleModelsLoaded),
          get("", "/projects", ProjectsLoaded),
          fetch(model),
          case model.watching {
            "" -> effect.none()
            rid -> transcript(model, rid)
          },
          effect.before_paint(fn(send, root) { send(Attached(root)) }),
        ]),
      ))
    }
    Attached(root) ->
      Some(#(
        Model(..model, root: root),
        effect.from(fn(send) {
          images(root, fn(id, image) { send(Format(id, image, "")) }, fn(text) {
            send(Failed(text))
          })
        }),
      ))
    Listening(source) ->
      Some(#(Model(..model, source: Some(source)), effect.none()))
    // Replies may never come once removed: the task shown before goes, and
    // `Connected` asks for the new one again.
    Disconnected ->
      Some(#(
        Model(
          ..unstage(stop(model)),
          connected: False,
          noting: False,
          generation: model.generation + 1,
        ),
        effect.none(),
      ))
    TaskChanged(id) if id == model.id -> Some(#(model, effect.none()))
    TaskChanged(id) -> Some(open(Model(..model, id: id)))
    BaseChanged(base) if base == model.base -> Some(#(model, effect.none()))
    BaseChanged(base) -> Some(open(Model(..model, base: base)))
    DirChanged(dir) -> Some(#(Model(..model, dir: dir), effect.none()))
    WorkspaceChanged(on) ->
      Some(#(Model(..model, workspace: on), effect.none()))
    StrategyChanged(on) -> Some(#(Model(..model, strategy: on), effect.none()))
    BarChanged(on) -> Some(#(Model(..model, bar: on), effect.none()))
    HostedChanged(True) ->
      Some(#(Model(..close(model), hosted: True), effect.none()))
    HostedChanged(False) -> {
      let model = Model(..model, hosted: False)
      Some(
        #(model, case model.connected, model.source {
          True, None -> listen(model)
          _, _ -> effect.none()
        }),
      )
    }
    _ -> None
  }
}

fn live_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    Live(text) -> Some(live(model, text))
    Handed(messages) -> {
      let fresh =
        messages
        |> list.filter(fn(m) { m.0 > model.seen })
        |> list.sort(fn(a, b) { int.compare(a.0, b.0) })
      Some(
        list.fold(fresh, #(model, effect.none()), fn(acc, m) {
          let #(so_far, fx) = acc
          let #(next, more) =
            live(Model(..so_far, seen: int.max(so_far.seen, m.0)), m.1)
          #(next, effect.batch([fx, more]))
        }),
      )
    }
    // Its own stream (re)connected: anything missed meanwhile is fetched.
    CaughtUp -> Some(#(Model(..model, caught_up: True), refresh(model)))
    _ -> None
  }
}

fn opening_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    Reveal("", _) -> Some(#(model, effect.none()))
    // The host may name the task before or after it opens it: an open task
    // shows now, another once it loads.
    Reveal(tab, id) ->
      Some(case model.task {
        Some(t) if t.id == id -> #(
          Model(..model, tab: tab, revealing: ""),
          reveal(),
        )
        _ -> #(Model(..model, tab: tab, revealing: id), effect.none())
      })
    TaskLoaded(generation, text) if generation == model.generation ->
      Some(case json.parse(text, task_decoder()) {
        Ok(task) ->
          case model.staged {
            Some(#(_, w)) ->
              settle(Model(..model, staged: Some(#(Some(task), w)), error: ""))
            None -> show(model, task)
          }
        Error(_) -> #(
          Model(..unstage(model), error: error(text)),
          effect.none(),
        )
      })
    TaskLoaded(_, _) -> Some(#(model, effect.none()))
    // The whole task is taken; fields being edited are the model's, so they
    // stay as typed.
    Refetched(generation, text) ->
      Some(case model.task, json.parse(text, task_decoder()) {
        Some(t), Ok(fresh)
          if generation == model.generation && t.id == fresh.id
        -> {
          let model = case fresh.column != t.column {
            True ->
              choose_role_model(Model(..model, task: Some(fresh), error: ""))
            False -> Model(..model, task: Some(fresh), error: "")
          }
          let #(model, deps) = want_deps(model)
          #(model, effect.batch([review(model, fresh), deps]))
        }
        _, _ ->
          case generation == model.generation, connection_lost(text) {
            True, True -> #(Model(..model, error: error(text)), effect.none())
            _, _ -> #(model, effect.none())
          }
      })
    WorkflowLoaded(generation, text) if generation == model.generation ->
      Some({
        case json.parse(text, workflow_with_roles_decoder()) {
          Ok(#(w, roles)) ->
            case model.staged {
              Some(#(t, _)) ->
                settle(
                  Model(
                    ..model,
                    action_roles: roles,
                    staged: Some(#(t, Some(w))),
                  ),
                )
              None -> #(
                choose_role_model(
                  Model(..model, workflow: w, action_roles: roles),
                ),
                effect.none(),
              )
            }
          Error(_) -> #(
            Model(..unstage(model), error: error(text)),
            effect.none(),
          )
        }
      })
    WorkflowLoaded(_, _) -> Some(#(model, effect.none()))
    _ -> None
  }
}

fn model_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    TryAgain if model.refreshing -> Some(#(model, effect.none()))
    TryAgain ->
      Some(#(
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
      ))
    ModelsLoaded(text) ->
      Some(case json.parse(text, decode.list(agent_decoder())) {
        Ok(models) -> {
          let model =
            Model(..model, models: models, error: "", refreshing: False)
          let model = case model.model_source {
            Human ->
              case
                list.find(models, fn(a) { a.id == model.model && a.available })
              {
                Ok(a) -> Model(..model, effort: kept_effort(a, model.effort))
                Error(_) ->
                  choose_role_model(Model(..model, model_source: RoleDefault))
              }
            RoleDefault -> choose_role_model(model)
          }
          #(model, effect.none())
        }
        Error(_) -> #(
          Model(..model, error: error(text), refreshing: False),
          effect.none(),
        )
      })
    // A pick that can start stays chosen; its effort stays while the model
    // still offers it. One that can't start goes back to the role's model.
    ModelsRefreshed(text) ->
      Some(case json.parse(text, decode.list(agent_decoder())) {
        Ok(models) -> {
          let model = case
            list.find(models, fn(a) { a.id == model.model && a.available })
          {
            Ok(a) ->
              Model(
                ..model,
                models: models,
                effort: kept_effort(a, model.effort),
                error: "",
              )
            Error(_) ->
              choose_role_model(
                Model(
                  ..model,
                  models: models,
                  model_source: RoleDefault,
                  error: "",
                ),
              )
          }
          #(model, effect.none())
        }
        Error(_) -> #(model, effect.none())
      })
    RoleModelsLoaded(text) ->
      Some(case json.parse(text, role_models_decoder()) {
        Ok(role_models) -> #(
          choose_role_model(Model(..model, role_models: role_models)),
          effect.none(),
        )
        Error(_) -> #(model, effect.none())
      })
    // The Scope tab's projects; without them it lists none.
    ProjectsLoaded(text) ->
      Some(case json.parse(text, projects_decoder()) {
        Ok(projects) -> #(Model(..model, projects: projects), effect.none())
        Error(_) -> #(model, effect.none())
      })
    ReviewLoaded(text) ->
      Some(case json.parse(text, decode.at(["summary"], decode.string)) {
        Ok(summary) -> #(
          Model(..model, review_summary: summary, error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, review_summary: ""), effect.none())
      })
    _ -> None
  }
}

fn reply_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    // The reply to an action; it may have waited for the agent's command (D-77 point 5).
    // A lost reply doesn't mean the action failed: the task is fetched again,
    // and the run shows if the action started one.
    Acted(generation, _) if generation != model.generation ->
      Some(#(model, effect.none()))
    Acted(_, text) -> Some(acted(model, "", "", text))
    ActedClearing(generation, _, _, _) if generation != model.generation ->
      Some(#(model, effect.none()))
    ActedClearing(_, editor, sent, text) ->
      Some(acted(model, editor, sent, text))
    ChangedAt(generation, _) if generation != model.generation ->
      Some(#(model, effect.none()))
    ChangedAt(_, text) -> Some(update(model, Changed(text)))
    Noted(generation, _, _) if generation != model.generation ->
      Some(#(model, effect.none()))
    Noted(_, sent, text) ->
      Some(changed(Model(..model, noting: False), "note-text", sent, text))
    Changed(text) -> Some(changed(model, "", "", text))
    ChangedClearing(generation, _, _, _) if generation != model.generation ->
      Some(#(model, effect.none()))
    ChangedClearing(_, editor, sent, text) ->
      Some(changed(model, editor, sent, text))
    _ -> None
  }
}

fn transcript_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    Watch(rid) ->
      Some(#(Model(..model, watching: rid, entries: []), transcript(model, rid)))
    Unwatch -> Some(#(Model(..model, watching: "", entries: []), effect.none()))
    // One refresh chain per connection, including through a server restart: a
    // step from an ended chain (disconnected, or restarted) fetches and
    // schedules nothing.
    Refresh(poll, rid) if poll == model.poll && rid == model.watching ->
      Some(#(model, transcript(model, rid)))
    Refresh(_, _) -> Some(#(model, effect.none()))
    TranscriptPolled(poll, rid, text) if poll == model.poll -> {
      let #(next, _) = update(model, TranscriptLoaded(rid, text))
      let running =
        rid == next.watching
        && json.parse(text, decode.at(["state"], decode.string))
        |> result.map(fn(state) { state != "ended" })
        |> result.unwrap(False)
      Some(
        #(next, case running {
          True ->
            effect.from(fn(send) {
              after(refresh_ms, fn() { send(Refresh(poll, rid)) })
            })
          False -> effect.none()
        }),
      )
    }
    TranscriptPolled(_, _, _) -> Some(#(model, effect.none()))
    TranscriptLoaded(rid, text) if rid == model.watching -> {
      let decoder = {
        use state <- decode.field("state", decode.string)
        use entries <- decode.field("transcript", decode.list(entry_decoder()))
        decode.success(#(state, entries))
      }
      Some(case json.parse(text, decoder) {
        Ok(#(_, entries)) -> #(
          Model(..model, entries: entries, error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      })
    }
    TranscriptLoaded(_, _) -> Some(#(model, effect.none()))
    _ -> None
  }
}

fn editor_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    // Efforts differ between models, so a new model starts at its default.
    AgentSelected(id) ->
      Some(#(
        Model(..model, model: id, effort: "", model_source: Human),
        effect.none(),
      ))
    EffortSelected(effort) ->
      Some(#(Model(..model, effort: effort), effect.none()))
    Text(text) -> {
      case model.task {
        None -> Nil
        Some(t) -> draft_save("note-text", t.id, text)
      }
      Some(#(Model(..model, text: text), effect.none()))
    }
    Title(text) -> Some(#(Model(..model, title: text), effect.none()))
    Body(text) -> {
      case model.task {
        None -> Nil
        Some(t) -> draft_save("edit-body", t.id, text)
      }
      Some(#(Model(..model, body: text), effect.none()))
    }
    Rank(rank) -> Some(#(Model(..model, rank: rank), effect.none()))
    Edit(editing) ->
      Some(case model.task {
        Some(t) -> {
          let body = case editing {
            True ->
              case draft_load("edit-body", t.id) {
                "" -> t.body
                draft -> draft
              }
            False -> t.body
          }
          case editing {
            True -> Nil
            False -> draft_clear("edit-body", t.id)
          }
          #(
            Model(
              ..model,
              editing: editing,
              title: t.title,
              body: body,
              rank: float.to_string(t.rank),
            ),
            effect.none(),
          )
        }
        None -> #(model, effect.none())
      })
    Tab(tab) -> Some(#(Model(..model, tab: tab), effect.none()))
    Format(id, before, after) -> {
      let #(text, msg) = field(model, id)
      Some(#(
        model,
        effect.from(fn(send) {
          send(msg(wrap(model.root, id, text, before, after)))
        }),
      ))
    }
    Block(id, kind) -> {
      let #(text, msg) = field(model, id)
      Some(#(
        model,
        effect.from(fn(send) { send(msg(block(model.root, id, text, kind))) }),
      ))
    }
    Failed(message) -> Some(#(Model(..model, error: message), effect.none()))
    Save ->
      Some(case model.task {
        None -> #(model, effect.none())
        Some(t) ->
          case float.parse(model.rank) {
            Ok(rank) -> #(
              model,
              send(
                model,
                "/tasks/" <> t.id,
                "PATCH",
                [
                  #("title", json.string(model.title)),
                  #("body", json.string(model.body)),
                  #("rank", json.float(rank)),
                ],
                fn(generation, text) {
                  ChangedClearing(generation, "edit-body", model.body, text)
                },
              ),
            )
            Error(_) -> #(
              Model(..model, error: "Rank must be a number"),
              effect.none(),
            )
          }
      })
    Action(_) if model.acting != "" -> Some(#(model, effect.none()))
    Action(name) ->
      Some(case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let proceed = case name {
            "cancel" ->
              confirm(
                "Cancel task \""
                <> t.title
                <> "\"? This stops its agents and reverts its changes. This cannot be undone.",
              )
            "complete" ->
              confirm(
                "Complete task \""
                <> t.title
                <> "\"? This keeps its changes and closes the task. Its undo records will be removed, so it cannot be reopened or reverted through the board.",
              )
            "stop" ->
              confirm(
                "Stop the agent working on \""
                <> t.title
                <> "\"? This ends its session; its changes are kept, and a later run starts a fresh agent.",
              )
            _ -> True
          }
          case proceed {
            True -> {
              let sent = model.text
              let reply = fn(text) {
                case sent {
                  "" -> Acted(model.generation, text)
                  _ -> ActedClearing(model.generation, "note-text", sent, text)
                }
              }
              #(
                Model(..model, acting: name),
                effect.from(fn(dispatch) {
                  request(
                    model.base,
                    "/tasks/" <> t.id <> "/actions",
                    "POST",
                    action_body(model, name),
                    fn(text) { dispatch(reply(text)) },
                  )
                }),
              )
            }
            False -> #(model, effect.none())
          }
        }
      })
    WriteNote -> Some(write_note(model))
    AddNote -> Some(add_note(model))
    DeleteNote(i) -> Some(delete_note(model, i))
    Answer(qid, option) -> Some(answer_question(model, qid, option))
    _ -> None
  }
}

fn scope_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    SetScope(dir, on) ->
      Some(case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let current = scope_of(model.dir, t)
          let wanted = case on {
            True -> [dir, ..current]
            False -> list.filter(current, fn(d) { d != dir })
          }
          // In the order the projects are listed, each once.
          let ordered =
            model.projects
            |> list.map(fn(p) { p.dir })
            |> list.filter(list.contains(wanted, _))
          #(
            model,
            send(
              model,
              "/tasks/" <> t.id <> "/scope",
              "POST",
              [
                #("scope", json.array(ordered, json.string)),
              ],
              ChangedAt,
            ),
          )
        }
      })
    // Pasted into any text, the reference is a link to the task (D-98).
    CopyId ->
      Some(case model.task {
        None -> #(model, effect.none())
        Some(t) -> {
          let n = model.copy.0 + 1
          #(
            Model(..model, copy: #(n, "")),
            effect.from(fn(send) {
              copy_reference(t.id, fn(ok) { send(CopyDone(n, ok)) })
            }),
          )
        }
      })
    CopyDone(n, ok) if n == model.copy.0 ->
      Some(#(
        Model(..model, copy: #(n, copy_said(ok))),
        effect.from(fn(send) { after_copied(fn() { send(CopyReset(n)) }) }),
      ))
    CopyReset(n) if n == model.copy.0 ->
      Some(#(Model(..model, copy: #(n, "")), effect.none()))
    CopyDone(_, _) | CopyReset(_) -> Some(#(model, effect.none()))
    _ -> None
  }
}

fn dependency_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    // A dependency that can't be found keeps None: its id shows.
    // One with no project (D-129) is asked for at the root.
    DepFound(generation, id, text) if generation == model.generation ->
      Some(case dep_base(model, text) {
        Ok(base) -> #(
          model,
          get(base, "/tasks/" <> id, DepLoaded(generation, id, _)),
        )
        Error(_) -> #(model, effect.none())
      })
    DepLoaded(generation, id, text) if generation == model.generation ->
      Some(case json.parse(text, task_decoder()) {
        Ok(d) if d.id == id -> #(
          Model(
            ..model,
            deps: list.key_set(model.deps, id, Some(Dep(d.title, d.column))),
          ),
          effect.none(),
        )
        _ -> #(model, effect.none())
      })
    DepFound(_, _, _) | DepLoaded(_, _, _) -> Some(#(model, effect.none()))
    // A feature that can't be found keeps None: its id shows.
    FeatureLoaded(generation, text) if generation == model.generation -> {
      let decoder = {
        use id <- decode.field("id", decode.string)
        use title <- decode.field("title", decode.string)
        decode.success(#(id, title))
      }
      Some(case json.parse(text, decoder), model.feature {
        Ok(#(id, title)), Some(#(asked, _)) if id == asked -> #(
          Model(..model, feature: Some(#(id, Some(title)))),
          effect.none(),
        )
        _, _ -> #(model, effect.none())
      })
    }
    // A failed reply leaves the list empty, so no **Needed by** shows.
    DependentsLoaded(generation, text) if generation == model.generation -> {
      let decoder =
        decode.at(
          ["tasks"],
          decode.list({
            use id <- decode.field("id", decode.string)
            use title <- decode.field("title", decode.string)
            use column <- decode.field("column", decode.string)
            decode.success(#(id, Dep(title, column)))
          }),
        )
      Some(case json.parse(text, decoder) {
        // The reply is a snapshot taken when the task opened: a dependent a
        // live event added between the request and the reply stays until an
        // event says otherwise.
        Ok(dependents) -> {
          let live =
            model.dependents
            |> option.unwrap([])
            |> list.filter(fn(e) { !list.any(dependents, fn(d) { d.0 == e.0 }) })
          #(
            Model(..model, dependents: Some(list.append(dependents, live))),
            effect.none(),
          )
        }
        Error(_) -> #(model, effect.none())
      })
    }
    FeatureLoaded(_, _) | DependentsLoaded(_, _) ->
      Some(#(model, effect.none()))
    _ -> None
  }
}

/// A changed body or vision text as a line diff in its order: a line only in
/// the old text takes the deletion tint and one only in the new text the
/// addition tint; unchanged lines keep one of context on each side of a change
/// and collapse the rest to one muted line. No line is struck through.
fn text_diff(old: String, new: String) -> Element(msg) {
  h.div(
    [a.class("body-diff")],
    list.map(diff_lines(old, new), fn(line) {
      case line {
        OldLine(text) -> h.p([a.class("old-value")], [h.text(text)])
        NewLine(text) -> h.p([a.class("new-value")], [h.text(text)])
        SameLine(text) -> h.p([a.class("context")], [h.text(text)])
        CollapsedLines(count) ->
          h.p([a.class("unchanged")], [
            h.text(collapsed_text(count)),
          ])
      }
    }),
  )
}

/// A line of `text_diff`: only in the old text, only in the new, the same in
/// both, or a run of unchanged lines left out.
type DiffLine {
  OldLine(String)
  NewLine(String)
  SameLine(String)
  CollapsedLines(Int)
}

/// A run of the raw diff: one changed line, or consecutive unchanged ones.
type DiffRun {
  OneLine(DiffLine)
  SameRun(List(String))
}

/// The lines of `text_diff`: changed lines in the text's order, with each
/// unchanged run collapsed to a count and one line of context on each side
/// of a change.
fn diff_lines(old: String, new: String) -> List(DiffLine) {
  let old_lines = text_lines(old)
  let new_lines = text_lines(new)
  raw_diff(old_lines, new_lines, lcs_matches(old_lines, new_lines))
  |> collapse_unchanged
}

/// The pairs of indexes of a longest common subsequence of `old` and `new`.
fn lcs_matches(old: List(String), new: List(String)) -> List(#(Int, Int)) {
  case list.reverse(lcs_rows(old, new)) {
    [current, previous, ..above] ->
      lcs_backtrack(
        list.reverse(old),
        list.reverse(new),
        current,
        previous,
        above,
        list.length(old),
        list.length(new),
        list.length(new),
        [],
      )
    _ -> []
  }
}

/// The diff lines in order: lines before each shared one, the shared line,
/// then the rest, with a removal before the additions that replace it.
fn raw_diff(
  old: List(String),
  new: List(String),
  matches: List(#(Int, Int)),
) -> List(DiffLine) {
  let #(chunks, old_end, new_end) =
    list.fold(matches, #([], 0, 0), fn(state, match) {
      let #(chunks, old_from, new_from) = state
      let #(old_at, new_at) = match
      let removed =
        list.drop(old, old_from)
        |> list.take(old_at - old_from)
        |> list.map(OldLine)
      let added =
        list.drop(new, new_from)
        |> list.take(new_at - new_from)
        |> list.map(NewLine)
      let same = case list.drop(old, old_at) {
        [line, ..] -> [SameLine(line)]
        [] -> []
      }
      #([same, added, removed, ..chunks], old_at + 1, new_at + 1)
    })
  let removed = list.drop(old, old_end) |> list.map(OldLine)
  let added = list.drop(new, new_end) |> list.map(NewLine)
  list.flatten(list.append(list.reverse(chunks), [removed, added]))
}

fn collapse_unchanged(lines: List(DiffLine)) -> List(DiffLine) {
  let runs = group_runs(lines)
  let last = list.length(runs) - 1
  runs
  |> list.index_map(fn(run, at) { collapse_run(run, at == 0, at == last) })
  |> list.flatten
}

fn group_runs(lines: List(DiffLine)) -> List(DiffRun) {
  case lines {
    [] -> []
    [SameLine(line), ..rest] -> {
      let #(same, rest) = take_same(rest, [line])
      [SameRun(list.reverse(same)), ..group_runs(rest)]
    }
    [line, ..rest] -> [OneLine(line), ..group_runs(rest)]
  }
}

fn take_same(
  lines: List(DiffLine),
  acc: List(String),
) -> #(List(String), List(DiffLine)) {
  case lines {
    [SameLine(line), ..rest] -> take_same(rest, [line, ..acc])
    _ -> #(acc, lines)
  }
}

fn collapse_run(run: DiffRun, first: Bool, last: Bool) -> List(DiffLine) {
  case run {
    OneLine(line) -> [line]
    SameRun(lines) ->
      case first, last {
        True, True -> [CollapsedLines(list.length(lines))]
        True, False -> collapse_start(lines)
        False, True -> collapse_end(lines)
        False, False -> collapse_middle(lines)
      }
  }
}

fn collapsed_text(count: Int) -> String {
  case count {
    1 -> "1 line unchanged"
    _ -> int.to_string(count) <> " lines unchanged"
  }
}

/// The longest-common-subsequence lengths for every prefix pair, one row per
/// old line and one column per new line. Each row is stored from its last
/// column back, so a backtrack can walk it from the end.
fn lcs_rows(old: List(String), new: List(String)) -> List(List(Int)) {
  let columns = list.length(new) + 1
  let empty = list.repeat(0, columns)
  old
  |> list.fold([empty], fn(rows, old_line) {
    [lcs_row(old_line, new, first_row(rows)), ..rows]
  })
  |> list.reverse
}

fn lcs_row(
  old_line: String,
  new: List(String),
  previous: List(Int),
) -> List(Int) {
  let #(diagonal, up_rest) = case list.reverse(previous) {
    [first, ..rest] -> #(first, rest)
    [] -> #(0, [])
  }
  let #(reversed, _, _, _) =
    list.fold(new, #([0], diagonal, up_rest, 0), fn(state, new_line) {
      let #(reversed, diagonal, up_rest, left) = state
      let up = row_value(up_rest, 0)
      let value = case old_line == new_line {
        True -> diagonal + 1
        False ->
          case left > up {
            True -> left
            False -> up
          }
      }
      #([value, ..reversed], up, list.drop(up_rest, 1), value)
    })
  reversed
}

fn lcs_backtrack(
  old: List(String),
  new: List(String),
  current: List(Int),
  previous: List(Int),
  above: List(List(Int)),
  i: Int,
  j: Int,
  columns: Int,
  matches: List(#(Int, Int)),
) -> List(#(Int, Int)) {
  case i, j {
    _, 0 -> matches
    0, _ -> matches
    _, _ -> {
      let current_at = row_value(current, 0)
      let previous_at = row_value(previous, 0)
      let same = case old, new {
        [old_line, ..], [new_line, ..] -> old_line == new_line
        _, _ -> False
      }
      case same && current_at == row_value(previous, 1) + 1 {
        True ->
          lcs_backtrack(
            list.drop(old, 1),
            list.drop(new, 1),
            list.drop(previous, 1),
            from_column(first_row(above), columns, j - 1),
            list.drop(above, 1),
            i - 1,
            j - 1,
            columns,
            [#(i - 1, j - 1), ..matches],
          )
        False ->
          case previous_at >= row_value(current, 1) {
            True ->
              lcs_backtrack(
                list.drop(old, 1),
                new,
                previous,
                from_column(first_row(above), columns, j),
                list.drop(above, 1),
                i - 1,
                j,
                columns,
                matches,
              )
            False ->
              lcs_backtrack(
                old,
                list.drop(new, 1),
                list.drop(current, 1),
                list.drop(previous, 1),
                above,
                i,
                j - 1,
                columns,
                matches,
              )
          }
      }
    }
  }
}

fn first_row(rows: List(List(Int))) -> List(Int) {
  case rows {
    [row, ..] -> row
    [] -> []
  }
}

fn from_column(row: List(Int), columns: Int, j: Int) -> List(Int) {
  list.drop(row, columns - j)
}

fn row_value(row: List(Int), offset: Int) -> Int {
  case list.drop(row, offset) {
    [value, ..] -> value
    [] -> 0
  }
}

fn collapse_start(lines: List(String)) -> List(DiffLine) {
  case list.reverse(lines) {
    [] -> []
    [only] -> [SameLine(only)]
    [last, ..] -> [
      CollapsedLines(list.length(lines) - 1),
      SameLine(last),
    ]
  }
}

fn collapse_end(lines: List(String)) -> List(DiffLine) {
  case lines {
    [] -> []
    [only] -> [SameLine(only)]
    [first, ..] -> [
      SameLine(first),
      CollapsedLines(list.length(lines) - 1),
    ]
  }
}

fn collapse_middle(lines: List(String)) -> List(DiffLine) {
  case lines {
    [] -> []
    [only] -> [SameLine(only)]
    [first, second] -> [SameLine(first), SameLine(second)]
    [first, ..] ->
      case list.reverse(lines) {
        [last, ..] -> [
          SameLine(first),
          CollapsedLines(list.length(lines) - 2),
          SameLine(last),
        ]
        [] -> []
      }
  }
}

/// The markdown tables' look, shared by the task detail and the Strategy page:
/// cells in the page's lines and padding, and a wide table scrolls sideways
/// inside its panel.
pub const table_styles =
  ".md table{display:block;max-width:100%;overflow-x:auto;border-collapse:collapse;margin:var(--space-2) 0}.md th,.md td{border:1px solid var(--line);padding:var(--space-1) var(--space-2);vertical-align:top}.md th{background:var(--well);font-weight:600;text-align:left}\n"

/// The model a role starts with, as `GET /role-models` lists it.
pub type RoleModel {
  RoleModel(role: String, name: String, default: String)
}

/// Where the model pick came from.
pub type ModelSource {
  RoleDefault
  Human
}

/// The roles' model assignments, as `GET /role-models` lists them.
pub fn role_models_decoder() -> decode.Decoder(List(RoleModel)) {
  decode.list(role_model_decoder())
}

fn role_model_decoder() -> decode.Decoder(RoleModel) {
  use role <- decode.field("role", decode.string)
  use name <- decode.optional_field("name", role, decode.string)
  use default <- decode.optional_field("default", "", decode.string)
  decode.success(RoleModel(role, name, default))
}

fn choose_role_model(model: Model) -> Model {
  case model.model_source {
    Human -> model
    RoleDefault -> {
      let id =
        role_model_chosen(model.models, model.role_models, main_role(model))
      Model(..model, model: id, effort: "", model_source: RoleDefault)
    }
  }
}

fn main_role(model: Model) -> String {
  case model.task {
    Some(t) ->
      case list.find(model.workflow.columns, fn(c) { c.id == t.column }) {
        Ok(c) -> c.role
        Error(_) -> ""
      }
    None -> ""
  }
}

/// `effort` while `agent` offers it, else the agent's default effort.
pub fn kept_effort(agent: Agent, effort: String) -> String {
  case list.contains(agent.efforts, effort) {
    True -> effort
    False -> ""
  }
}

/// The role an action's run step gives its model pick, "" when it runs no
/// agent.
fn action_role(model: Model, name: String) -> String {
  case dict.get(model.action_roles, name) {
    Ok(role) -> role
    Error(_) -> ""
  }
}

/// The model and effort an action uses: a human's pick for every action, else
/// its own role's default.
fn action_choice(model: Model, name: String) -> #(String, String) {
  case model.model_source {
    Human -> #(model.model, model.effort)
    RoleDefault ->
      case action_role(model, name) {
        "" -> #(model.model, model.effort)
        role -> {
          let id = role_model_chosen(model.models, model.role_models, role)
          #(id, "")
        }
      }
  }
}

fn action_view(model: Model, name: String, label: String) -> Element(Msg) {
  action_button(name, label, model.acting != "")
}

/// The model a fresh role-aware list chooses: the role's model when it can
/// start, else no choice.
pub fn role_model_chosen(
  models: List(Agent),
  assignments: List(RoleModel),
  role: String,
) -> String {
  let assignment = list.find(assignments, fn(a) { a.role == role })
  case assignment {
    Ok(a) ->
      case list.find(models, fn(m) { m.id == a.default && m.available }) {
        Ok(m) -> m.id
        Error(_) -> ""
      }
    Error(_) -> ""
  }
}

fn workflow_with_roles_decoder() -> decode.Decoder(
  #(Workflow, Dict(String, String)),
) {
  use columns <- decode.field("columns", decode.list(column_decoder()))
  let label = decode.optional_field("label", "", decode.string, decode.success)
  use labels <- decode.optional_field(
    "actions",
    dict.new(),
    decode.dict(decode.string, label),
  )
  use roles <- decode.optional_field(
    "roles",
    dict.new(),
    decode.dict(decode.string, decode.string),
  )
  decode.success(#(
    Workflow(columns, dict.filter(labels, fn(_, l) { l != "" })),
    roles,
  ))
}
