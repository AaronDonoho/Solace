import changes
import diagram
import gleam/dict
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
import panel
import plan_panel
import render
import solace_task

/// Registers the element on its own, for `index.html`.
pub fn main() -> Nil {
  let assert Ok(_) = register()
  Nil
}

pub type Tab {
  Architecture
  UiUx
}

pub type Model {
  Model(
    tab: Tab,
    designs: List(Design),
    /// The design id in the address, "" for none.
    open: String,
    drawing: Option(render.Design),
    design_error: String,
    /// A restore refusal that belongs to the page rather than one version.
    restore_error: String,
    loading: Bool,
    error: String,
    source: Option(solace_task.Source),
    connected: Bool,
    caught_up: Bool,
    replay: List(String),
    expanded: List(String),
    histories: List(History),
    viewed: Option(Viewed),
    /// The screen the click-through preview shows, if previewing.
    preview: Option(String),
    /// Whether the canvas shows an open proposal rather than the saved design.
    view_proposal: Bool,
    /// Whether the proposal's change outlines show.
    show_changes: Bool,
    /// The agent panel's state, shared by both tabs.
    panel: panel.Model,
  )
}

pub type Msg {
  Select(Tab)
  /// The shell shows this tab: a load, Back or Forward.
  TabShown(Tab)
  /// The shell shows this design: a load, Back or Forward.
  DesignShown(String)
  /// A design in the list was picked.
  SelectDesign(String)
  /// The reply to `GET /designs/{id}`, with the id it was asked for.
  DesignLoaded(String, String)
  /// The reply to `GET /designs`.
  DesignsLoaded(String)
  Connected
  Disconnected
  Listening(solace_task.Source)
  Live(String)
  CaughtUp
  ToggleHistory(String)
  HistoryLoaded(String, String)
  ViewVersion(String, Int)
  VersionLoaded(String, Int, String)
  Restore(String, Int)
  Restored(String, Int, String)
  /// Starts the click-through preview at the first screen.
  Preview
  /// Leaves the preview and shows every screen again.
  ExitPreview
  /// A click in the preview, with the element's id.
  PreviewClick(String)
  /// Shows the open proposal instead of the saved design.
  ShowProposed
  /// Shows the saved design while a proposal is open.
  ShowCurrent
  /// Shows or hides the proposal's change outlines.
  ToggleChanges
  /// A message for the agent panel.
  Panel(panel.Msg)
}

pub fn tabs() -> List(Tab) {
  [Architecture, UiUx]
}

/// The tab's name in the URL: `/design?tab=<name>`.
pub fn tab_name(tab: Tab) -> String {
  case tab {
    Architecture -> "architecture"
    UiUx -> "ui-ux"
  }
}

pub fn label(tab: Tab) -> String {
  case tab {
    Architecture -> "Architecture"
    UiUx -> "UI/UX"
  }
}

