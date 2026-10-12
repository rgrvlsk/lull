//! WP1.2 — discovery and classification (contract §4): pure functions over a snapshot.
//!
//! [`discover`] groups a [`Snapshot`] by pgid and sorts every group into protected groups
//! (§4.4), servers (§4.3, ADR-0002) and unmanaged listeners, then classifies and labels the
//! servers (§4.5, ADR-0001) and keeps their identity across scans (§4.6–4.7).

mod labels;
mod patterns;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use lull_protocol::server::{Origin, OriginKind};

use crate::model::{
    Discovery, KnownGroup, ObservedServer, ProcessInfo, Snapshot, UnmanagedListener,
};

pub use patterns::{BROWSER_BUNDLES, is_marker_key};

/// What discovery needs besides the snapshot.
pub struct Context<'a> {
    pub home: &'a str,
    pub uid: u32,
    pub daemon_pid: u32,
    /// `[[agent_marker]]` entries from config, on top of the built-in table.
    pub extra_markers: &'a [lull_protocol::AgentMarker],
    /// Directory of the git root containing a cwd (injected; core never touches the fs).
    pub git_root: &'a dyn Fn(&str) -> Option<String>,
}

/// Pure: classify every group of the snapshot. `prev` = previous scan's servers (id
/// stability, sticky classification and launch command); `resume` = identities restored
/// from state.json on daemon start (§4.7).
pub fn discover(
    snap: &Snapshot,
    ctx: &Context,
    prev: &[ObservedServer],
    resume: &[KnownGroup],
) -> Discovery {
    let mut groups: BTreeMap<u32, Vec<&ProcessInfo>> = BTreeMap::new();
    for p in &snap.processes {
        groups.entry(p.pgid).or_default().push(p);
    }
    let mut ports_of: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
    for l in &snap.listeners {
        ports_of.entry(l.pid).or_default().push(l.port);
    }

    let mut out = Discovery::default();
    // §4.8: a known server whose member list could not be read is kept as it was.
    // ponytail: ports and primaryUrl are not refreshed for such a scan; it is transient.
    for s in prev {
        if snap.unreadable_groups.contains(&s.pgid) && !groups.contains_key(&s.pgid) {
            out.servers.push(ObservedServer {
                incomplete: vec!["member list unreadable".into()],
                ..s.clone()
            });
        }
    }

    for (&pgid, members) in &mut groups {
        members.sort_by_key(|p| p.pid);
        let members = members.as_slice();
        if members
            .iter()
            .any(|p| patterns::protected_by(p, ctx).is_some())
        {
            out.protected.insert(pgid);
            continue;
        }
        let mut ports: Vec<u16> = members
            .iter()
            .flat_map(|p| ports_of.get(&p.pid).into_iter().flatten().copied())
            .collect();
        ports.sort_unstable();
        ports.dedup();
        let incomplete = incomplete(pgid, members, snap);
        let prev = prev
            .iter()
            .find(|s| s.pgid == pgid && members.iter().any(|p| s.members.contains(&p.witness())));
        let anchor = members.iter().copied().find(|p| {
            ports_of.contains_key(&p.pid)
                && p.cwd
                    .as_deref()
                    .is_some_and(|c| is_project_location(c, ctx.home))
        });
        let from_app = members.iter().any(|p| {
            p.exe
                .as_deref()
                .is_some_and(|e| is_installed_app(e, ctx.home))
        });
        // A group lull could not signal as a whole is never a server (§7).
        let foreign = members.iter().any(|p| p.uid != ctx.uid);
        // §4.8: a known server keeps its verdict without a listener or with unknowns.
        let is_server = match prev {
            Some(_) if ports.is_empty() || !incomplete.is_empty() => true,
            _ => !ports.is_empty() && anchor.is_some() && !from_app && !foreign,
        };
        if is_server {
            out.servers.push(server(
                pgid, members, ports, anchor, incomplete, prev, resume, ctx,
            ));
        } else if !ports.is_empty() {
            out.unmanaged.push(UnmanagedListener {
                pgid,
                pids: members.iter().map(|p| p.pid).collect(),
                cwd: members
                    .iter()
                    .find(|p| ports_of.contains_key(&p.pid))
                    .and_then(|p| p.cwd.clone()),
                ports,
                command: labels::command_line(oldest(members)),
            });
        }
    }
    out.servers.sort_by_key(|s| s.pgid);
    out
}

