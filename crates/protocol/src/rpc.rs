//! Contract §8: JSON-RPC 2.0 over NDJSON.
//!
//! Three layers:
//! - the generic envelope ([`Message`], [`Request`], [`Response`], [`Notification`]);
//! - typed method enums, tagged by `method` with the params as content
//!   ([`ClientRequest`], [`BrowserRequest`], [`DaemonNotification`], [`BrowserCommand`]);
//! - the param and result structs of each method.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, ConfigPatch};
use crate::history::{StopReason, StoppedServer};
use crate::server::Server;

/// `hello` was sent with a different `protocol.major` (§8). `error.data` carries a
/// [`ProtocolMismatch`].
pub const PROTOCOL_MISMATCH: i64 = -32010;
/// `servers.restart` on a stopped-server record without a launch command (§7).
pub const NOT_RESTARTABLE: i64 = -32011;
/// JSON-RPC "Invalid params"; used for `config.set` validation errors and rejected
/// `servers.open` URLs.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC "Method not found".
pub const METHOD_NOT_FOUND: i64 = -32601;

// ───────────────────────── envelope ─────────────────────────

/// JSON-RPC request id: number or string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The literal string `"2.0"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JsonRpcVersion {
    #[default]
    #[serde(rename = "2.0")]
    V2,
}

/// Any one line of the NDJSON stream. Order matters for `untagged`: a request has
/// `id` + `method`, a notification `method` only, a response `id` + `result`/`error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Notification(Notification),
    Response(Response),
}

// ───────────────────────── hello ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    App,
    Cli,
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloParams {
    pub client: ClientKind,
    /// Client software version.
    pub version: String,
    pub protocol: ProtocolVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserInfo {
    /// e.g. `helium`, `chrome`, `firefox`, `zen`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub version: String,
    pub extension_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResult {
    pub daemon_version: String,
    pub protocol: ProtocolVersion,
    pub connection_id: String,
    pub capabilities: Vec<String>,
    pub activity_backend: ActivityBackend,
}

/// `error.data` of a [`PROTOCOL_MISMATCH`] error: the daemon's versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolMismatch {
    pub daemon_version: String,
    pub protocol: ProtocolVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityBackend {
    Ntstat,
    Scan,
}

// ───────────────────────── client → daemon ─────────────────────────

/// Every client → daemon method with its typed params (§8, "Client → daemon" and
/// "Browser → daemon").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum ClientRequest {
    #[serde(rename = "hello")]
    Hello(HelloParams),
    #[serde(rename = "servers.list")]
    ServersList(Empty),
    #[serde(rename = "servers.stop")]
    ServersStop(ServerId),
    #[serde(rename = "servers.pin")]
    ServersPin(PinParams),
    #[serde(rename = "servers.setAutoStop")]
    ServersSetAutoStop(SetAutoStopParams),
    #[serde(rename = "servers.touch")]
    ServersTouch(ServerId),
    #[serde(rename = "servers.history")]
    ServersHistory(Empty),
    #[serde(rename = "servers.restart")]
    ServersRestart(ServerId),
    #[serde(rename = "servers.forget")]
    ServersForget(ServerId),
    #[serde(rename = "servers.open")]
    ServersOpen(OpenParams),
    #[serde(rename = "config.get")]
    ConfigGet(Empty),
    #[serde(rename = "config.set")]
    ConfigSet(ConfigSetParams),
    #[serde(rename = "subscribe")]
    Subscribe(SubscribeParams),
    #[serde(rename = "browser.tabs.sync")]
    BrowserTabsSync(TabsSyncParams),
    #[serde(rename = "browser.activity")]
    BrowserActivity(BrowserActivityParams),
}

/// `{}` — a method without parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerId {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinParams {
    pub id: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAutoStopParams {
    pub id: String,
    /// Per-server override; `null` clears it.
    pub auto_stop: Option<bool>,
}

/// `servers.open`: either a known server (optionally with a path) or a bare URL.
/// `focus` overrides the default (focus unless the caller runs under an agent, §11);
/// added in protocol 1.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum OpenParams {
    Server {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus: Option<bool>,
    },
    /// Must be `http(s)` with a loopback or `*.localhost` host.
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenResult {
    /// Whether a connected browser confirmed the open.
    pub opened: bool,
    /// Name of the browser connection that handled it, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartResult {
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSetParams {
    pub patch: ConfigPatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Topic {
    Servers,
    Browsers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    pub topics: Vec<Topic>,
}

/// Full snapshot returned by `subscribe`; later notifications carry `seq + n`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeResult {
    pub seq: u64,
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabsSyncParams {
    pub tabs: Vec<BrowserTab>,
}

/// One browser tab as the extension reports it (§8 `browser.tabs.sync`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserTab {
    pub tab_id: i64,
    pub window_id: i64,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    pub active: bool,
    pub window_focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserActivityParams {
    pub tab_id: i64,
    pub url: String,
}

/// Typed results, by method, for the client → daemon requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClientResult {
    Hello(HelloResult),
    Servers(Vec<Server>),
    Server(Box<Server>),
    History(Vec<StoppedServer>),
    Restart(RestartResult),
    Open(OpenResult),
    Config(Box<Config>),
    Subscribe(SubscribeResult),
    Empty(Empty),
}

// ───────────────────────── daemon → client ─────────────────────────

/// Notifications sent to subscribers (§8). Every one carries the subscription `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum DaemonNotification {
    #[serde(rename = "server.added")]
    ServerAdded(ServerEvent),
    #[serde(rename = "server.updated")]
    ServerUpdated(ServerEvent),
    #[serde(rename = "server.removed")]
    ServerRemoved(ServerRemoved),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEvent {
    pub seq: u64,
    pub server: Server,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRemoved {
    pub seq: u64,
    pub id: String,
    pub reason: StopReason,
}

// ───────────────────────── daemon → browser commands ─────────────────────────

/// Commands the extension must act on (§8). They are JSON-RPC requests (they carry an
/// `id`) and are answered with an [`Ack`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum BrowserCommand {
    #[serde(rename = "tab.open")]
    TabOpen(TabOpen),
    #[serde(rename = "tab.close")]
    TabClose(TabIds),
    #[serde(rename = "tab.discard")]
    TabDiscard(TabIds),
    #[serde(rename = "tab.reload")]
    TabReload(TabIds),
    #[serde(rename = "tab.focus")]
    TabFocus(TabId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabOpen {
    pub url: String,
    pub focus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TabIds {
    pub tab_ids: Vec<i64>,
    /// Ports of the server the command is about (protocol 1.2). The extension acts on a
    /// tab only while its URL is loopback on one of them; absent → origin re-check only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TabId {
    pub tab_id: i64,
    /// As in [`TabIds::ports`] (protocol 1.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
}

/// Acknowledgement of a browser command: `{ok: true}` or `{error}` (§8, delivery rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Ack {
    Ok { ok: bool },
    Error { error: String },
}

// ───────────────────────── requests from either side ─────────────────────────

/// Any typed request: client → daemon or daemon → browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnyRequest {
    Client(ClientRequest),
    Browser(BrowserCommand),
}

impl Request {
    /// Decode `method` + `params` into the typed request enum.
    pub fn typed(&self) -> Result<AnyRequest, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "method": self.method,
            "params": self.params.clone().unwrap_or(Value::Object(Default::default())),
        }))
    }
}

