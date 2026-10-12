//! Fakes for every platform trait, for table-driven tests (plan, "Ground rules").

use std::collections::{BTreeMap, VecDeque};
use std::io;

use crate::traits::{
    ActivityBackend, ActivityBackendKind, ActivityEvent, Clock, Instant, ProcessSource, Signal,
    Signaler, Snapshot, SpawnRequest, Spawner, StateStore,
};

/// Returns a pre-set snapshot on every scan; replace it with `set`.
#[derive(Debug, Default)]
pub struct FakeProcessSource {
    pub snapshot: Snapshot,
    /// When set, the next scan fails with this error instead.
    pub fail_next: Option<io::ErrorKind>,
}

impl FakeProcessSource {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            fail_next: None,
        }
    }
}

impl ProcessSource for FakeProcessSource {
    fn snapshot(&mut self) -> io::Result<Snapshot> {
        match self.fail_next.take() {
            Some(kind) => Err(io::Error::from(kind)),
            None => Ok(self.snapshot.clone()),
        }
    }
}

/// Replays queued activity events.
#[derive(Debug)]
pub struct FakeActivityBackend {
    pub kind: ActivityBackendKind,
    pub queue: VecDeque<ActivityEvent>,
}

impl Default for FakeActivityBackend {
    fn default() -> Self {
        Self {
            kind: ActivityBackendKind::Scan,
            queue: VecDeque::new(),
        }
    }
}

impl ActivityBackend for FakeActivityBackend {
    fn kind(&self) -> ActivityBackendKind {
        self.kind
    }
    fn drain(&mut self) -> Vec<ActivityEvent> {
        self.queue.drain(..).collect()
    }
}

/// A clock the test moves by hand. `advance` moves awake time and wall time together;
/// `sleep` moves wall time only, like a closed lid.
#[derive(Debug, Clone)]
pub struct FakeClock {
    pub uptime_ms: u64,
    pub wall_ms: u64,
    pub boot_session: String,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            uptime_ms: 0,
            wall_ms: 1_760_227_200_000,
            boot_session: "boot-A".into(),
        }
    }
}

impl FakeClock {
    pub fn advance_ms(&mut self, ms: u64) {
        self.uptime_ms += ms;
        self.wall_ms += ms;
    }
    pub fn sleep_ms(&mut self, ms: u64) {
        self.wall_ms += ms;
    }
    pub fn reboot(&mut self, session: &str) {
        self.uptime_ms = 0;
        self.boot_session = session.to_owned();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        Instant {
            uptime_ms: self.uptime_ms,
            boot_session: self.boot_session.clone(),
        }
    }
    fn wall_now_ms(&self) -> u64 {
        self.wall_ms
    }
}

/// Records every signal instead of sending it.
#[derive(Debug, Default)]
pub struct RecordingSignaler {
    pub sent: Vec<(u32, Signal)>,
    pub fail_next: Option<io::ErrorKind>,
}

impl Signaler for RecordingSignaler {
    fn signal_group(&mut self, pgid: u32, signal: Signal) -> io::Result<()> {
        if let Some(kind) = self.fail_next.take() {
            return Err(io::Error::from(kind));
        }
        self.sent.push((pgid, signal));
        Ok(())
    }
}

/// Records spawn requests and hands out sequential pids.
#[derive(Debug, Default)]
pub struct FakeSpawner {
    pub requests: Vec<SpawnRequest>,
    pub next_pid: u32,
    pub fail_next: Option<io::ErrorKind>,
}

impl Spawner for FakeSpawner {
    fn spawn(&mut self, request: &SpawnRequest) -> io::Result<u32> {
        if let Some(kind) = self.fail_next.take() {
            return Err(io::Error::from(kind));
        }
        self.requests.push(request.clone());
        self.next_pid += 1;
        Ok(self.next_pid)
    }
}

/// Keeps state in memory and counts saves.
#[derive(Debug, Default)]
pub struct MemoryStateStore {
    pub state: Option<serde_json::Value>,
    pub saves: usize,
}

impl StateStore for MemoryStateStore {
    fn load(&mut self) -> io::Result<Option<serde_json::Value>> {
        Ok(self.state.clone())
    }
    fn save(&mut self, state: &serde_json::Value) -> io::Result<()> {
        self.state = Some(state.clone());
        self.saves += 1;
        Ok(())
    }
}

/// Convenience for building fake environments in tests.
pub fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}
