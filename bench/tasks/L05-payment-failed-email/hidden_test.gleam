import gleam/list
import gleam/string
import pilot/api
import pilot/audit
import pilot/checkout
import pilot/clock
import pilot/inventory
import pilot/notifications
import pilot/order
import pilot/payments
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

fn submitted(sku, quantity) {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), c, seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, sku, quantity)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  #(db, o.id)
}

fn pay(db, id, token) {
  checkout.pay(db, clock.fixed(seed.epoch), payments.fake_gateway(), id, token, Standard)
}

fn failures(db) {
  store.sent_messages(db) |> list.filter(fn(m) { m.kind == notifications.PaymentFailed })
}

pub fn declined_card_emails_customer_test() {
  let #(db, id) = submitted("PHONES-BLK", 1)
  let #(after, result) = pay(db, id, "tok_decline")
  assert result == Error(checkout.Payment(payments.PaymentDeclined("card declined", 1)))
  let assert [m] = failures(after)
  assert m.to == "ada@example.com"
  assert m.order_id == id
  assert string.contains(m.body, "Attempts: 1")
  let actions =
    store.audit_entries(after)
    |> audit.for_subject(audit.order_subject(id))
    |> list.map(fn(e) { e.action })
  assert list.contains(actions, "payment failed")
  let assert Ok(o) = store.get_order(after, id)
  assert o.status == order.Submitted
  assert inventory.available(after.stock, "WH-EAST", "PHONES-BLK") == 5
  assert store.payment_for_order(after, id) == Error(Nil)
}

pub fn provider_unavailable_reports_attempts_test() {
  let #(db, id) = submitted("PHONES-BLK", 1)
  let #(after, result) = pay(db, id, "tok_network")
  assert result == Error(checkout.Payment(payments.GatewayUnavailable(3)))
  let assert [m] = failures(after)
  assert string.contains(m.body, "Attempts: 3")
}

pub fn other_failures_leave_store_unchanged_test() {
  let #(db, id) = submitted("LAMP-OAK", 7)
  let #(after, result) = pay(db, id, "tok_ok")
  let assert Error(checkout.Inventory(_)) = result
  assert after == db
}

pub fn success_sends_no_failure_email_test() {
  let #(db, id) = submitted("MUG-BLUE", 1)
  let #(after, result) = pay(db, id, "tok_flaky")
  let assert Ok(o) = result
  assert o.status == order.Paid
  assert failures(after) == []
}

pub fn api_returns_store_with_email_test() {
  let #(db, id) = submitted("MUG-BLUE", 1)
  let env = api.Env(clock: clock.fixed(seed.epoch), gateway: payments.fake_gateway())
  let #(after, response) = api.handle(db, env, api.PayOrder(id, "tok_decline", Standard))
  assert response.status == 402
  let assert [_] = failures(after)
}
