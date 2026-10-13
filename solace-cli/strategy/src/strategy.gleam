import features
import gleam/dynamic
import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import plan
import roadmap
import solace_task

/// Registers the element on its own, for `index.html`.
pub fn main() {
  let assert Ok(_) = register()
}

pub type Tab {
  Vision
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
    /// The task open below the list: the project it is stored in, and its id.
    selected: Option(#(String, String)),
    error: String,
    source: Option(Source),
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
    preview: Bool,
    /// A save sent and not yet answered.
    saving: Bool,
    vision_error: String,
    /// The feature being written on the Features tab, None when no form is open.
    feature_draft: Option(features.Draft),
    feature_preview: Bool,
    /// A feature save sent and not yet answered.
    feature_saving: Bool,
    feature_error: String,
    /// The `request` id of the open new feature's create: a repeated save of
    /// the same draft makes one feature (Spec 15).
    feature_request: String,
    /// The feature whose delete is being confirmed.
    feature_deleting: Option(String),
    /// The milestone being written on the Roadmap tab, None when no form is open.
    milestone_draft: Option(roadmap.Draft),
    milestone_preview: Bool,
    /// A milestone save sent and not yet answered.
    milestone_saving: Bool,
    /// The Roadmap tab's refusal: a save, move or delete, a feature's target
    /// or a task's dependencies.
    milestone_error: String,
    /// The `request` id of the open new milestone's create (Spec 15).
    milestone_request: String,
    /// The milestone whose delete is being confirmed.
    milestone_deleting: Option(String),
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
    /// The root's strategy runs (D-93), oldest first.
    runs: List(plan.Run),
    /// The run shown in the planning panel, "" for none.
    plan_open: String,
    plan_collapsed: Bool,
    /// The planning panel's width in pixels, kept in the browser.
    plan_width: Int,
    /// The panel's message box: a new run's first message, or the next one.
    plan_text: String,
    /// Whether the message box shows its Preview.
    plan_preview: Bool,
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
    plan_model: String,
    plan_effort: String,
    /// The shadow root, once it has rendered, for the editor's tools.
    root: dynamic.Dynamic,
    /// The task a task link asked to show (D-98), and how many times one was
    /// asked, so asking again is news to `<solace-task>`.
    reveal: #(String, Int),
  )
}

pub type Msg {
  Select(Tab)
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
  /// `<solace-task>` says the open task now lives in another project.
  TaskMoved(String, String)
  Filter(Filters)
  /// An inline link: the task's project and id, "feature" or "milestone",
  /// and the record's id, "" for none.
  Link(String, String, String, String)
  /// The reply to a link: the task's project, and the task as it is now.
  Patched(String, String)
  BaseChanged(String)
  Listening(Source)
  Connected
  Disconnected
  /// Begins editing the vision as it is now.
  EditVision
  CancelVision
  Draft(String)
  VisionPreview(Bool)
  VisionFormat(String, String)
  VisionBlock(String)
  SaveVision
  /// The reply to a save: the vision as it is now, or a refusal.
  VisionSaved(String)
  /// Drops the draft and edits the vision as it is now.
  ReloadVision
  /// Opens the form for a new feature.
  NewFeature
  /// Opens the form on the feature with this id.
  EditFeature(String)
  CancelFeature
  FeatureTitle(String)
  FeatureText(String)
  FeatureState(String)
  /// The milestone's id, "" for none.
  FeatureMilestone(String)
  FeaturePreview(Bool)
  FeatureFormat(String, String)
  FeatureBlock(String)
  SaveFeature
  /// The reply to a feature save: the feature as it is now, or a refusal.
  FeatureSaved(String)
  /// Moves the feature with this id to this rank: one PATCH.
  MoveFeature(String, Float)
  /// The reply to a move: the feature, or a refusal.
  FeatureMoved(String)
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
  /// Opens the form for a new milestone.
  NewMilestone
  /// Opens the form on the milestone with this id.
  EditMilestone(String)
  CancelMilestone
  MilestoneTitle(String)
  MilestoneText(String)
  /// The target date as the input gives it, "" for none.
  MilestoneTargetDate(String)
  MilestoneState(String)
  MilestonePreview(Bool)
  MilestoneFormat(String, String)
  MilestoneBlock(String)
  SaveMilestone
  /// The reply to a milestone save: the milestone as it is now, or a refusal.
  MilestoneSaved(String)
  /// Moves the milestone with this id to this rank: one PATCH.
  MoveMilestone(String, Float)
  MilestoneMoved(String)
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
  PlanPreview(Bool)
  PlanFormat(String, String)
  PlanBlock(String)
  PlanModel(String)
  PlanEffort(String)
  /// The reply to `GET /models`.
  ModelsLoaded(String)
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

@external(javascript, "./api.mjs", "set_tab")
fn set_tab(name: String) -> Nil

pub fn tabs() -> List(Tab) {
  [Vision, Features, Roadmap, Tasks]
}

/// The tab's name in the URL: `/strategy?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    Vision -> "vision"
    Features -> "features"
    Roadmap -> "roadmap"
    Tasks -> "tasks"
  }
}

