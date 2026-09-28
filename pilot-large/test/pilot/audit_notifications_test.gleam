import gleam/list
import gleam/string
import pilot/audit
import pilot/notifications
import pilot/seed
import pilot/store
import support

pub fn audit_trail_test() {
  let #(db, o) = support.paid(seed.demo_store(), seed.ada, [#("MUG-BLUE", 1)])
  let actions =
    store.audit_entries(db)
    |> audit.for_subject(audit.order_subject(o.id))
    |> list.map(fn(e) { e.action })
  assert actions == ["create", "submit", "pay"]
  let assert [first, ..] = store.audit_entries(db)
  assert audit.format(first) == "2026-03-02 checkout create order:100"
  assert audit.between(store.audit_entries(db), seed.epoch + 1, seed.epoch + 2)
    == []
}

pub fn confirmation_email_test() {
  let #(db, _) = support.paid(seed.demo_store(), seed.grace, [#("LAMP-OAK", 1)])
  let assert [m] = store.sent_messages(db)
  assert m.kind == notifications.OrderConfirmation
  assert m.subject == "We received your order #100"
  assert string.contains(m.body, "1 x LAMP-OAK  $74.00")
  assert string.contains(m.body, "Discounts: -$3.70")
  assert string.contains(m.body, "Total: $81.40")
}
