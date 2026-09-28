You are picking up work another developer started. Their work record:

> **Goal:** support refunds of shipped orders.
> **Done:** `Refunded` status and `Refund` action in `order.gleam`, with the transition `Shipped -> Refunded` and a test for it.
> **Remaining:**
> 1. `checkout.refund(db: Store, clock: Clock, order_id: Int) -> Result(#(Store, Order), CheckoutError)`: applies the transition and returns the order's stock to inventory.
> 2. Checkout tests for refunds.

Finish the remaining work.
