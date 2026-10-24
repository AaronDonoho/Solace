import frontend.{Board, Contracts, RouteChanged}
import gleam/int
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleam/uri
import gleeunit
import knowledgebase
import lustre/element
import solace_task

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn routes_test() {
  let cases = [
    #("/", Board),
    #("/workspace", frontend.Workspace),
    #("/migrate", frontend.Workspace),
    #("/projects", Board),
    #("/contracts", Contracts("")),
    #("/p/web/contracts", Contracts("/p/web")),
    #("/contracts?task=t1", Contracts("")),
    #("/strategy", frontend.Strategy("vision")),
    #("/strategy?tab=tasks", frontend.Strategy("tasks")),
    #("/strategy?tab=nope", frontend.Strategy("vision")),
    #("/settings", frontend.Settings),
    #("/knowledgebase", frontend.Knowledgebase),
    #("/guide", frontend.Knowledgebase),
    #("/view", Board),
    #("/migration", Board),
    #("/nope", Board),
    #("/workspace?filter=all#top", frontend.Workspace),
    #("/migrate?filter=all#top", frontend.Workspace),
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
    #(#(frontend.Workspace, True), "<solace-migration></solace-migration>"),
    #(#(Contracts(""), True), "<solace-contracts></solace-contracts>"),
    #(
      #(Contracts("/p/web"), True),
      "<solace-contracts base=\"/p/web\"></solace-contracts>",
    ),
    #(#(Board, False), "<solace-board hidden></solace-board>"),
    #(
      #(frontend.Strategy("vision"), True),
      "<solace-strategy tab=\"vision\"></solace-strategy>",
    ),
    #(#(frontend.Settings, True), "<solace-settings></solace-settings>"),
    #(#(frontend.Settings, False), "<solace-settings hidden></solace-settings>"),
    #(
      #(Contracts("/p/web"), False),
      "<solace-contracts base=\"/p/web\" hidden></solace-contracts>",
    ),
  ]
  list.each(cases, fn(entry) {
    let #(#(route, shown), expected) = entry
    assert element.to_string(frontend.page(route, shown, frontend.no_focus, []))
      == expected
  })
}

