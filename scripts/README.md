# scripts

Maintenance scripts, run by hand and never in CI (see `docs/plan.md`).

- `retire-prototype.sh` (WP1.6): boots out the bash `devserver-reaper` prototype
  (`local.devserver-reaper`) and removes its plist and state, for the M1 cutover. Run it
  once, then `brew services start lull`.

`dev-launchagent.sh` ran the daemon from a checkout before the Homebrew formula existed; it was
removed when the formula shipped (ADR-0008).

- `build-app.sh [release|debug]` (WP4.4): builds `macos/.build/Lull.app` and signs it ad
  hoc. Open it with `open macos/.build/Lull.app`; a debug build also takes
  `open --env LULL_PREVIEW=fixtures|worst|empty|not-running|mismatch` to show preview
  data without a daemon, and `--env LULL_PANEL_WINDOW=1` to show the panel in an ordinary
  window for UI automation that cannot click a status item.
