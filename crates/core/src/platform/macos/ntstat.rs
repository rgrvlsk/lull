//! ntstat activity backend over the kernel control socket `com.apple.network.statistics`
//! (contract §5, ADR-0003 and its S1 addendum).
//!
//! Message flow: `ADD_ALL_SRCS` for the kernel TCP provider enumerates every existing TCP
//! source (`SRC_ADDED`, no endpoints) and subscribes to new ones; each `SRC_ADDED` after
//! the subscription is answered with `GET_SRC_DESC`, and the `SRC_DESC` reply carries the
//! `nstat_tcp_descriptor` (pid, endpoints). A source that has not connected yet
//! (client sockets are added at `socket()`, before `connect()`) is re-queried on a tick;
//! the kernel also sends a final `SRC_DESC` (flag `CLOSING`) at teardown, which catches
//! connections shorter than a tick. The client end of a loopback connection emits the
//! event (its pid is the client, its remote port the server). Resolution matches that
//! destination against server ports. Non-loopback peers emit the local port without a pid.
//!
//! Byte counters (§5 pulses, ADR-0010): four times a second, each connected source that
//! reaches a watched port from a client outside its group is asked for its counts
//! (`QUERY_SRC`), and the `SRC_COUNTS` reply becomes a `Bytes` event with the bytes moved
//! since the previous reading, stamped with the sample's grid instant. Sources that existed before the subscription are described too, so
//! their first reading is a baseline rather than traffic. A new source removed before its
//! first reading emits `Unread`.
//!
//! Layout validation: there is no version field. `hdr.length` is validated for every
//! message, the TCP descriptor must be at least the published 344 bytes (macOS 27 sends
//! 360 with a shifted tail), and the first descriptor must decode to plausible sockaddrs before `open()` succeeds. Anything implausible later flips health to
//! `Failed` and the daemon falls back to the scan backend.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant as StdInstant};

use crate::model::{BackendHealth, ConnectionKind, Snapshot, TrafficEvent};
use crate::platform::macos::clock::MacClock;
use crate::traits::{ActivityBackend, ActivityBackendKind, Clock, Instant};

const CONTROL_NAME: &str = "com.apple.network.statistics";

// bsd/net/ntstat.h: message types, provider id, header layout.
const MSG_SUCCESS: u32 = 0;
const MSG_ERROR: u32 = 1;
const MSG_ADD_ALL_SRCS: u32 = 1002;
const MSG_QUERY_SRC: u32 = 1004;
const MSG_GET_SRC_DESC: u32 = 1005;
const MSG_SRC_ADDED: u32 = 10001;
const MSG_SRC_REMOVED: u32 = 10002;
const MSG_SRC_DESC: u32 = 10003;
const MSG_SRC_COUNTS: u32 = 10004;
const PROVIDER_TCP_KERNEL: u32 = 2;
/// `sizeof(nstat_msg_hdr)`: context u64, type u32, length u16, flags u16.
const HDR_LEN: usize = 16;
/// `sizeof(nstat_msg_add_all_srcs)`.
const ADD_ALL_LEN: usize = 56;
/// `sizeof(nstat_msg_get_src_description)`: header plus srcref.
const GET_SRC_DESC_LEN: usize = 24;
/// `sizeof(nstat_msg_query_src)`: header plus srcref.
const QUERY_SRC_LEN: usize = 24;
/// `nstat_msg_src_counts`: header, srcref, event_flags, then `nstat_counts`, whose
/// `nstat_rxbytes` and `nstat_txbytes` follow the rx and tx packet counts.
const COUNTS_RX_BYTES: usize = 40;
const COUNTS_TX_BYTES: usize = 56;
const COUNTS_MIN: usize = COUNTS_TX_BYTES + 8;
/// `offsetof(nstat_msg_src_description, data)`: header, srcref, event_flags, provider.
const SRC_DESC_DATA: usize = 40;
/// Published minimum size; only stable prefix offsets are decoded.
const TCP_DESC_MIN: usize = 344;
const TCP_DESC_MAX: usize = 4096;
// nstat_tcp_descriptor offsets that have held across layouts (everything before the uuids).
const DESC_STATE: usize = 76;
const DESC_PID: usize = 116;
const DESC_LOCAL: usize = 124;
const DESC_REMOTE: usize = 152;
const SOCKADDR_LEN: usize = 28;
const TCPS_LISTEN: u32 = 1;

const ADD_ALL_CONTEXT: u64 = 1;
const TICK: Duration = Duration::from_millis(250);
/// How often byte counters are read. Bytes are stamped with the sample's instant, so this
/// bounds their timing error, which must stay well inside the 1 s train tolerance (§5).
const SAMPLE: Duration = Duration::from_millis(250);
// ponytail: fixed burst and memory ceilings suit a developer machine; overflow fails
// to scan. Tune these from measured workloads before supporting high-volume hosts.
const REQUESTS_PER_TICK: usize = 16;
/// Counter queries per sample; past it, the least recently read sources go first.
const QUERIES_PER_SAMPLE: usize = 256;
const MAX_SOURCES: usize = 16_384;
const MAX_EVENTS: usize = 4_096;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const RECV_BUF: usize = 1 << 16;

