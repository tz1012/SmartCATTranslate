use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize)]
pub struct JsonRpcRequest<T> {
    pub id: u64,
    pub method: String,
    pub params: T,
}

#[derive(Deserialize)]
pub struct JsonRpcResponse<T> {
    pub id: u64,
    #[serde(default)]
    pub result: Option<T>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    #[serde(default, rename = "message")]
    _message: Option<String>,
    #[serde(default, rename = "data")]
    _data: Option<Value>,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct AppServerNotification {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl fmt::Debug for AppServerNotification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppServerNotification")
            .field("method", &"<redacted>")
            .field("params", &"<redacted>")
            .finish()
    }
}
