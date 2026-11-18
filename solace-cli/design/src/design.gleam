//// Design page: architecture and UI/UX designs, their versions and proposals.

import changes
import diagram
import editing
import gleam/dict.{type Dict}
import gleam/dynamic.{type Dynamic}
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import lustre
import lustre/attribute
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import lustre/event
import moving
import panel
import plan_panel
import region
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
    /// Whether a live list has replaced the designs, so a later list reply
    /// is older and must not replace it.
    live_listed: Bool,
    /// The design id in the address, "" for none.
    open: String,
    drawing: Option(render.Design),
    /// The ids of the drawn elements the human selected, in selection order.
    selected: List(String),
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
    /// The version before the shown one, with the shown design's id and
    /// version, when it has been loaded.
    previous: Option(#(String, Int, render.Design)),
    /// The screen the click-through preview shows, if previewing.
    preview: Option(String),
    /// Whether the canvas shows an open proposal rather than the saved design.
    view_proposal: Bool,
    /// Whether the proposal's change outlines show.
    show_changes: Bool,
    /// The agent panel's state, shared by both tabs.
    panel: panel.Model,
    /// Waiting changes per design, keyed by design id.
    waiting: Dict(String, List(editing.Change)),
    /// Whether the panel shows the edit tab rather than the agent.
    edit_tab: Bool,
    /// Whether the human can change the open design's properties.
    edit_mode: Bool,
    /// Whether the prompt to leave edit mode with waiting changes shows.
    confirm_exit: Bool,
    /// Whether a save from the exit prompt ends edit mode when it succeeds.
    leaving: Bool,
    /// Why the last save was refused.
    edit_error: String,
    /// The title being edited, None when the title is not a text box.
    renaming: Option(String),
    /// Whether the delete confirmation is shown.
    confirm_delete: Bool,
    /// The design the Undo button brings back, while it shows.
    undo: Option(String),
    /// The element being dragged in edit mode.
    dragging: Option(Drag),
    /// The selected element's border being dragged in edit mode.
    resizing: Option(Resize),
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
  /// The reply to the version before the shown one, with the shown version.
  PreviousLoaded(String, Int, String)
  Restore(String, Int)
  Restored(String, Int, String)
  /// Starts the click-through preview at the first screen.
  Preview
  /// Leaves the preview and shows every screen again.
  ExitPreview
  /// A click in the preview, with the element's id.
  PreviewClick(String)
  /// A click on a drawn element outside Preview; multi adds or removes it.
  ElementClicked(String, Bool)
  /// A dragged-out region: the ids wholly inside, and whether a modifier
  /// asked to add them.
  RegionSelected(List(String), Bool)
  /// Clears the element selection.
  ClearSelection
  /// Escape clears the element selection.
  Escape
  /// Shows the open proposal instead of the saved design.
  ShowProposed
  /// Shows the saved design while a proposal is open.
  ShowCurrent
  /// Shows or hides the proposal's change outlines.
  ToggleChanges
  /// A message for the agent panel.
  Panel(panel.Msg)
  /// Shows the panel's edit tab.
  EditTab
  /// Shows the panel's agent tab.
  AgentTab
  /// The Edit button turns edit mode on.
  EditStart
  /// The Edit button turns edit mode off, asking first when changes wait.
  EditDone
  /// The exit prompt's Keep editing button.
  EditCancel
  /// The exit prompt's Save button.
  EditSave
  /// The exit prompt's Discard button.
  EditDiscard
  /// A property input, from a text box, a choice or a color picker.
  EditProperty(String, String, String)
  /// Saves the open design's waiting changes.
  SaveEdits
  /// Drops the open design's waiting changes.
  DiscardEdits
  /// The reply to a save, with the design it was asked for and the changes
  /// it sent.
  EditsSaved(String, String, List(editing.Change))
  /// The saved design's title is clicked, turning it into a text box.
  RenameStart
  /// The title text box's text changed.
  RenameText(String)
  /// The title text box's text settled; saves it.
  RenameSaved(String)
  /// A key pressed in the title text box.
  RenameKey(String)
  /// The title edit ended without a new title.
  RenameCancel
  /// The reply to a rename, with the design it was asked for.
  Renamed(String, String)
  /// Delete is clicked; the confirmation shows.
  DeleteAsk
  /// The delete confirmation is dismissed.
  DeleteCancel
  /// The delete confirmation is accepted.
  Delete
  /// The reply to a delete, with the design it removed.
  Deleted(String, String)
  /// Undo is clicked for the design it brings back.
  UndoDelete(String)
  /// The reply to an undo.
  Undone(String)
  /// The undo window closed for this design.
  UndoExpired(String)
  /// In edit mode, a drag on this element began.
  DragStarted(String)
  /// The pointer moved during an element drag, with each drawn element's
  /// id and bounds.
  Dragged(Float, Float, List(#(String, Float, Float, Float, Float)))
  /// The dragged element was let go.
  Dropped
  /// The element drag ended without a drop.
  DragCancelled
  /// A drag on a selected element's side or corner, with its id, side and
  /// drawn size.
  Resized(String, String, Float, Float)
  /// The resize drag was let go.
  ResizeDropped
  /// The resize drag ended without a drop.
  ResizeCancelled
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
        selected: [],
        design_error: "",
        restore_error: "",
        expanded: [],
        histories: [],
        viewed: None,
        previous: None,
        preview: None,
        edit_mode: False,
        confirm_exit: False,
        leaving: False,
      ),
      event.emit("tab", json.string(tab_name(tab))),
    )
    TabShown(tab) if tab == model.tab -> #(model, effect.none())
    TabShown(tab) -> {
      let model =
        Model(
          ..model,
          tab: tab,
          selected: [],
          restore_error: "",
          expanded: [],
          histories: [],
          viewed: None,
          previous: None,
          preview: None,
          edit_mode: False,
          confirm_exit: False,
          leaving: False,
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
          selected: [],
          design_error: "",
          restore_error: "",
          viewed: None,
          previous: None,
          preview: None,
          edit_mode: False,
          confirm_exit: False,
          leaving: False,
        )
      #(model, load(model))
    }
    SelectDesign(id) -> {
      let model =
        Model(
          ..clear_history_error(model),
          open: id,
          drawing: None,
          selected: [],
          design_error: "",
          restore_error: "",
          viewed: None,
          previous: None,
          preview: None,
          edit_mode: False,
          confirm_exit: False,
          leaving: False,
        )
      #(
        model,
        effect.batch([
          event.emit("design", json.string(id)),
          load(model),
        ]),
      )
    }
    DesignLoaded(id, text) -> {
      let loaded = load_pending(load_design(model, id, text), id)
      let model = case id == model.open {
        True ->
          Model(
            ..loaded,
            edit_mode: loaded.edit_mode || waiting_for(loaded, id) != [],
            confirm_exit: loaded.confirm_exit,
          )
        False -> loaded
      }
      let model = Model(..model, edit_error: "")
      #(model, current_previous_request(model))
    }
    DesignsLoaded(text) -> #(
      case model.live_listed {
        True -> model
        False -> loaded(model, text)
      },
      effect.none(),
    )
    ToggleHistory(id) -> toggle_history(model, id)
    HistoryLoaded(id, text) -> #(load_history(model, id, text), effect.none())
    ViewVersion(id, number) -> view_version(model, id, number)
    VersionLoaded(id, number, text) -> #(
      load_version(model, id, number, text),
      effect.none(),
    )
    PreviousLoaded(id, number, text) -> #(
      load_previous(model, id, number, text),
      effect.none(),
    )
    Restore(id, number) -> restore(model, id, number)
    Restored(id, number, text) -> restored(model, id, number, text)
    Preview -> start_preview(model)
    ExitPreview -> #(Model(..model, preview: None, selected: []), effect.none())
    PreviewClick(id) -> preview_click(model, id)
    ElementClicked(id, multi) -> {
      let next = select_element(model, id, multi)
      #(next, place_note_box(next.selected))
    }
    RegionSelected(ids, multi) -> {
      let next =
        Model(..model, selected: region_selection(model.selected, ids, multi))
      #(next, place_note_box(next.selected))
    }
    ClearSelection -> #(Model(..model, selected: []), effect.none())
    Escape -> #(Model(..model, selected: []), effect.none())
    ShowProposed -> #(
      Model(..model, view_proposal: True, selected: []),
      effect.none(),
    )
    ShowCurrent -> #(
      Model(..model, view_proposal: False, selected: []),
      effect.none(),
    )
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
          effect.from(fn(send) { listen_escape(fn(_) { send(Escape) }) }),
          effect.map(panel_fx, Panel),
          effect.before_paint(fn(send, root) {
            watch_region(root, send)
            watch_moves(root, send)
            watch_resizes(root, send)
            drag_note_box(root, fn() { Nil })
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
      effect.from(fn(_) { ignore_escape() }),
    )
    Listening(source) -> #(Model(..model, source: Some(source)), effect.none())
    Live(text) if !model.caught_up -> #(
      Model(..model, replay: [text, ..model.replay]),
      effect.none(),
    )
    Live(text) -> {
      case json.parse(text, decode.dynamic) {
        Ok(event) -> live(model, event)
        Error(_) -> #(model, effect.none())
      }
    }
    CaughtUp -> {
      let #(model, fx) = settle(model)
      let #(state, panel_fx) = panel.resume(model.panel)
      #(
        Model(..model, panel: state),
        effect.batch([
          fx,
          effect.from(fn(send) { listen_escape(fn(_) { send(Escape) }) }),
          effect.map(panel_fx, Panel),
          effect.map(panel.load_models(state), Panel),
        ]),
      )
    }
    RenameStart ->
      case open_design(model) {
        Some(design) -> #(
          Model(..model, renaming: Some(design.title)),
          effect.none(),
        )
        None -> #(model, effect.none())
      }
    RenameText(text) -> #(Model(..model, renaming: Some(text)), effect.none())
    RenameSaved(title) -> rename(model, title)
    RenameKey(key) if key == "Escape" -> #(
      Model(..model, renaming: None),
      effect.none(),
    )
    RenameKey(_) -> #(model, effect.none())
    RenameCancel -> #(Model(..model, renaming: None), effect.none())
    Renamed(id, text) -> renamed(model, id, text)
    DeleteAsk -> #(Model(..model, confirm_delete: True), effect.none())
    DeleteCancel -> #(Model(..model, confirm_delete: False), effect.none())
    Delete -> delete_design(model)
    Deleted(id, text) -> deleted(model, id, text)
    UndoDelete(id) -> undo_delete(model, id)
    Undone(text) -> undone(model, text)
    UndoExpired(id) ->
      case model.undo {
        Some(showing) if showing == id -> #(
          Model(..model, undo: None),
          effect.none(),
        )
        _ -> #(model, effect.none())
      }
    Panel(msg) -> {
      let #(state, fx) = panel.update(model.panel, msg)
      #(Model(..model, panel: state), effect.map(fx, Panel))
    }
    EditTab -> #(Model(..model, edit_tab: True), effect.none())
    AgentTab -> #(Model(..model, edit_tab: False), effect.none())
    EditStart -> #(Model(..model, edit_mode: True), effect.none())
    EditDone ->
      case model.edit_mode, waiting_for(model, model.open) {
        True, [] -> #(
          Model(..model, edit_mode: False, confirm_exit: False),
          effect.none(),
        )
        True, _ -> #(Model(..model, confirm_exit: True), effect.none())
        False, _ -> #(model, effect.none())
      }
    EditCancel -> #(Model(..model, confirm_exit: False), effect.none())
    EditSave ->
      case waiting_for(model, model.open) {
        [] -> #(
          Model(..model, edit_mode: False, confirm_exit: False, leaving: False),
          effect.none(),
        )
        _ -> {
          let #(model, fx) =
            save_edits(Model(..model, confirm_exit: False, leaving: True))
          #(model, fx)
        }
      }
    EditDiscard -> {
      let #(model, fx) = discard_edits(model)
      #(Model(..model, edit_mode: False, confirm_exit: False), fx)
    }
    EditProperty(field, property, value) ->
      edit_property(model, field, property, value)
    SaveEdits -> save_edits(model)
    DiscardEdits -> discard_edits(model)
    EditsSaved(id, text, sent) -> #(
      edits_saved(model, id, text, sent),
      effect.none(),
    )
    DragStarted(id) -> #(start_drag(model, id), effect.none())
    Dragged(x, y, positions) -> #(
      drag_to(model, x, y, positions),
      effect.none(),
    )
    Dropped -> #(drop(model), effect.none())
    DragCancelled -> #(Model(..model, dragging: None), effect.none())
    Resized(id, side, width, height) -> #(
      resize(model, id, side, width, height),
      effect.none(),
    )
    ResizeDropped -> #(resize_drop(model), effect.none())
    ResizeCancelled -> #(Model(..model, resizing: None), effect.none())
  }
}

