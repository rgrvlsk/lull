# scripts

Development and maintenance scripts, run by hand and never in CI (see `docs/plan.md`).
Both touch the user's real LaunchAgents.

- `dev-launchagent.sh install|uninstall` (WP1.6): builds `target/release/lull` and
  bootstraps/bootouts the dev LaunchAgent `io.github.rgrvlsk.lull.daemon` running
  `lull daemon`. Removed when the Homebrew formula ships (ADR-0008).
- `retire-prototype.sh` (WP1.6): boots out `local.devserver-reaper` and removes its
  plist and state, for the M1 cutover.
