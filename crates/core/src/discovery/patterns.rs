//! The pattern tables: protected groups (§4.4), agent markers (§4.5, ADR-0001) and the
//! browser bundles shared with `lull browsers install` (§9). Every entry is covered by a
//! snapshot fixture (`tests::every_protected_pattern_has_a_fixture`).

use std::collections::BTreeMap;

use lull_protocol::AgentMarker;
use lull_protocol::server::Agent;

use super::{Context, basename};
use crate::model::ProcessInfo;

/// App bundles of the browsers lull knows (§9), matched as a whole path component of the
/// executable wherever the bundle is installed (helpers live inside the same bundle).
pub const BROWSER_BUNDLES: &[&str] = &[
    "Helium.app",
    "Google Chrome.app",
    "Google Chrome Beta.app",
    "Google Chrome Dev.app",
    "Google Chrome Canary.app",
    "Google Chrome for Testing.app",
    "Chromium.app",
    "Brave Browser.app",
    "Microsoft Edge.app",
    "Arc.app",
    "Vivaldi.app",
    "Opera.app",
    "Firefox.app",
    "Firefox Developer Edition.app",
    "Firefox Nightly.app",
    "Zen.app",
    "Zen Browser.app",
];

/// One way a process makes its whole group protected (§4.4). Decided by executable path and
/// argv, never by process name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protect {
    /// The daemon itself.
    Daemon,
    /// Executable basename (native `claude` / `codex` binaries).
    ExeName(&'static str),
    /// Executable inside a directory whose path ends with this. The native Claude Code
    /// installer runs `~/.local/share/claude/versions/<version>` (basename is the version).
    ExeDir(&'static str),
    /// `node`/`bun` whose script path contains this (npm-installed agents).
    Script(&'static str),
    /// `node`/`bun` whose script basename is this: npm's bin symlink, e.g.
    /// `node /opt/homebrew/bin/codex` (argv keeps the unresolved symlink).
    ScriptName(&'static str),
    /// Executable inside an app bundle whose name starts with this (`T3 Code.app`,
    /// `T3 Code (Nightly).app`).
    App(&'static str),
    /// A direct child of such an app: its parent's executable is inside the bundle.
    AppChild(&'static str),
    /// Executable inside a [`BROWSER_BUNDLES`] bundle.
    Browser,
}

pub(crate) const PROTECTED: &[Protect] = &[
    Protect::Daemon,
    Protect::ExeName("claude"),
    Protect::ExeName("codex"),
    Protect::ExeDir("/claude/versions"),
    Protect::Script("/@anthropic-ai/claude-code/"),
    Protect::Script("/@openai/codex/"),
    Protect::ScriptName("claude"),
    Protect::ScriptName("codex"),
    Protect::App("T3 Code"),
    Protect::AppChild("T3 Code"),
    Protect::Browser,
];

/// The first [`PROTECTED`] pattern this process matches. Unreadable fields match nothing:
/// completeness is the caller's check (§4.8), so an exiting member never blocks `SIGKILL`.
pub(crate) fn protected_by(p: &ProcessInfo, ctx: &Context) -> Option<Protect> {
    PROTECTED
        .iter()
        .copied()
        .find(|rule| matches(*rule, p, ctx))
}

pub(crate) fn matches(rule: Protect, p: &ProcessInfo, ctx: &Context) -> bool {
    let exe = p.exe.as_deref();
    match rule {
        Protect::Daemon => p.pid == ctx.daemon_pid,
        Protect::ExeName(name) => exe.is_some_and(|e| basename(e) == name),
        Protect::ExeDir(dir) => exe
            .and_then(|e| e.rsplit_once('/'))
            .is_some_and(|(parent, _)| parent.ends_with(dir)),
        Protect::Script(part) => script(p).is_some_and(|s| s.contains(part)),
        Protect::ScriptName(name) => script(p).is_some_and(|s| basename(s) == name),
        Protect::App(prefix) => exe.is_some_and(|e| in_app(e, prefix)),
        Protect::AppChild(prefix) => p.parent_exe.as_deref().is_some_and(|e| in_app(e, prefix)),
        Protect::Browser => exe.is_some_and(|e| e.split('/').any(|c| BROWSER_BUNDLES.contains(&c))),
    }
}

/// The script a `node`/`bun` process runs: its first non-flag argument.
fn script(p: &ProcessInfo) -> Option<&str> {
    let runtime = basename(p.exe.as_deref()?);
    if runtime != "node" && runtime != "bun" {
        return None;
    }
    let argv = p.argv.as_deref()?;
    argv.iter()
        .skip(1)
        .map(String::as_str)
        .find(|a| !a.starts_with('-'))
}

fn in_app(exe: &str, prefix: &str) -> bool {
    exe.split('/')
        .any(|c| c.starts_with(prefix) && c.ends_with(".app"))
}

/// Built-in agent markers (ADR-0001), after `vercel/detect-agent`: `(env, required value,
/// agent)`; `None` means any non-empty value. First match wins, so markers of agents
/// that run inside another agent's environment come first.
///
/// Left out on purpose: `AI_AGENT` (only some agents set it), `T3CODE_CLI_PATH` and
/// `CURSOR_TRACE_ID` (set for everything the host app launches, not only its agent),
/// `COPILOT_GITHUB_TOKEN` (a secret; it would survive snapshot scrubbing), `REPL_ID` and
/// Devin's `/opt/.devin` (cloud-only).
pub(crate) const MARKERS: &[(&str, Option<&str>, &str)] = &[
    ("CLAUDECODE", Some("1"), "claude"),
    ("CODEX_THREAD_ID", None, "codex"),
    ("CODEX_SANDBOX", None, "codex"),
    ("CODEX_CI", None, "codex"),
    ("GEMINI_CLI", None, "gemini"),
    ("OPENCODE_CLIENT", None, "opencode"),
    ("COPILOT_MODEL", None, "copilot"),
    ("COPILOT_ALLOW_ALL", None, "copilot"),
    ("CURSOR_AGENT", None, "cursor"),
    ("CURSOR_EXTENSION_HOST_ROLE", Some("agent-exec"), "cursor"),
    ("AUGMENT_AGENT", None, "augment"),
    ("ANTIGRAVITY_AGENT", None, "antigravity"),
];

/// Env keys carrying the agent session id (§4.5 `origin.sessionId`).
pub(crate) const SESSION_KEYS: &[&str] = &["CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID"];

/// Env keys classification reads besides `PORT`/`HOST`/`NODE_ENV`: the built-in agent
/// markers, the session id and `LULL_KEEP`. A debug snapshot keeps only these.
pub fn is_marker_key(key: &str) -> bool {
    MARKERS.iter().any(|(k, _, _)| *k == key) || SESSION_KEYS.contains(&key) || key == "LULL_KEEP"
}

/// The agent named by the first marker in `env`; config entries are checked first so a
/// user entry can name an agent more precisely than the built-in table.
pub fn marker(env: &BTreeMap<String, String>, extra: &[AgentMarker]) -> Option<Agent> {
    let extra = extra
        .iter()
        .map(|m| (m.env.as_str(), m.value.as_deref(), m.agent.as_str()));
    extra
        .chain(MARKERS.iter().copied())
        .find(|(key, value, _)| match (env.get(*key), value) {
            (Some(v), Some(want)) => v == want,
            (Some(v), None) => !v.is_empty(),
            (None, _) => false,
        })
        .map(|(_, _, agent)| agent_named(agent))
}

/// `"claude"` → `Agent::Known(Claude)`, anything unknown → `Agent::Other`.
fn agent_named(name: &str) -> Agent {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .unwrap_or_else(|_| Agent::Other(name.to_owned()))
}
