import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/order.{type Order, Eq, Gt, Lt}
import gleam/string
import gleam/uri
import lustre
import lustre/attribute
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import rsvp

pub fn main() {
  let app = lustre.application(init:, update:, view:)
  let assert Ok(_) = lustre.start(app, "#app", Nil)
  Nil
}

pub type Group {
  Group(id: Int, name: String, paths: List(String))
}

pub type Entry {
  Entry(name: String, path: String, kind: String)
}

pub type Row {
  Row(
    group_id: String,
    group: String,
    source: String,
    task: String,
    rep: Int,
    model: String,
    arm: String,
    hidden_pass: Bool,
    tokens: Option(Int),
    work_tokens: Option(Int),
    uncached_tokens: Option(Int),
    output_tokens: Option(Int),
    duration_ms: Option(Int),
    cost_usd: Option(Float),
  )
}

pub type Results {
  Results(rows: List(Row), errors: List(String))
}

pub type BrowseResponse {
  BrowseResponse(path: String, parent: Option(String), entries: List(Entry))
}

pub type Metric {
  Tokens
  WorkTokens
  CachedInput
  UncachedInput
  OutputTokens
  Duration
  Cost
}

pub type Sort {
  Sort(key: String, descending: Bool)
}

type Series {
  Series(group: Group, rows: List(Row))
}

type Model {
  Model(
    groups: List(Group),
    rows: List(Row),
    selected: List(Row),
    anchor: Option(Row),
    sorts: List(Sort),
    picker_group: Option(Int),
    browse_path: String,
    parent: Option(String),
    entries: List(Entry),
    errors: List(String),
    loading: Bool,
    next_group: Int,
  )
}

type Msg {
  AddGroup
  RenameGroup(Int, String)
  DeleteGroup(Int)
  RemovePath(Int, String)
  OpenPicker(Int)
  ClosePicker
  Browse(String)
  BrowseLoaded(Result(BrowseResponse, rsvp.Error(String)))
  AddPath(String)
  Refresh
  ResultsLoaded(Result(Results, rsvp.Error(String)))
  SelectRow(Row, Bool, Bool)
  SortBy(String, Bool)
}

fn init(_) -> #(Model, Effect(Msg)) {
  let groups = [Group(1, "A", [])]
  #(
    Model(
      groups:,
      rows: [],
      selected: [],
      anchor: None,
      sorts: [],
      picker_group: None,
      browse_path: "bench/results",
      parent: None,
      entries: [],
      errors: [],
      loading: True,
      next_group: 2,
    ),
    fetch_results(groups),
  )
}

fn update(model: Model, message: Msg) -> #(Model, Effect(Msg)) {
  case message {
    AddGroup -> {
      let groups =
        list.append(model.groups, [
          Group(model.next_group, group_name(model.next_group), []),
        ])
      #(
        Model(..model, groups:, next_group: model.next_group + 1),
        effect.none(),
      )
    }
    RenameGroup(id, raw) -> {
      let name = case string.trim(raw) {
        "" -> "Untitled"
        name -> name
      }
      let groups =
        map_group(model.groups, id, fn(group) { Group(..group, name:) })
      #(Model(..model, groups:, loading: True), fetch_results(groups))
    }
    DeleteGroup(id) ->
      reload(model, list.filter(model.groups, fn(group) { group.id != id }))
    RemovePath(id, path) ->
      reload(
        model,
        map_group(model.groups, id, fn(group) {
          Group(
            ..group,
            paths: list.filter(group.paths, fn(item) { item != path }),
          )
        }),
      )
    OpenPicker(id) -> #(
      Model(..model, picker_group: Some(id)),
      fetch_browse(model.browse_path),
    )
    ClosePicker -> #(Model(..model, picker_group: None), effect.none())
    Browse(path) -> #(Model(..model, browse_path: path), fetch_browse(path))
    BrowseLoaded(Ok(response)) -> #(
      Model(
        ..model,
        browse_path: response.path,
        parent: response.parent,
        entries: response.entries,
        errors: [],
      ),
      effect.none(),
    )
    BrowseLoaded(Error(error)) -> #(
      Model(..model, errors: [string.inspect(error)]),
      effect.none(),
    )
    AddPath(path) ->
      case model.picker_group {
        None -> #(model, effect.none())
        Some(id) ->
          reload(
            model,
            map_group(model.groups, id, fn(group) {
              case list.contains(group.paths, path) {
                True -> group
                False -> Group(..group, paths: list.append(group.paths, [path]))
              }
            }),
          )
      }
    Refresh -> #(Model(..model, loading: True), fetch_results(model.groups))
    ResultsLoaded(Ok(results)) -> #(
      Model(
        ..model,
        rows: results.rows,
        errors: results.errors,
        selected: keep_selected(model.selected, results.rows),
        anchor: keep_anchor(model.anchor, results.rows),
        loading: False,
      ),
      effect.none(),
    )
    ResultsLoaded(Error(error)) -> #(
      Model(..model, errors: [string.inspect(error)], loading: False),
      effect.none(),
    )
    SelectRow(row, shifted, modified) -> {
      let #(selected, anchor) =
        select_rows(
          sort_rows(model.rows, model.sorts),
          model.selected,
          model.anchor,
          row,
          shifted,
          modified,
        )
      #(Model(..model, selected:, anchor:), effect.none())
    }
    SortBy(key, shifted) -> #(
      Model(..model, sorts: change_sort(model.sorts, key, shifted)),
      effect.none(),
    )
  }
}

