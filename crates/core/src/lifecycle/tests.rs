use std::collections::BTreeSet;
use std::io;

use lull_protocol::server::{Origin, OriginKind};
use lull_protocol::{AutoStop, Config, Framework, ProjectRule, RestartStatus};

use super::*;
use crate::model::{Discovery, ObservedServer, ProcessInfo, Witness};
use crate::testing::{
    FakeClock, FakeProcessSource, FakeSpawner, MemoryStateStore, RecordingSignaler,
};
use crate::traits::{Clock, Signal};

const UID: u32 = 501;
const MIN: u64 = 60_000;
const ID: &str = "100-1000";

fn process(pid: u32, pgid: u32, start: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        pgid,
        ppid: 1,
        uid: UID,
        start_epoch_ms: start,
        ..ProcessInfo::default()
    }
}

fn server(pgid: u32, kind: OriginKind) -> ObservedServer {
    ObservedServer {
        id: format!("{pgid}-1000"),
        pgid,
        members: vec![Witness {
            pid: pgid + 1,
            start_epoch_ms: 1000,
        }],
        ports: vec![5173],
        primary_url: Some("http://127.0.0.1:5173/".into()),
        cwd: format!("/Users/x/p{pgid}"),
        project: format!("p{pgid}"),
        framework: Framework::Vite,
        origin: Origin {
            kind,
            agent: None,
            session_id: None,
        },
        command: "npm run dev".into(),
        launch_argv: vec!["npm".into(), "run".into(), "dev".into()],
        restart_env: crate::testing::env(&[("PORT", "5173")]),
        env_pin: false,
        started_at_ms: 1000,
        cpu_percent: 0.0,
        rss_bytes: 0,
        incomplete: vec![],
    }
}

/// One lifecycle on fakes. `servers` is what discovery reports; `src` is what the fresh
/// `group()` re-check sees. `add`/`exit` keep the two in step.
struct H {
    clock: FakeClock,
    lc: Lifecycle,
    sig: RecordingSignaler,
    src: FakeProcessSource,
    config: Config,
    servers: Vec<ObservedServer>,
    protected_pgids: BTreeSet<u32>,
    daemon_pgid: u32,
    hold: bool,
    dry: bool,
    events: Vec<ActivityEvent>,
}

impl H {
    fn new() -> Self {
        let mut h = Self {
            clock: FakeClock::default(),
            lc: Lifecycle::new(),
            sig: RecordingSignaler::default(),
            src: FakeProcessSource::default(),
            config: Config::default(),
            servers: vec![],
            protected_pgids: BTreeSet::new(),
            daemon_pgid: 50,
            hold: false,
            dry: false,
            events: vec![],
        };
        h.add(server(100, OriginKind::Agent));
        h
    }

    fn add(&mut self, s: ObservedServer) {
        for w in &s.members {
            let p = process(w.pid, s.pgid, w.start_epoch_ms);
            self.src.snapshot.processes.push(p);
        }
        self.servers.push(s);
    }

    fn exit(&mut self, pgid: u32) {
        self.servers.retain(|s| s.pgid != pgid);
        self.src.snapshot.processes.retain(|p| p.pgid != pgid);
    }

    fn server_mut(&mut self, pgid: u32) -> &mut ObservedServer {
        self.servers.iter_mut().find(|s| s.pgid == pgid).unwrap()
    }

    fn tick(&mut self) -> Vec<Effect> {
        let discovery = Discovery {
            servers: self.servers.clone(),
            unmanaged: vec![],
            protected: self.protected_pgids.clone(),
        };
        let protected = |m: &[ProcessInfo]| {
            m.iter()
                .any(|p| p.exe.as_deref() == Some("/protected/claude"))
        };
        let mut sys = Sys {
            signaler: &mut self.sig,
            source: &mut self.src,
            protected: &protected,
            uid: UID,
            daemon_pgid: self.daemon_pgid,
        };
        let tick = Tick {
            now: self.clock.now(),
            wall_ms: self.clock.wall_now_ms(),
            discovery: &discovery,
            events: std::mem::take(&mut self.events),
            config: &self.config,
            home: "/Users/x",
            hold_stops: self.hold,
            dry_run: self.dry,
            log_dir: "/logs",
        };
        self.lc.tick(tick, &mut sys)
    }

