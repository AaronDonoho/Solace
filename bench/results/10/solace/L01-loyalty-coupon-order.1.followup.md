Solace made this task cheap: it took three calls and about $0.13. Most of the gaps were in checking the answer, not in finding the bug.

**What worked**
- `session start --task` listed `promotions.discount`, `pricing.quote` and the two loyalty/coupon pricing tests near the top. I never had to search.
- One `inspect pricing.quote promotions.discount --context` showed the whole bug. `quote` computed the loyalty discount on `subtotal - coupon_discount`. Meanwhile `discount`'s doc says its base is "the order value it applies to after any earlier discounts", and `loyalty_discount_percent`'s doc says the percentage comes "off the subtotal". The code and docs disagreed, and both were on screen.
- Running `submit` then `test` in one call, with the import of `FixedOff` added automatically, meant no fix-up round trip.

**What cost me a guess or skipped work**
1. **I never reproduced the ticket's $71.21.** I didn't check that TENOFF is `FixedOff(1000)`, that the Desk Lamp is 7400, or what California tax plus shipping comes to. `session start` listed `seed.coupons` and `seed.grace` by signature only. When a ticket names seed entities (Grace, TENOFF, Desk Lamp), showing the matching seed values inline would have let me confirm the case for free. A suggested command would also help, such as `probe --from checkout_test.coupons_test --trace pricing.quote` set up for Grace's order.
2. **I guessed the test helpers.** I wrote `coupon(FixedOff(1000), True)` and `ship(600)` from how the existing tests call them, without seeing `pricing_test.coupon` or `pricing_test.ship`. It type-checked, but it was a guess. When `inspect` shows a test as an example, it could also print the signatures of the private helpers that test uses.
3. **The blast radius was only one level deep.** "Used by" showed `checkout.quote`. But my change moves money between `loyalty_discount_cents` and `coupon_discount_cents`, and `return_policy.refund_amount` reads the `Quote`. I didn't check whether refunds depend on that split. The full test run passing is weak evidence there.

**`probe`:** I used neither `probe --from` nor `probe --trace`. I skipped them because the bug was plain from the source, and the arithmetic (7400 × 5% = 370, then −1000, giving 6030) was simple enough to do by hand. That saved a call but left the ticket's $71.21 unexplained. A single `probe --from … --trace pricing.quote promotions.discount` would have shown the old discounts of 320 and 1000. That would have tied the ticket to the cause before I submitted, and it was the right step to take.

**Which CodeGraph ideas fit Solace**
- **One strong exploration call (`codegraph_explore`):** Solace already has this in `inspect --context`, and it worked. CodeGraph's note about context costs applies here too. The reply repeated full type definitions (`Coupon`, `Quote`, `ShippingQuote`) that I barely needed, so a lighter default would be fine.
- **Impact analysis / blast radius:** this fits and would have helped with gap 3. Because Solace's symbols are typed, it could go further than call edges and list readers of the specific `Quote` fields whose values changed.
- **`codegraph affected`:** this is less useful here. `test` runs all 99 tests quickly, and `inspect` already lists the tests that use a symbol.
- **Auto-sync, file watchers and staleness banners:** these don't apply. Solace's store is the source of truth and every `submit` is type-checked on the spot, so the index can't go stale.
- **Full-text search, multi-language parsing and framework routes:** these don't add much. `find` covers search, and a single-language typed store makes the rest unnecessary.
