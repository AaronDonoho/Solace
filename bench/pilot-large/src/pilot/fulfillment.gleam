//// Fulfillment: picking, packing, shipping and delivering paid orders.

import gleam/dict
import gleam/int
import gleam/list
import gleam/result
import pilot/audit
import pilot/checkout
import pilot/clock.{type Clock}
import pilot/date
import pilot/inventory.{type Allocation, type InventoryError}
import pilot/notifications
import pilot/order.{type Order, type OrderError}
import pilot/payments.{type Gateway, type PaymentError}
import pilot/shipments.{type Shipment, type ShipmentError}
import pilot/shipping.{type Method}
import pilot/store.{type Store}

pub type FulfillmentError {
  UnknownOrder(id: Int)
  UnknownShipment(id: Int)
  UnknownCustomer(id: Int)
  OrderRejected(OrderError)
  ShipmentFailed(ShipmentError)
  InventoryFailed(InventoryError)
  PaymentFailed(PaymentError)
  MissingPayment(order_id: Int)
  NotAllocated(order_id: Int, warehouse_id: String, sku: String)
}

const actor = "fulfillment"

/// Starts picking a paid order and creates its shipment. Orders ship from a
/// single warehouse: if the order's stock is reserved across several, it is
/// moved to one warehouse that can supply everything, when there is one.
pub fn start_picking(
  db: Store,
  clock: Clock,
  order_id: Int,
  method: Method,
) -> Result(#(Store, Shipment), FulfillmentError) {
  let now = clock()
  use existing <- result.try(get_order(db, order_id))
  use picking <- result.try(
    order.transition(existing, order.StartPicking, now)
    |> result.map_error(OrderRejected),
  )
  let allocations = store.allocations_for(db, order_id)
  use #(db, allocations) <- result.try(
    case shipments.from_allocations(0, order_id, allocations, method) {
      Error(shipments.MultipleWarehouses(_)) ->
        consolidate(db, order_id, allocations)
      _ -> Ok(#(db, allocations))
    },
  )
  let #(db, shipment_id) = store.next_id(db)
  use shipment <- result.try(
    shipments.from_allocations(shipment_id, order_id, allocations, method)
    |> result.map_error(ShipmentFailed),
  )
  let db =
    db
    |> store.put_order(picking)
    |> store.put_shipment(shipment)
    |> store.record(
      now,
      actor,
      "start picking",
      audit.order_subject(order_id),
      audit.shipment_subject(shipment_id) <> " from " <> shipment.warehouse_id,
    )
  Ok(#(db, shipment))
}

/// Moves an order's reservations to a single warehouse that has enough of
/// every item, counting what the order already holds there.
fn consolidate(
  db: Store,
  order_id: Int,
  allocations: List(Allocation),
) -> Result(#(Store, List(Allocation)), FulfillmentError) {
  let needed =
    inventory.merge_allocations(
      list.map(allocations, fn(a) {
        inventory.Allocation("", a.sku, a.quantity)
      }),
    )
  let released = inventory.release(db.stock, allocations)
  let candidate =
    list.find(db.warehouses, fn(w) {
      list.all(needed, fn(n) {
        inventory.available(released, w.id, n.sku) >= n.quantity
      })
    })
  case candidate {
    Error(Nil) ->
      Error(
        ShipmentFailed(shipments.MultipleWarehouses(
          allocations |> list.map(fn(a) { a.warehouse_id }) |> list.unique,
        )),
      )
    Ok(warehouse) -> {
      use #(stock, moved) <- result.try(
        list.try_fold(needed, #(released, []), fn(acc, n) {
          inventory.reserve_at(acc.0, warehouse.id, n.sku, n.quantity)
          |> result.map(fn(r) { #(r.0, [r.1, ..acc.1]) })
        })
        |> result.map_error(InventoryFailed),
      )
      let moved = list.reverse(moved)
      Ok(#(
        db |> store.set_stock(stock) |> store.set_allocations(order_id, moved),
        moved,
      ))
    }
  }
}