    /// Advances awake time, then ticks.
    fn at(&mut self, ms: u64) -> Vec<Effect> {
        self.clock.advance_ms(ms);
        self.tick()
    }

    fn state(&self, id: &str) -> ServerState {
        self.lc.record(id).unwrap().state
    }

    fn traffic(&mut self, id: &str) {
        self.events.push(ActivityEvent {
            server_id: id.into(),
            source: Activity::Traffic,
            at: self.clock.now(),
        });
    }
}

fn has(fx: &[Effect], wanted: &Effect) -> bool {
    fx.contains(wanted)
}

fn removed(id: &str, reason: StopReason) -> Effect {
    Effect::Removed {
        id: id.into(),
        reason,
    }
}

#[test]
fn idle_agent_server_warns_then_stops_and_is_recorded() {
    let mut h = H::new();
    h.tick();
    assert_eq!(
        h.lc.record(ID).unwrap().last_activity_source,
        Activity::Started
    );
    h.at(27 * MIN);
    assert_eq!(h.state(ID), ServerState::Running);
    h.at(MIN);
    assert_eq!(h.state(ID), ServerState::StoppingSoon);
    h.at(2 * MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);
    assert_eq!(h.state(ID), ServerState::Stopping);

    h.exit(100);
    let fx = h.at(1000);
    assert!(has(&fx, &removed(ID, StopReason::Idle)));
    assert!(h.lc.record(ID).is_none());
    let history = h.lc.history();
    assert_eq!(history.len(), 1);
    let rec = &history[0];
    assert_eq!(
        (rec.reason, rec.status, rec.argv.join(" "), rec.cwd.as_str()),
        (
            StopReason::Idle,
            RestartStatus::Stopped,
            "npm run dev".into(),
            "/Users/x/p100"
        )
    );
    assert!(rec.tabs.is_empty());
    assert_eq!(
        rec.restart_env.get("PORT").map(String::as_str),
        Some("5173")
    );
}

#[test]
fn activity_pin_or_policy_change_returns_stopping_soon_to_running() {
    type Change = fn(&mut H);
    let changes: [(&str, Change); 4] = [
        ("traffic", |h| h.traffic(ID)),
        ("pin", |h| {
            let now = h.clock.now();
            h.lc.pin(ID, true, &now, 0).unwrap();
        }),
        ("autoStop off", |h| {
            h.lc.set_auto_stop(ID, Some(false)).unwrap()
        }),
        ("policy", |h| h.config.auto_stop = AutoStop::None),
    ];
    for (name, change) in changes {
        let mut h = H::new();
        h.tick();
        h.at(29 * MIN);
        assert_eq!(h.state(ID), ServerState::StoppingSoon, "{name}");
        change(&mut h);
        h.at(2 * MIN);
        assert_eq!(h.state(ID), ServerState::Running, "{name}");
        assert!(h.sig.sent.is_empty(), "{name}");
    }

    // Activity is reflected at once, before the next tick.
    let mut h = H::new();
    h.tick();
    h.at(29 * MIN);
    let now = h.clock.now();
    h.lc.activity(
        ActivityEvent {
            server_id: ID.into(),
            source: Activity::Tab,
            at: now.clone(),
        },
        h.clock.wall_ms,
    );
    let rec = h.lc.record(ID).unwrap();
    assert_eq!(rec.state, ServerState::Running);
    assert_eq!(rec.stops_at, Some(now.plus_ms(30 * MIN)));
    assert_eq!(rec.last_activity_source, Activity::Tab);
}

#[test]
fn incomplete_observation_or_hold_defers_the_stop() {
    type Block = fn(&mut H, bool);
    let blocks: [(&str, Block); 2] = [
        ("incomplete", |h, on| {
            h.server_mut(100).incomplete = if on { vec!["env".into()] } else { vec![] }
        }),
        ("hold", |h, on| h.hold = on),
    ];
    for (name, block) in blocks {
        let mut h = H::new();
        h.tick();
        block(&mut h, true);
        let fx = h.at(31 * MIN);
        assert!(h.sig.sent.is_empty(), "{name}");
        assert_eq!(fx.iter().filter(|f| matches!(f, Effect::Log(_))).count(), 1);
        assert!(h.at(5000).is_empty(), "{name}: deferral logged once");
        assert!(
            h.lc.ineligible_reason(ID).is_some(),
            "{name}: doctor explains it"
        );
        block(&mut h, false);
        h.at(5000);
        assert_eq!(h.sig.sent, vec![(100, Signal::Term)], "{name}");
    }
}