pub fn label(tab: Tab) -> String {
  case tab {
    Vision -> "Vision"
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
  #(
    Model(..new(tab_from(tab_query())), plan_width: stored_width()),
    effect.none(),
  )
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Select(tab) -> #(
      Model(..model, tab: tab),
      effect.from(fn(_) { set_tab(tab_name(tab)) }),
    )
    Connected -> {
      let #(model, fx) = start(Model(..model, connected: True))
      let #(model, plan_fx) = case model.plan_open {
        "" -> #(model, effect.none())
        rid -> plan_fetch(model, rid)
      }
      #(
        model,
        effect.batch([
          fx,
          plan_fx,
          effect.before_paint(fn(send, root) { send(Attached(root)) }),
        ]),
      )
    }
    Attached(root) -> #(Model(..model, root: root), effect.none())
    EditVision -> #(
      Model(
        ..model,
        draft: Some(#(model.vision.text, model.vision.revision)),
        preview: False,
        vision_error: "",
      ),
      effect.none(),
    )
    ReloadVision -> #(
      Model(
        ..model,
        draft: Some(#(model.vision.text, model.vision.revision)),
        vision_error: "",
      ),
      effect.none(),
    )
    CancelVision -> #(
      Model(..model, draft: None, vision_error: ""),
      effect.none(),
    )
    Draft(text) ->
      case model.draft {
        Some(#(_, base)) -> #(
          Model(..model, draft: Some(#(text, base))),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    VisionPreview(on) -> #(Model(..model, preview: on), effect.none())
    VisionFormat(before, after) ->
      case model.draft {
        Some(#(text, _)) -> #(
          model,
          effect.from(fn(send) {
            send(
              Draft(solace_task.wrap(model.root, vision_id, text, before, after)),
            )
          }),
        )
        None -> #(model, effect.none())
      }
    VisionBlock(kind) ->
      case model.draft {
        Some(#(text, _)) -> #(
          model,
          effect.from(fn(send) {
            send(Draft(solace_task.block(model.root, vision_id, text, kind)))
          }),
        )
        None -> #(model, effect.none())
      }
    // One save at a time, within the limit (D-92); its reply ends the edit or
    // says why not.
    SaveVision -> {
      let fits = case model.draft {
        Some(#(text, _)) -> vision_count(text) <= vision_limit
        None -> False
      }
      case model.draft, model.saving, fits {
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
      }
    }
    VisionSaved(text) ->
      case json.parse(text, vision_decoder()) {
        Ok(v) -> #(
          Model(
            ..model,
            vision: v,
            draft: None,
            preview: False,
            saving: False,
            vision_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, saving: False, vision_error: error(text)),
          effect.none(),
        )
      }
    Disconnected -> #(
      Model(..stop(model), connected: False, plan_poll: model.plan_poll + 1),
      effect.none(),
    )
    NewFeature if !model.feature_saving -> #(
      Model(
        ..model,
        feature_draft: Some(features.new_draft()),
        feature_preview: False,
        feature_error: "",
        feature_request: request_id(),
      ),
      effect.none(),
    )
    EditFeature(id) if !model.feature_saving ->
      case list.find(model.features, fn(f) { f.id == id }) {
        Ok(f) -> #(
          Model(
            ..model,
            feature_draft: Some(features.draft_of(f)),
            feature_preview: False,
            feature_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(model, effect.none())
      }
    NewFeature | EditFeature(_) -> #(model, effect.none())
    CancelFeature -> #(
      Model(..model, feature_draft: None, feature_error: ""),
      effect.none(),
    )
    FeatureTitle(title) ->
      edit_feature(model, fn(d) { features.Draft(..d, title: title) })
    FeatureText(body) ->
      edit_feature(model, fn(d) { features.Draft(..d, body: body) })
    FeatureState(state) ->
      edit_feature(model, fn(d) { features.Draft(..d, state: state) })
    FeatureMilestone(id) ->
      edit_feature(model, fn(d) {
        features.Draft(..d, milestone: case id {
          "" -> None
          id -> Some(id)
        })
      })
    FeaturePreview(on) -> #(Model(..model, feature_preview: on), effect.none())
    FeatureFormat(before, after) ->
      case model.feature_draft {
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
      }
    FeatureBlock(kind) ->
      case model.feature_draft {
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
      }
    // One save at a time. A create names its `request`; an edit sends only
    // what changed, and an edit that changed nothing just closes.
    SaveFeature ->
      case model.feature_draft, model.feature_saving {
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
                [] -> #(
                  Model(..model, feature_draft: None, feature_error: ""),
                  effect.none(),
                )
                fields -> save("/features/" <> id, "PATCH", json.object(fields))
              }
            }
          }
        }
        _, _ -> #(model, effect.none())
      }
    FeatureSaved(text) ->
      case json.parse(text, features.decoder()) {
        Ok(f) -> #(
          Model(
            ..model,
            features: keep(model.features, f, fn(f) { f.id }),
            feature_draft: None,
            feature_preview: False,
            feature_saving: False,
            feature_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, feature_saving: False, feature_error: error(text)),
          effect.none(),
        )
      }
    // An attribute set before the element is added only takes effect then.
    MoveFeature(id, rank) -> #(
      Model(..model, feature_error: ""),
      effect.from(fn(send) {
        request(
          model.prefix,
          "/features/" <> id,
          "PATCH",
          json.to_string(json.object([#("rank", json.float(rank))])),
          fn(reply) { send(FeatureMoved(reply)) },
        )
      }),
    )
    // A reply after the feature was removed doesn't bring it back.
    FeatureMoved(text) ->
      case json.parse(text, features.decoder()) {
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
      }
    // The `{removedFeature}` and `{task}` events update the page.
    FeatureDeleted(text) ->
      case json.parse(text, features.decoder()) {
        Ok(_) -> #(
          Model(..model, feature_deleting: None, feature_error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, feature_error: error(text)), effect.none())
      }
    DeleteFeature(id) if model.feature_draft == None -> #(
      Model(..model, feature_deleting: Some(id), feature_error: ""),
      effect.none(),
    )
    DeleteFeature(_) -> #(model, effect.none())
    CancelDeleteFeature -> #(
      Model(..model, feature_deleting: None, feature_error: ""),
      effect.none(),
    )
    ConfirmDeleteFeature ->
      case model.feature_deleting {
        Some(id) -> #(
          model,
          effect.from(fn(send) {
            request(model.prefix, "/features/" <> id, "DELETE", "", fn(reply) {
              send(FeatureDeleted(reply))
            })
          }),
        )
        None -> #(model, effect.none())
      }
    TaskTitle(feature, title) -> #(
      Model(..model, task_titles: set(model.task_titles, feature, title)),
      effect.none(),
    )
    TaskProject(dir) -> #(Model(..model, task_project: dir), effect.none())
    NewMilestone if !model.milestone_saving -> #(
      Model(
        ..model,
        milestone_draft: Some(roadmap.new_draft()),
        milestone_preview: False,
        milestone_error: "",
        milestone_request: request_id(),
      ),
      effect.none(),
    )
    EditMilestone(id) if !model.milestone_saving ->
      case list.find(model.milestones, fn(m) { m.id == id }) {
        Ok(m) -> #(
          Model(
            ..model,
            milestone_draft: Some(roadmap.draft_of(m)),
            milestone_preview: False,
            milestone_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(model, effect.none())
      }
    NewMilestone | EditMilestone(_) -> #(model, effect.none())
    CancelMilestone -> #(
      Model(..model, milestone_draft: None, milestone_error: ""),
      effect.none(),
    )
    MilestoneTitle(title) ->
      edit_milestone(model, fn(d) { roadmap.Draft(..d, title: title) })
    MilestoneText(body) ->
      edit_milestone(model, fn(d) { roadmap.Draft(..d, body: body) })
    MilestoneState(state) ->
      edit_milestone(model, fn(d) { roadmap.Draft(..d, state: state) })
    MilestoneTargetDate(date) ->
      edit_milestone(model, fn(d) {
        roadmap.Draft(..d, target_date: case date {
          "" -> None
          date -> Some(date)
        })
      })
    MilestonePreview(on) -> #(
      Model(..model, milestone_preview: on),
      effect.none(),
    )
    MilestoneFormat(before, after) ->
      case model.milestone_draft {
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
      }
    MilestoneBlock(kind) ->
      case model.milestone_draft {
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
      }
    // As SaveFeature: one at a time, a create names its `request`, an edit
    // sends only what changed.
    SaveMilestone ->
      case model.milestone_draft, model.milestone_saving {
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
                [] -> #(
                  Model(..model, milestone_draft: None, milestone_error: ""),
                  effect.none(),
                )
                fields ->
                  save("/milestones/" <> id, "PATCH", json.object(fields))
              }
            }
          }
        }
        _, _ -> #(model, effect.none())
      }
    MilestoneSaved(text) ->
      case json.parse(text, features.milestone_decoder()) {
        Ok(m) -> #(
          Model(
            ..model,
            milestones: keep(model.milestones, m, fn(m) { m.id }),
            milestone_draft: None,
            milestone_preview: False,
            milestone_saving: False,
            milestone_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_saving: False, milestone_error: error(text)),
          effect.none(),
        )
      }
    MoveMilestone(id, rank) -> #(
      Model(..model, milestone_error: ""),
      effect.from(fn(send) {
        request(
          model.prefix,
          "/milestones/" <> id,
          "PATCH",
          json.to_string(json.object([#("rank", json.float(rank))])),
          fn(reply) { send(MilestoneMoved(reply)) },
        )
      }),
    )
    // A reply after the milestone was removed doesn't bring it back.
    MilestoneMoved(text) ->
      case json.parse(text, features.milestone_decoder()) {
        Ok(m) -> #(
          Model(
            ..model,
            milestones: case
              list.any(model.milestones, fn(x) { x.id == m.id })
            {
              True -> keep(model.milestones, m, fn(m) { m.id })
              False -> model.milestones
            },
            milestone_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      }
    DeleteMilestone(id) if model.milestone_draft == None -> #(
      Model(..model, milestone_deleting: Some(id), milestone_error: ""),
      effect.none(),
    )
    DeleteMilestone(_) -> #(model, effect.none())
    CancelDeleteMilestone -> #(
      Model(..model, milestone_deleting: None, milestone_error: ""),
      effect.none(),
    )
    ConfirmDeleteMilestone ->
      case model.milestone_deleting {
        Some(id) -> #(
          model,
          effect.from(fn(send) {
            request(model.prefix, "/milestones/" <> id, "DELETE", "", fn(reply) {
              send(MilestoneDeleted(reply))
            })
          }),
        )
        None -> #(model, effect.none())
      }
    // The `{removedMilestone}`, `{feature}` and `{task}` events update the page.
    MilestoneDeleted(text) ->
      case json.parse(text, features.milestone_decoder()) {
        Ok(_) -> #(
          Model(..model, milestone_deleting: None, milestone_error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      }
    TargetFeature(id, milestone) -> #(
      Model(..model, milestone_error: ""),
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
    )
    FeatureTargeted(text) ->
      case json.parse(text, features.decoder()) {
        Ok(f) -> #(
          Model(
            ..model,
            features: case list.any(model.features, fn(x) { x.id == f.id }) {
              True -> keep(model.features, f, fn(f) { f.id })
              False -> model.features
            },
            milestone_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      }
    // Refused during a run (D-100, E7003), unlike the links.
    SetDepends(dir, id, ids) -> #(
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
    )
    DependsPatched(dir, text) ->
      case json.parse(text, row_decoder()) {
        Ok(row) -> #(
          Model(..put(model, dir, row), milestone_error: "", depends_query: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, milestone_error: error(text)),
          effect.none(),
        )
      }
    OpenDepends(dir, id) -> #(
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
    )
    DependsQuery(query) -> #(
      Model(..model, depends_query: query),
      effect.none(),
    )
    // One create at a time. Then the link is one `Link`, whose `Patched`
    // reports a refusal on the feature.
    AddTask(feature) -> {
      let title =
        list.key_find(model.task_titles, feature)
        |> result.unwrap("")
        |> string.trim
      case title, model.task_saving {
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
      }
    }
    TaskCreated(dir, feature, text) ->
      case json.parse(text, row_decoder()) {
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
      }
    BaseChanged(base) -> {
      let model = Model(..model, prefix: base)
      case model.connected {
        True -> start(model)
        False -> #(model, effect.none())
      }
    }
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    ProjectsLoaded(text) ->
      case json.parse(text, projects_decoder()) {
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
      }
    WorkflowLoaded(dir, text) ->
      case json.parse(text, solace_task.workflow_decoder()) {
        Ok(w) -> #(
          Model(..model, workflows: set(model.workflows, dir, w)),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: error(text)), effect.none())
      }
    // What a new connection missed is replayed, then applied at once.
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> {
      let fx = case projects_event(text) {
        True -> get(model, "/projects", ProjectsLoaded)
        False -> effect.none()
      }
      case knows(model, text) {
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
      }
    }
    CaughtUp -> #(
      resync(settle(Model(..model, caught_up: True))),
      effect.none(),
    )
    PlanCollapse -> #(
      Model(..model, plan_collapsed: !model.plan_collapsed),
      effect.none(),
    )
    PlanDrag(x) -> #(
      model,
      effect.from(fn(send) {
        drag(x, model.plan_width, fn(w) { send(PlanWidth(w)) }, fn() {
          send(PlanWidthDone)
        })
      }),
    )
    PlanWidth(width) -> #(
      Model(..model, plan_width: plan.clamp(width, window_width())),
      effect.none(),
    )
    PlanWidthDone -> #(model, store_width(model.plan_width))
    // The handle is the panel's left edge: Left widens it.
    PlanKey(key) -> {
      let window = window_width()
      let width =
        case key {
          "ArrowLeft" -> model.plan_width + plan.key_step
          "ArrowRight" -> model.plan_width - plan.key_step
          "Home" -> plan.min_width
          "End" -> plan.max_width(window)
          _ -> model.plan_width
        }
        |> plan.clamp(window)
      #(Model(..model, plan_width: width), store_width(width))
    }
    PlanText(text) -> #(Model(..model, plan_text: text), plan_fit())
    PlanPreview(on) -> #(Model(..model, plan_preview: on), plan_fit())
    PlanFormat(before, after) -> #(
      model,
      effect.from(fn(send) {
        send(
          PlanText(solace_task.wrap(
            model.root,
            plan.text_id,
            model.plan_text,
            before,
            after,
          )),
        )
      }),
    )
    PlanBlock(kind) -> #(
      model,
      effect.from(fn(send) {
        send(
          PlanText(solace_task.block(
            model.root,
            plan.text_id,
            model.plan_text,
            kind,
          )),
        )
      }),
    )
    // Efforts differ between models, so a new model starts at its default.
    PlanModel(id) -> #(
      Model(..model, plan_model: id, plan_effort: ""),
      effect.none(),
    )
    PlanEffort(effort) -> #(Model(..model, plan_effort: effort), effect.none())
    // The model picked stays while it is available; without the list the
    // server takes its default.
    ModelsLoaded(text) ->
      case json.parse(text, decode.list(solace_task.agent_decoder())) {
        Ok(models) -> {
          let id = case
            list.find(models, fn(a) { a.available && a.id == model.plan_model })
          {
            Ok(a) -> a.id
            Error(_) ->
              list.find(models, fn(a) { a.available })
              |> result.map(fn(a) { a.id })
              |> result.unwrap("")
          }
          #(
            Model(
              ..model,
              models: models,
              plan_model: id,
              plan_effort: case id == model.plan_model {
                True -> model.plan_effort
                False -> ""
              },
            ),
            effect.none(),
          )
        }
        Error(_) -> #(model, effect.none())
      }
    StartPlan ->
      case string.trim(model.plan_text) {
        "" -> #(model, effect.none())
        _ ->
          plan_send(
            model,
            "/strategy/runs",
            plan.start_body(
              model.plan_model,
              model.plan_effort,
              model.plan_text,
              plan_context(model),
            ),
            True,
          )
      }
    SendPlan ->
      case plan_shown(model), string.trim(model.plan_text) {
        Some(run), text if text != "" && run.state != "ended" ->
          plan_send(
            model,
            "/strategy/runs/" <> run.id <> "/notes",
            json.object([#("text", json.string(model.plan_text))]),
            True,
          )
        _, _ -> #(model, effect.none())
      }
    AnswerPlan(qid, option) ->
      case plan_shown(model) {
        Some(run) ->
          plan_send(
            model,
            "/strategy/runs/" <> run.id <> "/questions/" <> qid,
            json.object([#("option", json.string(option))]),
            False,
          )
        None -> #(model, effect.none())
      }
    EndPlan | StopPlan ->
      case plan_shown(model) {
        Some(run) ->
          plan_send(
            model,
            "/strategy/runs/"
              <> run.id
              <> case msg {
              EndPlan -> "/end"
              _ -> "/stop"
            },
            json.object([]),
            False,
          )
        None -> #(model, effect.none())
      }
    // The reply's run is shown, and its transcript fetched again.
    PlanReplied(clears, text) ->
      case json.parse(text, plan.decoder()) {
        Ok(run) -> {
          let #(next, fx) =
            plan_track(
              model,
              plan_fetch(
                Model(
                  ..model,
                  runs: keep(model.runs, run, fn(r) { r.id }),
                  plan_open: run.id,
                  plan_sending: False,
                  plan_error: "",
                  plan_text: case clears {
                    True -> ""
                    False -> model.plan_text
                  },
                  plan_preview: model.plan_preview && !clears,
                  plan_jump: model.plan_jump || run.id != model.plan_open,
                ),
                run.id,
              ),
            )
          #(next, case clears {
            True -> effect.batch([fx, plan_fit()])
            False -> fx
          })
        }
        Error(_) -> #(
          Model(..model, plan_sending: False, plan_error: error(text)),
          effect.none(),
        )
      }
    ShowPlan(rid) -> {
      let model =
        Model(
          ..model,
          plan_open: rid,
          plan_entries: [],
          plan_error: "",
          plan_preview: False,
          plan_collapsed: False,
          plan_at_bottom: True,
          plan_new: False,
          plan_jump: rid != "",
        )
      case rid {
        "" -> #(Model(..model, plan_poll: model.plan_poll + 1), effect.none())
        _ -> plan_fetch(model, rid)
      }
    }
    // While the agent works its transcript is fetched again, in one chain.
    PlanLoaded(poll, rid, text)
      if poll == model.plan_poll && rid == model.plan_open
    -> {
      let decoder = {
        use run <- decode.then(plan.decoder())
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
      }
    }
    PlanLoaded(_, _, _) -> #(model, effect.none())
    PlanRefresh(poll, rid)
      if poll == model.plan_poll && rid == model.plan_open
    -> #(model, plan_get(model, poll, rid))
    PlanRefresh(_, _) -> #(model, effect.none())
    PlanScrolled(near) -> #(
      Model(..model, plan_at_bottom: near, plan_new: model.plan_new && !near),
      effect.none(),
    )
    PlanNewest -> #(
      Model(..model, plan_at_bottom: True, plan_new: False),
      plan_scroll(),
    )
    // From the Features tab the task shows on the Tasks tab.
    Open(dir, id) -> #(
      Model(..model, tab: Tasks, selected: Some(#(dir, id)), live: []),
      case model.tab {
        Tasks -> effect.none()
        _ -> effect.from(fn(_) { set_tab(tab_name(Tasks)) })
      },
    )
    Focus(_, _, "") -> #(model, effect.none())
    Focus("feature", _, id) -> show_record(model, Features, "feature", id)
    Focus("milestone", _, id) -> show_record(model, Roadmap, "milestone", id)
    // `<solace-task>` scrolls into view now, or once the task loads.
    Focus(_, dir, id) -> #(
      Model(
        ..model,
        tab: Tasks,
        selected: Some(#(dir, id)),
        live: case model.selected == Some(#(dir, id)) {
          True -> model.live
          False -> []
        },
        reveal: #(id, model.reveal.1 + 1),
      ),
      effect.from(fn(_) { set_tab(tab_name(Tasks)) }),
    )
    TaskMoved(dir, id) ->
      case model.selected {
        Some(#(_, shown)) if shown == id -> #(
          Model(..model, selected: Some(#(dir, id))),
          effect.none(),
        )
        _ -> #(model, effect.none())
      }
    Filter(filters) -> #(Model(..model, filters: filters), effect.none())
    // Allowed in final columns and during runs (D-87, D-100).
    Link(dir, id, field, value) -> #(
      model,
      effect.from(fn(send) {
        request(
          model.prefix,
          base_of(model, dir) <> "/tasks/" <> id,
          "PATCH",
          json.to_string(link_body(field, value)),
          fn(text) { send(Patched(dir, text)) },
        )
      }),
    )
    // The link after a new task's create reports on its feature.
    Patched(dir, text) ->
      case json.parse(text, row_decoder()), model.task_linking {
        Ok(row), Some(#(d, id, _)) if d == dir && row.id == id -> #(
          Model(..put(model, dir, row), task_saving: None, task_linking: None),
          effect.none(),
        )
        Ok(row), _ -> #(Model(..put(model, dir, row), error: ""), effect.none())
        Error(_), Some(#(d, _, feature)) if d == dir -> #(
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
      }
  }
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
        [
          html.h2([], [html.text(label(tab))]),
          ..case tab {
            Tasks -> tasks_view(model)
            Vision -> vision_view(model)
            Features -> features_view(model)
            Roadmap -> roadmap_view(model)
          }
        ],
      )
    })
  ])
}

