import gleam/dict
import pilot/checkout
import pilot/clock
import pilot/fulfillment
import pilot/payments
import pilot/return_policy.{Damaged, Item}
import pilot/returns
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

fn at(day) {
  clock.fixed(seed.epoch + day * seed.day)
}

fn delivered() {
  let g = payments.fake_gateway()
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), at(0), seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, at(0), o.id, "TEE-M-BLK", 2)
  let assert Ok(#(db, _)) = checkout.add_line(db, at(0), o.id, "MUG-BLUE", 1)
  let assert Ok(#(db, _)) = checkout.submit(db, at(0), o.id)
  let assert Ok(#(db, _)) = checkout.pay(db, at(0), g, o.id, "tok_ok", Standard)
  let assert Ok(#(db, s)) = fulfillment.start_picking(db, at(1), o.id, Standard)
  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), s.id)
  let assert Ok(#(db, _)) = fulfillment.ship(db, at(1), g, s.id)
  let assert Ok(#(db, _)) = fulfillment.deliver(db, at(3), s.id)
  #(db, o.id)
}

pub fn returning_all_of_one_line_keeps_shipping_test() {
  let #(db, id) = delivered()
  let assert Ok(#(db, first)) = returns.request(db, at(5), id, [Item("TEE-M-BLK", 2)], Damaged)
  assert first.refund.shipping_cents == 0
  assert first.refund.total_cents == 3761
  let assert Ok(#(_, last)) = returns.request(db, at(5), id, [Item("MUG-BLUE", 1)], Damaged)
  assert last.refund.shipping_cents == 600
  assert last.refund.total_cents == 2062
}

pub fn completes_order_checks_every_line_test() {
  let #(db, id) = delivered()
  let assert Ok(o) = store.get_order(db, id)
  assert !return_policy.completes_order(o, [Item("TEE-M-BLK", 2)], dict.new())
  assert return_policy.completes_order(o, [Item("MUG-BLUE", 1)], dict.from_list([#("TEE-M-BLK", 2)]))
}
