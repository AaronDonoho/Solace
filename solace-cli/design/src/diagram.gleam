//// Draws architecture diagrams. Flow is the first diagram type; another type
//// adds its own branch here.

import contracts
import gleam/dict.{type Dict}
import gleam/dynamic/decode
import gleam/float
import gleam/int
import gleam/json
import gleam/list
import gleam/string
import lustre/attribute
import lustre/element.{type Element}
import lustre/element/html
import lustre/element/svg
import lustre/event
import render

pub const styles =
  ".design-flow{position:relative;margin:0;background:var(--paper);border:1px solid var(--line)}"
  <> ".design-flow-edges{position:absolute;inset:0;width:100%;height:100%;pointer-events:none}"
  <> ".design-node{position:absolute;box-sizing:border-box;width:220px;height:120px;padding:12px;overflow:hidden;background:var(--paper);border:1.5px solid var(--edge);font:13px/1.4 'IBM Plex Sans',system-ui,sans-serif;cursor:pointer}"
  <> ".design-node-label{font-weight:700;font-size:15px}"
  <> ".design-node-description{margin-top:4px;color:var(--soft)}"
  <> ".design-edge{fill:none;stroke:var(--edge);stroke-width:1.6;pointer-events:stroke;cursor:pointer}"
  <> ".design-edge-label{fill:var(--ink);font:12px 'IBM Plex Sans',system-ui,sans-serif;pointer-events:none}"
  <> ".design-edge-note{pointer-events:none}"
  <> ".design-edge-note rect{fill:#c2410c}"
  <> ".design-edge-note text{fill:#fff;font:11px 'IBM Plex Sans',system-ui,sans-serif;text-anchor:middle}"
  <> ".design-edge-hit{fill:none;stroke:transparent;stroke-width:12;pointer-events:stroke;cursor:pointer}"
  <> ".design-edge.design-change-added{stroke:#1a6b30}"
  <> ".design-edge.design-change-removed{stroke:#b3261e;stroke-dasharray:6 4;opacity:.6}"
  <> ".design-edge.design-change-changed,.design-edge.design-change-moved,.design-edge.design-change-both{stroke:#174f78}"
  <> ".design-edge.design-selected{stroke:#c2410c;stroke-width:3}"

const node_width = 220.0

const node_height = 120.0

const margin = 24.0

pub fn draw(
  design: render.Design,
  marks: Dict(String, render.Change),
  selection: Selection(message),
) -> Element(message) {
  case design.elements {
    [flow, ..] ->
      case flow.kind {
        render.Flow -> flow_diagram(flow, marks, selection)
        _ -> element.none()
      }
    [] -> element.none()
  }
}

