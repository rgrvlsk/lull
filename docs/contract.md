# lull — v1 contract

Status: **draft for review**. This file is the single source of truth that the daemon,
the browser extension and the menu-bar app are built against. Anything marked
**OPEN** must be decided (and recorded as an ADR in `docs/adr/`) before the part
that depends on it is built.

## 1. Problem

AI coding agents (Claude Code, Codex, T3 Code threads) start dev servers — `vite`,
`next dev`, `astro dev`, `php -S`, `python -m http.server`, wrangler/workerd — and
rarely stop them. Their owning agent session ends, the server is reparented to
`launchd`, and it keeps running for days. Evidence from the author's machine on
2026-10-11: 11 orphaned servers, oldest 2 d 19 h, all with a dead parent agent.

A bash prototype (`~/.local/bin/devserver-reaper`, launchd every 60 s) proved that
env markers identify agent-started processes (after one false positive was fixed — see
ADR-0001), and that
CPU-delta is a poor activity signal: in its first night it stopped 9 servers,
but a vite + workerd pair stayed "active" from background CPU noise alone. The real signal — *is a human looking at it?* — lives in
the browser.

## 2. Goals / non-goals

Goals (v1)
- G1 Discover every dev server owned by the current user, label its origin
  (agent / user) and project.
- G2 Stop idle **agent-started** servers after `idleMinutes` (default 30).
  User-started servers are shown but never auto-stopped unless the user opts in.
- G3 Keep dev-server tabs in one "Dev" tab group per browser window and use tab
  visibility as the primary activity signal.
- G4 Menu-bar app to monitor, open, stop, pin servers and change policy.
- G5 A CLI agents can call (`lull open <url>`, `lull ls`).
- G6 Zero instructions required for correctness; at most one line in agent
  instruction files for convenience.

Non-goals (v1): Safari, Windows/Linux UI (core stays portable), *automatic* waking
(restarting a stopped server when its port is requested) — manual restart from the
recently-stopped list is in scope (§7) — remote/containers (Docker, VMs).

## 3. Components

```
 Swift menu-bar app ──┐                       ┌── lull CLI (same binary)
                      ▼                       ▼
            ┌──────── lulld daemon (Rust) ────────┐
            │ discovery · activity · policy · stop    │
            │ unix socket: JSON-RPC 2.0, NDJSON       │
            └───────────────▲─────────────────────────┘
                            │ same socket
              lull-nm-host (same binary, argv[0] dispatch)
                            ▲ stdio, Chrome/Firefox native messaging framing
              Browser extension (MV3; Chromium + Firefox/Zen builds)
```

- One Rust binary, dispatched by `argv[0]` / subcommand: `lull daemon`,
  `lull <cli-cmd>`, and the symlink `lull-nm-host` (native messaging forbids
  arguments in the manifest `path`).
- The daemon runs as a per-user LaunchAgent registered by the app via
  `SMAppService.agent`; it keeps running when the app is closed. v1 ships only as a
  Homebrew cask (app + embedded binary, `lull` linked onto `PATH`); single instance via
  `flock` on `~/Library/Application Support/lull/daemon.lock` (ADR-0006).
- The app is a pure client of the socket; it owns no policy.

## 4. Discovery (daemon)

Scan every `scanIntervalSec` (default 5):

1. Listening TCP sockets of processes owned by the current uid (libproc on macOS;
   trait `ProcessSource` so Linux `/proc` can be added later).
2. Group by **process group** (pgid). Read argv, env and cwd of every member via
   `KERN_PROCARGS2` / `proc_pidinfo`.
3. A *server* = one pgid with ≥1 listener where some listening member's cwd is a
   **project location** — strictly inside `$HOME` but neither `$HOME` itself nor under
   `~/Library`, or under `/tmp`, `/private/tmp`, `/private/var/folders` — and no member's
   executable is under `/Applications`, `~/Applications` or `/System` (ADR-0002). Every
   other listening group is an **unmanaged listener**: shown only by `lull ls --all`,
   never stopped, no activity tracking.
