//// Build page: run the workspace's recipes, follow each build and download a release.

import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import solace_task

pub fn main() -> Nil {
  let assert Ok(_) = register()
  Nil
}

pub type Tab {
  BuildTab
}

pub fn tabs() -> List(Tab) {
  [BuildTab]
}

/// The tab's name in the URL: `/delivery?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    BuildTab -> "build"
  }
}

pub fn label(tab: Tab) -> String {
  case tab {
    BuildTab -> "Build"
  }
}

/// The tab a URL query names; Build when it names none or an unknown one.
pub fn tab_from(_query: String) -> Tab {
  BuildTab
}

/// A failed step's short summary (D-246).
pub type Summary {
  Summary(
    step: String,
    command: String,
    exit_code: Option(Int),
    lines: List(String),
  )
}

/// One step of a build as the server reports it.
pub type Step {
  Step(
    project: String,
    name: String,
    command: List(String),
    timeout: Int,
    state: String,
    exit_code: Option(Int),
    duration: Int,
    summary: Option(Summary),
    task: Option(String),
  )
}

pub type Build {
  Build(
    id: String,
    state: String,
    started_at: Int,
    ended_at: Option(Int),
    steps: List(Step),
  )
}

pub type PlanStep {
  PlanStep(name: String, command: List(String), timeout: Int)
}

pub type Recipe {
  Recipe(project: String, steps: List(PlanStep))
}

/// The step log open below the list.
pub type Log {
  Log(build: String, step: Int, text: String)
}

pub type Model {
  Model(
    tab: Tab,
    connected: Bool,
    source: Option(solace_task.Source),
    caught_up: Bool,
    replay: List(String),
    plan: List(Recipe),
    plan_error: String,
    build: Option(Build),
    /// "all" for every project, "chosen" for the ones ticked.
    choice: String,
    chosen: List(String),
    log: Option(Log),
    form: Option(Form),
    filing: Bool,
    error: String,
    starting: Bool,
    canceling: Bool,
    output: String,
    read_only: Bool,
    output_draft: String,
    read_only_draft: Bool,
    saving_settings: Bool,
    settings_error: String,
    versions: List(VersionFile),
    versions_agree: Bool,
    version: String,
    newest: String,
    versions_error: String,
    release: Option(Release),
    desktop: Bool,
  )
}

pub type Msg {
  Select(Tab)
  /// The shell shows this tab: a load, Back or Forward.
  TabShown(Tab)
  Connected
  Disconnected
  Listening(solace_task.Source)
  Live(String)
  CaughtUp
  PlanLoaded(String)
  /// The workspace's version files and the next version.
  VersionsLoaded(String)
  VersionChanged(String)
  StartBuild
  /// The reply to a start: the build, or a refusal.
  BuildStarted(String)
  CancelBuild
  /// The reply to a cancel: the build, or a refusal.
  BuildCanceled(String)
  /// The project choice: "all" or "chosen".
  ChoiceSelected(String)
  ToggleProject(String, Bool)
  OpenStep(Int)
  LogLoaded(String, Int, String)
  CloseLog
  /// A failed step's **File a task** button.
  FileTask(Int)
  FormTitle(String)
  FormBody(String)
  CreateTask
  /// The reply to filing a task: the form's step and the task, or a refusal.
  TaskFiled(Int, String)
  CancelForm
  /// The release setting as the server reports it.
  SettingsLoaded(String)
  OutputChanged(String)
  ReadOnlyChanged(Bool)
  SaveSettings
  /// The reply to saving the setting, or a refusal.
  SettingsSaved(String)
  /// The latest release, or a refusal.
  ReleasesLoaded(String)
  OpenFolder
  /// The desktop app's reply to opening the folder: empty on success.
  FolderOpened(String)
}