/// The tab a URL query names ("?tab=ui-ux"); Architecture when it names
/// none or an unknown one.
pub fn tab_from(query: String) -> Tab {
  let named =
    query
    |> string.replace("?", "")
    |> string.split("&")
    |> list.find_map(fn(pair) {
      let name = string.remove_prefix(pair, "tab=")
      case name == pair {
        True -> Error(Nil)
        False -> Ok(name)
      }
    })
  case named {
    Ok(name) ->
      case list.find(tabs(), fn(tab) { tab_name(tab) == name }) {
        Ok(tab) -> tab
        Error(_) -> Architecture
      }
    Error(_) -> Architecture
  }
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(new(Architecture), effect.none())
}

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    Select(tab) if tab == model.tab -> #(model, effect.none())
    Select(tab) -> #(
      Model(
        ..model,
        tab: tab,
        open: "",
        drawing: None,
        design_error: "",
        restore_error: "",
        expanded: [],
        histories: [],
        viewed: None,
        preview: None,
      ),
      event.emit("tab", json.string(tab_name(tab))),
    )
    TabShown(tab) if tab == model.tab -> #(model, effect.none())
    TabShown(tab) -> {
      let model =
        Model(
          ..model,
          tab: tab,
          restore_error: "",
          expanded: [],
          histories: [],
          viewed: None,
          preview: None,
        )
      #(model, load(model))
    }
    DesignShown(id) if id == model.open -> #(model, effect.none())
    DesignShown(id) -> {
      let model =
        Model(
          ..clear_history_error(model),
          open: id,
          drawing: None,
          design_error: "",
          restore_error: "",
          viewed: None,
          preview: None,
        )
      #(model, load(model))
    }
    SelectDesign(id) -> {
      let model =
        Model(
          ..clear_history_error(model),
          open: id,
          drawing: None,
          design_error: "",
          restore_error: "",
          viewed: None,
          preview: None,
        )
      #(
        model,
        effect.batch([
          event.emit("design", json.string(id)),
          load(model),
        ]),
      )
    }
    DesignLoaded(id, text) -> #(load_design(model, id, text), effect.none())
    DesignsLoaded(text) -> #(loaded(model, text), effect.none())
    ToggleHistory(id) -> toggle_history(model, id)
    HistoryLoaded(id, text) -> #(load_history(model, id, text), effect.none())
    ViewVersion(id, number) -> view_version(model, id, number)
    VersionLoaded(id, number, text) -> #(
      load_version(model, id, number, text),
      effect.none(),
    )
    Restore(id, number) -> restore(model, id, number)
    Restored(id, number, text) -> restored(model, id, number, text)
    Preview -> start_preview(model)
    ExitPreview -> #(Model(..model, preview: None), effect.none())
    PreviewClick(id) -> preview_click(model, id)
    ShowProposed -> #(Model(..model, view_proposal: True), effect.none())
    ShowCurrent -> #(Model(..model, view_proposal: False), effect.none())
    ToggleChanges -> #(
      Model(..model, show_changes: !model.show_changes),
      effect.none(),
    )
    Connected -> {
      let #(model, fx) =
        start(
          Model(..model, connected: True, panel: panel.connect(model.panel)),
        )
      let #(state, panel_fx) = panel.resume(model.panel)
      #(
        Model(..model, panel: state),
        effect.batch([
          fx,
          effect.map(panel_fx, Panel),
          effect.before_paint(fn(send, root) {
            send(Panel(panel.Attached(root)))
          }),
        ]),
      )
    }
    Disconnected -> #(
      Model(
        ..stop(model),
        connected: False,
        panel: panel.disconnected(model.panel),
      ),
      effect.none(),
    )
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> {
      let before = model.panel
      let updated = apply(model, text)
      let answered = proposal_answered(model, updated)
      let updated = case answered {
        True -> Model(..updated, drawing: None)
        False -> updated
      }
      let #(after, panel_fx) = panel.follow(before, updated.panel)
      let follows = case panel.shown(before), panel.shown(after) {
        Some(a), Some(b) -> a.id == b.id && a.state != b.state
        _, _ -> False
      }
      let attention = panel.needs_attention(before.runs, after.runs)
      let effects =
        list.flatten([
          case updated.designs == model.designs && !answered {
            True -> []
            False -> [reload(updated)]
          },
          case follows {
            True -> [effect.map(panel_fx, Panel)]
            False -> []
          },
          case attention {
            True -> [event.emit("attention", json.null())]
            False -> []
          },
          case panel.models_event(text) {
            True -> [effect.map(panel.refresh_models(after), Panel)]
            False -> []
          },
        ])
      let model = Model(..updated, panel: after)
      let fx = case effects {
        [] -> effect.none()
        [one] -> one
        many -> effect.batch(many)
      }
      #(model, fx)
    }
    CaughtUp -> {
      let #(model, fx) = settle(model)
      let #(state, panel_fx) = panel.resume(model.panel)
      #(
        Model(..model, panel: state),
        effect.batch([
          fx,
          effect.map(panel_fx, Panel),
          effect.map(panel.load_models(state), Panel),
        ]),
      )
    }
    Panel(msg) -> {
      let #(state, fx) = panel.update(model.panel, msg)
      #(Model(..model, panel: state), effect.map(fx, Panel))
    }
  }
}

