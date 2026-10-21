//// The planning panel (D-93 point 7): one component docked beside every tab.
//// It starts a strategy run on what is selected, shows a run's conversation,
//// proposals and transcript with a message box, and lists the past runs on
//// the same record to reopen. The page keeps the state; this draws it.

import gleam/dynamic
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/string
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import solace_task

/// What a run is on: the page (`id` ""), or a task, feature or milestone.
pub type Context {
  Context(kind: String, id: String)
}

pub const page = Context("page", "")

/// A run's question: each is a proposal (Spec 15 §6), answered with an option.
/// `time` is when it was asked (D-118), 0 for one saved before it was kept.
/// `after` is how many of the run's notes came before it, which orders it
/// among notes of the same second; -1 for one saved before it was kept.
pub type Question {
  Question(
    id: String,
    text: String,
    options: List(String),
    answer: String,
    proposal: Option(solace_task.Proposal),
    time: Int,
    after: Int,
  )
}

/// A strategy run (D-93). The list the stream replays holds no notes.
pub type Run {
  Run(
    id: String,
    model: String,
    effort: String,
    /// "running", "waiting" or "ended".
    state: String,
    started: Int,
    ended: Int,
    /// "planned" or "stopped" once ended.
    outcome: String,
    context: Context,
    notes: List(solace_task.Note),
    questions: List(Question),
    /// The model the agent's start event named, when it named one: an alias
    /// shows the model the run used.
    used: String,
  )
}

@external(javascript, "./api.mjs", "clock")
fn clock(seconds: Int) -> String

