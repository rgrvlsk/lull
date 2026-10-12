//! Serde round-trips and naming checks for each wire type family.

use lull_protocol::config::{AgentMarker, AutoStop, Config, ConfigPatch, ProjectRule, TabsOnStop};
use lull_protocol::history::{RestartStatus, StopReason};
use lull_protocol::rpc::{
    Ack, BrowserCommand, ClientRequest, DaemonNotification, Message, OpenParams, RequestId,
    ServerRemoved, TabIds,
};
use lull_protocol::server::{Activity, Agent, Framework, KnownAgent, OriginKind, ServerState};
use serde_json::{Value, json};

fn rt<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let s = serde_json::to_string(v).unwrap();
    let back: T = serde_json::from_str(&s).unwrap();
    assert_eq!(&back, v, "{s}");
}

#[test]
fn enums_use_contract_spelling() {
    assert_eq!(json!(ServerState::StoppingSoon), json!("stopping-soon"));
    assert_eq!(json!(ServerState::NoListener), json!("no-listener"));
    assert_eq!(json!(Framework::PhpBuiltin), json!("php-builtin"));
    assert_eq!(json!(Framework::PythonHttp), json!("python-http"));
    assert_eq!(json!(Activity::Traffic), json!("traffic"));
    assert_eq!(json!(OriginKind::Agent), json!("agent"));
    assert_eq!(json!(StopReason::Exited), json!("exited"));
    assert_eq!(json!(RestartStatus::RestartFailed), json!("restartFailed"));
    assert_eq!(json!(AutoStop::None), json!("none"));
    assert_eq!(json!(TabsOnStop::Discard), json!("discard"));
}

#[test]
fn agent_is_an_open_list() {
    let known: Agent = serde_json::from_value(json!("claude")).unwrap();
    assert_eq!(known, Agent::Known(KnownAgent::Claude));
    let other: Agent = serde_json::from_value(json!("my-agent")).unwrap();
    assert_eq!(other, Agent::Other("my-agent".into()));
    rt(&other);
}

#[test]
fn config_defaults_match_contract_and_toml_round_trips() {
    let d = Config::default();
    assert_eq!(d.idle_minutes, 30);
    assert_eq!(d.auto_stop, AutoStop::Agent);
    assert_eq!(d.warn_before_minutes, 2);
    assert_eq!(d.wake_grace_minutes, 5);
    assert_eq!(d.tabs_on_stop, TabsOnStop::Discard);
    assert_eq!(d.history_size, 20);
    assert_eq!(d.history_hours, 24);

    let text = r#"
idle_minutes = 30
auto_stop = "agent"
warn_before_minutes = 2
wake_grace_minutes = 5
tabs_on_stop = "discard"
open_with = ""
history_size = 20
history_hours = 24

[[project]]
path = "~/dev/example-app"
idle_minutes = 90

[[agent_marker]]
env = "MY_AGENT"
value = "1"
agent = "my-agent"
"#;
    let cfg: Config = toml::from_str(text).unwrap();
    assert_eq!(
        cfg.projects,
        vec![ProjectRule {
            path: "~/dev/example-app".into(),
            idle_minutes: Some(90),
            auto_stop: None,
        }]
    );
    assert_eq!(
        cfg.agent_markers,
        vec![AgentMarker {
            env: "MY_AGENT".into(),
            value: Some("1".into()),
            agent: "my-agent".into(),
        }]
    );
    let again: Config = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(again, cfg);
    rt(&cfg);

    // An empty file is the defaults; unknown keys are rejected as a whole (§6).
    assert_eq!(toml::from_str::<Config>("").unwrap(), Config::default());
    assert!(toml::from_str::<Config>("idle_minute = 3").is_err());
}

#[test]
fn config_patch_keeps_only_present_fields() {
    let p: ConfigPatch = serde_json::from_value(json!({"idle_minutes": 45})).unwrap();
    assert_eq!(p.idle_minutes, Some(45));
    assert_eq!(
        serde_json::to_value(&p).unwrap(),
        json!({"idle_minutes": 45})
    );
    assert!(serde_json::from_value::<ConfigPatch>(json!({"nope": 1})).is_err());
}

#[test]
fn requests_are_tagged_by_method() {
    let v = json!({"method": "servers.setAutoStop", "params": {"id": "1-2", "autoStop": null}});
    let r: ClientRequest = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(r.method(), "servers.setAutoStop");
    assert_eq!(serde_json::to_value(&r).unwrap(), v);

    let by_id: OpenParams = serde_json::from_value(json!({"id": "1-2"})).unwrap();
    assert!(matches!(by_id, OpenParams::Server { path: None, .. }));
    let by_url: OpenParams = serde_json::from_value(json!({"url": "http://localhost:1/"})).unwrap();
    assert!(matches!(by_url, OpenParams::Url { .. }));
    assert!(serde_json::from_value::<OpenParams>(json!({"id": "1", "url": "u"})).is_err());

    let c = BrowserCommand::TabDiscard(TabIds {
        tab_ids: vec![1, 2],
        ports: Vec::new(),
    });
    assert_eq!(
        json!(c),
        json!({"method": "tab.discard", "params": {"tabIds": [1, 2]}})
    );
    rt(&c);
}

#[test]
fn notifications_and_acks() {
    let n = DaemonNotification::ServerRemoved(ServerRemoved {
        seq: 7,
        id: "1-2".into(),
        reason: StopReason::Idle,
    });
    assert_eq!(
        json!(n),
        json!({"method": "server.removed", "params": {"seq": 7, "id": "1-2", "reason": "idle"}})
    );
    rt(&n);
    rt(&Ack::Ok { ok: true });
    rt(&Ack::Error {
        error: "mismatch".into(),
    });
    assert!(serde_json::from_value::<Ack>(json!({"ok": true, "error": "x"})).is_err());
}

#[test]
fn envelope_distinguishes_request_notification_response() {
    let req: Message = serde_json::from_value(
        json!({"jsonrpc": "2.0", "id": 1, "method": "servers.list", "params": {}}),
    )
    .unwrap();
    assert!(matches!(req, Message::Request(ref r) if r.id == RequestId::Number(1)));
    let ntf: Message =
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": "server.added", "params": {}}))
            .unwrap();
    assert!(matches!(ntf, Message::Notification(_)));
    let res: Message =
        serde_json::from_value(json!({"jsonrpc": "2.0", "id": "d1", "result": {"ok": true}}))
            .unwrap();
    assert!(matches!(res, Message::Response(ref r) if r.id == RequestId::String("d1".into())));
    assert!(
        serde_json::from_value::<Message>(json!({"jsonrpc": "1.0", "id": 1, "result": {}}))
            .is_err()
    );
    assert!(serde_json::from_value::<Value>(json!(null)).is_ok());
}
