# lull — v1 implementation plan

Built against [`contract.md`](contract.md) (source of truth), [`adr/`](adr/) and
[`../GLOSSARY.md`](../GLOSSARY.md). Each work package (WP) below is sized for one agent,
owns a disjoint set of paths, and names the interfaces it consumes and provides. An agent
must not change another WP's paths. If a WP needs to change the contract, it stops and
reports instead of improvising.

Revised 2026-10-12 after a three-way scope and safety review: v1 is M1 + M2 shipped as a
Homebrew formula (ADR-0008); M3–M5 are v1.1. M0.5 was added so parallel agents share
domain types instead of inventing them.

## Ground rules for every WP

- **Toolchains:** Rust stable (edition 2024); Swift 6 with Xcode 26, deployment target
  macOS 14; TypeScript with esbuild for the extension; Node only as a dev tool.
- **Protocol:** the wire types live once, in `crates/protocol` (serde). Golden NDJSON
  exchanges in `protocol/fixtures/*.ndjson` are the cross-language contract. The Rust,
  TypeScript and Swift test suites all decode and re-encode them, so the three cannot
  drift. Any protocol change edits the Rust types, the fixtures and `contract.md` §8 in
  one commit, and bumps `protocol.minor` (or `major` if it breaks compatibility).
- **Platform behind traits:** core logic never calls libproc, ntstat, signals, clocks
  or the filesystem directly. It goes through `ProcessSource`, `ActivityBackend`,
  `Clock`, `Signaler`, `Spawner` and `StateStore`. Every trait has a fake in
  `crates/core/src/testing/`.
- **Fixtures, not live agents:** `lull debug snapshot > file.json` records a scrubbed
  process/socket snapshot. Env keeps only marker keys plus `PORT`/`HOST`/`NODE_ENV`, and
  paths under `$HOME` are rewritten to `/Users/x`. Classification tests run on
  snapshots, including the author's machine recorded on 2026-10-12: 13 listening
  groups, 2 dev servers and Zed. Before auto-stop is trusted for a launch shape, a
  snapshot fixture must cover it: one per supported agent (Claude Code's Bash tool,
  Codex CLI in a terminal and under T3, at least one other marker-table agent) and one
  per common wrapper (`npm run`, `concurrently`, `turbo`, `wrangler dev`/workerd,
  esbuild/vite helpers).
- **Definition of done:** tests green in CI (macOS runner), `cargo clippy -D warnings`,
  `swift test`, `tsc --noEmit`; behaviour matches the contract section the WP cites; new
  terms are in the glossary.
- **Commits:** small, imperative, ending with the harness's Co-Authored-By line.

## Dependency graph

```
                                    v1
M0 ─▶ M0.5 model ─┬─▶ WP1.1 proc source ─┐
                  ├─▶ WP1.2 discovery ───┼─▶ WP1.5 daemon+CLI ─▶ WP1.7 e2e + cutover ─┐
                  ├─▶ WP1.3 activity ────┤                                            │
                  ├─▶ WP1.4 policy/stop ─┘                                            ├─▶ WP-R v1 release
                  ├─▶ WP2.1 nm-host ───────────────┐                                  │
                  ├─▶ WP2.2 extension core (mock) ─┼─▶ WP2.4 daemon browser integ ────┘
                  └─▶ WP2.3 browsers install ──────┘
                                    v1.1
WP-R ─▶ WP-T terminal UI · M3 Firefox/Zen · M4 app (WP4.1 may start any time after M0.5) ─▶ M5 cask
Spikes first: S1 ntstat parsing (in WP1.3) · S2 Zen tab groups (WP3.1) · S3 signed
embedded binary + SMAppService + nm-host link (WP5.1, v1.1)
```

After M0.5 lands, up to six agents can work at once on v1: WP1.1–1.4, 2.1 and 2.2.

---

## M0 — Scaffold (1 agent, serial, blocks everything)

**WP0 — repo skeleton and protocol crate.**
Paths: `Cargo.toml`, `crates/{protocol,core,cli}/`, `protocol/fixtures/`, `extension/`
(package.json, tsconfig, empty src), `macos/` (SwiftPM package `LullKit` + app target
stub), `.github/workflows/ci.yml`, `scripts/`.

- **`crates/protocol`:** every type in contract §4 data model, §7 stopped-server record,
  §8 methods/notifications/commands, and `Config` (§6), with serde round-trip tests.
