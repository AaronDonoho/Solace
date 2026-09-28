import pilot/customer.{Domestic, International}
import pilot/order.{Line}
import pilot/pricing.{Quote}
import pilot/product.{Product}

fn lines(price: Int, quantity: Int) {
  [#(Line("A", quantity), Product("A", "Widget", price, 100))]
}

pub fn small_domestic_order_pays_shipping_and_tax_test() {
  assert pricing.quote(lines(1000, 2), Domestic)
    == Quote(
      subtotal_cents: 2000,
      discount_cents: 0,
      shipping_cents: 500,
      tax_cents: 160,
      total_cents: 2660,
    )
}

pub fn free_shipping_at_threshold_test() {
  let q = pricing.quote(lines(5000, 1), Domestic)
  assert q.shipping_cents == 0
  let q = pricing.quote(lines(4999, 1), Domestic)
  assert q.shipping_cents == 500
}

pub fn discount_at_threshold_test() {
  let q = pricing.quote(lines(10_000, 1), Domestic)
  assert q.discount_cents == 1000
  assert q.total_cents == 9000 + 720
  let q = pricing.quote(lines(9999, 1), Domestic)
  assert q.discount_cents == 0
}

pub fn international_flat_shipping_no_tax_test() {
  assert pricing.quote(lines(20_000, 1), International)
    == Quote(
      subtotal_cents: 20_000,
      discount_cents: 2000,
      shipping_cents: 2000,
      tax_cents: 0,
      total_cents: 20_000,
    )
}
