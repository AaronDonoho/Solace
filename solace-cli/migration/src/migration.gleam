import gleam/dynamic/decode
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/result
import gleam/string
import gleam/uri
import lustre
import lustre/attribute as a
import lustre/component
import lustre/effect.{type Effect}
import lustre/element.{type Element}
import lustre/element/html as h
import lustre/event as e
import migration/api.{
  type Migration, type MigrationItem, type MigrationReport, Migration, clock,
  migration_decoder,
}
import solace_task

/// Standalone development mount, exercising the same custom element as the shell.
pub fn main() {
  let assert Ok(_) = register()
  let assert Ok(_) =
    lustre.element(element.element("solace-migration", [], []))
    |> lustre.start("#app", Nil)
  Nil
}

pub type Model {
  Model(
    migration: Option(Migration),
    migration_open: Bool,
    migration_projects: List(String),
    migration_smoke: String,
    migration_busy: Bool,
    migration_generation: Int,
    migration_error: String,
    migration_dismissed: Bool,
    migration_connected: Bool,
    migration_poll_id: Int,
    projects: Projects,
  )
}

pub type Msg {
  MigrationStarted(Int)
  MigrationLoaded(Int, String)
  MigrationRefresh(Int)
  MigrationConnected
  MigrationDisconnected
  ToggleMigration
  MigrationProject(String, Bool)
  MigrationSmoke(String)
  MigrationAction(String)
  MigrationChanged(Int, String)
  DismissMigration
  ProjectsLoaded(String, keep: Bool)
  AddProject(String)
  RemoveProject(String)
  RemoveOnly
  RemoveFiles
  RemoveTyped(String)
  RemoveDelete
  RemoveCancel
  ProjectName(String)
  ProjectDir(String)
  ProjectTarget(String)
  CreateProject
}

pub fn init(_flags: Nil) -> #(Model, Effect(Msg)) {
  #(
    Model(None, False, [], "", False, 0, "", False, True, 0, no_projects),
    effect.batch([
      effect.from(fn(send) { send(MigrationStarted(new_poll_id())) }),
      get("/projects", ProjectsLoaded(_, True)),
    ]),
  )
}

/// Register <solace-migration>. Reads no attributes; the API belongs to the root.
pub fn register() -> Result(Nil, lustre.Error) {
  lustre.component(init, update, view, [
    component.adopt_styles(False),
    component.on_connect(MigrationConnected),
    component.on_disconnect(MigrationDisconnected),
  ])
  |> lustre.register("solace-migration")
}

pub fn view(model: Model) -> Element(Msg) {
  h.div([a.class("migration-page")], [
    element.element("style", [], [
      h.text(styles <> solace_task.modal_styles <> projects_styles),
    ]),
    h.h1([], [h.text("Workspace")]),
    projects_view(model.projects),
    case model.migration {
      None -> h.text("")
      Some(_) ->
        h.section([a.class("bring-in")], [
          h.h2([], [h.text("Bring in an existing project")]),
          migration_view(model),
        ])
    },
  ])
}

const styles = ":host{display:block;color:var(--ink);background:var(--sheet);font:16px system-ui,sans-serif;--sheet:#f7f6f0;--paper:#fffefa;--ink:#172226;--soft:#46575a;--line:#94a7a5;--edge:#172226;--hover:#e7eeea;--focus:#174f78;--code:#eef1ec;--ok-text:#1a6b30;--warn-text:#8a4a0a;--revision:#a32628}.migration-page{padding:24px}button,input,textarea{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:4px}button{cursor:pointer;background:var(--paper)}button:hover{background:var(--hover)}button:focus-visible{outline:3px solid var(--focus)}label{display:block}textarea{display:block;width:min(95%,700px);min-height:70px}.editor{border:1px solid var(--line);padding:16px;border-radius:6px}small{display:block;color:var(--soft);overflow-wrap:anywhere}[role=alert]{color:var(--revision)}article{border-bottom:1px solid var(--line);padding:8px}li{white-space:pre-wrap;margin-bottom:8px}code{font:13px ui-monospace,monospace;background:var(--code);padding:1px 4px;border-radius:3px}.entry{margin:6px 0}.primary{background:var(--focus);color:var(--paper);border-color:var(--focus)}.primary:hover{background:var(--focus)}.diff{background:var(--code);padding:8px;border-radius:4px}input,textarea,pre,code{user-select:text}.migration{margin-bottom:16px}.migration .editor{margin:8px 0}.migration input[type=checkbox]{width:auto}.projects-pick{margin:8px 0}.projects-pick label{display:inline-block;margin-right:16px}.migration fieldset{border:1px solid var(--line);border-radius:4px}.migration code{overflow-wrap:anywhere}.migration summary{cursor:pointer;font-weight:600}.migration pre{max-height:400px;max-width:none;overflow:auto;white-space:pre;font:13px ui-monospace,monospace;user-select:text}.migration [role=alert]{white-space:pre-wrap}.migration .mark{display:inline-block;width:1.4em;text-align:center;font-weight:700;margin-right:4px}.migration .migration-verified{color:var(--ok-text)}.migration .migration-failed{color:var(--revision)}.migration .migration-not-checked{color:var(--warn-text)}.migration .migration-action{display:flex;gap:12px;align-items:baseline;margin:8px 0}.migration .migration-action button{flex:none}.migration .migration-legend{color:var(--soft)}.migration .migration-report ul{list-style:none;padding-left:0}.migration button:disabled{cursor:default;opacity:.55}.migration p{overflow-wrap:anywhere}"

