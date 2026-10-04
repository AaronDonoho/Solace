# Spec 17: Migrating an Existing Project

**Status:** agreed design 2026-10-04, built 2026-10-04, review fixes 2026-10-04 (`handoff.md`) · **Implements:** D-59 · **Needed by:** converting an existing project, such as this repository, to Solace.

Converting a project must not touch the user's files. Solace builds and checks its own complete copy of the project in a separate folder. The original stays where it is, unchanged and usable, until the user adopts the copy. Adopting it swaps the two folders, so the copy takes the original's path and the original is kept, unchanged, beside it. Backing out at any point before adoption leaves the user exactly where they started.

Terms: the **original** is the user's project folder, as it was. The **copy** is `solace-migration/`, the folder Solace builds beside it. **Adopting** is the final swap.

## 1. What the user decided, and what is proposed

Aaron's requirements (2026-10-04):
- The migration doesn't touch the original's files.
- Agents and humans never confuse the original's files with Solace's: the separation lasts until the migration is complete.
- Solace builds the whole project in a separate folder, `solace-migration/`, beside the original. There is one migration at a time.
- The copy holds the entire folder, with every subfolder, so one project or several can be migrated at once.
- The project keeps its path, since changing it may break builds, so the copy takes the original's path at the end. The swap happens at the end, not the start.
- Regenerated files don't have to match the originals byte for byte. Testing and review decide, through the checks in §4.
- The human decides when to accept a migration. The files move only after the build, test and check suite has succeeded and the human has approved.
- `solace create` only creates brand-new projects and subprojects (§8).

The rest of this spec is Claude's proposal, drawing on Astra's review.

## 2. Starting a migration

The user runs `solace serve` in the project folder, as today. A folder with Gleam projects but no Solace store is offered **Migrate this project** on the board (`solace migrate` does the same from a shell). The user picks which Gleam projects to migrate: one, several or all. Then Solace:

1. Refuses if `../solace-migration/` already exists ("a migration is already in progress: adopt or abandon it first"), or isn't on the same filesystem as the original (the swap needs an atomic rename).
2. Records an **inventory** of the original: every entry's path and permissions, each file's size, modified time and SHA-256, and each symlink's target, including `.git`, untracked and ignored files. Build output (`build/` beside a `gleam.toml`) is left out of the inventory, since building the original rewrites it; it is still copied.
3. Creates `../solace-migration/` and records the migration in it before copying anything, so a copy that fails partway can still be abandoned. Copies the whole folder there, with permissions. Symlinks stay symlinks, except that one pointing into the original is pointed at the same place in the copy, by a relative path. Sockets, FIFOs and devices aren't copied, and Coverage lists them.
4. Checks the copy for what could reach the original (§3) before running anything. A link out of the copy from a selected project's `src/`, `test/`, `build/` or `.solace/` stops the migration, since Solace's own writes would follow it. Anything else found stops it for the user to look at, on the board or as a list in the shell, until they continue (`--confirm-paths`).
5. Runs a **baseline** in a second, throwaway copy in the system's temporary folder, deleted afterwards: `gleam build` and `gleam test` in each selected project, plus the smoke checks (below). It records which tests pass and fail. The baseline runs there so the migration copy holds no build output from the old source.
6. Sets the copy up as a Solace root with the selected projects, as `create` does today. Folders keep their places.
7. **Regenerates** each selected project's Gleam source from the store into the copy, after deleting the copied source (only files whose real path is inside the copy; symlinked folders aren't followed), so leftover files can't mask anything the store is missing. It then builds, tests and runs the smoke checks again.
8. Shows the **migration report** (§4).

If any step fails, the migration stops and keeps the copy, with the reason, so the user can see why; Abandon removes it. A Gleam file that isn't UTF-8, or defines a name twice, is named under a failed Coverage. Solace says the original is unchanged only after comparing it with its inventory.

**Smoke checks** are commands the user names when starting the migration, such as starting the app and requesting a page. Each must exit with 0. With none named, the report marks app behavior **not checked**.

From then on the board serves the copy, and its header shows the copy's full path and "Migration in progress". Agents work only in the copy: its store, its files, and commands run in its folders. The original isn't under any Solace root, so no command can reach it.

## 3. Paths that would escape the copy