/// The page before it hears from anywhere.
pub fn initial() -> Model {
  Model(
    tab: BuildTab,
    connected: False,
    source: None,
    caught_up: False,
    replay: [],
    plan: [],
    plan_error: "",
    build: None,
    choice: "all",
    chosen: [],
    log: None,
    form: None,
    filing: False,
    error: "",
    starting: False,
    canceling: False,
    output: "build-output",
    read_only: True,
    output_draft: "build-output",
    read_only_draft: True,
    saving_settings: False,
    settings_error: "",
    versions: [],
    versions_agree: True,
    version: "",
    newest: "",
    versions_error: "",
    release: None,
    desktop: False,
  )
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(Model(..initial(), desktop: desktop_ffi()), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Select(tab) if tab == model.tab -> #(model, effect.none())
    Select(tab) -> #(
      Model(..model, tab:),
      event.emit("tab", json.string(tab_name(tab))),
    )
    TabShown(tab) -> #(Model(..model, tab:), effect.none())
    Connected ->
      case model.connected {
        True -> #(model, effect.none())
        False -> #(
          Model(..model, connected: True, caught_up: False, replay: []),
          effect.batch([
            effect.from(fn(send) {
              send(
                Listening(
                  solace_task.events(
                    "",
                    fn(text) { send(Live(text)) },
                    fn() { send(CaughtUp) },
                    ["build", "buildLog", "builds"],
                  ),
                ),
              )
            }),
            get("/build/plan", PlanLoaded),
            get("/build/settings", SettingsLoaded),
            get("/build/versions", VersionsLoaded),
            get("/releases", ReleasesLoaded),
          ]),
        )
      }
    Disconnected -> #(
      Model(..stop(model), connected: False, caught_up: False, replay: []),
      effect.none(),
    )
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    // A new connection replays what the page missed; it raises no attention.
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> {
      let #(next, ended) = apply(model, text)
      let attention = case ended {
        True -> event.emit("attention", json.null())
        False -> effect.none()
      }
      case released(ended, next.build) {
        True -> #(
          next,
          effect.batch([
            attention,
            get("/releases", ReleasesLoaded),
            get("/build/versions", VersionsLoaded),
          ]),
        )
        False -> #(next, attention)
      }
    }
    CaughtUp -> {
      let model =
        list.fold(list.reverse(model.replay), model, fn(acc, text) {
          let #(next, _) = apply(acc, text)
          next
        })
      #(Model(..model, caught_up: True, replay: []), effect.none())
    }
    PlanLoaded(text) ->
      case json.parse(text, decode.list(recipe_decoder())) {
        Ok(plan) -> {
          let names = list.map(plan, fn(recipe) { recipe.project })
          #(
            Model(
              ..model,
              plan:,
              plan_error: "",
              chosen: list.filter(model.chosen, fn(name) {
                list.contains(names, name)
              }),
            ),
            effect.none(),
          )
        }
        Error(_) -> #(Model(..model, plan_error: refusal(text)), effect.none())
      }
    VersionsLoaded(text) ->
      case json.parse(text, versions_decoder()) {
        Ok(ProposedVersions(files, agree, proposal, newest)) -> #(
          Model(
            ..model,
            versions: files,
            versions_agree: agree,
            version: proposal,
            newest: newest,
            versions_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, versions_error: refusal(text)),
          effect.none(),
        )
      }
    VersionChanged(value) -> #(Model(..model, version: value), effect.none())
    StartBuild -> {
      let body = case model.choice {
        "chosen" ->
          json.to_string(
            json.object([
              #("version", json.string(model.version)),
              #("projects", json.array(model.chosen, json.string)),
            ]),
          )
        _ ->
          json.to_string(
            json.object([#("version", json.string(model.version))]),
          )
      }
      #(
        Model(..model, starting: True, error: ""),
        post("/builds", body, BuildStarted),
      )
    }
    BuildStarted(text) ->
      case json.parse(text, build_decoder()) {
        Ok(build) -> #(
          Model(
            ..model,
            build: Some(build),
            starting: False,
            canceling: False,
            log: None,
            error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, starting: False, error: refusal(text)),
          effect.none(),
        )
      }
    CancelBuild ->
      case model.build {
        Some(build) if build.state == "running" -> #(
          Model(..model, canceling: True, error: ""),
          post("/builds/" <> build.id <> "/cancel", "{}", BuildCanceled),
        )
        _ -> #(model, effect.none())
      }
    BuildCanceled(text) ->
      case json.parse(text, build_decoder()) {
        Ok(build) -> #(
          Model(..model, build: Some(build), canceling: False, error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, canceling: False, error: refusal(text)),
          effect.none(),
        )
      }
    ChoiceSelected(value) -> #(Model(..model, choice: value), effect.none())
    ToggleProject(project, on) -> {
      let chosen = case on {
        True ->
          case list.contains(model.chosen, project) {
            True -> model.chosen
            False -> list.append(model.chosen, [project])
          }
        False -> list.filter(model.chosen, fn(name) { name != project })
      }
      #(Model(..model, chosen:), effect.none())
    }
    OpenStep(index) ->
      case model.build {
        Some(build) -> #(
          Model(..model, log: Some(Log(build.id, index, ""))),
          get(
            "/builds/"
              <> build.id
              <> "/steps/"
              <> int.to_string(index)
              <> "/log",
            fn(text) { LogLoaded(build.id, index, text) },
          ),
        )
        None -> #(model, effect.none())
      }
    LogLoaded(id, index, text) ->
      case model.log {
        Some(Log(build, step, _)) if build == id && step == index -> #(
          Model(..model, log: Some(Log(id, index, text))),
          effect.none(),
        )
        _ -> #(model, effect.none())
      }
    CloseLog -> #(Model(..model, log: None), effect.none())
    FileTask(index) ->
      case model.build {
        Some(build) ->
          case list.first(list.drop(build.steps, index)) {
            Ok(step) ->
              case step.task, step.summary {
                None, Some(summary) -> #(
                  Model(
                    ..model,
                    form: Some(Form(
                      build.id,
                      index,
                      step.project,
                      task_title(step),
                      failure_body(build, step, summary),
                    )),
                    error: "",
                  ),
                  effect.none(),
                )
                _, _ -> #(model, effect.none())
              }
            Error(_) -> #(model, effect.none())
          }
        None -> #(model, effect.none())
      }
    FormTitle(value) ->
      case model.form {
        Some(Form(build, step, project, _, body)) -> #(
          Model(..model, form: Some(Form(build, step, project, value, body))),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    FormBody(value) ->
      case model.form {
        Some(Form(build, step, project, title, _)) -> #(
          Model(..model, form: Some(Form(build, step, project, title, value))),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    CreateTask ->
      case model.form {
        Some(Form(build, step, _, title, body)) -> #(
          Model(..model, filing: True, error: ""),
          post(
            "/builds/" <> build <> "/steps/" <> int.to_string(step) <> "/task",
            json.to_string(
              json.object([
                #("title", json.string(title)),
                #("body", json.string(body)),
              ]),
            ),
            fn(text) { TaskFiled(step, text) },
          ),
        )
        None -> #(model, effect.none())
      }
    TaskFiled(index, text) ->
      case json.parse(text, task_decoder()) {
        Ok(id) -> #(
          Model(
            ..model,
            build: set_step_task(model.build, index, id),
            form: None,
            filing: False,
            error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, filing: False, error: refusal(text)),
          effect.none(),
        )
      }
    CancelForm -> #(Model(..model, form: None, filing: False), effect.none())
    SettingsLoaded(text) ->
      case json.parse(text, settings_decoder()) {
        Ok(ReleaseSettings(output, read_only)) -> #(
          Model(
            ..model,
            output:,
            read_only:,
            output_draft: output,
            read_only_draft: read_only,
            settings_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, settings_error: refusal(text)),
          effect.none(),
        )
      }
    OutputChanged(value) -> #(
      Model(..model, output_draft: value),
      effect.none(),
    )
    ReadOnlyChanged(value) -> #(
      Model(..model, read_only_draft: value),
      effect.none(),
    )
    SaveSettings -> #(
      Model(..model, saving_settings: True, settings_error: ""),
      put(
        "/build/settings",
        json.to_string(
          json.object([
            #("output", json.string(model.output_draft)),
            #("read_only", json.bool(model.read_only_draft)),
          ]),
        ),
        SettingsSaved,
      ),
    )
    SettingsSaved(text) ->
      case json.parse(text, settings_decoder()) {
        Ok(ReleaseSettings(output, read_only)) -> #(
          Model(
            ..model,
            output:,
            read_only:,
            output_draft: output,
            read_only_draft: read_only,
            saving_settings: False,
            settings_error: "",
          ),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, saving_settings: False, settings_error: refusal(text)),
          effect.none(),
        )
      }
    ReleasesLoaded(text) ->
      case json.parse(text, releases_decoder()) {
        Ok(releases) -> #(
          Model(..model, release: case list.first(releases) {
            Ok(release) -> Some(release)
            Error(_) -> None
          }),
          effect.none(),
        )
        Error(_) -> #(Model(..model, error: refusal(text)), effect.none())
      }
    OpenFolder ->
      case model.release {
        Some(Release(version, _)) -> #(
          model,
          effect.from(fn(send) {
            open_folder_ffi(version, fn(text) { send(FolderOpened(text)) })
          }),
        )
        None -> #(model, effect.none())
      }
    FolderOpened(text) -> #(
      Model(..model, error: case text {
        "" -> model.error
        message -> message
      }),
      effect.none(),
    )
  }
}

