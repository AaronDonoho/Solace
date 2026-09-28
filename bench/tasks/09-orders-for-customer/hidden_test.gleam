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

pub fn newest_first_test() {
  let db = setup()
  let assert Ok(#(db, a)) = checkout.create_order(db, clock.fixed(10), 1)
  let assert Ok(#(db, b)) = checkout.create_order(db, clock.fixed(30), 1)
  let assert Ok(#(db, _)) = checkout.create_order(db, clock.fixed(20), 2)
  let assert Ok(#(db, c)) = checkout.create_order(db, clock.fixed(10), 1)
  let assert Ok(orders) = checkout.orders_for(db, 1)
  assert orders == [b, c, a]
}

pub fn no_orders_test() {
  assert checkout.orders_for(setup(), 2) == Ok([])
}

pub fn unknown_customer_test() {
  assert checkout.orders_for(setup(), 7) == Error(checkout.UnknownCustomer(7))
}
