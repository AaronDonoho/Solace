//// Helpers shared by the tests: fixed clocks and orders driven to a given
//// point in their lifecycle.

import gleam/list
import pilot/checkout
import pilot/clock
import pilot/fulfillment
import pilot/order.{type Order}
import pilot/payments
import pilot/seed
import pilot/shipments.{type Shipment}
import pilot/shipping
import pilot/store.{type Store}

/// A clock fixed `days` days after the seed epoch.
pub fn on_day(days: Int) -> clock.Clock {
  clock.fixed(seed.epoch + days * seed.day)
}

/// A draft order with the given (sku, quantity) lines.
pub fn draft(
  db: Store,
  customer_id: Int,
  lines: List(#(String, Int)),
) -> #(Store, Order) {
  let c = on_day(0)
  let assert Ok(#(db, o)) = checkout.create_order(db, c, customer_id)
  let db =
    list.fold(lines, db, fn(db, line) {
      let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, line.0, line.1)
      db
    })
  let assert Ok(o) = store.get_order(db, o.id)
  #(db, o)
}

/// A submitted and paid order, paid on day 0 with standard shipping.
pub fn paid(
  db: Store,
  customer_id: Int,
  lines: List(#(String, Int)),
) -> #(Store, Order) {
  let #(db, o) = draft(db, customer_id, lines)
  let c = on_day(0)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  let assert Ok(#(db, o)) =
    checkout.pay(
      db,
      c,
      payments.fake_gateway(),
      o.id,
      "tok_ok",
      shipping.Standard,
    )
  #(db, o)
}

/// A paid order shipped on day 1 and delivered on `delivered_day`.
pub fn delivered(
  db: Store,
  customer_id: Int,
  lines: List(#(String, Int)),
  delivered_day: Int,
) -> #(Store, Order, Shipment) {
  let #(db, o) = paid(db, customer_id, lines)
  let #(db, s) = shipped(db, o.id)
  let assert Ok(#(db, s)) = fulfillment.deliver(db, on_day(delivered_day), s.id)
  let assert Ok(o) = store.get_order(db, o.id)
  #(db, o, s)
}

/// Picks, packs and ships a paid order on day 1.
pub fn shipped(db: Store, order_id: Int) -> #(Store, Shipment) {
  let c = on_day(1)
  let assert Ok(#(db, s)) =
    fulfillment.start_picking(db, c, order_id, shipping.Standard)
  let assert Ok(#(db, s)) = fulfillment.pack(db, c, s.id)
  let assert Ok(#(db, s)) =
    fulfillment.ship(db, c, payments.fake_gateway(), s.id)
  #(db, s)
}
