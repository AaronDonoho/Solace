import gleam/json
import gleam/option.{None, Some}
import migration/api

pub fn migration_decoder_test() {
  let assert Ok(value) =
    json.parse(
      "{\"state\":\"in_progress\",\"top\":\"/work\",\"original\":null,\"copy\":\"/copy\",\"step\":null,\"problem\":null,\"candidates\":[{\"dir\":\".\",\"name\":\"demo\"}],\"report\":{\"checked_at\":\"today\",\"current\":true,\"checks\":[{\"name\":\"paths\",\"status\":\"verified\",\"summary\":\"safe\",\"items\":[{\"text\":\"file\",\"status\":null,\"detail\":null}]}]}}",
      api.migration_decoder(),
    )
  assert value.original == ""
  assert value.step == ""
  assert value.problem == ""
  assert value.candidates == [#(".", "demo")]
  assert value.flagged == []
  assert value.blockers == []
  assert value.undo == False
  let assert Some(report) = value.report
  assert report.checked_at == "today"
  assert report.warnings == []
  assert report.checks
    == [
      api.MigrationCheck("paths", "verified", "safe", [
        api.MigrationItem("file", "", ""),
      ]),
    ]
}

pub fn migration_decoder_defaults_test() {
  let assert Ok(value) =
    json.parse(
      "{\"state\":\"none\",\"top\":\"/work\",\"report\":null}",
      api.migration_decoder(),
    )
  assert value.report == None
  assert value.candidates == []
  assert value.copy == ""
  let assert Error(_) =
    json.parse("{\"top\":\"/work\"}", api.migration_decoder())
}

pub fn migration_decoder_seconds_test() {
  let assert Ok(value) =
    json.parse(
      "{\"state\":\"adopted\",\"top\":\"/work\",\"report\":{\"checked_at\":0,\"current\":false,\"checks\":[],\"warnings\":[\"look\"]}}",
      api.migration_decoder(),
    )
  let assert Some(report) = value.report
  assert report.checked_at != ""
  assert report.warnings == ["look"]
}