/// Watched ports, each with the pids whose connections are not read (§5 pulses).
type Watch = BTreeMap<u16, BTreeSet<u32>>;

/// Traffic events from the kernel network-statistics socket. A reader thread owns the
/// socket and pushes `New`, `Bytes` and `Unread` events into a channel; `drain` collects
/// them.
pub struct NtstatBackend {
    rx: Receiver<TrafficEvent>,
    watch: Arc<Mutex<Watch>>,
    health: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl NtstatBackend {
    /// Connects, subscribes to kernel TCP sources and validates the descriptor layout.
    /// `Err` when the socket cannot be opened, the kernel refuses the subscription or the
    /// layout is unknown.
    pub fn open() -> io::Result<Self> {
        let sock = Socket::open()?;
        sock.send(&add_all_tcp())?;
        let (tx, rx) = mpsc::sync_channel(MAX_EVENTS);
        let watch = Arc::new(Mutex::new(Watch::new()));
        let mut reader = Reader::new(sock, tx, Arc::clone(&watch));
        let mut buf = vec![0u8; RECV_BUF];
        let deadline = StdInstant::now() + HANDSHAKE_TIMEOUT;
        while !(reader.subscribed && reader.validated) {
            if StdInstant::now() > deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ntstat: no usable answer to ADD_ALL_SRCS",
                ));
            }
            reader
                .step(&mut buf)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        }
        let health = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_health = Arc::clone(&health);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("lull-ntstat".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    if let Err(reason) = reader.step(&mut buf) {
                        *lock(&thread_health) = Some(reason);
                        return;
                    }
                }
            })?;
        Ok(Self {
            rx,
            watch,
            health,
            stop,
            reader: Some(handle),
        })
    }
}

impl Drop for NtstatBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.reader.take() {
            // The reader wakes at least once per TICK.
            let _ = handle.join();
        }
    }
}

impl ActivityBackend for NtstatBackend {
    fn kind(&self) -> ActivityBackendKind {
        ActivityBackendKind::Ntstat
    }

    fn drain(&mut self, _snapshot: &Snapshot, _now: &Instant) -> Vec<TrafficEvent> {
        self.rx.try_iter().collect()
    }

    fn reads_bytes(&self) -> bool {
        true
    }

    fn watch(&mut self, ports: &Watch) {
        *lock(&self.watch) = ports.clone();
    }

