use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use url::Url;

use crate::codex::manifest::{
    EmbeddedRuntimeManifest, HostTarget, PinnedRuntime, MAX_DOWNLOAD_BYTES,
};
use crate::codex::runtime::{RuntimeCandidate, RuntimeError};

const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const OFFICIAL_RELEASE_PATH_PREFIX: &str = "/openai/codex/releases/download/rust-v0.144.4/";

pub struct DownloadResponse {
    pub content_length: Option<u64>,
    pub stream: Box<dyn RuntimeByteStream>,
}

#[async_trait]
pub trait RuntimeByteStream: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, RuntimeError>;
}

#[async_trait]
pub trait RuntimeDownloader: Send + Sync {
    async fn open(&self, url: &Url, expected_size: u64) -> Result<DownloadResponse, RuntimeError>;
}

pub struct ProductionRuntimeDownloader {
    client: reqwest::Client,
}

impl ProductionRuntimeDownloader {
    pub fn new() -> Result<Self, RuntimeError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| RuntimeError::DownloadFailed)?;
        Ok(Self { client })
    }
}

#[async_trait]
impl RuntimeDownloader for ProductionRuntimeDownloader {
    async fn open(&self, url: &Url, expected_size: u64) -> Result<DownloadResponse, RuntimeError> {
        if expected_size == 0 || expected_size > MAX_DOWNLOAD_BYTES {
            return Err(RuntimeError::ContentLengthInvalid);
        }
        let redirect = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| RuntimeError::DownloadFailed)?;
        if !redirect.status().is_redirection() {
            return Err(RuntimeError::DownloadFailed);
        }
        let location = redirect
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(RuntimeError::DownloadFailed)?;
        let destination = validate_release_redirect(url, location)?;
        drop(redirect);

        let response = self
            .client
            .get(destination.clone())
            .send()
            .await
            .map_err(|_| RuntimeError::DownloadFailed)?;
        if !response.status().is_success() || response.url() != &destination {
            return Err(RuntimeError::DownloadFailed);
        }
        Ok(DownloadResponse {
            content_length: response.content_length(),
            stream: Box::new(ReqwestByteStream { response }),
        })
    }
}

struct ReqwestByteStream {
    response: reqwest::Response,
}

#[async_trait]
impl RuntimeByteStream for ReqwestByteStream {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, RuntimeError> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|_| RuntimeError::DownloadFailed)
    }
}

pub(crate) fn validate_release_redirect(source: &Url, location: &str) -> Result<Url, RuntimeError> {
    let source_asset = source
        .path()
        .strip_prefix(OFFICIAL_RELEASE_PATH_PREFIX)
        .filter(|asset| !asset.is_empty() && !asset.contains('/'));
    if source.scheme() != "https"
        || source.host_str() != Some("github.com")
        || !source.username().is_empty()
        || source.password().is_some()
        || source.port().is_some()
        || source.query().is_some()
        || source.fragment().is_some()
        || source_asset.is_none()
    {
        return Err(RuntimeError::DownloadFailed);
    }

    let destination = Url::parse(location).map_err(|_| RuntimeError::DownloadFailed)?;
    if destination.scheme() != "https"
        || destination.host_str() != Some("release-assets.githubusercontent.com")
        || !destination.username().is_empty()
        || destination.password().is_some()
        || destination.port().is_some()
        || destination.fragment().is_some()
    {
        return Err(RuntimeError::DownloadFailed);
    }
    Ok(destination)
}

pub struct AppLocalRuntimeInstaller {
    root: PathBuf,
    downloader: Arc<dyn RuntimeDownloader>,
    pinned: PinnedRuntime,
}