pub fn update(model: Model, msg: Msg) -> #(Model, Effect(Msg)) {
  case msg {
    MigrationStarted(poll_id) ->
      case model.migration_poll_id {
        0 -> {
          let next = Model(..model, migration_poll_id: poll_id)
          case model.migration_connected {
            True -> #(
              next,
              get("/migration", MigrationLoaded(model.migration_generation, _)),
            )
            False -> #(next, effect.none())
          }
        }
        _ -> #(model, effect.none())
      }
    MigrationConnected ->
      case model.migration_connected {
        True -> #(model, effect.none())
        False -> {
          let generation = model.migration_generation + 1
          #(
            Model(
              ..model,
              migration_connected: True,
              migration_busy: False,
              migration_generation: generation,
            ),
            case model.migration_poll_id {
              0 -> get("/projects", ProjectsLoaded(_, True))
              _ ->
                effect.batch([
                  get("/migration", MigrationLoaded(generation, _)),
                  get("/projects", ProjectsLoaded(_, True)),
                ])
            },
          )
        }
      }
    MigrationDisconnected ->
      case model.migration_connected {
        False -> #(model, effect.none())
        True -> #(
          Model(
            ..model,
            migration_connected: False,
            migration_generation: model.migration_generation + 1,
          ),
          effect.from(fn(_send) { cancel_poll(model.migration_poll_id) }),
        )
      }
    MigrationRefresh(generation) ->
      case
        model.migration_connected && generation == model.migration_generation
      {
        True -> #(model, get("/migration", MigrationLoaded(generation, _)))
        False -> #(model, effect.none())
      }
    MigrationLoaded(generation, text) -> {
      case
        model.migration_connected
        && generation == model.migration_generation
        && !model.migration_busy
      {
        False -> #(model, effect.none())
        True -> {
          // Keep one polling chain, including through a server restart. A reply
          // fetched before an action was accepted must not end its progress panel.
          let next = case
            json.parse(text, migration_decoder()),
            model.migration_busy
          {
            Ok(snapshot), False -> {
              case model.migration {
                Some(old)
                  if old.state == "running"
                  && { snapshot.state != old.state || snapshot.top != old.top }
                -> reload()
                _ -> Nil
              }
              let failure =
                json.parse(text, decode.at(["error"], decode.string))
                |> result.unwrap("")
              Model(
                ..model,
                migration: Some(snapshot),
                migration_error: case failure {
                  "" -> model.migration_error
                  message -> message
                },
              )
            }
            _, _ -> model
          }
          let delay = case next.migration {
            Some(m) if m.state == "running" -> 1000
            _ -> 5000
          }
          #(
            next,
            effect.from(fn(send) {
              after(model.migration_poll_id, delay, fn() {
                send(MigrationRefresh(generation))
              })
            }),
          )
        }
      }
    }
    ToggleMigration -> {
      let projects = case model.migration {
        Some(snapshot) -> list.map(snapshot.candidates, fn(c) { c.0 })
        None -> []
      }
      #(
        Model(
          ..model,
          migration_open: !model.migration_open,
          migration_projects: projects,
        ),
        effect.none(),
      )
    }
    MigrationProject(dir, checked) -> #(
      Model(..model, migration_projects: case checked {
        True -> list.unique([dir, ..model.migration_projects])
        False -> list.filter(model.migration_projects, fn(p) { p != dir })
      }),
      effect.none(),
    )
    MigrationSmoke(text) -> #(
      Model(..model, migration_smoke: text),
      effect.none(),
    )
    DismissMigration -> #(
      Model(..model, migration_dismissed: True),
      effect.none(),
    )
    MigrationAction(action) -> migration_action(model, action)
    ProjectsLoaded(..)
    | AddProject(_)
    | RemoveProject(_)
    | RemoveOnly
    | RemoveFiles
    | RemoveTyped(_)
    | RemoveDelete
    | RemoveCancel
    | ProjectName(_)
    | ProjectDir(_)
    | ProjectTarget(_)
    | CreateProject -> {
      let #(projects, fx) = update_projects(model.projects, msg)
      #(Model(..model, projects: projects), fx)
    }
    MigrationChanged(generation, text) ->
      case
        model.migration_connected && generation == model.migration_generation
      {
        False -> #(model, effect.none())
        True -> {
          let code = json.parse(text, decode.at(["code"], decode.string))
          let next = case code, model.migration {
            Ok(code), _ ->
              Model(
                ..model,
                migration_busy: False,
                migration_error: code <> ": " <> error(text),
              )
            _, Some(snapshot) ->
              Model(
                ..model,
                migration_busy: False,
                migration_open: False,
                migration: Some(
                  Migration(
                    ..snapshot,
                    state: "running",
                    step: "Waiting for the server…",
                  ),
                ),
                migration_error: case
                  json.parse(text, decode.at(["state"], decode.string))
                {
                  Ok(_) -> ""
                  Error(_) -> error(text)
                },
              )
            _, None ->
              Model(
                ..model,
                migration_busy: False,
                migration_error: error(text),
              )
          }
          let delay = case next.migration {
            Some(m) if m.state == "running" -> 1000
            _ -> 5000
          }
          #(
            next,
            effect.from(fn(send) {
              after(model.migration_poll_id, delay, fn() {
                send(MigrationRefresh(generation))
              })
            }),
          )
        }
      }
  }
}

