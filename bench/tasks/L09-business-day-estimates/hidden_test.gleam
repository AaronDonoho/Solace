import gleam/list
import gleam/string
import pilot/checkout
import pilot/clock
import pilot/customer.{Domestic}
import pilot/date.{Date}
import pilot/fulfillment
import pilot/notifications
import pilot/payments
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

pub fn business_days_skip_weekends_test() {
  assert date.add_business_days(Date(2026, 3, 5), 2) == Date(2026, 3, 9)
  assert date.add_business_days(Date(2026, 3, 6), 1) == Date(2026, 3, 9)
  assert date.add_business_days(Date(2026, 3, 7), 1) == Date(2026, 3, 9)
  assert date.add_business_days(Date(2026, 3, 2), 5) == Date(2026, 3, 9)
  assert date.add_business_days(Date(2026, 3, 2), 3) == Date(2026, 3, 5)
}

pub fn estimate_test() {
  let q = shipping.quote(Standard, "NJ", Domestic("NY"), 850, 0)
  assert shipping.estimate_delivery(Date(2026, 3, 5), q) == Date(2026, 3, 11)
}

pub fn shipped_email_test() {
  // Shipped Tuesday 2026-03-03: 4 business days is Monday 2026-03-09
  let at = fn(day) { clock.fixed(seed.epoch + day * seed.day) }
  let g = payments.fake_gateway()
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), at(0), seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, at(0), o.id, "MUG-BLUE", 1)
  let assert Ok(#(db, _)) = checkout.submit(db, at(0), o.id)
  let assert Ok(#(db, _)) = checkout.pay(db, at(0), g, o.id, "tok_ok", Standard)
  let assert Ok(#(db, s)) = fulfillment.start_picking(db, at(1), o.id, Standard)
  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), s.id)
  let assert Ok(#(db, _)) = fulfillment.ship(db, at(1), g, s.id)
  let assert Ok(m) =
    list.find(store.sent_messages(db), fn(m) { m.kind == notifications.OrderShipped })
  assert string.contains(m.body, "should arrive by 2026-03-09")
}
