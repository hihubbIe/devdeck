//! JSON-RPC 2.0, one message per line (MCP's framing on stdio). No batches; a message
//! without an id is a notification, which never gets an answer.
//!
//! Errors: protocol errors keep JSON-RPC's codes, devdeck's own use [`SERVER_ERROR`];
//! every error carries its [`ErrorKind`] in `data.kind`, so a client only reads that.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const VERSION: &str = "2.0";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// Every error of devdeck's own (refused, not found, permission…).
pub const SERVER_ERROR: i64 = -32000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// None: a notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self { jsonrpc: VERSION.into(), id: Some(id.into()), method: method.into(), params: Some(params) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    /// Null when the request couldn't be read.
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: VERSION.into(), id, result: Some(result), error: None }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self { jsonrpc: VERSION.into(), id, result: None, error: Some(error) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// What went wrong, for a client to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    NotFound,
    Invalid,
    /// devdeck won't, for the reason given (as the UI would say it).
    Refused,
    PermissionDenied,
    ApprovalRequired,
    ApprovalDenied,
    /// One of devdeck's own rules: a protected env, main/master.
    Gated,
    Conflict,
    Busy,
    /// What the call was bound to is gone (a caller's shell, a cursor).
    Expired,
    RateLimited,
    /// Not built yet, or devdeck can't now.
    Unavailable,
    /// A kind from a later level.
    #[serde(other)]
    Unknown,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::Invalid => "invalid",
            ErrorKind::Refused => "refused",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::ApprovalRequired => "approval_required",
            ErrorKind::ApprovalDenied => "approval_denied",
            ErrorKind::Gated => "gated",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Busy => "busy",
            ErrorKind::Expired => "expired",
            ErrorKind::RateLimited => "rate_limited",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::Unknown => "unknown",
        }
    }
}

/// An error as a client sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ApiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }

    /// As sent: devdeck's code, the kind in `data`.
    pub fn to_rpc(&self) -> RpcError {
        self.to_rpc_with(SERVER_ERROR)
    }

    /// As sent with a protocol code (parse, invalid request, unknown method, params).
    pub fn to_rpc_with(&self, code: i64) -> RpcError {
        RpcError { code, message: self.message.clone(), data: Some(serde_json::json!({ "kind": self.kind })) }
    }
}

impl From<RpcError> for ApiError {
    /// The kind in `data`, else one from the protocol's code.
    fn from(e: RpcError) -> Self {
        let kind = e.data.as_ref().and_then(|d| d.get("kind")).and_then(|k| serde_json::from_value(k.clone()).ok()).unwrap_or(match e.code {
            METHOD_NOT_FOUND => ErrorKind::NotFound,
            PARSE_ERROR | INVALID_REQUEST | INVALID_PARAMS => ErrorKind::Invalid,
            INTERNAL_ERROR => ErrorKind::Unavailable,
            _ => ErrorKind::Unknown,
        });
        Self { kind, message: e.message }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_carry_their_kind_both_ways() {
        let e = ApiError::not_found("no app front");
        let rpc = e.to_rpc();
        assert_eq!(rpc.code, SERVER_ERROR);
        assert_eq!(rpc.data, Some(json!({ "kind": "not_found" })));
        assert_eq!(ApiError::from(rpc), e);
        let later = RpcError { code: SERVER_ERROR, message: "m".into(), data: Some(json!({ "kind": "from_level_9" })) };
        assert_eq!(ApiError::from(later).kind, ErrorKind::Unknown, "a later kind doesn't break an older client");
        let bare = RpcError { code: METHOD_NOT_FOUND, message: "m".into(), data: None };
        assert_eq!(ApiError::from(bare).kind, ErrorKind::NotFound);
    }

    #[test]
    fn a_request_without_id_is_a_notification() {
        let r: Request = serde_json::from_str(r#"{"jsonrpc":"2.0","method":"x"}"#).unwrap();
        assert_eq!(r.id, None);
        let line = serde_json::to_string(&Request::new(3, "run.list", json!({}))).unwrap();
        assert_eq!(line, r#"{"jsonrpc":"2.0","id":3,"method":"run.list","params":{}}"#);
    }
}
