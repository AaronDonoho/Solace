//// The Roadmap tab (D-90): milestones by target date, each with its features
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

/// The milestones by target date, earliest first, empty ones too; ones with no
/// date follow by rank then id. A milestone holds the features (`fs`) that
/// target it (D-188), each once, by rank then id. A task is in its own
/// milestone, else in its feature's target; a milestone also lists its tasks
/// with no feature. Then "No milestone", when it holds anything: the features
/// in no milestone or one not loaded. A task with neither is not on the
/// roadmap.
pub fn groups(
  milestones: List(Milestone),
  fs: List(Feature),
  tasks: List(Task),
) -> List(Group) {
  let ordered = list.sort(fs, by_rank)
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
        case targets(f) {
          True -> Ok(#(f, of(f)))
          False -> Error(Nil)
        }
      }),
      case m {
        Some(_) -> list.filter(here, fn(t) { !known(t) })
        None -> []
      },
    )
  }
  let placed =
    list.map(by_target_date(milestones), fn(m) {
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
/// also the list's deletes, feature targets and dependencies.
pub type Editor(msg) {
  Editor(
    draft: Option(Draft),
    /// A save sent and not yet answered.
    saving: Bool,
    /// A feature target sent and not yet answered.
    targeting: Bool,
    error: String,
    /// The milestones shown expanded, by id; the page session only.
    expanded: List(String),
    /// A milestone heading: shows its body, features and tasks, or hides them.
    on_toggle: fn(String) -> msg,
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
    /// The milestone whose delete is being confirmed.
    deleting: Option(String),
    on_delete: fn(String) -> msg,
    on_confirm_delete: msg,
    on_cancel_delete: msg,
    /// A feature's id and the milestone it now targets, "" for none.
    on_target: fn(String, String) -> msg,
    /// The words searched for a feature to add.
    feature_query: String,
    on_feature_query: fn(String) -> msg,
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
  let page =
    Page(
      base: base,
      fs: fs,
      tasks: tasks,
      open: open,
      editor: editor,
      busy: busy,
      planning: planning,
    )
  let top = case editor.draft {
    Some(Draft(id: None, ..) as draft) -> [
      form(base, fs, editor, draft, ""),
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
          [html.text(solace_task.ui_label("new_milestone"))],
        ),
      ]),
    ]
  }
  list.append(top, case groups(milestones, fs, tasks) {
    [] -> [html.p([], [html.text("No milestones yet.")])]
    all -> list.map(all, fn(group) { group_view(page, group) })
  })
}

