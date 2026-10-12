//! Contract §6: configuration.
//!
//! One type serves both the TOML file on disk and the `config.get` / `config.set` wire
//! payloads, so field names stay `snake_case` on the wire as well (unlike the rest of
//! the protocol, which is `camelCase`).

use serde::{Deserialize, Serialize};

/// Full configuration with every default applied (§6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub idle_minutes: u32,
    pub auto_stop: AutoStop,
    pub warn_before_minutes: u32,
    pub wake_grace_minutes: u32,
    pub tabs_on_stop: TabsOnStop,
    /// App for `open(1)` when no browser is connected; empty means the system default.
    pub open_with: String,
    pub history_size: u32,
    pub history_hours: u32,
    pub faithful_restart: bool,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectRule>,
    #[serde(rename = "agent_marker", skip_serializing_if = "Vec::is_empty")]
    pub agent_markers: Vec<AgentMarker>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idle_minutes: 30,
            auto_stop: AutoStop::Agent,
            warn_before_minutes: 2,
            wake_grace_minutes: 5,
            tabs_on_stop: TabsOnStop::Discard,
            open_with: String::new(),
            history_size: 20,
            history_hours: 24,
            faithful_restart: false,
            projects: Vec::new(),
            agent_markers: Vec::new(),
        }
    }
}

/// Partial configuration sent as `config.set {patch}`; absent fields are left alone.
/// Lists (`project`, `agent_marker`) are replaced as a whole when present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_stop: Option<AutoStop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn_before_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_grace_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tabs_on_stop: Option<TabsOnStop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_hours: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faithful_restart: Option<bool>,
    #[serde(rename = "project", skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<ProjectRule>>,
    #[serde(rename = "agent_marker", skip_serializing_if = "Option::is_none")]
    pub agent_markers: Option<Vec<AgentMarker>>,
}

/// Which servers are stopped automatically (§6 `auto_stop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoStop {
    Agent,
    All,
    None,
}

/// What happens to a stopped server's tabs (§7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabsOnStop {
    Discard,
    Close,
    Keep,
}

/// A `[[project]]` rule: prefix match on whole path components of `cwd`; the longest
/// matching path wins (§6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRule {
    /// `~` is expanded by the daemon.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_stop: Option<AutoStop>,
}

/// An `[[agent_marker]]` entry extending the built-in marker table (§4.5, ADR-0001).
///
/// The contract names the table but not its fields; this shape mirrors the built-in
/// entries (`CLAUDECODE=1` is `env` + `value`, `CODEX_THREAD_ID` is `env` alone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMarker {
    /// Environment variable name that marks the agent.
    pub env: String,
    /// Required value; when absent, the variable's presence is enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Agent name reported in `origin.agent`.
    pub agent: String,
}
