# Solace on Windows

How Solace reaches Windows machines with little setup (D-224). Last updated 2026-11-03.

**Where things stand:** the first installer is built and works on one Windows 11 x64 laptop. It's for us to try, not for clients. None of this work is committed yet.

## Direction

- **Windows runs Solace through WSL2.** The `solace` CLI stays a Linux program; it isn't ported to Windows.
- **Solace ships its own WSL distro.** It is Debian 13 (trixie) slim with `solace`, Git, bubblewrap, curl, `ca-certificates` and `procps`. It sits alongside any Linux the client already has and is the same on every machine.
- **The base uses glibc, not musl.** Alpine (musl) is smaller, but many prebuilt programs only run on glibc: some agents, Node packages with native code, and standard toolchain installers. Clients may use any agent, so glibc avoids problems we can't control.
- **No agent and no language toolchain is shipped.** Clients aren't tied to one agent. A later feature installs agents and toolchains from inside Solace.
- **The desktop app is built for Windows** and starts the server inside the distro.

## Done

- [x] **Decision recorded:** D-224 in `DECISIONS.md`, and the README points here.
- [x] **Distro image:** a 206 MB tar, with a regular `solace` user and no `sudo`.
- [x] **Launcher inside the distro:** `solace-desktop` finds the login PATH, reports a running server, and ties the server's life to the app's.
- [x] **Windows app:** starts the server through `wsl.exe`, stops it by closing its input, converts folder paths, and refuses folders outside the distro.
- [x] **First-launch setup:** imports the distro with no input needed. The "Turn on WSL" step is written but untested.
- [x] **Open Solace terminal** button on the start screen.
- [x] **Port handling:** the app avoids a port Windows is using, and waits until Windows can reach the server.
- [x] **Installer:** `Solace_0.1.0_x64-setup.exe`, 47 MB and unsigned, built by a script in the repository.
- [x] **Tests:** 67 app tests pass on Linux, including the `wsl` and launcher tests. See [Test results](#test-results) for what ran on Windows.
- [x] **Two bugs found by running on Windows, both fixed:**
  - The image had no `kill` program, and Solace ignores that failure, so timed-out commands would have kept running. Adding `procps` fixed it.
  - `open_project` hung, because creating a window from a synchronous Tauri command deadlocks WebView2 on Windows. It's now asynchronous on Windows only, because on Linux the server's parent-death signal depends on the thread that starts it.
- [x] **Security review:** see [Security](#security-what-someone-installing-solace-should-know).

## Left to do

### 1. Finish the first build

- [ ] **Commit this work.** First bring `solace-app`'s store up to date: `main.rs`, `ui/index.html` and the new files were changed directly, not through `solace submit`.
- [ ] **Try a real task.** Install an agent and Gleam or Rust from the terminal, sign in, create a project and run a task. Agent install and sign-in are untested, and an agent can't open a browser from inside the distro. Claude Code offers a URL and a pasted code; Codex offers `codex login --device-auth` (beta).
- [ ] **Test on a machine without WSL:** the "Turn on WSL" button, its administrator prompt and the restart.
- [ ] **Test sleep, Fast Startup and `wsl --shutdown`** with a board open.
- [ ] **Keep agents running when Solace closes.** With every `wsl.exe` gone, the distro stopped within about 35 seconds and took a background process with it. On Linux, agents outlive the app and the next `serve` picks them back up. The fix is to keep a `wsl.exe` open while a run is active.
- [ ] **Run the app's tests on Windows.** Seven tests start `sleep`, which doesn't exist there, so they need a small helper first.
- [ ] **Fix two misleading messages and docs:**
  - In a folder with no `.solace`, a mistyped command reports a store error instead of "Unknown command".
  - README lines 84 and 249 and Spec 15 say Solace stores no credentials, but pasted API keys go in `keys.json`. The spec changes through a decision.

### 2. Before any client gets it

Each of these is its own iteration, in rough order.

- [ ] **Install from Solace.** Today a new client is stuck, confirmed on the running board: its Debian steps (`installs.json`) use `sudo` and compile Erlang, but the distro has neither `sudo` nor a compiler; Rust and JS runtimes aren't listed; and with nothing installed, **no project can be created at all**, because new projects need `gleam new` or `cargo new` and an empty folder is refused.
  - Agents and toolchains install into the home folder without root, on a fixed PATH.
  - Anything that needs root, such as Rust's C compiler, runs from the app's start screen through `wsl -u root`, or ships in the image, because only the start screen can talk to the app.
  - Erlang/OTP 29 needs a prebuilt copy somewhere.
  - The prerequisites check should ask only for what the open project needs.
  - Newly installed tools should work without restarting the app. Today the server reads the PATH when it starts.
- [ ] **Sandbox Gleam commands.** See Security, item 1. This needs its own decision.
- [ ] **Sandbox the review screen's test run.** See Security, item 2.
- [ ] **Decide whether Gleam dependencies need approval.** See Security, item 4.
- [ ] **Get code in and out.** With `C:\` unmounted, clients can't reach Windows files or Windows' Git credentials. Replace the folder picker with My projects, New, Bring in from a Windows folder, Clone and Export. Export doubles as backup.
- [ ] **Auto-update.** Without the updater in the first client release, every later fix means a manual reinstall.
  - Back up the updater's signing key: losing it means installed copies can never update.
  - Clients need a public update address. The repository is private, so it can't serve them from GitHub Releases.
  - Replace `/usr/local/bin/solace` inside the existing distro, never by re-importing, and only when no agent is running, because the Windows installer closes the app. This needs `solace --version`, which doesn't exist yet.
  - Keep Debian's packages patched. The distro has no systemd, so the app runs this through `wsl -u root`, without blocking startup.
- [ ] **Code signing.** Azure Artifact Signing (about $10 a month; individuals in the US and Canada only) or an OV certificate. Signing doesn't stop SmartScreen at once; reputation builds over weeks. A cross-compiled build needs a custom signing command, because Tauri only signs on Windows by default.
- [ ] **A preflight screen** with clear messages for ARM laptops (the distro is x64 only), virtualization turned off, machines inside a VM, company policy blocking WSL, and VPNs.
- [ ] **A way to remove the distro.** The installer never removes it, because client projects live in it. Add a separate, explicit action in the app that says this deletes the projects inside.
- [ ] **CI and a test matrix.** The repository has no CI, and the build environment exists only on one laptop. Hosted Windows runners may not run WSL2 reliably, so keep a manual pass on clean machines: Home, Pro, company-managed, on a VPN, and with Docker Desktop.
- [ ] **Consider a login for the board.** See Security, item 8. It matters most on machines shared by several people.
- [ ] **A private pilot** with one or two friendly clients before a wider release.

### 3. Legal

- [ ] **A license or terms.** The repository has no `LICENSE` file; to be decided later.
- [ ] **Third-party notices.** The distro redistributes GPL software (Git and much of Debian), which carries source obligations. Get a lawyer's view.

## Keep in mind

### Tell clients plainly
- A program built in Solace on Windows is a **Linux** program. A Rust project produces a Linux binary, not a Windows `.exe`.
- Local model servers on Windows aren't reachable at `localhost` from WSL's default networking, so endpoints need the host's address.

### Hard to change after the first client
- The updater's signing key and update address.
- The app identifier `dev.solace.app`, the distro name `Solace`, the Linux user `solace`, and per-user install.
- The disk location. `%LOCALAPPDATA%\Solace\wsl` survives uninstall. `%LOCALAPPDATA%\dev.solace.app` does not: the uninstaller's delete-data option removes it recursively.
- Uninstall hooks also run around updates, so they must never touch the distro.

## How it's built

`solace-app/distro/build-installer.sh`, run as root on Debian 13, builds everything; its header lists what it needs. It cross-compiles with MinGW, so no Visual Studio is needed. Tauri calls cross-compiling experimental.

| File | What it is |
|---|---|
| `solace-app/distro/build.sh` | Builds the distro with `mmdebstrap` |
| `solace-app/distro/wsl.conf` | Default user `solace`; Windows program access and drive mounting off |
| `solace-app/distro/solace-desktop` | What the app runs inside the distro |
| `solace-app/src/wsl.rs` | The app's way into the distro, as plain functions with their own tests |
| `solace-app/tauri.windows.conf.json` | Windows-only build settings |

The installer goes to `%LOCALAPPDATA%\Solace` with no administrator rights, and first launch puts the distro in `%LOCALAPPDATA%\Solace\wsl`.

On Aaron's laptop, the latest installer is at `D:\solace-work\Solace-Setup.exe`, and the build environment is a WSL distro called `solace-build` on D:. Neither is in the repository.

## Test results

All of this ran on Windows, against the installed app or the real distro.

| Question | Result |
|---|---|
| Does the installer run without admin rights? | Yes, and upgrading over an install keeps the distro |
| Does first launch import the distro? | Yes, with no input needed |
| Does a project open on the board? | Yes, in 1.4 seconds, in a folder with a space in its name |
| Is a folder outside the distro refused? | Yes, with a message naming where to choose instead |
| Does the server stop with the app? | Yes: closing the window, closing its input (0.02 s) and killing the app all leave no Solace process |
| Is a running server found? | Yes, and it's correctly reported gone after it stops |
| Does the terminal button work? | Yes, it opens a login shell |
| Does uninstall keep the distro? | Yes: the distro, projects and disk folder all stay |
| Does the sandbox work under WSL's kernel? | Yes: no network, emptied home folder, read-only system, and a protected folder stays read-only |
| Can the `solace` user mount `C:`? | No: "must be superuser" |
| Can a Linux program start a Windows program? | `whoami.exe` didn't run, but WSL timed out instead of refusing, and the socket `/run/WSL/1_interop` is still world-writable. Don't read more into this than that one test |
| Port already in use on Windows? | Without a fix, the server binds inside WSL and Windows reaches the **other program**. The app now checks the port on Windows first, then confirms it's reaching Solace |
| How fast does a new port reach Windows? | About 0.6 s after the server says it's listening, so the app waits before opening the board |

## Security: what someone installing Solace should know

Solace has agents write code and then runs that code, so treat it as a developer tool running with your own privileges. Microsoft's WSL security model says a distro is **not** a security boundary from Windows or from other distros, and that code in WSL should get the same trust as the Windows account that started it. For untrusted code, it recommends a separate VM outside WSL. Don't describe Solace on WSL as a sandbox.

### What Solace already limits
- **Agents get no shell and no file tools.** Claude Code runs with Bash, Read, Write, Edit and the rest denied, and hooks, plugins and slash commands off. Codex runs with its shell, patching and other tools off. Claude Code can use only the Solace tool, web fetch and web search; Codex, the Solace tool and web search (`agent_config` in `solace-cli/src/lockdown.rs`). This depends on the agent letting Solace turn those tools off, so a new agent is only safe to add if it can.
- **Rust commands are sandboxed on Linux.** Check, `test` and `build` run with no network, an emptied home folder, and only `target/` writable.
- **Project settings need approval.** An agent can only propose changes to `solace.toml`, `AGENTS.md`, Rust dependencies and build settings.
- **The board only accepts requests from its own address.** The server listens on `127.0.0.1` and refuses a `Host` or `Origin` that isn't its own, so a web page can't drive it (`solace-cli/src/board/server.rs`).

### What remains
1. **Agent-written code can steal credentials.** This is the most serious item. Gleam commands, including `gleam test`, run with network access and the whole home folder visible (`solace-cli/src/store_core.rs:232`). Code an agent writes can read `~/.codex/auth.json`, `~/.claude/.credentials.json` and `~/.config/solace/keys.json`, send them out, and change `.solace` and `solace.toml` directly, getting around approvals. Fix: give Gleam the sandbox Rust has, with dependencies fetched outside it after a change.
2. **The review screen's test run skips the sandbox.** It runs `cargo test` with the sandbox off (`solace-cli/src/view/review.rs:288`), against Spec 14. This looks like an existing bug.
3. **Prompt injection through the web.** Agents can fetch or search the web. A page written to manipulate agents can steer what they write or get project details into a URL. With item 1, that is a full attack chain.
4. **Gleam dependencies need no approval.** An agent can add one by writing `gleam.toml` (Spec 14, A6); only Rust dependency changes are proposals. Dependencies are third-party code that runs during builds and tests.
5. **Approved commands run with no sandbox.** Treat them as trusted code.
6. **The distro is open to Windows programs running as the same user.** Anything the client runs on Windows can read and change the distro through `\\wsl.localhost\Solace`, including projects and sign-in files.
7. **Windows drives and programs.** Mounting and Windows program access are turned off in `/etc/wsl.conf`, but Microsoft's docs note drives can still be mounted manually. It's hardening, so the non-root user matters too.
8. **The board has no login.** Anything that can reach its port can drive it: other Windows users on a shared machine, and anything in the client's other WSL distros, which share the same network. A per-session token would close this.
9. **Solace does keep some credentials.** The Settings page stores pasted API keys in `~/.config/solace/keys.json`, readable only by its owner.
10. **Agent use costs money and sends code out.** Runs call Anthropic or OpenAI under the client's account. Clients with confidentiality rules should check those services' terms.
11. **Installing WSL changes the machine.** It needs administrator rights and turns on Windows' virtualization platform. Company-managed laptops may block it.
12. **Back up projects separately.** Projects live in one virtual disk file, which backup and sync tools such as OneDrive often skip. Removing the distro deletes them.

## Sources

- [WSL configuration](https://learn.microsoft.com/en-us/windows/wsl/wsl-config), [networking](https://learn.microsoft.com/en-us/windows/wsl/networking), [commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands) and [security model](https://wsl.dev/technical-documentation/security/)
- [Windows code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
- [Tauri updater](https://v2.tauri.app/plugin/updater/) and [NSIS template](https://docs.rs/crate/tauri-bundler/2.9.2/source/src/bundle/windows/nsis/installer.nsi)
- [Codex authentication](https://developers.openai.com/codex/auth)