impl AppLocalRuntimeInstaller {
    pub fn new(
        root: PathBuf,
        downloader: Arc<dyn RuntimeDownloader>,
    ) -> Result<Self, RuntimeError> {
        Ok(Self {
            root,
            downloader,
            pinned: EmbeddedRuntimeManifest::load_for_host(HostTarget::current())?,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_pinned_for_test(
        root: PathBuf,
        downloader: Arc<dyn RuntimeDownloader>,
        pinned: PinnedRuntime,
    ) -> Self {
        Self {
            root,
            downloader,
            pinned,
        }
    }

    pub async fn install(&self) -> Result<RuntimeCandidate, RuntimeError> {
        let install_dir = self
            .root
            .join("codex")
            .join(self.pinned.version().to_string())
            .join(self.pinned.target());
        fs::create_dir_all(&install_dir).map_err(|_| RuntimeError::FilesystemFailed)?;
        require_owned_directory(&self.root, &install_dir)?;

        let mut archive =
            NamedTempFile::new_in(&install_dir).map_err(|_| RuntimeError::FilesystemFailed)?;
        self.download_archive(&mut archive).await?;
        archive
            .as_file_mut()
            .seek(io::SeekFrom::Start(0))
            .map_err(|_| RuntimeError::FilesystemFailed)?;

        let mut executable =
            NamedTempFile::new_in(&install_dir).map_err(|_| RuntimeError::FilesystemFailed)?;
        extract_main(
            archive.as_file_mut(),
            &self.pinned,
            executable.as_file_mut(),
        )?;
        executable
            .as_file_mut()
            .sync_all()
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        set_executable_permissions(executable.path())?;

        let final_name = if self.pinned.target().contains("windows") {
            "codex.exe"
        } else {
            "codex"
        };
        let final_path = install_dir.join(final_name);
        match executable.persist_noclobber(&final_path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                if !files_equal(error.file.path(), &final_path)? {
                    return Err(RuntimeError::InstallConflict);
                }
                set_executable_permissions(&final_path)?;
            }
            Err(_) => return Err(RuntimeError::FilesystemFailed),
        }
        require_regular_installed_file(&install_dir, &final_path)?;

        Ok(RuntimeCandidate::app_local(
            final_path,
            self.pinned.version().clone(),
        ))
    }

    async fn download_archive(&self, destination: &mut NamedTempFile) -> Result<(), RuntimeError> {
        let mut response = self
            .downloader
            .open(self.pinned.url(), self.pinned.size())
            .await?;
        if response.content_length != Some(self.pinned.size())
            || self.pinned.size() > MAX_DOWNLOAD_BYTES
        {
            return Err(RuntimeError::ContentLengthInvalid);
        }

        let mut received = 0_u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = response.stream.next_chunk().await? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(RuntimeError::DownloadTooLarge)?;
            if received > self.pinned.size() || received > MAX_DOWNLOAD_BYTES {
                return Err(RuntimeError::DownloadTooLarge);
            }
            digest.update(&chunk);
            destination
                .write_all(&chunk)
                .map_err(|_| RuntimeError::FilesystemFailed)?;
        }
        if received != self.pinned.size() {
            return Err(RuntimeError::ContentLengthInvalid);
        }
        if format!("{:x}", digest.finalize()) != self.pinned.sha256() {
            return Err(RuntimeError::ChecksumMismatch);
        }
        destination
            .as_file_mut()
            .sync_all()
            .map_err(|_| RuntimeError::FilesystemFailed)
    }
}

fn require_regular_installed_file(
    install_dir: &Path,
    final_path: &Path,
) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(final_path).map_err(|_| RuntimeError::FilesystemFailed)?;
    let install_dir = fs::canonicalize(install_dir).map_err(|_| RuntimeError::FilesystemFailed)?;
    let final_path = fs::canonicalize(final_path).map_err(|_| RuntimeError::FilesystemFailed)?;
    if metadata.file_type().is_file() && final_path.starts_with(install_dir) {
        Ok(())
    } else {
        Err(RuntimeError::InstallConflict)
    }
}

fn require_owned_directory(root: &Path, install_dir: &Path) -> Result<(), RuntimeError> {
    let root = fs::canonicalize(root).map_err(|_| RuntimeError::FilesystemFailed)?;
    let install_dir = fs::canonicalize(install_dir).map_err(|_| RuntimeError::FilesystemFailed)?;
    if install_dir.starts_with(root) {
        Ok(())
    } else {
        Err(RuntimeError::FilesystemFailed)
    }
}

fn extract_main(
    archive: &mut File,
    pinned: &PinnedRuntime,
    output: &mut File,
) -> Result<(), RuntimeError> {
    if pinned.url().path().ends_with(".zip") {
        extract_zip(archive, pinned, output)
    } else if pinned.url().path().ends_with(".tar.gz") {
        extract_tar_gz(archive, pinned, output)
    } else {
        Err(RuntimeError::ArchiveUnsupported)
    }
}

