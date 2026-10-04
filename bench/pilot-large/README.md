# pilot

Order-management service: catalog, cart and checkout, payments, fulfillment across warehouses, returns, notifications and reporting.

```sh
gleam test   # run the tests
```

## Design

- **Pure core, explicit state.** All data lives in an immutable `store.Store`. Service functions take the current store and return the updated one along with their result. Nothing is saved unless the whole operation succeeds.
- **The outside world is passed in.** Time comes in as a `clock.Clock`, and the payment provider as a `payments.Gateway`. Tests use `clock.fixed` and `payments.fake_gateway()`. Customer emails are queued in the store's outbox (`store.sent_messages`).
- **Money is integer cents** throughout. Rounding rules are in `money`.
- **Demo data:** `seed.demo_store()` has three warehouses, a small catalog, customers in each loyalty tier and region, and a set of coupons. Tests build on it, with helpers in `test/support.gleam`.

## Modules

Services (take and return a `Store`):

| Module | Purpose |
| --- | --- |
| `api` | Request handler: typed requests in, status and JSON body out |
| `checkout` | Build orders, apply coupons, quote, submit, pay, cancel, goodwill refunds |
| `fulfillment` | Pick, pack, ship and deliver; consolidate stock to one warehouse; stock shortages |
| `returns` | Request, approve, receive and refund returns |
| `accounts` | Register customers and change their details |
| `reporting` | Sales, top products, returns, tax and stock reports |

Domain logic (pure):

| Module | Purpose |
| --- | --- |
| `order` | Order record, lines and the order state machine |
| `catalog` | Products, variants and categories |
| `customer` | Customers, addresses, regions and loyalty tiers |
| `inventory` | Stock per warehouse, reservations and allocation |
| `pricing` | Order totals: discounts, shipping and tax combined |
| `promotions` | Coupons: validation and discounts |
| `shipping` | Shipping zones, rates and delivery estimates |
| `tax` | Sales tax rates and rules |
| `payments` | Payment gateway, authorization, capture, void and refund |
| `shipments` | Shipments and their lifecycle |
| `return_policy` | Return eligibility and refund amounts |
| `notifications` | Customer email messages |
| `audit` | Audit trail entries |

Support: `store` (persistence), `seed` (demo data), `clock`, `date` (calendar and business days), `money`.

## Order lifecycle

Draft → Submitted → Paid → Picking → Shipped → Delivered. An order can be cancelled until it is picked.

- **Paying** reserves stock and authorizes the card.
- **Shipping** takes the stock out of the warehouse and captures the payment.
- **Returns** are accepted up to 30 days after delivery.
