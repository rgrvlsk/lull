//! WP1.1 — `ProcessSource` over libproc and `KERN_PROCARGS2` (contract §4.1–4.2, §4.8,
//! §5).
//!
//! One scan walks the socket fds of every same-uid process (`socket.rs`), keeps the TCP
//! listeners and established connections, and only then reads process details for the
//! listeners' process groups (plus any requested pgids).
//!
//! Decisions:
//! - **Listener pid** is the pid holding the fd, exact. A listening socket shared by
//!   several processes (fork) yields one `Listener` per holder; identical
//!   (pid, addr, port) entries (dup'd fds) are merged.
//! - **Controlling tty** is `e_tdev != NODEV` (the session still has a terminal), the same
//!   test `ps` uses for its TTY column. A process started under `setsid` without a
//!   terminal reports `Some(false)`.

mod socket;
mod sys;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::net::IpAddr;
use std::time::Instant;

use crate::model::{Connection, Listener, ProcessInfo, Snapshot};
use crate::traits::ProcessSource;

use socket::{Sock, TSI_S_ESTABLISHED, TSI_S_LISTEN};
use sys::Args;

/// Env keys a scrubbed fixture always keeps (§7.6 restart env).
const KEEP_ENV: [&str; 3] = ["PORT", "HOST", "NODE_ENV"];
/// Ancestry walks stop here even without reaching launchd (guards against pid cycles).
const MAX_ANCESTRY: usize = 64;

/// macOS [`ProcessSource`]. Caches argv/env per (pid, start time) and keeps the previous
/// scan's CPU times to report `cpu_percent`.
#[derive(Debug)]
pub struct MacProcessSource {
    uid: u32,
    args: HashMap<(u32, u64), Args>,
    /// (pid, start) → (total CPU ns, when sampled) from the previous snapshot.
    cpu: HashMap<(u32, u64), (u64, Instant)>,
}

impl Default for MacProcessSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MacProcessSource {
    pub fn new() -> Self {
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        Self {
            uid,
            args: HashMap::new(),
            cpu: HashMap::new(),
        }
    }

    /// One member's details; `None` when it exited before it could be read. `cpu` collects
    /// this scan's CPU sample.
    fn member(
        &mut self,
        pid: u32,
        pgid: u32,
        cpu: &mut HashMap<(u32, u64), (u64, Instant)>,
    ) -> Option<ProcessInfo> {
        let mut p = ProcessInfo {
            pid,
            pgid,
            ..ProcessInfo::default()
        };
        match sys::bsdinfo(pid) {
            Ok(b) => {
                p.pgid = b.pbi_pgid;
                p.ppid = b.pbi_ppid;
                p.uid = b.pbi_uid;
                p.start_epoch_ms = b.pbi_start_tvsec * 1000 + b.pbi_start_tvusec / 1000;
                p.has_tty = Some(b.e_tdev != sys::NODEV);
            }
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => return None,
            Err(e) => {
                p.errors.push(format!("bsdinfo: {e}"));
                if let Ok(s) = sys::shortinfo(pid) {
                    (p.pgid, p.ppid, p.uid) = (s.pbsi_pgid, s.pbsi_ppid, s.pbsi_uid);
                }
            }
        }
        let errors = &mut p.errors;
        let mut field = |name: &str, r: io::Result<String>| {
            r.map_err(|e| errors.push(format!("{name}: {e}"))).ok()
        };
        p.exe = field("exe", sys::exe(pid));
        p.parent_exe = match p.ppid {
            0 => None,
            ppid => field("parent_exe", sys::exe(ppid)),
        };
        p.cwd = field("cwd", sys::cwd(pid));

        let key = (pid, p.start_epoch_ms);
        let args = match self.args.get(&key) {
            Some(a) => Ok(a.clone()),
            None => sys::procargs(pid).and_then(|buf| {
                sys::parse_procargs(&buf).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "malformed KERN_PROCARGS2")
                })
            }),
        };
        match args {
            Ok(a) => {
                if a.env.is_none() {
                    p.errors.push("env: omitted by the kernel".into());
                }
                p.argv = Some(a.argv.clone());
                p.env = a.env.clone();
                if p.start_epoch_ms != 0 {
                    self.args.insert(key, a);
                }
            }
            Err(e) => p.errors.push(format!("argv/env: {e}")),
        }

        match sys::taskinfo(pid) {
            Ok(t) => {
                p.rss_bytes = t.pti_resident_size;
                let cpu_ns = sys::ticks_to_ns(t.pti_total_user + t.pti_total_system);
                let now = Instant::now();
                if let Some(&(prev_ns, at)) = self.cpu.get(&key) {
                    let wall = now.duration_since(at).as_nanos() as f64;
                    if wall > 0.0 {
                        p.cpu_percent = cpu_ns.saturating_sub(prev_ns) as f64 / wall * 100.0;
                    }
                }
                cpu.insert(key, (cpu_ns, now));
            }
            Err(e) => p.errors.push(format!("taskinfo: {e}")),
        }
        Some(p)
    }

    fn read_group(
        &mut self,
        pgid: u32,
        cpu: &mut HashMap<(u32, u64), (u64, Instant)>,
    ) -> io::Result<Vec<ProcessInfo>> {
        let pids = sys::group_pids(pgid)?;
        Ok(pids
            .into_iter()
            .filter_map(|pid| self.member(pid, pgid, cpu))
            .collect())
    }

    /// `pid` then its parents, up to but excluding launchd.
    fn ancestry(&mut self, pid: u32) -> Vec<u32> {
        let mut chain = vec![pid];
        let mut cur = pid;
        while chain.len() < MAX_ANCESTRY {
            match self.parent(cur) {
                Some(ppid) if ppid > 1 && !chain.contains(&ppid) => {
                    chain.push(ppid);
                    cur = ppid;
                }
                _ => break,
            }
        }
        chain
    }
}

