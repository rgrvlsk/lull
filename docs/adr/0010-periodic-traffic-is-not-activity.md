# Periodic traffic is not activity, judged from ntstat byte counters

Dev pages poll their server on a timer: HMR pings, status checks, dashboards. A hidden
tab keeps polling, and a poll every 10 s outlives Node's default 5 s keep-alive, so each
poll opens a new connection and fires `traffic` (a). Without the extension it is worse:
contract §5 `traffic` (b) counted any established connection from an uncovered client,
so a hidden tab holding an HMR websocket kept its server alive for as long as the tab
stayed open. The extension is optional, so the answer cannot depend on it, and the
daemon cannot tell which tab a connection belongs to: Chromium opens every tab's
connections from one network-service process.

We judge traffic in **pulses** (a client opening a connection to a server or moving bytes
on one, in one or two one-second slots) and drop pulses that continue a confirmed
**train**: a run of pulses from one client at a steady period with the same size. Bytes
come from the ntstat socket ADR-0003 already opens: `QUERY_SRC` (1004) for one source
returns `SRC_COUNTS` (10004), whose `nstat_counts` start with rx/tx packets and bytes
(xnu-12377.1.9 `bsd/net/ntstat.h`, consulted 2026-10-13). The daemon queries only the
sources on server ports with a client outside the group, once a second, so the cost
grows with the dev connections, not with every socket on the machine. `traffic` (b) now
fires for bytes moving on an established connection, not for the connection being open.
The thresholds (period 3 s to 10 min, timing within max(1 s, *P*/10), size within 10 %
or 32 bytes, four pulses to confirm, 16 trains per client) are in contract §5.

## Considered Options

- **Attribute requests to tabs in the extension** (`webRequest`, then tell the daemon
  which connections came from hidden tabs). Precise, but only with the extension
  installed, and it adds a permission.
- **Read request contents** (match HMR paths such as `/__vite_ping`). Needs packet capture
  (root or the `access_bpf` group), fails on HTTPS dev servers, and needs a path list per
  framework that breaks with each release.
- **Detect a steady period per client from connection timing alone.** Hidden tabs share a
  client pid and interleave, so the gaps alternate and no single period appears. Without
  sizes, continuous use also looks like several interleaved trains, which would hide a
  person using the server. Sizes are what separate a poll from a person.
- **Keep (b) as it was.** A hidden tab would keep its server alive for as long as it stays
  open whenever the extension is missing.

## Consequences

- Without the extension, a page that a person reads for longer than the idle time
  without clicking anything no longer counts, so its server can stop. "Idle" means no
  evidence of use, and an open connection is not evidence. A pin, `LULL_KEEP=1` or the
  extension keep such a server.
- A poller fires `traffic` for its first three pulses, and for three more each time its
  period or size changes (Chromium throttles hidden tabs to one timer per minute after
  five minutes). With `idle_minutes = 30` this does not matter.
- A client that varies the size or timing of every request is not recognised and keeps
  its server alive. The filter fails towards keeping servers, never towards stopping
  one in use.
- The scan fallback has no counters, so in that mode (b) still counts every established
  uncovered connection and nothing is filtered. This replaces the sentence in ADR-0003's
  consequences that relies on (b) to keep quiet pages alive in either mode.
- Activity depends on one more undocumented message pair. A `SRC_COUNTS` of the wrong
  length or an error reply to `QUERY_SRC` makes the backend unhealthy, which switches to
  the scan fallback as ADR-0003 describes.

## Addendum (2026-10-13): counters are read four times a second

Reading the counters once a second, as first decided, stamps each pulse up to a second
late, and the train tolerance is also one second. With that, a poll at 5.02 s (a 5 s timer
plus request latency) and 30 ms of timer jitter now and then shows an interval two seconds
off another. The simulation in `crates/core/src/activity/periodic.rs` broke the train at
181 s, 683 s and 934 s, and each break fired `traffic` again, so such a server would never
go idle. The daemon now reads the counters every 250 ms on a fixed grid and stamps bytes
with the grid instant. The same simulation stays one train for periods from 3 s to 60 s.
Each train's period is the mean of its intervals, so the error does not add up. The cost
is four small queries a second per watched connection; a development machine has a few
dozen at most. Pulse slots remain one second.

Live check on macOS 27.0.1 (Darwin 27.0.0, arm64, `net.statistics_privcheck=0`): the test
`crates/core/tests/ntstat_macos.rs` holds a loopback connection to a fixture server
without elevation. It sends 16 bytes, then 11, and receives `Bytes(16)` then `Bytes(11)`
for its own pid and the server's port. It passed five consecutive runs. That confirms
`QUERY_SRC` (1004), `SRC_COUNTS` (10004) and the `nstat_rxbytes`/`nstat_txbytes` offsets
(40 and 56 in the message) against a live kernel.

The sources that existed before the subscription are described as well, so their bytes
count after a daemon restart. The reader describes new sources first; otherwise a backlog
of a few hundred older sources delayed new connections by seconds.
