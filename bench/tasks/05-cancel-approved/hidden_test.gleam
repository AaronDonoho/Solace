import pilot/checkout
import pilot/clock
import pilot/customer.{Customer, Domestic, International}
import pilot/order
import pilot/product.{Product}
import pilot/store

fn setup() {
  store.new()
  |> store.put_customer(Customer(1, "Ada", Domestic))
  |> store.put_customer(Customer(2, "Bo", International))
  |> store.put_product(Product("A", "Widget", 1000, 5))
  |> store.put_product(Product("B", "Gadget", 2500, 1))
}

fn draft(lines: List(#(String, Int))) {
  let c = clock.fixed(1)
  let assert Ok(#(db, o)) = checkout.create_order(setup(), c, 1)
  let assert Ok(db) = add_all(db, o.id, lines)
  #(db, o.id)
}

fn add_all(db, id, lines) {
  case lines {
    [] -> Ok(db)
    [#(sku, q), ..rest] ->
      case checkout.add_line(db, clock.fixed(1), id, sku, q) {
        Ok(#(db, _)) -> add_all(db, id, rest)
        Error(e) -> Error(e)
      }
  }
}

fn approved() {
  let #(db, id) = draft([#("A", 2), #("B", 1)])
  let assert Ok(#(db, _)) = checkout.submit(db, clock.fixed(2), id)
  let assert Ok(#(db, _)) = checkout.approve(db, clock.fixed(3), id)
  #(db, id)
}

fn stock(db, sku) {
  let assert Ok(p) = store.get_product(db, sku)
  p.stock
}

pub fn cancel_approved_restocks_test() {
  let #(db, id) = approved()
  assert stock(db, "A") == 3
  let assert Ok(#(db, o)) = checkout.cancel(db, clock.fixed(4), id, "ops")
  assert o.status == order.Cancelled("ops")
  assert stock(db, "A") == 5
  assert stock(db, "B") == 1
}

pub fn cancel_submitted_leaves_stock_test() {
  let #(db, id) = draft([#("A", 2)])
  let assert Ok(#(db, _)) = checkout.submit(db, clock.fixed(2), id)
  let assert Ok(#(db, _)) = checkout.cancel(db, clock.fixed(4), id, "x")
  assert stock(db, "A") == 5
}

pub fn cannot_cancel_shipped_test() {
  let #(db, id) = approved()
  let assert Ok(#(db, _)) = checkout.ship(db, clock.fixed(4), id)
  assert checkout.cancel(db, clock.fixed(5), id, "late")
    == Error(
      checkout.Rejected(order.InvalidTransition(
        from: order.Shipped,
        action: order.Cancel("late"),
      )),
    )
}
