//// Returns: requesting, approving, receiving and refunding returned items.

import gleam/dict
import gleam/list
import gleam/result
import pilot/audit
import pilot/catalog
import pilot/clock.{type Clock}
import pilot/inventory
import pilot/notifications
import pilot/payments.{type Gateway, type PaymentError}
import pilot/return_policy.{
  type Item, type Reason, type ReturnError, type ReturnRequest,
}
import pilot/store.{type Store}

pub type ReturnsError {
  UnknownOrder(id: Int)
  UnknownReturn(id: Int)
  UnknownCustomer(id: Int)
  Policy(ReturnError)
  MissingQuote(order_id: Int)
  MissingPayment(order_id: Int)
  RefundFailed(PaymentError)
}

const actor = "returns"

/// Opens a return for items of a delivered order and works out the refund.
pub fn request(
  db: Store,
  clock: Clock,
  order_id: Int,
  items: List(Item),
  reason: Reason,
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  let now = clock()
  use existing <- result.try(
    store.get_order(db, order_id)
    |> result.replace_error(UnknownOrder(order_id)),
  )
  let already = store.returned_quantities(db, order_id)
  use items <- result.try(
    return_policy.validate(existing, items, already, now)
    |> result.map_error(Policy),
  )
  use quote <- result.try(
    store.get_quote(db, order_id)
    |> result.replace_error(MissingQuote(order_id)),
  )
  let categories =
    items
    |> list.filter_map(fn(i) {
      catalog.category_of(db.products, i.sku)
      |> result.map(fn(c) { #(i.sku, c) })
    })
    |> dict.from_list
  let whole = return_policy.completes_order(existing, items, already)
  let refund =
    return_policy.refund_amount(
      existing,
      quote,
      items,
      categories,
      reason,
      whole,
    )
  let #(db, id) = store.next_id(db)
  let request =
    return_policy.ReturnRequest(
      id:,
      order_id:,
      items:,
      reason:,
      status: return_policy.Requested,
      refund:,
      requested_at: now,
    )
  let db =
    db
    |> store.put_return(request)
    |> store.record(
      now,
      actor,
      "request",
      audit.return_subject(id),
      return_policy.reason_name(reason),
    )
  Ok(#(db, request))
}

pub fn approve(
  db: Store,
  clock: Clock,
  return_id: Int,
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  update(db, clock, return_id, "approve", return_policy.approve)
}

pub fn reject(
  db: Store,
  clock: Clock,
  return_id: Int,
  reason: String,
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  update(db, clock, return_id, "reject", return_policy.reject(_, reason))
}

/// Records the returned items arriving. They go back into stock at the
/// warehouse they shipped from.
pub fn receive(
  db: Store,
  clock: Clock,
  return_id: Int,
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  let now = clock()
  use request <- result.try(get_return(db, return_id))
  use received <- result.try(
    return_policy.mark_received(request) |> result.map_error(Policy),
  )
  use customer <- result.try(customer_for(db, request.order_id))
  let sent = store.shipments_for_order(db, request.order_id)
  let stock =
    list.fold(request.items, db.stock, fn(stock, item) {
      let warehouse =
        sent
        |> list.find(fn(s) { list.any(s.items, fn(i) { i.sku == item.sku }) })
        |> result.map(fn(s) { s.warehouse_id })
        |> result.lazy_or(fn() {
          db.warehouses |> list.first |> result.map(fn(w) { w.id })
        })
        |> result.unwrap("")
      inventory.receive(stock, warehouse, item.sku, item.quantity)
    })
  let db =
    db
    |> store.set_stock(stock)
    |> store.put_return(received)
    |> store.send(notifications.return_received(customer, received, now))
    |> store.record(now, actor, "receive", audit.return_subject(return_id), "")
  Ok(#(db, received))
}

/// Refunds a received return to the customer's card.
pub fn refund(
  db: Store,
  clock: Clock,
  gateway: Gateway,
  return_id: Int,
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  let now = clock()
  use request <- result.try(get_return(db, return_id))
  use refunded <- result.try(
    return_policy.mark_refunded(request) |> result.map_error(Policy),
  )
  use customer <- result.try(customer_for(db, request.order_id))
  use payment <- result.try(
    store.payment_for_order(db, request.order_id)
    |> result.replace_error(MissingPayment(request.order_id)),
  )
  let amount = request.refund.total_cents
  use #(payment, refund_id) <- result.try(
    payments.refund(gateway, payment, amount) |> result.map_error(RefundFailed),
  )
  let db =
    db
    |> store.put_payment(payment)
    |> store.put_return(refunded)
    |> store.send(notifications.refund_issued(
      customer,
      request.order_id,
      amount,
      now,
    ))
    |> store.record(
      now,
      actor,
      "refund",
      audit.return_subject(return_id),
      refund_id,
    )
  Ok(#(db, refunded))
}

fn update(
  db: Store,
  clock: Clock,
  return_id: Int,
  action: String,
  change: fn(ReturnRequest) -> Result(ReturnRequest, ReturnError),
) -> Result(#(Store, ReturnRequest), ReturnsError) {
  use request <- result.try(get_return(db, return_id))
  use updated <- result.try(change(request) |> result.map_error(Policy))
  let db =
    db
    |> store.put_return(updated)
    |> store.record(clock(), actor, action, audit.return_subject(return_id), "")
  Ok(#(db, updated))
}

fn get_return(db: Store, id: Int) -> Result(ReturnRequest, ReturnsError) {
  store.get_return(db, id) |> result.replace_error(UnknownReturn(id))
}

fn customer_for(db: Store, order_id: Int) {
  use existing <- result.try(
    store.get_order(db, order_id)
    |> result.replace_error(UnknownOrder(order_id)),
  )
  store.get_customer(db, existing.customer_id)
  |> result.replace_error(UnknownCustomer(existing.customer_id))
}
