//! CLI commands that talk to the daemon (contract §11, §13 M1).

use std::process::ExitCode;

use lull_protocol::{PinSource, Server, StoppedServer};
use serde_json::{Value, json};

use crate::client::Client;
use crate::paths::Paths;

pub fn run(cmd: &str, args: Vec<String>) -> ExitCode {
    match command(cmd, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull {cmd}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn command(cmd: &str, args: Vec<String>) -> Result<(), String> {
    let (flags, pos): (Vec<String>, Vec<String>) =
        args.into_iter().partition(|a| a.starts_with("--"));
    let flag = |f: &str| flags.iter().any(|a| a == f);
    let mut c = Client::connect(&Paths::from_env())?;
    match cmd {
        "ls" => {
            let servers = c.call("servers.list", json!({}))?;
            if flag("--json") {
                println!("{}", serde_json::to_string_pretty(&servers).unwrap());
            } else {
                let servers: Vec<Server> = from(servers)?;
                print!("{}", format_ls(&servers, now_ms()));
            }
            if flag("--all") {
                print!("{}", crate::local::unmanaged_listing()?);
            }
        }
        "stop" | "pin" | "unpin" | "touch" => {
            let id = resolve(live_ids(&mut c)?, arg(&pos, 0)?)?;
            let (method, params) = match cmd {
                "stop" => ("servers.stop", json!({"id": id})),
                "pin" => ("servers.pin", json!({"id": id, "pinned": true})),
                "unpin" => ("servers.pin", json!({"id": id, "pinned": false})),
                _ => ("servers.touch", json!({"id": id})),
            };
            c.call(method, params)?;
            println!("{} {id}", past(cmd));
        }
        "restart" | "forget" => {
            let id = resolve(history_ids(&mut c)?, arg(&pos, 0)?)?;
            let result = c.call(&format!("servers.{cmd}"), json!({"id": id}))?;
            match result.get("pid") {
                Some(pid) => println!(
                    "restarting {id} (pid {pid}); log: {}",
                    Paths::from_env()
                        .restart_logs()
                        .join(format!("restart-{id}.log"))
                        .display()
                ),
                None => println!("forgot {id}"),
            }
        }
        "history" => {
            let history = c.call("servers.history", json!({}))?;
            if flag("--json") {
                println!("{}", serde_json::to_string_pretty(&history).unwrap());
            } else {
                print!("{}", format_history(&from::<Vec<StoppedServer>>(history)?));
            }
        }
        "open" => {
            let target = arg(&pos, 0)?;
            let mut params = if target.contains("://") {
                json!({"url": target})
            } else {
                let id = resolve(live_ids(&mut c)?, target)?;
                match pos.get(1) {
                    Some(path) => json!({"id": id, "path": path}),
                    None => json!({"id": id}),
                }
            };
            // Without a flag the daemon decides from this process's environment (§11).
            if flag("--focus") || flag("--no-focus") {
                params["focus"] = json!(flag("--focus"));
            }
            let result = c.call("servers.open", params)?;
            match result["browser"].as_str() {
                Some(b) if result["opened"] == json!(true) => println!("opened in {b}"),
                _ => println!("opened"),
            }
        }
        "config" => match pos.first().map(String::as_str) {
            Some("get") | None => print_config(c.call("config.get", json!({}))?)?,
            Some("set") => {
                let patch = config_patch(&pos[1..])?;
                print_config(c.call("config.set", json!({"patch": patch}))?)?;
            }
            Some(other) => return Err(format!("unknown subcommand {other}; use get|set")),
        },
        _ => unreachable!("dispatched by main"),
    }
    Ok(())
}

fn from<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("unexpected reply from the daemon: {e}"))
}

fn arg(pos: &[String], i: usize) -> Result<&str, String> {
    pos.get(i)
        .map(String::as_str)
        .ok_or_else(|| "missing <id>".into())
}

fn past(cmd: &str) -> &'static str {
    match cmd {
        "stop" => "stopping",
        "pin" => "pinned",
        "unpin" => "unpinned",
        _ => "touched",
    }
}

fn live_ids(c: &mut Client) -> Result<Vec<String>, String> {
    let servers: Vec<Server> = from(c.call("servers.list", json!({}))?)?;
    Ok(servers.into_iter().map(|s| s.id).collect())
}

fn history_ids(c: &mut Client) -> Result<Vec<String>, String> {
    let history: Vec<StoppedServer> = from(c.call("servers.history", json!({}))?)?;
    Ok(history.into_iter().map(|s| s.id).collect())
}

/// Exact id or a unique prefix of one (`lull stop 22249`).
fn resolve(ids: Vec<String>, arg: &str) -> Result<String, String> {
    if ids.iter().any(|id| id == arg) {
        return Ok(arg.to_owned());
    }
    let matches: Vec<_> = ids.into_iter().filter(|id| id.starts_with(arg)).collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("no server matches {arg}")),
        _ => Err(format!("{arg} is ambiguous: {}", matches.join(", "))),
    }
}

