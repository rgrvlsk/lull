# v1 ships only as a Homebrew cask; the app registers the daemon with SMAppService

_Deferred to v1.1 by [ADR-0008](0008-v1-ships-as-formula.md): v1 ships as a formula; this
decision applies once the app exists. Read "v1" below as "v1.1"._

The first draft had two ways to run the daemon: the menu-bar app registering it with
`SMAppService.agent` (which only reads plists from the app bundle) and a CLI-only Homebrew
formula registering its own LaunchAgent. That allows two daemons and an app talking to a
daemon of a different version. v1 has a single artefact: a Homebrew cask installing
`Lull.app`, which embeds the `lull` binary, registers it with `SMAppService.agent`, and
links `lull` onto `PATH` through the cask's `binary` stanza. The daemon runs whether or not
the app is open; CLI-only users install the cask and hide the menu icon. A `flock` on
`~/Library/Application Support/lull/daemon.lock` still guarantees a single instance.

## Consequences

- A Homebrew formula and `cargo install` are deferred until after v1.
- Before the app exists (M1–M3) the daemon is run for development through an unsupported
  `scripts/dev-launchagent.sh` that bootstraps a plist in `~/Library/LaunchAgents`; it is
  removed when M4 lands and is not documented for users.
- The app must handle `SMAppService` status `requiresApproval` by sending the user to
  Login Items, and re-register after updates.