4. Exclude **protected** groups: any pgid that contains the daemon itself, an agent
   runtime (`claude`, `codex`), the T3 Code app or its direct children (e.g. its
   `cloudflared` tunnel), or a browser. Protected groups are never listed.
5. Classify:
   - `origin.kind = "agent"` if any member's initial env carries an agent marker
     (`CLAUDECODE=1`, `CODEX_THREAD_ID`, or an entry of the built-in marker table —
     Cursor, Gemini CLI, opencode, Copilot… after `vercel/detect-agent` — extensible via
     `[[agent_marker]]` in config) **and** no member has a controlling tty **and** no
     member's executable is under `/Applications`, `~/Applications` or `/System`
     (ADR-0001). `T3CODE_CLI_PATH` is not a marker.
     `origin.agent ∈ {claude, codex, cursor, gemini, opencode, copilot, …, unknown}`;
     `origin.sessionId` from `CLAUDE_CODE_SESSION_ID` or `CODEX_THREAD_ID`.
   - else `origin.kind = "user"`.
   - `framework` by argv heuristics (vite, next, astro, nuxt, remix, wrangler,
     php-builtin, artisan, python-http, rails, django, generic).
   - `project` = basename of the git root containing `cwd`, else `cwd` basename.
6. `id` = `"{pgid}-{oldestMemberStartEpochMs}"`, computed the first time the group is seen
   and kept while any member survives (the leader is usually an agent shell that exits
   at once, so neither "leader" nor "lowest pid" is stable). xnu never reuses a pgid
   number while the group has a member, so the id cannot collide.
7. Persistent state: `~/Library/Application Support/lull/state.json` (versioned schema,
   written atomically via temp file + rename) maps `id → {pgid, witnesses: [{pid,
   startEpochMs}], lastActivityAt, lastActivitySource, pinned, autoStop}`; written on
   change, entry dropped on `server.removed`. On daemon start a live group resumes a
   persisted entry only if its pgid matches **and** at least one witness (pid + start
   time) is still a member — the computed id alone may differ because the oldest member
   can have exited. Otherwise, and for every server at first install, the server starts
   with the `started` grace period and is never stopped on the first scan. A corrupt or
   unknown-version file is moved aside (`state.json.corrupt-<ts>`) and the daemon starts
   fresh, logging it.
8. Incomplete observation: if a member's env, cwd or sockets cannot be read in a scan
   (process exiting, permission), the server keeps its previous classification and is
   **not eligible for an automatic stop** in that scan. A server is removed only when
   its pgid has no members left; losing its listeners while members survive keeps it
   listed with `ports: []` and `state: "no-listener"` (no auto-stop until a listener
   returns or the group exits).

### Data model

```jsonc
// Server
{
  "id": "22249-1791530155000",
  "pgid": 22249, "pids": [22250, 22276, 22293],
  "command": "npm run dev", "argv": ["npm","run","dev"],
  "cwd": "/Users/x/proj/apps/crm", "project": "crm",
  "framework": "vite",
  "ports": [5187, 56306], "primaryUrl": "http://127.0.0.1:5187/",
  "origin": { "kind": "agent", "agent": "claude", "sessionId": "d263…" },
  "startedAt": "2026-10-09T00:35:55Z",
  "cpuPercent": 0.1, "rssBytes": 182452224,
  "lastActivityAt": "2026-10-11T20:12:00Z",
  "lastActivitySource": "tab",          // tab | traffic | started | manual
  "idleMinutes": 30,                    // effective policy value
  "autoStop": true,                     // effective policy value
  "pinned": false,
  "state": "running",                   // running | stopping-soon | stopping | no-listener
  "stopsAt": "2026-10-11T20:42:00Z",   // null when autoStop=false or pinned
  "tabs": [{ "connectionId": "c7", "browser": "helium", "tabId": 812, "windowId": 3 }]
}
```

