//// System Review page: run a review of the whole workspace by hand, of the kinds ticked,
//// and list the reviews that completed.

import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import lustre
import lustre/attribute
import lustre/component
import lustre/effect
import lustre/element
import lustre/element/html
import lustre/event
import schedule
import solace_task

pub fn main() -> Nil {
  let assert Ok(_) = register()
  Nil
}

/// A kind of review, one of those the server's start route knows.
pub type Kind {
  Security
  CodeQuality
  UserExperience
}

pub fn kinds() -> List(Kind) {
  [Security, CodeQuality, UserExperience]
}

/// The id a start request names the kind by.
pub fn kind_id(kind: Kind) -> String {
  case kind {
    Security -> "security"
    CodeQuality -> "code-quality"
    UserExperience -> "user-experience"
  }
}

pub fn kind_label(kind: Kind) -> String {
  case kind {
    Security -> "Security"
    CodeQuality -> "Code quality"
    UserExperience -> "User experience"
  }
}

pub type Model {
  Model(
    tab: Tab,
    chosen: List(Kind),
    starting: Bool,
    /// The kinds of the review last started; [] before one starts.
    started: List(Kind),
    error: String,
    /// The completed reviews, newest first, as the server last listed them.
    past: Option(List(Review)),
    past_error: String,
    /// The id of the review whose details show in the dialog.
    open: Option(String),
    /// The text of `SYSTEMREVIEWS.md`, as the server last read it.
    log: Option(String),
    log_error: String,
    /// The schedules, as the server last listed them.
    schedules: Option(List(schedule.Schedule)),
    schedules_error: String,
    /// The schedule open in the form: a new one has no id.
    editing: Option(schedule.Schedule),
    /// How many times the form has opened; a save's reply names the form it was sent from.
    opened: Int,
    saving: Bool,
    edit_error: String,
    /// The missed reviews not yet answered, as the server last listed them.
    missed: List(MissedReview),
    missed_error: String,
    /// Whether a missed review's answer is on its way.
    answering: Bool,
    /// Whether the shell shows this page; a missed review is read once it is.
    shown: Bool,
    source: Option(solace_task.Source),
  )
}

pub type Msg {
  Select(Tab)
  ToggleKind(Kind, Bool)
  StartReview
  /// The reply to a start: the kinds it asked for, and the run or a refusal.
  ReviewStarted(List(Kind), String)
  /// The reply to `GET /system-reviews`: the completed reviews, or a refusal.
  PastLoaded(String)
  OpenReview(String)
  CloseReview
  /// The reply to `GET /system-reviews/log`: the log's text, or a refusal.
  LogLoaded(String)
  /// The reply to `GET /system-reviews/schedules`: every schedule, or a refusal.
  SchedulesLoaded(String)
  AddSchedule
  EditSchedule(String)
  CloseSchedule
  EditKind(Kind, Bool)
  EditRepeat(String)
  EditTime(String)
  EditWeekday(String)
  EditDay(String)
  EditDate(String)
  SaveSchedule
  DeleteSchedule
  /// The reply to a schedule's save or removal from the form opened `n`th:
  /// every schedule, or a refusal.
  ScheduleSaved(Int, String)
  Connected
  Disconnected
  Listening(solace_task.Source)
  /// A `missedReviews` event: the missed reviews not yet answered.
  MissedHeard(String)
  /// The shell shows this page, or hides it.
  Shown(Bool)
  /// A missed review's **Run now** button: its schedule.
  RunMissed(String)
  /// A missed review's **Skip** button: its schedule.
  SkipMissed(String)
  /// The reply to answering a missed review: those left, or a refusal.
  MissedAnswered(String)
  /// The reply to reading missed reviews: those left, or a refusal.
  MissedRead(String)
}

/// Every kind is chosen at first.
pub fn initial() -> Model {
  Model(
    tab: RunTab,
    chosen: kinds(),
    starting: False,
    started: [],
    error: "",
    past: None,
    past_error: "",
    open: None,
    log: None,
    log_error: "",
    schedules: None,
    schedules_error: "",
    editing: None,
    opened: 0,
    saving: False,
    edit_error: "",
    missed: [],
    missed_error: "",
    answering: False,
    shown: False,
    source: None,
  )
}

