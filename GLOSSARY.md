# lull

lull finds the dev servers running on a developer's machine, works out whether a human is
still using each one, and stops the forgotten ones.

## Servers

**Server**:
A group of processes started together from a project location that accepts connections
on at least one local port, treated as one unit for listing and stopping.
_Avoid_: service, app, process, instance

**Project location**:
A directory a developer works in: somewhere inside their home folder other than the home
folder itself and its Library, or a temporary directory.
_Avoid_: workspace, repo

**Unmanaged listener**:
Something else of the user's that accepts local connections (system services, installed
apps, background databases); lull can show it but never acts on it.
_Avoid_: service, daemon

**Protected group**:
A group of processes lull must never list or signal because it contains lull itself, an
agent, an agent-hosting app or a browser.
_Avoid_: whitelist, excluded server

## Origin

**Origin**:
Who started a server: an agent or the user.
_Avoid_: owner, source, creator

**Agent-started**:
A server whose processes inherited an agent marker and were not run from a human's
terminal or as an installed app.
_Avoid_: AI server, bot server, orphan

**User-started**:
Any server that is not agent-started, including ones launched through an agent-hosting
app's own UI.
_Avoid_: manual server, human server

**Agent marker**:
An environment variable that a coding agent sets for the commands it runs and that
identifies that agent.
_Avoid_: tag, flag, signature
