Marketing wants coupons that apply to one product category only, e.g. "20% off books".

Add a field `category: Option(catalog.Category)` to `promotions.Coupon`. `None` means the coupon applies to the whole order, as today; all existing coupons have `None`. For a coupon with a category:

- It can only be applied to an order with at least one line in that category. Otherwise it is rejected with `NotApplicable(code, reason)`.
- Its minimum spend is checked against the total of the order's lines in that category, not the whole subtotal.
- `PercentOff` is a percentage of the total of the lines in that category. `FixedOff` is capped at that total.

Everything else about coupons (expiry, usage limits, stacking with loyalty) works as before.
