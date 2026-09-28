//// Customer emails. Messages are built here and queued in the store's
//// outbox; delivery happens elsewhere.

import gleam/int
import gleam/list
import gleam/option.{Some}
import pilot/customer.{type Customer}
import pilot/date.{type Date}
import pilot/money
import pilot/order.{type Order}
import pilot/pricing.{type Quote}
import pilot/return_policy.{type ReturnRequest}
import pilot/shipments.{type Shipment}

pub type Kind {
  OrderConfirmation
  OrderShipped
  OrderDelivered
  OrderCancelled
  RefundIssued
  ReturnReceived
}

pub type Message {
  Message(
    to: String,
    subject: String,
    body: String,
    kind: Kind,
    order_id: Int,
    created_at: Int,
  )
}

fn order_ref(order_id: Int) -> String {
  "order #" <> int.to_string(order_id)
}

fn greeting(customer: Customer) -> String {
  "Hi " <> customer.name <> ",\n\n"
}

pub fn order_confirmation(
  customer: Customer,
  order: Order,
  quote: Quote,
  now: Int,
) -> Message {
  let lines =
    quote.lines
    |> list.map(fn(l) {
      "  "
      <> int.to_string(l.quantity)
      <> " x "
      <> l.sku
      <> "  "
      <> money.format(l.total_cents)
    })
  let discounts = pricing.discounts_cents(quote)
  let body =
    greeting(customer)
    <> "Thanks for your order. We'll email you when it ships.\n\n"
    <> join(lines)
    <> case discounts {
      0 -> ""
      d -> "  Discounts: -" <> money.format(d) <> "\n"
    }
    <> "  Shipping: "
    <> money.format(quote.shipping_cents)
    <> "\n  Tax: "
    <> money.format(quote.tax_cents)
    <> "\n  Total: "
    <> money.format(quote.total_cents)
    <> "\n"
  Message(
    to: customer.email,
    subject: "We received your " <> order_ref(order.id),
    body:,
    kind: OrderConfirmation,
    order_id: order.id,
    created_at: now,
  )
}

pub fn shipped(
  customer: Customer,
  shipment: Shipment,
  estimate: Date,
  now: Int,
) -> Message {
  let tracking = case shipment.tracking_number {
    Some(number) -> "Tracking number: " <> number <> "\n"
    _ -> ""
  }
  Message(
    to: customer.email,
    subject: "Your " <> order_ref(shipment.order_id) <> " is on its way",
    body: greeting(customer)
      <> "Your order has shipped and should arrive by "
      <> date.to_iso(estimate)
      <> ".\n"
      <> tracking,
    kind: OrderShipped,
    order_id: shipment.order_id,
    created_at: now,
  )
}

pub fn delivered(customer: Customer, order: Order, now: Int) -> Message {
  Message(
    to: customer.email,
    subject: "Your " <> order_ref(order.id) <> " was delivered",
    body: greeting(customer)
      <> "Your order was delivered. You can return items within "
      <> int.to_string(return_policy.window_days)
      <> " days.\n",
    kind: OrderDelivered,
    order_id: order.id,
    created_at: now,
  )
}

pub fn cancelled(
  customer: Customer,
  order: Order,
  reason: String,
  now: Int,
) -> Message {
  Message(
    to: customer.email,
    subject: "Your " <> order_ref(order.id) <> " was cancelled",
    body: greeting(customer) <> "Your order was cancelled: " <> reason <> ".\n",
    kind: OrderCancelled,
    order_id: order.id,
    created_at: now,
  )
}

pub fn refund_issued(
  customer: Customer,
  order_id: Int,
  amount_cents: Int,
  now: Int,
) -> Message {
  Message(
    to: customer.email,
    subject: "Refund for your " <> order_ref(order_id),
    body: greeting(customer)
      <> "We've refunded "
      <> money.format(amount_cents)
      <> " to your card. It can take 5 to 10 days to appear.\n",
    kind: RefundIssued,
    order_id:,
    created_at: now,
  )
}

pub fn return_received(
  customer: Customer,
  request: ReturnRequest,
  now: Int,
) -> Message {
  Message(
    to: customer.email,
    subject: "We received your return for " <> order_ref(request.order_id),
    body: greeting(customer)
      <> "We received your returned items. Your refund of "
      <> money.format(request.refund.total_cents)
      <> " is on its way.\n",
    kind: ReturnReceived,
    order_id: request.order_id,
    created_at: now,
  )
}

fn join(lines: List(String)) -> String {
  list.fold(lines, "", fn(acc, l) { acc <> l <> "\n" })
}
