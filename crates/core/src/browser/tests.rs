//! Plan WP2.4 acceptance: two profiles, a reused port (no takeover until reload),
//! disconnect clearing activity and coverage, a second browser staying uncovered.

use super::*;
use crate::model::Witness;
use lull_protocol::server::{Framework, Origin, OriginKind};

fn server(id: &str, port: u16) -> ObservedServer {
    ObservedServer {
        id: id.into(),
        pgid: 1,
        members: vec![Witness {
            pid: 1,
            start_epoch_ms: 1,
        }],
        ports: vec![port],
        primary_url: Some(format!("http://127.0.0.1:{port}/")),
        cwd: "/Users/x/p".into(),
        project: "p".into(),
        framework: Framework::Vite,
        origin: Origin {
            kind: OriginKind::Agent,
            agent: None,
            session_id: None,
        },
        command: "vite".into(),
        launch_argv: vec!["vite".into()],
        restart_env: Default::default(),
        env_pin: false,
        started_at_ms: 1,
        cpu_percent: 0.0,
        rss_bytes: 0,
        incomplete: Vec::new(),
    }
}

fn tab(tab_id: i64, url: &str, focused: bool) -> BrowserTab {
    BrowserTab {
        tab_id,
        window_id: 1,
        url: url.into(),
        group_id: None,
        active: false,
        window_focused: focused,
    }
}

fn now() -> Instant {
    Instant {
        uptime_ms: 5,
        boot_session: "b".into(),
    }
}

#[test]
fn two_profiles_bind_their_own_tabs_and_open_goes_to_the_last_focused() {
    let servers = [server("A", 5173)];
    let mut r = Registry::default();
    r.connect(1, "helium".into(), Some(100));
    r.connect(2, "helium".into(), Some(100));
    r.sync(1, vec![tab(7, "http://localhost:5173/", true)], &servers);
    r.sync(2, vec![tab(7, "http://127.0.0.1:5173/x", false)], &servers);
    let tabs = r.tabs_of("A");
    assert_eq!(tabs.len(), 2);
    assert_eq!(
        tabs.iter()
            .map(|t| t.connection_id.as_str())
            .collect::<Vec<_>>(),
        ["c1", "c2"]
    );
    assert_eq!(r.open_target(), Some(1));
    r.sync(2, vec![tab(7, "http://127.0.0.1:5173/x", true)], &servers);
    assert_eq!(r.open_target(), Some(2));
    assert_eq!(r.tab_showing("http://127.0.0.1:5173/x"), Some((2, 7)));
}

#[test]
fn a_reused_port_does_not_take_over_a_tab_until_it_loads_again() {
    let mut r = Registry::default();
    r.connect(1, "helium".into(), None);
    r.sync(
        1,
        vec![tab(7, "http://localhost:5173/", true)],
        &[server("A", 5173)],
    );
    assert_eq!(r.bound_tabs("A")[&1], [7]);

    // A stops; B later listens on the same port. The tab stays bound to A's record.
    let b = [server("B", 5173)];
    r.sync(1, vec![tab(7, "http://localhost:5173/", true)], &b);
    assert_eq!(r.tabs_of("A").len(), 1);
    assert!(r.tabs_of("B").is_empty());

    // The page loads again (its content script reports activity): matched afresh.
    let ev = r
        .activity(1, 7, "http://localhost:5173/", &b, &now())
        .unwrap();
    assert_eq!(ev.server_id, "B");
    assert_eq!(ev.source, Activity::Tab);
    assert!(r.tabs_of("A").is_empty());

    // Navigation to another port is a new load too.
    r.sync(1, vec![tab(7, "http://localhost:9999/", true)], &b);
    assert!(r.tabs_of("B").is_empty());
}

#[test]
fn disconnect_drops_tabs_activity_and_coverage() {
    let servers = [server("A", 5173)];
    let mut r = Registry::default();
    r.connect(1, "helium".into(), Some(100));
    r.sync(1, vec![tab(7, "http://localhost:5173/", true)], &servers);
    assert!(r.covered().covers(&[300, 100]));
    r.disconnect(1);
    assert!(r.tabs_of("A").is_empty());
    assert!(!r.covered().covers(&[300, 100]));
    assert!(
        r.activity(1, 7, "http://localhost:5173/", &servers, &now())
            .is_none()
    );
    assert_eq!(r.open_target(), None);
}

#[test]
fn a_browser_without_the_extension_stays_uncovered() {
    let mut r = Registry::default();
    r.connect(1, "helium".into(), Some(100));
    // Chrome (pid 200) has no extension connection: its network helper is uncovered.
    assert!(!r.covered().covers(&[201, 200]));
    assert!(r.covered().covers(&[101, 100]));
}

#[test]
fn restart_moves_the_records_tabs_to_the_new_server() {
    let mut r = Registry::default();
    r.connect(1, "helium".into(), None);
    r.sync(
        1,
        vec![tab(7, "http://localhost:5173/", true)],
        &[server("A", 5173)],
    );
    r.rebind("A", "C");
    assert_eq!(r.bound_tabs("C")[&1], [7]);
}

#[test]
fn non_loopback_and_unknown_ports_are_never_bound() {
    let servers = [server("A", 5173)];
    let mut r = Registry::default();
    r.connect(1, "helium".into(), None);
    r.sync(
        1,
        vec![
            tab(1, "https://example.com:5173/", true),
            tab(2, "http://localhost:1/", true),
        ],
        &servers,
    );
    assert!(r.tabs_of("A").is_empty());
    assert!(
        r.activity(1, 1, "https://example.com:5173/", &servers, &now())
            .is_none()
    );
}