    fn health(&self) -> BackendHealth {
        if let Some(reason) = lock(&self.health).clone() {
            return BackendHealth::Failed(reason);
        }
        match &self.reader {
            Some(handle) if handle.is_finished() => {
                BackendHealth::Failed("ntstat reader thread exited".into())
            }
            _ => BackendHealth::Healthy,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// ───────────────────────── reader ─────────────────────────

struct Reader {
    sock: Socket,
    clock: MacClock,
    tx: SyncSender<TrafficEvent>,
    watch: Arc<Mutex<Watch>>,
    /// `SUCCESS` for `ADD_ALL_SRCS` seen: sources added from now on are new connections.
    subscribed: bool,
    /// One TCP descriptor decoded with a plausible layout.
    validated: bool,
    sources: HashMap<u64, Source>,
    last_tick: StdInstant,
    /// Samples run on a fixed grid; `Bytes` events carry the grid instant of the sample
    /// that read them, not the time the reader got round to it.
    next_sample: StdInstant,
    sample_at: Option<Instant>,
}

enum Source {
    Pending {
        emit: bool,
        next_query: StdInstant,
    },
    /// A connected source: the port it reaches, its client, and its byte total at the last
    /// reading (`None` before the first).
    Connected {
        port: u16,
        client_pid: Option<u32>,
        /// Added after the subscription; a preexisting source's first reading is a baseline.
        new: bool,
        total: Option<u64>,
        last_query: Option<StdInstant>,
    },
    Done,
}

impl Reader {
    fn new(sock: Socket, tx: SyncSender<TrafficEvent>, watch: Arc<Mutex<Watch>>) -> Self {
        Self {
            sock,
            clock: MacClock::new(),
            tx,
            watch,
            subscribed: false,
            validated: false,
            sources: HashMap::new(),
            last_tick: StdInstant::now(),
            next_sample: StdInstant::now() + SAMPLE,
            sample_at: None,
        }
    }

    /// One receive (bounded by `TICK`) plus any due re-queries. `Err` is fatal: the
    /// socket is gone or the kernel speaks an unknown layout.
    fn step(&mut self, buf: &mut [u8]) -> Result<(), String> {
        match self.sock.recv(buf) {
            Ok(Some(0)) => return Err("ntstat socket closed".into()),
            Ok(Some(n)) => self.datagram(&buf[..n])?,
            Ok(None) => {}
            Err(e) => return Err(format!("ntstat recv: {e}")),
        }
        if self.last_tick.elapsed() >= TICK {
            self.last_tick = StdInstant::now();
            self.tick()?;
        }
        let now = StdInstant::now();
        if now >= self.next_sample {
            let late = now - self.next_sample;
            let clock_now = self.clock.now();
            self.sample_at = Some(Instant {
                uptime_ms: clock_now
                    .uptime_ms
                    .saturating_sub(u64::try_from(late.as_millis()).unwrap_or(u64::MAX)),
                boot_session: clock_now.boot_session,
            });
            self.next_sample += SAMPLE;
            if self.next_sample <= now {
                // Fell behind by more than a sample (the thread was not scheduled): restart
                // the grid rather than sampling in a burst.
                self.next_sample = now + SAMPLE;
            }
            self.sample()?;
        }
        Ok(())
    }

    /// Asks for the byte counters of every watched source.
    fn sample(&mut self) -> Result<(), String> {
        let now = StdInstant::now();
        let due = counts_due(&self.sources, &lock(&self.watch));
        for srcref in due {
            if let Some(Source::Connected { last_query, .. }) = self.sources.get_mut(&srcref) {
                *last_query = Some(now);
            }
            if let Err(e) = self.sock.send(&query_src(srcref)) {
                match e.raw_os_error() {
                    Some(libc::ENOENT) => self.remove(srcref)?,
                    Some(libc::EAGAIN) | Some(libc::EINTR) => break,
                    _ => return Err(format!("ntstat counts request: {e}")),
                }
            }
        }
        Ok(())
    }

    /// Forgets a source. A new watched connection that never had its counters read
    /// emits `Unread`: the pulse it belongs to has an unknown size.
    fn remove(&mut self, srcref: u64) -> Result<(), String> {
        if let Some(Source::Connected {
            port,
            client_pid,
            new: true,
            total: None,
            ..
        }) = self.sources.remove(&srcref)
            && watched(&lock(&self.watch), port, client_pid)
        {
            self.emit(port, client_pid, ConnectionKind::Unread, self.clock.now())?;
        }
        Ok(())
    }

    /// A `SRC_COUNTS` reply: emits the bytes moved since the previous reading.
    fn counts(&mut self, srcref: u64, now_total: u64) -> Result<(), String> {
        let Some(Source::Connected {
            port,
            client_pid,
            new,
            total,
            ..
        }) = self.sources.get_mut(&srcref)
        else {
            return Ok(());
        };
        let moved = match *total {
            Some(previous) => now_total.saturating_sub(previous),
            None if *new => now_total,
            None => 0,
        };
        *total = Some(now_total);
        let (port, client_pid) = (*port, *client_pid);
        if moved > 0 {
            let at = self.sample_at.clone().unwrap_or_else(|| self.clock.now());
            self.emit(port, client_pid, ConnectionKind::Bytes(moved), at)?;
        }
        Ok(())
    }

    fn emit(
        &self,
        port: u16,
        client_pid: Option<u32>,
        kind: ConnectionKind,
        at: Instant,
    ) -> Result<(), String> {
        self.tx
            .try_send(TrafficEvent {
                port,
                client_pid,
                kind,
                at,
            })
            .map_err(|e| format!("ntstat event channel: {e}"))
    }

    fn tick(&mut self) -> Result<(), String> {
        let now = StdInstant::now();
        for srcref in descriptions_due(&self.sources, now) {
            if let Some(Source::Pending { next_query, .. }) = self.sources.get_mut(&srcref) {
                *next_query = now + TICK;
            }
            if let Err(e) = self.sock.send(&get_src_desc(srcref)) {
                match e.raw_os_error() {
                    Some(libc::ENOENT) => {
                        self.remove(srcref)?;
                    }
                    Some(libc::EAGAIN) | Some(libc::EINTR) => break,
                    _ => return Err(format!("ntstat descriptor request: {e}")),
                }
            }
        }
        Ok(())
    }

    /// A datagram holds one or more messages, each prefixed by `nstat_msg_hdr`.
    fn datagram(&mut self, data: &[u8]) -> Result<(), String> {
        let mut off = 0;
        while off + HDR_LEN <= data.len() {
            let context = u64_at(data, off);
            let kind = u32_at(data, off + 8);
            let len = usize::from(u16_at(data, off + 12));
            if len < HDR_LEN || off + len > data.len() {
                return Err("ntstat: invalid message length".into());
            }
            if u16_at(data, off + 14) & (1 << 3) != 0 {
                return Err("ntstat: kernel reported dropped messages".into());
            }
            self.message(context, kind, &data[off..off + len])?;
            off += len;
        }
        if off != data.len() {
            return Err("ntstat: truncated message header".into());
        }
        Ok(())
    }

    fn message(&mut self, context: u64, kind: u32, m: &[u8]) -> Result<(), String> {
        match kind {
            MSG_SUCCESS if context == ADD_ALL_CONTEXT => self.subscribed = true,
            MSG_ERROR if m.len() >= 20 => {
                let errno = u32_at(m, 16) as i32;
                if context == ADD_ALL_CONTEXT || errno != libc::ENOENT {
                    return Err(format!("ntstat request rejected (errno {errno})"));
                }
                self.remove(context)?;
            }
            MSG_SRC_ADDED if m.len() >= 28 => {
                let srcref = u64_at(m, 16);
                if u32_at(m, 24) != PROVIDER_TCP_KERNEL {
                    return Err("ntstat: unexpected provider".into());
                }
                if self.sources.len() >= MAX_SOURCES {
                    return Err("ntstat: source capacity exceeded".into());
                }
                // Preexisting sources are described too: the first validates the layout,
                // and connected ones get a byte baseline.
                self.sources.entry(srcref).or_insert(Source::Pending {
                    emit: self.subscribed,
                    next_query: StdInstant::now(),
                });
            }
            MSG_SRC_REMOVED if m.len() >= 24 => {
                self.remove(u64_at(m, 16))?;
            }
            MSG_SRC_DESC if m.len() >= SRC_DESC_DATA => {
                if u32_at(m, 32) != PROVIDER_TCP_KERNEL {
                    return Err("ntstat: unexpected descriptor provider".into());
                }
                let srcref = u64_at(m, 16);
                let desc = parse_tcp_desc(&m[SRC_DESC_DATA..])?;
                self.validated |= desc.local.is_some() || desc.remote.is_some();
                self.descriptor(srcref, &desc)?;
            }
            MSG_SRC_COUNTS if m.len() >= COUNTS_MIN => {
                let total = u64_at(m, COUNTS_RX_BYTES).saturating_add(u64_at(m, COUNTS_TX_BYTES));
                self.counts(u64_at(m, 16), total)?;
            }
            // SUCCESS may acknowledge another request.
            MSG_SUCCESS => {}
            _ => return Err(format!("ntstat: unknown or truncated message type {kind}")),
        }
        Ok(())
    }

    fn descriptor(&mut self, srcref: u64, desc: &TcpDesc) -> Result<(), String> {
        let Some(source @ Source::Pending { .. }) = self.sources.get_mut(&srcref) else {
            return Ok(());
        };
        if desc.state == TCPS_LISTEN {
            *source = Source::Done;
            return Ok(());
        }
        let (Some(local), Some(remote)) = (desc.local, desc.remote) else {
            return Ok(());
        };
        if remote.port == 0 || local.port == 0 {
            return Ok(());
        }
        let emit = matches!(source, Source::Pending { emit: true, .. });
        // No tail role flags: on loopback the source with remote == server port
        // carries the client pid. Activity resolves the destination to a server.
        let (port, client_pid) = if remote.loopback {
            (remote.port, (desc.pid != 0).then_some(desc.pid))
        } else {
            // ponytail: a non-loopback client counts as outside the group, even a
            // same-machine one via the LAN address. Upgrade path: pair the reversed
            // tuple for local non-loopback addresses too.
            (local.port, None)
        };
        *source = Source::Connected {
            port,
            client_pid,
            new: emit,
            total: None,
            last_query: None,
        };
        if emit {
            self.emit(port, client_pid, ConnectionKind::New, self.clock.now())?;
        }
        Ok(())
    }
}

// ───────────────────────── messages ─────────────────────────

fn header(context: u64, kind: u32, len: usize) -> [u8; HDR_LEN] {
    let mut h = [0u8; HDR_LEN];
    h[0..8].copy_from_slice(&context.to_ne_bytes());
    h[8..12].copy_from_slice(&kind.to_ne_bytes());
    h[12..14].copy_from_slice(&(len as u16).to_ne_bytes());
    h
}

/// `nstat_msg_add_all_srcs` for the kernel TCP provider: no filter, no events, no target.
fn add_all_tcp() -> Vec<u8> {
    let mut m = Vec::with_capacity(ADD_ALL_LEN);
    m.extend_from_slice(&header(ADD_ALL_CONTEXT, MSG_ADD_ALL_SRCS, ADD_ALL_LEN));
    m.extend_from_slice(&0u64.to_ne_bytes()); // filter
    m.extend_from_slice(&0u64.to_ne_bytes()); // events
    m.extend_from_slice(&PROVIDER_TCP_KERNEL.to_ne_bytes());
    m.extend_from_slice(&0i32.to_ne_bytes()); // target_pid
    m.extend_from_slice(&[0u8; 16]); // target_uuid
    m
}

/// `nstat_msg_get_src_description`; the context is the srcref so an error reply can be
/// matched to it.
fn get_src_desc(srcref: u64) -> Vec<u8> {
    let mut m = Vec::with_capacity(GET_SRC_DESC_LEN);
    m.extend_from_slice(&header(srcref, MSG_GET_SRC_DESC, GET_SRC_DESC_LEN));
    m.extend_from_slice(&srcref.to_ne_bytes());
    m
}

/// `nstat_msg_query_src` for one source; the context is the srcref, as for descriptors.
fn query_src(srcref: u64) -> Vec<u8> {
    let mut m = Vec::with_capacity(QUERY_SRC_LEN);
    m.extend_from_slice(&header(srcref, MSG_QUERY_SRC, QUERY_SRC_LEN));
    m.extend_from_slice(&srcref.to_ne_bytes());
    m
}

fn watched(watch: &Watch, port: u16, client_pid: Option<u32>) -> bool {
    watch
        .get(&port)
        .is_some_and(|skip| client_pid.is_none_or(|pid| !skip.contains(&pid)))
}

/// Pending sources due for a descriptor request, at most `REQUESTS_PER_TICK`. New
/// connections go first: describing the sources that existed before the subscription
/// (for their byte baseline) must not delay them.
fn descriptions_due(sources: &HashMap<u64, Source>, now: StdInstant) -> Vec<u64> {
    let mut due: Vec<_> = sources
        .iter()
        .filter_map(|(&srcref, source)| match source {
            Source::Pending { emit, next_query } if *next_query <= now => {
                Some((!emit, *next_query, srcref))
            }
            _ => None,
        })
        .collect();
    due.sort_unstable();
    due.into_iter()
        .take(REQUESTS_PER_TICK)
        .map(|(_, _, srcref)| srcref)
        .collect()
}

/// Watched connected sources, least recently read first, at most `QUERIES_PER_SAMPLE`.
fn counts_due(sources: &HashMap<u64, Source>, watch: &Watch) -> Vec<u64> {
    let mut due: Vec<_> = sources
        .iter()
        .filter_map(|(&srcref, source)| match source {
            Source::Connected {
                port,
                client_pid,
                last_query,
                ..
            } if watched(watch, *port, *client_pid) => Some((*last_query, srcref)),
            _ => None,
        })
        .collect();
    due.sort_unstable();
    due.into_iter()
        .take(QUERIES_PER_SAMPLE)
        .map(|(_, srcref)| srcref)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Endpoint {
    port: u16,
    loopback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TcpDesc {
    state: u32,
    pid: u32,
    local: Option<Endpoint>,
    remote: Option<Endpoint>,
}

/// Decodes the fields this backend needs from an `nstat_tcp_descriptor`; `Err` means the
/// layout is not one we understand.
fn parse_tcp_desc(d: &[u8]) -> Result<TcpDesc, String> {
    if d.len() < TCP_DESC_MIN || d.len() > TCP_DESC_MAX {
        return Err(format!("ntstat: tcp descriptor of {} bytes", d.len()));
    }
    if u32_at(d, DESC_STATE) > 10 || u32_at(d, DESC_PID) > i32::MAX as u32 {
        return Err("ntstat: implausible TCP state or pid".into());
    }
    Ok(TcpDesc {
        state: u32_at(d, DESC_STATE),
        pid: u32_at(d, DESC_PID),
        local: parse_sockaddr(&d[DESC_LOCAL..DESC_LOCAL + SOCKADDR_LEN])?,
        remote: parse_sockaddr(&d[DESC_REMOTE..DESC_REMOTE + SOCKADDR_LEN])?,
    })
}

/// `union { sockaddr_in; sockaddr_in6 }`: `Ok(None)` when unset (all zero).
fn parse_sockaddr(b: &[u8]) -> Result<Option<Endpoint>, String> {
    let (len, family) = (b[0], b[1]);
    let port = u16::from_be_bytes([b[2], b[3]]);
    match (len, family) {
        (0, 0) if b.iter().all(|&byte| byte == 0) => Ok(None),
        (16, 2) => Ok(Some(Endpoint {
            port,
            loopback: b[4] == 127,
        })),
        (28, 30) => {
            let octets: [u8; 16] = b[8..24].try_into().expect("16-byte slice");
            let addr = std::net::Ipv6Addr::from(octets);
            Ok(Some(Endpoint {
                port,
                loopback: addr.is_loopback()
                    || addr.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
            }))
        }
        _ => Err(format!("ntstat: sockaddr len {len} family {family}")),
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_ne_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_ne_bytes(b[o..o + 8].try_into().expect("8-byte slice"))
}

// ───────────────────────── kernel control socket ─────────────────────────

struct Socket(libc::c_int);

impl Drop for Socket {
    fn drop(&mut self) {
        // SAFETY: the descriptor is owned by this struct and closed exactly once.
        unsafe { libc::close(self.0) };
    }
}

impl Socket {
    fn open() -> io::Result<Self> {
        // SAFETY: plain socket creation.
        let fd = unsafe { libc::socket(libc::PF_SYSTEM, libc::SOCK_DGRAM, libc::SYSPROTO_CONTROL) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let sock = Self(fd);
        // SAFETY: fd is owned and valid. Restarted servers must not inherit this socket.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: zeroed ctl_info is valid; the name fits in ctl_name with a NUL.
        let mut info: libc::ctl_info = unsafe { mem::zeroed() };
        for (dst, src) in info.ctl_name.iter_mut().zip(CONTROL_NAME.bytes()) {
            *dst = src as libc::c_char;
        }
        // SAFETY: CTLIOCGINFO takes a pointer to ctl_info.
        if unsafe { libc::ioctl(fd, libc::CTLIOCGINFO, &mut info) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: zeroed sockaddr_ctl is valid; the fields are set before use.
        let mut addr: libc::sockaddr_ctl = unsafe { mem::zeroed() };
        addr.sc_len = mem::size_of::<libc::sockaddr_ctl>() as u8;
        addr.sc_family = libc::AF_SYSTEM as u8;
        addr.ss_sysaddr = libc::AF_SYS_CONTROL as u16;
        addr.sc_id = info.ctl_id;
        addr.sc_unit = 0;
        // SAFETY: `addr` is a fully initialised sockaddr_ctl of the stated length.
        let rc = unsafe {
            libc::connect(
                fd,
                (&raw const addr).cast::<libc::sockaddr>(),
                libc::socklen_t::from(addr.sc_len),
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        // A burst of SRC_ADDED after subscribing exceeds the 8 KiB default; failure here
        // would hide activity loss, so fail opening if the buffer cannot be raised.
        let rcvbuf: libc::c_int = 1 << 20;
        // SAFETY: option value pointer and length match.
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&raw const rcvbuf).cast(),
                mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        let tv = libc::timeval {
            tv_sec: 0,
            tv_usec: libc::suseconds_t::try_from(TICK.as_micros()).unwrap_or(250_000),
        };
        // SAFETY: option value pointer and length match.
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&raw const tv).cast(),
                mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sock)
    }

    fn send(&self, msg: &[u8]) -> io::Result<()> {
        // SAFETY: `msg` is a valid slice for its length.
        let n = unsafe { libc::send(self.0, msg.as_ptr().cast(), msg.len(), libc::MSG_DONTWAIT) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n as usize != msg.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short ntstat send",
            ));
        }
        Ok(())
    }

    /// `Ok(None)` when the receive timeout elapsed.
    fn recv(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        // SAFETY: zero is valid for the optional name/control fields.
        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        // SAFETY: msg describes one live writable buffer, with no ancillary storage.
        let n = unsafe { libc::recvmsg(self.0, &mut msg, 0) };
        if msg.msg_flags & libc::MSG_TRUNC != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated ntstat datagram",
            ));
        }
        if n < 0 {
            let err = io::Error::last_os_error();
            return match err.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => Ok(None),
                _ => Err(err),
            };
        }
        Ok(Some(n as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(port: u16) -> [u8; SOCKADDR_LEN] {
        let mut b = [0u8; SOCKADDR_LEN];
        b[0] = 16;
        b[1] = 2;
        b[2..4].copy_from_slice(&port.to_be_bytes());
        b[4] = 127;
        b[7] = 1;
        b
    }

    fn descriptor(len: usize) -> Vec<u8> {
        let mut d = vec![0u8; len];
        d[DESC_STATE..DESC_STATE + 4].copy_from_slice(&4u32.to_ne_bytes());
        d[DESC_PID..DESC_PID + 4].copy_from_slice(&77u32.to_ne_bytes());
        d[DESC_LOCAL..DESC_LOCAL + SOCKADDR_LEN].copy_from_slice(&v4(51000));
        d[DESC_REMOTE..DESC_REMOTE + SOCKADDR_LEN].copy_from_slice(&v4(3000));
        d
    }

    fn description(srcref: u64, d: &[u8]) -> Vec<u8> {
        let mut m = header(srcref, MSG_SRC_DESC, SRC_DESC_DATA + d.len()).to_vec();
        m.extend_from_slice(&srcref.to_ne_bytes());
        m.extend_from_slice(&0u64.to_ne_bytes());
        m.extend_from_slice(&PROVIDER_TCP_KERNEL.to_ne_bytes());
        m.extend_from_slice(&[0; 4]);
        m.extend_from_slice(d);
        m
    }

    fn pending(reader: &mut Reader, srcref: u64, emit: bool) {
        reader.sources.insert(
            srcref,
            Source::Pending {
                emit,
                next_query: StdInstant::now(),
            },
        );
    }

    #[test]
    fn decodes_published_and_live_prefix_without_trusting_the_tail() {
        for len in [344, 360] {
            let mut d = descriptor(len);
            d[260..].fill(0xff); // arbitrary shifted tail must not affect decoding
            let desc = parse_tcp_desc(&d).unwrap();
            assert_eq!(desc.pid, 77);
            assert_eq!(desc.local.unwrap().port, 51000);
            assert_eq!(desc.remote.unwrap().port, 3000);
            assert!(desc.remote.unwrap().loopback);
        }
    }

    #[test]
    fn rejects_unknown_layouts_and_truncated_datagrams_without_panicking() {
        for len in 0..TCP_DESC_MIN {
            assert!(parse_tcp_desc(&vec![0; len]).is_err());
        }
        let mut d = descriptor(360);
        d[DESC_LOCAL + 1] = 7;
        assert!(parse_tcp_desc(&d).is_err());
        let (tx, _) = mpsc::sync_channel(MAX_EVENTS);
        let mut reader = Reader::new(Socket(-1), tx, Arc::default());
        let m = description(9, &descriptor(360));
        for len in 1..m.len() {
            assert!(reader.datagram(&m[..len]).is_err());
        }
        let mut dropped = header(0, MSG_SUCCESS, HDR_LEN);
        dropped[14..16].copy_from_slice(&(1u16 << 3).to_ne_bytes());
        assert!(reader.datagram(&dropped).is_err());
        assert!(reader.datagram(&header(0, 0xffff, HDR_LEN)).is_err());
        assert!(reader.datagram(&header(0, MSG_SRC_ADDED, HDR_LEN)).is_err());
    }

    #[test]
    fn validates_unset_v4_v6_and_mapped_sockaddrs() {
        assert_eq!(parse_sockaddr(&[0; SOCKADDR_LEN]).unwrap(), None);
        let mut b = [0; SOCKADDR_LEN];
        b[2] = 1;
        assert!(parse_sockaddr(&b).is_err());
        b[0] = 28;
        b[1] = 30;
        b[2..4].copy_from_slice(&8080u16.to_be_bytes());
        b[23] = 1;
        let endpoint = parse_sockaddr(&b).unwrap().unwrap();
        assert_eq!(endpoint.port, 8080);
        assert!(endpoint.loopback);
        b[18] = 0xff;
        b[19] = 0xff;
        b[20] = 127;
        assert!(parse_sockaddr(&b).unwrap().unwrap().loopback);
        b[20] = 10;
        assert!(!parse_sockaddr(&b).unwrap().unwrap().loopback);
    }

    #[test]
    fn batches_deduplicate_and_preserve_unresolved_sources_until_connected() {
        let (tx, rx) = mpsc::sync_channel(MAX_EVENTS);
        let mut reader = Reader::new(Socket(-1), tx, Arc::default());
        pending(&mut reader, 9, true);
        reader.datagram(&description(9, &[0; 360])).unwrap();
        assert!(matches!(
            reader.sources.get(&9),
            Some(Source::Pending { .. })
        ));
        assert!(rx.try_recv().is_err());
        let message = description(9, &descriptor(360));
        reader
            .datagram(&[message.clone(), message].concat())
            .unwrap();
        let event = rx.try_recv().unwrap();
        assert_eq!(
            (event.port, event.client_pid, event.kind),
            (3000, Some(77), ConnectionKind::New)
        );
        assert!(rx.try_recv().is_err());
        // The pre-subscription enumeration validates the ABI without creating activity.
        pending(&mut reader, 10, false);
        reader.datagram(&description(10, &descriptor(344))).unwrap();
        assert!(reader.validated);
        assert!(rx.try_recv().is_err());
        // Listening sources never create activity, even with populated endpoints.
        pending(&mut reader, 11, true);
        let mut d = descriptor(360);
        d[DESC_STATE..DESC_STATE + 4].copy_from_slice(&TCPS_LISTEN.to_ne_bytes());
        reader.datagram(&description(11, &d)).unwrap();
        assert!(matches!(reader.sources.get(&11), Some(Source::Done)));
        assert!(rx.try_recv().is_err());
        let mut removed = header(0, MSG_SRC_REMOVED, 24).to_vec();
        removed.extend_from_slice(&9u64.to_ne_bytes());
        reader.datagram(&removed).unwrap();
        assert!(!reader.sources.contains_key(&9));
    }

    #[test]
    fn remote_peers_have_no_pid_and_channel_overflow_fails() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut reader = Reader::new(Socket(-1), tx, Arc::default());
        let mut d = descriptor(360);
        d[DESC_REMOTE + 4] = 10;
        pending(&mut reader, 9, true);
        reader.datagram(&description(9, &d)).unwrap();
        pending(&mut reader, 10, true);
        assert!(reader.datagram(&description(10, &d)).is_err());
        let event = rx.try_recv().unwrap();
        assert_eq!((event.port, event.client_pid), (51000, None));
    }

    fn counts(srcref: u64, rx: u64, tx: u64) -> Vec<u8> {
        let mut m = header(srcref, MSG_SRC_COUNTS, 144).to_vec();
        m.extend_from_slice(&srcref.to_ne_bytes());
        m.extend_from_slice(&0u64.to_ne_bytes()); // event_flags
        for value in [1, rx, 1, tx] {
            m.extend_from_slice(&value.to_ne_bytes());
        }
        m.resize(144, 0);
        m
    }

    fn removed(srcref: u64) -> Vec<u8> {
        let mut m = header(0, MSG_SRC_REMOVED, 24).to_vec();
        m.extend_from_slice(&srcref.to_ne_bytes());
        m
    }

    fn watching(port: u16, skip: &[u32]) -> Arc<Mutex<Watch>> {
        Arc::new(Mutex::new(Watch::from([(
            port,
            skip.iter().copied().collect(),
        )])))
    }

    #[test]
    fn counts_become_bytes_moved_since_the_previous_reading() {
        let (tx, rx) = mpsc::sync_channel(MAX_EVENTS);
        let mut reader = Reader::new(Socket(-1), tx, watching(3000, &[]));
        // A new connection: its first reading is all traffic.
        pending(&mut reader, 9, true);
        reader.datagram(&description(9, &descriptor(360))).unwrap();
        assert_eq!(rx.try_recv().unwrap().kind, ConnectionKind::New);
        reader.datagram(&counts(9, 100, 20)).unwrap();
        let event = rx.try_recv().unwrap();
        assert_eq!(
            (event.port, event.client_pid, event.kind),
            (3000, Some(77), ConnectionKind::Bytes(120))
        );
        reader.datagram(&counts(9, 100, 20)).unwrap();
        assert!(rx.try_recv().is_err(), "nothing moved");
        reader.datagram(&counts(9, 150, 30)).unwrap();
        assert_eq!(rx.try_recv().unwrap().kind, ConnectionKind::Bytes(60));
        // A connection from before the subscription: the first reading is a baseline.
        pending(&mut reader, 10, false);
        reader.datagram(&description(10, &descriptor(360))).unwrap();
        reader.datagram(&counts(10, 5_000, 900)).unwrap();
        assert!(rx.try_recv().is_err());
        reader.datagram(&counts(10, 5_010, 900)).unwrap();
        assert_eq!(rx.try_recv().unwrap().kind, ConnectionKind::Bytes(10));
        // Counts for an unknown source are ignored; a truncated reply is not.
        reader.datagram(&counts(99, 1, 1)).unwrap();
        assert!(rx.try_recv().is_err());
        let short = counts(9, 1, 1);
        let mut short = short[..COUNTS_MIN - 1].to_vec();
        short[12..14].copy_from_slice(&((COUNTS_MIN - 1) as u16).to_ne_bytes());
        assert!(reader.datagram(&short).is_err());
    }

    #[test]
    fn a_new_watched_connection_removed_before_its_first_reading_is_unread() {
        let (tx, rx) = mpsc::sync_channel(MAX_EVENTS);
        let mut reader = Reader::new(Socket(-1), tx, watching(3000, &[]));
        for srcref in [9, 10, 11] {
            pending(&mut reader, srcref, true);
            reader
                .datagram(&description(srcref, &descriptor(360)))
                .unwrap();
            assert_eq!(rx.try_recv().unwrap().kind, ConnectionKind::New);
        }
        reader.datagram(&counts(10, 1, 1)).unwrap();
        assert_eq!(rx.try_recv().unwrap().kind, ConnectionKind::Bytes(2));
        reader.datagram(&removed(9)).unwrap();
        let event = rx.try_recv().unwrap();
        assert_eq!(
            (event.port, event.client_pid, event.kind),
            (3000, Some(77), ConnectionKind::Unread)
        );
        // Already read: nothing is unknown about it.
        reader.datagram(&removed(10)).unwrap();
        assert!(rx.try_recv().is_err());
        // Not watched any more: nobody needs to know.
        *lock(&reader.watch) = Watch::new();
        reader.datagram(&removed(11)).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn only_watched_sources_outside_the_group_are_read_least_recent_first() {
        let connected = |port, client_pid, last_query| Source::Connected {
            port,
            client_pid,
            new: true,
            total: None,
            last_query,
        };
        let now = StdInstant::now();
        let sources = HashMap::from([
            (1, connected(3000, Some(7), Some(now))),
            (2, connected(3000, Some(7), None)),
            (3, connected(3000, Some(100), None)), // the server's own group
            (4, connected(4000, Some(7), None)),   // not a server port
            (5, connected(3000, None, Some(now - SAMPLE))), // remote client
            (6, Source::Done),
        ]);
        let watch = Watch::from([(3000, BTreeSet::from([100]))]);
        assert_eq!(counts_due(&sources, &watch), vec![2, 5, 1]);
        assert!(counts_due(&sources, &Watch::new()).is_empty());
    }

    #[test]
    fn new_connections_are_described_before_the_preexisting_backlog() {
        let now = StdInstant::now();
        let pending = |emit, next_query| Source::Pending { emit, next_query };
        let mut sources: HashMap<u64, Source> = (100..140)
            .map(|srcref| (srcref, pending(false, now - TICK)))
            .collect();
        sources.insert(7, pending(true, now));
        sources.insert(8, pending(true, now + TICK)); // not due yet
        let due = descriptions_due(&sources, now);
        assert_eq!(due.len(), REQUESTS_PER_TICK);
        assert_eq!(due[0], 7);
        assert!(!due.contains(&8));
    }

    #[test]
    fn only_missing_source_errors_are_tolerated() {
        let (tx, _) = mpsc::sync_channel(1);
        let mut reader = Reader::new(Socket(-1), tx, Arc::default());
        for errno in [libc::ENOENT, libc::EPERM] {
            pending(&mut reader, 9, true);
            let mut error = header(9, MSG_ERROR, 24).to_vec();
            error.extend_from_slice(&errno.to_ne_bytes());
            error.extend_from_slice(&[0; 4]);
            assert_eq!(reader.datagram(&error).is_ok(), errno == libc::ENOENT);
        }
    }
}
