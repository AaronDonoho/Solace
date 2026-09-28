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


fn intl(price) {
  pricing.quote([#(order.Line("X", 1), Product("X", "X", price, 1))], International)
}

pub fn free_at_threshold_test() {
  // 22_222 - 2_222 discount = 20_000
  assert { intl(22_222) }.shipping_cents == 0
  assert { intl(22_222) }.total_cents == 20_000
}

pub fn flat_rate_below_threshold_test() {
  // 22_221 - 2_222 discount = 19_999
  assert { intl(22_221) }.shipping_cents == 2000
}

pub fn domestic_unchanged_test() {
  let q =
    pricing.quote([#(order.Line("X", 1), Product("X", "X", 4999, 1))], Domestic)
  assert q.shipping_cents == 500
}