fn extract_zip(
    archive: &mut File,
    pinned: &PinnedRuntime,
    output: &mut File,
) -> Result<(), RuntimeError> {
    let mut zip = zip::ZipArchive::new(archive).map_err(|_| RuntimeError::ArchiveUnsupported)?;
    let expected_names: &[&str] = if pinned.target() == "x86_64-pc-windows-msvc" {
        &[
            "codex-command-runner.exe",
            "codex-windows-sandbox-setup.exe",
            pinned.archive_entry(),
        ]
    } else {
        &[pinned.archive_entry()]
    };
    let main_entry_count = (0..zip.len())
        .filter(|index| {
            zip.by_index(*index)
                .is_ok_and(|entry| entry.name() == pinned.archive_entry())
        })
        .count();
    if main_entry_count != 1 {
        return Err(RuntimeError::ArchiveEntryMismatch);
    }
    if zip.len() != expected_names.len() {
        return Err(RuntimeError::ArchiveUnsupported);
    }
    for expected in expected_names {
        let count = (0..zip.len())
            .filter(|index| {
                zip.by_index(*index)
                    .is_ok_and(|entry| entry.name() == *expected)
            })
            .count();
        if count != 1 {
            return Err(RuntimeError::ArchiveEntryMismatch);
        }
    }

    let mut entry = zip
        .by_name(pinned.archive_entry())
        .map_err(|_| RuntimeError::ArchiveEntryMismatch)?;
    if !entry.is_file()
        || entry.enclosed_name().is_none()
        || entry.size() > MAX_EXPANDED_BYTES
        || entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err(RuntimeError::ArchiveUnsupported);
    }
    copy_bounded(&mut entry, output, MAX_EXPANDED_BYTES)
}

