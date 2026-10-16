//! Contract §4 data model: a live server as the daemon reports it.

use serde::{Deserialize, Serialize};

/// A live server (contract §4, "Data model").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Server {
    /// `"{pgid}-{oldestMemberStartEpochMs}"`, stable while any member survives (§4.6).
    pub id: String,
    pub pgid: u32,
    pub pids: Vec<u32>,
    /// Human-readable command line of the oldest member, e.g. `npm run dev`.
    pub command: String,
    pub argv: Vec<String>,
    pub cwd: String,
    /// Basename of the git root containing `cwd`, else the `cwd` basename.
    pub project: String,
    pub framework: Framework,
    /// Listening ports; empty while `state` is `no-listener` (§4.8).
    pub ports: Vec<u16>,
    /// `null` while the server has no listener (§4, "primaryUrl").
    pub primary_url: Option<String>,
    pub origin: Origin,
    /// RFC 3339 wall-clock instant.
    pub started_at: String,
    /// Display only, never an activity source (§5).
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    /// RFC 3339 wall-clock instant, display only (§5, "Clock").
    pub last_activity_at: String,
    pub last_activity_source: Activity,
    /// Effective policy value (§6).
    pub idle_minutes: u32,
    /// Effective policy value (§6).
    pub auto_stop: bool,
    pub pinned: bool,
    /// Where the pin came from (protocol 1.3); `null` when not pinned.
    pub pin_source: Option<PinSource>,
    pub state: ServerState,
    /// `null` when `autoStop` is false or the server is pinned.
    pub stops_at: Option<String>,
    pub tabs: Vec<TabRef>,
}

/// Who started a server (§4.5, ADR-0001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    pub kind: OriginKind,
    /// Present only when `kind` is `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    /// From `CLAUDE_CODE_SESSION_ID` or `CODEX_THREAD_ID` when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OriginKind {
    Agent,
    User,
}

/// The agent that started a server. The list is open (§4.5 "…"): agents added through
/// `[[agent_marker]]` travel as their configured name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Agent {
    Known(KnownAgent),
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnownAgent {
    Claude,
    Codex,
    Cursor,
    Gemini,
    Opencode,
    Copilot,
    Unknown,
}

/// Framework guessed from argv (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Framework {
    Vite,
    Next,
    Astro,
    Nuxt,
    Remix,
    Wrangler,
    PhpBuiltin,
    Artisan,
    PythonHttp,
    Rails,
    Django,
    Generic,
}

/// Activity source (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Activity {
    Tab,
    Traffic,
    Started,
    Manual,
}

/// `env`: the server started with `LULL_KEEP=1`; `user`: a pin request (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PinSource {
    Env,
    User,
}

/// Stop state machine position (§7) plus the lost-listener state (§4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerState {
    Running,
    StoppingSoon,
    Stopping,
    NoListener,
}

/// A browser tab bound to a server, keyed by the browser connection (§8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TabRef {
    pub connection_id: String,
    /// Browser name as announced in `hello.browser.name`, e.g. `helium`.
    pub browser: String,
    pub tab_id: i64,
    pub window_id: i64,
}
