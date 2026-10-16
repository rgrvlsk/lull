//! Effective policy and config validation (contract §6).

use std::collections::BTreeSet;

use lull_protocol::server::OriginKind;
use lull_protocol::{AutoStop, Config, ConfigPatch, ProjectRule};

use crate::model::{EffectivePolicy, ObservedServer};

/// Effective policy of one server, per field: per-server override → the longest matching
/// `[[project]]` → defaults. `auto_stop = "agent"` means agent-started servers only.
pub fn effective(
    config: &Config,
    server: &ObservedServer,
    auto_stop_override: Option<bool>,
    home: &str,
) -> EffectivePolicy {
    let rule = project_rule(config, &server.cwd, home);
    let idle_minutes = rule
        .and_then(|r| r.idle_minutes)
        .unwrap_or(config.idle_minutes);
    let mode = rule.and_then(|r| r.auto_stop).unwrap_or(config.auto_stop);
    let auto_stop = auto_stop_override.unwrap_or(match mode {
        AutoStop::Agent => server.origin.kind == OriginKind::Agent,
        AutoStop::All => true,
        AutoStop::None => false,
    });
    EffectivePolicy {
        idle_minutes,
        auto_stop,
    }
}

/// Why the settings leave a server out of auto-stop, naming the setting that did (§6):
/// `config: auto_stop = "none"`, `[[project]] ~/dev/x: auto_stop = "none"`, or
/// `config: auto_stop = "agent", and you started it`. When the settings would include the
/// server, only its per-server toggle can have turned auto-stop off.
pub fn auto_stop_off(
    config: &Config,
    cwd: &str,
    kind: OriginKind,
    restarted: bool,
    home: &str,
) -> String {
    let (mode, from) = match project_rule(config, cwd, home).and_then(|r| Some((r.auto_stop?, r))) {
        Some((mode, r)) => (mode, format!("[[project]] {}", r.path)),
        None => (config.auto_stop, "config".to_owned()),
    };
    match mode {
        AutoStop::None => format!("{from}: auto_stop = \"none\""),
        AutoStop::Agent if kind == OriginKind::User => {
            let who = if restarted {
                "lull restarted"
            } else {
                "you started"
            };
            format!("{from}: auto_stop = \"agent\", and {who} it")
        }
        _ => "auto-stop turned off for this server".into(),
    }
}

/// The longest `[[project]]` rule covering `cwd`, if any.
pub fn project_rule<'a>(config: &'a Config, cwd: &str, home: &str) -> Option<&'a ProjectRule> {
    config
        .projects
        .iter()
        .filter_map(|rule| {
            let path = expand(&rule.path, home);
            covers(&path, cwd).then_some((path.len(), rule))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, rule)| rule)
}

/// `~` expanded, trailing slashes dropped.
fn expand(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    let full = match path.strip_prefix('~') {
        Some("") => home.to_owned(),
        Some(rest) if rest.starts_with('/') => format!("{home}{rest}"),
        _ => path.to_owned(),
    };
    full.trim_end_matches('/').to_owned()
}

