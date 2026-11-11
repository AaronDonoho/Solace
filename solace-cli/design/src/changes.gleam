//// Compares a proposed design with the saved one, by element id, and marks
//// what changes. Removed elements stay in the drawing at their old place.

import gleam/dict.{type Dict}
import gleam/list
import gleam/option.{type Option, None, Some}
import render

/// A design ready to draw: the proposed elements, with removed ones kept, and
/// the change marked on each element id.
pub type Marked {
  Marked(design: render.Design, marks: Dict(String, render.Change))
}

/// Mark what `proposed` changes from `current`.
pub fn mark(current: render.Design, proposed: render.Design) -> Marked {
  let proposed_ids = ids(proposed.elements)
  let current_nodes = nodes(current.elements)
  let proposed_nodes = nodes(proposed.elements)
  let marks =
    change_marks(current, proposed, proposed_ids, current_nodes, proposed_nodes)
  let elements =
    merge_children(
      proposed.elements,
      current.elements,
      current_nodes,
      proposed_ids,
    )
  Marked(
    render.Design(proposed.id, proposed.title, elements, proposed.links),
    marks,
  )
}

/// A design a proposal creates: every element is added.
pub fn all_added(design: render.Design) -> Marked {
  let empty = render.Design(design.id, design.title, [], dict.new())
  mark(empty, design)
}

fn change_marks(
  current: render.Design,
  proposed: render.Design,
  proposed_ids: Dict(String, Nil),
  current_nodes: Dict(String, render.Node),
  proposed_nodes: Dict(String, render.Node),
) -> Dict(String, render.Change) {
  let proposed_places = places(proposed.elements)
  let current_places = places(current.elements)
  let proposed_flows = flows(proposed.elements)
  let current_flows = flows(current.elements)
  let common =
    list.fold(dict.keys(proposed_nodes), dict.new(), fn(acc, id) {
      case dict.get(current_nodes, id) {
        Error(_) -> dict.insert(acc, id, render.Added)
        Ok(old) ->
          case dict.get(proposed_nodes, id) {
            Error(_) -> acc
            Ok(new) ->
              case
                change_of(
                  old,
                  new,
                  id,
                  current,
                  proposed,
                  current_places,
                  proposed_places,
                  current_flows,
                  proposed_flows,
                )
              {
                Some(change) -> dict.insert(acc, id, change)
                None -> acc
              }
          }
      }
    })
  list.fold(dict.keys(current_nodes), common, fn(acc, id) {
    case member(proposed_ids, id) {
      True -> acc
      False -> dict.insert(acc, id, render.Removed)
    }
  })
}