pub fn group_name(number: Int) -> String {
  let index = number - 1
  let letter = string.slice("ABCDEFGHIJKLMNOPQRSTUVWXYZ", index % 26, 1)
  case index < 26 {
    True -> letter
    False -> group_name(index / 26) <> letter
  }
}

fn change_sort(sorts: List(Sort), key: String, shifted: Bool) -> List(Sort) {
  case shifted, sorts {
    False, [Sort(first, descending), ..rest] if first == key -> [
      Sort(key, !descending),
      ..rest
    ]
    False, _ -> [Sort(key, False)]
    True, [] -> [Sort(key, False)]
    True, [Sort(first, descending), ..rest] if first == key -> [
      Sort(key, !descending),
      ..rest
    ]
    True, [first, Sort(second, descending)] if second == key -> [
      first,
      Sort(key, !descending),
    ]
    True, [first, ..] -> [first, Sort(key, False)]
  }
}

pub fn sort_rows(rows: List(Row), sorts: List(Sort)) -> List(Row) {
  case sorts {
    [] -> rows
    _ -> list.sort(rows, fn(a, b) { compare_rows(a, b, sorts) })
  }
}

fn compare_rows(a: Row, b: Row, sorts: List(Sort)) -> Order {
  case sorts {
    [] -> Eq
    [Sort(key, descending), ..rest] -> {
      let order = compare_column(a, b, key)
      let order = case descending, order {
        True, Lt -> Gt
        True, Gt -> Lt
        _, order -> order
      }
      case order {
        Eq -> compare_rows(a, b, rest)
        _ -> order
      }
    }
  }
}

fn compare_column(a: Row, b: Row, key: String) -> Order {
  case key {
    "group" -> string.compare(a.group, b.group)
    "rep" -> int.compare(a.rep, b.rep)
    "arm" -> string.compare(a.arm, b.arm)
    "task" -> string.compare(a.task, b.task)
    "model" -> string.compare(a.model, b.model)
    "pass" ->
      case a.hidden_pass, b.hidden_pass {
        True, False -> Gt
        False, True -> Lt
        _, _ -> Eq
      }
    "tokens" -> compare_optional(a.tokens, b.tokens, int.compare)
    "work_tokens" -> compare_optional(a.work_tokens, b.work_tokens, int.compare)
    "cached_input" ->
      compare_optional(cached_input(a), cached_input(b), int.compare)
    "uncached_input" ->
      compare_optional(uncached_input(a), uncached_input(b), int.compare)
    "output_tokens" ->
      compare_optional(a.output_tokens, b.output_tokens, int.compare)
    "duration_ms" -> compare_optional(a.duration_ms, b.duration_ms, int.compare)
    "cost_usd" -> compare_optional(a.cost_usd, b.cost_usd, float.compare)
    _ -> Eq
  }
}

fn compare_optional(
  a: Option(value),
  b: Option(value),
  compare: fn(value, value) -> Order,
) -> Order {
  case a, b {
    Some(a), Some(b) -> compare(a, b)
    None, Some(_) -> Gt
    Some(_), None -> Lt
    None, None -> Eq
  }
}