pub fn init(_flags: Nil) -> #(Model, effect.Effect(Msg)) {
  #(initial(), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, effect.Effect(Msg)) {
  case msg {
    Connected ->
      case model.source {
        Some(_) -> #(model, effect.none())
        None -> #(
          model,
          effect.from(fn(send) {
            send(
              Listening(
                solace_task.events(
                  "",
                  fn(text) { send(MissedHeard(text)) },
                  fn() { Nil },
                  ["missedReviews"],
                ),
              ),
            )
          }),
        )
      }
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    Disconnected -> {
      option.map(model.source, solace_task.close_events)
      #(Model(..model, source: None), effect.none())
    }
    MissedHeard(text) ->
      case json.parse(text, decode.at(["missedReviews"], missed_decoder())) {
        Ok(missed) -> heard(model, missed)
        Error(_) -> #(model, effect.none())
      }
    Shown(shown) -> heard(Model(..model, shown:), model.missed)
    RunMissed(id) -> answer(model, id, "run")
    SkipMissed(id) -> answer(model, id, "skip")
    MissedAnswered(text) ->
      missed_listed(Model(..model, answering: False), text)
    MissedRead(text) -> missed_listed(model, text)
    Select(RunTab) -> #(Model(..model, tab: RunTab), effect.none())
    // Listed afresh each time, so a due time that passed since shows its next one.
    Select(SchedulesTab) -> #(
      Model(..model, tab: SchedulesTab),
      get(schedules_path, SchedulesLoaded),
    )
    SchedulesLoaded(text) ->
      case json.parse(text, decode.list(schedule.decoder())) {
        Ok(all) -> #(
          Model(..model, schedules: Some(all), schedules_error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, schedules: None, schedules_error: refusal(text)),
          effect.none(),
        )
      }
    AddSchedule -> #(
      Model(
        ..model,
        editing: Some(schedule.new(list.map(kinds(), kind_id))),
        opened: model.opened + 1,
        edit_error: "",
      ),
      effect.none(),
    )
    EditSchedule(id) -> {
      let found =
        option.unwrap(model.schedules, [])
        |> list.find(fn(s) { s.id == id })
        |> option.from_result
      #(
        Model(..model, editing: found, opened: model.opened + 1, edit_error: ""),
        effect.none(),
      )
    }
    CloseSchedule -> #(
      Model(..model, editing: None, saving: False, edit_error: ""),
      effect.none(),
    )
    EditKind(kind, on) ->
      edit(model, fn(s) {
        let chosen =
          list.filter(kinds(), fn(k) { list.contains(s.kinds, kind_id(k)) })
        schedule.Schedule(
          ..s,
          kinds: list.map(choose(chosen, kind, on), kind_id),
        )
      })
    EditRepeat(id) ->
      edit(model, fn(s) {
        schedule.Schedule(
          ..s,
          repeat: result.unwrap(schedule.repeat_from_id(id), s.repeat),
        )
      })
    EditTime(time) -> edit(model, fn(s) { schedule.Schedule(..s, time:) })
    EditWeekday(text) ->
      edit(model, fn(s) {
        schedule.Schedule(
          ..s,
          weekday: result.unwrap(int.parse(text), s.weekday),
        )
      })
    EditDay(text) ->
      edit(model, fn(s) {
        schedule.Schedule(..s, day: result.unwrap(int.parse(text), s.day))
      })
    EditDate(date) -> edit(model, fn(s) { schedule.Schedule(..s, date:) })
    SaveSchedule ->
      case model.editing, save_disabled(model) {
        Some(s), False -> #(
          Model(..model, saving: True, edit_error: ""),
          save(s, model.opened),
        )
        _, _ -> #(model, effect.none())
      }
    DeleteSchedule ->
      case model.editing {
        Some(s) if s.id != "" && !model.saving -> #(
          Model(..model, saving: True, edit_error: ""),
          send("DELETE", schedules_path <> "/" <> s.id, "{}", fn(text) {
            ScheduleSaved(model.opened, text)
          }),
        )
        _ -> #(model, effect.none())
      }
    ScheduleSaved(opened, text) ->
      case
        json.parse(text, decode.list(schedule.decoder())),
        opened == model.opened
      {
        Ok(all), True -> #(
          Model(
            ..model,
            schedules: Some(all),
            schedules_error: "",
            editing: None,
            saving: False,
            edit_error: "",
          ),
          effect.none(),
        )
        Ok(all), False -> #(
          Model(..model, schedules: Some(all), schedules_error: ""),
          effect.none(),
        )
        Error(_), True -> #(
          Model(..model, saving: False, edit_error: refusal(text)),
          effect.none(),
        )
        Error(_), False -> #(model, effect.none())
      }
    // Listed afresh each time, so a review that completed since shows.
    Select(PastTab) -> #(
      Model(..model, tab: PastTab),
      get("/system-reviews", PastLoaded),
    )
    // Read afresh each time, so a review that completed since shows.
    Select(LogTab) -> #(
      Model(..model, tab: LogTab),
      get("/system-reviews/log", LogLoaded),
    )
    LogLoaded(text) ->
      case
        json.parse(text, decode.field("text", decode.string, decode.success))
      {
        Ok(log) -> #(
          Model(..model, log: Some(log), log_error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, log_error: refusal(text)), effect.none())
      }
    PastLoaded(text) ->
      case json.parse(text, decode.list(review_decoder())) {
        Ok(past) -> #(
          Model(..model, past: Some(past), past_error: ""),
          effect.none(),
        )
        Error(_) -> #(Model(..model, past_error: refusal(text)), effect.none())
      }
    OpenReview(id) -> #(Model(..model, open: Some(id)), effect.none())
    CloseReview -> #(Model(..model, open: None), effect.none())
    ToggleKind(kind, on) -> #(
      Model(..model, chosen: choose(model.chosen, kind, on)),
      effect.none(),
    )
    StartReview ->
      case start_disabled(model) {
        True -> #(model, effect.none())
        False -> {
          let chosen = model.chosen
          #(
            Model(..model, starting: True, started: [], error: ""),
            post("/system-reviews", start_body(chosen), fn(text) {
              ReviewStarted(chosen, text)
            }),
          )
        }
      }
    ReviewStarted(kinds, text) ->
      case json.parse(text, run_decoder()) {
        Ok(_) -> #(
          Model(..model, starting: False, started: kinds, error: ""),
          effect.none(),
        )
        Error(_) -> #(
          Model(..model, starting: False, error: refusal(text)),
          effect.none(),
        )
      }
  }
}

