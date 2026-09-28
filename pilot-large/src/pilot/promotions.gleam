//// Coupons: what they discount and when they can be used.

import gleam/int
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import pilot/money
import pilot/order.{type Line}

pub type CouponKind {
  PercentOff(percent: Int)
  FixedOff(cents: Int)
  FreeShipping
  /// Buy `buy` units of `sku` and get `get` more free, repeatedly.
  BuyXGetY(sku: String, buy: Int, get: Int)
}

pub type Coupon {
  Coupon(
    code: String,
    kind: CouponKind,
    min_spend_cents: Int,
    expires_at: Option(Int),
    max_uses_per_customer: Option(Int),
    stacks_with_loyalty: Bool,
    active: Bool,
  )
}

/// What a coupon is checked against.
pub type CouponContext {
  CouponContext(
    lines: List(Line),
    subtotal_cents: Int,
    now: Int,
    customer_uses: Int,
  )
}

pub type CouponError {
  CouponInactive(code: String)
  CouponExpired(code: String)
  BelowMinimumSpend(code: String, minimum_cents: Int)
  UsageLimitReached(code: String)
  NotApplicable(code: String, reason: String)
}

/// Coupon codes are case-insensitive and ignore surrounding spaces.
pub fn normalize_code(code: String) -> String {
  code |> string.trim |> string.uppercase
}

pub fn validate(
  coupon: Coupon,
  context: CouponContext,
) -> Result(Coupon, CouponError) {
  let code = coupon.code
  let expired = case coupon.expires_at {
    Some(at) -> context.now >= at
    None -> False
  }
  let used_up = case coupon.max_uses_per_customer {
    Some(max) -> context.customer_uses >= max
    None -> False
  }
  case coupon.kind {
    _ if !coupon.active -> Error(CouponInactive(code))
    _ if expired -> Error(CouponExpired(code))
    _ if used_up -> Error(UsageLimitReached(code))
    _ if context.subtotal_cents < coupon.min_spend_cents ->
      Error(BelowMinimumSpend(code, coupon.min_spend_cents))
    BuyXGetY(sku:, buy:, get:) ->
      case list.find(context.lines, fn(l) { l.sku == sku }) {
        Ok(line) if line.quantity >= buy + get -> Ok(coupon)
        _ ->
          Error(NotApplicable(
            code,
            "needs " <> int.to_string(buy + get) <> " units of " <> sku,
          ))
      }
    _ -> Ok(coupon)
  }
}

/// The amount a coupon takes off `base_cents`, the order value it applies
/// to after any earlier discounts. Never more than `base_cents`.
pub fn discount(coupon: Coupon, lines: List(Line), base_cents: Int) -> Int {
  let amount = case coupon.kind {
    PercentOff(percent) -> money.percent(base_cents, percent)
    FixedOff(cents) -> cents
    FreeShipping -> 0
    BuyXGetY(sku:, buy:, get:) ->
      case list.find(lines, fn(l) { l.sku == sku }) {
        Ok(line) -> line.quantity / { buy + get } * get * line.unit_price_cents
        Error(_) -> 0
      }
  }
  int.clamp(amount, 0, base_cents)
}

pub fn gives_free_shipping(coupon: Coupon) -> Bool {
  coupon.kind == FreeShipping
}

pub fn describe(coupon: Coupon) -> String {
  case coupon.kind {
    PercentOff(p) -> int.to_string(p) <> "% off"
    FixedOff(c) -> money.format(c) <> " off"
    FreeShipping -> "free shipping"
    BuyXGetY(sku:, buy:, get:) ->
      "buy "
      <> int.to_string(buy)
      <> " "
      <> sku
      <> ", get "
      <> int.to_string(get)
      <> " free"
  }
}