/// `key=value` pairs → `ConfigPatch` JSON. Values are TOML (`idle_minutes=90`,
/// `auto_stop=all`); a value that is not valid TOML is taken as a string.
fn config_patch(pairs: &[String]) -> Result<Value, String> {
    let mut patch = serde_json::Map::new();
    for pair in pairs {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got {pair}"))?;
        let value = toml::from_str::<toml::Table>(&format!("v = {v}"))
            .ok()
            .and_then(|t| t.get("v").cloned())
            .map(|v| serde_json::to_value(v).unwrap())
            .unwrap_or_else(|| json!(v));
        patch.insert(k.trim().to_owned(), value);
    }
    Ok(Value::Object(patch))
}

fn print_config(config: Value) -> Result<(), String> {
    let config: lull_protocol::Config = from(config)?;
    print!("{}", toml::to_string(&config).map_err(|e| e.to_string())?);
    Ok(())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Inverse of `lull_core::model::rfc3339` for `YYYY-MM-DDTHH:MM:SSZ`.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let secs = n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?;
    // Days-from-civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from((days * 86_400 + secs) * 1000).ok()
}

fn ago_or_in(target_ms: u64, now_ms: u64) -> String {
    let fmt = |ms: u64| {
        let m = ms / 60_000;
        if m >= 60 {
            format!("{}h{:02}m", m / 60, m % 60)
        } else {
            format!("{m}m")
        }
    };
    if target_ms > now_ms {
        format!("in {}", fmt(target_ms - now_ms))
    } else {
        format!("{} ago", fmt(now_ms - target_ms))
    }
}

fn table(rows: Vec<Vec<String>>) -> String {
    let mut widths = vec![0; rows.first().map_or(0, Vec::len)];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

pub(crate) fn origin(o: &lull_protocol::Origin) -> String {
    match &o.agent {
        Some(agent) => format!(
            "agent/{}",
            serde_json::to_value(agent).unwrap().as_str().unwrap_or("?")
        ),
        None => "you".into(),
    }
}

/// The ORIGIN column: `lull restart` for a server lull restarted (§7), which would
/// otherwise read `you`.
pub(crate) fn server_origin(s: &lull_protocol::Server) -> String {
    match s.restarted_from {
        Some(_) => "lull restart".into(),
        None => origin(&s.origin),
    }
}

pub(crate) fn enum_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap_or("?")
        .to_owned()
}

pub fn format_ls(servers: &[Server], now_ms: u64) -> String {
    if servers.is_empty() {
        return "no dev servers running\n".into();
    }
    let mut rows = vec![
        [
            "ID",
            "PROJECT",
            "FRAMEWORK",
            "URL",
            "ORIGIN",
            "STATE",
            "STOPS",
        ]
        .map(String::from)
        .to_vec(),
    ];
    for s in servers {
        let stops = match (&s.stops_at, s.pinned) {
            (_, true) if s.pin_source == Some(PinSource::Env) => "pinned (env)".to_owned(),
            (_, true) => "pinned".to_owned(),
            (Some(at), _) => parse_rfc3339(at).map_or(at.clone(), |t| ago_or_in(t, now_ms)),
            (None, _) => "never".to_owned(),
        };
        rows.push(vec![
            s.id.clone(),
            s.project.clone(),
            enum_str(&s.framework),
            s.primary_url.clone().unwrap_or_else(|| "-".into()),
            server_origin(s),
            enum_str(&s.state),
            stops,
        ]);
    }
    table(rows)
}

fn format_history(history: &[StoppedServer]) -> String {
    if history.is_empty() {
        return "no recently stopped servers\n".into();
    }
    let mut rows = vec![
        ["ID", "PROJECT", "STOPPED", "REASON", "STATUS", "COMMAND"]
            .map(String::from)
            .to_vec(),
    ];
    let now = now_ms();
    for h in history {
        rows.push(vec![
            h.id.clone(),
            h.project.clone(),
            parse_rfc3339(&h.stopped_at).map_or(h.stopped_at.clone(), |t| ago_or_in(t, now)),
            enum_str(&h.reason),
            enum_str(&h.status),
            h.command.clone(),
        ]);
    }
    table(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_servers() -> Vec<Server> {
        let text = include_str!("../../../protocol/fixtures/servers-list.ndjson");
        let reply: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        serde_json::from_value(reply["result"].clone()).unwrap()
    }

    #[test]
    fn ls_matches_golden() {
        let now = parse_rfc3339("2026-10-11T20:30:00Z").unwrap();
        let got = format_ls(&fixture_servers(), now);
        assert_eq!(got, include_str!("../tests/golden/ls.txt"));
    }

    #[test]
    fn rfc3339_round_trips() {
        let ms = 1_791_530_155_000;
        assert_eq!(parse_rfc3339(&lull_core::model::rfc3339(ms)), Some(ms));
    }

    #[test]
    fn ids_resolve_by_unique_prefix() {
        let ids = vec!["22249-1".to_owned(), "22250-2".to_owned()];
        assert_eq!(resolve(ids.clone(), "22249").unwrap(), "22249-1");
        assert!(resolve(ids.clone(), "2224").is_ok());
        assert!(resolve(ids.clone(), "222").is_err());
        assert!(resolve(ids, "9").is_err());
    }

    #[test]
    fn config_pairs_become_a_patch() {
        let patch = config_patch(&["idle_minutes=90".into(), "auto_stop=all".into()]).unwrap();
        assert_eq!(patch, json!({"idle_minutes": 90, "auto_stop": "all"}));
    }
}
