import gleam/dict
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import lustre/element
import render

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn a_screen_lays_out_as_a_column_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fixed(300.0),
      render.SpacingPixels(8.0),
      render.SpacingPixels(4.0),
      render.AlignCenter,
      render.JustifyBetween,
      render.Plain,
    )
  let html =
    element.to_string(render.draw(node("screen", render.Screen(container), [])))
  assert string.contains(html, "data-design-id=\"screen\"")
  assert string.contains(html, "display:flex")
  assert string.contains(html, "flex-direction:column")
  assert string.contains(html, "gap:8.0px")
  assert string.contains(html, "padding:4.0px")
  assert string.contains(html, "align-items:center")
  assert string.contains(html, "justify-content:space-between")
}

pub fn unstated_spacing_and_alignment_start_at_zero_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let html =
    element.to_string(render.draw(node("screen", render.Screen(container), [])))
  assert string.contains(html, "gap:0.0px")
  assert string.contains(html, "padding:0.0px")
  assert string.contains(html, "align-items:flex-start")
  assert string.contains(html, "justify-content:flex-start")
}

pub fn every_alignment_is_mapped_test() {
  list.each(
    [
      #(render.AlignStart, "flex-start"),
      #(render.AlignCenter, "center"),
      #(render.AlignEnd, "flex-end"),
      #(render.AlignStretch, "stretch"),
    ],
    fn(pair) {
      let #(value, css) = pair
      let container =
        render.Container(
          render.Fill,
          render.Fill,
          render.SpacingPixels(0.0),
          render.SpacingPixels(0.0),
          value,
          render.JustifyStart,
          render.Plain,
        )
      let html =
        element.to_string(
          render.draw(node("screen", render.Screen(container), [])),
        )
      assert string.contains(html, "align-items:" <> css)
    },
  )
}

pub fn every_justification_is_mapped_test() {
  list.each(
    [
      #(render.JustifyStart, "flex-start"),
      #(render.JustifyCenter, "center"),
      #(render.JustifyEnd, "flex-end"),
      #(render.JustifyBetween, "space-between"),
      #(render.JustifyAround, "space-around"),
    ],
    fn(pair) {
      let #(value, css) = pair
      let container =
        render.Container(
          render.Fill,
          render.Fill,
          render.SpacingPixels(0.0),
          render.SpacingPixels(0.0),
          render.AlignStart,
          value,
          render.Plain,
        )
      let html =
        element.to_string(
          render.draw(node("screen", render.Screen(container), [])),
        )
      assert string.contains(html, "justify-content:" <> css)
    },
  )
}

pub fn every_text_alignment_is_mapped_test() {
  list.each(
    [
      #(render.TextLeft, "left"),
      #(render.TextCenter, "center"),
      #(render.TextRight, "right"),
    ],
    fn(pair) {
      let #(value, css) = pair
      let text =
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          value,
          "",
          render.Body,
        ))
      let html = element.to_string(render.draw(node("label", text, [])))
      assert string.contains(html, "text-align:" <> css)
    },
  )
}

