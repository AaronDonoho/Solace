//// The Roadmap tab (D-90): milestones in rank order, each with its features
//// and their tasks (D-87 point 2), a form to create and edit milestones, and
//// each task's `dependsOn`.

import features.{type Feature, type Milestone}
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

/// A task as the Roadmap tab shows it, from any project.
pub type Task {
  Task(
    dir: String,
    id: String,
    title: String,
    feature: Option(String),
    milestone: Option(String),
    /// The ids of the tasks it depends on.
    depends: List(String),
    /// Whether it has reached a final column.
    closed: Bool,
  )
}

/// A milestone (None for "No milestone"), its features in order, each with
/// its tasks there, then its tasks with no feature.
pub type Group {
  Group(
    milestone: Option(Milestone),
    features: List(#(Feature, List(Task))),
    tasks: List(Task),
  )
}

/// The loaded milestone a task is shown in: its own, else its feature's
/// target, else none.
fn home(
  task: Task,
  milestones: List(Milestone),
  fs: List(Feature),
) -> Option(String) {
  let loaded = fn(link: Option(String)) {
    case link {
      Some(id) -> list.any(milestones, fn(m) { m.id == id })
      None -> False
    }
  }
  let target =
    list.find(fs, fn(f) { Some(f.id) == task.feature })
    |> result.map(fn(f) { f.milestone })
    |> result.unwrap(None)
  case loaded(task.milestone), loaded(target) {
    True, _ -> task.milestone
    False, True -> target
    False, False -> None
  }
}

/// The milestones by rank then id, empty ones too. A milestone holds the
/// features (`fs`) that target it and those with a task in it (D-87 point 2),
/// each once, by rank then id. A task is in its own milestone, else in its
/// feature's target; a milestone also lists its tasks with no feature. Then
/// "No milestone", when it holds anything: the features in no milestone, and
/// those with a task in none. A task with neither is not on the roadmap.
pub fn groups(
  milestones: List(Milestone),
  fs: List(Feature),
  tasks: List(Task),
) -> List(Group) {
  let ordered =
    list.sort(fs, fn(a, b) {
      float.compare(a.rank, b.rank)
      |> order.break_tie(string.compare(a.id, b.id))
    })
  let homes = list.map(tasks, fn(t) { #(t, home(t, milestones, fs)) })
  let known = fn(t: Task) { list.any(fs, fn(f) { Some(f.id) == t.feature }) }
  let group = fn(m: Option(Milestone), targets: fn(Feature) -> Bool) {
    let at = option.map(m, fn(m) { m.id })
    let here =
      list.filter_map(homes, fn(p) {
        case p.1 == at {
          True -> Ok(p.0)
          False -> Error(Nil)
        }
      })
    let of = fn(f: Feature) {
      list.filter(here, fn(t) { t.feature == Some(f.id) })
    }
    Group(
      m,
      list.filter_map(ordered, fn(f) {
        case targets(f), of(f) {
          False, [] -> Error(Nil)
          _, ts -> Ok(#(f, ts))
        }
      }),
      case m {
        Some(_) -> list.filter(here, fn(t) { !known(t) })
        None -> []
      },
    )
  }
  let placed =
    list.map(features.ranked(milestones), fn(m) {
      group(Some(m), fn(f) { f.milestone == Some(m.id) })
    })
  let shown = fn(f: Feature) {
    list.any(placed, fn(g) {
      list.any(g.features, fn(p) { { p.0 }.id == f.id })
    })
  }
  case group(None, fn(f) { !shown(f) }) {
    Group(_, [], []) -> placed
    loose -> list.append(placed, [loose])
  }
}

/// A milestone being written: `id` is None for a new one.
pub type Draft {
  Draft(
    id: Option(String),
    title: String,
    body: String,
    state: String,
    target_date: Option(String),
  )
}

/// The milestone form: the draft, if one is open, and what the form sends;
/// also the list's moves, deletes, feature targets and dependencies.
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
    /// The date as the input gives it ("2027-01-01"), "" for none.
    on_target_date: fn(String) -> msg,
    on_state: fn(String) -> msg,
    on_format: fn(String, String) -> msg,
    on_block: fn(String) -> msg,
    on_save: msg,
    on_cancel: msg,
    /// The milestone's id and its new rank.
    on_move: fn(String, Float) -> msg,
    /// The milestone whose delete is being confirmed.
    deleting: Option(String),
    on_delete: fn(String) -> msg,
    on_confirm_delete: msg,
    on_cancel_delete: msg,
    /// A feature's id and the milestone it now targets, "" for none.
    on_target: fn(String, String) -> msg,
    /// A task's project, its id and every id it now depends on.
    on_depends: fn(String, String, List(String)) -> msg,
    /// The task whose dependencies are being edited: its project and id.
    depends_open: Option(#(String, String)),
    /// The words searched for a new dependency.
    depends_query: String,
    /// Opens a task's dependency editor, or closes it when it is open.
    on_depends_open: fn(String, String) -> msg,
    on_depends_query: fn(String) -> msg,
  )
}

/// The milestone editor's textarea.
pub const text_id = "milestone-text"

/// A new milestone: it starts `proposed` with no target date (D-87).
pub fn new_draft() -> Draft {
  Draft(None, "", "", "proposed", None)
}

pub fn draft_of(milestone: Milestone) -> Draft {
  Draft(
    Some(milestone.id),
    milestone.title,
    milestone.body,
    milestone.state,
    milestone.target_date,
  )
}

/// The body of `POST /milestones`. `request` makes a repeated create return
/// the milestone the first one made (Spec 15).
pub fn create_body(draft: Draft, request: String) -> json.Json {
  json.object(
    list.flatten([
      [
        #("title", json.string(draft.title)),
        #("body", json.string(draft.body)),
        #("state", json.string(draft.state)),
      ],
      case draft.target_date {
        Some(d) -> [#("targetDate", json.string(d))]
        None -> []
      },
      [#("request", json.string(request))],
    ]),
  )
}

/// The fields of `PATCH /milestones/{id}`: only those the draft changed, a
/// cleared target date as null.
pub fn changes(
  milestone: Milestone,
  draft: Draft,
) -> List(#(String, json.Json)) {
  let field = fn(changed, name, value) {
    case changed {
      True -> [#(name, value)]
      False -> []
    }
  }
  list.flatten([
    field(draft.title != milestone.title, "title", json.string(draft.title)),
    field(draft.body != milestone.body, "body", json.string(draft.body)),
    field(draft.state != milestone.state, "state", json.string(draft.state)),
    field(
      draft.target_date != milestone.target_date,
      "targetDate",
      json.nullable(draft.target_date, json.string),
    ),
  ])
}

/// The body of `PATCH /tasks/{id}` that sets every dependency of a task.
pub fn depends_body(ids: List(String)) -> json.Json {
  json.object([#("dependsOn", json.array(ids, json.string))])
}

/// What a delete's confirmation says after "Delete *title*?": the features
/// that target the milestone and the tasks in it.
pub fn delete_text(features: Int, tasks: Int) -> String {
  let count = fn(n, noun) {
    int.to_string(n)
    <> " "
    <> noun
    <> case n {
      1 -> ""
      _ -> "s"
    }
  }
  "Its "
  <> count(features, "feature")
  <> " and "
  <> count(tasks, "task")
  <> " lose their link to it; they stay."
}

/// The Roadmap tab: a group per milestone, then "No milestone". `tasks` are
/// every project's; a click on one sends `open(project, id)`. The form for a
/// new milestone shows above the groups; an edited milestone's form shows in
/// place of its heading. `planning` are the milestones with an open planning
/// run, which show a marker.
pub fn view(
  base: String,
  milestones: List(Milestone),
  fs: List(Feature),
  tasks: List(Task),
  open: fn(String, String) -> msg,
  editor: Editor(msg),
  planning: List(String),
) -> List(Element(msg)) {
  let busy = editor.draft != None || editor.deleting != None
  let top = case editor.draft {
    Some(Draft(id: None, ..) as draft) -> [form(base, editor, draft)]
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
          [html.text(solace_task.ui_label("new_milestone"))],
        ),
      ]),
    ]
  }
  let ranks = list.map(features.ranked(milestones), fn(m) { #(m.id, m.rank) })
  let heading = fn(m: Milestone) {
    case editor.draft {
      Some(Draft(id: Some(id), ..) as draft) if id == m.id -> [
        form(base, editor, draft),
      ]
      _ -> {
        let held = #(
          list.count(fs, fn(f) { f.milestone == Some(m.id) }),
          list.count(tasks, fn(t) { t.milestone == Some(m.id) }),
        )
        milestone_view(
          base,
          m,
          ranks,
          held,
          editor,
          busy,
          list.contains(planning, m.id),
        )
      }
    }
  }
  list.append(top, case groups(milestones, fs, tasks) {
    [] -> [html.p([], [html.text("No milestones yet.")])]
    all ->
      list.map(all, fn(g) {
        let rows = fn(ts) { task_list(ts, tasks, open, editor, busy) }
        html.section(
          [attribute.class("milestone")],
          list.flatten([
            case g.milestone {
              Some(m) -> heading(m)
              None -> [html.h3([], [html.text("No milestone")])]
            },
            list.map(g.features, fn(p) {
              feature_view(p.0, rows(p.1), milestones, editor, busy)
            }),
            case g.features, g.tasks {
              [], [] -> [html.p([], [html.text("No features.")])]
              _, [] -> []
              _, ts -> [
                html.article([attribute.class("feature")], [
                  html.h4([], [html.text("No feature")]),
                  rows(ts),
                ]),
              ]
            },
          ]),
        )
      })
  })
}

/// A milestone's heading: its title, state and target date, with Edit, Up,
/// Down and Delete; then the delete's confirmation and its body. `held` is
/// what loses its link on a delete: the features and tasks.
fn milestone_view(
  base: String,
  milestone: Milestone,
  ranks: List(#(String, Float)),
  held: #(Int, Int),
  editor: Editor(msg),
  busy: Bool,
  planning: Bool,
) -> List(Element(msg)) {
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
  let move = fn(label, name, up) {
    let rank = features.rank_move(ranks, milestone.id, up)
    small(
      label,
      name <> " " <> milestone.title,
      rank != None,
      editor.on_move(milestone.id, option.unwrap(rank, milestone.rank)),
    )
  }
  [
    // A milestone link (D-122) finds its heading by this.
    html.h3([attribute.attribute("data-milestone-id", milestone.id)], [
      html.text(milestone.title),
      html.text(" "),
      html.span([attribute.class("state")], [html.text(milestone.state)]),
      case planning {
        True -> plan.marker()
        False -> element.none()
      },
      case milestone.target_date {
        Some(d) -> html.span([attribute.class("target")], [html.text(d)])
        None -> element.none()
      },
      small(
        solace_task.ui_label("edit"),
        solace_task.ui_label("edit") <> " " <> milestone.title,
        True,
        editor.on_edit(milestone.id),
      ),
      move("Up", "Move up", True),
      move("Down", "Move down", False),
      small(
        solace_task.ui_label("delete"),
        solace_task.ui_label("delete") <> " " <> milestone.title,
        True,
        editor.on_delete(milestone.id),
      ),
    ]),
    case editor.deleting == Some(milestone.id) {
      False -> element.none()
      True ->
        html.div([attribute.role("alertdialog"), attribute.class("confirm")], [
          html.p([], [
            html.text("Delete "),
            html.em([], [html.text(milestone.title)]),
            html.text("? " <> delete_text(held.0, held.1)),
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
              [html.text("Cancel")],
            ),
          ]),
        ])
    },
    case milestone.body {
      "" -> element.none()
      body -> solace_task.md(base, body)
    },
  ]
}

/// A feature in a milestone: its title, state and a select that moves it to
/// another milestone, then its tasks there.
fn feature_view(
  feature: Feature,
  tasks: Element(msg),
  milestones: List(Milestone),
  editor: Editor(msg),
  busy: Bool,
) -> Element(msg) {
  let listed = list.map(features.ranked(milestones), fn(m) { #(m.id, m.title) })
  // A target not loaded yet shows its id.
  let listed = case feature.milestone {
    Some(m) ->
      case list.any(listed, fn(o) { o.0 == m }) {
        True -> listed
        False -> list.append(listed, [#(m, m)])
      }
    None -> listed
  }
  let target = option.unwrap(feature.milestone, "")
  html.article([attribute.class("feature")], [
    html.h4([], [
      html.text(feature.title),
      html.text(" "),
      html.span([attribute.class("state")], [html.text(feature.state)]),
      html.select(
        [
          attribute.attribute("aria-label", "Milestone of " <> feature.title),
          attribute.disabled(busy),
          event.on_change(editor.on_target(feature.id, _)),
        ],
        list.map([#("", "(none)"), ..listed], fn(o) {
          html.option(
            [attribute.value(o.0), attribute.selected(o.0 == target)],
            o.1,
          )
        }),
      ),
    ]),
    tasks,
  ])
}

/// Tasks, each with what it depends on and a button that opens its
/// dependency editor. `all` is every task of the root, which any task can
/// depend on (D-87).
fn task_list(
  tasks: List(Task),
  all: List(Task),
  open: fn(String, String) -> msg,
  editor: Editor(msg),
  busy: Bool,
) -> Element(msg) {
  case tasks {
    [] -> html.p([], [html.text("No tasks.")])
    _ ->
      html.ul(
        [attribute.class("feature-tasks")],
        list.map(tasks, fn(t) {
          // Another project's task names its project.
          let name = fn(id) {
            case list.find(all, fn(o) { o.id == id }) {
              Ok(o) if o.dir != t.dir -> o.title <> " (" <> o.dir <> ")"
              Ok(o) -> o.title
              Error(_) -> id
            }
          }
          let editing = editor.depends_open == Some(#(t.dir, t.id))
          html.li([], [
            html.button(
              [
                attribute.type_("button"),
                attribute.class("open"),
                event.on_click(open(t.dir, t.id)),
              ],
              [html.text(t.title)],
            ),
            case t.depends {
              [] -> element.none()
              ds ->
                html.span([attribute.class("depends")], [
                  html.text(
                    " depends on " <> string.join(list.map(ds, name), ", "),
                  ),
                ])
            },
            html.button(
              [
                attribute.type_("button"),
                attribute.class("edit"),
                attribute.attribute("aria-label", "Dependencies of " <> t.title),
                attribute.attribute("aria-expanded", case editing {
                  True -> "true"
                  False -> "false"
                }),
                attribute.disabled(busy),
                event.on_click(editor.on_depends_open(t.dir, t.id)),
              ],
              [html.text(solace_task.ui_label("dependencies"))],
            ),
            case editing {
              True -> depends_editor(t, all, name, editor, busy)
              False -> element.none()
            },
          ])
        }),
      )
  }
}

/// The form for a new milestone or an edit: title, target date, description
/// and state. A refusal (`E7001`, `E7023`) shows above Save and keeps the draft.
fn form(base: String, editor: Editor(msg), draft: Draft) -> Element(msg) {
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
  html.div([attribute.class("feature-form")], [
    html.h3([], [
      html.text(case draft.id {
        None -> solace_task.ui_label("new_milestone")
        Some(_) -> solace_task.ui_label("edit_milestone")
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
      html.label([], [
        html.text("Target date"),
        html.input([
          attribute.type_("date"),
          attribute.value(option.unwrap(draft.target_date, "")),
          event.on_input(editor.on_target_date),
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
      html.label([], [
        html.text("State"),
        html.select(
          [event.on_change(editor.on_state)],
          list.map(features.states, fn(s) {
            html.option(
              [attribute.value(s), attribute.selected(s == draft.state)],
              s,
            )
          }),
        ),
      ]),
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

/// The most found tasks a dependency editor shows at once.
const shown_limit = 10

/// A task's dependency editor: each dependency with Remove, then a search
/// over the tasks it can come to depend on, each with a button that adds it.
fn depends_editor(
  task: Task,
  all: List(Task),
  name: fn(String) -> String,
  editor: Editor(msg),
  busy: Bool,
) -> Element(msg) {
  let found = candidates(all, task, editor.depends_query)
  let small = fn(label, aria, msg) {
    html.button(
      [
        attribute.type_("button"),
        attribute.class("edit"),
        attribute.attribute("aria-label", aria),
        attribute.disabled(busy),
        event.on_click(msg),
      ],
      [html.text(label)],
    )
  }
  html.div([attribute.class("depends-edit")], [
    case task.depends {
      [] -> html.p([], [html.text("No dependencies.")])
      ds ->
        html.ul(
          [],
          list.map(ds, fn(d) {
            html.li([], [
              html.text(name(d) <> " "),
              small(
                solace_task.ui_label("remove"),
                "Remove " <> name(d),
                editor.on_depends(
                  task.dir,
                  task.id,
                  list.filter(task.depends, fn(x) { x != d }),
                ),
              ),
            ])
          }),
        )
    },
    html.input([
      attribute.type_("search"),
      attribute.attribute(
        "aria-label",
        "Find a task " <> task.title <> " depends on",
      ),
      attribute.placeholder(solace_task.ui_label("find_task")),
      attribute.value(editor.depends_query),
      attribute.disabled(busy),
      event.on_input(editor.on_depends_query),
    ]),
    case list.take(found, shown_limit) {
      [] ->
        html.p([], [
          html.text(case string.trim(editor.depends_query) {
            "" -> "No other task to add."
            _ -> "No task matches."
          }),
        ])
      shown ->
        html.ul(
          [],
          list.map(shown, fn(o) {
            html.li([], [
              small(
                solace_task.ui_label("add"),
                "Depend on " <> name(o.id),
                editor.on_depends(
                  task.dir,
                  task.id,
                  list.append(task.depends, [o.id]),
                ),
              ),
              html.text(
                " "
                <> name(o.id)
                <> case o.closed {
                  True -> " (closed)"
                  False -> ""
                },
              ),
            ])
          }),
        )
    },
    case list.length(found) - shown_limit {
      more if more > 0 ->
        html.p([], [
          html.text(int.to_string(more) <> " more: narrow the search."),
        ])
      _ -> element.none()
    },
  ])
}

/// The tasks `task` can come to depend on, open ones first, then by title:
/// any task of the root (D-87) but itself, those it already depends on, and
/// those that depend on it, directly or through others, since the server
/// refuses a cycle (E7025). Every word of `query` is in the title, ignoring
/// case.
pub fn candidates(all: List(Task), task: Task, query: String) -> List(Task) {
  let words =
    string.split(string.lowercase(query), " ")
    |> list.filter(fn(w) { w != "" })
  let dependents = dependents(all, [task.id], [])
  list.filter(all, fn(o) {
    o.id != task.id
    && !list.contains(task.depends, o.id)
    && !list.contains(dependents, o.id)
    && list.all(words, fn(w) { string.contains(string.lowercase(o.title), w) })
  })
  |> list.sort(fn(a, b) {
    case a.closed, b.closed {
      False, True -> order.Lt
      True, False -> order.Gt
      _, _ -> order.Eq
    }
    |> order.break_tie(string.compare(a.title, b.title))
    |> order.break_tie(string.compare(a.id, b.id))
  })
}

/// The ids of the tasks that depend on any of `frontier`, directly or through
/// others, found by walking the dependencies backwards once. `seen` keeps a
/// cycle already in the data from looping.
fn dependents(
  all: List(Task),
  frontier: List(String),
  seen: List(String),
) -> List(String) {
  case frontier {
    [] -> seen
    _ -> {
      let next =
        list.filter_map(all, fn(t) {
          case
            !list.contains(seen, t.id)
            && list.any(t.depends, fn(d) { list.contains(frontier, d) })
          {
            True -> Ok(t.id)
            False -> Error(Nil)
          }
        })
      dependents(all, next, list.append(next, seen))
    }
  }
}