`primaryUrl`: the listener whose port is not an inspector/internal port (heuristic:
port mentioned in argv, else lowest port that answers HTTP `HEAD /` within 300 ms;
inspector ports 9229–9239 are skipped). The probe runs once per server id and is cached,
re-run only when the server's listener set changes or the previous probe failed. Connections opened by the daemon itself never count as activity (§5).

## 5. Activity model

A server is **active** at time *t* if any source fired within the last
`idleMinutes`:

| Source | Fires when | Notes |
|---|---|---|
| `tab` | one of the server's tabs is **visible** (`document.visibilityState == "visible"`, reported by a content script) and the OS user is present (`idle.queryState(60) == "active"`), whether or not its window has focus | heartbeat every 30 s while the condition holds; primary signal. Focus is used only for display and `open` routing. A visible window on a second monitor counts — that is intended |
| `traffic` | a **new** TCP connection to one of the server's ports from a process outside the group (ntstat event; fallback: 5-tuple not seen in the previous scan) | catches new page loads, Playwright, curl; traffic on already-open connections (keep-alive reloads, HMR websockets) does not count |
| `started` | server first seen | grace period = `idleMinutes` |
| `manual` | user/agent ran `open`, `touch`, or un-pinned | |

CPU is reported for display only and is **not** an activity source.

`traffic` is observed through the kernel network-statistics control socket
(`com.apple.network.statistics`, ABI in xnu `bsd/net/ntstat.h`; ADR-0003): the daemon
subscribes to TCP source add events and fires `traffic` when a connection's local port
belongs to a server and its client end is owned by a pid outside the group and other than
the daemon. This catches 50 ms agent `curl`s that a periodic scan misses. If the control
socket cannot be opened (`net.statistics_privcheck=1`) or reports an unknown message
version, the daemon falls back to diffing the group's accepted sockets every scan and
reports `activityBackend: "scan"` in `hello`/`lull doctor`.

## 6. Policy

Effective policy, per field, = first defined value of: per-server override (pin /
autoStop toggle, persisted in `state.json`, dropped when the server goes away) → the
**longest** matching `[[project]]` path → defaults. Invalid config is rejected as a
whole (`config.set` returns the validation error; a bad file on disk keeps the last good
config and is reported by `lull doctor`).

```toml
# ~/Library/Application Support/lull/config.toml
idle_minutes = 30
auto_stop = "agent"          # "agent" | "all" | "none"
warn_before_minutes = 2      # stopping-soon state + notification
tabs_on_stop = "discard"   # "discard" | "close" | "keep"
history_size = 20
history_hours = 24
faithful_restart = false     # store env in $TMPDIR for faithful restarts (§7)

[[project]]
path = "~/dev/example-app"   # `~` expanded; prefix match on whole path components of cwd
idle_minutes = 90
# auto_stop = "none"
```

Opt-out per process: env `LULL_KEEP=1` → `pinned = true`.

## 7. Stopping

Stop state machine: `running → stopping-soon → stopping → (removed)`. Any activity,
pin, `autoStop` change or policy change that makes the server ineligible returns
`stopping-soon → running`; once `SIGTERM` is sent the stop cannot be cancelled. A manual
`servers.stop` skips `stopping-soon`; concurrent stops of the same id are coalesced.

1. `state = "stopping-soon"` at `stopsAt - warn_before_minutes`; app may show a
   notification with *Keep alive* (→ `manual` activity).
2. At `stopsAt`: `SIGTERM` to the pgid, `state = "stopping"`, non-blocking.
3. After 5 s, `SIGKILL` to whatever remains in the pgid.
4. Emit `server.removed {id, reason}` and apply `tabs_on_stop` to the server's tabs in
   every connected browser: `discard` (default — `tabs.discard`: the tab keeps its URL
   and place in the Dev group but frees its memory; the active tab cannot be discarded
   and is left as is), `close`, or `keep`.
