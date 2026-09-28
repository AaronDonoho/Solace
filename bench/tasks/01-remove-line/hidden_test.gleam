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

pub fn removes_only_that_line_test() {
  let #(db, id) = draft([#("A", 2), #("B", 1)])
  let assert Ok(#(db, o)) = checkout.remove_line(db, clock.fixed(9), id, "A")
  assert o.lines == [order.Line("B", 1)]
  assert o.updated_at == 9
  assert store.get_order(db, id) == Ok(o)
}

pub fn line_not_found_test() {
  let #(db, id) = draft([#("A", 1)])
  assert checkout.remove_line(db, clock.fixed(9), id, "B")
    == Error(checkout.LineNotFound("B"))
}

pub fn unknown_order_test() {
  assert checkout.remove_line(setup(), clock.fixed(9), 42, "A")
    == Error(checkout.UnknownOrder(42))
}

pub fn not_editable_test() {
  let #(db, id) = draft([#("A", 1)])
  let assert Ok(#(db, _)) = checkout.submit(db, clock.fixed(2), id)
  assert checkout.remove_line(db, clock.fixed(9), id, "A")
    == Error(checkout.OrderNotEditable(order.Submitted))
}
