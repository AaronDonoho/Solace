//// The planning panel (D-93 point 7): one component docked beside every tab.
//// It starts a strategy run on what is selected, shows a run's conversation,
//// proposals and transcript with a message box, and lists the past runs on
//// the same record to reopen. The page keeps the state; this draws it.

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
pub type Question {
  Question(
    id: String,
    text: String,
    options: List(String),
    answer: String,
    proposal: Option(solace_task.Proposal),
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
  decode.success(Question(id, text, options, answer, proposal))
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
      attribute.title("A planning run is open on this"),
    ],
    [html.text("Planning")],
  )
}

/// What a run is doing, or how it ended, and its model.
pub fn state_text(run: Run) -> String {
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
  <> run.model
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
    on_model: fn(String) -> msg,
    on_effort: fn(String) -> msg,
    on_start: msg,
    on_send: msg,
    /// A question's id and the option chosen.
    on_answer: fn(String, String) -> msg,
    /// Shows the run with this id, "" for none.
    on_show: fn(String) -> msg,
    on_end: msg,
    on_stop: msg,
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
      attribute.attribute("aria-label", "Planning"),
      attribute.id("plan"),
      attribute.style("--plan-width", int.to_string(p.width) <> "px"),
    ],
    [
      case p.collapsed {
        True -> element.none()
        False -> resize_handle(p)
      },
      html.header([], [
        html.h2([], [html.text("Plan")]),
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
              True -> "Show"
              False -> "Hide"
            }),
          ],
        ),
      ]),
      case p.collapsed {
        True ->
          html.p([attribute.class("plan-summary")], [html.text(summary(p))])
        False -> html.div([attribute.class("plan-body")], body(base, p))
      },
    ],
  )
}

/// The collapsed panel's one line.
fn summary(p: Panel(msg)) -> String {
  case p.shown, open_on(p.runs, p.context) {
    Some(run), _ -> state_text(run)
    None, Some(_) -> "A run on this is open"
    None, None -> "No run open"
  }
}

fn context_view(context: Context, name: fn(String, String) -> String) {
  case context.kind {
    "page" -> [html.text("Planning Context: Whole Project")]
    kind -> [
      html.text("Planning Context: " <> kind <> " "),
      html.em([], [
        html.text(case name(kind, context.id) {
          "" -> context.id
          title -> title
        }),
      ]),
    ]
  }
}

fn body(base: String, p: Panel(msg)) -> List(Element(msg)) {
  list.flatten([
    [
      case p.error {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      },
    ],
    case p.shown {
      Some(run) -> run_view(base, p, run)
      None -> start_view(p)
    },
    past_view(p),
  ])
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

/// The open run: what it is on and doing, its conversation and proposals, the
/// agent's transcript, then the message box with End and Stop while it is
/// open. An ended run is read-only.
fn run_view(base: String, p: Panel(msg), run: Run) -> List(Element(msg)) {
  let open = run.state != "ended"
  list.flatten([
    [
      html.p(
        [attribute.class("plan-context")],
        context_view(run.context, p.name),
      ),
      html.p([attribute.class("plan-meta")], [
        html.span(
          [
            attribute.class(case run.state {
              "running" -> "state working"
              _ -> "state"
            }),
          ],
          [html.text(state_text(run))],
        ),
        small("Close", False, p.on_show("")),
      ]),
    ],
    list.map(run.notes, fn(n) {
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
    }),
    list.map(run.questions, fn(q) {
      case q.proposal {
        Some(proposal) ->
          solace_task.proposal_view(
            base,
            proposal,
            q.options,
            q.answer,
            p.name,
            p.sending || !open,
            p.on_answer(q.id, _),
          )
        None ->
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
    case p.entries {
      [] -> []
      entries -> [
        html.details([attribute.class("plan-transcript")], [
          html.summary([], [html.text("Agent transcript")]),
          ..list.map(entries, solace_task.entry_view(base, _))
        ]),
      ]
    },
    case open {
      False -> []
      True -> [
        html.label([attribute.for(text_id)], [html.text("Message")]),
        html.textarea(
          [
            attribute.id(text_id),
            attribute.value(p.text),
            event.on_input(p.on_text),
          ],
          p.text,
        ),
        html.div([attribute.class("vision-actions")], [
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(p.sending || string.trim(p.text) == ""),
              event.on_click(p.on_send),
            ],
            [html.text("Send")],
          ),
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(p.sending),
              event.on_click(p.on_end),
            ],
            [html.text("End")],
          ),
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(p.sending),
              event.on_click(p.on_stop),
            ],
            [html.text("Stop")],
          ),
        ]),
      ]
    },
  ])
}

/// Nothing shown: the open run on the context to open, or the form that
/// starts one with a first message.
fn start_view(p: Panel(msg)) -> List(Element(msg)) {
  let context =
    html.p([attribute.class("plan-context")], context_view(p.context, p.name))
  case open_on(p.runs, p.context) {
    Some(run) -> [
      context,
      html.p([], [html.text("A planning run on this is open.")]),
      small("Open it", False, p.on_show(run.id)),
    ]
    None -> [
      context,
      agent_choice(p),
      html.label([attribute.for(text_id)], [html.text("First message")]),
      html.textarea(
        [
          attribute.id(text_id),
          attribute.value(p.text),
          attribute.placeholder("What should we plan?"),
          event.on_input(p.on_text),
        ],
        p.text,
      ),
      html.div([attribute.class("vision-actions")], [
        html.button(
          [
            attribute.type_("button"),
            attribute.disabled(p.sending || string.trim(p.text) == ""),
            event.on_click(p.on_start),
          ],
          [html.text("Start")],
        ),
      ]),
    ]
  }
}

