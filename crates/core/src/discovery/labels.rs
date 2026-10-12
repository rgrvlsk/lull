//! Labels of a server (§4.5, §4 `primaryUrl`, §7.6 launch command): argv heuristics only,
//! never a network probe.

use std::collections::BTreeMap;

use lull_protocol::server::Framework;

use super::basename;
use crate::model::ProcessInfo;

/// argv as the process shows it. A process that set its title (npm sets `npm run dev`)
/// reads back as `["npm run dev", "", "", …]`: that is split on whitespace.
// ponytail: a retitled argv loses its quoting; fine for `npm run dev`-style titles. Upgrade
// path: resolve the script and re-read the real argv if a title with spaces in one argument
// ever matters.
pub(super) fn argv(p: &ProcessInfo) -> Vec<String> {
    let Some(argv) = p.argv.as_deref() else {
        return Vec::new();
    };
    let used = argv.len() - argv.iter().rev().take_while(|a| a.is_empty()).count();
    match &argv[..used] {
        [title] if used < argv.len() && title.contains(' ') => {
            title.split_whitespace().map(str::to_owned).collect()
        }
        args => args.to_vec(),
    }
}

/// POSIX shell quoting, word by word.
pub(super) fn quote(args: &[String]) -> String {
    let word = |a: &String| {
        let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
        if !a.is_empty() && a.chars().all(safe) {
            a.clone()
        } else {
            format!("'{}'", a.replace('\'', r"'\''"))
        }
    };
    args.iter().map(word).collect::<Vec<_>>().join(" ")
}

/// A process's command line for display: quoted argv, else its executable.
pub(super) fn command_line(p: &ProcessInfo) -> String {
    let argv = argv(p);
    if argv.is_empty() {
        p.exe.clone().unwrap_or_default()
    } else {
        quote(&argv)
    }
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "fish"];

/// A shell running a command string (`-c`, `-lc`, `-c -l`, …).
fn is_shell_c(p: &ProcessInfo) -> bool {
    let argv = argv(p);
    let name = p.exe.as_deref().or(argv.first().map(String::as_str));
    let is_shell = name.is_some_and(|n| SHELLS.contains(&basename(n).trim_start_matches('-')));
    let mut flags = argv
        .iter()
        .skip(1)
        .take_while(|a| a.starts_with('-') && *a != "--");
    is_shell && flags.any(|f| !f.starts_with("--") && f.contains('c'))
}

/// §7.6: walk up from `start` through parents still in the group; take the topmost, or its
/// child on that path when the topmost is a `-c` shell. Empty when none can be found.
pub(super) fn launch_argv(members: &[&ProcessInfo], start: &ProcessInfo) -> Vec<String> {
    let by_pid: BTreeMap<u32, &ProcessInfo> = members.iter().map(|p| (p.pid, *p)).collect();
    let mut path = vec![start];
    while let Some(parent) = by_pid.get(&path[path.len() - 1].ppid) {
        if path.len() > members.len() || path.iter().any(|p| p.pid == parent.pid) {
            break;
        }
        path.push(parent);
    }
    let launch = match path.as_slice() {
        [.., child, top] if is_shell_c(top) => child,
        [top] if is_shell_c(top) => return Vec::new(),
        [.., top] => top,
        [] => unreachable!("path starts with `start`"),
    };
    argv(launch)
}

/// §4.5 framework heuristics over every member's argv, most specific first.
pub(super) fn framework(members: &[&ProcessInfo]) -> Framework {
    let argvs: Vec<Vec<String>> = members.iter().map(|p| argv(p)).collect();
    let name = |a: &str| -> String {
        let n = basename(a);
        n.strip_suffix(".js")
            .or_else(|| n.strip_suffix(".mjs"))
            .unwrap_or(n)
            .to_owned()
    };
    let any = |f: &dyn Fn(&[String]) -> bool| argvs.iter().any(|a| f(a));
    let word = |w: &str| any(&|a| a.iter().any(|x| name(x) == w));
    let has = |a: &[String], w: &str| a.iter().any(|x| x == w);
    if word("astro") {
        Framework::Astro
    } else if word("nuxi") || word("nuxt") {
        Framework::Nuxt
    } else if word("next") || any(&|a| a.iter().any(|x| x.starts_with("next-server"))) {
        Framework::Next
    } else if word("remix") || word("react-router") {
        Framework::Remix
    } else if word("vite") {
        Framework::Vite
    } else if word("wrangler") {
        Framework::Wrangler
    } else if any(&|a| a.iter().any(|x| name(x) == "artisan") && has(a, "serve")) {
        Framework::Artisan
    } else if any(&|a| a.first().is_some_and(|x| name(x).starts_with("php")) && has(a, "-S")) {
        Framework::PhpBuiltin
    } else if any(&|a| a.iter().any(|x| name(x) == "rails") && (has(a, "s") || has(a, "server"))) {
        Framework::Rails
    } else if any(&|a| a.iter().any(|x| name(x) == "manage.py") && has(a, "runserver")) {
        Framework::Django
    } else if any(&|a| a.windows(2).any(|w| w[0] == "-m" && w[1] == "http.server")) {
        Framework::PythonHttp
    } else {
        Framework::Generic
    }
}

/// §4 `primaryUrl`: a listening port named in a member's argv or `PORT` env (oldest member
/// first), else the lowest port outside the inspector range 9229–9239, else the lowest.
pub(super) fn primary_url(members: &[&ProcessInfo], ports: &[u16]) -> Option<String> {
    let mut oldest_first = members.to_vec();
    oldest_first.sort_by_key(|p| (p.start_epoch_ms, p.pid));
    let named = oldest_first.iter().find_map(|p| {
        let env_port = p.env.as_ref().and_then(|e| e.get("PORT")).cloned();
        argv(p)
            .iter()
            .filter_map(|a| port_in(a))
            .chain(env_port.and_then(|v| v.parse().ok()))
            .find(|port| ports.contains(port))
    });
    let not_inspector = ports.iter().copied().find(|p| !(9229..=9239).contains(p));
    let port = named.or(not_inspector).or(ports.first().copied())?;
    Some(format!("http://127.0.0.1:{port}/"))
}

/// A port written as an argument: `5173`, `--port=5173`, `-p=5173` or `host:5173`.
/// Arguments with any other `=` are ignored: workerd's `--external-addr=loopback=…:62430`
/// names an internal port, not the server's.
// ponytail: flag-shaped heuristic, no per-tool parsing; ceiling is an exotic flag naming
// another listening port first. Upgrade path: per-framework port flags.
fn port_in(arg: &str) -> Option<u16> {
    let arg = arg
        .strip_prefix("--port=")
        .or_else(|| arg.strip_prefix("-p="))
        .unwrap_or(arg);
    if arg.contains('=') {
        return None;
    }
    let digits = arg.rsplit(':').next()?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}