/// The full light color set: `<solace-task>` sets none of its own, so its
/// colors come from here. The dark set is the page's (`index.html`, and the
/// shell's for `/strategy`).
const view_styles =
  ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec;--add:#dcefe2;--del:#f6dcdc;--sel:#bfd5ce;--frame:#e9f1ee99;--ok-text:#1a6b30;--warn-text:#8a4a0a;--shadow:#17222626;--grid:#dce7e1;--revision:#a32628;--entry:#23883a;--logic:#2f6db5;--effect:#b8691b;--test:#6a5ba6;--file:#8c8470;--area:#3d7f7a}
.strategy{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;height:calc(100dvh - 56px);overflow:hidden;background:var(--sheet);color:var(--ink);display:grid;grid-template-columns:minmax(0,1fr) auto;grid-template-rows:auto minmax(0,1fr)}
.strategy>.tabs{grid-column:1/-1}.strategy>[role=tabpanel]{grid-column:1;grid-row:2;min-width:0;min-height:0;overflow:auto}
@media (max-width:900px){.strategy{display:block;height:auto;overflow:visible}.strategy>[role=tabpanel]{overflow:visible}}
.tabs{display:flex;background:var(--sheet);border-bottom:2px solid var(--edge)}
.tabs button{font:inherit;color:inherit;border:0;border-right:1px solid var(--line);background:transparent;padding:10px 16px;cursor:pointer}
.tabs button:hover{background:var(--hover)}
.tabs button[aria-selected=true]{background:var(--paper);font-weight:700;box-shadow:inset 0 -3px var(--ink)}
.tabs button:focus-visible{outline:3px solid var(--focus);outline-offset:-3px}
[role=tabpanel]{padding:24px}
[role=tabpanel] h2{margin:0 0 10px;font-size:19px}
[role=tabpanel] p{color:var(--soft);margin:8px 0}
[role=alert]{color:var(--revision)}
.filters{display:flex;flex-wrap:wrap;gap:8px 16px;align-items:flex-end;margin:0 0 16px}
.filters label{display:flex;flex-direction:column;gap:4px;font-size:13px;color:var(--soft)}
.filters input,.filters select,.tasks select{font:inherit;font-size:15px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:6px 8px}
.filters input{min-width:240px}.filters input::placeholder{color:var(--soft);opacity:1}
.filters :focus-visible,.tasks :focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.tasks{width:100%;border-collapse:collapse;background:var(--paper);border:1px solid var(--edge)}
.tasks th{text-align:left;font-size:12px;text-transform:uppercase;letter-spacing:.08em;padding:8px 10px;border-bottom:2px solid var(--edge)}
.tasks td{padding:6px 10px;border-top:1px solid var(--line);vertical-align:middle}
.tasks tr:hover td{background:var(--hover)}.tasks tr[aria-current=true] td{background:var(--sel)}
.tasks .open{font:inherit;font-weight:600;color:var(--ink);background:none;border:0;padding:0;text-align:left;cursor:pointer}
.tasks .open:hover{text-decoration:underline}
.tasks .closed{white-space:nowrap;color:var(--soft)}
solace-task{display:block;margin-top:16px}
.milestone{margin:0 0 20px}.milestone h3{margin:0 0 8px;font-size:16px;border-bottom:1px solid var(--line)}
.feature{background:var(--paper);border:1px solid var(--edge);padding:8px 14px;margin:0 0 10px;max-width:760px}
.feature h4{margin:0 0 4px;font-size:15px}
.state{font-size:11px;text-transform:uppercase;letter-spacing:.08em;border:1px solid var(--line);padding:1px 6px;margin-left:6px;color:var(--soft);font-weight:400}
.feature-tasks{margin:6px 0 0;padding-left:20px}
.feature .open{font:inherit;color:var(--ink);background:none;border:0;padding:0;text-align:left;cursor:pointer}.feature .open:hover{text-decoration:underline}
.feature .open:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.feature.revealed,.milestone h3.revealed{outline:3px solid var(--focus);outline-offset:2px}
"
  <> solace_task.editor_styles
  <> plan.styles
  <> ".vision{background:var(--paper);border:1px solid var(--edge);padding:12px 20px;max-width:760px}
