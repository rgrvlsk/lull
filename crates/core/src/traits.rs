//! Platform traits. Core logic (M1) only talks to the operating system through these.
//! Fakes for every trait live in [`crate::testing`].

use std::collections::BTreeMap;
use std::io;

pub use lull_protocol::rpc::ActivityBackend as ActivityBackendKind;

/// A monotonic instant that stops while the Mac sleeps (contract §5, "Clock"):
/// `CLOCK_UPTIME_RAW` plus the boot session it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Instant {
    /// Milliseconds of awake time since boot.
    pub uptime_ms: u64,
    /// `kern.bootsessionuuid`; instants from different sessions are not comparable.
    pub boot_session: String,
}

/// Time source (§5).
pub trait Clock {
    /// Awake-time instant used for every deadline.
    fn now(&self) -> Instant;
    /// Wall-clock milliseconds since the Unix epoch, display only.
    fn wall_now_ms(&self) -> u64;
}

/// A TCP listening socket of a process.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListenerSocket {
    pub pid: u32,
    pub local_addr: String,
    pub local_port: u16,
}

/// An accepted or outgoing TCP connection, with its 5-tuple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TcpConnection {
    pub pid: u32,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
}

/// One process as observed in a scan (§4.2). Fields that could not be read are `None`
/// and the failure is listed in `errors`, so discovery can apply §4.8.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub pgid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub start_epoch_ms: u64,
    pub exe: Option<String>,
    pub argv: Option<Vec<String>>,
    /// Initial environment; `None` when unreadable.
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<String>,
    /// Whether the process has a controlling tty (ADR-0001 veto).
    pub has_tty: Option<bool>,
    pub listeners: Option<Vec<ListenerSocket>>,
    pub connections: Option<Vec<TcpConnection>>,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    pub errors: Vec<String>,
}

/// One scan of every process of the current uid.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub taken_at: Option<Instant>,
    pub processes: Vec<ProcessInfo>,
}

/// Source of process and socket information (§4.1–4.2; libproc on macOS, `/proc` later).
pub trait ProcessSource {
    fn snapshot(&mut self) -> io::Result<Snapshot>;
}

/// An activity observation from the platform (§5 `traffic`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityEvent {
    /// A new TCP connection to `local_port` from `client_pid` (unknown when `None`).
    Traffic {
        local_port: u16,
        client_pid: Option<u32>,
        at: Instant,
    },
}

/// Source of `traffic` events: ntstat, or the accepted-socket diff fallback (ADR-0003).
pub trait ActivityBackend {
    fn kind(&self) -> ActivityBackendKind;
    /// Events observed since the previous call.
    fn drain(&mut self) -> Vec<ActivityEvent>;
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

/// Persistence of `state.json` (§4.7). The schema is owned by core (WP1.4); the store
/// moves bytes atomically and reports a corrupt file by moving it aside.
pub trait StateStore {
    /// `Ok(None)` when no state exists yet.
    fn load(&mut self) -> io::Result<Option<serde_json::Value>>;
    fn save(&mut self, state: &serde_json::Value) -> io::Result<()>;
}