fn reload(model: Model, groups: List(Group)) -> #(Model, Effect(Msg)) {
  #(Model(..model, groups:, loading: True), fetch_results(groups))
}

fn map_group(
  groups: List(Group),
  id: Int,
  change: fn(Group) -> Group,
) -> List(Group) {
  list.map(groups, fn(group) {
    case group.id == id {
      True -> change(group)
      False -> group
    }
  })
}

fn same_row(a: Row, b: Row) -> Bool {
  a.group_id == b.group_id
  && a.source == b.source
  && a.task == b.task
  && a.rep == b.rep
  && a.model == b.model
  && a.arm == b.arm
}

fn keep_selected(selected: List(Row), rows: List(Row)) -> List(Row) {
  list.filter_map(selected, fn(old) { list.find(rows, same_row(_, old)) })
}

fn keep_anchor(anchor: Option(Row), rows: List(Row)) -> Option(Row) {
  case anchor {
    None -> None
    Some(old) ->
      case list.find(rows, same_row(_, old)) {
        Ok(row) -> Some(row)
        Error(_) -> None
      }
  }
}

pub fn select_rows(
  visible: List(Row),
  selected: List(Row),
  anchor: Option(Row),
  row: Row,
  shifted: Bool,
  modified: Bool,
) -> #(List(Row), Option(Row)) {
  case shifted, anchor {
    True, Some(start) ->
      case row_index(visible, start, 0), row_index(visible, row, 0) {
        Ok(start_index), Ok(end_index) -> {
          let first = int.min(start_index, end_index)
          let count = int.max(start_index, end_index) - first + 1
          let range = visible |> list.drop(first) |> list.take(count)
          let selected = case modified {
            True -> list.unique(list.append(selected, range))
            False -> range
          }
          #(selected, Some(start))
        }
        _, _ -> #([row], Some(row))
      }
    _, _ -> {
      let selected = case modified {
        True ->
          case list.any(selected, same_row(_, row)) {
            True -> list.filter(selected, fn(old) { !same_row(old, row) })
            False -> list.append(selected, [row])
          }
        False -> [row]
      }
      #(selected, Some(row))
    }
  }
}

fn row_index(rows: List(Row), target: Row, index: Int) -> Result(Int, Nil) {
  case rows {
    [] -> Error(Nil)
    [first, ..rest] ->
      case same_row(first, target) {
        True -> Ok(index)
        False -> row_index(rest, target, index + 1)
      }
  }
}

fn fetch_results(groups: List(Group)) -> Effect(Msg) {
  let body =
    json.object([
      #(
        "groups",
        json.array(groups, fn(group) {
          json.object([
            #("id", json.string(int.to_string(group.id))),
            #("name", json.string(group.name)),
            #("paths", json.array(group.paths, json.string)),
          ])
        }),
      ),
    ])
  rsvp.post(
    "/api/results",
    body,
    rsvp.expect_json(results_decoder(), ResultsLoaded),
  )
}

fn fetch_browse(path: String) -> Effect(Msg) {
  rsvp.get(
    "/api/browse?path=" <> uri.percent_encode(path),
    rsvp.expect_json(browse_decoder(), BrowseLoaded),
  )
}

pub fn results_decoder() -> decode.Decoder(Results) {
  use rows <- decode.field("rows", decode.list(row_decoder()))
  use errors <- decode.field("errors", decode.list(decode.string))
  decode.success(Results(rows:, errors:))
}

