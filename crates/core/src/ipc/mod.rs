//! WP1.5 — the daemon's state and its JSON-RPC handling (contract §8), independent of the
//! transport: the runtime in `crates/cli` feeds it lines and connection events and sends
//! the [`Out`] lines it returns. Every platform touch point goes through [`Platform`].

use std::collections::BTreeMap;
use std::io;

use lull_protocol::rpc::{
    AnyRequest, ClientKind, ClientRequest, HelloResult, INVALID_PARAMS, METHOD_NOT_FOUND, Message,
    NOT_RESTARTABLE, OpenParams, OpenResult, PROTOCOL_MISMATCH, ProtocolMismatch, Request,
    RestartResult, SubscribeResult,
};
use lull_protocol::{Config, PROTOCOL_VERSION, Server};
use serde_json::{Value, json};

use crate::activity::{Activity, ScanInput};
use crate::discovery::{self, Context};
use crate::lifecycle::{Effect, Lifecycle, LifecycleError, Sys, Tick};
use crate::model::{ActivityEvent, CoveredBrowsers, Discovery, ObservedServer, rfc3339};
use crate::traits::{Clock, ConfigStore, Instant, ProcessSource, Signaler, Spawner, StateStore};
use crate::{policy, traits::ActivityBackend};

pub type ConnId = u64;

/// A line for one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub conn: ConnId,
    pub line: String,
}

/// `open(1)` fallback: url, `open_with` app (`""` = default), background.
pub type OpenUrl = Box<dyn FnMut(&str, &str, bool) -> io::Result<()> + Send>;
/// Directory of the git root containing a cwd.
pub type GitRoot = Box<dyn Fn(&str) -> Option<String> + Send>;

/// Everything the daemon needs from the OS.
pub struct Platform {
    pub source: Box<dyn ProcessSource + Send>,
    pub signaler: Box<dyn Signaler + Send>,
    pub spawner: Box<dyn Spawner + Send>,
    pub store: Box<dyn StateStore + Send>,
    pub config: Box<dyn ConfigStore + Send>,
    pub clock: Box<dyn Clock + Send>,
    pub open_url: OpenUrl,
    /// Appends one line to `lull.log`.
    pub log: Box<dyn FnMut(&str) + Send>,
    /// `discovery::find_git_root` in production.
    pub git_root: GitRoot,
}

/// Fixed for the life of the daemon.
pub struct Settings {
    pub home: String,
    pub uid: u32,
    pub daemon_pid: u32,
    pub daemon_pgid: u32,
    /// `~/Library/Logs/lull` (restart logs).
    pub restart_log_dir: String,
    pub dry_run: bool,
    pub scan_interval_ms: u64,
}

struct Conn {
    client: Option<ClientKind>,
    subscribed: bool,
    /// The peer process carries an agent marker: `servers.open` stays in the background (§11).
    agent_peer: bool,
}

pub struct Daemon {
    p: Platform,
    s: Settings,
    config: Config,
    config_text: Option<String>,
    config_error: Option<String>,
    discovery: Discovery,
    lifecycle: Lifecycle,
    activity: Activity,
    covered: CoveredBrowsers,
    conns: BTreeMap<ConnId, Conn>,
    /// Last wire form sent to subscribers, for `server.added` / `server.updated`.
    sent: BTreeMap<String, Server>,
    seq: u64,
    first_scan: bool,
    scan_now: bool,
    warnings: Vec<String>,
    status: Value,
}

const KNOWN_METHODS: &[&str] = &[
    "hello",
    "servers.list",
    "servers.stop",
    "servers.pin",
    "servers.setAutoStop",
    "servers.touch",
    "servers.history",
    "servers.restart",
    "servers.forget",
    "servers.open",
    "config.get",
    "config.set",
    "subscribe",
    "browser.tabs.sync",
    "browser.activity",
];

struct RpcErr(i64, String, Option<Value>);

