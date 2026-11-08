import gleam/dict
import gleam/list
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
    )
  let html =
    element.to_string(
      render.draw(render.Node("screen", render.Screen(container), [])),
    )
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
    )
  let html =
    element.to_string(
      render.draw(render.Node("screen", render.Screen(container), [])),
    )
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
        )
      let html =
        element.to_string(
          render.draw(render.Node("screen", render.Screen(container), [])),
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
        )
      let html =
        element.to_string(
          render.draw(render.Node("screen", render.Screen(container), [])),
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
        render.Text(render.TextProperties(render.Hug, render.Hug, value, ""))
      let html = element.to_string(render.draw(render.Node("label", text, [])))
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
    )
  let text =
    render.Node(
      "text",
      render.Text(render.TextProperties(
        render.Hug,
        render.Hug,
        render.TextLeft,
        "",
      )),
      [],
    )
  let column = render.Node("column", render.Column(container), [text])
  let row = render.Node("row", render.Row(container), [column])
  let screen = render.Node("screen", render.Screen(container), [row])
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
    )
  let row =
    render.Node("row", render.Row(row_container), [
      render.Node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
        )),
        [],
      ),
      render.Node(
        "hug",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "",
        )),
        [],
      ),
      render.Node(
        "fixed",
        render.Text(render.TextProperties(
          render.Fixed(12.5),
          render.Fixed(3.0),
          render.TextLeft,
          "",
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
    )
  let column =
    render.Node("column", render.Column(column_container), [
      render.Node(
        "tall",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
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
    )
  let styles =
    styles_of(
      element.to_string(
        render.draw(render.Node("screen", render.Screen(container), [])),
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
  let assert Ok(render.Design(id, title, [screen])) = render.decode(json)
  assert id == "0123456789abcdef"
  assert title == "Home"
  assert screen.id == "screen"
  assert screen.children
    == [
      render.Node(
        "label",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextRight,
          "Body",
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
    )
  let html =
    element.to_string(
      render.draw(render.Node("screen", render.Screen(container), [])),
    )
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
    )
  let column =
    render.Node("column", render.Column(container), [
      render.Node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
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
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let leaf =
    render.Node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
      )),
      [],
    )
  let middle = render.Node("middle", render.Column(fill), [leaf])
  let top = render.Node("top", render.Column(hug), [middle])
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
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let tall =
    render.Node(
      "tall",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(200.0),
        render.TextLeft,
        "",
      )),
      [],
    )
  let leaf =
    render.Node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
      )),
      [],
    )
  let column = render.Node("column", render.Column(fill), [leaf])
  let row = render.Node("row", render.Row(hug), [tall, column])
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
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let leaf =
    render.Node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
      )),
      [],
    )
  let column = render.Node("column", render.Column(fill), [leaf])
  let screen = render.Node("screen", render.Screen(fixed), [column])
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
    )
  let leaf =
    render.Node(
      "leaf",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
      )),
      [],
    )
  let column = render.Node("column", render.Column(fixed), [leaf])
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
    )
  let column_container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let tall =
    render.Node(
      "tall",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(500.0),
        render.TextLeft,
        "",
      )),
      [],
    )
  let left = render.Node("left", render.Column(column_container), [tall])
  let right = render.Node("right", render.Column(column_container), [])
  let screen =
    render.Node("screen", render.Screen(screen_container), [left, right])
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
    )
  let row =
    render.Node("row", render.Row(hug), [
      render.Node(
        "fill",
        render.Text(render.TextProperties(
          render.Fill,
          render.Fill,
          render.TextLeft,
          "",
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
    )
  let fixed_screen_container =
    render.Container(
      render.Fixed(400.0),
      render.Fixed(600.0),
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let hug_row = render.Node("hug-row", render.Row(hug_container), [])
  let hug_column = render.Node("hug-column", render.Column(hug_container), [])
  let hug_screen =
    render.Node("hug-screen", render.Screen(fixed_screen_container), [
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
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let item =
    render.Node(
      "item",
      render.Text(render.TextProperties(
        render.Hug,
        render.Fixed(100.0),
        render.TextLeft,
        "",
      )),
      [],
    )
  let deeper_row = render.Node("deeper-row", render.Row(fill), [item])
  let deeper_column =
    render.Node("deeper-column", render.Column(fill), [deeper_row])
  let inner = render.Node("inner", render.Row(fill), [deeper_column])
  let column = render.Node("column", render.Column(fill), [inner])
  let row = render.Node("row", render.Row(hug), [column])
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
    )
  let hug =
    render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let fill =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let text =
    render.Node(
      "text",
      render.Text(render.TextProperties(
        render.Fill,
        render.Fill,
        render.TextLeft,
        "",
      )),
      [],
    )
  let row = render.Node("row", render.Row(fill), [text])
  let hug_column = render.Node("hug-column", render.Column(hug), [row])
  let fixed_column =
    render.Node("fixed-column", render.Column(fixed), [hug_column])
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
    )
  let screen =
    render.Node("screen", render.Screen(container), [
      render.Node(
        "text",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Body",
        )),
        [],
      ),
      render.Node(
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
      render.Node(
        "button",
        render.Button(render.ButtonProperties(render.Hug, render.Hug, "Save")),
        [],
      ),
      render.Node(
        "input",
        render.TextInput(render.InputProperties(render.Hug, render.Hug, "Name")),
        [],
      ),
      render.Node(
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
  let assert Ok(render.Design(_, _, [screen])) = render.decode(json)
  let assert render.Node("screen", render.Screen(screen_container), [row]) =
    screen
  assert screen_container
    == render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let assert render.Node("row", render.Row(row_container), [column]) = row
  assert row_container
    == render.Container(
      render.Fill,
      render.Hug,
      render.SpacingPixels(8.0),
      render.SpacingPixels(4.0),
      render.AlignCenter,
      render.JustifyBetween,
    )
  let assert render.Node(
    "column",
    render.Column(column_container),
    [text, heading, button, input, image],
  ) = column
  assert column_container
    == render.Container(
      render.Hug,
      render.Hug,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let assert render.Node("text", render.Text(text_properties), []) = text
  assert text_properties
    == render.TextProperties(render.Hug, render.Hug, render.TextLeft, "Body")
  let assert render.Node("heading", render.Heading(heading_properties), []) =
    heading
  assert heading_properties
    == render.HeadingProperties(
      render.Hug,
      render.Hug,
      render.TextLeft,
      "Title",
      2,
    )
  let assert render.Node("button", render.Button(button_properties), []) =
    button
  assert button_properties
    == render.ButtonProperties(render.Hug, render.Hug, "Save")
  let assert render.Node("input", render.TextInput(input_properties), []) =
    input
  assert input_properties
    == render.InputProperties(render.Hug, render.Hug, "Name")
  let assert render.Node("image", render.ImagePlaceholder(image_properties), []) =
    image
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

pub fn the_image_placeholder_draws_in_the_current_color_test() {
  let container =
    render.Container(
      render.Fill,
      render.Fill,
      render.SpacingPixels(0.0),
      render.SpacingPixels(0.0),
      render.AlignStart,
      render.JustifyStart,
    )
  let screen =
    render.Node("screen", render.Screen(container), [
      render.Node(
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
    )
  let screen =
    render.Node("screen", render.Screen(container), [
      render.Node(
        "text",
        render.Text(render.TextProperties(
          render.Hug,
          render.Hug,
          render.TextLeft,
          "Body",
        )),
        [],
      ),
      render.Node(
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
      render.Node(
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
      render.Node(
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
      render.Node(
        "button",
        render.Button(render.ButtonProperties(render.Hug, render.Hug, "Save")),
        [],
      ),
      render.Node(
        "input",
        render.TextInput(render.InputProperties(render.Hug, render.Hug, "Name")),
        [],
      ),
      render.Node(
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
    )
  let screen =
    render.Node("screen", render.Screen(container), [
      render.Node(
        "button",
        render.Button(render.ButtonProperties(render.Hug, render.Hug, "Save")),
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
  let assert Ok(render.Design(_, _, [screen])) = render.decode(json)
  let assert render.Node("screen", render.Screen(container), []) = screen
  assert container.gap == render.SpacingToken("md")
  assert container.padding == render.SpacingPixels(4.0)
}