fn extract_tar_gz(
    archive: &mut File,
    pinned: &PinnedRuntime,
    output: &mut File,
) -> Result<(), RuntimeError> {
    let decoder = GzDecoder::new(BufReader::new(archive));
    let mut tar = tar::Archive::new(decoder);
    let mut found = false;
    let mut expanded = 0_u64;
    for entry in tar
        .entries()
        .map_err(|_| RuntimeError::ArchiveUnsupported)?
    {
        let mut entry = entry.map_err(|_| RuntimeError::ArchiveUnsupported)?;
        let size = entry
            .header()
            .size()
            .map_err(|_| RuntimeError::ArchiveUnsupported)?;
        expanded = expanded
            .checked_add(size)
            .ok_or(RuntimeError::ArchiveExpansionTooLarge)?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(RuntimeError::ArchiveExpansionTooLarge);
        }
        let path = entry.path().map_err(|_| RuntimeError::ArchiveUnsupported)?;
        if path == Path::new(pinned.archive_entry()) {
            if found || !entry.header().entry_type().is_file() {
                return Err(RuntimeError::ArchiveEntryMismatch);
            }
            copy_bounded(&mut entry, output, MAX_EXPANDED_BYTES)?;
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err(RuntimeError::ArchiveEntryMismatch)
    }
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    maximum: u64,
) -> Result<(), RuntimeError> {
    let copied = io::copy(&mut reader.take(maximum + 1), writer)
        .map_err(|_| RuntimeError::ArchiveUnsupported)?;
    if copied > maximum {
        Err(RuntimeError::ArchiveExpansionTooLarge)
    } else {
        Ok(())
    }
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, RuntimeError> {
    if fs::symlink_metadata(right)
        .map_err(|_| RuntimeError::FilesystemFailed)?
        .file_type()
        .is_symlink()
    {
        return Ok(false);
    }
    let mut left = BufReader::new(File::open(left).map_err(|_| RuntimeError::FilesystemFailed)?);
    let mut right = BufReader::new(File::open(right).map_err(|_| RuntimeError::FilesystemFailed)?);
    let mut left_chunk = [0_u8; 64 * 1024];
    let mut right_chunk = [0_u8; 64 * 1024];
    loop {
        let left_read = left
            .read(&mut left_chunk)
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        let right_read = right
            .read(&mut right_chunk)
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        if left_read != right_read || left_chunk[..left_read] != right_chunk[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<(), RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| RuntimeError::FilesystemFailed)
}

#[cfg(windows)]
fn set_executable_permissions(_path: &Path) -> Result<(), RuntimeError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use semver::Version;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;
    use url::Url;
    use zip::write::SimpleFileOptions;

    use super::{
        validate_release_redirect, AppLocalRuntimeInstaller, DownloadResponse,
        ProductionRuntimeDownloader, RuntimeByteStream, RuntimeDownloader,
    };
    use crate::codex::manifest::{EmbeddedRuntimeManifest, HostTarget, PinnedRuntime};
    use crate::codex::runtime::{RuntimeError, RuntimeSource};

    #[tokio::test]
    async fn verified_installer_streams_to_a_temp_file_and_exposes_only_the_main_executable() {
        let archive = zip_archive(&[
            ("codex-command-runner.exe", b"helper"),
            ("codex-windows-sandbox-setup.exe", b"setup"),
            ("codex.exe", b"verified executable"),
        ]);
        let pinned = pinned_for(&archive, "codex.exe");
        let root = tempdir().unwrap();
        let downloader = Arc::new(FakeDownloader::new(archive, 5));
        let installer = AppLocalRuntimeInstaller::with_pinned_for_test(
            root.path().to_owned(),
            downloader.clone(),
            pinned,
        );

        let candidate = installer.install().await.unwrap();

        assert_eq!(candidate.source(), RuntimeSource::AppLocal);
        assert_eq!(
            std::fs::read(candidate.path()).unwrap(),
            b"verified executable"
        );
        assert!(!root.path().join("codex-command-runner.exe").exists());
        assert!(downloader.chunks_read() > 1);
        let exposed_files: Vec<_> = candidate
            .path()
            .parent()
            .unwrap()
            .read_dir()
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(exposed_files, [std::ffi::OsString::from("codex.exe")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(candidate.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[tokio::test]
    async fn installer_rejects_checksum_or_entry_mismatch_without_exposing_a_runtime() {
        let archive = zip_archive(&[("different.exe", b"content")]);
        let root = tempdir().unwrap();

        let wrong_hash = PinnedRuntime::for_test(
            Version::parse("0.144.4").unwrap(),
            "x86_64-pc-windows-msvc",
            Url::parse("https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-x86_64-pc-windows-msvc.exe.zip").unwrap(),
            "0".repeat(64),
            archive.len() as u64,
            "codex.exe",
        );
        let installer = AppLocalRuntimeInstaller::with_pinned_for_test(
            root.path().to_owned(),
            Arc::new(FakeDownloader::new(archive.clone(), archive.len())),
            wrong_hash,
        );
        assert!(matches!(
            installer.install().await,
            Err(RuntimeError::ChecksumMismatch)
        ));

        let wrong_entry = pinned_for(&archive, "codex.exe");
        let installer = AppLocalRuntimeInstaller::with_pinned_for_test(
            root.path().to_owned(),
            Arc::new(FakeDownloader::new(archive.clone(), archive.len())),
            wrong_entry,
        );
        assert!(matches!(
            installer.install().await,
            Err(RuntimeError::ArchiveEntryMismatch)
        ));
        assert!(root.path().read_dir().unwrap().all(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            !name.ends_with(".exe")
        }));
    }

    #[tokio::test]
    async fn production_installer_derives_the_host_asset_from_the_embedded_manifest() {
        let root = tempdir().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let downloader = Arc::new(RecordingRejectingDownloader {
            requests: requests.clone(),
        });
        let installer = AppLocalRuntimeInstaller::new(root.path().to_owned(), downloader).unwrap();

        assert!(matches!(
            installer.install().await,
            Err(RuntimeError::ContentLengthInvalid)
        ));
        let requests = requests.lock().unwrap();
        let expected = EmbeddedRuntimeManifest::load_for_host(HostTarget::current()).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(&requests[0].0, expected.url());
        assert_eq!(requests[0].1, expected.size());
    }

    #[test]
    fn production_download_policy_requires_the_exact_official_release_and_single_cdn_handoff() {
        let official = Url::parse(
            "https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-x86_64-pc-windows-msvc.exe.zip",
        )
        .unwrap();
        let cdn = "https://release-assets.githubusercontent.com/github-production-release-asset/file?sig=opaque";

        assert_eq!(
            validate_release_redirect(&official, cdn)
                .unwrap()
                .host_str(),
            Some("release-assets.githubusercontent.com")
        );

        for (source, location) in [
            (
                "https://github.com.evil.example/openai/codex/releases/download/rust-v0.144.4/file.zip",
                cdn,
            ),
            (
                official.as_str(),
                "https://release-assets.githubusercontent.com.evil.example/file",
            ),
            (official.as_str(), "http://release-assets.githubusercontent.com/file"),
            (
                official.as_str(),
                "https://user@release-assets.githubusercontent.com/file",
            ),
        ] {
            let source = Url::parse(source).unwrap();
            assert!(validate_release_redirect(&source, location).is_err());
        }
    }

    #[test]
    fn production_https_downloader_can_be_constructed_for_the_verified_installer() {
        let downloader: Arc<dyn RuntimeDownloader> =
            Arc::new(ProductionRuntimeDownloader::new().unwrap());
        drop(downloader);
    }

    #[tokio::test]
    async fn installer_aborts_on_the_first_chunk_that_exceeds_the_manifest_size() {
        let archive = zip_archive(&[
            ("codex-command-runner.exe", b"helper"),
            ("codex-windows-sandbox-setup.exe", b"setup"),
            ("codex.exe", b"verified executable"),
        ]);
        let pinned = pinned_for(&archive, "codex.exe");
        let mut oversized = archive.clone();
        oversized.push(0);
        let downloader = Arc::new(FakeDownloader::with_declared_size(
            oversized.clone(),
            oversized.len(),
            archive.len() as u64,
        ));
        let root = tempdir().unwrap();
        let installer = AppLocalRuntimeInstaller::with_pinned_for_test(
            root.path().to_owned(),
            downloader.clone(),
            pinned,
        );

        assert!(matches!(
            installer.install().await,
            Err(RuntimeError::DownloadTooLarge)
        ));
        assert_eq!(downloader.chunks_read(), 1);
    }

    fn pinned_for(bytes: &[u8], entry: &str) -> PinnedRuntime {
        PinnedRuntime::for_test(
            Version::parse("0.144.4").unwrap(),
            "x86_64-pc-windows-msvc",
            Url::parse("https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-x86_64-pc-windows-msvc.exe.zip").unwrap(),
            format!("{:x}", Sha256::digest(bytes)),
            bytes.len() as u64,
            entry,
        )
    }

    fn zip_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut output);
            for (name, bytes) in entries {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        output.into_inner()
    }

    struct FakeDownloader {
        bytes: Vec<u8>,
        chunk_size: usize,
        content_length: u64,
        chunks: Arc<Mutex<usize>>,
    }

    impl FakeDownloader {
        fn new(bytes: Vec<u8>, chunk_size: usize) -> Self {
            let content_length = bytes.len() as u64;
            Self::with_declared_size(bytes, chunk_size, content_length)
        }

        fn with_declared_size(bytes: Vec<u8>, chunk_size: usize, content_length: u64) -> Self {
            Self {
                bytes,
                chunk_size,
                content_length,
                chunks: Arc::new(Mutex::new(0)),
            }
        }

        fn chunks_read(&self) -> usize {
            *self.chunks.lock().unwrap()
        }
    }

    #[async_trait]
    impl RuntimeDownloader for FakeDownloader {
        async fn open(
            &self,
            _url: &Url,
            _expected_size: u64,
        ) -> Result<DownloadResponse, RuntimeError> {
            Ok(DownloadResponse {
                content_length: Some(self.content_length),
                stream: Box::new(FakeStream {
                    chunks: self
                        .bytes
                        .chunks(self.chunk_size)
                        .map(|chunk| chunk.to_vec())
                        .collect(),
                    index: 0,
                    reads: self.chunks.clone(),
                }),
            })
        }
    }

    struct FakeStream {
        chunks: Vec<Vec<u8>>,
        index: usize,
        reads: Arc<Mutex<usize>>,
    }

    struct RecordingRejectingDownloader {
        requests: Arc<Mutex<Vec<(Url, u64)>>>,
    }

    #[async_trait]
    impl RuntimeDownloader for RecordingRejectingDownloader {
        async fn open(
            &self,
            url: &Url,
            expected_size: u64,
        ) -> Result<DownloadResponse, RuntimeError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.clone(), expected_size));
            Ok(DownloadResponse {
                content_length: None,
                stream: Box::new(FakeStream {
                    chunks: Vec::new(),
                    index: 0,
                    reads: Arc::new(Mutex::new(0)),
                }),
            })
        }
    }

    #[async_trait]
    impl RuntimeByteStream for FakeStream {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, RuntimeError> {
            let chunk = self.chunks.get(self.index).cloned();
            if chunk.is_some() {
                self.index += 1;
                *self.reads.lock().unwrap() += 1;
            }
            Ok(chunk)
        }
    }
}
