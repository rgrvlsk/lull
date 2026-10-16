# The browser extension is optional; a stale one is turned away, not accommodated

ADR-0008 shipped the Chromium extension in v1 because tab visibility was the best activity
signal, and contract §5 called it the primary one. lull must be useful on a machine where
the extension was never installed, is disabled, or lags behind the daemon, and ADR-0010
already judges traffic without it: pulses from a client outside the group fire `traffic`,
periodic polling does not. So the extension is a nice-to-have that sharpens activity, not
a requirement. From 2026-10-13 it is frozen as it is and gets only the changes a protocol
bump forces on it.

Protocol 1.3 adds fields (`Server.pinSource`) that the 1.2 extension's wire guards reject
as unknown, so the daemon refuses a browser `hello` with minor < 3 using
`protocol_mismatch`. The old extension already shows the "updt" badge for that error, and
its browser stays uncovered, which is exactly the no-extension case. From 1.3 on the
extension ignores unknown fields (its enums stay closed), so later additive minors need no
new gate.

## Considered Options

- **Per-client feature gating in the daemon** (strip new fields for old browsers). Keeps a
  stale extension working, but adds a code path per field and per version to a component
  whose absence is already a supported state.
- **Major bump to 2.0.** Turns away every old client, CLI and app included, for an additive
  change.

## Consequences

- An extension older than 1.3 shows "updt" and does nothing until `brew upgrade` and a
  browser restart (ADR-0007). Servers in that browser are judged by `traffic` alone, so, as
  ADR-0010 says, a page read for longer than the idle time without any request can have its
  server stopped; a pin or `LULL_KEEP=1` keeps it.
- Nothing in the daemon, the CLI, `lull ui` or the app may require a connected browser.
  Features that need the extension (tab grouping, `tab` activity, `tabs_on_stop`,
  `servers.open` routing) degrade to their no-browser behaviour.
- The app adds no "install extension" helper; `lull browsers install` stays the way in.
