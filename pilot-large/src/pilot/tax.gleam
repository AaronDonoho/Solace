//// Sales tax.

import gleam/list
import gleam/result
import pilot/customer.{type Region, Domestic, International}
import pilot/money

pub type TaxQuote {
  TaxQuote(rate_bps: Int, taxable_cents: Int, tax_cents: Int)
}

/// State rates in basis points. States not listed use `default_rate_bps`.
const state_rates = [
  #("CA", 725),
  #("NY", 400),
  #("TX", 625),
  #("WA", 650),
  #("OR", 0),
  #("MT", 0),
]

const default_rate_bps = 500

/// States that tax shipping charges as well as goods.
const shipping_taxed_in = ["NY", "TX", "WA"]

pub fn rate_bps(region: Region) -> Int {
  case region {
    Domestic(state) ->
      list.key_find(state_rates, state) |> result.unwrap(default_rate_bps)
    International(_) -> 0
  }
}

pub fn taxes_shipping(region: Region) -> Bool {
  case region {
    Domestic(state) -> list.contains(shipping_taxed_in, state)
    International(_) -> False
  }
}

/// Tax on goods (after discounts) and, where taxed, shipping.
pub fn quote(
  region: Region,
  goods_cents: Int,
  shipping_cents: Int,
) -> TaxQuote {
  let taxable = case taxes_shipping(region) {
    True -> goods_cents + shipping_cents
    False -> goods_cents
  }
  let rate = rate_bps(region)
  TaxQuote(
    rate_bps: rate,
    taxable_cents: taxable,
    tax_cents: money.basis_points(taxable, rate),
  )
}
