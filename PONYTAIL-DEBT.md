# Ponytail debt

Deliberate shortcuts, each marked in the code with a `ponytail:` comment. A row leaves this
ledger when its trigger fires and the shortcut is upgraded. Maintenance rules: [`AGENTS.md`](AGENTS.md).

## crates/core

- `crates/core/src/model.rs:241`, the pin is a bare bool. ceiling: a `LULL_KEEP=1` pin is
  indistinguishable from a manual one, on the record and on the wire. upgrade: a pin-source
  field when the menu-bar app (M4) must show "pinned (env)".
- `crates/core/src/activity/mod.rs:168`, traffic is matched to a server by port alone.
  ceiling: two groups on the same port at different addresses share activity. upgrade: carry
  the local address in `TrafficEvent`/`Connection` if that happens.
- `crates/core/src/platform/macos/signal.rs:86`, one parked thread reaps each restart.
  ceiling: one thread per restart. upgrade: a single SIGCHLD/`waitpid(-1)` loop if restarts
  become frequent.
- `crates/core/src/platform/macos/ntstat.rs:64`, fixed request burst (16) and source cap
  (16 384). ceiling: overflow falls back to the scan backend. upgrade: tune from measured
  workloads before supporting high-volume hosts.
- `crates/core/src/platform/macos/ntstat.rs:339`, a non-loopback client counts as outside the
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
- `crates/core/src/lifecycle/mod.rs:426`, no `server.removed` for the old id on a turnover
  abort. ceiling: subscribers keep a ghost entry until they re-subscribe. upgrade: when the
  menu-bar app (M4) subscribes.
- `crates/core/src/lifecycle/mod.rs:611`, the fresh observation before a stop is the
  preceding scan plus a member re-read. ceiling: up to one scan interval (5 s) between
  classification and signal. upgrade: re-run discovery on the group right before signalling.
- `crates/core/src/ipc/mod.rs:835`, `lull doctor` reads `status.json`, not a protocol method.
  ceiling: an internal file format. upgrade: a `daemon.status` method when the menu-bar app
  (M4) needs the same data.
- `crates/core/src/browser/mod.rs:71`, a hidden tab reloaded on the same URL keeps its old
  binding. ceiling: rebound only by its next visible heartbeat. upgrade: a load counter in
  `tabs.sync`.

## crates/cli

- `crates/cli/src/daemon.rs:185`, no per-line size cap on the daemon socket. ceiling: a
  misbehaving same-uid client can send huge lines (nm-host caps browser messages at 1 MiB).
  upgrade: read through `Read::take` per line if a client misbehaves.
- `crates/cli/src/nm_host.rs:46`, bridge workers end with the host process. ceiling: only
  correct as a standalone process. upgrade: interruptible fd reads and joins if embedded.

## extension

- `extension/esbuild.config.mjs:4`, builds Chromium only. ceiling: no Firefox/Zen. upgrade:
  the Firefox event-page manifest and build in WP3 (M3).
- `extension/src/background.ts:53`, dragged-out tabs are remembered in memory only. ceiling:
  forgotten when the service worker restarts. upgrade: the `storage` permission
  (`chrome.storage.session`) if users report dragged-out tabs being regrouped.

16 markers, 0 with no trigger.