pub fn view(model: Model) -> Element(Msg) {
  html.div([attribute.class("design")], [
    html.style([], view_styles),
    html.div(
      [
        attribute.class("tabs"),
        attribute.role("tablist"),
        attribute.attribute("aria-label", "Design"),
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
    element.map(panel.view(model.panel), Panel),
    ..list.map(tabs(), fn(tab) {
      html.section(
        [
          attribute.id("panel-" <> tab_name(tab)),
          attribute.role("tabpanel"),
          attribute.attribute("aria-labelledby", "tab-" <> tab_name(tab)),
          attribute.hidden(tab != model.tab),
        ],
        [
          html.div([attribute.class("design-body")], [
            html.div([attribute.class("design-list")], [
              case model.error {
                "" -> element.none()
                message ->
                  html.p([attribute.role("alert")], [html.text(message)])
              },
              case tab == model.tab {
                False -> element.none()
                True ->
                  case model.restore_error {
                    "" -> element.none()
                    message ->
                      html.p([attribute.role("alert")], [html.text(message)])
                  }
              },
              case designs_for(model, tab) {
                [] ->
                  html.p([attribute.class("design-empty")], [
                    html.text("No designs yet."),
                  ])
                designs ->
                  html.ul(
                    [],
                    list.map(designs, fn(design) { design_row(model, design) }),
                  )
              },
            ]),
            html.div([attribute.class("design-canvas")], canvas(model, tab)),
          ]),
        ],
      )
    })
  ])
}

const view_styles =
  solace_task.control_styles
  <> solace_task.tab_styles
  <> render.outline_styles
  <> diagram.styles
  <> ":host{display:block;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--well:#eceee6;--code:#eef1ec;--ok-text:#1a6b30}
.design{font:15px/1.45 'IBM Plex Sans',system-ui,sans-serif;--shell-height:56px;--tabs-height:44px;--view-height:calc(100dvh - var(--shell-height));height:var(--view-height);overflow:hidden;background:var(--sheet);color:var(--ink);display:grid;grid-template-columns:minmax(0,1fr) auto;grid-template-rows:auto minmax(0,1fr)}
.design>.tabs{grid-column:1/-1}
.design>.plan{grid-column:2;grid-row:2}
.design>[role=tabpanel]{grid-column:1;grid-row:2;min-width:0;min-height:0;overflow:auto;padding:var(--space-6)}
.design>[role=tabpanel][hidden]{display:none}
@media (max-width:900px){.design{display:block;height:auto;overflow:visible}.design>[role=tabpanel]{overflow:visible}}
.design-body{display:flex;gap:var(--space-4);height:100%;min-height:0}
.design-list{flex:0 0 240px;min-height:0;overflow:auto;border-right:1px solid var(--line);padding-right:var(--space-3)}
.design-list ul{list-style:none;margin:0;padding:0}
.design-list li{margin:0}
.design-row{display:flex;align-items:stretch;margin:0 0 var(--space-1)}
.design-row .design-item{margin:0;flex:1;min-width:0}
.design-item{display:block;width:100%;box-sizing:border-box;margin:0 0 var(--space-1);padding:var(--space-2);text-align:left;font:inherit;color:var(--ink);background:transparent;border:1px solid transparent;cursor:pointer}
.design-item:hover{background:var(--hover)}
.design-item[aria-current=true]{background:var(--paper);border-color:var(--edge);font-weight:700}
.design-arrow{flex:none;padding:0 var(--space-2);font:inherit;color:var(--ink);background:transparent;border:1px solid transparent;cursor:pointer}
.design-arrow:hover{background:var(--hover)}
.design-history{margin:0 0 var(--space-2);padding:0 0 0 var(--space-2)}
.design-version{display:block;width:100%;box-sizing:border-box;padding:var(--space-1) var(--space-2);text-align:left;font:inherit;color:var(--soft);background:transparent;border:1px solid transparent;cursor:pointer}
.design-version:hover{background:var(--hover)}
.design-version[aria-current=true]{background:var(--paper);border-color:var(--edge);color:var(--ink)}
.design-empty{color:var(--soft);margin:var(--space-2) 0}
.design-canvas{flex:1;min-width:0;min-height:0;overflow:auto;background:var(--paper);border:1px solid var(--line);padding:var(--pad-card)}
.design-canvas h2{margin:0 0 var(--space-3);font-size:var(--h2)}
.design-heading{display:flex;align-items:baseline;gap:var(--space-3);flex-wrap:wrap;margin:0 0 var(--space-3)}
.design-heading h2{margin:0}
.design-version-note{color:var(--soft)}
.design-restore{padding:var(--space-1) var(--space-2);font:inherit;color:var(--paper);background:var(--edge);border:1px solid var(--edge);cursor:pointer}
.design-preview{padding:var(--space-1) var(--space-2);font:inherit;color:var(--paper);background:var(--edge);border:1px solid var(--edge);cursor:pointer}
.design-choice{padding:var(--space-1) var(--space-2);font:inherit;color:var(--ink);background:transparent;border:1px solid var(--line);cursor:pointer}
.design-choice[aria-pressed=true]{background:var(--edge);color:var(--paper);border-color:var(--edge)}
.design-screens{display:flex;flex-direction:column;align-items:flex-start;gap:var(--space-4);height:100%}
@media (max-width:900px){.design-body{flex-direction:column;height:auto}.design-list{flex:none;overflow:visible;border-right:0;border-bottom:1px solid var(--line);padding:0 0 var(--space-3)}.design-screens{height:60vh}}
"
  <> solace_task.editor_styles
  <> plan_panel.styles

pub fn register() -> Result(Nil, lustre.Error) {
  lustre.register(
    lustre.component(init, update, view, [
      component.on_attribute_change("tab", fn(name) {
        Ok(TabShown(tab_from("?tab=" <> name)))
      }),
      component.on_attribute_change("design", fn(id) { Ok(DesignShown(id)) }),
      component.on_connect(Connected),
      component.on_disconnect(Disconnected),
      component.adopt_styles(False),
    ]),
    "solace-design",
  )
}

pub type Design {
  Design(id: String, title: String, tab: Tab)
}

/// The design id a URL query names ("?design=<id>"), "" for none.
pub fn design_from(query: String) -> String {
  let named =
    query
    |> string.replace("?", "")
    |> string.split("&")
    |> list.find_map(fn(pair) {
      let id = string.remove_prefix(pair, "design=")
      case id == pair {
        True -> Error(Nil)
        False -> Ok(id)
      }
    })
  case named {
    Ok(id) -> id
    Error(_) -> ""
  }
}

/// A page on `tab` with nothing loaded yet.
pub fn new(tab: Tab) -> Model {
  Model(
    tab,
    [],
    "",
    None,
    "",
    "",
    False,
    "",
    None,
    False,
    False,
    [],
    [],
    [],
    None,
    None,
    True,
    True,
    panel.new(),
  )
}

/// The designs a tab lists, in the server's order.
pub fn designs_for(model: Model, tab: Tab) -> List(Design) {
  list.filter(model.designs, fn(design) { design.tab == tab })
}

/// The design the address names, when it belongs to the tab shown.
pub fn open_design(model: Model) -> Option(Design) {
  case
    list.find(designs_for(model, model.tab), fn(design) {
      design.id == model.open
    })
  {
    Ok(design) -> Some(design)
    Error(_) -> None
  }
}

fn design_decoder() -> decode.Decoder(Design) {
  use id <- decode.field("id", decode.string)
  use title <- decode.field("title", decode.string)
  use tab <- decode.field("tab", decode.string)
  decode.success(Design(id, title, tab_of(tab)))
}

fn designs_decoder() -> decode.Decoder(List(Design)) {
  use designs <- decode.field("designs", decode.list(design_decoder()))
  decode.success(designs)
}

fn tab_of(name: String) -> Tab {
  case name {
    "ui" -> UiUx
    _ -> Architecture
  }
}

fn loaded(model: Model, text: String) -> Model {
  case json.parse(text, designs_decoder()) {
    Ok(designs) ->
      refresh_open(Model(..model, designs: designs, loading: False, error: ""))
    Error(_) -> Model(..model, loading: False, error: error(text))
  }
}

fn apply(model: Model, text: String) -> Model {
  let model = case json.parse(text, designs_decoder()) {
    Ok(designs) ->
      refresh_open(Model(..model, designs: designs, loading: False, error: ""))
    Error(_) -> model
  }
  Model(..model, panel: panel.live(model.panel, text))
}

fn settle(model: Model) -> #(Model, Effect(Msg)) {
  let designs = model.designs
  let runs = model.panel.runs
  let before = model
  let model = Model(..model, caught_up: True, loading: False)
  let model =
    list.fold(list.reverse(model.replay), Model(..model, replay: []), apply)
  let answered = proposal_answered(before, model)
  let model = case answered {
    True -> Model(..model, drawing: None)
    False -> model
  }
  let attention = panel.needs_attention(runs, model.panel.runs)
  let effects =
    list.flatten([
      case model.designs == designs && !answered {
        True -> []
        False -> [reload(model)]
      },
      case attention {
        True -> [event.emit("attention", json.null())]
        False -> []
      },
    ])
  let fx = case effects {
    [] -> effect.none()
    [one] -> one
    many -> effect.batch(many)
  }
  #(model, fx)
}

fn error(text: String) -> String {
  case reason(text) {
    Some(message) -> message
    None -> text
  }
}

fn start(model: Model) -> #(Model, Effect(Msg)) {
  let model =
    Model(
      ..stop(model),
      connected: True,
      caught_up: False,
      replay: [],
      loading: True,
      error: "",
    )
  #(
    model,
    effect.batch([
      load(model),
      get("/designs", DesignsLoaded),
      effect.from(fn(send) {
        send(
          Listening(
            solace_task.events("", fn(text) { send(Live(text)) }, fn() {
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
    Some(source) -> solace_task.close_events(source)
    None -> Nil
  }
  Model(..model, source: None)
}

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request("", path, "GET", "", fn(text) { send(msg(text)) })
  })
}

@external(javascript, "./api.mjs", "request")
fn request(
  base: String,
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

fn refresh_open(model: Model) -> Model {
  let listed = designs_for(model, model.tab)
  let model =
    Model(
      ..model,
      expanded: list.filter(model.expanded, fn(id) {
        list.any(listed, fn(design) { design.id == id })
      }),
      histories: list.filter(model.histories, fn(history) {
        list.any(listed, fn(design) { design.id == history.id })
      }),
    )
  case open_design(model) {
    Some(_) -> model
    None ->
      Model(
        ..model,
        drawing: None,
        design_error: "",
        viewed: None,
        preview: None,
      )
  }
}

fn load_design(model: Model, id: String, text: String) -> Model {
  case id == model.open {
    True ->
      case render.decode(text) {
        Ok(drawing) ->
          Model(
            ..model,
            drawing: Some(drawing),
            design_error: "",
            preview: keep_preview(model.preview, drawing),
          )
        Error(_) ->
          Model(
            ..model,
            drawing: None,
            design_error: load_failure(text),
            preview: None,
          )
      }
    False -> model
  }
}

fn load(model: Model) -> Effect(Msg) {
  case model.open {
    "" -> effect.none()
    id -> get("/designs/" <> id, fn(text) { DesignLoaded(id, text) })
  }
}

fn reload(model: Model) -> Effect(Msg) {
  case open_design(model) {
    Some(design) ->
      case list.contains(model.expanded, design.id) {
        False -> load(model)
        True ->
          effect.batch([
            load(model),
            get("/designs/" <> design.id <> "/versions", fn(text) {
              HistoryLoaded(design.id, text)
            }),
          ])
      }
    None -> effect.none()
  }
}

fn canvas(model: Model, tab: Tab) -> List(Element(Msg)) {
  case tab == model.tab {
    False -> []
    True ->
      case viewed_for(model) {
        Some(viewed) -> version_canvas(model, viewed)
        None ->
          case open_design(model) {
            Some(design) -> current_canvas(model, design)
            None -> create_canvas(model, tab)
          }
      }
  }
}

fn reason(text: String) -> Option(String) {
  case json.parse(text, decode.at(["message"], decode.string)) {
    Ok(message) -> Some(message)
    Error(_) -> None
  }
}

fn load_failure(text: String) -> String {
  let message = "This design could not be loaded."
  case reason(text) {
    Some(reason) -> message <> " " <> without_code(code(text), reason)
    None -> message
  }
}

fn code(text: String) -> Option(String) {
  case json.parse(text, decode.at(["code"], decode.string)) {
    Ok(code) -> Some(code)
    Error(_) -> None
  }
}

fn without_code(code: Option(String), message: String) -> String {
  case code {
    Some(code) ->
      case string.split_once(message, code) {
        Ok(#("", rest)) -> string.trim(rest)
        _ -> message
      }
    None -> message
  }
}

pub type Version {
  Version(number: Int, time: Int, source: VersionSource)
}

pub type VersionSource {
  Human
  Run
  Repo
}

pub type History {
  History(id: String, versions: List(Version), error: String)
}

pub type Viewed {
  Viewed(id: String, number: Int, drawing: Option(render.Design), error: String)
}

fn versions_decoder() -> decode.Decoder(List(Version)) {
  decode.list(version_decoder())
}

fn version_decoder() -> decode.Decoder(Version) {
  use number <- decode.field("number", decode.int)
  use time <- decode.field("time", decode.int)
  use source <- decode.field("source", source_decoder())
  decode.success(Version(number, time, source))
}

fn source_decoder() -> decode.Decoder(VersionSource) {
  use kind <- decode.field("kind", decode.string)
  decode.success(case kind {
    "run" -> Run
    "repo" -> Repo
    _ -> Human
  })
}

@external(javascript, "./api.mjs", "clock")
fn clock(seconds: Int) -> String

fn toggle_history(model: Model, id: String) -> #(Model, Effect(Msg)) {
  case list.contains(model.expanded, id) {
    True -> #(
      Model(
        ..model,
        expanded: list.filter(model.expanded, fn(other) { other != id }),
        histories: list.filter(model.histories, fn(history) { history.id != id }),
      ),
      effect.none(),
    )
    False -> {
      let model =
        Model(..model, expanded: [id, ..model.expanded], histories: [
          History(id, [], ""),
          ..model.histories
        ])
      #(
        model,
        get("/designs/" <> id <> "/versions", fn(text) {
          HistoryLoaded(id, text)
        }),
      )
    }
  }
}

fn load_history(model: Model, id: String, text: String) -> Model {
  case list.contains(model.expanded, id) {
    False -> model
    True ->
      case json.parse(text, versions_decoder()) {
        Ok(versions) -> {
          let older = case history_of(model, id) {
            Some(history) -> newest(history.versions) > newest(versions)
            None -> False
          }
          case older {
            True -> model
            False -> set_history(model, History(id, versions, ""))
          }
        }
        Error(_) -> {
          let loaded = case history_of(model, id) {
            Some(history) -> history.versions != []
            None -> False
          }
          case loaded {
            True -> model
            False ->
              set_history(
                model,
                History(
                  id,
                  [],
                  failure(text, "The history could not be loaded."),
                ),
              )
          }
        }
      }
  }
}

fn set_history(model: Model, history: History) -> Model {
  Model(..model, histories: [
    history,
    ..list.filter(model.histories, fn(existing) { existing.id != history.id })
  ])
}

fn history_of(model: Model, id: String) -> Option(History) {
  case list.find(model.histories, fn(history) { history.id == id }) {
    Ok(history) -> Some(history)
    Error(_) -> None
  }
}

fn history_view(model: Model, id: String, expanded: Bool) -> Option(History) {
  case expanded {
    True -> history_of(model, id)
    False -> None
  }
}

fn history_list(model: Model, history: History) -> Element(Msg) {
  html.div([attribute.class("design-history")], [
    case history.error {
      "" -> element.none()
      message -> html.p([attribute.role("alert")], [html.text(message)])
    },
    html.ul(
      [],
      list.map(history.versions, fn(version) {
        html.li([], [
          html.button(
            [
              attribute.class("design-version"),
              case viewing(model, history.id, version.number) {
                True -> attribute.attribute("aria-current", "true")
                False -> attribute.none()
              },
              event.on_click(ViewVersion(history.id, version.number)),
            ],
            [
              html.text("Version " <> int.to_string(version.number) <> " · "),
              html.time([attribute.attribute("datetime", stamp(version.time))], [
                html.text(clock(version.time)),
              ]),
              html.text(" · " <> source_name(version.source)),
            ],
          ),
        ])
      }),
    ),
  ])
}

fn viewing(model: Model, id: String, number: Int) -> Bool {
  case model.viewed {
    Some(viewed) -> viewed.id == id && viewed.number == number
    None -> False
  }
}

fn source_name(source: VersionSource) -> String {
  case source {
    Human -> "Person"
    Run -> "Agent"
    Repo -> "Repo"
  }
}

fn view_version(
  model: Model,
  id: String,
  number: Int,
) -> #(Model, Effect(Msg)) {
  let model = clear_history_error(model)
  let changed = id != model.open
  let model =
    Model(
      ..model,
      open: id,
      drawing: case changed {
        True -> None
        False -> model.drawing
      },
      design_error: "",
      restore_error: "",
      viewed: Some(Viewed(id, number, None, "")),
      preview: None,
    )
  let open_requests = case changed {
    True -> [event.emit("design", json.string(id)), load(model)]
    False -> []
  }
  #(
    model,
    effect.batch([
      get(version_path(id, number), fn(text) { VersionLoaded(id, number, text) }),
      ..open_requests
    ]),
  )
}

fn version_path(id: String, number: Int) -> String {
  "/designs/" <> id <> "/versions/" <> int.to_string(number)
}

fn load_version(model: Model, id: String, number: Int, text: String) -> Model {
  case viewed_for(model) {
    Some(viewed) if viewed.id == id && viewed.number == number ->
      case json.parse(text, decode.at(["design"], render.design_decoder())) {
        Ok(drawing) ->
          Model(..model, viewed: Some(Viewed(id, number, Some(drawing), "")))
        Error(_) ->
          Model(
            ..model,
            viewed: Some(Viewed(
              id,
              number,
              None,
              failure(text, "This version has no design to show."),
            )),
          )
      }
    _ -> model
  }
}

fn restore(model: Model, id: String, number: Int) -> #(Model, Effect(Msg)) {
  let model = Model(..model, restore_error: "")
  let model = case model.viewed {
    Some(viewed) if viewed.id == id && viewed.number == number ->
      Model(..model, viewed: Some(Viewed(..viewed, error: "")))
    _ -> model
  }
  let body =
    json.object([#("version", json.int(number))])
    |> json.to_string
  #(
    model,
    post("/designs/" <> id <> "/restore", body, fn(text) {
      Restored(id, number, text)
    }),
  )
}

fn restored(
  model: Model,
  id: String,
  number: Int,
  text: String,
) -> #(Model, Effect(Msg)) {
  case json.parse(text, versions_decoder()) {
    Ok(versions) -> {
      let model = case list.contains(model.expanded, id) {
        True -> set_history(model, History(id, versions, ""))
        False -> model
      }
      let model = case viewing(model, id, number) {
        True -> Model(..model, viewed: None)
        False -> model
      }
      let request = case id == model.open {
        True -> load(model)
        False -> effect.none()
      }
      #(model, request)
    }
    Error(_) -> {
      let message = failure(text, "The version could not be restored.")
      case model.viewed {
        Some(viewed) if viewed.id == id && viewed.number == number -> #(
          Model(..model, viewed: Some(Viewed(..viewed, error: message))),
          effect.none(),
        )
        _ -> #(
          case history_of(model, id) {
            Some(history) if history.versions != [] ->
              set_history(model, History(id, history.versions, message))
            _ -> Model(..model, restore_error: message)
          },
          effect.none(),
        )
      }
    }
  }
}

