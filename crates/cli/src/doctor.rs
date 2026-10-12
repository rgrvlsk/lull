//! `lull doctor` (plan WP1.5): daemon, socket and lock state, activity backend and its
//! health, config validity, aborted or stuck stops, and why servers are not eligible
//! to stop. The daemon's side comes from its internal `status.json`.

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::process::ExitCode;

use serde_json::{Value, json};

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
    let hello = Client::connect(&paths).and_then(|mut c| c.call("servers.list", json!({})));
    match (&mode, &hello) {
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
    if let Err(e) = &hello {
        line("daemon", false, e.clone());
    }

    let status: Value = fs::read_to_string(paths.status())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if status.is_object() {
        line(
            "daemon",
            hello.is_ok(),
            format!(
                "pid {} version {}{}",
                status["pid"],
                status["version"].as_str().unwrap_or("?"),
                if status["dryRun"] == json!(true) {
                    " (dry run: no signals)"
                } else {
                    ""
                }
            ),
        );
        let health = status["activityHealth"].as_str().unwrap_or("?");
        line(
            "activity",
            health == "Healthy",
            format!(
                "{} backend, {health}",
                status["activityBackend"].as_str().unwrap_or("?")
            ),
        );
        for w in status["warnings"].as_array().into_iter().flatten() {
            line("state", false, w.as_str().unwrap_or("").into());
        }
        let stops = status["stops"].as_array().cloned().unwrap_or_default();
        line(
            "stops",
            stops.is_empty(),
            if stops.is_empty() {
                "no aborted or stuck stops".into()
            } else {
                stops
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            },
        );
    }

    match fs::read_to_string(paths.config()) {
        Err(_) => line("config", true, "defaults (no config.toml)".into()),
        Ok(text) => match lull_core::policy::parse_toml(&text)
            .and_then(|c| lull_core::policy::validate(&c))
        {
            Ok(()) => line("config", true, paths.config().display().to_string()),
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

    if let Some(ineligible) = status["ineligible"].as_object().filter(|m| !m.is_empty()) {
        println!("\nnot eligible to stop:");
        for (id, reason) in ineligible {
            println!("  {id}  {}", reason.as_str().unwrap_or("?"));
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
