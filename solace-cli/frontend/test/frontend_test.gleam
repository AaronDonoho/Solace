import frontend.{Board, Contracts, Migrate, RouteChanged}
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
    #(Board, "<solace-board></solace-board>"),
    #(Migrate, "<solace-migration></solace-migration>"),
    #(Contracts(""), "<solace-contracts></solace-contracts>"),
    #(
      Contracts("/p/web"),
      "<solace-contracts base=\"/p/web\"></solace-contracts>",
    ),
  ]
  list.each(cases, fn(entry) {
    let #(route, expected) = entry
    assert element.to_string(frontend.page(route)) == expected
  })
}

pub fn menu_current_test() {
  let board = element.to_string(frontend.view(Board))
  let migration = element.to_string(frontend.view(Migrate))
  let contract = element.to_string(frontend.view(Contracts("/p/web")))
  assert string.contains(
    board,
    "<a aria-current=\"page\" class=\"current\" href=\"/\">Board</a>",
  )
  assert string.contains(board, "<a href=\"/migrate\">Migration</a>")
  assert string.contains(board, "<a href=\"/contracts\">Contracts</a>")
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
  let #(route, _effect) = frontend.init(Nil)
  assert route == Board
}

pub fn a_route_change_shows_the_new_page_test() {
  let #(route, _effect) =
    frontend.update(Migrate, RouteChanged(Contracts("/p/web")))
  assert route == Contracts("/p/web")
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
