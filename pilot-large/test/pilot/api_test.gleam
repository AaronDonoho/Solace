import gleam/string
import pilot/api.{
  AddItem, ApplyCoupon, CreateOrder, Env, GetOrder, GetQuote, ListOrders,
  PayOrder, Response, SalesReport, SearchProducts, SubmitOrder,
}
import pilot/date.{Date}
import pilot/payments
import pilot/seed
import pilot/shipping.{Standard}
import support

fn env() {
  Env(clock: support.on_day(0), gateway: payments.fake_gateway())
}

pub fn order_flow_test() {
  let #(db, r) = api.handle(seed.demo_store(), env(), CreateOrder(seed.ada))
  assert r.status == 201
  assert string.contains(r.body, "\"id\":100")
  let #(db, r) = api.handle(db, env(), AddItem(100, "MUG-BLUE", 2))
  assert r.status == 200
  let #(db, r) = api.handle(db, env(), GetQuote(100, Standard))
  assert r.body
    == "{\"subtotal_cents\":2800,\"discount_cents\":0,\"shipping_cents\":600,\"tax_cents\":136,\"total_cents\":3536}"
  let #(db, _) = api.handle(db, env(), SubmitOrder(100))
  let #(db, r) = api.handle(db, env(), PayOrder(100, "tok_ok", Standard))
  assert r.status == 200
  assert string.contains(r.body, "\"status\":\"paid\"")
  let #(_, r) = api.handle(db, env(), ListOrders(seed.ada))
  assert string.starts_with(r.body, "[{\"id\":100")
}

pub fn errors_test() {
  let db = seed.demo_store()
  assert api.handle(db, env(), GetOrder(5)).1
    == Response(404, "{\"error\":\"not_found\",\"message\":\"No order 5\"}")
  let #(db, _) = api.handle(db, env(), CreateOrder(seed.ada))
  assert { api.handle(db, env(), AddItem(100, "MUG-BLUE", 0)).1 }.status == 422
  assert { api.handle(db, env(), ApplyCoupon(100, "SPRING")).1 }.body
    == "{\"error\":\"coupon_expired\",\"message\":\"SPRING has expired\"}"
  assert { api.handle(db, env(), PayOrder(100, "tok_ok", Standard)).1 }.status
    == 409
  let #(db, _) = api.handle(db, env(), AddItem(100, "MUG-BLUE", 1))
  let #(db, _) = api.handle(db, env(), SubmitOrder(100))
  assert { api.handle(db, env(), PayOrder(100, "tok_decline", Standard)).1 }.status
    == 402
  assert { api.handle(db, env(), PayOrder(100, "tok_network", Standard)).1 }.status
    == 503
}

pub fn search_and_report_test() {
  let db = seed.demo_store()
  assert { api.handle(db, env(), SearchProducts("mug")).1 }.body
    == "[{\"id\":\"mug\",\"name\":\"Stoneware Mug\",\"category\":\"Home goods\",\"in_stock\":true}]"
  assert { api.handle(db, env(), SalesReport("2026-03-02", "2026-03-02")).1 }.body
    == "[{\"date\":\"2026-03-02\",\"orders\":0,\"net_cents\":0}]"
  assert { api.handle(db, env(), SalesReport("March", "2026-03-02")).1 }.status
    == 422
}

pub fn parsing_test() {
  assert api.parse_date("2026-03-02") == Ok(Date(2026, 3, 2))
  assert api.parse_date("2026-13-02") == Error(Nil)
  assert api.parse_reason(" Damaged ") == api.parse_reason("damaged")
}