.vision .md p,.vision .md li{color:var(--ink)}.md p,.md ul,.md ol,.md blockquote{margin:6px 0}.md blockquote{padding-left:10px;border-left:3px solid var(--line);color:var(--soft)}
.md pre{background:var(--code);padding:8px;overflow:auto;white-space:pre-wrap;font:13px 'IBM Plex Mono',ui-monospace,monospace}.md code{font:13px 'IBM Plex Mono',ui-monospace,monospace;background:var(--code);padding:1px 4px}
.vision-actions{display:flex;flex-wrap:wrap;gap:8px;margin:12px 4px}
.vision-count{margin:4px;font-size:13px}[role=tabpanel] .vision-count[role=alert]{color:var(--revision);font-weight:600}
.vision-actions button,.toolbar button,.toolbar .button{font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;cursor:pointer}.vision-actions button{padding:6px 14px}
.vision-actions button:hover,.toolbar button:hover{background:var(--hover)}.vision-actions button:disabled{color:var(--soft);cursor:default}
.vision-actions button:focus-visible,.toolbar button:focus-visible,.composer textarea:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.composer textarea{display:block;font:inherit;color:var(--ink);background:var(--paper);min-height:320px;padding:10px;resize:vertical}
.feature-form{background:var(--paper);border:1px solid var(--edge);padding:12px 14px;margin:0 0 16px;max-width:760px}.feature-form h3{margin:0 0 10px;font-size:16px}.feature-form .filters{margin:10px 0}
.feature .edit{font:inherit;font-size:13px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:1px 8px;margin-left:6px;cursor:pointer}.feature .edit:hover{background:var(--hover)}.feature .edit:disabled{color:var(--soft);cursor:default}.feature .edit:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.new-task{display:flex;flex-wrap:wrap;gap:6px;align-items:center;margin:8px 0 2px}.new-task input,.new-task select{font:inherit;font-size:14px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:2px 6px}.new-task input{min-width:220px}.new-task input::placeholder{color:var(--soft);opacity:1}.new-task :focus-visible{outline:3px solid var(--focus);outline-offset:2px}.new-task .edit{margin-left:0}.new-task [role=alert]{flex-basis:100%;margin:2px 0}
.milestone h3 .edit{font:inherit;font-size:13px;font-weight:400;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:1px 8px;margin-left:6px;cursor:pointer}.milestone h3 .edit:hover{background:var(--hover)}.milestone h3 .edit:disabled{color:var(--soft);cursor:default}
.milestone .target{font-size:13px;font-weight:400;color:var(--soft);margin-left:6px}
.feature h4 select{font:inherit;font-size:13px;font-weight:400;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:1px 6px;margin-left:6px}
.feature-tasks .depends{font-size:13px;color:var(--soft)}
.feature-tasks li .edit{margin-left:6px}
.depends-edit{border:1px solid var(--line);margin:4px 0 8px;padding:6px 10px;max-width:600px}.depends-edit ul{list-style:none;margin:4px 0;padding:0}.depends-edit li{margin:2px 0}.depends-edit li .edit{margin:0 6px 0 0}.depends-edit p{font-size:13px;margin:4px 0}
.depends-edit input{font:inherit;font-size:14px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:2px 6px;min-width:260px;margin:4px 0}.depends-edit input::placeholder{color:var(--soft);opacity:1}
.milestone h3 .edit:focus-visible,.feature h4 select:focus-visible,.depends-edit :focus-visible{outline:3px solid var(--focus);outline-offset:2px}
"