5. Append a line to `~/Library/Logs/lull.log`.
6. Record a **stopped-server record** in `state.json` history (last `history_size` = 20
   entries, none older than `history_hours` = 24): `{id, project, framework, command,
   argv, cwd, ports, origin, stoppedAt, reason, restartEnv, tabs}`. `argv` is the oldest
   member's argv captured when the server was first seen (e.g. `npm run dev`).
   `restartEnv` keeps only `PORT`, `HOST`, `NODE_ENV` when present. Servers that exit on
   their own (`reason: "exited"`) are recorded too.

### Restart (ADR-0005)

`servers.restart {id}` / `lull restart <id>` on a stopped-server record:

- Spawns `$SHELL -l -c 'exec "$@"' lull-restart <argv…>` in `cwd`, in a new session
  (`setsid`, so a new pgid outside the daemon's), stdin `/dev/null`, stdout+stderr to
  `~/Library/Logs/lull/restart-<id>.log`. The environment is the user's login-shell
  environment plus `restartEnv` — never the original environment, so secrets are not
  persisted and agent markers are dropped.
- Optional `faithful_restart = true`: at first sight the daemon also stores the server's
  initial env minus agent markers in `$TMPDIR/lull/env/<id>.json` (`0600`; `$TMPDIR` is
  per-user `0700` and emptied by macOS at boot), and restart applies it on top of the
  login env. Lost after a reboot by design.
- The restarted process has no agent marker and no tty, so it is classified
  `origin.kind = "user"` and not auto-stopped by default; it gets a new id. The record is
  marked `restarting` (a second restart of the same record is refused) and removed from
  history only when a new server with a listener is discovered in `cwd` within 60 s; its
  `tabs` (still bound to the record, §9) are then reloaded. On timeout or early exit
  the record stays, marked `restartFailed`, with a link to the restart log. The
  confirmation shows the exact command that will run. If the port is taken, the server's own behaviour applies (fail
  or pick another port); the log shows it.

Never signal a pgid that is protected (§4.4) or not owned by the current uid,
re-checked immediately before each signal. Additionally refuse pgid ≤ 1 and the
daemon's own pgid, and abort unless at least one member recorded at decision time (pid +
start time) is still in the group — that proves the pgid has not been reused, so new
children forked meanwhile are part of the same group and are signalled too. The same
check is repeated after the 5 s wait, before `SIGKILL`; members that exited on `SIGTERM`
are expected.

## 8. IPC

Socket: `~/Library/Application Support/lull/daemon.sock`, mode `0600`.
Framing: one JSON-RPC 2.0 message per line (NDJSON). Every client sends `hello`
first.

Trust model (ADR-0004): the boundary is the uid. Every client — app, CLI, browser via
nm-host, coding agents — may call every method; there are no roles. Visibility, not
access control, is the defence: the app shows every live server, every pin and every
recent stop.

### Client → daemon

| Method | Params | Result |
|---|---|---|
| `hello` | `{client: "app"\|"cli"\|"browser", version, protocol: {major, minor}, browser?: {name, profile?, version, extensionVersion}}` | `{daemonVersion, protocol: {major, minor}, connectionId, capabilities: [..], activityBackend: "ntstat"\|"scan"}` — a different `major` is rejected with error `-32010 protocol_mismatch` carrying the daemon's version; clients show "update lull" |
| `servers.list` | `{}` | `Server[]` |
| `servers.stop` | `{id}` | `{}` |
| `servers.pin` | `{id, pinned}` | `Server` |
| `servers.setAutoStop` | `{id, autoStop: bool \| null}` | `Server` — per-server override; `null` clears it |
| `servers.touch` | `{id}` | `Server` (manual activity) |
| `servers.history` | `{}` | `StoppedServer[]` |
| `servers.restart` | `{id}` (stopped-server record) | `{pid}` |
| `servers.forget` | `{id}` | `{}` — drop a stopped-server record |
| `servers.open` | `{id, path?}` or `{url}` | `{opened: bool, browser?}` — routed to a connected browser, else `open(1)` with the configured browser. `url` must be `http(s)` with a loopback or `*.localhost` host; anything else is rejected (never passed to `open(1)`) |
| `config.get` / `config.set` | `{}` / `{patch}` | `Config` |
| `subscribe` | `{topics: ["servers","browsers"]}` | stream of notifications below |

