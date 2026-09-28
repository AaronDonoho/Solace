import gleam/option.{None}
import pilot/accounts
import pilot/checkout
import pilot/clock
import pilot/customer.{Address, Customer, Domestic}
import pilot/date.{Date}
import pilot/order.{Line}
import pilot/payments
import pilot/pricing
import pilot/reporting.{RegionTax}
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

const school = 50

fn db() {
  seed.demo_store()
  |> store.put_customer(Customer(
    id: school,
    name: "Riverside School",
    email: "office@riverside.example",
    shipping_address: Address("5 School Rd", "Albany", "12208", Domestic("NY")),
    lifetime_spend_cents: 0,
    created_at: seed.epoch,
    tax_exempt: True,
  ))
}

fn order_for(db, customer_id) {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(db, c, customer_id)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "TEE-M-BLK", 2)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "MUG-BLUE", 1)
  #(db, o.id)
}

pub fn exempt_customers_pay_no_tax_test() {
  let #(db, id) = order_for(db(), school)
  let assert Ok(q) = checkout.quote(db, id, Standard)
  assert q.tax_cents == 0
  assert q.total_cents == 5600
  let #(db, id) = order_for(db, seed.ada)
  let assert Ok(q) = checkout.quote(db, id, Standard)
  assert q.tax_cents == 224
}

pub fn pricing_flag_test() {
  let ship = shipping.ShippingQuote(Standard, shipping.National, 600, 4)
  let q = pricing.quote([Line("A", 1, 5000)], customer.Standard, None, ship, Domestic("NY"), True)
  assert q.tax_cents == 0
  let q = pricing.quote([Line("A", 1, 5000)], customer.Standard, None, ship, Domestic("NY"), False)
  assert q.tax_cents == 224
}

pub fn tax_report_counts_exempt_orders_test() {
  let c = clock.fixed(seed.epoch)
  let pay = fn(db, id) {
    let assert Ok(#(db, _)) = checkout.submit(db, c, id)
    let assert Ok(#(db, _)) = checkout.pay(db, c, payments.fake_gateway(), id, "tok_ok", Standard)
    db
  }
  let #(db, a) = order_for(db(), school)
  let db = pay(db, a)
  let #(db, b) = order_for(db, seed.ada)
  let db = pay(db, b)
  assert reporting.tax_by_region(db, Date(2026, 3, 2), Date(2026, 3, 2))
    == [RegionTax("US-NY", 2, 224, 1)]
}

pub fn new_accounts_are_not_exempt_test() {
  let assert Ok(#(_, c)) =
    accounts.register(db(), clock.fixed(seed.epoch), "Hedy", "hedy@example.com", Address("1 A St", "Austin", "73301", Domestic("TX")))
  assert !c.tax_exempt
}
