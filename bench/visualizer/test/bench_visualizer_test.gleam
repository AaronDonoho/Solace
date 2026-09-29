import bench_visualizer
import gleeunit
import gleeunit/should

pub fn main() {
  gleeunit.main()
}

pub fn result_row_context_test() {
  let row = "{\"task\":\"L01\",\"tokens\":12}"
  bench_visualizer.is_result_row(row)
  |> should.be_true
  bench_visualizer.is_result_row("{\"task\":broken}")
  |> should.be_false
  bench_visualizer.contextualize(row, "a", "Files", "bench/results/run")
  |> should.equal(
    "{\"group_id\":\"a\",\"group\":\"Files\",\"source\":\"bench/results/run\",\"task\":\"L01\",\"tokens\":12}",
  )
}
