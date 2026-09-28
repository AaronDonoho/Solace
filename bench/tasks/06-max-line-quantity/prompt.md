A single order line may hold at most 10 units of a product.

`checkout.add_line` must return `InvalidQuantity(n)` when the line's resulting quantity `n` would exceed 10. That includes adding to a line that already exists (e.g. 6 then 5 gives `InvalidQuantity(11)`). The order must be left unchanged in that case.
