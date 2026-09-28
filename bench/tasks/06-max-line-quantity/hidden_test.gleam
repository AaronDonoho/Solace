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

pub fn ten_is_allowed_test() {
  let db = setup() |> store.put_product(Product("A", "Widget", 1000, 50))
  let assert Ok(#(db, o)) = checkout.create_order(db, clock.fixed(1), 1)
  let assert Ok(#(_, o)) = checkout.add_line(db, clock.fixed(1), o.id, "A", 10)
  assert o.lines == [order.Line("A", 10)]
}

pub fn eleven_rejected_test() {
  let #(db, id) = draft([])
  assert checkout.add_line(db, clock.fixed(1), id, "A", 11)
    == Error(checkout.InvalidQuantity(11))
}

pub fn merge_over_limit_rejected_test() {
  let #(db, id) = draft([#("A", 6)])
  assert checkout.add_line(db, clock.fixed(1), id, "A", 5)
    == Error(checkout.InvalidQuantity(11))
  let assert Ok(o) = store.get_order(db, id)
  assert o.lines == [order.Line("A", 6)]
}
