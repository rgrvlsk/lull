# A server is a listening process group whose cwd is a project location

Listing "every TCP listener of the current uid" (the first draft of G1) surfaces
ControlCenter, rapportd, Herd's nginx, Homebrew redis/postgres/mariadb, mail clients,
editors and the agent host app itself. We define a server positively instead: a process
group with a listener where a listening member's cwd is strictly inside `$HOME` (not
`$HOME` itself, not `~/Library`) or in a temp directory, and no member runs from an
installed app location. Everything else is an *unmanaged listener*, visible only with
`lull ls --all`. On the author's machine this rule separated all 13 listening groups
correctly: system and GUI apps run with cwd `/`, `$HOME` or `~/Library/…`, Homebrew
services with cwd `/opt/homebrew/var/…`, and every dev server from its project directory.

## Considered Options

- **All listeners minus exclusions** (launchd jobs, brew services, apps). Rejected: needs a
  growing deny-list and a per-process launchd lookup, and still misses apps not managed
  by launchd.
- **Runtime allow-list** (node, bun, deno, python, php, ruby, workerd…). Rejected: misses
  compiled dev servers (`hugo server`, Go/Rust binaries) and needs constant upkeep.
- **Path deny-list on the executable** (e.g. `/opt/homebrew/Cellar`). Rejected: Homebrew
  Python lives there, so `python -m http.server` would disappear.

## Consequences

A dev server started from `$HOME` itself or from a directory outside `$HOME` (e.g.
`/Volumes/…`) is not managed; `[[project]]` rules in config can add extra roots later if
this bites. User LaunchAgents whose working directory is a project are listed as
user-started servers and are not auto-stopped by default.
