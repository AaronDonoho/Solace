# Large pilot plan

The benchmark's second size point (D-7). This file is for us, not for benchmark agents: it lives outside the project, so agents never see it.

## Targets

- **Source:** about 30k tokens, measured as source bytes ÷ 4 (`bench/bench.py size`). That's about 4,500 lines of Gleam in `src/`. Tests are extra, roughly 2,000–2,500 lines.
- **Style:** plain, idiomatic Gleam, as a competent team would write it. A README with a module overview, doc comments on public functions, and tests per module. The baseline must be a well-kept conventional repo, not a strawman.
- **Same domain as the small pilot:** order management. External services come in as values (like `Clock`), so everything stays deterministic.

## Modules (`src/pilot/`)

Rough line budgets, trimmed while writing to hit the token target.

| Module | Responsibility | ~Lines |
| --- | --- | --- |
| `clock` | Time source; fixed and stepping fakes | 30 |
| `date` | Calendar dates from Unix ms, day of week, add business days, ISO format | 150 |
| `money` | Integer-cents helpers: percent with rounding, formatting, sums | 80 |
| `store` | Immutable in-memory tables for every entity, with get/put/list | 280 |
| `audit` | Append-only audit entries written by services; queries | 120 |
| `catalog` | Products, variants (size and colour), categories, active flag, price and weight lookup, search | 280 |
| `inventory` | Stock per warehouse per variant; reserve, release, commit; warehouse allocation; low-stock report | 320 |
| `customer` | Accounts, addresses, email validation, loyalty tier from lifetime spend, tax-exempt flag | 250 |
| `order` | Order record and state machine: Draft → Submitted → Paid → Picking → Shipped → Delivered, plus Cancelled and (partly) Returned | 250 |
| `promotions` | Coupons: percent off, fixed off, free shipping, buy-X-get-Y; minimum spend, expiry, per-customer limit, stacking rules; `validate` | 330 |
| `tax` | Rates by region and state, exempt customers, tax on shipping in some regions; `quote` | 180 |
| `shipping` | Zones, weight bands, carriers, free-shipping thresholds, delivery estimate in business days; `quote` | 300 |
| `pricing` | Line totals, loyalty discount, coupon application order, order-level tiers; combines tax and shipping; `quote` | 320 |
| `payments` | `Gateway` value (authorize, capture, refund) with a fake; payment records; idempotency keys; capture on ship; partial refunds | 330 |
| `checkout` | Cart operations, apply coupon, submit, pay, cancel; `refund` for cancelled paid orders | 380 |
| `fulfillment` | Pick lists, shipments split by warehouse, tracking numbers, mark delivered | 280 |
| `returns` | Return requests, 30-day eligibility after delivery, restocking fee per category, refund amount, restock; `validate`, `refund` | 300 |
| `notifications` | Message building (confirmation, shipped, refund); messages are queued in the store's outbox | 220 |
| `reporting` | Daily sales, top products, refunds, inventory valuation, tax per region | 280 |
| `api` | `handle(Request) -> Response`: a typed request per operation, errors mapped to status codes and messages | 300 |

About 4,960 lines in total, to be trimmed toward about 4,500.

**Realistic friction, not traps:** several modules each have their own `quote`, `validate` or `refund`; business rules span modules (a coupon affects pricing, checkout, reporting and notifications); entry points reach the logic through two or three layers (`api` → `checkout` → `pricing` → `promotions`).

## Tasks (written before the code)

Each task's area is noted here for us; the prompts given to agents describe only the symptom or the feature, the way a ticket would. Hidden tests are written after the code, against the API each prompt names.

| # | Kind | Prompt (short) | Where the change really is |
| --- | --- | --- | --- |
| L01 | Defect (seeded) | Gold customers using a coupon are charged slightly too much | `pricing`: loyalty discount applied after the coupon instead of before |
| L02 | Feature | Coupons can be restricted to one product category | `promotions` validation and discount base; `pricing`; `api` error |
| L03 | Cross-caller refactor | Only exporting warehouses ship internationally; `inventory.reserve` takes a `Region` | `inventory`, `Warehouse` type, `seed`, callers in `checkout` and `fulfillment` (reserve and consolidate), tests |
| L04 | Defect (seeded) | A partial return refunds the shipping fee | `returns` refund amount |
| L05 | Feature | Email the customer when a payment fails, with the attempt count | `checkout.pay` returns `#(Store, Result(..))` so failures can be recorded; `notifications`, `api`, test helpers |
| L06 | Feature | Report of refunds per month grouped by reason | `reporting`, reading `returns` and `payments` records |
| L07 | Resume work | Split shipments: `shipments.split_by_warehouse` done, the rest listed in a work record | `fulfillment` (start_picking, ship, deliver), `api`, test helpers |
| L08 | Feature | Tax-exempt customers skip tax, domestic only | `customer`, `tax`, `pricing`, `reporting` tax report |
| L09 | Defect (seeded) | Delivery estimates count weekends | `date` / `shipping` |
| L10 | Feature | API endpoint to preview an order total with a coupon, without applying it | `api`, `pricing`, `promotions` |

## As built (2026-09-27)

- 22 modules, 29.5k tokens of source (118k bytes). Two modules were added beyond the table: `accounts` (registration and profile changes) and `seed` (demo data used by tests).
- Pure domain types that the store holds live in their own modules (`shipments`, `return_policy`, `payments`) because Gleam forbids import cycles; the services that use the store are `checkout`, `fulfillment`, `returns`, `reporting`, `accounts` and `api`.
- 98 tests (1,444 lines), no compiler warnings.
- All 10 tasks validated: hidden tests fail before, pass with the reference solution. Setup patches keep the visible tests green and add no compiler warnings, so the planted bugs aren't flagged by the tooling.
