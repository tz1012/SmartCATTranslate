use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub event: DiagnosticEventName,
    pub outcome: DiagnosticOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<JobKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    pub platform: &'static str,
    pub app_version: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticEventName {
    JobLifecycle,
    HistoryMaintenance,
    TemporaryCleanup,
    SecureStorage,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticOutcome {
    Started,
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Text,
    Popup,
    Capture,
    Document,
}

impl DiagnosticEvent {
    pub fn new(event: DiagnosticEventName, outcome: DiagnosticOutcome) -> Self {
        Self {
            event,
            outcome,
            duration_ms: None,
            job_kind: None,
            stage: None,
            error_code: None,
            item_count: None,
            byte_count: None,
            platform: std::env::consts::OS,
            app_version: env!("CARGO_PKG_VERSION"),
        }
    }
    pub fn with_error_code(mut self, code: &str) -> Self {
        self.error_code = valid_code(code).then(|| code.to_owned());
        self
    }
    pub fn with_stage(mut self, stage: &str) -> Self {
        self.stage = valid_code(stage).then(|| stage.to_owned());
        self
    }
    pub fn with_job_kind(mut self, job_kind: JobKind) -> Self {
        self.job_kind = Some(job_kind);
        self
    }
    pub fn with_counts(mut self, item_count: u64, byte_count: u64) -> Self {
        self.item_count = Some(item_count);
        self.byte_count = Some(byte_count);
        self
    }
    pub fn emit(&self) {
        if let Ok(encoded) = serde_json::to_string(self) {
            eprintln!("{encoded}");
        }
    }
}
fn valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || matches!(v, b'_' | b'-'))
}