fn migration_action(model: Model, action: String) -> #(Model, Effect(Msg)) {
  case model.migration {
    None -> #(model, effect.none())
    Some(m) -> {
      let allowed =
        model.migration_connected
        && !model.migration_busy
        && case action {
          "start" -> m.state == "available" && model.migration_projects != []
          "confirm" -> m.state == "confirm_paths"
          "recheck" -> m.state == "in_progress"
          "abandon" ->
            list.contains(
              ["in_progress", "confirm_paths", "failed", "copying"],
              m.state,
            )
          "adopt" -> m.state == "in_progress" && m.blockers == []
          "undo" -> m.state == "adopted" && m.undo
          _ -> False
        }
      let question = case action {
        "confirm" ->
          "Continue? Solace will now build and test your code, and run your check commands, in the copy and in a temporary folder."
        "adopt" -> {
          let unchecked = case m.report {
            None -> []
            Some(report) ->
              list.flat_map(report.checks, fn(c) {
                list.filter(c.items, fn(i) { i.status == "not checked" })
                |> list.map(fn(i) { i.text })
              })
          }
          "Switch to the copy?\n\nYour original folder moves to "
          <> m.original
          <> ".pre-solace, and the copy takes its place at "
          <> m.original
          <> ". Solace then builds and tests it there; if anything fails, both folders go back. You can switch back later."
          <> "\n\nIf your project has build errors or failing tests, the copy has the same ones."
          <> case unchecked {
            [] -> ""
            _ -> "\n\nWorth a look first:\n• " <> string.join(unchecked, "\n• ")
          }
        }
        "abandon" ->
          "Delete the copy at "
          <> m.copy
          <> "? Anything done in it is lost. Your original folder at "
          <> m.original
          <> " stays as it is."
        "undo" ->
          "Switch back to your original folder? It returns to "
          <> m.original
          <> ", and the Solace version moves to ../solace-migration with any work done in it since. You can switch again or delete it later."
        _ -> ""
      }
      case allowed && { question == "" || confirm(question) } {
        False -> #(model, effect.none())
        True -> {
          let fields = case action {
            "start" -> [
              #("projects", json.array(model.migration_projects, json.string)),
              #(
                "smoke",
                string.split(model.migration_smoke, "\n")
                  |> list.map(string.trim)
                  |> list.filter(fn(s) { s != "" })
                  |> json.array(json.string),
              ),
            ]
            _ -> []
          }
          let generation = model.migration_generation + 1
          #(
            Model(
              ..model,
              migration_busy: True,
              migration_generation: generation,
              migration_error: "",
            ),
            effect.from(fn(send) {
              cancel_poll(model.migration_poll_id)
              request(
                "/migration/" <> action,
                "POST",
                json.to_string(json.object(fields)),
                fn(text) { send(MigrationChanged(generation, text)) },
              )
            }),
          )
        }
      }
    }
  }
}