### Browser → daemon (through nm-host)

| Method | Params |
|---|---|
| `browser.tabs.sync` | `{tabs: [{tabId, windowId, url, groupId?, active, windowFocused}]}` — full snapshot on connect and on any change to a localhost tab |
| `browser.activity` | `{tabId, url, at}` — heartbeat while §5 `tab` condition holds |

### Daemon → client notifications

`server.added`, `server.updated`,
`server.removed {id, reason: "idle"|"manual"|"exited"}` — `exited` means the server went
away without lull stopping it.

Each connection gets a daemon-assigned `connectionId` in the `hello` result. Several
browser profiles (or browsers) are several connections; `tabs[]` entries and `tab.*`
commands are keyed by `connectionId`, and `servers.open` goes to the most recently
focused browser connection.

Delivery rules:
- `subscribe` first returns a full snapshot with a sequence number `seq`; every later
  notification carries `seq + n`. A client that sees a gap re-subscribes.
- Daemon → browser commands carry an `id` and are acknowledged (`{ok}` or `{error}`);
  before acting, the extension re-checks that the tab's current URL still matches the
  server's origin, and refuses otherwise.
- Mutations (`stop`, `pin`, `setAutoStop`, `restart`, `forget`) are idempotent.
- When a browser connection closes, all its tabs and visibility reports are dropped at
  once (no stale activity); the reconnecting extension sends a fresh snapshot.
- `servers.open`: if a bound tab with the same URL exists, focus it; else open a new tab
  in the Dev group of the most recently focused window of the most recently focused
  browser connection; with no browser connected, `open(1)` with `open_with` from config
  (default: system browser). The result says whether a connected browser confirmed it.

### Daemon → browser commands (notifications the extension must act on)

`tab.open {url, focus: bool}`, `tab.close {tabIds}`, `tab.discard {tabIds}`,
`tab.reload {tabIds}`, `tab.focus {tabId}`.

## 9. Browser extension

- Host permissions: `http://` and `https://` for `localhost`, `127.0.0.1`, `[::1]`,
  `*.localhost`. Permissions: `tabs`, `tabGroups`, `idle`, `nativeMessaging`.
- On connect: `connectNative("io.github.rgrvlsk.lull")`, send `hello`, then a tab snapshot.
- Adoption: any tab whose origin matches a known server port is moved into the
  window's "Dev" group (created on demand; colour configurable). Tabs the user
  drags out of the group stay out (remember per tabId).
- Binding: a tab is bound to the server id it was matched to. After a stop it stays
  bound to the stopped-server record; a different server later listening on the same
  port (Vite reuses 5173) does **not** take it over until the tab loads again (user
  reload or navigation), at which point it is matched afresh.
- Activity heartbeat per §5: a static content script on the host-permission origins
  reports `visibilitychange` and ticks every 30 s while visible; the background checks
  `idle.queryState(60)` and forwards `browser.activity`. Chromium reports occluded
  windows as hidden; Firefox never treats its key window as occluded.
- Executes `tab.*` commands.
- Without the daemon running, the extension does nothing and shows a badge.
- Builds: one MV3 codebase, two manifests — Chromium (service worker) and Firefox
  (event page; minimum Firefox 139 for `tabGroups`). In both, the open native port keeps
  the background alive; a 30 s `alarms` tick reconnects the port if it was dropped.
  Zen: M3 starts with a spike on Zen 1.23 — if API-created groups do not render
  correctly or collide with Zen folders, Zen runs with grouping disabled (visibility,
  open, discard still work) until fixed.
  Verified facts: Chrome ≥105 keeps the service worker alive while a native port is
  open; Firefox's event page is likewise kept alive by an open native port (source:
  `ext-backgroundPage.js`, contrary to MDN); `tabs.group` ships in Firefox 138 and
  `tabGroups` in 139 (title/colour set via `tabGroups.update`); Zen hides native groups
  in its UI and builds "folders" on them — how API-created groups render in Zen is
  unverified. Browsers without tab-group APIs degrade to "no grouping".
