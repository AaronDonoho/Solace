//// In-memory persistence for everything the service stores. Immutable:
//// every write returns a new Store.

import gleam/dict.{type Dict}
import gleam/int
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import pilot/audit.{type Entry}
import pilot/catalog.{type Product}
import pilot/customer.{type Customer}
import pilot/inventory.{type Allocation, type Stock, type Warehouse}
import pilot/notifications.{type Message}
import pilot/order.{type Order}
import pilot/payments.{type Payment}
import pilot/pricing.{type Quote}
import pilot/promotions.{type Coupon}
import pilot/return_policy.{type ReturnRequest}
import pilot/shipments.{type Shipment}

pub type Store {
  Store(
    products: Dict(String, Product),
    customers: Dict(Int, Customer),
    orders: Dict(Int, Order),
    warehouses: List(Warehouse),
    stock: Stock,
    coupons: Dict(String, Coupon),
    /// Times each (coupon code, customer id) has been used on a paid order.
    coupon_uses: Dict(#(String, Int), Int),
    /// The quote each order was paid at.
    quotes: Dict(Int, Quote),
    /// Stock reserved for each paid order.
    allocations: Dict(Int, List(Allocation)),
    payments: Dict(Int, Payment),
    shipments: Dict(Int, Shipment),
    returns: Dict(Int, ReturnRequest),
    /// Newest first.
    outbox: List(Message),
    /// Newest first.
    audit_log: List(Entry),
    next_id: Int,
  )
}

pub fn new() -> Store {
  Store(
    products: dict.new(),
    customers: dict.new(),
    orders: dict.new(),
    warehouses: [],
    stock: inventory.new(),
    coupons: dict.new(),
    coupon_uses: dict.new(),
    quotes: dict.new(),
    allocations: dict.new(),
    payments: dict.new(),
    shipments: dict.new(),
    returns: dict.new(),
    outbox: [],
    audit_log: [],
    next_id: 1,
  )
}

/// A fresh id, shared by all entity kinds.
pub fn next_id(db: Store) -> #(Store, Int) {
  #(Store(..db, next_id: db.next_id + 1), db.next_id)
}

// --- writes ---------------------------------------------------------------

pub fn put_product(db: Store, product: Product) -> Store {
  Store(..db, products: dict.insert(db.products, product.id, product))
}

pub fn put_customer(db: Store, customer: Customer) -> Store {
  Store(..db, customers: dict.insert(db.customers, customer.id, customer))
}

pub fn put_order(db: Store, order: Order) -> Store {
  Store(..db, orders: dict.insert(db.orders, order.id, order))
}

pub fn put_warehouse(db: Store, warehouse: Warehouse) -> Store {
  let others = list.filter(db.warehouses, fn(w) { w.id != warehouse.id })
  Store(..db, warehouses: list.append(others, [warehouse]))
}

pub fn put_coupon(db: Store, coupon: Coupon) -> Store {
  let code = promotions.normalize_code(coupon.code)
  Store(
    ..db,
    coupons: dict.insert(db.coupons, code, promotions.Coupon(..coupon, code:)),
  )
}

pub fn put_payment(db: Store, payment: Payment) -> Store {
  Store(..db, payments: dict.insert(db.payments, payment.id, payment))
}

pub fn put_shipment(db: Store, shipment: Shipment) -> Store {
  Store(..db, shipments: dict.insert(db.shipments, shipment.id, shipment))
}

pub fn put_return(db: Store, request: ReturnRequest) -> Store {
  Store(..db, returns: dict.insert(db.returns, request.id, request))
}

pub fn put_quote(db: Store, order_id: Int, quote: Quote) -> Store {
  Store(..db, quotes: dict.insert(db.quotes, order_id, quote))
}

pub fn set_stock(db: Store, stock: Stock) -> Store {
  Store(..db, stock:)
}

pub fn set_allocations(
  db: Store,
  order_id: Int,
  allocations: List(Allocation),
) -> Store {
  Store(..db, allocations: dict.insert(db.allocations, order_id, allocations))
}

pub fn clear_allocations(db: Store, order_id: Int) -> Store {
  Store(..db, allocations: dict.delete(db.allocations, order_id))
}

