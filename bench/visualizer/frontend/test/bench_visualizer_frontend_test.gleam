import bench_visualizer_frontend
import gleam/json
import gleeunit
import gleeunit/should

pub fn main() {
  gleeunit.main()
}

pub fn results_decoder_test() {
  let body =
    "{\"rows\":[{\"group\":\"A\",\"source\":\"bench/results/run\",\"task\":\"sample\",\"rep\":1,\"model\":\"test\",\"hidden_pass\":true,\"tokens\":7,\"duration_ms\":12,\"cost_usd\":0.5}],\"errors\":[]}"
  json.parse(from: body, using: bench_visualizer_frontend.results_decoder())
  |> should.be_ok
}