fn viewed_for(model: Model) -> Option(Viewed) {
  case model.viewed {
    Some(viewed) if viewed.id == model.open -> Some(viewed)
    _ -> None
  }
}

fn post(path: String, body: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request("", path, "POST", body, fn(text) { send(msg(text)) })
  })
}

fn version_canvas(model: Model, viewed: Viewed) -> List(Element(Msg)) {
  let title = case viewed.drawing {
    Some(drawing) -> drawing.title
    None ->
      case open_design(model) {
        Some(design) -> design.title
        None -> ""
      }
  }
  let tab = case open_design(model) {
    Some(design) -> Some(design.tab)
    None -> None
  }
  [
    html.div([attribute.class("design-heading")], [
      html.h2([], [html.text(title)]),
      html.span([attribute.class("design-version-note")], [
        html.text("Version " <> int.to_string(viewed.number)),
      ]),
      case restore_button(model, viewed) {
        Some(button) -> button
        None -> element.none()
      },
    ]),
    case viewed.drawing, tab {
      Some(drawing), Some(UiUx) ->
        html.div(
          [attribute.class("design-screens")],
          list.map(drawing.elements, render.draw),
        )
      Some(drawing), Some(Architecture) -> diagram.draw(drawing, dict.new())
      _, _ -> element.none()
    },
    case viewed.error {
      "" -> element.none()
      message -> html.p([attribute.role("alert")], [html.text(message)])
    },
  ]
}