fn row_decoder() -> decode.Decoder(Row) {
  use group_id <- decode.optional_field("group_id", "", decode.string)
  use group <- decode.field("group", decode.string)
  use source <- decode.field("source", decode.string)
  use task <- decode.field("task", decode.string)
  use rep <- decode.field("rep", decode.int)
  use model <- decode.field("model", decode.string)
  use effort <- decode.optional_field(
    "effort",
    None,
    decode.optional(decode.string),
  )
  // older rows keep effort apart from the model name
  let model = case effort {
    Some(effort) ->
      case string.ends_with(model, " (" <> effort <> ")") {
        True -> model
        False -> model <> " (" <> effort <> ")"
      }
    None -> model
  }
  use arm <- decode.optional_field("arm", "", decode.string)
  use hidden_pass <- decode.field("hidden_pass", decode.bool)
  use tokens <- decode.optional_field(
    "tokens",
    None,
    decode.optional(decode.int),
  )
  use work_tokens <- decode.optional_field(
    "work_tokens",
    None,
    decode.optional(decode.int),
  )
  use uncached_tokens <- decode.optional_field(
    "uncached_tokens",
    None,
    decode.optional(decode.int),
  )
  use output_tokens <- decode.optional_field(
    "output_tokens",
    None,
    decode.optional(decode.int),
  )
  use duration_ms <- decode.optional_field(
    "duration_ms",
    None,
    decode.optional(decode.int),
  )
  use cost_usd <- decode.optional_field(
    "cost_usd",
    None,
    decode.optional(decode.float),
  )
  decode.success(Row(
    group_id:,
    group:,
    source:,
    task:,
    rep:,
    model:,
    arm:,
    hidden_pass:,
    tokens:,
    work_tokens:,
    uncached_tokens:,
    output_tokens:,
    duration_ms:,
    cost_usd:,
  ))
}

fn browse_decoder() -> decode.Decoder(BrowseResponse) {
  use path <- decode.field("path", decode.string)
  use parent <- decode.field("parent", decode.optional(decode.string))
  use entries <- decode.field("entries", decode.list(entry_decoder()))
  decode.success(BrowseResponse(path:, parent:, entries:))
}

fn entry_decoder() -> decode.Decoder(Entry) {
  use name <- decode.field("name", decode.string)
  use path <- decode.field("path", decode.string)
  use kind <- decode.field("kind", decode.string)
  decode.success(Entry(name:, path:, kind:))
}

fn view(model: Model) -> Element(Msg) {
  html.div([], [
    html.header([], [
      html.h1([], [html.text("Bench Visualizer")]),
      button("Refresh", Refresh),
    ]),
    html.main([], [
      html.div([attribute.class("toolbar")], [
        button("+ Comparison group", AddGroup),
        html.span([attribute.class("status")], [
          html.text(case model.loading {
            True -> "Loading…"
            False -> int.to_string(list.length(model.rows)) <> " rows"
          }),
        ]),
      ]),
      html.section(
        [attribute.class("groups")],
        list.map(model.groups, view_group),
      ),
      html.div([attribute.class("status")], [
        html.text(string.join(model.errors, " · ")),
      ]),
      view_table(model),
      html.p([attribute.class("status")], [
        html.text(
          "Click a column to sort; Shift-click a second. Click a row to select it, Ctrl-click to toggle, or Shift-click to select a range. The chart averages runs per selected task, then tasks per group.",
        ),
      ]),
      html.p([attribute.class("status")], [
        html.text(
          "Total = cached input + uncached input + output. Work excludes repeated initial context.",
        ),
      ]),
      view_chart(model),
    ]),
    view_picker(model),
  ])
}

fn button(label: String, message: Msg) -> Element(Msg) {
  html.button([attribute.type_("button"), event.on_click(message)], [
    html.text(label),
  ])
}

fn view_group(group: Group) -> Element(Msg) {
  html.article([attribute.class("group")], [
    html.div([attribute.class("group-head")], [
      html.input([
        attribute.aria_label("Group name"),
        attribute.value(group.name),
        event.on_change(fn(name) { RenameGroup(group.id, name) }),
      ]),
      html.button(
        [
          attribute.class("icon"),
          attribute.type_("button"),
          attribute.aria_label("Delete group"),
          event.on_click(DeleteGroup(group.id)),
        ],
        [html.text("×")],
      ),
    ]),
    html.div([attribute.class("paths")], case group.paths {
      [] -> [
        html.span([attribute.class("status")], [
          html.text("No files or folders selected"),
        ]),
      ]
      paths ->
        list.map(paths, fn(path) {
          html.span([attribute.class("path")], [
            html.span([attribute.title(path)], [html.text(path)]),
            html.button(
              [
                attribute.type_("button"),
                attribute.aria_label("Remove path"),
                event.on_click(RemovePath(group.id, path)),
              ],
              [html.text("×")],
            ),
          ])
        })
    }),
    button("Select files / folders", OpenPicker(group.id)),
  ])
}

