import pilot/order.{
  Approve, Approved, Cancel, Cancelled, Draft, EmptyOrder, InvalidTransition,
  Line, Ship, Shipped, Submit, Submitted,
}

fn with_line() {
  order.Order(..order.new(1, 1, 0), lines: [Line("A", 1)])
}

pub fn happy_path_test() {
  let assert Ok(o) = order.transition(with_line(), Submit, 1)
  assert o.status == Submitted
  let assert Ok(o) = order.transition(o, Approve, 2)
  assert o.status == Approved
  let assert Ok(o) = order.transition(o, Ship, 3)
  assert o.status == Shipped
  assert o.updated_at == 3
}

pub fn empty_order_cannot_submit_test() {
  assert order.transition(order.new(1, 1, 0), Submit, 1) == Error(EmptyOrder)
}

pub fn cancel_from_draft_and_submitted_test() {
  let assert Ok(o) = order.transition(with_line(), Cancel("x"), 1)
  assert o.status == Cancelled("x")
  let assert Ok(o) = order.transition(with_line(), Submit, 1)
  let assert Ok(o) = order.transition(o, Cancel("y"), 2)
  assert o.status == Cancelled("y")
}

pub fn invalid_transitions_test() {
  let draft = with_line()
  assert order.transition(draft, Ship, 1)
    == Error(InvalidTransition(from: Draft, action: Ship))
  let shipped = order.Order(..draft, status: Shipped)
  assert order.transition(shipped, Cancel("late"), 1)
    == Error(InvalidTransition(from: Shipped, action: Cancel("late")))
}