/// The kinds chosen once `kind` is ticked or unticked, in the page's order.
fn choose(chosen: List(Kind), kind: Kind, on: Bool) -> List(Kind) {
  list.filter(kinds(), fn(k) {
    case k == kind {
      True -> on
      False -> list.contains(chosen, k)
    }
  })
}

pub fn start_disabled(model: Model) -> Bool {
  model.starting || model.chosen == []
}

/// A start request's body: `{"kinds": [<id>, ...]}`.
pub fn start_body(kinds: List(Kind)) -> String {
  json.to_string(
    json.object([
      #("kinds", json.array(kinds, fn(kind) { json.string(kind_id(kind)) })),
    ]),
  )
}

pub fn started_text(kinds: List(Kind)) -> String {
  "Review started: "
  <> string.join(list.map(kinds, kind_label), ", ")
  <> ". Its findings will appear as tasks in Planning."
}

/// A started review's run: the server answers a start with it.
fn run_decoder() -> decode.Decoder(String) {
  decode.field("id", decode.string, decode.success)
}

fn refusal(text: String) -> String {
  case json.parse(text, message_decoder()) {
    Ok(message) -> message
    Error(_) -> text
  }
}

@external(javascript, "./api.mjs", "request")
fn request_ffi(
  path: String,
  method: String,
  body: String,
  receive: fn(Int, String) -> Nil,
) -> Nil

fn post(
  path: String,
  body: String,
  msg: fn(String) -> Msg,
) -> effect.Effect(Msg) {
  send("POST", path, body, msg)
}

pub fn view(model: Model) -> element.Element(Msg) {
  html.main([attribute.class("sysreview")], [
    html.style([], view_styles),
    missed_notice(
      model.missed,
      model.missed_error,
      model.answering,
      RunMissed,
      SkipMissed,
    ),
    html.div(
      [
        attribute.class("tabs"),
        attribute.role("tablist"),
        attribute.attribute("aria-label", "System Review"),
      ],
      list.map(tabs(), fn(tab) { tab_button(tab, model.tab) }),
    ),
    html.div(
      [
        attribute.id("panel-" <> tab_name(model.tab)),
        attribute.role("tabpanel"),
        attribute.attribute("aria-labelledby", "tab-" <> tab_name(model.tab)),
      ],
      case model.tab {
        RunTab -> run_view(model)
        SchedulesTab -> schedules_view(model)
        PastTab -> past_view(model)
        LogTab -> log_view(model)
      },
    ),
    review_dialog(model),
    schedule_dialog(model),
  ])
}

