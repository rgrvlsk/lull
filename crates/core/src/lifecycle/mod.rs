//! Stop state machine, activity merge, history and restart (contract §5–§7).
//!
//! `running → stopping-soon → stopping → (removed)`. Lifecycle is the only owner of the
//! per-server record: every `ActivityEvent` is merged here, every signal is decided and
//! re-checked here, and `state.json` is (de)serialised here.

use std::collections::BTreeMap;
use std::fmt;
use std::io;

use lull_protocol::{Config, RestartStatus};
use serde::{Deserialize, Serialize};

use crate::model::{
    Activity, ActivityEvent, Discovery, EffectivePolicy, KnownGroup, ObservedServer, PendingStop,
    ProcessInfo, ServerId, ServerRecord, ServerState, StopReason, StoppedServer, Witness, rfc3339,
};
use crate::policy;
use crate::traits::{Instant, ProcessSource, Signal, Signaler, SpawnRequest, Spawner, StateStore};

const SIGKILL_AFTER_MS: u64 = 5_000;
const STUCK_AFTER_MS: u64 = 5_000;
const RESTART_WINDOW_MS: u64 = 60_000;
const STATE_VERSION: u64 = 1;
/// Aborted stops kept for `lull doctor`.
const DOCTOR_NOTES: usize = 20;

/// Inputs of one scan.
pub struct Tick<'a> {
    pub now: Instant,
    pub wall_ms: u64,
    /// This scan, taken immediately before the tick.
    pub discovery: &'a Discovery,
    /// Traffic/tab events of this scan.
    pub events: Vec<ActivityEvent>,
    pub config: &'a Config,
    pub home: &'a str,
    /// Wake/start grace or backend switch: no automatic stop may start.
    pub hold_stops: bool,
    pub dry_run: bool,
    /// `~/Library/Logs/lull` (restart logs live here).
    pub log_dir: &'a str,
}

/// Platform access for the signal path.
pub struct Sys<'a> {
    pub signaler: &'a mut dyn Signaler,
    /// Fresh `group(pgid)` re-check before every signal.
    pub source: &'a mut dyn ProcessSource,
    /// Discovery's §4.4 protected-group test.
    pub protected: &'a dyn Fn(&[ProcessInfo]) -> bool,
    pub uid: u32,
    pub daemon_pgid: u32,
}

/// What the daemon must broadcast or log after a tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Removed {
        id: ServerId,
        reason: StopReason,
    },
    StopAborted {
        id: ServerId,
    },
    StopStuck {
        id: ServerId,
    },
    RestartSucceeded {
        record_id: ServerId,
        server_id: ServerId,
    },
    RestartFailed {
        record_id: ServerId,
    },
    /// A line for `lull.log`.
    Log(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    UnknownId,
    /// The record has no launch command (`-32011 not_restartable`).
    NotRestartable,
    AlreadyRestarting,
    /// The spawn itself failed; the record is now `restartFailed`.
    SpawnFailed(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId => f.write_str("unknown id"),
            Self::NotRestartable => f.write_str("not_restartable: no launch command"),
            Self::AlreadyRestarting => f.write_str("already restarting"),
            Self::SpawnFailed(err) => write!(f, "restart failed: {err}"),
        }
    }
}

impl std::error::Error for LifecycleError {}

/// A stopped-server record as persisted: wall ms for retention, the restart instant for
/// the 60 s window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    server: StoppedServer,
    stopped_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restart_started: Option<Instant>,
}

impl HistoryEntry {
    fn restart_window_open(&self, now: &Instant) -> bool {
        self.restart_started
            .as_ref()
            .is_some_and(|start| now.ms_since(start).is_some_and(|d| d < RESTART_WINDOW_MS))
    }
}

/// `state.json`, version 1.
#[derive(Serialize, Deserialize)]
struct StateFile {
    version: u64,
    servers: BTreeMap<ServerId, ServerRecord>,
    #[serde(default)]
    history: Vec<HistoryEntry>,
}