pub fn pack(
  db: Store,
  clock: Clock,
  shipment_id: Int,
) -> Result(#(Store, Shipment), FulfillmentError) {
  use shipment <- result.try(get_shipment(db, shipment_id))
  use packed <- result.try(
    shipments.pack(shipment) |> result.map_error(ShipmentFailed),
  )
  let db =
    db
    |> store.put_shipment(packed)
    |> store.record(
      clock(),
      actor,
      "pack",
      audit.shipment_subject(shipment_id),
      "",
    )
  Ok(#(db, packed))
}

/// Sends a packed shipment: stock leaves the warehouse, the payment is
/// captured, the order is marked shipped and the customer is told.
pub fn ship(
  db: Store,
  clock: Clock,
  gateway: Gateway,
  shipment_id: Int,
) -> Result(#(Store, Shipment), FulfillmentError) {
  let now = clock()
  use shipment <- result.try(get_shipment(db, shipment_id))
  use existing <- result.try(get_order(db, shipment.order_id))
  use customer <- result.try(
    store.get_customer(db, existing.customer_id)
    |> result.replace_error(UnknownCustomer(existing.customer_id)),
  )
  use sent <- result.try(
    shipments.dispatch(shipment, now) |> result.map_error(ShipmentFailed),
  )
  use payment <- result.try(
    store.payment_for_order(db, existing.id)
    |> result.replace_error(MissingPayment(existing.id)),
  )
  use payment <- result.try(case payment.status {
    payments.Authorized ->
      payments.capture(gateway, payment) |> result.map_error(PaymentFailed)
    _ -> Ok(payment)
  })
  use shipped <- result.try(
    order.transition(existing, order.Ship, now)
    |> result.map_error(OrderRejected),
  )
  let origin = case
    inventory.find_warehouse(db.warehouses, shipment.warehouse_id)
  {
    Ok(w) -> w.state
    Error(_) -> ""
  }
  let estimate =
    shipping.estimate_delivery(
      date.from_unix_ms(now),
      shipping.quote(
        shipment.method,
        origin,
        existing.shipping_address.region,
        0,
        0,
      ),
    )
  let db =
    db
    |> store.set_stock(inventory.commit(
      db.stock,
      shipments.allocations(shipment),
    ))
    |> store.clear_allocations(existing.id)
    |> store.put_payment(payment)
    |> store.put_shipment(sent)
    |> store.put_order(shipped)
    |> store.send(notifications.shipped(customer, sent, estimate, now))
    |> store.record(
      now,
      actor,
      "ship",
      audit.shipment_subject(shipment_id),
      "estimated delivery " <> date.to_iso(estimate),
    )
  Ok(#(db, sent))
}

/// Records delivery of a shipment and of its order.
pub fn deliver(
  db: Store,
  clock: Clock,
  shipment_id: Int,
) -> Result(#(Store, Shipment), FulfillmentError) {
  let now = clock()
  use shipment <- result.try(get_shipment(db, shipment_id))
  use existing <- result.try(get_order(db, shipment.order_id))
  use customer <- result.try(
    store.get_customer(db, existing.customer_id)
    |> result.replace_error(UnknownCustomer(existing.customer_id)),
  )
  use delivered <- result.try(
    shipments.deliver(shipment, now) |> result.map_error(ShipmentFailed),
  )
  use delivered_order <- result.try(
    order.transition(existing, order.Deliver, now)
    |> result.map_error(OrderRejected),
  )
  let db =
    db
    |> store.put_shipment(delivered)
    |> store.put_order(delivered_order)
    |> store.send(notifications.delivered(customer, delivered_order, now))
    |> store.record(
      now,
      actor,
      "deliver",
      audit.shipment_subject(shipment_id),
      "",
    )
  Ok(#(db, delivered))
}

/// Records that units reserved for an order at a warehouse are missing or
/// damaged. They are written off, and the order's reservation is replaced
/// from other stock.
pub fn report_shortage(
  db: Store,
  clock: Clock,
  order_id: Int,
  warehouse_id: String,
  sku: String,
  quantity: Int,
) -> Result(Store, FulfillmentError) {
  let now = clock()
  use existing <- result.try(get_order(db, order_id))
  let allocations = store.allocations_for(db, order_id)
  let held =
    allocations
    |> list.filter(fn(a) { a.warehouse_id == warehouse_id && a.sku == sku })
    |> list.fold(0, fn(sum, a) { sum + a.quantity })
  case held >= quantity && quantity > 0 {
    False -> Error(NotAllocated(order_id, warehouse_id, sku))
    True -> {
      let short = inventory.Allocation(warehouse_id, sku, quantity)
      let stock = inventory.release(db.stock, [short])
      use stock <- result.try(
        inventory.write_off(stock, warehouse_id, sku, quantity)
        |> result.map_error(InventoryFailed),
      )
      use #(stock, replacement) <- result.try(
        inventory.reserve(
          stock,
          db.warehouses,
          sku,
          quantity,
          checkout.destination_state(existing.shipping_address.region),
        )
        |> result.map_error(InventoryFailed),
      )
      let remaining = subtract(allocations, short)
      let db =
        db
        |> store.set_stock(stock)
        |> store.set_allocations(order_id, list.append(remaining, replacement))
        |> store.record(
          now,
          actor,
          "shortage",
          audit.order_subject(order_id),
          int.to_string(quantity) <> " x " <> sku <> " at " <> warehouse_id,
        )
      Ok(db)
    }
  }
}

/// Removes `short` units from matching allocations.
fn subtract(
  allocations: List(Allocation),
  short: Allocation,
) -> List(Allocation) {
  let #(kept, _) =
    list.fold(allocations, #([], short.quantity), fn(acc, a) {
      let #(kept, left) = acc
      case
        a.warehouse_id == short.warehouse_id && a.sku == short.sku && left > 0
      {
        True -> {
          let take = int.min(left, a.quantity)
          case a.quantity - take {
            0 -> #(kept, left - take)
            rest -> #(
              [inventory.Allocation(..a, quantity: rest), ..kept],
              left - take,
            )
          }
        }
        False -> #([a, ..kept], left)
      }
    })
  list.reverse(kept)
}

/// Units still to be shipped per SKU for an order.
pub fn outstanding(db: Store, order_id: Int) -> dict.Dict(String, Int) {
  store.allocations_for(db, order_id)
  |> list.fold(dict.new(), fn(acc, a) {
    dict.insert(
      acc,
      a.sku,
      { dict.get(acc, a.sku) |> result.unwrap(0) } + a.quantity,
    )
  })
}

fn get_order(db: Store, id: Int) -> Result(Order, FulfillmentError) {
  store.get_order(db, id) |> result.replace_error(UnknownOrder(id))
}

fn get_shipment(db: Store, id: Int) -> Result(Shipment, FulfillmentError) {
  store.get_shipment(db, id) |> result.replace_error(UnknownShipment(id))
}
