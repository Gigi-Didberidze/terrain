//! JSON-RPC 2.0 envelope types and the outbound message channel.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::mpsc::Sender;

#[derive(Debug, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: Option<Value>,
    pub method: Option<String>,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub const PARSE: i64 = -32700;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INTERNAL: i64 = -32603;
    pub const NOT_INITIALIZED: i64 = -32002;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid_params(e: impl std::fmt::Display) -> Self {
        Self::new(Self::INVALID_PARAMS, e.to_string())
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(Self::INTERNAL, e.to_string())
    }
}

pub type RpcResult = Result<Value, RpcError>;

/// Cloneable handle used by any thread to send responses and notifications.
#[derive(Clone)]
pub struct Outbox(pub Sender<Vec<u8>>);

impl Outbox {
    pub fn respond(&self, id: Value, result: RpcResult) {
        let msg = match result {
            Ok(v) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": v }),
            Err(e) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        };
        let _ = self.0.send(serde_json::to_vec(&msg).expect("serialisable"));
    }

    #[allow(dead_code)] // first used by index progress in M1
    pub fn notify<T: Serialize>(&self, method: &str, params: &T) {
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let _ = self.0.send(serde_json::to_vec(&msg).expect("serialisable"));
    }
}

pub fn parse_params<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, RpcError> {
    serde_json::from_value(v).map_err(RpcError::invalid_params)
}
