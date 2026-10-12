//! `lull` binary. One executable dispatched by `argv[0]` / subcommand (contract §3):
//! `lull daemon`, `lull <cli-cmd>`, and the `lull-nm-host` symlink, which native
//! messaging launches without our arguments. `main.rs` is owned by WP1.5; the other
//! owners expose `pub fn run(args) -> ExitCode` from their own module.

use std::io::IsTerminal;
use std::process::ExitCode;

mod browsers;
mod cli;
mod client;
mod daemon;
mod doctor;
mod local;
mod nm_host;
mod paths;
mod ui;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    let argv0 = args.remove(0);
    if argv0.rsplit('/').next() == Some("lull-nm-host") {
        return nm_host::run(args);
    }
    let Some(cmd) = args.first().cloned() else {
        // Bare `lull` at a terminal opens the UI; in a script it stays an error.
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            return ui::run(Vec::new());
        }
        return usage();
    };
    let rest = args.split_off(1);
    match cmd.as_str() {
        "ls" | "stop" | "pin" | "unpin" | "touch" | "open" | "restart" | "history" | "forget"
        | "config" => cli::run(&cmd, rest),
        "ui" => ui::run(rest),
        "daemon" => daemon::run(rest),
        "doctor" => doctor::run(rest),
        "debug" => local::debug(rest),
        "browsers" => browsers::run(rest),
        "nm-host" => nm_host::run(rest),
        "--version" => {
            let v = lull_protocol::PROTOCOL_VERSION;
            println!(
                "lull {} (protocol {}.{})",
                env!("CARGO_PKG_VERSION"),
                v.major,
                v.minor
            );
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: lull <ui|daemon|ls|stop|pin|unpin|touch|open|restart|history|forget|config|doctor|browsers>"
    );
    ExitCode::from(2)
}