/// A milestone's heading: its title, state, target date and feature progress,
/// with Edit and Delete; then the delete's confirmation, and its body when
/// expanded.
fn milestone_view(
  base: String,
  milestone: Milestone,
  editor: Editor(msg),
  busy: Bool,
  shown: Shown,
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
  [
    // A milestone link (D-122) finds its heading by this.
    html.h3([attribute.attribute("data-milestone-id", milestone.id)], [
      html.button(
        [
          attribute.type_("button"),
          attribute.class("toggle"),
          attribute.attribute("aria-expanded", case shown.expanded {
            True -> "true"
            False -> "false"
          }),
          event.on_click(editor.on_toggle(milestone.id)),
        ],
        [
          html.span([attribute.class("title")], [html.text(milestone.title)]),
          html.span([attribute.class("facts")], [
            html.span([attribute.class(features.state_class(milestone.state))], [
              html.text(milestone.state),
            ]),
            case shown.planning {
              True -> plan.marker()
              False -> element.none()
            },
            case milestone.target_date {
              Some(d) if d != "" ->
                html.span([attribute.class("target")], [html.text(d)])
              _ -> element.none()
            },
            html.span([attribute.class("feature-count")], [
              html.text(feature_count(shown.progress.0, shown.progress.1)),
            ]),
          ]),
        ],
      ),
      small(
        solace_task.ui_label("edit"),
        solace_task.ui_label("edit") <> " " <> milestone.title,
        True,
        editor.on_edit(milestone.id),
      ),
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
            html.text("? " <> delete_text(shown.held.0, shown.held.1)),
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
    case shown.expanded {
      False -> element.none()
      True ->
        case milestone.body {
          "" -> element.none()
          body -> solace_task.md(base, body)
        }
    },
  ]
}

/// A feature in a milestone: its title and state, then its tasks there.
fn feature_view(feature: Feature, tasks: Element(msg)) -> Element(msg) {
  html.article([attribute.class("feature")], [
    html.h4([], [
      html.text(feature.title),
      html.text(" "),
      html.span([attribute.class(features.state_class(feature.state))], [
        html.text(feature.state),
      ]),
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
/// and state; an edit also lists the features it targets. A refusal (`E7001`,
/// `E7023`) shows above Save and keeps the draft.
fn form(
  base: String,
  fs: List(Feature),
  editor: Editor(msg),
  draft: Draft,
  milestone_title: String,
) -> Element(msg) {
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
      solace_task.EditorConfig(
        id: text_id,
        task: "",
        base: base,
        label: "Description",
        text: draft.body,
        on_input: editor.on_text,
        on_format: editor.on_format,
        on_block: editor.on_block,
        actions: [],
      ),
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
    feature_editor(draft, fs, editor, milestone_title),
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

/// The most found features or tasks a picker shows at once.
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
  html.div([attribute.class("picker")], [
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
  let dependents = dependents(all, [task.id], [])
  list.filter(all, fn(o) {
    o.id != task.id
    && !list.contains(task.depends, o.id)
    && !list.contains(dependents, o.id)
    && title_matches(query, o.title)
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

/// The milestones by target date, earliest first; those without one follow by
/// rank then id.
fn by_target_date(milestones: List(Milestone)) -> List(Milestone) {
  let has_date = fn(m: Milestone) {
    case m.target_date {
      Some(date) -> date != ""
      None -> False
    }
  }
  let date = fn(m: Milestone) { option.unwrap(m.target_date, "") }
  let dated =
    milestones
    |> list.filter(has_date)
    |> list.sort(fn(a, b) {
      string.compare(date(a), date(b))
      |> order.break_tie(features.by_rank(a, b))
    })
  let undated =
    milestones
    |> list.filter(fn(m) { !has_date(m) })
    |> features.ranked
  list.append(dated, undated)
}

/// The milestone's features with Remove, then a search over the others with
/// Add. A new milestone has no id yet, so it offers neither.
fn feature_editor(
  draft: Draft,
  fs: List(Feature),
  editor: Editor(msg),
  milestone_title: String,
) -> Element(msg) {
  case draft.id {
    None -> element.none()
    Some(id) -> {
      let current = targets(fs, id)
      let found = feature_candidates(fs, id, editor.feature_query)
      let busy = editor.saving || editor.targeting
      let label = case milestone_title {
        "" -> "Find a feature"
        title -> "Find a feature for " <> title
      }
      let button = fn(
        label: String,
        aria: String,
        feature: Feature,
        milestone: String,
      ) {
        html.button(
          [
            attribute.type_("button"),
            attribute.class("edit"),
            attribute.attribute("aria-label", aria),
            attribute.disabled(busy),
            event.on_click(editor.on_target(feature.id, milestone)),
          ],
          [html.text(label)],
        )
      }
      html.div([attribute.class("picker")], [
        html.h4([], [html.text("Features")]),
        case current {
          [] -> html.p([], [html.text("No features.")])
          listed ->
            html.ul(
              [],
              list.map(listed, fn(f) {
                html.li([], [
                  html.text(f.title <> " "),
                  button(
                    solace_task.ui_label("remove"),
                    "Remove " <> f.title,
                    f,
                    "",
                  ),
                ])
              }),
            )
        },
        html.input([
          attribute.type_("search"),
          attribute.attribute("aria-label", label),
          attribute.placeholder("Find a feature"),
          attribute.value(editor.feature_query),
          attribute.disabled(busy),
          event.on_input(editor.on_feature_query),
        ]),
        case list.take(found, shown_limit) {
          [] ->
            html.p([], [
              html.text(case string.trim(editor.feature_query) {
                "" -> "No other feature to add."
                _ -> "No feature matches."
              }),
            ])
          shown ->
            html.ul(
              [],
              list.map(shown, fn(f) {
                html.li([], [
                  button(solace_task.ui_label("add"), "Add " <> f.title, f, id),
                  html.text(" " <> f.title),
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
  }
}

/// The features targeting `milestone`, by rank then id.
pub fn targets(fs: List(Feature), milestone: String) -> List(Feature) {
  list.filter(fs, fn(f) { f.milestone == Some(milestone) })
  |> list.sort(by_rank)
}

/// The features not targeting `milestone` whose title has every word of
/// `query`, ignoring case, by title then id.
pub fn feature_candidates(
  fs: List(Feature),
  milestone: String,
  query: String,
) -> List(Feature) {
  list.filter(fs, fn(f) {
    f.milestone != Some(milestone) && title_matches(query, f.title)
  })
  |> list.sort(fn(a, b) {
    string.compare(a.title, b.title)
    |> order.break_tie(string.compare(a.id, b.id))
  })
}

fn title_matches(query: String, title: String) -> Bool {
  string.split(string.lowercase(query), " ")
  |> list.filter(fn(w) { w != "" })
  |> list.all(fn(w) { string.contains(string.lowercase(title), w) })
}

fn by_rank(a: Feature, b: Feature) -> order.Order {
  float.compare(a.rank, b.rank)
  |> order.break_tie(string.compare(a.id, b.id))
}

/// "2 / 3 features complete", or "1 / 1 feature complete".
fn feature_count(complete: Int, total: Int) -> String {
  let noun = case total {
    1 -> "feature"
    _ -> "features"
  }
  int.to_string(complete)
  <> " / "
  <> int.to_string(total)
  <> " "
  <> noun
  <> " complete"
}

/// The Roadmap tab's shared view state.
type Page(msg) {
  Page(
    base: String,
    fs: List(Feature),
    tasks: List(Task),
    open: fn(String, String) -> msg,
    editor: Editor(msg),
    busy: Bool,
    planning: List(String),
  )
}

/// The milestone heading's facts: what a delete unlinks, the feature progress,
/// and whether it is planned or expanded.
type Shown {
  Shown(
    held: #(Int, Int),
    progress: #(Int, Int),
    planning: Bool,
    expanded: Bool,
  )
}

/// A milestone's section: its heading, then its body when open.
fn group_view(page: Page(msg), group: Group) -> Element(msg) {
  let rows = fn(ts) {
    task_list(ts, page.tasks, page.open, page.editor, page.busy)
  }
  let head = case group.milestone {
    Some(m) ->
      heading_view(
        page,
        m,
        feature_progress(list.map(group.features, fn(f) { f.0 })),
      )
    None -> [html.h3([], [html.text("No milestone")])]
  }
  // "No milestone" has no heading to open or close.
  let open = case group.milestone {
    Some(m) -> list.contains(page.editor.expanded, m.id)
    None -> True
  }
  let body = case open {
    False -> []
    True ->
      list.append(
        list.map(group.features, fn(p) { feature_view(p.0, rows(p.1)) }),
        case group.features, group.tasks {
          [], [] -> [html.p([], [html.text("No features.")])]
          _, [] -> []
          _, ts -> [
            html.article([attribute.class("feature")], [
              html.h4([], [html.text("No feature")]),
              rows(ts),
            ]),
          ]
        },
      )
  }
  html.section([attribute.class("milestone")], list.append(head, body))
}

/// A milestone's heading, or its form when it is being edited.
fn heading_view(
  page: Page(msg),
  milestone: Milestone,
  progress: #(Int, Int),
) -> List(Element(msg)) {
  case page.editor.draft {
    Some(Draft(id: Some(id), ..) as draft) if id == milestone.id -> [
      form(page.base, page.fs, page.editor, draft, milestone.title),
    ]
    _ -> {
      let held = #(
        list.count(page.fs, fn(f) { f.milestone == Some(milestone.id) }),
        list.count(page.tasks, fn(t) { t.milestone == Some(milestone.id) }),
      )
      milestone_view(
        page.base,
        milestone,
        page.editor,
        page.busy,
        Shown(
          held: held,
          progress: progress,
          planning: list.contains(page.planning, milestone.id),
          expanded: list.contains(page.editor.expanded, milestone.id),
        ),
      )
    }
  }
}

/// A milestone's completed features, and its features that are not canceled.
fn feature_progress(fs: List(Feature)) -> #(Int, Int) {
  #(
    list.count(fs, fn(f) { f.state == "complete" }),
    list.count(fs, fn(f) { f.state != "canceled" }),
  )
}
