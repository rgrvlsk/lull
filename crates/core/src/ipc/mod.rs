//! WP1.5 — the daemon's state and its JSON-RPC handling (contract §8), independent of the
//! transport: the runtime in `crates/cli` feeds it lines and connection events and sends
//! the [`Out`] lines it returns. Every platform touch point goes through [`Platform`].

use std::collections::BTreeMap;
use std::io;

use lull_protocol::rpc::{
    AnyRequest, ClientKind, ClientRequest, HelloResult, INVALID_PARAMS, METHOD_NOT_FOUND, Message,
    NOT_RESTARTABLE, OpenParams, OpenResult, PROTOCOL_MISMATCH, ProtocolMismatch, Request,
    RequestId, RestartResult, SubscribeResult,
};
use lull_protocol::{Config, PROTOCOL_VERSION, Server, StopReason, TabsOnStop};
use serde_json::{Value, json};

use crate::activity::{Activity, ScanInput};
use crate::browser::Registry;
use crate::discovery::{self, Context};
use crate::lifecycle::{Effect, Lifecycle, LifecycleError, Sys, Tick};
use crate::model::{ActivityEvent, Discovery, ObservedServer, rfc3339};
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
    /// `LOCAL_PEERPID`: for a browser connection, the nm-host whose parent is the browser.
    peer_pid: Option<u32>,
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
    browsers: Registry,
    /// `servers.open` requests waiting for a browser's `tab.open` ack, by command id.
    pending: BTreeMap<String, PendingOpen>,
    next_command: u64,
    /// Lines produced while handling something else (browser commands, deferred replies).
    outbox: Vec<Out>,
    /// Set by a request whose reply is sent later (`servers.open` routed to a browser).
    defer_reply: bool,
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

struct PendingOpen {
    client: ConnId,
    browser_conn: ConnId,
    request: RequestId,
    browser: String,
    since: Instant,
}

