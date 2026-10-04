import gleam/list
import gleam/option.{None, Some}
import pilot/catalog
import pilot/checkout.{
  CouponRejected, InvalidQuantity, LineNotFound, OrderNotEditable, UnknownCoupon,
}
import pilot/customer
import pilot/inventory
import pilot/notifications
import pilot/order.{Line}
import pilot/payments
import pilot/promotions
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store
import support

pub fn build_an_order_test() {
  let #(db, o) =
    support.draft(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
      #("TEE-M-BLK", 1),
    ])
  assert o.lines == [Line("TEE-M-BLK", 3, 1800), Line("MUG-BLUE", 1, 1400)]
  let c = support.on_day(0)
  let assert Ok(#(db, o)) = checkout.set_quantity(db, c, o.id, "TEE-M-BLK", 2)
  assert o.lines == [Line("TEE-M-BLK", 2, 1800), Line("MUG-BLUE", 1, 1400)]
  let assert Ok(#(_, o)) = checkout.remove_line(db, c, o.id, "MUG-BLUE")
  assert o.lines == [Line("TEE-M-BLK", 2, 1800)]
}

pub fn add_line_errors_test() {
  let #(db, o) = support.draft(seed.demo_store(), seed.ada, [])
  let c = support.on_day(0)
  assert checkout.add_line(db, c, o.id, "TEE-M-BLK", 0)
    == Error(InvalidQuantity(0))
  assert checkout.add_line(db, c, o.id, "NOPE", 1)
    == Error(checkout.Catalog(catalog.UnknownSku("NOPE")))
  assert checkout.add_line(db, c, o.id, "CAP-RED", 1)
    == Error(checkout.Catalog(catalog.InactiveProduct("retired-cap")))
  assert checkout.remove_line(db, c, o.id, "TEE-M-BLK")
    == Error(LineNotFound("TEE-M-BLK"))
}

pub fn quote_test() {
  let #(db, o) =
    support.draft(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  let assert Ok(q) = checkout.quote(db, o.id, Standard)
  assert q.total_cents == 5824
  let #(db, o) = support.draft(db, seed.grace, [#("LAMP-OAK", 1)])
  let assert Ok(q) = checkout.quote(db, o.id, Standard)
  assert q.loyalty_discount_cents == 370
  assert q.shipping_cents == 600
  assert q.total_cents == 8140
}

pub fn coupons_test() {
  let #(db, o) = support.draft(seed.demo_store(), seed.ada, [#("TEE-M-BLK", 1)])
  let c = support.on_day(0)
  assert checkout.apply_coupon(db, c, o.id, "NOPE")
    == Error(UnknownCoupon("NOPE"))
  assert checkout.apply_coupon(db, c, o.id, "save10")
    == Error(CouponRejected(promotions.BelowMinimumSpend("SAVE10", 5000)))
  assert checkout.apply_coupon(db, c, o.id, "spring")
    == Error(CouponRejected(promotions.CouponExpired("SPRING")))
  let assert Ok(#(db, o)) = checkout.apply_coupon(db, c, o.id, " tenoff ")
  assert o.coupon_code == Some("TENOFF")
  let assert Ok(q) = checkout.quote(db, o.id, Standard)
  assert q.coupon_discount_cents == 1000
  let assert Ok(#(_, o)) = checkout.remove_coupon(db, c, o.id)
  assert o.coupon_code == None
}

pub fn pay_test() {
  let #(db, o) =
    support.paid(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  assert o.status == order.Paid
  let assert Ok(p) = store.payment_for_order(db, o.id)
  assert p.amount_cents == 5824
  assert p.status == payments.Authorized
  assert store.allocations_for(db, o.id)
    == [
      inventory.Allocation("WH-EAST", "TEE-M-BLK", 2),
      inventory.Allocation("WH-EAST", "MUG-BLUE", 1),
    ]
  assert inventory.available(db.stock, "WH-EAST", "TEE-M-BLK") == 18
  let assert Ok(ada) = store.get_customer(db, seed.ada)
  assert ada.lifetime_spend_cents == 5824
  let assert [message] = store.sent_messages(db)
  assert message.kind == notifications.OrderConfirmation
  assert message.to == "ada@example.com"
  assert checkout.add_line(db, support.on_day(0), o.id, "MUG-BLUE", 1)
    == Error(OrderNotEditable(order.Paid))
}

pub fn failed_payment_changes_nothing_test() {
  let #(db, o) =
    support.draft(seed.demo_store(), seed.ada, [#("PHONES-BLK", 1)])
  let c = support.on_day(0)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  assert checkout.pay(
      db,
      c,
      payments.fake_gateway(),
      o.id,
      "tok_decline",
      Standard,
    )
    == Error(checkout.Payment(payments.PaymentDeclined("card declined", 1)))
  assert inventory.available(db.stock, "WH-EAST", "PHONES-BLK") == 5
}

pub fn pay_fails_when_out_of_stock_test() {
  let #(db, o) = support.draft(seed.demo_store(), seed.ada, [#("LAMP-OAK", 7)])
  let c = support.on_day(0)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  assert checkout.pay(db, c, payments.fake_gateway(), o.id, "tok_ok", Standard)
    == Error(checkout.Inventory(inventory.InsufficientStock("LAMP-OAK", 7, 6)))
}

pub fn one_use_coupon_test() {
  let #(db, o) = support.draft(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)])
  let c = support.on_day(0)
  let assert Ok(#(db, _)) = checkout.apply_coupon(db, c, o.id, "WELCOME15")
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  let assert Ok(#(db, _)) =
    checkout.pay(db, c, payments.fake_gateway(), o.id, "tok_ok", Standard)
  assert store.coupon_uses(db, "welcome15", seed.ada) == 1
  let #(db, second) = support.draft(db, seed.ada, [#("MUG-BLUE", 1)])
  assert checkout.apply_coupon(db, c, second.id, "WELCOME15")
    == Error(CouponRejected(promotions.UsageLimitReached("WELCOME15")))
}

pub fn cancel_paid_order_test() {
  let #(db, o) = support.paid(seed.demo_store(), seed.ada, [#("LAMP-OAK", 2)])
  let assert Ok(#(db, o)) =
    checkout.cancel(db, support.on_day(0), o.id, "changed mind")
  assert o.status == order.Cancelled("changed mind")
  assert inventory.available(db.stock, "WH-EAST", "LAMP-OAK") == 4
  assert store.allocations_for(db, o.id) == []
  assert store.payment_for_order(db, o.id) == Error(Nil)
  let kinds = store.sent_messages(db) |> list.map(fn(m) { m.kind })
  assert kinds
    == [notifications.OrderConfirmation, notifications.OrderCancelled]
}

pub fn goodwill_refund_requires_capture_test() {
  let #(db, o) = support.paid(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)])
  let gateway = payments.fake_gateway()
  assert checkout.refund(db, support.on_day(1), gateway, o.id, 100, "sorry")
    == Error(
      checkout.Payment(payments.InvalidPaymentState(payments.Authorized)),
    )
  let #(db, _) = support.shipped(db, o.id)
  let assert Ok(#(db, 100)) =
    checkout.refund(db, support.on_day(2), gateway, o.id, 100, "sorry")
  let assert Ok(p) = store.payment_for_order(db, o.id)
  assert p.refunded_cents == 100
}

pub fn origin_warehouse_test() {
  let ws = seed.warehouses()
  let assert Ok(w) = checkout.origin_warehouse(ws, customer.Domestic("CA"))
  assert w.id == "WH-WEST"
  let assert Ok(w) = checkout.origin_warehouse(ws, customer.International("DE"))
  assert w.id == "WH-EAST"
  assert checkout.origin_warehouse([], customer.Domestic("CA"))
    == Error(checkout.NoWarehouse)
}
