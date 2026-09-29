import ewe
import gleam/bit_array
import gleam/dynamic/decode
import gleam/erlang/process
import gleam/http.{Get, Post}
import gleam/http/request.{type Request}
import gleam/http/response.{type Response}
import gleam/int
import gleam/io
import gleam/json
import gleam/list
import gleam/result
import gleam/string
import simplifile

pub type Group {
  Group(id: String, name: String, paths: List(String))
}

type Loaded {
  Loaded(rows: List(String), errors: List(String), seen: List(String))
}

type Entry {
  Entry(name: String, path: String, kind: String)
}

pub fn main() {
  let server =
    ewe.new(handler: handle)
    |> ewe.bind(to: "127.0.0.1")
  let started = case server |> ewe.listening(on: 8765) |> ewe.start {
    Ok(started) -> Ok(started)
    Error(_) -> server |> ewe.listening_random |> ewe.start
  }
  let assert Ok(started) = started
  let assert ewe.TcpSocketAddress(_, port) = started.data
  io.println("Bench visualizer: http://127.0.0.1:" <> int.to_string(port))
  process.sleep_forever()
}

fn handle(req: Request(ewe.Connection)) -> Response(ewe.Body) {
  case req.method, request.path_segments(req) {
    Get, [] ->
      case simplifile.read("priv/index.html") {
        Ok(html) -> reply(200, html, "text/html")
        Error(error) ->
          reply(500, simplifile.describe_error(error), "text/plain")
      }
    Get, ["app.js"] ->
      case simplifile.read("priv/app.js") {
        Ok(javascript) -> reply(200, javascript, "text/javascript")
        Error(error) ->
          reply(500, simplifile.describe_error(error), "text/plain")
      }
    Get, ["api", "browse"] -> browse(req)
    Post, ["api", "results"] -> results(req)
    _, _ -> reply(404, "{\"error\":\"Not found\"}", "application/json")
  }
}

fn browse(req: Request(ewe.Connection)) -> Response(ewe.Body) {
  let raw = case request.get_query(req) {
    Ok(query) ->
      query |> list.key_find("path") |> result.unwrap("bench/results")
    Error(_) -> "bench/results"
  }
  use root <- with_root
  case safe_path(raw, root) {
    Error(message) -> json_error(400, message)
    Ok(path) ->
      case simplifile.is_directory(path), simplifile.read_directory(path) {
        Ok(True), Ok(names) -> {
          let entries =
            names
            |> list.sort(string.compare)
            |> list.filter_map(entry(path, root, _))
          let parent = case path == root {
            True -> json.null()
            False -> json.string(relative(parent(path), root))
          }
          json.object([
            #("path", json.string(relative(path, root))),
            #("parent", parent),
            #("entries", json.array(entries, entry_json)),
          ])
          |> json.to_string
          |> reply(200, _, "application/json")
        }
        Ok(False), _ -> json_error(400, "Can only browse directories")
        Error(error), _ -> json_error(400, simplifile.describe_error(error))
        _, Error(error) -> json_error(400, simplifile.describe_error(error))
      }
  }
}

fn results(req: Request(ewe.Connection)) -> Response(ewe.Body) {
  use root <- with_root
  case ewe.read_body(req, limit: 1_000_000) {
    Error(ewe.BodyTooLarge) -> json_error(400, "Request is too large")
    Error(ewe.InvalidBody) -> json_error(400, "Could not read request body")
    Ok(req) ->
      case bit_array.to_string(req.body) {
        Error(_) -> json_error(400, "Request body must be UTF-8")
        Ok(body) ->
          case json.parse(from: body, using: groups_decoder()) {
            Error(_) -> json_error(400, "Expected a groups array")
            Ok(groups) ->
              reply(200, load_groups(groups, root), "application/json")
          }
      }
  }
}

fn with_root(next: fn(String) -> Response(ewe.Body)) -> Response(ewe.Body) {
  case simplifile.resolve("../..") {
    Ok(root) -> next(root)
    Error(error) -> json_error(500, simplifile.describe_error(error))
  }
}

fn groups_decoder() -> decode.Decoder(List(Group)) {
  use groups <- decode.field("groups", decode.list(group_decoder()))
  decode.success(groups)
}

fn group_decoder() -> decode.Decoder(Group) {
  use id <- decode.field("id", decode.string)
  use name <- decode.field("name", decode.string)
  use paths <- decode.field("paths", decode.list(decode.string))
  decode.success(Group(id:, name:, paths:))
}

fn load_groups(groups: List(Group), root: String) -> String {
  let loaded =
    list.fold(groups, Loaded([], [], []), fn(state, group) {
      list.fold(group.paths, state, fn(state, path) {
        load_path(state, group, root, path)
      })
    })
  "{\"rows\":["
  <> string.join(loaded.rows, ",")
  <> "],\"errors\":["
  <> string.join(list.map(loaded.errors, quote), ",")
  <> "]}"
}

fn load_path(state: Loaded, group: Group, root: String, raw: String) -> Loaded {
  case safe_path(raw, root) {
    Error(message) -> Loaded(..state, errors: [message, ..state.errors])
    Ok(path) ->
      case result_files(path) {
        Error(message) -> Loaded(..state, errors: [message, ..state.errors])
        Ok(files) ->
          list.fold(files, state, fn(state, file) {
            load_file(state, group, root, file)
          })
      }
  }
}

