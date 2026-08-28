use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use semver::Version;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::codex::install::AppLocalRuntimeInstaller;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum RuntimeSource {
    System,
    AppLocal,
    Bundled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCandidate {
    path: PathBuf,
    version: Version,
    source: RuntimeSource,
}

impl RuntimeCandidate {
    pub fn system(path: impl Into<PathBuf>, version: Version) -> Self {
        Self {
            path: path.into(),
            version,
            source: RuntimeSource::System,
        }
    }

    pub(crate) fn app_local(path: impl Into<PathBuf>, version: Version) -> Self {
        Self {
            path: path.into(),
            version,
            source: RuntimeSource::AppLocal,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn source(&self) -> RuntimeSource {
        self.source
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

pub struct LiveRuntimeChannel {
    reader: Box<dyn AsyncRead + Send + Unpin>,
    writer: Box<dyn AsyncWrite + Send + Unpin>,
}

impl LiveRuntimeChannel {
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Box<dyn AsyncRead + Send + Unpin>,
        Box<dyn AsyncWrite + Send + Unpin>,
    ) {
        (self.reader, self.writer)
    }
}

#[async_trait]
pub trait LiveRuntimeSession: Send {
    fn session_id(&self) -> &str;
    async fn initialize(&mut self) -> Result<String, RuntimeError>;
    async fn stop(&mut self) -> Result<(), RuntimeError>;

    /// Immediately prevents the runtime from outliving its owner.
    /// Implementations must be idempotent, synchronous, and non-blocking.
    fn abort(&mut self);

    fn take_transport_channel(&mut self) -> Result<LiveRuntimeChannel, RuntimeError> {
        Err(RuntimeError::TransportUnavailable)
    }
}

pub struct OwnedLiveRuntimeSession {
    session: Option<Box<dyn LiveRuntimeSession>>,
}

impl OwnedLiveRuntimeSession {
    fn new(session: Box<dyn LiveRuntimeSession>) -> Self {
        Self {
            session: Some(session),
        }
    }

    fn session_id(&self) -> &str {
        self.session
            .as_ref()
            .expect("owned live runtime session is present")
            .session_id()
    }

    fn take_transport_channel(&mut self) -> Result<LiveRuntimeChannel, RuntimeError> {
        self.session
            .as_mut()
            .ok_or(RuntimeError::TransportUnavailable)?
            .take_transport_channel()
    }

    pub async fn stop(&mut self) -> Result<(), RuntimeError> {
        let result = match self.session.as_mut() {
            Some(session) => session.stop().await,
            None => return Ok(()),
        };
        if result.is_err() {
            if let Some(session) = self.session.as_mut() {
                session.abort();
            }
        }
        self.session.take();
        result
    }

    pub fn abort(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.abort();
        }
    }
}

impl Drop for OwnedLiveRuntimeSession {
    fn drop(&mut self) {
        self.abort();
    }
}

#[async_trait]
pub trait RuntimeLauncher: Send + Sync {
    async fn start(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError>;
}

pub struct ResolvedRuntime {
    source: RuntimeSource,
    version: Version,
    session: OwnedLiveRuntimeSession,
}

impl ResolvedRuntime {
    pub fn source(&self) -> RuntimeSource {
        self.source
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn session_id(&self) -> &str {
        self.session.session_id()
    }

    pub fn into_live_transport(
        mut self,
    ) -> Result<(OwnedLiveRuntimeSession, LiveRuntimeChannel), RuntimeError> {
        let channel = self.session.take_transport_channel()?;
        Ok((self.session, channel))
    }
}

pub struct RuntimeResolver {
    system_candidates: Vec<RuntimeCandidate>,
    app_local: Option<Arc<dyn AppLocalCandidateProvider>>,
    minimum_version: String,
    expected_protocol: String,
    launcher: Arc<dyn RuntimeLauncher>,
    failure_recorder: Arc<dyn RuntimeFailureRecorder>,
}

impl RuntimeResolver {
    pub fn system_only(
        system_candidates: Vec<RuntimeCandidate>,
        minimum_version: impl Into<String>,
        expected_protocol: impl Into<String>,
        launcher: Arc<dyn RuntimeLauncher>,
        failure_recorder: Arc<dyn RuntimeFailureRecorder>,
    ) -> Self {
        Self {
            system_candidates,
            app_local: None,
            minimum_version: minimum_version.into(),
            expected_protocol: expected_protocol.into(),
            launcher,
            failure_recorder,
        }
    }

    pub fn with_verified_installer(
        discovery: &dyn SystemRuntimeDiscovery,
        installer: AppLocalRuntimeInstaller,
        minimum_version: impl Into<String>,
        expected_protocol: impl Into<String>,
        launcher: Arc<dyn RuntimeLauncher>,
        failure_recorder: Arc<dyn RuntimeFailureRecorder>,
    ) -> Result<Self, RuntimeError> {
        Ok(Self {
            system_candidates: discovery.discover()?,
            app_local: Some(Arc::new(installer)),
            minimum_version: minimum_version.into(),
            expected_protocol: expected_protocol.into(),
            launcher,
            failure_recorder,
        })
    }

    #[cfg(test)]
    fn with_provider_for_test(
        system_candidates: Vec<RuntimeCandidate>,
        provider: Arc<dyn AppLocalCandidateProvider>,
        minimum_version: impl Into<String>,
        expected_protocol: impl Into<String>,
        launcher: Arc<dyn RuntimeLauncher>,
        failure_recorder: Arc<dyn RuntimeFailureRecorder>,
    ) -> Self {
        Self {
            system_candidates,
            app_local: Some(provider),
            minimum_version: minimum_version.into(),
            expected_protocol: expected_protocol.into(),
            launcher,
            failure_recorder,
        }
    }

    pub async fn resolve(&self) -> Result<ResolvedRuntime, RuntimeError> {
        let minimum = Version::parse(&self.minimum_version)?;
        let mut candidates: Vec<_> = self
            .system_candidates
            .iter()
            .filter(|candidate| candidate.version >= minimum)
            .cloned()
            .collect();
        candidates.sort_by_key(|candidate| source_priority(candidate.source));

        for candidate in candidates {
            if let Some(resolved) = self.try_candidate(candidate).await? {
                return Ok(resolved);
            }
        }

        if let Some(provider) = &self.app_local {
            let candidate = provider.install_verified().await?;
            if candidate.version >= minimum {
                if let Some(resolved) = self.try_candidate(candidate).await? {
                    return Ok(resolved);
                }
            }
        }

        Err(RuntimeError::NoCompatibleRuntime)
    }

    async fn try_candidate(
        &self,
        candidate: RuntimeCandidate,
    ) -> Result<Option<ResolvedRuntime>, RuntimeError> {
        let mut session = self.launcher.start(&candidate).await?;
        let result = session.initialize().await;
        if matches!(&result, Ok(protocol) if protocol == &self.expected_protocol) {
            return Ok(Some(ResolvedRuntime {
                source: candidate.source,
                version: candidate.version,
                session: OwnedLiveRuntimeSession::new(session),
            }));
        }

        let failure = result.err().unwrap_or(RuntimeError::ProtocolIncompatible);
        if session.stop().await.is_err() {
            session.abort();
            self.failure_recorder.record(RuntimeFailureRecord {
                source: candidate.source,
                version: candidate.version.to_string(),
                error_code: RuntimeError::StopFailed.code(),
            });
            return Err(RuntimeError::StopFailed);
        }
        self.failure_recorder.record(RuntimeFailureRecord {
            source: candidate.source,
            version: candidate.version.to_string(),
            error_code: failure.code(),
        });
        Ok(None)
    }
}

#[async_trait]
pub(crate) trait AppLocalCandidateProvider: Send + Sync {
    async fn install_verified(&self) -> Result<RuntimeCandidate, RuntimeError>;
}

#[async_trait]
impl AppLocalCandidateProvider for AppLocalRuntimeInstaller {
    async fn install_verified(&self) -> Result<RuntimeCandidate, RuntimeError> {
        self.install().await
    }
}

pub trait RuntimeVersionProbe: Send + Sync {
    fn version(&self, path: &Path) -> Result<Version, RuntimeError>;
}

pub trait SystemRuntimeDiscovery: Send + Sync {
    fn discover(&self) -> Result<Vec<RuntimeCandidate>, RuntimeError>;
}

pub struct OfficialSystemDiscovery {
    path: Option<OsString>,
    probe: Arc<dyn RuntimeVersionProbe>,
}

impl OfficialSystemDiscovery {
    pub fn new() -> Self {
        Self {
            path: None,
            probe: Arc::new(CommandVersionProbe),
        }
    }

    #[cfg(test)]
    fn with_path_for_test(path: OsString, probe: Arc<dyn RuntimeVersionProbe>) -> Self {
        Self {
            path: Some(path),
            probe,
        }
    }
}

impl Default for OfficialSystemDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemRuntimeDiscovery for OfficialSystemDiscovery {
    fn discover(&self) -> Result<Vec<RuntimeCandidate>, RuntimeError> {
        let path = self.path.clone().or_else(|| env::var_os("PATH"));
        let Some(path) = path else {
            return Ok(Vec::new());
        };
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for directory in env::split_paths(&path) {
            let executable = directory.join(executable_name);
            if !executable.is_file() || !seen.insert(executable.clone()) {
                continue;
            }
            if let Ok(version) = self.probe.version(&executable) {
                candidates.push(RuntimeCandidate::system(executable, version));
            }
        }
        Ok(candidates)
    }
}

struct CommandVersionProbe;

impl RuntimeVersionProbe for CommandVersionProbe {
    fn version(&self, path: &Path) -> Result<Version, RuntimeError> {
        let output = Command::new(path)
            .arg("--version")
            .output()
            .map_err(|_| RuntimeError::VersionProbeFailed)?;
        if !output.status.success() {
            return Err(RuntimeError::VersionProbeFailed);
        }
        let stdout =
            std::str::from_utf8(&output.stdout).map_err(|_| RuntimeError::VersionProbeFailed)?;
        stdout
            .split_whitespace()
            .find_map(|token| Version::parse(token).ok())
            .ok_or(RuntimeError::VersionProbeFailed)
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
    #[error("the Codex runtime could not be stopped")]
    StopFailed,
    #[error("the downloaded Codex runtime checksum does not match")]
    ChecksumMismatch,
    #[error("the Codex runtime download failed")]
    DownloadFailed,
    #[error("the embedded Codex runtime manifest is invalid")]
    ManifestInvalid,
    #[error("the Codex runtime response length is invalid")]
    ContentLengthInvalid,
    #[error("the Codex runtime download exceeded its bound")]
    DownloadTooLarge,
    #[error("the Codex runtime archive is unsupported")]
    ArchiveUnsupported,
    #[error("the Codex runtime archive entry does not match")]
    ArchiveEntryMismatch,
    #[error("the Codex runtime archive expansion exceeded its bound")]
    ArchiveExpansionTooLarge,
    #[error("the Codex runtime filesystem operation failed")]
    FilesystemFailed,
    #[error("the installed Codex runtime conflicts with verified bytes")]
    InstallConflict,
    #[error("the system Codex version probe failed")]
    VersionProbeFailed,
    #[error("the initialized Codex runtime transport is unavailable")]
    TransportUnavailable,
}

impl RuntimeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidVersion => "invalid_version",
            Self::NoCompatibleRuntime => "no_compatible_runtime",
            Self::ProtocolIncompatible => "protocol_incompatible",
            Self::StopFailed => "runtime_stop_failed",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::DownloadFailed => "download_failed",
            Self::ManifestInvalid => "manifest_invalid",
            Self::ContentLengthInvalid => "content_length_invalid",
            Self::DownloadTooLarge => "download_too_large",
            Self::ArchiveUnsupported => "archive_unsupported",
            Self::ArchiveEntryMismatch => "archive_entry_mismatch",
            Self::ArchiveExpansionTooLarge => "archive_expansion_too_large",
            Self::FilesystemFailed => "filesystem_failed",
            Self::InstallConflict => "install_conflict",
            Self::VersionProbeFailed => "version_probe_failed",
            Self::TransportUnavailable => "runtime_transport_unavailable",
        }
    }
}

impl From<semver::Error> for RuntimeError {
    fn from(_: semver::Error) -> Self {
        Self::InvalidVersion
    }
}

#[cfg(test)]
mod security_tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use semver::Version;
    use tempfile::tempdir;

    use super::{
        AppLocalCandidateProvider, LiveRuntimeSession, OfficialSystemDiscovery, RuntimeCandidate,
        RuntimeError, RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeLauncher,
        RuntimeResolver, RuntimeSource, RuntimeVersionProbe, SystemRuntimeDiscovery,
    };

    #[test]
    fn official_system_discovery_derives_candidates_from_path_and_a_version_probe() {
        let directory = tempdir().unwrap();
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = directory.path().join(executable_name);
        std::fs::write(&executable, b"fixture, never executed").unwrap();
        let probe = Arc::new(FakeVersionProbe::default());
        let discovery = OfficialSystemDiscovery::with_path_for_test(
            OsString::from(directory.path()),
            probe.clone(),
        );

        let candidates = discovery.discover().unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path(), executable.as_path());
        assert_eq!(candidates[0].version(), &Version::parse("0.145.0").unwrap());
        assert_eq!(probe.paths.lock().unwrap().as_slice(), &[executable]);
    }

    #[derive(Default)]
    struct FakeVersionProbe {
        paths: Mutex<Vec<PathBuf>>,
    }

    impl RuntimeVersionProbe for FakeVersionProbe {
        fn version(&self, path: &Path) -> Result<Version, RuntimeError> {
            self.paths.lock().unwrap().push(path.to_owned());
            Ok(Version::parse("0.145.0").unwrap())
        }
    }

    #[tokio::test]
    async fn verified_app_local_provider_is_used_only_after_system_handshake_failure() {
        let installed = Arc::new(Mutex::new(0));
        let provider = Arc::new(FakeAppLocalProvider {
            installed: installed.clone(),
        });
        let resolver = RuntimeResolver::with_provider_for_test(
            vec![RuntimeCandidate::system(
                "system-codex",
                Version::parse("0.145.0").unwrap(),
            )],
            provider,
            "0.144.0",
            "pinned",
            Arc::new(SourceLauncher),
            Arc::new(NoopRecorder),
        );

        let resolved = resolver.resolve().await.unwrap();

        assert_eq!(resolved.source(), RuntimeSource::AppLocal);
        assert_eq!(*installed.lock().unwrap(), 1);
        assert_eq!(resolved.session_id(), "app-local-session");
    }

    #[tokio::test]
    async fn compatible_system_session_prevents_app_local_install() {
        let installed = Arc::new(Mutex::new(0));
        let resolver = RuntimeResolver::with_provider_for_test(
            vec![RuntimeCandidate::system(
                "system-codex",
                Version::parse("0.145.0").unwrap(),
            )],
            Arc::new(FakeAppLocalProvider {
                installed: installed.clone(),
            }),
            "0.144.0",
            "different",
            Arc::new(SourceLauncher),
            Arc::new(NoopRecorder),
        );

        let resolved = resolver.resolve().await.unwrap();

        assert_eq!(resolved.source(), RuntimeSource::System);
        assert_eq!(*installed.lock().unwrap(), 0);
        assert_eq!(resolved.session_id(), "system-session");
    }

    struct FakeAppLocalProvider {
        installed: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl AppLocalCandidateProvider for FakeAppLocalProvider {
        async fn install_verified(&self) -> Result<RuntimeCandidate, RuntimeError> {
            *self.installed.lock().unwrap() += 1;
            Ok(RuntimeCandidate::app_local(
                "app-local-codex",
                Version::parse("0.144.4").unwrap(),
            ))
        }
    }

    struct SourceLauncher;

    #[async_trait]
    impl RuntimeLauncher for SourceLauncher {
        async fn start(
            &self,
            candidate: &RuntimeCandidate,
        ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError> {
            Ok(Box::new(SourceSession {
                source: candidate.source(),
            }))
        }
    }

    struct SourceSession {
        source: RuntimeSource,
    }

    #[async_trait]
    impl LiveRuntimeSession for SourceSession {
        fn session_id(&self) -> &str {
            match self.source {
                RuntimeSource::System => "system-session",
                RuntimeSource::AppLocal => "app-local-session",
                RuntimeSource::Bundled => "bundled-session",
            }
        }

        async fn initialize(&mut self) -> Result<String, RuntimeError> {
            Ok(match self.source {
                RuntimeSource::System => "different",
                RuntimeSource::AppLocal => "pinned",
                RuntimeSource::Bundled => "different",
            }
            .to_owned())
        }

        async fn stop(&mut self) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn abort(&mut self) {}
    }

    struct NoopRecorder;

    impl RuntimeFailureRecorder for NoopRecorder {
        fn record(&self, _record: RuntimeFailureRecord) {}
    }
}
