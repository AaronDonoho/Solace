import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order
import gleam/result
import gleam/string
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import plan
import solace_task

/// A feature as the Features tab shows it (D-87). Only `id` is required.
pub type Feature {
  Feature(
    id: String,
    title: String,
    body: String,
    milestone: Option(String),
    rank: Float,
    state: String,
    /// The color slot the server kept when it first became `active`; None
    /// until then (D-114, `solace_task.feature_colors`).
    slot: Option(Int),
  )
}

/// A milestone: a planned release. Features and milestone pickers order
/// milestones by `rank`; the Roadmap orders them by `target_date`.
/// `target_date` is the `targetDate` the server keeps as written
/// ("2027-01-01").
pub type Milestone {
  Milestone(
    id: String,
    title: String,
    rank: Float,
    body: String,
    target_date: Option(String),
    state: String,
  )
}

/// A milestone (None for "No milestone") and its features in order.
pub type Group {
  Group(milestone: Option(Milestone), features: List(Feature))
}

/// A feature in the stream. A new record starts `proposed` (D-87).
pub fn decoder() -> decode.Decoder(Feature) {
  use id <- decode.field("id", decode.string)
  use title <- decode.optional_field("title", "", decode.string)
  use body <- decode.optional_field("body", "", decode.string)
  use milestone <- decode.optional_field(
    "milestone",
    None,
    decode.optional(decode.string),
  )
  use rank <- decode.optional_field("rank", 0.0, decode.float)
  use state <- decode.optional_field("state", "proposed", decode.string)
  use slot <- decode.optional_field(
    "colorSlot",
    None,
    decode.optional(decode.int),
  )
  decode.success(Feature(id, title, body, milestone, rank, state, slot))
}

/// A milestone in the stream. A new record starts `proposed` (D-87).
pub fn milestone_decoder() -> decode.Decoder(Milestone) {
  use id <- decode.field("id", decode.string)
  use title <- decode.optional_field("title", "", decode.string)
  use rank <- decode.optional_field("rank", 0.0, decode.float)
  use body <- decode.optional_field("body", "", decode.string)
  use target_date <- decode.optional_field(
    "targetDate",
    None,
    decode.optional(decode.string),
  )
  use state <- decode.optional_field("state", "proposed", decode.string)
  decode.success(Milestone(id, title, rank, body, target_date, state))
}

/// The milestones by rank then id, each with its features by rank then id
/// (empty ones too), then the features with no milestone, or one that is not
/// loaded, when there are any.
pub fn groups(
  milestones: List(Milestone),
  features: List(Feature),
) -> List(Group) {
  let ordered = ranked(milestones)
  let in_order = fn(fs: List(Feature)) {
    list.sort(fs, fn(a, b) {
      float.compare(a.rank, b.rank)
      |> order.break_tie(string.compare(a.id, b.id))
    })
  }
  let known = fn(f: Feature) {
    case f.milestone {
      Some(m) -> list.any(milestones, fn(x) { x.id == m })
      None -> False
    }
  }
  let loose = list.filter(features, fn(f) { !known(f) })
  list.append(
    list.map(ordered, fn(m) {
      Group(
        Some(m),
        in_order(list.filter(features, fn(f) { f.milestone == Some(m.id) })),
      )
    }),
    case loose {
      [] -> []
      _ -> [Group(None, in_order(loose))]
    },
  )
}

