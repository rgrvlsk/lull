# lull — v1 contract

Status: **accepted for v1** (2026-10-12, after review and grilling; revised the same day
after a three-way scope and safety review: v1 narrowed to daemon + CLI + Chromium
extension, ADR-0008). This file is the
single source of truth that the daemon, the browser extension and the menu-bar app are
built against. Decisions and their reasons live in `docs/adr/`; vocabulary in
`GLOSSARY.md`. Any change of behaviour updates this file in the same commit.

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
- G3 Keep dev-server tabs in one "Dev" tab group per window of a Chromium browser and
  use tab visibility as the primary activity signal.
- G5 A CLI agents can call (`lull open <url>`, `lull ls`).
- G6 Zero instructions required for correctness; at most one line in agent
  instruction files for convenience.

v1.1 (specified here so v1 does not paint them into a corner, built after v1 ships;
ADR-0008):
- G7 Terminal UI (`lull ui`) to watch and manage servers live (§3, ADR-0009).
- G4 Menu-bar app to monitor, open, stop, pin servers and change policy (§10).
- G3 for Firefox and Zen (§9).
- Cask distribution with `SMAppService` and signed, notarised artefacts (ADR-0006).

Non-goals (v1): Safari, Windows/Linux UI (core stays portable), *automatic* waking
(restarting a stopped server when its port is requested) — manual restart from the
recently-stopped list is in scope (§7) — remote/containers (Docker, VMs).

## 3. Components

```
 Swift menu-bar app ──┐                       ┌── lull CLI (same binary)
                      ▼                       ▼
            ┌──────── lull daemon (Rust) ─────────┐
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
- The daemon runs as a per-user LaunchAgent. v1 ships as a Homebrew formula whose
  `service` block runs `lull daemon` (`brew services start lull`, ADR-0008). From v1.1
  the cask's app registers it via `SMAppService.agent` instead (ADR-0006). Either way:
  single instance via `flock` on `~/Library/Application Support/lull/daemon.lock`.
- The daemon exits cleanly when its own executable path no longer exists (checked every
  scan), so after an upgrade launchd restarts it from the new version.
- The app (v1.1) is a pure client of the socket; it owns no policy.
- `lull ui`, and bare `lull` when stdin and stdout are terminals, is a terminal UI
  (v1.1, ADR-0009): a pure client of the socket that owns no policy, like the app.
  Process-supplied text (argv, cwd, project, URLs) is drawn with control characters
  replaced. Stopping a user-started server and restarting any server ask first.

## 4. Discovery (daemon)

Scan every `scanIntervalSec` (default 5):

1. Listening TCP sockets of processes owned by the current uid (on macOS libproc lists
   the uid's processes and reads only their socket fds — `proc_listpids`,
   `PROC_PIDLISTFDS`, `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` — which also yields both ends
   of local connections with their pids; trait `ProcessSource` so Linux `/proc` can be
   added later). One scan must stay under 50 ms on the author's machine (measured
   2026-10-12: 3 ms for 751 processes). `net.inet.tcp.pcblist_n` is not usable: since
   macOS 27 it returns only the caller's own sockets unless the caller is an Apple
   platform binary.
2. Group by **process group** (pgid). Read argv, env and cwd of every member via
   `KERN_PROCARGS2` / `proc_pidinfo`. argv and initial env never change for a running
   process, so they are cached per (pid, start time).
3. A *server* = one pgid with ≥1 listener where some listening member's cwd is a
   **project location** — strictly inside `$HOME` but neither `$HOME` itself nor under
   `~/Library`, or under `/tmp`, `/private/tmp`, `/private/var/folders` — and no member's
   executable is under `/Applications`, `~/Applications` or `/System` (ADR-0002). Every
   other listening group is an **unmanaged listener**: shown only by `lull ls --all`,
   never stopped, no activity tracking.
4. Exclude **protected** groups: any pgid that contains the daemon itself, an agent
   runtime, the T3 Code app or its direct children (e.g. its `cloudflared` tunnel), or
   a browser. Protected groups are never listed. Membership is decided by executable
   path and argv, never by process name alone (an npm-installed Claude Code runs as
   `node`):
   - agent runtime: executable basename `claude` or `codex`, or a `node`/`bun` process
     whose argv names a script inside `@anthropic-ai/claude-code` or `@openai/codex`;
   - T3 Code: executable inside the T3 Code app bundle, or a direct child of it;
   - browser: executable inside the app bundle of a browser from the
     `lull browsers install` table (§9), wherever the bundle is installed. Other
     installed apps are not protected; they are unmanaged listeners (step 3).
   The pattern list lives in one table in `crates/core` and every entry has a snapshot
   fixture.
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
8. Incomplete observation: if the group's member list, or any member's env, cwd,
   executable path, controlling tty or sockets cannot be read in a scan (process
   exiting, permission), the server keeps its previous classification and is **not
   eligible for an automatic stop** in that scan. Unknown is never treated as safe. A server is removed only when
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

`primaryUrl`: `http://127.0.0.1:<port>/` for the first of: a listening port mentioned
in argv or in the `PORT` env of a member, else the lowest listening port that is not an
inspector port (9229–9239). It is recomputed when the listener set changes. lull never
sends requests to a server: an HTTP probe would make Next, Nuxt or Astro compile routes
just to be labelled. Connections opened by the daemon itself never count as activity (§5).