/// How long a browser has to acknowledge `tab.open` before `servers.open` answers
/// `opened: false`.
const OPEN_ACK_MS: u64 = 3_000;

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
            browsers: Registry::default(),
            pending: BTreeMap::new(),
            next_command: 0,
            outbox: Vec::new(),
            defer_reply: false,
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

    pub fn connected(&mut self, conn: ConnId, peer_pid: Option<u32>, agent_peer: bool) {
        self.conns.insert(
            conn,
            Conn {
                client: None,
                subscribed: false,
                peer_pid,
                agent_peer,
            },
        );
    }

    /// A closed browser connection drops its tabs, visibility and coverage at once (§8).
    pub fn disconnected(&mut self, conn: ConnId) -> Vec<Out> {
        self.conns.remove(&conn);
        self.browsers.disconnect(conn);
        let lost: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.browser_conn == conn || p.client == conn)
            .map(|(id, _)| id.clone())
            .collect();
        for id in lost {
            self.answer_open(&id, false);
        }
        let mut out = std::mem::take(&mut self.outbox);
        out.extend(self.changes());
        out
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
        if let Message::Response(resp) = &msg {
            // A browser's ack of a `tab.*` command (§8).
            // Only the connection the command went to can answer it.
            let from_target = |p: &PendingOpen| p.browser_conn == conn;
            if let RequestId::String(id) = &resp.id
                && self.pending.get(id).is_some_and(from_target)
            {
                let ok = resp.result.as_ref().and_then(|r| r.get("ok")) == Some(&json!(true));
                self.answer_open(id, ok);
            }
        }
        if let Message::Request(req) = msg {
            self.defer_reply = false;
            let reply = match self.request(conn, &req) {
                Ok(_) if self.defer_reply => Value::Null,
                Ok(result) => json!({"jsonrpc": "2.0", "id": req.id, "result": result}),
                Err(RpcErr(code, message, data)) => {
                    let mut error = json!({"code": code, "message": message});
                    if let Some(data) = data {
                        error["data"] = data;
                    }
                    json!({"jsonrpc": "2.0", "id": req.id, "error": error})
                }
            };
            if !reply.is_null() {
                out.push(Out {
                    conn,
                    line: reply.to_string(),
                });
            }
            // Acknowledged mutations are on disk before the next scan (§4.7); `save`
            // writes only when something changed.
            if let Err(e) = self.lifecycle.save(self.p.store.as_mut()) {
                (self.p.log)(&format!("cannot save state: {e}"));
            }
        }
        out.append(&mut self.outbox);
        out.extend(self.changes());
        out
    }

    /// Send a daemon → browser command; returns its id (§8).
    // Callers pass `ports_of` on tab.* so the extension acts only while the tab is still
    // on the server, not merely on the origin it last synced (protocol 1.2, §8).
    fn command(&mut self, conn: ConnId, method: &str, params: Value) -> String {
        self.next_command += 1;
        let id = format!("d{}", self.next_command);
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.outbox.push(Out {
            conn,
            line: line.to_string(),
        });
        id
    }

    /// Reply to a `servers.open` that waited for a browser.
    fn answer_open(&mut self, command_id: &str, opened: bool) {
        let Some(p) = self.pending.remove(command_id) else {
            return;
        };
        let result = OpenResult {
            opened,
            browser: opened.then_some(p.browser),
        };
        let line = json!({"jsonrpc": "2.0", "id": p.request, "result": result});
        self.outbox.push(Out {
            conn: p.client,
            line: line.to_string(),
        });
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
                if h.client == ClientKind::Browser {
                    let name = h.browser.as_ref().map_or("browser", |b| b.name.as_str());
                    let browser_pid = c.peer_pid.and_then(|pid| self.p.source.parent(pid));
                    self.browsers.connect(conn, name.to_owned(), browser_pid);
                }
            }
            return Ok(json!(HelloResult {
                daemon_version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                connection_id: format!("c{conn}"),
                capabilities: ["servers", "browsers", "history", "restart"]
                    .map(String::from)
                    .to_vec(),
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
            ClientRequest::ServersHistory(_) => {
                let mut history = self.lifecycle.history();
                for h in &mut history {
                    h.tabs = self.browsers.tabs_of(&h.id);
                }
                Ok(json!(history))
            }
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
            ClientRequest::ServersOpen(p) => self.open(conn, &req.id, p, now, wall),
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
            ClientRequest::BrowserTabsSync(p) => {
                self.browsers.sync(conn, p.tabs, &self.discovery.servers);
                Ok(json!({}))
            }
            ClientRequest::BrowserActivity(p) => {
                let servers = &self.discovery.servers;
                if let Some(ev) = self
                    .browsers
                    .activity(conn, p.tab_id, &p.url, servers, &now)
                {
                    self.lifecycle.activity(ev, wall);
                }
                Ok(json!({}))
            }
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
        request: &RequestId,
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
            self.manual(id, now.clone(), wall);
        }
        let agent = self.conns.get(&conn).is_some_and(|c| c.agent_peer);
        let background = !focus.unwrap_or(!agent);
        if let Some(target) = self.browsers.open_target() {
            // A bound tab already showing the URL is focused, or left alone for an agent;
            // otherwise a new tab opens in the most recently focused browser. Either way
            // the reply waits for that browser's ack (§8).
            let (browser, id) = match self.browsers.tab_showing(&url) {
                Some((browser, tab)) if !background => {
                    let ports = self.ports_of(id.as_deref(), &url);
                    let params = json!({"tabId": tab, "ports": ports});
                    (browser, self.command(browser, "tab.focus", params))
                }
                Some((browser, _)) => {
                    let name = self.browsers.name(browser).map(str::to_owned);
                    return Ok(json!(OpenResult {
                        opened: true,
                        browser: name,
                    }));
                }
                None => {
                    let params = json!({"url": url, "focus": !background});
                    (target, self.command(target, "tab.open", params))
                }
            };
            let name = self.browsers.name(browser).unwrap_or("browser").to_owned();
            self.pending.insert(
                id,
                PendingOpen {
                    client: conn,
                    browser_conn: browser,
                    request: request.clone(),
                    browser: name,
                    since: now,
                },
            );
            self.defer_reply = true;
            return Ok(Value::Null);
        }
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
        let covered = self.browsers.covered();
        let events = self.activity.scan(ScanInput {
            snapshot: &snapshot,
            discovery: &discovery,
            covered: &covered,
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
                    if reason != StopReason::Exited {
                        self.tabs_on_stop(&id);
                    }
                    self.sent.remove(&id);
                    self.seq += 1;
                    let n = json!({"jsonrpc": "2.0", "method": "server.removed",
                        "params": {"seq": self.seq, "id": id, "reason": reason}});
                    out.extend(self.broadcast(&n));
                }
                Effect::Log(line) => (self.p.log)(&line),
                // The record's tabs now show the new server: reload them (§7 Restart).
                Effect::RestartSucceeded {
                    record_id,
                    server_id,
                } => {
                    self.browsers.rebind(&record_id, &server_id);
                    let ports = self.ports_of(Some(&server_id), "");
                    for (browser, tab_ids) in self.browsers.bound_tabs(&server_id) {
                        let params = json!({"tabIds": tab_ids, "ports": ports});
                        self.command(browser, "tab.reload", params);
                    }
                }
                Effect::StopAborted { .. }
                | Effect::StopStuck { .. }
                | Effect::RestartFailed { .. } => {}
            }
        }
        // A browser that never acknowledged `tab.open` gets `opened: false`.
        let late: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| now.ms_since(&p.since).is_none_or(|ms| ms >= OPEN_ACK_MS))
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            self.answer_open(&id, false);
        }
        if let Err(e) = self.lifecycle.save(self.p.store.as_mut()) {
            (self.p.log)(&format!("cannot save state: {e}"));
        }
        out.append(&mut self.outbox);
        out.extend(self.changes());
        out
    }

    /// `tabs_on_stop` for a server lull stopped (§7.4): discard (the active tab is left
    /// as it is by the browser), close, or keep, in every connected browser.
    fn tabs_on_stop(&mut self, id: &str) {
        let method = match self.config.tabs_on_stop {
            TabsOnStop::Discard => "tab.discard",
            TabsOnStop::Close => "tab.close",
            TabsOnStop::Keep => return,
        };
        let ports = self.ports_of(Some(id), "");
        for (browser, tab_ids) in self.browsers.bound_tabs(id) {
            self.command(browser, method, json!({"tabIds": tab_ids, "ports": ports}));
        }
    }

    /// Ports of a live server or stopped-server record, else the port of `url`: what a
    /// `tab.*` command lets the extension act on (protocol 1.2).
    fn ports_of(&self, id: Option<&str>, url: &str) -> Vec<u16> {
        let live = id.and_then(|id| self.observed(id)).map(|s| s.ports.clone());
        let stopped = || {
            let history = self.lifecycle.history();
            history
                .into_iter()
                .find(|h| Some(h.id.as_str()) == id)
                .map(|h| h.ports)
        };
        live.or_else(stopped)
            .filter(|p| !p.is_empty())
            .or_else(|| loopback_url_port(url).map(|p| vec![p]))
            .unwrap_or_default()
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
            tabs: self.browsers.tabs_of(&o.id),
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
            "browsers": self.browsers.summary().iter().map(|(id, name, pid, tabs)| {
                json!({"connectionId": id, "name": name, "browserPid": pid, "tabs": tabs})
            }).collect::<Vec<_>>(),
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