#[derive(Debug)]
struct Entry {
    record: ServerRecord,
    /// Last observation, classification kept across incomplete scans (§4.8); `None` for
    /// a record restored from `state.json` and not matched yet.
    observed: Option<ObservedServer>,
    /// Policy from config alone (no per-server override), as of the last tick.
    base: EffectivePolicy,
    warn_ms: u64,
    manual_stop: bool,
    stuck: bool,
    /// Last deduplicated log line, so a standing decision is not logged every tick.
    last_log: Option<String>,
}

impl Entry {
    fn new(record: ServerRecord) -> Self {
        Self {
            record,
            observed: None,
            base: EffectivePolicy {
                idle_minutes: Config::default().idle_minutes,
                auto_stop: false,
            },
            warn_ms: 0,
            manual_stop: false,
            stuck: false,
            last_log: None,
        }
    }

    fn policy(&self) -> EffectivePolicy {
        EffectivePolicy {
            idle_minutes: self.base.idle_minutes,
            auto_stop: self.record.auto_stop.unwrap_or(self.base.auto_stop),
        }
    }

    fn idle_ms(&self) -> u64 {
        minutes(self.policy().idle_minutes)
    }

    fn warn_at(&self) -> Instant {
        let lead = self.idle_ms().saturating_sub(self.warn_ms);
        self.record.last_activity.plus_ms(lead)
    }

    /// Recomputes `stops_at` and `state` from the record, the policy and `now`.
    fn refresh(&mut self, now: &Instant) {
        if self.record.stop.is_some() {
            self.record.state = ServerState::Stopping;
            return;
        }
        let listening = self.observed.as_ref().is_none_or(|o| !o.ports.is_empty());
        let eligible = self.policy().auto_stop && !self.record.pinned && listening;
        self.record.stops_at = eligible.then(|| self.record.last_activity.plus_ms(self.idle_ms()));
        self.record.state = if !listening {
            ServerState::NoListener
        } else if eligible && reached(now, &self.warn_at()) {
            ServerState::StoppingSoon
        } else {
            ServerState::Running
        };
    }

    /// Merges one activity; never moves the instant backwards.
    fn touch(&mut self, at: Instant, source: Activity, wall_ms: u64) {
        if self.record.last_activity.ms_since(&at) == Some(0)
            || at.ms_since(&self.record.last_activity).is_none()
        {
            self.record.last_activity = at;
            self.record.last_activity_wall_ms = wall_ms;
            self.record.last_activity_source = source;
        }
    }

    fn log_once(&mut self, line: String, fx: &mut Vec<Effect>) {
        if self.last_log.as_ref() != Some(&line) {
            fx.push(Effect::Log(line.clone()));
            self.last_log = Some(line);
        }
    }
}

fn minutes(m: u32) -> u64 {
    u64::from(m) * 60_000
}

/// `at` is not after `now` (same boot session only).
fn reached(now: &Instant, at: &Instant) -> bool {
    at.ms_since(now) == Some(0)
}

/// `a` is strictly before `b` (same boot session only).
fn before(a: &Instant, b: &Instant) -> bool {
    b.ms_since(a).is_some_and(|d| d > 0)
}

fn elapsed(now: &Instant, since: &Instant, ms: u64) -> bool {
    now.ms_since(since).is_some_and(|d| d >= ms)
}

fn reason_name(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Idle => "idle",
        StopReason::Manual => "manual",
        StopReason::Exited => "exited",
    }
}

