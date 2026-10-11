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
env markers identify agent-started processes with zero false positives, and that
CPU-delta is a poor activity signal: in its first night it stopped 9 servers,
but a vite + workerd pair stayed "active" from background CPU noise alone. The real signal — *is a human looking at it?* — lives in
the browser.

## 2. Goals / non-goals

Goals (v1)
- G1 Discover every dev server owned by the current user, label its origin
  (agent / user) and project.
- G2 Put idle **agent-started** servers to sleep after `idleMinutes` (default 30).
  User-started servers are shown but never auto-stopped unless the user opts in.
- G3 Keep dev-server tabs in one "Dev" tab group per browser window and use tab
  focus as the primary activity signal.
- G4 Menu-bar app to monitor, open, stop, pin servers and change policy.
- G5 A CLI agents can call (`lull open <url>`, `lull ls`).
- G6 Zero instructions required for correctness; at most one line in agent
  instruction files for convenience.

Non-goals (v1): Safari, Windows/Linux UI (core stays portable), restarting
("waking") stopped servers (see §9), remote/containers (Docker, VMs).

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
  `SMAppService.agent`; it works without the app (CLI-only installs via Homebrew).
- The app is a pure client of the socket; it owns no policy.

## 4. Discovery (daemon)

Scan every `scanIntervalSec` (default 5):

1. Listening TCP sockets of processes owned by the current uid (libproc on macOS;
   trait `ProcessSource` so Linux `/proc` can be added later).
2. Group by **process group** (pgid). A *server* = one pgid with ≥1 listener.
3. Read argv, env and cwd of the group leader (or lowest pid) via
   `KERN_PROCARGS2` / `proc_pidinfo`.
4. Exclude **protected** groups: any pgid that contains the daemon itself, an agent
   runtime (`claude`, `codex`), the T3 Code app or its direct children (e.g. its
   `cloudflared` tunnel), a browser, or `launchd`-managed services (parent = 1 *and*
   a matching launchd job / brew service). Excluded groups are never listed.
5. Classify:
   - `origin.kind = "agent"` if any member's env has `CLAUDECODE=1`,
     `T3CODE_CLI_PATH`, or a Codex marker (**OPEN-1**: confirm Codex's own env marker
     beyond T3's). `origin.agent ∈ {claude, codex, t3, unknown}`;
     `origin.sessionId` from `CLAUDE_CODE_SESSION_ID` when present.
   - else `origin.kind = "user"`.
   - `framework` by argv heuristics (vite, next, astro, nuxt, remix, wrangler,
     php-builtin, artisan, python-http, rails, django, generic).
   - `project` = basename of the git root containing `cwd`, else `cwd` basename.
6. `id` = stable string `"{pgid}-{leaderStartEpochMs}"` (survives restarts of the
   daemon, changes when the pgid is reused).

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
  "state": "running",                   // running | sleeping-soon | stopping
  "sleepsAt": "2026-10-11T20:42:00Z",   // null when autoStop=false or pinned
  "tabs": [{ "browser": "helium", "tabId": 812, "windowId": 3 }]
}
```

`primaryUrl`: the listener whose port is not an inspector/internal port (heuristic:
port mentioned in argv, else lowest port that answers HTTP `HEAD /` within 300 ms).

## 5. Activity model

A server is **active** at time *t* if any source fired within the last
`idleMinutes`:

| Source | Fires when | Notes |
|---|---|---|
| `tab` | one of the server's tabs is the active tab of a **focused** browser window and the OS user is not idle (`idle.queryState(60) == "active"`) | heartbeat every 30 s from the extension; primary signal |
| `traffic` | a **new** TCP connection to one of the server's ports from a process outside the group (5-tuple not seen in the previous scan) | catches page reloads, Playwright, curl; long-lived HMR websockets do not count |
| `started` | server first seen | grace period = `idleMinutes` |
| `manual` | user/agent ran `open`, `touch`, or un-pinned | |

CPU is reported for display only and is **not** an activity source.

**OPEN-2**: short-lived connections (agent `curl`, 50 ms) can fall between 5 s
scans. Options: (a) accept the miss — agents verify right after starting, inside
the grace period; (b) shorten scans for agent-origin servers; (c) NStat private
API. Proposed: (a).

## 6. Policy

Effective policy = first match of: per-server override (pin / autoStop toggle,
in-memory, lost on stop) → per-project rule in config → defaults.

```toml
# ~/Library/Application Support/lull/config.toml
idle_minutes = 30
auto_stop = "agent"          # "agent" | "all" | "none"
warn_before_minutes = 2      # sleeping-soon state + notification
close_tabs_on_stop = true

