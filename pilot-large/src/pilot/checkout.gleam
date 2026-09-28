//// Checkout: building an order, paying for it, and cancelling or refunding
//// it. Each operation takes the current Store and returns the updated one.

import gleam/bool
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import pilot/audit
import pilot/catalog.{type CatalogError}
import pilot/clock.{type Clock}
import pilot/customer.{type Customer, type Region, Domestic, International}
import pilot/inventory.{type Allocation, type InventoryError, type Warehouse}
import pilot/notifications
import pilot/order.{type Order, type OrderError, Line}
import pilot/payments.{type Gateway, type PaymentError}
import pilot/pricing.{type Quote}
import pilot/promotions.{type CouponError}
import pilot/shipping.{type Method}
import pilot/store.{type Store}

pub type CheckoutError {
  UnknownCustomer(id: Int)
  UnknownOrder(id: Int)
  UnknownCoupon(code: String)
  Catalog(CatalogError)
  InvalidQuantity(quantity: Int)
  OrderNotEditable(status: order.Status)
  LineNotFound(sku: String)
  CouponRejected(CouponError)
  Inventory(InventoryError)
  Payment(PaymentError)
  Rejected(OrderError)
  NoWarehouse
  NoPayment(order_id: Int)
}

const actor = "checkout"

pub fn create_order(
  db: Store,
  clock: Clock,
  customer_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use customer <- result.try(get_customer(db, customer_id))
  let #(db, id) = store.next_id(db)
  let now = clock()
  let created = order.new(id, customer_id, customer.shipping_address, now)
  let db =
    db
    |> store.put_order(created)
    |> store.record(now, actor, "create", audit.order_subject(id), "")
  Ok(#(db, created))
}

/// Adds units of a SKU at its current price, merging with an existing line.
pub fn add_line(
  db: Store,
  clock: Clock,
  order_id: Int,
  sku: String,
  quantity: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_editable(db, order_id))
  use <- bool.guard(quantity <= 0, Error(InvalidQuantity(quantity)))
  use #(_, variant) <- result.try(
    catalog.find_sellable(db.products, sku) |> result.map_error(Catalog),
  )
  existing
  |> order.add_line(Line(sku:, quantity:, unit_price_cents: variant.price_cents))
  |> save(db, clock)
}

pub fn remove_line(
  db: Store,
  clock: Clock,
  order_id: Int,
  sku: String,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_editable(db, order_id))
  use _ <- result.try(
    order.find_line(existing, sku) |> result.replace_error(LineNotFound(sku)),
  )
  existing |> order.remove_line(sku) |> save(db, clock)
}

/// Sets the quantity of an existing line; zero removes it.
pub fn set_quantity(
  db: Store,
  clock: Clock,
  order_id: Int,
  sku: String,
  quantity: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_editable(db, order_id))
  use <- bool.guard(quantity < 0, Error(InvalidQuantity(quantity)))
  use _ <- result.try(
    order.find_line(existing, sku) |> result.replace_error(LineNotFound(sku)),
  )
  existing |> order.set_quantity(sku, quantity) |> save(db, clock)
}

pub fn apply_coupon(
  db: Store,
  clock: Clock,
  order_id: Int,
  code: String,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_editable(db, order_id))
  use coupon <- result.try(
    store.get_coupon(db, code) |> result.replace_error(UnknownCoupon(code)),
  )
  use _ <- result.try(check_coupon(db, existing, coupon.code, clock()))
  order.Order(..existing, coupon_code: Some(coupon.code)) |> save(db, clock)
}

pub fn remove_coupon(
  db: Store,
  clock: Clock,
  order_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_editable(db, order_id))
  order.Order(..existing, coupon_code: None) |> save(db, clock)
}

