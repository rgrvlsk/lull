//! `lull daemon [--dry-run]` (contract §3, §8): single instance via `flock`, the `0600`
//! socket, and the loop that scans every 5 s and serves clients between scans. All state
//! lives in `lull_core::ipc::Daemon` on this thread; socket threads only move lines.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use std::{io, thread};

use lull_core::ipc::{ConnId, Daemon, Platform, Settings};
use lull_core::model::rfc3339;
use lull_core::platform::macos::{
    clock::MacClock,
    ntstat::NtstatBackend,
    proc::MacProcessSource,
    signal::{KillpgSignaler, LoginShellSpawner},
};
use lull_core::state::FileStateStore;
use lull_core::traits::ConfigStore;

use ratatui::crossterm::style::{Color, Stylize};
use ratatui::crossterm::terminal;

use crate::paths::{Paths, home};
use crate::ui::{LOGO, LOGO_WIDTH};

enum Msg {
    /// Connection id, writer, `LOCAL_PEERPID`, whether the peer runs under an agent.
    Connected(ConnId, Sender<String>, Option<u32>, bool),
    Line(ConnId, String),
    Closed(ConnId),
}

pub fn run(args: Vec<String>) -> ExitCode {
    match serve(args.iter().any(|a| a == "--dry-run")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull daemon: {e}");
            ExitCode::FAILURE
        }
    }
}

fn serve(dry_run: bool) -> io::Result<()> {
    let paths = Paths::from_env();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.dir)?;
    fs::create_dir_all(paths.restart_logs())?;

    let lock = File::create(paths.lock())?;
    // SAFETY: flock on an fd we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::other("another lull daemon is running"));
    }
    let listener = bind(&paths.socket())?;
    let mut log = Logger::new(paths.log_file());
    log.line(&format!(
        "daemon {} started (pid {}{})",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        if dry_run { ", dry run" } else { "" }
    ));

    let ntstat = match NtstatBackend::open() {
        Ok(b) => Some(Box::new(b) as Box<dyn lull_core::ActivityBackend + Send>),
        Err(e) => {
            log.line(&format!("ntstat unavailable, using the scan backend: {e}"));
            None
        }
    };
    // Started by hand in a terminal rather than by launchd: say what is running, echo
    // events, and let Enter open the UI next to it.
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        banner(&paths, ntstat.is_some(), dry_run);
        log.echo = true;
        thread::spawn(open_ui_on_enter);
    }
    let scan_ms = env_u64("LULL_SCAN_MS").unwrap_or(5_000);
    let platform = Platform {
        source: Box::new(MacProcessSource::new()),
        signaler: Box::new(KillpgSignaler),
        spawner: Box::new(LoginShellSpawner),
        store: Box::new(FileStateStore::new(paths.state())),
        config: Box::new(FileConfigStore(paths.config())),
        clock: Box::new(MacClock::new()),
        open_url: Box::new(open_url),
        log: Box::new(move |line: &str| log.line(line)),
        git_root: Box::new(|cwd: &str| lull_core::discovery::find_git_root(cwd)),
    };
    // SAFETY: plain libc getters.
    let (uid, pgid) = unsafe { (libc::getuid(), libc::getpgrp() as u32) };
    let settings = Settings {
        home: home(),
        uid,
        daemon_pid: std::process::id(),
        daemon_pgid: pgid,
        restart_log_dir: paths.restart_logs().to_string_lossy().into_owned(),
        dry_run,
        scan_interval_ms: scan_ms,
    };
    let mut daemon = Daemon::new(platform, settings, ntstat);

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || accept(listener, tx));

    // The daemon exits when its own executable disappears, so launchd restarts the new
    // version after an upgrade (§3).
    // Resolved: launchd starts us through Homebrew's `opt` symlink, which survives an
    // upgrade; the versioned Cellar path behind it is what disappears.
    let exe = std::env::current_exe().and_then(fs::canonicalize)?;
    let mut writers: HashMap<ConnId, Sender<String>> = HashMap::new();
    let mut next_scan = Instant::now();
    loop {
        let wait = next_scan.saturating_duration_since(Instant::now());
        let outs = match rx.recv_timeout(wait) {
            Ok(Msg::Connected(id, writer, peer, agent)) => {
                writers.insert(id, writer);
                daemon.connected(id, peer, agent);
                continue;
            }
            Ok(Msg::Closed(id)) => {
                writers.remove(&id);
                daemon.disconnected(id)
            }
            Ok(Msg::Line(id, line)) => {
                let outs = daemon.handle_line(id, &line);
                if daemon.take_scan_request() {
                    next_scan = Instant::now();
                }
                outs
            }
            Err(RecvTimeoutError::Timeout) => {
                if !exe.exists() {
                    eprintln!("lull daemon: {} is gone, exiting", exe.display());
                    return Ok(());
                }
                let outs = daemon.scan();
                next_scan =
                    Instant::now() + Duration::from_millis(daemon.next_scan_in_ms().max(50));
                outs
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        for out in outs {
            if let Some(w) = writers.get(&out.conn) {
                let _ = w.send(out.line);
            }
        }
    }
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    // We hold the lock, so any socket file left behind is stale.
    let _ = fs::remove_file(path);
    // SAFETY: umask is process-wide; nothing else creates files during startup.
    let old = unsafe { libc::umask(0o177) };
    let listener = UnixListener::bind(path);
    unsafe { libc::umask(old) };
    let listener = listener?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn accept(listener: UnixListener, tx: Sender<Msg>) {
    for (id, stream) in (1..).zip(listener.incoming().flatten()) {
        let peer = peer_pid(&stream);
        let agent = peer.is_some_and(crate::local::is_agent_process);
        let (wtx, wrx) = mpsc::channel::<String>();
        let Ok(mut writer) = stream.try_clone() else {
            continue;
        };
        thread::spawn(move || {
            for line in wrx {
                if writeln!(writer, "{line}").is_err() {
                    break;
                }
            }
        });
        if tx.send(Msg::Connected(id, wtx, peer, agent)).is_err() {
            return;
        }
        let tx = tx.clone();
        thread::spawn(move || {
            // ponytail: no per-line size cap on the daemon socket; every client is the
            // same uid (ADR-0004) and nm-host caps browser messages at 1 MiB. Upgrade
            // path: read through `Read::take` per line if a client ever misbehaves.
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if tx.send(Msg::Line(id, line)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Closed(id));
        });
    }
}

/// `LOCAL_PEERPID` of a connected client (§5 covered browsers, §11 agent CLI).
pub fn peer_pid(stream: &UnixStream) -> Option<u32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: getsockopt writes at most `len` bytes into `pid`.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &mut len,
        )
    };
    (rc == 0 && pid > 0).then_some(pid as u32)
}