[[project]]
path = "~/dev/example-app"   # prefix match on cwd
idle_minutes = 90
# auto_stop = "none"
```

Opt-out per process: env `LULL_KEEP=1` → `pinned = true`.

## 7. Stopping

1. `state = "sleeping-soon"` at `sleepsAt - warn_before_minutes`; app may show a
   notification with *Keep alive* (→ `manual` activity).
2. At `sleepsAt`: `SIGTERM` to the pgid, `state = "stopping"`, non-blocking.
3. After 5 s, `SIGKILL` to whatever remains in the pgid.
4. Emit `server.stopped {id, reason}`; if `close_tabs_on_stop`, ask every
   connected browser to close that server's tabs.
5. Append a line to `~/Library/Logs/lull.log`.

Never signal a pgid that is protected (§4.4) or not owned by the current uid,
re-checked immediately before each signal.

## 8. IPC

Socket: `~/Library/Application Support/lull/daemon.sock`, mode `0600`.
Framing: one JSON-RPC 2.0 message per line (NDJSON). Every client sends `hello`
first.

### Client → daemon

| Method | Params | Result |
|---|---|---|
| `hello` | `{client: "app"\|"cli"\|"browser", version, browser?: {name, version, extensionVersion}}` | `{daemonVersion, protocol: 1}` |
| `servers.list` | `{}` | `Server[]` |
| `servers.stop` | `{id}` | `{}` |
| `servers.pin` | `{id, pinned}` | `Server` |
| `servers.touch` | `{id}` | `Server` (manual activity) |
| `servers.open` | `{id}` or `{url}` | `{opened: bool, browser?}` — routed to a connected browser, else `open(1)` with the configured browser |
| `config.get` / `config.set` | `{}` / `{patch}` | `Config` |
| `subscribe` | `{topics: ["servers","browsers"]}` | stream of notifications below |

### Browser → daemon (through nm-host)

| Method | Params |
|---|---|
| `browser.tabs.sync` | `{tabs: [{tabId, windowId, url, groupId?, active, windowFocused}]}` — full snapshot on connect and on any change to a localhost tab |
| `browser.activity` | `{tabId, url, at}` — heartbeat while §5 `tab` condition holds |

### Daemon → client notifications

`server.added`, `server.updated`, `server.removed {id, reason}`,
`server.stopped {id, reason: "idle"|"manual"|"exited"}`.

### Daemon → browser commands (notifications the extension must act on)

`tab.open {url, focus: bool}`, `tab.close {tabIds}`, `tab.focus {tabId}`.

## 9. Browser extension

- Host permissions: `http://localhost/*`, `http://127.0.0.1/*`, `http://[::1]/*`,
  `http://*.localhost/*`. Permissions: `tabs`, `tabGroups`, `idle`,
  `nativeMessaging`.
- On connect: `connectNative("<reverse-dns, OPEN-7>")`, send `hello`, then a tab snapshot.
- Adoption: any tab whose origin matches a known server port is moved into the
  window's "Dev" group (created on demand; colour configurable). Tabs the user
  drags out of the group stay out (remember per tabId).
- Activity heartbeat per §5.
- Executes `tab.*` commands.
- Without the daemon running, the extension does nothing and shows a badge.
- Builds: Chromium MV3 (service worker; an open native port keeps it alive) and
  Firefox MV3 (event page). **OPEN-3**: confirm Firefox/Zen `tabGroups` API
  coverage and Zen's native-messaging manifest directory; Arc lacks tab-group APIs
  (degrade to "no grouping").
- Native-messaging manifests are installed by `lull browsers install` into each
  detected browser's `NativeMessagingHosts` dir (Helium:
  `~/Library/Application Support/net.imput.helium/NativeMessagingHosts`).

## 10. Menu-bar app

- `MenuBarExtra` list grouped by project: name, framework icon, port, origin badge
  (agent / you), idle countdown, CPU / RSS.
- Row actions: Open, Stop, Pin/Unpin, Copy URL, Reveal in Finder, Show log.
- Settings: idle minutes, auto-stop scope, warning, browser for `open`, per-project
  rules, "Install browser extension…" helper, launch at login.
- Icon state: count of running servers; dot when something is `sleeping-soon`.
- Wording: "lull" suggests a resumable pause, but v1 **terminates** processes.
  Every surface (notification, row, log, CLI) must say *stopped*, never *paused*.

## 11. Agent integration

- Detection needs nothing from agents.
- Optional instruction line: "Open dev-server previews with `lull open <url>`;
  idle servers are stopped after 30 min — restart them if needed."
- **OPEN-4**: map `CLAUDE_CODE_SESSION_ID` → T3 thread title/link for display.

## 12. Repository layout

```
crates/core      discovery, activity, policy, stop (platform traits)
crates/cli       binary: daemon, CLI, nm-host
extension/       shared TS source, manifests/{chromium,firefox}.json
macos/           SwiftUI app (SwiftPM + Xcode project), embeds the binary
docs/            contract.md, adr/, glossary
```

## 13. Milestones

1. **M1** daemon + CLI (`ls`, `stop`, `pin`, `open`, `daemon`) with `traffic`
   activity; replaces the bash reaper.
2. **M2** nm-host + Chromium extension (Helium first), `tab` activity, grouping.
3. **M3** Firefox/Zen build.
4. **M4** menu-bar app.
5. **M5** packaging: Homebrew cask + formula, signing/notarisation, CWS/AMO listings.

## 14. Open decisions

OPEN-1 Codex env marker · OPEN-2 short-connection detection · OPEN-3 Firefox/Zen
tab groups & manifest paths · OPEN-4 T3 thread mapping · OPEN-5 licence
(MIT vs MIT/Apache-2.0) · OPEN-6 extension distribution (stores vs unpacked +
pinned key) · OPEN-7 bundle id / reverse-DNS (`<reverse-dns, OPEN-7>`).
