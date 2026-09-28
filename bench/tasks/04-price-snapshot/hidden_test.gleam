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

pub fn price_locked_at_add_test() {
  let #(db, id) = draft([#("A", 2)])
  let db = store.put_product(db, Product("A", "Widget", 9000, 5))
  let assert Ok(q) = checkout.quote(db, id)
  assert q.subtotal_cents == 2000
}

pub fn merge_keeps_original_price_test() {
  let #(db, id) = draft([#("A", 1)])
  let db = store.put_product(db, Product("A", "Widget", 9000, 5))
  let assert Ok(db) = add_all(db, id, [#("A", 1)])
  let assert Ok(o) = store.get_order(db, id)
  assert o.lines == [order.Line("A", 2, 1000)]
}

pub fn pricing_uses_line_prices_test() {
  let q =
    pricing.quote(
      [order.Line("A", 2, 1000), order.Line("B", 1, 2500)],
      Domestic,
    )
  assert q.subtotal_cents == 4500
}
