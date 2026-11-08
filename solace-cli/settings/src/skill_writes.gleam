//// The queue of skills requests and the role-card ticks shown over the server's skills.

import gleam/list
import gleam/option.{type Option, None, Some}

/// One role-card tick for a skill.
pub type SkillTick {
  SkillTick(name: String, role: String, chosen: Bool)
}

/// A skills request held until the one in flight has a reply.
pub type SkillWrite {
  TickWrite(SkillTick)
  SaveWrite(String)
  DeleteWrite(String)
  LoadWrite
}

/// Starts a request, or holds it until the one in flight has a reply.
pub fn start(
  writing: Option(SkillWrite),
  waiting: List(SkillWrite),
  write: SkillWrite,
) -> #(Option(SkillWrite), List(SkillWrite)) {
  case writing {
    None -> #(Some(write), waiting)
    Some(current) -> #(Some(current), list.append(waiting, [write]))
  }
}

/// Starts the next held request after the one in flight has a reply.
pub fn next(
  waiting: List(SkillWrite),
) -> #(Option(SkillWrite), List(SkillWrite)) {
  case waiting {
    [] -> #(None, [])
    [write, ..rest] -> #(Some(write), rest)
  }
}

/// Adds a reload unless one is already in flight or waiting.
pub fn queue_load(
  writing: Option(SkillWrite),
  waiting: List(SkillWrite),
) -> #(Option(SkillWrite), List(SkillWrite)) {
  case has_load(writing) || list.any(waiting, is_load) {
    True -> #(writing, waiting)
    False -> #(writing, list.append(waiting, [LoadWrite]))
  }
}

/// The ticks shown over the server's skills, in the order they were made.
pub fn ticks(
  writing: Option(SkillWrite),
  waiting: List(SkillWrite),
) -> List(SkillTick) {
  let in_flight = case writing {
    Some(write) -> ticks_for([write])
    None -> []
  }
  list.append(in_flight, ticks_for(waiting))
}

fn has_load(writing: Option(SkillWrite)) -> Bool {
  case writing {
    Some(write) -> is_load(write)
    None -> False
  }
}

fn is_load(write: SkillWrite) -> Bool {
  case write {
    LoadWrite -> True
    _ -> False
  }
}

fn ticks_for(writes: List(SkillWrite)) -> List(SkillTick) {
  list.fold(writes, [], fn(ticks, write) {
    case write {
      TickWrite(tick) -> list.append(ticks, [tick])
      SaveWrite(_) -> ticks
      DeleteWrite(_) -> ticks
      LoadWrite -> ticks
    }
  })
}