fn invalid(msg: impl Into<String>) -> RpcErr {
    RpcErr(INVALID_PARAMS, msg.into(), None)
}

impl Daemon {
    pub fn new(
        mut p: Platform,
        s: Settings,
        ntstat: Option<Box<dyn ActivityBackend + Send>>,
    ) -> Daemon {
        let now = p.clock.now();
        let wall = p.clock.wall_now_ms();
        let (lifecycle, warnings) = Lifecycle::load(p.store.as_mut(), &now);
        let mut d = Daemon {
            activity: Activity::new(ntstat, &now, wall),
            p,
            s,
            config: Config::default(),
            config_text: None,
            config_error: None,
            discovery: Discovery::default(),
            lifecycle,
            covered: CoveredBrowsers::default(),
            conns: BTreeMap::new(),
            sent: BTreeMap::new(),
            seq: 0,
            first_scan: true,
            scan_now: false,
            warnings,
            status: Value::Null,
        };
        for w in d.warnings.clone() {
            (d.p.log)(&w);
        }
        d.reload_config();
        d
    }

    // ───────────────────────── connections ─────────────────────────

    pub fn connected(&mut self, conn: ConnId, agent_peer: bool) {
        self.conns.insert(
            conn,
            Conn {
                client: None,
                subscribed: false,
                agent_peer,
            },
        );
    }

    pub fn disconnected(&mut self, conn: ConnId) {
        self.conns.remove(&conn);
    }

