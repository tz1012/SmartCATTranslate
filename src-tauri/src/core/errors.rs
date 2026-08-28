use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum TranslationError {
    #[error("the translation request is invalid")]
    InvalidInput,
    #[error("the translation workspace is not application-owned and empty")]
    UnsafeWorkspace,
    #[error("the translation response violated the expected schema")]
    InvalidOutput,
    #[error("the translation exceeded its size limit")]
    SizeLimitExceeded,
    #[error("the translation attempted to use a prohibited tool")]
    ToolUseRejected,
    #[error("the translation runtime is unavailable")]
    RuntimeUnavailable,
    #[error("the translation runtime protocol was invalid")]
    ProtocolViolation,
    #[error("the translation timed out")]
    TimedOut,
    #[error("the translation was cancelled")]
    Cancelled,
    #[error("the application is shutting down")]
    ShuttingDown,
}

#[cfg(test)]
mod tests {
    use super::TranslationError;

    #[test]
    fn serializes_tool_rejection_as_the_typescript_wire_value() {
        assert_eq!(
            serde_json::to_string(&TranslationError::ToolUseRejected).unwrap(),
            r#""toolUseRejected""#
        );
    }
}