pub fn register() -> Result(Nil, lustre.Error) {
  // The open task's details are `<solace-task>`, registered once per page.
  let _ = solace_task.register()
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("base", fn(base) { Ok(BaseChanged(base)) }),
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
/// shows, filters and searches (D-87). Notes and changes are not part of it.
pub type Row {
  Row(
    id: String,
    title: String,
    body: String,
    column: String,
    feature: Option(String),
    milestone: Option(String),
    /// When the task first reached a final column, in seconds; None while open.
    closed: Option(Int),
    /// The ids of the tasks this one depends on (D-87).
    depends: List(String),
  )
}

/// What the list keeps. "" in a field keeps every task; "-" for feature or
/// milestone keeps the tasks without one. `query` is searched in the title
/// and the body.
pub type Filters {
  Filters(
    column: String,
    project: String,
    feature: String,
    milestone: String,
    query: String,
  )
}

pub type Source

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "events")
fn events(
  base: String,
  receive: fn(String) -> Nil,
  ready: fn() -> Nil,
) -> Source

@external(javascript, "./api.mjs", "close_events")
fn close_events(source: Source) -> Nil

@external(javascript, "./api.mjs", "clock")
fn clock(seconds: Int) -> String

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
    selected: None,
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
    preview: False,
    saving: False,
    vision_error: "",
    feature_draft: None,
    feature_preview: False,
    feature_saving: False,
    feature_error: "",
    feature_request: "",
    feature_deleting: None,
    milestone_draft: None,
    milestone_preview: False,
    milestone_saving: False,
    milestone_error: "",
    milestone_request: "",
    milestone_deleting: None,
    depends_open: None,
    depends_query: "",
    task_titles: [],
    task_project: "",
    task_requests: [],
    task_saving: None,
    task_linking: None,
    task_errors: [],
    runs: [],
    plan_open: "",
    plan_collapsed: False,
    plan_width: plan.default_width,
    plan_text: "",
    plan_preview: False,
    plan_sending: False,
    plan_error: "",
    plan_entries: [],
    plan_poll: 0,
    plan_at_bottom: True,
    plan_new: False,
    plan_jump: False,
    models: [],
    plan_model: "",
    plan_effort: "",
    root: no_root(),
    reveal: #("", 0),
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
  #(
    model,
    effect.batch([
      get(model, "/projects", ProjectsLoaded),
      get(model, "/models", ModelsLoaded),
      effect.from(fn(send) {
        send(
          Listening(
            events(model.prefix, fn(text) { send(Live(text)) }, fn() {
              send(CaughtUp)
            }),
          ),
        )
      }),
    ]),
  )
}

