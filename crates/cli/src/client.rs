//! Socket client used by the CLI: connect, `hello`, then one call per request (§8).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use lull_protocol::rpc::{ClientKind, HelloParams, PROTOCOL_MISMATCH};
use lull_protocol::{PROTOCOL_VERSION, ProtocolVersion};
use serde_json::{Value, json};

use crate::paths::Paths;

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: i64,
}

impl Client {
    pub fn connect(paths: &Paths) -> Result<Client, String> {
        let socket = paths.socket();
        let writer = UnixStream::connect(&socket).map_err(|e| {
            format!(
                "cannot reach the lull daemon at {} ({e}); is it running? Try `brew services start lull`.",
                socket.display()
            )
        })?;
        let reader = BufReader::new(writer.try_clone().map_err(|e| e.to_string())?);
        let mut client = Client {
            reader,
            writer,
            next_id: 0,
        };
        client.hello(PROTOCOL_VERSION)?;
        Ok(client)
    }

    fn hello(&mut self, protocol: ProtocolVersion) -> Result<Value, String> {
        let params = HelloParams {
            client: ClientKind::Cli,
            version: env!("CARGO_PKG_VERSION").into(),
            protocol,
            browser: None,
        };
        self.call("hello", serde_json::to_value(params).unwrap())
    }

    /// One JSON-RPC call; an error response becomes `Err(message)`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.writer, "{line}").map_err(|e| e.to_string())?;
        loop {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf).map_err(|e| e.to_string())? == 0 {
                return Err("the daemon closed the connection".into());
            }
            let msg: Value = serde_json::from_str(&buf).map_err(|e| e.to_string())?;
            if msg.get("id") != Some(&json!(id)) {
                continue; // a notification
            }
            if let Some(err) = msg.get("error") {
                let code = err["code"].as_i64().unwrap_or(0);
                let message = err["message"].as_str().unwrap_or("error");
                if code == PROTOCOL_MISMATCH {
                    return Err(format!("{message}: update lull (daemon {})", err["data"]));
                }
                return Err(message.to_owned());
            }
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}
