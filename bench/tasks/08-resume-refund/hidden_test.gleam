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

fn shipped() {
  let #(db, id) = draft([#("A", 2)])
  let assert Ok(#(db, _)) = checkout.submit(db, clock.fixed(2), id)
  let assert Ok(#(db, _)) = checkout.approve(db, clock.fixed(3), id)
  let assert Ok(#(db, _)) = checkout.ship(db, clock.fixed(4), id)
  #(db, id)
}

pub fn refund_restocks_test() {
  let #(db, id) = shipped()
  let assert Ok(#(db, o)) = checkout.refund(db, clock.fixed(5), id)
  assert o.status == order.Refunded
  assert o.updated_at == 5
  let assert Ok(a) = store.get_product(db, "A")
  assert a.stock == 5
}

pub fn refund_requires_shipped_test() {
  let #(db, id) = draft([#("A", 2)])
  assert checkout.refund(db, clock.fixed(5), id)
    == Error(
      checkout.Rejected(order.InvalidTransition(
        from: order.Draft,
        action: order.Refund,
      )),
    )
}

pub fn refund_twice_rejected_test() {
  let #(db, id) = shipped()
  let assert Ok(#(db, _)) = checkout.refund(db, clock.fixed(5), id)
  let assert Error(_) = checkout.refund(db, clock.fixed(6), id)
  let assert Ok(a) = store.get_product(db, "A")
  assert a.stock == 5
}