fn view_table(model: Model) -> Element(Msg) {
  let headings = [
    #("group", "Group"),
    #("rep", "Rep"),
    #("arm", "Arm"),
    #("task", "Task"),
    #("model", "Model"),
    #("pass", "Pass"),
    #("tokens", "Total tokens"),
    #("work_tokens", "Work tokens"),
    #("cached_input", "Cached input"),
    #("uncached_input", "Uncached input"),
    #("output_tokens", "Output tokens"),
    #("duration_ms", "Duration (s)"),
    #("cost_usd", "Cost (USD)"),
  ]
  html.div([attribute.class("table-wrap")], [
    html.table([], [
      html.thead([], [
        html.tr([], list.map(headings, view_heading(_, model.sorts))),
      ]),
      html.tbody([], case model.rows {
        [] -> [
          html.tr([], [
            html.td([attribute.class("empty"), attribute.colspan(13)], [
              html.text("Add result files or folders to a comparison group."),
            ]),
          ]),
        ]
        rows ->
          list.map(sort_rows(rows, model.sorts), view_row(_, model.selected))
      }),
    ]),
  ])
}

fn view_heading(heading: #(String, String), sorts: List(Sort)) -> Element(Msg) {
  let #(key, label) = heading
  let #(indicator, aria_sort) = case sorts {
    [Sort(first, descending), ..] if first == key -> #(
      case descending {
        True -> " ▼1"
        False -> " ▲1"
      },
      case descending {
        True -> "descending"
        False -> "ascending"
      },
    )
    [_, Sort(second, descending)] if second == key -> #(
      case descending {
        True -> " ▼2"
        False -> " ▲2"
      },
      "none",
    )
    _ -> #("", "none")
  }
  html.th([attribute.aria_sort(aria_sort)], [
    html.button(
      [
        attribute.type_("button"),
        attribute.title("Click to sort; Shift-click for a second column"),
        event.on("click", {
          use shifted <- decode.field("shiftKey", decode.bool)
          decode.success(SortBy(key, shifted))
        }),
      ],
      [html.text(label <> indicator)],
    ),
  ])
}

fn view_row(row: Row, selected: List(Row)) -> Element(Msg) {
  let is_selected = list.any(selected, same_row(_, row))
  html.tr(
    [
      attribute.class(case is_selected {
        True -> "selected"
        False -> ""
      }),
      attribute.tabindex(0),
      attribute.aria_selected(is_selected),
      event.on("click", {
        use shifted <- decode.field("shiftKey", decode.bool)
        use ctrl <- decode.field("ctrlKey", decode.bool)
        use meta <- decode.field("metaKey", decode.bool)
        decode.success(SelectRow(row, shifted, ctrl || meta))
      }),
      event.advanced("keydown", {
        use key <- decode.field("key", decode.string)
        use shifted <- decode.field("shiftKey", decode.bool)
        use ctrl <- decode.field("ctrlKey", decode.bool)
        use meta <- decode.field("metaKey", decode.bool)
        case key {
          "Enter" | " " ->
            decode.success(event.handler(
              SelectRow(row, shifted, ctrl || meta),
              prevent_default: True,
              stop_propagation: False,
            ))
          _ ->
            decode.failure(
              event.handler(
                SelectRow(row, False, False),
                prevent_default: False,
                stop_propagation: False,
              ),
              expected: "selection key",
            )
        }
      }),
    ],
    [
      cell(row.group),
      cell(int.to_string(row.rep)),
      cell(case row.arm {
        "" -> "—"
        arm -> arm
      }),
      cell(short_task(row.task)),
      cell(row.model),
      html.td(
        [
          attribute.class(case row.hidden_pass {
            True -> "pass"
            False -> "fail"
          }),
        ],
        [
          html.text(case row.hidden_pass {
            True -> "✓"
            False -> "✕"
          }),
        ],
      ),
      cell(format_tokens(row.tokens)),
      cell(format_tokens(row.work_tokens)),
      cell(format_tokens(cached_input(row))),
      cell(format_tokens(uncached_input(row))),
      cell(format_tokens(row.output_tokens)),
      cell(format_duration(row.duration_ms)),
      cell(format_float(row.cost_usd)),
    ],
  )
}

fn cell(content: String) -> Element(Msg) {
  html.td([], [html.text(content)])
}

