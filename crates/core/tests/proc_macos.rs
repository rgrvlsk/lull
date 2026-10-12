#![cfg(target_os = "macos")]
//! WP1.1 acceptance: the macOS process source against a real server it spawns.

use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lull_core::model::Snapshot;
use lull_core::platform::macos::proc::MacProcessSource;
use lull_core::traits::ProcessSource;

/// A static file server on 127.0.0.1 that listens without `python3 -m http.server`'s
/// reverse-DNS lookup between bind and listen, which hangs on CI runners. Argument: port
/// (0 = any); prints `Serving HTTP on 127.0.0.1 port <n>`.
const SERVE: &str = "import socketserver as s, http.server as h, sys; srv = s.TCPServer(('127.0.0.1', int(sys.argv[1])), h.SimpleHTTPRequestHandler); print('Serving HTTP on 127.0.0.1 port', srv.server_address[1], flush=True); srv.serve_forever()";

/// A Python test server (`SERVE`) in its own session; its whole group is killed on drop.
struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Server {
    fn spawn() -> Server {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir =
            std::env::temp_dir().join(format!("lull-proc-test-{}-{port}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cmd = Command::new("python3");
        cmd.args(["-c", SERVE, &port.to_string()])
            .current_dir(&dir)
            .env("CLAUDECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| match libc::setsid() {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let child = cmd.spawn().expect("python3 must be installed");
        Server { child, port, dir }
    }

    fn pgid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: signals only the group of the process this test spawned.
        unsafe { libc::killpg(self.pgid() as libc::pid_t, libc::SIGKILL) };
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn wait_for(source: &mut MacProcessSource, what: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let snap = source.snapshot(&[]).unwrap();
        if what(&snap) {
            return snap;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; last snapshot: {snap:#?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Member details without the socket read: everything `group` reports for the server.
#[test]
fn group_reads_setsid_agent_server() {
    let server = Server::spawn();
    let mut source = MacProcessSource::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(("127.0.0.1", server.port)).is_err() {
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(100));
    }
    let members = source.group(server.pgid()).unwrap();
    let proc = members
        .iter()
        .find(|p| p.argv.iter().flatten().any(|a| a == SERVE))
        .unwrap_or_else(|| panic!("{members:#?}"));
    assert_eq!(proc.pgid, server.pgid());
    let cwd = std::fs::canonicalize(&server.dir).unwrap();
    assert_eq!(proc.cwd.as_deref(), cwd.to_str());
    let env = proc.env.as_ref().expect("env readable");
    assert_eq!(env.get("CLAUDECODE").map(String::as_str), Some("1"));
    assert_eq!(proc.has_tty, Some(false));
    assert!(proc.start_epoch_ms > 0 && proc.rss_bytes > 0, "{proc:#?}");
    assert!(proc.errors.is_empty(), "{:?}", proc.errors);
}

#[test]
fn finds_setsid_agent_server_and_its_local_client() {
    let server = Server::spawn();
    let mut source = MacProcessSource::new();
    let snap = wait_for(&mut source, |s| {
        s.listeners.iter().any(|l| l.port == server.port)
    });

    let listener = snap
        .listeners
        .iter()
        .find(|l| l.port == server.port)
        .unwrap();
    assert_eq!(listener.addr, "127.0.0.1");
    let proc = snap
        .processes
        .iter()
        .find(|p| p.pid == listener.pid)
        .expect("listener process is in the snapshot");
    assert_eq!(proc.pgid, server.pgid(), "{proc:#?}");
    let cwd = std::fs::canonicalize(&server.dir).unwrap();
    assert_eq!(proc.cwd.as_deref(), cwd.to_str());
    let env = proc.env.as_ref().expect("env readable");
    assert_eq!(env.get("CLAUDECODE").map(String::as_str), Some("1"));
    assert_eq!(proc.has_tty, Some(false));
    assert!(proc.argv.as_ref().unwrap().iter().any(|a| a == SERVE));
    assert!(proc.start_epoch_ms > 0 && proc.rss_bytes > 0, "{proc:#?}");
    assert!(proc.errors.is_empty(), "{:?}", proc.errors);

    let mut members = source.group(server.pgid()).unwrap();
    members.retain(|m| m.pid == listener.pid);
    assert_eq!(members.len(), 1);

    let client = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let local_port = client.local_addr().unwrap().port();
    let me = std::process::id();
    let snap = wait_for(&mut source, |s| {
        s.connections.iter().any(|c| c.remote_port == local_port)
    });
    let conn = snap
        .connections
        .iter()
        .find(|c| c.remote_port == local_port)
        .unwrap();
    assert_eq!(conn.port, server.port);
    assert_eq!(conn.remote_addr, "127.0.0.1");
    assert_eq!(conn.client_pid, Some(me));
    assert_eq!(conn.client_ancestry.first(), Some(&me));
    assert!(!conn.client_ancestry.contains(&1));
    assert_eq!(source.parent(me), Some(std::os::unix::process::parent_id()));
}

#[test]
fn unknown_group_is_empty() {
    // pgids are pids; a pid this large is never allocated (PID_MAX is 99999).
    assert_eq!(MacProcessSource::new().group(4_000_000).unwrap(), vec![]);
}

/// Prints one scan's duration: `cargo test --release -p lull-core -- --ignored`.
#[test]
#[ignore]
fn scan_time() {
    let mut source = MacProcessSource::new();
    let t = Instant::now();
    let snap = source.snapshot(&[]).unwrap();
    let cold = t.elapsed();
    let t = Instant::now();
    source.snapshot(&[]).unwrap();
    let warm = t.elapsed();
    println!(
        "scan: cold {cold:?}, warm {warm:?}; {} listeners, {} processes, {} connections",
        snap.listeners.len(),
        snap.processes.len(),
        snap.connections.len()
    );
    assert!(warm < Duration::from_millis(50), "warm scan took {warm:?}");
}
