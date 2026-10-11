import frontend.{Board, Contracts, Migrate, RouteChanged}
import gleam/int
import gleam/list
import gleam/string
import gleam/uri
import gleeunit
import lustre/element

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn routes_test() {
  let cases = [
    #("/", Board),
    #("/migrate", Migrate),
    #("/contracts", Contracts("")),
    #("/p/web/contracts", Contracts("/p/web")),
    #("/contracts?task=t1", Contracts("")),
    #("/strategy", frontend.Strategy),
    #("/strategy?tab=tasks", frontend.Strategy),
    #("/settings", frontend.Settings),
    #("/view", Board),
    #("/migration", Board),
    #("/nope", Board),
    #("/migrate?filter=all#top", Migrate),
  ]
  list.each(cases, fn(entry) {
    let #(path, expected) = entry
    let assert Ok(uri) = uri.parse(path)
    assert frontend.route(uri) == expected
  })
}

pub fn page_elements_test() {
  let cases = [
    #(#(Board, True), "<solace-board></solace-board>"),
    #(#(Migrate, True), "<solace-migration></solace-migration>"),
    #(#(Contracts(""), True), "<solace-contracts></solace-contracts>"),
    #(
      #(Contracts("/p/web"), True),
      "<solace-contracts base=\"/p/web\"></solace-contracts>",
    ),
    #(#(Board, False), "<solace-board hidden></solace-board>"),
    #(#(frontend.Strategy, True), "<solace-strategy></solace-strategy>"),
    #(#(frontend.Settings, True), "<solace-settings></solace-settings>"),
    #(#(frontend.Settings, False), "<solace-settings hidden></solace-settings>"),
    #(
      #(Contracts("/p/web"), False),
      "<solace-contracts base=\"/p/web\" hidden></solace-contracts>",
    ),
  ]
  list.each(cases, fn(entry) {
    let #(#(route, shown), expected) = entry
    assert element.to_string(frontend.page(route, shown, frontend.no_focus))
      == expected
  })
}

pub fn menu_current_test() {
  let board =
    element.to_string(
      frontend.view(frontend.Model(Board, [Board], frontend.no_focus)),
    )
  let migration =
    element.to_string(
      frontend.view(frontend.Model(Migrate, [Migrate], frontend.no_focus)),
    )
  let contract =
    element.to_string(
      frontend.view(frontend.Model(
        Contracts("/p/web"),
        [Contracts("/p/web")],
        frontend.no_focus,
      )),
    )
  assert string.contains(
    board,
    "<a aria-current=\"page\" class=\"current\" href=\"/\">Board</a>",
  )
  assert string.contains(board, "<a href=\"/migrate\">Migration</a>")
  assert string.contains(board, "<a href=\"/contracts\">Contracts</a>")
  assert string.contains(board, "<a href=\"/strategy\">Strategy</a>")
  assert string.contains(board, "<a href=\"/settings\">Settings</a>")
  let order =
    ["Migration", "Strategy", "Board", "Contracts", "Settings"]
    |> list.map(fn(label) {
      let assert [before, _] = string.split(board, label <> "</a>")
      string.length(before)
    })
  assert order == list.sort(order, int.compare)
  let strategy_page =
    element.to_string(
      frontend.view(frontend.Model(
        frontend.Strategy,
        [frontend.Strategy],
        frontend.no_focus,
      )),
    )
  assert string.contains(
    strategy_page,
    "<a aria-current=\"page\" class=\"current\" href=\"/strategy\">Strategy</a>",
  )
  let settings_page =
    element.to_string(
      frontend.view(frontend.Model(
        frontend.Settings,
        [frontend.Settings],
        frontend.no_focus,
      )),
    )
  assert string.contains(
    settings_page,
    "<a aria-current=\"page\" class=\"current\" href=\"/settings\">Settings</a>",
  )
  assert string.contains(settings_page, "<a href=\"/\">Board</a>")
  assert string.contains(migration, "<a href=\"/\">Board</a>")
  assert string.contains(
    migration,
    "<a aria-current=\"page\" class=\"current\" href=\"/migrate\">Migration</a>",
  )
  assert string.contains(migration, "<a href=\"/contracts\">Contracts</a>")
  assert string.contains(contract, "<a href=\"/\">Board</a>")
  assert string.contains(contract, "<a href=\"/migrate\">Migration</a>")
  // A workspace project's contract page highlights Contracts too.
  assert string.contains(
    contract,
    "<a aria-current=\"page\" class=\"current\" href=\"/contracts\">Contracts</a>",
  )
}

