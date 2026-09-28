import gleam/option.{None, Some}
import pilot/order.{Line}
import pilot/promotions.{
  BelowMinimumSpend, BuyXGetY, Coupon, CouponContext, CouponExpired,
  CouponInactive, FixedOff, FreeShipping, NotApplicable, PercentOff,
  UsageLimitReached,
}

fn coupon(kind) {
  Coupon("C", kind, 0, None, None, True, True)
}

fn context(lines, uses) {
  CouponContext(
    lines:,
    subtotal_cents: order.subtotal_cents(lines),
    now: 100,
    customer_uses: uses,
  )
}

pub fn validate_test() {
  let lines = [Line("A", 1, 5000)]
  let ok = coupon(PercentOff(10))
  assert promotions.validate(ok, context(lines, 0)) == Ok(ok)
  assert promotions.validate(Coupon(..ok, active: False), context(lines, 0))
    == Error(CouponInactive("C"))
  assert promotions.validate(
      Coupon(..ok, expires_at: Some(100)),
      context(lines, 0),
    )
    == Error(CouponExpired("C"))
  let assert Ok(_) =
    promotions.validate(Coupon(..ok, expires_at: Some(101)), context(lines, 0))
  assert promotions.validate(
      Coupon(..ok, max_uses_per_customer: Some(1)),
      context(lines, 1),
    )
    == Error(UsageLimitReached("C"))
  assert promotions.validate(
      Coupon(..ok, min_spend_cents: 5001),
      context(lines, 0),
    )
    == Error(BelowMinimumSpend("C", 5001))
}

pub fn buy_x_get_y_needs_enough_units_test() {
  let c = coupon(BuyXGetY("A", 2, 1))
  let assert Error(NotApplicable("C", _)) =
    promotions.validate(c, context([Line("A", 2, 100)], 0))
  let assert Ok(_) = promotions.validate(c, context([Line("A", 3, 100)], 0))
}

pub fn discount_test() {
  let lines = [Line("A", 7, 100), Line("B", 1, 1000)]
  assert promotions.discount(coupon(PercentOff(10)), lines, 1700) == 170
  assert promotions.discount(coupon(FixedOff(1000)), lines, 1700) == 1000
  assert promotions.discount(coupon(FixedOff(5000)), lines, 1700) == 1700
  assert promotions.discount(coupon(FreeShipping), lines, 1700) == 0
  // 7 units: two sets of 3, so 2 free
  assert promotions.discount(coupon(BuyXGetY("A", 2, 1)), lines, 1700) == 200
}

pub fn normalize_and_describe_test() {
  assert promotions.normalize_code("  save10 ") == "SAVE10"
  assert promotions.describe(coupon(FixedOff(1000))) == "$10.00 off"
  assert promotions.gives_free_shipping(coupon(FreeShipping))
}
