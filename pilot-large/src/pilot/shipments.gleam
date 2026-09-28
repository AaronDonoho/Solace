//// Shipments: the parcels an order leaves a warehouse in.

import gleam/int
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import pilot/inventory.{type Allocation}
import pilot/shipping.{type Method}

pub type ShipmentStatus {
  Pending
  Packed
  InTransit
  Delivered
}

pub type Item {
  Item(sku: String, quantity: Int)
}

pub type Shipment {
  Shipment(
    id: Int,
    order_id: Int,
    warehouse_id: String,
    items: List(Item),
    method: Method,
    status: ShipmentStatus,
    tracking_number: Option(String),
    shipped_at: Option(Int),
    delivered_at: Option(Int),
  )
}

pub type ShipmentError {
  /// Orders are currently shipped from a single warehouse.
  MultipleWarehouses(warehouse_ids: List(String))
  NothingToShip
  InvalidShipmentState(status: ShipmentStatus, action: String)
}

/// One shipment covering all of an order's allocations.
pub fn from_allocations(
  id: Int,
  order_id: Int,
  allocations: List(Allocation),
  method: Method,
) -> Result(Shipment, ShipmentError) {
  let warehouses =
    allocations |> list.map(fn(a) { a.warehouse_id }) |> list.unique
  case warehouses {
    [] -> Error(NothingToShip)
    [warehouse_id] ->
      Ok(Shipment(
        id:,
        order_id:,
        warehouse_id:,
        items: allocations
          |> inventory.merge_allocations
          |> list.map(fn(a) { Item(a.sku, a.quantity) }),
        method:,
        status: Pending,
        tracking_number: None,
        shipped_at: None,
        delivered_at: None,
      ))
    many -> Error(MultipleWarehouses(list.sort(many, string.compare)))
  }
}

/// Lines for the warehouse pick list, sorted by SKU.
pub fn pick_list(shipment: Shipment) -> List(String) {
  shipment.items
  |> list.sort(fn(a, b) { string.compare(a.sku, b.sku) })
  |> list.map(fn(i) { i.sku <> " x " <> int.to_string(i.quantity) })
}

pub fn tracking_number(shipment: Shipment) -> String {
  "SOL" <> string.pad_start(int.to_string(shipment.id), 9, "0")
}

pub fn pack(shipment: Shipment) -> Result(Shipment, ShipmentError) {
  case shipment.status {
    Pending -> Ok(Shipment(..shipment, status: Packed))
    status -> Error(InvalidShipmentState(status, "pack"))
  }
}

pub fn dispatch(
  shipment: Shipment,
  now: Int,
) -> Result(Shipment, ShipmentError) {
  case shipment.status {
    Packed ->
      Ok(
        Shipment(
          ..shipment,
          status: InTransit,
          tracking_number: Some(tracking_number(shipment)),
          shipped_at: Some(now),
        ),
      )
    status -> Error(InvalidShipmentState(status, "dispatch"))
  }
}

pub fn deliver(
  shipment: Shipment,
  now: Int,
) -> Result(Shipment, ShipmentError) {
  case shipment.status {
    InTransit ->
      Ok(Shipment(..shipment, status: Delivered, delivered_at: Some(now)))
    status -> Error(InvalidShipmentState(status, "deliver"))
  }
}

/// The allocations a shipment consumes when it leaves the warehouse.
pub fn allocations(shipment: Shipment) -> List(Allocation) {
  list.map(shipment.items, fn(i) {
    inventory.Allocation(shipment.warehouse_id, i.sku, i.quantity)
  })
}
