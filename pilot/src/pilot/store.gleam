//// In-memory persistence. Immutable: every write returns a new Store.

import gleam/dict.{type Dict}
import pilot/customer.{type Customer}
import pilot/order.{type Order}
import pilot/product.{type Product}

pub type Store {
  Store(
    products: Dict(String, Product),
    customers: Dict(Int, Customer),
    orders: Dict(Int, Order),
    next_order_id: Int,
  )
}

pub fn new() -> Store {
  Store(
    products: dict.new(),
    customers: dict.new(),
    orders: dict.new(),
    next_order_id: 1,
  )
}

pub fn put_product(store: Store, product: Product) -> Store {
  Store(..store, products: dict.insert(store.products, product.sku, product))
}

pub fn put_customer(store: Store, customer: Customer) -> Store {
  Store(..store, customers: dict.insert(store.customers, customer.id, customer))
}

pub fn put_order(store: Store, order: Order) -> Store {
  Store(..store, orders: dict.insert(store.orders, order.id, order))
}

pub fn get_product(store: Store, sku: String) -> Result(Product, Nil) {
  dict.get(store.products, sku)
}

pub fn get_customer(store: Store, id: Int) -> Result(Customer, Nil) {
  dict.get(store.customers, id)
}

pub fn get_order(store: Store, id: Int) -> Result(Order, Nil) {
  dict.get(store.orders, id)
}

/// Returns the id to use for a new order and the store with the counter advanced.
pub fn allocate_order_id(store: Store) -> #(Store, Int) {
  #(Store(..store, next_order_id: store.next_order_id + 1), store.next_order_id)
}
