import gleam/string
import lustre/element
import zoom_bar

pub fn the_bar_shows_its_level_and_names_every_button_test() {
  let html =
    element.to_string(zoom_bar.bar(
      1.25,
      "in",
      "out",
      "reset",
      zoom_bar.Tips(
        "Zoom in (Ctrl + scroll up)",
        "Zoom out (Ctrl + scroll down)",
        "Back to the default view",
      ),
    ))
  assert string.contains(html, "125%")
  assert string.contains(html, "aria-label=\"Zoom in\"")
  assert string.contains(html, "aria-label=\"Zoom out\"")
  assert string.contains(html, "aria-label=\"Reset view\"")
  assert string.contains(html, "data-tip=\"Zoom in (Ctrl + scroll up)\"")
  assert string.contains(html, "data-tip=\"Zoom out (Ctrl + scroll down)\"")
  assert string.contains(html, "data-tip=\"Back to the default view\"")
}

pub fn tips_wait_two_seconds_and_a_click_hides_them_test() {
  assert string.contains(zoom_bar.styles, "transition-delay:2s")
  assert string.contains(zoom_bar.styles, "button:focus::after")
}
