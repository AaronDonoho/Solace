import gleam/dynamic/decode
import gleam/option.{type Option, None}

pub type MigrationItem {
  MigrationItem(text: String, status: String, detail: String)
}

pub type MigrationCheck {
  MigrationCheck(
    name: String,
    status: String,
    summary: String,
    items: List(MigrationItem),
  )
}

pub type MigrationReport {
  MigrationReport(
    checked_at: String,
    current: Bool,
    checks: List(MigrationCheck),
    warnings: List(String),
  )
}

pub type Migration {
  Migration(
    state: String,
    top: String,
    original: String,
    copy: String,
    step: String,
    candidates: List(#(String, String)),
    report: Option(MigrationReport),
    flagged: List(MigrationItem),
    problem: String,
    blockers: List(String),
    undo: Bool,
  )
}

pub fn migration_decoder() -> decode.Decoder(Migration) {
  let nullable_text =
    decode.optional(decode.string) |> decode.map(option.unwrap(_, ""))
  let candidate = {
    use dir <- decode.field("dir", decode.string)
    use name <- decode.field("name", decode.string)
    decode.success(#(dir, name))
  }
  let item = {
    use text <- decode.field("text", decode.string)
    use status <- decode.optional_field("status", "", nullable_text)
    use detail <- decode.optional_field("detail", "", nullable_text)
    decode.success(MigrationItem(text, status, detail))
  }
  let check = {
    use name <- decode.field("name", decode.string)
    use status <- decode.field("status", decode.string)
    use summary <- decode.field("summary", decode.string)
    use items <- decode.field("items", decode.list(item))
    decode.success(MigrationCheck(name, status, summary, items))
  }
  let report = {
    use checked_at <- decode.field(
      "checked_at",
      decode.one_of(decode.string, [decode.map(decode.int, clock)]),
    )
    use current <- decode.field("current", decode.bool)
    use checks <- decode.field("checks", decode.list(check))
    use warnings <- decode.optional_field(
      "warnings",
      [],
      decode.list(decode.string),
    )
    decode.success(MigrationReport(checked_at, current, checks, warnings))
  }
  use state <- decode.field("state", decode.string)
  use top <- decode.field("top", decode.string)
  use original <- decode.optional_field("original", "", nullable_text)
  use copy <- decode.optional_field("copy", "", nullable_text)
  use step <- decode.optional_field("step", "", nullable_text)
  use candidates <- decode.optional_field(
    "candidates",
    [],
    decode.list(candidate),
  )
  use report <- decode.optional_field("report", None, decode.optional(report))
  use flagged <- decode.optional_field("flagged", [], decode.list(item))
  use problem <- decode.optional_field("problem", "", nullable_text)
  use blockers <- decode.optional_field(
    "blockers",
    [],
    decode.list(decode.string),
  )
  use undo <- decode.optional_field("undo", False, decode.bool)
  decode.success(Migration(
    state,
    top,
    original,
    copy,
    step,
    candidates,
    report,
    flagged,
    problem,
    blockers,
    undo,
  ))
}

@external(javascript, "../api.mjs", "clock")
pub fn clock(seconds: Int) -> String