pub fn nested_containers_draw_in_order_test() {
  let container =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let text =
    node(
      "text",
      render.Text(render.TextProperties(
        render.Hug,
        render.Hug,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let column = node("column", render.Column(container), [text])
  let row = node("row", render.Row(container), [column])
  let screen = node("screen", render.Screen(container), [row])
  assert ids(element.to_string(render.draw(screen)))
    == ["screen", "row", "column", "text"]
}

pub fn fill_hug_and_fixed_size_on_both_axes_test() {
  let row_container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let row =
    node("row", render.Row(row_container), [
      node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
      node(
        "hug",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
      node(
        "fixed",
        render.Text(render.TextProperties(
          render.Fixed(12.5),
          render.Fixed(3.0),
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
    ])
  let html = element.to_string(render.draw(row))
  let fill = styles_of(html, "fill")
  assert string.contains(fill, "flex:1 1 0")
  assert string.contains(fill, "min-width:0")
  assert string.contains(fill, "align-self:stretch")
  assert string.contains(fill, "min-height:0")
  let hug = styles_of(html, "hug")
  assert !string.contains(hug, "width")
  assert !string.contains(hug, "height")
  assert string.contains(hug, "flex:none")
  assert !string.contains(hug, "align-self")
  let fixed = styles_of(html, "fixed")
  assert string.contains(fixed, "width:12.5px")
  assert string.contains(fixed, "height:3.0px")
  assert string.contains(fixed, "flex:none")

  let column_container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let column =
    node("column", render.Column(column_container), [
      node(
        "tall",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
    ])
  let tall = styles_of(element.to_string(render.draw(column)), "tall")
  assert string.contains(tall, "flex:1 1 0")
  assert string.contains(tall, "min-height:0")
  assert string.contains(tall, "align-self:stretch")
  assert string.contains(tall, "min-width:0")
}

pub fn a_screen_that_fills_takes_the_drawing_area_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let styles =
    styles_of(
      element.to_string(
        render.draw(node("screen", render.Screen(container), [])),
      ),
      "screen",
    )
  assert string.contains(styles, "height:100%")
  assert string.contains(styles, "flex:none")
  assert string.contains(styles, "align-self:stretch")
}

pub fn a_design_decodes_for_drawing_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"screen\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":{\"fixed\":300}},\"children\":[{\"id\":\"label\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"align\":\"right\",\"text\":\"Body\"}}]}]}"
  let assert Ok(render.Design(id, title, [screen], _)) = render.decode(json)
  assert id == "0123456789abcdef"
  assert title == "Home"
  assert screen.id == "screen"
  assert screen.children
    == [
      node(
        "label",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextRight,
          "Body",
          render.Body,
        )),
        [],
      ),
    ]
}

pub fn an_unknown_element_type_is_an_error_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"button\",\"type\":\"button\",\"properties\":{}}]}"
  assert result.is_error(render.decode(json))
}

fn ids(html: String) -> List(String) {
  string.split(html, "data-design-id=\"")
  |> list.drop(1)
  |> list.map(fn(part) {
    case string.split_once(part, "\"") {
      Ok(#(id, _)) -> id
      Error(_) -> ""
    }
  })
}

fn styles_of(html: String, id: String) -> String {
  case string.split_once(html, "data-design-id=\"" <> id <> "\"") {
    Ok(#(_, after)) ->
      case string.split_once(after, ">") {
        Ok(#(attributes, _)) -> attributes
        Error(_) -> ""
      }
    Error(_) -> ""
  }
}

pub fn elements_carry_the_outline_class_test() {
  let container =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let html =
    element.to_string(render.draw(node("screen", render.Screen(container), [])))
  assert string.contains(html, "class=\"design-element\"")
  assert string.contains(render.outline_styles, "outline:1px dotted")
}

pub fn a_fill_child_of_a_hug_column_hugs_along_and_stretches_across_test() {
  let container =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let column =
    node("column", render.Column(container), [
      node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
    ])
  let fill = styles_of(element.to_string(render.draw(column)), "fill")
  assert !string.contains(fill, "flex:1 1 0")
  assert string.contains(fill, "flex:none")
  assert !string.contains(fill, "min-height:0")
  assert string.contains(fill, "align-self:stretch")
  assert string.contains(fill, "min-width:0")
}

pub fn a_fill_child_deep_inside_a_hug_column_hugs_along_and_stretches_across_test() {
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let leaf =
    node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let middle = node("middle", render.Column(fill), [leaf])
  let top = node("top", render.Column(hug), [middle])
  let styles = styles_of(element.to_string(render.draw(top)), "leaf")
  assert !string.contains(styles, "flex:1 1 0")
  assert !string.contains(styles, "flex:1 0 auto")
  assert string.contains(styles, "flex:none")
  assert string.contains(styles, "align-self:stretch")
}

pub fn a_fill_child_of_a_stretched_column_fills_test() {
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let tall =
    node(
      "tall",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(200.0),
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let leaf =
    node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let column = node("column", render.Column(fill), [leaf])
  let row = node("row", render.Row(hug), [tall, column])
  let html = element.to_string(render.draw(row))
  let column_styles = styles_of(html, "column")
  assert string.contains(column_styles, "align-self:stretch")
  let styles = styles_of(html, "leaf")
  assert string.contains(styles, "flex:1 1 auto")
  assert string.contains(styles, "min-height:0")
  assert !string.contains(styles, "flex:1 0 auto")
  assert !string.contains(styles, "flex:1 1 0")
}

pub fn a_fill_child_of_a_fill_parent_under_a_fixed_parent_fills_test() {
  let fixed =
    render.Container(
      render.Fixed(400.0),
      render.Fixed(600.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let leaf =
    node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let column = node("column", render.Column(fill), [leaf])
  let screen = node("screen", render.Screen(fixed), [column])
  let styles = styles_of(element.to_string(render.draw(screen)), "leaf")
  assert string.contains(styles, "flex:1 1 0")
}

pub fn a_fill_child_of_a_fixed_parent_does_not_collapse_test() {
  let fixed =
    render.Container(
      render.Fill,
      render.Fixed(600.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let leaf =
    node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let column = node("column", render.Column(fixed), [leaf])
  let styles = styles_of(element.to_string(render.draw(column)), "leaf")
  assert string.contains(styles, "flex:1 1 0")
  assert string.contains(styles, "min-height:0")
  assert !string.contains(styles, "flex:1 0 auto")
}

pub fn fixed_screen_fill_columns_share_the_space_test() {
  let screen_container =
    render.Container(
      render.Fixed(400.0),
      render.Fixed(600.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let column_container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let tall =
    node(
      "tall",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(500.0),
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let left = node("left", render.Column(column_container), [tall])
  let right = node("right", render.Column(column_container), [])
  let screen = node("screen", render.Screen(screen_container), [left, right])
  let html = element.to_string(render.draw(screen))
  let left_styles = styles_of(html, "left")
  assert string.contains(left_styles, "flex:1 1 0")
  assert !string.contains(left_styles, "flex:1 0 auto")
  let right_styles = styles_of(html, "right")
  assert string.contains(right_styles, "flex:1 1 0")
  assert !string.contains(right_styles, "flex:1 0 auto")
}

pub fn a_fill_child_of_a_hug_row_stretches_test() {
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let row =
    node("row", render.Row(hug), [
      node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
          render.Body,
        )),
        [],
      ),
    ])
  let styles = styles_of(element.to_string(render.draw(row)), "fill")
  assert string.contains(styles, "align-self:stretch")
  assert string.contains(styles, "min-height:0")
  assert string.contains(styles, "flex:none")
  assert !string.contains(styles, "flex:1 1 0")
  assert !string.contains(styles, "min-width:0")
}

pub fn hug_containers_do_not_stretch_across_test() {
  let hug_container =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fixed_screen_container =
    render.Container(
      render.Fixed(400.0),
      render.Fixed(600.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let hug_row = node("hug-row", render.Row(hug_container), [])
  let hug_column = node("hug-column", render.Column(hug_container), [])
  let hug_screen =
    node("hug-screen", render.Screen(fixed_screen_container), [
      hug_row,
      hug_column,
    ])
  let html = element.to_string(render.draw(hug_screen))
  let row_styles = styles_of(html, "hug-row")
  assert !string.contains(row_styles, "align-self")
  assert !string.contains(row_styles, "width")
  assert !string.contains(row_styles, "height")
  let column_styles = styles_of(html, "hug-column")
  assert !string.contains(column_styles, "align-self")
  assert !string.contains(column_styles, "width")
  assert !string.contains(column_styles, "height")
}

pub fn a_fill_row_inside_a_stretched_column_does_not_collapse_test() {
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let item =
    node(
      "item",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(100.0),
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let deeper_row = node("deeper-row", render.Row(fill), [item])
  let deeper_column = node("deeper-column", render.Column(fill), [deeper_row])
  let inner = node("inner", render.Row(fill), [deeper_column])
  let column = node("column", render.Column(fill), [inner])
  let row = node("row", render.Row(hug), [column])
  let html = element.to_string(render.draw(row))
  let inner_styles = styles_of(html, "inner")
  assert string.contains(inner_styles, "flex:1 1 auto")
  assert !string.contains(inner_styles, "flex:1 0 auto")
  assert !string.contains(inner_styles, "flex:1 1 0")
  let deeper_styles = styles_of(html, "deeper-row")
  assert string.contains(deeper_styles, "flex:1 1 auto")
  assert !string.contains(deeper_styles, "flex:1 0 auto")
  assert !string.contains(deeper_styles, "flex:1 1 0")
}

pub fn a_fill_text_inside_a_fixed_column_can_shrink_test() {
  let fixed =
    render.Container(
      render.Fixed(300.0),
      render.Fixed(400.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let text =
    node(
      "text",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
        render.Body,
      )),
      [],
    )
  let row = node("row", render.Row(fill), [text])
  let hug_column = node("hug-column", render.Column(hug), [row])
  let fixed_column = node("fixed-column", render.Column(fixed), [hug_column])
  let styles = styles_of(element.to_string(render.draw(fixed_column)), "text")
  assert string.contains(styles, "flex:1 1 auto")
  assert string.contains(styles, "min-width:0")
  assert !string.contains(styles, "flex:1 0 auto")
}

pub fn every_default_component_draws_with_its_design_id_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let screen =
    node("screen", render.Screen(container), [
      node(
        "text",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Body",
          render.Body,
        )),
        [],
      ),
      node(
        "heading",
        render.Heading(render.HeadingProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Title",
          2,
        )),
        [],
      ),
      node(
        "button",
        render.Button(render.ButtonProperties(
          render.Hug,
          render.Hug,
          "Save",
          render.Primary,
        )),
        [],
      ),
      node(
        "input",
        render.TextInput(render.InputProperties(render.Hug, render.Hug, "Name")),
        [],
      ),
      node(
        "image",
        render.ImagePlaceholder(render.ImageProperties(
          render.Hug,
          render.Hug,
          "Photo",
        )),
        [],
      ),
    ])
  let html = element.to_string(render.draw(screen))
  assert ids(html) == ["screen", "text", "heading", "button", "input", "image"]
  assert string.contains(html, ">Body<")
  assert string.contains(html, "<h2")
  assert string.contains(html, ">Title<")
  assert string.contains(html, ">Save<")
  assert string.contains(html, "placeholder=\"Name\"")
  assert string.contains(html, ">Photo<")
}

pub fn a_design_decodes_every_component_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"screen\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"row\",\"type\":\"row\",\"properties\":{\"width\":\"fill\",\"height\":\"hug\",\"gap\":8,\"padding\":4,\"align\":\"center\",\"justify\":\"between\"},\"children\":[{\"id\":\"column\",\"type\":\"column\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\"},\"children\":[{\"id\":\"text\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Body\"}},{\"id\":\"heading\",\"type\":\"heading\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Title\",\"level\":2}},{\"id\":\"button\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Save\"}},{\"id\":\"input\",\"type\":\"text_input\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"placeholder\":\"Name\"}},{\"id\":\"image\",\"type\":\"image_placeholder\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Photo\"}}]}]}]}]}"
  let assert Ok(render.Design(_, _, [screen], _)) = render.decode(json)
  let assert render.Node("screen", render.Screen(screen_container), [row], _) =
    screen
  assert screen_container
    == render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let assert render.Node("row", render.Row(row_container), [column], _) = row
  assert row_container
    == render.Container(
      render.Fill,
      render.Hug,
      render.SpacingPixels(8.0),
      render.SpacingPixels(4.0),
      render.AlignCenter,
      render.JustifyBetween,
      render.Plain,
    )
  let assert render.Node(
    "column",
    render.Column(column_container),
    [text, heading, button, input, image],
    _,
  ) = column
  assert column_container
    == render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let assert render.Node("text", render.Text(text_properties), [], _) = text
  assert text_properties
    == render.TextProperties(
      render.Hug,
      render.Hug,
      render.TextLeft,
      "Body",
      render.Body,
    )
  let assert render.Node("heading", render.Heading(heading_properties), [], _) =
    heading
  assert heading_properties
    == render.HeadingProperties(
      render.Hug,
      render.Hug,
      render.TextLeft,
      "Title",
      2,
    )
  let assert render.Node("button", render.Button(button_properties), [], _) =
    button
  assert button_properties
    == render.ButtonProperties(render.Hug, render.Hug, "Save", render.Primary)
  let assert render.Node("input", render.TextInput(input_properties), [], _) =
    input
  assert input_properties
    == render.InputProperties(render.Hug, render.Hug, "Name")
  let assert render.Node(
    "image",
    render.ImagePlaceholder(image_properties),
    [],
    _,
  ) = image
  assert image_properties
    == render.ImageProperties(render.Hug, render.Hug, "Photo")
  let html = element.to_string(render.draw(screen))
  assert ids(html)
    == [
      "screen",
      "row",
      "column",
      "text",
      "heading",
      "button",
      "input",
      "image",
    ]
  assert string.contains(html, ">Body<")
  assert string.contains(html, "<h2")
  assert string.contains(html, ">Title<")
  assert string.contains(html, ">Save<")
  assert string.contains(html, "placeholder=\"Name\"")
  assert string.contains(html, ">Photo<")
}

pub fn the_image_placeholder_draws_in_the_muted_color_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let screen =
    node("screen", render.Screen(container), [
      node(
        "image",
        render.ImagePlaceholder(render.ImageProperties(
          render.Hug,
          render.Hug,
          "Photo",
        )),
        [],
      ),
    ])
  let styles = styles_of(element.to_string(render.draw(screen)), "image")
  assert !string.contains(styles, "#666")
  assert string.contains(
    styles,
    render.theme_color(render.default_theme(), "muted"),
  )
}

pub fn components_draw_from_theme_tokens_test() {
  let theme = render.default_theme()
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let screen =
    node("screen", render.Screen(container), [
      node(
        "text",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Body",
          render.Body,
        )),
        [],
      ),
      node(
        "heading1",
        render.Heading(render.HeadingProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "One",
          1,
        )),
        [],
      ),
      node(
        "heading2",
        render.Heading(render.HeadingProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Two",
          2,
        )),
        [],
      ),
      node(
        "heading3",
        render.Heading(render.HeadingProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Three",
          3,
        )),
        [],
      ),
      node(
        "button",
        render.Button(render.ButtonProperties(
          render.Hug,
          render.Hug,
          "Save",
          render.Primary,
        )),
        [],
      ),
      node(
        "input",
        render.TextInput(render.InputProperties(render.Hug, render.Hug, "Name")),
        [],
      ),
      node(
        "image",
        render.ImagePlaceholder(render.ImageProperties(
          render.Hug,
          render.Hug,
          "Photo",
        )),
        [],
      ),
    ])
  let html = element.to_string(render.draw_with(theme, screen))
  assert string.contains(styles_of(html, "text"), "font-size:16.0px")
  assert string.contains(styles_of(html, "heading1"), "font-size:32.0px")
  assert string.contains(styles_of(html, "heading2"), "font-size:24.0px")
  assert string.contains(styles_of(html, "heading3"), "font-size:20.0px")
  let button = styles_of(html, "button")
  assert string.contains(
    button,
    "background:" <> render.theme_color(theme, "primary"),
  )
  assert string.contains(
    button,
    "color:" <> render.theme_color(theme, "on_primary"),
  )
  assert string.contains(button, "border-radius:4.0px")
  let input = styles_of(html, "input")
  assert string.contains(
    input,
    "background:" <> render.theme_color(theme, "surface"),
  )
  assert string.contains(
    input,
    "border:1px solid " <> render.theme_color(theme, "border"),
  )
  assert string.contains(input, "border-radius:4.0px")
  assert string.contains(
    styles_of(html, "image"),
    "color:" <> render.theme_color(theme, "muted"),
  )
}

pub fn changing_a_token_value_changes_what_is_drawn_test() {
  let theme = render.default_theme()
  let changed =
    render.Theme(
      dict.insert(theme.colors, "primary", "#123456"),
      dict.insert(theme.spacing, "md", 99.0),
      theme.text_sizes,
      theme.radii,
    )
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingToken("md"),
      render.SpacingToken("md"),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let screen =
    node("screen", render.Screen(container), [
      node(
        "button",
        render.Button(render.ButtonProperties(
          render.Hug,
          render.Hug,
          "Save",
          render.Primary,
        )),
        [],
      ),
    ])
  let before = element.to_string(render.draw_with(theme, screen))
  let after = element.to_string(render.draw_with(changed, screen))
  assert string.contains(before, "gap:16.0px")
  assert string.contains(after, "gap:99.0px")
  assert string.contains(before, render.theme_color(theme, "primary"))
  assert string.contains(after, "#123456")
  assert before != after
}

pub fn a_spacing_token_is_decoded_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"screen\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\",\"gap\":\"md\",\"padding\":4}}]}"
  let assert Ok(render.Design(_, _, [screen], _)) = render.decode(json)
  let assert render.Node("screen", render.Screen(container), [], _) = screen
  assert container.gap == render.SpacingToken("md")
  assert container.padding == render.SpacingPixels(4.0)
}

pub fn each_button_variant_draws_differently_test() {
  let theme = render.default_theme()
  let styles =
    list.map([render.Primary, render.Secondary, render.Ghost], fn(variant) {
      let button =
        node(
          "button",
          render.Button(render.ButtonProperties(
            render.Hug,
            render.Hug,
            "Save",
            variant,
          )),
          [],
        )
      styles_of(element.to_string(render.draw_with(theme, button)), "button")
    })
  let assert [primary, secondary, ghost] = styles
  assert primary != secondary
  assert secondary != ghost
  assert primary != ghost
  assert string.contains(
    primary,
    "background:" <> render.theme_color(theme, "primary"),
  )
  assert string.contains(
    secondary,
    "background:" <> render.theme_color(theme, "surface"),
  )
  assert string.contains(
    secondary,
    "border:1px solid " <> render.theme_color(theme, "primary"),
  )
  assert string.contains(ghost, "background:transparent")
  assert string.contains(
    ghost,
    "color:" <> render.theme_color(theme, "primary"),
  )
}

pub fn each_text_variant_draws_differently_test() {
  let theme = render.default_theme()
  let styles =
    list.map([render.Body, render.Muted, render.Small], fn(variant) {
      let text =
        node(
          "text",
          render.Text(render.TextProperties(
            render.Hug,
            render.Hug,
            render.TextLeft,
            "Body",
            variant,
          )),
          [],
        )
      styles_of(element.to_string(render.draw_with(theme, text)), "text")
    })
  let assert [body, muted, small] = styles
  assert body != muted
  assert muted != small
  assert body != small
  assert string.contains(body, "font-size:16.0px")
  assert string.contains(body, "color:" <> render.theme_color(theme, "text"))
  assert string.contains(muted, "color:" <> render.theme_color(theme, "muted"))
  assert string.contains(small, "font-size:14.0px")
}

pub fn each_container_variant_draws_differently_test() {
  let theme = render.default_theme()
  let plain =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Plain,
    )
  let card =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
      render.Card,
    )
  let plain_html =
    element.to_string(render.draw(node("row", render.Row(plain), [])))
  let card_html =
    element.to_string(render.draw(node("row", render.Row(card), [])))
  assert plain_html != card_html
  let plain_styles = styles_of(plain_html, "row")
  let card_styles = styles_of(card_html, "row")
  assert !string.contains(plain_styles, "background")
  assert string.contains(
    card_styles,
    "background:" <> render.theme_color(theme, "surface"),
  )
  assert string.contains(
    card_styles,
    "border:1px solid " <> render.theme_color(theme, "border"),
  )
  assert string.contains(card_styles, "border-radius:8.0px")
  let card_column =
    element.to_string(render.draw(node("column", render.Column(card), [])))
  assert string.contains(
    styles_of(card_column, "column"),
    "background:" <> render.theme_color(theme, "surface"),
  )
}

pub fn an_unstated_variant_decodes_to_the_default_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"screen\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"row\",\"type\":\"row\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\"}},{\"id\":\"text\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Body\"}},{\"id\":\"button\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Save\"}}]}]}"
  let assert Ok(render.Design(_, _, [screen], _)) = render.decode(json)
  let assert render.Node(
    "screen",
    render.Screen(screen_container),
    [row, text, button],
    _,
  ) = screen
  assert screen_container.variant == render.Plain
  let assert render.Node("row", render.Row(row_container), [], _) = row
  assert row_container.variant == render.Plain
  let assert render.Node("text", render.Text(text_properties), [], _) = text
  assert text_properties.variant == render.Body
  let assert render.Node("button", render.Button(button_properties), [], _) =
    button
  assert button_properties.variant == render.Primary
}

