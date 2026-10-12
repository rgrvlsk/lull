//! Real macOS daemon tests. Global auto-stop is disabled; only this test's project
//! can be stopped. Every child starts in a new session, and cleanup signals only
//! groups belonging to those children or their descendants.
#![cfg(target_os = "macos")]

use std::fs::{self, File};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_lull");
const POLL: Duration = Duration::from_millis(100);
const TIMEOUT: Duration = Duration::from_secs(15);
const SURVIVAL: Duration = Duration::from_secs(7);
const IDLE_MINUTES: u32 = 3;

/// Rebuild the environment so the test runner's agent markers and LULL_KEEP never
/// leak into a user server. Use a login shell without personal startup files.
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    for key in ["PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe. A successful spawn has its own pgid.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    cmd
}

/// pid, parent pid, process group. `script` can create another session for Python;
/// restart children also have their own sessions. Trace ownership by ancestry.
fn processes() -> Vec<[i32; 3]> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .output()
        .expect("read process ancestry for cleanup");
    assert!(output.status.success(), "ps failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields: Vec<i32> = line
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            fields.try_into().ok()
        })
        .collect()
}

fn descendant_groups(roots: &[i32]) -> Vec<i32> {
    let rows = processes();
    let mut owned = roots.to_vec();
    loop {
        let before = owned.len();
        for [pid, parent, _] in &rows {
            if owned.contains(parent) && !owned.contains(pid) {
                owned.push(*pid);
            }
        }
        if owned.len() == before {
            break;
        }
    }
    rows.iter()
        .filter(|[pid, _, _]| owned.contains(pid))
        .map(|[_, _, group]| *group)
        .collect()
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    dir: PathBuf,
    children: Vec<Child>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Keep the Unix socket path below macOS's sockaddr_un limit. /private/tmp
        // is a contract §4 project location, and each directory is created fresh.
        let root = loop {
            let candidate = PathBuf::from(format!(
                "/private/tmp/lull-e2e-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate.canonicalize().unwrap(),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create isolated directory: {e}"),
            }
        };
        let mut fixture = Self {
            project: root.join("project"),
            dir: root.join("daemon"),
            root,
            children: Vec::new(),
        };
        fs::create_dir(&fixture.project).unwrap();
        fs::create_dir(&fixture.dir).unwrap();
        fixture.project = fixture.project.canonicalize().unwrap();
        // Mandatory safety boundary: never use auto_stop=agent globally. The only
        // eligible cwd is this fresh directory, disjoint from every other test.
        let config = format!(
            "auto_stop = \"none\"\nwake_grace_minutes = 0\nwarn_before_minutes = 0\n\
             [[project]]\npath = {}\nauto_stop = \"agent\"\nidle_minutes = {IDLE_MINUTES}\n",
            serde_json::to_string(fixture.project.to_str().unwrap()).unwrap()
        );
        fs::write(fixture.dir.join("config.toml"), config).unwrap();
        let mut daemon = fixture.cli_command();
        daemon
            .arg("daemon")
            .env("LULL_MINUTE_MS", "1000")
            .env("LULL_SCAN_MS", "250")
            .stdout(File::create(fixture.dir.join("daemon.stdout")).unwrap())
            .stderr(File::create(fixture.dir.join("daemon.stderr")).unwrap());
        fixture.spawn(daemon);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (status, output) = fixture.cli(&["ls", "--json"]);
            if status.success() {
                assert!(serde_json::from_str::<Value>(&output).unwrap().is_array());
                break;
            }
            fixture.check_deadline(deadline, "daemon startup");
            thread::sleep(POLL);
        }
        fixture
    }

    fn cli_command(&self) -> Command {
        let mut cmd = command(BIN);
        cmd.env("LULL_DIR", &self.dir).current_dir(&self.project);
        cmd
    }

    fn spawn(&mut self, mut cmd: Command) -> usize {
        let child = cmd.spawn().expect("spawn test process in its own session");
        self.children.push(child);
        self.children.len() - 1
    }

    fn cli(&mut self, args: &[&str]) -> (ExitStatus, String) {
        let output_path = self.dir.join("cli.stdout");
        let error_path = self.dir.join("cli.stderr");
        let mut cmd = self.cli_command();
        cmd.args(args)
            .stdout(File::create(&output_path).unwrap())
            .stderr(File::create(&error_path).unwrap());
        let child = self.spawn(cmd);
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = self.children[child].try_wait().unwrap() {
                break status;
            }
            self.check_deadline(deadline, &format!("CLI {args:?}"));
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = fs::read_to_string(output_path).unwrap();
        if !status.success() {
            eprintln!("CLI {args:?}: {}", fs::read_to_string(error_path).unwrap());
        }
        (status, stdout)
    }

    fn json(&mut self, verb: &str) -> Vec<Value> {
        let (status, output) = self.cli(&[verb, "--json"]);
        assert!(status.success(), "lull {verb} failed");
        serde_json::from_str(&output).expect("wire JSON array")
    }

    fn check_deadline(&self, deadline: Instant, description: &str) {
        assert!(
            Instant::now() < deadline,
            "timed out: {description}; daemon stderr: {}; log: {}",
            fs::read_to_string(self.dir.join("daemon.stderr")).unwrap_or_default(),
            fs::read_to_string(self.dir.join("logs/lull.log")).unwrap_or_default()
        );
    }

    fn start_server(&mut self, agent: bool, tty: bool, keep: bool) -> u16 {
        static NEXT_PORT: AtomicU64 = AtomicU64::new(0);
        // Keep server ports out of macOS's standard ephemeral range. The daemon
        // observes the whole machine; unrelated client ports must not confound an
        // idle assertion. The atomic counter also separates parallel test servers.
        let reservation = (0..20_000)
            .find_map(|_| {
                let offset =
                    u64::from(std::process::id()) + NEXT_PORT.fetch_add(1, Ordering::Relaxed);
                let port = 10_000 + (offset % 20_000) as u16;
                TcpListener::bind(("127.0.0.1", port)).ok()
            })
            .expect("a free test server port below the ephemeral range");
        let port = reservation.local_addr().unwrap().port();
        let mut cmd = if tty {
            let mut cmd = command("/usr/bin/script");
            cmd.args(["-q", "/dev/null", "python3"]);
            cmd
        } else {
            command("python3")
        };
        cmd.args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(&self.project)
        .stdout(File::create(self.dir.join(format!("server-{port}.log"))).unwrap());
        if agent {
            cmd.env("CLAUDECODE", "1");
        }
        if keep {
            cmd.env("LULL_KEEP", "1");
        }
        drop(reservation);
        self.spawn(cmd);
        port
    }

    fn server(&mut self, port: u16) -> Option<Value> {
        let found = self.json("ls").into_iter().find(|s| {
            s["ports"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p.as_u64() == Some(u64::from(port)))
        })?;
        assert_eq!(found["cwd"].as_str(), self.project.to_str());
        let group = found["pgid"].as_i64().unwrap() as i32;
        let roots = self.live_roots();
        assert!(
            descendant_groups(&roots).contains(&group),
            "port {port} must belong to a process spawned by this fixture"
        );
        Some(found)
    }

    fn wait_server(&mut self, port: u16, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(server) = self.server(port) {
                return server;
            }
            self.check_deadline(deadline, &format!("discover port {port}"));
            thread::sleep(POLL);
        }
    }

    fn stays(&mut self, port: u16, id: &Value) -> Value {
        let deadline = Instant::now() + SURVIVAL;
        loop {
            let server = self
                .server(port)
                .expect("server must survive past idle time");
            assert_eq!(&server["id"], id);
            assert_ne!(server["state"], "stopping");
            if Instant::now() >= deadline {
                return server;
            }
            thread::sleep(POLL);
        }
    }

    fn wait_stopped(&mut self, server: &Value) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        let group = server["pgid"].as_i64().unwrap() as i32;
        loop {
            // Reap direct Python children, otherwise their zombie keeps the group
            // present and correctly delays the daemon's history record.
            for child in &mut self.children {
                child.try_wait().unwrap();
            }
            let current = self
                .json("ls")
                .into_iter()
                .find(|s| s["id"] == server["id"]);
            let live = current.is_some();
            let record = self
                .json("history")
                .into_iter()
                .find(|s| s["id"] == server["id"]);
            // SAFETY: signal 0 only probes the group we previously proved we own.
            let gone = unsafe { libc::killpg(group, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            let diagnostic = format!(
                "idle stop, group exit and history record: original={server}, current={current:?}, group_gone={gone}, history={record:?}"
            );
            if !live && gone {
                if let Some(record) = record {
                    assert_eq!(record["reason"], "idle");
                    return record;
                }
            }
            self.check_deadline(deadline, &diagnostic);
            thread::sleep(POLL);
        }
    }

    fn backend(&mut self) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            // doctor may fail for an uninstalled browser integration; inspect the
            // activity line rather than requiring all doctor checks to pass.
            let (_, output) = self.cli(&["doctor"]);
            for backend in ["ntstat", "scan"] {
                if output.lines().any(|line| {
                    line.contains("activity") && line.contains(&format!("{backend} backend"))
                }) {
                    eprintln!("activityBackend={backend}");
                    return backend.into();
                }
            }
            self.check_deadline(deadline, "doctor activity backend");
            thread::sleep(POLL);
        }
    }
}