pub fn format_tokens(value: Option(Int)) -> String {
  case value {
    Some(value) -> compact_tokens(int.to_float(value))
    None -> "—"
  }
}

fn compact_tokens(value: Float) -> String {
  case value >. 10_000.0 {
    True ->
      float.to_string(int.to_float(float.round(value /. 100.0)) /. 10.0) <> "k"
    False -> int.to_string(float.round(value))
  }
}

pub fn format_duration(value: Option(Int)) -> String {
  case value {
    Some(ms) -> float.to_string(int.to_float(ms) /. 1000.0) <> "s"
    None -> "—"
  }
}

pub fn cached_input(row: Row) -> Option(Int) {
  case row.tokens, row.uncached_tokens {
    Some(total), Some(uncached) -> Some(total - uncached)
    _, _ -> None
  }
}

pub fn uncached_input(row: Row) -> Option(Int) {
  case row.uncached_tokens, row.output_tokens {
    Some(uncached), Some(output) -> Some(uncached - output)
    _, _ -> None
  }
}

fn format_float(value: Option(Float)) -> String {
  case value {
    Some(value) -> "$" <> float.to_string(value)
    None -> "—"
  }
}

fn view_chart(model: Model) -> Element(Msg) {
  case model.selected {
    [] -> html.text("")
    _ -> {
      let chart_rows = chart_rows(model.rows, model.selected)
      let series =
        list.filter_map(model.groups, fn(group) {
          let rows =
            list.filter(chart_rows, fn(row) {
              row.group_id == int.to_string(group.id)
            })
          case rows {
            [] -> Error(Nil)
            _ -> Ok(Series(group, rows))
          }
        })
      let groups =
        series
        |> list.map(fn(item) { item.group })
        |> list.unique
      let metrics = [
        #(Tokens, "Total tokens"),
        #(WorkTokens, "Work tokens"),
        #(CachedInput, "Cached input tokens"),
        #(UncachedInput, "Uncached input tokens"),
        #(OutputTokens, "Output tokens"),
        #(Duration, "Duration (s)"),
        #(Cost, "Cost (USD)"),
      ]
      html.section([attribute.class("chart")], [
        html.div([attribute.class("chart-head")], [
          html.h2([], [html.text("Selected tasks — average by group")]),
        ]),
        html.div(
          [attribute.class("chart-legend")],
          list.map(groups, fn(group) {
            html.span([], [
              html.span(
                [
                  attribute.class("legend-swatch"),
                  attribute.style("background-color", group_color(group.id)),
                ],
                [],
              ),
              html.text(group.name),
            ])
          }),
        ),
        html.div(
          [attribute.class("chart-graph")],
          list.map(metrics, fn(metric) {
            let #(kind, label) = metric
            view_metric_chart(kind, label, series)
          }),
        ),
      ])
    }
  }
}

pub fn chart_rows(rows: List(Row), selected: List(Row)) -> List(Row) {
  list.filter(rows, fn(row) {
    list.any(selected, fn(chosen) {
      chosen.group_id == row.group_id
      && short_task(chosen.task) == short_task(row.task)
    })
  })
}

pub fn short_task(task: String) -> String {
  case string.split(task, "-") {
    [first, ..] -> first
    [] -> task
  }
}

fn group_color(id: Int) -> String {
  "hsl(" <> int.to_string({ id - 1 } * 137 % 360) <> " 65% 65%)"
}

