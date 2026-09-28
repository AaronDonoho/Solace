import pilot/checkout
import pilot/date.{Date}
import pilot/payments
import pilot/reporting.{DailySales, ProductSales, RegionTax, ReturnsByReason}
import pilot/return_policy.{Item, NoLongerNeeded}
import pilot/returns
import pilot/seed
import support

pub fn daily_sales_test() {
  let #(db, _) =
    support.paid(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  let #(db, _) = support.paid(db, seed.grace, [#("LAMP-OAK", 1)])
  let #(db, cancelled) = support.paid(db, seed.ada, [#("MUG-BLUE", 1)])
  let assert Ok(#(db, _)) =
    checkout.cancel(db, support.on_day(0), cancelled.id, "x")
  assert reporting.daily_sales(db, Date(2026, 3, 1), Date(2026, 3, 3))
    == [
      DailySales(Date(2026, 3, 1), 0, 0, 0, 0, 0, 0),
      DailySales(Date(2026, 3, 2), 2, 12_400, 370, 1200, 734, 13_964),
      DailySales(Date(2026, 3, 3), 0, 0, 0, 0, 0, 0),
    ]
}

pub fn top_products_test() {
  let #(db, _) =
    support.paid(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  let #(db, _) = support.paid(db, seed.barbara, [#("MUG-BLUE", 3)])
  assert reporting.top_products(db, 2)
    == [ProductSales("MUG-BLUE", 4, 5600), ProductSales("TEE-M-BLK", 2, 3600)]
}

pub fn tax_by_region_test() {
  let #(db, _) =
    support.paid(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  let #(db, _) = support.paid(db, seed.grace, [#("LAMP-OAK", 1)])
  let #(db, _) = support.paid(db, seed.linus, [#("MUG-BLUE", 1)])
  assert reporting.tax_by_region(db, Date(2026, 3, 2), Date(2026, 3, 2))
    == [
      RegionTax("DE", 1, 0),
      RegionTax("US-CA", 1, 510),
      RegionTax("US-NY", 1, 224),
    ]
}

pub fn returns_by_reason_test() {
  let #(db, o, _) =
    support.delivered(
      seed.demo_store(),
      seed.ada,
      [#("TEE-M-BLK", 2), #("MUG-BLUE", 1)],
      3,
    )
  let c = support.on_day(5)
  let assert Ok(#(db, r)) =
    returns.request(db, c, o.id, [Item("TEE-M-BLK", 1)], NoLongerNeeded)
  let assert Ok(#(db, _)) = returns.approve(db, c, r.id)
  let assert Ok(#(db, _)) = returns.receive(db, c, r.id)
  let assert Ok(#(db, _)) = returns.refund(db, c, payments.fake_gateway(), r.id)
  assert reporting.returns_by_reason(db)
    == [ReturnsByReason("no longer needed", 1, 1880)]
  assert reporting.total_refunded_cents(db) == 1880
}

pub fn inventory_reports_test() {
  let db = seed.demo_store()
  assert reporting.inventory_valuation_cents(db) > 0
  assert reporting.low_stock_report(db, 2) == ["WH-WEST LAMP-OAK: 2 left"]
}
