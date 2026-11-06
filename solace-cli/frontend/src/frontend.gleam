import contracts
import design
import gleam/bool
import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import gleam/uri.{type Uri}
import knowledgebase
import lustre
import lustre/attribute
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html
import migration
import modem
import settings
import setup
import solace_board
import solace_task
import strategy

pub fn main() -> Nil {
  let assert Ok(_) = solace_board.register()
  let assert Ok(_) = contracts.register()
  let assert Ok(_) = strategy.register()
  let assert Ok(_) = migration.register()
  let assert Ok(_) = settings.register()
  let assert Ok(_) = design.register()
  let assert Ok(_) =
    lustre.application(init, update, view)
    |> lustre.start("#app", Nil)
  Nil
}

pub type Route {
  Board
  /// The Workspace page (D-146): the workspace and its migration. `/migrate` lands here.
  Workspace
  Contracts(base: String, task: String)
  /// The Strategy page and the tab its address names.
  Strategy(tab: String)
  /// The Design page and the tab its address names.
  Design(tab: String)
  Settings
  /// The Knowledgebase page (D-140): the shell's own page, holding the guide. `/guide` lands here.
  Knowledgebase
  /// A link's address, `/task/<id>` (D-98), `/feature/<id>` or `/milestone/<id>` (D-122): resolved to the
  /// board or Strategy, never shown itself. `kind` is "task", "feature" or "milestone".
  RecordLink(kind: String, id: String)
}

pub type Msg {
  /// A page's address changed: its route, and the state saved with that entry.
  RouteChanged(Route, String)
  /// A page's tab event: the page's element, and the tab shown changed.
  TabChanged(String, String)
  /// A link was clicked in a page: the record's kind and id.
  Follow(String, String)
  /// A link in a page's shadow root to the Board or a Contract page: its address.
  PageLink(String)
  /// The reply to `GET /<kind>s/find/{id}`: the kind, the id, the reply, whether the link's own address
  /// is replaced in the history (it was loaded or reached by back or forward) rather than added to, and
  /// the route and visit count when it was asked. A reply after the page was left is ignored, even when
  /// it is shown again.
  Found(String, String, String, Bool, Route, Int)
  /// A page's `attention` event: something there needs the human. The page's element.
  Attention(String)
  /// The reply to `GET /workflow`: the root project's columns, which the
  /// Knowledgebase's column table follows (D-140).
  WorkflowLoaded(String)
  /// The reply to `GET /setup`: which recommended tools are still missing.
  SetupLoaded(String)
  /// The checklist's **Check again** button.
  CheckSetup
}

pub fn route(uri: Uri) -> Route {
  case uri.path_segments(uri.path) {
    [] -> Board
    ["workspace"] | ["migrate"] -> Workspace
    ["contracts"] -> Contracts("", task_query(uri.query))
    ["strategy"] -> {
      let query = case uri.query {
        Some(query) -> query
        None -> ""
      }
      Strategy(strategy.tab_name(strategy.tab_from(query)))
    }
    ["design"] -> {
      let query = case uri.query {
        Some(query) -> query
        None -> ""
      }
      Design(design.tab_name(design.tab_from(query)))
    }
    ["settings"] -> Settings
    ["knowledgebase"] | ["guide"] -> Knowledgebase
    ["p", dir, "contracts"] -> Contracts("/p/" <> dir, task_query(uri.query))
    ["task" as kind, id]
    | ["feature" as kind, id]
    | ["milestone" as kind, id] -> RecordLink(kind, id)
    _ -> Board
  }
}

