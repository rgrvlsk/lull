//! `lull doctor` (plan WP1.5): daemon, socket and lock state, activity backend and its
//! health, config validity, aborted or stuck stops, and why servers are not eligible
//! to stop. The daemon's side comes from `daemon.status` (protocol 1.3).

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::process::ExitCode;

use lull_protocol::rpc::{ActivityHealth, DaemonStatus};
use serde_json::json;

use crate::client::Client;
use crate::paths::Paths;

pub fn run(_args: Vec<String>) -> ExitCode {
    let paths = Paths::from_env();
    let mut ok = true;
    let mut line = |label: &str, good: bool, text: String| {
        ok &= good;
        println!("{} {label:<10} {text}", if good { "✓" } else { "✗" });
    };

    let locked = File::open(paths.lock()).is_ok_and(|f| {
        // SAFETY: flock on an fd we own; released when `f` drops.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) != 0 }
    });
    line(
        "lock",
        locked,
        if locked {
            "held by a running daemon".into()
        } else {
            format!("free: no daemon running ({})", paths.lock().display())
        },
    );

    let mode = fs::metadata(paths.socket()).map(|m| m.permissions().mode() & 0o777);
    let status = Client::connect(&paths)
        .and_then(|mut c| c.call("daemon.status", json!({})))
        .and_then(|v| serde_json::from_value::<DaemonStatus>(v).map_err(|e| e.to_string()));
    match (&mode, &status) {
        (Ok(0o600), Ok(_)) => line(
            "socket",
            true,
            format!("{} (0600)", paths.socket().display()),
        ),
        (Ok(m), _) => line("socket", false, format!("mode {m:o}, expected 600")),
        (Err(e), _) => line(
            "socket",
            false,
            format!("{}: {e}", paths.socket().display()),
        ),
    }
    match &status {
        Err(e) => line("daemon", false, e.clone()),
        Ok(status) => {
            line(
                "daemon",
                true,
                format!(
                    "pid {} version {}{}",
                    status.pid,
                    status.version,
                    if status.dry_run {
                        " (dry run: no signals)"
                    } else {
                        ""
                    }
                ),
            );
            let backend = json!(status.activity_backend);
            let backend = backend.as_str().unwrap_or("?");
            line(
                "activity",
                status.activity_health == ActivityHealth::Healthy,
                match &status.activity_error {
                    None => format!("{backend} backend, healthy"),
                    Some(why) => format!("{backend} backend, failed: {why}"),
                },
            );
            for w in &status.warnings {
                line("state", false, w.clone());
            }
            line(
                "stops",
                status.stops.is_empty(),
                if status.stops.is_empty() {
                    "no aborted or stuck stops".into()
                } else {
                    status.stops.join("; ")
                },
            );
        }
    }

    match fs::read_to_string(paths.config()) {
        Err(_) => line("config", true, "defaults (no config.toml)".into()),
        Ok(text) => match lull_core::policy::parse_toml(&text) {
            Ok(_) => line("config", true, paths.config().display().to_string()),
            Err(e) => line(
                "config",
                false,
                format!("invalid, daemon keeps the last good one: {e}"),
            ),
        },
    }

    for (label, good, text) in crate::browsers::doctor_lines() {
        line(&label, good, text);
    }

    if let Some(status) = status.as_ref().ok().filter(|s| !s.ineligible.is_empty()) {
        println!("\nnot eligible to stop:");
        for (id, reason) in &status.ineligible {
            println!("  {id}  {reason}");
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