fn open_url(url: &str, app: &str, background: bool) -> io::Result<()> {
    let mut cmd = Command::new("/usr/bin/open");
    if background {
        cmd.arg("-g");
    }
    if !app.is_empty() {
        cmd.args(["-a", app]);
    }
    let status = cmd.arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("open(1) exited with {status}")))
    }
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok()?.parse().ok()
}

pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

struct FileConfigStore(PathBuf);

impl ConfigStore for FileConfigStore {
    fn load(&mut self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.0) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn save(&mut self, text: &str) -> io::Result<()> {
        write_atomic(&self.0, text)
    }
}

/// `~/Library/Logs/lull.log`, rotated once to `lull.log.1` past 1 MiB.
struct Logger {
    path: PathBuf,
    /// Also print each line to the terminal the daemon was started from.
    echo: bool,
}

impl Logger {
    fn new(path: PathBuf) -> Logger {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        Logger { path, echo: false }
    }

    fn line(&mut self, line: &str) {
        if fs::metadata(&self.path).is_ok_and(|m| m.len() > 1 << 20) {
            let _ = fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        let stamp = rfc3339(crate::cli::now_ms());
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(f, "{stamp} {line}");
        }
        if self.echo {
            // Lines name processes and commands, which agents control (ADR-0009).
            println!("  {}  {}", clock().dim(), crate::ui::clean(line));
        }
    }
}

/// Local wall-clock time, `HH:MM:SS`.
fn clock() -> String {
    // SAFETY: `time` and `localtime_r` write only to the values we pass.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Brand lavender on indigo, or the nearest 256-colour pair when the terminal does not
/// announce 24-bit colour. A filled plate reads the same on light and dark themes.
fn brand_colours() -> (Color, Color) {
    match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor" | "24bit") => (
            Color::Rgb {
                r: 0xee,
                g: 0xed,
                b: 0xf6,
            },
            Color::Rgb {
                r: 0x27,
                g: 0x27,
                b: 0x44,
            },
        ),
        _ => (Color::AnsiValue(255), Color::AnsiValue(236)),
    }
}

fn banner(paths: &Paths, ntstat: bool, dry_run: bool) {
    // Margin around the cat: a quarter of its height.
    let side = 4;
    println!();
    if terminal::size().is_ok_and(|(w, _)| usize::from(w) >= LOGO_WIDTH + 2 * side + 2) {
        let (ink, paper) = brand_colours();
        for line in [""].iter().chain(&LOGO).chain(&[""]) {
            let line = format!("{:side$}{line:<LOGO_WIDTH$}{:side$}", "", "");
            println!("  {}", line.with(ink).on(paper));
        }
        println!();
    }
    println!(
        "  {} {} · {}",
        "lull".bold(),
        env!("CARGO_PKG_VERSION"),
        "stops the dev servers you forgot".dim()
    );
    let backend = if ntstat { "ntstat" } else { "scan" };
    let dry = if dry_run {
        " · dry run, nothing is stopped"
    } else {
        ""
    };
    println!();
    println!(
        "  {} daemon running · pid {} · {backend}{dry}",
        "●".green(),
        std::process::id()
    );
    let tilde = |p: PathBuf| crate::ui::tilde(&p.display().to_string());
    println!("    {}  {}", "socket".dim(), tilde(paths.socket()));
    println!("    {}     {}", "log".dim(), tilde(paths.log_file()));
    println!();
    match crate::ui::tab::target() {
        Some(place) => println!("  {} open the UI in {place}", "⏎".bold()),
        None => println!(
            "  open the UI from another tab: {}",
            crate::ui::tab::typed().bold()
        ),
    }
    println!("  {} stop the daemon", "ctrl-c".bold());
    println!();
}

/// Enter in the daemon's terminal opens the UI next to it.
fn open_ui_on_enter() {
    for line in io::stdin().lines() {
        if line.is_err() {
            return;
        }
        match crate::ui::tab::open() {
            Ok(place) => println!("  opened the UI in {place}"),
            Err(e) => println!("  {e}"),
        }
    }
}