/// A route's page element, hidden unless it is the one shown.
/// The page a link asked for gets the record in its `focus` property, `{kind, dir, id, n}`.
/// Its `attention` event flags its menu tab.
/// The Knowledgebase page takes these columns for its column table.
pub fn page(
  route: Route,
  shown: Bool,
  focus: Focus,
  columns: List(solace_task.Column),
) -> Element(Msg) {
  use <- bool.lazy_guard(route == Knowledgebase, fn() {
    knowledgebase.view(shown, columns)
  })
  let attributes = case route {
    Contracts(base, task) -> [
      attribute.attribute("base", base),
      attribute.attribute("task", task),
    ]
    Strategy(tab) -> [attribute.attribute("tab", tab)]
    Design(tab) -> [attribute.attribute("tab", tab)]
    _ -> []
  }
  let attributes = case focus.page == page_of(route) && focus.id != "" {
    True -> [
      attribute.property(
        "focus",
        json.object([
          #("kind", json.string(focus.kind)),
          #("dir", json.string(focus.dir)),
          #("id", json.string(focus.id)),
          #("n", json.int(focus.n)),
        ]),
      ),
      ..attributes
    ]
    False -> attributes
  }
  let attributes = case shown {
    True -> attributes
    False -> [attribute.attribute("hidden", ""), ..attributes]
  }
  element.element(
    page_of(route),
    [
      event.on("attention", decode.success(Attention(page_of(route)))),
      event.on("tab", tab_changed(page_of(route))),
      ..attributes
    ],
    [],
  )
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  let initial = case modem.initial_uri() {
    Ok(uri) -> route(uri)
    Error(_) -> Board
  }
  let renamed = case modem.initial_uri() {
    Ok(uri) ->
      case renamed_path(uri.path) {
        Some(path) -> modem.replace(path, None, None)
        None -> effect.none()
      }
    Error(_) -> effect.none()
  }
  // A link's address shows the board until the record is found.
  let #(shown, found) = case initial {
    RecordLink(kind, id) -> #(Board, find(kind, id, True, Board, 0))
    _ -> #(initial, effect.none())
  }
  // The board and Strategy are open from the start, hidden, so they hear of
  // what needs the human and can flag their tab.
  let strategy_page = case initial {
    Strategy(tab) -> Strategy(tab)
    _ -> Strategy(strategy.tab_name(strategy.Vision))
  }
  let opened = [shown] |> open(Board) |> open(strategy_page)
  #(
    Model(
      route: shown,
      opened: opened,
      focus: no_focus,
      attention: [],
      columns: [],
      visits: 0,
      setup: [],
    ),
    effect.batch([
      modem.init(fn(uri) { RouteChanged(route(uri), history_state()) }),
      effect.from(fn(send) {
        follow_links(fn(kind, id) { send(Follow(kind, id)) }, fn(href) {
          send(PageLink(href))
        })
      }),
      // A bare link shows its record's title (D-122), in every page.
      effect.from(fn(_) { watch_titles() }),
      // The guide's column table follows the server's workflow (D-140).
      effect.from(fn(send) {
        load_workflow(fn(text) { send(WorkflowLoaded(text)) })
      }),
      // The first-run checklist asks what this computer still needs.
      effect.from(fn(send) { load_setup(fn(text) { send(SetupLoaded(text)) }) }),
      // A tool that breaks after it passed brings the checklist back.
      effect.from(fn(send) { recheck_setup(fn() { send(CheckSetup) }) }),
      found,
      renamed,
    ]),
  )
}

import lustre/event

