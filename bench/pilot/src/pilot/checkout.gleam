//// Checkout service: the operations callers use. Each takes the current
//// Store and returns the updated Store alongside its result.

import gleam/bool
import gleam/list
import gleam/result
import pilot/clock.{type Clock}
import pilot/order.{type Order, type OrderError, Line}
import pilot/pricing.{type Quote}
import pilot/product.{type Product, Product}
import pilot/store.{type Store}

pub type CheckoutError {
  UnknownCustomer(id: Int)
  UnknownOrder(id: Int)
  UnknownProduct(sku: String)
  InvalidQuantity(quantity: Int)
  OrderNotEditable(status: order.Status)
  OutOfStock(sku: String, requested: Int, available: Int)
  Rejected(OrderError)
}

pub fn create_order(
  db: Store,
  clock: Clock,
  customer_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use _ <- result.try(
    store.get_customer(db, customer_id)
    |> result.replace_error(UnknownCustomer(customer_id)),
  )
  let #(db, id) = store.allocate_order_id(db)
  let created = order.new(id, customer_id, clock())
  Ok(#(store.put_order(db, created), created))
}

/// Adds quantity of a product to a draft order, merging with an existing line.
pub fn add_line(
  db: Store,
  clock: Clock,
  order_id: Int,
  sku: String,
  quantity: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use <- bool.guard(
    !order.is_editable(existing),
    Error(OrderNotEditable(existing.status)),
  )
  use <- bool.guard(quantity <= 0, Error(InvalidQuantity(quantity)))
  use _ <- result.try(get_product(db, sku))
  let lines = case list.any(existing.lines, fn(l) { l.sku == sku }) {
    True ->
      list.map(existing.lines, fn(l) {
        case l.sku == sku {
          True -> Line(..l, quantity: l.quantity + quantity)
          False -> l
        }
      })
    False -> list.append(existing.lines, [Line(sku:, quantity:)])
  }
  let updated = order.Order(..existing, lines:, updated_at: clock())
  Ok(#(store.put_order(db, updated), updated))
}

pub fn submit(
  db: Store,
  clock: Clock,
  order_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  apply(db, clock, order_id, order.Submit)
}

/// Approves a submitted order, reserving stock for every line.
pub fn approve(
  db: Store,
  clock: Clock,
  order_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use approved <- result.try(
    order.transition(existing, order.Approve, clock())
    |> result.map_error(Rejected),
  )
  use db <- result.try(
    list.try_fold(existing.lines, db, fn(db, line) {
      use product <- result.try(get_product(db, line.sku))
      use <- bool.guard(
        product.stock < line.quantity,
        Error(OutOfStock(line.sku, line.quantity, product.stock)),
      )
      Ok(store.put_product(
        db,
        Product(..product, stock: product.stock - line.quantity),
      ))
    }),
  )
  Ok(#(store.put_order(db, approved), approved))
}

pub fn ship(
  db: Store,
  clock: Clock,
  order_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  apply(db, clock, order_id, order.Ship)
}

pub fn cancel(
  db: Store,
  clock: Clock,
  order_id: Int,
  reason: String,
) -> Result(#(Store, Order), CheckoutError) {
  apply(db, clock, order_id, order.Cancel(reason))
}

pub fn quote(db: Store, order_id: Int) -> Result(Quote, CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use customer <- result.try(
    store.get_customer(db, existing.customer_id)
    |> result.replace_error(UnknownCustomer(existing.customer_id)),
  )
  use priced <- result.try(
    list.try_map(existing.lines, fn(line) {
      use product <- result.map(get_product(db, line.sku))
      #(line, product)
    }),
  )
  Ok(pricing.quote(priced, customer.region))
}

fn apply(
  db: Store,
  clock: Clock,
  order_id: Int,
  action: order.Action,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use updated <- result.try(
    order.transition(existing, action, clock())
    |> result.map_error(Rejected),
  )
  Ok(#(store.put_order(db, updated), updated))
}

fn get_order(db: Store, id: Int) -> Result(Order, CheckoutError) {
  store.get_order(db, id) |> result.replace_error(UnknownOrder(id))
}

fn get_product(db: Store, sku: String) -> Result(Product, CheckoutError) {
  store.get_product(db, sku) |> result.replace_error(UnknownProduct(sku))
}
