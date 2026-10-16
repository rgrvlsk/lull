//! Domain types shared by discovery, activity, lifecycle, the browser registry and the
//! daemon (M0.5). Types only: every work package names these at its boundaries instead of
//! inventing its own. Wire shapes stay in `lull-protocol`; these are the in-process ones.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use lull_protocol::server::{Activity, Framework, Origin, ServerState};
pub use lull_protocol::{StopReason, StoppedServer};

use crate::traits::Instant;

/// `"{pgid}-{oldestMemberStartEpochMs}"` (§4.6).
pub type ServerId = String;

// ───────────────────────── snapshot (ProcessSource → discovery, activity) ─────────────────

/// One process as observed in a scan (§4.2). Fields that could not be read are `None` and
/// the failure is described in `errors`, so discovery can apply §4.8.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub pgid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub start_epoch_ms: u64,
    pub exe: Option<String>,
    /// Executable of the parent process (§4.4 "a direct child of" the T3 Code app).
    pub parent_exe: Option<String>,
    pub argv: Option<Vec<String>>,
    /// Initial environment; `None` when unreadable.
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<String>,
    /// Whether the process has a controlling tty (ADR-0001 veto).
    pub has_tty: Option<bool>,
    /// Display only (§5): CPU share since the previous scan.
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    pub errors: Vec<String>,
}

impl ProcessInfo {
    pub fn witness(&self) -> Witness {
        Witness {
            pid: self.pid,
            start_epoch_ms: self.start_epoch_ms,
        }
    }
}

/// A same-uid TCP listening socket (v4 or v6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Listener {
    pub pid: u32,
    pub addr: String,
    pub port: u16,
}

/// An established TCP connection to a same-uid listening port, seen from the server side.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// The listening port the client connected to.
    pub port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    /// Same-uid process holding the client end; `None` for remote or other-uid clients.
    pub client_pid: Option<u32>,
    /// `client_pid` followed by its ancestors up to (excluding) launchd; empty when
    /// `client_pid` is `None`. Used to decide whether a covered browser owns it (§5).
    pub client_ancestry: Vec<u32>,
}

/// One scan (§4.1–4.2): every same-uid listening group plus any requested pgids. Sockets
/// come from one system-wide read; if it fails the whole snapshot fails, so there is no
/// per-member socket failure (§4.8). Its
/// JSON form is also the `lull debug snapshot` fixture format (plan, "Fixtures").
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    /// Every member of every group that has a same-uid listener, plus every member of the
    /// groups the caller asked for (known servers that may have lost their listener).
    pub processes: Vec<ProcessInfo>,
    pub listeners: Vec<Listener>,
    pub connections: Vec<Connection>,
    /// Groups whose member list could not be read in this scan (§4.8).
    pub unreadable_groups: BTreeSet<u32>,
}

// ───────────────────────── discovery output ─────────────────────────

/// A process recorded by pid and start time; proves a pgid was not reused (§4.7, §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Witness {
    pub pid: u32,
    pub start_epoch_ms: u64,
}

/// A server as discovered in one scan (§4.3–4.6, §7 launch command).
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedServer {
    pub id: ServerId,
    pub pgid: u32,
    /// Every member in this scan, sorted by pid.
    pub members: Vec<Witness>,
    /// Sorted, deduplicated listening ports; empty means `no-listener` (§4.8).
    pub ports: Vec<u16>,
    pub primary_url: Option<String>,
    /// cwd of the listening member that made the group a server.
    pub cwd: String,
    pub project: String,
    pub framework: Framework,
    pub origin: Origin,
    /// Shell-quoted launch command, else the oldest member's command line (§7.6).
    pub command: String,
    /// The launch command captured when first seen; empty when none was found.
    pub launch_argv: Vec<String>,
    /// Only `PORT`, `HOST`, `NODE_ENV` (§7.6).
    pub restart_env: BTreeMap<String, String>,
    /// Some member has `LULL_KEEP=1` (§6): the server starts pinned.
    pub env_pin: bool,
    pub started_at_ms: u64,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    /// Why this scan's observation is incomplete (§4.8); empty means complete.
    pub incomplete: Vec<String>,
}

impl ObservedServer {
    pub fn complete(&self) -> bool {
        self.incomplete.is_empty()
    }
}

/// A listening group that is not a server (§4.3): shown by `lull ls --all`, never acted on.
#[derive(Debug, Clone, PartialEq)]
pub struct UnmanagedListener {
    pub pgid: u32,
    pub pids: Vec<u32>,
    pub ports: Vec<u16>,
    pub cwd: Option<String>,
    pub command: String,
}

/// Result of one discovery pass.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Discovery {
    pub servers: Vec<ObservedServer>,
    pub unmanaged: Vec<UnmanagedListener>,
    /// Protected pgids (§4.4): never listed, never signalled.
    pub protected: BTreeSet<u32>,
}

/// An identity restored from `state.json` on daemon start (§4.7): resumed only when the
/// pgid matches and at least one witness is still a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownGroup {
    pub id: ServerId,
    pub pgid: u32,
    pub witnesses: Vec<Witness>,
}

// ───────────────────────── activity ─────────────────────────

