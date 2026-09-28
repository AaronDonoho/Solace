//// Stock levels per warehouse, reservations and allocation.
////
//// Stock is tracked per (warehouse, SKU) as units on hand and units
//// reserved for paid orders. Reserved units stay on hand until they ship.

import gleam/dict.{type Dict}
import gleam/int
import gleam/list
import gleam/order as ordering
import gleam/result
import gleam/string

pub type Warehouse {
  Warehouse(id: String, name: String, state: String)
}

pub type Stock {
  Stock(
    on_hand: Dict(#(String, String), Int),
    reserved: Dict(#(String, String), Int),
  )
}

/// Units of a SKU reserved at one warehouse.
pub type Allocation {
  Allocation(warehouse_id: String, sku: String, quantity: Int)
}

pub type InventoryError {
  InsufficientStock(sku: String, requested: Int, available: Int)
  UnknownWarehouse(id: String)
  InvalidAdjustment(warehouse_id: String, sku: String, quantity: Int)
}

pub fn new() -> Stock {
  Stock(on_hand: dict.new(), reserved: dict.new())
}

fn get(counts: Dict(#(String, String), Int), key: #(String, String)) -> Int {
  dict.get(counts, key) |> result.unwrap(0)
}

fn bump(
  counts: Dict(#(String, String), Int),
  key: #(String, String),
  by: Int,
) -> Dict(#(String, String), Int) {
  dict.insert(counts, key, get(counts, key) + by)
}

/// Adds received units to a warehouse.
pub fn receive(
  stock: Stock,
  warehouse_id: String,
  sku: String,
  quantity: Int,
) -> Stock {
  Stock(..stock, on_hand: bump(stock.on_hand, #(warehouse_id, sku), quantity))
}

/// Units that can still be reserved at a warehouse.
pub fn available(stock: Stock, warehouse_id: String, sku: String) -> Int {
  let key = #(warehouse_id, sku)
  get(stock.on_hand, key) - get(stock.reserved, key)
}

pub fn total_available(stock: Stock, sku: String) -> Int {
  stock.on_hand
  |> dict.keys
  |> list.filter(fn(key) { key.1 == sku })
  |> list.fold(0, fn(sum, key) { sum + available(stock, key.0, sku) })
}

/// Reserves units of a SKU, preferring warehouses in the destination state,
/// then the warehouses with the most units available. A reservation may be
/// split across warehouses.
pub fn reserve(
  stock: Stock,
  warehouses: List(Warehouse),
  sku: String,
  quantity: Int,
  destination_state: String,
) -> Result(#(Stock, List(Allocation)), InventoryError) {
  let total = total_available(stock, sku)
  use <- guard(total >= quantity, InsufficientStock(sku, quantity, total))
  let ranked =
    list.sort(warehouses, fn(a, b) {
      let local = fn(w: Warehouse) { w.state == destination_state }
      case local(a), local(b) {
        True, False -> ordering.Lt
        False, True -> ordering.Gt
        _, _ ->
          case
            int.compare(
              available(stock, b.id, sku),
              available(stock, a.id, sku),
            )
          {
            ordering.Eq -> string.compare(a.id, b.id)
            other -> other
          }
      }
    })
  let #(stock, remaining, allocations) =
    list.fold(ranked, #(stock, quantity, []), fn(acc, warehouse) {
      let #(stock, remaining, allocations) = acc
      let take = int.min(remaining, available(stock, warehouse.id, sku))
      case take > 0 {
        True -> #(
          reserve_units(stock, warehouse.id, sku, take),
          remaining - take,
          [Allocation(warehouse.id, sku, take), ..allocations],
        )
        False -> acc
      }
    })
  case remaining {
    0 -> Ok(#(stock, list.reverse(allocations)))
    _ -> Error(InsufficientStock(sku, quantity, total))
  }
}

/// Reserves units at one specific warehouse.
pub fn reserve_at(
  stock: Stock,
  warehouse_id: String,
  sku: String,
  quantity: Int,
) -> Result(#(Stock, Allocation), InventoryError) {
  let can = available(stock, warehouse_id, sku)
  use <- guard(can >= quantity, InsufficientStock(sku, quantity, can))
  Ok(#(
    reserve_units(stock, warehouse_id, sku, quantity),
    Allocation(warehouse_id, sku, quantity),
  ))
}

fn reserve_units(
  stock: Stock,
  warehouse_id: String,
  sku: String,
  n: Int,
) -> Stock {
  Stock(..stock, reserved: bump(stock.reserved, #(warehouse_id, sku), n))
}

/// Returns reserved units to available stock, e.g. when an order is cancelled.
pub fn release(stock: Stock, allocations: List(Allocation)) -> Stock {
  list.fold(allocations, stock, fn(stock, a) {
    reserve_units(stock, a.warehouse_id, a.sku, 0 - a.quantity)
  })
}

/// Removes shipped units: they leave both reserved and on-hand stock.
pub fn commit(stock: Stock, allocations: List(Allocation)) -> Stock {
  list.fold(allocations, stock, fn(stock, a) {
    let key = #(a.warehouse_id, a.sku)
    Stock(
      on_hand: bump(stock.on_hand, key, 0 - a.quantity),
      reserved: bump(stock.reserved, key, 0 - a.quantity),
    )
  })
}

/// Records units found missing or damaged at a warehouse.
pub fn write_off(
  stock: Stock,
  warehouse_id: String,
  sku: String,
  quantity: Int,
) -> Result(Stock, InventoryError) {
  let key = #(warehouse_id, sku)
  use <- guard(
    quantity > 0 && quantity <= get(stock.on_hand, key),
    InvalidAdjustment(warehouse_id, sku, quantity),
  )
  Ok(Stock(..stock, on_hand: bump(stock.on_hand, key, 0 - quantity)))
}

/// Allocations summed per (warehouse, SKU).
pub fn merge_allocations(allocations: List(Allocation)) -> List(Allocation) {
  allocations
  |> list.fold(dict.new(), fn(acc, a) {
    bump(acc, #(a.warehouse_id, a.sku), a.quantity)
  })
  |> dict.to_list
  |> list.map(fn(pair) { Allocation(pair.0.0, pair.0.1, pair.1) })
  |> list.sort(fn(a, b) {
    string.compare(
      a.warehouse_id <> "/" <> a.sku,
      b.warehouse_id <> "/" <> b.sku,
    )
  })
}

/// (warehouse, SKU, available) for every stock level at or below `threshold`.
pub fn low_stock(stock: Stock, threshold: Int) -> List(#(String, String, Int)) {
  stock.on_hand
  |> dict.keys
  |> list.map(fn(key) { #(key.0, key.1, available(stock, key.0, key.1)) })
  |> list.filter(fn(row) { row.2 <= threshold })
  |> list.sort(fn(a, b) {
    case int.compare(a.2, b.2) {
      ordering.Eq -> string.compare(a.0 <> a.1, b.0 <> b.1)
      other -> other
    }
  })
}

pub fn find_warehouse(
  warehouses: List(Warehouse),
  id: String,
) -> Result(Warehouse, InventoryError) {
  list.find(warehouses, fn(w) { w.id == id })
  |> result.replace_error(UnknownWarehouse(id))
}

fn guard(
  ok: Bool,
  error: InventoryError,
  continue: fn() -> Result(a, InventoryError),
) -> Result(a, InventoryError) {
  case ok {
    True -> continue()
    False -> Error(error)
  }
}