- **Fixtures:** `protocol/fixtures/`, one NDJSON exchange per method (hello including
  `protocol_mismatch`, list, subscribe snapshot + gap, stop, pin, setAutoStop, touch,
  history, restart, forget, open, config get/set with validation error, the browser
  sync/activity calls and `tab.*` commands with acks).
- **Traits and fakes:** `crates/core` trait definitions with fakes, no logic.
- **CI:** macOS runner running cargo test/clippy, `swift test` in `macos/`, and
  tsc + unit tests in `extension/`.

**Accept:** CI green on an empty implementation; the fixtures decode in Rust; the TS
and Swift fixture-decoding test harnesses exist (they may be skipped until WP2.2/4.1).

---

## M0.5 — Shared model (1 agent, serial, blocks M1/M2)

**WP0.5 — domain types and ownership** (`crates/core/src/model.rs`,
`crates/core/src/traits.rs`, `crates/cli/src/main.rs`). Types only, no logic:
- **Discovery output:** an observed group (members, listeners, completeness flags per
  §4.8) and a classified server (origin, framework, project, launch command,
  `primaryUrl`, protected / unmanaged / server verdict).
- **Lifecycle:** the per-server record WP1.4 owns (state, witnesses, activity instant
  and source, overrides, `stopsAt`) and the stop outcome (`stopped`, `aborted`,
  `stuck`).
- **Activity:** one `ActivityEvent` stream that WP1.3 (traffic), WP2.4 (tab) and WP1.5
  (manual) produce and **WP1.4 alone merges** into the lifecycle record.
- **Backend health:** `ActivityBackend` gains a health report so failure is
  distinguishable from silence (§5); `ActivityEvent::Traffic` gains whether the
  connection is new or established, and the snapshot carries established connections
  with client pids.
- **Covered browsers:** a `CoveredBrowsers` input (set of browser pids) that WP2.4
  provides and WP1.3 consumes; empty in M1.
- **CLI dispatch:** `main.rs` is owned by WP1.5. M0.5 stubs the subcommand table with
  one module per owner (`nm_host` → WP2.1, `browsers` → WP2.3), each exposing
  `pub fn run(args) -> ExitCode`, so no other WP edits `main.rs`.

Accept: compiles, fakes updated, every M1/M2 WP below names only types from `model.rs`
and `traits.rs` at its boundaries.

---

## M1 — Daemon + CLI (replaces the prototype)

**WP1.1 — macOS `ProcessSource`** (`crates/core/src/platform/macos/proc.rs`).
- **Listeners first:** walk the socket fds of every same-uid process with libproc
  (`PROC_PIDLISTFDS`, then `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` on socket fds only);
  that gives every TCP socket with its state and exact owning pid. Only listener pids,
  their group members and connection peers on server ports are inspected further.
  (Revised 2026-10-12: `pcblist_n` only returns the caller's own sockets on macOS 27;
  the fd walk measured 3 ms.)
- **Process details:** pgid, ppid, start time, executable path, controlling tty, cwd
  (`proc_pidinfo`), and argv + initial env (`KERN_PROCARGS2`, tolerating env omitted
  for restricted binaries), cached per (pid, start time).
- **Sockets:** listeners (v4/v6), and established connections to server ports with
  local/remote endpoints and the client pid where it is a same-uid process.
- **Failures:** reported per process and per field (members, env, cwd, executable,
  tty, sockets), never panicking (feeds contract §4.8).
- **Debug command:** `lull debug snapshot` writes the scrubbed fixture format.

Accept: integration test spawns `python3 -m http.server` under `setsid` with
`CLAUDECODE=1` and finds pgid, cwd, marker, no tty and the listener. One scan of the
author's machine takes under 50 ms.

**WP1.2 — Discovery and classification** (`crates/core/src/discovery/`). Pure functions
over a snapshot.
- **Grouping:** group by pgid; apply the server rule (ADR-0002, project locations) and
  sort the rest into protected groups (§4.4 pattern table: exe path + argv, never
  process name alone) and unmanaged listeners.
- **Origin:** marker table (built-in + `[[agent_marker]]` config) with tty and
  installed-app vetoes (ADR-0001).
- **Labels:** framework heuristics, project (git root), `primaryUrl` (argv/`PORT`
  port, else lowest non-inspector port; no network probe), and the launch command
  (§7 step 6: skip `-c` shell wrappers; empty when unknown).
- **Eligibility input:** any unreadable field (§4.8) marks the group incomplete.
- **Identity:** id assignment and witness matching (§4.6–4.7).

