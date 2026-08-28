use regex::{Captures, Regex};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent<'a> {
    pub kind: &'a str,
    pub outcome: &'a str,
    pub detail: String,
}

pub fn sanitize_detail(input: &str) -> String {
    let bearer = Regex::new(r"Bearer\s+[^\s]+\s*").expect("valid bearer regex");
    let windows =
        Regex::new(r"[A-Za-z]:\\Users\\[^\\\s]+\\[^\s,\)\]\}]+").expect("valid Windows path regex");
    let mac = Regex::new(r"/Users/[^/\s]+/[^\s,\)\]\}]+").expect("valid macOS path regex");
    let value = bearer.replace_all(input, |captures: &Captures<'_>| {
        let matched = captures.get(0).expect("bearer match is present").as_str();
        let suffix = &matched[matched.trim_end().len()..];
        format!("[REDACTED]{suffix}")
    });
    let value = windows.replace_all(&value, "[LOCAL_PATH]");
    mac.replace_all(&value, "[LOCAL_PATH]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::{sanitize_detail, AuditEvent};

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
    fn removes_every_bearer_token_in_a_detail() {
        let input = "first Bearer alpha second Bearer beta";

        assert_eq!(sanitize_detail(input), "first [REDACTED] second [REDACTED]");
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
        let event = AuditEvent {
            kind: "translationCompleted",
            outcome: "success",
            detail: sanitize_detail("Saved C:\\Users\\alex\\private.docx"),
        };

        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"kind":"translationCompleted","outcome":"success","detail":"Saved [LOCAL_PATH]"}"#
        );
    }
}
