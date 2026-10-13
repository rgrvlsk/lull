# Ponytail debt

Deliberate shortcuts, each marked in the code with a `ponytail:` comment. A row leaves this
ledger when its trigger fires and the shortcut is upgraded. Maintenance rules: [`AGENTS.md`](AGENTS.md).

## crates/core

- `crates/core/src/activity/mod.rs:180`, a client's ancestry comes only from connections
  open at a scan. ceiling: bytes from a covered browser's connection that opened and closed
  between scans count as uncovered traffic. upgrade: read the client's ancestry from the
  process table if hidden tabs of covered browsers keep servers alive in practice.
- `crates/core/src/activity/mod.rs:251`, traffic is matched to a server by port alone.
  ceiling: two groups on the same port at different addresses share activity. upgrade: carry
  the local address in `TrafficEvent`/`Connection` if that happens.
- `crates/core/src/platform/macos/signal.rs:86`, one parked thread reaps each restart.
  ceiling: one thread per restart. upgrade: a single SIGCHLD/`waitpid(-1)` loop if restarts
  become frequent.
- `crates/core/src/platform/macos/ntstat.rs:83`, fixed request burst (16), counter queries
  per sample (256) and source cap (16 384). ceiling: overflow falls back to the scan backend. upgrade: tune from measured
  workloads before supporting high-volume hosts.
- `crates/core/src/platform/macos/ntstat.rs:473`, a non-loopback client counts as outside the
  group. ceiling: a same-machine client via the LAN address counts as traffic. upgrade: pair
  the reversed tuple for local non-loopback addresses.
- `crates/core/src/discovery/mod.rs:53`, ports and `primaryUrl` are kept from the previous
  scan for an unreadable group. ceiling: stale until the group is readable again. upgrade:
  rebuild them from `snap.listeners` if unreadable groups persist.
- `crates/core/src/discovery/labels.rs:13`, a retitled argv is split on whitespace.
  ceiling: an argument containing spaces loses its quoting. upgrade: resolve the script and
  re-read the real argv if that matters.
- `crates/core/src/discovery/labels.rs:148`, primary port found by flag shape. ceiling: an
  exotic flag naming another listening port first wins. upgrade: per-framework port flags.
- `crates/core/src/lifecycle/mod.rs:615`, the fresh observation before a stop is the
  preceding scan plus a member re-read. ceiling: up to one scan interval (5 s) between
  classification and signal. upgrade: re-run discovery on the group right before signalling.
- `crates/core/src/browser/mod.rs:71`, a hidden tab reloaded on the same URL keeps its old
  binding. ceiling: rebound only by its next visible heartbeat. upgrade: a load counter in
  `tabs.sync`.

## crates/cli

- `crates/cli/src/daemon.rs:193`, no per-line size cap on the daemon socket. ceiling: a
  misbehaving same-uid client can send huge lines (nm-host caps browser messages at 1 MiB).
  upgrade: read through `Read::take` per line if a client misbehaves.
- `crates/cli/src/nm_host.rs:46`, bridge workers end with the host process. ceiling: only
  correct as a standalone process. upgrade: interruptible fd reads and joins if embedded.

## macos

- `macos/Sources/LullKit/Store.swift:144`, history is re-fetched on `server.added`/`removed`
  and when the panel opens; there is no history notification. ceiling: a record another
  client forgot or restarted stays listed until then. upgrade: a `history.changed`
  notification if stale records confuse anyone.

## extension

- `extension/esbuild.config.mjs:4`, builds Chromium only. ceiling: no Firefox/Zen. upgrade:
  the Firefox event-page manifest and build in WP3 (M3).
- `extension/src/background.ts:53`, dragged-out tabs are remembered in memory only. ceiling:
  forgotten when the service worker restarts. upgrade: the `storage` permission
  (`chrome.storage.session`) if users report dragged-out tabs being regrouped.

15 markers, 0 with no trigger.
