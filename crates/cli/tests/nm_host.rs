//! Native host contracts exercised through the installed binary entry points.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::symlink;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

// Use the same source of caller identifiers as the binary without copying ids.
#[allow(dead_code)]
#[path = "../src/browsers/mod.rs"]
mod browsers;

const MAX_MESSAGE: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

struct Host {
    dir: PathBuf,
    child: Option<Child>,
    input: Option<ChildStdin>,
}

impl Host {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = loop {
            // A short path also fits macOS's Unix socket address limit.
            let dir = PathBuf::from(format!(
                "/tmp/lull-nm-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create fixture directory: {error}"),
            }
        };
        Self {
            dir,
            child: None,
            input: None,
        }
    }

    fn listener(&self) -> UnixListener {
        let listener = UnixListener::bind(self.dir.join("daemon.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        listener
    }

    fn spawn(&mut self, args: &[&str], use_symlink: bool, pipe_output: bool) {
        let binary = if use_symlink {
            let link = self.dir.join("lull-nm-host");
            symlink(env!("CARGO_BIN_EXE_lull"), &link).unwrap();
            link
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_lull"))
        };
        let mut command = Command::new(binary);
        if !use_symlink {
            command.arg("nm-host");
        }
        let mut child = command
            .args(args)
            .env("LULL_DIR", &self.dir)
            .stdin(Stdio::piped())
            .stdout(if pipe_output {
                Stdio::piped()
            } else {
                Stdio::from(File::create(self.dir.join("stdout")).unwrap())
            })
            .stderr(File::create(self.dir.join("stderr")).unwrap())
            .spawn()
            .unwrap();
        self.input = child.stdin.take();
        self.child = Some(child);
    }

    fn accept(&mut self, listener: &UnixListener) -> UnixStream {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    // macOS inherits the listener's nonblocking mode on accept.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                        "host exited before connecting: {}",
                        self.stderr()
                    );
                    assert!(Instant::now() < deadline, "host did not connect");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept fake daemon: {error}"),
            }
        }
    }

    fn send(&mut self, body: &[u8]) {
        let input = self.input.as_mut().unwrap();
        input.write_all(&(body.len() as u32).to_ne_bytes()).unwrap();
        input.write_all(body).unwrap();
        input.flush().unwrap();
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "host did not exit: {}",
                self.stderr()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn stderr(&self) -> String {
        fs::read_to_string(self.dir.join("stderr")).unwrap()
    }

    fn output_frames(&self) -> Vec<Vec<u8>> {
        let mut bytes = fs::read(self.dir.join("stdout")).unwrap();
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            assert!(bytes.len() >= 4);
            let length = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert!(length <= MAX_MESSAGE);
            assert!(bytes.len() >= 4 + length);
            frames.push(bytes[4..4 + length].to_vec());
            bytes.drain(..4 + length);
        }
        frames
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.input.take();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn chromium_origin() -> String {
    format!("chrome-extension://{}/", browsers::CHROMIUM_EXTENSION_ID)
}

fn read_value(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

#[test]
fn rejects_wrong_callers_before_connecting() {
    let origin = chromium_origin();
    let missing_slash = origin.trim_end_matches('/');
    for args in [
        vec![],
        vec!["chrome-extension://wrong/"],
        vec![missing_slash],
        vec!["/manifest.json", "wrong@addon"],
        vec!["/manifest.json", &origin],
        vec!["https://secret.example/path?token=private"],
    ] {
        let mut host = Host::new();
        let listener = host.listener();
        host.spawn(&args, false, false);
        assert_eq!(host.wait().code(), Some(1));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(host.stderr(), "lull-nm-host: caller is not allowed\n");
        assert!(host.output_frames().is_empty());
    }
}

#[test]
fn accepts_chromium_and_firefox_through_both_entry_points() {
    let origin = chromium_origin();
    for use_symlink in [false, true] {
        for args in [
            vec![origin.as_str()],
            vec!["/manifest.json", browsers::FIREFOX_ADDON_ID],
        ] {
            let mut host = Host::new();
            let listener = host.listener();
            host.spawn(&args, use_symlink, false);
            let mut daemon = host.accept(&listener);
            // No injected hello or other data before the extension sends a frame.
            host.input.take();
            assert_eq!(host.wait().code(), Some(0));
            let mut bytes = Vec::new();
            daemon.read_to_end(&mut bytes).unwrap();
            assert!(bytes.is_empty());
            assert!(host.output_frames().is_empty());
            assert!(host.stderr().is_empty());
        }
    }
}

#[test]
fn absent_or_refusing_daemon_fails_without_stdout() {
    for stale_socket in [false, true] {
        let mut host = Host::new();
        if stale_socket {
            drop(host.listener());
        }
        host.spawn(&[&chromium_origin()], false, false);
        assert_eq!(host.wait().code(), Some(1));
        assert_eq!(
            host.stderr(),
            "lull-nm-host: cannot connect to the daemon\n"
        );
        assert!(host.output_frames().is_empty());
    }
}

#[test]
fn browser_fixtures_and_command_acks_cross_the_real_bridge() {
    let fixtures = [
        include_str!("../../../protocol/fixtures/hello-browser.ndjson"),
        include_str!("../../../protocol/fixtures/browser-tabs-sync.ndjson"),
        include_str!("../../../protocol/fixtures/browser-activity.ndjson"),
        include_str!("../../../protocol/fixtures/tab-open.ndjson"),
        include_str!("../../../protocol/fixtures/tab-close.ndjson"),
        include_str!("../../../protocol/fixtures/tab-discard.ndjson"),
        include_str!("../../../protocol/fixtures/tab-reload.ndjson"),
        include_str!("../../../protocol/fixtures/tab-focus.ndjson"),
    ];
    let mut host = Host::new();
    let listener = host.listener();
    host.spawn(&[&chromium_origin()], true, false);
    let mut daemon = BufReader::new(host.accept(&listener));
    let mut expected_output = Vec::new();
    for fixture in fixtures {
        let values: Vec<Value> = fixture
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for pair in values.chunks_exact(2) {
            if pair[0]["id"].is_string() {
                writeln!(daemon.get_mut(), "{}", pair[0]).unwrap();
                expected_output.push(pair[0].clone());
                host.send(&serde_json::to_vec_pretty(&pair[1]).unwrap());
                assert_eq!(read_value(&mut daemon), pair[1]);
            } else {
                host.send(&serde_json::to_vec_pretty(&pair[0]).unwrap());
                assert_eq!(read_value(&mut daemon), pair[0]);
                writeln!(daemon.get_mut(), "{}", pair[1]).unwrap();
                expected_output.push(pair[1].clone());
            }
        }
    }
    daemon.get_mut().shutdown(Shutdown::Both).unwrap();
    assert_eq!(host.wait().code(), Some(0));
    let output: Vec<Value> = host
        .output_frames()
        .iter()
        .map(|body| serde_json::from_slice(body).unwrap())
        .collect();
    assert_eq!(output, expected_output);
    assert!(host.stderr().is_empty());
}

#[test]
fn malformed_inbound_frames_fail_and_never_reach_the_socket() {
    let mut malformed = Vec::new();
    for length in [MAX_MESSAGE as u32 + 1, u32::MAX] {
        malformed.push(length.to_ne_bytes().to_vec());
    }
    for count in 1..4 {
        malformed.push(8_u32.to_ne_bytes()[..count].to_vec());
    }
    for body in [
        b"secret.example/?token=private".as_slice(),
        b"\"\xff\"",
        b"",
        b"{}{}",
    ] {
        let mut wire = (body.len() as u32).to_ne_bytes().to_vec();
        wire.extend_from_slice(body);
        malformed.push(wire);
    }
    let mut truncated = 100_u32.to_ne_bytes().to_vec();
    truncated.extend_from_slice(b"secret.example/?token=private");
    malformed.push(truncated);
    for wire in malformed {
        let mut host = Host::new();
        let listener = host.listener();
        host.spawn(&[&chromium_origin()], false, false);
        let mut daemon = host.accept(&listener);
        host.input.as_mut().unwrap().write_all(&wire).unwrap();
        host.input.take();
        assert_eq!(host.wait().code(), Some(1));
        let mut bytes = Vec::new();
        daemon.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert!(host.output_frames().is_empty());
        assert_eq!(
            host.stderr(),
            "lull-nm-host: invalid message or bridge I/O failure\n"
        );
    }
}

#[test]
fn exact_limit_round_trips_and_oversized_daemon_lines_are_dropped() {
    let mut host = Host::new();
    let listener = host.listener();
    host.spawn(&[&chromium_origin()], false, false);
    let mut daemon = BufReader::new(host.accept(&listener));
    let body = format!("\"{}\"", "a".repeat(MAX_MESSAGE - 2));
    host.send(body.as_bytes());
    assert_eq!(read_value(&mut daemon), json!("a".repeat(MAX_MESSAGE - 2)));
    daemon.get_mut().write_all(body.as_bytes()).unwrap();
    daemon.get_mut().write_all(b"\n").unwrap();
    let oversized = format!(
        "\"https://secret.example/?token={}\"",
        "private".repeat(MAX_MESSAGE)
    );
    daemon.get_mut().write_all(oversized.as_bytes()).unwrap();
    daemon.get_mut().write_all(b"\n{}").unwrap();
    // A final line without a newline follows BufRead's EOF line semantics.
    daemon.get_mut().shutdown(Shutdown::Both).unwrap();
    assert_eq!(host.wait().code(), Some(0));
    assert_eq!(
        host.output_frames(),
        vec![body.into_bytes(), b"{}".to_vec()]
    );
    assert_eq!(
        host.stderr(),
        "lull-nm-host: dropped oversized daemon message\n"
    );
}

#[test]
fn daemon_eof_exits_even_with_a_partial_stdin_frame() {
    let mut host = Host::new();
    let listener = host.listener();
    host.spawn(&[&chromium_origin()], false, false);
    let daemon = host.accept(&listener);
    host.input.as_mut().unwrap().write_all(&[1, 0]).unwrap();
    daemon.shutdown(Shutdown::Both).unwrap();
    assert_eq!(host.wait().code(), Some(0));
    assert!(host.stderr().is_empty());
}

#[test]
fn stdin_eof_exits_even_with_a_partial_daemon_line() {
    let mut host = Host::new();
    let listener = host.listener();
    host.spawn(&[&chromium_origin()], false, false);
    let mut daemon = host.accept(&listener);
    daemon.write_all(b"{\"unfinished\":").unwrap();
    host.input.take();
    assert_eq!(host.wait().code(), Some(0));
    assert!(host.stderr().is_empty());
}

#[test]
fn stdin_eof_exits_when_browser_stdout_is_blocked() {
    let mut host = Host::new();
    let listener = host.listener();
    host.spawn(&[&chromium_origin()], false, true);
    let mut daemon = host.accept(&listener);
    // Keep the read end open but never drain it. One frame exceeds pipe capacity.
    let body = format!("\"{}\"\n", "a".repeat(MAX_MESSAGE - 2));
    let sender = thread::spawn(move || daemon.write_all(body.as_bytes()));
    thread::sleep(Duration::from_millis(50));
    host.input.take();
    assert_eq!(host.wait().code(), Some(0));
    let _ = sender.join().unwrap();
    assert!(host.stderr().is_empty());
}
