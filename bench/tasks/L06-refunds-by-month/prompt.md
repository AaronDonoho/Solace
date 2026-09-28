Finance wants a report of refunded returns per month, broken down by reason. Returns don't currently record when they were refunded.

- Add a field `refunded_at: Option(Int)` to `return_policy.ReturnRequest`. It is `None` until the return is refunded; `returns.refund` sets it to the refund time.
- Add `reporting.MonthlyRefunds(month: String, reason: String, returns: Int, refunded_cents: Int)` and `reporting.refunds_by_month(db: Store) -> List(MonthlyRefunds)`:
  - one row per (month of refund, reason), for refunded returns only
  - `month` is `"YYYY-MM"`, and `reason` is `return_policy.reason_name(reason)`
  - `refunded_cents` is the sum of the returns' refund totals
  - rows are sorted by month, then reason
