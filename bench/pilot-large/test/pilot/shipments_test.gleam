import gleam/option.{None, Some}
import pilot/inventory.{Allocation}
import pilot/shipments.{
  InTransit, Item, MultipleWarehouses, NothingToShip, Packed, Pending,
}
import pilot/shipping.{Standard}

pub fn from_allocations_test() {
  let assert Ok(s) =
    shipments.from_allocations(
      5,
      9,
      [
        Allocation("E", "B", 1),
        Allocation("E", "A", 2),
        Allocation("E", "A", 1),
      ],
      Standard,
    )
  assert s.warehouse_id == "E"
  assert s.items == [Item("A", 3), Item("B", 1)]
  assert s.status == Pending
  assert shipments.pick_list(s) == ["A x 3", "B x 1"]
}

pub fn single_warehouse_only_test() {
  assert shipments.from_allocations(
      5,
      9,
      [Allocation("W", "A", 1), Allocation("E", "B", 1)],
      Standard,
    )
    == Error(MultipleWarehouses(["E", "W"]))
  assert shipments.from_allocations(5, 9, [], Standard) == Error(NothingToShip)
}

pub fn lifecycle_test() {
  let assert Ok(s) =
    shipments.from_allocations(5, 9, [Allocation("E", "A", 1)], Standard)
  let assert Error(_) = shipments.dispatch(s, 1)
  let assert Ok(s) = shipments.pack(s)
  assert s.status == Packed
  assert s.tracking_number == None
  let assert Ok(s) = shipments.dispatch(s, 10)
  assert s.status == InTransit
  assert s.tracking_number == Some("SOL000000005")
  assert s.shipped_at == Some(10)
  let assert Ok(s) = shipments.deliver(s, 20)
  assert s.delivered_at == Some(20)
  assert shipments.allocations(s) == [Allocation("E", "A", 1)]
}