/// Prefix match on whole path components (`""` is the root after trimming).
fn covers(prefix: &str, cwd: &str) -> bool {
    cwd.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Rejects a config that cannot be applied; the caller keeps the previous one.
pub fn validate(config: &Config) -> Result<(), String> {
    if config.idle_minutes == 0 {
        return Err("idle_minutes must be at least 1".into());
    }
    let mut paths = BTreeSet::new();
    for rule in &config.projects {
        let path = &rule.path;
        if !(path.starts_with('/') || path == "~" || path.starts_with("~/")) {
            return Err(format!(
                "project path {path:?} must be absolute or start with ~/"
            ));
        }
        if rule.idle_minutes == Some(0) {
            return Err(format!("project {path:?}: idle_minutes must be at least 1"));
        }
        if !paths.insert(path.trim_end_matches('/')) {
            return Err(format!("project path {path:?} appears twice"));
        }
    }
    for marker in &config.agent_markers {
        if marker.env.is_empty() || marker.env.contains('=') {
            return Err(format!(
                "agent_marker env {:?} is not a variable name",
                marker.env
            ));
        }
        if marker.agent.is_empty() {
            return Err(format!("agent_marker {:?} needs an agent name", marker.env));
        }
    }
    Ok(())
}

/// `config` with `patch` applied; the whole patch is rejected if the result is invalid.
pub fn apply_patch(config: &Config, patch: &ConfigPatch) -> Result<Config, String> {
    let ConfigPatch {
        idle_minutes,
        auto_stop,
        warn_before_minutes,
        wake_grace_minutes,
        tabs_on_stop,
        open_with,
        history_size,
        history_hours,
        projects,
        agent_markers,
    } = patch.clone();
    let mut next = config.clone();
    if let Some(v) = idle_minutes {
        next.idle_minutes = v;
    }
    if let Some(v) = auto_stop {
        next.auto_stop = v;
    }
    if let Some(v) = warn_before_minutes {
        next.warn_before_minutes = v;
    }
    if let Some(v) = wake_grace_minutes {
        next.wake_grace_minutes = v;
    }
    if let Some(v) = tabs_on_stop {
        next.tabs_on_stop = v;
    }
    if let Some(v) = open_with {
        next.open_with = v;
    }
    if let Some(v) = history_size {
        next.history_size = v;
    }
    if let Some(v) = history_hours {
        next.history_hours = v;
    }
    if let Some(v) = projects {
        next.projects = v;
    }
    if let Some(v) = agent_markers {
        next.agent_markers = v;
    }
    validate(&next)?;
    Ok(next)
}

/// Parses and validates `config.toml`.
pub fn parse_toml(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    validate(&config)?;
    Ok(config)
}

pub fn to_toml(config: &Config) -> String {
    toml::to_string(config).expect("Config always serialises to TOML")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_protocol::server::Origin;
    use lull_protocol::{Framework, ProjectRule};

    fn server(cwd: &str, kind: OriginKind) -> ObservedServer {
        ObservedServer {
            id: "100-1".into(),
            pgid: 100,
            members: vec![],
            ports: vec![5173],
            primary_url: None,
            cwd: cwd.into(),
            project: "p".into(),
            framework: Framework::Vite,
            origin: Origin {
                kind,
                agent: None,
                session_id: None,
            },
            command: String::new(),
            launch_argv: vec![],
            restart_env: Default::default(),
            env_pin: false,
            started_at_ms: 0,
            cpu_percent: 0.0,
            rss_bytes: 0,
            incomplete: vec![],
        }
    }

    fn rule(path: &str, idle: Option<u32>, auto: Option<AutoStop>) -> ProjectRule {
        ProjectRule {
            path: path.into(),
            idle_minutes: idle,
            auto_stop: auto,
        }
    }

    #[test]
    fn effective_policy_table() {
        let config = Config {
            projects: vec![
                rule("~/dev", Some(60), None),
                rule("~/dev/shop/", Some(90), Some(AutoStop::None)),
                rule("/tmp", None, Some(AutoStop::All)),
            ],
            ..Config::default()
        };
        let home = "/Users/x";
        use OriginKind::{Agent, User};
        // (cwd, origin, override, idle, auto)
        let cases = [
            ("/Users/x/other", Agent, None, 30, true),
            ("/Users/x/other", User, None, 30, false),
            ("/Users/x/dev/crm", Agent, None, 60, true),
            ("/Users/x/dev/shop", Agent, None, 90, false), // longest rule wins
            ("/Users/x/dev/shop/web", Agent, None, 90, false),
            ("/Users/x/dev/shopping", Agent, None, 60, true), // whole components only
            ("/Users/x/dev/shop", Agent, Some(true), 90, true), // override first
            ("/tmp/site", User, None, 30, true),
            ("/Users/x/other", Agent, Some(false), 30, false),
        ];
        for (cwd, kind, over, idle, auto) in cases {
            let got = effective(&config, &server(cwd, kind), over, home);
            assert_eq!(
                got,
                EffectivePolicy {
                    idle_minutes: idle,
                    auto_stop: auto
                },
                "{cwd} {kind:?} {over:?}"
            );
        }
    }

    #[test]
    fn auto_stop_all_and_none() {
        let user = server("/Users/x/p", OriginKind::User);
        let mut config = Config {
            auto_stop: AutoStop::All,
            ..Config::default()
        };
        assert!(effective(&config, &user, None, "/Users/x").auto_stop);
        config.auto_stop = AutoStop::None;
        let agent = server("/Users/x/p", OriginKind::Agent);
        assert!(!effective(&config, &agent, None, "/Users/x").auto_stop);
    }

    #[test]
    fn invalid_patch_is_rejected_as_a_whole() {
        let config = Config::default();
        let patch = ConfigPatch {
            history_size: Some(5),
            projects: Some(vec![rule("relative/path", Some(10), None)]),
            ..ConfigPatch::default()
        };
        assert!(apply_patch(&config, &patch).is_err());
        let patch = ConfigPatch {
            history_size: Some(5),
            idle_minutes: Some(0),
            ..ConfigPatch::default()
        };
        assert!(apply_patch(&config, &patch).is_err());

        let ok = ConfigPatch {
            history_size: Some(5),
            projects: Some(vec![rule("~/dev", Some(10), None)]),
            ..ConfigPatch::default()
        };
        let next = apply_patch(&config, &ok).unwrap();
        assert_eq!(next.history_size, 5);
        assert_eq!(next.projects.len(), 1);
        assert_eq!(next.idle_minutes, config.idle_minutes);
    }

    #[test]
    fn validation_table() {
        let bad = [
            Config {
                projects: vec![rule("~/a", None, None), rule("~/a/", None, None)],
                ..Config::default()
            },
            Config {
                projects: vec![rule("~/a", Some(0), None)],
                ..Config::default()
            },
            Config {
                agent_markers: vec![lull_protocol::AgentMarker {
                    env: "A=B".into(),
                    value: None,
                    agent: "x".into(),
                }],
                ..Config::default()
            },
        ];
        for config in bad {
            assert!(validate(&config).is_err(), "{config:?}");
        }
        assert!(validate(&Config::default()).is_ok());
    }

    #[test]
    fn toml_round_trip_and_rejection() {
        let config = Config {
            projects: vec![rule("~/dev", Some(90), Some(AutoStop::None))],
            ..Config::default()
        };
        assert_eq!(parse_toml(&to_toml(&config)).unwrap(), config);
        assert!(parse_toml("idle_minutes = 0").is_err());
        assert!(parse_toml("idle_minute = 3").is_err());
        assert_eq!(parse_toml("").unwrap(), Config::default());
    }
}