fn feature_view(
  base: String,
  feature: Feature,
  color: String,
  group: List(Feature),
  tasks: List(#(String, String, String, Option(String), Bool)),
  open: fn(String, String) -> msg,
  editor: Editor(msg),
  new_task: TaskForm(msg),
  planning: Bool,
) -> Element(msg) {
  let linked = list.filter(tasks, fn(t) { t.3 == Some(feature.id) })
  let full = editor.focused == Some(feature.id)
  let busy = editor.draft != None || editor.deleting != None
  let small = fn(label, name, enabled: Bool, msg) {
    html.button(
      [
        attribute.type_("button"),
        attribute.class("edit"),
        attribute.attribute("aria-label", name),
        attribute.disabled(busy || !enabled),
        event.on_click(msg),
      ],
      [html.text(label)],
    )
  }
  let grip =
    html.button(
      [
        attribute.type_("button"),
        attribute.class("grip"),
        attribute.attribute("aria-label", "Reorder " <> feature.title),
        attribute.attribute("aria-roledescription", "reorder handle"),
        attribute.tabindex(0),
        attribute.disabled(busy || editor.saving),
        // The card follows the pointer from here; the arrows reorder without
        // one, so a card can be moved with no pointer.
        event.prevent_default(
          event.on("pointerdown", {
            use y <- decode.field(
              "clientY",
              decode.one_of(decode.int, [decode.map(decode.float, float.round)]),
            )
            decode.success(editor.on_grab(feature.id, y))
          }),
        ),
        event.advanced("keydown", {
          use key <- decode.field("key", decode.string)
          let handler = fn(rank) {
            event.handler(
              editor.on_move(feature.id, rank),
              prevent_default: True,
              stop_propagation: False,
            )
          }
          case grip_key(key) {
            None -> decode.failure(handler(feature.rank), "an arrow key")
            Some(up) ->
              case move_rank(group, feature.id, up) {
                // An arrow at the end of the group: the card stays where it
                // is, and no PATCH goes out for the rank it already has.
                None -> decode.failure(handler(feature.rank), "a move that way")
                Some(rank) -> decode.success(handler(rank))
              }
          }
        }),
      ],
      [html.text("⠿")],
    )
  html.article(
    [
      attribute.class("feature"),
      // A feature link (D-122) finds its card by this.
      attribute.attribute("data-feature-id", feature.id),
      case color {
        "" -> attribute.none()
        _ -> attribute.style("border-left", "8px solid " <> color)
      },
    ],
    [
      grip,
      html.div([attribute.class("feature-body")], [
        html.h4([], [
          html.text(feature.title),
          html.text(" "),
          html.span([attribute.class("state")], [html.text(feature.state)]),
          case planning {
            True -> plan.marker()
            False -> element.none()
          },
          html.text(" "),
          html.button(
            [
              attribute.type_("button"),
              attribute.class("toggle"),
              attribute.attribute("aria-expanded", case full {
                True -> "true"
                False -> "false"
              }),
              event.on_click(editor.on_focus(feature.id)),
            ],
            [
              html.text(case full {
                True -> "Less"
                False -> "More"
              }),
            ],
          ),
          html.button(
            [
              attribute.type_("button"),
              attribute.class("edit"),
              attribute.disabled(busy),
              event.on_click(editor.on_edit(feature.id)),
            ],
            [html.text(solace_task.ui_label("edit"))],
          ),
          // Only a feature not started yet can start (D-154).
          case feature.state {
            "proposed" | "upcoming" ->
              small(
                solace_task.ui_label("start"),
                solace_task.ui_label("start") <> " " <> feature.title,
                True,
                editor.on_start(feature.id),
              )
            _ -> element.none()
          },
          small(
            solace_task.ui_label("delete"),
            solace_task.ui_label("delete") <> " " <> feature.title,
            True,
            editor.on_delete(feature.id),
          ),
        ]),
        case editor.deleting == Some(feature.id) {
          False -> element.none()
          True ->
            html.div(
              [attribute.role("alertdialog"), attribute.class("confirm")],
              [
                html.p([], [
                  html.text("Delete "),
                  html.em([], [html.text(feature.title)]),
                  html.text("? " <> delete_text(list.length(linked))),
                ]),
                html.div([attribute.class("vision-actions")], [
                  html.button(
                    [
                      attribute.type_("button"),
                      event.on_click(editor.on_confirm_delete),
                    ],
                    [html.text(solace_task.ui_label("delete"))],
                  ),
                  html.button(
                    [
                      attribute.type_("button"),
                      event.on_click(editor.on_cancel_delete),
                    ],
                    [html.text(solace_task.ui_label("cancel"))],
                  ),
                ]),
              ],
            )
        },
        case full {
          False ->
            html.p([attribute.class("feature-count")], [
              html.text(
                int.to_string(list.count(linked, fn(t) { t.4 }))
                <> " / "
                <> int.to_string(list.length(linked))
                <> " final",
              ),
            ])
          True -> element.none()
        },
        case full, feature.body {
          False, _ | _, "" -> element.none()
          True, body -> solace_task.md(base, body)
        },
        case full, linked {
          False, _ -> element.none()
          True, [] -> html.p([], [html.text("No tasks.")])
          True, _ ->
            html.ul(
              [attribute.class("feature-tasks")],
              list.map(linked, fn(t) {
                html.li([], [
                  html.button(
                    [
                      attribute.type_("button"),
                      attribute.class("open"),
                      event.on_click(open(t.0, t.1)),
                    ],
                    [html.text(t.2)],
                  ),
                ])
              }),
            )
        },
        case full {
          True -> task_form(feature, busy, new_task)
          False -> element.none()
        },
      ]),
    ],
  )
}

/// The Features tab: a group per milestone, then "No milestone". `tasks` are
/// every project's tasks as (project, id, title, feature, final); a click on
/// one sends `open(project, id)`. The form for a new feature shows above the
/// groups; an edited feature's form shows in its place. Each feature ends
/// with a form for a new task linked to it. `planning` are the features with
/// an open planning run, which show a marker. A started feature's edge has
/// the color its tasks' cards show on the board. Only the focused feature
/// shows its body, tasks and new-task form; the others show a task count.
pub fn view(
  base: String,
  milestones: List(Milestone),
  features: List(Feature),
  tasks: List(#(String, String, String, Option(String), Bool)),
  open: fn(String, String) -> msg,
  editor: Editor(msg),
  new_task: TaskForm(msg),
  planning: List(String),
) -> List(Element(msg)) {
  let colors =
    solace_task.feature_colors(list.map(features, fn(f) { #(f.id, f.slot) }))
  let top = case editor.draft {
    Some(Draft(id: None, ..) as draft) -> [
      form(base, milestones, editor, draft),
    ]
    _ -> [
      case editor.draft, editor.error {
        None, message if message != "" ->
          html.p([attribute.role("alert")], [html.text(message)])
        _, _ -> element.none()
      },
      html.div([attribute.class("vision-actions")], [
        html.button(
          [
            attribute.type_("button"),
            attribute.disabled(editor.draft != None),
            event.on_click(editor.on_new),
          ],
          [html.text(solace_task.ui_label("new_feature"))],
        ),
      ]),
    ]
  }
  let shown = fn(f: Feature, group) {
    case editor.draft {
      Some(Draft(id: Some(id), ..) as draft) if id == f.id ->
        form(base, milestones, editor, draft)
      _ ->
        feature_view(
          base,
          f,
          list.key_find(colors, f.id) |> result.unwrap(""),
          group,
          tasks,
          open,
          editor,
          new_task,
          list.contains(planning, f.id),
        )
    }
  }
  list.append(top, case milestones, features {
    [], [] -> [html.p([], [html.text("No features yet.")])]
    _, _ ->
      list.map(groups(milestones, features), fn(g) {
        html.section([attribute.class("milestone")], [
          html.h3([], [
            html.text(case g.milestone {
              Some(m) -> m.title
              None -> "No milestone"
            }),
          ]),
          ..case g.features {
            [] -> [html.p([], [html.text("No features.")])]
            fs -> list.map(fs, shown(_, fs))
          }
        ])
      })
  })
}

/// The states a feature can be in, the same for milestones (D-87; `upcoming`
/// was `planned` before D-128).
pub const states = ["proposed", "upcoming", "active", "complete", "canceled"]

/// A feature being written: `id` is None for a new one.
pub type Draft {
  Draft(
    id: Option(String),
    title: String,
    body: String,
    state: String,
    milestone: Option(String),
  )
}

/// The feature form: the draft, if one is open, and what the form sends;
/// also the list's moves and deletes.
pub type Editor(msg) {
  Editor(
    draft: Option(Draft),
    /// A save sent and not yet answered.
    saving: Bool,
    error: String,
    on_new: msg,
    on_edit: fn(String) -> msg,
    on_title: fn(String) -> msg,
    on_text: fn(String) -> msg,
    on_state: fn(String) -> msg,
    /// The milestone's id, "" for none.
    on_milestone: fn(String) -> msg,
    on_format: fn(String, String) -> msg,
    on_block: fn(String) -> msg,
    on_save: msg,
    on_cancel: msg,
    /// The feature's id and its new rank.
    on_move: fn(String, Float) -> msg,
    /// A card's grip pressed at this clientY: the drag begins.
    on_grab: fn(String, Int) -> msg,
    /// Starts the feature with this id: it becomes active.
    on_start: fn(String) -> msg,
    /// The feature whose delete is being confirmed.
    deleting: Option(String),
    on_delete: fn(String) -> msg,
    on_confirm_delete: msg,
    on_cancel_delete: msg,
    /// The feature shown in full; the others show a compact card.
    focused: Option(String),
    /// Shows the feature with this id in full, or compacts it again.
    on_focus: fn(String) -> msg,
  )
}

/// The feature editor's textarea.
pub const text_id = "feature-text"

/// A new feature: it starts `proposed` with no milestone (D-87).
pub fn new_draft() -> Draft {
  Draft(None, "", "", "proposed", None)
}

pub fn draft_of(feature: Feature) -> Draft {
  Draft(
    Some(feature.id),
    feature.title,
    feature.body,
    feature.state,
    feature.milestone,
  )
}

/// The body of `POST /features`. `request` makes a repeated create return the
/// feature the first one made (Spec 15).
pub fn create_body(draft: Draft, request: String) -> json.Json {
  json.object(
    list.flatten([
      [
        #("title", json.string(draft.title)),
        #("body", json.string(draft.body)),
        #("state", json.string(draft.state)),
      ],
      case draft.milestone {
        Some(m) -> [#("milestone", json.string(m))]
        None -> []
      },
      [#("request", json.string(request))],
    ]),
  )
}

/// The fields of `PATCH /features/{id}`: only those the draft changed, a
/// cleared milestone as null.
pub fn changes(feature: Feature, draft: Draft) -> List(#(String, json.Json)) {
  let field = fn(changed, name, value) {
    case changed {
      True -> [#(name, value)]
      False -> []
    }
  }
  list.flatten([
    field(draft.title != feature.title, "title", json.string(draft.title)),
    field(draft.body != feature.body, "body", json.string(draft.body)),
    field(draft.state != feature.state, "state", json.string(draft.state)),
    field(
      draft.milestone != feature.milestone,
      "milestone",
      json.nullable(draft.milestone, json.string),
    ),
  ])
}

/// Milestones by rank, then id.
pub fn ranked(milestones: List(Milestone)) -> List(Milestone) {
  list.sort(milestones, by_rank)
}

/// The form for a new feature or an edit: title, description, state and
/// milestone. A refusal (`E7001`, `E7023`) shows above Save and keeps the draft.
fn form(
  base: String,
  milestones: List(Milestone),
  editor: Editor(msg),
  draft: Draft,
) -> Element(msg) {
  let select = fn(name, value, options: List(#(String, String)), msg) {
    html.label([], [
      html.text(name),
      html.select(
        [event.on_change(msg)],
        list.map(options, fn(o) {
          html.option(
            [attribute.value(o.0), attribute.selected(o.0 == value)],
            o.1,
          )
        }),
      ),
    ])
  }
  let button = fn(label, msg) {
    html.button(
      [
        attribute.type_("button"),
        attribute.disabled(editor.saving),
        event.on_click(msg),
      ],
      [html.text(label)],
    )
  }
  let listed = list.map(ranked(milestones), fn(m) { #(m.id, m.title) })
  // A milestone not loaded yet shows its id.
  let listed = case draft.milestone {
    Some(m) ->
      case list.any(listed, fn(o) { o.0 == m }) {
        True -> listed
        False -> list.append(listed, [#(m, m)])
      }
    None -> listed
  }
  let attrs = case draft.id {
    None -> [attribute.class("feature-form")]
    // The form stands where the feature's card would: `api.mjs` finds the card
    // by this id to reveal or drag it.
    Some(id) -> [
      attribute.class("feature-form"),
      attribute.attribute("data-feature-id", id),
    ]
  }
  html.div(attrs, [
    html.h3([], [
      html.text(case draft.id {
        None -> solace_task.ui_label("new_feature")
        Some(_) -> solace_task.ui_label("edit_feature")
      }),
    ]),
    html.div([attribute.class("filters")], [
      html.label([], [
        html.text("Title"),
        html.input([
          attribute.type_("text"),
          attribute.value(draft.title),
          event.on_input(editor.on_title),
        ]),
      ]),
    ]),
    solace_task.editor(
      text_id,
      "",
      base,
      "Description",
      draft.body,
      editor.on_text,
      editor.on_format,
      editor.on_block,
    ),
    html.div([attribute.class("filters")], [
      select(
        "State",
        draft.state,
        list.map(states, fn(s) { #(s, s) }),
        editor.on_state,
      ),
      select(
        "Milestone",
        option.unwrap(draft.milestone, ""),
        [#("", "(none)"), ..listed],
        editor.on_milestone,
      ),
    ]),
    case editor.error {
      "" -> element.none()
      message -> html.p([attribute.role("alert")], [html.text(message)])
    },
    html.div([attribute.class("vision-actions")], [
      button(solace_task.ui_label("save"), editor.on_save),
      button(solace_task.ui_label("cancel"), editor.on_cancel),
    ]),
  ])
}

/// The rank that moves the feature `id` one place up or down in `group`, its
/// group's features in order: the midpoint of its new neighbors, or the
/// neighbor's rank ∓ 1 at either end. None when it can't move that way.
pub fn move_rank(group: List(Feature), id: String, up: Bool) -> Option(Float) {
  rank_move(list.map(group, fn(f) { #(f.id, f.rank) }), id, up)
}

/// What a delete's confirmation says after "Delete *title*?".
pub fn delete_text(tasks: Int) -> String {
  let noun = case tasks {
    1 -> " task loses its"
    _ -> " tasks lose their"
  }
  "Its " <> int.to_string(tasks) <> noun <> " link to it; the tasks stay."
}

/// Each feature's new-task form: the typed titles and errors by feature id,
/// and the projects a task can go to as (folder, label), empty unless the
/// root has several.
pub type TaskForm(msg) {
  TaskForm(
    titles: List(#(String, String)),
    errors: List(#(String, String)),
    projects: List(#(String, String)),
    /// The folder picked.
    project: String,
    /// The feature whose new task is being created and linked.
    saving: Option(String),
    on_title: fn(String, String) -> msg,
    on_project: fn(String) -> msg,
    on_add: fn(String) -> msg,
  )
}

/// A feature's new-task form: a title, the project in a workspace, and Add.
fn task_form(
  feature: Feature,
  busy: Bool,
  new_task: TaskForm(msg),
) -> Element(msg) {
  let title = list.key_find(new_task.titles, feature.id) |> result.unwrap("")
  html.form(
    [
      attribute.class("new-task"),
      event.on_submit(fn(_) { new_task.on_add(feature.id) }),
    ],
    [
      html.input([
        attribute.type_("text"),
        attribute.attribute("aria-label", "New task for " <> feature.title),
        attribute.placeholder(solace_task.ui_label("new_task")),
        attribute.value(title),
        event.on_input(new_task.on_title(feature.id, _)),
      ]),
      case new_task.projects {
        [_, _, ..] ->
          html.select(
            [
              attribute.name("project"),
              attribute.attribute("aria-label", solace_task.ui_label("project")),
              event.on_change(new_task.on_project),
            ],
            list.map(new_task.projects, fn(p) {
              html.option(
                [
                  attribute.value(p.0),
                  attribute.selected(p.0 == new_task.project),
                ],
                p.1,
              )
            }),
          )
        _ -> element.none()
      },
      html.button(
        [
          attribute.type_("submit"),
          attribute.class("edit"),
          attribute.disabled(
            busy || new_task.saving != None || string.trim(title) == "",
          ),
        ],
        [html.text(solace_task.ui_label("create_task"))],
      ),
      case list.key_find(new_task.errors, feature.id) {
        Ok(message) -> html.p([attribute.role("alert")], [html.text(message)])
        Error(_) -> element.none()
      },
    ],
  )
}

/// `move_rank` for any ordered records, given as (id, rank).
pub fn rank_move(
  group: List(#(String, Float)),
  id: String,
  up: Bool,
) -> Option(Float) {
  let index =
    list.index_map(group, fn(r, i) { #(r.0, i) })
    |> list.key_find(id)
  case index {
    Ok(i) ->
      drop(group, id, case up {
        True -> i - 1
        False -> i + 1
      })
    Error(_) -> None
  }
}

/// The rank that puts the record `id` at index `to` among `group`, the
/// ordered records as (id, rank): the midpoint of its new neighbors, or the
/// neighbor's rank ∓ 1 at either end. None when it lands where it is, and
/// for an index outside the group.
fn drop(group: List(#(String, Float)), id: String, to: Int) -> Option(Float) {
  let index =
    list.index_map(group, fn(r, i) { #(r.0, i) })
    |> list.key_find(id)
  let others = list.filter(group, fn(r) { r.0 != id })
  let last = list.length(others)
  let at = fn(i: Int) -> Option(Float) {
    case i < 0, list.drop(others, i) {
      False, [r, ..] -> Some(r.1)
      _, _ -> None
    }
  }
  case index, to {
    Ok(from), to if to != from && to >= 0 && to <= last ->
      case at(to - 1), at(to) {
        Some(a), Some(b) -> Some({ a +. b } /. 2.0)
        None, Some(b) -> Some(b -. 1.0)
        Some(a), None -> Some(a +. 1.0)
        _, _ -> None
      }
    _, _ -> None
  }
}

/// The rank that puts the feature `id` at index `to` in `group`, its group's
/// features in order: the rank `drop_rank` gives. None when it lands where it
/// is, and for an index outside the group.
pub fn drop_rank(group: List(Feature), id: String, to: Int) -> Option(Float) {
  drop(list.map(group, fn(f) { #(f.id, f.rank) }), id, to)
}

/// The grip's reorder keys: the arrows, up for the first. Any other key,
/// Tab among them, keeps its default.
pub fn grip_key(key: String) -> Option(Bool) {
  case key {
    "ArrowUp" -> Some(True)
    "ArrowDown" -> Some(False)
    _ -> None
  }
}

/// Milestones by rank, then id.
pub fn by_rank(a: Milestone, b: Milestone) -> order.Order {
  float.compare(a.rank, b.rank)
  |> order.break_tie(string.compare(a.id, b.id))
}