#[test]
fn reused_pgid_aborts_before_sigterm() {
    // The scan recorded 101@1000; the fresh re-check finds the pgid with other members.
    let fresh_groups = [
        vec![process(102, 100, 5000)],
        vec![process(101, 100, 9999)], // same pid, different start time
    ];
    for fresh in fresh_groups {
        let mut h = H::new();
        h.tick();
        h.clock.advance_ms(30 * MIN);
        h.src.snapshot.processes = fresh;
        let fx = h.tick();
        assert!(h.sig.sent.is_empty());
        assert!(has(&fx, &Effect::StopAborted { id: ID.into() }));
        let rec = h.lc.record(ID).unwrap();
        assert_eq!(rec.state, ServerState::Running);
        assert_eq!(rec.last_activity_source, Activity::Manual);
        assert!(h.lc.history().is_empty());
        assert_eq!(h.lc.doctor().len(), 1);
    }
}

#[test]
fn partial_exit_on_sigterm_still_sigkills_then_stuck_then_removed() {
    let mut h = H::new();
    let s = h.server_mut(100);
    s.members.push(Witness {
        pid: 102,
        start_epoch_ms: 1001,
    });
    h.src.snapshot.processes.push(process(102, 100, 1001));
    h.tick();
    h.at(30 * MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);

    // 102 exits on SIGTERM, witness 101 survives.
    h.src.snapshot.processes.retain(|p| p.pid != 102);
    h.server_mut(100).members.retain(|w| w.pid != 102);
    assert!(h.at(4000).is_empty());
    h.at(1000);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term), (100, Signal::Kill)]);

    let fx = h.at(5000);
    assert!(has(&fx, &Effect::StopStuck { id: ID.into() }));
    assert_eq!(h.state(ID), ServerState::Stopping);
    assert!(h.lc.doctor()[0].contains("stuck"));
    assert!(h.at(5000).is_empty(), "stuck reported once");

    h.exit(100);
    let fx = h.at(5000);
    assert!(has(&fx, &removed(ID, StopReason::Idle)));
    assert_eq!(h.sig.sent.len(), 2);
}

#[test]
fn all_witnesses_gone_after_sigterm_aborts_without_history() {
    let mut h = H::new();
    h.tick();
    h.at(30 * MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);

    // 101 exited; a new child 103 is in the group.
    h.src.snapshot.processes = vec![process(103, 100, 9000)];
    h.server_mut(100).members = vec![Witness {
        pid: 103,
        start_epoch_ms: 9000,
    }];
    let fx = h.at(5000);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)], "no SIGKILL");
    assert!(has(&fx, &Effect::StopAborted { id: ID.into() }));
    assert!(!fx.iter().any(|f| matches!(f, Effect::Removed { .. })));
    let rec = h.lc.record(ID).unwrap();
    assert_eq!(rec.state, ServerState::Running);
    assert_eq!(rec.last_activity_source, Activity::Manual);
    assert_eq!(rec.stops_at, Some(h.clock.now().plus_ms(30 * MIN)));
    assert!(h.lc.history().is_empty());
    assert!(h.lc.doctor()[0].contains("aborted"));
}

#[test]
fn unsafe_groups_are_never_signalled() {
    type Setup = fn(&mut H);
    let cases: [(&str, Setup); 6] = [
        ("protected exe", |h| {
            h.src.snapshot.processes[0].exe = Some("/protected/claude".into())
        }),
        ("protected pgid", |h| {
            h.protected_pgids.insert(100);
        }),
        ("other uid", |h| {
            h.src.snapshot.processes.push(ProcessInfo {
                uid: 0,
                ..process(105, 100, 1000)
            })
        }),
        ("daemon pgid", |h| h.daemon_pgid = 100),
        ("unreadable group", |h| {
            h.src.fail_next = Some(io::ErrorKind::PermissionDenied)
        }),
        ("empty group", |h| h.src.snapshot.processes.clear()),
    ];
    for (name, setup) in cases {
        for manual in [false, true] {
            let mut h = H::new();
            h.tick();
            setup(&mut h);
            if manual {
                h.lc.request_stop(ID).unwrap();
                h.at(1000);
            } else {
                h.at(31 * MIN);
            }
            assert!(h.sig.sent.is_empty(), "{name} manual={manual}");
            assert_ne!(h.state(ID), ServerState::Stopping, "{name}");
        }
    }

    // pgid 1 is refused even when discovery reports it.
    let mut h = H::new();
    h.exit(100);
    h.add(server(1, OriginKind::Agent));
    h.tick();
    h.lc.request_stop("1-1000").unwrap();
    h.at(31 * MIN);
    assert!(h.sig.sent.is_empty());
}

