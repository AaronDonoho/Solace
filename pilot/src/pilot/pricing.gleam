import gleam/int
import gleam/list
import pilot/customer.{type Region, Domestic, International}
import pilot/order.{type Line}
import pilot/product.{type Product}

pub type Quote {
  Quote(
    subtotal_cents: Int,
    discount_cents: Int,
    shipping_cents: Int,
    tax_cents: Int,
    total_cents: Int,
  )
}

const discount_threshold_cents = 10_000

const free_shipping_threshold_cents = 5000

const domestic_shipping_cents = 500

const international_shipping_cents = 2000

const domestic_tax_percent = 8

/// Prices lines whose products have already been looked up.
pub fn quote(lines: List(#(Line, Product)), region: Region) -> Quote {
  let subtotal =
    lines
    |> list.map(fn(pair) { { pair.0 }.quantity * { pair.1 }.price_cents })
    |> int.sum
  let discount = case subtotal >= discount_threshold_cents {
    True -> subtotal / 10
    False -> 0
  }
  let taxable = subtotal - discount
  let shipping = case region {
    Domestic if taxable >= free_shipping_threshold_cents -> 0
    Domestic -> domestic_shipping_cents
    International -> international_shipping_cents
  }
  let tax = case region {
    Domestic -> taxable * domestic_tax_percent / 100
    International -> 0
  }
  Quote(
    subtotal_cents: subtotal,
    discount_cents: discount,
    shipping_cents: shipping,
    tax_cents: tax,
    total_cents: taxable + shipping + tax,
  )
}
