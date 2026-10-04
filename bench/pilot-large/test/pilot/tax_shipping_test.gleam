import pilot/customer.{Domestic, International}
import pilot/date.{Date}
import pilot/shipping.{
  Express, Local, National, Overseas, ShippingQuote, Standard,
}
import pilot/tax.{TaxQuote}

pub fn tax_rates_test() {
  assert tax.rate_bps(Domestic("CA")) == 725
  assert tax.rate_bps(Domestic("OR")) == 0
  assert tax.rate_bps(Domestic("NJ")) == 500
  assert tax.rate_bps(International("DE")) == 0
}

pub fn tax_on_shipping_only_where_taxed_test() {
  assert tax.quote(Domestic("NY"), 5000, 600) == TaxQuote(400, 5600, 224)
  assert tax.quote(Domestic("CA"), 7030, 600) == TaxQuote(725, 7030, 510)
  assert tax.quote(International("DE"), 5000, 1800) == TaxQuote(0, 5000, 0)
}

pub fn zones_test() {
  assert shipping.zone("CA", Domestic("CA")) == Local
  assert shipping.zone("NJ", Domestic("NY")) == National
  assert shipping.zone("NJ", International("DE")) == Overseas
}

pub fn cost_by_weight_test() {
  assert shipping.cost_cents(Standard, National, 1000) == 600
  assert shipping.cost_cents(Standard, National, 1001) == 750
  assert shipping.cost_cents(Standard, Local, 2600) == 600
  assert shipping.cost_cents(Express, Overseas, 3000) == 4200 + 1800
}

pub fn free_standard_shipping_test() {
  assert shipping.quote(Standard, "NJ", Domestic("NY"), 850, 7500)
    == ShippingQuote(Standard, National, 0, 4)
  assert { shipping.quote(Standard, "NJ", Domestic("NY"), 850, 7499) }.cost_cents
    == 600
  assert { shipping.quote(Express, "NJ", Domestic("NY"), 850, 9000) }.cost_cents
    == 1700
  assert { shipping.quote(Standard, "NJ", International("DE"), 850, 9000) }.cost_cents
    == 1800
}

pub fn estimate_delivery_test() {
  // Shipped Thursday 2026-03-05, national standard: 4 business days
  let q = shipping.quote(Standard, "NJ", Domestic("NY"), 850, 0)
  assert shipping.estimate_delivery(Date(2026, 3, 5), q) == Date(2026, 3, 11)
}
