You are picking up work another developer started. Their work record:

> **Goal:** orders whose stock is reserved across several warehouses, and can't be consolidated into one, should ship as one shipment per warehouse instead of failing with `MultipleWarehouses`.
>
> **Done:** `shipments.split_by_warehouse(first_id, order_id, allocations, method)` builds one shipment per warehouse, with consecutive ids starting at `first_id`, in warehouse id order. It has tests.
>
> **Remaining:**
> 1. `fulfillment.start_picking` returns `Result(#(Store, List(Shipment)), FulfillmentError)`: one shipment when the order can ship from one warehouse (after consolidation, as today), otherwise one per warehouse. Shipment ids must not collide with other ids in the store.
> 2. `fulfillment.ship`: the payment is captured on the first shipment; each shipment commits only its own stock, and the order's remaining allocations shrink accordingly. The order becomes `Shipped` only when its last shipment ships; until then it stays `Picking`.
> 3. `fulfillment.deliver`: the order becomes `Delivered` only when all its shipments are delivered.
> 4. The API's `StartPicking` responds 201 with a JSON array of the shipments.
> 5. Tests for all of the above.

Finish the remaining work.
