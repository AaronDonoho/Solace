import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
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
    group: String,
    source: String,
    task: String,
    rep: Int,
    model: String,
    arm: String,
    hidden_pass: Bool,
    tokens: Option(Int),
    work_tokens: Option(Int),
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

type Metric {
  Tokens
  Duration
  Cost
}

type Model {
  Model(
    groups: List(Group),
    rows: List(Row),
    selected: Option(Row),
    metric: Metric,
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
  SelectRow(Row)
  SetMetric(String)
}

fn init(_) -> #(Model, Effect(Msg)) {
  let groups = [Group(1, "Comparison 1", [])]
  #(
    Model(
      groups:,
      rows: [],
      selected: None,
      metric: Tokens,
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
          Group(
            model.next_group,
            "Comparison " <> int.to_string(model.next_group),
            [],
          ),
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
        loading: False,
      ),
      effect.none(),
    )
    ResultsLoaded(Error(error)) -> #(
      Model(..model, errors: [string.inspect(error)], loading: False),
      effect.none(),
    )
    SelectRow(row) -> #(Model(..model, selected: Some(row)), effect.none())
    SetMetric(name) -> #(
      Model(..model, metric: case name {
        "duration_ms" -> Duration
        "cost_usd" -> Cost
        _ -> Tokens
      }),
      effect.none(),
    )
  }
}

fn reload(model: Model, groups: List(Group)) -> #(Model, Effect(Msg)) {
  #(
    Model(..model, groups:, selected: None, loading: True),
    fetch_results(groups),
  )
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

fn keep_selected(selected: Option(Row), rows: List(Row)) -> Option(Row) {
  case selected {
    None -> None
    Some(old) ->
      case
        list.find(rows, fn(row) {
          row.group == old.group
          && row.source == old.source
          && row.task == old.task
          && row.rep == old.rep
        })
      {
        Ok(row) -> Some(row)
        Error(_) -> None
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
  use group <- decode.field("group", decode.string)
  use source <- decode.field("source", decode.string)
  use task <- decode.field("task", decode.string)
  use rep <- decode.field("rep", decode.int)
  use model <- decode.field("model", decode.string)
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
    group:,
    source:,
    task:,
    rep:,
    model:,
    arm:,
    hidden_pass:,
    tokens:,
    work_tokens:,
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
    "Group",
    "Source",
    "Task",
    "Rep",
    "Model",
    "Arm",
    "Pass",
    "Tokens",
    "Work",
    "Duration",
    "Cost",
  ]
  html.div([attribute.class("table-wrap")], [
    html.table([], [
      html.thead([], [
        html.tr(
          [],
          list.map(headings, fn(name) { html.th([], [html.text(name)]) }),
        ),
      ]),
      html.tbody([], case model.rows {
        [] -> [
          html.tr([], [
            html.td([attribute.class("empty"), attribute.colspan(11)], [
              html.text("Add result files or folders to a comparison group."),
            ]),
          ]),
        ]
        rows -> list.map(rows, view_row(_, model.selected))
      }),
    ]),
  ])
}

fn view_row(row: Row, selected: Option(Row)) -> Element(Msg) {
  let selected_class = case selected {
    Some(value) if value == row -> "selected"
    _ -> ""
  }
  html.tr([attribute.class(selected_class), event.on_click(SelectRow(row))], [
    cell(row.group),
    cell(row.source),
    cell(row.task),
    cell(int.to_string(row.rep)),
    cell(row.model),
    cell(case row.arm {
      "" -> "—"
      arm -> arm
    }),
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
    cell(format_int(row.tokens)),
    cell(format_int(row.work_tokens)),
    cell(format_int(row.duration_ms)),
    cell(format_float(row.cost_usd)),
  ])
}

fn cell(content: String) -> Element(Msg) {
  html.td([], [html.text(content)])
}

fn format_int(value: Option(Int)) -> String {
  case value {
    Some(value) -> int.to_string(value)
    None -> "—"
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
    None -> html.text("")
    Some(selected) -> {
      let peers =
        list.filter(model.rows, fn(row) {
          row.task == selected.task && metric_value(row, model.metric) != None
        })
      let maximum =
        list.fold(peers, 1.0, fn(maximum, row) {
          metric_value(row, model.metric)
          |> option.unwrap(0.0)
          |> float.max(maximum)
        })
      html.section([attribute.class("chart")], [
        html.div([attribute.class("chart-head")], [
          html.h2([], [html.text(selected.task <> " comparison")]),
          html.label([], [
            html.text("Metric "),
            html.select([event.on_change(SetMetric)], [
              metric_option("tokens", "Tokens", model.metric == Tokens),
              metric_option(
                "duration_ms",
                "Duration (ms)",
                model.metric == Duration,
              ),
              metric_option("cost_usd", "Cost (USD)", model.metric == Cost),
            ]),
          ]),
        ]),
        html.div(
          [attribute.class("bars")],
          list.map(peers, view_bar(_, model.metric, maximum)),
        ),
      ])
    }
  }
}

fn metric_option(value: String, label: String, selected: Bool) -> Element(Msg) {
  html.option([attribute.value(value), attribute.selected(selected)], label)
}

fn metric_value(row: Row, metric: Metric) -> Option(Float) {
  case metric {
    Tokens -> option.map(row.tokens, int.to_float)
    Duration -> option.map(row.duration_ms, int.to_float)
    Cost -> row.cost_usd
  }
}

fn view_bar(row: Row, metric: Metric, maximum: Float) -> Element(Msg) {
  let value = metric_value(row, metric) |> option.unwrap(0.0)
  html.div([], [
    html.div([attribute.class("bar-label")], [
      html.span([], [
        html.text(
          row.group
          <> " · "
          <> row.source
          <> " · run "
          <> int.to_string(row.rep),
        ),
      ]),
      html.strong([], [html.text(float.to_string(value))]),
    ]),
    html.div([attribute.class("track")], [
      html.div(
        [
          attribute.class("fill"),
          attribute.style(
            "width",
            float.to_string(100.0 *. value /. maximum) <> "%",
          ),
        ],
        [],
      ),
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