fn kind_pick(
  checked: Bool,
  kind: Kind,
  msg: fn(Bool) -> Msg,
) -> element.Element(Msg) {
  html.label([], [
    html.input([
      attribute.type_("checkbox"),
      attribute.name("kind"),
      attribute.value(kind_id(kind)),
      attribute.checked(checked),
      event.on_check(msg),
    ]),
    html.text(" " <> kind_label(kind)),
  ])
}

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("shown", fn(shown) {
        Ok(Shown(shown == "true"))
      }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-system-review",
  )
}

const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> solace_task.modal_styles
  <> missed_styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec;--ok-text:#1a6b30;--warn-text:#8a4a0a;--revision:#a32628;--shadow:#17222626}
.sysreview{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;min-height:calc(100dvh - 56px);box-sizing:border-box;background:var(--sheet);color:var(--ink);--tabs-height:44px;--tab-pad:0 18px}
.sysreview>[role=tabpanel]{padding:var(--space-6)}
.review-run{display:flex;flex-wrap:wrap;gap:var(--space-3);align-items:center;margin:0 0 var(--space-4)}
.kind-choice{margin:0;border:1px solid var(--line);padding:var(--space-2) var(--space-3);background:var(--paper);display:flex;flex-wrap:wrap;gap:var(--space-1) var(--space-3)}
.kind-choice label{display:inline-flex;gap:var(--space-1);align-items:center;font-size:14px}
.error{color:var(--ink);border:1px solid var(--revision);border-left:6px solid var(--revision);background:var(--paper);padding:var(--space-2) var(--space-3);font-weight:600}
.started{margin:0 0 var(--space-3);color:var(--ok-text)}
.no-reviews{color:var(--soft)}
.past-reviews{border-collapse:collapse;background:var(--paper);border:1px solid var(--edge)}
.past-reviews th,.past-reviews td{text-align:left;padding:var(--space-1) var(--space-3);border-top:1px solid var(--line)}
.past-reviews th{font-size:13px;color:var(--soft);font-weight:600}
.past-reviews tbody tr{cursor:pointer}
.past-reviews tbody tr:hover{background:var(--hover)}
.review-open{font:inherit;color:var(--ink);background:none;border:0;padding:0;min-height:0;text-align:left;cursor:pointer}
.review-open:focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.review-details{display:grid;grid-template-columns:auto 1fr;gap:var(--space-1) var(--space-3);margin:0 0 var(--space-4)}
.review-details dt{color:var(--soft)}
.review-details dd{margin:0}
.review-details ul{margin:0;padding-left:var(--space-5)}
.actions{display:flex;justify-content:flex-end;gap:var(--space-2)}
.schedule-add{margin:0 0 var(--space-4)}
.schedule-form{display:grid;gap:var(--space-3);margin:0 0 var(--space-4)}
.schedule-form>label{display:grid;gap:var(--space-1);font-size:14px;color:var(--soft)}
"

fn run_view(model: Model) -> List(element.Element(Msg)) {
  [
    html.div([attribute.class("review-run")], [
      html.fieldset(
        [
          attribute.class("kind-choice"),
          attribute.attribute("aria-label", "Review kinds"),
        ],
        list.map(kinds(), fn(kind) {
          kind_pick(list.contains(model.chosen, kind), kind, fn(on) {
            ToggleKind(kind, on)
          })
        }),
      ),
      html.button(
        [
          attribute.class("review-start"),
          attribute.disabled(start_disabled(model)),
          event.on_click(StartReview),
        ],
        [
          html.text(case model.starting {
            True -> "Starting…"
            False -> "Run review"
          }),
        ],
      ),
    ]),
    error_box(model.error),
    case model.started {
      [] -> element.none()
      kinds ->
        html.p([attribute.class("started"), attribute.role("status")], [
          html.text(started_text(kinds)),
        ])
    },
  ]
}

