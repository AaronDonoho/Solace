import gleam/dict
import gleam/list
import pilot/inventory
import pilot/notifications
import pilot/payments
import pilot/return_policy.{Damaged, Item, NoLongerNeeded}
import pilot/returns
import pilot/seed
import pilot/store
import support

fn gateway() {
  payments.fake_gateway()
}

pub fn full_return_flow_test() {
  let #(db, o, _) =
    support.delivered(
      seed.demo_store(),
      seed.ada,
      [#("TEE-M-BLK", 2), #("MUG-BLUE", 1)],
      3,
    )
  let c = support.on_day(5)
  let assert Ok(#(db, r)) =
    returns.request(db, c, o.id, [Item("TEE-M-BLK", 1)], NoLongerNeeded)
  assert r.refund.total_cents == 1880
  let assert Ok(#(db, _)) = returns.approve(db, c, r.id)
  let assert Ok(#(db, _)) = returns.receive(db, c, r.id)
  assert dict.get(db.stock.on_hand, #("WH-EAST", "TEE-M-BLK")) == Ok(19)
  let assert Ok(#(db, r)) = returns.refund(db, c, gateway(), r.id)
  assert r.status == return_policy.Refunded
  let assert Ok(p) = store.payment_for_order(db, o.id)
  assert p.refunded_cents == 1880
  let kinds =
    store.sent_messages(db)
    |> list.map(fn(m) { m.kind })
    |> list.reverse
    |> list.take(2)
  assert kinds == [notifications.RefundIssued, notifications.ReturnReceived]
}

pub fn returning_everything_refunds_shipping_test() {
  let #(db, o, _) =
    support.delivered(
      seed.demo_store(),
      seed.ada,
      [#("TEE-M-BLK", 2), #("MUG-BLUE", 1)],
      3,
    )
  let c = support.on_day(5)
  let assert Ok(#(db, first)) =
    returns.request(db, c, o.id, [Item("TEE-M-BLK", 2)], Damaged)
  assert first.refund.shipping_cents == 0
  let assert Ok(#(_, last)) =
    returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
  assert last.refund.shipping_cents == 600
}

pub fn electronics_restocking_fee_test() {
  let #(db, o, _) =
    support.delivered(seed.demo_store(), seed.ada, [#("PHONES-BLK", 1)], 3)
  let c = support.on_day(5)
  let assert Ok(#(_, r)) =
    returns.request(db, c, o.id, [Item("PHONES-BLK", 1)], NoLongerNeeded)
  assert r.refund.restocking_fee_cents == 1935
  assert r.refund.total_cents == 11_481
  let assert Ok(#(_, r)) =
    returns.request(db, c, o.id, [Item("PHONES-BLK", 1)], Damaged)
  assert r.refund.total_cents == 13_416
}

pub fn cannot_return_twice_test() {
  let #(db, o, _) =
    support.delivered(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)], 3)
  let c = support.on_day(5)
  let assert Ok(#(db, _)) =
    returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
  assert returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
    == Error(
      returns.Policy(return_policy.QuantityExceedsPurchased("MUG-BLUE", 0)),
    )
}

pub fn rejected_returns_free_the_items_test() {
  let #(db, o, _) =
    support.delivered(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)], 3)
  let c = support.on_day(5)
  let assert Ok(#(db, r)) =
    returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
  let assert Ok(#(db, _)) = returns.reject(db, c, r.id, "no damage found")
  let assert Ok(_) =
    returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
}

pub fn must_receive_before_refund_test() {
  let #(db, o, _) =
    support.delivered(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)], 3)
  let c = support.on_day(5)
  let assert Ok(#(db, r)) =
    returns.request(db, c, o.id, [Item("MUG-BLUE", 1)], Damaged)
  let assert Error(returns.Policy(return_policy.InvalidReturnState(_))) =
    returns.refund(db, c, gateway(), r.id)
  assert inventory.available(db.stock, "WH-EAST", "MUG-BLUE") == 29
}
