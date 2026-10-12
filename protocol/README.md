# Protocol fixtures

Golden JSON-RPC 2.0 exchanges, one NDJSON file per method, notification or browser
command of `docs/contract.md` §8. Every line is one complete message exactly as it travels
over the daemon socket; a file reads top to bottom as one conversation. Request ids that
are strings (`"d1"`) belong to daemon → browser commands, numeric ids to client → daemon
calls.

The Rust (`crates/protocol`), TypeScript (`extension/`) and Swift (`macos/`) test suites
all decode every file, so the three implementations cannot drift. A protocol change edits
the Rust types, these fixtures and contract §8 in one commit and bumps `protocol.minor`
(or `major` when it breaks compatibility).

Shapes the contract leaves implicit, pinned here (confirm or amend in `contract.md`):

- Notifications carry the subscription `seq` inside `params` next to the payload
  (`{seq, server}` / `{seq, id, reason}`); the `subscribe` result is `{seq, servers}`.
  The `browsers` topic has no payload yet.
- Browser command acks are `{"ok": true}` or `{"error": "<reason>"}`.
- `config.set` validation errors and rejected `servers.open` URLs use JSON-RPC
  `-32602` (invalid params); only `-32010 protocol_mismatch` is lull-specific.
- `Config` keeps its TOML `snake_case` keys on the wire.
- `[[agent_marker]]` entries are `{env, value?, agent}`.