fn view_metric_chart(
  metric: Metric,
  label: String,
  series: List(Series),
) -> Element(Msg) {
  let points =
    list.filter_map(series, fn(item) {
      case average_per_task(item.rows, metric) {
        Some(#(value, count)) -> Ok(#(item, value, count))
        None -> Error(Nil)
      }
    })
  let maximum =
    list.fold(points, 0.0, fn(maximum, point) {
      let #(_, value, _) = point
      float.max(maximum, value)
    })
  let maximum = case maximum <=. 0.0 {
    True -> 1.0
    False -> maximum
  }
  html.div([attribute.class("metric-row")], [
    html.h3([], [html.text(label)]),
    case points {
      [] -> html.p([attribute.class("status")], [html.text("No data")])
      _ ->
        html.div(
          [attribute.class("metric-bars")],
          list.map(points, view_bar(_, metric, maximum)),
        )
    },
  ])
}

fn metric_value(row: Row, metric: Metric) -> Option(Float) {
  case metric {
    Tokens -> option.map(row.tokens, int.to_float)
    WorkTokens -> option.map(row.work_tokens, int.to_float)
    CachedInput -> option.map(cached_input(row), int.to_float)
    UncachedInput -> option.map(uncached_input(row), int.to_float)
    OutputTokens -> option.map(row.output_tokens, int.to_float)
    Duration ->
      option.map(row.duration_ms, fn(ms) { int.to_float(ms) /. 1000.0 })
    Cost -> row.cost_usd
  }
}

pub fn average(rows: List(Row), metric: Metric) -> Option(#(Float, Int)) {
  let #(sum, count) =
    list.fold(rows, #(0.0, 0), fn(state, row) {
      let #(sum, count) = state
      case metric_value(row, metric) {
        Some(value) -> #(sum +. value, count + 1)
        None -> state
      }
    })
  case count {
    0 -> None
    _ -> Some(#(sum /. int.to_float(count), count))
  }
}

pub fn average_per_task(
  rows: List(Row),
  metric: Metric,
) -> Option(#(Float, Int)) {
  let tasks = rows |> list.map(fn(row) { short_task(row.task) }) |> list.unique
  let #(sum, count) =
    list.fold(tasks, #(0.0, 0), fn(state, task) {
      let #(sum, count) = state
      let task_rows =
        list.filter(rows, fn(row) { short_task(row.task) == task })
      case average(task_rows, metric) {
        Some(#(value, _)) -> #(sum +. value, count + 1)
        None -> state
      }
    })
  case count {
    0 -> None
    _ -> Some(#(sum /. int.to_float(count), count))
  }
}

fn format_metric(value: Float, metric: Metric) -> String {
  case metric {
    Duration ->
      float.to_string(int.to_float(float.round(value *. 100.0)) /. 100.0) <> "s"
    Cost ->
      "$"
      <> float.to_string(
        int.to_float(float.round(value *. 10_000.0)) /. 10_000.0,
      )
    _ -> compact_tokens(value)
  }
}

fn view_bar(
  point: #(Series, Float, Int),
  metric: Metric,
  maximum: Float,
) -> Element(Msg) {
  let #(series, value, count) = point
  html.div([attribute.class("chart-item")], [
    html.span([attribute.class("chart-label")], [
      html.text(series.group.name),
    ]),
    html.div([attribute.class("chart-bar-space")], [
      html.div(
        [
          attribute.class("chart-bar"),
          attribute.style(
            "width",
            float.to_string(100.0 *. value /. maximum) <> "%",
          ),
          attribute.style("background-color", group_color(series.group.id)),
          attribute.title(
            "Average of "
            <> int.to_string(count)
            <> case count {
              1 -> " task"
              _ -> " tasks"
            },
          ),
        ],
        [],
      ),
    ]),
    html.strong([attribute.class("chart-value")], [
      html.text(format_metric(value, metric)),
    ]),
  ])
}

fn view_picker(model: Model) -> Element(Msg) {
  case model.picker_group {
    None -> html.text("")
    Some(_) ->
      html.div([attribute.class("modal")], [
        html.div([attribute.class("dialog")], [
          html.div([attribute.class("dialog-head")], [
            case model.parent {
              Some(parent) -> button("↑", Browse(parent))
              None -> html.text("")
            },
            html.strong([], [html.text(model.browse_path)]),
            button("Add this folder", AddPath(model.browse_path)),
            button("Close", ClosePicker),
          ]),
          html.div([attribute.class("entries")], case model.entries {
            [] -> [
              html.div([attribute.class("empty")], [
                html.text("No result files here."),
              ]),
            ]
            entries -> list.map(entries, view_entry)
          }),
        ]),
      ])
  }
}

fn view_entry(entry: Entry) -> Element(Msg) {
  html.div([attribute.class("entry")], [
    html.span([], [
      html.text(case entry.kind {
        "directory" -> "▸ " <> entry.name
        _ -> "▤ " <> entry.name
      }),
    ]),
    case entry.kind {
      "directory" -> button("Open", Browse(entry.path))
      _ -> html.text("")
    },
    button("Add", AddPath(entry.path)),
  ])
}
