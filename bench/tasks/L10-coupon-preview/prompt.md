The storefront wants to show "your total with this coupon" before the customer commits to a coupon.

Add a request `PreviewCoupon(order_id: Int, code: String, method: Method)` to `api.Request`. For a draft order, it returns what `GetQuote` would return if the coupon were applied: status 200 and the same JSON body format. It must not change anything. The returned store is the one passed in, and the order keeps whatever coupon it had.

If the coupon can't be applied, it returns the same error response that `ApplyCoupon` would.
