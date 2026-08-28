use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use smartcat_translate::codex::runtime::{
    choose_runtime, download_verified, verify_sha256, RuntimeCandidate, RuntimeDownloader,
    RuntimeError, RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeHandshake,
    RuntimeHandshakeResult, RuntimeResolver, RuntimeSource,
};
use url::Url;

#[test]
fn prefers_compatible_system_runtime() {
    let candidates = vec![
        RuntimeCandidate::new("app-codex", "0.144.4", RuntimeSource::Bundled),
        RuntimeCandidate::new("system-codex", "0.145.0", RuntimeSource::System),
    ];

    let chosen = choose_runtime(candidates, "0.144.0").unwrap();

    assert_eq!(chosen.source, RuntimeSource::System);
}

#[test]
fn rejects_runtime_below_protocol_floor() {
    let result = choose_runtime(
        vec![RuntimeCandidate::new(
            "old-codex",
            "0.120.0",
            RuntimeSource::System,
        )],
        "0.144.0",
    );

    assert!(matches!(result, Err(RuntimeError::NoCompatibleRuntime)));
}

#[test]
fn accepts_bytes_with_the_pinned_checksum() {
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    assert_eq!(verify_sha256(b"abc", expected), Ok(()));
}

#[test]
fn rejects_bytes_that_do_not_match_the_pinned_checksum() {
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    assert!(matches!(
        verify_sha256(b"altered", expected),
        Err(RuntimeError::ChecksumMismatch)
    ));
}

#[tokio::test]
async fn verified_download_returns_only_checksum_matching_bytes() {
    let downloader = FakeDownloader { bytes: b"abc" };
    let url =
        Url::parse("https://github.com/openai/codex/releases/download/test/codex.zip").unwrap();
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    let bytes = download_verified(&downloader, &url, expected)
        .await
        .unwrap();

    assert_eq!(bytes, b"abc");
}

#[tokio::test]
async fn falls_back_to_app_local_when_system_handshake_is_incompatible() {
    let handshake = Arc::new(FakeHandshake::new(&[
        (RuntimeSource::System, "different-protocol"),
        (RuntimeSource::AppLocal, "pinned-protocol"),
    ]));
    let recorder = Arc::new(CapturedFailures::default());
    let resolver = RuntimeResolver::new(
        vec![
            RuntimeCandidate::new("system-codex", "0.145.0", RuntimeSource::System),
            RuntimeCandidate::new("app-codex", "0.144.4", RuntimeSource::AppLocal),
        ],
        "0.144.0",
        "pinned-protocol",
        handshake.clone(),
        recorder.clone(),
    );

    let runtime = resolver.resolve().await.unwrap();

    assert_eq!(runtime.source, RuntimeSource::AppLocal);
    assert_eq!(
        handshake.stopped.lock().unwrap().as_slice(),
        &[RuntimeSource::System]
    );
    assert_eq!(
        recorder.records.lock().unwrap().as_slice(),
        &[RuntimeFailureRecord {
            source: RuntimeSource::System,
            version: "0.145.0".to_owned(),
            error_code: "protocol_incompatible",
        }]
    );
    assert!(!format!("{:?}", recorder.records.lock().unwrap()).contains("system-codex"));
}

struct FakeDownloader {
    bytes: &'static [u8],
}

#[async_trait]
impl RuntimeDownloader for FakeDownloader {
    async fn download(&self, _url: &Url) -> Result<Vec<u8>, RuntimeError> {
        Ok(self.bytes.to_vec())
    }
}

struct FakeHandshake {
    results: HashMap<RuntimeSource, &'static str>,
    stopped: Mutex<Vec<RuntimeSource>>,
}

impl FakeHandshake {
    fn new(results: &[(RuntimeSource, &'static str)]) -> Self {
        Self {
            results: results.iter().copied().collect(),
            stopped: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl RuntimeHandshake for FakeHandshake {
    async fn initialize(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<RuntimeHandshakeResult, RuntimeError> {
        match self.results.get(&candidate.source) {
            Some(protocol_version) => Ok(RuntimeHandshakeResult {
                protocol_version: (*protocol_version).to_owned(),
            }),
            None => Err(RuntimeError::ProtocolIncompatible),
        }
    }

    async fn stop(&self, candidate: &RuntimeCandidate) {
        self.stopped.lock().unwrap().push(candidate.source);
    }
}

#[derive(Default)]
struct CapturedFailures {
    records: Mutex<Vec<RuntimeFailureRecord>>,
}

impl RuntimeFailureRecorder for CapturedFailures {
    fn record(&self, record: RuntimeFailureRecord) {
        self.records.lock().unwrap().push(record);
    }
}