#[test]
fn manual_stop_skips_stopping_soon_and_is_coalesced() {
    let mut h = H::new();
    h.exit(100);
    let mut s = server(100, OriginKind::User);
    s.env_pin = true;
    h.add(s);
    h.tick();
    assert_eq!(h.lc.request_stop("nope"), Err(LifecycleError::UnknownId));
    h.lc.request_stop(ID).unwrap();
    h.lc.request_stop(ID).unwrap();
    h.at(1000);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);
    assert_eq!(h.state(ID), ServerState::Stopping);
    h.lc.request_stop(ID).unwrap();
    h.at(1000);
    assert_eq!(h.sig.sent.len(), 1, "coalesced with the stop under way");

    h.exit(100);
    let fx = h.at(1000);
    assert!(has(&fx, &removed(ID, StopReason::Manual)));
    assert_eq!(h.lc.history()[0].reason, StopReason::Manual);
}

#[test]
fn origin_and_auto_stop_policy_table() {
    use OriginKind::{Agent, User};
    let cases = [
        (AutoStop::Agent, User, false),
        (AutoStop::Agent, Agent, true),
        (AutoStop::All, User, true),
        (AutoStop::None, Agent, false),
    ];
    for (mode, kind, stops) in cases {
        let mut h = H::new();
        h.exit(100);
        h.add(server(100, kind));
        h.config.auto_stop = mode;
        h.tick();
        assert_eq!(
            h.lc.ineligible_reason(ID).is_none(),
            stops,
            "{mode:?} {kind:?}"
        );
        h.at(31 * MIN);
        assert_eq!(!h.sig.sent.is_empty(), stops, "{mode:?} {kind:?}");
    }
}

#[test]
fn longest_project_rule_sets_idle_time() {
    let mut h = H::new();
    let rule = |path: &str, idle| ProjectRule {
        path: path.into(),
        idle_minutes: Some(idle),
        auto_stop: None,
    };
    h.config.projects = vec![rule("~", 10), rule("~/p100", 60)];
    h.tick();
    h.at(31 * MIN);
    assert!(h.sig.sent.is_empty());
    h.at(29 * MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);
}

#[test]
fn no_listener_is_never_auto_stopped() {
    let mut h = H::new();
    h.server_mut(100).ports.clear();
    h.tick();
    h.at(90 * MIN);
    assert!(h.sig.sent.is_empty());
    let rec = h.lc.record(ID).unwrap();
    assert_eq!(
        (rec.state, rec.stops_at.clone()),
        (ServerState::NoListener, None)
    );
}

#[test]
fn exited_server_is_recorded() {
    let mut h = H::new();
    h.tick();
    h.exit(100);
    let fx = h.at(MIN);
    assert!(has(&fx, &removed(ID, StopReason::Exited)));
    assert_eq!(h.lc.history()[0].reason, StopReason::Exited);
}

#[test]
fn env_pin_overridden_by_unpin_persists_across_save_and_load() {
    let mut h = H::new();
    h.server_mut(100).env_pin = true;
    h.tick();
    assert!(h.lc.record(ID).unwrap().pinned);
    h.at(40 * MIN);
    assert!(h.sig.sent.is_empty(), "env-pinned");

    let now = h.clock.now();
    h.lc.pin(ID, false, &now, h.clock.wall_ms).unwrap();
    assert_eq!(
        h.lc.record(ID).unwrap().last_activity_source,
        Activity::Manual
    );
    let mut store = MemoryStateStore::default();
    h.lc.save(&mut store).unwrap();

    let (lc, warnings) = Lifecycle::load(&mut store, &h.clock.now());
    assert!(warnings.is_empty());
    h.lc = lc;
    h.tick();
    assert!(
        !h.lc.record(ID).unwrap().pinned,
        "unpin survives, env pin ignored"
    );
    h.at(30 * MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)]);
}