/// The model, and its effort when it offers several; unavailable models
/// show why and can't be picked.
fn agent_choice(p: Panel(msg)) -> Element(msg) {
  case p.models {
    [] -> element.none()
    models ->
      html.div([attribute.class("filters")], [
        html.label([], [
          html.text("Model"),
          html.select(
            [attribute.value(p.model), event.on_input(p.on_model)],
            list.map(models, fn(m) {
              html.option(
                [
                  attribute.value(m.id),
                  attribute.selected(m.id == p.model),
                  attribute.disabled(!m.available),
                ],
                case m.available {
                  True -> m.label
                  False -> m.label <> " (unavailable: " <> m.reason <> ")"
                },
              )
            }),
          ),
        ]),
        case list.find(models, fn(m) { m.id == p.model }) {
          Ok(m) if m.efforts != [] ->
            html.label([], [
              html.text("Effort"),
              html.select(
                [attribute.value(p.effort), event.on_input(p.on_effort)],
                [
                  html.option(
                    [attribute.value(""), attribute.selected(p.effort == "")],
                    case m.effort {
                      "" -> "Default"
                      effort -> "Default (" <> effort <> ")"
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
              ),
            ])
          _ -> element.none()
        },
      ])
  }
}

/// The runs on the context, newest first, each reopened by its button.
fn past_view(p: Panel(msg)) -> List(Element(msg)) {
  case past(p.runs, p.context) {
    [] -> []
    runs -> [
      html.h3([], [html.text("Runs on this")]),
      html.ul(
        [attribute.class("plan-runs")],
        list.map(runs, fn(run) {
          let current = case p.shown {
            Some(shown) -> shown.id == run.id
            None -> False
          }
          html.li([], [
            html.button(
              [
                attribute.type_("button"),
                attribute.class("open"),
                case current {
                  True -> attribute.attribute("aria-current", "true")
                  False -> attribute.none()
                },
                event.on_click(p.on_show(run.id)),
              ],
              [html.text(clock(run.started) <> " · " <> state_text(run))],
            ),
          ])
        }),
      ),
    ]
  }
}

/// The panel's look; colours come from the page's custom properties.
pub const styles =
  ".plan{grid-column:2;grid-row:2;position:relative;display:flex;flex-direction:column;min-height:0;box-sizing:border-box;width:clamp(280px,var(--plan-width,380px),60vw);background:var(--paper);border-left:2px solid var(--edge);padding:12px 16px 0}.plan.collapsed{width:auto;min-width:160px;padding-bottom:12px}
.plan>header{flex:none}.plan-body{flex:1;min-height:0;overflow:auto;overscroll-behavior:contain;margin:0 -16px;padding:0 16px 12px}
.plan-resize{position:absolute;left:-7px;top:0;bottom:0;width:12px;cursor:col-resize;touch-action:none;z-index:1}.plan-resize:hover{background:var(--line);opacity:.6}.plan .plan-resize:focus-visible{outline:3px solid var(--focus);outline-offset:-3px}
.plan header{display:flex;align-items:center;gap:8px}.plan h2{margin:0;font-size:17px;flex:1}.plan h3{font-size:14px;margin:16px 0 4px}
.plan p{margin:6px 0}.plan .plan-summary,.plan .plan-context{color:var(--soft);font-size:13px}.plan-meta{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.plan .edit,.plan .vision-actions button{font:inherit;font-size:13px;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:2px 10px;cursor:pointer}.plan .edit:hover,.plan .vision-actions button:hover{background:var(--hover)}.plan button:disabled{color:var(--soft);cursor:default}
.plan label{display:block;font-size:13px;color:var(--soft);margin:8px 0 2px}.plan .filters label{margin:0}.plan .filters select{max-width:100%}
.plan textarea{display:block;box-sizing:border-box;width:100%;min-height:90px;font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--edge);border-radius:0;padding:6px 8px;resize:vertical}
.plan :focus-visible{outline:3px solid var(--focus);outline-offset:2px}
.plan-note{border-top:1px solid var(--line);padding:6px 0}.plan-note.human strong{color:var(--focus)}.plan-note .md p{color:var(--ink)}
.plan-transcript{margin:8px 0;border-left:1px solid var(--edge);padding-left:8px}.plan-transcript>summary{cursor:pointer;color:var(--soft)}.plan-transcript pre{background:var(--code);padding:6px;white-space:pre-wrap;overflow:auto;max-height:240px;font:12px 'IBM Plex Mono',ui-monospace,monospace}.plan-transcript .entry{margin:4px 0}.plan-transcript .tool.ok .icon{color:var(--ok-text)}.plan-transcript .tool.failed .icon{color:var(--revision)}
.plan-runs{list-style:none;margin:0;padding:0}.plan-runs li{margin:2px 0}.plan-runs .open{font:inherit;font-size:13px;color:var(--ink);background:none;border:0;padding:0;text-align:left;cursor:pointer}.plan-runs .open:hover{text-decoration:underline}.plan-runs [aria-current=true]{font-weight:700}
.working::before,.planning::before{content:\"\";display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--entry);margin-right:6px;animation:pulse 1.2s ease-in-out infinite}@keyframes pulse{50%{opacity:.25}}@media (prefers-reduced-motion:reduce){.working::before,.planning::before{animation:none}}
.planning{font-size:11px;font-weight:400;text-transform:uppercase;letter-spacing:.08em;color:var(--ok-text);border:1px solid var(--ok-text);padding:1px 6px;margin-left:6px;white-space:nowrap}
@media (max-width:900px){.plan{display:block;width:auto;border-left:0;border-top:2px solid var(--edge);padding:12px 16px}.plan-body{overflow:visible;margin:0;padding:0}.plan-resize{display:none}}
"
  <> solace_task.proposal_styles

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