pub type Tab {
  RunTab
  SchedulesTab
  PastTab
  LogTab
}

pub fn tabs() -> List(Tab) {
  [RunTab, SchedulesTab, PastTab, LogTab]
}

fn tab_name(tab: Tab) -> String {
  case tab {
    RunTab -> "run"
    SchedulesTab -> "schedules"
    PastTab -> "past"
    LogTab -> "log"
  }
}

pub fn tab_label(tab: Tab) -> String {
  case tab {
    RunTab -> "Run review"
    SchedulesTab -> "Schedules"
    PastTab -> "Past reviews"
    LogTab -> "Log"
  }
}

/// A completed review: its run, when the run started and ended (seconds), the
/// kind ids, the version reviewed and its finding tasks' ids.
pub type Review {
  Review(
    id: String,
    started: Int,
    ended: Int,
    kinds: List(String),
    version: Option(String),
    findings: List(String),
  )
}

fn review_decoder() -> decode.Decoder(Review) {
  use id <- decode.field("id", decode.string)
  use started <- decode.field("started", decode.int)
  use ended <- decode.field("ended", decode.int)
  use kinds <- decode.field("kinds", decode.list(decode.string))
  use version <- decode.field("version", decode.optional(decode.string))
  use findings <- decode.field("findings", decode.list(decode.string))
  decode.success(Review(id:, started:, ended:, kinds:, version:, findings:))
}

/// A kind id's label; an id the page doesn't know shows as it is.
fn kind_text(id: String) -> String {
  case list.find(kinds(), fn(kind) { kind_id(kind) == id }) {
    Ok(kind) -> kind_label(kind)
    Error(_) -> id
  }
}

pub fn kinds_text(review: Review) -> String {
  kind_ids_text(review.kinds)
}

pub fn version_text(review: Review) -> String {
  option.unwrap(review.version, "None named")
}

/// A time, in seconds, as the human's locale writes a date and time.
@external(javascript, "./api.mjs", "clock")
pub fn clock(seconds: Int) -> String

fn get(path: String, msg: fn(String) -> Msg) -> effect.Effect(Msg) {
  send("GET", path, "", msg)
}

fn tab_button(tab: Tab, current: Tab) -> element.Element(Msg) {
  html.button(
    [
      attribute.id("tab-" <> tab_name(tab)),
      attribute.role("tab"),
      attribute.attribute("aria-selected", case tab == current {
        True -> "true"
        False -> "false"
      }),
      attribute.attribute("aria-controls", "panel-" <> tab_name(tab)),
      event.on_click(Select(tab)),
    ],
    [html.text(tab_label(tab))],
  )
}

fn error_box(message: String) -> element.Element(Msg) {
  case message {
    "" -> element.none()
    message ->
      html.p([attribute.class("error"), attribute.role("alert")], [
        html.text(message),
      ])
  }
}

fn past_view(model: Model) -> List(element.Element(Msg)) {
  [
    error_box(model.past_error),
    case model.past {
      None -> element.none()
      Some([]) ->
        html.p([attribute.class("no-reviews")], [
          html.text("No completed reviews yet."),
        ])
      Some(past) ->
        html.table([attribute.class("past-reviews")], [
          html.thead([], [
            html.tr(
              [],
              list.map(
                ["Started", "Ended", "Kinds", "Version", "Findings"],
                fn(name) { html.th([], [html.text(name)]) },
              ),
            ),
          ]),
          html.tbody([], list.map(past, review_row)),
        ])
    },
  ]
}

/// A review's row; clicking it, or its Started button from the keyboard, opens its details.
fn review_row(review: Review) -> element.Element(Msg) {
  html.tr([event.on_click(OpenReview(review.id))], [
    html.td([], [
      html.button([attribute.class("review-open")], [
        html.text(clock(review.started)),
      ]),
    ]),
    html.td([], [html.text(clock(review.ended))]),
    html.td([], [html.text(kinds_text(review))]),
    html.td([], [html.text(version_text(review))]),
    html.td([], [html.text(int.to_string(list.length(review.findings)))]),
  ])
}

