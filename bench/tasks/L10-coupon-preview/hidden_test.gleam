import gleam/option.{None}
import pilot/api.{Env, PreviewCoupon, Response}
import pilot/checkout
import pilot/clock
import pilot/payments
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

fn env() {
  Env(clock: clock.fixed(seed.epoch), gateway: payments.fake_gateway())
}

fn draft() {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), c, seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, "MUG-BLUE", 2)
  #(db, o.id)
}

pub fn preview_test() {
  let #(db, id) = draft()
  let #(after, response) = api.handle(db, env(), PreviewCoupon(id, "TENOFF", Standard))
  assert response
    == Response(
      200,
      "{\"subtotal_cents\":2800,\"discount_cents\":1000,\"shipping_cents\":600,\"tax_cents\":96,\"total_cents\":2496}",
    )
  assert after == db
  let assert Ok(o) = store.get_order(after, id)
  assert o.coupon_code == None
}

pub fn errors_match_apply_coupon_test() {
  let #(db, id) = draft()
  let #(after, response) = api.handle(db, env(), PreviewCoupon(id, "SPRING", Standard))
  assert response == Response(422, "{\"error\":\"coupon_expired\",\"message\":\"SPRING has expired\"}")
  assert after == db
  let #(_, response) = api.handle(db, env(), PreviewCoupon(id, "NOPE", Standard))
  assert response == Response(404, "{\"error\":\"not_found\",\"message\":\"No coupon NOPE\"}")
  let #(_, response) = api.handle(db, env(), PreviewCoupon(999, "TENOFF", Standard))
  assert response.status == 404
}
