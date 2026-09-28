//// The order lifecycle as a pure state machine.
////
////   submit:  Draft -> Submitted        (requires at least one line)
////   approve: Submitted -> Approved
////   ship:    Approved -> Shipped
////   cancel:  Draft | Submitted -> Cancelled

pub type Status {
  Draft
  Submitted
  Approved
  Shipped
  Cancelled(reason: String)
}

pub type Line {
  Line(sku: String, quantity: Int)
}

pub type Order {
  Order(
    id: Int,
    customer_id: Int,
    lines: List(Line),
    status: Status,
    created_at: Int,
    updated_at: Int,
  )
}

pub type Action {
  Submit
  Approve
  Ship
  Cancel(reason: String)
}

pub type OrderError {
  EmptyOrder
  InvalidTransition(from: Status, action: Action)
}

pub fn new(id: Int, customer_id: Int, now: Int) -> Order {
  Order(
    id:,
    customer_id:,
    lines: [],
    status: Draft,
    created_at: now,
    updated_at: now,
  )
}

pub fn transition(
  order: Order,
  action: Action,
  now: Int,
) -> Result(Order, OrderError) {
  case order.status, action {
    Draft, Submit ->
      case order.lines {
        [] -> Error(EmptyOrder)
        _ -> Ok(Order(..order, status: Submitted, updated_at: now))
      }
    Submitted, Approve -> Ok(Order(..order, status: Approved, updated_at: now))
    Approved, Ship -> Ok(Order(..order, status: Shipped, updated_at: now))
    Draft, Cancel(reason) | Submitted, Cancel(reason) ->
      Ok(Order(..order, status: Cancelled(reason), updated_at: now))
    from, action -> Error(InvalidTransition(from:, action:))
  }
}

pub fn is_editable(order: Order) -> Bool {
  order.status == Draft
}
