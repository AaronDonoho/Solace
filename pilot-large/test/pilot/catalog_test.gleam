import gleam/dict
import gleam/list
import gleam/option.{None}
import pilot/catalog.{Product, Variant}
import pilot/seed

fn products() {
  seed.products() |> list.map(fn(p) { #(p.id, p) }) |> dict.from_list
}

pub fn find_variant_test() {
  let assert Ok(#(product, variant)) =
    catalog.find_variant(products(), "TEE-M-BLK")
  assert product.id == "tee"
  assert variant.price_cents == 1800
  assert catalog.find_variant(products(), "NOPE")
    == Error(catalog.UnknownSku("NOPE"))
}

pub fn inactive_products_are_not_sellable_test() {
  assert catalog.find_sellable(products(), "CAP-RED")
    == Error(catalog.InactiveProduct("retired-cap"))
}

pub fn search_ignores_case_and_inactive_test() {
  let names = catalog.search(products(), "SHIRT") |> list.map(fn(p) { p.id })
  assert names == ["tee"]
  assert catalog.search(products(), "cap") == []
}

pub fn in_category_test() {
  let ids =
    catalog.in_category(products(), catalog.Books) |> list.map(fn(p) { p.id })
  assert ids == ["novel", "cookbook"]
}

pub fn validate_test() {
  let good =
    Product("socks", "Socks", catalog.Apparel, True, [
      Variant("SOCK-1", None, None, 900, 50),
    ])
  assert catalog.validate(good, products()) == Ok(good)
  let taken =
    Product(..good, variants: [Variant("TEE-M-BLK", None, None, 900, 50)])
  let assert Error(catalog.InvalidProduct("socks", _)) =
    catalog.validate(taken, products())
  let free = Product(..good, variants: [Variant("SOCK-1", None, None, 0, 50)])
  let assert Error(catalog.InvalidProduct(_, _)) =
    catalog.validate(free, products())
  let assert Error(_) =
    catalog.validate(Product(..good, variants: []), products())
}
