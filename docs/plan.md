# lull — v1 implementation plan

Built against [`contract.md`](contract.md) (source of truth), [`adr/`](adr/) and
[`../GLOSSARY.md`](../GLOSSARY.md). Each work package (WP) below is sized for one agent,
owns a disjoint set of paths, and names the interfaces it consumes and provides. An agent
must not change another WP's paths. If a WP needs to change the contract, it stops and
reports instead of improvising.

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
  groups, 2 dev servers and Zed.
- **Definition of done:** tests green in CI (macOS runner), `cargo clippy -D warnings`,
  `swift test`, `tsc --noEmit`; behaviour matches the contract section the WP cites; new
  terms are in the glossary.
- **Commits:** small, imperative, ending with the harness's Co-Authored-By line.

## Dependency graph

```
M0 scaffold ──┬─▶ WP1.1 proc source ─┐
              ├─▶ WP1.2 discovery ───┼─▶ WP1.5 daemon+CLI ─▶ WP1.7 e2e + cutover
              ├─▶ WP1.3 activity ────┤
              ├─▶ WP1.4 policy/stop ─┘
              ├─▶ WP2.1 nm-host ───────────────┐
              ├─▶ WP2.2 extension core (mock) ─┼─▶ WP2.4 daemon browser integ ─▶ M3
              ├─▶ WP2.3 browsers install ──────┘
              └─▶ WP4.1 Swift client (fixtures) ─▶ WP4.2/4.3/4.4 app ─▶ M5
Spikes first: S1 ntstat parsing (in WP1.3) · S2 Zen tab groups (WP3.1) · S3 signed
embedded binary + SMAppService + nm-host link (WP4.4)
```

After M0 lands, up to seven agents can work at once: WP1.1–1.4, 2.1, 2.2 and 4.1.

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

## M1 — Daemon + CLI (replaces the prototype)

**WP1.1 — macOS `ProcessSource`** (`crates/core/src/platform/macos/proc.rs`).
- **Listing:** list same-uid pids; for each, read pgid, ppid, start time, executable
  path, controlling tty, cwd (`proc_pidinfo`), and argv + initial env
  (`KERN_PROCARGS2`, tolerating env omitted for restricted binaries).
- **Sockets:** TCP sockets via `PROC_PIDLISTFDS` / `PROC_PIDFDSOCKETINFO`: listeners
  (v4/v6) and accepted connections with local/remote endpoints.
- **Failures:** reported per process, never panicking (feeds contract §4.8).
- **Debug command:** `lull debug snapshot` writes the scrubbed fixture format.

Accept: integration test spawns `python3 -m http.server` under `setsid` with
`CLAUDECODE=1` and finds pgid, cwd, marker, no tty and the listener. One scan of the
author's machine takes under 50 ms.

**WP1.2 — Discovery and classification** (`crates/core/src/discovery/`). Pure functions
over a snapshot.
- **Grouping:** group by pgid; apply the server rule (ADR-0002, project locations) and
  sort the rest into protected groups and unmanaged listeners.
- **Origin:** marker table (built-in + `[[agent_marker]]` config) with tty and
  installed-app vetoes (ADR-0001).
- **Labels:** framework heuristics, project (git root), and `primaryUrl` selection
  with the probe abstracted.
- **Identity:** id assignment and witness matching (§4.6–4.7).

Accept: snapshot tests cover the author's machine (only the vite+workerd and astro
groups become servers; Zed, Herd nginx, brew databases, ControlCenter and Canary Mail
do not). Also covered: an IDE terminal with `CLAUDECODE=1` plus a tty is `user`;
Homebrew `Python.app` is a server; a leader exit keeps the id; a daemon restart
re-matches via witness.

**WP1.3 — Activity** (`crates/core/src/activity/`, `platform/macos/ntstat.rs`).
- **S1 spike first:** subscribe to TCP sources on `com.apple.network.statistics`,
  parse source-add descriptors (local/remote endpoint, pid) for the running kernel's
  message version, and write the findings into ADR-0003 as an addendum. If parsing is
  not feasible in two days, ship the scan backend only and report.
- **`ActivityBackend`:** ntstat implementation plus the accepted-socket-diff fallback;
  exclude the daemon's own pid and same-group clients.
- **`Clock`:** `CLOCK_UPTIME_RAW` + `kern.bootsessionuuid`, wake detection, and the
  wake/start grace (§5).

Accept: a short `curl` against a fixture server raises `traffic` with ntstat; the
fallback catches a 2 s connection; `FakeClock` tests cover sleep (no idle accrual),
wake grace and daemon restart within the same boot.

**WP1.4 — Policy, stop state machine, persistence, history, restart**
(`crates/core/src/{policy,lifecycle,state}/`).
- **Policy:** effective policy per field (override → longest `[[project]]` → defaults)
  and config validation.
- **Stop state machine:** cancellation rules, coalescing, the safety checks before
  `SIGTERM`/`SIGKILL` with witnesses (§7), and dry-run.