/// The open review's details over the page; none while no review is open.
fn review_dialog(model: Model) -> element.Element(Msg) {
  let open = case model.open, model.past {
    Some(id), Some(past) -> list.find(past, fn(review) { review.id == id })
    _, _ -> Error(Nil)
  }
  case open {
    Error(_) -> element.none()
    Ok(review) ->
      solace_task.modal([
        html.dl([attribute.class("review-details")], [
          html.dt([], [html.text("Started")]),
          html.dd([], [html.text(clock(review.started))]),
          html.dt([], [html.text("Ended")]),
          html.dd([], [html.text(clock(review.ended))]),
          html.dt([], [html.text("Kinds")]),
          html.dd([], [html.text(kinds_text(review))]),
          html.dt([], [html.text("Version")]),
          html.dd([], [html.text(version_text(review))]),
          html.dt([], [html.text("Findings")]),
          html.dd([], [findings_view(review.findings)]),
        ]),
        html.div([attribute.class("actions")], [
          html.button([event.on_click(CloseReview)], [html.text("Close")]),
        ]),
      ])
  }
}

fn findings_view(findings: List(String)) -> element.Element(Msg) {
  case findings {
    [] -> html.text("None")
    _ ->
      html.ul(
        [],
        list.map(findings, fn(id) {
          html.li([], [
            html.a(
              [
                attribute.href("/task/" <> id),
                attribute.attribute("data-task", id),
              ],
              [html.text("task:" <> id)],
            ),
          ])
        }),
      )
  }
}

/// The log as Markdown, so each finding's `task:<id>` is a link.
fn log_view(model: Model) -> List(element.Element(Msg)) {
  [
    error_box(model.log_error),
    case model.log {
      None -> element.none()
      Some(text) -> solace_task.md("", text)
    },
  ]
}

const schedules_path = "/system-reviews/schedules"

/// The model with `change` made to the schedule open in the form.
fn edit(
  model: Model,
  change: fn(schedule.Schedule) -> schedule.Schedule,
) -> #(Model, effect.Effect(Msg)) {
  #(Model(..model, editing: option.map(model.editing, change)), effect.none())
}

pub fn save_disabled(model: Model) -> Bool {
  case model.editing {
    Some(s) -> model.saving || !schedule.ready(s)
    None -> True
  }
}

/// Adds a new schedule, or changes a saved one.
fn save(s: schedule.Schedule, opened: Int) -> effect.Effect(Msg) {
  let saved = fn(text) { ScheduleSaved(opened, text) }
  case s.id {
    "" -> send("POST", schedules_path, schedule.body(s), saved)
    id -> send("PUT", schedules_path <> "/" <> id, schedule.body(s), saved)
  }
}

fn send(
  method: String,
  path: String,
  body: String,
  msg: fn(String) -> Msg,
) -> effect.Effect(Msg) {
  effect.from(fn(dispatch) {
    request_ffi(path, method, body, fn(status, text) {
      dispatch(msg(reply_text(status, text)))
    })
  })
}

/// The labels of the kinds `ids` names, in order.
pub fn kind_ids_text(ids: List(String)) -> String {
  string.join(list.map(ids, kind_text), ", ")
}

/// When the schedule next runs; none once a one-off time has passed.
pub fn next_text(s: schedule.Schedule) -> String {
  case s.next {
    Some(seconds) -> clock(seconds)
    None -> "None"
  }
}

fn schedules_view(model: Model) -> List(element.Element(Msg)) {
  [
    html.button([attribute.class("schedule-add"), event.on_click(AddSchedule)], [
      html.text("Add schedule"),
    ]),
    error_box(model.schedules_error),
    case model.schedules {
      None -> element.none()
      Some([]) ->
        html.p([attribute.class("no-reviews")], [html.text("No schedules yet.")])
      Some(all) ->
        html.table([attribute.class("past-reviews")], [
          html.thead([], [
            html.tr(
              [],
              list.map(["Kinds", "Repeats", "Next run"], fn(name) {
                html.th([], [html.text(name)])
              }),
            ),
          ]),
          html.tbody([], list.map(all, schedule_row)),
        ])
    },
  ]
}

/// A schedule's row; clicking it, or its Kinds button from the keyboard, opens it in the form.
fn schedule_row(s: schedule.Schedule) -> element.Element(Msg) {
  html.tr([event.on_click(EditSchedule(s.id))], [
    html.td([], [
      html.button([attribute.class("review-open")], [
        html.text(kind_ids_text(s.kinds)),
      ]),
    ]),
    html.td([], [html.text(schedule.repeat_text(s))]),
    html.td([], [html.text(next_text(s))]),
  ])
}