fn restore_button(model: Model, viewed: Viewed) -> Option(Element(Msg)) {
  case viewed.drawing {
    None -> None
    Some(drawing) ->
      case model.drawing {
        Some(current) if current == drawing -> None
        _ ->
          Some(
            html.button(
              [
                attribute.class("design-restore"),
                event.on_click(Restore(viewed.id, viewed.number)),
              ],
              [
                html.text("Restore version " <> int.to_string(viewed.number)),
              ],
            ),
          )
      }
  }
}

fn failure(text: String, fallback: String) -> String {
  case reason(text) {
    Some(message) -> without_code(code(text), message)
    None -> fallback
  }
}

fn design_row(model: Model, design: Design) -> Element(Msg) {
  let expanded = list.contains(model.expanded, design.id)
  html.li([], [
    html.div([attribute.class("design-row")], [
      html.button(
        [
          attribute.class("design-item"),
          case model.open == design.id {
            True -> attribute.attribute("aria-current", "true")
            False -> attribute.none()
          },
          event.on_click(SelectDesign(design.id)),
        ],
        [html.text(design.title)],
      ),
      html.button(
        [
          attribute.class("design-arrow"),
          attribute.attribute("aria-expanded", case expanded {
            True -> "true"
            False -> "false"
          }),
          attribute.attribute("aria-label", "History of " <> design.title),
          event.on_click(ToggleHistory(design.id)),
        ],
        [
          html.text(case expanded {
            True -> "▾"
            False -> "▸"
          }),
        ],
      ),
    ]),
    case history_view(model, design.id, expanded) {
      Some(history) -> history_list(model, history)
      None -> element.none()
    },
  ])
}

