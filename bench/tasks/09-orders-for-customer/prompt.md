The account page needs a customer's order history.

Add `checkout.orders_for(db: Store, customer_id: Int) -> Result(List(Order), CheckoutError)`. It returns all orders for the customer, newest first by `created_at`, with ties broken by order id (highest first). It returns `UnknownCustomer(id)` if the customer doesn't exist.
