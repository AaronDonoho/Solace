import pilot/accounts.{EmailTaken, Invalid, UnknownAccount}
import pilot/customer.{Address, Domestic}
import pilot/seed
import pilot/store
import support

fn address() {
  Address("3 Oak Ln", "Austin", "73301", Domestic("TX"))
}

pub fn register_test() {
  let assert Ok(#(db, c)) =
    accounts.register(
      seed.demo_store(),
      support.on_day(0),
      "Hedy",
      " Hedy@Example.com",
      address(),
    )
  assert c.email == "hedy@example.com"
  assert c.id == 100
  assert accounts.find_by_email(db, "HEDY@example.com") == Ok(c)
  let assert [entry, ..] = db.audit_log
  assert entry.subject == "customer:100"
}

pub fn register_rejects_duplicates_and_invalid_test() {
  assert accounts.register(
      seed.demo_store(),
      support.on_day(0),
      "Ada 2",
      "ADA@example.com",
      address(),
    )
    == Error(EmailTaken("ada@example.com"))
  assert accounts.register(
      seed.demo_store(),
      support.on_day(0),
      "",
      "x@y.z",
      address(),
    )
    == Error(Invalid([customer.MissingName]))
}

pub fn change_email_and_address_test() {
  let db = seed.demo_store()
  let c = support.on_day(1)
  assert accounts.change_email(db, c, seed.ada, "grace@example.com")
    == Error(EmailTaken("grace@example.com"))
  let assert Ok(#(db, ada)) =
    accounts.change_email(db, c, seed.ada, "ada@new.example.com")
  assert ada.email == "ada@new.example.com"
  let assert Ok(#(db, _)) = accounts.change_address(db, c, seed.ada, address())
  let assert Ok(ada) = store.get_customer(db, seed.ada)
  assert ada.shipping_address == address()
  assert accounts.change_address(db, c, 999, address())
    == Error(UnknownAccount(999))
}
