//! WP1.3 — activity (contract §5): resolves `TrafficEvent`s and established connections
//! to per-server `ActivityEvent`s, the scan fallback backend, and wake/start grace.

use std::collections::{BTreeMap, HashSet};

use crate::model::{
    Activity as Source, ActivityEvent, BackendHealth, ConnectionKind, CoveredBrowsers, Discovery,
    ServerId, Snapshot, TrafficEvent,
};
use crate::traits::{ActivityBackend, ActivityBackendKind, Instant};

/// Accepted-socket diff (ADR-0003 fallback): every `Snapshot::connections` 4-tuple not
/// seen in the previous drain is a `New` connection. Only sees connections open at a scan
/// instant; `traffic` (b) carries the load for anything held open.
#[derive(Debug, Default)]
pub struct ScanBackend {
    seen: HashSet<(u16, String, u16, Option<u32>)>,
}

impl ActivityBackend for ScanBackend {
    fn kind(&self) -> ActivityBackendKind {
        ActivityBackendKind::Scan
    }

    fn drain(&mut self, snapshot: &Snapshot, now: &Instant) -> Vec<TrafficEvent> {
        let current: HashSet<_> = snapshot
            .connections
            .iter()
            .map(|c| (c.port, c.remote_addr.clone(), c.remote_port, c.client_pid))
            .collect();
        let events = current
            .iter()
            .filter(|key| !self.seen.contains(key))
            .map(|&(port, _, _, client_pid)| TrafficEvent {
                port,
                client_pid,
                kind: ConnectionKind::New,
                at: now.clone(),
            })
            .collect();
        self.seen = current;
        events
    }

    fn health(&self) -> BackendHealth {
        BackendHealth::Healthy
    }
}

/// Everything one scan needs to resolve traffic to servers.
pub struct ScanInput<'a> {
    pub snapshot: &'a Snapshot,
    pub discovery: &'a Discovery,
    pub covered: &'a CoveredBrowsers,
    pub daemon_pid: u32,
    pub now: Instant,
    pub wall_ms: u64,
    pub scan_interval_ms: u64,
    pub wake_grace_ms: u64,
}

/// The `traffic` source (§5): owns the active backend, the scan fallback, wake detection
/// and the start / wake / post-switch holds on stopping.
pub struct Activity {
    primary: Option<Box<dyn ActivityBackend + Send>>,
    scan: ScanBackend,
    /// Why the primary was abandoned; reported by `health` for `lull doctor`.
    fallback: Option<String>,
    /// Daemon start or the last detected wake: no stops for `wake_grace_ms` after it.
    grace_from: Instant,
    last_scan: Option<(Instant, u64)>,
    /// Switch time and required awake interval; cleared only by a later scan.
    switch_hold: Option<(Instant, u64)>,
}

impl Activity {
    /// `primary` = ntstat when it opened; `None` → scan backend only. Starts in start grace
    /// (§5: no stops for `wake_grace` after daemon start, including a restart within the
    /// same boot).
    pub fn new(
        primary: Option<Box<dyn ActivityBackend + Send>>,
        now: &Instant,
        wall_ms: u64,
    ) -> Self {
        Self {
            primary,
            scan: ScanBackend::default(),
            fallback: None,
            grace_from: now.clone(),
            last_scan: Some((now.clone(), wall_ms)),
            switch_hold: None,
        }
    }

    /// Called once per scan: detects wake, drains the active backend (falling back to the
    /// scan backend when the primary reports `Failed`), and resolves traffic to servers.
    pub fn scan(&mut self, input: ScanInput) -> Vec<ActivityEvent> {
        let now = &input.now;
        self.detect_wake(now, input.wall_ms, input.scan_interval_ms);
        self.last_scan = Some((now.clone(), input.wall_ms));
        let mut events = Vec::new();
        if let Some(primary) = self.primary.as_mut() {
            events = primary.drain(input.snapshot, now);
            if let BackendHealth::Failed(reason) = primary.health() {
                self.primary = None;
                self.fallback = Some(reason);
                self.switch_hold = Some((now.clone(), input.scan_interval_ms));
            }
        }
        if self.primary.is_none() {
            events.extend(self.scan.drain(input.snapshot, now));
            if self.switch_hold.as_ref().is_some_and(|(from, interval)| {
                now.ms_since(from)
                    .is_some_and(|elapsed| elapsed >= *interval)
            }) {
                self.switch_hold = None;
            }
        }
        resolve(&input, &events)
    }

