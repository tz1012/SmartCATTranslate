use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tauri::Runtime;

use super::types::{AppSettings, SettingsError};

const SETTINGS_KEY: &str = "settings";

#[async_trait]
pub trait SettingsBackend: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Value>, String>;
    async fn set(&self, key: &str, value: Value) -> Result<(), String>;
    async fn save(&self) -> Result<(), String>;
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
            .get(SETTINGS_KEY)
            .await
            .map_err(|_| SettingsError::Persistence)?;
        let Some(value) = value else {
            let settings = AppSettings::default();
            self.persist(&settings).await?;
            return Ok(settings);
        };
        let needs_migration = value.get("schemaVersion").is_none();
        let settings: AppSettings =
            serde_json::from_value(value).map_err(|_| SettingsError::InvalidDocument)?;
        settings.validate()?;
        if needs_migration {
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
            .set(SETTINGS_KEY, value)
            .await
            .map_err(|_| SettingsError::Persistence)?;
        self.backend
            .save()
            .await
            .map_err(|_| SettingsError::Persistence)
    }
}

pub struct TauriStoreBackend<R: Runtime> {
    store: Arc<tauri_plugin_store::Store<R>>,
    path: PathBuf,
}

impl<R: Runtime> TauriStoreBackend<R> {
    pub fn new(store: Arc<tauri_plugin_store::Store<R>>, path: PathBuf) -> Self {
        Self { store, path }
    }
}

#[async_trait]
impl<R: Runtime> SettingsBackend for TauriStoreBackend<R> {
    async fn get(&self, key: &str) -> Result<Option<Value>, String> {
        Ok(self.store.get(key))
    }

    async fn set(&self, key: &str, value: Value) -> Result<(), String> {
        self.store.set(key, value);
        Ok(())
    }

    async fn save(&self) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "invalid store path".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|_| "create store directory failed".to_owned())?;
        let values: HashMap<String, Value> = self.store.entries().into_iter().collect();
        let bytes = serde_json::to_vec(&values).map_err(|_| "serialize store failed".to_owned())?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".smartcat-settings-")
            .tempfile_in(parent)
            .map_err(|_| "create temporary store failed".to_owned())?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file_mut().sync_all())
            .map_err(|_| "write temporary store failed".to_owned())?;
        temporary
            .persist(&self.path)
            .map_err(|_| "replace store failed".to_owned())?;
        Ok(())
    }
}