pub fn an_unknown_variant_is_refused_test() {
  list.each(["button", "text", "row"], fn(ty) {
    let json =
      "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"part\",\"type\":\""
      <> ty
      <> "\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Save\",\"variant\":\"fancy\"}}]}"
    assert result.is_error(render.decode(json))
  })
}

pub fn each_style_override_draws_with_a_token_and_a_literal_test() {
  let theme = render.default_theme()
  let text = fn(overrides) {
    render.Node(
      "text",
      render.Text(render.TextProperties(
        render.Hug,
        render.Hug,
        render.TextLeft,
        "Body",
        render.Body,
      )),
      [],
      overrides,
    )
  }
  let cases = [
    #(
      render.Overrides(Some("surface"), None, None, None, None),
      "background:" <> render.theme_color(theme, "surface"),
    ),
    #(
      render.Overrides(Some("#123456"), None, None, None, None),
      "background:#123456",
    ),
    #(
      render.Overrides(None, Some("primary"), None, None, None),
      "border:1px solid " <> render.theme_color(theme, "primary"),
    ),
    #(
      render.Overrides(None, Some("#cc3333"), None, None, None),
      "border:1px solid #cc3333",
    ),
    #(
      render.Overrides(None, None, Some("muted"), None, None),
      "color:" <> render.theme_color(theme, "muted"),
    ),
    #(
      render.Overrides(None, None, Some("#00ff00"), None, None),
      "color:#00ff00",
    ),
    #(
      render.Overrides(None, None, None, Some(render.TextSizeToken("h2")), None),
      "font-size:24.0px",
    ),
    #(
      render.Overrides(
        None,
        None,
        None,
        Some(render.TextSizePixels(18.0)),
        None,
      ),
      "font-size:18.0px",
    ),
    #(
      render.Overrides(None, None, None, None, Some(render.SpacingToken("md"))),
      "padding:16.0px",
    ),
    #(
      render.Overrides(None, None, None, None, Some(render.SpacingPixels(3.0))),
      "padding:3.0px",
    ),
  ]
  list.each(cases, fn(pair) {
    let #(overrides, expected) = pair
    let styles =
      styles_of(
        element.to_string(render.draw_with(theme, text(overrides))),
        "text",
      )
    assert string.contains(styles, expected)
  })
}