Accept: snapshot tests cover the author's machine (only the vite+workerd and astro
groups become servers; Zed, Herd nginx, brew databases, ControlCenter and Canary Mail
do not). Also covered: an IDE terminal with `CLAUDECODE=1` plus a tty is `user`;
Homebrew `Python.app` is a server; an npm-installed Claude Code (`node …/claude-code/…`)
is protected; a leader exit keeps the id; a daemon restart re-matches via witness; the
launch command of a Claude Code Bash-tool server is `npm run dev`, not the harness's
shell wrapper; an unreadable tty makes the group incomplete; the per-agent and wrapper
fixtures from the ground rules.

**WP1.3 — Activity** (`crates/core/src/activity/`, `platform/macos/ntstat.rs`).
- **S1 spike first:** subscribe to TCP sources on `com.apple.network.statistics`,
  parse source-add descriptors (local/remote endpoint, pid) for the running kernel's
  message version, and write the findings into ADR-0003 as an addendum. If parsing is
  not feasible in two days, ship the scan backend only and report.
- **`ActivityBackend`:** ntstat implementation plus the accepted-socket-diff fallback;
  exclude the daemon's own pid and same-group clients; report health, and switch to the
  fallback when ntstat fails at runtime (§5).
- **Established connections (§5 `traffic` (b)):** at every scan, an established
  connection from a client outside the group that is not in `CoveredBrowsers` fires
  `traffic`. This keeps the prototype's behaviour for open tabs during M1.
- **`Clock`:** `CLOCK_UPTIME_RAW` + `kern.bootsessionuuid`, wake detection, and the
  wake/start grace (§5).

Accept: a short `curl` against a fixture server raises `traffic` with ntstat; the
fallback catches a connection held open across a scan (not a fixed duration: a
connection shorter than the scan interval is caught only by chance); an idle open
websocket from an uncovered client keeps a server active, and the same websocket from a
covered pid does not; a failing ntstat fake reports unhealthy and the daemon falls back;
`FakeClock` tests cover sleep (no idle accrual), wake grace and daemon restart within
the same boot.

**WP1.4 — Policy, stop state machine, persistence, history, restart**
(`crates/core/src/{policy,lifecycle,state}/`).
- **Policy:** effective policy per field (override → longest `[[project]]` → defaults)
  and config validation.
- **Stop state machine:** cancellation rules, coalescing, the fresh-observation and
  eligibility re-check before an automatic `SIGTERM`, the witness checks before
  `SIGTERM`/`SIGKILL`, the `aborted` and `stuck` outcomes, removal only once the group
  is empty (§7), and dry-run.
- **Activity merge:** sole owner of the per-server activity instant and source;
  consumes every `ActivityEvent`.
- **Persistence:** `StateStore` with an atomic, versioned `state.json` and
  corrupt-file handling.
- **History:** retention (20 entries / 24 h).
- **Restart:** login-shell spawn under `setsid` of the launch command, success
  tracking within 60 s, `restartFailed`, and `not_restartable` for records without a
  launch command (ADR-0005).

Accept: table-driven tests using `FakeClock` + `RecordingSignaler` +
`FakeProcessSource`, including: activity during `stopping-soon` returns the server to
running; an incomplete observation at `stopsAt` defers the stop; pgid reuse aborts the
signal; a partial exit on SIGTERM still triggers SIGKILL; all witnesses gone but new
members present after SIGTERM aborts the stop with no history record; an env pin
overridden by Unpin persists; a failed restart keeps the record.

**WP1.5 — Daemon, IPC server, CLI** (`crates/cli/`, `crates/core/src/ipc/`).
- **Runtime:** the daemon loop (5 s scan + backend events), single instance via
  `flock`, exit when the daemon's own executable path disappears (§3), socket `0600`, JSON-RPC/NDJSON with `hello` negotiation, `seq`'d
  subscriptions, `connectionId`, idempotent mutations, loopback-only URL validation,
  logs with size-capped rotation.
- **Commands:** `lull ls [--all] [--json]`, `stop`, `pin`/`unpin`, `touch`, `open`
  (agent env ⇒ no focus), `restart`, `history`, `forget`, `config get/set`,
  `daemon [--dry-run]`.
- **Dispatch:** owns `crates/cli/src/main.rs` (stubbed in M0.5).
- **`lull doctor`:** activity backend and its health, aborted or stuck stops, socket/lock state, config validity, browsers
  connected, NM manifests present, and reasons a server is not eligible to stop.

