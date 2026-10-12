//! Commands that read this machine directly instead of asking the daemon: `lull ls
//! --all`'s unmanaged listeners (not part of the protocol) and `lull debug snapshot`.

use std::process::ExitCode;

use lull_core::discovery::{self, Context};
use lull_core::model::{Discovery, Snapshot};
use lull_core::platform::macos::proc::{MacProcessSource, process_env, scrub};
use lull_core::traits::ProcessSource;

use crate::paths::home;

/// Whether `pid` runs with an agent marker in its environment (§11 `lull open`).
pub fn is_agent_process(pid: u32) -> bool {
    process_env(pid).is_some_and(|env| discovery::marker(&env, &[]).is_some())
}

fn scan() -> Result<(Snapshot, Discovery), String> {
    let snapshot = MacProcessSource::new()
        .snapshot(&[])
        .map_err(|e| format!("cannot read processes: {e}"))?;
    let home = home();
    let ctx = Context {
        home: &home,
        // SAFETY: plain getter.
        uid: unsafe { libc::getuid() },
        daemon_pid: std::process::id(),
        extra_markers: &[],
        git_root: &|cwd: &str| discovery::find_git_root(cwd),
    };
    let found = discovery::discover(&snapshot, &ctx, &[], &[]);
    Ok((snapshot, found))
}

pub fn unmanaged_listing() -> Result<String, String> {
    let (_, found) = scan()?;
    let mut out = String::from("\nunmanaged listeners (never stopped):\n");
    if found.unmanaged.is_empty() {
        out.push_str("  none\n");
    }
    for u in &found.unmanaged {
        let ports: Vec<String> = u.ports.iter().map(u16::to_string).collect();
        out.push_str(&format!(
            "  {:<7} {:<16} {}  ({})\n",
            u.pgid,
            ports.join(","),
            u.command,
            u.cwd.as_deref().unwrap_or("?")
        ));
    }
    Ok(out)
}

/// `lull debug snapshot`: the scrubbed fixture format of the plan's ground rules.
pub fn debug(args: Vec<String>) -> ExitCode {
    if args.first().map(String::as_str) != Some("snapshot") {
        eprintln!("usage: lull debug snapshot > file.json");
        return ExitCode::from(2);
    }
    match scan() {
        Ok((snapshot, _)) => {
            let clean = scrub(&snapshot, &home(), &discovery::is_marker_key);
            println!("{}", serde_json::to_string_pretty(&clean).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lull debug: {e}");
            ExitCode::FAILURE
        }
    }
}
