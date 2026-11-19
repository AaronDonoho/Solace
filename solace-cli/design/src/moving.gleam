//// Moves an element within a UI design's element tree: the places the design
//// rules allow, and where a dragged element lands under the pointer.

import gleam/dict.{type Dict}
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import region
import render

/// Where a moved element goes: its parent, None for the top level, and its
/// index among that parent's children once it has been taken out.
pub type Place {
  Place(parent: Option(String), index: Int)
}

/// The design with `id` moved to `place`, or Error when the design rules
/// refuse it: a screen below the top level, anything else at the top level,
/// a parent that can't hold children or lies inside the element, or an index
/// outside the parent's children.
pub fn move(
  design: render.Design,
  id: String,
  place: Place,
) -> Result(render.Design, Nil) {
  use #(node, rest) <- result.try(take(design.elements, id))
  use elements <- result.try(insert(rest, node, place))
  case valid_structure(elements, None) {
    True -> Ok(render.Design(..design, elements: elements))
    False -> Error(Nil)
  }
}

/// Where `id` sits now.
pub fn place_of(design: render.Design, id: String) -> Option(Place) {
  find_place(design.elements, None, id)
}

/// Where `id` lands with the pointer at `x`, `y`, given the drawn elements'
/// bounds: the deepest place under the pointer the rules allow, or None.
pub fn drop_place(
  design: render.Design,
  id: String,
  x: Float,
  y: Float,
  bounds: List(#(String, region.Rect)),
) -> Option(Place) {
  case take(design.elements, id) {
    Error(_) -> None
    Ok(#(_, rest)) -> {
      let rects = dict.from_list(bounds)
      let candidates =
        list.flat_map(hovered(rest, None, Vertical, x, y, rects), fn(entry) {
          let #(parent, along, node, index) = entry
          [
            into(node, x, y, rects),
            Some(beside(parent, along, node, index, x, y, rects)),
          ]
        })
      list.append(candidates, [
        Some(Place(None, index_among(rest, Vertical, x, y, rects))),
      ])
      |> list.filter_map(option.to_result(_, Nil))
      |> list.find(fn(place) { result.is_ok(move(design, id, place)) })
      |> option.from_result
    }
  }
}

type Axis {
  Vertical
  Horizontal
}

fn take(
  nodes: List(render.Node),
  id: String,
) -> Result(#(render.Node, List(render.Node)), Nil) {
  case nodes {
    [] -> Error(Nil)
    [node, ..rest] if node.id == id -> Ok(#(node, rest))
    [node, ..rest] ->
      case take(node.children, id) {
        Ok(#(found, children)) ->
          Ok(#(found, [render.Node(..node, children: children), ..rest]))
        Error(_) ->
          result.map(take(rest, id), fn(pair) { #(pair.0, [node, ..pair.1]) })
      }
  }
}

fn insert(
  nodes: List(render.Node),
  node: render.Node,
  place: Place,
) -> Result(List(render.Node), Nil) {
  case place.parent, is_screen(node.kind), is_tab(node.kind) {
    None, False, True -> Error(Nil)
    None, True, False -> insert_at(nodes, node, place.index)
    Some(parent), False, _ -> insert_into(nodes, parent, node, place.index)
    _, _, _ -> Error(Nil)
  }
}

fn insert_into(
  nodes: List(render.Node),
  parent: String,
  node: render.Node,
  index: Int,
) -> Result(List(render.Node), Nil) {
  case nodes {
    [] -> Error(Nil)
    [first, ..rest] if first.id == parent ->
      case holds_children(first.kind) && allows_child(first.kind, node.kind) {
        True ->
          case first.kind, list.length(first.children) {
            render.SplitPane(_), 2 -> Error(Nil)
            _, _ ->
              result.map(insert_at(first.children, node, index), fn(children) {
                [render.Node(..first, children: children), ..rest]
              })
          }
        False -> Error(Nil)
      }
    [first, ..rest] ->
      case insert_into(first.children, parent, node, index) {
        Ok(children) -> Ok([render.Node(..first, children: children), ..rest])
        Error(_) ->
          result.map(insert_into(rest, parent, node, index), fn(rest) {
            [first, ..rest]
          })
      }
  }
}

fn insert_at(
  nodes: List(render.Node),
  node: render.Node,
  index: Int,
) -> Result(List(render.Node), Nil) {
  case index >= 0 && index <= list.length(nodes) {
    True ->
      Ok(
        list.flatten([list.take(nodes, index), [node], list.drop(nodes, index)]),
      )
    False -> Error(Nil)
  }
}

fn find_place(
  nodes: List(render.Node),
  parent: Option(String),
  id: String,
) -> Option(Place) {
  list.index_fold(nodes, None, fn(found, node, index) {
    case found, node.id == id {
      Some(_), _ -> found
      None, True -> Some(Place(parent, index))
      None, False -> find_place(node.children, Some(node.id), id)
    }
  })
}

fn is_screen(kind: render.Kind) -> Bool {
  case kind {
    render.Screen(_) -> True
    _ -> False
  }
}

fn holds_children(kind: render.Kind) -> Bool {
  case kind {
    render.Screen(_)
    | render.Row(_)
    | render.Column(_)
    | render.Tabs(_)
    | render.Tab(_)
    | render.ScrollArea(_)
    | render.SplitPane(_) -> True
    _ -> False
  }
}

fn axis(kind: render.Kind) -> Axis {
  case kind {
    render.Row(_) -> Horizontal
    render.SplitPane(properties) ->
      case properties.direction {
        render.SideBySide -> Horizontal
        render.Stacked -> Vertical
      }
    _ -> Vertical
  }
}

/// The elements under the pointer with their parent, the parent's axis and
/// their index, deepest first.
fn hovered(
  nodes: List(render.Node),
  parent: Option(String),
  along: Axis,
  x: Float,
  y: Float,
  rects: Dict(String, region.Rect),
) -> List(#(Option(String), Axis, render.Node, Int)) {
  let under =
    list.index_map(nodes, fn(node, index) { #(node, index) })
    |> list.find(fn(entry) {
      case dict.get(rects, { entry.0 }.id) {
        Ok(rect) -> contains(rect, x, y)
        Error(_) -> False
      }
    })
  case under {
    Ok(#(node, index)) ->
      list.append(
        hovered(node.children, Some(node.id), axis(node.kind), x, y, rects),
        [#(parent, along, node, index)],
      )
    Error(_) -> []
  }
}

fn into(
  node: render.Node,
  x: Float,
  y: Float,
  rects: Dict(String, region.Rect),
) -> Option(Place) {
  case holds_children(node.kind) {
    True ->
      Some(Place(
        Some(node.id),
        index_among(node.children, axis(node.kind), x, y, rects),
      ))
    False -> None
  }
}

/// Before or after `node` in its parent, by which half the pointer is in.
fn beside(
  parent: Option(String),
  along: Axis,
  node: render.Node,
  index: Int,
  x: Float,
  y: Float,
  rects: Dict(String, region.Rect),
) -> Place {
  case dict.get(rects, node.id) {
    Ok(rect) ->
      case past_middle(rect, along, x, y) {
        True -> Place(parent, index + 1)
        False -> Place(parent, index)
      }
    Error(_) -> Place(parent, index)
  }
}

/// How many of `nodes` the pointer is past, along `along`.
fn index_among(
  nodes: List(render.Node),
  along: Axis,
  x: Float,
  y: Float,
  rects: Dict(String, region.Rect),
) -> Int {
  list.count(nodes, fn(node) {
    case dict.get(rects, node.id) {
      Ok(rect) -> past_middle(rect, along, x, y)
      Error(_) -> False
    }
  })
}

fn past_middle(rect: region.Rect, along: Axis, x: Float, y: Float) -> Bool {
  case along {
    Horizontal -> x >. { rect.left +. rect.right } /. 2.0
    Vertical -> y >. { rect.top +. rect.bottom } /. 2.0
  }
}

fn contains(rect: region.Rect, x: Float, y: Float) -> Bool {
  x >=. rect.left && x <=. rect.right && y >=. rect.top && y <=. rect.bottom
}

fn is_tab(kind: render.Kind) -> Bool {
  case kind {
    render.Tab(_) -> True
    _ -> False
  }
}

fn allows_child(parent: render.Kind, child: render.Kind) -> Bool {
  case parent {
    render.Tabs(_) -> is_tab(child)
    _ -> !is_tab(child)
  }
}

fn valid_structure(
  nodes: List(render.Node),
  parent: Option(render.Kind),
) -> Bool {
  list.fold(nodes, True, fn(valid, node) {
    valid
    && case node.kind {
      render.Tab(_) ->
        case parent {
          Some(render.Tabs(_)) ->
            valid_structure(node.children, Some(node.kind))
          _ -> False
        }
      render.Tabs(properties) ->
        case node.children {
          [] -> False
          _ ->
            list.any(node.children, fn(child) { is_tab(child.kind) })
            && list.any(node.children, fn(child) {
              child.id == properties.current
            })
            && valid_structure(node.children, Some(node.kind))
        }
      render.SplitPane(_) ->
        list.length(node.children) == 2
        && valid_structure(node.children, Some(node.kind))
      _ -> valid_structure(node.children, Some(node.kind))
    }
  })
}