Accept: every fixture in `protocol/fixtures/` replays against the daemon running on
fakes; `lull ls` output matches a golden file.

**WP1.6 — Dev scripts** (`scripts/dev-launchagent.sh`, `scripts/retire-prototype.sh`;
the first is deleted by WP-R).
Bootstrap/bootout a dev LaunchAgent `io.github.rgrvlsk.lull.daemon` pointing at
`target/release/lull`; retire `local.devserver-reaper` (bootout, remove plist,
`~/.local/state/devserver-reaper`). Run them manually, never in CI.

**WP1.7 — End-to-end and cutover** (`tests/e2e/`, after 1.1–1.6). Real daemon with
`idle_minutes` set to seconds via a test config: agent-marked servers stop, user and tty
servers stay, pins hold, `curl` keeps a server alive, an open websocket keeps a server
alive, restart brings it back as `user`.
Then the author performs the direct cutover (M1 exit criterion).

---

## M2 — Native messaging + Chromium extension (Helium first)

**WP2.1 — nm-host** (`crates/cli/src/nm_host.rs`). argv[0] dispatch, Chrome/Firefox
length-prefixed framing, caller validation (Chromium origin, Firefox add-on id), 1 MiB
cap, bridge to the daemon socket as `client: "browser"`, clean exit on either side's EOF.
Accept: framing fuzz test; wrong origin is rejected; the bridge passes the browser
fixtures.

**WP2.2 — Extension core** (`extension/src/`). Single MV3 codebase.
- **Connection:** connection manager (`connectNative`, `hello`, reconnect via
  `alarms`), tab snapshot/sync.
- **Grouping and binding:** Dev group create/reuse per window, drag-out memory, binding
  rules (§9).
- **Visibility:** content script on the loopback origins.
- **Commands:** `tab.*` executors with acks and URL re-check, and the badge when the
  daemon is absent.

Develop against a mock port that replays fixtures. Accept: unit tests on the mock; a
manual Helium checklist (group forms, the visible-but-unfocused second-monitor case
counts, discard on stop, reload after restart).

**WP2.3 — `lull browsers install`** (`crates/cli/src/browsers/`).
- **Detection:** Helium, Chrome, Brave, Edge, Vivaldi, Chromium; Firefox and Zen via
  the Mozilla dir.
- **NM manifests:** `allowed_origins` / `allowed_extensions` from the pinned ids.
- **Extension files:** copy the unpacked extension to
  `~/Library/Application Support/lull/extension/chromium`, open the extensions page or
  the XPI.
- **Removal:** `lull browsers uninstall`.

Accept: tests against a temp `$HOME`; `lull doctor` reports per-browser state.

**WP2.4 — Daemon browser integration** (`crates/core/src/browser/`). Per-connection tab
registry, `tab` activity (receipt-timestamped), binding to server ids and stopped-server
records, `tabs_on_stop`, `open` routing, dropping a connection's state on disconnect,
and `CoveredBrowsers`: the nm-host's pid from `LOCAL_PEERPID`, its parent as the
browser pid, plus descendants (§5).
Accept: fake-connection tests covering two profiles, a reused port (no takeover until
reload), disconnect clearing activity and coverage, and a second browser without the
extension staying uncovered.

---

## WP-R — v1 release (after WP1.7 and WP2.4)

- **Formula:** `rgrvlsk/homebrew-tap` `Formula/lull.rb` building `lull` from the
  release tag, installing the unpacked Chromium extension under `share/lull`, and a
  `service do` block running `lull daemon` with logs in `~/Library/Logs/lull`.
- **Chromium id:** generate the extension key pair; commit the public `key` to the
  manifest; keep the private half as a maintainer secret (ADR-0007).
- **Docs:** README install (`brew install rgrvlsk/tap/lull`, `brew services start
  lull`, `lull browsers install`) and uninstall; delete `scripts/dev-launchagent.sh`.
- Ask the author before creating any remote repository.

Exit: on a clean macOS 14 VM, `brew install` → `brew services start lull` →
`lull browsers install` → Helium groups tabs, and an agent-started server is stopped
after its idle time while one shown in a visible tab is not.

---

# v1.1

Specified so v1 keeps their seams; started after WP-R ships (WP4.1 may start earlier
against fixtures).

---

## WP-T — Terminal UI (first in v1.1; ADR-0009)