/// Keeps what a scan cannot be trusted to recompute: the launch command captured when the
/// server was first seen, and the whole classification when this scan is incomplete.
fn merge(prev: Option<&ObservedServer>, mut next: ObservedServer) -> ObservedServer {
    let Some(prev) = prev else { return next };
    if !prev.launch_argv.is_empty() {
        next.launch_argv.clone_from(&prev.launch_argv);
        next.command.clone_from(&prev.command);
        next.restart_env.clone_from(&prev.restart_env);
    }
    if !next.complete() {
        next.origin = prev.origin.clone();
        next.framework = prev.framework;
        next.project.clone_from(&prev.project);
        next.cwd.clone_from(&prev.cwd);
    }
    next
}

/// Why a signal must not be sent now.
enum Refusal {
    /// The member list could not be read: retry on the next tick.
    Unreadable(String),
    /// The group has no members left: removal follows from the next scan.
    Empty,
    /// Protected, other uid, pgid ≤ 1 or the daemon's own group.
    Unsafe(String),
    /// No member recorded at decision time is still in the group.
    NoWitness,
}

/// The re-check of §7 immediately before every signal.
fn check_group(
    sys: &mut Sys,
    discovery: &Discovery,
    pgid: u32,
    witnesses: &[Witness],
) -> Result<(), Refusal> {
    if pgid <= 1 || pgid == sys.daemon_pgid || discovery.protected.contains(&pgid) {
        return Err(Refusal::Unsafe(format!("pgid {pgid} is never signalled")));
    }
    let members = sys
        .source
        .group(pgid)
        .map_err(|err| Refusal::Unreadable(err.to_string()))?;
    if members.is_empty() {
        return Err(Refusal::Empty);
    }
    if let Some(p) = members.iter().find(|p| p.uid != sys.uid) {
        return Err(Refusal::Unsafe(format!(
            "member {} has uid {}",
            p.pid, p.uid
        )));
    }
    if (sys.protected)(&members) {
        return Err(Refusal::Unsafe("the group is protected".into()));
    }
    if !members.iter().any(|p| witnesses.contains(&p.witness())) {
        return Err(Refusal::NoWitness);
    }
    Ok(())
}

/// The lifecycle of every live server plus the stopped-server history.
#[derive(Debug, Default)]
pub struct Lifecycle {
    entries: BTreeMap<ServerId, Entry>,
    /// Newest first.
    history: Vec<HistoryEntry>,
    /// Aborted stops, for `lull doctor`.
    notes: Vec<String>,
    /// Effects produced outside a tick (a failed spawn), emitted by the next tick.
    queued: Vec<Effect>,
    /// `now` of the last tick.
    now: Option<Instant>,
    hold: bool,
    ticked: bool,
    last_saved: Option<serde_json::Value>,
}

