# scripts

Maintenance scripts, run by hand and never in CI (see `docs/plan.md`).

- `retire-prototype.sh` (WP1.6): boots out the bash `devserver-reaper` prototype
  (`local.devserver-reaper`) and removes its plist and state, for the M1 cutover. Run it
  once, then `brew services start lull`.

`dev-launchagent.sh` ran the daemon from a checkout before the Homebrew formula existed; it was
removed when the formula shipped (ADR-0008).