    /// Handle one NDJSON line from `conn`.
    pub fn handle_line(&mut self, conn: ConnId, line: &str) -> Vec<Out> {
        let mut out = Vec::new();
        let msg: Message = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                let err = json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}});
                out.push(Out {
                    conn,
                    line: err.to_string(),
                });
                return out;
            }
        };
        if let Message::Request(req) = msg {
            let reply = match self.request(conn, &req) {
                Ok(result) => json!({"jsonrpc": "2.0", "id": req.id, "result": result}),
                Err(RpcErr(code, message, data)) => {
                    let mut error = json!({"code": code, "message": message});
                    if let Some(data) = data {
                        error["data"] = data;
                    }
                    json!({"jsonrpc": "2.0", "id": req.id, "error": error})
                }
            };
            out.push(Out {
                conn,
                line: reply.to_string(),
            });
            // Acknowledged mutations are on disk before the next scan (§4.7); `save`
            // writes only when something changed.
            if let Err(e) = self.lifecycle.save(self.p.store.as_mut()) {
                (self.p.log)(&format!("cannot save state: {e}"));
            }
        }
        // Responses (browser command acks) and notifications from clients: nothing in M1.
        out.extend(self.changes());
        out
    }

    /// Whether a request asked for an immediate scan (manual stop). Cleared on read.
    pub fn take_scan_request(&mut self) -> bool {
        std::mem::take(&mut self.scan_now)
    }

    fn request(&mut self, conn: ConnId, req: &Request) -> Result<Value, RpcErr> {
        let typed = match req.typed() {
            Ok(AnyRequest::Client(r)) => r,
            Ok(AnyRequest::Browser(_)) => return Err(invalid("not a client request")),
            Err(e) if KNOWN_METHODS.contains(&req.method.as_str()) => {
                return Err(invalid(e.to_string()));
            }
            Err(_) => {
                return Err(RpcErr(
                    METHOD_NOT_FOUND,
                    format!("unknown method {}", req.method),
                    None,
                ));
            }
        };
        let now = self.p.clock.now();
        let wall = self.p.clock.wall_now_ms();
        if let ClientRequest::Hello(h) = &typed {
            if h.protocol.major != PROTOCOL_VERSION.major {
                let data = ProtocolMismatch {
                    daemon_version: env!("CARGO_PKG_VERSION").into(),
                    protocol: PROTOCOL_VERSION,
                };
                return Err(RpcErr(
                    PROTOCOL_MISMATCH,
                    "protocol_mismatch".into(),
                    Some(json!(data)),
                ));
            }
            if let Some(c) = self.conns.get_mut(&conn) {
                c.client = Some(h.client);
            }
            return Ok(json!(HelloResult {
                daemon_version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                connection_id: format!("c{conn}"),
                capabilities: vec!["servers".into(), "history".into(), "restart".into()],
                activity_backend: self.activity.kind(),
            }));
        }
        if self.conns.get(&conn).is_none_or(|c| c.client.is_none()) {
            return Err(RpcErr(-32600, "send hello first".into(), None));
        }
        let unknown = |e: LifecycleError, id: &str| match e {
            LifecycleError::UnknownId => invalid(format!("unknown server {id}")),
            LifecycleError::NotRestartable => RpcErr(
                NOT_RESTARTABLE,
                "not_restartable: no launch command recorded; copy the command instead".into(),
                None,
            ),
            LifecycleError::AlreadyRestarting => invalid(format!("{id} is already restarting")),
            LifecycleError::SpawnFailed(e) => invalid(format!("restart of {id} failed: {e}")),
        };
        match typed {
            ClientRequest::Hello(_) => unreachable!(),
            ClientRequest::ServersList(_) => Ok(json!(self.servers())),
            ClientRequest::ServersStop(p) => {
                if self.lifecycle.request_stop(&p.id).is_ok() {
                    self.scan_now = true;
                } else if !self.lifecycle.history().iter().any(|h| h.id == p.id) {
                    return Err(invalid(format!("unknown server {}", p.id)));
                }
                Ok(json!({}))
            }
            ClientRequest::ServersPin(p) => {
                self.lifecycle
                    .pin(&p.id, p.pinned, &now, wall)
                    .map_err(|e| unknown(e, &p.id))?;
                self.server_json(&p.id)
            }
            ClientRequest::ServersSetAutoStop(p) => {
                self.lifecycle
                    .set_auto_stop(&p.id, p.auto_stop)
                    .map_err(|e| unknown(e, &p.id))?;
                self.server_json(&p.id)
            }
            ClientRequest::ServersTouch(p) => {
                self.server_json(&p.id)?; // unknown id → error, no record created
                self.manual(&p.id, now, wall);
                self.server_json(&p.id)
            }
            ClientRequest::ServersHistory(_) => Ok(json!(self.lifecycle.history())),
            ClientRequest::ServersRestart(p) => {
                let pid = self
                    .lifecycle
                    .restart(
                        &p.id,
                        self.p.spawner.as_mut(),
                        &self.s.restart_log_dir,
                        &now,
                    )
                    .map_err(|e| unknown(e, &p.id))?;
                (self.p.log)(&format!("restart {} (pid {pid})", p.id));
                Ok(json!(RestartResult { pid }))
            }
            ClientRequest::ServersForget(p) => {
                self.lifecycle.forget(&p.id);
                Ok(json!({}))
            }
            ClientRequest::ServersOpen(p) => self.open(conn, p, now, wall),
            ClientRequest::ConfigGet(_) => Ok(json!(self.config)),
            ClientRequest::ConfigSet(p) => {
                let config = policy::apply_patch(&self.config, &p.patch).map_err(invalid)?;
                let text = policy::to_toml(&config);
                self.p
                    .config
                    .save(&text)
                    .map_err(|e| invalid(format!("cannot write config: {e}")))?;
                self.config_text = Some(text);
                self.config = config;
                self.config_error = None;
                Ok(json!(self.config))
            }
            ClientRequest::Subscribe(_) => {
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.subscribed = true;
                }
                let servers = self.servers();
                self.sent = servers.iter().map(|s| (s.id.clone(), s.clone())).collect();
                Ok(json!(SubscribeResult {
                    seq: self.seq,
                    servers,
                }))
            }
            // ponytail: browser methods are accepted and ignored until WP2.4.
            ClientRequest::BrowserTabsSync(_) | ClientRequest::BrowserActivity(_) => Ok(json!({})),
        }
    }

    fn manual(&mut self, id: &str, at: Instant, wall: u64) {
        let ev = ActivityEvent {
            server_id: id.to_owned(),
            source: lull_protocol::Activity::Manual,
            at,
        };
        self.lifecycle.activity(ev, wall);
    }

    fn open(
        &mut self,
        conn: ConnId,
        p: OpenParams,
        now: Instant,
        wall: u64,
    ) -> Result<Value, RpcErr> {
        let (url, id, focus) = match p {
            OpenParams::Server { id, path, focus } => {
                let server = self
                    .observed(&id)
                    .ok_or_else(|| invalid(format!("unknown server {id}")))?;
                let base = server
                    .primary_url
                    .clone()
                    .ok_or_else(|| invalid(format!("{id} has no listening port")))?;
                let path = path.unwrap_or_default();
                (
                    format!("{base}{}", path.trim_start_matches('/')),
                    Some(id),
                    focus,
                )
            }
            OpenParams::Url { url, focus } => {
                let port = loopback_url_port(&url).ok_or_else(|| {
                    invalid("url must be http(s) with a loopback or *.localhost host")
                })?;
                let id = self
                    .discovery
                    .servers
                    .iter()
                    .find(|s| s.ports.contains(&port))
                    .map(|s| s.id.clone());
                (url, id, focus)
            }
        };
        if let Some(id) = &id {
            self.manual(id, now, wall);
        }
        let agent = self.conns.get(&conn).is_some_and(|c| c.agent_peer);
        let background = !focus.unwrap_or(!agent);
        (self.p.open_url)(&url, &self.config.open_with, background)
            .map_err(|e| invalid(format!("open failed: {e}")))?;
        Ok(json!(OpenResult {
            opened: false,
            browser: None,
        }))
    }

    // ───────────────────────── scanning ─────────────────────────

    /// One scan: snapshot → discovery → activity → lifecycle, then notifications.
    pub fn scan(&mut self) -> Vec<Out> {
        self.reload_config();
        let now = self.p.clock.now();
        let wall = self.p.clock.wall_now_ms();
        let resume = if self.first_scan {
            self.lifecycle.known_groups()
        } else {
            Vec::new()
        };
        // Known groups are read even without a listener (§4.8); on the first scan that
        // includes the ones restored from state.json.
        let also: Vec<u32> = self
            .discovery
            .servers
            .iter()
            .map(|s| s.pgid)
            .chain(resume.iter().map(|k| k.pgid))
            .collect();
        let snapshot = match self.p.source.snapshot(&also) {
            Ok(s) => s,
            Err(e) => {
                (self.p.log)(&format!("scan failed: {e}"));
                return Vec::new();
            }
        };
        let ctx = Context {
            home: &self.s.home,
            uid: self.s.uid,
            daemon_pid: self.s.daemon_pid,
            extra_markers: &self.config.agent_markers,
            git_root: &*self.p.git_root,
        };
        let discovery = discovery::discover(&snapshot, &ctx, &self.discovery.servers, &resume);
        let minute = crate::model::minute_ms();
        let grace_ms = u64::from(self.config.wake_grace_minutes) * minute;
        let events = self.activity.scan(ScanInput {
            snapshot: &snapshot,
            discovery: &discovery,
            covered: &self.covered,
            daemon_pid: self.s.daemon_pid,
            now: now.clone(),
            wall_ms: wall,
            scan_interval_ms: self.s.scan_interval_ms,
        });
        let hold_stops = self.activity.holds_stops(&now, grace_ms);
        let protected =
            |members: &[crate::model::ProcessInfo]| discovery::is_protected(members, &ctx);
        let effects = self.lifecycle.tick(
            Tick {
                now: now.clone(),
                wall_ms: wall,
                discovery: &discovery,
                events,
                config: &self.config,
                home: &self.s.home,
                hold_stops,
                dry_run: self.s.dry_run,
                log_dir: &self.s.restart_log_dir,
            },
            &mut Sys {
                signaler: self.p.signaler.as_mut(),
                source: self.p.source.as_mut(),
                protected: &protected,
                uid: self.s.uid,
                daemon_pgid: self.s.daemon_pgid,
            },
        );
        self.discovery = discovery;
        self.first_scan = false;

        let mut out = Vec::new();
        // Lifecycle already logs every outcome as an `Effect::Log` line.
        for effect in effects {
            match effect {
                Effect::Removed { id, reason } => {
                    self.sent.remove(&id);
                    self.seq += 1;
                    let n = json!({"jsonrpc": "2.0", "method": "server.removed",
                        "params": {"seq": self.seq, "id": id, "reason": reason}});
                    out.extend(self.broadcast(&n));
                }
                Effect::Log(line) => (self.p.log)(&line),
                // ponytail: tab reload after a restart arrives with the browser registry
                // (WP2.4); M1 has nothing else to do for these.
                Effect::StopAborted { .. }
                | Effect::StopStuck { .. }
                | Effect::RestartSucceeded { .. }
                | Effect::RestartFailed { .. } => {}
            }
        }
        if let Err(e) = self.lifecycle.save(self.p.store.as_mut()) {
            (self.p.log)(&format!("cannot save state: {e}"));
        }
        out.extend(self.changes());
        out
    }

    /// Milliseconds until the next scan is due.
    pub fn next_scan_in_ms(&self) -> u64 {
        let now = self.p.clock.now();
        let deadline = self.lifecycle.next_deadline().and_then(|d| {
            d.ms_since(&now)
                .map(|_| d.uptime_ms.saturating_sub(now.uptime_ms))
        });
        deadline.map_or(self.s.scan_interval_ms, |d| d.min(self.s.scan_interval_ms))
    }

    fn reload_config(&mut self) {
        let text = match self.p.config.load() {
            Ok(t) => t,
            Err(e) => {
                self.config_error = Some(format!("cannot read config: {e}"));
                return;
            }
        };
        if text == self.config_text && !self.first_scan {
            return;
        }
        let parsed = match &text {
            None => Ok(Config::default()),
            Some(t) => policy::parse_toml(t),
        };
        match parsed {
            Ok(config) => {
                self.config = config;
                self.config_error = None;
            }
            Err(e) => {
                if self.config_error.as_ref() != Some(&e) {
                    (self.p.log)(&format!(
                        "config.toml rejected, keeping the last good config: {e}"
                    ));
                }
                self.config_error = Some(e);
            }
        }
        self.config_text = text;
    }

    // ───────────────────────── views ─────────────────────────

    fn observed(&self, id: &str) -> Option<&ObservedServer> {
        self.discovery.servers.iter().find(|s| s.id == id)
    }

    fn server_json(&self, id: &str) -> Result<Value, RpcErr> {
        self.observed(id)
            .and_then(|o| self.wire(o))
            .map(|s| json!(s))
            .ok_or_else(|| invalid(format!("unknown server {id}")))
    }

    /// Every live server in wire form (§4 data model).
    pub fn servers(&self) -> Vec<Server> {
        self.discovery
            .servers
            .iter()
            .filter_map(|o| self.wire(o))
            .collect()
    }

    fn wire(&self, o: &ObservedServer) -> Option<Server> {
        let rec = self.lifecycle.record(&o.id)?;
        let policy = self.lifecycle.policy(o, &self.config, &self.s.home);
        let now = self.p.clock.now();
        let wall = self.p.clock.wall_now_ms();
        let stops_at = rec.stops_at.as_ref().map(|at| {
            let ahead = at.uptime_ms.saturating_sub(now.uptime_ms);
            rfc3339(wall + ahead)
        });
        Some(Server {
            id: o.id.clone(),
            pgid: o.pgid,
            pids: o.members.iter().map(|m| m.pid).collect(),
            command: o.command.clone(),
            argv: o.launch_argv.clone(),
            cwd: o.cwd.clone(),
            project: o.project.clone(),
            framework: o.framework,
            ports: o.ports.clone(),
            primary_url: o.primary_url.clone(),
            origin: o.origin.clone(),
            started_at: rfc3339(o.started_at_ms),
            cpu_percent: (o.cpu_percent * 10.0).round() / 10.0,
            rss_bytes: o.rss_bytes,
            last_activity_at: rfc3339(rec.last_activity_wall_ms),
            last_activity_source: rec.last_activity_source,
            idle_minutes: policy.idle_minutes,
            auto_stop: policy.auto_stop,
            pinned: rec.pinned,
            state: rec.state,
            stops_at,
            tabs: Vec::new(),
        })
    }

    /// `server.added` / `server.updated` for every change since the last call. CPU and
    /// RSS alone do not count as a change.
    fn changes(&mut self) -> Vec<Out> {
        if !self.conns.values().any(|c| c.subscribed) {
            return Vec::new();
        }
        let mut notes = Vec::new();
        for server in self.servers() {
            let method = match self.sent.get(&server.id) {
                None => "server.added",
                Some(old) if !same_but_load(old, &server) => "server.updated",
                Some(_) => continue,
            };
            self.seq += 1;
            notes.push(json!({"jsonrpc": "2.0", "method": method,
                "params": {"seq": self.seq, "server": server}}));
            self.sent.insert(server.id.clone(), server);
        }
        notes.iter().flat_map(|n| self.broadcast(n)).collect()
    }

    fn broadcast(&self, note: &Value) -> Vec<Out> {
        let line = note.to_string();
        self.conns
            .iter()
            .filter(|(_, c)| c.subscribed)
            .map(|(id, _)| Out {
                conn: *id,
                line: line.clone(),
            })
            .collect()
    }

    /// Internal status for `lull doctor` (not part of the protocol).
    // ponytail: doctor reads this as `status.json`, not through a protocol method.
    // Upgrade path: a `daemon.status` method when the v1.1 app needs the same data.
    pub fn status(&mut self) -> Option<&Value> {
        let ineligible: BTreeMap<_, _> = self
            .discovery
            .servers
            .iter()
            .filter_map(|s| Some((s.id.clone(), self.lifecycle.ineligible_reason(&s.id)?)))
            .collect();
        let status = json!({
            "pid": self.s.daemon_pid,
            "version": env!("CARGO_PKG_VERSION"),
            "dryRun": self.s.dry_run,
            "activityBackend": self.activity.kind(),
            "activityHealth": format!("{:?}", self.activity.health()),
            "configError": self.config_error,
            "warnings": self.warnings,
            "stops": self.lifecycle.doctor(),
            "ineligible": ineligible,
            "browsers": [],
        });
        if status == self.status {
            return None;
        }
        self.status = status;
        Some(&self.status)
    }
}

fn same_but_load(a: &Server, b: &Server) -> bool {
    let strip = |s: &Server| Server {
        cpu_percent: 0.0,
        rss_bytes: 0,
        ..s.clone()
    };
    strip(a) == strip(b)
}

/// The port of an `http(s)` URL whose host is loopback or `*.localhost` (§8
/// `servers.open`); `None` for anything else.
pub fn loopback_url_port(url: &str) -> Option<u16> {
    let (scheme, rest) = url.split_once("://")?;
    let default = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !p.contains(']') => (h, p.parse().ok()?),
        _ => (authority, default),
    };
    let host = host.to_ascii_lowercase();
    // Only plain DNS characters: a browser reads `\`, `%` and the like differently from
    // this parser (`http://example.com\.localhost/` is example.com to a browser).
    let plain = host == "[::1]"
        || host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let loopback = plain
        && (host == "localhost"
            || host.ends_with(".localhost")
            || host == "[::1]"
            || host.strip_prefix("127.").is_some_and(|r| {
                r.split('.').count() == 3 && r.split('.').all(|o| o.parse::<u8>().is_ok())
            }));
    loopback.then_some(port)
}

#[cfg(test)]
mod tests;