/// Prices an order as it stands, shipping from the warehouse that would
/// normally serve its destination.
pub fn quote(
  db: Store,
  order_id: Int,
  method: Method,
) -> Result(Quote, CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use customer <- result.try(get_customer(db, existing.customer_id))
  use origin <- result.try(origin_warehouse(
    db.warehouses,
    existing.shipping_address.region,
  ))
  let coupon = case existing.coupon_code {
    Some(code) -> store.get_coupon(db, code) |> option.from_result
    None -> None
  }
  let goods = order.subtotal_cents(existing.lines)
  let weight =
    existing.lines
    |> list.map(fn(l) {
      case catalog.find_variant(db.products, l.sku) {
        Ok(#(_, v)) -> v.weight_grams * l.quantity
        Error(_) -> 0
      }
    })
    |> list.fold(0, fn(a, b) { a + b })
  let region = existing.shipping_address.region
  let shipping_quote =
    shipping.quote(method, origin.state, region, weight, goods)
  Ok(pricing.quote(
    existing.lines,
    customer.tier(customer),
    coupon,
    shipping_quote,
    region,
  ))
}

pub fn submit(
  db: Store,
  clock: Clock,
  order_id: Int,
) -> Result(#(Store, Order), CheckoutError) {
  use existing <- result.try(get_order(db, order_id))
  use submitted <- result.try(
    order.transition(existing, order.Submit, clock())
    |> result.map_error(Rejected),
  )
  let db =
    store.record(
      db,
      clock(),
      actor,
      "submit",
      audit.order_subject(order_id),
      "",
    )
  Ok(#(store.put_order(db, submitted), submitted))
}

/// Pays for a submitted order: re-checks the coupon, reserves stock and
/// authorizes the card. Nothing is stored unless every step succeeds.
pub fn pay(
  db: Store,
  clock: Clock,
  gateway: Gateway,
  order_id: Int,
  card_token: String,
  method: Method,
) -> Result(#(Store, Order), CheckoutError) {
  let now = clock()
  use existing <- result.try(get_order(db, order_id))
  use paid <- result.try(
    order.transition(existing, order.Pay, now) |> result.map_error(Rejected),
  )
  use customer <- result.try(get_customer(db, existing.customer_id))
  use _ <- result.try(case existing.coupon_code {
    Some(code) -> check_coupon(db, existing, code, now)
    None -> Ok(Nil)
  })
  use quote <- result.try(quote(db, order_id, method))
  use #(stock, allocations) <- result.try(reserve_all(
    db,
    existing.lines,
    existing.shipping_address.region,
  ))
  let #(db, payment_id) = store.next_id(db)
  use payment <- result.try(
    payments.authorize(
      gateway,
      payment_id,
      order_id,
      card_token,
      quote.total_cents,
      now,
    )
    |> result.map_error(Payment),
  )
  let db = case existing.coupon_code {
    Some(code) -> store.count_coupon_use(db, code, customer.id)
    None -> db
  }
  let db =
    db
    |> store.set_stock(stock)
    |> store.set_allocations(order_id, allocations)
    |> store.put_payment(payment)
    |> store.put_quote(order_id, quote)
    |> store.put_order(paid)
    |> store.put_customer(customer.record_spend(customer, quote.total_cents))
    |> store.send(notifications.order_confirmation(customer, paid, quote, now))
    |> store.record(
      now,
      actor,
      "pay",
      audit.order_subject(order_id),
      "payment " <> audit.payment_subject(payment.id),
    )
  Ok(#(db, paid))
}

/// Cancels an order before it is picked. Reserved stock is released and an
/// authorized payment is voided.
pub fn cancel(
  db: Store,
  clock: Clock,
  order_id: Int,
  reason: String,
) -> Result(#(Store, Order), CheckoutError) {
  let now = clock()
  use existing <- result.try(get_order(db, order_id))
  use cancelled <- result.try(
    order.transition(existing, order.Cancel(reason), now)
    |> result.map_error(Rejected),
  )
  use customer <- result.try(get_customer(db, existing.customer_id))
  let db =
    db
    |> store.set_stock(inventory.release(
      db.stock,
      store.allocations_for(db, order_id),
    ))
    |> store.clear_allocations(order_id)
  use db <- result.try(case store.payment_for_order(db, order_id) {
    Ok(payment) ->
      payments.void(payment)
      |> result.map(store.put_payment(db, _))
      |> result.map_error(Payment)
    Error(Nil) -> Ok(db)
  })
  let db =
    db
    |> store.put_order(cancelled)
    |> store.send(notifications.cancelled(customer, cancelled, reason, now))
    |> store.record(now, actor, "cancel", audit.order_subject(order_id), reason)
  Ok(#(db, cancelled))
}

/// A goodwill refund of part of a captured payment, without a return.
pub fn refund(
  db: Store,
  clock: Clock,
  gateway: Gateway,
  order_id: Int,
  amount_cents: Int,
  reason: String,
) -> Result(#(Store, Int), CheckoutError) {
  let now = clock()
  use existing <- result.try(get_order(db, order_id))
  use customer <- result.try(get_customer(db, existing.customer_id))
  use payment <- result.try(
    store.payment_for_order(db, order_id)
    |> result.replace_error(NoPayment(order_id)),
  )
  use #(payment, refund_id) <- result.try(
    payments.refund(gateway, payment, amount_cents) |> result.map_error(Payment),
  )
  let db =
    db
    |> store.put_payment(payment)
    |> store.send(notifications.refund_issued(
      customer,
      order_id,
      amount_cents,
      now,
    ))
    |> store.record(
      now,
      actor,
      "refund",
      audit.order_subject(order_id),
      refund_id <> " " <> reason,
    )
  Ok(#(db, amount_cents))
}

/// The warehouse whose rates are used for quotes: one in the destination
/// state if there is one, otherwise the first warehouse.
pub fn origin_warehouse(
  warehouses: List(Warehouse),
  region: Region,
) -> Result(Warehouse, CheckoutError) {
  let local = case region {
    Domestic(state) -> list.find(warehouses, fn(w) { w.state == state })
    International(_) -> Error(Nil)
  }
  case local, warehouses {
    Ok(w), _ -> Ok(w)
    Error(_), [first, ..] -> Ok(first)
    Error(_), [] -> Error(NoWarehouse)
  }
}

pub fn destination_state(region: Region) -> String {
  case region {
    Domestic(state) -> state
    International(_) -> ""
  }
}

fn reserve_all(
  db: Store,
  lines: List(order.Line),
  region: Region,
) -> Result(#(inventory.Stock, List(Allocation)), CheckoutError) {
  list.try_fold(lines, #(db.stock, []), fn(acc, line) {
    let #(stock, allocations) = acc
    inventory.reserve(
      stock,
      db.warehouses,
      line.sku,
      line.quantity,
      destination_state(region),
    )
    |> result.map(fn(r) { #(r.0, list.append(allocations, r.1)) })
    |> result.map_error(Inventory)
  })
}

fn check_coupon(
  db: Store,
  existing: Order,
  code: String,
  now: Int,
) -> Result(Nil, CheckoutError) {
  use coupon <- result.try(
    store.get_coupon(db, code) |> result.replace_error(UnknownCoupon(code)),
  )
  let context =
    promotions.CouponContext(
      lines: existing.lines,
      subtotal_cents: order.subtotal_cents(existing.lines),
      now:,
      customer_uses: store.coupon_uses(db, code, existing.customer_id),
    )
  promotions.validate(coupon, context)
  |> result.replace(Nil)
  |> result.map_error(CouponRejected)
}

fn save(
  order: Order,
  db: Store,
  clock: Clock,
) -> Result(#(Store, Order), CheckoutError) {
  let updated = order.Order(..order, updated_at: clock())
  Ok(#(store.put_order(db, updated), updated))
}

fn get_order(db: Store, id: Int) -> Result(Order, CheckoutError) {
  store.get_order(db, id) |> result.replace_error(UnknownOrder(id))
}

fn get_editable(db: Store, id: Int) -> Result(Order, CheckoutError) {
  use existing <- result.try(get_order(db, id))
  case order.is_editable(existing) {
    True -> Ok(existing)
    False -> Error(OrderNotEditable(existing.status))
  }
}

fn get_customer(db: Store, id: Int) -> Result(Customer, CheckoutError) {
  store.get_customer(db, id) |> result.replace_error(UnknownCustomer(id))
}
