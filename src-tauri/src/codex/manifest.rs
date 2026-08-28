use std::collections::HashSet;

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

#[cfg(test)]
mod tests {
    use super::{EmbeddedRuntimeManifest, HostTarget};

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
