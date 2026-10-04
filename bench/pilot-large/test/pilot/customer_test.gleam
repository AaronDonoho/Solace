import pilot/customer.{Address, Customer, Domestic, International}

fn customer(spend: Int) {
  Customer(
    1,
    "Ada",
    "Ada@Example.com ",
    Address("1 Main", "Albany", "12207", Domestic("NY")),
    spend,
    0,
  )
}

pub fn tiers_test() {
  assert customer.tier(customer(0)) == customer.Standard
  assert customer.tier(customer(25_000)) == customer.Silver
  assert customer.tier(customer(99_999)) == customer.Silver
  assert customer.tier(customer(100_000)) == customer.Gold
  assert customer.loyalty_discount_percent(customer.Gold) == 5
}

pub fn record_spend_test() {
  assert { customer.record_spend(customer(100), 50) }.lifetime_spend_cents
    == 150
}

pub fn validate_normalizes_email_test() {
  let assert Ok(c) = customer.validate(customer(0))
  assert c.email == "ada@example.com"
}

pub fn validate_collects_every_error_test() {
  let bad =
    Customer(
      ..customer(0),
      name: " ",
      email: "nope",
      shipping_address: Address("x", "y", "12", Domestic("NY")),
    )
  assert customer.validate(bad)
    == Error([
      customer.MissingName,
      customer.InvalidEmail("nope"),
      customer.InvalidPostalCode("12"),
    ])
}

pub fn international_postal_codes_are_free_form_test() {
  let c =
    Customer(
      ..customer(0),
      shipping_address: Address("x", "Berlin", "10115 B", International("DE")),
    )
  let assert Ok(_) = customer.validate(c)
}

pub fn email_check_test() {
  assert customer.is_valid_email("a@b.co")
  assert !customer.is_valid_email("@b.co")
  assert !customer.is_valid_email("a@b")
  assert !customer.is_valid_email("a@.co")
  assert !customer.is_valid_email("a@b@c.co")
}
