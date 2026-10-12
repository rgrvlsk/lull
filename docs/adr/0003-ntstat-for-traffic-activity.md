# Traffic activity comes from the kernel network-statistics socket, with a scan fallback

Agents verify servers with short requests (a `curl` lasting ~50 ms) that a 5 s scan of
accepted sockets almost always misses, and browser reloads reuse keep-alive connections.
We subscribe to the kernel control socket `com.apple.network.statistics` (the interface
behind `nettop`), whose message ABI is published in xnu's open-source `bsd/net/ntstat.h`,
to receive every TCP connection as it is created. On macOS 27 `net.statistics_privcheck`
is 0 and an unsigned, unprivileged process can connect (verified 2026-10-12). We talk to
the kernel control directly, not to the private `NetworkStatistics.framework`, which
requires the Apple-only `com.apple.private.network.statistics` entitlement.

## Considered Options

- **Accept the miss** and rely on the start-up grace period, visible tabs and an explicit
  `lull touch`. Zero risk, but agent checks after the grace period go unseen.
- **Scan faster (1 s) for agent servers.** Still misses most short connections at five
  times the background cost.

## Consequences

This is undocumented ABI: Apple can change message versions or turn the privilege check
on. The ntstat reader lives behind the activity-backend trait, validates the message
version, and the daemon falls back to the periodic accepted-socket diff when the socket
cannot be opened or speaks an unknown version, surfacing the active backend in
`lull doctor`. Because the fallback only sees connections open at a scan instant, contract
§5 also counts established connections from clients that are not covered browsers, which
keeps open but quiet pages alive in either mode. CI should include a smoke test that opens
the socket on each new macOS.

## Addendum (S1, 2026-10-12)

S1 works on the tested machine: macOS 27.0.1 (26A434), Darwin 27.0.0,
xnu-13432.1.9, arm64, `net.statistics_privcheck=0`. The Rust integration test opens
ntstat without elevation, starts `python3 -m http.server` on an OS-assigned loopback
port, and verifies that a short `curl` produces `New` with that port and the exact
curl PID. Five consecutive live-test runs each took 10 ms for the HTTP request,
with no skips. The test kills and reaps only its own Python server; curl has a
request timeout and exits normally.
An `open()` error prints an explicit skip reason, so a skipped test is not live proof.

The public reference is Apple's
[xnu-12377.1.9 `bsd/net/ntstat.h`](https://github.com/apple-oss-distributions/xnu/blob/xnu-12377.1.9/bsd/net/ntstat.h)
(header revision 9; consulted 2026-10-12). The wire header has no version field.
Compatibility is therefore checked from message lengths, provider and plausible
prefix fields, rather than an invented version handshake. The published TCP
descriptor is 344 bytes. A fresh live sample returned nine plausible descriptors,
all **360 bytes**. The earlier S1 probe decoded pid at 116, local sockaddr at 124,
remote sockaddr at 152 and pname at 196. The descriptor tail has shifted; the backend
never reads tail role flags or UUID offsets. It uses state at 76, pid and endpoints.
Lengths below 344 or above 4096, impossible state/PID values and invalid sockaddr
length/family pairs fail parsing. IPv4 is `(16, 2)`, IPv6 is `(28, 30)`; all-zero
endpoints remain unresolved. IPv4, IPv6 and IPv4-mapped loopback decoding have unit
coverage; the live HTTP fixture uses IPv4.

The message flow is `ADD_ALL_SRCS(1002)` with TCP_KERNEL(2), filter/events zero,
then `SRC_ADDED(10001)` with a source reference and provider, **without endpoints**.
`GET_SRC_DESC(1005)` returns `SRC_DESC(10003)` with descriptor data at byte 40.
`SUCCESS` separates the initial enumeration from new sources. Initial sources
validate the ABI but do not create new-connection activity. Unconnected sources
remain pending until resolved or removed; requests are paced at up to 16 per 250 ms.
Closing descriptors can resolve requests shorter than that period. Duplicate
descriptions produce at most one event per source. For a loopback pair, the source
whose remote port is the server port supplies the client PID. Non-loopback peers
supply the local port with an unknown client. Activity matches these candidate ports
against discovery, excludes the daemon and group members, and applies covered-browser
exclusion only to established snapshot connections.

The reader raises `SO_RCVBUF` to 1 MiB, receives into 64 KiB, walks batched messages
by validated header lengths, and detects receive truncation. Descriptor `ENOENT`
means the source disappeared; other request failures are unhealthy. State is bounded
at 16,384 sources and 4,096 queued events. Overflow, malformed/unknown messages,
reported kernel drops, read failures and unexpected reader exit report failure.
`open()` requires a usable descriptor within three seconds; an idle machine with
no usable descriptors can conservatively use scan mode.

This is **not lossless**. Kernel queues can drop events without an observable error;
short connections may disappear before description; initial enumeration is suppressed;
port-only matching cannot distinguish servers bound to different addresses at the
same port. No remote-network HTTP, sustained-load or other macOS-version validation
is claimed. The fixed pacing and capacity limits are a documented `ponytail:` debt;
measure real workloads before tuning them or introducing adaptive scheduling.

On open failure the caller selects `ScanBackend`. On runtime failure, `Activity`
keeps the failure reason in `health()`, reports scan kind, drains any final events,
and starts diffing snapshot connection four-tuples. Stops remain held until a scan
runs after one full awake scan interval. Established uncovered connections continue
to refresh activity every scan. Start/wake grace uses awake time; sleeping advances
wall time only, and a daemon restart in the same boot receives a fresh start grace.
No shared boundary types, dependencies or contract behavior were changed.
