import contracts
import gleam/bool
import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None}
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
import solace_board
import strategy

pub fn main() -> Nil {
  let assert Ok(_) = solace_board.register()
  let assert Ok(_) = contracts.register()
  let assert Ok(_) = strategy.register()
  let assert Ok(_) = migration.register()
  let assert Ok(_) = settings.register()
  let assert Ok(_) =
    lustre.application(init, update, view)
    |> lustre.start("#app", Nil)
  Nil
}

pub type Route {
  Board
  Migrate
  Contracts(base: String)
  Strategy
  Settings
  /// The Knowledgebase page (D-140): the shell's own page, holding the guide. `/guide` lands here.
  Knowledgebase
  /// A link's address, `/task/<id>` (D-98), `/feature/<id>` or `/milestone/<id>` (D-122): resolved to the
  /// board or Strategy, never shown itself. `kind` is "task", "feature" or "milestone".
  RecordLink(kind: String, id: String)
}

pub type Msg {
  RouteChanged(Route)
  /// A link was clicked in a page: the record's kind and id.
  Follow(String, String)
  /// The reply to `GET /<kind>s/find/{id}`: the kind, the id, the reply, and whether the link's own address
  /// is replaced in the history (it was loaded or reached by back or forward) rather than added to.
  Found(String, String, String, Bool)
  /// A page's `attention` event: something there needs the human. The page's element.
  Attention(String)
}

pub fn route(uri: Uri) -> Route {
  case uri.path_segments(uri.path) {
    [] -> Board
    ["migrate"] -> Migrate
    ["contracts"] -> Contracts("")
    ["strategy"] -> Strategy
    ["settings"] -> Settings
    ["knowledgebase"] | ["guide"] -> Knowledgebase
    ["p", dir, "contracts"] -> Contracts("/p/" <> dir)
    ["task" as kind, id]
    | ["feature" as kind, id]
    | ["milestone" as kind, id] -> RecordLink(kind, id)
    _ -> Board
  }
}

/// A route's page element, hidden unless it is the one shown.
/// The page a link asked for gets the record in its `focus` property, `{kind, dir, id, n}`.
/// Its `attention` event flags its menu tab.
pub fn page(route: Route, shown: Bool, focus: Focus) -> Element(Msg) {
  use <- bool.lazy_guard(route == Knowledgebase, fn() {
    knowledgebase.view(shown)
  })
  let attributes = case route {
    Contracts(base) if base != "" -> [attribute.attribute("base", base)]
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
  // The guide's old address shows the Knowledgebase under its own (D-140).
  let renamed = case modem.initial_uri() {
    Ok(uri) if uri.path == "/guide" ->
      modem.replace("/knowledgebase", None, None)
    _ -> effect.none()
  }
  // A link's address shows the board until the record is found.
  let #(shown, found) = case initial {
    RecordLink(kind, id) -> #(Board, find(kind, id, True))
    _ -> #(initial, effect.none())
  }
  // The board and Strategy are open from the start, hidden, so they hear of
  // what needs the human and can flag their tab.
  let opened = [shown] |> open(Board) |> open(Strategy)
  #(
    Model(route: shown, opened: opened, focus: no_focus, attention: []),
    effect.batch([
      modem.init(fn(uri) { RouteChanged(route(uri)) }),
      effect.from(fn(send) {
        follow_links(fn(kind, id) { send(Follow(kind, id)) })
      }),
      // A bare link shows its record's title (D-122), in every page.
      effect.from(fn(_) { watch_titles() }),
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
    RouteChanged(RecordLink(kind, id)) -> #(model, find(kind, id, True))
    RouteChanged(route) -> show(model, route)
    Follow(kind, id) -> #(model, find(kind, id, False))
    Found(kind, id, reply, replace) ->
      case target(kind, reply) {
        OnBoard(dir) -> go(model, Board, "/", kind, dir, id, replace)
        InStrategy(dir) ->
          go(model, Strategy, "/strategy", kind, dir, id, replace)
        nowhere -> {
          let text = case nowhere {
            Missing -> missing_text(kind)
            _ -> failed_text(kind)
          }
          #(
            model,
            effect.batch([
              effect.from(fn(_) { toast(text) }),
              case replace {
                True -> modem.replace("/", None, None)
                False -> effect.none()
              },
            ]),
          )
        }
      }
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
  let link = fn(path, label, route) {
    let page = page_of(route)
    link(path, label, shown == page, list.contains(model.attention, page))
  }
  element.fragment([
    html.nav([attribute.class("menu")], [
      link("/migrate", "Migration", Migrate),
      link("/strategy", "Strategy", Strategy),
      link("/", "Board", Board),
      link("/contracts", "Contracts", Contracts("")),
      link("/knowledgebase", "Knowledgebase", Knowledgebase),
      link("/settings", "Settings", Settings),
    ]),
    ..list.map(model.opened, fn(r) { page(r, page_of(r) == shown, model.focus) })
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
pub type Model {
  Model(
    route: Route,
    opened: List(Route),
    focus: Focus,
    attention: List(String),
  )
}

/// The page a route shows: its custom element. Routes to the same page share one element and its state.
pub fn page_of(route: Route) -> String {
  case route {
    Board -> "solace-board"
    Migrate -> "solace-migration"
    Contracts(_) -> "solace-contracts"
    Strategy -> "solace-strategy"
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
  let next =
    Model(
      ..model,
      route: route,
      opened: open(model.opened, route),
      attention: list.filter(model.attention, fn(p) { p != entering }),
    )
  case page_of(model.route) == entering {
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
/// `{dir, archived, project}`, a feature's or milestone's `{id, title}`; `E7023` for no such record; any
/// other reply (another error, or a network failure's message) leaves it unknown.
pub fn target(kind: String, reply: String) -> Target {
  let decoder = case kind {
    "task" -> {
      use dir <- decode.field("dir", decode.string)
      use archived <- decode.field("archived", decode.bool)
      use project <- decode.optional_field("project", True, decode.bool)
      decode.success(#(dir, archived, project))
    }
    _ -> {
      use _ <- decode.field("id", decode.string)
      decode.success(#("", True, True))
    }
  }
  case json.parse(reply, decoder) {
    // A task with no project (D-129) opens on the Strategy page, like an
    // archived one.
    Ok(#(dir, False, True)) -> OnBoard(dir)
    Ok(#(dir, _, _)) -> InStrategy(dir)
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
/// the page's, added to the history for a click, in place of the task link's own address otherwise; a click
/// on the page shown changes nothing.
fn go(
  model: Model,
  route: Route,
  path: String,
  kind: String,
  dir: String,
  id: String,
  replace: Bool,
) -> #(Model, Effect(Msg)) {
  let model =
    Model(
      ..model,
      focus: Focus(page_of(route), kind, dir, id, model.focus.n + 1),
    )
  case replace, page_of(model.route) == page_of(route) {
    True, _ -> #(model, modem.replace(path, None, None))
    False, True -> #(model, effect.none())
    False, False -> #(model, modem.push(path, None, None))
  }
}

/// Asks the server about record `id` of `kind`: where a task is (D-98), whether a feature or milestone is
/// (D-122).
fn find(kind: String, id: String, replace: Bool) -> Effect(Msg) {
  effect.from(fn(send) {
    find_record(kind, id, fn(reply) { send(Found(kind, id, reply, replace)) })
  })
}

@external(javascript, "./task_link.mjs", "follow_links")
fn follow_links(follow: fn(String, String) -> Nil) -> Nil

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
