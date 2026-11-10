import combobox
import features
import form_draft
import gleam/bool
import gleam/dict
import gleam/dynamic
import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import handlers
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import personas
import plan_panel
import plan_setup
import roadmap
import solace_task
import text_order

/// Registers the element on its own, for `index.html`.
pub fn main() {
  let assert Ok(_) = register()
}

pub type Tab {
  Vision
  Personas
  Features
  Roadmap
  Tasks
}

pub type Model {
  Model(
    tab: Tab,
    projects: List(Project),
    /// Whether the root has several projects, so rows name theirs.
    workspace: Bool,
    /// Each project's tasks, by folder.
    rows: List(#(String, List(Row))),
    workflows: List(#(String, solace_task.Workflow)),
    features: List(features.Feature),
    milestones: List(features.Milestone),
    filters: Filters,
    project_box: combobox.State,
    feature_box: combobox.State,
    milestone_box: combobox.State,
    /// The Tasks tab's page, from 1.
    page: Int,
    /// The Tasks tab's sort: the heading clicked and whether it is ascending,
    /// None for the default order. Kept for the page session.
    sort: Option(#(SortKey, Bool)),
    /// The task open in the bottom band: the project it is stored in, and its id.
    selected: Option(#(String, String)),
    /// Whether the selected task's band is collapsed to its bar.
    task_collapsed: Bool,
    error: String,
    source: Option(solace_task.Source),
    /// The `base` attribute: this instance's API prefix, "" for the page's.
    prefix: String,
    connected: Bool,
    caught_up: Bool,
    replay: List(String),
    /// The live messages about the open task, numbered, oldest first, handed
    /// to its `<solace-task>` so it needs no stream of its own.
    live: List(#(Int, String)),
    live_n: Int,
    /// The root's vision (D-91), as the server has it.
    vision: Document,
    /// The vision being edited: its text, and the revision the edit began
    /// from, which a save names as its `base`. None when not editing.
    draft: Option(#(String, String)),
    /// A save sent and not yet answered.
    saving: Bool,
    vision_error: String,
    /// The Personas tab's saved list and open form (D-216).
    persona_state: personas.State,
    /// The feature being written on the Features tab, None when no form is open.
    feature_draft: Option(features.Draft),
    /// Whether the feature band is collapsed to its bar.
    feature_collapsed: Bool,
    /// A feature save sent and not yet answered.
    feature_saving: Bool,
    feature_error: String,
    /// The `request` id of the open new feature's create: a repeated save of
    /// the same draft makes one feature (Spec 15).
    feature_request: String,
    /// The feature whose delete is being confirmed.
    feature_deleting: Option(String),
    /// A dropped feature shown at its new rank before the server answers:
    /// its id and the rank a refusal puts back.
    feature_moving: Option(#(String, Float)),
    /// The milestone being written on the Roadmap tab, None when no form is open.
    milestone_draft: Option(roadmap.Draft),
    /// A milestone save sent and not yet answered.
    milestone_saving: Bool,
    /// A feature target PATCH sent and not yet answered.
    milestone_targeting: Bool,
    /// The Roadmap tab's refusal: a save or delete, a feature's target or a
    /// task's dependencies.
    milestone_error: String,
    /// The `request` id of the open new milestone's create (Spec 15).
    milestone_request: String,
    /// The milestone whose delete is being confirmed.
    milestone_deleting: Option(String),
    /// The milestones shown expanded on the Roadmap, by id; kept for the page
    /// session.
    milestone_open: List(String),
    /// The words searched for a feature to add to the open milestone.
    milestone_feature_query: String,
    /// The task whose dependency editor is open: its project and id.
    depends_open: Option(#(String, String)),
    /// The words searched for a new dependency.
    depends_query: String,
    /// The title typed in each feature's new-task form, by feature id.
    task_titles: List(#(String, String)),
    /// The folder a new task goes to, "" for the root's own project.
    task_project: String,
    /// Each feature's create `request` id, kept until a create succeeds so a
    /// retry makes one task (Spec 15).
    task_requests: List(#(String, String)),
    /// The feature whose new task is being created and linked.
    task_saving: Option(String),
    /// The link sent after a create: the task's project, its id and the feature.
    task_linking: Option(#(String, String, String)),
    /// Each feature's new-task error, by feature id.
    task_errors: List(#(String, String)),
    /// The Tasks tab's new task, None when its form is closed. Its create and
    /// link use `task_saving` and `task_linking` with "" for the feature.
    new_task: Option(NewTask),
    /// The `request` id of the Tasks tab's create, chosen when the form opens
    /// and kept until a create succeeds (Spec 15).
    new_task_request: String,
    new_task_error: String,
    /// The root's strategy runs (D-93), oldest first.
    runs: List(plan_panel.Run),
    /// The run shown in the planning panel, "" for none.
    plan_open: String,
    plan_collapsed: Bool,
    /// The planning panel's width in pixels, kept in the browser.
    plan_width: Int,
    /// The panel's message box: a new run's first message, or the next one.
    plan_text: String,
    /// The message a successful start or send clears, None when none. It is
    /// what the request was sent with.
    plan_clearing: Option(String),
    /// A start, message, answer, end or stop sent and not yet answered.
    plan_sending: Bool,
    plan_error: String,
    /// The shown run's transcript.
    plan_entries: List(solace_task.Entry),
    /// The transcript polling chain; an ended chain schedules nothing.
    plan_poll: Int,
    /// Whether the shown run's end is in reach in the panel, so a new message
    /// scrolls to it; else "New message ↓" shows (D-118).
    plan_at_bottom: Bool,
    plan_new: Bool,
    /// A run was opened: once it loads, the panel shows its newest message.
    plan_jump: Bool,
    /// The models a run may use, from `GET /models`, and the one picked.
    models: List(solace_task.Agent),
    /// The default model assigned to each role.
    role_models: List(solace_task.RoleModel),
    plan_model: String,
    plan_effort: String,
    /// Where the plan's current model pick came from.
    plan_source: solace_task.ModelSource,
    /// A model refresh sent and not yet answered.
    plan_refreshing: Bool,
    /// The shadow root, once it has rendered, for the editor's tools.
    root: dynamic.Dynamic,
    /// The task a task link asked to show (D-98), and how many times one was
    /// asked, so asking again is news to `<solace-task>`.
    reveal: #(String, Int),
    /// The feature the Features tab shows in full, None for none.
    focused_feature: Option(String),
  )
}

pub type Msg {
  Select(Tab)
  /// The shell shows this tab: a load, Back or Forward.
  TabShown(Tab)
  ProjectsLoaded(String)
  WorkflowLoaded(String, String)
  Live(String)
  CaughtUp
  /// A row: its task opens below the list.
  Open(String, String)
  /// A link (D-98, D-122): the shell asks to show a record, scrolled into
  /// view: its kind ("task", "feature" or "milestone"), the project `dir` a
  /// task is stored in, and its id. A task shows on the Tasks tab, a feature
  /// on Features, a milestone on Roadmap.
  Focus(String, String, String)
  /// A feature card's More or Less: shows the feature in full, or compacts it.
  FocusFeature(String)
  /// `<solace-task>` says the open task now lives in another project.
  TaskMoved(String, String)
  /// Collapse the selected task's band to its bar, or expand it back.
  ToggleTask
  Filter(Filters)
  /// Text typed in a Tasks filter's combobox.
  FilterText(FilterField, String)
  /// A click in a Tasks filter's combobox.
  FilterOpen(FilterField)
  /// A key in a Tasks filter's combobox.
  FilterKey(FilterField, String)
  /// A Tasks filter suggestion: the filter and the record's id.
  FilterChoose(FilterField, String)
  /// A Tasks filter's input lost focus.
  FilterBlur(FilterField)
  /// The Tasks tab's Prev or Next: the page to show.
  Page(Int)
  /// A Tasks tab heading: sorts by it ascending, or flips its direction.
  SortBy(SortKey)
  /// An inline link: the task's project and id, the field to set, and the
  /// record's id, "" for none.
  Link(String, String, String, String)
  /// Links as one PATCH: the task's project and id, and each field with its
  /// record's id, "" for none.
  Links(String, String, List(#(String, String)))
  /// The reply to a link: the task's project, and the task as it is now.
  Patched(String, String, String)
  /// A row's Plan (D-132): the task's project and id.
  PlanTask(String, String)
  /// The reply to a row's Plan: the task's project, and the task or a refusal.
  Planned(String, String)
  BaseChanged(String)
  Listening(solace_task.Source)
  Connected
  Disconnected
  /// Begins editing the vision as it is now.
  EditVision
  CancelVision
  Draft(String)
  VisionFormat(String, String)
  VisionBlock(String)
  SaveVision
  /// The reply to a save: the vision as it is now, or a refusal.
  VisionSaved(String)
  /// Drops the draft and edits the vision as it is now.
  ReloadVision
  /// Opens the form for a new persona.
  NewPersona
  /// Opens the form on the persona with this id.
  EditPersona(String)
  CancelPersona
  PersonaName(String)
  PersonaText(personas.Field, String)
  PersonaFormat(personas.Field, String, String)
  PersonaBlock(personas.Field, String)
  SavePersona
  /// The reply to a persona save: the persona as it is now, or a refusal.
  PersonaSaved(String)
  /// Drops the draft and edits the saved persona as it is now.
  ReloadPersona
  /// Asks to confirm deleting the persona with this id.
  DeletePersona(String)
  CancelDeletePersona
  ConfirmDeletePersona
  /// The reply to a delete: the deleted persona, or a refusal.
  PersonaDeleted(String)
  /// Opens the form for a new feature.
  NewFeature
  /// Opens the form on the feature with this id.
  EditFeature(String)
  CancelFeature
  /// Collapse the feature band to its bar, or expand it back.
  ToggleFeature
  FeatureTitle(String)
  FeatureText(String)
  FeatureState(String)
  /// The milestone's id, "" for none.
  FeatureMilestone(String)
  FeatureFormat(String, String)
  FeatureBlock(String)
  SaveFeature
  /// The reply to a feature save: the feature as it is now, or a refusal.
  FeatureSaved(String)
  /// Moves the feature with this id to this rank: one PATCH.
  MoveFeature(String, Float)
  /// A press on a feature card's grip at this clientY: the drag begins.
  GrabFeature(String, Int)
  /// The drag ended: the feature's id and the index it lands at in its
  /// group, -1 when cancelled.
  DropFeature(String, Int)
  /// Starts the feature with this id: one PATCH to `active` (D-154).
  StartFeature(String)
  /// The reply to a move: the feature, or a refusal.
  FeatureMoved(String)
  /// The reply to a start: the feature, or a refusal.
  FeatureStarted(String)
  /// The reply to a delete: the deleted feature, or a refusal.
  FeatureDeleted(String)
  /// Asks to confirm deleting the feature with this id.
  DeleteFeature(String)
  CancelDeleteFeature
  ConfirmDeleteFeature
  /// A feature's new-task title: the feature's id and the text.
  TaskTitle(String, String)
  /// The folder new tasks go to.
  TaskProject(String)
  /// Creates a task with the feature's typed title, then links it.
  AddTask(String)
  /// The reply to a create: the task's project, the feature, and the task or
  /// a refusal.
  TaskCreated(String, String, String)
  /// Opens the Tasks tab's new-task form.
  NewTaskOpen
  NewTaskCancel
  NewTaskTitle(String)
  NewTaskText(String)
  /// The folder the new task goes to.
  NewTaskProject(String)
  /// The feature's id, "" for none.
  NewTaskFeature(String)
  /// The milestone's id, "" for none.
  NewTaskMilestone(String)
  NewTaskFormat(String, String)
  NewTaskBlock(String)
  /// Creates the Tasks tab's new task, then links what was picked.
  CreateTask
  /// Opens the form for a new milestone.
  NewMilestone
  /// Opens the form on the milestone with this id.
  EditMilestone(String)
  /// A milestone heading: shows its body, features and tasks, or hides them.
  ToggleMilestone(String)
  CancelMilestone
  MilestoneTitle(String)
  MilestoneText(String)
  /// The target date as the input gives it, "" for none.
  MilestoneTargetDate(String)
  /// The words searched for a feature to add to the open milestone.
  MilestoneFeatureQuery(String)
  MilestoneState(String)
  MilestoneFormat(String, String)
  MilestoneBlock(String)
  SaveMilestone
  /// The reply to a milestone save: the milestone as it is now, or a refusal.
  MilestoneSaved(String)
  /// Asks to confirm deleting the milestone with this id.
  DeleteMilestone(String)
  CancelDeleteMilestone
  ConfirmDeleteMilestone
  MilestoneDeleted(String)
  /// A feature's id and the milestone it now targets, "" for none.
  TargetFeature(String, String)
  /// The reply to a target change: the feature, or a refusal.
  FeatureTargeted(String)
  /// A task's project, its id and every id it now depends on.
  SetDepends(String, String, List(String))
  /// The reply to a dependency change: the task's project, and the task or a
  /// refusal (E7024, E7025, or E7003 during a run).
  DependsPatched(String, String)
  /// Opens a task's dependency editor (its project and id), or closes it.
  OpenDepends(String, String)
  DependsQuery(String)
  Attached(dynamic.Dynamic)
  /// Collapses the planning panel, or opens it.
  PlanCollapse
  /// A press on the panel's resize handle at this clientX: a drag begins.
  PlanDrag(Int)
  /// The panel's width during a drag.
  PlanWidth(Int)
  /// The drag ended: the width is stored.
  PlanWidthDone
  /// A key on the focused resize handle: the arrows, Home or End.
  PlanKey(String)
  PlanText(String)
  PlanFormat(String, String)
  PlanBlock(String)
  PlanModel(String)
  PlanEffort(String)
  /// The reply to `GET /models`.
  ModelsLoaded(String)
  /// The server's default model for each role.
  RoleModelsLoaded(String)
  /// The models fetched again on catch-up or a live run or task update: a
  /// failed reply changes nothing, and a pick that can start stays.
  ModelsRefreshed(String)
  /// Try again: recheck every model and clear the holds.
  TryAgainModel
  /// Starts a strategy run on the panel's context with the typed message.
  StartPlan
  /// Sends the typed message to the shown run.
  SendPlan
  /// Answers a proposal of the shown run: the question's id and the option.
  AnswerPlan(String, String)
  EndPlan
  StopPlan
  /// The reply to a start, message, answer, end or stop: whether it empties
  /// the message box, and the run or a refusal.
  PlanReplied(Bool, String)
  /// Shows the run with this id in the panel, "" for none.
  ShowPlan(String)
  /// The shown run with its transcript: the polling chain, the run's id and
  /// the reply.
  PlanLoaded(Int, String, String)
  /// The next step of a polling chain: the chain and the run's id.
  PlanRefresh(Int, String)
  /// The panel was scrolled: whether the shown run's end is in reach.
  PlanScrolled(Bool)
  /// "New message ↓": scrolls to the shown run's newest message.
  PlanNewest
}

@external(javascript, "./api.mjs", "tab_query")
fn tab_query() -> String

pub fn tabs() -> List(Tab) {
  [Vision, Personas, Features, Roadmap, Tasks]
}

/// The tab's name in the URL: `/strategy?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    Vision -> "vision"
    Personas -> "personas"
    Features -> "features"
    Roadmap -> "roadmap"
    Tasks -> "tasks"
  }
}

pub fn label(tab: Tab) -> String {
  case tab {
    Vision -> "Vision"
    Personas -> "Personas"
    Features -> "Features"
    Roadmap -> "Roadmap"
    Tasks -> "Tasks"
  }
}

/// The tab a URL query names ("?tab=tasks"); Vision when it names none or an unknown one.
pub fn tab_from(query: String) -> Tab {
  let named =
    query
    |> string.replace("?", "")
    |> string.split("&")
    |> list.find_map(fn(pair) {
      string.remove_prefix(pair, "tab=") |> ok_if_changed(pair)
    })
  case named {
    Ok(name) ->
      case list.find(tabs(), fn(tab) { tab_name(tab) == name }) {
        Ok(tab) -> tab
        Error(_) -> Vision
      }
    Error(_) -> Vision
  }
}

fn ok_if_changed(rest: String, pair: String) -> Result(String, Nil) {
  case rest == pair {
    True -> Error(Nil)
    False -> Ok(rest)
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  // The transport starts when the instance connects, once `base` is known.
  let model = Model(..new(tab_from(tab_query())), plan_width: stored_width())
  #(restore_new_task(model), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  [
    fn() { navigation_events(model, msg) },
    fn() { connection_events(model, msg) },
    fn() { vision_events(model, msg) },
    fn() { persona_format_events(model, msg) },
    fn() { persona_events(model, msg) },
    fn() { feature_events(model, msg) },
    fn() { milestone_events(model, msg) },
    fn() { task_events(model, msg) },
    fn() { plan_events(model, msg) },
    fn() { plan_reply_events(model, msg) },
  ]
  |> handlers.first(#(model, effect.none()))
}

pub fn view(model: Model) -> Element(Msg) {
  html.div([attribute.class("strategy")], [
    html.style([], view_styles),
    html.div(
      [
        attribute.class("tabs"),
        attribute.role("tablist"),
        attribute.attribute("aria-label", "Strategy"),
      ],
      list.map(tabs(), fn(tab) {
        let current = tab == model.tab
        html.button(
          [
            attribute.id("tab-" <> tab_name(tab)),
            attribute.role("tab"),
            attribute.attribute("aria-selected", case current {
              True -> "true"
              False -> "false"
            }),
            attribute.attribute("aria-controls", "panel-" <> tab_name(tab)),
            event.on_click(Select(tab)),
          ],
          [html.text(label(tab))],
        )
      }),
    ),
    plan_view(model),
    ..list.map(tabs(), fn(tab) {
      html.section(
        [
          attribute.id("panel-" <> tab_name(tab)),
          attribute.role("tabpanel"),
          attribute.attribute("aria-labelledby", "tab-" <> tab_name(tab)),
          attribute.hidden(tab != model.tab),
        ],
        case tab {
          Tasks -> tasks_view(model)
          Vision -> vision_view(model)
          Personas -> personas_view(model)
          Features -> features_view(model)
          Roadmap -> roadmap_view(model)
        },
      )
    })
  ])
}

/// The full light color set: `<solace-task>` sets none of its own, so its
/// colors come from here. The dark set is the page's (`index.html`, and the
/// shell's for `/strategy`).
const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> solace_task.tool_styles
  <> shell_styles
  <> filter_styles
  <> tasks_styles
  <> feature_styles
  <> state_styles
  <> roadmap_styles
  <> detail_styles
  <> vision_styles
  <> persona_styles
  <> solace_task.table_styles
  <> narrow_styles
  <> solace_task.editor_styles
  <> plan_panel.styles

pub fn register() -> Result(Nil, lustre.Error) {
  // The open task's details are `<solace-task>`, registered once per page.
  let _ = solace_task.register()
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("base", fn(base) { Ok(BaseChanged(base)) }),
      component.on_attribute_change("tab", fn(name) {
        Ok(TabShown(tab_from("?tab=" <> name)))
      }),
      // The shell's `focus` property, `{kind, dir, id, n}`: `n` makes asking
      // again news. Without a kind it is a task.
      component.on_property_change("focus", {
        use kind <- decode.optional_field("kind", "task", decode.string)
        use dir <- decode.field("dir", decode.string)
        use id <- decode.field("id", decode.string)
        decode.success(Focus(kind, dir, id))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-strategy",
  )
}

/// A project of the root: its folder, its name on screen and its API prefix,
/// "" for the root's own project and `/p/<dir>` for the others (D-57).
pub type Project {
  Project(dir: String, label: String, base: String)
}

/// A task as the Tasks tab lists it: what `/events` carries that the list
/// shows, filters and searches, and the fields its band's status line needs
/// (D-87). Notes and changes are not part of it.
pub type Row {
  Row(
    id: String,
    title: String,
    body: String,
    column: String,
    feature: Option(String),
    milestone: Option(String),
    /// When the task was created, in seconds (D-130); None from an older server.
    created: Option(Int),
    /// When the task first reached a final column, in seconds; None while open.
    closed: Option(Int),
    /// The paused state, the last outcome, and each run's kind and state, for
    /// the band's meta line.
    paused: String,
    finished: String,
    runs: List(#(String, String)),
    /// The ids of the tasks this one depends on (D-87).
    depends: List(String),
    /// Whether the board shows the task (D-196).
    on_board: Bool,
  )
}

/// What the list keeps and how its search ranks. "" in a field keeps every
/// task; "-" for feature or milestone keeps the tasks without one. Every word
/// of the query is in the task's title or body, ignoring case; a word equal
/// to a task's id, bare or as `task:<id>`, finds that task. A query with text
/// ranks the id or a whole title phrase first, then every word in the title,
/// then the rest.
pub type Filters {
  Filters(
    column: String,
    project: String,
    feature: String,
    milestone: String,
    query: String,
  )
}

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

/// A page on `tab` with nothing loaded yet.
pub fn new(tab: Tab) -> Model {
  Model(
    tab: tab,
    projects: [],
    workspace: False,
    rows: [],
    workflows: [],
    features: [],
    milestones: [],
    filters: Filters("", "", "", "", ""),
    project_box: combobox.blank(),
    feature_box: combobox.blank(),
    milestone_box: combobox.blank(),
    page: 1,
    sort: None,
    selected: None,
    task_collapsed: False,
    error: "",
    source: None,
    prefix: "",
    connected: False,
    caught_up: False,
    replay: [],
    live: [],
    live_n: 0,
    vision: Document("", ""),
    draft: None,
    saving: False,
    vision_error: "",
    persona_state: personas.blank(),
    feature_draft: None,
    feature_collapsed: False,
    feature_saving: False,
    feature_error: "",
    feature_request: "",
    feature_deleting: None,
    milestone_draft: None,
    milestone_saving: False,
    milestone_targeting: False,
    milestone_error: "",
    milestone_request: "",
    milestone_deleting: None,
    milestone_open: [],
    milestone_feature_query: "",
    depends_open: None,
    depends_query: "",
    task_titles: [],
    task_project: "",
    task_requests: [],
    task_saving: None,
    task_linking: None,
    task_errors: [],
    new_task: None,
    new_task_request: "",
    new_task_error: "",
    runs: [],
    plan_open: "",
    plan_collapsed: False,
    plan_width: plan_panel.default_width,
    plan_text: plan_panel.load_draft(plan_setup.settings()),
    plan_clearing: None,
    plan_sending: False,
    plan_error: "",
    plan_entries: [],
    plan_poll: 0,
    plan_at_bottom: True,
    plan_new: False,
    plan_jump: False,
    models: [],
    role_models: [],
    plan_model: "",
    plan_effort: "",
    plan_source: solace_task.RoleDefault,
    plan_refreshing: False,
    root: no_root(),
    reveal: #("", 0),
    focused_feature: None,
    feature_moving: None,
  )
}

fn get(model: Model, path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(model.prefix, path, "GET", "", fn(text) { send(msg(text)) })
  })
}

/// Opens the event stream and asks for the project list. The root's stream
/// replays every project's `{workflow}` and `{tasks}`, then the features and
/// milestones (Spec 15): the root's own `GET /tasks` holds only its project.
fn start(model: Model) -> #(Model, Effect(Msg)) {
  let model = Model(..stop(model), caught_up: False, replay: [])
  let models = case model.models {
    [] -> get(model, "/models", ModelsLoaded)
    _ -> get(model, "/models", ModelsRefreshed)
  }
  #(
    model,
    effect.batch([
      get(model, "/projects", ProjectsLoaded),
      models,
      get(model, "/role-models", RoleModelsLoaded),
      effect.from(fn(send) {
        send(
          Listening(
            solace_task.events(
              model.prefix,
              fn(text) { send(Live(text)) },
              fn() { send(CaughtUp) },
            ),
          ),
        )
      }),
    ]),
  )
}

fn stop(model: Model) -> Model {
  case model.source {
    Some(source) -> solace_task.close_events(source)
    None -> Nil
  }
  Model(..model, source: None)
}

fn error(text: String) -> String {
  case json.parse(text, decode.at(["message"], decode.string)) {
    Ok(message) -> message
    Error(_) -> text
  }
}

/// The body of an inline link's `PATCH /tasks/{id}`: the one field, null to
/// clear it.
pub fn link_body(field: String, value: String) -> json.Json {
  links_body([#(field, value)])
}

/// Applies one `/events` message; a workspace's name their `project`.
pub fn live(model: Model, text: String) -> Model {
  use <- result.lazy_unwrap(records(model, text))
  let dir = event_dir(model, text)
  case at(text, "task", row_decoder()) {
    Ok(row) -> put(model, dir, row)
    Error(_) ->
      case at(text, "tasks", decode.list(row_decoder())) {
        Ok(rows) -> Model(..model, rows: set(model.rows, dir, rows))
        Error(_) ->
          case at(text, "workflow", solace_task.workflow_decoder()) {
            Ok(w) -> Model(..model, workflows: set(model.workflows, dir, w))
            Error(_) ->
              case at(text, "removed", decode.string) {
                Ok(id) ->
                  Model(
                    ..model,
                    rows: set(
                      model.rows,
                      dir,
                      list.filter(rows_of(model, dir), fn(r) { r.id != id }),
                    ),
                  )
                  |> follow_move(dir, id)
                Error(_) -> model
              }
          }
      }
  }
}

/// Applies a message about the root's features, milestones, vision, personas,
/// role models and strategy runs, which names no project (D-87); an error for
/// any other message.
fn records(model: Model, text: String) -> Result(Model, Nil) {
  let without = fn(records: List(a), id: String, id_of: fn(a) -> String) {
    list.filter(records, fn(r) { id_of(r) != id })
  }
  at(text, "milestones", decode.list(features.milestone_decoder()))
  |> result.map(fn(rs) { Model(..model, milestones: rs) })
  |> result.lazy_or(fn() {
    at(text, "features", decode.list(features.decoder()))
    |> result.map(fn(rs) { Model(..model, features: rs) })
  })
  |> result.lazy_or(fn() {
    at(text, "milestone", features.milestone_decoder())
    |> result.map(fn(r) {
      Model(..model, milestones: keep(model.milestones, r, fn(m) { m.id }))
    })
  })
  |> result.lazy_or(fn() {
    at(text, "feature", features.decoder())
    |> result.map(fn(r) {
      Model(..model, features: keep(model.features, r, fn(f) { f.id }))
    })
  })
  |> result.lazy_or(fn() {
    at(text, "removedMilestone", decode.string)
    |> result.map(fn(id) {
      Model(..model, milestones: without(model.milestones, id, fn(m) { m.id }))
    })
  })
  |> result.lazy_or(fn() {
    at(text, "removedFeature", decode.string)
    |> result.map(fn(id) {
      Model(..model, features: without(model.features, id, fn(f) { f.id }))
    })
  })
  |> result.lazy_or(fn() {
    at(text, "vision", vision_decoder())
    |> result.map(fn(v) { Model(..model, vision: v) })
  })
  |> result.lazy_or(fn() {
    at(text, "personas", decode.list(personas.decoder()))
    |> result.map(fn(rs) {
      Model(
        ..model,
        persona_state: personas.State(..model.persona_state, personas: rs),
      )
    })
  })
  |> result.lazy_or(fn() {
    at(text, "roleModels", solace_task.role_models_decoder())
    |> result.map(fn(rs) { choose_plan_model(Model(..model, role_models: rs)) })
  })
  // The list holds no notes: a run's stay until the run comes in full.
  |> result.lazy_or(fn() {
    at(text, "strategyRuns", decode.list(plan_panel.decoder()))
    |> result.map(fn(rs) {
      Model(
        ..model,
        runs: list.map(rs, fn(r) {
          case list.find(model.runs, fn(old) { old.id == r.id }) {
            Ok(old) -> plan_panel.Run(..r, notes: old.notes)
            Error(_) -> r
          }
        }),
      )
    })
  })
  |> result.lazy_or(fn() {
    at(text, "strategyRun", plan_panel.decoder())
    |> result.map(fn(r) {
      Model(..model, runs: keep(model.runs, r, fn(r) { r.id }))
    })
  })
}

/// The message's `key`, decoded.
fn at(text: String, key: String, decoder: decode.Decoder(a)) -> Result(a, Nil) {
  json.parse(text, decode.at([key], decoder)) |> result.replace_error(Nil)
}

/// The record replaced where it is, else added at the end.
fn keep(records: List(a), record: a, id_of: fn(a) -> String) -> List(a) {
  case list.any(records, fn(r) { id_of(r) == id_of(record) }) {
    True ->
      list.map(records, fn(r) {
        case id_of(r) == id_of(record) {
          True -> record
          False -> r
        }
      })
    False -> list.append(records, [record])
  }
}

fn set(pairs: List(#(String, a)), key: String, value: a) -> List(#(String, a)) {
  [#(key, value), ..list.filter(pairs, fn(p) { p.0 != key })]
}

fn rows_of(model: Model, dir: String) -> List(Row) {
  list.key_find(model.rows, dir) |> result.unwrap([])
}

fn put(model: Model, dir: String, row: Row) -> Model {
  let others = list.filter(rows_of(model, dir), fn(r) { r.id != row.id })
  Model(..model, rows: set(model.rows, dir, [row, ..others]))
}

fn project_of(model: Model, dir: String) -> Result(Project, Nil) {
  list.find(model.projects, fn(p) { p.dir == dir })
}

fn base_of(model: Model, dir: String) -> String {
  project_of(model, dir) |> result.map(fn(p) { p.base }) |> result.unwrap("")
}

fn label_of(model: Model, dir: String) -> String {
  project_of(model, dir) |> result.map(fn(p) { p.label }) |> result.unwrap(dir)
}

/// The project a live message is for: the homeless bucket when it marks a
/// task with no project (D-129), else the one it names, else the root's.
fn event_dir(model: Model, text: String) -> String {
  case homeless_event(text) {
    True -> homeless_dir
    False ->
      case at(text, "project", decode.string) {
        Ok(dir) -> dir
        Error(_) ->
          list.find(model.projects, fn(p) { p.base == "" })
          |> result.map(fn(p) { p.dir })
          |> result.unwrap("")
      }
  }
}

/// Whether a live update says the workspace's project list changed.
fn projects_event(text: String) -> Bool {
  at(text, "projects", decode.list(decode.string)) |> result.is_ok
}

/// Whether a live message can be applied now: its project is listed, it
/// marks a task with no project (D-129), or it names none (the project
/// list, features, milestones and role models).
fn knows(model: Model, text: String) -> Bool {
  projects_event(text)
  || result.is_ok(records(model, text))
  || homeless_event(text)
  || result.is_ok(project_of(model, event_dir(model, text)))
}

/// Applies the held messages whose project is listed; the others wait.
fn settle(model: Model) -> Model {
  case model.caught_up {
    False -> model
    True -> {
      let #(known, unknown) =
        list.partition(list.reverse(model.replay), fn(t) { knows(model, t) })
      list.fold(
        known,
        Model(..model, replay: list.reverse(unknown)),
        fn(m, text) { hand(live(m, text), text) },
      )
    }
  }
}

/// Whether a live message is for the open task's details: its own update
/// under any project (`<solace-task>` fetches itself again when one names
/// another than its home, D-61), the update of a task it depends on, for its
/// Depends on line, its project's workflow, a change to the list of projects,
/// which its Scope tab shows, or a role model change, which its picker
/// follows.
fn for_detail(model: Model, text: String) -> Bool {
  case model.selected {
    None -> False
    Some(#(home, id)) ->
      case at(text, "task", decode.at(["id"], decode.string)) {
        Ok(task) -> task == id || list.contains(depends_of(model, id), task)
        Error(_) ->
          solace_task.role_models_event(text)
          || solace_task.projects_changed(text)
          || event_dir(model, text) == home
          && result.is_ok(at(text, "workflow", decode.dynamic))
      }
  }
}

/// Adds a live message to those handed to the open task's details, if it is
/// for them; only the last few are kept.
pub fn hand(model: Model, text: String) -> Model {
  case for_detail(model, text) {
    False -> model
    True -> push(model, text)
  }
}

/// The open task left its project: if another project holds it now (D-61),
/// it is opened from there.
fn follow_move(model: Model, dir: String, id: String) -> Model {
  case model.selected {
    Some(#(home, shown)) if home == dir && shown == id ->
      case
        list.find(model.projects, fn(p) {
          p.dir != dir && list.any(rows_of(model, p.dir), fn(r) { r.id == id })
        })
      {
        Ok(p) -> Model(..model, selected: Some(#(p.dir, id)))
        Error(_) -> model
      }
    _ -> model
  }
}

/// A task in `GET /tasks` or `/events`. A task saved before D-87 has no
/// feature, milestone, dependsOn or closedTime; one before D-130 no createdTime.
pub fn row_decoder() -> decode.Decoder(Row) {
  use id <- decode.field("id", decode.string)
  use title <- decode.optional_field("title", "", decode.string)
  use body <- decode.optional_field("body", "", decode.string)
  use column <- decode.field("column", decode.string)
  use feature <- decode.optional_field(
    "feature",
    None,
    decode.optional(decode.string),
  )
  use milestone <- decode.optional_field(
    "milestone",
    None,
    decode.optional(decode.string),
  )
  use created <- decode.optional_field(
    "createdTime",
    None,
    decode.optional(decode.int),
  )
  use closed <- decode.optional_field(
    "closedTime",
    None,
    decode.optional(decode.int),
  )
  use paused <- decode.optional_field(
    "paused_from",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use finished <- decode.optional_field(
    "finished",
    "",
    decode.one_of(decode.at(["outcome"], decode.string), [decode.success("")]),
  )
  use runs <- decode.optional_field(
    "runs",
    [],
    decode.list({
      use kind <- decode.field("kind", decode.string)
      use state <- decode.field("state", decode.string)
      decode.success(#(kind, state))
    }),
  )
  use depends <- decode.optional_field(
    "dependsOn",
    [],
    decode.list(decode.string),
  )
  use on_board <- decode.optional_field("onBoard", True, decode.bool)
  decode.success(Row(
    id,
    title,
    body,
    column,
    feature,
    milestone,
    created,
    closed,
    paused,
    finished,
    runs,
    depends,
    on_board,
  ))
}

fn projects_decoder() -> decode.Decoder(#(Bool, List(Project))) {
  let project = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use base <- decode.field("base", decode.string)
    decode.success(Project(dir, label, base))
  }
  use workspace <- decode.optional_field("workspace", False, decode.bool)
  use projects <- decode.field("projects", decode.list(project))
  decode.success(#(workspace, projects))
}

/// The tasks the list shows, each with its project: those the filters keep,
/// in the heading's sort, else the search's ranking, else the default order.
/// Tasks with no project (D-129) show too, in the homeless bucket.
pub fn visible(model: Model) -> List(#(String, Row)) {
  task_dirs(model)
  |> list.flat_map(fn(dir) {
    list.map(rows_of(model, dir), fn(r) { #(dir, r) })
  })
  |> list.filter(fn(pair) { keeps(model.filters, pair.0, pair.1) })
  |> sort_rows(model, _)
}

/// Closed tasks newest first, then open tasks by title.
pub fn sorted(rows: List(#(String, Row))) -> List(#(String, Row)) {
  list.sort(rows, fn(a, b) {
    case { a.1 }.closed, { b.1 }.closed {
      Some(x), Some(y) -> int.compare(y, x)
      Some(_), None -> order.Lt
      None, Some(_) -> order.Gt
      None, None -> text_order.compare({ a.1 }.title, { b.1 }.title)
    }
  })
}

/// Whether the filters keep a task stored in `dir`.
pub fn keeps(filters: Filters, dir: String, row: Row) -> Bool {
  { filters.column == "" || filters.column == row.column }
  && { filters.project == "" || filters.project == dir }
  && linked(filters.feature, row.feature)
  && linked(filters.milestone, row.milestone)
  && searched(filters.query, row)
}

fn linked(wanted: String, link: Option(String)) -> Bool {
  case wanted, link {
    "", _ -> True
    "-", None -> True
    "-", Some(_) -> False
    _, Some(id) -> id == wanted
    _, None -> False
  }
}

/// Every word of the query is in the title or the body, ignoring case, or is
/// the task's id, bare or as `task:<id>`. Notes and changes are not in
/// `GET /tasks`, so they are not searched.
fn searched(query: String, row: Row) -> Bool {
  let text = string.lowercase(row.title <> "\n" <> row.body)
  query
  |> query_words
  |> list.all(fn(word) {
    string.contains(text, word) || word == row.id || word == "task:" <> row.id
  })
}

/// Every project's columns, each id once, in workflow order.
fn columns(model: Model) -> List(solace_task.Column) {
  model.projects
  |> list.flat_map(fn(p) {
    case list.key_find(model.workflows, p.dir) {
      Ok(w) -> w.columns
      Error(_) -> []
    }
  })
  |> list.unique
  |> list.fold([], fn(seen: List(solace_task.Column), c) {
    case list.any(seen, fn(s) { s.id == c.id }) {
      True -> seen
      False -> list.append(seen, [c])
    }
  })
}

fn column_label(model: Model, dir: String, column: String) -> String {
  // A homeless row's (D-129) column label comes from the root's workflow.
  let dir = case dir == homeless_dir {
    True -> root_dir(model)
    False -> dir
  }
  list.key_find(model.workflows, dir)
  |> result.try(fn(w) { list.find(w.columns, fn(c) { c.id == column }) })
  |> result.map(fn(c) { c.label })
  |> result.unwrap(column)
}

/// The Tasks tab: the filters, one page of rows, the page controls with the
/// count, and the open task's band below them.
fn tasks_view(model: Model) -> List(Element(Msg)) {
  let rows = visible(model)
  let total = list.length(rows)
  let page = shown_page(model.page, total)
  [
    html.div([attribute.class("tasks-tools")], [
      new_task_view(model),
      filters_view(model),
      case model.error {
        "" -> element.none()
        text -> html.p([attribute.role("alert")], [html.text(text)])
      },
    ]),
    html.div([attribute.class("tasks-scroll")], [
      case rows {
        [] -> html.p([], [html.text("No tasks match.")])
        _ ->
          html.table([attribute.class("tasks")], [
            html.thead([], [
              html.tr(
                [],
                list.map(headings(model), fn(h) { heading_view(model, h) }),
              ),
            ]),
            html.tbody(
              [],
              list.map(page_rows(rows, page), fn(pair) {
                row_view(model, pair.0, pair.1)
              }),
            ),
          ])
      },
    ]),
    html.div([attribute.class("pager")], [
      html.button(
        [
          attribute.disabled(page == 1),
          event.on_click(Page(page - 1)),
        ],
        [html.text("← Prev")],
      ),
      html.span(
        [
          attribute.class("count"),
          attribute.attribute("aria-live", "polite"),
        ],
        [html.text(page_text(page, total))],
      ),
      html.button(
        [
          attribute.disabled(page == pages(total)),
          event.on_click(Page(page + 1)),
        ],
        [html.text("Next →")],
      ),
    ]),
    task_band(model),
  ]
}

/// The Tasks tab's headings, each with what it sorts by.
fn headings(model: Model) -> List(#(String, SortKey)) {
  case model.workspace {
    True -> [
      #("Title", ByTitle),
      #("Column", ByColumn),
      #("Project", ByProject),
      #("Feature", ByFeature),
      #("Milestone", ByMilestone),
      #("Created", ByCreated),
      #("Closed", ByClosed),
    ]
    False -> [
      #("Title", ByTitle),
      #("Column", ByColumn),
      #("Feature", ByFeature),
      #("Milestone", ByMilestone),
      #("Created", ByCreated),
      #("Closed", ByClosed),
    ]
  }
}

fn filters_view(model: Model) -> Element(Msg) {
  let f = model.filters
  html.div([attribute.class("filters")], [
    html.label([], [
      html.text("Search"),
      html.input([
        attribute.type_("search"),
        attribute.value(f.query),
        attribute.placeholder("Title, description, or task:<id>"),
        event.on_input(fn(v) { Filter(Filters(..f, query: v)) }),
      ]),
    ]),
    choice(
      "Column",
      f.column,
      list.map(columns(model), fn(c) { #(c.id, c.label) }),
      fn(v) { Filter(Filters(..f, column: v)) },
    ),
    case model.workspace {
      True -> filter_box(model, ProjectField)
      False -> element.none()
    },
    filter_box(model, FeatureField),
    filter_box(model, MilestoneField),
  ])
}

/// A filter's select: "All", then each option as (value, label).
fn choice(
  name: String,
  value: String,
  options: List(#(String, String)),
  msg: fn(String) -> Msg,
) -> Element(Msg) {
  html.label([], [
    html.text(name),
    html.select([event.on_change(msg)], [
      html.option([attribute.value(""), attribute.selected(value == "")], "All"),
      ..list.map(options, fn(o) {
        html.option(
          [attribute.value(o.0), attribute.selected(o.0 == value)],
          o.1,
        )
      })
    ]),
  ])
}

fn row_view(model: Model, dir: String, row: Row) -> Element(Msg) {
  let current = model.selected == Some(#(dir, row.id))
  html.tr(
    [
      case current {
        True -> attribute.attribute("aria-current", "true")
        False -> attribute.none()
      },
    ],
    [
      html.td([], [
        html.button(
          [attribute.class("open"), event.on_click(Open(dir, row.id))],
          [
            html.text(row.title),
          ],
        ),
        case list.contains(plan_panel.open_ids(model.runs, "task"), row.id) {
          True -> plan_panel.marker(plan_setup.settings().labels)
          False -> element.none()
        },
        // A single project's table has no Project column: the row says it here.
        case !model.workspace && dir == homeless_dir {
          True ->
            html.span([attribute.class("homeless")], [html.text("No project")])
          False -> element.none()
        },
        plan_button(model, dir, row),
      ]),
      html.td([], [html.text(column_label(model, dir, row.column))]),
      case model.workspace {
        True ->
          html.td([], [
            html.text(case dir == homeless_dir {
              True -> "No project"
              False -> label_of(model, dir)
            }),
          ])
        False -> element.none()
      },
      html.td([], [
        link_select(
          feature_pairs(model),
          row.feature,
          "Feature of " <> row.title,
          fn(v) { Link(dir, row.id, "feature", v) },
        ),
      ]),
      html.td([], [
        milestone_cell(model, row),
      ]),
      html.td([attribute.class("created")], case row.created {
        Some(seconds) -> clock_cell(seconds)
        None -> []
      }),
      html.td([attribute.class("closed")], case row.closed {
        Some(seconds) -> clock_cell(seconds)
        None -> []
      }),
    ],
  )
}

/// An inline link: "(none)" and every record.
/// A link to a record not loaded yet shows its id.
fn link_select(
  records: List(#(String, String)),
  link: Option(String),
  name: String,
  msg: fn(String) -> Msg,
) -> Element(Msg) {
  let chosen = option.unwrap(link, "")
  let records = case
    chosen == "" || list.any(records, fn(r) { r.0 == chosen })
  {
    True -> records
    False -> list.append(records, [#(chosen, chosen)])
  }
  html.select(
    [
      attribute.attribute("aria-label", name),
      event.on_change(msg),
    ],
    [
      html.option(
        [attribute.value(""), attribute.selected(chosen == "")],
        "(none)",
      ),
      ..list.map(records, fn(r) {
        html.option(
          [attribute.value(r.0), attribute.selected(r.0 == chosen)],
          r.1,
        )
      })
    ],
  )
}

/// The API prefix the open task's project answers at: a workspace project's
/// own `/p/<dir>`, else this page's.
fn task_base(model: Model, dir: String) -> String {
  let base = base_of(model, dir)
  case string.starts_with(base, "/p/") {
    True -> base
    False -> model.prefix <> base
  }
}

/// The open task's details: `<solace-task>` from the components project,
/// which loads and changes the task itself. This page hands it the task's
/// events (`live`) and follows it when it moves to another of its projects.
fn task_view(model: Model, dir: String, id: String) -> Element(Msg) {
  let #(shown, asked) = model.reveal
  element.element(
    "solace-task",
    [
      attribute.attribute("base", task_base(model, dir)),
      attribute.attribute("dir", dir),
      attribute.attribute("task", id),
      attribute.id("task-details"),
      attribute.attribute("hosted", ""),
      attribute.attribute("bar", ""),
      // Back to backlog is offered from Planning only here (D-132).
      attribute.attribute("strategy", ""),
      case model.workspace {
        True -> attribute.attribute("workspace", "")
        False -> attribute.none()
      },
      attribute.property(
        "live",
        json.array(model.live, fn(m) {
          json.object([#("n", json.int(m.0)), #("text", json.string(m.1))])
        }),
      ),
      case shown {
        "" -> attribute.none()
        _ ->
          attribute.property(
            "reveal",
            json.object([
              #("tab", json.string("details")),
              #("id", json.string(shown)),
              #("n", json.int(asked)),
            ]),
          )
      },
      event.on("task-moved", {
        use dir <- decode.subfield(["detail", "dir"], decode.string)
        use id <- decode.subfield(["detail", "id"], decode.string)
        decode.success(TaskMoved(dir, id))
      }),
    ],
    [],
  )
}

/// The root's vision document and its revision (D-91).
pub type Document {
  Document(text: String, revision: String)
}

fn vision_decoder() -> decode.Decoder(Document) {
  use text <- decode.field("text", decode.string)
  use revision <- decode.field("revision", decode.string)
  decode.success(Document(text, revision))
}

@external(javascript, "./api.mjs", "no_root")
fn no_root() -> dynamic.Dynamic

/// The vision editor's textarea.
const vision_id = "vision-text"

/// A save of `text` over the revision `base` (D-91).
pub fn vision_body(text: String, base: String) -> json.Json {
  json.object([#("text", json.string(text)), #("base", json.string(base))])
}

/// The Vision tab: the document as Markdown with an Edit button, or the
/// editor while editing. A draft begun from an older revision says so, since
/// the server refuses its save.
fn vision_view(model: Model) -> List(Element(Msg)) {
  let button = fn(label, msg, disabled) {
    html.button(
      [
        attribute.type_("button"),
        attribute.disabled(disabled),
        event.on_click(msg),
      ],
      [html.text(label)],
    )
  }
  let alert = case model.vision_error {
    "" -> []
    message -> [html.p([attribute.role("alert")], [html.text(message)])]
  }
  case model.draft {
    None -> [
      case model.vision.text {
        "" ->
          html.p([], [
            html.text(
              "No vision yet. Write what the product is for and who uses it; agents read it in their briefs.",
            ),
          ])
        text ->
          html.div([attribute.class("vision")], [
            solace_task.md(model.prefix, text),
          ])
      },
      html.div([attribute.class("vision-actions")], [
        button(solace_task.ui_label("edit"), EditVision, False),
      ]),
    ]
    Some(#(text, base)) -> {
      // Past the limit the count is an alert and Save waits (D-92).
      let over = vision_count(text) > vision_limit
      [
        solace_task.editor(
          solace_task.EditorConfig(
            id: vision_id,
            task: "",
            base: model.prefix,
            label: "Vision",
            placeholder: "",
            text: text,
            on_input: Draft,
            on_format: VisionFormat,
            on_block: VisionBlock,
            actions: [],
          ),
        ),
        ..list.flatten([
          [
            html.p(
              [
                attribute.class("vision-count"),
                case over {
                  True -> attribute.role("alert")
                  False -> attribute.attribute("aria-live", "polite")
                },
              ],
              [html.text(count_text(text))],
            ),
          ],
          case base == model.vision.revision {
            True -> []
            False -> [
              html.p([attribute.role("alert")], [
                html.text(
                  "The vision changed since you began editing, so this draft can't be saved over it. Reload takes the saved vision; your draft is lost.",
                ),
              ]),
            ]
          },
          alert,
          [
            html.div([attribute.class("vision-actions")], [
              button(
                solace_task.ui_label("save"),
                SaveVision,
                model.saving || over,
              ),
              button(solace_task.ui_label("cancel"), CancelVision, model.saving),
              ..case base == model.vision.revision {
                True -> []
                False -> [button("Reload", ReloadVision, model.saving)]
              }
            ]),
          ],
        ])
      ]
    }
  }
}

/// The most characters the vision may hold (D-92); the server refuses more.
pub const vision_limit = 1000

/// The vision's size as the server counts it: Unicode code points, as Rust's
/// `chars().count()`, not graphemes.
pub fn vision_count(text: String) -> Int {
  string.to_utf_codepoints(text) |> list.length
}

/// The line below the editor: "973 / 1000 character limit".
pub fn count_text(text: String) -> String {
  int.to_string(vision_count(text))
  <> " / "
  <> int.to_string(vision_limit)
  <> " character limit"
}

fn feature_pairs(model: Model) -> List(#(String, String)) {
  list.map(model.features, fn(f) { #(f.id, f.title) })
}

fn milestone_pairs(model: Model) -> List(#(String, String)) {
  list.map(model.milestones, fn(m) { #(m.id, m.title) })
}

/// The Features tab: the milestones' features, each with its tasks from
/// every project and the homeless bucket (D-129).
fn features_view(model: Model) -> List(Element(Msg)) {
  let tasks =
    list.flat_map(task_dirs(model), fn(dir) {
      list.map(rows_of(model, dir), fn(r) {
        #(dir, r.id, r.title, r.feature, r.closed != None, r.column)
      })
    })
  let editor = feature_editor(model)
  [
    html.div(
      [attribute.class("features-scroll")],
      features.view(
        model.prefix,
        model.milestones,
        model.features,
        tasks,
        Open,
        editor,
        features.TaskForm(
          titles: model.task_titles,
          errors: model.task_errors,
          projects: case model.workspace {
            True -> list.map(model.projects, fn(p) { #(p.dir, p.label) })
            False -> []
          },
          project: task_dir(model),
          saving: model.task_saving,
          on_title: TaskTitle,
          on_project: TaskProject,
          on_add: AddTask,
        ),
        plan_panel.open_ids(model.runs, "feature"),
      ),
    ),
    feature_band(model, editor),
  ]
}

/// Changes the open feature draft; nothing when no form is open.
fn edit_feature(
  model: Model,
  change: fn(features.Draft) -> features.Draft,
) -> #(Model, Effect(Msg)) {
  case model.feature_draft {
    Some(d) -> #(Model(..model, feature_draft: Some(change(d))), effect.none())
    None -> #(model, effect.none())
  }
}

@external(javascript, "./api.mjs", "request_id")
fn request_id() -> String

/// The pairs without `key`.
fn drop(pairs: List(#(String, a)), key: String) -> List(#(String, a)) {
  list.filter(pairs, fn(pair) { pair.0 != key })
}

/// The folder a new task goes to: the one picked, else the root's own project.
fn task_dir(model: Model) -> String {
  case model.task_project {
    "" -> root_dir(model)
    dir -> dir
  }
}

/// The Roadmap tab: the milestones with their features, and every project's
/// tasks with their dependencies, homeless ones (D-129) included.
fn roadmap_view(model: Model) -> List(Element(Msg)) {
  let tasks = roadmap_tasks(model)
  roadmap.view(
    model.prefix,
    model.milestones,
    model.features,
    tasks,
    Open,
    roadmap.Editor(
      draft: model.milestone_draft,
      saving: model.milestone_saving,
      targeting: model.milestone_targeting,
      error: model.milestone_error,
      expanded: model.milestone_open,
      on_toggle: ToggleMilestone,
      on_new: NewMilestone,
      on_edit: EditMilestone,
      on_title: MilestoneTitle,
      on_text: MilestoneText,
      on_target_date: MilestoneTargetDate,
      on_state: MilestoneState,
      on_format: MilestoneFormat,
      on_block: MilestoneBlock,
      on_save: SaveMilestone,
      on_cancel: CancelMilestone,
      // A milestone removed while confirming no longer holds the list.
      deleting: case model.milestone_deleting {
        Some(id) ->
          case list.any(model.milestones, fn(m) { m.id == id }) {
            True -> Some(id)
            False -> None
          }
        None -> None
      },
      on_delete: DeleteMilestone,
      on_confirm_delete: ConfirmDeleteMilestone,
      on_cancel_delete: CancelDeleteMilestone,
      on_target: TargetFeature,
      feature_query: model.milestone_feature_query,
      on_feature_query: MilestoneFeatureQuery,
      on_depends: SetDepends,
      depends_open: model.depends_open,
      depends_query: model.depends_query,
      on_depends_open: OpenDepends,
      on_depends_query: DependsQuery,
    ),
    plan_panel.open_ids(model.runs, "milestone"),
  )
}

/// Changes the open milestone draft; nothing when no form is open.
fn edit_milestone(
  model: Model,
  change: fn(roadmap.Draft) -> roadmap.Draft,
) -> #(Model, Effect(Msg)) {
  case model.milestone_draft {
    Some(d) -> #(
      Model(..model, milestone_draft: Some(change(d))),
      effect.none(),
    )
    None -> #(model, effect.none())
  }
}

@external(javascript, "./api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

/// While the shown run's agent works, its transcript is fetched again this often.
const refresh_ms = 1500

/// The run shown in the planning panel.
fn plan_shown(model: Model) -> Option(plan_panel.Run) {
  list.find(model.runs, fn(r) { r.id == model.plan_open })
  |> option.from_result
}

/// The title of the task, feature or milestone `id`, "" when not loaded.
fn record_name(model: Model, kind: String, id: String) -> String {
  case kind {
    "task" ->
      list.flat_map(model.rows, fn(p) { p.1 })
      |> list.find(fn(r) { r.id == id })
      |> result.map(fn(r) { r.title })
    "feature" ->
      list.find(model.features, fn(f) { f.id == id })
      |> result.map(fn(f) { f.title })
    "milestone" ->
      list.find(model.milestones, fn(m) { m.id == id })
      |> result.map(fn(m) { m.title })
    _ -> Error(Nil)
  }
  |> result.unwrap("")
}

/// One run request at a time; its reply is `PlanReplied`.
fn plan_send(
  model: Model,
  path: String,
  body: json.Json,
  clears: Bool,
) -> #(Model, Effect(Msg)) {
  case model.plan_sending {
    True -> #(model, effect.none())
    False -> #(
      Model(
        ..model,
        plan_sending: True,
        plan_error: "",
        plan_clearing: case clears {
          True -> Some(model.plan_text)
          False -> None
        },
      ),
      effect.from(fn(send) {
        request(model.prefix, path, "POST", json.to_string(body), fn(reply) {
          send(PlanReplied(clears, reply))
        })
      }),
    )
  }
}

/// Asks for run `rid` with its transcript in a new polling chain; the one
/// before ends.
fn plan_fetch(model: Model, rid: String) -> #(Model, Effect(Msg)) {
  let poll = model.plan_poll + 1
  #(Model(..model, plan_poll: poll), plan_get(model, poll, rid))
}

fn plan_get(model: Model, poll: Int, rid: String) -> Effect(Msg) {
  get(model, plan_panel.run_path(plan_setup.settings(), rid), PlanLoaded(
    poll,
    rid,
    _,
  ))
}

/// A live change of the shown run's state fetches its transcript again,
/// which polls while the agent works.
fn plan_follow(old: Model, new: Model) -> #(Model, Effect(Msg)) {
  case plan_shown(old), plan_shown(new) {
    Some(a), Some(b) if a.state != b.state -> plan_fetch(new, b.id)
    _, _ -> #(new, effect.none())
  }
}

/// The planning panel, docked beside every tab (D-93 point 7).
fn plan_view(model: Model) -> Element(Msg) {
  plan_panel.view(
    model.prefix,
    plan_setup.settings(),
    plan_panel.Panel(
      runs: model.runs,
      context: plan_panel.page,
      shown: plan_shown(model),
      entries: model.plan_entries,
      collapsed: model.plan_collapsed,
      width: model.plan_width,
      max_width: plan_panel.max_width(window_width()),
      text: model.plan_text,
      sending: model.plan_sending,
      error: model.plan_error,
      refreshing: model.plan_refreshing,
      models: model.models,
      model: model.plan_model,
      effort: model.plan_effort,
      name: fn(kind, id) { record_name(model, kind, id) },
      on_collapse: PlanCollapse,
      on_drag: PlanDrag,
      on_key: PlanKey,
      on_text: PlanText,
      on_format: PlanFormat,
      on_block: PlanBlock,
      on_model: PlanModel,
      on_effort: PlanEffort,
      on_try_again: TryAgainModel,
      on_start: StartPlan,
      on_send: SendPlan,
      on_answer: AnswerPlan,
      on_show: ShowPlan,
      on_end: EndPlan,
      on_stop: StopPlan,
      at_bottom: model.plan_at_bottom,
      new_message: model.plan_new,
      on_scroll: PlanScrolled,
      on_newest: PlanNewest,
    ),
  )
}

fn push(model: Model, text: String) -> Model {
  let n = model.live_n + 1
  Model(
    ..model,
    live_n: n,
    live: list.append(model.live, [#(n, text)])
      |> list.reverse
      |> list.take(50)
      |> list.reverse,
  )
}

/// Once the stream has (re)connected and caught up, the open task's details
/// fetch the task again: an event lost meanwhile can't leave them stale.
fn resync(model: Model) -> Model {
  case model.selected {
    None -> model
    Some(_) -> push(model, solace_task.resync)
  }
}

/// Where the browser keeps the planning panel's width.
const width_key = "solace-strategy-plan-width"

/// The planning panel's stored width, else the default.
fn stored_width() -> Int {
  load(width_key)
  |> int.parse
  |> result.unwrap(plan_panel.default_width)
  |> plan_panel.clamp(window_width())
}

fn store_width(width: Int) -> Effect(Msg) {
  effect.from(fn(_) { store(width_key, int.to_string(width)) })
}

@external(javascript, "./api.mjs", "load")
fn load(key: String) -> String

@external(javascript, "./api.mjs", "store")
fn store(key: String, value: String) -> Nil

/// The window's width in pixels, 0 with no window.
@external(javascript, "./api.mjs", "window_width")
fn window_width() -> Int

@external(javascript, "./api.mjs", "drag")
fn drag(x: Int, width: Int, move: fn(Int) -> Nil, done: fn() -> Nil) -> Nil

/// Whether a live change of the strategy runs needs the human, which flags the
/// Strategy tab while another page shows (the host hears `attention`): a run
/// went waiting, so its agent replied, or a run asked a new proposal.
pub fn needs_attention(old: List(plan_panel.Run), new: List(plan_panel.Run)) -> Bool {
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

/// The ids of the tasks task `id` depends on, as the rows hold it.
fn depends_of(model: Model, id: String) -> List(String) {
  list.find_map(model.rows, fn(p) { list.find(p.1, fn(r) { r.id == id }) })
  |> result.map(fn(r) { r.depends })
  |> result.unwrap([])
}

/// A new message or proposal in the shown run (D-118) scrolls to it when the
/// human was at the run's end; else "New message ↓" shows, with no jump.
/// Transcript entries, drawn above the conversation, keep the human at the
/// end when they were there, with no button.
fn plan_track(
  old: Model,
  step: #(Model, Effect(Msg)),
) -> #(Model, Effect(Msg)) {
  let #(new, fx) = step
  let grew = list.length(new.plan_entries) > list.length(old.plan_entries)
  case plan_shown(old), plan_shown(new) {
    Some(a), Some(b) if a.id == b.id && !new.plan_jump ->
      case plan_panel.size(b) > plan_panel.size(a), new.plan_at_bottom {
        True, True -> #(new, effect.batch([fx, plan_scroll()]))
        True, False -> #(Model(..new, plan_new: True), fx)
        False, True if grew -> #(new, effect.batch([fx, plan_scroll()]))
        False, _ -> step
      }
    _, _ -> step
  }
}

/// Scrolls the planning panel to the shown run's end, once it has rendered.
fn plan_scroll() -> Effect(Msg) {
  effect.after_paint(fn(_, root) { scroll_plan(root) })
}

@external(javascript, "./api.mjs", "scroll_plan")
fn scroll_plan(root: dynamic.Dynamic) -> Nil

/// A feature or milestone link (D-122): its tab opens, and its card scrolls
/// into view once drawn; the stream may not have brought it yet.
fn show_record(
  model: Model,
  tab: Tab,
  kind: String,
  id: String,
) -> #(Model, Effect(Msg)) {
  #(
    Model(..model, tab: tab),
    effect.after_paint(fn(_, root) { reveal_card(root, kind, id) }),
  )
}

@external(javascript, "./api.mjs", "reveal_card")
fn reveal_card(root: dynamic.Dynamic, kind: String, id: String) -> Nil

/// The bucket tasks with no project (D-129) live in. No project folder is
/// named this, so it can't collide with one.
const homeless_dir = "no-project"

/// The root project's folder, as the board names it.
fn root_dir(model: Model) -> String {
  list.find(model.projects, fn(p) { p.base == "" })
  |> result.map(fn(p) { p.dir })
  |> result.unwrap(".")
}

/// Every bucket the task list reads: each project's, and the homeless one.
fn task_dirs(model: Model) -> List(String) {
  list.append(list.map(model.projects, fn(p) { p.dir }), [homeless_dir])
}

/// The bucket a named task's row lives in: the homeless one when its id is
/// listed there (D-129), else the folder it was named with.
fn bucket_of(model: Model, dir: String, id: String) -> String {
  case list.any(rows_of(model, homeless_dir), fn(r) { r.id == id }) {
    True -> homeless_dir
    False -> dir
  }
}

/// Whether a live message marks a task with no project (D-129).
fn homeless_event(text: String) -> Bool {
  case at(text, "noProject", decode.bool) {
    Ok(True) -> True
    _ -> False
  }
}

/// The body of a link's `PATCH /tasks/{id}`: each field, null to clear it.
pub fn links_body(fields: List(#(String, String))) -> json.Json {
  json.object(
    list.map(fields, fn(f) {
      #(f.0, case f.1 {
        "" -> json.null()
        id -> json.string(id)
      })
    }),
  )
}

/// The Tasks tab's new task: its title, description, the folder it goes to,
/// and the feature and milestone it is linked to, "" for none.
pub type NewTask {
  NewTask(
    title: String,
    body: String,
    project: String,
    feature: String,
    milestone: String,
  )
}

/// The new task's description editor.
const new_task_id = "new-task-text"

/// The links a new task's PATCH sends: its feature, whose milestone it takes,
/// or else the milestone picked.
pub fn new_task_links(task: NewTask) -> List(#(String, String)) {
  case task.feature, task.milestone {
    "", "" -> []
    "", milestone -> [#("milestone", milestone)]
    feature, _ -> [#("feature", feature)]
  }
}

/// Changes the open new task; nothing when its form is closed.
fn edit_new_task(
  model: Model,
  change: fn(NewTask) -> NewTask,
) -> #(Model, Effect(Msg)) {
  case model.new_task {
    Some(d) -> #(Model(..model, new_task: Some(change(d))), effect.none())
    None -> #(model, effect.none())
  }
}

/// The Tasks tab's new task is made: the form closes and the task opens in
/// the bottom band, as a row click does.
fn new_task_done(model: Model, dir: String, id: String) -> Model {
  case model.new_task {
    Some(_) -> solace_task.draft_clear(new_task_id, "new")
    None -> Nil
  }
  Model(
    ..model,
    new_task: None,
    new_task_request: "",
    new_task_error: "",
    task_saving: None,
    task_linking: None,
    selected: Some(#(dir, id)),
    task_collapsed: False,
    live: [],
  )
}

/// The Tasks tab's New task button, or its form: a title, the description,
/// the project in a workspace, the feature and milestone, Create and Cancel.
fn new_task_view(model: Model) -> Element(Msg) {
  case model.new_task {
    None ->
      html.div([attribute.class("vision-actions")], [
        html.button([attribute.type_("button"), event.on_click(NewTaskOpen)], [
          html.text(solace_task.ui_label("new_task")),
        ]),
      ])
    Some(d) -> {
      let select = fn(name, value, options: List(#(String, String)), msg) {
        html.label([], [
          html.text(name),
          html.select(
            [attribute.name(string.lowercase(name)), event.on_change(msg)],
            list.map(options, fn(o) {
              html.option(
                [attribute.value(o.0), attribute.selected(o.0 == value)],
                o.1,
              )
            }),
          ),
        ])
      }
      html.div([attribute.class("feature-form new-task-form")], [
        html.h3([], [html.text(solace_task.ui_label("new_task"))]),
        html.div([attribute.class("filters")], [
          html.label([], [
            html.text("Title"),
            html.input([
              attribute.type_("text"),
              attribute.value(d.title),
              event.on_input(NewTaskTitle),
            ]),
          ]),
        ]),
        solace_task.editor(
          solace_task.EditorConfig(
            id: new_task_id,
            task: "",
            base: model.prefix,
            label: "Description",
            placeholder: "",
            text: d.body,
            on_input: NewTaskText,
            on_format: NewTaskFormat,
            on_block: NewTaskBlock,
            actions: [],
          ),
        ),
        html.div([attribute.class("filters")], [
          case model.workspace && list.length(model.projects) > 1 {
            True ->
              select(
                "Project",
                d.project,
                list.map(model.projects, fn(p) { #(p.dir, p.label) }),
                NewTaskProject,
              )
            False -> element.none()
          },
          select(
            "Feature",
            d.feature,
            [#("", "(none)"), ..feature_pairs(model)],
            NewTaskFeature,
          ),
          case d.feature {
            "" ->
              select(
                "Milestone",
                d.milestone,
                [#("", "(none)"), ..milestone_pairs(model)],
                NewTaskMilestone,
              )
            _ -> element.none()
          },
        ]),
        case model.new_task_error {
          "" -> element.none()
          message -> html.p([attribute.role("alert")], [html.text(message)])
        },
        html.div([attribute.class("vision-actions")], [
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(
                model.task_saving != None || string.trim(d.title) == "",
              ),
              event.on_click(CreateTask),
            ],
            [html.text(solace_task.ui_label("create_task"))],
          ),
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(model.task_saving == Some("")),
              event.on_click(NewTaskCancel),
            ],
            [html.text(solace_task.ui_label("cancel"))],
          ),
        ]),
      ])
    }
  }
}

/// Tasks per page on the Tasks tab.
pub const page_size = 10

/// How many pages `total` tasks make, at least one.
pub fn pages(total: Int) -> Int {
  int.max(1, { total + page_size - 1 } / page_size)
}

/// The page to show, kept within the pages: live events can shrink the list
/// under the page being shown, and the view never shows an empty page past
/// the end.
pub fn shown_page(page: Int, total: Int) -> Int {
  page
  |> int.max(1)
  |> int.min(pages(total))
}

/// The rows the page `page` shows, of the filtered and sorted rows.
pub fn page_rows(
  rows: List(#(String, Row)),
  page: Int,
) -> List(#(String, Row)) {
  let p = shown_page(page, list.length(rows))
  rows
  |> list.drop({ p - 1 } * page_size)
  |> list.take(page_size)
}

/// The Tasks tab's count line: "page 2 of 4, 96 tasks".
pub fn page_text(page: Int, total: Int) -> String {
  let p = shown_page(page, total)
  "page "
  <> int.to_string(p)
  <> " of "
  <> int.to_string(pages(total))
  <> ", "
  <> int.to_string(total)
  <> case total == 1 {
    True -> " task"
    False -> " tasks"
  }
}

/// The page holding the task `dir`/`id` in the filtered rows, so a task link
/// (D-98) shows its row beside its details; `fallback` when the filters hide it.
fn page_of(
  rows: List(#(String, Row)),
  dir: String,
  id: String,
  fallback: Int,
) -> Int {
  // The row's place in the list, found once: `#(found, place)`.
  let #(found, _) =
    list.fold(rows, #(-1, 0), fn(acc, pair) {
      case acc.0, pair.0 == dir && pair.1.id == id {
        -1, True -> #(acc.1, 0)
        _, _ -> #(acc.0, acc.1 + 1)
      }
    })
  case found {
    -1 -> fallback
    i -> i / page_size + 1
  }
}

/// A Tasks tab column the table can be sorted on.
pub type SortKey {
  ByTitle
  ByColumn
  ByProject
  ByFeature
  ByMilestone
  ByCreated
  ByClosed
}

/// What a row is sorted by: text, or a number (a column's place, a time).
type SortValue {
  Text(String)
  Number(Int)
}

/// The rows in the Tasks tab's order: by the heading clicked, else the
/// search's ranking, else the default. Missing values go last either way;
/// ties fall back to the title, then the id, then the project.
pub fn sort_rows(
  model: Model,
  rows: List(#(String, Row)),
) -> List(#(String, Row)) {
  case model.sort {
    None -> by_search(model.filters.query, rows)
    Some(#(key, ascending)) -> {
      let ranks = list.map(columns(model), fn(c) { c.id })
      list.sort(rows, fn(a, b) {
        case
          sort_value(model, ranks, key, a),
          sort_value(model, ranks, key, b)
        {
          Some(x), Some(y) ->
            case ascending {
              True -> compare_values(x, y)
              False -> order.negate(compare_values(x, y))
            }
          Some(_), None -> order.Lt
          None, Some(_) -> order.Gt
          None, None -> order.Eq
        }
        |> order.lazy_break_tie(fn() {
          text_order.compare({ a.1 }.title, { b.1 }.title)
        })
        |> order.lazy_break_tie(fn() { string.compare({ a.1 }.id, { b.1 }.id) })
        |> order.lazy_break_tie(fn() { string.compare(a.0, b.0) })
      })
    }
  }
}

/// A row's value for `key`, None when it has none. `ranks` are the column
/// ids in workflow order.
fn sort_value(
  model: Model,
  ranks: List(String),
  key: SortKey,
  pair: #(String, Row),
) -> Option(SortValue) {
  let #(dir, row) = pair
  let title = fn(pairs, id) { Text(record_title(pairs, id)) }
  case key {
    ByTitle -> Some(Text(row.title))
    ByColumn -> rank(ranks, row.column, 0) |> option.map(Number)
    ByProject ->
      case dir == homeless_dir {
        True -> None
        False -> Some(Text(label_of(model, dir)))
      }
    ByFeature -> option.map(row.feature, title(feature_pairs(model), _))
    ByMilestone -> option.map(row.milestone, title(milestone_pairs(model), _))
    ByCreated -> option.map(row.created, Number)
    ByClosed -> option.map(row.closed, Number)
  }
}

/// The place of `id` in `ids`, counting from `at`; None when absent.
fn rank(ids: List(String), id: String, at: Int) -> Option(Int) {
  case ids {
    [] -> None
    [first, ..] if first == id -> Some(at)
    [_, ..rest] -> rank(rest, id, at + 1)
  }
}

fn compare_values(a: SortValue, b: SortValue) -> order.Order {
  case a, b {
    Text(x), Text(y) -> text_order.compare(x, y)
    Number(x), Number(y) -> int.compare(x, y)
    Text(_), Number(_) -> order.Lt
    Number(_), Text(_) -> order.Gt
  }
}

/// A Tasks tab heading: a button that sorts by it. The sorted one shows its
/// direction, as an arrow and in `aria-sort`.
fn heading_view(model: Model, heading: #(String, SortKey)) -> Element(Msg) {
  let #(label, key) = heading
  let #(sort, arrow) = case model.sort {
    Some(#(k, True)) if k == key -> #("ascending", " ▲")
    Some(#(k, False)) if k == key -> #("descending", " ▼")
    _ -> #("", "")
  }
  html.th(
    [
      attribute.attribute("scope", "col"),
      case sort {
        "" -> attribute.none()
        s -> attribute.attribute("aria-sort", s)
      },
    ],
    [
      html.button([attribute.class("sort"), event.on_click(SortBy(key))], [
        html.text(label),
        case arrow {
          "" -> element.none()
          a ->
            html.span([attribute.attribute("aria-hidden", "true")], [
              html.text(a),
            ])
        },
      ]),
    ],
  )
}

/// A row's Plan (D-132): shown when its project's workflow offers `plan` in
/// the row's column. A task with no project (D-129) has no workflow, so none.
fn plan_button(model: Model, dir: String, row: Row) -> Element(Msg) {
  case list.key_find(model.workflows, dir) {
    Ok(w) -> {
      let offered =
        list.any(w.columns, fn(c) {
          c.id == row.column && list.contains(c.actions, "plan")
        })
      use <- bool.guard(!offered, element.none())
      let label = solace_task.action_label(w, "plan")
      html.button(
        [
          attribute.class("act"),
          attribute.attribute("aria-label", label <> " " <> row.title),
          event.on_click(PlanTask(dir, row.id)),
        ],
        [html.text(label)],
      )
    }
    Error(_) -> element.none()
  }
}

@external(javascript, "./api.mjs", "drag_card")
fn drag_card(
  root: dynamic.Dynamic,
  id: String,
  y: Int,
  done: fn(Int) -> Nil,
) -> Nil

/// Whether a live update is one the planner shows: a strategy run or a task.
/// These also refresh the model list, so a config edit reaches the pick.
pub fn models_event(text: String) -> Bool {
  ["strategyRun", "strategyRuns", "task", "tasks", "removed"]
  |> list.any(fn(key) { result.is_ok(at(text, key, decode.dynamic)) })
}

/// Shows the Roadmap's milestone `id` expanded.
fn expand_milestone(model: Model, id: String) -> Model {
  case list.contains(model.milestone_open, id) {
    True -> model
    False -> Model(..model, milestone_open: [id, ..model.milestone_open])
  }
}

/// Shows the feature's milestone expanded, when it is loaded.
fn expand_feature_milestone(model: Model, id: String) -> Model {
  let target = case list.find(model.features, fn(f) { f.id == id }) {
    Ok(feature) -> feature.milestone
    Error(_) -> None
  }
  case target {
    Some(mid) ->
      case list.any(model.milestones, fn(m) { m.id == mid }) {
        True -> expand_milestone(model, mid)
        False -> model
      }
    None -> model
  }
}

/// Opens or closes the Roadmap's milestone `id`.
fn toggle_milestone(model: Model, id: String) -> Model {
  case list.contains(model.milestone_open, id) {
    True -> collapse_milestone(model, id)
    False -> expand_milestone(model, id)
  }
}

/// Every project's tasks as the Roadmap shows them.
fn roadmap_tasks(model: Model) -> List(roadmap.Task) {
  list.flat_map(task_dirs(model), fn(dir) {
    list.map(rows_of(model, dir), fn(r) {
      roadmap.Task(
        dir,
        r.id,
        r.title,
        r.feature,
        r.milestone,
        r.depends,
        r.closed != None,
      )
    })
  })
}

/// Hides the Roadmap's milestone `id`.
fn collapse_milestone(model: Model, id: String) -> Model {
  Model(
    ..model,
    milestone_open: list.filter(model.milestone_open, fn(open) { open != id }),
  )
}

/// A Tasks filter with its own typed text, open list and active suggestion.
pub type FilterField {
  ProjectField
  FeatureField
  MilestoneField
}

fn filter_box(model: Model, field: FilterField) -> Element(Msg) {
  let f = filter(field)
  combobox.view(
    f.name,
    f.key,
    f.box(model),
    f.chosen(model),
    f.items(model),
    combobox.Handlers(
      on_text: fn(text) { FilterText(field, text) },
      on_open: FilterOpen(field),
      on_key: fn(pressed) { FilterKey(field, pressed) },
      on_choose: fn(id) { FilterChoose(field, id) },
      on_blur: FilterBlur(field),
    ),
  )
}

/// The suggestions a Tasks filter shows for `text`.
pub fn filter_suggestions(
  model: Model,
  field: FilterField,
  text: String,
) -> List(combobox.Item) {
  let f = filter(field)
  combobox.matches(f.items(model), text)
}

/// The selected task's band, pinned below the table and pager. The task stays
/// mounted while collapsed so its loaded details and live events survive.
fn task_band(model: Model) -> Element(Msg) {
  case model.selected {
    None -> element.none()
    Some(#(dir, id)) -> {
      let title = task_title(model, dir, id)
      detail_band(
        model.task_collapsed,
        task_bar(
          case title {
            "" -> id
            title -> title
          },
          task_meta(model, dir, id),
          model.task_collapsed,
        ),
        task_view(model, dir, id),
      )
    }
  }
}

/// The band's bar: the task's title, its column, and the control that
/// collapses or expands its details.
fn task_bar(
  title: String,
  meta: Element(Msg),
  collapsed: Bool,
) -> Element(Msg) {
  detail_bar(title, meta, collapsed, "task-details", "task details", ToggleTask)
}

/// The selected task's title from the loaded rows, "" when not loaded.
fn task_title(model: Model, dir: String, id: String) -> String {
  task_row(model, dir, id)
  |> result.map(fn(row) { row.title })
  |> result.unwrap("")
}

/// The selected task's row, when it is loaded.
fn task_row(model: Model, dir: String, id: String) -> Result(Row, Nil) {
  list.find(rows_of(model, dir), fn(row) { row.id == id })
}

/// The task's meta line: its column, status and no-project marker.
fn task_meta(model: Model, dir: String, id: String) -> Element(Msg) {
  case task_row(model, dir, id) {
    Error(_) -> element.none()
    Ok(row) ->
      html.span(
        [attribute.class("meta")],
        solace_task.meta(
          task_workflow(model, dir),
          row_task(row, dir == homeless_dir),
        ),
      )
  }
}

/// The project's workflow, or an empty one when it has not loaded.
fn task_workflow(model: Model, dir: String) -> solace_task.Workflow {
  let dir = case dir == homeless_dir {
    True -> root_dir(model)
    False -> dir
  }
  list.key_find(model.workflows, dir)
  |> result.unwrap(solace_task.Workflow([], dict.new()))
}

/// A task built from the row and its event fields, for the component's meta
/// line. The details themselves stay in `<solace-task>`.
fn row_task(row: Row, no_project: Bool) -> solace_task.Task {
  solace_task.Task(
    id: row.id,
    title: row.title,
    body: row.body,
    column: row.column,
    rank: 0.0,
    paused: row.paused,
    notes: [],
    questions: [],
    runs: list.map(row.runs, fn(run) {
      solace_task.Run(
        id: "",
        kind: run.0,
        state: run.1,
        model: "",
        effort: "",
        log: "",
        outcome: "",
        started: 0,
        ended: 0,
        used: "",
        failed: False,
      )
    }),
    links: [],
    changes: [],
    stamps: [],
    attention: "",
    finished: row.finished,
    unexpected: False,
    tech_debt: False,
    scope: [],
    archived: False,
    depends: row.depends,
    feature: row.feature,
    no_project: no_project,
    on_board: row.on_board,
  )
}

/// One Tasks filter's name, combobox id, model fields and suggestions.
type FilterBox {
  FilterBox(
    name: String,
    key: String,
    box: fn(Model) -> combobox.State,
    set_box: fn(Model, combobox.State) -> Model,
    chosen: fn(Model) -> String,
    set_chosen: fn(Model, String) -> Model,
    items: fn(Model) -> List(combobox.Item),
  )
}

fn filter(field: FilterField) -> FilterBox {
  case field {
    ProjectField ->
      FilterBox(
        name: "Project",
        key: "filter-project",
        box: fn(m) { m.project_box },
        set_box: fn(m, box) { Model(..m, project_box: box) },
        chosen: fn(m) { m.filters.project },
        set_chosen: fn(m, id) {
          with_filter(m, fn(f) { Filters(..f, project: id) })
        },
        items: project_filter_items,
      )
    FeatureField ->
      FilterBox(
        name: "Feature",
        key: "filter-feature",
        box: fn(m) { m.feature_box },
        set_box: fn(m, box) { Model(..m, feature_box: box) },
        chosen: fn(m) { m.filters.feature },
        set_chosen: fn(m, id) {
          with_filter(m, fn(f) { Filters(..f, feature: id) })
        },
        items: feature_filter_items,
      )
    MilestoneField ->
      FilterBox(
        name: "Milestone",
        key: "filter-milestone",
        box: fn(m) { m.milestone_box },
        set_box: fn(m, box) { Model(..m, milestone_box: box) },
        chosen: fn(m) { m.filters.milestone },
        set_chosen: fn(m, id) {
          with_filter(m, fn(f) { Filters(..f, milestone: id) })
        },
        items: milestone_filter_items,
      )
  }
}

fn with_filter(model: Model, change: fn(Filters) -> Filters) -> Model {
  Model(..model, filters: change(model.filters), page: 1)
}

fn project_filter_items(model: Model) -> List(combobox.Item) {
  list.map(model.projects, fn(p) {
    combobox.Item(p.dir, p.label, "folder " <> p.dir, [p.label, p.dir])
  })
  |> list.append([
    combobox.Item(homeless_dir, "(no project)", "folder " <> homeless_dir, [
      "(no project)",
      homeless_dir,
    ]),
  ])
}

fn linked_filter_items(
  records: List(#(String, String)),
) -> List(combobox.Item) {
  list.append(
    [combobox.Item("-", "(none)", "", ["(none)", "-"])],
    list.map(records, fn(record) {
      combobox.Item(record.0, record.1, record.0, [record.1, record.0])
    }),
  )
}

fn feature_filter_items(model: Model) -> List(combobox.Item) {
  linked_filter_items(feature_pairs(model))
}

fn milestone_filter_items(model: Model) -> List(combobox.Item) {
  linked_filter_items(milestone_pairs(model))
}

fn scroll_filter_option(
  key: String,
  pressed: String,
  box: combobox.State,
) -> Effect(Msg) {
  case box.open, box.active {
    True, Some(index) ->
      case pressed {
        "ArrowDown" | "ArrowUp" ->
          effect.after_paint(fn(_, root) {
            scroll_option(root, combobox.option_id(key, index))
          })
        _ -> effect.none()
      }
    _, _ -> effect.none()
  }
}

@external(javascript, "./api.mjs", "scroll_option")
fn scroll_option(root: dynamic.Dynamic, id: String) -> Nil

fn feature_editor(model: Model) -> features.Editor(Msg) {
  features.Editor(
    draft: model.feature_draft,
    saving: model.feature_saving,
    error: model.feature_error,
    on_new: NewFeature,
    on_edit: EditFeature,
    on_title: FeatureTitle,
    on_text: FeatureText,
    on_state: FeatureState,
    on_milestone: FeatureMilestone,
    on_format: FeatureFormat,
    on_block: FeatureBlock,
    on_save: SaveFeature,
    on_cancel: CancelFeature,
    on_move: MoveFeature,
    on_grab: GrabFeature,
    on_start: StartFeature,
    // A feature removed while confirming no longer holds the list.
    deleting: case model.feature_deleting {
      Some(id) ->
        case list.any(model.features, fn(f) { f.id == id }) {
          True -> Some(id)
          False -> None
        }
      None -> None
    },
    on_delete: DeleteFeature,
    on_confirm_delete: ConfirmDeleteFeature,
    on_cancel_delete: CancelDeleteFeature,
    focused: model.focused_feature,
    on_focus: FocusFeature,
  )
}

/// The feature form's band, pinned below the stack. The form stays mounted
/// while collapsed so its draft survives.
fn feature_band(model: Model, editor: features.Editor(Msg)) -> Element(Msg) {
  case model.feature_draft {
    None -> element.none()
    Some(draft) -> {
      let title = case draft.id {
        None -> solace_task.ui_label("new_feature")
        Some(id) ->
          solace_task.ui_label("edit") <> " " <> stored_feature_title(model, id)
      }
      detail_band(
        model.feature_collapsed,
        detail_bar(
          title,
          element.none(),
          model.feature_collapsed,
          "feature-form",
          "feature details",
          ToggleFeature,
        ),
        features.form(model.prefix, model.milestones, editor, draft),
      )
    }
  }
}

fn detail_band(
  collapsed: Bool,
  bar: Element(Msg),
  body: Element(Msg),
) -> Element(Msg) {
  html.div(
    [
      attribute.class(case collapsed {
        True -> "detail-band collapsed"
        False -> "detail-band"
      }),
    ],
    [bar, body],
  )
}

fn detail_bar(
  title: String,
  meta: Element(Msg),
  collapsed: Bool,
  controls: String,
  subject: String,
  on_toggle: Msg,
) -> Element(Msg) {
  let label =
    case collapsed {
      True -> "Expand the "
      False -> "Collapse the "
    }
    <> subject
    <> ": "
    <> title
  html.header([attribute.class("detail-bar")], [
    html.button(
      [
        attribute.class("detail-toggle"),
        attribute.attribute("aria-expanded", case collapsed {
          True -> "false"
          False -> "true"
        }),
        attribute.attribute("aria-controls", controls),
        attribute.attribute("aria-label", label),
        event.on_click(on_toggle),
      ],
      [
        html.text(case collapsed {
          True -> "▸ "
          False -> "▾ "
        }),
        html.text(title),
      ],
    ),
    meta,
  ])
}

fn stored_feature_title(model: Model, id: String) -> String {
  case list.find(model.features, fn(f) { f.id == id }) {
    Ok(feature) ->
      case feature.title {
        "" -> id
        title -> title
      }
    Error(_) -> id
  }
}

const shell_styles =
  ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a}
.strategy{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;--shell-height:56px;--tabs-height:44px;--view-height:calc(100dvh - var(--shell-height));height:var(--view-height);overflow:hidden;background:var(--sheet);color:var(--ink);display:grid;grid-template-columns:minmax(0,1fr) auto;grid-template-rows:auto minmax(0,1fr)}
.strategy>.plan{grid-column:2;grid-row:2}.strategy>.tabs{grid-column:1/-1}.strategy>[role=tabpanel]{grid-column:1;grid-row:2;min-width:0;min-height:0;overflow:auto}.strategy>[role=tabpanel][hidden]{display:none}
@media (max-width:900px){.strategy{display:block;height:auto;overflow:visible}.strategy>[role=tabpanel]{overflow:visible}}
[role=tabpanel]{padding:var(--space-6)}
[role=tabpanel] p{color:var(--soft);margin:8px 0}
[role=alert]{color:var(--revision)}"

const filter_styles =
  ".filters{display:flex;flex-wrap:wrap;gap:var(--space-2) var(--space-4);align-items:flex-end;margin:0 0 var(--space-4)}
.filters>label,.filters>.combobox{flex:1 1 0;min-width:0}
.filters label{display:flex;flex-direction:column;gap:var(--space-1);font-size:13px;color:var(--soft)}
.filters .combobox{display:flex;flex-direction:column;gap:var(--space-1);font-size:13px;color:var(--soft)}
.filters input,.filters select,.tasks select{font:inherit;font-size:15px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:var(--control-pad)}
.filters input,.filters select{width:100%;margin:0}.filters input::placeholder{color:var(--soft);opacity:1}
.filters .choice{position:relative;min-width:0}
.filters .combobox input{width:100%}
.filters .options{position:absolute;z-index:5;top:calc(100% + var(--space-1));left:0;min-width:100%;max-height:240px;overflow:auto;margin:0;padding:0;list-style:none;background:var(--paper);border:1px solid var(--edge);box-shadow:0 6px 18px var(--shadow)}
.filters .options li{display:flex;gap:var(--space-3);justify-content:space-between;align-items:baseline;padding:var(--space-1) var(--space-2);color:var(--ink);cursor:pointer;white-space:nowrap}
.filters .options li.active,.filters .options li:hover{background:var(--hover)}
.filters .options li[aria-selected=true]{background:var(--sel)}
.filters .options .hint{color:var(--soft);font-size:12px}
.filters .options .no-match{color:var(--soft);cursor:default}.filters .options .no-match:hover{background:transparent}
.filters :focus-visible,.tasks :focus-visible{outline:3px solid var(--focus);outline-offset:2px}"

const tasks_styles =
  ".tasks{width:100%;border-collapse:collapse;background:var(--paper);border:1px solid var(--edge)}
.tasks th{text-align:left;font-size:12px;text-transform:uppercase;letter-spacing:.08em;padding:var(--space-2) var(--space-3);border-bottom:2px solid var(--edge)}
.tasks th .sort{font:inherit;font-weight:700;color:var(--ink);text-transform:inherit;letter-spacing:inherit;background:none;border:0;padding:0;min-height:0;text-align:inherit;cursor:pointer;white-space:nowrap}.tasks th .sort:hover{text-decoration:underline}
.tasks td{padding:var(--space-1) var(--space-3);border-top:1px solid var(--line);vertical-align:middle}
.tasks tr:hover td{background:var(--hover)}.tasks tr[aria-current=true] td{background:var(--sel)}
.tasks .open{font:inherit;font-weight:600;color:var(--ink);background:none;border:0;padding:0;min-height:0;text-align:left;cursor:pointer}
.tasks .open:hover{text-decoration:underline}
.tasks .act{"
  <> solace_task.small_control
  <> ";padding:0 var(--space-2);margin-left:var(--space-2)}.tasks .act:hover{background:var(--hover)}
.tasks .created,.tasks .closed{color:var(--soft);text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
.tasks .created span,.tasks .closed span{display:block}
.tasks th:nth-last-child(-n+2){text-align:right}
.tasks .planning{margin-left:var(--space-1)}
.tasks .homeless{font-size:13px;color:var(--soft);margin-left:var(--space-1);white-space:nowrap}
.strategy>#panel-tasks:not([hidden]){display:flex;flex-direction:column;overflow:hidden;padding-bottom:0}
.strategy>#panel-tasks:not([hidden])>.tasks-tools,.strategy>#panel-tasks:not([hidden])>.pager{flex:none}
.strategy>#panel-tasks:not([hidden])>.tasks-scroll{flex:1;min-height:0;overflow:auto;overscroll-behavior:contain;scrollbar-gutter:stable}
.pager{display:flex;align-items:center;gap:var(--space-3);margin:var(--space-2) 0 0}
.pager button{font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:var(--control-pad);cursor:pointer}.pager button:hover{background:var(--hover)}.pager button:disabled{color:var(--soft);cursor:default}.pager button:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.pager .count{font-size:13px;color:var(--soft)}"

const feature_styles =
  ".strategy>#panel-features:not([hidden]){display:flex;flex-direction:column;overflow:hidden;padding-bottom:0}
.strategy>#panel-features:not([hidden])>.features-scroll{flex:1;min-height:0;overflow:auto;overscroll-behavior:contain;scrollbar-gutter:stable}
.feature{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card);margin:0 0 var(--space-3)}
.feature h4{display:flex;flex-wrap:wrap;align-items:center;gap:var(--space-1);margin:0 0 var(--space-1);font-size:var(--h4)}
.feature-count{font-size:13px;color:var(--soft);margin:0}
.feature .feature-count{margin:var(--space-1) 0}
.feature-tasks{margin:var(--space-1) 0 0;padding-left:var(--space-5)}
.feature .open{font:inherit;color:var(--ink);background:none;border:0;padding:0;min-height:0;text-align:left;cursor:pointer}.feature .open:hover{text-decoration:underline}
.feature .open:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.feature.revealed,.milestone h3.revealed{outline:3px solid var(--focus);outline-offset:2px}
.feature{display:flex;gap:var(--space-2);position:relative;transition:transform .15s ease}.feature-body{flex:1;min-width:0}
.feature .grip{flex:none;align-self:stretch;width:var(--space-5);margin:0;padding:0;font:inherit;font-size:14px;color:var(--muted,inherit);background:none;border:0;border-radius:0;cursor:grab;touch-action:none}.feature .grip:hover{background:var(--line)}.feature .grip:disabled{cursor:default;opacity:.4}.feature .grip:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.feature.dragging{transition:none;z-index:2;box-shadow:0 6px 18px rgba(0,0,0,.18);cursor:grabbing}.feature.dragging .grip{cursor:grabbing}
.feature-form{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card);margin:0 0 var(--space-4)}.feature-form h3{margin:0 0 var(--space-3);font-size:var(--h3)}.feature-form .filters{margin:var(--space-2) 0}
.feature .edit,.feature .toggle{"
  <> solace_task.small_control
  <> ";padding:0 var(--space-2)}.feature .edit:hover,.feature .toggle:hover{background:var(--hover)}.feature .edit:disabled{color:var(--soft);cursor:default}.feature .edit:focus-visible,.feature .toggle:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.new-task{display:flex;flex-wrap:wrap;gap:var(--space-1);align-items:center;margin:var(--space-2) 0 0}.new-task input,.new-task select{font:inherit;font-size:14px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:var(--space-1) var(--space-2);min-height:var(--control-h-small)}.new-task input{flex:1 1 0;min-width:0}.new-task input::placeholder{color:var(--soft);opacity:1}.new-task :focus-visible{outline:3px solid var(--focus);outline-offset:2px}.new-task [role=alert]{flex-basis:100%;margin:var(--space-1) 0}"

const roadmap_styles =
  ".milestone{margin:0 0 var(--space-5)}.milestone h3{display:flex;flex-wrap:wrap;align-items:center;gap:var(--space-2);margin:0 0 var(--space-2);font-size:var(--h3);border-bottom:1px solid var(--line)}
.milestone h3 .toggle{display:flex;flex:1;flex-wrap:wrap;align-items:baseline;gap:var(--space-2);min-width:0;font:inherit;color:inherit;background:none;border:0;padding:0;min-height:0;text-align:left;cursor:pointer}.milestone h3 .toggle:hover{text-decoration:underline}.milestone h3 .toggle:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.milestone h3 .facts{display:flex;flex-wrap:wrap;align-items:baseline;gap:var(--space-2);min-width:0;font-weight:400}
.milestone h3 .edit{"
  <> solace_task.small_control
  <> ";font-weight:400;padding:0 var(--space-2)}.milestone h3 .edit:hover{background:var(--hover)}.milestone h3 .edit:disabled{color:var(--soft);cursor:default}
.milestone h3 .target{font-size:13px;font-weight:400;color:var(--soft)}
.feature-tasks .depends{font-size:13px;color:var(--soft)}
.feature-tasks li .edit{margin-left:var(--space-1)}
.picker{border:1px solid var(--line);margin:var(--space-1) 0 var(--space-2);padding:var(--space-1) var(--space-3)}.picker ul{list-style:none;margin:var(--space-1) 0;padding:0}.picker li{margin:0}.picker li .edit{margin:0 var(--space-1) 0 0}.picker p{font-size:13px;margin:var(--space-1) 0}
.picker input{flex:1 1 0;width:100%;min-width:0;font:inherit;font-size:14px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:var(--space-1) var(--space-2);min-height:var(--control-h-small);margin:var(--space-1) 0}.picker input::placeholder{color:var(--soft);opacity:1}
.milestone h3 .edit:focus-visible,.picker :focus-visible{outline:3px solid var(--focus);outline-offset:2px}"

const detail_styles =
  ".detail-band{flex:none;display:flex;flex-direction:column;position:relative;border-top:2px solid var(--edge);background:var(--paper);margin-top:var(--space-2)}
.detail-band>solace-task{flex:none;height:min(45dvh,calc(100dvh - 190px));min-height:0;overflow-y:auto;visibility:visible;transition:height .18s ease-out,visibility .18s;margin:0}
.detail-band.collapsed>solace-task{height:0 !important;overflow:hidden;visibility:hidden}
.detail-band>.feature-form{flex:none;max-height:min(45dvh,calc(100dvh - 190px));overflow-y:auto;margin:0}
.detail-band.collapsed>.feature-form{display:none}
.detail-bar{display:flex;flex-wrap:wrap;gap:var(--space-1) var(--space-3);align-items:center;padding:var(--space-1) 0}.detail-bar .meta{display:flex;gap:var(--space-3);align-items:center;margin:0;color:var(--soft);font-size:13px}
.detail-toggle{border:0;background:transparent;font-weight:700;font-size:17px;margin:0;padding:var(--space-1) var(--space-3);min-height:var(--control-h-small)}
.detail-toggle:hover{background:var(--hover)}
.detail-toggle:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
solace-task{display:block;margin-top:var(--space-4)}"

const vision_styles =
  ".vision{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}
.vision .md p,.vision .md li{color:var(--ink)}.md p,.md ul,.md ol,.md blockquote{margin:var(--space-1) 0}.md blockquote{padding-left:var(--space-2);border-left:3px solid var(--line);color:var(--soft)}
.md pre{background:var(--code);padding:var(--space-2);overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace}.md code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px var(--space-1)}
.vision-actions{display:flex;flex-wrap:wrap;gap:var(--space-2);margin:var(--space-3) var(--space-1)}
.vision-count{margin:var(--space-1);font-size:13px}[role=tabpanel] .vision-count[role=alert]{color:var(--revision);font-weight:600}
.vision-actions button,.toolbar button,.toolbar .button{font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;cursor:pointer}.vision-actions button{padding:var(--control-pad)}
.vision-actions button:hover,.toolbar button:hover{background:var(--hover)}.vision-actions button:disabled{color:var(--soft);cursor:default}
.vision-actions button:focus-visible,.toolbar button:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.composer solace-md-editor{--md-min-height:320px}"

const narrow_styles =
  "@media (max-width:900px){.strategy>#panel-tasks:not([hidden]){display:flex;height:calc(var(--view-height) - var(--tabs-height));overflow:hidden;padding:var(--space-6);padding-bottom:0}.detail-band>solace-task{height:min(50dvh,calc(100dvh - 230px))}.strategy>#panel-features:not([hidden]){display:flex;flex-direction:column;overflow:visible;padding:var(--space-6);padding-bottom:0}.strategy>#panel-features:not([hidden])>.features-scroll{flex:none;overflow:visible}}"

@external(javascript, "./api.mjs", "clock_date")
fn clock_date(seconds: Int) -> String

@external(javascript, "./api.mjs", "clock_time")
fn clock_time(seconds: Int) -> String

/// A moment as the two lines of a date column: its date, then its time.
fn clock_cell(seconds: Int) -> List(Element(Msg)) {
  [
    html.span([attribute.class("date")], [html.text(clock_date(seconds))]),
    html.span([attribute.class("time")], [html.text(clock_time(seconds))]),
  ]
}

/// A record's title, or its id when the record isn't loaded.
fn record_title(records: List(#(String, String)), id: String) -> String {
  list.key_find(records, id) |> result.unwrap(id)
}

/// A task's milestone: a link to its record, or its id until the record
/// loads. A task with no milestone shows a dash.
fn milestone_cell(model: Model, row: Row) -> Element(Msg) {
  case row.milestone {
    None -> html.text("—")
    Some("") -> html.text("—")
    Some(id) -> {
      let title = record_title(milestone_pairs(model), id)
      html.a(
        [
          attribute.attribute("href", "/milestone/" <> id),
          attribute.attribute(
            "aria-label",
            title <> ", milestone of " <> row.title,
          ),
        ],
        [html.text(title)],
      )
    }
  }
}

const state_styles =
  ":host{--state-proposed:#6a5ba6;--state-upcoming:#2f6db5;--state-active:#1f7d35;--state-complete:#000;--state-canceled:#46575a;--state-ink:#fff}
.state{font-size:11px;text-transform:uppercase;letter-spacing:.08em;border:1px solid var(--line);padding:1px var(--space-1);color:var(--soft);font-weight:400}
.state.state-proposed{background:var(--state-proposed);border-color:var(--state-proposed);color:var(--state-ink)}
.state.state-upcoming{background:var(--state-upcoming);border-color:var(--state-upcoming);color:var(--state-ink)}
.state.state-active{background:var(--state-active);border-color:var(--state-active);color:var(--state-ink)}
.state.state-complete{background:var(--state-complete);border-color:var(--state-complete);color:var(--state-ink)}
.state.state-canceled{background:var(--state-canceled);border-color:var(--state-canceled);color:var(--state-ink)}
"

/// Shows the forms whose descriptions were saved, so a reload brings an active
/// edit back without opening it again.
fn restore_forms(model: Model) -> Model {
  model
  |> restore_feature
  |> restore_milestone
  |> restore_new_task
}

fn restore_feature(model: Model) -> Model {
  let fresh = with_feature_draft(features.new_draft())
  case
    restored_draft(
      model.feature_draft,
      fresh,
      fn(draft) { draft.body },
      model.features,
      features.draft_of,
      fn(feature) {
        form_draft.load(features.text_id, form_draft.owner(Some(feature.id)))
        != ""
      },
    )
  {
    None -> model
    Some(Restored(draft, fresh)) ->
      Model(
        ..model,
        feature_draft: Some(draft),
        feature_collapsed: False,
        feature_error: "",
        feature_request: case fresh {
          True ->
            case model.feature_request {
              "" -> request_id()
              id -> id
            }
          False -> model.feature_request
        },
      )
  }
}

fn restore_milestone(model: Model) -> Model {
  let fresh = with_milestone_draft(roadmap.new_draft())
  case
    restored_draft(
      model.milestone_draft,
      fresh,
      fn(draft) { draft.body },
      model.milestones,
      roadmap.draft_of,
      fn(milestone) {
        form_draft.load(roadmap.text_id, form_draft.owner(Some(milestone.id)))
        != ""
      },
    )
  {
    None -> model
    Some(Restored(draft, fresh)) ->
      Model(
        ..model,
        milestone_draft: Some(draft),
        milestone_error: "",
        milestone_feature_query: "",
        milestone_request: case fresh {
          True ->
            case model.milestone_request {
              "" -> request_id()
              id -> id
            }
          False -> model.milestone_request
        },
      )
  }
}

/// Opens the Tasks tab's form when its description was saved.
fn restore_new_task(model: Model) -> Model {
  case model.new_task {
    Some(_) -> model
    None ->
      case solace_task.draft_load(new_task_id, "new") {
        "" -> model
        body ->
          Model(
            ..model,
            new_task: Some(NewTask("", body, root_dir(model), "", "")),
            new_task_error: "",
            new_task_request: case model.new_task_request {
              "" -> request_id()
              id -> id
            },
          )
      }
  }
}

/// The new task's saved description, "" when there is none.
fn new_task_draft_load() -> String {
  form_draft.load(new_task_id, "new")
}

/// Saves the new task's description as it is typed.
fn new_task_draft_save(body: String) -> Nil {
  form_draft.save(new_task_id, "new", body)
}

/// Removes the new task's description draft.
fn new_task_draft_clear() -> Nil {
  form_draft.clear(new_task_id, "new")
}

fn with_feature_draft(draft: features.Draft) -> features.Draft {
  form_draft.with_body(
    features.text_id,
    form_draft.owner(draft.id),
    draft,
    fn(d, body) { features.Draft(..d, body: body) },
  )
}

fn with_milestone_draft(draft: roadmap.Draft) -> roadmap.Draft {
  form_draft.with_body(
    roadmap.text_id,
    form_draft.owner(draft.id),
    draft,
    fn(d, body) { roadmap.Draft(..d, body: body) },
  )
}

type Restored(a) {
  Restored(draft: a, fresh: Bool)
}

fn restored_draft(
  current: Option(a),
  fresh: a,
  body: fn(a) -> String,
  records: List(b),
  draft_of: fn(b) -> a,
  stored: fn(b) -> Bool,
) -> Option(Restored(a)) {
  case current {
    Some(_) -> None
    None ->
      case body(fresh) {
        "" ->
          case list.find(records, stored) {
            Ok(record) -> Some(Restored(draft_of(record), False))
            Error(_) -> None
          }
        _ -> Some(Restored(fresh, True))
      }
  }
}

fn navigation_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    Select(tab) if tab == model.tab -> Some(#(model, effect.none()))
    Select(tab) ->
      Some(#(
        Model(..model, tab: tab),
        event.emit("tab", json.string(tab_name(tab))),
      ))
    TabShown(tab) -> Some(#(Model(..model, tab: tab), effect.none()))
    // From the Features tab the task shows on the Tasks tab.
    Open(dir, id) ->
      Some(
        #(
          Model(
            ..model,
            tab: Tasks,
            selected: Some(#(dir, id)),
            task_collapsed: False,
            live: [],
          ),
          case model.tab {
            Tasks -> effect.none()
            _ -> event.emit("tab", json.string(tab_name(Tasks)))
          },
        ),
      )
    Focus(_, _, "") -> Some(#(model, effect.none()))
    Focus("feature", _, id) -> {
      let model = expand_feature_milestone(model, id)
      Some(show_record(
        Model(..model, focused_feature: Some(id)),
        Features,
        "feature",
        id,
      ))
    }
    FocusFeature(id) ->
      Some(#(
        Model(..model, focused_feature: case model.focused_feature == Some(id) {
          True -> None
          False -> Some(id)
        }),
        effect.none(),
      ))
    Focus("milestone", _, id) ->
      Some(show_record(expand_milestone(model, id), Roadmap, "milestone", id))
    // `<solace-task>` scrolls into view now, or once the task loads. A link
    // names the task's home folder; a homeless task's row (D-129) lives in
    // the homeless bucket.
    Focus(_, named, id) -> {
      let dir = bucket_of(model, named, id)
      Some(#(
        Model(
          ..model,
          tab: Tasks,
          selected: Some(#(dir, id)),
          task_collapsed: False,
          page: page_of(visible(model), dir, id, model.page),
          live: case model.selected == Some(#(dir, id)) {
            True -> model.live
            False -> []
          },
          reveal: #(id, model.reveal.1 + 1),
        ),
        effect.none(),
      ))
    }
    TaskMoved(dir, id) ->
      Some(case model.selected {
        Some(#(_, shown)) if shown == id -> #(
          Model(..model, selected: Some(#(bucket_of(model, dir, id), id))),
          effect.none(),
        )
        _ -> #(model, effect.none())
      })
    ToggleTask ->
      Some(case model.selected {
        None -> #(model, effect.none())
        Some(_) -> #(
          Model(..model, task_collapsed: !model.task_collapsed),
          effect.none(),
        )
      })
    Filter(filters) ->
      Some(#(Model(..model, filters: filters, page: 1), effect.none()))
    FilterText(field, text) -> {
      let f = filter(field)
      Some(case text {
        "" -> {
          let model = f.set_chosen(model, "")
          #(
            f.set_box(model, combobox.typing(combobox.blank(), "")),
            effect.none(),
          )
        }
        _ -> #(
          f.set_box(model, combobox.typing(f.box(model), text)),
          effect.none(),
        )
      })
    }
    FilterOpen(field) -> {
      let f = filter(field)
      let box = f.box(model)
      Some(case box.open {
        True -> #(model, effect.none())
        False -> #(
          f.set_box(
            model,
            combobox.opened(box, f.items(model), f.chosen(model)),
          ),
          effect.none(),
        )
      })
    }
    FilterKey(field, key) -> {
      let f = filter(field)
      let #(box, picked) =
        combobox.key(f.box(model), key, f.items(model), f.chosen(model))
      let model = f.set_box(model, box)
      Some(case picked {
        Some(id) -> #(f.set_chosen(model, id), effect.none())
        None -> #(model, scroll_filter_option(f.key, key, box))
      })
    }
    FilterChoose(field, id) -> {
      let f = filter(field)
      Some(#(
        f.set_box(f.set_chosen(model, id), combobox.blank()),
        effect.none(),
      ))
    }
    FilterBlur(field) -> {
      let f = filter(field)
      Some(#(f.set_box(model, combobox.blank()), effect.none()))
    }
    Page(n) -> Some(#(Model(..model, page: n), effect.none()))
    SortBy(key) -> {
      let ascending = case model.sort {
        Some(#(k, asc)) if k == key -> !asc
        _ -> True
      }
      Some(#(
        Model(..model, sort: Some(#(key, ascending)), page: 1),
        effect.none(),
      ))
    }
    // Allowed in final columns and during runs (D-87, D-100).
    Link(dir, id, field, value) ->
      Some(update(model, Links(dir, id, [#(field, value)])))
    // Through the board's actions API, as the task's details send it.
    PlanTask(dir, id) ->
      Some(#(
        model,
        effect.from(fn(send) {
          request(
            model.prefix,
            base_of(model, dir) <> "/tasks/" <> id <> "/actions",
            "POST",
            json.to_string(json.object([#("action", json.string("plan"))])),
            fn(text) { send(Planned(dir, text)) },
          )
        }),
      ))
    Planned(dir, text) ->
      Some(case json.parse(text, row_decoder()) {
        Ok(row) -> #(Model(..put(model, dir, row), error: ""), effect.none())
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      })
    Links(dir, id, fields) ->
      Some(#(
        model,
        effect.from(fn(send) {
          request(
            model.prefix,
            base_of(model, dir) <> "/tasks/" <> id,
            "PATCH",
            json.to_string(links_body(fields)),
            fn(text) { send(Patched(dir, id, text)) },
          )
        }),
      ))
    // The link after a new task's create reports on its feature, or in the
    // Tasks tab's form ("" for the feature). A refusal is matched by its
    // task, so another task's refused PATCH isn't reported there.
    Patched(dir, pid, text) ->
      Some(case json.parse(text, row_decoder()), model.task_linking {
        Ok(row), Some(#(d, id, "")) if d == dir && row.id == id -> #(
          new_task_done(put(model, dir, row), dir, id),
          effect.none(),
        )
        // The task exists without its links: it opens, and the form stays
        // with a fresh draft so a retry doesn't make it again.
        Error(_), Some(#(d, id, "")) if d == dir && id == pid -> {
          let #(picked, project) = case model.new_task {
            Some(t) -> #(
              case new_task_links(t) {
                [#("feature", _)] -> "feature"
                [#("milestone", _)] -> "milestone"
                _ -> "feature or milestone"
              },
              t.project,
            )
            None -> #("feature or milestone", root_dir(model))
          }
          #(
            Model(
              ..new_task_done(model, dir, id),
              new_task: Some(NewTask("", "", project, "", "")),
              new_task_request: request_id(),
              new_task_error: "The task was created without its "
                <> picked
                <> ": "
                <> error(text),
            ),
            effect.none(),
          )
        }
        Ok(row), Some(#(d, id, _)) if d == dir && row.id == id -> #(
          Model(..put(model, dir, row), task_saving: None, task_linking: None),
          effect.none(),
        )
        Ok(row), _ -> #(Model(..put(model, dir, row), error: ""), effect.none())
        Error(_), Some(#(d, id, feature)) if d == dir && id == pid -> #(
          Model(
            ..model,
            task_saving: None,
            task_linking: None,
            task_errors: set(
              model.task_errors,
              feature,
              "The task was created without the link to this feature: "
                <> error(text),
            ),
          ),
          effect.none(),
        )
        Error(_), _ -> #(Model(..model, error: error(text)), effect.none())
      })
    _ -> None
  }
}

fn connection_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    Connected -> {
      let #(model, fx) = start(Model(..model, connected: True))
      let #(model, plan_fx) = case model.plan_open {
        "" -> #(model, effect.none())
        rid -> plan_fetch(model, rid)
      }
      Some(#(
        model,
        effect.batch([
          fx,
          plan_fx,
          effect.before_paint(fn(send, root) { send(Attached(root)) }),
        ]),
      ))
    }
    Attached(root) -> Some(#(Model(..model, root: root), effect.none()))
    Disconnected ->
      Some(#(
        Model(
          ..stop(model),
          connected: False,
          plan_poll: model.plan_poll + 1,
          plan_clearing: None,
        ),
        effect.none(),
      ))
    BaseChanged(base) -> {
      let model = Model(..model, prefix: base)
      Some(case model.connected {
        True -> start(model)
        False -> #(model, effect.none())
      })
    }
    Listening(source) ->
      Some(#(Model(..model, source: Some(source)), effect.none()))
    ProjectsLoaded(text) ->
      Some(case json.parse(text, projects_decoder()) {
        Ok(#(workspace, projects)) -> #(
          settle(
            Model(..model, projects: projects, workspace: workspace, error: ""),
          ),
          effect.batch(
            list.map(projects, fn(p) {
              get(model, p.base <> "/workflow", WorkflowLoaded(p.dir, _))
            }),
          ),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      })
    WorkflowLoaded(dir, text) ->
      Some(case json.parse(text, solace_task.workflow_decoder()) {
        Ok(w) -> #(
          Model(..model, workflows: set(model.workflows, dir, w)),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      })
    // What a new connection missed is replayed, then applied at once.
    Live(text) if !model.caught_up ->
      Some(#(Model(..model, replay: [text, ..model.replay]), effect.none()))
    Live(text) -> {
      let fx =
        effect.batch([
          case projects_event(text) {
            True -> get(model, "/projects", ProjectsLoaded)
            False -> effect.none()
          },
          case models_event(text) {
            True -> get(model, "/models", ModelsRefreshed)
            False -> effect.none()
          },
        ])
      Some(case knows(model, text) {
        True -> {
          let #(next, more) =
            plan_track(model, plan_follow(model, hand(live(model, text), text)))
          let more = case needs_attention(model.runs, next.runs) {
            True -> effect.batch([more, event.emit("attention", json.null())])
            False -> more
          }
          #(next, effect.batch([fx, more]))
        }
        // A project not listed yet: held until the list names it.
        False -> #(Model(..model, replay: [text, ..model.replay]), fx)
      })
    }
    CaughtUp -> {
      let model = restore_forms(resync(settle(Model(..model, caught_up: True))))
      Some(#(
        model,
        effect.batch([
          get(model, "/models", ModelsRefreshed),
          get(model, "/role-models", RoleModelsLoaded),
        ]),
      ))
    }
    _ -> None
  }
}

fn vision_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    EditVision ->
      Some(#(
        Model(
          ..model,
          draft: Some(#(model.vision.text, model.vision.revision)),
          vision_error: "",
        ),
        effect.none(),
      ))
    ReloadVision ->
      Some(#(
        Model(
          ..model,
          draft: Some(#(model.vision.text, model.vision.revision)),
          vision_error: "",
        ),
        effect.none(),
      ))
    CancelVision ->
      Some(#(Model(..model, draft: None, vision_error: ""), effect.none()))
    Draft(text) ->
      Some(case model.draft {
        Some(#(_, base)) -> #(
          Model(..model, draft: Some(#(text, base))),
          effect.none(),
        )
        None -> #(model, effect.none())
      })
    VisionFormat(before, after) ->
      Some(case model.draft {
        Some(#(text, _)) -> #(
          model,
          effect.from(fn(send) {
            send(
              Draft(solace_task.wrap(model.root, vision_id, text, before, after)),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    VisionBlock(kind) ->
      Some(case model.draft {
        Some(#(text, _)) -> #(
          model,
          effect.from(fn(send) {
            send(Draft(solace_task.block(model.root, vision_id, text, kind)))
          }),
        )
        None -> #(model, effect.none())
      })
    // One save at a time, within the limit (D-92); its reply ends the edit or
    // says why not.
    SaveVision -> {
      let fits = case model.draft {
        Some(#(text, _)) -> vision_count(text) <= vision_limit
        None -> False
      }
      Some(case model.draft, model.saving, fits {
        Some(#(text, base)), False, True -> #(
          Model(..model, saving: True, vision_error: ""),
          effect.from(fn(send) {
            request(
              model.prefix,
              "/vision",
              "PUT",
              json.to_string(vision_body(text, base)),
              fn(reply) { send(VisionSaved(reply)) },
            )
          }),
        )
        _, _, _ -> #(model, effect.none())
      })
    }
    VisionSaved(text) ->
      Some(case json.parse(text, vision_decoder()) {
        Ok(v) -> #(
          Model(
            ..model,
            vision: v,
            draft: None,
            saving: False,
            vision_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, saving: False, vision_error: error(text)),
          effect.none(),
        )
      })
    _ -> None
  }
}

fn feature_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    NewFeature if !model.feature_saving ->
      Some(#(
        Model(
          ..model,
          feature_draft: Some(with_feature_draft(features.new_draft())),
          feature_collapsed: False,
          feature_error: "",
          feature_request: request_id(),
        ),
        effect.none(),
      ))
    EditFeature(id) if !model.feature_saving ->
      Some(case list.find(model.features, fn(f) { f.id == id }) {
        Ok(f) -> #(
          Model(
            ..model,
            feature_draft: Some(with_feature_draft(features.draft_of(f))),
            feature_collapsed: False,
            feature_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(model, effect.none())
      })
    NewFeature | EditFeature(_) -> Some(#(model, effect.none()))
    CancelFeature -> {
      case model.feature_draft {
        Some(draft) ->
          form_draft.clear(features.text_id, form_draft.owner(draft.id))
        None -> Nil
      }
      Some(#(
        Model(
          ..model,
          feature_draft: None,
          feature_collapsed: False,
          feature_error: "",
        ),
        effect.none(),
      ))
    }
    ToggleFeature ->
      Some(case model.feature_draft {
        None -> #(model, effect.none())
        Some(_) -> #(
          Model(..model, feature_collapsed: !model.feature_collapsed),
          effect.none(),
        )
      })
    FeatureTitle(title) ->
      Some(edit_feature(model, fn(d) { features.Draft(..d, title: title) }))
    FeatureText(body) -> {
      case model.feature_draft {
        Some(draft) ->
          form_draft.save(features.text_id, form_draft.owner(draft.id), body)
        None -> Nil
      }
      Some(edit_feature(model, fn(d) { features.Draft(..d, body: body) }))
    }
    FeatureState(state) ->
      Some(edit_feature(model, fn(d) { features.Draft(..d, state: state) }))
    FeatureMilestone(id) ->
      Some(
        edit_feature(model, fn(d) {
          features.Draft(..d, milestone: case id {
            "" -> None
            id -> Some(id)
          })
        }),
      )
    FeatureFormat(before, after) ->
      Some(case model.feature_draft {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              FeatureText(solace_task.wrap(
                model.root,
                features.text_id,
                d.body,
                before,
                after,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    FeatureBlock(kind) ->
      Some(case model.feature_draft {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              FeatureText(solace_task.block(
                model.root,
                features.text_id,
                d.body,
                kind,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    // One save at a time. A create names its `request`; an edit sends only
    // what changed, and an edit that changed nothing just closes.
    SaveFeature ->
      Some(case model.feature_draft, model.feature_saving {
        Some(d), False -> {
          let save = fn(path, method, body) {
            #(
              Model(..model, feature_saving: True, feature_error: ""),
              effect.from(fn(send) {
                request(
                  model.prefix,
                  path,
                  method,
                  json.to_string(body),
                  fn(reply) { send(FeatureSaved(reply)) },
                )
              }),
            )
          }
          case d.id {
            None ->
              save(
                "/features",
                "POST",
                features.create_body(d, model.feature_request),
              )
            Some(id) -> {
              // A feature removed meanwhile: the server says so (E7023).
              let before =
                list.find(model.features, fn(f) { f.id == id })
                |> result.unwrap(features.Feature(
                  id,
                  "",
                  "",
                  None,
                  0.0,
                  "",
                  None,
                ))
              case features.changes(before, d) {
                [] -> {
                  form_draft.clear(features.text_id, form_draft.owner(d.id))
                  #(
                    Model(..model, feature_draft: None, feature_error: ""),
                    effect.none(),
                  )
                }
                fields -> save("/features/" <> id, "PATCH", json.object(fields))
              }
            }
          }
        }
        _, _ -> #(model, effect.none())
      })
    FeatureSaved(text) ->
      Some(case json.parse(text, features.decoder()) {
        Ok(f) -> {
          case model.feature_draft {
            Some(draft) ->
              form_draft.clear(features.text_id, form_draft.owner(draft.id))
            None -> Nil
          }
          #(
            Model(
              ..model,
              features: keep(model.features, f, fn(f) { f.id }),
              feature_draft: None,
              feature_collapsed: False,
              feature_saving: False,
              feature_error: "",
            ),
            effect.none(),
          )
        }
        Error(_) -> #(
          Model(..model, feature_saving: False, feature_error: error(text)),
          effect.none(),
        )
      })
    // An attribute set before the element is added only takes effect then.
    // One move or save at a time: a second before the first is answered
    // would take the one `feature_moving` slot a refusal restores from.
    MoveFeature(id, rank) if !model.feature_saving ->
      Some(#(
        Model(..model, feature_error: "", feature_saving: True),
        effect.from(fn(send) {
          request(
            model.prefix,
            "/features/" <> id,
            "PATCH",
            json.to_string(json.object([#("rank", json.float(rank))])),
            fn(reply) { send(FeatureMoved(reply)) },
          )
        }),
      ))
    GrabFeature(id, y) if !model.feature_saving ->
      Some(#(
        model,
        effect.from(fn(send) {
          drag_card(model.root, id, y, fn(index) {
            send(DropFeature(id, index))
          })
        }),
      ))
    // The new order shows at once, with no jump back while the PATCH is
    // answered; a save waits for the same answer, so a drag and a save of one
    // feature don't send two PATCHes at once.
    DropFeature(id, index) if !model.feature_saving -> {
      let group =
        features.groups(model.milestones, model.features)
        |> list.find(fn(g) {
          list.any(features.group_features(g), fn(f) { f.id == id })
        })
        |> result.map(features.group_features)
        |> result.unwrap([])
      Some(
        case
          features.drop_rank(group, id, index),
          list.find(model.features, fn(f) { f.id == id })
        {
          Some(rank), Ok(f) -> {
            let #(model, fx) = update(model, MoveFeature(id, rank))
            #(
              Model(
                ..model,
                features: keep(
                  model.features,
                  features.Feature(..f, rank: rank),
                  fn(f) { f.id },
                ),
                feature_moving: Some(#(id, f.rank)),
                feature_saving: True,
              ),
              fx,
            )
          }
          _, _ -> #(model, effect.none())
        },
      )
    }
    MoveFeature(_, _) | GrabFeature(_, _) | DropFeature(_, _) ->
      Some(#(model, effect.none()))
    // The server keeps its start time and color and plans its Backlog tasks
    // (D-114, D-134). Its reply answers the start alone: a start changes the
    // state, not the order, so it never moves a feature a drag is answering.
    StartFeature(id) ->
      Some(#(
        Model(..model, feature_error: ""),
        effect.from(fn(send) {
          request(
            model.prefix,
            "/features/" <> id,
            "PATCH",
            json.to_string(json.object([#("state", json.string("active"))])),
            fn(reply) { send(FeatureStarted(reply)) },
          )
        }),
      ))
    // A reply after the feature was removed doesn't bring it back.
    FeatureMoved(text) ->
      Some(case json.parse(text, features.decoder()) {
        Ok(f) -> #(
          Model(
            ..model,
            features: case list.any(model.features, fn(x) { x.id == f.id }) {
              True -> keep(model.features, f, fn(f) { f.id })
              False -> model.features
            },
            feature_error: "",
            feature_moving: None,
            feature_saving: False,
          ),
          effect.none(),
        )
        // A refused drop goes back to where it was.
        Error(_) -> #(
          Model(
            ..model,
            features: case model.feature_moving {
              Some(#(id, rank)) ->
                list.map(model.features, fn(f) {
                  case f.id == id {
                    True -> features.Feature(..f, rank: rank)
                    False -> f
                  }
                })
              None -> model.features
            },
            feature_error: error(text),
            feature_moving: None,
            feature_saving: False,
          ),
          effect.none(),
        )
      })
    // A start's reply: the feature as the server has it. A refused start
    // says why and changes nothing, so the order a drag set stays as it is.
    FeatureStarted(text) ->
      Some(case json.parse(text, features.decoder()) {
        Ok(f) -> #(
          Model(
            ..model,
            features: case list.any(model.features, fn(x) { x.id == f.id }) {
              True -> keep(model.features, f, fn(f) { f.id })
              False -> model.features
            },
            feature_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(Model(..model, feature_error: error(text)), effect.none())
      })
    // The `{removedFeature}` and `{task}` events update the page.
    FeatureDeleted(text) ->
      Some(case json.parse(text, features.decoder()) {
        Ok(_) -> #(
          Model(..model, feature_deleting: None, feature_error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, feature_error: error(text)), effect.none())
      })
    DeleteFeature(id) if model.feature_draft == None ->
      Some(#(
        Model(..model, feature_deleting: Some(id), feature_error: ""),
        effect.none(),
      ))
    DeleteFeature(_) -> Some(#(model, effect.none()))
    CancelDeleteFeature ->
      Some(#(
        Model(..model, feature_deleting: None, feature_error: ""),
        effect.none(),
      ))
    ConfirmDeleteFeature ->
      Some(case model.feature_deleting {
        Some(id) -> #(
          model,
          effect.from(fn(send) {
            request(model.prefix, "/features/" <> id, "DELETE", "", fn(reply) {
              send(FeatureDeleted(reply))
            })
          }),
        )
        None -> #(model, effect.none())
      })
    _ -> None
  }
}

fn milestone_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    NewMilestone if !model.milestone_saving ->
      Some(#(
        Model(
          ..model,
          milestone_draft: Some(with_milestone_draft(roadmap.new_draft())),
          milestone_error: "",
          milestone_request: request_id(),
          milestone_feature_query: "",
        ),
        effect.none(),
      ))
    EditMilestone(id) if !model.milestone_saving ->
      Some(case list.find(model.milestones, fn(m) { m.id == id }) {
        Ok(m) -> {
          let model = expand_milestone(model, m.id)
          #(
            Model(
              ..model,
              milestone_draft: Some(with_milestone_draft(roadmap.draft_of(m))),
              milestone_error: "",
              milestone_feature_query: "",
            ),
            effect.none(),
          )
        }
        Error(_) -> #(model, effect.none())
      })
    NewMilestone | EditMilestone(_) -> Some(#(model, effect.none()))
    ToggleMilestone(id) -> Some(#(toggle_milestone(model, id), effect.none()))
    CancelMilestone -> {
      case model.milestone_draft {
        Some(draft) ->
          form_draft.clear(roadmap.text_id, form_draft.owner(draft.id))
        None -> Nil
      }
      Some(#(
        Model(
          ..model,
          milestone_draft: None,
          milestone_error: "",
          milestone_feature_query: "",
        ),
        effect.none(),
      ))
    }
    MilestoneTitle(title) ->
      Some(edit_milestone(model, fn(d) { roadmap.Draft(..d, title: title) }))
    MilestoneText(body) -> {
      case model.milestone_draft {
        Some(draft) ->
          form_draft.save(roadmap.text_id, form_draft.owner(draft.id), body)
        None -> Nil
      }
      Some(edit_milestone(model, fn(d) { roadmap.Draft(..d, body: body) }))
    }
    MilestoneState(state) ->
      Some(edit_milestone(model, fn(d) { roadmap.Draft(..d, state: state) }))
    MilestoneTargetDate(date) ->
      Some(
        edit_milestone(model, fn(d) {
          roadmap.Draft(..d, target_date: case date {
            "" -> None
            date -> Some(date)
          })
        }),
      )
    MilestoneFeatureQuery(query) ->
      Some(#(Model(..model, milestone_feature_query: query), effect.none()))
    MilestoneFormat(before, after) ->
      Some(case model.milestone_draft {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              MilestoneText(solace_task.wrap(
                model.root,
                roadmap.text_id,
                d.body,
                before,
                after,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    MilestoneBlock(kind) ->
      Some(case model.milestone_draft {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              MilestoneText(solace_task.block(
                model.root,
                roadmap.text_id,
                d.body,
                kind,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    // As SaveFeature: one at a time, a create names its `request`, an edit
    // sends only what changed.
    SaveMilestone ->
      Some(case model.milestone_draft, model.milestone_saving {
        Some(d), False -> {
          let save = fn(path, method, body) {
            #(
              Model(..model, milestone_saving: True, milestone_error: ""),
              effect.from(fn(send) {
                request(
                  model.prefix,
                  path,
                  method,
                  json.to_string(body),
                  fn(reply) { send(MilestoneSaved(reply)) },
                )
              }),
            )
          }
          case d.id {
            None ->
              save(
                "/milestones",
                "POST",
                roadmap.create_body(d, model.milestone_request),
              )
            Some(id) -> {
              // A milestone removed meanwhile: the server says so (E7023).
              let before =
                list.find(model.milestones, fn(m) { m.id == id })
                |> result.unwrap(features.Milestone(id, "", 0.0, "", None, ""))
              case roadmap.changes(before, d) {
                [] -> {
                  form_draft.clear(roadmap.text_id, form_draft.owner(d.id))
                  #(
                    Model(..model, milestone_draft: None, milestone_error: ""),
                    effect.none(),
                  )
                }
                fields ->
                  save("/milestones/" <> id, "PATCH", json.object(fields))
              }
            }
          }
        }
        _, _ -> #(model, effect.none())
      })
    MilestoneSaved(text) ->
      Some(case json.parse(text, features.milestone_decoder()) {
        Ok(m) -> {
          case model.milestone_draft {
            Some(draft) ->
              form_draft.clear(roadmap.text_id, form_draft.owner(draft.id))
            None -> Nil
          }
          #(
            Model(
              ..model,
              milestones: keep(model.milestones, m, fn(m) { m.id }),
              milestone_draft: None,
              milestone_saving: False,
              milestone_error: "",
            ),
            effect.none(),
          )
        }
        Error(_) -> #(
          Model(..model, milestone_saving: False, milestone_error: error(text)),
          effect.none(),
        )
      })
    DeleteMilestone(id) if model.milestone_draft == None ->
      Some(#(
        Model(..model, milestone_deleting: Some(id), milestone_error: ""),
        effect.none(),
      ))
    DeleteMilestone(_) -> Some(#(model, effect.none()))
    CancelDeleteMilestone ->
      Some(#(
        Model(..model, milestone_deleting: None, milestone_error: ""),
        effect.none(),
      ))
    ConfirmDeleteMilestone ->
      Some(case model.milestone_deleting {
        Some(id) -> #(
          model,
          effect.from(fn(send) {
            request(model.prefix, "/milestones/" <> id, "DELETE", "", fn(reply) {
              send(MilestoneDeleted(reply))
            })
          }),
        )
        None -> #(model, effect.none())
      })
    // The `{removedMilestone}`, `{feature}` and `{task}` events update the page.
    MilestoneDeleted(text) ->
      Some(case json.parse(text, features.milestone_decoder()) {
        Ok(m) -> {
          let model = collapse_milestone(model, m.id)
          #(
            Model(..model, milestone_deleting: None, milestone_error: ""),
            effect.none(),
          )
        }
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      })
    TargetFeature(_, _) if model.milestone_targeting ->
      Some(#(model, effect.none()))
    TargetFeature(id, milestone) ->
      Some(#(
        Model(..model, milestone_error: "", milestone_targeting: True),
        effect.from(fn(send) {
          request(
            model.prefix,
            "/features/" <> id,
            "PATCH",
            json.to_string(
              json.object([
                #("milestone", case milestone {
                  "" -> json.null()
                  m -> json.string(m)
                }),
              ]),
            ),
            fn(reply) { send(FeatureTargeted(reply)) },
          )
        }),
      ))
    FeatureTargeted(text) ->
      Some(case json.parse(text, features.decoder()) {
        Ok(f) -> #(
          Model(
            ..model,
            features: case list.any(model.features, fn(x) { x.id == f.id }) {
              True -> keep(model.features, f, fn(f) { f.id })
              False -> model.features
            },
            milestone_error: "",
            milestone_targeting: False,
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(
            ..model,
            milestone_error: error(text),
            milestone_targeting: False,
          ),
          effect.none(),
        )
      })
    _ -> None
  }
}

fn task_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    TaskTitle(feature, title) ->
      Some(#(
        Model(..model, task_titles: set(model.task_titles, feature, title)),
        effect.none(),
      ))
    TaskProject(dir) ->
      Some(#(Model(..model, task_project: dir), effect.none()))
    // One create at a time. Then the link is one `Link`, whose `Patched`
    // reports a refusal on the feature.
    AddTask(feature) -> {
      let title =
        list.key_find(model.task_titles, feature)
        |> result.unwrap("")
        |> string.trim
      Some(case title, model.task_saving {
        "", _ | _, Some(_) -> #(model, effect.none())
        _, None -> {
          let dir = task_dir(model)
          let id = case list.key_find(model.task_requests, feature) {
            Ok(id) -> id
            Error(_) -> request_id()
          }
          #(
            Model(
              ..model,
              task_saving: Some(feature),
              task_requests: set(model.task_requests, feature, id),
              task_errors: drop(model.task_errors, feature),
            ),
            effect.from(fn(send) {
              request(
                model.prefix,
                base_of(model, dir) <> "/tasks",
                "POST",
                json.to_string(
                  json.object([
                    #("title", json.string(title)),
                    #("body", json.string("")),
                    #("request", json.string(id)),
                  ]),
                ),
                fn(reply) { send(TaskCreated(dir, feature, reply)) },
              )
            }),
          )
        }
      })
    }
    NewTaskOpen if model.new_task == None ->
      Some(#(
        Model(
          ..model,
          new_task: Some(NewTask(
            "",
            new_task_draft_load(),
            root_dir(model),
            "",
            "",
          )),
          new_task_error: "",
          new_task_request: request_id(),
        ),
        effect.none(),
      ))
    NewTaskOpen -> Some(#(model, effect.none()))
    // A create or its link in flight finishes in the form.
    NewTaskCancel if model.task_saving != Some("") -> {
      case model.new_task {
        Some(_) -> new_task_draft_clear()
        None -> Nil
      }
      Some(#(Model(..model, new_task: None, new_task_error: ""), effect.none()))
    }
    NewTaskCancel -> Some(#(model, effect.none()))
    NewTaskTitle(title) ->
      Some(edit_new_task(model, fn(d) { NewTask(..d, title: title) }))
    NewTaskText(body) -> {
      case model.new_task {
        Some(_) -> new_task_draft_save(body)
        None -> Nil
      }
      Some(edit_new_task(model, fn(d) { NewTask(..d, body: body) }))
    }
    NewTaskProject(dir) ->
      Some(edit_new_task(model, fn(d) { NewTask(..d, project: dir) }))
    NewTaskFeature(id) ->
      Some(edit_new_task(model, fn(d) { NewTask(..d, feature: id) }))
    NewTaskMilestone(id) ->
      Some(edit_new_task(model, fn(d) { NewTask(..d, milestone: id) }))
    NewTaskFormat(before, after) ->
      Some(case model.new_task {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              NewTaskText(solace_task.wrap(
                model.root,
                new_task_id,
                d.body,
                before,
                after,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    NewTaskBlock(kind) ->
      Some(case model.new_task {
        Some(d) -> #(
          model,
          effect.from(fn(send) {
            send(
              NewTaskText(solace_task.block(
                model.root,
                new_task_id,
                d.body,
                kind,
              )),
            )
          }),
        )
        None -> #(model, effect.none())
      })
    // One create at a time, under the request chosen when the form opened.
    CreateTask ->
      Some(case model.new_task, model.task_saving {
        Some(d), None ->
          case string.trim(d.title) {
            "" -> #(model, effect.none())
            title -> {
              let rid = case model.new_task_request {
                "" -> request_id()
                rid -> rid
              }
              #(
                Model(
                  ..model,
                  task_saving: Some(""),
                  new_task_request: rid,
                  new_task_error: "",
                ),
                effect.from(fn(send) {
                  request(
                    model.prefix,
                    base_of(model, d.project) <> "/tasks",
                    "POST",
                    json.to_string(
                      json.object([
                        #("title", json.string(title)),
                        #("body", json.string(d.body)),
                        #("request", json.string(rid)),
                      ]),
                    ),
                    fn(reply) { send(TaskCreated(d.project, "", reply)) },
                  )
                }),
              )
            }
          }
        _, _ -> #(model, effect.none())
      })
    // The Tasks tab's create: the links picked go in one `Links`, whose
    // `Patched` reports in the form; with none the task opens now.
    TaskCreated(dir, "", text) ->
      Some(case json.parse(text, row_decoder()) {
        Ok(row) -> {
          let model = Model(..put(model, dir, row), new_task_request: "")
          let fields = case model.new_task {
            Some(d) -> new_task_links(d)
            None -> []
          }
          case fields {
            [] -> #(new_task_done(model, dir, row.id), effect.none())
            _ -> #(
              Model(..model, task_linking: Some(#(dir, row.id, ""))),
              effect.from(fn(send) { send(Links(dir, row.id, fields)) }),
            )
          }
        }
        Error(_) -> #(
          Model(
            ..model,
            task_saving: None,
            new_task_error: "Couldn't create the task: " <> error(text),
          ),
          effect.none(),
        )
      })
    TaskCreated(dir, feature, text) ->
      Some(case json.parse(text, row_decoder()) {
        Ok(row) -> #(
          Model(
            ..put(model, dir, row),
            task_titles: drop(model.task_titles, feature),
            task_requests: drop(model.task_requests, feature),
            task_errors: drop(model.task_errors, feature),
            task_linking: Some(#(dir, row.id, feature)),
          ),
          effect.from(fn(send) { send(Link(dir, row.id, "feature", feature)) }),
        )
        Error(_) -> #(
          Model(
            ..model,
            task_saving: None,
            task_errors: set(
              model.task_errors,
              feature,
              "Couldn't create the task: " <> error(text),
            ),
          ),
          effect.none(),
        )
      })
    // Refused during a run (D-100, E7003), unlike the links.
    SetDepends(dir, id, ids) ->
      Some(#(
        Model(..model, milestone_error: ""),
        effect.from(fn(send) {
          request(
            model.prefix,
            base_of(model, dir) <> "/tasks/" <> id,
            "PATCH",
            json.to_string(roadmap.depends_body(ids)),
            fn(reply) { send(DependsPatched(dir, reply)) },
          )
        }),
      ))
    DependsPatched(dir, text) ->
      Some(case json.parse(text, row_decoder()) {
        Ok(row) -> #(
          Model(..put(model, dir, row), milestone_error: "", depends_query: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      })
    OpenDepends(dir, id) ->
      Some(#(
        Model(
          ..model,
          depends_open: case model.depends_open == Some(#(dir, id)) {
            True -> None
            False -> Some(#(dir, id))
          },
          depends_query: "",
          milestone_error: "",
        ),
        effect.none(),
      ))
    DependsQuery(query) ->
      Some(#(Model(..model, depends_query: query), effect.none()))
    _ -> None
  }
}

fn plan_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    PlanCollapse ->
      Some(#(
        Model(..model, plan_collapsed: !model.plan_collapsed),
        effect.none(),
      ))
    PlanDrag(x) ->
      Some(#(
        model,
        effect.from(fn(send) {
          drag(x, model.plan_width, fn(w) { send(PlanWidth(w)) }, fn() {
            send(PlanWidthDone)
          })
        }),
      ))
    PlanWidth(width) ->
      Some(#(
        Model(..model, plan_width: plan_panel.clamp(width, window_width())),
        effect.none(),
      ))
    PlanWidthDone -> Some(#(model, store_width(model.plan_width)))
    // The handle is the panel's left edge: Left widens it.
    PlanKey(key) -> {
      let window = window_width()
      let width =
        case key {
          "ArrowLeft" -> model.plan_width + plan_panel.key_step
          "ArrowRight" -> model.plan_width - plan_panel.key_step
          "Home" -> plan_panel.min_width
          "End" -> plan_panel.max_width(window)
          _ -> model.plan_width
        }
        |> plan_panel.clamp(window)
      Some(#(Model(..model, plan_width: width), store_width(width)))
    }
    // The message box grows with its text by itself (`<solace-md-editor>`).
    PlanText(text) -> {
      plan_panel.save_draft(plan_setup.settings(), text)
      Some(#(Model(..model, plan_text: text), effect.none()))
    }
    PlanFormat(before, after) ->
      Some(#(
        model,
        effect.from(fn(send) {
          send(
            PlanText(solace_task.wrap(
              model.root,
              plan_setup.settings().text_id,
              model.plan_text,
              before,
              after,
            )),
          )
        }),
      ))
    PlanBlock(kind) ->
      Some(#(
        model,
        effect.from(fn(send) {
          send(
            PlanText(solace_task.block(
              model.root,
              plan_setup.settings().text_id,
              model.plan_text,
              kind,
            )),
          )
        }),
      ))
    // Efforts differ between models, so a new model starts at its default.
    PlanModel(id) ->
      Some(#(
        Model(
          ..model,
          plan_model: id,
          plan_effort: "",
          plan_source: solace_task.Human,
        ),
        effect.none(),
      ))
    PlanEffort(effort) ->
      Some(#(Model(..model, plan_effort: effort), effect.none()))
    // The model picked stays while it is available; without the list the
    // server takes its default.
    TryAgainModel if model.plan_refreshing -> Some(#(model, effect.none()))
    TryAgainModel ->
      Some(#(
        Model(..model, plan_refreshing: True),
        effect.from(fn(send) {
          request(
            model.prefix,
            "/models/try-again",
            "POST",
            json.to_string(json.object([])),
            fn(text) { send(ModelsLoaded(text)) },
          )
        }),
      ))
    ModelsLoaded(text) ->
      Some(case json.parse(text, decode.list(solace_task.agent_decoder())) {
        Ok(models) -> {
          let model = Model(..model, models: models, plan_refreshing: False)
          let model = case model.plan_source {
            solace_task.Human ->
              case
                list.find(models, fn(a) {
                  a.id == model.plan_model && a.available
                })
              {
                Ok(a) ->
                  Model(
                    ..model,
                    plan_effort: solace_task.kept_effort(a, model.plan_effort),
                  )
                Error(_) ->
                  choose_plan_model(
                    Model(..model, plan_source: solace_task.RoleDefault),
                  )
              }
            solace_task.RoleDefault -> choose_plan_model(model)
          }
          #(model, effect.none())
        }
        Error(_) -> #(Model(..model, plan_refreshing: False), effect.none())
      })
    ModelsRefreshed(text) ->
      Some(case json.parse(text, decode.list(solace_task.agent_decoder())) {
        Ok(models) -> {
          let model = case
            list.find(models, fn(a) { a.id == model.plan_model && a.available })
          {
            Ok(a) ->
              Model(
                ..model,
                models: models,
                plan_effort: solace_task.kept_effort(a, model.plan_effort),
              )
            Error(_) ->
              choose_plan_model(
                Model(
                  ..model,
                  models: models,
                  plan_source: solace_task.RoleDefault,
                ),
              )
          }
          #(model, effect.none())
        }
        Error(_) -> #(model, effect.none())
      })
    RoleModelsLoaded(text) ->
      Some(case json.parse(text, solace_task.role_models_decoder()) {
        Ok(role_models) -> #(
          choose_plan_model(Model(..model, role_models: role_models)),
          effect.none(),
        )
        Error(_) -> #(model, effect.none())
      })
    StartPlan ->
      Some(case string.trim(model.plan_text) {
        "" -> #(model, effect.none())
        _ ->
          plan_send(
            model,
            plan_panel.runs_path(plan_setup.settings()),
            plan_panel.start_body(
              model.plan_model,
              model.plan_effort,
              model.plan_text,
              plan_panel.page,
            ),
            True,
          )
      })
    SendPlan ->
      Some(case plan_shown(model), string.trim(model.plan_text) {
        Some(run), text if text != "" && run.state != "ended" ->
          plan_send(
            model,
            plan_panel.run_path(plan_setup.settings(), run.id) <> "/notes",
            json.object([#("text", json.string(model.plan_text))]),
            True,
          )
        _, _ -> #(model, effect.none())
      })
    AnswerPlan(qid, option) ->
      Some(case plan_shown(model) {
        Some(run) ->
          plan_send(
            model,
            plan_panel.run_path(plan_setup.settings(), run.id)
              <> "/questions/"
              <> qid,
            json.object([#("option", json.string(option))]),
            False,
          )
        None -> #(model, effect.none())
      })
    EndPlan | StopPlan ->
      Some(case plan_shown(model) {
        Some(run) ->
          plan_send(
            model,
            plan_panel.run_path(plan_setup.settings(), run.id)
              <> case msg {
              EndPlan -> "/end"
              _ -> "/stop"
            },
            json.object([]),
            False,
          )
        None -> #(model, effect.none())
      })
    ShowPlan(rid) -> {
      let model =
        Model(
          ..model,
          plan_open: rid,
          plan_entries: [],
          plan_error: "",
          plan_collapsed: False,
          plan_at_bottom: True,
          plan_new: False,
          plan_jump: rid != "",
        )
      Some(case rid {
        "" -> #(Model(..model, plan_poll: model.plan_poll + 1), effect.none())
        _ -> plan_fetch(model, rid)
      })
    }
    PlanScrolled(near) ->
      Some(#(
        Model(..model, plan_at_bottom: near, plan_new: model.plan_new && !near),
        effect.none(),
      ))
    PlanNewest ->
      Some(#(
        Model(..model, plan_at_bottom: True, plan_new: False),
        plan_scroll(),
      ))
    _ -> None
  }
}

fn plan_reply_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  case msg {
    // The reply's run is shown, and its transcript fetched again.
    PlanReplied(clears, text) ->
      Some(case json.parse(text, plan_panel.decoder()) {
        Ok(run) -> {
          let sent_text = case model.plan_clearing {
            Some(sent_text) -> sent_text
            None -> ""
          }
          let clear_text = clears && model.plan_text == sent_text
          case clear_text {
            True -> plan_panel.clear_draft(plan_setup.settings())
            False -> Nil
          }
          plan_track(
            model,
            plan_fetch(
              Model(
                ..model,
                runs: keep(model.runs, run, fn(r) { r.id }),
                plan_open: run.id,
                plan_sending: False,
                plan_error: "",
                plan_text: case clear_text {
                  True -> ""
                  False -> model.plan_text
                },
                plan_clearing: None,
                plan_jump: model.plan_jump || run.id != model.plan_open,
              ),
              run.id,
            ),
          )
        }
        Error(_) -> #(
          Model(
            ..model,
            plan_sending: False,
            plan_error: error(text),
            plan_clearing: None,
          ),
          effect.none(),
        )
      })
    // While the agent works its transcript is fetched again, in one chain.
    PlanLoaded(poll, rid, text)
      if poll == model.plan_poll && rid == model.plan_open
    -> {
      let decoder = {
        use run <- decode.then(plan_panel.decoder())
        use entries <- decode.field(
          "transcript",
          decode.list(solace_task.entry_decoder()),
        )
        decode.success(#(run, entries))
      }
      Some(case json.parse(text, decoder) {
        Ok(#(run, entries)) -> {
          let next =
            Model(
              ..model,
              runs: keep(model.runs, run, fn(r) { r.id }),
              plan_entries: entries,
              plan_jump: False,
            )
          let poll_fx = case run.state == "running" && model.connected {
            True ->
              effect.from(fn(send) {
                after(refresh_ms, fn() { send(PlanRefresh(poll, rid)) })
              })
            False -> effect.none()
          }
          // An opened run starts at its newest message.
          case model.plan_jump {
            True -> #(
              Model(..next, plan_at_bottom: True, plan_new: False),
              effect.batch([poll_fx, plan_scroll()]),
            )
            False -> plan_track(model, #(next, poll_fx))
          }
        }
        Error(_) -> #(Model(..model, plan_error: error(text)), effect.none())
      })
    }
    PlanLoaded(_, _, _) -> Some(#(model, effect.none()))
    PlanRefresh(poll, rid)
      if poll == model.plan_poll && rid == model.plan_open
    -> Some(#(model, plan_get(model, poll, rid)))
    PlanRefresh(_, _) -> Some(#(model, effect.none()))
    _ -> None
  }
}

/// The query's lowercased words, without empty ones from repeated or
/// surrounding spaces.
fn query_words(query: String) -> List(String) {
  query
  |> string.lowercase
  |> string.split(" ")
  |> list.filter(fn(word) { word != "" })
}

/// The default order, then by how well each row matches the query. The
/// default order stays within each search group.
fn by_search(
  query: String,
  rows: List(#(String, Row)),
) -> List(#(String, Row)) {
  let query = string.trim(query) |> string.lowercase
  let rows = sorted(rows)
  case query {
    "" -> rows
    _ ->
      rows
      |> list.map(fn(row) { #(search_rank(query, row.1), row) })
      |> list.sort(fn(a, b) { int.compare(a.0, b.0) })
      |> list.map(fn(ranked) { ranked.1 })
  }
}

/// A row's search group: 0 for the id or the whole phrase in the title, 1
/// for every word in the title, 2 for the rest. `query` is trimmed and
/// lowercased.
fn search_rank(query: String, row: Row) -> Int {
  let title = string.lowercase(row.title)
  let named = query == row.id || query == "task:" <> row.id
  case named || string.contains(title, query) {
    True -> 0
    False ->
      case
        list.all(query_words(query), fn(word) { string.contains(title, word) })
      {
        True -> 1
        False -> 2
      }
  }
}

/// The Personas tab: the form while one is open, and the saved personas.
fn personas_view(model: Model) -> List(Element(Msg)) {
  personas.view(
    model.prefix,
    model.persona_state.personas,
    persona_editor(model),
  )
}

fn persona_editor(model: Model) -> personas.Editor(Msg) {
  personas.Editor(
    draft: model.persona_state.draft,
    saving: model.persona_state.saving,
    error: model.persona_state.error,
    on_new: NewPersona,
    on_edit: EditPersona,
    on_cancel: CancelPersona,
    on_name: PersonaName,
    on_text: PersonaText,
    on_format: PersonaFormat,
    on_block: PersonaBlock,
    on_save: SavePersona,
    on_reload: ReloadPersona,
    deleting: personas.confirming(model.persona_state),
    on_delete: DeletePersona,
    on_confirm_delete: ConfirmDeletePersona,
    on_cancel_delete: CancelDeletePersona,
  )
}

fn persona_events(model: Model, msg: Msg) -> Option(#(Model, Effect(Msg))) {
  let change = case msg {
    NewPersona -> Ok(personas.New)
    EditPersona(id) -> Ok(personas.Edit(id))
    CancelPersona -> Ok(personas.Cancel)
    ReloadPersona -> Ok(personas.Reload)
    PersonaName(name) -> Ok(personas.Name(name))
    PersonaText(field, text) -> Ok(personas.Text(field, text))
    SavePersona -> Ok(personas.Save)
    PersonaSaved(text) -> Ok(personas.Saved(persona_reply(text)))
    DeletePersona(id) -> Ok(personas.Delete(id))
    CancelDeletePersona -> Ok(personas.CancelDelete)
    ConfirmDeletePersona -> Ok(personas.ConfirmDelete)
    PersonaDeleted(text) -> Ok(personas.Deleted(persona_reply(text)))
    _ -> Error(Nil)
  }
  case change {
    Error(_) -> None
    Ok(change) -> {
      let #(state, request) = personas.update(model.persona_state, change)
      let fx = case request {
        personas.Idle -> effect.none()
        personas.SaveRequest(draft) -> persona_save_effect(model, draft)
        personas.DeleteRequest(id) -> persona_delete_effect(model, id)
      }
      Some(#(Model(..model, persona_state: state), fx))
    }
  }
}

const persona_styles =
  ".persona{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card);margin:0 0 var(--space-3)}
.persona h3{display:flex;flex-wrap:wrap;align-items:center;gap:var(--space-2);margin:0 0 var(--space-2);font-size:var(--h4)}
.persona h3 button{"
  <> solace_task.small_control
  <> ";padding:0 var(--space-2);margin-left:var(--space-1)}.persona h3 button:hover{background:var(--hover)}
.persona h4{margin:var(--space-2) 0 var(--space-1);font-size:var(--h4)}
.persona-field{margin:var(--space-2) 0 0}
.persona-form .composer solace-md-editor{--md-min-height:180px}"

fn persona_format_events(
  model: Model,
  msg: Msg,
) -> Option(#(Model, Effect(Msg))) {
  case msg {
    PersonaFormat(field, before, after) ->
      Some(
        persona_format_effect(model, field, fn(text) {
          solace_task.wrap(
            model.root,
            personas.field_id(field),
            text,
            before,
            after,
          )
        }),
      )
    PersonaBlock(field, kind) ->
      Some(
        persona_format_effect(model, field, fn(text) {
          solace_task.block(model.root, personas.field_id(field), text, kind)
        }),
      )
    _ -> None
  }
}

fn persona_format_effect(
  model: Model,
  field: personas.Field,
  change: fn(String) -> String,
) -> #(Model, Effect(Msg)) {
  case model.persona_state.draft {
    Some(draft) -> #(
      model,
      effect.from(fn(send) {
        send(PersonaText(field, change(personas.field_text(draft, field))))
      }),
    )
    None -> #(model, effect.none())
  }
}

fn persona_reply(text: String) -> Result(personas.Persona, String) {
  case json.parse(text, personas.decoder()) {
    Ok(persona) -> Ok(persona)
    Error(_) -> Error(error(text))
  }
}

fn persona_save_effect(model: Model, draft: personas.Draft) -> Effect(Msg) {
  let #(path, method, body) = case draft.id {
    None -> #("/personas", "POST", personas.create_body(draft))
    Some(id) -> #("/personas/" <> id, "PUT", personas.replace_body(draft))
  }
  effect.from(fn(send) {
    request(model.prefix, path, method, json.to_string(body), fn(reply) {
      send(PersonaSaved(reply))
    })
  })
}

fn persona_delete_effect(model: Model, id: String) -> Effect(Msg) {
  effect.from(fn(send) {
    request(model.prefix, "/personas/" <> id, "DELETE", "", fn(reply) {
      send(PersonaDeleted(reply))
    })
  })
}

fn choose_plan_model(model: Model) -> Model {
  case model.plan_source {
    solace_task.Human -> model
    solace_task.RoleDefault -> {
      let id =
        solace_task.role_model_chosen(
          model.models,
          model.role_models,
          "strategist",
        )
      let effort = case id == model.plan_model {
        True -> solace_task.effort_for(model.models, id, model.plan_effort)
        False -> ""
      }
      Model(
        ..model,
        plan_model: id,
        plan_effort: effort,
        plan_source: solace_task.RoleDefault,
      )
    }
  }
}
