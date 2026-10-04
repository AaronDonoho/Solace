import pilot/payments.{
  Authorized, Captured, GatewayUnavailable, InvalidPaymentState,
  PartiallyRefunded, PaymentDeclined, RefundExceedsCaptured, Refunded, Voided,
}

fn gateway() {
  payments.fake_gateway()
}

pub fn authorize_test() {
  let assert Ok(p) = payments.authorize(gateway(), 7, 42, "tok_ok", 5000, 9)
  assert p.status == Authorized
  assert p.auth_id == "auth_order-42-1"
  assert p.attempts == 1
  assert p.created_at == 9
}

pub fn network_errors_are_retried_test() {
  let assert Ok(p) = payments.authorize(gateway(), 7, 42, "tok_flaky", 5000, 9)
  assert p.attempts == 2
  assert payments.authorize(gateway(), 7, 42, "tok_network", 5000, 9)
    == Error(GatewayUnavailable(3))
}

pub fn declines_are_not_retried_test() {
  assert payments.authorize(gateway(), 7, 42, "tok_decline", 5000, 9)
    == Error(PaymentDeclined("card declined", 1))
}

pub fn capture_void_refund_test() {
  let assert Ok(p) = payments.authorize(gateway(), 7, 42, "tok_ok", 5000, 9)
  let assert Ok(voided) = payments.void(p)
  assert voided.status == Voided
  assert payments.capture(gateway(), voided)
    == Error(InvalidPaymentState(Voided))

  let assert Ok(p) = payments.capture(gateway(), p)
  assert p.status == Captured
  assert p.captured_cents == 5000
  assert payments.void(p) == Error(InvalidPaymentState(Captured))

  let assert Ok(#(p, _)) = payments.refund(gateway(), p, 1200)
  assert p.status == PartiallyRefunded
  assert payments.refundable_cents(p) == 3800
  assert payments.refund(gateway(), p, 3801)
    == Error(RefundExceedsCaptured(3801, 3800))
  let assert Ok(#(p, refund_id)) = payments.refund(gateway(), p, 3800)
  assert p.status == Refunded
  assert refund_id == "ref_auth_order-42-1_3800"
}

pub fn cannot_refund_before_capture_test() {
  let assert Ok(p) = payments.authorize(gateway(), 7, 42, "tok_ok", 5000, 9)
  assert payments.refund(gateway(), p, 100)
    == Error(InvalidPaymentState(Authorized))
}
