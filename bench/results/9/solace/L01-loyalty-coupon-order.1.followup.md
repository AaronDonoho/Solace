# How Solace worked on this task, and what Benzi ideas would help

## What went well

The task took three calls: start, inspect, then submit and test.

- **The starting list was accurate.** `session start --task` listed `promotions.discount`, `customer.loyalty_discount_percent`, `pricing.quote` and both related pricing tests near the top. I didn't need to search.
- **One `inspect --context` call was enough.** It showed the full body of `pricing.quote` and `promotions.discount`, the types they use, the only caller (`checkout.quote`) and the two tests. The bug was visible there. Once I saw `money.percent(subtotal - coupon_discount, …)`, the cause was clear. The doc comment on `discount` ("the order value it applies to after any earlier discounts") confirmed the intended order.
- **Submit and test in one call** checked the types, added the missing `FixedOff` import itself, and ran all 99 tests. That's about as cheap as editing can be.

Benzi's main claim is that you read less source because you query an index. Solace already does that in this setting, and with a more exact unit: typed symbols instead of tree-sitter text.

## Where I guessed or cut corners

1. **I never reproduced the $71.21 figure.** I checked the arithmetic myself: the 50¢ difference in the loyalty discount (5% of $74.00 vs 5% of $64.00). I didn't check the ticket's number end to end through `checkout.quote` with the seed data (Grace, the Desk Lamp, TENOFF, standard shipping to her address). `probe` could have done this. I skipped it because building a store, an order and a clock by hand in throwaway code looked like several calls of trial and error. So my fix is only verified against my own reading of the ticket, not against the reported number.
   - **What would help:** a way to run the ticket's own example against the seed data without hand-building a store. For example: `probe --seed` with the seeded store already bound, or a `run quote <customer> <skus> <coupon> <method>` command declared in `solace.toml`.

2. **The new test is weaker than it should be.** I didn't know the CA tax result for $60.30 plus $6.00 shipping (is it rounded or truncated?). So I asserted `total == 6030 + 600 + q.tax_cents`, which can't fail. A real expected number would have cost one `probe`, or a value readout from the test run.
   - **What would help:** when a test passes, optionally show the values of the expressions it asserts on. Or a one-line `eval` that's cheaper to use than writing a probe module.

3. **I didn't check the side effect before submitting.** Moving the coupon's base from `subtotal` to `subtotal - loyalty` changes the cap on BuyXGetY coupons. I noticed this afterwards and mentioned it, but I didn't check whether any seed coupon or test covers it. All tests passed, so nothing covers it. A warning on submit would have helped, something like: "this argument to `promotions.discount` changed meaning; these tests exercise `discount` through `quote`; these `CouponKind` variants have no test through `quote`."

## Which Benzi ideas fit Solace

- **Runtime tracer, `execute_from` / `check_last_execution`: a strong fit.** Gleam has no mutation, so a trace of let-bindings is easy to read. If I could run `checkout.quote` on the seed order and see `loyalty = 320`, `coupon_discount = 1000`, `goods = 6080`, I'd have confirmed the cause from the reported symptom instead of from reading the code. It would also have covered points 1 and 2.
- **`execute_generated_testcase`, run a repro and debug the change: a fit.** It's `probe` with seed data and the test helpers already in scope, which is exactly the gap in point 1.
- **Blast radius checked on the way in: a fit, if made more semantic.** Solace already lists callers and the tests that use a symbol. Benzi's version also reports when a change to how a value is computed affects what callers see. That's my point 3.
- **`backflow` / `forwardflow`: a weak fit here.** The wrong value came from a local calculation in one pure function, and `inspect` showed all of it. Tracing values across modules matters more in bigger, effectful codebases than in this one.
- **Six resolution states (candidate, unresolved, and so on): not needed.** Gleam's type checker already resolves every reference statically, with no dynamic dispatch or inheritance. Solace's references are already Benzi's "resolved" state.
- **Snapshot rollback: mostly covered.** Submits are all-or-nothing, rejected code is kept as a draft, and nothing broken is saved silently. An explicit `undo` of the last saved submit would still be useful when a change type-checks but turns out wrong.
- **`get_hierarchy`, the markup engine, `upgrade_to_pro`: don't apply.** Gleam has no classes, there's no frontend here, and changing the model isn't Solace's job.
- **Persistent memory:** Solace's work records already cover this. Nothing on this task needed it.

## Summary

For a bug like this, Solace's reading tools were already enough. What I lacked was a cheap way to run the ticket's example against the seed data and see the intermediate values. That gap cost me an end-to-end check against the $71.21 figure and left the new test asserting on less than it should. Benzi's run-and-trace tools (`execute_from`, `check_last_execution`, generated repros) are the ideas worth adopting. Its methods for resolving ambiguous references aren't needed in Gleam.