@external(javascript, "./api.mjs", "request")
fn request_ffi(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request_ffi(path, "GET", "", fn(text) { send(msg(text)) })
  })
}

fn post(path: String, body: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request_ffi(path, "POST", body, fn(text) { send(msg(text)) })
  })
}

fn stop(model: Model) -> Model {
  case model.source {
    Some(source) -> {
      solace_task.close_events(source)
      Model(..model, source: None)
    }
    None -> model
  }
}

/// The flag says the single build this message carried just ended, so attention is raised once.
fn apply(model: Model, text: String) -> #(Model, Bool) {
  case
    json.parse(
      text,
      decode.field("build", build_decoder(), fn(build) { decode.success(build) }),
    )
  {
    Ok(build) -> #(
      set_build(model, build),
      build_ended(Some(build)) && !build_ended(model.build),
    )
    Error(_) ->
      case
        json.parse(
          text,
          decode.field("builds", decode.list(build_decoder()), fn(builds) {
            decode.success(builds)
          }),
        )
      {
        Ok(builds) ->
          case list.last(builds) {
            Ok(build) -> #(set_build(model, build), False)
            Error(_) -> #(model, False)
          }
        Error(_) ->
          case
            json.parse(
              text,
              decode.field("buildLog", log_decoder(), fn(log) {
                decode.success(log)
              }),
            )
          {
            Ok(#(id, step, text)) -> #(append_log(model, id, step, text), False)
            Error(_) -> #(model, False)
          }
      }
  }
}

fn set_build(model: Model, build: Build) -> Model {
  let same = case model.build {
    Some(old) -> old.id == build.id
    None -> False
  }
  let log = case same {
    True -> model.log
    False -> None
  }
  Model(..model, build: Some(build), log:)
}

fn append_log(model: Model, id: String, step: Int, text: String) -> Model {
  case model.log {
    Some(Log(build, at, current)) if build == id && at == step ->
      Model(..model, log: Some(Log(id, step, current <> text)))
    _ -> model
  }
}

fn build_ended(build: Option(Build)) -> Bool {
  case build {
    Some(build) ->
      build.state == "passed"
      || build.state == "failed"
      || build.state == "canceled"
    None -> False
  }
}

fn refusal(text: String) -> String {
  case
    json.parse(
      text,
      decode.field("message", decode.string, fn(message) {
        decode.success(message)
      }),
    )
  {
    Ok(message) -> message
    Error(_) -> text
  }
}

fn summary_decoder() -> decode.Decoder(Summary) {
  use step <- decode.field("step", decode.string)
  use command <- decode.field("command", decode.string)
  use exit_code <- decode.optional_field(
    "exitCode",
    None,
    decode.optional(decode.int),
  )
  use lines <- decode.optional_field("lines", [], decode.list(decode.string))
  decode.success(Summary(step, command, exit_code, lines))
}

