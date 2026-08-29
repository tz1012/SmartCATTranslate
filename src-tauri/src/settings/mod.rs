pub mod store;
pub mod types;

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::codex::transport::{AppServerTransport, TransportError};
use types::AvailableModel;

const MODEL_PAGE_SIZE: usize = 100;
const MAX_MODEL_PAGES: usize = 100;
const MAX_MODELS: usize = MODEL_PAGE_SIZE * MAX_MODEL_PAGES;
const MAX_MODEL_FIELD_CHARS: usize = 120;
const MAX_MODEL_FIELD_BYTES: usize = 512;
const MAX_REASONING_EFFORTS: usize = 16;
const MAX_REASONING_EFFORT_CHARS: usize = 32;
const MAX_CURSOR_BYTES: usize = 512;

pub struct ModelCatalogService {
    transport: Arc<dyn AppServerTransport>,
}

impl ModelCatalogService {
    pub fn new(transport: Arc<dyn AppServerTransport>) -> Self {
        Self { transport }
    }

    pub async fn list(&self) -> Result<Vec<AvailableModel>, ModelCatalogError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut seen_model_ids = HashSet::new();
        let mut default_count = 0_usize;
        let mut page_count = 0_usize;
        loop {
            page_count += 1;
            let value = self
                .transport
                .request(
                    "model/list",
                    json!({
                        "cursor": cursor,
                        "limit": 100,
                        "includeHidden": false
                    }),
                )
                .await?;
            let response: ModelListWire =
                serde_json::from_value(value).map_err(|_| ModelCatalogError::InvalidResponse)?;
            if response.data.len() > MODEL_PAGE_SIZE
                || models.len().saturating_add(response.data.len()) > MAX_MODELS
            {
                return Err(ModelCatalogError::InvalidResponse);
            }
            for model in response.data {
                if !valid_model_field(&model.id)
                    || !valid_model_field(&model.display_name)
                    || !seen_model_ids.insert(model.id.clone())
                    || model.supported_reasoning_efforts.len() > MAX_REASONING_EFFORTS
                {
                    return Err(ModelCatalogError::InvalidResponse);
                }
                let supported_reasoning_efforts = model
                    .supported_reasoning_efforts
                    .into_iter()
                    .map(|effort| effort.reasoning_effort)
                    .collect::<Vec<_>>();
                if supported_reasoning_efforts.iter().any(|effort| {
                    effort.trim().is_empty()
                        || effort.chars().count() > MAX_REASONING_EFFORT_CHARS
                        || effort.len() > MAX_MODEL_FIELD_BYTES
                }) {
                    return Err(ModelCatalogError::InvalidResponse);
                }
                default_count += usize::from(model.is_default);
                if default_count > 1 {
                    return Err(ModelCatalogError::InvalidResponse);
                }
                models.push(AvailableModel {
                    id: model.id,
                    display_name: model.display_name,
                    supported_reasoning_efforts,
                    is_default: model.is_default,
                });
            }
            let Some(next) = response.next_cursor else {
                return Ok(models);
            };
            if page_count >= MAX_MODEL_PAGES
                || next.is_empty()
                || next.len() > MAX_CURSOR_BYTES
                || !seen_cursors.insert(next.clone())
            {
                return Err(ModelCatalogError::InvalidResponse);
            }
            cursor = Some(next);
        }
    }
}

fn valid_model_field(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= MAX_MODEL_FIELD_CHARS
        && value.len() <= MAX_MODEL_FIELD_BYTES
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelListWire {
    data: Vec<ModelWire>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelWire {
    id: String,
    display_name: String,
    supported_reasoning_efforts: Vec<ReasoningEffortWire>,
    is_default: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReasoningEffortWire {
    reasoning_effort: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelCatalogError {
    #[error("the Codex model catalog request failed")]
    Transport(#[from] TransportError),
    #[error("the Codex model catalog response was invalid")]
    InvalidResponse,
}

impl ModelCatalogError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Transport(_) => "model_catalog_unavailable",
            Self::InvalidResponse => "invalid_model_catalog",
        }
    }
}