impl ProcessSource for MacProcessSource {
    fn snapshot(&mut self, also_pgids: &[u32]) -> io::Result<Snapshot> {
        let (socks, sockets_unreadable) = socket::tcp_sockets(self.uid)?;
        let mut listeners: Vec<Listener> = socks
            .iter()
            .filter(|s| s.state == TSI_S_LISTEN)
            .map(|s| Listener {
                pid: s.pid,
                addr: s.local.to_string(),
                port: s.lport,
            })
            .collect();
        listeners.sort_by(|a, b| (a.port, a.pid, &a.addr).cmp(&(b.port, b.pid, &b.addr)));
        listeners.dedup();

        let mut pgids: Vec<u32> = listeners
            .iter()
            .filter_map(|l| sys::shortinfo(l.pid).ok().map(|s| s.pbsi_pgid))
            .chain(also_pgids.iter().copied())
            .collect();
        pgids.sort_unstable();
        pgids.dedup();

        let mut snap = Snapshot {
            listeners,
            ..Snapshot::default()
        };
        let mut cpu = HashMap::new();
        for pgid in pgids {
            match self.read_group(pgid, &mut cpu) {
                Ok(members) => snap.processes.extend(members),
                Err(_) => {
                    snap.unreadable_groups.insert(pgid);
                }
            }
        }
        self.cpu = cpu;
        for p in &mut snap.processes {
            if sockets_unreadable.contains(&p.pid) {
                p.errors.push(crate::discovery::SOCKETS_UNREADABLE.into());
            }
        }
        let live: HashSet<(u32, u64)> = snap
            .processes
            .iter()
            .map(|p| (p.pid, p.start_epoch_ms))
            .collect();
        self.args.retain(|k, _| live.contains(k));

        let ports: HashSet<u16> = snap.listeners.iter().map(|l| l.port).collect();
        let mut ancestry: HashMap<u32, Vec<u32>> = HashMap::new();
        for (server, client_pid) in connections(&socks, &ports) {
            let client_ancestry = match client_pid {
                Some(pid) => ancestry
                    .entry(pid)
                    .or_insert_with(|| self.ancestry(pid))
                    .clone(),
                None => Vec::new(),
            };
            snap.connections.push(Connection {
                port: server.lport,
                remote_addr: server.foreign.to_string(),
                remote_port: server.fport,
                client_pid,
                client_ancestry,
            });
        }
        Ok(snap)
    }

    fn group(&mut self, pgid: u32) -> io::Result<Vec<ProcessInfo>> {
        // A fresh read must not move the CPU baseline of the periodic scan.
        self.read_group(pgid, &mut HashMap::new())
    }

    fn parent(&mut self, pid: u32) -> Option<u32> {
        sys::shortinfo(pid).ok().map(|s| s.pbsi_ppid)
    }
}

/// Server ends of established connections to `ports` (one per 4-tuple, however many
/// processes share the socket), each with the pid holding the reversed 4-tuple: the
/// client end, when it is a same-uid process on this machine.
fn connections<'a>(socks: &'a [Sock], ports: &HashSet<u16>) -> Vec<(&'a Sock, Option<u32>)> {
    let established = || socks.iter().filter(|s| s.state == TSI_S_ESTABLISHED);
    let tuple = |s: &Sock| (s.local, s.lport, s.foreign, s.fport);
    let mut holders: HashMap<(IpAddr, u16, IpAddr, u16), u32> = HashMap::new();
    for s in established() {
        let pid = holders.entry(tuple(s)).or_insert(s.pid);
        *pid = (*pid).min(s.pid);
    }
    let mut seen = HashSet::new();
    established()
        .filter(|s| ports.contains(&s.lport) && seen.insert(tuple(s)))
        .map(|s| {
            (
                s,
                holders
                    .get(&(s.foreign, s.fport, s.local, s.lport))
                    .copied(),
            )
        })
        .collect()
}