fn change_of(
  old: render.Node,
  new: render.Node,
  id: String,
  current: render.Design,
  proposed: render.Design,
  current_places: Dict(String, #(Option(String), List(String))),
  proposed_places: Dict(String, #(Option(String), List(String))),
  current_flows: Dict(String, Nil),
  proposed_flows: Dict(String, Nil),
) -> Option(render.Change) {
  let changed =
    old.kind != new.kind
    || old.overrides != new.overrides
    || dict.get(current.links, id) != dict.get(proposed.links, id)
  let moved =
    moved(id, current_places, proposed_places, current_flows, proposed_flows)
  case changed, moved {
    True, True -> Some(render.ChangedAndMoved)
    True, False -> Some(render.Changed)
    False, True -> Some(render.Moved)
    False, False -> None
  }
}

fn moved(
  id: String,
  current_places: Dict(String, #(Option(String), List(String))),
  proposed_places: Dict(String, #(Option(String), List(String))),
  current_flows: Dict(String, Nil),
  proposed_flows: Dict(String, Nil),
) -> Bool {
  case dict.get(current_places, id), dict.get(proposed_places, id) {
    Ok(#(current_parent, current_siblings)),
      Ok(#(proposed_parent, proposed_siblings))
    ->
      current_parent != proposed_parent
      || {
        let ordered = case current_parent {
          Some(parent) ->
            !member(current_flows, parent) && !member(proposed_flows, parent)
          None -> True
        }
        ordered
        && relative(id, current_siblings, proposed_siblings)
        != relative(id, proposed_siblings, current_siblings)
      }
    _, _ -> False
  }
}

fn relative(id: String, siblings: List(String), others: List(String)) -> Int {
  let other_ids =
    list.fold(others, dict.new(), fn(acc, other) {
      dict.insert(acc, other, Nil)
    })
  let common = list.filter(siblings, fn(sibling) { member(other_ids, sibling) })
  position(common, id)
}

fn position(items: List(String), id: String) -> Int {
  case items {
    [] -> -1
    [first, ..rest] ->
      case first == id {
        True -> 0
        False ->
          case position(rest, id) {
            -1 -> -1
            index -> index + 1
          }
      }
  }
}

fn places(
  elements: List(render.Node),
) -> Dict(String, #(Option(String), List(String))) {
  let siblings = list.map(elements, fn(node) { node.id })
  place_children(elements, None, siblings, dict.new())
}

fn place_children(
  elements: List(render.Node),
  parent: Option(String),
  siblings: List(String),
  acc: Dict(String, #(Option(String), List(String))),
) -> Dict(String, #(Option(String), List(String))) {
  list.fold(elements, acc, fn(acc, node) {
    let acc = dict.insert(acc, node.id, #(parent, siblings))
    let children = list.map(node.children, fn(child) { child.id })
    place_children(node.children, Some(node.id), children, acc)
  })
}

fn merge(
  node: render.Node,
  current_nodes: Dict(String, render.Node),
  proposed_ids: Dict(String, Nil),
) -> render.Node {
  let current_children = case dict.get(current_nodes, node.id) {
    Ok(current) -> current.children
    Error(_) -> []
  }
  render.Node(
    node.id,
    node.kind,
    merge_children(node.children, current_children, current_nodes, proposed_ids),
    node.overrides,
  )
}

fn merge_children(
  proposed_children: List(render.Node),
  current_children: List(render.Node),
  current_nodes: Dict(String, render.Node),
  proposed_ids: Dict(String, Nil),
) -> List(render.Node) {
  let proposed_here =
    list.fold(proposed_children, dict.new(), fn(acc, node) {
      dict.insert(acc, node.id, Nil)
    })
  let groups = removed_groups(current_children, proposed_here, proposed_ids)
  let ordered =
    list.fold(proposed_children, [], fn(acc, node) {
      let before =
        list.map(group(groups.before, node.id), fn(node) {
          without_proposed(node, proposed_ids)
        })
      let after =
        list.map(group(groups.after, node.id), fn(node) {
          without_proposed(node, proposed_ids)
        })
      let merged = merge(node, current_nodes, proposed_ids)
      list.append(acc, list.append(before, [merged, ..after]))
    })
  let trailing =
    list.map(groups.trailing, fn(node) { without_proposed(node, proposed_ids) })
  list.append(ordered, trailing)
}

type Groups {
  Groups(
    before: Dict(String, List(render.Node)),
    after: Dict(String, List(render.Node)),
    trailing: List(render.Node),
  )
}

fn removed_groups(
  current_children: List(render.Node),
  proposed_here: Dict(String, Nil),
  proposed_ids: Dict(String, Nil),
) -> Groups {
  let #(before, after, leading, _) =
    list.fold(
      current_children,
      #(dict.new(), dict.new(), [], None),
      fn(state, node) {
        let #(before, after, leading, previous) = state
        case member(proposed_here, node.id) {
          True -> {
            let before = case leading {
              [] -> before
              _ -> dict.insert(before, node.id, leading)
            }
            #(before, after, [], Some(node.id))
          }
          False ->
            case member(proposed_ids, node.id) {
              True -> #(before, after, leading, previous)
              False ->
                case previous {
                  Some(id) -> #(
                    before,
                    add(after, id, [node]),
                    leading,
                    previous,
                  )
                  None -> #(
                    before,
                    after,
                    list.append(leading, [node]),
                    previous,
                  )
                }
            }
        }
      },
    )
  Groups(before, after, leading)
}

fn add(
  map: Dict(String, List(render.Node)),
  key: String,
  nodes: List(render.Node),
) -> Dict(String, List(render.Node)) {
  dict.insert(map, key, list.append(group(map, key), nodes))
}

fn group(
  map: Dict(String, List(render.Node)),
  key: String,
) -> List(render.Node) {
  case dict.get(map, key) {
    Ok(nodes) -> nodes
    Error(_) -> []
  }
}

fn ids(elements: List(render.Node)) -> Dict(String, Nil) {
  list.fold(elements, dict.new(), ids_of)
}

fn ids_of(seen: Dict(String, Nil), node: render.Node) -> Dict(String, Nil) {
  let seen = dict.insert(seen, node.id, Nil)
  list.fold(node.children, seen, ids_of)
}

fn nodes(elements: List(render.Node)) -> Dict(String, render.Node) {
  list.fold(elements, dict.new(), nodes_of)
}

fn nodes_of(
  all: Dict(String, render.Node),
  node: render.Node,
) -> Dict(String, render.Node) {
  let all = dict.insert(all, node.id, node)
  list.fold(node.children, all, nodes_of)
}

fn member(ids: Dict(String, Nil), id: String) -> Bool {
  case dict.get(ids, id) {
    Ok(_) -> True
    Error(_) -> False
  }
}

/// A child the proposal moved elsewhere must not stay inside its removed parent.
fn without_proposed(
  node: render.Node,
  proposed_ids: Dict(String, Nil),
) -> render.Node {
  let kept =
    list.filter(node.children, fn(child) { !member(proposed_ids, child.id) })
  render.Node(
    node.id,
    node.kind,
    list.map(kept, fn(child) { without_proposed(child, proposed_ids) }),
    node.overrides,
  )
}

fn flows(elements: List(render.Node)) -> Dict(String, Nil) {
  list.fold(elements, dict.new(), flows_of)
}

fn flows_of(acc: Dict(String, Nil), node: render.Node) -> Dict(String, Nil) {
  let acc = case node.kind {
    render.Flow -> dict.insert(acc, node.id, Nil)
    _ -> acc
  }
  list.fold(node.children, acc, flows_of)
}
