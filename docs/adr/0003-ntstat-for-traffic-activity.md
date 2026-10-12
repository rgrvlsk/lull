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