impl Lifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restores `state.json`. A corrupt or unknown-version file is quarantined and the
    /// lifecycle starts fresh; the returned warnings say so.
    pub fn load(store: &mut dyn StateStore, now: &Instant) -> (Self, Vec<String>) {
        let mut lc = Self::new();
        let mut warnings = Vec::new();
        let value = match store.load() {
            Ok(Some(value)) => value,
            Ok(None) => return (lc, warnings),
            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                quarantine(
                    store,
                    &mut warnings,
                    &format!("state.json is corrupt ({err})"),
                );
                return (lc, warnings);
            }
            Err(err) => {
                warnings.push(format!("cannot read state.json ({err}); starting fresh"));
                return (lc, warnings);
            }
        };
        let file = match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(STATE_VERSION) => {
                serde_json::from_value::<StateFile>(value.clone()).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown state.json version {other:?}")),
        };
        let file = match file {
            Ok(file) => file,
            Err(why) => {
                quarantine(store, &mut warnings, &why);
                return (lc, warnings);
            }
        };
        for (id, mut record) in file.servers {
            // An instant from another boot session cannot be compared: it counts as now.
            if now.ms_since(&record.last_activity).is_none() {
                record.last_activity = now.clone();
            }
            record.id.clone_from(&id);
            lc.entries.insert(id, Entry::new(record));
        }
        lc.history = file.history;
        lc.last_saved = Some(value);
        (lc, warnings)
    }

    /// Writes `state.json` when its content changed since the last save or load.
    pub fn save(&mut self, store: &mut dyn StateStore) -> io::Result<()> {
        let file = StateFile {
            version: STATE_VERSION,
            servers: self
                .entries
                .iter()
                .map(|(id, e)| (id.clone(), e.record.clone()))
                .collect(),
            history: self.history.clone(),
        };
        let value = serde_json::to_value(&file).map_err(io::Error::other)?;
        if self.last_saved.as_ref() != Some(&value) {
            store.save(&value)?;
            self.last_saved = Some(value);
        }
        Ok(())
    }

    /// Identities for discovery's `resume` on the first scan (§4.7).
    pub fn known_groups(&self) -> Vec<KnownGroup> {
        self.entries
            .values()
            .map(|e| KnownGroup {
                id: e.record.id.clone(),
                pgid: e.record.pgid,
                witnesses: e.record.witnesses.clone(),
            })
            .collect()
    }

    pub fn tick(&mut self, t: Tick, sys: &mut Sys) -> Vec<Effect> {
        let Tick {
            now,
            wall_ms,
            discovery,
            events,
            config,
            ..
        } = &t;
        let mut fx = std::mem::take(&mut self.queued);
        self.now = Some(now.clone());
        self.hold = t.hold_stops;

        // A server is removed only once its pgid has no members, i.e. discovery no
        // longer lists it.
        let gone: Vec<ServerId> = self
            .entries
            .keys()
            .filter(|id| !discovery.servers.iter().any(|s| &s.id == *id))
            .cloned()
            .collect();
        for id in gone {
            let entry = self.entries.remove(&id).expect("listed above");
            // Restored from state.json and not resumed: dropped silently.
            let Some(obs) = entry.observed else { continue };
            let reason = entry.record.stop.map_or(StopReason::Exited, |s| s.reason);
            fx.push(Effect::Log(format!(
                "{id} ({}, pgid {}) removed: {}",
                obs.project,
                obs.pgid,
                reason_name(reason)
            )));
            fx.push(Effect::Removed {
                id: id.clone(),
                reason,
            });
            self.history.insert(
                0,
                HistoryEntry {
                    server: StoppedServer {
                        id,
                        project: obs.project,
                        framework: obs.framework,
                        command: obs.command,
                        argv: obs.launch_argv,
                        cwd: obs.cwd,
                        ports: obs.ports,
                        origin: obs.origin,
                        stopped_at: rfc3339(*wall_ms),
                        reason,
                        restart_env: obs.restart_env,
                        tabs: Vec::new(),
                        status: RestartStatus::Stopped,
                    },
                    stopped_at_ms: *wall_ms,
                    restart_started: None,
                },
            );
        }

        for s in &discovery.servers {
            let entry = match self.entries.get_mut(&s.id) {
                Some(entry) => {
                    entry.record.pgid = s.pgid;
                    entry.record.witnesses.clone_from(&s.members);
                    entry.observed = Some(merge(entry.observed.as_ref(), s.clone()));
                    entry
                }
                None => {
                    if !s.ports.is_empty() {
                        self.match_restart(s, now, &mut fx);
                    }
                    let mut entry = Entry::new(ServerRecord {
                        id: s.id.clone(),
                        pgid: s.pgid,
                        witnesses: s.members.clone(),
                        last_activity: now.clone(),
                        last_activity_wall_ms: *wall_ms,
                        last_activity_source: Activity::Started,
                        pinned: s.env_pin,
                        auto_stop: None,
                        state: ServerState::Running,
                        stops_at: None,
                        stop: None,
                    });
                    entry.observed = Some(s.clone());
                    self.entries.entry(s.id.clone()).or_insert(entry)
                }
            };
            let obs = entry.observed.as_ref().expect("set above");
            entry.base = policy::effective(config, obs, None, t.home);
            entry.warn_ms = minutes(config.warn_before_minutes);
        }

        for ev in events.iter().cloned() {
            self.activity(ev, *wall_ms);
        }

        let ids: Vec<ServerId> = self.entries.keys().cloned().collect();
        for id in &ids {
            self.step(id, &t, sys, &mut fx);
        }

        for h in &mut self.history {
            if h.server.status == RestartStatus::Restarting && !h.restart_window_open(now) {
                h.server.status = RestartStatus::RestartFailed;
                h.restart_started = None;
                fx.push(Effect::Log(format!(
                    "restart of {} failed: no server listening in {} within 60 s; see {}/restart-{}.log",
                    h.server.id, h.server.cwd, t.log_dir, h.server.id
                )));
                fx.push(Effect::RestartFailed {
                    record_id: h.server.id.clone(),
                });
            }
        }
        self.prune(config, *wall_ms);
        self.ticked = true;
        fx
    }

    /// A new server listening in the cwd of a `restarting` record completes that restart.
    fn match_restart(&mut self, s: &ObservedServer, now: &Instant, fx: &mut Vec<Effect>) {
        let found = self.history.iter().position(|h| {
            h.server.status == RestartStatus::Restarting
                && h.server.cwd == s.cwd
                && h.restart_window_open(now)
        });
        if let Some(pos) = found {
            let h = self.history.remove(pos);
            fx.push(Effect::Log(format!(
                "restart of {} succeeded as {}",
                h.server.id, s.id
            )));
            fx.push(Effect::RestartSucceeded {
                record_id: h.server.id,
                server_id: s.id.clone(),
            });
        }
    }

    fn prune(&mut self, config: &Config, wall_ms: u64) {
        let max_age = u64::from(config.history_hours) * 3_600_000;
        self.history
            .retain(|h| wall_ms.saturating_sub(h.stopped_at_ms) <= max_age);
        self.history.truncate(config.history_size as usize);
    }

    /// One server's stop decision for this tick.
    fn step(&mut self, id: &str, t: &Tick, sys: &mut Sys, fx: &mut Vec<Effect>) {
        let ticked = self.ticked;
        let Some(e) = self.entries.get_mut(id) else {
            return;
        };
        e.refresh(&t.now);
        let Some(obs) = e.observed.as_ref() else {
            return;
        };
        let (pgid, members, incomplete) = (obs.pgid, obs.members.clone(), obs.incomplete.clone());
        if e.record.stop.is_some() {
            Self::progress(e, &mut self.notes, t, sys, fx);
            return;
        }
        let due = e
            .record
            .stops_at
            .as_ref()
            .is_some_and(|at| reached(&t.now, at));
        if !e.manual_stop && !due {
            e.last_log = None;
            return;
        }
        let reason = if e.manual_stop {
            StopReason::Manual
        } else {
            StopReason::Idle
        };
        if reason == StopReason::Idle {
            // ponytail: the "fresh observation" of §7.2 is the scan taken immediately
            // before this tick (`Tick::discovery`); only the member list is re-read
            // below, right before the signal. Ceiling: one scan interval (5 s) between
            // the classification and the signal. Upgrade: re-run discovery on
            // `sys.source.snapshot(&[pgid])` here if ticks drift away from scans.
            let wait = if !incomplete.is_empty() {
                Some(format!(
                    "incomplete observation ({})",
                    incomplete.join("; ")
                ))
            } else if t.hold_stops {
                Some("wake/start grace or backend switch".to_owned())
            } else if !ticked {
                Some("first scan".to_owned())
            } else {
                None
            };
            if let Some(why) = wait {
                e.log_once(format!("{id}: idle stop deferred: {why}"), fx);
                return;
            }
        }
        match check_group(sys, t.discovery, pgid, &members) {
            Ok(()) => {}
            Err(Refusal::NoWitness) => {
                let why =
                    format!("pgid {pgid} was reused: no member recorded at decision time is left");
                Self::abort(e, &mut self.notes, t, &why, fx);
                return;
            }
            Err(Refusal::Empty) => {
                e.manual_stop = false;
                e.log_once(format!("{id}: pgid {pgid} already has no members"), fx);
                return;
            }
            Err(Refusal::Unreadable(err)) => {
                e.log_once(
                    format!("{id}: stop deferred, cannot read pgid {pgid}: {err}"),
                    fx,
                );
                return;
            }
            Err(Refusal::Unsafe(why)) => {
                e.manual_stop = false;
                e.log_once(format!("{id}: refusing to signal: {why}"), fx);
                return;
            }
        }
        if t.dry_run {
            let line = format!(
                "{id}: dry-run, would send SIGTERM to pgid {pgid} ({})",
                reason_name(reason)
            );
            if e.manual_stop {
                e.manual_stop = false;
                fx.push(Effect::Log(line));
            } else {
                e.log_once(line, fx);
            }
            return;
        }
        match sys.signaler.signal_group(pgid, Signal::Term) {
            Ok(()) => {
                e.manual_stop = false;
                e.last_log = None;
                e.record.stop = Some(PendingStop {
                    reason,
                    witnesses: members,
                    sigterm_at: t.now.clone(),
                    sigkill_at: None,
                });
                e.record.state = ServerState::Stopping;
                fx.push(Effect::Log(format!(
                    "{id}: SIGTERM sent to pgid {pgid} ({})",
                    reason_name(reason)
                )));
            }
            Err(err) => e.log_once(format!("{id}: SIGTERM to pgid {pgid} failed: {err}"), fx),
        }
    }

    /// After `SIGTERM`: `SIGKILL` at 5 s, `stuck` 5 s after that.
    fn progress(
        e: &mut Entry,
        notes: &mut Vec<String>,
        t: &Tick,
        sys: &mut Sys,
        fx: &mut Vec<Effect>,
    ) {
        let id = e.record.id.clone();
        let pgid = e.record.pgid;
        let stop = e.record.stop.clone().expect("caller checked");
        match &stop.sigkill_at {
            None if elapsed(&t.now, &stop.sigterm_at, SIGKILL_AFTER_MS) => {
                match check_group(sys, t.discovery, pgid, &stop.witnesses) {
                    Ok(()) => match sys.signaler.signal_group(pgid, Signal::Kill) {
                        Ok(()) => {
                            if let Some(s) = e.record.stop.as_mut() {
                                s.sigkill_at = Some(t.now.clone());
                            }
                            fx.push(Effect::Log(format!("{id}: SIGKILL sent to pgid {pgid}")));
                        }
                        Err(err) => {
                            e.log_once(format!("{id}: SIGKILL to pgid {pgid} failed: {err}"), fx)
                        }
                    },
                    // Removed once the scan no longer lists the group.
                    Err(Refusal::Empty) => {}
                    Err(Refusal::NoWitness) => {
                        let why = "every member recorded at decision time exited but the group still has members";
                        Self::abort(e, notes, t, why, fx);
                    }
                    Err(Refusal::Unreadable(why) | Refusal::Unsafe(why)) => {
                        e.log_once(format!("{id}: SIGKILL withheld: {why}"), fx)
                    }
                }
            }
            Some(kill_at) if !e.stuck && elapsed(&t.now, kill_at, STUCK_AFTER_MS) => {
                e.stuck = true;
                fx.push(Effect::Log(format!(
                    "{id}: stop stuck, pgid {pgid} still has members 5 s after SIGKILL"
                )));
                fx.push(Effect::StopStuck { id });
            }
            _ => {}
        }
    }

    /// The stop is abandoned: back to `running` with `manual` activity (§7.3).
    fn abort(e: &mut Entry, notes: &mut Vec<String>, t: &Tick, why: &str, fx: &mut Vec<Effect>) {
        e.record.stop = None;
        e.manual_stop = false;
        e.stuck = false;
        e.last_log = None;
        e.touch(t.now.clone(), Activity::Manual, t.wall_ms);
        e.refresh(&t.now);
        let line = format!(
            "{}: stop of {} (pgid {}) aborted: {why}",
            rfc3339(t.wall_ms),
            e.record.id,
            e.record.pgid
        );
        notes.push(line.clone());
        if notes.len() > DOCTOR_NOTES {
            notes.remove(0);
        }
        fx.push(Effect::Log(line));
        fx.push(Effect::StopAborted {
            id: e.record.id.clone(),
        });
    }

    pub fn record(&self, id: &str) -> Option<&ServerRecord> {
        self.entries.get(id).map(|e| &e.record)
    }

    /// The last observation with the classification kept across incomplete scans and the
    /// launch command captured when first seen; build the wire `Server` from this.
    pub fn observed(&self, id: &str) -> Option<&ObservedServer> {
        self.entries.get(id).and_then(|e| e.observed.as_ref())
    }

    pub fn policy(&self, server: &ObservedServer, config: &Config, home: &str) -> EffectivePolicy {
        let over = self
            .entries
            .get(&server.id)
            .and_then(|e| e.record.auto_stop);
        policy::effective(config, server, over, home)
    }

    /// The sole merge point of every `ActivityEvent`.
    pub fn activity(&mut self, ev: ActivityEvent, wall_ms: u64) {
        if let Some(e) = self.entries.get_mut(&ev.server_id) {
            let at = ev.at.clone();
            e.touch(ev.at, ev.source, wall_ms);
            e.refresh(&at);
        }
    }

    /// Unpin counts as `manual` activity (§5). An explicit change overrides `LULL_KEEP`.
    pub fn pin(
        &mut self,
        id: &str,
        pinned: bool,
        now: &Instant,
        wall_ms: u64,
    ) -> Result<(), LifecycleError> {
        let e = self.entries.get_mut(id).ok_or(LifecycleError::UnknownId)?;
        e.record.pinned = pinned;
        if !pinned {
            e.touch(now.clone(), Activity::Manual, wall_ms);
        }
        e.refresh(now);
        Ok(())
    }

    pub fn set_auto_stop(
        &mut self,
        id: &str,
        auto_stop: Option<bool>,
    ) -> Result<(), LifecycleError> {
        let e = self.entries.get_mut(id).ok_or(LifecycleError::UnknownId)?;
        e.record.auto_stop = auto_stop;
        if let Some(now) = &self.now {
            e.refresh(now);
        }
        Ok(())
    }

    /// Manual stop, executed by the next tick: skips `stopping-soon`, coalesced with any
    /// stop already requested or under way.
    pub fn request_stop(&mut self, id: &str) -> Result<(), LifecycleError> {
        let e = self.entries.get_mut(id).ok_or(LifecycleError::UnknownId)?;
        if e.record.stop.is_none() {
            e.manual_stop = true;
        }
        Ok(())
    }

    /// Stopped-server records, newest first. `tabs` is filled by the browser registry.
    pub fn history(&self) -> Vec<StoppedServer> {
        self.history.iter().map(|h| h.server.clone()).collect()
    }

    /// Re-runs a stopped server's launch command (ADR-0005); returns the new pid.
    pub fn restart(
        &mut self,
        record_id: &str,
        spawner: &mut dyn Spawner,
        log_dir: &str,
        now: &Instant,
    ) -> Result<u32, LifecycleError> {
        let h = self
            .history
            .iter_mut()
            .find(|h| h.server.id == record_id)
            .ok_or(LifecycleError::UnknownId)?;
        if h.server.argv.is_empty() {
            return Err(LifecycleError::NotRestartable);
        }
        if h.server.status == RestartStatus::Restarting {
            return Err(LifecycleError::AlreadyRestarting);
        }
        let request = SpawnRequest {
            argv: h.server.argv.clone(),
            cwd: h.server.cwd.clone(),
            env: h.server.restart_env.clone(),
            log_path: format!("{log_dir}/restart-{record_id}.log"),
        };
        match spawner.spawn(&request) {
            Ok(pid) => {
                h.server.status = RestartStatus::Restarting;
                h.restart_started = Some(now.clone());
                Ok(pid)
            }
            Err(err) => {
                h.server.status = RestartStatus::RestartFailed;
                h.restart_started = None;
                self.queued.push(Effect::Log(format!(
                    "restart of {record_id} failed to spawn: {err}"
                )));
                self.queued.push(Effect::RestartFailed {
                    record_id: record_id.to_owned(),
                });
                Err(LifecycleError::SpawnFailed(err.to_string()))
            }
        }
    }

    pub fn forget(&mut self, record_id: &str) {
        self.history.retain(|h| h.server.id != record_id);
    }

    /// The earliest future deadline (stopping-soon, `stopsAt`, `SIGKILL`, stuck check,
    /// restart timeout), so the daemon can tick before the next regular scan.
    pub fn next_deadline(&self) -> Option<Instant> {
        let mut best: Option<Instant> = None;
        let mut offer = |at: Instant| {
            let past = self.now.as_ref().is_some_and(|now| reached(now, &at));
            if !past && best.as_ref().is_none_or(|b| before(&at, b)) {
                best = Some(at);
            }
        };
        for e in self.entries.values() {
            match &e.record.stop {
                Some(stop) => match &stop.sigkill_at {
                    None => offer(stop.sigterm_at.plus_ms(SIGKILL_AFTER_MS)),
                    Some(kill_at) if !e.stuck => offer(kill_at.plus_ms(STUCK_AFTER_MS)),
                    Some(_) => {}
                },
                None => {
                    if let Some(at) = &e.record.stops_at {
                        offer(e.warn_at());
                        offer(at.clone());
                    }
                }
            }
        }
        for h in &self.history {
            if let Some(start) = &h.restart_started {
                offer(start.plus_ms(RESTART_WINDOW_MS));
            }
        }
        best
    }

    /// Why a server would not be stopped automatically now; `None` when it would be once
    /// idle (`lull doctor`).
    pub fn ineligible_reason(&self, id: &str) -> Option<String> {
        let Some(e) = self.entries.get(id) else {
            return Some("unknown server".into());
        };
        let Some(obs) = &e.observed else {
            return Some("not observed yet".into());
        };
        let reason = if e.record.stop.is_some() {
            "a stop is already under way".to_owned()
        } else if e.record.pinned {
            "pinned".to_owned()
        } else if e.record.auto_stop == Some(false) {
            "auto-stop turned off for this server".to_owned()
        } else if !e.policy().auto_stop {
            format!(
                "auto_stop policy excludes it (origin {:?})",
                obs.origin.kind
            )
        } else if obs.ports.is_empty() {
            "no listener".to_owned()
        } else if !obs.complete() {
            format!("incomplete observation ({})", obs.incomplete.join("; "))
        } else if self.hold {
            "wake/start grace or backend switch".to_owned()
        } else {
            return None;
        };
        Some(reason)
    }

    /// Aborted and stuck stops.
    pub fn doctor(&self) -> Vec<String> {
        let stuck = self.entries.values().filter(|e| e.stuck).map(|e| {
            format!(
                "stop of {} (pgid {}) is stuck: members survived SIGKILL",
                e.record.id, e.record.pgid
            )
        });
        self.notes.iter().cloned().chain(stuck).collect()
    }
}

fn quarantine(store: &mut dyn StateStore, warnings: &mut Vec<String>, why: &str) {
    warnings.push(match store.quarantine() {
        Ok(dest) => format!("{why}; moved to {dest}, starting fresh"),
        Err(err) => format!("{why}; could not move it aside ({err}), starting fresh"),
    });
}

#[cfg(test)]
mod tests;
