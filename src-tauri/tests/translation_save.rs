use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use smartcat_translate::commands::translation_save::{
    save_translation_text_with_picker, SaveTranslationError, SaveTranslationOutcome,
    TranslationSavePicker,
};
use tempfile::tempdir;

struct FakePicker {
    result: Result<Option<PathBuf>, ()>,
    labels: Mutex<Vec<(String, String)>>,
}

impl FakePicker {
    fn new(result: Result<Option<PathBuf>, ()>) -> Self {
        Self {
            result,
            labels: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl TranslationSavePicker for FakePicker {
    async fn choose_text_path(
        &self,
        _suggested_name: &str,
        title: &str,
        filter_name: &str,
    ) -> Result<Option<PathBuf>, SaveTranslationError> {
        self.labels
            .lock()
            .unwrap()
            .push((title.to_owned(), filter_name.to_owned()));
        self.result.clone().map_err(|_| SaveTranslationError)
    }
}

#[tokio::test]
async fn native_save_writes_the_exact_text_to_the_selected_file() {
    let root = tempdir().unwrap();
    let path = root.path().join("translated.txt");

    let outcome = save_translation_text_with_picker(
        &FakePicker::new(Ok(Some(path.clone()))),
        "translated content",
        "ko",
        "ko",
    )
    .await
    .unwrap();

    assert_eq!(outcome, SaveTranslationOutcome::Saved);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "translated content");
}

#[tokio::test]
async fn native_save_reports_cancel_without_creating_a_file() {
    let outcome =
        save_translation_text_with_picker(&FakePicker::new(Ok(None)), "private", "en", "en")
            .await
            .unwrap();

    assert_eq!(outcome, SaveTranslationOutcome::Cancelled);
}

#[tokio::test]
async fn native_save_returns_one_fixed_error_without_content_or_paths() {
    let root = tempdir().unwrap();
    for picker in [
        FakePicker::new(Err(())),
        FakePicker::new(Ok(Some(root.path().to_path_buf()))),
    ] {
        let error = save_translation_text_with_picker(&picker, "PRIVATE-CONTENT", "ko", "ko")
            .await
            .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert_eq!(error, SaveTranslationError);
        assert!(!rendered.contains("PRIVATE-CONTENT"));
        assert!(!rendered.contains(&root.path().display().to_string()));
    }
}

#[tokio::test]
async fn native_save_localizes_only_the_korean_and_english_dialog_labels() {
    for (locale, expected) in [
        ("ko", ("번역문 저장", "텍스트 파일")),
        ("en", ("Save translation", "Text file")),
    ] {
        let picker = FakePicker::new(Ok(None));
        let outcome = save_translation_text_with_picker(&picker, "private", "ko", locale)
            .await
            .unwrap();

        assert_eq!(outcome, SaveTranslationOutcome::Cancelled);
        assert_eq!(
            picker.labels.lock().unwrap().as_slice(),
            &[(expected.0.to_owned(), expected.1.to_owned())]
        );
    }

    let picker = FakePicker::new(Ok(None));
    let error = save_translation_text_with_picker(
        &picker,
        "PRIVATE-CONTENT",
        "ko",
        "../../ARBITRARY-LOCALE",
    )
    .await
    .unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert_eq!(error, SaveTranslationError);
    assert!(picker.labels.lock().unwrap().is_empty());
    assert!(!rendered.contains("ARBITRARY-LOCALE"));
    assert!(!rendered.contains("PRIVATE-CONTENT"));
}