fn migration_view(model: Model) -> Element(Msg) {
  case model.migration {
    None -> h.text("")
    Some(m) -> {
      let busy = model.migration_busy
      let delete =
        migration_button(
          "Delete the copy",
          "Deletes the copy and anything done in it. Your original folder stays as it is.",
          "abandon",
          busy,
          False,
        )
      let folders =
        h.div([], [
          h.p([], [
            h.text("Your original folder: "),
            h.code([], [h.text(m.original)]),
          ]),
          h.p([], [h.text("The copy: "), h.code([], [h.text(m.copy)])]),
        ])
      h.section([a.class("migration")], [
        case model.migration_error {
          "" -> h.text("")
          message -> h.p([a.role("alert")], [h.text(message)])
        },
        case m.state {
          "available" ->
            h.div([], [
              button("Migrate this project", ToggleMigration),
              case model.migration_open {
                False -> h.text("")
                True ->
                  h.section(
                    [
                      a.class("editor"),
                      a.role("dialog"),
                      a.attribute("aria-labelledby", "migration-title"),
                    ],
                    [
                      h.h3([a.id("migration-title")], [
                        h.text("Migrate this project"),
                      ]),
                      h.p([], [
                        h.text(
                          "Solace makes a copy of this folder next to it, called solace-migration, and sets up the copy to use Solace. Your folder isn't changed. You can try Solace in the copy, then switch to it, or delete it.",
                        ),
                      ]),
                      h.p([], [
                        h.text(
                          "If your project has build errors or failing tests now, the copy will have the same ones.",
                        ),
                      ]),
                      h.fieldset([a.disabled(busy)], [
                        h.legend([], [h.text("Projects to set up for Solace")]),
                        h.div(
                          [],
                          list.map(m.candidates, fn(c) {
                            h.label([], [
                              h.input([
                                a.type_("checkbox"),
                                a.checked(list.contains(
                                  model.migration_projects,
                                  c.0,
                                )),
                                e.on_check(MigrationProject(c.0, _)),
                              ]),
                              h.text(case c.0 {
                                "." -> c.1 <> " (this folder)"
                                dir -> c.1 <> " (" <> dir <> ")"
                              }),
                            ])
                          }),
                        ),
                      ]),
                      h.label([], [
                        h.text("Commands that check your app works (optional)"),
                        h.textarea(
                          [
                            a.value(model.migration_smoke),
                            a.disabled(busy),
                            a.attribute(
                              "aria-describedby",
                              "migration-smoke-hint",
                            ),
                            e.on_input(MigrationSmoke),
                          ],
                          model.migration_smoke,
                        ),
                      ]),
                      h.small([a.id("migration-smoke-hint")], [
                        h.text(
                          "One command per line, run from this folder, such as a script that starts your app and requests a page. Solace runs each one before and after migrating, and compares the results.",
                        ),
                      ]),
                      h.button(
                        [
                          a.class("primary"),
                          a.disabled(model.migration_projects == [] || busy),
                          e.on_click(MigrationAction("start")),
                        ],
                        [h.text("Make the copy")],
                      ),
                      button("Close", ToggleMigration),
                    ],
                  )
              },
            ])
          "running" ->
            h.section(
              [
                a.class("editor"),
                a.role("status"),
                a.attribute("aria-live", "polite"),
              ],
              [
                h.h3([], [h.text("Migrating")]),
                h.p([], [h.text(m.step)]),
                h.small([], [h.text("This page reloads when it's done.")]),
              ],
            )
          "confirm_paths" ->
            h.div([a.class("editor")], [
              h.h3([], [h.text("Check these files before going on")]),
              h.p([], [
                h.text(
                  "Each one could change your original folder if it runs from the copy, for example a script that uses the folder's full path. Nothing has run yet. Look at each one, then continue.",
                ),
              ]),
              h.ul([], list.map(m.flagged, migration_item)),
              folders,
              migration_button(
                "Continue",
                "Builds and tests your code, and runs your check commands.",
                "confirm",
                busy,
                True,
              ),
              delete,
            ])
          "failed" ->
            h.div([a.class("editor")], [
              h.h3([], [h.text("The migration stopped")]),
              h.p([a.role("alert")], [h.text(m.problem)]),
              folders,
              case m.report {
                Some(_) -> migration_report(m.report)
                None -> h.text("")
              },
              delete,
            ])
          "copying" ->
            h.div([a.class("editor")], [
              h.h3([], [h.text("Copying your folder")]),
              h.p([], [
                h.text(
                  "A copy of your folder is being made, or an earlier attempt was interrupted.",
                ),
              ]),
              folders,
              delete,
            ])
          "in_progress" ->
            h.div([], [
              h.div([a.class("editor")], [
                h.h3([], [
                  h.text("You're trying Solace on a copy of your project"),
                ]),
                h.p([], [
                  h.text(
                    "Solace is using the copy. Your original folder hasn't been changed.",
                  ),
                ]),
                folders,
                case m.blockers {
                  [] -> h.text("")
                  _ ->
                    h.div([a.id("migration-blockers")], [
                      h.p([], [h.text("Before you can switch to the copy:")]),
                      h.ul(
                        [],
                        list.map(m.blockers, fn(b) { h.li([], [h.text(b)]) }),
                      ),
                    ])
                },
                migration_button(
                  "Switch to the copy",
                  "Moves your original folder to "
                    <> m.original
                    <> ".pre-solace and puts the copy in its place, then builds and tests it there. If that fails, both folders go back.",
                  "adopt",
                  busy || m.blockers != [],
                  True,
                ),
                migration_button(
                  "Run the checks again",
                  "Builds and tests the copy again and compares it with your original folder. Do this after changing the copy.",
                  "recheck",
                  busy,
                  False,
                ),
                delete,
              ]),
              migration_report(m.report),
            ])
          "adopted" if m.undo && !model.migration_dismissed ->
            h.div([a.class("notice editor")], [
              h.strong([], [h.text("Your project now uses Solace")]),
              h.p([], [
                h.text("Your old folder is kept at "),
                h.code([], [h.text(m.original <> ".pre-solace")]),
                h.text(
                  ", in case you want to switch back. Delete it yourself once you're sure.",
                ),
              ]),
              migration_button(
                "Switch back",
                "Puts your old folder back, and moves this one to ../solace-migration.",
                "undo",
                busy,
                False,
              ),
              button("Dismiss", DismissMigration),
            ])
          _ -> h.text("")
        },
        case busy {
          True -> h.p([a.role("status")], [h.text("Starting…")])
          False -> h.text("")
        },
      ])
    }
  }
}

