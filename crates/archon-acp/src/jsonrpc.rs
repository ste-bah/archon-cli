//! JSON-RPC 2.0 framing for ACP (#189 Phase 11).
//!
//! One JSON value per line, both directions. ACP uses newline-delimited JSON
//! rather than the `Content-Length` headers LSP uses, so a message must never
//! contain a raw newline — `serde_json::to_string` guarantees that, and this
//! module is the only place that writes to the stream.

use serde::{Deserialize, Serialize};

/// A message read off the wire.
///
/// Requests and notifications differ only by the presence of `id`, so they
/// arrive as one type and are told apart by asking.
#[derive(Debug, Clone, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Present when this is a reply to something *we* sent.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

impl Incoming {
    /// A call that expects a reply.
    #[must_use]
    pub fn is_request(&self) -> bool {
        self.method.is_some() && self.id.is_some()
    }

    /// A one-way message. `session/cancel` is the one that matters: it arrives
    /// while a turn is in flight, so it must be handled without waiting for
    /// anything the turn is doing.
    #[must_use]
    pub fn is_notification(&self) -> bool {
        self.method.is_some() && self.id.is_none()
    }

    /// A reply to a request this side sent, such as a permission answer.
    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.method.is_none() && self.id.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: serde_json::Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl Response {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(ResponseError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
}

/// JSON-RPC's own codes. Using the right one matters: a client distinguishes
/// "I asked for something you do not have" from "you broke" and reports them
/// very differently.
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

#[cfg(test)]
#[path = "jsonrpc_tests.rs"]
mod tests;
