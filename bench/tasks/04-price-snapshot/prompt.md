Prices must be locked in when a line is added to an order. If a product's catalog price changes afterwards, existing orders keep the price from the time the line was added.

Required API:
- `order.Line` becomes `Line(sku: String, quantity: Int, unit_price_cents: Int)`.
- `pricing.quote(lines: List(Line), region: Region) -> Quote`: prices lines using their own `unit_price_cents`, with no product lookup.
- `checkout.quote(db, order_id)` keeps its signature and uses the locked prices.
- When `add_line` merges more quantity into an existing line, the line keeps its original price.

Update the existing code and tests to match.
