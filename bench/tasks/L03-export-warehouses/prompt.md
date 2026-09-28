Only some of our warehouses can ship internationally, but international orders currently reserve stock at any warehouse.

- Add a field `exports: Bool` to `inventory.Warehouse`. In the seed data, WH-EAST exports and WH-WEST and WH-SOUTH don't.
- Change `inventory.reserve` to take the order's destination as a `customer.Region` (replacing the destination state code) as its last parameter:
  - For domestic destinations, behavior is unchanged: warehouses in the destination state first, then the ones with the most available.
  - For international destinations, only exporting warehouses may be used. If they don't have enough, fail with `InsufficientStock(sku, requested, available)`, where `available` counts only exporting warehouses.
- The same rule applies everywhere stock is reserved for an international order, including replacement stock after a shortage and consolidating an order into one warehouse.

Update all callers.