fn current_canvas(model: Model, design: Design) -> List(Element(Msg)) {
  let proposal = proposal_for(model, design.id)
  let showing = case proposal {
    Some(_) -> model.view_proposal
    None -> False
  }
  let title = case proposal, showing {
    Some(proposed), True -> proposed.title
    _, _ -> design.title
  }
  let controls = case proposal {
    Some(_) -> proposal_controls(model, True)
    None -> []
  }
  [
    html.div([attribute.class("design-heading")], [
      html.h2([], [html.text(title)]),
      case design.tab, model.drawing {
        UiUx, Some(_) -> preview_button(model)
        _, _ -> element.none()
      },
      ..controls
    ]),
    case design.tab {
      Architecture ->
        case proposal, showing {
          Some(proposed), True -> proposed_diagram(model, proposed)
          _, _ -> current_diagram(model)
        }
      UiUx ->
        case proposal, showing {
          Some(proposed), True -> proposed_drawing(model, proposed)
          _, _ ->
            case model.drawing {
              Some(drawing) -> screens(model, drawing)
              None ->
                case model.design_error {
                  "" -> element.none()
                  message ->
                    html.p([attribute.role("alert")], [
                      html.text(message),
                    ])
                }
            }
        }
    },
  ]
}

@external(javascript, "./api.mjs", "stamp")
fn stamp(seconds: Int) -> String

