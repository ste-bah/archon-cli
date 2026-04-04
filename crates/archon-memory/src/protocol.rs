//! JSON-RPC-over-TCP wire types for the singleton memory server.

use serde::{Deserialize, Serialize};

use crate::types::MemoryError;

/// A JSON-RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

/// A JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

/// A JSON-RPC error payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub message: String,
}

/// Serialize a request to a newline-terminated JSON string.
pub fn make_request(id: u64, method: &str, params: serde_json::Value) -> String {
    let req = Request {
        id,
        method: method.to_string(),
        params,
    };
    let mut json = serde_json::to_string(&req).unwrap_or_default();
    json.push('\n');
    json
}

/// Parse a response from a newline-terminated JSON line.
pub fn parse_response(line: &str) -> Result<Response, MemoryError> {
    serde_json::from_str(line.trim()).map_err(MemoryError::from)
}

/// Serialize a success response to a newline-terminated JSON string.
pub fn make_response_ok(id: u64, result: serde_json::Value) -> String {
    let resp = Response {
        id,
        result: Some(result),
        error: None,
    };
    let mut json = serde_json::to_string(&resp).unwrap_or_default();
    json.push('\n');
    json
}

/// Serialize an error response to a newline-terminated JSON string.
pub fn make_response_err(id: u64, message: String) -> String {
    let resp = Response {
        id,
        result: None,
        error: Some(RpcError { message }),
    };
    let mut json = serde_json::to_string(&resp).unwrap_or_default();
    json.push('\n');
    json
}
