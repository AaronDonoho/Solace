//// The service's request handler. Each request is one typed operation;
//// responses carry an HTTP-style status and a JSON body.

import gleam/int
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import pilot/catalog
import pilot/checkout.{type CheckoutError}
import pilot/clock.{type Clock}
import pilot/date.{type Date}
import pilot/fulfillment.{type FulfillmentError}
import pilot/inventory
import pilot/order.{type Order}
import pilot/payments.{type Gateway}
import pilot/pricing.{type Quote}
import pilot/promotions
import pilot/reporting
import pilot/return_policy
import pilot/returns.{type ReturnsError}
import pilot/shipments.{type Shipment}
import pilot/shipping.{type Method}
import pilot/store.{type Store}

/// What the handler needs from the outside world.
pub type Env {
  Env(clock: Clock, gateway: Gateway)
}

pub type Request {
  CreateOrder(customer_id: Int)
  AddItem(order_id: Int, sku: String, quantity: Int)
  RemoveItem(order_id: Int, sku: String)
  ChangeQuantity(order_id: Int, sku: String, quantity: Int)
  ApplyCoupon(order_id: Int, code: String)
  RemoveCoupon(order_id: Int)
  GetQuote(order_id: Int, method: Method)
  SubmitOrder(order_id: Int)
  PayOrder(order_id: Int, card_token: String, method: Method)
  CancelOrder(order_id: Int, reason: String)
  GetOrder(order_id: Int)
  ListOrders(customer_id: Int)
  StartPicking(order_id: Int, method: Method)
  PackShipment(shipment_id: Int)
  ShipShipment(shipment_id: Int)
  DeliverShipment(shipment_id: Int)
  RequestReturn(order_id: Int, items: List(#(String, Int)), reason: String)
  ApproveReturn(return_id: Int)
  ReceiveReturn(return_id: Int)
  RefundReturn(return_id: Int)
  SalesReport(from: String, to: String)
  SearchProducts(query: String)
}

pub type Response {
  Response(status: Int, body: String)
}

pub fn handle(db: Store, env: Env, request: Request) -> #(Store, Response) {
  let clock = env.clock
  case request {
    CreateOrder(customer_id) ->
      checkout.create_order(db, clock, customer_id) |> order_result(db, 201)
    AddItem(order_id, sku, quantity) ->
      checkout.add_line(db, clock, order_id, sku, quantity)
      |> order_result(db, 200)
    RemoveItem(order_id, sku) ->
      checkout.remove_line(db, clock, order_id, sku) |> order_result(db, 200)
    ChangeQuantity(order_id, sku, quantity) ->
      checkout.set_quantity(db, clock, order_id, sku, quantity)
      |> order_result(db, 200)
    ApplyCoupon(order_id, code) ->
      checkout.apply_coupon(db, clock, order_id, code) |> order_result(db, 200)
    RemoveCoupon(order_id) ->
      checkout.remove_coupon(db, clock, order_id) |> order_result(db, 200)
    GetQuote(order_id, method) ->
      case checkout.quote(db, order_id, method) {
        Ok(quote) -> #(db, Response(200, quote_json(quote)))
        Error(e) -> #(db, checkout_error(e))
      }
    SubmitOrder(order_id) ->
      checkout.submit(db, clock, order_id) |> order_result(db, 200)
    PayOrder(order_id, token, method) ->
      checkout.pay(db, clock, env.gateway, order_id, token, method)
      |> order_result(db, 200)
    CancelOrder(order_id, reason) ->
      checkout.cancel(db, clock, order_id, reason) |> order_result(db, 200)
    GetOrder(order_id) ->
      case store.get_order(db, order_id) {
        Ok(o) -> #(db, Response(200, order_json(o)))
        Error(Nil) -> #(db, not_found("order", int.to_string(order_id)))
      }
    ListOrders(customer_id) ->
      case store.get_customer(db, customer_id) {
        Ok(_) -> {
          let orders = store.orders_for_customer(db, customer_id)
          #(db, Response(200, array(list.map(orders, order_json))))
        }
        Error(Nil) -> #(db, not_found("customer", int.to_string(customer_id)))
      }
    StartPicking(order_id, method) ->
      fulfillment.start_picking(db, clock, order_id, method)
      |> shipment_result(db, 201)
    PackShipment(id) ->
      fulfillment.pack(db, clock, id) |> shipment_result(db, 200)
    ShipShipment(id) ->
      fulfillment.ship(db, clock, env.gateway, id) |> shipment_result(db, 200)
    DeliverShipment(id) ->
      fulfillment.deliver(db, clock, id) |> shipment_result(db, 200)
    RequestReturn(order_id, items, reason) -> {
      let items = list.map(items, fn(i) { return_policy.Item(i.0, i.1) })
      returns.request(db, clock, order_id, items, parse_reason(reason))
      |> return_result(db, 201)
    }
    ApproveReturn(id) ->
      returns.approve(db, clock, id) |> return_result(db, 200)
    ReceiveReturn(id) ->
      returns.receive(db, clock, id) |> return_result(db, 200)
    RefundReturn(id) ->
      returns.refund(db, clock, env.gateway, id) |> return_result(db, 200)
    SalesReport(from, to) ->
      case parse_date(from), parse_date(to) {
        Ok(from), Ok(to) -> #(db, Response(200, sales_json(db, from, to)))
        _, _ -> #(db, error(422, "invalid_date", "Dates must be YYYY-MM-DD"))
      }
    SearchProducts(query) -> {
      let results =
        catalog.search(db.products, query)
        |> list.map(fn(p) {
          object([
            #("id", text(p.id)),
            #("name", text(p.name)),
            #("category", text(catalog.category_name(p.category))),
            #(
              "in_stock",
              bool(
                list.any(p.variants, fn(v) {
                  inventory.total_available(db.stock, v.sku) > 0
                }),
              ),
            ),
          ])
        })
      #(db, Response(200, array(results)))
    }
  }
}

