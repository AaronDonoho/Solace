//// Customer accounts, addresses and loyalty tiers.

import gleam/list
import gleam/string

pub type Region {
  /// A US state, by its two-letter code.
  Domestic(state: String)
  International(country: String)
}

pub type Address {
  Address(line1: String, city: String, postal_code: String, region: Region)
}

pub type LoyaltyTier {
  Standard
  Silver
  Gold
}

pub type Customer {
  Customer(
    id: Int,
    name: String,
    email: String,
    shipping_address: Address,
    lifetime_spend_cents: Int,
    created_at: Int,
  )
}

pub type CustomerError {
  MissingName
  InvalidEmail(email: String)
  InvalidPostalCode(postal_code: String)
}

const silver_threshold_cents = 25_000

const gold_threshold_cents = 100_000

/// Tier earned from lifetime spend.
pub fn tier(customer: Customer) -> LoyaltyTier {
  case customer.lifetime_spend_cents {
    spend if spend >= gold_threshold_cents -> Gold
    spend if spend >= silver_threshold_cents -> Silver
    _ -> Standard
  }
}

/// Percentage taken off the subtotal for a tier.
pub fn loyalty_discount_percent(tier: LoyaltyTier) -> Int {
  case tier {
    Gold -> 5
    Silver -> 2
    Standard -> 0
  }
}

pub fn record_spend(customer: Customer, cents: Int) -> Customer {
  Customer(
    ..customer,
    lifetime_spend_cents: customer.lifetime_spend_cents + cents,
  )
}

pub fn normalize_email(email: String) -> String {
  email |> string.trim |> string.lowercase
}

/// A deliberately simple check: one `@`, something before it, and a dot in
/// the domain.
pub fn is_valid_email(email: String) -> Bool {
  case string.split(email, "@") {
    [local, domain] ->
      local != ""
      && string.contains(domain, ".")
      && !string.starts_with(domain, ".")
      && !string.ends_with(domain, ".")
    _ -> False
  }
}

/// Validates a customer, returning every problem found.
pub fn validate(customer: Customer) -> Result(Customer, List(CustomerError)) {
  let email = normalize_email(customer.email)
  let postal = customer.shipping_address.postal_code
  let errors =
    [
      #(string.trim(customer.name) == "", MissingName),
      #(!is_valid_email(email), InvalidEmail(customer.email)),
      #(
        !valid_postal_code(postal, customer.shipping_address.region),
        InvalidPostalCode(postal),
      ),
    ]
    |> list.filter(fn(check) { check.0 })
    |> list.map(fn(check) { check.1 })
  case errors {
    [] -> Ok(Customer(..customer, email:))
    _ -> Error(errors)
  }
}

fn valid_postal_code(code: String, region: Region) -> Bool {
  case region {
    Domestic(_) ->
      string.length(code) == 5
      && list.all(string.to_graphemes(code), fn(c) {
        string.contains("0123456789", c)
      })
    International(_) -> string.trim(code) != ""
  }
}

pub fn region_name(region: Region) -> String {
  case region {
    Domestic(state) -> "US-" <> state
    International(country) -> country
  }
}