fn migration_report(report: Option(MigrationReport)) -> Element(Msg) {
  h.section([a.class("editor migration-report")], [
    h.h2([], [h.text("What Solace checked")]),
    case report {
      None ->
        h.p([], [
          h.text("The checks haven't run yet. Run the checks to see them."),
        ])
      Some(report) ->
        h.div([], [
          h.div(
            [],
            list.map(report.warnings, fn(warning) {
              h.p([a.role("alert")], [h.strong([], [h.text(warning)])])
            }),
          ),
          case report.current {
            True -> h.text("")
            False ->
              h.p([a.role("alert")], [
                h.text(
                  "The copy changed after these checks ran. Run the checks again before switching.",
                ),
              ])
          },
          h.p(
            [a.class("migration-legend")],
            list.flat_map(
              [
                #("verified", "✓", "fine"),
                #("failed", "✗", "a problem to fix before switching"),
                #("not checked", "!", "worth a look"),
              ],
              fn(l) {
                [
                  h.span(
                    [
                      a.class(
                        "mark migration-" <> string.replace(l.0, " ", "-"),
                      ),
                      a.attribute("aria-hidden", "true"),
                    ],
                    [h.text(l.1)],
                  ),
                  h.text(l.2 <> " "),
                ]
              },
            ),
          ),
          h.div(
            [],
            list.map(report.checks, fn(c) {
              h.article([], [
                h.h3([], [migration_mark(c.status), h.text(c.name)]),
                h.p([], [h.text(c.summary)]),
                h.ul([], list.map(c.items, migration_item)),
              ])
            }),
          ),
          h.small([], [
            h.text(
              "Last checked "
              <> {
                int.parse(report.checked_at)
                |> result.map(clock)
                |> result.unwrap(report.checked_at)
              },
            ),
          ]),
        ])
    },
  ])
}