Paths: `crates/cli/src/ui/`, the `ui` arm and bare-`lull` dispatch in
`crates/cli/src/main.rs`, `ratatui` and `crossterm` in `Cargo.toml` and
`crates/cli/Cargo.toml`, README "Use".

- **Live data:** one socket connection; `subscribe` with gap re-subscribe, `servers.list`
  every few seconds for CPU/RSS, history and config fetched when shown and after
  `server.removed`; reconnect with a visible "daemon unreachable" state.
- **Views:** servers grouped by project with a detail pane (stacked below on narrow
  terminals), history, config; actions open, stop, pin/unpin, keep alive (`touch`),
  restart, forget, config edits through `config.set`.
- **Safety:** control characters in process-supplied text are replaced before drawing;
  confirm before stopping a user-started server and before restart (shows the command).
- **Look:** 16-colour ANSI palette, `NO_COLOR`, keyboard only, wording rules (§10).

Accept: renders `protocol/fixtures/` data into a test backend without panics at
40×10 and 200×60, including long paths, 30 servers and none; a server whose argv
contains `\x1b[2J` draws no escape; idle CPU stays near zero; the terminal is restored
after `q`, Ctrl-C and a panic.

---

## M3 — Firefox / Zen

**WP3.1 — S2 Zen spike (first).** On Zen 1.23.x: does `tabs.group` + `tabGroups.update`
create a visible, titled group or collide with Zen folders and workspaces? Record the
result in the contract (§9) and decide grouping on/off for Zen.
**WP3.2 — Firefox build.** `manifests/firefox.json` (event page, `gecko.id`,
`strict_min_version` 139, `update_url`), API shims, visibility caveat for the key window.
Accept: same checklist as M2 in Firefox and Zen; the native host spawns on macOS 27.

---

## M4 — Menu-bar app (WP4.1 can start right after M0.5 against fixtures)

**WP4.1 — `LullKit` client** (`macos/Sources/LullKit/`). Socket client, Codable models
verified against `protocol/fixtures/`, subscription with gap recovery, `@Observable`
store. Accept: decodes every fixture; reconnects after daemon restart.
**WP4.2 — Menu UI** (`macos/Sources/LullApp/Menu/`). Live servers grouped by project
with origin and pin badges and countdown; Recently stopped with Restart (only when the record has a launch command) / Copy command /
Reveal / Open terminal / Forget; row actions; the cat as the icon, with the running count and the stopping-soon dot as optional
app preferences that are off by default; wording
rules (§10). Preview data from fixtures, including worst cases (long paths, 30 servers,
no servers).
**WP4.3 — Settings** (`macos/Sources/LullApp/Settings/`). Policy, per-project rules,
`open_with`, launch at login, validation errors from `config.set`. No extension install
helper: the extension is optional (ADR-0011) and `lull browsers install` stays the way in.
**WP4.4 — Notifications** (`macos/Sources/LullApp/System/`). Stopping-soon notifications
with *Keep alive*; `Info.plist` and `scripts/build-app.sh`, which builds `Lull.app` and
signs it ad hoc.

M4 is connect-only and publishes nothing: without a Developer ID the app connects to
whichever daemon owns the socket (the formula's `brew services` job) and registers none.
Accept: the app shows, stops, pins and restarts servers of the formula's daemon,
reconnects after `brew services restart lull`, and notifies before a stop.

---

## M5 — Packaging and release

**WP5.1 — Registration, signing and notarisation.** **S3 spike first:** a Developer
ID-signed app that embeds `lull`, registers it with `SMAppService.agent`
(`BundleProgram`), and serves a working `lull-nm-host` link from the bundle. Then handle
`requiresApproval` and re-register on update. Release workflow: build universal `lull`,
the app, both extension builds; sign everything (hardened runtime), notarise, staple.
**WP5.2 — Firefox distribution.** AMO unlisted signing via `web-ext sign`;
`updates.json` published on GitHub releases.
**WP5.3 — Cask and docs.** `rgrvlsk/homebrew-tap` cask with a `binary` stanza and
`conflicts_with formula: "lull"`, a caveat to `brew services stop lull` first, and the
formula retired (ADR-0008); README install and uninstall. Ask the author before
creating any remote repository.

Exit: `brew install --cask rgrvlsk/tap/lull` on a clean macOS 14 VM → app registers
daemon → `lull browsers install` → Helium groups tabs and an agent-started server
is stopped after its idle time.
