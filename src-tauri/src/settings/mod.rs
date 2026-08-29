pub mod store;
pub mod types;

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::codex::transport::{AppServerTransport, TransportError};
use types::AvailableModel;

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
        loop {
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
            for model in response.data {
                if model.id.trim().is_empty() || model.display_name.trim().is_empty() {
                    return Err(ModelCatalogError::InvalidResponse);
                }
                let supported_reasoning_efforts = model
                    .supported_reasoning_efforts
                    .into_iter()
                    .map(|effort| effort.reasoning_effort)
                    .collect::<Vec<_>>();
                if supported_reasoning_efforts
                    .iter()
                    .any(|effort| effort.trim().is_empty())
                {
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
            if next.is_empty() || !seen_cursors.insert(next.clone()) || seen_cursors.len() > 100 {
                return Err(ModelCatalogError::InvalidResponse);
            }
            cursor = Some(next);
        }
    }
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