/// The schedule being added or changed, in a form over the page; none while no schedule is open.
fn schedule_dialog(model: Model) -> element.Element(Msg) {
  case model.editing {
    None -> element.none()
    Some(s) ->
      solace_task.modal([
        html.div([attribute.class("schedule-form")], [
          html.fieldset(
            [
              attribute.class("kind-choice"),
              attribute.attribute("aria-label", "Review kinds"),
            ],
            list.map(kinds(), fn(kind) {
              kind_pick(list.contains(s.kinds, kind_id(kind)), kind, fn(on) {
                EditKind(kind, on)
              })
            }),
          ),
          field(
            "Repeats",
            html.select(
              [attribute.name("repeat"), event.on_change(EditRepeat)],
              list.map(schedule.repeats(), fn(repeat) {
                html.option(
                  [
                    attribute.value(schedule.repeat_id(repeat)),
                    attribute.selected(repeat == s.repeat),
                  ],
                  schedule.repeat_label(repeat),
                )
              }),
            ),
          ),
          field(
            "Time",
            html.input([
              attribute.type_("time"),
              attribute.name("time"),
              attribute.value(s.time),
              event.on_input(EditTime),
            ]),
          ),
          case s.repeat {
            schedule.Daily -> element.none()
            schedule.Weekly ->
              field(
                "Day",
                number_pick(
                  "weekday",
                  schedule.weekdays(),
                  s.weekday,
                  schedule.weekday_name,
                  EditWeekday,
                ),
              )
            schedule.Monthly ->
              field(
                "Day of the month",
                number_pick(
                  "day",
                  schedule.days(),
                  s.day,
                  int.to_string,
                  EditDay,
                ),
              )
            schedule.Once ->
              field(
                "Date",
                html.input([
                  attribute.type_("date"),
                  attribute.name("date"),
                  attribute.value(s.date),
                  event.on_input(EditDate),
                ]),
              )
          },
        ]),
        error_box(model.edit_error),
        html.div([attribute.class("actions")], [
          case s.id {
            "" -> element.none()
            _ ->
              html.button(
                [
                  attribute.class("schedule-delete"),
                  attribute.disabled(model.saving),
                  event.on_click(DeleteSchedule),
                ],
                [html.text("Delete")],
              )
          },
          html.button([event.on_click(CloseSchedule)], [html.text("Cancel")]),
          html.button(
            [
              attribute.class("schedule-save"),
              attribute.disabled(save_disabled(model)),
              event.on_click(SaveSchedule),
            ],
            [html.text("Save")],
          ),
        ]),
      ])
  }
}

fn field(label: String, control: element.Element(Msg)) -> element.Element(Msg) {
  html.label([], [html.text(label), control])
}

/// A choice of one of `numbers`, each shown by `name`.
fn number_pick(
  field_name: String,
  numbers: List(Int),
  chosen: Int,
  name: fn(Int) -> String,
  msg: fn(String) -> Msg,
) -> element.Element(Msg) {
  html.select(
    [attribute.name(field_name), event.on_change(msg)],
    list.map(numbers, fn(n) {
      html.option(
        [attribute.value(int.to_string(n)), attribute.selected(n == chosen)],
        name(n),
      )
    }),
  )
}

/// The model holding `missed`. An unread one is read once the page shows; until
/// then the page asks for the human's attention.
fn heard(
  model: Model,
  missed: List(MissedReview),
) -> #(Model, effect.Effect(Msg)) {
  let model = Model(..model, missed:)
  case list.any(missed, fn(m) { !m.read }), model.shown {
    False, _ -> #(model, effect.none())
    True, True -> #(
      model,
      post("/system-reviews/missed/read", "{}", MissedRead),
    )
    True, False -> #(model, event.emit("attention", json.null()))
  }
}

/// A scheduled review that didn't run, as the server lists it: its schedule's
/// id, kind ids and repeat, the latest time it was due, in seconds, and whether
/// the human has read it.
pub type MissedReview {
  MissedReview(
    schedule: String,
    kinds: List(String),
    repeat: schedule.Repeat,
    due: Int,
    read: Bool,
  )
}

