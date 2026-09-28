When a customer's payment fails, nobody tells them. The order just stays unpaid.

Email the customer when a payment is declined or the payment provider is unavailable:

- Add `PaymentFailed` to `notifications.Kind`. The message goes to the customer's email, and its body must contain the line `Attempts: N`, where N is the number of attempts made.
- Record an audit entry with action `"payment failed"` for the order (subject `audit.order_subject(order_id)`).
- To make that possible, change `checkout.pay` to return `#(Store, Result(Order, CheckoutError))`:
  - On success, the store is the same as today's successful result.
  - On a declined card or an unavailable provider, the store is the original store plus the email and the audit entry. The order, stock and payments are unchanged.
  - On any other error, the original store is returned unchanged.
- The API's `PayOrder` keeps its status codes, and returns the store with the email and audit entry when payment fails.

Update all callers.