fn flow_diagram(
  flow: render.Node,
  marks: Dict(String, render.Change),
  selection: Selection(message),
) -> Element(message) {
  let placed = positions(flow.children)
  let #(width, height) = bounds(flow.children, placed)
  html.div(
    [
      attribute.class(element_class("design-flow", flow.id, marks, selection)),
      attribute.data("design-id", flow.id),
      attribute.data("region", ""),
      attribute.styles([#("width", px(width)), #("height", px(height))]),
      event.on_click(selection.on_clear),
    ],
    [
      edges(flow.children, placed, marks, width, height, selection),
      ..list.filter_map(flow.children, fn(child) {
        case child.kind {
          render.DiagramNode(properties) ->
            Ok(node_box(child.id, properties, placed, marks, selection))
          _ -> Error(Nil)
        }
      })
    ],
  )
}

fn node_box(
  id: String,
  properties: render.DiagramNodeProperties,
  placed: Dict(String, contracts.Point),
  marks: Dict(String, render.Change),
  selection: Selection(message),
) -> Element(message) {
  case dict.get(placed, id) {
    Error(_) -> element.none()
    Ok(point) -> {
      let point = box_point(point)
      let note = case dict.get(selection.markers, id) {
        Ok(number) -> [attribute.data("note", number)]
        Error(_) -> []
      }
      let clicks = [
        event.stop_propagation(
          event.prevent_default(event.on(
            "click",
            render.click_decoder(id, selection.on_click),
          )),
        ),
      ]
      html.div(
        [
          attribute.class(element_class(
            "design-element design-node",
            id,
            marks,
            selection,
          )),
          attribute.data("design-id", id),
          attribute.styles([
            #("left", px(point.x)),
            #("top", px(point.y)),
          ]),
          ..list.append(note, clicks)
        ],
        [
          html.div([attribute.class("design-node-label")], [
            html.text(properties.label),
          ]),
          case properties.description {
            "" -> element.none()
            description ->
              html.div([attribute.class("design-node-description")], [
                html.text(description),
              ])
          },
        ],
      )
    }
  }
}

fn edges(
  children: List(render.Node),
  placed: Dict(String, contracts.Point),
  marks: Dict(String, render.Change),
  width: Float,
  height: Float,
  selection: Selection(message),
) -> Element(message) {
  svg.svg(
    [
      attribute.class("design-flow-edges"),
      attribute.attribute("width", px(width)),
      attribute.attribute("height", px(height)),
      attribute.attribute(
        "viewBox",
        "0 0 " <> float.to_string(width) <> " " <> float.to_string(height),
      ),
    ],
    [
      svg.defs([], [arrow()]),
      ..list.filter_map(children, fn(child) {
        case child.kind {
          render.DiagramEdge(edge) ->
            Ok(edge_line(child.id, edge, placed, marks, selection))
          _ -> Error(Nil)
        }
      })
    ],
  )
}

fn arrow() -> Element(message) {
  svg.marker(
    [
      attribute.attribute("id", "design-flow-arrow"),
      attribute.attribute("viewBox", "0 0 10 10"),
      attribute.attribute("refX", "9"),
      attribute.attribute("refY", "5"),
      attribute.attribute("markerWidth", "7"),
      attribute.attribute("markerHeight", "7"),
      attribute.attribute("orient", "auto"),
    ],
    [
      svg.path([
        attribute.attribute("d", "M 1 1 L 9 5 L 1 9"),
        attribute.attribute("fill", "context-stroke"),
      ]),
    ],
  )
}

fn edge_line(
  id: String,
  edge: render.DiagramEdgeProperties,
  placed: Dict(String, contracts.Point),
  marks: Dict(String, render.Change),
  selection: Selection(message),
) -> Element(message) {
  case dict.get(placed, edge.from), dict.get(placed, edge.to) {
    Ok(from), Ok(to) -> {
      let from = box_point(from)
      let to = box_point(to)
      let route = contracts.connector(from, to)
      let marker = dict.get(selection.markers, id)
      let note = case marker {
        Ok(number) -> [attribute.data("note", number)]
        Error(_) -> []
      }
      let clicks = [
        event.stop_propagation(
          event.prevent_default(event.on(
            "click",
            render.click_decoder(id, selection.on_click),
          )),
        ),
      ]
      svg.g([], [
        svg.path(list.append(
          [
            attribute.class(element_class("design-edge", id, marks, selection)),
            attribute.data("design-id", id),
            attribute.attribute("d", route),
            attribute.attribute("marker-end", "url(#design-flow-arrow)"),
          ],
          list.append(note, clicks),
        )),
        case edge.label {
          "" -> element.none()
          label ->
            svg.text(
              [
                attribute.class("design-edge-label"),
                attribute.attribute("x", float.to_string(mid_x(from, to))),
                attribute.attribute(
                  "y",
                  float.to_string(mid_y(from, to) -. 6.0),
                ),
                attribute.attribute("text-anchor", "middle"),
              ],
              label,
            )
        },
        case marker {
          Ok(number) -> note_badge(number, from, to)
          Error(_) -> element.none()
        },
        svg.path([
          attribute.class("design-edge-hit"),
          attribute.attribute("d", route),
          ..clicks
        ]),
      ])
    }
    _, _ -> element.none()
  }
}

fn positions(children: List(render.Node)) -> Dict(String, contracts.Point) {
  let nodes =
    list.filter_map(children, fn(child) {
      case child.kind {
        render.DiagramNode(_) ->
          Ok(json.object([#("id", json.string(child.id))]))
        _ -> Error(Nil)
      }
    })
  let edges =
    list.filter_map(children, fn(child) {
      case child.kind {
        render.DiagramEdge(edge) ->
          Ok(
            json.object([
              #("from", json.string(edge.from)),
              #("to", json.string(edge.to)),
            ]),
          )
        _ -> Error(Nil)
      }
    })
  let placed =
    contracts.flow_positions(
      json.to_string(json.array(nodes, fn(value) { value })),
      json.to_string(json.array(edges, fn(value) { value })),
    )
  case json.parse(placed, decode.list(point_decoder())) {
    Ok(points) ->
      dict.from_list(list.map(points, fn(point) { #(point.id, point) }))
    Error(_) -> dict.new()
  }
}

fn point_decoder() -> decode.Decoder(contracts.Point) {
  use id <- decode.field("id", decode.string)
  use x <- decode.field("x", decode.float)
  use y <- decode.field("y", decode.float)
  decode.success(contracts.Point(id, x, y))
}

fn bounds(
  children: List(render.Node),
  placed: Dict(String, contracts.Point),
) -> #(Float, Float) {
  let #(right, bottom) =
    list.fold(children, #(0.0, 0.0), fn(acc, child) {
      case dict.get(placed, child.id) {
        Error(_) -> acc
        Ok(point) -> #(
          case point.x >. acc.0 {
            True -> point.x
            False -> acc.0
          },
          case point.y >. acc.1 {
            True -> point.y
            False -> acc.1
          },
        )
      }
    })
  #(
    right +. node_width +. margin *. 2.0,
    bottom +. node_height +. margin *. 2.0,
  )
}

fn mid_x(from: contracts.Point, to: contracts.Point) -> Float {
  { from.x +. to.x } /. 2.0 +. node_width /. 2.0
}

fn mid_y(from: contracts.Point, to: contracts.Point) -> Float {
  { from.y +. to.y } /. 2.0 +. node_height /. 2.0
}

fn element_class(
  base: String,
  id: String,
  marks: Dict(String, render.Change),
  selection: Selection(message),
) -> String {
  let change = case dict.get(marks, id) {
    Ok(change) -> " " <> render.change_class(change)
    Error(_) -> ""
  }
  base
  <> change
  <> render.selected_class(id, selection.selected)
  <> render.marker_class(id, selection.markers)
}

fn px(value: Float) -> String {
  float.to_string(value) <> "px"
}

fn box_point(point: contracts.Point) -> contracts.Point {
  contracts.Point(point.id, point.x +. margin, point.y +. margin)
}

/// How a diagram shows a selection and reports a click or a background click.
pub type Selection(message) {
  Selection(
    selected: List(String),
    markers: Dict(String, String),
    on_click: fn(String, Bool) -> message,
    on_clear: message,
  )
}

fn note_badge(
  number: String,
  from: contracts.Point,
  to: contracts.Point,
) -> Element(message) {
  let x = mid_x(from, to)
  let y = mid_y(from, to) -. 32.0
  let width = 18.0 +. int.to_float(string.length(number) - 1) *. 6.0
  svg.g([attribute.class("design-edge-note")], [
    svg.rect([
      attribute.attribute("x", float.to_string(x -. width /. 2.0)),
      attribute.attribute("y", float.to_string(y -. 9.0)),
      attribute.attribute("width", float.to_string(width)),
      attribute.attribute("height", "18"),
      attribute.attribute("rx", "9"),
    ]),
    svg.text(
      [
        attribute.attribute("x", float.to_string(x)),
        attribute.attribute("y", float.to_string(y +. 4.0)),
        attribute.attribute("text-anchor", "middle"),
      ],
      number,
    ),
  ])
}
