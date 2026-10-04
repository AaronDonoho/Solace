import gleam/option.{None, Some}
import pilot/customer.{Domestic, Gold, Standard}
import pilot/order.{Line}
import pilot/pricing
import pilot/promotions.{Coupon, FixedOff, FreeShipping, PercentOff}
import pilot/shipping

fn ship(cost) {
  shipping.ShippingQuote(shipping.Standard, shipping.National, cost, 4)
}

fn coupon(kind, stacks) {
  Coupon("C", kind, 0, None, None, stacks, True)
}

pub fn plain_order_test() {
  let q =
    pricing.quote(
      [Line("A", 2, 1800), Line("B", 1, 1400)],
      Standard,
      None,
      ship(600),
      Domestic("NY"),
    )
  assert q.subtotal_cents == 5000
  assert pricing.discounts_cents(q) == 0
  assert q.tax_cents == 224
  assert q.total_cents == 5824
}

pub fn loyalty_then_fixed_coupon_test() {
  // Gold: 5% of 7400 = 370, then $10 off the remaining 7030
  let q =
    pricing.quote(
      [Line("L", 1, 7400)],
      Gold,
      Some(coupon(FixedOff(1000), True)),
      ship(600),
      Domestic("CA"),
    )
  assert q.loyalty_discount_cents == 370
  assert q.coupon_discount_cents == 1000
  assert pricing.goods_cents(q) == 6030
  assert q.tax_cents == 437
  assert q.total_cents == 6030 + 600 + 437
}

pub fn non_stacking_coupon_skips_loyalty_test() {
  let q =
    pricing.quote(
      [Line("L", 1, 10_000)],
      Gold,
      Some(coupon(PercentOff(15), False)),
      ship(0),
      Domestic("OR"),
    )
  assert q.loyalty_discount_cents == 0
  assert q.coupon_discount_cents == 1500
  assert q.total_cents == 8500
}

pub fn volume_discount_test() {
  let q =
    pricing.quote([Line("A", 20, 100)], Standard, None, ship(0), Domestic("OR"))
  assert q.volume_discount_cents == 100
  let q =
    pricing.quote([Line("A", 19, 100)], Standard, None, ship(0), Domestic("OR"))
  assert q.volume_discount_cents == 0
}

pub fn free_shipping_coupon_test() {
  let q =
    pricing.quote(
      [Line("A", 1, 1000)],
      Standard,
      Some(coupon(FreeShipping, True)),
      ship(600),
      Domestic("NY"),
    )
  assert q.shipping_cents == 0
  assert q.tax_cents == 40
}
