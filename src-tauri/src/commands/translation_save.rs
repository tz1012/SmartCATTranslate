use std::path::PathBuf;

use async_trait::async_trait;
use serde::Serialize;
use tauri_plugin_dialog::DialogExt;

const MAX_SAVED_CHARS: usize = 400_000;
const MAX_SAVED_BYTES: usize = MAX_SAVED_CHARS * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SaveTranslationOutcome {
    Saved,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the translation could not be saved")]
pub struct SaveTranslationError;

#[async_trait]
pub trait TranslationSavePicker: Send + Sync {
    async fn choose_text_path(
        &self,
        suggested_name: &str,
    ) -> Result<Option<PathBuf>, SaveTranslationError>;
}

struct TauriTranslationSavePicker(tauri::AppHandle);

#[async_trait]
impl TranslationSavePicker for TauriTranslationSavePicker {
    async fn choose_text_path(
        &self,
        suggested_name: &str,
    ) -> Result<Option<PathBuf>, SaveTranslationError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.0
            .dialog()
            .file()
            .set_title("Save translation")
            .set_file_name(suggested_name)
            .add_filter("Text", &["txt"])
            .save_file(move |selection| {
                let result = selection
                    .map(|path| path.into_path().map_err(|_| SaveTranslationError))
                    .transpose();
                let _ = sender.send(result);
            });
        receiver.await.map_err(|_| SaveTranslationError)?
    }
}

pub async fn save_translation_text_with_picker(
    picker: &(dyn TranslationSavePicker + Sync),
    text: &str,
    target_language: &str,
) -> Result<SaveTranslationOutcome, SaveTranslationError> {
    if text.is_empty() || text.chars().count() > MAX_SAVED_CHARS || text.len() > MAX_SAVED_BYTES {
        return Err(SaveTranslationError);
    }
    let language = target_language
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(32)
        .collect::<String>();
    let suggested_name = format!(
        "smartcat-translation-{}.txt",
        if language.is_empty() {
            "translated"
        } else {
            &language
        }
    );
    let Some(path) = picker.choose_text_path(&suggested_name).await? else {
        return Ok(SaveTranslationOutcome::Cancelled);
    };
    tokio::fs::write(path, text)
        .await
        .map_err(|_| SaveTranslationError)?;
    Ok(SaveTranslationOutcome::Saved)
}

#[tauri::command]
pub async fn save_translation_text(
    app: tauri::AppHandle,
    text: String,
    target_language: String,
) -> Result<SaveTranslationOutcome, String> {
    save_translation_text_with_picker(&TauriTranslationSavePicker(app), &text, &target_language)
        .await
        .map_err(|_| "translation_save_failed".to_owned())
}