    /// Wake: wall time advanced more than awake time by over a scan interval (§5). A new
    /// boot session (instants no longer comparable) is treated as a fresh start.
    fn detect_wake(&mut self, now: &Instant, wall_ms: u64, scan_interval_ms: u64) {
        let Some((prev_now, prev_wall)) = &self.last_scan else {
            return;
        };
        let woke = match now.ms_since(prev_now) {
            None => true,
            Some(awake) => {
                wall_ms.saturating_sub(*prev_wall)
                    > awake.saturating_add(scan_interval_ms.max(1_000))
            }
        };
        if woke {
            self.grace_from = now.clone();
        }
    }

    /// Start / wake grace, or the post-switch hold.
    pub fn holds_stops(&self, now: &Instant, wake_grace_ms: u64) -> bool {
        let in_grace = now
            .ms_since(&self.grace_from)
            .is_none_or(|elapsed| elapsed < wake_grace_ms);
        in_grace || self.switch_hold.is_some()
    }

    /// Reported in `hello` / `lull doctor`.
    pub fn kind(&self) -> ActivityBackendKind {
        self.primary
            .as_ref()
            .map_or(ActivityBackendKind::Scan, |p| p.kind())
    }

    /// `Failed(reason)` after a runtime fallback, for doctor.
    pub fn health(&self) -> BackendHealth {
        match (&self.primary, &self.fallback) {
            (Some(primary), _) => primary.health(),
            (None, Some(reason)) => BackendHealth::Failed(reason.clone()),
            (None, None) => BackendHealth::Healthy,
        }
    }
}