pub fn context_decoder() -> decode.Decoder(Context) {
  use kind <- decode.field("kind", decode.string)
  use id <- decode.optional_field(
    "id",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  decode.success(Context(kind, id))
}

fn question_decoder() -> decode.Decoder(Question) {
  use id <- decode.field("id", decode.string)
  use text <- decode.field("text", decode.string)
  use options <- decode.field("options", decode.list(decode.string))
  use answer <- decode.optional_field(
    "answer",
    "",
    decode.one_of(decode.string, [decode.success("")]),
  )
  use proposal <- decode.optional_field(
    "proposal",
    None,
    decode.optional(solace_task.proposal_decoder()),
  )
  use time <- decode.optional_field(
    "time",
    0,
    decode.one_of(decode.int, [decode.success(0)]),
  )
  use after <- decode.optional_field(
    "after",
    -1,
    decode.one_of(decode.int, [decode.success(-1)]),
  )
  decode.success(Question(id, text, options, answer, proposal, time, after))
}

/// A run in `GET /strategy/runs...`, a reply or the stream.
pub fn decoder() -> decode.Decoder(Run) {
  let text = fn(name, next) {
    decode.optional_field(
      name,
      "",
      decode.one_of(decode.string, [decode.success("")]),
      next,
    )
  }
  let time = fn(name, next) {
    decode.optional_field(
      name,
      0,
      decode.one_of(decode.int, [decode.success(0)]),
      next,
    )
  }
  use id <- decode.field("id", decode.string)
  use model <- text("model")
  use effort <- text("effort")
  use state <- decode.field("state", decode.string)
  use started <- time("started")
  use ended <- time("ended")
  use outcome <- text("outcome")
  use context <- decode.optional_field("context", page, context_decoder())
  use notes <- decode.optional_field(
    "notes",
    [],
    decode.list(solace_task.note_decoder()),
  )
  use questions <- decode.optional_field(
    "questions",
    [],
    decode.list(question_decoder()),
  )
  use used <- text("used")
  decode.success(Run(
    id,
    model,
    effort,
    state,
    started,
    ended,
    outcome,
    context,
    notes,
    questions,
    used,
  ))
}

pub fn context_json(context: Context) -> json.Json {
  case context.id {
    "" -> json.object([#("kind", json.string(context.kind))])
    id ->
      json.object([
        #("kind", json.string(context.kind)),
        #("id", json.string(id)),
      ])
  }
}

/// The body of `POST /strategy/runs`: `text` is the first message. With no
/// model chosen the server takes its default.
pub fn start_body(
  model: String,
  effort: String,
  text: String,
  context: Context,
) -> json.Json {
  json.object(
    list.flatten([
      case model {
        "" -> []
        m -> [#("model", json.string(m))]
      },
      case effort {
        "" -> []
        e -> [#("effort", json.string(e))]
      },
      [#("text", json.string(text)), #("context", context_json(context))],
    ]),
  )
}

/// The open run on `context`, if there is one: the server allows one.
pub fn open_on(runs: List(Run), context: Context) -> Option(Run) {
  list.find(runs, fn(r) { r.state != "ended" && r.context == context })
  |> option.from_result
}

/// The ids of the records of `kind` with an open run, which show a marker.
pub fn open_ids(runs: List(Run), kind: String) -> List(String) {
  runs
  |> list.filter(fn(r) { r.state != "ended" && r.context.kind == kind })
  |> list.map(fn(r) { r.context.id })
}

/// The runs on `context`, newest first.
pub fn past(runs: List(Run), context: Context) -> List(Run) {
  runs
  |> list.filter(fn(r) { r.context == context })
  |> list.sort(fn(a, b) {
    int.compare(b.started, a.started)
    |> order.lazy_break_tie(fn() { string.compare(b.id, a.id) })
  })
}

/// The badge on a task row, feature or milestone with an open run.
pub fn marker() -> Element(msg) {
  html.span(
    [
      attribute.class("planning"),
      attribute.title(solace_task.ui_label("plan_marker")),
    ],
    [html.text(solace_task.ui_label("plan"))],
  )
}

/// What a run is doing, or how it ended, and the one label of the model its
/// agent used.
pub fn state_text(run: Run, models: List(solace_task.Agent)) -> String {
  case run.state {
    "running" -> "Working"
    "waiting" -> "Waiting for you"
    _ ->
      case run.outcome {
        "" -> "Ended"
        outcome -> "Ended: " <> outcome
      }
  }
  <> " · "
  <> solace_task.run_model_label(models, run.model, run.used)
}

/// What the panel shows and what it sends. `shown` is the run open in it;
/// `name(kind, id)` is a record's title, "" when not loaded.
pub type Panel(msg) {
  Panel(
    runs: List(Run),
    /// What Start plans: what is selected, else the page.
    context: Context,
    shown: Option(Run),
    entries: List(solace_task.Entry),
    collapsed: Bool,
    /// The panel's width in pixels, and the widest the window allows.
    width: Int,
    max_width: Int,
    /// The message box.
    text: String,
    /// A request sent and not yet answered.
    sending: Bool,
    error: String,
    models: List(solace_task.Agent),
    model: String,
    effort: String,
    name: fn(String, String) -> String,
    on_collapse: msg,
    /// A press on the resize handle, at this clientX.
    on_drag: fn(Int) -> msg,
    /// A resize key on the focused handle: the arrows, Home or End.
    on_key: fn(String) -> msg,
    on_text: fn(String) -> msg,
    /// A formatting tool: the markup before and after the selection.
    on_format: fn(String, String) -> msg,
    /// A block tool: "heading", "list", "numbered", "quote" or "code".
    on_block: fn(String) -> msg,
    on_model: fn(String) -> msg,
    on_effort: fn(String) -> msg,
    /// A usage limit's Try again: the agent kind whose limit to clear.
    on_try_again: fn(String) -> msg,
    on_start: msg,
    on_send: msg,
    /// A question's id and the option chosen.
    on_answer: fn(String, String) -> msg,
    /// Shows the run with this id, "" for none.
    on_show: fn(String) -> msg,
    on_end: msg,
    on_stop: msg,
    /// Whether the run's end is in reach in the panel, so a new message
    /// scrolls to it (D-118).
    at_bottom: Bool,
    /// A message arrived while the human had scrolled up to read.
    new_message: Bool,
    /// A scroll: whether the run's end is now in reach.
    on_scroll: fn(Bool) -> msg,
    /// "New message ↓": scrolls to the newest.
    on_newest: msg,
  )
}

/// The message box's textarea.
pub const text_id = "plan-text"

pub fn view(base: String, p: Panel(msg)) -> Element(msg) {
  html.aside(
    [
      attribute.class(case p.collapsed {
        True -> "plan collapsed"
        False -> "plan"
      }),
      attribute.attribute("aria-label", solace_task.ui_label("plan")),
      attribute.id("plan"),
      attribute.style("--plan-width", int.to_string(p.width) <> "px"),
    ],
    [
      case p.collapsed {
        True -> element.none()
        False -> resize_handle(p)
      },
      html.header([], [
        html.h2([], [html.text(solace_task.ui_label("plan"))]),
        html.button(
          [
            attribute.type_("button"),
            attribute.class("edit"),
            attribute.attribute("aria-expanded", case p.collapsed {
              True -> "false"
              False -> "true"
            }),
            event.on_click(p.on_collapse),
          ],
          [
            html.text(case p.collapsed {
              True -> solace_task.ui_label("show")
              False -> solace_task.ui_label("hide")
            }),
          ],
        ),
      ]),
      case p.collapsed {
        True -> element.none()
        False -> bar(p)
      },
      case p.collapsed {
        True ->
          html.p([attribute.class("plan-summary")], [html.text(summary(p))])
        False ->
          html.div(
            [attribute.class("plan-body"), event.on("scroll", scrolled(p))],
            body(base, p),
          )
      },
      case p.collapsed {
        True -> element.none()
        False -> compose(base, p)
      },
    ],
  )
}

/// The collapsed panel's one line.
fn summary(p: Panel(msg)) -> String {
  case p.shown, open_on(p.runs, p.context) {
    Some(run), _ -> state_text(run, p.models)
    None, Some(_) -> "A run on this is open"
    None, None -> "No run open"
  }
}

fn context_view(context: Context, name: fn(String, String) -> String) {
  case context.kind {
    "page" -> [
      html.text(solace_task.ui_label("plan_context") <> ": Whole Project"),
    ]
    kind -> [
      html.text(solace_task.ui_label("plan_context") <> ": " <> kind <> " "),
      html.em([], [
        html.text(case name(kind, context.id) {
          "" -> context.id
          title -> title
        }),
      ]),
    ]
  }
}

/// The conversation, between the run bar and the message box: the only part
/// of the panel that scrolls.
fn body(base: String, p: Panel(msg)) -> List(Element(msg)) {
  case p.shown {
    Some(run) -> run_view(base, p, run)
    None -> start_view(p)
  }
}

fn small(label: String, disabled: Bool, msg: msg) -> Element(msg) {
  html.button(
    [
      attribute.type_("button"),
      attribute.class("edit"),
      attribute.disabled(disabled),
      event.on_click(msg),
    ],
    [html.text(label)],
  )
}

/// The shown run's conversation: the agent's transcript (collapsed), then its
/// notes and proposals in the order they happened (D-118). The newest message
/// sits at the run's end, right above the message box. An ended run's
/// proposals can't be answered.
fn run_view(base: String, p: Panel(msg), run: Run) -> List(Element(msg)) {
  let open = run.state != "ended"
  list.flatten([
    case p.entries {
      [] -> []
      entries -> [
        html.details([attribute.class("plan-transcript")], [
          html.summary([], [html.text("Agent transcript")]),
          ..list.map(entries, solace_task.entry_view(base, _))
        ]),
      ]
    },
    list.map(conversation(run), fn(said) {
      case said {
        Wrote(n) ->
          html.article([attribute.class("plan-note " <> n.author)], [
            html.strong([], [
              html.text(case n.author {
                "human" -> "You"
                "solace" -> "Solace"
                _ -> "Agent"
              }),
            ]),
            solace_task.md(base, n.text),
          ])
        Asked(Question(proposal: Some(proposal), ..) as q) ->
          solace_task.proposal_view(
            base,
            proposal,
            q.options,
            q.answer,
            p.name,
            p.sending || !open,
            p.on_answer(q.id, _),
          )
        Asked(q) ->
          html.article([attribute.class("proposal")], [
            solace_task.md(base, q.text),
            case q.answer {
              "" ->
                html.div(
                  [attribute.class("actions")],
                  list.map(q.options, fn(o) {
                    small(o, p.sending || !open, p.on_answer(q.id, o))
                  }),
                )
              answer -> html.p([], [html.text("Answer: " <> answer)])
            },
          ])
      }
    }),
    [
      case p.new_message {
        True ->
          html.button(
            [
              attribute.type_("button"),
              attribute.class("plan-new"),
              event.on_click(p.on_newest),
            ],
            [html.text("New message ↓")],
          )
        False -> element.none()
      },
      html.div([attribute.class("plan-end")], []),
    ],
  ])
}

/// Nothing shown: the open run on the context to open, or a hint that the
/// message box below starts a run.
fn start_view(p: Panel(msg)) -> List(Element(msg)) {
  case open_on(p.runs, p.context) {
    Some(run) -> [
      html.p([], [html.text(solace_task.ui_label("plan_open_here"))]),
      small(solace_task.ui_label("open_run"), False, p.on_show(run.id)),
    ]
    // Nothing open: the box below starts the run, and its Start button says so.
    None -> []
  }
}

/// The model, and its effort when it offers several. Only models that can
/// start are offered; a line under the pick says why the others can't, and
/// with none that can, what to set up. A pick shows what is chosen, so it
/// names itself; while none is, a placeholder says what to pick.
fn agent_choice(p: Panel(msg)) -> Element(msg) {
  let ready = list.filter(p.models, fn(m) { m.available })
  case p.models, ready {
    [], _ -> element.none()
    _, [] ->
      html.div([attribute.class("filters")], [
        solace_task.no_agents_view(p.models),
        solace_task.unavailable_view(p.models, p.on_try_again),
      ])
    _, models ->
      html.div([attribute.class("filters")], [
        html.select(
          [
            attribute.value(p.model),
            attribute.attribute("aria-label", solace_task.ui_label("model")),
            event.on_input(p.on_model),
          ],
          list.append(
            solace_task.placeholder_option(
              solace_task.ui_label("pick_model"),
              p.model,
            ),
            list.map(models, fn(m) {
              html.option(
                [attribute.value(m.id), attribute.selected(m.id == p.model)],
                solace_task.agent_option(m),
              )
            }),
          ),
        ),
        case list.find(models, fn(m) { m.id == p.model }) {
          Ok(m) if m.efforts != [] ->
            html.select(
              [
                attribute.value(p.effort),
                attribute.attribute(
                  "aria-label",
                  solace_task.ui_label("effort"),
                ),
                event.on_input(p.on_effort),
              ],
              [
                html.option(
                  [attribute.value(""), attribute.selected(p.effort == "")],
                  case m.effort {
                    "" -> solace_task.ui_label("default_effort")
                    effort ->
                      solace_task.ui_label("default_effort")
                      <> " ("
                      <> effort
                      <> ")"
                  },
                ),
                ..list.map(m.efforts, fn(effort) {
                  html.option(
                    [
                      attribute.value(effort),
                      attribute.selected(effort == p.effort),
                    ],
                    effort,
                  )
                })
              ],
            )
          _ -> element.none()
        },
        solace_task.unavailable_view(p.models, p.on_try_again),
      ])
  }
}

/// The panel's look; colors come from the page's custom properties.
pub const styles =
  ".plan{grid-column:2;grid-row:2;position:relative;display:flex;flex-direction:column;min-height:0;box-sizing:border-box;width:clamp(280px,var(--plan-width,380px),60vw);background:var(--paper);border-left:2px solid var(--edge);padding:12px 16px 0}.plan.collapsed{width:auto;min-width:160px;padding-bottom:12px}
.plan>header,.plan-bar,.plan-compose{flex:none}.plan-body{flex:1;min-height:0;overflow:auto;overscroll-behavior:contain;margin:0 -16px;padding:0 16px 12px}
.plan-bar{border-bottom:1px solid var(--edge);padding:0 0 8px;margin-bottom:4px}.plan-bar .plan-context{margin:6px 0 4px}.plan-bar select{"
  <> solace_task.small_control
  <> ";padding:2px 6px;max-width:100%}
.plan-compose{border-top:1px solid var(--edge);padding:8px 0 12px}.plan-compose .filters{margin:0 0 8px}.plan-compose .vision-actions{margin:8px 0 0}
.plan .composer{margin:0}.plan .composer solace-md-editor{--md-min-height:5lh;--md-max-height:40dvh}
.plan-resize{position:absolute;left:-7px;top:0;bottom:0;width:12px;cursor:col-resize;touch-action:none;z-index:1}.plan-resize:hover{background:var(--line);opacity:.6}.plan .plan-resize:focus-visible{outline:3px solid var(--focus);outline-offset:-3px}
.plan header{display:flex;align-items:center;gap:8px}.plan h2{margin:0;font-size:17px;flex:1}.plan h3{font-size:14px;margin:16px 0 4px}
.plan p{margin:6px 0}.plan .plan-summary,.plan .plan-context{color:var(--soft);font-size:13px}.plan-meta{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.plan .edit,.plan .vision-actions button{"
  <> solace_task.small_control
  <> ";padding:0 10px}.plan .edit:hover,.plan .vision-actions button:hover{background:var(--hover)}.plan button:disabled{color:var(--soft);cursor:default}
.plan label{display:block;font-size:13px;color:var(--soft);margin:8px 0 2px}.plan .filters label{margin:0}.plan .filters select{max-width:100%}
.plan :focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.plan-note{border-top:1px solid var(--line);padding:6px 0}.plan-note.human strong{color:var(--focus)}.plan-note .md p{color:var(--ink)}
.plan-transcript{margin:8px 0;border-left:1px solid var(--edge);padding-left:8px}.plan-transcript>summary{cursor:pointer;color:var(--soft)}.plan-transcript pre{background:var(--code);padding:6px;white-space:pre-wrap;overflow:auto;max-height:240px;font:12px 'IBM Plex Mono',ui-monospace,monospace}.plan-transcript .entry{margin:4px 0}.plan-transcript .tool.ok .icon{color:var(--ok-text)}.plan-transcript .tool.failed .icon{color:var(--revision)}
.working::before,.planning::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--entry);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before,.planning::before{animation:none}}
.planning{font-size:11px;font-weight:400;text-transform:uppercase;letter-spacing:.08em;color:var(--ok-text);border:1px solid var(--ok-text);padding:1px 6px;margin-left:6px;white-space:nowrap}
.plan .plan-new{position:sticky;bottom:8px;display:block;margin:8px 0 0 auto;font:inherit;font-size:13px;color:var(--paper);background:var(--focus);border:1px solid var(--focus);border-radius:0;padding:4px 12px;cursor:pointer;z-index:1}
@media (max-width:900px){.plan{display:block;width:auto;border-left:0;border-top:2px solid var(--edge);padding:12px 16px}.plan-body{overflow:visible;margin:0;padding:0}.plan-resize{display:none}}
"
  <> solace_task.proposal_styles
  <> solace_task.model_styles

/// The narrowest the panel gets, in pixels.
pub const min_width = 280

/// The panel's width until one is chosen.
pub const default_width = 380

/// How much an arrow key on the resize handle changes the width.
pub const key_step = 16

/// The widest the panel gets in a `window` this wide: 60% of it, never under
/// `min_width`.
pub fn max_width(window: Int) -> Int {
  int.max(window * 60 / 100, min_width)
}

/// `width` kept between `min_width` and `max_width(window)`; a `window` of 0
/// (no page to measure) sets no upper bound.
pub fn clamp(width: Int, window: Int) -> Int {
  case window {
    0 -> int.max(width, min_width)
    _ -> int.clamp(width, min_width, max_width(window))
  }
}

const resize_keys = ["ArrowLeft", "ArrowRight", "Home", "End"]

/// The panel's left edge, a separator that sets its width: dragged, or with
/// the arrow keys, Home and End while focused. Left widens the panel.
fn resize_handle(p: Panel(msg)) -> Element(msg) {
  html.div(
    [
      attribute.class("plan-resize"),
      attribute.role("separator"),
      attribute.attribute("aria-orientation", "vertical"),
      attribute.attribute("aria-label", "Resize the planning panel"),
      attribute.attribute("aria-controls", "plan"),
      attribute.attribute("aria-valuenow", int.to_string(p.width)),
      attribute.attribute("aria-valuemin", int.to_string(min_width)),
      attribute.attribute("aria-valuemax", int.to_string(p.max_width)),
      attribute.tabindex(0),
      event.prevent_default(
        event.on("pointerdown", {
          use x <- decode.field(
            "clientX",
            decode.one_of(decode.int, [decode.map(decode.float, float.round)]),
          )
          decode.success(p.on_drag(x))
        }),
      ),
      // Other keys, Tab among them, keep their default.
      event.advanced("keydown", {
        use key <- decode.field("key", decode.string)
        let handler =
          event.handler(
            p.on_key(key),
            prevent_default: True,
            stop_propagation: False,
          )
        case list.contains(resize_keys, key) {
          True -> decode.success(handler)
          False -> decode.failure(handler, "a resize key")
        }
      }),
    ],
    [],
  )
}

/// One message of a run's conversation: a note, or a question.
type Said {
  Wrote(solace_task.Note)
  Asked(Question)
}

/// The run's notes and questions as one conversation, in the order they
/// happened (D-118). In the same second, a question comes after the notes the
/// server had when it was asked, and before the rest; one saved without that
/// count comes after the notes. A question saved without a time comes after
/// the notes, in the questions' order.
fn conversation(run: Run) -> List(Said) {
  let #(timed, untimed) = list.partition(run.questions, fn(q) { q.time > 0 })
  let notes = list.length(run.notes)
  list.append(
    list.index_map(run.notes, fn(n, i) { #(n.time, i, 1, Wrote(n)) }),
    list.map(timed, fn(q) {
      let place = case q.after >= 0 {
        True -> q.after
        False -> notes
      }
      #(q.time, place, 0, Asked(q))
    }),
  )
  |> list.index_map(fn(said, i) { #(said, i) })
  |> list.sort(fn(a, b) {
    let #(#(time_a, place_a, kind_a, _), i) = a
    let #(#(time_b, place_b, kind_b, _), j) = b
    int.compare(time_a, time_b)
    |> order.break_tie(int.compare(place_a, place_b))
    |> order.break_tie(int.compare(kind_a, kind_b))
    |> order.break_tie(int.compare(i, j))
  })
  |> list.map(fn(said) { said.0.3 })
  |> list.append(list.map(untimed, Asked))
}

/// How many messages a run's conversation holds: its notes and questions.
pub fn size(run: Run) -> Int {
  list.length(run.notes) + list.length(run.questions)
}

/// A scroll of the panel: whether the run's end is now in reach, sent only
/// when that changes.
fn scrolled(p: Panel(msg)) -> decode.Decoder(msg) {
  use body <- decode.field("target", decode.dynamic)
  let near = near_end(body)
  case near == p.at_bottom {
    True -> decode.failure(p.on_scroll(near), "no change")
    False -> decode.success(p.on_scroll(near))
  }
}

@external(javascript, "./api.mjs", "near_end")
fn near_end(body: dynamic.Dynamic) -> Bool

/// The run New run shows: the open run on `context`, since the server allows
/// one, else "" for the form that starts one.
pub fn new_run(runs: List(Run), context: Context) -> String {
  case open_on(runs, context) {
    Some(run) -> run.id
    None -> ""
  }
}

/// The run bar, pinned above the conversation: what the shown run is on (else
/// what a new run would plan), its state and model, End and Stop while it is
/// open, the Earlier runs on the same record, newest first, and New run.
fn bar(p: Panel(msg)) -> Element(msg) {
  let current = case p.shown {
    Some(run) -> run.id
    None -> ""
  }
  let context = case p.shown {
    Some(run) -> run.context
    None -> p.context
  }
  let #(class, state) = case p.shown, open_on(p.runs, p.context) {
    Some(Run(state: "running", ..) as run), _ -> #(
      "state working",
      state_text(run, p.models),
    )
    Some(run), _ -> #("state", state_text(run, p.models))
    None, Some(_) -> #("state", "A run on this is open")
    None, None -> #("state", "No run open")
  }
  let target = new_run(p.runs, p.context)
  html.div([attribute.class("plan-bar")], [
    html.p([attribute.class("plan-context")], context_view(context, p.name)),
    html.div(
      [attribute.class("plan-meta")],
      list.flatten([
        [html.span([attribute.class(class)], [html.text(state)])],
        case p.shown {
          Some(run) if run.state != "ended" -> [
            small(solace_task.ui_label("end"), p.sending, p.on_end),
            small(solace_task.ui_label("stop"), p.sending, p.on_stop),
          ]
          _ -> []
        },
        case past(p.runs, context) {
          [] -> []
          runs -> [
            html.select(
              [
                attribute.class("plan-earlier"),
                attribute.attribute("aria-label", "Earlier runs"),
                event.on_input(p.on_show),
              ],
              [
                html.option(
                  [
                    attribute.value(""),
                    attribute.disabled(True),
                    attribute.selected(current == ""),
                  ],
                  "Earlier runs",
                ),
                ..list.map(runs, fn(run) {
                  html.option(
                    [
                      attribute.value(run.id),
                      attribute.selected(run.id == current),
                    ],
                    clock(run.started) <> " · " <> state_text(run, p.models),
                  )
                })
              ],
            ),
          ]
        },
        // Nothing to change when New run would show what is shown.
        [
          small(
            solace_task.ui_label("new_run"),
            target == current,
            p.on_show(target),
          ),
        ],
      ]),
    ),
    case p.error {
      "" -> element.none()
      message -> html.p([attribute.role("alert")], [html.text(message)])
    },
  ])
}

/// The message box, pinned below the conversation: the markdown editor and
/// its button, which Ctrl or Cmd+Enter presses too. For the shown open run
/// the button is Send and sends a message; with nothing shown and no open
/// run on the context, the model choice sits above it and the button is
/// Start, which starts a run. Otherwise there is none.
fn compose(base: String, p: Panel(msg)) -> Element(msg) {
  let box = fn(label, send, choice, text) {
    html.div(
      [attribute.class("plan-compose"), event.on("keydown", send_key(send))],
      [
        choice,
        solace_task.editor(
          text_id,
          "",
          base,
          label,
          p.text,
          p.on_text,
          p.on_format,
          p.on_block,
        ),
        html.div([attribute.class("vision-actions")], [
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(p.sending || string.trim(p.text) == ""),
              event.on_click(send),
            ],
            [html.text(text)],
          ),
        ]),
      ],
    )
  }
  case p.shown, open_on(p.runs, p.context) {
    Some(run), _ if run.state != "ended" ->
      box("Message", p.on_send, element.none(), solace_task.ui_label("send"))
    None, None ->
      box(
        "First message",
        p.on_start,
        agent_choice(p),
        solace_task.ui_label("start"),
      )
    _, _ -> element.none()
  }
}

/// Ctrl or Cmd+Enter in the message box: `send`. Other keys pass.
fn send_key(send: msg) -> decode.Decoder(msg) {
  use key <- decode.field("key", decode.string)
  use ctrl <- decode.field("ctrlKey", decode.bool)
  use meta <- decode.field("metaKey", decode.bool)
  case key == "Enter" && { ctrl || meta } {
    True -> decode.success(send)
    False -> decode.failure(send, "not a send")
  }
}
