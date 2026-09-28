import gleam/dict
import gleam/string
import pilot/api
import pilot/checkout
import pilot/clock
import pilot/fulfillment
import pilot/inventory.{Allocation}
import pilot/order
import pilot/payments
import pilot/seed
import pilot/shipments.{Item}
import pilot/shipping.{Standard}
import pilot/store

fn at(day) {
  clock.fixed(seed.epoch + day * seed.day)
}

fn paid(sku, quantity) {
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), at(0), seed.ada)
  let assert Ok(#(db, _)) = checkout.add_line(db, at(0), o.id, sku, quantity)
  let assert Ok(#(db, _)) = checkout.submit(db, at(0), o.id)
  let assert Ok(#(db, _)) =
    checkout.pay(db, at(0), payments.fake_gateway(), o.id, "tok_ok", Standard)
  #(db, o.id)
}

fn status(db, id) {
  let assert Ok(o) = store.get_order(db, id)
  o.status
}

pub fn split_shipments_lifecycle_test() {
  // 10 hoodies: 8 at WH-EAST, 5 at WH-WEST; no single warehouse has 10
  let #(db, id) = paid("HOOD-M-GRY", 10)
  let g = payments.fake_gateway()
  let assert Ok(#(db, [east, west])) = fulfillment.start_picking(db, at(1), id, Standard)
  assert east.warehouse_id == "WH-EAST"
  assert east.items == [Item("HOOD-M-GRY", 8)]
  assert west.warehouse_id == "WH-WEST"
  assert west.items == [Item("HOOD-M-GRY", 2)]
  assert east.id != west.id
  assert store.get_shipment(db, east.id) == Ok(east)
  assert store.get_shipment(db, west.id) == Ok(west)
  let #(db, next) = store.next_id(db)
  assert next != east.id && next != west.id

  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), east.id)
  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), west.id)
  let assert Ok(#(db, _)) = fulfillment.ship(db, at(1), g, east.id)
  assert status(db, id) == order.Picking
  let assert Ok(p) = store.payment_for_order(db, id)
  assert p.status == payments.Captured
  assert store.allocations_for(db, id) == [Allocation("WH-WEST", "HOOD-M-GRY", 2)]
  assert dict.get(db.stock.on_hand, #("WH-EAST", "HOOD-M-GRY")) == Ok(0)

  let assert Ok(#(db, _)) = fulfillment.ship(db, at(2), g, west.id)
  assert status(db, id) == order.Shipped
  assert store.allocations_for(db, id) == []
  assert dict.get(db.stock.on_hand, #("WH-WEST", "HOOD-M-GRY")) == Ok(3)

  let assert Ok(#(db, _)) = fulfillment.deliver(db, at(4), east.id)
  assert status(db, id) == order.Shipped
  let assert Ok(#(db, _)) = fulfillment.deliver(db, at(5), west.id)
  assert status(db, id) == order.Delivered
}

pub fn single_warehouse_still_one_shipment_test() {
  let #(db, id) = paid("MUG-BLUE", 2)
  let assert Ok(#(db, [s])) = fulfillment.start_picking(db, at(1), id, Standard)
  let assert Ok(#(db, _)) = fulfillment.pack(db, at(1), s.id)
  let assert Ok(#(db, _)) = fulfillment.ship(db, at(1), payments.fake_gateway(), s.id)
  assert status(db, id) == order.Shipped
}

pub fn api_returns_array_test() {
  let #(db, id) = paid("HOOD-M-GRY", 10)
  let env = api.Env(clock: at(1), gateway: payments.fake_gateway())
  let #(_, response) = api.handle(db, env, api.StartPicking(id, Standard))
  assert response.status == 201
  assert string.starts_with(response.body, "[{")
  assert string.contains(response.body, "WH-WEST")
}