#[test]
fn daemon_restart_within_a_boot_keeps_idle_time() {
    let mut h = H::new();
    h.tick();
    h.at(20 * MIN);
    let mut store = MemoryStateStore::default();
    h.lc.save(&mut store).unwrap();

    h.clock.advance_ms(MIN);
    let (lc, _) = Lifecycle::load(&mut store, &h.clock.now());
    assert_eq!(lc.known_groups()[0].witnesses, h.servers[0].members);
    h.lc = lc;
    h.hold = true; // start grace
    h.tick();
    h.hold = false;
    h.at(8 * MIN);
    assert!(h.sig.sent.is_empty());
    h.at(MIN);
    assert_eq!(h.sig.sent, vec![(100, Signal::Term)], "30 min after start");

    // An instant from another boot session counts as now.
    let mut h = H::new();
    h.tick();
    h.lc.save(&mut store).unwrap();
    h.clock.reboot("boot-B");
    h.clock.advance_ms(5 * MIN);
    let now = h.clock.now();
    let (lc, _) = Lifecycle::load(&mut store, &now);
    assert_eq!(lc.record(ID).unwrap().last_activity, now);
}

#[test]
fn persisted_records_not_seen_on_first_tick_are_dropped_silently() {
    let mut h = H::new();
    h.tick();
    let mut store = MemoryStateStore::default();
    h.lc.save(&mut store).unwrap();
    let (lc, _) = Lifecycle::load(&mut store, &h.clock.now());
    h.lc = lc;
    h.exit(100);
    assert!(h.tick().is_empty());
    assert!(h.lc.history().is_empty());
    h.lc.save(&mut store).unwrap();
    assert_eq!(store.state.unwrap()["servers"], serde_json::json!({}));
}

#[test]
fn save_writes_only_on_change() {
    let mut h = H::new();
    h.tick();
    let mut store = MemoryStateStore::default();
    h.lc.save(&mut store).unwrap();
    h.at(5000);
    h.lc.save(&mut store).unwrap();
    assert_eq!(store.saves, 1);
    h.traffic(ID);
    h.at(5000);
    h.lc.save(&mut store).unwrap();
    assert_eq!(store.saves, 2);
}

#[test]
fn corrupt_or_unknown_version_state_is_quarantined() {
    let stores = [
        MemoryStateStore {
            corrupt: true,
            ..MemoryStateStore::default()
        },
        MemoryStateStore {
            state: Some(serde_json::json!({"version": 2, "servers": {}})),
            ..MemoryStateStore::default()
        },
        MemoryStateStore {
            state: Some(serde_json::json!({"version": 1, "servers": 3})),
            ..MemoryStateStore::default()
        },
    ];
    for mut store in stores {
        let (lc, warnings) = Lifecycle::load(&mut store, &FakeClock::default().now());
        assert_eq!(store.quarantined, 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("state.json.corrupt-0"));
        assert!(lc.known_groups().is_empty());
    }
}

/// Stops server 100 by exit so it lands in history.
fn with_history(argv: Vec<String>) -> H {
    let mut h = H::new();
    h.server_mut(100).launch_argv = argv;
    h.tick();
    h.exit(100);
    h.at(1000);
    h
}

#[test]
fn restart_succeeds_when_a_new_server_listens_in_cwd() {
    let mut h = with_history(vec!["npm".into(), "run".into(), "dev".into()]);
    let mut spawner = FakeSpawner::default();
    let now = h.clock.now();
    let pid = h.lc.restart(ID, &mut spawner, "/logs", &now).unwrap();
    assert_eq!(pid, 1);
    let req = &spawner.requests[0];
    assert_eq!(req.argv.join(" "), "npm run dev");
    assert_eq!(req.cwd, "/Users/x/p100");
    assert_eq!(req.log_path, "/logs/restart-100-1000.log");
    assert_eq!(req.env, crate::testing::env(&[("PORT", "5173")]));
    assert_eq!(h.lc.history()[0].status, RestartStatus::Restarting);
    assert_eq!(
        h.lc.restart(ID, &mut spawner, "/logs", &now),
        Err(LifecycleError::AlreadyRestarting)
    );

    let mut s = server(200, OriginKind::User);
    s.cwd = "/Users/x/p100".into();
    h.add(s);
    let fx = h.at(10_000);
    assert!(has(
        &fx,
        &Effect::RestartSucceeded {
            record_id: ID.into(),
            server_id: "200-1000".into()
        }
    ));
    assert!(h.lc.history().is_empty());
}

