import gleam/option.{None, Some}
import pilot/catalog
import pilot/checkout
import pilot/clock
import pilot/promotions.{Coupon, FixedOff, PercentOff}
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

fn books(code, kind, min) {
  Coupon(
    code:,
    kind:,
    min_spend_cents: min,
    expires_at: None,
    max_uses_per_customer: None,
    stacks_with_loyalty: True,
    active: True,
    category: Some(catalog.Books),
  )
}

fn db() {
  seed.demo_store()
  |> store.put_coupon(books("BOOKS20", PercentOff(20), 0))
  |> store.put_coupon(books("BOOKS3000", FixedOff(500), 3000))
  |> store.put_coupon(books("BOOKS50", FixedOff(5000), 0))
}

fn draft(lines) {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(db(), c, seed.ada)
  let db = add(db, o.id, lines)
  #(db, o.id)
}

fn add(db, id, lines) {
  case lines {
    [] -> db
    [#(sku, q), ..rest] -> {
      let assert Ok(#(db, _)) = checkout.add_line(db, clock.fixed(seed.epoch), id, sku, q)
      add(db, id, rest)
    }
  }
}

fn coupon_discount(lines, code) {
  let #(db, id) = draft(lines)
  let assert Ok(#(db, _)) = checkout.apply_coupon(db, clock.fixed(seed.epoch), id, code)
  let assert Ok(q) = checkout.quote(db, id, Standard)
  q.coupon_discount_cents
}

pub fn percent_applies_to_category_lines_only_test() {
  assert coupon_discount([#("BOOK-NOVEL", 2), #("MUG-BLUE", 1)], "BOOKS20") == 480
}

pub fn needs_a_line_in_the_category_test() {
  let #(db, id) = draft([#("MUG-BLUE", 1)])
  let assert Error(checkout.CouponRejected(promotions.NotApplicable("BOOKS20", _))) =
    checkout.apply_coupon(db, clock.fixed(seed.epoch), id, "BOOKS20")
}

pub fn minimum_spend_counts_category_lines_only_test() {
  let #(db, id) = draft([#("BOOK-NOVEL", 2), #("MUG-BLUE", 1)])
  assert checkout.apply_coupon(db, clock.fixed(seed.epoch), id, "BOOKS3000")
    == Error(checkout.CouponRejected(promotions.BelowMinimumSpend("BOOKS3000", 3000)))
}

pub fn fixed_off_capped_at_category_total_test() {
  assert coupon_discount([#("BOOK-NOVEL", 1), #("LAMP-OAK", 1)], "BOOKS50") == 1200
}

pub fn whole_order_coupons_unchanged_test() {
  assert coupon_discount([#("TEE-M-BLK", 2), #("MUG-BLUE", 1)], "SAVE10") == 500
  assert coupon_discount([#("MUG-BLUE", 1)], "TENOFF") == 1000
}