/// What a traffic observation saw on a connection (§5 `traffic`, pulses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionKind {
    /// A new connection (§5 `traffic` (a)), emitted by an [`crate::ActivityBackend`].
    New,
    /// A connection held open at a scan (§5 `traffic` (b) in the scan fallback), derived
    /// by activity from `Snapshot::connections`.
    Established,
    /// Bytes received plus sent on one connection since its previous reading, emitted by
    /// a backend that reads byte counters.
    Bytes(u64),
    /// A new connection closed before its byte counters were read.
    Unread,
}

/// Raw traffic seen by an activity backend, before it is resolved to a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficEvent {
    /// The server-side (listening) port.
    pub port: u16,
    /// Same-uid client process, `None` when remote or unknown (counts as outside).
    pub client_pid: Option<u32>,
    pub kind: ConnectionKind,
    /// Receipt time.
    pub at: Instant,
}

/// Activity attributed to a server. Produced by activity (traffic), the browser registry
/// (tab) and the daemon (manual); merged into the lifecycle record by lifecycle alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEvent {
    pub server_id: ServerId,
    /// `Tab`, `Traffic` or `Manual`; `Started` is set by lifecycle itself.
    pub source: Activity,
    /// The daemon's receipt time, never the sender's (§5).
    pub at: Instant,
}

/// Health of an activity backend, so failure is distinguishable from silence (§5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendHealth {
    Healthy,
    Failed(String),
}

/// Browser processes with a live extension connection (§5 "Covered browser"). A client
/// pid is covered when it or one of its ancestors is in the set. Empty throughout M1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoveredBrowsers(pub BTreeSet<u32>);

impl CoveredBrowsers {
    pub fn covers(&self, ancestry: &[u32]) -> bool {
        ancestry.iter().any(|pid| self.0.contains(pid))
    }
}

// ───────────────────────── policy and lifecycle ─────────────────────────

/// Effective policy of one server (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectivePolicy {
    pub idle_minutes: u32,
    pub auto_stop: bool,
}

/// Per-server record owned by lifecycle and persisted in `state.json` (§4.7). Fields after
/// `auto_stop` are runtime-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRecord {
    pub id: ServerId,
    pub pgid: u32,
    pub witnesses: Vec<Witness>,
    /// Awake-time instant of the last activity (§5 "Clock").
    pub last_activity: Instant,
    /// Wall clock of the same moment, display only.
    pub last_activity_wall_ms: u64,
    pub last_activity_source: Activity,
    /// Starts as `ObservedServer::env_pin`, then only explicit pin/unpin changes it (§6).
    pub pinned: bool,
    /// The pin came from `LULL_KEEP=1`, not from a pin request; cleared by any explicit
    /// pin or unpin. Absent from state files written before protocol 1.3.
    #[serde(default)]
    pub pinned_by_env: bool,
    /// Per-server override; `None` follows the policy (§6).
    pub auto_stop: Option<bool>,
    #[serde(skip, default = "running")]
    pub state: ServerState,
    /// Awake-time deadline; `None` when not auto-stoppable.
    #[serde(skip)]
    pub stops_at: Option<Instant>,
    #[serde(skip)]
    pub stop: Option<PendingStop>,
}

fn running() -> ServerState {
    ServerState::Running
}

/// A stop in progress: `SIGTERM` has been sent (§7.2–7.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStop {
    pub reason: StopReason,
    /// Members recorded at decision time; the witness check before `SIGKILL` uses them.
    pub witnesses: Vec<Witness>,
    pub sigterm_at: Instant,
    pub sigkill_at: Option<Instant>,
}

/// How a stop ended (§7.3–7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopOutcome {
    /// The group emptied; the server is removed.
    Stopped,
    /// Every recorded member exited but the group still has members: no `SIGKILL`.
    Aborted,
    /// Members survive 5 s after `SIGKILL`; stays `stopping`.
    Stuck,
}

// ───────────────────────── helpers ─────────────────────────

/// Length of one policy minute (`idle_minutes`, `warn_before_minutes`,
/// `wake_grace_minutes`) in ms.
// Test seam: in debug builds `LULL_MINUTE_MS` lets the e2e suite run "minutes" in
// seconds against a real daemon. Release builds always use 60 000.
pub fn minute_ms() -> u64 {
    static MINUTE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MINUTE.get_or_init(|| {
        let test = cfg!(debug_assertions)
            .then(|| std::env::var("LULL_MINUTE_MS").ok()?.parse().ok())
            .flatten();
        test.filter(|&ms| ms > 0).unwrap_or(60_000)
    })
}

/// RFC 3339 UTC string (`2026-10-11T20:12:00Z`) for wall-clock milliseconds.
pub fn rfc3339(epoch_ms: u64) -> String {
    let secs = epoch_ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant), valid for every date after 1970.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_formats_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_791_530_155_000), "2026-10-09T07:15:55Z");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn covered_browsers_match_ancestry() {
        let covered = CoveredBrowsers([100].into());
        assert!(covered.covers(&[300, 100]));
        assert!(!covered.covers(&[300, 200]));
        assert!(!covered.covers(&[]));
    }
}
