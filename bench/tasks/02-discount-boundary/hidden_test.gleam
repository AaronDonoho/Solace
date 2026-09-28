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

pub fn exactly_100_gets_discount_test() {
  let #(db, id) = draft([#("A", 5)])
  let db = store.put_product(db, Product("A", "Widget", 2000, 5))
  let assert Ok(q) = checkout.quote(db, id)
  assert q.discount_cents == 1000
  assert q.total_cents == 9000 + 0 + 720
}

pub fn just_under_100_no_discount_test() {
  let db =
    setup() |> store.put_product(Product("C", "Thing", 9999, 5))
  let assert Ok(#(db, o)) = checkout.create_order(db, clock.fixed(1), 1)
  let assert Ok(#(db, _)) = checkout.add_line(db, clock.fixed(1), o.id, "C", 1)
  let assert Ok(q) = checkout.quote(db, o.id)
  assert q.discount_cents == 0
}
