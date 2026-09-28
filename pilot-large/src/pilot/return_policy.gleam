//// Return eligibility and refund amounts.
////
//// Delivered orders can be returned within 30 days of delivery. The refund
//// for returned items is what the customer paid for them: the item price,
//// less the items' share of order discounts, plus their share of tax, less
//// any restocking fee. Shipping is refunded only when the whole order has
//// been returned.

import gleam/bool
import gleam/dict.{type Dict}
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import pilot/catalog.{type Category, Electronics}
import pilot/date
import pilot/money
import pilot/order.{type Order, Delivered}
import pilot/pricing.{type Quote}

pub type Reason {
  Damaged
  WrongItem
  NoLongerNeeded
  Other(String)
}

pub type Item {
  Item(sku: String, quantity: Int)
}

pub type Status {
  Requested
  Approved
  Received
  Refunded
  Rejected(reason: String)
}

pub type RefundBreakdown {
  RefundBreakdown(
    items_cents: Int,
    discount_share_cents: Int,
    tax_share_cents: Int,
    restocking_fee_cents: Int,
    shipping_cents: Int,
    total_cents: Int,
  )
}

pub type ReturnRequest {
  ReturnRequest(
    id: Int,
    order_id: Int,
    items: List(Item),
    reason: Reason,
    status: Status,
    refund: RefundBreakdown,
    requested_at: Int,
  )
}

pub type ReturnError {
  NotDelivered(status: order.Status)
  WindowClosed(days_since_delivery: Int)
  EmptyReturn
  ItemNotInOrder(sku: String)
  QuantityExceedsPurchased(sku: String, returnable: Int)
  InvalidReturnState(status: Status)
}

pub const window_days = 30

pub fn check_window(order: Order, now: Int) -> Result(Nil, ReturnError) {
  case order.status, order.delivered_at {
    Delivered, Some(delivered_at) -> {
      let days =
        date.days_between(
          date.from_unix_ms(delivered_at),
          date.from_unix_ms(now),
        )
      case days <= window_days {
        True -> Ok(Nil)
        False -> Error(WindowClosed(days))
      }
    }
    status, _ -> Error(NotDelivered(status))
  }
}

/// Checks a return against the order and what has already been returned
/// (quantities per SKU). Duplicate SKUs in `items` are combined.
pub fn validate(
  order: Order,
  items: List(Item),
  already_returned: Dict(String, Int),
  now: Int,
) -> Result(List(Item), ReturnError) {
  use _ <- result.try(check_window(order, now))
  let items = combine(items)
  use <- bool.guard(items == [], Error(EmptyReturn))
  list.try_map(items, fn(item) {
    case order.find_line(order, item.sku) {
      Error(Nil) -> Error(ItemNotInOrder(item.sku))
      Ok(line) -> {
        let returned = dict.get(already_returned, item.sku) |> result.unwrap(0)
        let returnable = line.quantity - returned
        case item.quantity > 0 && item.quantity <= returnable {
          True -> Ok(item)
          False -> Error(QuantityExceedsPurchased(item.sku, returnable))
        }
      }
    }
  })
}

fn combine(items: List(Item)) -> List(Item) {
  items
  |> list.fold(dict.new(), fn(acc, i) {
    dict.upsert(acc, i.sku, fn(q) {
      case q {
        Some(q) -> q + i.quantity
        None -> i.quantity
      }
    })
  })
  |> dict.to_list
  |> list.filter(fn(pair) { pair.1 > 0 })
  |> list.map(fn(pair) { Item(pair.0, pair.1) })
  |> list.sort(fn(a, b) { string.compare(a.sku, b.sku) })
}

pub fn restocking_fee_percent(category: Category, reason: Reason) -> Int {
  case reason, category {
    Damaged, _ | WrongItem, _ -> 0
    _, Electronics -> 15
    _, _ -> 0
  }
}

/// True when, after this return, every unit of the order has been returned.
pub fn completes_order(
  order: Order,
  items: List(Item),
  already_returned: Dict(String, Int),
) -> Bool {
  list.all(order.lines, fn(line) {
    let before = dict.get(already_returned, line.sku) |> result.unwrap(0)
    let now =
      items
      |> list.filter(fn(i) { i.sku == line.sku })
      |> list.fold(0, fn(sum, i) { sum + i.quantity })
    before + now >= line.quantity
  })
}

pub fn refund_amount(
  order: Order,
  quote: Quote,
  items: List(Item),
  categories: Dict(String, Category),
  reason: Reason,
  whole_order: Bool,
) -> RefundBreakdown {
  let priced =
    list.map(items, fn(item) {
      let price = case order.find_line(order, item.sku) {
        Ok(line) -> line.unit_price_cents
        Error(Nil) -> 0
      }
      #(item, price * item.quantity)
    })
  let items_cents = money.sum(list.map(priced, fn(p) { p.1 }))
  let discount_share =
    money.apportion(
      pricing.discounts_cents(quote),
      items_cents,
      quote.subtotal_cents,
    )
  let goods = pricing.goods_cents(quote)
  let tax_share =
    money.apportion(quote.tax_cents, items_cents - discount_share, goods)
  let fee =
    priced
    |> list.map(fn(p) {
      let category =
        dict.get(categories, { p.0 }.sku) |> result.unwrap(catalog.HomeGoods)
      money.percent(p.1, restocking_fee_percent(category, reason))
    })
    |> money.sum
  let shipping = case whole_order {
    True -> quote.shipping_cents
    False -> 0
  }
  RefundBreakdown(
    items_cents:,
    discount_share_cents: discount_share,
    tax_share_cents: tax_share,
    restocking_fee_cents: fee,
    shipping_cents: shipping,
    total_cents: items_cents - discount_share + tax_share - fee + shipping,
  )
}

pub fn reason_name(reason: Reason) -> String {
  case reason {
    Damaged -> "damaged"
    WrongItem -> "wrong item"
    NoLongerNeeded -> "no longer needed"
    Other(text) -> text
  }
}

pub fn approve(request: ReturnRequest) -> Result(ReturnRequest, ReturnError) {
  move(request, Requested, Approved)
}

pub fn mark_received(
  request: ReturnRequest,
) -> Result(ReturnRequest, ReturnError) {
  move(request, Approved, Received)
}

pub fn mark_refunded(
  request: ReturnRequest,
) -> Result(ReturnRequest, ReturnError) {
  move(request, Received, Refunded)
}

pub fn reject(
  request: ReturnRequest,
  reason: String,
) -> Result(ReturnRequest, ReturnError) {
  case request.status {
    Requested | Approved ->
      Ok(ReturnRequest(..request, status: Rejected(reason)))
    status -> Error(InvalidReturnState(status))
  }
}

fn move(
  request: ReturnRequest,
  from: Status,
  to: Status,
) -> Result(ReturnRequest, ReturnError) {
  case request.status == from {
    True -> Ok(ReturnRequest(..request, status: to))
    False -> Error(InvalidReturnState(request.status))
  }
}
