//// Order totals.
////
//// Discounts apply in this order, each to what remains after the ones
//// before it:
////
//// 1. Loyalty: the customer's tier percentage of the subtotal. Skipped when
////    the order has a coupon that doesn't stack with loyalty.
//// 2. Coupon.
//// 3. Volume: 5% off orders of 20 or more units.
////
//// A free-shipping coupon waives shipping. Tax is charged on the discounted
//// goods, plus shipping where the destination taxes it.

import gleam/list
import gleam/option.{type Option, None, Some}
import pilot/customer.{type LoyaltyTier, type Region}
import pilot/money
import pilot/order.{type Line}
import pilot/promotions.{type Coupon}
import pilot/shipping.{type ShippingQuote}
import pilot/tax

pub type LineQuote {
  LineQuote(sku: String, quantity: Int, unit_price_cents: Int, total_cents: Int)
}

pub type Quote {
  Quote(
    lines: List(LineQuote),
    subtotal_cents: Int,
    loyalty_discount_cents: Int,
    coupon_discount_cents: Int,
    volume_discount_cents: Int,
    shipping_cents: Int,
    tax_cents: Int,
    total_cents: Int,
  )
}

const volume_min_units = 20

const volume_percent = 5

pub fn quote(
  lines: List(Line),
  tier: LoyaltyTier,
  coupon: Option(Coupon),
  shipping: ShippingQuote,
  destination: Region,
) -> Quote {
  let line_quotes =
    list.map(lines, fn(l) {
      LineQuote(
        sku: l.sku,
        quantity: l.quantity,
        unit_price_cents: l.unit_price_cents,
        total_cents: l.quantity * l.unit_price_cents,
      )
    })
  let subtotal = order.subtotal_cents(lines)
  let loyalty_applies = case coupon {
    Some(c) -> c.stacks_with_loyalty
    None -> True
  }
  let loyalty = case loyalty_applies {
    True -> money.percent(subtotal, customer.loyalty_discount_percent(tier))
    False -> 0
  }
  let coupon_discount = case coupon {
    Some(c) -> promotions.discount(c, lines, subtotal - loyalty)
    None -> 0
  }
  let units = list.fold(lines, 0, fn(sum, l) { sum + l.quantity })
  let after_coupon = subtotal - loyalty - coupon_discount
  let volume = case units >= volume_min_units {
    True -> money.percent(after_coupon, volume_percent)
    False -> 0
  }
  let goods = after_coupon - volume
  let shipping_cents = case coupon {
    Some(c) ->
      case promotions.gives_free_shipping(c) {
        True -> 0
        False -> shipping.cost_cents
      }
    None -> shipping.cost_cents
  }
  let tax = tax.quote(destination, goods, shipping_cents)
  Quote(
    lines: line_quotes,
    subtotal_cents: subtotal,
    loyalty_discount_cents: loyalty,
    coupon_discount_cents: coupon_discount,
    volume_discount_cents: volume,
    shipping_cents:,
    tax_cents: tax.tax_cents,
    total_cents: goods + shipping_cents + tax.tax_cents,
  )
}

/// All discounts on the quote combined.
pub fn discounts_cents(quote: Quote) -> Int {
  quote.loyalty_discount_cents
  + quote.coupon_discount_cents
  + quote.volume_discount_cents
}

/// Goods value after discounts, before shipping and tax.
pub fn goods_cents(quote: Quote) -> Int {
  quote.subtotal_cents - discounts_cents(quote)
}
