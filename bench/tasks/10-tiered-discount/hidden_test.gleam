import pilot/checkout
import pilot/clock
import pilot/customer.{Customer, Domestic, International}
import pilot/order
import pilot/pricing
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


fn discount(price) {
  { pricing.quote([#(order.Line("X", 1), Product("X", "X", price, 1))], Domestic) }.discount_cents
}

pub fn tiers_test() {
  assert discount(20_000) == 3000
  assert discount(20_001) == 3000
  assert discount(19_999) == 1999
  assert discount(10_000) == 1000
  assert discount(9999) == 0
}
