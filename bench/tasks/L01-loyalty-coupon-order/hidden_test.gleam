import pilot/checkout
import pilot/clock
import pilot/seed
import pilot/shipping.{Standard}

fn quote_with(customer_id, code) {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), c, customer_id)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "LAMP-OAK", 1)
  let assert Ok(#(db, _)) = checkout.apply_coupon(db, c, o.id, code)
  let assert Ok(q) = checkout.quote(db, o.id, Standard)
  q
}

pub fn loyalty_before_fixed_coupon_test() {
  let q = quote_with(seed.grace, "TENOFF")
  assert q.loyalty_discount_cents == 370
  assert q.coupon_discount_cents == 1000
  assert q.tax_cents == 437
  assert q.total_cents == 7067
}

pub fn percent_coupon_after_loyalty_test() {
  let q = quote_with(seed.grace, "SAVE10")
  assert q.loyalty_discount_cents == 370
  assert q.coupon_discount_cents == 703
  assert q.total_cents == 6327 + 600 + 459
}

pub fn non_stacking_coupon_test() {
  let q = quote_with(seed.grace, "WELCOME15")
  assert q.loyalty_discount_cents == 0
  assert q.coupon_discount_cents == 1110
}
