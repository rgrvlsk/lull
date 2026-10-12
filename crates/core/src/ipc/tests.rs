//! Plan WP1.5 acceptance: every fixture in `protocol/fixtures/` replays against the
//! daemon running on fakes. Each request must get the fixture's outcome: the same error
//! code, or a result that decodes as the method's typed result.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lull_protocol::rpc::{AnyRequest, Request};
use serde_json::{Value, json};

use super::*;
use crate::model::{Listener, ProcessInfo, Snapshot};
use crate::testing::{
    FakeClock, FakeProcessSource, FakeSpawner, MemoryConfigStore, MemoryStateStore,
    RecordingSignaler, env,
};
use crate::traits::Signal;

/// A fake shared between the daemon and the test, so the test can move it afterwards.
#[derive(Default)]
struct Shared<T>(Arc<Mutex<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl Clock for Shared<FakeClock> {
    fn now(&self) -> Instant {
        self.0.lock().unwrap().now()
    }
    fn wall_now_ms(&self) -> u64 {
        self.0.lock().unwrap().wall_now_ms()
    }
}

impl ProcessSource for Shared<FakeProcessSource> {
    fn snapshot(&mut self, also: &[u32]) -> io::Result<Snapshot> {
        self.0.lock().unwrap().snapshot(also)
    }
    fn group(&mut self, pgid: u32) -> io::Result<Vec<ProcessInfo>> {
        self.0.lock().unwrap().group(pgid)
    }
    fn parent(&mut self, pid: u32) -> Option<u32> {
        self.0.lock().unwrap().parent(pid)
    }
}

impl Signaler for Shared<RecordingSignaler> {
    fn signal_group(&mut self, pgid: u32, signal: Signal) -> io::Result<()> {
        self.0.lock().unwrap().signal_group(pgid, signal)
    }
}

impl StateStore for Shared<MemoryStateStore> {
    fn load(&mut self) -> io::Result<Option<Value>> {
        self.0.lock().unwrap().load()
    }
    fn save(&mut self, state: &Value) -> io::Result<()> {
        self.0.lock().unwrap().save(state)
    }
    fn quarantine(&mut self) -> io::Result<String> {
        self.0.lock().unwrap().quarantine()
    }
}

const START: u64 = 1_791_530_155_000;

fn proc(pid: u32, start: u64, argv: &[&str]) -> ProcessInfo {
    ProcessInfo {
        pid,
        pgid: 22249,
        ppid: 1,
        uid: 501,
        start_epoch_ms: start,
        exe: Some("/opt/homebrew/bin/node".into()),
        argv: Some(argv.iter().map(|s| (*s).to_owned()).collect()),
        env: Some(env(&[
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "d263"),
        ])),
        cwd: Some("/Users/x/proj/apps/crm".into()),
        has_tty: Some(false),
        ..ProcessInfo::default()
    }
}

/// The fixtures' agent-started vite server `22249-1791530155000`.
fn crm_snapshot() -> Snapshot {
    let mut vite = proc(
        22276,
        START + 500,
        &["node", "/Users/x/proj/apps/crm/node_modules/.bin/vite"],
    );
    vite.ppid = 22250;
    Snapshot {
        processes: vec![proc(22250, START, &["npm", "run", "dev"]), vite],
        listeners: vec![Listener {
            pid: 22276,
            addr: "127.0.0.1".into(),
            port: 5187,
        }],
        ..Snapshot::default()
    }
}