fn stop(model: Model) -> Model {
  case model.source {
    Some(source) -> close_events(source)
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
  json.object([
    #(field, case value {
      "" -> json.null()
      id -> json.string(id)
    }),
  ])
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

/// Applies a message about the root's features, milestones, vision and
/// strategy runs, which names no project (D-87); an error for any other
/// message.
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
  // The list holds no notes: a run's stay until the run comes in full.
  |> result.lazy_or(fn() {
    at(text, "strategyRuns", decode.list(plan.decoder()))
    |> result.map(fn(rs) {
      Model(
        ..model,
        runs: list.map(rs, fn(r) {
          case list.find(model.runs, fn(old) { old.id == r.id }) {
            Ok(old) -> plan.Run(..r, notes: old.notes)
            Error(_) -> r
          }
        }),
      )
    })
  })
  |> result.lazy_or(fn() {
    at(text, "strategyRun", plan.decoder())
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

/// The project a live message is for: the one it names, else the root's.
fn event_dir(model: Model, text: String) -> String {
  case at(text, "project", decode.string) {
    Ok(dir) -> dir
    Error(_) ->
      list.find(model.projects, fn(p) { p.base == "" })
      |> result.map(fn(p) { p.dir })
      |> result.unwrap("")
  }
}

/// Whether a live update says the workspace's project list changed.
fn projects_event(text: String) -> Bool {
  at(text, "projects", decode.list(decode.string)) |> result.is_ok
}

/// Whether a live message can be applied now: its project is listed, or it
/// names none (the project list, features and milestones).
fn knows(model: Model, text: String) -> Bool {
  projects_event(text)
  || result.is_ok(records(model, text))
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
/// Depends on line, its project's workflow, or a change to the list of
/// projects, which its Scope tab shows.
fn for_detail(model: Model, text: String) -> Bool {
  case model.selected {
    None -> False
    Some(#(home, id)) ->
      case at(text, "task", decode.at(["id"], decode.string)) {
        Ok(task) -> task == id || list.contains(depends_of(model, id), task)
        Error(_) ->
          solace_task.projects_changed(text)
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
/// feature, milestone, dependsOn or closedTime.
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
  use closed <- decode.optional_field(
    "closedTime",
    None,
    decode.optional(decode.int),
  )
  use depends <- decode.optional_field(
    "dependsOn",
    [],
    decode.list(decode.string),
  )
  decode.success(Row(
    id,
    title,
    body,
    column,
    feature,
    milestone,
    closed,
    depends,
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
/// in the default order.
pub fn visible(model: Model) -> List(#(String, Row)) {
  model.projects
  |> list.flat_map(fn(p) {
    list.map(rows_of(model, p.dir), fn(r) { #(p.dir, r) })
  })
  |> list.filter(fn(pair) { keeps(model.filters, pair.0, pair.1) })
  |> sorted
}

/// Closed tasks newest first, then open tasks by title.
pub fn sorted(rows: List(#(String, Row))) -> List(#(String, Row)) {
  list.sort(rows, fn(a, b) {
    case { a.1 }.closed, { b.1 }.closed {
      Some(x), Some(y) -> int.compare(y, x)
      Some(_), None -> order.Lt
      None, Some(_) -> order.Gt
      None, None -> string.compare({ a.1 }.title, { b.1 }.title)
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

/// Every word of the query is in the title or the body, ignoring case. Notes
/// and changes are not in `GET /tasks`, so they are not searched.
fn searched(query: String, row: Row) -> Bool {
  let text = string.lowercase(row.title <> "\n" <> row.body)
  query
  |> string.lowercase
  |> string.split(" ")
  |> list.filter(fn(word) { word != "" })
  |> list.all(fn(word) { string.contains(text, word) })
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
  list.key_find(model.workflows, dir)
  |> result.try(fn(w) { list.find(w.columns, fn(c) { c.id == column }) })
  |> result.map(fn(c) { c.label })
  |> result.unwrap(column)
}

/// The Tasks tab: the filters, a row per task, and the open task's details
/// below them.
fn tasks_view(model: Model) -> List(Element(Msg)) {
  let rows = visible(model)
  [
    filters_view(model),
    case model.error {
      "" -> element.none()
      text -> html.p([attribute.role("alert")], [html.text(text)])
    },
    case rows {
      [] -> html.p([], [html.text("No tasks match.")])
      _ ->
        html.table([attribute.class("tasks")], [
          html.thead([], [
            html.tr(
              [],
              list.map(headings(model), fn(h) {
                html.th([attribute.attribute("scope", "col")], [html.text(h)])
              }),
            ),
          ]),
          html.tbody(
            [],
            list.map(rows, fn(pair) { row_view(model, pair.0, pair.1) }),
          ),
        ])
    },
    case model.selected {
      Some(#(dir, id)) -> task_view(model, dir, id)
      None -> element.none()
    },
  ]
}

fn headings(model: Model) -> List(String) {
  case model.workspace {
    True -> ["Title", "Column", "Project", "Feature", "Milestone", "Closed"]
    False -> ["Title", "Column", "Feature", "Milestone", "Closed"]
  }
}

fn filters_view(model: Model) -> Element(Msg) {
  let f = model.filters
  let records = fn(rs: List(#(String, String))) { [#("-", "(none)"), ..rs] }
  html.div([attribute.class("filters")], [
    html.label([], [
      html.text("Search"),
      html.input([
        attribute.type_("search"),
        attribute.value(f.query),
        attribute.placeholder("Title or description"),
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
      True ->
        choice(
          "Project",
          f.project,
          list.map(model.projects, fn(p) { #(p.dir, p.label) }),
          fn(v) { Filter(Filters(..f, project: v)) },
        )
      False -> element.none()
    },
    choice("Feature", f.feature, records(feature_pairs(model)), fn(v) {
      Filter(Filters(..f, feature: v))
    }),
    choice("Milestone", f.milestone, records(milestone_pairs(model)), fn(v) {
      Filter(Filters(..f, milestone: v))
    }),
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
        case list.contains(plan.open_ids(model.runs, "task"), row.id) {
          True -> plan.marker()
          False -> element.none()
        },
      ]),
      html.td([], [html.text(column_label(model, dir, row.column))]),
      case model.workspace {
        True -> html.td([], [html.text(label_of(model, dir))])
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
        link_select(
          milestone_pairs(model),
          row.milestone,
          "Milestone of " <> row.title,
          fn(v) { Link(dir, row.id, "milestone", v) },
        ),
      ]),
      html.td([attribute.class("closed")], [
        html.text(case row.closed {
          Some(seconds) -> clock(seconds)
          None -> ""
        }),
      ]),
    ],
  )
}

/// An inline link: "(none)" and every record. A link to a record not loaded
/// yet shows its id.
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
  html.select([attribute.attribute("aria-label", name), event.on_change(msg)], [
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
  ])
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
      attribute.attribute("hosted", ""),
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
        button("Edit", EditVision, False),
      ]),
    ]
    Some(#(text, base)) -> {
      // Past the limit the count is an alert and Save waits (D-92).
      let over = vision_count(text) > vision_limit
      [
        solace_task.editor(
          vision_id,
          "",
          model.prefix,
          "Vision",
          text,
          model.preview,
          Draft,
          VisionPreview,
          VisionFormat,
          VisionBlock,
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
              button("Save", SaveVision, model.saving || over),
              button("Cancel", CancelVision, model.saving),
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
/// every project.
fn features_view(model: Model) -> List(Element(Msg)) {
  let tasks =
    list.flat_map(model.projects, fn(p) {
      list.map(rows_of(model, p.dir), fn(r) {
        #(p.dir, r.id, r.title, r.feature)
      })
    })
  features.view(
    model.prefix,
    model.milestones,
    model.features,
    tasks,
    Open,
    features.Editor(
      draft: model.feature_draft,
      preview: model.feature_preview,
      saving: model.feature_saving,
      error: model.feature_error,
      on_new: NewFeature,
      on_edit: EditFeature,
      on_title: FeatureTitle,
      on_text: FeatureText,
      on_state: FeatureState,
      on_milestone: FeatureMilestone,
      on_preview: FeaturePreview,
      on_format: FeatureFormat,
      on_block: FeatureBlock,
      on_save: SaveFeature,
      on_cancel: CancelFeature,
      on_move: MoveFeature,
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
    ),
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
    plan.open_ids(model.runs, "feature"),
  )
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
    "" ->
      list.find(model.projects, fn(p) { p.base == "" })
      |> result.map(fn(p) { p.dir })
      |> result.unwrap(".")
    dir -> dir
  }
}

/// The Roadmap tab: the milestones with their features, and every project's
/// tasks with their dependencies.
fn roadmap_view(model: Model) -> List(Element(Msg)) {
  let tasks =
    list.flat_map(model.projects, fn(p) {
      list.map(rows_of(model, p.dir), fn(r) {
        roadmap.Task(
          p.dir,
          r.id,
          r.title,
          r.feature,
          r.milestone,
          r.depends,
          r.closed != None,
        )
      })
    })
  roadmap.view(
    model.prefix,
    model.milestones,
    model.features,
    tasks,
    Open,
    roadmap.Editor(
      draft: model.milestone_draft,
      preview: model.milestone_preview,
      saving: model.milestone_saving,
      error: model.milestone_error,
      on_new: NewMilestone,
      on_edit: EditMilestone,
      on_title: MilestoneTitle,
      on_text: MilestoneText,
      on_target_date: MilestoneTargetDate,
      on_state: MilestoneState,
      on_preview: MilestonePreview,
      on_format: MilestoneFormat,
      on_block: MilestoneBlock,
      on_save: SaveMilestone,
      on_cancel: CancelMilestone,
      on_move: MoveMilestone,
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
      on_depends: SetDepends,
      depends_open: model.depends_open,
      depends_query: model.depends_query,
      on_depends_open: OpenDepends,
      on_depends_query: DependsQuery,
    ),
    plan.open_ids(model.runs, "milestone"),
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

/// What Start plans: the task open on the Tasks tab, else the whole page.
pub fn plan_context(model: Model) -> plan.Context {
  case model.tab, model.selected {
    Tasks, Some(#(_, id)) -> plan.Context("task", id)
    _, _ -> plan.page
  }
}

/// The run shown in the planning panel.
fn plan_shown(model: Model) -> Option(plan.Run) {
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
      Model(..model, plan_sending: True, plan_error: ""),
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
  get(model, "/strategy/runs/" <> rid, PlanLoaded(poll, rid, _))
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
  plan.view(
    model.prefix,
    plan.Panel(
      runs: model.runs,
      context: plan_context(model),
      shown: plan_shown(model),
      entries: model.plan_entries,
      collapsed: model.plan_collapsed,
      width: model.plan_width,
      max_width: plan.max_width(window_width()),
      text: model.plan_text,
      preview: model.plan_preview,
      sending: model.plan_sending,
      error: model.plan_error,
      models: model.models,
      model: model.plan_model,
      effort: model.plan_effort,
      name: fn(kind, id) { record_name(model, kind, id) },
      on_collapse: PlanCollapse,
      on_drag: PlanDrag,
      on_key: PlanKey,
      on_text: PlanText,
      on_preview: PlanPreview,
      on_format: PlanFormat,
      on_block: PlanBlock,
      on_model: PlanModel,
      on_effort: PlanEffort,
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
  |> result.unwrap(plan.default_width)
  |> plan.clamp(window_width())
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
pub fn needs_attention(old: List(plan.Run), new: List(plan.Run)) -> Bool {
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
fn plan_track(
  old: Model,
  step: #(Model, Effect(Msg)),
) -> #(Model, Effect(Msg)) {
  let #(new, fx) = step
  case plan_shown(old), plan_shown(new) {
    Some(a), Some(b) if a.id == b.id && !new.plan_jump ->
      case plan.size(b) > plan.size(a), new.plan_at_bottom {
        True, True -> #(new, effect.batch([fx, plan_scroll()]))
        True, False -> #(Model(..new, plan_new: True), fx)
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

@external(javascript, "./api.mjs", "fit_plan_text")
fn fit_plan_text(root: dynamic.Dynamic) -> Nil

/// Sizes the panel's message box to its text, once it has rendered, where
/// the browser can't (see `fit_plan_text` in api.mjs).
fn plan_fit() -> Effect(Msg) {
  effect.after_paint(fn(_, root) { fit_plan_text(root) })
}

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
    effect.batch([
      effect.from(fn(_) { set_tab(tab_name(tab)) }),
      effect.after_paint(fn(_, root) { reveal_card(root, kind, id) }),
    ]),
  )
}

@external(javascript, "./api.mjs", "reveal_card")
fn reveal_card(root: dynamic.Dynamic, kind: String, id: String) -> Nil