/// Showing another page keeps the window's scroll of the one left, read before the view hides it, and
/// puts back the scroll of the one shown once it is drawn.
pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    // The page shown is already in front of the human: its tab is never flagged.
    Attention(page) ->
      case
        page == page_of(model.route) || list.contains(model.attention, page)
      {
        True -> #(model, effect.none())
        False -> #(
          Model(..model, attention: list.append(model.attention, [page])),
          effect.none(),
        )
      }
    TabChanged(page, name) -> {
      let next = case page {
        "solace-strategy" -> strategy_route(name)
        "solace-design" -> design_route(name)
        _ -> model.route
      }
      case page == page_of(model.route) && model.route != next {
        False -> #(model, effect.none())
        True -> {
          let #(model, shown) = show(model, next)
          let path = path_of(next)
          #(
            model,
            effect.batch([
              shown,
              effect.from(fn(_) {
                navigate(
                  path,
                  query_of(next),
                  // The mounted page keeps its record focus; a tab entry
                  // holds none, so restoring it never switches tabs.
                  "{}",
                  False,
                )
              }),
            ]),
          )
        }
      }
    }
    RouteChanged(RecordLink(kind, id), _) -> #(
      model,
      find(kind, id, True, model.route, model.visits),
    )
    RouteChanged(route, state) -> {
      let focus = restored_focus(state)
      let focus = case focus.page == page_of(route) {
        True -> focus
        False -> no_focus
      }
      show(Model(..model, focus: focus), route)
    }
    Follow(kind, id) -> #(
      model,
      find(kind, id, False, model.route, model.visits),
    )
    PageLink(href) ->
      case uri.parse(href) {
        Ok(uri) -> {
          let route = route(uri)
          let #(model, shown) = show(model, route)
          #(
            model,
            effect.batch([
              shown,
              effect.from(fn(_) {
                navigate(uri.path, query_of(route), "{}", False)
              }),
            ]),
          )
        }
        Error(_) -> #(model, effect.none())
      }
    Found(kind, id, reply, replace, asked, visits) ->
      case page_of(asked) == page_of(model.route) && visits == model.visits {
        False -> #(model, effect.none())
        True ->
          case target(kind, reply) {
            OnBoard(dir) -> go(model, Board, "/", kind, dir, id, replace)
            InStrategy(dir) ->
              go(model, record_route(kind), "/strategy", kind, dir, id, replace)
            nowhere -> {
              let text = case nowhere {
                Missing -> missing_text(kind)
                _ -> failed_text(kind)
              }
              case replace {
                True -> {
                  let #(model, shown) = show(model, Board)
                  #(
                    model,
                    effect.batch([
                      effect.from(fn(_) { toast(text) }),
                      shown,
                      effect.from(fn(_) { navigate("/", "", "{}", True) }),
                    ]),
                  )
                }
                False -> #(model, effect.from(fn(_) { toast(text) }))
              }
            }
          }
      }
    // A reply that is not a workflow (a server that is down, a project with no
    // board section) keeps a loaded workflow, or gives the guide its known
    // columns.
    WorkflowLoaded(text) ->
      case json.parse(text, solace_task.workflow_decoder()) {
        Ok(workflow) -> #(
          Model(..model, columns: workflow.columns),
          effect.none(),
        )
        Error(_) ->
          case model.columns {
            [] -> #(
              Model(..model, columns: knowledgebase.known_columns()),
              effect.none(),
            )
            _ -> #(model, effect.none())
          }
      }
    // A reply that is not a checklist keeps the one shown: a server that is
    // down says nothing about this computer.
    SetupLoaded(text) ->
      case setup.decode(text) {
        Ok(items) -> #(Model(..model, setup: items), effect.none())
        Error(_) -> #(model, effect.none())
      }
    CheckSetup -> #(
      model,
      effect.from(fn(send) { load_setup(fn(text) { send(SetupLoaded(text)) }) }),
    )
  }
}

/// A menu tab: the current page's is marked; one that needs attention flashes and says so to screen readers.
fn link(
  path: String,
  label: String,
  current: Bool,
  attention: Bool,
) -> Element(msg) {
  let attributes = case current, attention {
    True, _ -> [
      attribute.href(path),
      attribute.class("current"),
      attribute.attribute("aria-current", "page"),
    ]
    False, True -> [attribute.href(path), attribute.class("attention")]
    False, False -> [attribute.href(path)]
  }
  html.a(attributes, case current, attention {
    False, True -> [
      html.text(label),
      html.span([attribute.class("attention-note")], [
        html.text(" (needs attention)"),
      ]),
    ]
    _, _ -> [html.text(label)]
  })
}

pub fn view(model: Model) -> Element(Msg) {
  let shown = page_of(model.route)
  let tab = strategy_tab(model)
  let design_tab = design_tab(model)
  let link = fn(path, label, route) {
    let page = page_of(route)
    link(path, label, shown == page, list.contains(model.attention, page))
  }
  element.fragment([
    html.nav([attribute.class("menu")], [
      link("/workspace", "Workspace", Workspace),
      link(strategy_path(tab), "Strategy", Strategy(tab)),
      link(design_path(design_tab), "Design", Design(design_tab)),
      link("/", "Board", Board),
      link("/contracts", "Contracts", Contracts("", "")),
      link("/knowledgebase", "Knowledgebase", Knowledgebase),
      link("/settings", "Settings", Settings),
    ]),
    setup.view(model.setup, CheckSetup),
    ..list.map(model.opened, fn(r) {
      page(r, page_of(r) == shown, model.focus, model.columns)
    })
  ])
}

/// Whether a drag starting at an element is stopped: only a box you can type in may be dragged from.
pub fn blocks_drag(in_text_box: Bool) -> Bool {
  !in_text_box
}

/// Whether a click must skip modem's in-page router so the browser keeps its new-tab behaviour:
/// a click with a modifier key held, on a menu link.
pub fn keeps_new_tab(
  ctrl ctrl: Bool,
  meta meta: Bool,
  shift shift: Bool,
  alt alt: Bool,
  on_menu_link on_menu_link: Bool,
) -> Bool {
  { ctrl || meta || shift || alt } && on_menu_link
}

