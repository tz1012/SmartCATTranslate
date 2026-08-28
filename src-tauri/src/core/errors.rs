use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranslationError {
    ToolUseRejected,
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
