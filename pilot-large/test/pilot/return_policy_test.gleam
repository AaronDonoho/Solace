import gleam/dict
import gleam/option.{None, Some}
import pilot/catalog.{Apparel, Electronics}
import pilot/customer.{Address, Domestic}
import pilot/order.{Line}
import pilot/pricing.{Quote}
import pilot/return_policy.{
  Damaged, EmptyReturn, Item, ItemNotInOrder, NoLongerNeeded, NotDelivered,
  QuantityExceedsPurchased, RefundBreakdown, WindowClosed,
}
import pilot/seed

fn delivered_on(day: Int) {
  order.Order(
    ..order.new(1, 1, Address("1 Main", "Albany", "12207", Domestic("NY")), 0),
    lines: [Line("TEE", 2, 1800), Line("MUG", 1, 1400)],
    status: order.Delivered,
    delivered_at: Some(seed.epoch + day * seed.day),
  )
}

fn quote() {
  Quote(
    lines: [],
    subtotal_cents: 5000,
    loyalty_discount_cents: 0,
    coupon_discount_cents: 0,
    volume_discount_cents: 0,
    shipping_cents: 600,
    tax_cents: 224,
    total_cents: 5824,
  )
}

fn now(day: Int) {
  seed.epoch + day * seed.day
}

pub fn window_test() {
  let assert Ok(Nil) = return_policy.check_window(delivered_on(3), now(33))
  assert return_policy.check_window(delivered_on(3), now(34))
    == Error(WindowClosed(31))
  let not_yet =
    order.Order(..delivered_on(3), status: order.Shipped, delivered_at: None)
  assert return_policy.check_window(not_yet, now(4))
    == Error(NotDelivered(order.Shipped))
}

pub fn validate_test() {
  let o = delivered_on(0)
  let none = dict.new()
  assert return_policy.validate(
      o,
      [Item("TEE", 1), Item("TEE", 1)],
      none,
      now(1),
    )
    == Ok([Item("TEE", 2)])
  assert return_policy.validate(o, [], none, now(1)) == Error(EmptyReturn)
  assert return_policy.validate(o, [Item("HAT", 1)], none, now(1))
    == Error(ItemNotInOrder("HAT"))
  assert return_policy.validate(
      o,
      [Item("TEE", 2)],
      dict.from_list([#("TEE", 1)]),
      now(1),
    )
    == Error(QuantityExceedsPurchased("TEE", 1))
}

pub fn partial_refund_excludes_shipping_test() {
  let items = [Item("TEE", 1)]
  let whole = return_policy.completes_order(delivered_on(0), items, dict.new())
  assert !whole
  assert return_policy.refund_amount(
      delivered_on(0),
      quote(),
      items,
      dict.from_list([#("TEE", Apparel)]),
      NoLongerNeeded,
      whole,
    )
    == RefundBreakdown(1800, 0, 80, 0, 0, 1880)
}

pub fn whole_order_refund_includes_shipping_test() {
  let items = [Item("TEE", 2), Item("MUG", 1)]
  assert return_policy.completes_order(delivered_on(0), items, dict.new())
  let r =
    return_policy.refund_amount(
      delivered_on(0),
      quote(),
      items,
      dict.new(),
      Damaged,
      True,
    )
  assert r.total_cents == 5824
}

pub fn restocking_fee_test() {
  assert return_policy.restocking_fee_percent(Electronics, NoLongerNeeded) == 15
  assert return_policy.restocking_fee_percent(Electronics, Damaged) == 0
  assert return_policy.restocking_fee_percent(Apparel, NoLongerNeeded) == 0
}
