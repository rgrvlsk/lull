//! Decodes every golden exchange in `protocol/fixtures/*.ndjson` with the typed enums and
//! re-encodes it, failing on any fixture that does not parse or does not round-trip.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use lull_protocol::rpc::{
    Ack, AnyRequest, ClientRequest, Message, PROTOCOL_MISMATCH, ProtocolMismatch, Request,
    RequestId,
};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures")
}

fn roundtrip<T: serde::Serialize>(name: &str, line: usize, typed: &T, raw: &Value) {
    let again = serde_json::to_value(typed).unwrap();
    assert_eq!(&again, raw, "{name}:{line} does not round-trip");
}

#[test]
fn every_fixture_decodes_and_round_trips() {
    let mut files: Vec<_> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 23,
        "expected the full fixture set, found {}",
        files.len()
    );

    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut pending: HashMap<RequestId, AnyRequest> = HashMap::new();
        let mut lines = 0;

        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            lines += 1;
            let raw: Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("{name}:{n} not JSON: {e}"));
            let msg: Message = serde_json::from_value(raw.clone())
                .unwrap_or_else(|e| panic!("{name}:{n} not a JSON-RPC message: {e}"));
            roundtrip(&name, n, &msg, &raw);

            match msg {
                Message::Request(req) => {
                    let typed = req
                        .typed()
                        .unwrap_or_else(|e| panic!("{name}:{n} unknown request: {e}"));
                    roundtrip(&name, n, &typed, &untyped(&req));
                    pending.insert(req.id.clone(), typed);
                }
                Message::Notification(ntf) => {
                    let typed = ntf
                        .typed()
                        .unwrap_or_else(|e| panic!("{name}:{n} unknown notification: {e}"));
                    roundtrip(&name, n, &typed, &raw_without_jsonrpc(&raw));
                }
                Message::Response(res) => {
                    let req = pending
                        .remove(&res.id)
                        .unwrap_or_else(|| panic!("{name}:{n} response to unknown id"));
                    match (req, res.result, res.error) {
                        (AnyRequest::Client(c), Some(result), None) => {
                            let typed = c.decode_result(&result).unwrap_or_else(|e| {
                                panic!("{name}:{n} bad result for {}: {e}", c.method())
                            });
                            roundtrip(&name, n, &typed, &result);
                        }
                        (AnyRequest::Client(ClientRequest::Hello(_)), None, Some(err)) => {
                            assert_eq!(err.code, PROTOCOL_MISMATCH, "{name}:{n}");
                            let data: ProtocolMismatch =
                                serde_json::from_value(err.data.expect("mismatch data"))
                                    .unwrap_or_else(|e| panic!("{name}:{n} bad data: {e}"));
                            assert_eq!(data.protocol, lull_protocol::PROTOCOL_VERSION);
                        }
                        (AnyRequest::Client(_), None, Some(err)) => {
                            assert!(err.code < 0, "{name}:{n} error code");
                        }
                        (AnyRequest::Browser(_), Some(result), None) => {
                            let ack: Ack = serde_json::from_value(result.clone())
                                .unwrap_or_else(|e| panic!("{name}:{n} bad ack: {e}"));
                            roundtrip(&name, n, &ack, &result);
                        }
                        _ => panic!("{name}:{n} response must have exactly one of result/error"),
                    }
                }
            }
        }
        assert!(lines >= 2, "{name} is empty");
        assert!(
            pending.is_empty(),
            "{name}: unanswered requests {:?}",
            pending.keys()
        );
    }
}

fn untyped(req: &Request) -> Value {
    serde_json::json!({ "method": req.method, "params": req.params })
}

fn raw_without_jsonrpc(raw: &Value) -> Value {
    let mut v = raw.clone();
    v.as_object_mut().unwrap().remove("jsonrpc");
    v
}
