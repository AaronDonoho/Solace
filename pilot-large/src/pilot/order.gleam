//// Orders and their lifecycle.
////
////   submit:        Draft -> Submitted        (requires at least one line)
////   pay:           Submitted -> Paid
////   start_picking: Paid -> Picking
////   ship:          Picking -> Shipped
////   deliver:       Shipped -> Delivered
////   cancel:        Draft | Submitted | Paid -> Cancelled

import gleam/list
import gleam/option.{type Option, None, Some}
import pilot/customer.{type Address}

pub type Status {
  Draft
  Submitted
  Paid
  Picking
  Shipped
  Delivered
  Cancelled(reason: String)
}

/// `unit_price_cents` is the price when the line was added.
pub type Line {
  Line(sku: String, quantity: Int, unit_price_cents: Int)
}

pub type Order {
  Order(
    id: Int,
    customer_id: Int,
    lines: List(Line),
    status: Status,
    coupon_code: Option(String),
    shipping_address: Address,
    created_at: Int,
    updated_at: Int,
    paid_at: Option(Int),
    delivered_at: Option(Int),
  )
}

pub type Action {
  Submit
  Pay
  StartPicking
  Ship
  Deliver
  Cancel(reason: String)
}

pub type OrderError {
  EmptyOrder
  InvalidTransition(from: Status, action: Action)
}

pub fn new(id: Int, customer_id: Int, address: Address, now: Int) -> Order {
  Order(
    id:,
    customer_id:,
    lines: [],
    status: Draft,
    coupon_code: None,
    shipping_address: address,
    created_at: now,
    updated_at: now,
    paid_at: None,
    delivered_at: None,
  )
}

pub fn transition(
  order: Order,
  action: Action,
  now: Int,
) -> Result(Order, OrderError) {
  let to = fn(status) { Ok(Order(..order, status:, updated_at: now)) }
  case order.status, action {
    Draft, Submit ->
      case order.lines {
        [] -> Error(EmptyOrder)
        _ -> to(Submitted)
      }
    Submitted, Pay ->
      Ok(Order(..order, status: Paid, paid_at: Some(now), updated_at: now))
    Paid, StartPicking -> to(Picking)
    Picking, Ship -> to(Shipped)
    Shipped, Deliver ->
      Ok(
        Order(
          ..order,
          status: Delivered,
          delivered_at: Some(now),
          updated_at: now,
        ),
      )
    Draft, Cancel(reason) | Submitted, Cancel(reason) | Paid, Cancel(reason) ->
      to(Cancelled(reason))
    from, action -> Error(InvalidTransition(from:, action:))
  }
}

pub fn is_editable(order: Order) -> Bool {
  order.status == Draft
}

/// Adds units of a SKU, merging with an existing line (which keeps its price).
pub fn add_line(order: Order, line: Line) -> Order {
  let lines = case list.any(order.lines, fn(l) { l.sku == line.sku }) {
    True ->
      list.map(order.lines, fn(l) {
        case l.sku == line.sku {
          True -> Line(..l, quantity: l.quantity + line.quantity)
          False -> l
        }
      })
    False -> list.append(order.lines, [line])
  }
  Order(..order, lines:)
}

pub fn remove_line(order: Order, sku: String) -> Order {
  Order(..order, lines: list.filter(order.lines, fn(l) { l.sku != sku }))
}

/// Sets a line's quantity; zero removes the line.
pub fn set_quantity(order: Order, sku: String, quantity: Int) -> Order {
  case quantity {
    0 -> remove_line(order, sku)
    _ ->
      Order(
        ..order,
        lines: list.map(order.lines, fn(l) {
          case l.sku == sku {
            True -> Line(..l, quantity:)
            False -> l
          }
        }),
      )
  }
}

pub fn find_line(order: Order, sku: String) -> Result(Line, Nil) {
  list.find(order.lines, fn(l) { l.sku == sku })
}

pub fn subtotal_cents(lines: List(Line)) -> Int {
  list.fold(lines, 0, fn(sum, l) { sum + l.quantity * l.unit_price_cents })
}

pub fn total_quantity(order: Order) -> Int {
  list.fold(order.lines, 0, fn(sum, l) { sum + l.quantity })
}

pub fn status_name(status: Status) -> String {
  case status {
    Draft -> "draft"
    Submitted -> "submitted"
    Paid -> "paid"
    Picking -> "picking"
    Shipped -> "shipped"
    Delivered -> "delivered"
    Cancelled(_) -> "cancelled"
  }
}
