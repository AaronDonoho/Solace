import pilot/money

pub fn percent_rounds_down_test() {
  assert money.percent(999, 10) == 99
  assert money.percent(7400, 5) == 370
}

pub fn basis_points_round_half_up_test() {
  assert money.basis_points(7030, 725) == 510
  assert money.basis_points(200, 725) == 15
  assert money.basis_points(0, 725) == 0
}

pub fn apportion_test() {
  assert money.apportion(1000, 1, 4) == 250
  assert money.apportion(1000, 5, 0) == 0
}

pub fn format_test() {
  assert money.format(5) == "$0.05"
  assert money.format(123_456) == "$1,234.56"
  assert money.format(-1000) == "-$10.00"
  assert money.format(100_000_000) == "$1,000,000.00"
}