pub fn starts_on_the_board_outside_a_browser_test() {
  let #(model, _effect) = frontend.init(Nil)
  assert model == frontend.Model(Board, [Board], frontend.no_focus)
}

pub fn a_route_change_shows_the_new_page_test() {
  let #(model, _effect) =
    frontend.update(
      frontend.Model(Migrate, [Migrate], frontend.no_focus),
      RouteChanged(Contracts("/p/web")),
    )
  assert model
    == frontend.Model(
      Contracts("/p/web"),
      [Migrate, Contracts("/p/web")],
      frontend.no_focus,
    )
}

pub fn dragging_is_blocked_except_from_a_text_box_test() {
  assert frontend.blocks_drag(True) == False
  assert frontend.blocks_drag(False) == True
}

pub fn a_modified_click_on_a_menu_link_keeps_the_browser_test() {
  assert frontend.keeps_new_tab(
    ctrl: True,
    meta: False,
    shift: False,
    alt: False,
    on_menu_link: True,
  )
  assert frontend.keeps_new_tab(
    ctrl: False,
    meta: True,
    shift: False,
    alt: False,
    on_menu_link: True,
  )
  assert frontend.keeps_new_tab(
    ctrl: False,
    meta: False,
    shift: True,
    alt: False,
    on_menu_link: True,
  )
  assert frontend.keeps_new_tab(
    ctrl: False,
    meta: False,
    shift: False,
    alt: True,
    on_menu_link: True,
  )
  assert frontend.keeps_new_tab(
      ctrl: False,
      meta: False,
      shift: False,
      alt: False,
      on_menu_link: True,
    )
    == False
  assert frontend.keeps_new_tab(
      ctrl: True,
      meta: True,
      shift: True,
      alt: True,
      on_menu_link: False,
    )
    == False
}

pub fn ctrl_f_opens_find_test() {
  assert frontend.find_opens(ctrl: True, meta: False, alt: False, key: "f")
  assert frontend.find_opens(ctrl: False, meta: True, alt: False, key: "f")
  assert frontend.find_opens(ctrl: True, meta: False, alt: False, key: "F")
  assert frontend.find_opens(ctrl: False, meta: False, alt: False, key: "f")
    == False
  assert frontend.find_opens(ctrl: True, meta: False, alt: True, key: "f")
    == False
  assert frontend.find_opens(ctrl: True, meta: False, alt: False, key: "g")
    == False
}

pub fn find_steps_wrap_around_test() {
  assert frontend.next_match(index: 0, count: 3, back: False) == 1
  assert frontend.next_match(index: 2, count: 3, back: False) == 0
  assert frontend.next_match(index: 0, count: 3, back: True) == 2
  assert frontend.next_match(index: 2, count: 3, back: True) == 1
  assert frontend.next_match(index: -1, count: 3, back: False) == 0
  assert frontend.next_match(index: 0, count: 0, back: False) == -1
  assert frontend.next_match(index: 0, count: 0, back: True) == -1
}

pub fn find_count_text_test() {
  assert frontend.count_text(query: "ab", index: 2, count: 12) == "3 of 12"
  assert frontend.count_text(query: "ab", index: -1, count: 0) == "No matches"
  assert frontend.count_text(query: "", index: -1, count: 0) == ""
}