pub fn count_coupon_use(db: Store, code: String, customer_id: Int) -> Store {
  Store(
    ..db,
    coupon_uses: dict.upsert(db.coupon_uses, #(code, customer_id), fn(n) {
      case n {
        Some(n) -> n + 1
        None -> 1
      }
    }),
  )
}

pub fn send(db: Store, message: Message) -> Store {
  Store(..db, outbox: [message, ..db.outbox])
}

pub fn record(
  db: Store,
  at: Int,
  actor: String,
  action: String,
  subject: String,
  detail: String,
) -> Store {
  Store(..db, audit_log: [
    audit.Entry(at:, actor:, action:, subject:, detail:),
    ..db.audit_log
  ])
}

// --- reads ----------------------------------------------------------------

pub fn get_customer(db: Store, id: Int) -> Result(Customer, Nil) {
  dict.get(db.customers, id)
}

pub fn get_order(db: Store, id: Int) -> Result(Order, Nil) {
  dict.get(db.orders, id)
}

pub fn get_product(db: Store, id: String) -> Result(Product, Nil) {
  dict.get(db.products, id)
}

pub fn get_coupon(db: Store, code: String) -> Result(Coupon, Nil) {
  dict.get(db.coupons, promotions.normalize_code(code))
}

pub fn get_payment(db: Store, id: Int) -> Result(Payment, Nil) {
  dict.get(db.payments, id)
}

pub fn get_shipment(db: Store, id: Int) -> Result(Shipment, Nil) {
  dict.get(db.shipments, id)
}

pub fn get_return(db: Store, id: Int) -> Result(ReturnRequest, Nil) {
  dict.get(db.returns, id)
}

pub fn get_quote(db: Store, order_id: Int) -> Result(Quote, Nil) {
  dict.get(db.quotes, order_id)
}

pub fn allocations_for(db: Store, order_id: Int) -> List(Allocation) {
  dict.get(db.allocations, order_id) |> result.unwrap([])
}

pub fn coupon_uses(db: Store, code: String, customer_id: Int) -> Int {
  dict.get(db.coupon_uses, #(promotions.normalize_code(code), customer_id))
  |> result.unwrap(0)
}

/// The order's payment that is still live (not voided), if any.
pub fn payment_for_order(db: Store, order_id: Int) -> Result(Payment, Nil) {
  db.payments
  |> dict.values
  |> list.find(fn(p) { p.order_id == order_id && p.status != payments.Voided })
}

pub fn shipments_for_order(db: Store, order_id: Int) -> List(Shipment) {
  db.shipments
  |> dict.values
  |> list.filter(fn(s) { s.order_id == order_id })
  |> list.sort(fn(a, b) { int.compare(a.id, b.id) })
}

pub fn returns_for_order(db: Store, order_id: Int) -> List(ReturnRequest) {
  db.returns
  |> dict.values
  |> list.filter(fn(r) { r.order_id == order_id })
  |> list.sort(fn(a, b) { int.compare(a.id, b.id) })
}

pub fn orders_for_customer(db: Store, customer_id: Int) -> List(Order) {
  db.orders
  |> dict.values
  |> list.filter(fn(o) { o.customer_id == customer_id })
  |> list.sort(fn(a, b) { int.compare(b.created_at, a.created_at) })
}

/// Units per SKU already returned on an order, counting every return that
/// hasn't been rejected.
pub fn returned_quantities(db: Store, order_id: Int) -> Dict(String, Int) {
  returns_for_order(db, order_id)
  |> list.filter(fn(r) {
    case r.status {
      return_policy.Rejected(_) -> False
      _ -> True
    }
  })
  |> list.flat_map(fn(r) { r.items })
  |> list.fold(dict.new(), fn(acc, item) {
    dict.upsert(acc, item.sku, fn(n) {
      case n {
        Some(n) -> n + item.quantity
        None -> item.quantity
      }
    })
  })
}

/// Messages in the order they were sent.
pub fn sent_messages(db: Store) -> List(Message) {
  list.reverse(db.outbox)
}

/// Audit entries, oldest first.
pub fn audit_entries(db: Store) -> List(Entry) {
  list.reverse(db.audit_log)
}
