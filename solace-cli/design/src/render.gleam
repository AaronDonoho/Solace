//// Draws a design's element tree with Lustre. Layout properties become
//// styles here, so the design format stays free of CSS.

import gleam/dict.{type Dict}
import gleam/dynamic/decode
import gleam/float
import gleam/json
import gleam/list
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html

pub const outline_styles =
  ".design-element{outline:1px dotted currentColor;outline-offset:-1px}"

pub type Design {
  Design(id: String, title: String, elements: List(Node))
}

pub type Node {
  Node(id: String, kind: Kind, children: List(Node))
}

pub type Kind {
  Screen(Container)
  Row(Container)
  Column(Container)
  Text(TextProperties)
  Heading(HeadingProperties)
  Button(ButtonProperties)
  TextInput(InputProperties)
  ImagePlaceholder(ImageProperties)
}

pub type Container {
  Container(
    width: Size,
    height: Size,
    gap: SpacingValue,
    padding: SpacingValue,
    align: Align,
    justify: Justify,
  )
}

pub type TextProperties {
  TextProperties(width: Size, height: Size, align: TextAlign, content: String)
}

pub type Size {
  Fill
  Hug
  Fixed(Float)
}

pub type Align {
  AlignStart
  AlignCenter
  AlignEnd
  AlignStretch
}

pub type Justify {
  JustifyStart
  JustifyCenter
  JustifyEnd
  JustifyBetween
  JustifyAround
}

pub type TextAlign {
  TextLeft
  TextCenter
  TextRight
}

type Axis {
  Main
  Cross
}

type ParentAxis {
  Across
  Down
}

/// The whole design in a `GET /designs/{id}` reply.
pub fn decode(text: String) -> Result(Design, Nil) {
  case json.parse(text, design_decoder()) {
    Ok(design) -> Ok(design)
    Error(_) -> Error(Nil)
  }
}

/// Draw one element, with its children below it.
pub fn draw(node: Node) -> Element(message) {
  draw_with(default_theme(), node)
}

fn draw_node(
  theme: Theme,
  node: Node,
  parent: ParentAxis,
  parent_width: DrawnSize,
  parent_height: DrawnSize,
) -> Element(message) {
  let #(width, height) =
    drawn_sizes(node.kind, parent, parent_width, parent_height)
  let attributes = [
    attribute.class("design-element"),
    attribute.data("design-id", node.id),
    attribute.styles(list.append(
      size_styles(node, parent, width, height),
      kind_styles(theme, node.kind),
    )),
  ]
  case node.kind {
    Text(text) -> html.div(attributes, [html.text(text.content)])
    Heading(heading) ->
      case heading.level {
        1 -> html.h1(attributes, [html.text(heading.text)])
        2 -> html.h2(attributes, [html.text(heading.text)])
        _ -> html.h3(attributes, [html.text(heading.text)])
      }
    Button(button) -> html.button(attributes, [html.text(button.label)])
    TextInput(input) ->
      html.input(
        list.append(attributes, [
          attribute.attribute("type", "text"),
          attribute.attribute("placeholder", input.placeholder),
        ]),
      )
    ImagePlaceholder(image) -> html.div(attributes, [html.text(image.label)])
    _ ->
      html.div(
        attributes,
        list.map(node.children, fn(child) {
          draw_node(theme, child, child_axis(node.kind), width, height)
        }),
      )
  }
}

fn child_axis(kind: Kind) -> ParentAxis {
  case kind {
    Row(_) -> Across
    _ -> Down
  }
}

fn size_styles(
  node: Node,
  parent: ParentAxis,
  width: DrawnSize,
  height: DrawnSize,
) -> List(#(String, String)) {
  let width_axis = horizontal_axis(parent)
  let height_axis = vertical_axis(parent)
  let screen = is_screen(node.kind)
  list.append(
    axis_styles(width, "width", "min-width", width_axis, screen),
    axis_styles(height, "height", "min-height", height_axis, screen),
  )
}

fn node_sizes(kind: Kind) -> #(Size, Size) {
  case kind {
    Screen(container) -> #(container.width, container.height)
    Row(container) -> #(container.width, container.height)
    Column(container) -> #(container.width, container.height)
    Text(text) -> #(text.width, text.height)
    Heading(heading) -> #(heading.width, heading.height)
    Button(button) -> #(button.width, button.height)
    TextInput(input) -> #(input.width, input.height)
    ImagePlaceholder(image) -> #(image.width, image.height)
  }
}

