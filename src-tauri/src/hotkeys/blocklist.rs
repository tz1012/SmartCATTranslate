use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{AppIdentity, Platform};

pub const MAX_BLOCKED_APPS: usize = 128;
const MAX_IDENTITY_CHARS: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockedApp {
    pub platform: Platform,
    executable: Option<String>,
    bundle_id: Option<String>,
    catalog_name: Option<String>,
}

impl BlockedApp {
    pub fn new(
        platform: Platform,
        executable: Option<&str>,
        bundle_id: Option<&str>,
        catalog_name: Option<&str>,
    ) -> Result<Self, BlocklistError> {
        let identity =
            AppIdentity::new(executable, bundle_id).ok_or(BlocklistError::InvalidEntry)?;
        let catalog_name = catalog_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if catalog_name.as_ref().is_some_and(|value| {
            value.chars().count() > MAX_IDENTITY_CHARS || value.chars().any(char::is_control)
        }) {
            return Err(BlocklistError::InvalidEntry);
        }
        Ok(Self {
            platform,
            executable: identity.executable_basename().map(str::to_owned),
            bundle_id: identity.bundle_id().map(str::to_owned),
            catalog_name,
        })
    }

    pub fn catalog_name(&self) -> Option<&str> {
        self.catalog_name.as_deref()
    }

    pub fn validate(&self) -> Result<(), BlocklistError> {
        Self::new(
            self.platform,
            self.executable.as_deref(),
            self.bundle_id.as_deref(),
            self.catalog_name.as_deref(),
        )
        .map(|_| ())
    }

    fn matches(&self, app: &AppIdentity) -> bool {
        self.platform == current_platform()
            && (self.executable.as_ref().is_some_and(|expected| {
                app.executable_basename()
                    .is_some_and(|actual| expected.eq_ignore_ascii_case(actual))
            }) || self.bundle_id.as_ref().is_some_and(|expected| {
                app.bundle_id()
                    .is_some_and(|actual| expected.eq_ignore_ascii_case(actual))
            }))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocklist {
    entries: Vec<BlockedApp>,
}

impl Blocklist {
    pub fn new(entries: Vec<BlockedApp>) -> Result<Self, BlocklistError> {
        if entries.len() > MAX_BLOCKED_APPS {
            return Err(BlocklistError::TooManyEntries);
        }
        let mut unique = HashSet::new();
        for entry in &entries {
            entry.validate()?;
            let key = (
                entry.platform,
                entry
                    .executable
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                entry
                    .bundle_id
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            );
            if !unique.insert(key) {
                return Err(BlocklistError::DuplicateEntry);
            }
        }
        Ok(Self { entries })
    }

    pub fn allows(&self, app: &AppIdentity) -> bool {
        self.blocking_entry(app).is_none()
    }

    pub fn blocking_entry(&self, app: &AppIdentity) -> Option<&BlockedApp> {
        self.entries.iter().find(|entry| entry.matches(app))
    }

    pub fn entries(&self) -> &[BlockedApp] {
        &self.entries
    }
}

fn current_platform() -> Platform {
    #[cfg(windows)]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BlocklistError {
    #[error("invalid blocked application identity")]
    InvalidEntry,
    #[error("too many blocked applications")]
    TooManyEntries,
    #[error("duplicate blocked application")]
    DuplicateEntry,
}