fn step_decoder() -> decode.Decoder(Step) {
  use project <- decode.field("project", decode.string)
  use name <- decode.field("name", decode.string)
  use command <- decode.field("command", decode.list(decode.string))
  use timeout <- decode.optional_field("timeout", 0, decode.int)
  use state <- decode.field("state", decode.string)
  use exit_code <- decode.optional_field(
    "exitCode",
    None,
    decode.optional(decode.int),
  )
  use duration <- decode.optional_field("duration", 0, decode.int)
  use summary <- decode.optional_field(
    "summary",
    None,
    decode.optional(summary_decoder()),
  )
  use task <- decode.optional_field(
    "task",
    None,
    decode.optional(decode.string),
  )
  decode.success(Step(
    project,
    name,
    command,
    timeout,
    state,
    exit_code,
    duration,
    summary,
    task,
  ))
}

pub fn build_decoder() -> decode.Decoder(Build) {
  use id <- decode.field("id", decode.string)
  use state <- decode.field("state", decode.string)
  use started_at <- decode.optional_field("startedAt", 0, decode.int)
  use ended_at <- decode.optional_field(
    "endedAt",
    None,
    decode.optional(decode.int),
  )
  use steps <- decode.field("steps", decode.list(step_decoder()))
  decode.success(Build(id, state, started_at, ended_at, steps))
}

fn plan_step_decoder() -> decode.Decoder(PlanStep) {
  use name <- decode.field("name", decode.string)
  use command <- decode.field("run", decode.list(decode.string))
  use timeout <- decode.optional_field("timeout", 0, decode.int)
  decode.success(PlanStep(name, command, timeout))
}

fn recipe_decoder() -> decode.Decoder(Recipe) {
  use project <- decode.field("project", decode.string)
  use steps <- decode.field("steps", decode.list(plan_step_decoder()))
  decode.success(Recipe(project, steps))
}

