use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::Deserialize;
use url::Url;

use crate::codex::runtime::RuntimeError;

const VERSION: &str = "0.144.4";
const TAG: &str = "rust-v0.144.4";
const RELEASE_URL: &str = "https://github.com/openai/codex/releases/tag/rust-v0.144.4";
const LICENSE_URL: &str = "https://raw.githubusercontent.com/openai/codex/rust-v0.144.4/LICENSE";
const NOTICE_URL: &str = "https://raw.githubusercontent.com/openai/codex/rust-v0.144.4/NOTICE";
const DOWNLOAD_PREFIX: &str = "https://github.com/openai/codex/releases/download/rust-v0.144.4/";
const UPSTREAM_COMMIT: &str = "8c68d4c87dc54d38861f5114e920c3de2efa5876";
const SOURCE_ARCHIVE_SHA256: &str =
    "14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009";
const PATCH_VERSION: &str = "smartcat-1";
const PATCH_SHA256: &str = "277656cea5ca940c30cf692bff1bcbe398ca18a60ed57bcdc6f0a1a82388704a";
const MAX_BUNDLED_RUNTIME_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostTarget {
    WindowsX86_64,
    MacosAarch64,
    MacosX86_64,
}

impl HostTarget {
    #[allow(unreachable_code)]
    pub(crate) const fn current() -> Self {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            return Self::WindowsX86_64;
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Self::MacosAarch64;
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            return Self::MacosX86_64;
        }
        panic!("unsupported Codex runtime host target")
    }

    pub(crate) const fn triple(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "x86_64-pc-windows-msvc",
            Self::MacosAarch64 => "aarch64-apple-darwin",
            Self::MacosX86_64 => "x86_64-apple-darwin",
        }
    }

    const fn asset_name(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "codex-x86_64-pc-windows-msvc.exe.zip",
            Self::MacosAarch64 => "codex-aarch64-apple-darwin.tar.gz",
            Self::MacosX86_64 => "codex-x86_64-apple-darwin.tar.gz",
        }
    }

    const fn archive_entry(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "codex-x86_64-pc-windows-msvc.exe",
            Self::MacosAarch64 => "codex-aarch64-apple-darwin",
            Self::MacosX86_64 => "codex-x86_64-apple-darwin",
        }
    }

    const fn all() -> [Self; 3] {
        [Self::WindowsX86_64, Self::MacosAarch64, Self::MacosX86_64]
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PinnedRuntime {
    pub(crate) version: Version,
    pub(crate) target: String,
    pub(crate) url: Url,
    pub(crate) sha256: String,
    pub(crate) size: u64,
    pub(crate) archive_entry: String,
}

impl PinnedRuntime {
    pub(crate) fn version(&self) -> &Version {
        &self.version
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn archive_entry(&self) -> &str {
        &self.archive_entry
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        version: Version,
        target: impl Into<String>,
        url: Url,
        sha256: String,
        size: u64,
        archive_entry: impl Into<String>,
    ) -> Self {
        Self {
            version,
            target: target.into(),
            url,
            sha256,
            size,
            archive_entry: archive_entry.into(),
        }
    }
}

pub(crate) struct EmbeddedRuntimeManifest;

impl EmbeddedRuntimeManifest {
    pub(crate) fn load_for_host(target: HostTarget) -> Result<PinnedRuntime, RuntimeError> {
        Self::validate_for_host(include_str!("../../resources/codex-runtime.json"), target)
    }

    fn validate_for_host(json: &str, target: HostTarget) -> Result<PinnedRuntime, RuntimeError> {
        let manifest: Manifest =
            serde_json::from_str(json).map_err(|_| RuntimeError::ManifestInvalid)?;
        if manifest.version != VERSION
            || manifest.tag != TAG
            || manifest.release_url != RELEASE_URL
            || manifest.license.spdx != "Apache-2.0"
            || manifest.license.url != LICENSE_URL
            || manifest.license.notice_url != NOTICE_URL
            || manifest.runtimes.len() != 3
        {
            return Err(RuntimeError::ManifestInvalid);
        }

        let mut seen = HashSet::new();
        for expected in HostTarget::all() {
            let runtime = manifest
                .runtimes
                .iter()
                .find(|runtime| runtime.target == expected.triple())
                .ok_or(RuntimeError::ManifestInvalid)?;
            if !seen.insert(runtime.target.as_str()) {
                return Err(RuntimeError::ManifestInvalid);
            }
            validate_runtime(runtime, expected)?;
        }
        if seen.len() != 3 {
            return Err(RuntimeError::ManifestInvalid);
        }

        let selected = manifest
            .runtimes
            .into_iter()
            .find(|runtime| runtime.target == target.triple())
            .ok_or(RuntimeError::ManifestInvalid)?;
        Ok(PinnedRuntime {
            version: Version::parse(VERSION).map_err(|_| RuntimeError::ManifestInvalid)?,
            target: selected.target,
            url: Url::parse(&selected.url).map_err(|_| RuntimeError::ManifestInvalid)?,
            sha256: selected.sha256,
            size: selected.size,
            archive_entry: selected.archive_entry,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedBundledRuntime {
    path: PathBuf,
    version: Version,
}

impl VerifiedBundledRuntime {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn version(&self) -> &Version {
        &self.version
    }
}

pub(crate) struct BundledRuntimeManifest;

impl BundledRuntimeManifest {
    pub(crate) fn load_and_verify(
        manifest_path: &Path,
        binary_path: &Path,
        target: HostTarget,
    ) -> Result<VerifiedBundledRuntime, RuntimeError> {
        reject_link_or_reparse_ancestors(manifest_path)?;
        reject_link_or_reparse_ancestors(binary_path)?;
        let manifest_metadata =
            std::fs::symlink_metadata(manifest_path).map_err(|_| RuntimeError::ManifestInvalid)?;
        if !manifest_metadata.is_file()
            || manifest_metadata.len() == 0
            || manifest_metadata.len() > 64 * 1024
        {
            return Err(RuntimeError::ManifestInvalid);
        }
        let bytes = std::fs::read(manifest_path).map_err(|_| RuntimeError::ManifestInvalid)?;
        if bytes.len() as u64 != manifest_metadata.len() {
            return Err(RuntimeError::ManifestInvalid);
        }
        let manifest: BundledManifest =
            serde_json::from_slice(&bytes).map_err(|_| RuntimeError::ManifestInvalid)?;
        let expected_binary = if target == HostTarget::WindowsX86_64 {
            format!("smartcat-codex-{}.exe", target.triple())
        } else {
            format!("smartcat-codex-{}", target.triple())
        };
        if manifest.schema_version != 1
            || manifest.target != target.triple()
            || manifest.binary != expected_binary
            || manifest.upstream_tag != TAG
            || manifest.upstream_commit != UPSTREAM_COMMIT
            || manifest.source_archive_sha256 != SOURCE_ARCHIVE_SHA256
            || manifest.patch_version != PATCH_VERSION
            || manifest.patch_sha256 != PATCH_SHA256
            || !manifest.cargo_locked
            || manifest.size == 0
            || manifest.size > MAX_BUNDLED_RUNTIME_BYTES
            || !valid_sha256(&manifest.sha256)
        {
            return Err(RuntimeError::ManifestInvalid);
        }
        let binary_metadata =
            std::fs::symlink_metadata(binary_path).map_err(|_| RuntimeError::ManifestInvalid)?;
        if !binary_metadata.is_file() || binary_metadata.len() != manifest.size {
            return Err(RuntimeError::ManifestInvalid);
        }
        let actual = sha256_file(binary_path, manifest.size)?;
        if actual != manifest.sha256 {
            return Err(RuntimeError::ChecksumMismatch);
        }
        Ok(VerifiedBundledRuntime {
            path: binary_path.to_path_buf(),
            version: Version::parse(VERSION).map_err(|_| RuntimeError::ManifestInvalid)?,
        })
    }
}

fn sha256_file(path: &Path, expected_size: u64) -> Result<String, RuntimeError> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path).map_err(|_| RuntimeError::ManifestInvalid)?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| RuntimeError::ManifestInvalid)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(RuntimeError::ManifestInvalid)?;
        if total > expected_size {
            return Err(RuntimeError::ManifestInvalid);
        }
        digest.update(&buffer[..read]);
    }
    if total != expected_size {
        return Err(RuntimeError::ManifestInvalid);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn reject_link_or_reparse_ancestors(path: &Path) -> Result<(), RuntimeError> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || is_reparse(&metadata) => {
                return Err(RuntimeError::ManifestInvalid)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RuntimeError::ManifestInvalid),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn validate_runtime(runtime: &ManifestRuntime, target: HostTarget) -> Result<(), RuntimeError> {
    let expected_url = format!("{DOWNLOAD_PREFIX}{}", target.asset_name());
    if runtime.url != expected_url
        || runtime.archive_entry != target.archive_entry()
        || runtime.archive_entry.contains(['/', '\\'])
        || runtime.size == 0
        || runtime.size > MAX_DOWNLOAD_BYTES
        || runtime.sha256.len() != 64
        || !runtime
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RuntimeError::ManifestInvalid);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: String,
    tag: String,
    release_url: String,
    license: ManifestLicense,
    runtimes: Vec<ManifestRuntime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestLicense {
    spdx: String,
    url: String,
    notice_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRuntime {
    target: String,
    url: String,
    sha256: String,
    size: u64,
    archive_entry: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundledManifest {
    schema_version: u32,
    target: String,
    binary: String,
    sha256: String,
    size: u64,
    upstream_tag: String,
    upstream_commit: String,
    source_archive_sha256: String,
    patch_version: String,
    patch_sha256: String,
    cargo_locked: bool,
}

#[cfg(test)]
mod tests {
    use super::{BundledRuntimeManifest, EmbeddedRuntimeManifest, HostTarget};
    use sha2::{Digest, Sha256};

    #[test]
    fn embedded_manifest_selects_the_current_host_target() {
        let pinned = EmbeddedRuntimeManifest::load_for_host(HostTarget::current()).unwrap();

        assert_eq!(pinned.target(), HostTarget::current().triple());
        assert!(pinned.size() > 0);
        assert_eq!(pinned.sha256().len(), 64);
    }

    #[test]
    fn manifest_rejects_a_hash_or_archive_entry_that_cannot_establish_trust() {
        let bad_hash = manifest_fixture("short", "codex.exe");
        let bad_entry = manifest_fixture(&"a".repeat(64), "../codex.exe");

        assert!(
            EmbeddedRuntimeManifest::validate_for_host(&bad_hash, HostTarget::WindowsX86_64)
                .is_err()
        );
        assert!(
            EmbeddedRuntimeManifest::validate_for_host(&bad_entry, HostTarget::WindowsX86_64)
                .is_err()
        );
    }

    #[test]
    fn bundled_runtime_manifest_binds_the_exact_sidecar_bytes_and_provenance() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("smartcat-codex.exe");
        std::fs::write(&binary, b"patched-codex-fixture").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"patched-codex-fixture"));
        let manifest = root.path().join("runtime.json");
        std::fs::write(
            &manifest,
            format!(
                r#"{{"schemaVersion":1,"target":"x86_64-pc-windows-msvc","binary":"smartcat-codex-x86_64-pc-windows-msvc.exe","sha256":"{hash}","size":21,"upstreamTag":"rust-v0.144.4","upstreamCommit":"8c68d4c87dc54d38861f5114e920c3de2efa5876","sourceArchiveSha256":"14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009","patchVersion":"smartcat-1","patchSha256":"277656cea5ca940c30cf692bff1bcbe398ca18a60ed57bcdc6f0a1a82388704a","cargoLocked":true}}"#
            ),
        )
        .unwrap();

        let verified =
            BundledRuntimeManifest::load_and_verify(&manifest, &binary, HostTarget::WindowsX86_64)
                .unwrap();
        assert_eq!(verified.path(), binary.as_path());

        std::fs::write(&binary, b"tampered").unwrap();
        assert!(BundledRuntimeManifest::load_and_verify(
            &manifest,
            &binary,
            HostTarget::WindowsX86_64,
        )
        .is_err());
    }

    fn manifest_fixture(hash: &str, archive_entry: &str) -> String {
        format!(
            r#"{{
              "version":"0.144.4",
              "tag":"rust-v0.144.4",
              "releaseUrl":"https://github.com/openai/codex/releases/tag/rust-v0.144.4",
              "license":{{"spdx":"Apache-2.0","url":"https://raw.githubusercontent.com/openai/codex/rust-v0.144.4/LICENSE","noticeUrl":"https://raw.githubusercontent.com/openai/codex/rust-v0.144.4/NOTICE"}},
              "runtimes":[
                {{"target":"x86_64-pc-windows-msvc","url":"https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-x86_64-pc-windows-msvc.exe.zip","sha256":"{hash}","size":1,"archiveEntry":"{archive_entry}"}},
                {{"target":"aarch64-apple-darwin","url":"https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-aarch64-apple-darwin.tar.gz","sha256":"{mac_hash}","size":1,"archiveEntry":"codex-aarch64-apple-darwin"}},
                {{"target":"x86_64-apple-darwin","url":"https://github.com/openai/codex/releases/download/rust-v0.144.4/codex-x86_64-apple-darwin.tar.gz","sha256":"{mac_hash}","size":1,"archiveEntry":"codex-x86_64-apple-darwin"}}
              ]
            }}"#,
            mac_hash = "b".repeat(64)
        )
    }
}
