use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use semver::Version;
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum RuntimeSource {
    System,
    AppLocal,
    Bundled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCandidate {
    pub path: PathBuf,
    pub version: Version,
    pub source: RuntimeSource,
}

impl RuntimeCandidate {
    pub fn new(path: impl Into<PathBuf>, version: &str, source: RuntimeSource) -> Self {
        Self {
            path: path.into(),
            version: Version::parse(version).expect("runtime version must be valid semver"),
            source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexRuntime {
    pub path: PathBuf,
    pub version: Version,
    pub source: RuntimeSource,
}

impl From<RuntimeCandidate> for CodexRuntime {
    fn from(candidate: RuntimeCandidate) -> Self {
        Self {
            path: candidate.path,
            version: candidate.version,
            source: candidate.source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFailureRecord {
    pub source: RuntimeSource,
    pub version: String,
    pub error_code: &'static str,
}

pub trait RuntimeFailureRecorder: Send + Sync {
    fn record(&self, record: RuntimeFailureRecord);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHandshakeResult {
    pub protocol_version: String,
}

#[async_trait]
pub trait RuntimeHandshake: Send + Sync {
    async fn initialize(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<RuntimeHandshakeResult, RuntimeError>;

    async fn stop(&self, candidate: &RuntimeCandidate);
}

#[async_trait]
pub trait RuntimeDownloader: Send + Sync {
    async fn download(&self, url: &Url) -> Result<Vec<u8>, RuntimeError>;
}

pub struct RuntimeResolver {
    candidates: Vec<RuntimeCandidate>,
    minimum_version: String,
    expected_protocol: String,
    handshake: Arc<dyn RuntimeHandshake>,
    failure_recorder: Arc<dyn RuntimeFailureRecorder>,
}

impl RuntimeResolver {
    pub fn new(
        candidates: Vec<RuntimeCandidate>,
        minimum_version: impl Into<String>,
        expected_protocol: impl Into<String>,
        handshake: Arc<dyn RuntimeHandshake>,
        failure_recorder: Arc<dyn RuntimeFailureRecorder>,
    ) -> Self {
        Self {
            candidates,
            minimum_version: minimum_version.into(),
            expected_protocol: expected_protocol.into(),
            handshake,
            failure_recorder,
        }
    }

    pub async fn resolve(&self) -> Result<CodexRuntime, RuntimeError> {
        let minimum = Version::parse(&self.minimum_version)?;
        let mut candidates: Vec<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.version >= minimum)
            .cloned()
            .collect();
        candidates.sort_by_key(|candidate| source_priority(candidate.source));

        for candidate in candidates {
            let result = self.handshake.initialize(&candidate).await;
            let error = match result {
                Ok(handshake) if handshake.protocol_version == self.expected_protocol => {
                    return Ok(candidate.into());
                }
                Ok(_) => RuntimeError::ProtocolIncompatible,
                Err(error) => error,
            };
            self.handshake.stop(&candidate).await;
            self.failure_recorder.record(RuntimeFailureRecord {
                source: candidate.source,
                version: candidate.version.to_string(),
                error_code: error.code(),
            });
        }

        Err(RuntimeError::NoCompatibleRuntime)
    }
}

pub fn choose_runtime(
    candidates: Vec<RuntimeCandidate>,
    minimum: &str,
) -> Result<RuntimeCandidate, RuntimeError> {
    let minimum = Version::parse(minimum)?;
    candidates
        .into_iter()
        .filter(|candidate| candidate.version >= minimum)
        .min_by_key(|candidate| source_priority(candidate.source))
        .ok_or(RuntimeError::NoCompatibleRuntime)
}

pub async fn download_verified(
    downloader: &(dyn RuntimeDownloader + Send + Sync),
    url: &Url,
    expected_hex: &str,
) -> Result<Vec<u8>, RuntimeError> {
    let bytes = downloader.download(url).await?;
    verify_sha256(&bytes, expected_hex)?;
    Ok(bytes)
}

pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), RuntimeError> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    (actual == expected_hex)
        .then_some(())
        .ok_or(RuntimeError::ChecksumMismatch)
}

fn source_priority(source: RuntimeSource) -> u8 {
    match source {
        RuntimeSource::System => 0,
        RuntimeSource::AppLocal => 1,
        RuntimeSource::Bundled => 2,
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    #[error("runtime version is invalid")]
    InvalidVersion,
    #[error("no compatible Codex runtime is available")]
    NoCompatibleRuntime,
    #[error("the Codex App Server protocol is incompatible")]
    ProtocolIncompatible,
    #[error("the downloaded Codex runtime checksum does not match")]
    ChecksumMismatch,
    #[error("the Codex runtime download failed")]
    DownloadFailed,
}

impl RuntimeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidVersion => "invalid_version",
            Self::NoCompatibleRuntime => "no_compatible_runtime",
            Self::ProtocolIncompatible => "protocol_incompatible",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::DownloadFailed => "download_failed",
        }
    }
}

impl From<semver::Error> for RuntimeError {
    fn from(_: semver::Error) -> Self {
        Self::InvalidVersion
    }
}
