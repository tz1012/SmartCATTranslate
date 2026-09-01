use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use super::types::{AppSettings, SettingsError, MAX_SETTINGS_DOCUMENT_BYTES};

#[async_trait]
pub trait SettingsBackend: Send + Sync {
    async fn read(&self) -> Result<Option<Value>, String>;
    async fn replace(&self, value: Value) -> Result<(), String>;
}

pub struct SettingsStore<B> {
    backend: B,
}

impl<B: SettingsBackend> SettingsStore<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub async fn load(&self) -> Result<AppSettings, SettingsError> {
        let value = self
            .backend
            .read()
            .await
            .map_err(|_| SettingsError::Persistence)?;
        let Some(value) = value else {
            let settings = AppSettings::default();
            self.persist(&settings).await?;
            return Ok(settings);
        };
        let (mut value, legacy_wrapper) = match value.get("settings") {
            Some(settings) => (settings.clone(), true),
            None => (value, false),
        };
        let missing_version = value.get("schemaVersion").is_none();
        let schema_version = value
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let migrated_schema_one = schema_version == 1;
        if migrated_schema_one {
            if value
                .get("theme")
                .and_then(Value::as_str)
                .unwrap_or("system")
                == "system"
            {
                value["theme"] = Value::String("light".to_owned());
            }
            value["schemaVersion"] = Value::from(super::types::SETTINGS_SCHEMA_VERSION);
        }
        let settings: AppSettings =
            serde_json::from_value(value).map_err(|_| SettingsError::InvalidDocument)?;
        settings.validate()?;
        if legacy_wrapper || missing_version || migrated_schema_one {
            self.persist(&settings).await?;
        }
        Ok(settings)
    }

    pub async fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        self.persist(settings).await
    }

    async fn persist(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        let value = serde_json::to_value(settings).map_err(|_| SettingsError::InvalidDocument)?;
        self.backend
            .replace(value)
            .await
            .map_err(|_| SettingsError::Persistence)
    }
}

#[derive(Clone, Debug)]
pub struct FileSettingsBackend {
    path: PathBuf,
}

impl FileSettingsBackend {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl SettingsBackend for FileSettingsBackend {
    async fn read(&self) -> Result<Option<Value>, String> {
        let path = self.path.clone();
        tauri::async_runtime::spawn_blocking(move || read_document(&path))
            .await
            .map_err(|_| "settings read task failed".to_owned())?
    }

    async fn replace(&self, value: Value) -> Result<(), String> {
        let path = self.path.clone();
        tauri::async_runtime::spawn_blocking(move || replace_document(&path, &value))
            .await
            .map_err(|_| "settings write task failed".to_owned())?
    }
}

fn read_document(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_SETTINGS_DOCUMENT_BYTES as u64 => {
            return Err("settings document is too large".to_owned())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("settings metadata read failed".to_owned()),
    }
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "settings document is invalid".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("settings read failed".to_owned()),
    }
}

fn replace_document(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid settings path".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|_| "create settings directory failed".to_owned())?;
    let bytes = serde_json::to_vec(value).map_err(|_| "serialize settings failed".to_owned())?;
    if bytes.len() > MAX_SETTINGS_DOCUMENT_BYTES {
        return Err("settings document is too large".to_owned());
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".smartcat-settings-")
        .tempfile_in(parent)
        .map_err(|_| "create temporary settings failed".to_owned())?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file_mut().sync_all())
        .map_err(|_| "write temporary settings failed".to_owned())?;
    temporary
        .persist(path)
        .map_err(|_| "replace settings failed".to_owned())?;
    sync_parent(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), String> {
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "sync settings directory failed".to_owned())
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), String> {
    Ok(())
}