impl Fixture {
    fn live_roots(&mut self) -> Vec<i32> {
        self.children
            .iter_mut()
            .filter_map(|child| match child.try_wait() {
                Ok(Some(_)) => None,
                _ => Some(child.id() as i32),
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let roots = self.live_roots();
        let mut groups = descendant_groups(&roots);
        groups.extend(roots);
        groups.sort_unstable();
        groups.dedup();
        // SAFETY: only isolated child sessions and their descendants are selected.
        let own_group = unsafe { libc::getpgrp() };
        for group in groups {
            if group > 1 && group != own_group {
                unsafe { libc::killpg(group, libc::SIGKILL) };
            }
        }
        for child in &mut self.children {
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn idle_agent_is_stopped_and_recorded() {
    let mut fixture = Fixture::new();
    let port = fixture.start_server(true, false, false);
    let server = fixture.wait_server(port, TIMEOUT);
    assert_eq!(server["origin"]["kind"], "agent");
    assert_eq!(server["autoStop"], true);
    assert_eq!(server["idleMinutes"], IDLE_MINUTES);
    fixture.wait_stopped(&server);
}

#[test]
fn user_server_survives_agent_policy() {
    let mut fixture = Fixture::new();
    let port = fixture.start_server(false, false, false);
    let server = fixture.wait_server(port, TIMEOUT);
    assert_eq!(server["origin"]["kind"], "user");
    assert_eq!(server["autoStop"], false);
    fixture.stays(port, &server["id"]);
}

#[test]
fn tty_overrides_agent_marker() {
    let mut fixture = Fixture::new();
    let port = fixture.start_server(true, true, false);
    let server = fixture.wait_server(port, TIMEOUT);
    assert_eq!(server["origin"]["kind"], "user");
    assert_eq!(server["autoStop"], false);
    fixture.stays(port, &server["id"]);
}

#[test]
fn environment_and_cli_pins_hold() {
    let mut fixture = Fixture::new();
    let env_port = fixture.start_server(true, false, true);
    let cli_port = fixture.start_server(true, false, false);
    let env_server = fixture.wait_server(env_port, TIMEOUT);
    let cli_server = fixture.wait_server(cli_port, TIMEOUT);
    assert_eq!(env_server["origin"]["kind"], "agent");
    assert_eq!(env_server["pinned"], true);
    assert_eq!(cli_server["origin"]["kind"], "agent");
    assert_eq!(cli_server["pinned"], false);
    // server() checked both cwd and process ancestry before this mutation.
    assert!(
        fixture
            .cli(&["pin", cli_server["id"].as_str().unwrap()])
            .0
            .success()
    );
    let deadline = Instant::now() + SURVIVAL;
    loop {
        for (port, original) in [(env_port, &env_server), (cli_port, &cli_server)] {
            let server = fixture.server(port).expect("pinned server stays alive");
            assert_eq!(server["id"], original["id"]);
            assert_eq!(server["pinned"], true);
            assert_eq!(server["stopsAt"], Value::Null);
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(POLL);
    }
}

#[test]
fn curls_refresh_traffic_then_idle_stop() {
    let mut fixture = Fixture::new();
    if fixture.backend() == "scan" {
        eprintln!("SKIP curls: scan backend can miss short requests between scans");
        return;
    }
    let port = fixture.start_server(true, false, false);
    let server = fixture.wait_server(port, TIMEOUT);
    assert_eq!(server["origin"]["kind"], "agent");
    let deadline = Instant::now() + SURVIVAL;
    let mut next_curl = Instant::now();
    let mut traffic_seen = false;
    loop {
        if Instant::now() >= next_curl {
            let mut curl = command("/usr/bin/curl");
            curl.args([
                "-s",
                "--max-time",
                "2",
                &format!("http://127.0.0.1:{port}/"),
            ]);
            let child = fixture.spawn(curl);
            assert!(
                fixture.children[child].wait().unwrap().success(),
                "curl failed"
            );
            next_curl = Instant::now() + Duration::from_millis(700);
        }
        let current = fixture.server(port).expect("curls keep server alive");
        assert_eq!(current["id"], server["id"]);
        traffic_seen |= current["lastActivitySource"] == "traffic";
        if Instant::now() >= deadline {
            assert!(
                traffic_seen,
                "curl connections must produce traffic activity"
            );
            assert_eq!(current["lastActivitySource"], "traffic");
            break;
        }
        thread::sleep(POLL);
    }
    fixture.wait_stopped(&server);
}

#[test]
fn open_connection_holds_server_until_closed() {
    let mut fixture = Fixture::new();
    let port = fixture.start_server(true, false, false);
    let server = fixture.wait_server(port, TIMEOUT);
    assert_eq!(server["origin"]["kind"], "agent");
    let connection = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let current = fixture.stays(port, &server["id"]);
    assert_eq!(current["lastActivitySource"], "traffic");
    drop(connection);
    fixture.wait_stopped(&server);
}

#[test]
fn restart_returns_as_user_and_clears_history() {
    let mut fixture = Fixture::new();
    let port = fixture.start_server(true, false, false);
    let original = fixture.wait_server(port, TIMEOUT);
    assert_eq!(original["origin"]["kind"], "agent");
    let record = fixture.wait_stopped(&original);
    assert_eq!(record["cwd"].as_str(), fixture.project.to_str());
    assert_eq!(record["argv"][1], "-m");
    assert_eq!(record["argv"][2], "http.server");
    // The id is the history record of our previously verified Python group.
    assert!(
        fixture
            .cli(&["restart", original["id"].as_str().unwrap()])
            .0
            .success()
    );
    let restarted = fixture.wait_server(port, Duration::from_secs(60));
    assert_ne!(restarted["id"], original["id"]);
    assert_eq!(restarted["cwd"], original["cwd"]);
    assert_eq!(restarted["origin"]["kind"], "user");
    assert_eq!(restarted["autoStop"], false);
    TcpStream::connect(("127.0.0.1", port)).expect("restarted listener accepts connections");
    let deadline = Instant::now() + TIMEOUT;
    while fixture
        .json("history")
        .iter()
        .any(|s| s["id"] == original["id"])
    {
        fixture.check_deadline(deadline, "remove restarted history record");
        thread::sleep(POLL);
    }
}
