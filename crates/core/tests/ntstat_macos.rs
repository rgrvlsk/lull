//! Live S1 probe. Only the fixture Python server receives a signal on cleanup.
#![cfg(target_os = "macos")]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lull_core::model::{BackendHealth, ConnectionKind, Snapshot, TrafficEvent};
use lull_core::platform::macos::{clock::MacClock, ntstat::NtstatBackend};
use lull_core::traits::{ActivityBackend, Clock};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A static file server on 127.0.0.1 that listens without `python3 -m http.server`'s
/// reverse-DNS lookup between bind and listen, which hangs on CI runners. Argument: port
/// (0 = any); prints `Serving HTTP on 127.0.0.1 port <n>`.
const SERVE: &str = "import socketserver as s, http.server as h, sys; srv = s.TCPServer(('127.0.0.1', int(sys.argv[1])), h.SimpleHTTPRequestHandler); print('Serving HTTP on 127.0.0.1 port', srv.server_address[1], flush=True); srv.serve_forever()";
/// Starts the fixture server on an OS-assigned loopback port.
fn serve() -> (Server, u16) {
    let mut server = Server(
        Command::new("python3")
            .args(["-u", "-c", SERVE, "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the python3 fixture server"),
    );
    let stdout = server.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let output_reader = thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("server startup timeout")
        .unwrap();
    output_reader.join().unwrap();
    let port: u16 = line
        .split(" port ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .expect("Python startup line contains port")
        .parse()
        .unwrap();
    (server, port)
}

/// Drains until an event matches, failing after three seconds.
fn wait_for(
    backend: &mut NtstatBackend,
    what: &str,
    mut matches: impl FnMut(&TrafficEvent) -> bool,
) -> TrafficEvent {
    let clock = MacClock::new();
    let snapshot = Snapshot::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let events = backend.drain(&snapshot, &clock.now());
        assert_eq!(backend.health(), BackendHealth::Healthy);
        if let Some(event) = events.into_iter().find(|event| matches(event)) {
            return event;
        }
        assert!(Instant::now() < deadline, "no {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn bytes_on_a_watched_connection_are_reported_as_they_move() {
    let mut backend = match NtstatBackend::open() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("SKIP ntstat live test: open failed: {error}");
            return;
        }
    };
    let (server, port) = serve();
    backend.watch(&BTreeMap::from([(port, BTreeSet::from([server.0.id()]))]));
    let me = Some(std::process::id());
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    wait_for(&mut backend, "New for this test's connection", |event| {
        event.port == port && event.client_pid == me && event.kind == ConnectionKind::New
    });
    // An unfinished request: the server waits for the rest and keeps the connection open.
    stream.write_all(b"GET / HTTP/1.0\r\n").unwrap();
    let first = wait_for(&mut backend, "Bytes for the request line", |event| {
        event.port == port
            && event.client_pid == me
            && matches!(event.kind, ConnectionKind::Bytes(_))
    });
    assert_eq!(first.kind, ConnectionKind::Bytes(16));
    stream.write_all(b"X-Lull: 1\r\n").unwrap();
    let second = wait_for(&mut backend, "Bytes for the header", |event| {
        event.port == port
            && event.client_pid == me
            && matches!(event.kind, ConnectionKind::Bytes(_))
    });
    assert_eq!(second.kind, ConnectionKind::Bytes(11));
    eprintln!("ntstat live: Bytes(16) then Bytes(11) on port={port}");
}

#[test]
fn short_curl_reports_the_server_port_and_client_pid() {
    let mut backend = match NtstatBackend::open() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("SKIP ntstat live test: open failed: {error}");
            return;
        }
    };
    let (_server, port) = serve();
    let started = Instant::now();
    let mut curl = Command::new("/usr/bin/curl")
        .args([
            "--noproxy",
            "*",
            "--max-time",
            "5",
            "--silent",
            "--show-error",
            "--output",
            "/dev/null",
        ])
        .arg(format!("http://127.0.0.1:{port}/"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn curl");
    let client_pid = curl.id();
    assert!(curl.wait().unwrap().success());
    let request_ms = started.elapsed().as_millis();
    assert!(
        request_ms < 5_000,
        "fixture request must be shorter than a scan interval"
    );
    wait_for(
        &mut backend,
        &format!("New for port={port} curl_pid={client_pid}"),
        |event| {
            event.port == port
                && event.client_pid == Some(client_pid)
                && event.kind == ConnectionKind::New
        },
    );
    eprintln!("ntstat live: New port={port} curl_pid={client_pid}, request={request_ms}ms");
}
