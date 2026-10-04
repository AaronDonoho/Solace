//// Products, their variants and categories.

import gleam/dict.{type Dict}
import gleam/list
import gleam/option.{type Option}
import gleam/result
import gleam/string

pub type Category {
  Apparel
  Electronics
  HomeGoods
  Books
}

/// A sellable version of a product, identified by its SKU.
pub type Variant {
  Variant(
    sku: String,
    size: Option(String),
    colour: Option(String),
    price_cents: Int,
    weight_grams: Int,
  )
}

pub type Product {
  Product(
    id: String,
    name: String,
    category: Category,
    active: Bool,
    variants: List(Variant),
  )
}

pub type CatalogError {
  UnknownSku(sku: String)
  InactiveProduct(id: String)
  InvalidProduct(id: String, reason: String)
}

/// Finds the product and variant for a SKU.
pub fn find_variant(
  products: Dict(String, Product),
  sku: String,
) -> Result(#(Product, Variant), CatalogError) {
  products
  |> dict.values
  |> list.find_map(fn(product) {
    product.variants
    |> list.find(fn(v) { v.sku == sku })
    |> result.map(fn(v) { #(product, v) })
  })
  |> result.replace_error(UnknownSku(sku))
}

/// Like `find_variant`, but fails for inactive products.
pub fn find_sellable(
  products: Dict(String, Product),
  sku: String,
) -> Result(#(Product, Variant), CatalogError) {
  use #(product, variant) <- result.try(find_variant(products, sku))
  case product.active {
    True -> Ok(#(product, variant))
    False -> Error(InactiveProduct(product.id))
  }
}

pub fn category_of(
  products: Dict(String, Product),
  sku: String,
) -> Result(Category, CatalogError) {
  find_variant(products, sku) |> result.map(fn(pair) { { pair.0 }.category })
}

/// Active products whose name contains `query`, ignoring case, sorted by name.
pub fn search(products: Dict(String, Product), query: String) -> List(Product) {
  let query = string.lowercase(query)
  products
  |> dict.values
  |> list.filter(fn(p) {
    p.active && string.contains(string.lowercase(p.name), query)
  })
  |> list.sort(fn(a, b) { string.compare(a.name, b.name) })
}

pub fn in_category(
  products: Dict(String, Product),
  category: Category,
) -> List(Product) {
  products
  |> dict.values
  |> list.filter(fn(p) { p.active && p.category == category })
  |> list.sort(fn(a, b) { string.compare(a.name, b.name) })
}

/// Checks a product before it is added to the catalog.
pub fn validate(
  product: Product,
  existing: Dict(String, Product),
) -> Result(Product, CatalogError) {
  let invalid = fn(reason) { Error(InvalidProduct(product.id, reason)) }
  let skus = list.map(product.variants, fn(v) { v.sku })
  let taken =
    list.filter(skus, fn(sku) {
      case find_variant(existing, sku) {
        Ok(#(owner, _)) -> owner.id != product.id
        Error(_) -> False
      }
    })
  case product.variants, taken {
    _, _ if product.name == "" -> invalid("name is empty")
    [], _ -> invalid("product has no variants")
    _, [sku, ..] -> invalid("SKU " <> sku <> " belongs to another product")
    variants, [] ->
      case list.find(variants, fn(v) { v.price_cents <= 0 }) {
        Ok(v) -> invalid("variant " <> v.sku <> " has no price")
        Error(_) ->
          case list.length(list.unique(skus)) == list.length(skus) {
            True -> Ok(product)
            False -> invalid("duplicate SKU")
          }
      }
  }
}

pub fn category_name(category: Category) -> String {
  case category {
    Apparel -> "Apparel"
    Electronics -> "Electronics"
    HomeGoods -> "Home goods"
    Books -> "Books"
  }
}