- **Persistence:** `StateStore` with an atomic, versioned `state.json` and
  corrupt-file handling.
- **History:** retention (20 entries / 24 h).
- **Restart:** login-shell spawn under `setsid`, success tracking within 60 s,
  `restartFailed`, and the optional faithful env in `$TMPDIR` (ADR-0005).

Accept: table-driven tests using `FakeClock` + `RecordingSignaler` +
`FakeProcessSource`, including: activity during `stopping-soon` returns the server to
running; pgid reuse aborts the signal; a partial exit on SIGTERM still triggers
SIGKILL; an env pin overridden by Unpin persists; a failed restart keeps the record.

**WP1.5 — Daemon, IPC server, CLI** (`crates/cli/`, `crates/core/src/ipc/`).
- **Runtime:** the daemon loop (5 s scan + backend events), single instance via
  `flock`, socket `0600`, JSON-RPC/NDJSON with `hello` negotiation, `seq`'d
  subscriptions, `connectionId`, idempotent mutations, loopback-only URL validation,
  logs with size-capped rotation.
- **Commands:** `lull ls [--all] [--json]`, `stop`, `pin`/`unpin`, `touch`, `open`
  (agent env ⇒ no focus), `restart`, `history`, `forget`, `config get/set`,
  `daemon [--dry-run]`.
- **`lull doctor`:** activity backend, socket/lock state, config validity, browsers
  connected, NM manifests present, and reasons a server is not eligible to stop.

Accept: every fixture in `protocol/fixtures/` replays against the daemon running on
fakes; `lull ls` output matches a golden file.

**WP1.6 — Dev scripts** (`scripts/dev-launchagent.sh`, `scripts/retire-prototype.sh`).
Bootstrap/bootout a dev LaunchAgent `io.github.rgrvlsk.lull.daemon` pointing at
`target/release/lull`; retire `local.devserver-reaper` (bootout, remove plist,
`~/.local/state/devserver-reaper`). Run them manually, never in CI.

**WP1.7 — End-to-end and cutover** (`tests/e2e/`, after 1.1–1.6). Real daemon with
`idle_minutes` set to seconds via a test config: agent-marked servers stop, user and tty
servers stay, pins hold, `curl` keeps a server alive, restart brings it back as `user`.
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
records, `tabs_on_stop`, `open` routing, dropping a connection's state on disconnect.
Accept: fake-connection tests covering two profiles, a reused port (no takeover until
reload) and disconnect clearing activity.

---

## M3 — Firefox / Zen

**WP3.1 — S2 Zen spike (first).** On Zen 1.23.x: does `tabs.group` + `tabGroups.update`
create a visible, titled group or collide with Zen folders and workspaces? Record the
result in the contract (§9) and decide grouping on/off for Zen.
**WP3.2 — Firefox build.** `manifests/firefox.json` (event page, `gecko.id`,
`strict_min_version` 139, `update_url`), API shims, visibility caveat for the key window.
Accept: same checklist as M2 in Firefox and Zen; the native host spawns on macOS 27.

---

## M4 — Menu-bar app (can start right after M0 against fixtures)

**WP4.1 — `LullKit` client** (`macos/Sources/LullKit/`). Socket client, Codable models
verified against `protocol/fixtures/`, subscription with gap recovery, `@Observable`
store. Accept: decodes every fixture; reconnects after daemon restart.
**WP4.2 — Menu UI** (`macos/Sources/LullApp/Menu/`). Live servers grouped by project
with origin and pin badges and countdown; Recently stopped with Restart / Copy command /
Reveal / Open terminal / Forget; row actions; icon count + stopping-soon dot; wording
rules (§10). Preview data from fixtures, including worst cases (long paths, 30 servers,
no servers).
**WP4.3 — Settings** (`macos/Sources/LullApp/Settings/`). Policy, per-project rules,
`open_with`, extension install helper (calls `lull browsers install`), validation errors
from `config.set`.
**WP4.4 — Registration and notifications** (`macos/Sources/LullApp/System/`). **S3
spike first:** a Developer ID-signed app that embeds `lull`, registers it with
`SMAppService.agent` (`BundleProgram`), and serves a working `lull-nm-host` link from
the bundle. Then handle `requiresApproval`, re-register on update, and send
stopping-soon notifications with *Keep alive*.

---

## M5 — Packaging and release

**WP5.1 — Signing and notarisation CI.** Release workflow: build universal `lull`, the
app, both extension builds; sign everything (hardened runtime), notarise, staple.
**WP5.2 — Extension distribution.** Chromium `key` (private half as a secret); AMO
unlisted signing via `web-ext sign`; `updates.json` published on GitHub releases.
**WP5.3 — Cask and docs.** `rgrvlsk/homebrew-tap` cask with a `binary` stanza; README
install and uninstall; delete `scripts/dev-launchagent.sh`. Ask the author before
creating any remote repository.

Exit: `brew install --cask rgrvlsk/tap/lull` on a clean macOS 14 VM → app registers
daemon → `lull browsers install` → Helium groups tabs and an agent-started server
is stopped after its idle time.
