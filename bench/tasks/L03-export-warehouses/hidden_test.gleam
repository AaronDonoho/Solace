import gleam/list
import pilot/checkout
import pilot/clock
import pilot/customer.{Domestic, International}
import pilot/fulfillment
import pilot/inventory.{Allocation, InsufficientStock, Warehouse}
import pilot/payments
import pilot/seed
import pilot/shipping.{Standard}
import pilot/store

fn warehouses() {
  [
    Warehouse(id: "E", name: "East", state: "NJ", exports: True),
    Warehouse(id: "W", name: "West", state: "CA", exports: False),
  ]
}

fn stock() {
  inventory.new() |> inventory.receive("E", "A", 2) |> inventory.receive("W", "A", 5)
}

pub fn international_uses_exporting_warehouses_only_test() {
  assert inventory.reserve(stock(), warehouses(), "A", 3, International("DE"))
    == Error(InsufficientStock("A", 3, 2))
  let assert Ok(#(_, allocations)) = inventory.reserve(stock(), warehouses(), "A", 2, International("DE"))
  assert allocations == [Allocation("E", "A", 2)]
}

pub fn domestic_unchanged_test() {
  let assert Ok(#(_, allocations)) = inventory.reserve(stock(), warehouses(), "A", 3, Domestic("CA"))
  assert allocations == [Allocation("W", "A", 3)]
  let assert Ok(#(_, allocations)) = inventory.reserve(stock(), warehouses(), "A", 6, Domestic("NY"))
  assert allocations == [Allocation("W", "A", 5), Allocation("E", "A", 1)]
}

pub fn seed_warehouses_test() {
  let exporting = seed.warehouses() |> list.filter(fn(w) { w.exports }) |> list.map(fn(w) { w.id })
  assert exporting == ["WH-EAST"]
}

fn submitted(customer_id, sku, quantity) {
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, o)) = checkout.create_order(seed.demo_store(), c, customer_id)
  let assert Ok(#(db, _)) = checkout.add_line(db, c, o.id, sku, quantity)
  let assert Ok(#(db, _)) = checkout.submit(db, c, o.id)
  #(db, o.id)
}

pub fn checkout_respects_exports_test() {
  let #(db, id) = submitted(seed.linus, "PHONES-BLK", 6)
  let c = clock.fixed(seed.epoch)
  let result = checkout.pay(db, c, payments.fake_gateway(), id, "tok_ok", Standard)
  let assert Error(checkout.Inventory(InsufficientStock("PHONES-BLK", 6, 5))) = result
  let #(db, id) = submitted(seed.ada, "PHONES-BLK", 6)
  let assert Ok(#(db, _)) = checkout.pay(db, c, payments.fake_gateway(), id, "tok_ok", Standard)
  assert store.allocations_for(db, id)
    == [Allocation("WH-EAST", "PHONES-BLK", 5), Allocation("WH-WEST", "PHONES-BLK", 1)]
}

pub fn shortage_replacement_respects_exports_test() {
  let #(db, id) = submitted(seed.linus, "LAMP-OAK", 4)
  let c = clock.fixed(seed.epoch)
  let assert Ok(#(db, _)) = checkout.pay(db, c, payments.fake_gateway(), id, "tok_ok", Standard)
  assert fulfillment.report_shortage(db, c, id, "WH-EAST", "LAMP-OAK", 1)
    == Error(fulfillment.InventoryFailed(InsufficientStock("LAMP-OAK", 1, 0)))
}