pub fn a_style_override_beats_the_variant_test() {
  let theme = render.default_theme()
  let button =
    render.Node(
      "button",
      render.Button(render.ButtonProperties(
        render.Hug,
        render.Hug,
        "Save",
        render.Secondary,
      )),
      [],
      render.Overrides(
        Some("#ff0000"),
        Some("#00ff00"),
        Some("#0000ff"),
        None,
        None,
      ),
    )
  let styles =
    styles_of(element.to_string(render.draw_with(theme, button)), "button")
  let assert Ok(#(_, after_variant)) =
    string.split_once(
      styles,
      "background:" <> render.theme_color(theme, "surface"),
    )
  assert string.contains(after_variant, "background:#ff0000")
  assert string.contains(styles, "border:1px solid #00ff00")
  assert string.contains(styles, "color:#0000ff")
}

pub fn a_missing_style_draws_as_no_overrides_test() {
  let styleless =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"text\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Body\"}}]}"
  let empty =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"text\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Body\"},\"style\":{}}]}"
  let assert Ok(render.Design(_, _, [first], _)) = render.decode(styleless)
  let assert Ok(render.Design(_, _, [second], _)) = render.decode(empty)
  let assert render.Text(_) = first.kind
  assert element.to_string(render.draw(first))
    == element.to_string(render.draw(second))
}

pub fn a_decoded_style_override_is_drawn_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"button\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Save\",\"variant\":\"secondary\"},\"style\":{\"fill\":\"#ff0000\",\"size\":18,\"padding\":\"md\"}}]}"
  let assert Ok(render.Design(_, _, [button], _)) = render.decode(json)
  let styles = styles_of(element.to_string(render.draw(button)), "button")
  assert string.contains(styles, "background:#ff0000")
  assert string.contains(styles, "font-size:18.0px")
  assert string.contains(styles, "padding:16.0px")
}

