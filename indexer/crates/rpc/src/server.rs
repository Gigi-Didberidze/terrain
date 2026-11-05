//! Request dispatch. Cheap requests are answered inline; heavy work runs on worker threads.

use crate::protocol::{self, method};
use crate::rpc::{parse_params, Incoming, Outbox, RpcError, RpcResult};
use serde_json::Value;
use std::time::Instant;

#[derive(PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

pub struct Server {
    outbox: Outbox,
    started: Instant,
    initialized: bool,
}

impl Server {
    pub fn new(outbox: Outbox) -> Self {
        Self {
            outbox,
            started: Instant::now(),
            initialized: false,
        }
    }

    pub fn handle_raw(&mut self, body: &[u8]) -> Flow {
        let msg: Incoming = match serde_json::from_slice(body) {
            Ok(m) => m,
            Err(e) => {
                self.outbox.respond(
                    Value::Null,
                    Err(RpcError::new(RpcError::PARSE, e.to_string())),
                );
                return Flow::Continue;
            }
        };
        let Some(m) = msg.method.clone() else {
            return Flow::Continue;
        };
        if m == "exit" {
            return Flow::Exit;
        }
        let Some(id) = msg.id else {
            // Notifications from the client; none are handled yet.
            return Flow::Continue;
        };
        let result = self.dispatch(&m, msg.params);
        self.outbox.respond(id, result);
        Flow::Continue
    }

    fn dispatch(&mut self, m: &str, params: Value) -> RpcResult {
        match m {
            method::INITIALIZE => {
                let p: protocol::InitializeParams = parse_params(params)?;
                let _ = p;
                self.initialized = true;
                to_value(protocol::InitializeResult {
                    server_version: env!("CARGO_PKG_VERSION").into(),
                    protocol_version: protocol::PROTOCOL_VERSION,
                })
            }
            method::PING => to_value(protocol::PingResult {
                uptime_ms: self.started.elapsed().as_millis() as u64,
            }),
            method::SHUTDOWN => Ok(Value::Null),
            _ if !self.initialized => {
                Err(RpcError::new(RpcError::NOT_INITIALIZED, "not initialized"))
            }
            _ => Err(RpcError::new(
                RpcError::METHOD_NOT_FOUND,
                format!("unknown method {m}"),
            )),
        }
    }
}

pub fn to_value<T: serde::Serialize>(v: T) -> RpcResult {
    serde_json::to_value(v).map_err(RpcError::internal)
}
