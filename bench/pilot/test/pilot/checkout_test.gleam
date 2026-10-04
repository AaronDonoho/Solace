import pilot/checkout.{
  InvalidQuantity, OrderNotEditable, OutOfStock, Rejected, UnknownCustomer,
  UnknownProduct,
}
import pilot/clock
import pilot/customer.{Customer, Domestic}
import pilot/order.{EmptyOrder, Line, Submitted}
import pilot/product.{Product}
import pilot/store

const now = 1_700_000_000_000

fn setup() {
  store.new()
  |> store.put_customer(Customer(1, "Ada", Domestic))
  |> store.put_product(Product("A", "Widget", 1000, 5))
  |> store.put_product(Product("B", "Gadget", 2500, 1))
}

pub fn full_checkout_test() {
  let c = clock.fixed(now)
  let assert Ok(#(db, o)) = checkout.create_order(setup(), c, 1)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "A", 2)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "A", 1)
  let assert Ok(#(db, o)) = checkout.add_line(db, c, o.id, "B", 1)
  assert o.lines == [Line("A", 3), Line("B", 1)]
  let assert Ok(q) = checkout.quote(db, o.id)
  assert q.total_cents == 5500 + 0 + 440
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  let assert Ok(#(db, _)) = checkout.approve(db, c, o.id)
  let assert Ok(a) = store.get_product(db, "A")
  assert a.stock == 2
  let assert Ok(#(_, o)) = checkout.ship(db, c, o.id)
  assert o.status == order.Shipped
}

pub fn unknown_customer_test() {
  assert checkout.create_order(setup(), clock.fixed(now), 99)
    == Error(UnknownCustomer(99))
}

pub fn add_line_validation_test() {
  let c = clock.fixed(now)
  let assert Ok(#(db, o)) = checkout.create_order(setup(), c, 1)
  assert checkout.add_line(db, c, o.id, "Z", 1) == Error(UnknownProduct("Z"))
  assert checkout.add_line(db, c, o.id, "A", 0) == Error(InvalidQuantity(0))
  assert checkout.submit(db, c, o.id) == Error(Rejected(EmptyOrder))
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "A", 1)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  assert checkout.add_line(db, c, o.id, "A", 1)
    == Error(OrderNotEditable(Submitted))
}

pub fn approve_fails_when_out_of_stock_and_reserves_nothing_test() {
  let c = clock.fixed(now)
  let assert Ok(#(db, o)) = checkout.create_order(setup(), c, 1)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "A", 1)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "B", 2)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  assert checkout.approve(db, c, o.id) == Error(OutOfStock("B", 2, 1))
}