/// Whether a key press opens the find bar: Ctrl+F or Cmd+F, from anywhere, including a text box.
/// The key is compared in lower case, so Shift or Caps Lock doesn't stop it.
pub fn find_opens(
  ctrl ctrl: Bool,
  meta meta: Bool,
  alt alt: Bool,
  key key: String,
) -> Bool {
  { ctrl || meta } && !alt && string.lowercase(key) == "f"
}

/// The match to show after Enter (forward) or Shift+Enter (back), wrapping around; -1 when there are none.
pub fn next_match(index index: Int, count count: Int, back back: Bool) -> Int {
  case count <= 0, back {
    True, _ -> -1
    False, False -> { index + 1 } % count
    False, True if index <= 0 -> count - 1
    False, True -> { index - 1 } % count
  }
}

/// The find bar's count: "3 of 12", "No matches", or nothing while the query is empty.
pub fn count_text(
  query query: String,
  index index: Int,
  count count: Int,
) -> String {
  case query, count {
    "", _ -> ""
    _, 0 -> "No matches"
    _, _ -> int.to_string(index + 1) <> " of " <> int.to_string(count)
  }
}

/// A node of copied HTML, as `shell.mjs` hands a selection to `to_markdown`.
pub type Html {
  Text(String)
  Tag(name: String, attributes: List(#(String, String)), children: List(Html))
}

/// The markdown for copied HTML, so a copy of rendered markdown gets its source back. It undoes the board's
/// renderer (`markdown` in the board's api.mjs): paragraphs and line breaks, headings (the renderer draws every
/// level as h4, so they come back as ####), blockquotes, nested lists (a numbered one from its start), code blocks
/// and spans, bold, links, and task images,
/// whose src loses the project's prefix. Other elements give their text. A task, feature or milestone link
/// gives its reference back: bare when it was drawn bare, though it shows the record's title (D-122).
pub fn to_markdown(nodes: List(Html)) -> String {
  blocks(nodes) |> string.join("\n\n")
}

fn blocks(nodes: List(Html)) -> List(String) {
  let #(done, run) =
    list.fold(nodes, #([], []), fn(acc, node) {
      let #(done, run) = acc
      case is_block(node) {
        True -> #(
          list.append(list.reverse(block(node)), paragraph(run, done)),
          [],
        )
        False -> #(done, [node, ..run])
      }
    })
  paragraph(run, done)
  |> list.reverse
  |> list.filter(fn(b) { string.trim(b) != "" })
}

// Inline nodes collected in reverse become one paragraph on top of the blocks so far, also in reverse.
fn paragraph(run: List(Html), done: List(String)) -> List(String) {
  case run {
    [] -> done
    _ -> [inline(list.reverse(run)), ..done]
  }
}

fn is_block(node: Html) -> Bool {
  case node {
    Text(_) -> False
    Tag(name, _, _) ->
      list.contains(
        [
          "p", "div", "section", "article", "details", "summary", "blockquote",
          "pre", "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6",
        ],
        name,
      )
  }
}

fn block(node: Html) -> List(String) {
  case node {
    Tag("p", _, children) -> [inline(children)]
    Tag("pre", _, children) -> ["```\n" <> plain(children) <> "\n```"]
    Tag("ul", _, children) -> [items(children, False, 1, "")]
    Tag("ol", attributes, children) -> [
      items(children, True, first_number(attributes), ""),
    ]
    Tag("li", _, _) -> [items([node], False, 1, "")]
    Tag("blockquote", _, children) -> [
      blocks(children)
      |> string.join("\n\n")
      |> string.split("\n")
      |> list.map(fn(line) {
        case line {
          "" -> ">"
          _ -> "> " <> line
        }
      })
      |> string.join("\n"),
    ]
    Tag("h" <> level, _, children) ->
      case int.parse(level) {
        Ok(n) -> [string.repeat("#", n) <> " " <> inline(children)]
        Error(_) -> blocks(children)
      }
    Tag(_, _, children) -> blocks(children)
    Text(text) -> [text]
  }
}

