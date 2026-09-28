Operations needs to cancel orders that are approved but not yet shipped.

- Allow `cancel` from `Approved` (in addition to `Draft` and `Submitted`). Shipped orders still can't be cancelled.
- Cancelling an approved order must return its reserved stock to inventory. Cancelling a draft or submitted order must not change stock (it was never reserved).
- `checkout.cancel(db, clock, order_id, reason)` keeps its signature.
