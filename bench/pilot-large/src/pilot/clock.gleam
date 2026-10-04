/// Source of the current time, in Unix milliseconds.
pub type Clock =
  fn() -> Int

pub fn fixed(at: Int) -> Clock {
  fn() { at }
}