pub fn menu_current_test() {
  let board =
    element.to_string(
      frontend.view(
        frontend.Model(Board, [Board], frontend.no_focus, [], [], 0, []),
      ),
    )
  let workspace =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Workspace,
          [frontend.Workspace],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  let contract =
    element.to_string(
      frontend.view(
        frontend.Model(
          Contracts("/p/web"),
          [Contracts("/p/web")],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert string.contains(
    board,
    "<a aria-current=\"page\" class=\"current\" href=\"/\">Board</a>",
  )
  assert string.contains(board, "<a href=\"/workspace\">Workspace</a>")
  assert !string.contains(board, "Migration</a>")
  assert string.contains(board, "<a href=\"/contracts\">Contracts</a>")
  assert string.contains(board, "<a href=\"/strategy\">Strategy</a>")
  assert string.contains(board, "<a href=\"/settings\">Settings</a>")
  assert string.contains(board, "<a href=\"/knowledgebase\">Knowledgebase</a>")
  let order =
    ["Workspace", "Strategy", "Board", "Contracts", "Knowledgebase", "Settings"]
    |> list.map(fn(label) {
      let assert [before, _] = string.split(board, label <> "</a>")
      string.length(before)
    })
  assert order == list.sort(order, int.compare)
  let strategy_page =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Strategy("vision"),
          [frontend.Strategy("vision")],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert string.contains(
    strategy_page,
    "<a aria-current=\"page\" class=\"current\" href=\"/strategy\">Strategy</a>",
  )
  let strategy_elsewhere =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Board,
          [frontend.Board, frontend.Strategy("tasks")],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert string.contains(
    strategy_elsewhere,
    "<a href=\"/strategy?tab=tasks\">Strategy</a>",
  )
  let settings_page =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Settings,
          [frontend.Settings],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert string.contains(
    settings_page,
    "<a aria-current=\"page\" class=\"current\" href=\"/settings\">Settings</a>",
  )
  assert string.contains(settings_page, "<a href=\"/\">Board</a>")
  assert string.contains(workspace, "<a href=\"/\">Board</a>")
  assert string.contains(
    workspace,
    "<a aria-current=\"page\" class=\"current\" href=\"/workspace\">Workspace</a>",
  )
  assert string.contains(workspace, "<a href=\"/contracts\">Contracts</a>")
  assert string.contains(contract, "<a href=\"/\">Board</a>")
  assert string.contains(contract, "<a href=\"/workspace\">Workspace</a>")
  // A workspace project's contract page highlights Contracts too.
  assert string.contains(
    contract,
    "<a aria-current=\"page\" class=\"current\" href=\"/contracts\">Contracts</a>",
  )
}

pub fn starts_on_the_board_outside_a_browser_test() {
  let #(model, _effect) = frontend.init(Nil)
  // The board and Strategy are open from the start, hidden, to hear of what needs attention.
  assert model
    == frontend.Model(
      Board,
      [Board, frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
}

pub fn a_route_change_shows_the_new_page_test() {
  let #(model, _effect) =
    frontend.update(
      frontend.Model(
        frontend.Workspace,
        [frontend.Workspace],
        frontend.no_focus,
        [],
        [],
        0,
        [],
      ),
      RouteChanged(Contracts("/p/web"), "{}"),
    )
  assert model
    == frontend.Model(
      Contracts("/p/web"),
      [frontend.Workspace, Contracts("/p/web")],
      frontend.no_focus,
      [],
      [],
      1,
      [],
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
      frontend.Model(Board, [Board], frontend.no_focus, [], [], 0, []),
      RouteChanged(frontend.Workspace, "{}"),
    )
  let #(model, _) = frontend.update(model, RouteChanged(Board, "{}"))
  assert model
    == frontend.Model(
      Board,
      [Board, frontend.Workspace],
      frontend.no_focus,
      [],
      [],
      2,
      [],
    )
  let html = element.to_string(frontend.view(model))
  assert string.contains(
    html,
    "<solace-board></solace-board><solace-migration hidden></solace-migration>",
  )
}

pub fn a_page_not_opened_is_not_drawn_test() {
  let html =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Workspace,
          [frontend.Workspace],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert !string.contains(html, "<solace-board")
  assert !string.contains(html, "<solace-contracts")
}

/// A contract page opened for another project is the same element, in the same place, with the new base.
pub fn a_reopened_page_keeps_its_place_test() {
  assert frontend.open([Contracts(""), Board], Contracts("/p/web"))
    == [Contracts("/p/web"), Board]
  assert frontend.open([Board], frontend.Workspace)
    == [Board, frontend.Workspace]
}

/// D-98: a task link's address is resolved, never shown; the server's reply says where the task opens.
pub fn a_task_link_opens_where_its_task_is_test() {
  let assert Ok(link) = uri.parse("/task/0123456789abcdef")
  assert frontend.route(link) == frontend.RecordLink("task", "0123456789abcdef")
  assert frontend.target(
      "task",
      "{\"id\":\"t\",\"dir\":\".\",\"archived\":false}",
    )
    == frontend.OnBoard(".")
  assert frontend.target(
      "task",
      "{\"id\":\"t\",\"dir\":\"web\",\"archived\":true}",
    )
    == frontend.InStrategy("web")
  // D-129: a task whose home is no project opens on the Strategy page, marked
  // `project` false; a reply without `project` (an older server) is as before.
  assert frontend.target(
      "task",
      "{\"id\":\"t\",\"dir\":\"web\",\"archived\":false,\"project\":false}",
    )
    == frontend.InStrategy("web")
  assert frontend.target(
      "task",
      "{\"id\":\"t\",\"dir\":\".\",\"archived\":false,\"project\":true}",
    )
    == frontend.OnBoard(".")
  assert frontend.target(
      "task",
      "{\"code\":\"E7023\",\"message\":\"E7023 unknown task t\"}",
    )
    == frontend.Missing
  // A network failure or another error (an unreadable task file is E7001 with 409) says nothing about the task.
  assert frontend.target("task", "{\"message\":\"TypeError: Failed to fetch\"}")
    == frontend.Unknown
  assert frontend.target(
      "task",
      "{\"code\":\"E7001\",\"message\":\"E7001 bad json\"}",
    )
    == frontend.Unknown
  assert frontend.target("task", "not json") == frontend.Unknown
  assert frontend.missing_text("task") == "Task does not exist"
  assert frontend.failed_text("task") == "Couldn't look up the task. Try again."
}

/// A found task goes to its page's `focus`, counted so the same link again is news; a click is followed
/// without changing the page model until the server answers, and no task changes nothing.
pub fn a_found_task_is_focused_on_its_page_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(clicked, _) = frontend.update(model, frontend.Follow("task", "t1"))
  assert clicked == model
  let on_board = "{\"id\":\"t1\",\"dir\":\".\",\"archived\":false}"
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found(
        "task",
        "t1",
        on_board,
        False,
        frontend.Strategy("vision"),
        0,
      ),
    )
  assert model.focus == frontend.Focus("solace-board", "task", ".", "t1", 1)
  let archived = "{\"id\":\"t1\",\"dir\":\"a\",\"archived\":true}"
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found("task", "t1", archived, False, frontend.Board, 1),
    )
  assert model.focus == frontend.Focus("solace-strategy", "task", "a", "t1", 2)
  let #(same, _) =
    frontend.update(
      model,
      frontend.Found(
        "task",
        "t9",
        "{\"code\":\"E7023\"}",
        False,
        frontend.Board,
        2,
      ),
    )
  assert same == model
  let #(same, _) =
    frontend.update(
      model,
      frontend.Found(
        "task",
        "t9",
        "{\"message\":\"down\"}",
        False,
        frontend.Board,
        2,
      ),
    )
  assert same == model
  // A task link's own address, reached again, shows nothing new until the task is found.
  let #(back, _) =
    frontend.update(
      model,
      RouteChanged(frontend.RecordLink("task", "t1"), "{}"),
    )
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

/// A hidden page's `attention` flashes its tab until it is shown; the page shown never flags its own.
pub fn a_hidden_page_flags_its_tab_until_shown_test() {
  let model =
    frontend.Model(
      Board,
      [Board, frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(same, _) = frontend.update(model, frontend.Attention("solace-board"))
  assert same == model
  let #(flagged, _) =
    frontend.update(model, frontend.Attention("solace-strategy"))
  assert flagged.attention == ["solace-strategy"]
  let #(again, _) =
    frontend.update(flagged, frontend.Attention("solace-strategy"))
  assert again == flagged
  let html = element.to_string(frontend.view(flagged))
  assert string.contains(
    html,
    "<a class=\"attention\" href=\"/strategy\">Strategy<span class=\"attention-note\"> (needs attention)</span></a>",
  )
  assert string.contains(html, "<a href=\"/workspace\">Workspace</a>")
  let #(shown, _) =
    frontend.update(flagged, RouteChanged(frontend.Strategy("vision"), "{}"))
  assert shown.attention == []
  let html = element.to_string(frontend.view(shown))
  assert string.contains(
    html,
    "<a aria-current=\"page\" class=\"current\" href=\"/strategy\">Strategy</a>",
  )
}

/// D-122: feature and milestone links are resolved as task links are; both open on the Strategy page, and
/// an unknown one says which kind it was.
pub fn a_feature_or_milestone_link_opens_on_strategy_test() {
  let id = "0123456789abcdef"
  let assert Ok(feature) = uri.parse("/feature/" <> id)
  let assert Ok(milestone) = uri.parse("/milestone/" <> id)
  let assert Ok(other) = uri.parse("/epic/" <> id)
  assert frontend.route(feature) == frontend.RecordLink("feature", id)
  assert frontend.route(milestone) == frontend.RecordLink("milestone", id)
  assert frontend.route(other) == frontend.Board
  let found = "{\"id\":\"" <> id <> "\",\"title\":\"Search\"}"
  assert frontend.target("feature", found) == frontend.InStrategy("")
  assert frontend.target("milestone", found) == frontend.InStrategy("")
  assert frontend.target("feature", "{\"code\":\"E7023\"}") == frontend.Missing
  assert frontend.target("milestone", "{\"message\":\"down\"}")
    == frontend.Unknown
  assert frontend.missing_text("feature") == "Feature does not exist"
  assert frontend.missing_text("milestone") == "Milestone does not exist"
  assert frontend.failed_text("milestone")
    == "Couldn't look up the milestone. Try again."
  let model =
    frontend.Model(
      frontend.Board,
      [frontend.Board],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found("feature", "f1", found, False, frontend.Board, 0),
    )
  assert model.focus
    == frontend.Focus("solace-strategy", "feature", "", "f1", 1)
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found(
        "milestone",
        "m1",
        found,
        True,
        frontend.Strategy("features"),
        1,
      ),
    )
  assert model.focus
    == frontend.Focus("solace-strategy", "milestone", "", "m1", 2)
}

/// D-122: copying a feature or milestone link gives its markdown back, and a bare link showing its record's
/// title (marked `data-bare`) is still copied bare.
pub fn copy_markdown_of_feature_and_milestone_links_test() {
  let id = "0123456789abcdef"
  let link = fn(kind, label, bare) {
    frontend.Tag(
      "a",
      [
        #("href", "/" <> kind <> "/" <> id),
        #("data-" <> kind, id),
        ..case bare {
          True -> [#("data-bare", "")]
          False -> []
        }
      ],
      [frontend.Text(label)],
    )
  }
  assert frontend.to_markdown([
      frontend.Tag("p", [], [
        link("feature", "the plan", False),
        frontend.Text(", "),
        link("milestone", "milestone:" <> id, False),
        frontend.Text(", "),
        link("task", "Fix the cache", True),
        frontend.Text(", "),
        link("feature", "Search", True),
      ]),
    ])
    == "[the plan](feature:"
    <> id
    <> "), milestone:"
    <> id
    <> ", task:"
    <> id
    <> ", feature:"
    <> id
}

/// D-140: the Knowledgebase page is the guide, with a table of contents linking to each section, and no
/// chrome of the old separate page.
pub fn the_knowledgebase_shows_the_guide_test() {
  let page =
    element.to_string(
      frontend.view(
        frontend.Model(
          frontend.Knowledgebase,
          [frontend.Knowledgebase],
          frontend.no_focus,
          [],
          [],
          0,
          [],
        ),
      ),
    )
  assert string.contains(
    page,
    "<a aria-current=\"page\" class=\"current\" href=\"/knowledgebase\">Knowledgebase</a>",
  )
  assert string.contains(page, "<article class=\"knowledgebase\">")
  assert list.length(knowledgebase.sections()) == 10
  list.each(knowledgebase.sections(), fn(s) {
    assert string.contains(
      page,
      "<a href=\"#" <> s.id <> "\">" <> s.title <> "</a>",
    )
    assert string.contains(
      page,
      "<section id=\"" <> s.id <> "\"><h2>" <> s.title <> "</h2>",
    )
  })
  assert string.contains(page, "Using Solace")
  assert !string.contains(page, "This guide is a separate page")
  assert !string.contains(page, "Back to the board")
  assert element.to_string(
      frontend.page(frontend.Knowledgebase, False, frontend.no_focus, []),
    )
    |> string.starts_with("<article class=\"knowledgebase\" hidden")
}

/// `/migrate` (D-146) and `/guide` (D-140) are replaced in the history by their page's own address.
pub fn old_addresses_are_renamed_test() {
  assert frontend.renamed_path("/migrate") == Some("/workspace")
  assert frontend.renamed_path("/guide") == Some("/knowledgebase")
  assert frontend.renamed_path("/workspace") == None
  assert frontend.renamed_path("/migration") == None
  assert frontend.renamed_path("/") == None
}

/// D-140: the guide's column table follows the workflow the server answers:
/// its rows are the server's columns, in its order, with the guide's words for
/// the ones it knows and a general pair for a project's own.
pub fn the_guide_follows_the_workflow_test() {
  let columns = [
    solace_task.Column("backlog", "Backlog", [], False),
    solace_task.Column("planning", "Planning", [], False),
    solace_task.Column("needs_review", "Review", [], False),
    solace_task.Column("ship_it", "Ship It", [], True),
  ]
  let page =
    element.to_string(frontend.page(
      frontend.Knowledgebase,
      True,
      frontend.no_focus,
      columns,
    ))
  assert string.contains(
    page,
    "<tr><th>Column</th><th>What it means</th><th>What you can do</th></tr>",
  )
  assert string.contains(page, "<td>Backlog</td>")
  assert string.contains(page, "<td>Review</td>")
  assert string.contains(page, "<td>Ship It</td>")
  assert string.contains(page, "The agent says it's finished")
  assert string.contains(page, "Set by the project's own workflow")
  assert !string.contains(page, "<!--columns-->")
  assert !string.contains(page, "<td>Canceled</td>")
  // The shell asks once, at startup. An answer that is not a workflow (a
  // server that is down) gives the guide its known columns.
  let #(model, _effect) = frontend.init(Nil)
  assert model.columns == []
  let #(down, _) =
    frontend.update(model, frontend.WorkflowLoaded("{\"message\":\"down\"}"))
  assert down.columns == knowledgebase.known_columns()
  let fallback =
    element.to_string(frontend.page(
      frontend.Knowledgebase,
      True,
      frontend.no_focus,
      down.columns,
    ))
  assert string.contains(fallback, "<td>Backlog</td>")
  assert string.contains(fallback, "<td>Review</td>")
  assert string.contains(fallback, "<td>Canceled</td>")
  let #(loaded, _) =
    frontend.update(
      down,
      frontend.WorkflowLoaded(
        "{\"columns\":[{\"id\":\"planning\",\"label\":\"Planning\"}]}",
      ),
    )
  assert loaded.columns
    == [solace_task.Column("planning", "Planning", [], False)]
  let #(still, _) =
    frontend.update(loaded, frontend.WorkflowLoaded("{\"message\":\"down\"}"))
  assert still.columns == loaded.columns
}

/// D-189: a tab change moves the route, and a history move restores the tab and focus.
pub fn a_strategy_tab_and_focus_follow_history_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.Focus("solace-strategy", "feature", "", "f0", 1),
      [],
      [],
      0,
      [],
    )
  let #(model, _) = frontend.update(model, frontend.TabChanged("features"))
  assert model.route == frontend.Strategy("features")
  assert model.opened == [frontend.Strategy("features")]
  assert model.focus
    == frontend.Focus("solace-strategy", "feature", "", "f0", 1)
  let state =
    "{\"focus\":{\"page\":\"solace-strategy\",\"kind\":\"feature\",\"dir\":\"\",\"id\":\"f1\",\"n\":3}}"
  let #(model, _) =
    frontend.update(
      model,
      frontend.RouteChanged(frontend.Strategy("features"), state),
    )
  assert model.route == frontend.Strategy("features")
  assert model.focus
    == frontend.Focus("solace-strategy", "feature", "", "f1", 3)
  let #(model, _) =
    frontend.update(
      model,
      frontend.RouteChanged(frontend.Strategy("tasks"), "{}"),
    )
  assert model.route == frontend.Strategy("tasks")
  assert model.focus == frontend.no_focus
}

/// A record link that stays on the page still gives the page a new focus.
pub fn a_same_page_link_changes_the_focus_test() {
  let model =
    frontend.Model(
      frontend.Board,
      [frontend.Board],
      frontend.Focus("solace-board", "task", ".", "a", 0),
      [],
      [],
      0,
      [],
    )
  let found = "{\"id\":\"b\",\"dir\":\".\",\"archived\":false}"
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found("task", "b", found, False, frontend.Board, 0),
    )
  assert model.route == frontend.Board
  assert model.focus == frontend.Focus("solace-board", "task", ".", "b", 1)
}

/// A lookup answered after the human moved to another page changes nothing.
pub fn a_late_lookup_reply_is_ignored_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(model, _) = frontend.update(model, frontend.Follow("task", "t1"))
  let #(away, _) =
    frontend.update(model, frontend.RouteChanged(frontend.Board, "{}"))
  let on_board = "{\"id\":\"t1\",\"dir\":\".\",\"archived\":false}"
  let #(late, _) =
    frontend.update(
      away,
      frontend.Found(
        "task",
        "t1",
        on_board,
        False,
        frontend.Strategy("vision"),
        0,
      ),
    )
  assert late == away
}

/// A record link reached by history that no longer resolves is replaced by the board.
pub fn a_failed_history_lookup_shows_the_board_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(model, _) =
    frontend.update(
      model,
      frontend.Found(
        "task",
        "t9",
        "{\"message\":\"down\"}",
        True,
        frontend.Strategy("vision"),
        0,
      ),
    )
  assert model.route == frontend.Board
}

