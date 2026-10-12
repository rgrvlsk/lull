//! Platform traits. Core logic only talks to the operating system through these.
//! Fakes for every trait live in [`crate::testing`].

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

pub use lull_protocol::rpc::ActivityBackend as ActivityBackendKind;

use crate::model::{BackendHealth, ProcessInfo, Snapshot, TrafficEvent};

/// A monotonic instant that stops while the Mac sleeps (contract §5, "Clock"):
/// `CLOCK_UPTIME_RAW` plus the boot session it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instant {
    /// Milliseconds of awake time since boot.
    pub uptime_ms: u64,
    /// `kern.bootsessionuuid`; instants from different sessions are not comparable.
    pub boot_session: String,
}

impl Instant {
    /// This instant moved forward by `ms` of awake time.
    pub fn plus_ms(&self, ms: u64) -> Instant {
        Instant {
            uptime_ms: self.uptime_ms + ms,
            boot_session: self.boot_session.clone(),
        }
    }
}

/// Time source (§5).
pub trait Clock {
    /// Awake-time instant used for every deadline.
    fn now(&self) -> Instant;
    /// Wall-clock milliseconds since the Unix epoch, display only.
    fn wall_now_ms(&self) -> u64;
}

/// Source of process and socket information (§4.1–4.2; libproc on macOS, `/proc` later).
pub trait ProcessSource {
    /// Every same-uid listening group, plus every member of `also_pgids` (known servers
    /// that may have lost their listener, §4.8).
    fn snapshot(&mut self, also_pgids: &[u32]) -> io::Result<Snapshot>;
    /// Current members of one group, read fresh: the re-check immediately before a
    /// signal (§7). An empty vec means the group has no members left.
    fn group(&mut self, pgid: u32) -> io::Result<Vec<ProcessInfo>>;
    /// Parent pid of a live process (covered browsers, §5).
    fn parent(&mut self, pid: u32) -> Option<u32>;
}

/// Source of `traffic` events: ntstat, or the accepted-socket diff fallback (ADR-0003).
pub trait ActivityBackend {
    fn kind(&self) -> ActivityBackendKind;
    /// New connections observed since the previous call. The scan fallback diffs
    /// `snapshot.connections` against the previous one; ntstat ignores it.
    fn drain(&mut self, snapshot: &Snapshot, now: &Instant) -> Vec<TrafficEvent>;
    fn health(&self) -> BackendHealth;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    Term,
    Kill,
}

/// Sends signals to a whole process group (§7). The safety checks before signalling are
/// core logic and live in the caller; the signaler only delivers.
pub trait Signaler {
    fn signal_group(&mut self, pgid: u32, signal: Signal) -> io::Result<()>;
}

/// What to start when restarting a stopped-server record (§7, ADR-0005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub argv: Vec<String>,
    pub cwd: String,
    /// Added on top of the login-shell environment.
    pub env: BTreeMap<String, String>,
    /// Path of the log file receiving stdout and stderr.
    pub log_path: String,
}

/// Starts a process in a new session through the user's login shell.
pub trait Spawner {
    /// Returns the pid of the new session leader.
    fn spawn(&mut self, request: &SpawnRequest) -> io::Result<u32>;
}

/// Persistence of `state.json` (§4.7). The schema is owned by lifecycle; the store moves
/// bytes atomically.
pub trait StateStore {
    /// `Ok(None)` when no state exists yet; `Err(InvalidData)` when it is not JSON.
    fn load(&mut self) -> io::Result<Option<serde_json::Value>>;
    fn save(&mut self, state: &serde_json::Value) -> io::Result<()>;
    /// Move a corrupt or unknown-version file aside (`state.json.corrupt-<ts>`); returns
    /// where it went.
    fn quarantine(&mut self) -> io::Result<String>;
}
