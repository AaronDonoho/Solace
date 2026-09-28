Some customers, such as schools, are exempt from sales tax, and we need to support that.

- Add a field `tax_exempt: Bool` to `customer.Customer`. Seed customers and newly registered customers (`accounts.register`) are not exempt.
- Exempt customers pay no tax on their orders. Add a final parameter `tax_exempt: Bool` to `pricing.quote`, and make checkout pass the customer's flag.
- The tax report needs to show exemptions: add a final field `exempt_orders: Int` to `reporting.RegionTax`, counting that region's orders from exempt customers.

Update all callers.