/// A record link's click adds a history entry; a load or history move replaces it.
pub fn a_click_pushes_and_a_load_replaces_test() {
  assert frontend.history_move(False) == "push"
  assert frontend.history_move(True) == "replace"
}

/// Vision has one address, whether reached from the menu, a tab change, or a route.
pub fn a_vision_tab_has_one_address_test() {
  assert frontend.strategy_path("vision") == "/strategy"
  assert frontend.strategy_path("features") == "/strategy?tab=features"
}

/// A lookup asked for on Strategy still opens after a tab switch: the page has not moved on.
pub fn a_lookup_survives_a_tab_change_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(model, _) = frontend.update(model, frontend.Follow("feature", "f1"))
  let #(moved, _) =
    frontend.update(
      model,
      frontend.RouteChanged(frontend.Strategy("features"), "{}"),
    )
  let found = "{\"id\":\"f1\",\"title\":\"Search\"}"
  let #(model, _) =
    frontend.update(
      moved,
      frontend.Found(
        "feature",
        "f1",
        found,
        False,
        frontend.Strategy("vision"),
        0,
      ),
    )
  assert model.route == frontend.Strategy("features")
  assert model.focus
    == frontend.Focus("solace-strategy", "feature", "", "f1", 1)
}

/// A menu click's history entry holds no focus, so it clears the page's.
pub fn a_menu_route_change_clears_the_page_focus_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.Focus("solace-strategy", "feature", "", "f1", 2),
      [],
      [],
      0,
      [],
    )
  let #(model, _) =
    frontend.update(
      model,
      frontend.RouteChanged(frontend.Strategy("vision"), "{}"),
    )
  assert model.focus == frontend.no_focus
}

/// A lookup asked on Strategy is ignored if the human left the page and came back before it answered.
pub fn a_lookup_after_leaving_and_returning_is_ignored_test() {
  let model =
    frontend.Model(
      frontend.Strategy("vision"),
      [frontend.Strategy("vision")],
      frontend.no_focus,
      [],
      [],
      0,
      [],
    )
  let #(model, _) = frontend.update(model, frontend.Follow("task", "t1"))
  let #(away, _) =
    frontend.update(model, frontend.RouteChanged(frontend.Board, "{}"))
  let #(back, _) =
    frontend.update(
      away,
      frontend.RouteChanged(frontend.Strategy("vision"), "{}"),
    )
  let on_board = "{\"id\":\"t1\",\"dir\":\".\",\"archived\":false}"
  let #(late, _) =
    frontend.update(
      back,
      frontend.Found(
        "task",
        "t1",
        on_board,
        False,
        frontend.Strategy("vision"),
        0,
      ),
    )
  assert late == back
}