// --- results and errors ---------------------------------------------------

fn order_result(
  result: Result(#(Store, Order), CheckoutError),
  db: Store,
  status: Int,
) -> #(Store, Response) {
  case result {
    Ok(#(db, o)) -> #(db, Response(status, order_json(o)))
    Error(e) -> #(db, checkout_error(e))
  }
}

fn shipment_result(
  result: Result(#(Store, Shipment), FulfillmentError),
  db: Store,
  status: Int,
) -> #(Store, Response) {
  case result {
    Ok(#(db, s)) -> #(db, Response(status, shipment_json(s)))
    Error(e) -> #(db, fulfillment_error(e))
  }
}

fn return_result(
  result: Result(#(Store, return_policy.ReturnRequest), ReturnsError),
  db: Store,
  status: Int,
) -> #(Store, Response) {
  case result {
    Ok(#(db, r)) -> #(db, Response(status, return_json(r)))
    Error(e) -> #(db, returns_error(e))
  }
}

pub fn checkout_error(e: CheckoutError) -> Response {
  case e {
    checkout.UnknownCustomer(id) -> not_found("customer", int.to_string(id))
    checkout.UnknownOrder(id) -> not_found("order", int.to_string(id))
    checkout.UnknownCoupon(code) -> not_found("coupon", code)
    checkout.Catalog(catalog.UnknownSku(sku)) -> not_found("product", sku)
    checkout.Catalog(catalog.InactiveProduct(id)) ->
      error(422, "product_unavailable", "Product " <> id <> " is not available")
    checkout.Catalog(catalog.InvalidProduct(id, reason)) ->
      error(422, "invalid_product", id <> ": " <> reason)
    checkout.InvalidQuantity(q) ->
      error(422, "invalid_quantity", "Invalid quantity " <> int.to_string(q))
    checkout.OrderNotEditable(status) ->
      error(409, "order_locked", "Order is " <> order.status_name(status))
    checkout.LineNotFound(sku) -> not_found("order line", sku)
    checkout.CouponRejected(reason) -> coupon_error(reason)
    checkout.Inventory(inventory.InsufficientStock(sku, requested, available)) ->
      error(
        409,
        "out_of_stock",
        "Only "
          <> int.to_string(available)
          <> " of "
          <> sku
          <> " available, "
          <> int.to_string(requested)
          <> " requested",
      )
    checkout.Inventory(_) -> error(500, "inventory_error", "Inventory error")
    checkout.Payment(payments.PaymentDeclined(reason, _)) ->
      error(402, "payment_declined", reason)
    checkout.Payment(payments.GatewayUnavailable(_) as e) ->
      error(503, "payment_unavailable", payments.describe_error(e))
    checkout.Payment(e) ->
      error(502, "payment_error", payments.describe_error(e))
    checkout.Rejected(order.EmptyOrder) ->
      error(422, "empty_order", "Order has no items")
    checkout.Rejected(order.InvalidTransition(from, _)) ->
      error(
        409,
        "invalid_state",
        "Not allowed while order is " <> order.status_name(from),
      )
    checkout.NoWarehouse ->
      error(500, "no_warehouse", "No warehouse configured")
    checkout.NoPayment(id) -> not_found("payment for order", int.to_string(id))
  }
}

fn coupon_error(e: promotions.CouponError) -> Response {
  case e {
    promotions.CouponInactive(code) ->
      error(422, "coupon_inactive", code <> " is not active")
    promotions.CouponExpired(code) ->
      error(422, "coupon_expired", code <> " has expired")
    promotions.BelowMinimumSpend(code, min) ->
      error(
        422,
        "coupon_minimum",
        code
          <> " needs a subtotal of at least "
          <> int.to_string(min)
          <> " cents",
      )
    promotions.UsageLimitReached(code) ->
      error(422, "coupon_used", code <> " has already been used")
    promotions.NotApplicable(code, reason) ->
      error(422, "coupon_not_applicable", code <> " " <> reason)
  }
}

pub fn fulfillment_error(e: FulfillmentError) -> Response {
  case e {
    fulfillment.UnknownOrder(id) -> not_found("order", int.to_string(id))
    fulfillment.UnknownShipment(id) -> not_found("shipment", int.to_string(id))
    fulfillment.UnknownCustomer(id) -> not_found("customer", int.to_string(id))
    fulfillment.OrderRejected(order.InvalidTransition(from, _)) ->
      error(
        409,
        "invalid_state",
        "Not allowed while order is " <> order.status_name(from),
      )
    fulfillment.OrderRejected(order.EmptyOrder) ->
      error(422, "empty_order", "Order has no items")
    fulfillment.ShipmentFailed(shipments.MultipleWarehouses(ids)) ->
      error(
        409,
        "split_shipment",
        "Stock is spread across " <> string.join(ids, ", "),
      )
    fulfillment.ShipmentFailed(shipments.NothingToShip) ->
      error(409, "nothing_to_ship", "Order has no reserved stock")
    fulfillment.ShipmentFailed(shipments.InvalidShipmentState(_, action)) ->
      error(409, "invalid_state", "Shipment cannot " <> action <> " now")
    fulfillment.InventoryFailed(_) ->
      error(409, "inventory_error", "Stock problem")
    fulfillment.PaymentFailed(e) ->
      error(502, "payment_error", payments.describe_error(e))
    fulfillment.MissingPayment(id) ->
      not_found("payment for order", int.to_string(id))
    fulfillment.NotAllocated(_, warehouse, sku) ->
      error(409, "not_allocated", sku <> " is not reserved at " <> warehouse)
  }
}

pub fn returns_error(e: ReturnsError) -> Response {
  case e {
    returns.UnknownOrder(id) -> not_found("order", int.to_string(id))
    returns.UnknownReturn(id) -> not_found("return", int.to_string(id))
    returns.UnknownCustomer(id) -> not_found("customer", int.to_string(id))
    returns.Policy(return_policy.NotDelivered(_)) ->
      error(409, "not_delivered", "Order has not been delivered")
    returns.Policy(return_policy.WindowClosed(days)) ->
      error(
        422,
        "return_window_closed",
        "Delivered " <> int.to_string(days) <> " days ago",
      )
    returns.Policy(return_policy.EmptyReturn) ->
      error(422, "empty_return", "No items to return")
    returns.Policy(return_policy.ItemNotInOrder(sku)) ->
      not_found("order line", sku)
    returns.Policy(return_policy.QuantityExceedsPurchased(sku, max)) ->
      error(
        422,
        "return_quantity",
        "At most " <> int.to_string(max) <> " of " <> sku <> " can be returned",
      )
    returns.Policy(return_policy.InvalidReturnState(_)) ->
      error(409, "invalid_state", "Return cannot do that now")
    returns.MissingQuote(id) -> not_found("quote for order", int.to_string(id))
    returns.MissingPayment(id) ->
      not_found("payment for order", int.to_string(id))
    returns.RefundFailed(e) ->
      error(502, "payment_error", payments.describe_error(e))
  }
}

fn not_found(what: String, id: String) -> Response {
  error(404, "not_found", "No " <> what <> " " <> id)
}

fn error(status: Int, code: String, message: String) -> Response {
  Response(
    status,
    object([#("error", text(code)), #("message", text(message))]),
  )
}

// --- parsing --------------------------------------------------------------

pub fn parse_reason(reason: String) -> return_policy.Reason {
  case string.lowercase(string.trim(reason)) {
    "damaged" -> return_policy.Damaged
    "wrong item" | "wrong_item" -> return_policy.WrongItem
    "no longer needed" | "unwanted" -> return_policy.NoLongerNeeded
    other -> return_policy.Other(other)
  }
}

/// Parses `YYYY-MM-DD`.
pub fn parse_date(text: String) -> Result(Date, Nil) {
  case string.split(text, "-") {
    [y, m, d] ->
      case int.parse(y), int.parse(m), int.parse(d) {
        Ok(year), Ok(month), Ok(day)
          if month >= 1 && month <= 12 && day >= 1 && day <= 31
        -> Ok(date.Date(year, month, day))
        _, _, _ -> Error(Nil)
      }
    _ -> Error(Nil)
  }
}

// --- JSON -----------------------------------------------------------------

fn order_json(o: Order) -> String {
  object([
    #("id", int(o.id)),
    #("customer_id", int(o.customer_id)),
    #("status", text(order.status_name(o.status))),
    #("coupon", optional(o.coupon_code, text)),
    #(
      "lines",
      array(
        list.map(o.lines, fn(l) {
          object([
            #("sku", text(l.sku)),
            #("quantity", int(l.quantity)),
            #("unit_price_cents", int(l.unit_price_cents)),
          ])
        }),
      ),
    ),
  ])
}

fn quote_json(q: Quote) -> String {
  object([
    #("subtotal_cents", int(q.subtotal_cents)),
    #("discount_cents", int(pricing.discounts_cents(q))),
    #("shipping_cents", int(q.shipping_cents)),
    #("tax_cents", int(q.tax_cents)),
    #("total_cents", int(q.total_cents)),
  ])
}

fn shipment_json(s: Shipment) -> String {
  object([
    #("id", int(s.id)),
    #("order_id", int(s.order_id)),
    #("warehouse", text(s.warehouse_id)),
    #("method", text(shipping.method_name(s.method))),
    #("tracking_number", optional(s.tracking_number, text)),
  ])
}

fn return_json(r: return_policy.ReturnRequest) -> String {
  object([
    #("id", int(r.id)),
    #("order_id", int(r.order_id)),
    #("reason", text(return_policy.reason_name(r.reason))),
    #("refund_cents", int(r.refund.total_cents)),
  ])
}

fn sales_json(db: Store, from: Date, to: Date) -> String {
  reporting.daily_sales(db, from, to)
  |> list.map(fn(d) {
    object([
      #("date", text(date.to_iso(d.date))),
      #("orders", int(d.orders)),
      #("net_cents", int(d.net_cents)),
    ])
  })
  |> array
}

fn object(fields: List(#(String, String))) -> String {
  "{"
  <> fields
  |> list.map(fn(f) { text(f.0) <> ":" <> f.1 })
  |> string.join(",")
  <> "}"
}

fn array(items: List(String)) -> String {
  "[" <> string.join(items, ",") <> "]"
}

fn text(s: String) -> String {
  "\""
  <> s
  |> string.replace("\\", "\\\\")
  |> string.replace("\"", "\\\"")
  |> string.replace("\n", "\\n")
  <> "\""
}

fn int(n: Int) -> String {
  int.to_string(n)
}

fn bool(b: Bool) -> String {
  case b {
    True -> "true"
    False -> "false"
  }
}

fn optional(value: Option(a), encode: fn(a) -> String) -> String {
  case value {
    Some(v) -> encode(v)
    None -> "null"
  }
}
