use regex::{Captures, Regex};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent<'a> {
    kind: &'a str,
    outcome: &'a str,
    detail: SafeAuditDetail,
}

impl<'a> AuditEvent<'a> {
    pub fn new(kind: &'a str, outcome: &'a str, detail: SafeAuditDetail) -> Self {
        Self {
            kind,
            outcome,
            detail,
        }
    }
}

const MAX_DIAGNOSTIC_CODE_LENGTH: usize = 64;
const ALLOWED_DIAGNOSTIC_CODES: &[&str] = &[
    "authentication_required",
    "request_cancelled",
    "runtime_unavailable",
    "tool_use_rejected",
    "translation_completed",
    "translation_failed",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditDetailError {
    InvalidDiagnosticCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SafeAuditDetail(String);

impl SafeAuditDetail {
    /// Accepts only short, lowercase diagnostic codes from the documented allowlist.
    pub fn diagnostic_code(code: &str) -> Result<Self, AuditDetailError> {
        let has_valid_grammar = !code.is_empty()
            && code.len() <= MAX_DIAGNOSTIC_CODE_LENGTH
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_');

        if has_valid_grammar && ALLOWED_DIAGNOSTIC_CODES.contains(&code) {
            Ok(Self(code.to_owned()))
        } else {
            Err(AuditDetailError::InvalidDiagnosticCode)
        }
    }

    /// Sanitizes an untrusted error string before accepting it only when it is an allowed code.
    pub fn from_untrusted_error(input: &str) -> Result<Self, AuditDetailError> {
        Self::diagnostic_code(&sanitize_detail(input))
    }
}

pub fn sanitize_detail(input: &str) -> String {
    let credential =
        Regex::new(r"(?i)\b(?:bearer|basic|token)\s+[^\s]+\s*").expect("valid credential regex");
    let api_key = Regex::new(r"(?i)(?P<prefix>\b(?:x-)?api[-_ ]?key\s*[:=]\s*)[^\s]+\s*")
        .expect("valid API key regex");
    let windows = Regex::new(r"[A-Za-z]:\\Users\\[^\\\s,\)\]\}]+(?:\\[^\s,\)\]\}]*)?")
        .expect("valid Windows path regex");
    let mac =
        Regex::new(r"/Users/[^/\s,\)\]\}]+(?:/[^\s,\)\]\}]*)?").expect("valid macOS path regex");
    let value = credential.replace_all(input, redacted_match);
    let value = api_key.replace_all(&value, |captures: &Captures<'_>| {
        let prefix = captures
            .name("prefix")
            .expect("API key prefix is present")
            .as_str();
        format!("{prefix}{}", redacted_match(captures))
    });
    let value = windows.replace_all(&value, "[LOCAL_PATH]");
    mac.replace_all(&value, "[LOCAL_PATH]").into_owned()
}

fn redacted_match(captures: &Captures<'_>) -> String {
    let matched = captures.get(0).expect("redacted match is present").as_str();
    let suffix = &matched[matched.trim_end().len()..];
    format!("[REDACTED]{suffix}")
}

#[cfg(test)]
mod tests {
    use super::{sanitize_detail, AuditEvent, SafeAuditDetail};

    #[test]
    fn removes_bearer_tokens_and_windows_paths() {
        let input = "Authorization: Bearer secret C:\\Users\\alex\\private.docx";
        let safe = sanitize_detail(input);
        assert_eq!(safe, "Authorization: [REDACTED] [LOCAL_PATH]");
    }

    #[test]
    fn removes_unix_home_paths() {
        assert_eq!(sanitize_detail("/Users/alex/private.pdf"), "[LOCAL_PATH]");
    }

    #[test]
    fn removes_windows_user_home_roots() {
        assert_eq!(sanitize_detail("C:\\Users\\alex"), "[LOCAL_PATH]");
    }

    #[test]
    fn removes_macos_user_home_roots() {
        assert_eq!(sanitize_detail("/Users/alex"), "[LOCAL_PATH]");
    }

    #[test]
    fn removes_every_bearer_token_in_a_detail() {
        let input = "first Bearer alpha second Bearer beta";

        assert_eq!(sanitize_detail(input), "first [REDACTED] second [REDACTED]");
    }

    #[test]
    fn removes_case_insensitive_basic_token_and_api_key_credentials() {
        let input = "authorization: bAsIc basicValue\nTOKEN tokenValue\nX-API-Key: apiValue";

        assert_eq!(
            sanitize_detail(input),
            "authorization: [REDACTED]\n[REDACTED]\nX-API-Key: [REDACTED]"
        );
    }

    #[test]
    fn preserves_punctuation_adjacent_to_local_paths() {
        let input = "Open (C:\\Users\\alex\\private.docx), then (/Users/alex/private.pdf).";

        assert_eq!(
            sanitize_detail(input),
            "Open ([LOCAL_PATH]), then ([LOCAL_PATH])."
        );
    }

    #[test]
    fn leaves_ordinary_non_sensitive_details_unchanged() {
        let input = "Translation completed in 42 ms";

        assert_eq!(sanitize_detail(input), input);
    }

    #[test]
    fn serializes_audit_fields_as_camel_case() {
        let event = AuditEvent::new(
            "translationCompleted",
            "success",
            SafeAuditDetail::diagnostic_code("translation_completed").unwrap(),
        );

        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"kind":"translationCompleted","outcome":"success","detail":"translation_completed"}"#
        );
    }

    #[test]
    fn rejects_translation_prose_before_it_can_be_serialized_as_an_audit_event() {
        let attempted_translation =
            "Sensitive translated sentence Bearer credential C:\\Users\\alex\\private.docx";
        let serialization_attempt = SafeAuditDetail::diagnostic_code(attempted_translation)
            .map(|detail| AuditEvent::new("translationCompleted", "success", detail))
            .map(|event| serde_json::to_string(&event).unwrap());

        assert!(serialization_attempt.is_err());
    }
}