// A list's items, one line each: `- `, or numbered from `start` by position; a nested list is indented under its item's text.
fn items(
  children: List(Html),
  ordered: Bool,
  start: Int,
  indent: String,
) -> String {
  children
  |> list.filter(fn(child) {
    case child {
      Tag("li", _, _) -> True
      _ -> False
    }
  })
  |> list.index_map(fn(item, i) {
    let marker = case ordered {
      True -> int.to_string(start + i) <> ". "
      False -> "- "
    }
    let inner = case item {
      Tag(_, _, children) -> children
      Text(_) -> []
    }
    let #(nested, text) =
      list.partition(inner, fn(child) {
        case child {
          Tag("ul", _, _) | Tag("ol", _, _) -> True
          _ -> False
        }
      })
    let deeper = indent <> string.repeat(" ", string.length(marker))
    [
      indent <> marker <> inline(text),
      ..list.map(nested, fn(sub) {
        case sub {
          Tag("ol", attributes, children) ->
            items(children, True, first_number(attributes), deeper)
          Tag(_, _, children) -> items(children, False, 1, deeper)
          Text(_) -> ""
        }
      })
    ]
    |> list.filter(fn(line) { line != "" })
    |> string.join("\n")
  })
  |> string.join("\n")
}

// An empty tag, left at the edge of a selection, adds no markup.
fn inline(nodes: List(Html)) -> String {
  list.map(nodes, fn(node) {
    case node {
      Text(text) -> text
      Tag("br", _, _) -> "\n"
      Tag("img", attributes, _) -> image(attributes)
      Tag("strong", _, children) | Tag("b", _, children) ->
        case inline(children) {
          "" -> ""
          text -> "**" <> text <> "**"
        }
      Tag("code", _, children) ->
        case plain(children) {
          "" -> ""
          text -> "`" <> text <> "`"
        }
      Tag("a", attributes, children) ->
        case inline(children), list.key_find(attributes, "href") {
          "", _ -> ""
          text, Ok(href) ->
            case record_ref(href) {
              Ok(ref) ->
                case
                  text == ref
                  || result.is_ok(list.key_find(attributes, "data-bare"))
                {
                  True -> ref
                  False -> "[" <> text <> "](" <> ref <> ")"
                }
              Error(_) -> "[" <> text <> "](" <> href <> ")"
            }
          text, Error(_) -> text
        }
      Tag(_, _, children) -> inline(children)
    }
  })
  |> string.concat
}

