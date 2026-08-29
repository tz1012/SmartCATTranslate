use std::path::PathBuf;

use async_trait::async_trait;
use smartcat_translate::commands::translation_save::{
    save_translation_text_with_picker, SaveTranslationError, SaveTranslationOutcome,
    TranslationSavePicker,
};
use tempfile::tempdir;

struct FakePicker(Result<Option<PathBuf>, ()>);

#[async_trait]
impl TranslationSavePicker for FakePicker {
    async fn choose_text_path(
        &self,
        _suggested_name: &str,
    ) -> Result<Option<PathBuf>, SaveTranslationError> {
        self.0.clone().map_err(|_| SaveTranslationError)
    }
}

#[tokio::test]
async fn native_save_writes_the_exact_text_to_the_selected_file() {
    let root = tempdir().unwrap();
    let path = root.path().join("translated.txt");

    let outcome = save_translation_text_with_picker(
        &FakePicker(Ok(Some(path.clone()))),
        "translated content",
        "ko",
    )
    .await
    .unwrap();

    assert_eq!(outcome, SaveTranslationOutcome::Saved);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "translated content");
}

#[tokio::test]
async fn native_save_reports_cancel_without_creating_a_file() {
    let outcome = save_translation_text_with_picker(&FakePicker(Ok(None)), "private", "en")
        .await
        .unwrap();

    assert_eq!(outcome, SaveTranslationOutcome::Cancelled);
}

#[tokio::test]
async fn native_save_returns_one_fixed_error_without_content_or_paths() {
    let root = tempdir().unwrap();
    for picker in [
        FakePicker(Err(())),
        FakePicker(Ok(Some(root.path().to_path_buf()))),
    ] {
        let error = save_translation_text_with_picker(&picker, "PRIVATE-CONTENT", "ko")
            .await
            .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert_eq!(error, SaveTranslationError);
        assert!(!rendered.contains("PRIVATE-CONTENT"));
        assert!(!rendered.contains(&root.path().display().to_string()));
    }
}