fn load_file(
  state: Loaded,
  group: Group,
  root: String,
  file: String,
) -> Loaded {
  case list.contains(state.seen, file), safe_path(file, root) {
    True, _ | _, Error(_) -> state
    False, Ok(file) ->
      case simplifile.read(file) {
        Error(error) ->
          Loaded(..state, errors: [
            relative(file, root) <> ": " <> simplifile.describe_error(error),
            ..state.errors
          ])
        Ok(contents) -> {
          let source = relative(parent(file), root)
          let rows =
            contents
            |> string.split("\n")
            |> list.map(string.trim)
            |> list.filter(is_result_row)
            |> list.map(contextualize(_, group.id, group.name, source))
          Loaded(
            rows: list.append(state.rows, rows),
            errors: state.errors,
            seen: [file, ..state.seen],
          )
        }
      }
  }
}

fn result_files(path: String) -> Result(List(String), String) {
  case simplifile.is_directory(path) {
    Error(error) -> Error(simplifile.describe_error(error))
    Ok(False) -> Ok([path])
    Ok(True) ->
      case simplifile.get_files(path) {
        Error(error) -> Error(simplifile.describe_error(error))
        Ok(files) -> {
          let summaries =
            list.filter(files, string.ends_with(_, "/summary.jsonl"))
          case list.is_empty(summaries) {
            False -> Ok(summaries)
            True ->
              Ok(
                list.filter(files, fn(file) {
                  string.ends_with(file, ".jsonl")
                  || string.ends_with(file, ".json")
                }),
              )
          }
        }
      }
  }
}

pub fn is_result_row(row: String) -> Bool {
  case json.parse(from: row, using: task_decoder()) {
    Ok(_) -> True
    Error(_) -> False
  }
}

pub fn contextualize(
  row: String,
  id: String,
  name: String,
  source: String,
) -> String {
  "{\"group_id\":"
  <> quote(id)
  <> ",\"group\":"
  <> quote(name)
  <> ",\"source\":"
  <> quote(source)
  <> ","
  <> string.drop_start(row, 1)
}

fn safe_path(raw: String, root: String) -> Result(String, String) {
  let candidate = case string.starts_with(raw, "/") {
    True -> raw
    False -> root <> "/" <> raw
  }
  use path <- result.try(
    simplifile.resolve(candidate)
    |> result.map_error(fn(_) { "Path does not exist: " <> raw }),
  )
  let inside = path == root || string.starts_with(path, root <> "/")
  case inside, symlink_free(path, root), simplifile.exists(path, True) {
    False, _, _ -> Error("Paths must stay inside the repository")
    _, False, _ -> Error("Symbolic-link paths are not allowed")
    _, _, Ok(True) -> Ok(path)
    _, _, _ -> Error("Path does not exist: " <> raw)
  }
}

fn symlink_free(path: String, root: String) -> Bool {
  path
  |> relative(root)
  |> string.split("/")
  |> symlink_segments(root)
}

fn symlink_segments(segments: List(String), current: String) -> Bool {
  case segments {
    [] | [""] -> True
    [segment, ..rest] -> {
      let next = current <> "/" <> segment
      case simplifile.is_symlink(next) {
        Ok(False) -> symlink_segments(rest, next)
        _ -> False
      }
    }
  }
}

fn entry(directory: String, root: String, name: String) -> Result(Entry, Nil) {
  let path = directory <> "/" <> name
  case safe_path(path, root), simplifile.is_directory(path) {
    Ok(path), Ok(True) -> Ok(Entry(name, relative(path, root), "directory"))
    Ok(path), Ok(False) ->
      case string.ends_with(name, ".json") || string.ends_with(name, ".jsonl") {
        True -> Ok(Entry(name, relative(path, root), "file"))
        False -> Error(Nil)
      }
    _, _ -> Error(Nil)
  }
}

fn entry_json(entry: Entry) -> json.Json {
  json.object([
    #("name", json.string(entry.name)),
    #("path", json.string(entry.path)),
    #("kind", json.string(entry.kind)),
  ])
}

fn relative(path: String, root: String) -> String {
  case path == root {
    True -> "."
    False -> string.drop_start(path, string.length(root) + 1)
  }
}

fn parent(path: String) -> String {
  case path |> string.split("/") |> list.reverse {
    [_, ..rest] -> rest |> list.reverse |> string.join("/")
    _ -> path
  }
}

fn quote(value: String) -> String {
  value |> json.string |> json.to_string
}

fn json_error(status: Int, message: String) -> Response(ewe.Body) {
  reply(status, "{\"error\":" <> quote(message) <> "}", "application/json")
}

fn reply(
  status: Int,
  body: String,
  content_type: String,
) -> Response(ewe.Body) {
  response.new(status)
  |> response.set_header("content-type", content_type <> "; charset=utf-8")
  |> response.set_body(ewe.Text(body))
}

fn task_decoder() -> decode.Decoder(String) {
  use task <- decode.field("task", decode.string)
  decode.success(task)
}