Before the report says the copy is ready, Solace checks:
- **Path dependencies** in every `gleam.toml` in the copy (`path = "../shared"`), resolved through symlinks. One that resolves outside the copied folder blocks readiness. The user has to bring that folder into the migrated tree.
- **Symlinks** that lead out of the copy. One into the original that wasn't redirected fails; one elsewhere is listed as needing attention.
- **Git pointers:** a `.git` file, `commondir` or `alternates` entry pointing outside the copy. Into the original, it fails.
- **The original's absolute path** appearing in any text file in the copy, such as a script or config file, or in a smoke check. Each one is listed as needing attention, since it would reach the original from inside the copy. Files over 256 KiB aren't searched, and are listed as not checked.

**Isolation:** where bubblewrap (`bwrap`) is installed, every command a migration runs (baseline, builds, tests, smoke checks, Recovery, the build at the real path) sees the original as read-only, and so do agents' commands in the copy while the migration is in progress. After adoption, `<name>.pre-solace` is the read-only folder. Without bubblewrap, Paths says the commands weren't isolated, as not checked.

## 4. The migration report

The report is on the board, under "What Solace checked". Each check and line carries a mark, explained in a legend above them: ✓ fine (verified), ✗ a problem to fix before switching (failed), ! worth a look (not checked). Each check has a plain title and a sentence saying what it found, which depends on the result. Any project that doesn't build before or after the migration is named first.

A migration reproduces the original exactly, errors included (Aaron, 2026-10-04): a build or test that fails before migrating fails after it too. Such failures are marked not checked, as failing before the migration, and don't block adoption. The guide, the start dialog and the Adopt confirmation say so.
- **Coverage** ("Copying your folder"): the projects and Gleam files imported, the other files copied, what was left out (projects not selected, Gleam files outside any project), and anything Solace couldn't read in or resolve, such as a name defined twice, a file that isn't UTF-8, or a path dependency outside the folder.
- **Reconstruction** ("Your code, written back by Solace"): the complete diff between each original Gleam file and its regenerated version, comments and layout included. The original is split into its header and definitions again and compared with the store's: if each is identical, the files can only differ between definitions, which is layout, and the report names the kinds (blank lines, line ends, final newline). A definition that differs is **failed**, and so is a multi-line string holding a line that looks like a definition, where a split would change the string. Byte-identical files are listed as such. Reconstruction is judged once, right after the first regeneration; later work in the copy is reviewed through tasks, as it is after adoption.
- **Behavior** ("Builds and tests"): the baseline against the regenerated project, project by project: build, each test, and each smoke check. A test that already failed in the baseline is reported as failing before the migration, not as broken by it. When only the regenerated project builds, its tests are still reported. A successful build alone doesn't count as the same behavior.
- **Paths** ("Anything that could reach your original folder"): the results of §3.
- **Preservation** ("Your original folder"): the original compared with its inventory: paths, permissions, contents, sizes, modified times and symlink targets, untracked and ignored files included, build output excluded (the summary says so). Git status alone isn't used. A git index is compared by what it records (each staged path, mode and content), since any git command may rewrite it to refresh cached file times; that doesn't count as a change, but staging does (Aaron, 2026-10-04). Git's short-lived `index.lock` is left out.
- **Recovery** ("Your code without Solace"): the selected projects are exported from the copy without Solace's files (`.solace/` and `solace.toml`) into a throwaway folder, then built and tested there with plain `gleam`. This shows the code still builds without Solace. Preservation shows that abandoning the copy needs no repair to the original.

Any change in the copy after the checks, whether an agent's task or a human edit, puts the checks out of date until they run again, which the board says above them (the copy's build output, `.git` and `.solace/` aside). The copy's fingerprint covers each entry's path, kind, permissions, link target and content; it is taken after the build and tests and before the other checks, so a change made while they run leaves the report out of date. A status read compares sizes and change times first, and the content only when those differ. **Recheck** on the board runs them again.

**Wording** (Aaron, 2026-10-04): every screen and message is written for a user who knows nothing of how Solace works inside, in American English. Each button sits beside a sentence saying what it does to the user's folders. The board's names: **Make the copy** (start), **Continue** (after looking at flagged files), **Run the checks again** (recheck), **Switch to the copy** (adopt), **Delete the copy** (abandon), **Switch back** (undo adoption). Smoke checks are "commands that check your app works".

## 5. Working in the copy before adopting

The user can try Solace for real in the copy: run tasks, review them, and run the app from the copy's folders. That work stays in the copy and comes along when it's adopted. While the migration is in progress, the original remains the user's working project, so editing it is allowed. It just means adoption will refuse (§6).

## 6. Adopting

The human decides when to adopt. **Adopt** on the board (`solace migrate --adopt`) is offered only when no check is failed or out of date, and lists each blocker until then. Lines that are **not checked**, such as smoke checks when none were named, are shown in the confirmation the human approves. Then Solace:

