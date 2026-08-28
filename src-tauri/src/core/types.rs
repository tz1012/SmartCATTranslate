use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationProfile {
    pub source_language: Option<String>,
    pub target_language: String,
    pub quality: Quality,
    pub tone: Tone,
    pub protected_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub text: String,
    pub profile: TranslationProfile,
    pub mode: TranslationMode,
    pub secret: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    pub translated_text: String,
    pub detected_language: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Fast,
    Balanced,
    Precise,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Tone {
    Natural,
    Literal,
    Formal,
    Casual,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranslationMode {
    Translate,
    Rewrite,
}

#[cfg(test)]
mod tests {
    use super::{Quality, Tone, TranslationMode, TranslationProfile, TranslationRequest};

    #[test]
    fn serializes_the_translation_request_with_camel_case_fields_and_values() {
        let request = TranslationRequest {
            text: "Hello".to_owned(),
            profile: TranslationProfile {
                source_language: Some("en".to_owned()),
                target_language: "ko".to_owned(),
                quality: Quality::Balanced,
                tone: Tone::Natural,
                protected_terms: vec!["SmartCAT".to_owned()],
            },
            mode: TranslationMode::Translate,
            secret: true,
        };

        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"text":"Hello","profile":{"sourceLanguage":"en","targetLanguage":"ko","quality":"balanced","tone":"natural","protectedTerms":["SmartCAT"]},"mode":"translate","secret":true}"#
        );
    }
}