fn is_screen(kind: Kind) -> Bool {
  case kind {
    Screen(_) -> True
    _ -> False
  }
}

fn axis_styles(
  size: DrawnSize,
  property: String,
  minimum: String,
  axis: Axis,
  screen: Bool,
) -> List(#(String, String)) {
  case size {
    DrawnFill ->
      case axis {
        Main if screen -> [
          #(property, "100%"),
          #("flex", "none"),
          #(minimum, "0"),
        ]
        Main -> [#("flex", "1 1 0"), #(minimum, "0")]
        Cross -> [#("align-self", "stretch"), #(minimum, "0")]
      }
    DrawnStretch ->
      case axis {
        Main -> [#("flex", "1 1 auto"), #(minimum, "0")]
        Cross -> [#("align-self", "stretch"), #(minimum, "0")]
      }
    DrawnHug ->
      case axis {
        Main -> [#("flex", "none")]
        Cross -> []
      }
    DrawnFixed(value) ->
      case axis {
        Main -> [#(property, px(value)), #("flex", "none")]
        Cross -> [#(property, px(value))]
      }
  }
}

fn kind_styles(theme: Theme, kind: Kind) -> List(#(String, String)) {
  case kind {
    Screen(container) ->
      list.append(
        [#("background", theme_color(theme, "background"))],
        container_styles(theme, container, "column"),
      )
    Row(container) -> container_styles(theme, container, "row")
    Column(container) -> container_styles(theme, container, "column")
    Text(text) -> [
      #("display", "block"),
      #("box-sizing", "border-box"),
      #("text-align", text_align(text.align)),
      #("color", theme_color(theme, "text")),
      #("font-size", px(theme_text_size(theme, "body"))),
    ]
    Heading(heading) -> [
      #("display", "block"),
      #("box-sizing", "border-box"),
      #("text-align", text_align(heading.align)),
      #("color", theme_color(theme, "text")),
      #("font-size", px(theme_text_size(theme, heading_token(heading.level)))),
    ]
    Button(_) -> [
      #("display", "inline-flex"),
      #("box-sizing", "border-box"),
      #("align-items", "center"),
      #("justify-content", "center"),
      #("background", theme_color(theme, "primary")),
      #("color", theme_color(theme, "on_primary")),
      #("border-radius", px(theme_radius(theme, "sm"))),
    ]
    TextInput(_) -> [
      #("box-sizing", "border-box"),
      #("background", theme_color(theme, "surface")),
      #("border", "1px solid " <> theme_color(theme, "border")),
      #("border-radius", px(theme_radius(theme, "sm"))),
    ]
    ImagePlaceholder(_) -> [
      #("display", "flex"),
      #("box-sizing", "border-box"),
      #("align-items", "center"),
      #("justify-content", "center"),
      #("border", "1px dashed " <> theme_color(theme, "muted")),
      #("color", theme_color(theme, "muted")),
    ]
  }
}

fn container_styles(
  theme: Theme,
  container: Container,
  direction: String,
) -> List(#(String, String)) {
  [
    #("display", "flex"),
    #("flex-direction", direction),
    #("box-sizing", "border-box"),
    #("gap", px(spacing_value(theme, container.gap))),
    #("padding", px(spacing_value(theme, container.padding))),
    #("align-items", align(container.align)),
    #("justify-content", justify(container.justify)),
  ]
}

fn align(value: Align) -> String {
  case value {
    AlignStart -> "flex-start"
    AlignCenter -> "center"
    AlignEnd -> "flex-end"
    AlignStretch -> "stretch"
  }
}

fn justify(value: Justify) -> String {
  case value {
    JustifyStart -> "flex-start"
    JustifyCenter -> "center"
    JustifyEnd -> "flex-end"
    JustifyBetween -> "space-between"
    JustifyAround -> "space-around"
  }
}

fn text_align(value: TextAlign) -> String {
  case value {
    TextLeft -> "left"
    TextCenter -> "center"
    TextRight -> "right"
  }
}

fn px(value: Float) -> String {
  float.to_string(value) <> "px"
}

pub fn design_decoder() -> decode.Decoder(Design) {
  use id <- decode.field("id", decode.string)
  use title <- decode.field("title", decode.string)
  use elements <- decode.field("elements", decode.list(node_decoder()))
  decode.success(Design(id, title, elements))
}

fn node_decoder() -> decode.Decoder(Node) {
  decode.recursive(fn() {
    use id <- decode.field("id", decode.string)
    use ty <- decode.field("type", decode.string)
    use children <- decode.optional_field(
      "children",
      [],
      decode.list(node_decoder()),
    )
    decode.then(kind_decoder(ty), fn(kind) {
      decode.success(Node(id, kind, children))
    })
  })
}

fn kind_decoder(ty: String) -> decode.Decoder(Kind) {
  case ty {
    "screen" ->
      decode.field("properties", container_decoder(), fn(value) {
        decode.success(Screen(value))
      })
    "row" ->
      decode.field("properties", container_decoder(), fn(value) {
        decode.success(Row(value))
      })
    "column" ->
      decode.field("properties", container_decoder(), fn(value) {
        decode.success(Column(value))
      })
    "text" ->
      decode.field("properties", text_decoder(), fn(value) {
        decode.success(Text(value))
      })
    "heading" ->
      decode.field("properties", heading_decoder(), fn(value) {
        decode.success(Heading(value))
      })
    "button" ->
      decode.field("properties", button_decoder(), fn(value) {
        decode.success(Button(value))
      })
    "text_input" ->
      decode.field("properties", input_decoder(), fn(value) {
        decode.success(TextInput(value))
      })
    "image_placeholder" ->
      decode.field("properties", image_decoder(), fn(value) {
        decode.success(ImagePlaceholder(value))
      })
    _ ->
      decode.failure(
        Text(TextProperties(Fill, Fill, TextLeft, "")),
        "known element type",
      )
  }
}

fn container_decoder() -> decode.Decoder(Container) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use gap <- decode.optional_field("gap", SpacingPixels(0.0), spacing_decoder())
  use padding <- decode.optional_field(
    "padding",
    SpacingPixels(0.0),
    spacing_decoder(),
  )
  use align <- decode.optional_field("align", AlignStart, align_decoder())
  use justify <- decode.optional_field(
    "justify",
    JustifyStart,
    justify_decoder(),
  )
  decode.success(Container(width, height, gap, padding, align, justify))
}

fn text_decoder() -> decode.Decoder(TextProperties) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use align <- decode.optional_field("align", TextLeft, text_align_decoder())
  use content <- decode.optional_field("text", "", decode.string)
  decode.success(TextProperties(width, height, align, content))
}

fn size_decoder() -> decode.Decoder(Size) {
  decode.one_of(
    decode.then(decode.string, fn(value) {
      case value {
        "fill" -> decode.success(Fill)
        "hug" -> decode.success(Hug)
        _ -> decode.failure(Hug, "fill, hug or a fixed size")
      }
    }),
    [decode.map(decode.at(["fixed"], decode.float), Fixed)],
  )
}

fn align_decoder() -> decode.Decoder(Align) {
  decode.then(decode.string, fn(value) {
    case value {
      "start" -> decode.success(AlignStart)
      "center" -> decode.success(AlignCenter)
      "end" -> decode.success(AlignEnd)
      "stretch" -> decode.success(AlignStretch)
      _ -> decode.failure(AlignStart, "align")
    }
  })
}

fn justify_decoder() -> decode.Decoder(Justify) {
  decode.then(decode.string, fn(value) {
    case value {
      "start" -> decode.success(JustifyStart)
      "center" -> decode.success(JustifyCenter)
      "end" -> decode.success(JustifyEnd)
      "between" -> decode.success(JustifyBetween)
      "around" -> decode.success(JustifyAround)
      _ -> decode.failure(JustifyStart, "justify")
    }
  })
}

fn text_align_decoder() -> decode.Decoder(TextAlign) {
  decode.then(decode.string, fn(value) {
    case value {
      "left" -> decode.success(TextLeft)
      "center" -> decode.success(TextCenter)
      "right" -> decode.success(TextRight)
      _ -> decode.failure(TextLeft, "text align")
    }
  })
}

fn draw_size(stated: Size, parent: DrawnSize, axis: Axis) -> DrawnSize {
  case stated, parent, axis {
    Fill, DrawnHug, Main -> DrawnHug
    Fill, DrawnHug, Cross -> DrawnStretch
    Fill, DrawnStretch, _ -> DrawnStretch
    Fill, _, _ -> DrawnFill
    Hug, _, _ -> DrawnHug
    Fixed(value), _, _ -> DrawnFixed(value)
  }
}

fn drawn_sizes(
  kind: Kind,
  parent: ParentAxis,
  parent_width: DrawnSize,
  parent_height: DrawnSize,
) -> #(DrawnSize, DrawnSize) {
  let #(stated_width, stated_height) = node_sizes(kind)
  #(
    draw_size(stated_width, parent_width, horizontal_axis(parent)),
    draw_size(stated_height, parent_height, vertical_axis(parent)),
  )
}

fn horizontal_axis(parent: ParentAxis) -> Axis {
  case parent {
    Across -> Main
    Down -> Cross
  }
}

fn vertical_axis(parent: ParentAxis) -> Axis {
  case parent {
    Across -> Cross
    Down -> Main
  }
}

type DrawnSize {
  DrawnFill
  DrawnStretch
  DrawnHug
  DrawnFixed(Float)
}

pub type HeadingProperties {
  HeadingProperties(
    width: Size,
    height: Size,
    align: TextAlign,
    text: String,
    level: Int,
  )
}

pub type ButtonProperties {
  ButtonProperties(width: Size, height: Size, label: String)
}

pub type InputProperties {
  InputProperties(width: Size, height: Size, placeholder: String)
}

pub type ImageProperties {
  ImageProperties(width: Size, height: Size, label: String)
}

fn heading_decoder() -> decode.Decoder(HeadingProperties) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use align <- decode.optional_field("align", TextLeft, text_align_decoder())
  use text <- decode.field("text", decode.string)
  use level <- decode.field("level", decode.int)
  decode.success(HeadingProperties(width, height, align, text, level))
}

fn button_decoder() -> decode.Decoder(ButtonProperties) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use label <- decode.field("label", decode.string)
  decode.success(ButtonProperties(width, height, label))
}

fn input_decoder() -> decode.Decoder(InputProperties) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use placeholder <- decode.field("placeholder", decode.string)
  decode.success(InputProperties(width, height, placeholder))
}

fn image_decoder() -> decode.Decoder(ImageProperties) {
  use width <- decode.field("width", size_decoder())
  use height <- decode.field("height", size_decoder())
  use label <- decode.field("label", decode.string)
  decode.success(ImageProperties(width, height, label))
}

pub type SpacingValue {
  SpacingToken(String)
  SpacingPixels(Float)
}

pub type Theme {
  Theme(
    colors: Dict(String, String),
    spacing: Dict(String, Float),
    text_sizes: Dict(String, Float),
    radii: Dict(String, Float),
  )
}

pub fn default_theme() -> Theme {
  Theme(
    dict.from_list([
      #("background", "#faf9f5"),
      #("surface", "#ffffff"),
      #("text", "#172226"),
      #("muted", "#667274"),
      #("primary", "#174f78"),
      #("on_primary", "#ffffff"),
      #("border", "#94a7a5"),
    ]),
    dict.from_list([
      #("none", 0.0),
      #("xs", 4.0),
      #("sm", 8.0),
      #("md", 16.0),
      #("lg", 24.0),
      #("xl", 32.0),
    ]),
    dict.from_list([
      #("small", 14.0),
      #("body", 16.0),
      #("h3", 20.0),
      #("h2", 24.0),
      #("h1", 32.0),
    ]),
    dict.from_list([
      #("none", 0.0),
      #("sm", 4.0),
      #("md", 8.0),
    ]),
  )
}

pub fn theme_color(theme: Theme, name: String) -> String {
  case dict.get(theme.colors, name) {
    Ok(value) -> value
    Error(_) -> ""
  }
}

pub fn theme_spacing(theme: Theme, name: String) -> Float {
  case dict.get(theme.spacing, name) {
    Ok(value) -> value
    Error(_) -> 0.0
  }
}

pub fn theme_text_size(theme: Theme, name: String) -> Float {
  case dict.get(theme.text_sizes, name) {
    Ok(value) -> value
    Error(_) -> 0.0
  }
}

pub fn theme_radius(theme: Theme, name: String) -> Float {
  case dict.get(theme.radii, name) {
    Ok(value) -> value
    Error(_) -> 0.0
  }
}

pub fn draw_with(theme: Theme, node: Node) -> Element(message) {
  draw_node(theme, node, Down, DrawnFill, DrawnFill)
}

fn spacing_value(theme: Theme, value: SpacingValue) -> Float {
  case value {
    SpacingToken(name) -> theme_spacing(theme, name)
    SpacingPixels(pixels) -> pixels
  }
}

fn heading_token(level: Int) -> String {
  case level {
    1 -> "h1"
    2 -> "h2"
    _ -> "h3"
  }
}

fn spacing_decoder() -> decode.Decoder(SpacingValue) {
  decode.one_of(decode.map(decode.float, SpacingPixels), [
    decode.map(decode.string, SpacingToken),
  ])
}