/// Stopped-server records the history/restart/forget fixtures refer to, plus one without
/// a launch command.
fn seeded_state() -> Value {
    let text = fs::read_to_string(fixtures_dir().join("servers-history.ndjson")).unwrap();
    let reply: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
    let mut history: Vec<Value> = reply["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| json!({"server": s, "stoppedAtMs": 1_760_227_200_000u64}))
        .collect();
    let mut bare = history[0]["server"].clone();
    bare["id"] = json!("18300-1791410000000");
    bare["argv"] = json!([]);
    history.push(json!({"server": bare, "stoppedAtMs": 1_760_227_200_000u64}));
    json!({"version": 1, "servers": {}, "history": history})
}

struct Harness {
    d: Daemon,
    clock: Shared<FakeClock>,
    source: Shared<FakeProcessSource>,
    signals: Shared<RecordingSignaler>,
    store: Shared<MemoryStateStore>,
    opened: Arc<Mutex<Vec<(String, bool)>>>,
}

fn harness() -> Harness {
    let clock = Shared::<FakeClock>::default();
    let source = Shared(Arc::new(Mutex::new(FakeProcessSource::new(crm_snapshot()))));
    let signals = Shared::<RecordingSignaler>::default();
    let store = Shared(Arc::new(Mutex::new(MemoryStateStore {
        state: Some(seeded_state()),
        ..MemoryStateStore::default()
    })));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let sink = opened.clone();
    let platform = Platform {
        source: Box::new(source.clone()),
        signaler: Box::new(signals.clone()),
        spawner: Box::new(FakeSpawner {
            next_pid: 40020,
            ..FakeSpawner::default()
        }),
        store: Box::new(store.clone()),
        config: Box::new(MemoryConfigStore::default()),
        clock: Box::new(clock.clone()),
        open_url: Box::new(move |url, _app, background| {
            sink.lock().unwrap().push((url.to_owned(), background));
            Ok(())
        }),
        log: Box::new(|_| {}),
        git_root: Box::new(|_| None),
    };
    let settings = Settings {
        home: "/Users/x".into(),
        uid: 501,
        daemon_pid: 999,
        daemon_pgid: 999,
        restart_log_dir: "/tmp/lull-test-logs".into(),
        dry_run: false,
        scan_interval_ms: 5_000,
    };
    let mut d = Daemon::new(platform, settings, None);
    d.scan();
    Harness {
        d,
        clock,
        source,
        signals,
        store,
        opened,
    }
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures")
}

const HELLO: &str = r#"{"jsonrpc":"2.0","id":0,"method":"hello","params":{"client":"cli","version":"0.1.0","protocol":{"major":1,"minor":1}}}"#;

fn reply(outs: &[Out], id: &Value) -> Value {
    outs.iter()
        .map(|o| serde_json::from_str::<Value>(&o.line).unwrap())
        .find(|v| v.get("id") == Some(id) && v.get("method").is_none())
        .unwrap_or_else(|| panic!("no reply to {id}"))
}

#[test]
fn every_fixture_request_replays() {
    let mut files: Vec<_> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    files.sort();
    let mut replayed = 0;
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let lines: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let mut h = harness();
        h.d.connected(1, false);
        if !name.starts_with("hello") {
            h.d.handle_line(1, HELLO);
        }
        for (i, msg) in lines.iter().enumerate() {
            // Client → daemon requests have numeric ids; string ids are daemon → browser.
            let (Some(id), Some(_)) = (msg.get("id"), msg.get("method")) else {
                continue;
            };
            if !id.is_number() {
                continue;
            }
            let expected = lines[i + 1..]
                .iter()
                .find(|l| l.get("id") == Some(id) && l.get("method").is_none())
                .unwrap_or_else(|| panic!("{name}: fixture has no reply to {id}"));
            let got = reply(&h.d.handle_line(1, &msg.to_string()), id);
            let at = format!("{name} id {id}: got {got}");
            match expected.get("error") {
                Some(err) => assert_eq!(got["error"]["code"], err["code"], "{at}"),
                None => {
                    assert!(got.get("error").is_none(), "{at}");
                    let req: Request = serde_json::from_value(msg.clone()).unwrap();
                    let Ok(AnyRequest::Client(typed)) = req.typed() else {
                        panic!("{at}")
                    };
                    typed
                        .decode_result(&got["result"])
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                }
            }
            replayed += 1;
        }
    }
    // Every numeric-id request in the fixture set (string ids are browser commands).
    assert_eq!(replayed, 24, "replayed {replayed} requests");
}

#[test]
fn requests_before_hello_are_refused() {
    let mut h = harness();
    h.d.connected(1, false);
    let outs = h.d.handle_line(
        1,
        r#"{"jsonrpc":"2.0","id":1,"method":"servers.list","params":{}}"#,
    );
    assert_eq!(reply(&outs, &json!(1))["error"]["code"], json!(-32600));
}

