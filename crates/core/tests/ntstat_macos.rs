//! Live S1 probe. Only the fixture Python server receives a signal on cleanup.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lull_core::model::{BackendHealth, ConnectionKind, Snapshot};
use lull_core::platform::macos::{clock::MacClock, ntstat::NtstatBackend};
use lull_core::traits::{ActivityBackend, Clock};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    let mut server = Server(
        Command::new("python3")
            .args(["-u", "-m", "http.server", "0", "--bind", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fixture python3 -m http.server"),
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
    let clock = MacClock::new();
    let snapshot = Snapshot::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let events = backend.drain(&snapshot, &clock.now());
        assert_eq!(backend.health(), BackendHealth::Healthy);
        if events.iter().any(|event| {
            event.port == port
                && event.client_pid == Some(client_pid)
                && event.kind == ConnectionKind::New
        }) {
            eprintln!("ntstat live: New port={port} curl_pid={client_pid}, request={request_ms}ms");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no New event for port={port} curl_pid={client_pid}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