// Task images load from the project's prefix on this server; the markdown keeps the path stored with the task.
fn image(attributes: List(#(String, String))) -> String {
  let alt = list.key_find(attributes, "alt") |> result.unwrap("")
  case list.key_find(attributes, "src") {
    Error(_) -> ""
    Ok(src) -> {
      let path = case list.reverse(string.split(src, "/tasks/")) {
        [last, _, ..] -> "/tasks/" <> last
        _ -> src
      }
      "![" <> alt <> "](" <> path <> ")"
    }
  }
}

fn plain(nodes: List(Html)) -> String {
  list.map(nodes, fn(node) {
    case node {
      Text(text) -> text
      Tag("br", _, _) -> "\n"
      Tag(_, _, children) -> plain(children)
    }
  })
  |> string.concat
}

// The number an `ol` begins at: its `start` attribute, or 1.
fn first_number(attributes: List(#(String, String))) -> Int {
  list.key_find(attributes, "start")
  |> result.try(int.parse)
  |> result.unwrap(1)
}

/// The shell's state: the route shown, and every page opened so far, in the order first opened. A page
/// stays in the document once opened, hidden while another shows, so its work is there when you come back.
/// `focus` is the last link followed. `attention` holds the pages, by element, whose menu tab flashes:
/// a page hidden at the time asked for it, and it hasn't been shown since.
/// `columns` are the root project's board columns, as `GET /workflow` answers
/// them; [] until it answers, and the guide's known columns when it answers
/// with no workflow.
pub type Model {
  Model(
    route: Route,
    opened: List(Route),
    focus: Focus,
    attention: List(String),
    columns: List(solace_task.Column),
    /// How many times the shown page changed; a lookup asked before a change
    /// ignores its reply, even when the page is shown again.
    visits: Int,
    /// The tools the first-run check still recommends; [] hides the checklist.
    setup: List(setup.Item),
  )
}

/// The page a route shows: its custom element. Routes to the same page share one element and its state.
pub fn page_of(route: Route) -> String {
  case route {
    Board -> "solace-board"
    Workspace -> "solace-migration"
    Contracts(_, _) -> "solace-contracts"
    Strategy(_) -> "solace-strategy"
    Design(_) -> "solace-design"
    Settings -> "solace-settings"
    Knowledgebase -> "solace-knowledgebase"
    RecordLink(_, _) -> "solace-board"
  }
}

/// The pages open after showing a route: a new page is added at the end, so the others keep their place
/// and Lustre never recreates them; an open one takes the route's attributes (a contract page's base).
pub fn open(opened: List(Route), route: Route) -> List(Route) {
  let page = page_of(route)
  case list.any(opened, fn(r) { page_of(r) == page }) {
    True ->
      list.map(opened, fn(r) {
        case page_of(r) == page {
          True -> route
          False -> r
        }
      })
    False -> list.append(opened, [route])
  }
}

@external(javascript, "./page_scroll.mjs", "leave")
fn leave_scroll(page: String) -> Nil

@external(javascript, "./page_scroll.mjs", "enter")
fn enter_scroll(page: String) -> Nil

/// Shows a route's page; its tab no longer needs attention.
fn show(model: Model, route: Route) -> #(Model, Effect(Msg)) {
  let entering = page_of(route)
  let leaving = page_of(model.route)
  let next =
    Model(
      ..model,
      route: route,
      opened: open(model.opened, route),
      attention: list.filter(model.attention, fn(p) { p != entering }),
      visits: case leaving == entering {
        True -> model.visits
        False -> model.visits + 1
      },
    )
  case leaving == entering {
    True -> #(next, effect.none())
    False -> {
      leave_scroll(page_of(model.route))
      #(next, effect.before_paint(fn(_, _) { enter_scroll(entering) }))
    }
  }
}

/// A link's request to a page (D-98, D-122): the page's element, the record's kind, the project a task is
/// stored in ("" for a feature or milestone), its id, and how many links were followed, so following the
/// same one again is news to the page.
pub type Focus {
  Focus(page: String, kind: String, dir: String, id: String, n: Int)
}

/// No link followed yet.
pub const no_focus = Focus("", "", "", "", 0)

/// Where a link opens: on the board, on the Strategy page (an archived task is off the board, D-95, and
/// features and milestones are only there, D-122), or nowhere: there is no such record, or the lookup failed
/// (the server or the network), so whether there is one isn't known. The first two name the project a task
/// is stored in, "" for a feature or milestone.
pub type Target {
  OnBoard(dir: String)
  InStrategy(dir: String)
  Missing
  Unknown
}

/// Where a link to a record of `kind` opens, from the reply to `GET /<kind>s/find/{id}`: a task's
/// `{dir, archived, project, onBoard}`, a feature's or milestone's `{id, title}`; `E7023` for no such
/// record; any other reply (another error, or a network failure's message) leaves it unknown.
pub fn target(kind: String, reply: String) -> Target {
  let decoder = case kind {
    "task" -> {
      use dir <- decode.field("dir", decode.string)
      use archived <- decode.field("archived", decode.bool)
      use project <- decode.optional_field("project", True, decode.bool)
      use on_board <- decode.optional_field(
        "onBoard",
        None,
        decode.optional(decode.bool),
      )
      decode.success(#(dir, archived, project, on_board))
    }
    _ -> {
      use _ <- decode.field("id", decode.string)
      decode.success(#("", False, False, None))
    }
  }
  case json.parse(reply, decoder) {
    Ok(#(dir, _, _, Some(True))) -> OnBoard(dir)
    Ok(#(dir, _, _, Some(False))) -> InStrategy(dir)
    // An older server has no `onBoard`: a task opens on the board unless it
    // is archived or has no project (D-98, D-129).
    Ok(#(dir, False, True, None)) -> OnBoard(dir)
    Ok(#(dir, _, _, None)) -> InStrategy(dir)
    Error(_) ->
      case json.parse(reply, decode.at(["code"], decode.string)) {
        Ok("E7023") -> Missing
        _ -> Unknown
      }
  }
}

/// The toast for a link to no record of `kind`: "Task does not exist".
pub fn missing_text(kind: String) -> String {
  string.capitalise(kind) <> " does not exist"
}

/// Opens a found record on `route`'s page: the page gets it in its `focus` property. The address changes to
/// the page's, added to the history for a click, even on the page shown, and in place of the link's own
/// address for a load or a history move.
fn go(
  model: Model,
  route: Route,
  path: String,
  kind: String,
  dir: String,
  id: String,
  replace: Bool,
) -> #(Model, Effect(Msg)) {
  let focus = Focus(page_of(route), kind, dir, id, model.focus.n + 1)
  let #(model, shown) = show(Model(..model, focus: focus), route)
  #(
    model,
    effect.batch([
      shown,
      effect.from(fn(_) {
        navigate(path, query_of(route), focus_state(focus), replace)
      }),
    ]),
  )
}

/// Asks the server about record `id` of `kind`: where a task is (D-98), whether a feature or milestone is
/// (D-122), and the route and visit count when it was asked.
fn find(
  kind: String,
  id: String,
  replace: Bool,
  asked: Route,
  visits: Int,
) -> Effect(Msg) {
  effect.from(fn(send) {
    find_record(kind, id, fn(reply) {
      send(Found(kind, id, reply, replace, asked, visits))
    })
  })
}

@external(javascript, "./task_link.mjs", "follow_links")
fn follow_links(
  follow: fn(String, String) -> Nil,
  page: fn(String) -> Nil,
) -> Nil

@external(javascript, "./task_link.mjs", "toast")
fn toast(text: String) -> Nil

/// The toast for a link whose lookup failed: the record may well exist.
pub fn failed_text(kind: String) -> String {
  "Couldn't look up the " <> kind <> ". Try again."
}

/// The markdown reference a link's address stands for (D-98, D-122): `task:<id>` for `/task/<id>`, and
/// so for a feature or a milestone.
fn record_ref(href: String) -> Result(String, Nil) {
  case href {
    "/task/" <> id -> Ok("task:" <> id)
    "/feature/" <> id -> Ok("feature:" <> id)
    "/milestone/" <> id -> Ok("milestone:" <> id)
    _ -> Error(Nil)
  }
}

@external(javascript, "./task_link.mjs", "find_record")
fn find_record(kind: String, id: String, receive: fn(String) -> Nil) -> Nil

@external(javascript, "./task_link.mjs", "watch_titles")
fn watch_titles() -> Nil

/// A page's old address, replaced in the history by its own: the guide's (D-140) and the Migration page's (D-146).
pub fn renamed_path(path: String) -> Option(String) {
  case path {
    "/guide" -> Some("/knowledgebase")
    "/migrate" -> Some("/workspace")
    _ -> None
  }
}

@external(javascript, "./workflow.mjs", "load_workflow")
fn load_workflow(receive: fn(String) -> Nil) -> Nil

/// The route for a Strategy tab name, Vision for an unknown one.
fn strategy_route(tab: String) -> Route {
  tab_route(tab, strategy.tab_name, strategy.tab_from, Strategy)
}

/// The record's page: a feature opens Features, a milestone Roadmap and a task Tasks.
fn record_route(kind: String) -> Route {
  let tab = case kind {
    "feature" -> strategy.Features
    "milestone" -> strategy.Roadmap
    _ -> strategy.Tasks
  }
  Strategy(strategy.tab_name(tab))
}

/// The tab shown on Strategy, or the tab it will open on.
fn strategy_tab(model: Model) -> String {
  page_tab(model, "solace-strategy", strategy.tab_name(strategy.Vision))
}

/// The menu's address for a Strategy tab: Vision has no query.
pub fn strategy_path(tab: String) -> String {
  page_path(Strategy(tab), strategy.tab_name(strategy.Vision))
}

/// The query a route's address carries.
fn query_of(route: Route) -> String {
  case route {
    Strategy(tab) -> strategy_query(tab)
    Design(tab) -> design_query(tab)
    Contracts(_, task) ->
      case task {
        "" -> ""
        _ -> "task=" <> task
      }
    _ -> ""
  }
}

fn focus_json(focus: Focus) -> json.Json {
  json.object([
    #("page", json.string(focus.page)),
    #("kind", json.string(focus.kind)),
    #("dir", json.string(focus.dir)),
    #("id", json.string(focus.id)),
    #("n", json.int(focus.n)),
  ])
}

/// The page focus kept in a history entry.
fn focus_state(focus: Focus) -> String {
  json.object([#("focus", focus_json(focus))]) |> json.to_string
}

fn focus_decoder() -> decode.Decoder(Focus) {
  use page <- decode.field("page", decode.string)
  use kind <- decode.field("kind", decode.string)
  use dir <- decode.field("dir", decode.string)
  use id <- decode.field("id", decode.string)
  use n <- decode.field("n", decode.int)
  decode.success(Focus(page, kind, dir, id, n))
}

/// The focus a history entry holds; none when it holds none.
fn restored_focus(state: String) -> Focus {
  let decoder = {
    use focus <- decode.optional_field("focus", no_focus, focus_decoder())
    decode.success(focus)
  }
  case json.parse(state, decoder) {
    Ok(focus) -> focus
    Error(_) -> no_focus
  }
}

fn tab_changed(page: String) -> decode.Decoder(Msg) {
  use name <- decode.then(decode.string)
  decode.success(TabChanged(page, name))
}

fn navigate(path: String, query: String, state: String, replace: Bool) -> Nil {
  history_navigate(path, query, state, history_move(replace))
}

@external(javascript, "./history.mjs", "navigate")
fn history_navigate(
  path: String,
  query: String,
  state: String,
  move: String,
) -> Nil

/// Whether a history change adds an entry or replaces the one shown: a click adds one, a load or a
/// history move replaces it.
pub fn history_move(replace: Bool) -> String {
  case replace {
    True -> "replace"
    False -> "push"
  }
}

fn strategy_query(tab: String) -> String {
  page_query(tab, strategy.tab_name(strategy.Vision))
}

@external(javascript, "./history.mjs", "state")
fn history_state() -> String

@external(javascript, "./setup_ffi.mjs", "load_setup")
fn load_setup(receive: fn(String) -> Nil) -> Nil

@external(javascript, "./setup_ffi.mjs", "recheck_setup")
fn recheck_setup(receive: fn() -> Nil) -> Nil

/// The `task` query a contract address names, "" when it names none.
fn task_query(query: Option(String)) -> String {
  case query {
    None -> ""
    Some(query) ->
      query
      |> string.split("&")
      |> list.find(fn(pair) { string.starts_with(pair, "task=") })
      |> result.map(fn(pair) { string.drop_start(pair, 5) })
      |> result.unwrap("")
  }
}

/// The route for a Design tab name, Architecture for an unknown one.
fn design_route(tab: String) -> Route {
  tab_route(tab, design.tab_name, design.tab_from, Design)
}

/// The tab shown on Design, or the tab it will open on.
fn design_tab(model: Model) -> String {
  page_tab(model, "solace-design", design.tab_name(design.Architecture))
}

/// The menu's address for a Design tab: Architecture has no query.
pub fn design_path(tab: String) -> String {
  page_path(Design(tab), design.tab_name(design.Architecture))
}

fn design_query(tab: String) -> String {
  page_query(tab, design.tab_name(design.Architecture))
}

/// The tab `page` shows, from the route shown or the last one opened, else `first`.
fn page_tab(model: Model, page: String, first: String) -> String {
  case
    list.find([model.route, ..model.opened], fn(route) {
      page_of(route) == page
    })
  {
    Ok(route) -> route_tab(route, first)
    _ -> first
  }
}

/// A tab's query, which the page's first tab leaves out.
fn page_query(tab: String, first: String) -> String {
  case tab == first {
    True -> ""
    False -> "tab=" <> tab
  }
}

/// A tabbed route's menu address: its path and query.
fn page_path(route: Route, first: String) -> String {
  let tab = route_tab(route, "")
  case page_query(tab, first) {
    "" -> path_of(route)
    query -> path_of(route) <> "?" <> query
  }
}

/// The route for a tab name, through the page's own tab names.
fn tab_route(
  tab: String,
  name: fn(a) -> String,
  from: fn(String) -> a,
  route: fn(String) -> Route,
) -> Route {
  route(name(from("tab=" <> tab)))
}

/// The page's path, without its query: a tab change and a link both use it.
pub fn path_of(route: Route) -> String {
  case route {
    Board -> "/"
    Workspace -> "/workspace"
    Contracts(base, _) -> base <> "/contracts"
    Strategy(_) -> "/strategy"
    Design(_) -> "/design"
    Settings -> "/settings"
    Knowledgebase -> "/knowledgebase"
    RecordLink(kind, id) -> "/" <> kind <> "/" <> id
  }
}

/// The tab a route names, or `first` when it isn't a tabbed page.
fn route_tab(route: Route, first: String) -> String {
  case route {
    Strategy(tab) -> tab
    Design(tab) -> tab
    _ -> first
  }
}
