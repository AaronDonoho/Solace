//// A human's waiting changes to a design: the operations Save sends, how
//// they redraw the saved design, and where the browser keeps them.

import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import moving
import render

pub type Value {
  Text(String)
  Number(Float)
  Integer(Int)
  Fixed(Float)
  Null
}

pub type Change {
  Change(id: String, field: String, property: String, value: Value)
  /// Moves the element to a place among a parent's children.
  Move(id: String, place: moving.Place)
}

pub fn text_change(
  id: String,
  field: String,
  property: String,
  value: String,
) -> Change {
  Change(id, field, property, Text(value))
}

pub fn number_change(
  id: String,
  field: String,
  property: String,
  value: Float,
) -> Change {
  Change(id, field, property, Number(value))
}

pub fn clear_change(id: String, field: String, property: String) -> Change {
  Change(id, field, property, Null)
}

/// The body of `POST /designs/{id}/edit`.
pub fn operations(changes: List(Change)) -> json.Json {
  json.object([#("operations", json.array(changes, operation_json))])
}

fn operation_json(change: Change) -> json.Json {
  case change {
    Change(id:, field:, property:, value:) ->
      json.object([
        #("op", json.string("set")),
        #("id", json.string(id)),
        #("field", json.string(field)),
        #("property", json.string(property)),
        #("value", value_json(value)),
      ])
    Move(id:, place:) ->
      json.object([
        #("op", json.string("move")),
        #("id", json.string(id)),
        #("parent", json.nullable(place.parent, json.string)),
        #("index", json.int(place.index)),
      ])
  }
}

fn value_json(value: Value) -> json.Json {
  case value {
    Text(text) -> json.string(text)
    Number(number) -> json.float(number)
    Integer(number) -> json.int(number)
    Fixed(number) -> json.object([#("fixed", json.float(number))])
    Null -> json.null()
  }
}

/// The saved design with the waiting changes applied.
pub fn apply(changes: List(Change), design: render.Design) -> render.Design {
  let moved =
    list.fold(changes, design, fn(design, change) {
      case change {
        Move(id:, place:) ->
          result.unwrap(moving.move(design, id, place), design)
        Change(..) -> design
      }
    })
  render.Design(
    ..moved,
    elements: list.map(moved.elements, fn(node) { apply_node(changes, node) }),
  )
}

fn apply_node(changes: List(Change), node: render.Node) -> render.Node {
  let mine = list.filter(changes, fn(change) { change.id == node.id })
  render.Node(
    ..node,
    kind: list.fold(mine, node.kind, apply_property),
    overrides: list.fold(mine, node.overrides, apply_style),
    children: list.map(node.children, fn(child) { apply_node(changes, child) }),
  )
}

fn apply_property(kind: render.Kind, change: Change) -> render.Kind {
  case change {
    Change(field: "properties", property:, value:, ..) ->
      case property {
        "width" -> set_size(kind, "width", size(value))
        "height" -> set_size(kind, "height", size(value))
        "gap" -> set_spacing(kind, "gap", spacing(value))
        "padding" -> set_spacing(kind, "padding", spacing(value))
        "align" -> set_align(kind, value)
        "justify" -> set_justify(kind, justify(value))
        "variant" -> set_variant(kind, variant(value))
        "text" -> set_text(kind, text(value))
        "label" -> set_label(kind, text(value))
        "description" -> set_description(kind, text(value))
        "placeholder" -> set_placeholder(kind, text(value))
        "level" -> set_level(kind, level(value))
        _ -> kind
      }
    _ -> kind
  }
}

fn apply_style(
  overrides: render.Overrides,
  change: Change,
) -> render.Overrides {
  case change {
    Change(field: "style", property:, value:, ..) ->
      case property {
        "fill" -> render.Overrides(..overrides, fill: color(value))
        "border" -> render.Overrides(..overrides, border: color(value))
        "text_color" -> render.Overrides(..overrides, text_color: color(value))
        "size" -> render.Overrides(..overrides, size: text_size(value))
        "padding" ->
          render.Overrides(..overrides, padding: style_spacing(value))
        _ -> overrides
      }
    _ -> overrides
  }
}

fn text(value: Value) -> Option(String) {
  case value {
    Text(text) -> Some(text)
    _ -> None
  }
}

fn size(value: Value) -> Option(render.Size) {
  case value {
    Text("fill") -> Some(render.Fill)
    Text("hug") -> Some(render.Hug)
    Number(number) if number >. 0.0 -> Some(render.Fixed(number))
    Fixed(number) if number >. 0.0 -> Some(render.Fixed(number))
    _ -> None
  }
}

fn spacing(value: Value) -> Option(render.SpacingValue) {
  case value {
    Text(name) if name != "" -> Some(render.SpacingToken(name))
    Number(number) if number >=. 0.0 -> Some(render.SpacingPixels(number))
    _ -> None
  }
}

fn style_spacing(value: Value) -> Option(render.SpacingValue) {
  case value {
    Null -> None
    _ -> spacing(value)
  }
}

fn text_size(value: Value) -> Option(render.TextSizeValue) {
  case value {
    Text(name) if name != "" -> Some(render.TextSizeToken(name))
    Number(number) if number >. 0.0 -> Some(render.TextSizePixels(number))
    _ -> None
  }
}

fn color(value: Value) -> Option(String) {
  case value {
    Text(name) ->
      case string.trim(name) {
        "" -> None
        _ -> Some(name)
      }
    _ -> None
  }
}

fn level(value: Value) -> Option(Int) {
  case value {
    Integer(number) -> Some(number)
    _ -> None
  }
}

fn align_text(value: Value) -> Option(render.TextAlign) {
  case value {
    Text("left") -> Some(render.TextLeft)
    Text("center") -> Some(render.TextCenter)
    Text("right") -> Some(render.TextRight)
    _ -> None
  }
}

fn align(value: Value) -> Option(render.Align) {
  case value {
    Text("start") -> Some(render.AlignStart)
    Text("center") -> Some(render.AlignCenter)
    Text("end") -> Some(render.AlignEnd)
    Text("stretch") -> Some(render.AlignStretch)
    _ -> None
  }
}

fn justify(value: Value) -> Option(render.Justify) {
  case value {
    Text("start") -> Some(render.JustifyStart)
    Text("center") -> Some(render.JustifyCenter)
    Text("end") -> Some(render.JustifyEnd)
    Text("between") -> Some(render.JustifyBetween)
    Text("around") -> Some(render.JustifyAround)
    _ -> None
  }
}

fn variant(value: Value) -> Option(render.ContainerVariant) {
  case value {
    Text("plain") -> Some(render.Plain)
    Text("card") -> Some(render.Card)
    _ -> None
  }
}

fn set_size(
  kind: render.Kind,
  property: String,
  value: Option(render.Size),
) -> render.Kind {
  case value {
    None -> kind
    Some(size) ->
      case kind, property {
        render.Screen(container), "width" ->
          render.Screen(render.Container(..container, width: size))
        render.Screen(container), _ ->
          render.Screen(render.Container(..container, height: size))
        render.Row(container), "width" ->
          render.Row(render.Container(..container, width: size))
        render.Row(container), _ ->
          render.Row(render.Container(..container, height: size))
        render.Column(container), "width" ->
          render.Column(render.Container(..container, width: size))
        render.Column(container), _ ->
          render.Column(render.Container(..container, height: size))
        render.Text(properties), "width" ->
          render.Text(render.TextProperties(..properties, width: size))
        render.Text(properties), _ ->
          render.Text(render.TextProperties(..properties, height: size))
        render.Heading(properties), "width" ->
          render.Heading(render.HeadingProperties(..properties, width: size))
        render.Heading(properties), _ ->
          render.Heading(render.HeadingProperties(..properties, height: size))
        render.Button(properties), "width" ->
          render.Button(render.ButtonProperties(..properties, width: size))
        render.Button(properties), _ ->
          render.Button(render.ButtonProperties(..properties, height: size))
        render.TextInput(properties), "width" ->
          render.TextInput(render.InputProperties(..properties, width: size))
        render.TextInput(properties), _ ->
          render.TextInput(render.InputProperties(..properties, height: size))
        render.ImagePlaceholder(properties), "width" ->
          render.ImagePlaceholder(
            render.ImageProperties(..properties, width: size),
          )
        render.ImagePlaceholder(properties), _ ->
          render.ImagePlaceholder(
            render.ImageProperties(..properties, height: size),
          )
        _, _ -> kind
      }
  }
}

fn set_spacing(
  kind: render.Kind,
  property: String,
  value: Option(render.SpacingValue),
) -> render.Kind {
  case value {
    None -> kind
    Some(spacing) ->
      case kind, property {
        render.Screen(container), "gap" ->
          render.Screen(render.Container(..container, gap: spacing))
        render.Screen(container), _ ->
          render.Screen(render.Container(..container, padding: spacing))
        render.Row(container), "gap" ->
          render.Row(render.Container(..container, gap: spacing))
        render.Row(container), _ ->
          render.Row(render.Container(..container, padding: spacing))
        render.Column(container), "gap" ->
          render.Column(render.Container(..container, gap: spacing))
        render.Column(container), _ ->
          render.Column(render.Container(..container, padding: spacing))
        _, _ -> kind
      }
  }
}

fn set_align(kind: render.Kind, value: Value) -> render.Kind {
  case kind {
    render.Text(properties) ->
      case align_text(value) {
        Some(align) ->
          render.Text(render.TextProperties(..properties, align: align))
        None -> kind
      }
    render.Heading(properties) ->
      case align_text(value) {
        Some(align) ->
          render.Heading(render.HeadingProperties(..properties, align: align))
        None -> kind
      }
    render.Screen(container) ->
      case align(value) {
        Some(align) ->
          render.Screen(render.Container(..container, align: align))
        None -> kind
      }
    render.Row(container) ->
      case align(value) {
        Some(align) -> render.Row(render.Container(..container, align: align))
        None -> kind
      }
    render.Column(container) ->
      case align(value) {
        Some(align) ->
          render.Column(render.Container(..container, align: align))
        None -> kind
      }
    _ -> kind
  }
}

fn set_justify(
  kind: render.Kind,
  value: Option(render.Justify),
) -> render.Kind {
  case value {
    None -> kind
    Some(justify) ->
      case kind {
        render.Screen(container) ->
          render.Screen(render.Container(..container, justify: justify))
        render.Row(container) ->
          render.Row(render.Container(..container, justify: justify))
        render.Column(container) ->
          render.Column(render.Container(..container, justify: justify))
        _ -> kind
      }
  }
}

fn set_variant(
  kind: render.Kind,
  value: Option(render.ContainerVariant),
) -> render.Kind {
  case value {
    None -> kind
    Some(variant) ->
      case kind {
        render.Screen(container) ->
          render.Screen(render.Container(..container, variant: variant))
        render.Row(container) ->
          render.Row(render.Container(..container, variant: variant))
        render.Column(container) ->
          render.Column(render.Container(..container, variant: variant))
        _ -> kind
      }
  }
}

fn set_text(kind: render.Kind, value: Option(String)) -> render.Kind {
  case value {
    Some(text) ->
      case kind {
        render.Text(properties) ->
          render.Text(render.TextProperties(..properties, content: text))
        render.Heading(properties) ->
          render.Heading(render.HeadingProperties(..properties, text: text))
        _ -> kind
      }
    None -> kind
  }
}

fn set_label(kind: render.Kind, value: Option(String)) -> render.Kind {
  case value {
    Some(label) ->
      case kind {
        render.Button(properties) ->
          render.Button(render.ButtonProperties(..properties, label: label))
        render.ImagePlaceholder(properties) ->
          render.ImagePlaceholder(
            render.ImageProperties(..properties, label: label),
          )
        render.DiagramNode(properties) ->
          render.DiagramNode(
            render.DiagramNodeProperties(..properties, label: label),
          )
        render.DiagramEdge(properties) ->
          render.DiagramEdge(
            render.DiagramEdgeProperties(..properties, label: label),
          )
        _ -> kind
      }
    None -> kind
  }
}

fn set_placeholder(kind: render.Kind, value: Option(String)) -> render.Kind {
  case value {
    Some(placeholder) ->
      case kind {
        render.TextInput(properties) ->
          render.TextInput(
            render.InputProperties(..properties, placeholder: placeholder),
          )
        _ -> kind
      }
    None -> kind
  }
}

fn set_level(kind: render.Kind, value: Option(Int)) -> render.Kind {
  case value {
    Some(level) ->
      case kind {
        render.Heading(properties) ->
          render.Heading(render.HeadingProperties(..properties, level: level))
        _ -> kind
      }
    None -> kind
  }
}

pub type Control {
  TextControl
  NumberControl
  Choice(List(#(String, String)))
  ColorControl
  TextSizeControl
  SpacingControl
}

pub type Property {
  Property(
    field: String,
    property: String,
    label: String,
    control: Control,
    value: String,
  )
}

/// The properties an element's type holds, then its style overrides.
pub fn properties(node: render.Node) -> List(Property) {
  case node.kind {
    render.DiagramNode(_) -> kind_properties(node.kind)
    render.DiagramEdge(_) -> kind_properties(node.kind)
    _ ->
      list.append(kind_properties(node.kind), style_properties(node.overrides))
  }
}

pub fn type_name(kind: render.Kind) -> String {
  case kind {
    render.Screen(_) -> "Screen"
    render.Row(_) -> "Row"
    render.Column(_) -> "Column"
    render.Text(_) -> "Text"
    render.Heading(_) -> "Heading"
    render.Button(_) -> "Button"
    render.TextInput(_) -> "Text input"
    render.ImagePlaceholder(_) -> "Image placeholder"
    render.Flow -> "Flow"
    render.DiagramNode(_) -> "Node"
    render.DiagramEdge(_) -> "Edge"
  }
}

pub fn node(design: render.Design, id: String) -> Option(render.Node) {
  find_node(design.elements, id)
}

fn find_node(nodes: List(render.Node), id: String) -> Option(render.Node) {
  case nodes {
    [] -> None
    [node, ..rest] ->
      case node.id == id {
        True -> Some(node)
        False ->
          case find_node(node.children, id) {
            Some(found) -> Some(found)
            None -> find_node(rest, id)
          }
      }
  }
}

fn kind_properties(kind: render.Kind) -> List(Property) {
  let #(width, height) = sizes(kind)
  let layout = [
    property("properties", "width", "Width", TextControl, size_text(width)),
    property("properties", "height", "Height", TextControl, size_text(height)),
  ]
  case kind {
    render.Screen(container) ->
      list.append(layout, container_properties(container, False))
    render.Row(container) ->
      list.append(layout, container_properties(container, True))
    render.Column(container) ->
      list.append(layout, container_properties(container, True))
    render.Text(properties) ->
      list.append(layout, [
        property("properties", "text", "Text", TextControl, properties.content),
        choice(
          "properties",
          "align",
          "Align",
          text_aligns,
          text_align_name(properties.align),
        ),
        choice(
          "properties",
          "variant",
          "Variant",
          text_variants,
          text_variant_name(properties.variant),
        ),
      ])
    render.Heading(properties) ->
      list.append(layout, [
        property("properties", "text", "Text", TextControl, properties.text),
        choice(
          "properties",
          "align",
          "Align",
          text_aligns,
          text_align_name(properties.align),
        ),
        property(
          "properties",
          "level",
          "Level",
          NumberControl,
          int.to_string(properties.level),
        ),
      ])
    render.Button(properties) ->
      list.append(layout, [
        property("properties", "label", "Label", TextControl, properties.label),
        choice(
          "properties",
          "variant",
          "Variant",
          button_variants,
          button_variant_name(properties.variant),
        ),
      ])
    render.TextInput(properties) ->
      list.append(layout, [
        property(
          "properties",
          "placeholder",
          "Placeholder",
          TextControl,
          properties.placeholder,
        ),
      ])
    render.ImagePlaceholder(properties) ->
      list.append(layout, [
        property("properties", "label", "Label", TextControl, properties.label),
      ])
    render.Flow -> layout
    render.DiagramNode(properties) -> [
      property("properties", "label", "Label", TextControl, properties.label),
      property(
        "properties",
        "description",
        "Description",
        TextControl,
        properties.description,
      ),
    ]
    render.DiagramEdge(properties) -> [
      property("properties", "label", "Label", TextControl, properties.label),
    ]
  }
}

fn container_properties(
  container: render.Container,
  variant: Bool,
) -> List(Property) {
  let common = [
    property(
      "properties",
      "gap",
      "Gap",
      SpacingControl,
      spacing_text(container.gap),
    ),
    property(
      "properties",
      "padding",
      "Padding",
      SpacingControl,
      spacing_text(container.padding),
    ),
    choice("properties", "align", "Align", aligns, align_name(container.align)),
    choice(
      "properties",
      "justify",
      "Justify",
      justifies,
      justify_name(container.justify),
    ),
  ]
  case variant {
    True ->
      list.append(common, [
        choice(
          "properties",
          "variant",
          "Variant",
          container_variants,
          container_variant_name(container.variant),
        ),
      ])
    False -> common
  }
}

fn style_properties(overrides: render.Overrides) -> List(Property) {
  [
    property("style", "fill", "Fill", ColorControl, optional(overrides.fill)),
    property(
      "style",
      "border",
      "Border",
      ColorControl,
      optional(overrides.border),
    ),
    property(
      "style",
      "text_color",
      "Text color",
      ColorControl,
      optional(overrides.text_color),
    ),
    property(
      "style",
      "size",
      "Text size",
      TextSizeControl,
      optional_text_size(overrides.size),
    ),
    property(
      "style",
      "padding",
      "Padding",
      SpacingControl,
      optional_spacing(overrides.padding),
    ),
  ]
}

fn property(
  field: String,
  name: String,
  label: String,
  control: Control,
  value: String,
) -> Property {
  Property(field, name, label, control, value)
}

fn choice(
  field: String,
  name: String,
  label: String,
  options: List(#(String, String)),
  value: String,
) -> Property {
  Property(field, name, label, Choice(options), value)
}

fn optional(value: Option(String)) -> String {
  case value {
    Some(text) -> text
    None -> ""
  }
}

fn optional_text_size(value: Option(render.TextSizeValue)) -> String {
  case value {
    Some(render.TextSizeToken(name)) -> name
    Some(render.TextSizePixels(number)) -> float.to_string(number)
    None -> ""
  }
}

fn optional_spacing(value: Option(render.SpacingValue)) -> String {
  case value {
    Some(value) -> spacing_text(value)
    None -> ""
  }
}

fn sizes(kind: render.Kind) -> #(render.Size, render.Size) {
  case kind {
    render.Screen(container) -> #(container.width, container.height)
    render.Row(container) -> #(container.width, container.height)
    render.Column(container) -> #(container.width, container.height)
    render.Text(properties) -> #(properties.width, properties.height)
    render.Heading(properties) -> #(properties.width, properties.height)
    render.Button(properties) -> #(properties.width, properties.height)
    render.TextInput(properties) -> #(properties.width, properties.height)
    render.ImagePlaceholder(properties) -> #(
      properties.width,
      properties.height,
    )
    render.Flow -> #(render.Fill, render.Fill)
    render.DiagramNode(_) -> #(render.Fill, render.Fill)
    render.DiagramEdge(_) -> #(render.Fill, render.Fill)
  }
}

fn size_text(size: render.Size) -> String {
  case size {
    render.Fill -> "fill"
    render.Hug -> "hug"
    render.Fixed(number) -> float.to_string(number)
  }
}

fn spacing_text(value: render.SpacingValue) -> String {
  case value {
    render.SpacingToken(name) -> name
    render.SpacingPixels(number) -> float.to_string(number)
  }
}

fn align_name(value: render.Align) -> String {
  case value {
    render.AlignStart -> "start"
    render.AlignCenter -> "center"
    render.AlignEnd -> "end"
    render.AlignStretch -> "stretch"
  }
}

fn justify_name(value: render.Justify) -> String {
  case value {
    render.JustifyStart -> "start"
    render.JustifyCenter -> "center"
    render.JustifyEnd -> "end"
    render.JustifyBetween -> "between"
    render.JustifyAround -> "around"
  }
}

fn text_align_name(value: render.TextAlign) -> String {
  case value {
    render.TextLeft -> "left"
    render.TextCenter -> "center"
    render.TextRight -> "right"
  }
}

fn text_variant_name(value: render.TextVariant) -> String {
  case value {
    render.Body -> "body"
    render.Muted -> "muted"
    render.Small -> "small"
  }
}

fn button_variant_name(value: render.ButtonVariant) -> String {
  case value {
    render.Primary -> "primary"
    render.Secondary -> "secondary"
    render.Ghost -> "ghost"
  }
}

fn container_variant_name(value: render.ContainerVariant) -> String {
  case value {
    render.Plain -> "plain"
    render.Card -> "card"
  }
}

const aligns = [
  #("start", "Start"),
  #("center", "Center"),
  #("end", "End"),
  #("stretch", "Stretch"),
]

const justifies = [
  #("start", "Start"),
  #("center", "Center"),
  #("end", "End"),
  #("between", "Between"),
  #("around", "Around"),
]

const text_aligns = [
  #("left", "Left"),
  #("center", "Center"),
  #("right", "Right"),
]

const text_variants = [
  #("body", "Body"),
  #("muted", "Muted"),
  #("small", "Small"),
]

const button_variants = [
  #("primary", "Primary"),
  #("secondary", "Secondary"),
  #("ghost", "Ghost"),
]

const container_variants = [
  #("plain", "Plain"),
  #("card", "Card"),
]

/// The browser key that keeps one design's waiting changes.
fn key(id: String) -> String {
  "solace-design-edits:" <> id
}

@external(javascript, "./api.mjs", "load")
fn load_key(key: String) -> String

@external(javascript, "./api.mjs", "store")
fn store_key(key: String, value: String) -> Nil

pub fn load(id: String) -> List(Change) {
  case json.parse(load_key(key(id)), decode.list(stored_decoder())) {
    Ok(changes) -> changes
    Error(_) -> []
  }
}

pub fn store(id: String, changes: List(Change)) -> Nil {
  case changes {
    [] -> store_key(key(id), "")
    _ -> store_key(key(id), json.to_string(json.array(changes, stored_json)))
  }
}

fn stored_json(change: Change) -> json.Json {
  case change {
    Change(id:, field:, property:, value:) ->
      json.object([
        #("id", json.string(id)),
        #("field", json.string(field)),
        #("property", json.string(property)),
        #("kind", json.string(value_kind(value))),
        #("value", json.string(value_text(value))),
      ])
    Move(..) -> operation_json(change)
  }
}

fn value_kind(value: Value) -> String {
  case value {
    Text(_) -> "text"
    Number(_) -> "number"
    Integer(_) -> "integer"
    Fixed(_) -> "fixed"
    Null -> "null"
  }
}

fn value_text(value: Value) -> String {
  case value {
    Text(text) -> text
    Number(number) -> float.to_string(number)
    Integer(number) -> int.to_string(number)
    Fixed(number) -> float.to_string(number)
    Null -> ""
  }
}

fn stored_decoder() -> decode.Decoder(Change) {
  use op <- decode.optional_field("op", "set", decode.string)
  case op {
    "move" -> stored_move_decoder()
    _ -> stored_set_decoder()
  }
}

pub fn integer_change(
  id: String,
  field: String,
  property: String,
  value: Int,
) -> Change {
  Change(id, field, property, Integer(value))
}

pub fn fixed_change(
  id: String,
  field: String,
  property: String,
  value: Float,
) -> Change {
  Change(id, field, property, Fixed(value))
}

fn stored_move_decoder() -> decode.Decoder(Change) {
  use id <- decode.field("id", decode.string)
  use parent <- decode.field("parent", decode.optional(decode.string))
  use index <- decode.field("index", decode.int)
  decode.success(Move(id, moving.Place(parent, index)))
}

fn stored_set_decoder() -> decode.Decoder(Change) {
  use id <- decode.field("id", decode.string)
  use field <- decode.field("field", decode.string)
  use property <- decode.field("property", decode.string)
  use kind <- decode.field("kind", decode.string)
  use raw <- decode.field("value", decode.string)
  let value = case kind {
    "number" ->
      case float.parse(raw) {
        Ok(number) -> Number(number)
        Error(_) -> Text(raw)
      }
    "integer" ->
      case int.parse(raw) {
        Ok(number) -> Integer(number)
        Error(_) -> Text(raw)
      }
    "fixed" ->
      case float.parse(raw) {
        Ok(number) -> Fixed(number)
        Error(_) -> Text(raw)
      }
    "null" -> Null
    _ -> Text(raw)
  }
  decode.success(Change(id, field, property, value))
}

/// The waiting changes with `change` added last: a property change replaces
/// the waiting one for the same property, and moves keep their order.
pub fn with_change(changes: List(Change), change: Change) -> List(Change) {
  let kept = case change {
    Change(id:, field:, property:, ..) ->
      list.filter(changes, fn(existing) {
        case existing {
          Change(id: other, field: other_field, property: other_property, ..) ->
            other != id || other_field != field || other_property != property
          Move(..) -> True
        }
      })
    Move(..) -> changes
  }
  list.append(kept, [change])
}

fn set_description(kind: render.Kind, value: Option(String)) -> render.Kind {
  case value {
    Some(description) ->
      case kind {
        render.DiagramNode(properties) ->
          render.DiagramNode(
            render.DiagramNodeProperties(..properties, description: description),
          )
        _ -> kind
      }
    None -> kind
  }
}
