//// Card payments through a payment gateway.
////
//// A payment is authorized when the order is paid, captured when the first
//// shipment leaves, and voided if the order is cancelled before capture.
//// Captured payments can be refunded in part or in full.

import gleam/int
import gleam/string

pub type GatewayError {
  Declined(reason: String)
  NetworkError
  InvalidAmount
}

/// The payment provider. Passed in, so tests use `fake_gateway`.
pub type Gateway {
  Gateway(
    /// (card token, amount in cents, idempotency key) -> authorization id
    authorize: fn(String, Int, String) -> Result(String, GatewayError),
    /// (authorization id, amount) -> Nil
    capture: fn(String, Int) -> Result(Nil, GatewayError),
    /// (authorization id, amount) -> refund id
    refund: fn(String, Int) -> Result(String, GatewayError),
  )
}

pub type PaymentStatus {
  Authorized
  Captured
  PartiallyRefunded
  Refunded
  Voided
}

pub type Payment {
  Payment(
    id: Int,
    order_id: Int,
    auth_id: String,
    amount_cents: Int,
    captured_cents: Int,
    refunded_cents: Int,
    status: PaymentStatus,
    /// Gateway attempts it took to authorize.
    attempts: Int,
    created_at: Int,
  )
}

pub type PaymentError {
  PaymentDeclined(reason: String, attempts: Int)
  GatewayUnavailable(attempts: Int)
  RefundExceedsCaptured(requested: Int, refundable: Int)
  InvalidPaymentState(status: PaymentStatus)
  GatewayFailure(GatewayError)
}

/// Network errors are retried up to this many attempts in total.
pub const max_attempts = 3

/// A deterministic gateway for tests and local runs. Card tokens:
/// `"tok_decline"` is declined, `"tok_network"` always fails with a network
/// error, `"tok_flaky"` fails once per idempotency key ending in `-1`, and
/// anything else succeeds.
pub fn fake_gateway() -> Gateway {
  Gateway(
    authorize: fn(token, amount, key) {
      case token, amount {
        _, amount if amount <= 0 -> Error(InvalidAmount)
        "tok_decline", _ -> Error(Declined("card declined"))
        "tok_network", _ -> Error(NetworkError)
        "tok_flaky", _ ->
          case string.ends_with(key, "-1") {
            True -> Error(NetworkError)
            False -> Ok("auth_" <> key)
          }
        _, _ -> Ok("auth_" <> key)
      }
    },
    capture: fn(_auth, amount) {
      case amount > 0 {
        True -> Ok(Nil)
        False -> Error(InvalidAmount)
      }
    },
    refund: fn(auth, amount) {
      case amount > 0 {
        True -> Ok("ref_" <> auth <> "_" <> int.to_string(amount))
        False -> Error(InvalidAmount)
      }
    },
  )
}

pub fn idempotency_key(order_id: Int, attempt: Int) -> String {
  "order-" <> int.to_string(order_id) <> "-" <> int.to_string(attempt)
}

/// Authorizes `amount_cents`, retrying network errors.
pub fn authorize(
  gateway: Gateway,
  id: Int,
  order_id: Int,
  card_token: String,
  amount_cents: Int,
  now: Int,
) -> Result(Payment, PaymentError) {
  authorize_attempt(gateway, id, order_id, card_token, amount_cents, now, 1)
}

fn authorize_attempt(
  gateway: Gateway,
  id: Int,
  order_id: Int,
  token: String,
  amount: Int,
  now: Int,
  attempt: Int,
) -> Result(Payment, PaymentError) {
  case gateway.authorize(token, amount, idempotency_key(order_id, attempt)) {
    Ok(auth_id) ->
      Ok(Payment(
        id:,
        order_id:,
        auth_id:,
        amount_cents: amount,
        captured_cents: 0,
        refunded_cents: 0,
        status: Authorized,
        attempts: attempt,
        created_at: now,
      ))
    Error(NetworkError) if attempt < max_attempts ->
      authorize_attempt(gateway, id, order_id, token, amount, now, attempt + 1)
    Error(NetworkError) -> Error(GatewayUnavailable(attempt))
    Error(Declined(reason)) -> Error(PaymentDeclined(reason, attempt))
    Error(other) -> Error(GatewayFailure(other))
  }
}

/// Captures the full authorized amount.
pub fn capture(
  gateway: Gateway,
  payment: Payment,
) -> Result(Payment, PaymentError) {
  case payment.status {
    Authorized ->
      case gateway.capture(payment.auth_id, payment.amount_cents) {
        Ok(Nil) ->
          Ok(
            Payment(
              ..payment,
              status: Captured,
              captured_cents: payment.amount_cents,
            ),
          )
        Error(e) -> Error(GatewayFailure(e))
      }
    status -> Error(InvalidPaymentState(status))
  }
}

/// Releases an authorization that was never captured.
pub fn void(payment: Payment) -> Result(Payment, PaymentError) {
  case payment.status {
    Authorized -> Ok(Payment(..payment, status: Voided))
    status -> Error(InvalidPaymentState(status))
  }
}

pub fn refundable_cents(payment: Payment) -> Int {
  payment.captured_cents - payment.refunded_cents
}

/// Refunds part or all of a captured payment. Returns the updated payment
/// and the gateway's refund id.
pub fn refund(
  gateway: Gateway,
  payment: Payment,
  amount_cents: Int,
) -> Result(#(Payment, String), PaymentError) {
  let refundable = refundable_cents(payment)
  case payment.status {
    Captured | PartiallyRefunded if amount_cents > refundable ->
      Error(RefundExceedsCaptured(amount_cents, refundable))
    Captured | PartiallyRefunded ->
      case gateway.refund(payment.auth_id, amount_cents) {
        Ok(refund_id) -> {
          let refunded = payment.refunded_cents + amount_cents
          let status = case refunded == payment.captured_cents {
            True -> Refunded
            False -> PartiallyRefunded
          }
          Ok(#(Payment(..payment, refunded_cents: refunded, status:), refund_id))
        }
        Error(e) -> Error(GatewayFailure(e))
      }
    status -> Error(InvalidPaymentState(status))
  }
}

pub fn describe_error(error: PaymentError) -> String {
  case error {
    PaymentDeclined(reason, _) -> "Payment declined: " <> reason
    GatewayUnavailable(attempts) ->
      "Payment provider unavailable after "
      <> int.to_string(attempts)
      <> " attempts"
    RefundExceedsCaptured(requested, refundable) ->
      "Refund of "
      <> int.to_string(requested)
      <> " exceeds refundable "
      <> int.to_string(refundable)
    InvalidPaymentState(_) -> "Payment is not in a state that allows this"
    GatewayFailure(_) -> "Payment provider error"
  }
}
