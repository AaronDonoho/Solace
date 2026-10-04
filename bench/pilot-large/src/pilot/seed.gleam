//// Demo data for local runs and tests: three warehouses, a small catalog,
//// customers in each loyalty tier and region, and a set of coupons.
////
//// All timestamps are relative to `epoch`, 2026-03-02 (a Monday) 00:00 UTC.

import gleam/list
import gleam/option.{None, Some}
import pilot/catalog.{Apparel, Books, Electronics, HomeGoods, Product, Variant}
import pilot/customer.{Address, Customer, Domestic, International}
import pilot/inventory.{Warehouse}
import pilot/promotions.{BuyXGetY, Coupon, FixedOff, FreeShipping, PercentOff}
import pilot/store.{type Store}

pub const epoch = 1_772_409_600_000

pub const day = 86_400_000

// Customers
pub const ada = 1

pub const grace = 2

pub const linus = 3

pub const barbara = 4

pub fn demo_store() -> Store {
  store.new()
  |> list.fold(warehouses(), _, store.put_warehouse)
  |> list.fold(products(), _, store.put_product)
  |> list.fold(customers(), _, store.put_customer)
  |> list.fold(coupons(), _, store.put_coupon)
  |> stock_up
  |> fn(db) { store.Store(..db, next_id: 100) }
}

pub fn warehouses() -> List(inventory.Warehouse) {
  [
    Warehouse(id: "WH-EAST", name: "Newark", state: "NJ"),
    Warehouse(id: "WH-WEST", name: "Oakland", state: "CA"),
    Warehouse(id: "WH-SOUTH", name: "Dallas", state: "TX"),
  ]
}

pub fn products() -> List(catalog.Product) {
  [
    Product(
      id: "tee",
      name: "Cotton T-shirt",
      category: Apparel,
      active: True,
      variants: [
        Variant("TEE-S-BLK", Some("S"), Some("black"), 1800, 180),
        Variant("TEE-M-BLK", Some("M"), Some("black"), 1800, 200),
        Variant("TEE-L-BLK", Some("L"), Some("black"), 1800, 220),
        Variant("TEE-M-WHT", Some("M"), Some("white"), 1800, 200),
      ],
    ),
    Product(
      id: "hoodie",
      name: "Fleece Hoodie",
      category: Apparel,
      active: True,
      variants: [
        Variant("HOOD-M-GRY", Some("M"), Some("grey"), 5500, 650),
        Variant("HOOD-L-GRY", Some("L"), Some("grey"), 5500, 700),
      ],
    ),
    Product(
      id: "headphones",
      name: "Wireless Headphones",
      category: Electronics,
      active: True,
      variants: [Variant("PHONES-BLK", None, Some("black"), 12_900, 450)],
    ),
    Product(
      id: "charger",
      name: "USB-C Charger",
      category: Electronics,
      active: True,
      variants: [Variant("CHARGER-65W", None, None, 3900, 300)],
    ),
    Product(
      id: "lamp",
      name: "Desk Lamp",
      category: HomeGoods,
      active: True,
      variants: [Variant("LAMP-OAK", None, Some("oak"), 7400, 2600)],
    ),
    Product(
      id: "mug",
      name: "Stoneware Mug",
      category: HomeGoods,
      active: True,
      variants: [Variant("MUG-BLUE", None, Some("blue"), 1400, 450)],
    ),
    Product(
      id: "novel",
      name: "Paperback Novel",
      category: Books,
      active: True,
      variants: [Variant("BOOK-NOVEL", None, None, 1200, 350)],
    ),
    Product(
      id: "cookbook",
      name: "Weeknight Cookbook",
      category: Books,
      active: True,
      variants: [Variant("BOOK-COOK", None, None, 3200, 900)],
    ),
    Product(
      id: "retired-cap",
      name: "Logo Cap",
      category: Apparel,
      active: False,
      variants: [Variant("CAP-RED", None, Some("red"), 2000, 120)],
    ),
  ]
}

pub fn customers() -> List(customer.Customer) {
  [
    Customer(
      id: ada,
      name: "Ada",
      email: "ada@example.com",
      shipping_address: Address("1 Main St", "Albany", "12207", Domestic("NY")),
      lifetime_spend_cents: 0,
      created_at: epoch,
    ),
    Customer(
      id: grace,
      name: "Grace",
      email: "grace@example.com",
      shipping_address: Address("9 Bay Rd", "San Jose", "95112", Domestic("CA")),
      lifetime_spend_cents: 150_000,
      created_at: epoch,
    ),
    Customer(
      id: linus,
      name: "Linus",
      email: "linus@example.com",
      shipping_address: Address(
        "Hauptstr. 5",
        "Berlin",
        "10115",
        International("DE"),
      ),
      lifetime_spend_cents: 30_000,
      created_at: epoch,
    ),
    Customer(
      id: barbara,
      name: "Barbara",
      email: "barbara@example.com",
      shipping_address: Address(
        "20 Elm Ave",
        "Portland",
        "97205",
        Domestic("OR"),
      ),
      lifetime_spend_cents: 0,
      created_at: epoch,
    ),
  ]
}

pub fn coupons() -> List(promotions.Coupon) {
  let coupon = fn(code, kind) {
    Coupon(
      code:,
      kind:,
      min_spend_cents: 0,
      expires_at: None,
      max_uses_per_customer: None,
      stacks_with_loyalty: True,
      active: True,
    )
  }
  [
    Coupon(..coupon("SAVE10", PercentOff(10)), min_spend_cents: 5000),
    coupon("TENOFF", FixedOff(1000)),
    coupon("SHIPFREE", FreeShipping),
    coupon("BOOKS3FOR2", BuyXGetY("BOOK-NOVEL", 2, 1)),
    Coupon(
      ..coupon("WELCOME15", PercentOff(15)),
      max_uses_per_customer: Some(1),
      stacks_with_loyalty: False,
    ),
    Coupon(..coupon("SPRING", PercentOff(20)), expires_at: Some(epoch)),
    Coupon(..coupon("RETIRED", FixedOff(500)), active: False),
  ]
}

/// Starting stock: every SKU at WH-EAST, a smaller range elsewhere.
fn stock_up(db: Store) -> Store {
  let levels = [
    #("WH-EAST", "TEE-S-BLK", 20),
    #("WH-EAST", "TEE-M-BLK", 20),
    #("WH-EAST", "TEE-L-BLK", 20),
    #("WH-EAST", "TEE-M-WHT", 10),
    #("WH-EAST", "HOOD-M-GRY", 8),
    #("WH-EAST", "HOOD-L-GRY", 8),
    #("WH-EAST", "PHONES-BLK", 5),
    #("WH-EAST", "CHARGER-65W", 15),
    #("WH-EAST", "LAMP-OAK", 4),
    #("WH-EAST", "MUG-BLUE", 30),
    #("WH-EAST", "BOOK-NOVEL", 40),
    #("WH-EAST", "BOOK-COOK", 12),
    #("WH-WEST", "TEE-M-BLK", 10),
    #("WH-WEST", "HOOD-M-GRY", 5),
    #("WH-WEST", "PHONES-BLK", 3),
    #("WH-WEST", "LAMP-OAK", 2),
    #("WH-WEST", "BOOK-NOVEL", 10),
    #("WH-SOUTH", "TEE-M-BLK", 5),
    #("WH-SOUTH", "MUG-BLUE", 10),
    #("WH-SOUTH", "CHARGER-65W", 6),
  ]
  store.set_stock(
    db,
    list.fold(levels, db.stock, fn(stock, l) {
      inventory.receive(stock, l.0, l.1, l.2)
    }),
  )
}