#[allow(clippy::too_many_arguments)]
fn server(
    pgid: u32,
    members: &[&ProcessInfo],
    ports: Vec<u16>,
    anchor: Option<&ProcessInfo>,
    incomplete: Vec<String>,
    prev: Option<&ObservedServer>,
    resume: &[KnownGroup],
    ctx: &Context,
) -> ObservedServer {
    let origin = match prev {
        Some(p) if !incomplete.is_empty() => p.origin.clone(),
        _ => origin(members, ctx),
    };
    let primary_url = labels::primary_url(members, &ports);
    let mut s = match prev {
        // Identity and everything captured when first seen are kept (§4.6, §7.6).
        Some(p) => ObservedServer {
            origin,
            ports,
            primary_url,
            incomplete,
            ..p.clone()
        },
        None => {
            // A new server always has an anchor: `is_server` requires one without `prev`.
            let anchor = anchor.unwrap_or(members[0]);
            let cwd = anchor.cwd.clone().unwrap_or_default();
            let project = (ctx.git_root)(&cwd).unwrap_or_else(|| cwd.clone());
            let launch_argv = labels::launch_argv(members, anchor);
            let launcher = members
                .iter()
                .find(|p| !launch_argv.is_empty() && labels::argv(p) == launch_argv)
                .copied()
                .unwrap_or(anchor);
            let oldest = oldest(members);
            let id = resume
                .iter()
                .find(|k| {
                    k.pgid == pgid && members.iter().any(|p| k.witnesses.contains(&p.witness()))
                })
                .map(|k| k.id.clone())
                .unwrap_or_else(|| format!("{pgid}-{}", oldest.start_epoch_ms));
            ObservedServer {
                id,
                pgid,
                members: Vec::new(),
                project: basename(&project).to_owned(),
                cwd,
                framework: labels::framework(members),
                origin,
                command: if launch_argv.is_empty() {
                    labels::command_line(oldest)
                } else {
                    labels::quote(&launch_argv)
                },
                launch_argv,
                restart_env: launcher
                    .env
                    .iter()
                    .flatten()
                    .filter(|(k, _)| matches!(k.as_str(), "PORT" | "HOST" | "NODE_ENV"))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                env_pin: false,
                started_at_ms: oldest.start_epoch_ms,
                cpu_percent: 0.0,
                rss_bytes: 0,
                ports,
                primary_url,
                incomplete,
            }
        }
    };
    s.members = members.iter().map(|p| p.witness()).collect();
    s.env_pin = members.iter().any(|p| {
        p.env
            .as_ref()
            .and_then(|e| e.get("LULL_KEEP"))
            .map(String::as_str)
            == Some("1")
    });
    s.cpu_percent = members.iter().map(|p| p.cpu_percent).sum();
    s.rss_bytes = members.iter().map(|p| p.rss_bytes).sum();
    s
}

/// ADR-0001: agent iff some member carries a marker, no member has (or may have) a
/// controlling tty and no member runs (or may run) from an installed-app location.
fn origin(members: &[&ProcessInfo], ctx: &Context) -> Origin {
    let vetoed = members.iter().any(|p| {
        p.has_tty != Some(false)
            || p.exe
                .as_deref()
                .is_none_or(|e| is_installed_app(e, ctx.home))
    });
    let agent = members.iter().filter(|_| !vetoed).find_map(|p| {
        let env = p.env.as_ref()?;
        let agent = patterns::marker(env, ctx.extra_markers)?;
        let session = patterns::SESSION_KEYS
            .iter()
            .find_map(|k| env.get(*k))
            .cloned();
        Some((agent, session))
    });
    match agent {
        Some((agent, session_id)) => Origin {
            kind: OriginKind::Agent,
            agent: Some(agent),
            session_id,
        },
        None => Origin {
            kind: OriginKind::User,
            agent: None,
            session_id: None,
        },
    }
}

/// §4.8 reasons this scan's observation of the group is incomplete; empty when complete.
fn incomplete(pgid: u32, members: &[&ProcessInfo], snap: &Snapshot) -> Vec<String> {
    let mut reasons = Vec::new();
    if snap.unreadable_groups.contains(&pgid) {
        reasons.push("member list unreadable".to_owned());
    }
    for p in members {
        let unread = [
            ("executable", p.exe.is_none()),
            ("argv", p.argv.is_none()),
            ("env", p.env.is_none()),
            ("cwd", p.cwd.is_none()),
            ("tty", p.has_tty.is_none()),
        ];
        for (field, missing) in unread {
            if missing {
                reasons.push(format!("pid {}: {field} unreadable", p.pid));
            }
        }
    }
    reasons
}

/// §4.4 protected-group test on a fresh member list; lifecycle calls it right before every signal.
pub fn is_protected(members: &[ProcessInfo], ctx: &Context) -> bool {
    members
        .iter()
        .any(|p| patterns::protected_by(p, ctx).is_some())
}

/// Real `git_root` implementation (walks up looking for `.git`); the daemon injects it.
pub fn find_git_root(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(|dir| dir.to_string_lossy().into_owned())
}

/// ADR-0002: strictly inside `$HOME` (not `$HOME`, not `~/Library`) or a temp directory.
fn is_project_location(cwd: &str, home: &str) -> bool {
    let home = home.trim_end_matches('/');
    let under = |root: &str| cwd == root || cwd.starts_with(&format!("{root}/"));
    let in_home = cwd.starts_with(&format!("{home}/")) && !under(&format!("{home}/Library"));
    in_home
        || ["/tmp", "/private/tmp", "/private/var/folders"]
            .into_iter()
            .any(under)
}

/// ADR-0001/0002: `/Applications`, `~/Applications` or `/System`.
fn is_installed_app(exe: &str, home: &str) -> bool {
    let home_apps = format!("{}/Applications/", home.trim_end_matches('/'));
    exe.starts_with("/Applications/") || exe.starts_with(&home_apps) || exe.starts_with("/System/")
}

fn oldest<'a>(members: &[&'a ProcessInfo]) -> &'a ProcessInfo {
    members
        .iter()
        .copied()
        .min_by_key(|p| (p.start_epoch_ms, p.pid))
        .expect("a group has members")
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}
