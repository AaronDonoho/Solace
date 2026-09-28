import gleam/option.{None, Some}
import pilot/checkout
import pilot/clock
import pilot/fulfillment
import pilot/payments
import pilot/reporting.{MonthlyRefunds}
import pilot/return_policy.{Damaged, Item, NoLongerNeeded}
import pilot/returns
import pilot/seed
import pilot/shipping.{Standard}

fn at(day) {
  clock.fixed(seed.epoch + day * seed.day)
}

/// A delivered order for one mug, then a return refunded on `refund_day`.
fn refunded(db, reason, refund_day) {
  let g = payments.fake_gateway()
  let assert Ok(#(db, o)) = checkout.create_order(db, at(0), seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, at(0), o.id, "MUG-BLUE", 1)
  let assert Ok(#(db, _)) = checkout.submit(db, at(0), o.id)
  let assert Ok(#(db, _)) = checkout.pay(db, at(0), g, o.id, "tok_ok", Standard)
  let assert Ok(#(db, s)) = fulfillment.start_picking(db, at(1), o.id, Standard)
  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), s.id)
  let assert Ok(#(db, _)) = fulfillment.ship(db, at(1), g, s.id)
  let assert Ok(#(db, _)) = fulfillment.deliver(db, at(3), s.id)
  let assert Ok(#(db, r)) = returns.request(db, at(refund_day), o.id, [Item("MUG-BLUE", 1)], reason)
  assert r.refunded_at == None
  let assert Ok(#(db, _)) = returns.approve(db, at(refund_day), r.id)
  let assert Ok(#(db, _)) = returns.receive(db, at(refund_day), r.id)
  let assert Ok(#(db, r)) = returns.refund(db, at(refund_day), g, r.id)
  assert r.refunded_at == Some(seed.epoch + refund_day * seed.day)
  db
}

pub fn refunds_by_month_test() {
  let db =
    seed.demo_store()
    |> refunded(Damaged, 6)
    |> refunded(NoLongerNeeded, 30)
    |> refunded(Damaged, 29)
  // an unrefunded return is not counted
  let assert Ok(#(db, o)) = checkout.create_order(db, at(0), seed.ada)
  let _ = o
  assert reporting.refunds_by_month(db)
    == [
      MonthlyRefunds("2026-03", "damaged", 2, 4160),
      MonthlyRefunds("2026-04", "no longer needed", 1, 2080),
    ]
}

pub fn empty_test() {
  assert reporting.refunds_by_month(seed.demo_store()) == []
}