/// §5 `traffic`: (a) a `New` backend event for a server port from a client outside the
/// group and not the daemon (a `None` client is outside; covered browsers count); (b) an
/// established connection in the snapshot from such a client that is not covered. One
/// event per server per scan, at the latest receipt time.
fn resolve(input: &ScanInput, events: &[TrafficEvent]) -> Vec<ActivityEvent> {
    let mut fired: BTreeMap<ServerId, Instant> = BTreeMap::new();
    for server in &input.discovery.servers {
        let outside = |client: Option<u32>| {
            !client.is_some_and(|pid| {
                pid == input.daemon_pid || server.members.iter().any(|member| member.pid == pid)
            })
        };
        let mut fire = |at: &Instant| {
            let entry = fired.entry(server.id.clone()).or_insert_with(|| at.clone());
            if at.ms_since(entry).is_some_and(|later| later > 0) {
                *entry = at.clone();
            }
        };
        for event in events {
            if event.kind == ConnectionKind::New
                && server.ports.contains(&event.port)
                && outside(event.client_pid)
            {
                fire(&event.at);
            }
        }
        for conn in &input.snapshot.connections {
            if server.ports.contains(&conn.port)
                && outside(conn.client_pid)
                && (conn.client_pid.is_none() || !input.covered.covers(&conn.client_ancestry))
            {
                fire(&input.now);
            }
        }
    }
    fired
        .into_iter()
        .map(|(server_id, at)| ActivityEvent {
            server_id,
            source: Source::Traffic,
            at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::model::{Connection, Framework, ObservedServer, Origin, Witness};
    use crate::testing::{FakeActivityBackend, FakeClock};
    use crate::traits::Clock;
    use lull_protocol::server::OriginKind;

    const SCAN: u64 = 5_000;
    const GRACE: u64 = 300_000;
    const DAEMON: u32 = 900;

    fn server(id: &str, pgid: u32, pids: &[u32], ports: &[u16]) -> ObservedServer {
        ObservedServer {
            id: id.into(),
            pgid,
            members: pids
                .iter()
                .map(|&pid| Witness {
                    pid,
                    start_epoch_ms: 1,
                })
                .collect(),
            ports: ports.to_vec(),
            primary_url: None,
            cwd: "/Users/x/app".into(),
            project: "app".into(),
            framework: Framework::Generic,
            origin: Origin {
                kind: OriginKind::Agent,
                agent: None,
                session_id: None,
            },
            command: "node".into(),
            launch_argv: vec![],
            restart_env: BTreeMap::new(),
            env_pin: false,
            started_at_ms: 0,
            cpu_percent: 0.0,
            rss_bytes: 0,
            incomplete: vec![],
        }
    }

    fn discovery() -> Discovery {
        Discovery {
            servers: vec![
                server("100-1", 100, &[100, 101], &[3000]),
                server("200-1", 200, &[200], &[4000, 4001]),
            ],
            ..Discovery::default()
        }
    }

    fn conn(port: u16, remote_port: u16, client: Option<u32>, ancestry: &[u32]) -> Connection {
        Connection {
            port,
            remote_addr: "127.0.0.1".into(),
            remote_port,
            client_pid: client,
            client_ancestry: ancestry.to_vec(),
        }
    }

    fn snapshot(connections: Vec<Connection>) -> Snapshot {
        Snapshot {
            connections,
            ..Snapshot::default()
        }
    }

    struct Harness {
        clock: FakeClock,
        activity: Activity,
        discovery: Discovery,
        covered: CoveredBrowsers,
    }

    impl Harness {
        fn new(primary: Option<Box<dyn ActivityBackend + Send>>) -> Self {
            let clock = FakeClock::default();
            let activity = Activity::new(primary, &clock.now(), clock.wall_now_ms());
            Self {
                clock,
                activity,
                discovery: discovery(),
                covered: CoveredBrowsers::default(),
            }
        }

        fn scan(&mut self, snapshot: &Snapshot) -> Vec<ActivityEvent> {
            self.activity.scan(ScanInput {
                snapshot,
                discovery: &self.discovery,
                covered: &self.covered,
                daemon_pid: DAEMON,
                now: self.clock.now(),
                wall_ms: self.clock.wall_now_ms(),
                scan_interval_ms: SCAN,
                wake_grace_ms: GRACE,
            })
        }

        fn ids(&mut self, snapshot: &Snapshot) -> Vec<String> {
            self.scan(snapshot)
                .into_iter()
                .map(|e| e.server_id)
                .collect()
        }

        fn holds(&self) -> bool {
            self.activity.holds_stops(&self.clock.now(), GRACE)
        }
    }

    fn fake(events: &[(u16, Option<u32>)], at: &Instant) -> FakeActivityBackend {
        FakeActivityBackend {
            kind: ActivityBackendKind::Ntstat,
            queue: events
                .iter()
                .map(|&(port, client_pid)| TrafficEvent {
                    port,
                    client_pid,
                    kind: ConnectionKind::New,
                    at: at.clone(),
                })
                .collect::<VecDeque<_>>(),
            health: BackendHealth::Healthy,
        }
    }

    #[test]
    fn new_events_fire_unless_client_is_in_group_or_daemon() {
        let at = FakeClock::default().now();
        let backend = fake(
            &[
                (3000, Some(101)),    // same group: ignored
                (4000, Some(DAEMON)), // the daemon: ignored
                (4001, None),         // remote client: counts
                (5000, Some(7)),      // no such server
            ],
            &at,
        );
        let mut h = Harness::new(Some(Box::new(backend)));
        assert_eq!(h.ids(&snapshot(vec![])), vec!["200-1"]);
        assert_eq!(h.activity.kind(), ActivityBackendKind::Ntstat);
        assert_eq!(h.activity.health(), BackendHealth::Healthy);

        let mut h = Harness::new(Some(Box::new(fake(&[(3000, Some(7))], &at))));
        assert_eq!(h.ids(&snapshot(vec![])), vec!["100-1"]);
    }

    #[test]
    fn established_backend_events_do_not_bypass_browser_coverage() {
        let at = FakeClock::default().now();
        let mut backend = fake(&[(3000, Some(300))], &at);
        backend.queue.front_mut().unwrap().kind = ConnectionKind::Established;
        let mut h = Harness::new(Some(Box::new(backend)));
        h.covered = CoveredBrowsers([250].into());
        assert!(
            h.scan(&snapshot(vec![conn(3000, 52000, Some(300), &[300, 250])]))
                .is_empty()
        );
    }

    #[test]
    fn shared_port_matches_each_server_and_established_events_use_scan_time() {
        let mut h = Harness::new(Some(Box::new(FakeActivityBackend::default())));
        h.discovery.servers[1].ports.push(3000);
        h.clock.advance_ms(100);
        let events = h.scan(&snapshot(vec![conn(3000, 52000, None, &[])]));
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.at == h.clock.now()));
        assert!(
            h.scan(&snapshot(vec![conn(3000, 52000, Some(DAEMON), &[DAEMON])]))
                .is_empty()
        );
    }

    #[test]
    fn scan_fallback_catches_a_connection_held_open_across_a_scan() {
        let mut h = Harness::new(None);
        assert_eq!(h.activity.kind(), ActivityBackendKind::Scan);
        let mut scan = ScanBackend::default();
        let now = h.clock.now();
        assert!(scan.drain(&snapshot(vec![]), &now).is_empty());
        let open = snapshot(vec![conn(3000, 51000, Some(7), &[7])]);
        let events = scan.drain(&open, &now);
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].port, events[0].client_pid), (3000, Some(7)));
        // Still open at the next scan: not new again.
        assert!(scan.drain(&open, &now).is_empty());
        assert!(scan.drain(&snapshot(vec![]), &now).is_empty());
        // Through Activity the same connection fires traffic at every scan via (b).
        assert_eq!(h.ids(&open), vec!["100-1"]);
        h.clock.advance_ms(SCAN);
        assert_eq!(h.ids(&open), vec!["100-1"]);
    }

    #[test]
    fn idle_open_connection_counts_unless_the_client_is_covered() {
        let mut h = Harness::new(None);
        let websocket = snapshot(vec![conn(4000, 52000, Some(300), &[300, 250, 1])]);
        assert_eq!(h.ids(&websocket), vec!["200-1"]);
        // The same connection from a process under a covered browser (pid 250).
        h.covered = CoveredBrowsers([250].into());
        h.clock.advance_ms(SCAN);
        assert!(h.ids(&websocket).is_empty());
        // A remote client is never covered; a same-group client never counts.
        let remote = snapshot(vec![conn(4000, 52000, None, &[])]);
        assert_eq!(h.ids(&remote), vec!["200-1"]);
        let own = snapshot(vec![conn(4000, 52000, Some(200), &[200])]);
        assert!(h.ids(&own).is_empty());
    }

    #[test]
    fn covered_browser_new_connection_still_counts_through_events() {
        let at = FakeClock::default().now();
        let mut h = Harness::new(Some(Box::new(fake(&[(3000, Some(300))], &at))));
        h.covered = CoveredBrowsers([250].into());
        let page = snapshot(vec![conn(3000, 52000, Some(300), &[300, 250])]);
        assert_eq!(h.ids(&page), vec!["100-1"]);
    }

    #[test]
    fn one_event_per_server_per_scan_at_the_latest_receipt() {
        let early = FakeClock::default().now();
        let late = early.plus_ms(400);
        let backend = FakeActivityBackend {
            kind: ActivityBackendKind::Ntstat,
            queue: VecDeque::from(vec![
                TrafficEvent {
                    port: 3000,
                    client_pid: Some(7),
                    kind: ConnectionKind::New,
                    at: early.clone(),
                },
                TrafficEvent {
                    port: 3000,
                    client_pid: None,
                    kind: ConnectionKind::New,
                    at: late.clone(),
                },
            ]),
            health: BackendHealth::Healthy,
        };
        let mut h = Harness::new(Some(Box::new(backend)));
        h.clock.advance_ms(200);
        let events = h.scan(&snapshot(vec![conn(3000, 1, Some(8), &[8])]));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].server_id, "100-1");
        assert_eq!(events[0].source, Source::Traffic);
        assert_eq!(events[0].at, late);
    }

    #[test]
    fn failing_primary_falls_back_to_scan_and_holds_stops_for_one_interval() {
        let at = FakeClock::default().now();
        let mut backend = fake(&[(3000, Some(7))], &at);
        backend.health = BackendHealth::Failed("socket closed".into());
        let mut h = Harness::new(Some(Box::new(backend)));
        h.clock.advance_ms(GRACE); // past the start grace
        assert!(!h.holds());
        let open = snapshot(vec![conn(4000, 1, Some(7), &[7])]);
        // The failing backend's last events are still used; then the switch happens.
        assert_eq!(h.ids(&open), vec!["100-1", "200-1"]);
        assert_eq!(h.activity.kind(), ActivityBackendKind::Scan);
        assert_eq!(
            h.activity.health(),
            BackendHealth::Failed("socket closed".into())
        );
        assert!(h.holds());
        h.clock.advance_ms(SCAN - 1);
        h.ids(&open); // An early scan must not end the interval hold.
        assert!(h.holds());
        h.clock.advance_ms(1);
        assert!(h.holds(), "held until a scan has run on the new backend");
        h.ids(&open);
        assert!(!h.holds());
        // The scan backend now diffs: nothing new, (b) still fires for the open one.
        assert_eq!(h.ids(&open), vec!["200-1"]);
    }

    #[test]
    fn start_grace_then_sleep_accrues_no_idle_time_and_triggers_wake_grace() {
        let mut h = Harness::new(None);
        assert!(h.holds(), "start grace");
        h.clock.advance_ms(GRACE - 1);
        assert!(h.holds());
        h.clock.advance_ms(1);
        assert!(!h.holds());
        let before = h.clock.now();
        h.scan(&snapshot(vec![]));
        // Lid closed for an hour: wall advances, uptime does not.
        h.clock.sleep_ms(3_600_000);
        assert_eq!(h.clock.now().ms_since(&before), Some(0), "no idle accrued");
        h.scan(&snapshot(vec![]));
        assert!(h.holds(), "wake grace");
        h.clock.advance_ms(GRACE);
        assert!(!h.holds());
        // Ordinary scans never look like a wake.
        h.clock.advance_ms(SCAN);
        h.scan(&snapshot(vec![]));
        assert!(!h.holds());
    }

    #[test]
    fn restart_within_the_same_boot_holds_stops_for_wake_grace() {
        let mut clock = FakeClock::default();
        clock.advance_ms(50 * GRACE); // the Mac has been up a while
        let fresh = Activity::new(None, &clock.now(), clock.wall_now_ms());
        assert!(fresh.holds_stops(&clock.now(), GRACE));
        assert!(fresh.holds_stops(&clock.now().plus_ms(GRACE - 1), GRACE));
        assert!(!fresh.holds_stops(&clock.now().plus_ms(GRACE), GRACE));
    }

    #[test]
    fn a_new_boot_session_is_a_fresh_start() {
        let mut h = Harness::new(None);
        h.clock.advance_ms(GRACE);
        h.scan(&snapshot(vec![]));
        assert!(!h.holds());
        h.clock.reboot("boot-B");
        h.scan(&snapshot(vec![]));
        assert!(h.holds());
    }
}
