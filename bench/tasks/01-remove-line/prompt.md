Customers need to remove a product from their cart before submitting.

Add `checkout.remove_line(db: Store, clock: Clock, order_id: Int, sku: String) -> Result(#(Store, Order), CheckoutError)`. It removes the line for `sku` from a draft order and sets `updated_at` from the clock.

Errors:
- `UnknownOrder(id)` if the order doesn't exist
- `OrderNotEditable(status)` if the order isn't a draft
- a new `CheckoutError` variant `LineNotFound(sku: String)` if the order has no line for that sku