#[test]
fn subscribers_get_sequenced_updates_and_removal() {
    let mut h = harness();
    h.d.connected(1, false);
    h.d.handle_line(1, HELLO);
    let sub = h.d.handle_line(
        1,
        r#"{"jsonrpc":"2.0","id":1,"method":"subscribe","params":{"topics":["servers"]}}"#,
    );
    let seq = reply(&sub, &json!(1))["result"]["seq"].as_u64().unwrap();
    let outs = h.d.handle_line(1, r#"{"jsonrpc":"2.0","id":2,"method":"servers.pin","params":{"id":"22249-1791530155000","pinned":true}}"#);
    let note: Value = serde_json::from_str(&outs.last().unwrap().line).unwrap();
    assert_eq!(note["method"], "server.updated");
    assert_eq!(note["params"]["seq"], json!(seq + 1));
    assert_eq!(note["params"]["server"]["pinned"], json!(true));

    // Manual stop: SIGTERM at the next scan, removal once the group is empty.
    h.d.handle_line(
        1,
        r#"{"jsonrpc":"2.0","id":3,"method":"servers.stop","params":{"id":"22249-1791530155000"}}"#,
    );
    assert!(h.d.take_scan_request());
    h.d.scan();
    assert_eq!(
        h.signals.0.lock().unwrap().sent,
        vec![(22249, Signal::Term)]
    );
    h.source.0.lock().unwrap().snapshot = Snapshot::default();
    h.clock.0.lock().unwrap().advance_ms(5_000);
    let outs = h.d.scan();
    let note: Value = serde_json::from_str(&outs.last().unwrap().line).unwrap();
    assert_eq!(note["method"], "server.removed");
    assert_eq!(note["params"]["reason"], "manual");
    assert_eq!(note["params"]["seq"], json!(seq + 3));
}

#[test]
fn open_defaults_to_background_for_agent_callers() {
    let mut h = harness();
    h.d.connected(1, true);
    h.d.handle_line(1, HELLO);
    let open = |h: &mut Harness, params: &str| {
        let line =
            format!(r#"{{"jsonrpc":"2.0","id":5,"method":"servers.open","params":{params}}}"#);
        reply(&h.d.handle_line(1, &line), &json!(5))
    };
    open(&mut h, r#"{"url":"http://localhost:5187/a"}"#);
    open(&mut h, r#"{"url":"http://localhost:5187/b","focus":true}"#);
    open(&mut h, r#"{"id":"22249-1791530155000","path":"/admin"}"#);
    assert_eq!(
        *h.opened.lock().unwrap(),
        vec![
            ("http://localhost:5187/a".to_owned(), true),
            ("http://localhost:5187/b".to_owned(), false),
            ("http://127.0.0.1:5187/admin".to_owned(), true),
        ]
    );
    let bad = open(&mut h, r#"{"url":"http://example.com/"}"#);
    assert_eq!(bad["error"]["code"], json!(INVALID_PARAMS));
    let server = h.d.servers().remove(0);
    assert_eq!(server.last_activity_source, lull_protocol::Activity::Manual);
}

#[test]
fn only_loopback_urls_are_opened() {
    for ok in [
        "http://localhost:5173/",
        "https://app.localhost",
        "http://127.0.0.1:8080/x?y",
        "http://[::1]:3000/",
        "HTTP://LOCALHOST:1/",
    ] {
        assert!(loopback_url_port(ok).is_some(), "{ok}");
    }
    for bad in [
        "https://example.com/",
        "file:///etc/passwd",
        "http://localhost@evil.com/",
        "http://127.0.0.1.evil.com/",
        "javascript:alert(1)",
        "http://localhost.evil.com/",
        "https://example.com\\.localhost/",
        "http://example.com%2F.localhost/",
    ] {
        assert!(loopback_url_port(bad).is_none(), "{bad}");
    }
}

#[test]
fn pin_is_saved_before_the_next_scan() {
    let mut h = harness();
    h.d.connected(1, false);
    h.d.handle_line(1, HELLO);
    let saves_before = h.store.0.lock().unwrap().saves;
    let outs = h.d.handle_line(1, r#"{"jsonrpc":"2.0","id":2,"method":"servers.pin","params":{"id":"22249-1791530155000","pinned":true}}"#);
    let ack = reply(&outs, &json!(2));
    assert!(ack.get("error").is_none(), "{ack}");
    assert_eq!(ack["result"]["pinned"], json!(true));
    // No scan after the request: the acknowledged mutation must already be saved.
    let store = h.store.0.lock().unwrap();
    assert_eq!(store.saves, saves_before + 1);
    assert_eq!(
        store.state.as_ref().unwrap()["servers"]["22249-1791530155000"]["pinned"],
        json!(true)
    );
}