fn newest(versions: List(Version)) -> Int {
  case versions {
    [latest, ..] -> latest.number
    [] -> 0
  }
}

fn clear_history_error(model: Model) -> Model {
  Model(
    ..model,
    histories: list.map(model.histories, fn(history) {
      case history.versions {
        [] -> history
        _ -> History(..history, error: "")
      }
    }),
  )
}

fn start_preview(model: Model) -> #(Model, Effect(Msg)) {
  case model.drawing {
    Some(drawing) ->
      case render.first_screen(drawing) {
        Some(screen) -> #(
          Model(..model, preview: Some(screen.id)),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    None -> #(model, effect.none())
  }
}

fn preview_click(model: Model, id: String) -> #(Model, Effect(Msg)) {
  case model.preview, model.drawing {
    Some(_), Some(drawing) ->
      case render.link_target(drawing, id) {
        Some(target) -> #(Model(..model, preview: Some(target)), effect.none())
        None -> #(model, effect.none())
      }
    _, _ -> #(model, effect.none())
  }
}

fn keep_preview(
  preview: Option(String),
  drawing: render.Design,
) -> Option(String) {
  case preview {
    Some(id) ->
      case render.screen(drawing, id) {
        Some(_) -> Some(id)
        None -> None
      }
    None -> None
  }
}

fn preview_button(model: Model) -> Element(Msg) {
  case model.preview {
    Some(_) ->
      html.button(
        [attribute.class("design-preview"), event.on_click(ExitPreview)],
        [html.text("Exit preview")],
      )
    None ->
      html.button([attribute.class("design-preview"), event.on_click(Preview)], [
        html.text("Preview"),
      ])
  }
}

