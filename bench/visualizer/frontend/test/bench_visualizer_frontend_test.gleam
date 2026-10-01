import bench_visualizer_frontend
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleeunit
import gleeunit/should

pub fn main() {
  gleeunit.main()
}

pub fn results_decoder_test() {
  let body =
    "{\"rows\":[{\"group\":\"A\",\"source\":\"bench/results/run/sample\",\"task\":\"sample\",\"rep\":1,\"model\":\"test\",\"effort\":\"medium\",\"hidden_pass\":true,\"tokens\":7,\"work_tokens\":6,\"uncached_tokens\":5,\"output_tokens\":4,\"duration_ms\":12,\"cost_usd\":0.5}],\"errors\":[]}"
  let assert Ok(results) =
    json.parse(from: body, using: bench_visualizer_frontend.results_decoder())
  let assert [row] = results.rows
  row.model |> should.equal("test (medium)")
  row.tokens |> should.equal(Some(7))
  row.work_tokens |> should.equal(Some(6))
  row.uncached_tokens |> should.equal(Some(5))
  row.output_tokens |> should.equal(Some(4))
  bench_visualizer_frontend.cached_input(row) |> should.equal(Some(2))
  bench_visualizer_frontend.uncached_input(row) |> should.equal(Some(1))
}

pub fn group_names_test() {
  bench_visualizer_frontend.group_name(1) |> should.equal("A")
  bench_visualizer_frontend.group_name(26) |> should.equal("Z")
  bench_visualizer_frontend.group_name(27) |> should.equal("AA")
}

pub fn sort_average_and_format_test() {
  let body =
    "{\"rows\":[{\"group_id\":\"1\",\"group\":\"A\",\"source\":\"first\",\"task\":\"scenario\",\"rep\":1,\"model\":\"test\",\"hidden_pass\":true,\"tokens\":10000}],\"errors\":[]}"
  let assert Ok(results) =
    json.parse(from: body, using: bench_visualizer_frontend.results_decoder())
  let assert [first] = results.rows
  let second =
    bench_visualizer_frontend.Row(
      ..first,
      source: "second",
      rep: 2,
      tokens: Some(20_000),
    )
  let other_group =
    bench_visualizer_frontend.Row(
      ..first,
      group_id: "2",
      group: "B",
      source: "third",
    )
  let rows = [first, other_group, second]
  bench_visualizer_frontend.sort_rows(rows, [
    bench_visualizer_frontend.Sort("group", False),
    bench_visualizer_frontend.Sort("rep", True),
  ])
  |> list.map(fn(row) { row.source })
  |> should.equal(["second", "first", "third"])
  bench_visualizer_frontend.average(
    [first, second],
    bench_visualizer_frontend.Tokens,
  )
  |> should.equal(Some(#(15_000.0, 2)))
  bench_visualizer_frontend.format_tokens(Some(10_000))
  |> should.equal("10000")
  bench_visualizer_frontend.format_tokens(Some(10_050))
  |> should.equal("10.1k")
  bench_visualizer_frontend.format_duration(Some(1250))
  |> should.equal("1.25s")
}

pub fn row_modifier_selection_test() {
  let body =
    "{\"rows\":[{\"group_id\":\"1\",\"group\":\"A\",\"source\":\"first\",\"task\":\"scenario\",\"rep\":1,\"model\":\"test\",\"hidden_pass\":true}],\"errors\":[]}"
  let assert Ok(results) =
    json.parse(from: body, using: bench_visualizer_frontend.results_decoder())
  let assert [first] = results.rows
  let second = bench_visualizer_frontend.Row(..first, source: "second", rep: 2)
  let third = bench_visualizer_frontend.Row(..first, source: "third", rep: 3)
  let rows = [first, second, third]
  let #(single, anchor) =
    bench_visualizer_frontend.select_rows(rows, [], None, first, False, False)
  let #(range, anchor) =
    bench_visualizer_frontend.select_rows(
      rows,
      single,
      anchor,
      third,
      True,
      False,
    )
  range
  |> list.map(fn(row) { row.source })
  |> should.equal(["first", "second", "third"])
  let #(toggled, _) =
    bench_visualizer_frontend.select_rows(
      rows,
      range,
      anchor,
      second,
      False,
      True,
    )
  toggled
  |> list.map(fn(row) { row.source })
  |> should.equal(["first", "third"])
}

pub fn chart_rows_average_scope_test() {
  let first =
    bench_visualizer_frontend.Row(
      "1",
      "A",
      "first",
      "L01-first",
      1,
      "model",
      "",
      True,
      Some(10),
      None,
      None,
      None,
      None,
      None,
    )
  let second =
    bench_visualizer_frontend.Row(
      ..first,
      source: "second",
      task: "L01-variant",
      rep: 2,
      tokens: Some(30),
    )
  let other_group =
    bench_visualizer_frontend.Row(
      ..first,
      group_id: "2",
      group: "B",
      source: "third",
      task: "L01-other",
      tokens: Some(50),
    )
  let other_task =
    bench_visualizer_frontend.Row(
      ..first,
      task: "L02-other",
      source: "fourth",
      tokens: Some(50),
    )
  let rows = [first, second, other_group, other_task]
  let selected = bench_visualizer_frontend.chart_rows(rows, [first])
  selected
  |> list.map(fn(row) { row.source })
  |> should.equal(["first", "second"])
  bench_visualizer_frontend.average(selected, bench_visualizer_frontend.Tokens)
  |> should.equal(Some(#(20.0, 2)))
  bench_visualizer_frontend.chart_rows(rows, [first, other_group])
  |> list.map(fn(row) { row.source })
  |> should.equal(["first", "second", "third"])
  let selected = bench_visualizer_frontend.chart_rows(rows, [first, other_task])
  bench_visualizer_frontend.average_per_task(
    selected,
    bench_visualizer_frontend.Tokens,
  )
  |> should.equal(Some(#(35.0, 2)))
  bench_visualizer_frontend.short_task("L02-other-details")
  |> should.equal("L02")
  bench_visualizer_frontend.short_task("scenario")
  |> should.equal("scenario")
}