fn node(
  id: String,
  kind: render.Kind,
  children: List(render.Node),
) -> render.Node {
  render.Node(id, kind, children, render.no_overrides())
}

pub fn a_link_decodes_into_the_design_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"one\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"go\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Next\",\"navigates_to\":\"two\"}},{\"id\":\"act\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Act\",\"action\":\"save\"}}]},{\"id\":\"two\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"}}]}"
  let assert Ok(design) = render.decode(json)
  assert render.link_target(design, "go") == Some("two")
  assert render.link_target(design, "act") == None
  let assert Some(first) = render.first_screen(design)
  assert first.id == "one"
  let assert Some(target) = render.screen(design, "two")
  assert target.id == "two"
}

pub fn nested_links_use_the_nearest_target_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Home\",\"tab\":\"ui\",\"elements\":[{\"id\":\"one\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"},\"children\":[{\"id\":\"card\",\"type\":\"column\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"navigates_to\":\"two\"},\"children\":[{\"id\":\"share\",\"type\":\"button\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"label\":\"Share\",\"navigates_to\":\"three\"}},{\"id\":\"label\",\"type\":\"text\",\"properties\":{\"width\":\"hug\",\"height\":\"hug\",\"text\":\"Plain\"}}]}]},{\"id\":\"two\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"}},{\"id\":\"three\",\"type\":\"screen\",\"properties\":{\"width\":\"fill\",\"height\":\"fill\"}}]}"
  let assert Ok(design) = render.decode(json)
  assert render.link_target(design, "card") == Some("two")
  assert render.link_target(design, "share") == Some("three")
  assert render.link_target(design, "label") == Some("two")
  let assert Some(screen) = render.first_screen(design)
  let html = element.to_string(render.draw_preview(screen, fn(id) { id }))
  assert ids(html) == ["one", "card", "share", "label"]
}

pub fn a_flow_design_decodes_into_nodes_and_edges_test() {
  let json =
    "{\"schemaVersion\":1,\"id\":\"0123456789abcdef\",\"title\":\"Flow\",\"tab\":\"architecture\",\"elements\":[{\"id\":\"flow\",\"type\":\"flow\",\"properties\":{},\"children\":[{\"id\":\"start\",\"type\":\"node\",\"properties\":{\"label\":\"Start\",\"description\":\"First\"}},{\"id\":\"end\",\"type\":\"node\",\"properties\":{\"label\":\"End\",\"description\":\"Last\"}},{\"id\":\"step\",\"type\":\"edge\",\"properties\":{\"from\":\"start\",\"to\":\"end\",\"label\":\"Next\"}}]}]}"
  let assert Ok(render.Design(_, _, [flow], _)) = render.decode(json)
  let assert render.Node("flow", render.Flow, [start, end, step], _) = flow
  assert start.kind
    == render.DiagramNode(render.DiagramNodeProperties("Start", "First"))
  assert end.kind
    == render.DiagramNode(render.DiagramNodeProperties("End", "Last"))
  assert step.kind
    == render.DiagramEdge(render.DiagramEdgeProperties("start", "end", "Next"))
}
