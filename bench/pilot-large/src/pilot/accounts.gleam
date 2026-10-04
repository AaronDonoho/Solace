//// Customer accounts: registration and profile changes.

import gleam/dict
import gleam/list
import gleam/result
import pilot/audit
import pilot/clock.{type Clock}
import pilot/customer.{type Address, type Customer, type CustomerError}
import pilot/store.{type Store}

pub type AccountError {
  Invalid(List(CustomerError))
  EmailTaken(email: String)
  UnknownAccount(id: Int)
}

const actor = "accounts"

pub fn register(
  db: Store,
  clock: Clock,
  name: String,
  email: String,
  address: Address,
) -> Result(#(Store, Customer), AccountError) {
  let now = clock()
  let #(next, id) = store.next_id(db)
  use customer <- result.try(
    customer.Customer(
      id:,
      name:,
      email:,
      shipping_address: address,
      lifetime_spend_cents: 0,
      created_at: now,
    )
    |> customer.validate
    |> result.map_error(Invalid),
  )
  use <- guard_email_free(db, customer.email, id)
  let db =
    next
    |> store.put_customer(customer)
    |> store.record(
      now,
      actor,
      "register",
      audit.customer_subject(id),
      customer.email,
    )
  Ok(#(db, customer))
}

pub fn change_email(
  db: Store,
  clock: Clock,
  customer_id: Int,
  email: String,
) -> Result(#(Store, Customer), AccountError) {
  use existing <- result.try(get(db, customer_id))
  use updated <- result.try(
    customer.Customer(..existing, email:)
    |> customer.validate
    |> result.map_error(Invalid),
  )
  use <- guard_email_free(db, updated.email, customer_id)
  let db =
    db
    |> store.put_customer(updated)
    |> store.record(
      clock(),
      actor,
      "change email",
      audit.customer_subject(customer_id),
      updated.email,
    )
  Ok(#(db, updated))
}

/// Changes the default shipping address. Orders already created keep the
/// address they were created with.
pub fn change_address(
  db: Store,
  clock: Clock,
  customer_id: Int,
  address: Address,
) -> Result(#(Store, Customer), AccountError) {
  use existing <- result.try(get(db, customer_id))
  use updated <- result.try(
    customer.Customer(..existing, shipping_address: address)
    |> customer.validate
    |> result.map_error(Invalid),
  )
  let db =
    db
    |> store.put_customer(updated)
    |> store.record(
      clock(),
      actor,
      "change address",
      audit.customer_subject(customer_id),
      "",
    )
  Ok(#(db, updated))
}

pub fn find_by_email(db: Store, email: String) -> Result(Customer, Nil) {
  let email = customer.normalize_email(email)
  db.customers |> dict.values |> list.find(fn(c) { c.email == email })
}

fn get(db: Store, id: Int) -> Result(Customer, AccountError) {
  store.get_customer(db, id) |> result.replace_error(UnknownAccount(id))
}

fn guard_email_free(
  db: Store,
  email: String,
  owner: Int,
  continue: fn() -> Result(a, AccountError),
) -> Result(a, AccountError) {
  case find_by_email(db, email) {
    Ok(other) if other.id != owner -> Error(EmailTaken(email))
    _ -> continue()
  }
}