#[test]
fn failed_restart_keeps_the_record() {
    // Timeout.
    let mut h = with_history(vec!["npm".into(), "run".into(), "dev".into()]);
    let mut spawner = FakeSpawner::default();
    let now = h.clock.now();
    h.lc.restart(ID, &mut spawner, "/logs", &now).unwrap();
    assert_eq!(
        h.lc.next_deadline(),
        Some(now.plus_ms(RESTART_WINDOW_MS)),
        "restart window"
    );
    assert!(h.at(59_000).is_empty());
    let fx = h.at(1000);
    assert!(has(
        &fx,
        &Effect::RestartFailed {
            record_id: ID.into()
        }
    ));
    assert_eq!(h.lc.history()[0].status, RestartStatus::RestartFailed);
    // A later server in the same cwd does not count.
    let mut s = server(200, OriginKind::User);
    s.cwd = "/Users/x/p100".into();
    h.add(s);
    h.at(1000);
    assert_eq!(h.lc.history()[0].status, RestartStatus::RestartFailed);
    // A failed record may be retried.
    h.lc.restart(ID, &mut spawner, "/logs", &h.clock.now())
        .unwrap();

    // Spawn error.
    let mut h = with_history(vec!["npm".into(), "run".into(), "dev".into()]);
    let mut spawner = FakeSpawner {
        fail_next: Some(io::ErrorKind::NotFound),
        ..FakeSpawner::default()
    };
    let now = h.clock.now();
    assert!(matches!(
        h.lc.restart(ID, &mut spawner, "/logs", &now),
        Err(LifecycleError::SpawnFailed(_))
    ));
    assert_eq!(h.lc.history()[0].status, RestartStatus::RestartFailed);
    let fx = h.tick();
    assert!(has(
        &fx,
        &Effect::RestartFailed {
            record_id: ID.into()
        }
    ));
}

#[test]
fn restart_refusals() {
    let mut h = with_history(vec![]);
    let mut spawner = FakeSpawner::default();
    let now = h.clock.now();
    assert_eq!(
        h.lc.restart(ID, &mut spawner, "/logs", &now),
        Err(LifecycleError::NotRestartable)
    );
    assert_eq!(
        h.lc.restart("nope", &mut spawner, "/logs", &now),
        Err(LifecycleError::UnknownId)
    );
    assert!(spawner.requests.is_empty());
    h.lc.forget(ID);
    h.lc.forget(ID);
    assert!(h.lc.history().is_empty());
}

#[test]
fn history_retention_by_size_and_age() {
    let mut h = H::new();
    h.config.history_size = 2;
    h.add(server(200, OriginKind::Agent));
    h.add(server(300, OriginKind::Agent));
    h.tick();
    for pgid in [100, 200, 300] {
        h.exit(pgid);
        h.at(1000);
    }
    let ids: Vec<String> = h.lc.history().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, ["300-1000", "200-1000"]);

    h.clock.sleep_ms(23 * 60 * MIN);
    h.tick();
    assert_eq!(h.lc.history().len(), 2);
    h.clock.sleep_ms(61 * MIN);
    h.tick();
    assert!(h.lc.history().is_empty());
}

#[test]
fn dry_run_decides_and_logs_without_signals() {
    let mut h = H::new();
    h.dry = true;
    h.tick();
    let fx = h.at(30 * MIN);
    assert!(h.sig.sent.is_empty());
    assert!(
        fx.iter()
            .any(|f| matches!(f, Effect::Log(l) if l.contains("dry-run")))
    );
    assert!(h.at(5000).is_empty(), "not repeated every tick");
    h.lc.request_stop(ID).unwrap();
    let fx = h.at(5000);
    assert!(
        fx.iter()
            .any(|f| matches!(f, Effect::Log(l) if l.contains("manual")))
    );
    assert!(h.sig.sent.is_empty());
}

#[test]
fn next_deadline_follows_the_state_machine() {
    let mut h = H::new();
    h.tick();
    let start = h.clock.now();
    assert_eq!(h.lc.next_deadline(), Some(start.plus_ms(28 * MIN)));
    h.at(28 * MIN);
    assert_eq!(h.lc.next_deadline(), Some(start.plus_ms(30 * MIN)));
    h.at(2 * MIN);
    assert_eq!(h.lc.next_deadline(), Some(h.clock.now().plus_ms(5000)));
}