fn log_decoder() -> decode.Decoder(#(String, Int, String)) {
  use id <- decode.field("id", decode.string)
  use step <- decode.field("step", decode.int)
  use text <- decode.field("text", decode.string)
  decode.success(#(id, step, text))
}

pub fn view(model: Model) -> Element(Msg) {
  html.main([attribute.class("delivery")], [
    html.style([], view_styles),
    html.div(
      [
        attribute.class("tabs"),
        attribute.role("tablist"),
        attribute.attribute("aria-label", "Delivery"),
      ],
      list.map(tabs(), fn(tab) { tab_button(tab, model.tab) }),
    ),
    html.div(
      [
        attribute.id("panel-" <> tab_name(model.tab)),
        attribute.role("tabpanel"),
        attribute.attribute("aria-labelledby", "tab-" <> tab_name(model.tab)),
      ],
      build_view(model),
    ),
  ])
}

fn tab_button(tab: Tab, current: Tab) -> Element(Msg) {
  let selected = tab == current
  html.button(
    [
      attribute.id("tab-" <> tab_name(tab)),
      attribute.role("tab"),
      attribute.attribute("aria-selected", bool_text(selected)),
      attribute.attribute("aria-controls", "panel-" <> tab_name(tab)),
      event.on_click(Select(tab)),
    ],
    [html.text(label(tab))],
  )
}

fn bool_text(value: Bool) -> String {
  case value {
    True -> "true"
    False -> "false"
  }
}

fn build_view(model: Model) -> List(Element(Msg)) {
  [
    settings_view(model),
    versions_view(model),
    html.div([attribute.class("build-head")], [
      html.div([attribute.class("build-actions")], [
        html.button(
          [
            attribute.class("build-start"),
            attribute.disabled(start_disabled(model)),
            event.on_click(StartBuild),
          ],
          [
            html.text(case model.starting {
              True -> "Starting…"
              False -> "Build"
            }),
          ],
        ),
        cancel_button(model),
      ]),
      choice_view(model),
    ]),
    case model.plan_error {
      "" -> element.none()
      message -> error_box(message)
    },
    case model.error {
      "" -> element.none()
      message -> error_box(message)
    },
    build_line(model),
    release_view(model),
    failure_summary(model),
    steps_view(model),
    log_view(model),
  ]
}

pub fn start_disabled(model: Model) -> Bool {
  model.starting
  || is_running(model.build)
  || model.version == ""
  || model.choice == "chosen"
  && model.chosen == []
}

fn is_running(build: Option(Build)) -> Bool {
  case build {
    Some(build) -> build.state == "running"
    None -> False
  }
}

fn cancel_button(model: Model) -> Element(Msg) {
  case is_running(model.build) {
    False -> element.none()
    True ->
      html.button(
        [
          attribute.class("build-cancel"),
          attribute.disabled(model.canceling),
          event.on_click(CancelBuild),
        ],
        [
          html.text(case model.canceling {
            True -> "Canceling…"
            False -> "Cancel"
          }),
        ],
      )
  }
}

fn choice_view(model: Model) -> Element(Msg) {
  html.fieldset([attribute.class("project-choice")], [
    html.legend([], [html.text("Projects")]),
    html.label([], [
      html.input([
        attribute.type_("radio"),
        attribute.name("projects"),
        attribute.value("all"),
        attribute.checked(model.choice != "chosen"),
        event.on_input(ChoiceSelected),
      ]),
      html.text(" All projects"),
    ]),
    html.label([], [
      html.input([
        attribute.type_("radio"),
        attribute.name("projects"),
        attribute.value("chosen"),
        attribute.checked(model.choice == "chosen"),
        event.on_input(ChoiceSelected),
      ]),
      html.text(" Chosen projects"),
    ]),
    case model.choice == "chosen" {
      True ->
        html.ul(
          [attribute.class("chosen")],
          list.map(model.plan, fn(recipe) { project_pick(model, recipe) }),
        )
      False -> element.none()
    },
  ])
}

fn project_pick(model: Model, recipe: Recipe) -> Element(Msg) {
  let project = recipe.project
  html.li([], [
    html.label([], [
      html.input([
        attribute.type_("checkbox"),
        attribute.name("project"),
        attribute.value(project),
        attribute.checked(list.contains(model.chosen, project)),
        event.on_check(fn(on) { ToggleProject(project, on) }),
      ]),
      html.text(" " <> project),
    ]),
  ])
}

fn error_box(message: String) -> Element(Msg) {
  html.p([attribute.class("error"), attribute.role("alert")], [
    html.text(message),
  ])
}

fn build_line(model: Model) -> Element(Msg) {
  case model.build {
    None ->
      html.p([attribute.class("last-build")], [html.text("No builds yet.")])
    Some(build) ->
      html.p([attribute.class("last-build")], [
        html.text("Last build: " <> state_label(build.state)),
      ])
  }
}

fn failure_summary(model: Model) -> Element(Msg) {
  case model.build {
    Some(build) ->
      case
        list.find(build.steps, fn(step) {
          step.state == "failed" && step.summary != None
        })
      {
        Ok(found) ->
          case
            list.index_map(build.steps, fn(step, index) { #(index, step) })
            |> list.find(fn(pair) { pair.1 == found })
          {
            Ok(#(index, step)) ->
              case step.summary {
                Some(summary) ->
                  html.section(
                    [attribute.class("failure"), attribute.role("alert")],
                    [
                      html.h2([], [html.text("Build failed")]),
                      html.p([], [
                        html.text(summary.step <> " · " <> summary.command),
                      ]),
                      html.p([attribute.class("exit")], [
                        html.text(exit_text(summary.exit_code)),
                      ]),
                      summary_lines(summary.lines),
                      task_action(model, index, step),
                      task_form(model, index),
                    ],
                  )
                None -> element.none()
              }
            Error(_) -> element.none()
          }
        Error(_) -> element.none()
      }
    None -> element.none()
  }
}

fn exit_text(exit_code: Option(Int)) -> String {
  case exit_code {
    Some(code) -> "Exit code " <> int.to_string(code)
    None -> "No exit code"
  }
}

fn summary_lines(lines: List(String)) -> Element(Msg) {
  case lines {
    [] -> html.p([attribute.class("no-output")], [html.text("No output.")])
    _ ->
      html.ul(
        [attribute.class("failure-lines")],
        list.map(lines, fn(line) { html.li([], [html.text(line)]) }),
      )
  }
}

type ShownStep {
  ShownStep(
    index: Int,
    project: String,
    name: String,
    state: String,
    duration: Int,
  )
}

fn steps_view(model: Model) -> Element(Msg) {
  case step_groups(model) {
    [] ->
      html.p([attribute.class("no-steps")], [
        html.text(case model.build {
          None -> "No build recipe yet."
          Some(_) -> "This build has no steps."
        }),
      ])
    groups ->
      html.div([attribute.class("build-steps")], list.map(groups, group_view))
  }
}

fn group_view(group: #(String, List(ShownStep))) -> Element(Msg) {
  let #(project, steps) = group
  html.section([attribute.class("project-group")], [
    html.h2([], [html.text(project)]),
    html.ol([attribute.class("steps")], list.map(steps, step_view)),
  ])
}

fn step_view(step: ShownStep) -> Element(Msg) {
  let name = case step.index >= 0 {
    True ->
      html.button(
        [attribute.class("step-name"), event.on_click(OpenStep(step.index))],
        [html.text(step.name)],
      )
    False -> html.span([attribute.class("step-name")], [html.text(step.name)])
  }
  html.li([attribute.class("step " <> step.state)], [
    name,
    html.span([attribute.class("state")], [html.text(state_label(step.state))]),
    html.span([attribute.class("duration")], [
      html.text(duration_text(step.duration)),
    ]),
  ])
}

fn step_groups(model: Model) -> List(#(String, List(ShownStep))) {
  list.fold(shown_steps(model), [], fn(groups, step) {
    add_group(groups, step.project, step)
  })
}

fn add_group(
  groups: List(#(String, List(ShownStep))),
  project: String,
  step: ShownStep,
) -> List(#(String, List(ShownStep))) {
  case list.any(groups, fn(group) { group.0 == project }) {
    True ->
      list.map(groups, fn(group) {
        case group.0 == project {
          True -> #(project, list.append(group.1, [step]))
          False -> group
        }
      })
    False -> list.append(groups, [#(project, [step])])
  }
}

fn shown_steps(model: Model) -> List(ShownStep) {
  case model.build {
    Some(build) ->
      list.index_map(build.steps, fn(step, index) {
        ShownStep(index, step.project, step.name, step.state, step.duration)
      })
    None ->
      model.plan
      |> list.map(fn(recipe) {
        list.map(recipe.steps, fn(step) {
          ShownStep(-1, recipe.project, step.name, "waiting", 0)
        })
      })
      |> list.flatten
  }
}

fn log_view(model: Model) -> Element(Msg) {
  case model.log {
    None -> element.none()
    Some(log) ->
      html.section([attribute.class("log")], [
        html.div([attribute.class("log-head")], [
          html.h2([], [html.text("Step log")]),
          html.button([event.on_click(CloseLog)], [html.text("Close")]),
        ]),
        html.pre([], [
          html.text(case log.text {
            "" -> "No output yet."
            text -> text
          }),
        ]),
      ])
  }
}

pub fn state_label(state: String) -> String {
  case state {
    "waiting" -> "Waiting"
    "running" -> "Running"
    "passed" -> "Passed"
    "failed" -> "Failed"
    "skipped" -> "Skipped"
    "canceled" -> "Canceled"
    _ -> "Waiting"
  }
}

pub fn duration_text(ms: Int) -> String {
  case ms >= 1000 {
    True ->
      int.to_string(ms / 1000) <> "." <> int.to_string(ms % 1000 / 100) <> " s"
    False -> int.to_string(ms) <> " ms"
  }
}

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("tab", fn(name) {
        Ok(TabShown(tab_from("?tab=" <> name)))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-delivery",
  )
}

const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec;--ok-text:#1a6b30;--warn-text:#8a4a0a;--revision:#a32628;--shadow:#17222626}
.delivery{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:calc(100dvh - 56px);box-sizing:border-box;background:var(--sheet);color:var(--ink);--tabs-height:44px;--tab-pad:0 18px}
.delivery>[role=tabpanel]{padding:var(--space-6)}
.build-head{display:flex;flex-wrap:wrap;gap:var(--space-3) var(--space-6);align-items:flex-start;justify-content:space-between;margin-bottom:var(--space-4)}
.build-actions{display:flex;gap:var(--space-2);align-items:center}
.project-choice{margin:0;border:1px solid var(--line);padding:var(--space-2) var(--space-3);background:var(--paper)}
.project-choice legend{font-size:13px;color:var(--soft);padding:0 var(--space-1)}
.project-choice label{display:inline-flex;gap:var(--space-1);align-items:center;margin:0 var(--space-3) 0 0;font-size:14px}
.project-choice .chosen{list-style:none;margin:var(--space-2) 0 0;padding:0;display:flex;flex-wrap:wrap;gap:var(--space-1) var(--space-3)}
.project-choice .chosen label{margin:0}
.last-build{color:var(--soft);margin:0 0 var(--space-3)}
.build-steps{display:grid;gap:var(--space-4)}
.project-group{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}
.project-group h2{margin:0 0 var(--space-2);font-size:var(--h3)}
.steps{list-style:none;margin:0;padding:0}
.step{display:grid;grid-template-columns:minmax(0,1fr) auto auto;gap:var(--space-3);align-items:center;padding:var(--space-1) 0;border-top:1px solid var(--line)}
.step-name{font:inherit;color:var(--ink);background:none;border:0;padding:0;min-height:0;text-align:left;cursor:pointer}
.step-name:hover{text-decoration:underline}
.step-name:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.state{font-size:11px;text-transform:uppercase;letter-spacing:.08em;border:1px solid var(--line);padding:1px var(--space-2);color:var(--soft);white-space:nowrap}
.step.running .state{background:var(--well);color:var(--ink)}
.step.passed .state{color:var(--ok-text);border-color:var(--ok-text)}
.step.failed .state{color:var(--revision);border-color:var(--revision);font-weight:700}
.step.skipped .state{color:var(--soft);text-decoration:line-through}
.duration{font-size:13px;color:var(--soft);font-variant-numeric:tabular-nums;white-space:nowrap}
.failure{border:2px solid var(--revision);background:var(--paper);padding:var(--pad-card);margin:0 0 var(--space-4)}
.failure h2{margin:0 0 var(--space-1);font-size:var(--h3);color:var(--revision)}
.failure p{margin:var(--space-1) 0;color:var(--ink)}
.failure .failure-lines{margin:var(--space-2) 0 0;padding-left:var(--space-5);font:13px 'IBM Plex Mono',ui-monospace,monospace}
.failure .failure-lines li{white-space:pre-wrap;margin:var(--space-1) 0}
.error{color:var(--ink);border:1px solid var(--revision);border-left:6px solid var(--revision);background:var(--paper);padding:var(--space-2) var(--space-3);font-weight:600}
.no-steps{color:var(--soft)}
.log{margin:var(--space-4) 0 0;border:1px solid var(--edge);background:var(--paper);padding:var(--pad-card)}
.log-head{display:flex;align-items:center;justify-content:space-between;gap:var(--space-3)}
.log h2{margin:0;font-size:var(--h3)}
.log pre{margin:var(--space-2) 0 0;max-height:40vh;overflow:auto;white-space:pre-wrap;background:var(--code);color:var(--ink);padding:var(--space-3);font:13px 'IBM Plex Mono',ui-monospace,monospace}
@media (max-width:700px){.step{grid-template-columns:minmax(0,1fr) auto}.step .duration{grid-column:2;text-align:right}}
.release{margin:0 0 var(--space-4);border:1px solid var(--edge);background:var(--paper);padding:var(--pad-card)}
.release-head{display:flex;align-items:center;justify-content:space-between;gap:var(--space-3)}
.release h2{margin:0;font-size:var(--h3)}
.artifacts{list-style:none;margin:var(--space-2) 0 0;padding:0;display:grid;gap:var(--space-2)}
.artifact{display:grid;grid-template-columns:minmax(0,1fr) auto auto auto;gap:var(--space-3);align-items:center;border-top:1px solid var(--line);padding:var(--space-1) 0}
.artifact-name{overflow-wrap:anywhere}
.artifact-size{font-size:13px;color:var(--soft);white-space:nowrap}
.artifact-checksum{font:12px 'IBM Plex Mono',ui-monospace,monospace;color:var(--soft);overflow-wrap:anywhere}
"

/// The new-task form a failed step opens.
pub type Form {
  Form(build: String, step: Int, project: String, title: String, body: String)
}

fn task_decoder() -> decode.Decoder(String) {
  use id <- decode.field("id", decode.string)
  decode.success(id)
}

fn task_action(model: Model, index: Int, step: Step) -> Element(Msg) {
  case step.task {
    Some(id) ->
      html.p([attribute.class("failure-task")], [
        html.a([attribute.href("/task/" <> id)], [html.text("Open task")]),
      ])
    None ->
      html.button(
        [
          attribute.class("file-task"),
          attribute.disabled(model.filing),
          event.on_click(FileTask(index)),
        ],
        [html.text("File a task")],
      )
  }
}

fn task_form(model: Model, index: Int) -> Element(Msg) {
  case model.form {
    Some(Form(_, step, project, title, body)) if step == index ->
      html.div([attribute.class("file-task-form")], [
        html.h3([], [html.text("New task")]),
        html.label([], [
          html.text("Title"),
          html.input([
            attribute.type_("text"),
            attribute.value(title),
            event.on_input(FormTitle),
          ]),
        ]),
        html.label([], [
          html.text("Body"),
          html.textarea([event.on_input(FormBody)], body),
        ]),
        html.p([], [html.text("Project: " <> project)]),
        html.div([attribute.class("form-actions")], [
          html.button(
            [attribute.disabled(model.filing), event.on_click(CreateTask)],
            [
              html.text(case model.filing {
                True -> "Creating…"
                False -> "Create task"
              }),
            ],
          ),
          html.button([event.on_click(CancelForm)], [html.text("Cancel")]),
        ]),
      ])
    _ -> element.none()
  }
}

fn task_title(step: Step) -> String {
  "build: " <> step.project <> " " <> step.name <> " fails"
}

fn failure_body(build: Build, step: Step, summary: Summary) -> String {
  let lines = case summary.lines {
    [] -> "No output."
    lines -> list.map(lines, fn(line) { "- " <> line }) |> string.join("\n")
  }
  "### Build failure\n\n"
  <> "**"
  <> step.project
  <> " · "
  <> summary.step
  <> "**\n\n"
  <> exit_text(summary.exit_code)
  <> "\n\nCommand: `"
  <> summary.command
  <> "`\n\n"
  <> lines
  <> "\n\n[Open the build](/delivery?build="
  <> build.id
  <> ")"
}

fn set_step_task(
  build: Option(Build),
  index: Int,
  task: String,
) -> Option(Build) {
  case build {
    Some(build) ->
      Some(
        Build(
          ..build,
          steps: list.index_map(build.steps, fn(step, i) {
            case i == index {
              True -> Step(..step, task: Some(task))
              False -> step
            }
          }),
        ),
      )
    None -> None
  }
}

/// The release output folder and its read-only option.
pub type ReleaseSettings {
  ReleaseSettings(output: String, read_only: Bool)
}

fn put(path: String, body: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request_ffi(path, "PUT", body, fn(text) { send(msg(text)) })
  })
}

fn settings_decoder() -> decode.Decoder(ReleaseSettings) {
  use output <- decode.field("output", decode.string)
  use read_only <- decode.field("read_only", decode.bool)
  decode.success(ReleaseSettings(output, read_only))
}

fn settings_view(model: Model) -> Element(Msg) {
  html.fieldset([attribute.class("project-choice")], [
    html.legend([], [html.text("Releases")]),
    html.label([], [
      html.text("Output folder"),
      html.input([
        attribute.type_("text"),
        attribute.value(model.output_draft),
        event.on_input(OutputChanged),
      ]),
    ]),
    html.label([], [
      html.input([
        attribute.type_("radio"),
        attribute.name("release-read-only"),
        attribute.value("on"),
        attribute.checked(model.read_only_draft),
        event.on_input(fn(_) { ReadOnlyChanged(True) }),
      ]),
      html.text(" Read-only for agents"),
    ]),
    html.label([], [
      html.input([
        attribute.type_("radio"),
        attribute.name("release-read-only"),
        attribute.value("off"),
        attribute.checked(!model.read_only_draft),
        event.on_input(fn(_) { ReadOnlyChanged(False) }),
      ]),
      html.text(" Writable for agents"),
    ]),
    case model.settings_error {
      "" -> element.none()
      message -> error_box(message)
    },
    html.button(
      [
        attribute.disabled(model.saving_settings),
        event.on_click(SaveSettings),
      ],
      [
        html.text(case model.saving_settings {
          True -> "Saving…"
          False -> "Save"
        }),
      ],
    ),
  ])
}

pub type Artifact {
  Artifact(path: String, size: Int, sha256: String)
}

pub type Release {
  Release(version: String, artifacts: List(Artifact))
}

@external(javascript, "./api.mjs", "desktop")
fn desktop_ffi() -> Bool

@external(javascript, "./api.mjs", "open_folder")
fn open_folder_ffi(version: String, receive: fn(String) -> Nil) -> Nil

fn release_view(model: Model) -> Element(Msg) {
  case model.release {
    None -> element.none()
    Some(Release(version, artifacts)) ->
      html.section([attribute.class("release")], [
        html.div([attribute.class("release-head")], [
          html.h2([], [html.text("Release " <> version)]),
          case model.desktop {
            True ->
              html.button([event.on_click(OpenFolder)], [
                html.text("Open folder"),
              ])
            False -> element.none()
          },
        ]),
        html.ul(
          [attribute.class("artifacts")],
          list.map(artifacts, fn(artifact) { artifact_view(version, artifact) }),
        ),
      ])
  }
}

fn artifact_view(version: String, artifact: Artifact) -> Element(Msg) {
  let Artifact(path, size, sha256) = artifact
  html.li([attribute.class("artifact")], [
    html.span([attribute.class("artifact-name")], [html.text(path)]),
    html.span([attribute.class("artifact-size")], [
      html.text(int.to_string(size) <> " bytes"),
    ]),
    html.code([attribute.class("artifact-checksum")], [html.text(sha256)]),
    html.a(
      [
        attribute.href("/releases/" <> version <> "/" <> path),
        attribute.attribute("download", ""),
      ],
      [html.text("Download")],
    ),
  ])
}

fn artifact_decoder() -> decode.Decoder(Artifact) {
  use path <- decode.field("path", decode.string)
  use size <- decode.field("size", decode.int)
  use sha256 <- decode.field("sha256", decode.string)
  decode.success(Artifact(path, size, sha256))
}

fn release_decoder() -> decode.Decoder(Release) {
  use version <- decode.field("version", decode.string)
  use artifacts <- decode.field("artifacts", decode.list(artifact_decoder()))
  decode.success(Release(version, artifacts))
}

fn releases_decoder() -> decode.Decoder(List(Release)) {
  use releases <- decode.field("releases", decode.list(release_decoder()))
  decode.success(releases)
}

/// A build that just passed has a release to load.
pub fn released(ended: Bool, build: Option(Build)) -> Bool {
  case ended, build {
    True, Some(build) -> build.state == "passed"
    _, _ -> False
  }
}

pub type VersionFile {
  VersionFile(project: String, file: String, version: String)
}

pub type ProposedVersions {
  ProposedVersions(
    files: List(VersionFile),
    agree: Bool,
    proposal: String,
    newest: String,
  )
}

fn versions_view(model: Model) -> Element(Msg) {
  html.fieldset([attribute.class("versions")], [
    html.legend([], [html.text("Version")]),
    html.label([], [
      html.text("New version"),
      html.input([
        attribute.type_("text"),
        attribute.value(model.version),
        event.on_input(VersionChanged),
      ]),
    ]),
    case model.versions_error {
      "" -> element.none()
      message -> error_box(message)
    },
    case older(model.version, model.newest) {
      True ->
        html.p([attribute.class("version-warning")], [
          html.text("This version is older than the newest release."),
        ])
      False -> element.none()
    },
    html.ul(
      [attribute.class("version-files")],
      list.map(model.versions, version_file_view),
    ),
    case model.versions_agree {
      True -> element.none()
      False ->
        html.p([attribute.class("version-disagreement")], [
          html.text("Version files disagree"),
        ])
    },
  ])
}

fn version_file_view(file: VersionFile) -> Element(Msg) {
  case file {
    VersionFile(project, name, version) ->
      html.li([], [html.text(project <> " / " <> name <> ": " <> version)])
  }
}

fn version_file_decoder() -> decode.Decoder(VersionFile) {
  use project <- decode.field("project", decode.string)
  use file <- decode.field("file", decode.string)
  use version <- decode.field("version", decode.string)
  decode.success(VersionFile(project, file, version))
}

fn versions_decoder() -> decode.Decoder(ProposedVersions) {
  use files <- decode.field("versions", decode.list(version_file_decoder()))
  use agree <- decode.field("agree", decode.bool)
  use proposal <- decode.field("proposal", decode.string)
  use newest <- decode.field("newest", decode.string)
  decode.success(ProposedVersions(files, agree, proposal, newest))
}

type Version =
  #(Int, Int, Int)

fn semver(version: String) -> Option(Version) {
  case string.split(version, ".") {
    [major, minor, patch] ->
      case int.parse(major), int.parse(minor), int.parse(patch) {
        Ok(major), Ok(minor), Ok(patch) -> Some(#(major, minor, patch))
        _, _, _ -> None
      }
    _ -> None
  }
}

/// Whether `version` is valid semver and older than the newest release.
fn older(version: String, newest: String) -> Bool {
  case semver(version), semver(newest) {
    Some(#(major, minor, patch)), Some(#(new_major, new_minor, new_patch)) ->
      major < new_major
      || major == new_major
      && minor < new_minor
      || major == new_major
      && minor == new_minor
      && patch < new_patch
    _, _ -> False
  }
}
