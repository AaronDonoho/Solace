import pilot/inventory.{Allocation, InsufficientStock, Warehouse}

fn warehouses() {
  [
    Warehouse("E", "East", "NJ"),
    Warehouse("W", "West", "CA"),
    Warehouse("S", "South", "TX"),
  ]
}

fn stock() {
  inventory.new()
  |> inventory.receive("E", "A", 5)
  |> inventory.receive("W", "A", 3)
  |> inventory.receive("S", "A", 8)
}

pub fn available_test() {
  assert inventory.available(stock(), "E", "A") == 5
  assert inventory.total_available(stock(), "A") == 16
  assert inventory.total_available(stock(), "B") == 0
}

pub fn reserve_prefers_destination_state_test() {
  let assert Ok(#(s, allocations)) =
    inventory.reserve(stock(), warehouses(), "A", 2, "CA")
  assert allocations == [Allocation("W", "A", 2)]
  assert inventory.available(s, "W", "A") == 1
}

pub fn reserve_then_most_stock_and_splits_test() {
  let assert Ok(#(_, allocations)) =
    inventory.reserve(stock(), warehouses(), "A", 10, "NY")
  assert allocations == [Allocation("S", "A", 8), Allocation("E", "A", 2)]
  let assert Ok(#(_, allocations)) =
    inventory.reserve(stock(), warehouses(), "A", 5, "CA")
  assert allocations == [Allocation("W", "A", 3), Allocation("S", "A", 2)]
}

pub fn reserve_insufficient_test() {
  assert inventory.reserve(stock(), warehouses(), "A", 17, "CA")
    == Error(InsufficientStock("A", 17, 16))
}

pub fn release_and_commit_test() {
  let assert Ok(#(s, allocations)) =
    inventory.reserve(stock(), warehouses(), "A", 4, "NJ")
  assert inventory.available(inventory.release(s, allocations), "E", "A") == 5
  let shipped = inventory.commit(s, allocations)
  assert inventory.available(shipped, "E", "A") == 1
  assert inventory.total_available(shipped, "A") == 12
}

pub fn reserve_at_test() {
  let assert Ok(#(s, a)) = inventory.reserve_at(stock(), "W", "A", 3)
  assert a == Allocation("W", "A", 3)
  assert inventory.reserve_at(s, "W", "A", 1)
    == Error(InsufficientStock("A", 1, 0))
}

pub fn write_off_test() {
  let assert Ok(s) = inventory.write_off(stock(), "E", "A", 2)
  assert inventory.available(s, "E", "A") == 3
  assert inventory.write_off(stock(), "E", "A", 6)
    == Error(inventory.InvalidAdjustment("E", "A", 6))
}

pub fn merge_and_low_stock_test() {
  assert inventory.merge_allocations([
      Allocation("E", "A", 1),
      Allocation("W", "B", 2),
      Allocation("E", "A", 3),
    ])
    == [Allocation("E", "A", 4), Allocation("W", "B", 2)]
  assert inventory.low_stock(stock(), 5) == [#("W", "A", 3), #("E", "A", 5)]
}
