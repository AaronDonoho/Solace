//// Shipping methods, rates and delivery estimates.

import pilot/customer.{type Region, Domestic, International}
import pilot/date.{type Date}

pub type Method {
  Standard
  Express
}

pub type Zone {
  /// Warehouse and destination in the same state.
  Local
  National
  Overseas
}

pub type ShippingQuote {
  ShippingQuote(method: Method, zone: Zone, cost_cents: Int, business_days: Int)
}

/// Standard shipping is free within the US at or above this goods value.
const free_standard_threshold_cents = 7500

pub fn zone(origin_state: String, destination: Region) -> Zone {
  case destination {
    Domestic(state) if state == origin_state -> Local
    Domestic(_) -> National
    International(_) -> Overseas
  }
}

/// Price for the first kilogram, and for each started kilogram after it.
fn rates(method: Method, zone: Zone) -> #(Int, Int) {
  case method, zone {
    Standard, Local -> #(400, 100)
    Standard, National -> #(600, 150)
    Standard, Overseas -> #(1800, 500)
    Express, Local -> #(1200, 200)
    Express, National -> #(1700, 300)
    Express, Overseas -> #(4200, 900)
  }
}

pub fn business_days(method: Method, zone: Zone) -> Int {
  case method, zone {
    Standard, Local -> 2
    Standard, National -> 4
    Standard, Overseas -> 10
    Express, Local -> 1
    Express, National -> 2
    Express, Overseas -> 5
  }
}

pub fn cost_cents(method: Method, zone: Zone, weight_grams: Int) -> Int {
  let #(first, extra) = rates(method, zone)
  let extra_kgs = case weight_grams > 1000 {
    True -> { weight_grams - 1000 + 999 } / 1000
    False -> 0
  }
  first + extra * extra_kgs
}

pub fn quote(
  method: Method,
  origin_state: String,
  destination: Region,
  weight_grams: Int,
  goods_cents: Int,
) -> ShippingQuote {
  let zone = zone(origin_state, destination)
  let free =
    method == Standard
    && zone != Overseas
    && goods_cents >= free_standard_threshold_cents
  let cost = case free {
    True -> 0
    False -> cost_cents(method, zone, weight_grams)
  }
  ShippingQuote(
    method:,
    zone:,
    cost_cents: cost,
    business_days: business_days(method, zone),
  )
}

/// Expected delivery date for a shipment sent on `shipped_on`.
pub fn estimate_delivery(shipped_on: Date, quote: ShippingQuote) -> Date {
  date.add_business_days(shipped_on, quote.business_days)
}

pub fn method_name(method: Method) -> String {
  case method {
    Standard -> "standard"
    Express -> "express"
  }
}
