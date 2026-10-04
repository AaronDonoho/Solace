import gleam/option.{Some}
import pilot/customer.{Address, Domestic}
import pilot/order.{
  Cancel, Cancelled, Deliver, Delivered, Draft, EmptyOrder, InvalidTransition,
  Line, Paid, Pay, Picking, Ship, Shipped, StartPicking, Submit, Submitted,
}

fn new() {
  order.new(1, 1, Address("1 Main", "Albany", "12207", Domestic("NY")), 0)
}

fn with_line() {
  order.add_line(new(), Line("A", 1, 100))
}

pub fn lifecycle_test() {
  let assert Ok(o) = order.transition(with_line(), Submit, 1)
  assert o.status == Submitted
  let assert Ok(o) = order.transition(o, Pay, 2)
  assert o.status == Paid
  assert o.paid_at == Some(2)
  let assert Ok(o) = order.transition(o, StartPicking, 3)
  assert o.status == Picking
  let assert Ok(o) = order.transition(o, Ship, 4)
  assert o.status == Shipped
  let assert Ok(o) = order.transition(o, Deliver, 5)
  assert o.status == Delivered
  assert o.delivered_at == Some(5)
}

pub fn empty_order_cannot_submit_test() {
  assert order.transition(new(), Submit, 1) == Error(EmptyOrder)
}

pub fn cancel_before_picking_only_test() {
  let assert Ok(o) = order.transition(with_line(), Cancel("x"), 1)
  assert o.status == Cancelled("x")
  let picking = order.Order(..with_line(), status: Picking)
  assert order.transition(picking, Cancel("x"), 1)
    == Error(InvalidTransition(from: Picking, action: Cancel("x")))
}

pub fn invalid_transition_test() {
  assert order.transition(with_line(), Ship, 1)
    == Error(InvalidTransition(from: Draft, action: Ship))
}

pub fn lines_test() {
  let o =
    new()
    |> order.add_line(Line("A", 1, 100))
    |> order.add_line(Line("B", 2, 250))
    |> order.add_line(Line("A", 2, 999))
  assert o.lines == [Line("A", 3, 100), Line("B", 2, 250)]
  assert order.subtotal_cents(o.lines) == 800
  assert order.total_quantity(o) == 5
  assert { order.set_quantity(o, "B", 0) }.lines == [Line("A", 3, 100)]
  assert { order.set_quantity(o, "B", 7) }.lines
    == [Line("A", 3, 100), Line("B", 7, 250)]
  assert { order.remove_line(o, "A") }.lines == [Line("B", 2, 250)]
}