pub fn view(model: Model) -> Element(Msg) {
  html.div([attribute.class("design")], [
    html.style([], view_styles),
    html.style([], edit_styles),
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
    panel.view_tabbed(
      model.panel,
      edit_panel(model),
      model.edit_tab,
      EditTab,
      AgentTab,
      Panel,
    ),
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
            case model.undo {
              Some(id) ->
                html.div(
                  [
                    attribute.class("design-undo-notice"),
                    attribute.role("status"),
                  ],
                  [
                    html.span([], [html.text("Design deleted.")]),
                    html.button(
                      [
                        attribute.class("design-undo"),
                        event.on_click(UndoDelete(id)),
                      ],
                      [html.text("Undo")],
                    ),
                  ],
                )
              None -> element.none()
            },
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
.design-canvas{position:relative;flex:1;min-width:0;min-height:0;overflow:auto;background:var(--paper);border:1px solid var(--line);padding:var(--pad-card)}
.design-canvas h2{margin:0 0 var(--space-3);font-size:var(--h2)}
.design-heading{display:flex;align-items:baseline;gap:var(--space-3);flex-wrap:wrap;margin:0 0 var(--space-3)}
.design-heading h2{margin:0}
.design-version-note{color:var(--soft)}
.design-restore{padding:var(--space-1) var(--space-2);font:inherit;color:var(--paper);background:var(--edge);border:1px solid var(--edge);cursor:pointer}
.design-preview{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}
.design-choice{padding:var(--space-1) var(--space-2);font:inherit;color:var(--ink);background:transparent;border:1px solid var(--line);cursor:pointer}
.design-choice[aria-pressed=true]{background:var(--edge);color:var(--paper);border-color:var(--edge)}
.design-changes{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}
.design-changes[aria-pressed=true]{background:var(--edge);color:var(--paper);border-color:var(--edge)}
.design-screens{display:flex;flex-direction:column;align-items:flex-start;gap:var(--space-4);height:100%}
.design-note{position:absolute;top:var(--space-3);right:var(--space-3);z-index:4;display:flex;flex-direction:column;gap:var(--space-2);width:min(280px,80vw);padding:var(--space-3);background:color-mix(in srgb,var(--paper) 60%,transparent);border:2px solid #c2410c;box-shadow:0 4px 16px rgba(0,0,0,.18)}
.design-note-handle{align-self:stretch;padding:var(--space-1) 0;font-size:13px;color:var(--soft);cursor:grab;user-select:none}
.design-note label{font-size:13px;color:var(--soft)}
.design-note input{padding:var(--space-1) var(--space-2);font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--line)}
.design-note button{align-self:flex-end;padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}
.design-notes{list-style:none;margin:0;padding:var(--space-2) 0;border-top:1px solid var(--line);max-height:28vh;overflow:auto}
.design-note-row{display:grid;grid-template-columns:auto minmax(0,1fr) auto auto;align-items:start;gap:var(--space-2);padding:var(--space-1) 0}
.design-note-number{display:inline-block;min-width:18px;height:18px;border-radius:9px;background:#c2410c;color:#fff;font-size:11px;line-height:18px;text-align:center}
.design-note-text{color:var(--ink);overflow-wrap:anywhere}
.design-note-elements{color:var(--soft);font-size:12px;overflow-wrap:anywhere}
.design-note-remove{padding:0 var(--space-1);font:inherit;color:var(--soft);background:transparent;border:0;cursor:pointer}
.design-title{padding:0;font:inherit;font-size:var(--h2);font-weight:700;color:inherit;background:transparent;border:1px solid transparent;cursor:text}
.design-title:hover{border-color:var(--line)}
.design-title-input{padding:0 var(--space-1);font:inherit;font-size:var(--h2);font-weight:700;color:var(--ink);background:var(--paper);border:1px solid var(--focus)}
.design-heading-actions{display:flex;align-items:baseline;gap:var(--space-2);margin-left:auto}
.design-delete{padding:var(--space-1) var(--space-2);font:inherit;color:#a12b1f;background:transparent;border:1px solid #a12b1f;cursor:pointer}
.design-danger{padding:var(--space-1) var(--space-2);font:inherit;color:var(--paper);background:#a12b1f;border:1px solid #a12b1f;cursor:pointer}
.design-undo{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}
.design-confirm{display:flex;align-items:baseline;gap:var(--space-2);margin-left:auto}
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
  Design(id: String, title: String, tab: Tab, version: Int)
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
    False,
    "",
    None,
    [],
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
    None,
    True,
    True,
    panel.new(),
    dict.new(),
    False,
    False,
    False,
    False,
    "",
    None,
    False,
    None,
    None,
    None,
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
  use version <- decode.optional_field(
    "version",
    0,
    decode.one_of(decode.int, [decode.success(0)]),
  )
  decode.success(Design(id, title, tab_of(tab), version))
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

fn apply(model: Model, event: Dynamic) -> #(Model, Bool) {
  let model = case decode.run(event, designs_decoder()) {
    Ok(designs) ->
      refresh_open(
        Model(
          ..model,
          designs: designs,
          live_listed: True,
          loading: False,
          error: "",
        ),
      )
    Error(_) -> model
  }
  let #(panel, refresh_models) = panel.live(model.panel, event)
  #(prune_selection(Model(..model, panel: panel)), refresh_models)
}

/// Applies the events received while catching up, now that the page has its
/// list. Runs already waiting before launch are not new, so it never flags
/// attention.
pub fn settle(model: Model) -> #(Model, Effect(Msg)) {
  let designs = model.designs
  let before = model
  let model = Model(..model, caught_up: True, loading: False)
  let model =
    list.fold(
      list.reverse(model.replay),
      Model(..model, replay: []),
      apply_text,
    )
  let answered = proposal_answered(before, model)
  let model = case answered {
    True -> Model(..model, drawing: None, selected: [])
    False -> model
  }
  case model.designs == designs && !answered {
    True -> #(model, effect.none())
    False -> #(model, reload(before, model))
  }
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
      live_listed: False,
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
            solace_task.events(
              "",
              fn(text) { send(Live(text)) },
              fn() { send(CaughtUp) },
              ["designs", "designRun", "designRuns", "roleModels"],
            ),
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
        selected: [],
        design_error: "",
        viewed: None,
        previous: None,
        preview: None,
        edit_mode: False,
        confirm_exit: False,
        leaving: False,
      )
  }
}

fn load_design(model: Model, id: String, text: String) -> Model {
  case id == model.open {
    True ->
      case render.decode(text) {
        Ok(drawing) ->
          prune_selection(
            Model(
              ..model,
              drawing: Some(drawing),
              previous: case model.viewed {
                None -> None
                Some(_) -> model.previous
              },
              design_error: "",
              preview: keep_preview(model.preview, drawing),
            ),
          )
        Error(_) ->
          Model(
            ..model,
            drawing: None,
            selected: [],
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

fn reload(before: Model, model: Model) -> Effect(Msg) {
  let open = case open_design(model) {
    Some(_) -> [load(model)]
    None -> []
  }
  let histories =
    list.map(history_ids(before, model), fn(id) {
      get("/designs/" <> id <> "/versions", fn(text) { HistoryLoaded(id, text) })
    })
  case list.flatten([open, histories]) {
    [] -> effect.none()
    [one] -> one
    many -> effect.batch(many)
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
      previous: None,
      preview: None,
      selected: [],
      edit_mode: False,
      confirm_exit: False,
      leaving: False,
    )
  let open_requests = case changed {
    True -> [event.emit("design", json.string(id)), load(model)]
    False -> []
  }
  #(
    model,
    effect.batch([
      get(version_path(id, number), fn(text) { VersionLoaded(id, number, text) }),
      previous_request(model, id, number),
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
          Model(
            ..model,
            viewed: Some(Viewed(id, number, Some(drawing), "")),
            selected: keep_selected(model.selected, drawing),
          )
        Error(_) ->
          Model(
            ..model,
            viewed: Some(Viewed(
              id,
              number,
              None,
              failure(text, "This version has no design to show."),
            )),
            selected: [],
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
        True -> Model(..model, viewed: None, previous: None, selected: [])
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
  let controls = case previous_version(model, viewed.id, viewed.number) {
    Some(_) -> [changes_control(model)]
    None -> []
  }
  [
    html.div([attribute.class("design-heading")], [
      html.h2([], [html.text(title)]),
      html.span([attribute.class("design-version-note")], [
        html.text("Version " <> int.to_string(viewed.number)),
      ]),
      case tab, viewed.drawing {
        Some(UiUx), Some(_) -> preview_button(model)
        _, _ -> element.none()
      },
      case restore_button(model, viewed) {
        Some(button) -> button
        None -> element.none()
      },
      ..controls
    ]),
    case viewed.drawing, tab {
      Some(drawing), Some(UiUx) -> version_screens(model, viewed, drawing)
      Some(drawing), Some(Architecture) ->
        case version_marks(model, viewed, drawing) {
          Some(marked) -> marked_diagram(model, marked)
          None -> diagram.draw(drawing, dict.new(), diagram_selection(model))
        }
      _, _ -> element.none()
    },
    case viewed.error {
      "" -> element.none()
      message -> html.p([attribute.role("alert")], [html.text(message)])
    },
    note_box(model),
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
    None ->
      case previous_version(model, design.id, design.version) {
        Some(_) -> [changes_control(model)]
        None -> []
      }
  }
  let editable = !showing
  [
    html.div([attribute.class("design-heading")], [
      html.h2([], [title_control(model, title, editable)]),
      case design.tab, model.drawing {
        UiUx, Some(_) -> preview_button(model)
        _, _ -> element.none()
      },
      ..list.append(controls, [heading_actions(model, design)])
    ]),
    case model.edit_error {
      "" -> element.none()
      message ->
        html.p([attribute.class("design-edit-error"), attribute.role("alert")], [
          html.text(message),
        ])
    },
    case model.drawing, model.design_error {
      Some(_), message if message != "" ->
        html.p([attribute.role("alert")], [html.text(message)])
      _, _ -> element.none()
    },
    case design.tab {
      Architecture ->
        case proposal, showing {
          Some(proposed), True -> proposed_diagram(model, proposed)
          _, _ -> current_diagram(model, design)
        }
      UiUx ->
        case proposal, showing {
          Some(proposed), True -> proposed_drawing(model, proposed)
          _, _ -> current_screens(model, design)
        }
    },
    note_box(model),
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
  let model = Model(..model, selected: [])
  case preview_drawing(model) {
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
  case model.preview, preview_drawing(model) {
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

fn screens(
  model: Model,
  drawing: render.Design,
  resizable: Bool,
) -> Element(Msg) {
  case model.preview {
    Some(id) ->
      case render.screen(drawing, id) {
        Some(screen) ->
          html.div([attribute.class("design-screens")], [
            render.draw_preview(screen, PreviewClick),
          ])
        None -> all_screens(model, drawing, resizable)
      }
    None -> all_screens(model, drawing, resizable)
  }
}

fn all_screens(
  model: Model,
  drawing: render.Design,
  resizable: Bool,
) -> Element(Msg) {
  let markers = panel.markers(model.panel, model.open)
  html.div(
    [
      attribute.class("design-screens"),
      canvas_gesture(model),
      event.on_click(ClearSelection),
    ],
    list.map(drawing.elements, fn(node) {
      case resizable {
        True ->
          render.draw_selectable_resizable(
            node,
            model.selected,
            markers,
            ElementClicked,
          )
        False ->
          render.draw_selectable(node, model.selected, markers, ElementClicked)
      }
    }),
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
      marked_screens(model, changes.mark(current, proposed), False)
    _, _ -> screens(model, proposed, False)
  }
}

fn marked_screens(
  model: Model,
  marked: changes.Marked,
  resizable: Bool,
) -> Element(Msg) {
  case model.preview {
    Some(id) ->
      case render.screen(marked.design, id) {
        Some(screen) ->
          html.div([attribute.class("design-screens")], [
            render.draw_preview(screen, PreviewClick),
          ])
        None -> marked_all_screens(model, marked, resizable)
      }
    None -> marked_all_screens(model, marked, resizable)
  }
}

fn marked_all_screens(
  model: Model,
  marked: changes.Marked,
  resizable: Bool,
) -> Element(Msg) {
  let markers = panel.markers(model.panel, model.open)
  html.div(
    [
      attribute.class("design-screens"),
      canvas_gesture(model),
      event.on_click(ClearSelection),
    ],
    list.map(marked.design.elements, fn(node) {
      case resizable {
        True ->
          render.draw_marked_selectable_resizable(
            node,
            model.selected,
            marked.marks,
            markers,
            ElementClicked,
          )
        False ->
          render.draw_marked_selectable(
            node,
            model.selected,
            marked.marks,
            markers,
            ElementClicked,
          )
      }
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
  list.append(switch, [changes_control(model)])
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
            True -> marked_diagram(model, changes.all_added(proposed))
            False ->
              diagram.draw(proposed, dict.new(), diagram_selection(model))
          }
        UiUx ->
          case model.show_changes {
            True ->
              marked_all_screens(model, changes.all_added(proposed), False)
            False -> all_screens(model, proposed, False)
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

fn current_diagram(model: Model, design: Design) -> Element(Msg) {
  case model.drawing {
    Some(saved) -> {
      let drawing = editing.apply(waiting_for(model, design.id), saved)
      case current_marks(model, design, drawing) {
        Some(marked) -> marked_diagram(model, marked)
        None -> diagram.draw(drawing, dict.new(), diagram_selection(model))
      }
    }
    None ->
      case model.design_error {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      }
  }
}

fn proposed_diagram(model: Model, proposed: render.Design) -> Element(Msg) {
  case model.show_changes, model.drawing {
    True, Some(current) ->
      marked_diagram(model, changes.mark(current, proposed))
    _, _ -> diagram.draw(proposed, dict.new(), diagram_selection(model))
  }
}

fn marked_diagram(model: Model, marked: changes.Marked) -> Element(Msg) {
  diagram.draw(marked.design, marked.marks, diagram_selection(model))
}

@external(javascript, "./api.mjs", "listen_escape")
fn listen_escape(dispatch: fn(Nil) -> Nil) -> Nil

@external(javascript, "./api.mjs", "ignore_escape")
fn ignore_escape() -> Nil

fn select_element(model: Model, id: String, multi: Bool) -> Model {
  case multi {
    False -> Model(..model, selected: [id])
    True ->
      case list.contains(model.selected, id) {
        True ->
          Model(
            ..model,
            selected: list.filter(model.selected, fn(other) { other != id }),
          )
        False -> Model(..model, selected: list.append(model.selected, [id]))
      }
  }
}

fn keep_selected(
  selected: List(String),
  drawing: render.Design,
) -> List(String) {
  list.filter(selected, fn(id) { render.has_element(drawing, id) })
}

fn prune_selection(model: Model) -> Model {
  let selected = case shown_drawing(model) {
    Some(drawing) -> keep_selected(model.selected, drawing)
    None -> []
  }
  Model(..model, selected: selected)
}

fn shown_drawing(model: Model) -> Option(render.Design) {
  case viewed_for(model) {
    Some(viewed) -> viewed.drawing
    None ->
      case proposal_for(model, model.open) {
        Some(proposed) if model.view_proposal -> Some(proposed)
        _ -> model.drawing
      }
  }
}

fn live(model: Model, event: Dynamic) -> #(Model, Effect(Msg)) {
  let before = model.panel
  let #(updated, refresh_models) = apply(model, event)
  let answered = proposal_answered(model, updated)
  let updated = case answered {
    True -> Model(..updated, drawing: None, selected: [])
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
        False -> [reload(model, updated)]
      },
      case follows {
        True -> [effect.map(panel_fx, Panel)]
        False -> []
      },
      case attention {
        True -> [event.emit("attention", json.null())]
        False -> []
      },
      case refresh_models {
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

fn apply_text(model: Model, text: String) -> Model {
  case json.parse(text, decode.dynamic) {
    Ok(event) -> {
      let #(model, _) = apply(model, event)
      model
    }
    Error(_) -> model
  }
}

fn region_selection(
  selected: List(String),
  ids: List(String),
  multi: Bool,
) -> List(String) {
  case ids, multi {
    [], _ -> []
    _, False -> ids
    _, True ->
      list.fold(ids, selected, fn(current, id) {
        case list.contains(current, id) {
          True -> current
          False -> list.append(current, [id])
        }
      })
  }
}

@external(javascript, "./api.mjs", "listen_region")
fn listen_region(
  root: Dynamic,
  report: fn(
    Float,
    Float,
    Float,
    Float,
    List(#(String, Float, Float, Float, Float)),
    Bool,
  ) -> Nil,
) -> Nil

fn watch_region(root: Dynamic, send: fn(Msg) -> Nil) -> Nil {
  listen_region(root, fn(start_x, start_y, end_x, end_y, positions, multi) {
    let ids =
      region.ids_inside(
        region.between(start_x, start_y, end_x, end_y),
        element_rects(positions),
      )
    send(RegionSelected(ids, multi))
  })
}

@external(javascript, "./api.mjs", "place_note_box")
fn place_note_box_at(root: Dynamic, id: String) -> Nil

fn place_note_box(selected: List(String)) -> Effect(Msg) {
  case selected {
    [first, ..] ->
      effect.after_paint(fn(_, root) { place_note_box_at(root, first) })
    [] -> effect.none()
  }
}

fn note_box(model: Model) -> Element(Msg) {
  case model.preview, model.selected, model.open, model.edit_mode {
    None, [_, ..], open, False if open != "" ->
      html.div(
        [attribute.class("design-note"), attribute.data("note-box", "")],
        [
          html.div(
            [
              attribute.class("design-note-handle"),
              attribute.data("note-handle", ""),
            ],
            [html.text("Move")],
          ),
          html.label([attribute.attribute("for", "design-note-text")], [
            html.text("Note"),
          ]),
          html.input([
            attribute.id("design-note-text"),
            attribute.type_("text"),
            attribute.value(model.panel.note_text),
            attribute.attribute("placeholder", "What should the agent know?"),
            event.on_input(fn(text) { Panel(panel.NoteText(text)) }),
          ]),
          html.button(
            [
              attribute.type_("button"),
              attribute.disabled(string.trim(model.panel.note_text) == ""),
              event.on_click(Panel(panel.AddNote(model.open, model.selected))),
            ],
            [html.text("Add note")],
          ),
        ],
      )
    _, _, _, _ -> element.none()
  }
}

fn diagram_selection(model: Model) -> diagram.Selection(Msg) {
  diagram.Selection(
    selected: model.selected,
    markers: panel.markers(model.panel, model.open),
    on_click: ElementClicked,
    on_clear: ClearSelection,
  )
}

/// The expanded entries whose history has to be fetched again: the open one,
/// and any whose newest version changed.
fn history_ids(before: Model, model: Model) -> List(String) {
  let open = case open_design(model) {
    Some(design) ->
      case list.contains(model.expanded, design.id) {
        True -> [design.id]
        False -> []
      }
    None -> []
  }
  list.fold(model.expanded, open, fn(ids, id) {
    case list.contains(ids, id) || !version_changed(before, model, id) {
      True -> ids
      False -> list.append(ids, [id])
    }
  })
}

fn version_changed(before: Model, after: Model, id: String) -> Bool {
  case design_version(before, id), design_version(after, id) {
    Ok(old), Ok(new) -> old != new
    _, _ -> False
  }
}

fn design_version(model: Model, id: String) -> Result(Int, Nil) {
  case list.find(model.designs, fn(design) { design.id == id }) {
    Ok(design) -> Ok(design.version)
    Error(_) -> Error(Nil)
  }
}

fn title_control(model: Model, title: String, editable: Bool) -> Element(Msg) {
  case model.renaming, editable {
    Some(draft), True ->
      html.input([
        attribute.class("design-title-input"),
        attribute.value(draft),
        attribute.autofocus(True),
        attribute.attribute("aria-label", "Design title"),
        event.on_input(RenameText),
        event.on_change(RenameSaved),
        event.on_keydown(RenameKey),
        event.on_blur(RenameCancel),
      ])
    _, True ->
      html.button(
        [attribute.class("design-title"), event.on_click(RenameStart)],
        [html.text(title)],
      )
    _, False -> html.text(title)
  }
}

fn heading_actions(model: Model, design: Design) -> Element(Msg) {
  case model.confirm_delete, model.confirm_exit {
    True, _ ->
      html.div(
        [
          attribute.class("design-confirm"),
          attribute.role("alertdialog"),
        ],
        [
          html.span([], [
            html.text("Delete this design and all its versions?"),
          ]),
          html.button(
            [attribute.class("design-danger"), event.on_click(Delete)],
            [html.text("Delete")],
          ),
          html.button(
            [attribute.class("design-choice"), event.on_click(DeleteCancel)],
            [html.text("Cancel")],
          ),
        ],
      )
    False, True ->
      html.div(
        [
          attribute.class("design-confirm"),
          attribute.role("alertdialog"),
        ],
        [
          html.span([], [
            html.text("Save your changes before leaving edit mode?"),
          ]),
          html.button(
            [attribute.class("design-choice"), event.on_click(EditSave)],
            [html.text("Save")],
          ),
          html.button(
            [attribute.class("design-choice"), event.on_click(EditDiscard)],
            [html.text("Discard")],
          ),
          html.button(
            [attribute.class("design-choice"), event.on_click(EditCancel)],
            [html.text("Keep editing")],
          ),
        ],
      )
    False, False ->
      html.div(
        [attribute.class("design-heading-actions")],
        list.flatten([
          case editable_design(model) {
            Some(_) -> [
              html.button(
                [
                  attribute.class("design-choice"),
                  attribute.attribute("aria-pressed", case model.edit_mode {
                    True -> "true"
                    False -> "false"
                  }),
                  event.on_click(case model.edit_mode {
                    True -> EditDone
                    False -> EditStart
                  }),
                ],
                [
                  html.text(case model.edit_mode {
                    True -> "Done editing"
                    False -> "Edit"
                  }),
                ],
              ),
            ]
            None -> []
          },
          [
            html.button(
              [
                attribute.class("design-delete"),
                attribute.attribute("aria-label", "Delete " <> design.title),
                event.on_click(DeleteAsk),
              ],
              [html.text("Delete")],
            ),
          ],
        ]),
      )
  }
}

fn rename(model: Model, title: String) -> #(Model, Effect(Msg)) {
  case model.open, string.trim(title) {
    id, title if id != "" && title != "" -> {
      let body =
        json.object([#("title", json.string(title))])
        |> json.to_string
      #(
        Model(..model, renaming: None),
        post("/designs/" <> id <> "/rename", body, fn(text) {
          Renamed(id, text)
        }),
      )
    }
    _, _ -> #(Model(..model, renaming: None), effect.none())
  }
}

fn renamed(model: Model, id: String, text: String) -> #(Model, Effect(Msg)) {
  case render.decode(text) {
    Ok(_) -> #(
      load_design(Model(..model, renaming: None), id, text),
      get("/designs", DesignsLoaded),
    )
    Error(_) -> #(
      Model(
        ..model,
        renaming: None,
        design_error: failure(text, "The title was refused."),
      ),
      effect.none(),
    )
  }
}

fn delete_design(model: Model) -> #(Model, Effect(Msg)) {
  case model.open {
    "" -> #(Model(..model, confirm_delete: False), effect.none())
    id -> #(
      Model(..model, confirm_delete: False),
      effect.from(fn(send) {
        request("", "/designs/" <> id, "DELETE", "{}", fn(text) {
          send(Deleted(id, text))
        })
      }),
    )
  }
}

fn deleted(model: Model, id: String, text: String) -> #(Model, Effect(Msg)) {
  case reason(text) {
    Some(_) -> #(
      Model(
        ..model,
        confirm_delete: False,
        error: failure(text, "The design could not be deleted."),
      ),
      effect.none(),
    )
    None -> {
      let was_open = model.open == id
      let model = forget_deleted_notes(loaded_keeping_live(model, text), id)
      let model = case was_open {
        True ->
          Model(
            ..model,
            open: "",
            drawing: None,
            selected: [],
            viewed: None,
            preview: None,
            design_error: "",
            renaming: None,
          )
        False -> model
      }
      let model = Model(..model, confirm_delete: False, undo: Some(id))
      let timer =
        effect.from(fn(send) { after(undo_ms, fn() { send(UndoExpired(id)) }) })
      let changed = case was_open {
        True -> effect.batch([event.emit("design", json.string("")), timer])
        False -> timer
      }
      #(model, changed)
    }
  }
}

fn undo_delete(model: Model, id: String) -> #(Model, Effect(Msg)) {
  #(model, post("/designs/" <> id <> "/undo", "{}", fn(text) { Undone(text) }))
}

fn undone(model: Model, text: String) -> #(Model, Effect(Msg)) {
  #(Model(..loaded_keeping_live(model, text), undo: None), effect.none())
}

fn forget_deleted_notes(model: Model, id: String) -> Model {
  Model(..model, panel: panel.forget_design(model.panel, id))
}

@external(javascript, "./api.mjs", "after")
fn after(ms: Int, callback: fn() -> Nil) -> Nil

/// How long the Undo button stays after a delete.
const undo_ms = 8000

@external(javascript, "./api.mjs", "drag_note_box")
fn drag_note_box(root: Dynamic, moved: fn() -> Nil) -> Nil

fn load_previous(model: Model, id: String, number: Int, text: String) -> Model {
  case showing_version(model, id, number) {
    False -> model
    True ->
      case json.parse(text, decode.at(["design"], render.design_decoder())) {
        Ok(drawing) -> Model(..model, previous: Some(#(id, number, drawing)))
        Error(_) -> Model(..model, previous: None)
      }
  }
}

fn showing_version(model: Model, id: String, number: Int) -> Bool {
  case model.viewed {
    Some(viewed) -> viewed.id == id && viewed.number == number
    None ->
      case open_design(model) {
        Some(design) -> design.id == id && design.version == number
        None -> False
      }
  }
}

fn current_previous_request(model: Model) -> Effect(Msg) {
  case model.viewed, open_design(model) {
    None, Some(design) -> previous_request(model, design.id, design.version)
    _, _ -> effect.none()
  }
}

fn previous_request(model: Model, id: String, number: Int) -> Effect(Msg) {
  case previous_version(model, id, number) {
    Some(previous) ->
      get(version_path(id, previous), fn(text) {
        PreviousLoaded(id, number, text)
      })
    None -> effect.none()
  }
}

/// The next-older version in the history, or the number before when the
/// history is not loaded.
fn previous_version(model: Model, id: String, number: Int) -> Option(Int) {
  case history_of(model, id) {
    Some(history) -> older_version(history.versions, number)
    None ->
      case number > 1 {
        True -> Some(number - 1)
        False -> None
      }
  }
}

fn older_version(versions: List(Version), number: Int) -> Option(Int) {
  list.fold(versions, None, fn(older, version) {
    case version.number < number {
      True ->
        case older {
          Some(found) if found > version.number -> older
          _ -> Some(version.number)
        }
      False -> older
    }
  })
}

fn version_screens(
  model: Model,
  viewed: Viewed,
  drawing: render.Design,
) -> Element(Msg) {
  case version_marks(model, viewed, drawing) {
    Some(marked) -> marked_screens(model, marked, False)
    None -> screens(model, drawing, False)
  }
}

fn version_marks(
  model: Model,
  viewed: Viewed,
  drawing: render.Design,
) -> Option(changes.Marked) {
  case model.show_changes, model.previous {
    True, Some(#(previous_id, previous_number, previous))
      if previous_id == viewed.id && previous_number == viewed.number
    ->
      case previous_version(model, viewed.id, viewed.number) {
        Some(_) -> Some(changes.mark(previous, drawing))
        None -> None
      }
    _, _ -> None
  }
}

fn preview_drawing(model: Model) -> Option(render.Design) {
  case viewed_for(model) {
    Some(viewed) -> viewed.drawing
    None -> model.drawing
  }
}

fn changes_control(model: Model) -> Element(Msg) {
  let label = case model.show_changes {
    True -> "Hide changes"
    False -> "Show changes"
  }
  html.button(
    [
      attribute.class("design-changes"),
      attribute.attribute("aria-pressed", bool_text(model.show_changes)),
      attribute.attribute("aria-label", label),
      attribute.attribute("title", label),
      event.on_click(ToggleChanges),
    ],
    [html.text("🔍")],
  )
}

fn current_screens(model: Model, design: Design) -> Element(Msg) {
  case model.drawing {
    Some(saved) -> {
      let drawing =
        dragged(model, editing.apply(waiting_for(model, design.id), saved))
      let drawing = resize_preview(model, drawing)
      let resizable = model.edit_mode && one_selected(model)
      case current_marks(model, design, drawing) {
        Some(marked) -> marked_screens(model, marked, resizable)
        None -> screens(model, drawing, resizable)
      }
    }
    None ->
      case model.design_error {
        "" -> element.none()
        message -> html.p([attribute.role("alert")], [html.text(message)])
      }
  }
}

fn current_marks(
  model: Model,
  design: Design,
  drawing: render.Design,
) -> Option(changes.Marked) {
  case model.show_changes, model.previous {
    True, Some(#(previous_id, previous_number, previous))
      if previous_id == design.id && previous_number == design.version
    ->
      case previous_version(model, design.id, design.version) {
        Some(_) -> Some(changes.mark(previous, drawing))
        None -> None
      }
    _, _ -> None
  }
}

const edit_styles =
  ".design-edit{display:flex;flex-direction:column;gap:var(--space-2);padding:var(--space-1) 0}
.design-edit h3{margin:0}
.design-edit-field{display:flex;flex-direction:column;gap:var(--space-1)}
.design-edit-field label{font-size:13px;color:var(--soft)}
.design-edit-field input,.design-edit-field select{box-sizing:border-box;width:100%;padding:var(--space-1) var(--space-2);font:inherit;color:var(--ink);background:var(--paper);border:1px solid var(--line)}
.design-edit-color{display:flex;gap:var(--space-2);align-items:center}
.design-edit-color input[type=color]{flex:none;width:44px;padding:0}
.design-edit-color input[type=text]{flex:1;min-width:0}
.design-edit-actions{display:flex;gap:var(--space-2);margin-top:var(--space-2)}
.design-edit-actions button{padding:var(--space-1) var(--space-2);font:inherit;cursor:pointer}
.design-edit-actions button:disabled{color:var(--soft);cursor:default}
.design-edit-error{color:#a12b1f}
.design-edit-hint{color:var(--soft)}
"

fn waiting_for(model: Model, id: String) -> List(editing.Change) {
  case dict.get(model.waiting, id) {
    Ok(changes) -> changes
    Error(_) -> []
  }
}

fn load_pending(model: Model, id: String) -> Model {
  case dict.get(model.waiting, id) {
    Ok(_) -> model
    Error(_) ->
      Model(..model, waiting: dict.insert(model.waiting, id, editing.load(id)))
  }
}

fn editable_design(model: Model) -> Option(render.Design) {
  case
    model.tab,
    model.open,
    model.drawing,
    model.viewed,
    model.preview,
    model.view_proposal
  {
    UiUx, id, Some(drawing), None, None, showing_proposal if id != "" ->
      case showing_proposal, proposal_for(model, id) {
        True, Some(_) -> None
        _, _ -> Some(drawing)
      }
    Architecture, id, Some(drawing), None, None, showing_proposal if id != "" ->
      case showing_proposal, proposal_for(model, id) {
        True, Some(_) -> None
        _, _ -> Some(drawing)
      }
    _, _, _, _, _, _ -> None
  }
}

fn editable_node(model: Model) -> Option(render.Node) {
  case drawn_for_editing(model), model.selected {
    Some(drawing), [id] -> editing.node(drawing, id)
    _, _ -> None
  }
}

fn edit_property(
  model: Model,
  field: String,
  property: String,
  value: String,
) -> #(Model, Effect(Msg)) {
  case model.edit_mode, editable_node(model) {
    False, _ -> #(model, effect.none())
    True, Some(node) ->
      case change_for(node.id, field, property, value) {
        Some(change) -> {
          let id = model.open
          let changes = editing.with_change(waiting_for(model, id), change)
          editing.store(id, changes)
          #(
            Model(
              ..model,
              waiting: dict.insert(model.waiting, id, changes),
              edit_error: "",
            ),
            effect.none(),
          )
        }
        None -> #(model, effect.none())
      }
    True, None -> #(model, effect.none())
  }
}

fn change_for(
  id: String,
  field: String,
  property: String,
  value: String,
) -> Option(editing.Change) {
  case field {
    "style" ->
      case value {
        "" -> Some(editing.clear_change(id, field, property))
        _ ->
          case property {
            "size" | "padding" ->
              case parsed_number(value) {
                Some(number) ->
                  Some(editing.number_change(id, field, property, number))
                None -> Some(editing.text_change(id, field, property, value))
              }
            _ -> Some(editing.text_change(id, field, property, value))
          }
      }
    _ ->
      case property {
        "width" | "height" ->
          case value {
            "" -> None
            "fill" | "hug" ->
              Some(editing.text_change(id, field, property, value))
            _ ->
              case parsed_number(value) {
                Some(number) ->
                  Some(editing.fixed_change(id, field, property, number))
                None -> None
              }
          }
        "gap" | "padding" ->
          case value {
            "" -> None
            _ ->
              case parsed_number(value) {
                Some(number) ->
                  Some(editing.number_change(id, field, property, number))
                None -> Some(editing.text_change(id, field, property, value))
              }
          }
        "level" ->
          case int.parse(value) {
            Ok(number) ->
              Some(editing.integer_change(id, field, property, number))
            Error(_) -> None
          }
        _ -> Some(editing.text_change(id, field, property, value))
      }
  }
}

fn save_edits(model: Model) -> #(Model, Effect(Msg)) {
  case model.open, waiting_for(model, model.open) {
    "", _ -> #(model, effect.none())
    _, [] -> #(Model(..model, edit_error: ""), effect.none())
    id, changes -> #(
      Model(..model, edit_error: ""),
      post(
        "/designs/" <> id <> "/edit",
        json.to_string(editing.operations(changes)),
        fn(text) { EditsSaved(id, text, changes) },
      ),
    )
  }
}

fn discard_edits(model: Model) -> #(Model, Effect(Msg)) {
  case model.open {
    "" -> #(model, effect.none())
    id -> {
      editing.store(id, [])
      #(
        Model(
          ..model,
          waiting: dict.insert(model.waiting, id, []),
          edit_error: "",
          leaving: False,
        ),
        effect.none(),
      )
    }
  }
}

fn edits_saved(
  model: Model,
  id: String,
  text: String,
  sent: List(editing.Change),
) -> Model {
  case render.decode(text) {
    Ok(_) -> {
      let waiting =
        list.filter(waiting_for(model, id), fn(change) {
          !list.contains(sent, change)
        })
      editing.store(id, waiting)
      let model =
        Model(..model, waiting: dict.insert(model.waiting, id, waiting))
      case id == model.open {
        True ->
          load_design(
            Model(
              ..model,
              edit_error: "",
              edit_mode: case model.leaving && waiting == [] {
                True -> False
                False -> model.edit_mode
              },
              confirm_exit: False,
              leaving: False,
            ),
            id,
            text,
          )
        False -> model
      }
    }
    Error(_) ->
      case id == model.open {
        True ->
          Model(
            ..model,
            edit_error: failure(text, "The change was refused."),
            leaving: False,
          )
        False -> model
      }
  }
}

fn edit_panel(model: Model) -> Element(Msg) {
  let changes = waiting_for(model, model.open)
  let dirty = changes != []
  let fields = case editable_node(model) {
    Some(node) -> [
      html.h3([], [html.text(editing.type_name(node.kind))]),
      ..list.map(editing.properties(node), fn(field) {
        edit_field(field, model.edit_mode)
      })
    ]
    None -> [
      html.p([attribute.class("design-edit-hint")], [
        html.text("Select one element in the saved design to edit it."),
      ]),
    ]
  }
  let error = case model.edit_error {
    "" -> []
    message -> [
      html.p([attribute.class("design-edit-error"), attribute.role("alert")], [
        html.text(message),
      ]),
    ]
  }
  let actions = case dirty {
    False -> []
    True -> [
      html.div([attribute.class("design-edit-actions")], [
        html.button([attribute.type_("button"), event.on_click(SaveEdits)], [
          html.text("Save"),
        ]),
        html.button([attribute.type_("button"), event.on_click(DiscardEdits)], [
          html.text("Discard"),
        ]),
      ]),
    ]
  }
  html.div(
    [attribute.class("design-edit")],
    list.flatten([fields, error, actions]),
  )
}

fn edit_field(field: editing.Property, enabled: Bool) -> Element(Msg) {
  let id = "edit-" <> field.field <> "-" <> field.property
  let locked = attribute.disabled(!enabled)
  let control = case field.control {
    editing.TextControl ->
      html.input([
        attribute.id(id),
        attribute.type_("text"),
        attribute.value(field.value),
        locked,
        event.on_input(fn(value) {
          EditProperty(field.field, field.property, value)
        }),
      ])
    editing.NumberControl ->
      html.input([
        attribute.id(id),
        attribute.type_("number"),
        attribute.value(field.value),
        locked,
        event.on_input(fn(value) {
          EditProperty(field.field, field.property, value)
        }),
      ])
    editing.Choice(options) ->
      html.select(
        [
          attribute.id(id),
          locked,
          event.on_input(fn(value) {
            EditProperty(field.field, field.property, value)
          }),
        ],
        list.map(options, fn(option) {
          html.option(
            [
              attribute.value(option.0),
              attribute.selected(option.0 == field.value),
            ],
            option.1,
          )
        }),
      )
    editing.ColorControl ->
      html.div([attribute.class("design-edit-color")], [
        html.input([
          attribute.id(id <> "-pick"),
          attribute.type_("color"),
          attribute.value(hex_color(field.value)),
          locked,
          event.on_input(fn(value) {
            EditProperty(field.field, field.property, value)
          }),
        ]),
        html.input([
          attribute.id(id),
          attribute.type_("text"),
          attribute.value(field.value),
          attribute.attribute("placeholder", "Theme color or #hex"),
          locked,
          event.on_input(fn(value) {
            EditProperty(field.field, field.property, value)
          }),
        ]),
      ])
    editing.TextSizeControl | editing.SpacingControl ->
      html.input([
        attribute.id(id),
        attribute.type_("text"),
        attribute.value(field.value),
        attribute.attribute("placeholder", "Token or number"),
        locked,
        event.on_input(fn(value) {
          EditProperty(field.field, field.property, value)
        }),
      ])
  }
  html.div([attribute.class("design-edit-field")], [
    html.label([attribute.attribute("for", id)], [html.text(field.label)]),
    control,
  ])
}

fn hex_color(value: String) -> String {
  case string.starts_with(value, "#") {
    True -> expand_hex(value)
    False ->
      case render.theme_color(render.default_theme(), value) {
        "" -> "#000000"
        hex -> hex
      }
  }
}

fn parsed_number(value: String) -> Option(Float) {
  case int.parse(value) {
    Ok(whole) -> Some(int.to_float(whole))
    Error(_) ->
      case float.parse(value) {
        Ok(decimal) -> Some(decimal)
        Error(_) -> None
      }
  }
}

fn expand_hex(value: String) -> String {
  case string.split(value, "") {
    ["#", red, green, blue] ->
      "#" <> red <> red <> green <> green <> blue <> blue
    _ -> value
  }
}

/// An element drag: the element, and where it would land if dropped now.
pub type Drag {
  Drag(id: String, place: Option(moving.Place))
}

fn start_drag(model: Model, id: String) -> Model {
  case model.edit_mode, drawn_for_editing(model) {
    True, Some(design) ->
      case moving.place_of(design, id) {
        Some(place) -> Model(..model, dragging: Some(Drag(id, Some(place))))
        None -> model
      }
    _, _ -> model
  }
}

fn drag_to(
  model: Model,
  x: Float,
  y: Float,
  positions: List(#(String, Float, Float, Float, Float)),
) -> Model {
  case model.dragging, drawn_for_editing(model) {
    Some(drag), Some(design) -> {
      let place =
        moving.drop_place(design, drag.id, x, y, element_rects(positions))
      Model(..model, dragging: Some(Drag(..drag, place: place)))
    }
    _, _ -> model
  }
}

/// Ends a drag, adding a waiting move when the element lands somewhere new
/// that the design rules allow.
fn drop(model: Model) -> Model {
  let ended = Model(..model, dragging: None)
  case model.dragging, drawn_for_editing(model) {
    Some(Drag(id, Some(place))), Some(design) ->
      case moving.place_of(design, id) == Some(place) {
        False if model.open != "" ->
          case moving.move(design, id, place) {
            Ok(_) -> {
              let changes =
                editing.with_change(
                  waiting_for(model, model.open),
                  editing.Move(id, place),
                )
              editing.store(model.open, changes)
              Model(
                ..ended,
                waiting: dict.insert(model.waiting, model.open, changes),
                edit_error: "",
              )
            }
            Error(_) -> ended
          }
        _ -> ended
      }
    _, _ -> ended
  }
}

/// The editable design with its waiting changes applied.
fn drawn_for_editing(model: Model) -> Option(render.Design) {
  option.map(editable_design(model), fn(saved) {
    editing.apply(waiting_for(model, model.open), saved)
  })
}

/// The drawing with the dragged element shown where it would land.
fn dragged(model: Model, drawing: render.Design) -> render.Design {
  case model.dragging {
    Some(Drag(id, Some(place))) ->
      result.unwrap(moving.move(drawing, id, place), drawing)
    _ -> drawing
  }
}

/// In edit mode a drag moves an element; otherwise it selects a region.
fn canvas_gesture(model: Model) -> attribute.Attribute(Msg) {
  case model.edit_mode && editable_design(model) != None {
    True -> attribute.data("move", "")
    False -> attribute.data("region", "")
  }
}

fn element_rects(
  positions: List(#(String, Float, Float, Float, Float)),
) -> List(#(String, region.Rect)) {
  list.map(positions, fn(position) {
    let #(id, left, top, right, bottom) = position
    #(id, region.Rect(left, top, right, bottom))
  })
}

fn watch_moves(root: Dynamic, send: fn(Msg) -> Nil) -> Nil {
  listen_move(
    root,
    fn(id) { send(DragStarted(id)) },
    fn(x, y, positions) { send(Dragged(x, y, positions)) },
    fn() { send(Dropped) },
    fn() { send(DragCancelled) },
  )
}

@external(javascript, "./api.mjs", "listen_move")
fn listen_move(
  root: Dynamic,
  started: fn(String) -> Nil,
  moved: fn(Float, Float, List(#(String, Float, Float, Float, Float))) -> Nil,
  dropped: fn() -> Nil,
  cancelled: fn() -> Nil,
) -> Nil

/// A selected element's side or corner being dragged.
pub type Side {
  Left
  Right
  Top
  Bottom
  TopLeft
  TopRight
  BottomLeft
  BottomRight
}

/// A resize drag: the element, the side or corner dragged, and its drawn size.
pub type Resize {
  Resize(id: String, side: Side, width: Float, height: Float)
}

/// Starts or moves a resize drag on the one selected editable element.
fn resize(
  model: Model,
  id: String,
  side: String,
  width: Float,
  height: Float,
) -> Model {
  case
    model.edit_mode,
    model.selected,
    resize_side(side),
    editable_node(model)
  {
    True, [selected], Some(side), Some(_) if selected == id ->
      Model(..model, resizing: Some(Resize(id, side, width, height)))
    _, _, _, _ -> model
  }
}

/// Ends a resize drag, adding a waiting fixed size when the design rules
/// allow it, and otherwise leaving the element at its earlier size.
fn resize_drop(model: Model) -> Model {
  let ended = Model(..model, resizing: None)
  case model.resizing, model.open {
    Some(Resize(id, side, width, height)), open if open != "" ->
      case resize_changes(id, side, width, height) {
        [] -> ended
        changes -> {
          let all =
            list.fold(changes, waiting_for(model, open), editing.with_change)
          editing.store(open, all)
          Model(
            ..ended,
            waiting: dict.insert(model.waiting, open, all),
            edit_error: "",
          )
        }
      }
    _, _ -> ended
  }
}

/// The drawing with a resize drag's size shown before it is dropped.
fn resize_preview(model: Model, drawing: render.Design) -> render.Design {
  case model.resizing {
    Some(Resize(id, side, width, height)) ->
      editing.apply(resize_changes(id, side, width, height), drawing)
    None -> drawing
  }
}

fn resize_changes(
  id: String,
  side: Side,
  width: Float,
  height: Float,
) -> List(editing.Change) {
  case
    fixed_size_changes(id, "width", width, width_side(side)),
    fixed_size_changes(id, "height", height, height_side(side))
  {
    Ok(widths), Ok(heights) -> list.append(widths, heights)
    _, _ -> []
  }
}

fn fixed_size_changes(
  id: String,
  property: String,
  value: Float,
  needed: Bool,
) -> Result(List(editing.Change), Nil) {
  case needed {
    False -> Ok([])
    True ->
      case float.round(value) {
        whole if whole > 0 ->
          Ok([
            editing.fixed_change(
              id,
              "properties",
              property,
              int.to_float(whole),
            ),
          ])
        _ -> Error(Nil)
      }
  }
}

fn width_side(side: Side) -> Bool {
  case side {
    Left | Right | TopLeft | TopRight | BottomLeft | BottomRight -> True
    Top | Bottom -> False
  }
}

fn height_side(side: Side) -> Bool {
  case side {
    Top | Bottom | TopLeft | TopRight | BottomLeft | BottomRight -> True
    Left | Right -> False
  }
}

fn resize_side(name: String) -> Option(Side) {
  case name {
    "left" -> Some(Left)
    "right" -> Some(Right)
    "top" -> Some(Top)
    "bottom" -> Some(Bottom)
    "top-left" -> Some(TopLeft)
    "top-right" -> Some(TopRight)
    "bottom-left" -> Some(BottomLeft)
    "bottom-right" -> Some(BottomRight)
    _ -> None
  }
}

fn one_selected(model: Model) -> Bool {
  case model.selected {
    [_] -> True
    _ -> False
  }
}

fn watch_resizes(root: Dynamic, send: fn(Msg) -> Nil) -> Nil {
  listen_resize(
    root,
    fn(id, side, width, height) { send(Resized(id, side, width, height)) },
    fn() { send(ResizeDropped) },
    fn() { send(ResizeCancelled) },
  )
}

@external(javascript, "./api.mjs", "listen_resize")
fn listen_resize(
  root: Dynamic,
  resized: fn(String, String, Float, Float) -> Nil,
  dropped: fn() -> Nil,
  cancelled: fn() -> Nil,
) -> Nil

/// A delete or undo reply can be older than the live list, which stays the
/// source of the list once it has arrived.
fn loaded_keeping_live(model: Model, text: String) -> Model {
  case model.live_listed {
    True ->
      case json.parse(text, designs_decoder()) {
        Ok(_) -> refresh_open(Model(..model, loading: False, error: ""))
        Error(_) -> Model(..model, loading: False, error: error(text))
      }
    False -> loaded(model, text)
  }
}
