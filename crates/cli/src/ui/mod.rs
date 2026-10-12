//! `lull ui`: the live terminal UI (contract §3, ADR-0009). A pure socket client: it
//! subscribes to server changes and sends the same requests as the CLI.
//!
//! Three threads: keys, daemon lines, and this one, which owns all state and blocks
//! until either sends something or the next whole second (for countdowns). Idle, it
//! uses no CPU.

mod app;
pub mod tab;
mod view;

pub(crate) use app::{clean, tilde};

use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use lull_protocol::rpc::{HelloResult, Message};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use serde_json::json;

use crate::cli::now_ms;
use crate::client::Client;
use crate::paths::Paths;
use app::{App, Cmd, Req};

/// The logo, sleeping cat and wordmark, two pixel rows per line in half blocks: the
/// daemon banner draws it on a brand plate, the UI as a faint watermark.
pub(crate) const LOGO: [&str; 7] = [
    "          ▄█▄     ▄",
    "    ▄▄███████▄▄ ▄██     ██         ██ ██",
    "  ▄████████████████     ██ ▄▄   ▄▄ ██ ██",
    " ███▄▄▄▀███▀▀██████     ██ ██   ██ ██ ██",
    " ███████▄▀█████▀▀██     ██ ██   ██ ██ ██",
    " ▀███████▄▀▀▀▀▀██▀      ██ ▀█████▀ ██ ██",
    "   ▀▀█████████▀",
];
pub(crate) const LOGO_WIDTH: usize = 40;

/// CPU and RSS changes are not notified (§8), so the list is re-read this often.
const REFRESH_MS: u64 = 5_000;
const RETRY_MS: u64 = 2_000;

pub fn run(args: Vec<String>) -> ExitCode {
    if !args.is_empty() {
        eprintln!("usage: lull ui");
        return ExitCode::from(2);
    }
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        eprintln!("lull ui: needs a terminal; scripts can use `lull ls --json`");
        return ExitCode::from(2);
    }
    // `ratatui::run` restores the terminal on return and on panic.
    match ratatui::run(event_loop) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull ui: {e}");
            ExitCode::FAILURE
        }
    }
}

enum Ev {
    Key(KeyEvent),
    Redraw,
    /// A line from the daemon, tagged with the connection it came on.
    Line(u32, String),
    Closed(u32),
}

struct Conn {
    generation: u32,
    writer: UnixStream,
    next_id: i64,
    pending: HashMap<i64, Req>,
}

impl Conn {
    fn open(tx: &Sender<Ev>, generation: u32) -> Result<(Conn, HelloResult), String> {
        let client = Client::connect(&Paths::from_env())?;
        let hello = serde_json::from_value(client.hello.clone()).map_err(|e| e.to_string())?;
        let (mut reader, writer) = client.into_parts();
        let tx = tx.clone();
        thread::spawn(move || {
            let mut line = String::new();
            while matches!(reader.read_line(&mut line), Ok(n) if n > 0) {
                if tx
                    .send(Ev::Line(generation, std::mem::take(&mut line)))
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Ev::Closed(generation));
        });
        let conn = Conn {
            generation,
            writer,
            next_id: 100,
            pending: HashMap::new(),
        };
        Ok((conn, hello))
    }

    fn send(&mut self, cmd: Cmd) -> io::Result<()> {
        self.next_id += 1;
        self.pending.insert(self.next_id, cmd.req);
        let msg = json!({"jsonrpc": "2.0", "id": self.next_id, "method": cmd.method, "params": cmd.params});
        writeln!(self.writer, "{msg}")
    }
}

fn event_loop(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    let keys = tx.clone();
    thread::spawn(move || {
        while let Ok(e) = event::read() {
            let ev = match e {
                Event::Key(k) if k.kind == KeyEventKind::Press => Ev::Key(k),
                Event::Resize(..) | Event::FocusGained => Ev::Redraw,
                _ => continue,
            };
            if keys.send(ev).is_err() {
                return;
            }
        }
    });

    let mut app = App::new(now_ms());
    let mut conn: Option<Conn> = None;
    let mut generation = 0;
    let (mut next_connect, mut next_refresh) = (0, 0);
    loop {
        let now = now_ms();
        app.now = now;
        if conn.is_none() && now >= next_connect {
            next_connect = now + RETRY_MS;
            generation += 1;
            if let Ok((c, hello)) = Conn::open(&tx, generation) {
                conn = Some(c);
                app.connected(hello);
                next_refresh = now + REFRESH_MS;
            }
        }
        if conn.is_some() && now >= next_refresh {
            next_refresh = now + REFRESH_MS;
            app.out.push(Cmd::new(Req::List, "servers.list", json!({})));
        }
        for cmd in std::mem::take(&mut app.out) {
            if let Some(c) = &mut conn
                && c.send(cmd).is_err()
            {
                let _ = c.writer.shutdown(std::net::Shutdown::Both);
                conn = None;
                app.disconnected();
            }
        }

        terminal.draw(|f| view::draw(f, &mut app))?;
        if app.quit {
            return Ok(());
        }

        // Wake on the next whole second so every countdown ticks together.
        let wait = Duration::from_millis(1000 - now_ms() % 1000);
        match rx.recv_timeout(wait) {
            Ok(Ev::Key(k)) => app.on_key(k),
            Ok(Ev::Line(g, line)) => {
                if let Some(c) = conn.as_mut().filter(|c| c.generation == g) {
                    receive(&mut app, c, &line);
                }
            }
            Ok(Ev::Closed(g)) => {
                if conn.as_ref().is_some_and(|c| c.generation == g) {
                    conn = None;
                    app.disconnected();
                }
            }
            Ok(Ev::Redraw) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn receive(app: &mut App, conn: &mut Conn, line: &str) {
    match serde_json::from_str::<Message>(line) {
        Ok(Message::Response(r)) => {
            let lull_protocol::rpc::RequestId::Number(id) = r.id else {
                return;
            };
            if let Some(req) = conn.pending.remove(&id) {
                let result = match r.error {
                    Some(e) => Err(e.message),
                    None => Ok(r.result.unwrap_or_default()),
                };
                app.on_result(req, result);
            }
        }
        Ok(Message::Notification(n)) => {
            if let Ok(note) = n.typed() {
                app.on_note(note);
            }
        }
        _ => {}
    }
}