fn screens(model: Model, drawing: render.Design) -> Element(Msg) {
  case model.preview {
    Some(id) ->
      case render.screen(drawing, id) {
        Some(screen) ->
          html.div([attribute.class("design-screens")], [
            render.draw_preview(screen, PreviewClick),
          ])
        None -> all_screens(drawing)
      }
    None -> all_screens(drawing)
  }
}

fn all_screens(drawing: render.Design) -> Element(Msg) {
  html.div(
    [attribute.class("design-screens")],
    list.map(drawing.elements, render.draw),
  )
}

fn proposal_for(model: Model, id: String) -> Option(render.Design) {
  case
    list.find(model.panel.proposals, fn(proposed) { proposed.design.id == id })
  {
    Ok(proposed) -> Some(proposed.design)
    Error(_) -> None
  }
}

fn create_proposal(model: Model, tab: Tab) -> Option(render.Design) {
  let name = case tab {
    UiUx -> "ui"
    Architecture -> "architecture"
  }
  case
    list.find(model.panel.proposals, fn(proposed) {
      proposed.tab == name
      && !list.any(model.designs, fn(design) { design.id == proposed.design.id })
    })
  {
    Ok(proposed) -> Some(proposed.design)
    Error(_) -> None
  }
}

fn proposed_drawing(model: Model, proposed: render.Design) -> Element(Msg) {
  case model.show_changes, model.drawing {
    True, Some(current) ->
      marked_screens(model, changes.mark(current, proposed))
    _, _ -> screens(model, proposed)
  }
}

fn marked_screens(model: Model, marked: changes.Marked) -> Element(Msg) {
  case model.preview {
    Some(id) ->
      case render.screen(marked.design, id) {
        Some(screen) ->
          html.div([attribute.class("design-screens")], [
            render.draw_preview(screen, PreviewClick),
          ])
        None -> marked_all_screens(marked)
      }
    None -> marked_all_screens(marked)
  }
}

fn marked_all_screens(marked: changes.Marked) -> Element(Msg) {
  html.div(
    [attribute.class("design-screens")],
    list.map(marked.design.elements, fn(node) {
      render.draw_marked(node, marked.marks)
    }),
  )
}

fn proposal_controls(model: Model, choices: Bool) -> List(Element(Msg)) {
  let switch = case choices {
    True -> [
      html.button(
        [
          attribute.class("design-choice"),
          attribute.attribute("aria-pressed", bool_text(model.view_proposal)),
          event.on_click(ShowProposed),
        ],
        [html.text("Proposed")],
      ),
      html.button(
        [
          attribute.class("design-choice"),
          attribute.attribute("aria-pressed", bool_text(!model.view_proposal)),
          event.on_click(ShowCurrent),
        ],
        [html.text("Current")],
      ),
    ]
    False -> []
  }
  let label = case model.show_changes {
    True -> "Hide changes"
    False -> "Show changes"
  }
  list.append(switch, [
    html.button(
      [
        attribute.class("design-choice"),
        attribute.attribute("aria-pressed", bool_text(model.show_changes)),
        attribute.attribute("aria-label", label),
        attribute.attribute("title", label),
        event.on_click(ToggleChanges),
      ],
      [html.text("🔍")],
    ),
  ])
}

fn bool_text(value: Bool) -> String {
  case value {
    True -> "true"
    False -> "false"
  }
}

fn create_canvas(model: Model, tab: Tab) -> List(Element(Msg)) {
  case create_proposal(model, tab) {
    None -> []
    Some(proposed) -> [
      html.div([attribute.class("design-heading")], [
        html.h2([], [html.text(proposed.title)]),
        ..proposal_controls(model, False)
      ]),
      case tab {
        Architecture ->
          case model.show_changes {
            True -> marked_diagram(changes.all_added(proposed))
            False -> diagram.draw(proposed, dict.new())
          }
        UiUx ->
          case model.show_changes {
            True -> marked_all_screens(changes.all_added(proposed))
            False -> all_screens(proposed)
          }
      },
    ]
  }
}

/// Whether answering a proposal took it off the open design's canvas, so the
/// saved design has to be fetched again rather than the old drawing kept.
fn proposal_answered(before: Model, after: Model) -> Bool {
  case before.open {
    "" -> False
    id ->
      case proposal_for(before, id), proposal_for(after, id) {
        Some(_), None -> True
        _, _ -> False
      }
  }
}

fn current_diagram(model: Model) -> Element(Msg) {
  case model.drawing {
    Some(drawing) -> diagram.draw(drawing, dict.new())
    None ->
      case model.design_error {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      }
  }
}

fn proposed_diagram(model: Model, proposed: render.Design) -> Element(Msg) {
  case model.show_changes, model.drawing {
    True, Some(current) -> marked_diagram(changes.mark(current, proposed))
    _, _ -> diagram.draw(proposed, dict.new())
  }
}

fn marked_diagram(marked: changes.Marked) -> Element(Msg) {
  diagram.draw(marked.design, marked.marks)
}
