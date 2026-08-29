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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    #[default]
    General,
    Technical,
    Legal,
    Medical,
    Business,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryMapping {
    pub source_term: String,
    pub target_term: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub text: String,
    pub profile: TranslationProfile,
    pub field: Field,
    pub glossary: Vec<GlossaryMapping>,
    pub mode: TranslationMode,
    pub secret: bool,
    #[serde(skip)]
    pub model: TranslationModel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TranslationModel {
    #[default]
    Automatic,
    Specific(String),
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
    use super::{
        Field, GlossaryMapping, Quality, Tone, TranslationMode, TranslationModel,
        TranslationProfile, TranslationRequest,
    };

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
            field: Field::Technical,
            glossary: vec![GlossaryMapping {
                source_term: "cloud".to_owned(),
                target_term: "클라우드".to_owned(),
            }],
            mode: TranslationMode::Translate,
            secret: true,
            model: TranslationModel::Automatic,
        };

        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"text":"Hello","profile":{"sourceLanguage":"en","targetLanguage":"ko","quality":"balanced","tone":"natural","protectedTerms":["SmartCAT"]},"field":"technical","glossary":[{"sourceTerm":"cloud","targetTerm":"클라우드"}],"mode":"translate","secret":true}"#
        );
    }
}