pub fn missed_decoder() -> decode.Decoder(List(MissedReview)) {
  decode.list({
    use schedule <- decode.field("schedule", decode.string)
    use kinds <- decode.field("kinds", decode.list(decode.string))
    use repeat <- decode.field("repeat", schedule.repeat_decoder())
    use due <- decode.field("due", decode.int)
    use read <- decode.field("read", decode.bool)
    decode.success(MissedReview(schedule:, kinds:, repeat:, due:, read:))
  })
}

/// The route answering schedule `id`'s missed review with `action`, "run" or "skip".
pub fn missed_path(id: String, action: String) -> String {
  "/system-reviews/missed/" <> id <> "/" <> action
}

/// "Weekly security review was due <date and time> and didn't run."
pub fn missed_text(missed: MissedReview) -> String {
  let repeat = case missed.repeat {
    schedule.Once -> "Scheduled"
    repeat -> schedule.repeat_label(repeat)
  }
  repeat
  <> " "
  <> string.lowercase(kind_ids_text(missed.kinds))
  <> " review was due "
  <> clock(missed.due)
  <> " and didn't run."
}

/// The missed-review banner's styles, for each page that shows it.
pub const missed_styles =
  ".missed-reviews{margin:var(--space-3) var(--space-6) 0;background:#fff4c2;color:#172226;border:1px solid #8a4a0a;border-left:6px solid #8a4a0a;padding:var(--space-2) var(--space-3)}
.missed-reviews ul{list-style:none;margin:0;padding:0;display:grid;gap:var(--space-2)}
.missed-review{display:flex;flex-wrap:wrap;gap:var(--space-2);align-items:center}
.missed-review span{flex:1 1 20em}
.missed-error{margin:var(--space-2) 0 0;font-weight:600}
"

/// Answers schedule `id`'s missed review with `action`, unless an answer is on its way.
fn answer(
  model: Model,
  id: String,
  action: String,
) -> #(Model, effect.Effect(Msg)) {
  case model.answering {
    True -> #(model, effect.none())
    False -> #(
      Model(..model, answering: True, missed_error: ""),
      post(missed_path(id, action), "{}", MissedAnswered),
    )
  }
}

/// The model holding the missed reviews a reply lists, or its refusal.
fn missed_listed(model: Model, text: String) -> #(Model, effect.Effect(Msg)) {
  case json.parse(text, missed_decoder()) {
    Ok(missed) -> heard(Model(..model, missed_error: ""), missed)
    Error(_) -> #(Model(..model, missed_error: refusal(text)), effect.none())
  }
}

fn message_decoder() -> decode.Decoder(String) {
  decode.field("message", decode.string, decode.success)
}

/// A reply as the page reads it: an error reply without a message becomes a
/// refusal naming its status, so it never reads as a success. A request that
/// got no reply has status 0 and its own message.
pub fn reply_text(status: Int, text: String) -> String {
  case status >= 200 && status < 300, json.parse(text, message_decoder()) {
    True, _ -> text
    False, Ok(message) if message != "" -> text
    False, _ ->
      json.to_string(
        json.object([
          #("message", json.string("HTTP " <> int.to_string(status))),
        ]),
      )
  }
}

/// The missed reviews above a page's tabs, each with **Run now** and **Skip**;
/// none while there are none. Both wait while an answer is on its way.
pub fn missed_notice(
  missed: List(MissedReview),
  error: String,
  answering: Bool,
  run: fn(String) -> msg,
  skip: fn(String) -> msg,
) -> element.Element(msg) {
  case missed, error {
    [], "" -> element.none()
    _, _ ->
      html.div([attribute.class("missed-reviews"), attribute.role("status")], [
        html.ul(
          [],
          list.map(missed, fn(m) {
            html.li([attribute.class("missed-review")], [
              html.span([], [html.text(missed_text(m))]),
              html.button(
                [
                  attribute.disabled(answering),
                  event.on_click(run(m.schedule)),
                ],
                [html.text("Run now")],
              ),
              html.button(
                [
                  attribute.disabled(answering),
                  event.on_click(skip(m.schedule)),
                ],
                [html.text("Skip")],
              ),
            ])
          }),
        ),
        case error {
          "" -> element.none()
          error ->
            html.p([attribute.class("missed-error"), attribute.role("alert")], [
              html.text(error),
            ])
        },
      ])
  }
}
