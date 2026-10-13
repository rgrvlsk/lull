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

## Lifecycle

**Witness**:
A process of a server recorded by its pid and start time, used to prove the process
group is still the same one before lull acts on it.
_Avoid_: anchor, fingerprint

**Stop**:
To end a server's processes for good; its in-memory state is lost.
_Avoid_: sleep, pause, suspend, kill, reap

**Stopping soon**:
The short warning period before an idle server is stopped, during which the user can keep
it alive.
_Avoid_: sleeping soon, grace

**Stopped-server record**:
What lull remembers about a recently stopped server so the user can see it and restart it.
_Avoid_: history item, tombstone

**Launch command**:
The command that started a server, as lull would re-run it on restart (e.g. `npm run dev`),
without the shell or agent wrapper around it.
_Avoid_: start command, original command

**Restart**:
To start a stopped server again, by hand, from its stopped-server record; the result is a
new server.
_Avoid_: wake, resume, revive

## Browser

**Dev group**:
The tab group in each browser window that holds the tabs showing servers.
_Avoid_: localhost group, dev tabs folder

**Bound tab**:
A tab that lull has matched to a particular server or stopped-server record.
_Avoid_: adopted tab, linked tab

## Activity

**Activity**:
Evidence that a human or a tool is using a server right now.
_Avoid_: usage, load, busy

**Activity source**:
One kind of evidence of activity: a visible tab, new traffic, the server having just
started, or an explicit request.
_Avoid_: signal, trigger, heartbeat

**Visible tab**:
A browser tab showing a server on screen while the user is at the computer, whether or
not the browser has focus.
_Avoid_: focused tab, active tab

**Idle**:
A server with no activity for longer than its idle time.
_Avoid_: inactive, unused, asleep

**Pulse**:
One or two seconds in which a client opened a connection to a server or moved bytes on
one; the unit traffic activity is judged in.
_Avoid_: burst, hit, ping

**Train**:
A run of pulses from one client at a steady period with matching sizes; once it holds
four pulses, the pulses that continue it are periodic traffic.
_Avoid_: rhythm, pattern, beat

**Periodic traffic**:
Pulses from one client that repeat at a steady period with the same size, such as an HMR
ping or a status poll from a hidden tab. It is not activity.
_Avoid_: polling noise, keep-alive traffic

**Covered browser**:
A browser whose lull extension is connected, so its tabs are judged by visibility rather
than by the connections it holds open. All profiles of a Chromium browser are covered
together, so the extension belongs in each of them.
_Avoid_: connected browser, instrumented browser

**Grace period**:
The time a newly seen server is treated as active before any other activity is required.
_Avoid_: warm-up, cooldown

**Pin**:
A user's instruction that one server must never be stopped automatically, kept until the
server goes away. Its *pin source* is `env` when the server started with `LULL_KEEP=1`
(shown as *pinned (env)*) and `user` once anyone pins or unpins it explicitly.
_Avoid_: lock, keep-alive, favourite

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