pub fn copy_markdown_test() {
  // A task body as the board renders it.
  let body = [
    frontend.Tag("h4", [], [frontend.Text("Plan")]),
    frontend.Tag("p", [], [
      frontend.Text("Run "),
      frontend.Tag("code", [], [frontend.Text("gleam test")]),
      frontend.Text(" first"),
      frontend.Tag("br", [], []),
      frontend.Text("then "),
      frontend.Tag("strong", [], [frontend.Text("ship")]),
    ]),
    frontend.Tag("ul", [], [
      frontend.Tag("li", [], [
        frontend.Text("a"),
        frontend.Tag("ol", [], [
          frontend.Tag("li", [], [frontend.Text("b")]),
          frontend.Tag("li", [], [frontend.Text("c")]),
        ]),
      ]),
      frontend.Tag("li", [], [
        frontend.Tag(
          "a",
          [#("href", "https://x.dev"), #("rel", "noopener noreferrer")],
          [frontend.Text("docs")],
        ),
      ]),
    ]),
    frontend.Tag("pre", [], [
      frontend.Tag("code", [], [frontend.Text("let x = 1\nlet y = 2")]),
    ]),
    frontend.Tag("p", [], [
      frontend.Tag(
        "img",
        [
          #("alt", "shot"),
          #("src", "/p/web/tasks/t-1/files/0123456789abcdef.png"),
        ],
        [],
      ),
    ]),
  ]
  assert frontend.to_markdown(body)
    == "#### Plan\n\nRun `gleam test` first\nthen **ship**\n\n- a\n  1. b\n  2. c\n- [docs](https://x.dev)\n\n```\nlet x = 1\nlet y = 2\n```\n\n![shot](/tasks/t-1/files/0123456789abcdef.png)"
}

pub fn copy_markdown_of_part_test() {
  // A word inside bold, which shell.mjs wraps again in its inline ancestors.
  assert frontend.to_markdown([
      frontend.Tag("strong", [], [frontend.Text("ship")]),
    ])
    == "**ship**"
  // Two items of a list, wrapped again in the list.
  assert frontend.to_markdown([
      frontend.Tag("ol", [], [
        frontend.Tag("li", [], [frontend.Text("b")]),
        frontend.Tag("li", [], [frontend.Text("c")]),
      ]),
    ])
    == "1. b\n2. c"
  // Text without markup stays as it is; an empty tag at the selection's edge adds nothing.
  assert frontend.to_markdown([frontend.Text("plain *text*")]) == "plain *text*"
  assert frontend.to_markdown([
      frontend.Text("y"),
      frontend.Tag("strong", [], []),
    ])
    == "y"
  // Text around a paragraph becomes paragraphs of its own.
  assert frontend.to_markdown([
      frontend.Text("before"),
      frontend.Tag("p", [], [frontend.Text("inside")]),
      frontend.Tag("span", [], [frontend.Text("after")]),
    ])
    == "before\n\ninside\n\nafter"
}

pub fn copy_markdown_quote_and_numbers_test() {
  let quote =
    frontend.Tag("blockquote", [], [
      frontend.Tag("p", [], [frontend.Text("wise")]),
      frontend.Tag("ul", [], [
        frontend.Tag("li", [], [frontend.Text("x")]),
      ]),
    ])
  assert frontend.to_markdown([quote]) == "> wise\n>\n> - x"
  assert frontend.to_markdown([
      frontend.Tag("blockquote", [], [quote]),
    ])
    == "> > wise\n> >\n> > - x"
  let numbered =
    frontend.Tag("ol", [#("start", "3")], [
      frontend.Tag("li", [], [frontend.Text("a")]),
      frontend.Tag("li", [], [
        frontend.Text("b"),
        frontend.Tag("ol", [], [
          frontend.Tag("li", [], [frontend.Text("c")]),
        ]),
      ]),
    ])
  assert frontend.to_markdown([numbered]) == "3. a\n4. b\n   1. c"
}

/// Going back to a page shows the same element, kept with its state; the page left stays, hidden.
pub fn a_page_left_stays_hidden_test() {
  let #(model, _) =
    frontend.update(
      frontend.Model(Board, [Board], frontend.no_focus),
      RouteChanged(Migrate),
    )
  let #(model, _) = frontend.update(model, RouteChanged(Board))
  assert model == frontend.Model(Board, [Board, Migrate], frontend.no_focus)
  let html = element.to_string(frontend.view(model))
  assert string.contains(
    html,
    "<solace-board></solace-board><solace-migration hidden></solace-migration>",
  )
}

pub fn a_page_not_opened_is_not_drawn_test() {
  let html =
    element.to_string(
      frontend.view(frontend.Model(Migrate, [Migrate], frontend.no_focus)),
    )
  assert !string.contains(html, "<solace-board")
  assert !string.contains(html, "<solace-contracts")
}

/// A contract page opened for another project is the same element, in the same place, with the new base.
pub fn a_reopened_page_keeps_its_place_test() {
  assert frontend.open([Contracts(""), Board], Contracts("/p/web"))
    == [Contracts("/p/web"), Board]
  assert frontend.open([Board], Migrate) == [Board, Migrate]
}

/// D-98: a task link's address is resolved, never shown; the server's reply says where the task opens.
pub fn a_task_link_opens_where_its_task_is_test() {
  let assert Ok(link) = uri.parse("/task/0123456789abcdef")
  assert frontend.route(link) == frontend.TaskLink("0123456789abcdef")
  assert frontend.target("{\"id\":\"t\",\"dir\":\".\",\"archived\":false}")
    == frontend.OnBoard(".")
  assert frontend.target("{\"id\":\"t\",\"dir\":\"web\",\"archived\":true}")
    == frontend.InStrategy("web")
  assert frontend.target(
      "{\"code\":\"E7023\",\"message\":\"E7023 unknown task t\"}",
    )
    == frontend.Missing
  // A network failure or another error (an unreadable task file is E7001 with 409) says nothing about the task.
  assert frontend.target("{\"message\":\"TypeError: Failed to fetch\"}")
    == frontend.Unknown
  assert frontend.target("{\"code\":\"E7001\",\"message\":\"E7001 bad json\"}")
    == frontend.Unknown
  assert frontend.target("not json") == frontend.Unknown
  assert frontend.missing_text == "Task does not exist"
}

/// A found task goes to its page's `focus`, counted so the same link again is news; a click is followed
/// without changing the page model until the server answers, and no task changes nothing.
pub fn a_found_task_is_focused_on_its_page_test() {
  let model =
    frontend.Model(frontend.Strategy, [frontend.Strategy], frontend.no_focus)
  let #(clicked, _) = frontend.update(model, frontend.Follow("t1"))
  assert clicked == model
  let on_board = "{\"id\":\"t1\",\"dir\":\".\",\"archived\":false}"
  let #(model, _) =
    frontend.update(model, frontend.Found("t1", on_board, False))
  assert model.focus == frontend.Focus("solace-board", ".", "t1", 1)
  let archived = "{\"id\":\"t1\",\"dir\":\"a\",\"archived\":true}"
  let #(model, _) =
    frontend.update(model, frontend.Found("t1", archived, False))
  assert model.focus == frontend.Focus("solace-strategy", "a", "t1", 2)
  let #(same, _) =
    frontend.update(model, frontend.Found("t9", "{\"code\":\"E7023\"}", False))
  assert same == model
  let #(same, _) =
    frontend.update(
      model,
      frontend.Found("t9", "{\"message\":\"down\"}", False),
    )
  assert same == model
  // A task link's own address, reached again, shows nothing new until the task is found.
  let #(back, _) = frontend.update(model, RouteChanged(frontend.TaskLink("t1")))
  assert back == model
}

/// Copying a task link gives its markdown back: a bare reference as itself, a labelled one with its label.
pub fn copy_markdown_of_task_links_test() {
  let id = "0123456789abcdef"
  let link = fn(label) {
    frontend.Tag("a", [#("href", "/task/" <> id), #("data-task", id)], [
      frontend.Text(label),
    ])
  }
  assert frontend.to_markdown([
      frontend.Tag("p", [], [
        frontend.Text("See "),
        link("the fix"),
        frontend.Text(" and "),
        link("task:" <> id),
      ]),
    ])
    == "See [the fix](task:" <> id <> ") and task:" <> id
}
