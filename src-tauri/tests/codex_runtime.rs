use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use semver::Version;
use smartcat_translate::codex::runtime::{
    choose_runtime, verify_sha256, LiveRuntimeSession, RuntimeCandidate, RuntimeError,
    RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeLauncher, RuntimeResolver, RuntimeSource,
};

#[test]
fn prefers_compatible_system_runtime() {
    let candidates = vec![
        RuntimeCandidate::system("older-system", Version::parse("0.144.4").unwrap()),
        RuntimeCandidate::system("newer-system", Version::parse("0.145.0").unwrap()),
    ];
    let chosen = choose_runtime(candidates, "0.144.0").unwrap();
    assert_eq!(chosen.path().to_string_lossy(), "older-system");
    assert_eq!(chosen.source(), RuntimeSource::System);
}

#[test]
fn rejects_runtime_below_protocol_floor() {
    let result = choose_runtime(
        vec![RuntimeCandidate::system(
            "old-system",
            Version::parse("0.120.0").unwrap(),
        )],
        "0.144.0",
    );
    assert!(matches!(result, Err(RuntimeError::NoCompatibleRuntime)));
}

#[test]
fn verifies_pinned_sha256_bytes() {
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(verify_sha256(b"abc", expected), Ok(()));
    assert!(matches!(
        verify_sha256(b"altered", expected),
        Err(RuntimeError::ChecksumMismatch)
    ));
}

#[tokio::test]
async fn returns_the_exact_initialized_live_session_without_relaunch() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let launcher = Arc::new(FakeLauncher::new(
        &[
            ("system-one", "different-protocol", false),
            ("system-two", "pinned-protocol", false),
        ],
        events.clone(),
    ));
    let recorder = Arc::new(CapturedFailures::default());
    let resolver = RuntimeResolver::system_only(
        vec![
            RuntimeCandidate::system("system-one", Version::parse("0.145.0").unwrap()),
            RuntimeCandidate::system("system-two", Version::parse("0.145.0").unwrap()),
        ],
        "0.144.0",
        "pinned-protocol",
        launcher.clone(),
        recorder.clone(),
    );

    let resolved = resolver.resolve().await.unwrap();

    assert_eq!(resolved.source(), RuntimeSource::System);
    assert_eq!(resolved.session_id(), "session:system-two");
    assert_eq!(launcher.starts(), vec!["system-one", "system-two"]);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        &[
            "initialize:system-one",
            "stop:system-one",
            "initialize:system-two"
        ]
    );
    assert_eq!(
        recorder.records.lock().unwrap().as_slice(),
        &[RuntimeFailureRecord {
            source: RuntimeSource::System,
            version: "0.145.0".to_owned(),
            error_code: "protocol_incompatible",
        }]
    );
}

#[tokio::test]
async fn stop_failure_blocks_fallback_and_is_sanitized() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let launcher = Arc::new(FakeLauncher::new(
        &[
            ("system-one", "different-protocol", true),
            ("system-two", "pinned-protocol", false),
        ],
        events,
    ));
    let resolver = RuntimeResolver::system_only(
        vec![
            RuntimeCandidate::system("system-one", Version::parse("0.145.0").unwrap()),
            RuntimeCandidate::system("system-two", Version::parse("0.145.0").unwrap()),
        ],
        "0.144.0",
        "pinned-protocol",
        launcher.clone(),
        Arc::new(CapturedFailures::default()),
    );

    let result = resolver.resolve().await;

    assert!(matches!(result, Err(RuntimeError::StopFailed)));
    assert_eq!(launcher.starts(), vec!["system-one"]);
    assert_eq!(RuntimeError::StopFailed.code(), "runtime_stop_failed");
}

struct FakeLauncher {
    outcomes: HashMap<String, (&'static str, bool)>,
    starts: Mutex<Vec<String>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl FakeLauncher {
    fn new(outcomes: &[(&str, &'static str, bool)], events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            outcomes: outcomes
                .iter()
                .map(|(path, protocol, stop_fails)| ((*path).to_owned(), (*protocol, *stop_fails)))
                .collect(),
            starts: Mutex::new(Vec::new()),
            events,
        }
    }

    fn starts(&self) -> Vec<String> {
        self.starts.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimeLauncher for FakeLauncher {
    async fn start(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError> {
        let path = candidate.path().to_string_lossy().into_owned();
        self.starts.lock().unwrap().push(path.clone());
        let (protocol, stop_fails) = self.outcomes[&path];
        Ok(Box::new(FakeSession {
            path,
            protocol,
            stop_fails,
            events: self.events.clone(),
        }))
    }
}

struct FakeSession {
    path: String,
    protocol: &'static str,
    stop_fails: bool,
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl LiveRuntimeSession for FakeSession {
    fn session_id(&self) -> &str {
        match self.path.as_str() {
            "system-one" => "session:system-one",
            "system-two" => "session:system-two",
            _ => "session:unknown",
        }
    }

    async fn initialize(&mut self) -> Result<String, RuntimeError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("initialize:{}", self.path));
        Ok(self.protocol.to_owned())
    }

    async fn stop(&mut self) -> Result<(), RuntimeError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("stop:{}", self.path));
        if self.stop_fails {
            Err(RuntimeError::StopFailed)
        } else {
            Ok(())
        }
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
