//// Business reports computed from the store.

import gleam/dict.{type Dict}
import gleam/int
import gleam/list
import gleam/option.{Some}
import gleam/order as ordering
import gleam/result
import gleam/string
import pilot/catalog
import pilot/customer
import pilot/date.{type Date}
import pilot/inventory
import pilot/money
import pilot/order.{type Order}
import pilot/pricing.{type Quote}
import pilot/return_policy
import pilot/store.{type Store}

pub type DailySales {
  DailySales(
    date: Date,
    orders: Int,
    gross_cents: Int,
    discounts_cents: Int,
    shipping_cents: Int,
    tax_cents: Int,
    net_cents: Int,
  )
}

pub type ProductSales {
  ProductSales(sku: String, units: Int, revenue_cents: Int)
}

pub type ReturnsByReason {
  ReturnsByReason(reason: String, returns: Int, refunded_cents: Int)
}

pub type RegionTax {
  RegionTax(region: String, orders: Int, tax_cents: Int)
}

/// Paid orders that were not cancelled, with the quote they were paid at
/// and the date they were paid.
fn sales(db: Store) -> List(#(Order, Quote, Date)) {
  db.orders
  |> dict.values
  |> list.filter_map(fn(o) {
    case o.status, o.paid_at, store.get_quote(db, o.id) {
      order.Cancelled(_), _, _ -> Error(Nil)
      _, Some(paid_at), Ok(quote) -> Ok(#(o, quote, date.from_unix_ms(paid_at)))
      _, _, _ -> Error(Nil)
    }
  })
}

fn in_range(d: Date, from: Date, to: Date) -> Bool {
  date.compare(d, from) != ordering.Lt && date.compare(d, to) != ordering.Gt
}

/// One row per day from `from` to `to` inclusive, including days with no sales.
pub fn daily_sales(db: Store, from: Date, to: Date) -> List(DailySales) {
  let by_day =
    sales(db)
    |> list.filter(fn(s) { in_range(s.2, from, to) })
    |> list.group(fn(s) { date.to_days(s.2) })
  int.range(
    from: date.to_days(to),
    to: date.to_days(from) - 1,
    with: [],
    run: list.prepend,
  )
  |> list.map(fn(day) {
    let rows = dict.get(by_day, day) |> result.unwrap([])
    let total = fn(f: fn(Quote) -> Int) {
      rows |> list.map(fn(r) { f(r.1) }) |> money.sum
    }
    DailySales(
      date: date.from_days(day),
      orders: list.length(rows),
      gross_cents: total(fn(q) { q.subtotal_cents }),
      discounts_cents: total(pricing.discounts_cents),
      shipping_cents: total(fn(q) { q.shipping_cents }),
      tax_cents: total(fn(q) { q.tax_cents }),
      net_cents: total(fn(q) { q.total_cents }),
    )
  })
}

/// Best-selling SKUs by units, then by SKU.
pub fn top_products(db: Store, limit: Int) -> List(ProductSales) {
  sales(db)
  |> list.flat_map(fn(s) { { s.1 }.lines })
  |> list.fold(dict.new(), fn(acc: Dict(String, ProductSales), l) {
    let current =
      dict.get(acc, l.sku) |> result.unwrap(ProductSales(l.sku, 0, 0))
    dict.insert(
      acc,
      l.sku,
      ProductSales(
        l.sku,
        current.units + l.quantity,
        current.revenue_cents + l.total_cents,
      ),
    )
  })
  |> dict.values
  |> list.sort(fn(a, b) {
    case int.compare(b.units, a.units) {
      ordering.Eq -> string.compare(a.sku, b.sku)
      other -> other
    }
  })
  |> list.take(limit)
}

/// Refunded returns grouped by reason, largest refund total first.
pub fn returns_by_reason(db: Store) -> List(ReturnsByReason) {
  db.returns
  |> dict.values
  |> list.filter(fn(r) { r.status == return_policy.Refunded })
  |> list.group(fn(r) { return_policy.reason_name(r.reason) })
  |> dict.to_list
  |> list.map(fn(pair) {
    ReturnsByReason(
      reason: pair.0,
      returns: list.length(pair.1),
      refunded_cents: money.sum(
        list.map(pair.1, fn(r) { r.refund.total_cents }),
      ),
    )
  })
  |> list.sort(fn(a, b) {
    case int.compare(b.refunded_cents, a.refunded_cents) {
      ordering.Eq -> string.compare(a.reason, b.reason)
      other -> other
    }
  })
}

/// Everything refunded through payments, from returns and goodwill refunds.
pub fn total_refunded_cents(db: Store) -> Int {
  db.payments
  |> dict.values
  |> list.map(fn(p) { p.refunded_cents })
  |> money.sum
}

/// Value of all stock on hand at current catalog prices.
pub fn inventory_valuation_cents(db: Store) -> Int {
  db.stock.on_hand
  |> dict.to_list
  |> list.map(fn(pair) {
    let #(#(_, sku), units) = pair
    case catalog.find_variant(db.products, sku) {
      Ok(#(_, variant)) -> units * variant.price_cents
      Error(_) -> 0
    }
  })
  |> money.sum
}

/// Tax collected per destination region for orders paid in the date range.
pub fn tax_by_region(db: Store, from: Date, to: Date) -> List(RegionTax) {
  sales(db)
  |> list.filter(fn(s) { in_range(s.2, from, to) })
  |> list.group(fn(s) { customer.region_name({ s.0 }.shipping_address.region) })
  |> dict.to_list
  |> list.map(fn(pair) {
    RegionTax(
      region: pair.0,
      orders: list.length(pair.1),
      tax_cents: money.sum(list.map(pair.1, fn(s) { { s.1 }.tax_cents })),
    )
  })
  |> list.sort(fn(a, b) { string.compare(a.region, b.region) })
}

/// Human-readable low-stock lines, e.g. `"WH-EAST TEE-M-BLK: 2 left"`.
pub fn low_stock_report(db: Store, threshold: Int) -> List(String) {
  inventory.low_stock(db.stock, threshold)
  |> list.map(fn(row) {
    row.0 <> " " <> row.1 <> ": " <> int.to_string(row.2) <> " left"
  })
}

/// A plain-text sales summary for the date range.
pub fn sales_summary(db: Store, from: Date, to: Date) -> String {
  let days = daily_sales(db, from, to)
  let sum = fn(f: fn(DailySales) -> Int) { days |> list.map(f) |> money.sum }
  "Sales "
  <> date.to_iso(from)
  <> " to "
  <> date.to_iso(to)
  <> "\n  Orders: "
  <> int.to_string(sum(fn(d) { d.orders }))
  <> "\n  Gross: "
  <> money.format(sum(fn(d) { d.gross_cents }))
  <> "\n  Discounts: "
  <> money.format(sum(fn(d) { d.discounts_cents }))
  <> "\n  Net: "
  <> money.format(sum(fn(d) { d.net_cents }))
  <> "\n"
}