## 5. Activity model

A server is **active** at time *t* if any source fired within the last
`idleMinutes`:

| Source | Fires when | Notes |
|---|---|---|
| `tab` | one of the server's tabs is **visible** (`document.visibilityState == "visible"`, reported by a content script) and the OS user is present (`idle.queryState(60) == "active"`), whether or not its window has focus | heartbeat every 30 s while the condition holds; primary signal. Focus is used only for display and `open` routing. A visible window on a second monitor counts — that is intended |
| `traffic` | a **pulse** from a process outside the group that is not **periodic traffic** (below), made of (a) a **new** TCP connection to one of the server's ports (ntstat event; fallback: 5-tuple not seen in the previous scan), or (b) bytes moving on an **established** connection to one of the server's ports from a client that is not a *covered browser* | (a) catches new page loads, Playwright, curl. (b) catches requests that reuse a keep-alive connection and HMR updates pushed to a browser without the extension; a connection held open with nothing moving on it is not activity. Connections from covered browsers count only through (a); their open tabs are judged by `tab` |
| `started` | server first seen | grace period = `idleMinutes` |
| `manual` | user/agent ran `open`, `touch`, or un-pinned | |

CPU is reported for display only and is **not** an activity source.

**Covered browser:** a browser process with a live extension connection, plus all its
descendants (Chromium opens connections from a network-service helper, not the main
process). The daemon learns it per connection: `LOCAL_PEERPID` on the socket gives the
nm-host's pid, whose parent is the browser. A connected browser covers only itself:
another browser without the extension stays uncovered, and traffic on its established
connections keeps servers alive through `traffic` (b). Coverage is per browser process, and a Chromium
browser runs all its profiles in one process, so connecting the extension in one profile
covers every profile of that browser: a page open only in a profile without the extension
counts through new connections (`traffic` (a)) alone. The extension belongs in every
profile, and `lull doctor` says so for a browser with several profiles. With no browser
connected (all of M1), traffic on every established outside connection counts.

**Pulses and periodic traffic** (ADR-0010). Pages poll their dev server on a timer (HMR
pings, status checks), and a hidden tab keeps polling; a health check or uptime monitor
does the same. That repetition is a machine, not a person using the server, so it must
not keep the server alive, whether or not the extension is installed. Traffic is read
per **client key**: one server and one client pid (all remote or unknown clients share
one key), in one-second slots of the daemon's clock.

- A slot is *busy* when the key opened a connection to one of the server's ports in it,
  or moved bytes (received plus sent, summed over its connections to those ports).
- A **pulse** is a run of at most two consecutive busy slots (a response that crosses a
  slot boundary stays one pulse; a third busy slot starts the next pulse). Its instant is
  its first slot; its size is the bytes moved, or *unknown* when a connection in it
  closed before its counters were read.