1. Refuses while an agent is working in the copy, or another migration step is running. Every migration step, from the board or a shell, holds `../.solace-migration.lock` and each of the copy's project locks. While a step runs, the board refuses other changes (`E7003`). A shell refuses to act on a migration that a board is serving. The copy's fingerprint is compared by content.
2. Compares the original with its inventory. If anything changed since the copy was taken, it refuses and lists the changes, because swapping would lose that work. The user can bring the changes into the copy, through Unexpected Changes (D-58) or a task, and record a new inventory, or start over.
3. Renames the original to `<name>.pre-solace` (refused if that exists), then renames `solace-migration/` to the original's name. A journal beside the folders, `../.solace-migration-swap.json`, records each rename as it's done. If the swap is interrupted, the next `solace migrate` or `solace serve` there puts both folders back before doing anything else, judging what was done from the folders themselves. If putting them back fails, Solace names the folders and keeps the journal, and never claims a rollback that didn't happen.
4. Builds, tests and runs the smoke checks again at the real path.
5. If anything fails, renames both folders back, so the original is in place exactly as it was, and reports what failed.
6. On success, the server restarts itself at the original path and reports the migration complete. The original stays as `<name>.pre-solace` until the user deletes it. From a shell, Solace prints the `cd` command for the adopted path, since a shell that was inside either folder moved with it.

## 7. Backing out

- **Before adopting:** **Abandon migration** deletes `solace-migration/` after a confirmation. The original was never touched.
- **After adopting:** **Undo adoption** swaps the folders back while `<name>.pre-solace` exists and still matches the inventory, after a confirmation, with the same journal. Work done after adopting stays in the migrated project, which goes back to `../solace-migration/`, still in progress, so it can be adopted again or abandoned.

## 8. `solace create`

`solace create` only creates brand-new projects and subprojects. It requires a project name whose folder doesn't exist, shows what it will create, and proceeds only after the user confirms. It runs `gleam new` and sets the project up, as the board's **New project** does. In a folder that already holds a Gleam project it refuses, and points to **Migrate this project**. It no longer reads existing code into a store.

The benchmark harness and the acceptance tests set up their throwaway fixtures with `solace migrate` and `--adopt` (the harness with `--confirm-paths`, since nobody is there to look), which adds a baseline build to setup but costs agents nothing.

## Tests (acceptance)

- M1. Migrating a fixture repository (two Gleam projects, a non-Gleam folder, an ignored data file, a symlink, a script holding the original's absolute path) leaves every file of the original byte-identical to its inventory, including modified times and `.git`. The copy holds every file. The script is listed under Paths.
- M2. A `gleam.toml` path dependency that leaves the folder blocks Adopt.
- M3. A test that fails in the original is reported as failing in the baseline, not as broken by the migration. A regenerated file that differs from the original appears in Reconstruction with its diff and the cause of each difference. A failing smoke check is **failed**; none named is **not checked**.
- M4. Adopting swaps the folders; the project then builds and tests at the original path; `<name>.pre-solace` matches the inventory.
- M5. An edit to the original after the copy was taken makes Adopt refuse, naming the file. A build that fails after the swap swaps both folders back. A change in the copy after the checks hides Adopt until Recheck.
- M6. Abandon removes the copy and leaves the original unchanged. Undo adoption restores both folders.
- M7. Recovery builds and tests the exported projects with no `.solace/` or `solace.toml` present.
- M8. A second `solace migrate` while `../solace-migration/` exists is refused. `solace create` refuses an existing folder name and a folder holding a Gleam project, and creates nothing without confirmation.
- M9. Absolute links into the original from `src/` and `build/` are pointed into the copy, and the original is unchanged. A link out of the copy from `src/` stops the migration and keeps the copy, which Abandon removes.
- M10. A smoke check that names the original stops the migration before anything runs; with bubblewrap, continuing leaves the original unchanged.
- M11. An adopt or undo interrupted after each rename is put back by the next command.
- M12. A held project lock or migration lock refuses a step; a shell adopt is refused while a board serves the copy.
- M13. A project that doesn't build before or after the migration is reported first, and can be adopted. A change made during the checks leaves the report out of date. A dependency through a symlink outside, and a `.git` file pointing into the original, appear under Paths. Changing the copy's code doesn't block adoption. Folder permissions are kept, and a FIFO is listed as not copied. An unreadable file stops the migration before the copy; a Gleam file that isn't UTF-8 is named under Coverage.