- Native-messaging manifests are installed by `lull browsers install` into each
  detected browser's `NativeMessagingHosts` dir. Helium:
  `~/Library/Application Support/net.imput.helium/NativeMessagingHosts` (it also falls
  back to Chrome's dir). Firefox **and Zen**:
  `~/Library/Application Support/Mozilla/NativeMessagingHosts` (Zen does not read its
  own dir; Zen < 1.22 on macOS 27 never spawns hosts).
- `lull-nm-host` validates the caller it is launched with (Chromium passes the origin
  `chrome-extension://<id>/` as argv[1]; Firefox passes the manifest path and add-on id)
  against the allowed extension ids, caps messages at 1 MiB, and never logs tab URLs
  beyond origin + path.

## 10. Menu-bar app

- Not sandboxed (Developer ID distribution): a sandboxed app cannot reach the socket.
- `MenuBarExtra` list of **every live server** (agent- and user-started, pinned or
  not) grouped by project: name, framework icon, port, origin badge (agent / you),
  pin badge, idle countdown, CPU / RSS. Monitoring and actions work on all of them.
- **Recently stopped** section below the live list (§7): project, command, stop time
  and reason. Actions: Restart, Copy command, Reveal in Finder, Open terminal here,
  Forget.
- Row actions: Open, Stop, Pin/Unpin, Copy URL, Reveal in Finder, Show log.
- Settings: idle minutes, auto-stop scope, warning, browser for `open`, per-project
  rules, "Install browser extension…" helper, launch at login.
- Icon state: count of running servers; dot when something is `stopping-soon`.
- Wording: "lull" suggests a resumable pause, but lull **terminates** processes.
  Every surface (protocol, notification, row, log, CLI) says *stop* / *stopped* /
  *restart*, never *sleep*, *pause*, *wake* or *kill* (see GLOSSARY.md).

## 11. Agent integration

- Detection needs nothing from agents.
- Optional instruction line: "Open dev-server previews with `lull open <url>`;
  idle servers are stopped after 30 min — restart them if needed."
- v1 shows the agent and a short session id with a copy button. Mapping a session to a
  T3 thread title/link is post-v1 (T3 exports no thread id and has no public lookup),
  behind a `SessionResolver` trait so it needs no protocol change.

## 12. Repository layout

```
crates/core      discovery, activity, policy, stop (platform traits)
crates/cli       binary: daemon, CLI, nm-host
extension/       shared TS source, manifests/{chromium,firefox}.json
macos/           SwiftUI app (SwiftPM + Xcode project), embeds the binary
docs/            contract.md, adr/
GLOSSARY.md      domain vocabulary
```

## 13. Milestones

1. **M1** daemon + CLI (`ls`, `stop`, `pin`, `open`, `daemon`) with `traffic`
   activity; replaces the bash reaper.
2. **M2** nm-host + Chromium extension (Helium first), `tab` activity, grouping.
3. **M3** Firefox/Zen build.
4. **M4** menu-bar app.
5. **M5** packaging: Homebrew cask, signing/notarisation, CWS/AMO listings. (Before M4
   the daemon runs through the dev-only `scripts/dev-launchagent.sh`, ADR-0006.)

## 14. Identifiers and licence

- Reverse-DNS prefix `io.github.rgrvlsk.lull`: app bundle id `io.github.rgrvlsk.lull`,
  LaunchAgent label `io.github.rgrvlsk.lull.daemon`, native-messaging host
  `io.github.rgrvlsk.lull`, AMO add-on id `lull@rgrvlsk.github.io`.
- Licence: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`).

## 15. Open decisions

~~OPEN-1~~ (ADR-0001) · ~~OPEN-2~~ (ADR-0003) · ~~OPEN-3~~ (§9) · ~~OPEN-4~~ (post-v1,
§11) · ~~OPEN-5~~ (§14) · OPEN-6 extension distribution · ~~OPEN-7~~ (§14).