impl Notification {
    /// Decode `method` + `params` into the typed notification enum.
    pub fn typed(&self) -> Result<DaemonNotification, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "method": self.method,
            "params": self.params.clone().unwrap_or(Value::Object(Default::default())),
        }))
    }
}

/// Name of the method each typed client request answers to.
impl ClientRequest {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::ServersList(_) => "servers.list",
            Self::ServersStop(_) => "servers.stop",
            Self::ServersPin(_) => "servers.pin",
            Self::ServersSetAutoStop(_) => "servers.setAutoStop",
            Self::ServersTouch(_) => "servers.touch",
            Self::ServersHistory(_) => "servers.history",
            Self::ServersRestart(_) => "servers.restart",
            Self::ServersForget(_) => "servers.forget",
            Self::ServersOpen(_) => "servers.open",
            Self::ConfigGet(_) => "config.get",
            Self::ConfigSet(_) => "config.set",
            Self::Subscribe(_) => "subscribe",
            Self::BrowserTabsSync(_) => "browser.tabs.sync",
            Self::BrowserActivity(_) => "browser.activity",
        }
    }

    /// Decode the `result` of a response to this request into its typed shape.
    pub fn decode_result(&self, result: &Value) -> Result<ClientResult, serde_json::Error> {
        use serde_json::from_value as de;
        let r = result.clone();
        Ok(match self {
            Self::Hello(_) => ClientResult::Hello(de(r)?),
            Self::ServersList(_) => ClientResult::Servers(de(r)?),
            Self::ServersPin(_) | Self::ServersSetAutoStop(_) | Self::ServersTouch(_) => {
                ClientResult::Server(Box::new(de(r)?))
            }
            Self::ServersHistory(_) => ClientResult::History(de(r)?),
            Self::ServersRestart(_) => ClientResult::Restart(de(r)?),
            Self::ServersOpen(_) => ClientResult::Open(de(r)?),
            Self::ConfigGet(_) | Self::ConfigSet(_) => ClientResult::Config(Box::new(de(r)?)),
            Self::Subscribe(_) => ClientResult::Subscribe(de(r)?),
            Self::ServersStop(_)
            | Self::ServersForget(_)
            | Self::BrowserTabsSync(_)
            | Self::BrowserActivity(_) => ClientResult::Empty(de(r)?),
        })
    }
}
