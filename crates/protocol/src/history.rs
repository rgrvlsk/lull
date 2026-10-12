//! Contract §7: the stopped-server record kept in history.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::server::{Framework, Origin, TabRef};

/// What lull remembers about a recently stopped server (§7.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoppedServer {
    pub id: String,
    pub project: String,
    pub framework: Framework,
    pub command: String,
    /// The oldest member's argv captured when the server was first seen.
    pub argv: Vec<String>,
    pub cwd: String,
    pub ports: Vec<u16>,
    pub origin: Origin,
    /// RFC 3339 wall-clock instant.
    pub stopped_at: String,
    pub reason: StopReason,
    /// Only `PORT`, `HOST` and `NODE_ENV`, when they were present.
    pub restart_env: BTreeMap<String, String>,
    /// Tabs still bound to this record (§9, "Binding").
    pub tabs: Vec<TabRef>,
    pub status: RestartStatus,
}

/// Why a server went away (§7, §8 `server.removed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    Idle,
    Manual,
    /// The server went away without lull stopping it.
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestartStatus {
    Stopped,
    Restarting,
    RestartFailed,
}
