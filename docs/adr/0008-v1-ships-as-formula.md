# v1 is daemon + CLI + Chromium extension, shipped as a Homebrew formula

ADR-0006 made the cask the only artefact, so nothing could ship before the menu-bar app
(M4) and Developer ID signing and notarisation (M5) existed. Most of the value is in
M1 and M2: agent-started servers get stopped, and the ones a human is looking at do
not. The cask was chosen to avoid two daemons and an app talking to a daemon of a
different version, but there is no app in v1, and the `flock` on `daemon.lock` already
guarantees one daemon. So v1 is M1 + M2 (Chromium, Helium first), released as a
formula in `rgrvlsk/homebrew-tap` whose `service` block runs `lull daemon`
(`brew services start lull`). Firefox/Zen (M3), the app (M4) and the cask with
`SMAppService` (M5) become v1.1, and ADR-0006 applies from then on.

## Considered Options

- **Keep the cask-only v1.** One artefact from day one, but the first release waits on
  the least familiar work (S3: signed embedded binary + `SMAppService` + nm-host link)
  and a paid Developer ID.
- **M1 alone as v1.** Smallest release. Rejected because without the extension the
  only activity sources are traffic and the start-up grace: a page someone is reading
  in a browser that holds no connection open still gets its server stopped.

## Consequences

- v1 needs no Developer ID and no notarisation CI. The unpacked Chromium extension
  (ADR-0007) needs no signing either; Firefox's AMO signing moves to v1.1 with M3.
- v1 has no stopping-soon notification; stops are visible in `lull history` and the log.
- Moving from the formula to the cask in v1.1 must stop the `brew services` job before
  the app registers its agent. The cask's `conflicts_with formula:` and a caveat cover
  it; the lock keeps a second daemon from starting meanwhile.
- `scripts/dev-launchagent.sh` is removed when the formula ships, not when M4 lands.