/// Initial environment of one process (`KERN_PROCARGS2`); `None` when it cannot be read
/// or the kernel omits it. Used to tell whether a socket peer runs under an agent.
pub fn process_env(pid: u32) -> Option<BTreeMap<String, String>> {
    sys::parse_procargs(&sys::procargs(pid).ok()?)?.env
}

/// A fixture-safe copy of `snapshot` (plan, "Fixtures"): env keeps only keys accepted by
/// `keep_env` plus `PORT`/`HOST`/`NODE_ENV`, and every occurrence of `home` in paths, argv
/// and env values becomes `/Users/x`.
pub fn scrub(
    snapshot: &Snapshot,
    home: &str,
    host: &str,
    keep_env: &dyn Fn(&str) -> bool,
) -> Snapshot {
    let home = home.trim_end_matches('/');
    // The machine name shows up in arguments too (mysqld's `--pid-file=<host>.pid`).
    let short = host.trim_end_matches(".local");
    let fix = |s: &str| {
        let mut s = s.to_owned();
        if !home.is_empty() {
            s = s.replace(home, "/Users/x");
        }
        if short.len() > 2 {
            s = s.replace(short, "host");
        }
        s
    };
    let fix_opt = |s: &Option<String>| s.as_deref().map(fix);
    let mut out = snapshot.clone();
    for p in &mut out.processes {
        p.exe = fix_opt(&p.exe);
        p.parent_exe = fix_opt(&p.parent_exe);
        p.cwd = fix_opt(&p.cwd);
        if let Some(argv) = &mut p.argv {
            *argv = argv.iter().map(|a| fix(a)).collect();
        }
        if let Some(env) = &mut p.env {
            *env = env
                .iter()
                .filter(|(k, _)| keep_env(k) || KEEP_ENV.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), fix(v)))
                .collect::<BTreeMap<_, _>>();
        }
        for e in &mut p.errors {
            *e = fix(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sock(pid: u32, state: i32, local: (IpAddr, u16), foreign: (IpAddr, u16)) -> Sock {
        Sock {
            pid,
            state,
            local: local.0,
            lport: local.1,
            foreign: foreign.0,
            fport: foreign.1,
        }
    }

    #[test]
    fn connections_pair_local_clients() {
        let (lo, any) = (IpAddr::from([127, 0, 0, 1]), IpAddr::from([0, 0, 0, 0]));
        let lo6 = IpAddr::from(std::net::Ipv6Addr::LOCALHOST);
        let remote = IpAddr::from([10, 0, 0, 2]);
        let est = TSI_S_ESTABLISHED;
        let socks = [
            sock(10, TSI_S_LISTEN, (any, 8000), (any, 0)),
            sock(10, est, (lo, 8000), (lo, 50000)),
            sock(11, est, (lo, 8000), (lo, 50000)), // same socket in a forked worker
            sock(20, est, (lo, 50000), (lo, 8000)),
            sock(10, est, (lo, 8000), (lo, 50001)), // client is another uid: unseen
            sock(10, est, (lo, 8000), (remote, 40000)),
            sock(10, est, (lo6, 8000), (lo6, 50003)),
            sock(21, est, (lo6, 50003), (lo6, 8000)),
            sock(12, est, (lo, 9000), (lo, 50002)), // not a listener port
        ];
        let got: Vec<_> = connections(&socks, &HashSet::from([8000]))
            .into_iter()
            .map(|(s, c)| (s.fport, c))
            .collect();
        let want = [
            (50000, Some(20)),
            (50001, None),
            (40000, None),
            (50003, Some(21)),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn scrub_filters_env_and_rewrites_home() {
        let env = [
            ("PORT", "3000"),
            ("CLAUDECODE", "1"),
            ("SECRET", "x"),
            ("CFG", "/Users/r/a"),
        ];
        let snap = Snapshot {
            processes: vec![ProcessInfo {
                cwd: Some("/Users/r/proj".into()),
                exe: Some("/usr/bin/node".into()),
                argv: Some(vec!["node".into(), "--c=/Users/r/x.js".into()]),
                env: Some(
                    env.iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ),
                ..ProcessInfo::default()
            }],
            ..Snapshot::default()
        };
        let out = scrub(&snap, "/Users/r/", "", &|k| k == "CLAUDECODE" || k == "CFG");
        let p = &out.processes[0];
        assert_eq!(p.cwd.as_deref(), Some("/Users/x/proj"));
        assert_eq!(p.exe.as_deref(), Some("/usr/bin/node"));
        assert_eq!(p.argv.as_ref().unwrap()[1], "--c=/Users/x/x.js");
        let env = p.env.as_ref().unwrap();
        let keys: Vec<_> = env.keys().map(String::as_str).collect();
        assert_eq!(keys, ["CFG", "CLAUDECODE", "PORT"]);
        assert_eq!(env["CFG"], "/Users/x/a");
    }
}