fn migration_item(item: MigrationItem) -> Element(Msg) {
  h.li([], [
    migration_mark(item.status),
    h.text(item.text),
    case item.detail {
      "" -> h.text("")
      detail ->
        h.details([a.class("entry")], [
          h.summary([], [h.text("Show details")]),
          h.pre([a.class("diff")], [h.text(detail)]),
        ])
    },
  ])
}

/// A result's mark: fine, a problem, or something worth a look. The words
/// are for screen readers; the legend above the checks explains the marks.
fn migration_mark(status: String) -> Element(Msg) {
  let #(mark, words) = case status {
    "verified" -> #("✓", "fine")
    "failed" -> #("✗", "a problem")
    "not checked" -> #("!", "worth a look")
    _ -> #("", "")
  }
  case mark {
    "" -> h.text("")
    _ ->
      h.span(
        [
          a.class("mark migration-" <> string.replace(status, " ", "-")),
          a.attribute("aria-label", words),
          a.attribute("title", words),
        ],
        [h.text(mark)],
      )
  }
}

/// A button with a sentence beside it saying what it does to the user's folders.
fn migration_button(
  label: String,
  what: String,
  action: String,
  disabled: Bool,
  primary: Bool,
) -> Element(Msg) {
  h.div([a.class("migration-action")], [
    h.button(
      [
        a.classes([#("primary", primary)]),
        a.disabled(disabled),
        e.on_click(MigrationAction(action)),
      ],
      [h.text(label)],
    ),
    h.span([], [h.text(what)]),
  ])
}

fn button(label: String, msg: Msg) -> Element(Msg) {
  h.button([e.on_click(msg)], [h.text(label)])
}

fn get(path: String, msg: fn(String) -> Msg) -> Effect(Msg) {
  effect.from(fn(send) {
    request(path, "GET", "", fn(text) { send(msg(text)) })
  })
}

@external(javascript, "./api.mjs", "request")
fn request(
  path: String,
  method: String,
  body: String,
  receive: fn(String) -> Nil,
) -> Nil

@external(javascript, "./api.mjs", "after")
fn after(poll_id: Int, ms: Int, callback: fn() -> Nil) -> Nil

@external(javascript, "./api.mjs", "confirm")
fn confirm(question: String) -> Bool

@external(javascript, "./api.mjs", "reload")
fn reload() -> Nil

fn error(text: String) -> String {
  json.parse(text, decode.at(["message"], decode.string))
  |> result.unwrap("Invalid API response")
}

@external(javascript, "./api.mjs", "new_poll_id")
fn new_poll_id() -> Int

@external(javascript, "./api.mjs", "cancel_poll")
fn cancel_poll(poll_id: Int) -> Nil

/// The root's projects section: the listed projects, the Gleam projects
/// found in its folder, the new-project form and a removal in progress.
pub type Projects {
  Projects(
    /// Whether the list has loaded once; the section shows from then on.
    listed: Bool,
    projects: List(Project),
    found: List(Found),
    error: String,
    name: String,
    dir: String,
    target: String,
    /// The project being removed: its dir, whether the second step (deleting
    /// its files) shows, and the name typed to confirm it.
    removing: Option(#(String, Bool, String)),
  )
}

/// A project on the root's board: its folder, label and package name.
pub type Project {
  Project(dir: String, label: String, name: String)
}

/// A Gleam project found in the root's folder.
pub type Found {
  Found(dir: String, name: String, imported: Bool, member: Bool)
}

pub const no_projects = Projects(False, [], [], "", "", "", "erlang", None)

/// The Projects section's messages. Every change to the list answers with
/// the new list, which the board's lanes follow through its event stream.
fn update_projects(p: Projects, msg: Msg) -> #(Projects, Effect(Msg)) {
  case msg {
    // A reload, such as after a reconnect, keeps what the human typed; a
    // change's reply clears the form.
    ProjectsLoaded(text, keep) ->
      case json.parse(text, projects_decoder()) {
        Ok(#(projects, found)) -> {
          let p =
            Projects(
              ..p,
              listed: True,
              projects: projects,
              found: found,
              error: "",
            )
          #(
            case keep {
              // A reload keeps what the human typed; a removal whose
              // project has left the list has nothing left to confirm.
              True ->
                case p.removing {
                  None -> p
                  Some(#(dir, _, _)) ->
                    case list.any(projects, fn(x) { x.dir == dir }) {
                      True -> p
                      False -> Projects(..p, removing: None)
                    }
                }
              False -> Projects(..p, name: "", dir: "", removing: None)
            },
            effect.none(),
          )
        }
        Error(_) -> #(Projects(..p, error: error(text)), effect.none())
      }
    AddProject(dir) -> #(
      p,
      case
        list.any(p.found, fn(f) { f.dir == dir && f.imported && !f.member })
      {
        True ->
          projects_request("/projects", "POST", [#("dir", json.string(dir))])
        False -> effect.none()
      },
    )
    // Removing asks first whether to keep the files; deleting them takes a
    // second warning and the project's name (D-74).
    RemoveProject(dir) -> #(
      Projects(..p, removing: Some(#(dir, False, ""))),
      effect.none(),
    )
    RemoveOnly ->
      case p.removing {
        Some(#(dir, _, _)) -> #(
          p,
          projects_request(
            "/projects/" <> uri.percent_encode(dir),
            "DELETE",
            [],
          ),
        )
        None -> #(p, effect.none())
      }
    RemoveFiles ->
      case p.removing {
        Some(#(dir, _, _)) -> #(
          Projects(..p, removing: Some(#(dir, True, ""))),
          effect.none(),
        )
        None -> #(p, effect.none())
      }
    RemoveTyped(text) ->
      case p.removing {
        Some(#(dir, True, _)) -> #(
          Projects(..p, removing: Some(#(dir, True, text))),
          effect.none(),
        )
        _ -> #(p, effect.none())
      }
    RemoveDelete ->
      case p.removing {
        Some(#(dir, True, typed)) if typed == dir -> #(
          p,
          projects_request("/projects/" <> uri.percent_encode(dir), "DELETE", [
            #("delete", json.bool(True)),
            #("name", json.string(typed)),
          ]),
        )
        _ -> #(p, effect.none())
      }
    RemoveCancel -> #(Projects(..p, removing: None), effect.none())
    ProjectName(text) -> #(Projects(..p, name: text), effect.none())
    ProjectDir(text) -> #(Projects(..p, dir: text), effect.none())
    ProjectTarget(text) -> #(Projects(..p, target: text), effect.none())
    CreateProject -> #(
      p,
      projects_request("/projects/new", "POST", [
        #("name", json.string(p.name)),
        #("dir", json.string(p.dir)),
        #("target", json.string(p.target)),
      ]),
    )
    _ -> #(p, effect.none())
  }
}

/// A `/projects` change; the reply is the new list, or an error.
fn projects_request(
  path: String,
  method: String,
  fields: List(#(String, json.Json)),
) -> Effect(Msg) {
  effect.from(fn(send) {
    request(path, method, json.to_string(json.object(fields)), fn(text) {
      send(ProjectsLoaded(text, False))
    })
  })
}

fn projects_decoder() -> decode.Decoder(#(List(Project), List(Found))) {
  let project = {
    use dir <- decode.field("dir", decode.string)
    use label <- decode.optional_field("label", dir, decode.string)
    use name <- decode.field("name", decode.string)
    decode.success(Project(dir, label, name))
  }
  let found = {
    use dir <- decode.field("dir", decode.string)
    use name <- decode.field("name", decode.string)
    use imported <- decode.field("imported", decode.bool)
    use member <- decode.field("member", decode.bool)
    decode.success(Found(dir, name, imported, member))
  }
  use projects <- decode.field("projects", decode.list(project))
  use found <- decode.optional_field("found", [], decode.list(found))
  decode.success(#(projects, found))
}

/// The root's projects: remove one, add one found in the folder, or create
/// a new Gleam project. Existing code without Solace is brought in by the
/// section below.
fn projects_view(p: Projects) -> Element(Msg) {
  case p.listed {
    False -> h.text("")
    True -> {
      let others = list.filter(p.found, fn(f) { !f.member })
      h.section([a.class("projects")], [
        h.h2([], [h.text(solace_task.ui_label("projects"))]),
        case p.error {
          "" -> h.text("")
          message -> h.p([a.role("alert")], [h.text(message)])
        },
        h.ul(
          [],
          list.map(p.projects, fn(l) {
            h.li([], [
              h.strong([], [h.text(l.label)]),
              h.text(" " <> l.name <> " "),
              // The root's own project always stays on its board.
              case l.dir {
                "." -> h.small([], [h.text("(this folder)")])
                dir ->
                  button(solace_task.ui_label("remove"), RemoveProject(dir))
              },
            ])
          }),
        ),
        removal_view(p),
        case others {
          [] -> h.text("")
          _ ->
            h.div([], [
              h.h3([], [h.text("Found in this folder")]),
              h.ul(
                [],
                list.map(others, fn(f) {
                  h.li([], [
                    h.strong([], [
                      h.text(case f.dir {
                        "." -> "This folder"
                        dir -> dir
                      }),
                    ]),
                    h.text(
                      " "
                      <> f.name
                      <> case f.imported {
                        True -> " (already set up) "
                        False -> " (not set up) "
                      },
                    ),
                    case f.imported {
                      True ->
                        button(solace_task.ui_label("add"), AddProject(f.dir))
                      False ->
                        h.small([], [
                          h.text(
                            "Existing code is brought in under Bring in an existing project, below.",
                          ),
                        ])
                    },
                  ])
                }),
              ),
            ])
        },
        h.h3([], [h.text("New project")]),
        h.div([a.class("new-project")], [
          h.input([
            a.placeholder("Package name, e.g. my_app"),
            a.attribute("aria-label", "Package name"),
            a.value(p.name),
            e.on_input(ProjectName),
          ]),
          h.input([
            a.placeholder(case p.name {
              "" -> "Folder (default: the package name)"
              name -> "Folder (default: " <> name <> ")"
            }),
            a.attribute("aria-label", "Folder"),
            a.value(p.dir),
            e.on_input(ProjectDir),
          ]),
          h.label([], [
            h.text("Runs on"),
            h.select([a.value(p.target), e.on_input(ProjectTarget)], [
              h.option([a.value("erlang")], "Erlang (BEAM)"),
              h.option([a.value("javascript")], "JavaScript"),
            ]),
          ]),
          button(solace_task.ui_label("create_project"), CreateProject),
        ]),
      ])
    }
  }
}

/// The dialog for removing a project: keep its files, or delete them after a
/// second warning and typing its name.
fn removal_view(p: Projects) -> Element(Msg) {
  case p.removing {
    None -> h.text("")
    Some(#(dir, False, _)) ->
      solace_task.modal([
        h.p([], [
          h.text(
            "Remove "
            <> dir
            <> " from this board? Its tasks and Solace setup stay in its folder, and it can be added again. Should its folder and files be deleted too?",
          ),
        ]),
        button(solace_task.ui_label("remove_only"), RemoveOnly),
        button("Remove and delete files…", RemoveFiles),
        button(solace_task.ui_label("cancel"), RemoveCancel),
      ])
    Some(#(dir, True, typed)) ->
      solace_task.modal([
        h.p([], [
          h.strong([], [
            h.text(
              "This permanently deletes "
              <> dir
              <> " and everything in it. It cannot be undone.",
            ),
          ]),
        ]),
        h.label([], [
          h.text("Type " <> dir <> " to confirm"),
          h.input([
            a.attribute("aria-label", "Project name"),
            a.attribute("autofocus", ""),
            a.value(typed),
            e.on_input(RemoveTyped),
          ]),
        ]),
        h.button([a.disabled(typed != dir), e.on_click(RemoveDelete)], [
          h.text(solace_task.ui_label("delete_permanently")),
        ]),
        button(solace_task.ui_label("cancel"), RemoveCancel),
      ])
  }
}

const projects_styles =
  ":host{--shadow:#17222626}.projects{margin-bottom:24px}.projects h3{font-size:15px;margin:16px 0 4px}.projects ul{list-style:none;padding:0;margin:0}.projects li{display:flex;align-items:center;gap:6px;white-space:normal;margin:0}.projects li button{padding:2px 10px}.new-project{display:flex;flex-wrap:wrap;align-items:flex-end;gap:4px}.new-project input{flex:1;min-width:220px}.new-project label{display:flex;flex-direction:column}.new-project select{font:inherit;padding:8px;margin:4px;border:1px solid var(--edge);border-radius:4px}.modal p{margin:0 0 12px;overflow-wrap:anywhere}.modal input{margin:0}.modal button:disabled{opacity:.5;cursor:not-allowed}"