- A **train** is a sequence of a key's pulses at a steady period *P* (3 s ≤ *P* ≤ 10 min)
  with matching sizes (two sizes match when they differ by at most 10 % of the larger or
  32 bytes, whichever is more). A pulse continues a train when it lands within
  max(1 s, *P*/10) of the train's next expected instant, or of one of the two after it,
  and its size matches. A pulse that
  continues no train starts a candidate train with each of the key's pulses from the
  last 10 minutes that continued no confirmed train. A train is **confirmed** once it
  holds four pulses, and dropped when three expected instants pass without one. A key
  keeps at most 16 trains; past that, the train whose last pulse is oldest is dropped.
  Pulses of unknown size never join a train.
- A pulse that brings a train to four pulses or continues a confirmed train is
  **periodic traffic** and fires nothing. Every other pulse fires `traffic` at its
  instant, under (a) or (b) as its connections allow.

Every pulse of a key feeds the trains, including those that cannot fire by themselves
(bytes on a covered browser's established connection), so a poll that sometimes reuses
a connection and sometimes opens one is learned as one train. Consequences: a poller
fires `traffic` for its first three pulses only, and again for three pulses after its
period or size changes (Chromium throttles a hidden tab's timers to once a minute after
five minutes); several hidden tabs of one browser, which share one network process,
form separate trains; a page load, a reload, an agent's request and an HMR update after
a save each land off the rhythm or with another size, and fire. A client that varies the
size or timing of each request is not recognised and keeps the server alive, which is
the safe way to fail.

Clock: idle time counts **awake time only**. Activity instants are kept as
`CLOCK_UPTIME_RAW` (stops while the Mac sleeps) plus `kern.bootsessionuuid`; persisted
entries store both, so idle time survives daemon restarts within one boot and wall-clock
changes never move deadlines. `lastActivityAt` (wall clock) is for display only. After a
wake (detected when wall time advances much more than uptime between two scans) and
after daemon start, no server is stopped for `wake_grace_minutes` (default 5) so browsers
can reconnect and report visible tabs. Activity timestamps are the daemon's receipt
time, never the sender's.

`traffic` is observed through the kernel network-statistics control socket
(`com.apple.network.statistics`, ABI in xnu `bsd/net/ntstat.h`; ADR-0003): the daemon
subscribes to TCP source add events and fires `traffic` when a connection's local port
belongs to a server and its client end is owned by a pid outside the group and other than
the daemon. This catches 50 ms agent `curl`s that a periodic scan misses. Bytes come
from the same socket: four times a second, on a fixed grid, the daemon queries the
counters (`QUERY_SRC` → `SRC_COUNTS`) of each connection to a server's port whose client is
outside the server's group, one source at a time, never every source on the machine. Bytes
are stamped with the grid instant of the reading that saw them, so their timing error stays
under 250 ms, well inside a train's 1 s tolerance. Connections that existed before the
daemon subscribed are read too; their first reading is a baseline, not traffic. If the control
socket cannot be opened (`net.statistics_privcheck=1`) or reports an unknown message
version, the daemon falls back to diffing the group's accepted sockets every scan and
reports `activityBackend: "scan"` in `hello`/`lull doctor`. The scan fallback only sees
connections that are open at a scan instant, so it misses most requests shorter than
`scanIntervalSec`; `traffic` (b) and `tab` carry the load in that mode. The fallback has
no byte counters, so it cannot tell a quiet connection from a busy one: in that mode (b)
fires at every scan for every established connection from a client outside the group
that is not a covered browser, and periodic traffic is not filtered. When lull cannot
see traffic, it keeps servers alive rather than stop one that may be in use. A backend that
fails while running (socket closed, read errors) reports itself unhealthy; the daemon
switches to the scan fallback, and no server is stopped until one full scan interval
has run on the new backend.

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
wake_grace_minutes = 5       # no stops right after wake or daemon start (§5)
tabs_on_stop = "discard"     # "discard" | "close" | "keep"
open_with = ""               # app for open(1) when no browser is connected; "" = system default
history_size = 20
history_hours = 24

[[project]]
path = "~/dev/example-app"   # `~` expanded; prefix match on whole path components of cwd
idle_minutes = 90
# auto_stop = "none"
```

Opt-out per process: env `LULL_KEEP=1` → the server starts pinned (shown as *pinned
(env)*). It is only the initial value: an explicit Unpin by the user wins and is
persisted. `DEVSERVER_KEEP` from the prototype is **not** honoured.

`lull daemon --dry-run` makes every decision, logs and shows it, but sends no signals
(for testing and fixtures; off by default).

## 7. Stopping

A stop always targets the **whole process group**: every member, including ones that
do not listen (a worker or watcher the agent started in the same command), ends with
the server. That is the meaning of *server* in the glossary, not a side effect.

Stop state machine: `running → stopping-soon → stopping → (removed)`. Any activity,
pin, `autoStop` change or policy change that makes the server ineligible returns
`stopping-soon → running`; once `SIGTERM` is sent the stop cannot be cancelled. A manual
`servers.stop` skips `stopping-soon`; concurrent stops of the same id are coalesced.

1. `state = "stopping-soon"` at `stopsAt - warn_before_minutes`; the app, if running,
   shows a notification with *Keep alive* (→ `manual` activity). The warning is a
   courtesy: with no app connected the stop proceeds on time and is logged and listed
   under *Recently stopped*.
2. At `stopsAt`, an automatic stop first takes a fresh observation of the group and
   proceeds only if it is complete (§4.8) and the server is still eligible (origin,
   policy, pin, no activity, not in wake grace). Then `SIGTERM` to the pgid,
   `state = "stopping"`, non-blocking.
3. After 5 s, `SIGKILL` to whatever remains in the pgid. If the witness check below
   fails here — every member recorded at decision time has exited but the group still
   has members — the stop is **aborted**: no `SIGKILL`, no `server.removed`, no tab
   action, no stopped-server record. The server returns to `running` with `manual`
   activity, and the abort is logged and shown by `lull doctor`.
4. Only once the pgid has no members left: emit `server.removed {id, reason}` and apply
   `tabs_on_stop` to the server's tabs in
   every connected browser: `discard` (default — `tabs.discard`: the tab keeps its URL
   and place in the Dev group but frees its memory; the active tab cannot be discarded
   and is left as is), `close`, or `keep`. If members survive 5 s after `SIGKILL`
   (uninterruptible sleep), the server stays in `stopping`, is logged and shown by
   `lull doctor`, and is removed when the group finally empties.
5. Append a line to `~/Library/Logs/lull.log`.
6. Record a **stopped-server record** in `state.json` history (last `history_size` = 20
   entries, none older than `history_hours` = 24): `{id, project, framework, command,
   argv, cwd, ports, origin, stoppedAt, reason, restartEnv, tabs, status}` with
   `status ∈ {stopped, restarting, restartFailed}`. `command` is the shell-quoted
   launch command (below), or the oldest member's command line when there is none; it
   is what Copy command copies. `argv` is the **launch command**, captured when
   the server was first seen: walk up from a listening member through parents that are
   still in the group; take the topmost, and if it is a shell run with `-c` (an agent
   harness wrapper, or `sh -c 'migrate; dev'`), take its child on that path instead
   (e.g. `npm run dev`). If no launch command can be found, `argv` is empty and the
   record cannot be restarted, only copied. `restartEnv` keeps only `PORT`, `HOST`,
   `NODE_ENV` when present. Servers that exit on their own (`reason: "exited"`) are
   recorded too.

### Restart (ADR-0005)

`servers.restart {id}` / `lull restart <id>` on a stopped-server record:

- Spawns `$SHELL -l -c 'exec "$@"' lull-restart <argv…>` in `cwd`, in a new session
  (`setsid`, so a new pgid outside the daemon's), stdin `/dev/null`, stdout+stderr to
  `~/Library/Logs/lull/restart-<id>.log`. The environment is the user's login-shell
  environment plus `restartEnv` — never the original environment, so secrets are not
  persisted and agent markers are dropped. (The opt-in env-preserving restart of
  ADR-0005 is deferred past v1.)
- A record with an empty `argv` is refused with error `-32011 not_restartable`; clients
  offer Copy command and Open terminal instead.
- The restarted process has no agent marker and no tty, so it is classified
  `origin.kind = "user"` and not auto-stopped by default; it gets a new id. The record is
  marked `restarting` (a second restart of the same record is refused) and removed from
  history only when a new server with a listener is discovered in `cwd` within 60 s; its
  `tabs` (still bound to the record, §9) are then reloaded. On timeout or early exit
  the record stays, marked `restartFailed`, with a link to the restart log. The
  confirmation shows the exact command that will run. If the port is taken, the
  server's own behaviour applies (fail or pick another port); the log shows it.

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
| `servers.restart` | `{id}` (stopped-server record) | `{pid}` — a record without a launch command is refused with error `-32011 not_restartable` (§7) |
| `servers.forget` | `{id}` | `{}` — drop a stopped-server record |
| `servers.open` | `{id, path?, focus?}` or `{url, focus?}` | `{opened: bool, browser?}` — routed to a connected browser, else `open(1)` with the configured browser. `url` must be `http(s)` with a loopback or `*.localhost` host; anything else is rejected (never passed to `open(1)`). `focus` (protocol 1.1) overrides the default: focused, unless the caller's process carries an agent marker (§11; the daemon reads the caller's environment via `LOCAL_PEERPID`) |
| `config.get` / `config.set` | `{}` / `{patch}` | `Config` |
| `subscribe` | `{topics: ["servers","browsers"]}` | stream of notifications below |

### Browser → daemon (through nm-host)

| Method | Params |
|---|---|
| `browser.tabs.sync` | `{tabs: [{tabId, windowId, url, groupId?, active, windowFocused}]}` — full snapshot on connect and on any change to a localhost tab |
| `browser.activity` | `{tabId, url}` — heartbeat while §5 `tab` condition holds; timestamped by the daemon on receipt |

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

`tab.open {url, focus: bool}`, `tab.close {tabIds, ports?}`, `tab.discard {tabIds, ports?}`,
`tab.reload {tabIds, ports?}`, `tab.focus {tabId, ports?}`. `ports` (protocol 1.2) are the
ports of the server the command is about: the extension acts on a tab only while its
current URL is loopback on one of them, so a tab that navigated elsewhere after the command
was sent is left alone and the ack is an error.

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
- Builds: one MV3 codebase, two manifests — Chromium (service worker, v1) and Firefox
  (event page, v1.1; minimum Firefox 139 for `tabGroups`). In both, the open native port keeps
  the background alive; a 30 s `alarms` tick reconnects the port if it was dropped.
  Zen (v1.1): M3 starts with a spike on Zen 1.23 — if API-created groups do not render
  correctly or collide with Zen folders, Zen runs with grouping disabled (visibility,
  open, discard still work) until fixed.
  Verified facts: Chrome ≥105 keeps the service worker alive while a native port is
  open; Firefox's event page is likewise kept alive by an open native port (source:
  `ext-backgroundPage.js`, contrary to MDN); `tabs.group` ships in Firefox 138 and
  `tabGroups` in 139 (title/colour set via `tabGroups.update`); Zen hides native groups
  in its UI and builds "folders" on them — how API-created groups render in Zen is
  unverified. Browsers without tab-group APIs degrade to "no grouping".
- Distribution (ADR-0007): not on the stores. Chromium: unpacked from
  `~/Library/Application Support/lull/extension/chromium`, id pinned by the manifest
  `key`. Firefox/Zen: unlisted AMO-signed XPI, id `lull@rgrvlsk.github.io`, auto-updates
  via `gecko.update_url` on GitHub releases. `lull browsers install` places the files,
  opens the browser's extensions page or the XPI, and writes the NM manifest.
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

## 10. Menu-bar app (v1.1)

Until the app exists, v1 has no stopping-soon notification; stops are listed by
`lull history` and logged (§7).

- Minimum macOS 14 Sonoma (whole product).
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
- `lull open <url>`: when the CLI's own environment carries an agent marker, the tab
  opens in the background (in the Dev group, no focus change); run by a human it is
  focused. `--focus` / `--no-focus` override, sent as `servers.open {focus}`. Either way
  it counts as `manual` activity.
- Optional instruction line: "Open dev-server previews with `lull open <url>`;
  idle servers are stopped after 30 min — restart them if needed."
- v1 shows the agent and a short session id with a copy button. Mapping a session to a
  T3 thread title/link is post-v1 (T3 exports no thread id and has no public lookup),
  behind a `SessionResolver` trait so it needs no protocol change.

## 12. Repository layout

```
crates/protocol  wire types (serde) — the one definition of §4/§7/§8 shapes
crates/core      discovery, activity, policy, stop (platform traits + fakes)
crates/cli       binary: daemon, CLI, nm-host
protocol/        golden NDJSON fixtures every client decodes in tests
extension/       shared TS source, manifests/{chromium,firefox}.json
macos/           SwiftUI app (SwiftPM + Xcode project), embeds the binary
docs/            contract.md, adr/, plan.md
GLOSSARY.md      domain vocabulary
```

## 13. Milestones

v1:
1. **M1** daemon + CLI (`ls`, `stop`, `pin`, `open`, `restart`, `doctor`, `daemon`)
   with `traffic` activity, including established connections (§5); replaces the bash
   reaper by direct cutover: `scripts/retire-prototype.sh` boots out
   `local.devserver-reaper` and removes its plist and state, then the dev
   LaunchAgent for lull is loaded. No `DEVSERVER_KEEP` alias.
2. **M2** nm-host + Chromium extension (Helium first), `tab` activity, covered
   browsers, grouping.
3. **v1 release** Homebrew formula in `rgrvlsk/homebrew-tap` with a `service` block,
   shipping the unpacked Chromium extension (ADR-0007, ADR-0008).

v1.1:
4. **WP-T** terminal UI (`lull ui`, ADR-0009), first.
5. **M3** Firefox/Zen build, unlisted AMO signing with self-hosted updates.
6. **M4** menu-bar app.
7. **M5** cask packaging: app embedding the binary, `SMAppService`, Developer ID
   signing and notarisation (ADR-0006); the formula is retired in favour of the cask.

Until the formula exists the daemon runs through the dev-only
`scripts/dev-launchagent.sh`.

## 14. Identifiers and licence

- Reverse-DNS prefix `io.github.rgrvlsk.lull`: app bundle id `io.github.rgrvlsk.lull`,
  LaunchAgent label `io.github.rgrvlsk.lull.daemon` (dev script and v1.1 app; under the
  v1 formula `brew services` names it `homebrew.mxcl.lull`), native-messaging host
  `io.github.rgrvlsk.lull`, AMO add-on id `lull@rgrvlsk.github.io`.
- Licence: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`).
- Distribution: v1 is a formula in the tap `rgrvlsk/homebrew-tap`
  (`brew install rgrvlsk/tap/lull`, then `brew services start lull`); Homebrew builds
  or bottles the binary, so v1 needs no Developer ID. v1.1 replaces it with the cask
  `brew install --cask rgrvlsk/tap/lull`.
- Signing (v1.1): app, embedded `lull` binary and the `lull-nm-host` link target are
  signed with Developer ID + hardened runtime and notarised in CI (certificate and
  notary key are GitHub Actions secrets).
- Privacy: lull makes no network requests at all — no probes, no telemetry, no update
  checks of its own (Homebrew and the AMO `update_url` handle updates).

## 15. Open decisions

~~OPEN-1~~ (ADR-0001) · ~~OPEN-2~~ (ADR-0003) · ~~OPEN-3~~ (§9) · ~~OPEN-4~~ (post-v1,
§11) · ~~OPEN-5~~ (§14) · ~~OPEN-6~~ (ADR-0007) · ~~OPEN-7~~ (§14) · ~~OPEN-8~~ v1
scope (ADR-0008).
