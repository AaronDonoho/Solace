import frontend.{Board, Migrate, RouteChanged, View}
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
    #("/view", View("")),
    #("/p/web/view", View("/p/web")),
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
    #(View(""), "<solace-view></solace-view>"),
    #(View("/p/web"), "<solace-view base=\"/p/web\"></solace-view>"),
  ]
  list.each(cases, fn(entry) {
    let #(route, expected) = entry
    assert element.to_string(frontend.page(route)) == expected
  })
}

pub fn menu_current_test() {
  let board = element.to_string(frontend.view(Board))
  let migration = element.to_string(frontend.view(Migrate))
  let contract = element.to_string(frontend.view(View("/p/web")))
  assert string.contains(
    board,
    "<a aria-current=\"page\" class=\"current\" href=\"/\">Board</a>",
  )
  assert string.contains(board, "<a href=\"/migrate\">Migration</a>")
  assert string.contains(migration, "<a href=\"/\">Board</a>")
  assert string.contains(
    migration,
    "<a aria-current=\"page\" class=\"current\" href=\"/migrate\">Migration</a>",
  )
  assert string.contains(contract, "aria-current") == False
  assert string.contains(contract, "class=\"current\"") == False
}

pub fn starts_on_the_board_outside_a_browser_test() {
  let #(route, _effect) = frontend.init(Nil)
  assert route == Board
}

pub fn a_route_change_shows_the_new_page_test() {
  let #(route, _effect) = frontend.update(Migrate, RouteChanged(View("/p/web")))
  assert route == View("/p/web")
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
