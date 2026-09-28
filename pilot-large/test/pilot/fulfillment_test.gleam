import gleam/dict
import gleam/list
import gleam/option.{Some}
import gleam/string
import pilot/fulfillment
import pilot/inventory.{Allocation}
import pilot/notifications
import pilot/order
import pilot/payments
import pilot/seed
import pilot/shipments
import pilot/shipping.{Standard}
import pilot/store
import support

pub fn ship_and_deliver_test() {
  let #(db, o) =
    support.paid(seed.demo_store(), seed.ada, [
      #("TEE-M-BLK", 2),
      #("MUG-BLUE", 1),
    ])
  let #(db, s) = support.shipped(db, o.id)
  assert s.warehouse_id == "WH-EAST"
  assert s.tracking_number == Some(shipments.tracking_number(s))
  let assert Ok(o) = store.get_order(db, o.id)
  assert o.status == order.Shipped
  // shipped units leave the warehouse and the reservation
  assert inventory.available(db.stock, "WH-EAST", "TEE-M-BLK") == 18
  assert dict.get(db.stock.on_hand, #("WH-EAST", "TEE-M-BLK")) == Ok(18)
  assert store.allocations_for(db, o.id) == []
  let assert Ok(p) = store.payment_for_order(db, o.id)
  assert p.status == payments.Captured
  let assert Ok(#(db, _)) = fulfillment.deliver(db, support.on_day(4), s.id)
  let assert Ok(o) = store.get_order(db, o.id)
  assert o.status == order.Delivered
  let kinds = store.sent_messages(db) |> list.map(fn(m) { m.kind })
  assert kinds
    == [
      notifications.OrderConfirmation,
      notifications.OrderShipped,
      notifications.OrderDelivered,
    ]
}

pub fn shipped_email_has_estimate_test() {
  // Shipped on day 1 (Tuesday 2026-03-03) from NJ to NY: 4 business days
  let #(db, o) = support.paid(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)])
  let #(db, _) = support.shipped(db, o.id)
  let assert Ok(message) =
    list.find(store.sent_messages(db), fn(m) {
      m.kind == notifications.OrderShipped
    })
  assert string.contains(message.body, "2026-03-09")
}

pub fn consolidates_split_reservations_test() {
  // Grace is in CA: headphones reserve at WH-WEST, the large tee only exists at WH-EAST
  let #(db, o) =
    support.paid(seed.demo_store(), seed.grace, [
      #("PHONES-BLK", 1),
      #("TEE-L-BLK", 1),
    ])
  assert store.allocations_for(db, o.id)
    == [
      Allocation("WH-WEST", "PHONES-BLK", 1),
      Allocation("WH-EAST", "TEE-L-BLK", 1),
    ]
  let assert Ok(#(db, s)) =
    fulfillment.start_picking(db, support.on_day(1), o.id, Standard)
  assert s.warehouse_id == "WH-EAST"
  assert inventory.available(db.stock, "WH-WEST", "PHONES-BLK") == 3
  assert inventory.available(db.stock, "WH-EAST", "PHONES-BLK") == 4
}

pub fn cannot_consolidate_test() {
  // 10 hoodies: 8 at WH-EAST, 5 at WH-WEST, so no single warehouse has them all
  let #(db, o) =
    support.paid(seed.demo_store(), seed.ada, [#("HOOD-M-GRY", 10)])
  assert fulfillment.start_picking(db, support.on_day(1), o.id, Standard)
    == Error(
      fulfillment.ShipmentFailed(
        shipments.MultipleWarehouses(["WH-EAST", "WH-WEST"]),
      ),
    )
}

pub fn report_shortage_moves_reservation_test() {
  // All 4 lamps at WH-EAST are reserved, so the replacement comes from WH-WEST
  let #(db, o) = support.paid(seed.demo_store(), seed.ada, [#("LAMP-OAK", 4)])
  assert store.allocations_for(db, o.id)
    == [Allocation("WH-EAST", "LAMP-OAK", 4)]
  let assert Ok(db) =
    fulfillment.report_shortage(
      db,
      support.on_day(1),
      o.id,
      "WH-EAST",
      "LAMP-OAK",
      1,
    )
  assert inventory.merge_allocations(store.allocations_for(db, o.id))
    == [
      Allocation("WH-EAST", "LAMP-OAK", 3),
      Allocation("WH-WEST", "LAMP-OAK", 1),
    ]
  assert dict.get(db.stock.on_hand, #("WH-EAST", "LAMP-OAK")) == Ok(3)
  assert fulfillment.report_shortage(
      db,
      support.on_day(1),
      o.id,
      "WH-SOUTH",
      "LAMP-OAK",
      1,
    )
    == Error(fulfillment.NotAllocated(o.id, "WH-SOUTH", "LAMP-OAK"))
}

pub fn must_be_paid_to_pick_test() {
  let #(db, o) = support.draft(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)])
  let assert Error(fulfillment.OrderRejected(_)) =
    fulfillment.start_picking(db, support.on_day(1), o.id, Standard)
}
